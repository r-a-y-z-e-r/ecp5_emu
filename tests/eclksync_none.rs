//! R09/X13: nextpnr's used ECLKSYNC MODE=NONE is a buffer
//! (HI12); HI13 clears enabled pin-clock synchronizers at configuration.
mod board;
#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::{topology::Wire, DeviceModel},
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, sync::Arc};

const PERIOD: u64 = 40_000;
const PLL: &str = r#"wire source;
EHXPLLL #(.CLKI_DIV(1),.CLKFB_DIV(4),.CLKOP_DIV(6),.CLKOP_CPHASE(2),
.CLKOP_FPHASE(0),.CLKOP_ENABLE("ENABLED"),.FEEDBK_PATH("INT_OP")) pll(
.CLKI(clk),.CLKFB(source),.CLKOP(source),.RST(1'b0),.STDBY(1'b0),
.PHASESEL0(1'b0),.PHASESEL1(1'b0),.PHASEDIR(1'b0),.PHASESTEP(1'b0),
.PHASELOADREG(1'b0),.PLLWAKESYNC(1'b0),.ENCLKOP(1'b0),.ENCLKOS(1'b0),
.ENCLKOS2(1'b0),.ENCLKOS3(1'b0));"#;

struct Build(PathBuf);
impl Drop for Build {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
impl Build {
    fn new(label: &str, pll: bool, div: &str, stop: Option<&str>) -> Self {
        Self::from_source(label, if pll { PLL } else { "wire source=clk;" }, div, stop)
    }

    fn from_source(label: &str, source: &str, div: &str, stop: Option<&str>) -> Self {
        let dir = std::env::temp_dir().join(format!("ecp5-r09-{}-{label}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let result = Self(dir);
        let params = if div == "default" {
            String::new()
        } else {
            format!(r#"#(.DIV("{div}"))"#)
        };
        let sync = stop.map_or_else(|| "wire divided_input=source;".into(), |stop| format!("wire divided_input; ECLKSYNCB sync(.ECLKI(source),.STOP({stop}),.ECLKO(divided_input));"));
        let v = format!("module top(input clk,output q); {} {sync} CLKDIVF {params} u(.CLKI(divided_input),.RST(1'b0),.ALIGNWD(1'b0),.CDIVX(q)); endmodule\n", source);
        std::fs::write(result.0.join("top.v"), v).unwrap();
        std::fs::write(result.0.join("top.lpf"), "LOCATE COMP \"clk\" SITE \"G2\";\nIOBUF PORT \"clk\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\nLOCATE COMP \"q\" SITE \"B2\";\nIOBUF PORT \"q\" IO_TYPE=LVCMOS33;\n").unwrap();
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
                    "--allow-fabric-eclk",
                    "--85k",
                    "--package",
                    "CABGA381",
                    "--seed",
                    "1",
                    "--json",
                    "top.json",
                    "--lpf",
                    "top.lpf",
                    "--textcfg",
                    "top.config",
                    "--write",
                    "routed.json",
                    "--quiet",
                ],
            ),
            ("ecppack", vec!["top.config", "top.bit"]),
        ] {
            let output = tools::command(program)
                .args(args)
                .current_dir(&result.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        result
    }

    fn compile(&self) -> anyhow::Result<(Compiled, Vec<Reference>)> {
        let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None)?;
        let model = Arc::new(DeviceModel::load(&db, &evidence)?);
        let image = Arc::new(ConfigurationImage::load(
            &self.0.join("top.config"),
            &db,
            None,
        )?);
        let device = Arc::new(ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence)?,
        )?);
        let routed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.0.join("routed.json"))?)?;
        let bel = routed["modules"]["top"]["cells"]["u"]["attributes"]["NEXTPNR_BEL"]
            .as_str()
            .unwrap();
        let bel_parts: Vec<_> = bel.split('/').collect();
        let key = format!(
            "LFE5U-85F/R{}C{}/{}",
            &bel_parts[1][1..],
            &bel_parts[0][1..],
            bel_parts[2]
        );
        let id = model.resource_id(&key).unwrap();
        let lane = bel_parts[2].strip_prefix("CLKDIV").unwrap();
        let location = model.resource(id).location;
        let probes = vec![
            Reference::Wire(Wire {
                location,
                name: format!("ECLKI_ECLKSYNC{lane}"),
            }),
            Reference::Wire(Wire {
                location,
                name: format!("CLKI_CLKDIV{lane}"),
            }),
            Reference::Wire(Wire {
                location,
                name: format!("JCDIVX_CLKDIV{lane}"),
            }),
            board::pin("B2"),
        ];
        let c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: vec![(board::pin("G2"), Logic::Zero)],
                probes: probes.clone(),
                options: Options {
                    trace_capacity: 65_536,
                    ..Options::default()
                },
            },
        )?;
        Ok((c, probes))
    }
}

fn check(build: &Build, seven: bool, control: bool, delayed: bool) {
    let (mut c, refs) = build.compile().unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(c.report.counts.eclk_syncs, 1);
    assert_eq!(c.report.counts.clock_dividers, 1);
    assert_eq!(c.report.undriven.len(), 0);
    let sync = c
        .report
        .resources
        .iter()
        .find(|r| r.family == "ECLKSYNCB")
        .unwrap();
    assert!(sync
        .rule
        .contains(if control { "HI04" } else { "HI12: MODE=NONE" }));
    if control {
        assert!(
            sync.rule.contains(if delayed {
                "stages initially 1"
            } else {
                "stages initially 0"
            }),
            "{}",
            sync.rule
        );
    }
    let probes: Vec<_> = refs.iter().map(|r| c.probes[&r.label()]).collect();
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD)).unwrap();
    fast.trace(&probes).unwrap();
    c.kernel.drain_trace();
    let mut before: Vec<_> = probes.iter().map(|&p| c.kernel.value(p).unwrap()).collect();
    assert_eq!(before, [Logic::Zero; 4]);
    let initial = before.clone();
    let mut rising = 0usize;
    let mut half_edges = 0usize;
    let mut wanted_q = false;
    // Sample every 2.5 ns, including every 100 MHz PLL edge, from startup.
    // Also compare the full per-signal transition traces below.
    for tick in 1..=4096 {
        let ps = tick * 2500;
        if ps % (PERIOD / 2) == 0 {
            c.kernel
                .schedule(Time::from_ps(ps), clock, Logic::from_bool(ps % PERIOD != 0))
                .unwrap();
        }
        c.kernel.run_until(Time::from_ps(ps)).unwrap();
        fast.run_until(Time::from_ps(ps)).unwrap();
        let values: Vec<_> = probes.iter().map(|&p| c.kernel.value(p).unwrap()).collect();
        for (k, &p) in probes.iter().enumerate() {
            assert_eq!(fast.value(p), Some(values[k]), "probe {k} at {ps} ps");
        }
        if values[0] != before[0] && values[0] == Logic::One {
            rising += 1;
        }
        if !control {
            assert_eq!(values[1], values[0], "NONE must buffer from the first edge");
        } else {
            assert_eq!(
                values[1] == Logic::One,
                (rising > 4 || !delayed) && values[0] == Logic::One
            );
        }
        if values[1] != before[1] {
            if seven {
                wanted_q = half_edges % 7 < 4;
                half_edges += 1;
            } else if values[1] == Logic::One {
                wanted_q = !wanted_q;
            }
        }
        assert_eq!(values[2], Logic::from_bool(wanted_q), "CDIVX at {ps} ps");
        assert_eq!(values[3], values[2], "output pad at {ps} ps");
        before = values;
    }
    assert!(rising > 10);
    let kernel_trace = c.kernel.drain_trace();
    let trace = fast.take_trace();
    for (k, reference) in refs.iter().enumerate() {
        let Reference::Wire(wire) = reference else {
            continue;
        };
        let kernel: Vec<_> = kernel_trace
            .iter()
            .filter(|t| &t.wire == wire)
            .map(|t| {
                let (n, d) = t.time.ratio();
                (n / d, t.value)
            })
            .collect();
        let mut previous = initial[k];
        let fast: Vec<_> = trace
            .times_ps
            .iter()
            .zip(trace.rows.chunks_exact(4))
            .filter_map(|(&ps, row)| {
                let value = Logic::from_bool(row[k] == 1);
                let changed = value != previous;
                previous = value;
                changed.then_some((ps, value))
            })
            .collect();
        assert!(!kernel.is_empty());
        assert_eq!(kernel, fast, "transition trace of {}", reference.label());
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang, native image reader and Trellis database"]
fn x13_and_explicit_dividers_buffer_from_the_first_pin_edge() {
    for div in ["default", "2.0", "3.5"] {
        check(
            &Build::new(&format!("pin-{div}"), false, div, None),
            div == "3.5",
            false,
            false,
        );
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang, native image reader and Trellis database"]
fn pll_dividers_buffer_from_the_first_pll_edge() {
    for div in ["default", "2.0", "3.5"] {
        check(
            &Build::new(&format!("pll-{div}"), true, div, None),
            div == "3.5",
            false,
            false,
        );
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang, native image reader and Trellis database"]
fn enabled_sync_pin_starts_cleared_and_unmeasured_pll_keeps_hi04_startup() {
    for pll in [false, true] {
        for div in ["2.0", "3.5"] {
            check(
                &Build::new(&format!("sync-{pll}-{div}"), pll, div, Some("1'b0")),
                div == "3.5",
                true,
                pll,
            );
        }
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, native image reader and Trellis database"]
fn none_with_a_connected_stop_stays_refused() {
    for (label, stop) in [("logic", "clk"), ("constant", "1'b0")] {
        let build = Build::new(label, false, "2.0", Some(stop));
        let path = build.0.join("top.config");
        let config = std::fs::read_to_string(&path).unwrap();
        let mut changed = 0;
        let config: String = config
            .lines()
            .map(|line| {
                if line.contains("ECLKSYNC") && line.ends_with(".MODE ECLKSYNCB") {
                    changed += 1;
                    format!("{} NONE\n", line.strip_suffix(" ECLKSYNCB").unwrap())
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        assert_eq!(changed, 1);
        std::fs::write(path, config).unwrap();
        let error = build.compile().err().expect("connected STOP must fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("MODE=NONE")
                && message.contains("STOP")
                && message.contains("not measured"),
            "{message}"
        );
    }
}

#[test]
#[ignore = "requires Yosys, nextpnr, ecppack, clang, native image reader and Trellis database"]
fn only_constant_enabled_primary_pin_paths_start_cleared() {
    for (label, source, delayed) in [
        ("dcc-one", "wire source; DCCA c(.CLKI(clk),.CE(1'b1),.CLKO(source));", false),
        ("dcc-chain", "wire mid,source; DCCA a(.CLKI(clk),.CE(1'b1),.CLKO(mid)); DCCA b(.CLKI(mid),.CE(1'b1),.CLKO(source));", false),
        ("dcc-dynamic", "wire source; DCCA c(.CLKI(clk),.CE(clk),.CLKO(source));", true),
        ("fabric", "reg [1:0] count=0; always @(posedge clk) count<=count+1'b1; wire source=count[1];", true),
        ("oscg", "wire source; OSCG #(.DIV(128)) c(.OSC(source));", true),
    ] {
        check(&Build::from_source(label, source, "2.0", Some("1'b0")), false, true, delayed);
    }
    let disabled = Build::from_source(
        "dcc-zero",
        "wire source; DCCA c(.CLKI(clk),.CE(1'b0),.CLKO(source));",
        "2.0",
        Some("1'b0"),
    );
    let (c, _) = disabled.compile().unwrap();
    assert!(c
        .report
        .resources
        .iter()
        .any(|r| r.family == "ECLKSYNCB" && r.rule.contains("stages initially 1")));
}
