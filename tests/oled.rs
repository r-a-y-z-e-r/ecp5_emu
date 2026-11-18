//! The routed OLED bitstream, sampled at every clock edge, against SPI RAM
//! and the independent CRC printed by the design's UART.
mod common;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::oled::{Oled, OledPins, CLK_BALL, CSN_BALL, DC_BALL, MOSI_BALL, RESN_BALL},
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn expected_image(frame: u16) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(96 * 64 * 3);
    for y in 0..64u16 {
        for x in 0..96u16 {
            let (r, g, b) = if x >= frame & 63
                && x < (frame & 63) + 16
                && y >= frame & 31
                && y < (frame & 31) + 16
            {
                (31, 63, 31)
            } else {
                ((x + frame) & 31, (y + frame) & 63, (x ^ y ^ frame) & 31)
            };
            rgb.extend_from_slice(&[
                (r * 8 + r / 4) as u8,
                (g * 4 + g / 16) as u8,
                (b * 8 + b / 4) as u8,
            ]);
        }
    }
    rgb
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn oled_pattern_matches_pixels_and_uart_crc_for_three_frames() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = common::reference::build("oled_pattern");
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
    // Probe package balls directly, so routing and PIO direction are covered.
    let references: Vec<_> = [CSN_BALL, DC_BALL, CLK_BALL, MOSI_BALL, RESN_BALL, "L4"]
        .map(|ball| Reference::Pin(ball.into()))
        .into();
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(Reference::Pin("G2".into()), Logic::Zero)],
            probes: references.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap();
    let probes: Vec<_> = references.iter().map(|r| c.probes[&r.label()]).collect();
    let clock = c.drivers["pin:G2"];
    let mut oled = Oled::new();
    let mut receiver = Receiver::new(115_200);
    let mut line = Vec::new();
    let mut edge = 0u64;
    let sample = |oled: &mut Oled, receiver: &mut Receiver, row: &[u8], edge: u64| {
        oled.feed(
            edge * 20_000,
            OledPins {
                csn: row[0],
                dc: row[1],
                clk: row[2],
                mosi: row[3],
                resn: row[4],
            },
        );
        assert!(row[5] <= 1, "UART is unknown at edge {edge}");
        receiver.sample(edge * 20, row[5] == 1)
    };
    let levels = |c: &compile::Compiled| -> Vec<u8> {
        probes
            .iter()
            .map(|&p| match c.kernel.value(p).unwrap() {
                Logic::Zero => 0,
                Logic::One => 1,
                Logic::X => 2,
                Logic::Z => 3,
            })
            .collect()
    };
    sample(&mut oled, &mut receiver, &levels(&c), 0);
    // Retry only StartsUnknown; sample both halves during the warm start too.
    let mut fast = loop {
        match FastSim::new(&c.kernel) {
            Ok(fast) => break fast,
            Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && edge < 2000 => {
                for level in [Logic::One, Logic::Zero] {
                    edge += 1;
                    let time = Time::from_ps(edge * 20_000);
                    c.kernel.schedule(time, clock, level).unwrap();
                    c.kernel.run_until(time).unwrap();
                    assert!(sample(&mut oled, &mut receiver, &levels(&c), edge).is_none());
                }
            }
            Err(e) => panic!("{e:#}"),
        }
    };
    println!("OLED warm start: {} cycles", edge / 2);
    fast.clock(clock, Time::from_ps(40_000)).unwrap();
    let mut frames = Vec::new();
    let mut received = 0;
    let mut start = Instant::now();
    // About 0.02 s of board time per frame. The bound catches stuck designs.
    'run: while edge < 5_000_000 {
        let rows = fast.run_edges_sampled(8192, &probes).unwrap();
        for row in rows.chunks_exact(probes.len()) {
            edge += 1;
            let byte = sample(&mut oled, &mut receiver, row, edge);
            if oled.frames() > frames.len() as u64 {
                let frame = frames.len() as u16;
                let seconds = start.elapsed().as_secs_f64();
                start = Instant::now();
                let status = oled.status();
                assert!(status.display_on);
                assert_eq!((status.errors, status.unknown_commands), (0, 0));
                assert_eq!(
                    &*status.image,
                    &expected_image(frame),
                    "frame {frame:04X} image"
                );
                assert_eq!(status.data_bytes, (u64::from(frame) + 1) * 96 * 64 * 2);
                frames.push((status.crc32, seconds));
            }
            if let Some(byte) = byte {
                line.push(byte);
                if byte == b'\n' {
                    let text = std::str::from_utf8(&line).unwrap();
                    assert_eq!(text.len(), 17, "{text:?}");
                    let frame = u16::from_str_radix(&text[1..5], 16).unwrap();
                    let crc = u32::from_str_radix(&text[7..15], 16).unwrap();
                    assert_eq!(usize::from(frame), received);
                    let (model_crc, seconds) = frames[received];
                    assert_eq!(text, format!("O{frame:04X} C{model_crc:08X}\r\n"));
                    assert_eq!(model_crc, crc);
                    println!("OLED frame {frame:04X}: model {model_crc:08X}, UART {crc:08X}, image exact, {seconds:.3} seconds/frame");
                    received += 1;
                    line.clear();
                    if received == 3 {
                        break 'run;
                    }
                }
            }
        }
    }
    assert_eq!(received, 3, "timed out waiting for three UART reports");
    assert_eq!(oled.frames(), 3);
    assert_eq!(receiver.framing_errors, 0);
}
