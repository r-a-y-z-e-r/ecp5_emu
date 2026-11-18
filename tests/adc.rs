//! Routed adc_read, the real PIO pulls and the answering loop used by fast mode.
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
    peripherals::Attached,
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn run(overrides: &[(usize, f64)], expected: &str, expected_codes: [u16; 8]) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = common::reference::build("adc_read");
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
    let used: std::collections::BTreeMap<String, board::Direction> =
        board::used_balls(&device).unwrap().into_iter().collect();
    for ball in board::ADC_AIN {
        assert_eq!(
            board::direction(&device, ball).unwrap(),
            Some(board::Direction::Input),
            "header pad {ball} must never be driven"
        );
    }
    for (ball, direction) in &used {
        if direction.drives() {
            assert!(
                board::LEDS.contains(&ball.as_str())
                    || [
                        board::UART_TX,
                        board::ADC_CSN,
                        board::ADC_SCLK,
                        board::ADC_MOSI
                    ]
                    .contains(&ball.as_str()),
                "unexpected output {ball}"
            );
        }
    }
    let mut attached = Attached::new(&used);
    assert_eq!(attached.balls().len(), 11);
    for &(channel, volts) in overrides {
        assert!(attached.set_volts(channel, volts));
    }
    let pin = |ball: &str| Reference::Pin(ball.into());
    let trace_balls = attached.balls();
    let refs: Vec<_> = trace_balls
        .iter()
        .map(|b| pin(b))
        .chain([pin(board::UART_TX)])
        .chain(board::LEDS.map(pin))
        .collect();
    let compiled = compile::compile(
        device,
        Request {
            package: board::PACKAGE.into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![
                (pin(board::CLOCK), Logic::Zero),
                (pin(board::ADC_MISO), Logic::One),
            ],
            probes: refs.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap();
    let probes: Vec<_> = refs.iter().map(|r| compiled.probes[&r.label()]).collect();
    let drivers = [compiled.drivers[&pin(board::ADC_MISO).label()]];
    let mut fast = FastSim::new(&compiled.kernel).unwrap();
    fast.clock(
        compiled.drivers[&pin(board::CLOCK).label()],
        Time::from_ps(40_000),
    )
    .unwrap();
    fast.trace(&probes[..11]).unwrap();
    let mut uart = Receiver::new(board::BAUD);
    let mut text = String::new();
    let mut line_cycles = Vec::new();
    let mut leds = 0;
    let start = Instant::now();
    let mut edges = 0;
    while edges < 11_000_000 && line_cycles.len() < 2 {
        edges += fast
            .run_edges_with(1024, &probes[11..], &drivers, |sample, trace, changes| {
                attached.feed(trace);
                attached.respond_into(changes);
                if let Some((ps, row)) = sample {
                    if let Some(byte) = uart.sample(ps / 1000, row[0] == 1) {
                        text.push(byte as char);
                        if byte == b'\n' {
                            line_cycles.push(ps / 40_000);
                        }
                    }
                    leds = row[1..]
                        .iter()
                        .enumerate()
                        .fold(0, |v, (i, &b)| v | (u8::from(b == 1) << i));
                }
                Ok(true)
            })
            .unwrap();
    }
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "adc_read: {} cycles in {seconds:.3} s, {:.0} cycles/s",
        edges / 2,
        edges as f64 / 2.0 / seconds
    );
    println!("UART: {}", text.trim_end());
    assert_eq!(uart.framing_errors, 0);
    assert_eq!(line_cycles.len(), 2, "timed out: {text:?}");
    for line in text.lines() {
        assert_eq!(line, expected);
    }
    assert!(
        line_cycles[1].abs_diff(line_cycles[0] + 5_000_000) <= 2,
        "5 Hz: {line_cycles:?}"
    );
    let status = attached.view().adc.unwrap();
    assert_eq!(status.codes, expected_codes.map(Some));
    assert_eq!(status.unsupported_frames, 0);
    assert_eq!(leds, (expected_codes[0] >> 4) as u8);
    text
}

fn adc_codes(line: &str) -> [u16; 8] {
    let mut words = line.split_whitespace();
    assert_eq!(words.next(), Some("ADC"), "invalid ADC line: {line:?}");
    let codes = std::array::from_fn(|channel| {
        let word = words.next().expect("ADC line needs eight channel codes");
        assert!(
            word.len() == 4 && word.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid AIN{channel} code in {line:?}"
        );
        let code = u16::from_str_radix(word, 16).unwrap();
        assert!(code <= 4095, "AIN{channel} exceeds 12 bits in {line:?}");
        code
    });
    assert!(words.next().is_none(), "extra ADC fields in {line:?}");
    codes
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn adc_read_reads_the_eight_safe_header_pulls() {
    run(
        &[],
        "ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000",
        [4095, 0, 4095, 0, 4095, 0, 4095, 0],
    );
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn adc_read_reads_external_voltage_overrides() {
    run(
        &[(0, 1.65), (1, 0.825), (6, -1.0), (7, 4.0)],
        "ADC 0800 0400 0FFF 0000 0FFF 0000 0000 0FFF",
        [2048, 1024, 4095, 0, 4095, 0, 0, 4095],
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_board_codes_are_within_the_measured_bound() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let log =
        std::fs::read_to_string(root.join("docs/inventory/LFE5U-85F/hr08_adc/observations.log"))
            .expect("HR08 board observations");
    let board: Vec<_> = log
        .lines()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with("ADC"))
        .map(|(line, text)| (line + 1, adc_codes(text)))
        .collect();
    assert_eq!(board.len(), 45, "HR08 has three loads of 15 complete lines");
    let text = run(
        &[],
        "ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000",
        [4095, 0, 4095, 0, 4095, 0, 4095, 0],
    );
    let emulator = adc_codes(text.lines().next().expect("first emulator ADC line"));
    let mut largest = [0u16; 8];
    for (_, codes) in &board {
        for channel in 0..8 {
            largest[channel] = largest[channel].max(codes[channel].abs_diff(emulator[channel]));
        }
    }
    println!("HR08 largest differences AIN0..AIN7: {largest:?}");
    // The measured maximum (30), rounded up to 32: a bound for this board,
    // not a MAX11125 accuracy specification.
    for (line, codes) in board {
        for channel in 0..8 {
            assert!(
                codes[channel].abs_diff(emulator[channel]) <= 32,
                "HR08 line {line}, AIN{channel}: board {}, emulator {}",
                codes[channel],
                emulator[channel]
            );
        }
    }
}
