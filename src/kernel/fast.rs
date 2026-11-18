//! Fast mode: the execution plan as a two-valued, cycle-based C program.
//!
//! The event kernel keeps every routing segment, four-valued logic and delta
//! waves, so it can explain each transition. Fast mode keeps only what decides
//! the values: one slot per net (a collapsed buffer chain is one net), the
//! gates in dependency order, and the registers grouped by clock net. A step
//! settles the logic, applies asynchronous resets, captures every register
//! whose clock net saw its edge from the same settled state, and repeats until
//! no clock edge is left, so clocks derived from register outputs work too.
//! Independent captures share a test of their enable and synchronous reset;
//! inactive groups write nothing. Q dependencies retain reader-before-writer
//! order (cycles use temporaries), and predicates a capture can write are saved
//! before the wave. Thread-owned captures use the same grouping.
//!
//! It covers LUT4, carries, muxes, buffers, receivers, flip-flops, PIO,
//! IOLOGIC registers and DDR X1, LUTRAM, EBR, and PLL and OSC outputs. A
//! LUTRAM column keeps its sixteen cells in slots: its reader indexes them and
//! its write port stores into them at the clock edge. Gates that only the data
//! input of registers with an enable read are evaluated on demand, in the
//! capture, when one of those enables is 1. PLL and OSC outputs
//! are inputs of the program: the host (src/fast.rs) lays out their edges in
//! time as the event kernel does. Anything else in the plan, an X or Z where
//! the program would read it, or a net with two active drivers is refused with
//! the reason; the event kernel then remains the way to run that design.
//!
//! The settle after a capture, the work of every cycle, is folded and split.
//! A gate that only one other gate of it reads (no register, LUTRAM write,
//! EBR, clock or probe), or several within one register's logic, is folded
//! into the gates that read it, which become tables of up to twelve inputs
//! (`FAST_COLLAPSE`, `FAST_FOLD`); the folded net is then up to date only
//! after the full settles. A settle of 2000 lines or more (`FAST_THREAD_MIN`)
//! is split across three threads (`FAST_THREADS`), each writing slots of its
//! own, apart from the others' (`split`); `FAST_THREAD_REPORT` prints how. A
//! part that a worker has not started is run by a worker done with its own,
//! or by the caller once done with its own (`POOL`), so a busy machine that
//! has not scheduled a worker does not hold the settle up.
use super::iologic::{DIVIDER_STAGES, GEAR_IN7_ORDER};
use super::*;
use std::fmt::Write as _;

/// The C source of one design and the map from kernel handles to its slots.
pub struct FastProgram {
    pub source: String,
    /// What it was built with.
    pub options: FastOptions,
    /// Value slots: nets first, then the previous level of each clock group.
    pub slots: usize,
    /// Where each slot of the layout before the threads' regions is, in this
    /// program (`place.len()` of them): every program of one kernel numbers
    /// these the same, whatever its options, so state carries over between
    /// them through it.
    pub place: Vec<usize>,
    /// Lines of each clock group's settle after a capture, after folding, and
    /// the phases it is split into across threads (0 when it is not split).
    pub settle_lines: Vec<usize>,
    pub settle_phases: Vec<usize>,
    /// Initial slot values: 0 or 1, or 2 for a net that is X or Z and that the
    /// program never reads.
    pub initial: Vec<u8>,
    scope: u64,
    /// Slot of each signal (roots and their aliases), if the program has it,
    /// and whether the signal is the inverse of the slot.
    slot_of: Vec<Option<(usize, bool)>>,
    /// Slots not up to date after every clock edge (`fresh`).
    stale: HashSet<usize>,
    /// Slot that each external driver sets.
    input_of: HashMap<usize, usize>,
    /// Input slots that accept Z (`3`): external drivers of tristate pads.
    z_inputs: HashSet<usize>,
    pub gates: usize,
    /// Gates evaluated in the captures, only when an enable reads them.
    pub demanded_gates: usize,
    pub registers: usize,
    pub iologic: usize,
    /// EBR resource keys, in the order of the error units.
    pub ebrs: Vec<String>,
    /// Tristate pad net names with an external driver, in the order of error code 4 units.
    pub pads: Vec<String>,
    /// The slot that flags a pad driven by the design and from outside with
    /// different levels: 0, else the pad's unit + 1. It is in the slot
    /// array, not a static of the library, because simulations of one
    /// design share the library (and its statics) and may run at once.
    pub pad_conflict: Option<usize>,
    /// First slot of each EBR's 2048x9 array, a byte per cell.
    pub memories: Vec<usize>,
    /// First slot of the sixteen cells of each LUTRAM column, in the order of
    /// the kernel's LUTRAM writers.
    pub lutrams: Vec<usize>,
    pub clock_groups: usize,
    pub plls: Vec<FastPll>,
    pub oscillators: Vec<FastOscillator>,
    pub dtrs: Vec<FastDtr>,
    /// DCSC guards cleared only when physical time advances. All handshake
    /// state lives in `initial`/the simulation's slots and follows `place`.
    pub(crate) dcs_blocked: Vec<usize>,
}

/// Host-driven DTR outputs and the STARTPULSE captured before publication of
/// a wave's registers. Completion times carry over from the event kernel.
pub struct FastDtr {
    pub captured: usize,
    pub outputs: [usize; 8],
    pub code: u8,
    pub previous: u8,
    pub completion: Option<Time>,
}

/// A PLL whose output slots the host drives.
pub struct FastPll {
    pub label: String,
    /// The input slot of the reference clock, and whether the reference is its
    /// inverse.
    pub reference: (usize, bool),
    /// VCO period = reference period × a / b.
    pub a: u64,
    pub b: u64,
    /// (output slot, divider, phase in eighths of a VCO period)
    pub outputs: Vec<(usize, u64, i64)>,
    /// What the kernel's PLL has learnt: its last reference rising edge, and
    /// the reference period with the edge its outputs are laid out from.
    pub last_rise: Option<Time>,
    pub learnt: Option<(Time, Time)>,
}

/// The kernel's state has an unknown register, latch or stage that fast mode
/// would read. Running the event kernel further, until the design has
/// written it, may make it known.
#[derive(Debug)]
pub struct StartsUnknown(pub String);
impl std::fmt::Display for StartsUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fast mode is two-valued; {} is unknown", self.0)
    }
}
impl std::error::Error for StartsUnknown {}

/// An oscillator output slot: low at time 0, then toggling every half period
/// (osc.rs).
pub struct FastOscillator {
    pub slot: usize,
    pub half_period: Time,
}

impl FastProgram {
    pub fn slot(&self, signal: SignalId) -> Option<(usize, bool)> {
        (signal.scope == self.scope)
            .then(|| self.slot_of.get(signal.index).copied().flatten())
            .flatten()
    }
    /// Whether the program has `signal` up to date after every clock edge,
    /// as a probe, a register or an input is (a sample of it after each edge
    /// is its value): not a net folded into the table of a gate that reads
    /// it, nor a gate evaluated only for a capture that needs it, which are
    /// up to date only after a full settle (`FastSim::step`).
    /// `Kernel::watch` keeps a net computed.
    pub fn fresh(&self, signal: SignalId) -> bool {
        self.slot(signal)
            .is_some_and(|(slot, _)| !self.stale.contains(&slot))
    }
    pub fn input_slot(&self, driver: DriverId) -> Option<usize> {
        (driver.scope == self.scope)
            .then(|| self.input_of.get(&driver.index).copied())
            .flatten()
    }
    pub(crate) fn allows_z(&self, slot: usize) -> bool {
        self.z_inputs.contains(&slot)
    }
}

/// A gate input or a folded value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Term {
    Lit(u8),
    Net { slot: usize, inverted: bool },
}
impl Term {
    fn invert_if(self, invert: bool) -> Self {
        match self {
            Term::Lit(v) => Term::Lit(v ^ u8::from(invert)),
            Term::Net { slot, inverted } => Term::Net {
                slot,
                inverted: inverted ^ invert,
            },
        }
    }
    pub(super) fn c(self) -> String {
        match self {
            Term::Lit(v) => v.to_string(),
            Term::Net {
                slot,
                inverted: false,
            } => format!("v[{slot}]"),
            Term::Net {
                slot,
                inverted: true,
            } => format!("(v[{slot}]^1)"),
        }
    }
}

/// Group register resets without moving a read across a write, or changing
/// the order of local/GSR writes to the same Q. Independent writes commute;
/// `changed` still records every intermediate value that differs.
fn asynchronous_registers(resets: &[(Term, usize, Term)]) -> String {
    let mut after = vec![Vec::new(); resets.len()];
    let mut waiting = vec![0; resets.len()];
    let mut writer = HashMap::new();
    let mut readers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &(reset, q, value)) in resets.iter().enumerate() {
        let mut dependencies = Vec::new();
        for input in [reset, value] {
            if let Term::Net { slot, .. } = input {
                if let Some(&previous) = writer.get(&slot) {
                    dependencies.push(previous);
                }
                readers.entry(slot).or_default().push(i);
            }
        }
        if let Some(previous) = writer.insert(q, i) {
            dependencies.push(previous);
        }
        dependencies.extend(readers.remove(&q).unwrap_or_default());
        dependencies.retain(|&previous| previous != i);
        dependencies.sort_unstable();
        dependencies.dedup();
        waiting[i] = dependencies.len();
        for previous in dependencies {
            after[previous].push(i);
        }
    }
    let mut ready: BTreeSet<usize> = (0..resets.len()).filter(|&i| waiting[i] == 0).collect();
    let mut text = String::new();
    while let Some(&first) = ready.first() {
        let reset = resets[first].0;
        let _ = writeln!(text, "  if ({}) {{", reset.c());
        while let Some(i) = ready.iter().copied().find(|&i| resets[i].0 == reset) {
            ready.remove(&i);
            let (_, q, value) = resets[i];
            let value = value.c();
            let _ = writeln!(
                text,
                "    if (v[{q}] != {value}) {{ v[{q}] = {value}; changed = 1; }}"
            );
            for &next in &after[i] {
                waiting[next] -= 1;
                if waiting[next] == 0 {
                    ready.insert(next);
                }
            }
            // A reset can itself be a register output. Its new level must
            // be tested again before another register uses that reset.
            if matches!(reset, Term::Net { slot, .. } if slot == q) {
                break;
            }
        }
        text.push_str("  }\n");
    }
    text
}

/// A flip-flop capture, before selecting its clock edge. Forces keep their
/// existing ternary priority; only the synchronous reset and CE are grouped.
struct RegisterCapture {
    q: usize,
    data: Term,
    enable: Term,
    reset: Option<(Term, Term)>,
    forced: Vec<(Term, Term)>,
    reads: Vec<usize>,
    group: usize,
    edge: Edge,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CaptureGuard {
    edge: Option<String>,
    enable: String,
    reset: Option<String>,
}

#[derive(Clone, Copy)]
enum CaptureStore {
    Direct,
    Temporary,
    Commit,
}

impl RegisterCapture {
    fn predicates(&self) -> impl Iterator<Item = Term> + '_ {
        std::iter::once(self.enable)
            .chain(self.reset.map(|r| r.0))
            .chain(self.forced.iter().map(|r| r.0))
    }

    fn guard(&self, saved: &HashMap<Term, String>, general: bool) -> CaptureGuard {
        let predicate = |t: Term| saved.get(&t).cloned().unwrap_or_else(|| t.c());
        let mut enables: Vec<String> = std::iter::once(self.enable)
            .chain(self.forced.iter().map(|r| r.0))
            .filter(|t| *t != Term::Lit(0))
            .map(predicate)
            .collect();
        enables.sort_unstable();
        enables.dedup();
        let enable = if enables.iter().any(|e| e == "1") {
            "1".to_string()
        } else if enables.is_empty() {
            "0".to_string()
        } else {
            enables.join(" | ")
        };
        CaptureGuard {
            edge: general.then(|| {
                format!(
                    "{}{}",
                    if self.edge == Edge::Rising {
                        "rise"
                    } else {
                        "fall"
                    },
                    self.group
                )
            }),
            enable,
            reset: self
                .reset
                .filter(|r| r.0 != Term::Lit(0))
                .map(|r| predicate(r.0)),
        }
    }

    fn value(&self, reset: bool, saved: &HashMap<Term, String>) -> String {
        let mut value = if reset {
            self.reset.unwrap().1
        } else {
            self.data
        }
        .c();
        for &(force, to) in &self.forced {
            let force = saved.get(&force).cloned().unwrap_or_else(|| force.c());
            value = format!("{force} ? {} : ({value})", to.c());
        }
        value
    }
}

/// Snapshot only predicates that a capture can overwrite. The same locals
/// guard temporary evaluation and commit, even if intervening writes change
/// CE/LSR. Other predicates are settled gates/inputs, stable through captures.
fn capture_predicates(
    chosen: &[usize],
    captures: &[RegisterCapture],
) -> (String, HashMap<Term, String>) {
    let written: HashSet<usize> = chosen.iter().map(|&r| captures[r].q).collect();
    let mut saved = HashMap::new();
    let mut text = String::new();
    for &r in chosen {
        for t in captures[r].predicates() {
            if matches!(t, Term::Net { slot, .. } if written.contains(&slot))
                && !saved.contains_key(&t)
            {
                let name = format!("cp{}", saved.len());
                let _ = writeln!(text, "  u8 {name} = !!({});", t.c());
                saved.insert(t, name);
            }
        }
    }
    (text, saved)
}

/// `batch` contains mutually independent writes (or evaluations into n[], or
/// commits of already evaluated n[]). No dependency is crossed by bucketing.
fn capture_batch(
    batch: &[usize],
    captures: &[RegisterCapture],
    saved: &HashMap<Term, String>,
    general: bool,
    store: CaptureStore,
) -> String {
    let mut buckets: Vec<(CaptureGuard, Vec<usize>)> = Vec::new();
    let mut bucket_of = HashMap::new();
    for &r in batch {
        let guard = captures[r].guard(saved, general);
        let k = *bucket_of.entry(guard.clone()).or_insert_with(|| {
            buckets.push((guard, Vec::new()));
            buckets.len() - 1
        });
        buckets[k].1.push(r);
    }
    let mut text = String::new();
    for (guard, registers) in buckets {
        if let Some(edge) = &guard.edge {
            let _ = writeln!(text, "  if ({edge}) {{");
        }
        let writes = |reset: bool| -> String {
            registers
                .iter()
                .map(|&r| {
                    let q = captures[r].q;
                    match store {
                        CaptureStore::Commit => format!("  v[{q}] = n[{r}];\n"),
                        CaptureStore::Direct => {
                            format!("  v[{q}] = {};\n", captures[r].value(reset, saved))
                        }
                        CaptureStore::Temporary => {
                            format!("  n[{r}] = {};\n", captures[r].value(reset, saved))
                        }
                    }
                })
                .collect()
        };
        if matches!(store, CaptureStore::Commit) {
            let active = match guard.reset.as_deref() {
                Some("1") => "1".to_string(),
                Some(reset) if guard.enable != "1" => format!("({reset}) | ({})", guard.enable),
                _ => guard.enable.clone(),
            };
            if active != "0" {
                if active != "1" {
                    let _ = writeln!(text, "  if ({active}) {{");
                }
                text.push_str(&writes(false));
                if active != "1" {
                    text.push_str("  }\n");
                }
            }
        } else if guard.reset.as_deref() == Some("1") {
            text.push_str(&writes(true));
        } else {
            if let Some(reset) = &guard.reset {
                let _ = writeln!(text, "  if ({reset}) {{\n{}  }}", writes(true));
            }
            if guard.enable != "0" {
                let otherwise = if guard.reset.is_some() {
                    " else "
                } else {
                    "  "
                };
                if guard.enable == "1" {
                    if guard.reset.is_some() {
                        text.push_str(" else {\n");
                    }
                    text.push_str(&writes(false));
                    if guard.reset.is_some() {
                        text.push_str("  }\n");
                    }
                } else {
                    let _ = writeln!(
                        text,
                        "{otherwise}if ({}) {{\n{}  }}",
                        guard.enable,
                        writes(false)
                    );
                }
            } else if guard.reset.is_some() {
                text.push('\n');
            }
        }
        if guard.edge.is_some() {
            text.push_str("  }\n");
        }
    }
    text
}

/// Keep place's reader-before-writer DAG. Only a frontier with no internal Q
/// dependencies may be reordered by predicate; release successors afterwards.
fn place_captures(
    chosen: &[usize],
    captures: &[RegisterCapture],
    general: bool,
) -> (String, String, String) {
    let writer: HashMap<usize, usize> = chosen.iter().map(|&r| (captures[r].q, r)).collect();
    let mut waiting: HashMap<usize, usize> = chosen.iter().map(|&r| (r, 0)).collect();
    let mut before: HashMap<usize, Vec<usize>> = HashMap::new();
    for &r in chosen {
        for input in &captures[r].reads {
            if let Some(&s) = writer.get(input).filter(|&&s| s != r) {
                *waiting.get_mut(&s).unwrap() += 1;
                before.entry(r).or_default().push(s);
            }
        }
    }
    let mut ready: Vec<usize> = chosen.iter().copied().filter(|r| waiting[r] == 0).collect();
    let mut frontiers = Vec::new();
    let mut placed = HashSet::new();
    while !ready.is_empty() {
        let batch = std::mem::take(&mut ready);
        for &r in &batch {
            placed.insert(r);
            for &s in before.get(&r).into_iter().flatten() {
                let w = waiting.get_mut(&s).unwrap();
                *w -= 1;
                if *w == 0 {
                    ready.push(s);
                }
            }
        }
        frontiers.push(batch);
    }
    let cyclic: Vec<usize> = chosen
        .iter()
        .copied()
        .filter(|r| !placed.contains(r))
        .collect();
    let (predicates, saved) = capture_predicates(chosen, captures);
    let mut capture = capture_batch(&cyclic, captures, &saved, general, CaptureStore::Temporary);
    for batch in frontiers {
        capture.push_str(&capture_batch(
            &batch,
            captures,
            &saved,
            general,
            CaptureStore::Direct,
        ));
    }
    let commit = capture_batch(&cyclic, captures, &saved, general, CaptureStore::Commit);
    (predicates, capture, commit)
}

/// A net that a tristate driver may leave undriven: whether it is driven and
/// its value then, as C expressions, and the terms they read.
#[derive(Clone)]
struct Tri {
    en: String,
    val: String,
    reads: Vec<Term>,
}

enum Source {
    Gate(usize),
    Register,
    Input,
    Constant(Logic),
}

/// Only the signal fields C generation reads. Names and driver lists are
/// pooled to avoid a heap allocation for every routing segment.
struct GenerationSignal {
    location: crate::model::topology::Location,
    name: usize,
    drivers: std::ops::Range<usize>,
    value: Logic,
    read: bool,
    watched: bool,
    system: bool,
}

struct GenerationSignals {
    signals: Vec<GenerationSignal>,
    names: Vec<String>,
    drivers: Vec<usize>,
}

impl GenerationSignals {
    fn new(signals: &[Signal]) -> crate::snapshot::AllocationResult<Self> {
        use crate::snapshot;
        let mut out = snapshot::buffer(signals.len())?;
        let mut names = Vec::new();
        let mut name_of = HashMap::new();
        let mut drivers = snapshot::buffer(signals.iter().map(|s| s.drivers.len()).sum())?;
        for s in signals {
            let name = if let Some(&name) = name_of.get(s.wire.name.as_str()) {
                name
            } else {
                name_of.try_reserve(1)?;
                names.try_reserve(1)?;
                let name = names.len();
                names.push(snapshot::string(&s.wire.name)?);
                name_of.insert(s.wire.name.as_str(), name);
                name
            };
            let start = drivers.len();
            drivers.extend_from_slice(&s.drivers);
            out.push(GenerationSignal {
                location: s.wire.location,
                name,
                drivers: start..drivers.len(),
                value: s.value,
                read: s.read,
                watched: s.watched,
                system: s.system,
            });
        }
        Ok(Self {
            signals: out,
            names,
            drivers,
        })
    }
}

struct GenerationWire<'a> {
    location: crate::model::topology::Location,
    name: &'a str,
}

struct GenerationSignalRef<'a> {
    wire: GenerationWire<'a>,
    drivers: &'a [usize],
    value: Logic,
    read: bool,
    watched: bool,
    system: bool,
}

enum Signals<'a> {
    Live(&'a [Signal]),
    Snapshot(&'a GenerationSignals),
}

impl Signals<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Live(s) => s.len(),
            Self::Snapshot(s) => s.signals.len(),
        }
    }
    fn get(&self, index: usize) -> GenerationSignalRef<'_> {
        match self {
            Self::Live(signals) => {
                let s = &signals[index];
                GenerationSignalRef {
                    wire: GenerationWire {
                        location: s.wire.location,
                        name: &s.wire.name,
                    },
                    drivers: &s.drivers,
                    value: s.value,
                    read: s.read,
                    watched: s.watched,
                    system: s.system,
                }
            }
            Self::Snapshot(snapshot) => {
                let s = &snapshot.signals[index];
                GenerationSignalRef {
                    wire: GenerationWire {
                        location: s.location,
                        name: &snapshot.names[s.name],
                    },
                    drivers: &snapshot.drivers[s.drivers.clone()],
                    value: s.value,
                    read: s.read,
                    watched: s.watched,
                    system: s.system,
                }
            }
        }
    }
    fn iter(&self) -> impl Iterator<Item = GenerationSignalRef<'_>> {
        (0..self.len()).map(|i| self.get(i))
    }
}

/// The generator sees borrowed live inputs or an owned snapshot, through
/// the same read-only view. Event-kernel vectors remain plain Vec.
pub(super) struct FastKernel<'a> {
    kernel: &'a Kernel,
    signals: Signals<'a>,
    /// Resolved on the live kernel before copying: no event queue is retained.
    dtr_completions: Option<&'a [Option<Time>]>,
}

impl std::ops::Deref for FastKernel<'_> {
    type Target = Kernel;
    fn deref(&self) -> &Self::Target {
        self.kernel
    }
}

/// Owned generation inputs for the tuning worker. Its private kernel has
/// no event queue or signal read indices; only generation is exposed.
pub(crate) struct FastGenerator(Kernel, GenerationSignals, Vec<Option<Time>>);

impl FastGenerator {
    pub(crate) fn program(&self, options: &FastOptions) -> Result<FastProgram> {
        FastKernel {
            kernel: &self.0,
            signals: Signals::Snapshot(&self.1),
            dtr_completions: Some(&self.2),
        }
        .program(options)
    }
}

impl Kernel {
    /// Copy generation inputs once, when tuning starts. Signal fanout lists,
    /// event bookkeeping and wire-name lookup stay with the live kernel.
    /// Later event execution and watch changes never detach a shared vector.
    pub(crate) fn fast_generator(&self) -> crate::snapshot::AllocationResult<FastGenerator> {
        use crate::snapshot;
        let mut dtrs = snapshot::buffer(self.dtrs.len())?;
        let mut dtr_completions = snapshot::buffer(self.dtrs.len())?;
        for (index, dtr) in self.dtrs.iter().enumerate() {
            dtrs.push(dtr.generation_snapshot());
            dtr_completions.push(self.fast_dtr_completion(index));
        }
        let signals = GenerationSignals::new(&self.signals)?;
        let mut drivers = snapshot::buffer(self.drivers.len())?;
        for d in &self.drivers {
            drivers.push(Driver {
                signal: d.signal,
                value: d.value,
                strength: d.strength,
                label: snapshot::string(&d.label)?,
                external: d.external,
            });
        }
        let mut aliases = snapshot::buffer(self.aliases.len())?;
        for list in &self.aliases {
            aliases.push(snapshot::cloned(list)?);
        }
        Ok(FastGenerator(
            Self {
                device: Arc::clone(&self.device),
                state: self.state.fast_snapshot()?,
                scope: self.scope,
                signals: Vec::new(),
                signal_ids: HashMap::new(),
                drivers,
                combinational: snapshot::cloned(&self.combinational)?,
                registers: snapshot::cloned(&self.registers)?,
                aliases,
                clock_groups: Vec::new(),
                dirty_clock_groups: DirtySet::default(),
                register_seen: Vec::new(),
                register_order: Vec::new(),
                lut_writes: snapshot::cloned(&self.lut_writes)?,
                system: self.system.clone(),
                ebr: snapshot::cloned(&self.ebr)?,
                multipliers: snapshot::cloned(&self.multipliers)?,
                alus: snapshot::cloned(&self.alus)?,
                plls: snapshot::cloned(&self.plls)?,
                dirty_plls: DirtySet::default(),
                iologic: snapshot::cloned(&self.iologic)?,
                dirty_iologic: DirtySet::default(),
                dcs: snapshot::cloned(&self.dcs)?,
                dtrs,
                dirty_dcs: DirtySet::default(),
                dirty_dtrs: DirtySet::default(),
                oscillators: snapshot::cloned(&self.oscillators)?,
                oscillator_timers: HashMap::new(),
                signal_rank: Vec::new(),
                driver_rank: Vec::new(),
                affected: Vec::new(),
                spare_updates: Vec::new(),
                dirty_combinational: DirtySet::default(),
                dirty_registers: DirtySet::default(),
                dirty_lut_writes: DirtySet::default(),
                dirty_system: false,
                dirty_ebr: DirtySet::default(),
                dirty_multipliers: DirtySet::default(),
                dirty_multiplier_registers: DirtySet::default(),
                timers: Vec::new(),
                timer_labels: HashSet::new(),
                queue: BTreeMap::new(),
                now: self.now,
                delta: 0,
                options: self.options,
                trace: VecDeque::new(),
                ebr_notices: VecDeque::new(),
                stats: Statistics::default(),
                recent_activity: VecDeque::new(),
                fault: None,
            },
            signals,
            dtr_completions,
        ))
    }

    /// Only the current timer generation can complete a conversion; cancelled
    /// events can still be in the queue. Snapshots retain this deadline alone.
    fn fast_dtr_completion(&self, index: usize) -> Option<Time> {
        let dtr = &self.dtrs[index];
        let timer = dtr.timer?.index;
        let generation = self.timers[timer].generation;
        self.queue.iter().find_map(|(&time, events)| {
            events
                .iter()
                .any(|e| e.timer == Some((timer, generation)) && dtr.drivers.contains(&e.driver))
                .then_some(time)
        })
    }

    /// The two-valued C program of this kernel's current plan and state,
    /// with the options of the environment.
    pub fn fast_program(&self) -> Result<FastProgram> {
        self.fast_program_with(&FastOptions::from_env())
    }

    /// Non-routing gates, before folding, for the startup thread guess.
    /// Counting these avoids generating C just to learn the design's size.
    pub(crate) fn fast_gate_count(&self) -> usize {
        self.combinational
            .iter()
            .filter(|g| !matches!(g.function, Function::Buffer(_) | Function::Receiver(_)))
            .count()
    }

    /// Identity for a performance hint, not a cached executable or state.
    /// Bind the physical configuration and selected plan without generating
    /// C. Handles use their dense indices, not the per-opening scope. Watches,
    /// probe labels, live values and clock history do not identify a design.
    pub(crate) fn fast_choice_identity(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.device.fingerprint().hash(&mut h);
        // Buffer aliases and the derived read/system flags depend on which
        // routing segments were watched when the event plan was collapsed.
        // The original gates and physical nets below identify the plan.
        for s in &self.signals {
            (&s.wire, &s.drivers).hash(&mut h);
        }
        for d in &self.drivers {
            (d.signal, d.external, d.strength as u8).hash(&mut h);
        }
        let input = |i: Input| match i {
            Input::Constant(v) => (0_u8, v as usize),
            Input::Signal(s) => (1, s.index),
            Input::Inverted(s) => (2, s.index),
        };
        for g in &self.combinational {
            g.driver.hash(&mut h);
            std::mem::discriminant(&g.function).hash(&mut h);
            match &g.function {
                Function::Buffer(i) | Function::Receiver(i) => input(*i).hash(&mut h),
                Function::Mux { select, low, high } => {
                    [*select, *low, *high].map(input).hash(&mut h);
                }
                Function::Lut4 { node, inputs } => {
                    (node, inputs.map(input)).hash(&mut h);
                }
                Function::Carry {
                    node,
                    inputs,
                    cin,
                    inject,
                    carry_output,
                } => {
                    (node, inputs.map(input), input(*cin), inject, carry_output).hash(&mut h);
                }
            }
        }
        for r in &self.registers {
            let c = &r.config;
            (
                c.node,
                [c.data, c.clock, c.enable].map(input),
                c.edge as u8,
                c.reset
                    .map(|r| (input(r.input), r.kind as u8, r.value as u8)),
                r.global_reset.map(|r| (input(r.input), r.value as u8)),
                r.driver,
                r.group,
            )
                .hash(&mut h);
        }
        for d in &self.dcs {
            (
                d.config.mode as u8,
                d.config.clocks.map(input),
                d.config.select.map(input),
                input(d.config.mode_select),
                d.driver,
            )
                .hash(&mut h);
        }
        // Peripheral configuration is bound by the device fingerprint;
        // their selected ports are among the signals and drivers above.
        (
            self.lut_writes.len(),
            self.ebr.len(),
            self.multipliers.len(),
            self.alus.len(),
            self.plls.len(),
            self.iologic.len(),
            self.oscillators.len(),
        )
            .hash(&mut h);
        h.finish()
    }

    /// The program with `options`.
    pub fn fast_program_with(&self, options: &FastOptions) -> Result<FastProgram> {
        FastKernel {
            kernel: self,
            signals: Signals::Live(&self.signals),
            dtr_completions: None,
        }
        .program(options)
    }
}

impl FastKernel<'_> {
    fn signal(&self, index: usize) -> GenerationSignalRef<'_> {
        self.signals.get(index)
    }

    fn program(&self, options: &FastOptions) -> Result<FastProgram> {
        // Multiplier outputs are nodes next to the gates, numbered after
        // them (fast_dsp.rs).
        let dsp = super::fast_dsp::DspNodes::new(self)?;
        let gates_n = self.combinational.len();
        let driver_of = |g: usize| {
            if g < gates_n {
                self.combinational[g].driver
            } else {
                dsp.driver(g - gates_n)
            }
        };
        let n = self.signals.len();
        let mut root = (0..n).collect::<Vec<_>>();
        for (r, list) in self.aliases.iter().enumerate() {
            for &a in list {
                root[a] = r;
            }
        }
        let mut gate_of_driver = HashMap::new();
        for (g, gate) in self.combinational.iter().enumerate() {
            gate_of_driver.insert(gate.driver, g);
        }
        // IOLOGIC and EBR outputs are written at captures, like flip-flop
        // outputs.
        let register_drivers: HashSet<usize> = self
            .registers
            .iter()
            .map(|r| r.driver)
            .chain(
                self.iologic
                    .iter()
                    .flat_map(|io| io.drivers.iter().copied()),
            )
            .chain(
                self.ebr
                    .iter()
                    .flat_map(|b| b.outputs.iter().flatten().flatten().copied()),
            )
            .chain(self.dcs.iter().map(|d| d.driver))
            .collect();
        // PLL and OSC outputs are inputs that the host drives in time.
        let generated_drivers: HashSet<usize> = self
            .plls
            .iter()
            .flat_map(|p| p.outputs.iter().map(|o| o.0))
            .chain(self.oscillators.iter().map(|o| o.driver))
            .chain(self.dtrs.iter().flat_map(|d| d.drivers))
            .collect();
        let mut generated_nets = Vec::new();
        // What drives each root net.
        let mut source: Vec<Option<Source>> = (0..n).map(|_| None).collect();
        let mut input_signals = HashMap::new();
        for s in 0..n {
            if root[s] != s {
                continue;
            }
            let signal = &self.signal(s);
            let (mut gates, mut registers, mut externals) = (Vec::new(), 0, Vec::new());
            for &d in signal.drivers {
                if let Some(&g) = gate_of_driver.get(&d) {
                    gates.push(g);
                } else if let Some(node) = dsp.of_driver(d) {
                    gates.push(gates_n + node);
                } else if register_drivers.contains(&d) {
                    registers += 1;
                } else if self.drivers[d].external || generated_drivers.contains(&d) {
                    externals.push(d);
                }
            }
            ensure!(
                !signal.system || externals.is_empty(),
                "fast mode does not drive the system controller ({})",
                signal.wire.name
            );
            source[s] = Some(match (gates.as_slice(), registers, externals.as_slice()) {
                ([], 0, []) => Source::Constant(signal.value),
                ([], 0, [e]) => {
                    if generated_drivers.contains(e) {
                        generated_nets.push(s);
                    } else {
                        input_signals.insert(*e, s);
                    }
                    Source::Input
                }
                ([g], 0, _) => Source::Gate(*g),
                ([], 1, _) => Source::Register,
                _ => bail!(
                    "fast mode needs one active driver per net; R{}C{}:{} has {} gates, {registers} registers and {} external drivers",
                    signal.wire.location.row,
                    signal.wire.location.col,
                    signal.wire.name,
                    gates.len(),
                    externals.len()
                ),
            });
        }
        // These lookups serve only source classification. In a full-chip
        // design the driver map alone is tens of MiB; free it before the
        // folding tables, generated text and output maps coexist.
        drop(gate_of_driver);
        drop(register_drivers);
        drop(generated_drivers);
        // Slots for root nets; aliases share their root's slot.
        let mut slot = vec![usize::MAX; n];
        let mut slots = 0;
        for s in 0..n {
            if root[s] == s {
                slot[s] = slots;
                slots += 1;
            }
        }
        let mut input_of: HashMap<usize, usize> =
            input_signals.iter().map(|(&d, &s)| (d, slot[s])).collect();
        let mut z_inputs = HashSet::new();
        let mut pads = Vec::new();
        let mut pad_conflict: Option<usize> = None;
        let mut tri_slots = Vec::new();
        let bit = |v: Logic| match v {
            Logic::Zero => 0,
            Logic::One => 1,
            _ => 2,
        };
        let mut initial: Vec<u8> = (0..n)
            .filter(|&s| root[s] == s)
            .map(|s| bit(self.signal(s).value))
            .collect();
        // The cells of each LUTRAM column follow the nets; its reader is the
        // gate that indexes them.
        let net_name = |s: usize| {
            let wire = &self.signal(s).wire;
            format!("R{}C{}:{}", wire.location.row, wire.location.col, wire.name)
        };
        // The planner binds a complete DPR16X4 even for a narrower RAM.
        // Only columns whose read output has a consumer or probe can affect
        // the program. The others may have floating read/write ports.
        let watched_roots: HashSet<usize> = self
            .signals
            .iter()
            .enumerate()
            .filter(|(_, s)| s.watched)
            .map(|(s, _)| root[s])
            .collect();
        let lutram_active: Vec<bool> = self
            .lut_writes
            .iter()
            .map(|w| {
                let r = root[self.drivers[self.combinational[w.reader].driver].signal];
                self.signal(r).read || watched_roots.contains(&r)
            })
            .collect();
        let dead_lut_readers: HashSet<usize> = self
            .lut_writes
            .iter()
            .enumerate()
            .filter(|(w, _)| !lutram_active[*w])
            .map(|(_, w)| w.reader)
            .collect();
        let mut lutrams = Vec::new();
        let mut lutram_of_gate = HashMap::new();
        for (w, writer) in self.lut_writes.iter().enumerate() {
            lutrams.push(slots);
            lutram_of_gate.insert(writer.reader, w);
            for &cell in self.state.lut(writer.config.node)? {
                initial.push(match bit(cell) {
                    2 if lutram_active[w] => {
                        let reader = self.drivers[self.combinational[writer.reader].driver].signal;
                        return Err(StartsUnknown(format!(
                            "a cell of the LUTRAM read at {}",
                            net_name(reader)
                        ))
                        .into());
                    }
                    b => b,
                });
            }
            slots += 16;
        }
        // The MULT18 register stages follow (fast_dsp.rs).
        let dsp_regs = dsp.registers(self, &mut slots, &mut initial)?;
        // What each root net folds to: a literal when it is constant, or the
        // net it copies (through a buffer or receiver, maybe inverted).
        let mut constant: Vec<Option<Term>> = vec![None; n];
        for s in 0..n {
            if let Some(Source::Constant(v)) = source[s] {
                constant[s] = match bit(v) {
                    2 => None,
                    b => Some(Term::Lit(b)),
                };
            }
        }
        let term = |input: Input, constant: &[Option<Term>]| -> Result<Term> {
            Ok(match input {
                Input::Constant(v) => match bit(v) {
                    2 => bail!("fast mode reads a constant {v}"),
                    b => Term::Lit(b),
                },
                Input::Signal(s) | Input::Inverted(s) => {
                    let r = root[s.index];
                    let inverted = matches!(input, Input::Inverted(_));
                    match constant[r] {
                        Some(Term::Lit(b)) => Term::Lit(b ^ u8::from(inverted)),
                        Some(Term::Net { slot, inverted: i }) => Term::Net {
                            slot,
                            inverted: i ^ inverted,
                        },
                        None => {
                            ensure!(
                                !matches!(source[r], Some(Source::Constant(_))),
                                "fast mode reads R{}C{}:{}, which is {}",
                                self.signal(r).wire.location.row,
                                self.signal(r).wire.location.col,
                                self.signal(r).wire.name,
                                self.signal(r).value
                            );
                            Term::Net {
                                slot: slot[r],
                                inverted,
                            }
                        }
                    }
                }
            })
        };
        // Gates in dependency order (Kahn), folding constants as they settle.
        let gate_net = |g: usize| root[self.drivers[driver_of(g)].signal];
        let live: Vec<usize> = (0..gates_n + dsp.len())
            .filter(|&g| {
                let s = self.drivers[driver_of(g)].signal;
                root[s] == s
                    && !dead_lut_readers.contains(&g)
                    && matches!(source[s], Some(Source::Gate(x)) if x == g)
            })
            .collect();
        // The inputs a LUT's table depends on: nextpnr may route a signal to
        // an input the INIT ignores, and when that signal depends on the
        // LUT's own output the loop is only structural. The kernel, which
        // settles values, runs it; the order here must not see it.
        let used = |node: NodeId| -> [bool; 4] {
            let Ok(table) = self.state.lut(node) else {
                return [true; 4];
            };
            std::array::from_fn(|k| {
                (0..16).any(|a| {
                    let (x, y) = (table[a], table[a ^ (1 << k)]);
                    x != y || x.bit().is_none()
                })
            })
        };
        // Nets known before any gate is ordered: constant sources and the
        // gates whose inputs settle to constants (nextpnr's GND and VCC LUTs
        // and what they feed). nextpnr starts a carry chain with a CCU2 whose
        // propagate term a GND input holds at 0; its FCI is wired to the FCO
        // of whatever chain ends just before it in the row, which it never
        // passes, so that read is only structural and the order must not see
        // it (with the chains feeding each other through logic, it is a
        // loop the kernel settles).
        let mut known: Vec<Option<u8>> = constant
            .iter()
            .map(|c| match c {
                Some(Term::Lit(b)) => Some(*b),
                _ => None,
            })
            .collect();
        let value_of = |i: Input, known: &[Option<u8>]| -> Option<u8> {
            match i {
                Input::Constant(v) => match bit(v) {
                    2 => None,
                    b => Some(b),
                },
                Input::Signal(s) => known[root[s.index]],
                Input::Inverted(s) => known[root[s.index]].map(|b| b ^ 1),
            }
        };
        // The one value a table takes over the addresses consistent with
        // the known inputs (`width` of them address it), if it is one.
        let settled = |init: u16, inputs: &[Option<u8>], width: usize| -> Option<u8> {
            let mut seen = [false; 2];
            for address in 0..1_usize << width {
                if inputs[..width]
                    .iter()
                    .enumerate()
                    .all(|(k, v)| v.is_none_or(|b| (address >> k) & 1 == usize::from(b)))
                {
                    seen[usize::from((init >> address) & 1 == 1)] = true;
                }
            }
            match seen {
                [true, false] => Some(0),
                [false, true] => Some(1),
                _ => None,
            }
        };
        let lut_known = |node: NodeId, inputs: &[Input; 4], known: &[Option<u8>]| {
            let init = self.state.lut(node).ok().and_then(|t| lut_init(t).ok())?;
            Some((init, inputs.map(|i| value_of(i, known))))
        };
        loop {
            let mut changed = false;
            for &g in &live {
                if g >= gates_n || known[gate_net(g)].is_some() {
                    continue;
                }
                let v = match &self.combinational[g].function {
                    Function::Buffer(i) => value_of(*i, &known),
                    Function::Receiver(_) => None,
                    Function::Mux { select, low, high } => match value_of(*select, &known) {
                        Some(0) => value_of(*low, &known),
                        Some(_) => value_of(*high, &known),
                        None => match (value_of(*low, &known), value_of(*high, &known)) {
                            (Some(a), Some(b)) if a == b => Some(a),
                            _ => None,
                        },
                    },
                    Function::Lut4 { .. } if lutram_of_gate.contains_key(&g) => None,
                    Function::Lut4 { node, inputs } => {
                        lut_known(*node, inputs, &known).and_then(|(init, v)| settled(init, &v, 4))
                    }
                    Function::Carry {
                        node,
                        inputs,
                        cin,
                        inject,
                        carry_output,
                    } => lut_known(*node, inputs, &known).and_then(|(init, v)| {
                        let p = settled(init, &v, 4);
                        let c = value_of(*cin, &known);
                        match (*carry_output, *inject) {
                            (true, _) => match p? {
                                1 => c,
                                _ if *inject => Some(0),
                                _ => settled(init, &v, 2),
                            },
                            (false, true) => p,
                            (false, false) => Some(p? ^ c?),
                        }
                    }),
                };
                if let Some(v) = v {
                    known[gate_net(g)] = Some(v);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Whether a carry gate reads its carry-in: a sum with INJECT1 does
        // not, nor does a carry-out whose propagate term the known inputs
        // hold at 0.
        let reads_cin = |node: NodeId, inputs: &[Input; 4], inject: bool, carry_output: bool| {
            if !carry_output {
                return !inject;
            }
            lut_known(node, inputs, &known).is_none_or(|(init, v)| settled(init, &v, 4) != Some(0))
        };
        let inputs_of = |g: usize| -> Vec<Input> {
            if g >= gates_n {
                return dsp.inputs(self, &dsp_regs, g - gates_n);
            }
            match &self.combinational[g].function {
                Function::Buffer(i) | Function::Receiver(i) => vec![*i],
                Function::Mux { select, low, high } => vec![*select, *low, *high],
                Function::Lut4 { inputs, .. } if lutram_of_gate.contains_key(&g) => inputs.to_vec(),
                Function::Lut4 { node, inputs } => {
                    let used = used(*node);
                    (0..4).filter(|&k| used[k]).map(|k| inputs[k]).collect()
                }
                Function::Carry {
                    node,
                    inputs,
                    cin,
                    inject,
                    carry_output,
                } => {
                    let used = used(*node);
                    let mut v: Vec<Input> =
                        (0..4).filter(|&k| used[k]).map(|k| inputs[k]).collect();
                    if reads_cin(*node, inputs, *inject, *carry_output) {
                        v.push(*cin);
                    }
                    v
                }
            }
        };
        let mut waiting: HashMap<usize, usize> = HashMap::new();
        let mut readers: HashMap<usize, Vec<usize>> = HashMap::new();
        for &g in &live {
            let mut count = 0;
            for input in inputs_of(g) {
                if let Some(s) = input.signal() {
                    let r = root[s.index];
                    if matches!(source[r], Some(Source::Gate(_))) {
                        count += 1;
                        readers.entry(r).or_default().push(g);
                    }
                }
            }
            waiting.insert(g, count);
        }
        let mut ready: VecDeque<usize> = live.iter().copied().filter(|g| waiting[g] == 0).collect();
        let mut order = Vec::new();
        while let Some(g) = ready.pop_front() {
            order.push(g);
            for &next in readers
                .get(&gate_net(g))
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let w = waiting.get_mut(&next).unwrap();
                *w -= 1;
                if *w == 0 {
                    ready.push_back(next);
                }
            }
        }
        if order.len() != live.len() {
            let placed: HashSet<usize> = order.iter().copied().collect();
            // One loop: follow unplaced inputs from an unplaced gate until a
            // gate repeats.
            let gate_of_net: HashMap<usize, usize> = live
                .iter()
                .filter(|g| !placed.contains(g))
                .map(|&g| (gate_net(g), g))
                .collect();
            let mut path = Vec::new();
            let mut g = *live.iter().find(|g| !placed.contains(g)).unwrap();
            while !path.contains(&g) && path.len() < 10_000 {
                path.push(g);
                let Some(next) = inputs_of(g)
                    .iter()
                    .filter_map(|i| i.signal())
                    .find_map(|s| gate_of_net.get(&root[s.index]).copied())
                else {
                    break;
                };
                g = next;
            }
            let start = path.iter().position(|&x| x == g).unwrap_or(0);
            let cycle = &path[start..];
            let shown: Vec<usize> = if cycle.len() <= 12 {
                cycle.to_vec()
            } else {
                cycle[..6]
                    .iter()
                    .chain(&cycle[cycle.len() - 6..])
                    .copied()
                    .collect()
            };
            let mut names: Vec<String> = shown.iter().map(|&g| net_name(gate_net(g))).collect();
            if cycle.len() > 12 {
                names.insert(6, format!("({} more)", cycle.len() - 12));
            }
            bail!(
                "fast mode needs acyclic logic; {} gates are in combinational loops, one through {}",
                live.len() - order.len(),
                names.join(" -> ")
            );
        }
        // (C line, output slot, slots it reads), in dependency order.
        // (C statement, slot it writes, slots it reads)
        let mut lines: Vec<(String, usize, Vec<usize>)> = Vec::new();
        // What each line computes, for collapsing (`collapse`).
        let mut funcs: Vec<LineFn> = Vec::new();
        let reads = |terms: &[Term]| -> Vec<usize> {
            terms
                .iter()
                .filter_map(|t| match t {
                    Term::Net { slot, .. } => Some(*slot),
                    Term::Lit(_) => None,
                })
                .collect()
        };
        // Tristate nets: a mux with a Z input, or reading such a net, as for a
        // PIO output with a tristate control or an open drain. Each is a
        // driven flag and a value (C, and the terms they read); its slot
        // holds the value when driven, else the net's pull, else 3 (Z), which
        // only probes may see (`floating`, checked against `observed`).
        let mut tris: HashMap<usize, Tri> = HashMap::new();
        let mut floating: Vec<(usize, usize)> = Vec::new();
        for &g in &order {
            let out = gate_net(g);
            if g < gates_n {
                if let Function::Mux { select, low, high } = &self.combinational[g].function {
                    let tri_of = |i: Input| -> Result<Option<Tri>> {
                        Ok(match i {
                            Input::Constant(Logic::Z) => Some(Tri {
                                en: "0".into(),
                                val: "0".into(),
                                reads: Vec::new(),
                            }),
                            Input::Signal(s) | Input::Inverted(s) => match tris.get(&root[s.index])
                            {
                                Some(t) => {
                                    ensure!(
                                        matches!(i, Input::Signal(_)),
                                        "fast mode does not invert a tristate net ({})",
                                        net_name(root[s.index])
                                    );
                                    Some(t.clone())
                                }
                                None => None,
                            },
                            _ => None,
                        })
                    };
                    if tri_of(*low)?.is_some() || tri_of(*high)?.is_some() {
                        ensure!(
                            tri_of(*select)?.is_none(),
                            "fast mode does not select by a tristate net ({})",
                            net_name(out)
                        );
                        let side = |i: Input| -> Result<Tri> {
                            Ok(match tri_of(i)? {
                                Some(t) => t,
                                None => {
                                    let t = term(i, &constant)?;
                                    Tri {
                                        en: "1".into(),
                                        val: t.c(),
                                        reads: vec![t],
                                    }
                                }
                            })
                        };
                        let (lo, hi) = (side(*low)?, side(*high)?);
                        let t = match term(*select, &constant)? {
                            Term::Lit(0) => lo,
                            Term::Lit(_) => hi,
                            sel => {
                                let pick = |h: &str, l: &str| {
                                    if h == l {
                                        h.to_string()
                                    } else {
                                        format!("({} ? {h} : {l})", sel.c())
                                    }
                                };
                                let mut reads = vec![sel];
                                reads.extend(lo.reads.iter().chain(&hi.reads));
                                Tri {
                                    en: pick(&hi.en, &lo.en),
                                    val: pick(&hi.val, &lo.val),
                                    reads,
                                }
                            }
                        };
                        // Undriven, the net takes its pull; with one external
                        // driver on a tristate pad, that driver gets its own
                        // input slot (0, 1, or 3 for Z).
                        let mut pulls = Vec::new();
                        let mut externals = Vec::new();
                        for &d in self.signal(out).drivers {
                            let driver = &self.drivers[d];
                            if driver.strength == Strength::Pull {
                                pulls.push(bit(driver.value));
                            } else if driver.external {
                                externals.push(d);
                            }
                        }
                        pulls.dedup();
                        let float = match pulls.as_slice() {
                            [b] if *b < 2 => *b,
                            _ => 3,
                        };
                        let mut read = reads(&t.reads);
                        let text = if t.en == "1" {
                            ensure!(
                                externals.len() <= 1,
                                "fast mode needs one active driver per net; {} has 1 gate, 0 registers and {} external drivers",
                                net_name(out),
                                externals.len()
                            );
                            for &d in &externals {
                                ensure!(
                                    self.drivers[d].value == Logic::Z,
                                    "fast mode does not drive a tristate pad from outside ({})",
                                    net_name(out)
                                );
                            }
                            format!("  v[{}] = {};", slot[out], t.val)
                        } else if externals.is_empty() {
                            match t.en.as_str() {
                                "0" => format!("  v[{}] = {float};", slot[out]),
                                en => format!("  v[{}] = {en} ? {} : {float};", slot[out], t.val),
                            }
                        } else {
                            ensure!(
                                externals.len() == 1,
                                "fast mode needs one active driver per net; {} has 1 gate, 0 registers and {} external drivers",
                                net_name(out),
                                externals.len()
                            );
                            let d = externals[0];
                            let driver = &self.drivers[d];
                            let ext_init = match driver.value {
                                Logic::Zero => 0,
                                Logic::One => 1,
                                Logic::Z => 3,
                                Logic::X => bail!(
                                    "fast mode is two-valued; input {} starts at {}",
                                    driver.label,
                                    driver.value
                                ),
                            };
                            let ext_slot = slots;
                            slots += 1;
                            initial.push(ext_init);
                            input_of.insert(d, ext_slot);
                            z_inputs.insert(ext_slot);
                            read.push(ext_slot);
                            match t.en.as_str() {
                                "0" => format!(
                                    "  v[{}] = v[{ext_slot}] != 3 ? v[{ext_slot}] : {float};",
                                    slot[out]
                                ),
                                en => {
                                    let unit = pads.len();
                                    ensure!(unit < 254, "fast mode flags at most 254 tristate pads driven from outside");
                                    let flag = match pad_conflict {
                                        Some(f) => f,
                                        None => {
                                            let f = slots;
                                            slots += 1;
                                            initial.push(0);
                                            pad_conflict = Some(f);
                                            f
                                        }
                                    };
                                    pads.push(net_name(out));
                                    tri_slots.push(slot[out]);
                                    format!(
                                        "  if ({en}) {{ u8 _v = {}; if (v[{ext_slot}] < 2 && v[{ext_slot}] != _v) tri_error(v, {flag}, {unit}); v[{}] = _v; }} else v[{}] = v[{ext_slot}] != 3 ? v[{ext_slot}] : {float};",
                                        t.val,
                                        slot[out],
                                        slot[out]
                                    )
                                }
                            }
                        };
                        if float == 3 && t.en != "1" {
                            floating.push((slot[out], out));
                        }
                        funcs.push(LineFn::Opaque);
                        lines.push((text, slot[out], read));
                        tris.insert(out, t);
                        continue;
                    }
                }
            }
            if g >= gates_n {
                match dsp.value(self, &dsp_regs, g - gates_n, &|i| term(i, &constant))? {
                    super::fast_dsp::NodeValue::Alias(expr) => {
                        if let Term::Lit(b) = expr {
                            initial[slot[out]] = b;
                        }
                        constant[out] = Some(expr);
                    }
                    super::fast_dsp::NodeValue::Line(rhs, read) => {
                        funcs.push(LineFn::Opaque);
                        lines.push((
                            format!("  v[{}] = {rhs};", slot[out]),
                            slot[out],
                            reads(&read),
                        ));
                    }
                }
                continue;
            }
            let gate = &self.combinational[g];
            let expr = match &gate.function {
                Function::Buffer(i) | Function::Receiver(i) => term(*i, &constant)?,
                Function::Mux { select, low, high } => {
                    let (s, l, h) = (
                        term(*select, &constant)?,
                        term(*low, &constant),
                        term(*high, &constant),
                    );
                    match s {
                        Term::Lit(0) => l?,
                        Term::Lit(_) => h?,
                        _ => {
                            let (l, h) = (l?, h?);
                            funcs.push(LineFn::Mux(s, l, h));
                            lines.push((
                                format!("  v[{}] = {} ? {} : {};", slot[out], s.c(), h.c(), l.c()),
                                slot[out],
                                reads(&[s, l, h]),
                            ));
                            continue;
                        }
                    }
                }
                Function::Lut4 { inputs, .. } if lutram_of_gate.contains_key(&g) => {
                    let cells = lutrams[lutram_of_gate[&g]];
                    let t = inputs
                        .iter()
                        .map(|i| term(*i, &constant))
                        .collect::<Result<Vec<_>>>()?;
                    let mut read = reads(&t);
                    read.extend(cells..cells + 16);
                    funcs.push(LineFn::Opaque);
                    lines.push((
                        format!(
                            "  v[{}] = v[{cells} + ({} | {} << 1 | {} << 2 | {} << 3)];",
                            slot[out],
                            t[0].c(),
                            t[1].c(),
                            t[2].c(),
                            t[3].c()
                        ),
                        slot[out],
                        read,
                    ));
                    continue;
                }
                Function::Lut4 { node, inputs } => {
                    let table = self.state.lut(*node)?;
                    let init = lut_init(table)?;
                    // An input the table ignores reads nothing (see `used`).
                    let used = used(*node);
                    let t = inputs
                        .iter()
                        .enumerate()
                        .map(|(k, i)| {
                            if used[k] {
                                term(*i, &constant)
                            } else {
                                Ok(Term::Lit(0))
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if let Some(r) = reduce(init, &t) {
                        r
                    } else {
                        let (init, t) = specialize(init, &t);
                        let index: Vec<String> = t
                            .iter()
                            .filter(|x| matches!(x, Term::Net { .. }))
                            .enumerate()
                            .map(|(k, x)| {
                                if k == 0 {
                                    x.c()
                                } else {
                                    format!("{} << {k}", x.c())
                                }
                            })
                            .collect();
                        funcs.push(LineFn::Lut(init, t));
                        lines.push((
                            format!(
                                // Keep raw LUTs 64-bit like folded tables. Clang 21's
                                // SLP vectorizer can miscompile adjacent 32-bit LUT
                                // shifts (Life, unfolded, two threads), even with
                                // independent outputs and all inputs in 0..=1.
                                "  v[{}] = ({init}ull >> ({})) & 1;",
                                slot[out],
                                index.join(" | ")
                            ),
                            slot[out],
                            reads(&t),
                        ));
                        continue;
                    }
                }
                Function::Carry {
                    node,
                    inputs,
                    cin,
                    inject,
                    carry_output,
                } => {
                    let init = lut_init(self.state.lut(*node)?)?;
                    let used = used(*node);
                    let t = inputs
                        .iter()
                        .enumerate()
                        .map(|(k, i)| {
                            if used[k] {
                                term(*i, &constant)
                            } else {
                                Ok(Term::Lit(0))
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    // An unknown carry-in that the propagate term never passes
                    // (LOGIC-mode FCO, S04) cannot reach the output, nor can
                    // one the order left out (`reads_cin`).
                    let c = match term(*cin, &constant) {
                        _ if !reads_cin(*node, inputs, *inject, *carry_output) => Term::Lit(0),
                        Ok(c) => c,
                        Err(_) if (*carry_output || !*inject) && !propagates(init, &t) => {
                            Term::Lit(0)
                        }
                        Err(_) if !*carry_output && *inject => Term::Lit(0),
                        Err(e) => return Err(e),
                    };
                    // Propagate and generate, folded when they depend on at
                    // most one net (a counter bit propagates its own Q).
                    let pr = reduce(init, &t);
                    let gr = if *inject {
                        Some(Term::Lit(0))
                    } else {
                        reduce(init, &t[..2])
                    };
                    let p = pr.map_or_else(
                        || {
                            format!(
                                "(({init}ull >> ({} | {} << 1 | {} << 2 | {} << 3)) & 1)",
                                t[0].c(),
                                t[1].c(),
                                t[2].c(),
                                t[3].c()
                            )
                        },
                        Term::c,
                    );
                    let g = gr.map_or_else(
                        || format!("(({init}ull >> ({} | {} << 1)) & 1)", t[0].c(), t[1].c()),
                        Term::c,
                    );
                    let folded = match (*carry_output, *inject, pr) {
                        (true, _, Some(Term::Lit(1))) => Some(c),
                        (true, _, Some(Term::Lit(_))) => gr,
                        (false, true, _) => pr,
                        (false, false, Some(Term::Lit(b))) => Some(c.invert_if(b == 1)),
                        _ => None,
                    };
                    if let Some(expr) = folded {
                        expr
                    } else {
                        let line = if *carry_output {
                            let line = match gr {
                                Some(Term::Lit(0)) => format!("{p} & {}", c.c()),
                                Some(Term::Lit(_)) => format!("({p} ^ 1) | {}", c.c()),
                                _ => format!("{p} ? {} : {g}", c.c()),
                            };
                            line
                        } else if *inject {
                            p
                        } else {
                            format!("{p} ^ {}", c.c())
                        };
                        let mut read = reads(&t);
                        read.extend(reads(&[c]));
                        funcs.push(LineFn::Carry {
                            init,
                            t: [t[0], t[1], t[2], t[3]],
                            c,
                            inject: *inject,
                            carry_output: *carry_output,
                        });
                        lines.push((format!("  v[{}] = {line};", slot[out]), slot[out], read));
                        continue;
                    }
                }
            };
            // A buffer or receiver is no gate: its net reads its input.
            if let Term::Lit(b) = expr {
                initial[slot[out]] = b;
            }
            constant[out] = Some(expr);
        }
        // A GSR primitive driven by logic acts through the system controller
        // (kernel/system.rs). Without wake phases, which fast mode refuses,
        // its GSR and user GSR outputs are the primitive's input, polarity
        // applied: they become that net for the registers, EBR and MULT18
        // stages that read them. A gate reading them has folded them already.
        if let Some(user_gsr) = self.user_gsr() {
            if let Ok(t @ Term::Net { .. }) = term(user_gsr, &constant) {
                for port in [SystemPort::Gsr, SystemPort::UserGsr] {
                    let Some(s) = self.system_signal(port) else {
                        continue;
                    };
                    let r = root[s.index];
                    ensure!(
                        !order.iter().any(|&g| inputs_of(g)
                            .iter()
                            .any(|i| i.signal().is_some_and(|i| root[i.index] == r))),
                        "fast mode does not run logic that reads the {} of a GSR driven by logic; the event kernel does",
                        port.name()
                    );
                    constant[r] = Some(t);
                }
            }
        }
        let gates = lines.len();
        // The gates downstream of some slots, in dependency order.
        let cone = |sources: &[usize]| -> String {
            let mut reached: HashSet<usize> = sources.iter().copied().collect();
            let mut text = String::new();
            for (line, out, read) in &lines {
                if read.iter().any(|r| reached.contains(r)) {
                    reached.insert(*out);
                    text.push_str(line);
                    text.push('\n');
                }
            }
            text
        };
        let settle = cone(&(0..slots).collect::<Vec<_>>());
        // Clock groups: (clock, uses rising edges, uses falling edges). The
        // group's previous level has its own slot.
        let mut groups: Vec<(Term, bool, bool)> = Vec::new();
        let mut group_of_clock: HashMap<(usize, bool, u8), usize> = HashMap::new();
        let mut group = |clock: Term, rising: bool, falling: bool| {
            let key = match clock {
                Term::Lit(b) => (usize::MAX, false, b),
                Term::Net { slot, inverted } => (slot, inverted, 0),
            };
            let g = *group_of_clock.entry(key).or_insert_with(|| {
                groups.push((clock, false, false));
                groups.len() - 1
            });
            groups[g].1 |= rising;
            groups[g].2 |= falling;
            g
        };
        let mut register_resets = Vec::new();
        let mut updates: Vec<RegisterCapture> = Vec::new();
        // (group, Q slot) of every register and IOLOGIC output.
        let mut members: Vec<(usize, usize)> = Vec::new();
        // What the captures read. The data input of a register with an enable,
        // and the address and data of a LUTRAM write, are read only when their
        // enable is 1: (group, edge, the enable in C, slots read). Everything
        // else is read on every capture (`always`).
        let mut demand: Vec<(usize, Edge, String, Vec<usize>)> = Vec::new();
        let mut always: Vec<usize> = Vec::new();
        let mut preload_wave = String::new();
        let mut preload_count = 0;
        for (r, register) in self.registers.iter().enumerate() {
            let q = slot[root[self.drivers[register.driver].signal]];
            let config = &register.config;
            if register.initialize_preload && initial[q] > 1 {
                return Err(StartsUnknown(format!(
                    "the configured preload of {}",
                    self.drivers[register.driver].label
                ))
                .into());
            }
            let clock = term(config.clock, &constant)?;
            let group = group(
                clock,
                config.edge == Edge::Rising,
                config.edge == Edge::Falling,
            );
            members.push((group, q));
            let data_term = term(config.data, &constant)?;
            let enable_term = term(config.enable, &constant)?;
            match enable_term {
                Term::Lit(0) => {}
                Term::Lit(_) => always.extend(reads(&[data_term])),
                _ => {
                    always.extend(reads(&[enable_term]));
                    demand.push((group, config.edge, enable_term.c(), reads(&[data_term])));
                }
            }
            let mut read = reads(&[data_term, enable_term]);
            let preload = config.preload.map(|m| term(m, &constant)).transpose()?;
            if let Some(m) = preload {
                read.extend(reads(&[m]));
                always.extend(reads(&[m]));
            }
            // An asynchronous preload samples the same pre-publication wave
            // as the captures. Its saved M/LSR also remember changes between
            // instants; the flags below span all waves of one instant.
            let mut preload_value = preload;
            let mut preload_lsr = None;
            let mut preload_global = None;
            if let Some(m) = preload.filter(|_| {
                config
                    .reset
                    .is_some_and(|s| s.kind == ResetKind::Asynchronous)
                    || register.global_reset.is_some()
            }) {
                let p = slots;
                let lsr = config
                    .reset
                    .map(|s| term(s.input, &constant))
                    .transpose()?
                    .unwrap_or(Term::Lit(0));
                let global = register
                    .global_reset
                    .map(|s| term(s.input, &constant))
                    .transpose()?
                    .unwrap_or(Term::Lit(0));
                for t in [m, lsr, global] {
                    initial.push(match t {
                        Term::Lit(b) => b,
                        Term::Net { slot, inverted } => initial[slot] ^ u8::from(inverted),
                    });
                }
                slots += 3;
                let l = p + 1;
                let g = p + 2;
                let k = preload_count;
                preload_count += 1;
                let _ = writeln!(
                    preload_wave,
                    "    {{ u8 m = {}, lsr = {};\n      prld[{k}] |= (m != v[{p}]) | ((v[{l}] && !lsr) << 1);\n      v[{p}] = m; v[{l}] = lsr; v[{g}] = {};",
                    m.c(), lsr.c(), global.c()
                );
                if config
                    .reset
                    .is_some_and(|s| s.kind == ResetKind::Asynchronous)
                {
                    let _ = writeln!(
                        preload_wave,
                        "      if (!lsr && prld[{k}] == 3 && !v[{g}]) return fast_fail(v, 5, {r});"
                    );
                }
                preload_wave.push_str("    }\n");
                preload_value = Some(Term::Net {
                    slot: p,
                    inverted: false,
                });
                preload_lsr = Some(Term::Net {
                    slot: l,
                    inverted: false,
                });
                preload_global = Some(Term::Net {
                    slot: g,
                    inverted: false,
                });
            }
            let mut synchronous = None;
            // Asynchronous resets act on their own and also win over a
            // capture on the same wave, as in the kernel.
            let mut forced = Vec::new();
            if let Some(reset) = config.reset {
                let rv = preload_value.unwrap_or_else(|| Term::Lit(bit(reset.value)));
                let rst_term = term(reset.input, &constant)?;
                read.extend(reads(&[rst_term]));
                always.extend(reads(&[rst_term]));
                match reset.kind {
                    ResetKind::Synchronous => {
                        synchronous = Some((rst_term, rv));
                    }
                    ResetKind::Asynchronous => {
                        forced.push((preload_lsr.unwrap_or(rst_term), rv));
                    }
                }
            }
            if let Some(reset) = register.global_reset {
                let rst_term = term(reset.input, &constant)?;
                read.extend(reads(&[rst_term]));
                always.extend(reads(&[rst_term]));
                forced.push((
                    preload_global.unwrap_or(rst_term),
                    preload_value.unwrap_or_else(|| Term::Lit(bit(reset.value))),
                ));
            }
            for &(rst, rv) in &forced {
                register_resets.push((rst, q, rv));
            }
            updates.push(RegisterCapture {
                q,
                data: data_term,
                enable: enable_term,
                reset: synchronous,
                forced,
                reads: read,
                group,
                edge: config.edge,
            });
        }
        let mut asynchronous = asynchronous_registers(&register_resets);
        // IOLOGIC: (clock group, path, data terms, reset term, output slots).
        let mut paths = Vec::new();
        for io in &self.iologic {
            let clock = term(io.config.clock, &constant)?;
            let (data, rising, falling) = match &io.config.path {
                IoPath::OutputDdr { data, .. } => (data.to_vec(), true, true),
                IoPath::InputDdr { data, .. } => (vec![*data], true, true),
                IoPath::ClockGate { enable, .. } => (vec![*enable], true, true),
                IoPath::Divider { align, .. } => (vec![*align], true, true),
                IoPath::StopSync { stop, .. } => (vec![*stop], false, true),
                // d[2] is the latch's clock; it runs in asynchronous() only.
                IoPath::Latch { data, enable, .. } => {
                    (vec![*data, *enable, io.config.clock], true, false)
                }
                IoPath::GearOut { data, sclk, .. } => {
                    let mut v = data.to_vec();
                    v.push(*sclk);
                    (v, true, true)
                }
                IoPath::GearIn { data, sclk, .. } => (vec![*data, *sclk], true, true),
                IoPath::GearOut7 { data, sclk, .. } => {
                    let mut v = data.to_vec();
                    v.push(*sclk);
                    (v, true, true)
                }
                IoPath::GearIn7 { data, sclk, .. } => (vec![*data, *sclk], true, true),
                IoPath::DelayCounter {
                    loadn, direction, ..
                } => (vec![*loadn, *direction], true, true),
                IoPath::Register {
                    data,
                    enable,
                    sync_reset,
                    ..
                } => (
                    vec![
                        *data,
                        *enable,
                        sync_reset.unwrap_or(Input::Constant(Logic::Zero)),
                    ],
                    true,
                    false,
                ),
            };
            let g = group(clock, rising, falling);
            let data = data
                .into_iter()
                .map(|d| term(d, &constant))
                .collect::<Result<Vec<_>>>()?;
            // The asynchronous resets: the path's own and, for an IO register
            // with GSR=ENABLED, the global reset.
            let mut reset: Vec<Term> = io
                .config
                .reset
                .map(|r| term(r, &constant))
                .transpose()?
                .into_iter()
                .collect();
            // The GSR of an IO register or an X1 DDR path (HI05, HI09).
            if let Some(g) = io.config.path.global_reset() {
                reset.push(term(g, &constant)?);
            }
            // DELAYF's LOADN, active low, loads DEL_VALUE at once: an
            // asynchronous "reset" whose value differs per bit (below).
            if let IoPath::DelayCounter { loadn, .. } = &io.config.path {
                let load = match *loadn {
                    Input::Constant(v) => Input::Constant(v.invert()),
                    Input::Signal(s) => Input::Inverted(s),
                    Input::Inverted(s) => Input::Signal(s),
                };
                reset.push(term(load, &constant)?);
            }
            let outputs: Vec<usize> = io
                .drivers
                .iter()
                .map(|&d| slot[root[self.drivers[d].signal]])
                .collect();
            members.extend(outputs.iter().map(|&o| (g, o)));
            paths.push((g, data, reset, outputs));
        }
        // DSP register stages (fast_dsp.rs): rising-edge captures, with
        // falling-edge captures where the selected divider is disabled.
        let dsp_captures = dsp.captures(self, &dsp_regs, &|i| term(i, &constant))?;
        let dsp_groups: Vec<usize> = dsp_captures
            .iter()
            .map(|c| group(c.clock, true, c.both_edges))
            .collect();
        members.extend(
            dsp_captures
                .iter()
                .zip(&dsp_groups)
                .map(|(c, &g)| (g, c.slot)),
        );
        // EBR: the clock group of each port and the output slots of each side.
        let mut memories: Vec<([usize; 2], [Vec<usize>; 2])> = Vec::new();
        for block in &self.ebr {
            let mut g = [0; 2];
            let mut outputs: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
            for (side, port) in block.config.ports.iter().enumerate() {
                g[side] = group(term(port.clock, &constant)?, true, false);
            }
            for (side, out) in outputs.iter_mut().enumerate() {
                for &d in block.outputs[side].iter().flatten() {
                    out.push(slot[root[self.drivers[d].signal]]);
                }
                // A 36-bit PDP read fills both paths, DOA too, on CLKB.
                let owner = if block.config.wide_read() { 1 } else { side };
                members.extend(out.iter().map(|&o| (g[owner], o)));
            }
            memories.push((g, outputs));
        }
        // LUTRAM write ports: (group, edge, the store). A store reads only
        // nets and its cells are read only by gates, so it can run with the
        // IOLOGIC captures, before any flip-flop is written. The cells are
        // members of the group, so its settle reaches the readers.
        let mut lut_stores: Vec<(usize, Edge, String)> = Vec::new();
        let mut lut_terms = Vec::new();
        for (w, writer) in self.lut_writes.iter().enumerate() {
            if !lutram_active[w] {
                continue;
            }
            let config = &writer.config;
            let enable = term(config.enable, &constant)?;
            let inhibit = term(writer.inhibit, &constant)?;
            let test = match (enable, inhibit) {
                (Term::Lit(0), _) | (_, Term::Lit(1)) => continue,
                (Term::Lit(_), Term::Lit(_)) => None,
                (e, Term::Lit(_)) => Some(e.c()),
                (Term::Lit(_), i) => Some(format!("!{}", i.c())),
                (e, i) => Some(format!("{} && !{}", e.c(), i.c())),
            };
            let condition = test
                .as_ref()
                .map_or_else(String::new, |t| format!("if ({t}) "));
            let clock = term(config.clock, &constant)?;
            let g = group(
                clock,
                config.edge == Edge::Rising,
                config.edge == Edge::Falling,
            );
            let cells = lutrams[w];
            members.extend((cells..cells + 16).map(|c| (g, c)));
            let a = config
                .write_address
                .iter()
                .map(|&i| term(i, &constant))
                .collect::<Result<Vec<_>>>()?;
            let data = term(config.data, &constant)?;
            always.extend(reads(&[enable, inhibit]));
            let mut written = reads(&a);
            written.extend(reads(&[data]));
            match test {
                Some(t) => demand.push((g, config.edge, format!("({t})"), written)),
                None => always.extend(written),
            }
            lut_stores.push((
                g,
                config.edge,
                format!(
                    "  {condition}v[{cells} + ({} | {} << 1 | {} << 2 | {} << 3)] = {};\n",
                    a[0].c(),
                    a[1].c(),
                    a[2].c(),
                    a[3].c(),
                    data.c()
                ),
            ));
            lut_terms.extend(a.into_iter().chain([enable, inhibit, data]));
        }
        let q_register: HashMap<usize, usize> =
            updates.iter().enumerate().map(|(r, c)| (c.q, r)).collect();
        let all: Vec<usize> = (0..updates.len()).collect();
        let (general_predicates, capture, commit) = place_captures(&all, &updates, true);
        let q_slots: Vec<usize> = self
            .registers
            .iter()
            .map(|r| slot[root[self.drivers[r.driver].signal]])
            .chain(paths.iter().flat_map(|p| p.3.iter().copied()))
            .chain(memories.iter().flat_map(|m| m.1.iter().flatten().copied()))
            .chain(dsp_captures.iter().map(|c| c.slot))
            .collect();
        let settle_registers = cone(&q_slots);
        // After a reset most gates are downstream of the registers: then the
        // full settle does (with a few gates more, which come out the same),
        // and the program is not twice as large.
        let settle_registers = if 2 * settle_registers.lines().count() >= settle.lines().count() {
            "  settle(v);\n".to_string()
        } else {
            settle_registers
        };
        let mut input_slots: Vec<usize> = input_of
            .values()
            .copied()
            .chain(generated_nets.iter().map(|&s| slot[s]))
            .collect();
        input_slots.sort_unstable();
        input_slots.dedup();
        let mut settle_input = String::new();
        for &i in &input_slots {
            let _ = write!(settle_input, "  case {i}:\n{}    break;\n", cone(&[i]));
        }
        let prev0 = slots;
        slots += groups.len();
        let mut edges = String::new();
        let mut any = Vec::new();
        for (i, (clock, rising, falling)) in groups.iter().enumerate() {
            let p = prev0 + i;
            let now = clock.c();
            initial.push(match clock {
                Term::Lit(b) => *b,
                Term::Net { slot, inverted } => initial[*slot] ^ u8::from(*inverted),
            });
            let _ = writeln!(
                edges,
                "  u8 now{i} = {now}; u8 rise{i} = !v[{p}] && now{i}; u8 fall{i} = v[{p}] && !now{i}; v[{p}] = now{i};"
            );
            // Only the edges that some member uses wake the group.
            if *rising {
                any.push(format!("rise{i}"));
            }
            if *falling {
                any.push(format!("fall{i}"));
            }
        }
        // IOLOGIC stages get slots after the groups. Their next values are
        // computed before any flip-flop is written and stored after, so both
        // sample the state from before the edge.
        let (mut io_capture, mut io_commit) = (String::new(), String::new());
        // (group, edge, target slot, value on that edge) of each IOLOGIC stage
        // and output that the edge changes.
        let mut io_edge: Vec<(usize, Edge, usize, String)> = Vec::new();
        for (i, ((g, data, reset, outputs), io)) in paths.iter().zip(&self.iologic).enumerate() {
            let s0 = slots;
            slots += io.stages.len();
            for &stage in &io.stages {
                initial.push(match bit(stage) {
                    2 => return Err(StartsUnknown("an IOLOGIC stage".into()).into()),
                    b => b,
                });
            }
            let d: Vec<String> = data.iter().map(|t| t.c()).collect();
            let s = |k: usize| format!("v[{}]", s0 + k);
            let o = |k: usize| format!("v[{}]", outputs[k]);
            // (target slot, next value)
            let (next, reset_value): (Vec<(usize, String)>, u8) = match &io.config.path {
                IoPath::OutputDdr { .. } => (
                    vec![
                        (s0, format!("rise{g} ? {} : {}", d[0], s(0))),
                        (s0 + 1, format!("rise{g} ? {} : {}", d[1], s(1))),
                        (s0 + 2, format!("rise{g} ? {} : {}", s(0), s(2))),
                        (s0 + 3, format!("rise{g} ? {} : {}", s(1), s(3))),
                        (s0 + 4, format!("rise{g} ? {} : {}", s(3), s(4))),
                        (
                            outputs[0],
                            format!("rise{g} ? {} : fall{g} ? {} : {}", s(2), s(4), o(0)),
                        ),
                    ],
                    0,
                ),
                IoPath::InputDdr { .. } => (
                    vec![
                        (outputs[0], format!("rise{g} ? {} : {}", s(0), o(0))),
                        (outputs[1], format!("rise{g} ? {} : {}", s(1), o(1))),
                        (s0, format!("rise{g} ? {} : {}", d[0], s(0))),
                        (s0 + 1, format!("fall{g} ? {} : {}", d[0], s(1))),
                    ],
                    0,
                ),
                IoPath::ClockGate { .. } => (
                    vec![(
                        outputs[0],
                        format!("rise{g} ? {} : fall{g} ? 0 : {}", d[0], o(0)),
                    )],
                    0,
                ),
                IoPath::Divider { seven_halves, .. } => {
                    let (rise, fall) = divider_c(&s, &o(0), &d[0], *seven_halves);
                    let next = rise
                        .iter()
                        .enumerate()
                        .map(|(k, r)| {
                            let target = if k == DIVIDER_STAGES {
                                outputs[0]
                            } else {
                                s0 + k
                            };
                            let f = &fall[k];
                            let keep = if k == DIVIDER_STAGES { o(0) } else { s(k) };
                            let value = if *f == keep {
                                format!("rise{g} ? {r} : {keep}")
                            } else {
                                format!("rise{g} ? {r} : fall{g} ? {f} : {keep}")
                            };
                            (target, value)
                        })
                        .collect();
                    (next, 0)
                }
                IoPath::StopSync { .. } => (
                    vec![
                        (s0, format!("fall{g} ? {} : {}", d[0], s(0))),
                        (s0 + 1, format!("fall{g} ? {} : {}", s(0), s(1))),
                        (s0 + 2, format!("fall{g} ? {} : {}", s(1), s(2))),
                        (outputs[0], format!("fall{g} ? {} : {}", s(2), o(0))),
                    ],
                    0,
                ),
                IoPath::GearOut { .. } => {
                    // Stages: w0..w3, m; d[0..3] the word, d[4] SCLK.
                    let mut next: Vec<(usize, String)> = (0..4)
                        .map(|k| {
                            (
                                s0 + k,
                                format!("rise{g} ? ({} ? {} : {}) : {}", d[4], d[k], s(k), s(k)),
                            )
                        })
                        .collect();
                    next.push((s0 + 4, format!("rise{g} ? {} : {}", d[4], s(4))));
                    next.push((
                        outputs[0],
                        format!(
                            "rise{g} ? ({} ? {} : {}) : fall{g} ? ({} ? {} : {}) : {}",
                            d[4],
                            d[0],
                            s(2),
                            s(4),
                            s(1),
                            s(3),
                            o(0)
                        ),
                    ));
                    (next, 0)
                }
                IoPath::GearIn { .. } => {
                    // Stages: s0..s3, m; d[0] the pad, d[1] SCLK.
                    let mut next: Vec<(usize, String)> = (0..4)
                        .map(|k| {
                            (
                                outputs[k],
                                format!("rise{g} ? ({} ? {} : {}) : {}", d[1], s(k), o(k), o(k)),
                            )
                        })
                        .collect();
                    next.push((
                        s0,
                        format!("rise{g} ? ({} ? {} : {}) : {}", d[1], d[0], s(0), s(0)),
                    ));
                    next.push((
                        s0 + 2,
                        format!("rise{g} ? ({} ? {} : {}) : {}", d[1], s(2), d[0], s(2)),
                    ));
                    next.push((s0 + 4, format!("rise{g} ? {} : {}", d[1], s(4))));
                    next.push((
                        s0 + 1,
                        format!("fall{g} ? ({} ? {} : {}) : {}", s(4), d[0], s(1), s(1)),
                    ));
                    next.push((
                        s0 + 3,
                        format!("fall{g} ? ({} ? {} : {}) : {}", s(4), s(3), d[0], s(3)),
                    ));
                    (next, 0)
                }
                IoPath::Latch { .. } => (Vec::new(), 0),
                IoPath::DelayCounter { .. } => {
                    // DIRECTION taken at MOVE's rise (stage 0), the step at its fall.
                    let mut next: Vec<(usize, String)> = (0..7)
                        .map(|k| {
                            (
                                outputs[k],
                                format!("fall{g} ? {} : {}", delay_c(&o, &s(0), k), o(k)),
                            )
                        })
                        .collect();
                    next.push((s0, format!("rise{g} ? {} : {}", d[1], s(0))));
                    (next, 0)
                }
                IoPath::GearOut7 { .. } | IoPath::GearIn7 { .. } => {
                    // Both edges act the same (kernel/iologic.rs).
                    let next = gear7_c(&io.config.path, &s, &o, &d, s0, outputs)
                        .into_iter()
                        .map(|(target, value, keep)| {
                            (target, format!("(rise{g} | fall{g}) ? ({value}) : {keep}"))
                        })
                        .collect();
                    (next, 0)
                }
                IoPath::Register { value, .. } => (
                    vec![(
                        outputs[0],
                        format!(
                            "rise{g} ? ({} ? {} : ({} ? {} : {})) : {}",
                            d[2],
                            bit(*value),
                            d[1],
                            d[0],
                            o(0),
                            o(0)
                        ),
                    )],
                    bit(*value),
                ),
            };
            let edged: Vec<(Edge, usize, String)> = match &io.config.path {
                IoPath::OutputDdr { .. } => vec![
                    (Edge::Rising, s0, d[0].clone()),
                    (Edge::Rising, s0 + 1, d[1].clone()),
                    (Edge::Rising, s0 + 2, s(0)),
                    (Edge::Rising, s0 + 3, s(1)),
                    (Edge::Rising, s0 + 4, s(3)),
                    (Edge::Rising, outputs[0], s(2)),
                    (Edge::Falling, outputs[0], s(4)),
                ],
                IoPath::InputDdr { .. } => vec![
                    (Edge::Rising, outputs[0], s(0)),
                    (Edge::Rising, outputs[1], s(1)),
                    (Edge::Rising, s0, d[0].clone()),
                    (Edge::Falling, s0 + 1, d[0].clone()),
                ],
                IoPath::ClockGate { .. } => vec![
                    (Edge::Rising, outputs[0], d[0].clone()),
                    (Edge::Falling, outputs[0], "0".into()),
                ],
                IoPath::Divider { seven_halves, .. } => {
                    let (rise, fall) = divider_c(&s, &o(0), &d[0], *seven_halves);
                    let mut v = Vec::new();
                    for k in 0..=DIVIDER_STAGES {
                        let (target, keep) = if k == DIVIDER_STAGES {
                            (outputs[0], o(0))
                        } else {
                            (s0 + k, s(k))
                        };
                        v.push((Edge::Rising, target, rise[k].clone()));
                        if fall[k] != keep {
                            v.push((Edge::Falling, target, fall[k].clone()));
                        }
                    }
                    v
                }
                IoPath::StopSync { .. } => vec![
                    (Edge::Falling, s0, d[0].clone()),
                    (Edge::Falling, s0 + 1, s(0)),
                    (Edge::Falling, s0 + 2, s(1)),
                    (Edge::Falling, outputs[0], s(2)),
                ],
                IoPath::GearOut { .. } => {
                    let mut v: Vec<(Edge, usize, String)> = (0..4)
                        .map(|k| {
                            (
                                Edge::Rising,
                                s0 + k,
                                format!("({} ? {} : {})", d[4], d[k], s(k)),
                            )
                        })
                        .collect();
                    v.push((Edge::Rising, s0 + 4, d[4].clone()));
                    v.push((
                        Edge::Rising,
                        outputs[0],
                        format!("({} ? {} : {})", d[4], d[0], s(2)),
                    ));
                    v.push((
                        Edge::Falling,
                        outputs[0],
                        format!("({} ? {} : {})", s(4), s(1), s(3)),
                    ));
                    v
                }
                IoPath::GearIn { .. } => {
                    let mut v: Vec<(Edge, usize, String)> = (0..4)
                        .map(|k| {
                            (
                                Edge::Rising,
                                outputs[k],
                                format!("({} ? {} : {})", d[1], s(k), o(k)),
                            )
                        })
                        .collect();
                    v.push((
                        Edge::Rising,
                        s0,
                        format!("({} ? {} : {})", d[1], d[0], s(0)),
                    ));
                    v.push((
                        Edge::Rising,
                        s0 + 2,
                        format!("({} ? {} : {})", d[1], s(2), d[0]),
                    ));
                    v.push((Edge::Rising, s0 + 4, d[1].clone()));
                    v.push((
                        Edge::Falling,
                        s0 + 1,
                        format!("({} ? {} : {})", s(4), d[0], s(1)),
                    ));
                    v.push((
                        Edge::Falling,
                        s0 + 3,
                        format!("({} ? {} : {})", s(4), s(3), d[0]),
                    ));
                    v
                }
                IoPath::Latch { .. } => Vec::new(),
                IoPath::DelayCounter { .. } => {
                    let mut v: Vec<(Edge, usize, String)> = (0..7)
                        .map(|k| (Edge::Falling, outputs[k], delay_c(&o, &s(0), k)))
                        .collect();
                    v.push((Edge::Rising, s0, d[1].clone()));
                    v
                }
                IoPath::GearOut7 { .. } | IoPath::GearIn7 { .. } => {
                    let mut v = Vec::new();
                    for (target, value, _) in gear7_c(&io.config.path, &s, &o, &d, s0, outputs) {
                        v.push((Edge::Rising, target, value.clone()));
                        v.push((Edge::Falling, target, value));
                    }
                    v
                }
                IoPath::Register { value, .. } => vec![(
                    Edge::Rising,
                    outputs[0],
                    format!(
                        "({} ? {} : ({} ? {} : {}))",
                        d[2],
                        bit(*value),
                        d[1],
                        d[0],
                        o(0)
                    ),
                )],
            };
            let reset: Option<String> = (!reset.is_empty()).then(|| {
                let any: Vec<String> = reset.iter().map(|r| r.c()).collect();
                format!("({})", any.join(" | "))
            });
            // The value a reset gives each target: one for the path, or per
            // bit for DELAYF's counter (DEL_VALUE).
            let reset_to = |target: usize| -> u8 {
                match &io.config.path {
                    IoPath::DelayCounter { value, .. } => outputs
                        .iter()
                        .position(|&o| o == target)
                        .map_or(reset_value, |k| (value >> k) & 1),
                    _ => reset_value,
                }
            };
            for (edge, target, value) in edged {
                let value = match &reset {
                    Some(r) => format!("{r} ? {} : ({value})", reset_to(target)),
                    None => value,
                };
                io_edge.push((*g, edge, target, value));
            }
            for (k, (target, value)) in next.iter().enumerate() {
                // On an edge the reset wins; without one the stage keeps its
                // value, and asynchronous() clears it after the captures.
                let to = reset_to(*target);
                let value = match &reset {
                    Some(r) => {
                        format!("(rise{g} | fall{g}) ? ({r} ? {to} : ({value})) : v[{target}]")
                    }
                    None => value.clone(),
                };
                let _ = writeln!(io_capture, "  u8 io{i}_{k} = {value};");
                let _ = writeln!(io_commit, "  v[{target}] = io{i}_{k};");
                if let Some(r) = &reset {
                    let _ = writeln!(
                        asynchronous,
                        "  if ({r} && v[{target}] != {to}) {{ v[{target}] = {to}; changed = 1; }}"
                    );
                }
            }
            // An IO latch (HI10) follows D while its clock is low and CE
            // high, at any time; its reset wins.
            if let IoPath::Latch { .. } = &io.config.path {
                let q = outputs[0];
                let follow = format!(
                    "if (!{} && {} && v[{q}] != {}) {{ v[{q}] = {}; changed = 1; }}",
                    d[2], d[1], d[0], d[0]
                );
                let _ = match &reset {
                    Some(r) => writeln!(
                        asynchronous,
                        "  if ({r}) {{ if (v[{q}] != 0) {{ v[{q}] = 0; changed = 1; }} }} else {follow}"
                    ),
                    None => writeln!(asynchronous, "  {follow}"),
                };
            }
        }
        // MULT18 stages: computed before any flip-flop is written and stored
        // after, as the IOLOGIC ones; a clear acts at any time.
        for (i, (c, &g)) in dsp_captures.iter().zip(&dsp_groups).enumerate() {
            let wrap = |value: String| {
                c.clears
                    .iter()
                    .fold(value, |v, r| format!("{r} ? 0 : ({v})"))
            };
            io_edge.push((g, Edge::Rising, c.slot, wrap(c.next.clone())));
            if c.both_edges {
                io_edge.push((g, Edge::Falling, c.slot, wrap(c.next.clone())));
            }
            let edge = if c.both_edges {
                format!("(rise{g} | fall{g})")
            } else {
                format!("rise{g}")
            };
            let _ = writeln!(
                io_capture,
                "  u8 dsp{i} = {edge} ? {} : v[{}];",
                wrap(c.next.clone()),
                c.slot
            );
            let _ = writeln!(io_commit, "  v[{}] = dsp{i};", c.slot);
            for r in &c.clears {
                let _ = writeln!(
                    asynchronous,
                    "  if ({r} && v[{s}]) {{ v[{s}] = 0; changed = 1; }}",
                    s = c.slot
                );
            }
        }
        // Slots that gates, registers, IOLOGIC, EBR ports or clock groups
        // read. An unknown value elsewhere (an EBR port nothing reads, say)
        // cannot reach the logic.
        let mut observed: HashSet<usize> = lines
            .iter()
            .flat_map(|l| l.2.iter().copied())
            .chain(updates.iter().flat_map(|u| u.reads.iter().copied()))
            .collect();
        for &t in &lut_terms {
            if let Term::Net { slot, .. } = t {
                observed.insert(slot);
            }
        }
        let mut look = |t: Term| {
            if let Term::Net { slot, .. } = t {
                observed.insert(slot);
                always.push(slot);
            }
        };
        for (clock, _, _) in &groups {
            look(*clock);
        }
        let dtr_starts = self
            .dtrs
            .iter()
            .map(|d| term(d.config.start, &constant))
            .collect::<Result<Vec<_>>>()?;
        for &start in &dtr_starts {
            look(start);
        }
        let dcs_inputs = self.dcs.iter().enumerate().map(|(i, d)| {
            let inputs = [d.config.clocks[0], d.config.clocks[1], d.config.select[0], d.config.select[1], d.config.mode_select];
            let terms = inputs.into_iter().map(|input| {
                if matches!(input, Input::Constant(v) if v.bit().is_none()) || input.signal().is_some_and(|s| matches!(source[root[s.index]], Some(Source::Constant(v)) if v.bit().is_none())) {
                    return Ok(Term::Lit(2));
                }
                term(input, &constant).with_context(|| format!("fast mode DCSC {i} has an unknown clock/control; a two-valued run cannot reproduce X"))
            }).collect::<Result<Vec<_>>>()?;
            Ok(<[Term; 5]>::try_from(terms).unwrap_or_else(|_| unreachable!()))
        }).collect::<Result<Vec<_>>>()?;
        for inputs in &dcs_inputs {
            for &input in inputs {
                look(input);
            }
        }
        for c in &dsp_captures {
            c.reads.iter().for_each(|&t| look(t));
        }
        for (_, data, reset, _) in &paths {
            data.iter().chain(reset).for_each(|&t| look(t));
        }
        for block in &self.ebr {
            let c = &block.config;
            for port in &c.ports {
                let inputs = port
                    .address
                    .iter()
                    .chain(&port.data)
                    .chain(&port.select)
                    .chain([
                        &port.clock,
                        &port.enable,
                        &port.output_enable,
                        &port.write_enable,
                        &port.reset,
                    ]);
                for &input in inputs.chain([&c.write_inhibit]).chain(&c.global_reset) {
                    if let Ok(t) = term(input, &constant) {
                        look(t);
                    }
                }
            }
        }
        // An EBR port whose outputs neither the logic reads nor a probe
        // watches is not evaluated.
        // A net that can float with no pull holds Z, which only a probe may
        // read: logic, a capture or a memory port reading it would need the
        // kernel's X.
        if let Some(&(_, net)) = floating.iter().find(|(s, _)| observed.contains(s)) {
            bail!(
                "fast mode refuses a pad that can float with no pull while the logic reads it ({}); the event kernel makes that read X",
                net_name(net)
            );
        }
        let mut needed = observed.clone();
        for s in (0..n).filter(|&s| self.signal(s).watched) {
            let watched = match constant[root[s]] {
                Some(Term::Net { slot, .. }) => slot,
                _ => slot[root[s]],
            };
            needed.insert(watched);
            always.push(watched);
        }
        always.extend(tri_slots);
        // Gates on demand: those that only the demanded reads use, directly or
        // through other such gates. They are evaluated in the capture that
        // reads them, before any register is written, when one of the enables
        // is 1, so the settles skip them. A gate that nothing reads, not even
        // a probe, is evaluated only by the full settles. Only probed signals
        // are therefore up to date between captures.
        let line_of: HashMap<usize, usize> =
            lines.iter().enumerate().map(|(i, l)| (l.1, i)).collect();
        let mut settled = vec![false; lines.len()];
        let mut stack: Vec<usize> = always
            .iter()
            .filter_map(|s| line_of.get(s).copied())
            .collect();
        while let Some(i) = stack.pop() {
            if !std::mem::replace(&mut settled[i], true) {
                stack.extend(lines[i].2.iter().filter_map(|s| line_of.get(s).copied()));
            }
        }
        // The demands that need each gate on demand.
        let mut demands_of: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
        for (d, (_, _, _, slots)) in demand.iter().enumerate() {
            let mut seen = HashSet::new();
            let mut stack: Vec<usize> = slots
                .iter()
                .filter_map(|s| line_of.get(s).copied())
                .collect();
            while let Some(i) = stack.pop() {
                if !settled[i] && seen.insert(i) {
                    demands_of[i].push(d);
                    stack.extend(lines[i].2.iter().filter_map(|s| line_of.get(s).copied()));
                }
            }
        }
        let demanded_gates = settled.iter().filter(|&&s| !s).count();
        // The nets not up to date after a capture's settle: the gates on
        // demand, and below the nets folded away in a group's settle.
        let mut stale: HashSet<usize> = (0..lines.len())
            .filter(|&i| !settled[i])
            .map(|i| lines[i].1)
            .collect();
        // The gates on demand of the capture on (group, edge), in dependency
        // order. Gates whose demands there share their enables are scheduled
        // together, and a run gets one test of those enables. A run of fewer
        // than four gates is cheaper without one.
        let on_demand = |g: usize, edge: Edge| -> String {
            let mut signature: HashMap<usize, Vec<&str>> = HashMap::new();
            for i in (0..lines.len()).filter(|&i| !settled[i]) {
                let mut enables: Vec<&str> = demands_of[i]
                    .iter()
                    .filter(|&&d| demand[d].0 == g && demand[d].1 == edge)
                    .map(|&d| demand[d].2.as_str())
                    .collect();
                if !enables.is_empty() {
                    enables.sort_unstable();
                    enables.dedup();
                    signature.insert(i, enables);
                }
            }
            let mut waiting: HashMap<usize, usize> = HashMap::new();
            let mut readers: HashMap<usize, Vec<usize>> = HashMap::new();
            for &i in signature.keys() {
                let inputs: Vec<usize> = lines[i]
                    .2
                    .iter()
                    .filter_map(|s| line_of.get(s).copied())
                    .filter(|j| signature.contains_key(j))
                    .collect();
                for &j in &inputs {
                    readers.entry(j).or_default().push(i);
                }
                waiting.insert(i, inputs.len());
            }
            let mut ready: BTreeSet<usize> =
                waiting.iter().filter(|w| *w.1 == 0).map(|w| *w.0).collect();
            let mut runs: Vec<(&Vec<&str>, Vec<usize>)> = Vec::new();
            while !ready.is_empty() {
                let current = runs.last().map(|r| r.0);
                let next = current
                    .and_then(|c| ready.iter().copied().find(|i| &signature[i] == c))
                    .unwrap_or_else(|| *ready.iter().next().unwrap());
                ready.remove(&next);
                match runs.last_mut() {
                    Some(run) if *run.0 == signature[&next] => run.1.push(next),
                    _ => runs.push((&signature[&next], vec![next])),
                }
                for &reader in readers.get(&next).map(Vec::as_slice).unwrap_or_default() {
                    let w = waiting.get_mut(&reader).unwrap();
                    *w -= 1;
                    if *w == 0 {
                        ready.insert(reader);
                    }
                }
            }
            let mut text = String::new();
            for (enables, run) in runs {
                let body: String = run.iter().map(|&i| format!("{}\n", lines[i].0)).collect();
                if run.len() < 4 {
                    text.push_str(&body);
                } else {
                    let _ = write!(text, "  if ({}) {{\n{body}  }}\n", enables.join(" | "));
                }
            }
            text
        };
        // Several groups still capture the same old state. Evaluate their
        // union of demanded gates before any capture/commit, in the original
        // dependency order, and only for edges and enables that actually fire.
        let demand_edges: Vec<String> = groups
            .iter()
            .enumerate()
            .flat_map(|(g, (_, rising, falling))| {
                [(rising, "rise"), (falling, "fall")]
                    .into_iter()
                    .filter(|(used, _)| **used)
                    .map(move |(_, name)| format!("{name}{g}"))
            })
            .collect();
        let demand_parameters: String = demand_edges.iter().map(|e| format!(", u8 {e}")).collect();
        let demand_arguments: String = demand_edges.iter().map(|e| format!(", {e}")).collect();
        let mut all_on_demand = String::new();
        let mut previous = Vec::new();
        for i in (0..lines.len()).filter(|&i| !settled[i]) {
            let mut guards: Vec<String> = demands_of[i]
                .iter()
                .map(|&d| {
                    let (g, edge, enable, _) = &demand[d];
                    let name = if *edge == Edge::Rising {
                        "rise"
                    } else {
                        "fall"
                    };
                    format!("({name}{g} && ({enable}))")
                })
                .collect();
            guards.sort_unstable();
            guards.dedup();
            if guards.is_empty() {
                continue;
            }
            if guards != previous {
                if !previous.is_empty() {
                    all_on_demand.push_str("  }\n");
                }
                let _ = writeln!(all_on_demand, "  if ({}) {{", guards.join(" | "));
                previous = guards;
            }
            let _ = writeln!(all_on_demand, "{}", lines[i].0);
        }
        if !previous.is_empty() {
            all_on_demand.push_str("  }\n");
        }
        // Diagnostics belong to this simulation, just like pad_conflict.
        // One byte for the code and two for the unit returned by fast_error.
        let error_slot = slots;
        slots += 3;
        initial.resize(slots, 0);
        let mut dtrs = Vec::new();
        let mut dtr_capture = String::new();
        if !self.dtrs.is_empty() {
            dtr_capture.push_str("    int dtr_changed = 0;\n");
        }
        for (index, (dtr, start)) in self.dtrs.iter().zip(&dtr_starts).enumerate() {
            let previous = dtr
                .previous
                .bit()
                .map(u8::from)
                .ok_or_else(|| StartsUnknown(format!("STARTPULSE of {}", index)))?;
            let captured = slots;
            slots += 1;
            initial.push(previous);
            let _ = writeln!(dtr_capture, "    u8 dtr{index} = {};", start.c());
            let _ = writeln!(
                dtr_capture,
                "    if (dtr{index} > 1) return fast_fail(v, 6, {index});"
            );
            let _ = writeln!(
                dtr_capture,
                "    dtr_changed |= v[{captured}] != dtr{index};\n    v[{captured}] = dtr{index};"
            );
            dtr.timer.context("DTR has no completion timer")?;
            let completion = self.dtr_completions.map_or_else(
                || self.kernel.fast_dtr_completion(index),
                |completions| completions[index],
            );
            dtrs.push(FastDtr {
                captured,
                outputs: dtr.config.outputs.map(|s| slot[root[s.index]]),
                code: dtr.config.code,
                previous,
                completion,
            });
        }
        // PRLD's same-instant history must survive a DTR host pause. EBR
        // scratch is committed before returning and needs no persistence.
        let dtr_preload = (!dtrs.is_empty() && preload_count > 0).then_some(slots);
        if dtr_preload.is_some() {
            slots += preload_count;
            initial.resize(slots, 0);
        }
        let dcs_outputs: Vec<_> = self
            .dcs
            .iter()
            .map(|d| slot[root[d.config.output.index]])
            .collect();
        let dcs = super::fast_dcs::generate(
            &self.dcs,
            self.now,
            &dcs_inputs,
            &dcs_outputs,
            &mut initial,
        )?;
        slots = initial.len();
        let mut ebr_code = String::new();
        let mut ebrs = Vec::new();
        let mut memory_slots = Vec::new();
        for (b, (block, (groups_of, outputs))) in self.ebr.iter().zip(&memories).enumerate() {
            memory_slots.push(slots);
            debug_assert_eq!(slots, initial.len());
            let (code, asynchronous_code) = self.ebr_program(
                b,
                block,
                outputs,
                Readers {
                    logic: &observed,
                    any: &needed,
                },
                &mut initial,
                &|input| term(input, &constant),
            )?;
            slots = initial.len();
            ebr_code.push_str(&code);
            asynchronous.push_str(&asynchronous_code);
            ebrs.push((
                self.device
                    .model()
                    .resource(block.config.resource)
                    .key
                    .clone(),
                *groups_of,
            ));
        }
        let any = if any.is_empty() {
            "0".into()
        } else {
            any.join(" | ")
        };
        // The lines of `settled_cone`, by index.
        let settled_lines = |sources: &[usize]| -> Vec<usize> {
            let mut reached: HashSet<usize> = sources.iter().copied().collect();
            let mut out = Vec::new();
            for (i, (_, o, read)) in lines.iter().enumerate() {
                if settled[i] && read.iter().any(|r| reached.contains(r)) {
                    reached.insert(*o);
                    out.push(i);
                }
            }
            out
        };
        // After a capture, only the logic downstream of the groups that saw
        // an edge they use. A large one is split across threads: the lines
        // that several threads' parts read first, then each part at once.
        let (threads, thread_min) = (options.threads.clamp(1, 64), options.thread_min);
        // Collapsing: the lines each slot is read by, and the slots that
        // something other than a line reads (a capture, a LUTRAM write, an
        // EBR, a clock, a probe): those stay computed.
        let collapse_inputs = options.collapse.min(MOST_FOLDED);
        let mut readers = vec![0u32; slots];
        for (_, _, read) in &lines {
            let mut r = read.clone();
            r.sort_unstable();
            r.dedup();
            for s in r {
                readers[s] += 1;
            }
        }
        let external: HashSet<usize> = always
            .iter()
            .copied()
            .chain(updates.iter().flat_map(|u| u.reads.iter().copied()))
            .chain(demand.iter().flat_map(|d| d.3.iter().copied()))
            .chain(lut_terms.iter().filter_map(|t| match t {
                Term::Net { slot, .. } => Some(*slot),
                Term::Lit(_) => None,
            }))
            .collect();
        // A net folded away in one group's settle is up to date only after
        // the full settles. Another settle that computes a line reading it
        // (another group's, an input's, the one after a reset) and not the
        // net itself would read it stale: such a net stays computed, as
        // something outside the settles reads it. A DELAYF's CFLAG, settled
        // on the design's clock, read the ends of mux chains over the counter
        // that the MOVE clock's settle had folded into one table.
        let external: HashSet<usize> = {
            let mut external = external;
            let inside = |sources: &[usize], only_settled: bool| -> Vec<bool> {
                let mut reached: HashSet<usize> = sources.iter().copied().collect();
                let mut inside = vec![false; lines.len()];
                for (i, (_, out, read)) in lines.iter().enumerate() {
                    if (!only_settled || settled[i]) && read.iter().any(|r| reached.contains(r)) {
                        reached.insert(*out);
                        inside[i] = true;
                    }
                }
                inside
            };
            let mut settles: Vec<Vec<bool>> = (0..groups.len())
                .map(|g| {
                    let q: Vec<usize> = members.iter().filter(|m| m.0 == g).map(|m| m.1).collect();
                    inside(&q, true)
                })
                .collect();
            settles.extend(input_slots.iter().map(|&i| inside(&[i], false)));
            settles.push(inside(&q_slots, false));
            for cone in &settles {
                for (i, (_, _, read)) in lines.iter().enumerate() {
                    if cone[i] {
                        external.extend(
                            read.iter()
                                .filter(|s| line_of.get(s).is_some_and(|&j| !cone[j]))
                                .copied(),
                        );
                    }
                }
            }
            external
        };
        // `FAST_SPLIT=place`, an experiment: where each slot's net is on the
        // chip, for a split of the cones by place (`split_phase`).
        let slot_place: Option<Vec<(i32, i32)>> =
            (std::env::var("FAST_SPLIT").as_deref() == Ok("place")).then(|| {
                let mut place = vec![(0, 0); slots];
                for s in (0..n).filter(|&s| root[s] == s && slot[s] < slots) {
                    let at = &self.signal(s).wire.location;
                    place[slot[s]] = (i32::from(at.row), i32::from(at.col));
                }
                place
            });
        // One table per function: many gates of a design compute the same one.
        let mut wide = Wide::default();
        let (mut settle_lines, mut settle_phases) = (Vec::new(), Vec::new());
        let mut threaded = false;
        // The slots each thread writes, to be laid out apart.
        let mut written: Vec<Vec<usize>> = vec![Vec::new(); threads];
        // What the caller's shared lines write, which several threads read:
        // together, on as few cache lines as they fill.
        let mut shared_written: Vec<usize> = Vec::new();
        // The flip-flops, and how often each thread's lines read each (the
        // caller's shared lines count as thread 0's): each goes to the region
        // of the thread that reads it most, so that a thread finds the
        // registers it reads on a few cache lines, not one line apiece.
        let flip_flops: HashSet<usize> = self
            .registers
            .iter()
            .map(|r| slot[root[self.drivers[r.driver].signal]])
            .collect();
        let mut register_reads: HashMap<usize, Vec<u32>> = HashMap::new();
        // The flags and copies of the settles that evaluate what changed.
        let activity = fast_activity();
        // Owner capture is the default: on the M3, life_full rose from ~145
        // to ~230 kHz at 10 threads and ~105 to ~195 at 8; under load its
        // median was equal or better. `FAST_CAPTURE=caller` restores caller capture.
        let owner_capture = !activity
            && groups.len() <= 1
            && std::env::var("FAST_CAPTURE").as_deref() != Ok("caller");
        // A slot written by several threads has no single capture owner.
        let mut capture_writer: HashMap<usize, usize> = HashMap::new();
        let mut capture_owner: HashMap<usize, usize> = HashMap::new();
        let mut act: Vec<(usize, u8)> = Vec::new();
        let mut group_settles = String::new();
        let mut settle_groups = String::new();
        for (g, (_, rising, falling)) in groups.iter().enumerate() {
            let q: Vec<usize> = members.iter().filter(|m| m.0 == g).map(|m| m.1).collect();
            let cone = settled_lines(&q);
            // The settle's own lines: collapsed into tables, or as they are.
            let group_lines: Vec<(String, usize, Vec<usize>)> = if collapse_inputs > 1 {
                // Widening cone folding by default (FAST_FOLD=cones-widen):
                // life_full ran at 117-166 kHz with it on 10 threads against
                // 46-64, picorv32_console 3% faster, Life the same
                // (docs/FAST_MODE.md). `single` is the folding of one-reader
                // gates only, `cones` the cones without widening.
                let fold = std::env::var("FAST_FOLD");
                match fold.as_deref() {
                    Ok("single") => collapse(
                        &lines,
                        &funcs,
                        &cone,
                        &line_of,
                        &readers,
                        &external,
                        collapse_inputs,
                        &mut wide,
                    ),
                    fold => collapse_cones(
                        &lines,
                        &funcs,
                        &cone,
                        &line_of,
                        &readers,
                        &external,
                        collapse_inputs,
                        fold != Ok("cones"),
                        &mut wide,
                    ),
                }
            } else {
                cone.iter().map(|&i| lines[i].clone()).collect()
            };
            if std::env::var_os("FAST_THREAD_REPORT").is_some() {
                eprintln!(
                    "fast mode: group {g}: {} lines, {} after collapsing",
                    cone.len(),
                    group_lines.len()
                );
            }
            settle_lines.push(group_lines.len());
            let kept: HashSet<usize> = group_lines.iter().map(|l| l.1).collect();
            stale.extend(
                cone.iter()
                    .map(|&i| lines[i].1)
                    .filter(|o| !kept.contains(o)),
            );
            let group_line_of: HashMap<usize, usize> = group_lines
                .iter()
                .enumerate()
                .map(|(i, l)| (l.1, i))
                .collect();
            let (lines, cone, line_of) = (
                &group_lines,
                (0..group_lines.len()).collect::<Vec<_>>(),
                &group_line_of,
            );
            if threads > 1 && cone.len() >= thread_min {
                threaded = true;
                // The caller's captures, about two weights each (a load and a
                // store, often with an enable).
                let phases = split(
                    lines,
                    &cone,
                    line_of,
                    threads,
                    options.barrier_lines,
                    2 * q.len(),
                    slot_place.as_deref(),
                );
                settle_phases.push(phases.len());
                if std::env::var_os("FAST_THREAD_REPORT").is_some() {
                    for (p, phase) in phases.iter().enumerate() {
                        eprintln!(
                            "fast mode: group {g} phase {p}: {} shared, parts {:?}",
                            phase.0.len(),
                            phase.1.iter().map(Vec::len).collect::<Vec<_>>()
                        );
                    }
                }
                let text = |list: &[usize]| -> String {
                    beside_twins(lines, list)
                        .iter()
                        .map(|&i| format!("{}\n", lines[i].0))
                        .collect()
                };
                for (shared, parts) in &phases {
                    let by_thread = std::iter::once((0, shared)).chain(parts.iter().enumerate());
                    for (t, list) in by_thread {
                        for &i in list {
                            if owner_capture {
                                capture_writer
                                    .entry(lines[i].1)
                                    .and_modify(|owner| {
                                        if *owner != t {
                                            *owner = usize::MAX;
                                        }
                                    })
                                    .or_insert(t);
                            }
                            for &r in lines[i].2.iter().filter(|r| flip_flops.contains(r)) {
                                register_reads.entry(r).or_insert_with(|| vec![0; threads])[t] += 1;
                            }
                        }
                    }
                    for (t, part) in parts.iter().enumerate() {
                        written[t].extend(part.iter().map(|&i| lines[i].1));
                    }
                    shared_written.extend(shared.iter().map(|&i| lines[i].1));
                }
                let mut body = String::new();
                for (p, (shared, parts)) in phases.iter().enumerate() {
                    let names: Vec<String> = (0..threads)
                        .map(|t| format!("settle_group{g}_p{p}_t{t}"))
                        .collect();
                    for (t, (name, part)) in names.iter().zip(parts).enumerate() {
                        let body = match activity {
                            true => activity_body(lines, part, t, &mut act),
                            false => text(part),
                        };
                        let _ = write!(
                            group_settles,
                            "static void {name}(u8 *restrict v) {{\n{body}}}\n\n"
                        );
                    }
                    let _ = writeln!(
                        group_settles,
                        "static void (*const settle_group{g}_p{p}[{threads}])(u8 *restrict) = {{{}}};\n",
                        names.join(", ")
                    );
                    // A phase with lines for the caller only (life_full's
                    // population chain) runs here, without a meeting of the
                    // threads, which alone takes 0.4-0.7 us.
                    if !activity && parts.iter().skip(1).all(Vec::is_empty) {
                        let _ = writeln!(body, "{}  settle_group{g}_p{p}_t0(v);", text(shared));
                    } else {
                        let _ =
                            writeln!(body, "{}  pool_run(v, settle_group{g}_p{p});", text(shared));
                    }
                }
                let _ = write!(
                    group_settles,
                    "static void settle_group{g}(u8 *restrict v) {{\n{body}}}\n\n"
                );
            } else {
                settle_phases.push(0);
                let body: String = match activity {
                    true => activity_body(lines, &cone, 0, &mut act),
                    false => beside_twins(lines, &cone)
                        .iter()
                        .map(|&i| format!("{}\n", lines[i].0))
                        .collect(),
                };
                let _ = write!(
                    group_settles,
                    "static void settle_group{g}(u8 *restrict v) {{\n{body}}}\n\n"
                );
            }
            let edged: Vec<String> = [(*rising, "rise"), (*falling, "fall")]
                .iter()
                .filter(|e| e.0)
                .map(|e| format!("{}{g}", e.1))
                .collect();
            if !edged.is_empty() {
                let _ = writeln!(
                    settle_groups,
                    "    if ({}) settle_group{g}(v);",
                    edged.join(" | ")
                );
            }
        }
        // When one group sees an edge it uses, which is the usual case, only
        // that group's members are written, with no selection by the edge.
        // Edges of several groups on one wave take the general path.
        let mut fired = Vec::new();
        let mut blocks = String::new();
        for (g, (_, rising, falling)) in groups.iter().enumerate() {
            let mut edged = Vec::new();
            for (used, edge, name) in [
                (*rising, Edge::Rising, "rise"),
                (*falling, Edge::Falling, "fall"),
            ] {
                if !used {
                    continue;
                }
                edged.push(format!("{name}{g}"));
                let chosen: Vec<usize> = (0..updates.len())
                    .filter(|&r| updates[r].group == g && updates[r].edge == edge)
                    .collect();
                let (predicates, capture, commit) = if owner_capture && settle_phases[g] > 0 {
                    let inside: HashSet<usize> = chosen.iter().copied().collect();
                    let mut group2 = HashSet::new();
                    for &r in &chosen {
                        for slot in &updates[r].reads {
                            if let Some(&s) = q_register
                                .get(slot)
                                .filter(|&&s| s != r && inside.contains(&s))
                            {
                                group2.insert(s);
                            }
                        }
                    }
                    // Group 1 sees the old group-2 values. Close group 2
                    // over its inputs so it reads nothing group 1 writes.
                    let mut pending: Vec<usize> = group2.iter().copied().collect();
                    while let Some(r) = pending.pop() {
                        for slot in &updates[r].reads {
                            if let Some(&s) = q_register.get(slot).filter(|s| inside.contains(s)) {
                                if group2.insert(s) {
                                    pending.push(s);
                                }
                            }
                        }
                    }
                    let mut owned: Vec<Vec<usize>> = vec![Vec::new(); threads];
                    for &r in chosen.iter().filter(|r| !group2.contains(r)) {
                        let q = updates[r].q;
                        let mut by: Vec<usize> = updates[r]
                            .reads
                            .iter()
                            .filter(|&&s| s != q)
                            .filter_map(|s| capture_writer.get(s).copied())
                            .collect();
                        by.sort_unstable();
                        by.dedup();
                        let t = match by.as_slice() {
                            [t] if *t < threads => *t,
                            _ => 0,
                        };
                        capture_owner.insert(q, t);
                        owned[t].push(r);
                    }
                    if std::env::var_os("FAST_THREAD_REPORT").is_some() {
                        eprintln!(
                            "fast mode: group {g} {name} owner captures: group 1 parts {:?}, group 2 {}",
                            owned.iter().map(Vec::len).collect::<Vec<_>>(),
                            group2.len()
                        );
                    }
                    let names: Vec<String> = (0..threads)
                        .map(|t| format!("capture_g{g}_{name}_t{t}"))
                        .collect();
                    for (name, registers) in names.iter().zip(&owned) {
                        // Group 1 never reads another group-1 Q. Group 2 is
                        // still unchanged until the pool joins; per-worker
                        // predicate snapshots therefore see the old wave too.
                        let (predicates, saved) = capture_predicates(registers, &updates);
                        let body = predicates
                            + &capture_batch(
                                registers,
                                &updates,
                                &saved,
                                false,
                                CaptureStore::Direct,
                            );
                        let _ = write!(
                            group_settles,
                            "static void {name}(u8 *restrict v) {{\n{body}}}\n\n"
                        );
                    }
                    let _ = writeln!(
                        group_settles,
                        "static void (*const capture_g{g}_{name}[THREADS])(u8 *restrict) = {{{}}};\n",
                        names.join(", ")
                    );
                    let call = if owned.iter().skip(1).all(Vec::is_empty) {
                        format!("  capture_g{g}_{name}_t0(v);\n")
                    } else {
                        format!("  pool_run(v, capture_g{g}_{name});\n")
                    };
                    let serial: Vec<usize> = chosen
                        .iter()
                        .copied()
                        .filter(|r| group2.contains(r))
                        .collect();
                    let (predicates, capture, commit) = place_captures(&serial, &updates, false);
                    (predicates, call + &capture, commit)
                } else {
                    place_captures(&chosen, &updates, false)
                };
                let (mut io_capture, mut io_commit) = (String::new(), String::new());
                for (k, (_, _, target, value)) in io_edge
                    .iter()
                    .filter(|e| e.0 == g && e.1 == edge)
                    .enumerate()
                {
                    let _ = writeln!(io_capture, "  u8 io{k} = {value};");
                    let _ = writeln!(io_commit, "  v[{target}] = io{k};");
                }
                for (_, _, store) in lut_stores.iter().filter(|w| w.0 == g && w.1 == edge) {
                    io_capture.push_str(store);
                }
                if edge == Edge::Rising {
                    for (b, (_, of)) in ebrs.iter().enumerate() {
                        if of.contains(&g) {
                            let edges =
                                format!("{}, {}", u8::from(of[0] == g), u8::from(of[1] == g));
                            let _ = writeln!(
                                io_capture,
                                "  if (ebr{b}(v, {edges}, ebr_t[{b}])) return 1;"
                            );
                            let _ = writeln!(io_commit, "  ebr{b}_commit(v, {edges}, ebr_t[{b}]);");
                        }
                    }
                }
                let _ = write!(
                    blocks,
                    "    {}if ({name}{g}) {{\n{}{predicates}{io_capture}{capture}{commit}{io_commit}    settle_group{g}(v);\n    }}",
                    if blocks.is_empty() { "" } else { " else " },
                    on_demand(g, edge)
                );
            }
            if !edged.is_empty() {
                fired.push(format!("({})", edged.join(" | ")));
            }
        }
        blocks.push('\n');
        for (b, (_, of)) in ebrs.iter().enumerate() {
            let edges = format!("rise{}, rise{}", of[0], of[1]);
            io_capture.insert_str(
                0,
                &format!(
                    "  if ((rise{} | rise{}) && ebr{b}(v, {edges}, ebr_t[{b}])) return 1;\n",
                    of[0], of[1]
                ),
            );
            let _ = writeln!(io_commit, "  ebr{b}_commit(v, {edges}, ebr_t[{b}]);");
        }
        for (g, edge, store) in &lut_stores {
            let name = match edge {
                Edge::Rising => "rise",
                Edge::Falling => "fall",
            };
            let _ = write!(io_capture, "  if ({name}{g}) {{\n  {store}  }}\n");
        }
        // One group, or none (a design without a clocked register: no edge
        // ever reaches the dispatch, whose test would then be empty C).
        let dispatch = if groups.len() <= 1 {
            blocks
        } else {
            format!(
                "    if ({} == 1) {{\n{blocks}    }} else {{\n    settle_demanded(v{demand_arguments});\n{general_predicates}{io_capture}{capture}{commit}{io_commit}{settle_groups}    }}\n",
                fired.join(" + ")
            )
        };
        let registers = self.registers.len().max(1);
        let pool = if threaded {
            let mut pool = String::new();
            if std::env::var("FAST_THREAD_STEAL").is_ok_and(|v| v.trim() == "0") {
                pool.push_str("#define POOL_STEAL 0\n");
            }
            if std::env::var_os("FAST_THREAD_STATS").is_some() {
                pool.push_str("#define POOL_STATS 1\n");
            }
            pool + &POOL.replace("THREADS_COUNT", &threads.to_string())
        } else {
            "/* No settle is split across threads. */\nvoid fast_stop(void) {}\n".into()
        };
        let wide = wide.c();
        // Only a program with multipliers has their prelude, so that every
        // other program stays as it was.
        let dsp_prelude = if dsp.is_empty() {
            String::new()
        } else {
            format!("{}\n", super::fast_dsp::PRELUDE)
        };
        let has_tri_pads = pad_conflict.is_some();
        let flag = pad_conflict.unwrap_or(0);
        let error_comment = if has_tri_pads {
            format!("/* Why the program stopped: 1 no convergence, 2 a value the event kernel\n   makes unknown, 3 an EBR reset the event kernel rejects; and the unit.\n   A tristate pad driven by the design and from outside with different\n   levels sets v[{flag}] to its unit + 1, in this simulation's slots. */")
        } else {
            "/* Why the program stopped: 1 no convergence, 2 a value the event kernel\n   makes unknown, 3 an EBR reset the event kernel rejects; and the unit. */".to_string()
        };
        let tri_helper = if has_tri_pads {
            "\nstatic inline void tri_error(u8 *v, uint32_t flag, uint32_t unit) {\n  __atomic_store_n(&v[flag], (u8)(unit + 1), __ATOMIC_RELAXED);\n}"
        } else {
            ""
        };
        let mut clear_error = if has_tri_pads {
            format!("  v[{flag}] = 0;\n")
        } else {
            String::new()
        };
        if let Some(start) = dtr_preload {
            for slot in start..start + preload_count {
                let _ = writeln!(clear_error, "  v[{slot}] = 0;");
            }
        }
        let check_error = if has_tri_pads {
            format!("  if (v[{flag}]) return 1;\n")
        } else {
            String::new()
        };
        let dispatch_check = if has_tri_pads {
            format!("      if (v[{flag}]) return 1;\n")
        } else {
            String::new()
        };
        let reset_settle = if has_tri_pads {
            format!("    if (reset) {{ settle_registers(v); if (v[{flag}]) return 1; }}")
        } else {
            "    if (reset) settle_registers(v);".to_string()
        };
        let fast_point_body = if has_tri_pads {
            format!(
                r#"  v[{flag}] = 0;
  int changed = 0;
  if (n <= 64) {{
    uint64_t mask = 0;
    for (uint32_t i = 0; i < n; i++) {{
      if (v[slots[i]] != values[i]) {{
        v[slots[i]] = values[i];
        mask |= 1ull << i;
        changed = 1;
      }}
    }}
    if (!changed) return 0;
    for (uint32_t i = 0; i < n; i++)
      if (mask >> i & 1) settle_input(v, slots[i]);
  }} else {{
    for (uint32_t i = 0; i < n; i++) {{
      if (v[slots[i]] != values[i]) {{
        v[slots[i]] = values[i];
        changed = 1;
      }}
    }}
    if (!changed) return 0;
    settle(v);
  }}
  if (v[{flag}]) return 1;
  return converge(v);"#
            )
        } else {
            r#"  int changed = 0;
  for (uint32_t i = 0; i < n; i++) {
    if (v[slots[i]] != values[i]) {
      v[slots[i]] = values[i];
      settle_input(v, slots[i]);
      changed = 1;
    }
  }
  return changed ? converge(v) : 0;"#
                .to_string()
        };
        // DCS controls/clocks and DTROUT/clock batches publish every input
        // before settling, as a Stimulus batch does in advance().
        let fast_point_body = if self.dtrs.is_empty() && self.dcs.is_empty() {
            fast_point_body
        } else {
            format!("{clear_error}  for (uint32_t i = 0; i < n; i++) v[slots[i]] = values[i];\n  settle(v);\n{check_error}  return converge(v);")
        };
        let dtr_return = if self.dtrs.is_empty() {
            ""
        } else {
            "    if (dtr_changed) return 2; /* host publishes this wave's DTROUT */\n"
        };
        // Captures and commits are separate to preserve simultaneous edges.
        // Scratch belongs to this simulation, never to the DSO's statics.
        let ebr_scratch = if ebrs.is_empty() {
            String::new()
        } else {
            format!("  u8 ebr_t[{}][72];\n", ebrs.len())
        };
        let preload_scratch = if preload_count == 0 {
            String::new()
        } else if let Some(start) = dtr_preload {
            format!("  u8 *prld = &v[{start}];\n")
        } else {
            format!("  u8 prld[{preload_count}] = {{0}};\n")
        };
        let error_unit_lo = error_slot + 1;
        let error_unit_hi = error_slot + 2;
        let dcs_prelude = if self.dcs.is_empty() {
            String::new()
        } else {
            super::fast_dcs::PRELUDE.to_string()
        };
        let dcs_begin = &dcs.begin;
        let dcs_capture = &dcs.capture;
        let dcs_commit = if self.dcs.is_empty() {
            String::new()
        } else {
            format!(
                "{}    if (dcs_changed) settle(v);\n{check_error}",
                dcs.commit
            )
        };
        let dcs_pending = if self.dcs.is_empty() {
            ""
        } else {
            " && !dcs_changed"
        };
        let text = format!(
            r#"// Generated by ecp5_emu fast mode: two-valued, cycle-based.
#include <stdint.h>
{dsp_prelude}typedef uint8_t u8;

{pool}
{error_comment}
uint32_t fast_error(const u8 *v) {{
  return (uint32_t)v[{error_slot}] << 16 | (uint32_t)v[{error_unit_hi}] << 8 | v[{error_unit_lo}];
}}
static int fast_fail(u8 *v, uint32_t code, uint32_t unit) {{
  v[{error_slot}] = (u8)code;
  v[{error_unit_lo}] = (u8)unit;
  v[{error_unit_hi}] = (u8)(unit >> 8);
  return 1;
}}{tri_helper}{dcs_prelude}

/* Inside an EBR capture, 2 stands for unknown. */
#define MERGE(a, b) ((a) == (b) ? (a) : 2)
static int row_in(const uint32_t *cells, int n, uint32_t row) {{
  for (int i = 0; i < n; i++)
    if (cells[i] / 9 == row) return 1;
  return 0;
}}

static void settle(u8 *restrict v) {{
{settle}}}

/* The gates downstream of the registers, after a capture or a reset. */
static void settle_registers(u8 *restrict v) {{
{settle_registers}}}

{wide}{group_settles}/* Demanded gates of the active edges, before simultaneous captures. */
static void settle_demanded(u8 *restrict v{demand_parameters}) {{
{all_on_demand}}}

/* The gates downstream of one input slot. */
static void settle_input(u8 *restrict v, uint32_t input) {{
  switch (input) {{
{settle_input}  default:
    settle(v);
  }}
}}

/* Returns whether a reset changed a register, or -1 on an error. */
static int asynchronous(u8 *restrict v) {{
  int changed = 0;
{asynchronous}  return changed;
}}

{ebr_code}/* Captures registers and applies resets until no clock edge is left, from
   settled logic. As in the event kernel, one wave captures every register
   from the state before it, so the captures come before the asynchronous
   resets of the same wave; a register whose own reset is on takes its reset
   value (its capture includes the reset). Returns 0, or 1 on an error (see
   fast_error). */
static int converge(u8 *restrict v) {{
{ebr_scratch}{preload_scratch}  u8 n[{registers}];
  for (int iteration = 0; iteration < 10000; iteration++) {{
{dcs_capture}{dtr_capture}{preload_wave}{edges}    int edge = {any};
    if (edge) {{
{dispatch}{dispatch_check}    }}
{dcs_commit}    int reset = asynchronous(v);
    if (reset < 0) return 1;
{dtr_return}{reset_settle}
    else if (!edge{dcs_pending}) return 0;
  }}
  return fast_fail(v, 1, 0);
}}

/* After any change of the slots. */
int fast_step(u8 *restrict v) {{
{clear_error}  settle(v);
{check_error}  return converge(v);
}}

/* After a change of one input slot. */
int fast_step_input(u8 *restrict v, uint32_t input) {{
{dcs_begin}{clear_error}  settle_input(v, input);
{check_error}  return converge(v);
}}

/* Sets `n` input slots at one instant, then settles and captures. */
int fast_point(u8 *restrict v, const uint32_t *slots, const u8 *values, uint32_t n) {{
{fast_point_body}
}}

/* Runs a periodic schedule: instant i sets the slots slots[first[i]] ..
   slots[first[i + 1] - 1] and, when sample[i], stores the n probe slots
   after it.
   Wraps after `instants` and stops after `samples` samples. Returns the
   number of instants run, or UINT64_MAX when one does not converge. */
uint64_t fast_schedule(u8 *restrict v, const uint32_t *first, const uint32_t *slots, const u8 *values, const u8 *sample, uint32_t instants, uint64_t samples, const uint32_t *probes, uint32_t n, u8 *restrict out) {{
  uint64_t run = 0, taken = 0;
  uint32_t i = 0;
  while (taken < samples) {{
{dcs_begin}    if (fast_point(v, slots + first[i], values + first[i], first[i + 1] - first[i])) return UINT64_MAX;
    run++;
    if (sample[i]) {{
      for (uint32_t k = 0; k < n; k++) out[taken * n + k] = v[probes[k]];
      taken++;
    }}
    if (++i == instants) i = 0;
  }}
  return run;
}}

/* Toggles the clock slot for `cycles` cycles (rising, then falling) and stores
   the n probe slots after each falling edge, and after each rising edge too
   when `every`. Returns the number of complete cycles. */
uint64_t fast_run(u8 *restrict v, uint32_t clock, uint64_t cycles, const uint32_t *probes, uint32_t n, uint32_t every, u8 *restrict out) {{
  uint64_t row = 0;
  for (uint64_t c = 0; c < cycles; c++) {{
    v[clock] = 1;
    if (fast_step_input(v, clock)) return c;
    if (every) {{
      for (uint32_t k = 0; k < n; k++) out[row * n + k] = v[probes[k]];
      row++;
    }}
    v[clock] = 0;
    if (fast_step_input(v, clock)) return c;
    for (uint32_t k = 0; k < n; k++) out[row * n + k] = v[probes[k]];
    row++;
  }}
  return cycles;
}}
"#
        );
        for (&d, &s) in &input_signals {
            ensure!(
                initial[slot[s]] <= 1,
                "fast mode is two-valued; input {} starts at {}",
                self.drivers[d].label,
                self.signal(s).value
            );
        }
        let mut net_of_slot = vec![usize::MAX; slots];
        for r in 0..n {
            if root[r] == r {
                net_of_slot[slot[r]] = r;
            }
        }
        if let Some(&q) = q_slots
            .iter()
            .find(|&&q| initial[q] > 1 && observed.contains(&q))
        {
            let net = net_of_slot[q];
            return Err(StartsUnknown(format!(
                "the register output R{}C{}:{}",
                self.signal(net).wire.location.row,
                self.signal(net).wire.location.col,
                self.signal(net).wire.name
            ))
            .into());
        }
        let output_slot = |driver: usize| slot[root[self.drivers[driver].signal]];
        let mut plls = Vec::new();
        for pll in &self.plls {
            ensure! {pll.controls.is_empty(), "fast mode does not run {} with RST/STDBY enabled (HC07); the event kernel does", pll.label};
            // Receivers and buffers are folded, so the reference reads the
            // clock input directly.
            let reference = match term(pll.reference, &constant)? {
                Term::Net { slot, inverted }
                    if matches!(source[net_of_slot[slot]], Some(Source::Input)) =>
                {
                    (slot, inverted)
                }
                _ => bail!(
                    "fast mode runs {} only from a clock input; its reference comes from logic",
                    pll.label
                ),
            };
            plls.push(FastPll {
                label: pll.label.clone(),
                reference,
                a: pll.a,
                b: pll.b,
                outputs: pll
                    .outputs
                    .iter()
                    .map(|&(d, divider, phase)| (output_slot(d), divider, phase))
                    .collect(),
                last_rise: pll.last_rise,
                learnt: pll.period.zip(pll.anchor),
            });
        }
        let oscillators = self
            .oscillators
            .iter()
            .map(|o| FastOscillator {
                slot: output_slot(o.driver),
                half_period: o.half_period,
            })
            .collect();
        // Source classification and the PLL scratch map are no longer read.
        // Drop them before allocating the per-signal output map, then release
        // the root/slot/constant tables before remapping the C text.
        drop(source);
        drop(net_of_slot);
        let mut slot_of: Vec<Option<(usize, bool)>> = (0..n)
            .map(|s| match constant[root[s]] {
                Some(Term::Net { slot, inverted }) => Some((slot, inverted)),
                _ => Some((slot[root[s]], false)).filter(|x| x.0 != usize::MAX),
            })
            .collect();
        drop(root);
        drop(slot);
        drop(constant);
        drop(input_signals);
        // What each thread of a split settle writes moves to a region of its
        // own past the other slots, a cache line apart from the others':
        // threads that wrote bytes of one line would take it from each other
        // at every write. A moved slot's old place is no longer used.
        let mut text = text;
        if !dtrs.is_empty() {
            // DTR batches report the completed instants and the return reason
            // separately. The stopped instant has captured, but needs the
            // host's DTR publication before it can be sampled. No-DTR source
            // and its ABI stay byte for byte unchanged.
            let begin = text.find("/* Runs a periodic schedule:").unwrap();
            text.truncate(begin);
            let _ = writeln!(text, "int fast_resume(u8 *restrict v) {{\n  settle(v);\n{check_error}  return converge(v);\n}}");
            text.push_str(r#"/* DTR ABI: (completed instants << 2) | reason: 0 done, 1 error,
   2 host publication required at the next, unsampled instant. limit counts
   instants, every completed instant is sampled. */
uint64_t fast_schedule(u8 *restrict v, const uint32_t *first, const uint32_t *slots, const u8 *values, const u8 *sample, uint32_t instants, uint64_t limit, const uint32_t *probes, uint32_t n, u8 *restrict out) {
  (void)sample;
  for (uint64_t run = 0; run < limit; run++) {
    uint32_t i = run % instants;
    int status = fast_point(v, slots + first[i], values + first[i], first[i + 1] - first[i]);
    if (status) return (run << 2) | status;
    for (uint32_t k = 0; k < n; k++) out[run * n + k] = v[probes[k]];
  }
  return limit << 2;
}

/* DTR variant: limit counts edges, each of which is sampled. */
uint64_t fast_run(u8 *restrict v, uint32_t clock, uint64_t limit, const uint32_t *probes, uint32_t n, uint32_t every, u8 *restrict out) {
  (void)every;
  for (uint64_t run = 0; run < limit; run++) {
    v[clock] ^= 1;
    int status = fast_step_input(v, clock);
    if (status) return (run << 2) | status;
    for (uint32_t k = 0; k < n; k++) out[run * n + k] = v[probes[k]];
  }
  return limit << 2;
}
"#);
            if !self.dcs.is_empty() {
                text = text.replace(
                    "    int status = fast_point(v,",
                    &format!("{dcs_begin}    int status = fast_point(v,"),
                );
            }
        }
        let base = slots;
        let mut moved: HashMap<usize, usize> = HashMap::new();
        if threaded {
            // Owner captures write registers in their thread's region, on
            // lines of their own. Keep the place experiment's layout when
            // captures still run on the caller.
            if !owner_capture && slot_place.is_some() {
                let writer: HashMap<usize, usize> = written
                    .iter()
                    .enumerate()
                    .flat_map(|(t, w)| w.iter().map(move |&s| (s, t)))
                    .collect();
                capture_owner.extend(updates.iter().filter_map(|c| {
                    let (q, read) = (&c.q, &c.reads);
                    let mut by: Vec<usize> = read
                        .iter()
                        .filter(|&s| s != q)
                        .filter_map(|s| writer.get(s).copied())
                        .collect();
                    by.sort_unstable();
                    by.dedup();
                    (by.len() == 1).then(|| (*q, by[0]))
                }));
            }
            let mut registers: Vec<(usize, usize)> = register_reads
                .iter()
                .map(|(&r, counts)| {
                    let t = capture_owner.get(&r).copied().unwrap_or_else(|| {
                        (0..threads)
                            .max_by_key(|&t| (counts[t], std::cmp::Reverse(t)))
                            .unwrap_or(0)
                    });
                    (t, r)
                })
                .collect();
            if owner_capture {
                // An owned Q with no settle readers still needs its owner's
                // cache lines rather than the original packed register slots.
                registers.extend(
                    capture_owner
                        .iter()
                        .filter(|(r, _)| !register_reads.contains_key(r))
                        .map(|(&r, &t)| (t, r)),
                );
            }
            registers.sort_unstable();
            for (t, r) in registers {
                written[t].push(r);
            }
            const LINE: usize = 128;
            // In each region what only its thread reads comes first; then,
            // on lines of their own, the nets something else reads (the
            // captures, probes, gates on demand), and the registers on lines
            // of their own again. The caller reads and writes those every
            // cycle, and a thread that then writes a line the caller has read
            // must take it back. Registers sharing lines with the nets the
            // thread writes cost the threads that read them a miss each
            // cycle: on life_full each part read 7-9 such lines, and ten
            // parts on one array without any capture took 2.5-2.8 us each
            // against 1.5 on arrays of their own.
            for region in written.iter().chain(std::iter::once(&shared_written)) {
                let (registers, rest): (Vec<usize>, Vec<usize>) =
                    region.iter().partition(|s| flip_flops.contains(s));
                let (outside, inside): (Vec<usize>, Vec<usize>) =
                    rest.into_iter().partition(|s| external.contains(s));
                for part in [inside, outside, registers] {
                    slots = slots.div_ceil(LINE) * LINE;
                    initial.resize(slots, 0);
                    for old in part {
                        if let std::collections::hash_map::Entry::Vacant(e) = moved.entry(old) {
                            e.insert(slots);
                            initial.push(0);
                            slots += 1;
                        }
                    }
                }
            }
            slots = slots.div_ceil(LINE) * LINE;
            initial.resize(slots, 0);
            for (&old, &new) in &moved {
                initial[new] = initial[old];
            }
            text = remap_slots(&text, &moved);
            for entry in slot_of.iter_mut().flatten() {
                if let Some(&new) = moved.get(&entry.0) {
                    entry.0 = new;
                }
            }
            stale = stale
                .into_iter()
                .map(|s| moved.get(&s).copied().unwrap_or(s))
                .collect();
        }
        // The flags and copies of activity_body: a block a thread, on lines of
        // its own, past every other slot; not in `place`, so a program that
        // takes over a state starts with every flag set.
        let text = if act.is_empty() {
            text
        } else {
            const LINE: usize = 128;
            let mut at = vec![0usize; act.len()];
            for t in 0..threads.max(1) {
                slots = slots.div_ceil(LINE) * LINE;
                for (k, &(thread, value)) in act.iter().enumerate() {
                    if thread == t {
                        at[k] = slots;
                        initial.resize(slots, 0);
                        initial.push(value);
                        slots += 1;
                    }
                }
            }
            slots = slots.div_ceil(LINE) * LINE;
            initial.resize(slots, 0);
            let mut out = String::with_capacity(text.len() + act.len() * 8);
            let mut rest = text.as_str();
            while let Some(k) = rest.find("v[ACT") {
                out.push_str(&rest[..k]);
                let tail = &rest[k + 5..];
                let end = tail.find(']').unwrap_or(0);
                let index: usize = tail[..end].parse().unwrap_or(0);
                let _ = write!(out, "v[{}]", at[index]);
                rest = &tail[end + 1..];
            }
            out.push_str(rest);
            out
        };
        let text = split_functions(&text, function_lines());
        let place = (0..base)
            .map(|s| moved.get(&s).copied().unwrap_or(s))
            .collect();
        for dtr in &mut dtrs {
            dtr.captured = moved.get(&dtr.captured).copied().unwrap_or(dtr.captured);
            for output in &mut dtr.outputs {
                *output = moved.get(output).copied().unwrap_or(*output);
            }
        }
        Ok(FastProgram {
            source: text,
            options: *options,
            slots,
            place,
            settle_lines,
            settle_phases,
            initial,
            scope: self.scope,
            slot_of,
            stale,
            input_of,
            z_inputs,
            gates,
            demanded_gates,
            registers: self.registers.len(),
            iologic: self.iologic.len(),
            ebrs: ebrs.into_iter().map(|e| e.0).collect(),
            pads,
            pad_conflict,
            memories: memory_slots,
            lutrams,
            clock_groups: groups.len(),
            plls,
            oscillators,
            dtrs,
            dcs_blocked: dcs.blocked,
        })
    }
}

impl FastKernel<'_> {
    /// The capture of EBR `b`: `ebr{b}(v, e0, e1, scratch)` computes the edges of
    /// ports 0 and 1 from the state before the edge, as `capture_ebr`, writes
    /// the memory and keeps the new latches and output registers for
    /// `ebr{b}_commit`, which stores them after the flip-flops. Inside, 2
    /// stands for X and merges as in the kernel; an X in an output that the
    /// logic reads (`observed`) stops the program (error 2). A port with no
    /// output in `needed` (read or probed) is not evaluated. Also returns the
    /// asynchronous resets for `asynchronous`.
    fn ebr_program(
        &self,
        b: usize,
        block: &super::ebr::Block,
        outputs: &[Vec<usize>; 2],
        readers: Readers,
        initial: &mut Vec<u8>,
        term: &dyn Fn(Input) -> Result<Term>,
    ) -> Result<(String, String)> {
        let (observed, needed) = (readers.logic, readers.any);
        let config = &block.config;
        let key = &self.device.model().resource(config.resource).key;
        // Inside an EBR, 2 is X; only a value the logic reads must be known.
        let value = |v: Logic| v.bit().map_or(2, u8::from);
        let pdp = config.mode == EbrMode::Pdp;
        let pipeline = self.state.ebr_pipeline(config.resource)?;
        // The memory, a byte per canonical 2048x9 cell.
        // New slots are pushed onto `initial`.
        let memory = initial.len();
        for row in 0..2048 {
            for &cell in self.state.ebr_word(config.resource, row)? {
                initial.push(value(cell));
            }
        }
        // Latch and output register slots of the reading sides: a registered
        // port's output nets are its registers, an unregistered port's are
        // its latches.
        let mut latch: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        let mut register: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for side in 0..2 {
            if outputs[side].is_empty() {
                continue;
            }
            if config.path_controls(side).output_register {
                register[side] = outputs[side].clone();
                for bit in 0..outputs[side].len() {
                    latch[side].push(initial.len());
                    initial.push(value(pipeline.read_latches[side][bit]));
                }
            } else {
                latch[side] = outputs[side].clone();
            }
            for (bit, &o) in outputs[side].iter().enumerate() {
                let v = value(if config.path_controls(side).output_register {
                    pipeline.output_registers[side][bit]
                } else {
                    pipeline.read_latches[side][bit]
                });
                if v == 2 && observed.contains(&o) {
                    return Err(
                        StartsUnknown(format!("output {bit} of EBR {key} port {side}")).into(),
                    );
                }
                initial[o] = v;
            }
        }
        let t = |input: Input| -> Result<String> { Ok(term(input)?.c()) };
        let shift = |width: usize| match width {
            1 => 0,
            2 => 1,
            4 => 2,
            9 => 3,
            18 => 4,
            _ => 5,
        };
        // The cell of `bit` at address variable `a`, as ebr_cell.
        let cell = |width: usize, a: &str, bit: usize| {
            if width < 9 {
                let per = 8 / width;
                format!(
                    "({a} >> {}) * 9 + ({a} & {}) * {width} + {bit}",
                    per.trailing_zeros(),
                    per - 1
                )
            } else {
                format!("({a} * {} + {}) * 9 + {}", width / 9, bit / 9, bit % 9)
            }
        };
        let mask = |side: usize, bit: usize| -> Result<Option<String>> {
            let port = &config.ports[side];
            (port.width >= 18)
                .then(|| t(port.address[bit / 9]))
                .transpose()
        };
        // Builder::build captures after wiring all routes, caching the
        // kernel's clock_source test. Folded terms could equate clocks
        // beyond plain buffers, so use that source/polarity result here.
        // Only emit the HB09 branch if its scope can be reached. In
        // particular, ROMs and disabled ports keep their existing C source.
        // An unread X control (e.g. OCEA in PDP) disqualifies HB09 without
        // adding a new requirement on the existing capture.
        let hb09_scope = std::iter::once((config.write_inhibit, 0))
            .chain(config.global_reset.map(|i| (i, 0)))
            .chain(config.ports.iter().flat_map(|p| {
                [(p.enable, 1), (p.output_enable, 1), (p.reset, 0)]
                    .into_iter()
                    .chain(
                        p.select
                            .iter()
                            .enumerate()
                            .map(|(bit, &s)| (s, (p.decode >> bit) & 1)),
                    )
                    .chain(
                        (0..p.width / 9)
                            .filter_map(|group| (p.width >= 18).then_some((p.address[group], 1))),
                    )
            }))
            .map(|(input, wanted)| term(input).map(|t| (t, wanted)))
            .collect::<Result<Vec<_>>>();
        let hb09 = !pdp
            && block.same_clock == Some(true)
            && config.ports[0].width == config.ports[1].width
            && config.ports[0].write_mode == config.ports[1].write_mode
            && hb09_scope.as_ref().is_ok_and(|scope| {
                scope
                    .iter()
                    .all(|&(t, wanted)| !matches!(t, Term::Lit(v) if v != wanted))
            })
            && config
                .ports
                .iter()
                .map(|p| term(p.write_enable))
                .collect::<Result<Vec<_>>>()?
                .iter()
                .any(|&t| t != Term::Lit(0));
        let hb09p_scope = [
            (config.ports[0].output_enable, 0),
            (config.ports[1].output_enable, 1),
        ]
        .into_iter()
        .chain(config.ports[0].address[..4].iter().map(|&i| (i, 1)))
        .map(|(i, want)| term(i).map(|t| (t, want)))
        .collect::<Result<Vec<_>>>();
        let hb09p = config.hb09p_profile()
            && block.same_clock == Some(true)
            && hb09p_scope.as_ref().is_ok_and(|scope| {
                scope
                    .iter()
                    .all(|&(t, want)| !matches!(t, Term::Lit(v) if v != want))
            });
        let mut c = String::new();
        let fail = format!("{{ return fast_fail(v, 2, {b}); }}");
        let _ = writeln!(
            c,
            "static int ebr{b}(u8 *restrict v, int e0, int e1, u8 *restrict ebr{b}_t) {{"
        );
        let _ = writeln!(c, "  u8 inh = {};", t(config.write_inhibit)?);
        let _ = writeln!(
            c,
            "  u8 glb = {};",
            config
                .global_reset
                .map(t)
                .transpose()?
                .unwrap_or("0".into())
        );
        let writes = |side: usize| if pdp { side == 0 } else { true };
        // A 36-bit PDP read is one read at port B into both paths (A, B);
        // otherwise each side reads into its own path.
        let wide = config.wide_read();
        let paths_of = |side: usize| if wide { vec![0, 1] } else { vec![side] };
        let path_needed = |path: usize| outputs[path].iter().any(|o| needed.contains(o));
        let reads = |side: usize| {
            if wide {
                side == 1 && (path_needed(0) || path_needed(1))
            } else {
                path_needed(side)
            }
        };
        for side in 0..2 {
            let port = &config.ports[side];
            let mut access = vec![t(port.enable)?];
            for (bit, &select) in port.select.iter().enumerate() {
                access.push(term(select)?.invert_if(port.decode & (1 << bit) == 0).c());
            }
            let address: Vec<String> = port.address[shift(port.width)..]
                .iter()
                .enumerate()
                .map(|(i, &a)| Ok(format!("(uint32_t){} << {i}", t(a)?)))
                .collect::<Result<_>>()?;
            let _ = writeln!(c, "  u8 acc{side} = {};", access.join(" & "));
            let _ = writeln!(c, "  uint32_t a{side} = {};", address.join(" | "));
            let _ = writeln!(c, "  u8 rst{side} = {}; (void)rst{side};", t(port.reset)?);
            let _ = writeln!(
                c,
                "  uint32_t wc{side}[36]; u8 wv{side}[36]; int nw{side} = 0;"
            );
        }
        if hb09 {
            let mut scope = vec!["e0".into(), "e1".into()];
            scope.extend(
                hb09_scope?
                    .iter()
                    .map(|(t, wanted)| format!("{} == {wanted}", t.c())),
            );
            for port in &config.ports {
                // EBR inputs are observed by fast mode, so they are binary;
                // retain the explicit known-WE/address scope at the capture.
                for input in
                    std::iter::once(&port.write_enable).chain(&port.address[shift(port.width)..])
                {
                    scope.push(format!("{} <= 1", t(*input)?));
                }
            }
            let _ = writeln!(c, "  int hb09 = {};", scope.join(" && "));
        }
        if hb09p {
            let scope = hb09p_scope?
                .iter()
                .map(|&(t, want)| format!("{} == {want}", t.c()))
                .collect::<Vec<_>>()
                .join(" && ");
            let _ = writeln!(c, "  int hb09p = e0 && e1 && acc0 && acc1 && !rst0 && !rst1 && !inh && !glb && {scope};");
        }
        // Writes, from the memory before the edge.
        for side in (0..2).filter(|&s| writes(s)) {
            let port = &config.ports[side];
            let we = if pdp {
                "1".into()
            } else {
                t(port.write_enable)?
            };
            if we == "0" {
                continue;
            }
            let _ = writeln!(c, "  if (e{side} && acc{side} && {we} && !inh) {{");
            let _ = writeln!(c, "    u8 amb = rst{side} | glb;");
            for bit in 0..port.width {
                let data = if pdp {
                    t(config.ports[bit / 18].data[bit % 18])?
                } else {
                    t(port.data[bit])?
                };
                let guard = mask(side, bit)?.map_or(String::new(), |m| format!("if ({m}) "));
                let _ = writeln!(
                    c,
                    "    {guard}{{ uint32_t c = {}; u8 d = {data}; wc{side}[nw{side}] = c; wv{side}[nw{side}] = amb ? MERGE(v[{memory} + c], d) : d; nw{side}++; }}",
                    cell(port.width, &format!("a{side}"), bit)
                );
            }
            let _ = writeln!(c, "  }}");
        }
        // Reads.
        for side in (0..2).filter(|&s| reads(s)) {
            let port = &config.ports[side];
            let other = if pdp { 0 } else { 1 - side };
            let we = if pdp {
                "0".into()
            } else {
                t(port.write_enable)?
            };
            let _ = writeln!(c, "  if (e{side}) {{");
            let _ = writeln!(c, "    u8 we = {we}; (void)we;");
            for path in paths_of(side) {
                let controls = config.path_controls(path);
                let _ = writeln!(
                    c,
                    "    u8 oce{path} = {}; u8 prst{path} = {}; (void)oce{path};",
                    t(controls.output_enable)?,
                    t(controls.reset)?
                );
            }
            for bit in 0..port.width {
                let (path, pin) = config.read_pin(side, bit);
                let registered = config.path_controls(path).output_register;
                let _ = writeln!(c, "    {{");
                let _ = writeln!(
                    c,
                    "      uint32_t c = {}; u8 mem = v[{memory} + c];",
                    cell(port.width, &format!("a{side}"), bit)
                );
                if writes(other) {
                    if hb09 {
                        let _ = writeln!(c, "      if (hb09) {{");
                        if port.write_mode != WriteMode::ReadBeforeWrite {
                            let _ = writeln!(c, "        if (!we && a0 == a1) for (int i = 0; i < nw{other}; i++) if (wc{other}[i] == c) mem = wv{other}[i];");
                        }
                        let _ = writeln!(
                            c,
                            "      }} else if (row_in(wc{other}, nw{other}, c / 9)) mem = 2;"
                        );
                    } else if hb09p {
                        let _ = writeln!(c, "      if (hb09p && a0 == a1) {{ for (int i = 0; i < nw0; i++) if (wc0[i] == c) mem = wv0[i]; }} else if (row_in(wc0, nw0, c / 9)) mem = 2;");
                    } else {
                        let _ =
                            writeln!(c, "      if (row_in(wc{other}, nw{other}, c / 9)) mem = 2;");
                    }
                }
                let _ = writeln!(c, "      u8 held = v[{}], next;", latch[path][pin]);
                let _ = writeln!(c, "      if (!acc{side}) next = held;");
                let _ = writeln!(c, "      else {{");
                if we == "0" {
                    let _ = writeln!(c, "        next = mem;");
                } else {
                    let m = mask(side, bit)?.unwrap_or("1".into());
                    let result = match port.write_mode {
                        WriteMode::Normal => "held".to_string(),
                        WriteMode::WriteThrough => {
                            let d = t(port.data[bit])?;
                            format!("{m} ? {d} : MERGE(MERGE(held, mem), {d})")
                        }
                        WriteMode::ReadBeforeWrite => format!("{m} ? mem : MERGE(held, mem)"),
                    };
                    let _ = writeln!(
                        c,
                        "        if (we) {{ u8 r = {result}; if (inh) r = MERGE(r, mem); next = r; }} else next = mem;"
                    );
                }
                if !registered {
                    let _ = writeln!(c, "        if (!oce{path}) next = MERGE(held, next);");
                }
                let _ = writeln!(c, "      }}");
                let _ = writeln!(c, "      if (prst{path} || glb) next = 0;");
                let _ = writeln!(c, "      ebr{b}_t[{}] = next;", path * 36 + pin);
                if registered {
                    let _ = writeln!(
                        c,
                        "      u8 o = v[{}]; if (oce{path}) o = acc{side} ? held : MERGE(o, held);",
                        register[path][pin]
                    );
                    let _ = writeln!(c, "      if (prst{path} || glb) o = 0;");
                    let _ = writeln!(c, "      ebr{b}_t[{}] = o;", path * 36 + 18 + pin);
                }
                let _ = writeln!(c, "    }}");
            }
            let _ = writeln!(c, "  }}");
        }
        // Writes A then B implement HB09's narrow same-word B winner and
        // preserve disjoint words (including the ninth bit). Width 18 still
        // poisons a double-written word, after capturing each port's result.
        for side in (0..2).filter(|&s| writes(s)) {
            let _ = writeln!(
                c,
                "  for (int i = 0; i < nw{side}; i++) v[{memory} + wc{side}[i]] = wv{side}[i];"
            );
        }
        if !pdp {
            let guard = if hb09 {
                if config.ports[0].width == 18 {
                    "if (!hb09 || a0 == a1) "
                } else {
                    "if (!hb09) "
                }
            } else {
                ""
            };
            let _ = writeln!(
                c,
                "  {guard}for (int i = 0; i < nw0; i++) if (row_in(wc1, nw1, wc0[i] / 9)) for (int k = 0; k < 9; k++) v[{memory} + wc0[i] / 9 * 9 + k] = 2;"
            );
        }
        // An unknown output that the logic reads stops the program.
        for side in (0..2).filter(|&s| reads(s)) {
            let checks: Vec<String> = paths_of(side)
                .into_iter()
                .flat_map(|path| {
                    let registered = config.path_controls(path).output_register;
                    outputs[path]
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| observed.contains(o))
                        .map(move |(pin, _)| {
                            format!(
                                "ebr{b}_t[{}] == 2",
                                path * 36 + pin + if registered { 18 } else { 0 }
                            )
                        })
                })
                .collect();
            if !checks.is_empty() {
                let _ = writeln!(c, "  if (e{side} && ({})) {fail}", checks.join(" || "));
            }
        }
        let _ = writeln!(c, "  return 0;\n}}");
        let _ = writeln!(
            c,
            "static void ebr{b}_commit(u8 *restrict v, int e0, int e1, const u8 *restrict ebr{b}_t) {{"
        );
        let _ = writeln!(c, "  (void)e0; (void)e1;");
        for side in (0..2).filter(|&s| reads(s)) {
            let _ = writeln!(c, "  if (e{side}) {{");
            for bit in 0..config.ports[side].width {
                let (path, pin) = config.read_pin(side, bit);
                let _ = writeln!(
                    c,
                    "    v[{}] = ebr{b}_t[{}];",
                    latch[path][pin],
                    path * 36 + pin
                );
                if config.path_controls(path).output_register {
                    let _ = writeln!(
                        c,
                        "    v[{}] = ebr{b}_t[{}];",
                        register[path][pin],
                        path * 36 + 18 + pin
                    );
                }
            }
            let _ = writeln!(c, "  }}");
        }
        let _ = writeln!(c, "}}\n");
        // Asynchronous local resets and GSR clear the latches and output
        // registers; a local one with synchronous release faults, as in
        // capture_ebr.
        let mut a = String::new();
        let clear = |a: &mut String, condition: &str, side: usize| {
            let targets = latch[side].iter().chain(&register[side]);
            for &s in targets {
                let _ = writeln!(
                    a,
                    "  if ({condition} && v[{s}]) {{ v[{s}] = 0; changed = 1; }}"
                );
            }
        };
        let read_paths: Vec<usize> = (0..2).filter(|&s| reads(s)).flat_map(paths_of).collect();
        if config.reset_kind == ResetKind::Asynchronous {
            if !config.asynchronous_release {
                for side in 0..2 {
                    let rst = t(config.ports[side].reset)?;
                    let _ = writeln!(a, "  if ({rst}) return -fast_fail(v, 3, {b});");
                }
            } else {
                for &path in &read_paths {
                    let rst = t(config.path_controls(path).reset)?;
                    clear(&mut a, &rst, path);
                }
            }
        }
        if let Some(global) = config.global_reset {
            let global = t(global)?;
            for &path in &read_paths {
                clear(&mut a, &global, path);
            }
        }
        Ok((c, a))
    }
}

/// Slots that the logic reads, and slots that the logic reads or a probe
/// watches.
struct Readers<'a> {
    logic: &'a HashSet<usize>,
    any: &'a HashSet<usize>,
}

/// A LUT over `inputs` (address bit k from input k) folded to a literal or to
/// one net, maybe inverted, when it depends on at most one net.
fn reduce(init: u16, inputs: &[Term]) -> Option<Term> {
    let mut nets: Vec<usize> = inputs
        .iter()
        .filter_map(|t| match t {
            Term::Net { slot, .. } => Some(*slot),
            Term::Lit(_) => None,
        })
        .collect();
    nets.sort_unstable();
    nets.dedup();
    let value = |assignment: usize| {
        let address: usize = inputs
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let bit = match t {
                    Term::Lit(v) => usize::from(*v),
                    Term::Net { slot, inverted } => {
                        let n = nets.iter().position(|s| s == slot).unwrap();
                        ((assignment >> n) & 1) ^ usize::from(*inverted)
                    }
                };
                bit << k
            })
            .sum();
        u8::from((init >> address) & 1 == 1)
    };
    let table: Vec<u8> = (0..1_usize << nets.len()).map(value).collect();
    let depends = |n: usize| {
        table
            .iter()
            .enumerate()
            .any(|(a, &f)| f != table[a ^ (1 << n)])
    };
    let used: Vec<usize> = (0..nets.len()).filter(|&n| depends(n)).collect();
    match used.as_slice() {
        [] => Some(Term::Lit(table[0])),
        [n] => {
            // f is the net or its inverse; read it where the net is 0.
            let zero = table.iter().enumerate().find(|(a, _)| (a >> n) & 1 == 0)?;
            Some(Term::Net {
                slot: nets[*n],
                inverted: *zero.1 == 1,
            })
        }
        _ => None,
    }
}

/// A LUT over `inputs` as the same function of only the nets it depends on,
/// each read as it is: literal inputs and inversions go into the INIT word,
/// which saves the line an operation each (the line's machine code is what
/// limits a large design, docs/FAST_MODE.md). The nets come first in the
/// returned inputs, in slot order as a folded line's leaves are, so that
/// lines reading the same nets compute the same index; then literal zeros.
fn specialize(init: u16, inputs: &[Term]) -> (u16, [Term; 4]) {
    let mut nets: Vec<usize> = inputs
        .iter()
        .filter_map(|t| match t {
            Term::Net { slot, .. } => Some(*slot),
            Term::Lit(_) => None,
        })
        .collect();
    nets.sort_unstable();
    nets.dedup();
    let mut table: Vec<bool> = (0..1_usize << nets.len())
        .map(|assignment| {
            let address: usize = inputs
                .iter()
                .enumerate()
                .map(|(k, t)| {
                    let bit = match t {
                        Term::Lit(v) => usize::from(*v),
                        Term::Net { slot, inverted } => {
                            let n = nets.iter().position(|s| s == slot).unwrap();
                            ((assignment >> n) & 1) ^ usize::from(*inverted)
                        }
                    };
                    bit << k
                })
                .sum();
            (init >> address) & 1 == 1
        })
        .collect();
    let mut n = 0;
    while n < nets.len() {
        if (0..table.len()).all(|a| table[a] == table[a ^ (1 << n)]) {
            let low = (1 << n) - 1;
            table = (0..table.len() / 2)
                .map(|a| table[(a & low) | ((a & !low) << 1)])
                .collect();
            nets.remove(n);
        } else {
            n += 1;
        }
    }
    let init = table
        .iter()
        .enumerate()
        .fold(0u16, |a, (k, &b)| a | u16::from(b) << k);
    let mut terms = [Term::Lit(0); 4];
    for (k, &slot) in nets.iter().enumerate() {
        terms[k] = Term::Net {
            slot,
            inverted: false,
        };
    }
    (init, terms)
}

/// Whether the propagate LUT can be 1 for some address that the inputs reach:
/// literal inputs fix their address bits, net inputs can take both values.
fn propagates(init: u16, inputs: &[Term]) -> bool {
    (0..16_usize).any(|address| {
        inputs.iter().enumerate().all(|(bit, t)| match t {
            Term::Lit(v) => (address >> bit) & 1 == usize::from(*v),
            Term::Net { .. } => true,
        }) && (init >> address) & 1 == 1
    })
}

/// A LUT's 16 entries as an INIT word; X or Z entries are refused.
fn lut_init(table: &[Logic; 16]) -> Result<u16> {
    let mut init = 0_u16;
    for (address, &entry) in table.iter().enumerate() {
        match entry {
            Logic::Zero => {}
            Logic::One => init |= 1 << address,
            other => bail!("fast mode needs a known LUT; entry {address} is {other}"),
        }
    }
    Ok(init)
}

/// What a fast program is built with (docs/FAST_MODE.md). `from_env` gives
/// the defaults, each replaced by its environment variable when it is set;
/// `FastSim::tune` measures others on the design and the machine it runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FastOptions {
    /// Threads a large settle is split across; 1 keeps one (`FAST_THREADS`).
    pub threads: usize,
    /// The fewest lines a settle has for it to be split (`FAST_THREAD_MIN`).
    pub thread_min: usize,
    /// The inputs a folded line may read; 0 or 1 folds nothing
    /// (`FAST_COLLAPSE`).
    pub collapse: usize,
    /// A meeting of the threads, counted in lines, when a split settle may be
    /// cut into two phases (`FAST_BARRIER_LINES`).
    pub barrier_lines: usize,
}

impl FastOptions {
    pub fn from_env() -> Self {
        Self {
            threads: fast_threads(),
            thread_min: fast_thread_min(),
            collapse: fast_collapse(),
            barrier_lines: fast_barrier_lines(),
        }
    }
    /// Which options the environment sets: `[threads, thread_min, collapse,
    /// barrier_lines]`. Tuning leaves those as they are.
    pub fn pinned() -> [bool; 4] {
        [
            "FAST_THREADS",
            "FAST_THREAD_MIN",
            "FAST_COLLAPSE",
            "FAST_BARRIER_LINES",
        ]
        .map(|v| std::env::var_os(v).is_some())
    }
}

/// Threads a large settle is split across: `FAST_THREADS`, or three (at most
/// the cores). This and the other defaults below are where a run starts and
/// what an untuned one keeps; they were measured on one machine (an M3 Max:
/// on Life, 10 000 LUTs (examples/ulx3s/life), folded into tables, 737-765
/// kHz with three threads, 707-727 with four, 101 with one; unfolded, five
/// and six threads ran slower than three or four, their meetings costing more
/// than their parts save). `FastSim::tune` measures others on the design and
/// the machine it runs on.
fn fast_threads() -> usize {
    std::env::var("FAST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(3)))
        .clamp(1, 64)
}

/// The fewest lines a settle has for it to be split: `FAST_THREAD_MIN`, or
/// 2000. Below it the threads' meeting costs more than they save.
fn fast_thread_min() -> usize {
    std::env::var("FAST_THREAD_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

/// A meeting of the threads in lines, for the cut into phases:
/// `FAST_BARRIER_LINES`, or 300.
fn fast_barrier_lines() -> usize {
    std::env::var("FAST_BARRIER_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

/// A phase of a settle split across threads: the lines several threads read
/// (run first, by the caller), then each thread's own lines, at once.
type Phase = (Vec<usize>, Vec<Vec<usize>>);
/// For `FAST_SPLIT=place`, by position in the cone: each line's place on the
/// chip (row, column of its net) and the slots it reads from outside the cone.
type Place = (Vec<(i32, i32)>, Vec<Vec<usize>>);

/// Splits the lines `cone` of a settle (indices into `lines`, in dependency
/// order) across `threads`, in one phase or two. In a phase, each line that
/// no other line of the phase reads takes its cone with it to the thread
/// whose lines overlap its cone most, while that thread has room; the
/// largest go first. A line that only one thread's lines need is that
/// thread's; one that several need is shared, and runs before them, so that
/// no thread waits for another inside a phase. One large cone (a counter's
/// adder tree, which every bit of it reads) would leave one thread with most
/// of the work, so the settle may be cut at a depth into two phases, the
/// second starting when every thread has done the first: the cut, if any,
/// that a model of the time (the shared lines, the largest part, and a
/// meeting of the threads counted as `FAST_BARRIER_LINES` lines, 300) finds
/// quickest. Lines are weighed by what they read (`line_weight`), for the
/// time and for the machine code. The caller also runs the shared lines and
/// `head` more (its captures, in the same weight): when all of it with its
/// own parts would not fit `code_budget` (`FAST_CODE_BUDGET`, unset by
/// default), its part of the first phase is cut to what does and the other
/// threads take the rest, since a thread whose code does not fit the
/// instruction cache runs it 2-6 times slower (docs/FAST_MODE.md).
fn split(
    lines: &[(String, usize, Vec<usize>)],
    cone: &[usize],
    line_of: &HashMap<usize, usize>,
    threads: usize,
    barrier: usize,
    head: usize,
    slot_place: Option<&[(i32, i32)]>,
) -> Vec<Phase> {
    let position: HashMap<usize, usize> = cone.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    // With `slot_place`: each line's place on the chip and the slots it
    // reads from outside the cone (registers, inputs).
    let place: Option<Place> = slot_place.map(|at| {
        let here = cone
            .iter()
            .map(|&i| at.get(lines[i].1).copied().unwrap_or((0, 0)))
            .collect();
        let outside = cone
            .iter()
            .map(|&i| {
                let mut r: Vec<usize> = lines[i]
                    .2
                    .iter()
                    .filter(|s| !line_of.get(s).is_some_and(|j| position.contains_key(j)))
                    .copied()
                    .collect();
                r.sort_unstable();
                r.dedup();
                r
            })
            .collect();
        (here, outside)
    });
    // What each line of the cone reads of the cone, by position, and what
    // reads it.
    let producers: Vec<Vec<usize>> = cone
        .iter()
        .map(|&i| {
            let mut p: Vec<usize> = lines[i]
                .2
                .iter()
                .filter_map(|s| line_of.get(s))
                .filter_map(|j| position.get(j).copied())
                .collect();
            p.sort_unstable();
            p.dedup();
            p
        })
        .collect();
    let mut consumers = vec![Vec::new(); cone.len()];
    for (k, p) in producers.iter().enumerate() {
        for &x in p {
            consumers[x].push(k);
        }
    }
    let mut depth = vec![0usize; cone.len()];
    for k in 0..cone.len() {
        depth[k] = producers[k]
            .iter()
            .map(|&p| depth[p] + 1)
            .max()
            .unwrap_or(0);
    }
    let weight: Vec<usize> = cone.iter().map(|&i| line_weight(&lines[i].2)).collect();
    let sum = |list: &[usize]| list.iter().map(|&k| weight[k]).sum::<usize>();
    let barrier = barrier * sum(&(0..cone.len()).collect::<Vec<_>>()) / cone.len().max(1);
    let cost = |phase: &Phase| sum(&phase.0) + phase.1.iter().map(|p| sum(p)).max().unwrap_or(0);
    let budget = code_budget();
    // The phases over `subsets`, the caller's part of the first cut to fit.
    let plan = |subsets: &[Vec<usize>]| -> Vec<Phase> {
        let mut phases: Vec<Phase> = subsets
            .iter()
            .map(|s| {
                split_phase(
                    s,
                    &producers,
                    &consumers,
                    threads,
                    &weight,
                    None,
                    place.as_ref(),
                )
            })
            .collect();
        let caller = |phases: &[Phase]| {
            head + phases
                .iter()
                .map(|p| sum(&p.0) + sum(&p.1[0]))
                .sum::<usize>()
        };
        if threads > 1 && caller(&phases) > budget {
            let others = caller(&phases) - sum(&phases[0].1[0]);
            let cap = budget.saturating_sub(others);
            phases[0] = split_phase(
                &subsets[0],
                &producers,
                &consumers,
                threads,
                &weight,
                Some(cap),
                place.as_ref(),
            );
        }
        phases
    };
    let all: Vec<usize> = (0..cone.len()).collect();
    let one = plan(std::slice::from_ref(&all));
    let mut best = (cost(&one[0]), one);
    let deepest = depth.iter().copied().max().unwrap_or(0);
    for cut in (2..deepest).step_by(2) {
        let (low, high): (Vec<usize>, Vec<usize>) = all.iter().partition(|&&k| depth[k] <= cut);
        let two = plan(&[low, high]);
        let time = cost(&two[0]) + barrier + cost(&two[1]);
        if time < best.0 {
            best = (time, two);
        }
    }
    best.1
        .into_iter()
        .map(|(shared, parts)| {
            (
                shared.iter().map(|&k| cone[k]).collect(),
                parts
                    .iter()
                    .map(|p| p.iter().map(|&k| cone[k]).collect())
                    .collect(),
            )
        })
        .collect()
}

/// The lines `list` (indices into `lines`, in dependency order) with each
/// line that reads the same slots as an earlier one moved to just after it.
/// That is safe, its inputs being computed by then, and the two lines then
/// load their inputs and build their index once between them (written in
/// one order, `specialize` and `collapse`): in a large design a fifth of the
/// lines of a thread's part have such a twin, and the part's machine code is
/// what decides its speed (docs/FAST_MODE.md).
fn beside_twins(lines: &[(String, usize, Vec<usize>)], list: &[usize]) -> Vec<usize> {
    let mut groups: Vec<Vec<usize>> = Vec::with_capacity(list.len());
    let mut group_of: HashMap<Vec<usize>, usize> = HashMap::new();
    for &i in list {
        let mut reads = lines[i].2.clone();
        reads.sort_unstable();
        reads.dedup();
        if reads.len() < 2 {
            groups.push(vec![i]);
            continue;
        }
        match group_of.entry(reads) {
            std::collections::hash_map::Entry::Occupied(g) => groups[*g.get()].push(i),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups.concat()
}

/// What a line costs, in time and in machine code: about two operations an
/// input and two more (docs/FAST_MODE.md).
fn line_weight(reads: &[usize]) -> usize {
    let mut r = reads.to_vec();
    r.sort_unstable();
    r.dedup();
    r.len() + 2
}

/// The machine code, in line weights, that the caller may run a cycle:
/// `FAST_CODE_BUDGET` kilobytes (170 suits the 192 KB instruction cache of
/// an Apple performance core), at 6.25 bytes a weight (life_full's parts).
/// Unset, no limit: on life_full the limit kept the caller's code in the
/// cache but moved more of the population counter into shared lines, and
/// the cycle took as long (docs/FAST_MODE.md).
fn code_budget() -> usize {
    std::env::var("FAST_CODE_BUDGET")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(usize::MAX, |kb| kb * 1024 * 4 / 25)
}

/// One phase of `split`, over the lines `subset` (positions in the cone, in
/// dependency order); returns positions. `cap`: at most this weight for the
/// caller's part, the others sharing the rest.
fn split_phase(
    subset: &[usize],
    producers: &[Vec<usize>],
    consumers: &[Vec<usize>],
    threads: usize,
    weight: &[usize],
    cap: Option<usize>,
    place: Option<&Place>,
) -> Phase {
    let n = producers.len();
    let mut inside = vec![false; n];
    for &k in subset {
        inside[k] = true;
    }
    // The lines no other line of the phase reads, each with its cone in
    // the phase, largest first.
    let mut stamp = vec![usize::MAX; n];
    let mut sinks: Vec<(usize, Vec<usize>)> = Vec::new();
    for &k in subset {
        if consumers[k].iter().any(|&c| inside[c]) {
            continue;
        }
        let mut cone = Vec::new();
        let mut stack = vec![k];
        while let Some(x) = stack.pop() {
            if stamp[x] == k || !inside[x] {
                continue;
            }
            stamp[x] = k;
            cone.push(x);
            stack.extend(&producers[x]);
        }
        sinks.push((k, cone));
    }
    let sum = |list: &[usize]| list.iter().map(|&k| weight[k]).sum::<usize>();
    sinks.sort_by_key(|s| std::cmp::Reverse(sum(&s.1)));
    // By place: a line in the cones of many sinks (life_full's noise for the
    // cells' turn, read across the grid) is shared whatever the split does,
    // so it neither draws a cone to a thread nor counts in its load. A phase
    // of few sinks (a population counter's last adders) stays as it was.
    let place = place.filter(|_| sinks.len() >= 4 * threads);
    let mut fan = vec![0usize; n];
    if place.is_some() {
        for (_, cone) in &sinks {
            for &x in cone {
                fan[x] += 1;
            }
        }
    }
    let local = |x: usize| place.is_none() || fan[x] <= 16;
    let total = subset
        .iter()
        .filter(|&&k| local(k))
        .map(|&k| weight[k])
        .sum::<usize>() as f64;
    let mut room = vec![total / threads as f64 * 1.05; threads];
    if let Some(cap) = cap.filter(|&c| (c as f64) < room[0] && threads > 1) {
        room = vec![(total - cap as f64) / (threads - 1) as f64 * 1.05; threads];
        room[0] = cap as f64;
    }
    let home = place.map(|place| homes(&sinks, place, threads, weight, &local));
    let mut have = vec![vec![false; n]; threads];
    let mut load = vec![0usize; threads];
    let mut owners = vec![0u64; n];
    let mut at_home = 0;
    for (s, (k, cone)) in sinks.iter().enumerate() {
        let (mut best, mut best_key) = (0, (true, usize::MAX, true, usize::MAX));
        for t in 0..threads {
            let new: usize = cone
                .iter()
                .filter(|&&x| !have[t][x] && local(x))
                .map(|&x| weight[x])
                .sum();
            let away = home.as_ref().is_some_and(|h| h[s] != t);
            let key = ((load[t] + new) as f64 > room[t], new, away, load[t]);
            if key < best_key {
                (best, best_key) = (t, key);
            }
        }
        for &x in cone {
            have[best][x] = true;
        }
        load[best] += best_key.1;
        at_home += usize::from(home.as_ref().is_some_and(|h| h[s] == best));
        owners[*k] |= 1 << best;
    }
    if home.is_some() && std::env::var_os("FAST_SPLIT_REPORT").is_some() {
        eprintln!(
            "fast mode: by place, {at_home} of {} cones on their block's thread, loads {load:?}",
            sinks.len()
        );
    }
    // Back through the phase: a line belongs to every thread that reads it.
    for &k in subset.iter().rev() {
        let mine = owners[k];
        for &p in &producers[k] {
            if inside[p] {
                owners[p] |= mine;
            }
        }
    }
    let mut shared = Vec::new();
    let mut parts = vec![Vec::new(); threads];
    for &k in subset {
        if owners[k].count_ones() == 1 {
            parts[owners[k].trailing_zeros() as usize].push(k);
        } else {
            shared.push(k);
        }
    }
    (shared, parts)
}

/// `FAST_SPLIT=place`, an experiment: the thread each sink's cone would
/// rather go to (`sinks` as in `split_phase`). The sinks are cut by their
/// place on the chip into `threads` blocks of equal weight (the placer keeps
/// gates that talk near each other), then each moves, while the blocks stay
/// within 3% of equal, to the block where most of the other sinks that read
/// its registers are: a thread then reads few registers another one writes,
/// and finds those of its own block on few cache lines.
fn homes(
    sinks: &[(usize, Vec<usize>)],
    place: &Place,
    threads: usize,
    weight: &[usize],
    local: &dyn Fn(usize) -> bool,
) -> Vec<usize> {
    let (at, outside) = place;
    let size: Vec<usize> = sinks
        .iter()
        .map(|s| s.1.iter().filter(|&&x| local(x)).map(|&x| weight[x]).sum())
        .collect();
    let reads: Vec<Vec<usize>> = sinks
        .iter()
        .map(|s| {
            // The registers its local lines read, and those lines themselves
            // (shared with neighbours' cones), as ties to other sinks.
            let mut r: Vec<usize> =
                s.1.iter()
                    .filter(|&&x| local(x))
                    .flat_map(|&x| outside[x].iter().copied().chain([usize::MAX - x]))
                    .collect();
            r.sort_unstable();
            r.dedup();
            r
        })
        .collect();
    // The sinks that read each slot; a slot most of the design reads (a
    // clock enable, a noise register) says nothing about where a sink goes.
    let mut readers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (s, r) in reads.iter().enumerate() {
        for &x in r {
            readers.entry(x).or_default().push(s);
        }
    }
    readers.retain(|_, r| r.len() <= 64);
    let mut home = vec![0; sinks.len()];
    // Blocks: halve the longer side by weight, recursively.
    let mut stack = vec![((0..sinks.len()).collect::<Vec<_>>(), 0, threads)];
    while let Some((mut list, first, count)) = stack.pop() {
        if count == 1 {
            for s in list {
                home[s] = first;
            }
            continue;
        }
        let span = |f: fn(&(i32, i32)) -> i32| {
            let v = list.iter().map(|&s| f(&at[sinks[s].0]));
            v.clone().max().unwrap_or(0) - v.min().unwrap_or(0)
        };
        let rows = span(|p| p.0) >= span(|p| p.1);
        list.sort_by_key(|&s| {
            let p = at[sinks[s].0];
            if rows {
                (p.0, p.1)
            } else {
                (p.1, p.0)
            }
        });
        let low = count / 2;
        let total: usize = list.iter().map(|&s| size[s]).sum();
        let (mut cut, mut acc) = (0, 0);
        while cut < list.len() && (acc + size[list[cut]]) * count <= total * low {
            acc += size[list[cut]];
            cut += 1;
        }
        let high = list.split_off(cut);
        stack.push((list, first, low));
        stack.push((high, first + low, count - low));
    }
    let mut load = vec![0usize; threads];
    for (s, &h) in home.iter().enumerate() {
        load[h] += size[s];
    }
    let cap = size.iter().sum::<usize>() * 103 / 100 / threads + 1;
    let mut count = vec![0usize; threads];
    for _ in 0..30 {
        let mut moved = false;
        for s in 0..sinks.len() {
            count.iter_mut().for_each(|c| *c = 0);
            for x in &reads[s] {
                for &o in readers.get(x).map_or(&[][..], Vec::as_slice) {
                    if o != s {
                        count[home[o]] += 1;
                    }
                }
            }
            let h = home[s];
            let best = (0..threads)
                .max_by_key(|&t| (count[t], t == h))
                .unwrap_or(h);
            if best != h && count[best] > count[h] && load[best] + size[s] <= cap {
                load[h] -= size[s];
                load[best] += size[s];
                home[s] = best;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    home
}

/// A pool of `THREADS - 1` workers for the settles split across threads; the
/// caller runs the first part. A worker spins a while for the next settle
/// (one comes every few microseconds while the design runs), then sleeps.
///
/// A worker claims its part as it starts it. A worker done with its own part
/// runs the parts nobody has claimed; so does the caller once it has finished
/// its own part and waited about 2.5 us (`POOL_STEAL_SPINS`, 2000 spins): on
/// a loaded machine a worker the system has not scheduled would otherwise
/// hold up the settle until it runs again, a millisecond or more. The parts of a phase are
/// independent, so whoever runs one gets the same values. A worker almost
/// always claims within 0.3 us, or after 5 us or more, so the wait sits in
/// the gap. A part already claimed is waited for: a worker preempted in the
/// middle of it still holds the settle up. `FAST_THREAD_STEAL=0` never
/// takes a part; `FAST_THREAD_STATS` prints, when the pool stops, the waits,
/// the parts taken, when workers claim, and part times by kind of core.
///
/// One simulation uses the pool at a time: another one of the same design,
/// which shares this library, runs the parts in turn. `fast_stop` stops the
/// workers before the library is unloaded.
const POOL: &str = r#"#include <pthread.h>
#include <stdatomic.h>
#ifdef __APPLE__
#include <pthread/qos.h>
#endif
#define THREADS THREADS_COUNT
/* Workers done with their own part take the parts nobody has claimed, and
   the caller does after it has waited POOL_STEAL_SPINS spins at the
   meeting; POOL_STEAL 0 never takes them. */
#ifndef POOL_STEAL
#define POOL_STEAL 1
#endif
#ifndef POOL_STEAL_SPINS
#define POOL_STEAL_SPINS 2000
#endif
#if defined(__aarch64__)
#define SPIN() __asm__ __volatile__("yield")
#elif defined(__x86_64__)
#define SPIN() __builtin_ia32_pause()
#else
#define SPIN() ((void)0)
#endif
/* Each of these takes whole cache lines of its own (the alignment is the
   type's, so nothing else is placed in the rest of the line). */
#define POOL_LINE 128
/* What the workers read on each settle: the generation, the settle, and the
   rarely written exit and sleeper count. */
static struct __attribute__((aligned(POOL_LINE))) {
  _Atomic uint32_t gen;
  _Atomic int exit, sleepers;
  u8 *v;
  void (*const *jobs)(u8 *restrict);
} pool;
/* The generation each part was last finished in, a line per part, written
   by whoever ran it and read by the caller. One counter that every worker
   adds to takes the line from core to core once a worker: life_full's
   meeting of ten threads took 0.7 us with it, 0.35 with these. */
static struct __attribute__((aligned(POOL_LINE))) {
  _Atomic uint32_t gen;
} pool_fin[THREADS];
/* The generation each part was last claimed in, a line per part: the
   worker that owns the part finds its line in its own cache. */
static struct __attribute__((aligned(POOL_LINE))) {
  _Atomic uint32_t gen;
} pool_claim[THREADS];
/* The caller's own. */
static struct __attribute__((aligned(POOL_LINE))) {
  _Atomic int busy;
  uint32_t start_gen;
  int started, failed;
  pthread_t threads[THREADS];
} pool_own;
static pthread_mutex_t pool_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t pool_cond = PTHREAD_COND_INITIALIZER;

#ifdef POOL_STATS
#include <stdio.h>
#ifdef __APPLE__
#include <mach/mach_time.h>
#include <sys/sysctl.h>
static uint64_t pool_ticks(void) { return mach_absolute_time(); }
/* The kind of core this thread runs on: 0 an efficiency core (their numbers
   come first), then the performance clusters. */
static const char *const pool_kinds[4] = {"E", "P0", "P1", "P2+"};
static pthread_once_t pool_topology_once = PTHREAD_ONCE_INIT;
static int efficiency, cluster = 1;
static void pool_topology(void) {
  int n = 0;
  size_t size = sizeof n;
  efficiency = sysctlbyname("hw.perflevel1.logicalcpu", &n, &size, 0, 0) ? 0 : n;
  size = sizeof n;
  if (!sysctlbyname("hw.perflevel0.cpusperl2", &n, &size, 0, 0) && n > 0) cluster = n;
}
static int pool_core(void) {
  pthread_once(&pool_topology_once, pool_topology);
  size_t cpu = 0;
  if (pthread_cpu_number_np(&cpu)) return 1;
  if ((int)cpu < efficiency) return 0;
  int c = 1 + ((int)cpu - efficiency) / cluster;
  return c < 3 ? c : 3;
}
#else
#include <time.h>
static uint64_t pool_ticks(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}
static const char *const pool_kinds[4] = {"", "any", "", ""};
static int pool_core(void) { return 1; }
#endif
static int pool_log2(uint64_t x) { int b = 0; while (x > 1 && b < 23) { x >>= 1; b++; } return b; }
/* Settles, parts taken by the caller, ticks waited at meetings of 1024
   ticks or more, and log2 histograms: the caller's wait, and part times by
   thread and kind of core. */
static uint64_t st_runs, st_taken, st_stall, st_wait[24];
static _Atomic uint64_t st_worker_taken;
static _Atomic uint64_t st_part[THREADS][4][24], st_late[THREADS][24], st_published;
static void st_part_time(int t, uint64_t t0) {
  uint64_t d = pool_ticks() - t0;
  atomic_fetch_add_explicit(&st_part[t][pool_core()][pool_log2(d)], 1, memory_order_relaxed);
}
static void st_print(void) {
  if (!st_runs) return;
  fprintf(stderr, "fast mode pool: %llu settles, %llu parts taken by the caller, %llu by workers, %llu ticks waiting 1024 or more\n", (unsigned long long)st_runs, (unsigned long long)st_taken, (unsigned long long)atomic_load(&st_worker_taken), (unsigned long long)st_stall);
  fprintf(stderr, "fast mode pool: caller's wait, log2 ticks:");
  for (int b = 0; b < 24; b++) if (st_wait[b]) fprintf(stderr, " %d:%llu", b, (unsigned long long)st_wait[b]);
  fprintf(stderr, "\n");
  for (int t = 1; t < THREADS; t++) {
    fprintf(stderr, "fast mode pool: thread %d claims its part, log2 ticks after the settle starts:", t);
    for (int b = 0; b < 24; b++) if (st_late[t][b]) fprintf(stderr, " %d:%llu", b, (unsigned long long)st_late[t][b]);
    fprintf(stderr, "\n");
  }
  for (int t = 0; t < THREADS; t++)
    for (int k = 0; k < 4; k++) {
      uint64_t n = 0;
      for (int b = 0; b < 24; b++) n += st_part[t][k][b];
      if (!n) continue;
      fprintf(stderr, "fast mode pool: thread %d on %s cores, part log2 ticks:", t, pool_kinds[k]);
      for (int b = 0; b < 24; b++) if (st_part[t][k][b]) fprintf(stderr, " %d:%llu", b, (unsigned long long)st_part[t][k][b]);
      fprintf(stderr, "\n");
    }
  st_runs = 0;
}
#define STAT(x) x
#else
#define STAT(x)
#endif

static void *pool_worker(void *arg) {
  int t = (int)(intptr_t)arg;
  uint32_t seen = pool_own.start_gen;
#ifdef __APPLE__
  pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
#endif
  for (;;) {
    int spins = 0;
    uint32_t gen;
    while ((gen = atomic_load_explicit(&pool.gen, memory_order_acquire)) == seen) {
      if (atomic_load_explicit(&pool.exit, memory_order_relaxed)) return 0;
      if (++spins < 200000) {
        SPIN();
        continue;
      }
      pthread_mutex_lock(&pool_mutex);
      atomic_fetch_add(&pool.sleepers, 1);
      while (atomic_load(&pool.gen) == seen && !atomic_load(&pool.exit))
        pthread_cond_wait(&pool_cond, &pool_mutex);
      atomic_fetch_sub(&pool.sleepers, 1);
      pthread_mutex_unlock(&pool_mutex);
      spins = 0;
    }
    if (atomic_load(&pool.exit)) return 0;
    seen = gen;
    /* The part is this worker's unless the caller took it while it was
       away: then the claim already holds this generation, or a later one. */
    uint32_t last = gen - 1;
    if (atomic_compare_exchange_strong_explicit(&pool_claim[t].gen, &last, gen, memory_order_relaxed, memory_order_relaxed)) {
      STAT(uint64_t t0 = pool_ticks(); atomic_fetch_add_explicit(&st_late[t][pool_log2(t0 - atomic_load_explicit(&st_published, memory_order_relaxed))], 1, memory_order_relaxed));
      pool.jobs[t](pool.v);
      STAT(st_part_time(t, t0));
      atomic_store_explicit(&pool_fin[t].gen, gen, memory_order_release);
    }
#if POOL_STEAL
    /* Then every part nobody has claimed. By now each worker that runs has
       claimed its own, so these are of workers the system has not scheduled:
       on a loaded machine, taken by whoever is free, not all by the caller
       in turn after its own (life_full on 10 threads with 8 busy loops:
       24 kHz, 34 with this). A worker claiming late finds them taken, as
       when the caller takes them. */
    for (int k = 1; k < THREADS; k++) {
      int p = (t + k) % THREADS;
      if (p == 0) continue;
      uint32_t before = gen - 1;
      if (atomic_load_explicit(&pool_claim[p].gen, memory_order_relaxed) == before &&
          atomic_compare_exchange_strong_explicit(&pool_claim[p].gen, &before, gen, memory_order_relaxed, memory_order_relaxed)) {
        pool.jobs[p](pool.v);
        STAT(atomic_fetch_add_explicit(&st_worker_taken, 1, memory_order_relaxed));
        atomic_store_explicit(&pool_fin[p].gen, gen, memory_order_release);
      }
    }
#endif
  }
}

static void pool_run(u8 *restrict v, void (*const *jobs)(u8 *restrict)) {
  int idle = 0;
  if (!atomic_compare_exchange_strong(&pool_own.busy, &idle, 1)) {
    for (int t = 0; t < THREADS; t++) jobs[t](v);
    return;
  }
  if (pool_own.failed) {
    atomic_store(&pool_own.busy, 0);
    for (int t = 0; t < THREADS; t++) jobs[t](v);
    return;
  }
  if (!pool_own.started) {
#ifdef __APPLE__
    pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
#endif
    pool_own.start_gen = atomic_load_explicit(&pool.gen, memory_order_relaxed);
    int made = 1;
    for (; made < THREADS; made++)
      if (pthread_create(&pool_own.threads[made], 0, pool_worker, (void *)(intptr_t)made)) break;
    if (made < THREADS) {
      atomic_store(&pool.exit, 1);
      for (int t = 1; t < made; t++) pthread_join(pool_own.threads[t], 0);
      atomic_store(&pool.exit, 0);
      pool_own.failed = 1;
      atomic_store(&pool_own.busy, 0);
      for (int t = 0; t < THREADS; t++) jobs[t](v);
      return;
    }
    pool_own.started = 1;
  }
  /* The workers spin on this line: each write takes it from them, so the
     settle is written only when it changed (Life: never). */
  if (pool.v != v) pool.v = v;
  if (pool.jobs != jobs) pool.jobs = jobs;
  uint32_t gen = atomic_load_explicit(&pool.gen, memory_order_relaxed) + 1;
  STAT(atomic_store_explicit(&st_published, pool_ticks(), memory_order_relaxed));
  atomic_store(&pool.gen, gen);
  STAT(uint64_t c0 = pool_ticks());
  jobs[0](v);
  STAT(st_part_time(0, c0); uint64_t w0 = pool_ticks());
  /* Sleeping workers are woken after the caller's part: read right after
     the store above, the count would wait for the store to complete. */
  if (atomic_load(&pool.sleepers)) {
    pthread_mutex_lock(&pool_mutex);
    pthread_cond_broadcast(&pool_cond);
    pthread_mutex_unlock(&pool_mutex);
  }
  /* The workers' parts: each worker claims its own as it starts it. One that
     has not claimed its part by the time the caller has waited a while (it
     is not running, or slow to wake) loses it to the caller, so the settle
     never waits on a thread that has not started. */
  int spins = 0;
  for (int p = 1; p < THREADS;) {
    if (atomic_load_explicit(&pool_fin[p].gen, memory_order_acquire) == gen) {
      p++;
      continue;
    }
#if POOL_STEAL
    if (spins >= 0 && ++spins > POOL_STEAL_SPINS) {
      spins = -1;
      for (int q = 1; q < THREADS; q++) {
        uint32_t last = gen - 1;
        if (atomic_load_explicit(&pool_claim[q].gen, memory_order_relaxed) == last &&
            atomic_compare_exchange_strong_explicit(&pool_claim[q].gen, &last, gen, memory_order_relaxed, memory_order_relaxed)) {
          jobs[q](v);
          atomic_store_explicit(&pool_fin[q].gen, gen, memory_order_release);
          STAT(st_taken++);
        }
      }
      continue;
    }
#endif
    SPIN();
  }
  (void)spins;
  STAT(uint64_t w = pool_ticks() - w0; st_runs++; st_wait[pool_log2(w)]++; if (w >= 1024) st_stall += w);
  atomic_store(&pool_own.busy, 0);
}

/* Stops the workers, after the settle running now; the next settle starts
   them again. */
void fast_stop(void) {
  int idle = 0;
  while (!atomic_compare_exchange_weak(&pool_own.busy, &idle, 1)) {
    idle = 0;
    SPIN();
  }
  STAT(st_print());
  if (pool_own.started) {
    atomic_store(&pool.exit, 1);
    pthread_mutex_lock(&pool_mutex);
    pthread_cond_broadcast(&pool_cond);
    pthread_mutex_unlock(&pool_mutex);
    for (int t = 1; t < THREADS; t++) pthread_join(pool_own.threads[t], 0);
    atomic_store(&pool.exit, 0);
    pool_own.started = 0;
  }
  atomic_store(&pool_own.busy, 0);
}
"#;

/// Whether the settle after a capture evaluates only the groups of lines
/// whose inputs changed (`activity_body`): `FAST_ACTIVITY`, 1 or 0.
fn fast_activity() -> bool {
    std::env::var("FAST_ACTIVITY").is_ok_and(|v| v.trim() == "1")
}

/// The body of one thread's part of a settle (or of a whole settle) that
/// evaluates only what changed, as ESSENT does (Beamer and Donofrio, DAC
/// 2020). Its lines go into groups: in reverse order, a line joins the group
/// of the lines that read it when they are all in one. A group has a flag;
/// it runs only when the flag is set, clears it, and sets the flags of the
/// groups that read an output of it which changed. What the part reads from
/// outside (registers, the shared lines, an earlier phase, inputs) is
/// compared at its start with a copy kept from the last time, and a change
/// sets the flags of the groups that read it: whatever changed it (a capture,
/// a reset, an input's settle, a full settle), the part sees it.
///
/// The flags and copies are `v[ACTk]`, placed later (`act` holds, for each,
/// its thread and initial value: 1, a set flag, or 2, a copy no value
/// equals), on lines of the thread's own.
fn activity_body(
    lines: &[(String, usize, Vec<usize>)],
    part: &[usize],
    thread: usize,
    act: &mut Vec<(usize, u8)>,
) -> String {
    let n = part.len();
    let local: HashMap<usize, usize> = part
        .iter()
        .enumerate()
        .map(|(k, &i)| (lines[i].1, k))
        .collect();
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, &i) in part.iter().enumerate() {
        for s in &lines[i].2 {
            if let Some(&j) = local.get(s) {
                if j < k && !readers[j].contains(&k) {
                    readers[j].push(k);
                }
            }
        }
    }
    let mut group: Vec<usize> = (0..n).collect();
    for k in (0..n).rev() {
        if let Some(&first) = readers[k].first() {
            let g = group[first];
            if readers[k].iter().all(|&r| group[r] == g) {
                group[k] = g;
            }
        }
    }
    let mut flag: HashMap<usize, usize> = HashMap::new();
    for (k, &g) in group.iter().enumerate() {
        if g == k {
            flag.insert(k, act.len());
            act.push((thread, 1));
        }
    }
    let mut text = String::new();
    // What the part reads from outside, and the groups that read each.
    let mut sources: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut source_of: HashMap<usize, usize> = HashMap::new();
    for (k, &i) in part.iter().enumerate() {
        for &s in &lines[i].2 {
            if local.get(&s).is_some_and(|&j| j < k) {
                continue;
            }
            let at = *source_of.entry(s).or_insert_with(|| {
                sources.push((s, Vec::new()));
                sources.len() - 1
            });
            let f = flag[&group[k]];
            if !sources[at].1.contains(&f) {
                sources[at].1.push(f);
            }
        }
    }
    for (s, flags) in &sources {
        let copy = act.len();
        act.push((thread, 2));
        let _ = write!(
            text,
            "  {{ unsigned i = v[{s}] ^ v[ACT{copy}]; v[ACT{copy}] = v[{s}];"
        );
        for f in flags {
            let _ = write!(text, " v[ACT{f}] |= i;");
        }
        text.push_str(" }\n");
    }
    // The groups, each where its last line was: whatever a group reads of
    // another is that one's last line, which comes before.
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, &g) in group.iter().enumerate() {
        members.entry(g).or_default().push(k);
    }
    for k in 0..n {
        if group[k] != k {
            continue;
        }
        let f = flag[&k];
        let _ = write!(text, "  if (v[ACT{f}]) {{ v[ACT{f}] = 0;");
        for &m in &members[&k] {
            let i = part[m];
            let line = lines[i].0.trim();
            let mut marks: Vec<usize> = readers[m]
                .iter()
                .map(|&r| group[r])
                .filter(|&g| g != k)
                .map(|g| flag[&g])
                .collect();
            marks.sort_unstable();
            marks.dedup();
            if marks.is_empty() {
                let _ = write!(text, " {line}");
            } else {
                let o = lines[i].1;
                let _ = write!(text, " {{ unsigned i = v[{o}]; {line} i ^= v[{o}];");
                for g in marks {
                    let _ = write!(text, " v[ACT{g}] |= i;");
                }
                text.push_str(" }");
            }
        }
        text.push_str(" }\n");
    }
    text
}

/// The most lines of a run a generated function keeps: `FAST_FUNCTION_LINES`,
/// or 500; 0 cuts none. Clang's instruction selection and scheduling grow faster
/// than the lines of a basic block, and a settle is one block: one part of
/// life_full, 14 800 lines, took 93 s at -O3 (68.6 s selecting
/// instructions) as one function and 13 s cut into functions of 500 lines.
fn function_lines() -> usize {
    std::env::var("FAST_FUNCTION_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

/// Pieces go to translation units of their own, compiled at once (unit 0
/// is the rest), when they come to more than this many lines (a cold line
/// counting as an eighth: `optnone`, see `split_functions`); about this many
/// to a unit, at most `MAX_UNITS` units. A unit of 3000 lines compiles in
/// about 3 s at -O3.
const UNIT_LINES: usize = 3000;
const MAX_UNITS: usize = 64;

/// A line a piece can take: one whole statement that writes a slot and reads
/// nothing but slots, tables and constants, so it means the same in a
/// function of its own. (A capture on several clock groups reads the edge
/// flags of `converge`, `rise0`, and stays where it is.)
fn movable(line: &str) -> bool {
    if !(line.starts_with("  v[")
        || line.starts_with("  { unsigned i")
        || line.starts_with("  if (v["))
        || !(line.ends_with(';') || line.ends_with('}'))
        || line.matches('{').count() != line.matches('}').count()
    {
        return false;
    }
    // Every name: the slots, the wide tables, a line's own index; a number
    // with its suffix (26220u, 0x96ull) is skipped whole.
    let bytes = line.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        let b = bytes[k];
        if b.is_ascii_digit() {
            while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                k += 1;
            }
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = k;
            while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                k += 1;
            }
            let name = &line[start..k];
            if !matches!(
                name,
                "v" | "unsigned" | "i" | "if" | "wide" | "wide_bits" | "mul18"
            ) {
                return false;
            }
        } else {
            k += 1;
        }
    }
    true
}

/// Cuts every run of more than `max` movable lines (anywhere: a settle, a
/// part of one, a case of `settle_input`, the captures of `converge`) into
/// functions of at most `max` lines (`pieceK`), called in turn where the
/// lines were; `v` is the slot array in every function of the program. The
/// pieces of the full settles (`settle`, `settle_registers`), which run
/// only at a start, a step, a reset or a change of program, are compiled
/// without optimisation (`COLD`, `optnone` unless `-DCOLD=`): six times
/// faster to compile; a full settle of life_full takes 1.3 ms instead of
/// about 0.5. When the pieces come to more than `UNIT_LINES` they are
/// spread over units 1.. of the source, each `#if TU == k`, and the rest is
/// unit 0; the tables, which the pieces read, are in every unit (each keeps
/// those it uses). The first lines say how many units there are
/// (`/* ecp5_emu units: N */`), for `FastSim`, which compiles them at once
/// and links them.
fn split_functions(text: &str, max: usize) -> String {
    if max == 0 {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut rest: Vec<String> = Vec::with_capacity(lines.len() / 4);
    // Each piece: its name, lines, whether it is cold, and the index in
    // `rest` of the line it was taken from.
    let mut pieces: Vec<(String, Vec<&str>, bool, usize)> = Vec::new();
    let mut function = "";
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with(' ') && line.ends_with('{') {
            if let Some(open) = line.find('(') {
                function = line[..open].rsplit(' ').next().unwrap_or("");
            }
        }
        if !movable(line) {
            rest.push(line.to_string());
            i += 1;
            continue;
        }
        let end = (i..lines.len())
            .find(|&k| !movable(lines[k]))
            .unwrap_or(lines.len());
        if end - i <= max {
            rest.extend(lines[i..end].iter().map(|l| l.to_string()));
        } else {
            let cold = function == "settle" || function == "settle_registers";
            for chunk in lines[i..end].chunks(max) {
                let name = format!("piece{}", pieces.len());
                pieces.push((name.clone(), chunk.to_vec(), cold, rest.len()));
                rest.push(format!("  {name}(v);"));
            }
        }
        i = end;
    }
    let cost = |p: &(String, Vec<&str>, bool, usize)| {
        if p.2 {
            p.1.len().div_ceil(8)
        } else {
            p.1.len()
        }
    };
    let total: usize = pieces.iter().map(cost).sum();
    let units = total.div_ceil(UNIT_LINES).clamp(1, MAX_UNITS - 1);
    let definition = |p: &(String, Vec<&str>, bool, usize), shared: bool| -> String {
        let attributes = match (shared, p.2) {
            (true, true) => "COLD __attribute__((visibility(\"hidden\"), noinline))",
            (true, false) => "__attribute__((visibility(\"hidden\"), noinline))",
            (false, true) => "static COLD __attribute__((noinline))",
            (false, false) => "static __attribute__((noinline))",
        };
        let mut f = format!("{attributes} void {}(u8 *restrict v) {{\n", p.0);
        for l in &p.1 {
            f.push_str(l);
            f.push('\n');
        }
        f.push_str("}\n");
        f
    };
    // The header of the function a line of `rest` is in.
    let header_of = |k: usize| -> usize {
        (0..=k)
            .rev()
            .find(|&h| !rest[h].starts_with(' ') && rest[h].ends_with('{') && rest[h].contains('('))
            .unwrap_or(0)
    };
    // `-DCOLD=` in FAST_CFLAGS compiles the cold pieces optimised too.
    let cold_macro = "#ifndef COLD\n#define COLD __attribute__((optnone))\n#endif\n";
    let after_typedef = |out: &mut String, l: &str| {
        out.push_str(l);
        out.push('\n');
        if l == "typedef uint8_t u8;" {
            out.push_str(cold_macro);
        }
    };
    if units == 1 {
        // One unit: the pieces go just before the function they came from.
        let mut before: HashMap<usize, String> = HashMap::new();
        for p in &pieces {
            before
                .entry(header_of(p.3))
                .or_default()
                .push_str(&definition(p, false));
        }
        let mut out = String::with_capacity(text.len() + pieces.len() * 64);
        for (k, l) in rest.iter().enumerate() {
            if let Some(defs) = before.get(&k) {
                out.push_str(defs);
            }
            after_typedef(&mut out, l);
        }
        return out;
    }
    // Spread over units 1..=units in the order of the source, each taking
    // the next pieces up to its share: the pieces of one function (a
    // thread's part) stay together and in turn in the library, where the
    // processor fetches them ahead.
    let share = total.div_ceil(units).max(1);
    let mut unit_of = vec![0usize; pieces.len()];
    let mut done = 0;
    for (k, p) in pieces.iter().enumerate() {
        unit_of[k] = (done / share).min(units - 1) + 1;
        done += cost(p);
    }
    let prelude = rest
        .iter()
        .position(|l| l == "typedef uint8_t u8;")
        .map_or(0, |k| k + 1);
    let table = |l: &String| l.starts_with("extern const ");
    let mut out = String::with_capacity(text.len() + pieces.len() * 128);
    for l in &rest[..prelude] {
        after_typedef(&mut out, l);
    }
    let _ = writeln!(
        out,
        "#ifndef TU\n#define TU 0\n#endif\n/* ecp5_emu units: {} */",
        units + 1
    );
    for l in rest.iter().filter(|l| table(l)) {
        out.push_str(l);
        out.push('\n');
    }
    for p in &pieces {
        let _ = writeln!(
            out,
            "void {}(u8 *restrict v) __attribute__((visibility(\"hidden\")));",
            p.0
        );
    }
    out.push_str("#if TU == 0\n");
    for l in rest[prelude..].iter().filter(|l| !table(l)) {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("#endif\n");
    for u in 1..=units {
        let _ = writeln!(out, "#if TU == {u}");
        for (p, _) in pieces.iter().zip(&unit_of).filter(|(_, &k)| k == u) {
            out.push_str(&definition(p, true));
        }
        out.push_str("#endif\n");
    }
    out
}

/// The C source with each slot `v[old]` of `moved` read and written at its
/// new place.
fn remap_slots(text: &str, moved: &HashMap<usize, usize>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut i) = (0, 0);
    while i + 1 < bytes.len() {
        let word = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if bytes[i] == b'v' && bytes[i + 1] == b'[' && !word {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 2 && j < bytes.len() && bytes[j] == b']' {
                let old: usize = text[i + 2..j].parse().unwrap_or(usize::MAX);
                if let Some(new) = moved.get(&old) {
                    out.push_str(&text[copied..i]);
                    let _ = write!(out, "v[{new}]");
                    copied = j + 1;
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// The inputs a collapsed line may read: `FAST_COLLAPSE`, or 12; 0 or 1 keeps
/// every line as it is.
fn fast_collapse() -> usize {
    std::env::var("FAST_COLLAPSE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MOST_FOLDED)
        .min(MOST_FOLDED)
}

/// The most inputs a folded line may read (`FAST_COLLAPSE`): a table of 2^12
/// bits, 512 bytes.
const MOST_FOLDED: usize = 12;

/// What a line computes, for collapsing: a LUT4, a mux (select, low, high),
/// a half of a carry chain, or something to keep as it is (a LUTRAM read).
#[derive(Clone)]
enum LineFn {
    Lut(u16, [Term; 4]),
    Mux(Term, Term, Term),
    Carry {
        init: u16,
        t: [Term; 4],
        c: Term,
        inject: bool,
        carry_output: bool,
    },
    Opaque,
}

impl LineFn {
    /// Its value with the slots at `value`, as the C line computes it.
    fn eval(&self, value: &dyn Fn(usize) -> bool) -> bool {
        let term = |t: Term| match t {
            Term::Lit(b) => b == 1,
            Term::Net { slot, inverted } => value(slot) ^ inverted,
        };
        let lut = |init: u16, t: &[Term]| {
            let i = t
                .iter()
                .enumerate()
                .fold(0, |a, (k, &x)| a | usize::from(term(x)) << k);
            init >> i & 1 == 1
        };
        match self {
            LineFn::Lut(init, t) => lut(*init, t),
            LineFn::Mux(s, l, h) => {
                if term(*s) {
                    term(*h)
                } else {
                    term(*l)
                }
            }
            LineFn::Carry {
                init,
                t,
                c,
                inject,
                carry_output,
            } => {
                let p = lut(*init, t);
                let g = !*inject && lut(*init, &t[..2]);
                let c = term(*c);
                match (*carry_output, *inject) {
                    (true, _) => {
                        if p {
                            c
                        } else {
                            g
                        }
                    }
                    (false, true) => p,
                    (false, false) => p ^ c,
                }
            }
            LineFn::Opaque => unreachable!("an opaque line is kept as it is"),
        }
    }
}

/// The value of each slot, as a table is filled in.
type Bits<'a> = &'a dyn Fn(usize) -> bool;

/// A line as a table: the slots it reads, `leaves[i]` the bit i of the index
/// into `table` (2^leaves bits, at least one word).
#[derive(Clone)]
struct Table {
    leaves: Vec<usize>,
    table: Vec<u64>,
}

impl Table {
    fn at(&self, value: &dyn Fn(usize) -> bool) -> bool {
        let i = self
            .leaves
            .iter()
            .enumerate()
            .fold(0, |a, (k, &s)| a | usize::from(value(s)) << k);
        self.table[i >> 6] >> (i & 63) & 1 == 1
    }
}

/// The tables of folded lines of seven or eight inputs, a byte per entry, in
/// one array of the program: a line reads its entry with one load, where a
/// bit table took six operations, and a large design's speed is decided by
/// its machine code (docs/FAST_MODE.md). One per function, shared by every
/// line that computes it.
#[derive(Default)]
struct Wide {
    at: HashMap<(Vec<u64>, usize), usize>,
    bytes: Vec<u8>,
    /// Tables of nine inputs or more, too large for a byte apiece.
    bits: Vec<u64>,
}

impl Wide {
    /// The C line computing `out_slot` from the table `t`: a constant in the
    /// line up to six inputs, a byte of `wide` for seven and eight, a bit of
    /// `wide_bits` beyond.
    fn line(&mut self, out_slot: usize, t: &Table) -> String {
        let index: Vec<String> = t
            .leaves
            .iter()
            .enumerate()
            .map(|(k, s)| {
                if k == 0 {
                    format!("v[{s}]")
                } else {
                    format!("v[{s}] << {k}")
                }
            })
            .collect();
        match t.leaves.len() {
            0 => format!("  v[{out_slot}] = {};", t.table[0] & 1),
            1..=6 => {
                let used = 1u32 << t.leaves.len();
                let mask = if used == 64 {
                    t.table[0]
                } else {
                    t.table[0] & ((1u64 << used) - 1)
                };
                format!(
                    "  v[{out_slot}] = ({mask:#x}ull >> ({})) & 1;",
                    index.join(" | ")
                )
            }
            n @ 7..=8 => {
                let at = *self.at.entry((t.table.clone(), n)).or_insert_with(|| {
                    let at = self.bytes.len();
                    self.bytes
                        .extend((0..1usize << n).map(|i| (t.table[i >> 6] >> (i & 63)) as u8 & 1));
                    at
                });
                format!("  v[{out_slot}] = wide[{at} + ({})];", index.join(" | "))
            }
            n => {
                let at = *self.at.entry((t.table.clone(), n)).or_insert_with(|| {
                    let at = self.bits.len();
                    self.bits.extend(&t.table);
                    at
                });
                format!(
                    "  {{ unsigned i = {}; v[{out_slot}] = (wide_bits[{at} + (i >> 6)] >> (i & 63)) & 1; }}",
                    index.join(" | ")
                )
            }
        }
    }

    /// The array's declaration and definition (split_functions shares the
    /// first among the units and keeps the second in unit 0); nothing when
    /// no line is that wide.
    fn c(&self) -> String {
        let mut s = String::with_capacity(self.bytes.len() * 2 + self.bits.len() * 20 + 400);
        if !self.bytes.is_empty() {
            s.push_str("extern const u8 wide[] __attribute__((visibility(\"hidden\")));\n");
            let _ = write!(
                s,
                "__attribute__((visibility(\"hidden\"), aligned(128))) const u8 wide[{}] = {{",
                self.bytes.len()
            );
            for (k, b) in self.bytes.iter().enumerate() {
                s.push_str(if k == 0 { "" } else { "," });
                s.push(if *b == 1 { '1' } else { '0' });
            }
            s.push_str("};\n");
        }
        if !self.bits.is_empty() {
            s.push_str(
                "extern const uint64_t wide_bits[] __attribute__((visibility(\"hidden\")));\n",
            );
            let _ = write!(
                s,
                "__attribute__((visibility(\"hidden\"), aligned(128))) const uint64_t wide_bits[{}] = {{",
                self.bits.len()
            );
            for (k, b) in self.bits.iter().enumerate() {
                let _ = write!(s, "{}{b:#x}ull", if k == 0 { "" } else { "," });
            }
            s.push_str("};\n");
        }
        s
    }
}

/// The lines `cone` of a settle (indices into `lines`, in dependency order)
/// collapsed into tables of at most `most` inputs. A line that one other line
/// of the settle reads, and nothing else (no capture, LUTRAM write, EBR,
/// clock or probe: `external`), is folded into the line that reads it, whose
/// table then reads the folded line's inputs instead, while it has at most
/// `most` of them; the widest folds are given up first. Only a line that
/// folded another becomes a table. A line folded away is
/// not computed by this settle, only by the full ones, so its slot is stale
/// in between: nothing but its reader reads it. Lines kept as they are (a
/// LUTRAM read) neither fold nor take folds. A table of up to six inputs is
/// a constant in its line; a wider one is a byte apiece in `wide`.
#[allow(clippy::too_many_arguments)]
fn collapse(
    lines: &[(String, usize, Vec<usize>)],
    funcs: &[LineFn],
    cone: &[usize],
    line_of: &HashMap<usize, usize>,
    readers: &[u32],
    external: &HashSet<usize>,
    most: usize,
    wide: &mut Wide,
) -> Vec<(String, usize, Vec<usize>)> {
    let inside: HashSet<usize> = cone.iter().copied().collect();
    let mut tables: HashMap<usize, Table> = HashMap::new();
    let mut kept: HashSet<usize> = cone.iter().copied().collect();
    // The lines that folded another: only those become tables. One that
    // folded nothing stays as written, which is often cheaper than a table
    // (a counter bit's `v[a] ^ v[cin]`, on a carry chain's critical path).
    let mut composed: HashSet<usize> = HashSet::new();
    let tabulate = |leaves: &[usize], value: &dyn Fn(Bits) -> bool| {
        let mut table = vec![0u64; (1usize << leaves.len()).div_ceil(64)];
        for i in 0..1usize << leaves.len() {
            let bit = |s: usize| {
                leaves
                    .iter()
                    .position(|&l| l == s)
                    .is_some_and(|k| i >> k & 1 == 1)
            };
            if value(&bit) {
                table[i >> 6] |= 1 << (i & 63);
            }
        }
        table
    };
    for &l in cone {
        if matches!(funcs[l], LineFn::Opaque) {
            continue;
        }
        let mut own: Vec<usize> = lines[l].2.clone();
        own.sort_unstable();
        own.dedup();
        // The inputs that are lines this one may fold, widest first.
        let mut folds: Vec<usize> = own
            .iter()
            .filter(|&&s| readers[s] == 1 && !external.contains(&s))
            .filter_map(|s| line_of.get(s).copied())
            .filter(|i| inside.contains(i) && tables.contains_key(i))
            .collect();
        folds.sort_by_key(|i| std::cmp::Reverse(tables[i].leaves.len()));
        let leaves_of = |folds: &[usize]| -> Vec<usize> {
            let folded: HashSet<usize> = folds.iter().map(|&i| lines[i].1).collect();
            let mut leaves: Vec<usize> = own
                .iter()
                .copied()
                .filter(|s| !folded.contains(s))
                .collect();
            for &i in folds {
                leaves.extend(&tables[&i].leaves);
            }
            leaves.sort_unstable();
            leaves.dedup();
            leaves
        };
        let mut leaves = leaves_of(&folds);
        while leaves.len() > most && !folds.is_empty() {
            folds.remove(0);
            leaves = leaves_of(&folds);
        }
        if leaves.len() > most {
            // Wider than a table even unfolded (a carry half reads five): it
            // stays as it is, and so do its inputs.
            continue;
        }
        let folded: HashMap<usize, &Table> =
            folds.iter().map(|i| (lines[*i].1, &tables[i])).collect();
        let table = tabulate(&leaves, &|bit: Bits| {
            funcs[l].eval(&|s: usize| match folded.get(&s) {
                Some(t) => t.at(bit),
                None => bit(s),
            })
        });
        for &i in &folds {
            kept.remove(&i);
        }
        if !folds.is_empty() {
            composed.insert(l);
        }
        tables.insert(l, Table { leaves, table });
    }
    let mut result = Vec::new();
    for &l in cone.iter().filter(|l| kept.contains(l)) {
        let out_slot = lines[l].1;
        let Some(t) = tables.get(&l).filter(|_| composed.contains(&l)) else {
            result.push(lines[l].clone());
            continue;
        };
        let line = wide.line(out_slot, t);
        result.push((line, out_slot, t.leaves.clone()));
    }
    result
}

/// `collapse` that also folds a line several lines read (`FAST_FOLD=cones`):
/// each line, in dependency order, takes in the inputs that are lines while
/// its table stays within `most` inputs, the ones that widen it least
/// first, and a line read by several only when that does not widen it or
/// when it has one reader. Then, from the end, a line is kept only when
/// something keeps reading it: outside this settle (`external`, or a reader
/// in another), or a kept line whose table has it among its inputs. A Life
/// cell's adder, which several lines of the cell read, becomes part of the
/// cell's one table that way, where `collapse` stops at its first shared
/// line. The tables are built over words of 64 entries.
#[allow(clippy::too_many_arguments)]
fn collapse_cones(
    lines: &[(String, usize, Vec<usize>)],
    funcs: &[LineFn],
    cone: &[usize],
    line_of: &HashMap<usize, usize>,
    readers: &[u32],
    external: &HashSet<usize>,
    most: usize,
    shared_widen: bool,
    wide: &mut Wide,
) -> Vec<(String, usize, Vec<usize>)> {
    let inside: HashSet<usize> = cone.iter().copied().collect();
    let line_in = |s: usize| line_of.get(&s).copied().filter(|i| inside.contains(i));
    let own_of = |l: usize| {
        let mut own = lines[l].2.clone();
        own.sort_unstable();
        own.dedup();
        own
    };
    // The inputs of each line's table (its frontier), for the lines that
    // can be tables; `widen(c)`: whether a line several read may be taken
    // in although it widens the table.
    let frontiers = |widen: &dyn Fn(usize) -> bool| -> HashMap<usize, Vec<usize>> {
        let mut front: HashMap<usize, Vec<usize>> = HashMap::new();
        for &l in cone {
            if matches!(funcs[l], LineFn::Opaque) {
                continue;
            }
            let mut f = own_of(l);
            if f.len() > most {
                continue;
            }
            loop {
                let mut best: Option<(bool, Vec<usize>)> = None;
                for &c in &f {
                    let Some(m) = line_in(c) else { continue };
                    let Some(fc) = front.get(&m) else { continue };
                    let mut new: Vec<usize> = f
                        .iter()
                        .copied()
                        .filter(|&s| s != c)
                        .chain(fc.iter().copied())
                        .collect();
                    new.sort_unstable();
                    new.dedup();
                    if new.len() > most {
                        continue;
                    }
                    let single = readers[c] == 1 && !external.contains(&c);
                    if !single && new.len() > f.len() && !widen(c) {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some((s, b)) => (new.len(), !single) < (b.len(), !*s),
                    };
                    if better {
                        best = Some((single, new));
                    }
                }
                match best {
                    Some((_, new)) => f = new,
                    None => break,
                }
            }
            front.insert(l, f);
        }
        front
    };
    // Which lines are kept, from the end.
    let mut in_cone_readers: HashMap<usize, u32> = HashMap::new();
    for &l in cone {
        for s in own_of(l) {
            if line_in(s).is_some() {
                *in_cone_readers.entry(s).or_default() += 1;
            }
        }
    }
    let keep = |front: &HashMap<usize, Vec<usize>>| -> HashSet<usize> {
        let mut wanted: HashSet<usize> = HashSet::new();
        let mut kept: HashSet<usize> = HashSet::new();
        for &l in cone.iter().rev() {
            let s = lines[l].1;
            let outside = readers[s] > in_cone_readers.get(&s).copied().unwrap_or(0);
            if !(external.contains(&s) || outside || wanted.contains(&s)) {
                continue;
            }
            kept.insert(l);
            wanted.extend(front.get(&l).cloned().unwrap_or_else(|| own_of(l)));
        }
        kept
    };
    // The inputs read, a measure of the work: a widened table that takes in
    // a line another kept line still needs computes it twice. So a line
    // several read is taken in widening only when the round before dropped
    // it, a few rounds, the cheapest kept.
    let cost = |front: &HashMap<usize, Vec<usize>>, kept: &HashSet<usize>| -> usize {
        kept.iter()
            .map(|l| front.get(l).map_or_else(|| own_of(*l).len(), Vec::len) + 2)
            .sum()
    };
    // Widening pays inside the logic of one sink (a Life cell's adder, all
    // of it read only on the way to that cell's register), where every line
    // it takes in is dropped; a line that leads to several sinks (a
    // counter's carry chain, read by every higher bit) widens each of them
    // and is computed again in each: on life_full the population counter
    // grew from 600 lines to 2 758, all on the caller. So only a line whose
    // readers all lead to one sink (`single`) is taken in widening.
    let mut sink: HashMap<usize, Option<usize>> = HashMap::new();
    let mut readers_in: HashMap<usize, Vec<usize>> = HashMap::new();
    for &l in cone {
        for s in own_of(l) {
            if line_in(s).is_some() {
                readers_in.entry(s).or_default().push(l);
            }
        }
    }
    for &l in cone.iter().rev() {
        let s = lines[l].1;
        let outside = readers[s] > in_cone_readers.get(&s).copied().unwrap_or(0);
        let mut one: Option<Option<usize>> = (external.contains(&s) || outside).then_some(Some(s));
        for r in readers_in.get(&s).into_iter().flatten() {
            let theirs = sink.get(r).copied().unwrap_or(None);
            one = match one {
                None => Some(theirs),
                Some(mine) if mine == theirs => Some(mine),
                Some(_) => Some(None),
            };
        }
        sink.insert(l, one.unwrap_or(None));
    }
    let single = |c: usize| line_in(c).is_some_and(|m| sink.get(&m).copied().flatten().is_some());
    let mut front = frontiers(&|c| shared_widen && single(c));
    let mut kept = keep(&front);
    if shared_widen {
        let mut best = (cost(&front, &kept), front.clone(), kept.clone());
        for _ in 0..3 {
            let dropped: HashSet<usize> = cone
                .iter()
                .filter(|l| !kept.contains(l))
                .map(|&l| lines[l].1)
                .collect();
            let f = frontiers(&|c| dropped.contains(&c) && single(c));
            let k = keep(&f);
            let c = cost(&f, &k);
            let same = k == kept;
            (front, kept) = (f, k);
            if c < best.0 {
                best = (c, front.clone(), kept.clone());
            }
            if same {
                break;
            }
        }
        if std::env::var_os("FAST_THREAD_REPORT").is_some() {
            eprintln!(
                "fast mode: folding cones, {} lines kept, cost {}",
                best.2.len(),
                best.0
            );
        }
        (front, kept) = (best.1, best.2);
    }
    // The kept lines, as tables where they took in another line.
    let mut result = Vec::new();
    for &l in cone.iter().filter(|l| kept.contains(l)) {
        let out_slot = lines[l].1;
        let own = own_of(l);
        let Some(f) = front.get(&l).filter(|f| **f != own) else {
            result.push(lines[l].clone());
            continue;
        };
        let words = (1usize << f.len()).div_ceil(64);
        let mut memo: HashMap<usize, Vec<u64>> = HashMap::new();
        for (k, &s) in f.iter().enumerate() {
            let w: Vec<u64> = (0..words)
                .map(|x| {
                    if k < 6 {
                        let pattern = [
                            0xaaaa_aaaa_aaaa_aaaau64,
                            0xcccc_cccc_cccc_cccc,
                            0xf0f0_f0f0_f0f0_f0f0,
                            0xff00_ff00_ff00_ff00,
                            0xffff_0000_ffff_0000,
                            0xffff_ffff_0000_0000,
                        ];
                        pattern[k]
                    } else if (x >> (k - 6)) & 1 == 1 {
                        u64::MAX
                    } else {
                        0
                    }
                })
                .collect();
            memo.insert(s, w);
        }
        let mut table = cone_words(lines[l].1, funcs, line_of, &own_of, &mut memo, words);
        if f.len() < 6 {
            table[0] &= (1u64 << (1 << f.len())) - 1;
        }
        let t = Table {
            leaves: f.clone(),
            table,
        };
        result.push((wide.line(out_slot, &t), out_slot, f.clone()));
    }
    result
}

/// The value of `slot` over every entry of a table (words of 64), from the
/// values in `memo` (the table's inputs, as projections): a line's own
/// function applied, word by word, to the values of its inputs.
fn cone_words(
    slot: usize,
    funcs: &[LineFn],
    line_of: &HashMap<usize, usize>,
    own_of: &dyn Fn(usize) -> Vec<usize>,
    memo: &mut HashMap<usize, Vec<u64>>,
    words: usize,
) -> Vec<u64> {
    if let Some(w) = memo.get(&slot) {
        return w.clone();
    }
    let m = line_of[&slot];
    let inputs = own_of(m);
    let values: Vec<Vec<u64>> = inputs
        .iter()
        .map(|&s| cone_words(s, funcs, line_of, own_of, memo, words))
        .collect();
    let mut out = vec![0u64; words];
    for a in 0..1usize << inputs.len() {
        let on = funcs[m].eval(&|s: usize| {
            let k = inputs
                .iter()
                .position(|&x| x == s)
                .expect("a line reads its inputs");
            (a >> k) & 1 == 1
        });
        if !on {
            continue;
        }
        for (x, o) in out.iter_mut().enumerate() {
            let mut w = u64::MAX;
            for (k, v) in values.iter().enumerate() {
                w &= if (a >> k) & 1 == 1 { v[x] } else { !v[x] };
            }
            *o |= w;
        }
    }
    memo.insert(slot, out.clone());
    out
}

/// A CLKDIVF's next stages and CDIVX (index `DIVIDER_STAGES`) as C, after a
/// rising and after a falling edge: `divider_next` of kernel/iologic.rs over
/// 0/1 bytes. `s(k)` is stage k, `q` CDIVX and `align` ALIGNWD.
fn divider_c(
    s: &dyn Fn(usize) -> String,
    q: &str,
    align: &str,
    seven: bool,
) -> (Vec<String>, Vec<String>) {
    let (run, armed) = (s(0), s(1));
    let c: Vec<String> = (2..5).map(s).collect();
    let (sampled, p1, p2, skip, parity) = (s(5), s(6), s(7), s(8), s(9));
    let count = format!("({} | {} << 1 | {} << 2)", c[0], c[1], c[2]);
    let next = format!("({count} >= 6 ? 0 : {count} + 1)");
    let trigger = format!("({align} & ({sampled} ^ 1))");
    let mut rise = Vec::new();
    rise.push(format!("({run} | {armed})"));
    rise.push(format!("(({run} | {armed}) ^ 1)"));
    for (k, ck) in c.iter().enumerate() {
        let counted = if seven {
            format!("(({next} >> {k}) & 1)")
        } else {
            ck.clone()
        };
        rise.push(format!(
            "({run} ? ({p2} ? {ck} : {counted}) : ({armed} ? 0 : {ck}))"
        ));
    }
    rise.push(align.to_string());
    rise.push(if seven {
        trigger.clone()
    } else {
        format!("({trigger} & {parity})")
    });
    rise.push(p1.clone());
    rise.push(format!("({run} & {p2})"));
    rise.push(if seven {
        parity.clone()
    } else {
        format!("({parity} ^ {trigger})")
    });
    let counted_q = if seven {
        format!("((({next} >> 2) & 1) ^ 1)")
    } else {
        format!("({q} ^ 1)")
    };
    rise.push(format!(
        "({run} ? ({p2} ? {q} : {counted_q}) : ({armed} ? 1 : {q}))"
    ));
    let mut fall: Vec<String> = (0..DIVIDER_STAGES).map(s).collect();
    fall[8] = "0".into();
    let mut fall_q = q.to_string();
    if seven {
        let step = format!("({run} & ({skip} ^ 1))");
        for k in 0..3 {
            fall[2 + k] = format!("({step} ? (({next} >> {k}) & 1) : {})", c[k]);
        }
        fall_q = format!("({step} ? ((({next} >> 2) & 1) ^ 1) : {q})");
    }
    fall.push(fall_q);
    (rise, fall)
}

/// A 7:1 gearbox path's next stage and output values as C, the same on both
/// ECLK edges: (target slot, value, value kept without an edge). The C form
/// of `gear_out7_next`/`gear_in7_next` in kernel/iologic.rs over 0/1 bytes.
/// `d` holds the data terms (GearOut7: D0..D6 and SCLK; GearIn7: the pad and
/// SCLK), `s(k)` stage k and `o(k)` output k.
fn gear7_c(
    path: &IoPath,
    s: &dyn Fn(usize) -> String,
    o: &dyn Fn(usize) -> String,
    d: &[String],
    s0: usize,
    outputs: &[usize],
) -> Vec<(usize, String, String)> {
    let mut v = Vec::new();
    match path {
        IoPath::GearOut7 { .. } => {
            let load = format!("({} & ({} ^ 1))", d[7], s(10));
            let index = format!("({} | {} << 1 | {} << 2)", s(7), s(8), s(9));
            for (k, data) in d.iter().take(7).enumerate() {
                v.push((s0 + k, format!("{load} ? {data} : {}", s(k)), s(k)));
            }
            let counted = format!("({index} >= 7 ? 7 : {index} + 1)");
            for b in 0..3 {
                let start = u8::from(b == 0);
                v.push((
                    s0 + 7 + b,
                    format!("{load} ? {start} : (({counted} >> {b}) & 1)"),
                    s(7 + b),
                ));
            }
            v.push((s0 + 10, d[7].clone(), s(10)));
            let shown = (0..7).rev().fold(o(0), |rest, k| {
                format!("({index} == {k} ? {} : {rest})", s(k))
            });
            v.push((outputs[0], format!("{load} ? {} : {shown}", d[0]), o(0)));
        }
        IoPath::GearIn7 { .. } => {
            let load = format!("({} & ({} ^ 1))", d[1], s(7));
            let sample = |j: usize| if j < 6 { s(j + 1) } else { d[0].clone() };
            for q in 0..7 {
                v.push((
                    outputs[q],
                    format!("{load} ? {} : {}", sample(GEAR_IN7_ORDER[q]), o(q)),
                    o(q),
                ));
            }
            for j in 0..7 {
                v.push((s0 + j, sample(j), s(j)));
            }
            v.push((s0 + 7, d[1].clone(), s(7)));
        }
        _ => unreachable!("gear7_c is for the 7:1 gearbox paths"),
    }
    v
}

/// Bit `k` of DELAYF's counter after one MOVE step, as C: the counter is
/// the outputs `o(0..7)`, LSB first; DIRECTION 0 counts up, 1 down,
/// saturating at 127 and 0 (`delay_step` in kernel/iologic.rs).
fn delay_c(o: &dyn Fn(usize) -> String, direction: &str, k: usize) -> String {
    let count = (0..7)
        .map(|b| format!("{} << {b}", o(b)))
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "((({direction}) ? (({count}) == 0 ? 0 : ({count}) - 1) : (({count}) == 127 ? 127 : ({count}) + 1)) >> {k}) & 1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_groups_split_q_dependencies_and_share_reset_tests() {
        let net = |slot| Term::Net {
            slot,
            inverted: false,
        };
        let make = |q, data, reads| RegisterCapture {
            q,
            data,
            reads,
            enable: net(8),
            reset: Some((net(9), net(10))),
            forced: Vec::new(),
            group: 0,
            edge: Edge::Rising,
        };
        let independent = [
            make(0, Term::Lit(1), vec![8, 9, 10]),
            make(1, Term::Lit(0), vec![8, 9, 10]),
        ];
        let (_, body, commit) = place_captures(&[0, 1], &independent, false);
        assert_eq!(body.matches("if (v[9])").count(), 1);
        assert_eq!(body.matches("if (v[8])").count(), 1);
        assert!(body.contains("v[0] = v[10]"));
        assert!(commit.is_empty());
        let dependent = [
            make(0, net(1), vec![1, 8, 9, 10]),
            make(1, Term::Lit(0), vec![8, 9, 10]),
        ];
        let (_, body, _) = place_captures(&[0, 1], &dependent, false);
        assert_eq!(body.matches("if (v[8])").count(), 2);
        assert!(body.find("v[0] = v[1]").unwrap() < body.find("v[1] = 0").unwrap());
    }

    #[test]
    #[ignore = "requires clang"]
    fn grouped_captures_match_old_wave_values() {
        let net = |slot| Term::Net {
            slot,
            inverted: false,
        };
        let make = |q: usize,
                    data: Term,
                    enable: Term,
                    reset: Option<(Term, Term)>,
                    forced: Vec<(Term, Term)>| {
            let mut reads = Vec::new();
            for t in [data, enable]
                .into_iter()
                .chain(reset.into_iter().flat_map(|(a, b)| [a, b]))
                .chain(forced.iter().flat_map(|&(a, b)| [a, b]))
            {
                if let Term::Net { slot, .. } = t {
                    reads.push(slot);
                }
            }
            RegisterCapture {
                q,
                data,
                enable,
                reset,
                forced,
                reads,
                group: q % 2,
                edge: if q.is_multiple_of(3) {
                    Edge::Falling
                } else {
                    Edge::Rising
                },
            }
        };
        let mut source = String::from("typedef unsigned char u8;\n");
        for cyclic in [false, true] {
            let captures = vec![
                make(0, Term::Lit(0), net(0), None, vec![]),
                make(1, net(2), net(0), Some((net(8), net(3))), vec![]),
                make(
                    2,
                    if cyclic { net(1) } else { Term::Lit(0) },
                    net(9),
                    None,
                    vec![],
                ),
                make(3, net(4), Term::Lit(1), None, vec![]),
                make(4, net(5), net(6), Some((net(7), net(11))), vec![]),
                make(
                    5,
                    Term::Lit(1),
                    net(0),
                    None,
                    vec![(net(10), net(2)), (net(11), Term::Lit(0))],
                ),
                make(6, Term::Lit(0), Term::Lit(1), None, vec![]),
                make(7, Term::Lit(0), Term::Lit(1), None, vec![]),
            ];
            let chosen: Vec<_> = (0..captures.len()).collect();
            for general in [false, true] {
                let name = format!("c{}_g{}", u8::from(cyclic), u8::from(general));
                let (predicates, capture, commit) = place_captures(&chosen, &captures, general);
                let _=writeln!(source,"static void grouped_{name}(u8 *v,unsigned e) {{ u8 n[8]; u8 rise0=e&1,fall0=(e>>1)&1,rise1=(e>>2)&1,fall1=(e>>3)&1; {predicates}{capture}{commit} }}");
                let mut oracle = String::new();
                for (r, c) in captures.iter().enumerate() {
                    let mut next = format!("({} ? {} : v[{}])", c.enable.c(), c.data.c(), c.q);
                    if let Some((reset, value)) = c.reset {
                        next = format!("{} ? {} : ({next})", reset.c(), value.c());
                    }
                    for (reset, value) in &c.forced {
                        next = format!("{} ? {} : ({next})", reset.c(), value.c());
                    }
                    if general {
                        next = format!(
                            "{}{} ? ({next}) : v[{}]",
                            if c.edge == Edge::Rising {
                                "rise"
                            } else {
                                "fall"
                            },
                            c.group,
                            c.q
                        );
                    }
                    let _ = writeln!(oracle, "n[{r}]={next};");
                }
                for (r, c) in captures.iter().enumerate() {
                    let _ = writeln!(oracle, "v[{}]=n[{r}];", c.q);
                }
                let _=writeln!(source,"static void oracle_{name}(u8 *v,unsigned e) {{ u8 n[8]; u8 rise0=e&1,fall0=(e>>1)&1,rise1=(e>>2)&1,fall1=(e>>3)&1; {oracle} }}");
            }
        }
        source.push_str(
            "int main(void) { for(unsigned bits=0;bits<4096;bits++) for(unsigned e=0;e<16;e++) {\n",
        );
        for cyclic in [0, 1] {
            for general in [0, 1] {
                let name = format!("c{cyclic}_g{general}");
                let _=writeln!(source,"{{ u8 a[12],b[12]; for(unsigned i=0;i<12;i++) a[i]=b[i]=(bits>>i)&1; grouped_{name}(a,e); oracle_{name}(b,e); for(unsigned i=0;i<12;i++) if(a[i]!=b[i]) return 1; }}");
            }
        }
        source.push_str("} return 0; }\n");
        let dir =
            std::env::temp_dir().join(format!("ecp5-grouped-captures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = dir.join("captures.c");
        let binary = dir.join("captures");
        std::fs::write(&c, source).unwrap();
        let built = std::process::Command::new(crate::fast::compiler())
            .args(["-O3", "-o"])
            .arg(&binary)
            .arg(c)
            .output()
            .unwrap();
        let ran = built
            .status
            .success()
            .then(|| std::process::Command::new(binary).status().unwrap());
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        assert!(
            ran.unwrap().success(),
            "capture differs from simultaneous old-state oracle"
        );
    }

    #[test]
    #[ignore = "requires clang"]
    fn grouped_resets_preserve_dependent_effects_and_changed() {
        let net = |slot| Term::Net {
            slot,
            inverted: false,
        };
        let inverted = |slot| Term::Net {
            slot,
            inverted: true,
        };
        // Repeated local/GSR writes, a reset written by another reset, a
        // self-clearing reset, and slot-valued forces (PRLD's saved M).
        let resets = [
            (net(0), 1, Term::Lit(1)),
            (net(2), 3, Term::Lit(0)),
            (net(0), 2, Term::Lit(1)),
            (net(2), 4, net(1)),
            (inverted(3), 0, Term::Lit(0)),
            (net(2), 3, Term::Lit(1)),
            (inverted(3), 5, net(4)),
            (net(5), 5, Term::Lit(0)),
            (net(5), 6, net(7)),
            (net(7), 3, Term::Lit(0)),
        ];
        let grouped = asynchronous_registers(&resets);
        let mut sequential = String::new();
        for (reset, q, value) in resets {
            let (reset, value) = (reset.c(), value.c());
            let _ = writeln!(
                sequential,
                "if ({reset} && v[{q}] != {value}) {{ v[{q}] = {value}; changed = 1; }}"
            );
        }
        let source = format!(
            "typedef unsigned char u8;\n\
             static int grouped(u8 *v) {{ int changed = 0; {grouped} return changed; }}\n\
             static int sequential(u8 *v) {{ int changed = 0; {sequential} return changed; }}\n\
             int main(void) {{ for (unsigned bits = 0; bits < 256; bits++) {{\n\
               u8 a[8], b[8]; for (unsigned i = 0; i < 8; i++) a[i] = b[i] = (bits >> i) & 1;\n\
               if (grouped(a) != sequential(b)) return 1;\n\
               for (unsigned i = 0; i < 8; i++) if (a[i] != b[i]) return 2;\n\
             }} return 0; }}\n"
        );
        let dir = std::env::temp_dir().join(format!("ecp5-grouped-resets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = dir.join("resets.c");
        let binary = dir.join("resets");
        std::fs::write(&c, source).unwrap();
        let built = std::process::Command::new(crate::fast::compiler())
            .args(["-O3", "-o"])
            .arg(&binary)
            .arg(c)
            .output()
            .unwrap();
        let ran = built
            .status
            .success()
            .then(|| std::process::Command::new(binary).status().unwrap());
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        assert!(ran.unwrap().success(), "grouped reset effects differ");
    }

    #[test]
    #[ignore = "requires the native image reader and a local Trellis database"]
    fn dtr_generation_snapshot_keeps_only_the_active_completion() {
        use crate::{
            database::Database, image::ConfigurationImage, model::DeviceModel,
            prepared::load_catalogue,
        };
        let evidence =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let db = Database::discover(None).unwrap();
        let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
        let image = Arc::new(
            ConfigurationImage::load(&evidence.join("hy04_dtr/hy04.config"), &db, None).unwrap(),
        );
        let device = Arc::new(
            crate::prepared::ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        let resource = model.resource_id("LFE5U-85F/R94C22/DTR").unwrap();
        let ports = model.resource_ports(resource);
        let mut builder = Builder::new(device).unwrap();
        let mut port = |name: &str| {
            builder
                .signal(ports.iter().find(|p| p.name == name).unwrap().wire.clone())
                .unwrap()
        };
        let start = port("STARTPULSE");
        let outputs = std::array::from_fn(|bit| port(&format!("DTROUT{bit}")));
        let driver = builder
            .external_driver(start, "start", Strength::Strong, Logic::Zero)
            .unwrap();
        builder
            .dtr(super::super::DtrConfig {
                resource,
                start: start.into(),
                outputs,
                code: 0xA1,
            })
            .unwrap();
        let mut kernel = builder.build().unwrap();
        for (time, value) in [
            (1, Logic::One),
            (2, Logic::Zero),
            (3, Logic::One),
            (4, Logic::Zero),
        ] {
            kernel.schedule(Time::from_ps(time), driver, value).unwrap();
        }
        let options = FastOptions {
            collapse: 6,
            ..FastOptions::from_env()
        };
        for (time, falling) in [(0, None), (2, Some(2)), (3, None), (4, Some(4))] {
            kernel.run_until(Time::from_ps(time)).unwrap();
            let generator = kernel.fast_generator().unwrap();
            assert!(generator.0.queue.is_empty());
            assert!(generator.0.timers.is_empty());
            let live = kernel.fast_program_with(&options).unwrap();
            let copied = generator.program(&options).unwrap();
            assert_eq!(copied.source, live.source);
            assert_eq!(copied.initial, live.initial);
            assert_eq!(copied.slot_of, live.slot_of);
            assert_eq!(copied.dtrs.len(), 1);
            let dtr = &copied.dtrs[0];
            assert_eq!(dtr.captured, live.dtrs[0].captured);
            assert_eq!(dtr.outputs, live.dtrs[0].outputs);
            assert_eq!(dtr.previous, live.dtrs[0].previous);
            assert_eq!(dtr.code, 0xA1);
            let deadline =
                falling.map(|fall| Time::from_ps(fall + super::super::DTR_CONVERSION_PS));
            assert_eq!(live.dtrs[0].completion, deadline);
            assert_eq!(dtr.completion, deadline);
            if time == 4 {
                kernel.run_until(deadline.unwrap()).unwrap();
                assert_eq!(
                    kernel.fast_program_with(&options).unwrap().dtrs[0].completion,
                    None
                );
                assert_eq!(
                    generator.program(&options).unwrap().dtrs[0].completion,
                    deadline,
                    "later event execution cannot alter the generation snapshot"
                );
            }
        }
    }
}
