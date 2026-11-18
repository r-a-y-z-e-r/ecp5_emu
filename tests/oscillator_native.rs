//! OSCG against the board: the HC06 configurations that ran on the ULX3S
//! (docs/inventory/LFE5U-85F/hc06_osc) report the oscillator's start-up and
//! its edges per 65 536 reference cycles. With the board's measured base
//! frequency the emulator must land within the board's own spread.
mod board;
use board::{compile_on, decode_uart, run_uart, BIT};
use ecp5_emu::compile::Board;

/// The base frequency HC06 measured on this ULX3S.
const BOARD_HZ: u64 = 311_800_000;

/// E, S, W and F of the first report line.
fn first_report(config: &str, board: Board) -> [u64; 4] {
    let mut c = compile_on(config, &["L4"], board);
    assert!(c.report.resources.iter().any(|r| r.family == "OSCG"));
    // The first window is sampled at 0x18000 cycles; a line is 35 characters.
    let text = decode_uart(&run_uart(&mut c, (0x18000 + 36 * 10 * BIT) as u64, "L4"));
    let line = text
        .split("\r\n")
        .find(|l| l.starts_with("E="))
        .unwrap_or_else(|| panic!("no report line in {text:?}"));
    let fields: Vec<u64> = line
        .split(' ')
        .map(|f| u64::from_str_radix(&f[2..], 16).unwrap())
        .collect();
    fields.try_into().unwrap()
}

/// Mean F of every load in an observation log.
fn board_means(log: &str) -> Vec<f64> {
    log.lines()
        .filter_map(|l| l.split("mean=").nth(1))
        .map(|m| m.split(' ').next().unwrap().parse().unwrap())
        .collect()
}

fn check(config: &str, log: &str, divider: u64, max_e: u64) {
    let [e, s, w, f] = first_report(
        config,
        Board {
            oscillator_hz: Some(BOARD_HZ),
            ..Board::default()
        },
    );
    assert_eq!(w, 1);
    assert!(e >= 2 && e <= max_e, "start-up E={e}");
    let means = board_means(log);
    assert!(!means.is_empty());
    for mean in means {
        let error = (f as f64 - mean).abs() / mean;
        assert!(error < 1e-3, "F={f} vs board mean {mean}");
        assert!((s as f64 - mean).abs() / mean < 1e-3, "S={s} vs {mean}");
    }
    // The nominal profile is plain 310 MHz over the divider.
    let [_, _, _, nominal] = first_report(config, Board::default());
    let expected = 310e6 / divider as f64 * 65_536.0 / 25e6;
    assert!(
        (nominal as f64 - expected).abs() <= 1.0,
        "{nominal} vs {expected}"
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc06_div16_matches_the_board_frequency() {
    let log = include_str!("../docs/inventory/LFE5U-85F/hc06_osc/observations_div16.log");
    check("hc06_osc/hc06_16.config", log, 16, 4);
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc06_div127_divides_by_128_like_the_board() {
    // nextpnr writes DIV=127 and DIV=128 with the same bits; the chip divides by 128.
    let log = include_str!("../docs/inventory/LFE5U-85F/hc06_osc/observations_div127.log");
    check("hc06_osc/hc06_127.config", log, 128, 13);
}
