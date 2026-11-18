mod common;
use common::{write, Fixture};
use ecp5_emu::{
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        ebr_cell, Builder, DriverId, Ebr, EbrMode, EbrPort, Input, Kernel, ResetKind, SignalId,
        Strength, Time, WriteMode, PDP36_A_PATH_OWN_CONTROLS, PDP36_LOW_PATH,
    },
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        ResourceId,
    },
    prepared::ConfiguredDevice,
    state::DeviceState,
};
use serde_json::json;
use std::{collections::HashMap, fs, sync::Arc};

fn width_shift(w: usize) -> usize {
    match w {
        1 => 0,
        2 => 1,
        4 => 2,
        9 => 3,
        18 => 4,
        36 => 5,
        _ => panic!(),
    }
}
fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}
fn fixture() -> (Fixture, Arc<ConfiguredDevice>, ResourceId) {
    let f = Fixture::with_compiler();
    f.add_ebr("EBR0");
    let path = f.evidence.join("resource_ports.csv");
    let mut ports = fs::read_to_string(&path).unwrap();
    for side in ['A', 'B'] {
        let inputs = (0..18)
            .map(|b| format!("DI{side}{b}"))
            .chain((0..14).map(|b| format!("AD{side}{b}")))
            .chain((0..3).map(|b| format!("CS{side}{b}")))
            .chain(
                ["CLK", "CE", "OCE", "WE", "RST"]
                    .into_iter()
                    .map(|p| format!("{p}{side}")),
            );
        for input in inputs {
            ports.push_str(&format!("LFE5U-85F/R1C1/EBR0,{input},in,1,1,{input}_EBR\n"));
        }
        for bit in 0..18 {
            ports.push_str(&format!(
                "LFE5U-85F/R1C1/EBR0,DO{side}{bit},out,1,1,DO{side}{bit}_EBR\n"
            ));
        }
    }
    write(&path, ports);
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
    let id = device.model().resource_id("LFE5U-85F/R1C1/EBR0").unwrap();
    (f, device, id)
}
struct Harness {
    _fixture: Fixture,
    k: Kernel,
    id: ResourceId,
    outputs: [Vec<SignalId>; 2],
    drivers: HashMap<String, DriverId>,
    time: u64,
    widths: [usize; 2],
}
impl Harness {
    fn new(widths: [usize; 2], edit: impl FnOnce(&mut Ebr)) -> Self {
        let (f, device, id) = fixture();
        let mut state = DeviceState::new(&device).unwrap();
        for row in 0..2048 {
            let v = ((row * 73) ^ 0x155) & 511;
            state
                .write_ebr_word(
                    id,
                    row,
                    std::array::from_fn(|bit| Logic::from_bool(v & (1 << bit) != 0)),
                )
                .unwrap();
        }
        let mut b = Builder::with_state(device, state).unwrap();
        let mut drivers = HashMap::new();
        let mut input = |name: String, value: Logic| {
            let signal = b.signal(wire(&name)).unwrap();
            let driver = b
                .external_driver(signal, &name, Strength::Strong, value)
                .unwrap();
            drivers.insert(name, driver);
            Input::from(signal)
        };
        let ports = std::array::from_fn(|side| {
            let letter = ['A', 'B'][side];
            EbrPort {
                width: widths[side],
                output_register: false,
                write_mode: WriteMode::Normal,
                clock: input(format!("CLK{letter}"), Zero),
                enable: input(format!("CE{letter}"), One),
                output_enable: input(format!("OCE{letter}"), One),
                write_enable: input(format!("WE{letter}"), Zero),
                select: std::array::from_fn(|bit| input(format!("CS{letter}{bit}"), Zero)),
                decode: 0,
                reset: input(format!("RST{letter}"), Zero),
                address: std::array::from_fn(|bit| {
                    input(
                        format!("AD{letter}{bit}"),
                        Logic::from_bool(widths[side] >= 18 && bit < widths[side] / 9),
                    )
                }),
                data: std::array::from_fn(|bit| input(format!("DI{letter}{bit}"), Zero)),
            }
        });
        let gsr = input("GSR".into(), Zero);
        let inhibit = input("GWDIS".into(), Zero);
        let mut config = Ebr {
            resource: id,
            mode: if widths[0] == 36 {
                EbrMode::Pdp
            } else {
                EbrMode::Dp
            },
            ports,
            reset_kind: ResetKind::Synchronous,
            asynchronous_release: true,
            global_reset: Some(gsr),
            write_inhibit: inhibit,
        };
        edit(&mut config);
        let outputs = b.ebr(config.clone()).unwrap();
        assert!(b.ebr(config).is_err());
        // Probed, as compile does for observed signals.
        for &s in outputs.iter().flatten() {
            b.watch(s).unwrap();
        }
        Self {
            _fixture: f,
            k: b.build().unwrap(),
            id,
            outputs,
            drivers,
            time: 0,
            widths,
        }
    }
    fn set(&mut self, changes: &[(&str, Logic)]) {
        self.time += 1;
        let updates = changes
            .iter()
            .map(|(name, value)| (self.drivers[*name], *value))
            .collect::<Vec<_>>();
        self.k
            .schedule_batch(Time::from_ps(self.time), &updates, None)
            .unwrap();
        self.k.run_until(Time::from_ps(self.time)).unwrap();
    }
    fn bus(&mut self, prefix: &str, value: usize, count: usize) {
        let names = (0..count)
            .map(|bit| format!("{prefix}{bit}"))
            .collect::<Vec<_>>();
        let values = names
            .iter()
            .enumerate()
            .map(|(bit, name)| (name.as_str(), Logic::from_bool(value & (1 << bit) != 0)))
            .collect::<Vec<_>>();
        self.set(&values);
    }
    fn address(&mut self, side: usize, address: usize, mask: usize) {
        self.bus(
            &format!("AD{}", ['A', 'B'][side]),
            (address << width_shift(self.widths[side]))
                | if self.widths[side] >= 18 { mask } else { 0 },
            14,
        );
    }
    fn data(&mut self, side: usize, value: usize) {
        self.bus(&format!("DI{}", ['A', 'B'][side]), value, 18);
    }
    fn edge(&mut self, side: usize) {
        let clock = format!("CLK{}", ['A', 'B'][side]);
        self.set(&[(&clock, Zero)]);
        self.set(&[(&clock, One)]);
    }
    fn output(&self, side: usize) -> Vec<Logic> {
        self.outputs[side]
            .iter()
            .map(|s| self.k.value(*s).unwrap())
            .collect()
    }
    fn stored(&self, width: usize, address: usize) -> Vec<Logic> {
        (0..width)
            .map(|bit| {
                let (row, b) = ebr_cell(width, address, bit).unwrap();
                self.k.state().ebr_word(self.id, row).unwrap()[b]
            })
            .collect()
    }
}

#[test]
fn canonical_views_cover_capacity_without_losing_parity_bits() {
    for width in [1, 2, 4, 9, 18, 36] {
        let mut seen = std::collections::BTreeSet::new();
        for a in 0..(1 << (14 - width_shift(width))) {
            for b in 0..width {
                assert!(seen.insert(ebr_cell(width, a, b).unwrap()));
            }
        }
        assert_eq!(seen.len(), if width < 9 { 16384 } else { 18432 });
        assert_eq!(seen.iter().map(|(r, _)| *r).max(), Some(2047));
        assert!(ebr_cell(width, 1 << (14 - width_shift(width)), 0).is_err());
    }
    assert!(ebr_cell(3, 0, 0).is_err());
}
#[test]
fn all_25_width_pairs_read_and_write_one_shared_array_with_independent_clocks() {
    for a in [1, 2, 4, 9, 18] {
        for b in [1, 2, 4, 9, 18] {
            let mut h = Harness::new([a, b], |_| {});
            h.address(0, 17, 3);
            let before = h.stored(a, 17);
            let data = before
                .iter()
                .enumerate()
                .fold(0, |n, (bit, v)| n | (((*v == Zero) as usize) << bit));
            h.data(0, data);
            h.set(&[("WEA", One)]);
            assert_eq!(h.output(0), vec![X; a]);
            h.edge(0);
            assert_eq!(
                h.stored(a, 17),
                (0..a)
                    .map(|bit| Logic::from_bool(data & (1 << bit) != 0))
                    .collect::<Vec<_>>()
            );
            assert_ne!(before, h.stored(a, 17));
            assert_eq!(h.output(0), vec![X; a]);
            assert_eq!(h.output(1), vec![X; b]);
            h.set(&[("WEA", Zero)]);
            let (row, _) = ebr_cell(a, 17, 0).unwrap();
            let read_address = if b < 9 { row * (8 / b) } else { row / (b / 9) };
            h.address(1, read_address, 3);
            h.edge(1);
            assert_eq!(h.output(1), h.stored(b, read_address));
            let held = h.output(1);
            h.address(1, read_address ^ 1, 3);
            assert_eq!(h.output(1), held); // NOREG stays synchronous
            h.edge(1);
            assert_eq!(h.output(1), h.stored(b, read_address ^ 1));
        }
    }
}
#[test]
fn write_modes_masks_and_extra_output_stage_have_distinct_results() {
    for mode in [
        WriteMode::Normal,
        WriteMode::WriteThrough,
        WriteMode::ReadBeforeWrite,
    ] {
        for mask in 0..4 {
            let mut h = Harness::new([18, 9], |c| c.ports[0].write_mode = mode);
            h.address(0, 5, 3);
            h.edge(0);
            let held = h.output(0);
            h.address(0, 7, mask);
            let memory = h.stored(18, 7);
            h.data(0, 0x2aaaa);
            h.set(&[("WEA", One)]);
            h.edge(0);
            let out = h.output(0);
            for bit in 0..18 {
                let data = Logic::from_bool(0x2aaaa & (1 << bit) != 0);
                let written = mask & (1 << (bit / 9)) != 0;
                assert_eq!(
                    h.stored(18, 7)[bit],
                    if written { data } else { memory[bit] }
                );
                let expect = match mode {
                    WriteMode::Normal => held[bit],
                    WriteMode::WriteThrough => {
                        if written {
                            data
                        } else {
                            Logic::mux(X, Logic::mux(X, held[bit], memory[bit]), data)
                        }
                    }
                    WriteMode::ReadBeforeWrite => {
                        if written {
                            memory[bit]
                        } else {
                            Logic::mux(X, held[bit], memory[bit])
                        }
                    }
                };
                assert_eq!(out[bit], expect);
            }
        }
    }
    let mut h = Harness::new([9, 9], |c| c.ports[1].output_register = true);
    h.address(1, 6, 0);
    h.edge(1);
    assert_eq!(h.output(1), vec![X; 9]);
    h.address(1, 7, 0);
    h.edge(1);
    assert_eq!(h.output(1), h.stored(9, 6));
    h.edge(1);
    assert_eq!(h.output(1), h.stored(9, 7));
}
#[test]
fn ce_cs_and_oce_preserve_or_merge_pending_reads() {
    let mut h = Harness::new([9, 9], |c| {
        c.ports[0].decode = 5;
        c.ports[1].output_register = true;
    });
    h.edge(0);
    assert_eq!(h.output(0), vec![X; 9]);
    h.bus("CSA", 5, 3);
    h.edge(0);
    let held = h.output(0);
    assert_eq!(held, h.stored(9, 0));
    h.address(0, 1, 0);
    h.set(&[("CEA", Zero)]);
    h.edge(0);
    assert_eq!(h.output(0), held);
    h.set(&[("CEA", One), ("OCEA", Zero)]);
    h.edge(0);
    assert_eq!(
        h.output(0),
        held.iter()
            .zip(h.stored(9, 1))
            .map(|(&a, b)| Logic::mux(X, a, b))
            .collect::<Vec<_>>()
    );
    h.edge(1);
    h.set(&[("OCEB", Zero)]);
    h.address(1, 1, 0);
    h.edge(1);
    assert_eq!(h.output(1), vec![X; 9]);
    h.set(&[("OCEB", One), ("CEB", Zero)]);
    h.edge(1);
    assert!(h.k.statistics().ebr_control_ambiguities > 0);
    assert!(h.k.ebr_notices().count() > 0);
}
#[test]
fn unknown_address_enable_and_clock_merge_writes_and_z_data_becomes_x() {
    for uncertain in ["ADA3", "WEA", "CEA", "CLKA"] {
        let mut h = Harness::new([9, 9], |_| {});
        h.data(0, 0);
        h.set(&[("WEA", One)]);
        h.set(&[(uncertain, X), ("DIA0", Z)]);
        let old0 = h.stored(9, 0);
        let old1 = h.stored(9, 1);
        if uncertain == "CLKA" {
            h.set(&[("CLKA", One)]);
        } else {
            h.edge(0);
        }
        let result = h.stored(9, 0);
        for bit in 0..9 {
            assert_eq!(
                result[bit],
                if bit == 0 {
                    X
                } else {
                    Logic::mux(X, old0[bit], Zero)
                }
            );
        }
        if uncertain == "ADA3" {
            assert_eq!(
                h.stored(9, 1),
                (0..9)
                    .map(|bit| if bit == 0 {
                        X
                    } else {
                        Logic::mux(X, old1[bit], Zero)
                    })
                    .collect::<Vec<_>>()
            );
        } else {
            assert_eq!(h.stored(9, 1), old1);
        }
    }
}

#[test]
fn unknown_mask_and_address_keep_known_agreements_without_clearing_other_groups() {
    let mut h = Harness::new([18, 9], |_| {});
    h.address(0, 4, 3);
    let old = h.stored(18, 4);
    h.data(0, 0);
    h.set(&[("WEA", One), ("ADA0", X), ("ADA1", Zero)]);
    h.edge(0);
    for (bit, &v) in old.iter().enumerate() {
        assert_eq!(
            h.stored(18, 4)[bit],
            if bit < 9 { Logic::mux(X, v, Zero) } else { v }
        );
    }
    h.set(&[("WEA", Zero), ("ADA4", X)]); // addresses 4 and 5, no new write
    let a = h.stored(18, 4);
    let b = h.stored(18, 5);
    h.edge(0);
    assert_eq!(
        h.output(0),
        a.iter()
            .zip(b)
            .map(|(&a, b)| Logic::mux(X, a, b))
            .collect::<Vec<_>>()
    );
}
#[test]
fn simultaneous_collisions_do_not_choose_a_port_and_separated_edges_are_safe() {
    for swap in [false, true] {
        let mut h = Harness::new([1, 1], |_| {});
        h.address(0, 0, 0);
        h.address(1, if swap { 1 } else { 0 }, 0);
        h.data(0, 1);
        h.data(1, 0);
        h.set(&[("WEA", One), ("WEB", One)]);
        h.set(&[("CLKA", One), ("CLKB", One)]);
        assert_eq!(h.k.state().ebr_word(h.id, 0).unwrap(), &[X; 9]);
        assert_eq!(h.k.statistics().ebr_collision_rows, 1);
    }
    let mut h = Harness::new([9, 9], |_| {});
    h.set(&[("WEA", One)]);
    h.data(0, 0x1a5);
    h.set(&[("CLKA", One), ("CLKB", One)]);
    assert_eq!(h.output(1), vec![X; 9]);
    assert_eq!(
        h.stored(9, 0),
        (0..9)
            .map(|b| Logic::from_bool(0x1a5 & (1 << b) != 0))
            .collect::<Vec<_>>()
    );
    h.set(&[("WEA", Zero)]);
    h.edge(1);
    assert_eq!(h.output(1), h.stored(9, 0));
}

#[test]
fn hb09_same_clock_full_words_follow_write_modes_and_b_wins_below_18() {
    for width in [1, 2, 4, 9, 18] {
        for mode in [
            WriteMode::Normal,
            WriteMode::WriteThrough,
            WriteMode::ReadBeforeWrite,
        ] {
            let mut h = Harness::new([width; 2], |c| {
                c.ports[1].clock = c.ports[0].clock;
                for port in &mut c.ports {
                    port.write_mode = mode;
                }
            });
            // Establish known held outputs before testing NORMAL writes.
            h.edge(0);
            for writes in [[true, false], [false, true], [true, true]] {
                let old = h.stored(width, 0);
                let held = [h.output(0), h.output(1)];
                let data = [vec![One; width], vec![Zero; width]];
                h.data(0, (1 << width) - 1);
                h.data(1, 0);
                h.set(&[
                    ("WEA", Logic::from_bool(writes[0])),
                    ("WEB", Logic::from_bool(writes[1])),
                ]);
                h.edge(0);
                for side in 0..2 {
                    let want = if writes[side] {
                        match mode {
                            WriteMode::Normal => &held[side],
                            WriteMode::WriteThrough => &data[side],
                            WriteMode::ReadBeforeWrite => &old,
                        }
                    } else if mode == WriteMode::ReadBeforeWrite {
                        &old
                    } else {
                        &data[1 - side]
                    };
                    assert_eq!(
                        &h.output(side),
                        want,
                        "width {width}, {mode:?}, {writes:?}, port {side}"
                    );
                }
                let want = if writes == [true, true] && width == 18 {
                    vec![X; width]
                } else if writes[1] {
                    data[1].clone()
                } else {
                    data[0].clone()
                };
                assert_eq!(h.stored(width, 0), want);
            }
        }
    }
}

#[test]
fn hb09_disjoint_narrow_words_in_one_row_are_independent() {
    for width in [1, 2, 4] {
        let mut h = Harness::new([width; 2], |c| {
            c.ports[1].clock = c.ports[0].clock;
            for port in &mut c.ports {
                port.write_mode = WriteMode::ReadBeforeWrite;
            }
        });
        h.address(1, 1, 0);
        let old = [h.stored(width, 0), h.stored(width, 1)];
        let mut row = *h.k.state().ebr_word(h.id, 0).unwrap();
        h.data(0, (1 << width) - 1);
        h.data(1, 0);
        h.set(&[("WEA", One), ("WEB", One)]);
        h.edge(0);
        assert_eq!(h.output(0), old[0]);
        assert_eq!(h.output(1), old[1]);
        row[..width].fill(One);
        row[width..2 * width].fill(Zero);
        assert_eq!(h.k.state().ebr_word(h.id, 0).unwrap(), &row);
        assert_eq!(h.k.statistics().ebr_collision_rows, 0);
    }
}

#[test]
fn hb09_does_not_resolve_mixed_widths_modes_or_partial_masks() {
    for (widths, mixed_mode, mask) in [([2, 4], false, 0), ([9, 9], true, 0), ([18, 18], false, 1)]
    {
        let mut h = Harness::new(widths, |c| {
            c.ports[1].clock = c.ports[0].clock;
            c.ports[0].write_mode = WriteMode::ReadBeforeWrite;
            c.ports[1].write_mode = if mixed_mode {
                WriteMode::Normal
            } else {
                WriteMode::ReadBeforeWrite
            };
        });
        h.address(1, 0, mask);
        let mut want = h.stored(widths[0], 0);
        want[..widths[0].min(9)].fill(X);
        h.set(&[("WEB", One)]);
        h.edge(0);
        assert_eq!(
            h.output(0),
            want,
            "{widths:?}, mixed mode {mixed_mode}, mask {mask}"
        );
        assert_eq!(h.k.statistics().ebr_collision_rows, 1);
    }
}
#[test]
fn resets_and_gwdis_preserve_the_array_and_global_enable_is_per_resource() {
    for asynchronous in [false, true] {
        let mut h = Harness::new([18, 9], |c| {
            if asynchronous {
                c.reset_kind = ResetKind::Asynchronous
            }
        });
        h.edge(0);
        let memory = h.stored(18, 0);
        h.set(&[("RSTA", One), ("CEA", Zero)]);
        if !asynchronous {
            h.edge(0);
        }
        assert_eq!(h.output(0), vec![Zero; 18]);
        assert_eq!(h.stored(18, 0), memory);
        h.set(&[("RSTA", Zero), ("CEA", One), ("WEA", One), ("GWDIS", One)]);
        h.data(0, 0);
        h.edge(0);
        assert_eq!(h.stored(18, 0), memory);
        h.set(&[("WEA", Zero), ("GSR", One)]);
        assert_eq!(h.output(0), vec![Zero; 18]);
        assert_eq!(h.output(1), vec![Zero; 9]);
        assert_eq!(h.stored(18, 0), memory);
    }
    let mut h = Harness::new([9, 9], |c| c.global_reset = None);
    h.edge(0);
    let held = h.output(0);
    h.set(&[("GSR", One)]);
    assert_eq!(h.output(0), held);
}
#[test]
fn pdp36_masks_write_four_groups_for_a_narrow_independent_read_port() {
    for width in [1, 2, 4, 9, 18] {
        for mask in 0..16 {
            let mut h = Harness::new([36, width], |_| {});
            h.address(0, 13, mask);
            let memory = h.stored(36, 13);
            h.data(0, 0x15555);
            h.data(1, 0x2aaaa);
            h.edge(0);
            for (bit, &old) in memory.iter().enumerate() {
                let v = if mask & (1 << (bit / 9)) == 0 {
                    old
                } else {
                    Logic::from_bool(
                        (if bit < 18 { 0x15555 } else { 0x2aaaa }) & (1 << (bit % 18)) != 0,
                    )
                };
                assert_eq!(h.stored(36, 13)[bit], v);
            }
            let address = if width < 9 {
                52 * (8 / width)
            } else {
                52 / (width / 9)
            };
            h.address(1, address, 3);
            h.edge(1);
            assert_eq!(h.output(1), h.stored(width, address));
            assert!(h.outputs[0].is_empty());
        }
    }
}
/// The bits of a 36-bit word, low bit first, as an integer (X as 0).
fn bits(values: &[Logic]) -> u64 {
    values
        .iter()
        .enumerate()
        .fold(0, |w, (bit, &v)| w | (u64::from(v == One) << bit))
}

/// A 36-bit PDP read (HB01): one read at port B drives both read paths,
/// bits 17:0 on path PDP36_LOW_PATH and 35:18 on the other; the read port's
/// RST and OCE act on both paths unless PDP36_A_PATH_OWN_CONTROLS.
#[test]
fn pdp36_wide_read_drives_doa_and_dob_from_the_read_port() {
    let low = PDP36_LOW_PATH;
    let a_port = if PDP36_A_PATH_OWN_CONTROLS { "A" } else { "B" };
    for registered in [false, true] {
        let mut h = Harness::new([36, 36], |c| {
            c.ports[0].output_register = registered;
            c.ports[1].output_register = registered;
        });
        assert_eq!(h.outputs.each_ref().map(Vec::len), [18, 18]);
        let mut model: Vec<u64> = (0..512).map(|a| bits(&h.stored(36, a))).collect();
        let mut r = Xorshift(0x9e37_79b9_7f4a_7c15);
        let read = |h: &mut Harness, address: usize| {
            h.address(1, address, 0);
            h.edge(1);
            if registered {
                h.edge(1);
            }
            let out = [h.output(0), h.output(1)];
            assert!(out.iter().flatten().all(|v| v.bit().is_some()));
            bits(&out[low]) | bits(&out[1 - low]) << 18
        };
        for _ in 0..200 {
            let (address, mask) = ((r.next() % 512) as usize, (r.next() % 16) as usize);
            let word = r.next() & ((1 << 36) - 1);
            h.address(0, address, mask);
            h.data(0, (word & 0x3ffff) as usize);
            h.data(1, (word >> 18) as usize);
            h.edge(0);
            for group in (0..4).filter(|g| mask & (1 << g) != 0) {
                let m = 0x1ff << (9 * group);
                model[address] = (model[address] & !m) | (word & m);
            }
            let address = if r.next().is_multiple_of(2) {
                address
            } else {
                (r.next() % 512) as usize
            };
            assert_eq!(read(&mut h, address), model[address], "word {address}");
        }
        // The read port's reset clears both paths; port A's reaches only
        // the path that HB01 gives it.
        assert_ne!(read(&mut h, 7), 0);
        h.set(&[("RSTB", One)]);
        h.edge(1);
        if registered {
            h.edge(1);
        }
        h.set(&[("RSTB", Zero)]);
        let cleared = [h.output(0), h.output(1)];
        let expected_b = |path: usize| path == 1 || a_port == "B";
        for (path, out) in cleared.iter().enumerate() {
            assert_eq!(
                bits(out) == 0,
                expected_b(path) || model[7] & (0x3ffff << (18 * usize::from(path != low))) == 0,
                "path {path} after RSTB"
            );
        }
        if registered {
            // OCEB=0 holds both registers of a B-controlled read.
            assert_eq!(read(&mut h, 9), model[9]);
            h.set(&[("OCEB", Zero)]);
            h.address(1, 10, 0);
            h.edge(1);
            h.edge(1);
            let held = bits(&h.output(low)) | bits(&h.output(1 - low)) << 18;
            let half = |w: u64, path: usize| (w >> (18 * usize::from(path != low))) & 0x3ffff;
            for path in 0..2 {
                let out = bits(&h.output(path));
                let follows_b = expected_b(path);
                assert_eq!(
                    out,
                    half(if follows_b { model[9] } else { model[10] }, path),
                    "path {path} with OCEB=0 ({held:x})"
                );
            }
            h.set(&[("OCEB", One)]);
        }
    }
}

#[test]
fn unsupported_release_fails_when_reset_is_used_and_polarity_inversion_is_an_edge() {
    let mut h = Harness::new([9, 9], |c| {
        c.reset_kind = ResetKind::Asynchronous;
        c.asynchronous_release = false;
        c.ports[0].clock = match c.ports[0].clock {
            Input::Signal(s) => Input::Inverted(s),
            _ => panic!(),
        };
    });
    h.set(&[("CLKA", One)]);
    assert_eq!(h.output(0), vec![X; 9]);
    h.set(&[("CLKA", Zero)]);
    assert_eq!(h.output(0), h.stored(9, 0));
    h.time += 1;
    h.k.schedule(Time::from_ps(h.time), h.drivers["RSTA"], One)
        .unwrap();
    assert!(h
        .k
        .run_until(Time::from_ps(h.time))
        .unwrap_err()
        .to_string()
        .contains("synchronous release"));
}

#[test]
fn unqualified_widths_are_rejected_before_execution() {
    for (mode, widths, write_mode) in [
        (EbrMode::Pdp, [18, 36], WriteMode::Normal),
        (EbrMode::Dp, [36, 18], WriteMode::Normal),
    ] {
        let (_f, device, id) = fixture();
        let mut b = Builder::new(device).unwrap();
        let ports = std::array::from_fn(|side| EbrPort {
            width: widths[side],
            output_register: false,
            write_mode,
            clock: Input::Constant(Zero),
            enable: Input::Constant(One),
            output_enable: Input::Constant(One),
            write_enable: Input::Constant(Zero),
            select: [Input::Constant(Zero); 3],
            decode: 0,
            reset: Input::Constant(Zero),
            address: [Input::Constant(Zero); 14],
            data: [Input::Constant(Zero); 18],
        });
        assert!(b
            .ebr(Ebr {
                resource: id,
                mode,
                ports,
                reset_kind: ResetKind::Synchronous,
                asynchronous_release: true,
                global_reset: None,
                write_inhibit: Input::Constant(Zero)
            })
            .is_err());
    }
}

#[test]
fn compiler_uses_composed_words_muxes_and_roots_unobserved_memory_writes() {
    use ecp5_emu::compile::{self, Reference, Request, Scope, Startup};
    let (_f, original, id) = fixture();
    let mut fields = Vec::new();
    for (name, value) in [
        ("MODE", "DP16KD"),
        ("RESETMODE", "SYNC"),
        ("ASYNC_RESET_RELEASE", "ASYNC"),
        ("GSR", "DISABLED"),
        ("DP16KD.DATA_WIDTH_A", "9"),
        ("DP16KD.DATA_WIDTH_B", "9"),
        ("DP16KD.WRITEMODE_A", "NORMAL"),
        ("DP16KD.WRITEMODE_B", "NORMAL"),
        ("REGMODE_A", "NOREG"),
        ("REGMODE_B", "NOREG"),
    ] {
        fields.push(json!({"name":format!("EBR0.{name}"),"kind":"enum","fragments":[{"tile":"R1C1:PLC2","source":name,"default":value,"options":{value:[[2,3,false]]}}]}));
    }
    for side in ['A', 'B'] {
        for name in ["CLK", "CE", "OCE", "WE", "RST"]
            .into_iter()
            .map(|p| format!("{p}{side}"))
            .chain((0..if side == 'A' { 4 } else { 2 }).map(|b| format!("AD{side}{b}")))
        {
            fields.push(json!({"name":format!("EBR0.{name}MUX"),"kind":"enum","fragments":[{"tile":"R1C1:PLC2","source":name,"default":name,"options":{name.clone():[[2,3,false]]}}]}));
        }
        // Two fragments of a word, including empty groups. B=110 is stored
        // reversed as 011; the runtime must not compare that value directly.
        let bits = if side == 'A' {
            vec![true, false, true]
        } else {
            vec![false, false, true]
        };
        fields.push(json!({"name":format!("EBR0.CSDECODE_{side}"),"kind":"word","fragments":[
            {"tile":"R1C1:PLC2","source":format!("CS{side}:lo"),"default":"000","word_bits":[[[2,4,bits[0]]],[],[]]},
            {"tile":"R1C1:PLC2","source":format!("CS{side}:hi"),"default":"000","word_bits":[[],[[2,5,bits[1]]],[[2,6,bits[2]]]]}]}));
    }
    let bind = |fields: &Vec<serde_json::Value>| {
        let catalog=Catalog::merge_documents(&[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[{"id":"LFE5U-85F/R1C1/EBR0","family":"EBR","fields":fields}]})).unwrap()],"LFE5U-85F",4,16).unwrap();
        Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(original.model()),
                Arc::clone(original.image()),
                catalog,
            )
            .unwrap(),
        )
    };
    let reference = |name: &str| Reference::Wire(wire(&format!("{name}_EBR")));
    let mut drives = Vec::new();
    for side in ['A', 'B'] {
        for (name, value) in [
            ("CLK", Zero),
            ("CE", One),
            ("OCE", One),
            ("WE", if side == 'A' { One } else { Zero }),
            ("RST", Zero),
        ] {
            drives.push((reference(&format!("{name}{side}")), value));
        }
        for bit in 0..14 {
            drives.push((reference(&format!("AD{side}{bit}")), Zero));
        }
        for bit in 0..9 {
            drives.push((reference(&format!("DI{side}{bit}")), One));
        }
        for bit in 0..3 {
            drives.push((
                reference(&format!("CS{side}{bit}")),
                Logic::from_bool((if side == 'A' { 2 } else { 6 }) & (1 << bit) != 0),
            ));
        }
    }
    let request = |scope, probes| Request {
        package: "CABGA381".into(),
        scope,
        startup: Startup::ReadyUnknown,
        drives: drives.clone(),
        probes,
        options: Default::default(),
    };
    let mut c = compile::compile(bind(&fields), request(Scope::AllPlcRegisters, vec![])).unwrap();
    assert_eq!(c.report.counts.ebr_blocks, 1);
    assert_eq!(c.report.ebr[0].raw_cs_decode, [2, 6]);
    c.kernel
        .schedule(Time::from_ps(1), c.drivers[&reference("CLKA").label()], One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(c.kernel.state().ebr_word(id, 0).unwrap(), &[One; 9]);
    let c = compile::compile(
        bind(&fields),
        request(
            Scope::Observed,
            vec![Reference::System(ecp5_emu::kernel::SystemPort::Ready)],
        ),
    )
    .unwrap();
    assert_eq!(c.report.counts.ebr_blocks, 0);
    assert!(compile::compile(
        bind(&fields),
        request(Scope::Observed, vec![reference("DOA9")])
    )
    .is_err());
    let mut invalid = fields.clone();
    invalid[0]["fragments"][0]["options"] = json!({"DP16KD":[[2,3,true]]});
    assert!(compile::compile(bind(&invalid), request(Scope::AllPlcRegisters, vec![])).is_err());
    // HB11 measured narrow RBW; it is now accepted, including mixed widths.
    let mut narrow = fields.clone();
    narrow[4]["fragments"][0]["options"] = json!({"4":[[2,3,false]]});
    narrow[6]["fragments"][0]["options"] = json!({"READBEFOREWRITE":[[2,3,false]]});
    let c = compile::compile(
        bind(&narrow),
        request(Scope::Observed, vec![reference("DOA0")]),
    )
    .unwrap();
    assert_eq!(c.report.ebr[0].widths, [4, 9]);
    assert_eq!(c.report.ebr[0].write_modes[0], WriteMode::ReadBeforeWrite);
    let mut pdp = fields.clone();
    pdp[0]["fragments"][0]["options"] = json!({"PDPW16KD":[[2,3,false]]});
    pdp.push(json!({"name":"EBR0.PDPW16KD.DATA_WIDTH_R","kind":"enum","fragments":[{"tile":"R1C1:PLC2","source":"pdp width","default":"9","options":{"9":[[2,3,false]]}}]}));
    let mut req = request(Scope::Observed, vec![reference("DOB0")]);
    req.drives
        .iter_mut()
        .find(|(r, _)| *r == reference("ADA0"))
        .unwrap()
        .1 = One;
    let mut c = compile::compile(bind(&pdp), req).unwrap();
    for (t, clock) in [(1, "CLKA"), (2, "CLKB")] {
        c.kernel
            .schedule(Time::from_ps(t), c.drivers[&reference(clock).label()], One)
            .unwrap();
        c.kernel.run_until(Time::from_ps(t)).unwrap();
    }
    assert_eq!(c.value(&reference("DOB0")).unwrap(), One);
    assert!(compile::compile(
        bind(&pdp),
        request(Scope::Observed, vec![reference("DOA0")])
    )
    .is_err());
    // A 36-bit read drives DOA and DOB; REGMODE_A must equal REGMODE_B.
    pdp.last_mut().unwrap()["fragments"][0]["options"] = json!({"36":[[2,3,false]]});
    let c = compile::compile(
        bind(&pdp),
        request(Scope::Observed, vec![reference("DOA17"), reference("DOB0")]),
    )
    .unwrap();
    assert_eq!(c.report.ebr[0].widths, [36, 36]);
    let mut mixed = pdp.clone();
    mixed
        .iter_mut()
        .find(|f| f["name"] == "EBR0.REGMODE_A")
        .unwrap()["fragments"][0]["options"] = json!({"OUTREG":[[2,3,false]]});
    mixed
        .iter_mut()
        .find(|f| f["name"] == "EBR0.REGMODE_A")
        .unwrap()["fragments"][0]["default"] = json!("OUTREG");
    let error = compile::compile(
        bind(&mixed),
        request(Scope::Observed, vec![reference("DOB0")]),
    )
    .err()
    .expect("REGMODE_A != REGMODE_B");
    assert!(format!("{error:#}").contains("REGMODE_A"), "{error:#}");
}

/// Fast mode (src/fast.rs) against the event kernel on one EBR with random
/// stimulus: every DP width pair, and PDP36 with every read width (36: both
/// read paths) twice, with write modes, output registers, CS decodes and reset kinds
/// drawn at random. Nothing reads the outputs here, so fast mode keeps their
/// X as the kernel does. Every output, X included, agrees after every
/// instant, and the whole array at the end.
///
/// The first pass keeps away from what the kernel makes X: byte masks or a
/// reset during a write with access, OCE without access on a registered port,
/// OCE low on an unregistered one, GWDIS, both clocks in one instant. The
/// second pass draws all of them, cross-port collisions included.
#[test]
#[ignore = "requires clang"]
fn fast_mode_matches_the_event_kernel_on_random_ebr_configurations() {
    random_ebr_configurations(false, false);
    random_ebr_configurations(true, false);
}

#[test]
#[ignore = "requires clang"]
fn fast_mode_matches_hb09_and_runtime_scope_fallbacks() {
    random_ebr_configurations(false, true);
    random_ebr_configurations(true, true);
}

#[test]
#[ignore = "requires clang"]
fn fast_mode_does_not_require_unused_controls_to_decide_hb09_scope() {
    let h = Harness::new([36, 9], |c| {
        // PDP port A has no read path; OCEA is not read by either executor.
        c.ports[0].output_enable = Input::Constant(X);
    });
    ecp5_emu::fast::FastSim::new(&h.k).unwrap();
}

fn random_ebr_configurations(wild: bool, same_clock: bool) {
    let mut r = Xorshift(0x2545_f491_4f6c_dd1d);
    let mut configs: Vec<[usize; 2]> = Vec::new();
    for a in [1, 2, 4, 9, 18] {
        for b in [1, 2, 4, 9, 18] {
            if !same_clock || a == b {
                configs.extend(std::iter::repeat_n([a, b], if same_clock { 3 } else { 1 }));
            }
        }
    }
    if !same_clock {
        for b in [1, 2, 4, 9, 18, 36] {
            configs.extend([[36, b], [36, b]]);
        }
    }
    let mut instants = 0;
    for (configuration, widths) in configs.into_iter().enumerate() {
        let pdp = widths[0] == 36;
        let modes = [0, 1].map(|side| {
            match if same_clock {
                configuration % 3
            } else {
                (r.next() % 3) as usize
            } {
                2 if same_clock || pdp || widths[side] >= 9 => WriteMode::ReadBeforeWrite,
                1 => WriteMode::WriteThrough,
                _ => WriteMode::Normal,
            }
        });
        let registered = [r.next().is_multiple_of(2), r.next().is_multiple_of(2)];
        let decode = [(r.next() % 8) as u8, (r.next() % 8) as u8];
        let asynchronous = r.next().is_multiple_of(2);
        let mut h = Harness::new(widths, |c| {
            if same_clock {
                c.ports[1].clock = c.ports[0].clock;
            }
            for side in 0..2 {
                c.ports[side].write_mode = modes[side];
                c.ports[side].output_register = registered[side];
                c.ports[side].decode = decode[side];
            }
            if asynchronous {
                c.reset_kind = ResetKind::Asynchronous;
            }
        });
        let label = format!("{widths:?} {modes:?} registered {registered:?} decode {decode:?} asynchronous {asynchronous}");
        // One instant of new input values; returns them for fast mode too.
        let draw =
            |r: &mut Xorshift, h: &Harness, clock: Option<usize>| -> Vec<(DriverId, Logic)> {
                let mut set = Vec::new();
                let mut put =
                    |name: String, v: bool| set.push((h.drivers[&name], Logic::from_bool(v)));
                let mut writing = false;
                let mut access = [false; 2];
                let mut write = [false; 2];
                for side in 0..2 {
                    access[side] = r.next() % 10 < 7;
                    write[side] = access[side] && (pdp && side == 0 || !pdp && r.next() % 10 < 4);
                    writing |= write[side];
                }
                for side in 0..2 {
                    let l = ['A', 'B'][side];
                    let ce = access[side] || r.next() % 10 < 3;
                    let cs = if access[side] {
                        decode[side] as u64
                    } else {
                        r.next() % 8
                    };
                    let access = ce && cs == decode[side] as u64;
                    put(format!("CE{l}"), ce);
                    for bit in 0..3 {
                        put(format!("CS{l}{bit}"), cs >> bit & 1 == 1);
                    }
                    let we = if pdp {
                        r.next().is_multiple_of(2)
                    } else {
                        write[side] || r.next() % 10 < 2
                    };
                    put(format!("WE{l}"), we);
                    let writes = access && (pdp && side == 0 || !pdp && we);
                    put(format!("RST{l}"), (wild || !writes) && r.next() % 100 < 4);
                    let oce = if wild {
                        r.next() % 10 < 8
                    } else if registered[side] {
                        access && r.next() % 10 < 8
                    } else {
                        true
                    };
                    put(format!("OCE{l}"), oce);
                    let shift = width_shift(widths[side]);
                    // Concentrate shared-clock traffic in one narrow row,
                    // including same-word reads/writes and double writes.
                    let address = (r.next() % if same_clock { 4 } else { 32 }) << shift;
                    let mask = if widths[side] >= 18 {
                        if (writes || we) && !wild {
                            (1 << (widths[side] / 9)) - 1
                        } else {
                            r.next() % 16
                        }
                    } else {
                        0
                    };
                    for bit in 0..14 {
                        put(format!("AD{l}{bit}"), (address | mask) >> bit & 1 == 1);
                    }
                    let data = r.next();
                    for bit in 0..18 {
                        put(format!("DI{l}{bit}"), data >> bit & 1 == 1);
                    }
                }
                put("GSR".into(), (wild || !writing) && r.next() % 100 < 2);
                put("GWDIS".into(), wild && r.next() % 100 < 5);
                let sides = match clock {
                    Some(2) => vec![0, 1],
                    Some(side) => vec![side],
                    None => Vec::new(),
                };
                for side in sides {
                    let name = format!("CLK{}", ['A', 'B'][side]);
                    let old = h.k.value(h.k.signal(&wire(&name)).unwrap()).unwrap() == One;
                    put(name, !old);
                }
                set
            };
        let apply = |h: &mut Harness, set: &[(DriverId, Logic)]| {
            h.time += 1;
            h.k.schedule_batch(Time::from_ps(h.time), set, None)
                .unwrap();
            h.k.run_until(Time::from_ps(h.time)).unwrap();
        };
        let mut warm = 0;
        let mut fast = loop {
            match ecp5_emu::fast::FastSim::new(&h.k) {
                Ok(fast) => break fast,
                Err(e)
                    if e.downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                        .is_some()
                        && warm < 400 =>
                {
                    warm += 1;
                    let set = draw(&mut r, &h, Some(warm % 2));
                    apply(&mut h, &set);
                }
                Err(e) => panic!("{label}: {e:#}"),
            }
        };
        for step in 0..3000 {
            let clock = match r.next() % 5 {
                _ if same_clock => Some(0),
                0 => Some(0),
                1 => Some(1),
                2 if wild => Some(2),
                _ => None,
            };
            let set = draw(&mut r, &h, clock);
            apply(&mut h, &set);
            let inputs: Vec<_> = set.iter().map(|&(d, v)| (d, v == One)).collect();
            fast.set_at(Time::from_ps(h.time), &inputs)
                .unwrap_or_else(|e| panic!("{label} step {step}: {e:#}"));
            for side in 0..2 {
                for (bit, &s) in h.outputs[side].iter().enumerate() {
                    assert_eq!(
                        fast.value(s),
                        Some(h.k.value(s).unwrap()),
                        "{label} step {step}: port {side} bit {bit}"
                    );
                }
            }
            instants += 1;
        }
        let memory = fast.memory(0).unwrap();
        for row in 0..2048 {
            let word = h.k.state().ebr_word(h.id, row).unwrap();
            for bit in 0..9 {
                let kernel = word[bit].bit().map_or(2, u8::from);
                assert_eq!(
                    memory[row * 9 + bit],
                    kernel,
                    "{label}: row {row} bit {bit}"
                );
            }
        }
        let unknown = (0..2)
            .flat_map(|side| h.output(side))
            .filter(|v| v.bit().is_none())
            .count();
        println!("EBR {label}: fast mode from instant {warm}; {unknown} outputs X at the end");
    }
    assert_eq!(instants, if same_clock { 15 } else { 37 } * 3000);
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
