//! HI13's routed lanes and HI12 sync_2: board counts and complete event/fast
//! transition traces, including configuration start and the delayed DCCA.
mod board;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, process::Command, sync::Arc};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn replay(hi13: bool) {
    let series = if hi13 {
        "hi13_eclksync_start"
    } else {
        "hi12_eclksync_none"
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("ecp5-start-{}-{series}", std::process::id())));
    std::fs::create_dir(&scratch.0).unwrap();
    let output = if hi13 {
        Command::new("python3")
            .arg("-B")
            .arg(evidence.join(series).join("build.py"))
            .arg(&scratch.0)
            .output()
            .unwrap()
    } else {
        // Reuse the board builder, including its direct-pin pre-route hook,
        // HY01 observer and all validation; only sync_2 is needed here.
        Command::new("python3").args(["-B", "-c", "import build,sys; from pathlib import Path; build.VARIANTS=['sync_2']; build.build(Path(sys.argv[1]))"])
            .arg(&scratch.0).current_dir(evidence.join(series)).output().unwrap()
    };
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let design = if hi13 {
        scratch.0.clone()
    } else {
        scratch.0.join("sync_2")
    };
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    // Packed bits, not a patched config or a synthetic synchronizer.
    let image = Arc::new(ConfigurationImage::load(&design.join("top.bit"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let mut wires = vec![
        Wire {
            location: Location { row: 46, col: 0 },
            name: "ECLKI_ECLKSYNC1".into(),
        },
        Wire {
            location: Location { row: 46, col: 0 },
            name: "CLKI_CLKDIV1".into(),
        },
        Wire {
            location: Location { row: 46, col: 0 },
            name: "JCDIVX_CLKDIV1".into(),
        },
    ];
    if hi13 {
        wires.extend([
            Wire {
                location: Location { row: 47, col: 0 },
                name: "ECLKI_ECLKSYNC0".into(),
            },
            Wire {
                location: Location { row: 46, col: 0 },
                name: "CLKI_CLKDIV0".into(),
            },
            Wire {
                location: Location { row: 46, col: 0 },
                name: "JCDIVX_CLKDIV0".into(),
            },
        ]);
    }
    let refs: Vec<_> = wires.iter().cloned().map(Reference::Wire).collect();
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(board::pin("G2"), Logic::Zero)],
            probes: refs.clone(),
            options: Options {
                trace_capacity: 8192,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    let rules: Vec<_> = c
        .report
        .resources
        .iter()
        .filter(|r| r.family == "ECLKSYNCB")
        .map(|r| r.rule.as_str())
        .collect();
    assert_eq!(rules.len(), if hi13 { 2 } else { 1 });
    assert_eq!(
        rules
            .iter()
            .filter(|r| r.contains("stages initially 0"))
            .count(),
        1
    );
    if hi13 {
        assert_eq!(
            rules
                .iter()
                .filter(|r| r.contains("stages initially 1"))
                .count(),
            1
        );
    }
    let probes: Vec<_> = refs.iter().map(|r| c.probes[&r.label()]).collect();
    let clock = c.drivers["pin:G2"];
    let mut fast = FastSim::new(&c.kernel).unwrap();
    fast.clock(clock, Time::from_ps(board::PERIOD_PS)).unwrap();
    fast.trace(&probes).unwrap();
    c.kernel.drain_trace();
    let mut previous: Vec<_> = probes.iter().map(|&p| c.kernel.value(p).unwrap()).collect();
    let mut counts = vec![[0u64; 2]; probes.len()];
    for end in (128..=1_048_576u64).step_by(128) {
        for cycle in end - 127..=end {
            c.kernel
                .schedule(
                    Time::from_ps((2 * cycle - 1) * board::PERIOD_PS / 2),
                    clock,
                    Logic::One,
                )
                .unwrap();
            c.kernel
                .schedule(Time::from_ps(cycle * board::PERIOD_PS), clock, Logic::Zero)
                .unwrap();
        }
        let until = Time::from_ps(end * board::PERIOD_PS);
        c.kernel.run_until(until).unwrap();
        fast.run_until(until).unwrap();
        let event = c.kernel.drain_trace();
        let trace = fast.take_trace();
        for (k, wire) in wires.iter().enumerate() {
            let expected: Vec<_> = event
                .iter()
                .filter(|t| t.wire == *wire)
                .map(|t| {
                    let (n, d) = t.time.ratio();
                    (n / d, t.value)
                })
                .collect();
            let actual: Vec<_> = trace
                .times_ps
                .iter()
                .zip(trace.rows.chunks_exact(probes.len()))
                .filter_map(|(&ps, row)| {
                    let value = Logic::from_bool(row[k] == 1);
                    let changed = value != previous[k];
                    previous[k] = value;
                    changed.then_some((ps, value))
                })
                .collect();
            assert_eq!(actual, expected, "{series}: trace {wire:?}, cycle {end}");
            for (_, value) in expected {
                counts[k][usize::from(value == Logic::Zero)] += 1;
            }
            assert_eq!(
                fast.value(probes[k]),
                Some(c.kernel.value(probes[k]).unwrap())
            );
        }
        if end == 524288 || end == 1048576 {
            assert_eq!(counts[0], [end, end]);
            assert_eq!(
                counts[2],
                [end / 2, end / 2],
                "{series} P/sync_2 board counts"
            );
            if hi13 {
                assert_eq!(
                    counts[3],
                    [end - 64, end - 64],
                    "DCCA qualification G_in=A=I-64"
                );
                assert_eq!(
                    counts[5],
                    [(end - 68) / 2, (end - 68) / 2],
                    "HI13 G board counts"
                );
            }
        }
    }
    // Verify the explicit expectations against every preserved raw board load.
    let directory = evidence
        .join(series)
        .join(if hi13 { "run_1" } else { "run_2" });
    for load in 1..=3 {
        let name = if hi13 {
            format!("hi13.load-{load:03}.bin")
        } else {
            format!("hi12-sync_2.load-{load:03}.bin")
        };
        let raw = std::fs::read(directory.join(name)).unwrap();
        let text = String::from_utf8_lossy(&raw);
        let prefix = if hi13 { "HCD " } else { "HC3 " };
        let mut records = 0;
        for line in text.split("\r\n").filter_map(|s| s.strip_prefix(prefix)) {
            let expected: Vec<u64> = if hi13 {
                vec![
                    524288, 524224, 64, 524224, 262144, 262144, 262110, 262110, 1048576, 1048512,
                    64, 1048512, 524288, 524288, 524254, 524254,
                ]
            } else {
                vec![524288, 262144, 262144, 1048576, 524288, 524288]
            };
            if line.len() < expected.len() * 8 {
                continue;
            }
            for (k, wanted) in expected.into_iter().enumerate() {
                assert_eq!(
                    u64::from_str_radix(&line[k * 8..k * 8 + 8], 16).unwrap(),
                    wanted
                );
            }
            records += 1;
        }
        assert!(records > 0, "{series} load {load}: no board record");
    }
}

#[test]
#[ignore = "requires FPGA tools, clang, native image reader and Trellis database"]
fn hi13_both_lanes_match_board_counts_and_equal_traces() {
    replay(true);
}

#[test]
#[ignore = "requires FPGA tools, clang, native image reader and Trellis database"]
fn hi12_sync_2_matches_board_counts_and_equal_traces() {
    replay(false);
}
