//! IO latches as HI10 measured them on the board
//! (docs/inventory/LFE5U-85F/hi10_io_latch), on a synthetic plan: the rules,
//! and fast mode against the event kernel on random traffic. The traffic also
//! moves a latch's clock, CE and D in the same step as another domain's edge:
//! a register on CLK2 samples latch Q1, and latch Q2 takes that register.
mod common;
use common::Fixture;
use ecp5_emu::{
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{Builder, DriverId, Input, IoLogicConfig, IoPath, Kernel, SignalId, Strength, Time},
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
const INPUTS: [&str; 5] = ["CLK", "D", "CE", "SR", "CLK2"];

struct Bench {
    kernel: Kernel,
    drivers: [DriverId; 5],
    /// Q1 (latch on D, reset SR), R (register of Q1 on CLK2), Q2 (latch on R,
    /// no reset).
    outputs: [SignalId; 3],
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
    let [clk, d, ce, sr, clk2] = inputs;
    let outputs = ["Q1", "R", "Q2"].map(|n| signal(&mut b, n));
    let [q1, r, q2] = outputs;
    let io = |name, path, clock: SignalId, reset: Option<SignalId>| IoLogicConfig {
        resource: plc,
        name,
        path,
        clock: clock.into(),
        reset: reset.map(Into::into),
    };
    let latch = |data: SignalId, output| IoPath::Latch {
        data: data.into(),
        enable: ce.into(),
        output,
    };
    b.iologic(io("Q1", latch(d, q1), clk, Some(sr))).unwrap();
    let register = IoPath::Register {
        data: q1.into(),
        output: r,
        value: Zero,
        enable: Input::Constant(One),
        sync_reset: None,
        global_reset: None,
    };
    b.iologic(io("R", register, clk2, None)).unwrap();
    b.iologic(io("Q2", latch(r, q2), clk, None)).unwrap();
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
/// returns Q1 after each step.
fn run(steps: &[&[(usize, bool)]]) -> Vec<Logic> {
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
        out.push(b.kernel.value(b.outputs[0]).unwrap());
    }
    out
}

const D: usize = 1;
const CE: usize = 2;
const SR: usize = 3;

#[test]
fn io_latch_is_open_while_its_clock_is_low_and_ce_high() {
    let q = run(&[
        &[],                         // 0: start: 0; CLK low, CE low: closed
        &[(D, true)],                // 1: closed: holds 0
        &[(CE, true)],               // 2: open: D passes at once
        &[(D, false)],               // 3: follows D
        &[(D, true)],                // 4
        &[(0, true)],                // 5: CLK high: closed, holds 1
        &[(D, false)],               // 6: holds 1
        &[(0, false)],               // 7: CLK low: open again, takes D = 0
        &[(D, true), (CE, false)],   // 8: closes as D changes: holds 0
        &[(SR, true)],               // 9: reset at once (already 0)
        &[(CE, true)],               // 10: open, but reset wins
        &[(SR, false), (CE, false)], // 11: reset off as it closes: holds 0
        &[(CE, true)],               // 12: open: D = 1
        &[(SR, true)],               // 13: reset clears at once
    ]);
    let want = [
        Zero, Zero, One, Zero, One, One, One, Zero, Zero, Zero, Zero, Zero, One, Zero,
    ];
    assert_eq!(q, want);
}

#[test]
fn fast_mode_matches_the_kernel_on_random_io_latch_traffic() {
    let f = Fixture::with_kernel();
    let mut b = bench(&f);
    let mut fast = FastSim::new(&b.kernel).unwrap();
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut level = [false; 5];
    let mut changes = [0_u32; 3];
    let mut last = [Zero; 3];
    for step in 1..=30_000_u64 {
        let mut set = Vec::new();
        for (k, (level, &driver)) in level.iter_mut().zip(&b.drivers).enumerate() {
            let odds = match k {
                0 | 4 => 2,
                3 => 9,
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
            assert_eq!(fast.value(s), Some(v), "step {step}, output {k}");
            if v != last[k] {
                changes[k] += 1;
                last[k] = v;
            }
        }
    }
    assert!(changes.iter().all(|&c| c > 300), "{changes:?}");
}
