//! Fast mode against the event kernel when an asynchronous reset changes in
//! the same instant as a clock edge. The kernel captures every register of a
//! wave from the state before it, and a register's own asynchronous reset
//! wins over its own edge (kernel/mod.rs); fast mode's `converge` must do the
//! same: the captures first, then `asynchronous()`.
mod common;
use common::Fixture;
use ecp5_emu::{
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        Builder, DriverId, Edge, FlipFlop, Input, Reset, ResetKind, SignalId, Strength, Time,
    },
    logic::Logic::{self, *},
    model::topology::{Location, Wire},
    prepared::ConfiguredDevice,
};
use serde_json::json;
use std::sync::Arc;

fn device() -> (Fixture, Arc<ConfiguredDevice>) {
    let f = Fixture::with_compiler();
    f.seal();
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,"encoding":"frame_byte_lsb0","cram_hex":"0000000000000000","bram_data":{},"origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalog = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[]})).unwrap()],
        "LFE5U-85F",
        4,
        16,
    )
    .unwrap();
    let device =
        Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalog).unwrap());
    (f, device)
}

/// FF1: D=1 on C1, asynchronous reset R to 0. FF2: D=Q1 on C2.
struct Pair {
    k: ecp5_emu::kernel::Kernel,
    fast: FastSim,
    q: [SignalId; 2],
    c1: DriverId,
    c2: DriverId,
    r: DriverId,
    time: u64,
}

impl Pair {
    fn new() -> Self {
        Self::with_preload(false)
    }
    fn with_preload(preload: bool) -> Self {
        let (_f, device) = device();
        let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let ffs: Vec<_> = device
            .model()
            .nodes(plc)
            .filter(|n| n.kind == "TRELLIS_FF")
            .map(|n| n.id)
            .collect();
        let mut b = Builder::new(device).unwrap();
        let ext = |b: &mut Builder, name: &str| {
            let s = b
                .signal(Wire {
                    location: Location { row: 1, col: 1 },
                    name: name.into(),
                })
                .unwrap();
            (
                s,
                b.external_driver(s, name, Strength::Strong, Zero).unwrap(),
            )
        };
        let (c1, dc1) = ext(&mut b, "C1");
        let (c2, dc2) = ext(&mut b, "C2");
        let (r, dr) = ext(&mut b, "R");
        let q1 = b
            .flip_flop(FlipFlop {
                preload: preload.then_some(r.into()),
                node: ffs[0],
                data: Input::Constant(One),
                clock: c1.into(),
                enable: Input::Constant(One),
                edge: Edge::Rising,
                reset: Some(Reset {
                    input: if preload {
                        Input::Constant(One)
                    } else {
                        r.into()
                    },
                    kind: ResetKind::Asynchronous,
                    value: Zero,
                }),
            })
            .unwrap();
        let q2 = b
            .flip_flop(FlipFlop {
                preload: None,
                node: ffs[1],
                data: q1.into(),
                clock: c2.into(),
                enable: Input::Constant(One),
                edge: Edge::Rising,
                reset: None,
            })
            .unwrap();
        b.watch(q1).unwrap();
        b.watch(q2).unwrap();
        let mut k = b.build().unwrap();
        // Both clocks once: Q1 = 1, Q2 known.
        let mut time = 0;
        for level in [true, false, true, false] {
            time += 1;
            let set = [
                (dc1, Logic::from_bool(level)),
                (dc2, Logic::from_bool(level)),
            ];
            k.schedule_batch(Time::from_ps(time), &set, None).unwrap();
            k.run_until(Time::from_ps(time)).unwrap();
        }
        let initial = if preload { [Zero, Zero] } else { [One, One] };
        assert_eq!([k.value(q1).unwrap(), k.value(q2).unwrap()], initial);
        let fast = FastSim::new(&k).unwrap();
        Self {
            k,
            fast,
            q: [q1, q2],
            c1: dc1,
            c2: dc2,
            r: dr,
            time,
        }
    }
    /// One instant in both executors; returns the kernel's Q1, Q2 after it.
    fn instant(&mut self, set: &[(DriverId, bool)]) -> [Logic; 2] {
        self.time += 1;
        let at = Time::from_ps(self.time);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        self.k.schedule_batch(at, &batch, None).unwrap();
        self.k.run_until(at).unwrap();
        self.fast.set_at(at, set).unwrap();
        let kernel = self.q.map(|q| self.k.value(q).unwrap());
        let fast = self.q.map(|q| self.fast.value(q).unwrap());
        assert_eq!(fast, kernel, "fast mode after {set:?}");
        kernel
    }
}

/// R and FF2's clock rise together: FF2 captures Q1 from before the reset.
#[test]
#[ignore = "requires clang"]
fn a_reset_at_a_downstream_edge_leaves_the_capture_the_old_value() {
    let mut p = Pair::new();
    let (r, c2) = (p.r, p.c2);
    assert_eq!(p.instant(&[(r, true), (c2, true)]), [Zero, One]);
    // The next edge of C2 takes the reset Q1.
    p.instant(&[(c2, false)]);
    assert_eq!(p.instant(&[(c2, true)]), [Zero, Zero]);
}

/// R rises with FF1's own clock: the reset wins over FF1's own capture.
#[test]
#[ignore = "requires clang"]
fn a_reset_wins_over_its_registers_own_edge() {
    let mut p = Pair::new();
    let (r, c1) = (p.r, p.c1);
    assert_eq!(p.instant(&[(r, true), (c1, true)]), [Zero, One]);
    // Released with an edge of C1 in the same instant: FF1 captures D = 1.
    p.instant(&[(c1, false)]);
    assert_eq!(p.instant(&[(r, false), (c1, true)]), [One, One]);
}

/// The configured M value, rather than REGSET, initializes both FF kinds.
fn preload_pair(m: Logic) -> (ecp5_emu::kernel::Kernel, [SignalId; 2], [DriverId; 5]) {
    let (_fixture, device) = device();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let nodes: Vec<_> = device
        .model()
        .nodes(plc)
        .filter(|n| n.kind == "TRELLIS_FF")
        .map(|n| n.id)
        .collect();
    let mut b = Builder::new(device).unwrap();
    let mut drivers = Vec::new();
    let pins: Vec<Input> = [
        ("CLK", Zero),
        ("DI", One),
        ("M", m),
        ("CE", Zero),
        ("LSR", Zero),
    ]
    .into_iter()
    .map(|(name, value)| {
        let s = b
            .signal(Wire {
                location: Location { row: 1, col: 1 },
                name: name.into(),
            })
            .unwrap();
        drivers.push(b.external_driver(s, name, Strength::Strong, value).unwrap());
        s.into()
    })
    .collect();
    let q = std::array::from_fn(|i| {
        let q = b
            .flip_flop(FlipFlop {
                node: nodes[i],
                data: pins[1],
                preload: Some(pins[2]),
                clock: pins[0],
                enable: pins[3],
                edge: Edge::Rising,
                reset: Some(Reset {
                    input: pins[4],
                    kind: if i == 0 {
                        ResetKind::Synchronous
                    } else {
                        ResetKind::Asynchronous
                    },
                    value: Zero,
                }),
            })
            .unwrap();
        b.initial_preload_register(nodes[i]).unwrap();
        b.watch(q).unwrap();
        q
    });
    (b.build().unwrap(), q, drivers.try_into().unwrap())
}

#[test]
#[ignore = "requires clang"]
fn preload_priority_level_following_and_release_race() {
    let (mut k, q, [clk, di, m, ce, lsr]) = preload_pair(One);
    let mut fast = FastSim::new(&k).unwrap();
    assert_eq!(q.map(|q| fast.value(q).unwrap()), [One, One]);
    for (time, changes, want) in [
        (1, vec![(lsr, true), (m, false)], [One, Zero]),
        (2, vec![(clk, true)], [Zero, Zero]), // PRLD overrides CE=0 and REGSET.
        (3, vec![(clk, false), (m, true)], [Zero, One]),
        (4, vec![(clk, true)], [One, One]),
        (5, vec![(clk, false), (lsr, false)], [One, One]),
        (6, vec![(m, false)], [One, One]), // A different instant is no race.
        (7, vec![(ce, true), (di, false), (clk, true)], [Zero, Zero]),
        (8, vec![(clk, false), (lsr, true), (m, true)], [Zero, One]),
    ] {
        let at = Time::from_ps(time);
        let batch: Vec<_> = changes
            .iter()
            .map(|&(d, v)| (d, Logic::from_bool(v)))
            .collect();
        k.schedule_batch(at, &batch, None).unwrap();
        k.run_until(at).unwrap();
        fast.set_at(at, &changes).unwrap();
        assert_eq!(q.map(|q| k.value(q).unwrap()), want, "kernel at {time}");
        assert_eq!(q.map(|q| fast.value(q).unwrap()), want, "fast at {time}");
    }
    let at = Time::from_ps(9);
    k.schedule_batch(at, &[(lsr, Zero), (m, Zero)], None)
        .unwrap();
    k.run_until(at).unwrap();
    assert_eq!(k.value(q[1]).unwrap(), X);
    let error = fast.set_at(at, &[(lsr, false), (m, false)]).unwrap_err();
    assert!(
        error.to_string().contains("asynchronous PRLD register"),
        "{error:#}"
    );
}

#[test]
fn configured_unknown_preload_is_starts_unknown() {
    for m in [X, Z] {
        let (k, q, _) = preload_pair(m);
        assert_eq!(q.map(|q| k.value(q).unwrap()), [X, X]);
        let error = k.fast_program().err().unwrap();
        assert!(
            error
                .downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                .is_some(),
            "{error:#}"
        );
    }
}

/// M and falling LSR change in different capture waves of one timestamp.
#[test]
#[ignore = "requires clang"]
fn preload_release_race_spans_waves_in_either_order() {
    for release_first in [false, true] {
        let (_fixture, device) = device();
        let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let nodes: Vec<_> = device
            .model()
            .nodes(plc)
            .filter(|n| n.kind == "TRELLIS_FF")
            .map(|n| n.id)
            .collect();
        let mut state = ecp5_emu::state::DeviceState::new(&device).unwrap();
        state.write_register(nodes[0], Zero).unwrap();
        let mut b = Builder::with_state(device, state).unwrap();
        let mut add = |name: &str, value| {
            let s = b
                .signal(Wire {
                    location: Location { row: 1, col: 1 },
                    name: name.into(),
                })
                .unwrap();
            let d = b.external_driver(s, name, Strength::Strong, value).unwrap();
            (s, d)
        };
        let (clock, clk) = add("CLK", Zero);
        let (control, ctl) = add("CONTROL", if release_first { One } else { Zero });
        let source = b
            .flip_flop(FlipFlop {
                node: nodes[0],
                data: Input::Constant(One),
                preload: None,
                clock: clock.into(),
                enable: Input::Constant(One),
                edge: Edge::Rising,
                reset: None,
            })
            .unwrap();
        let (m, lsr) = if release_first {
            (source.into(), control.into())
        } else {
            (control.into(), Input::Inverted(source))
        };
        let q = b
            .flip_flop(FlipFlop {
                node: nodes[1],
                data: Input::Constant(Zero),
                preload: Some(m),
                clock: clock.into(),
                enable: Input::Constant(Zero),
                edge: Edge::Rising,
                reset: Some(Reset {
                    input: lsr,
                    kind: ResetKind::Asynchronous,
                    value: One,
                }),
            })
            .unwrap();
        b.initial_preload_register(nodes[1]).unwrap();
        b.watch(q).unwrap();
        let mut k = b.build().unwrap();
        let mut fast = FastSim::new(&k).unwrap();
        let at = Time::from_ps(1);
        let level = !release_first;
        k.schedule_batch(at, &[(clk, One), (ctl, Logic::from_bool(level))], None)
            .unwrap();
        k.run_until(at).unwrap();
        assert_eq!(k.value(q).unwrap(), X);
        let error = fast.set_at(at, &[(clk, true), (ctl, level)]).unwrap_err();
        assert!(
            error.to_string().contains("asynchronous PRLD register"),
            "{error:#}"
        );
    }
}

/// M changes with a downstream edge: the downstream FF still samples old Q.
#[test]
#[ignore = "requires clang"]
fn asynchronous_preload_and_downstream_capture_share_the_old_wave() {
    let mut p = Pair::with_preload(true);
    let (m, c2) = (p.r, p.c2);
    assert_eq!(p.instant(&[(m, true), (c2, true)]), [One, Zero]);
    p.instant(&[(c2, false)]);
    assert_eq!(p.instant(&[(c2, true)]), [One, One]);
}

#[test]
#[ignore = "requires clang"]
fn asynchronous_preload_tracks_m_after_capture_settles() {
    for global in [false, true] {
        let (_fixture, device) = device();
        let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let nodes: Vec<_> = device
            .model()
            .nodes(plc)
            .filter(|n| n.kind == "TRELLIS_FF")
            .map(|n| n.id)
            .collect();
        let mut state = ecp5_emu::state::DeviceState::new(&device).unwrap();
        state.write_register(nodes[0], Zero).unwrap();
        let mut b = Builder::with_state(device, state).unwrap();
        let clock = b
            .signal(Wire {
                location: Location { row: 1, col: 1 },
                name: "CLK".into(),
            })
            .unwrap();
        let clk = b
            .external_driver(clock, "CLK", Strength::Strong, Zero)
            .unwrap();
        let source = b
            .flip_flop(FlipFlop {
                node: nodes[0],
                data: Input::Constant(One),
                preload: None,
                clock: clock.into(),
                enable: Input::Constant(One),
                edge: Edge::Rising,
                reset: None,
            })
            .unwrap();
        let m = b
            .signal(Wire {
                location: Location { row: 1, col: 1 },
                name: "M".into(),
            })
            .unwrap();
        b.mux(
            "M logic",
            source.into(),
            Input::Constant(One),
            Input::Constant(Zero),
            m,
        )
        .unwrap();
        let q = b
            .flip_flop_with_global_reset(
                FlipFlop {
                    node: nodes[1],
                    data: Input::Constant(Zero),
                    preload: Some(Input::Inverted(m)),
                    clock: Input::Constant(Zero),
                    enable: Input::Constant(Zero),
                    edge: Edge::Rising,
                    reset: Some(Reset {
                        input: Input::Constant(if global { Zero } else { One }),
                        kind: ResetKind::Asynchronous,
                        value: One,
                    }),
                },
                global.then_some(ecp5_emu::kernel::GlobalReset {
                    input: Input::Constant(One),
                    value: Zero,
                }),
            )
            .unwrap();
        b.initial_preload_register(nodes[1]).unwrap();
        b.watch(q).unwrap();
        let mut k = b.build().unwrap();
        let mut fast = FastSim::new(&k).unwrap();
        assert_eq!(fast.value(q), Some(Zero));
        let at = Time::from_ps(1);
        k.schedule(at, clk, One).unwrap();
        k.run_until(at).unwrap();
        fast.set_at(at, &[(clk, true)]).unwrap();
        assert_eq!(k.value(q).unwrap(), One);
        assert_eq!(fast.value(q), Some(One));
    }
}

/// Shared demanded cones on opposite edges still read the old register
/// state when both groups fire, including when only one enable is active.
#[test]
#[ignore = "requires clang"]
fn simultaneous_groups_share_demanded_gates_before_committing() {
    let (_fixture, device) = device();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let nodes: Vec<_> = device
        .model()
        .nodes(plc)
        .filter(|n| n.kind == "TRELLIS_FF")
        .map(|n| n.id)
        .collect();
    let mut state = ecp5_emu::state::DeviceState::new(&device).unwrap();
    for &node in &nodes {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let mut net = |name: &str| {
        b.signal(Wire {
            location: Location { row: 1, col: 1 },
            name: name.into(),
        })
        .unwrap()
    };
    let inputs = ["CA", "CB", "EA", "EB", "SELECT"].map(&mut net);
    let feedback = net("FEEDBACK");
    let gates: Vec<_> = (0..7).map(|i| net(&format!("DEMAND{i}"))).collect();
    let drivers: [_; 5] = std::array::from_fn(|i| {
        b.external_driver(inputs[i], &format!("input{i}"), Strength::Strong, Zero)
            .unwrap()
    });
    let mut shared = feedback;
    for (i, &out) in gates.iter().enumerate() {
        b.mux(
            &format!("demand{i}"),
            inputs[4].into(),
            shared.into(),
            Input::Inverted(shared),
            out,
        )
        .unwrap();
        shared = out;
    }
    let qa = b
        .flip_flop(FlipFlop {
            node: nodes[0],
            data: shared.into(),
            preload: None,
            clock: inputs[0].into(),
            enable: inputs[2].into(),
            edge: Edge::Rising,
            reset: None,
        })
        .unwrap();
    let db = b
        .signal(Wire {
            location: Location { row: 1, col: 1 },
            name: "DB".into(),
        })
        .unwrap();
    b.mux(
        "cross capture",
        qa.into(),
        shared.into(),
        Input::Inverted(shared),
        db,
    )
    .unwrap();
    let qb = b
        .flip_flop(FlipFlop {
            node: nodes[1],
            data: db.into(),
            preload: None,
            clock: inputs[1].into(),
            enable: inputs[3].into(),
            edge: Edge::Falling,
            reset: None,
        })
        .unwrap();
    b.buffer("feedback", qb.into(), feedback).unwrap();
    b.watch(qa).unwrap();
    b.watch(qb).unwrap();
    let mut k = b.build().unwrap();
    let mut fast = FastSim::new(&k).unwrap();
    assert!(fast.program().demanded_gates >= gates.len());
    let mut random = 0x9e37_79b9_7f4a_7c15_u64;
    let mut moved = [0; 2];
    let mut last = [Zero; 2];
    for step in 1..=10_000 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let set: Vec<_> = drivers
            .iter()
            .enumerate()
            .map(|(i, &d)| (d, random >> i & 1 != 0))
            .collect();
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        let at = Time::from_ps(step);
        k.schedule_batch(at, &batch, None).unwrap();
        k.run_until(at).unwrap();
        fast.set_at(at, &set).unwrap();
        for (i, q) in [qa, qb].into_iter().enumerate() {
            let value = k.value(q).unwrap();
            assert_eq!(fast.value(q), Some(value), "step {step}, register {i}");
            moved[i] += usize::from(value != last[i]);
            last[i] = value;
        }
    }
    assert!(moved.iter().all(|&n| n > 100), "{moved:?}");
}
