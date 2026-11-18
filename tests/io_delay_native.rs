//! DELAYF's control against the board: the HI08 configurations
//! (docs/inventory/LFE5U-85F/hi08_delay) run a 2 048-step program of LOADN,
//! MOVE and DIRECTION on two DELAYF and print every step's controls and both
//! CFLAG as one UART line. Fast mode prints the board's line for HI08. In
//! HI08b and HI08c the program also changes DIRECTION while MOVE is high,
//! outside the documented protocol; there the lines differ in exactly the
//! samples the README's item 5 describes (8 and 30), and only between such a
//! change and the next load.
mod board;
use board::{compile_driven, decode_uart, PERIOD_PS};
use ecp5_emu::{compile::Board, fast::FastSim, kernel::Time, logic::Logic};

/// The 2 048 samples of {loadn, move, direction, cflag1, cflag0} in a
/// "D=<2560 hex>" line.
fn samples(line: &str) -> Vec<[bool; 5]> {
    let hex = line.trim().strip_prefix("D=").expect("a D= line");
    assert_eq!(hex.len(), 2560);
    let bits: Vec<bool> = hex
        .chars()
        .flat_map(|c| {
            let v = c.to_digit(16).unwrap();
            (0..4).rev().map(move |k| (v >> k) & 1 == 1)
        })
        .collect();
    bits.chunks(5)
        .map(|s| [s[0], s[1], s[2], s[3], s[4]])
        .collect()
}

/// The samples fast mode records for `config`, from its first line.
fn emulated(config: &str) -> Vec<[bool; 5]> {
    // The buttons the delays read stay released (low), as in the board runs.
    let drives = [("R1", Logic::Zero), ("T1", Logic::Zero)];
    let c = compile_driven(config, &["L4"], Board::default(), &drives);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    // 65 536 cycles of wait, 4 095 steps, then 2 564 characters of 2 170
    // cycles each.
    let cycles = 65_536 + 4_096 + 2_564 * 2_170 + 10_000;
    let line = fast.run_cycles(cycles, c.probes["pin:L4"]).unwrap();
    let line: Vec<bool> = std::iter::once(true)
        .chain(line.iter().map(|&b| b == 1))
        .collect();
    let text = decode_uart(&line);
    let first = text.split("\r\n").next().unwrap();
    samples(first)
}

/// The samples from a DIRECTION change while MOVE is high to the next load:
/// outside the protocol (hi08_model.py leaves them out).
fn outside_protocol(board: &[[bool; 5]]) -> Vec<bool> {
    let mut out = vec![false; board.len()];
    let mut skip = false;
    for i in 1..board.len() {
        let [loadn, move_, direction, ..] = board[i];
        let [_, prev_move, prev_direction, ..] = board[i - 1];
        if move_ && prev_move && direction != prev_direction {
            skip = true;
        }
        if !loadn {
            skip = false;
        }
        out[i] = skip;
    }
    out
}

fn compare(config: &str, record: &str, differing: usize) {
    let board = samples(record.split_whitespace().next().unwrap());
    let emulated = emulated(config);
    let outside = outside_protocol(&board);
    let mut differ = Vec::new();
    for (i, (b, e)) in board.iter().zip(&emulated).enumerate() {
        if b != e {
            assert!(
                outside[i],
                "{config}: sample {i} differs: board {b:?}, emulator {e:?}"
            );
            differ.push(i);
        }
    }
    assert_eq!(differ.len(), differing, "{config}: {differ:?}");
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi08_delayf_control_prints_the_board_line() {
    compare(
        "hi08_delay/hi08.config",
        include_str!("../docs/inventory/LFE5U-85F/hi08_delay/pass.txt"),
        0,
    );
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hi08b_and_hi08c_differ_only_outside_the_protocol() {
    compare(
        "hi08_delay/hi08b.config",
        include_str!("../docs/inventory/LFE5U-85F/hi08_delay/pass_b.txt"),
        8,
    );
    compare(
        "hi08_delay/hi08c.config",
        include_str!("../docs/inventory/LFE5U-85F/hi08_delay/pass_c.txt"),
        30,
    );
}
