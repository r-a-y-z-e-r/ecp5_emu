mod common;
use common::Fixture;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{Builder, Edge, FlipFlop, Input, LutRam, Options, Reset, ResetKind, Strength, Time},
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        NodeId,
    },
    prepared::ConfiguredDevice,
    state::DeviceState,
};
use serde_json::json;
use std::sync::Arc;

fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}
fn reference(name: &str) -> Reference {
    Reference::Wire(wire(name))
}
fn configured(f: &Fixture, data: &[u8], frames: usize, bits: usize) -> Arc<ConfiguredDevice> {
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F",
        "frames":frames,"bits_per_frame":bits,"encoding":"frame_byte_lsb0",
        "cram_hex":data.iter().map(|v|format!("{v:02x}")).collect::<String>(),"bram_data":{},
        "origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalogue = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[]})).unwrap()],
        "LFE5U-85F",
        frames,
        bits,
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap())
}
fn node(device: &ConfiguredDevice, name: &str) -> NodeId {
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    device
        .model()
        .nodes(plc)
        .find(|n| n.native_name == name)
        .unwrap()
        .id
}
fn ps(time: u64) -> Time {
    Time::from_ps(time)
}

#[test]
fn writes_share_physical_lut_storage_and_invalidate_a_stationary_read_address() {
    let f = Fixture::with_kernel();
    let device = configured(&f, &[0; 8], 4, 16);
    let lut = node(&device, "SLICEA.K0");
    let mut state = DeviceState::new(&device).unwrap();
    for bit in 0..16 {
        state.write_lut(lut, bit, Zero).unwrap();
    }
    let mut b = Builder::with_state(Arc::clone(&device), state).unwrap();
    let clock = b.signal(wire("WCK")).unwrap();
    let data = b.signal(wire("WD")).unwrap();
    let ck = b
        .external_driver(clock, "clock", Strength::Strong, One)
        .unwrap();
    let wd = b
        .external_driver(data, "data", Strength::Strong, One)
        .unwrap();
    let output = b
        .lut_ram(LutRam {
            node: lut,
            read_address: [Input::Constant(Zero); 4],
            write_address: [Input::Constant(Zero); 4],
            data: data.into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: Edge::Rising,
        })
        .unwrap();
    // A LUT cannot acquire a second independent behavior/storage owner.
    assert!(b.lut4(lut).is_err());
    let mut k = b.build().unwrap();
    assert_eq!(k.value(output).unwrap(), Zero); // initially high clock creates no write
    k.schedule(ps(1), ck, Zero).unwrap();
    k.schedule(ps(2), ck, One).unwrap();
    k.run_until(ps(2)).unwrap();
    assert_eq!(k.value(output).unwrap(), One); // no read-address transition
    assert_eq!(k.state().lut(lut).unwrap()[0], One);
    assert!(k.state().lut(lut).unwrap()[1..].iter().all(|&v| v == Zero));
    k.schedule(ps(3), wd, Zero).unwrap();
    k.run_until(ps(3)).unwrap();
    assert_eq!(k.value(output).unwrap(), One); // high clock is not a transparent write
    k.schedule(ps(4), ck, Zero).unwrap();
    k.schedule(ps(5), ck, One).unwrap();
    k.run_until(ps(5)).unwrap();
    assert_eq!(k.value(output).unwrap(), Zero);
    assert_eq!(k.statistics().lut_bit_changes, 2);
    assert_eq!(
        device
            .word(
                device.model().tile_id("R1C1:PLC2").unwrap(),
                "SLICEA.K0.INIT"
            )
            .unwrap(),
        [One; 16]
    );
}

#[test]
fn uncertain_writes_merge_only_compatible_cells_and_normalize_high_impedance_data() {
    let f = Fixture::with_kernel();
    for edge in [Edge::Rising, Edge::Falling] {
        let device = configured(&f, &[0; 8], 4, 16);
        let lut = node(&device, "SLICEA.K0");
        let mut state = DeviceState::new(&device).unwrap();
        for bit in 0..16 {
            state.write_lut(lut, bit, Zero).unwrap();
        }
        let mut b = Builder::with_state(device, state).unwrap();
        let mut signals = Vec::new();
        let mut drivers = Vec::new();
        let initial_clock = if edge == Edge::Rising { Zero } else { One };
        let active_clock = initial_clock.invert();
        for (name, value) in [("CK", initial_clock), ("EN", One), ("WD", One), ("WA", X)] {
            let s = b.signal(wire(name)).unwrap();
            drivers.push(b.external_driver(s, name, Strength::Strong, value).unwrap());
            signals.push(s);
        }
        b.lut_ram(LutRam {
            node: lut,
            read_address: [Input::Constant(Zero); 4],
            write_address: [
                signals[3].into(),
                Input::Constant(Zero),
                Input::Constant(Zero),
                Input::Constant(Zero),
            ],
            data: signals[2].into(),
            clock: signals[0].into(),
            enable: signals[1].into(),
            edge,
        })
        .unwrap();
        let mut k = b.build().unwrap();
        let stages = [
            (active_clock, One, One, X, [X, X]), // exactly one of cells 0 and 1 may change
            (active_clock, One, Zero, Zero, [X, X]), // inputs alone never write
            (initial_clock, One, Zero, Zero, [X, X]),
            (active_clock, One, Zero, Zero, [Zero, X]), // definite write recovers cell 0
            (initial_clock, One, One, One, [Zero, X]),
            (active_clock, One, One, One, [Zero, One]),
            (initial_clock, X, One, Zero, [Zero, One]),
            (active_clock, X, One, Zero, [X, One]), // uncertain enable holds or writes
            (initial_clock, Z, One, One, [X, One]),
            (active_clock, Z, One, One, [X, One]), // both possibilities agree at cell 1
            (initial_clock, Zero, Z, Zero, [X, One]),
            (active_clock, Zero, Z, Zero, [X, One]), // disabled writes preserve storage
            (initial_clock, One, Z, One, [X, One]),
            (active_clock, One, Z, One, [X, X]), // Z is never stored as a known memory bit
        ];
        for (i, (clock, enable, data, address, expected)) in stages.into_iter().enumerate() {
            let values = [clock, enable, data, address];
            let updates = drivers.iter().copied().zip(values).collect::<Vec<_>>();
            k.schedule_batch(ps(i as u64 + 1), &updates, None).unwrap();
            k.run_until(ps(i as u64 + 1)).unwrap();
            assert_eq!(
                &k.state().lut(lut).unwrap()[..2],
                &expected,
                "stage {i}, {edge:?}"
            );
            assert!(k.state().lut(lut).unwrap()[2..].iter().all(|&v| v == Zero));
        }
        // A possible edge merges the write with holding, rather than treating X
        // as a definite active transition. Both polarities use the same rule.
        k.schedule(ps(20), drivers[0], initial_clock).unwrap();
        k.schedule_batch(
            ps(21),
            &[(drivers[0], X), (drivers[2], One), (drivers[3], Zero)],
            None,
        )
        .unwrap();
        k.run_until(ps(21)).unwrap();
        assert_eq!(k.state().lut(lut).unwrap()[0], X);
        assert!(k.statistics().possible_clock_edges > 0);
    }
}

#[test]
fn simultaneous_ff_capture_sees_prewrite_read_and_local_reset_preserves_memory() {
    let f = Fixture::with_kernel();
    let device = configured(&f, &[0; 8], 4, 16);
    let lut = node(&device, "SLICEA.K0");
    let ff = node(&device, "SLICEA.FF0");
    let mut state = DeviceState::new(&device).unwrap();
    state.write_lut(lut, 0, Zero).unwrap();
    let mut b = Builder::with_state(device, state).unwrap();
    let clock = b.signal(wire("CK")).unwrap();
    let reset = b.signal(wire("RESET")).unwrap();
    let ck = b
        .external_driver(clock, "clock", Strength::Strong, Zero)
        .unwrap();
    let rst = b
        .external_driver(reset, "reset", Strength::Strong, Zero)
        .unwrap();
    let output = b
        .lut_ram(LutRam {
            node: lut,
            read_address: [Input::Constant(Zero); 4],
            write_address: [Input::Constant(Zero); 4],
            data: Input::Constant(One),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: Edge::Rising,
        })
        .unwrap();
    let q = b
        .flip_flop(FlipFlop {
            preload: None,
            node: ff,
            data: output.into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: Edge::Rising,
            reset: Some(Reset {
                input: reset.into(),
                kind: ResetKind::Asynchronous,
                value: Zero,
            }),
        })
        .unwrap();
    let mut k = b.build().unwrap();
    k.schedule(ps(1), ck, One).unwrap();
    k.run_until(ps(1)).unwrap();
    assert_eq!(k.value(q).unwrap(), Zero);
    assert_eq!(k.value(output).unwrap(), One);
    k.schedule(ps(2), ck, Zero).unwrap();
    k.schedule(ps(3), ck, One).unwrap();
    k.run_until(ps(3)).unwrap();
    assert_eq!(k.value(q).unwrap(), One);
    k.schedule(ps(4), rst, One).unwrap();
    k.run_until(ps(4)).unwrap();
    assert_eq!(k.value(q).unwrap(), Zero);
    assert_eq!(k.state().lut(lut).unwrap()[0], One);
}

fn set_bit(data: &mut [u8; 48], frame: usize, bit: usize, value: bool) {
    let offset = 3 + bit;
    let byte = &mut data[(frame + 2) * 4 + offset / 8];
    if value {
        *byte |= 1 << (offset % 8);
    } else {
        *byte &= !(1 << (offset % 8));
    }
}
fn image_data() -> [u8; 48] {
    let mut data = [0; 48];
    for lane in 0..8 {
        for physical in 0..16 {
            let address = ((physical & 1) << 3) | (physical & 6) | ((physical & 8) >> 3);
            let value = lane < 4 && ((3 * address + 5) & (1 << lane) != 0);
            set_bit(&mut data, lane, physical, !value);
        }
    }
    set_bit(&mut data, 8, 0, true);
    data
}
fn request(scope: Scope) -> Request {
    let mut drives = vec![
        (reference("CLK1"), Zero),
        (reference("LSR1"), One),
        (reference("CLK0"), Zero),
        (reference("LSR0"), Zero),
    ];
    for name in [
        "A0", "B0", "C0", "D0", "A1", "B1", "C1", "D1", "A2", "B2", "C2", "D2", "A3", "B3", "C3",
        "D3", "A4", "B4", "C4", "D4", "A5", "B5", "C5", "D5",
    ] {
        drives.push((reference(&format!("{name}_SLICE")), Zero));
    }
    Request {
        package: "CABGA381".into(),
        scope,
        startup: Startup::ReadyUnknown,
        drives,
        probes: vec![
            reference("F0_SLICE"),
            reference("F1_SLICE"),
            reference("F2_SLICE"),
            reference("F3_SLICE"),
        ],
        options: Options::default(),
    }
}
fn observe(c: &compile::Compiled) -> u8 {
    (0..4).fold(0, |value, lane| {
        value
            | ((c
                .value(&reference(&format!("F{lane}_SLICE")))
                .unwrap()
                .bit()
                .unwrap() as u8)
                << lane)
    })
}

#[test]
fn automatic_ramw_mapping_reaches_all_addresses_and_columns_with_both_clock_polarities() {
    let f = Fixture::with_lutram();
    for inverted in [false, true] {
        let mut data = image_data();
        set_bit(&mut data, 9, 0, inverted);
        let device = configured(&f, &data, 12, 32);
        let mut req = request(Scope::Observed);
        req.drives[0].1 = Logic::from_bool(inverted);
        let mut c = compile::compile(device, req).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(c.report.counts.lutram_columns, 4);
        assert_eq!(c.report.counts.ramw, 1);
        assert_eq!(c.report.counts.lut4, 0);
        let mut expected = [0u8; 16];
        for (a, value) in expected.iter_mut().enumerate() {
            *value = (3 * a + 5) as u8 & 15;
        }
        let mut time = 0;
        let mut apply = |c: &mut compile::Compiled, values: Vec<(String, Logic)>| {
            time += 1;
            let updates = values
                .iter()
                .map(|(name, value)| (c.drivers[&format!("R1C1:{name}")], *value))
                .collect::<Vec<_>>();
            c.kernel.schedule_batch(ps(time), &updates, None).unwrap();
            c.kernel.run_until(ps(time)).unwrap();
        };
        let read = |a: usize| {
            (0..4)
                .flat_map(|lane| {
                    ["D", "B", "C", "A"]
                        .into_iter()
                        .enumerate()
                        .map(move |(bit, p)| {
                            (
                                format!("{p}{lane}_SLICE"),
                                Logic::from_bool(a & (1 << bit) != 0),
                            )
                        })
                })
                .collect::<Vec<_>>()
        };
        for (a, &value) in expected.iter().enumerate() {
            apply(&mut c, read(a));
            assert_eq!(observe(&c), value);
        }
        for (a, stored) in expected.iter_mut().enumerate() {
            let value = (a ^ 10) as u8;
            let mut updates = read(a);
            for (bit, p) in ["D4", "B4", "C4", "A4"].into_iter().enumerate() {
                updates.push((format!("{p}_SLICE"), Logic::from_bool(a & (1 << bit) != 0)));
            }
            for (lane, p) in ["C5", "A5", "D5", "B5"].into_iter().enumerate() {
                updates.push((
                    format!("{p}_SLICE"),
                    Logic::from_bool(value & (1 << lane) != 0),
                ));
            }
            apply(&mut c, updates);
            assert_eq!(observe(&c), *stored); // changing WAD/DI does not write
            apply(&mut c, vec![("CLK1".into(), Logic::from_bool(!inverted))]);
            *stored = value;
            assert_eq!(observe(&c), value);
            apply(&mut c, vec![("CLK1".into(), Logic::from_bool(inverted))]);
        }
        for (a, &value) in expected.iter().enumerate() {
            apply(&mut c, read(a));
            assert_eq!(observe(&c), value);
        }
        assert_eq!(c.kernel.state().summary().lut_arrays, 8);
    }
}

#[test]
fn automatic_write_enable_modes_and_inactive_unobserved_memory_keep_their_semantics() {
    let f = Fixture::with_lutram();
    for mode in ["WRE", "INV", "0", "1"] {
        let mut data = image_data();
        set_bit(&mut data, 9, 1, ["INV", "0"].contains(&mode));
        set_bit(&mut data, 9, 2, ["0", "1"].contains(&mode));
        set_bit(&mut data, 9, 3, ["0", "1"].contains(&mode));
        let mut req = request(Scope::AllPlcRegisters);
        req.probes.clear(); // no read result is requested: writes must still run
        let mut c = compile::compile(configured(&f, &data, 12, 32), req)
            .unwrap_or_else(|e| panic!("{mode}: {e:#}"));
        let lane = node(c.kernel.device(), "SLICEA.K0");
        assert_eq!(c.report.counts.lutram_columns, 4);
        assert_eq!(c.report.counts.flip_flops, 8);
        let clock = c.drivers["R1C1:CLK1"];
        c.kernel.schedule(ps(1), clock, One).unwrap();
        c.kernel.run_until(ps(1)).unwrap();
        assert_eq!(
            c.kernel.state().lut(lane).unwrap()[0],
            if ["WRE", "1"].contains(&mode) {
                Zero
            } else {
                One
            },
            "{mode}"
        );
        c.kernel
            .schedule_batch(
                ps(2),
                &[(clock, Zero), (c.drivers["R1C1:LSR1"], Zero)],
                None,
            )
            .unwrap();
        c.kernel.schedule(ps(3), clock, One).unwrap();
        c.kernel.run_until(ps(3)).unwrap();
        assert_eq!(
            c.kernel.state().lut(lane).unwrap()[0],
            if mode == "0" { One } else { Zero },
            "{mode}"
        );
    }
}

#[test]
fn ramw_c_registers_preserve_m_path_and_report_unmodelled_di_without_executing_c_luts() {
    let f = Fixture::with_lutram();
    for sd_one in [false, true] {
        let mut data = image_data();
        set_bit(&mut data, 9, 4, sd_one);
        let mut req = request(Scope::Observed);
        req.probes = vec![reference("Q4_SLICE")];
        req.drives.push((reference("M4_SLICE"), One));
        let mut c = compile::compile(configured(&f, &data, 12, 32), req).unwrap();
        c.kernel
            .schedule(ps(1), c.drivers["R1C1:CLK0"], One)
            .unwrap();
        c.kernel.run_until(ps(1)).unwrap();
        assert_eq!(
            c.value(&reference("Q4_SLICE")).unwrap(),
            if sd_one { X } else { One }
        );
        assert_eq!(c.report.counts.lut4, 0);
        if sd_one {
            assert_eq!(c.report.lutram[0].unmodelled_c_ff_di, ["SLICEC.FF0"]);
        } else {
            assert!(c.report.lutram.is_empty());
        }
    }
}

#[test]
fn mixed_modes_nonzero_c_init_and_ramw_combinational_outputs_are_rejected() {
    let f = Fixture::with_lutram();
    for case in ["mixed", "init", "c-output", "wide"] {
        let mut data = image_data();
        let mut req = request(Scope::Observed);
        match case {
            "mixed" => set_bit(&mut data, 8, 2, true),
            "init" => set_bit(&mut data, 4, 0, false),
            "c-output" => req.probes = vec![reference("F4_SLICE")],
            "wide" => req.probes = vec![reference("F5A_SLICE")],
            _ => unreachable!(),
        }
        let error = compile::compile(configured(&f, &data, 12, 32), req)
            .err()
            .expect("unsupported memory configuration was accepted");
        let message = format!("{error:#}");
        assert!(
            message.contains(match case {
                "mixed" => "ambiguous",
                "init" => "C INIT",
                _ => "unsupported PLC mode",
            }),
            "{case}: {message}"
        );
    }
}

struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Fast mode (src/fast.rs) against the kernel on random LUTRAM traffic, in
/// two runs: a column written on the rising clock edge with an enable and a
/// write inhibit, then one written on the inverted clock with an inverted
/// enable. Both flip-flops are on the write edge: one registers the write
/// data, so the write must store its value from before the edge; the other
/// reads the column's output, so its capture must see the cell from before
/// the write. Read address, write address, data, enable and inhibit change at
/// random, alone or with a clock edge; the output, both flip-flops and all
/// sixteen cells must agree after each instant.
#[test]
#[ignore = "requires clang"]
fn fast_mode_matches_the_kernel_on_random_lutram_traffic() {
    random_lutram_traffic(false);
    random_lutram_traffic(true);
}

fn random_lutram_traffic(inverted: bool) {
    let f = Fixture::with_kernel();
    let device = configured(&f, &[0; 8], 4, 16);
    let column = node(&device, "SLICEA.K0");
    let flip_flops = [node(&device, "SLICEA.FF0"), node(&device, "SLICEA.FF1")];
    let mut r = Xorshift(0x9e37_79b9_7f4a_7c15 ^ u64::from(inverted));
    let mut state = DeviceState::new(&device).unwrap();
    for bit in 0..16 {
        let v = Logic::from_bool(r.next().is_multiple_of(2));
        state.write_lut(column, bit, v).unwrap();
    }
    for &ff in &flip_flops {
        state.write_register(ff, Zero).unwrap();
    }
    let mut b = Builder::with_state(Arc::clone(&device), state).unwrap();
    let mut drivers = Vec::new();
    let mut input = |b: &mut Builder, name: String| {
        let s = b.signal(wire(&name)).unwrap();
        let d = b.external_driver(s, &name, Strength::Strong, Zero).unwrap();
        drivers.push((name, d));
        s
    };
    let clock = input(&mut b, "CLK".into());
    let wad = [0, 1, 2, 3].map(|i| input(&mut b, format!("WAD{i}")));
    let rad = [0, 1, 2, 3].map(|i| input(&mut b, format!("RAD{i}")));
    let wd = input(&mut b, "WD".into());
    let wre = input(&mut b, "WRE".into());
    let inhibit = input(&mut b, "GWDIS".into());
    let write_edge = if inverted {
        Edge::Falling
    } else {
        Edge::Rising
    };
    let flip_flop = |b: &mut Builder, node: NodeId, data: Input| {
        b.flip_flop(FlipFlop {
            preload: None,
            node,
            data,
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: write_edge,
            reset: None,
        })
        .unwrap()
    };
    let data = flip_flop(&mut b, flip_flops[0], wd.into());
    let config = LutRam {
        node: column,
        read_address: rad.map(Input::from),
        write_address: wad.map(Input::from),
        data: data.into(),
        clock: if inverted {
            Input::Inverted(clock)
        } else {
            clock.into()
        },
        enable: if inverted {
            Input::Inverted(wre)
        } else {
            wre.into()
        },
        edge: Edge::Rising,
    };
    let output = if inverted {
        b.lut_ram(config).unwrap()
    } else {
        b.lut_ram_with_inhibit(config, inhibit.into()).unwrap()
    };
    let q = [data, flip_flop(&mut b, flip_flops[1], output.into())];
    let mut k = b.build().unwrap();
    let mut fast = ecp5_emu::fast::FastSim::new(&k).unwrap();
    assert_eq!(fast.program().lutrams.len(), 1);
    let mut level = vec![false; drivers.len()];
    for step in 1..=20_000_u64 {
        let mut set = Vec::new();
        for (i, (name, driver)) in drivers.iter().enumerate() {
            let flip = match name.as_str() {
                "CLK" => r.next().is_multiple_of(3),
                "GWDIS" => r.next().is_multiple_of(50),
                _ => r.next().is_multiple_of(4),
            };
            if flip {
                level[i] = !level[i];
                set.push((*driver, level[i]));
            }
        }
        let time = Time::from_ps(step);
        let logic: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        k.schedule_batch(time, &logic, None).unwrap();
        k.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        let what = format!("inverted {inverted}, step {step}");
        assert_eq!(
            fast.value(output),
            Some(k.value(output).unwrap()),
            "{what}: output"
        );
        for (c, &q) in q.iter().enumerate() {
            assert_eq!(
                fast.value(q),
                Some(k.value(q).unwrap()),
                "{what}: flip-flop {c}"
            );
        }
        let cells: Vec<u8> = k
            .state()
            .lut(column)
            .unwrap()
            .iter()
            .map(|&v| u8::from(v == One))
            .collect();
        assert_eq!(fast.lutram(0).unwrap(), cells.as_slice(), "{what}: cells");
    }
    // The traffic did write: about 3300 write edges, half of them enabled,
    // half of those not inhibited, half of those changing the cell.
    let writes = k.statistics().lut_bit_changes;
    assert!(writes > 200, "inverted {inverted}: {writes} cell changes");
}

/// The planner owns all four physical columns even when only F0 is used.
/// Unused RAMW data and read-address ports may float. A live port may not.
#[test]
fn fast_lutram_ignores_dead_columns_but_refuses_unknown_live_inputs() {
    let f = Fixture::with_lutram();
    let device = configured(&f, &image_data(), 12, 32);
    let narrow = || {
        let mut req = request(Scope::Observed);
        req.probes.truncate(1);
        req.drives.retain(|(r, _)| {
            let label = r.label();
            ![
                "A1", "B1", "C1", "D1", "A2", "B2", "C2", "D2", "A3", "B3", "C3", "D3", "A5", "B5",
                "D5",
            ]
            .iter()
            .any(|p| label == format!("R1C1:{p}_SLICE"))
        });
        req
    };
    let c = compile::compile(Arc::clone(&device), narrow()).unwrap();
    c.kernel.fast_program().unwrap_or_else(|e| panic!("{e:#}"));
    for port in ["A0_SLICE", "D4_SLICE", "C5_SLICE", "CLK1", "LSR1"] {
        for unknown in [Some(X), Some(Z), None] {
            let mut req = narrow();
            let r = reference(port);
            if let Some(unknown) = unknown {
                let drive = req.drives.iter_mut().find(|(p, _)| *p == r).unwrap();
                drive.1 = unknown;
            } else {
                req.drives.retain(|(p, _)| *p != r);
            }
            let c = compile::compile(Arc::clone(&device), req).unwrap();
            let error = c
                .kernel
                .fast_program()
                .err()
                .expect("a live unknown input was accepted");
            let error = format!("{error:#}");
            assert!(
                error.contains("fast mode reads") || error.contains("fast mode is two-valued"),
                "{port}={unknown:?}: {error}"
            );
        }
    }
}

/// A used read-only column does not consume the physical write port either.
#[test]
fn fast_lutram_omits_disabled_write_address_data_and_clock() {
    let f = Fixture::with_lutram();
    let mut data = image_data();
    for bit in [1, 2, 3] {
        set_bit(&mut data, 9, bit, true); // WREMUX=0
    }
    let mut req = request(Scope::Observed);
    req.probes.truncate(1);
    req.drives.retain(|(r, _)| {
        ![
            "CLK1", "A4_SLICE", "B4_SLICE", "C4_SLICE", "D4_SLICE", "C5_SLICE",
        ]
        .iter()
        .any(|p| *r == reference(p))
    });
    let c = compile::compile(configured(&f, &data, 12, 32), req).unwrap();
    c.kernel.fast_program().unwrap_or_else(|e| panic!("{e:#}"));
}
