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
    let result = tools::command(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    fs::write(dir.join(log), [result.stdout, result.stderr].concat()).unwrap();
    assert!(
        result.status.success(),
        "{program}: see {}",
        dir.join(log).display()
    );
}
type Operand = Option<(u64, bool)>;
fn product(a: Operand, b: Operand) -> Option<u64> {
    a.zip(b).map(|((a, sa), (b, sb))| {
        let signed = |v: u64, s| v as i128 - if s && v >= 131072 { 262144 } else { 0 };
        (signed(a, sa) * signed(b, sb)).rem_euclid(1i128 << 36) as u64
    })
}
#[test]
#[ignore = "requires local Yosys/nextpnr/Trellis and native image reader"]
fn shared_mult_output_registers_use_all_four_clocks_and_preserve_peer_lanes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let block = model
        .resources()
        .find(|(_, r)| r.family == "DSP_BLOCK")
        .unwrap()
        .0;
    let mults = model
        .children(block)
        .iter()
        .flat_map(|&s| model.children(s))
        .copied()
        .filter(|&id| model.resource(id).family == "MULT18")
        .collect::<Vec<_>>();
    assert_eq!(mults.len(), 4);
    let masks = [0, 3, 4, 7];
    let dir = root.join("target/dsp-output");
    fs::create_dir_all(&dir).unwrap();
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
        for n in 0..width {
            pins.insert(format!("{name}[{n}]"), balls.next().unwrap());
        }
    }
    for name in [
        "sa", "sb", "ck1", "ck2", "ck3", "ce0", "ce1", "ce2", "ce3", "rst0", "rst1", "rst2",
        "rst3", "q0", "q1", "q2", "q3",
    ] {
        pins.insert(name.into(), balls.next().unwrap());
    }
    fs::write(dir.join("top.lpf"),pins.iter().map(|(p,b)|format!("LOCATE COMP \"{p}\" SITE \"{b}\";\nIOBUF PORT \"{p}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect::<String>()).unwrap();
    let mut hdl=String::from("module top(input [17:0] a,b,input sa,sb,ck0,ck1,ck2,ck3,ce0,ce1,ce2,ce3,rst0,rst1,rst2,rst3,output [35:0] p0,p1,p2,p3,output q0,q1,q2,q3);\n");
    for (i, &id) in mults.iter().enumerate() {
        let r = model.resource(id);
        let mut params = vec![
            ".GSR(\"DISABLED\")".into(),
            format!(".REG_OUTPUT_CLK(\"CLK{i}\")"),
        ];
        let mut ports = vec![
            ".SIGNEDA(sa)".into(),
            ".SIGNEDB(sb)".into(),
            ".SOURCEA(1'b0)".into(),
            ".SOURCEB(1'b0)".into(),
            ".SIGNEDP()".into(),
        ];
        for (n, stage) in ["INPUTA", "INPUTB", "PIPELINE"].iter().enumerate() {
            params.push(format!(
                ".REG_{stage}_CLK(\"{}\")",
                if masks[i] & (1 << n) != 0 {
                    format!("CLK{n}")
                } else {
                    "NONE".into()
                }
            ));
        }
        for n in 0..4 {
            ports.push(format!(".CLK{n}(ck{n}),.CE{n}(ce{n}),.RST{n}(rst{n})"));
        }
        for n in 0..18 {
            ports.push(format!(".A{n}(a[{n}]),.B{n}(b[{n}])"));
        }
        for n in 0..36 {
            ports.push(format!(".P{n}(p{i}[{n}])"));
        }
        hdl.push_str(&format!(
            "(* BEL=\"X{}/Y{}/{}\" *) MULT18X18D #({}) m{i}({});\n",
            r.location.col,
            r.location.row,
            r.name,
            params.join(","),
            ports.join(",")
        ));
        hdl.push_str(&format!("TRELLIS_FF #(.GSR(\"DISABLED\")) observer{i}(.CLK(ck{i}),.CE(1'b1),.LSR(rst0),.DI(p{i}[0]),.Q(q{i}));\n"));
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
        let mut inputs = BTreeMap::from([("sa".to_string(), Zero), ("sb".into(), Zero)]);
        for prefix in ["ck", "ce", "rst"] {
            for n in 0..4 {
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
        let forwarding = mults
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
        let mut probes = pins
            .keys()
            .filter(|p| p.starts_with('p') || p.starts_with('q'))
            .map(|p| pin(p))
            .collect::<Vec<_>>();
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
        .unwrap_or_else(|e| panic!("{extension} {scope:?}: {e:#}"));
        fs::write(
            dir.join(format!("plan-{extension}-{scope:?}.json")),
            serde_json::to_vec_pretty(&c.report).unwrap(),
        )
        .unwrap();
        assert_eq!(c.report.counts.multiplier18, 4);
        assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 0);
        // Every physical slice has two disjoint logical views; all 156 views
        // together cover the documented 78 x 72-bit banks exactly once.
        let mut ranges = BTreeMap::new();
        for (id, resource) in model.resources().filter(|(_, r)| r.family == "MULT18") {
            let (owner, range) = c.kernel.state().multiplier_output_view(id).unwrap();
            let entry = ranges
                .entry(model.resource(owner).key.clone())
                .or_insert_with(|| vec![false; 72]);
            for bit in range {
                assert!(!entry[bit], "{} duplicate output bit", resource.key);
                entry[bit] = true;
            }
        }
        assert_eq!(ranges.len(), 78);
        assert!(ranges.values().all(|bits| bits.iter().all(|&v| v)));
        for (i, r) in c.report.multipliers.iter().enumerate() {
            let output = r.output_register.as_ref().unwrap();
            let (_, range) = c.kernel.state().multiplier_output_view(mults[i]).unwrap();
            assert_eq!(output.logical_bit_start, range.start);
            assert_eq!(output.width, 36);
            assert!(!r.gsr_enabled);
        }
        let mut ar: [Operand; 4] = [None; 4];
        let mut br = ar;
        let mut pr = [None; 4];
        let mut qr = [None; 4];
        let mut ff = [X; 4];
        let mut previous = [false; 4];
        let mut random = 0x37bd814a09u64;
        let mut samples = Vec::new();
        let mut events = vec![
            json!({"time":0,"set":inputs.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}),
        ];
        for t in 0..513u64 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = random & 262143;
            let b = (random >> 32) & 262143;
            let sa = t & 2 != 0;
            let sb = t & 4 != 0;
            let clocks = [
                t & 1 != 0,
                (t / 3) & 1 != 0,
                (t / 5) & 1 != 0,
                (t / 7) & 1 != 0,
            ];
            let edges = std::array::from_fn::<_, 4, _>(|n| !previous[n] && clocks[n]);
            inputs.insert("sa".into(), Logic::from_bool(sa));
            inputs.insert("sb".into(), Logic::from_bool(sb));
            for (bus, word) in [("a", a), ("b", b)] {
                for n in 0..18 {
                    inputs.insert(
                        format!("{bus}[{n}]"),
                        Logic::from_bool(word & (1 << n) != 0),
                    );
                }
            }
            for (n, &clock) in clocks.iter().enumerate() {
                inputs.insert(format!("ck{n}"), Logic::from_bool(clock));
            }
            for i in 0..4 {
                let mask = masks[i];
                let av = if mask & 1 != 0 { ar[i] } else { Some((a, sa)) };
                let bv = if mask & 2 != 0 { br[i] } else { Some((b, sb)) };
                let p = product(av, bv);
                if edges[i] {
                    ff[i] = qr[i].map_or(X, |word| Logic::from_bool(word & 1 != 0));
                    qr[i] = if mask & 4 != 0 { pr[i] } else { p };
                }
                if mask & 4 != 0 && edges[2] {
                    pr[i] = p;
                }
                if mask & 1 != 0 && edges[0] {
                    ar[i] = Some((a, sa));
                }
                if mask & 2 != 0 && edges[1] {
                    br[i] = Some((b, sb));
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
                let mask = masks[i];
                for (side, operand) in [
                    if mask & 1 != 0 { ar[i] } else { Some((a, sa)) },
                    if mask & 2 != 0 { br[i] } else { Some((b, sb)) },
                ]
                .into_iter()
                .enumerate()
                {
                    for (bit, reference) in forwarding[i][side].iter().enumerate() {
                        assert_eq!(
                            c.value(reference).unwrap(),
                            operand.map_or(X, |(v, _)| Logic::from_bool(v & (1 << bit) != 0)),
                            "{extension} {scope:?} t={t} mult={i} forwarding side={side} bit={bit}"
                        );
                    }
                }
                let actual = c.kernel.state().multiplier_output(mults[i]).unwrap();
                if let Some(word) = qr[i] {
                    for (n, &actual_bit) in actual.iter().enumerate() {
                        let expected = Logic::from_bool(word & (1 << n) != 0);
                        assert_eq!(
                            actual_bit, expected,
                            "{extension} {scope:?} t={t} bank lane{i}[{n}]"
                        );
                        assert_eq!(c.value(&pin(&format!("p{i}[{n}]"))).unwrap(), expected);
                    }
                }
                if ff[i] != X {
                    assert_eq!(
                        c.value(&pin(&format!("q{i}"))).unwrap(),
                        ff[i],
                        "t={t} q{i}"
                    );
                }
            }
            let values = c
                .probes
                .iter()
                .map(|(name, &s)| (name.clone(), c.kernel.value(s).unwrap()))
                .collect::<BTreeMap<_, _>>();
            samples.push(json!({"time_ns":t+1,"a":a,"b":b,"sa":sa,"sb":sb,"expected_products":qr,"expected_ff":ff,"values":values}));
            events.push(json!({"time":t+1,"set":inputs.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}));
        }
        assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 144);
        assert_eq!(c.kernel.state().summary().dsp_shared_output_bits, 5616);
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
            let ports = (0..4)
                .flat_map(|i| {
                    [
                        format!("p{i}[0]"),
                        format!("p{i}[17]"),
                        format!("p{i}[35]"),
                        format!("q{i}"),
                    ]
                })
                .collect::<Vec<_>>();
            for p in &ports {
                args.extend(["--probe".into(), pin(p).label()]);
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
            for (s, actual) in samples
                .iter()
                .zip(cli["observations"].as_array().unwrap().iter().skip(1))
            {
                for p in &ports {
                    let label = pin(p).label();
                    assert_eq!(s["values"][&label], actual["values"][&label]);
                }
            }
        }
        reports.push(json!({"source":extension,"scope":scope,"counts":c.report.counts,"multipliers":c.report.multipliers,"state":c.kernel.state().summary(),"statistics":c.kernel.statistics(),"excluded_dsp_routes":c.report.excluded_routes.iter().filter(|r|r.rule.contains("MULT18")).collect::<Vec<_>>(),"samples":samples}));
        // HD01: CE0 low now holds the output stage (it no longer stops the
        // kernel); the words stay as they were.
        let held = Time::from_ps(514000);
        let before: Vec<_> = (0..4)
            .map(|m| c.kernel.state().multiplier_output(mults[m]).unwrap())
            .collect();
        c.kernel
            .schedule(held, c.drivers[&pin("ce0").label()], Zero)
            .unwrap();
        c.kernel.run_until(held).unwrap();
        let after: Vec<_> = (0..4)
            .map(|m| c.kernel.state().multiplier_output(mults[m]).unwrap())
            .collect();
        assert_eq!(before, after);
    }
    fs::write(dir.join("native.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"hardware_results":0,"profiles":masks,"pins":pins,"runs":reports})).unwrap()).unwrap();
}
