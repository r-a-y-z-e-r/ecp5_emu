//! Real routed ULX3S bitstreams with a digital SDRAM on the package balls,
//! in both the event kernel and fast mode.
mod board;
mod common;
use board::{pin, PERIOD_PS};
use ecp5_emu::{
    compile::{self, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{DriverId, Options, SignalId, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::sdram::{Command as SdramCommand, Sdram, SdramPins},
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{path::PathBuf, sync::Arc, time::Instant};

const DQ: [&str; 16] = [
    "J16", "L18", "M18", "N18", "P18", "T18", "T17", "U20", "E19", "D20", "D19", "C20", "E18",
    "F18", "J18", "J17",
];
// Clock, enables, commands, bank, address, masks, DQ, UART, error LED.
const PADS: [&str; 49] = [
    "F19", "F20", "P20", "R20", "T19", "T20", "P19", "N20", "M20", "M19", "L20", "L19", "K20",
    "K19", "K18", "J20", "J19", "H20", "N19", "G20", "G19", "U19", "E20", "J16", "L18", "M18",
    "N18", "P18", "T18", "T17", "U20", "E19", "D20", "D19", "C20", "E18", "F18", "J18", "J17",
    "L4", "H3", "B2", "C2", "C1", "D2", "D1", "E2", "E1", "G2",
];
struct Board {
    c: Compiled,
    signals: Vec<SignalId>,
    dq: [DriverId; 16],
    clock: DriverId,
    chip: Sdram,
    drive: [Logic; 16],
    uart: Receiver,
    text: String,
}
impl Board {
    fn build(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let out = common::reference::build(name);
        let db = Database::discover(None).unwrap();
        let evidence = root.join("docs/inventory/LFE5U-85F");
        let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
        let image = Arc::new(ConfigurationImage::load(&out.join("top.config"), &db, None).unwrap());
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: std::iter::once((pin("G2"), Logic::Zero))
                    .chain(DQ.iter().map(|p| (pin(p), Logic::Z)))
                    .collect(),
                probes: PADS.iter().map(|p| pin(p)).collect(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
        .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let signals = PADS.iter().map(|p| c.probes[&pin(p).label()]).collect();
        let dq = std::array::from_fn(|i| c.drivers[&pin(DQ[i]).label()]);
        let clock = c.drivers["pin:G2"];
        println!(
            "{name}: compiled {} LUT4, {} FF, {} EBR, {} LUTRAM columns, {} PIO",
            c.report.counts.lut4,
            c.report.counts.flip_flops,
            c.report.counts.ebr_blocks,
            c.report.counts.lutram_columns,
            c.report.counts.pio
        );
        let mut b = Self {
            c,
            signals,
            dq,
            clock,
            chip: Sdram::new(),
            drive: [Logic::Z; 16],
            uart: Receiver::new(115200),
            text: String::new(),
        };
        b.sample(0);
        b
    }
    fn sample(&mut self, ps: u64) -> bool {
        let p: Vec<_> = self
            .signals
            .iter()
            .map(|&s| self.c.kernel.value(s).unwrap())
            .collect();
        let pins = SdramPins {
            clk: p[0],
            cke: p[1],
            csn: p[2],
            rasn: p[3],
            casn: p[4],
            wen: p[5],
            ba: p[6..8].try_into().unwrap(),
            a: p[8..21].try_into().unwrap(),
            dqm: p[21..23].try_into().unwrap(),
            dq: p[23..39].try_into().unwrap(),
        };
        let value = self.chip.feed(ps, &pins);
        let mask = self.chip.drive_mask();
        let drive = std::array::from_fn(|i| match value {
            Some(v) if mask & (1 << (i / 8)) != 0 => Logic::from_bool(v & (1 << i) != 0),
            _ => Logic::Z,
        });
        if let Some(byte) = self
            .uart
            .sample(ps / 1000, p[39].bit().expect("known UART"))
        {
            self.text.push(byte as char);
        }
        if drive == self.drive {
            return false;
        }
        self.drive = drive;
        let updates: Vec<_> = self.dq.iter().zip(drive).map(|(&d, v)| (d, v)).collect();
        // run_until closes its timestamp; the earliest legal delivery is 1 ps
        // later. It is still far ahead of the next 25 MHz capture edge.
        self.c
            .kernel
            .schedule_batch(Time::from_ps(ps + 1), &updates, None)
            .unwrap();
        true
    }
    fn edge(&mut self, ps: u64, level: Logic) {
        self.c
            .kernel
            .schedule(Time::from_ps(ps), self.clock, level)
            .unwrap();
        self.c.kernel.run_until(Time::from_ps(ps)).unwrap();
        if self.sample(ps) {
            self.c.kernel.run_until(Time::from_ps(ps + 1)).unwrap();
            assert!(!self.sample(ps + 1));
        }
    }
    fn cycle(&mut self, n: u64) {
        self.edge((2 * n - 1) * PERIOD_PS / 2, Logic::One);
        self.edge(n * PERIOD_PS, Logic::Zero);
    }
    fn report(&self, name: &str, n: u64, started: Instant) {
        let s = self.chip.status();
        println!("{name}: UART {:?}", self.text);
        println!(
            "{name}: {n} cycles in {:.3} s, {:.0} cycles/s",
            started.elapsed().as_secs_f64(),
            n as f64 / started.elapsed().as_secs_f64()
        );
        println!("{name}: status {s:?}");
    }
    fn healthy(&self) {
        let s = self.chip.status();
        assert!(s.initialised);
        assert_eq!(s.commands_before_init, 0, "{:?}", s.messages);
        assert_eq!(s.timing_violations, 0, "{:?}", s.messages);
        assert_eq!(self.uart.framing_errors, 0);
        assert_eq!(s.unknown_samples, 0);
        assert!(s.commands[&SdramCommand::Refresh] > 2);
    }
}
#[test]
#[ignore = "requires yosys/nextpnr and a local Trellis database"]
fn sdram_test_checks_all_64_kb() {
    let mut b = Board::build("sdram_test");
    let start = Instant::now();
    let mut n = 0;
    while !b.text.ends_with('\n') && n < 1_000_000 {
        n += 1;
        b.cycle(n);
        if n % 100_000 == 0 {
            println!(
                "sdram_test: {n} cycles, {:.0} cycles/s, {} reads, {} writes",
                n as f64 / start.elapsed().as_secs_f64(),
                b.chip.status().reads,
                b.chip.status().writes
            );
        }
    }
    b.report("sdram_test", n, start);
    assert_eq!(b.text, "SDRAM W8000 E0000\r\n");
    b.healthy();
    assert!(b.chip.status().reads >= 32768);
    assert_eq!(b.chip.status().bytes_touched, 65536);
    assert_eq!(b.c.kernel.value(b.signals[40]).unwrap(), Logic::Zero);
}
#[test]
#[ignore = "requires yosys/nextpnr and a local Trellis database"]
fn picorv32_executes_from_sdram() {
    let mut b = Board::build("picorv32_sdram");
    let start = Instant::now();
    let mut n = 0;
    while !(b.text.contains("primes below 2000:") && b.text.ends_with('\n')) && n < 5_000_000 {
        n += 1;
        b.cycle(n);
        if n % 100_000 == 0 {
            let rate = n as f64 / start.elapsed().as_secs_f64();
            println!(
                "picorv32_sdram: {n} cycles, {rate:.0} cycles/s, UART {:?}",
                b.text
            );
        }
        assert!(
            start.elapsed().as_secs() < 660,
            "CPU did not reach its first primes line in ten minutes"
        );
    }
    b.report("picorv32_sdram", n, start);
    b.healthy();
    assert!(b.text.starts_with("PicoRV32 from SDRAM\r\n"));
    assert!(b.text.contains("pass 0: primes below 2000: 303, "));
    assert!(b.text.contains(" cycles\r\n"));
}

fn to_logic(v: u8) -> Logic {
    match v {
        0 => Logic::Zero,
        1 => Logic::One,
        3 => Logic::Z,
        _ => Logic::X,
    }
}

fn from_logic(l: Logic) -> u8 {
    match l {
        Logic::Zero => 0,
        Logic::One => 1,
        Logic::X => 2,
        Logic::Z => 3,
    }
}

fn run_fast(
    name: &str,
    max_cycles: u64,
    done: impl Fn(&str) -> bool,
) -> (Sdram, Receiver, String, u8, u64, f64) {
    let mut b = Board::build(name);
    let mut prev_drive: [Option<bool>; 16] = [None; 16];
    let mut edge = 0u64;

    let mut feed_row = |chip: &mut Sdram,
                        uart: &mut Receiver,
                        text: &mut String,
                        row: &[u8],
                        ps: u64,
                        updates: &mut Vec<(usize, Option<bool>)>| {
        let pins = SdramPins {
            clk: to_logic(row[0]),
            cke: to_logic(row[1]),
            csn: to_logic(row[2]),
            rasn: to_logic(row[3]),
            casn: to_logic(row[4]),
            wen: to_logic(row[5]),
            ba: std::array::from_fn(|i| to_logic(row[6 + i])),
            a: std::array::from_fn(|i| to_logic(row[8 + i])),
            dqm: std::array::from_fn(|i| to_logic(row[21 + i])),
            dq: std::array::from_fn(|i| to_logic(row[23 + i])),
        };
        let value = chip.feed(ps, &pins);
        let mask = chip.drive_mask();
        if let Some(byte) = uart.sample(ps / 1000, to_logic(row[39]).bit().expect("known UART")) {
            text.push(byte as char);
        }
        for (i, previous) in prev_drive.iter_mut().enumerate() {
            let lvl = match value {
                Some(v) if mask & (1 << (i / 8)) != 0 => Some(v & (1 << i) != 0),
                _ => None,
            };
            if lvl != *previous {
                *previous = lvl;
                updates.push((i, lvl));
            }
        }
    };

    let kernel_row = |c: &Compiled, signals: &[SignalId]| -> Vec<u8> {
        signals
            .iter()
            .map(|&s| from_logic(c.kernel.value(s).unwrap()))
            .collect()
    };

    let row0 = kernel_row(&b.c, &b.signals);
    let mut updates = Vec::new();
    feed_row(
        &mut b.chip,
        &mut b.uart,
        &mut b.text,
        &row0,
        0,
        &mut updates,
    );
    assert!(updates.is_empty());

    let mut fast = loop {
        match FastSim::new(&b.c.kernel) {
            Ok(fast) => break fast,
            Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && edge < 2000 => {
                for level in [Logic::One, Logic::Zero] {
                    edge += 1;
                    let ps = edge * PERIOD_PS / 2;
                    let time = Time::from_ps(ps);
                    b.c.kernel.schedule(time, b.clock, level).unwrap();
                    b.c.kernel.run_until(time).unwrap();
                    let row = kernel_row(&b.c, &b.signals);
                    updates.clear();
                    feed_row(
                        &mut b.chip,
                        &mut b.uart,
                        &mut b.text,
                        &row,
                        ps,
                        &mut updates,
                    );
                    if !updates.is_empty() {
                        let k_updates: Vec<_> = updates
                            .iter()
                            .map(|&(i, lvl)| (b.dq[i], lvl.map_or(Logic::Z, Logic::from_bool)))
                            .collect();
                        let t1 = Time::from_ps(ps + 1);
                        b.c.kernel.schedule_batch(t1, &k_updates, None).unwrap();
                        b.c.kernel.run_until(t1).unwrap();
                    }
                }
            }
            Err(e) => panic!("{name}: {e:#}"),
        }
    };
    println!("{name} fast: warm start {} cycles", edge / 2);

    fast.clock(b.clock, Time::from_ps(PERIOD_PS)).unwrap();
    let start = Instant::now();
    let mut last_led7 = 2;
    let max_edges = max_cycles * 2;
    edge += fast
        .run_edges_with(max_edges - edge, &b.signals, &b.dq, |sample, _, updates| {
            if let Some((ps, row)) = sample {
                last_led7 = row[40];
                feed_row(&mut b.chip, &mut b.uart, &mut b.text, row, ps, updates);
            }
            Ok(!done(&b.text))
        })
        .unwrap();
    let seconds = start.elapsed().as_secs_f64();
    let cycles = edge / 2;
    let cps = cycles as f64 / seconds;
    println!(
        "{name} fast: {cycles} cycles in {seconds:.3} s, {cps:.0} cycles/s ({:.2} MHz)",
        cps / 1e6
    );
    println!("{name} fast: UART {:?}", b.text);
    println!("{name} fast: status {:?}", b.chip.status());
    (b.chip, b.uart, b.text, last_led7, cycles, cps)
}

fn check_healthy(chip: &Sdram, uart: &Receiver) {
    let s = chip.status();
    assert!(s.initialised);
    assert_eq!(s.commands_before_init, 0, "{:?}", s.messages);
    assert_eq!(s.timing_violations, 0, "{:?}", s.messages);
    assert_eq!(uart.framing_errors, 0);
    assert_eq!(s.unknown_samples, 0);
    assert!(s.commands[&SdramCommand::Refresh] > 2);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn fast_sdram_test_checks_all_64_kb() {
    let (chip, uart, text, led7, _, _) = run_fast("sdram_test", 1_000_000, |t| t.ends_with('\n'));
    assert_eq!(text, "SDRAM W8000 E0000\r\n");
    check_healthy(&chip, &uart);
    assert!(chip.status().reads >= 32768);
    assert_eq!(chip.status().bytes_touched, 65536);
    assert_eq!(led7, 0);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn fast_picorv32_executes_from_sdram() {
    // Two passes by default; SDRAM_PASSES=N runs N, to compare more of them
    // with the board (HR06).
    let passes: usize = std::env::var("SDRAM_PASSES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(2)
        .max(2);
    let last = format!("pass {}: memtest 256 KB:", passes - 1);
    let (chip, uart, text, _, _, _) = run_fast("picorv32_sdram", 40_000_000 * passes as u64, |t| {
        t.contains(&last) && t.ends_with('\n')
    });
    for line in text.lines() {
        println!("picorv32_sdram fast: {line}");
    }
    check_healthy(&chip, &uart);
    let lines: Vec<_> = text.lines().collect();
    assert!(lines.len() >= 5, "unexpected UART lines: {lines:?}");
    assert_eq!(lines[0], "PicoRV32 from SDRAM");
    let c0: u64 = lines[1]
        .strip_prefix("pass 0: primes below 2000: 303, ")
        .and_then(|s| s.strip_suffix(" cycles"))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("bad pass 0 primes line: {:?}", lines[1]));
    assert_eq!(lines[2], "pass 0: memtest 256 KB: 0 errors");
    let c1: u64 = lines[3]
        .strip_prefix("pass 1: primes below 2000: 303, ")
        .and_then(|s| s.strip_suffix(" cycles"))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("bad pass 1 primes line: {:?}", lines[3]));
    assert!(
        c0.abs_diff(c1) < 2000,
        "pass 0 ({c0}) and pass 1 ({c1}) cycle counts diverged"
    );
    // The board printed this very count for pass 1 (HR06; pass 0's line
    // comes before its serial port opens). The passes differ by the phase of
    // the controller's refresh counter when each starts.
    assert_eq!(c1, 854_997, "pass 1 cycles: the board counted 854997");
    assert_eq!(lines[4], "pass 1: memtest 256 KB: 0 errors");
    for pass in 2..passes {
        assert_eq!(
            lines[1 + 2 * pass],
            format!("pass {pass}: primes below 2000: 303, 855452 cycles")
        );
        assert_eq!(
            lines[2 + 2 * pass],
            format!("pass {pass}: memtest 256 KB: 0 errors")
        );
    }
}
