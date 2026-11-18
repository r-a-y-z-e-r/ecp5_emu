//! The IOLOGIC input delay (DELAYG, DELAYF) on its data path: a design built
//! here with Yosys and nextpnr drives three pads from a counter and reads
//! them back through a DELAYG (DEL_VALUE 40), a DELAYF moved at run time,
//! and a DELAYF into an input register. The emulator executes the delay with
//! zero delay, as every propagation delay (HI08); the LEDs show the data one
//! register later in the event kernel and in fast mode alike.
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
  wire [2:0] o;
  BB b0(.I(c[2]), .T(1'b0), .O(o[0]), .B(gp[0]));
  BB b1(.I(c[3]), .T(1'b0), .O(o[1]), .B(gp[1]));
  BB b2(.I(c[4]), .T(1'b0), .O(o[2]), .B(gp[2]));
  wire z0, z1, z2, f1, f2;
  DELAYG #(.DEL_MODE(\"USER_DEFINED\"), .DEL_VALUE(40)) d0(.A(o[0]), .Z(z0));
  DELAYF #(.DEL_MODE(\"USER_DEFINED\"), .DEL_VALUE(5)) d1(.A(o[1]), .LOADN(c[7]), .MOVE(c[5]), .DIRECTION(c[6]), .Z(z1), .CFLAG(f1));
  DELAYF #(.DEL_MODE(\"USER_DEFINED\"), .DEL_VALUE(90)) d2(.A(o[2]), .LOADN(1'b1), .MOVE(c[6]), .DIRECTION(1'b0), .Z(z2), .CFLAG(f2));
  wire q2;
  IFS1P3DX r2(.SCLK(clk_25mhz), .SP(1'b1), .CD(1'b0), .D(z2), .Q(q2));
  reg [2:0] seen = 0;
  always @(posedge clk_25mhz) seen <= {q2, z1, z0};
  assign led = {5'b0, seen};
endmodule
";

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn input_delays_pass_their_data_in_the_kernel_and_in_fast_mode() {
    let (mut c, probes) = design("io-delay", VERILOG, &lpf(["NONE", "NONE", "NONE"]));
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut history = Vec::new();
    for n in 1..=400u64 {
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
        let led: Vec<Logic> = probes[..3]
            .iter()
            .map(|&p| c.kernel.value(p).unwrap())
            .collect();
        history.push(led);
    }
    // After cycle n the counter is n (mod 256): seen took c[2] and c[3] as
    // they were before edge n (value n - 1), and the input register's c[4]
    // one edge earlier (value n - 2).
    for n in 3..=400usize {
        let bit = |v: usize, b: usize| Logic::from_bool((v >> b) & 1 == 1);
        let want = [bit(n - 1, 2), bit(n - 1, 3), bit(n - 2, 4)];
        assert_eq!(history[n - 1], want, "cycle {n}");
    }
}

/// DELAYF's control as the HI08 board records show it, with DIRECTION taken
/// at MOVE's rise: `step` is one 25 MHz cycle with the controls registered
/// at its rising edge, returning the counter.
#[derive(Clone, Copy)]
struct Counter {
    value: u8,
    count: u8,
    direction: bool,
    move_: bool,
}

impl Counter {
    fn step(&mut self, loadn: bool, move_: bool, direction: bool) -> u8 {
        if !loadn {
            self.count = self.value;
            self.direction = false;
        } else {
            if move_ && !self.move_ {
                self.direction = direction;
            }
            if !move_ && self.move_ {
                self.count = if self.direction {
                    self.count.saturating_sub(1)
                } else {
                    (self.count + 1).min(127)
                };
            }
        }
        self.move_ = move_;
        self.count
    }
}

const CFLAG: &str = "module top(input clk_25mhz, inout [2:0] gp, output [7:0] led);
  reg [15:0] lfsr = 16'hace1;
  reg [11:0] c = 0;
  always @(posedge clk_25mhz) begin
    lfsr <= {lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]};
    c <= c + 1;
  end
  reg loadn = 1, move = 0, direction = 0;
  always @(posedge clk_25mhz) begin
    loadn <= !(c[10:2] == 0 || lfsr[7:0] == 8'h5a);
    move <= lfsr[0];
    direction <= c[9] ^ (lfsr[15:9] == 0);
  end
  wire o0, o1, z0, z1, f0, f1;
  BB b0(.I(c[0]), .T(1'b0), .O(o0), .B(gp[0]));
  BB b1(.I(c[1]), .T(1'b0), .O(o1), .B(gp[1]));
  DELAYF #(.DEL_MODE(\"USER_DEFINED\"), .DEL_VALUE(3)) d0(.A(o0), .LOADN(loadn), .MOVE(move), .DIRECTION(direction), .Z(z0), .CFLAG(f0));
  DELAYF #(.DEL_MODE(\"USER_DEFINED\"), .DEL_VALUE(125)) d1(.A(o1), .LOADN(loadn), .MOVE(move), .DIRECTION(direction), .Z(z1), .CFLAG(f1));
  assign led = {z1, z0, 1'b0, direction, move, loadn, f1, f0};
endmodule
";

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn delayf_cflag_follows_its_counter_in_the_kernel_and_in_fast_mode() {
    // gp[2] is not used. Pulled up, it is not left floating: older yosys and
    // nextpnr (0.52, 0.9) keep the unused pad as an input that starts at Z,
    // which fast mode refuses.
    let (mut c, probes) = design("io-delay-cflag", CFLAG, &lpf(["NONE", "NONE", "UP"]));
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut counters = [3, 125].map(|value| Counter {
        value,
        count: 0,
        direction: false,
        move_: false,
    });
    let (mut ends, mut changes) = ([0; 2], 0);
    for n in 1..=6000u64 {
        let rise = Time::from_ps((2 * n - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(n * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        fast.run_until(fall).unwrap();
        for (k, &p) in probes.iter().enumerate().take(6) {
            assert_eq!(
                fast.value(p),
                Some(c.kernel.value(p).unwrap()),
                "probe {k} after cycle {n}"
            );
        }
        let bit = |k: usize| c.kernel.value(probes[k]).unwrap() == Logic::One;
        let (loadn, move_, direction) = (bit(2), bit(3), bit(4));
        if move_ && counters[0].move_ && direction != counters[0].direction {
            changes += 1;
        }
        for (k, counter) in counters.iter_mut().enumerate() {
            let count = counter.step(loadn, move_, direction);
            let end = if direction { 0 } else { 127 };
            ends[k] += usize::from(count == end);
            assert_eq!(
                bit(k),
                count == end,
                "CFLAG{k} after cycle {n} (counter {count})"
            );
        }
    }
    // Both ends reached by both delays, and DIRECTION changed while MOVE
    // was high (outside the protocol) some times as well.
    assert!(ends.iter().all(|&e| e > 50), "{ends:?}");
    assert!(changes > 5, "{changes}");
}
