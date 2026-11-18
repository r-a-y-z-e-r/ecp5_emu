//! HB09P raw PDP36, masked-X demand, and the exact inferred X03 RAM.
mod common;
mod ebr_fixture;
use common::{write, Fixture};
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        Builder, DriverId, Ebr, EbrMode, EbrPort, Input, Kernel, Options, ResetKind, SignalId,
        Strength, Time, WriteMode,
    },
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}

struct Ram {
    _fixture: Fixture,
    k: Kernel,
    drivers: HashMap<String, DriverId>,
    raw: Vec<SignalId>,
    q: SignalId,
    time: u64,
}

impl Ram {
    fn new(lut_mux: bool, edit: impl FnOnce(&mut Ebr)) -> Self {
        let f = Fixture::with_compiler();
        f.add_ebr("EBR0");
        let mut ports = fs::read_to_string(f.evidence.join("resource_ports.csv")).unwrap();
        for side in ['A', 'B'] {
            for bit in 0..18 {
                ports.push_str(&format!(
                    "LFE5U-85F/R1C1/EBR0,DO{side}{bit},out,1,1,DO{side}{bit}_EBR\n"
                ));
            }
        }
        write(&f.evidence.join("resource_ports.csv"), ports);
        f.seal();
        let image = ConfigurationImage::from_snapshot(
            b".device LFE5U-85F\n".to_vec(),
            SourceKind::TextConfig,
            &serde_json::to_vec(&json!({"schema_version":1,
            "image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,
            "encoding":"frame_byte_lsb0","cram_hex":"0000000000000000","bram_data":{},
            "origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,
            "native_metadata_hex":[]}))
            .unwrap(),
        )
        .unwrap();
        let catalog = Catalog::merge_documents(
            &[serde_json::to_vec(&json!({
            "device":"LFE5U-85F","resources":[]}))
            .unwrap()],
            "LFE5U-85F",
            4,
            16,
        )
        .unwrap();
        let device =
            Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalog).unwrap());
        let id = device.model().resource_id("LFE5U-85F/R1C1/EBR0").unwrap();
        let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        let lut = device
            .model()
            .nodes(plc)
            .find(|n| n.native_name == "SLICEA.K0")
            .unwrap()
            .id;
        let mut state = DeviceState::new(&device).unwrap();
        for row in 0..2048 {
            state.write_ebr_word(id, row, [Zero; 9]).unwrap();
        }
        // A=select, B=low, C=high: a mapped LUT mux as well as Function::Mux.
        for a in 0..16 {
            state
                .write_lut(
                    lut,
                    a,
                    Logic::from_bool((a >> if a & 1 == 1 { 2 } else { 1 }) & 1 == 1),
                )
                .unwrap();
        }
        let mut b = Builder::with_state(device, state).unwrap();
        let mut drivers = HashMap::new();
        let mut input = |name: String, value: Logic| {
            let s = b.signal(wire(&name)).unwrap();
            let d = b
                .external_driver(s, &name, Strength::Strong, value)
                .unwrap();
            drivers.insert(name, d);
            Input::from(s)
        };
        let clock = input("CLK".into(), Zero);
        let pick = input("pick".into(), Zero);
        let ports = std::array::from_fn(|side| EbrPort {
            width: 36,
            output_register: false,
            write_mode: WriteMode::Normal,
            clock,
            enable: input(format!("CE{side}"), One),
            output_enable: input(format!("OCE{side}"), if side == 0 { Zero } else { One }),
            write_enable: Input::Constant(if side == 0 { One } else { Zero }),
            select: [Input::Constant(Zero); 3],
            decode: 0,
            reset: input(format!("RST{side}"), Zero),
            address: std::array::from_fn(|bit| input(format!("AD{side}_{bit}"), Zero)),
            data: std::array::from_fn(|bit| input(format!("DI{side}_{bit}"), Zero)),
        });
        let mut config = Ebr {
            resource: id,
            mode: EbrMode::Pdp,
            ports,
            reset_kind: ResetKind::Asynchronous,
            asynchronous_release: true,
            global_reset: None,
            write_inhibit: input("GWDIS".into(), Zero),
        };
        edit(&mut config);
        b.clear_ebr_pipeline(id).unwrap();
        let outputs = b.ebr(config).unwrap();
        let raw: Vec<_> = outputs.into_iter().flatten().collect();
        let q = if lut_mux {
            b.lut4_with_inputs(
                lut,
                [
                    pick,
                    raw[0].into(),
                    Input::Constant(One),
                    Input::Constant(Zero),
                ],
            )
            .unwrap()
        } else {
            let q = b.signal(wire("q")).unwrap();
            b.mux("q mux", pick, raw[0].into(), Input::Constant(One), q)
                .unwrap();
            q
        };
        for &s in raw.iter().chain([&q]) {
            b.watch(s).unwrap();
        }
        Self {
            _fixture: f,
            k: b.build().unwrap(),
            drivers,
            raw,
            q,
            time: 0,
        }
    }

    fn set(&mut self, values: Vec<(String, Logic)>, fast: &mut Option<FastSim>) {
        self.time += 1;
        let updates: Vec<_> = values
            .into_iter()
            .map(|(n, v)| (self.drivers[&n], v))
            .collect();
        let time = Time::from_ps(self.time);
        self.k.schedule_batch(time, &updates, None).unwrap();
        self.k.run_until(time).unwrap();
        if let Some(f) = fast {
            let binary: Vec<_> = updates.iter().map(|&(d, v)| (d, v == One)).collect();
            f.set_at(time, &binary).unwrap();
            for &s in self.raw.iter().chain([&self.q]) {
                assert_eq!(
                    f.value(s),
                    Some(self.k.value(s).unwrap()),
                    "equal trace at {time}"
                );
            }
        }
    }

    fn write(&mut self, row: usize, data: u64, mask: usize, fast: &mut Option<FastSim>) {
        let mut values = vec![("CLK".into(), Zero)];
        for side in 0..2 {
            for bit in 0..14 {
                let addr = (row << 5) | if side == 0 { mask } else { 0 };
                values.push((
                    format!("AD{side}_{bit}"),
                    Logic::from_bool(addr & (1 << bit) != 0),
                ));
            }
            for bit in 0..18 {
                values.push((
                    format!("DI{side}_{bit}"),
                    Logic::from_bool(data & (1 << (18 * side + bit)) != 0),
                ));
            }
        }
        self.set(values, fast);
        self.set(vec![("CLK".into(), One)], fast);
    }
}

fn full_words(fast: bool) {
    let mut h = Ram::new(false, |_| {});
    let mut fast = fast.then(|| FastSim::new(&h.k).unwrap());
    for row in 0..512 {
        let word = ((row as u64 * 0x1234567) ^ 0x987654321) & ((1 << 36) - 1);
        h.write(row, word, 15, &mut fast);
        for (bit, &s) in h.raw.iter().enumerate() {
            assert_eq!(
                h.k.value(s).unwrap(),
                Logic::from_bool(word & (1 << bit) != 0),
                "row {row}, bit {bit}"
            );
        }
    }
    assert_eq!(h.k.statistics().ebr_collision_rows, 0);
}

#[test]
fn pdp36_new_word_in_every_physical_row() {
    full_words(false);
}

#[test]
#[ignore = "requires clang"]
fn fast_pdp36_new_word_equal_trace() {
    full_words(true);
}

#[test]
fn pdp_unmeasured_profiles_remain_conservative() {
    for variant in 0..8 {
        let mut h = Ram::new(false, |c| match variant {
            0 => c.reset_kind = ResetKind::Synchronous,
            1 => c.ports[0].write_mode = WriteMode::ReadBeforeWrite,
            2 => c.ports[0].output_enable = Input::Constant(One),
            3 => c.global_reset = Some(Input::Constant(Zero)),
            4 => c.ports[1].clock = c.ports[0].address[4],
            5 => c.ports[1].width = 18,
            6 => c.ports.iter_mut().for_each(|p| p.output_register = true),
            _ => {} // partial masks
        });
        h.write(0, 0x123456789, if variant == 7 { 3 } else { 15 }, &mut None);
        if variant == 4 {
            h.set(
                vec![("CLK".into(), Zero), ("AD0_4".into(), Zero)],
                &mut None,
            );
            h.set(vec![("CLK".into(), One), ("AD0_4".into(), One)], &mut None);
        }
        if variant == 6 {
            h.write(0, 0x123456789, 15, &mut None);
        }
        assert!(
            h.raw.iter().any(|&s| h.k.value(s).unwrap() == X),
            "variant {variant}"
        );
    }
}

fn masked_mux(use_fast: bool) {
    for lut in [false, true] {
        let mut h = Ram::new(lut, |_| {});
        h.set(vec![("pick".into(), One)], &mut None);
        let mut fast = use_fast.then(|| FastSim::new(&h.k).unwrap());
        // Deliberately outside HB09P: only two of the four lanes write.
        h.write(0, 1, 3, &mut fast);
        assert_eq!(h.k.value(h.raw[0]).unwrap(), X);
        assert_eq!(h.k.value(h.q).unwrap(), One);
        h.time += 1;
        let time = Time::from_ps(h.time);
        let updates = [(h.drivers["pick"], Zero)];
        h.k.schedule_batch(time, &updates, None).unwrap();
        h.k.run_until(time).unwrap();
        assert_eq!(h.k.value(h.q).unwrap(), X);
        if let Some(f) = &mut fast {
            assert!(
                f.set_at(time, &[(h.drivers["pick"], false)])
                    .unwrap_err()
                    .to_string()
                    .contains("output of EBR"),
                "selecting the X must still refuse without another EBR edge"
            );
        }
    }
}

#[test]
fn unselected_mux_collision_x_stays_known() {
    masked_mux(false);
}

#[test]
#[ignore = "ECP5_HELD_TESTS=1: pending fast hunk B (selected-input runtime demand); requires clang"]
fn fast_unselected_mux_collision_x_equal_trace_then_selected_x_refuses() {
    if std::env::var("ECP5_HELD_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping held test: set ECP5_HELD_TESTS=1; fast hunk B (selected-input runtime demand) is pending");
        return;
    }
    masked_mux(true);
}

fn native_device(config: &std::path::Path) -> Arc<ConfiguredDevice> {
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            Arc::new(ConfigurationImage::load(config, &db, None).unwrap()),
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    )
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb09ps_raw_outputs_match_all_board_bits() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let device =
        native_device(&root.join("docs/inventory/LFE5U-85F/hb09p_pdp_collision/hb09ps.config"));
    let state = DeviceState::new(&device).unwrap();
    let mut probes = Vec::new();
    for index in 0..8 {
        let seed = 0x4842303950u64.wrapping_add((index as u64).wrapping_mul(0x9E3779B97F4A7C15));
        let word = (seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
            >> 20)
            & if index < 4 {
                0xffff_ffff
            } else {
                (1 << 36) - 1
            };
        let id = device
            .model()
            .resources()
            .filter(|(_, r)| r.family == "EBR")
            .find(|(id, _)| {
                (0..36).all(|b| {
                    state.ebr_word(*id, b / 9).unwrap()[b % 9]
                        == Logic::from_bool(word & (1 << b) != 0)
                })
            })
            .unwrap()
            .0;
        let ports = device.model().resource_ports(id);
        for bit in 0..if index < 4 { 32 } else { 36 } {
            let name = format!("DO{}{}", if bit < 18 { 'A' } else { 'B' }, bit % 18);
            probes.push(Reference::Wire(
                ports.iter().find(|p| p.name == name).unwrap().wire.clone(),
            ));
        }
    }
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(Reference::Pin("G2".into()), Zero)],
            probes: probes.clone(),
            options: Options::default(),
        },
    )
    .unwrap();
    let log = include_str!("../docs/inventory/LFE5U-85F/hb09p_pdp_collision/observations_s.log");
    let windows: std::collections::HashSet<_> = log
        .split('\'')
        .filter(|s| s.len() == 17 + 64 * 68 && s.starts_with('R'))
        .collect();
    assert_eq!(windows.len(), 1);
    let line = windows.into_iter().next().unwrap();
    let mut checked = 0;
    for sample in 0..64 {
        let mut offset = 17 + sample * 68;
        let mut probe = 0;
        for index in 0..8 {
            let width = if index < 4 { 32 } else { 36 };
            let word = u64::from_str_radix(&line[offset..offset + width / 4], 16).unwrap();
            for bit in 0..width {
                assert_eq!(
                    c.value(&probes[probe]).unwrap(),
                    Logic::from_bool(word & (1 << bit) != 0),
                    "sample {sample}, block {index}, bit {bit}"
                );
                probe += 1;
                checked += 1;
            }
            offset += width / 4;
        }
        let clock = c.drivers["pin:G2"];
        c.kernel
            .schedule(Time::from_ps((sample * 40_000 + 20_000) as u64), clock, One)
            .unwrap();
        c.kernel
            .schedule(Time::from_ps(((sample + 1) * 40_000) as u64), clock, Zero)
            .unwrap();
        c.kernel
            .run_until(Time::from_ps(((sample + 1) * 40_000) as u64))
            .unwrap();
    }
    assert_eq!(checked, 17_408);
    assert_eq!(c.kernel.statistics().ebr_collision_rows, 0);
}

fn x03() -> (Compiled, Names) {
    static BUILD: OnceLock<PathBuf> = OnceLock::new();
    let dir = BUILD.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/ebr-r02-x03");
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("top.v"), "module top(input clk,we,input [8:0] a,input [31:0] d,output reg [31:0] q); reg [31:0] m[0:511]; integer i; initial for(i=0;i<512;i=i+1)m[i]=0; always @(posedge clk) begin if(we)m[a]<=d; q<=m[a]; end endmodule\n");
        ebr_fixture::run(&dir, "yosys", &["-p", "read_verilog top.v; synth_ecp5 -top top -json top.json"], "yosys.log");
        let db = Database::discover(None).unwrap();
        let model = DeviceModel::load(&db, &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F")).unwrap();
        let mut balls = model.package_pins().filter(|p| p.package == "CABGA381" && p.pio.is_some() && p.ball != "G2");
        let mut lpf = "LOCATE COMP \"clk\" SITE \"G2\";\n".to_string();
        for (name, width) in [("we",1),("a",9),("d",32),("q",32)] {
            for bit in 0..width {
                let name = if width == 1 { name.into() } else { format!("{name}[{bit}]") };
                lpf.push_str(&format!("LOCATE COMP \"{name}\" SITE \"{}\";\nIOBUF PORT \"{name}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n", balls.next().unwrap().ball));
            }
        }
        write(&dir.join("top.lpf"), lpf);
        ebr_fixture::run(&dir, "nextpnr-ecp5", &["--85k","--package","CABGA381","--json","top.json","--lpf","top.lpf","--textcfg","top.config","--write","top_routed.json","--seed","1"], "nextpnr.log");
        dir
    });
    let device = native_device(&dir.join("top.config"));
    let names = Names::load(&dir.join("top_routed.json"), device.model(), "CABGA381").unwrap();
    let mut drives = vec![
        (names.expand("clk").unwrap()[0].1.clone(), Zero),
        (names.expand("we").unwrap()[0].1.clone(), One),
    ];
    drives.extend(
        names
            .expand("a")
            .unwrap()
            .into_iter()
            .chain(names.expand("d").unwrap())
            .map(|(_, r)| (r, Zero)),
    );
    let probes = names
        .expand("q")
        .unwrap()
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    let c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives,
            probes,
            options: Options::default(),
        },
    )
    .unwrap();
    (c, names)
}

fn x03_full_words(use_fast: bool) {
    let (mut c, names) = x03();
    assert_eq!(c.report.ebr[0].widths, [36, 36]);
    let mut fast = use_fast.then(|| FastSim::new(&c.kernel).unwrap());
    let clock = c.drivers[&names.expand("clk").unwrap()[0].1.label()];
    let data = names.expand("d").unwrap();
    let addr = names.expand("a").unwrap();
    let q = names.expand("q").unwrap();
    let mut cycle = 0;
    for row in 0..512u64 {
        let word = ((row.wrapping_mul(0x1234567)) ^ 1) & 0xffff_ffff;
        for n in 0..4 {
            // Change the current D on the third edge: q must still expose
            // the previous logical write, while the raw EBR returns its new write.
            let current = if n == 2 { !word & 0xffff_ffff } else { word };
            let inputs: Vec<_> = addr
                .iter()
                .enumerate()
                .map(|(b, (_, r))| (c.drivers[&r.label()], Logic::from_bool(row & (1 << b) != 0)))
                .chain(data.iter().enumerate().map(|(b, (_, r))| {
                    (
                        c.drivers[&r.label()],
                        Logic::from_bool(current & (1 << b) != 0),
                    )
                }))
                .collect();
            c.kernel
                .schedule_batch(Time::from_ps(cycle * 40_000 + 1), &inputs, None)
                .unwrap();
            c.kernel
                .schedule(Time::from_ps(cycle * 40_000 + 20_000), clock, One)
                .unwrap();
            c.kernel
                .schedule(Time::from_ps((cycle + 1) * 40_000), clock, Zero)
                .unwrap();
            c.kernel
                .run_until(Time::from_ps((cycle + 1) * 40_000))
                .unwrap();
            if let Some(f) = &mut fast {
                let binary: Vec<_> = inputs.iter().map(|&(d, v)| (d, v == One)).collect();
                for (time, updates) in [
                    (Time::from_ps(cycle * 40_000 + 1), binary),
                    (Time::from_ps(cycle * 40_000 + 20_000), vec![(clock, true)]),
                    (Time::from_ps((cycle + 1) * 40_000), vec![(clock, false)]),
                ] {
                    f.set_at(time, &updates).unwrap_or_else(|e| {
                        panic!("X03 row {row}, cycle {n}, input update at {time}: {e:#}")
                    });
                }
            }
            let expected = match n {
                0 => 0,
                1 | 2 => word,
                _ => !word & 0xffff_ffff,
            };
            for (bit, (_, r)) in q.iter().enumerate() {
                assert_eq!(
                    c.value(r).unwrap(),
                    Logic::from_bool(expected & (1 << bit) != 0),
                    "row {row}, cycle {n}, bit {bit}"
                );
                if let Some(f) = &fast {
                    assert_eq!(
                        f.value(c.probes[&r.label()]),
                        Some(Logic::from_bool(expected & (1 << bit) != 0)),
                        "fast row {row}, cycle {n}, bit {bit}"
                    );
                }
            }
            cycle += 1;
        }
    }
    assert_eq!(c.kernel.statistics().ebr_collision_rows, 0);
    if use_fast {
        println!("R02 X03: {cycle} identical q rows (32 bits) across 512 addresses");
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn x03_inferred_q_is_known_new_word_in_every_row() {
    x03_full_words(false);
}

#[test]
#[ignore = "requires clang, Yosys, nextpnr, the native image reader and a local Trellis database"]
fn fast_x03_inferred_q_is_known_new_word_in_every_row() {
    x03_full_words(true);
}
