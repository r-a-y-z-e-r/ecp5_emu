//! HY04's exact routed board configuration. The planner checks its DTR path;
//! the conversion harness isolates the primitive on its actual native wires,
//! avoiding contention with the board design's STARTPULSE register and three
//! seconds of unrelated UART/observer logic.
use ecp5_emu::{
    compile::{self, Board, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Builder, DtrConfig, Options, Strength, Time, DTR_CONVERSION_PS},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

fn compile_hy04(code: Option<u8>) -> (Compiled, Reference, Vec<Reference>) {
    static DEVICE: OnceLock<Arc<ConfiguredDevice>> = OnceLock::new();
    let device = Arc::clone(DEVICE.get_or_init(|| {
        let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None).unwrap();
        let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
        let image = Arc::new(
            ConfigurationImage::load(&evidence.join("hy04_dtr/hy04.config"), &db, None).unwrap(),
        );
        Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        )
    }));
    let id = device.model().resource_id("LFE5U-85F/R94C22/DTR").unwrap();
    let ports = device.model().resource_ports(id);
    let port =
        |name: &str| Reference::Wire(ports.iter().find(|p| p.name == name).unwrap().wire.clone());
    let start = port("STARTPULSE");
    let outputs: Vec<_> = (0..8).map(|bit| port(&format!("DTROUT{bit}"))).collect();
    let compiled = compile::compile_on_board(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(start.clone(), Logic::Zero)],
            probes: outputs.clone(),
            options: Options {
                trace_capacity: 1000,
                ..Options::default()
            },
        },
        &Board {
            dtr_code: code,
            ..Board::default()
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        compiled
            .report
            .resources
            .iter()
            .filter(|r| r.family == "DTR")
            .count(),
        1
    );
    (compiled, start, outputs)
}

fn bus(compiled: &Compiled, outputs: &[Reference]) -> Vec<Logic> {
    outputs.iter().map(|p| compiled.value(p).unwrap()).collect()
}
fn byte(code: u8) -> Vec<Logic> {
    (0..8)
        .map(|bit| Logic::from_bool(code & (1 << bit) != 0))
        .collect()
}

struct BoardConversion {
    width: u64,
    changes: Vec<(u64, u8)>,
}

fn board_conversions() -> Vec<BoardConversion> {
    let mut conversions = Vec::new();
    let mut active = None;
    for line in include_str!("../docs/inventory/LFE5U-85F/hy04_dtr/observations.log").lines() {
        if line.starts_with("Y4B") {
            let kind = u8::from_str_radix(&line[11..12], 16).unwrap();
            active = matches!(kind, 1..=4 | 9).then(|| BoardConversion {
                width: u64::from_str_radix(&line[28..32], 16).unwrap(),
                changes: Vec::new(),
            });
        } else if line.starts_with("Y4C") {
            if let Some(conversion) = &mut active {
                conversion.changes.push((
                    u64::from_str_radix(&line[11..15], 16).unwrap(),
                    u8::from_str_radix(&line[15..17], 16).unwrap(),
                ));
            }
        } else if line.starts_with("Y4E") {
            if let Some(conversion) = active.take() {
                conversions.push(conversion);
            }
        }
    }
    assert_eq!(conversions.len(), 1995);
    conversions
}

fn measured_post_fall_median_ps() -> u64 {
    // Independent extraction from the frozen capture, excluding startup replays.
    let mut samples: Vec<_> = board_conversions()
        .iter()
        .map(|c| c.changes.last().unwrap().0 - 16 - c.width)
        .collect();
    samples.sort_unstable();
    (2 * samples[samples.len() / 2] - 5) * 20_000
}

#[test]
fn hy04_board_count_requires_zero_until_done_fallback() {
    let mut skew_count = 0;
    let mut first_offsets = Vec::new();
    let mut increments = Vec::new();
    for conversion in board_conversions() {
        let mut stable = Vec::new();
        for (j, &(t, code)) in conversion.changes.iter().enumerate() {
            if j + 1 < conversion.changes.len() && conversion.changes[j + 1].0 - t == 1 {
                let previous = conversion.changes[j - 1].1;
                let next = conversion.changes[j + 1].1;
                assert_eq!(next, previous + 1);
                assert_eq!(code & !(previous | next), 0);
                assert_eq!(code & (previous & next), previous & next);
                skew_count += 1;
            } else {
                stable.push((t, code));
            }
        }
        assert_eq!(
            stable.iter().map(|&(_, c)| c).collect::<Vec<_>>(),
            (0..=0x1E).chain([0x9E]).collect::<Vec<_>>()
        );
        first_offsets.push(stable[1].0 - 16 - conversion.width);
        increments.extend(stable[1..31].windows(2).map(|pair| pair[1].0 - pair[0].0));
    }
    first_offsets.sort_unstable();
    increments.sort_unstable();
    assert_eq!(skew_count, 173);
    assert_eq!(
        (first_offsets[0], first_offsets[997], first_offsets[1994]),
        (27, 88, 101)
    );
    assert_eq!(
        (
            increments[0],
            increments[increments.len() / 2],
            *increments.last().unwrap()
        ),
        (24, 25, 25)
    );
    // The pooled first count is much later than the 24/25-tick period.
    // A floor(time-since-fall / step) cannot fit.
    assert!(first_offsets[first_offsets.len() / 2] - increments[increments.len() / 2] > 2);
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hy04_config_initial_conversion_timing_final_code_and_profile_override() {
    assert_eq!(DTR_CONVERSION_PS, measured_post_fall_median_ps());
    assert!(
        include_str!("../docs/inventory/LFE5U-85F/hy04_dtr/hy04.config")
            .contains("enum: DTR.MODE DTR")
    );
    for (profile, code) in [(None, 0x9E), (Some(0xA1), 0xA1)] {
        let (mut compiled, start, outputs) = compile_hy04(profile);
        assert_eq!(bus(&compiled, &outputs), byte(code));
        // Request.drives adds a resolved driver; it does not disconnect the
        // routed FF. Isolate only DTR for the conversion stimulus, using the
        // same configured device, resource and exact native port wires.
        let device = Arc::clone(compiled.kernel.device());
        let resource = device.model().resource_id("LFE5U-85F/R94C22/DTR").unwrap();
        let mut builder = Builder::new(device).unwrap();
        let Reference::Wire(start_wire) = &start else {
            unreachable!()
        };
        let input = builder.signal(start_wire.clone()).unwrap();
        let drive = builder
            .external_driver(input, "native-dtr-start", Strength::Strong, Logic::Zero)
            .unwrap();
        let signals: Vec<_> = outputs
            .iter()
            .map(|output| {
                let Reference::Wire(wire) = output else {
                    unreachable!()
                };
                builder.signal(wire.clone()).unwrap()
            })
            .collect();
        builder
            .dtr(DtrConfig {
                resource,
                start: input.into(),
                outputs: signals.try_into().unwrap(),
                code,
            })
            .unwrap();
        compiled.kernel = builder.build().unwrap();
        // Compiled::value resolves native wire references in the new kernel.
        for output in &outputs {
            let Reference::Wire(wire) = output else {
                unreachable!()
            };
            compiled
                .probes
                .insert(output.label(), compiled.kernel.signal(wire).unwrap());
        }
        let mut fast = FastSim::new(&compiled.kernel).unwrap();
        let agree = |fast: &FastSim, compiled: &Compiled| {
            let row: Vec<_> = outputs
                .iter()
                .map(|p| fast.value(compiled.probes[&p.label()]).unwrap())
                .collect();
            assert_eq!(row, bus(compiled, &outputs));
        };
        agree(&fast, &compiled);
        let rise = 1_000_000;
        let fall = rise + 40_000;
        compiled
            .kernel
            .schedule(Time::from_ps(rise), drive, Logic::One)
            .unwrap();
        compiled
            .kernel
            .schedule(Time::from_ps(fall), drive, Logic::Zero)
            .unwrap();
        compiled.kernel.run_until(Time::from_ps(rise - 1)).unwrap();
        assert_eq!(bus(&compiled, &outputs), byte(code));
        fast.run_until(Time::from_ps(rise - 1)).unwrap();
        agree(&fast, &compiled);
        compiled.kernel.run_until(Time::from_ps(rise)).unwrap();
        fast.set_at(Time::from_ps(rise), &[(drive, true)]).unwrap();
        agree(&fast, &compiled);
        fast.set_at(Time::from_ps(fall), &[(drive, false)]).unwrap();
        assert_eq!(bus(&compiled, &outputs), byte(0));
        // A real one-tick conversion at the pooled final-offset median.
        // Compare each observer acquisition after the fall. A recorded change
        // at k means a physical edge in (k-3, k-2] ticks; sampling at k-2
        // tests the same boundary without inventing sub-tick bus coherence.
        let record = board_conversions()
            .into_iter()
            .find(|c| c.width == 1 && c.changes.last().unwrap().0 - 17 == 821)
            .unwrap();
        let persistent: Vec<_> = record
            .changes
            .iter()
            .enumerate()
            .filter(|&(j, &(t, _))| {
                j + 1 == record.changes.len() || record.changes[j + 1].0 - t > 1
            })
            .map(|(_, &(_, c))| c)
            .collect();
        assert_eq!(persistent, (0..=0x1E).chain([0x9E]).collect::<Vec<_>>());
        let board_fall = 16 + record.width;
        let board_done = record.changes.last().unwrap().0;
        let mut observed = 0;
        for tick in board_fall + 2..=board_done + 1 {
            for &(offset, value) in &record.changes {
                if offset <= tick {
                    observed = value;
                }
            }
            if tick == board_done {
                // The board's final change is in (818,819] ticks from fall.
                // The policy median 818.5 lies inside that observer interval.
                assert!(DTR_CONVERSION_PS > (board_done - board_fall - 3) * 40_000);
                assert!(DTR_CONVERSION_PS <= (board_done - board_fall - 2) * 40_000);
                compiled
                    .kernel
                    .run_until(Time::from_ps(fall + DTR_CONVERSION_PS - 1))
                    .unwrap();
                fast.run_until(Time::from_ps(fall + DTR_CONVERSION_PS - 1))
                    .unwrap();
                agree(&fast, &compiled);
                assert_eq!(bus(&compiled, &outputs), byte(0));
                compiled
                    .kernel
                    .run_until(Time::from_ps(fall + DTR_CONVERSION_PS))
                    .unwrap();
                fast.run_until(Time::from_ps(fall + DTR_CONVERSION_PS))
                    .unwrap();
                agree(&fast, &compiled);
                assert_eq!(bus(&compiled, &outputs), byte(code));
            }
            let acquisition = fall + (tick - board_fall - 2) * 40_000;
            compiled
                .kernel
                .run_until(Time::from_ps(acquisition))
                .unwrap();
            fast.run_until(Time::from_ps(acquisition)).unwrap();
            agree(&fast, &compiled);
            let actual = bus(&compiled, &outputs);
            assert_eq!(
                actual[7],
                Logic::from_bool(observed & 0x80 != 0),
                "observer tick {tick}"
            );
            // Selected fallback intentionally maps the board's variable low
            // counts to zero; compare the full byte as soon as done is set.
            assert_eq!(actual, byte(if observed & 0x80 != 0 { code } else { 0 }));
        }
        println!("HY04 native: initial/final {code:02X}, 00 at rise; sampled board done sequence matches, completion at fall + {DTR_CONVERSION_PS} ps");
    }
}
