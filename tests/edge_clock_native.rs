//! The edge-clock controls against the board: the HI04 and HI04b
//! configurations (docs/inventory/LFE5U-85F/hi04_eclk) run ECLKSYNCB STOP,
//! CLKDIVF /2 and /3.5, RST and ALIGNWD experiments and print one line of
//! 256 samples each, over and over.
//!
//! Fast mode runs two passes of each design. Every line it prints is one the
//! board printed, except E01 and E02 of the first pass: on the board those
//! come out within 70 ms of the load, before the reader opens the port, so
//! the recorded set has only the later passes' lines (whose dividers start
//! from another phase). The event kernel is too slow for these designs
//! (~7 kHz); tests/edge_clock.rs checks it against fast mode and the same
//! rules on a synthetic plan.
mod board;
use board::{compile_board, decode_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::HashSet;

/// Characters of a report line, without CR LF.
const LINE: usize = 388;

/// The distinct report lines that `observations` recorded.
fn board_lines(observations: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in observations.lines() {
        let Some((_, list)) = line.split_once("distinct=") else {
            continue;
        };
        // A Python list of quoted strings.
        for item in list.split('\'').skip(1).step_by(2) {
            if item.len() == LINE && item.starts_with('E') {
                out.insert(item.to_string());
            }
        }
    }
    out
}

/// The report lines that fast mode prints for `config` in `cycles`, by pass.
fn emulated(config: &str, cycles: u64) -> Vec<Vec<String>> {
    let c = compile_board(config, &["L4"]);
    assert_eq!(c.report.counts.eclk_syncs, 2);
    assert_eq!(c.report.counts.clock_dividers, 2);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast.run_cycles(cycles, c.probes["pin:L4"]).unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    let mut passes: Vec<Vec<String>> = Vec::new();
    for l in decode_uart(&line).split("\r\n") {
        if l.len() != LINE || !l.starts_with('E') {
            continue;
        }
        if l.starts_with("E00 ") || passes.is_empty() {
            passes.push(Vec::new());
        }
        passes.last_mut().unwrap().push(l.to_string());
    }
    passes
}

fn check(config: &str, observations: &str, lines: usize, cycles: u64) {
    let board = board_lines(observations);
    let passes = emulated(config, cycles);
    assert!(passes.len() >= 2, "{} passes", passes.len());
    for (p, pass) in passes.iter().enumerate().take(2) {
        assert_eq!(pass.len(), lines, "pass {p}");
        for line in pass {
            let id = &line[..3];
            if p == 0 && (id == "E01" || id == "E02") {
                continue;
            }
            assert!(board.contains(line), "pass {p}: {id} is not a board line");
        }
    }
    // Every experiment the board printed, the emulator printed too.
    let ids: HashSet<&str> = board.iter().map(|l| &l[..3]).collect();
    let printed: HashSet<&str> = passes[1].iter().map(|l| &l[..3]).collect();
    assert_eq!(ids, printed);
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; ~1 minute"]
fn hi04_prints_the_board_lines_in_fast_mode() {
    // 23 lines of 390 characters a pass, then a 0.1 s pause: 22 M cycles.
    check(
        "hi04_eclk/hi04.config",
        include_str!("../docs/inventory/LFE5U-85F/hi04_eclk/observations_hi04.log"),
        23,
        46_000_000,
    );
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; ~2 minutes"]
fn hi04b_prints_the_board_lines_in_fast_mode() {
    // 39 lines a pass and the pause: 35.5 M cycles.
    check(
        "hi04_eclk/hi04b.config",
        include_str!("../docs/inventory/LFE5U-85F/hi04_eclk/observations_hi04b_seed1.log"),
        39,
        73_000_000,
    );
}
