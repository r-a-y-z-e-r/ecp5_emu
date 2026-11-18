//! The edge-clock blocks as HI04 measured them on the board
//! (docs/inventory/LFE5U-85F/hi04_eclk): ECLKSYNCB's STOP synchronizer and
//! CLKDIVF /2 and /3.5 with RST and ALIGNWD, on a synthetic kernel.
//!
//! Time is counted in units of 40 ns, as the 25 MHz samples of the board run:
//! ECLKI rises at 8m+4 and falls at 8m+8, and a control set "at a" changes
//! at a. A value "at u" is the one held from u to u+1.
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

const UNIT_PS: u64 = 40_000;

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
const INPUTS: [&str; 6] = ["ECLKI", "STOP", "RST2", "RST35", "AL2", "AL35"];

struct Bench {
    kernel: Kernel,
    drivers: [DriverId; 6],
    /// ECLKO, CDIVX of /2, CDIVX of /3.5, the synchronizer's gate.
    watched: [SignalId; 4],
}

/// ECLKSYNCB on ECLKI, and a /2 and a /3.5 CLKDIVF on its ECLKO, as the
/// planner builds them (compile/eclk.rs).
fn bench(f: &Fixture) -> Bench {
    bench_initial(f, One)
}

fn bench_initial(f: &Fixture, initial: Logic) -> Bench {
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
    let [eclki, stop, rst2, rst35, al2, al35] = inputs;
    let [eclko, cd2, cd35, gate] = ["ECLKO", "CD2", "CD35", "GATE"].map(|n| signal(&mut b, n));
    let io = |name, path, clock: SignalId, reset: Option<SignalId>| IoLogicConfig {
        resource: plc,
        name,
        path,
        clock: clock.into(),
        reset: reset.map(Input::from),
    };
    b.iologic(io(
        "SYNC",
        IoPath::StopSync {
            stop: stop.into(),
            gate,
            initial,
        },
        eclki,
        None,
    ))
    .unwrap();
    b.mux(
        "eclko",
        gate.into(),
        eclki.into(),
        Input::Constant(Zero),
        eclko,
    )
    .unwrap();
    for (name, align, output, seven_halves, reset) in [
        ("CD2", al2, cd2, false, rst2),
        ("CD35", al35, cd35, true, rst35),
    ] {
        b.iologic(io(
            name,
            IoPath::Divider {
                align: align.into(),
                output,
                seven_halves,
            },
            eclko,
            Some(reset),
        ))
        .unwrap();
    }
    let watched = [eclko, cd2, cd35, gate];
    for &s in &watched {
        b.watch(s).unwrap();
    }
    Bench {
        kernel: b.build().unwrap(),
        drivers,
        watched,
    }
}

/// Runs the kernel for `units` with ECLKI free-running and the controls
/// high over the given (input index, from, to) spans; returns the units at
/// which ECLKO, /2 and /3.5 rose and fell.
fn edges(controls: &[(usize, u64, u64)], units: u64) -> [(Vec<u64>, Vec<u64>); 3] {
    edges_initial(controls, units, One)
}

fn edges_initial(
    controls: &[(usize, u64, u64)],
    units: u64,
    initial: Logic,
) -> [(Vec<u64>, Vec<u64>); 3] {
    let f = Fixture::with_kernel();
    let mut b = bench_initial(&f, initial);
    let mut out: [(Vec<u64>, Vec<u64>); 3] = Default::default();
    let mut last = [Zero; 3];
    for u in 1..=units {
        let time = Time::from_ps(u * UNIT_PS);
        let mut batch = Vec::new();
        if u % 4 == 0 {
            batch.push((b.drivers[0], Logic::from_bool(u % 8 == 4)));
        }
        for &(k, from, to) in controls {
            if u == from || u == to {
                batch.push((b.drivers[k], Logic::from_bool(u == from)));
            }
        }
        b.kernel.schedule_batch(time, &batch, None).unwrap();
        b.kernel.run_until(time).unwrap();
        for k in 0..3 {
            let v = b.kernel.value(b.watched[k]).unwrap();
            if v != last[k] {
                if v == One {
                    out[k].0.push(u);
                } else {
                    out[k].1.push(u);
                }
                last[k] = v;
            }
        }
    }
    out
}

fn first(v: &[u64], n: usize) -> Vec<u64> {
    v.iter().copied().take(n).collect()
}

#[test]
fn eclko_and_the_dividers_start_as_on_the_board() {
    // HI04 E00: the STOP synchronizer starts closed, so ECLKO starts with the
    // fifth rising edge of ECLKI; both dividers rise with it.
    let [eclko, cd2, cd35] = edges(&[], 120);
    assert_eq!(first(&eclko.0, 3), [36, 44, 52]);
    assert_eq!(first(&cd2.0, 3), [36, 52, 68]);
    assert_eq!(first(&cd2.1, 3), [44, 60, 76]);
    // /3.5: high for four ECLK halves, low for three.
    assert_eq!(first(&cd35.0, 4), [36, 64, 92, 120]);
    assert_eq!(first(&cd35.1, 3), [52, 80, 108]);
}

#[test]
fn stop_takes_effect_at_the_fourth_falling_edge() {
    // HI04 E01 (STOP from 33 for 64) and HI04b E01 (from 290 for 8), both
    // relative to an ECLKI period: ECLKO stops low after the fourth falling
    // edge that sees STOP and restarts after the fourth that sees it low.
    let [eclko, ..] = edges(&[(1, 256 + 33, 256 + 97)], 512);
    let gap: Vec<u64> = eclko
        .0
        .windows(2)
        .filter(|w| w[1] - w[0] > 8)
        .map(|w| w[0])
        .collect();
    assert_eq!(gap, [256 + 60]);
    assert!(eclko.0.contains(&(256 + 132)) && !eclko.0.contains(&(256 + 124)));
    assert!(eclko.1.contains(&(256 + 64)) && !eclko.1.contains(&(256 + 72)));
    let [eclko, ..] = edges(&[(1, 256 + 34, 256 + 42)], 512);
    let missing: Vec<u64> = (256 + 4..512)
        .step_by(8)
        .filter(|u| !eclko.0.contains(u))
        .collect();
    assert_eq!(missing, [256 + 68]);
}

#[test]
fn reset_clears_the_divider_and_consumes_one_rising_edge() {
    // As HI04 E07-E0E: a one-unit RST at 294, in an ECLKI high half, while
    // both CDIVX are high (/2 rose at 292, /3.5 at 288). Both fall at once;
    // the rising edge at 300 is consumed and both rise at 308. Then /2
    // toggles on, and /3.5 is high for four halves and low for three.
    let [_, cd2, cd35] = edges(&[(2, 294, 295), (3, 294, 295)], 400);
    let around = |v: &[u64]| -> Vec<u64> {
        v.iter()
            .copied()
            .filter(|u| (288..340).contains(u))
            .collect()
    };
    assert_eq!(around(&cd2.0), [292, 308, 324]);
    assert_eq!(around(&cd2.1), [294, 316, 332]);
    assert_eq!(around(&cd35.0), [288, 308, 336]);
    assert_eq!(around(&cd35.1), [294, 324]);
}

#[test]
fn alignwd_slips_one_eclk_period_two_rising_edges_later() {
    // HI04b E1C, 256 units after the start: /3.5 reset at 10, ALIGNWD from
    // 66 for 8 (sampled at the rising edge 68): the fall due at 92 comes at
    // 100, and everything after it one ECLK period later.
    let w = 256;
    let [_, _, cd35] = edges(&[(3, w + 10, w + 11), (5, w + 66, w + 74)], w + 160);
    let window =
        |v: &[u64]| -> Vec<u64> { v.iter().filter(|&&u| u > w + 10).map(|u| u - w).collect() };
    assert_eq!(window(&cd35.0), [20, 48, 76, 112, 140]);
    assert_eq!(window(&cd35.1), [36, 64, 100, 128, 156]);
    // /2 slips on the second ALIGNWD after a reset only (HI04 E11, E12;
    // HI04b E10-E1B): the second, sampled at 132, holds CDIVX low over the
    // rising edge at 148.
    let [_, cd2, _] = edges(
        &[
            (2, w + 10, w + 11),
            (4, w + 66, w + 74),
            (4, w + 130, w + 138),
        ],
        w + 200,
    );
    let expected: Vec<u64> = (20..200)
        .step_by(16)
        .map(|u| if u >= 148 { u + 8 } else { u })
        .filter(|&u| u < 200)
        .collect();
    assert_eq!(window(&cd2.0), expected);
}

#[test]
fn fast_mode_matches_the_kernel_on_random_edge_clock_controls() {
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
    let mut level = [false; 6];
    let mut moved = [0_u32; 3];
    let mut last = [Zero; 3];
    for u in 1..=40_000_u64 {
        let mut set = Vec::new();
        if u % 4 == 0 {
            level[0] = u % 8 == 4;
            set.push((b.drivers[0], level[0]));
        } else {
            // Controls change between edges, as on the board; STOP and the
            // resets rarely, ALIGNWD more often.
            for (k, (level, &driver)) in level.iter_mut().zip(&b.drivers).enumerate().skip(1) {
                let odds = if k >= 4 { 40 } else { 150 };
                if next() % odds == 0 {
                    *level = !*level;
                    set.push((driver, *level));
                }
            }
        }
        let time = Time::from_ps(u * UNIT_PS);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        b.kernel.schedule_batch(time, &batch, None).unwrap();
        b.kernel.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        for (k, &s) in b.watched.iter().enumerate() {
            let v = b.kernel.value(s).unwrap();
            assert_eq!(fast.value(s), Some(v), "unit {u}, signal {k}");
            if k < 3 && v != last[k] {
                moved[k] += 1;
                last[k] = v;
            }
        }
    }
    // The clocks did run, through stops and resets.
    assert!(moved.iter().all(|&m| m > 300), "{moved:?}");
}

#[test]
fn cleared_pin_start_keeps_the_four_stage_runtime_stop_pipeline() {
    let controls = [(1, 33, 97)];
    let [pin, cd2, _] = edges_initial(&controls, 160, Zero);
    let [fabric, ..] = edges_initial(&controls, 160, One);
    assert_eq!(first(&pin.0, 5), [4, 12, 20, 28, 36]);
    assert_eq!(first(&cd2.0, 3), [4, 20, 36]);
    // STOP sees falls 40,48,56,64; release sees 104,112,120,128.
    assert!(pin.0.contains(&60));
    assert!(!pin.0.contains(&68));
    assert!(!pin.0.contains(&124));
    assert!(pin.0.contains(&132));
    assert_eq!(
        pin.0
            .iter()
            .copied()
            .filter(|u| *u >= 36)
            .collect::<Vec<_>>(),
        fabric.0
    );
    assert_eq!(
        pin.1
            .iter()
            .copied()
            .filter(|u| *u >= 40)
            .collect::<Vec<_>>(),
        fabric.1
    );
}
