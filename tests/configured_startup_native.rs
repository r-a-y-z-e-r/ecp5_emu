//! The `configured` startup profile against the first hardware observation
//! (docs/inventory/LFE5U-85F/hs01_init_slice): flip-flops start at REGSET
//! whatever SLICE GSR says, and an unselected local reset is inactive. The
//! IO constraints are the ones used on the board, so the jointly decoded PIO
//! settings (DRIVE=4, default PULLMODE) are exercised too.
#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{fs, path::Path, path::PathBuf, sync::Arc};

/// ULX3S LED balls, led[0] first; active-high on the board.
const LEDS: [&str; 8] = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];

/// `inputs` are (port, ball, IOBUF attributes); every LED gets `led_io`.
fn build(dir: &Path, hdl: &str, inputs: &[(&str, &str, &str)], led_io: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("top.v"), hdl).unwrap();
    let mut lpf = String::new();
    for (port, ball, io) in inputs.iter().map(|&(p, b, m)| (p.to_string(), b, m)).chain(
        LEDS.iter()
            .enumerate()
            .map(|(i, &ball)| (format!("led[{i}]"), ball, led_io)),
    ) {
        lpf.push_str(&format!(
            "LOCATE COMP \"{port}\" SITE \"{ball}\";\nIOBUF PORT \"{port}\" {io};\n"
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
            .current_dir(dir)
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
}

fn pin(ball: &str) -> Reference {
    Reference::Pin(ball.into())
}

fn compile_image(
    path: &Path,
    scope: Scope,
    startup: Startup,
    drives: &[(&str, Logic)],
) -> Compiled {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(ConfigurationImage::load(path, &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let request = Request {
        package: "CABGA381".into(),
        scope,
        startup,
        drives: drives.iter().map(|&(b, v)| (pin(b), v)).collect(),
        probes: LEDS.iter().map(|b| pin(b)).collect(),
        options: Options::default(),
    };
    compile::compile(device, request).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
}

/// LED values as a string, led[7] first, like the board notes.
fn leds(compiled: &Compiled) -> String {
    LEDS.iter()
        .rev()
        .map(|b| compiled.value(&pin(b)).unwrap().to_string())
        .collect()
}

fn drive(compiled: &mut Compiled, time: u64, values: &[(&str, Logic)]) {
    let updates = values
        .iter()
        .map(|&(ball, value)| (compiled.drivers[&pin(ball).label()], value))
        .collect::<Vec<_>>();
    let time = Time::from_ps(time * 1000);
    compiled
        .kernel
        .schedule_batch(time, &updates, None)
        .unwrap();
    compiled.kernel.run_until(time).unwrap();
}

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and the native image reader"]
fn hs01_run_b_matches_the_board_observation() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/configured-startup/hs01b");
    build(
        &dir,
        include_str!("../docs/inventory/LFE5U-85F/hs01_init_slice/hs01_noclk.v"),
        // The LPF used on the board (ulx3s.lpf in the evidence).
        &[
            ("clk_25mhz", "G2", "IO_TYPE=LVCMOS33 PULLMODE=NONE"),
            ("btn[0]", "D6", "IO_TYPE=LVCMOS33 PULLMODE=UP"),
            ("btn[1]", "R1", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
            ("btn[2]", "T1", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
            ("btn[3]", "R18", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
            ("btn[4]", "V1", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
            ("btn[5]", "U1", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
            ("btn[6]", "H16", "IO_TYPE=LVCMOS33 PULLMODE=DOWN"),
        ],
        "IO_TYPE=LVCMOS33 DRIVE=4",
    );
    let idle = [("R1", Logic::Zero), ("T1", Logic::Zero)];
    for extension in ["config", "bit"] {
        let path = dir.join(format!("top.{extension}"));
        let mut c = compile_image(&path, Scope::Observed, Startup::Configured, &idle);
        assert_eq!(c.report.system.gsr_enabled_ff, 3, "{extension}");
        assert_eq!(c.report.system.gsr_disabled_ff, 3, "{extension}");
        // Board, run B: D7, D5, D3, D2 and D1 lit right after configuration.
        assert_eq!(leds(&c), "10101110", "{extension}");
        // Board: holding FIRE1 and pressing FIRE2 inverts all six flip-flops.
        drive(&mut c, 1, &[("R1", Logic::One)]);
        drive(&mut c, 2, &[("T1", Logic::One)]);
        assert_eq!(leds(&c), "10010001", "{extension}");
        drive(&mut c, 3, &[("T1", Logic::Zero)]);
        drive(&mut c, 4, &[("T1", Logic::One)]);
        assert_eq!(leds(&c), "10101110", "{extension}");
        // Without FIRE1 the clock edge is not enabled.
        drive(&mut c, 5, &[("R1", Logic::Zero), ("T1", Logic::Zero)]);
        drive(&mut c, 6, &[("T1", Logic::One)]);
        assert_eq!(leds(&c), "10101110", "{extension}");
    }
    // ready-unknown keeps the documented conservative start: only the
    // constants are known.
    let c = compile_image(
        &dir.join("top.config"),
        Scope::Observed,
        Startup::ReadyUnknown,
        &idle,
    );
    assert_eq!(leds(&c), "10xxxxxx");
}

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and the native image reader"]
fn ulx3s_blinky_without_reset_counts_from_its_init_value() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/configured-startup/blinky");
    build(
        &dir,
        r#"module top(input clk_25mhz, output [7:0] led);
  reg [7:0] count = 0;
  always @(posedge clk_25mhz) count <= count + 1;
  assign led = count;
endmodule
"#,
        // The official ULX3S constraint style for LEDs.
        &[("clk_25mhz", "G2", "IO_TYPE=LVCMOS33 PULLMODE=NONE")],
        "IO_TYPE=LVCMOS33 PULLMODE=NONE DRIVE=4",
    );
    for (extension, scope) in [
        ("config", Scope::Observed),
        ("bit", Scope::Observed),
        ("bit", Scope::AllPlcRegisters),
    ] {
        let path = dir.join(format!("top.{extension}"));
        let mut c = compile_image(&path, scope, Startup::Configured, &[("G2", Logic::Zero)]);
        // The counter has no reset, so every local reset selector is unrouted.
        assert!(
            c.report
                .constants
                .iter()
                .any(|u| u.field.starts_with("MUXLSR")),
            "{extension} {scope:?}"
        );
        assert_eq!(leds(&c), "00000000", "{extension} {scope:?}");
        for edge in 1..=300_u64 {
            drive(&mut c, 2 * edge - 1, &[("G2", Logic::One)]);
            drive(&mut c, 2 * edge, &[("G2", Logic::Zero)]);
            assert_eq!(
                leds(&c),
                format!("{:08b}", edge % 256),
                "{extension} {scope:?} after {edge} edges"
            );
        }
    }
}
