//! Native SD through the pad-fed fast CLI. Real CID and B0RAW stay in out/sd.
mod common;
use ecp5_emu::peripherals::sdcard::{
    native::{cid, csd},
    SdCard,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
fn command() -> Command {
    let out = common::reference::build("sd_native_read");
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
            "4000000",
            "--console-out",
            "net:ftdi_rxd",
        ])
        .env_remove("ECP5_SD_IMAGE")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0");
    c
}
fn run(mut c: Command) -> (String, String) {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "native CLI failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02X}")).collect()
}
fn crc32(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in b {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ if c & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !c
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn sd_native_read_prints_the_demo_registers_and_block_crc() {
    let block = SdCard::demo().image()[..512].to_vec();
    let expected=format!("SDN OCR C0FF8000 RCA 0001 CSD {}\r\nSDN CID {}\r\nSDN B0 {:08X} CRC16 ok\r\nSDN B0RAW {}\r\n",hex(&csd(131072)),hex(&cid()),crc32(&block),hex(&block));
    let (text, status) = run(command());
    assert_eq!(text, expected, "synthetic demo only");
    assert!(status.contains("blocks_read=1 blocks_written=0 crc_errors=0 illegal_commands=0 mode=sd4 card_state=tran rca=0001 bus_width=4 commands=16 responses=15 dat3_pullup=1"),"{status}");
    println!("{}", text.lines().take(3).collect::<Vec<_>>().join("\n"));
}
struct Capture {
    block: [u8; 512],
    crc_line: String,
}
fn board_read(log: &str) -> Option<Capture> {
    let mut raw = None;
    let mut crc = None;
    for line in log.split_inclusive('\n').filter(|l| l.ends_with('\n')) {
        let Some(line) = line.trim_end().strip_prefix("  SDN ") else {
            continue;
        };
        if let Some(hex) = line.strip_prefix("B0RAW ") {
            if raw.is_none() && hex.len() == 1024 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                raw = Some(std::array::from_fn(|i| {
                    u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()
                }));
            }
        } else if let Some(check) = line.strip_prefix("B0 ") {
            if crc.is_none()
                && check.len() == 17
                && check.ends_with(" CRC16 ok")
                && check[..8].bytes().all(|c| c.is_ascii_hexdigit())
            {
                crc = Some(format!("SDN {line}"));
            }
        }
        if raw.is_some() && crc.is_some() {
            break;
        }
    }
    Some(Capture {
        block: raw?,
        crc_line: crc?,
    })
}
struct PrivateImage(PathBuf);
impl PrivateImage {
    fn new(folder: &Path, bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        fs::create_dir_all(folder).unwrap();
        loop {
            let path = folder.join(format!(
                "native-replay-{}-{}.img",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut f) => {
                    f.write_all(bytes).unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("private native image: {e}"),
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
#[ignore = "requires a private out/sd/board_native.log and reference tools"]
fn the_private_board_block_reproduces_in_the_emulator() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/sd");
    let log = match fs::read_to_string(folder.join("board_native.log")) {
        Ok(log) => log,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipping native SD replay: private log missing");
            return;
        }
        Err(e) => panic!("private native SD log: {e}"),
    };
    let capture = board_read(&log).expect("complete indented B0 and B0RAW with CRC16 ok");
    let image = PrivateImage::new(&folder, &capture.block);
    let mut c = command();
    c.env("ECP5_SD_IMAGE", &image.0);
    let (text, _) = run(c);
    let actual = text
        .lines()
        .find(|l| l.starts_with("SDN B0 "))
        .expect("complete B0 CRC report");
    // Neither raw bytes nor their non-private-hash CRC belong in a failure dump.
    assert!(
        actual == capture.crc_line && actual.ends_with(" CRC16 ok"),
        "private B0 CRC report differs"
    );
}
#[test]
fn private_parser_requires_complete_lines_and_ignores_identity() {
    let block = [0x96u8; 512];
    let log = format!(
        "load 1\n  SDN CID not-copied\n  SDN B0 12345678 CRC16 ok\n  SDN B0RAW {}\n",
        hex(&block)
    );
    let c = board_read(&log).unwrap();
    assert_eq!(c.block, block);
    assert_eq!(c.crc_line, "SDN B0 12345678 CRC16 ok");
    assert!(board_read(log.trim_end()).is_none());
    assert_eq!(crc32(b"123456789"), 0xcbf43926);
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
fn reference_can_only_send_read_only_initialization_and_cmd17() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/sd_native_read");
    let mut files: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["reader.v", "top.v", "uart_tx.v"]);
    let reader = normalized(&fs::read_to_string(root.join("reader.v")).unwrap());
    let indices: Vec<_> = reader
        .split("command_index=6'd")
        .skip(1)
        .map(|s| s.split(';').next().unwrap().parse::<u8>().unwrap())
        .collect();
    assert_eq!(indices, [0, 8, 55, 41, 2, 3, 9, 7, 55, 6, 16, 17, 0]);
    for i in 0..4 {
        assert!(reader.contains(&format!(
            "BBd{i}(.I(1'b0),.T(1'b1),.O(dat[{i}]),.B(sd_d[{i}]));"
        )));
    }
    for (name, digest) in SEALS {
        let text = normalized(&fs::read_to_string(root.join(name)).unwrap());
        assert_eq!(
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            *digest,
            "review the whole reference before changing its read-only seal: {name}"
        );
    }
}
const SEALS: &[(&str, &str)] = &[
    (
        "reader.v",
        "abb6c36cf51f74f54ce2fdd46fb96d627d83259c6952b6d20483bb17e32ad4ee",
    ),
    (
        "top.v",
        "c4dc9d1caa9012a82454b3ca930ae1440fa0887e67cb61eaca7982d74742f621",
    ),
    (
        "uart_tx.v",
        "af9ccc8e35144a4615acbf203b20b2723722f48b49aafa018053e9fdeb3965a3",
    ),
    (
        "top.lpf",
        // P4 header (ffdaaa8) only; constraints identical to reviewed 069d7f2.
        "b280cd5331130f9d92544c88195d156ceed7a6a7dd32a24978c551d208c1e6ec",
    ),
];
