//! The X2 gearbox against the board: the HI03 configuration
//! (docs/inventory/LFE5U-85F/hi03_gearbox) runs ECLKSYNCB, CLKDIVF /2,
//! ODDRX2F and IDDRX2F from an edge clock made in the fabric and prints one
//! line: 192 samples of {eclk_in, st, pad} at 25 MHz (O), 32 sclk cycles of
//! the IDDRX2F outputs (I) and the sclk rising edges counted before the
//! first sample (N).
//!
//! Both executors print exactly the board's line. Its N also pins the start:
//! ECLKO begins with the fifth rising edge of ECLKI, through ECLKSYNCB's STOP
//! synchronizer, which starts closed (HI04, CLOCK_NETWORK.md).
mod board;
use board::{compile_board, decode_uart, run_uart, BIT, PERIOD_PS};
use ecp5_emu::{compile::Compiled, fast::FastSim, kernel::Time};

/// Characters of the line, CR LF included.
const LINE: usize = 188;
/// The line starts at t = 0x1fff; one line and a little more.
const CYCLES: u64 = (0x1fff + LINE * 10 * BIT + 1000) as u64;

fn compiled() -> Compiled {
    let c = compile_board("hi03_gearbox/hi03.config", &["L4"]);
    assert_eq!(c.report.counts.eclk_syncs, 1);
    assert_eq!(c.report.counts.clock_dividers, 1);
    c
}

/// O (hex), I (hex) and N of the first line in `text`.
fn fields(text: &str) -> (String, String, usize) {
    let line = text
        .split("\r\n")
        .find(|l| l.len() == LINE - 2 && l.starts_with("O="))
        .unwrap_or_else(|| panic!("no line in {text:?}"));
    let f: Vec<&str> = line.split(' ').collect();
    (
        f[0][2..].to_string(),
        f[1][2..].to_string(),
        usize::from_str_radix(&f[2][2..], 16).unwrap(),
    )
}

fn check(text: &str) {
    let board = include_str!("../docs/inventory/LFE5U-85F/hi03_gearbox/pass.txt");
    let (bo, bi, bn) = fields(&format!("{}\r\n", board.trim_end()));
    let (eo, ei, en) = fields(text);
    assert_eq!((en, &eo, &ei), (bn, &bo, &bi));
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hi03_prints_the_board_line_in_the_event_kernel() {
    let mut c = compiled();
    check(&decode_uart(&run_uart(&mut c, CYCLES, "L4")));
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi03_prints_the_board_line_in_fast_mode() {
    let c = compiled();
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast.run_cycles(CYCLES, c.probes["pin:L4"]).unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().map(|&b| b == 1))
        .collect();
    check(&decode_uart(&line));
}
