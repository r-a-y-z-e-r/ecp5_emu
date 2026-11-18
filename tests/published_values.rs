//! Pin the source hover's value source, not just the output probes: the same
//! board/Request and overlay signal mapping as live::work, followed by the
//! FastSim::step + Reader path used by its publish closure.
//! Run: FAST_FOLD=cones cargo test --release --test published_values -- --ignored --nocapture --test-threads=1
use ecp5_emu::{
    board,
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fabric, fabric_overlay,
    fast::{FastSim, Reader},
    image::ConfigurationImage,
    kernel::{FastOptions, FastProgram, SignalId, StartsUnknown, Strength, Time},
    layout, live,
    logic::Logic,
    model::{topology::Wire, DeviceModel, Direction},
    names::Names,
    peripherals::Attached,
    prepared::{load_catalogue, ConfiguredDevice},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

const PERIOD_PS: u64 = board::CLOCK_PERIOD_NS * 1000;
const CHECKPOINTS: [u64; 4] = [1, 7, 100, 1000];

// There is no public Kernel reader accessor. Decode only the leading fields
// of the construction cache (kernel/cache.rs::Core), with its envelope/version
// checked first. This is actual post-collapse reader metadata, not an inference
// from an empty nextpnr port or from fast-mode constant folding. A schema change
// fails closed; no unsafe access and no emulator source changes are needed.
#[derive(Deserialize)]
struct PlanSignal {
    wire: Wire,
    _value: Logic,
    _drivers: Vec<usize>,
    combinational: Vec<usize>,
    sequential: Vec<usize>,
    clock_groups: Vec<usize>,
    read: bool,
    lut_writes: Vec<usize>,
    system: bool,
    ebr: Vec<usize>,
    multipliers: Vec<usize>,
    plls: Vec<usize>,
    iologic: Vec<usize>,
    dcs: Vec<usize>,
    dtrs: Vec<usize>,
    watched: bool,
}

impl PlanSignal {
    fn has_reader(&self) -> bool {
        self.system
            || [
                &self.combinational,
                &self.sequential,
                &self.clock_groups,
                &self.lut_writes,
                &self.ebr,
                &self.multipliers,
                &self.plls,
                &self.iologic,
                &self.dcs,
                &self.dtrs,
            ]
            .iter()
            .any(|list| !list.is_empty())
    }
}

#[derive(Deserialize)]
struct PlanDriver {
    signal: usize,
    _value: Logic,
    _strength: Strength,
    _label: String,
    _external: bool,
}

#[derive(Clone, Copy, Deserialize)]
enum PlanInput {
    Constant(Logic),
    Signal(usize),
    Inverted(usize),
}

// Exact postcard variant/field order of kernel::Function. Non-carry variants
// are decoded solely to reach the next gate in this schema.
#[allow(dead_code)]
#[derive(Deserialize)]
enum PlanFunction {
    Buffer(PlanInput),
    Receiver(PlanInput),
    Mux {
        select: PlanInput,
        low: PlanInput,
        high: PlanInput,
    },
    Lut4 {
        node: usize,
        inputs: [PlanInput; 4],
    },
    Carry {
        node: usize,
        inputs: [PlanInput; 4],
        cin: PlanInput,
        inject: bool,
        carry_output: bool,
    },
}

#[derive(Deserialize)]
struct PlanGate {
    function: PlanFunction,
    driver: usize,
}

#[derive(Deserialize)]
struct PlanPrefix {
    signals: Vec<PlanSignal>,
    drivers: Vec<PlanDriver>,
    combinational: Vec<PlanGate>,
}

impl PlanPrefix {
    fn load(key: &[u8; 32]) -> Self {
        let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir()
            .join("ecp5_emu_compile")
            .join(format!("{hex}.bin"));
        let bytes = std::fs::read(path).expect("reader audit requires the construction plan cache");
        assert!(bytes.len() >= 84);
        assert_eq!(&bytes[..8], b"ECP5PLN1");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 4);
        assert_eq!(&bytes[52..84], key);
        let payload = &bytes[84..];
        assert_eq!(
            u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            payload.len() as u64
        );
        assert_eq!(&Sha256::digest(payload)[..], &bytes[20..52]);
        let (plan, _) = postcard::take_from_bytes(payload).expect("construction prefix schema");
        plan
    }

    fn unread_slots(&self, compiled: &Compiled, program: &FastProgram) -> HashSet<usize> {
        let mut readers: HashMap<usize, bool> = HashMap::new();
        for (index, signal) in self.signals.iter().enumerate() {
            let id = compiled.kernel.signal(&signal.wire).unwrap();
            // Ensure the cached graph really belongs to this compiled kernel.
            assert_eq!(serde_json::to_value(id).unwrap(), index);
            assert_eq!(signal.read, signal.has_reader());
            if let Some((slot, _)) = program.slot(id) {
                // Alias readers were transferred to the root by collapse_buffers.
                // Aggregate every signal sharing a slot, not just the hovered wire.
                *readers.entry(slot).or_default() |= signal.read || signal.watched;
            }
        }
        readers
            .into_iter()
            .filter_map(|(slot, read)| (!read).then_some(slot))
            .collect()
    }

    fn carry_details(&self, wire: &Wire, compiled: &Compiled, model: &DeviceModel) -> String {
        let read = |input| match input {
            PlanInput::Constant(value) => value,
            PlanInput::Signal(index) | PlanInput::Inverted(index) => {
                let signal = compiled.kernel.signal(&self.signals[index].wire).unwrap();
                let value = compiled.kernel.value(signal).unwrap();
                if matches!(input, PlanInput::Inverted(_)) {
                    value.invert()
                } else {
                    value
                }
            }
        };
        for gate in &self.combinational {
            if &self.signals[self.drivers[gate.driver].signal].wire != wire {
                continue;
            }
            if let PlanFunction::Carry {
                node,
                inputs,
                cin,
                inject,
                carry_output,
            } = gate.function
            {
                // Matches kernel/cache.rs::IdentityGuard's node ordering.
                let node = model
                    .resources()
                    .filter(|(_, resource)| resource.family == "PLC2")
                    .flat_map(|(id, _)| model.nodes(id).map(|node| node.id))
                    .nth(node)
                    .unwrap();
                let inputs = inputs.map(read);
                let propagate = Logic::table(compiled.kernel.state().lut(node).unwrap(), &inputs);
                return format!(
                    "carry_output={carry_output}, inject={inject}, inputs={inputs:?}, propagate={propagate:?}, cin={:?}",
                    read(cin)
                );
            }
        }
        "not a direct carry output".into()
    }
}

fn advance_event(compiled: &mut Compiled, from: u64, to: u64) {
    let clock = compiled.drivers[&Reference::Pin(board::CLOCK.into()).label()];
    for edge in from + 1..=to {
        let time = Time::from_ps(edge * PERIOD_PS / 2);
        compiled
            .kernel
            .schedule_batch(time, &[(clock, Logic::from_bool(edge % 2 == 1))], None)
            .unwrap();
        compiled.kernel.run_until(time).unwrap();
    }
}

fn setup(
    design: &str,
) -> (
    Compiled,
    fabric_overlay::Overlay,
    Arc<DeviceModel>,
    PlanPrefix,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/reference")
        .join(design);
    assert!(root.join("top.config").is_file(), "build {design} first");
    let architecture = live::architecture();
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &architecture).unwrap());
    let image = Arc::new(ConfigurationImage::load(&root.join("top.config"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &architecture).unwrap(),
        )
        .unwrap(),
    );
    // Keep this in sync with live::work: no extra watches or internal probes,
    // which would disable folding and make the test vacuous.
    let used: BTreeMap<_, _> = board::used_balls(&device).unwrap().into_iter().collect();
    let attached = Attached::new(&used);
    assert!(
        attached.input_balls().is_empty(),
        "{design} now needs peripheral answers; mirror live::answer before comparing"
    );
    let pin = |ball: &str| Reference::Pin(ball.into());
    let mut drives = vec![(pin(board::CLOCK), Logic::Zero)];
    drives.extend(
        board::idle_inputs(&device, [false; 4])
            .unwrap()
            .into_iter()
            .map(|(ball, level)| (pin(ball), Logic::from_bool(level))),
    );
    let probes = used
        .iter()
        .filter(|(_, direction)| direction.drives())
        .map(|(ball, _)| pin(ball))
        .chain(attached.balls().into_iter().map(pin))
        .collect();
    let request = Request {
        package: board::PACKAGE.into(),
        scope: Scope::Observed,
        startup: Startup::Configured,
        drives,
        probes,
        options: ecp5_emu::kernel::Options {
            trace_capacity: 0,
            ..Default::default()
        },
    };
    let board = compile::Board::default();
    let key = compile::cache_key(&device, &request, &board).unwrap();
    let compiled = compile::compile_on_board(device, request, &board).unwrap();
    let plan = PlanPrefix::load(&key);
    let fabric = fabric::cached(&model, &architecture);
    let report = &compiled.report;
    let overlay = fabric_overlay::build(
        &fabric,
        &model,
        &report.routes,
        &report.constants,
        &layout::logic_usage(&model, report),
        &report.fields,
    );
    (compiled, overlay, model, plan)
}

#[derive(Default)]
struct Counts {
    equal: usize,
    differing: usize,
    allowed: usize,
}

fn published(logic: Logic) -> u8 {
    match logic {
        Logic::Zero => 0,
        Logic::One => 1,
        Logic::X | Logic::Z => 2,
    }
}

// nextpnr's carry-chain helper has a carry output, but no connected sum F
// or carry-in. Classify it from fixture metadata, never by the observed
// mismatch or by hard-coded coordinates. This diagnostic tag does not grant
// an exception: the independent no-reader + event-X rule below does that.
fn unused_carry_sums(design: &str) -> HashSet<Wire> {
    let routed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/reference")
        .join(design)
        .join("top_routed.json");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(routed).unwrap()).unwrap();
    let top = json["modules"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    top["cells"]
        .as_object()
        .unwrap()
        .values()
        .filter(|cell| {
            cell["parameters"]["MODE"] == "CCU2"
                && cell["parameters"]["CCU2_INJECT1"] == "NO"
                && cell["connections"]["F"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                && cell["connections"]["FCI"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
        })
        .map(|cell| {
            let bel = cell["attributes"]["NEXTPNR_BEL"].as_str().unwrap();
            let mut parts = bel.split('/');
            let col = parts
                .next()
                .unwrap()
                .strip_prefix('X')
                .unwrap()
                .parse()
                .unwrap();
            let row = parts
                .next()
                .unwrap()
                .strip_prefix('Y')
                .unwrap()
                .parse()
                .unwrap();
            let part = parts.next().unwrap().strip_prefix("SLICE").unwrap();
            let (slice, lut) = part.split_once(".K").unwrap();
            let index = u32::from(slice.as_bytes()[0] - b'A') * 2 + lut.parse::<u32>().unwrap();
            Wire {
                location: ecp5_emu::model::topology::Location { row, col },
                name: format!("F{index}_SLICE"),
            }
        })
        .collect()
}

// These physical classes overlap freshness classes. Map by fast slot so all
// overlay routing aliases of an output are counted, including inversions.
fn output_slots(
    compiled: &Compiled,
    model: &DeviceModel,
    program: &FastProgram,
    family: &str,
) -> HashSet<usize> {
    compiled
        .report
        .resources
        .iter()
        .filter(|resource| resource.family == family)
        .flat_map(|resource| model.resource_ports(model.resource_id(&resource.id).unwrap()))
        .filter(|port| matches!(port.direction, Direction::Output | Direction::InOut))
        .filter_map(|port| compiled.kernel.signal(&port.wire))
        .filter_map(|signal| program.slot(signal).map(|(slot, _)| slot))
        .collect()
}

fn check(design: &str) {
    let since = Instant::now();
    assert_ne!(std::env::var("FAST_FOLD").as_deref(), Ok("single"));
    let (mut compiled, overlay, model, plan) = setup(design);
    let options = FastOptions {
        threads: 1,
        collapse: 12,
        thread_min: usize::MAX,
        barrier_lines: usize::MAX,
    };
    // Same warm-start retry/whole-edge semantics as live::start_fast. No
    // FastSim tuning here: hold cone folding on for all four checkpoints.
    let mut warm_edges = 0;
    let (mut next_try, mut wait) = (0, 2);
    let mut fast = loop {
        if warm_edges >= next_try {
            match FastSim::with_options(&compiled.kernel, &options) {
                Ok(fast) => break fast,
                Err(error)
                    if error.downcast_ref::<StartsUnknown>().is_some() && warm_edges < 100_000 =>
                {
                    next_try = warm_edges + wait;
                    wait *= 2;
                }
                Err(error) => panic!("{design}: {error:#}"),
            }
        }
        advance_event(&mut compiled, warm_edges, warm_edges + 2);
        warm_edges += 2;
    };
    let no_fold = compiled
        .kernel
        .fast_program_with(&FastOptions {
            collapse: 0,
            ..options
        })
        .unwrap();
    let lit_signals: Vec<Option<SignalId>> = overlay
        .values
        .iter()
        .map(|wire| {
            let wire = model
                .clock_segment(wire)
                .unwrap()
                .unwrap_or_else(|| wire.clone());
            compiled.kernel.signal(&wire)
        })
        .collect();
    assert!(!lit_signals.is_empty());
    let program = fast.program();
    let pads = output_slots(&compiled, &model, program, "PIO");
    let ebr = output_slots(&compiled, &model, program, "EBR");
    let unused_sums = unused_carry_sums(design);
    let unread = plan.unread_slots(&compiled, program);
    let is_unread: Vec<_> = lit_signals
        .iter()
        .map(|signal| {
            signal
                .and_then(|signal| program.slot(signal))
                .is_some_and(|(slot, _)| unread.contains(&slot))
        })
        .collect();
    let routed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/reference")
        .join(design)
        .join("top_routed.json");
    let names = Names::load(&routed, &model, board::PACKAGE).unwrap();
    let mut names_by_net: HashMap<u32, Vec<String>> = HashMap::new();
    // Check every resolvable HDL name, including hidden/generated aliases and
    // package ports, through the very resolver used by the source index.
    for (name, _) in names.observations() {
        for net in names.overlay_nets(name, &model, board::PACKAGE, &overlay) {
            names_by_net.entry(net).or_default().push(name.into());
        }
    }
    let tags: Vec<_> = lit_signals
        .iter()
        .enumerate()
        .map(|(i, &signal)| {
            let Some(signal) = signal else {
                return vec!["outside-event-plan"];
            };
            let Some((slot, _)) = program.slot(signal) else {
                return vec!["missing-fast-slot"];
            };
            let mut tags = vec![if !no_fold.fresh(signal) {
                "on-demand"
            } else if !program.fresh(signal) {
                "folded"
            } else {
                "edge-fresh"
            }];
            if pads.contains(&slot) {
                tags.push("pad-output");
            }
            if ebr.contains(&slot) {
                tags.push("EBR-output");
            }
            if unused_sums.contains(&overlay.values[i]) {
                tags.push("unused-carry-sum");
            }
            if is_unread[i] {
                tags.push("unread");
            }
            tags
        })
        .collect();
    assert!(tags.iter().any(|tags| tags.contains(&"folded")));
    assert!(tags.iter().any(|tags| tags.contains(&"on-demand")));
    let clock = compiled.drivers[&Reference::Pin(board::CLOCK.into()).label()];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut reader = Reader::new(&lit_signals);
    let mut lit = Vec::new();
    let mut counts: BTreeMap<&str, Counts> = [
        "all",
        "folded",
        "on-demand",
        "edge-fresh",
        "pad-output",
        "EBR-output",
        "outside-event-plan",
        "missing-fast-slot",
        "unused-carry-sum",
        "unread",
    ]
    .into_iter()
    .map(|class| (class, Counts::default()))
    .collect();
    let mut from = 0;
    let mut examples = Vec::new();
    let mut unexpected = Vec::new();
    let mut coerced_z = 0;
    for cycle in CHECKPOINTS {
        fast.run_cycles_sampled(cycle - from, &[]).unwrap();
        advance_event(&mut compiled, warm_edges + from * 2, warm_edges + cycle * 2);
        let at = Time::from_ps((warm_edges / 2 + cycle) * PERIOD_PS);
        assert_eq!(fast.now().unwrap(), at);
        assert_eq!(compiled.kernel.now(), at);
        // Exactly the settle/read that live::work::publish uses. A plain
        // run_cycles/value comparison would read stale folded/on-demand slots.
        fast.step().unwrap();
        fast.read(&mut reader, &mut lit);
        assert_eq!(lit.len(), overlay.values.len());
        for (i, (&actual, &signal)) in lit.iter().zip(&lit_signals).enumerate() {
            let logic = signal.map(|s| compiled.kernel.value(s).unwrap());
            coerced_z += usize::from(logic == Some(Logic::Z));
            let expected = logic.map_or(2, published);
            let equal = actual == expected;
            // Only this semantic class is allowed. A missing slot, event Z,
            // binary disagreement, or any reader prevents the exception.
            // Neither coordinates nor the carry-helper metadata grant it.
            let allowed = !equal && is_unread[i] && logic == Some(Logic::X) && actual < 2;
            for &class in std::iter::once(&"all").chain(&tags[i]) {
                let count = counts.get_mut(class).unwrap();
                count.equal += usize::from(equal);
                count.differing += usize::from(!equal);
                count.allowed += usize::from(allowed);
            }
            if !equal && examples.len() < 12 {
                let detail = format!(
                    "cycle {} {:?} {:?}: published {actual}, event {logic:?}, no_reader={}, allowed={allowed}, overlay net {}, pieces {}, HDL names {:?}; {}",
                    warm_edges / 2 + cycle,
                    tags[i],
                    overlay.values[i],
                    is_unread[i],
                    overlay.nets[i],
                    overlay.pieces_of(overlay.nets[i]).len(),
                    names_by_net.get(&overlay.nets[i]).map_or(&[][..], Vec::as_slice),
                    plan.carry_details(&overlay.values[i], &compiled, &model)
                );
                examples.push(detail.clone());
                if !allowed {
                    unexpected.push(detail);
                }
            } else if !equal && !allowed && unexpected.len() < 12 {
                unexpected.push(format!(
                    "cycle {} {:?} {:?}: published {actual}, event {logic:?}, no_reader={}",
                    warm_edges / 2 + cycle,
                    tags[i],
                    overlay.values[i],
                    is_unread[i]
                ));
            }
        }
        from = cycle;
    }
    println!(
        "{design}: {} overlay entries, {} overlay nets, warm cycle {}, offsets {CHECKPOINTS:?}, folding=cones/12, {:.2}s",
        overlay.values.len(),
        overlay.nets.iter().collect::<HashSet<_>>().len(),
        warm_edges / 2,
        since.elapsed().as_secs_f64()
    );
    for (class, count) in &counts {
        println!(
            "{design} {class}: equal={} differing={} allowed={} unexpected={}",
            count.equal,
            count.differing,
            count.allowed,
            count.differing - count.allowed
        );
    }
    println!("{design}: event Z -> published x: {coerced_z}; differences: {examples:?}");
    assert_eq!(
        counts["all"].differing - counts["all"].allowed,
        0,
        "{design}: unexpected differences: {unexpected:#?}"
    );
}

#[test]
#[ignore = "release reference-design check; requires life, clang, native image reader and local Trellis database"]
fn life_published_overlay_values_match_event_kernel() {
    check("life");
}

#[test]
#[ignore = "release reference-design check; requires life_full, clang, native image reader and local Trellis database"]
fn life_full_published_overlay_values_match_event_kernel() {
    check("life_full");
}
