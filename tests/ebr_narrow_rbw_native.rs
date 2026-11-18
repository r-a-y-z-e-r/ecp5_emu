//! HB11 (docs/inventory/LFE5U-85F/hb11_ebr_narrow_rbw): DP16KD
//! READBEFOREWRITE at the narrow widths 1, 2 and 4.
//!
//! Eight DP16KD run on the 25 MHz clock with NOREG and OUTREG, widths 1/1,
//! 2/2, 4/4, 1/4, 9/9 and a NORMAL/WRITETHROUGH control. Both executors run
//! hb11s from configuration; fast mode also runs the free-running hb11.
mod board;
use board::{compile_board, decode_uart, run_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::HashSet;

/// Characters of a line, without CR LF: "R", 16 hex of win_lfsr, and 128 * 14 hex of samples.
const LINE: usize = 1 + 16 + 1_792;
/// Cycles of one 128-sample window and its UART line (LINE + CR LF characters).
const WINDOW: u64 = 128 + (LINE as u64 + 2) * (10 * 217 + 1);

fn board_lines(observations: &str) -> HashSet<&str> {
    let mut out = HashSet::new();
    for line in observations.lines() {
        if let Some((_, list)) = line.split_once("distinct=") {
            out.extend(
                list.split('\'')
                    .skip(1)
                    .step_by(2)
                    .filter(|i| i.len() == LINE && i.starts_with('R')),
            );
        }
    }
    out
}

fn board_s_line() -> String {
    let lines = board_lines(include_str!(
        "../docs/inventory/LFE5U-85F/hb11_ebr_narrow_rbw/observations_s.log"
    ));
    assert_eq!(lines.len(), 1, "hb11s gave one distinct full window");
    lines.into_iter().next().unwrap().to_string()
}

fn lines(samples: &[bool]) -> Vec<String> {
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().copied())
        .collect();
    decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE && l.starts_with('R'))
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb11s_first_window_in_the_event_kernel_is_the_boards() {
    let mut c = compile_board("hb11_ebr_narrow_rbw/hb11s.config", &["L4"]);
    assert_eq!(c.report.counts.ebr_blocks, 8);
    let got = lines(&run_uart(&mut c, WINDOW + 3_000, "L4"));
    assert_eq!(got.len(), 1, "{} lines", got.len());
    assert_eq!(got[0], board_s_line(), "first window differs from board");
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb11s_first_window_in_fast_mode_is_the_boards() {
    let c = compile_board("hb11_ebr_narrow_rbw/hb11s.config", &["L4"]);
    assert_eq!(c.report.counts.ebr_blocks, 8);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast
        .run_cycles(2 * WINDOW + 3_000, c.probes["pin:L4"])
        .unwrap();
    let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
    let got = lines(&samples);
    assert!(got.len() >= 2, "{} lines", got.len());
    let want = board_s_line();
    assert!(
        got.iter().all(|l| *l == want),
        "a window differs from the board's"
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb11_windows_in_fast_mode_are_the_boards() {
    let board = board_lines(include_str!(
        "../docs/inventory/LFE5U-85F/hb11_ebr_narrow_rbw/observations.log"
    ));
    let c = compile_board("hb11_ebr_narrow_rbw/hb11.config", &["L4"]);
    assert_eq!(c.report.counts.ebr_blocks, 8);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast
        .run_cycles(4 * WINDOW + 3_000, c.probes["pin:L4"])
        .unwrap();
    let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
    let got = lines(&samples);
    assert!(got.len() >= 4, "{} lines", got.len());
    // The serial reader opens while window 0 is on its way; hb11s recorded window 0.
    assert_eq!(got[0], board_s_line(), "window 0 differs from hb11s");
    let missing: Vec<usize> = (1..4)
        .filter(|&n| !board.contains(got[n].as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "windows {missing:?} are not in observations.log"
    );
}
