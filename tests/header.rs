//! Header logic probes and the inputs-only board reference.
mod common;
use ecp5_emu::board;
use std::{path::PathBuf, process::Command};

pub const FIRST: &str = "HEADER 00000001 00000002 00000005 0000000A 55555555555555 E1\r\n";
pub const DRIVEN_FIRST: &str = "HEADER 000003E8 000007D0 00001388 00002710 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1\r\n";
pub const DRIVEN_SECOND: &str = "HEADER 000007D0 00000FA0 00002710 00004E20 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1\r\n";
fn command(driven: bool) -> Command {
    command_cycles(driven, 400_000)
}
fn command_cycles(driven: bool, cycles: u64) -> Command {
    let out = common::reference::build(if driven {
        "examples/ulx3s/header_probe/driven"
    } else {
        "examples/ulx3s/header_probe"
    });
    let mut c = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
    c.arg("emulate")
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
        ])
        .arg(cycles.to_string())
        .args(["--console-out", "net:ftdi_rxd"])
        .env_remove("ECP5_HEADER")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0");
    c
}
fn run(c: &mut Command) -> (String, String) {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}
// The two public board captures are stored together; each design has
// two identical three-line loads. Keep the capture offset explicit below.
fn board_lines(driven: bool) -> Vec<String> {
    let log = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("docs/inventory/LFE5U-85F/hr13_header/observations.log"),
    )
    .unwrap();
    let heading = if driven {
        "# design header_probe/driven\n"
    } else {
        "# design header_probe\n"
    };
    let section = log
        .split_once(heading)
        .unwrap()
        .1
        .split("# design ")
        .next()
        .unwrap();
    let lines: Vec<_> = section
        .split_inclusive('\n')
        .filter(|l| l.ends_with('\n'))
        .filter_map(|line| line.trim_end().strip_prefix("  HEADER "))
        .map(|line| format!("HEADER {line}\r\n"))
        .collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(&lines[..3], &lines[3..]);
    lines[..3].to_vec()
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn board_safe_counters_and_pulls_are_deterministic() {
    let (text, report) = run(&mut command_cycles(false, 80_000_000));
    assert_eq!(text, format!("{FIRST}{}", board_lines(false).concat()));
    assert!(report.lines().any(|l| l == "peripheral: header"));
    println!("{}", text.trim());
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn driven_pad_readback_frequency_and_duty() {
    let (stdout, report) = run(&mut command_cycles(true, 80_000_000));
    println!("{}", stdout.trim());
    let board = board_lines(true);
    assert_eq!(board[0], DRIVEN_FIRST);
    assert_eq!(board[1], DRIVEN_SECOND);
    assert_eq!(stdout, board.concat());
    let fields: Vec<_> = stdout.split_whitespace().collect();
    assert_eq!(fields[5], "P");
    for (i, (rises, high)) in [
        (1000, 6_250_000),
        (2000, 12_500_000),
        (5000, 18_750_000),
        (10000, 2_500_000),
    ]
    .into_iter()
    .enumerate()
    {
        let (r, h) = fields[6 + i].split_once('/').unwrap();
        assert_eq!(u32::from_str_radix(r, 16).unwrap(), rises);
        assert_eq!(u32::from_str_radix(h, 16).unwrap(), high);
    }
    let line = report
        .lines()
        .find(|l| l.starts_with("peripheral: header "))
        .unwrap();
    for (i, (hz, duty)) in [
        (1000.0, 25.0),
        (2000.0, 50.0),
        (5000.0, 75.0),
        (10000.0, 10.0),
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("GP{i}=");
        let field = line
            .split_whitespace()
            .find_map(|s| s.strip_prefix(&key))
            .unwrap();
        let (f, d) = field.split_once("Hz/").unwrap();
        assert!((f.parse::<f64>().unwrap() - hz).abs() < 0.1, "{line}");
        assert!(
            (d.trim_end_matches('%').parse::<f64>().unwrap() - duty).abs() < 0.1,
            "{line}"
        );
    }
    println!("{line}");
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn cli_header_defaults_and_explicit_set_change_echo() {
    let (text, _) = run(command(false).env("ECP5_HEADER", "GN0=0"));
    assert!(text.ends_with("55555555555554 E0\r\n"), "{text}");
    let (text, _) = run(command(false)
        .env("ECP5_HEADER", "GN0=0")
        .args(["--set", "pin:C11=1"]));
    assert_eq!(text, FIRST);
    let stimulus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("header-stimulus-{}.json", std::process::id()));
    std::fs::write(&stimulus, r#"[{"time":500000,"set":{"pin:C11":"1"}}]"#).unwrap();
    let (text, _) = run(command(false)
        .env("ECP5_HEADER", "GN0=0")
        .arg("--stimulus")
        .arg(&stimulus));
    std::fs::remove_file(stimulus).unwrap();
    assert_eq!(text, FIRST);
}
fn normalized(s: &str) -> String {
    s.lines()
        .map(|l| l.split("//").next().unwrap())
        .collect::<String>()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}
#[test]
fn board_safe_top_has_only_input_header_ports_and_pulls() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/header_probe");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["header_logic.v", "top.v", "uart_tx.v"]);
    let top = normalized(&std::fs::read_to_string(root.join("top.v")).unwrap());
    assert!(top.contains("input[27:0]gp,input[27:0]gn,"));
    assert!(!top.contains("assign") && !top.contains("inout"));
    assert!(top.contains("header_logic#(.DRIVE_PWM(0))"));
    let lpf = std::fs::read_to_string(root.join("top.lpf")).unwrap();
    for pin in board::HEADER {
        let port = format!("{}[{}]", pin.name[..2].to_lowercase(), &pin.name[2..]);
        let pull = if pin.name.starts_with("GN") {
            "UP"
        } else {
            "DOWN"
        };
        assert!(lpf.contains(&format!("LOCATE COMP \"{port}\" SITE \"{}\";", pin.ball)));
        assert!(lpf.contains(&format!(
            "IOBUF PORT \"{port}\" PULLMODE={pull} IO_TYPE=LVCMOS33;"
        )));
    }
    // Seal the entire reviewed wrapper and its child: a new output port,
    // driver, instance or parameter path requires an explicit safety review.
    for (name, digest) in SAFETY_SEALS {
        let text = normalized(&std::fs::read_to_string(root.join(name)).unwrap());
        assert_eq!(
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            *digest,
            "review board safety in {name}"
        );
    }
}
const SAFETY_SEALS: &[(&str, &str)] = &[
    (
        "top.v",
        "566602f9781bbddc0ff8bd181c9d06b26a177bb8d774845ca1da3196a1efbe33",
    ),
    (
        "header_logic.v",
        "966bdd5d44e8195bb285aaefc44f4860d41fd4126b511ea59f77446bc626fc2c",
    ),
    (
        "uart_tx.v",
        "af9ccc8e35144a4615acbf203b20b2723722f48b49aafa018053e9fdeb3965a3",
    ),
    (
        "top.lpf",
        // ffdaaa8: comments/blank lines only; constraints identical to reviewed d8ccad6.
        "0c2bee44ab3f0801933ce7a210e4015915f0ae5a892946d99065cbb3043deb22",
    ),
];

#[test]
fn driven_top_only_drives_the_four_private_nets_at_drive_four() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/header_probe/driven");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["header_logic.v", "top.v", "uart_tx.v"]);
    let top = normalized(&std::fs::read_to_string(root.join("top.v")).unwrap());
    assert!(top.contains("input[27:4]gp,inout[3:0]gp_pwm,input[27:0]gn,"));
    assert!(!top.contains("assign"));
    assert_eq!(top.matches("BBgp").count(), 4);
    assert!(top.contains(".gp({gp,pad_read})"));
    for i in 0..4 {
        assert!(top.contains(&format!(
            "BBgp{i}_pad(.I(pwm[{i}]),.T(!DRIVE_PWM),.O(pad_read[{i}]),.B(gp_pwm[{i}]));"
        )));
    }
    let lpf = std::fs::read_to_string(root.join("top.lpf")).unwrap();
    assert_eq!(lpf.matches("DRIVE=4").count(), 4);
    for pin in board::HEADER {
        let driven = pin.name.starts_with("GP") && pin.name[2..].parse::<usize>().unwrap() < 4;
        let port = if driven {
            format!("gp_pwm[{}]", &pin.name[2..])
        } else {
            format!("{}[{}]", pin.name[..2].to_lowercase(), &pin.name[2..])
        };
        let pull = if pin.name.starts_with("GN") {
            "UP"
        } else {
            "DOWN"
        };
        let drive = if driven { " DRIVE=4" } else { "" };
        assert!(lpf.contains(&format!("LOCATE COMP \"{port}\" SITE \"{}\";", pin.ball)));
        assert!(lpf.contains(&format!(
            "IOBUF PORT \"{port}\" PULLMODE={pull} IO_TYPE=LVCMOS33{drive};"
        )));
    }
    for (name, digest) in DRIVEN_SEALS {
        let text = normalized(&std::fs::read_to_string(root.join(name)).unwrap());
        assert_eq!(
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            *digest,
            "review driven safety in {name}"
        );
    }
    for name in ["header_logic.v", "uart_tx.v"] {
        assert_eq!(
            std::fs::read(root.join(name)).unwrap(),
            std::fs::read(root.parent().unwrap().join(name)).unwrap()
        );
    }
}
const DRIVEN_SEALS: &[(&str, &str)] = &[
    (
        "top.v",
        // Privacy edit: comment only; constraints/logic identical to reviewed 77babf9.
        "da2d1496491516845e3576c05ff32d0e07fb91e4eee227a6c71218ce64c20b1f",
    ),
    (
        "header_logic.v",
        "966bdd5d44e8195bb285aaefc44f4860d41fd4126b511ea59f77446bc626fc2c",
    ),
    (
        "uart_tx.v",
        "af9ccc8e35144a4615acbf203b20b2723722f48b49aafa018053e9fdeb3965a3",
    ),
    (
        "top.lpf",
        // ffdaaa8: comments/blank lines only; constraints identical to reviewed 2fc0c73.
        "82e2958fc68451ce510b8705c84f9a7930f9c20bf376da691f1a0d64030e4518",
    ),
];

#[test]
fn driven_gp0_to_gp3_only_connect_the_fpga_and_j1_in_the_pcb() {
    let pcb = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/ulx3s/ulx3s_v3.0.8.kicad_pcb"),
    )
    .unwrap();
    if pcb.lines().next() == Some("version https://git-lfs.github.com/spec/v1") {
        println!(
            "PCB is an LFS pointer in this checkout; PCB cross-check skipped (fetch it with `git lfs pull` to run it)"
        );
        return;
    }
    // This PCB uses one top-level module per footprint. Read each pad's
    // net through its closing parenthesis, ignoring tracks and zones.
    let modules = pcb.split("\n  (module ").skip(1);
    assert!(
        modules.clone().next().is_some(),
        "PCB cross-check found no modules in the real PCB"
    );
    for (i, ball) in ["B11", "A10", "A9", "B9"].into_iter().enumerate() {
        let name = format!("GP{i}");
        let mut endpoints = Vec::new();
        for module in modules.clone() {
            let reference = module
                .split("(fp_text reference ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            for pad in module.split("\n    (pad ").skip(1) {
                let Some(net) = pad.split("(net ").nth(1) else {
                    continue;
                };
                if net.split(')').next().unwrap().split_whitespace().nth(1) == Some(name.as_str()) {
                    endpoints.push((
                        reference.to_owned(),
                        pad.split_whitespace().next().unwrap().to_owned(),
                    ));
                }
            }
        }
        endpoints.sort();
        assert_eq!(
            endpoints,
            vec![
                ("J1".into(), (6 + 2 * i).to_string()),
                ("U1".into(), ball.into())
            ],
            "{name}: another part would forbid this driven reference"
        );
    }
}
