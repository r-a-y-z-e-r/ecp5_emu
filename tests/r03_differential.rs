//! R03/X05: route ordinary FFs with differential LPF constraints, then compare
//! every sampled pad in the event kernel and fast mode. No board evidence.
mod board;
#[path = "common/tools.rs"]
mod tools;

use board::{pin, PERIOD_PS};
use ecp5_emu::{
    compile::{self, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

const FF: &str = "module top(input clk,d,output reg q); always @(posedge clk)q<=d; endmodule";

fn design(name: &str, verilog: &str, lpf: &str) -> Arc<ConfiguredDevice> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/r03-differential").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("top.v"), verilog).unwrap();
    std::fs::write(dir.join("top.lpf"), lpf).unwrap();
    for (program, args) in [
        (
            "yosys",
            vec![
                "-Qq",
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
                "-q",
            ],
        ),
        ("ecppack", vec!["top.config", "top.bit"]),
    ] {
        let output = tools::command(program)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    static MODEL: OnceLock<Arc<DeviceModel>> = OnceLock::new();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model =
        Arc::clone(MODEL.get_or_init(|| Arc::new(DeviceModel::load(&db, &evidence).unwrap())));
    let image = Arc::new(ConfigurationImage::load(&dir.join("top.config"), &db, None).unwrap());
    Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    )
}

fn lpf(standard: &str, input: bool, p: &str) -> String {
    [("clk", "G2", "LVCMOS25"), ("d", if input { p } else { "L4" }, if input { standard } else { "LVCMOS25" }), ("q", if input { "B11" } else { p }, if input { "LVCMOS25" } else { standard })]
        .into_iter().map(|(port, ball, io)| format!("LOCATE COMP \"{port}\" SITE \"{ball}\"; IOBUF PORT \"{port}\" IO_TYPE={io} PULLMODE=NONE;\n")).collect()
}

fn compile(
    device: Arc<ConfiguredDevice>,
    probes: &[&str],
    drives: &[(&str, Logic)],
) -> anyhow::Result<Compiled> {
    compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: std::iter::once(("G2", Logic::Zero))
                .chain(drives.iter().copied())
                .map(|(b, v)| (pin(b), v))
                .collect(),
            probes: probes.iter().map(|b| pin(b)).collect(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
}

fn agree(c: &mut Compiled, fast: &mut FastSim, signals: &[SignalId], at: Time) {
    c.kernel.run_until(at).unwrap();
    fast.run_until(at).unwrap();
    for &signal in signals {
        assert_eq!(
            fast.value(signal),
            Some(c.kernel.value(signal).unwrap()),
            "at {at:?}"
        );
    }
}

fn trace(device: Arc<ConfiguredDevice>, input: bool, p: &str, n: &str, negative: bool) {
    let data = if input { p } else { "L4" };
    let q = if input { "B11" } else { p };
    let mut drives = vec![(data, Logic::Zero)];
    if negative {
        drives.push((n, Logic::One));
    }
    let probes = if input && !negative {
        vec![p, q]
    } else {
        vec![p, n, q]
    };
    let mut c = compile(device, &probes, &drives).unwrap_or_else(|e| panic!("{e:#}"));
    let key = if p == "B9" {
        "LFE5U-85F/R0C58/PIOB"
    } else {
        "LFE5U-85F/R35C0/PIOB"
    };
    if !input || negative {
        assert!(c.report.pio_modes[key].complement_of.is_some());
    }
    let signals: Vec<_> = probes.iter().map(|b| c.probes[&pin(b).label()]).collect();
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    for cycle in 1..=128u64 {
        // First stimulus is exactly X05's D=1; then both logic levels vary.
        let value = cycle == 1 || (cycle ^ (cycle >> 2)) & 1 != 0;
        let at = Time::from_ps((cycle - 1) * PERIOD_PS + PERIOD_PS / 4);
        let mut updates = vec![(c.drivers[&pin(data).label()], value)];
        if negative {
            // Complementary in the first half; equal pads in the second half
            // demonstrate that N does not affect this digital input model.
            updates.push((
                c.drivers[&pin(n).label()],
                if cycle <= 64 { !value } else { value },
            ));
        }
        let levels: Vec<_> = updates
            .iter()
            .map(|&(d, v)| (d, Logic::from_bool(v)))
            .collect();
        c.kernel.schedule_batch(at, &levels, None).unwrap();
        fast.set_at(at, &updates).unwrap();
        agree(&mut c, &mut fast, &signals, at);
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        for t in [rise, fall] {
            agree(&mut c, &mut fast, &signals, t);
            assert_eq!(c.value(&pin(q)).unwrap(), Logic::from_bool(value));
            if !input {
                assert_eq!(c.value(&pin(n)).unwrap(), Logic::from_bool(!value));
            }
        }
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang and the native Trellis reader/database"]
fn x05_lvds_output_has_equal_kernel_and_fast_traces() {
    trace(
        design("x05", FF, &lpf("LVDS", false, "A2")),
        false,
        "A2",
        "B1",
        false,
    );
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang and the native Trellis reader/database"]
fn lvds_input_reads_p_with_equal_kernel_and_fast_traces() {
    let device = design("lvds-input", FF, &lpf("LVDS", true, "A2"));
    trace(Arc::clone(&device), true, "A2", "B1", true);
    trace(device, true, "A2", "B1", false);
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang and the native Trellis reader/database"]
fn differential_io_aliases_have_equal_kernel_and_fast_traces() {
    for standard in [
        "LVCMOS18D",
        "LVCMOS25D",
        "LVCMOS33D",
        "LVDS25E",
        "LVPECL33E",
        "MLVDS25E",
        "HSUL12D",
        "SSTL18D_I",
        "SSTL18D_II",
        "SSTL15D_I",
        "SSTL15D_II",
        "SSTL135D_I",
        "SSTL135D_II",
    ] {
        trace(
            design(&format!("out-{standard}"), FF, &lpf(standard, false, "A2")),
            false,
            "A2",
            "B1",
            false,
        );
    }
    for standard in [
        "BLVDS25",
        "MLVDS25",
        "SLVS",
        "SUBLVDS",
        "LVPECL33",
        "LVCMOS18D",
        "LVCMOS25D",
        "LVCMOS33D",
        "HSUL12D",
        "SSTL18D_I",
        "SSTL18D_II",
        "SSTL15D_I",
        "SSTL15D_II",
        "SSTL135D_I",
        "SSTL135D_II",
    ] {
        trace(
            design(&format!("in-{standard}"), FF, &lpf(standard, true, "A2")),
            true,
            "A2",
            "B1",
            true,
        );
    }
    for standard in ["LVCMOS12D", "LVCMOS15D"] {
        trace(
            design(&format!("top-{standard}"), FF, &lpf(standard, false, "B9")),
            false,
            "B9",
            "C10",
            false,
        );
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack and the native Trellis reader/database"]
fn differential_bidir_and_open_drain_are_refused_with_a_reason() {
    let verilog = "module top(input clk,d,en,inout p,output reg q); assign p=en?d:1'bz; always @(posedge clk)q<=p; endmodule";
    for standard in ["LVDS", "LVCMOS33D", "MLVDS25E"] {
        let constraints = lpf(standard, false, "A2").replace("COMP \"q\" SITE \"A2\"", "COMP \"p\" SITE \"A2\"").replace("PORT \"q\" IO_TYPE=", "PORT \"p\" IO_TYPE=") + "LOCATE COMP \"q\" SITE \"B11\"; IOBUF PORT \"q\" IO_TYPE=LVCMOS25 PULLMODE=NONE; LOCATE COMP \"en\" SITE \"R1\"; IOBUF PORT \"en\" IO_TYPE=LVCMOS25 PULLMODE=NONE;";
        let device = design(&format!("bidir-{standard}"), verilog, &constraints);
        let error = compile(device, &["A2"], &[("L4", Logic::Zero), ("R1", Logic::Zero)])
            .err()
            .unwrap();
        let error = format!("{error:#}");
        assert!(
            error.contains(standard) && error.contains("bidirectional differential"),
            "{standard}: {error}"
        );
    }
    let constraints = lpf("LVDS", false, "A2") + "IOBUF PORT \"q\" OPENDRAIN=ON;";
    let device = design("open-drain", FF, &constraints);
    let error = compile(device, &["A2"], &[("L4", Logic::Zero)])
        .err()
        .unwrap();
    let error = format!("{error:#}");
    assert!(
        error.contains("LVDS") && error.contains("open-drain"),
        "{error}"
    );
}
