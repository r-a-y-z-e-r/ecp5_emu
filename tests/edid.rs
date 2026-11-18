//! Read-only DDC through the CLI answering loop; private monitor data stays in out/edid.
mod common;
use ecp5_emu::peripherals::edid::default_block;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
fn line(bytes: &[u8]) -> String {
    let sum = bytes.iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
    format!(
        "EDID {} sum {}\r\n",
        hex(bytes),
        if sum == 0 { "ok" } else { "bad" }
    )
}
fn command() -> Command {
    let out = common::reference::build("edid_read");
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
            "2000000",
            "--console-out",
            "net:ftdi_rxd",
        ])
        .env_remove("ECP5_EDID")
        .env_remove("ECP5_EDID_IMAGE")
        .env_remove("ECP5_RTC_IMAGE")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0");
    c
}
fn run(mut c: Command) -> (String, String) {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "EDID CLI failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn edid_read_prints_the_synthetic_base_block() {
    let (stdout, stderr) = run(command());
    assert_eq!(stdout, line(&default_block()));
    assert!(
        stderr.contains(
            "peripheral: edid present=yes reads=1 bytes_read=128 last_pointer=7F nacks=1"
        ),
        "{stderr}"
    );
    let hdmi = stderr
        .lines()
        .find(|l| l.starts_with("peripheral: hdmi "))
        .expect("DVI receiver");
    assert!(!hdmi.starts_with("peripheral: hdmi frames=0 "), "{hdmi}");
    println!("EDID {}... sum ok", &hex(&default_block())[..32]);
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn edid_off_nacks_while_the_source_still_drives_dvi() {
    let mut c = command();
    c.env("ECP5_EDID", "off");
    let (stdout, stderr) = run(c);
    assert_eq!(stdout, "EDID nack\r\n");
    assert!(
        stderr.contains("peripheral: edid present=no reads=0 bytes_read=0 last_pointer=00 nacks=1"),
        "{stderr}"
    );
}

enum Capture {
    Nack,
    Image([u8; 128], String),
}
fn board_read(log: &str) -> Option<Capture> {
    for line in log.split_inclusive('\n').filter(|l| l.ends_with('\n')) {
        let Some(text) = line.trim_end().strip_prefix("  EDID ") else {
            continue;
        };
        if text == "nack" {
            return Some(Capture::Nack);
        }
        let Some((raw, sum)) = text.split_once(" sum ") else {
            continue;
        };
        if raw.len() != 256
            || !raw.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(sum, "ok" | "bad")
        {
            continue;
        }
        let bytes =
            std::array::from_fn(|i| u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16).unwrap());
        return Some(Capture::Image(bytes, format!("EDID {text}\r\n")));
    }
    None
}
struct PrivateImage(PathBuf);
impl PrivateImage {
    fn new(folder: &Path, bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        fs::create_dir_all(folder).unwrap();
        loop {
            let path = folder.join(format!(
                "replay-{}-{}.bin",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    file.write_all(bytes).unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("private EDID replay image: {e}"),
            }
        }
    }
}
impl Drop for PrivateImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "requires a private out/edid/board_edid.log, clang and the reference tools"]
fn the_board_read_reproduces_in_the_emulator() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/edid");
    let path = folder.join("board_edid.log");
    let log = match fs::read_to_string(&path) {
        Ok(log) => log,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipping EDID board replay: private out/edid/board_edid.log missing");
            return;
        }
        Err(e) => panic!("EDID board log: {e}"),
    };
    match board_read(&log).expect("first complete indented EDID report") {
        Capture::Nack => {
            let mut c = command();
            c.env("ECP5_EDID", "off");
            assert_eq!(run(c).0, "EDID nack\r\n");
        }
        Capture::Image(bytes, expected) => {
            let image = PrivateImage::new(&folder, &bytes);
            let mut c = command();
            c.env("ECP5_EDID_IMAGE", &image.0);
            let actual = run(c).0;
            // Never put a monitor's identifying EDID into an assertion dump.
            assert!(
                actual == expected,
                "private EDID report differs from the emulator"
            );
        }
    }
}
#[test]
fn private_log_parser_requires_an_indented_newline_terminated_full_result() {
    let text = line(&default_block());
    assert!(board_read(&text).is_none());
    assert!(board_read(&format!("  {}", text.trim_end())).is_none());
    assert!(board_read("  EDID 00 sum ok\n").is_none());
    assert!(matches!(
        board_read("load 1\n  EDID nack\n"),
        Some(Capture::Nack)
    ));
    match board_read(&format!("load 1\n  {text}")) {
        Some(Capture::Image(bytes, _)) => assert_eq!(bytes, default_block()),
        _ => panic!("complete synthetic EDID not parsed"),
    }
}
