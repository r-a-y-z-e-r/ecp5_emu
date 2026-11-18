//! R08/X12: output DELAYG/DELAYF after an IO register or a fabric FF.
//! Build real configurations, then compare the pad/CFLAG traces at control
//! changes, clock edges and between edges in the event kernel and fast mode.
mod board;
#[path = "output_delay/observations.rs"]
mod observations;
use board::{pin, PERIOD_PS};
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, process::Command, sync::Arc};

const X12: &str = "module top(input clk,d,output q);
  wire v; OFS1P3BX r(.SCLK(clk),.SP(1'b1),.PD(1'b0),.D(d),.Q(v));
  DELAYG #(.DEL_VALUE(1)) u(.A(v),.Z(q)); endmodule";
const DELAYF: &str = "module top(input clk,d,loadn,move,direction,output q,flag);
  wire v; OFS1P3BX r(.SCLK(clk),.SP(1'b1),.PD(1'b0),.D(d),.Q(v));
  DELAYF #(.DEL_VALUE(10)) u(.A(v),.Z(q),.LOADN(loadn),.MOVE(move),
                            .DIRECTION(direction),.CFLAG(flag)); endmodule";
const FABRIC: &str = "module top(input clk,d,output q);
  (* syn_useioff=0 *) reg v=0; always @(posedge clk) v<=d;
  DELAYG #(.DEL_VALUE(127)) u(.A(v),.Z(q)); endmodule";

struct Fixture {
    dir: PathBuf,
    output: &'static str,
    flag: bool,
}

impl Fixture {
    fn build(name: &str, verilog: &str, output: &'static str, flag: bool) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("output-delay-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("top.v"), verilog).unwrap();
        let mut pins = vec![("clk", "G2"), ("d", "B2"), ("q", output)];
        if flag {
            pins.extend([
                ("loadn", "C2"),
                ("move", "C1"),
                ("direction", "D2"),
                ("flag", "D1"),
            ]);
        }
        let lpf: String = pins.iter().map(|(name, ball)| format!(
            "LOCATE COMP \"{name}\" SITE \"{ball}\";\nIOBUF PORT \"{name}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n"
        )).collect();
        std::fs::write(dir.join("top.lpf"), lpf).unwrap();
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
                    "top_routed.json",
                    "-q",
                ],
            ),
        ] {
            let result = Command::new(program)
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let config = std::fs::read_to_string(dir.join("top.config")).unwrap();
        assert!(config.contains("DELAY.OUTDEL ENABLED"));
        // Side IOLOGIC distinguishes NONE from IREG_OREG; SIOLOGIC's bits
        // coincide and must be qualified by the PIO mux at execution time.
        let mode = if verilog == FABRIC {
            "NONE"
        } else {
            "IREG_OREG"
        };
        assert!((b'A'..=b'D').any(|letter| {
            config.contains(&format!("enum: IOLOGIC{}.MODE {mode}", char::from(letter)))
        }));
        Self { dir, output, flag }
    }

    fn compile(&self, wait_for_edge: bool) -> anyhow::Result<Compiled> {
        let config = self.dir.join(if wait_for_edge {
            "wait.config"
        } else {
            "top.config"
        });
        if wait_for_edge {
            let original = std::fs::read_to_string(self.dir.join("top.config"))?;
            assert!(original.contains("DELAY.WAIT_FOR_EDGE DISABLED"));
            std::fs::write(
                &config,
                original.replace(
                    "DELAY.WAIT_FOR_EDGE DISABLED",
                    "DELAY.WAIT_FOR_EDGE ENABLED",
                ),
            )?;
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let evidence = root.join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None)?;
        let model = Arc::new(DeviceModel::load(&db, &evidence)?);
        let image = Arc::new(ConfigurationImage::load(&config, &db, None)?);
        let device = Arc::new(ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence)?,
        )?);
        let mut drives = vec![(pin("G2"), Logic::Zero), (pin("B2"), Logic::Zero)];
        let mut probes = vec![pin(self.output)];
        if self.flag {
            drives.extend([
                (pin("C2"), Logic::One),
                (pin("C1"), Logic::Zero),
                (pin("D2"), Logic::One),
            ]);
            probes.push(pin("D1"));
        }
        compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives,
                probes,
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
    }
}

/// Protocol-valid HI08/HI11 control program: startup without load, both end
/// stops, direction-only changes, long MOVE, two-cycle pulses and load priority.
fn program() -> Vec<(bool, bool, bool)> {
    let mut steps = vec![(true, false, true); 4];
    for _ in 0..4 {
        steps.extend([(true, true, true), (true, false, true)]);
    }
    steps.extend([(true, false, false), (true, false, true)]);
    for direction in [false, true] {
        steps.extend([(false, false, direction); 4]);
        steps.extend([(true, false, direction); 4]);
        for _ in 0..140 {
            steps.extend([(true, true, direction), (true, false, direction)]);
        }
        steps.extend([(true, false, !direction); 4]);
    }
    for direction in [false, true] {
        steps.push((false, false, direction));
        steps.extend([(true, true, direction); 32]);
        steps.extend([(true, false, direction); 8]);
    }
    for loadn in [true, false] {
        for _ in 0..20 {
            steps.extend([(loadn, true, false); 2]);
            steps.extend([(loadn, false, false); 2]);
        }
    }
    steps
}

struct Comparison {
    event: Vec<Vec<Logic>>,
    fast: Vec<Vec<Logic>>,
}

impl Comparison {
    fn sample(
        &mut self,
        c: &mut Compiled,
        fast: &mut FastSim,
        probes: &[SignalId],
        ps: u64,
        want: &[Logic],
    ) {
        let time = Time::from_ps(ps);
        c.kernel.run_until(time).unwrap();
        fast.run_until(time).unwrap();
        let event: Vec<_> = probes.iter().map(|&p| c.kernel.value(p).unwrap()).collect();
        let quick: Vec<_> = probes.iter().map(|&p| fast.value(p).unwrap()).collect();
        assert_eq!(event, want, "data/CFLAG at {time}");
        assert_eq!(quick, event, "fast/kernel at {time}");
        self.event.push(event);
        self.fast.push(quick);
    }
}

fn exercise(fixture: &Fixture, initial: bool, counter_initial: u8) {
    let mut c = fixture.compile(false).unwrap_or_else(|e| panic!("{e:#}"));
    assert!(c
        .report
        .resources
        .iter()
        .any(|r| r.rule.contains("output delay to IOLDOD") && r.rule.contains("HI11")));
    let mut probes = vec![c.probes[&format!("pin:{}", fixture.output)]];
    if fixture.flag {
        probes.push(c.probes["pin:D1"]);
    }
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut comparison = Comparison {
        event: Vec::new(),
        fast: Vec::new(),
    };
    let mut q = initial;
    let mut count = counter_initial;
    let mut prev_move = false;
    let mut ends = [0; 2];
    let expected = |q, flag| {
        let mut values = vec![Logic::from_bool(q)];
        if fixture.flag {
            values.push(Logic::from_bool(flag));
        }
        values
    };
    comparison.sample(&mut c, &mut fast, &probes, 0, &expected(q, count == 0));
    for (i, (loadn, move_, direction)) in program().into_iter().enumerate() {
        let base = i as u64 * PERIOD_PS;
        let d = (i ^ (i >> 2) ^ (i >> 5)) & 1 != 0;
        // All direction changes occur with MOVE low, so the independent
        // endpoint model needs only the direction at the falling edge.
        if !loadn {
            count = 10;
        } else if prev_move && !move_ {
            count = if direction {
                count.saturating_sub(1)
            } else {
                (count + 1).min(127)
            };
        }
        prev_move = move_;
        ends[0] += usize::from(count == 0);
        ends[1] += usize::from(count == 127);
        let flag = count == if direction { 0 } else { 127 };
        let mut updates = vec![(c.drivers["pin:B2"], d)];
        if fixture.flag {
            updates.extend([
                (c.drivers["pin:C2"], loadn),
                (c.drivers["pin:C1"], move_),
                (c.drivers["pin:D2"], direction),
            ]);
        }
        let change = Time::from_ps(base + PERIOD_PS / 4);
        for &(driver, value) in &updates {
            c.kernel
                .schedule(change, driver, Logic::from_bool(value))
                .unwrap();
        }
        fast.set_at(change, &updates).unwrap();
        comparison.sample(
            &mut c,
            &mut fast,
            &probes,
            base + PERIOD_PS / 4,
            &expected(q, flag),
        );
        let rise = base + PERIOD_PS / 2;
        c.kernel
            .schedule(Time::from_ps(rise), clock, Logic::One)
            .unwrap();
        q = d;
        comparison.sample(&mut c, &mut fast, &probes, rise, &expected(q, flag));
        // Data toggles away from the clock; both register variants must hold.
        let between = Time::from_ps(base + 3 * PERIOD_PS / 4);
        c.kernel
            .schedule(between, c.drivers["pin:B2"], Logic::from_bool(!d))
            .unwrap();
        fast.set_at(between, &[(c.drivers["pin:B2"], !d)]).unwrap();
        comparison.sample(
            &mut c,
            &mut fast,
            &probes,
            base + 3 * PERIOD_PS / 4,
            &expected(q, flag),
        );
        let fall = base + PERIOD_PS;
        c.kernel
            .schedule(Time::from_ps(fall), clock, Logic::Zero)
            .unwrap();
        comparison.sample(&mut c, &mut fast, &probes, fall, &expected(q, flag));
    }
    assert!(
        ends.iter().all(|&n| n > 20),
        "both endpoint plateaus: {ends:?}"
    );
    assert_eq!(comparison.event, comparison.fast, "equal pad/CFLAG traces");
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn x12_sdr_delayg_is_registered_uninverted_and_zero_delay() {
    exercise(&Fixture::build("x12", X12, "A2", false), true, 0);
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn output_delayf_data_and_cflag_match_in_both_executors() {
    exercise(&Fixture::build("delayf", DELAYF, "A2", true), true, 10);
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn fabric_ff_delayg_bypasses_the_io_register_in_both_executors() {
    exercise(&Fixture::build("fabric", FABRIC, "A2", false), false, 0);
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn output_delays_work_on_the_hi11_uart_pad() {
    exercise(&Fixture::build("uart-delayf", DELAYF, "L4", true), true, 10);
    exercise(
        &Fixture::build("uart-fabric", FABRIC, "L4", false),
        false,
        0,
    );
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn sio_output_delays_qualify_the_ambiguous_mode() {
    exercise(&Fixture::build("sio-delayf", DELAYF, "A12", true), true, 0);
    exercise(
        &Fixture::build("sio-fabric", FABRIC, "A12", false),
        false,
        0,
    );
}

#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn wait_for_edge_still_refuses_registered_and_bypassed_delays() {
    for (name, verilog, ball) in [
        ("wait-oreg", X12, "A2"),
        ("wait-fabric", FABRIC, "A2"),
        ("wait-sio-oreg", X12, "A12"),
        ("wait-sio-fabric", FABRIC, "A12"),
    ] {
        let fixture = Fixture::build(name, verilog, ball, false);
        let error = match fixture.compile(true) {
            Ok(_) => panic!("WAIT_FOR_EDGE must refuse"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("DELAY.WAIT_FOR_EDGE"), "{error}");
    }
}

/// HI11's actual routed configurations, observing CFLAG and controls directly.
/// UART/sample RAM is outside this cone; the native 25 MHz control program runs.
fn measured_startup(value: u8, ident: u8, startup: u8) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let series = evidence.join("hi11_output_delay");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(
        ConfigurationImage::load(&series.join(format!("hi11s{value}.config")), &db, None).unwrap(),
    );
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let port = |key: &str, name: &str| {
        let id = model.resource_id(key).unwrap();
        Reference::Wire(
            model
                .resource_ports(id)
                .into_iter()
                .find(|port| port.name == name)
                .unwrap()
                .wire,
        )
    };
    let output = "LFE5U-85F/R56C0/IOLOGICC";
    let bottom = !matches!(value, 10 | 100);
    let mut probes = vec![
        port(output, "CFLAG"),
        port("LFE5U-85F/R35C0/IOLOGICA", "CFLAG"),
    ];
    if bottom {
        probes.push(port("LFE5U-85F/R95C4/SIOLOGICA", "CFLAG"));
    }
    for name in ["LOADN", "MOVE", "DIRECTION"] {
        probes.push(port(output, name));
    }
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(pin("G2"), Logic::Zero)],
            probes: probes.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("DEL_VALUE={value}: {e:#}"));
    let signals: Vec<_> = probes
        .iter()
        .map(|probe| c.probes[&probe.label()])
        .collect();
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let clock = c.drivers["pin:G2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();

    // Actual board samples, rather than another copy of the startup formula.
    let prefix = format!("H11S{ident}");
    let record = std::fs::read(series.join(format!("records/{prefix}-load-1.txt"))).unwrap();
    let line = record
        .split(|&byte| byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .find(|line| line.starts_with(prefix.as_bytes()) && line.len() == 4111)
        .unwrap();
    let samples: Vec<u8> = line[13..4109]
        .chunks_exact(2)
        .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
        .collect();
    assert_eq!(samples.len(), 2048);
    let values = |c: &Compiled| {
        signals
            .iter()
            .map(|&signal| c.kernel.value(signal).unwrap())
            .collect::<Vec<_>>()
    };
    let agree = |c: &Compiled, fast: &FastSim| {
        let quick: Vec<_> = signals.iter().map(|&s| fast.value(s).unwrap()).collect();
        assert_eq!(quick, values(c), "DEL_VALUE={value} at {}", c.kernel.now());
    };
    c.kernel.run_until(Time::ZERO).unwrap();
    fast.run_until(Time::ZERO).unwrap();
    agree(&c, &fast);
    assert_eq!(values(&c)[..2], [Logic::from_bool(startup == 0); 2]);
    if bottom {
        assert_eq!(values(&c)[2], Logic::One);
    }

    // Sample 0 is read before rising edge 65537, after the 65536-cycle warm-up.
    // Thus sample k is the stable state after falling edge 65536+k.
    let mut endpoint = [None; 2];
    for cycle in 1..=65_536 + 2047u64 {
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        fast.run_until(rise).unwrap();
        agree(&c, &fast);
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        fast.run_until(fall).unwrap();
        agree(&c, &fast);
        if let Some(index) = cycle.checked_sub(65_536) {
            let sample = samples[index as usize];
            let mut bits = vec![0, 1]; // output, side input
            if bottom {
                bits.push(6);
            }
            bits.extend([4, 3, 2]); // LOADN, MOVE, DIRECTION
            let want: Vec<_> = bits
                .into_iter()
                .map(|bit| Logic::from_bool(sample & (1 << bit) != 0))
                .collect();
            assert_eq!(values(&c), want, "DEL_VALUE={value}, board sample {index}");
            if index < 609 {
                for (lane, end) in endpoint.iter_mut().enumerate() {
                    if end.is_none() && want[lane] == Logic::One {
                        *end = Some(index);
                    }
                }
            }
        }
    }
    let first_endpoint = if startup == 0 {
        0
    } else {
        18 + 4 * (u64::from(startup) - 1)
    };
    assert_eq!(endpoint, [Some(first_endpoint); 2]);
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi11_measured_startup_and_loaded_flags_have_equal_kernel_fast_and_board_traces() {
    // Measured values: side output/input, then bottom input zero where present.
    for (value, ident, startup) in [
        (1, 2, 0),
        (10, 0, 10),
        (63, 3, 62),
        (64, 4, 0),
        (100, 1, 36),
        (127, 5, 62),
    ] {
        measured_startup(value, ident, startup);
    }
}
