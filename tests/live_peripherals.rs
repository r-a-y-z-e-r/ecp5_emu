//! The board's peripherals in the live runner (src/live.rs), as the window
//! sees them in `Shared::peripherals`: the HDMI monitor of `hdmi_moving` and
//! the OLED of `oled_pattern`, decoded from the pads while the design runs at
//! full speed. Each design prints the CRC of every frame it sends on its
//! UART, and the frames the runner shows must carry those CRCs, also after
//! going back and running on.
mod common;
use ecp5_emu::live::{self, Command, Design, Shared};
use std::{
    sync::{mpsc::Sender, Arc, Mutex},
    time::{Duration, Instant},
};

fn start(design: &str) -> (Arc<Mutex<Shared>>, Sender<Command>) {
    let out = common::reference::build(design);
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn(
        Design {
            config: out.join("top.config"),
            names: Some(out.join("top_routed.json")),
            run: false,
        },
        Arc::clone(&shared),
        || {},
    );
    (shared, commands)
}

fn wait(shared: &Mutex<Shared>, seconds: u64, what: &str, done: impl Fn(&Shared) -> bool) {
    let start = Instant::now();
    loop {
        {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
            if done(&s) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(seconds),
                "no {what} after {seconds} s: stage {:?}, cycle {}, peripherals {:?}",
                s.stage,
                s.cycles,
                s.peripherals
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// UART records need LF; a trailing CR or valid-looking prefix is still in flight.
fn complete_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive('\n')
        .filter_map(|line| line.strip_suffix('\n'))
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

/// The complete `<tag><number> C<crc>` UART lines, as (number, crc).
fn reports(text: &[u8], tag: char) -> Vec<(u64, u32)> {
    let text = String::from_utf8_lossy(text);
    complete_lines(&text)
        .filter_map(|l| {
            let (n, c) = l.trim().strip_prefix(tag)?.split_once(" C")?;
            Some((
                u64::from_str_radix(n, 16).ok()?,
                u32::from_str_radix(c, 16).ok()?,
            ))
        })
        .collect()
}

#[test]
fn reports_ignore_every_unterminated_uart_suffix() {
    assert_eq!(
        complete_lines("first\r\nsecond\npartial\r").collect::<Vec<_>>(),
        ["first", "second"]
    );
    for tag in ['O', 'F'] {
        let first = format!("{tag}0003 C12345678\r\n");
        let last = format!("{tag}0004 C2FB7D901\r\n");
        // Covers short CRCs (C2FB7D), all eight digits without LF, and CR
        // received before LF. None is a completed report yet.
        for cut in 0..last.len() {
            let text = format!("{first}{}", &last[..cut]);
            assert_eq!(reports(text.as_bytes(), tag), [(3, 0x1234_5678)]);
            assert!(reports(&last.as_bytes()[..cut], tag).is_empty());
        }
        assert_eq!(
            reports(format!("{first}{last}").as_bytes(), tag),
            [(3, 0x1234_5678), (4, 0x2fb7_d901)]
        );
        assert_eq!(
            reports(format!("{tag}0004 C2FB7D901\n").as_bytes(), tag),
            [(4, 0x2fb7_d901)]
        );
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_monitor_shows_the_frames_hdmi_moving_reports() {
    let (shared, commands) = start("hdmi_moving");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    let started = Instant::now();
    // A frame is complete when the next one starts, as its UART line does:
    // the line comes 1.5 ms later. The receiver numbers the frames it got;
    // from the start that is the design's number, after going back not.
    let reported = |s: &Shared, at_least: u64, by_number: bool| {
        s.peripherals.video.as_ref().is_some_and(|v| {
            v.frames_decoded >= at_least
                && v.last_frame.as_ref().is_some_and(|f| {
                    reports(&s.text, 'F')
                        .iter()
                        .any(|r| r.1 == f.crc32 && (!by_number || r.0 == f.number))
                })
        })
    };
    wait(
        &shared,
        120,
        "three decoded frames and their UART lines",
        |s| reported(s, 3, true),
    );
    let seconds = started.elapsed().as_secs_f64();
    let check = |s: &Shared, by_number: bool| {
        let video = s.peripherals.video.as_ref().unwrap();
        let frame = video.last_frame.as_ref().unwrap();
        let printed = reports(&s.text, 'F');
        assert!(
            printed
                .iter()
                .any(|r| r.1 == frame.crc32 && (!by_number || r.0 == frame.number)),
            "frame {} CRC {:08X} not among the UART lines {printed:X?}",
            frame.number,
            frame.crc32
        );
        assert_eq!((frame.width, frame.height), (640, 480));
        assert_eq!(
            s.peripherals.hdmi_mode().as_deref(),
            Some("640x480 59.52 Hz")
        );
        (frame.number, frame.crc32)
    };
    let shown = {
        let s = shared.lock().unwrap();
        println!(
            "hdmi_moving live: {} frames in {seconds:.1} s at {:.2} MHz of board clock",
            s.peripherals.video.as_ref().unwrap().frames_decoded,
            s.mhz
        );
        check(&s, true)
    };
    // Back a little over a frame and on again: the receiver locks again,
    // and the frames after are still the design's.
    commands.send(Command::Pause).unwrap();
    commands.send(Command::Back(500_000)).unwrap();
    wait(&shared, 60, "pause after going back", |s| {
        !s.running && s.stage == "Paused"
    });
    let decoded = shared
        .lock()
        .unwrap()
        .peripherals
        .video
        .as_ref()
        .unwrap()
        .frames_decoded;
    commands.send(Command::Run).unwrap();
    wait(&shared, 120, "two more frames and their UART lines", |s| {
        reported(s, decoded + 2, false)
    });
    let again = check(&shared.lock().unwrap(), false);
    println!(
        "hdmi_moving live: frame {shown:X?} before going back, {again:X?} after (number, CRC)"
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_oled_shows_the_frames_oled_pattern_reports() {
    let (shared, commands) = start("oled_pattern");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    wait(&shared, 120, "four OLED frames", |s| {
        s.peripherals.oled.as_ref().is_some_and(|o| o.frames >= 4)
    });
    commands.send(Command::Pause).unwrap();
    wait(&shared, 60, "pause", |s| !s.running);
    let s = shared.lock().unwrap();
    let oled = s.peripherals.oled.as_ref().unwrap();
    assert!(oled.display_on, "{oled:?}");
    let printed = reports(&s.text, 'O');
    // The UART line of frame n comes after the frame: the runner may have
    // the frame before the line.
    let n = oled.frames - 1;
    let line = printed.iter().find(|(number, _)| *number == n);
    if let Some(&(_, crc)) = line {
        assert_eq!(crc, oled.crc32, "OLED frame {n}");
    } else {
        let &(last, _) = printed.last().unwrap();
        assert_eq!(
            last + 1,
            n,
            "UART lines {printed:X?}, OLED frames {}",
            oled.frames
        );
    }
    let (w, h, rgba, _) = s.peripherals.oled_screen().unwrap();
    assert_eq!((w, h, rgba.len()), (96, 64, 96 * 64 * 4));
    assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]));
    println!(
        "oled_pattern live: {} frames, CRC {:08X}, {} UART lines",
        oled.frames,
        oled.crc32,
        printed.len()
    );
}

/// `sd_read` against the window's SD card (`SdCard::demo`, or the image
/// `ECP5_SD_IMAGE` names): the card
/// answers on MISO while the design runs at full speed, and the design
/// prints the card's OCR and block 0's first and last bytes. Going back and
/// running on, the card answers the same again.
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_sd_card_answers_sd_read() {
    // The card the runner uses: ECP5_SD_IMAGE, else the demo card.
    use ecp5_emu::peripherals::sdcard::SdCard;
    let card = match std::env::var_os("ECP5_SD_IMAGE") {
        Some(path) => SdCard::new(std::fs::read(path).unwrap()),
        None => SdCard::demo(),
    }
    .image();
    let first: String = card[..16].iter().map(|b| format!("{b:02X}")).collect();
    let expected = format!(
        "SD OCR C0FF8000 B0 {first} {:02X}{:02X}",
        card[510], card[511]
    );
    let (shared, commands) = start("sd_read");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    let started = Instant::now();
    // Whole lines only: the last one may still be arriving.
    let lines = |s: &Shared| {
        String::from_utf8_lossy(&s.text)
            .split_inclusive('\n')
            .filter(|l| l.ends_with('\n') && l.starts_with("SD "))
            .map(|l| l.trim_end().to_string())
            .collect::<Vec<_>>()
    };
    wait(&shared, 120, "the SD line", |s| {
        lines(s)
            .iter()
            .any(|l| l.trim_end() == expected || l.contains("ERR"))
    });
    {
        let s = shared.lock().unwrap();
        assert_eq!(lines(&s)[0].trim_end(), expected);
        let sd = s.peripherals.sd.as_ref().unwrap();
        assert_eq!(
            (sd.blocks_read, sd.crc_errors, sd.illegal_commands),
            (1, 0, 0),
            "{sd:?}"
        );
        println!(
            "sd_read live: {expected:?} after {:.1} s at {:.2} MHz",
            started.elapsed().as_secs_f64(),
            s.mhz
        );
    }
    // The design reads every second: back to before the first read and on
    // past the next.
    commands.send(Command::Pause).unwrap();
    wait(&shared, 60, "pause", |s| !s.running);
    let cycle = shared.lock().unwrap().cycles;
    commands.send(Command::GoTo(cycle / 2)).unwrap();
    wait(&shared, 60, "pause after going back", |s| {
        !s.running && s.stage == "Paused"
    });
    commands.send(Command::Run).unwrap();
    wait(&shared, 180, "the SD line again", |s| {
        s.cycles > cycle + 25_000_000
            && lines(s).iter().filter(|l| l.starts_with("SD OCR")).count() >= 1
    });
    let s = shared.lock().unwrap();
    for line in lines(&s) {
        assert_eq!(line.trim_end(), expected, "after going back");
    }
    println!(
        "sd_read live: after going back to cycle {}: {:?}",
        cycle / 2,
        lines(&s)
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_native_sd_card_answers_and_replays_its_pads() {
    use ecp5_emu::peripherals::sdcard::SdCard;
    let card = match std::env::var_os("ECP5_SD_IMAGE") {
        Some(path) => SdCard::new(std::fs::read(path).unwrap()),
        None => SdCard::demo(),
    };
    let block = &card.image()[..512];
    let mut crc = !0u32;
    for &byte in block {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    let expected = format!("SDN B0 {:08X} CRC16 ok", !crc);
    let (shared, commands) = start("sd_native_read");
    wait(&shared, 300, "native SD design ready", |s| {
        s.ready.is_some()
    });
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Step(4_000_000)).unwrap();
    wait(&shared, 180, "native SD UART", |s| {
        !s.running && s.cycles == 4_000_000
    });
    let console = |s: &Shared| {
        s.console
            .iter()
            .flatten()
            .chain(&s.text)
            .copied()
            .collect::<Vec<_>>()
    };
    let (sd, text) = {
        let s = shared.lock().unwrap();
        let sd = s.peripherals.sd.as_ref().expect("native SD attached");
        assert_eq!(sd.mode, "sd4");
        assert_eq!(sd.rca, 1);
        assert_eq!(
            (sd.blocks_read, sd.blocks_written, sd.crc_errors),
            (1, 0, 0)
        );
        assert_eq!(sd.illegal_commands, 0);
        let text = console(&s);
        assert!(
            complete_lines(&String::from_utf8_lossy(&text)).any(|l| l == expected),
            "native block CRC report differs (private data withheld)"
        );
        (sd.clone(), text)
    };
    // Before ACMD41: replay must restore both the bus width and all pad drivers.
    commands.send(Command::GoTo(20_000)).unwrap();
    wait(&shared, 180, "native SD rewind", |s| {
        !s.running && s.cycles == 20_000
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .peripherals
            .sd
            .as_ref()
            .unwrap()
            .bus_width,
        1
    );
    commands.send(Command::Step(3_980_000)).unwrap();
    wait(&shared, 180, "native SD replay", |s| {
        !s.running && s.cycles == 4_000_000
    });
    let s = shared.lock().unwrap();
    assert_eq!(s.peripherals.sd.as_ref().unwrap(), &sd);
    assert!(
        console(&s) == text,
        "native SD replay differs (private data withheld)"
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_sdram_runs_picorv32_sdram() {
    let (shared, commands) = start("picorv32_sdram");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    let started = Instant::now();
    let complete_line = |s: &Shared, prefix: &str| {
        String::from_utf8_lossy(&s.text)
            .split_inclusive('\n')
            .any(|l| l.ends_with('\n') && l.starts_with(prefix))
    };
    wait(&shared, 300, "pass 0 memtest line", |s| {
        complete_line(s, "pass 0: memtest 256 KB:")
    });
    let pass0_mhz = shared.lock().unwrap().mhz;
    commands.send(Command::Pause).unwrap();
    wait(&shared, 60, "pause", |s| !s.running);
    let (pass0_cycles, pass0_memtest, paused_cycle, paused_status, paused_text, paused_leds) = {
        let s = shared.lock().unwrap();
        let text = String::from_utf8_lossy(&s.text);
        let lines: Vec<_> = complete_lines(&text).collect();
        let sdram = s.peripherals.sdram.as_ref().expect("SDRAM attached");
        assert!(sdram.initialised, "SDRAM not initialised: {sdram:?}");
        assert_eq!(sdram.timing_violations, 0, "{:?}", sdram.messages);
        assert_eq!(sdram.commands_before_init, 0, "{:?}", sdram.messages);
        println!(
            "picorv32_sdram live (pass 0): {} cycles in {:.1} s at {:.2} MHz ({:.0} cycles/s), lines {:?}",
            s.cycles,
            started.elapsed().as_secs_f64(),
            pass0_mhz,
            pass0_mhz * 1e6,
            lines
        );
        let c0: u64 = lines
            .iter()
            .find(|l| l.starts_with("pass 0: primes below 2000: "))
            .expect("pass 0 primes line")
            .strip_prefix("pass 0: primes below 2000: 303, ")
            .and_then(|s| s.strip_suffix(" cycles"))
            .and_then(|s| s.parse().ok())
            .expect("pass 0 cycle count");
        let memtest = lines
            .iter()
            .find(|l| l.starts_with("pass 0: memtest 256 KB: "))
            .expect("pass 0 memtest line")
            .strip_prefix("pass 0: ")
            .unwrap()
            .to_string();
        assert_eq!(memtest, "memtest 256 KB: 0 errors");
        assert_eq!(c0, 855_498);
        (
            c0,
            memtest,
            s.cycles,
            format!("{sdram:?}"),
            s.text.clone(),
            s.leds,
        )
    };
    commands.send(Command::Back(100_000)).unwrap();
    wait(&shared, 60, "pause after going back", |s| {
        !s.running && s.stage == "Paused" && s.cycles == paused_cycle - 100_000
    });
    // Replay to the exact paused cycle: command and beat counts, timing,
    // UART text and outputs must agree, including an answer at the boundary.
    commands.send(Command::Step(100_000)).unwrap();
    wait(&shared, 60, "replay to the paused cycle", |s| {
        !s.running && s.stage == "Paused" && s.cycles == paused_cycle
    });
    {
        let s = shared.lock().unwrap();
        assert_eq!(
            format!("{:?}", s.peripherals.sdram.as_ref().unwrap()),
            paused_status
        );
        assert_eq!(s.console.as_ref().expect("replayed console"), &paused_text);
        assert_eq!(s.leds, paused_leds);
    }
    commands.send(Command::Run).unwrap();
    wait(&shared, 300, "pass 1 memtest line", |s| {
        complete_line(s, "pass 1: memtest 256 KB:")
    });
    let s = shared.lock().unwrap();
    let sdram = s.peripherals.sdram.as_ref().unwrap();
    assert!(sdram.initialised);
    assert_eq!(sdram.timing_violations, 0, "{:?}", sdram.messages);
    assert_eq!(sdram.commands_before_init, 0, "{:?}", sdram.messages);
    let text = String::from_utf8_lossy(&s.text);
    let lines: Vec<_> = complete_lines(&text).collect();
    let c1: u64 = lines
        .iter()
        .find(|l| l.starts_with("pass 1: primes below 2000: "))
        .expect("pass 1 primes line")
        .strip_prefix("pass 1: primes below 2000: 303, ")
        .and_then(|s| s.strip_suffix(" cycles"))
        .and_then(|s| s.parse().ok())
        .expect("pass 1 cycle count");
    let memtest1 = lines
        .iter()
        .find(|l| l.starts_with("pass 1: memtest 256 KB: "))
        .expect("pass 1 memtest line")
        .strip_prefix("pass 1: ")
        .unwrap();
    assert!(
        pass0_cycles.abs_diff(c1) < 2000,
        "pass 0 ({pass0_cycles}) and pass 1 ({c1}) cycle counts diverged"
    );
    assert_eq!(c1, 854_997);
    assert_eq!(memtest1, pass0_memtest);
    println!(
        "picorv32_sdram live (pass 1): {:.2} MHz ({:.0} cycles/s), lines {:?}",
        s.mhz,
        s.mhz * 1e6,
        lines
    );
}

/// XIP flash answers from the CCLK pad (U3), including after rewind.
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_flash_runs_picorv32_flash_and_rewinds() {
    let (shared, commands) = start("picorv32_flash");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    let complete = |s: &Shared| {
        String::from_utf8_lossy(&s.text)
            .split_inclusive('\n')
            .any(|l| l == "pass 1: primes below 2000: 303, 5849188 cycles\r\n")
    };
    wait(&shared, 300, "two XIP prime passes", complete);
    commands.send(Command::Pause).unwrap();
    wait(&shared, 60, "pause", |s| !s.running);
    let (cycle, text, flash) = {
        let s = shared.lock().unwrap();
        assert!(s.text.starts_with(b"PicoRV32 flash ID 9D6018\r\n"));
        assert!(String::from_utf8_lossy(&s.text)
            .contains("pass 0: primes below 2000: 303, 5849188 cycles\r\n"));
        let flash = s.peripherals.flash.as_ref().unwrap();
        assert!(flash.bytes_read > 1000);
        assert_eq!(flash.ignored_writes, 0);
        println!(
            "picorv32_flash live: {} at {:.2} MHz",
            String::from_utf8_lossy(&s.text),
            s.mhz
        );
        (s.cycles, s.text.clone(), flash.clone())
    };
    // Back before boot, then replay the same SPI exchanges and firmware.
    commands.send(Command::GoTo(0)).unwrap();
    wait(&shared, 60, "rewind to zero", |s| {
        !s.running && s.cycles < cycle / 2
    });
    assert!(
        shared
            .lock()
            .unwrap()
            .peripherals
            .flash
            .as_ref()
            .unwrap()
            .bytes_read
            < flash.bytes_read
    );
    commands.send(Command::Run).unwrap();
    wait(&shared, 300, "the XIP passes again", |s| {
        complete(s) && s.cycles >= cycle
    });
    commands.send(Command::Pause).unwrap();
    wait(&shared, 60, "pause after replay", |s| !s.running);
    let s = shared.lock().unwrap();
    assert!(s.text.starts_with(&text));
    assert_eq!(s.peripherals.flash.as_ref().unwrap().ignored_writes, 0);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_adc_records_voltage_inputs_and_replays_them() {
    let (shared, commands) = start("adc_read");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Step(250_000)).unwrap();
    wait(&shared, 120, "first ADC report", |s| {
        !s.running && s.cycles == 250_000
    });
    let initial = {
        let s = shared.lock().unwrap();
        let adc = s.peripherals.adc.as_ref().expect("ADC attached");
        assert_eq!(
            adc.codes,
            [
                Some(4095),
                Some(0),
                Some(4095),
                Some(0),
                Some(4095),
                Some(0),
                Some(4095),
                Some(0)
            ]
        );
        assert!(adc.driven.iter().all(|d| !*d));
        assert_eq!(
            s.text.as_slice(),
            b"ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000\r\n"
        );
        adc.clone()
    };
    // Many moves while paused must not advance any board-clock edge.
    for step in 0..40 {
        commands
            .send(Command::AdcVolts(0, f64::from(step) / 40.0))
            .unwrap();
    }
    commands.send(Command::AdcVolts(0, 1.65)).unwrap();
    wait(&shared, 60, "paused slider input", |s| {
        s.peripherals
            .adc
            .as_ref()
            .is_some_and(|a| a.volts[0] == 1.65)
    });
    assert_eq!(shared.lock().unwrap().cycles, 250_000);
    commands.send(Command::Step(5_000_000)).unwrap();
    wait(&shared, 180, "changed ADC report", |s| {
        !s.running && s.cycles == 5_250_000
    });
    let (status, console, leds) = {
        let s = shared.lock().unwrap();
        let adc = s.peripherals.adc.as_ref().unwrap();
        assert_eq!(adc.codes[0], Some(2048));
        let text = &s.text;
        assert!(String::from_utf8_lossy(text)
            .contains("ADC 0800 0000 0FFF 0000 0FFF 0000 0FFF 0000\r\n"));
        println!("adc_read live: {}", String::from_utf8_lossy(text));
        (adc.clone(), text.clone(), s.leds)
    };
    // Rewind before the voltage command, then cross it with both seek and Step.
    commands.send(Command::GoTo(200_000)).unwrap();
    wait(&shared, 60, "rewind before slider", |s| {
        !s.running && s.cycles == 200_000
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .peripherals
            .adc
            .as_ref()
            .unwrap()
            .volts[0],
        initial.volts[0]
    );
    commands.send(Command::GoTo(250_100)).unwrap();
    wait(&shared, 60, "seek across slider", |s| {
        !s.running && s.cycles == 250_100
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .peripherals
            .adc
            .as_ref()
            .unwrap()
            .volts[0],
        1.65
    );
    commands.send(Command::GoTo(200_000)).unwrap();
    wait(&shared, 60, "second rewind", |s| {
        !s.running && s.cycles == 200_000
    });
    commands.send(Command::Step(5_050_001)).unwrap();
    wait(&shared, 180, "replay through slider", |s| {
        !s.running && s.cycles == 5_250_001
    });
    let s = shared.lock().unwrap();
    assert_eq!(s.peripherals.adc.as_ref().unwrap(), &status);
    let replayed: Vec<_> = s.console.iter().flatten().chain(&s.text).copied().collect();
    assert_eq!(replayed, console);
    assert_eq!(s.leds, leds);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_rtc_answers_and_rewinds_its_clock_and_bus() {
    use ecp5_emu::peripherals::rtc::Rtc;
    let initial = Rtc::from_env();
    let image = initial.image();
    let registers: String = image[..32].iter().map(|b| format!("{b:02X}")).collect();
    let sram: String = image[32..].iter().map(|b| format!("{b:02X}")).collect();
    let expected = format!("RTC r {registers}\r\nRTC s {sram}\r\n");
    let (shared, commands) = start("rtc_read");
    wait(&shared, 300, "design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Step(26_000_000)).unwrap();
    wait(&shared, 180, "two RTC reports", |s| {
        !s.running && s.cycles == 26_000_000
    });
    let (status, console) = {
        let s = shared.lock().unwrap();
        let rtc = s.peripherals.rtc.as_ref().expect("RTC attached");
        assert_eq!(rtc.bytes_written, 0);
        assert_eq!(rtc.bytes_read, 192);
        assert_eq!(rtc.nacks, 4);
        assert!(String::from_utf8_lossy(&s.text).contains(&expected));
        if image == Rtc::default().image() {
            assert_eq!(rtc.time, [2026, 1, 1, 0, 0, 1]);
            assert!(String::from_utf8_lossy(&s.text).contains("RTC t 00:00:01 2026-01-01 st 1\r\n"));
        }
        println!(
            "rtc_read live: {} at {:.2} MHz",
            String::from_utf8_lossy(&s.text),
            s.mhz
        );
        (rtc.clone(), s.text.clone())
    };
    // Seek into the second read, then across it: restore a checkpoint with
    // the fractional second and I2C output, not merely the displayed date.
    commands.send(Command::GoTo(25_010_000)).unwrap();
    wait(&shared, 90, "rewind into I2C transfer", |s| {
        !s.running && s.cycles == 25_010_000
    });
    assert!(
        shared
            .lock()
            .unwrap()
            .peripherals
            .rtc
            .as_ref()
            .unwrap()
            .active
    );
    commands.send(Command::Step(990_000)).unwrap();
    wait(&shared, 90, "RTC replay", |s| {
        !s.running && s.cycles == 26_000_000
    });
    {
        let s = shared.lock().unwrap();
        assert_eq!(s.peripherals.rtc.as_ref().unwrap(), &status);
        let replayed: Vec<_> = s.console.iter().flatten().chain(&s.text).copied().collect();
        assert_eq!(replayed, console);
    }
    commands.send(Command::GoTo(0)).unwrap();
    wait(&shared, 90, "rewind RTC before first read", |s| {
        !s.running && s.cycles < 1000
    });
    commands.send(Command::Step(26_000_000)).unwrap();
    wait(&shared, 180, "RTC replay from boot", |s| {
        !s.running && s.cycles == 26_000_000
    });
    let s = shared.lock().unwrap();
    assert_eq!(s.peripherals.rtc.as_ref().unwrap(), &status);
    let replayed: Vec<_> = s.console.iter().flatten().chain(&s.text).copied().collect();
    assert_eq!(replayed, console);
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_audio_scope_and_tone_replay_after_going_back() {
    let (shared, commands) = start("audio_tone");
    wait(&shared, 300, "audio design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Step(26_000_000)).unwrap();
    wait(&shared, 180, "first tone report", |s| {
        !s.running && s.cycles == 26_000_000
    });
    let console = |s: &Shared| {
        s.console
            .iter()
            .flatten()
            .chain(&s.text)
            .copied()
            .collect::<Vec<_>>()
    };
    let (audio, text) = {
        let s = shared.lock().unwrap();
        let a = s.peripherals.audio.as_ref().expect("audio attached");
        let text = console(&s);
        assert_eq!(text, b"AUDIO 25000 F1BFDEEF\r\n");
        assert!((a.channels[0].hz.unwrap() - 440.0).abs() < 1.0);
        assert!((a.channels[1].hz.unwrap() - 660.0).abs() < 1.0);
        assert!((a.channels[0].vpp.unwrap() - 1.7969378830276124).abs() < 1e-12);
        assert_eq!(a.channels[2].vpp, Some(0.0));
        assert_eq!(a.channels[2].code, 8);
        assert_eq!(a.scope.len(), ecp5_emu::peripherals::audio::SCOPE_SAMPLES);
        (a.clone(), text)
    };
    commands.send(Command::GoTo(24_500_000)).unwrap();
    wait(&shared, 180, "audio rewind before report", |s| {
        !s.running && s.cycles == 24_500_000
    });
    {
        let s = shared.lock().unwrap();
        let a = s.peripherals.audio.as_ref().unwrap();
        assert!(a.time_ps < audio.time_ps);
        assert!(a.transitions < audio.transitions);
        assert!(a.scope.iter().all(|p| p.ps <= a.time_ps));
        assert!(console(&s).is_empty());
    }
    commands.send(Command::Step(1_500_000)).unwrap();
    wait(&shared, 180, "audio replay", |s| {
        !s.running && s.cycles == 26_000_000
    });
    let s = shared.lock().unwrap();
    assert_eq!(s.peripherals.audio.as_ref().unwrap(), &audio);
    assert_eq!(console(&s), text);
    println!(
        "audio_tone live: L {:.6} Hz R {:.6} Hz; scope, statistics and UART replay exactly",
        audio.channels[0].hz.unwrap(),
        audio.channels[1].hz.unwrap()
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_live_monitor_edid_answers_and_rewinds_on_shared_ddc() {
    use ecp5_emu::peripherals::edid::Edid;
    let monitor = Edid::from_env();
    let present = monitor.status().present;
    let image = monitor.image();
    let expected = if present {
        let raw: String = image[..128].iter().map(|b| format!("{b:02X}")).collect();
        let sum = image[..128].iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
        format!("EDID {raw} sum {}\r\n", if sum == 0 { "ok" } else { "bad" })
    } else {
        "EDID nack\r\n".into()
    };
    let (shared, commands) = start("edid_read");
    wait(&shared, 300, "EDID design ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Step(2_000_000)).unwrap();
    wait(&shared, 180, "EDID report", |s| {
        !s.running && s.cycles == 2_000_000
    });
    let console = |s: &Shared| {
        s.console
            .iter()
            .flatten()
            .chain(&s.text)
            .copied()
            .collect::<Vec<_>>()
    };
    let (edid, rtc, text) = {
        let s = shared.lock().unwrap();
        let e = s.peripherals.edid.as_ref().expect("EDID attached to HDMI");
        assert_eq!(e.present, present);
        assert_eq!(e.reads, u64::from(present));
        assert_eq!(e.bytes_read, if present { 128 } else { 0 });
        assert_eq!(e.ignored_writes, 0);
        assert!(
            console(&s) == expected.as_bytes(),
            "EDID report differs (identifying bytes withheld)"
        );
        assert!(s.peripherals.video.as_ref().unwrap().frames_decoded > 0);
        let r = s.peripherals.rtc.as_ref().unwrap();
        assert_eq!((r.bytes_read, r.bytes_written), (0, 0));
        (e.clone(), r.clone(), console(&s))
    };
    commands.send(Command::GoTo(100_000)).unwrap();
    wait(&shared, 180, "EDID rewind", |s| {
        !s.running && s.cycles == 100_000
    });
    if present {
        assert!(
            shared
                .lock()
                .unwrap()
                .peripherals
                .edid
                .as_ref()
                .unwrap()
                .bytes_read
                < 128
        );
    }
    commands.send(Command::Step(1_900_000)).unwrap();
    wait(&shared, 180, "EDID replay", |s| {
        !s.running && s.cycles == 2_000_000
    });
    let s = shared.lock().unwrap();
    assert_eq!(s.peripherals.edid.as_ref().unwrap(), &edid);
    assert_eq!(s.peripherals.rtc.as_ref().unwrap(), &rtc);
    assert!(
        console(&s) == text,
        "EDID replay differs (identifying bytes withheld)"
    );
}
