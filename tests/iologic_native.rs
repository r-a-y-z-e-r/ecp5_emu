//! IOLOGIC against the board: the exact HI02 configurations that ran on the
//! ULX3S (docs/inventory/LFE5U-85F/hi02_iologic) must print the same UART
//! report in the emulator as they did on the board in all loads.
mod board;
use board::{compile_board, decode_uart, run_uart, BIT};

fn report(config: &str, prefix: &str, cycles: u64) -> String {
    let mut c = compile_board(config, &["L4"]);
    assert!(c.report.counts.iologic_paths > 0);
    let text = decode_uart(&run_uart(&mut c, cycles, "L4"));
    text.split("\r\n")
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no report line in {text:?}"))
        .to_string()
}

/// ODDRX1F and IDDRX1F in steady state: order of D0/D1, latency and capture
/// edges.
#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hi02_ddr_configuration_prints_the_board_report() {
    // Report after 4096 cycles; one line is 87 characters.
    let line = report(
        "hi02_iologic/hi02.config",
        "O=",
        (4096 + 88 * 10 * BIT) as u64,
    );
    // The board printed this line in 4 of 4 loads.
    assert_eq!(
        line,
        "O=5555AAAA5555FFFF5555AAAA5555AAAA0000FFFF5555FFFF5555AAAA0000FFFF I=DADEC697732AFE04"
    );
}

/// The 6-bit samples of an HI02b line: {ODDR pad, Q0, Q1, OREG pad, TSREG pad,
/// IREG Q} for t = 0..127 and t = 1008..1263, first sample in the MSBs.
fn hi02b_samples(line: &str) -> Vec<[bool; 6]> {
    let nibbles = line[2..]
        .chars()
        .map(|c| c.to_digit(16).unwrap())
        .collect::<Vec<_>>();
    let bits = nibbles
        .iter()
        .flat_map(|n| (0..4).rev().map(move |b| n >> b & 1 == 1))
        .collect::<Vec<_>>();
    assert_eq!(bits.len(), 384 * 6);
    bits.chunks(6).map(|c| c.try_into().unwrap()).collect()
}

/// Runs an HI02b-style configuration and compares it with the board line in
/// `log`. The digital signals must match sample for sample. When the TSREG
/// releases its pad, the board still reads it high for up to `lag` samples
/// while the pad discharges through its pull-down and load (analog, not
/// modelled); `window_lag` samples at the start of the second window (index
/// 128) lie in such a tail of a release just before it.
fn compare_hi02b_style(config: &str, log: &str, lag: usize, window_lag: usize) {
    // Report after 4096 cycles; one line is 580 characters.
    let line = report(config, "S=", (4096 + 581 * 10 * BIT) as u64);
    let board = log
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("S="))
        .unwrap();
    let (emulated, observed) = (hi02b_samples(&line), hi02b_samples(board));
    for signal in [0, 1, 2, 3, 5] {
        for (n, (e, o)) in emulated.iter().zip(&observed).enumerate() {
            assert_eq!(e[signal], o[signal], "signal {signal} at sample {n}");
        }
    }
    let mut since_release = usize::MAX;
    for (n, (e, o)) in emulated.iter().zip(&observed).enumerate() {
        let released = n > 0 && n != 128 && emulated[n - 1][4] && !e[4];
        since_release = if released {
            0
        } else {
            since_release.saturating_add(1)
        };
        let lagging = since_release < lag || (128..128 + window_lag).contains(&n);
        if e[4] != o[4] {
            assert!(!e[4] && o[4] && lagging, "TSREG pad at sample {n}");
        }
    }
}

/// DDR start-up and asynchronous reset, and the IREG/OREG/TSREG registers, on
/// the left bank (LED pins).
#[test]
#[ignore = "requires the native image reader and a local Trellis database; slow (~14 min)"]
fn hi02b_startup_reset_configuration_prints_the_board_report() {
    // The LED pin reads high for 14 samples after a release; the second
    // window opens 8 samples after a release at t = 1000.
    compare_hi02b_style(
        "hi02_iologic/hi02b.config",
        include_str!("../docs/inventory/LFE5U-85F/hi02_iologic/hi02b_observations.log"),
        14,
        6,
    );
}

/// The same design on top-bank SIOLOGIC (ULX3S GPDI pins, nothing connected).
#[test]
#[ignore = "requires the native image reader and a local Trellis database; slow (~14 min)"]
fn hi02c_top_bank_configuration_prints_the_board_report() {
    // The unloaded HDMI pin reads high for 3 samples after a release.
    compare_hi02b_style(
        "hi02_iologic/hi02c.config",
        include_str!("../docs/inventory/LFE5U-85F/hi02_iologic/hi02c_observations.log"),
        3,
        0,
    );
}
