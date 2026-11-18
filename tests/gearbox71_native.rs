//! The 7:1 gearbox against the board: HI07 and HI07b
//! (docs/inventory/LFE5U-85F/hi07_gearbox71) run ECLKSYNCB, CLKDIVF /3.5,
//! ODDR71B and IDDR71B from an edge clock made in the fabric and print one
//! line: the ODDR71B pad and SCLK at 25 MHz (O), 16 SCLK cycles of the
//! IDDR71B outputs (I) and the SCLK edges before the first sample (N);
//! HI07b also the IDDR71B outputs at 25 MHz (Q). The emulator prints the
//! board's lines exactly: HI07 in the event kernel and in fast mode, HI07b
//! (2.6 M cycles) in fast mode.
mod board;
use board::{compile_board, decode_uart, run_uart, BIT, PERIOD_PS};
use ecp5_emu::{compile::Compiled, fast::FastSim, kernel::Time};

fn compiled(config: &str) -> Compiled {
    let c = compile_board(config, &["L4"]);
    assert_eq!(c.report.counts.clock_dividers, 1);
    c
}

/// The first complete line of `text`, starting with O=.
fn line(text: &str, chars: usize) -> String {
    text.split("\r\n")
        .find(|l| l.len() == chars && l.starts_with("O="))
        .unwrap_or_else(|| panic!("no line in {text:?}"))
        .to_string()
}

fn fast_text(c: &Compiled, cycles: u64) -> String {
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast.run_cycles(cycles, c.probes["pin:L4"]).unwrap();
    let bits: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    decode_uart(&bits)
}

const HI07: &str = include_str!("../docs/inventory/LFE5U-85F/hi07_gearbox71/pass.txt");
const HI07B: &str = include_str!("../docs/inventory/LFE5U-85F/hi07_gearbox71/pass_b.txt");
/// The line starts at t = 0x1fff; 424 characters (HI07) or 1195 (HI07b).
const HI07_CYCLES: u64 = (0x1fff + 424 * 10 * BIT + 2000) as u64;
const HI07B_CYCLES: u64 = (0x1fff + 1195 * 10 * BIT + 2000) as u64;

#[test]
#[ignore = "requires the native image reader and a local Trellis database; ~40 s"]
fn hi07_prints_the_board_line_in_the_event_kernel() {
    let mut c = compiled("hi07_gearbox71/hi07.config");
    let text = decode_uart(&run_uart(&mut c, HI07_CYCLES, "L4"));
    assert_eq!(line(&text, 422), HI07.trim_end());
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi07_and_hi07b_print_the_board_lines_in_fast_mode() {
    let c = compiled("hi07_gearbox71/hi07.config");
    assert_eq!(line(&fast_text(&c, HI07_CYCLES), 422), HI07.trim_end());
    let c = compiled("hi07_gearbox71/hi07b.config");
    assert_eq!(line(&fast_text(&c, HI07B_CYCLES), 1193), HI07B.trim_end());
}
