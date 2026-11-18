//! HD09/HS02 records at native DSP and FF pins, plus HD09 fast UART windows.
mod board;
use ecp5_emu::{
    compile::{self, Board, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{
        Builder, DriverId, Edge, FlipFlop, Input, Options, Reset, ResetKind, SignalId, Strength,
        Time,
    },
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use std::{path::PathBuf, sync::Arc};

const HD: &str = "hd09_dsp_clkdiv";
const HS: &str = "hs02_slice_prld";
const PERIOD: u64 = 40_000;
fn evidence() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F")
}
fn device(dir: &str, name: &str) -> Arc<ConfiguredDevice> {
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence()).unwrap());
    let image = Arc::new(
        ConfigurationImage::load(
            &evidence().join(dir).join(format!("{name}.config")),
            &db,
            None,
        )
        .unwrap(),
    );
    let catalog = load_catalogue(&model, &evidence()).unwrap();
    Arc::new(ConfiguredDevice::bind(model, image, catalog).unwrap())
}
fn records(dir: &str, name: &str, len: usize) -> Vec<String> {
    let text = std::fs::read_to_string(evidence().join(dir).join(name)).unwrap();
    let mut lines: Vec<String> = Vec::new();
    for row in text.lines() {
        if let Some((_, list)) = row.split_once("distinct=") {
            for s in list
                .split('\'')
                .skip(1)
                .step_by(2)
                .filter(|s| s.len() == len)
            {
                if !lines.iter().any(|l| l == s) {
                    lines.push(s.into());
                }
            }
        }
    }
    lines
}
fn compile(device: Arc<ConfiguredDevice>, probes: Vec<Reference>) -> Compiled {
    compile::compile_on_board(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(Reference::Pin("G2".into()), Logic::Zero)],
            probes,
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
        &Board::default(),
    )
    .unwrap_or_else(|e| panic!("{e:#}"))
}
fn dsp_replay(name: &str, log: &str, wide: bool) {
    let d = device(HD, name);
    let width = if wide { 160 } else { 60 };
    let n = if wide { 128 } else { 256 };
    let lines = records(HD, log, 1 + width * n / 4);
    assert_eq!(lines.len(), 1);
    let bits: Vec<_> = lines[0][1..]
        .chars()
        .flat_map(|c| {
            let v = c.to_digit(16).unwrap();
            (0..4).rev().map(move |k| Logic::from_bool(v >> k & 1 != 0))
        })
        .collect();
    let mut probes = Vec::new();
    let mut add = |column: usize, kind: &str, index: usize, first: usize, pins: &[usize]| {
        let id = d
            .model()
            .resource_id(&format!("LFE5U-85F/R58C{column}/{kind}_{index}"))
            .unwrap();
        let ports = d.model().resource_ports(id);
        for (k, pin) in pins.iter().enumerate() {
            let port = ports
                .iter()
                .find(|p| p.name == format!("{}{pin}", if kind == "ALU54" { "R" } else { "P" }))
                .unwrap();
            probes.push((first + k, Reference::Wire(port.wire.clone())));
        }
    };
    if wide {
        for (i, (block, second)) in [
            (13, false),
            (22, false),
            (31, false),
            (40, false),
            (49, false),
            (58, false),
            (58, true),
        ]
        .iter()
        .enumerate()
        {
            add(
                block + if *second { 7 } else { 3 },
                "ALU54",
                if *second { 7 } else { 3 },
                48 + 16 * i,
                &(0..16).rev().collect::<Vec<_>>(),
            );
        }
        for (i, block) in [13, 22, 31, 40].iter().enumerate() {
            add(
                block + 7,
                "ALU54",
                7,
                32 + 4 * i,
                if i == 1 {
                    &[50, 44, 40, 36]
                } else {
                    &[50, 30, 20, 0]
                },
            );
        }
    } else {
        for (i, col) in [16, 20, 34, 38].iter().enumerate() {
            add(
                *col,
                "ALU54",
                if i % 2 == 0 { 3 } else { 7 },
                24 + 4 * i,
                &[50, 30, 20, 0],
            );
        }
        for (i, (col, index)) in [(22, 0), (23, 1), (26, 4), (27, 5)].iter().enumerate() {
            add(*col, "MULT18", *index, 44 + 4 * i, &[33, 18, 16, 1]);
        }
    }
    let mut c = compile(d, probes.iter().map(|(_, r)| r.clone()).collect());
    let mut fast = match FastSim::new(&c.kernel) {
        Ok(fast) => Some(fast),
        Err(e)
            if name == "hd09ps"
                && e.downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                    .is_some() =>
        {
            None
        }
        Err(e) => panic!("{name}: {e:#}"),
    };
    let mut fast_bits = 0;
    let mut differ = Vec::new();
    for k in 0..n {
        if k > 0 {
            for (t, value) in [
                ((2 * k as u64 - 1) * PERIOD / 2, Logic::One),
                (k as u64 * PERIOD, Logic::Zero),
            ] {
                let at = Time::from_ps(t);
                c.kernel.schedule(at, c.drivers["pin:G2"], value).unwrap();
                c.kernel.run_until(at).unwrap();
                if let Some(fast) = &mut fast {
                    fast.set_at(at, &[(c.drivers["pin:G2"], value == Logic::One)])
                        .unwrap();
                } else {
                    // HD09ps leaves startup sign storage unmeasured (X).
                    // Its first rise loads the constant signs; no sample is skipped.
                    fast =
                        Some(FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{name}: {e:#}")));
                }
            }
        }
        for (bit, r) in &probes {
            let got = c.value(r).unwrap();
            let want = bits[k * width + bit];
            if let Some(fast) = &fast {
                assert_eq!(
                    fast.value(c.probes[&r.label()]),
                    Some(got),
                    "fast {name} sample {k} bit {bit}"
                );
                fast_bits += 1;
            }
            if got != want {
                differ.push(format!("sample {k} bit {bit}: {got} != {want}"));
            }
        }
    }
    println!(
        "{name}: {} / {} bits equal, {} mismatches",
        n * probes.len() - differ.len(),
        n * probes.len(),
        differ.len()
    );
    println!("fast {name}: {fast_bits} pin bits equal; startup is checked against the board by the kernel before sign warmup if needed");
    assert!(fast_bits >= (n - 1) * probes.len());
    assert!(differ.is_empty(), "{:?}", &differ[..differ.len().min(12)]);
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hd09s_board_replay() {
    dsp_replay("hd09s", "observations_s.log", true);
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hd09ms_board_replay() {
    dsp_replay("hd09ms", "observations_ms.log", false);
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hd09ps_board_replay() {
    dsp_replay("hd09ps", "observations_ps.log", false);
}

fn placements(name: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(evidence().join(HS).join(format!("{name}_probes.json"))).unwrap(),
    )
    .unwrap();
    use sha2::{Digest, Sha256};
    let hash: String =
        Sha256::digest(std::fs::read(evidence().join(HS).join(format!("{name}.config"))).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
    assert_eq!(hash, v["config_sha256"].as_str().unwrap());
    v["bels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().into())
        .collect()
}
fn ff_node(d: &ConfiguredDevice, bel: &str) -> ecp5_emu::model::NodeId {
    let a: Vec<_> = bel.split('/').collect();
    let x = &a[0][1..];
    let y = &a[1][1..];
    let plc = d
        .model()
        .resource_id(&format!("LFE5U-85F/R{y}C{x}/PLC2"))
        .unwrap();
    d.model()
        .nodes(plc)
        .find(|n| n.native_name == a[2])
        .unwrap()
        .id
}
fn shift(mut x: u64, n: usize) -> u64 {
    for _ in 0..n {
        x = (x << 1) | ((x >> 63 ^ x >> 62 ^ x >> 60 ^ x >> 59) & 1);
    }
    x
}
fn traffic(mut x: u64) -> [[Logic; 4]; 32] {
    std::array::from_fn(|_| {
        let r = [
            x & 1,
            (x >> 1) & 1,
            u64::from((x >> 2) & 3 != 0),
            u64::from((x >> 4) & 7 == 0),
        ]
        .map(|b| Logic::from_bool(b != 0));
        x = shift(x, 7);
        r
    })
}
// Binary linear-map exponentiation reconstructs window order from headers;
// observations.log stores a sorted distinct set rather than arrival order.
fn jump(mut x: u64, mut n: u64) -> u64 {
    fn apply(map: &[u64; 64], x: u64) -> u64 {
        (0..64)
            .filter(|i| x >> i & 1 != 0)
            .fold(0, |v, i| v ^ map[i])
    }
    let mut map = std::array::from_fn(|i| shift(1 << i, 1));
    while n > 0 {
        if n & 1 != 0 {
            x = apply(&map, x);
        }
        map = std::array::from_fn(|i| apply(&map, map[i]));
        n >>= 1;
    }
    x
}
fn first_free(rows: &[String]) -> &str {
    let mut seed = 0x1D2CACE15EED0001;
    for window in 0..=rows.len() + 1 {
        let prefix = format!("P{seed:016X}");
        if let Some(line) = rows.iter().find(|r| r.starts_with(&prefix)) {
            println!("hs02 first complete window {window}, header {prefix}");
            return line;
        }
        seed = jump(seed, 224 * (128 + 1043 * 2171));
    }
    panic!("no recorded header on the design's window grid");
}
// Each placed FF runs independently, so an X in one cannot hide the other
// known board probes. A race must stop fast mode. Its replacement starts from
// arbitrary Q=0 while the event kernel holds X; comparisons resume only when
// a later capture/preload overwrites that unknown, independently of old Q.
struct PinReplay {
    device: Arc<ConfiguredDevice>,
    node: ecp5_emu::model::NodeId,
    index: usize,
    fast: FastSim,
    q: SignalId,
    drivers: [DriverId; 5],
    pins: [Logic; 4],
    stops: usize,
}
impl PinReplay {
    fn new(
        device: Arc<ConfiguredDevice>,
        node: ecp5_emu::model::NodeId,
        index: usize,
        pins: [Logic; 4],
        clock: Logic,
        q: Logic,
        startup: bool,
    ) -> Self {
        let mut state = DeviceState::new(&device).unwrap();
        state.write_register(node, q).unwrap();
        let mut b = Builder::with_state(Arc::clone(&device), state).unwrap();
        let mut drivers = Vec::new();
        let inputs: Vec<Input> = [clock, pins[0], pins[1], pins[2], pins[3]]
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let name = format!("pin{i}");
                let s = b
                    .signal(Wire {
                        location: Location { row: 0, col: 0 },
                        name: name.clone(),
                    })
                    .unwrap();
                drivers.push(
                    b.external_driver(s, &name, Strength::Strong, value)
                        .unwrap(),
                );
                s.into()
            })
            .collect();
        let output = b
            .flip_flop(FlipFlop {
                node,
                data: inputs[1],
                preload: (index < 24).then_some(inputs[2]),
                clock: inputs[0],
                enable: inputs[3],
                edge: Edge::Rising,
                reset: Some(Reset {
                    input: inputs[4],
                    kind: if index % 8 >= 4 {
                        ResetKind::Asynchronous
                    } else {
                        ResetKind::Synchronous
                    },
                    value: Logic::from_bool(index % 4 >= 2),
                }),
            })
            .unwrap();
        if startup && index < 24 {
            b.initial_preload_register(node).unwrap();
        }
        b.watch(output).unwrap();
        let kernel = b.build().unwrap();
        Self {
            device,
            node,
            index,
            fast: FastSim::new(&kernel).unwrap(),
            q: output,
            drivers: drivers.try_into().unwrap(),
            pins,
            stops: 0,
        }
    }
    fn advance(&mut self, at: Time, clock: Logic, pins: [Logic; 4]) {
        let levels = [clock, pins[0], pins[1], pins[2], pins[3]];
        let changes: Vec<_> = self
            .drivers
            .iter()
            .zip(levels)
            .map(|(&d, v)| (d, v.bit().unwrap()))
            .collect();
        self.pins = pins;
        if let Err(error) = self.fast.set_at(at, &changes) {
            assert!(
                error.to_string().contains("asynchronous PRLD register"),
                "{error:#}"
            );
            let stops = self.stops + 1;
            *self = Self::new(
                Arc::clone(&self.device),
                self.node,
                self.index,
                pins,
                clock,
                Logic::Zero,
                false,
            );
            self.stops = stops;
        }
    }
    fn value(&self) -> Logic {
        self.fast.value(self.q).unwrap()
    }
}
fn hs_replay(startup: bool) {
    let name = if startup { "hs02s" } else { "hs02" };
    let rows = records(
        HS,
        if startup {
            "observations_s.log"
        } else {
            "observations.log"
        },
        1041,
    );
    let line = if startup {
        rows[0].as_str()
    } else {
        first_free(&rows)
    };
    let mut lfsr = u64::from_str_radix(&line[1..17], 16).unwrap();
    let words: Vec<_> = line.as_bytes()[17..]
        .chunks(8)
        .map(|s| u32::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect();
    let d = device(HS, name);
    let bels = placements(name);
    let probes: Vec<_> = (0..32)
        .step_by(2)
        .map(|i| {
            let node = ff_node(&d, &bels[i]);
            let ports = d.model().node_ports(node);
            (
                i,
                Reference::Wire(ports.into_iter().find(|p| p.name == "Q").unwrap().wire),
            )
        })
        .collect();
    if startup {
        let pin_refs: Vec<[Reference; 4]> = probes
            .iter()
            .map(|(i, _)| {
                let ports = d.model().node_ports(ff_node(&d, &bels[*i]));
                ["DI", "M", "CE", "LSR"].map(|name| {
                    Reference::Wire(ports.iter().find(|p| p.name == name).unwrap().wire.clone())
                })
            })
            .collect();
        let mut c = compile(
            Arc::clone(&d),
            probes
                .iter()
                .map(|(_, r)| r.clone())
                .chain(pin_refs.iter().flatten().cloned())
                .collect(),
        );
        let mut pins: Vec<_> = probes
            .iter()
            .zip(&pin_refs)
            .map(|((i, r), inputs)| {
                PinReplay::new(
                    Arc::clone(&d),
                    ff_node(&d, &bels[*i]),
                    *i,
                    inputs.clone().map(|r| c.value(&r).unwrap()),
                    Logic::Zero,
                    c.value(r).unwrap(),
                    true,
                )
            })
            .collect();
        let mut routed_fast = Some(FastSim::new(&c.kernel).unwrap());
        let mut routed_race = false;
        let mut equal = 0;
        let mut races = 0;
        for (k, word) in words.iter().enumerate() {
            if k > 0 {
                for (at, clock) in [
                    ((2 * k as u64 - 1) * PERIOD / 2, Logic::One),
                    (k as u64 * PERIOD, Logic::Zero),
                ] {
                    // The traffic FFs and observed FFs capture the old pins
                    // together; registered traffic then reaches DI/M/CE/LSR.
                    for pin in &mut pins {
                        pin.advance(Time::from_ps(at), clock, pin.pins);
                    }
                    c.kernel
                        .schedule(Time::from_ps(at), c.drivers["pin:G2"], clock)
                        .unwrap();
                    c.kernel.run_until(Time::from_ps(at)).unwrap();
                    for (pin, inputs) in pins.iter_mut().zip(&pin_refs) {
                        pin.advance(
                            Time::from_ps(at),
                            clock,
                            inputs.clone().map(|r| c.value(&r).unwrap()),
                        );
                    }
                    if let Some(fast) = &mut routed_fast {
                        if let Err(error) = fast.set_at(
                            Time::from_ps(at),
                            &[(c.drivers["pin:G2"], clock == Logic::One)],
                        ) {
                            assert!(
                                error.to_string().contains("asynchronous PRLD register"),
                                "{error:#}"
                            );
                            routed_race = true;
                            routed_fast = None;
                        } else {
                            for (_, r) in &probes {
                                assert_eq!(
                                    fast.value(c.probes[&r.label()]),
                                    Some(c.value(r).unwrap())
                                );
                            }
                        }
                    }
                }
            }
            for ((i, r), pin) in probes.iter().zip(&pins) {
                let got = c.value(r).unwrap();
                if got == Logic::X {
                    assert!(*i < 24 && i % 8 >= 4);
                    races += 1;
                    continue;
                }
                assert_eq!(
                    got,
                    Logic::from_bool(word >> i & 1 != 0),
                    "{name} sample {k} ff{i:02}"
                );
                assert_eq!(pin.value(), got, "fast {name} sample {k} ff{i:02}");
                equal += 1;
            }
        }
        assert!(routed_race);
        assert_eq!((equal, races), (1992, 56));
        assert!(pins.iter().map(|p| p.stops).sum::<usize>() > 0);
        println!("fast {name}: {equal}/1992 bits compared and equal; {} CEMUX=1 excluded; {races} async-race X samples excluded",128*16);
    } else {
        // The first complete free window begins after UART time. Replay its
        // regenerated traffic at the physical FFs, seeding Q from sample 0.
        let mut state = DeviceState::new(&d).unwrap();
        for (i, _) in &probes {
            state
                .write_register(
                    ff_node(&d, &bels[*i]),
                    Logic::from_bool(words[0] >> i & 1 != 0),
                )
                .unwrap();
        }
        let mut b = Builder::with_state(Arc::clone(&d), state).unwrap();
        let mut drivers = Vec::new();
        let mut outputs = Vec::new();
        let mut input = |b: &mut Builder, name: String, value: Logic| {
            let s = b
                .signal(Wire {
                    location: Location { row: 0, col: 0 },
                    name: name.clone(),
                })
                .unwrap();
            drivers.push(
                b.external_driver(s, &name, Strength::Strong, value)
                    .unwrap(),
            );
            Input::from(s)
        };
        let clock = input(&mut b, "clock".into(), Logic::Zero);
        let initial = traffic(lfsr);
        for (i, _) in &probes {
            let pins: [Input; 4] =
                std::array::from_fn(|j| input(&mut b, format!("ff{i}.{j}"), initial[*i][j]));
            outputs.push((
                *i,
                b.flip_flop(FlipFlop {
                    node: ff_node(&d, &bels[*i]),
                    data: pins[0],
                    preload: (*i < 24).then_some(pins[1]),
                    clock,
                    enable: pins[2],
                    edge: Edge::Rising,
                    reset: Some(Reset {
                        input: pins[3],
                        kind: if i % 8 >= 4 {
                            ResetKind::Asynchronous
                        } else {
                            ResetKind::Synchronous
                        },
                        value: Logic::from_bool(i % 4 >= 2),
                    }),
                })
                .unwrap(),
            ));
        }
        let mut kernel = b.build().unwrap();
        let mut pins: Vec<_> = outputs
            .iter()
            .map(|(i, q)| {
                PinReplay::new(
                    Arc::clone(&d),
                    ff_node(&d, &bels[*i]),
                    *i,
                    initial[*i],
                    Logic::Zero,
                    kernel.value(*q).unwrap(),
                    false,
                )
            })
            .collect();
        let mut equal = 0;
        let mut races = 0;
        for (k, word) in words.iter().enumerate() {
            if k > 0 {
                let rise = Time::from_ps((2 * k as u64 - 1) * PERIOD / 2);
                kernel.schedule(rise, drivers[0], Logic::One).unwrap();
                kernel.run_until(rise).unwrap();
                for pin in &mut pins {
                    pin.advance(rise, Logic::One, pin.pins);
                }
                lfsr = shift(lfsr, 224);
                let t = traffic(lfsr);
                let mut changes = Vec::new();
                for (n, (i, _)) in outputs.iter().enumerate() {
                    for j in 0..4 {
                        changes.push((drivers[1 + 4 * n + j], t[*i][j]));
                    }
                }
                // Registered traffic changes after the rising-edge sample.
                let after = Time::from_ps((2 * k as u64 - 1) * PERIOD / 2 + 1);
                kernel.schedule_batch(after, &changes, None).unwrap();
                kernel.run_until(after).unwrap();
                for pin in &mut pins {
                    pin.advance(after, Logic::One, t[pin.index]);
                }
                let fall = Time::from_ps(k as u64 * PERIOD);
                kernel.schedule(fall, drivers[0], Logic::Zero).unwrap();
                kernel.run_until(fall).unwrap();
                for pin in &mut pins {
                    pin.advance(fall, Logic::Zero, pin.pins);
                }
            }
            if k == 0 {
                continue;
            }
            for ((i, q), pin) in outputs.iter().zip(&pins) {
                let got = kernel.value(*q).unwrap();
                if got == Logic::X {
                    assert!(*i < 24 && i % 8 >= 4);
                    races += 1;
                    continue;
                }
                assert_eq!(
                    got,
                    Logic::from_bool(word >> i & 1 != 0),
                    "{name} sample {k} ff{i:02}"
                );
                assert_eq!(pin.value(), got, "fast {name} sample {k} ff{i:02}");
                equal += 1;
            }
        }
        assert_eq!((equal, races), (1980, 52));
        assert!(pins.iter().map(|p| p.stops).sum::<usize>() > 0);
        println!("fast {name}: {equal}/1980 bits compared and equal; 16 sample-zero bits seed Q; {} CEMUX=1 excluded; {races} async-race X samples excluded",128*16);
    }
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hs02s_board_replay() {
    hs_replay(true);
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hs02_free_board_replay() {
    hs_replay(false);
}
#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hd09_free_windows_in_fast_mode_are_the_boards() {
    const WINDOW: u64 = 256 + 3843 * (10 * 217 + 1);
    for (name, log) in [
        ("hd09m", "observations_m.log"),
        ("hd09p", "observations_p.log"),
    ] {
        let board = records(HD, log, 3841);
        assert_eq!(board.len(), 59);
        let mut c = compile(device(HD, name), vec![Reference::Pin("L4".into())]);
        // The input/pipeline sign bits are unmeasured at configuration. Run
        // the first rise in the kernel to load them before fast handover.
        let at = Time::from_ps(PERIOD / 2);
        c.kernel
            .schedule(at, c.drivers["pin:G2"], Logic::One)
            .unwrap();
        c.kernel.run_until(at).unwrap();
        let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD))
            .unwrap();
        let samples = fast
            .run_cycles(4 * WINDOW + 3_000, c.probes["pin:L4"])
            .unwrap();
        let bits: Vec<_> = std::iter::once(true)
            .chain(samples.iter().map(|&v| v == 1))
            .collect();
        let text = board::decode_uart(&bits);
        let lines: Vec<_> = text
            .split("\r\n")
            .filter(|l| l.len() == 3841 && l.starts_with('M'))
            .collect();
        assert!(lines.len() >= 4, "{name}: {} complete windows", lines.len());
        let startup = records(
            HD,
            if name == "hd09m" {
                "observations_ms.log"
            } else {
                "observations_ps.log"
            },
            3841,
        );
        assert_eq!(lines[0], startup[0], "{name} startup window");
        // The board reader can miss window zero while opening the port.
        for (i, line) in lines.iter().enumerate().skip(1) {
            assert!(
                board.iter().any(|b| b == line),
                "{name} window {i} differs from all board lines"
            );
        }
        println!(
            "fast {name}: {} complete windows equal to board lines, including the startup window",
            lines.len()
        );
    }
}

#[test]
#[ignore = "requires native image reader and Trellis database"]
fn hs02_config_fast_starts_at_configured_m() {
    let d = device(HS, "hs02");
    let bels = placements("hs02");
    let node = ff_node(&d, &bels[0]);
    let q = d
        .model()
        .node_ports(node)
        .into_iter()
        .find(|p| p.name == "Q")
        .unwrap();
    let c = compile(d, vec![Reference::Wire(q.wire.clone())]);
    let fast = FastSim::new(&c.kernel).unwrap();
    let q = c.kernel.signal(&q.wire).unwrap();
    assert_eq!(fast.value(q), Some(c.kernel.value(q).unwrap()));
}
