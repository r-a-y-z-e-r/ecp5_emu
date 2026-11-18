//! Routed ULX3S DVI designs, including their own RGB CRC reports, through
//! the same pad trace the live runner can feed to attached peripherals.
mod board;
mod common;
use ecp5_emu::{
    board::{GPDI_DN, GPDI_DP, UART_TX},
    compile::{self, Board, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::video::{Frame, VideoDecoder},
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const PERIOD: u64 = 40_000;
const FRAME_PIXELS: u64 = 800 * 525;

fn design(name: &str, balls: &[&str]) -> (Compiled, Vec<SignalId>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = common::reference::build(name);
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
    let probes: Vec<_> = balls.iter().map(|b| board::pin(b)).collect();
    let c = compile::compile_on_board(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(board::pin("G2"), Logic::Zero)],
            probes: probes.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
        &Board::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: {e:#}"));
    let signals = probes.iter().map(|r| c.probes[&r.label()]).collect();
    (c, signals)
}

/// Retry on StartsUnknown after whole reference cycles, as in fast_mode's
/// take_over. Other refusal text is preserved in full.
fn take_over(c: &mut Compiled) -> (FastSim, u64) {
    let clock = c.drivers["pin:G2"];
    let mut n = 0;
    let mut fast = loop {
        match FastSim::new(&c.kernel) {
            Ok(fast) => break fast,
            Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && n < 1000 => {
                n += 1;
                c.kernel
                    .schedule(Time::from_ps((2 * n - 1) * PERIOD / 2), clock, Logic::One)
                    .unwrap();
                c.kernel
                    .schedule(Time::from_ps(n * PERIOD), clock, Logic::Zero)
                    .unwrap();
                c.kernel.run_until(Time::from_ps(n * PERIOD)).unwrap();
            }
            Err(e) => panic!("fast mode: {e:#}"),
        }
    };
    fast.clock(clock, Time::from_ps(PERIOD)).unwrap();
    println!("HDMI warm start: {n} reference cycles");
    (fast, n)
}

fn run(name: &str, moving: bool) {
    let balls: Vec<_> = GPDI_DP
        .into_iter()
        .chain(moving.then_some(UART_TX))
        .collect();
    let (mut c, probes) = design(name, &balls);
    let (mut fast, warm) = take_over(&mut c);
    fast.trace(&probes[..4]).unwrap();
    let mut decoder = VideoDecoder::new();
    let mut uart = Receiver::new(115_200);
    uart.sample(warm * 40, true);
    let mut text = Vec::new();
    let mut frames: Vec<Arc<Frame>> = Vec::new();
    let mut done = warm * 2;
    let mut decode_wall = Duration::ZERO;
    let mut trace_rows = 0;
    let start = Instant::now();
    while done < 2_600_000 {
        let chunk = (2_600_000 - done).min(16_384);
        let sampled = fast
            .run_edges_sampled(chunk, if moving { &probes[4..] } else { &[] })
            .unwrap();
        if moving {
            for (i, &v) in sampled.iter().enumerate() {
                text.extend(uart.sample((done + i as u64 + 1) * 20, v == 1));
            }
        }
        let trace = fast.take_trace();
        assert!(!trace.gap);
        assert!(trace.times_ps.windows(2).all(|t| t[0] < t[1]));
        trace_rows += trace.times_ps.len();
        let before = Instant::now();
        decoder.feed(&trace);
        decode_wall += before.elapsed();
        if let Some(frame) = decoder.last_frame() {
            if frames.last().is_none_or(|f| f.number != frame.number) {
                println!("{name}: frame {} CRC {:08X}, {}x{} / {}x{}, {:.0} Hz pixels, {:.5} Hz refresh, sync {}/{} {} / {}", frame.number,
                    frame.crc32, frame.width, frame.height, frame.timing.htotal, frame.timing.vtotal,
                    frame.timing.pixel_clock_hz, frame.timing.refresh_hz, frame.timing.hsync_width, frame.timing.vsync_width,
                    if frame.timing.hsync_positive { "+" } else { "-" }, if frame.timing.vsync_positive { "+" } else { "-" });
                frames.push(frame);
            }
        }
        done += chunk;
    }
    let seconds = start.elapsed().as_secs_f64();
    let rate = (done / 2 - warm) as f64 / seconds;
    println!("{name}: {:.0} emulated pixel clocks/s, {:.6} seconds/frame; decoder {:.2} ns/row ({trace_rows} rows)",
        rate, FRAME_PIXELS as f64/rate, decode_wall.as_nanos() as f64 / trace_rows as f64);
    println!("{name}: status {:?}", decoder.status().timing);
    assert!(
        frames.len() >= 2,
        "{} complete frames, status {:?}",
        frames.len(),
        decoder.status()
    );
    for frame in &frames {
        let t = &frame.timing;
        assert_eq!(
            (frame.width, frame.height, t.htotal, t.vtotal),
            (640, 480, 800, 525)
        );
        assert_eq!(t.pixel_clock_hz, 25e6);
        assert_eq!((t.hsync_width, t.vsync_width), (96, 2));
        assert!(!t.hsync_positive && !t.vsync_positive && t.regular);
    }
    assert_eq!(frames[1].number, frames[0].number + 1);
    if moving {
        let text = String::from_utf8(text).unwrap();
        let mut reports = BTreeMap::new();
        for line in text.lines() {
            println!("{name}: UART {line}");
            let (number, crc) = line.strip_prefix('F').unwrap().split_once(" C").unwrap();
            reports.insert(
                u64::from_str_radix(number, 16).unwrap(),
                u32::from_str_radix(crc, 16).unwrap(),
            );
        }
        assert_eq!(uart.framing_errors, 0);
        assert_ne!(frames[0].rgb, frames[1].rgb);
        assert_ne!(frames[0].crc32, frames[1].crc32);
        for frame in &frames {
            assert_eq!(
                reports.get(&frame.number),
                Some(&frame.crc32),
                "frame {} UART CRC",
                frame.number
            );
        }
    } else {
        let colors = [
            [255, 255, 255],
            [255, 255, 0],
            [0, 255, 255],
            [0, 255, 0],
            [255, 0, 255],
            [255, 0, 0],
            [0, 0, 255],
            [0, 0, 0],
        ];
        let expected: Vec<u8> = (0..480)
            .flat_map(|_| (0..640).flat_map(|x| colors[x / 80]))
            .collect();
        for frame in &frames {
            assert_eq!(frame.rgb, expected);
            assert_eq!(frame.crc32, 0x2eb122d9);
        }
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn hdmi_bars() {
    run("hdmi_bars", false);
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn hdmi_moving() {
    run("hdmi_moving", true);
}

fn consistency(fast: &mut FastSim, probes: &[SignalId], count: u64, every: bool, label: &str) {
    fast.trace(&[]).unwrap();
    let snap = fast.snapshot();
    let (n, d) = fast.now().unwrap().ratio();
    let start_ps = n / d;
    let plain = if every {
        fast.run_edges_sampled(count, probes)
    } else {
        fast.run_cycles_sampled(count, probes)
    }
    .unwrap();
    let plain_end = fast.now().unwrap();
    fast.restore(&snap);
    fast.trace(probes).unwrap();
    let traced = if every {
        fast.run_edges_sampled(count, probes)
    } else {
        fast.run_cycles_sampled(count, probes)
    }
    .unwrap();
    assert_eq!(traced, plain, "{label}: caller samples");
    assert_eq!(fast.now().unwrap(), plain_end);
    let trace = fast.take_trace();
    assert!(!trace.gap);
    assert!(
        trace.times_ps.windows(2).all(|t| t[0] < t[1]),
        "{label}: monotone trace time"
    );
    let mut k = 0;
    for (&ps, row) in trace
        .times_ps
        .iter()
        .zip(trace.rows.chunks_exact(probes.len()))
    {
        let wanted = if every {
            ps % 20_000 == 0
        } else {
            ps % PERIOD == 0
        };
        if wanted {
            assert_eq!(ps, start_ps + (k + 1) * if every { 20_000 } else { PERIOD });
            assert_eq!(
                row,
                &plain[k as usize * probes.len()..(k as usize + 1) * probes.len()],
                "{label}: trace at {ps}"
            );
            k += 1;
        }
    }
    assert_eq!(k, count);
    fast.restore(&snap);
    assert!(fast.take_trace().gap, "restore marks discontinuity");
    fast.trace(&[]).unwrap();
    println!(
        "trace consistency: {label}, {count} samples, {} instant rows: identical",
        trace.times_ps.len()
    );
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn trace_preserves_sampled_rows_in_all_paths() {
    let name = "hdmi_bars";
    {
        let balls: Vec<_> = GPDI_DP.into_iter().chain(GPDI_DN).collect();
        let (mut c, probes) = design(name, &balls);
        let (mut fast, _) = take_over(&mut c);
        fast.run_cycles_sampled(8, &[]).unwrap();
        consistency(
            &mut fast,
            &probes,
            8192,
            true,
            &format!("{name} whole cycles"),
        );
        consistency(
            &mut fast,
            &probes,
            4096,
            false,
            &format!("{name} falling edges"),
        );
        // Next falling edge prevents the single-clock whole-cycle shortcut;
        // the long call uses fast_schedule, even on ddr_pattern.
        fast.run_edges_sampled(1, &[]).unwrap();
        consistency(&mut fast, &probes, 8192, true, &format!("{name} table"));
        // Odd short calls keep both designs in the host loop.
        consistency(&mut fast, &probes, 97, true, &format!("{name} host"));
        // run_until and set_at also reach instant(). Replaying their trace
        // must give the same pads as direct reads after those instants.
        fast.trace(&probes).unwrap();
        let now = fast.now().unwrap();
        let (n, d) = now.ratio();
        let end = Time::from_ps(n / d + 123_000);
        fast.run_until(end).unwrap();
        fast.set_at(Time::from_ps(n / d + 124_000), &[]).unwrap();
        let trace = fast.take_trace();
        assert!(trace.times_ps.windows(2).all(|t| t[0] < t[1]));
        assert_eq!(trace.times_ps.last(), Some(&(n / d + 124_000)));
        let expected: Vec<_> = probes
            .iter()
            .map(|&p| u8::from(fast.value(p) == Some(Logic::One)))
            .collect();
        assert_eq!(&trace.rows[trace.rows.len() - probes.len()..], expected);
        let (mut reference, _) = take_over(&mut c);
        let (mut tuned, _) = take_over(&mut c);
        assert!(tuned.tune_for_checks(&c.kernel).unwrap() >= 2);
        reference.trace(&probes).unwrap();
        tuned.trace(&probes).unwrap();
        let plain = reference.run_edges_sampled(20_001, &probes).unwrap();
        let switched = tuned.run_edges_sampled(20_001, &probes).unwrap();
        assert_eq!(plain, switched, "{name}: tuning caller rows");
        let a = reference.take_trace();
        let b = tuned.take_trace();
        assert_eq!(a.times_ps, b.times_ps, "{name}: tuning trace times");
        assert_eq!(a.rows, b.rows, "{name}: tuning trace rows and inversions");
        println!("trace consistency: {name} tuning across alternate layouts: identical");
    }
}
