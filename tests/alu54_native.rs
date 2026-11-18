//! ALU54B against the board: the HD02 configuration
//! (docs/inventory/LFE5U-85F/hd02_alu54) sweeps all 2 048 opcodes of one
//! ALU54B fed by two MULT18X18D, eight steps of fresh operands each, and
//! prints every step's operands, R and flags as one UART line.
//!
//! The event kernel runs the first steps; fast mode runs the whole pass of
//! 16 384 steps. Every line is the board's.
mod board;
use board::{compile_board, decode_uart, run_uart, PERIOD_PS};
use ecp5_emu::{fast::FastSim, kernel::Time};
use std::collections::HashMap;

/// Characters of a line, without CR LF: "T" and 56 hex digits.
const LINE: usize = 57;
/// Cycles of a step: 168 shifts, 8 to settle, the pulse, 8 to hold, one to
/// load each character, and 59 characters of 10 bits of 217 cycles.
const STEP: u64 = 168 + 8 + 1 + 8 + 59 * (10 * 217 + 1);

/// The board's lines by step number.
fn board_lines() -> HashMap<String, String> {
    let observations = include_str!("../docs/inventory/LFE5U-85F/hd02_alu54/observations.log");
    let mut out = HashMap::new();
    for line in observations.lines() {
        if let Some((_, list)) = line.split_once("distinct=") {
            for item in list.split('\'').skip(1).step_by(2) {
                if item.len() == LINE {
                    let old = out.insert(item[1..5].to_string(), item.to_string());
                    assert!(
                        old.is_none_or(|o| o == item),
                        "two board lines for one step"
                    );
                }
            }
        }
    }
    assert_eq!(out.len(), 16_384);
    out
}

fn lines(samples: &[bool]) -> Vec<String> {
    let line: Vec<bool> = std::iter::once(true)
        .chain(samples.iter().copied())
        .collect();
    decode_uart(&line)
        .split("\r\n")
        .filter(|l| l.len() == LINE && l.starts_with('T'))
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database; about a minute"]
fn hd02_first_steps_in_the_event_kernel_are_the_boards() {
    let board = board_lines();
    let mut c = compile_board("hd02_alu54/hd02.config", &["L4"]);
    assert_eq!(c.report.counts.alu54, 1);
    assert_eq!(c.report.counts.multiplier18, 2);
    let lines = lines(&run_uart(&mut c, 4 * STEP + 3_000, "L4"));
    assert!(lines.len() >= 4, "{} lines", lines.len());
    for l in &lines {
        assert_eq!(*l, board[&l[1..5]], "step {}", &l[1..5]);
    }
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database; several minutes"]
fn hd02_whole_pass_in_fast_mode_is_the_boards() {
    // Exhaustive by design: all 2 048 opcodes, eight operand sets each,
    // including operations that read the previous R. Skipping records would
    // lose the feedback history this whole-pass board comparison checks.
    let board = board_lines();
    let c = compile_board("hd02_alu54/hd02.config", &["L4"]);
    let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
    fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
        .unwrap();
    let samples = fast
        .run_cycles(16_384 * STEP + 3_000, c.probes["pin:L4"])
        .unwrap();
    let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
    let lines = lines(&samples);
    assert!(lines.len() >= 16_384, "{} lines", lines.len());
    for l in &lines {
        assert_eq!(*l, board[&l[1..5]], "step {}", &l[1..5]);
    }
}

/// HD05 (docs/inventory/LFE5U-85F/hd05_alu54_chain): eight ALU54B in four DSP
/// blocks, chained by the cascade, every output registered. Two sweep the
/// opcodes in two orders, three show CIN >> 18 with its fill, one has CE0 high
/// in the netlist but runs on the first ALU's pulse (nextpnr's remapping of
/// the block's CE lines), two have other flag patterns.
/// The first line shows the registers before any clock enable.
mod hd05 {
    use super::*;
    use std::collections::HashSet;

    /// Characters of a line, without CR LF: a letter and 202 hex digits.
    const LINE: usize = 203;
    /// Cycles of a step: as HD02's with 205 characters.
    const STEP: u64 = 168 + 8 + 1 + 8 + 205 * (10 * 217 + 1);
    /// Cycles to the end of the first line "I...": the wait, then the line.
    const START: u64 = (1 << 24) + 205 * (10 * 217 + 1);

    fn board_lines() -> HashSet<String> {
        let observations =
            include_str!("../docs/inventory/LFE5U-85F/hd05_alu54_chain/observations.log");
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
            .filter(|l| l.len() == LINE && (l.starts_with('T') || l.starts_with('I')))
            .map(str::to_string)
            .collect()
    }

    #[test]
    #[ignore = "requires clang, the native image reader and a local Trellis database; long (~0.8 h)"]
    fn hd05_whole_pass_in_fast_mode_is_the_boards() {
        // Exhaustive by design: replay the entire chain in both opcode
        // orders, including cascade/feedback history and the pass boundary.
        // A sample cannot replace this whole-record comparison.
        let board = board_lines();
        let c = compile_board("hd05_alu54_chain/hd05.config", &["L4"]);
        assert_eq!(c.report.counts.alu54, 8);
        let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        // The first line, the 8 192 steps of a pass and the first steps of
        // the next.
        let samples = fast
            .run_cycles(START + 8_200 * STEP + 3_000, c.probes["pin:L4"])
            .unwrap();
        let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
        let lines = lines(&samples);
        assert!(lines.len() >= 8_200, "{} lines", lines.len());
        assert!(lines[0].starts_with('I'));
        for (n, l) in lines.iter().enumerate() {
            assert!(
                board.contains(l),
                "line {n} (step {}) is not a board line",
                &l[1..5]
            );
        }
    }

    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; long (~0.4 h)"]
    fn hd05_start_in_the_event_kernel_is_the_boards() {
        // Keep all three startup records: configuration values and two
        // consecutive captures of every ALU and flag in the eight-cell
        // cascade. This is a bounded startup replay, not an opcode sweep.
        let board = board_lines();
        let mut c = compile_board("hd05_alu54_chain/hd05.config", &["L4"]);
        let lines = lines(&run_uart(&mut c, START + 2 * STEP + 3_000, "L4"));
        assert!(
            lines.len() >= 3 && lines[0].starts_with('I'),
            "{} lines",
            lines.len()
        );
        for (n, l) in lines.iter().enumerate() {
            assert!(board.contains(l), "line {n} is not a board line");
        }
    }
}

/// HD04 (docs/inventory/LFE5U-85F/hd04_alu54_cascade): HD05's chain and four
/// ALUs with output registers off in part or in whole. Those can read R
/// through unregistered bits, a loop on the chip, so fast mode refuses the
/// configuration; the event kernel runs its start, before any such opcode.
mod hd04 {
    use super::*;
    use std::collections::HashSet;

    /// Characters of a line, without CR LF: a letter and 266 hex digits.
    const LINE: usize = 267;
    const STEP: u64 = 168 + 8 + 1 + 8 + 269 * (10 * 217 + 1);
    const START: u64 = (1 << 24) + 269 * (10 * 217 + 1);

    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; long (~0.6 h)"]
    fn hd04_start_in_the_event_kernel_is_the_boards_and_fast_mode_refuses_the_loops() {
        // Keep the complete startup trace: the initial I record and two
        // consecutive captures compare every output/flag of all twelve
        // registered, partly registered and bypassed ALUs with the board.
        // Later opcodes can loop; HD05 checks the whole registered chain.
        let observations =
            include_str!("../docs/inventory/LFE5U-85F/hd04_alu54_cascade/observations.log");
        let mut board = HashSet::new();
        for line in observations.lines() {
            if let Some((_, list)) = line.split_once("distinct=") {
                board.extend(
                    list.split('\'')
                        .skip(1)
                        .step_by(2)
                        .filter(|i| i.len() == LINE),
                );
            }
        }
        let mut c = compile_board("hd04_alu54_cascade/hd04.config", &["L4"]);
        assert_eq!(c.report.counts.alu54, 12);
        let error = FastSim::new(&c.kernel)
            .err()
            .expect("fast mode must refuse ALUs that can read R through unregistered bits");
        assert!(format!("{error:#}").contains("unregistered"), "{error:#}");
        let samples = run_uart(&mut c, START + 2 * STEP + 3_000, "L4");
        let line: Vec<bool> = std::iter::once(true)
            .chain(samples.iter().copied())
            .collect();
        let text = decode_uart(&line);
        let lines: Vec<&str> = text.split("\r\n").filter(|l| l.len() == LINE).collect();
        assert!(
            lines.len() >= 3 && lines[0].starts_with('I'),
            "{} lines",
            lines.len()
        );
        for (n, l) in lines.iter().enumerate() {
            assert!(board.contains(l), "line {n} is not a board line");
        }
    }
}

/// HD06 and HD06b (docs/inventory/LFE5U-85F/hd06_alu54_reset): four ALU54B
/// computing R = C at every clock under random CE0..3, RST0..3 and a user
/// GSR, with RESETMODE of the ALUs and of their MULT18 in every combination
/// that matters. Each line is a window of 256 consecutive samples.
mod hd06 {
    use super::*;
    use std::collections::HashSet;

    /// Characters of a line, without CR LF: "M" and 2 560 hex digits.
    const LINE: usize = 2561;
    /// Cycles of a window: 256 samples, then 2 563 characters.
    const WINDOW: u64 = 256 + 2563 * (10 * 217 + 1);

    fn board_lines(observations: &str) -> HashSet<&str> {
        let mut out = HashSet::new();
        for line in observations.lines() {
            if let Some((_, list)) = line.split_once("distinct=") {
                out.extend(
                    list.split('\'')
                        .skip(1)
                        .step_by(2)
                        .filter(|i| i.len() == LINE),
                );
            }
        }
        out
    }

    fn windows(samples: &[bool]) -> Vec<String> {
        let line: Vec<bool> = std::iter::once(true)
            .chain(samples.iter().copied())
            .collect();
        decode_uart(&line)
            .split("\r\n")
            .filter(|l| l.len() == LINE)
            .map(str::to_string)
            .collect()
    }

    /// HD06b has every kind of reset: eb synchronous, ea half and half, ec
    /// and ed asynchronous. The reader opens the port while the first window
    /// is on its way, so the second is the one the board record must have.
    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; long (~1.7 h)"]
    fn hd06b_first_windows_in_the_event_kernel_are_the_boards() {
        // Whole-record replay by design: window zero may be absent from the
        // board record, so window one is the first guaranteed comparison.
        // Keep its full 256 CE/reset samples and the preceding clock history;
        // there is no redundant record sweep here to sample down.
        let board = board_lines(include_str!(
            "../docs/inventory/LFE5U-85F/hd06_alu54_reset/observations_b.log"
        ));
        let mut c = compile_board("hd06_alu54_reset/hd06b.config", &["L4"]);
        assert_eq!(c.report.counts.alu54, 4);
        let lines = windows(&run_uart(&mut c, 2 * WINDOW + 3_000, "L4"));
        assert!(lines.len() >= 2, "{} lines", lines.len());
        assert!(
            board.contains(lines[1].as_str()),
            "the second window is not the board's"
        );
    }

    #[test]
    #[ignore = "requires clang, the native image reader and a local Trellis database; minutes"]
    fn hd06_windows_in_fast_mode_are_the_boards() {
        for (config, observations) in [
            (
                "hd06_alu54_reset/hd06.config",
                include_str!("../docs/inventory/LFE5U-85F/hd06_alu54_reset/observations.log"),
            ),
            (
                "hd06_alu54_reset/hd06b.config",
                include_str!("../docs/inventory/LFE5U-85F/hd06_alu54_reset/observations_b.log"),
            ),
        ] {
            let board = board_lines(observations);
            let c = compile_board(config, &["L4"]);
            assert_eq!(c.report.counts.alu54, 4);
            let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
            fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
                .unwrap();
            let samples = fast
                .run_cycles(8 * WINDOW + 3_000, c.probes["pin:L4"])
                .unwrap();
            let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
            let lines = windows(&samples);
            assert!(lines.len() >= 8, "{config}: {} lines", lines.len());
            // The first window is on its way before the reader opens the
            // port, so the board record may lack it.
            let missing: Vec<usize> = (0..8)
                .filter(|&n| !board.contains(lines[n].as_str()))
                .collect();
            assert!(
                missing.is_empty() || missing == [0],
                "{config}: windows {missing:?} are not the board's"
            );
        }
    }
}

/// HD07 (docs/inventory/LFE5U-85F/hd07_dsp_gsr): GSR enabled in every slice,
/// four ALUs with every reset line and RESETMODE combination, and four
/// MULT18 with a registered output and no ALU. hd07s records the first
/// window, from the first clock, and sends it again and again.
mod hd07 {
    use super::*;
    use std::collections::HashSet;

    /// Characters of a line, without CR LF: "M" and 3 840 hex digits.
    const LINE: usize = 3841;
    /// Cycles of a window: 256 samples, then 3 843 characters.
    const WINDOW: u64 = 256 + 3843 * (10 * 217 + 1);

    fn board_lines(observations: &str) -> HashSet<&str> {
        let mut out = HashSet::new();
        for line in observations.lines() {
            if let Some((_, list)) = line.split_once("distinct=") {
                out.extend(
                    list.split('\'')
                        .skip(1)
                        .step_by(2)
                        .filter(|i| i.len() == LINE),
                );
            }
        }
        out
    }

    fn windows(samples: &[bool]) -> Vec<String> {
        let line: Vec<bool> = std::iter::once(true)
            .chain(samples.iter().copied())
            .collect();
        decode_uart(&line)
            .split("\r\n")
            .filter(|l| l.len() == LINE)
            .map(str::to_string)
            .collect()
    }

    fn first_window() -> String {
        let board = board_lines(include_str!(
            "../docs/inventory/LFE5U-85F/hd07_dsp_gsr/observations_s.log"
        ));
        assert_eq!(board.len(), 1, "hd07s gave one window");
        board.into_iter().next().unwrap().to_string()
    }

    fn fast_windows(config: &str, n: u64) -> Vec<String> {
        let c = compile_board(config, &["L4"]);
        assert_eq!(c.report.counts.alu54, 4);
        let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        let samples = fast
            .run_cycles(n * WINDOW + 3_000, c.probes["pin:L4"])
            .unwrap();
        let samples: Vec<bool> = samples.iter().map(|&b| b == 1).collect();
        windows(&samples)
    }

    #[test]
    #[ignore = "requires clang, the native image reader and a local Trellis database; minutes"]
    fn hd07_windows_in_fast_mode_are_the_boards() {
        let board = board_lines(include_str!(
            "../docs/inventory/LFE5U-85F/hd07_dsp_gsr/observations.log"
        ));
        let lines = fast_windows("hd07_dsp_gsr/hd07.config", 8);
        assert!(lines.len() >= 8, "{} lines", lines.len());
        // The reader opens the port while the first window is on its way;
        // hd07s, the same design, recorded it.
        assert_eq!(lines[0], first_window(), "the first window");
        let missing: Vec<usize> = (1..8)
            .filter(|&n| !board.contains(lines[n].as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "windows {missing:?} are not the board's"
        );
    }

    #[test]
    #[ignore = "requires clang, the native image reader and a local Trellis database; minutes"]
    fn hd07s_first_window_in_fast_mode_is_the_boards() {
        let lines = fast_windows("hd07_dsp_gsr/hd07s.config", 2);
        assert!(lines.len() >= 2, "{} lines", lines.len());
        let first = first_window();
        assert!(
            lines.iter().all(|l| *l == first),
            "a window is not the board's"
        );
    }

    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; long (~2.2 h)"]
    fn hd07s_first_window_in_the_event_kernel_is_the_boards() {
        // Whole-record replay by design: all 256 startup samples of the
        // recorded ALU and MULT18 output bits, including their initial state
        // and CE/reset/GSR interactions. This design records only one window;
        // sampling it down would weaken the complete board-record comparison.
        let mut c = compile_board("hd07_dsp_gsr/hd07s.config", &["L4"]);
        assert_eq!(c.report.counts.alu54, 4);
        let lines = windows(&run_uart(&mut c, WINDOW + 3_000, "L4"));
        assert_eq!(lines.len(), 1, "{} lines", lines.len());
        assert_eq!(lines[0], first_window(), "the first window");
    }
}

/// HD08 (docs/inventory/LFE5U-85F/hd08_alu54_cop): the C registers and the
/// opcode registers. hd08b and hd08c have every register the planner runs
/// (no IN_1), hd08c with the DSP constants on nets; hd08z reads Z = 7 against
/// RNDPAT; hd08 has IN_1, which is refused.
mod hd08 {
    use super::*;
    use std::collections::HashSet;

    /// Characters of a line, without CR LF: "M" and 5 120 hex digits.
    const LINE: usize = 5121;
    /// Cycles of a window: 128 samples, then 5 123 characters.
    const WINDOW: u64 = 128 + 5123 * (10 * 217 + 1);

    fn board_lines(observations: &str, len: usize) -> HashSet<&str> {
        let mut out = HashSet::new();
        for line in observations.lines() {
            if let Some((_, list)) = line.split_once("distinct=") {
                out.extend(
                    list.split('\'')
                        .skip(1)
                        .step_by(2)
                        .filter(|i| i.len() == len),
                );
            }
        }
        out
    }

    fn lines(samples: &[bool], len: usize) -> Vec<String> {
        let line: Vec<bool> = std::iter::once(true)
            .chain(samples.iter().copied())
            .collect();
        decode_uart(&line)
            .split("\r\n")
            .filter(|l| l.len() == len)
            .map(str::to_string)
            .collect()
    }

    fn fast(config: &str, cycles: u64) -> Vec<bool> {
        let c = compile_board(config, &["L4"]);
        let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        fast.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        let samples = fast.run_cycles(cycles, c.probes["pin:L4"]).unwrap();
        samples.iter().map(|&b| b == 1).collect()
    }

    /// hd08c: hd08b with the DSP constants on nets. In hd08b nextpnr leaves
    /// one live C pin of an ALU floating, which fast mode does not start on.
    #[test]
    #[ignore = "requires clang, the native image reader and a local Trellis database; long (~0.7 h)"]
    fn hd08c_windows_in_fast_mode_are_the_boards() {
        // The board may lack startup window zero. Windows one, two and
        // three are complete 128-sample records in modes 1, 2 and 0, so four
        // windows are the minimum guaranteed prefix covering every mode.
        // Keep every recorded ALU bit and CE/reset/GSR history in each mode.
        let board = board_lines(
            include_str!("../docs/inventory/LFE5U-85F/hd08_alu54_cop/observations_c.log"),
            LINE,
        );
        let lines = lines(
            &fast("hd08_alu54_cop/hd08c.config", 4 * WINDOW + 3_000),
            LINE,
        );
        assert!(lines.len() >= 4, "{} lines", lines.len());
        // The reader opens the port while the first window is on its way.
        let missing: Vec<usize> = (1..4)
            .filter(|&n| !board.contains(lines[n].as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "windows {missing:?} are not the board's"
        );
    }

    /// Z = 7 is RNDPAT's trailing-zero mask (HD08z): eight ALUs, one line
    /// each. The event kernel only: nextpnr leaves one of the ALU's own C pins
    /// (which do nothing on the chip) without a constant, and fast mode will
    /// not start on a floating net. hd08b checks Z = 7 in fast mode.
    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; minutes"]
    fn hd08z_lines_in_the_event_kernel_are_the_boards() {
        let board = board_lines(
            include_str!("../docs/inventory/LFE5U-85F/hd08_alu54_cop/observations_z.log"),
            16,
        );
        assert_eq!(board.len(), 8);
        // Eight lines of 17 characters, and one more for the start.
        let cycles = 9 * 17 * (10 * 217 + 1) + 3_000;
        let mut c = compile_board("hd08_alu54_cop/hd08z.config", &["L4"]);
        assert_eq!(c.report.counts.alu54, 8);
        let got: HashSet<String> = lines(&run_uart(&mut c, cycles, "L4"), 16)
            .into_iter()
            .collect();
        let got: HashSet<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, board);
    }

    /// hd08bs: hd08b recording its first window, from the first clock. The
    /// event kernel runs those 128 clocks and is read at the R pins of the
    /// eleven ALUs themselves, before every edge: every recorded bit is the
    /// board's. (Through the UART one window takes 11 M clocks, hours in the
    /// kernel.)
    #[test]
    #[ignore = "requires the native image reader and a local Trellis database; about a minute"]
    fn hd08bs_first_window_in_the_event_kernel_is_the_boards() {
        use ecp5_emu::{
            compile::{self, Board, Reference, Request, Scope, Startup},
            database::Database,
            image::ConfigurationImage,
            kernel::Options,
            logic::Logic,
            model::DeviceModel,
            prepared::{load_catalogue, ConfiguredDevice},
        };
        use std::{path::PathBuf, sync::Arc};
        let board = board_lines(
            include_str!("../docs/inventory/LFE5U-85F/hd08_alu54_cop/observations_bs.log"),
            LINE,
        );
        assert_eq!(board.len(), 1, "the board gave one window");
        let line = board.into_iter().next().unwrap();
        let bits: Vec<bool> = line[1..]
            .chars()
            .flat_map(|c| {
                let v = c.to_digit(16).unwrap();
                (0..4).rev().map(move |k| v >> k & 1 == 1)
            })
            .collect();
        // The recorded bits of each ALU: (first column of its block, second
        // slice, first bit of its field in a sample, R bits MSB first).
        let opcode = |n: usize, block: usize, second: bool| {
            (
                block,
                second,
                48 + 16 * n,
                (0..16).rev().collect::<Vec<usize>>(),
            )
        };
        let alus = [
            opcode(0, 13, false),
            opcode(1, 22, false),
            opcode(2, 31, false),
            opcode(3, 40, false),
            opcode(4, 49, false),
            opcode(5, 58, false),
            opcode(6, 58, true),
            (13, true, 32, vec![50, 30, 20, 0]),
            (22, true, 36, vec![50, 44, 40, 36]),
            (31, true, 40, vec![50, 30, 20, 0]),
            (40, true, 44, vec![50, 30, 20, 0]),
        ];
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let evidence = root.join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None).unwrap();
        let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
        // (sample bit, the R pin's wire)
        let mut probes = Vec::new();
        for (block, second, first, r) in &alus {
            let key = format!(
                "LFE5U-85F/R58C{}/ALU54_{}",
                block + if *second { 7 } else { 3 },
                if *second { 7 } else { 3 }
            );
            let id = model.resource_id(&key).unwrap();
            let ports = model.resource_ports(id);
            for (k, bit) in r.iter().enumerate() {
                let port = ports.iter().find(|p| p.name == format!("R{bit}")).unwrap();
                probes.push((first + k, Reference::Wire(port.wire.clone())));
            }
        }
        let image = Arc::new(
            ConfigurationImage::load(&evidence.join("hd08_alu54_cop/hd08bs.config"), &db, None)
                .unwrap(),
        );
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let mut c = compile::compile_on_board(
            device,
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: vec![(Reference::Pin("G2".into()), Logic::Zero)],
                probes: probes.iter().map(|(_, r)| r.clone()).collect(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
            &Board::default(),
        )
        .unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(c.report.counts.alu54, 11);
        // The compared bits move: a third or more of them are ones.
        let ones = (0..128)
            .flat_map(|k| probes.iter().map(move |(bit, _)| 160 * k + bit))
            .filter(|&i| bits[i])
            .count();
        assert!(
            (5_000..11_000).contains(&ones),
            "{ones} of 16 384 compared bits are ones"
        );
        let clock = c.drivers["pin:G2"];
        let mut differ = Vec::new();
        // Sample k is the state before edge k: edge k is the rise of cycle
        // k + 1.
        for k in 0..128u64 {
            if k > 0 {
                let rise = Time::from_ps((2 * k - 1) * PERIOD_PS / 2);
                let fall = Time::from_ps(k * PERIOD_PS);
                c.kernel.schedule(rise, clock, Logic::One).unwrap();
                c.kernel.run_until(rise).unwrap();
                c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
                c.kernel.run_until(fall).unwrap();
            }
            for (bit, r) in &probes {
                let want = bits[160 * k as usize + bit];
                let got = c.value(r).unwrap();
                if got != Logic::from_bool(want) {
                    differ.push(format!(
                        "sample {k} bit {bit}: kernel {got}, board {}",
                        u8::from(want)
                    ));
                }
            }
        }
        assert!(
            differ.is_empty(),
            "{} bits differ, first {:?}",
            differ.len(),
            &differ[..differ.len().min(8)]
        );
    }

    /// IN_1 followed no register model on the board (HD08 o4): refused.
    #[test]
    #[ignore = "requires the native image reader and a local Trellis database"]
    fn hd08_with_in_1_is_refused() {
        let error =
            std::panic::catch_unwind(|| compile_board("hd08_alu54_cop/hd08.config", &["L4"]))
                .err()
                .expect("the planner must refuse IN_1");
        let message = error.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(message.contains("REG_OPCODEIN_1_CLK"), "{message}");
    }
}
