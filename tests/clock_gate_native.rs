//! DCCA (a clock gate with a dynamic CE) against the board: the HC01
//! configuration (docs/inventory/LFE5U-85F/hc01_dcca) must print, in the event
//! kernel and in fast mode, the twelve lines the ULX3S printed: for CE
//! stopping and starting at each phase of CLKI, every rising and falling
//! edge of CLKO, cycle by cycle.
mod board;
use board::{compile_board, decode_uart, run_uart, BIT, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};

/// Characters of one report line, CR LF included.
const LINE: usize = 38;
/// Cycles of one pass of the twelve experiments, with some to spare.
const PASS: usize = 12 * (64 + 8 + 48 + 2 + LINE * 10 * BIT);

fn board() -> Vec<String> {
    include_str!("../docs/inventory/LFE5U-85F/hc01_dcca/pass.txt")
        .lines()
        .map(str::to_string)
        .collect()
}

/// The twelve distinct report lines in `text`, sorted as pass.txt is.
fn lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split("\r\n")
        .filter(|l| l.len() == LINE - 2 && l.starts_with('E'))
        .map(str::to_string)
        .collect();
    lines.sort();
    lines.dedup();
    lines
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc01_prints_the_board_lines_in_the_event_kernel() {
    let mut c = compile_board("hc01_dcca/hc01.config", &["L4"]);
    assert_eq!(c.report.counts.dcc_gates, 1);
    let text = decode_uart(&run_uart(&mut c, (PASS + 2 * LINE * 10 * BIT) as u64, "L4"));
    assert_eq!(lines(&text), board(), "{text}");
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hc01_prints_the_board_lines_in_fast_mode() {
    let c = compile_board("hc01_dcca/hc01.config", &["L4"]);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let tx = c.probes["pin:L4"];
    let samples = fast
        .run_cycles((PASS + 2 * LINE * 10 * BIT) as u64, tx)
        .unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    assert_eq!(lines(&decode_uart(&line)), board());
}
