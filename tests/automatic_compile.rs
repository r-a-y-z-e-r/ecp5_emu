mod common;
#[path = "common/tools.rs"]
mod tools;
use common::{write, Fixture};
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{Builder, Input, Options, Strength, Time},
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use serde_json::json;
use std::{fs, path::PathBuf, process::Command, sync::Arc};

fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}
fn w(name: &str) -> Reference {
    Reference::Wire(wire(name))
}
fn pin() -> Reference {
    Reference::Pin("G1".into())
}
fn request(probes: Vec<Reference>, drives: Vec<(Reference, Logic)>) -> Request {
    Request {
        package: "CABGA381".into(),
        scope: Scope::Observed,
        startup: Startup::ReadyUnknown,
        drives,
        probes,
        options: Options::default(),
    }
}
fn synthetic(f: &Fixture, data: [u8; 8], pio: bool) -> Arc<ConfiguredDevice> {
    let resources = if pio {
        let fields=[
            ("BASE_TYPE",json!({"INPUT_LVCMOS33":[[0,4,false]],"BIDIR_LVCMOS33":[[0,4,true]]})),
            ("OPENDRAIN",json!({"OFF":[[0,5,false]],"ON":[[0,5,true]]})),
            ("PULLMODE",json!({"NONE":[[0,6,false],[0,7,false]],"UP":[[0,6,true],[0,7,false]],"DOWN":[[0,6,false],[0,7,true]],"KEEPER":[[0,6,true],[0,7,true]]})),
            ("DATAMUX_OREG",json!({"PADDO":[],"IOLDO":[[1,4,true]]})),
            ("TRIMUX_TSREG",json!({"PADDT":[],"IOLTO":[[1,5,true]]})),
        ].into_iter().map(|(name,options)|json!({"kind":"enum","name":format!("PIOA.{name}"),"fragments":[{"tile":"CIB_R1C1:CIB","source":format!("fixture:{name}"),"default":null,"options":options}]})).collect::<Vec<_>>();
        json!([{"id":"LFE5U-85F/R1C1/PIOA","family":"PIO","fields":fields}])
    } else {
        json!([])
    };
    let doc = serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":resources})).unwrap();
    let catalog = Catalog::merge_documents(&[doc], "LFE5U-85F", 4, 16).unwrap();
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,
        "encoding":"frame_byte_lsb0","cram_hex":data.iter().map(|v|format!("{v:02x}")).collect::<String>(),
        "bram_data":{},"origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalog).unwrap())
}
fn local_bit(data: &mut [u8; 8], frame: usize, bit: usize) {
    let global_bit = 3 + bit;
    data[(frame + 2) * 2 + global_bit / 8] |= 1 << (global_bit % 8);
}

#[test]
fn cib_clock_lsr_constant_labels_are_qualified_before_consumer_inversion() {
    for name in ["JCLK0", "JCLK1", "JLSR0", "JLSR1"] {
        let f = Fixture::with_compiler();
        let path = f.db.join("ECP5/tiledata/CIB/bits.db");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(&format!("\n.config_enum CIB.{name}MUX {name}\n0 F0B0\n{name} -\n\n.config_enum CIB.JA4MUX JA4\n0 F0B1\n1 F0B2\nJA4 -\n"));
        write(&path, text);
        f.refresh_database();
        let mut data = [0; 8];
        data[0] = 0x30; // CIB local bits 0/1, at global bits 4/5
        let device = synthetic(&f, data, false);
        let c = compile::compile(device, request(vec![w(name), w("JA4")], vec![])).unwrap();
        assert_eq!(c.value(&w(name)).unwrap(), Logic::One);
        assert_eq!(c.value(&w("JA4")).unwrap(), Logic::Zero);
        assert!(c.report.constants.iter().any(|v| v.encoded_value == "0"
            && v.value == Logic::One
            && v.field == format!("CIB.{name}MUX")));
        let bad = fs::read_to_string(&path).unwrap().replace(
            &format!("0 F0B0\n{name} -"),
            &format!("0 F0B0\n1 F1B0\n{name} -"),
        );
        write(&path, bad);
        f.refresh_database();
        assert!(
            compile::compile(synthetic(&f, data, false), request(vec![w(name)], vec![])).is_err()
        );
    }
}

#[test]
fn config_selects_lut_ff_routes_and_async_reset_without_a_handwritten_plan() {
    let f = Fixture::with_compiler();
    let device = synthetic(&f, [0; 8], false);
    let before = device.image().decoded_sha256().to_string();
    let mut c = compile::compile(
        Arc::clone(&device),
        request(
            vec![w("Q0_SLICE")],
            vec![(w("CLK0"), Logic::Zero), (w("LSR0"), Logic::One)],
        ),
    )
    .unwrap();
    assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::Zero);
    assert_eq!(c.report.counts.lut4, 1);
    assert_eq!(c.report.counts.flip_flops, 1);
    assert!(c
        .report
        .routes
        .iter()
        .any(|r| r.source == "F0_SLICE" && r.sink == "F0"));
    assert!(c
        .report
        .fields
        .iter()
        .any(|f| f.name == "SLICEA.REG0.SD" && f.candidates == ["1"]));
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["R1C1:LSR0"], Logic::Zero)
        .unwrap();
    c.kernel
        .schedule(Time::from_ps(2), c.drivers["R1C1:CLK0"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::One);
    assert_eq!(device.image().decoded_sha256(), before);
}

#[test]
fn repeated_compile_preserves_state_and_keeps_cached_handles_independent() {
    let f = Fixture::with_compiler();
    let device = synthetic(&f, [0; 8], false);
    let mut request = request(
        vec![w("Q0_SLICE")],
        vec![(w("CLK0"), Logic::Zero), (w("LSR0"), Logic::One)],
    );
    request.startup = Startup::Configured;
    request.options.trace_capacity = 0;
    let mut original = compile::compile(Arc::clone(&device), request.clone()).unwrap();
    let mut reopened = compile::compile(device, request).unwrap();
    assert_eq!(
        serde_json::to_vec(&original.report).unwrap(),
        serde_json::to_vec(&reopened.report).unwrap()
    );
    assert!(reopened
        .kernel
        .value(original.probes["R1C1:Q0_SLICE"])
        .is_err());
    assert!(reopened
        .kernel
        .schedule(Time::from_ps(1), original.drivers["R1C1:CLK0"], Logic::One)
        .is_err());
    for c in [&mut original, &mut reopened] {
        c.kernel
            .schedule(Time::from_ps(1), c.drivers["R1C1:LSR0"], Logic::Zero)
            .unwrap();
        for edge in 2..=9 {
            c.kernel
                .schedule(
                    Time::from_ps(edge),
                    c.drivers["R1C1:CLK0"],
                    Logic::from_bool(edge % 2 == 0),
                )
                .unwrap();
            c.kernel.run_until(Time::from_ps(edge)).unwrap();
            assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::One);
        }
    }
    assert_eq!(
        serde_json::to_vec(&original.kernel.statistics()).unwrap(),
        serde_json::to_vec(&reopened.kernel.statistics()).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&original.kernel.state().summary()).unwrap(),
        serde_json::to_vec(&reopened.kernel.state().summary()).unwrap()
    );
}

#[test]
fn sd_zero_reads_m_without_executing_the_unused_di_lut_path() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    local_bit(&mut data, 0, 3);
    let mut c = compile::compile(
        synthetic(&f, data, false),
        request(
            vec![w("Q0_SLICE")],
            vec![
                (w("CLK0"), Logic::Zero),
                (w("LSR0"), Logic::One),
                (w("M0_SLICE"), Logic::One),
            ],
        ),
    )
    .unwrap();
    assert_eq!(c.report.counts.lut4, 0);
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["R1C1:LSR0"], Logic::Zero)
        .unwrap();
    c.kernel
        .schedule(Time::from_ps(2), c.drivers["R1C1:CLK0"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::One);
}

#[test]
fn logic_output_mux_can_select_the_wide_lut_path_with_shared_input_controls() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    local_bit(&mut data, 1, 1); // Fn selects F5 in LOGIC, not an independent INJECT.
    local_bit(&mut data, 1, 6); // Four tied-high LUT inputs select known INIT[15]=1.
    let mut c = compile::compile(
        synthetic(&f, data, false),
        request(
            vec![w("F0")],
            vec![(w("M0_SLICE"), Logic::Zero), (w("F1_SLICE"), Logic::Zero)],
        ),
    )
    .unwrap();
    assert_eq!(c.report.counts.wide_muxes, 1);
    assert_eq!(c.value(&w("F0")).unwrap(), Logic::One);
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["R1C1:M0_SLICE"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(c.value(&w("F0")).unwrap(), Logic::Zero);
}

#[test]
fn second_wide_mux_uses_native_coarse_fxa_fxb_and_shared_m1() {
    let f = Fixture::with_compiler();
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n.config SLICEA.K1.INIT 1111111111111111\n");
    for frame in 0..2 {
        for bit in 0..8 {
            text.push_str(&format!("!F{frame}B{bit}\n"));
        }
    }
    write(&path, text);
    let path = f.evidence.join("plc_research/node_templates.csv");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("split,SLICEA.K1,TRELLIS_COMB,4,split_view\n");
    write(&path, text);
    let path = f.evidence.join("plc_research/native_ports.csv");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("split,SLICEA.K1,TRELLIS_COMB,4,OFX,out,0,0,FXA_SLICE\nsplit,SLICEA.K1,TRELLIS_COMB,4,M,in,0,0,M1_SLICE\ncoarse,SLICEA,SLICE,0,FXA,in,0,0,FXAA_SLICE\ncoarse,SLICEA,SLICE,0,FXB,in,0,0,FXBA_SLICE\n");
    write(&path, text);
    f.refresh_database();
    let mut c = compile::compile(
        synthetic(&f, [0; 8], false),
        request(
            vec![w("FXA_SLICE")],
            vec![
                (w("M1_SLICE"), Logic::Zero),
                (w("FXAA_SLICE"), Logic::Zero),
                (w("FXBA_SLICE"), Logic::One),
            ],
        ),
    )
    .unwrap();
    assert_eq!(c.value(&w("FXA_SLICE")).unwrap(), Logic::Zero);
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["R1C1:M1_SLICE"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(c.value(&w("FXA_SLICE")).unwrap(), Logic::One);
    assert_eq!(c.report.counts.lut4, 0);
}

#[test]
fn enum_supersets_select_disable_but_incomparable_modes_remain_errors() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    local_bit(&mut data, 1, 1);
    local_bit(&mut data, 1, 2);
    let device = synthetic(&f, data, false);
    let tile = device.model().tile_id("R1C1:PLC2").unwrap();
    assert_eq!(device.enum_candidates(tile, "SLICEA.CEMUX").unwrap(), ["0"]);
    local_bit(&mut data, 0, 0);
    local_bit(&mut data, 0, 1);
    let error = compile::compile(
        synthetic(&f, data, false),
        request(vec![w("F0_SLICE")], vec![]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("ambiguous/unmatched"));
}

#[test]
fn selected_outside_density_routes_and_unqualified_fixed_overlap_are_rejected() {
    let f = Fixture::with_compiler();
    let error = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![w("DST")], vec![]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("leaves the device"));
    let path = f.db.join("ECP5/tiledata/CIB/bits.db");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n.fixed_conn CONFLICT A\n.fixed_conn CONFLICT B\n");
    write(&path, text);
    f.refresh_database();
    let error = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![w("CONFLICT")], vec![]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("multiple fixed sources"));
}

#[test]
fn basic_bidirectional_pio_keeps_pad_output_and_readback_drivers_separate() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    data[0] = 1 << 4;
    let mut c = compile::compile(
        synthetic(&f, data, true),
        request(
            vec![pin(), w("JPADDIA_PIO")],
            vec![
                (pin(), Logic::Z),
                (w("PADDOA_PIO"), Logic::One),
                (w("PADDTA_PIO"), Logic::Zero),
            ],
        ),
    )
    .unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::One);
    assert_eq!(c.value(&w("JPADDIA_PIO")).unwrap(), Logic::One);
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["pin:G1"], Logic::Zero)
        .unwrap();
    c.kernel.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::X);
    assert_eq!(c.value(&w("JPADDIA_PIO")).unwrap(), Logic::X);
    c.kernel
        .schedule(Time::from_ps(2), c.drivers["R1C1:PADDTA_PIO"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::Zero);
}

#[test]
fn open_drain_release_and_input_pull_use_the_normal_resolution_engine() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    data[0] = (1 << 4) | (1 << 5) | (1 << 6);
    let mut c = compile::compile(
        synthetic(&f, data, true),
        request(
            vec![pin(), w("JPADDIA_PIO")],
            vec![
                (pin(), Logic::Z),
                (w("PADDOA_PIO"), Logic::One),
                (w("PADDTA_PIO"), Logic::Zero),
            ],
        ),
    )
    .unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::One); // pullup while open drain releases
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["R1C1:PADDOA_PIO"], Logic::Zero)
        .unwrap();
    c.kernel.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::Zero);
    c.kernel
        .schedule(Time::from_ps(2), c.drivers["pin:G1"], Logic::One)
        .unwrap();
    c.kernel.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(c.value(&pin()).unwrap(), Logic::X);
}

#[test]
fn preload_executes_and_unsupported_pio_paths_remain_refused() {
    let f = Fixture::with_compiler();
    let mut data = [0; 8];
    local_bit(&mut data, 0, 6);
    let c = compile::compile(
        synthetic(&f, data, false),
        request(
            vec![w("Q0_SLICE")],
            vec![
                (w("M0_SLICE"), Logic::One),
                (w("CLK0"), Logic::Zero),
                (w("LSR0"), Logic::One),
            ],
        ),
    )
    .unwrap();
    assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::One);
    let mut data = [0; 8];
    data[0] = 1 << 4;
    data[2] = 1 << 4;
    let error = compile::compile(synthetic(&f, data, true), request(vec![pin()], vec![]))
        .err()
        .unwrap();
    // The IOLOGIC path is selected; the synthetic PIO has no IOLDO port.
    assert!(format!("{error:#}").contains("output path IOLDO"));
    let error = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![w("ECLKO")], vec![]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("ECLKSYNC bypass"));
}

#[test]
fn carry_half_implements_integer_addition_and_inject_still_propagates_carry() {
    let f = Fixture::with_compiler();
    let device = synthetic(&f, [0; 8], false);
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let node = device
        .model()
        .nodes(plc)
        .find(|n| n.native_name == "SLICEA.K0")
        .unwrap()
        .id;
    for inject in [false, true] {
        let mut state = DeviceState::new(&device).unwrap();
        for bit in 0..16 {
            state
                .write_lut(node, bit, Logic::from_bool(0x6008 & (1 << bit) != 0))
                .unwrap();
        }
        let mut b = Builder::with_state(Arc::clone(&device), state).unwrap();
        let mut signals = Vec::new();
        let mut drivers = Vec::new();
        for name in ["A", "B", "CIN"] {
            let s = b.signal(wire(name)).unwrap();
            signals.push(s);
            drivers.push(
                b.external_driver(s, name, Strength::Strong, Logic::Zero)
                    .unwrap(),
            );
        }
        let (sum, cout) = b
            .carry(
                node,
                [
                    signals[0].into(),
                    signals[1].into(),
                    Input::Constant(Logic::One),
                    Input::Constant(Logic::One),
                ],
                signals[2].into(),
                inject,
            )
            .unwrap();
        let mut k = b.build().unwrap();
        for value in 0..8_u64 {
            let updates = drivers
                .iter()
                .enumerate()
                .map(|(i, &d)| (d, Logic::from_bool(value & (1 << i) != 0)))
                .collect::<Vec<_>>();
            k.schedule_batch(Time::from_ps(value + 1), &updates, None)
                .unwrap();
            k.run_until(Time::from_ps(value + 1)).unwrap();
            let (a, b, cin) = (value & 1, (value >> 1) & 1, (value >> 2) & 1);
            let result = if inject {
                (a ^ b) + 2 * ((a ^ b) & cin)
            } else {
                a + b + cin
            };
            assert_eq!(k.value(sum).unwrap(), Logic::from_bool(result & 1 != 0));
            assert_eq!(k.value(cout).unwrap(), Logic::from_bool(result & 2 != 0));
        }
    }
}

#[test]
fn clock_segment_identity_requires_real_directed_spine_and_tap_connections() {
    let f = Fixture::with_compiler();
    let clock = w("G_HPBX0000");
    let c = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![clock.clone()], vec![]),
    )
    .unwrap();
    assert_eq!(c.value(&clock).unwrap(), Logic::Z);
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n.fixed_conn R_HPBX0000 G_VPTX0000\n.fixed_conn G_VPTX0000 G_HPRX0000\n.fixed_conn G_ULPCLK0 SOURCE\n");
    write(&path, text);
    f.refresh_database();
    let c = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![clock.clone()], vec![(w("SOURCE"), Logic::One)]),
    )
    .unwrap();
    assert_eq!(c.value(&clock).unwrap(), Logic::One);
    assert_eq!(c.report.counts.routes, 3);
    assert_eq!(c.report.clock_aliases.len(), 3);
    assert!(c.report.undriven.is_empty());
}

#[test]
fn ready_profile_records_inactive_test_access_instead_of_joining_it_to_the_data_path() {
    let f = Fixture::with_compiler();
    let path = f.db.join("ECP5/tiledata/CIB/bits.db");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n.fixed_conn FEED JF0_CIBTEST\n.fixed_conn FEED DATA\n");
    write(&path, text);
    f.refresh_database();
    let c = compile::compile(
        synthetic(&f, [0; 8], false),
        request(vec![w("FEED")], vec![(w("DATA"), Logic::One)]),
    )
    .unwrap();
    assert_eq!(c.value(&w("FEED")).unwrap(), Logic::One);
    assert_eq!(c.report.excluded_routes.len(), 1);
    assert!(c.report.excluded_routes[0]
        .rule
        .contains("not hardware-validated"));
    assert!(c
        .kernel
        .device()
        .model()
        .site_references()
        .iter()
        .any(|s| s.name == "CIBTEST"));
}

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and the native image reader"]
fn native_two_clock_carry_counters_run_from_config_and_bit_through_gpio() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/automatic-compile");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("top.v"),
        r#"module top(input clk_a, clk_b, reset, output reg [7:0] a, output reg [7:0] b);
always @(posedge clk_a or posedge reset) if (reset) a <= 0; else a <= a + 1'b1;
always @(negedge clk_b or posedge reset) if (reset) b <= 0; else b <= b + 8'd3;
endmodule
"#,
    )
    .unwrap();
    let outputs = [
        "F1", "G1", "H1", "J1", "J3", "J5", "K1", "K2", "K3", "K4", "K5", "L1", "L2", "L3", "L4",
        "L5",
    ];
    let mut lpf = String::new();
    for (port, ball) in [("clk_a", "G2"), ("clk_b", "J4"), ("reset", "H2")]
        .into_iter()
        .map(|(p, b)| (p.into(), b))
        .chain(outputs.iter().enumerate().map(|(i, &ball)| {
            (
                format!("{}[{}]", if i < 8 { "a" } else { "b" }, i % 8),
                ball,
            )
        }))
    {
        lpf.push_str(&format!(
            "LOCATE COMP \"{port}\" SITE \"{ball}\";\nIOBUF PORT \"{port}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n"
        ));
    }
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    for (program, args) in [
        (
            "yosys",
            vec![
                "-p",
                "read_verilog top.v; synth_ecp5 -top top -json top.json",
            ],
        ),
        (
            "nextpnr-ecp5",
            vec![
                "--85k",
                "--package",
                "CABGA381",
                "--json",
                "top.json",
                "--lpf",
                "top.lpf",
                "--textcfg",
                "top.config",
                "--seed",
                "1",
                "--timing-allow-fail",
            ],
        ),
        ("ecppack", vec!["--compress", "top.config", "top.bit"]),
    ] {
        let result = tools::command(program)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        fs::write(
            dir.join(format!("{}.log", program.rsplit('/').next().unwrap())),
            [result.stdout, result.stderr].concat(),
        )
        .unwrap();
        assert!(
            result.status.success(),
            "{program} failed; see logs in {}",
            dir.display()
        );
    }
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let mut results = Vec::new();
    let mut reports = Vec::new();
    for (extension, scope) in [
        ("config", Scope::Observed),
        ("bit", Scope::Observed),
        ("bit", Scope::AllPlcRegisters),
    ] {
        let image = Arc::new(
            ConfigurationImage::load(&dir.join(format!("top.{extension}")), &db, None).unwrap(),
        );
        let configured = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let reference = |ball: &str| Reference::Pin(ball.into());
        let request = Request {
            package: "CABGA381".into(),
            scope,
            startup: Startup::ReadyUnknown,
            drives: vec![
                (reference("G2"), Logic::Zero),
                (reference("J4"), Logic::One),
                (reference("H2"), Logic::One),
            ],
            probes: outputs.iter().map(|s| reference(s)).collect(),
            options: Options::default(),
        };
        let mut compiled =
            compile::compile(configured, request).unwrap_or_else(|e| panic!("{extension}: {e:#}"));
        if matches!(scope, Scope::Observed) {
            fs::write(
                dir.join(format!("plan-{extension}.json")),
                serde_json::to_vec_pretty(&compiled.report).unwrap(),
            )
            .unwrap();
        }
        assert!(compiled.report.counts.carry_halves > 0);
        assert_eq!(
            compiled.report.counts.flip_flops,
            if matches!(scope, Scope::Observed) {
                16
            } else {
                83640
            }
        );
        assert_eq!(compiled.report.counts.pio, 19);
        let get = |compiled: &compile::Compiled| {
            let mut values = [0_u8; 2];
            for (i, ball) in outputs.iter().enumerate() {
                let value = compiled.value(&reference(ball)).unwrap();
                let value = value
                    .bit()
                    .unwrap_or_else(|| panic!("{extension} {ball}: {value}"));
                values[i / 8] |= u8::from(value) << (i % 8);
            }
            values
        };
        assert_eq!(get(&compiled), [0, 0]);
        let reset = compiled.drivers["pin:H2"];
        let a = compiled.drivers["pin:G2"];
        let b = compiled.drivers["pin:J4"];
        compiled
            .kernel
            .schedule(Time::from_ps(1), reset, Logic::Zero)
            .unwrap();
        let (mut ca, mut cb) = (false, true);
        let mut expected = [0_u8; 2];
        let mut samples = Vec::new();
        for time in 1..=2000 {
            let mut updates = Vec::new();
            if time % 2 == 0 {
                ca = !ca;
                updates.push((a, Logic::from_bool(ca)));
                if ca {
                    expected[0] = expected[0].wrapping_add(1);
                }
            }
            if time % 3 == 0 {
                cb = !cb;
                updates.push((b, Logic::from_bool(cb)));
                if !cb {
                    expected[1] = expected[1].wrapping_add(3);
                }
            }
            if !updates.is_empty() {
                compiled
                    .kernel
                    .schedule_batch(Time::from_ps(time), &updates, None)
                    .unwrap();
            }
            compiled.kernel.run_until(Time::from_ps(time)).unwrap();
            assert_eq!(get(&compiled), expected, "{extension}, time {time}");
            samples.push(expected);
        }
        results.push(samples);
        let transitions = compiled.kernel.drain_trace();
        reports.push(json!({"source_kind":extension,"scope":scope,"image":compiled.kernel.device().image().summary(),
            "plan_counts":compiled.report.counts,"field_uses":compiled.report.fields.len(),"route_uses":compiled.report.routes.len(),
            "excluded_test_routes":compiled.report.excluded_routes.len(),"clock_aliases":compiled.report.clock_aliases.len(),
            "undriven_wires":compiled.report.undriven.len(),"timestamps_checked":2000,"final_counter_values":expected,
            "state":compiled.kernel.state().summary(),"statistics":compiled.kernel.statistics(),
            "retained_transitions":transitions.len(),"initial_trace":transitions.into_iter().take(80).collect::<Vec<_>>() }));
        fs::write(
            dir.join(format!("state-{extension}.json")),
            serde_json::to_vec_pretty(&compiled.kernel.state().summary()).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0], results[2]);
    fs::write(
        dir.join("validation.json"),
        serde_json::to_vec_pretty(&json!({"schema_version":1,
        "scope":"software two-clock carry/GPIO execution; ready profile, no hardware validation",
        "variants":reports}))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("cli-events.json"),
        r#"[{"time":1,"set":{"pin:H2":"0"}},{"time":0,"set":{"pin:H2":"1"}}]"#,
    )
    .unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .current_dir(&dir)
        .args([
            "emulate",
            "top.bit",
            "--startup",
            "ready-unknown",
            "--scope",
            "observed",
            "--clock",
            "pin:G2",
            "--cycles",
            "4",
            "--set",
            "pin:J4=1",
            "--probe",
            "pin:H1",
            "--stimulus",
            "cli-events.json",
            "--output",
            "cli-result.json",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("cli-result.json")).unwrap()).unwrap();
    assert_eq!(report["observations"].as_array().unwrap().len(), 9);
    assert_eq!(report["observations"][8]["values"]["pin:H1"], "one");
    // This hand-edited SYNC/ACTIVE_HIGH profile is outside HY01. Ordinary
    // ACTIVE_LOW SGSR execution is covered by sgsr_native, including X20.
    let gsr = dir.join("unmeasured-sgsr-polarity.config");
    fs::write(
        &gsr,
        format!(
            "{}\n.tile MIB_R95C4:EFB0_PICB0\nenum: GSR.GSRMODE ACTIVE_HIGH\n.tile MIB_R46C68:VIQ_BUF\nenum: GSR.SYNCMODE SYNC\n",
            fs::read_to_string(dir.join("top.config")).unwrap()
        ),
    )
    .unwrap();
    let image = Arc::new(ConfigurationImage::load(&gsr, &db, None).unwrap());
    let configured = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let error = compile::compile(
        configured,
        request(vec![Reference::Pin("F1".into())], vec![]),
    )
    .err()
    .unwrap();
    let error = format!("{error:#}");
    assert!(
        error.contains("HY01-measured ACTIVE_LOW polarity"),
        "{error}"
    );
    assert!(error.contains("got ACTIVE_HIGH"), "{error}");
    // The default PULLMODE overlays a BASE_TYPE bit, so independent matching
    // reads NONE. The joint writer inversion (decode/pio.rs) recovers the
    // output without trusting the original text declaration.
    let overlay = dir.join("unsupported-pio-overlay.config");
    let text = fs::read_to_string(dir.join("top.config")).unwrap();
    let text = text
        .lines()
        .filter(|line| *line != "enum: PIOB.PULLMODE NONE")
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&overlay, text).unwrap();
    let image = Arc::new(ConfigurationImage::load(&overlay, &db, None).unwrap());
    let configured = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let pio = model.resource_id("LFE5U-85F/R47C0/PIOB").unwrap();
    let settings = configured.resource_settings(pio).unwrap();
    let base = settings
        .fields
        .iter()
        .find(|f| f.name == "PIOB.BASE_TYPE")
        .unwrap();
    let pull = settings
        .fields
        .iter()
        .find(|f| f.name == "PIOB.PULLMODE")
        .unwrap();
    assert_eq!(base.candidates, ["NONE"]);
    assert_eq!(pull.candidates, ["DOWN"]);
    let modes = configured.pio_modes(pio).unwrap();
    let mode = &modes["LFE5U-85F/R47C0/PIOB"];
    assert_eq!(mode.direction.as_deref(), Some("OUTPUT"));
    assert!(mode.base_types.contains(&"OUTPUT_LVCMOS33".to_string()));
    assert_eq!(mode.overlays["PULLMODE"], ["DOWN"]);
    fs::write(
        dir.join("pio-overlay-evidence.json"),
        serde_json::to_vec_pretty(&json!({
        "scope":"native software writer/decoder overlap, resolved by the joint writer inversion (docs/PIO_BANKS.md); board output behaviour observed in HS01",
        "resource":settings.id,"base_declaration":"OUTPUT_LVCMOS33","decoded_base":base,
        "decoded_pull":pull,"joint_mode":mode,"questions":["P01","P04"]}))
        .unwrap(),
    )
    .unwrap();
    let compiled = compile::compile(
        configured,
        request(vec![Reference::Pin("F1".into())], vec![]),
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        compiled.report.pio_modes["LFE5U-85F/R47C0/PIOB"]
            .direction
            .as_deref(),
        Some("OUTPUT")
    );
}

#[test]
fn configured_preload_does_not_seed_a_feedback_m_from_regset() {
    let f = Fixture::with_compiler();
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    let mut bits = fs::read_to_string(&path).unwrap();
    bits.push_str("\n.fixed_conn M0_SLICE Q0_SLICE\n");
    write(&path, bits);
    f.refresh_database();
    let mut data = [0; 8];
    local_bit(&mut data, 0, 6);
    let mut r = request(
        vec![w("Q0_SLICE")],
        vec![(w("CLK0"), Logic::Zero), (w("LSR0"), Logic::Zero)],
    );
    r.startup = Startup::Configured;
    let c = compile::compile(synthetic(&f, data, false), r).unwrap();
    assert_eq!(c.value(&w("Q0_SLICE")).unwrap(), Logic::X);
}
