//! Real routed flash designs, with answers delivered 1 ps after each SCK edge.
mod common;
use ecp5_emu::{
    board,
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::{FastSim, Trace},
    image::ConfigurationImage,
    kernel::{DriverId, Options, SignalId, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::{
        spiflash::{Pins, SpiFlash, CAPACITY},
        video::crc32,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{fmt::Write, path::PathBuf, sync::Arc, time::Instant};

fn pin(ball: &str) -> Reference {
    Reference::Pin(ball.into())
}

struct Board {
    c: Compiled,
    probes: Vec<SignalId>,
    clock: DriverId,
    miso: DriverId,
    chip: SpiFlash,
    last_miso: u8,
    uart: Receiver,
    text: String,
    edge: u64,
}
impl Board {
    fn build(name: &str, chip: SpiFlash) -> Self {
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
        assert_eq!(
            board::direction(&device, board::FLASH_SCK).unwrap(),
            Some(board::Direction::Output)
        );
        assert!(board::used_balls(&device)
            .unwrap()
            .iter()
            .any(|(ball, direction)| ball == board::FLASH_SCK
                && *direction == board::Direction::Output));
        let refs = vec![
            pin(board::FLASH_CSN),
            pin(board::FLASH_SCK),
            pin(board::FLASH_MOSI),
            pin(board::UART_TX),
            pin(board::LEDS[0]),
            pin(board::FLASH_WPN),
            pin(board::FLASH_HOLDN),
        ];
        let c = compile::compile(
            device,
            Request {
                package: board::PACKAGE.into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: vec![
                    (pin(board::CLOCK), Logic::Zero),
                    (pin(board::FLASH_MISO), Logic::One),
                ],
                probes: refs.clone(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
        .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let probes = refs.iter().map(|r| c.probes[&r.label()]).collect();
        let clock = c.drivers[&pin(board::CLOCK).label()];
        let miso = c.drivers[&pin(board::FLASH_MISO).label()];
        Self {
            c,
            probes,
            clock,
            miso,
            chip,
            last_miso: 1,
            uart: Receiver::new(board::BAUD),
            text: String::new(),
            edge: 0,
        }
    }
    fn sample(&mut self, row: &[u8], ps: u64) {
        self.chip.feed(
            ps,
            Pins {
                cs: row[0],
                clk: row[1],
                mosi: row[2],
            },
        );
        if let Some(byte) = self.uart.sample(ps / 1000, row[3] == 1) {
            self.text.push(byte as char);
        }
    }
    fn event_edge(&mut self) {
        self.edge += 1;
        let ps = self.edge * 20_000;
        self.c
            .kernel
            .schedule(
                Time::from_ps(ps),
                self.clock,
                Logic::from_bool(self.edge & 1 == 1),
            )
            .unwrap();
        self.c.kernel.run_until(Time::from_ps(ps)).unwrap();
        let row: Vec<_> = self
            .probes
            .iter()
            .map(|&s| self.c.kernel.value(s).unwrap() as u8)
            .collect();
        self.sample(&row, ps);
        if self.chip.miso() != self.last_miso {
            self.last_miso = self.chip.miso();
            self.c
                .kernel
                .schedule(
                    Time::from_ps(ps + 1),
                    self.miso,
                    Logic::from_bool(self.last_miso == 1),
                )
                .unwrap();
            self.c.kernel.run_until(Time::from_ps(ps + 1)).unwrap();
        }
    }
    fn run(&mut self, max_cycles: u64, done: impl Fn(&str) -> bool) {
        let mut fast = loop {
            match FastSim::new(&self.c.kernel) {
                Ok(f) => break f,
                Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && self.edge < 2000 => {
                    self.event_edge();
                    self.event_edge();
                }
                Err(e) => panic!("{e:#}"),
            }
        };
        fast.clock(self.clock, Time::from_ps(40_000)).unwrap();
        fast.trace(&self.probes[..3]).unwrap();
        let start = Instant::now();
        while self.edge < max_cycles * 2 && !done(&self.text) {
            if self.chip.selected() {
                let ran = fast
                    .run_edges_with(
                        1024,
                        &self.probes[3..4],
                        &[self.miso],
                        |sample, trace, changes| {
                            feed(&mut self.chip, trace);
                            if let Some((ps, row)) = sample {
                                if let Some(byte) = self.uart.sample(ps / 1000, row[0] == 1) {
                                    self.text.push(byte as char);
                                }
                            }
                            if self.chip.miso() != self.last_miso {
                                self.last_miso = self.chip.miso();
                                changes.push((0, Some(self.last_miso == 1)));
                            }
                            Ok(self.chip.selected())
                        },
                    )
                    .unwrap();
                self.edge += ran;
            } else {
                let rows = fast.run_edges_sampled(64, &self.probes[3..4]).unwrap();
                let trace = fast.take_trace();
                feed(&mut self.chip, &trace);
                let end = fast.now().unwrap().ratio().0;
                for (i, &level) in rows.iter().enumerate() {
                    let ps = end - (rows.len() - 1 - i) as u64 * 20_000;
                    if let Some(byte) = self.uart.sample(ps / 1000, level == 1) {
                        self.text.push(byte as char);
                    }
                }
                self.edge += 64;
                if self.chip.miso() != self.last_miso {
                    self.last_miso = self.chip.miso();
                    fast.set_at(Time::from_ps(end + 1), &[(self.miso, self.last_miso == 1)])
                        .unwrap();
                }
            }
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "flash: {} cycles in {secs:.3} s, {:.0} cycles/s",
            self.edge / 2,
            self.edge as f64 / 2.0 / secs
        );
        for line in self.text.lines().filter(|l| {
            !l.split_once(':')
                .is_some_and(|(a, _)| a.len() == 6 && a.bytes().all(|b| b.is_ascii_hexdigit()))
        }) {
            println!("UART: {line}");
        }
        assert!(done(&self.text), "timed out: {}", self.text);
        assert_eq!(self.uart.framing_errors, 0);
        assert_eq!(self.chip.status().ignored_writes, 0);
    }
}
fn feed(chip: &mut SpiFlash, trace: &Trace) {
    for (&ps, row) in trace.times_ps.iter().zip(trace.rows.chunks_exact(3)) {
        chip.feed(
            ps,
            Pins {
                cs: row[0],
                clk: row[1],
                mosi: row[2],
            },
        );
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn usrmclk_pad_probe_follows_the_routed_clock_in_both_executors() {
    let mut b = Board::build("flash_dump", SpiFlash::demo());
    // LED0 is wired to the same fabric SCK as USRMCLKI in this reference design.
    let mut fast = FastSim::new(&b.c.kernel).unwrap();
    let mut highs = 0;
    for _ in 0..256 {
        b.edge += 1;
        let time = Time::from_ps(b.edge * 20_000);
        let level = b.edge & 1 == 1;
        b.c.kernel
            .schedule(time, b.clock, Logic::from_bool(level))
            .unwrap();
        b.c.kernel.run_until(time).unwrap();
        fast.set_at(time, &[(b.clock, level)]).unwrap();
        for &signal in &b.probes {
            assert_eq!(fast.value(signal), Some(b.c.kernel.value(signal).unwrap()));
        }
        assert_eq!(
            b.c.kernel.value(b.probes[1]).unwrap(),
            b.c.kernel.value(b.probes[4]).unwrap()
        );
        highs += usize::from(fast.value(b.probes[1]) == Some(Logic::One));
        assert_eq!(fast.value(b.probes[5]), Some(Logic::One));
        assert_eq!(fast.value(b.probes[6]), Some(Logic::One));
    }
    assert!(highs > 10 && highs < 200);
    println!("pin:U3: event/fast agree, CCLK follows the routed SCK");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn flash_dump_prints_both_exact_windows_and_crc() {
    let mut image = vec![0xff; CAPACITY];
    for (i, byte) in image.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(37).wrapping_add((i >> 16) as u8);
    }
    let mut expected = String::from("FLASH ID 9D6018 SR 00\r\n");
    let mut windows = Vec::new();
    for range in [0..0x10000, 0x200000..0x201000] {
        windows.extend_from_slice(&image[range.clone()]);
        for address in range.step_by(32) {
            write!(expected, "{address:06X}: ").unwrap();
            for byte in &image[address..address + 32] {
                write!(expected, "{byte:02X}").unwrap();
            }
            expected.push_str("\r\n");
        }
    }
    writeln!(expected, "CRC {:08X}\r", crc32(&windows)).unwrap();
    let mut b = Board::build("flash_dump", SpiFlash::new(image));
    b.run(380_000_000, |t| t.ends_with('\n') && t.contains("CRC "));
    assert_eq!(b.text, expected);
    println!("UART first: {}", b.text.lines().nth(1).unwrap());
    println!("UART last: {}", b.text.lines().rev().nth(1).unwrap());
}

fn flash_dump_windows() -> Vec<usize> {
    (0..0x10000)
        .step_by(32)
        .chain((0x200000..0x201000).step_by(32))
        .collect()
}

fn parse_hex_bytes(hex: &str, out: &mut [u8]) -> bool {
    if hex.len() != out.len() * 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
    }
    true
}

fn parse_board_dump(log: &str) -> Option<(u8, Vec<u8>, Vec<String>)> {
    let addrs = flash_dump_windows();
    let mut current: Option<(u8, Vec<u8>, usize, Vec<String>)> = None;
    for raw in log.lines() {
        let line = raw.trim_end_matches('\r');
        let Some(uart) = line.strip_prefix("  ") else {
            current = None;
            continue;
        };
        if let Some(rest) = uart.strip_prefix("FLASH ID ") {
            if rest.len() == 12
                && rest[..6].bytes().all(|b| b.is_ascii_hexdigit())
                && &rest[6..10] == " SR "
                && rest[10..12].bytes().all(|b| b.is_ascii_hexdigit())
            {
                let sr = u8::from_str_radix(&rest[10..12], 16).unwrap();
                current = Some((sr, vec![0xff; CAPACITY], 0, vec![uart.to_string()]));
            } else {
                current = None;
            }
            continue;
        }
        let Some((sr, image, idx, lines)) = &mut current else {
            continue;
        };
        if *idx < addrs.len() {
            let addr = addrs[*idx];
            let prefix = format!("{addr:06X}: ");
            if let Some(hex) = uart.strip_prefix(&prefix) {
                if parse_hex_bytes(hex, &mut image[addr..addr + 32]) {
                    lines.push(uart.to_string());
                    *idx += 1;
                    continue;
                }
            }
            current = None;
        } else if let Some(crc_hex) = uart.strip_prefix("CRC ") {
            if crc_hex.len() == 8 && crc_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                lines.push(uart.to_string());
                return Some((*sr, std::mem::take(image), std::mem::take(lines)));
            }
            current = None;
        } else {
            current = None;
        }
    }
    None
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_board_dump_reproduces_in_the_emulator() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dump_path = root.join("out/flash/board_dump.log");
    let log = match std::fs::read_to_string(&dump_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "skipping the_board_dump_reproduces_in_the_emulator: {} not found; \
                 put the output of docs/inventory/LFE5U-85F/hr04_hdmi_oled/run.py for flash_dump there",
                dump_path.display()
            );
            return;
        }
        Err(e) => panic!("failed to read {}: {e}", dump_path.display()),
    };
    let (sr, image, expected_lines) = parse_board_dump(&log)
        .unwrap_or_else(|| panic!("no complete flash_dump found in {}", dump_path.display()));
    let mut b = Board::build("flash_dump", SpiFlash::with_status(image, sr));
    b.run(380_000_000, |t| t.ends_with('\n') && t.contains("CRC "));
    let actual_lines: Vec<&str> = b.text.lines().collect();
    assert_eq!(actual_lines, expected_lines);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn picorv32_flash_executes_demo_firmware_in_place_twice() {
    let mut b = Board::build("picorv32_flash", SpiFlash::demo());
    b.run(30_000_000, |t| {
        t.lines()
            .any(|l| l.starts_with("pass 1: primes below 2000: 303, ") && l.ends_with(" cycles"))
            && t.ends_with('\n')
    });
    assert!(b.text.starts_with("PicoRV32 flash ID 9D6018\r\n"));
    for pass in 0..2 {
        assert!(b.text.contains(&format!(
            "pass {pass}: primes below 2000: 303, 5849188 cycles\r\n"
        )));
    }
    assert!(b.chip.status().last_address.unwrap() >= 0x200000);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn picorv32_flash_reports_missing_header() {
    let mut b = Board::build("picorv32_flash", SpiFlash::new(Vec::new()));
    b.run(1_000_000, |t| {
        t.contains("no firmware at 200000:") && t.ends_with('\n')
    });
    assert_eq!(
        b.text,
        "PicoRV32 flash ID 9D6018\r\nno firmware at 200000: FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\r\n"
    );
}

// This check needs no FPGA or RISC-V toolchain. Follow symlinks, including
// the shared controller and the unchanged PicoRV32 sources in both designs.
fn verilog_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            verilog_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "v") {
            out.push(path);
        }
    }
}
fn command_constants(source: &str) -> Vec<(String, u32)> {
    // Tokenise after removing both kinds of Verilog comment. A based literal
    // remains one token; underscores and upper-case bases are accepted.
    let bytes = source.as_bytes();
    let mut clean = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            i += 2;
            while i + 1 < bytes.len() && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            clean.push(' ');
        } else {
            clean.push(bytes[i] as char);
            i += 1;
        }
    }
    let mut found = Vec::new();
    for statement in clean.split(';') {
        if !statement.contains("param") {
            continue;
        }
        for declaration in statement.split(',') {
            let Some((lhs, rhs)) = declaration.split_once('=') else {
                continue;
            };
            let name = lhs
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .rfind(|s| !s.is_empty())
                .unwrap_or("");
            let lower = name.to_ascii_lowercase();
            if ![
                "cmd",
                "command",
                "opcode",
                "wren",
                "wrdi",
                "wrsr",
                "erase",
                "program",
                "write_status",
                "status_write",
                "write_enable",
                "write_disable",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                continue;
            }
            let literal: String = rhs
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '_')
                .collect();
            let literal = literal.trim_matches(['(', ')']).to_ascii_lowercase();
            let value = if let Some((_, based)) = literal.split_once('\'') {
                let based = based.strip_prefix('s').unwrap_or(based);
                let (base, digits) = based.split_at(1);
                u32::from_str_radix(
                    digits,
                    match base {
                        "h" => 16,
                        "b" => 2,
                        "o" => 8,
                        "d" => 10,
                        _ => panic!("unknown command base"),
                    },
                )
                .unwrap()
            } else {
                literal
                    .parse()
                    .expect("command constant must be a literal for the safety audit")
            };
            found.push((name.into(), value));
        }
    }
    found
}
#[test]
fn reference_flash_verilog_has_only_read_only_command_constants() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s");
    let mut files = Vec::new();
    for design in ["flash_dump", "picorv32_flash"] {
        verilog_files(&root.join(design), &mut files);
    }
    let forbidden = [
        0x06, 0x04, 0x02, 0x20, 0x52, 0xd8, 0xc7, 0x60, 0x01, 0x32, 0x38, 0xad, 0x31, 0x11, 0x50,
        0x42, 0x44, 0x7e, 0x98, 0x36, 0x39, 0xa6, 0x68, 0xb1, 0xc1,
    ];
    let mut count = 0;
    for file in &files {
        for (name, value) in command_constants(&std::fs::read_to_string(file).unwrap()) {
            assert!(
                !forbidden.contains(&value),
                "{}: {name} has forbidden write/erase/status opcode {value:02X}",
                file.display()
            );
            assert!(
                [0x03, 0x0b, 0x05, 0x9f].contains(&value),
                "{}: {name} is outside the read-only allowlist",
                file.display()
            );
            count += 1;
        }
    }
    assert_eq!(
        count, 6,
        "both designs must contain the three audited commands"
    );
    assert_eq!(
        files.len(),
        7,
        "scan every Verilog file, including symlinks"
    );
}
#[test]
fn command_scan_catches_write_constants_in_all_literal_bases() {
    let source = "// localparam CMD_FAKE=8'h9f;\nlocalparam [7:0] CMD_WREN = 8'H06, opcode_erase = 8'b0010_0000; /* ignored */ parameter command_status = 8'd1; localparam CMD_PROGRAM = 2;";
    assert_eq!(
        command_constants(source),
        [
            ("CMD_WREN".into(), 6),
            ("opcode_erase".into(), 32),
            ("command_status".into(), 1),
            ("CMD_PROGRAM".into(), 2)
        ]
    );
}
#[test]
fn board_dump_parser_skips_partial_fragments_and_extracts_first_complete_dump() {
    let mut log =
        String::from("load 1 at 2026-11-16T00:00:00Z: loader_done=True lines=2\n  00FF00: ");
    log.push_str(&"AA".repeat(32));
    log.push_str("\n  FLASH ID 9D6018 SR 00\n  000000: ");
    log.push_str(&"00".repeat(32));
    log.push_str(
        "\nload 2 at 2026-11-16T00:00:05Z: loader_done=True lines=2178\n  FLASH ID 9D6018 SR 02\n",
    );
    for addr in flash_dump_windows() {
        let byte = ((addr >> 5) & 0xff) as u8;
        writeln!(log, "  {addr:06X}: {}", format!("{byte:02X}").repeat(32)).unwrap();
    }
    log.push_str("  CRC 12345678\n");
    let (sr, image, lines) = parse_board_dump(&log).unwrap();
    assert_eq!(sr, 2);
    assert_eq!(lines.len(), 2178);
    assert_eq!(image[0], 0x00);
    assert_eq!(image[32], 0x01);
    assert_eq!(image[0x10000], 0xff);
    assert_eq!(image[0x200000], 0x00);
    assert_eq!(image[0x201000], 0xff);
}

/// Exercise Attached's CLI report with Rust pad samples, never a writing FPGA design.
#[test]
fn rust_only_configuration_write_reaches_the_report_and_rewinds() {
    use ecp5_emu::peripherals::{spiflash::T_PP_PS, Attached};
    let used = std::collections::BTreeMap::from([
        (board::FLASH_CSN.into(), board::Direction::Output),
        (board::FLASH_SCK.into(), board::Direction::Output),
        (board::FLASH_MOSI.into(), board::Direction::Output),
        (board::FLASH_MISO.into(), board::Direction::Input),
    ]);
    let mut attached = Attached::new(&used);
    assert_eq!(
        attached.balls(),
        [board::FLASH_CSN, board::FLASH_SCK, board::FLASH_MOSI]
    );
    let saved = attached.snapshot();
    let mut time = 0;
    let sample = |a: &mut Attached, time: &mut u64, cs, clk, mosi| {
        *time += 10_000;
        a.feed(&Trace {
            width: 3,
            times_ps: vec![*time],
            rows: vec![cs, clk, mosi],
            gap: false,
        });
    };
    for command in [&[0x06][..], &[0x02, 0, 0, 0, 0xa5][..]] {
        sample(&mut attached, &mut time, 1, 0, 1);
        sample(&mut attached, &mut time, 0, 0, 1);
        for byte in command {
            for bit in (0..8).rev() {
                let mosi = (byte >> bit) & 1;
                sample(&mut attached, &mut time, 0, 0, mosi);
                sample(&mut attached, &mut time, 0, 1, mosi);
            }
        }
        sample(&mut attached, &mut time, 1, 0, 1);
    }
    let lines = attached.report();
    assert_eq!(lines[0],"peripheral: flash jedec=9D6018 last_command=02 last_address=000000 bytes_read=0 ignored_writes=0 programs=0 erases=0 protected=0 busy_ignored=0 config_writes=1");
    assert_eq!(lines[1],"peripheral: flash warning: command 02 range [000000,000001): this would overwrite the board's FPGA configuration (bitstream) on a real ULX3S");
    time += T_PP_PS;
    attached.feed(&Trace {
        width: 3,
        times_ps: vec![time],
        rows: vec![1, 0, 1],
        gap: false,
    });
    assert_eq!(attached.view().flash.unwrap().programs, 1);
    assert!(attached.report()[0]
        .ends_with("programs=1 erases=0 protected=0 busy_ignored=0 config_writes=1"));
    attached.restore(&saved);
    let lines = attached.report();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with("programs=0 erases=0 protected=0 busy_ignored=0 config_writes=0"));
}

#[test]
fn cli_help_describes_memory_only_writes_and_configuration_warnings() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("programs=N erases=N protected=N busy_ignored=N config_writes=N"));
    assert!(help.contains("Flash writes change memory only"));
    assert!(help.contains("peripheral: flash warning: TEXT"));
    assert!(help.contains("SRAM loads only"));
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn read_only_xip_cli_keeps_the_report_prefix_and_appends_write_counters() {
    let out = common::reference::build("picorv32_flash");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("emulate")
        .arg(out.join("top.config"))
        .arg("--names")
        .arg(out.join("top_routed.json"))
        .args([
            "--fast",
            "--peripherals",
            "--startup",
            "configured",
            "--scope",
            "observed",
            "--clock",
            "net:clk_25mhz",
            "--period-ns",
            "40",
            "--cycles",
            "200000",
            "--console-out",
            "net:ftdi_rxd",
        ])
        .env_remove("ECP5_FLASH_IMAGE")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    assert!(
        stdout.starts_with("PicoRV32 flash ID 9D6018\r\n"),
        "{stdout:?}"
    );
    let report = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: flash jedec="))
        .expect("flash report");
    let keys: Vec<_> = report
        .split_whitespace()
        .skip(2)
        .map(|field| field.split_once('=').unwrap().0)
        .collect();
    assert_eq!(
        keys,
        [
            "jedec",
            "last_command",
            "last_address",
            "bytes_read",
            "ignored_writes",
            "programs",
            "erases",
            "protected",
            "busy_ignored",
            "config_writes"
        ]
    );
    assert!(
        report.ends_with(
            "ignored_writes=0 programs=0 erases=0 protected=0 busy_ignored=0 config_writes=0"
        ),
        "{report}"
    );
    assert!(!stderr.contains("peripheral: flash warning:"));
    println!("{report}");
}
