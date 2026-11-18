//! Tristate (inout) pads driven from outside in fast mode against the event
//! kernel: a design built here with Yosys and nextpnr has two inout pads:
//! - `gp[0]` (`oe0 ? d0 : 1'bz`) with PULLMODE=UP,
//! - `gp[1]` (`oe1 ? d1 : 1'bz`) with PULLMODE=DOWN,
//!
//! driven by a 16-bit LFSR (with `oe` low for stretches of cycles), and reads
//! both pads back into an 8-bit shift register on the LEDs.
//!
//! Each pad is compiled with an external driver starting at `Logic::Z`.
//! 1. Over 20 000 cycles, while `oe` is low, a seeded random schedule sets the
//!    external drivers to 0, 1, or Z (which shows the pull); every LED and both
//!    pads agree between the event kernel and fast mode after every clock edge.
//! 2. Driving an external level opposite to the design while `oe` is high makes
//!    the event kernel put X on the pad and makes fast mode stop with error 4
//!    at that time.
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
const PADS: [&str; 2] = ["B11", "A10"];

const VERILOG: &str = "module top(input clk_25mhz, inout [1:0] gp, output [7:0] led);
  reg [15:0] lfsr = 16'hACE1;
  always @(posedge clk_25mhz)
    lfsr <= {lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]};

  wire oe0 = lfsr[15] & lfsr[14];
  wire oe1 = lfsr[11] & lfsr[10];
  assign gp[0] = oe0 ? lfsr[0] : 1'bz;
  assign gp[1] = oe1 ? lfsr[1] : 1'bz;

  reg [7:0] seen = 0;
  always @(posedge clk_25mhz)
    seen <= {seen[5:0], gp[1], gp[0]};
  assign led = seen;
endmodule
";

fn lpf() -> String {
    let mut lpf = String::from(
        "LOCATE COMP \"clk_25mhz\" SITE \"G2\";\nIOBUF PORT \"clk_25mhz\" PULLMODE=NONE IO_TYPE=LVCMOS33;\n",
    );
    for (k, ball) in LEDS.iter().enumerate() {
        lpf.push_str(&format!("LOCATE COMP \"led[{k}]\" SITE \"{ball}\";\n"));
    }
    for (k, (ball, pull)) in PADS.iter().zip(["UP", "DOWN"]).enumerate() {
        lpf.push_str(&format!(
            "LOCATE COMP \"gp[{k}]\" SITE \"{ball}\";\nIOBUF PORT \"gp[{k}]\" PULLMODE={pull} IO_TYPE=LVCMOS33;\n"
        ));
    }
    lpf
}

fn design(dir: &str) -> (Compiled, Vec<SignalId>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("top.v"), VERILOG).unwrap();
    std::fs::write(dir.join("top.lpf"), lpf()).unwrap();
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
            drives: vec![
                (board::pin("G2"), Logic::Zero),
                (board::pin(PADS[0]), Logic::Z),
                (board::pin(PADS[1]), Logic::Z),
            ],
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

fn step_lfsr(lfsr: u16) -> u16 {
    let fb = ((lfsr >> 15) ^ (lfsr >> 13) ^ (lfsr >> 12) ^ (lfsr >> 10)) & 1;
    (lfsr << 1) | fb
}

fn oe_of(lfsr: u16) -> [bool; 2] {
    [
        ((lfsr >> 15) & (lfsr >> 14) & 1) == 1,
        ((lfsr >> 11) & (lfsr >> 10) & 1) == 1,
    ]
}

fn d_of(lfsr: u16) -> [bool; 2] {
    [(lfsr & 1) == 1, ((lfsr >> 1) & 1) == 1]
}

struct Rng(u64);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as u32
    }
    fn level(&mut self) -> Option<bool> {
        match self.next_u32() % 3 {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

fn to_logic(level: Option<bool>) -> Logic {
    match level {
        None => Logic::Z,
        Some(false) => Logic::Zero,
        Some(true) => Logic::One,
    }
}

fn assert_probes_equal(c: &Compiled, fast: &FastSim, probes: &[SignalId], what: &str) {
    for (k, &p) in probes.iter().enumerate() {
        assert_eq!(
            fast.value(p),
            Some(c.kernel.value(p).unwrap()),
            "probe {k} differs ({what})"
        );
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fast_mode_matches_the_event_kernel_with_external_tristate_drivers() {
    let (mut c, probes) = design("fast-tristate-external");
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    let ext = [c.drivers["pin:B11"], c.drivers["pin:A10"]];
    assert!(fast.program().input_slot(ext[0]).is_some());
    assert!(fast.program().input_slot(ext[1]).is_some());
    // Refuses Z on a non-tristate input.
    assert!(fast.set_at_levels(Time::ZERO, &[(clock, None)]).is_err());
    assert!(fast.set_levels(&[(clock, None)]).is_err());

    // Immediate form `set_levels` + `step` works with 0/1/Z (at t=0, oe0 is 0).
    fast.set_levels(&[(ext[0], Some(false))]).unwrap();
    fast.step().unwrap();
    assert_eq!(fast.value(probes[8]), Some(Logic::Zero));
    fast.set_levels(&[(ext[0], Some(true))]).unwrap();
    fast.step().unwrap();
    assert_eq!(fast.value(probes[8]), Some(Logic::One));
    fast.set_levels(&[(ext[0], None)]).unwrap();
    fast.step().unwrap();
    assert_eq!(fast.value(probes[8]), Some(Logic::One));

    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();

    let mut lfsr = 0xACE1u16;
    let mut rng = Rng(0x5EED_CAFE_1234_5678);
    let mut current: [Option<bool>; 2] = [None, None];
    let mut z_with_pull = [0usize; 2];
    let mut driven = [0usize; 2];

    for n in 1..=20_000u64 {
        let rise = Time::from_ps((2 * n - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(n * PERIOD_PS);

        let cur_oe = oe_of(lfsr);
        let next_lfsr = step_lfsr(lfsr);
        let next_oe = oe_of(next_lfsr);

        // Between fall(n-1) and rise(n), pick 0/1/Z for any pad whose oe is low
        // both now and after the rising edge; otherwise release to Z before the
        // rising edge so the design never collides with the external driver.
        let t_ext = Time::from_ps((n - 1) * PERIOD_PS + PERIOD_PS / 4);
        let mut updates = Vec::new();
        for i in 0..2 {
            let target = if !cur_oe[i] && !next_oe[i] {
                rng.level()
            } else {
                None
            };
            if target != current[i] {
                current[i] = target;
                updates.push((ext[i], target));
            }
        }
        if !updates.is_empty() {
            for &(d, lvl) in &updates {
                c.kernel.schedule(t_ext, d, to_logic(lvl)).unwrap();
            }
            c.kernel.run_until(t_ext).unwrap();
            fast.set_at_levels(t_ext, &updates).unwrap();
            assert_probes_equal(&c, &fast, &probes, &format!("at {t_ext}"));
        }

        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        fast.run_until(rise).unwrap();
        assert_probes_equal(&c, &fast, &probes, &format!("after rise {n}"));

        lfsr = next_lfsr;
        for i in 0..2 {
            if !cur_oe[i] && !next_oe[i] {
                match current[i] {
                    None => {
                        let expected_pull = if i == 0 { Logic::One } else { Logic::Zero };
                        assert_eq!(c.kernel.value(probes[8 + i]).unwrap(), expected_pull);
                        z_with_pull[i] += 1;
                    }
                    Some(_) => driven[i] += 1,
                }
            }
        }

        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        fast.run_until(fall).unwrap();
        assert_probes_equal(&c, &fast, &probes, &format!("after fall {n}"));
    }

    assert!(z_with_pull[0] > 1000 && z_with_pull[1] > 1000);
    assert!(driven[0] > 2000 && driven[1] > 2000);
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fast_mode_stops_when_external_driver_conflicts_with_design() {
    let (mut c, probes) = design("fast-tristate-external-conflict");
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    let ext0 = c.drivers["pin:B11"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();

    // Advance until gp[0]'s oe is high.
    let mut lfsr = 0xACE1u16;
    let mut n = 0u64;
    while !oe_of(lfsr)[0] {
        n += 1;
        let rise = Time::from_ps((2 * n - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(n * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        fast.run_until(fall).unwrap();
        lfsr = step_lfsr(lfsr);
    }

    let d0 = d_of(lfsr)[0];
    let t_same = Time::from_ps(n * PERIOD_PS + PERIOD_PS / 4);
    c.kernel.schedule(t_same, ext0, to_logic(Some(d0))).unwrap();
    c.kernel.run_until(t_same).unwrap();
    fast.set_at_levels(t_same, &[(ext0, Some(d0))]).unwrap();
    assert_eq!(c.kernel.value(probes[8]).unwrap(), to_logic(Some(d0)));
    assert_eq!(fast.value(probes[8]), Some(to_logic(Some(d0))));

    // Now drive the opposite level while oe0 is still high.
    let t_conflict = Time::from_ps(n * PERIOD_PS + PERIOD_PS / 2);
    c.kernel
        .schedule(t_conflict, ext0, to_logic(Some(!d0)))
        .unwrap();
    c.kernel.run_until(t_conflict).unwrap();
    assert_eq!(c.kernel.value(probes[8]).unwrap(), Logic::X);

    let err = fast
        .set_at_levels(t_conflict, &[(ext0, Some(!d0))])
        .expect_err("opposite external level must stop fast mode");
    let msg = format!("{err:#}");
    println!("conflict error: {msg}");
    assert_eq!(
        msg,
        "fast mode stopped at 180000 ps: pad R0C65:PAD_A is driven by the design and from outside with different levels; the event kernel makes it X"
    );
}
