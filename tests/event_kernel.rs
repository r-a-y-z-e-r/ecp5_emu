mod common;
#[path = "common/tools.rs"]
mod tools;
use common::{write, Fixture};
use ecp5_emu::{
    database::Database,
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        Builder, DcsConfig, DcsMode, DriverId, Edge, FlipFlop, Input, IoLogicConfig, IoPath,
        Kernel, Options, OscillatorConfig, Phase, PllConfig, PllOutput, Reset, ResetKind, SignalId,
        Strength, Time,
    },
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        DeviceModel, NodeId,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

fn configured(f: &Fixture, mask: u16) -> Arc<ConfiguredDevice> {
    // The synthetic PLC INIT has inverted local F0/1 B0..7 at CRAM F2/3 B3..10.
    let mut data = [0_u8; 8];
    for bit in 0..16 {
        if mask & (1 << bit) == 0 {
            let frame = 2 + bit / 8;
            let offset = 3 + bit % 8;
            data[frame * 2 + offset / 8] |= 1 << (offset % 8);
        }
    }
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F",
        "frames":4,"bits_per_frame":16,"encoding":"frame_byte_lsb0",
        "cram_hex":data.iter().map(|v|format!("{v:02x}")).collect::<String>(),"bram_data":{},
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
fn nodes(device: &ConfiguredDevice) -> (NodeId, [NodeId; 2]) {
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let find = |name| {
        device
            .model()
            .nodes(plc)
            .find(|n| n.native_name == name)
            .unwrap()
            .id
    };
    (find("SLICEA.K0"), [find("SLICEA.FF0"), find("SLICEA.FF1")])
}
fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}
fn signal(builder: &mut Builder, name: &str) -> SignalId {
    builder.signal(wire(name)).unwrap()
}
fn driver(builder: &mut Builder, signal: SignalId, name: &str, value: Logic) -> DriverId {
    builder
        .external_driver(signal, name, Strength::Strong, value)
        .unwrap()
}
fn ff(node: NodeId, data: Input, clock: Input) -> FlipFlop {
    FlipFlop {
        preload: None,
        node,
        data,
        clock,
        enable: Input::Constant(One),
        edge: Edge::Rising,
        reset: None,
    }
}
fn ps(value: u64) -> Time {
    Time::from_ps(value)
}

#[test]
fn rational_time_is_exact_ordered_and_checked() {
    assert_eq!(Time::new(2, 6).unwrap(), Time::new(1, 3).unwrap());
    assert_eq!(Time::new(0, u64::MAX).unwrap(), Time::ZERO);
    assert!(Time::new(1, 0).is_err());
    let period = Time::new(1_000_000, 3).unwrap();
    let mut accumulated = Time::ZERO;
    for _ in 0..10_000 {
        accumulated = accumulated.checked_add(period).unwrap();
    }
    assert_eq!(accumulated, period.checked_mul(10_000).unwrap());
    assert_eq!(accumulated.ratio(), (10_000_000_000, 3));
    assert!(Time::new(u64::MAX, u64::MAX - 1).unwrap() > ps(1));
    assert!(ps(u64::MAX).checked_add(ps(1)).is_err());
    // Reduction precedes range checking, including multiplication by zero.
    assert_eq!(
        Time::new(u64::MAX, 2).unwrap().checked_mul(2).unwrap(),
        ps(u64::MAX)
    );
    assert_eq!(ps(u64::MAX).checked_mul(0).unwrap(), Time::ZERO);
}

#[test]
fn resolution_handles_contention_pulls_release_and_directed_buffers() {
    let f = Fixture::with_kernel();
    let mut b = Builder::new(configured(&f, 0)).unwrap();
    let input = signal(&mut b, "BUS");
    let output = signal(&mut b, "BUFFERED_BUS");
    let first = driver(&mut b, input, "first pad", Z);
    let second = driver(&mut b, input, "second pad", Z);
    b.external_driver(input, "pullup", Strength::Pull, One)
        .unwrap();
    let sink = driver(&mut b, output, "sink output", Z);
    b.buffer("directed fabric buffer", input.into(), output)
        .unwrap();
    b.watch(input).unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(input).unwrap(), One);
    k.schedule(ps(1), first, Zero).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(input).unwrap(), Zero);
    k.schedule(ps(2), second, One).unwrap();
    k.run_until(ps(2)).unwrap();
    assert_eq!(k.value(input).unwrap(), X);
    k.schedule_batch(ps(3), &[(first, Z), (second, Z)], None)
        .unwrap();
    k.run_until(ps(3)).unwrap();
    assert_eq!(k.value(input).unwrap(), One);
    k.schedule(ps(4), sink, Zero).unwrap();
    k.run_until(ps(4)).unwrap();
    assert_eq!(k.value(output).unwrap(), X);
    assert_eq!(k.value(input).unwrap(), One); // no backwards propagation
    k.schedule_batch(ps(5), &[(sink, Z), (first, X)], None)
        .unwrap();
    k.run_until(ps(5)).unwrap();
    assert_eq!(k.value(input).unwrap(), X); // strong unknown defeats pullup
    assert_eq!(k.state().wire_value(&wire("BUS")), Some(X));
    assert_eq!(
        k.drain_trace().iter().map(|t| t.value).collect::<Vec<_>>(),
        [One, Zero, X, One, X]
    );
}

fn simultaneous(reverse: bool, trace_capacity: usize) -> (Kernel, [SignalId; 2], [DriverId; 2]) {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    b.options(Options {
        trace_capacity,
        ..Options::default()
    })
    .unwrap();
    let q = if reverse {
        let q1 = signal(&mut b, "Q1_SLICE");
        [signal(&mut b, "Q0_SLICE"), q1]
    } else {
        [signal(&mut b, "Q0_SLICE"), signal(&mut b, "Q1_SLICE")]
    };
    let clocks = [signal(&mut b, "CLK0"), signal(&mut b, "CLK1")];
    let drivers = [
        driver(&mut b, clocks[0], "clock 0", Zero),
        driver(&mut b, clocks[1], "clock 1", Zero),
    ];
    for index in if reverse { [1, 0] } else { [0, 1] } {
        let data = if index == 0 {
            Input::Constant(One)
        } else {
            q[0].into()
        };
        assert_eq!(
            b.flip_flop(ff(regs[index], data, clocks[index].into()))
                .unwrap(),
            q[index]
        );
        b.watch(q[index]).unwrap();
    }
    (b.build().unwrap(), q, drivers)
}

#[test]
fn coincident_independent_clocks_capture_before_any_register_publishes() {
    let mut traces = Vec::new();
    for reverse in [false, true] {
        let (mut k, q, clk) = simultaneous(reverse, 65_536);
        let events = if reverse {
            [(clk[1], One), (clk[0], One)]
        } else {
            [(clk[0], One), (clk[1], One)]
        };
        k.schedule_batch(ps(1), &events, None).unwrap();
        k.run_until(ps(1)).unwrap();
        assert_eq!(
            [k.value(q[0]).unwrap(), k.value(q[1]).unwrap()],
            [One, Zero]
        );
        k.schedule_batch(ps(2), &[(clk[0], Zero), (clk[1], Zero)], None)
            .unwrap();
        // Two equal rational timestamps must land in one event batch.
        k.schedule(Time::new(9, 3).unwrap(), clk[0], One).unwrap();
        k.schedule(Time::new(6, 2).unwrap(), clk[1], One).unwrap();
        k.run_until(ps(3)).unwrap();
        assert_eq!([k.value(q[0]).unwrap(), k.value(q[1]).unwrap()], [One, One]);
        traces.push(k.drain_trace());
    }
    assert_eq!(traces[0], traces[1]);
}

#[test]
fn disabling_trace_changes_neither_execution_nor_virtual_time() {
    let mut results = Vec::new();
    for capacity in [0, 32] {
        let (mut k, q, clk) = simultaneous(false, capacity);
        for time in 1..=20 {
            let value = Logic::from_bool(time % 2 == 1);
            k.schedule_batch(ps(time), &[(clk[0], value), (clk[1], value)], None)
                .unwrap();
        }
        k.run_until(ps(20)).unwrap();
        results.push((
            k.value(q[0]).unwrap(),
            k.value(q[1]).unwrap(),
            k.now(),
            k.statistics().driver_changes,
        ));
        assert_eq!(k.drain_trace().is_empty(), capacity == 0);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn initial_clock_level_does_not_invent_an_edge_and_both_edges_execute() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut b = Builder::new(device).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let drive = driver(&mut b, clock, "clock", One);
    let q0 = b
        .flip_flop(ff(regs[0], Input::Constant(One), clock.into()))
        .unwrap();
    let mut falling = ff(regs[1], Input::Constant(One), clock.into());
    falling.edge = Edge::Falling;
    let q1 = b.flip_flop(falling).unwrap();
    let mut k = b.build().unwrap();
    assert_eq!([k.value(q0).unwrap(), k.value(q1).unwrap()], [X, X]);
    k.schedule(ps(1), drive, Zero).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!([k.value(q0).unwrap(), k.value(q1).unwrap()], [X, One]);
    k.schedule(ps(2), drive, One).unwrap();
    k.run_until(ps(2)).unwrap();
    assert_eq!([k.value(q0).unwrap(), k.value(q1).unwrap()], [One, One]);
}

#[test]
fn local_reset_has_priority_over_enable_and_async_reset_needs_no_clock() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, One).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let reset = signal(&mut b, "RESET");
    let enable = signal(&mut b, "ENABLE");
    let clk = driver(&mut b, clock, "clock", Zero);
    let rst = driver(&mut b, reset, "local reset", Zero);
    let ce = driver(&mut b, enable, "enable", Zero);
    let mut q = Vec::new();
    for (index, kind) in [ResetKind::Asynchronous, ResetKind::Synchronous]
        .into_iter()
        .enumerate()
    {
        let mut config = ff(regs[index], Input::Constant(One), clock.into());
        config.enable = enable.into();
        config.reset = Some(Reset {
            input: reset.into(),
            kind,
            value: Zero,
        });
        q.push(b.flip_flop(config).unwrap());
    }
    let mut k = b.build().unwrap();
    k.schedule(ps(1), rst, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(
        [k.value(q[0]).unwrap(), k.value(q[1]).unwrap()],
        [Zero, One]
    );
    k.schedule(ps(2), clk, One).unwrap();
    k.run_until(ps(2)).unwrap();
    assert_eq!(
        [k.value(q[0]).unwrap(), k.value(q[1]).unwrap()],
        [Zero, Zero]
    );
    k.schedule_batch(ps(3), &[(rst, Zero), (ce, One)], None)
        .unwrap();
    k.run_until(ps(3)).unwrap();
    assert_eq!(k.value(q[0]).unwrap(), Zero); // release while high is not an edge
    k.schedule(ps(4), clk, Zero).unwrap();
    k.schedule(ps(5), clk, One).unwrap();
    k.run_until(ps(5)).unwrap();
    assert_eq!([k.value(q[0]).unwrap(), k.value(q[1]).unwrap()], [One, One]);
    k.schedule(ps(6), rst, X).unwrap();
    k.run_until(ps(6)).unwrap();
    assert_eq!(k.value(q[0]).unwrap(), X); // uncertain async reset merges 0 and 1
    assert_eq!(k.value(q[1]).unwrap(), One); // sync reset waits for an edge
}

#[test]
fn uncertain_edges_merge_capture_and_hold_and_sampled_z_becomes_x() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let data = signal(&mut b, "DATA");
    let clk = driver(&mut b, clock, "clock", Zero);
    let d = driver(&mut b, data, "data", One);
    let q0 = b.flip_flop(ff(regs[0], data.into(), clock.into())).unwrap();
    let q1 = b
        .flip_flop(ff(regs[1], Input::Constant(Zero), clock.into()))
        .unwrap();
    let mut k = b.build().unwrap();
    k.schedule(ps(1), clk, X).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!([k.value(q0).unwrap(), k.value(q1).unwrap()], [X, Zero]);
    assert_eq!(k.statistics().possible_clock_edges, 2);
    k.schedule_batch(ps(2), &[(clk, Zero), (d, Z)], None)
        .unwrap();
    k.schedule(ps(3), clk, One).unwrap();
    k.run_until(ps(3)).unwrap();
    assert_eq!(k.value(q0).unwrap(), X);
    assert_ne!(k.state().register(regs[0]).unwrap(), Z);
}

#[test]
fn unknown_enable_preserves_only_results_shared_by_hold_and_capture() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let clk = driver(&mut b, clock, "clock", Zero);
    let mut outputs = Vec::new();
    for (index, data) in [One, Zero].into_iter().enumerate() {
        let mut config = ff(regs[index], Input::Constant(data), clock.into());
        config.enable = Input::Constant(X);
        outputs.push(b.flip_flop(config).unwrap());
    }
    let mut k = b.build().unwrap();
    k.schedule(ps(1), clk, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(
        [k.value(outputs[0]).unwrap(), k.value(outputs[1]).unwrap()],
        [X, Zero]
    );
}

#[test]
fn lut_and_directed_fabric_settle_before_coincident_clock_capture() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0x6666); // A XOR B, independent of C/D
    let fingerprint = device.image().decoded_sha256().to_string();
    let (lut, regs) = nodes(&device);
    let mut b = Builder::new(Arc::clone(&device)).unwrap();
    let a = signal(&mut b, "A0_SLICE");
    let a_drive = driver(&mut b, a, "A", Zero);
    let b_input = signal(&mut b, "B0_SLICE");
    driver(&mut b, b_input, "B", One);
    let f_out = b.lut4(lut).unwrap();
    let di = signal(&mut b, "DI0_SLICE");
    b.buffer("F to DI", f_out.into(), di).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let clk = driver(&mut b, clock, "clock", Zero);
    let q = b.flip_flop(ff(regs[0], di.into(), clock.into())).unwrap();
    for signal in [a, f_out, di, q] {
        b.watch(signal).unwrap();
    }
    let mut k = b.build().unwrap();
    assert_eq!(k.value(f_out).unwrap(), One);
    assert_eq!(k.value(q).unwrap(), X);
    k.schedule_batch(ps(1), &[(clk, One), (a_drive, One)], None)
        .unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(q).unwrap(), Zero); // sees settled new LUT, not old F=1
    k.schedule(ps(2), a_drive, Zero).unwrap();
    k.run_until(ps(2)).unwrap();
    assert_eq!(k.value(f_out).unwrap(), One);
    assert_eq!(k.value(q).unwrap(), Zero);
    k.schedule(ps(3), clk, Zero).unwrap();
    k.schedule(ps(4), clk, One).unwrap();
    k.run_until(ps(4)).unwrap();
    assert_eq!(k.value(q).unwrap(), One);
    assert_eq!(k.state().register(regs[0]).unwrap(), One);
    assert_eq!(device.image().decoded_sha256(), fingerprint);
    let trace = k.drain_trace();
    let wave = trace.iter().filter(|t| t.time == ps(1)).collect::<Vec<_>>();
    assert_eq!(
        wave.iter().map(|t| t.phase).collect::<Vec<_>>(),
        [
            Phase::Stimulus,
            Phase::Combinational,
            Phase::Combinational,
            Phase::Capture
        ]
    );
    assert!(wave.windows(2).all(|p| p[0].delta < p[1].delta));
    assert!(wave.iter().all(|t| !t.causes.is_empty()));
}

#[test]
fn a_generated_clock_is_captured_in_a_later_delta_at_the_same_time() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let clk = driver(&mut b, clock, "clock", Zero);
    let q0 = b
        .flip_flop(ff(regs[0], Input::Constant(One), clock.into()))
        .unwrap();
    let q1 = b.flip_flop(ff(regs[1], q0.into(), q0.into())).unwrap();
    b.watch(q0).unwrap();
    b.watch(q1).unwrap();
    let mut k = b.build().unwrap();
    k.drain_trace();
    k.schedule(ps(1), clk, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!([k.value(q0).unwrap(), k.value(q1).unwrap()], [One, One]);
    let trace = k.drain_trace();
    assert_eq!(trace.len(), 2);
    assert!(trace
        .iter()
        .all(|t| t.time == ps(1) && t.phase == Phase::Capture));
    assert!(trace[0].delta < trace[1].delta);
}

#[test]
fn timer_generation_cancels_stale_events_and_queue_rejects_closed_time() {
    let f = Fixture::with_kernel();
    let mut b = Builder::new(configured(&f, 0)).unwrap();
    let input = signal(&mut b, "CLOCK");
    let drive = driver(&mut b, input, "clock driver", Zero);
    let mut k = b.build().unwrap();
    let timer = k.new_timer("oscillator").unwrap();
    k.schedule_batch(Time::new(1, 3).unwrap(), &[(drive, One)], Some(timer))
        .unwrap();
    k.cancel_timer(timer).unwrap();
    k.schedule_batch(Time::new(2, 6).unwrap(), &[(drive, Zero)], Some(timer))
        .unwrap();
    k.run_until(Time::new(1, 3).unwrap()).unwrap();
    assert_eq!(k.value(input).unwrap(), Zero);
    assert_eq!(k.statistics().discarded_timer_events, 1);
    assert!(k.schedule(Time::new(1, 3).unwrap(), drive, One).is_err());
    assert!(k.schedule(Time::ZERO, drive, One).is_err());
    k.schedule(ps(1), drive, One).unwrap(); // untimed event survives timer cancellation
    k.cancel_timer(timer).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(input).unwrap(), One);
    assert!(k.run_until(Time::ZERO).is_err());
    assert!(k.fault().is_none()); // invalid API requests don't corrupt execution
}

#[test]
fn equal_events_for_one_driver_keep_all_causes_independent_of_insertion_order() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let mut traces = Vec::new();
    for reverse in [false, true] {
        let mut b = Builder::new(Arc::clone(&device)).unwrap();
        let input = signal(&mut b, "INPUT");
        let drive = driver(&mut b, input, "input driver", Zero);
        b.watch(input).unwrap();
        let mut k = b.build().unwrap();
        k.drain_trace();
        let timer = k.new_timer("timer").unwrap();
        for timed in if reverse {
            [true, false]
        } else {
            [false, true]
        } {
            k.schedule_batch(
                ps(1),
                &[(drive, One)],
                if timed { Some(timer) } else { None },
            )
            .unwrap();
        }
        k.run_until(ps(1)).unwrap();
        let trace = k.drain_trace();
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].causes.len(), 2);
        traces.push(trace);
    }
    assert_eq!(traces[0], traces[1]);
}

#[test]
fn invalid_handles_and_conflicting_driver_events_do_not_get_silent_priority() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let mut a = Builder::new(Arc::clone(&device)).unwrap();
    let sig_a = signal(&mut a, "BUS");
    let drive_a = driver(&mut a, sig_a, "driver", Zero);
    let mut b = Builder::new(device).unwrap();
    let sig_b = signal(&mut b, "BUS");
    assert!(b.buffer("foreign", sig_a.into(), sig_b).is_err());
    let mut ka = a.build().unwrap();
    let mut kb = b.build().unwrap();
    assert!(kb.value(sig_a).is_err());
    assert!(kb.schedule(ps(1), drive_a, One).is_err());
    let timer = ka.new_timer("timer").unwrap();
    assert!(kb.cancel_timer(timer).is_err());
    assert!(ka
        .schedule_batch(ps(1), &[(drive_a, Zero), (drive_a, One)], None)
        .is_err());
    ka.schedule(ps(2), drive_a, One).unwrap();
    ka.schedule(ps(2), drive_a, Zero).unwrap();
    assert!(ka
        .run_until(ps(2))
        .unwrap_err()
        .to_string()
        .contains("conflicting events"));
    assert_eq!(ka.value(sig_a).unwrap(), Zero); // rejected before publishing any part
    assert!(ka.fault().is_some());
    assert!(ka.run_until(ps(3)).is_err());
}

#[test]
fn simultaneous_driver_handover_does_not_create_a_false_clock_edge() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let (_, regs) = nodes(&device);
    let mut b = Builder::new(device).unwrap();
    let clock = signal(&mut b, "CLOCK");
    let first = driver(&mut b, clock, "first", One);
    let second = driver(&mut b, clock, "second", Z);
    b.watch(clock).unwrap();
    let q = b
        .flip_flop(ff(regs[0], Input::Constant(One), clock.into()))
        .unwrap();
    let mut k = b.build().unwrap();
    k.drain_trace();
    k.schedule(ps(1), first, Z).unwrap();
    k.schedule(ps(1), second, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(q).unwrap(), X);
    assert!(k.drain_trace().is_empty());
}

#[test]
fn feedback_limit_faults_the_kernel_and_trace_retention_is_bounded() {
    let f = Fixture::with_kernel();
    let mut b = Builder::new(configured(&f, 0)).unwrap();
    b.options(Options {
        max_delta_cycles: 16,
        trace_capacity: 4,
    })
    .unwrap();
    let enable = signal(&mut b, "LOOP_ENABLE");
    let drive = driver(&mut b, enable, "enable", Zero);
    let feedback = signal(&mut b, "FEEDBACK");
    b.mux(
        "gated inverter",
        enable.into(),
        Input::Constant(Zero),
        Input::Inverted(feedback),
        feedback,
    )
    .unwrap();
    b.watch(feedback).unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(feedback).unwrap(), Zero);
    k.schedule(ps(1), drive, One).unwrap();
    let error = k.run_until(ps(1)).unwrap_err().to_string();
    assert!(error.contains("did not settle"));
    assert!(error.contains("gated inverter -> R1C1:FEEDBACK="));
    assert!(k.statistics().trace_overwrites > 0);
    assert_eq!(k.drain_trace().len(), 4);
    assert!(k.fault().is_some());
    assert!(k.schedule(ps(2), drive, Zero).is_err());
}

#[test]
fn live_tables_merge_unknown_addresses_and_unknown_storage_without_inventing_bits() {
    let mut table = [Zero; 16];
    table[15] = X;
    assert_eq!(Logic::table(&table, &[Zero, X, Z, X]), Zero);
    assert_eq!(Logic::table(&table, &[One, One, One, X]), X);
    table.fill(One);
    assert_eq!(Logic::table(&table, &[Z, X, Z, X]), One);
    table[0] = Z;
    assert_eq!(Logic::table(&table, &[Zero; 4]), X); // storage never drives high impedance
}

#[test]
#[ignore = "requires local Trellis, ecppack and trellis_image_reader"]
fn native_image_lut_and_ff_execute_in_the_full_device_with_an_explicit_plan() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let database = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&database, &evidence).unwrap());
    let temp = Fixture::new();
    let text = temp.root.join("kernel.config");
    write(&text,".device LFE5U-85F\n.tile R11C10:PLC2\nenum: SLICEA.MODE LOGIC\narc: F0 F0_SLICE\nword: SLICEA.K0.INIT 1010000101011100\n");
    let bit = temp.root.join("kernel.bit");
    let packed = tools::command("ecppack")
        .arg(&text)
        .arg(&bit)
        .arg("--db")
        .arg(&database)
        .arg("--compress")
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    let mut observations = Vec::new();
    let mut reports = Vec::new();
    for path in [text, bit] {
        let image = Arc::new(ConfigurationImage::load(&path, &database, None).unwrap());
        let image_digest = image.decoded_sha256().to_string();
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let plc = model.resource_id("LFE5U-85F/R11C10/PLC2").unwrap();
        let find = |name| model.nodes(plc).find(|n| n.native_name == name).unwrap().id;
        let tile = model.tile_id("R11C10:PLC2").unwrap();
        let selection = device.raw_mux(tile, "F0").unwrap();
        assert_eq!(selection.candidate_sources, ["F0_SLICE"]);
        let selected = model.connection(selection.candidates[0]).unwrap();
        let mut route_evidence = vec![json!({"key":selected.stable_key("LFE5U-85F"),
            "source_line":selected.definition.source_line,"qualification":"explicit LOGIC fixture and unique raw mux selection"})];
        let mut b = Builder::new(device).unwrap();
        let mut inputs = Vec::new();
        for port in ["A", "B", "C", "D"] {
            let p = model
                .node_ports(find("SLICEA.K0"))
                .into_iter()
                .find(|p| p.name == port)
                .unwrap();
            let signal = b.signal(p.wire).unwrap();
            inputs.push(driver(&mut b, signal, port, Zero));
        }
        let f = b.lut4(find("SLICEA.K0")).unwrap();
        // Explicit LOGIC mode and the verified F0_SLICE selection precede the
        // two fixed F0 -> DI0 -> DI0_SLICE routes. No general mode inference.
        let ecp5_emu::model::topology::Endpoint::Wire(dest) = selected.sink else {
            panic!("selected mux sink")
        };
        let dest = b.signal(dest).unwrap();
        b.buffer("qualified LOGIC F0 mux", f.into(), dest).unwrap();
        for sink in ["DI0", "DI0_SLICE"] {
            let route = model
                .connections(tile)
                .map(Result::unwrap)
                .find(|c| {
                    c.definition.sink == sink
                        && c.definition.kind == ecp5_emu::database::ArcKind::Fixed
                })
                .unwrap();
            route_evidence.push(json!({"key":route.stable_key("LFE5U-85F"),
                "source_line":route.definition.source_line,"qualification":"explicit fixture fixed route"}));
            let ecp5_emu::model::topology::Endpoint::Wire(source) = route.source else {
                panic!("route source")
            };
            let ecp5_emu::model::topology::Endpoint::Wire(dest) = route.sink else {
                panic!("route sink")
            };
            let source = b.signal(source).unwrap();
            let dest = b.signal(dest).unwrap();
            b.buffer(&format!("native fixed:{sink}"), source.into(), dest)
                .unwrap();
        }
        let ff_ports = model.node_ports(find("SLICEA.FF0"));
        let mut port = |name| {
            b.signal(
                ff_ports
                    .iter()
                    .find(|p| p.name == name)
                    .unwrap()
                    .wire
                    .clone(),
            )
            .unwrap()
        };
        let di = port("DI");
        let clock = port("CLK");
        let clk = driver(&mut b, clock, "external test clock", Zero);
        let q = b
            .flip_flop(ff(find("SLICEA.FF0"), di.into(), clock.into()))
            .unwrap();
        for observed in [f, di, clock, q] {
            b.watch(observed).unwrap();
        }
        let mut k = b.build().unwrap();
        assert_eq!(k.device().model().summary().tiles, 14231);
        assert_eq!(k.state().summary().lut_arrays, 83640);
        let mut samples = Vec::new();
        for address in 0..16 {
            let mut updates = inputs
                .iter()
                .enumerate()
                .map(|(bit, &d)| (d, Logic::from_bool(address & (1 << bit) != 0)))
                .collect::<Vec<_>>();
            updates.push((clk, Zero));
            k.schedule_batch(ps(2 * address + 1), &updates, None)
                .unwrap();
            k.schedule(ps(2 * address + 2), clk, One).unwrap();
            k.run_until(ps(2 * address + 2)).unwrap();
            let expected = Logic::from_bool(0xa15c & (1 << address) != 0);
            assert_eq!(k.value(f).unwrap(), expected);
            assert_eq!(k.value(q).unwrap(), expected);
            samples.push(expected);
        }
        assert_eq!(k.device().image().decoded_sha256(), image_digest);
        let trace = k.drain_trace();
        reports.push(json!({"image":k.device().image().summary(),"architecture_manifest_sha256":model.manifest_sha256(),
            "configuration_sha256":k.device().fingerprint(),"route_evidence":route_evidence,
            "manual_FF_profile":{"data":"DI","clock":"native CLK port, external stimulus","CE":"constant 1",
                "edge":"rising","local_reset":"none","startup":"X; no PUR/GSR applied"},
            "lut_mask":"a15c","address_count":16,"samples":samples,"state":k.state().summary(),
            "statistics":k.statistics(),"transitions":trace}));
        observations.push(samples);
    }
    assert_eq!(observations[0], observations[1]);
    let output = root.join("target/event-kernel/native_execution.json");
    write(&output,serde_json::to_vec_pretty(&json!({"schema_version":1,
        "scope":"native image LUT/FF execution with explicit partial plan; not whole-chip behavioral validation",
        "hardware_results":0,"variants":reports})).unwrap());
}

#[test]
fn pll_outputs_follow_dividers_and_phase_and_lock_is_high_from_the_start() {
    // ecppll 25 -> 100 MHz: CLKI_DIV 1, CLKFB_DIV 4, feedback CLKOP_DIV 6, so
    // VCO = reference / 24. CLKOS divides by 12, 3 VCO periods (24 eighths) late.
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut b = Builder::new(device).unwrap();
    let reference = signal(&mut b, "REF");
    let clock = driver(&mut b, reference, "reference oscillator", Zero);
    let (op, os, lock) = (
        signal(&mut b, "OP"),
        signal(&mut b, "OS"),
        signal(&mut b, "LOCK"),
    );
    b.pll(PllConfig {
        resource: plc,
        reference: reference.into(),
        vco_per_reference: (1, 24),
        outputs: vec![
            PllOutput {
                port: "CLKOP",
                signal: op,
                divider: 6,
                phase_eighths: 0,
            },
            PllOutput {
                port: "CLKOS",
                signal: os,
                divider: 12,
                phase_eighths: 24,
            },
        ],
        lock: vec![("LOCK", lock)],
        controls: vec![],
        reference_divider: 1,
        feedback_output: "CLKOP",
    })
    .unwrap();
    b.watch(op).unwrap();
    b.watch(os).unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(lock).unwrap(), One); // HC05: locked before user logic runs
    assert_eq!(k.value(op).unwrap(), Zero);
    k.drain_trace(); // initial resolution of the output drivers
    let ns = |n: u64| ps(n * 1000);
    // 25 MHz reference: rising edges at 40, 80, 120 ns.
    for (t, v) in [
        (20, Zero),
        (40, One),
        (60, Zero),
        (80, One),
        (100, Zero),
        (120, One),
    ] {
        k.schedule(ns(t), clock, v).unwrap();
    }
    k.run_until(ns(80)).unwrap();
    assert!(
        k.drain_trace().is_empty(),
        "outputs stay low until the period is known"
    );
    k.run_until(ns(120)).unwrap();
    let edges = |trace: &[ecp5_emu::kernel::Transition], name: &str| -> Vec<(Time, Logic)> {
        trace
            .iter()
            .filter(|t| t.wire == wire(name))
            .map(|t| (t.time, t.value))
            .collect()
    };
    let trace = k.drain_trace();
    // CLKOP: 10 ns period aligned to the reference (the edge at 80 ns itself is
    // not generated); CLKOS: 20 ns period, 5 ns late.
    assert_eq!(
        edges(&trace, "OP"),
        [
            (ns(90), One),
            (ns(95), Zero),
            (ns(100), One),
            (ns(105), Zero),
            (ns(110), One),
            (ns(115), Zero),
            (ns(120), One)
        ]
    );
    assert_eq!(
        edges(&trace, "OS"),
        [
            (ns(85), One),
            (ns(95), Zero),
            (ns(105), One),
            (ns(115), Zero)
        ]
    );
    // A reference edge off the learnt period faults instead of inventing a relock.
    k.schedule(ns(140), clock, Zero).unwrap();
    k.schedule(ns(161), clock, One).unwrap();
    let error = k.run_until(ns(161)).unwrap_err();
    assert!(format!("{error:#}").contains("reference period changed"));
}

#[test]
fn pll_rst_holds_outputs_and_lock_and_restarts_on_its_grid() {
    // HC07/HC07b short LOCK reports (98313, 98311, 491546) include one
    // observer cycle in the emulator. Check the underlying edge times and
    // feedback-dependent short restart, then the common 2.2 us long restart.
    for (feedback, reference_divider, short_delay_ns, lock_periods) in [
        ("CLKOP", 1, 40_u64, 98_312),
        ("CLKOS3", 1, 40, 98_310),
        ("CLKOP", 5, 80, 491_545),
        ("CLKOS3", 5, 180, 491_545),
    ] {
        let f = Fixture::with_kernel();
        let device = configured(&f, 0);
        let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let mut b = Builder::new(device).unwrap();
        let reference = signal(&mut b, "REF");
        let clock = driver(&mut b, reference, "reference oscillator", Zero);
        let rst = signal(&mut b, "RST");
        let reset = driver(&mut b, rst, "reset", Zero);
        let (op, lock) = (signal(&mut b, "OP"), signal(&mut b, "LOCK"));
        // 25 MHz in, 50 MHz out: VCO = reference / 24, CLKOP divides by 12.
        b.pll(PllConfig {
            resource: plc,
            reference: reference.into(),
            vco_per_reference: (1, 24),
            outputs: vec![PllOutput {
                port: "CLKOP",
                signal: op,
                divider: 12,
                phase_eighths: 0,
            }],
            lock: vec![("LOCK", lock)],
            controls: vec![rst.into()],
            reference_divider,
            feedback_output: feedback,
        })
        .unwrap();
        b.watch(op).unwrap();
        b.watch(lock).unwrap();
        let mut k = b.build().unwrap();
        k.drain_trace();
        let ns = |n: u64| ps(n * 1000);
        // Reference rising edges at 40, 80, ... ns; the grid starts at 80 ns.
        let mut next = 20;
        let mut run = |k: &mut Kernel, until: u64| -> Vec<(Time, &'static str, Logic)> {
            while next <= until {
                k.schedule(ns(next), clock, if next % 40 == 0 { One } else { Zero })
                    .unwrap();
                next += 20;
            }
            k.run_until(ns(until)).unwrap();
            k.drain_trace()
                .into_iter()
                .filter_map(|t| {
                    let name = if t.wire == wire("OP") {
                        "OP"
                    } else if t.wire == wire("LOCK") {
                        "LOCK"
                    } else {
                        return None;
                    };
                    Some((t.time, name, t.value))
                })
                .collect()
        };
        let first = |trace: &[(Time, &str, Logic)], name: &str, value: Logic| {
            trace
                .iter()
                .find(|e| e.1 == name && e.2 == value)
                .map(|e| e.0)
        };
        run(&mut k, 1000);
        // A short hold, 120 ns.
        k.schedule(ns(1010), reset, One).unwrap();
        k.schedule(ns(1130), reset, Zero).unwrap();
        let rise_short = 80 + (1130 + short_delay_ns - 80).div_ceil(20) * 20;
        let trace = run(&mut k, rise_short + 100);
        let after = ns(1010).checked_add(ps(1)).unwrap();
        assert_eq!(first(&trace, "LOCK", Zero), Some(after));
        assert!(trace
            .iter()
            .all(|e| e.1 != "OP" || e.0 <= after || e.0 >= ns(rise_short)));
        assert_eq!(first(&trace, "OP", One), Some(ns(rise_short)));
        // LOCK: the fixed counter and feedback latency, no acquisition.
        let lock_short = 1130 + lock_periods * 40;
        let trace = run(&mut k, lock_short + 100);
        assert_eq!(first(&trace, "LOCK", One), Some(ns(lock_short)));
        // A long hold, 50 us: restart 2.2 us after the fall, at the next grid edge.
        k.schedule(ns(lock_short + 1010), reset, One).unwrap();
        let fall = lock_short + 51_010;
        k.schedule(ns(fall), reset, Zero).unwrap();
        let trace = run(&mut k, fall + 3000);
        assert_eq!(
            first(&trace, "LOCK", Zero),
            Some(ns(lock_short + 1010).checked_add(ps(1)).unwrap())
        );
        let restart = fall + 2200;
        let rise = 80 + (restart - 80).div_ceil(20) * 20;
        assert!(trace
            .iter()
            .all(|e| e.1 != "OP" || e.0 <= ns(lock_short + 1011) || e.0 >= ns(rise)));
        let held = ns(lock_short + 1011);
        assert_eq!(
            trace
                .iter()
                .find(|e| e.1 == "OP" && e.2 == One && e.0 > held)
                .map(|e| e.0),
            Some(ns(rise))
        );
        // LOCK: plus the acquisition time after a hold of 50 us (424 us).
        let lock_long = fall + 424_000 + lock_periods * 40;
        let trace = run(&mut k, lock_long + 100);
        assert_eq!(first(&trace, "LOCK", One), Some(ns(lock_long)));
        // An unknown RST faults.
        k.schedule(ns(lock_long + 1000), reset, X).unwrap();
        let error = k.run_until(ns(lock_long + 1000)).unwrap_err();
        assert!(format!("{error:#}").contains("RST/STDBY is not binary"));
    }
}

#[test]
fn iologic_paths_follow_the_board_model() {
    // HI02 on the ULX3S: ODDRX1F sends D0 in the high and D1 in the low phase
    // two rising edges after capture; IDDRX1F publishes (pad at rising edge k,
    // pad at falling edge k) at rising edge k+1. DDR stages start at 0, an IO
    // register at REGSET; an asynchronous reset holds them.
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut b = Builder::new(device).unwrap();
    let names = ["SCLK", "D0", "D1", "PAD", "RST", "OUT", "Q0", "Q1", "REG"];
    let [sclk, d0, d1, pad, rst, out, q0, q1, reg] = names.map(|n| signal(&mut b, n));
    let drive = names[..5]
        .iter()
        .zip([sclk, d0, d1, pad, rst])
        .map(|(n, s)| driver(&mut b, s, n, Zero))
        .collect::<Vec<_>>();
    let [clock, d0_in, d1_in, pad_in, reset] = drive[..] else {
        unreachable!()
    };
    let io = |name, path, reset: Option<Input>| IoLogicConfig {
        resource: plc,
        name,
        path,
        clock: sclk.into(),
        reset,
    };
    b.iologic(io(
        "ODDR",
        IoPath::OutputDdr {
            data: [d0.into(), d1.into()],
            output: out,
            global_reset: None,
        },
        Some(rst.into()),
    ))
    .unwrap();
    b.iologic(io(
        "IDDR",
        IoPath::InputDdr {
            data: pad.into(),
            outputs: [q0, q1],
            global_reset: None,
        },
        Some(rst.into()),
    ))
    .unwrap();
    b.iologic(io(
        "OREG",
        IoPath::Register {
            data: d0.into(),
            output: reg,
            value: One,
            enable: Input::Constant(One),
            sync_reset: None,
            global_reset: None,
        },
        Some(rst.into()),
    ))
    .unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(
        [out, q0, q1, reg].map(|s| k.value(s).unwrap()),
        [Zero, Zero, Zero, One]
    );
    let ns = |n: u64| ps(n * 1000);
    let bit = |v: u8| if v == 1 { One } else { Zero };
    // Rising edge k at 10k+5 ns, falling at 10k+10 ns. D0/D1 change at 10k+1,
    // the pad at 10k+3 (seen by rising edge k) and 10k+8 (falling edge k).
    let (d0s, d1s) = ([1, 0, 0, 1, 1, 0], [0, 1, 0, 1, 0, 1]);
    let (rise_pad, fall_pad) = ([1, 1, 0, 0, 1, 0], [0, 1, 1, 0, 0, 1]);
    for edge in 1..=6_u64 {
        let (i, t) = (edge as usize - 1, 10 * edge);
        k.schedule(ns(t + 1), d0_in, bit(d0s[i])).unwrap();
        k.schedule(ns(t + 1), d1_in, bit(d1s[i])).unwrap();
        k.schedule(ns(t + 3), pad_in, bit(rise_pad[i])).unwrap();
        k.schedule(ns(t + 5), clock, One).unwrap();
        k.schedule(ns(t + 8), pad_in, bit(fall_pad[i])).unwrap();
        k.schedule(ns(t + 10), clock, Zero).unwrap();
    }
    let mut seen = Vec::new();
    for t in [17, 22, 27, 32, 37, 42, 47, 52, 57, 62, 67, 72] {
        k.run_until(ns(t)).unwrap();
        seen.push([out, q0, q1, reg].map(|s| k.value(s).unwrap()));
    }
    let expected = [
        // (high after edge 1) (low) (high after edge 2) (low): pipeline still 0
        [Zero, Zero, Zero, One],
        [Zero, Zero, Zero, One],
        [Zero, One, Zero, Zero],
        [Zero, One, Zero, Zero],
        // edge 3: D0 of edge 1 high, D1 of edge 1 low; Q = (rise 2, fall 2)
        [One, One, One, Zero],
        [Zero, One, One, Zero],
        // edge 4: D0/D1 of edge 2
        [Zero, Zero, One, One],
        [One, Zero, One, One],
        // edge 5: D0/D1 of edge 3
        [Zero, Zero, Zero, One],
        [Zero, Zero, Zero, One],
        // edge 6: D0/D1 of edge 4
        [One, One, Zero, Zero],
        [One, One, Zero, Zero],
    ];
    assert_eq!(seen, expected);
    // An asynchronous reset in the middle of a phase acts at once.
    k.schedule(ns(73), reset, One).unwrap();
    k.run_until(ns(73)).unwrap();
    assert_eq!(
        [out, q0, q1, reg].map(|s| k.value(s).unwrap()),
        [Zero, Zero, Zero, One]
    );
}

#[test]
fn oscillator_runs_free_with_exact_edges_and_one_queued_event() {
    let f = Fixture::with_kernel();
    let device = configured(&f, 0);
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut b = Builder::new(device).unwrap();
    let osc = signal(&mut b, "OSC");
    // A half period of 10/3 ps: edges fall on exact rational times.
    b.oscillator(OscillatorConfig {
        resource: plc,
        output: osc,
        half_period: Time::new(10, 3).unwrap(),
    })
    .unwrap();
    b.watch(osc).unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(osc).unwrap(), Zero);
    k.drain_trace();
    k.run_until(ps(20)).unwrap();
    let edges: Vec<(Time, Logic)> = k
        .drain_trace()
        .iter()
        .filter(|t| t.wire == wire("OSC"))
        .map(|t| (t.time, t.value))
        .collect();
    let at = |n: u64| Time::new(10 * n, 3).unwrap();
    assert_eq!(
        edges,
        [
            (at(1), One),
            (at(2), Zero),
            (at(3), One),
            (at(4), Zero),
            (at(5), One),
            (at(6), Zero)
        ]
    );
    // 300 000 half periods end exactly at 1 µs, on a falling edge.
    k.run_until(ps(1_000_000)).unwrap();
    assert_eq!(k.value(osc).unwrap(), Zero);
    k.run_until(Time::new(3_000_001 * 10, 30).unwrap()).unwrap();
    assert_eq!(k.value(osc).unwrap(), Zero);
    k.run_until(Time::new(3_000_010, 3).unwrap()).unwrap();
    assert_eq!(k.value(osc).unwrap(), One);
}

#[test]
fn hs02_preload_sync_level_async_release_race_and_startup() {
    let f = Fixture::with_kernel();
    let d = configured(&f, 0);
    let (_, nodes) = nodes(&d);
    let mut b = Builder::new(d).unwrap();
    let mut add = |name, value| {
        let s = signal(&mut b, name);
        let dr = driver(&mut b, s, name, value);
        (Input::from(s), dr)
    };
    let (clock, ck) = add("CLK", Zero);
    let (data, di) = add("D", One);
    let (m, md) = add("M", One);
    let (enable, ce) = add("CE", Zero);
    let (lsr, ld) = add("LSR", Zero);
    let q: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(i, &node)| {
            let q = b
                .flip_flop(FlipFlop {
                    node,
                    data,
                    preload: Some(m),
                    clock,
                    enable,
                    edge: Edge::Rising,
                    reset: Some(Reset {
                        input: lsr,
                        kind: if i == 0 {
                            ResetKind::Synchronous
                        } else {
                            ResetKind::Asynchronous
                        },
                        value: Zero,
                    }),
                })
                .unwrap();
            b.initial_preload_register(node).unwrap();
            q
        })
        .collect();
    let mut k = b.build().unwrap();
    let values = |k: &Kernel| q.iter().map(|&q| k.value(q).unwrap()).collect::<Vec<_>>();
    assert_eq!(values(&k), [One, One]);
    for (t, events, want) in [
        (1, vec![(ld, One), (md, Zero)], vec![One, Zero]),
        (2, vec![(ck, One)], vec![Zero, Zero]), // LSR beats CE=0.
        (3, vec![(ck, Zero), (md, One)], vec![Zero, One]), // async level follows M.
        (4, vec![(ld, Zero), (md, Zero)], vec![Zero, X]), // release and M change together.
        (5, vec![(ck, One)], vec![Zero, X]),    // CE=0 holds X after release.
        (6, vec![(ck, Zero), (ce, One), (di, One)], vec![Zero, X]),
        (7, vec![(ck, One)], vec![One, One]),
        (8, vec![(ck, Zero), (ld, One), (md, Z)], vec![One, X]),
        (9, vec![(ck, One)], vec![X, X]),
    ] {
        k.schedule_batch(ps(t), &events, None).unwrap();
        k.run_until(ps(t)).unwrap();
        assert_eq!(values(&k), want, "time {t}");
    }
}

fn dcs_fixture(mode: DcsMode, initial: [Logic; 5]) -> (Fixture, Kernel, [DriverId; 5], SignalId) {
    let fixture = Fixture::with_kernel();
    let device = configured(&fixture, 0);
    let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let (_, regs) = nodes(&device);
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut builder = Builder::with_state(device, state).unwrap();
    let inputs =
        ["DCS_C0", "DCS_C1", "DCS_SEL0", "DCS_SEL1", "DCS_MODE"].map(|n| signal(&mut builder, n));
    let drives = std::array::from_fn(|n| {
        driver(
            &mut builder,
            inputs[n],
            &format!("dcs-input-{n}"),
            initial[n],
        )
    });
    let output = signal(&mut builder, "DCS_OUT");
    builder.watch(output).unwrap();
    builder
        .dcs(DcsConfig {
            resource,
            mode,
            clocks: [inputs[0].into(), inputs[1].into()],
            select: [inputs[2].into(), inputs[3].into()],
            mode_select: inputs[4].into(),
            output,
        })
        .unwrap();
    // Both polarities must reach real downstream captures, through routing.
    let routed = signal(&mut builder, "DCS_ROUTED");
    builder.buffer("dcs-route", output.into(), routed).unwrap();
    for (n, node) in regs.into_iter().enumerate() {
        let q = signal(&mut builder, &format!("Q{n}_SLICE"));
        let mut capture = ff(node, Input::Inverted(q), routed.into());
        capture.edge = if n == 0 { Edge::Rising } else { Edge::Falling };
        assert_eq!(builder.flip_flop(capture).unwrap(), q);
        builder.watch(q).unwrap();
    }
    (fixture, builder.build().unwrap(), drives, output)
}

#[test]
fn dcs_all_static_tables_and_bypass_values() {
    use DcsMode::*;
    // An independent copy of the documented table: 2/3 denote clock sources.
    for (mode, table) in [
        (Pos, [1, 2, 3, 1]),
        (Neg, [0, 2, 3, 0]),
        (Clk0Low, [0, 2, 0, 0]),
        (Clk0High, [1, 2, 1, 1]),
        (Clk1Low, [0, 0, 3, 0]),
        (Clk1High, [1, 1, 3, 1]),
        (Clk0, [2; 4]),
        (Clk1, [3; 4]),
        (Low, [0; 4]),
        (High, [1; 4]),
    ] {
        for bypass in [false, true] {
            for (sel, &entry) in table.iter().enumerate() {
                let (_fixture, mut kernel, drives, output) = dcs_fixture(
                    mode,
                    [
                        Zero,
                        Zero,
                        Logic::from_bool(sel & 1 != 0),
                        Logic::from_bool(sel & 2 != 0),
                        Logic::from_bool(bypass),
                    ],
                );
                for (step, (c0, c1)) in [
                    (false, false),
                    (true, false),
                    (true, true),
                    (false, true),
                    (false, false),
                ]
                .into_iter()
                .enumerate()
                {
                    let target = if bypass { [0, 2, 3, 0][sel] } else { entry };
                    let expected = match target {
                        0 => Zero,
                        1 => One,
                        2 => Logic::from_bool(c0),
                        _ => Logic::from_bool(c1),
                    };
                    kernel
                        .schedule_batch(
                            ps(step as u64 + 1),
                            &[
                                (drives[0], Logic::from_bool(c0)),
                                (drives[1], Logic::from_bool(c1)),
                            ],
                            None,
                        )
                        .unwrap();
                    kernel.run_until(ps(step as u64 + 1)).unwrap();
                    assert_eq!(
                        kernel.value(output).unwrap(),
                        expected,
                        "{mode:?} SEL={sel} MODESEL={bypass}"
                    );
                }
            }
        }
    }
    assert_eq!("NONE".parse::<DcsMode>().unwrap(), Neg);
    assert!("invented".parse::<DcsMode>().is_err());
}

#[test]
fn dcs_protected_switches_have_exact_edges_at_many_phases_in_both_directions() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        for old in [0, 1] {
            let new = 1 - old;
            for phase in [0, 1, 3, 7, 11, 17] {
                for change in [69, 75, 83, 91] {
                    let sel = old + 1;
                    let (_fixture, mut kernel, drive, output) = dcs_fixture(
                        mode,
                        [
                            Zero,
                            Zero,
                            Logic::from_bool(sel & 1 != 0),
                            Logic::from_bool(sel & 2 != 0),
                            Zero,
                        ],
                    );
                    let source: [Vec<(u64, Logic)>; 2] = std::array::from_fn(|n| {
                        let half = [10, 14][n];
                        let start = if n == 0 { 2 } else { phase + 2 };
                        (0..400)
                            .filter_map(|i| {
                                let t = start + i * half;
                                (t <= 350).then_some((t, Logic::from_bool(i % 2 == 0)))
                            })
                            .collect()
                    });
                    for (n, edges) in source.iter().enumerate() {
                        for &(t, value) in edges {
                            kernel.schedule(ps(t), drive[n], value).unwrap();
                        }
                    }
                    kernel
                        .schedule_batch(
                            ps(change),
                            &[
                                (drive[2], Logic::from_bool((new + 1) & 1 != 0)),
                                (drive[3], Logic::from_bool((new + 1) & 2 != 0)),
                            ],
                            None,
                        )
                        .unwrap();
                    let active = if mode == DcsMode::Pos { One } else { Zero };
                    let stop = source[old]
                        .iter()
                        .find(|&&(t, v)| t >= change && v == active)
                        .unwrap()
                        .0;
                    let start = source[new]
                        .iter()
                        .filter(|&&(t, v)| t > stop && v == active)
                        .nth(1)
                        .unwrap()
                        .0;
                    let mut expected: Vec<_> = source[old]
                        .iter()
                        .copied()
                        .filter(|&(t, _)| t <= stop)
                        .collect();
                    expected.extend(source[new].iter().copied().filter(|&(t, _)| t > start));
                    kernel.drain_trace();
                    kernel.run_until(ps(350)).unwrap();
                    let actual: Vec<_> = kernel
                        .drain_trace()
                        .into_iter()
                        .filter(|t| t.wire == wire("DCS_OUT"))
                        .map(|t| (t.time, t.value))
                        .collect();
                    let expected: Vec<_> = expected.into_iter().map(|(t, v)| (ps(t), v)).collect();
                    assert_eq!(
                        actual, expected,
                        "{mode:?} {old}->{new} phase={phase} change={change}"
                    );
                    assert_eq!(kernel.value(output).unwrap(), expected.last().unwrap().1);
                }
            }
        }
    }
}

#[test]
fn dcs_bypass_entry_truncates_immediately_and_cancels_a_pending_switch() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        let (_f, mut kernel, d, q) = dcs_fixture(mode, [One, Zero, One, Zero, Zero]);
        kernel
            .schedule_batch(ps(1), &[(d[2], Zero), (d[3], One)], None)
            .unwrap();
        kernel.run_until(ps(1)).unwrap();
        assert_eq!(kernel.value(q).unwrap(), One); // Still following old until its active edge.
        kernel.schedule(ps(2), d[4], One).unwrap();
        kernel.run_until(ps(2)).unwrap();
        assert_eq!(kernel.value(q).unwrap(), Zero); // Bypass cuts the high phase.
        kernel.schedule(ps(3), d[1], One).unwrap();
        kernel.run_until(ps(3)).unwrap();
        assert_eq!(kernel.value(q).unwrap(), One);
        kernel.schedule(ps(4), d[2], One).unwrap(); // SEL=11 is low in bypass.
        kernel.run_until(ps(4)).unwrap();
        assert_eq!(kernel.value(q).unwrap(), Zero);
    }
}

#[test]
fn dcs_stopped_new_source_holds_without_timeout_then_resumes_after_two_edges() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        let active = if mode == DcsMode::Pos { One } else { Zero };
        let (_f, mut kernel, d, q) =
            dcs_fixture(mode, [active.invert(), active.invert(), One, Zero, Zero]);
        kernel
            .schedule_batch(ps(1), &[(d[2], Zero), (d[3], One)], None)
            .unwrap();
        kernel.schedule(ps(2), d[0], active).unwrap();
        kernel.run_until(ps(1_000_000)).unwrap();
        assert_eq!(kernel.value(q).unwrap(), active);
        for (t, v) in [
            (1_000_001, active),
            (1_000_002, active.invert()),
            (1_000_003, active),
            (1_000_004, active.invert()),
        ] {
            kernel.schedule(ps(t), d[1], v).unwrap();
            kernel.run_until(ps(t)).unwrap();
            assert_eq!(
                kernel.value(q).unwrap(),
                if t < 1_000_004 {
                    active
                } else {
                    active.invert()
                }
            );
        }
    }
}

#[test]
fn dcs_coincident_old_and_new_edges_do_not_consume_the_new_stage() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        let active = if mode == DcsMode::Pos { One } else { Zero };
        let (_f, mut kernel, d, q) =
            dcs_fixture(mode, [active.invert(), active.invert(), One, Zero, Zero]);
        kernel
            .schedule_batch(ps(1), &[(d[2], Zero), (d[3], One)], None)
            .unwrap();
        kernel
            .schedule_batch(ps(2), &[(d[0], active), (d[1], active)], None)
            .unwrap();
        for (t, v) in [
            (3, active.invert()),
            (4, active),
            (5, active.invert()),
            (6, active),
            (7, active.invert()),
        ] {
            kernel.schedule(ps(t), d[1], v).unwrap();
            kernel.run_until(ps(t)).unwrap();
            assert_eq!(
                kernel.value(q).unwrap(),
                if t < 7 { active } else { active.invert() }
            );
        }
    }
}

#[test]
fn dcs_unknown_controls_are_conservative_and_fast_mode_refuses_unknown_state() {
    let (_f, mut kernel, d, q) = dcs_fixture(DcsMode::Pos, [Zero, Zero, One, Zero, Zero]);
    assert!(
        kernel.fast_program().is_ok(),
        "known DCSC runs in fast mode"
    );
    kernel.schedule(ps(1), d[2], X).unwrap();
    kernel.run_until(ps(1)).unwrap();
    assert_eq!(kernel.value(q).unwrap(), X);
    let error = kernel
        .fast_program()
        .err()
        .expect("unknown DCSOUT cannot be two-valued");
    assert!(format!("{error:#}").contains("DCSC"), "{error:#}");
    kernel
        .schedule_batch(ps(2), &[(d[2], One), (d[4], One)], None)
        .unwrap();
    kernel.run_until(ps(2)).unwrap();
    assert_eq!(kernel.value(q).unwrap(), Zero);
}

/// Replay timestamp batches, including empty ones and separate batches at the
/// same timestamp. The output and both downstream edge captures must agree.
fn dcs_fast_twin(mode: DcsMode, initial: [Logic; 5], batches: &[(u64, Vec<(usize, Logic)>)]) {
    use ecp5_emu::fast::FastSim;
    let (_f, mut kernel, d, q) = dcs_fixture(mode, initial);
    let probes = [
        q,
        kernel.signal(&wire("Q0_SLICE")).unwrap(),
        kernel.signal(&wire("Q1_SLICE")).unwrap(),
    ];
    let mut fast = FastSim::new(&kernel).unwrap();
    for &(time, ref changes) in batches {
        let events: Vec<_> = changes.iter().map(|&(i, v)| (d[i], v)).collect();
        kernel.schedule_batch(ps(time), &events, None).unwrap();
        kernel.run_until(ps(time)).unwrap();
        let inputs: Vec<_> = events.iter().map(|&(d, v)| (d, v.bit().unwrap())).collect();
        fast.set_at(ps(time), &inputs).unwrap();
        for probe in probes {
            assert_eq!(
                fast.value(probe),
                Some(kernel.value(probe).unwrap()),
                "{mode:?} at {time}, {changes:?}, {probe:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_all_tables_bypass_and_static_selection_transitions() {
    for mode in [
        DcsMode::Pos,
        DcsMode::Neg,
        DcsMode::Clk0Low,
        DcsMode::Clk0High,
        DcsMode::Clk1Low,
        DcsMode::Clk1High,
        DcsMode::Clk0,
        DcsMode::Clk1,
        DcsMode::Low,
        DcsMode::High,
    ] {
        for bypass in [Zero, One] {
            for sel in 0..4 {
                let initial = [
                    Zero,
                    Zero,
                    Logic::from_bool(sel & 1 != 0),
                    Logic::from_bool(sel & 2 != 0),
                    bypass,
                ];
                let mut batches = vec![];
                for (i, (a, b)) in [
                    (Zero, Zero),
                    (One, Zero),
                    (One, One),
                    (Zero, One),
                    (Zero, Zero),
                ]
                .into_iter()
                .enumerate()
                {
                    batches.push((i as u64 + 1, vec![(0, a), (1, b)]));
                }
                // Every static mode also sees all table/control transitions.
                if !matches!(mode, DcsMode::Pos | DcsMode::Neg) {
                    for n in 0..16 {
                        batches.push((
                            n + 10,
                            vec![
                                (0, Logic::from_bool(n & 1 != 0)),
                                (1, Logic::from_bool(n & 2 != 0)),
                                (2, Logic::from_bool(n & 4 != 0)),
                                (3, Logic::from_bool(n & 8 != 0)),
                            ],
                        ));
                    }
                }
                dcs_fast_twin(mode, initial, &batches);
            }
        }
    }
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_exact_switch_edges_at_many_phases_in_both_directions() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        for old in [0, 1] {
            for phase in [0, 1, 3, 7, 11, 17] {
                for change in [69, 75, 83, 91] {
                    let mut batches = std::collections::BTreeMap::<u64, Vec<_>>::new();
                    for (n, half) in [10, 14].into_iter().enumerate() {
                        let start = if n == 0 { 2 } else { phase + 2 };
                        for i in 0..400 {
                            let t = start + i * half;
                            if t <= 350 {
                                batches
                                    .entry(t)
                                    .or_default()
                                    .push((n, Logic::from_bool(i % 2 == 0)));
                            }
                        }
                    }
                    let new_sel = 2 - old;
                    batches.entry(change).or_default().extend([
                        (2, Logic::from_bool(new_sel & 1 != 0)),
                        (3, Logic::from_bool(new_sel & 2 != 0)),
                    ]);
                    dcs_fast_twin(
                        mode,
                        [
                            Zero,
                            Zero,
                            Logic::from_bool(old == 0),
                            Logic::from_bool(old == 1),
                            Zero,
                        ],
                        &batches.into_iter().collect::<Vec<_>>(),
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_bypass_stopped_coincident_and_retargeted_switches() {
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        let a = if mode == DcsMode::Pos { One } else { Zero };
        dcs_fast_twin(
            mode,
            [One, Zero, One, Zero, Zero],
            &[
                (1, vec![(2, Zero), (3, One)]),
                (2, vec![(4, One)]),
                (3, vec![(1, One)]),
                (4, vec![(2, One)]),
            ],
        );
        for coincident in [false, true] {
            let mut stop = vec![(0, a)];
            if coincident {
                stop.push((1, a));
            }
            let mut batches = vec![
                (1, vec![(2, Zero), (3, One)]),
                (2, stop),
                (1_000_000, vec![]),
            ];
            for i in 0..6 {
                batches.push((
                    1_000_001 + i,
                    vec![(1, if i % 2 == 0 { a } else { a.invert() })],
                ));
            }
            dcs_fast_twin(mode, [a.invert(), a.invert(), One, Zero, Zero], &batches);
        }
        // Coincident old/new active edges cannot launch and consume a stage.
        dcs_fast_twin(
            mode,
            [a.invert(), a.invert(), One, Zero, Zero],
            &[
                (1, vec![(2, Zero), (3, One)]),
                (2, vec![(0, a), (1, a)]),
                (3, vec![(1, a.invert())]),
                (4, vec![(1, a)]),
                (5, vec![(1, a.invert())]),
                (6, vec![(1, a)]),
                (7, vec![(1, a.invert())]),
            ],
        );
        let mut batches = vec![];
        for t in 1..=150 {
            let mut values = vec![];
            if t % 3 == 0 {
                values.push((0, Logic::from_bool(t / 3 % 2 != 0)));
            }
            if t % 5 == 0 {
                values.push((1, Logic::from_bool(t / 5 % 2 != 0)));
            }
            if let Some((sel, bypass)) = match t {
                1 => Some((2, false)),
                2 => Some((1, false)),
                4 => Some((2, false)),
                7 => Some((0, false)),
                11 => Some((1, false)),
                12 => Some((2, false)),
                18 => Some((2, true)),
                19 => Some((1, true)),
                20 => Some((1, false)),
                28 => Some((3, false)),
                40 => Some((2, false)),
                45 => Some((1, false)),
                _ => None,
            } {
                values.extend([
                    (2, Logic::from_bool(sel & 1 != 0)),
                    (3, Logic::from_bool(sel & 2 != 0)),
                    (4, Logic::from_bool(bypass)),
                ]);
            }
            batches.push((t, values));
        }
        dcs_fast_twin(mode, [Zero, Zero, One, Zero, Zero], &batches);
    }
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_derived_waves_threads_snapshots_tuning_and_dtr() {
    use ecp5_emu::{fast::FastSim, kernel::FastOptions};
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        for oscillator in [false, true] {
            for with_dtr in [false, true] {
                let fixture = Fixture::with_kernel();
                let device = configured(&fixture, 0);
                let (_, regs) = nodes(&device);
                let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
                let mut state = DeviceState::new(&device).unwrap();
                for node in regs {
                    state.write_register(node, Zero).unwrap();
                }
                let mut b = Builder::with_state(device, state).unwrap();
                let clock = signal(&mut b, "DCS_ROOT");
                let drive = driver(&mut b, clock, "dcs-root", Zero);
                let sel = signal(&mut b, "DCS_SELECT");
                let select = driver(&mut b, sel, "dcs-select", Zero);
                let start = signal(&mut b, "DCS_DTR_START");
                let start_drive = driver(&mut b, start, "dcs-dtr-start", Zero);
                let source = signal(&mut b, "Q0_SLICE");
                let mut source_ff = ff(regs[0], Input::Inverted(source), clock.into());
                source_ff.edge = if mode == DcsMode::Pos {
                    Edge::Rising
                } else {
                    Edge::Falling
                };
                b.flip_flop(source_ff).unwrap();
                let mut c0 = clock;
                if oscillator {
                    c0 = signal(&mut b, "DCS_OSC");
                    b.oscillator(OscillatorConfig {
                        resource,
                        output: c0,
                        half_period: ps(30_000),
                    })
                    .unwrap();
                }
                let out = signal(&mut b, "DCS_GENERATED");
                b.dcs(DcsConfig {
                    resource,
                    mode,
                    clocks: [c0.into(), source.into()],
                    select: [Input::Inverted(sel), sel.into()],
                    mode_select: Input::Constant(Zero),
                    output: out,
                })
                .unwrap();
                let q = signal(&mut b, "Q1_SLICE");
                b.flip_flop(ff(regs[1], Input::Inverted(q), out.into()))
                    .unwrap();
                let mut probes = vec![out, q, source];
                if with_dtr {
                    let outputs = std::array::from_fn(|i| signal(&mut b, &format!("DCS_DTR{i}")));
                    b.dtr(ecp5_emu::kernel::DtrConfig {
                        resource,
                        start: start.into(),
                        outputs,
                        code: 0xA5,
                    })
                    .unwrap();
                    probes.extend(outputs);
                }
                let mut previous = q;
                for i in 0..24 {
                    let next = signal(&mut b, &format!("DCS_TUNE{i}"));
                    b.mux(
                        &format!("dcs-tune-{i}"),
                        sel.into(),
                        previous.into(),
                        clock.into(),
                        next,
                    )
                    .unwrap();
                    previous = next;
                }
                probes.push(previous);
                for &s in &probes {
                    b.watch(s).unwrap();
                }
                let mut kernel = b.build().unwrap();
                let options = FastOptions {
                    threads: 2,
                    thread_min: 1,
                    collapse: 0,
                    ..FastOptions::from_env()
                };
                let prepared = FastSim::prepare(&kernel, &options).unwrap();
                let library = prepared.compile(false).unwrap();
                let mut first = FastSim::from_prepared(prepared, &library).unwrap();
                let mut second =
                    FastSim::from_prepared(FastSim::prepare(&kernel, &options).unwrap(), &library)
                        .unwrap();
                first.clock(drive, ps(40_000)).unwrap();
                second.clock(drive, ps(40_000)).unwrap();
                assert!(first.program().settle_phases.iter().any(|&p| p > 0));
                for n in 1..=6000 {
                    kernel
                        .schedule(ps(n * 20_000), drive, Logic::from_bool(n % 2 == 1))
                        .unwrap();
                }
                kernel.schedule(ps(1), start_drive, One).unwrap();
                kernel.schedule(ps(2), start_drive, Zero).unwrap();
                for sim in [&mut first, &mut second] {
                    sim.set_at(ps(1), &[(start_drive, true)]).unwrap();
                    sim.set_at(ps(2), &[(start_drive, false)]).unwrap();
                }
                let second_untouched = second.raw_values().to_vec();
                // The old edge stops first, then Q0 changes the new clock in
                // another wave at exactly the same timestamp.
                kernel.schedule(ps(10_000), select, One).unwrap();
                first.set_at(ps(10_000), &[(select, true)]).unwrap();
                first.run_until(ps(40_000)).unwrap();
                kernel.run_until(ps(40_000)).unwrap();
                let snapshot = first.snapshot();
                // Warm generation inside a pending handoff must retain its
                // remaining stages and the physical-time guard.
                let mut warm = FastSim::with_options(&kernel, &options).unwrap();
                warm.clock(drive, ps(40_000)).unwrap();
                for &p in &probes {
                    assert_eq!(warm.value(p), first.value(p));
                }
                let key = ecp5_emu::fast::choice_memory(&kernel);
                assert!(first.tune_for_checks(&kernel).unwrap() > 1);
                let mut expected = vec![];
                for n in 3..=6000 {
                    kernel.run_until(ps(n * 20_000)).unwrap();
                    expected.extend(
                        probes
                            .iter()
                            .map(|&p| u8::from(kernel.value(p).unwrap() == One)),
                    );
                }
                assert_eq!(
                    ecp5_emu::fast::choice_memory(&kernel),
                    key,
                    "live DCS state must not change choice identity"
                );
                assert_eq!(
                    first.run_edges_sampled(5998, &probes).unwrap(),
                    expected,
                    "mode={mode:?} osc={oscillator} dtr={with_dtr}"
                );
                assert_eq!(warm.run_edges_sampled(5998, &probes).unwrap(), expected);
                first.restore(&snapshot);
                assert_eq!(
                    first.run_edges_sampled(5998, &probes).unwrap(),
                    expected,
                    "restore across tuned layouts"
                );
                assert_eq!(
                    second.raw_values(),
                    second_untouched,
                    "shared library must not share DCS state"
                );
                second.set_at(ps(10_000), &[(select, true)]).unwrap();
                second.run_until(ps(40_000)).unwrap();
                assert_eq!(
                    second.run_edges_sampled(5998, &probes).unwrap(),
                    expected,
                    "second simulation using the same library"
                );
                // Starting with a remembered choice still accepts DCSC.
                std::fs::create_dir_all(key.parent().unwrap()).unwrap();
                let saved_choice = std::fs::read(&key).ok();
                let base = FastOptions::from_env();
                std::fs::write(
                    &key,
                    format!("1 6 1 {} {}\n", base.thread_min, base.barrier_lines),
                )
                .unwrap();
                let started = FastSim::start(&kernel);
                if let Some(saved) = saved_choice {
                    std::fs::write(&key, saved).unwrap();
                } else {
                    std::fs::remove_file(&key).unwrap();
                }
                let started = started.unwrap();
                if !FastOptions::pinned()[0] {
                    assert_eq!(started.program().options.threads, 1);
                }
                if !FastOptions::pinned()[2] {
                    assert_eq!(started.program().options.collapse, 6);
                }
                assert_eq!(started.value(out), Some(kernel.value(out).unwrap()));
            }
        }
    }
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_unknown_constants_require_a_consistent_target() {
    use ecp5_emu::fast::FastSim;
    let fixture = Fixture::with_kernel();
    let device = configured(&fixture, 0);
    let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut b = Builder::new(device).unwrap();
    let mode = signal(&mut b, "DCS_BYPASS");
    let bypass = driver(&mut b, mode, "dcs-bypass", Zero);
    let out = signal(&mut b, "DCS_CONSTANT");
    b.dcs(DcsConfig {
        resource,
        mode: DcsMode::Clk0,
        clocks: [Input::Constant(Zero), Input::Constant(X)],
        select: [Input::Constant(X); 2],
        mode_select: mode.into(),
        output: out,
    })
    .unwrap();
    b.watch(out).unwrap();
    let mut k = b.build().unwrap();
    let mut fast = FastSim::new(&k).unwrap();
    fast.step().unwrap();
    assert_eq!(
        fast.value(out),
        Some(Zero),
        "irrelevant unknown controls/clock must not cause refusal"
    );
    k.schedule(ps(1), bypass, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(out).unwrap(), X);
    let error = fast.set_at(ps(1), &[(bypass, true)]).unwrap_err();
    assert!(
        error.to_string().contains("DCSC 0 has an ambiguous target"),
        "{error:#}"
    );
}

#[test]
#[ignore = "requires clang"]
fn dcs_fast_owner_captures_and_tuned_restore_match_the_event_kernel() {
    use ecp5_emu::{fast::FastSim, kernel::FastOptions};
    for mode in [DcsMode::Pos, DcsMode::Neg] {
        let fixture = Fixture::with_kernel();
        let device = configured(&fixture, 0);
        let (_, regs) = nodes(&device);
        let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let mut state = DeviceState::new(&device).unwrap();
        for node in regs {
            state.write_register(node, Zero).unwrap();
        }
        let mut b = Builder::with_state(device, state).unwrap();
        let clock = signal(&mut b, "DCS_OWNER_ROOT");
        let drive = driver(&mut b, clock, "dcs-owner-root", Zero);
        let sel = signal(&mut b, "DCS_OWNER_SEL");
        let select = driver(&mut b, sel, "dcs-owner-sel", Zero);
        let clocks = [clock, signal(&mut b, "DCS_OWNER_C1")];
        b.oscillator(OscillatorConfig {
            resource,
            output: clocks[1],
            half_period: ps(30_000),
        })
        .unwrap();
        let out = signal(&mut b, "DCS_OWNER_OUT");
        b.dcs(DcsConfig {
            resource,
            mode,
            clocks: clocks.map(Input::from),
            select: [Input::Inverted(sel), sel.into()],
            mode_select: Input::Constant(Zero),
            output: out,
        })
        .unwrap();
        let mut probes = vec![out];
        for (n, node) in regs.into_iter().enumerate() {
            let q = signal(&mut b, &format!("Q{n}_SLICE"));
            let mut previous = q;
            for i in 0..30 {
                let next = signal(&mut b, &format!("DCS_OWNER_{n}_{i}"));
                b.mux(
                    &format!("dcs-owner-{n}-{i}"),
                    if i % 2 == 0 { clock.into() } else { sel.into() },
                    previous.into(),
                    if n == 0 { Input::Inverted(q) } else { q.into() },
                    next,
                )
                .unwrap();
                previous = next;
            }
            b.flip_flop(ff(node, Input::Inverted(previous), out.into()))
                .unwrap();
            probes.extend([q, previous]);
        }
        for &p in &probes {
            b.watch(p).unwrap();
        }
        let mut kernel = b.build().unwrap();
        let options = FastOptions {
            threads: 2,
            thread_min: 1,
            collapse: 0,
            ..FastOptions::from_env()
        };
        let mut fast = FastSim::with_options(&kernel, &options).unwrap();
        assert_eq!(fast.program().clock_groups, 1);
        assert!(
            fast.program()
                .source
                .contains("pool_run(v, capture_g0_rise)"),
            "must exercise captures owned by a worker"
        );
        fast.clock(drive, ps(40_000)).unwrap();
        kernel.schedule(ps(10_001), select, One).unwrap();
        fast.set_at(ps(10_001), &[(select, true)]).unwrap();
        let snapshot = fast.snapshot();
        for n in 1..=6000 {
            kernel
                .schedule(ps(n * 20_000), drive, Logic::from_bool(n % 2 == 1))
                .unwrap();
        }
        let mut expected = vec![];
        for n in 1..=6000 {
            kernel.run_until(ps(n * 20_000)).unwrap();
            expected.extend(
                probes
                    .iter()
                    .map(|&p| u8::from(kernel.value(p).unwrap() == One)),
            );
        }
        assert_eq!(
            fast.run_edges_sampled(6000, &probes).unwrap(),
            expected,
            "owner captures and fast_schedule"
        );
        fast.restore(&snapshot);
        assert!(fast.tune_for_checks(&kernel).unwrap() > 1);
        assert_eq!(
            fast.run_edges_sampled(6000, &probes).unwrap(),
            expected,
            "owner/caller program swaps"
        );
        fast.restore(&snapshot);
        assert_eq!(
            fast.run_edges_sampled(6000, &probes).unwrap(),
            expected,
            "restore from an owner layout after program swaps"
        );
    }
}

fn dtr_fixture(initial: Logic, code: u8) -> (Fixture, Kernel, DriverId, [SignalId; 8]) {
    let fixture = Fixture::with_kernel();
    let device = configured(&fixture, 0);
    let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut builder = Builder::new(device).unwrap();
    let root = signal(&mut builder, "DTR_START_ROOT");
    let input = signal(&mut builder, "DTR_START_ALIAS");
    let drive = driver(&mut builder, root, "dtr-start", initial);
    builder
        .buffer("dtr-start-route", root.into(), input)
        .unwrap();
    let outputs = std::array::from_fn(|bit| signal(&mut builder, &format!("DTROUT{bit}")));
    builder
        .dtr(ecp5_emu::kernel::DtrConfig {
            resource,
            start: input.into(),
            outputs,
            code,
        })
        .unwrap();
    (fixture, builder.build().unwrap(), drive, outputs)
}

fn dtr_bus(kernel: &Kernel, outputs: [SignalId; 8]) -> [Logic; 8] {
    outputs.map(|output| kernel.value(output).unwrap())
}
fn dtr_code(code: u8) -> [Logic; 8] {
    std::array::from_fn(|bit| Logic::from_bool(code & (1 << bit) != 0))
}

#[test]
fn dtr_initial_profile_and_all_measured_pulse_widths() {
    use ecp5_emu::kernel::DTR_CONVERSION_PS;
    assert_eq!(DTR_CONVERSION_PS, 32_740_000);
    for code in [0x9E, 0xA5] {
        for width in [40_000, 400_000, 100_000_000, 600_000_000] {
            let (_fixture, mut kernel, start, outputs) = dtr_fixture(Zero, code);
            assert_eq!(dtr_bus(&kernel, outputs), dtr_code(code));
            kernel.run_until(ps(50_000_000)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), dtr_code(code));
            let rise = 50_040_000;
            let fall = rise + width;
            kernel.schedule(ps(rise), start, One).unwrap();
            kernel.schedule(ps(fall), start, Zero).unwrap();
            kernel.run_until(ps(rise)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
            // Check throughout high and conversion, including every observer tick.
            kernel.run_until(ps(fall - 1)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
            for offset in (0..DTR_CONVERSION_PS).step_by(40_000) {
                kernel.run_until(ps(fall + offset)).unwrap();
                assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
            }
            kernel.run_until(ps(fall + DTR_CONVERSION_PS - 1)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
            kernel.run_until(ps(fall + DTR_CONVERSION_PS)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), dtr_code(code));
            kernel.run_until(ps(fall + 2 * DTR_CONVERSION_PS)).unwrap();
            assert_eq!(dtr_bus(&kernel, outputs), dtr_code(code));
        }
    }
}

#[test]
fn dtr_retrigger_restarts_without_queue_and_cancels_old_completion() {
    use ecp5_emu::kernel::DTR_CONVERSION_PS;
    for (spacing, count) in [
        (10_000_000, 2),
        (30_000_000, 2),
        (60_000_000, 2),
        (20_000_000, 8),
    ] {
        let (_fixture, mut kernel, start, outputs) = dtr_fixture(Zero, 0x9E);
        for n in 0..count {
            let rise = 40_000 + n * spacing;
            kernel.schedule(ps(rise), start, One).unwrap();
            kernel.schedule(ps(rise + 40_000), start, Zero).unwrap();
        }
        let old_end = 80_000 + DTR_CONVERSION_PS;
        kernel.run_until(ps(old_end)).unwrap();
        assert_eq!(
            dtr_bus(&kernel, outputs),
            if spacing > DTR_CONVERSION_PS {
                dtr_code(0x9E)
            } else {
                [Zero; 8]
            }
        );
        let end = 80_000 + (count - 1) * spacing + DTR_CONVERSION_PS;
        kernel.run_until(ps(end - 1)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
        kernel.run_until(ps(end)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), dtr_code(0x9E));
        kernel.run_until(ps(end + 10 * DTR_CONVERSION_PS)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), dtr_code(0x9E));
        assert_eq!(
            kernel.statistics().discarded_timer_events > 0,
            spacing < DTR_CONVERSION_PS
        );
    }
}

#[test]
fn dtr_unknown_controls_and_initial_high_stay_two_valued() {
    use ecp5_emu::kernel::DTR_CONVERSION_PS;
    for initial in [One, X, Z] {
        let (_fixture, mut kernel, start, outputs) = dtr_fixture(initial, 0x9E);
        assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
        kernel.run_until(ps(2 * DTR_CONVERSION_PS)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
        let low = 3 * DTR_CONVERSION_PS;
        kernel.schedule(ps(low), start, Zero).unwrap();
        kernel.run_until(ps(low + DTR_CONVERSION_PS)).unwrap();
        assert_eq!(
            dtr_bus(&kernel, outputs),
            if initial == One {
                dtr_code(0x9E)
            } else {
                [Zero; 8]
            }
        );
        kernel
            .schedule(ps(5 * DTR_CONVERSION_PS), start, One)
            .unwrap();
        kernel
            .schedule(ps(6 * DTR_CONVERSION_PS), start, Zero)
            .unwrap();
        kernel
            .schedule(ps(6 * DTR_CONVERSION_PS + 1), start, X)
            .unwrap();
        kernel.run_until(ps(8 * DTR_CONVERSION_PS)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), [Zero; 8]);
        kernel
            .schedule(ps(9 * DTR_CONVERSION_PS), start, One)
            .unwrap();
        kernel
            .schedule(ps(10 * DTR_CONVERSION_PS), start, Zero)
            .unwrap();
        kernel.run_until(ps(11 * DTR_CONVERSION_PS)).unwrap();
        assert_eq!(dtr_bus(&kernel, outputs), dtr_code(0x9E));
    }
}

struct DtrClock {
    _fixture: Fixture,
    kernel: Kernel,
    data: DriverId,
    clock: DriverId,
    probes: Vec<SignalId>,
}

fn dtr_clock(oscillator: bool) -> DtrClock {
    dtr_clock_with_tuning_gates(oscillator, false)
}

fn dtr_clock_with_tuning_gates(oscillator: bool, tuning_gates: bool) -> DtrClock {
    let fixture = Fixture::with_kernel();
    let device = configured(&fixture, 0);
    let (_, regs) = nodes(&device);
    let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let mut state = DeviceState::new(&device).unwrap();
    for node in regs {
        state.write_register(node, Zero).unwrap();
    }
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = signal(&mut b, "DTR_CLOCK");
    let clk = driver(&mut b, clock, "clock", Zero);
    let data = signal(&mut b, "DTR_START_DATA");
    let di = driver(&mut b, data, "start-data", Zero);
    let start = b.flip_flop(ff(regs[0], data.into(), clock.into())).unwrap();
    // Keep a routed STARTPULSE alias.
    let routed = signal(&mut b, "DTR_START_ROUTED");
    b.buffer("start-route", start.into(), routed).unwrap();
    let outputs = std::array::from_fn(|bit| signal(&mut b, &format!("DTROUT{bit}")));
    b.dtr(ecp5_emu::kernel::DtrConfig {
        resource,
        start: routed.into(),
        outputs,
        code: 0x9E,
    })
    .unwrap();
    let mut capture = ff(regs[1], outputs[7].into(), clock.into());
    capture.edge = Edge::Falling;
    let sampled = b.flip_flop(capture).unwrap();
    let mut probes = outputs.to_vec();
    probes.extend([sampled, start]);
    if oscillator {
        let osc = signal(&mut b, "DTR_OSC");
        b.oscillator(OscillatorConfig {
            resource,
            output: osc,
            half_period: ps(30_000),
        })
        .unwrap();
        probes.push(osc);
    }
    if tuning_gates {
        // Different folding widths must produce real candidates, even on this
        // tiny primitive fixture. The extra gates do not drive STARTPULSE.
        let mut previous = start;
        for n in 0..24 {
            let next = signal(&mut b, &format!("DTR_FOLD{n}"));
            b.mux(
                &format!("dtr-fold-{n}"),
                outputs[n % 8].into(),
                previous.into(),
                clock.into(),
                next,
            )
            .unwrap();
            previous = next;
        }
        probes.push(previous);
    }
    for &probe in &probes {
        b.watch(probe).unwrap();
    }
    DtrClock {
        _fixture: fixture,
        kernel: b.build().unwrap(),
        data: di,
        clock: clk,
        probes,
    }
}

impl DtrClock {
    fn fast(&self) -> ecp5_emu::fast::FastSim {
        let mut sim = ecp5_emu::fast::FastSim::new(&self.kernel).unwrap();
        sim.clock(self.clock, ps(40_000)).unwrap();
        sim
    }
    fn clocks(&mut self, edges: u64) {
        for edge in 1..=edges {
            self.kernel
                .schedule(
                    ps(edge * 20_000),
                    self.clock,
                    Logic::from_bool(edge % 2 == 1),
                )
                .unwrap();
        }
    }
    fn row(&self) -> Vec<u8> {
        self.probes
            .iter()
            .map(|&p| u8::from(self.kernel.value(p).unwrap() == One))
            .collect()
    }
    fn agree(&mut self, sim: &mut ecp5_emu::fast::FastSim, time: Time) {
        self.kernel.run_until(time).unwrap();
        sim.run_until(time).unwrap();
        let row: Vec<_> = self
            .probes
            .iter()
            .map(|&p| u8::from(sim.value(p) == Some(One)))
            .collect();
        assert_eq!(row, self.row(), "at {time}");
    }
}

#[test]
fn dtr_fast_fall_completion_and_coincident_capture_rows() {
    for oscillator in [false, true] {
        let mut rig = dtr_clock(oscillator);
        let mut sim = rig.fast();
        rig.clocks(12_000);
        for (t, value) in [(5_000, true), (45_000, false)] {
            rig.kernel
                .schedule(ps(t), rig.data, Logic::from_bool(value))
                .unwrap();
            sim.set_at(ps(t), &[(rig.data, value)]).unwrap();
        }
        // The STARTPULSE FF falls on the second rising clock, at 60 ns.
        let complete = 60_000 + ecp5_emu::kernel::DTR_CONVERSION_PS;
        let state = sim.snapshot();
        let start = sim.now().unwrap().ratio().0;
        let mut expected = Vec::new();
        for edge in start / 20_000 + 1..=12_000 {
            rig.kernel.run_until(ps(edge * 20_000)).unwrap();
            expected.extend(rig.row());
            if edge * 20_000 == complete {
                assert_eq!(
                    rig.kernel.value(rig.probes[8]).unwrap(),
                    One,
                    "completion is published before the coincident falling-edge capture"
                );
            }
        }
        sim.trace(&rig.probes).unwrap();
        let rows = sim
            .run_edges_sampled(12_000 - start / 20_000, &rig.probes)
            .unwrap();
        assert_eq!(rows, expected, "oscillator={oscillator}");
        let trace = sim.take_trace();
        assert!(trace.times_ps.contains(&complete));
        sim.restore(&state);
        assert_eq!(
            sim.run_edges_sampled(12_000 - start / 20_000, &rig.probes)
                .unwrap(),
            expected
        );
        sim.restore(&state);
        let falling: Vec<_> = expected
            .chunks_exact(rig.probes.len())
            .enumerate()
            .filter(|(i, _)| (start / 20_000 + 1 + *i as u64).is_multiple_of(2))
            .flat_map(|(_, row)| row.iter().copied())
            .collect();
        assert_eq!(
            sim.run_cycles_sampled((12_000 - start / 20_000).div_ceil(2), &rig.probes)
                .unwrap(),
            falling
        );
        println!(
            "DTR rows match on {} instants, oscillator={oscillator}; coincident capture sees done",
            expected.len() / rig.probes.len()
        );
    }
}

#[test]
fn dtr_fast_retrigger_high_hold_and_exact_nonclock_completion() {
    let mut rig = dtr_clock(false);
    let mut sim = rig.fast();
    rig.clocks(10_000);
    let delay = ecp5_emu::kernel::DTR_CONVERSION_PS;
    // Retrigger before the first completion, then hold high past that timer.
    for (time, value) in [
        (5_000, true),
        (45_000, false),
        (85_000, true),
        (40_005_000, false),
    ] {
        rig.kernel
            .schedule(ps(time), rig.data, Logic::from_bool(value))
            .unwrap();
        sim.set_at(ps(time), &[(rig.data, value)]).unwrap();
        rig.agree(&mut sim, ps(time));
    }
    assert_eq!(&rig.row()[..8], &[0; 8]);
    let complete = 40_020_000 + delay;
    rig.agree(&mut sim, ps(complete - 1));
    assert_eq!(&rig.row()[..8], &[0; 8]);
    rig.agree(&mut sim, ps(complete));
    assert_eq!(&rig.row()[..8], &[0, 1, 1, 1, 1, 0, 0, 1]);

    // A direct STARTPULSE fall between clock edges completes between them.
    let (_fixture, mut kernel, start, outputs) = dtr_fixture(Zero, 0xA5);
    let mut sim = ecp5_emu::fast::FastSim::new(&kernel).unwrap();
    for (t, value) in [(1, true), (17, false)] {
        kernel
            .schedule(ps(t), start, Logic::from_bool(value))
            .unwrap();
        sim.set_at_levels(ps(t), &[(start, Some(value))]).unwrap();
    }
    for t in [17 + delay - 1, 17 + delay, 17 + 2 * delay] {
        kernel.run_until(ps(t)).unwrap();
        sim.run_until(ps(t)).unwrap();
        assert_eq!(
            outputs.map(|p| sim.value(p).unwrap()),
            dtr_bus(&kernel, outputs)
        );
    }
}

#[test]
fn dtr_fast_step_handoff_and_live_answering_rows() {
    let mut rig = dtr_clock(false);
    rig.clocks(6_000);
    rig.kernel.schedule(ps(5_000), rig.data, One).unwrap();
    rig.kernel.schedule(ps(45_000), rig.data, Zero).unwrap();
    rig.kernel.run_until(ps(80_000)).unwrap();
    let mut sim = rig.fast(); // conversion timer is already pending
    let mut expected = Vec::new();
    for edge in 5..=6_000 {
        rig.kernel.run_until(ps(edge * 20_000)).unwrap();
        expected.push(rig.row());
    }
    let mut index = 0;
    sim.run_edges_with(5_996, &rig.probes, &[], |row, _, _| {
        let (_, row) = row.unwrap();
        assert_eq!(row, expected[index]);
        index += 1;
        Ok(true)
    })
    .unwrap();
    assert_eq!(index, expected.len());

    let (_fixture, mut kernel, start, outputs) = dtr_fixture(One, 0x9E);
    let mut sim = ecp5_emu::fast::FastSim::new(&kernel).unwrap();
    sim.set(start, false).unwrap();
    sim.step().unwrap();
    kernel.schedule(ps(1), start, Zero).unwrap();
    kernel.run_until(ps(1)).unwrap();
    // step's fall was at zero, the kernel's at 1 ps.
    sim.run_until(ps(ecp5_emu::kernel::DTR_CONVERSION_PS))
        .unwrap();
    kernel
        .run_until(ps(ecp5_emu::kernel::DTR_CONVERSION_PS + 1))
        .unwrap();
    assert_eq!(
        outputs.map(|p| sim.value(p).unwrap()),
        dtr_bus(&kernel, outputs)
    );
}

#[test]
fn dtr_fast_rational_deadline_snapshot_and_unknown_start() {
    for initial in [X, Z] {
        let (_fixture, kernel, _, _) = dtr_fixture(initial, 0x9E);
        let error = kernel
            .fast_program()
            .err()
            .expect("unknown STARTPULSE must stop fast mode");
        assert!(format!("{error:#}").contains("fast mode"));
    }
    let (_fixture, mut kernel, start, outputs) = dtr_fixture(Zero, 0xA5);
    let mut sim = ecp5_emu::fast::FastSim::new(&kernel).unwrap();
    for (t, value) in [
        (Time::new(1, 3).unwrap(), true),
        (Time::new(2, 3).unwrap(), false),
    ] {
        kernel.schedule(t, start, Logic::from_bool(value)).unwrap();
        kernel.run_until(t).unwrap();
        sim.set_at(t, &[(start, value)]).unwrap();
    }
    let snapshot = sim.snapshot();
    let complete = Time::new(2, 3)
        .unwrap()
        .checked_add(ps(ecp5_emu::kernel::DTR_CONVERSION_PS))
        .unwrap();
    // Refining the host tick must scale the pending deadline too.
    let before = Time::new(complete.ratio().0 * 2 - 1, 6).unwrap();
    kernel.run_until(before).unwrap();
    sim.run_until(before).unwrap();
    assert_eq!(outputs.map(|p| sim.value(p).unwrap()), [Zero; 8]);
    kernel.run_until(complete).unwrap();
    sim.run_until(complete).unwrap();
    assert_eq!(
        outputs.map(|p| sim.value(p).unwrap()),
        dtr_bus(&kernel, outputs)
    );
    sim.restore(&snapshot);
    sim.run_until(complete).unwrap();
    assert_eq!(
        outputs.map(|p| sim.value(p).unwrap()),
        dtr_bus(&kernel, outputs)
    );
}

#[test]
fn dtr_background_tuning_matches_disabled_tuning() {
    use std::time::{Duration, Instant};
    const CHILD: &str = "ECP5_DTR_TUNING_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let temp = std::env::temp_dir().join(format!("ecp5-dtr-tune-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "dtr_background_tuning_matches_disabled_tuning",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("TMPDIR", &temp)
            .env("FAST_THREAD_REPORT", "1");
        for option in [
            "FAST_TUNE",
            "FAST_THREADS",
            "FAST_THREAD_MIN",
            "FAST_COLLAPSE",
            "FAST_BARRIER_LINES",
        ] {
            child.env_remove(option);
        }
        let status = child.status().unwrap();
        std::fs::remove_dir_all(&temp).unwrap();
        assert!(status.success());
        return;
    }
    let mut rig = dtr_clock_with_tuning_gates(false, true);
    rig.clocks(12_000);
    for (time, value) in [(5_000, One), (45_000, Zero), (65_000, One), (105_000, Zero)] {
        rig.kernel.schedule(ps(time), rig.data, value).unwrap();
    }
    // A retrigger left a cancelled deadline at 32.80 us, followed by the
    // active one at 32.88 us. Start both simulations mid-conversion.
    rig.kernel.run_until(ps(160_000)).unwrap();
    std::env::set_var("FAST_TUNE", "0");
    let mut plain = rig.fast();
    plain.tune(&rig.kernel).unwrap();
    assert!(plain.tuning_report().is_none());
    std::env::remove_var("FAST_TUNE");
    let mut tuned = rig.fast();
    tuned.tune(&rig.kernel).unwrap();
    assert!(tuned
        .tuning_report()
        .unwrap()
        .contains("generating candidates"));
    let count = 12_000 - 8;
    let expected = plain.run_edges_sampled(count, &rig.probes).unwrap();
    assert_eq!(
        tuned.run_edges_sampled(count, &rig.probes).unwrap(),
        expected
    );
    let width = rig.probes.len();
    let before = ((32_880_000 / 20_000) - 8 - 2) as usize;
    assert_eq!(
        expected[before * width + 7],
        0,
        "the cancelled completion must not publish"
    );
    assert_eq!(
        expected[(before + 1) * width + 7],
        1,
        "the active completion must publish"
    );
    let started = Instant::now();
    let mut pieces = 1usize;
    loop {
        // Keep converting while candidates are measured, rather than only
        // checking an idle DTR after its first completion.
        let at = plain.now().unwrap();
        assert_eq!(tuned.now().unwrap(), at);
        let high = pieces.is_multiple_of(3);
        plain.set_at(at, &[(rig.data, high)]).unwrap();
        tuned.set_at(at, &[(rig.data, high)]).unwrap();
        let expected = plain.run_edges_sampled(4096, &rig.probes).unwrap();
        assert_eq!(
            tuned.run_edges_sampled(4096, &rig.probes).unwrap(),
            expected
        );
        pieces += 1;
        let report = tuned.tuning_report().unwrap();
        assert!(!report.contains("failed:"), "{report}");
        if started.elapsed() >= Duration::from_secs(2)
            && !report.contains("generating candidates")
            && !report.contains("compiling")
            && !report.contains("; 0 measurements")
            && report.lines().count() > 1
        {
            println!("DTR background tuning: {pieces} sampled pieces match FAST_TUNE=0; {report}");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "{report}");
    }
}
