//! HC02's exact board configs, probed at DCS ports rather than through UART.
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time, Transition},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Instant,
};

const PORTS: [&str; 6] = ["CLK0", "CLK1", "SEL0", "SEL1", "MODESEL", "DCSOUT"];
const END_PS: u64 = 350_000_000;

fn compile_hc02(variant: &str) -> (Compiled, Vec<Reference>) {
    compile_hc02_scope(variant, false)
}

fn compile_hc02_scope(variant: &str, downstream: bool) -> (Compiled, Vec<Reference>) {
    static MODEL: OnceLock<Arc<DeviceModel>> = OnceLock::new();
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
    let db = Database::discover(None).unwrap();
    let model =
        Arc::clone(MODEL.get_or_init(|| Arc::new(DeviceModel::load(&db, &evidence).unwrap())));
    let image = Arc::new(
        ConfigurationImage::load(
            &evidence.join(format!("hc02_dcs/{variant}.config")),
            &db,
            None,
        )
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
    let mut probes = Vec::new();
    for site in 0..2 {
        let row = if site == 0 { 22 } else { 70 };
        let id = model
            .resource_id(&format!("LFE5U-85F/R{row}C66/DCS{site}"))
            .unwrap();
        let ports = model.resource_ports(id);
        probes.extend(PORTS.iter().map(|name| {
            Reference::Wire(ports.iter().find(|p| p.name == *name).unwrap().wire.clone())
        }));
    }
    if downstream {
        // UART depends on all physical edge-observer counters. Pull those
        // into the observed plan without executing unused PLC registers.
        probes.push(Reference::Pin("L4".into()));
    }
    let compiled = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(Reference::Pin("G2".into()), Logic::Zero)],
            probes: probes.clone(),
            options: Options {
                trace_capacity: 100_000,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        compiled
            .report
            .resources
            .iter()
            .filter(|r| r.family == "DCS")
            .count(),
        2
    );
    (compiled, probes)
}

fn run(compiled: &mut Compiled) -> Vec<Transition> {
    compiled.kernel.drain_trace();
    let reference = compiled.drivers["pin:G2"];
    for n in 1..=END_PS / 20_000 {
        compiled
            .kernel
            .schedule(
                Time::from_ps(n * 20_000),
                reference,
                Logic::from_bool(n % 2 == 1),
            )
            .unwrap();
    }
    compiled.kernel.run_until(Time::from_ps(END_PS)).unwrap();
    assert_eq!(compiled.kernel.statistics().trace_overwrites, 0);
    compiled.kernel.drain_trace()
}

fn edges(trace: &[Transition], port: &Reference) -> Vec<(Time, Logic)> {
    let Reference::Wire(wire) = port else {
        unreachable!()
    };
    trace
        .iter()
        .filter(|t| &t.wire == wire && t.previous.bit().is_some() && t.value.bit().is_some())
        .map(|t| (t.time, t.value))
        .collect()
}

fn control(v: &[Logic]) -> (u8, bool) {
    (
        u8::from(v[2] == Logic::One) + 2 * u8::from(v[3] == Logic::One),
        v[4] == Logic::One,
    )
}

// Independent documented truth table: 2 and 3 stand for CLK0 and CLK1.
fn table(mode: usize, sel: u8, bypass: bool, clocks: [Logic; 2]) -> Logic {
    let rows = [[1, 2, 3, 1], [0, 2, 3, 0], [0, 2, 0, 0], [1, 1, 3, 1]];
    let entry = if bypass {
        [0, 2, 3, 0][sel as usize]
    } else {
        rows[mode][sel as usize]
    };
    match entry {
        0 => Logic::Zero,
        1 => Logic::One,
        2 => clocks[0],
        _ => clocks[1],
    }
}

fn snapshots(
    initial: &[Logic],
    trace: &[Transition],
    ports: &[Reference],
) -> Vec<(Time, Vec<Logic>)> {
    let mut by_time: BTreeMap<Time, Vec<(usize, Logic)>> = BTreeMap::new();
    for tr in trace {
        if let Some(n) = ports
            .iter()
            .position(|p| matches!(p,Reference::Wire(w) if *w==tr.wire))
        {
            by_time.entry(tr.time).or_default().push((n, tr.value));
        }
    }
    let mut current = initial.to_vec();
    by_time
        .into_iter()
        .map(|(t, changes)| {
            for (n, v) in changes {
                current[n] = v;
            }
            (t, current.clone())
        })
        .collect()
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc02_dynamic_port_edges_follow_two_new_active_edges() {
    let started = Instant::now();
    let (mut compiled, probes) = compile_hc02("dynamic");
    let initial: Vec<_> = probes.iter().map(|p| compiled.value(p).unwrap()).collect();
    let trace = run(&mut compiled);
    let mut switches = [0; 2];
    for site in 0..2 {
        let ports = &probes[site * 6..site * 6 + 6];
        let snapshots = snapshots(&initial[site * 6..site * 6 + 6], &trace, ports);
        let source = [edges(&trace, &ports[0]), edges(&trace, &ports[1])];
        assert!(
            source.iter().all(|s| s.len() > 3000),
            "missing source trace"
        );
        let out = edges(&trace, &ports[5]);
        let mut previous = control(&initial[site * 6..site * 6 + 6]);
        for (t, values) in snapshots {
            let next = control(&values);
            if next == previous {
                continue;
            }
            let (old, old_bypass) = previous;
            let (new, new_bypass) = next;
            previous = next;
            if old_bypass
                || new_bypass
                || ![1, 2].contains(&old)
                || ![1, 2].contains(&new)
                || old == new
            {
                continue;
            }
            let active = if site == 0 { Logic::One } else { Logic::Zero };
            let a = &source[old as usize - 1];
            let b = &source[new as usize - 1];
            // At an exactly coincident SEL/clock timestamp the routed capture
            // waves may deliver SEL after that edge. Both boundary orders are
            // admissible; all other edges must match at exact rational time.
            let first = a
                .iter()
                .position(|&(time, v)| time >= t && v == active)
                .unwrap();
            let candidates = if a[first].0 == t {
                vec![first, first + 2]
            } else {
                vec![first]
            };
            let lo = t.checked_sub(Time::from_ps(1_000_000)).unwrap();
            let hi = t.checked_add(Time::from_ps(2_000_000)).unwrap();
            let actual: Vec<_> = out
                .iter()
                .copied()
                .filter(|&(time, _)| time >= lo && time <= hi)
                .collect();
            let fits = candidates
                .into_iter()
                .filter(|&index| {
                    let stop = a[index].0;
                    let release = b
                        .iter()
                        .filter(|&&(time, v)| time > stop && v == active)
                        .nth(1)
                        .unwrap()
                        .0;
                    let expected: Vec<_> = a
                        .iter()
                        .copied()
                        .filter(|&(time, _)| time >= lo && time <= stop)
                        .chain(
                            b.iter()
                                .copied()
                                .filter(|&(time, _)| time > release && time <= hi),
                        )
                        .collect();
                    expected == actual
                })
                .count();
            assert!(
                fits > 0,
                "DCS{site} {old}->{new} at {t}: output edges {actual:?}"
            );
            switches[site] += 1;
        }
        assert_eq!(
            switches[site], 2,
            "both startup/capture direction changes must execute"
        );
    }
    println!(
        "HC02 dynamic: 2 DCS, switches {:?}, exact source/output edge fits; 350 us in {:.2}s",
        switches,
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc02_static_ports_match_the_table_at_every_settled_time() {
    let started = Instant::now();
    let (mut compiled, probes) = compile_hc02("static");
    let initial: Vec<_> = probes.iter().map(|p| compiled.value(p).unwrap()).collect();
    let trace = run(&mut compiled);
    let mut checked = [0; 2];
    let mut switches = [0; 2];
    for site in 0..2 {
        let ports = &probes[site * 6..site * 6 + 6];
        let mut previous = control(&initial[site * 6..site * 6 + 6]);
        for (time, values) in snapshots(&initial[site * 6..site * 6 + 6], &trace, ports) {
            let (sel, bypass) = control(&values);
            assert_eq!(
                values[5],
                table(site + 2, sel, bypass, [values[0], values[1]]),
                "DCS{site} SEL={sel} MODESEL={bypass} at {time}"
            );
            checked[site] += 1;
            switches[site] += usize::from((sel, bypass) != previous);
            previous = (sel, bypass);
        }
        assert!(
            checked[site] > 3000 && switches[site] >= 2,
            "vacuous static check"
        );
    }
    println!(
        "HC02 static: settled table samples {:?}, control changes {:?}; 350 us in {:.2}s",
        checked,
        switches,
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hc02_dynamic_fast_edges_match_the_event_kernel() {
    fast_twin("dynamic");
}

#[test]
#[ignore = "requires the native image reader, a local Trellis database and clang"]
fn hc02_static_fast_edges_match_the_event_kernel() {
    fast_twin("static");
}

fn fast_twin(variant: &str) {
    use ecp5_emu::fast::FastSim;
    let (mut compiled, mut ports) = compile_hc02_scope(variant, true);
    ports.truncate(12);
    let mut fast = FastSim::new(&compiled.kernel).unwrap();
    let mut signals: Vec<_> = ports.iter().map(|p| compiled.probes[&p.label()]).collect();
    let mut references = ports.clone();
    let model = compiled.kernel.device().model();
    let clocks = [signals[5], signals[11]].map(|s| fast.program().slot(s).unwrap().0);
    let mut downstream = vec![];
    // Find the actual board counter FFs clocked by DCSOUT, using the collapsed
    // routed clock slot, without depending on nextpnr's placement or HDL names.
    for (id, _) in model.resources().filter(|(_, r)| r.family == "PLC2") {
        for node in model.nodes(id).filter(|n| n.kind == "TRELLIS_FF") {
            let ports = model.node_ports(node.id);
            let clock = ports.iter().find(|p| p.name == "CLK").unwrap();
            let Some(clock) = compiled.kernel.signal(&clock.wire) else {
                continue;
            };
            let Some((slot, _)) = fast.program().slot(clock) else {
                continue;
            };
            if !clocks.contains(&slot) {
                continue;
            }
            let q = ports.iter().find(|p| p.name == "Q").unwrap();
            let Some(signal) = compiled.kernel.signal(&q.wire) else {
                continue;
            };
            downstream.push((signal, Reference::Wire(q.wire.clone())));
        }
    }
    assert!(
        downstream.len() >= 16,
        "missing board DCS edge counters: {}",
        downstream.len()
    );
    for (signal, reference) in downstream {
        compiled.kernel.watch(signal).unwrap();
        signals.push(signal);
        references.push(reference);
    }
    let initial: Vec<_> = signals
        .iter()
        .map(|&s| compiled.kernel.value(s).unwrap())
        .collect();
    // Trace stores floor(picoseconds). HC02 has one PLL and a 40 ns input;
    // every edge is on its period/(16*b) grid, whose points are >1 ps apart.
    // Recover the unique exact rational grid point, rather than rounding the
    // event kernel's expected edges down to the trace representation.
    assert_eq!(fast.program().plls.len(), 1);
    let denominator = 16 * fast.program().plls[0].b;
    assert!(denominator < 40_000);
    let exact_time = |floor: u64| {
        let ticks = (floor * denominator).div_ceil(40_000);
        let time = Time::new(ticks * 40_000, denominator).unwrap();
        let (n, d) = time.ratio();
        assert_eq!(n / d, floor, "trace instant is off HC02's exact PLL grid");
        time
    };
    fast.trace(&signals).unwrap();
    fast.clock(compiled.drivers["pin:G2"], Time::from_ps(40_000))
        .unwrap();
    // Long enough to exercise fast_schedule after PLL acquisition, rather
    // than only the host's instant loop.
    fast.run_edges_sampled(END_PS / 20_000, &[signals[5]])
        .unwrap();
    assert_eq!(fast.now().unwrap(), Time::from_ps(END_PS));
    let actual = fast.take_trace();
    let expected = run(&mut compiled);
    let mut counts = vec![];
    for (n, reference) in references.iter().enumerate() {
        let mut previous = initial[n];
        let mut changes = vec![];
        for (&time, row) in actual
            .times_ps
            .iter()
            .zip(actual.rows.chunks_exact(actual.width))
        {
            let value = Logic::from_bool(row[n] == 1);
            if value != previous {
                changes.push((exact_time(time), value));
                previous = value;
            }
        }
        let wanted = edges(&expected, reference);
        let first = changes.iter().zip(&wanted).position(|(a, b)| a != b);
        assert!(
            first.is_none() && changes.len() == wanted.len(),
            "{variant} {reference:?}: {} fast / {} event edges; first mismatch {:?}",
            changes.len(),
            wanted.len(),
            first.map(|i| (i, changes[i], wanted[i]))
        );
        counts.push(changes.len());
    }
    println!("HC02 {variant}: DCSOUT and {} downstream FFs, exact edges through {END_PS} ps; edges {counts:?}", signals.len()-12);
}
