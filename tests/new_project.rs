//! Generated projects built with the real toolchain and run from top.config.
mod board;
use board::{decode_uart, pin, run_uart, BIT, PERIOD_PS};
use ecp5_emu::{
    board::{self as ulx3s, Direction},
    compile::{self, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    flow::{
        self,
        new_project::{self, Options as NewOptions, Resource},
        BuildOptions, Project, Tools,
    },
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "new project test {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn resource_ports_match_created_hdl_and_default_lpf() {
    use new_project::{Direction, Naming, Port};

    let folder = Folder::new();
    for resource in Resource::ALL {
        let mut options = NewOptions::new(folder.0.join(format!("{resource:?}")));
        options.resources = [Resource::Clock, resource].into();
        if matches!(
            resource,
            Resource::Clock | Resource::Buttons | Resource::Switches
        ) {
            options.resources.insert(Resource::Leds);
        }
        let project = new_project::create(&options).unwrap();
        let top = fs::read_to_string(project.root.join("top.v")).unwrap();
        let declarations = top
            .split_once("module top(")
            .unwrap()
            .1
            .split_once(");")
            .unwrap()
            .0;
        let mut declared = Vec::new();
        for declaration in declarations.split(',') {
            let declaration = declaration.trim();
            let declaration = declaration
                .strip_prefix("(* keep *)")
                .unwrap_or(declaration)
                .trim();
            let (direction, signal) = declaration.split_once(' ').unwrap();
            let direction = match direction {
                "input" => Direction::Input,
                "output" => Direction::Output,
                "inout" => Direction::Inout,
                _ => panic!("unexpected declaration: {declaration}"),
            };
            let (name, width) = if let Some(range) = signal.strip_prefix('[') {
                let (range, name) = range.split_once(']').unwrap();
                let (high, low) = range.split_once(':').unwrap();
                let width = high.parse::<usize>().unwrap() - low.parse::<usize>().unwrap() + 1;
                (name.trim(), Some(width))
            } else {
                (signal.trim().trim_start_matches('\\'), None)
            };
            // create() requires clock and, for the initial recipes, LEDs.
            // Compare only the selected resource's declarations, in order.
            if (name == "clk_25mhz" && resource != Resource::Clock)
                || (name == "led" && resource != Resource::Leds)
            {
                continue;
            }
            declared.push(Port {
                name: name.into(),
                width,
                direction,
            });
        }
        assert_eq!(new_project::ports(resource), declared, "{resource:?}");
        let mut expected_names = Vec::new();
        for port in declared {
            match port.width {
                Some(width) => {
                    expected_names.extend((0..width).map(|i| format!("{}[{i}]", port.name)))
                }
                None => expected_names.push(port.name),
            }
        }
        expected_names.sort();
        let lpf = new_project::lpf_lines(resource, Naming::Default).unwrap();
        // Both commands must cover every scalar and bus bit exactly once.
        for command in ["LOCATE COMP ", "IOBUF PORT "] {
            let mut actual_names: Vec<_> = lpf
                .lines()
                .filter(|line| line.starts_with(command))
                .map(|line| line.split('"').nth(1).unwrap().to_owned())
                .collect();
            actual_names.sort();
            assert_eq!(actual_names, expected_names, "{resource:?}: {command}");
        }
        for line in lpf.lines() {
            assert!(
                expected_names
                    .iter()
                    .any(|name| Some(name.as_str()) == line.split('"').nth(1)),
                "{resource:?}: {line}"
            );
        }
    }
}

fn build(project: &Project) -> flow::Outcome {
    let options = BuildOptions {
        tools: Tools::from_env(),
        ..BuildOptions::default()
    };
    let started = std::time::Instant::now();
    let outcome = flow::build(project, &options, &mut |_| {});
    eprintln!(
        "build {}: {:.3}s",
        project.name,
        started.elapsed().as_secs_f64()
    );
    assert!(
        outcome.ok,
        "build {}: {:?}",
        project.root.display(),
        outcome.messages
    );
    outcome
}

struct Design {
    compiled: Compiled,
    names: Names,
    fast: FastSim,
    startup: Vec<bool>,
}

fn compile(project: &Project, outcome: &flow::Outcome, resources: &BTreeSet<Resource>) -> Design {
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let names = Names::load(&outcome.outputs.routed_json, &model, ulx3s::PACKAGE).unwrap();
    let image = Arc::new(ConfigurationImage::load(&outcome.outputs.config, &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let has = |r| resources.contains(&r);
    let mut expected = vec![(ulx3s::CLOCK, Direction::Input, None)];
    if has(Resource::Leds) {
        expected.extend(ulx3s::LEDS.map(|b| (b, Direction::Output, None)));
    }
    if has(Resource::Buttons) {
        expected.extend(
            ulx3s::BUTTONS
                .iter()
                .map(|b| (b.ball, Direction::Input, Some(b.idle))),
        );
    }
    if has(Resource::Switches) {
        expected.extend(ulx3s::SWITCHES.map(|b| (b, Direction::Input, Some(false))));
    }
    if has(Resource::Uart) {
        expected.extend([
            (ulx3s::UART_TX, Direction::Output, Some(true)),
            (ulx3s::UART_RX, Direction::Input, Some(true)),
        ]);
    }
    assert_eq!(ulx3s::used_balls(&device).unwrap().len(), expected.len());
    for &(ball, direction, pull) in &expected {
        assert_eq!(
            ulx3s::direction(&device, ball).unwrap(),
            Some(direction),
            "{ball} direction"
        );
        assert_eq!(
            ulx3s::input_pull(&device, ball).unwrap(),
            pull,
            "{ball} pull"
        );
    }
    let mut probes: Vec<_> = expected
        .iter()
        .filter(|(_, d, _)| d.drives())
        .map(|(b, _, _)| pin(b))
        .collect();
    if has(Resource::Leds) {
        probes.extend(names.expand("count").unwrap().into_iter().map(|(_, r)| r));
    }
    let mut compiled = compile::compile(
        device.clone(),
        Request {
            package: ulx3s::PACKAGE.into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: std::iter::once((ulx3s::CLOCK, false))
                .chain(ulx3s::idle_inputs(&device, [false; 4]).unwrap())
                .map(|(ball, value)| (pin(ball), Logic::from_bool(value)))
                .collect(),
            probes,
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("{}: {e:#}", project.root.display()));
    // Preserve the initial level for the startup start-bit edge. Brief kernel
    // warmup also settles configured state before fast mode takes over.
    let sample = if has(Resource::Uart) {
        ulx3s::UART_TX
    } else {
        ulx3s::LEDS[0]
    };
    let mut startup = vec![compiled.value(&pin(sample)).unwrap().bit().unwrap()];
    startup.extend(run_uart(&mut compiled, 8, sample));
    let mut fast = FastSim::new(&compiled.kernel).unwrap();
    fast.clock(compiled.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    Design {
        compiled,
        names,
        fast,
        startup,
    }
}

impl Design {
    fn level(&self, ball: &str) -> bool {
        self.fast
            .value(self.compiled.probes[&format!("pin:{ball}")])
            .unwrap()
            .bit()
            .unwrap()
    }
    fn leds(&self) -> u8 {
        ulx3s::LEDS
            .iter()
            .enumerate()
            .fold(0, |n, (i, b)| n | (u8::from(self.level(b)) << i))
    }
    fn count(&self) -> u64 {
        self.names
            .expand("count")
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, (_, r))| {
                u64::from(
                    self.fast
                        .value(self.compiled.probes[&r.label()])
                        .unwrap()
                        .bit()
                        .unwrap(),
                ) << i
            })
            .sum()
    }
    fn cycles(&mut self, n: u64) {
        self.fast.run_cycles_sampled(n, &[]).unwrap();
    }
    fn set(&mut self, ball: &str, level: bool) {
        self.fast
            .set(self.compiled.drivers[&format!("pin:{ball}")], level)
            .unwrap();
    }
    fn uart(&mut self, n: u64) -> Vec<bool> {
        self.fast
            .run_cycles(n, self.compiled.probes[&format!("pin:{}", ulx3s::UART_TX)])
            .unwrap()
            .into_iter()
            .map(|v| {
                assert!(v <= 1, "UART must be a known level");
                v == 1
            })
            .collect()
    }
    fn buttons(&mut self) {
        assert_eq!(self.leds() & 1, 0, "released buttons, including PWR");
        for b in &ulx3s::BUTTONS {
            self.set(b.ball, !b.idle);
            self.cycles(1);
            assert_eq!(
                self.leds() & 1,
                0,
                "{} needs two synchronizer stages",
                b.name
            );
            self.cycles(2);
            assert_eq!(self.leds() & 1, 1, "{} pressed", b.name);
            self.set(b.ball, b.idle);
            self.cycles(3);
            assert_eq!(self.leds() & 1, 0, "{} released", b.name);
        }
    }
    fn switches(&mut self) {
        assert_eq!(self.leds() & 0x1e, 0);
        for pattern in 0..16 {
            for (i, ball) in ulx3s::SWITCHES.iter().enumerate() {
                self.set(ball, pattern & (1 << i) != 0);
            }
            self.cycles(3);
            assert_eq!(
                (self.leds() >> 1) & 15,
                pattern,
                "switch pattern {pattern:04b}"
            );
        }
        for ball in ulx3s::SWITCHES {
            self.set(ball, false);
        }
        self.cycles(3);
        assert_eq!(self.leds() & 0x1e, 0);
    }
    fn greetings(&mut self) {
        const HELLO: &str = "Hello, ULX3S!\r\n";
        let duration = (15 * 10 * BIT + 500) as u64;
        let mut samples = std::mem::take(&mut self.startup);
        samples.extend(self.uart(duration));
        assert_eq!(
            decode_uart(&samples),
            HELLO,
            "startup greeting at 40 ns / 115200 8N1"
        );
        let mut requested = vec![self.level(ulx3s::UART_TX)];
        self.set(ulx3s::UART_RX, false);
        requested.extend(self.uart(1000));
        // Another falling edge while transmitting must not restart a frame
        // or leave a deferred partial greeting after the current one.
        self.set(ulx3s::UART_RX, true);
        requested.extend(self.uart(4));
        self.set(ulx3s::UART_RX, false);
        requested.extend(self.uart(duration));
        assert_eq!(
            decode_uart(&requested),
            HELLO,
            "idle RX edge requests a complete greeting"
        );
        self.set(ulx3s::UART_RX, true);
        let mut periodic = vec![self.level(ulx3s::UART_TX)];
        periodic.extend(self.uart(2_500_000 + duration));
        assert_eq!(decode_uart(&periodic), HELLO, "periodic greeting");
    }
    fn counter(&mut self, resources: &BTreeSet<Resource>) {
        let before = self.count();
        let cycles = 1 << 25;
        self.cycles(cycles);
        let after = self.count();
        assert_eq!(after, (before + cycles) & ((1 << 28) - 1));
        assert_ne!(after >> 20, before >> 20, "real divider advances LEDs");
        let mut expected = (after >> 20) as u8;
        if resources.contains(&Resource::Buttons) {
            expected &= !1;
        }
        if resources.contains(&Resource::Switches) {
            expected &= !0x1e;
        }
        assert_eq!(self.leds(), expected);
    }
}

fn acceptance(optional: &[Resource], leds: bool, moved: bool) {
    let folder = Folder::new();
    let original = folder.0.join("original parent").join("generated project");
    let mut options = NewOptions::new(&original);
    if !leds {
        options.resources.remove(&Resource::Leds);
    }
    options.resources.extend(optional);
    let mut project = new_project::create(&options).unwrap();
    if moved {
        let destination = folder.0.join("different parent").join("relocated project");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(&original, &destination).unwrap();
        fs::remove_dir(original.parent().unwrap()).unwrap();
        project = Project::open(&destination).unwrap();
    }
    let outcome = build(&project);
    let mut design = compile(&project, &outcome, &options.resources);
    if leds {
        assert_eq!(design.count(), 8);
    }
    if optional.contains(&Resource::Uart) {
        design.greetings();
    }
    if optional.contains(&Resource::Buttons) {
        design.buttons();
    }
    if optional.contains(&Resource::Switches) {
        design.switches();
    }
    if leds {
        design.counter(&options.resources);
    }
    if moved {
        let again = build(&project);
        assert!(again
            .steps
            .iter()
            .all(|(_, status)| *status == flow::StepStatus::Skipped));
    }
}

#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn default_counter() {
    acceptance(&[], true, false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn buttons_including_pwr() {
    acceptance(&[Resource::Buttons], true, false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn all_switches() {
    acceptance(&[Resource::Switches], true, false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn uart_startup_periodic_and_rx() {
    acceptance(&[Resource::Uart], true, false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn uart_without_leds() {
    acceptance(&[Resource::Uart], false, false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn initial_resources() {
    acceptance(
        &[Resource::Buttons, Resource::Switches, Resource::Uart],
        true,
        false,
    );
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, the native image reader and a local Trellis database"]
fn relocated_folder_builds_and_reuses_cache() {
    acceptance(&[], true, true);
}

// CLI coverage also exercises the actual executable and the printed next steps.
#[test]
fn cli_creates_only_and_has_command_help() {
    let folder = Folder::new();
    let destination = folder.0.join("CLI 'quoted' project");
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("new")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(&destination.display().to_string()));
    assert!(text.contains("ecp5_emu build "));
    assert!(text.contains("ecp5_emu run "));
    let project = Project::open(&destination).unwrap();
    assert_eq!(project.name, "CLI 'quoted' project");
    assert!(!project.build_dir.exists());
    for flag in ["--help", "-h"] {
        let help = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
            .args(["new", flag])
            .output()
            .unwrap();
        assert!(help.status.success());
        assert!(String::from_utf8_lossy(&help.stdout).contains("--from-demo"));
    }
    let copied = folder.0.join("demo copy");
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("new")
        .arg(&copied)
        .args(["--from-demo", "ulx3s/uart_hello"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(Project::open(&copied).unwrap().name, "demo copy");
    let refused = folder.0.join("mixed demo resources");
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("new")
        .arg(&refused)
        .args([
            "--from-demo",
            "ulx3s/uart_hello",
            "--resources",
            "clock,leds",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutually exclusive"));
    assert!(!refused.exists());
}

// Peripheral starters hold the external devices idle, without running protocols.
fn peripheral_project(resources: &[Resource]) {
    let folder = Folder::new();
    let mut options = NewOptions::new(folder.0.join("peripheral starter"));
    options.name = format!("{:?}", resources);
    options.resources = std::iter::once(Resource::Clock)
        .chain(resources.iter().copied())
        .collect();
    let project = new_project::create(&options).unwrap();
    let outcome = build(&project);
    if resources.len() > 1 {
        emulate_idle_peripherals(&outcome);
    }
}

fn emulate_idle_peripherals(outcome: &flow::Outcome) {
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(ConfigurationImage::load(&outcome.outputs.config, &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let mut idle = vec![
        (ulx3s::OLED_CLK, false),
        (ulx3s::OLED_MOSI, false),
        (ulx3s::OLED_DC, false),
        (ulx3s::OLED_RESN, false),
        (ulx3s::OLED_CSN, true),
        (ulx3s::SD_CLK, false),
        (ulx3s::SD_CMD, true),
        (ulx3s::SD_D[3], true),
        (ulx3s::SDRAM_CLK, false),
        (ulx3s::SDRAM_CKE, false),
        (ulx3s::SDRAM_CSN, true),
        (ulx3s::SDRAM_RASN, true),
        (ulx3s::SDRAM_CASN, true),
        (ulx3s::SDRAM_WEN, true),
        (ulx3s::ADC_CSN, true),
        (ulx3s::ADC_SCLK, false),
        (ulx3s::ADC_MOSI, false),
        (ulx3s::FLASH_SCK, false),
        (ulx3s::FLASH_CSN, true),
        (ulx3s::FLASH_MOSI, true),
        (ulx3s::FLASH_WPN, true),
        (ulx3s::FLASH_HOLDN, true),
    ];
    for balls in [
        ulx3s::GPDI_DP.as_slice(),
        ulx3s::AUDIO_L.as_slice(),
        ulx3s::AUDIO_R.as_slice(),
        ulx3s::AUDIO_V.as_slice(),
        ulx3s::SDRAM_BA.as_slice(),
        ulx3s::SDRAM_A.as_slice(),
    ] {
        idle.extend(balls.iter().map(|ball| (*ball, false)));
    }
    idle.extend(ulx3s::GPDI_DN.map(|ball| (ball, true)));
    idle.extend(ulx3s::SDRAM_DQM.map(|ball| (ball, true)));
    for &(ball, _) in &idle {
        assert_eq!(
            ulx3s::direction(&device, ball).unwrap(),
            Some(Direction::Output),
            "{ball}"
        );
    }
    // A released bus must accept either external level. This also proves that
    // no SDRAM data or RTC pad is being driven by the generated starter.
    let released: Vec<_> = [ulx3s::RTC_SCL, ulx3s::RTC_SDA]
        .into_iter()
        .chain(ulx3s::SDRAM_D)
        .collect();
    for ball in &released {
        assert!(
            ulx3s::direction(&device, ball)
                .unwrap()
                .is_some_and(Direction::receives),
            "{ball} must read the released bus"
        );
    }
    for pin in ulx3s::HEADER {
        assert_eq!(
            ulx3s::direction(&device, pin.ball).unwrap(),
            Some(Direction::Input),
            "{} must never drive",
            pin.name
        );
    }
    let used = ulx3s::used_balls(&device).unwrap();
    let drives = used
        .iter()
        .filter(|(_, direction)| direction.receives())
        .map(|(ball, _)| {
            let value = if ball == ulx3s::CLOCK {
                false
            } else if let Some(button) = ulx3s::BUTTONS.iter().find(|b| b.ball == ball) {
                button.idle
            } else {
                ulx3s::input_pull(&device, ball).unwrap().unwrap_or(false)
            };
            (pin(ball), Logic::from_bool(value))
        })
        .collect();
    let mut compiled = compile::compile(
        device,
        Request {
            package: ulx3s::PACKAGE.into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives,
            probes: idle
                .iter()
                .map(|(ball, _)| pin(ball))
                .chain(released.iter().map(|ball| pin(ball)))
                .collect(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap();
    let clock = compiled.drivers["pin:G2"];
    for edge in 1..=1024 {
        let time = Time::from_ps(edge * PERIOD_PS / 2);
        if edge == 257 || edge == 513 {
            for ball in &released {
                compiled
                    .kernel
                    .schedule(
                        time,
                        compiled.drivers[&format!("pin:{ball}")],
                        Logic::from_bool(edge == 257),
                    )
                    .unwrap();
            }
        }
        compiled
            .kernel
            .schedule(time, clock, Logic::from_bool(edge % 2 == 1))
            .unwrap();
        compiled.kernel.run_until(time).unwrap();
        for &(ball, expected) in &idle {
            assert_eq!(
                compiled.value(&pin(ball)).unwrap(),
                Logic::from_bool(expected),
                "{ball} edge {edge}"
            );
        }
        if edge >= 257 {
            for ball in &released {
                assert_eq!(
                    compiled.value(&pin(ball)).unwrap(),
                    Logic::from_bool(edge < 513),
                    "released {ball} edge {edge}"
                );
            }
        }
    }
    eprintln!("all resources: idle outputs and released buses held for 512 cycles");
}

#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack"]
fn all_resources() {
    peripheral_project(&Resource::ALL);
}

macro_rules! peripheral_build {
    ($name:ident, $resource:ident) => {
        #[test]
        #[ignore = "requires yosys, nextpnr-ecp5, ecppack"]
        fn $name() {
            peripheral_project(&[Resource::$resource]);
        }
    };
}
peripheral_build!(oled_alone, Oled);
peripheral_build!(hdmi_alone, Hdmi);
peripheral_build!(sd_alone, Sd);
peripheral_build!(audio_alone, Audio);
peripheral_build!(sdram_alone, Sdram);
peripheral_build!(adc_alone, Adc);
peripheral_build!(rtc_alone, Rtc);
peripheral_build!(flash_alone, Flash);
peripheral_build!(header_alone, Header);

#[test]
fn cli_parses_every_peripheral_name_and_lists_them_in_help() {
    let folder = Folder::new();
    let help = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .args(["new", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for name in [
        "oled", "hdmi", "sd", "audio", "sdram", "adc", "rtc", "flash", "header",
    ] {
        assert!(help.contains(name), "help for {name}");
        let destination = folder.0.join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
            .arg("new")
            .arg(&destination)
            .arg("--resources")
            .arg(format!("clock,{name}"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(Project::open(destination).is_ok());
    }
}
