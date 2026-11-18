#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, sync::Arc};

#[test]
#[ignore = "requires installed Yosys/nextpnr/Trellis and the native image reader"]
fn native_two_lutrams_with_opposite_edges_run_from_config_and_bit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target/lutram");
    fs::create_dir_all(&dir).unwrap();
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let mut balls = model
        .package_pins()
        .filter(|p| p.package == "CABGA381" && p.pio.is_some() && p.ball != "G2" && p.ball != "J4")
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    let mut pinmap = BTreeMap::from([
        ("clk_a".to_string(), "G2".to_string()),
        ("clk_b".to_string(), "J4".to_string()),
    ]);
    pinmap.insert("we".into(), balls.next().unwrap());
    pinmap.insert("reset".into(), balls.next().unwrap());
    for bus in ["ra", "wa", "di", "qa", "qb", "captured"] {
        for bit in 0..4 {
            pinmap.insert(format!("{bus}[{bit}]"), balls.next().unwrap());
        }
    }
    let mut lpf = String::new();
    for (port, ball) in &pinmap {
        lpf.push_str(&format!("LOCATE COMP \"{port}\" SITE \"{ball}\";\nIOBUF PORT \"{port}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n"));
    }
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    fs::write(
        dir.join("top.v"),
        r#"module top(input clk_a, clk_b, we, reset, input [3:0] ra, wa, di,
    output [3:0] qa, qb, output reg [3:0] captured);
(* ram_style="distributed" *) reg [3:0] mem_a [0:15];
(* ram_style="distributed" *) reg [3:0] mem_b [0:15];
integer i;
initial for (i=0;i<16;i=i+1) begin mem_a[i]=(3*i+5)&15; mem_b[i]=(5*i+9)&15; end
always @(posedge clk_a) if (we) mem_a[wa] <= di;
always @(negedge clk_b) if (we) mem_b[wa] <= ~di;
assign qa=mem_a[ra];
assign qb=mem_b[ra];
always @(posedge clk_a or posedge reset) if (reset) captured <= 0; else captured <= mem_a[ra];
endmodule
"#,
    )
    .unwrap();
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
        let output = tools::command(program)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        fs::write(
            dir.join(format!("{}.log", program.rsplit('/').next().unwrap())),
            [output.stdout, output.stderr].concat(),
        )
        .unwrap();
        assert!(output.status.success(), "{program}: see {}", dir.display());
    }
    let synthesis: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("top.json")).unwrap()).unwrap();
    assert_eq!(
        synthesis["modules"]["top"]["cells"]
            .as_object()
            .unwrap()
            .values()
            .filter(|c| c["type"] == "TRELLIS_DPR16X4")
            .count(),
        2
    );
    let pin = |port: &str| Reference::Pin(pinmap[port].clone());
    let mut runs = Vec::new();
    let mut all_samples = Vec::new();
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
        let mut drives = vec![
            (pin("clk_a"), Logic::Zero),
            (pin("clk_b"), Logic::One),
            (pin("we"), Logic::Zero),
            (pin("reset"), Logic::Zero),
        ];
        for bus in ["ra", "wa", "di"] {
            for bit in 0..4 {
                drives.push((pin(&format!("{bus}[{bit}]")), Logic::Zero));
            }
        }
        let probes = ["qa", "qb", "captured"]
            .into_iter()
            .flat_map(|bus| (0..4).map(move |bit| pin(&format!("{bus}[{bit}]"))))
            .collect();
        let mut c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope,
                startup: Startup::ReadyUnknown,
                drives,
                probes,
                options: Options::default(),
            },
        )
        .unwrap_or_else(|e| panic!("{extension} {scope:?}: {e:#}"));
        assert_eq!(c.report.counts.lutram_columns, 8);
        assert_eq!(c.report.counts.ramw, 2);
        assert_eq!(
            c.report
                .lutram
                .iter()
                .map(|r| r.unmodelled_c_ff_di.len())
                .sum::<usize>(),
            if matches!(scope, Scope::Observed) {
                0
            } else {
                4
            }
        );
        assert_eq!(
            c.report.counts.flip_flops,
            if matches!(scope, Scope::Observed) {
                4
            } else {
                83640
            }
        );
        let mut mem_a = std::array::from_fn::<u8, 16, _>(|i| (3 * i + 5) as u8 & 15);
        let mut mem_b = std::array::from_fn::<u8, 16, _>(|i| (5 * i + 9) as u8 & 15);
        let read_bus = |c: &compile::Compiled, bus: &str| {
            (0..4).try_fold(0u8, |value, bit| {
                Some(
                    value
                        | ((c.value(&pin(&format!("{bus}[{bit}]"))).unwrap().bit()? as u8) << bit),
                )
            })
        };
        assert_eq!(read_bus(&c, "qa"), Some(5));
        assert_eq!(read_bus(&c, "qb"), Some(9));
        assert_eq!(read_bus(&c, "captured"), None);
        let (mut ca, mut cb) = (false, true);
        let mut captured = None;
        let mut samples = Vec::new();
        let mut write_edges = [0u64; 2];
        let mut initial_reads = 0;
        let mut collision_captures = 0;
        let mut written_addresses = [0u16; 2];
        for time in 1..=2048u64 {
            let ra = if time <= 16 {
                time as usize - 1
            } else {
                ((time * 7 + time / 11) & 15) as usize
            };
            let wa = if time % 5 == 0 {
                ra
            } else {
                ((time * 3 + time / 7) & 15) as usize
            };
            let data = ((time * 13 + time / 3) & 15) as u8;
            let enable = time > 16 && time % 7 != 0;
            let reset = (1024..=1027).contains(&time);
            let mut levels = vec![
                ("we".into(), Logic::from_bool(enable)),
                ("reset".into(), Logic::from_bool(reset)),
            ];
            for (bus, value) in [("ra", ra as u8), ("wa", wa as u8), ("di", data)] {
                for bit in 0..4 {
                    levels.push((
                        format!("{bus}[{bit}]"),
                        Logic::from_bool(value & (1 << bit) != 0),
                    ));
                }
            }
            let edge_a = time % 2 == 0 && !ca;
            let edge_b = time % 3 == 0 && cb;
            if edge_a {
                captured = Some(mem_a[ra]); // explicit pre-write capture policy
                if enable && ra == wa {
                    collision_captures += 1;
                }
            }
            if reset {
                captured = Some(0);
            }
            if time % 2 == 0 {
                ca = !ca;
                levels.push(("clk_a".into(), Logic::from_bool(ca)));
            }
            if time % 3 == 0 {
                cb = !cb;
                levels.push(("clk_b".into(), Logic::from_bool(cb)));
            }
            if enable && edge_a {
                mem_a[wa] = data;
                write_edges[0] += 1;
                written_addresses[0] |= 1 << wa;
            }
            if enable && edge_b {
                mem_b[wa] = (!data) & 15;
                write_edges[1] += 1;
                written_addresses[1] |= 1 << wa;
            }
            let updates = levels
                .iter()
                .map(|(port, value)| (c.drivers[&pin(port).label()], *value))
                .collect::<Vec<_>>();
            c.kernel
                .schedule_batch(Time::from_ps(time), &updates, None)
                .unwrap();
            c.kernel.run_until(Time::from_ps(time)).unwrap();
            let actual = [
                read_bus(&c, "qa"),
                read_bus(&c, "qb"),
                read_bus(&c, "captured"),
            ];
            let expected = [Some(mem_a[ra]), Some(mem_b[ra]), captured];
            assert_eq!(
                actual, expected,
                "{extension} {scope:?}, time={time}, ra={ra}, wa={wa}"
            );
            if time <= 16 {
                initial_reads += 1;
            }
            samples.push(expected);
        }
        assert_eq!(written_addresses, [u16::MAX; 2]);
        assert!(collision_captures > 0);
        let trace = c.kernel.drain_trace();
        if matches!(scope, Scope::Observed) {
            fs::write(
                dir.join(format!("plan-{extension}.json")),
                serde_json::to_vec_pretty(&c.report).unwrap(),
            )
            .unwrap();
        }
        runs.push(json!({"source_kind":extension,"scope":scope,"image":c.kernel.device().image().summary(),"counts":c.report.counts,
            "samples":samples.len(),"initial_addresses_checked":initial_reads,"written_address_masks":written_addresses,
            "write_edges":write_edges,"prewrite_collision_captures":collision_captures,"final_memory_a":mem_a,"final_memory_b":mem_b,
            "lutram_groups":c.report.lutram,"state":c.kernel.state().summary(),"statistics":c.kernel.statistics(),
            "trace_transitions":trace.len(),"trace_head":trace.into_iter().take(24).collect::<Vec<_>>()}));
        all_samples.push(samples);
    }
    assert_eq!(all_samples[0], all_samples[1]);
    assert_eq!(all_samples[0], all_samples[2]);
    let mut initial = BTreeMap::new();
    for port in ["clk_a", "clk_b", "we", "reset"] {
        initial.insert(
            pin(port).label(),
            if port == "clk_b" { "1" } else { "0" }.to_string(),
        );
    }
    for bus in ["ra", "wa", "di"] {
        for bit in 0..4 {
            initial.insert(pin(&format!("{bus}[{bit}]")).label(), "0".into());
        }
    }
    let mut write = BTreeMap::from([(pin("we").label(), "1".to_string())]);
    for bit in 0..4 {
        write.insert(
            pin(&format!("di[{bit}]")).label(),
            if 9 & (1 << bit) != 0 { "1" } else { "0" }.into(),
        );
    }
    let events = json!([
        {"time":0,"set":initial}, {"time":1,"set":write},
        {"time":2,"set":{pin("clk_a").label():"1"}},
        {"time":3,"set":{pin("clk_a").label():"0",pin("clk_b").label():"0"}},
        {"time":4,"set":{pin("we").label():"0",pin("ra[0]").label():"1"}},
        {"time":5,"set":{pin("reset").label():"1"}},
        {"time":6,"set":{pin("reset").label():"0"}}
    ]);
    fs::write(
        dir.join("cli-events.json"),
        serde_json::to_vec_pretty(&events).unwrap(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
    command
        .args([
            "emulate",
            "top.bit",
            "--startup",
            "ready-unknown",
            "--scope",
            "observed",
            "--stimulus",
            "cli-events.json",
            "--output",
            "cli-result.json",
        ])
        .current_dir(&dir);
    for bus in ["qa", "qb", "captured"] {
        for bit in 0..4 {
            command.args(["--probe", &pin(&format!("{bus}[{bit}]")).label()]);
        }
    }
    let output = command.output().unwrap();
    fs::write(dir.join("cli.log"), [output.stdout, output.stderr].concat()).unwrap();
    assert!(
        output.status.success(),
        "emulate CLI: see {}",
        dir.display()
    );
    let cli: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("cli-result.json")).unwrap()).unwrap();
    let observations = cli["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 7);
    for (bus, value) in [("qa", 8), ("qb", 14), ("captured", 0)] {
        for bit in 0..4 {
            assert_eq!(
                observations[6]["values"][pin(&format!("{bus}[{bit}]")).label()],
                if value & (1 << bit) != 0 {
                    "one"
                } else {
                    "zero"
                }
            );
        }
    }
    fs::write(dir.join("validation.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"date":"2026-10-22",
        "fixture":"two independent 16x4 distributed RAMs; posedge and negedge writes; asynchronous reads; four capturing FF",
        "pinmap":pinmap,"runs":runs,"all_formats_and_scopes_match":true,"cli_observations":observations.len(),"hardware_validated":false,
        "collision_policy":"settled read sampled by FF before simultaneous RAM write; digital policy, not hardware timing proof"})).unwrap()).unwrap();
}
