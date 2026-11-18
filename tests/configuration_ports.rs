//! Persistent configuration ports must never silently become user pads.
mod common;
use common::{write, Fixture};
use ecp5_emu::{
    compile::{self, Board, BoardWire, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{Options, Time},
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    prepared::{load_catalogue, ConfiguredDevice},
};
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

const PORTS: [&str; 3] = ["MASTER_SPI_PORT", "SLAVE_SPI_PORT", "SLAVE_PARALLEL_PORT"];

fn wire(name: &str) -> Reference {
    Reference::Wire(Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    })
}
fn pin(ball: &str) -> Reference {
    Reference::Pin(ball.into())
}
fn request(probes: Vec<Reference>) -> Request {
    Request {
        package: "CABGA381".into(),
        scope: Scope::Observed,
        startup: Startup::Configured,
        drives: vec![
            (wire("PADDOA_PIO"), Logic::Zero),
            (wire("PADDTA_PIO"), Logic::One),
        ],
        probes,
        options: Options::default(),
    }
}

fn fixture(function: &str) -> Fixture {
    let f = Fixture::with_compiler();
    f.replace(
        "package_system_research/physical_pins.csv",
        "PT1A,,0,GPIO",
        &format!("PT1A,{function},0,GPIO"),
    );
    let path = f.db.join("ECP5/tiledata/CIB/bits.db");
    let mut bits = fs::read_to_string(&path).unwrap();
    for (k, name) in PORTS.iter().enumerate() {
        bits.push_str(&format!(
            "\n.config_enum SYSCONFIG.{name} DISABLE\nDISABLE -\nENABLE F1B{}\n",
            4 + k
        ));
    }
    write(&path, bits);
    f.refresh_database();
    f
}

fn device(
    f: &Fixture,
    direction: &str,
    port: usize,
    cram: bool,
    directive: Option<&str>,
) -> Arc<ConfiguredDevice> {
    let fields = [
        ("BASE_TYPE", json!({format!("{direction}_LVCMOS33"):[[0,4,true]]})),
        ("PULLMODE", json!({"NONE":[[0,6,false],[0,7,false]]})),
        ("OPENDRAIN", json!({"OFF":[[0,5,false]]})),
        ("DATAMUX_OREG", json!({"PADDO":[],"IOLDO":[[1,4,true]]})),
        ("TRIMUX_TSREG", json!({"PADDT":[],"IOLTO":[[1,5,true]]})),
    ].into_iter().map(|(name, options)| json!({
        "kind":"enum", "name":format!("PIOA.{name}"),
        "fragments":[{"tile":"CIB_R1C1:CIB","source":"fixture","default":null,"options":options}]
    })).collect::<Vec<_>>();
    let catalogue = Catalog::merge_documents(&[serde_json::to_vec(&json!({
        "device":"LFE5U-85F", "resources":[{"id":"LFE5U-85F/R1C1/PIOA","family":"PIO","fields":fields}]
    })).unwrap()], "LFE5U-85F", 4, 16).unwrap();
    let mut data = [0u8; 8];
    data[0] = 0x10;
    if cram {
        data[3] |= 1 << port;
    }
    let snapshot = json!({
        "schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,
        "encoding":"frame_byte_lsb0","cram_hex":data.iter().map(|v|format!("{v:02x}")).collect::<String>(),
        "bram_data":{},"origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]
    });
    let mut source = String::from(".device LFE5U-85F\n");
    if let Some(value) = directive {
        source.push_str(&format!(".sysconfig {} {value}\n", PORTS[port]));
    }
    let image = ConfigurationImage::from_snapshot(
        source.into_bytes(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap())
}

fn error(result: anyhow::Result<Compiled>) -> String {
    format!("{:#}", result.err().expect("retained user IO accepted"))
}
fn assert_conflict(message: &str, port: &str, ball: &str, function: &str) {
    assert!(
        message.contains(&format!("SYSCONFIG.{port}=ENABLE")),
        "{message}"
    );
    assert!(message.contains(&format!("ball {ball}")), "{message}");
    assert!(message.contains(function), "{message}");
}

#[test]
fn each_enabled_port_refuses_input_output_and_tristate_user_pads() {
    let f = fixture("D0/MOSI/IO0");
    for (port, name) in PORTS.iter().enumerate() {
        for direction in ["INPUT", "OUTPUT", "BIDIR"] {
            for (cram, directive) in [(true, None), (false, Some("ENABLE"))] {
                let mut req = request(vec![pin("G1")]);
                if direction == "OUTPUT" {
                    req.drives[1].1 = Logic::Zero;
                }
                let message = error(compile::compile(
                    device(&f, direction, port, cram, directive),
                    req,
                ));
                assert_conflict(&message, name, "G1", "D0/MOSI/IO0");
            }
        }
    }
}

#[test]
fn unused_retained_pins_are_accepted_and_reported() {
    let f = fixture("D0/MOSI/IO0");
    for (port, _) in PORTS.iter().enumerate() {
        for (cram, directive) in [(true, None), (false, Some("ENABLE"))] {
            let c = compile::compile(
                device(&f, "INPUT", port, cram, directive),
                request(vec![wire("OUT")]),
            )
            .unwrap();
            let report = &c.report.system.configuration_ports[port];
            assert!(report.enabled);
            assert!(report.rule.contains("no effect on this plan"));
            assert!(c.report.pio_modes.is_empty());
            assert!(c
                .report
                .fields
                .iter()
                .any(|f| f.name == format!("SYSCONFIG.{}", PORTS[port])));
        }
    }
}

#[test]
fn disable_and_absent_directives_preserve_user_io() {
    let f = fixture("D0/MOSI/IO0");
    for port in 0..3 {
        for direction in ["INPUT", "OUTPUT", "BIDIR"] {
            for directive in [None, Some("DISABLE")] {
                let mut req = request(vec![pin("G1")]);
                req.drives.push((pin("G1"), Logic::Zero));
                let mut c = compile::compile(device(&f, direction, port, false, directive), req)
                    .unwrap_or_else(|e| panic!("{direction}: {e:#}"));
                c.kernel.run_until(Time::ZERO).unwrap();
                assert_eq!(c.value(&pin("G1")).unwrap(), Logic::Zero);
                assert!(c
                    .report
                    .system
                    .configuration_ports
                    .iter()
                    .all(|p| !p.enabled));
            }
        }
    }
}

#[test]
fn disable_text_cannot_hide_enabled_cram_and_unknown_values_fail() {
    let f = fixture("D0/MOSI/IO0");
    for (port, name) in PORTS.iter().enumerate() {
        let message = error(compile::compile(
            device(&f, "INPUT", port, true, Some("DISABLE")),
            request(vec![pin("G1")]),
        ));
        assert_conflict(&message, name, "G1", "D0/MOSI/IO0");
        let message = error(compile::compile(
            device(&f, "INPUT", port, false, Some("invalid")),
            request(vec![wire("OUT")]),
        ));
        assert!(
            message.contains(&format!("SYSCONFIG.{name} directive invalid")),
            "{message}"
        );
    }
}

#[test]
fn contradictory_slave_spi_cram_replicas_are_refused() {
    let f = fixture("SN/CSN");
    let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
    let mut bits = fs::read_to_string(&path).unwrap();
    bits.push_str("\n.config_enum SYSCONFIG.SLAVE_SPI_PORT DISABLE\nDISABLE -\nENABLE F1B7\n");
    write(&path, bits);
    f.refresh_database();
    let message = error(compile::compile(
        device(&f, "INPUT", 1, true, None),
        request(vec![wire("OUT")]),
    ));
    assert!(
        message.contains("inconsistent SYSCONFIG.SLAVE_SPI_PORT CRAM replicas"),
        "{message}"
    );
}

#[test]
fn metadata_aliases_cover_chip_select_quad_and_parallel_pins() {
    for (port, functions) in [
        (
            0,
            vec![
                "HOLDN/DI/BUSY/CSSPIN/CEN",
                "D2/IO2",
                "D3/IO3",
                "D5/MISO2/IO5",
            ],
        ),
        (1, vec!["SN/CSN", "D1/MISO/IO1"]),
        (
            2,
            vec!["D5/MISO2/IO5", "D7/IO7", "CS1N", "WRITEN", "DOUT/CSON"],
        ),
    ] {
        for function in functions {
            let f = fixture(function);
            let message = error(compile::compile(
                device(&f, "INPUT", port, false, Some("ENABLE")),
                request(vec![pin("G1")]),
            ));
            assert_conflict(&message, PORTS[port], "G1", function);
        }
    }
    // HOLDN is documented as configuration-only; MISO2 is not primary MISO.
    for function in ["HOLDN/DI/BUSY/CSSPIN/CEN", "D5/MISO2/IO5"] {
        let f = fixture(function);
        assert!(compile::compile(
            device(&f, "INPUT", 1, false, Some("ENABLE")),
            request(vec![pin("G1")])
        )
        .is_ok());
    }
}

#[test]
fn wire_fanin_external_drives_and_board_wires_cannot_bypass_the_guard() {
    let f = fixture("SN/CSN");
    let d = device(&f, "INPUT", 1, true, None);
    let req = request(vec![wire("JPADDIA_PIO")]);
    assert_conflict(
        &error(compile::compile(Arc::clone(&d), req)),
        PORTS[1],
        "G1",
        "SN/CSN",
    );
    let mut req = request(vec![wire("OUT")]);
    req.drives.push((pin("G1"), Logic::Zero));
    assert_conflict(
        &error(compile::compile(Arc::clone(&d), req)),
        PORTS[1],
        "G1",
        "SN/CSN",
    );
    let board = Board {
        wires: vec![BoardWire {
            from: pin("G1"),
            to: pin("G1"),
        }],
        ..Board::default()
    };
    assert_conflict(
        &error(compile::compile_on_board(
            d,
            request(vec![wire("OUT")]),
            &board,
        )),
        PORTS[1],
        "G1",
        "SN/CSN",
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hr07_flash_text_enable_refuses_each_flash_ball_and_usrmclk() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let source = fs::read_to_string(evidence.join("hr07_flash/flash_dump.config")).unwrap();
    let dir = root.join("target/configuration-ports");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hr07-master-enabled.config");
    fs::write(
        &path,
        format!("{source}\n.sysconfig MASTER_SPI_PORT ENABLE\n"),
    )
    .unwrap();
    let image = Arc::new(ConfigurationImage::load(&path, &db, None).unwrap());
    let d = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    for (ball, function) in [
        ("W2", "D0/MOSI/IO0"),
        ("V2", "D1/MISO/IO1"),
        ("R2", "HOLDN/DI/BUSY/CSSPIN/CEN"),
    ] {
        let mut req = request(vec![pin(ball)]);
        req.drives.clear();
        assert_conflict(
            &error(compile::compile(Arc::clone(&d), req)),
            PORTS[0],
            ball,
            function,
        );
    }
    // Only a system strobe observed: an unobserved USRMCLK still conflicts.
    let mut req = request(vec![Reference::parse(d.model(), "sys:goe").unwrap()]);
    req.drives.clear();
    let message = error(compile::compile(d, req));
    assert_conflict(&message, PORTS[0], "U3", "CCLK");
    assert!(message.contains("USRMCLK"), "{message}");
}
