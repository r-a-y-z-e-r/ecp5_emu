//! PDPW16KD with a 36-bit read against the board: the HB01 configuration
//! (docs/inventory/LFE5U-85F/hb01_pdp36) must print, in the emulator, the
//! lines the ULX3S printed for every step of its program.
mod board;
use board::{compile_board, decode_uart, run_uart, BIT};

/// Steps of the HB01 program (program.json).
const STEPS: usize = 46;
/// Characters of one report line, CR LF included.
const LINE: usize = 41;

/// The report lines of one printed pass, in step order.
fn emulated_pass() -> Vec<String> {
    let mut c = compile_board("hb01_pdp36/hb01.config", &["L4"]);
    let ebr: Vec<_> = c.report.ebr.iter().map(|e| e.widths).collect();
    assert_eq!(ebr.iter().filter(|w| w[1] == 36).count(), 2, "{ebr:?}");
    // A silent priming pass of 13 cycles a step, then the printed pass.
    let cycles = STEPS * (16 + LINE * 10 * BIT) + 2 * LINE * 10 * BIT;
    let text = decode_uart(&run_uart(&mut c, cycles as u64, "L4"));
    // Each line ends with a space before CR LF.
    let lines: Vec<String> = text
        .split("\r\n")
        .map(|l| l.trim_end().to_string())
        .collect();
    let pass: Vec<String> = lines
        .iter()
        .skip_while(|l| !l.starts_with("S00 "))
        .take(STEPS)
        .cloned()
        .collect();
    assert_eq!(pass.len(), STEPS, "{text}");
    pass
}

/// The lines of the board's observation log (one pass, identical in every
/// load and pass; README.md).
fn board_pass() -> Vec<String> {
    include_str!("../docs/inventory/LFE5U-85F/hb01_pdp36/pass.txt")
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb01_configuration_prints_the_board_lines() {
    let emulated = emulated_pass();
    for line in &emulated {
        println!("{line}");
    }
    let board = board_pass();
    assert_eq!(board.len(), STEPS);
    let differ: Vec<_> = emulated
        .iter()
        .zip(&board)
        .filter(|(e, b)| e != b)
        .map(|(e, b)| format!("emulator {e}\nboard    {b}"))
        .collect();
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
