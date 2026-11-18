//! R08 regression: a fabric-driven PIO need not have an OUTDEL setting.
mod common;
use common::{write, Fixture};
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{Options, Time},
    logic::Logic,
    model::topology::{Location, Wire},
    prepared::ConfiguredDevice,
};
use serde_json::{json, Value};
use std::{fs, sync::Arc};

fn wire(name: &str) -> Reference {
    Reference::Wire(Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    })
}

fn plan(family: &str, outdel: Option<Value>, shared: bool) -> anyhow::Result<Compiled> {
    let default = outdel.as_ref().map(|_| Some("DISABLED"));
    plan_state(family, outdel, shared, default, true, false)
}

fn plan_state(
    family: &str,
    outdel: Option<Value>,
    shared: bool,
    database_default: Option<Option<&str>>,
    decoded: bool,
    unused_tile: bool,
) -> anyhow::Result<Compiled> {
    let f = Fixture::with_compiler();
    let path = f.evidence.join("resource_instances.csv");
    let mut instances = fs::read_to_string(&path).unwrap();
    let anchor = if unused_tile {
        "R1C1:PLC2"
    } else {
        "CIB_R1C1:CIB"
    };
    instances.push_str(&format!(
        "LFE5U-85F/R1C1/{family}A,{family},1,1,{family}A,{anchor},,True,{family}A,output_delay_regression\n"
    ));
    write(&path, instances);
    let path = f.evidence.join("iologic_research/resource_ports.csv");
    let mut ports = fs::read_to_string(&path).unwrap();
    ports.push_str("LFE5U-85F/R1C1/PIOA,IOLDO,in,1,1,IOLDO_TO_PIO\n");
    ports.push_str(&format!(
        "LFE5U-85F/R1C1/{family}A,IOLDO,out,1,1,PLAIN_OUTPUT\n\
LFE5U-85F/R1C1/{family}A,IOLDOD,out,1,1,DELAY_OUTPUT\n\
LFE5U-85F/R1C1/{family}A,TXDATA0,in,1,1,TXDATA\n"
    ));
    write(&path, ports);
    let path = f.db.join("ECP5/tiledata/CIB/bits.db");
    let mut bits = fs::read_to_string(&path).unwrap();
    bits.push_str(
        "\n.fixed_conn SHARED_OUTPUT PLAIN_OUTPUT\n.fixed_conn SHARED_OUTPUT DELAY_OUTPUT\n",
    );
    write(&path, bits);
    if let Some(default) = database_default {
        let path = f.db.join(if unused_tile {
            "ECP5/tiledata/PLC2/bits.db"
        } else {
            "ECP5/tiledata/CIB/bits.db"
        });
        let mut bits = fs::read_to_string(&path).unwrap();
        bits.push_str(&format!(
            "\n.config_enum IOLOGICA.DELAY.OUTDEL{}\nDISABLED -\nENABLED F0B5\n",
            default.map(|v| format!(" {v}")).unwrap_or_default()
        ));
        write(&path, bits);
    }
    f.refresh_database();
    let field = |name: &str, options: Value| {
        json!({
            "kind":"enum", "name":name,
            "fragments":[{"tile":"CIB_R1C1:CIB", "source":"fixture", "default":null, "options":options}]
        })
    };
    let fields = vec![
        field("PIOA.BASE_TYPE", json!({"OUTPUT_LVCMOS33":[[0,4,true]]})),
        field("PIOA.OPENDRAIN", json!({"OFF":[[0,5,false]]})),
        field(
            "PIOA.DATAMUX_OREG",
            json!({"PADDO":[],"IOLDO":[[1,4,true]]}),
        ),
        field(
            "PIOA.TRIMUX_TSREG",
            json!({"PADDT":[],"IOLTO":[[1,5,true]]}),
        ),
    ];
    let mut delay_fields: Vec<_> = outdel
        .map(|options| field("IOLOGICA.DELAY.OUTDEL", options))
        .into_iter()
        .collect();
    for (name, value) in [
        ("MODE", "NONE"),
        ("GSR", "DISABLED"),
        ("SRMODE", "ASYNC"),
        ("DELAY.WAIT_FOR_EDGE", "DISABLED"),
        ("IDDRXN.MODE", "NONE"),
        ("ODDRXN.MODE", "NONE"),
        ("MIDDRX.MODE", "NONE"),
        ("MODDRX.MODE", "NONE"),
        ("MTDDRX.MODE", "NONE"),
    ] {
        delay_fields.push(field(&format!("IOLOGICA.{name}"), json!({value:[]})));
    }
    let mut resources = vec![json!({"id":"LFE5U-85F/R1C1/PIOA", "family":"PIO", "fields":fields})];
    if decoded {
        resources.push(json!({"id":format!("LFE5U-85F/R1C1/{family}A"), "family":family, "fields":delay_fields}));
    }
    let catalogue = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F", "resources":resources})).unwrap()],
        "LFE5U-85F",
        4,
        16,
    )
    .unwrap();
    let snapshot = json!({
        "schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,
        "encoding":"frame_byte_lsb0","cram_hex":"1000000000000000","bram_data":{},
        "origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]
    });
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let device =
        Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap());
    compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![
                (wire("PADDOA_PIO"), Logic::Zero),
                (wire("PADDTA_PIO"), Logic::Zero),
                // Opposite values prove which data path was selected.
                (wire("IOLDO_TO_PIO"), Logic::One),
                (wire("TXDATA"), Logic::Zero),
            ],
            probes: vec![if shared {
                wire("SHARED_OUTPUT")
            } else {
                Reference::Pin("G1".into())
            }],
            options: Options::default(),
        },
    )
}

fn assert_output(mut compiled: Compiled, want: Logic) {
    compiled.kernel.run_until(Time::ZERO).unwrap();
    assert_eq!(
        compiled
            .kernel
            .value(*compiled.probes.values().next().unwrap())
            .unwrap(),
        want
    );
}

#[test]
fn absent_sio_output_delay_keeps_the_fabric_driver() {
    for shared in [false, true] {
        assert_output(plan("SIOLOGIC", None, shared).unwrap(), Logic::Zero);
    }
}

#[test]
fn only_enabled_output_delay_selects_iologic() {
    for family in ["IOLOGIC", "SIOLOGIC"] {
        for (options, want) in [
            (
                json!({"DISABLED":[[0,5,false]],"ENABLED":[[0,5,true]]}),
                Logic::Zero,
            ),
            (json!({"ENABLED":[[0,5,false]]}), Logic::One),
        ] {
            for shared in [false, true] {
                let compiled = plan(family, Some(options.clone()), shared).unwrap();
                if shared {
                    let source = if want == Logic::One {
                        "DELAY_OUTPUT"
                    } else {
                        "PLAIN_OUTPUT"
                    };
                    assert!(compiled
                        .report
                        .routes
                        .iter()
                        .any(|route| route.sink == "SHARED_OUTPUT" && route.source == source));
                }
                assert_output(compiled, if shared { Logic::Zero } else { want });
            }
        }
    }
}

#[test]
fn invalid_output_delay_decode_still_refuses_the_plan() {
    for family in ["IOLOGIC", "SIOLOGIC"] {
        for shared in [false, true] {
            for options in [
                None,
                Some(json!({"ENABLED":[[0,5,true]]})),
                Some(json!({"DISABLED":[],"ENABLED":[]})),
            ] {
                let message = format!(
                    "{:#}",
                    plan_state(family, options, shared, Some(Some("DISABLED")), true, false)
                        .err()
                        .expect("invalid OUTDEL accepted")
                );
                assert!(
                    message.contains(&format!("LFE5U-85F/R1C1/{family}A"))
                        && message.contains("DELAY.OUTDEL"),
                    "{message}"
                );
            }
        }
        let message = format!(
            "{:#}",
            plan_state(family, None, false, Some(Some("DISABLED")), false, false)
                .err()
                .expect("configured tile without decode accepted")
        );
        assert!(
            message.contains("DELAY.OUTDEL") && message.contains("configured tile"),
            "{message}"
        );
    }
}

#[test]
fn undecoded_unused_tile_requires_a_disabled_database_default() {
    assert_output(
        plan_state("IOLOGIC", None, false, Some(Some("DISABLED")), false, true).unwrap(),
        Logic::Zero,
    );
    for default in [None, Some("ENABLED")] {
        let message = format!(
            "{:#}",
            plan_state("IOLOGIC", None, false, Some(default), false, true)
                .err()
                .expect("non-DISABLED default accepted")
        );
        assert!(
            message.contains("LFE5U-85F/R1C1/IOLOGICA") && message.contains("DELAY.OUTDEL"),
            "{message}"
        );
    }
}

#[test]
#[ignore = "requires the native image reader, Trellis database and dsp_register_native fixtures"]
fn r95c18_database_and_fixture_decode_prove_case_a() {
    use ecp5_emu::{
        database::Database, image::SourceKind, model::DeviceModel, prepared::load_catalogue,
    };
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let id = model.resource_id("LFE5U-85F/R95C18/SIOLOGICA").unwrap();
    let tile = model.tile(model.resource(id).anchor);
    assert_eq!(tile.kind, "SPICB0");
    let sites: Vec<_> = model
        .site_references()
        .iter()
        .filter(|s| s.resource == Some(id))
        .collect();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].tile, model.resource(id).anchor);
    let kind = &model.database().types[&tile.kind];
    assert!(!kind.enums.contains_key("IOLOGICA.DELAY.OUTDEL"));
    assert!(!kind.words.contains_key("IOLOGICA.DELAY.OUTDEL"));
    for mode in ["sync", "async"] {
        for extension in ["config", "bit"] {
            let image = Arc::new(
                ConfigurationImage::load(
                    &root.join(format!("target/dsp-registers-{mode}/top.{extension}")),
                    &db,
                    None,
                )
                .unwrap(),
            );
            if image.source_kind() == SourceKind::TextConfig {
                let cfg = ecp5_emu::config::Config::parse(
                    std::str::from_utf8(image.source_bytes()).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    cfg.tiles[&tile.name].enums["PIOA.BASE_TYPE"],
                    "OUTPUT_LVCMOS33"
                );
            }
            let device = Arc::new(
                ConfiguredDevice::bind(
                    Arc::clone(&model),
                    image,
                    load_catalogue(&model, &evidence).unwrap(),
                )
                .unwrap(),
            );
            assert!(device.resource_settings(id).is_none());
            let pio = model.resource_id("LFE5U-85F/R95C18/PIOA").unwrap();
            assert!(device.resource_settings(pio).is_some());
            let ball = model
                .package_pins()
                .find(|p| p.package == "CABGA381" && p.pio == Some(pio))
                .unwrap();
            compile::compile(
                device,
                Request {
                    package: "CABGA381".into(),
                    scope: Scope::Observed,
                    startup: Startup::Configured,
                    drives: vec![],
                    probes: vec![Reference::Pin(ball.ball.clone())],
                    options: Options::default(),
                },
            )
            .unwrap();
            println!("{mode}/{extension}: case (a), SPICB0 has no OUTDEL database field, SIOLOGICA decode absent, PIOA decode present; output plans successfully");
        }
    }
}
