//! The edge-clock bridge against the board: HI06
//! (docs/inventory/LFE5U-85F/hi06_eclkbridge). ECLKBRIDGECS selects one of
//! two fabric-made edge clocks, a BRGECLKSYNC (ECLKSYNCB) with STOP follows,
//! then CLKDIVF /2 and an ODDRX2F that shows ECLKO on a pad. Two builds:
//! - `hi06.config`, as nextpnr writes it, without `ECLKBRIDGECS0.MODE`: SEL
//!   does nothing on the board; every experiment line fast mode prints is a
//!   board line.
//! - `hi06_mode.config`, the same with the mode set: the bridge switches.
//!   Both builds started ECLKO one ECLKI edge later than the emulator does
//!   (HI04: four of eight builds), and in this build every window repeats
//!   the divider's phase, so the comparison is per column: the clocks, SEL
//!   and ECLKO on the pad, glitches included, equal the board's in every
//!   experiment.
mod board;
use board::{compile_board, decode_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::{HashMap, HashSet};

/// Characters of a report line, without CR LF.
const LINE: usize = 388;

fn board_lines(observations: &str) -> HashSet<String> {
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

/// The report lines fast mode prints for `config`: two passes of 8 lines.
fn emulated(config: &str) -> Vec<String> {
    let c = compile_board(config, &["L4"]);
    assert_eq!(c.report.counts.eclk_bridges, 1);
    assert_eq!(c.report.counts.eclk_syncs, 1);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast.run_cycles(16_000_000, c.probes["pin:L4"]).unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    let lines: Vec<String> = decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE && l.starts_with('E'))
        .map(str::to_string)
        .collect();
    assert!(lines.len() >= 15, "{} lines", lines.len());
    lines
}

/// Columns {c0, c1, sel, pad} of the 256 samples of a line.
fn clock_columns(line: &str) -> Vec<u8> {
    let hex = &line[4..];
    let bits: Vec<u8> = hex
        .chars()
        .flat_map(|c| {
            let v = c.to_digit(16).unwrap() as u8;
            (0..4).rev().map(move |k| (v >> k) & 1)
        })
        .collect();
    bits.chunks(6).flat_map(|s| s[..4].to_vec()).collect()
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi06_bridge_as_nextpnr_writes_it_prints_the_board_lines() {
    let board = board_lines(include_str!(
        "../docs/inventory/LFE5U-85F/hi06_eclkbridge/observations_nextpnr.log"
    ));
    for line in emulated("hi06_eclkbridge/hi06.config") {
        if line.starts_with("E00") {
            continue;
        }
        assert!(board.contains(&line), "{} is not a board line", &line[..3]);
    }
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi06_bridge_with_its_mode_switches_as_on_the_board() {
    let board: HashMap<String, String> = board_lines(include_str!(
        "../docs/inventory/LFE5U-85F/hi06_eclkbridge/observations_mode.log"
    ))
    .into_iter()
    .map(|l| (l[..3].to_string(), l))
    .collect();
    for line in emulated("hi06_eclkbridge/hi06_mode.config") {
        if line.starts_with("E00") {
            continue;
        }
        assert_eq!(
            clock_columns(&line),
            clock_columns(&board[&line[..3]]),
            "{}",
            &line[..3]
        );
    }
}
