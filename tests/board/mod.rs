#![allow(dead_code)]
// Replays configurations that ran on the ULX3S and decodes their UART reports.
use ecp5_emu::{
    compile::{self, Board, BoardWire, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, sync::Arc};

/// Reference period of the ULX3S 25 MHz oscillator.
pub const PERIOD_PS: u64 = 40_000;
/// Reference cycles per UART bit (115200 baud from 25 MHz, `baud == 216`).
pub const BIT: usize = 217;

pub fn pin(ball: &str) -> Reference {
    Reference::Pin(ball.into())
}

/// Compiles a configuration from `docs/inventory/LFE5U-85F` with the 25 MHz
/// clock on G2 as the only input.
pub fn compile_board(config: &str, probes: &[&str]) -> Compiled {
    compile_on(config, probes, Board::default())
}

/// Jumper wires on the board, as (from, to) balls.
pub fn wires(pairs: &[(&str, &str)]) -> Vec<BoardWire> {
    pairs
        .iter()
        .map(|&(from, to)| BoardWire {
            from: pin(from),
            to: pin(to),
        })
        .collect()
}

/// `compile_board` with a board description.
pub fn compile_on(config: &str, probes: &[&str], board: Board) -> Compiled {
    compile_driven(config, probes, board, &[])
}

/// `compile_on` with more pins driven from outside (as buttons are), each
/// starting at the given value; the 25 MHz clock on G2 is always one.
pub fn compile_driven(
    config: &str,
    probes: &[&str],
    board: Board,
    drives: &[(&str, Logic)],
) -> Compiled {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(ConfigurationImage::load(&evidence.join(config), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    compile::compile_on_board(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: std::iter::once(("G2", Logic::Zero))
                .chain(drives.iter().copied())
                .map(|(ball, value)| (pin(ball), value))
                .collect(),
            probes: probes.iter().map(|ball| pin(ball)).collect(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
        &board,
    )
    .unwrap_or_else(|e| panic!("{e:#}"))
}

/// Runs `cycles` reference cycles and samples the UART line `tx` after each
/// falling edge.
pub fn run_uart(c: &mut Compiled, cycles: u64, tx: &str) -> Vec<bool> {
    let reference = c.drivers["pin:G2"];
    let mut samples = Vec::new();
    for cycle in 1..=cycles {
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, reference, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        c.kernel.schedule(fall, reference, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        let value = c.value(&pin(tx)).unwrap();
        samples.push(
            value
                .bit()
                .unwrap_or_else(|| panic!("UART line is {value} at cycle {cycle}")),
        );
    }
    samples
}

pub fn decode_uart(samples: &[bool]) -> String {
    let mut text = String::new();
    let mut i = 1;
    while i < samples.len() {
        if samples[i - 1] && !samples[i] {
            // Start bit begins at i; sample each bit in its middle.
            let centre = |bit: usize| i + bit * BIT + BIT / 2;
            if centre(9) >= samples.len() {
                break;
            }
            let byte = (1..=8).fold(0_u8, |b, n| b | (u8::from(samples[centre(n)]) << (n - 1)));
            assert!(samples[centre(9)], "missing UART stop bit");
            text.push(byte as char);
            i = centre(9);
        }
        i += 1;
    }
    text
}
