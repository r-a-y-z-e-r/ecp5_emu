#[path = "common/tools.rs"]
mod tools;
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
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
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
fn sys(port: &str) -> Reference {
    Reference::System(SystemPort::parse(port).unwrap())
}

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and the native image reader"]
fn native_async_gsr_wake_phases_and_global_write_output_controls_share_the_kernel() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/system-controls");
    fs::create_dir_all(&dir).unwrap();
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let mut pinmap = BTreeMap::from([
        ("clk".to_string(), "G2".to_string()),
        ("gsr".into(), "J4".into()),
        ("reset".into(), "H2".into()),
    ]);
    let mut balls = model
        .package_pins()
        .filter(|p| {
            p.package == "CABGA381"
                && p.pio.is_some()
                && !["G2", "J4", "H2"].contains(&p.ball.as_str())
        })
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    pinmap.insert("we".into(), balls.next().unwrap());
    for bus in ["ra", "wa", "d", "qa"] {
        for bit in 0..4 {
            pinmap.insert(format!("{bus}[{bit}]"), balls.next().unwrap());
        }
    }
    for name in ["q_reset", "q_set", "q_nogsr"] {
        pinmap.insert(name.into(), balls.next().unwrap());
    }
    let mut lpf = String::new();
    for (port, ball) in &pinmap {
        lpf.push_str(&format!("LOCATE COMP \"{port}\" SITE \"{ball}\";\nIOBUF PORT \"{port}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n"));
    }
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    fs::write(
        dir.join("top.v"),
        r#"module top(input clk, gsr, reset, we, input [3:0] ra,wa,d,
    output q_reset,q_set,q_nogsr,output [3:0] qa);
GSR global_reset(.GSR(gsr));
TRELLIS_FF #(.GSR("ENABLED"),.REGSET("RESET"),.SRMODE("ASYNC")) r0
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[0]),.Q(q_reset));
TRELLIS_FF #(.GSR("ENABLED"),.REGSET("SET"),.SRMODE("ASYNC")) r1
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[1]),.Q(q_set));
TRELLIS_FF #(.GSR("DISABLED"),.REGSET("RESET"),.SRMODE("ASYNC")) r2
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[2]),.Q(q_nogsr));
(* ram_style="distributed" *) reg [3:0] mem [0:15];
integer i; initial for(i=0;i<16;i=i+1) mem[i]=(3*i+5)&15;
always @(posedge clk) if(we) mem[wa]<=d;
assign qa=mem[ra];
endmodule
"#,
    )
    .unwrap();
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
    let source = fs::read_to_string(dir.join("top.config")).unwrap();
    let efb0 = &model
        .tiles()
        .find(|(_, t)| t.kind == "EFB0_PICB0")
        .unwrap()
        .1
        .name;
    let efb2 = &model
        .tiles()
        .find(|(_, t)| t.kind == "EFB2_PICB0")
        .unwrap()
        .1
        .name;
    let filtered = source
        .lines()
        .filter(|l| {
            ![
                "enum: GSR.GSRMODE ",
                "enum: SYSCONFIG.WAKE_UP ",
                "enum: SYSCONFIG.DONEPHASE ",
                "enum: SYSCONFIG.DONE_EX ",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Both labels sharing the DONE phase bits must be explicit. Otherwise a
    // later parser default can overwrite a requested WAKE_UP=4 pattern.
    fs::write(dir.join("wake4.config"),format!("{filtered}\n.tile {efb0}\nenum: GSR.GSRMODE ACTIVE_LOW\nenum: SYSCONFIG.WAKE_UP 4\nenum: SYSCONFIG.DONEPHASE T0\nenum: SYSCONFIG.DONE_EX ON\n.tile {efb2}\nenum: SYSCONFIG.DONE_EX ON\n")).unwrap();
    fs::write(dir.join("high.config"),format!("{filtered}\n.tile {efb0}\nenum: GSR.GSRMODE ACTIVE_HIGH\nenum: SYSCONFIG.WAKE_UP 21\nenum: SYSCONFIG.DONEPHASE T3\nenum: SYSCONFIG.DONE_EX OFF\n.tile {efb2}\nenum: SYSCONFIG.DONE_EX OFF\n")).unwrap();
    for name in ["top", "wake4", "high"] {
        run(
            &dir,
            "ecppack",
            &[
                "--compress",
                &format!("{name}.config"),
                &format!("{name}.bit"),
            ],
            &format!("ecppack-{name}.log"),
        );
    }
    let pin = |port: &str| Reference::Pin(pinmap[port].clone());
    let mut reports = Vec::new();
    let mut sequences = Vec::new();
    for (name, extension, scope, startup) in [
        ("top", "config", Scope::Observed, Startup::WakePhases),
        ("top", "bit", Scope::Observed, Startup::WakePhases),
        ("wake4", "config", Scope::Observed, Startup::WakePhases),
        ("wake4", "bit", Scope::Observed, Startup::WakePhases),
        ("top", "bit", Scope::AllPlcRegisters, Startup::WakePhases),
        ("high", "bit", Scope::Observed, Startup::ReadyUnknown),
    ] {
        let seq4 = name == "wake4";
        let wake = matches!(startup, Startup::WakePhases);
        let active_low = name != "high";
        let image = Arc::new(
            ConfigurationImage::load(&dir.join(format!("{name}.{extension}")), &db, None).unwrap(),
        );
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let mut drives = vec![
            (pin("clk"), Zero),
            (pin("gsr"), Logic::from_bool(active_low)),
            (pin("reset"), Zero),
            (pin("we"), Zero),
        ];
        for bus in ["ra", "wa", "d"] {
            for bit in 0..4 {
                drives.push((pin(&format!("{bus}[{bit}]")), Zero));
            }
        }
        if wake {
            drives.push((sys("step"), Zero));
        }
        if seq4 {
            drives.push((sys("done"), Zero));
        }
        let mut probes = vec![pin("q_reset"), pin("q_set"), pin("q_nogsr")];
        for bit in 0..4 {
            probes.push(pin(&format!("qa[{bit}]")));
        }
        for port in [
            "goe",
            "gwdis",
            "startup_gsr",
            "user_gsr",
            "gsr",
            "done",
            "ready",
        ] {
            probes.push(sys(port));
        }
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
        .unwrap_or_else(|e| panic!("{name}.{extension} {scope:?}: {e:#}"));
        assert_eq!(c.report.counts.lutram_columns, 4);
        assert_eq!(c.report.counts.ramw, 1);
        assert_eq!(
            c.report.counts.flip_flops,
            if matches!(scope, Scope::Observed) {
                3
            } else {
                83640
            }
        );
        assert!(c.report.system.gsr_enabled_ff >= 2 && c.report.system.gsr_disabled_ff >= 1);
        let mut mem = std::array::from_fn::<u8, 16, _>(|i| (3 * i + 5) as u8 & 15);
        let mut ff = [X; 3];
        let mut clock = false;
        let mut samples = Vec::new();
        let mut allowed_writes = 0;
        let mut blocked_writes = 0;
        let mut writes_during_user_reset = 0;
        for time in 0..=1024u64 {
            let local_reset = time == 1;
            let user_reset = (19..=23).contains(&time) || (67..=71).contains(&time);
            let goe = !wake || time >= 4;
            let startup_reset = wake && time < 6;
            let write_inhibit = startup_reset;
            let ready = !wake || (time >= 8 && (!seq4 || time >= 12));
            let done = if seq4 {
                time >= 12 && time != 14
            } else {
                !wake || time >= 8
            };
            let ra = if time < 16 {
                time as usize
            } else {
                ((time * 3 + time / 7) & 15) as usize
            };
            let wa = ((time * 5 + time / 11) & 15) as usize;
            let data = ((time * 7 + time / 3) & 15) as u8;
            let we = time > 0 && time % 5 != 0;
            let edge = time > 0 && time % 2 == 0 && !clock;
            let mut levels = vec![
                ("reset".into(), Logic::from_bool(local_reset)),
                (
                    "gsr".into(),
                    Logic::from_bool(if active_low { !user_reset } else { user_reset }),
                ),
                ("we".into(), Logic::from_bool(we)),
            ];
            for (bus, value) in [("ra", ra as u8), ("wa", wa as u8), ("d", data)] {
                for bit in 0..4 {
                    levels.push((
                        format!("{bus}[{bit}]"),
                        Logic::from_bool(value & (1 << bit) != 0),
                    ));
                }
            }
            if time > 0 && time % 2 == 0 {
                clock = !clock;
                levels.push(("clk".into(), Logic::from_bool(clock)));
            }
            if local_reset {
                ff = [Zero, One, Zero];
            }
            if edge {
                for (bit, q) in ff.iter_mut().enumerate() {
                    if !local_reset {
                        *q = Logic::from_bool(data & (1 << bit) != 0);
                    }
                }
            }
            if startup_reset || user_reset {
                ff[0] = Zero;
                ff[1] = One;
            }
            if edge && we {
                if write_inhibit {
                    blocked_writes += 1;
                } else {
                    mem[wa] = data;
                    allowed_writes += 1;
                    if user_reset {
                        writes_during_user_reset += 1;
                    }
                }
            }
            if time > 0 {
                let mut updates = levels
                    .into_iter()
                    .map(|(port, value)| (c.drivers[&pin(&port).label()], value))
                    .collect::<Vec<_>>();
                if wake && (2..=9).contains(&time) {
                    updates.push((c.drivers["sys:step"], Logic::from_bool(time % 2 == 0)));
                }
                if seq4 && [12, 14, 15].contains(&time) {
                    updates.push((c.drivers["sys:done"], if time == 14 { Zero } else { Z }));
                }
                c.kernel
                    .schedule_batch(Time::from_ps(time), &updates, None)
                    .unwrap();
                c.kernel.run_until(Time::from_ps(time)).unwrap();
            }
            let mut actual = Vec::new();
            let mut expected = Vec::new();
            for (index, port) in ["q_reset", "q_set", "q_nogsr"].into_iter().enumerate() {
                actual.push(c.value(&pin(port)).unwrap());
                expected.push(if goe { ff[index] } else { Z });
            }
            for bit in 0..4 {
                actual.push(c.value(&pin(&format!("qa[{bit}]"))).unwrap());
                expected.push(if goe {
                    Logic::from_bool(mem[ra] & (1 << bit) != 0)
                } else {
                    Z
                });
            }
            for (port, level) in [
                ("goe", goe),
                ("gwdis", write_inhibit),
                ("startup_gsr", startup_reset),
                ("user_gsr", user_reset),
                ("gsr", startup_reset || user_reset),
                ("done", done),
                ("ready", ready),
            ] {
                actual.push(c.value(&sys(port)).unwrap());
                expected.push(Logic::from_bool(level));
            }
            assert_eq!(actual, expected, "{name}.{extension} {scope:?}, t={time}");
            samples.push(actual);
        }
        assert!(allowed_writes > 0 && writes_during_user_reset > 0);
        assert_eq!(blocked_writes, if wake { 1 } else { 0 });
        if matches!(scope, Scope::Observed) {
            fs::write(
                dir.join(format!("plan-{name}-{extension}.json")),
                serde_json::to_vec_pretty(&c.report).unwrap(),
            )
            .unwrap();
        }
        let trace = c.kernel.drain_trace();
        reports.push(json!({"fixture":name,"source_kind":extension,"scope":scope,"startup":startup,"samples":samples.len(),"image":c.kernel.device().image().summary(),
            "counts":c.report.counts,"system_plan":c.report.system,"final_state":c.kernel.state().summary(),"statistics":c.kernel.statistics(),"undriven":c.report.undriven.len(),
            "allowed_word_writes":allowed_writes,"inhibited_word_writes":blocked_writes,"word_writes_during_user_gsr":writes_during_user_reset,
            "final_memory":mem,"trace_transitions":trace.len(),"trace_head":trace.into_iter().take(30).collect::<Vec<_>>()}));
        sequences.push(samples);
    }
    assert_eq!(sequences[0], sequences[1]);
    assert_eq!(sequences[0], sequences[4]);
    assert_eq!(sequences[2], sequences[3]);
    let mut cli_counts = Vec::new();
    for (name, seq4) in [("top", false), ("wake4", true)] {
        let mut initial = BTreeMap::from([
            (pin("clk").label(), "0".to_string()),
            (pin("gsr").label(), "1".into()),
            (pin("reset").label(), "1".into()),
            (pin("we").label(), "1".into()),
        ]);
        for bus in ["ra", "wa", "d"] {
            for bit in 0..4 {
                initial.insert(
                    pin(&format!("{bus}[{bit}]")).label(),
                    if bus == "d" && 9 & (1 << bit) != 0 {
                        "1"
                    } else {
                        "0"
                    }
                    .into(),
                );
            }
        }
        if seq4 {
            initial.insert("sys:done".into(), "0".into());
        }
        let mut events = vec![
            json!({"time":0,"set":initial}),
            json!({"time":2,"set":{pin("clk").label():"1"}}),
            json!({"time":4,"set":{pin("clk").label():"0"}}),
            json!({"time":5,"set":{pin("clk").label():"1"}}),
            json!({"time":6,"set":{pin("reset").label():"0"}}),
        ];
        if seq4 {
            events.push(json!({"time":9,"set":{"sys:done":"z"}}));
        }
        fs::write(
            dir.join(format!("cli-{name}-events.json")),
            serde_json::to_vec_pretty(&events).unwrap(),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
        command.current_dir(&dir).args([
            "emulate",
            &format!("{name}.bit"),
            "--startup",
            "wake-phases",
            "--scope",
            "observed",
            "--clock",
            "sys:step",
            "--cycles",
            "4",
            "--stimulus",
            &format!("cli-{name}-events.json"),
            "--output",
            &format!("cli-{name}-result.json"),
        ]);
        for port in ["goe", "gwdis", "gsr", "done", "ready"] {
            command.args(["--probe", &sys(port).label()]);
        }
        for bit in 0..4 {
            command.args(["--probe", &pin(&format!("qa[{bit}]")).label()]);
        }
        let out = command.output().unwrap();
        fs::write(
            dir.join(format!("cli-{name}.log")),
            [out.stdout, out.stderr].concat(),
        )
        .unwrap();
        assert!(out.status.success(), "wake CLI {name}");
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(format!("cli-{name}-result.json"))).unwrap())
                .unwrap();
        let observations = result["observations"].as_array().unwrap();
        assert_eq!(observations.len(), if seq4 { 10 } else { 9 });
        let get = |time: usize, key: &str| observations[time]["values"][key].as_str().unwrap();
        assert_eq!(get(0, &pin("qa[0]").label()), "z");
        assert_eq!(get(3, "sys:goe"), "one");
        assert_eq!(get(4, "sys:gwdis"), "one");
        assert_eq!(get(5, "sys:gwdis"), "zero");
        assert_eq!(get(8, "sys:ready"), if seq4 { "zero" } else { "one" });
        assert_eq!(get(observations.len() - 1, "sys:ready"), "one");
        for bit in 0..4 {
            assert_eq!(
                get(5, &pin(&format!("qa[{bit}]")).label()),
                if 9 & (1 << bit) != 0 { "one" } else { "zero" }
            );
        }
        cli_counts.push(observations.len());
    }
    fs::write(dir.join("validation.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"date":"2026-10-22","device":"LFE5U-85F","pinmap":pinmap,"runs":reports,
        "cli_observations":cli_counts,"formats_and_observed_all_plc_match":true,"hardware_results":0,"scope":"logical wake phases and asynchronous user GSR; physical POR/START/transport and synchronized GSR unmodelled"})).unwrap()).unwrap();
}
