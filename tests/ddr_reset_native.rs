//! RST and GSR on the X1 DDR paths against the board: the HI09 configuration
//! (docs/inventory/LFE5U-85F/hi09_ddr_reset) runs ODDRX1F and IDDRX1F, each
//! with GSR=ENABLED and GSR=DISABLED, under a pseudo-random RST and user GSR
//! that change between sclk edges, and prints 256 samples a line: the start
//! (E00), then seven windows, over and over.
//!
//! Fast mode runs two rounds of eight lines; every line it prints is a line
//! the board printed, except the first round's E01, which the board prints
//! before the reader opens the port (as in HI05). The event kernel runs the
//! start: its E00 is the board's.
mod board;
use board::{compile_board, decode_uart, run_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::HashSet;

/// Characters of a report line, without CR LF.
const LINE: usize = 772;
/// A round: 8 lines of 774 characters, each 10 bits of 217 cycles and one
/// cycle to load, and 7 windows of 256 samples.
const ROUND: u64 = 8 * 774 * (10 * 217 + 1) + 7 * 257;

fn board_lines() -> HashSet<String> {
    let observations = include_str!("../docs/inventory/LFE5U-85F/hi09_ddr_reset/observations.log");
    let mut out = HashSet::new();
    for line in observations.lines() {
        if let Some((_, list)) = line.split_once("distinct=") {
            for item in list.split('\'').skip(1).step_by(2) {
                if item.len() == LINE {
                    out.insert(item.to_string());
                }
            }
        }
    }
    out
}

fn lines(samples: &[bool]) -> Vec<String> {
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().copied())
        .collect();
    decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE && l.starts_with('E'))
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; ~2 minutes"]
fn hi09_prints_the_board_lines_in_fast_mode() {
    let board = board_lines();
    let c = compile_board("hi09_ddr_reset/hi09.config", &["L4"]);
    assert_eq!(c.report.counts.iologic_paths, 4);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast
        .run_cycles(2 * ROUND + 2_000, c.probes["pin:L4"])
        .unwrap();
    let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
    let lines = lines(&samples);
    assert!(lines.len() >= 16, "{} lines", lines.len());
    // The first round's E01 comes within 0.15 s of the load, before run.py
    // opens the port, so the board record has it only if it repeats (HI05).
    let missing: Vec<usize> = (0..16).filter(|&n| !board.contains(&lines[n])).collect();
    assert_eq!(missing, [1], "lines that are not board lines");
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database; several minutes"]
fn hi09_start_line_in_the_event_kernel_is_the_boards() {
    let board = board_lines();
    let mut c = compile_board("hi09_ddr_reset/hi09.config", &["L4"]);
    // 512 cycles to the first line, then 774 characters of 2 171 cycles,
    // and the last stop bit.
    let samples = run_uart(&mut c, 512 + 774 * 2_171 + 3_000, "L4");
    let lines = lines(&samples);
    assert!(
        !lines.is_empty() && lines[0].starts_with("E00"),
        "{lines:?}"
    );
    let start = board.iter().find(|l| l.starts_with("E00")).unwrap();
    // The first differing sample, as 12-bit words, if any.
    let words = |l: &str| -> Vec<u16> {
        let hex: Vec<u32> = l[4..].chars().map(|c| c.to_digit(16).unwrap()).collect();
        hex.chunks(3)
            .map(|w| (w[0] << 8 | w[1] << 4 | w[2]) as u16)
            .collect()
    };
    let (ours, theirs) = (words(&lines[0]), words(start));
    let first = (0..256).find(|&i| ours[i] != theirs[i]);
    assert_eq!(
        first,
        None,
        "E00 differs; kernel {:012b} board {:012b} (sclk rst gsr_n din d0 d1 ob1 ob0 q0e q1e q0d q1d)",
        first.map_or(0, |i| ours[i]),
        first.map_or(0, |i| theirs[i])
    );
}
