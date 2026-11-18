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
type Operand = Option<(u64, bool)>;
fn product(a: Operand, b: Operand) -> Operand {
    a.zip(b).map(|((a, sa), (b, sb))| {
        let operand = |v: u64, s| v as i128 - if s && v >= 131072 { 262144 } else { 0 };
        (
            (operand(a, sa) * operand(b, sb)).rem_euclid(1i128 << 36) as u64,
            sa || sb,
        )
    })
}
#[test]
#[ignore = "requires local Yosys/nextpnr/Trellis and native image reader"]
fn registered_mult18_profiles_decode_and_execute_from_config_and_bit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let block = model
        .resources()
        .find(|(_, r)| r.family == "DSP_BLOCK")
        .unwrap()
        .0;
    let multipliers = model
        .children(block)
        .iter()
        .flat_map(|&s| model.children(s))
        .copied()
        .filter(|&id| model.resource(id).family == "MULT18")
        .collect::<Vec<_>>();
    assert_eq!(multipliers.len(), 4);
    let forwarding = multipliers
        .iter()
        .map(|&id| {
            ["ROA", "ROB"].map(|bus| {
                (0..18)
                    .map(|bit| {
                        Reference::Wire(
                            model
                                .resource_ports(id)
                                .into_iter()
                                .find(|p| p.name == format!("{bus}{bit}"))
                                .unwrap()
                                .wire,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    // A, B, product only, and all three; both input/pipeline bypass and
    // independent controls exercised in one physically placed DSP block.
    let profiles = [1, 2, 4, 7];
    let mut pins = BTreeMap::from([("ck0".to_string(), "G2".to_string())]);
    let mut balls = model
        .package_pins()
        .filter(|p| p.package == "CABGA381" && p.pio.is_some() && p.ball != "G2")
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    for (name, width) in [
        ("a", 18),
        ("b", 18),
        ("p0", 36),
        ("p1", 36),
        ("p2", 36),
        ("p3", 36),
    ] {
        for bit in 0..width {
            pins.insert(format!("{name}[{bit}]"), balls.next().unwrap());
        }
    }
    for name in [
        "sa", "sb", "ck1", "ck2", "ce0", "ce1", "ce2", "rst0", "rst1", "rst2", "gsrn", "q0", "q1",
        "q2", "q3",
    ] {
        pins.insert(name.into(), balls.next().unwrap());
    }
    let lpf=pins.iter().map(|(p,b)|format!("LOCATE COMP \"{p}\" SITE \"{b}\";\nIOBUF PORT \"{p}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect::<String>();
    for mode in ["SYNC", "ASYNC"] {
        let dir = root.join(format!("target/dsp-registers-{}", mode.to_lowercase()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("top.lpf"), &lpf).unwrap();
        let mut hdl=String::from("module top(input [17:0] a,b,input sa,sb,ck0,ck1,ck2,ce0,ce1,ce2,rst0,rst1,rst2,gsrn,output [35:0] p0,p1,p2,p3,output q0,q1,q2,q3);\nGSR global_reset(.GSR(gsrn));\n");
        for (i, &id) in multipliers.iter().enumerate() {
            let r = model.resource(id);
            let mask = profiles[i];
            let mut ports = Vec::new();
            let mut params = vec![
                format!(".RESETMODE(\"{mode}\")"),
                format!(".GSR(\"{}\")", if i < 2 { "ENABLED" } else { "DISABLED" }),
            ];
            for bit in 0..18 {
                ports.push(format!(".A{bit}(a[{bit}]),.B{bit}(b[{bit}])"));
            }
            for bit in 0..36 {
                ports.push(format!(".P{bit}(p{i}[{bit}])"));
            }
            ports.extend([
                ".SIGNEDA(sa)".into(),
                ".SIGNEDB(sb)".into(),
                ".SOURCEA(1'b0)".into(),
                ".SOURCEB(1'b0)".into(),
                ".SIGNEDP()".into(),
            ]);
            for (n, stage) in ["INPUTA", "INPUTB", "PIPELINE"].iter().enumerate() {
                params.extend([
                    format!(
                        ".REG_{stage}_CLK(\"{}\")",
                        if mask & (1 << n) != 0 {
                            format!("CLK{n}")
                        } else {
                            "NONE".into()
                        }
                    ),
                    format!(".REG_{stage}_CE(\"CE{n}\")"),
                    format!(".REG_{stage}_RST(\"RST{n}\")"),
                ]);
                ports.push(format!(".CLK{n}(ck{n}),.CE{n}(ce{n}),.RST{n}(rst{n})"));
            }
            ports.push(".CLK3(1'b0),.CE3(1'b0),.RST3(1'b0)".into());
            hdl.push_str(&format!(
                "(* BEL=\"X{}/Y{}/{}\" *) MULT18X18D #({}) m{i}({});\n",
                r.location.col,
                r.location.row,
                r.name,
                params.join(","),
                ports.join(",")
            ));
            hdl.push_str(&format!("TRELLIS_FF #(.GSR(\"DISABLED\"),.SRMODE(\"LSR_OVER_CE\")) observer{i}(.CLK(ck2),.LSR(rst2),.CE(1'b1),.DI(p{i}[0]),.Q(q{i}));\n"));
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
        let pin = |s: &str| Reference::Pin(pins[s].clone());
        let sign = |i: usize| {
            Reference::Wire(
                model
                    .resource_ports(multipliers[i])
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
                ("gsrn".into(), One),
            ]);
            for prefix in ["ck", "ce", "rst"] {
                for n in 0..3 {
                    inputs.insert(
                        format!("{prefix}{n}"),
                        if prefix == "ce" { One } else { Zero },
                    );
                }
            }
            for bus in ["a", "b"] {
                for n in 0..18 {
                    inputs.insert(format!("{bus}[{n}]"), Zero);
                }
            }
            let mut probes = pins
                .keys()
                .filter(|p| p.starts_with('p') || p.starts_with('q'))
                .map(|p| pin(p))
                .collect::<Vec<_>>();
            probes.extend((0..4).map(sign));
            probes.extend(forwarding.iter().flatten().flatten().cloned());
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
            .unwrap_or_else(|e| panic!("{mode} {extension} {scope:?}: {e:#}"));
            assert_eq!(c.report.counts.multiplier18, 4);
            for profile in &c.report.multipliers {
                assert!(!profile.stages.is_empty());
                assert_eq!(profile.behavioural_assumptions.len(), 3);
            }
            let mut ar: [Operand; 4] = [None; 4];
            let mut br = ar;
            let mut pr = ar;
            let mut ff = [X; 4];
            let mut previous = [false; 3];
            let mut random = 0x72d43197u64;
            let mut samples = Vec::new();
            let mut events = Vec::new();
            for t in 0..513u64 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let a = random & 262143;
                let b = (random >> 32) & 262143;
                let sa = t & 2 != 0;
                let sb = t & 4 != 0;
                let clocks = [t & 1 != 0, (t / 3) & 1 != 0, (t / 5) & 1 != 0];
                let ce = [t % 7 != 1, t % 11 != 2, t % 13 != 3];
                let rst = [t % 23 == 5, t % 29 == 7, t % 31 == 9];
                let gsr = t % 47 == 11;
                inputs.insert("sa".into(), Logic::from_bool(sa));
                inputs.insert("sb".into(), Logic::from_bool(sb));
                inputs.insert("gsrn".into(), Logic::from_bool(!gsr));
                for (bus, value) in [("a", a), ("b", b)] {
                    for bit in 0..18 {
                        inputs.insert(
                            format!("{bus}[{bit}]"),
                            Logic::from_bool(value & (1 << bit) != 0),
                        );
                    }
                }
                for n in 0..3 {
                    for (prefix, value) in [("ck", clocks[n]), ("ce", ce[n]), ("rst", rst[n])] {
                        inputs.insert(format!("{prefix}{n}"), Logic::from_bool(value));
                    }
                }
                let edges: [bool; 3] = std::array::from_fn(|n| !previous[n] && clocks[n]);
                let mut expected = [None; 4];
                for i in 0..4 {
                    let mask = profiles[i];
                    let av = if mask & 1 != 0 { ar[i] } else { Some((a, sa)) };
                    let bv = if mask & 2 != 0 { br[i] } else { Some((b, sb)) };
                    let old_output = if mask & 4 != 0 {
                        pr[i]
                    } else {
                        product(av, bv)
                    };
                    if edges[2] {
                        ff[i] = if rst[2] {
                            Zero
                        } else {
                            old_output.map_or(X, |(p, _)| Logic::from_bool(p & 1 != 0))
                        };
                    }
                    let next = |n: usize, old, data| {
                        if (i < 2 && gsr) || (rst[n] && (mode == "ASYNC" || edges[n])) {
                            Some((0, false))
                        } else if edges[n] && ce[n] {
                            data
                        } else {
                            old
                        }
                    };
                    if mask & 4 != 0 {
                        pr[i] = next(2, pr[i], product(av, bv));
                    }
                    if mask & 1 != 0 {
                        ar[i] = next(0, ar[i], Some((a, sa)));
                    }
                    if mask & 2 != 0 {
                        br[i] = next(1, br[i], Some((b, sb)));
                    }
                    expected[i] = if mask & 4 != 0 {
                        pr[i]
                    } else {
                        product(
                            if mask & 1 != 0 { ar[i] } else { Some((a, sa)) },
                            if mask & 2 != 0 { br[i] } else { Some((b, sb)) },
                        )
                    };
                }
                previous = clocks;
                let time = Time::from_ps((t + 1) * 1000);
                let changes = inputs
                    .iter()
                    .map(|(p, &v)| (c.drivers[&pin(p).label()], v))
                    .collect::<Vec<_>>();
                c.kernel.schedule_batch(time, &changes, None).unwrap();
                c.kernel.run_until(time).unwrap();
                for i in 0..4 {
                    let mask = profiles[i];
                    for (side, operand) in [
                        if mask & 1 != 0 { ar[i] } else { Some((a, sa)) },
                        if mask & 2 != 0 { br[i] } else { Some((b, sb)) },
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        for (bit, reference) in forwarding[i][side].iter().enumerate() {
                            assert_eq!(c.value(reference).unwrap(), operand.map_or(X, |(v, _)| Logic::from_bool(v & (1 << bit) != 0)), "{mode} {extension} {scope:?} t={t} mult={i} forwarding side={side} bit={bit}");
                        }
                    }
                    if let Some((word, signvalue)) = expected[i] {
                        for bit in 0..36 {
                            assert_eq!(
                                c.value(&pin(&format!("p{i}[{bit}]"))).unwrap(),
                                Logic::from_bool(word & (1 << bit) != 0),
                                "{mode} {extension} {scope:?} t={t} p{i}[{bit}]"
                            );
                        }
                        assert_eq!(
                            c.value(&sign(i)).unwrap(),
                            Logic::from_bool(signvalue),
                            "{mode} t={t} sign{i}"
                        );
                    }
                    if ff[i] != X {
                        assert_eq!(
                            c.value(&pin(&format!("q{i}"))).unwrap(),
                            ff[i],
                            "{mode} t={t} q{i}"
                        );
                    }
                }
                let values = c
                    .probes
                    .iter()
                    .map(|(name, &signal)| (name.clone(), c.kernel.value(signal).unwrap()))
                    .collect::<BTreeMap<_, _>>();
                samples.push(
                    json!({"time_ns":t+1,"expected":expected,"ff_expected":ff,"values":values}),
                );
                events.push(json!({"time":t+1,"set":inputs.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}));
            }
            assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 0);
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
                let mut probed = (0..4)
                    .flat_map(|i| [format!("p{i}[0]"), format!("p{i}[35]"), format!("q{i}")])
                    .map(|p| pin(&p))
                    .collect::<Vec<_>>();
                probed.extend(
                    forwarding
                        .iter()
                        .flatten()
                        .flat_map(|bus| [bus[0].clone(), bus[17].clone()]),
                );
                for p in &probed {
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
                assert_eq!(cli["observations"].as_array().unwrap().len(), 514);
                for (sample, actual) in samples
                    .iter()
                    .zip(cli["observations"].as_array().unwrap().iter().skip(1))
                {
                    for p in &probed {
                        let label = p.label();
                        assert_eq!(actual["values"][&label], sample["values"][&label]);
                    }
                }
            }
            reports.push(json!({"source":extension,"scope":scope,"counts":c.report.counts,"multipliers":c.report.multipliers,"statistics":c.kernel.statistics(),"state":c.kernel.state().summary(),"samples":samples}));
        }
        fs::write(dir.join("native.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"hardware_results":0,"mode":mode,"profiles":profiles,"pins":pins,"runs":reports})).unwrap()).unwrap();
    }
}

/// Fast mode against the event kernel on the register benches
/// (target/dsp-registers-sync and -async, written by the test above): INPUTA,
/// INPUTB, PIPELINE and all three, on independent clocks with CE/RST and a
/// user GSR from a pin. The bench's own clock ck0 becomes the CLI clock, and
/// its 513 stimulus events drive the rest, resets at clock edges included.
/// Both executors write the same VCD of all four products and the four
/// observing flip-flops.
#[test]
#[ignore = "requires clang, target/dsp-registers-{sync,async} from the test above, the native image reader and a local Trellis database"]
fn fast_mode_writes_the_kernel_vcd_on_the_register_benches() {
    for mode in ["sync", "async"] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join(format!("target/dsp-registers-{mode}"));
        let native: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("native.json")).unwrap()).unwrap();
        let pins = native["pins"].as_object().unwrap();
        let clock = format!("pin:{}", pins["ck0"].as_str().unwrap());
        // The bench raises ck0 at even nanoseconds; the CLI clock (2 ns)
        // rises at odd ones, so every event moves 1 ns earlier.
        let events: Vec<serde_json::Value> =
            serde_json::from_slice(&fs::read(dir.join("events.json")).unwrap()).unwrap();
        let events: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                let mut set = e["set"].as_object().unwrap().clone();
                set.remove(&clock);
                json!({"time": e["time"].as_u64().unwrap() - 1, "set": set})
            })
            .collect();
        let stimulus = dir.join("fast-events.json");
        fs::write(&stimulus, serde_json::to_vec(&events).unwrap()).unwrap();
        let mut probes = Vec::new();
        for (name, ball) in pins {
            if ["p0", "p1", "p2", "p3", "q0", "q1", "q2", "q3"]
                .contains(&name.split('[').next().unwrap())
            {
                probes.extend([
                    "--probe".to_string(),
                    format!("pin:{}", ball.as_str().unwrap()),
                ]);
            }
        }
        assert_eq!(probes.len(), 2 * (4 * 36 + 4));
        let vcd = |fast: bool| {
            let file = dir.join(if fast { "fast.vcd" } else { "kernel.vcd" });
            let mut args: Vec<String> = [
                "emulate",
                "top.config",
                "--startup",
                "configured",
                "--scope",
                "observed",
                "--stimulus",
                stimulus.to_str().unwrap(),
                "--clock",
                &clock,
                "--cycles",
                "257",
                "--vcd",
                file.to_str().unwrap(),
            ]
            .map(String::from)
            .to_vec();
            args.extend(probes.iter().cloned());
            if fast {
                args.push("--fast".into());
            }
            run(
                &dir,
                env!("CARGO_BIN_EXE_ecp5_emu"),
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
                if fast { "fast.log" } else { "kernel.log" },
            );
            fs::read(file).unwrap()
        };
        let kernel = vcd(false);
        let fast = vcd(true);
        assert!(fs::read_to_string(dir.join("fast.log"))
            .unwrap()
            .contains("Fast mode ran"));
        assert!(kernel == fast, "{mode}: the VCD files differ");
    }
}
