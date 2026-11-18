//! CLI adapter for the new kernel. Event-file times are integer nanoseconds.
use anyhow::{ensure, Context, Result};
use ecp5_emu::{
    board,
    compile::{self, Board, Reference, Request, Scope, Startup},
    fast::{self, FastSim},
    kernel::{self, DriverId, SignalId, StartsUnknown, Time},
    logic::Logic,
    names::Names,
    peripherals::Attached,
    prepared::ConfiguredDevice,
    uart,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufWriter, IsTerminal, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    time::Instant,
};

/// A status line: on stdout, or on stderr while a console prints to stdout.
macro_rules! say {
    ($console:expr, $($arg:tt)*) => {
        if $console {
            eprintln!($($arg)*)
        } else {
            println!($($arg)*)
        }
    };
}

pub struct Options {
    pub startup: Startup,
    pub package: String,
    pub scope: Scope,
    pub initial: Vec<(String, Logic)>,
    pub probes: Vec<String>,
    pub clock: Option<String>,
    /// Rising edges; none runs a console until Ctrl-C, or 16 without one.
    pub cycles: Option<u64>,
    pub stimulus: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub vcd: Option<PathBuf>,
    /// OSCG base frequency; None takes the nominal one.
    pub oscillator_hz: Option<u64>,
    /// nextpnr's routed JSON (`--write`), for `net:NAME` references.
    pub names: Option<PathBuf>,
    /// Run the plan compiled to C (src/fast.rs) instead of the event kernel.
    pub fast: bool,
    /// Attach the ULX3S peripherals, using the same pad path as the live runner.
    pub peripherals: bool,
    /// Clock period; the first rising edge is half a period after 0.
    pub period_ns: u64,
    pub console: Option<ConsoleOptions>,
}
/// A UART terminal on the design (src/uart.rs): the output it prints, the
/// input its keys drive, and the baud rate.
pub struct ConsoleOptions {
    pub output: String,
    pub input: Option<String>,
    pub baud: u64,
}
#[derive(Serialize)]
struct Observation {
    time_ns: u64,
    values: BTreeMap<String, Logic>,
}
/// Probe samples as VCD with a 1 ns timescale and the earlier `run` naming.
/// Values are written at the first sample and afterwards only when they change.
struct Vcd<W: Write> {
    out: W,
    last: Vec<Option<Logic>>,
}
impl<W: Write> Vcd<W> {
    fn new(mut out: W, names: &[&str]) -> Result<Self> {
        writeln!(
            out,
            "$version ecp5_emu $end\n$timescale 1ns $end\n$scope module ecp5 $end"
        )?;
        for (i, name) in names.iter().enumerate() {
            writeln!(out, "$var wire 1 s{i} \\{name} $end")?;
        }
        writeln!(out, "$upscope $end\n$enddefinitions $end")?;
        Ok(Self {
            out,
            last: vec![None; names.len()],
        })
    }
    fn sample(&mut self, time_ns: u64, values: impl IntoIterator<Item = Logic>) -> Result<()> {
        let mut stamped = false;
        for (i, value) in values.into_iter().enumerate() {
            let last = self
                .last
                .get_mut(i)
                .context("more VCD values than probes")?;
            if *last == Some(value) {
                continue;
            }
            if !stamped {
                writeln!(self.out, "#{time_ns}")?;
                stamped = true;
            }
            writeln!(self.out, "{value}s{i}")?;
            *last = Some(value);
        }
        Ok(())
    }
    fn finish(mut self) -> Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }
}
fn merge(map: &mut BTreeMap<Reference, Logic>, key: Reference, value: Logic) -> Result<()> {
    if let Some(old) = map.insert(key.clone(), value) {
        ensure!(
            old == value,
            "conflicting initial/same-time values for {}",
            key.label()
        );
    }
    Ok(())
}
pub fn run(device: Arc<ConfiguredDevice>, options: Options) -> Result<()> {
    ensure!(
        options.fast || !options.peripherals,
        "--peripherals needs --fast; the event kernel path does not attach board peripherals"
    );
    let names = options
        .names
        .as_deref()
        .map(|path| Names::load(path, device.model(), &options.package))
        .transpose()?;
    // net:NAME is an HDL name; a bus name expands to its bits.
    let expand = |text: &str| -> Result<Vec<(String, Reference)>> {
        if let Some(name) = text.strip_prefix("net:") {
            let names = names
                .as_ref()
                .context("net: references need --names with nextpnr's routed JSON")?;
            return names.expand(name);
        }
        Ok(vec![(text.into(), Reference::parse(device.model(), text)?)])
    };
    let parse = |text: &str| -> Result<Reference> {
        let mut references = expand(text)?;
        ensure!(
            references.len() == 1,
            "{text} names {} bits; give one bit",
            references.len()
        );
        Ok(references.remove(0).1)
    };
    let mut initial = BTreeMap::new();
    for (text, value) in options.initial {
        merge(&mut initial, parse(&text)?, value)?;
    }
    let console = options.console.is_some();
    let console_line = options
        .console
        .as_ref()
        .map(|c| parse(&c.output))
        .transpose()?;
    // The terminal's line idles high.
    let console_input = options
        .console
        .as_ref()
        .and_then(|c| c.input.as_deref())
        .map(parse)
        .transpose()?;
    if let Some(input) = &console_input {
        merge(&mut initial, input.clone(), Logic::One)?;
    }
    let events: Vec<super::Event> = if let Some(path) = options.stimulus {
        serde_json::from_slice(&std::fs::read(path)?)?
    } else {
        Vec::new()
    };
    let mut timeline: BTreeMap<u64, BTreeMap<Reference, Logic>> = BTreeMap::new();
    let mut future_inputs = BTreeSet::new();
    let clock = options.clock.as_deref().map(parse).transpose()?;
    for event in events {
        for (text, value) in event.set {
            let key = parse(&text)?;
            let value = value.parse()?;
            if event.time == 0 {
                merge(&mut initial, key, value)?;
            } else {
                ensure!(
                    clock.as_ref() != Some(&key),
                    "stimulus also drives --clock; select one source for that clock"
                );
                merge(timeline.entry(event.time).or_default(), key.clone(), value)?;
                future_inputs.insert(key);
            }
        }
    }
    if let Some(clock) = &clock {
        ensure!(
            console_input.as_ref() != Some(clock),
            "--console-in is also the --clock"
        );
        ensure!(
            initial.get(clock).is_none_or(|&v| v == Logic::Zero),
            "simple --clock requires initial 0"
        );
        initial.insert(clock.clone(), Logic::Zero);
    }
    // The --clock edges are generated while running, every half period from
    // the first rising edge, 2·cycles of them, and merged with the stimulus.
    ensure!(
        options.period_ns >= 2 && options.period_ns.is_multiple_of(2),
        "--period-ns must be even and at least 2"
    );
    let half_ns = options.period_ns / 2;
    let cycles = match options.cycles {
        Some(n) => Some(n),
        None if console => None,
        None => Some(16),
    };
    // Without a cycle count, a console runs until Ctrl-C: as many edges as
    // picoseconds can count.
    let clock_end = match (&clock, cycles) {
        (None, _) => 0,
        (Some(_), Some(n)) => n.checked_mul(2).context("cycle time overflow")?,
        (Some(_), None) => u64::MAX / 1000 / half_ns,
    };
    // Displayed name and reference of every probe, in the order given.
    let mut shown = Vec::new();
    for text in &options.probes {
        shown.extend(expand(text)?);
    }
    let mut probes = shown
        .iter()
        .map(|(_, r)| r.clone())
        .chain(console_line.clone())
        .collect::<Vec<_>>();
    let mut attached = if options.peripherals {
        let used = board::used_balls(&device)?.into_iter().collect();
        let attached = Attached::new(&used);
        for (ball, level) in attached.input_balls() {
            merge(
                &mut initial,
                Reference::Pin(ball.into()),
                level.map_or(Logic::Z, Logic::from_bool),
            )?;
        }
        probes.extend(
            attached
                .balls()
                .into_iter()
                .map(|ball| Reference::Pin(ball.into())),
        );
        Some(attached)
    } else {
        None
    };
    // Defaults fill only unspecified board inputs. In particular, a later
    // stimulus event does not make a released button start at Z.
    let switch_value = match std::env::var("ECP5_SWITCHES") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("ECP5_SWITCHES must be four 0/1 characters"),
    };
    let switches = board::parse_switches(switch_value.as_deref())?;
    let answering: BTreeSet<_> = attached
        .as_ref()
        .into_iter()
        .flat_map(Attached::input_balls)
        .map(|(ball, _)| ball)
        .collect();
    for (ball, level) in board::idle_inputs(&device, switches)? {
        if !answering.contains(ball) {
            initial
                .entry(Reference::Pin(ball.into()))
                .or_insert(Logic::from_bool(level));
        }
    }
    let explicit_header: BTreeSet<_> = initial.keys().cloned().collect();
    let header = board::header_from_env()?;
    for (ball, level) in board::header_inputs(&device, &header)? {
        if !answering.contains(ball) {
            initial
                .entry(Reference::Pin(ball.into()))
                .or_insert(level.map_or(Logic::Z, Logic::from_bool));
        }
    }
    if let Some(attached) = &mut attached {
        for (index, pin) in board::HEADER.iter().enumerate() {
            if let Some(value) = initial.get(&Reference::Pin(pin.ball.into())) {
                attached.set_header_level(
                    index,
                    if header[index].is_some()
                        || explicit_header.contains(&Reference::Pin(pin.ball.into()))
                    {
                        value.bit()
                    } else {
                        None
                    },
                );
            }
        }
    }
    for key in future_inputs {
        initial.entry(key).or_insert(Logic::Z);
    }
    let mut compiled = compile::compile_on_board(
        device,
        Request {
            package: options.package,
            scope: options.scope,
            startup: options.startup,
            drives: initial.into_iter().collect(),
            probes,
            options: kernel::Options {
                trace_capacity: if options.output.is_some() && !options.fast {
                    65_536
                } else {
                    0
                },
                ..kernel::Options::default()
            },
        },
        &Board {
            oscillator_hz: options.oscillator_hz,
            ..Board::default()
        },
    )?;
    say!(
        console,
        "New event kernel; scope: {:?}; startup: {:?}",
        compiled.report.scope,
        compiled.report.startup
    );
    say!(
        console,
        "Executed FF: {}; LUT4: {}; LUTRAM columns: {}; RAMW: {}; carry halves: {}; PIO: {}; routes: {}",
        compiled.report.counts.flip_flops,
        compiled.report.counts.lut4,
        compiled.report.counts.lutram_columns,
        compiled.report.counts.ramw,
        compiled.report.counts.carry_halves,
        compiled.report.counts.pio,
        compiled.report.counts.routes
    );
    say!(
        console,
        "Undriven wires: {}; inactive test-access descriptions: {}",
        compiled.report.undriven.len(),
        compiled.report.excluded_routes.len()
    );
    let unmodelled_ramw_di: usize = compiled
        .report
        .lutram
        .iter()
        .map(|r| r.unmodelled_c_ff_di.len())
        .sum();
    if unmodelled_ramw_di > 0 {
        say!(
            console,
            "Unmodelled RAMW C FF data paths: {unmodelled_ramw_di} (X)"
        );
    }
    let driver = |compiled: &compile::Compiled, key: &Reference| {
        compiled
            .drivers
            .get(&key.label())
            .copied()
            .context("missing boundary driver")
    };
    if options.fast {
        ensure!(
            options.output.is_none(),
            "--fast keeps no event trace for --output; use --vcd"
        );
        let clock = clock.as_ref().context("--fast needs --clock")?;
        let clock = driver(&compiled, clock)?;
        let mut stimulus = Vec::new();
        for (ns, set) in timeline {
            let mut inputs = Vec::new();
            for (key, value) in set {
                let bit = value.bit().with_context(|| {
                    format!(
                        "fast mode is two-valued; {} is {value} at {ns} ns",
                        key.label()
                    )
                })?;
                inputs.push((driver(&compiled, &key)?, bit));
            }
            stimulus.push((ns, inputs));
        }
        let console = match (options.console, console_line) {
            (Some(c), Some(line)) => Some(ConsoleWires {
                line: compiled.probes[&line.label()],
                input: console_input
                    .as_ref()
                    .map(|i| driver(&compiled, i))
                    .transpose()?,
                baud: c.baud,
            }),
            _ => None,
        };
        return run_fast(
            &mut compiled,
            &shown,
            clock,
            half_ns,
            clock_end,
            stimulus,
            options.vcd,
            console,
            attached,
        );
    }
    ensure!(
        !console,
        "--console-out needs --fast; the event kernel runs no console"
    );
    // One sample line per event time; buffered, since a run prints millions.
    let mut out = BufWriter::new(std::io::stdout().lock());
    let observe =
        |compiled: &compile::Compiled, time_ns, out: &mut dyn Write| -> Result<Observation> {
            let values = shown
                .iter()
                .map(|(name, reference)| {
                    let signal = compiled.probes[&reference.label()];
                    Ok((name.clone(), compiled.kernel.value(signal)?))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            write!(out, "{time_ns} ns:")?;
            for (name, value) in &values {
                write!(out, " {name}={value}")?;
            }
            writeln!(out)?;
            Ok(Observation { time_ns, values })
        };
    let mut observations = Vec::new();
    let first = observe(&compiled, 0, &mut out)?;
    // Header order follows the observation map, so samples line up with it.
    let mut vcd = options
        .vcd
        .map(|path| -> Result<_> {
            let names: Vec<&str> = first.values.keys().map(String::as_str).collect();
            let mut vcd = Vcd::new(BufWriter::new(std::fs::File::create(path)?), &names)?;
            vcd.sample(0, first.values.values().copied())?;
            Ok(vcd)
        })
        .transpose()?;
    observations.push(first);
    let clock_driver = clock.as_ref().map(|c| driver(&compiled, c)).transpose()?;
    let mut next_clock = 1_u64;
    let mut stimulus = timeline.into_iter().peekable();
    let mut updates = Vec::new();
    loop {
        let clock_time = (next_clock <= clock_end).then_some(next_clock * half_ns);
        let stimulus_time = stimulus.peek().map(|&(t, _)| t);
        let ns = match (clock_time, stimulus_time) {
            (None, None) => break,
            (Some(c), Some(s)) => c.min(s),
            (c, s) => c.or(s).unwrap(),
        };
        let time = Time::from_ps(
            ns.checked_mul(1000)
                .context("nanosecond timestamp exceeds time range")?,
        );
        updates.clear();
        if stimulus_time == Some(ns) {
            for (key, value) in stimulus.next().unwrap().1 {
                updates.push((driver(&compiled, &key)?, value));
            }
        }
        if let (Some(clock), Some(_)) = (clock_driver, clock_time.filter(|&c| c == ns)) {
            let level = if next_clock % 2 == 1 {
                Logic::One
            } else {
                Logic::Zero
            };
            updates.push((clock, level));
            next_clock += 1;
        }
        compiled.kernel.schedule_batch(time, &updates, None)?;
        compiled.kernel.run_until(time)?;
        let observation = observe(&compiled, ns, &mut out)?;
        if let Some(vcd) = &mut vcd {
            vcd.sample(ns, observation.values.values().copied())?;
        }
        if options.output.is_some() {
            observations.push(observation);
        }
    }
    out.flush()?;
    drop(out);
    if let Some(vcd) = vcd {
        vcd.finish()?;
    }
    if let Some(path) = options.output {
        let trace = compiled.kernel.drain_trace();
        let mut file = BufWriter::new(std::fs::File::create(path)?);
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({"schema_version":1,"plan":compiled.report,
            "image":compiled.kernel.device().image().summary(),"state":compiled.kernel.state().summary(),
            "statistics":compiled.kernel.statistics(),"ebr_notices":compiled.kernel.ebr_notices().collect::<Vec<_>>(),"observations":observations,"trace":trace}),
        )?;
        writeln!(file)?;
        file.flush()?;
    }
    Ok(())
}

/// Clock edges a fast run takes at once between VCD writes.
const FAST_CHUNK: u64 = 1 << 20;
/// Runs of at least this many edges tune fast mode (`FastSim::tune`).
const TUNE_EDGES: u64 = 4_000_000;
/// Clock edges between two looks at the terminal: a few ms at 2 MHz.
const CONSOLE_CHUNK: u64 = 1 << 14;
/// Ctrl-C, which ends a console run when stdin is a terminal.
const INTERRUPT: u8 = 3;

/// The design's side of a console: the output line it decodes and the input
/// its keys drive.
struct ConsoleWires {
    line: SignalId,
    input: Option<DriverId>,
    baud: u64,
}

/// Keys from stdin, read by a thread. A terminal is put in raw mode with
/// stty (no echo, no line editing, Enter as CR, Ctrl-C as a byte) and
/// restored on drop.
struct Keys {
    bytes: mpsc::Receiver<u8>,
    /// The terminal's settings before, as `stty -g` prints them.
    saved: Option<String>,
}
impl Keys {
    fn open() -> Result<Self> {
        let saved = if std::io::stdin().is_terminal() {
            let output = Command::new("stty")
                .arg("-g")
                .stdin(Stdio::inherit())
                .output()
                .context("running stty for the console")?;
            ensure!(output.status.success(), "stty -g failed");
            let saved = String::from_utf8(output.stdout)?.trim().to_string();
            let raw = Command::new("stty")
                .args([
                    "-icanon", "-echo", "-isig", "-iexten", "-icrnl", "-ixon", "min", "1", "time",
                    "0",
                ])
                .status()?;
            ensure!(raw.success(), "stty could not put the terminal in raw mode");
            Some(saved)
        } else {
            None
        };
        let (send, bytes) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 64];
            let mut stdin = std::io::stdin().lock();
            while let Ok(n @ 1..) = stdin.read(&mut buffer) {
                if buffer[..n].iter().any(|&b| send.send(b).is_err()) {
                    break;
                }
            }
        });
        Ok(Self { bytes, saved })
    }
}
impl Drop for Keys {
    fn drop(&mut self) {
        if let Some(saved) = &self.saved {
            let _ = Command::new("stty").arg(saved).status();
        }
    }
}

/// A UART terminal on the design while fast mode runs: what the design sends
/// goes to stdout, the keys go to its input.
struct Console {
    wires: ConsoleWires,
    receiver: uart::Receiver,
    transmitter: uart::Transmitter,
    keys: Keys,
    text: Vec<u8>,
    quit: bool,
}
impl Console {
    fn new(wires: ConsoleWires) -> Result<Self> {
        Ok(Self {
            receiver: uart::Receiver::new(wires.baud),
            transmitter: uart::Transmitter::new(wires.baud),
            wires,
            keys: Keys::open()?,
            text: Vec::new(),
            quit: false,
        })
    }
    fn sample(&mut self, ns: u64, level: bool) {
        self.text.extend(self.receiver.sample(ns, level));
    }
    /// Prints what arrived and queues the keys typed since the last look.
    fn poll(&mut self) -> Result<()> {
        if !self.text.is_empty() {
            let mut out = std::io::stdout().lock();
            out.write_all(&self.text)?;
            out.flush()?;
            self.text.clear();
        }
        while let Ok(byte) = self.keys.bytes.try_recv() {
            if byte == INTERRUPT && self.keys.saved.is_some() {
                self.quit = true;
                break;
            }
            if self.wires.input.is_some() {
                self.transmitter.push(byte);
            }
        }
        Ok(())
    }
}

/// The same trace and 1 ps answer path as the live runner. Peripheral
/// outputs are external drivers, so released bidirectional pads stay Z.
struct FastPeripherals {
    attached: Attached,
    signals: Vec<SignalId>,
    inputs: Vec<DriverId>,
    updates: Vec<(usize, Option<bool>)>,
    header_inputs: Vec<(DriverId, usize)>,
}
impl FastPeripherals {
    fn new(attached: Attached, compiled: &compile::Compiled) -> Result<Self> {
        let signals = attached
            .balls()
            .iter()
            .map(|ball| {
                compiled
                    .probes
                    .get(&format!("pin:{ball}"))
                    .copied()
                    .with_context(|| format!("no peripheral probe for pin:{ball}"))
            })
            .collect::<Result<_>>()?;
        let inputs = attached
            .input_balls()
            .iter()
            .map(|(ball, _)| {
                compiled
                    .drivers
                    .get(&format!("pin:{ball}"))
                    .copied()
                    .with_context(|| format!("no peripheral driver for pin:{ball}"))
            })
            .collect::<Result<_>>()?;
        let header_inputs = board::HEADER
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                compiled
                    .drivers
                    .get(&format!("pin:{}", p.ball))
                    .map(|&d| (d, i))
            })
            .collect();
        Ok(Self {
            header_inputs,
            attached,
            signals,
            inputs,
            updates: Vec::new(),
        })
    }
    fn feed(&mut self, fast: &mut FastSim) -> Result<()> {
        self.attached.feed(&fast.take_trace());
        self.updates.clear();
        self.attached.respond_into(&mut self.updates);
        if !self.updates.is_empty() {
            let inputs: Vec<_> = self
                .updates
                .iter()
                .map(|&(i, value)| (self.inputs[i], value))
                .collect();
            let at = fast.now()?.checked_add(Time::from_ps(1))?;
            fast.set_at_levels(at, &inputs)?;
            self.attached.feed(&fast.take_trace());
        }
        Ok(())
    }
    fn report(&mut self) {
        for line in self.attached.report() {
            eprintln!("{line}");
        }
    }
}

/// Fast mode: the plan compiled to C. Edge k of the clock comes at k·half
/// ns (odd k rising), `edges` of them; each stimulus is set in one instant
/// with a clock edge at the same time, as in the event kernel. A console's
/// keys are stimulus that arrives while the design runs.
///
/// While the kernel's state has an unknown register or latch that fast mode
/// would read (an EBR read latch after configuration, say), the event kernel
/// runs whole cycles; fast mode takes over from its state once everything is
/// known. Prints the values at 0 ns and after the last edge; the VCD gets
/// every probe after every clock edge and stimulus.
#[allow(clippy::too_many_arguments)]
fn run_fast(
    compiled: &mut compile::Compiled,
    shown: &[(String, Reference)],
    clock: DriverId,
    half_ns: u64,
    edges: u64,
    stimulus: Vec<(u64, Vec<(DriverId, bool)>)>,
    vcd: Option<PathBuf>,
    console: Option<ConsoleWires>,
    attached: Option<Attached>,
) -> Result<()> {
    let mut peripherals = attached
        .filter(Attached::any)
        .map(|attached| FastPeripherals::new(attached, compiled))
        .transpose()?;
    let tracing = peripherals.is_some();
    let quiet = console.is_some();
    // Probes in name order, as the event kernel's observations.
    let mut order: Vec<usize> = (0..shown.len()).collect();
    order.sort_by(|&a, &b| shown[a].0.cmp(&shown[b].0));
    order.dedup_by(|a, b| shown[*a].0 == shown[*b].0);
    let names: Vec<&str> = order.iter().map(|&i| shown[i].0.as_str()).collect();
    let signals: Vec<SignalId> = order
        .iter()
        .map(|&i| compiled.probes[&shown[i].1.label()])
        .collect();
    let in_kernel = |c: &compile::Compiled| -> Result<Vec<Logic>> {
        signals.iter().map(|&s| c.kernel.value(s)).collect()
    };
    let read = |fast: &FastSim| -> Vec<Logic> {
        signals
            .iter()
            .map(|&s| fast.value(s).unwrap_or(Logic::X))
            .collect()
    };
    let line = |ns: u64, values: &[Logic]| {
        // A console run without probes has nothing to report here.
        if quiet && names.is_empty() {
            return;
        }
        let mut text = format!("{ns} ns:");
        for (name, value) in names.iter().zip(values) {
            text.push_str(&format!(" {name}={value}"));
        }
        say!(quiet, "{text}");
    };
    let mut vcd = vcd
        .map(|path| -> Result<_> { Vcd::new(BufWriter::new(std::fs::File::create(path)?), &names) })
        .transpose()?;
    let mut console = console.map(Console::new).transpose()?;
    let first = in_kernel(compiled)?;
    line(0, &first);
    if let Some(vcd) = &mut vcd {
        vcd.sample(0, first)?;
    }
    let mut stimulus = stimulus.into_iter().peekable();
    let end_of_clock = edges * half_ns;
    if let Some(&(ns, _)) = stimulus.peek() {
        ensure!(ns >= 1, "stimulus at 0 ns belongs to the initial values");
    }
    if let Some((ns, _)) = stimulus.clone().last() {
        ensure!(
            ns <= end_of_clock,
            "--fast applies stimulus only while the clock runs; {ns} ns is after its last edge"
        );
    }
    // Event kernel cycles until fast mode can start, trying after 1, 2, 4, …
    // cycles.
    let mut done = 0_u64;
    let (mut next_try, mut wait) = (0_u64, 2_u64);
    let (mut fast, built) = loop {
        if done >= next_try || done >= edges {
            let built = Instant::now();
            match FastSim::start(&compiled.kernel) {
                Ok(fast) => break (fast, built.elapsed()),
                Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && done < edges => {
                    next_try = done + wait;
                    wait *= 2;
                }
                Err(e) if e.downcast_ref::<StartsUnknown>().is_some() => {
                    say!(
                        quiet,
                        "Fast mode did not start: {e:#}; the event kernel ran all {} cycles",
                        edges / 2
                    );
                    line(end_of_clock, &in_kernel(compiled)?);
                    if let Some(vcd) = vcd {
                        vcd.finish()?;
                    }
                    if let Some(peripherals) = &mut peripherals {
                        peripherals.report();
                    }
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        for _ in 0..2.min(edges - done) {
            done += 1;
            let t = done * half_ns;
            let mut batches: Vec<(u64, Vec<(DriverId, Logic)>)> = Vec::new();
            while let Some((ns, _)) = stimulus.peek().filter(|(ns, _)| *ns <= t) {
                let ns = *ns;
                let (_, inputs) = stimulus.next().unwrap();
                batches.push((
                    ns,
                    inputs
                        .into_iter()
                        .map(|(d, v)| (d, Logic::from_bool(v)))
                        .collect(),
                ));
            }
            let level = Logic::from_bool(done % 2 == 1);
            match batches.last_mut() {
                Some((ns, inputs)) if *ns == t => inputs.push((clock, level)),
                _ => batches.push((t, vec![(clock, level)])),
            }
            for (ns, updates) in batches {
                let time = Time::from_ps(ns * 1000);
                compiled.kernel.schedule_batch(time, &updates, None)?;
                compiled.kernel.run_until(time)?;
                if let Some(vcd) = &mut vcd {
                    vcd.sample(ns, in_kernel(compiled)?)?;
                }
                if let Some(console) = &mut console {
                    // An unknown line keeps its last level.
                    if let Some(level) = compiled.kernel.value(console.wires.line)?.bit() {
                        console.sample(ns, level);
                    }
                }
            }
        }
    };
    let p = fast.program();
    if done > 0 {
        say!(
            quiet,
            "Event kernel ran the first {} cycles, until every register and latch fast mode reads was known",
            done / 2
        );
    }
    say!(
        quiet,
        "Fast mode: {} gates ({} on demand), {} FF, {} LUTRAM columns, {} IOLOGIC, {} EBR, {} PLL, {} OSC; compiled in {:.2} s",
        p.gates,
        p.demanded_gates,
        p.registers,
        p.lutrams.len(),
        p.iologic,
        p.ebrs.len(),
        p.plls.len(),
        p.oscillators.len(),
        built.as_secs_f64()
    );
    // The VCD probes, then the console's line.
    let mut sampled = signals.clone();
    sampled.extend(console.as_ref().map(|c| c.wires.line));
    let choice_memory = fast::choice_memory(&compiled.kernel);
    if !tracing
        && edges - done >= TUNE_EDGES
        && !std::env::var("FAST_CHOOSE").is_ok_and(|v| v.trim() == "0")
    {
        let choosing = Instant::now();
        let progress = |message: String| say!(quiet, "{message}");
        // A failed choice is no reason to stop the run: the defaults run.
        match fast::choose_options(
            &compiled.kernel,
            fast,
            clock,
            Time::from_ps(2 * half_ns * 1000),
            &sampled,
            &progress,
            &|| false,
        ) {
            Ok(chosen) => {
                fast = chosen;
                let best = fast.program().options;
                say!(
                    quiet,
                    "Fast mode choice: {} thread{}, folding {}, chosen in {:.2} s",
                    best.threads,
                    if best.threads == 1 { "" } else { "s" },
                    best.collapse,
                    choosing.elapsed().as_secs_f64()
                );
            }
            Err(e) => {
                say!(quiet, "Fast mode choice failed, the defaults run: {e:#}");
                fast = FastSim::with_options(&compiled.kernel, &kernel::FastOptions::from_env())?;
            }
        }
    }
    fast.set_choice_memory(choice_memory);
    fast.clock(clock, Time::from_ps(2 * half_ns * 1000))?;
    if let Some(peripherals) = &mut peripherals {
        fast.trace(&peripherals.signals)?;
        peripherals.feed(&mut fast)?;
    }
    let tuning_details = std::env::var_os("FAST_THREAD_REPORT").is_some();
    let tuning_enabled = !std::env::var("FAST_TUNE").is_ok_and(|v| v.trim() == "0");
    // A long run measures programs with other options on this design and
    // this machine as it goes, and runs the fastest (FastSim::tune).
    if !tracing && edges - done >= TUNE_EDGES {
        let tuning = Instant::now();
        fast.tune(&compiled.kernel)?;
        if tuning_details && fast.tuning_report().is_some() {
            say!(
                quiet,
                "Fast mode tuning: started in {:.2} s; measuring as the design runs",
                tuning.elapsed().as_secs_f64()
            );
        }
    }
    let started = Instant::now();
    let warm = done;
    let width = signals.len();
    // Runs from clock edge `done` to edge `to`.
    let run = |fast: &mut FastSim,
               vcd: &mut Option<Vcd<_>>,
               console: &mut Option<Console>,
               done: &mut u64,
               to: u64,
               peripherals: &mut Option<FastPeripherals>|
     -> Result<()> {
        while *done < to {
            let count = (to - *done).min(FAST_CHUNK);
            if let Some(peripherals) = peripherals {
                let count = if peripherals.attached.observe_every_edge() {
                    // As in live.rs: answer each edge inside a bounded batch.
                    count.min(1024)
                } else {
                    count.min(peripherals.attached.horizon().unwrap_or(FAST_CHUNK))
                };
                let mut sample = |ps: u64, row: &[u8]| -> Result<()> {
                    if let Some(vcd) = vcd {
                        vcd.sample(
                            ps / 1000,
                            row[..width].iter().map(|&b| match b {
                                0 => Logic::Zero,
                                1 => Logic::One,
                                2 => Logic::X,
                                _ => Logic::Z,
                            }),
                        )?;
                    }
                    if let Some(console) = console {
                        console.sample(ps / 1000, row[width] == 1);
                    }
                    Ok(())
                };
                if peripherals.attached.observe_every_edge() {
                    fast.run_edges_with(
                        count,
                        &sampled,
                        &peripherals.inputs,
                        |row, trace, updates| {
                            peripherals.attached.feed(trace);
                            if let Some((ps, row)) = row {
                                sample(ps, row)?;
                                peripherals.attached.respond_into(updates);
                            }
                            Ok(true)
                        },
                    )?;
                } else {
                    let rows = fast.run_edges_sampled(count, &sampled)?;
                    if !sampled.is_empty() {
                        for (k, row) in rows.chunks(sampled.len()).enumerate() {
                            sample((*done + k as u64 + 1) * half_ns * 1000, row)?;
                        }
                    }
                    peripherals.feed(fast)?;
                }
                *done += count;
                continue;
            }
            if vcd.is_none() && console.is_none() {
                fast.run_edges_sampled(count, &[])?;
            } else {
                let rows = fast.run_edges_sampled(count, &sampled)?;
                for (k, row) in rows.chunks(sampled.len()).enumerate() {
                    let ns = (*done + k as u64 + 1) * half_ns;
                    if let Some(vcd) = vcd {
                        // 2 is X; 3 is Z, on a tristate pad that only
                        // probes read (kernel/fast.rs).
                        let values = row[..width].iter().map(|&b| match b {
                            0 => Logic::Zero,
                            1 => Logic::One,
                            2 => Logic::X,
                            _ => Logic::Z,
                        });
                        vcd.sample(ns, values)?;
                    }
                    if let Some(console) = console {
                        console.sample(ns, row[width] == 1);
                    }
                }
            }
            *done += count;
        }
        Ok(())
    };
    let mut now_ns = done * half_ns;
    loop {
        if let Some(console) = &mut console {
            console.poll()?;
            if console.quit {
                break;
            }
        }
        if done >= edges {
            break;
        }
        // The next instant with an input change: from the stimulus file, or a
        // bit of a key being sent.
        let file = stimulus.peek().map(|&(ns, _)| ns);
        let typed = console
            .as_mut()
            .and_then(|c| c.transmitter.next(now_ns + 1))
            .map(|(ns, _)| ns)
            .filter(|&ns| ns <= end_of_clock);
        let next = file.into_iter().chain(typed).min();
        // Every edge before it, then the change with an edge at the same time.
        let before = next.map_or(edges, |ns| (ns - 1) / half_ns);
        let to = match console {
            Some(_) => before.min(done + CONSOLE_CHUNK),
            None => before,
        };
        run(
            &mut fast,
            &mut vcd,
            &mut console,
            &mut done,
            to,
            &mut peripherals,
        )?;
        now_ns = now_ns.max(done * half_ns);
        let Some(ns) = next.filter(|_| done == before) else {
            continue;
        };
        let mut inputs = Vec::new();
        if file == Some(ns) {
            inputs.extend(stimulus.next().unwrap().1);
        }
        if typed == Some(ns) {
            let console = console.as_mut().unwrap();
            let (_, level) = console.transmitter.pop().unwrap();
            inputs.push((console.wires.input.unwrap(), level));
        }
        fast.set_at(Time::from_ps(ns * 1000), &inputs)?;
        if let Some(p) = &mut peripherals {
            for &(driver, level) in &inputs {
                if let Some(&(_, index)) = p.header_inputs.iter().find(|d| d.0 == driver) {
                    p.attached.set_header_level(index, Some(level));
                }
            }
        }
        if let Some(peripherals) = &mut peripherals {
            peripherals.feed(&mut fast)?;
        }
        if ns.is_multiple_of(half_ns) {
            done = ns / half_ns;
        }
        now_ns = ns;
        if let Some(vcd) = &mut vcd {
            vcd.sample(ns, read(&fast))?;
        }
        if let Some(console) = &mut console {
            if let Some(level) = fast.value(console.wires.line).and_then(Logic::bit) {
                console.sample(ns, level);
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if let Some(console) = &mut console {
        console.poll()?;
    }
    // Restores the terminal before the closing lines.
    drop(console);
    if quiet {
        eprintln!();
    }
    line(now_ns, &read(&fast));
    if let Some(vcd) = vcd {
        vcd.finish()?;
    }
    let cycles = (done - warm) / 2;
    say!(
        quiet,
        "Fast mode ran {cycles} cycles ({:.6} s of board time) in {seconds:.3} s: {:.2} MHz",
        ((done - warm) * half_ns) as f64 * 1e-9,
        cycles as f64 / seconds / 1e6
    );
    if let Some(summary) = fast.tuning_summary() {
        let hint = if tuning_details {
            ""
        } else {
            "; details: FAST_THREAD_REPORT=1"
        };
        say!(quiet, "Fast mode tuning: {summary}{hint}");
        if tuning_details {
            if let Some(report) = fast.tuning_report() {
                say!(quiet, "Fast mode tuning: {report}");
            }
        }
    } else if !tracing && tuning_enabled {
        say!(quiet, "Fast mode tuning: skipped (run too short)");
    }
    if let Some(peripherals) = &mut peripherals {
        peripherals.report();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcd_has_header_initial_values_and_only_changes() {
        let mut vcd = Vcd::new(Vec::new(), &["pin:F1", "sys:ready"]).unwrap();
        vcd.sample(0, [Logic::X, Logic::Zero]).unwrap();
        vcd.sample(1, [Logic::One, Logic::Zero]).unwrap();
        vcd.sample(2, [Logic::One, Logic::Zero]).unwrap();
        vcd.sample(3, [Logic::Z, Logic::One]).unwrap();
        let text = String::from_utf8(vcd.finish().unwrap()).unwrap();
        assert_eq!(
            text,
            "$version ecp5_emu $end\n$timescale 1ns $end\n$scope module ecp5 $end\n\
             $var wire 1 s0 \\pin:F1 $end\n$var wire 1 s1 \\sys:ready $end\n\
             $upscope $end\n$enddefinitions $end\n\
             #0\nxs0\n0s1\n#1\n1s0\n#3\nzs0\n1s1\n"
        );
    }

    #[test]
    fn vcd_rejects_more_values_than_probes() {
        let mut vcd = Vcd::new(Vec::new(), &["pin:F1"]).unwrap();
        assert!(vcd.sample(0, [Logic::One, Logic::Zero]).is_err());
    }
}
