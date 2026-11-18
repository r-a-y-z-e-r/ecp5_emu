//! `FastSim::trace`: the signals after every instant, against the same run
//! without a trace. Two routed designs: `ddr_pattern` (only the board clock,
//! four ODDRX1F pads and their complements) and `blinky_pll` (a PLL at
//! 100 MHz, so long runs take the schedule table). On each path (whole cycles
//! in the program, falling edges, the table, the host loop) the caller's
//! samples are byte-identical with and without a trace, the trace's times
//! increase, and its rows at board-clock edges are the caller's rows; after
//! `run_until`/`set_at` its last row is what `value` reads; and a run that
//! changes program every 97 cycles (`tune_for_checks`) traces the same as one
//! that keeps its program.
mod board;
mod common;
use ecp5_emu::{
    compile::{self, Board, Compiled, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, sync::Arc};

const PERIOD: u64 = 40_000;
/// The GPDI pairs of ddr_pattern: dp then dn.
const PADS: [&str; 8] = ["A16", "A14", "A12", "A17", "B16", "C14", "A13", "B18"];
const LEDS: [&str; 8] = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];

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
    println!("warm start: {n} reference cycles");
    (fast, n)
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
    for (name, balls) in [("ddr_pattern", &PADS[..]), ("blinky_pll", &LEDS[..])] {
        let (mut c, probes) = design(name, balls);
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
