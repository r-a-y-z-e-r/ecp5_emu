//! Fast mode (src/fast.rs) against the event kernel on the same design and
//! the same clock: every sample must agree, then fast mode is timed.
mod board;
mod common;
mod ebr_fixture;
use board::{decode_uart, pin, PERIOD_PS};
use common::reference;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

/// A stored build: `config` relative to the crate, and the nextpnr routed
/// netlist next to it when HDL names are used.
struct Build {
    device: Arc<ConfiguredDevice>,
    names: Option<Names>,
}

fn build(config: impl AsRef<Path>) -> Build {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let config = root.join(config);
    let routed = config.with_file_name("top_routed.json");
    let names = routed
        .exists()
        .then(|| Names::load(&routed, &model, "CABGA381").unwrap());
    let image = Arc::new(ConfigurationImage::load(&config, &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    Build { device, names }
}

impl Build {
    /// An HDL name (a bus expands to its bits) or `pin:BALL`.
    fn references(&self, name: &str) -> Vec<Reference> {
        match name.strip_prefix("pin:") {
            Some(ball) => vec![pin(ball)],
            None => self
                .names
                .as_ref()
                .unwrap()
                .expand(name)
                .unwrap()
                .into_iter()
                .map(|(_, r)| r)
                .collect(),
        }
    }
    /// Compiles with the clock on G2 and `inputs` driven from 0.
    fn compile(&self, probes: &[&str], inputs: &[&str]) -> (Compiled, Vec<SignalId>) {
        let inputs: Vec<(&str, Logic)> = inputs.iter().map(|&i| (i, Logic::Zero)).collect();
        self.compile_driven(probes, &inputs)
    }
    /// Compiles with the clock on G2 (from 0) and each input from its value.
    fn compile_driven(
        &self,
        probes: &[&str],
        inputs: &[(&str, Logic)],
    ) -> (Compiled, Vec<SignalId>) {
        let probes: Vec<Reference> = probes.iter().flat_map(|p| self.references(p)).collect();
        let c = compile::compile(
            Arc::clone(&self.device),
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: std::iter::once(("pin:G2", Logic::Zero))
                    .chain(inputs.iter().copied())
                    .flat_map(|(i, v)| self.references(i).into_iter().map(move |r| (r, v)))
                    .collect(),
                probes: probes.clone(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
        .unwrap();
        let signals = probes.iter().map(|r| c.probes[&r.label()]).collect();
        (c, signals)
    }
}

/// Starts fast mode on the 25 MHz clock at G2.
fn fast(c: &Compiled) -> FastSim {
    let mut fast = FastSim::new(&c.kernel).unwrap();
    let p = fast.program();
    println!(
        "FAST program: {} gates, {} registers, {} IOLOGIC, {} EBR, {} PLL, {} clock groups, {} slots",
        p.gates,
        p.registers,
        p.iologic,
        p.ebrs.len(),
        p.plls.len(),
        p.clock_groups,
        p.slots
    );
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    fast
}

/// Runs `FAST_SECONDS` (default 1) seconds of board time at 25 MHz and
/// prints the speed.
fn timed(fast: &mut FastSim, probe: SignalId, name: &str) {
    let board: f64 = std::env::var("FAST_SECONDS").map_or(1.0, |s| s.parse().unwrap());
    let cycles = (board * 25e6) as u64;
    let start = Instant::now();
    fast.run_cycles(cycles, probe).unwrap();
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "FAST {name}: {cycles} cycles of 25 MHz in {seconds:.3} s = {:.1} MHz ({:.2}x real time)",
        cycles as f64 / seconds / 1e6,
        board / seconds
    );
}

fn rise(n: u64) -> Time {
    Time::from_ps((2 * n - 1) * PERIOD_PS / 2)
}
fn fall(n: u64) -> Time {
    Time::from_ps(n * PERIOD_PS)
}

/// Both run up to `time`; every probe must agree there.
fn agree(c: &mut Compiled, fast: &mut FastSim, probes: &[SignalId], time: Time, what: &str) {
    c.kernel.run_until(time).unwrap();
    fast.run_until(time).unwrap();
    for (k, &p) in probes.iter().enumerate() {
        assert_eq!(
            fast.value(p),
            Some(c.kernel.value(p).unwrap()),
            "probe {k} at {time} ({what})"
        );
    }
}

/// Clock cycle `n` in the event kernel; fast mode runs its own clock.
/// `quarter` also compares a quarter period after the rising edge.
fn cycle(c: &mut Compiled, fast: &mut FastSim, probes: &[SignalId], n: u64, quarter: bool) {
    let clock = c.drivers["pin:G2"];
    c.kernel.schedule(rise(n), clock, Logic::One).unwrap();
    if quarter {
        let t = Time::from_ps((2 * n - 1) * PERIOD_PS / 2 + PERIOD_PS / 4);
        agree(c, fast, probes, t, "high phase");
    }
    c.kernel.schedule(fall(n), clock, Logic::Zero).unwrap();
    agree(c, fast, probes, fall(n), "after the falling edge");
}

/// Cycles `from..from + count` in one `run_cycles_sampled` call, long enough
/// for the host to hand a periodic schedule to the program: every probe must
/// agree after every falling edge. Returns the samples of the first probe.
fn segment(
    c: &mut Compiled,
    fast: &mut FastSim,
    probes: &[SignalId],
    from: u64,
    count: u64,
) -> Vec<u8> {
    let clock = c.drivers["pin:G2"];
    let mut event = Vec::new();
    for n in from..from + count {
        c.kernel.schedule(rise(n), clock, Logic::One).unwrap();
        c.kernel.schedule(fall(n), clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall(n)).unwrap();
        for &p in probes {
            event.push(u8::from(c.kernel.value(p).unwrap() == Logic::One));
        }
    }
    let samples = fast.run_cycles_sampled(count, probes).unwrap();
    let first = samples.iter().zip(&event).position(|(a, b)| a != b);
    assert_eq!(
        first.map(|i| (from + (i / probes.len()) as u64, i % probes.len())),
        None,
        "segment differs at (cycle, probe)"
    );
    agree(c, fast, probes, fall(from + count - 1), "after the segment");
    samples.iter().step_by(probes.len()).copied().collect()
}

/// Choosing settings on the UART reference leaves the kernel unchanged and
/// gives the same trace as the defaults, within the machine's core count.
#[test]
fn chosen_options_give_the_same_trace_as_the_defaults() {
    let (c, probes) = build(reference::prepared("uart_hello").join("top.config"))
        .compile(&["pin:L4", "led"], &[]);
    let mut plain = FastSim::new(&c.kernel).unwrap();
    let clock = c.drivers["pin:G2"];
    let period = Time::from_ps(PERIOD_PS);
    let now = c.kernel.now();
    let mut chosen = ecp5_emu::fast::choose_options(
        &c.kernel,
        FastSim::new(&c.kernel).unwrap(),
        clock,
        period,
        &probes,
        &|_| {},
        &|| false,
    )
    .unwrap();
    let options = chosen.program().options;
    assert!(options.threads >= 1);
    assert!(options.threads <= std::thread::available_parallelism().unwrap().get());
    assert_eq!(c.kernel.now(), now);
    assert_eq!(
        chosen.now().unwrap(),
        now,
        "the trial must not advance the returned simulation"
    );
    let fresh = FastSim::with_options(&c.kernel, &options).unwrap();
    assert_eq!(chosen.raw_values(), fresh.raw_values());
    assert_eq!(chosen.program().source, fresh.program().source);
    plain.clock(clock, period).unwrap();
    chosen.clock(clock, period).unwrap();
    assert_eq!(
        chosen.run_edges_sampled(12_000, &probes).unwrap(),
        plain.run_edges_sampled(12_000, &probes).unwrap()
    );
}

#[test]
fn remembered_simulation_is_reused_and_rewound() {
    // A child isolates the choice and library caches from parallel tests,
    // and lets this regression check run even under FAST_CHOOSE=full.
    const CHILD: &str = "ECP5_FAST_HANDOVER_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let scratch = std::env::temp_dir().join(format!(
            "ecp5-handover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&scratch).unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "remembered_simulation_is_reused_and_rewound",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("TMPDIR", &scratch)
            .env("FAST_TUNE", "0");
        for key in [
            "FAST_CHOOSE",
            "FAST_THREADS",
            "FAST_THREAD_MIN",
            "FAST_COLLAPSE",
            "FAST_BARRIER_LINES",
        ] {
            command.env_remove(key);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(scratch).unwrap();
        return;
    }
    let design = build(reference::prepared("uart_hello").join("top.config"));
    let (c, probes) = design.compile(&["pin:L4", "led"], &[]);
    let (mut equivalent, _) = design.compile(&["pin:L4", "led"], &[]);
    let path = ecp5_emu::fast::choice_memory(&c.kernel);
    assert_eq!(
        path,
        ecp5_emu::fast::choice_memory(&equivalent.kernel),
        "opening scopes are not part of the key"
    );
    let time = Time::from_ps(PERIOD_PS / 2);
    equivalent
        .kernel
        .schedule(time, equivalent.drivers["pin:G2"], Logic::One)
        .unwrap();
    equivalent.kernel.run_until(time).unwrap();
    assert_eq!(
        path,
        ecp5_emu::fast::choice_memory(&equivalent.kernel),
        "live state is not part of the key"
    );
    let (observed, _) = design.compile(&["pin:L4", "led", "clk_25mhz"], &[]);
    assert_eq!(
        path,
        ecp5_emu::fast::choice_memory(&observed.kernel),
        "probes of an existing plan are not part of the key"
    );
    let extra = design
        .names
        .as_ref()
        .unwrap()
        .resolved()
        .find_map(|(_, r)| match r {
            Reference::Wire(wire) => equivalent.kernel.signal(wire),
            _ => None,
        })
        .expect("an internal net in the execution plan");
    equivalent.kernel.watch(extra).unwrap();
    assert_eq!(
        path,
        ecp5_emu::fast::choice_memory(&equivalent.kernel),
        "the Waves watch API must not change the choice key"
    );

    let base = ecp5_emu::kernel::FastOptions::from_env();
    let options = ecp5_emu::kernel::FastOptions {
        threads: 1,
        collapse: 6,
        ..base
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!("1 6 1 {} {}\n", base.thread_min, base.barrier_lines),
    )
    .unwrap();
    assert_eq!(
        FastSim::new(&c.kernel).unwrap().program().options,
        base,
        "new must ignore the remembered options"
    );
    let changed_watch_start = FastSim::start(&equivalent.kernel).unwrap();
    assert_eq!(
        changed_watch_start.program().options,
        options,
        "a changed watch set must read the remembered options"
    );
    let initial = FastSim::start(&c.kernel).unwrap();
    assert_eq!(initial.program().options, options);
    let source = initial.program().source.as_ptr();
    let progress = std::cell::RefCell::new(Vec::new());
    let period = Time::from_ps(PERIOD_PS);
    let clock = c.drivers["pin:G2"];
    let mut chosen = ecp5_emu::fast::choose_options(
        &c.kernel,
        initial,
        clock,
        period,
        &probes,
        &|s| progress.borrow_mut().push(s),
        &|| false,
    )
    .unwrap();
    assert_eq!(progress.borrow().len(), 1);
    assert_eq!(
        chosen.program().source.as_ptr(),
        source,
        "reuse the generated program"
    );
    assert_eq!(
        chosen.program().options,
        options,
        "the UI must see the chosen options"
    );
    assert_eq!(chosen.now().unwrap(), c.kernel.now());
    let mut fresh = FastSim::with_options(&c.kernel, &options).unwrap();
    assert_eq!(chosen.raw_values(), fresh.raw_values());
    // A trial installs a clock: rewinding must remove it as well as rewind
    // registers, waves and PLLs, so the caller can start its own clock.
    chosen.clock(clock, period).unwrap();
    fresh.clock(clock, period).unwrap();
    assert_eq!(
        chosen.run_edges_sampled(12_000, &probes).unwrap(),
        fresh.run_edges_sampled(12_000, &probes).unwrap()
    );
    let stopped = ecp5_emu::fast::choose_options(
        &c.kernel,
        FastSim::new(&c.kernel).unwrap(),
        clock,
        period,
        &probes,
        &|_| panic!("a cancelled choice must not start a trial"),
        &|| true,
    );
    assert!(stopped.is_err());
}

#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn compiler_override_preserves_the_cli_probe_trace_and_reports_missing_paths() {
    use std::{os::unix::fs::PermissionsExt, process::Command};

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "ecp5 compiler override {}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let real_clang = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("clang"))
        .find(|path| {
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .expect("clang on PATH");
    let wrapper = scratch.0.join("compiler wrapper");
    let marker = scratch.0.join("invoked");
    let write_wrapper = |version| {
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then\n  printf '%s\\n' 'bundled clang version' '{version}'\n  exit 0\nfi\nprintf invoked > \"$ECP5_TEST_COMPILER_MARKER\"\nexec \"$ECP5_TEST_REAL_CLANG\" \"$@\"\n"
            ),
        )
        .unwrap();
    };
    write_wrapper("zig version one");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let design = reference::prepared("life");
    let run = |name: &str, compiler: Option<&Path>| {
        let tmp = scratch.0.join(name);
        std::fs::create_dir(&tmp).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
        command
            .arg("emulate")
            .arg(design.join("top.config"))
            .arg("--names")
            .arg(design.join("top_routed.json"))
            .args([
                "--fast",
                "--startup",
                "configured",
                "--scope",
                "observed",
                "--clock",
                "net:clk_25mhz",
                "--period-ns",
                "40",
                "--cycles",
                "4000",
                "--probe",
                "net:led",
            ])
            .arg("--vcd")
            .arg(tmp.join("probe.vcd"))
            .env("TMPDIR", &tmp)
            .env("FAST_CHOOSE", "0")
            .env("FAST_TUNE", "0")
            .env("ECP5_TEST_REAL_CLANG", &real_clang)
            .env("ECP5_TEST_COMPILER_MARKER", &marker)
            .env_remove("ECP5_CLANG");
        if let Some(compiler) = compiler {
            command.env("ECP5_CLANG", compiler);
        }
        command.output().unwrap()
    };
    for (name, compiler) in [("default", None), ("override", Some(wrapper.as_path()))] {
        let output = run(name, compiler);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(marker.is_file(), "the wrapper must actually run");
    let expected = std::fs::read(scratch.0.join("default/probe.vcd")).unwrap();
    assert!(!expected.is_empty());
    assert_eq!(
        expected,
        std::fs::read(scratch.0.join("override/probe.vcd")).unwrap()
    );
    let cache_name = |name: &str| {
        let libraries: Vec<_> = std::fs::read_dir(scratch.0.join(name).join("ecp5_emu_fast"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "so"))
            .collect();
        assert_eq!(libraries.len(), 1);
        libraries[0].file_name().unwrap().to_owned()
    };
    let original_name = cache_name("override");
    // Replace the compiler at the same path. Only its second version line changes.
    // Each run is a new process, with its own compiler identity cache.
    write_wrapper("zig version two");
    let output = run("upgraded", Some(&wrapper));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(original_name, cache_name("upgraded"));
    assert_eq!(
        expected,
        std::fs::read(scratch.0.join("upgraded/probe.vcd")).unwrap()
    );
    let missing = scratch.0.join("missing compiler");
    let output = run("missing", Some(&missing));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
        "fast mode: ECP5_CLANG={} is not an executable file",
        missing.display()
    )));
}

#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_and_runs_uart_hello_in_real_time() {
    let (mut c, probes) =
        build(reference::design("uart_hello").join("top.config")).compile(&["pin:L4"], &[]);
    let mut fast = fast(&c);
    let clock = c.drivers["pin:G2"];
    let cycles = 40_000;
    let mut event = Vec::new();
    for n in 1..=cycles {
        c.kernel.schedule(rise(n), clock, Logic::One).unwrap();
        c.kernel.schedule(fall(n), clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall(n)).unwrap();
        event.push(u8::from(c.kernel.value(probes[0]).unwrap() == Logic::One));
    }
    let samples = fast.run_cycles(cycles, probes[0]).unwrap();
    let first = samples.iter().zip(&event).position(|(a, b)| a != b);
    assert_eq!(
        first, None,
        "fast and event kernel differ from cycle {first:?}"
    );
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    assert!(decode_uart(&line).starts_with("Hello, ULX3S!\r\n"));
    // One second of board time at 25 MHz.
    timed(&mut fast, probes[0], "uart_hello");
}

/// A design with an input that changes while it runs: the debounced button
/// (examples/ulx3s/debounce): pressed with bounces, released, a pulse too
/// short to count, then pressed again. Both see the button change at the
/// same time, a quarter period before a rising edge.
/// Every LED must agree on every cycle.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_with_a_bouncing_button() {
    let leds = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"].map(|b| format!("pin:{b}"));
    let leds: Vec<&str> = leds.iter().map(String::as_str).collect();
    let (mut c, probes) =
        build(reference::design("debounce").join("top.config")).compile(&leds, &["pin:R1"]);
    let mut fast = fast(&c);
    let button = c.drivers["pin:R1"];
    let script = [
        (5_000, 0),
        (20_000, 2),
        (80_000, 1),
        (20_000, 2),
        (80_000, 0),
        (20_000, 1),
        (80_000, 0),
        (20_000, 2),
        (80_000, 1),
    ];
    let (mut n, mut level) = (0_u64, Logic::Zero);
    for (cycles, kind) in script {
        for i in 0..cycles {
            n += 1;
            let want = match kind {
                0 => Logic::Zero,
                1 => Logic::One,
                _ => Logic::from_bool((i / 100) % 2 == 0),
            };
            if want != level {
                let t = Time::from_ps((2 * n - 1) * PERIOD_PS / 2 - PERIOD_PS / 4);
                c.kernel.schedule(t, button, want).unwrap();
                fast.set_at(t, &[(button, want == Logic::One)]).unwrap();
                level = want;
            }
            cycle(&mut c, &mut fast, &probes, n, false);
        }
    }
    // Two real presses, one short pulse: the counter shows 2.
    let count: u32 = probes
        .iter()
        .enumerate()
        .map(|(k, &p)| u32::from(fast.value(p) == Some(Logic::One)) << k)
        .sum();
    assert_eq!(count, 2);
}

/// A user GSR driven by logic: `GSR(.GSR(~btn[1]))` over an 8-bit counter,
/// built here with Yosys and nextpnr. Fast mode takes the GSR of the system
/// controller from the button, as the event kernel does: the button is
/// pressed between clock edges, released, then pressed exactly at a rising
/// edge (the counter's own edge loses to the reset) and released at one.
/// Every LED agrees on every cycle.
#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fast_mode_follows_a_user_gsr_driven_by_logic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/fast-user-gsr");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("top.v"),
        "module top(input clk_25mhz, input [1:0] btn, output [7:0] led);\n  GSR global_reset(.GSR(~btn[1]));\n  reg [7:0] c = 0;\n  always @(posedge clk_25mhz) c <= c + 1;\n  assign led = c;\nendmodule\n",
    )
    .unwrap();
    let mut lpf = String::from("LOCATE COMP \"clk_25mhz\" SITE \"G2\";\nLOCATE COMP \"btn[1]\" SITE \"R1\";\nIOBUF PORT \"btn[1]\" PULLMODE=DOWN IO_TYPE=LVCMOS33;\nLOCATE COMP \"btn[0]\" SITE \"D6\";\n");
    for (k, ball) in ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"]
        .iter()
        .enumerate()
    {
        lpf.push_str(&format!("LOCATE COMP \"led[{k}]\" SITE \"{ball}\";\n"));
    }
    std::fs::write(dir.join("top.lpf"), lpf).unwrap();
    for (program, args) in [
        (
            "yosys",
            vec![
                "-q",
                "-p",
                "read_verilog top.v; synth_ecp5 -top top -json top.json",
            ],
        ),
        (
            "nextpnr-ecp5",
            vec![
                "--85k",
                "--package",
                "CABGA381",
                "--json",
                "top.json",
                "--lpf",
                "top.lpf",
                "--textcfg",
                "top.config",
                "--seed",
                "1",
                "-q",
            ],
        ),
    ] {
        let status = std::process::Command::new(program)
            .args(args)
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success(), "{program}");
    }
    let leds = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"].map(|b| format!("pin:{b}"));
    let leds: Vec<&str> = leds.iter().map(String::as_str).collect();
    let (mut c, probes) =
        build("target/fast-user-gsr/top.config").compile_driven(&leds, &[("pin:R1", Logic::Zero)]);
    let mut fast = fast(&c);
    let button = c.drivers["pin:R1"];
    // (cycle, level, at the rising edge rather than a quarter period before)
    let presses = [
        (40, true, false),
        (45, false, false),
        (90, true, true),
        (93, false, true),
    ];
    let value = |fast: &FastSim| -> u32 {
        probes
            .iter()
            .enumerate()
            .map(|(k, &p)| u32::from(fast.value(p) == Some(Logic::One)) << k)
            .sum()
    };
    for n in 1..=200 {
        for &(_, level, at_edge) in presses.iter().filter(|p| p.0 == n) {
            let t = if at_edge {
                rise(n)
            } else {
                Time::from_ps((2 * n - 1) * PERIOD_PS / 2 - PERIOD_PS / 4)
            };
            c.kernel
                .schedule(t, button, Logic::from_bool(level))
                .unwrap();
            fast.set_at(t, &[(button, level)]).unwrap();
        }
        cycle(&mut c, &mut fast, &probes, n, true);
        if n == 44 || n == 92 {
            assert_eq!(value(&fast), 0, "held in reset at cycle {n}");
        }
    }
    // Released at the rising edge of cycle 93, which already counts.
    assert_eq!(value(&fast), (200 - 92) as u32);
}

/// The PLL learns the reference period at its second rising edge and then
/// clocks the counter at 100 MHz: every counter bit agrees on every cycle,
/// from before the PLL starts. Then one second of board time is timed.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_through_a_pll() {
    let (mut c, probes) = build(reference::design("blinky_pll").join("top.config"))
        .compile(&["count", "led", "locked"], &[]);
    let mut fast = fast(&c);
    assert_eq!(fast.program().plls.len(), 1);
    for n in 1..=2_000 {
        cycle(&mut c, &mut fast, &probes, n, true);
    }
    segment(&mut c, &mut fast, &probes, 2_001, 8_192);
    let cycles = 11_192;
    for n in 10_193..=cycles {
        cycle(&mut c, &mut fast, &probes, n, true);
    }
    let count: u64 = probes[..32]
        .iter()
        .enumerate()
        .map(|(i, &p)| u64::from(fast.value(p) == Some(Logic::One)) << i)
        .sum();
    assert_eq!(count, 4 * cycles - 6);
    timed(&mut fast, probes[0], "blinky_pll");
}

/// ODDRX1F lanes and their LVCMOS33D complements, compared in both phases
/// of every clock cycle: D0 while the clock is high, D1 while it is low.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_on_ddr_outputs() {
    let pins = ["gpdi_dp", "pin:B16", "pin:C14", "pin:A13", "pin:B18"];
    let (mut c, probes) =
        build(reference::design("ddr_pattern").join("top.config")).compile(&pins, &[]);
    let mut fast = fast(&c);
    assert_eq!(fast.program().iologic, 4);
    for n in 1..=2_000 {
        cycle(&mut c, &mut fast, &probes, n, true);
    }
    timed(&mut fast, probes[0], "ddr_pattern");
}

/// HC06: the free-running OSCG (310 MHz / 16) clocks a counter that the
/// 25 MHz side samples over 65 536 cycles and prints over UART. The two
/// clocks are unrelated, so this checks the host's time keeping: the UART
/// line agrees on every cycle through the first report.
#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_with_the_oscillator() {
    let (mut c, probes) =
        build("docs/inventory/LFE5U-85F/hc06_osc/hc06_16.config").compile(&["pin:L4"], &[]);
    let mut fast = fast(&c);
    assert_eq!(fast.program().oscillators.len(), 1);
    let cycles = 0x18000 + 36 * 10 * board::BIT as u64;
    let mut line = vec![true];
    for n in 1..=2_000 {
        cycle(&mut c, &mut fast, &probes, n, false);
        line.push(fast.value(probes[0]) == Some(Logic::One));
    }
    let samples = segment(&mut c, &mut fast, &probes, 2_001, cycles - 3_000);
    line.extend(samples.iter().map(|&b| b == 1));
    for n in cycles - 999..=cycles {
        cycle(&mut c, &mut fast, &probes, n, false);
        line.push(fast.value(probes[0]) == Some(Logic::One));
    }
    let text = decode_uart(&line);
    assert!(text.contains("E="), "{text:?}");
    println!("HC06 report: {:?}", text.lines().find(|l| l.contains("E=")));
    timed(&mut fast, probes[0], "hc06");
}

/// A small xorshift generator, so the stimulus is the same on every run.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// The DP16KD fixtures of tests/ebr_native.rs (tests/ebr_fixture, built
/// here when missing): port A 18 bits WRITETHROUGH
/// without output register, port B 18 bits READBEFOREWRITE with OUTREG, CS
/// decoders, inverted CLKA/CEA/CSA0 (and WEA, or WEA tied low). The event
/// kernel runs until every latch is known, then fast mode takes over from its
/// state and both get the same random stimulus: every output agrees after
/// every instant. The stimulus keeps away from what the kernel makes X (byte
/// masks or a reset during a write, OCEB without access); last, writes with
/// half their byte mask show the kernel's X on the pins and stop fast mode
/// exactly when the X reaches a bit the logic reads.
fn ebr_fixture(dir: &str, inverted_write: bool, read_only_a: bool, steps: u64) {
    // A fresh checkout has no fixture yet: build it as tests/ebr_native.rs does.
    let built = ebr_fixture::ensure(inverted_write, read_only_a);
    assert!(built.ends_with(dir), "{dir} is not {}", built.display());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lpf = std::fs::read_to_string(root.join(dir).join("top.lpf")).unwrap();
    let ball: std::collections::BTreeMap<String, String> = lpf
        .lines()
        .filter_map(|l| l.strip_prefix("LOCATE COMP \""))
        .map(|l| {
            let (name, rest) = l.split_once('"').unwrap();
            (
                name.to_string(),
                rest.split('"').nth(1).unwrap().to_string(),
            )
        })
        .collect();
    let is_output = |n: &str| {
        n.starts_with("qa[") || n.starts_with("qb[") || n == "captured" || n == "wea_echo"
    };
    let outputs: Vec<String> = ball
        .keys()
        .filter(|n| is_output(n))
        .map(|n| format!("pin:{}", ball[n]))
        .collect();
    let inputs: Vec<String> = ball
        .keys()
        .filter(|n| !is_output(n) && n.as_str() != "clka")
        .map(|n| format!("pin:{}", ball[n]))
        .collect();
    let probes: Vec<&str> = outputs.iter().map(String::as_str).collect();
    let driven: Vec<&str> = inputs.iter().map(String::as_str).collect();
    let (mut c, signals) = build(format!("{dir}/top.config")).compile(&probes, &driven);
    let driver = |c: &Compiled, name: &str| c.drivers[&format!("pin:{}", ball[name])];
    let bus = |name: &str, width: usize| -> Vec<String> {
        (0..width).map(|i| format!("{name}[{i}]")).collect()
    };
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut level: std::collections::BTreeMap<String, bool> = ball
        .keys()
        .filter(|n| !is_output(n))
        .map(|n| (n.clone(), false))
        .collect();
    // New input values that keep the kernel two-valued.
    let draw = |random: &mut Random,
                level: &mut std::collections::BTreeMap<String, bool>,
                clock: Option<&str>| {
        let set_bus = |level: &mut std::collections::BTreeMap<String, bool>,
                       name: &str,
                       width: usize,
                       value: u64| {
            for (i, bit) in bus(name, width).into_iter().enumerate() {
                level.insert(bit, value >> i & 1 == 1);
            }
        };
        for side in ['a', 'b'] {
            let access = random.chance(70);
            let cs = if access || random.chance(50) {
                [2, 6][usize::from(side == 'b')]
            } else {
                random.next() % 8
            };
            let ce = access || random.chance(30);
            level.insert(format!("ce{side}"), if side == 'a' { !ce } else { ce });
            set_bus(level, &format!("cs{side}"), 3, cs);
            let access = ce && cs == [2, 6][usize::from(side == 'b')];
            let we = random.chance(40);
            let write = we && access && !(side == 'a' && read_only_a);
            level.insert(format!("we{side}"), we ^ (side == 'a' && inverted_write));
            level.insert(format!("rst{side}"), !write && random.chance(5));
            set_bus(
                level,
                &format!("be{side}"),
                2,
                if write { 3 } else { random.next() % 4 },
            );
            set_bus(level, &format!("ad{side}"), 10, random.next() % 16);
            set_bus(level, &format!("di{side}"), 18, random.next());
            let oce = if side == 'a' {
                true
            } else {
                access && random.chance(80)
            };
            level.insert(format!("oce{side}"), oce);
        }
        if let Some(clock) = clock {
            let old = level[clock];
            level.insert(clock.into(), !old);
        }
    };
    let mut time = 0_u64;
    let apply = |c: &mut Compiled,
                 fast: Option<&mut FastSim>,
                 level: &std::collections::BTreeMap<String, bool>,
                 time: u64|
     -> anyhow::Result<()> {
        let t = Time::from_ps(time * 1000);
        let updates: Vec<_> = level
            .iter()
            .map(|(n, &v)| (driver(c, n), Logic::from_bool(v)))
            .collect();
        c.kernel.schedule_batch(t, &updates, None).unwrap();
        c.kernel.run_until(t).unwrap();
        if let Some(fast) = fast {
            let inputs: Vec<_> = updates.iter().map(|&(d, v)| (d, v == Logic::One)).collect();
            fast.set_at(t, &inputs)?;
        }
        Ok(())
    };
    // The event kernel alone until fast mode can read every latch.
    let mut warm = 0;
    let mut fast = loop {
        match FastSim::new(&c.kernel) {
            Ok(fast) => break fast,
            Err(e)
                if e.downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                    .is_some()
                    && warm < 200 =>
            {
                warm += 1;
                time += 1;
                let clock = ["clka", "clkb"][warm % 2];
                draw(&mut random, &mut level, Some(clock));
                apply(&mut c, None, &level, time).unwrap();
            }
            Err(e) => panic!("{dir}: {e:#}"),
        }
    };
    println!(
        "EBR {dir}: fast mode from instant {warm}, {} EBR",
        fast.program().ebrs.len()
    );
    assert_eq!(fast.program().ebrs.len(), 1);
    for step in 0..steps {
        time += 1;
        let clock = match random.next() % 4 {
            0 => Some("clka"),
            1 => Some("clkb"),
            _ => None,
        };
        draw(&mut random, &mut level, clock);
        apply(&mut c, Some(&mut fast), &level, time)
            .unwrap_or_else(|e| panic!("{dir} step {step}: {e:#}"));
        for (k, &p) in signals.iter().enumerate() {
            assert_eq!(
                fast.value(p),
                Some(c.kernel.value(p).unwrap()),
                "{dir} step {step}: {}",
                outputs[k]
            );
        }
    }
    if read_only_a {
        return;
    }
    // A write through port A with byte group 0 masked: WRITETHROUGH merges
    // the latch, the memory and the data of that group, which the kernel
    // makes X where they differ. Fast mode shows the same X on the pins, and
    // must stop once the X is in qa[0], which the `captured` flip-flop reads.
    let qa0 = outputs
        .iter()
        .position(|o| *o == format!("pin:{}", ball["qa[0]"]))
        .unwrap();
    for attempt in 0..8 {
        for (name, value) in [
            ("cea", false),
            ("wea", !inverted_write),
            ("rsta", false),
            ("ocea", true),
        ] {
            level.insert(name.into(), value);
        }
        for (i, b) in bus("csa", 3).into_iter().enumerate() {
            level.insert(b, 2 >> i & 1 == 1);
        }
        for (i, b) in bus("bea", 2).into_iter().enumerate() {
            level.insert(b, i == 1);
        }
        for (i, b) in bus("dia", 18).into_iter().enumerate() {
            level.insert(b, (i + attempt) % 2 == 0);
        }
        // To a falling CLKA (the inverted clock's rising edge).
        level.insert("clka".into(), true);
        time += 1;
        apply(&mut c, Some(&mut fast), &level, time).unwrap();
        level.insert("clka".into(), false);
        time += 1;
        let result = apply(&mut c, Some(&mut fast), &level, time);
        let read_x = c.kernel.value(signals[qa0]).unwrap().bit().is_none();
        match result {
            Err(e) => {
                assert!(read_x, "fast mode stopped but qa[0] is known: {e:#}");
                assert!(format!("{e:#}").contains("EBR"), "{e:#}");
                println!("EBR {dir}: stopped where the logic reads the kernel's X: {e:#}");
                return;
            }
            Ok(()) => {
                assert!(!read_x, "qa[0] is X in the kernel but fast mode went on");
                for (k, &p) in signals.iter().enumerate() {
                    assert_eq!(
                        fast.value(p),
                        Some(c.kernel.value(p).unwrap()),
                        "{dir} masked write: {}",
                        outputs[k]
                    );
                }
            }
        }
    }
    panic!("{dir}: no masked write made qa[0] unknown");
}

#[test]
#[ignore = "requires clang, yosys and nextpnr-ecp5 (to build the EBR fixtures when target/ lacks them), the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_on_dual_port_ebr() {
    ebr_fixture("target/ebr-execution", false, false, 20_000);
    ebr_fixture("target/ebr-inverted-wea", true, false, 20_000);
    ebr_fixture("target/ebr-readonly-a", false, true, 20_000);
}

/// uart_ram: the message in an EBR ROM, copied into an EBR RAM and sent from
/// there. The RAM's read latch is X until the copy is done, so the event
/// kernel runs until it is known; fast mode then takes over from its state,
/// and the UART line agrees on every cycle through the whole message.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_takes_over_from_the_event_kernel_on_uart_ram() {
    let (mut c, probes) =
        build(reference::design("uart_ram").join("top.config")).compile(&["pin:L4"], &[]);
    let (mut fast, n, mut line) = take_over(&mut c, probes[0], "uart_ram");
    assert_eq!(fast.program().ebrs.len(), 2);
    let samples = segment(
        &mut c,
        &mut fast,
        &probes,
        n + 1,
        24 * 10 * board::BIT as u64,
    );
    line.extend(samples.iter().map(|&b| b == 1));
    let text = decode_uart(&line);
    assert!(text.starts_with("Hello from block RAM!\r\n"), "{text:?}");
    timed(&mut fast, probes[0], "uart_ram");
}

/// The event kernel runs cycles until fast mode can start from its state
/// (every value it reads is known), then fast mode takes the 25 MHz clock.
/// Returns it, the number of cycles the kernel ran, and the levels of `line`
/// from time 0 after each of those cycles.
fn take_over(c: &mut Compiled, line: SignalId, name: &str) -> (FastSim, u64, Vec<bool>) {
    let clock = c.drivers["pin:G2"];
    let mut levels = vec![c.kernel.value(line).unwrap() == Logic::One];
    let mut n = 0;
    let mut fast = loop {
        match FastSim::new(&c.kernel) {
            Ok(fast) => break fast,
            Err(e)
                if e.downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                    .is_some()
                    && n < 1000 =>
            {
                n += 1;
                c.kernel.schedule(rise(n), clock, Logic::One).unwrap();
                c.kernel.schedule(fall(n), clock, Logic::Zero).unwrap();
                c.kernel.run_until(fall(n)).unwrap();
                levels.push(c.kernel.value(line).unwrap() == Logic::One);
            }
            Err(e) => panic!("{e:#}"),
        }
    };
    println!("{name}: the event kernel ran {n} cycles first");
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    (fast, n, levels)
}

/// What fast mode keeps up to date after every clock edge (`fresh`): each
/// such net of Life, sampled after every edge, has the value a full settle
/// gives it then. Some nets are not (folded into a reader's table, or
/// evaluated on demand), and watching one makes it so.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fresh_nets_are_up_to_date_after_every_edge() {
    let b = build(reference::design("life").join("top.config"));
    let (mut c, _) = b.compile(&["led"], &[]);
    let mut signals: Vec<SignalId> = b
        .names
        .as_ref()
        .unwrap()
        .resolved()
        .filter_map(|(_, r)| match r {
            Reference::Wire(w) => c.kernel.signal(w),
            _ => None,
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    signals.retain(|s| seen.insert(*s));
    let line = signals[0];
    let (mut f, _, _) = take_over(&mut c, line, "life");
    let (fresh, stale): (Vec<SignalId>, Vec<SignalId>) =
        signals.iter().partition(|&&s| f.program().fresh(s));
    println!(
        "life: {} named nets, {} up to date after every edge, {} not",
        signals.len(),
        fresh.len(),
        stale.len()
    );
    assert!(fresh.len() > 1000 && !stale.is_empty());
    let byte = |v: Option<Logic>| match v {
        Some(Logic::Zero) => 0,
        Some(Logic::One) => 1,
        Some(Logic::Z) => 3,
        _ => 2,
    };
    let mut changed = 0;
    for edge in 0..400 {
        let row = f.run_edges_sampled(1, &fresh).unwrap();
        f.step().unwrap();
        for (k, &s) in fresh.iter().enumerate() {
            assert_eq!(row[k], byte(f.value(s)), "edge {edge}: {s:?}");
        }
        changed += row.iter().filter(|&&v| v == 1).count();
    }
    assert!(changed > 0, "the sampled nets never were 1");
    // A watched net stays computed.
    let s = stale[0];
    c.kernel.watch(s).unwrap();
    let watched = FastSim::new(&c.kernel).unwrap();
    assert!(watched.program().fresh(s));
}

/// picorv32_console: the stock PicoRV32 (RV32I), its register file in LUTRAM
/// and its firmware in four EBR. After the kernel's warm start both agree on
/// every cycle from the clearing of .bss into the sieve: the program counter,
/// the ALU output, the store data read from the register file, the LEDs and
/// the UART line. Fast mode alone then reaches the console. Its text is that
/// of an RTL simulation of top.v (iverilog), down to the cycle count the
/// firmware measures, and a line sent to the receiver comes back in capitals.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn fast_mode_runs_the_picorv32_console() {
    let (mut c, probes) = build(reference::design("picorv32_console").join("top.config"))
        .compile_driven(
            &[
                "pin:L4",
                "led",
                "cpu.reg_pc",
                "cpu.reg_out",
                "cpu.dbg_mem_wdata",
            ],
            &[("pin:M1", Logic::One)],
        );
    let (mut fast, n, mut line) = take_over(&mut c, probes[0], "picorv32_console");
    let p = fast.program();
    assert_eq!((p.lutrams.len(), p.ebrs.len()), (128, 4));
    let compared = 20_000;
    let samples = segment(&mut c, &mut fast, &probes, n + 1, compared);
    line.extend(samples.iter().map(|&b| b == 1));
    // To the prompt, about 311000 cycles in.
    let cycles = 320_000 - n - compared;
    let start = Instant::now();
    let samples = fast.run_cycles(cycles, probes[0]).unwrap();
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "FAST picorv32_console: {cycles} cycles of 25 MHz in {seconds:.3} s = {:.2} MHz",
        cycles as f64 / seconds / 1e6
    );
    line.extend(samples.iter().map(|&b| b == 1));
    let banner = "PicoRV32 on ECP5\r\nprimes below 2000: 303, 151493 cycles\r\n> ";
    assert_eq!(decode_uart(&line), banner);
    // "hi" and Enter at 115200 baud, while the echo is sampled.
    let rx = c.drivers["pin:M1"];
    for byte in b"hi\r" {
        let frame = std::iter::once(false)
            .chain((0..8).map(|k| byte >> k & 1 == 1))
            .chain(std::iter::once(true));
        for level in frame {
            fast.set_at(fast.now().unwrap(), &[(rx, level)]).unwrap();
            let samples = fast.run_cycles(board::BIT as u64, probes[0]).unwrap();
            line.extend(samples.iter().map(|&b| b == 1));
        }
    }
    let samples = fast.run_cycles(20_000, probes[0]).unwrap();
    line.extend(samples.iter().map(|&b| b == 1));
    assert_eq!(decode_uart(&line), format!("{banner}HI\r\n> "));
}

/// Build the gallery demo with SERV's stock 2-bit register file. Its same-clock
/// EBR read/write used to stop fast mode at cycle 32, during reset.
#[test]
#[ignore = "requires clang, yosys, nextpnr-ecp5, the native image reader and a local Trellis database"]
fn fast_mode_runs_stock_serv_rf_top_past_the_former_collision_stop() {
    use sha2::{Digest, Sha256};

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demo = root.join("examples/gallery/serv_shell");
    // Hash names and contents, following the gallery's Verilog symlinks.
    // Changed or removed inputs get a fresh directory, including builds
    // made before the demo switched to serv_rf_top itself.
    let mut inputs: Vec<_> = std::fs::read_dir(&demo)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(path.extension().and_then(|e| e.to_str()), Some("v" | "hex"))
                || matches!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some("top.lpf" | "ecpbram.txt")
                )
        })
        .collect();
    inputs.extend(["examples/ulx3s/build.sh", "examples/ulx3s/swap.sh"].map(|p| root.join(p)));
    inputs.sort();
    let mut hash = Sha256::new();
    for path in inputs {
        let contents = std::fs::read(&path).unwrap();
        hash.update(
            path.strip_prefix(&root)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes(),
        );
        hash.update([0]);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    let hash: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dir = format!("target/serv-shell-hb09-{hash}");
    let out = root.join(&dir);
    if ["top.config", "top_routed.json"]
        .iter()
        .any(|p| !out.join(p).is_file())
    {
        std::fs::create_dir_all(&out).unwrap();
        println!("stock serv_rf_top: building the gallery in {dir}");
        ebr_fixture::run(
            &root,
            "examples/ulx3s/build.sh",
            &[demo.to_str().unwrap(), out.to_str().unwrap()],
            &format!("{dir}/build.log"),
        );
    }
    let (mut c, probes) = build(format!("{dir}/top.config")).compile_driven(
        &[
            "pin:L4",
            "led",
            "cpu.rf_ram.rdata",
            "cpu.rf_ram.i_raddr",
            "cpu.cpu.wb_ibus_adr",
        ],
        &[("pin:M1", Logic::One), ("pin:D6", Logic::One)],
    );
    assert!(c.report.ebr.iter().any(|ebr| ebr.widths == [2, 2]));
    let (mut fast, warm, mut line) = take_over(&mut c, probes[0], "stock serv_rf_top");
    assert!(warm < 32, "must exercise the former stop in fast mode");
    let compared = 20_000;
    let samples = segment(&mut c, &mut fast, &probes, warm + 1, compared);
    line.extend(samples.iter().map(|&b| b == 1));
    let samples = fast
        .run_cycles(1_000_000 - warm - compared, probes[0])
        .unwrap();
    line.extend(samples.iter().map(|&b| b == 1));
    let text = decode_uart(&line);
    assert!(
        text.starts_with("RISC-V shell on ULX3S\r\nSERV RV32I"),
        "{text:?}"
    );
    println!("stock serv_rf_top: all {} probes match the kernel for {compared} cycles; fast mode reaches cycle 1000000 and prints the SERV banner", probes.len());
}

/// `emulate --fast --console-out/--console-in`: the CLI prints what the
/// firmware sends and sends what is written to its stdin. A line typed after
/// the prompt comes back in capitals, and the status lines stay on stderr.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn the_cli_console_talks_to_the_picorv32_firmware() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    let build = reference::design("picorv32_console");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("emulate")
        .arg(build.join("top.config"))
        .arg("--names")
        .arg(build.join("top_routed.json"))
        .args([
            "--fast",
            "--startup",
            "configured",
            "--scope",
            "observed",
            "--clock",
            "net:clk_25mhz",
            "--period-ns",
            "40",
            "--cycles",
            "50000000",
            "--console-out",
            "net:ftdi_rxd",
            "--console-in",
            "net:ftdi_txd",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (send, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 256];
        while let Ok(n @ 1..) = stdout.read(&mut buffer) {
            if send.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut text = Vec::new();
    let mut until = |marker: &str| {
        while !String::from_utf8_lossy(&text).contains(marker) {
            match received.recv_timeout(Duration::from_secs(60)) {
                Ok(bytes) => text.extend(bytes),
                Err(_) => panic!("no {marker:?} in {:?}", String::from_utf8_lossy(&text)),
            }
        }
    };
    until("> ");
    child.stdin.as_mut().unwrap().write_all(b"hi\r").unwrap();
    until("HI\r\n> ");
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&text),
        "PicoRV32 on ECP5\r\nprimes below 2000: 303, 151493 cycles\r\n> HI\r\n> "
    );
}

/// A firmware swapped in with `examples/ulx3s/swap.sh` (ecpbram over the
/// placed-and-routed placeholder build) runs without place and route, and
/// fast mode reuses the program compiled for the earlier firmware: its C
/// source is the same. The new image is the committed one with another banner
/// of the same length, so no RISC-V compiler is needed.
#[test]
#[ignore = "requires clang, ecpbram and ecppack, the reference build in target/reference, the native image reader and a local Trellis database"]
fn a_firmware_swapped_with_ecpbram_runs_without_place_and_route() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = root.join("examples/ulx3s/picorv32_console");
    let reference = reference::design("picorv32_console");
    // Include the process id so concurrent test runs do not collide on this scratch directory.
    let scratch = std::env::temp_dir().join(format!("ecp5_emu_swap_test_{}", std::process::id()));
    let (images, out) = (scratch.join("design"), scratch.join("out"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&images).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    // The image as bytes: byte k is in lane k % 4, word k / 4.
    let lanes: Vec<Vec<u8>> = (0..4)
        .map(|lane| {
            std::fs::read_to_string(design.join(format!("fw{lane}.hex")))
                .unwrap()
                .lines()
                .map(|l| u8::from_str_radix(l.trim(), 16).unwrap())
                .collect()
        })
        .collect();
    let mut image: Vec<u8> = (0..4 * lanes[0].len())
        .map(|k| lanes[k % 4][k / 4])
        .collect();
    // clang peels the first character of the banner into an immediate, so the
    // new text keeps it.
    let (old, new) = (b"PicoRV32 on ECP5", b"PicoRV32 swapped");
    let at = image.windows(old.len()).position(|w| w == old).unwrap();
    image[at..at + new.len()].copy_from_slice(new);
    for lane in 0..4 {
        let text: String = image
            .iter()
            .skip(lane)
            .step_by(4)
            .map(|b| format!("{b:02x}\n"))
            .collect();
        std::fs::write(images.join(format!("fw{lane}.hex")), text).unwrap();
    }
    std::fs::copy(design.join("ecpbram.txt"), images.join("ecpbram.txt")).unwrap();
    for file in ["top_placeholder.config", "top_routed.json"]
        .into_iter()
        .map(String::from)
        .chain((0..4).map(|lane| format!("placeholder_fw{lane}.hex")))
    {
        std::fs::copy(reference.join(&file), out.join(&file)).unwrap();
    }
    let swap = std::process::Command::new(root.join("examples/ulx3s/swap.sh"))
        .arg(&images)
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        swap.status.success(),
        "{}",
        String::from_utf8_lossy(&swap.stderr)
    );
    let run = |config: &std::path::Path| {
        let (mut c, probes) =
            build(config.to_str().unwrap()).compile_driven(&["pin:L4"], &[("pin:M1", Logic::One)]);
        let (mut fast, n, mut line) = take_over(&mut c, probes[0], "picorv32_console");
        let samples = fast.run_cycles(320_000 - n, probes[0]).unwrap();
        line.extend(samples.iter().map(|&b| b == 1));
        (fast.program().source.clone(), decode_uart(&line))
    };
    let (swapped_source, text) = run(&out.join("top.config"));
    assert_eq!(
        text,
        "PicoRV32 swapped\r\nprimes below 2000: 303, 151493 cycles\r\n> "
    );
    let (source, _) = run(&reference.join("top.config"));
    assert!(
        swapped_source == source,
        "the C program changed with the firmware"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// An unfolded program on two threads must give the same Life trace as one
/// thread. On rserv, clang 21 with AVX-512 miscompiled adjacent 32-bit LUT
/// shifts and gave a wrong trace; the LUT constants are now 64-bit.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn no_folding_on_threads_gives_the_trace_of_one_thread() {
    let (c, probes) =
        build(reference::design("life").join("top.config")).compile_driven(&["led"], &[]);
    let samples = [1, 2].map(|threads| {
        let options = ecp5_emu::kernel::FastOptions {
            threads,
            thread_min: 1,
            collapse: 0,
            ..ecp5_emu::kernel::FastOptions::from_env()
        };
        let mut fast = FastSim::with_options(&c.kernel, &options).unwrap();
        fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        fast.run_cycles_sampled(20_000, &probes).unwrap()
    });
    let first = samples[0].iter().zip(&samples[1]).position(|(a, b)| a != b);
    assert_eq!(first, None, "Life: the samples differ from row {first:?}");
}

/// dlopen shares one program's C statics between simulations. Keep a solo
/// instance loaded while two threads run that exact source at different
/// cycle offsets: EBR capture/commit and the shared worker pool must both
/// preserve each simulation's trace.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn concurrent_simulations_of_one_program_match_solo_traces() {
    // Inlining can eliminate the old shared scratch and hide the race.
    // Also test without it, in a child so parallel tests keep their flags.
    let flags = std::env::var("FAST_CFLAGS").unwrap_or_else(|_| "-O3".into());
    if !flags.split_whitespace().any(|f| f == "-fno-inline") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "concurrent_simulations_of_one_program_match_solo_traces",
                "--include-ignored",
                "--nocapture",
            ])
            .env("FAST_CFLAGS", format!("{flags} -fno-inline"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "concurrent run without C inlining:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (name, probes, inputs, threads) in [
        (
            "picorv32_console",
            &["pin:L4", "led", "cpu.reg_pc", "cpu.reg_out"][..],
            &[("pin:M1", Logic::One)][..],
            4,
        ),
        ("life", &["led"][..], &[][..], 4),
    ] {
        let (mut c, probes) =
            build(reference::design(name).join("top.config")).compile_driven(probes, inputs);
        let (warm, _, _) = take_over(&mut c, probes[0], name);
        drop(warm);
        let options = ecp5_emu::kernel::FastOptions {
            threads,
            thread_min: 1,
            ..ecp5_emu::kernel::FastOptions::from_env()
        };
        let clock = c.drivers["pin:G2"];
        let mut solo = FastSim::with_options(&c.kernel, &options).unwrap();
        solo.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
        if name == "picorv32_console" {
            assert!(!solo.program().ebrs.is_empty());
        } else {
            assert!(solo.program().settle_phases.iter().any(|&n| n > 0));
        }
        let start = solo.snapshot();
        let cycles = 100_000;
        let expected = [0, 137].map(|offset| {
            solo.restore(&start);
            solo.run_cycles_sampled(offset, &probes).unwrap();
            solo.run_cycles_sampled(cycles, &probes).unwrap()
        });
        let barrier = std::sync::Barrier::new(2);
        let traces = std::thread::scope(|scope| {
            let workers: Vec<_> = [0, 137]
                .into_iter()
                .map(|offset| {
                    let (kernel, probes, barrier, source) =
                        (&c.kernel, &probes, &barrier, &solo.program().source);
                    scope.spawn(move || {
                        let mut sim = FastSim::with_options(kernel, &options).unwrap();
                        assert_eq!(&sim.program().source, source);
                        sim.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
                        let start = sim.snapshot();
                        let mut traces = Vec::new();
                        for _ in 0..3 {
                            sim.restore(&start);
                            let prepared = sim.run_cycles_sampled(offset, probes);
                            barrier.wait();
                            traces.push(
                                prepared.and_then(|_| sim.run_cycles_sampled(cycles, probes)),
                            );
                        }
                        traces
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        for (worker, runs) in traces.into_iter().enumerate() {
            for (round, trace) in runs.into_iter().enumerate() {
                let trace = trace.unwrap();
                let first = trace
                    .iter()
                    .zip(&expected[worker])
                    .position(|(a, b)| a != b);
                assert_eq!(
                    first, None,
                    "{name}: worker {worker}, round {round} differs at sample {first:?}"
                );
            }
        }
        println!("{name}: both concurrent traces match solo runs in all three rounds");
    }
}

/// `FastSim::tune` changes the program a run uses as it goes (other thread
/// counts, folding widths): the trace must not depend on it. Each design runs
/// once as built and once going to the next of every program tune builds at
/// every 97 cycles or edges. Every sample, the EBR arrays and the LUTRAM
/// cells must agree, and so must the run after going back to a snapshot
/// taken under another program.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn a_run_changing_program_gives_the_same_trace() {
    struct Case {
        name: &'static str,
        probes: &'static [&'static str],
        inputs: &'static [(&'static str, Logic)],
        cycles: u64,
        every_edge: bool,
    }
    let cases = [
        Case {
            name: "life",
            probes: &["led"],
            inputs: &[],
            cycles: 20_000,
            every_edge: false,
        },
        Case {
            name: "picorv32_console",
            probes: &["pin:L4", "led", "cpu.reg_pc", "cpu.reg_out"],
            inputs: &[("pin:M1", Logic::One)],
            cycles: 30_000,
            every_edge: false,
        },
        Case {
            name: "uart_ram",
            probes: &["pin:L4"],
            inputs: &[],
            cycles: 30_000,
            every_edge: false,
        },
        Case {
            name: "blinky_pll",
            probes: &["count", "led", "locked"],
            inputs: &[],
            cycles: 6_000,
            every_edge: false,
        },
        Case {
            name: "ddr_pattern",
            probes: &["gpdi_dp", "pin:B16", "pin:C14", "pin:A13", "pin:B18"],
            inputs: &[],
            cycles: 6_000,
            every_edge: true,
        },
        Case {
            name: "uart_hello",
            probes: &["pin:L4"],
            inputs: &[],
            cycles: 20_000,
            every_edge: false,
        },
    ];
    for case in cases {
        let name = case.name;
        let (mut c, probes) = build(reference::design(name).join("top.config"))
            .compile_driven(case.probes, case.inputs);
        let (mut plain, _, _) = take_over(&mut c, probes[0], name);
        let mut tuned = FastSim::new(&c.kernel).unwrap();
        tuned
            .clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        let programs = tuned.tune_for_checks(&c.kernel).unwrap();
        println!("{name}: {programs} programs take turns");
        assert!(programs >= 2, "{name}: only one program");
        let run = |fast: &mut FastSim, n: u64| {
            if case.every_edge {
                fast.run_edges_sampled(2 * n, &probes).unwrap()
            } else {
                fast.run_cycles_sampled(n, &probes).unwrap()
            }
        };
        let state = |fast: &FastSim| -> Vec<Vec<u8>> {
            let p = fast.program();
            (0..p.ebrs.len())
                .map(|b| fast.memory(b).unwrap().to_vec())
                .chain((0..p.lutrams.len()).map(|w| fast.lutram(w).unwrap().to_vec()))
                .collect()
        };
        let (a, b) = (run(&mut plain, case.cycles), run(&mut tuned, case.cycles));
        let first = a.iter().zip(&b).position(|(x, y)| x != y);
        assert_eq!(first, None, "{name}: the samples differ from row {first:?}");
        // A bulk read, made under one program and used after others: as
        // `value` signal by signal.
        let signals: Vec<Option<SignalId>> = probes.iter().copied().map(Some).collect();
        let mut reader = ecp5_emu::fast::Reader::new(&signals);
        let mut read = Vec::new();
        for _ in 0..3 {
            tuned.read(&mut reader, &mut read);
            let one_by_one: Vec<u8> = probes
                .iter()
                .map(|&p| match tuned.value(p) {
                    Some(Logic::Zero) => 0,
                    Some(Logic::One) => 1,
                    _ => 2,
                })
                .collect();
            assert_eq!(read, one_by_one, "{name}: a bulk read differs");
            run(&mut tuned, 97);
            run(&mut plain, 97);
        }
        assert!(
            state(&plain) == state(&tuned),
            "{name}: EBR or LUTRAM differ"
        );
        // A snapshot taken now, under one program, restored later under
        // another: the same run again.
        let snapshot = tuned.snapshot();
        let later = run(&mut tuned, 1_001);
        tuned.restore(&snapshot);
        let again = run(&mut tuned, 1_001);
        assert_eq!(later, again, "{name}: the run after restoring differs");
        assert_eq!(
            run(&mut plain, 1_001),
            again,
            "{name}: the run after restoring differs from the plain one"
        );
        assert!(
            state(&plain) == state(&tuned),
            "{name}: EBR or LUTRAM differ after restoring"
        );
    }
}

#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn split_build_compiles_on_another_thread_and_matches_with_options() {
    let b = build(reference::design("blinky_cycle").join("top.config"));
    let (mut c, probes) = b.compile(&["led"], &[]);
    let (_, _, _) = take_over(&mut c, probes[0], "blinky_cycle");
    let options = ecp5_emu::kernel::FastOptions::from_env();
    let mut direct = FastSim::with_options(&c.kernel, &options).unwrap();
    let prepared = FastSim::prepare(&c.kernel, &options).unwrap();
    let (prepared, library) = std::thread::spawn(move || {
        let library = prepared.compile(true).unwrap();
        (prepared, library)
    })
    .join()
    .unwrap();
    let mut split = FastSim::from_prepared(prepared, &library).unwrap();
    let period = Time::from_ps(PERIOD_PS);
    direct.clock(c.drivers["pin:G2"], period).unwrap();
    split.clock(c.drivers["pin:G2"], period).unwrap();
    assert_eq!(
        direct.run_edges_sampled(200, &probes).unwrap(),
        split.run_edges_sampled(200, &probes).unwrap()
    );
}

/// R01: inferred 16x1 uses only one of the four physical DPR16X4 columns.
/// Build the RTL through the board toolchain, then compare every q row before
/// and after writes and on read-only cycles. X02 uses all four columns.
#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang, the native image reader and a local Trellis database"]
fn inferred_narrow_and_full_width_lutram_match_the_event_kernel() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (name, width) in [("narrow", 1), ("wide", 4)] {
        let config = format!("target/fast-lutram-{name}/top.config");
        ebr_fixture::run(
            &root,
            "sh",
            &[
                "examples/ulx3s/build.sh",
                &format!("tests/lutram_fixture/{name}"),
                &format!("target/fast-lutram-{name}"),
            ],
            &format!("target/fast-lutram-{name}.log"),
        );
        let address = ["pin:E8", "pin:D8", "pin:D7", "pin:E7"];
        let data = &["pin:D6", "pin:T1", "pin:R18", "pin:V1"][..width];
        let outputs = &["pin:B2", "pin:C2", "pin:C1", "pin:D2"][..width];
        let inputs: Vec<_> = std::iter::once(("pin:R1", Logic::One))
            .chain(address.iter().map(|&p| (p, Logic::Zero)))
            .chain(data.iter().map(|&p| (p, Logic::One)))
            .collect();
        let design = build(config);
        let (mut c, probes) = design.compile_driven(outputs, &inputs);
        assert_eq!(c.report.counts.lutram_columns, 4);
        let mut fast = fast(&c);
        let mut expected = [0usize; 16];
        let mut random = 0x9e37_79b9_7f4a_7c15u64;
        let mut rows = 0;
        for n in 1..=512 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            // Start with the audit's exact WE=1, A=0, D=1 trigger.
            let (we, a, d) = if n == 1 {
                (true, 0, 1)
            } else {
                (
                    random & 1 != 0,
                    (random >> 1) as usize & 15,
                    (random >> 5) as usize & ((1 << width) - 1),
                )
            };
            let updates: Vec<_> = std::iter::once((c.drivers["pin:R1"], we))
                .chain(
                    address
                        .iter()
                        .enumerate()
                        .map(|(k, p)| (c.drivers[*p], a & (1 << k) != 0)),
                )
                .chain(
                    data.iter()
                        .enumerate()
                        .map(|(k, p)| (c.drivers[*p], d & (1 << k) != 0)),
                )
                .collect();
            let at = Time::from_ps((n - 1) * PERIOD_PS + PERIOD_PS / 4);
            let logic: Vec<_> = updates
                .iter()
                .map(|&(p, v)| (p, Logic::from_bool(v)))
                .collect();
            c.kernel.schedule_batch(at, &logic, None).unwrap();
            fast.set_at(at, &updates).unwrap();
            agree(
                &mut c,
                &mut fast,
                &probes,
                at,
                "asynchronous read before write",
            );
            let value = |c: &Compiled| {
                probes.iter().enumerate().fold(0, |v, (k, &p)| {
                    v | (usize::from(c.kernel.value(p).unwrap() == Logic::One) << k)
                })
            };
            assert_eq!(value(&c), expected[a], "{name} cycle {n} before edge");
            if we {
                expected[a] = d;
            }
            cycle(&mut c, &mut fast, &probes, n, true);
            assert_eq!(value(&c), expected[a], "{name} cycle {n} after edge");
            rows += 3;
        }
        // The event kernel has also written the floating, unused columns;
        // their cells are now unknown and must not prevent a later handover.
        let resumed = FastSim::new(&c.kernel).unwrap();
        for &q in &probes {
            assert_eq!(resumed.value(q), Some(c.kernel.value(q).unwrap()));
        }
        println!("R01 {name}: 512 cycles, {rows} identical q rows ({width} bits), WE/A/D varied; warm handover agrees");
    }
}

/// Compare default generated C byte for byte across revisions without clang.
#[test]
#[ignore = "requires rserv reference builds; DCS_BASELINE selects write/check"]
fn dcs_free_programs_are_unchanged() {
    let Ok(mode) = std::env::var("DCS_BASELINE") else {
        return;
    };
    let directory = PathBuf::from(std::env::var_os("DCS_BASELINE_DIR").unwrap());
    std::fs::create_dir_all(&directory).unwrap();
    for name in ["life", "life_full"] {
        let (c, _) = build(reference::design(name).join("top.config")).compile(&["led"], &[]);
        let source = c.kernel.fast_program().unwrap().source;
        let path = directory.join(format!("{name}.c"));
        match mode.as_str() {
            "write" => std::fs::write(&path, &source).unwrap(),
            "check" => assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes()),
            _ => panic!("DCS_BASELINE must be write or check"),
        }
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&source)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        println!("{name}: {} bytes sha256 {hash}; {mode}", source.len());
    }
}

/// Keep a source baseline outside the checkout and compare the hot-loop timing.
#[test]
#[ignore = "requires rserv reference builds and clang; DTR_BASELINE selects write/check"]
fn dtr_free_programs_and_cost() {
    let mode = std::env::var("DTR_BASELINE").unwrap_or_else(|_| "report".into());
    assert!(["write", "check", "report"].contains(&mode.as_str()));
    let directory = std::env::var_os("DTR_BASELINE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("ecp5-dtr-baseline"));
    if mode == "write" {
        std::fs::create_dir_all(&directory).unwrap();
    }
    for name in ["life", "picorv32_console"] {
        let (mut c, probes) = build(reference::design(name).join("top.config")).compile_driven(
            &["led"],
            if name == "life" {
                &[]
            } else {
                &[("pin:M1", Logic::One)]
            },
        );
        let (mut sim, _, _) = take_over(&mut c, probes[0], name);
        assert!(sim.program().dtrs.is_empty());
        let source = &sim.program().source;
        let path = directory.join(format!("{name}.c"));
        if mode == "write" {
            std::fs::write(&path, source).unwrap();
        } else if mode == "check" {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *source);
        }
        use sha2::{Digest, Sha256};
        println!(
            "{name}: {} bytes sha256 {}; {mode}",
            source.len(),
            Sha256::digest(source)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let state = sim.snapshot();
        let mut times = Vec::new();
        for _ in 0..7 {
            sim.restore(&state);
            let started = Instant::now();
            sim.run_cycles(200_000, probes[0]).unwrap();
            times.push(started.elapsed().as_secs_f64());
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{name}: median {:.6} s, {:.3} MHz",
            times[3],
            0.2 / times[3]
        );
    }
}
