mod ebr_fixture;
use ebr_fixture::{init, run};
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, SystemPort, Time},
    logic::Logic::{self, *},
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};
fn merge(a: Logic, b: Logic) -> Logic {
    Logic::mux(X, a, b)
}
fn pin_value(map: &mut BTreeMap<String, Logic>, prefix: &str, value: usize, width: usize) {
    for bit in 0..width {
        map.insert(
            format!("{prefix}[{bit}]"),
            Logic::from_bool(value & (1 << bit) != 0),
        );
    }
}

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and native image reader"]
fn native_dual_clock_ebr_runs_from_config_and_bit_with_independent_oracle() {
    native_dual_clock(false, false);
}
#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and native image reader"]
fn native_inverted_write_enable_qualifies_mode_and_preserves_init_and_execution() {
    native_dual_clock(true, false);
}
#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and native image reader"]
fn native_read_only_a_port_uses_the_writer_inserted_wea_inversion() {
    native_dual_clock(false, true);
}
fn native_dual_clock(inverted_write: bool, read_only_a: bool) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let (dir, pinmap) = ebr_fixture::build(&model, inverted_write, read_only_a);
    let pin = |name: &str| Reference::Pin(pinmap[name].clone());
    let mut reports = Vec::new();
    let mut baselines = BTreeMap::new();
    for (extension, scope, startup) in [
        ("config", Scope::Observed, Startup::ReadyUnknown),
        ("bit", Scope::Observed, Startup::ReadyUnknown),
        ("config", Scope::Observed, Startup::WakePhases),
        ("bit", Scope::AllPlcRegisters, Startup::WakePhases),
    ] {
        let wake = matches!(startup, Startup::WakePhases);
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
        let mut values = BTreeMap::from([
            ("clka".into(), One),
            ("clkb".into(), Zero),
            ("cea".into(), Zero),
            ("ceb".into(), One),
            ("ocea".into(), One),
            ("oceb".into(), One),
            ("wea".into(), Logic::from_bool(inverted_write)),
            ("web".into(), Zero),
            ("rsta".into(), Zero),
            ("rstb".into(), Zero),
        ]);
        for (bus, value, width) in [
            ("csa", 2, 3),
            ("csb", 6, 3),
            ("ada", 0, 10),
            ("adb", 1, 10),
            ("bea", 3, 2),
            ("beb", 3, 2),
            ("dia", 0, 18),
            ("dib", 0, 18),
        ] {
            pin_value(&mut values, bus, value, width);
        }
        let mut drives = values.iter().map(|(p, v)| (pin(p), *v)).collect::<Vec<_>>();
        if wake {
            drives.push((Reference::System(SystemPort::Step), Zero));
        }
        let probes = pinmap
            .keys()
            .filter(|p| p.starts_with("qa[") || p.starts_with("qb[") || p.as_str() == "captured")
            .map(|p| pin(p))
            .collect();
        let mut c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope,
                startup,
                drives,
                probes,
                options: Options::default(),
            },
        )
        .unwrap_or_else(|e| panic!("{extension} {scope:?} {startup:?}: {e:#}"));
        assert_eq!(c.report.counts.ebr_blocks, 1);
        assert_eq!(c.report.ebr[0].raw_cs_decode, [2, 6]);
        assert_eq!(c.report.ebr[0].initialization, "bound");
        let qualified = inverted_write || read_only_a;
        assert_eq!(
            c.report.ebr[0].mode_decode.status,
            if qualified { "qualified" } else { "resolved" }
        );
        let resource = c
            .kernel
            .device()
            .resource_settings(
                c.kernel
                    .device()
                    .model()
                    .resource_id(&c.report.ebr[0].resource)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            resource
                .fields
                .iter()
                .find(|f| f.name.ends_with(".MODE"))
                .unwrap()
                .status,
            if qualified { "unmatched" } else { "resolved" }
        );
        if read_only_a {
            assert!(c
                .report
                .constants
                .iter()
                .any(|c| c.field.starts_with("CIB.JLSR")
                    && c.encoded_value == "0"
                    && c.value == One));
        }
        let id = c
            .kernel
            .device()
            .model()
            .resource_id(&c.report.ebr[0].resource)
            .unwrap();
        let mut memory = (0..1024)
            .map(|a| {
                std::array::from_fn::<Logic, 18, _>(|b| Logic::from_bool(init(a) & (1 << b) != 0))
            })
            .collect::<Vec<_>>();
        let mut core = [[if wake { Zero } else { X }; 18]; 2];
        let mut output = [if wake { Zero } else { X }; 18];
        let mut captured = if wake { Zero } else { X };
        let mut samples = Vec::new();
        let mut events = Vec::new();
        let mut reads = [0; 2];
        let mut writes = [0; 2];
        let mut blocked = 0;
        // Wake phase test first attempts a write before GWDIS releases; no
        // inferred physical startup duration is part of this fixture.
        if wake {
            for t in 1..=8 {
                let mut updates = Vec::new();
                if [1, 3, 5, 7].contains(&t) {
                    updates.push((c.drivers["sys:step"], One));
                }
                if [2, 4, 6, 8].contains(&t) {
                    updates.push((c.drivers["sys:step"], Zero));
                }
                if t == 1 {
                    updates.push((
                        c.drivers[&pin("wea").label()],
                        Logic::from_bool(!inverted_write),
                    ));
                    updates.push((c.drivers[&pin("clka").label()], Zero));
                    blocked += usize::from(!read_only_a);
                }
                if t == 2 {
                    updates.push((
                        c.drivers[&pin("wea").label()],
                        Logic::from_bool(inverted_write),
                    ));
                    updates.push((c.drivers[&pin("clka").label()], One));
                }
                c.kernel
                    .schedule_batch(Time::from_ps(t * 1000), &updates, None)
                    .unwrap();
                c.kernel.run_until(Time::from_ps(t * 1000)).unwrap();
            }
            for row in 0..2 {
                for bit in 0..9 {
                    assert_eq!(
                        c.kernel.state().ebr_word(id, row).unwrap()[bit],
                        memory[0][row * 9 + bit]
                    );
                }
            }
        }
        for t in 0..=512u64 {
            let clocks = [(t % 4) < 2, (t % 10) >= 5];
            let edges = [t % 4 == 2, t % 10 == 5];
            let address = [((t / 4) * 7 % 64) as usize, ((t / 10) * 11 % 64) as usize];
            let ce = [t % 23 != 7, t % 31 != 11];
            let cs = [
                if t % 29 == 13 { 0 } else { 2 },
                if t % 37 == 17 { 0 } else { 6 },
            ];
            let oce = [true, t % 43 != 9];
            let we = [!read_only_a && (t / 4) % 3 == 1, (t / 10) % 4 == 2];
            let reset = [(37..=42).contains(&t), (91..=96).contains(&t)];
            let mask = [(t / 4 % 4) as usize, (t / 10 % 4) as usize];
            let data = [
                ((t as usize) * 733) ^ 0x15555,
                ((t as usize) * 331) ^ 0x2aaaa,
            ];
            values.insert("clka".into(), Logic::from_bool(clocks[0]));
            values.insert("clkb".into(), Logic::from_bool(clocks[1]));
            for side in 0..2 {
                let l = ['a', 'b'][side];
                for (p, v) in [
                    ("ce", if side == 0 { !ce[side] } else { ce[side] }),
                    ("oce", oce[side]),
                    ("we", we[side] ^ (side == 0 && inverted_write)),
                    ("rst", reset[side]),
                ] {
                    values.insert(format!("{p}{l}"), Logic::from_bool(v));
                }
                for (p, v, w) in [
                    ("cs", cs[side], 3),
                    ("ad", address[side], 10),
                    ("be", mask[side], 2),
                    ("di", data[side], 18),
                ] {
                    pin_value(&mut values, &format!("{p}{l}"), v, w);
                }
            }
            let updates = values
                .iter()
                .map(|(p, v)| (c.drivers[&pin(p).label()], *v))
                .collect::<Vec<_>>();
            let time = Time::from_ps((t + 16) * 1000);
            c.kernel.schedule_batch(time, &updates, None).unwrap();
            c.kernel.run_until(time).unwrap();
            let old = core;
            if edges[0] {
                captured = if reset[0] { Zero } else { old[0][0] };
            }
            for side in 0..2 {
                if !edges[side] {
                    continue;
                }
                let access = ce[side] && cs[side] == [2, 6][side];
                let held = old[side];
                let mem = memory[address[side]];
                if side == 1 && oce[1] {
                    for bit in 0..18 {
                        output[bit] = if access {
                            held[bit]
                        } else {
                            merge(output[bit], held[bit])
                        };
                    }
                }
                if access {
                    if we[side] {
                        writes[side] += 1;
                        for bit in 0..18 {
                            let d = Logic::from_bool(data[side] & (1 << bit) != 0);
                            let masked = mask[side] & (1 << (bit / 9)) != 0;
                            core[side][bit] = if side == 0 {
                                if masked {
                                    d
                                } else {
                                    merge(merge(held[bit], mem[bit]), d)
                                }
                            } else if masked {
                                mem[bit]
                            } else {
                                merge(held[bit], mem[bit])
                            };
                            if masked {
                                memory[address[side]][bit] =
                                    if reset[side] { merge(mem[bit], d) } else { d };
                            }
                        }
                    } else {
                        reads[side] += 1;
                        core[side] = mem;
                    }
                }
                if reset[side] {
                    core[side] = [Zero; 18];
                    if side == 1 {
                        output = [Zero; 18];
                    }
                }
            }
            let qa = (0..18)
                .map(|b| c.value(&pin(&format!("qa[{b}]"))).unwrap())
                .collect::<Vec<_>>();
            let qb = (0..18)
                .map(|b| c.value(&pin(&format!("qb[{b}]"))).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(qa, core[0], "{extension} {startup:?} A t={t}");
            assert_eq!(qb, output, "{extension} {startup:?} B t={t}");
            assert_eq!(c.value(&pin("captured")).unwrap(), captured, "FF t={t}");
            samples.push(json!({"time_ns":t+16,"qa":qa,"qb":qb,"captured":captured}));
            events.push(json!({"time":t+16,"set":values.iter().map(|(p,v)|(pin(p).label(),v.to_string())).collect::<BTreeMap<_,_>>()}));
        }
        for (address, word) in memory.iter().enumerate() {
            for (bit, &v) in word.iter().enumerate() {
                assert_eq!(
                    c.kernel
                        .state()
                        .ebr_word(id, address * 2 + bit / 9)
                        .unwrap()[bit % 9],
                    v,
                    "array address={address} bit={bit}"
                );
            }
        }
        let key = format!("{startup:?}");
        if let Some(baseline) = baselines.get(&key) {
            assert_eq!(&samples, baseline);
        } else {
            baselines.insert(key, samples.clone());
        }
        assert_eq!(c.kernel.statistics().ebr_collision_rows, 0);
        assert!(reads.iter().all(|n| *n > 10));
        assert!(writes[1] > 10);
        assert_eq!(writes[0] == 0, read_only_a);
        if extension == "bit" && matches!(scope, Scope::Observed) {
            let mut initial = events[0].clone();
            initial["time"] = json!(0);
            events.insert(0, initial);
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
                "qa[0]", "qa[8]", "qa[17]", "qb[0]", "qb[8]", "qb[17]", "captured",
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
            for (sample, actual) in samples.iter().zip(
                cli["observations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|s| s["time_ns"].as_u64().unwrap() >= 16),
            ) {
                assert_eq!(sample["time_ns"], actual["time_ns"]);
                for port in ["qa[0]", "qa[8]", "qa[17]", "qb[0]", "qb[8]", "qb[17]"] {
                    let bit = port[3..port.len() - 1].parse::<usize>().unwrap();
                    assert_eq!(sample[&port[..2]][bit], actual["values"][pin(port).label()]);
                }
                assert_eq!(
                    sample["captured"],
                    actual["values"][pin("captured").label()]
                );
            }
            assert_eq!(cli["observations"].as_array().unwrap().len(), 514);
        }
        let plan = json!({"configuration_sha256":c.report.configuration_sha256,"scope":c.report.scope,"startup":c.report.startup,"roots":c.report.roots,"counts":c.report.counts,"ebr":c.report.ebr,"system":c.report.system,"clock_aliases":c.report.clock_aliases,"constants":c.report.constants,"resources":c.report.resources.iter().filter(|r|r.family=="EBR").collect::<Vec<_>>(),"limitations":c.report.limitations});
        reports.push(json!({"source":extension,"samples":samples,"reads":reads,"writes":writes,"startup_blocked_writes":blocked,"plan":plan,"configuration":c.kernel.device().summary(),"state":c.kernel.state().summary(),"statistics":c.kernel.statistics(),"notices":c.kernel.ebr_notices().collect::<Vec<_>>()}));
    }
    fs::write(
        dir.join("native.json"),
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"hardware_measured":false,"pins":pinmap,"runs":reports}),
        )
        .unwrap(),
    )
    .unwrap();
}
