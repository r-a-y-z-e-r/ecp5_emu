//! HB09: replay the board's first 128 clocks at the eight DP16KD outputs.
//! Sampling the pins avoids the millions of clocks needed to send the UART.
mod board;
use board::compile_board;
use ecp5_emu::{
    compile::{self, Board, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{ebr_cell, Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{collections::HashSet, path::PathBuf, sync::Arc};

const WIDTHS: [usize; 8] = [2, 1, 4, 9, 18, 9, 9, 9];
const SEED: u64 = 0x1D2C_ACE1_5EED_0001;
const PERIOD_PS: u64 = 40_000;

fn step(mut lfsr: u64) -> u64 {
    for _ in 0..32 {
        let feedback = ((lfsr >> 63) ^ (lfsr >> 62) ^ (lfsr >> 60) ^ (lfsr >> 59)) & 1;
        lfsr = (lfsr << 1) | feedback;
    }
    lfsr
}

fn traffic(lfsr: u64) -> ([usize; 2], [bool; 2]) {
    (
        [(lfsr & 3) as usize, ((lfsr >> 2) & 3) as usize],
        [lfsr & (1 << 4) != 0, lfsr & (1 << 5) != 0],
    )
}

fn board_samples() -> Vec<u128> {
    let log = include_str!("../docs/inventory/LFE5U-85F/hb09_ebr_collision/observations_s.log");
    let mut windows = HashSet::new();
    let mut loads = 0;
    for record in log.lines() {
        let (_, list) = record.split_once("distinct=").unwrap();
        let full: HashSet<_> = list
            .split('\'')
            .skip(1)
            .step_by(2)
            .filter(|line| line.len() == 17 + 128 * 31 && line.starts_with('R'))
            .collect();
        assert_eq!(full.len(), 1, "one full first window per load");
        windows.extend(full);
        loads += 1;
    }
    assert_eq!(loads, 3);
    assert_eq!(windows.len(), 1, "all three loads agree");
    let line = windows.into_iter().next().unwrap();
    assert_eq!(u64::from_str_radix(&line[1..17], 16).unwrap(), SEED);
    (0..128)
        .map(|k| {
            let sample = u128::from_str_radix(&line[17 + 31 * k..17 + 31 * (k + 1)], 16).unwrap();
            assert_eq!(sample & 3, 0, "two pad bits");
            sample
        })
        .collect()
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hb09s_first_window_in_the_event_kernel_is_the_boards() {
    first_window(false);
}

#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hb09s_first_window_in_fast_mode_matches_the_kernel_and_board() {
    first_window(true);
}

fn first_window(use_fast: bool) {
    let board = board_samples();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    // Placed blocks in hb09s.config, in the generator's e0..e7 order.
    let locations = [
        (46, 10, 3),
        (70, 10, 3),
        (70, 8, 2),
        (70, 6, 1),
        (70, 4, 0),
        (46, 4, 0),
        (46, 6, 1),
        (46, 8, 2),
    ];
    let mut probes = Vec::new();
    let mut offset = 124;
    for (ebr, ((row, col, block), width)) in locations.into_iter().zip(WIDTHS).enumerate() {
        let key = format!("LFE5U-85F/R{row}C{col}/EBR{block}");
        let ports = model.resource_ports(model.resource_id(&key).unwrap());
        for (side, letter) in ['A', 'B'].into_iter().enumerate() {
            offset -= width;
            for bit in 0..width {
                let port = ports
                    .iter()
                    .find(|p| p.name == format!("DO{letter}{bit}"))
                    .unwrap();
                probes.push((
                    ebr,
                    side,
                    bit,
                    offset + bit,
                    Reference::Wire(port.wire.clone()),
                ));
            }
        }
    }
    assert_eq!(offset, 2);
    assert_eq!(probes.len(), 122);
    let image = Arc::new(
        ConfigurationImage::load(&evidence.join("hb09_ebr_collision/hb09s.config"), &db, None)
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
            probes: probes.iter().map(|(_, _, _, _, r)| r.clone()).collect(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
        &Board::default(),
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(c.report.counts.ebr_blocks, 8);
    let mut fast = use_fast.then(|| {
        let mut f = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        f.clock(c.drivers["pin:G2"], Time::from_ps(PERIOD_PS))
            .unwrap();
        f
    });

    // Direct witnesses of the old policy: any other-port write to the
    // INIT9 row set the captured memory to X, even for disjoint words.
    // A read or an RBW write exposed that X (OUTREG one edge later).
    let mut legacy_x = [[[false; 2]; 8]; 128];
    let mut collisions = [[0; 3]; 8]; // A reads/B writes, B reads/A writes, both write
    let mut disjoint_row = [0; 3]; // e0/e1/e2: different words in one INIT9 row
    let mut lfsr = SEED;
    for edge in 0..127 {
        let (address, write) = traffic(lfsr);
        for (ebr, width) in WIDTHS.into_iter().enumerate() {
            let sample = edge + 1 + usize::from(ebr == 7);
            if sample >= 128 {
                continue;
            }
            if address[0] == address[1] {
                match write {
                    [false, true] => collisions[ebr][0] += 1,
                    [true, false] => collisions[ebr][1] += 1,
                    [true, true] => collisions[ebr][2] += 1,
                    _ => {}
                }
            }
            let same_row = ebr_cell(width, address[0], 0).unwrap().0
                == ebr_cell(width, address[1], 0).unwrap().0;
            if ebr < 3 && same_row && address[0] != address[1] && write.contains(&true) {
                disjoint_row[ebr] += 1;
            }
            for side in 0..2 {
                legacy_x[sample][ebr][side] =
                    same_row && write[1 - side] && (!write[side] || ![5, 6].contains(&ebr));
            }
        }
        lfsr = step(lfsr);
    }
    // These count traffic even when the analysis could not score an old
    // word after a double write (its KNOWN column excludes those cases).
    assert_eq!(&collisions[..7], &[[10, 6, 9]; 7]);
    assert_eq!(collisions[7], [9, 6, 9]);
    assert_eq!(disjoint_row, [69, 69, 20]);

    // Independent uncertainty tracking: only e4's array becomes unknown
    // after a same-word double write, until a single writer replaces it.
    // The writing ports themselves still capture the pre-write word.
    let mut unknown_words = [false; 4];
    let mut unknown_outputs = [false; 2];
    let mut known_bits = 0;
    let mut unknown_bits = 0;
    let mut resolved_legacy_x = 0;
    let mut differ = Vec::new();
    let clock = c.drivers["pin:G2"];
    lfsr = SEED;
    for (sample, &want) in board.iter().enumerate() {
        for (ebr, side, bit, position, reference) in &probes {
            let got = c.value(reference).unwrap();
            if let Some(fast) = &fast {
                assert_eq!(
                    fast.value(c.probes[&reference.label()]),
                    Some(got),
                    "fast/kernel sample {sample} e{ebr}.{side} bit {bit}"
                );
            }
            let unknown = *ebr == 4 && unknown_outputs[*side];
            if unknown {
                assert_eq!(
                    got,
                    Logic::X,
                    "sample {sample} e{ebr}.{side} bit {bit}: expected X"
                );
                unknown_bits += 1;
            } else {
                let expected = Logic::from_bool(want & (1 << position) != 0);
                if got != expected {
                    differ.push(format!(
                        "sample {sample} e{ebr}.{side} bit {bit}: kernel {got}, board {expected}"
                    ));
                }
                known_bits += 1;
                resolved_legacy_x += usize::from(legacy_x[sample][*ebr][*side]);
            }
        }
        if sample == 127 {
            break;
        }
        let (address, write) = traffic(lfsr);
        unknown_outputs = address.map(|a| unknown_words[a]);
        if address[0] == address[1] && write == [true, true] {
            unknown_words[address[0]] = true;
        } else {
            for side in 0..2 {
                if write[side] {
                    unknown_words[address[side]] = false;
                }
            }
        }
        let rise = Time::from_ps((2 * sample as u64 + 1) * PERIOD_PS / 2);
        let fall = Time::from_ps((sample as u64 + 1) * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        if let Some(fast) = &mut fast {
            fast.run_until(fall).unwrap();
        }
        lfsr = step(lfsr);
    }
    assert!(
        differ.is_empty(),
        "{} differing bits; first {:?}",
        differ.len(),
        differ.first()
    );
    assert_eq!(known_bits + unknown_bits, 128 * 122);
    assert_eq!(unknown_bits, 306, "width-18 double-write X domain");
    assert_eq!(
        resolved_legacy_x, 2_086,
        "the replay exercises the new rule"
    );
    assert_eq!(c.kernel.statistics().ebr_collision_rows, 18);
    eprintln!("HB09: {known_bits} known bits match; {unknown_bits} X bits exactly in e4's poisoned words; {resolved_legacy_x} directly witnessed former X bits resolved; collisions {collisions:?}; disjoint-row cycles {disjoint_row:?}");
}

// Both raw UART fixtures serialize e4's unknown words too. Preserving the
// required width-18 X policy precludes a complete, two-valued UART line.
// The capture guard may stop on the first raw X. Selected-input demand may
// defer it until that X is actually captured. Neither may invent a known
// register value after the event kernel has captured X.
#[test]
#[ignore = "requires clang, the native image reader and a local Trellis database"]
fn hb09_uart_stops_between_width18_x_and_its_first_register_capture() {
    for config in ["hb09.config", "hb09s.config"] {
        let mut c = compile_board(&format!("hb09_ebr_collision/{config}"), &["L4"]);
        let resource = c
            .report
            .ebr
            .iter()
            .find(|e| e.widths == [18; 2])
            .unwrap()
            .resource
            .clone();
        let model = c.kernel.device().model();
        let pins: Vec<_> = model
            .resource_ports(model.resource_id(&resource).unwrap())
            .into_iter()
            .filter(|p| p.name.starts_with("DO"))
            .map(|p| Reference::Wire(p.wire.clone()))
            .collect();
        assert_eq!(pins.len(), 36);
        let registers: Vec<_> = model
            .resources()
            .filter(|(_, r)| r.family == "PLC2")
            .flat_map(|(id, _)| model.nodes(id))
            .filter(|n| n.kind == "TRELLIS_FF")
            .filter(|n| {
                model
                    .node_ports(n.id)
                    .iter()
                    .any(|p| p.name == "Q" && c.kernel.signal(&p.wire).is_some())
            })
            .map(|n| n.id)
            .collect();
        assert!(!registers.is_empty());
        let mut fast = FastSim::new(&c.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        let clock = c.drivers["pin:G2"];
        fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
        let stopped = (1..=128).find(|&cycle| {
            let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
            let fall = Time::from_ps(cycle * PERIOD_PS);
            c.kernel.schedule(rise, clock, Logic::One).unwrap();
            c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
            c.kernel.run_until(fall).unwrap();
            let result = fast.run_until(fall);
            if let Err(error) = result {
                let error = format!("{error:#}");
                assert!(pins.iter().any(|p| c.value(p).unwrap() == Logic::X)
                    || registers.iter().any(|&n| c.kernel.state().register(n).unwrap() == Logic::X));
                assert!(error.contains(&resource), "{error}");
                assert!(cycle > 2, "the old narrow collision must be resolved");
                eprintln!("{config}: width-18 X refused at rising edge {cycle} ({rise}); full board UART equality is incompatible with the required X policy");
                true
            } else {
                assert!(registers.iter().all(|&n| c.kernel.state().register(n).unwrap().bit().is_some()),
                    "{config} cycle {cycle}: fast continued after a register captured X");
                assert_eq!(fast.value(c.probes["pin:L4"]), Some(c.value(&board::pin("L4")).unwrap()));
                false
            }
        });
        assert!(stopped.is_some(), "must exercise width-18 double-write X");
    }
}
