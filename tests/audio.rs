//! The routed tone's digital stream and the schematic-derived, unloaded DACs.
mod common;
use ecp5_emu::{
    board,
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::{FastSim, Trace},
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::{audio, video::crc32, Attached},
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Instant};

struct Run {
    fast: FastSim,
    attached: Attached,
    tx: SignalId,
    uart: Receiver,
}
impl Run {
    fn new() -> Self {
        let out = common::reference::build("audio_tone");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
        let used: BTreeMap<_, _> = board::used_balls(&device).unwrap().into_iter().collect();
        let dac: Vec<_> = board::AUDIO_L
            .into_iter()
            .chain(board::AUDIO_R)
            .chain(board::AUDIO_V)
            .collect();
        for (ball, direction) in &used {
            assert!(
                !direction.drives()
                    || dac.contains(&ball.as_str())
                    || board::LEDS.contains(&ball.as_str())
                    || ball == board::UART_TX,
                "unexpected output {ball}"
            );
        }
        let attached = Attached::new(&used);
        assert_eq!(attached.balls(), dac);
        assert!(attached.input_balls().is_empty());
        assert_eq!(attached.horizon(), None);
        let pin = |b: &str| Reference::Pin(b.into());
        let refs: Vec<_> = dac
            .iter()
            .map(|b| pin(b))
            .chain([pin(board::UART_TX)])
            .collect();
        let compiled = compile::compile(
            device,
            Request {
                package: board::PACKAGE.into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: vec![(pin(board::CLOCK), Logic::Zero)],
                probes: refs.clone(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
        .unwrap();
        let probes: Vec<_> = refs.iter().map(|r| compiled.probes[&r.label()]).collect();
        let mut fast = FastSim::new(&compiled.kernel).unwrap();
        fast.clock(
            compiled.drivers[&pin(board::CLOCK).label()],
            Time::from_ps(40_000),
        )
        .unwrap();
        fast.trace(&probes[..12]).unwrap();
        assert_eq!(fast.now().unwrap(), Time::ZERO);
        Self {
            fast,
            attached,
            tx: probes[12],
            uart: Receiver::new(board::BAUD),
        }
    }
    fn advance(&mut self, mut edges: u64, chunk: u64, mut observe: impl FnMut(&Trace)) -> String {
        let mut text = String::new();
        while edges > 0 {
            let n = chunk.min(edges);
            let (start, denominator) = self.fast.now().unwrap().ratio();
            assert_eq!(denominator, 1, "integer-picosecond 25 MHz clock");
            let rows = self.fast.run_edges_sampled(n, &[self.tx]).unwrap();
            for (i, level) in rows.into_iter().enumerate() {
                let ps = start + (i as u64 + 1) * 20_000;
                if let Some(b) = self.uart.sample(ps / 1000, level == 1) {
                    text.push(b as char);
                }
            }
            let trace = self.fast.take_trace();
            observe(&trace);
            self.attached.feed(&trace);
            edges -= n;
        }
        assert_eq!(self.uart.framing_errors, 0);
        text
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn audio_tone_matches_its_pad_crc_and_unloaded_ladder() {
    let mut run = Run::new();
    let mut bytes = Vec::new();
    // First divider strobe is the 1000th rising edge, at 39.98 us.
    let mut next_sample = 39_980_000;
    let started = Instant::now();
    let text = run.advance(102_000_000, 8192, |trace| {
        for (&ps, row) in trace.times_ps.iter().zip(trace.rows.chunks_exact(12)) {
            if bytes.len() < 100_000 && ps >= next_sample {
                assert_eq!(ps, next_sample, "each 25 kHz output strobe is traced");
                let codes: [u8; 3] = std::array::from_fn(|channel| {
                    row[channel * 4..channel * 4 + 4].iter().enumerate().fold(
                        0,
                        |v, (bit, &level)| {
                            assert!(level <= 1);
                            v | (level << bit)
                        },
                    )
                });
                assert!((4..=12).contains(&codes[0]) && (4..=12).contains(&codes[1]));
                assert_eq!(codes[2], 8);
                bytes.extend_from_slice(&codes[..2]);
                next_sample += 40_000_000;
            }
        }
    });
    assert_eq!(text, "AUDIO 25000 F1BFDEEF\r\nAUDIO 25000 2FB7D9FB\r\n");
    assert_eq!(bytes.len(), 100_000);
    for (block, expected) in bytes.chunks_exact(50_000).zip([0xf1bf_deef, 0x2fb7_d9fb]) {
        assert_eq!(
            crc32(block),
            expected,
            "CRC over the actual pad codes, L then R"
        );
    }
    let s = run.attached.view().audio.unwrap();
    let voltage = |code: u8| {
        audio::ladder(std::array::from_fn(|i| (code >> i) & 1))
            .0
            .unwrap()
    };
    for (c, target) in s.channels[..2].iter().zip([440.0, 660.0]) {
        // Each crossing is quantized to a 40 us design sample. Averaging
        // almost a second makes 1 Hz a conservative allowance, not tuning.
        assert!((c.hz.unwrap() - target).abs() < 1.0, "{c:?}");
        assert!((c.min_volts.unwrap() - voltage(4)).abs() < 1e-12);
        assert!((c.max_volts.unwrap() - voltage(12)).abs() < 1e-12);
        assert!((c.vpp.unwrap() - (voltage(12) - voltage(4))).abs() < 1e-12);
        assert_eq!(c.driven, 15);
        assert_eq!(c.unknown, 0);
        assert!(c.transitions > 1000);
    }
    assert_eq!(s.channels[2].code, 8);
    assert_eq!(s.channels[2].volts, Some(voltage(8)));
    assert_eq!(s.channels[2].vpp, Some(0.0));
    assert_eq!(s.channels[2].hz, None);
    assert_eq!(s.channels[2].transitions, 0);
    assert_eq!(s.scope.len(), audio::SCOPE_SAMPLES);
    assert!(s
        .scope
        .iter()
        .all(|p| p.volts[2] == Some(voltage(8) as f32)));
    println!(
        "{}; modelled L {:.6} Hz R {:.6} Hz, Vpp {:.9} V; {:.0} cycles/s",
        text.trim_end(),
        s.channels[0].hz.unwrap(),
        s.channels[1].hz.unwrap(),
        s.channels[0].vpp.unwrap(),
        51_000_000.0 / started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn traced_audio_restores_and_replays_with_different_chunk_sizes() {
    let mut run = Run::new();
    assert!(run.advance(2_000_000, 4096, |_| {}).is_empty());
    let fast = run.fast.snapshot();
    let audio = run.attached.snapshot();
    let before = run.attached.view().audio.unwrap();
    assert!(run.advance(4_000_000, 8192, |_| {}).is_empty());
    let later = run.attached.view().audio.unwrap();
    run.fast.restore(&fast);
    run.attached.restore(&audio);
    run.uart = Receiver::new(board::BAUD);
    assert_eq!(run.attached.view().audio.unwrap(), before);
    assert!(run.advance(4_000_000, 257, |_| {}).is_empty());
    assert_eq!(run.attached.view().audio.unwrap(), later);
}
