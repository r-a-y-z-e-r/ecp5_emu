//! Answering board peripherals through the real command-line fast path.
mod common;
use std::process::{Command, Output};

use common::reference::build;

fn command(name: &str, cycles: u64) -> Command {
    let out = build(name);
    let mut command = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
    command
        .arg("emulate")
        .arg(out.join("top.config"))
        .arg("--names")
        .arg(out.join("top_routed.json"))
        .args([
            "--fast",
            "--startup",
            "configured",
            "--scope",
            "observed",
            "--clock",
            "net:clk_25mhz",
            "--period-ns",
            "40",
            "--cycles",
        ])
        .arg(cycles.to_string())
        .args(["--console-out", "net:ftdi_rxd"])
        .env_remove("ECP5_SD_IMAGE")
        .env_remove("ECP5_FLASH_IMAGE")
        .env_remove("ECP5_RTC_IMAGE")
        .env_remove("ECP5_ADC_VOLTS")
        .env_remove("ECP5_EDID")
        .env_remove("ECP5_EDID_IMAGE")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0");
    command
}
fn text(output: Output) -> (String, String) {
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "CLI failed: {stdout}\n{stderr}");
    (stdout, stderr)
}
fn sdram_report(stderr: &str) {
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: sdram "))
        .expect("SDRAM report");
    assert!(
        line.starts_with("peripheral: sdram init=yes reads="),
        "{line}"
    );
    assert!(
        line.contains(" timing_violations=0 unknown_samples=0 refresh_shortfalls=0"),
        "{line}"
    );
    let fields: Vec<_> = line.split_whitespace().skip(2).collect();
    assert_eq!(fields.len(), 6, "{line}");
    assert!(!stderr.contains("peripheral: sdram message:"), "{stderr}");
    println!("{line}");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn uart_hello_prints_its_first_line_from_the_first_clock_edge() {
    let (stdout, _) = text(command("uart_hello", 1_600_000).output().unwrap());
    assert_eq!(stdout, "Hello, ULX3S!\r\n");
    println!("stdout: {stdout:?}");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn sdram_test_reads_the_model_through_the_cli() {
    let out = build("sdram_test");
    let vcd = out.join(format!("cli-{}.vcd", std::process::id()));
    let (stdout, stderr) = text(
        command("sdram_test", 1_000_000)
            .arg("--peripherals")
            .args(["--probe", "net:led", "--vcd"])
            .arg(&vcd)
            .output()
            .unwrap(),
    );
    assert_eq!(stdout, "SDRAM W8000 E0000\r\n");
    sdram_report(&stderr);
    assert!(!stderr.contains("Fast mode choice:"));
    assert!(!stderr.contains("Fast mode tuning:"));
    let wave = std::fs::read_to_string(vcd).unwrap();
    assert!(wave.contains("led[7]"));
    assert!(
        wave.contains("#0\n"),
        "VCD includes the initial probe values"
    );
    println!("stdout: {}", stdout.trim_end());
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn picorv32_sdram_prints_the_same_pass_zero_cycle_count() {
    let (stdout, stderr) = text(
        command("picorv32_sdram", 2_000_000)
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert!(stdout.starts_with("PicoRV32 from SDRAM\r\n"), "{stdout:?}");
    assert!(
        stdout.contains("pass 0: primes below 2000: 303, 855498 cycles\r\n"),
        "{stdout:?}"
    );
    sdram_report(&stderr);
    println!("stdout: {}", stdout.trim_end());
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn sd_read_uses_the_demo_card_through_the_cli() {
    let (stdout, stderr) = text(
        command("sd_read", 1_000_000)
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert_eq!(
        stdout,
        "SD OCR C0FF8000 B0 EB3C90454350355F454D550000000000 55AA\r\n"
    );
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: sd "))
        .expect("SD report");
    assert_eq!(line,"peripheral: sd state=ready last_command=17 last_argument=00000000 blocks_read=1 blocks_written=0 crc_errors=0 illegal_commands=0 mode=spi card_state=ready rca=0000 bus_width=1 commands=10 responses=10 dat3_pullup=0");
    println!("stdout: {}\n{line}", stdout.trim_end());
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn native_sd_reports_the_selected_bus_and_card_state() {
    let (stdout, stderr) = text(
        command("sd_native_read", 4_000_000)
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert!(stdout.contains("SDN B0 958745C6 CRC16 ok\r\n"));
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: sd "))
        .expect("native SD report");
    assert_eq!(line, "peripheral: sd state=ready last_command=17 last_argument=00000000 blocks_read=1 blocks_written=0 crc_errors=0 illegal_commands=0 mode=sd4 card_state=tran rca=0001 bus_width=4 commands=16 responses=15 dat3_pullup=1");
    println!("{line}");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn sdram_test_without_the_flag_reads_pulls_and_reports_errors() {
    let (stdout, stderr) = text(command("sdram_test", 1_000_000).output().unwrap());
    let errors = stdout
        .trim()
        .strip_prefix("SDRAM W8000 E")
        .expect("SDRAM error line");
    assert_ne!(
        u16::from_str_radix(errors, 16).unwrap(),
        0,
        "without an attached chip"
    );
    assert!(!stderr.contains("peripheral:"), "{stderr}");
    println!("without --peripherals: {}", stdout.trim_end());
}

#[test]
#[ignore = "requires yosys/nextpnr and a local Trellis database"]
fn event_kernel_refuses_peripherals_and_help_describes_the_reports() {
    let out = build("sd_read");
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("emulate")
        .arg(out.join("top.config"))
        .args(["--startup", "configured", "--peripherals"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--peripherals needs --fast"));
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--peripherals"));
    assert!(help.contains("peripheral: sdram init="));
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn audio_tone_reports_modelled_frequency_and_amplitude_through_the_cli() {
    let (stdout, stderr) = text(
        command("audio_tone", 26_000_000)
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert_eq!(stdout, "AUDIO 25000 F1BFDEEF\r\n");
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: audio "))
        .expect("audio report");
    let fields: Vec<_> = line.split_whitespace().skip(2).collect();
    let expected = [
        "l_hz",
        "l_vpp",
        "r_hz",
        "r_vpp",
        "v_hz",
        "v_vpp",
        "transitions",
    ];
    assert_eq!(fields.len(), expected.len());
    let mut numbers = Vec::new();
    for (field, key) in fields.iter().zip(expected) {
        let (name, value) = field.split_once('=').unwrap();
        assert_eq!(name, key);
        numbers.push(value);
    }
    for (index, target) in [(0, 440.0), (2, 660.0)] {
        assert!(
            (numbers[index].parse::<f64>().unwrap() - target).abs() < 1.0,
            "{line}"
        );
    }
    assert_eq!(numbers[1], "1.796938");
    assert_eq!(numbers[3], "1.796938");
    assert_eq!(numbers[4], "--");
    assert_eq!(numbers[5], "0.000000");
    assert!(numbers[6].parse::<u64>().unwrap() > 2000);
    assert!(!stderr.contains("Fast mode tuning:"));
    println!("stdout: {}\n{line}", stdout.trim_end());
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn edid_read_reports_the_monitor_eeprom_through_the_cli() {
    let bytes = ecp5_emu::peripherals::edid::default_block();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    let (stdout, stderr) = text(
        command("edid_read", 2_000_000)
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert_eq!(stdout, format!("EDID {hex} sum ok\r\n"));
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: edid "))
        .expect("EDID report");
    assert_eq!(
        line,
        "peripheral: edid present=yes reads=1 bytes_read=128 last_pointer=7F nacks=1"
    );
    println!("{line}");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn header_pwm_report_has_fixed_order_and_units() {
    let (stdout, stderr) = text(
        command("examples/ulx3s/header_probe/driven", 26_000_000)
            .env_remove("ECP5_HEADER")
            .arg("--peripherals")
            .output()
            .unwrap(),
    );
    assert!(stdout.starts_with("HEADER 000003E8 000007D0 00001388 00002710 P 000003E8/005F5E10"));
    let line = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: header "))
        .unwrap();
    assert!(
        line.contains("GP0=1000.000Hz/")
            && line.contains("GP1=2000.000Hz/")
            && line.contains("GP2=5000.000Hz/")
            && line.contains("GP3=10000.000Hz/"),
        "{line}"
    );
}
