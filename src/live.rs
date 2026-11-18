//! A design running live on the ULX3S around it, for an interactive view:
//! a thread loads the design, compiles it for the board (src/board.rs),
//! starts fast mode after the event kernel's warm start, and runs it on
//! commands (run, pause, step, buttons, UART keys, real-time pacing, going
//! back: from snapshots and a log of the inputs, src/fast.rs). What a
//! view shows is published in `Shared` after every chunk of edges: the value
//! of every wire and site of the design's geometry (src/layout.rs) and of
//! every wire it uses in the die's fabric (src/fabric.rs,
//! src/fabric_overlay.rs), the LEDs, the UART text, and what the board's
//! peripherals show (src/peripherals: an HDMI monitor, the OLED), decoded
//! from their pads after every instant (`FastSim::trace`).
use crate::{
    board,
    compile::{self, Board, Reference, Request, Scope, Startup},
    database::Database,
    fabric::{self, Fabric},
    fabric_overlay::{self, Overlay},
    fast::{self, FastSim},
    image::ConfigurationImage,
    kernel::{self, DriverId, SignalId, StartsUnknown, Time},
    layout::{self, Geometry},
    logic::Logic,
    model::{topology::Wire, DeviceModel},
    names::Names,
    peripherals::{self, Attached},
    prepared::{load_catalogue, ConfiguredDevice},
    sources, timing_path, uart,
};
use anyhow::{Context as _, Result};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const HALF_NS: u64 = board::CLOCK_PERIOD_NS / 2;
/// Clock edges between two looks at the commands, at most: at full speed
/// fewer, as many as run in about `PUBLISH`, so that a big design (80k
/// LUTs runs some 70k edges a second) still gives the window new values
/// as often, not a burst every half second that the map shows blinking.
const CHUNK: u64 = 1 << 14;
/// How often the window gets new values while running.
const PUBLISH: Duration = Duration::from_millis(30);
/// How often at full speed a sample comes with the wires that changed in
/// the cycle before it: often enough that a wire changing every cycle glows
/// on (the window's glow lasts longer), seldom enough to cost the run little.
const WATCH: Duration = Duration::from_millis(100);
/// How many of the last edges of a step are run one by one, to tell the
/// window every wire that changed on them, not only those whose value
/// differs at the end (a clock, after a cycle).
const STEP_EDGES: u64 = 64;
/// The board's clock, in cycles per second.
pub const REAL_TIME: f64 = 1e9 / board::CLOCK_PERIOD_NS as f64;

#[derive(Debug)]
pub enum Command {
    Run,
    Pause,
    /// Runs this many cycles, then pauses.
    Step(u64),
    /// Goes back this many cycles, as far as the start of fast mode, and
    /// pauses: the run is as it was there, and stepping on takes the same
    /// inputs again until a new one is given.
    Back(u64),
    /// Goes to this cycle and pauses: back to it, or on to it.
    GoTo(u64),
    /// A board button (src/board.rs order) pressed or released.
    Button(usize, bool),
    /// A DIP switch (src/board.rs order): ON reads high, OFF reads its pull.
    Switch(usize, bool),
    /// An external voltage on an ADC channel, recorded in the way back.
    AdcVolts(usize, f64),
    /// External level in board::HEADER order; None releases the pad.
    HeaderLevel(usize, Option<bool>),
    /// Bytes for the UART receiver.
    Keys(Vec<u8>),
    /// Cycles per second while running; none runs as fast as it can. The
    /// board's 25 MHz is real time; a few per second show each cycle.
    Rate(Option<f64>),
    /// Watch these wires of `Static::overlay` (indices of `Shared::lit`):
    /// their values after every clock edge come in `Shared::waves`, marked
    /// `id`, the window keeping about `keep` edges of them. None: watch
    /// nothing (the run reads only what it reads unwatched).
    Watch {
        id: u64,
        wires: Vec<u32>,
        keep: u64,
    },
}

/// What kind of tile a die position shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Logic,
    Memory,
    Dsp,
    Io,
    Clock,
    Routing,
    Filler,
}

/// One position of the die grid, with every tile at it.
pub struct Cell {
    pub row: u16,
    pub col: u16,
    pub class: Class,
    pub tiles: Vec<String>,
    /// Settings the design uses here, and routing transfers.
    pub fields: usize,
    pub routes: usize,
    /// A block other than logic that the plan executes here (EBR, PIO, …).
    pub blocks: Vec<String>,
    /// HDL names whose driving wire is in this position.
    pub names: Vec<String>,
}

/// A setting the plan makes: its name and the values it can be (one, when
/// the bitstream decides it).
pub type Setting = (String, Vec<String>);

/// What does not change while the design runs.
pub struct Static {
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<Cell>,
    /// Wires, pips and sites; their values are in `Shared::values`.
    pub geometry: Geometry,
    /// The die's whole fabric, and what the design uses in it; the values
    /// of `overlay.values` are in `Shared::lit`.
    pub fabric: Arc<Fabric>,
    pub overlay: Overlay,
    /// The HDL names of each net of the overlay.
    pub net_names: HashMap<u32, Vec<String>>,
    /// Built on the runner after ready. OnceLock publishes immutable data
    /// without a frame taking Shared's lock or waiting for the build.
    pub sources: OnceLock<Arc<sources::Index>>,
    /// After-ready build seconds, alongside the index for record notes.
    pub source_seconds: OnceLock<f64>,
    /// Parsed and mapped after ready, following the source index.
    pub timing: OnceLock<timing_path::Published>,
    /// The device model, for the wires of a site's pins, and the device as
    /// configured, for its words (a LUT's truth table) and its sites'
    /// decoded settings.
    pub model: Arc<DeviceModel>,
    pub device: Arc<ConfiguredDevice>,
    /// What the plan sets in the tiles of each position, as (setting, its
    /// values), and the routing transfers it makes there, as (source,
    /// sink) as the tiles name them.
    pub settings: HashMap<(u16, u16), Vec<Setting>>,
    pub transfers: HashMap<(u16, u16), Vec<(String, String)>>,
    pub leds: [bool; 8],
    pub buttons: [bool; 7],
    /// Which DIP switch inputs the design reads.
    pub switches: [bool; 4],
    /// Header indices and configured directions, in connector order.
    pub header: Vec<(usize, board::Direction)>,
    pub uart_tx: bool,
    pub uart_rx: bool,
    pub summary: String,
    /// Routing transfers in the plan.
    pub routes: usize,
}

#[derive(Default)]
pub struct Shared {
    pub stage: String,
    pub error: Option<String>,
    /// Published before source indexing starts. Static::sources arrives
    /// separately with a repaint, without changing what ready means.
    pub ready: Option<Arc<Static>>,
    pub running: bool,
    pub rate: Option<f64>,
    pub cycles: u64,
    /// Clock edges run: the cycle and which half of it (`cycles` is half).
    pub edges: u64,
    pub mhz: f64,
    /// The running program's settings (threads, folding).
    pub fast: Option<FastChoice>,
    pub leds: [Option<bool>; 8],
    /// DIP positions at this instant, including after going back.
    pub switches: [bool; 4],
    /// External header positions in board::HEADER order; None is released.
    pub header_levels: Vec<Option<bool>>,
    /// What arrived on the UART.
    pub text: Vec<u8>,
    /// The value of each wire of `Static::geometry`: 0, 1, or 2 when the
    /// design does not drive it or it is unknown.
    pub values: Vec<u8>,
    /// The same for each wire of `Static::overlay`.
    pub lit: Vec<u8>,
    /// How many times the run has published: the window draws more often
    /// than that, and takes `values` and `lit` only when this has moved.
    pub published: u64,
    /// The first cycle one can go back to, and how many cycles of the run
    /// lie recorded ahead after going back.
    pub earliest: u64,
    pub ahead: u64,
    /// After going back, the whole UART text up to there, in place of what
    /// arrived before.
    pub console: Option<Vec<u8>>,
    /// The window sees `lit` only at its frames, and a wire that changes
    /// on every edge (a clock) may have the same value at each: here, for
    /// each wire of `lit`, 1 if it changed since the window last took this.
    /// At a set rate (up to 1000 cycles a second) every edge is read; at
    /// full speed `lit` is a sample, and what counts is the last cycle
    /// before a sample, watched now and then. Empty when nothing changed or
    /// not running.
    pub toggled: Vec<u8>,
    /// What the peripherals the design drives show (src/peripherals).
    pub peripherals: peripherals::View,
    /// The watched wires' values after every edge (`Command::Watch`), as
    /// the run went (going back runs some edges again) since the window
    /// last took them.
    pub waves: Vec<WaveRun>,
    /// What the background watch-program builder is doing or running, when
    /// watched nets needed a program that keeps nets the default folds.
    pub watch_status: Option<String>,
}

/// The values of the watched wires after a run of clock edges: a row of
/// `width` bytes per edge (0, 1, 2 for X, 3 for Z, `fast::NOT_SAMPLED`
/// where the running program does not keep the wire's net up to date, or
/// it has no signal), the first after edge `first`, for watch list `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveRun {
    pub id: u64,
    pub first: u64,
    pub width: usize,
    pub rows: Vec<u8>,
}

impl WaveRun {
    /// The edges it has a row for.
    pub fn edges(&self) -> u64 {
        (self.rows.len() / self.width.max(1)) as u64
    }
}

/// `run` added to the runs the window has not taken: joined to the last
/// when it follows on, and the oldest edges let go past twice `keep`.
fn deliver(waves: &mut Vec<WaveRun>, run: WaveRun, keep: u64) {
    match waves.last_mut() {
        Some(last)
            if last.id == run.id
                && last.width == run.width
                && last.first + last.edges() == run.first =>
        {
            last.rows.extend_from_slice(&run.rows);
        }
        _ => waves.push(run),
    }
    let most = 2 * keep.max(1);
    let mut total: u64 = waves.iter().map(WaveRun::edges).sum();
    while total > most {
        let first = &mut waves[0];
        let drop = (total - most).min(first.edges());
        first.rows.drain(..drop as usize * first.width);
        first.first += drop;
        total -= drop;
        if first.rows.is_empty() {
            waves.remove(0);
        }
    }
}

/// The settings fast mode runs with: the threads a settle is split across
/// and the folding width (src/fast.rs `choose_options` picks them).
#[derive(Clone, Debug)]
pub struct FastChoice {
    pub threads: usize,
    pub folding: usize,
}

/// Commands received while loading wait here until the run can apply them.
/// Checking the channel also lets a loader stop when its window lets go.
struct Commands<'a> {
    receiver: &'a mpsc::Receiver<Command>,
    pending: std::cell::RefCell<VecDeque<Command>>,
}

#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("design closed")
    }
}
impl std::error::Error for Cancelled {}

impl Commands<'_> {
    fn check(&self) -> Result<()> {
        loop {
            match self.receiver.try_recv() {
                Ok(c) => self.pending.borrow_mut().push_back(c),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(Cancelled.into()),
            }
        }
    }

    fn next(&self, block: bool) -> Result<Option<Command>> {
        self.next_wait(block.then_some(Duration::from_millis(200)))
    }

    fn next_wait(&self, wait: Option<Duration>) -> Result<Option<Command>> {
        self.check()?;
        if let Some(c) = self.pending.borrow_mut().pop_front() {
            return Ok(Some(c));
        }
        let Some(timeout) = wait else {
            return Ok(None);
        };
        match self.receiver.recv_timeout(timeout) {
            Ok(c) => Ok(Some(c)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Cancelled.into()),
        }
    }
}

/// The model and fabric belong to the chip, so closing a design does not
/// discard them. Their worker can finish while another design is opening.
#[derive(Default)]
struct Chip {
    model: Option<Arc<DeviceModel>>,
    fabric: Option<Arc<Fabric>>,
    error: Option<String>,
}

type ChipCache = (Mutex<Chip>, Condvar);

/// Starts loading the device model and drawing the die's fabric, if not
/// already: a window calls it as it opens, so that the first design opened
/// (from the start screen, or once a project is built) finds them ready.
pub fn prepare_chip() {
    chip_cache();
}

/// The die's fabric once drawn, without waiting for the chip's worker.
pub fn chip_fabric() -> Option<Arc<Fabric>> {
    chip_cache().0.try_lock().ok()?.fabric.clone()
}

fn chip_cache() -> &'static Arc<ChipCache> {
    static CHIP: OnceLock<Arc<ChipCache>> = OnceLock::new();
    CHIP.get_or_init(|| {
        let cache = Arc::new((Mutex::new(Chip::default()), Condvar::new()));
        let worker = Arc::clone(&cache);
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let evidence = architecture();
                    let model = Arc::new(DeviceModel::load(&Database::discover(None)?, &evidence)?);
                    worker.0.lock().unwrap().model = Some(Arc::clone(&model));
                    worker.1.notify_all();
                    let fabric = Arc::new(fabric::cached(&model, &architecture()));
                    worker.0.lock().unwrap().fabric = Some(fabric);
                    Ok(())
                }));
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("{e:#}")),
                Err(_) => Some("loading the chip's model or fabric failed".into()),
            };
            if let Some(error) = error {
                worker.0.lock().unwrap().error = Some(error);
            }
            worker.1.notify_all();
        });
        cache
    })
}

fn cached<T>(commands: &Commands<'_>, get: impl Fn(&Chip) -> Option<T>) -> Result<T> {
    let cache = chip_cache();
    let mut chip = cache.0.lock().unwrap();
    loop {
        commands.check()?;
        if let Some(error) = &chip.error {
            anyhow::bail!("{error}");
        }
        if let Some(value) = get(&chip) {
            return Ok(value);
        }
        chip = cache
            .1
            .wait_timeout(chip, Duration::from_millis(50))
            .unwrap()
            .0;
    }
}

pub struct Design {
    pub config: PathBuf,
    pub names: Option<PathBuf>,
    /// Run as soon as the design is loaded.
    pub run: bool,
}

/// A snapshot of the run to go back to.
struct Checkpoint {
    /// Its number: they are taken in order.
    seq: u64,
    /// Clock edges run, and the time in ns: past them after an input
    /// between two edges.
    done: u64,
    now_ns: u64,
    fast: fast::Snapshot,
    receiver: uart::Receiver,
    /// How much UART text had arrived.
    text: usize,
    /// The peripherals' state (the OLED's RAM).
    peripherals: peripherals::Snapshot,
    switches: [bool; 4],
}

/// An input applied: when (ns), and new driver levels or an ADC voltage.
type Input = (u64, InputChange);
/// Queued time, pin, resolved level, and physical position for a DIP flip.
type HeaderPosition = Option<(usize, Option<bool>)>;
type QueuedPin = (
    u64,
    DriverId,
    Option<bool>,
    Option<(usize, bool)>,
    HeaderPosition,
);

#[derive(Clone)]
enum InputChange {
    /// Plain pin levels, with physical switch position when one was flipped.
    Pins(
        Vec<(DriverId, Option<bool>)>,
        Option<(usize, bool)>,
        HeaderPosition,
    ),
    AdcVolts(usize, f64),
}

/// Apply a recorded input, including the ADC's external analog source.
fn apply_input(
    fast: &mut FastSim,
    attached: &mut Attached,
    ns: u64,
    input: &InputChange,
) -> Result<()> {
    match input {
        InputChange::Pins(set, _, header) => {
            fast.set_at_levels(Time::from_ps(ns * 1000), set)?;
            if let Some((index, level)) = header {
                attached.set_header_level(*index, *level);
            }
            Ok(())
        }
        InputChange::AdcVolts(channel, volts) => {
            let at = Time::from_ps(ns * 1000);
            if fast.now()? < at {
                fast.set_at(at, &[])?;
            }
            attached.set_volts(*channel, *volts);
            Ok(())
        }
    }
}

/// The way back: snapshots of the run, every input it took and the UART
/// text it gave. Going to a point restores the last snapshot before it and
/// runs on to it with the same inputs. The inputs recorded after a point
/// one went back to stay, and stepping on takes them again, until a new
/// input starts another run from there.
struct History {
    checkpoints: Vec<Checkpoint>,
    /// Physical positions restored with checkpoints and updated by logged inputs.
    switches: [bool; 4],
    log: Vec<Input>,
    /// The next entry of `log`: before its end after going back.
    cursor: usize,
    /// The UART text of the recorded run, and how much of it has come by
    /// now.
    text: Vec<u8>,
    shown: usize,
    /// The furthest edge this run has reached.
    furthest: u64,
    taken: Instant,
    /// Snapshots taken so far.
    seq: u64,
}

/// How often a running design is snapshot. Going to a point runs from the
/// snapshot before it: near the present that is at most this long of the
/// run; further back the snapshots thin out (`kept`).
const SNAPSHOT_EVERY: Duration = Duration::from_millis(25);

/// Whether snapshot `s` is kept once `n` have been taken: the last 32, and
/// of older ones those whose number is a multiple of a sixteenth of their
/// age, rounded down to a power of two. One dropped is never needed again
/// (the multiple only grows with age), and going to a point of age a runs at
/// most about a/16 of the run, from about 16 snapshots for each doubling of
/// age (about 300 after an hour).
fn kept(n: u64, s: u64) -> bool {
    let age = n - s;
    s == 0 || age < 32 || s.is_multiple_of(1 << (age.ilog2() - 4))
}

/// Edges a run is rebuilt in at a time: whole cycles, from a rising edge,
/// run fastest (src/fast.rs), and the window hears how far it has come.
const REBUILD_CHUNK: u64 = 1 << 22;
/// The same while peripherals are traced: some 5 MB of trace for an HDMI
/// design (five instants an edge).
const TRACED_CHUNK: u64 = 1 << 16;

impl History {
    fn new(
        done: u64,
        now_ns: u64,
        fast: &FastSim,
        receiver: &uart::Receiver,
        attached: &Attached,
        text: &[u8],
        switches: [bool; 4],
    ) -> Self {
        let mut h = Self {
            checkpoints: Vec::new(),
            switches,
            log: Vec::new(),
            cursor: 0,
            text: text.to_vec(),
            shown: text.len(),
            furthest: done,
            taken: Instant::now(),
            seq: 0,
        };
        h.keep(done, now_ns, fast, receiver, attached, true);
        h
    }

    /// Snapshots the run here, unless one is here already or, without
    /// `now`, the last was taken a moment ago.
    fn keep(
        &mut self,
        done: u64,
        now_ns: u64,
        fast: &FastSim,
        receiver: &uart::Receiver,
        attached: &Attached,
        now: bool,
    ) {
        if !now && self.taken.elapsed() < SNAPSHOT_EVERY {
            return;
        }
        let at = self
            .checkpoints
            .partition_point(|c| (c.done, c.now_ns) < (done, now_ns));
        if self
            .checkpoints
            .get(at)
            .is_some_and(|c| (c.done, c.now_ns) == (done, now_ns))
        {
            return;
        }
        self.checkpoints.insert(
            at,
            Checkpoint {
                seq: self.seq,
                done,
                now_ns,
                fast: fast.snapshot(),
                receiver: receiver.clone(),
                text: self.shown,
                peripherals: attached.snapshot(),
                switches: self.switches,
            },
        );
        self.taken = Instant::now();
        let n = self.seq;
        self.checkpoints.retain(|c| kept(n, c.seq));
        self.seq += 1;
    }

    /// Apply position metadata only after the corresponding pin input was applied.
    fn switched(&mut self, input: &InputChange) {
        if let InputChange::Pins(_, Some((index, on)), _) = input {
            self.switches[*index] = *on;
        }
    }

    /// A byte from the UART: the next of the recorded run's text, or a new
    /// one past it.
    fn received(&mut self, b: u8) {
        match self.text.get_mut(self.shown) {
            Some(old) => *old = b,
            None => self.text.push(b),
        }
        self.shown += 1;
    }

    /// The UART text up to now.
    fn console(&self) -> Vec<u8> {
        self.text[..self.shown].to_vec()
    }

    /// A new input at this point: what was recorded after it is gone.
    fn branch(&mut self, done: u64, now_ns: u64) {
        self.log.truncate(self.cursor);
        self.text.truncate(self.shown);
        self.checkpoints
            .retain(|c| (c.done, c.now_ns) <= (done, now_ns));
        self.furthest = done;
    }
}

/// The levels the peripherals now drive into the design (the SD card's
/// MISO and the RTC's open-drain SDA), given 1 ps after the instant the run
/// reached, before the next edge, and what the pads did at that instant fed back. They are not in the
/// way back's log: going back restores the peripherals and they answer the
/// same again.
fn answer(fast: &mut FastSim, attached: &mut Attached, drivers: &[DriverId]) -> Result<()> {
    let set: Vec<(DriverId, Option<bool>)> = attached
        .respond()
        .into_iter()
        .map(|(i, level)| (drivers[i], level))
        .collect();
    if !set.is_empty() {
        let at = fast.now()?.checked_add(Time::from_ps(1))?;
        fast.set_at_levels(at, &set)?;
        attached.feed(&fast.take_trace());
    }
    Ok(())
}

/// The signals a run reads after every clock edge, a column each of its
/// rows: the UART's TX first when the design has one (the receiver decodes
/// it), then any others.
struct Sampled {
    signals: Vec<SignalId>,
    uart: bool,
    watching: Option<Watching>,
}

/// The wires the window watches: the column of each in the rows (none for
/// one without a signal), and how many edges of them the window keeps.
struct Watching {
    id: u64,
    columns: Vec<Option<usize>>,
    keep: u64,
}

impl Sampled {
    /// Watches `wires` (their signals: `signal_of`) under list `id`, in
    /// place of what was watched; none watches nothing.
    fn watch(&mut self, id: u64, wires: &[u32], keep: u64, signal_of: &[Option<SignalId>]) {
        self.signals.truncate(usize::from(self.uart));
        self.watching = (!wires.is_empty()).then(|| Watching {
            id,
            columns: wires
                .iter()
                .map(|&w| {
                    signal_of.get(w as usize).copied().flatten().map(|signal| {
                        self.signals.push(signal);
                        self.signals.len() - 1
                    })
                })
                .collect(),
            keep,
        });
    }

    /// Runs `edges` clock edges with a row of the signals after each; a
    /// watched one the program does not keep up to date is
    /// `fast::NOT_SAMPLED` (only when watching: unwatched, the run is as
    /// it was).
    fn run(&self, fast: &mut FastSim, edges: u64) -> Result<Vec<u8>> {
        match self.watching {
            Some(_) => fast.run_edges_watched(edges, &self.signals),
            None => fast.run_edges_sampled(edges, &self.signals),
        }
    }

    /// Runs `edges` clock edges with peripherals answering after each one,
    /// collecting one row of `self.signals` per edge and marking any stale
    /// watched columns `fast::NOT_SAMPLED`.
    fn run_with(
        &self,
        fast: &mut FastSim,
        edges: u64,
        attached: &mut Attached,
        answers: &[DriverId],
    ) -> Result<Vec<u8>> {
        let width = self.signals.len();
        let stale: Vec<usize> = if self.watching.is_some() {
            let program = fast.program();
            (0..width)
                .filter(|&k| !program.fresh(self.signals[k]))
                .collect()
        } else {
            Vec::new()
        };
        let mut rows = Vec::with_capacity(edges as usize * width);
        fast.run_edges_with(edges, &self.signals, answers, |sample, trace, updates| {
            attached.feed(trace);
            if let Some((_, row)) = sample {
                rows.extend_from_slice(row);
                attached.respond_into(updates);
            }
            Ok(true)
        })?;
        if !stale.is_empty() {
            for row in rows.chunks_exact_mut(width) {
                for &k in &stale {
                    row[k] = fast::NOT_SAMPLED;
                }
            }
        }
        Ok(rows)
    }

    /// The watched wires' values in `rows`, `edges` of them after edge
    /// `done`, handed to the window.
    fn deliver(&self, shared: &Mutex<Shared>, rows: &[u8], done: u64, edges: u64) {
        let Some(w) = &self.watching else {
            return;
        };
        let width = self.signals.len();
        let mut out = Vec::with_capacity(edges as usize * w.columns.len());
        for e in 0..edges as usize {
            out.extend(w.columns.iter().map(|c| {
                match c {
                    Some(k) => rows
                        .get(e * width + k)
                        .copied()
                        .unwrap_or(fast::NOT_SAMPLED),
                    None => fast::NOT_SAMPLED,
                }
            }));
        }
        let run = WaveRun {
            id: w.id,
            first: done + 1,
            width: w.columns.len(),
            rows: out,
        };
        deliver(&mut shared.lock().unwrap().waves, run, w.keep);
    }

    /// The UART bytes a run's `rows` complete, the first row after edge
    /// `done + 1`; none without a UART.
    fn uart_bytes(&self, receiver: &mut uart::Receiver, rows: &[u8], done: u64) -> Vec<u8> {
        if !self.uart {
            return Vec::new();
        }
        uart_bytes(receiver, rows, self.signals.len(), done)
    }
}

/// The UART bytes the run's `rows` complete: a row of `width` bytes per
/// clock edge, the TX's level first, the first row after edge `done + 1`.
fn uart_bytes(receiver: &mut uart::Receiver, rows: &[u8], width: usize, done: u64) -> Vec<u8> {
    rows.chunks_exact(width)
        .enumerate()
        .filter_map(|(k, row)| receiver.sample((done + k as u64 + 1) * HALF_NS, row[0] == 1))
        .collect()
}

/// Whether a signal is kept up to date after every clock edge.
trait Freshness {
    fn is_fresh(&self, signal: SignalId) -> bool;
}

impl Freshness for kernel::FastProgram {
    fn is_fresh(&self, signal: SignalId) -> bool {
        self.fresh(signal)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum WatchPhase {
    #[default]
    Idle,
    Building {
        building: usize,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BuildAction {
    None,
    Switch,
    Rebuild(Vec<SignalId>),
}

/// Decides when watched nets need a background program rebuild and what to do
/// when a build finishes, including Watch commands that arrive mid-build.
#[derive(Default)]
struct WatchPlanner {
    current: Vec<SignalId>,
    watched: HashSet<SignalId>,
    active_kept: usize,
    phase: WatchPhase,
}

fn watched_nets(n: usize) -> String {
    if n == 1 {
        "1 watched net".into()
    } else {
        format!("{n} watched nets")
    }
}

impl WatchPlanner {
    fn status(&self) -> Option<String> {
        match &self.phase {
            WatchPhase::Building { building } => Some(format!(
                "Building a program that keeps {}…",
                watched_nets(*building)
            )),
            WatchPhase::Failed(err) => Some(format!("Watch rebuild failed: {err}")),
            WatchPhase::Idle if self.active_kept > 0 => Some(format!(
                "Running a program that keeps {} the default folds: less folding, its speed may differ",
                watched_nets(self.active_kept)
            )),
            WatchPhase::Idle => None,
        }
    }

    fn missing<F: Freshness>(program: &F, signals: &[SignalId]) -> Vec<SignalId> {
        let mut seen = HashSet::new();
        signals
            .iter()
            .copied()
            .filter(|&s| seen.insert(s) && !program.is_fresh(s))
            .collect()
    }

    /// Updates the watched signals and returns the stale ones to add and
    /// rebuild for, or empty if no build should start now.
    fn on_watch<F: Freshness>(&mut self, running: &F, signals: Vec<SignalId>) -> Vec<SignalId> {
        let mut seen = HashSet::new();
        self.current = signals.into_iter().filter(|&s| seen.insert(s)).collect();
        if matches!(self.phase, WatchPhase::Building { .. }) {
            return Vec::new();
        }
        let need = Self::missing(running, &self.current);
        if need.is_empty() {
            self.phase = WatchPhase::Idle;
            return Vec::new();
        }
        self.watched.extend(need.iter().copied());
        self.phase = WatchPhase::Building {
            building: self.watched.len(),
        };
        need
    }

    /// Decides what to do when a background build completes with `built`,
    /// against the currently `running` program and latest watched list.
    fn on_build_ready<F: Freshness>(&mut self, running: &F, built: &F) -> BuildAction {
        if !Self::missing(built, &self.current).is_empty() {
            let need = Self::missing(running, &self.current);
            if need.is_empty() {
                self.phase = WatchPhase::Idle;
                return BuildAction::None;
            }
            self.watched.extend(need.iter().copied());
            self.phase = WatchPhase::Building {
                building: self.watched.len(),
            };
            return BuildAction::Rebuild(need);
        }
        if Self::missing(running, &self.current).is_empty() {
            self.phase = WatchPhase::Idle;
            return BuildAction::None;
        }
        BuildAction::Switch
    }

    fn on_switched(&mut self) {
        if let WatchPhase::Building { building } = self.phase {
            self.active_kept = building;
        }
        self.phase = WatchPhase::Idle;
    }

    fn on_build_failed(&mut self, err: String) {
        let first = err.lines().next().unwrap_or("build failed").trim();
        self.phase = WatchPhase::Failed(first.to_string());
    }
}

type WatchBuildResult = std::result::Result<(fast::Prepared, PathBuf), String>;

fn start_watch_build(
    kernel: &mut kernel::Kernel,
    options: &kernel::FastOptions,
    need: &[SignalId],
    planner: &mut WatchPlanner,
) -> Option<mpsc::Receiver<WatchBuildResult>> {
    for &s in need {
        if let Err(e) = kernel.watch(s) {
            planner.on_build_failed(format!("{e:#}"));
            return None;
        }
    }
    let prepared = match FastSim::prepare(kernel, options) {
        Ok(p) => p,
        Err(e) => {
            planner.on_build_failed(format!("{e:#}"));
            return None;
        }
    };
    let (send, recv) = mpsc::channel();
    match std::thread::Builder::new()
        .name("watch compile".into())
        .spawn(move || {
            let res = prepared
                .compile(true)
                .map(|library| (prepared, library))
                .map_err(|e| format!("{e:#}"));
            let _ = send.send(res);
        }) {
        Ok(_) => Some(recv),
        Err(e) => {
            planner.on_build_failed(format!("{e:#}"));
            None
        }
    }
}

/// Goes to clock edge `target` of the recorded run: the last snapshot at or
/// before it, then on with the inputs of the log, decoding the UART on the
/// way as a run does. Returns the edges run and the time.
fn seek(
    h: &mut History,
    fast: &mut FastSim,
    receiver: &mut uart::Receiver,
    (attached, answers): (&mut Attached, &[DriverId]),
    target: u64,
    (sampled, shared): (&Sampled, &Mutex<Shared>),
    progress: &mut dyn FnMut(u64, f64),
) -> Result<(u64, u64)> {
    let c = h
        .checkpoints
        .iter()
        .rev()
        .find(|c| c.done <= target && c.now_ns <= target * HALF_NS)
        .context("no snapshot before that point")?;
    fast.restore(&c.fast);
    *receiver = c.receiver.clone();
    attached.restore(&c.peripherals);
    h.switches = c.switches;
    h.shown = c.text;
    let (mut done, mut now_ns) = (c.done, c.now_ns);
    let from = done;
    h.cursor = h.log.partition_point(|e| e.0 <= now_ns);
    while done < target {
        let event = h
            .log
            .get(h.cursor)
            .map(|e| e.0)
            .filter(|&ns| (ns - 1) / HALF_NS < target);
        let mut to = event.map_or(target, |ns| ((ns - 1) / HALF_NS).min(target));
        let responding = attached.observe_every_edge();
        // A falling edge alone, then whole cycles, in chunks.
        if responding {
            to = to.min(done + 1024);
        } else if done % 2 == 1 {
            to = to.min(done + 1);
        } else {
            // A traced run keeps a row per instant until it is fed to the
            // peripherals: shorter chunks keep that small.
            to = to.min(
                done + if attached.any() {
                    TRACED_CHUNK
                } else {
                    REBUILD_CHUNK
                },
            );
            if let Some(h) = attached.horizon() {
                to = to.min(done + h);
            }
            if to - done > 1 && (to - done) % 2 == 1 {
                to -= 1;
            }
        }
        if to > done {
            let rows = if responding {
                sampled.run_with(fast, to - done, attached, answers)?
            } else {
                sampled.run(fast, to - done)?
            };
            for b in sampled.uart_bytes(receiver, &rows, done) {
                h.received(b);
            }
            sampled.deliver(shared, &rows, done, to - done);
            if attached.any() && !responding {
                attached.feed(&fast.take_trace());
                answer(fast, attached, answers)?;
            }
            done = to;
            now_ns = now_ns.max(done * HALF_NS);
            if target - from > REBUILD_CHUNK {
                progress(target, (done - from) as f64 / (target - from) as f64);
            }
        }
        if let Some(ns) = event.filter(|&ns| (ns - 1) / HALF_NS == done) {
            let input = h.log[h.cursor].1.clone();
            apply_input(fast, attached, ns, &input)?;
            h.switched(&input);
            if attached.any() {
                attached.feed(&fast.take_trace());
                answer(fast, attached, answers)?;
            }
            h.cursor += 1;
            if ns.is_multiple_of(HALF_NS) {
                done = ns / HALF_NS;
            }
            now_ns = ns;
        }
    }
    Ok((done, now_ns))
}

/// The values of `reader`'s signals now: 0, 1, or 2 for unknown or not in
/// fast mode. A reader keeps where the signals are in the running program,
/// so a million wires are one sweep, not a million lookups.
fn read_values(fast: &FastSim, reader: &std::cell::RefCell<fast::Reader>) -> Vec<u8> {
    let mut out = Vec::new();
    fast.read(&mut reader.borrow_mut(), &mut out);
    out
}

/// The LFE5U-85F's architecture catalogues (the measured behaviour the
/// device model is built from): `$ECP5_ARCHITECTURE`, as a packaged app
/// sets it, else the repository's docs/inventory/LFE5U-85F.
pub fn architecture() -> PathBuf {
    std::env::var_os("ECP5_ARCHITECTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F")
        })
}

/// Per-run input settings for a live design. Defaults retain the window's
/// environment-based start settings.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    /// Initial physical DIP positions, sw[0] first. Some overrides
    /// ECP5_SWITCHES; None reads that setting, defaulting to all OFF.
    pub switches: Option<[bool; 4]>,
}

/// Extended startup inputs. RunOptions::header preserves existing DIP-only
/// struct literals while allowing explicit header settings without env mutation.
#[derive(Clone, Debug, Default)]
pub struct HeaderRunOptions {
    pub switches: Option<[bool; 4]>,
    /// Some overrides ECP5_HEADER completely, including released pins.
    pub header: Option<board::HeaderLevels>,
}
impl From<RunOptions> for HeaderRunOptions {
    fn from(options: RunOptions) -> Self {
        Self {
            switches: options.switches,
            header: None,
        }
    }
}
impl RunOptions {
    /// Override all header positions; use [None; 56] to release every input.
    pub fn header(self, levels: board::HeaderLevels) -> HeaderRunOptions {
        HeaderRunOptions {
            switches: self.switches,
            header: Some(levels),
        }
    }
}

/// Starts the thread. `repaint` is called whenever `shared` has changed.
pub fn spawn(
    design: Design,
    shared: Arc<Mutex<Shared>>,
    repaint: impl Fn() + Send + 'static,
) -> mpsc::Sender<Command> {
    spawn_with(design, RunOptions::default(), shared, repaint)
}

/// Starts a run with explicit input settings, without changing the process
/// environment. `repaint` is called whenever `shared` has changed.
pub fn spawn_with(
    design: Design,
    options: impl Into<HeaderRunOptions>,
    shared: Arc<Mutex<Shared>>,
    repaint: impl Fn() + Send + 'static,
) -> mpsc::Sender<Command> {
    spawn_with_sources(design, options, None, shared, repaint)
}

/// Project windows supply the original input list. Config-only windows use
/// the source copies beside the routed JSON instead.
pub fn spawn_with_sources(
    design: Design,
    options: impl Into<HeaderRunOptions>,
    files: Option<sources::Files>,
    shared: Arc<Mutex<Shared>>,
    repaint: impl Fn() + Send + 'static,
) -> mpsc::Sender<Command> {
    let options = options.into();
    let (send, commands) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let commands = Commands {
                receiver: &commands,
                pending: Default::default(),
            };
            work(
                &design,
                &options,
                files.as_ref(),
                &shared,
                &commands,
                &repaint,
            )
        }));
        let error = match result {
            Ok(Ok(())) => return,
            Ok(Err(e)) if e.is::<Cancelled>() => return,
            Ok(Err(e)) => format!("{e:#}"),
            Err(panic) => format!(
                "internal error: {}",
                panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("panic")
            ),
        };
        eprintln!("live: {error}");
        if let Ok(mut s) = shared.lock() {
            s.running = false;
            s.error = Some(error);
        }
        repaint();
    });
    send
}

/// Says what loading the design is doing now; with `ECP5_LOAD_TIMES` set,
/// prints to stderr how long the stage before took.
fn stage(shared: &Mutex<Shared>, repaint: &dyn Fn(), text: &str) {
    let mut s = shared.lock().unwrap();
    if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
        thread_local!(static SINCE: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) });
        let now = Instant::now();
        if let Some(since) = SINCE.replace(Some(now)) {
            eprintln!(
                "load: {} took {:.1} s",
                s.stage,
                (now - since).as_secs_f64()
            );
        }
    }
    s.stage = text.into();
    drop(s);
    repaint();
}

fn load_elapsed(step: &str, since: Instant) {
    if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
        eprintln!("load: {step} took {:.3} s", since.elapsed().as_secs_f64());
    }
}

fn classify(kind: &str) -> Class {
    if kind == "PLC2" {
        Class::Logic
    } else if kind.contains("EBR") {
        Class::Memory
    } else if kind.contains("DSP") {
        Class::Dsp
    } else if kind.starts_with("PIO") || kind.starts_with("PIC") || kind.contains("PIC") {
        Class::Io
    } else if kind.starts_with("CIB") {
        Class::Routing
    } else if kind.starts_with("DUMMY") {
        Class::Filler
    } else {
        Class::Clock
    }
}

/// Which class wins when several tiles share a position.
fn rank(class: Class) -> u8 {
    match class {
        Class::Logic => 6,
        Class::Memory | Class::Dsp => 5,
        Class::Io => 4,
        Class::Clock => 3,
        Class::Routing => 2,
        Class::Filler => 1,
    }
}

/// `sw[0]` first; the default leaves all four physical contacts open.
fn parse_switches(value: Option<&str>) -> Result<[bool; 4]> {
    board::parse_switches(value)
}

// A startup result is moved once, after the worker has stopped calling C.
// No settings trial, tracing or tuner preparation has started yet.
struct StartedFast {
    fast: FastSim,
    receiver: uart::Receiver,
    text: Vec<u8>,
    done: u64,
}

// SAFETY: this exclusively owns the dlopen handle and its simulation. The
// library has no caller-thread affinity; its settle workers wait between C
// calls, and FastSim's destructor stops and joins them before dlclose. The
// scoped startup worker is joined before the runner uses this result.
unsafe impl Send for StartedFast {}

struct CancelStartup<'a>(&'a AtomicBool);
impl Drop for CancelStartup<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn start_fast(
    kernel: &mut kernel::Kernel,
    clock: DriverId,
    tx: Option<SignalId>,
    cancelled: &AtomicBool,
    progress: &dyn Fn(String),
) -> Result<StartedFast> {
    let check = || -> Result<()> {
        if cancelled.load(Ordering::Relaxed) {
            Err(Cancelled.into())
        } else {
            Ok(())
        }
    };
    let since = Instant::now();
    // Warm start: the event kernel runs whole cycles until fast mode can read
    // everything, as `emulate --fast` does.
    progress("Starting fast mode (clang)".into());
    let mut receiver = uart::Receiver::new(board::BAUD);
    let mut text = Vec::new();
    let mut done = 0_u64;
    let (mut next_try, mut wait) = (0_u64, 2_u64);
    let fast = loop {
        check()?;
        if done >= next_try {
            match FastSim::start(kernel) {
                Ok(fast) => break fast,
                Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && done < 100_000 => {
                    next_try = done + wait;
                    wait *= 2;
                }
                Err(e) => return Err(e),
            }
        }
        for _ in 0..2 {
            done += 1;
            let time = Time::from_ps(done * HALF_NS * 1000);
            kernel.schedule_batch(time, &[(clock, Logic::from_bool(done % 2 == 1))], None)?;
            kernel.run_until(time)?;
            if let Some(tx) = tx {
                if let Some(level) = kernel.value(tx)?.bit() {
                    text.extend(receiver.sample(done * HALF_NS, level));
                }
            }
        }
    };
    check()?;
    if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
        eprintln!(
            "load: fast startup Starting fast mode (clang) took {:.3} s (overlaps geometry/fabric)",
            since.elapsed().as_secs_f64()
        );
    }
    Ok(StartedFast {
        fast,
        receiver,
        text,
        done,
    })
}

fn work(
    design: &Design,
    options: &HeaderRunOptions,
    files: Option<&sources::Files>,
    shared: &Mutex<Shared>,
    commands: &Commands<'_>,
    repaint: &dyn Fn(),
) -> Result<()> {
    let stage = |text: &str| -> Result<()> {
        commands.check()?;
        stage(shared, repaint, text);
        Ok(())
    };
    let switch_positions = match options.switches {
        Some(positions) => positions,
        None => {
            let switch_value = match std::env::var("ECP5_SWITCHES") {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(error) => {
                    return Err(error).context("ECP5_SWITCHES must be four 0/1 characters")
                }
            };
            parse_switches(switch_value.as_deref())?
        }
    };
    let header_positions = options.header.map_or_else(board::header_from_env, Ok)?;
    stage("Loading the device model")?;
    let evidence = architecture();
    let db = Database::discover(None)?;
    let model = cached(commands, |c| c.model.clone())?;
    stage("Reading the configuration")?;
    let image = Arc::new(ConfigurationImage::load(&design.config, &db, None)?);
    let device = Arc::new(ConfiguredDevice::bind(
        Arc::clone(&model),
        image,
        load_catalogue(&model, &evidence)?,
    )?);
    let mut names = design
        .names
        .as_deref()
        .map(|p| {
            Names::load_with_sources(
                p,
                &model,
                board::PACKAGE,
                files.unwrap_or(&sources::Files::beside(&design.config)),
            )
        })
        .transpose()?;

    // The board around the design: only the balls it configures.
    stage("Reading the design's pins")?;
    let used: BTreeMap<String, board::Direction> =
        board::used_balls(&device)?.into_iter().collect();
    let receives = |ball: &str| used.get(ball).is_some_and(|d| d.receives());
    // The board's peripherals that the design's pins call for: an HDMI
    // monitor, the OLED (src/peripherals).
    let attached = RefCell::new(Attached::new(&used));
    let pin = |ball: &str| Reference::Pin(ball.into());
    anyhow::ensure!(
        receives(board::CLOCK),
        "the design does not use the 25 MHz clock on {}",
        board::CLOCK
    );
    let mut inputs = vec![(pin(board::CLOCK), Logic::Zero)];
    let buttons: [bool; 7] = std::array::from_fn(|i| receives(board::BUTTONS[i].ball));
    inputs.extend(
        board::idle_inputs(&device, switch_positions)?
            .into_iter()
            .map(|(ball, level)| (pin(ball), Logic::from_bool(level))),
    );
    let switches = board::SWITCHES.map(receives);
    let mut switch_off = [false; 4];
    for (i, &ball) in board::SWITCHES.iter().enumerate() {
        if switches[i] {
            // Pure fast-mode input pads accept only Boolean levels. OFF is
            // the decoded pull; no/unknown pull uses the documented zero fallback.
            switch_off[i] = board::input_pull(&device, ball)?.unwrap_or(false);
        }
    }
    let header: Vec<_> = board::HEADER
        .iter()
        .enumerate()
        .filter_map(|(i, p)| used.get(p.ball).map(|&d| (i, d)))
        .collect();
    for (ball, level) in board::header_inputs(&device, &header_positions)? {
        inputs.push((pin(ball), level.map_or(Logic::Z, Logic::from_bool)));
    }
    for &(index, direction) in &header {
        if direction.receives() {
            attached
                .borrow_mut()
                .set_header_level(index, header_positions[index]);
        }
    }
    let uart_rx = receives(board::UART_RX);
    // Answering inputs include the RTC open-drain SDA: Logic::Z releases
    // the external driver, leaving the FPGA drive or the pad pull-up.
    for (ball, level) in attached.borrow().input_balls() {
        inputs.push((pin(ball), level.map_or(Logic::Z, Logic::from_bool)));
    }
    // Every output is probed, so the observed scope is the whole design and
    // fast mode keeps those values fresh.
    let probes: Vec<Reference> = used
        .iter()
        .filter(|(_, d)| d.drives())
        .map(|(ball, _)| pin(ball))
        .chain(attached.borrow().balls().into_iter().map(pin))
        .collect();
    anyhow::ensure!(!probes.is_empty(), "the design drives no pin");
    stage("Compiling the execution plan")?;
    let request = Request {
        package: board::PACKAGE.into(),
        scope: Scope::Observed,
        startup: Startup::Configured,
        drives: inputs,
        probes,
        options: kernel::Options {
            trace_capacity: 0,
            ..kernel::Options::default()
        },
    };
    let board = Board::default();
    let compile_key = compile::cache_key(&device, &request, &board);
    let compiling = Instant::now();
    let mut compiled = compile::compile_on_board(Arc::clone(&device), request, &board)?;
    load_elapsed("plan kernel", compiling);
    let driver = |c: &compile::Compiled, ball: &str| -> Result<DriverId> {
        c.drivers
            .get(&pin(ball).label())
            .copied()
            .with_context(|| format!("no driver for pin:{ball}"))
    };
    let probe = |c: &compile::Compiled, ball: &str| c.probes.get(&pin(ball).label()).copied();
    let clock = driver(&compiled, board::CLOCK)?;
    let button_drivers: Vec<Option<DriverId>> = board::BUTTONS
        .iter()
        .zip(&buttons)
        .map(|(b, &used)| used.then(|| driver(&compiled, b.ball)).transpose())
        .collect::<Result<_>>()?;
    let switch_drivers: [Option<DriverId>; 4] = board::SWITCHES
        .iter()
        .zip(&switches)
        .map(|(ball, &used)| used.then(|| driver(&compiled, ball)).transpose())
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .unwrap();
    let header_drivers: Vec<_> = header
        .iter()
        .filter(|(_, d)| d.receives())
        .map(|&(index, direction)| {
            let ball = board::HEADER[index].ball;
            Ok((
                index,
                driver(&compiled, ball)?,
                direction,
                board::input_pull(&device, ball)?,
            ))
        })
        .collect::<Result<_>>()?;
    let rx = uart_rx
        .then(|| driver(&compiled, board::UART_RX))
        .transpose()?;
    let answers: Vec<DriverId> = attached
        .borrow()
        .input_balls()
        .iter()
        .map(|(ball, _)| driver(&compiled, ball))
        .collect::<Result<_>>()?;
    let leds: Vec<Option<SignalId>> = board::LEDS.iter().map(|b| probe(&compiled, b)).collect();
    let tx = probe(&compiled, board::UART_TX);

    // The peripherals' pads after every instant: an HDMI lane changes ten
    // times a pixel, faster than the board clock. Every output is probed, so
    // each pad has its signal.
    let traced: Vec<SignalId> = attached
        .borrow()
        .balls()
        .iter()
        .map(|b| probe(&compiled, b).with_context(|| format!("no peripheral probe for pin:{b}")))
        .collect::<Result<_>>()?;
    let cancelled = AtomicBool::new(false);
    let (
        cells,
        rows,
        cols,
        geometry,
        fabric,
        overlay,
        net_names,
        settings,
        transfers,
        summary,
        started,
    ) = std::thread::scope(|scope| -> Result<_> {
        let _cancel_on_exit = CancelStartup(&cancelled);
        let (progress, messages) = mpsc::channel();
        let kernel = &mut compiled.kernel;
        let cancelled = &cancelled;
        let startup = scope.spawn(move || {
            start_fast(kernel, clock, tx, cancelled, &|text| {
                let _ = progress.send(text);
            })
        });
        // The die and what the design uses in it.
        let mut cells: Vec<Cell> = Vec::new();
        let mut at: BTreeMap<(i16, i16), usize> = BTreeMap::new();
        let (mut rows, mut cols) = (0, 0);
        for (_, tile) in model.tiles() {
            let class = classify(&tile.kind);
            let i = *at.entry((tile.row, tile.col)).or_insert_with(|| {
                cells.push(Cell {
                    row: tile.row as u16,
                    col: tile.col as u16,
                    class,
                    tiles: Vec::new(),
                    fields: 0,
                    routes: 0,
                    blocks: Vec::new(),
                    names: Vec::new(),
                });
                cells.len() - 1
            });
            let cell = &mut cells[i];
            if rank(class) > rank(cell.class) {
                cell.class = class;
            }
            cell.tiles.push(format!("{} {}", tile.name, tile.kind));
            rows = rows.max(tile.row as u16 + 1);
            cols = cols.max(tile.col as u16 + 1);
        }
        let cell_of_tile = |name: &str| -> Option<usize> {
            let tile = model.tile(model.tile_id(name)?);
            at.get(&(tile.row, tile.col)).copied()
        };
        for field in &compiled.report.fields {
            if let Some(i) = cell_of_tile(&field.tile) {
                cells[i].fields += 1;
            }
        }
        for route in &compiled.report.routes {
            if let Some(i) = cell_of_tile(&route.tile) {
                cells[i].routes += 1;
            }
        }
        for resource in &compiled.report.resources {
            if let Some(id) = model.resource_id(&resource.id) {
                let r = model.resource(id);
                if let Some(&i) = at.get(&(r.location.row, r.location.col)) {
                    cells[i].blocks.push(r.name.clone());
                }
            }
        }
        if let Some(names) = &names {
            for (name, reference) in names.resolved() {
                if let Reference::Wire(wire) = reference {
                    if let Some(&i) = at.get(&(wire.location.row, wire.location.col)) {
                        cells[i].names.push(name.into());
                    }
                }
            }
        }
        stage("Drawing the die's fabric (alongside layout geometry)")?;
        commands.check()?;
        // Only the logic-use masks feed the overlay. Keep the complete cached
        // geometry on a scoped worker, joined before any geometry user.
        let drawing_logic = Instant::now();
        let logic = layout::logic_usage(&model, &compiled.report);
        load_elapsed("layout logic use before overlay", drawing_logic);
        let geometry = scope.spawn(|| {
            let drawing = Instant::now();
            let geometry = layout::cached(
                &model,
                &compiled.report,
                compile_key.as_ref().map(layout::GeometryKey::new).as_ref(),
            );
            load_elapsed("layout geometry (overlaps fabric overlay)", drawing);
            geometry
        });
        let fabric = cached(commands, |c| c.fabric.clone())?;
        let drawing_overlay = Instant::now();
        let overlay = fabric_overlay::cached(
            &fabric,
            &model,
            &compiled.report.routes,
            &compiled.report.constants,
            &logic,
            &compiled.report.fields,
            compile_key
                .as_ref()
                .map(fabric_overlay::OverlayKey::new)
                .as_ref(),
        );
        load_elapsed("fabric overlay (overlaps layout geometry)", drawing_overlay);
        let mut net_names: HashMap<u32, Vec<String>> = HashMap::new();
        if let Some(names) = &names {
            for (name, reference) in names.observations() {
                // Unidirectional ports observe only their selected data net.
                // Bidirectional ports retain their individually named PIO nets.
                let wires: Vec<(Wire, String)> = match reference {
                    Reference::Wire(wire) => vec![(wire.clone(), name.to_string())],
                    Reference::Pin(ball) => model
                        .package_pin(board::PACKAGE, ball)
                        .and_then(|p| p.pio)
                        .map(|id| {
                            model
                                .resource_ports(id)
                                .into_iter()
                                .map(|p| (p.wire, format!("{name} ({})", p.name)))
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                for (wire, label) in wires {
                    if let Some(v) = overlay.value_of(&model, &wire) {
                        net_names
                            .entry(overlay.nets[v as usize])
                            .or_default()
                            .push(label);
                    }
                }
            }
        }
        for list in net_names.values_mut() {
            list.sort();
            list.dedup();
        }
        let position_of = |tile: &str| {
            model
                .tile_id(tile)
                .map(|t| model.tile(t))
                .map(|t| (t.row as u16, t.col as u16))
        };
        let mut settings: HashMap<(u16, u16), Vec<Setting>> = HashMap::new();
        for field in &compiled.report.fields {
            if let Some(at) = position_of(&field.tile) {
                settings
                    .entry(at)
                    .or_default()
                    .push((field.name.clone(), field.candidates.clone()));
            }
        }
        let mut transfers: HashMap<(u16, u16), Vec<(String, String)>> = HashMap::new();
        for route in &compiled.report.routes {
            if let Some(at) = position_of(&route.tile) {
                transfers
                    .entry(at)
                    .or_default()
                    .push((route.source.clone(), route.sink.clone()));
            }
        }
        let counts = &compiled.report.counts;
        let summary = format!(
            "{} LUT4, {} FF, {} carry halves, {} LUTRAM columns, {} EBR, {} PIO",
            counts.lut4,
            counts.flip_flops,
            counts.carry_halves,
            counts.lutram_columns,
            counts.ebr_blocks,
            counts.pio
        );

        // Scope exit also joins this worker on cancellation or failure.
        while !geometry.is_finished() {
            commands.check()?;
            std::thread::sleep(Duration::from_millis(20));
        }
        let geometry = geometry
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));

        // Only announce fast work still in flight. Completed startup
        // stages were concurrent with the geometry and overlay above.
        let mut announced = String::new();
        while !startup.is_finished() {
            commands.check()?;
            if let Some(text) = messages.try_iter().last() {
                if text != announced {
                    stage(&text)?;
                    announced = text;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        commands.check()?;
        let started = startup
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        Ok((
            cells, rows, cols, geometry, fabric, overlay, net_names, settings, transfers, summary,
            started,
        ))
    })?;
    let value_signals: Vec<Option<SignalId>> = geometry
        .values
        .iter()
        .map(|w| compiled.kernel.signal(w))
        .collect();
    // A clock branch's value is that of its segment, as the plan has it.
    let lit_signals: Vec<Option<SignalId>> = overlay
        .values
        .iter()
        .map(|w| {
            let w = model
                .clock_segment(w)
                .ok()
                .flatten()
                .unwrap_or_else(|| w.clone());
            compiled.kernel.signal(&w)
        })
        .collect();
    let StartedFast {
        mut fast,
        mut receiver,
        mut text,
        mut done,
    } = started;
    let choice_memory = fast::choice_memory(&compiled.kernel);
    // Trials and tuner preparation must see idle cores, after every scoped
    // geometry/overlay worker and the non-measuring startup have joined.
    if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
        eprintln!("load: geometry and overlay complete; fast settings may now be measured");
    }
    let probes: Vec<SignalId> = tx.into_iter().collect();
    if std::env::var("FAST_CHOOSE").map_or(true, |v| v.trim() != "0") {
        stage("Choosing fast mode's settings…")?;
        let since = Instant::now();
        let say = |text: String| self::stage(shared, repaint, &text);
        let cancelled = || commands.check().is_err();
        let chosen = fast::choose_options(
            &compiled.kernel,
            fast,
            clock,
            Time::from_ps(board::CLOCK_PERIOD_NS * 1000),
            &probes,
            &say,
            &cancelled,
        );
        commands.check()?;
        fast = match chosen {
            Ok(chosen) => chosen,
            Err(e) => {
                eprintln!("fast mode: choosing its settings failed, the defaults run: {e:#}");
                FastSim::with_options(&compiled.kernel, &kernel::FastOptions::from_env())?
            }
        };
        load_elapsed("fast settings after overlay", since);
    }
    fast.set_choice_memory(choice_memory.clone());
    commands.check()?;
    // Trials install private clocks, so public clock setup follows selection.
    fast.clock(clock, Time::from_ps(board::CLOCK_PERIOD_NS * 1000))?;
    if !traced.is_empty() {
        fast.trace(&traced)?;
    } else {
        let options = fast.program().options;
        let starting = format!(
            "Starting with {} thread{}, folding {}; tuning in the background",
            options.threads,
            if options.threads == 1 { "" } else { "s" },
            options.collapse
        );
        let announce = shared.lock().unwrap().stage != starting;
        if announce {
            stage(&starting)?;
        }
        let since = Instant::now();
        fast.tune(&compiled.kernel)?;
        commands.check()?;
        load_elapsed("fast tuner preparation after overlay", since);
    }
    // The UART text of the warm start, the first of the run's.
    let statics_text: Vec<u8>;
    let statics = Arc::new(Static {
        rows,
        cols,
        cells,
        geometry,
        fabric,
        overlay,
        net_names,
        sources: OnceLock::new(),
        source_seconds: OnceLock::new(),
        timing: OnceLock::new(),
        model: Arc::clone(&model),
        device: Arc::clone(&device),
        settings,
        transfers,
        leds: std::array::from_fn(|i| leds[i].is_some()),
        buttons,
        switches,
        header,
        uart_tx: tx.is_some(),
        uart_rx,
        summary,
        routes: compiled.report.routes.len(),
    });
    stage("Ready")?;
    {
        let mut s = shared.lock().unwrap();
        s.ready = Some(Arc::clone(&statics));
        s.stage = "Paused".into();
        statics_text = text.clone();
        s.text = std::mem::take(&mut text);
        s.cycles = done / 2;
        s.switches = switch_positions;
        s.header_levels = attached.borrow().header_levels();
        s.rate = Some(REAL_TIME);
    }
    repaint();
    if let Some(names) = names.as_mut() {
        let collected = std::mem::take(&mut names.sources);
        let started = Instant::now();
        let index = collected
            .build(|name| names.overlay_nets(name, &model, board::PACKAGE, &statics.overlay));
        let seconds = started.elapsed().as_secs_f64();
        if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
            eprintln!(
                "probe load-pass {:.6} s; index {:.6} s {} nets {} locations",
                index.collect_time.as_secs_f64(),
                seconds,
                index.nets.len(),
                index.locations
            );
        }
        let _ = statics.source_seconds.set(seconds);
        let _ = statics.sources.set(Arc::new(index));
        repaint();
    }
    let started = Instant::now();
    let mut bytes = 0;
    let result = (|| {
        let names = names.as_ref().ok_or(timing_path::NO_PATH)?;
        timing_path::check_version(&names.creator)?;
        let log = design
            .names
            .as_ref()
            .and_then(|p| p.parent())
            .ok_or(timing_path::NO_PATH)?
            .join("nextpnr.log");
        let log = std::fs::read_to_string(log).map_err(|_| timing_path::NO_PATH)?;
        bytes = log.len();
        let report = timing_path::parse(&names.creator, &log)?;
        let index = statics.sources.get().ok_or(timing_path::NO_PATH)?;
        Ok(report.map(names, &model, &statics.overlay, index))
    })();
    let seconds = started.elapsed().as_secs_f64();
    if std::env::var_os("ECP5_LOAD_TIMES").is_some() {
        eprintln!("critical after-ready {seconds:.6} s; {bytes} log bytes");
    }
    let _ = statics.timing.set(timing_path::Published {
        result,
        seconds,
        bytes,
    });
    // Timing and source mapping own their compact results now.
    drop(names);
    repaint();

    let mut transmitter = uart::Transmitter::new(board::BAUD);
    let mut presses: VecDeque<QueuedPin> = VecDeque::new();
    let mut running = design.run;
    let mut budget: Option<u64> = None;
    let mut rate = Some(REAL_TIME);
    let mut now_ns = done * HALF_NS;
    let mut pace: Option<(Instant, u64)> = None;
    let mut published = Instant::now() - PUBLISH;
    let mut speed = (Instant::now(), done);
    let mut mhz = 0.0;
    let mut sampled = Sampled {
        signals: tx.into_iter().collect(),
        uart: tx.is_some(),
        watching: None,
    };
    let mut history = History::new(
        done,
        now_ns,
        &fast,
        &receiver,
        &attached.borrow(),
        &statics_text,
        switch_positions,
    );
    // A long way back or on shows how far it has come.
    let mut progress = |target: u64, share: f64| {
        shared.lock().unwrap().stage =
            format!("Going to cycle {}: {:.0}%", target / 2, share * 100.0);
        repaint();
    };
    // `back`: the run went back, and the console shows the text up to here.
    // `prior`: the values of the wires two edges and one edge back, to say
    // which changed in the last cycle with those read now; `stepped`: the
    // wires that changed on the last edges of a step.
    // The overlay's values as last published: only this thread writes them.
    let given: std::cell::RefCell<Vec<u8>> = Default::default();
    // The wires' and the overlay's signals, found in the program once.
    let value_reader = std::cell::RefCell::new(fast::Reader::new(&value_signals));
    let lit_reader = std::cell::RefCell::new(fast::Reader::new(&lit_signals));
    let publish = |fast: &mut FastSim,
                   text: &mut Vec<u8>,
                   (running, rate): (bool, Option<f64>),
                   done: u64,
                   mhz: f64,
                   (history, back): (&History, bool),
                   (prior, stepped): (Option<[Vec<u8>; 2]>, Option<Vec<u8>>)|
     -> Result<()> {
        // Gates that only registers with an enable read may be left from an
        // earlier cycle: settle everything before reading the LUT outputs.
        fast.step()?;
        // All read before taking the lock, which the window waits on: a big
        // design has a million wires and more.
        let values = read_values(fast, &value_reader);
        let lit = read_values(fast, &lit_reader);
        let console = back.then(|| history.console());
        let peripherals = attached.borrow_mut().view();
        // What changed, added under the lock to what the window has not
        // taken yet (a pause or going back lets that go).
        let mut changed = Vec::new();
        {
            let given = given.borrow();
            if running || stepped.is_some() {
                let every = running && rate.is_some_and(|r| r <= 1000.0);
                for (i, &v) in lit.iter().enumerate() {
                    let watched = prior
                        .as_ref()
                        .is_some_and(|[a, b]| a[i] != b[i] || b[i] != v);
                    let read = every && given.get(i).is_some_and(|&was| was != v);
                    let step = stepped.as_ref().is_some_and(|c| c[i] == 1);
                    if watched || read || step {
                        changed.push(i);
                    }
                }
            }
        }
        given.borrow_mut().clone_from(&lit);
        let mut s = shared.lock().unwrap();
        s.running = running;
        s.rate = rate;
        s.cycles = done / 2;
        s.edges = done;
        s.mhz = mhz;
        let options = fast.program().options;
        s.fast = Some(FastChoice {
            threads: options.threads,
            folding: options.collapse,
        });
        s.switches = history.switches;
        s.header_levels = attached.borrow().header_levels();
        s.leds =
            std::array::from_fn(|i| leds[i].and_then(|l| fast.value(l)).map(|v| v == Logic::One));
        s.earliest = history.checkpoints.first().map_or(0, |c| c.done / 2);
        s.ahead = history.furthest.saturating_sub(done) / 2;
        if let Some(console) = console {
            s.console = Some(console);
            s.text.clear();
            text.clear();
        }
        s.text.append(text);
        s.values = values;
        s.peripherals = peripherals;
        let sh = &mut *s;
        if !changed.is_empty() && sh.toggled.len() != lit.len() {
            sh.toggled = vec![0; lit.len()];
        }
        for i in changed {
            sh.toggled[i] = 1;
        }
        sh.lit = lit;
        s.published = s.published.wrapping_add(1);
        s.stage = if running {
            "Running".into()
        } else {
            "Paused".into()
        };
        drop(s);
        repaint();
        Ok(())
    };
    publish(
        &mut fast,
        &mut text,
        (false, rate),
        done,
        0.0,
        (&history, false),
        (None, None),
    )?;
    // The values two edges and one edge before the next sample, and when
    // they were last read.
    let mut prior: Option<[Vec<u8>; 2]> = None;
    // The wires that changed on the last edges of the step being run.
    let mut stepped: Option<Vec<u8>> = None;
    let mut watched = Instant::now() - WATCH;
    // How long the last run of edges took, and how many edges a run is to
    // be at full speed.
    let mut took = Duration::ZERO;
    let mut chunk = CHUNK;
    let mut watch_planner = WatchPlanner::default();
    let mut watch_build: Option<mpsc::Receiver<WatchBuildResult>> = None;
    let set_watch_status = |status: Option<String>| {
        let mut s = shared.lock().unwrap();
        if s.watch_status != status {
            s.watch_status = status;
            drop(s);
            repaint();
        }
    };
    loop {
        if let Some(recv) = watch_build.as_ref() {
            match recv.try_recv() {
                Ok(res) => {
                    watch_build = None;
                    match res {
                        Ok((prepared, library)) => {
                            match watch_planner.on_build_ready(fast.program(), prepared.program()) {
                                BuildAction::None => {}
                                BuildAction::Rebuild(need) => {
                                    watch_build = start_watch_build(
                                        &mut compiled.kernel,
                                        &fast.program().options,
                                        &need,
                                        &mut watch_planner,
                                    );
                                }
                                BuildAction::Switch => {
                                    let built = (|| -> Result<FastSim> {
                                        let mut next = FastSim::from_prepared(prepared, &library)?;
                                        next.set_choice_memory(choice_memory.clone());
                                        next.clock(
                                            clock,
                                            Time::from_ps(board::CLOCK_PERIOD_NS * 1000),
                                        )?;
                                        if !traced.is_empty() {
                                            next.trace(&traced)?;
                                        } else {
                                            next.tune(&compiled.kernel)?;
                                        }
                                        Ok(next)
                                    })();
                                    match built {
                                        Ok(mut next_fast) => {
                                            history.keep(
                                                done,
                                                now_ns,
                                                &fast,
                                                &receiver,
                                                &attached.borrow(),
                                                true,
                                            );
                                            (done, now_ns) = seek(
                                                &mut history,
                                                &mut next_fast,
                                                &mut receiver,
                                                (&mut attached.borrow_mut(), &answers),
                                                done,
                                                (&sampled, shared),
                                                &mut progress,
                                            )?;
                                            fast = next_fast;
                                            watch_planner.on_switched();
                                            if !running && budget.is_none() {
                                                publish(
                                                    &mut fast,
                                                    &mut text,
                                                    (false, rate),
                                                    done,
                                                    0.0,
                                                    (&history, false),
                                                    (None, None),
                                                )?;
                                                published = Instant::now();
                                            }
                                        }
                                        Err(e) => watch_planner.on_build_failed(format!("{e:#}")),
                                    }
                                }
                            }
                        }
                        Err(e) => watch_planner.on_build_failed(e),
                    }
                    set_watch_status(watch_planner.status());
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    watch_build = None;
                    watch_planner.on_build_failed("watch program compile stopped".into());
                    set_watch_status(watch_planner.status());
                }
            }
        }
        // Commands; block while paused (briefly while a watch build runs).
        let wait = (!running && budget.is_none()).then(|| {
            if watch_build.is_some() {
                Duration::from_millis(25)
            } else {
                Duration::from_millis(200)
            }
        });
        let mut next = commands.next_wait(wait)?;
        while let Some(mut command) = next.take() {
            // A cycle to go to is a way back or on from here.
            if let Command::GoTo(cycle) = command {
                let now = done / 2;
                command = match cycle.cmp(&now) {
                    std::cmp::Ordering::Less => Command::Back(now - cycle),
                    std::cmp::Ordering::Greater => Command::Step(cycle - now),
                    std::cmp::Ordering::Equal => Command::Pause,
                };
            }
            match command {
                Command::GoTo(_) => unreachable!("a way back or on"),
                Command::Run => {
                    running = true;
                    budget = None;
                    pace = None;
                    speed = (Instant::now(), done);
                }
                Command::Pause => {
                    running = false;
                    budget = None;
                    shared.lock().unwrap().toggled.clear();
                    history.keep(done, now_ns, &fast, &receiver, &attached.borrow(), true);
                    // The window shows it paused at once.
                    publish(
                        &mut fast,
                        &mut text,
                        (false, rate),
                        done,
                        0.0,
                        (&history, false),
                        (None, None),
                    )?;
                    published = Instant::now();
                }
                Command::Step(cycles) => {
                    running = false;
                    budget = Some(cycles * 2);
                    stepped = None;
                    // Within the recorded run, from a snapshot between here
                    // and there, if there is one: the rest runs as a step.
                    let target = done + cycles * 2;
                    let ahead = history
                        .checkpoints
                        .iter()
                        .rev()
                        .find(|c| c.done <= target)
                        .is_some_and(|c| c.done > done);
                    if target <= history.furthest && ahead {
                        (done, now_ns) = seek(
                            &mut history,
                            &mut fast,
                            &mut receiver,
                            (&mut attached.borrow_mut(), &answers),
                            target,
                            (&sampled, shared),
                            &mut progress,
                        )?;
                        budget = None;
                        history.keep(done, now_ns, &fast, &receiver, &attached.borrow(), true);
                        publish(
                            &mut fast,
                            &mut text,
                            (false, rate),
                            done,
                            mhz,
                            (&history, true),
                            (None, None),
                        )?;
                        published = Instant::now();
                    }
                }
                Command::Back(cycles) => {
                    running = false;
                    budget = None;
                    stepped = None;
                    shared.lock().unwrap().toggled.clear();
                    // Inputs not applied yet are let go.
                    presses.clear();
                    transmitter = uart::Transmitter::new(board::BAUD);
                    history.keep(done, now_ns, &fast, &receiver, &attached.borrow(), true);
                    let first = history.checkpoints.first().map_or(0, |c| c.done);
                    let target = done.saturating_sub(cycles * 2).max(first);
                    (done, now_ns) = seek(
                        &mut history,
                        &mut fast,
                        &mut receiver,
                        (&mut attached.borrow_mut(), &answers),
                        target,
                        (&sampled, shared),
                        &mut progress,
                    )?;
                    history.keep(done, now_ns, &fast, &receiver, &attached.borrow(), true);
                    publish(
                        &mut fast,
                        &mut text,
                        (false, rate),
                        done,
                        mhz,
                        (&history, true),
                        (None, None),
                    )?;
                    published = Instant::now();
                }
                Command::Button(i, pressed) => {
                    history.branch(done, now_ns);
                    if let Some(d) = button_drivers[i] {
                        let t = presses.back().map_or(now_ns, |p| p.0).max(now_ns) + 1;
                        presses.push_back((
                            t,
                            d,
                            Some(pressed != board::BUTTONS[i].idle),
                            None,
                            None,
                        ));
                    }
                }
                Command::Switch(i, on) => {
                    if let Some(Some(driver)) = switch_drivers.get(i) {
                        history.branch(done, now_ns);
                        let t = presses.back().map_or(now_ns, |p| p.0).max(now_ns) + 1;
                        presses.push_back((
                            t,
                            *driver,
                            Some(on || switch_off[i]),
                            Some((i, on)),
                            None,
                        ));
                    }
                }
                Command::HeaderLevel(index, level) => {
                    if let Some(&(_, driver, direction, pull)) =
                        header_drivers.iter().find(|d| d.0 == index)
                    {
                        history.branch(done, now_ns);
                        let t = presses.back().map_or(now_ns, |p| p.0).max(now_ns) + 1;
                        presses.push_back((
                            t,
                            driver,
                            board::header_level(direction, pull, level),
                            None,
                            Some((index, level)),
                        ));
                    }
                }
                Command::AdcVolts(channel, volts) => {
                    let accepted = attached.borrow_mut().set_volts(channel, volts);
                    if accepted {
                        history.branch(done, now_ns);
                        let ns = now_ns + 1;
                        let input = InputChange::AdcVolts(channel, volts);
                        // Analog sources change now without clocking a paused
                        // design. Several slider moves can share this time;
                        // replay applies them in their original order.
                        history.log.push((ns, input));
                        history.cursor = history.log.len();
                        publish(
                            &mut fast,
                            &mut text,
                            (running, rate),
                            done,
                            mhz,
                            (&history, false),
                            (None, None),
                        )?;
                        published = Instant::now();
                    }
                }
                Command::Keys(bytes) => {
                    history.branch(done, now_ns);
                    bytes.into_iter().for_each(|b| transmitter.push(b));
                }
                Command::Rate(r) => {
                    rate = r;
                    pace = None;
                }
                Command::Watch { id, wires, keep } => {
                    sampled.watch(id, &wires, keep, &lit_signals);
                    let signals: Vec<SignalId> = wires
                        .iter()
                        .filter_map(|&w| lit_signals.get(w as usize).copied().flatten())
                        .collect();
                    let need = watch_planner.on_watch(fast.program(), signals);
                    if !need.is_empty() {
                        watch_build = start_watch_build(
                            &mut compiled.kernel,
                            &fast.program().options,
                            &need,
                            &mut watch_planner,
                        );
                    }
                    set_watch_status(watch_planner.status());
                }
            }
            next = commands.next(false)?;
        }
        if !running && budget.is_none() {
            if published.elapsed() >= PUBLISH {
                publish(
                    &mut fast,
                    &mut text,
                    (false, rate),
                    done,
                    0.0,
                    (&history, false),
                    (None, None),
                )?;
                published = Instant::now();
            }
            continue;
        }
        // At a set rate, run only the edges that are due.
        let mut due = u64::MAX;
        if let (true, Some(r)) = (running, rate) {
            let (start, from) = *pace.get_or_insert((Instant::now(), done));
            due = from + (start.elapsed().as_secs_f64() * r * 2.0) as u64;
            if due <= done {
                let next = Duration::from_secs_f64((done + 1 - from) as f64 / (r * 2.0));
                std::thread::sleep(
                    next.saturating_sub(start.elapsed())
                        .min(Duration::from_millis(10)),
                );
                continue;
            }
        }
        // The next input change: a button, a bit of a key being sent, or
        // one recorded, after going back.
        let pressed = presses.front().map(|p| p.0);
        let typed = rx
            .and_then(|_| transmitter.next(now_ns + 1))
            .map(|(ns, _)| ns);
        let logged = history.log.get(history.cursor).map(|e| e.0);
        let change = pressed.into_iter().chain(typed).chain(logged).min();
        let mut to = (done + budget.unwrap_or(chunk).min(chunk)).min(due);
        if let Some(ns) = change {
            to = to.min((ns - 1) / HALF_NS);
        }
        // Answer every edge inside a bounded batch. Commands, pacing,
        // inputs, checkpoints and publishing are checked between batches.
        let responding = attached.borrow().observe_every_edge();
        if responding {
            to = to.min(done + 1024);
        } else if let Some(h) = attached.borrow().horizon() {
            to = to.min(done + h);
        }
        // At full speed the window sees a sample of the values now and then:
        // every WATCH the last cycle before one is run edge by edge, to see
        // which wires change in it, so that those glow even if the sample
        // shows them as the last did (a clock, at an even number of edges).
        // The sample itself is the third reading. Answering runs use the same
        // edges and keep answering between these readings too.
        let due_to_publish = published.elapsed() + took >= PUBLISH;
        let watch = running
            && !rate.is_some_and(|r| r <= 1000.0)
            && due_to_publish
            && watched.elapsed() >= WATCH
            && to >= done + 3;
        let started = Instant::now();
        let edges = to.saturating_sub(done);
        if to > done {
            let rows = if watch {
                let run = |fast: &mut FastSim, edges| {
                    if responding {
                        sampled.run_with(fast, edges, &mut attached.borrow_mut(), &answers)
                    } else {
                        sampled.run(fast, edges)
                    }
                };
                let mut rows = run(&mut fast, to - done - 2)?;
                fast.step()?;
                let a = read_values(&fast, &lit_reader);
                rows.extend(run(&mut fast, 1)?);
                fast.step()?;
                let b = read_values(&fast, &lit_reader);
                rows.extend(run(&mut fast, 1)?);
                prior = Some([a, b]);
                watched = Instant::now();
                rows
            } else if responding {
                let mut attached = attached.borrow_mut();
                sampled.run_with(&mut fast, to - done, &mut attached, &answers)?
            } else if budget == Some(to - done) && !running {
                // The last edges of a step one by one, each read.
                let tail = (to - done).min(STEP_EDGES);
                let mut rows = Vec::new();
                if to - done > tail {
                    rows = sampled.run(&mut fast, to - done - tail)?;
                }
                fast.step()?;
                let mut last = read_values(&fast, &lit_reader);
                let changed = stepped.get_or_insert_with(|| vec![0; last.len()]);
                for _ in 0..tail {
                    rows.extend(sampled.run(&mut fast, 1)?);
                    fast.step()?;
                    let now = read_values(&fast, &lit_reader);
                    for (i, (&n, l)) in now.iter().zip(&last).enumerate() {
                        if n != *l {
                            changed[i] = 1;
                        }
                    }
                    last = now;
                }
                rows
            } else {
                sampled.run(&mut fast, to - done)?
            };
            for b in sampled.uart_bytes(&mut receiver, &rows, done) {
                text.push(b);
                history.received(b);
            }
            sampled.deliver(shared, &rows, done, to - done);
            if let Some(b) = &mut budget {
                *b -= to - done;
            }
            took = started.elapsed();
            // A full run of edges says how many fit in a publish.
            if edges == chunk && took > Duration::ZERO {
                let rate = edges as f64 / took.as_secs_f64();
                chunk = ((rate * PUBLISH.as_secs_f64()) as u64).clamp(64, CHUNK);
            }
            done = to;
            now_ns = now_ns.max(done * HALF_NS);
        }
        let input_at = change.filter(|&ns| (ns - 1) / HALF_NS == done);
        if let Some(ns) = input_at {
            let set = if logged == Some(ns) {
                history.cursor += 1;
                history.log[history.cursor - 1].1.clone()
            } else {
                let mut set = Vec::new();
                let mut position = None;
                let mut header = None;
                if pressed == Some(ns) {
                    let (_, d, level, switch, external) = presses.pop_front().unwrap();
                    header = external;
                    position = switch;
                    set.push((d, level));
                }
                if typed == Some(ns) {
                    let (_, level) = transmitter.pop().unwrap();
                    set.push((rx.unwrap(), Some(level)));
                }
                let set = InputChange::Pins(set, position, header);
                history.log.push((ns, set.clone()));
                history.cursor = history.log.len();
                set
            };
            apply_input(&mut fast, &mut attached.borrow_mut(), ns, &set)?;
            history.switched(&set);
            if ns.is_multiple_of(HALF_NS) {
                done = ns / HALF_NS;
            }
            now_ns = ns;
        }
        // What the peripherals' pads did in those edges and that input.
        if !traced.is_empty() && (!responding || input_at.is_some()) {
            let mut attached = attached.borrow_mut();
            attached.feed(&fast.take_trace());
            answer(&mut fast, &mut attached, &answers)?;
        }
        history.furthest = history.furthest.max(done);
        let stopped = budget == Some(0);
        history.keep(done, now_ns, &fast, &receiver, &attached.borrow(), stopped);
        if stopped {
            budget = None;
        }
        if speed.0.elapsed() >= Duration::from_millis(500) {
            mhz = (done - speed.1) as f64 / 2.0 / speed.0.elapsed().as_secs_f64() / 1e6;
            speed = (Instant::now(), done);
        }
        // Slow runs show every edge.
        let slow = running && rate.is_some_and(|r| r <= 1000.0);
        // Decided before the edges, so that a sample comes with what changed.
        if due_to_publish || budget.is_none() && !running || slow {
            let (prior, stepped) = (prior.take(), if stopped { stepped.take() } else { None });
            publish(
                &mut fast,
                &mut text,
                (running, rate),
                done,
                mhz,
                (&history, false),
                (prior, stepped),
            )?;
            published = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_positions_parse_in_board_order() {
        assert_eq!(parse_switches(None).unwrap(), [false; 4]);
        assert_eq!(
            parse_switches(Some("1010")).unwrap(),
            [true, false, true, false]
        );
        assert_eq!(parse_switches(Some("0000")).unwrap(), [false; 4]);
        assert_eq!(parse_switches(Some("1111")).unwrap(), [true; 4]);
        for value in [
            "", "101", "10101", "1020", "true", " 1010", "1010\n", "١٠١٠",
        ] {
            assert!(parse_switches(Some(value))
                .unwrap_err()
                .to_string()
                .contains("ECP5_SWITCHES"));
        }
    }

    /// Watched rows go to the window joined while they follow on, apart
    /// when they do not (going back) or are of another list, and no more
    /// than twice the edges it keeps.
    #[test]
    fn wave_runs_join_and_keep_to_twice_the_window() {
        let run = |id, first, rows: &[u8]| WaveRun {
            id,
            first,
            width: 2,
            rows: rows.to_vec(),
        };
        let mut waves = Vec::new();
        deliver(&mut waves, run(1, 1, &[0, 1, 1, 1]), 10);
        deliver(&mut waves, run(1, 3, &[0, 0]), 10);
        assert_eq!(waves, [run(1, 1, &[0, 1, 1, 1, 0, 0])]);
        // Back to edge 2: apart; another list: apart.
        deliver(&mut waves, run(1, 2, &[1, 1]), 10);
        deliver(&mut waves, run(2, 3, &[1, 0]), 10);
        assert_eq!(waves.len(), 3);
        // Past twice `keep` (4 edges) the oldest go.
        deliver(&mut waves, run(2, 4, &[1, 1, 0, 0]), 2);
        assert_eq!(waves.iter().map(WaveRun::edges).sum::<u64>(), 4);
        assert_eq!(waves[0], run(1, 2, &[1, 1]));
        assert_eq!(waves.last().unwrap(), &run(2, 3, &[1, 0, 1, 1, 0, 0]));
    }

    /// The watched wires are the columns after the UART's, a wire without
    /// a signal not sampled; watching nothing samples the UART alone.
    #[test]
    fn watched_wires_are_columns_after_the_uart() {
        let shared = Mutex::new(Shared::default());
        let signal = |index| SignalId::for_tests(index);
        let mut sampled = Sampled {
            signals: vec![signal(100)],
            uart: true,
            watching: None,
        };
        let signal_of = [Some(signal(0)), None, Some(signal(2))];
        sampled.watch(7, &[2, 1, 0], 64, &signal_of);
        assert_eq!(sampled.signals, [signal(100), signal(2), signal(0)]);
        // Two edges: the TX, wire 2, wire 0.
        sampled.deliver(&shared, &[1, 0, 1, 1, 1, 0], 10, 2);
        let waves = std::mem::take(&mut shared.lock().unwrap().waves);
        assert_eq!(
            waves,
            [WaveRun {
                id: 7,
                first: 11,
                width: 3,
                rows: vec![0, fast::NOT_SAMPLED, 1, 1, fast::NOT_SAMPLED, 0],
            }]
        );
        sampled.watch(8, &[], 64, &signal_of);
        assert_eq!(sampled.signals, [signal(100)]);
        sampled.deliver(&shared, &[1], 12, 1);
        assert!(shared.lock().unwrap().waves.is_empty());
    }

    struct FreshSet(HashSet<SignalId>);

    impl Freshness for FreshSet {
        fn is_fresh(&self, signal: SignalId) -> bool {
            self.0.contains(&signal)
        }
    }

    #[test]
    fn watch_planner_rebuilds_only_when_stale_and_queues_during_build() {
        let s = |i| SignalId::for_tests(i);
        let default = FreshSet([s(0)].into());
        let mut planner = WatchPlanner::default();

        // Fresh or empty lists never trigger a build.
        assert!(planner.on_watch(&default, vec![]).is_empty());
        assert_eq!(planner.status(), None);
        assert!(planner.on_watch(&default, vec![s(0)]).is_empty());
        assert_eq!(planner.status(), None);

        // Watching a mix of fresh and stale starts a build for the stale ones only.
        assert_eq!(
            planner.on_watch(&default, vec![s(0), s(1), s(1)]),
            vec![s(1)]
        );
        assert_eq!(
            planner.status().as_deref(),
            Some("Building a program that keeps 1 watched net…")
        );

        // Adding another stale net mid-build does not start a second build yet,
        // and refuses switching to the first built program that lacks it.
        assert!(planner
            .on_watch(&default, vec![s(0), s(1), s(2)])
            .is_empty());
        let built1 = FreshSet([s(0), s(1)].into());
        assert_eq!(
            planner.on_build_ready(&default, &built1),
            BuildAction::Rebuild(vec![s(1), s(2)])
        );
        assert_eq!(
            planner.status().as_deref(),
            Some("Building a program that keeps 2 watched nets…")
        );

        // Adding a net that happens to be fresh in the built program still switches.
        let built2 = FreshSet([s(0), s(1), s(2), s(3)].into());
        assert!(planner
            .on_watch(&default, vec![s(0), s(1), s(2), s(3)])
            .is_empty());
        assert_eq!(
            planner.on_build_ready(&default, &built2),
            BuildAction::Switch
        );
        planner.on_switched();
        assert_eq!(
            planner.status().as_deref(),
            Some("Running a program that keeps 2 watched nets the default folds: less folding, its speed may differ")
        );

        // Removing nets does not rebuild and keeps the running status.
        assert!(planner.on_watch(&built2, vec![s(1)]).is_empty());
        assert!(planner.on_watch(&built2, vec![]).is_empty());
        assert!(planner
            .status()
            .as_deref()
            .is_some_and(|t| t.starts_with("Running a program that keeps 2 watched nets")));
    }

    #[test]
    fn watch_planner_handles_clear_mid_build_and_build_failure() {
        let s = |i| SignalId::for_tests(i);
        let default = FreshSet([s(0)].into());
        let mut planner = WatchPlanner::default();

        // Cleared while building from the default program: do not switch, clear status.
        assert_eq!(planner.on_watch(&default, vec![s(1)]), vec![s(1)]);
        assert!(planner.on_watch(&default, vec![]).is_empty());
        let built = FreshSet([s(0), s(1)].into());
        assert_eq!(planner.on_build_ready(&default, &built), BuildAction::None);
        assert_eq!(planner.status(), None);

        // Build failure records the error and stays until a later Watch retries.
        assert_eq!(planner.on_watch(&default, vec![s(2)]), vec![s(2)]);
        planner.on_build_failed("clang failed: exit 1\ndetail".into());
        assert_eq!(
            planner.status().as_deref(),
            Some("Watch rebuild failed: clang failed: exit 1")
        );
        assert_eq!(planner.on_watch(&default, vec![s(2)]), vec![s(2)]);
        assert!(planner
            .status()
            .as_deref()
            .is_some_and(|t| t.starts_with("Building a program")));
    }

    fn uart_tx_bits(text: &[u8]) -> Vec<u8> {
        let mut tx = uart::Transmitter::new(board::BAUD);
        for &b in text {
            tx.push(b);
        }
        // The TX level after each clock edge, from edge 1 to past the frames.
        let edges = text.len() as u64 * 10 * 1_000_000_000 / board::BAUD / HALF_NS + 200;
        let mut level = true;
        (1..=edges)
            .map(|e| {
                while let Some((_, l)) = tx.next(1000).filter(|&(ns, _)| ns <= e * HALF_NS) {
                    level = l;
                    tx.pop();
                }
                u8::from(level)
            })
            .collect()
    }

    /// The run's rows give the UART's TX in their first column, whatever
    /// follows it: rows with watched nets after it give the same bytes as
    /// rows of the TX alone.
    #[test]
    fn the_uart_is_read_from_the_first_column_of_the_rows() {
        let alone = uart_tx_bits(b"Hi!\r\n");
        // Two watched nets after it: one toggling every edge, one unknown.
        let wide: Vec<u8> = alone
            .iter()
            .enumerate()
            .flat_map(|(k, &v)| [v, (k % 2) as u8, 2])
            .collect();
        for (rows, width) in [(&alone, 1), (&wide, 3)] {
            // In two runs of edges, as the run loop takes them.
            let mut rx = uart::Receiver::new(board::BAUD);
            let half = rows.len() / width / 2;
            let mut bytes = uart_bytes(&mut rx, &rows[..half * width], width, 0);
            bytes.extend(uart_bytes(
                &mut rx,
                &rows[half * width..],
                width,
                half as u64,
            ));
            assert_eq!(bytes, b"Hi!\r\n", "rows {width} wide");
        }
    }

    /// Without a UART, watched nets fill column 0 and decode no console
    /// bytes even when their bits carry valid UART frames; with a UART,
    /// column 0 is its TX and decodes normally.
    #[test]
    fn without_a_uart_watched_columns_decode_no_bytes() {
        let bits = uart_tx_bits(b"Hi!\r\n");
        let rows: Vec<u8> = bits
            .iter()
            .enumerate()
            .flat_map(|(k, &v)| [v, (k % 2) as u8])
            .collect();
        let s = |index| SignalId::for_tests(index);
        let signal_of = [Some(s(0)), Some(s(1))];

        let mut no_uart = Sampled {
            signals: Vec::new(),
            uart: false,
            watching: None,
        };
        no_uart.watch(1, &[0, 1], 64, &signal_of);
        assert_eq!(no_uart.signals.len(), 2);
        let mut rx = uart::Receiver::new(board::BAUD);
        assert!(no_uart.uart_bytes(&mut rx, &rows, 0).is_empty());

        let mut with_uart = Sampled {
            signals: vec![s(100)],
            uart: true,
            watching: None,
        };
        with_uart.watch(1, &[0], 64, &signal_of);
        assert_eq!(with_uart.signals.len(), 2);
        let mut rx = uart::Receiver::new(board::BAUD);
        assert_eq!(with_uart.uart_bytes(&mut rx, &rows, 0), b"Hi!\r\n");
    }

    #[test]
    fn loading_keeps_commands_in_order_and_detects_disconnect() {
        let (send, receiver) = mpsc::channel();
        let commands = Commands {
            receiver: &receiver,
            pending: Default::default(),
        };
        send.send(Command::Rate(Some(2.0))).unwrap();
        commands.check().unwrap();
        send.send(Command::Run).unwrap();
        commands.check().unwrap();
        assert!(matches!(
            commands.next(false).unwrap(),
            Some(Command::Rate(Some(2.0)))
        ));
        assert!(matches!(commands.next(false).unwrap(), Some(Command::Run)));
        assert!(commands.next(false).unwrap().is_none());
        drop(send);
        assert!(commands.check().unwrap_err().is::<Cancelled>());
        assert!(commands.next(false).unwrap_err().is::<Cancelled>());
    }

    fn wait_for(shared: &Mutex<Shared>, done: impl Fn(&Shared) -> bool) {
        let start = Instant::now();
        loop {
            {
                let s = shared.lock().unwrap();
                assert!(s.error.is_none(), "{:?}", s.error);
                if done(&s) {
                    return;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(180),
                    "stage: {}",
                    s.stage
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct ClockPublication {
        published: u64,
        edges: u64,
        value: u8,
        toggled: bool,
    }

    impl ClockPublication {
        fn take(shared: &mut Shared, clock: usize) -> Self {
            let toggled = std::mem::take(&mut shared.toggled);
            Self {
                published: shared.published,
                edges: shared.edges,
                value: shared.lit[clock],
                toggled: toggled.get(clock) == Some(&1),
            }
        }

        fn reports_even_toggles_since(self, previous: Self) -> bool {
            self.published > previous.published
                && self.edges > previous.edges
                && (self.edges - previous.edges).is_multiple_of(2)
                && self.value < 2
                && self.value == previous.value
                && self.toggled
        }
    }

    #[test]
    fn even_count_clock_check_accepts_both_publication_phases() {
        for phase in [0, 1] {
            let previous = ClockPublication {
                published: 10,
                edges: 1024 + phase,
                value: phase as u8,
                toggled: false,
            };
            let next = ClockPublication {
                published: 11,
                edges: previous.edges + 1024,
                toggled: true,
                ..previous
            };
            assert!(next.reports_even_toggles_since(previous));
            assert!(!ClockPublication {
                toggled: false,
                ..next
            }
            .reports_even_toggles_since(previous));
            assert!(!ClockPublication {
                edges: next.edges + 1,
                ..next
            }
            .reports_even_toggles_since(previous));
            assert!(!ClockPublication {
                value: previous.value ^ 1,
                ..next
            }
            .reports_even_toggles_since(previous));
            assert!(!previous.reports_even_toggles_since(previous));
        }
    }

    #[test]
    #[ignore = "headless ADC regression needs the reference build, Trellis and clang"]
    fn responding_adc_reports_even_count_clock_toggles() {
        let root = crate::test_designs::reference_design("adc_read");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (repaint, frames) = mpsc::channel();
        let commands = spawn(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            Arc::clone(&shared),
            move || {
                let _ = repaint.send(());
            },
        );
        wait_for(&shared, |s| s.ready.is_some() && s.published > 0);
        let design = shared.lock().unwrap().ready.clone().unwrap();
        assert!(Attached::new(
            &board::used_balls(&design.device)
                .unwrap()
                .into_iter()
                .collect()
        )
        .observe_every_edge());
        let clock = design
            .overlay
            .value_of(
                &design.model,
                &Wire {
                    location: crate::model::topology::Location { row: 47, col: 0 },
                    name: "JPADDIA_PIO".into(),
                },
            )
            .unwrap() as usize;
        for rate in [Some(REAL_TIME), None] {
            // Pacing may leave a run on an odd edge. Once it is behind real
            // time, the 1024-edge answering batches preserve that phase.
            // Start there deliberately: max mode must never need an absolute
            // even edge to report an even number of toggles between samples.
            if shared.lock().unwrap().edges.is_multiple_of(2) {
                commands.send(Command::Rate(Some(4.0))).unwrap();
                commands.send(Command::Run).unwrap();
                wait_for(&shared, |s| {
                    s.running && s.rate == Some(4.0) && s.edges % 2 == 1
                });
                commands.send(Command::Pause).unwrap();
                wait_for(&shared, |s| !s.running);
            }
            let mut previous = {
                let mut s = shared.lock().unwrap();
                s.toggled.clear();
                ClockPublication::take(&mut s, clock)
            };
            assert_eq!(previous.edges % 2, 1);
            commands.send(Command::Rate(rate)).unwrap();
            commands.send(Command::Run).unwrap();
            let (mut publications, mut reports) = (0, 0);
            while publications < 32 && reports < 2 {
                // A dead-run watchdog, not a speed requirement: the assertion
                // below is bounded by publications, even on a slower machine.
                frames.recv_timeout(Duration::from_secs(30)).unwrap();
                let mut s = shared.lock().unwrap();
                assert!(s.error.is_none(), "{:?}", s.error);
                if !s.running || s.rate != rate || s.published == previous.published {
                    continue;
                }
                let current = ClockPublication::take(&mut s, clock);
                publications += current.published - previous.published;
                if rate.is_none() {
                    assert_eq!(current.edges % 2, 1, "max preserves the odd start");
                }
                if current.reports_even_toggles_since(previous) {
                    reports += 1;
                    eprintln!("rate {rate:?}: {previous:?} -> {current:?}");
                }
                previous = current;
            }
            assert_eq!(reports, 2,
                "rate {rate:?}: only {reports} even-count clock reports in {publications} publications; last {previous:?}");
            commands.send(Command::Pause).unwrap();
            wait_for(&shared, |s| !s.running);
        }
        drop(commands);
        exited(&shared);
    }

    /// No native window or GPU: the real runner publishes the actual overlay
    /// index and the timing notes, with the same readiness semantics as App.
    #[test]
    #[ignore = "release timing needs the local reference, Trellis and clang"]
    fn source_index_after_ready_timing() {
        let design = std::env::var("ECP5_PROBE_DESIGN").unwrap_or_else(|_| "life_full".into());
        let root = crate::test_designs::reference_design(&design);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let design_files = (design != "life_full").then(|| {
            sources::Files::project(root.clone(), ["top.v", "picorv32.v"].map(PathBuf::from))
        });
        let commands = spawn_with_sources(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            RunOptions::default(),
            design_files,
            Arc::clone(&shared),
            || {},
        );
        wait_for(&shared, |s| {
            s.ready.as_ref().is_some_and(|s| s.timing.get().is_some())
        });
        let s = shared.lock().unwrap().ready.clone().unwrap();
        let i = s.sources.get().unwrap();
        let seconds = *s.source_seconds.get().unwrap();
        println!("# probe load-pass {:.6}", i.collect_time.as_secs_f64());
        println!(
            "# probe index {seconds:.6} {} {}",
            i.nets.len(),
            i.locations
        );
        let timing = s.timing.get().unwrap();
        println!("# critical cost {:.6} {}", timing.seconds, timing.bytes);
        let paths = timing.result.as_ref().unwrap();
        let names = Names::load(&root.join("top_routed.json"), &s.model, board::PACKAGE).unwrap();
        for path in paths {
            println!("# {}", path.path.note());
            if path.path.domain.clock().is_some() {
                assert_eq!(path.label, "clk_25mhz");
            }
            for (n, hop) in path.hops.iter().enumerate() {
                println!("# {}", hop.note(n + 1));
                if let Some(net) = &hop.hop.net {
                    let mut expected =
                        names.overlay_nets(net, &s.model, board::PACKAGE, &s.overlay);
                    expected.sort_unstable();
                    expected.dedup();
                    assert_eq!(hop.nets, expected);
                    assert!(!hop.nets.is_empty() || hop.hop.tiles.is_some());
                }
                assert!(!hop.sites.is_empty());
                assert!(hop
                    .locations
                    .iter()
                    .all(|s| matches!(s.file.as_ref(), "top.v" | "picorv32.v")));
            }
        }
        assert!(paths
            .iter()
            .flat_map(|p| &p.hops)
            .any(|h| !h.nets.is_empty()));
        assert!(paths
            .iter()
            .flat_map(|p| &p.hops)
            .any(|h| !h.locations.is_empty()));
        if design == "blinky_cycle" {
            assert_eq!(i.resolve("top.v", 1, 1, "count").len(), 8);
        }
        if design == "switches" {
            assert!(!i.resolve("top.v", 1, 1, "sw").is_empty());
            assert!(!i.resolve("top.v", 1, 1, "led").is_empty());
        }
        let probe = match design.as_str() {
            "blinky_cycle" => Some(("top.v", "count")),
            "picorv32_console" => Some(("picorv32.v", "reg_pc")),
            _ => None,
        };
        if let Some((file, identifier)) = probe {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let (line, column) = text
                .lines()
                .enumerate()
                .find_map(|(line, text)| {
                    (text.contains("reg ") && text.contains(identifier))
                        .then(|| (line as u32 + 1, text.find(identifier).unwrap() as u32 + 1))
                })
                .unwrap();
            assert_eq!(sources::identifier_at(&text, line, column), identifier);
            let nets = i.resolve(file, line, column, identifier);
            assert!(!nets.is_empty());
            let span = i.net_locations(nets[0]).first().unwrap();
            assert!(i
                .resolve(&span.file, span.line, span.column.unwrap_or(1), "unmatched")
                .contains(&nets[0]));
            let summary = i.summary(&nets);
            println!(
                "# probe source {file}:{line}:{column} -> {} nets {}",
                nets.len(),
                summary.split_once(": ").unwrap().1
            );
            println!("# probe map {} -> {}", i.net_label(nets[0]), span.label());
        }
        assert!(i.nets.keys().all(|n| s.net_names.contains_key(n)));
        assert!(!i.nets.is_empty());
        drop(commands);
        exited(&shared);
        if design == "life_full" {
            assert!(
                seconds <= 0.3,
                "after-ready index {seconds:.6}s exceeds 0.3s"
            );
        }
    }

    /// The thread is the only other owner; releasing it proves that work
    /// and its running program have both finished, rather than just paused.
    fn exited(shared: &Arc<Mutex<Shared>>) {
        let start = Instant::now();
        while Arc::strong_count(shared) != 1 {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "the run thread is still alive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "requires the reference build, clang, the native image reader and a local Trellis database"]
    fn dropping_a_running_sender_exits_and_the_next_design_reuses_the_chip() {
        let design = |name: &str| {
            let root = crate::test_designs::reference_design(name);
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: true,
            }
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = spawn(design("blinky_cycle"), Arc::clone(&shared), || {});
        // This arrives before loading has completed and must survive it.
        commands.send(Command::Rate(Some(2.0))).unwrap();
        wait_for(&shared, |s| {
            s.running && s.cycles > s.earliest && s.rate == Some(2.0)
        });
        let first = shared.lock().unwrap().ready.clone().unwrap();
        assert!(shared.lock().unwrap().fast.is_some());
        commands.send(Command::Rate(None)).unwrap();
        wait_for(&shared, |s| s.running && s.rate.is_none());
        drop(commands);
        exited(&shared);

        let next = Arc::new(Mutex::new(Shared::default()));
        let commands = spawn(design("blinky_pll"), Arc::clone(&next), || {});
        wait_for(&next, |s| s.ready.is_some());
        let second = next.lock().unwrap().ready.clone().unwrap();
        assert!(Arc::ptr_eq(&first.model, &second.model));
        assert!(Arc::ptr_eq(&first.fabric, &second.fabric));
        drop(commands);
        exited(&next);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "requires the reference build, clang, the native image reader and a local Trellis database"]
    fn dropping_a_sender_during_loading_exits() {
        let reference = crate::test_designs::reference_design("blinky_cycle");
        let bytes = std::fs::read(reference.join("top.config")).unwrap();
        let root = std::env::temp_dir().join(format!(
            "ecp5_emu_drop_loading_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let config = root.join("top.config");
        assert!(std::process::Command::new("mkfifo")
            .arg(&config)
            .status()
            .unwrap()
            .success());
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = spawn(
            Design {
                config: config.clone(),
                names: None,
                run: true,
            },
            Arc::clone(&shared),
            || {},
        );
        // The FIFO holds this stage even when the model and fabric are cached.
        wait_for(&shared, |s| s.stage == "Reading the configuration");
        drop(commands);
        let writer = std::thread::spawn(move || std::fs::write(config, bytes).unwrap());
        exited(&shared);
        writer.join().unwrap();
        assert!(shared.lock().unwrap().error.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The window letting go while the first stage runs, and while the
    /// overlay and geometry build with fast mode's startup, stops all three.
    #[test]
    #[ignore = "requires the reference build, clang, the native image reader and a local Trellis database"]
    fn dropping_a_sender_at_loading_stages_exits() {
        // Isolate cache overrides: the delayed close must interrupt real
        // geometry/overlay work even after an earlier open warmed the caches.
        if std::env::var_os("ECP5_LOADING_CANCEL_TEST_CHILD").is_none() {
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "live::tests::dropping_a_sender_at_loading_stages_exits",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ECP5_LOADING_CANCEL_TEST_CHILD", "1")
                .env("ECP5_GEOMETRY_CACHE", "0")
                .env("ECP5_OVERLAY_CACHE", "0")
                .status()
                .unwrap()
                .success());
            return;
        }
        for (target, design, delay) in [
            ("Loading the device model", "blinky_cycle", false),
            (
                "Drawing the die's fabric (alongside layout geometry)",
                "blinky_cycle",
                false,
            ),
            (
                "Drawing the die's fabric (alongside layout geometry)",
                "picorv32_console",
                true,
            ),
        ] {
            let root = crate::test_designs::reference_design(design);
            let shared = Arc::new(Mutex::new(Shared::default()));
            let owner = Arc::new(Mutex::new(None::<mpsc::Sender<Command>>));
            let closing = Arc::clone(&owner);
            let view = Arc::clone(&shared);
            // Hold the slot until spawn returns. The delayed case lets the
            // geometry and overlay workers start before the window closes.
            let mut sender = owner.lock().unwrap();
            let commands = spawn(
                Design {
                    config: root.join("top.config"),
                    names: None,
                    run: false,
                },
                Arc::clone(&shared),
                move || {
                    let at_target = view.lock().unwrap().stage == target;
                    if at_target {
                        if delay {
                            let closing = Arc::clone(&closing);
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(20));
                                closing.lock().unwrap().take();
                            });
                        } else {
                            closing.lock().unwrap().take();
                        }
                    }
                },
            );
            *sender = Some(commands);
            drop(sender);
            wait_for(&shared, |_| owner.lock().unwrap().is_none());
            exited(&shared);
            let s = shared.lock().unwrap();
            assert!(s.error.is_none());
            assert!(s.ready.is_none(), "closed design reached ready");
        }
    }
}
