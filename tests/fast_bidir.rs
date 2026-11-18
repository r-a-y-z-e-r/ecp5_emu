//! Bidirectional and tristate pads in fast mode against the event kernel:
//! a design built here with Yosys and nextpnr drives
//! - `gp[0]` open-drain (`c[2] ? 1'bz : 1'b0`) with a pull-up,
//! - `gp[1]` tristate (`c[3] ? c[0] : 1'bz`) with a pull-down,
//! - `gp[2]` tristate with no pull, which only a probe watches,
//!
//! and reads `gp[0]` and `gp[1]` back into a shift register on the LEDs.
//! Nothing outside drives the pads. Every LED and pad agrees on every cycle,
//! a floating pad as Z in both.
mod board;
use board::PERIOD_PS;
use ecp5_emu::{
    compile::{self, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, process::Command, sync::Arc};

const LEDS: [&str; 8] = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];
const PADS: [&str; 3] = ["B11", "A10", "A9"];

/// Builds `top.v` with `top.lpf` in target/<dir> and compiles it with the
/// clock on G2 and every LED and pad probed.
fn design(dir: &str, verilog: &str, lpf: &str) -> (Compiled, Vec<SignalId>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("top.v"), verilog).unwrap();
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
                "--write",
                "top_routed.json",
                "--seed",
                "1",
                "-q",
            ],
        ),
    ] {
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
    let _names = Names::load(&dir.join("top_routed.json"), &model, "CABGA381").unwrap();
    let image = Arc::new(ConfigurationImage::load(&dir.join("top.config"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let probes: Vec<_> = LEDS.iter().chain(&PADS).map(|b| board::pin(b)).collect();
    let c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(board::pin("G2"), Logic::Zero)],
            probes: probes.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    let signals = probes.iter().map(|r| c.probes[&r.label()]).collect();
    (c, signals)
}

fn lpf(pulls: [&str; 3]) -> String {
    let mut lpf = String::from(
        "LOCATE COMP \"clk_25mhz\" SITE \"G2\";\nIOBUF PORT \"clk_25mhz\" PULLMODE=NONE IO_TYPE=LVCMOS33;\n",
    );
    for (k, ball) in LEDS.iter().enumerate() {
        lpf.push_str(&format!("LOCATE COMP \"led[{k}]\" SITE \"{ball}\";\n"));
    }
    for (k, (ball, pull)) in PADS.iter().zip(pulls).enumerate() {
        lpf.push_str(&format!(
            "LOCATE COMP \"gp[{k}]\" SITE \"{ball}\";\nIOBUF PORT \"gp[{k}]\" PULLMODE={pull} IO_TYPE=LVCMOS33;\n"
        ));
    }
    lpf
}

const VERILOG: &str = "module top(input clk_25mhz, inout [2:0] gp, output [7:0] led);
  reg [7:0] c = 0;
  always @(posedge clk_25mhz) c <= c + 1;
  assign gp[0] = c[2] ? 1'bz : 1'b0;
  assign gp[1] = c[3] ? c[0] : 1'bz;
  assign gp[2] = c[4] ? c[1] : 1'bz;
  reg [7:0] seen = 0;
  always @(posedge clk_25mhz) seen <= {seen[5:0], gp[1], gp[0]};
  assign led = seen;
endmodule
";

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_on_bidirectional_pads() {
    let (mut c, probes) = design("fast-bidir", VERILOG, &lpf(["UP", "DOWN", "NONE"]));
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut floating = 0;
    for n in 1..=300u64 {
        let rise = Time::from_ps((2 * n - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(n * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        fast.run_until(fall).unwrap();
        for (k, &p) in probes.iter().enumerate() {
            assert_eq!(
                fast.value(p),
                Some(c.kernel.value(p).unwrap()),
                "probe {k} after cycle {n}"
            );
        }
        floating += usize::from(c.kernel.value(probes[10]).unwrap() == Logic::Z);
    }
    // gp[2] floats half of the time, and both executors show Z then.
    assert!(floating > 100, "{floating}");
}

/// A pad that can float with no pull, read by the logic: the kernel makes
/// the read X while nothing drives it, which two-valued fast mode cannot
/// carry, so it refuses with the reason.
#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fast_mode_refuses_a_floating_pad_that_the_logic_reads() {
    let (c, _) = design("fast-bidir-floating", VERILOG, &lpf(["UP", "NONE", "NONE"]));
    let error = FastSim::new(&c.kernel).err().expect("refused");
    assert!(format!("{error:#}").contains("no pull"), "{error:#}");
}
