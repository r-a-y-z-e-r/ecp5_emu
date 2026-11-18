//! IO registers with a clock enable, synchronous and asynchronous set/reset
//! and the global reset, as HI05 measured them on the board
//! (docs/inventory/LFE5U-85F/hi05_ioreg), on a synthetic plan: the rules, and
//! fast mode against the event kernel on random traffic.
mod common;
use common::Fixture;
use ecp5_emu::{
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{Builder, DriverId, IoLogicConfig, IoPath, Kernel, SignalId, Strength, Time},
    logic::Logic::{self, *},
    model::topology::{Location, Wire},
    prepared::ConfiguredDevice,
};
use serde_json::json;
use std::sync::Arc;

fn configured(f: &Fixture) -> Arc<ConfiguredDevice> {
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F",
        "frames":4,"bits_per_frame":16,"encoding":"frame_byte_lsb0",
        "cram_hex":"0000000000000000","bram_data":{},
        "origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,
        "native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalogue = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[]})).unwrap()],
        "LFE5U-85F",
        4,
        16,
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap())
}

/// Driven inputs, in this order.
const INPUTS: [&str; 5] = ["CLK", "D", "CE", "SR", "GSR"];

/// The four kinds yosys maps IFS/OFS1P3{B,D,I,J}X to: (name, REGSET,
/// synchronous, GSR enabled), as in HI05.
const KINDS: [(&str, Logic, bool, bool); 4] = [
    ("B", One, false, true),
    ("D", Zero, false, false),
    ("I", Zero, true, true),
    ("J", One, true, false),
];

struct Bench {
    kernel: Kernel,
    drivers: [DriverId; 5],
    outputs: [SignalId; 4],
}

fn bench(f: &Fixture) -> Bench {
    let device = configured(f);
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut b = Builder::new(device).unwrap();
    let signal = |b: &mut Builder, name: &str| {
        b.signal(Wire {
            location: Location { row: 1, col: 1 },
            name: name.into(),
        })
        .unwrap()
    };
    let inputs = INPUTS.map(|n| signal(&mut b, n));
    let drivers = std::array::from_fn(|k| {
        b.external_driver(inputs[k], INPUTS[k], Strength::Strong, Zero)
            .unwrap()
    });
    let [clk, d, ce, sr, gsr] = inputs;
    let outputs = KINDS.map(|(name, _, _, _)| signal(&mut b, &format!("Q{name}")));
    for (k, &(name, value, sync, gsr_enabled)) in KINDS.iter().enumerate() {
        b.iologic(IoLogicConfig {
            resource: plc,
            name,
            path: IoPath::Register {
                data: d.into(),
                output: outputs[k],
                value,
                enable: ce.into(),
                sync_reset: sync.then_some(sr.into()),
                global_reset: gsr_enabled.then_some(gsr.into()),
            },
            clock: clk.into(),
            reset: (!sync).then_some(sr.into()),
        })
        .unwrap();
    }
    for &s in &outputs {
        b.watch(s).unwrap();
    }
    Bench {
        kernel: b.build().unwrap(),
        drivers,
        outputs,
    }
}

/// Applies `steps` of (input index, level) changes, one step per time unit;
/// returns the four outputs after each step.
fn run(steps: &[&[(usize, bool)]]) -> Vec<[Logic; 4]> {
    let f = Fixture::with_kernel();
    let mut b = bench(&f);
    let mut out = Vec::new();
    for (n, step) in steps.iter().enumerate() {
        let time = Time::from_ps((n as u64 + 1) * 1000);
        let batch: Vec<_> = step
            .iter()
            .map(|&(k, v)| (b.drivers[k], Logic::from_bool(v)))
            .collect();
        b.kernel.schedule_batch(time, &batch, None).unwrap();
        b.kernel.run_until(time).unwrap();
        out.push(b.outputs.map(|s| b.kernel.value(s).unwrap()));
    }
    out
}

const RISE: (usize, bool) = (0, true);
const FALL: (usize, bool) = (0, false);

#[test]
fn io_registers_start_at_regset_and_follow_ce_and_both_resets() {
    let o = run(&[
        &[],                 // 0: start, REGSET: B 1, D 0, I 0, J 1
        &[(1, true)],        // 1: D = 1, CE = 0
        &[RISE],             // 2: no enable, nothing captured
        &[FALL, (2, true)],  // 3: CE = 1
        &[RISE],             // 4: all capture 1
        &[FALL, (1, false)], // 5: D = 0
        &[(3, true)],        // 6: SR: B and D set/reset at once, I and J wait
        &[RISE],             // 7: I, J take REGSET at the edge
        &[FALL, (2, false)], // 8: CE = 0
        &[RISE],             // 9: SR still high: all hold REGSET, CE or not
        &[FALL, (3, false)], // 10: SR released
        &[RISE],             // 11: CE = 0: hold
        &[FALL, (2, true)],  // 12
        &[RISE],             // 13: capture D = 0
        &[FALL, (4, true)],  // 14: GSR: B and I at REGSET at once
        &[RISE],             // 15: an edge with GSR high holds REGSET
        &[FALL, (4, false)], // 16: GSR released after the edge, as a register does
        &[RISE],             // 17: capture D = 0 again
    ]);
    let want: [[Logic; 4]; 18] = [
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, One, One, One],
        [One, One, One, One],
        [One, Zero, One, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [One, Zero, Zero, One],
        [Zero, Zero, Zero, Zero],
        [One, Zero, Zero, Zero],
        [One, Zero, Zero, Zero],
        [One, Zero, Zero, Zero],
        [Zero, Zero, Zero, Zero],
    ];
    for (n, (got, want)) in o.iter().zip(want).enumerate() {
        assert_eq!(*got, want, "step {n}");
    }
}

#[test]
fn fast_mode_matches_the_kernel_on_random_io_register_traffic() {
    let f = Fixture::with_kernel();
    let mut b = bench(&f);
    let mut fast = FastSim::new(&b.kernel).unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut level = [false; 5];
    let mut changes = [0_u32; 4];
    let mut last = [Zero; 4];
    for step in 1..=30_000_u64 {
        let mut set = Vec::new();
        for (k, (level, &driver)) in level.iter_mut().zip(&b.drivers).enumerate() {
            let odds = match k {
                0 => 2,
                3 => 9,
                4 => 25,
                _ => 3,
            };
            if next() % odds == 0 {
                *level = !*level;
                set.push((driver, *level));
            }
        }
        let time = Time::from_ps(step * 1000);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        b.kernel.schedule_batch(time, &batch, None).unwrap();
        b.kernel.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        for (k, &s) in b.outputs.iter().enumerate() {
            let v = b.kernel.value(s).unwrap();
            assert_eq!(
                fast.value(s),
                Some(v),
                "step {step}, register {}",
                KINDS[k].0
            );
            if v != last[k] {
                changes[k] += 1;
                last[k] = v;
            }
        }
    }
    assert!(changes.iter().all(|&c| c > 300), "{changes:?}");
}
