//! HY01 SGSR: zero-delay raw-active OR two plain stages, initially 11.
//! Board comparisons exclude only 0..=2 observer ticks AFTER a modeled reset
//! transition. Values near those transitions are physical timing hazards.
#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
};

const SERIES: &str = "docs/inventory/LFE5U-85F/hy01_sgsr";
const TICK_PS: u64 = 10_000;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Environment {
    db: PathBuf,
    model: Arc<DeviceModel>,
    evidence: PathBuf,
}
fn environment() -> &'static Environment {
    static ENV: OnceLock<Environment> = OnceLock::new();
    ENV.get_or_init(|| {
        let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None).unwrap();
        let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
        Environment {
            db,
            model,
            evidence,
        }
    })
}
struct Design {
    dir: PathBuf,
    pins: BTreeMap<String, String>,
    config: String,
}
impl Drop for Design {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn run(dir: &Path, program: &str, args: &[&str]) {
    let output = tools::command(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    fs::write(
        dir.join(format!("{program}.log")),
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
impl Design {
    fn build(tag: &str, hdl: &str, ports: &[&str], post_synth: &str) -> Self {
        let env = environment();
        let dir = std::env::temp_dir().join(format!(
            "sgsr-native-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let mut design = Self {
            dir,
            pins: BTreeMap::new(),
            config: String::new(),
        };
        let mut balls = env
            .model
            .package_pins()
            .filter(|p| p.package == "CABGA381" && p.pio.is_some() && p.ball != "G2")
            .map(|p| p.ball.clone())
            .collect::<Vec<_>>();
        balls.sort();
        let mut balls = balls.into_iter();
        for &port in ports {
            design.pins.insert(
                port.into(),
                if port == "clk" {
                    "G2".into()
                } else {
                    balls.next().unwrap()
                },
            );
        }
        let lpf: String = design.pins.iter().map(|(port, ball)| format!("LOCATE COMP \"{port}\" SITE \"{ball}\";\nIOBUF PORT \"{port}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect();
        fs::write(design.dir.join("top.v"), hdl).unwrap();
        fs::write(design.dir.join("top.lpf"), lpf).unwrap();
        run(
            &design.dir,
            "yosys",
            &[
                "-Q",
                "-q",
                "-p",
                &format!(
                    "read_verilog top.v; synth_ecp5 -top top; {post_synth}; write_json top.json"
                ),
            ],
        );
        run(
            &design.dir,
            "nextpnr-ecp5",
            &[
                "--85k",
                "--package",
                "CABGA381",
                "--seed",
                "1",
                "--json",
                "top.json",
                "--lpf",
                "top.lpf",
                "--freq",
                "25",
                "--textcfg",
                "top.config",
                "--quiet",
            ],
        );
        design.config = fs::read_to_string(design.dir.join("top.config")).unwrap();
        design
    }
    fn device(&self, name: &str, source: Option<&str>) -> Arc<ConfiguredDevice> {
        let env = environment();
        if let Some(source) = source {
            fs::write(self.dir.join(name), source).unwrap();
        }
        let image = ConfigurationImage::load(&self.dir.join(name), &env.db, None).unwrap();
        Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&env.model),
                Arc::new(image),
                load_catalogue(&env.model, &env.evidence).unwrap(),
            )
            .unwrap(),
        )
    }
    fn request(&self, inputs: &[(&str, bool)], outputs: &[&str]) -> Request {
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: inputs
                .iter()
                .map(|(p, v)| (Reference::Pin(self.pins[*p].clone()), Logic::from_bool(*v)))
                .collect(),
            probes: outputs
                .iter()
                .map(|p| Reference::Pin(self.pins[*p].clone()))
                .collect(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        }
    }
    fn compile(&self, inputs: &[(&str, bool)], outputs: &[&str]) -> Compiled {
        compile::compile(
            self.device("top.config", None),
            self.request(inputs, outputs),
        )
        .unwrap_or_else(|e| panic!("{e:#}"))
    }
    fn reference(&self, port: &str) -> Reference {
        Reference::Pin(self.pins[port].clone())
    }
}
struct Pair<'a> {
    design: &'a Design,
    event: Compiled,
    fast: FastSim,
    outputs: Vec<String>,
    time: u64,
}
impl<'a> Pair<'a> {
    // Twenty ns per half-cycle keeps startup/X20 at the board's 25 MHz.
    // Capture 3 is then 40 ns after s2 releases, outside the hazard window.
    fn half_cycle(&mut self, inputs: &[(&str, bool)]) -> Vec<bool> {
        let values = self.step(inputs);
        assert_eq!(self.step(inputs), values);
        values
    }
    fn new(design: &'a Design, event: Compiled, outputs: &[&str]) -> Self {
        let fast = FastSim::new(&event.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        Self {
            design,
            event,
            fast,
            outputs: outputs.iter().map(|p| (*p).into()).collect(),
            time: 0,
        }
    }
    fn step(&mut self, inputs: &[(&str, bool)]) -> Vec<bool> {
        self.time += TICK_PS;
        let time = Time::from_ps(self.time);
        let updates: Vec<_> = inputs
            .iter()
            .map(|(p, v)| {
                (
                    self.event.drivers[&self.design.reference(p).label()],
                    Logic::from_bool(*v),
                )
            })
            .collect();
        self.event
            .kernel
            .schedule_batch(time, &updates, None)
            .unwrap();
        self.event.kernel.run_until(time).unwrap();
        self.fast.run_until(time).unwrap();
        let fast_inputs: Vec<_> = inputs
            .iter()
            .map(|(p, v)| {
                (
                    self.event.drivers[&self.design.reference(p).label()],
                    Some(*v),
                )
            })
            .collect();
        self.fast.set_levels(&fast_inputs).unwrap();
        self.fast.step().unwrap();
        self.outputs
            .iter()
            .map(|p| {
                let reference = self.design.reference(p);
                let signal = self.event.probes[&reference.label()];
                let actual = self.event.kernel.value(signal).unwrap();
                assert_eq!(
                    self.fast.value(signal),
                    Some(actual),
                    "equal traces: {p} at {time}"
                );
                actual.bit().unwrap()
            })
            .collect()
    }
}
const PROBE_OUTPUTS: [&str; 6] = ["q[0]", "q[1]", "q[2]", "q[3]", "q[4]", "q[5]"];
fn probes() -> Design {
    let mut hdl = String::from("module top(input clk,other,gsrn,input [2:0] d,output [5:0] q); SGSR u(.GSR(gsrn),.CLK(clk));\n");
    for group in 0..3 {
        for regset in 0..2 {
            hdl.push_str(&format!("(* keep=1 *) TRELLIS_FF #(.GSR(\"ENABLED\"),.REGSET(\"{}\"),.CLKMUX(\"{}\"),.CEMUX(\"1\"),.SRMODE(\"ASYNC\")) probe_{group}_{regset}(.CLK({}),.CE(1'b1),.LSR(1'b0),.DI(d[{group}]),.Q(q[{}]));\n", if regset == 0 { "RESET" } else { "SET" }, if group == 1 { "INV" } else { "CLK" }, if group == 2 { "other" } else { "clk" }, 2*group+regset));
        }
    }
    hdl.push_str("endmodule\n");
    Design::build(
        "probes",
        &hdl,
        &[
            "clk", "other", "gsrn", "d[0]", "d[1]", "d[2]", "q[0]", "q[1]", "q[2]", "q[3]", "q[4]",
            "q[5]",
        ],
        "setparam -set GSR \"ENABLED\" t:TRELLIS_FF",
    )
}
const PROBE_INPUTS: [(&str, bool); 6] = [
    ("clk", false),
    ("other", false),
    ("gsrn", true),
    ("d[0]", false),
    ("d[1]", false),
    ("d[2]", false),
];
#[derive(Clone)]
struct Rule {
    stages: [bool; 2],
    clocks: [bool; 2],
    q: [bool; 6],
}
impl Rule {
    fn new() -> Self {
        Self {
            stages: [true; 2],
            clocks: [false; 2],
            q: [false, true, false, true, false, true],
        }
    }
    fn step(&mut self, clk: bool, other: bool, active: bool, d: [bool; 3]) -> ([bool; 6], bool) {
        let edges = [
            clk && !self.clocks[0],
            !clk && self.clocks[0],
            other && !self.clocks[1],
        ];
        let old_s2 = self.stages[1];
        for (g, &edge) in edges.iter().enumerate() {
            for r in 0..2 {
                if active || old_s2 {
                    self.q[2 * g + r] = r != 0;
                } else if edge {
                    self.q[2 * g + r] = d[g];
                }
            }
        }
        if edges[0] {
            self.stages = [active, self.stages[0]];
        }
        if active || self.stages[1] {
            self.q = [false, true, false, true, false, true];
        }
        self.clocks = [clk, other];
        (self.q, old_s2 != self.stages[1])
    }
}
fn stimulus(clk: bool, other: bool, n: bool, d: [bool; 3]) -> [(&'static str, bool); 6] {
    [
        ("clk", clk),
        ("other", other),
        ("gsrn", n),
        ("d[0]", d[0]),
        ("d[1]", d[1]),
        ("d[2]", d[2]),
    ]
}
#[test]
#[ignore = "requires Python 3 to run the HY01 board-log analyser"]
fn all_four_board_logs_have_no_residual_outside_reset_transition_hazards() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("python3")
        .arg("-B")
        .arg(root.join(SERIES).join("hy01_residuals.py"))
        .args(["--selftest", "--candidate"])
        .args(
            [
                "observations.log",
                "observations_s.log",
                "observations_b.log",
                "observations_c.log",
            ]
            .map(|p| root.join(SERIES).join(p)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("within_2_ticks=120 outside_2_ticks=0"));
    assert_eq!(report.matches("execution_gate=PASS").count(), 4);
}

#[test]
#[ignore = "requires Yosys/nextpnr/Trellis, native image reader and clang"]
fn sgsr_native_pulses_and_board_replays_match_both_engines() {
    let design = probes();
    assert!(design.config.contains("enum: GSR.SYNCMODE SYNC"));
    let compiled = design.compile(&PROBE_INPUTS, &PROBE_OUTPUTS);
    assert_eq!(compiled.report.counts.flip_flops, 6); // two internal stages do not claim slice FFs
    assert!(compiled
        .report
        .resources
        .iter()
        .any(|r| r.rule.contains("hy01b/c")));
    let mut pair = Pair::new(&design, compiled, &PROBE_OUTPUTS);
    let mut rule = Rule::new();
    let mut previous_clk = false;
    let mut previous_other = false;
    let mut phase = [false; 3];
    let mut rise_holds = 0;
    for cycles in [1, 2, 3, 5, 20] {
        for offset in 0..4 {
            let onset = 64 + offset;
            for tick in 0..onset + 4 * cycles + 40 {
                let clk = tick % 4 >= 2;
                let other = tick % 3 == 1;
                let active = (onset..onset + 4 * cycles).contains(&tick);
                let data = phase;
                let (want, _) = rule.step(clk, other, active, data);
                let actual = pair.step(&stimulus(clk, other, !active, data));
                assert_eq!(actual, want, "cycles={cycles} phase={offset} tick={tick}");
                if clk && !previous_clk {
                    phase[0] = !phase[0];
                    if pair.time <= 7 * TICK_PS {
                        rise_holds += usize::from(!actual[0]);
                    }
                }
                if !clk && previous_clk {
                    phase[1] = !phase[1];
                }
                if other && !previous_other {
                    phase[2] = !phase[2];
                }
                previous_clk = clk;
                previous_other = other;
            }
        }
    }
    assert_eq!(rise_holds, 2); // HY01b: first capture is the third rising edge
    for falling_first in [false, true] {
        let mut inputs = PROBE_INPUTS;
        inputs[0].1 = falling_first;
        let mut startup = Pair::new(
            &design,
            design.compile(&inputs, &PROBE_OUTPUTS),
            &PROBE_OUTPUTS,
        );
        let mut reference = Rule::new();
        reference.clocks[0] = falling_first;
        let mut held_edges = [0; 2];
        for half_edge in 0..12 {
            let clk = (half_edge % 2 == 0) != falling_first;
            let other = half_edge % 3 == 1;
            let data = [true, false, true];
            let (want, _) = reference.step(clk, other, false, data);
            let actual = startup.half_cycle(&stimulus(clk, other, true, data));
            assert_eq!(
                actual, want,
                "RESET/SET startup falling_first={falling_first}"
            );
            let group = usize::from(!clk);
            held_edges[group] += usize::from(actual[2 * group] != actual[2 * group + 1]);
        }
        assert_eq!(held_edges, [2, if falling_first { 2 } else { 1 }]);
    }
    let mut compared = 0;
    let mut residuals = 0;
    for log in ["observations.log", "observations_s.log"] {
        let text = fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(SERIES)
                .join(log),
        )
        .unwrap();
        let mut seen = HashSet::new();
        let mut windows = 0;
        for line in text
            .lines()
            .filter(|s| s.starts_with("Y1R") || s.starts_with("Y1S"))
        {
            assert_eq!(line.len(), 2071);
            let payload = &line[19..2067];
            if !seen.insert(payload.to_string()) {
                continue;
            }
            windows += 1;
            let words: Vec<u16> = (0..512)
                .map(|i| u16::from_str_radix(&payload[4 * i..4 * i + 4], 16).unwrap())
                .collect();
            assert_eq!(
                words.iter().fold(0, |a, w| a ^ w),
                u16::from_str_radix(&line[2067..], 16).unwrap()
            );
            let bit = |t: usize, b: usize| words[t] & (1 << b) != 0;
            let start = words
                .iter()
                .rposition(|w| w & 0x8000 == 0)
                .map_or(23, |t| (t + 24).max(16));
            // The marker for the next physical edge tells the initial level.
            let first_rise = (1..512).find(|&t| bit(t, 11) != bit(t - 1, 11)).unwrap();
            let first_fall = (1..512).find(|&t| bit(t, 12) != bit(t - 1, 12)).unwrap();
            let mut clk = first_fall < first_rise;
            let mut reference = Rule::new();
            reference.clocks = [clk, false];
            let mut last_transition = 0;
            for t in 0..512 {
                if t > 0 && bit(t, 11) != bit(t - 1, 11) {
                    clk = true;
                }
                if t > 0 && bit(t, 12) != bit(t - 1, 12) {
                    clk = false;
                }
                let other = t > 0 && bit(t, 13) != bit(t - 1, 13);
                let data = std::array::from_fn(|g| bit(t.saturating_sub(1), 11 + g));
                let n = bit(t, 14);
                let (want, s2_transition) = reference.step(clk, other, !n, data);
                if s2_transition || (t > 0 && n != bit(t - 1, 14)) {
                    last_transition = t;
                }
                let actual = pair.step(&stimulus(clk, other, n, data));
                if t < start {
                    continue;
                }
                assert_eq!(actual, want, "rule replay {log} window={windows} tick={t}");
                for (k, &q) in actual.iter().enumerate() {
                    compared += 1;
                    if q != bit(t, 5 + k) {
                        residuals += 1;
                        assert!(t-last_transition <= 2, "board mismatch outside hazard: {log} window={windows} tick={t} channel={k}");
                    }
                }
            }
        }
        assert_eq!(windows, if log == "observations.log" { 310 } else { 2 });
    }
    assert_eq!(compared, 915_384);
    assert_eq!(residuals, 120);
    println!("SGSR: 20 pulse width/phase combinations and 312 board windows; {compared} board channel-bins, {residuals} within hazards; equal event/fast traces throughout");
}

fn counters(sgsr: bool) -> Design {
    let mut hdl = String::from("module top(input clk,output [3:0] ar,br,af,bf);\n");
    if sgsr {
        hdl.push_str("SGSR u(.GSR(1'b1),.CLK(clk));\n");
    }
    for port in ["ar", "br", "af", "bf"] {
        hdl.push_str(&format!("wire [3:0] d_{port}={port}+1'b1;\n"));
        for b in 0..4 {
            hdl.push_str(&format!("(* keep=1 *) TRELLIS_FF #(.GSR(\"{}\"),.REGSET(\"RESET\"),.CLKMUX(\"{}\"),.CEMUX(\"1\")) count_{port}_{b}(.CLK(clk),.CE(1'b1),.LSR(1'b0),.DI(d_{port}[{b}]),.Q({port}[{b}]));\n", if port.starts_with('a') {"ENABLED"} else {"DISABLED"}, if port.ends_with('f') {"INV"} else {"CLK"}));
        }
    }
    hdl.push_str("endmodule\n");
    let mut ports = vec!["clk".to_string()];
    for p in ["ar", "br", "af", "bf"] {
        for b in 0..4 {
            ports.push(format!("{p}[{b}]"));
        }
    }
    Design::build(if sgsr {"boot-b"} else {"boot-c"}, &hdl, &ports.iter().map(String::as_str).collect::<Vec<_>>(), "setparam -set GSR \"DISABLED\" t:TRELLIS_FF; setparam -set GSR \"ENABLED\" c:count_ar_* c:count_af_*")
}
#[test]
#[ignore = "requires Yosys/nextpnr/Trellis, native image reader and clang"]
fn sgsr_startup_board_offsets_and_x20_execute_in_both_engines() {
    for sgsr in [true, false] {
        let design = counters(sgsr);
        let outputs: Vec<String> = ["ar", "br", "af", "bf"]
            .iter()
            .flat_map(|p| (0..4).map(move |b| format!("{p}[{b}]")))
            .collect();
        let outputs: Vec<&str> = outputs.iter().map(String::as_str).collect();
        for falling_first in [false, true] {
            // Compile twice also exercises the compiled-plan cache with internal stages.
            let _ = design.compile(&[("clk", falling_first)], &outputs);
            let compiled = design.compile(&[("clk", falling_first)], &outputs);
            assert_eq!(compiled.report.counts.flip_flops, 16);
            let mut pair = Pair::new(&design, compiled, &outputs);
            let mut stages = [sgsr; 2];
            let mut want = [0u8; 4];
            for half_edge in 0..16 {
                let clk = (half_edge % 2 == 0) != falling_first;
                let group = if clk { 0 } else { 2 };
                want[group + 1] = (want[group + 1] + 1) & 15;
                want[group] = if stages[1] { 0 } else { (want[group] + 1) & 15 };
                if clk {
                    stages = [false, stages[0]];
                }
                let actual = pair.half_cycle(&[("clk", clk)]);
                let counts: Vec<u8> = actual
                    .chunks(4)
                    .map(|bits| {
                        bits.iter()
                            .enumerate()
                            .fold(0, |a, (b, &v)| a | ((v as u8) << b))
                    })
                    .collect();
                assert_eq!(
                    counts, want,
                    "startup sgsr={sgsr} falling_first={falling_first} edge={half_edge}"
                );
            }
            assert_eq!(
                (
                    want[1].wrapping_sub(want[0]) & 15,
                    want[3].wrapping_sub(want[2]) & 15
                ),
                if sgsr {
                    (2, if falling_first { 2 } else { 1 })
                } else {
                    (0, 0)
                }
            );
        }
        let log = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SERIES).join(
            if sgsr {
                "observations_b.log"
            } else {
                "observations_c.log"
            },
        ))
        .unwrap();
        let mut load = 0;
        let mut snapshots = BTreeMap::new();
        for line in log.lines() {
            if let Some(s) = line.strip_prefix("# load=") {
                load = s
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
            }
            if !line.starts_with(if sgsr { "Y1B" } else { "Y1C" }) {
                continue;
            }
            let values: Vec<u16> = (3..31)
                .step_by(4)
                .map(|i| u16::from_str_radix(&line[i..i + 4], 16).unwrap())
                .collect();
            assert_eq!(values[..6].iter().fold(0, |a, v| a ^ v), values[6]);
            let offsets = (
                values[1].wrapping_sub(values[0]),
                values[4].wrapping_sub(values[3]),
            );
            assert_eq!(offsets, (values[2], values[5]));
            let expected = if sgsr {
                (2, if [1, 2, 4, 10].contains(&load) { 1 } else { 2 })
            } else {
                (0, 0)
            };
            assert_eq!(offsets, expected);
            if let Some(previous) = snapshots.insert(load, line.to_string()) {
                assert_eq!(previous, line);
            }
        }
        assert_eq!(snapshots.len(), 10);
    }
    // Exact R15/X20 HDL from REFUSAL_AUDIT.md, both text and packed bitstream.
    let design = Design::build("x20", "module top(input clk,rst,d,output reg q); SGSR u(.GSR(rst),.CLK(clk)); always @(posedge clk) q<=d; endmodule\n", &["clk","rst","d","q"], "setparam -set GSR \"ENABLED\" t:TRELLIS_FF");
    run(&design.dir, "ecppack", &["top.config", "top.bit"]);
    for name in ["top.config", "top.bit"] {
        let compiled = compile::compile(
            design.device(name, None),
            design.request(&[("clk", false), ("rst", true), ("d", true)], &["q"]),
        )
        .unwrap();
        let mut pair = Pair::new(&design, compiled, &["q"]);
        for edge in 1..=4 {
            assert_eq!(
                pair.half_cycle(&[("clk", true)]),
                [edge >= 3],
                "X20 {name} rising edge {edge}"
            );
            pair.half_cycle(&[("clk", false)]);
        }
        assert_eq!(pair.step(&[("rst", false)]), [false]);
    }
}
#[test]
#[ignore = "requires Yosys/nextpnr/Trellis and native image reader"]
fn sgsr_unmeasured_polarity_and_missing_clock_are_refused() {
    let design = probes();
    let high = design.config.replace(
        "enum: GSR.GSRMODE ACTIVE_LOW",
        "enum: GSR.GSRMODE ACTIVE_HIGH",
    );
    let error = compile::compile(
        design.device("high.config", Some(&high)),
        design.request(&PROBE_INPUTS, &PROBE_OUTPUTS),
    )
    .err()
    .expect("unmeasured polarity accepted");
    assert!(format!("{error:#}").contains("HY01-measured ACTIVE_LOW"));
    // All physical clock routes removed, including the fixed GSR->PLC endpoint's feed.
    let disconnected = design
        .config
        .lines()
        .filter(|l| !l.starts_with("arc: JCLK") && !l.contains(" G_HPBX") && !l.contains(" G_VPTX"))
        .collect::<Vec<_>>()
        .join("\n");
    let error = compile::compile(
        design.device("no-clock.config", Some(&disconnected)),
        design.request(&PROBE_INPUTS, &PROBE_OUTPUTS),
    )
    .err()
    .expect("unrouted SGSR clock accepted");
    assert!(format!("{error:#}").contains("no CLK route"), "{error:#}");
}
