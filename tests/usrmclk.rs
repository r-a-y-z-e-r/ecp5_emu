//! Native USRMCLK/OSCG configurations: the dedicated CCLK pad, its package
//! query, and the event/fast traces, including the OSC.DIV/CCLK.MODE overlay.
mod board;
use board::{pin, PERIOD_PS};
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup, NOMINAL_OSCILLATOR_HZ},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::{DeviceModel, Direction},
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{collections::BTreeSet, path::PathBuf, process::Command, sync::Arc};

fn design(name: &str, usrmclk: bool, divider: u64) -> Arc<ConfiguredDevice> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let primitive = if usrmclk {
        "USRMCLK u(.USRMCLKI(c[1]), .USRMCLKTS(c[4]));"
    } else {
        ""
    };
    std::fs::write(
        dir.join("top.v"),
        format!(
            "module top(input clk_25mhz, output [2:0] led);
  reg [4:0] c = 0;
  always @(posedge clk_25mhz) c <= c + 1;
  {primitive}
  wire osc;
  OSCG #(.DIV({divider})) o(.OSC(osc));
  assign led = {{osc, c[4], c[1]}};
endmodule
"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("top.lpf"),
        "LOCATE COMP \"clk_25mhz\" SITE \"G2\";
IOBUF PORT \"clk_25mhz\" PULLMODE=NONE IO_TYPE=LVCMOS33;
LOCATE COMP \"led[0]\" SITE \"B2\";
LOCATE COMP \"led[1]\" SITE \"C2\";
LOCATE COMP \"led[2]\" SITE \"C1\";
",
    )
    .unwrap();
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
        println!("{name}: running {program}");
        let status = Command::new(program)
            .args(args)
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success(), "{program}");
    }
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(ConfigurationImage::load(&dir.join("top.config"), &db, None).unwrap());
    Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    )
}

fn compile(device: Arc<ConfiguredDevice>, probes: Vec<Reference>) -> Compiled {
    compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(pin("G2"), Logic::Zero)],
            probes,
            options: Options::default(),
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"))
}

fn check_packages(device: &Arc<ConfiguredDevice>, active: bool) {
    // Official pinout/BSDL, preserved by the architecture's package table.
    for (package, ball) in [
        ("CABGA381", "U3"),
        ("CABGA554", "AE3"),
        ("CABGA756", "AM3"),
        ("CSFBGA285", "U16"),
    ] {
        let pad = compile::cclk_pad(device, package).unwrap();
        assert_eq!(pad.ball, ball);
        assert_eq!(pad.direction, active.then_some(Direction::Output));
        assert!(device
            .model()
            .package_pin(package, ball)
            .unwrap()
            .pio
            .is_none());
        // Each package reference resolves to the same die pad.
        let c = compile::compile(
            Arc::clone(device),
            Request {
                package: package.into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: vec![],
                probes: vec![pin(ball)],
                options: Options::default(),
            },
        );
        assert!(c.is_ok(), "{package}: {:?}", c.err());
    }
    assert!(compile::cclk_pad(device, "unknown-package").is_err());
}

fn check_overlay(device: &ConfiguredDevice, shared: bool, active: bool) {
    assert_eq!(device.image().cram().bit(508, 1135), shared);
    assert_eq!(device.image().cram().bit(720, 1135), active);
    let id = device
        .model()
        .resource_id("LFE5U-85F/R95C0/USRMCLK")
        .unwrap();
    let raw = &device.resource_settings(id).unwrap().fields[0];
    assert_eq!(raw.name, "CCLK.MODE");
    // The raw composed view remains evidence, even when the false replica
    // conflicts; execution must use EFB3 rather than changing either bit.
    assert_eq!(
        raw.status,
        if shared == active {
            "resolved"
        } else {
            "unmatched"
        }
    );
    assert_eq!(device.image().cram().bit(508, 1135), shared);
}

fn check_trace(divider: u64) {
    let device = design(&format!("usrmclk-div{divider}"), true, divider);
    check_overlay(&device, divider == 16, true);
    check_packages(&device, true);
    let raw = |name: &str| Reference::parse(device.model(), &format!("R95C0:{name}")).unwrap();
    let probes = vec![
        pin("U3"),
        pin("B2"),
        pin("C2"),
        pin("C1"),
        raw("JPADDO_CCLK"),
        raw("JPADDT_CCLK"),
    ];
    let mut c = compile(Arc::clone(&device), probes.clone());
    assert!(c.report.resources.iter().any(|r| r.family == "USRMCLK"));
    assert!(c.report.resources.iter().any(|r| r.family == "OSCG"));
    let signals: Vec<_> = probes.iter().map(|p| c.probes[&p.label()]).collect();
    c.kernel.run_until(Time::ZERO).unwrap();
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    const CYCLES: u64 = 128;
    let end = Time::from_ps(CYCLES * PERIOD_PS);
    let half = Time::new(divider * 1_000_000_000_000, 2 * NOMINAL_OSCILLATOR_HZ).unwrap();
    let mut instants = BTreeSet::from([Time::ZERO]);
    for n in 1..=2 * CYCLES {
        let time = Time::from_ps(n * PERIOD_PS / 2);
        c.kernel
            .schedule(time, clock, Logic::from_bool(n % 2 == 1))
            .unwrap();
        instants.insert(time);
    }
    let mut osc_edges = 1;
    while half.checked_mul(osc_edges).unwrap() <= end {
        instants.insert(half.checked_mul(osc_edges).unwrap());
        osc_edges += 1;
    }
    let mut states = [0; 3];
    for time in instants {
        c.kernel.run_until(time).unwrap();
        fast.run_until(time).unwrap();
        let (num, den) = time.ratio();
        let rises = (num / den + PERIOD_PS / 2) / PERIOD_PS;
        let data = Logic::from_bool((rises >> 1) & 1 != 0);
        let ts = Logic::from_bool((rises >> 4) & 1 != 0);
        let pad = if ts == Logic::One { Logic::Z } else { data };
        let osc = Logic::from_bool(
            (u128::from(num) * u128::from(2 * NOMINAL_OSCILLATOR_HZ)
                / (u128::from(den) * u128::from(divider * 1_000_000_000_000)))
                % 2
                == 1,
        );
        let expected = [pad, data, ts, osc, data, ts];
        for ((probe, &signal), want) in probes.iter().zip(&signals).zip(expected) {
            assert_eq!(
                c.kernel.value(signal).unwrap(),
                want,
                "{} at {time}",
                probe.label()
            );
            assert_eq!(
                fast.value(signal),
                Some(want),
                "fast {} at {time}",
                probe.label()
            );
        }
        states[match pad {
            Logic::Zero => 0,
            Logic::One => 1,
            Logic::Z => 2,
            Logic::X => unreachable!(),
        }] += 1;
    }
    assert!(states.iter().all(|&n| n > 10), "0/1/Z samples: {states:?}");
    check_overlay(&device, divider == 16, true);
    println!(
        "DIV={divider}: {CYCLES} cycles, every reference/OSCG edge agrees (0/1/Z: {states:?})"
    );
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn usrmclk_and_oscg_div16_match_in_event_and_fast_modes() {
    check_trace(16);
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn usrmclk_and_oscg_div128_keep_the_shared_bit_independent() {
    check_trace(128);
}

#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn oscg_without_usrmclk_leaves_cclk_undriven() {
    let device = design("usrmclk-none", false, 16);
    check_overlay(&device, true, false);
    check_packages(&device, false);
    let mut c = compile(Arc::clone(&device), vec![pin("U3"), pin("C1")]);
    assert!(c.report.undriven.iter().any(|w| w.name == "PAD_CCLK"));
    assert!(!c
        .report
        .resources
        .iter()
        .any(|r| r.family == "PIO" && r.id.contains("R95C0")));
    c.kernel.run_until(Time::from_ps(1_000_000)).unwrap();
    assert_eq!(c.value(&pin("U3")).unwrap(), Logic::Z);
    check_overlay(&device, true, false);
}
