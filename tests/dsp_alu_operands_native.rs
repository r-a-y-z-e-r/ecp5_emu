//! Exercise the observable input stages of a real MULT18+ALU54 configuration.
//! No ALU arithmetic or flags are supplied by the emulator in this profile.
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

#[test]
#[ignore = "requires local Yosys/nextpnr/Trellis and native image reader"]
fn active_alu_slices_allow_operand_taps_without_product_or_alu_drivers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let block = model
        .resources()
        .find(|(_, r)| r.family == "DSP_BLOCK")
        .unwrap()
        .0;
    let mut slices = model.children(block).to_vec();
    slices.sort_by_key(|&id| model.resource(id).location.col);
    let mults = slices
        .iter()
        .flat_map(|&s| model.children(s))
        .copied()
        .filter(|&id| model.resource(id).family == "MULT18")
        .collect::<Vec<_>>();
    let alus = slices
        .iter()
        .flat_map(|&s| model.children(s))
        .copied()
        .filter(|&id| model.resource(id).family == "ALU54")
        .collect::<Vec<_>>();
    assert_eq!((mults.len(), alus.len()), (4, 2));
    let port = |id, name: &str| {
        Reference::Wire(
            model
                .resource_ports(id)
                .into_iter()
                .find(|p| p.name == name)
                .unwrap()
                .wire,
        )
    };
    let forwarding = mults
        .iter()
        .map(|&id| {
            ["ROA", "ROB"].map(|bus| {
                (0..18)
                    .map(|bit| port(id, &format!("{bus}{bit}")))
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let mut pins = BTreeMap::from([("ck0".to_string(), "G2".to_string())]);
    let mut balls = model
        .package_pins()
        .filter(|p| p.package == "CABGA381" && p.pio.is_some() && p.ball != "G2")
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    for (bus, width) in [("a", 18), ("b", 18), ("c", 54), ("op", 11)] {
        for bit in 0..width {
            pins.insert(format!("{bus}[{bit}]"), balls.next().unwrap());
        }
    }
    for name in [
        "ck1", "ck2", "ck3", "ce0", "ce1", "rst0", "rst1", "sa", "sb", "r0", "r1", "r2", "r3",
        "eq0", "eq1",
    ] {
        pins.insert(name.into(), balls.next().unwrap());
    }
    let lpf=pins.iter().map(|(p,b)|format!("LOCATE COMP \"{p}\" SITE \"{b}\";\nIOBUF PORT \"{p}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect::<String>();
    let masks = [0, 1, 2, 3];
    for mode in ["SYNC", "ASYNC"] {
        let dir = root.join(format!("target/dsp-alu-operands-{}", mode.to_lowercase()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("top.lpf"), &lpf).unwrap();
        let mut hdl=String::from("module top(input [17:0] a,b,input [53:0] c,input [10:0] op,input ck0,ck1,ck2,ck3,ce0,ce1,rst0,rst1,sa,sb,output r0,r1,r2,r3,eq0,eq1);\n");
        for (i, &id) in mults.iter().enumerate() {
            let r = model.resource(id);
            hdl.push_str(&format!(
                "wire [35:0] p{i};wire [17:0] ra{i},rb{i};wire s{i};\n"
            ));
            let mut params = vec![
                format!(".RESETMODE(\"{mode}\")"),
                ".GSR(\"DISABLED\")".into(),
            ];
            for (n, stage) in ["INPUTA", "INPUTB"].into_iter().enumerate() {
                params.extend([
                    format!(
                        ".REG_{stage}_CLK(\"{}\")",
                        if masks[i] & (1 << n) != 0 {
                            format!("CLK{n}")
                        } else {
                            "NONE".into()
                        }
                    ),
                    format!(".REG_{stage}_CE(\"CE{n}\")"),
                    format!(".REG_{stage}_RST(\"RST{n}\")"),
                ]);
            }
            let mut ports = Vec::new();
            for bit in 0..18 {
                ports.push(format!(".A{bit}(a[{bit}]),.B{bit}(b[{bit}]),.ROA{bit}(ra{i}[{bit}]),.ROB{bit}(rb{i}[{bit}])"));
            }
            for bit in 0..36 {
                ports.push(format!(".P{bit}(p{i}[{bit}])"));
            }
            ports.push(format!(
                ".SIGNEDA(sa),.SIGNEDB(sb),.SIGNEDP(s{i}),.SOURCEA(1'b0),.SOURCEB(1'b0)"
            ));
            ports.push(".CLK0(ck0),.CLK1(ck1),.CLK2(ck2),.CLK3(ck3),.CE0(ce0),.CE1(ce1),.CE2(1'b1),.CE3(1'b1),.RST0(rst0),.RST1(rst1),.RST2(1'b0),.RST3(1'b0)".into());
            hdl.push_str(&format!(
                "(* BEL=\"X{}/Y{}/{}\" *) MULT18X18D #({}) m{i}({});\n",
                r.location.col,
                r.location.row,
                r.name,
                params.join(","),
                ports.join(",")
            ));
        }
        for (i, &id) in alus.iter().enumerate() {
            let r = model.resource(id);
            let mut ports = Vec::new();
            let first = i * 2;
            let second = first + 1;
            for bit in 0..36 {
                let (bus, n) = if bit < 18 {
                    ("ra", bit)
                } else {
                    ("rb", bit - 18)
                };
                ports.push(format!(".A{bit}({bus}{first}[{n}]),.B{bit}({bus}{second}[{n}]),.MA{bit}(p{first}[{bit}]),.MB{bit}(p{second}[{bit}])"));
            }
            for bit in 0..54 {
                // Dedicated CIN/CFB chains are intentionally unconnected in
                // this operand-only fixture; no ALU result is an oracle.
                ports.push(format!(".C{bit}(c[{bit}])"));
            }
            for bit in 0..11 {
                ports.push(format!(".OP{bit}(op[{bit}])"));
            }
            ports.push(format!(
                ".SIGNEDIA(s{first}),.SIGNEDIB(s{second}),.R0(r{}),.R53(r{}),.EQZ(eq{i})",
                i * 2,
                i * 2 + 1
            ));
            ports.push(".CLK0(ck0),.CLK1(ck1),.CLK2(ck2),.CLK3(ck3),.CE0(ce0),.CE1(ce1),.CE2(1'b1),.CE3(1'b1),.RST0(rst0),.RST1(rst1),.RST2(1'b0),.RST3(1'b0)".into());
            let params = if i == 0 {
                format!(".RESETMODE(\"{mode}\"),.GSR(\"DISABLED\"),.REG_OUTPUT0_CLK(\"CLK2\"),.REG_OUTPUT1_CLK(\"CLK3\"),.REG_FLAG_CLK(\"CLK2\")")
            } else {
                format!(".RESETMODE(\"{mode}\"),.GSR(\"DISABLED\")")
            };
            hdl.push_str(&format!(
                "(* BEL=\"X{}/Y{}/{}\" *) ALU54B #({params}) alu{i}({});\n",
                r.location.col,
                r.location.row,
                r.name,
                ports.join(",")
            ));
        }
        hdl.push_str("endmodule\n");
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
        let mut reports = Vec::new();
        let mut baseline = None;
        for extension in ["config", "bit"] {
            let device = Arc::new(
                ConfiguredDevice::bind(
                    Arc::clone(&model),
                    Arc::new(
                        ConfigurationImage::load(&dir.join(format!("top.{extension}")), &db, None)
                            .unwrap(),
                    ),
                    load_catalogue(&model, &evidence).unwrap(),
                )
                .unwrap(),
            );
            let mut inputs = BTreeMap::new();
            for (bus, width) in [("a", 18), ("b", 18), ("c", 54), ("op", 11)] {
                for bit in 0..width {
                    inputs.insert(format!("{bus}[{bit}]"), Zero);
                }
            }
            for name in [
                "ck0", "ck1", "ck2", "ck3", "ce0", "ce1", "rst0", "rst1", "sa", "sb",
            ] {
                inputs.insert(name.into(), Logic::from_bool(name.starts_with("ce")));
            }
            let request = |scope, probes| Request {
                package: "CABGA381".into(),
                scope,
                startup: Startup::ReadyUnknown,
                drives: inputs.iter().map(|(p, &v)| (pin(p), v)).collect(),
                probes,
                options: Options::default(),
            };
            let mut probes = forwarding
                .iter()
                .flatten()
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            let mut c = compile::compile(
                Arc::clone(&device),
                request(Scope::Observed, probes.clone()),
            )
            .unwrap_or_else(|e| panic!("{mode} {extension}: {e:#}"));
            assert_eq!(c.report.counts.multiplier18, 4);
            assert!(c.report.resources.iter().all(|r| r.family != "ALU54"));
            // The left ALU has its output registers on two clocks, which is
            // not a measured profile: its multipliers give operands only. The
            // right ALU, with unregistered outputs, is one the planner can
            // execute since HD04, so its multipliers have their products too
            // (for MA and MB); nothing observes its R here.
            let mut views = BTreeMap::new();
            for p in &c.report.multipliers {
                *views.entry(p.execution_view).or_insert(0) += 1;
                assert!(p.output_register.is_none());
                assert!(p
                    .stages
                    .iter()
                    .all(|s| ["INPUTA", "INPUTB"].contains(&s.stage.as_str())));
            }
            assert_eq!(
                views,
                BTreeMap::from([("operands_only", 2), ("product_and_operands", 2)])
            );
            for &id in &alus {
                assert_eq!(device.resource_settings(id).unwrap().enabled, Some(true));
            }
            for unwanted in [
                port(mults[0], "P0"),
                port(mults[0], "SIGNEDP"),
                port(alus[0], "R0"),
                pin("r0"),
            ] {
                probes.push(unwanted);
                assert!(compile::compile(
                    Arc::clone(&device),
                    request(Scope::Observed, probes.clone())
                )
                .is_err());
                probes.pop();
            }
            assert!(
                compile::compile(Arc::clone(&device), request(Scope::AllPlcRegisters, probes))
                    .is_err()
            );
            let mut ar = [None; 4];
            let mut br = ar;
            let mut previous = [false; 2];
            let mut random = 0x9d1743u64;
            let mut samples = Vec::new();
            let mut events = Vec::new();
            for t in 0..257u64 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let a = random & 262143;
                let b = (random >> 32) & 262143;
                let clocks = [t & 1 != 0, (t / 3) & 1 != 0];
                let ce = [t % 7 != 1, t % 11 != 2];
                let rst = [t % 23 == 5, t % 29 == 7];
                for (bus, width, value) in [
                    ("a", 18, a),
                    ("b", 18, b),
                    ("c", 54, random & ((1 << 54) - 1)),
                    ("op", 11, random & 2047),
                ] {
                    for bit in 0..width {
                        inputs.insert(
                            format!("{bus}[{bit}]"),
                            Logic::from_bool(value & (1 << bit) != 0),
                        );
                    }
                }
                inputs.insert("sa".into(), Logic::from_bool(t & 2 != 0));
                inputs.insert("sb".into(), Logic::from_bool(t & 4 != 0));
                for n in 0..2 {
                    for (prefix, value) in [("ck", clocks[n]), ("ce", ce[n]), ("rst", rst[n])] {
                        inputs.insert(format!("{prefix}{n}"), Logic::from_bool(value));
                    }
                }
                inputs.insert("ck2".into(), Logic::from_bool((t / 5) & 1 != 0));
                inputs.insert("ck3".into(), Logic::from_bool((t / 7) & 1 != 0));
                for i in 0..4 {
                    for (side, register) in [&mut ar, &mut br].into_iter().enumerate() {
                        if masks[i] & (1 << side) == 0 {
                            continue;
                        }
                        let edge = !previous[side] && clocks[side];
                        if rst[side] && (mode == "ASYNC" || edge) {
                            register[i] = Some(0);
                        } else if edge && ce[side] {
                            register[i] = Some(if side == 0 { a } else { b });
                        }
                    }
                }
                previous = clocks;
                let time = Time::from_ps((t + 1) * 1000);
                c.kernel
                    .schedule_batch(
                        time,
                        &inputs
                            .iter()
                            .map(|(p, &v)| (c.drivers[&pin(p).label()], v))
                            .collect::<Vec<_>>(),
                        None,
                    )
                    .unwrap();
                c.kernel.run_until(time).unwrap();
                for i in 0..4 {
                    for (side, operand) in [
                        if masks[i] & 1 != 0 { ar[i] } else { Some(a) },
                        if masks[i] & 2 != 0 { br[i] } else { Some(b) },
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        for (bit, p) in forwarding[i][side].iter().enumerate() {
                            assert_eq!(
                                c.value(p).unwrap(),
                                operand.map_or(X, |v| Logic::from_bool(v & (1 << bit) != 0)),
                                "{mode} {extension} t={t} lane={i} side={side} bit={bit}"
                            );
                        }
                    }
                }
                samples.push(json!({"time_ns":t+1,"values":c.probes.iter().map(|(p,&s)|(p.clone(),c.kernel.value(s).unwrap())).collect::<BTreeMap<_,_>>()}));
                events.push(json!({"time":t+1,"set":inputs.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}));
            }
            assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 0);
            for &id in &mults {
                assert_eq!(c.kernel.state().multiplier(id).unwrap().product, [X; 36]);
            }
            if let Some(b) = &baseline {
                assert_eq!(&samples, b);
            } else {
                baseline = Some(samples.clone());
            }
            if extension == "bit" {
                fs::write(
                    dir.join("events.json"),
                    serde_json::to_vec_pretty(&events).unwrap(),
                )
                .unwrap();
                let probes = forwarding
                    .iter()
                    .flatten()
                    .flat_map(|bus| [bus[0].clone(), bus[17].clone()])
                    .collect::<Vec<_>>();
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
                for p in &probes {
                    args.extend(["--probe".into(), p.label()]);
                }
                run(
                    &dir,
                    env!("CARGO_BIN_EXE_ecp5_emu"),
                    &args.iter().map(String::as_str).collect::<Vec<_>>(),
                    "cli.log",
                );
                let cli: serde_json::Value =
                    serde_json::from_slice(&fs::read(dir.join("cli.json")).unwrap()).unwrap();
                assert_eq!(cli["observations"].as_array().unwrap().len(), 258);
                for (s, o) in samples
                    .iter()
                    .zip(cli["observations"].as_array().unwrap().iter().skip(1))
                {
                    for p in &probes {
                        let key = p.label();
                        assert_eq!(s["values"][&key], o["values"][&key]);
                    }
                }
            }
            reports.push(json!({"source":extension,"counts":c.report.counts,"multipliers":c.report.multipliers,"statistics":c.kernel.statistics(),"state":c.kernel.state().summary(),"alu_settings":alus.iter().map(|&id|device.resource_settings(id).unwrap()).collect::<Vec<_>>(),"samples":samples}));
        }
        fs::write(dir.join("native.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"mode":mode,"hardware_results":0,"profiles":masks,"runs":reports,"pins":pins})).unwrap()).unwrap();
    }
}
