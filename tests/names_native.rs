//! HDL names (src/names.rs) on a design that nextpnr routes now: the nets and
//! ports named in the Verilog resolve to wires and pins whose values follow
//! the design.
#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{fs, path::PathBuf, sync::Arc};

const LEDS: [&str; 8] = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];

#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn hdl_names_resolve_to_the_signals_they_name() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/names-native");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("top.v"),
        "module top(input clk_25mhz, output [7:0] led);\n  reg [7:0] count = 0;\n  \
         always @(posedge clk_25mhz) count <= count + 1;\n  assign led = ~count;\nendmodule\n",
    )
    .unwrap();
    let mut lpf =
        "LOCATE COMP \"clk_25mhz\" SITE \"G2\";\nIOBUF PORT \"clk_25mhz\" IO_TYPE=LVCMOS33;\n"
            .to_string();
    for (i, ball) in LEDS.iter().enumerate() {
        lpf += &format!("LOCATE COMP \"led[{i}]\" SITE \"{ball}\";\nIOBUF PORT \"led[{i}]\" IO_TYPE=LVCMOS33 DRIVE=4;\n");
    }
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    for (program, args) in [
        (
            "yosys",
            vec![
                "-q",
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
                "--write",
                "routed.json",
                "--seed",
                "1",
                "-q",
            ],
        ),
    ] {
        let result = tools::command(program)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let names = Names::load(&dir.join("routed.json"), &model, "CABGA381").unwrap();
    let count = names.expand("count").unwrap();
    let led = names.expand("led").unwrap();
    assert_eq!(count.len(), 8);
    assert_eq!(count[3].0, "count[3]");
    assert!(count.iter().all(|(_, r)| matches!(r, Reference::Wire(_))));
    // Ports resolve to the package pins of the LPF.
    let balls: Vec<Reference> = LEDS.iter().map(|b| Reference::Pin((*b).into())).collect();
    assert_eq!(
        led.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        balls
    );
    let clock = names.expand("clk_25mhz").unwrap();
    assert_eq!(clock[0].1, Reference::Pin("G2".into()));

    let image = Arc::new(ConfigurationImage::load(&dir.join("top.config"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(clock[0].1.clone(), Logic::Zero)],
            probes: count.iter().chain(&led).map(|(_, r)| r.clone()).collect(),
            options: Options::default(),
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    let clk = c.drivers["pin:G2"];
    let value = |c: &compile::Compiled, bits: &[(String, Reference)]| {
        bits.iter().enumerate().fold(0_u32, |v, (i, (_, r))| {
            v | u32::from(c.value(r).unwrap() == Logic::One) << i
        })
    };
    for cycle in 1..=20_u64 {
        let rise = Time::from_ps(cycle * 40_000 - 20_000);
        c.kernel.schedule(rise, clk, Logic::One).unwrap();
        c.kernel
            .schedule(Time::from_ps(cycle * 40_000), clk, Logic::Zero)
            .unwrap();
        c.kernel.run_until(Time::from_ps(cycle * 40_000)).unwrap();
        assert_eq!(value(&c, &count), cycle as u32, "count after {cycle} edges");
        assert_eq!(
            value(&c, &led),
            !(cycle as u32) & 0xff,
            "led after {cycle} edges"
        );
    }
}
