#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic::{self, *},
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
fn run(dir: &Path, program: &str, args: &[&str], log: &str) {
    let out = tools::command(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    fs::write(dir.join(log), [out.stdout, out.stderr].concat()).unwrap();
    assert!(
        out.status.success(),
        "{program}: see {}",
        dir.join(log).display()
    );
}
fn product(a: u64, b: u64, sa: bool, sb: bool) -> u64 {
    let operand = |v: u64, s: bool| v as i128 - if s && v >= 131072 { 262144 } else { 0 };
    (operand(a, sa) * operand(b, sb)).rem_euclid(1i128 << 36) as u64
}
#[test]
#[ignore = "requires local Yosys/nextpnr/Trellis and native image reader"]
fn native_parallel_mult18_products_and_fabric_ff_run_from_config_and_bit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/dsp-mult18");
    fs::create_dir_all(&dir).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let mut pins = BTreeMap::from([
        ("clk".to_string(), "G2".to_string()),
        ("reset".into(), "J4".into()),
    ]);
    let mut balls = model
        .package_pins()
        .filter(|p| {
            p.package == "CABGA381" && p.pio.is_some() && !["G2", "J4"].contains(&p.ball.as_str())
        })
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    for (name, width) in [("a", 18), ("b", 18), ("p0", 36), ("p1", 36)] {
        for bit in 0..width {
            pins.insert(format!("{name}[{bit}]"), balls.next().unwrap());
        }
    }
    for name in ["sa", "sb", "captured"] {
        pins.insert(name.into(), balls.next().unwrap());
    }
    let lpf=pins.iter().map(|(p,b)|format!("LOCATE COMP \"{p}\" SITE \"{b}\";\nIOBUF PORT \"{p}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect::<String>();
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    let slice = model
        .resources()
        .find(|(_, r)| r.family == "DSP_SLICE" && r.name == "DSP_SLICE_LEFT")
        .unwrap()
        .0;
    let multipliers = model
        .children(slice)
        .iter()
        .copied()
        .filter(|&id| model.resource(id).family == "MULT18")
        .collect::<Vec<_>>();
    assert_eq!(multipliers.len(), 2);
    let mut hdl = String::from(
        "module top(input [17:0] a,b,input sa,sb,clk,reset,output [35:0] p0,p1,output captured);\n",
    );
    for (i, &id) in multipliers.iter().enumerate() {
        let r = model.resource(id);
        let mut ports = Vec::new();
        for bit in 0..18 {
            ports.push(format!(".A{bit}(a[{bit}]),.B{bit}(b[{bit}])"));
        }
        for bit in 0..36 {
            ports.push(format!(".P{bit}(p{i}[{bit}])"));
        }
        ports.extend([
            format!(".SIGNEDA({})", if i == 0 { "sa" } else { "1'b0" }),
            format!(".SIGNEDB({})", if i == 0 { "sb" } else { "1'b0" }),
            ".SOURCEA(1'b0)".into(),
            ".SOURCEB(1'b0)".into(),
            ".SIGNEDP()".into(),
        ]);
        hdl.push_str(&format!(
            "(* BEL=\"X{}/Y{}/{}\" *) MULT18X18D m{i}({});\n",
            r.location.col,
            r.location.row,
            r.name,
            ports.join(",")
        ));
    }
    hdl.push_str("TRELLIS_FF #(.GSR(\"ENABLED\"),.SRMODE(\"LSR_OVER_CE\")) observer(.CLK(clk),.LSR(reset),.CE(1'b1),.DI(p0[0]),.Q(captured));\nendmodule\n");
    fs::write(dir.join("top.v"), hdl).unwrap();
    run(
        &dir,
        "yosys",
        &[
            "-p",
            "read_verilog top.v; synth_ecp5 -top top -json top.json",
        ],
        "yosys.log",
    );
    run(
        &dir,
        "nextpnr-ecp5",
        &[
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
        "nextpnr.log",
    );
    run(
        &dir,
        "ecppack",
        &["--compress", "top.config", "top.bit"],
        "ecppack.log",
    );
    let pin = |name: &str| Reference::Pin(pins[name].clone());
    let sign = |side: usize| {
        Reference::Wire(
            model
                .resource_ports(multipliers[side])
                .into_iter()
                .find(|p| p.name == "SIGNEDP")
                .unwrap()
                .wire,
        )
    };
    let mut reports = Vec::new();
    let mut baseline = None;
    for (extension, scope) in [
        ("config", Scope::Observed),
        ("bit", Scope::Observed),
        ("bit", Scope::AllPlcRegisters),
    ] {
        let image = Arc::new(
            ConfigurationImage::load(&dir.join(format!("top.{extension}")), &db, None).unwrap(),
        );
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let mut inputs = BTreeMap::from([
            ("sa".to_string(), Zero),
            ("sb".into(), Zero),
            ("clk".into(), Zero),
            ("reset".into(), Zero),
        ]);
        for bus in ["a", "b"] {
            for bit in 0..18 {
                inputs.insert(format!("{bus}[{bit}]"), Zero);
            }
        }
        let mut probes = pins
            .keys()
            .filter(|p| p.starts_with("p0[") || p.starts_with("p1[") || p.as_str() == "captured")
            .map(|p| pin(p))
            .collect::<Vec<_>>();
        probes.extend([sign(0), sign(1)]);
        let mut c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope,
                startup: Startup::ReadyUnknown,
                drives: inputs.iter().map(|(p, &v)| (pin(p), v)).collect(),
                probes,
                options: Options::default(),
            },
        )
        .unwrap_or_else(|e| panic!("{extension} {scope:?}: {e:#}"));
        assert_eq!(c.report.counts.multiplier18, 2);
        assert_eq!(c.kernel.state().summary().dsp_slices, 78);
        assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 0);
        let mut random = 0x709b4631u64;
        let mut captured = X;
        let mut samples = Vec::new();
        let mut events = Vec::new();
        for t in 0..=512u64 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = if t < 6 {
                [0, 1, 131071, 131072, 262143, 262143][t as usize]
            } else {
                random & 262143
            };
            let b = if t < 6 {
                [262143, 262143, 131071, 131072, 262143, 1][t as usize]
            } else {
                (random >> 32) & 262143
            };
            let sa = t & 2 != 0;
            let sb = t & 4 != 0;
            let clk = t & 1 != 0;
            let reset = t % 41 == 9;
            for (name, value) in [("sa", sa), ("sb", sb), ("clk", clk), ("reset", reset)] {
                inputs.insert(name.into(), Logic::from_bool(value));
            }
            for (bus, word) in [("a", a), ("b", b)] {
                for bit in 0..18 {
                    inputs.insert(
                        format!("{bus}[{bit}]"),
                        Logic::from_bool(word & (1 << bit) != 0),
                    );
                }
            }
            let updates = inputs
                .iter()
                .map(|(p, &v)| (c.drivers[&pin(p).label()], v))
                .collect::<Vec<_>>();
            let time = Time::from_ps((t + 1) * 1000);
            c.kernel.schedule_batch(time, &updates, None).unwrap();
            c.kernel.run_until(time).unwrap();
            let expected = [product(a, b, sa, sb), product(a, b, false, false)];
            if clk {
                captured = if reset {
                    Zero
                } else {
                    Logic::from_bool(expected[0] & 1 != 0)
                };
            }
            for (side, &word) in expected.iter().enumerate() {
                for bit in 0..36 {
                    assert_eq!(
                        c.value(&pin(&format!("p{side}[{bit}]"))).unwrap(),
                        Logic::from_bool(word & (1 << bit) != 0),
                        "{extension} {scope:?} t={t} p{side}[{bit}]"
                    );
                }
            }
            assert_eq!(c.value(&sign(0)).unwrap(), Logic::from_bool(sa || sb));
            assert_eq!(c.value(&sign(1)).unwrap(), Zero);
            assert_eq!(c.value(&pin("captured")).unwrap(), captured);
            samples.push(json!({"time_ns":t+1,"a":a,"b":b,"sa":sa,"sb":sb,"p0":expected[0],"p1":expected[1],"captured":captured}));
            events.push(json!({"time":t+1,"set":inputs.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}));
        }
        if let Some(b) = &baseline {
            assert_eq!(&samples, b);
        } else {
            baseline = Some(samples.clone());
        }
        if extension == "bit" && matches!(scope, Scope::Observed) {
            fs::write(
                dir.join("events.json"),
                serde_json::to_vec_pretty(&events).unwrap(),
            )
            .unwrap();
            let mut args = vec![
                "emulate".to_string(),
                "top.bit".into(),
                "--startup".into(),
                "ready-unknown".into(),
                "--scope".into(),
                "observed".into(),
                "--stimulus".into(),
                "events.json".into(),
                "--output".into(),
                "cli.json".into(),
            ];
            for port in [
                "p0[0]", "p0[17]", "p0[35]", "p1[0]", "p1[17]", "p1[35]", "captured",
            ] {
                args.extend(["--probe".into(), pin(port).label()]);
            }
            run(
                &dir,
                env!("CARGO_BIN_EXE_ecp5_emu"),
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
                "cli.log",
            );
            let cli: serde_json::Value =
                serde_json::from_slice(&fs::read(dir.join("cli.json")).unwrap()).unwrap();
            assert_eq!(cli["observations"].as_array().unwrap().len(), 514);
            for (sample, actual) in samples
                .iter()
                .zip(cli["observations"].as_array().unwrap().iter().skip(1))
            {
                assert_eq!(sample["time_ns"], actual["time_ns"]);
                for side in 0..2 {
                    for bit in [0, 17, 35] {
                        let value = sample[format!("p{side}")].as_u64().unwrap();
                        assert_eq!(
                            actual["values"][pin(&format!("p{side}[{bit}]")).label()],
                            json!(Logic::from_bool(value & (1 << bit) != 0))
                        );
                    }
                }
                assert_eq!(
                    sample["captured"],
                    actual["values"][pin("captured").label()]
                );
            }
        }
        reports.push(json!({"source":extension,"scope":scope,"samples":samples,"counts":c.report.counts,"multipliers":c.report.multipliers,"resources":c.report.resources.iter().filter(|r|["MULT18","DSP_SLICE"].contains(&r.family.as_str())).collect::<Vec<_>>(),"excluded_dsp_routes":c.report.excluded_routes.iter().filter(|r|r.rule.contains("MULT18")).collect::<Vec<_>>(),"state":c.kernel.state().summary(),"statistics":c.kernel.statistics()}));
    }
    fs::write(
        dir.join("native.json"),
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"hardware_results":0,"pins":pins,"runs":reports}),
        )
        .unwrap(),
    )
    .unwrap();
}
