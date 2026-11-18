//! EHXPLLL RST and STDBY against the board: the HC07 configurations that ran
//! on the ULX3S (docs/inventory/LFE5U-85F/hc07_pll_reset) run in the event
//! kernel, and each event they report is compared with the board's events of
//! the same pulse width.
mod board;
use board::{compile_board, decode_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time, logic::Logic};
use std::collections::BTreeMap;
use std::time::Instant;

const PLLS: [&str; 4] = ["pa", "pb", "pc", "pd"];
const FIELDS: [(&str, usize); 10] = [
    ("drop", 5),
    ("during", 4),
    ("level", 1),
    ("first", 5),
    ("stable", 5),
    ("lock", 5),
    ("bmin", 3),
    ("bmax", 3),
    ("ph0", 2),
    ("ph1", 2),
];

const RAMP: [&str; 12] = [
    "ramp0", "ramp1", "ramp2", "ramp3", "ramp4", "ramp5", "ramp6", "ramp7", "ramp8", "ramp9",
    "ramp10", "ramp11",
];

#[derive(Debug)]
struct Event {
    width: u64,
    /// Per PLL: the fields, and "lost" = (ph1 - ph0) mod 64.
    plls: Vec<BTreeMap<&'static str, u64>>,
}

/// One report line; `per` is 35 (hc06) or 72 (hc06b) hex per PLL.
fn parse(line: &str, per: usize) -> Option<Event> {
    if line.len() != 1 + 5 + per * 4 || !line.starts_with('H') {
        return None;
    }
    let hex = |at: usize, n: usize| u64::from_str_radix(&line[at..at + n], 16).ok();
    let width = hex(1, 5)?;
    let mut plls = Vec::new();
    for p in 0..4 {
        let mut at = 6 + p * per;
        let mut fields = BTreeMap::new();
        for (name, n) in FIELDS {
            fields.insert(name, hex(at, n)?);
            at += n;
        }
        fields.insert("lost", (fields["ph1"] + 64 - fields["ph0"]) % 64);
        if per == 72 {
            fields.insert("lrise", hex(at, 1)?);
            for (w, name) in RAMP.into_iter().enumerate() {
                fields.insert(name, hex(at + 1 + 3 * w, 3)?);
            }
        }
        plls.push(fields);
    }
    Some(Event { width, plls })
}

/// The board's events: every distinct line of every load in the record.
fn board_events(record: &str, per: usize) -> Vec<Event> {
    let path = format!(
        "{}/docs/inventory/LFE5U-85F/hc07_pll_reset/{record}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap();
    text.split('\'')
        .filter_map(|token| parse(token, per))
        .collect()
}

fn emulated(config: &str, cycles: u64, per: usize, count: usize) -> Vec<Event> {
    let started = Instant::now();
    let mut c = compile_board(&format!("hc07_pll_reset/{config}"), &["L4"]);
    assert_eq!(
        c.report
            .resources
            .iter()
            .filter(|r| r.family == "PLL")
            .count(),
        4
    );
    // As board::run_uart, but stop after the required complete reports. UART
    // shifts dominate these designs, so do not run an extra event for margin.
    let reference = c.drivers["pin:G2"];
    let uart = c.probes["pin:L4"];
    let mut samples = Vec::new();
    let mut events = Vec::new();
    let mut ran = 0;
    for cycle in 1..=cycles {
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, reference, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        c.kernel.schedule(fall, reference, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        samples.push(c.kernel.value(uart).unwrap().bit().expect("binary UART"));
        ran = cycle;
        if cycle % 16_384 == 0 || cycle == cycles {
            events = decode_uart(&samples)
                .split("\r\n")
                .filter_map(|line| parse(line, per))
                .collect();
            if events.len() >= count {
                break;
            }
        }
        if cycle % 250_000 == 0 {
            eprintln!(
                "{config}: {cycle} cycles, {} complete events, {:.2?}",
                events.len(),
                started.elapsed()
            );
        }
    }
    eprintln!(
        "{config}: {} events in {ran} reference cycles, {:.2?} (planning + replay)",
        events.len(),
        started.elapsed()
    );
    events
}

/// The board's range of `field` for PLL `p` at this width.
fn range(board: &[Event], width: u64, p: usize, field: &str) -> (u64, u64) {
    let values: Vec<u64> = board
        .iter()
        .filter(|e| e.width == width)
        .map(|e| e.plls[p][field])
        .collect();
    assert!(
        !values.is_empty(),
        "the board has no event of width {width}"
    );
    (*values.iter().min().unwrap(), *values.iter().max().unwrap())
}

/// Enabled controls and reference dividers in the two board configurations.
fn enabled(variant_b: bool, p: usize) -> bool {
    variant_b || p == 1 || p == 3
}

fn clki_div(variant_b: bool, p: usize) -> u64 {
    if variant_b && p >= 2 {
        5
    } else {
        1
    }
}

/// Every enabled PLL of this CLKI_DIV, across both records at this width.
fn lock_envelope(records: &[Vec<Event>; 2], width: u64, divider: u64) -> (u64, u64) {
    let mut lo = u64::MAX;
    let mut hi = 0;
    for (variant, board) in records.iter().enumerate() {
        for p in 0..4 {
            if enabled(variant == 1, p) && clki_div(variant == 1, p) == divider {
                let (min, max) = range(board, width, p, "lock");
                lo = lo.min(min);
                hi = hi.max(max);
            }
        }
    }
    assert!(lo <= hi);
    (lo, hi)
}

fn replay(variant_b: bool, cycles: u64, count: usize) {
    let records = [
        board_events("observations.log", 35),
        board_events("observations_b.log", 72),
    ];
    assert_eq!(records[0].len(), 3552);
    assert_eq!(records[1].len(), 2410);
    let (config, per) = if variant_b {
        ("hc06b.config", 72)
    } else {
        ("hc06.config", 35)
    };
    // hc06 latches ph1 at the same edge as its payload: the first report
    // contains its initial zero, not an earlier event's phase. That startup
    // phase sample has no match in the board record.
    let primer = usize::from(!variant_b);
    let mut events = emulated(config, cycles, per, count + primer);
    assert!(
        events.len() >= count + primer,
        "{config}: incomplete replay"
    );
    if primer != 0 {
        eprintln!(
            "{config}: priming previous-event ph1 with the first report (W={})",
            events[0].width
        );
        events.remove(0);
    }
    assert!(
        events.len() >= count,
        "{config}: need {count} events, got {}",
        events.len()
    );
    let board = &records[usize::from(variant_b)];
    let mut failures = Vec::new();
    for (n, e) in events.iter().enumerate() {
        for (p, name) in PLLS.iter().enumerate() {
            let active = enabled(variant_b, p);
            let short = e.width <= 8;
            for field in ["drop", "during", "first", "lock", "bmin", "bmax", "lost"] {
                let (lo, hi) = range(board, e.width, p, field);
                let value = e.plls[p][field];
                let bounds = if !active {
                    assert_eq!(
                        lo, hi,
                        "{config} W={} {name} {field}: disabled board value varies",
                        e.width
                    );
                    Some((lo as f64, hi as f64))
                } else if matches!(field, "drop" | "during") {
                    assert_eq!((lo, hi), if field == "drop" { (2, 2) } else { (0, 0) });
                    Some((lo as f64, hi as f64))
                } else if short {
                    let tolerance = match field {
                        "first" | "lock" => 0,
                        "lost" => 4,
                        "bmin" => 3,
                        _ => continue,
                    };
                    Some((lo.saturating_sub(tolerance) as f64, (hi + tolerance) as f64))
                } else if field == "first" {
                    Some((lo.saturating_sub(3) as f64, (hi + 3) as f64))
                } else if field == "lock" {
                    let divider = clki_div(variant_b, p);
                    let base = if divider == 1 { 98_311 } else { 491_546 };
                    let (low, high) = lock_envelope(&records, e.width, divider);
                    Some((
                        low as f64 - 0.15 * (low - base) as f64,
                        high as f64 + 0.15 * (high - base) as f64,
                    ))
                } else {
                    None
                };
                let rule = bounds.map_or_else(
                    || "not asserted (ramp)".to_string(),
                    |(low, high)| format!("allowed {low:.2}..{high:.2}"),
                );
                eprintln!("{config} event {} W={} {name} {field}: emulator {value}, board {lo}..{hi}; {rule}", n + 1, e.width);
                if let Some((low, high)) = bounds {
                    if (value as f64) < low || (value as f64) > high {
                        failures.push(format!("W={} {name} {field}: {value} outside {low}..{high}; offsets from board {:+}..{:+}", e.width, value as i64 - hi as i64, value as i64 - lo as i64));
                    }
                }
            }
            if active && !short {
                for field in ["stable", "level"].into_iter().chain(
                    if variant_b { RAMP.as_slice() } else { &[] }
                        .iter()
                        .copied(),
                ) {
                    let (lo, hi) = range(board, e.width, p, field);
                    eprintln!("{config} event {} W={} {name} {field}: emulator {}, board {lo}..{hi}; not asserted (ramp)", n + 1, e.width, e.plls[p][field]);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{config}:\n{}", failures.join("\n"));
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc06_events_are_the_boards() {
    replay(false, 3_000_000, 3);
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc06b_events_are_the_boards() {
    replay(true, 2_700_000, 2);
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc06_fast_mode_refuses_live_rst_stdby() {
    let started = Instant::now();
    let c = compile_board("hc07_pll_reset/hc06.config", &["L4"]);
    let error = FastSim::new(&c.kernel)
        .err()
        .expect("fast mode must refuse live PLL controls");
    let message = format!("{error:#}");
    assert!(
        message.contains("fast mode does not run pll:")
            && message.contains("with RST/STDBY enabled (HC07); the event kernel does"),
        "{message}"
    );
    eprintln!("hc06 fast refusal: {message}; {:.2?}", started.elapsed());
}
