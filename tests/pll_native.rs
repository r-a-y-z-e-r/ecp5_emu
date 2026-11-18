//! EHXPLLL against the board: the exact HC05 configuration that ran on the
//! ULX3S (docs/inventory/LFE5U-85F/hc05_pll_lock) must print the same UART
//! report in the emulator as it did on the board in all six loads.
mod board;
use board::{compile_board, decode_uart, pin, run_uart, BIT};
use ecp5_emu::logic::Logic;

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc05_configuration_prints_the_board_report() {
    let mut c = compile_board("hc05_pll_lock/hc05.config", &["L4", "D1"]);
    assert!(c.report.resources.iter().any(|r| r.family == "PLL"));
    // Lock + 3 x 65536 measurement cycles, then one full report line.
    let samples = run_uart(&mut c, (4 * 65_536 + 40 * 10 * BIT) as u64, "L4");
    // led[4] (D1) is the synchronized LOCK.
    assert_eq!(c.value(&pin("D1")).unwrap(), Logic::One);
    let text = decode_uart(&samples);
    let line = text
        .split("\r\n")
        .find(|l| l.starts_with("L="))
        .unwrap_or_else(|| panic!("no report line in {text:?}"));
    // The board printed this line in 6 of 6 loads.
    assert_eq!(line, "L=000002 P=000002 F=040000 D=00");
}
