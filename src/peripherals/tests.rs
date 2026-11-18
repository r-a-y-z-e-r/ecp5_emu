use super::{
    tmds::{Encoder, CONTROL},
    video::{crc32, Frame, VideoDecoder},
    Attached,
};
use crate::{board, fast::Trace};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

const PIXELS: usize = 800 * 525;
const COLORS: [[u8; 3]; 8] = [
    [255, 255, 255],
    [255, 255, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [255, 0, 0],
    [0, 0, 255],
    [0, 0, 0],
];
fn image(frame: usize, moving: bool) -> Vec<u8> {
    (0..480)
        .flat_map(|_| {
            (0..640).flat_map(move |x| COLORS[((x + if moving { frame } else { 0 }) % 640) / 80])
        })
        .collect()
}

/// Build the DDR pad rows, independently phased lanes and clock included.
/// Encoder state is advanced even through rows omitted at the stream's start.
fn stream(
    start_row: usize,
    frames: usize,
    hdmi: bool,
    moving: bool,
    gap: bool,
    skew: [usize; 4],
) -> (VideoDecoder, Vec<Arc<Frame>>) {
    stream_colors(start_row, frames, hdmi, moving, gap, skew, COLORS)
}
fn stream_colors(
    start_row: usize,
    frames: usize,
    hdmi: bool,
    moving: bool,
    gap: bool,
    skew: [usize; 4],
    colors: [[u8; 3]; 8],
) -> (VideoDecoder, Vec<Arc<Frame>>) {
    let mut decoder = VideoDecoder::new();
    let mut enc: [Encoder; 3] = std::array::from_fn(|_| Encoder::default());
    let mut trace = Trace {
        width: 4,
        ..Trace::default()
    };
    let mut history = [[0u8; 32]; 4];
    let mut received = Vec::new();
    let mut elapsed = Duration::ZERO;
    let mut rows = 0;
    let mut gap_sent = false;
    for p in 0..PIXELS * frames + 800 {
        let x = p % 800;
        let y = p / 800 % 525;
        let frame = p / PIXELS;
        let control =
            u8::from(!(656..752).contains(&x)) | (u8::from(!(490..492).contains(&y)) << 1);
        let active = x < 640 && y < 480;
        let preamble = hdmi && y < 479 && (790..798).contains(&x);
        let guard = hdmi && y < 479 && x >= 798;
        // The preamble and guards immediately precede the next active line.
        // Line 0's preamble is in the final line of vblank.
        let preamble0 = hdmi && y == 524 && (790..798).contains(&x);
        let guard0 = hdmi && y == 524 && x >= 798;
        let island_pre = hdmi && y == 500 && (100..108).contains(&x);
        let island = hdmi && y == 500 && (108..144).contains(&x);
        let rgb = colors[((x + if moving { frame } else { 0 }) % 640) / 80 % 8];
        let mut words = [0u16; 4];
        for lane in 0..3 {
            words[lane] = if active {
                enc[lane].data(rgb[2 - lane])
            } else if guard || guard0 {
                if lane == 1 {
                    0b0100110011
                } else {
                    0b1011001100
                }
            } else if island {
                if !(110..142).contains(&x) {
                    if lane == 0 {
                        0b1011000011
                    } else {
                        0b0100110011
                    }
                } else {
                    [0b1010001110, 0b1001110001, 0b1011100100][lane]
                }
            } else {
                enc[lane].control(if lane == 0 {
                    control
                } else if (preamble || preamble0) && lane == 1 || island_pre {
                    1
                } else {
                    0
                })
            };
        }
        words[3] = 0b0000011111;
        for b in 0..10 {
            let bit = p * 10 + b;
            for lane in 0..4 {
                history[lane][bit % 32] = ((words[lane] >> b) & 1) as u8;
            }
            for half in 0..2 {
                let row = bit * 2 + half;
                if row < start_row {
                    continue;
                }
                if gap && !gap_sent && p >= PIXELS + 800 * 200 {
                    decoder.feed(&trace);
                    trace.times_ps.clear();
                    trace.rows.clear();
                    decoder.gap();
                    gap_sent = true;
                }
                // Omit a full line, as a lost trace chunk would do.
                if gap && (PIXELS + 800 * 200..PIXELS + 800 * 201).contains(&p) {
                    continue;
                }
                trace.times_ps.push(row as u64 * 2000);
                for lane in 0..4 {
                    trace.rows.push(if bit >= skew[lane] {
                        history[lane][(bit - skew[lane]) % 32]
                    } else {
                        0
                    });
                }
                if trace.times_ps.len() >= 100_000 {
                    let before = Instant::now();
                    decoder.feed(&trace);
                    elapsed += before.elapsed();
                    rows += trace.times_ps.len();
                    trace.times_ps.clear();
                    trace.rows.clear();
                    if let Some(frame) = decoder.last_frame() {
                        if received
                            .last()
                            .is_none_or(|f: &Arc<Frame>| f.number != frame.number)
                        {
                            received.push(frame);
                        }
                    }
                }
            }
        }
    }
    decoder.feed(&trace);
    if let Some(frame) = decoder.last_frame() {
        if received.last().is_none_or(|f| f.number != frame.number) {
            received.push(frame);
        }
    }
    println!(
        "synthetic video: {:.2} ns/row, {} frames, {} invalid, {} relocks",
        elapsed.as_nanos() as f64 / rows as f64,
        received.len(),
        decoder.status().invalid_symbols,
        decoder.status().relocks
    );
    (decoder, received)
}
fn check(frame: &Frame, expected: &[u8]) {
    assert_eq!((frame.width, frame.height), (640, 480));
    assert_eq!(frame.rgb, expected);
    assert_eq!(frame.crc32, crc32(expected));
    let t = &frame.timing;
    assert_eq!((t.htotal, t.vtotal), (800, 525));
    assert_eq!((t.hsync_width, t.vsync_width), (96, 2));
    assert!(!t.hsync_positive && !t.vsync_positive);
    assert!(t.regular);
    assert_eq!(t.pixel_clock_hz, 25e6);
    assert!((t.refresh_hz - 59.5238095).abs() < 0.000001);
}
#[test]
fn bars_and_crc() {
    assert_eq!(crc32(b"123456789"), 0xcbf43926);
    // Python zlib.crc32 of the reference RGB image.
    assert_eq!(crc32(&image(0, false)), 0x2eb122d9);
    let (d, frames) = stream(0, 3, false, false, false, [0, 0, 0, 3]);
    assert!(d.status().locked, "{:?}", d.status());
    assert_eq!(frames.len(), 2);
    for f in frames {
        check(&f, &image(0, false));
    }
}
#[test]
fn starts_mid_frame_and_lanes_align_independently() {
    let (_, frames) = stream(217_000 * 20 + 7, 3, false, false, false, [0, 2, 7, 5]);
    assert!(!frames.is_empty());
    check(&frames[0], &image(0, false));
}
#[test]
fn consecutive_frames_have_numbers_and_different_crcs() {
    let (_, frames) = stream(0, 3, false, true, false, [0, 0, 0, 0]);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].number, frames[0].number + 1);
    assert_ne!(frames[0].crc32, frames[1].crc32);
    for f in frames {
        check(&f, &image(f.number as usize, true));
    }
}
#[test]
fn gap_drops_the_damaged_frame() {
    let (d, frames) = stream(0, 3, false, true, true, [0, 0, 0, 0]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].number, 2);
    assert!(d.status().relocks >= 1);
    check(&frames[0], &image(2, true));
}
#[test]
fn hdmi_preamble_guards_and_data_island() {
    let (_, frames) = stream(0, 3, true, false, false, [0, 0, 0, 4]);
    assert!(!frames.is_empty());
    for f in frames {
        check(&f, &image(0, false));
    }
}
#[test]
fn noise_constant_and_unknown_have_no_frame() {
    for mode in 0..5 {
        let mut d = VideoDecoder::new();
        let mut random = 0x12345678u32;
        for row in 0..100_000u64 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let mut pads = match mode {
                0 => [0; 4],
                1 => [1; 4],
                _ => std::array::from_fn(|i| ((random >> i) & 1) as u8),
            };
            if mode >= 3 {
                pads[1] = mode - 1;
            }
            d.sample(row * 2000, pads);
        }
        assert!(!d.status().locked);
        assert!(d.last_frame().is_none());
        assert!(!d.status().note.is_empty());
    }
}
#[test]
fn attaches_only_when_all_positive_pairs_drive() {
    let mut used = BTreeMap::new();
    assert!(Attached::new(&used).balls().is_empty());
    for ball in board::GPDI_DP {
        used.insert(ball.into(), board::Direction::Output);
    }
    assert_eq!(Attached::new(&used).balls(), board::GPDI_DP);
    used.insert("A14".into(), board::Direction::Input);
    assert!(Attached::new(&used).view().video.is_none());
    assert_eq!(CONTROL[0], 0b1101010100);
}

#[test]
fn guard_codes_are_also_video_bytes_without_a_guard_preamble() {
    let colors = [
        [0x55, 0xab, 0x55],
        [0xab, 0x55, 0xab],
        [0x12, 0x34, 0x56],
        [0x78, 0x9a, 0xbc],
        [0xde, 0xf0, 0x13],
        [0x57, 0x9b, 0xdf],
        [0x24, 0x68, 0xac],
        [0xef, 0xcd, 0x89],
    ];
    let expected: Vec<_> = (0..480)
        .flat_map(|_| (0..640).flat_map(|x| colors[x / 80]))
        .collect();
    for hdmi in [false, true] {
        let (_, frames) = stream_colors(0, 3, hdmi, false, false, [0, 0, 0, 3], colors);
        assert!(!frames.is_empty());
        for f in frames {
            check(&f, &expected);
        }
    }
}
#[test]
fn invalid_symbol_run_and_clock_stop_lose_lock() {
    use super::tmds::{decode, Symbol};
    let invalid = (0..1024).find(|&w| decode(w) == Symbol::Invalid).unwrap();
    let mut d = VideoDecoder::new();
    for pixel in 0..400u64 {
        let word = if (160..192).contains(&pixel) {
            invalid
        } else {
            CONTROL[0]
        };
        for bit in 0..10u64 {
            let data = ((word >> bit) & 1) as u8;
            for half in 0..2 {
                d.sample(
                    (pixel * 20 + bit * 2 + half) * 2000,
                    [data, data, data, u8::from(bit < 5)],
                );
            }
        }
        if pixel == 159 {
            assert!(d.status().locked);
        }
        if pixel == 191 {
            assert!(!d.status().locked);
            assert!(d.status().invalid_symbols >= 48);
            assert_eq!(d.status().relocks, 1);
        }
    }
    assert!(d.status().locked);
    for row in 8000..8100 {
        d.sample(row * 2000, [0; 4]);
    }
    assert!(!d.status().locked);
    assert_eq!(d.status().note, "clock stopped or samples missing");
    assert!(d.last_frame().is_none());
}

/// The OLED's five pads as trace rows: CS#, D/C, SCLK, MOSI, RES#, a row per
/// SCLK phase, 80 ns apart.
fn oled_rows(bytes: &[(u8, bool)], rows: &mut Vec<u8>, times: &mut Vec<u64>) {
    let mut push = |row: [u8; 5]| {
        times.push(times.last().map_or(0, |t| t + 80_000));
        rows.extend(row);
    };
    // RES# low, then high, CS# high.
    push([1, 0, 0, 0, 0]);
    push([1, 0, 0, 0, 1]);
    for &(byte, data) in bytes {
        for bit in (0..8).rev() {
            let b = byte >> bit & 1;
            push([0, u8::from(data), 0, b, 1]);
            push([0, u8::from(data), 1, b, 1]);
        }
        push([1, u8::from(data), 0, 0, 1]);
    }
}

#[test]
fn attached_oled_feeds_views_and_goes_back() {
    let mut used = BTreeMap::new();
    for ball in [
        board::OLED_CSN,
        board::OLED_DC,
        board::OLED_CLK,
        board::OLED_MOSI,
        board::OLED_RESN,
    ] {
        used.insert(ball.into(), board::Direction::Output);
    }
    let mut a = Attached::new(&used);
    assert!(a.any());
    assert_eq!(a.balls().len(), 5);
    assert!(a.view().video.is_none() && a.view().hdmi_mode().is_none());
    let before = a.snapshot();
    let (w, h, black, serial) = a.view().oled_screen().unwrap();
    assert_eq!((w, h, black.len()), (96, 64, 96 * 64 * 4));
    // Display on, remap 0x72 (65k colours), a full window of one red pixel
    // then zeros: the top-left pixel is red.
    let mut bytes = vec![(0xa0, false), (0x72, false), (0xaf, false)];
    bytes.extend([(0xf8, true), (0x00, true)]);
    let (mut rows, mut times) = (Vec::new(), Vec::new());
    oled_rows(&bytes, &mut rows, &mut times);
    a.feed(&Trace {
        width: 5,
        times_ps: times,
        rows,
        gap: false,
    });
    let view = a.view();
    assert!(view.oled.as_ref().unwrap().display_on);
    let (_, _, rgba, lit) = view.oled_screen().unwrap();
    assert_ne!(lit, serial);
    assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    // Going back shows the panel as it was.
    a.restore(&before);
    let view = a.view();
    assert!(!view.oled.unwrap().display_on);
    assert_eq!(view.oled_rgba.unwrap().0[..4], [0, 0, 0, 255]);
}

#[test]
fn attached_sdram_horizon_respond_and_snapshot() {
    let mut used = BTreeMap::new();
    for ball in [
        board::SDRAM_CLK,
        board::SDRAM_CKE,
        board::SDRAM_CSN,
        board::SDRAM_RASN,
        board::SDRAM_CASN,
        board::SDRAM_WEN,
    ]
    .into_iter()
    .chain(board::SDRAM_BA)
    .chain(board::SDRAM_A)
    .chain(board::SDRAM_DQM)
    {
        used.insert(ball.into(), board::Direction::Output);
    }
    for ball in board::SDRAM_D {
        used.insert(ball.into(), board::Direction::Bidirectional);
    }

    // Must not attach if any DQ ball is not Bidirectional.
    let mut non_bidir = used.clone();
    non_bidir.insert(board::SDRAM_D[0].into(), board::Direction::Output);
    assert!(Attached::new(&non_bidir).balls().is_empty());

    let mut a = Attached::new(&used);
    assert!(a.any());
    assert_eq!(a.balls().len(), 39);
    assert_eq!(a.horizon(), Some(1));
    assert_eq!(a.input_balls(), board::SDRAM_D.map(|b| (b, None)).to_vec());
    assert!(a.respond().is_empty());

    let mut time_ps = 0u64;
    let mut tick = |a: &mut Attached,
                    cmd: (u8, u8, u8, u8),
                    ba: u16,
                    addr: u16,
                    dqm: u8,
                    dq: u16|
     -> Vec<(usize, Option<bool>)> {
        let row_of = |clk: u8| -> [u8; 39] {
            let mut r = [0u8; 39];
            r[0] = clk;
            r[1] = 1; // cke
            r[2] = cmd.0;
            r[3] = cmd.1;
            r[4] = cmd.2;
            r[5] = cmd.3;
            for i in 0..2 {
                r[6 + i] = ((ba >> i) & 1) as u8;
            }
            for i in 0..13 {
                r[8 + i] = ((addr >> i) & 1) as u8;
            }
            for i in 0..2 {
                r[21 + i] = (dqm >> i) & 1;
            }
            for i in 0..16 {
                r[23 + i] = ((dq >> i) & 1) as u8;
            }
            r
        };
        let mut rows = Vec::with_capacity(78);
        rows.extend(row_of(0));
        rows.extend(row_of(1));
        let times_ps = vec![time_ps + 20_000, time_ps + 40_000];
        time_ps += 40_000;
        a.feed(&Trace {
            width: 39,
            times_ps,
            rows,
            gap: false,
        });
        a.respond()
    };

    const NOP: (u8, u8, u8, u8) = (0, 1, 1, 1);
    const ACTIVE: (u8, u8, u8, u8) = (0, 0, 1, 1);
    const READ: (u8, u8, u8, u8) = (0, 1, 0, 1);
    const WRITE: (u8, u8, u8, u8) = (0, 1, 0, 0);
    const PRECHARGE: (u8, u8, u8, u8) = (0, 0, 1, 0);
    const REFRESH: (u8, u8, u8, u8) = (0, 0, 0, 1);
    const LOAD_MODE: (u8, u8, u8, u8) = (0, 0, 0, 0);

    // 100 us wait, PRECHARGE ALL, 2x AUTO REFRESH, LOAD MODE (CL2, BL1).
    for _ in 0..2501 {
        assert!(tick(&mut a, NOP, 0, 0, 0, 0).is_empty());
    }
    tick(&mut a, PRECHARGE, 0, 1024, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, REFRESH, 0, 0, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, REFRESH, 0, 0, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, LOAD_MODE, 0, 0x20, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);

    tick(&mut a, ACTIVE, 0, 5, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, WRITE, 0, 3, 0, 0xabcd);
    tick(&mut a, NOP, 0, 0, 0, 0);
    tick(&mut a, NOP, 0, 0, 0, 0);
    let snap = a.snapshot();

    // Unmasked read: CL2 puts 0xabcd on the next rising edge, then releases to Z.
    assert!(tick(&mut a, READ, 0, 3, 0, 0).is_empty());
    let expected: Vec<_> = (0..16).map(|i| (i, Some((0xabcd >> i) & 1 != 0))).collect();
    assert_eq!(tick(&mut a, NOP, 0, 0, 0, 0), expected);
    let release: Vec<_> = (0..16).map(|i| (i, None)).collect();
    assert_eq!(tick(&mut a, NOP, 0, 0, 0, 0), release);

    // Restore checkpoint and test partial DQM release (dqm=1 masks low byte).
    a.restore(&snap);
    assert!(tick(&mut a, READ, 0, 3, 1, 0).is_empty());
    let high_only: Vec<_> = (8..16).map(|i| (i, Some((0xabcd >> i) & 1 != 0))).collect();
    assert_eq!(tick(&mut a, NOP, 0, 0, 0, 0), high_only);
    let release_high: Vec<_> = (8..16).map(|i| (i, None)).collect();
    assert_eq!(tick(&mut a, NOP, 0, 0, 0, 0), release_high);

    let status = a.view().sdram.unwrap();
    assert!(status.initialised);
    assert_eq!(status.timing_violations, 0, "{:?}", status.messages);
    assert_eq!(status.writes, 1);
    assert_eq!(status.reads, 1);
}

#[test]
fn flash_attachment_answers_and_restores_from_balls() {
    let mut used = BTreeMap::from([
        (board::FLASH_CSN.into(), board::Direction::Output),
        (board::FLASH_MOSI.into(), board::Direction::Output),
        (board::FLASH_MISO.into(), board::Direction::Input),
    ]);
    assert!(Attached::new(&used).view().flash.is_none());
    used.insert(board::FLASH_SCK.into(), board::Direction::Input);
    assert!(Attached::new(&used).view().flash.is_none());
    used.insert(board::FLASH_SCK.into(), board::Direction::Output);
    let mut attached = Attached::new(&used);
    assert_eq!(attached.input_balls(), [(board::FLASH_MISO, Some(true))]);
    assert_eq!(attached.horizon(), Some(64));
    assert!(attached.observe_every_edge());
    assert_eq!(
        attached.balls(),
        [board::FLASH_CSN, board::FLASH_SCK, board::FLASH_MOSI]
    );
    let sample = |a: &mut Attached, cs, clk, mosi| {
        a.feed(&Trace {
            width: 3,
            times_ps: vec![0],
            rows: vec![cs, clk, mosi],
            gap: false,
        });
    };
    sample(&mut attached, 1, 3, 1);
    assert!(!attached.view().flash.unwrap().selected);
    sample(&mut attached, 0, 0, 1);
    assert_eq!(attached.horizon(), Some(1));
    for bit in (0..8).rev() {
        let mosi = (0x9f >> bit) & 1;
        sample(&mut attached, 0, 0, mosi);
        sample(&mut attached, 0, 1, mosi);
    }
    let saved = attached.snapshot();
    let read = |a: &mut Attached| {
        let mut byte = 0;
        for _ in 0..8 {
            sample(a, 0, 0, 1);
            a.respond();
            byte = (byte << 1) | u8::from(a.input_balls()[0].1.unwrap());
            sample(a, 0, 1, 1);
        }
        byte
    };
    assert_eq!(read(&mut attached), 0x9d);
    attached.restore(&saved);
    assert_eq!(read(&mut attached), 0x9d);
    sample(&mut attached, 1, 0, 1);
    assert_eq!(attached.horizon(), Some(64));
}

#[test]
fn adc_attachment_traces_input_pulls_answers_and_restores() {
    let mut used = BTreeMap::from([
        (board::ADC_CSN.into(), board::Direction::Output),
        (board::ADC_SCLK.into(), board::Direction::Output),
        (board::ADC_MOSI.into(), board::Direction::Output),
    ]);
    assert!(Attached::new(&used).view().adc.is_none());
    used.insert(board::ADC_MISO.into(), board::Direction::Input);
    for ball in board::ADC_AIN {
        used.insert(ball.into(), board::Direction::Input);
    }
    let mut a = Attached::new(&used);
    assert_eq!(
        a.balls(),
        [board::ADC_CSN, board::ADC_SCLK, board::ADC_MOSI]
            .into_iter()
            .chain(board::ADC_AIN)
            .collect::<Vec<_>>()
    );
    assert_eq!(a.input_balls(), [(board::ADC_MISO, Some(true))]);
    assert_eq!(a.horizon(), Some(64));
    assert!(a.observe_every_edge());
    let sample = |a: &mut Attached, cs, clk, mosi| {
        let mut rows = vec![cs, clk, mosi];
        rows.extend([1, 0, 1, 0, 1, 0, 1, 0]);
        a.feed(&Trace {
            width: 11,
            times_ps: vec![0],
            rows,
            gap: false,
        });
    };
    sample(&mut a, 0, 1, 0);
    assert_eq!(a.horizon(), Some(1));
    for bit in (0..16).rev() {
        sample(&mut a, 0, 0, ((0x0804 >> bit) & 1) as u8);
        sample(&mut a, 0, 1, ((0x0804 >> bit) & 1) as u8);
    }
    sample(&mut a, 1, 1, 0);
    let saved = a.snapshot();
    assert!(a.set_volts(0, 1.65));
    sample(&mut a, 0, 1, 0);
    sample(&mut a, 0, 0, 0);
    assert_eq!(a.respond(), [(0, Some(false))]);
    assert_eq!(a.view().adc.unwrap().codes[0], Some(2048));
    a.restore(&saved);
    sample(&mut a, 0, 1, 0);
    assert_eq!(a.view().adc.unwrap().codes[0], Some(4095));
    assert!(a.view().adc.unwrap().driven.iter().all(|d| !d));
    used.insert(board::ADC_AIN[0].into(), board::Direction::Output);
    assert!(Attached::new(&used).view().adc.unwrap().driven[0]);
}

#[test]
fn rtc_attachment_is_open_drain_and_checkpoints_the_bus_and_clock() {
    let mut used = BTreeMap::from([(board::RTC_SCL.into(), board::Direction::Output)]);
    assert!(Attached::new(&used).view().rtc.is_none());
    used.insert(board::RTC_SDA.into(), board::Direction::Input);
    assert!(Attached::new(&used).view().rtc.is_none());
    used.insert(board::RTC_SDA.into(), board::Direction::Bidirectional);
    let mut a = Attached::new(&used);
    assert_eq!(a.balls(), [board::RTC_SCL, board::RTC_SDA]);
    assert_eq!(a.input_balls(), [(board::RTC_SDA, None)]);
    assert_eq!(a.horizon(), Some(16));
    let sample = |a: &mut Attached, ps, scl, sda| {
        a.feed(&Trace {
            width: 2,
            times_ps: vec![ps],
            rows: vec![scl, sda],
            gap: false,
        })
    };
    sample(&mut a, 0, 1, 0);
    assert_eq!(a.horizon(), Some(1));
    for bit in (0..8).rev() {
        sample(&mut a, 100, 0, (0xde >> bit) & 1);
        sample(&mut a, 200, 1, (0xde >> bit) & 1);
    }
    sample(&mut a, 300, 0, 1);
    assert_eq!(a.respond(), [(0, Some(false))]);
    let saved = a.snapshot();
    sample(&mut a, 400, 0, 0);
    sample(&mut a, 500, 1, 0);
    sample(&mut a, 600, 0, 0);
    assert_eq!(a.respond(), [(0, None)]);
    a.restore(&saved);
    assert_eq!(a.input_balls(), [(board::RTC_SDA, Some(false))]);
    sample(&mut a, 400, 0, 0);
    sample(&mut a, 500, 1, 0);
    sample(&mut a, 600, 0, 0);
    assert_eq!(a.respond(), [(0, None)]);
    sample(&mut a, 700, 1, 0);
    sample(&mut a, 800, 1, 1);
    assert_eq!(a.horizon(), Some(16));
    let saved = a.snapshot();
    sample(&mut a, super::rtc::SECOND_PS + 800, 1, 1);
    let status = a.view().rtc.unwrap();
    a.restore(&saved);
    sample(&mut a, super::rtc::SECOND_PS + 800, 1, 1);
    assert_eq!(a.view().rtc.unwrap(), status);
}

fn audio_pins(code: u8) -> [u8; 4] {
    std::array::from_fn(|i| (code >> i) & 1)
}

#[test]
fn audio_ladder_uses_conductance_and_excludes_disconnected_bits() {
    use super::audio::ladder;
    for (code, volts) in [
        (0, 0.0),
        (4, 0.865192314050332),
        (8, 1.7969378830276124),
        (12, 2.6621301970779445),
        (15, 3.3),
    ] {
        let (v, r) = ladder(audio_pins(code));
        assert!((v.unwrap() - volts).abs() < 1e-12);
        assert!((r.unwrap() - 70.78846205866353).abs() < 1e-12);
    }
    let (v, r) = ladder([1, 0, 3, 3]);
    assert!((v.unwrap() - 3.3 * 549.0 / 1649.0).abs() < 1e-12);
    assert!((r.unwrap() - 1100.0 * 549.0 / 1649.0).abs() < 1e-10);
    assert_eq!(ladder([3; 4]), (None, None));
    assert_eq!(ladder([0, 1, 2, 3]), (None, None));
}

#[test]
fn audio_estimates_quantized_sines_and_ages_the_one_second_window() {
    use super::audio::{Audio, SCOPE_SAMPLES};
    let mut audio = Audio::new([true; 3]);
    for sample in 0..=25_000u64 {
        let sine = |hz: f64| {
            (8.0 + 4.0 * (std::f64::consts::TAU * hz * sample as f64 / 25_000.0).sin()).round()
                as u8
        };
        audio.feed(
            sample * 40_000_000,
            [
                audio_pins(sine(440.0)),
                audio_pins(sine(660.0)),
                audio_pins(8),
            ],
        );
    }
    let s = audio.status();
    for (c, hz) in s.channels[..2].iter().zip([440.0, 660.0]) {
        assert!((c.hz.unwrap() - hz).abs() < 1.0, "{c:?}");
        assert!((c.vpp.unwrap() - 1.7969378830276124).abs() < 1e-12);
    }
    assert_eq!(s.channels[2].vpp, Some(0.0));
    assert_eq!(s.channels[2].hz, None);
    assert_eq!(s.channels[2].transitions, 0);
    assert_eq!(s.scope.len(), SCOPE_SAMPLES);
    for p in s.scope.windows(2) {
        assert!((20_833_333..=20_833_334).contains(&(p[1].ps - p[0].ps)));
    }
    audio.feed(1_100_000_000_000, [audio_pins(8); 3]);
    assert_eq!(audio.status().channels[0].hz, None);
    audio.feed(2_100_000_000_000, [audio_pins(8); 3]);
    let s = audio.status();
    assert_eq!(s.channels[0].vpp, Some(0.0));
    assert_eq!(s.channels[0].min_volts, s.channels[0].volts);
}

#[test]
fn audio_attaches_partial_ladders_and_restores_history_and_scope_exactly() {
    let used = BTreeMap::from([
        (board::AUDIO_L[0].to_string(), board::Direction::Output),
        (board::AUDIO_L[1].to_string(), board::Direction::Input),
        (
            board::AUDIO_R[3].to_string(),
            board::Direction::Bidirectional,
        ),
    ]);
    let mut attached = Attached::new(&used);
    assert_eq!(attached.balls(), [board::AUDIO_L[0], board::AUDIO_R[3]]);
    assert!(attached.input_balls().is_empty());
    assert_eq!(attached.horizon(), None);
    let trace = |time, row: [u8; 2]| Trace {
        width: 2,
        times_ps: vec![time],
        rows: row.into(),
        ..Trace::default()
    };
    attached.feed(&trace(0, [1, 0]));
    attached.feed(&trace(5_000_000_000, [0, 1]));
    let before = attached.view().audio.unwrap();
    assert_eq!(before.channels[0].code_text(), "zzz0");
    assert_eq!(before.channels[0].source_ohms, Some(1100.0));
    assert!((before.channels[0].vpp.unwrap() - 3.3).abs() < 1e-12);
    assert!(!before.channels[2].attached);
    let saved = attached.snapshot();
    attached.feed(&trace(20_000_000_000, [1, 3]));
    let later = attached.view().audio.unwrap();
    assert_eq!(later.channels[1].volts, None);
    attached.restore(&saved);
    assert_eq!(attached.view().audio.unwrap(), before);
    attached.feed(&trace(20_000_000_000, [1, 3]));
    assert_eq!(attached.view().audio.unwrap(), later);
    let line = attached.report().pop().unwrap();
    assert!(line.starts_with("peripheral: audio l_hz=-- l_vpp=3.300000 r_hz=-- r_vpp=3.300000 v_hz=-- v_vpp=-- transitions=4"), "{line}");
}

#[test]
fn hdmi_edid_and_rtc_share_one_open_drain_driver_and_one_trace() {
    use super::{
        edid::Edid,
        rtc::{Pins, Rtc},
    };
    fn attached() -> Attached {
        let used = board::GPDI_DP
            .into_iter()
            .map(|b| (b.to_string(), board::Direction::Output))
            .chain([
                (board::RTC_SCL.to_string(), board::Direction::Output),
                (board::RTC_SDA.to_string(), board::Direction::Bidirectional),
            ])
            .collect();
        let mut a = Attached::new(&used);
        let mut image = Rtc::default().image();
        image[0] &= 0x7f;
        image[7] = 0x80;
        a.rtc = Some(Rtc::new(image));
        a.edid = Some(Edid::default());
        a
    }
    struct Bus {
        a: Attached,
        scl: u8,
        low: bool,
        time: u64,
        trace: Trace,
    }
    impl Bus {
        fn line(&self) -> u8 {
            u8::from(!self.low && self.a.input_balls()[0].1.is_none())
        }
        fn row(&mut self, ps: u64) {
            let pins = Pins {
                scl: self.scl,
                sda: self.line(),
            };
            let row = [0, 0, 0, 0, pins.scl, pins.sda];
            self.trace.times_ps.push(ps);
            self.trace.rows.extend(row);
            self.a.feed(&Trace {
                width: 6,
                times_ps: vec![ps],
                rows: row.into(),
                ..Trace::default()
            });
        }
        fn set(&mut self, scl: u8, low: bool) {
            self.scl = scl;
            self.low = low;
            self.time += 1_000;
            self.row(self.time);
            let changes = self.a.respond();
            assert!(changes.iter().all(|&(i, v)| i == 0 && v != Some(true)));
            let expected = (self.a.rtc.as_ref().unwrap().sda() == Some(false)
                || self.a.edid.as_ref().unwrap().sda() == Some(false))
            .then_some(false);
            assert_eq!(self.a.input_balls(), [(board::RTC_SDA, expected)]);
            self.row(self.time + 1);
            assert!(self.a.respond().is_empty());
        }
        fn start(&mut self) {
            self.set(0, false);
            self.set(1, false);
            self.set(1, true);
            self.set(0, true);
            assert_eq!(self.a.horizon(), Some(1));
        }
        fn stop(&mut self) {
            self.set(0, true);
            self.set(1, true);
            self.set(1, false);
            assert_eq!(self.a.horizon(), Some(16));
        }
        fn write(&mut self, byte: u8) -> bool {
            for bit in (0..8).rev() {
                self.set(0, byte & (1 << bit) == 0);
                self.set(1, self.low);
            }
            self.set(0, false);
            self.set(1, false);
            let ack = self.line() == 0;
            self.set(0, false);
            ack
        }
        fn read(&mut self, ack: bool) -> u8 {
            let mut byte = 0;
            for _ in 0..8 {
                self.set(1, false);
                byte = (byte << 1) | self.line();
                self.set(0, false);
            }
            self.set(0, ack);
            self.set(1, ack);
            self.set(0, false);
            byte
        }
        fn random(&mut self, address: u8, pointer: u8, n: usize) -> Vec<u8> {
            self.start();
            assert!(self.write(address << 1));
            assert!(self.write(pointer));
            self.start();
            assert!(self.write((address << 1) | 1));
            let data = (0..n).map(|i| self.read(i + 1 < n)).collect();
            self.stop();
            data
        }
    }
    let mut bus = Bus {
        a: attached(),
        scl: 1,
        low: false,
        time: 0,
        trace: Trace {
            width: 6,
            ..Trace::default()
        },
    };
    assert_eq!(
        bus.a.balls(),
        board::GPDI_DP
            .into_iter()
            .chain([board::RTC_SCL, board::RTC_SDA])
            .collect::<Vec<_>>()
    );
    assert_eq!(bus.random(0x6f, 7, 1), [0x80]);
    assert_eq!(bus.a.edid.as_ref().unwrap().status().bytes_read, 0);
    assert_eq!(bus.random(0x50, 0, 8), super::edid::default_block()[..8]);
    assert_eq!(bus.a.rtc.as_ref().unwrap().status().bytes_read, 1);
    bus.start();
    assert!(!bus.write(0x60));
    bus.stop();
    let mut replay = attached();
    replay.feed(&bus.trace);
    replay.respond();
    assert_eq!(replay.view().rtc, bus.a.view().rtc);
    assert_eq!(replay.view().edid, bus.a.view().edid);
    let snapshot = bus.a.snapshot();
    let before = bus.a.view();
    bus.random(0x50, 126, 4);
    bus.a.restore(&snapshot);
    assert_eq!(bus.a.view().rtc, before.rtc);
    assert_eq!(bus.a.view().edid, before.edid);
    bus.a.edid = Some(Edid::new(&super::edid::default_block(), false).unwrap());
    bus.start();
    assert!(!bus.write(0xa0));
    bus.stop();
    assert_eq!(bus.random(0x6f, 7, 1), [0x80]);
    let mut video_only = Attached::new(
        &board::GPDI_DP
            .map(|b| (b.to_string(), board::Direction::Output))
            .into(),
    );
    assert!(video_only.view().edid.is_some());
    assert!(video_only.input_balls().is_empty());
    assert_eq!(video_only.horizon(), None);
    let mut rtc_only = Attached::new(
        &[
            (board::RTC_SCL.to_string(), board::Direction::Output),
            (board::RTC_SDA.to_string(), board::Direction::Bidirectional),
        ]
        .into(),
    );
    assert!(rtc_only.view().edid.is_none());
}

#[test]
fn header_and_adc_share_columns_and_checkpoint_external_sources() {
    let used = [
        (board::ADC_CSN.to_string(), board::Direction::Output),
        (board::ADC_SCLK.to_string(), board::Direction::Output),
        (board::ADC_MOSI.to_string(), board::Direction::Output),
        (board::ADC_MISO.to_string(), board::Direction::Input),
        (board::ADC_AIN[0].to_string(), board::Direction::Input),
    ]
    .into();
    let mut attached = Attached::new(&used);
    assert_eq!(
        attached
            .balls()
            .iter()
            .filter(|&&b| b == board::ADC_AIN[0])
            .count(),
        1
    );
    attached.set_volts(0, 1.25);
    assert!(attached.set_header_level(28, Some(true)));
    assert_eq!(attached.view().adc.unwrap().volts[0], 3.3);
    let saved = attached.snapshot();
    attached.set_header_level(28, None);
    assert_eq!(attached.view().adc.unwrap().volts[0], 1.25);
    attached.restore(&saved);
    assert_eq!(attached.header_levels()[28], Some(true));
    assert_eq!(attached.view().adc.unwrap().volts[0], 3.3);
}

#[test]
fn native_sd_attach_uses_variable_answer_group_and_restores_it() {
    let mut used: BTreeMap<String, board::Direction> = [
        (board::SD_CLK.to_string(), board::Direction::Output),
        (board::SD_CMD.to_string(), board::Direction::Bidirectional),
    ]
    .into();
    for b in board::SD_D {
        used.insert(b.into(), board::Direction::Bidirectional);
    }
    for b in [board::SD_CDN, board::SD_WP] {
        used.insert(b.into(), board::Direction::Input);
    }
    let mut a = Attached::new(&used);
    assert_eq!(
        a.input_balls().iter().map(|p| p.0).collect::<Vec<_>>(),
        [
            board::SD_CMD,
            board::SD_D[0],
            board::SD_D[1],
            board::SD_D[2],
            board::SD_D[3],
            board::SD_CDN,
            board::SD_WP
        ]
    );
    assert_eq!(a.horizon(), Some(16));
    assert_eq!(a.view().sd.unwrap().mode, "sd1");
    let saved = a.snapshot();
    let levels = a.input_balls();
    a.sd_levels[0] = Some(false);
    a.restore(&saved);
    assert_eq!(a.input_balls(), levels);
    let pins = [
        board::SD_CLK,
        board::SD_CMD,
        board::SD_D[0],
        board::SD_D[1],
        board::SD_D[2],
        board::SD_D[3],
    ];
    assert_eq!(a.balls(), pins);
    // A partial command shortens the horizon before the card can answer.
    a.feed(&Trace {
        width: 6,
        times_ps: vec![1, 2],
        rows: vec![0, 0, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1],
        ..Trace::default()
    });
    assert_eq!(a.horizon(), Some(1));
    // A one-bit-only host does not acquire probes or drivers on unwritten DAT pins.
    let one = [
        (board::SD_CLK.to_string(), board::Direction::Output),
        (board::SD_CMD.to_string(), board::Direction::Bidirectional),
        (board::SD_D[0].to_string(), board::Direction::Input),
    ]
    .into();
    let a = Attached::new(&one);
    assert_eq!(a.balls(), [board::SD_CLK, board::SD_CMD, board::SD_D[0]]);
    assert_eq!(a.input_balls().len(), 2);
}
