mod common;
#[path = "common/tools.rs"]
mod tools;
use common::{write, Fixture};
use ecp5_emu::{
    database::Database,
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::Arc};

fn snapshot(kind: SourceKind, data: &[u8], bram: Value) -> Value {
    json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,
        "encoding":"frame_byte_lsb0","cram_hex":data.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "bram_data":bram,"origin":if kind==SourceKind::Bitstream {"bitstream"} else {"text_config"},
        "native_registers":if kind==SourceKind::Bitstream {json!({"ctrl0":287454020_u32,"ctrl1":1432778632_u32,
            "sed_reference":2309737967_u32,"usercode":324508639_u32})} else {Value::Null},
        "parser_defaults_may_apply":kind==SourceKind::Bitstream,"native_metadata_hex":["ff0041"]})
}

fn image(data: &[u8], bram: Value) -> ConfigurationImage {
    ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n.comment source\n.sysconfig CONFIG_IOVOLTAGE 3.3\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot(SourceKind::TextConfig, data, bram)).unwrap(),
    )
    .unwrap()
}

fn bind(f: &Fixture, data: &[u8], bram: Value, resources: Value) -> ConfiguredDevice {
    let model = Arc::new(f.load());
    let doc = serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":resources})).unwrap();
    let catalog = Catalog::merge_documents(&[doc], "LFE5U-85F", 4, 16).unwrap();
    ConfiguredDevice::bind(model, Arc::new(image(data, bram)), catalog).unwrap()
}

#[test]
fn source_metadata_and_absent_text_registers_remain_distinct() {
    let source = vec![0xff, 0x00, 0x12, 0x00];
    let bit = ConfigurationImage::from_snapshot(
        source.clone(),
        SourceKind::Bitstream,
        &serde_json::to_vec(&snapshot(SourceKind::Bitstream, &[0; 8], json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(bit.source_bytes(), source);
    assert_eq!(
        bit.service()
            .native_registers
            .as_ref()
            .unwrap()
            .sed_reference,
        0x89abcdef
    );
    assert_eq!(bit.service().native_comments, [vec![0xff, 0, 0x41]]);
    assert!(bit.service().parser_defaults_may_apply);
    let text = image(&[0; 8], json!({}));
    assert!(text.service().native_registers.is_none());
    assert!(!text.service().parser_defaults_may_apply);
    assert_eq!(text.service().text_comments, ["source"]);
    assert_eq!(
        text.service().sysconfig_directives["CONFIG_IOVOLTAGE"],
        "3.3"
    );
    let mut malformed = snapshot(SourceKind::TextConfig, &[0; 8], json!({}));
    malformed["native_registers"] = json!({"ctrl0":0,"ctrl1":0,"usercode":0,"sed_reference":0});
    assert!(ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&malformed).unwrap()
    )
    .is_err());
}

#[test]
fn text_service_metadata_does_not_reinterpret_duplicate_dcs_arcs() {
    let source = b".device LFE5U-85F\n.sysconfig MASTER_SPI_PORT DISABLE\n.tile MIB_R22C67:CMUX_UL_0\narc: G_DCS0CLK0 G_VPFN0000\narc: G_DCS0CLK0 G_VPFS0700\n".to_vec();
    let image = ConfigurationImage::from_snapshot(
        source.clone(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot(SourceKind::TextConfig, &[0; 8], json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(image.source_bytes(), source);
    assert_eq!(
        image.service().sysconfig_directives["MASTER_SPI_PORT"],
        "DISABLE"
    );
}

#[test]
fn live_lut_and_ff_changes_do_not_modify_the_loaded_image_or_another_state() {
    let f = Fixture::with_storage();
    let device = bind(&f, &[0; 8], json!({}), json!([]));
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let lut = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_COMB")
        .unwrap()
        .id;
    let ff = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_FF")
        .unwrap()
        .id;
    let mut a = DeviceState::new(&device).unwrap();
    let b = DeviceState::new(&device).unwrap();
    assert_eq!(a.lut(lut).unwrap(), &[Logic::One; 16]);
    assert_eq!(a.register(ff).unwrap(), Logic::X);
    let before = device.image().cram().bytes().to_vec();
    a.write_lut(lut, 15, Logic::Zero).unwrap();
    a.write_register(ff, Logic::One).unwrap();
    assert_eq!(a.lut(lut).unwrap()[15], Logic::Zero);
    assert_eq!(b.lut(lut).unwrap()[15], Logic::One);
    assert_eq!(b.register(ff).unwrap(), Logic::X);
    assert_eq!(device.image().cram().bytes(), before);
    assert!(a.write_lut(lut, 16, Logic::One).is_err());
    assert!(a.lut(ff).is_err());
    assert!(a.belongs_to(&device));
    let wire = Wire {
        location: Location { row: 1, col: 1 },
        name: "Q0_SLICE".into(),
    };
    assert_eq!(a.wire_value(&wire), None);
    a.set_wire_value(wire.clone(), Logic::Z).unwrap();
    assert_eq!(a.wire_value(&wire), Some(Logic::Z));
}

#[test]
fn raw_mux_keeps_incomparable_sources_and_never_claims_physical_activation() {
    let f = Fixture::new();
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("G_HPBX0000 -", "G_HPBX0000 -\nA1 F0B1"),
    );
    f.refresh_database();
    let mut data = [0; 8];
    data[6] |= 1 << 4;
    data[4] |= 1 << 4; // F1B1=1, F0B0=0, F0B1=1 in the placed PLC.
    let device = bind(&f, &data, json!({}), json!([]));
    let tile = device.model().tile_id("R1C1:PLC2").unwrap();
    let mux = device.raw_mux(tile, "F0").unwrap();
    assert_eq!(mux.candidate_sources, ["A0", "A1"]);
    assert_eq!(mux.matching_sources.len(), 3);
    assert!(mux.requires_mode_interpretation);
}

fn ebr(name: &str) -> Value {
    let bits: Vec<_> = (4..=10)
        .map(|b| json!([[2, b, true]]))
        .chain((3..=4).map(|b| json!([[3, b, true]])))
        .collect();
    json!({"id":format!("LFE5U-85F/R1C1/{name}"),"family":"EBR","fields":[
        {"kind":"enum","name":format!("{name}.MODE"),"fragments":[{"tile":"R1C1:PLC2","source":"synthetic:1","default":"NONE",
            "options":{"DP16KD":[[2,3,true]],"NONE":[[2,3,false]]}}]},
        {"kind":"word","name":format!("{name}.WID"),"fragments":[{"tile":"R1C1:PLC2","source":"synthetic:2","default":"000000000","word_bits":bits}]}
    ]})
}

#[test]
fn ebr_init_preserves_ninth_bits_and_missing_or_duplicate_binding_remains_unknown() {
    let f = Fixture::with_storage();
    f.add_ebr("EBR0");
    let mut data = [0; 8];
    data[4] = 1 << 3;
    let init: Vec<_> = (0..2048).map(|n| 0x100 | (n & 255)).collect();
    let device = bind(&f, &data, json!({"0":init}), json!([ebr("EBR0")]));
    let id = device.model().resource_id("LFE5U-85F/R1C1/EBR0").unwrap();
    let mut state = DeviceState::new(&device).unwrap();
    assert_eq!(state.ebr_word(id, 0).unwrap()[8], Logic::One);
    assert!(state.ebr_word(id, 0).unwrap()[..8]
        .iter()
        .all(|b| *b == Logic::Zero));
    state.write_ebr_word(id, 0, [Logic::Zero; 9]).unwrap();
    assert_eq!(device.image().cram().bram[&0][0], 0x100);
    let missing = bind(&f, &data, json!({}), json!([ebr("EBR0")]));
    let missing_id = missing.model().resource_id("LFE5U-85F/R1C1/EBR0").unwrap();
    let state = DeviceState::new(&missing).unwrap();
    assert_eq!(state.ebr_word(missing_id, 0).unwrap(), &[Logic::X; 9]);
    assert_eq!(
        state.ebr_init_evidence(missing_id).unwrap().0,
        "missing_init"
    );
    f.add_ebr("EBR1");
    let duplicate = bind(
        &f,
        &data,
        json!({"0":init}),
        json!([ebr("EBR0"), ebr("EBR1")]),
    );
    let id = duplicate
        .model()
        .resource_id("LFE5U-85F/R1C1/EBR0")
        .unwrap();
    let state = DeviceState::new(&duplicate).unwrap();
    assert_eq!(state.ebr_init_evidence(id).unwrap().0, "duplicate_wid");
    assert_eq!(state.ebr_word(id, 0).unwrap(), &[Logic::X; 9]);
}

#[test]
fn merging_catalogues_deduplicates_exact_fragments_and_rejects_conflicting_ones() {
    let a = json!({"device":"LFE5U-85F","resources":[ebr("EBR0")]});
    let bytes = serde_json::to_vec(&a).unwrap();
    let merged =
        Catalog::merge_documents(&[bytes.clone(), bytes.clone()], "LFE5U-85F", 4, 16).unwrap();
    assert_eq!(merged.resources[0].fields.len(), 2);
    assert_eq!(merged.resources[0].fields[0].fragments.len(), 1);
    let mut b = a.clone();
    b["resources"][0]["fields"][0]["fragments"][0]["options"]["DP16KD"] = json!([[2, 4, true]]);
    assert!(Catalog::merge_documents(
        &[bytes, serde_json::to_vec(&b).unwrap()],
        "LFE5U-85F",
        4,
        16
    )
    .is_err());
}

#[test]
fn wrong_image_layout_is_rejected_before_reading_resource_bits() {
    let f = Fixture::new();
    let model = Arc::new(f.load());
    let mut snap = snapshot(SourceKind::TextConfig, &[0; 8], json!({}));
    snap["frames"] = json!(8);
    snap["bits_per_frame"] = json!(8);
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snap).unwrap(),
    )
    .unwrap();
    let catalog = Catalog::merge_documents(&[], "LFE5U-85F", 4, 16).unwrap();
    assert!(ConfiguredDevice::bind(model, Arc::new(image), catalog).is_err());
}

#[test]
fn state_identity_includes_decoded_bits_and_catalogue_not_only_source_bytes() {
    let f = Fixture::with_storage();
    let a = bind(&f, &[0; 8], json!({}), json!([]));
    let mut data = [0; 8];
    data[4] = 1 << 3;
    let b = bind(&f, &data, json!({}), json!([]));
    assert_eq!(a.image().source_sha256(), b.image().source_sha256());
    assert_ne!(a.image().decoded_sha256(), b.image().decoded_sha256());
    let state = DeviceState::new(&a).unwrap();
    assert!(!state.belongs_to(&b));
    let other = bind(
        &f,
        &[0; 8],
        json!({}),
        json!([{"id":"LFE5U-85F/R1C1/PIOA","family":"PIO","fields":[]} ]),
    );
    assert_eq!(a.image().decoded_sha256(), other.image().decoded_sha256());
    assert!(!state.belongs_to(&other));
}

#[test]
fn empty_and_disagreeing_word_replicas_do_not_substitute_the_default() {
    let f = Fixture::with_storage();
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    let mut db = fs::read_to_string(&path).unwrap();
    db.push_str("\n.config TEST.WORD 00\n-\nF0B0 F0B1\n");
    write(&path, db);
    f.refresh_database();
    let mut data = [0; 8];
    data[4] = 1 << 3;
    let device = bind(&f, &data, json!({}), json!([]));
    let tile = device.model().tile_id("R1C1:PLC2").unwrap();
    assert_eq!(
        device.word(tile, "TEST.WORD").unwrap(),
        [Logic::X, Logic::X]
    );
}

#[test]
#[ignore = "requires local Trellis, ecppack and the built trellis_image_reader"]
fn native_bit_and_text_keep_the_same_payload_without_losing_service_registers() {
    use std::fmt::Write as _;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let database = Database::discover(None).unwrap();
    let model =
        Arc::new(DeviceModel::load(&database, &root.join("docs/inventory/LFE5U-85F")).unwrap());
    let temp = Fixture::new();
    let config = temp.root.join("native.config");
    let mut text=String::from(".device LFE5U-85F\n.comment native image fixture\n.sysconfig CONFIG_IOVOLTAGE 3.3\n\
.tile R11C10:PLC2\nword: SLICEA.K0.INIT 1010000101011100\n\
.tile_group MIB_R22C105:MIB_EBR0 MIB_R22C106:MIB_EBR1\nenum: EBR0.MODE DP16KD\nword: EBR0.WID 110000000\n\
enum: EBR0.DP16KD.DATA_WIDTH_A 9\nenum: EBR0.DP16KD.DATA_WIDTH_B 9\n.bram_init 3\n");
    for n in 0..2048 {
        write!(
            text,
            "{:03x}{}",
            (n * 13 + 3) & 511,
            if n % 8 == 7 { '\n' } else { ' ' }
        )
        .unwrap();
    }
    write(&config, &text);
    let text_image = Arc::new(ConfigurationImage::load(&config, &database, None).unwrap());
    let prepared = ConfiguredDevice::bind(
        Arc::clone(&model),
        Arc::clone(&text_image),
        load_catalogue(&model, &root.join("docs/inventory/LFE5U-85F")).unwrap(),
    )
    .unwrap();
    let state = DeviceState::new(&prepared).unwrap();
    let plc = model.resource_id("LFE5U-85F/R11C10/PLC2").unwrap();
    let lut = model
        .nodes(plc)
        .find(|n| n.native_name == "SLICEA.K0")
        .unwrap()
        .id;
    for bit in 0..16 {
        assert_eq!(
            state.lut(lut).unwrap()[bit],
            Logic::from_bool(0xa15c & (1 << bit) != 0)
        );
    }
    assert_eq!(state.summary().flip_flops, 83640);
    assert_eq!(state.summary().known_flip_flops, 0);
    assert_eq!(state.summary().ebr_initialization["bound"], 1);
    for compressed in [false, true] {
        let bit = temp.root.join(if compressed {
            "compressed.bit"
        } else {
            "plain.bit"
        });
        let mut command = tools::command("ecppack");
        command.arg(&config).arg(&bit).arg("--db").arg(&database);
        if compressed {
            command.arg("--compress");
        }
        let packed = command.output().unwrap();
        assert!(
            packed.status.success(),
            "{}",
            String::from_utf8_lossy(&packed.stderr)
        );
        // Parser-only synthetic tail. It is never sent to hardware.
        let mut source = fs::read(&bit).unwrap();
        for (op, value) in [
            (0x22, 0x11223344_u32),
            (0x23, 0x55667788),
            (0xa2, 0x89abcdef),
            (0xc2, 0x13579bdf),
        ] {
            source.extend([op, 0, 0, 0]);
            source.extend(value.to_be_bytes());
        }
        write(&bit, &source);
        let image = ConfigurationImage::load(&bit, &database, None).unwrap();
        assert_eq!(image.source_bytes(), source);
        assert_eq!(image.cram().bytes(), text_image.cram().bytes());
        assert_eq!(image.cram().bram, text_image.cram().bram);
        let regs = image.service().native_registers.as_ref().unwrap();
        assert_eq!(
            (regs.ctrl0, regs.ctrl1, regs.sed_reference, regs.usercode),
            (0x11223344, 0x55667788, 0x89abcdef, 0x13579bdf)
        );
        assert!(image.service().parser_defaults_may_apply);
        let prepared_bit = ConfiguredDevice::bind(
            Arc::clone(&model),
            Arc::new(image),
            load_catalogue(&model, &root.join("docs/inventory/LFE5U-85F")).unwrap(),
        )
        .unwrap();
        let state_bit = DeviceState::new(&prepared_bit).unwrap();
        assert_eq!(state_bit.lut(lut).unwrap(), state.lut(lut).unwrap());
        let ebr = model.resource_id("LFE5U-85F/R22C105/EBR0").unwrap();
        assert_eq!(
            state_bit.ebr_init_evidence(ebr).unwrap(),
            ("bound", "native_decoded_without_write_mask")
        );
        for address in [0, 9, 2047] {
            assert_eq!(
                state_bit.ebr_word(ebr, address).unwrap(),
                state.ebr_word(ebr, address).unwrap()
            );
        }
    }
    assert!(text_image.service().native_registers.is_none());
    assert_eq!(
        text_image.service().sysconfig_directives["CONFIG_IOVOLTAGE"],
        "3.3"
    );
}
