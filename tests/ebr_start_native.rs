//! The EBR output latch after configuration (HB02,
//! docs/inventory/LFE5U-85F/hb02_ebr_start): eight DP16KD whose ports never
//! read, in every width, both REGMODEs, both GSR settings and both
//! RESETMODEs, read 0 from the first clock on; a ninth that reads shows its
//! word from the first edge. The board recorded its first 16 clocks.
mod board;
use board::{compile_board, decode_uart, run_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};

/// Characters of a line, without CR LF: "B" and 448 hex digits.
const LINE: usize = 449;
/// Cycles of two lines, after the 16 recorded clocks.
const CYCLES: u64 = 16 + 2 * 451 * (10 * 217 + 1) + 3_000;

fn board_line() -> String {
    let observations = include_str!("../docs/inventory/LFE5U-85F/hb02_ebr_start/observations.log");
    let lines: std::collections::HashSet<&str> = observations
        .lines()
        .filter_map(|l| l.split_once("distinct="))
        .flat_map(|(_, list)| list.split('\'').skip(1).step_by(2))
        .filter(|i| i.len() == LINE)
        .collect();
    assert_eq!(lines.len(), 1, "the board gave one line");
    lines.into_iter().next().unwrap().to_string()
}

fn lines(samples: &[bool]) -> Vec<String> {
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().copied())
        .collect();
    decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE)
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; about a minute"]
fn unread_ebr_latches_start_at_zero_in_fast_mode() {
    let c = compile_board("hb02_ebr_start/hb02.config", &["L4"]);
    // The latches start known, so fast mode starts at once.
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast.run_cycles(CYCLES, c.probes["pin:L4"]).unwrap();
    let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
    let got = lines(&samples);
    assert!(!got.is_empty(), "no line");
    assert!(
        got.iter().all(|l| *l == board_line()),
        "a line is not the board's"
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database; minutes"]
fn unread_ebr_latches_start_at_zero_in_the_event_kernel() {
    let mut c = compile_board("hb02_ebr_start/hb02.config", &["L4"]);
    let got = lines(&run_uart(&mut c, CYCLES, "L4"));
    assert!(!got.is_empty(), "no line");
    assert!(
        got.iter().all(|l| *l == board_line()),
        "a line is not the board's"
    );
}
