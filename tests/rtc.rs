//! Routed read-only RTC master, open-drain answers and private board replay.
mod common;
use ecp5_emu::{
    board,
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::{
        rtc::{Rtc, CAPACITY},
        Attached,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn run(mut rtc: Rtc, passes: usize) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = common::reference::build("rtc_read");
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
        board::direction(&device, board::RTC_SDA).unwrap(),
        Some(board::Direction::Bidirectional)
    );
    // SCL is open-drain too, but has no input consumer and may route as output.
    assert!(board::direction(&device, board::RTC_SCL)
        .unwrap()
        .unwrap()
        .drives());
    let used = board::used_balls(&device).unwrap().into_iter().collect();
    assert!(Attached::new(&used).view().rtc.is_some());
    let pin = |b: &str| Reference::Pin(b.into());
    let refs = [
        pin(board::RTC_SCL),
        pin(board::RTC_SDA),
        pin(board::UART_TX),
    ];
    let compiled = compile::compile(
        device,
        Request {
            package: board::PACKAGE.into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![
                (pin(board::CLOCK), Logic::Zero),
                (pin(board::RTC_SDA), Logic::Z),
            ],
            probes: refs.to_vec(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap();
    let probes: Vec<_> = refs.iter().map(|r| compiled.probes[&r.label()]).collect();
    let drivers = [compiled.drivers[&pin(board::RTC_SDA).label()]];
    let mut fast = FastSim::new(&compiled.kernel).unwrap();
    fast.clock(
        compiled.drivers[&pin(board::CLOCK).label()],
        Time::from_ps(40_000),
    )
    .unwrap();
    fast.trace(&probes[..2]).unwrap();
    let mut uart = Receiver::new(board::BAUD);
    let mut text = String::new();
    let mut last = None;
    let mut lines = 0;
    let mut edges = 0;
    let start = Instant::now();
    while edges < (passes as u64 * 25_000_000 + 2_000_000) * 2 && lines < passes * 3 {
        edges += fast
            .run_edges_with(1024, &probes[2..], &drivers, |sample, trace, changes| {
                for (&ps, row) in trace.times_ps.iter().zip(trace.rows.chunks_exact(2)) {
                    rtc.feed(
                        ps,
                        ecp5_emu::peripherals::rtc::Pins {
                            scl: row[0],
                            sda: row[1],
                        },
                    );
                }
                if rtc.sda() != last {
                    last = rtc.sda();
                    changes.push((0, last));
                }
                if let Some((ps, row)) = sample {
                    if let Some(b) = uart.sample(ps / 1000, row[0] == 1) {
                        text.push(b as char);
                        if b == b'\n' {
                            lines += 1;
                        }
                    }
                }
                Ok(true)
            })
            .unwrap();
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "rtc_read: {} cycles in {elapsed:.3} s, {:.0} cycles/s",
        edges / 2,
        edges as f64 / 2.0 / elapsed
    );
    println!("{}", text.trim_end());
    assert_eq!(uart.framing_errors, 0);
    assert_eq!(lines, passes * 3, "UART timed out: {text:?}");
    assert_eq!(rtc.status().bytes_written, 0);
    assert_eq!(rtc.status().bytes_read, passes as u64 * 96);
    assert_eq!(
        rtc.status().nacks,
        passes as u64 * 2,
        "only final master NACKs"
    );
    text
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
fn known() -> Rtc {
    let mut image = Rtc::default().image();
    image[..7].copy_from_slice(&[0x56, 0x34, 0x12, 0x0d, 0x17, 0x11, 0x26]);
    image[7] = 0x80;
    image[8] = 0x35;
    for (i, b) in image[0x20..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37);
    }
    Rtc::new(image)
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn rtc_read_prints_a_known_image() {
    let rtc = known();
    let image = rtc.image();
    assert_eq!(
        run(rtc, 1),
        format!(
            "RTC t 12:34:56 2026-11-17 st 0\r\nRTC r {}\r\nRTC s {}\r\n",
            hex(&image[..32]),
            hex(&image[32..])
        )
    );
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn rtc_read_advances_once_per_emulated_second() {
    let text = run(Rtc::default(), 3);
    let times: Vec<_> = text.lines().filter(|s| s.starts_with("RTC t ")).collect();
    assert_eq!(
        times,
        [
            "RTC t 00:00:00 2026-01-01 st 1",
            "RTC t 00:00:01 2026-01-01 st 1",
            "RTC t 00:00:02 2026-01-01 st 1"
        ]
    );
    let registers: Vec<_> = text
        .lines()
        .filter_map(|s| s.strip_prefix("RTC r "))
        .collect();
    assert!(registers[0].starts_with("80000025"));
    assert!(registers[1].starts_with("81000025"));
    assert!(registers[2].starts_with("82000025"));
}

fn decode(hex: &str, bytes: &mut [u8]) -> bool {
    if hex.len() != bytes.len() * 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    for (b, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        *b = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    true
}
fn board_read(log: &str) -> Option<([u8; CAPACITY], Vec<String>)> {
    let mut current = None;
    for raw in log.lines() {
        let Some(line) = raw.trim_end_matches('\r').strip_prefix("  ") else {
            current = None;
            continue;
        };
        if let Some(r) = line.strip_prefix("RTC r ") {
            let mut image = [0; CAPACITY];
            current = decode(r, &mut image[..32]).then(|| (image, vec![line.to_string()]));
        } else if let Some(s) = line.strip_prefix("RTC s ") {
            if let Some((mut image, mut lines)) = current.take() {
                if decode(s, &mut image[32..]) {
                    lines.push(line.to_string());
                    return Some((image, lines));
                }
            }
        } else {
            current = None;
        }
    }
    None
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_board_read_reproduces_in_the_emulator() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/rtc/board_rtc.log");
    let log = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipping the_board_read_reproduces_in_the_emulator: {} missing; put run.py output there",path.display());
            return;
        }
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let (image, expected) = board_read(&log).expect("first complete indented RTC r + RTC s pair");
    // Keep ST exactly as read, including stopped clocks. The first sample is
    // less than one second after the image's emulated epoch.
    let rtc = Rtc::new(image);
    assert_eq!(rtc.status().st, image[0] & 0x80 != 0);
    let text = run(rtc, 1);
    let actual: Vec<_> = text
        .lines()
        .filter(|s| s.starts_with("RTC r ") || s.starts_with("RTC s "))
        .collect();
    assert_eq!(actual, expected);
}
#[test]
fn private_board_parser_requires_complete_indented_pairs() {
    let image = known().image();
    let r = format!("RTC r {}", hex(&image[..32]));
    let s = format!("RTC s {}", hex(&image[32..]));
    assert!(board_read(&format!("{r}\n{s}\n")).is_none());
    assert!(board_read(&format!("  {r}\nload 2\n  {s}\n")).is_none());
    assert!(board_read(&format!("  {r}\n  RTC s FF\n")).is_none());
    let log = format!("load 1\n  RTC t anything\n  {r}\r\n  {s}\n");
    assert_eq!(board_read(&log).unwrap(), (image, vec![r, s]));
}

// This is deliberately fail-closed: the reviewed FSM, wiring wrapper and
// UART are sealed after removing line comments and whitespace. Structural
// checks explain the permitted byte path; the seals prevent an extra always
// block, pin driver or state transition from bypassing that path unnoticed.
fn normalized(source: &str) -> String {
    source
        .lines()
        .flat_map(|l| {
            l.split("//")
                .next()
                .unwrap()
                .chars()
                .filter(|c| !c.is_whitespace())
        })
        .collect()
}
fn audit_reader(source: &str) -> bool {
    use sha2::{Digest, Sha256};
    let code = normalized(source);
    let sends: Vec<_> = code
        .split("tx<=")
        .skip(1)
        .map(|s| s.split(';').next().unwrap())
        .collect();
    sends == ["8'hde", "{tx[6:0],1'b1}", "pointer", "8'hdf"]
        && code.contains("0:begintx<=pointer;stage<=1;state<=SEND_LOW;end")
        && code.contains("1:beginstage<=2;state<=RESTART_RELEASE;end")
        && code.contains("2:beginrx<=0;state<=READ_LOW;end")
        && code.contains("RESTART:beginsda_low<=1;tx<=8'hdf;state<=SEND_LOW;end")
        && code.contains("assignrtc_sda=sda_low?1'b0:1'bz;")
        && code.contains("assignrtc_scl=scl_low?1'b0:1'bz;")
        && hex(&Sha256::digest(code.as_bytes())).to_lowercase()
            == "bd9c12d0abedb94f2ed60b2ee34e66b45a94a4a6c7e0b9ca3c2b7fcadf553cc1"
}
fn audit_edid_reader(source: &str) -> bool {
    use sha2::{Digest, Sha256};
    let code = normalized(source);
    let sends: Vec<_> = code
        .split("tx<=")
        .skip(1)
        .map(|s| s.split(';').next().unwrap())
        .collect();
    sends == ["8'ha0", "{tx[6:0],1'b1}", "8'h00", "8'ha1"]
        && code.contains("remaining<=128;")
        && code.contains("0:begintx<=8'h00;stage<=1;state<=SEND_LOW;end")
        && code.contains("1:beginstage<=2;state<=RESTART_RELEASE;end")
        && code.contains("2:beginrx<=0;state<=READ_LOW;end")
        && code.contains("assignddc_sda=sda_low?1'b0:1'bz;")
        && code.contains("assignddc_scl=scl_low?1'b0:1'bz;")
        && hex(&Sha256::digest(code.as_bytes())).to_lowercase()
            == "7e979c086326fbb2ca63657328948e16c558a4ecde741f95c1225178f48aa430"
}
#[test]
fn reference_verilog_can_only_send_address_pointer_address_then_read() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/rtc_read");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["rtc_reader.v", "top.v", "uart_tx.v"]);
    assert!(
        audit_reader(&std::fs::read_to_string(root.join("rtc_reader.v")).unwrap()),
        "review the complete transmit FSM before changing its safety seal"
    );
    for (name, digest) in [
        (
            "top.v",
            "b7305d78647c922a8900a5951cac62aa8f24879e7196579003e3b48abceff401",
        ),
        (
            "uart_tx.v",
            "af9ccc8e35144a4615acbf203b20b2723722f48b49aafa018053e9fdeb3965a3",
        ),
    ] {
        let source = normalized(&std::fs::read_to_string(root.join(name)).unwrap());
        assert_eq!(
            hex(&Sha256::digest(source.as_bytes())).to_lowercase(),
            digest,
            "unreviewed reference wiring in {name}"
        );
    }
    let lpf = std::fs::read_to_string(root.join("top.lpf")).unwrap();
    for (port, ball) in [("rtc_scl", "E12"), ("rtc_sda", "B19")] {
        assert!(lpf.contains(&format!("LOCATE COMP \"{port}\" SITE \"{ball}\";")));
        assert!(lpf.contains(&format!(
            "IOBUF PORT \"{port}\" PULLMODE=UP IO_TYPE=LVCMOS33;"
        )));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/edid_read");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "dvi_source.v",
            "edid_reader.v",
            "pll.v",
            "tmds.v",
            "top.v",
            "uart_tx.v",
            "video_out.v"
        ]
    );
    assert!(
        audit_edid_reader(&std::fs::read_to_string(root.join("edid_reader.v")).unwrap()),
        "review the complete EDID FSM before changing its safety seal"
    );
    // Seal every driver and its pin mapping, including the copied DVI path.
    for (name, digest) in [
        (
            "dvi_source.v",
            "f7a982faf01e097b2bf726680b51fdcd3e41cc2cb417f23b9676d9ddd78506e9",
        ),
        (
            "pll.v",
            "bbc7acde8fe6096e46fbf61110c14a79ab7a49d9f71921bce392b5673d880d25",
        ),
        (
            "tmds.v",
            "42edf2e0177ef2481845f563f2dd0ec5bda2aedc9601e0127e6189221fbfe7cc",
        ),
        (
            "top.v",
            "03080d15a33d08525dd10ed4e7c45dc5dc8be4002884411554786b120fffbfb3",
        ),
        (
            "uart_tx.v",
            "af9ccc8e35144a4615acbf203b20b2723722f48b49aafa018053e9fdeb3965a3",
        ),
        (
            "video_out.v",
            "49a4f69ffffee6c4ee448fb2fa1b87a632a0e1e74591261d662561b79d021a35",
        ),
        (
            "top.lpf",
            // ffdaaa8: comments/blank lines only; constraints identical to reviewed 71ea501.
            "942e58575b47d0c29da97bde94a692ed92c8f32c5f1a7e362d071a79ad59ddb6",
        ),
    ] {
        let code = normalized(&std::fs::read_to_string(root.join(name)).unwrap());
        assert_eq!(
            hex(&Sha256::digest(code.as_bytes())).to_lowercase(),
            digest,
            "unreviewed EDID reference wiring in {name}"
        );
    }
}
#[test]
fn read_only_audit_rejects_new_bytes_states_and_drivers() {
    let source = include_str!("../examples/ulx3s/rtc_read/rtc_reader.v");
    assert!(audit_reader(source));
    assert!(audit_reader(&format!(
        "// an explanatory comment\n{source}\n"
    )));
    for change in [
        source.replace("8'hde", "8'ha0"),
        source.replace("tx <= pointer", "tx <= 8'h00"),
        source.replace("state <= RESTART_RELEASE", "state <= SEND_LOW"),
        source.replace(
            "2: begin rx <= 0; state <= READ_LOW; end",
            "2: begin tx <= pointer; state <= SEND_LOW; end",
        ),
        format!("{source}\nassign rtc_sda = 1'b0;"),
    ] {
        assert!(!audit_reader(&change));
    }
}

#[test]
fn edid_read_only_audit_rejects_payloads_extra_states_and_drivers() {
    let source = include_str!("../examples/ulx3s/edid_read/edid_reader.v");
    assert!(audit_edid_reader(source));
    assert!(audit_edid_reader(&format!("// comment\n{source}")));
    for changed in [
        source.replace("8'ha0", "8'hde"),
        source.replace("tx <= 8'h00", "tx <= data"),
        source.replace("state <= RESTART_RELEASE", "state <= SEND_LOW"),
        source.replace(
            "2: begin rx <= 0; state <= READ_LOW; end",
            "2: begin tx <= data; state <= SEND_LOW; end",
        ),
        format!("{source}\nassign ddc_sda = 1'b0;"),
    ] {
        assert!(!audit_edid_reader(&changed));
    }
}
