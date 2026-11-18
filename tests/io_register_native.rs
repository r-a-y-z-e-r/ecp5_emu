//! IO registers against the board: the HI05 configuration
//! (docs/inventory/LFE5U-85F/hi05_ioreg) drives eight IO registers of the
//! kinds yosys maps IFS1P3{B,D,I,J}X and OFS1P3{B,D,I,J}X to (LiteX's SDR IO)
//! with pseudo-random data, clock enable, set/reset and a user GSR, and
//! prints 256 samples a line: the start (E00), then seven windows.
//!
//! Fast mode runs two rounds of eight lines. Every line it prints is a line
//! the board printed, except the first round's E01: the board prints it
//! within 0.15 s of the load, before the reader opens the port. The event
//! kernel is too slow for 1.7 M cycles a line; tests/io_register.rs checks
//! it against fast mode and the same rules on a synthetic plan.
mod board;
use board::{compile_board, decode_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::HashSet;

/// Characters of a report line, without CR LF.
const LINE: usize = 772;

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; ~2 minutes"]
fn hi05_prints_the_board_lines_in_fast_mode() {
    let observations = include_str!("../docs/inventory/LFE5U-85F/hi05_ioreg/observations.log");
    let mut board = HashSet::new();
    for line in observations.lines() {
        if let Some((_, list)) = line.split_once("distinct=") {
            for item in list.split('\'').skip(1).step_by(2) {
                if item.len() == LINE {
                    board.insert(item.to_string());
                }
            }
        }
    }
    let c = compile_board("hi05_ioreg/hi05.config", &["L4"]);
    assert_eq!(c.report.counts.iologic_paths, 8);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    // A round: 8 lines of 774 characters, each 10 bits of 217 cycles and
    // one cycle to load, and 7 windows of 256 samples.
    let round = 8 * 774 * (10 * 217 + 1) + 7 * 257;
    let samples = fast
        .run_cycles(2 * round + 2_000, c.probes["pin:L4"])
        .unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    let lines: Vec<String> = decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE && l.starts_with('E'))
        .map(str::to_string)
        .collect();
    assert!(lines.len() >= 16, "{} lines", lines.len());
    for (n, l) in lines.iter().take(16).enumerate() {
        if n == 1 {
            continue;
        }
        assert!(
            board.contains(l),
            "line {n} ({}) is not a board line",
            &l[..3]
        );
    }
}
