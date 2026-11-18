//! Fast mode host: compiles a kernel's `FastProgram` with clang into a shared
//! library, loads it in process and runs it on a byte array of net values.
//!
//! The host keeps time, in whole ticks of a unit that divides every period it
//! knows, from the kernel's time when the program is built. A clock started
//! with `clock`, the OSC outputs and the PLL outputs are periodic edge
//! trains; at each instant with edges the program sets their slots together,
//! settles and captures. A PLL learns its reference period from the first two
//! rising edges and lays out its output edges from the second, as the event
//! kernel does (src/kernel/pll.rs); what the kernel's PLL has already learnt
//! carries over. The event kernel stays the reference; fast mode is checked
//! against it.
use crate::{
    kernel::{DriverId, FastOptions, FastProgram, Kernel, SignalId, Time, DTR_CONVERSION_PS},
    logic::Logic,
};
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void, CStr, CString, OsStr, OsString},
    hash::{Hash, Hasher},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

extern "C" {
    #[cfg(target_os = "macos")]
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: c_int = 2;

type Step = unsafe extern "C" fn(*mut u8) -> c_int;
type Run = unsafe extern "C" fn(*mut u8, u32, u64, *const u32, u32, u32, *mut u8) -> u64;
type Point = unsafe extern "C" fn(*mut u8, *const u32, *const u8, u32) -> c_int;
type Error = unsafe extern "C" fn(*const u8) -> u32;
type Schedule = unsafe extern "C" fn(
    *mut u8,
    *const u32,
    *const u32,
    *const u8,
    *const u8,
    u32,
    u64,
    *const u32,
    u32,
    *mut u8,
) -> u64;

/// Runs shorter than this many cycles stay in the host loop.
const SCHEDULE_CYCLES: u64 = 4096;
/// Hyperperiods with more instants than this stay in the host loop.
const SCHEDULE_INSTANTS: u128 = 1 << 20;

/// One hyperperiod of instants from now, for `fast_schedule`.
struct Table {
    /// Ticks from now to each instant.
    offsets: Vec<u64>,
    first: Vec<u32>,
    slots: Vec<u32>,
    values: Vec<u8>,
    sample: Vec<u8>,
    period: u64,
}

/// Edges on one slot: the next one, at `next` ticks, sets `value`, and the
/// value toggles every `half` ticks after it.
#[derive(Clone, Copy)]
struct Wave {
    slot: usize,
    next: u64,
    half: u64,
    value: u8,
}

impl Wave {
    /// Moves to the first edge after `now`.
    fn pass(&mut self, now: u64) {
        if self.next <= now {
            let k = (now - self.next) / self.half + 1;
            self.next += k * self.half;
            self.value ^= (k & 1) as u8;
        }
    }
}

/// What a PLL has learnt about its reference, in ticks.
#[derive(Clone)]
struct PllState {
    previous: u8,
    last_rise: Option<u64>,
    period: Option<u64>,
}

#[derive(Clone)]
struct DtrState {
    previous: u8,
    completion: Option<u64>,
}

/// Everything that changes as fast mode runs: the values of every slot (the
/// registers, the memories, LUTRAM), the time, the clock's and the PLLs'
/// edges. Restoring it goes back to where it was taken: fast mode is a
/// function of it and the inputs.
#[derive(Clone)]
pub struct Snapshot {
    values: Vec<u8>,
    /// The layout of `values` (`FastProgram::place` of the program it was
    /// taken with): a tuned simulation may have changed program since.
    place: Arc<[usize]>,
    tick: Time,
    now: u64,
    waves: Vec<Wave>,
    plls: Vec<PllState>,
    dtrs: Vec<DtrState>,
}

impl Snapshot {
    /// Its size in bytes, near enough.
    pub fn bytes(&self) -> usize {
        self.values.len() + self.waves.len() * std::mem::size_of::<Wave>()
    }
}

/// Pad values after every executed instant. Columns follow `trace`'s signals;
/// times are picoseconds, rounded down when a tick is fractional.
#[derive(Default, Debug)]
pub struct Trace {
    pub width: usize,
    pub times_ps: Vec<u64>,
    pub rows: Vec<u8>,
    pub gap: bool,
}

/// A list of signals read many times (`FastSim::read`): their slots in the
/// running program, kept with the layout they were found in.
pub struct Reader {
    signals: Vec<Option<SignalId>>,
    place: Option<Arc<[usize]>>,
    /// Slot and inversion of each signal; `u32::MAX` for one the program
    /// does not have.
    slots: Vec<(u32, u8)>,
}

impl Reader {
    pub fn new(signals: &[Option<SignalId>]) -> Self {
        Self {
            signals: signals.to_vec(),
            place: None,
            slots: Vec::new(),
        }
    }
}

/// A program compiled and loaded: its functions and the library they are in.
struct Loaded {
    program: FastProgram,
    place: Arc<[usize]>,
    library: *mut c_void,
    step: Step,
    resume: Option<Step>,
    run: Run,
    point: Point,
    schedule: Schedule,
    error: Error,
}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: the handle came from dlopen and is closed once; fast_stop
        // (every program has it) joins the settle threads first, so none runs
        // in the library when it is unloaded.
        unsafe {
            if let Ok(name) = CString::new("fast_stop") {
                let stop = dlsym(self.library, name.as_ptr());
                if !stop.is_null() {
                    let stop: extern "C" fn() = std::mem::transmute(stop);
                    stop();
                }
            }
            dlclose(self.library);
        }
    }
}

/// The clang flags: `FAST_CFLAGS`, or native optimisation with AVX-512 opt-in.
fn compile_flags() -> Vec<String> {
    std::env::var("FAST_CFLAGS")
        .unwrap_or_else(|_| {
            let mut flags = String::from("-O3 -march=native");
            // Clang has miscompiled LUT shifts with AVX-512; keep it opt-in on x86.
            if cfg!(target_arch = "x86_64") && !std::env::var("FAST_AVX512").is_ok_and(|v| v == "1")
            {
                flags.push_str(" -mno-avx512f");
            }
            flags
        })
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// One executable path or command name; compilers needing arguments use a wrapper.
pub(crate) fn compiler() -> OsString {
    std::env::var_os("ECP5_CLANG")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "clang".into())
}

/// Validate the compiler without changing or reading the process environment.
fn check_compiler(value: Option<&OsStr>, path: Option<&OsStr>) -> Result<OsString> {
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let on_path = |name: &OsStr| {
        path.is_some_and(|path| std::env::split_paths(path).any(|dir| executable(&dir.join(name))))
    };
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        ensure!(
            if value.as_encoded_bytes().contains(&b'/') {
                executable(Path::new(value))
            } else {
                on_path(value)
            },
            "fast mode: ECP5_CLANG={} is not an executable file",
            value.to_string_lossy()
        );
        return Ok(value.to_owned());
    }
    ensure!(
        on_path(OsStr::new("clang")),
        "fast mode needs a C compiler: install clang on PATH or set ECP5_CLANG to one"
    );
    Ok("clang".into())
}

/// Cache the whole version output once per compiler value, including failures.
fn compiler_identity_with(
    compiler: &OsStr,
    identities: &Mutex<HashMap<OsString, String>>,
    run: impl FnOnce(&OsStr) -> Option<String>,
) -> String {
    // Hold the lock during the probe so concurrent tuning builds cannot probe twice.
    identities
        .lock()
        .unwrap()
        .entry(compiler.to_owned())
        .or_insert_with(|| run(compiler).unwrap_or_default())
        .clone()
}

fn compiler_identity(compiler: &OsStr) -> String {
    static IDENTITIES: OnceLock<Mutex<HashMap<OsString, String>>> = OnceLock::new();
    compiler_identity_with(
        compiler,
        IDENTITIES.get_or_init(|| Mutex::new(HashMap::new())),
        |compiler| {
            let output = Command::new(compiler).arg("--version").output().ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        },
    )
}

/// The C file and the library of `source` in the cache, by the hash of the
/// source, flags, compiler identity and ECP5_CLANG when non-empty, and that hash.
fn cache_paths(source: &str, flags: &[String]) -> Result<(PathBuf, PathBuf, String)> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    flags.hash(&mut hasher);
    compiler_identity(&compiler()).hash(&mut hasher);
    if let Some(value) = std::env::var_os("ECP5_CLANG").filter(|value| !value.is_empty()) {
        value.hash(&mut hasher);
    }
    let dir = std::env::temp_dir().join("ecp5_emu_fast");
    std::fs::create_dir_all(&dir)?;
    let name = format!("{:016x}", hasher.finish());
    Ok((
        dir.join(format!("{name}.c")),
        dir.join(format!("{name}.so")),
        name,
    ))
}

/// Compiles `source` into the cache unless it is there; `background` runs
/// clang at a low priority (utility QoS on macOS) so that it leaves the cores
/// to the simulation. Returns the library.
fn compile(source: &str, flags: &[String], background: bool) -> Result<PathBuf> {
    let (c, library, _) = cache_paths(source, flags)?;
    if library.exists() {
        return Ok(library);
    }
    check_compiler(
        std::env::var_os("ECP5_CLANG").as_deref(),
        std::env::var_os("PATH").as_deref(),
    )?;
    // Written under names of their own and moved into place, so a library
    // another thread or process is building is never loaded half written.
    static BUILDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = format!(
        "{}-{}",
        std::process::id(),
        BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let stem = c.with_extension("");
    let (c_tmp, library_tmp) = (
        stem.with_extension(format!("{unique}.c")),
        stem.with_extension(format!("{unique}.so")),
    );
    std::fs::write(&c_tmp, source)?;
    // A large program comes in units (`split_functions`): each compiles on
    // its own core, then they are linked.
    let units = units_of(source);
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let jobs = if background { 2 } else { cores }.min(units).max(1);
    // The library moves into place in the same command, so a compile that
    // outlives this process (a short run that started tuning) still ends up
    // in the cache for the next one.
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(concat!(
            r#"out="$1"; tmp="$2"; c="$3"; ctmp="$4"; units="$5"; jobs="$6"; shift 6; "#,
            r#"if [ "$units" -le 1 ]; then "$@" -shared -fPIC "$ctmp" -o "$tmp"; s=$?; "#,
            r#"else seq 0 $((units - 1)) | xargs -P "$jobs" -I{} "$@" -fPIC -DTU={} -c "$ctmp" -o "$tmp.{}.o"; s=$?; "#,
            r#"[ $s -eq 0 ] && { "$@" -shared "$tmp".*.o -o "$tmp"; s=$?; }; rm -f "$tmp".*.o; fi; "#,
            r#"mv -f "$ctmp" "$c"; [ $s -eq 0 ] && mv -f "$tmp" "$out"; exit $s"#
        ))
        .arg("sh")
        .arg(&library)
        .arg(&library_tmp)
        .arg(&c)
        .arg(&c_tmp)
        .arg(units.to_string())
        .arg(jobs.to_string());
    if background && cfg!(target_os = "macos") && Path::new("/usr/sbin/taskpolicy").exists() {
        command.args(["/usr/sbin/taskpolicy", "-c", "utility"]);
    }
    let output = command
        .arg(compiler())
        .args(flags)
        .output()
        .context("running clang for fast mode")?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&library_tmp);
        bail!("clang failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(library)
}

/// The units a program's source comes in: the count `split_functions` writes
/// after the prelude, else one. The prelude has no fixed length (a program
/// with multipliers has `mul18` in it), so the count is looked for up to the
/// first unit, not in a fixed number of lines: with 16, a large program with
/// multipliers compiled as one unit and its pieces were missing at the link.
fn units_of(source: &str) -> usize {
    source
        .lines()
        .take_while(|l| !l.starts_with("#if TU == "))
        .find_map(|l| {
            l.strip_prefix("/* ecp5_emu units: ")?
                .strip_suffix(" */")?
                .parse()
                .ok()
        })
        .unwrap_or(1)
}

/// Loads a compiled program.
fn open(program: FastProgram, library: &Path) -> Result<Loaded> {
    let path = CString::new(library.to_string_lossy().as_bytes())?;
    // SAFETY: dlopen/dlsym are called with valid C strings; the symbols come
    // from the C source generated for `program`, with the signatures above.
    unsafe {
        let handle = dlopen(path.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            bail!("dlopen: {}", CStr::from_ptr(dlerror()).to_string_lossy());
        }
        let symbol = |name: &str| -> Result<*mut c_void> {
            let name = CString::new(name)?;
            let pointer = dlsym(handle, name.as_ptr());
            ensure!(!pointer.is_null(), "missing symbol {name:?}");
            Ok(pointer)
        };
        let loaded = (|| -> Result<Loaded> {
            Ok(Loaded {
                place: program.place.clone().into(),
                step: std::mem::transmute::<*mut c_void, Step>(symbol("fast_step")?),
                resume: if program.dtrs.is_empty() {
                    None
                } else {
                    Some(std::mem::transmute::<*mut c_void, Step>(symbol(
                        "fast_resume",
                    )?))
                },
                run: std::mem::transmute::<*mut c_void, Run>(symbol("fast_run")?),
                point: std::mem::transmute::<*mut c_void, Point>(symbol("fast_point")?),
                schedule: std::mem::transmute::<*mut c_void, Schedule>(symbol("fast_schedule")?),
                error: std::mem::transmute::<*mut c_void, Error>(symbol("fast_error")?),
                program,
                library: handle,
            })
        })();
        if loaded.is_err() {
            dlclose(handle);
        }
        loaded
    }
}

pub struct FastSim {
    loaded: Loaded,
    values: Vec<u8>,
    /// Time unit and the current time in it.
    tick: Time,
    now: u64,
    waves: Vec<Wave>,
    /// The wave of the clock started with `clock`.
    clock: Option<usize>,
    plls: Vec<PllState>,
    dtrs: Vec<DtrState>,
    /// Slots and values of one instant.
    slots: Vec<u32>,
    levels: Vec<u8>,
    /// Programs with other options being measured on this design (`tune`).
    tuning: Option<Box<Tuning>>,
    /// The default program's startup choice, shared with the in-run tuner.
    choice_memory: Option<PathBuf>,
    trace_signals: Vec<SignalId>,
    trace_place: Option<Arc<[usize]>>,
    trace_slots: Vec<(u32, u8)>,
    trace_buffer: Trace,
}

/// A fast program generated from a kernel, ready to compile on another thread
/// (`compile`) and open into a simulation (`FastSim::from_prepared`).
pub struct Prepared {
    program: FastProgram,
    flags: Vec<String>,
    start: Time,
}

impl Prepared {
    /// The generated program.
    pub fn program(&self) -> &FastProgram {
        &self.program
    }
    /// Compiles the program into the cache (if not already there); `background`
    /// runs clang at a low priority.
    pub fn compile(&self, background: bool) -> Result<PathBuf> {
        compile(&self.program.source, &self.flags, background)
    }
}

impl FastSim {
    /// Runs clock edges with a peripheral answering after each one. Slots
    /// and buffers are reused for the whole run; tuning must be off.
    ///
    /// The callback gets the instant trace and, on clock edges, the time in
    /// picoseconds and a row of probes. It fills `updates` with input indices
    /// and levels to deliver 1 ps later, and returns whether to keep running.
    /// After delivery it is called with no probe row to feed the answer's
    /// trace too. That call must leave `updates` empty. Traces are consumed,
    /// including any already buffered when this method starts.
    pub fn run_edges_with(
        &mut self,
        edges: u64,
        probes: &[SignalId],
        inputs: &[DriverId],
        mut callback: impl FnMut(
            Option<(u64, &[u8])>,
            &Trace,
            &mut Vec<(usize, Option<bool>)>,
        ) -> Result<bool>,
    ) -> Result<u64> {
        ensure!(self.tuning.is_none(), "answering runs cannot tune");
        ensure!(self.clock.is_some(), "start a clock first");
        if edges == 0 {
            return Ok(0);
        }
        self.refine(Time::from_ps(1))?;
        let delay = self.ticks(Time::from_ps(1))?;
        let slots = probes
            .iter()
            .map(|&p| {
                self.loaded
                    .program
                    .slot(p)
                    .context("probe is not in the fast program")
            })
            .collect::<Result<Vec<_>>>()?;
        let drivers = inputs
            .iter()
            .map(|&d| {
                let (slot, _) = self.input_level(d, Some(false))?;
                Ok((slot as u32, self.loaded.program.allows_z(slot)))
            })
            .collect::<Result<Vec<_>>>()?;
        let inverted = slots.iter().any(|&(_, inv)| inv);
        let mut row = vec![0; probes.len()];
        let mut updates = Vec::with_capacity(inputs.len());
        let mut extra = Vec::with_capacity(inputs.len());
        // The program also exports the single-input settle used by its
        // whole-cycle loop. With only the board clock it avoids dispatching
        // a one-element input batch on every edge.
        let clock_step = if self.waves.len() == 1 && self.plls.is_empty() && self.dtrs.is_empty() {
            type StepInput = unsafe extern "C" fn(*mut u8, u32) -> c_int;
            // SAFETY: every loaded program exports this function with this
            // signature, and the library stays loaded throughout the run.
            let symbol = unsafe { dlsym(self.loaded.library, c"fast_step_input".as_ptr()) };
            ensure!(!symbol.is_null(), "fast_step_input is missing");
            Some(unsafe { std::mem::transmute::<*mut c_void, StepInput>(symbol) })
        } else {
            None
        };
        for done in 1..=edges {
            if let Some(step) = clock_step {
                let clock = &mut self.waves[0];
                self.now = clock.next;
                self.values[clock.slot] = clock.value;
                clock.value ^= 1;
                clock.next += clock.half;
                // SAFETY: the clock slot belongs to this program and values
                // contains all its slots.
                if unsafe { step(self.values.as_mut_ptr(), clock.slot as u32) } != 0 {
                    return Err(self.failure());
                }
                if !self.trace_signals.is_empty() {
                    self.trace_layout()?;
                    self.trace_buffer.times_ps.push(self.trace_ps(self.now));
                    for &(slot, inv) in &self.trace_slots {
                        self.trace_buffer
                            .rows
                            .push(trace_value(self.values[slot as usize], inv));
                    }
                }
            } else {
                while self
                    .next_point(u64::MAX)?
                    .context("no edges left")?
                    .is_none()
                {}
            }
            if inverted {
                for (v, &(slot, inv)) in row.iter_mut().zip(&slots) {
                    *v = trace_value(self.values[slot], u8::from(inv));
                }
            } else {
                for (v, &(slot, _)) in row.iter_mut().zip(&slots) {
                    *v = self.values[slot];
                }
            }
            let keep_running = callback(
                Some((self.trace_ps(self.now), &row)),
                &self.trace_buffer,
                &mut updates,
            )?;
            self.trace_buffer.times_ps.clear();
            self.trace_buffer.rows.clear();
            self.trace_buffer.gap = false;
            if !updates.is_empty() {
                extra.clear();
                for &(i, value) in &updates {
                    let &(slot, allows_z) = drivers.get(i).context("no such answering input")?;
                    let value = match value {
                        Some(v) => u8::from(v),
                        None => {
                            ensure!(allows_z, "this input cannot be set to Z");
                            3
                        }
                    };
                    extra.push((slot, value));
                }
                updates.clear();
                let at = self
                    .now
                    .checked_add(delay)
                    .context("fast mode time overflows")?;
                while self.next_point(at - 1)?.is_some() {}
                self.instant(at, &extra)?;
                callback(None, &self.trace_buffer, &mut updates)?;
                self.trace_buffer.times_ps.clear();
                self.trace_buffer.rows.clear();
                self.trace_buffer.gap = false;
                ensure!(updates.is_empty(), "an answer's trace cannot answer again");
            }
            if !keep_running {
                return Ok(done);
            }
        }
        Ok(edges)
    }
}

impl FastSim {
    /// Builds the fast program of `kernel` from its current state and time, so
    /// it can take over from a kernel that has already run, with the options
    /// of the environment. Never reads the choice memory. Needs clang on
    /// PATH or ECP5_CLANG (a wrapper for `zig cc`); the library is cached.
    pub fn new(kernel: &Kernel) -> Result<Self> {
        Self::with_options(kernel, &FastOptions::from_env())
    }
    /// Warm start before `choose_options`: use a remembered choice when
    /// present, otherwise guess from the kernel's gate count without a
    /// preliminary default-program generation. Pinned options stay pinned.
    /// `FAST_CHOOSE=0` or `full` starts with defaults. Needs clang on PATH or
    /// ECP5_CLANG (a wrapper for `zig cc`); the library is cached.
    pub fn start(kernel: &Kernel) -> Result<Self> {
        let base = FastOptions::from_env();
        if std::env::var("FAST_CHOOSE").is_ok_and(|v| matches!(v.trim(), "0" | "full")) {
            return Self::with_options(kernel, &base);
        }
        if let Some(&(options, _)) = remembered_choices(&choice_memory(kernel), base).first() {
            match Self::with_options(kernel, &options) {
                Ok(sim) => return Ok(sim),
                Err(e) if e.downcast_ref::<crate::kernel::StartsUnknown>().is_some() => {
                    return Err(e)
                }
                Err(e) => {
                    eprintln!(
                        "fast mode: remembered start failed, the size-based guess runs: {e:#}"
                    )
                }
            }
        }
        let guess = starting_options(
            kernel.fast_gate_count(),
            base,
            FastOptions::pinned()[0],
            performance_cores(),
        );
        Self::with_options(kernel, &guess)
    }
    /// Generates a fast program from `kernel` with `options` without running
    /// clang, so compilation can run on another thread (`Prepared::compile`)
    /// before opening (`FastSim::from_prepared`).
    pub fn prepare(kernel: &Kernel, options: &FastOptions) -> Result<Prepared> {
        let program = kernel.fast_program_with(options)?;
        let flags = compile_flags();
        if std::env::var_os("FAST_THREAD_REPORT").is_some() {
            eprintln!(
                "fast mode: program {}",
                cache_paths(&program.source, &flags)?.2
            );
        }
        Ok(Prepared {
            program,
            flags,
            start: kernel.now(),
        })
    }
    /// Builds a simulation from a prepared program and its compiled `library`.
    pub fn from_prepared(prepared: Prepared, library: &Path) -> Result<Self> {
        let loaded = open(prepared.program, library)?;
        let program = &loaded.program;
        let plls = program
            .plls
            .iter()
            .map(|p| PllState {
                previous: program.initial[p.reference.0] ^ u8::from(p.reference.1),
                last_rise: None,
                period: None,
            })
            .collect();
        let values = program.initial.clone();
        let dtrs = program
            .dtrs
            .iter()
            .map(|d| DtrState {
                previous: d.previous,
                completion: None,
            })
            .collect();
        let start = prepared.start;
        let mut sim = Self {
            values,
            loaded,
            tick: Time::from_ps(1),
            now: 0,
            waves: Vec::new(),
            clock: None,
            plls,
            dtrs,
            slots: Vec::new(),
            levels: Vec::new(),
            tuning: None,
            choice_memory: None,
            trace_signals: Vec::new(),
            trace_place: None,
            trace_slots: Vec::new(),
            trace_buffer: Trace::default(),
        };
        sim.refine(start)?;
        sim.now = sim.ticks(start)?;
        for i in 0..sim.dtrs.len() {
            if let Some(time) = sim.loaded.program.dtrs[i].completion {
                sim.refine(time)?;
                sim.dtrs[i].completion = Some(sim.ticks(time)?);
            }
        }
        // Oscillators are low at time 0 and rise after half a period.
        for o in 0..sim.loaded.program.oscillators.len() {
            let half = sim.loaded.program.oscillators[o].half_period;
            sim.refine(half)?;
            let half = sim.ticks(half)?;
            let mut wave = Wave {
                slot: sim.loaded.program.oscillators[o].slot,
                next: half,
                half,
                value: 1,
            };
            wave.pass(sim.now);
            sim.waves.push(wave);
        }
        // What the kernel's PLLs have learnt.
        for p in 0..sim.loaded.program.plls.len() {
            let (last_rise, learnt, b) = {
                let pll = &sim.loaded.program.plls[p];
                (pll.last_rise, pll.learnt, pll.b)
            };
            if let Some(t) = last_rise {
                sim.refine(t)?;
                sim.plls[p].last_rise = Some(sim.ticks(t)?);
            }
            if let Some((period, anchor)) = learnt {
                let (n, d) = period.ratio();
                sim.refine(Time::new(n, 16 * b * d)?)?;
                sim.refine(anchor)?;
                let (period, anchor) = (sim.ticks(period)?, sim.ticks(anchor)?);
                sim.plls[p].period = Some(period);
                sim.start_outputs(p, anchor, period)?;
            }
        }
        Ok(sim)
    }
    /// As `new`, with `options`.
    pub fn with_options(kernel: &Kernel, options: &FastOptions) -> Result<Self> {
        let prepared = Self::prepare(kernel, options)?;
        let library = prepared.compile(false)?;
        Self::from_prepared(prepared, &library)
    }
    /// The current time.
    pub fn now(&self) -> Result<Time> {
        let (n, d) = self.tick.ratio();
        Time::new(
            n.checked_mul(self.now)
                .context("fast mode time overflows")?,
            d,
        )
    }
    /// `time` in ticks; it must be a whole number of them.
    fn ticks(&self, time: Time) -> Result<u64> {
        let ((n, d), (tn, td)) = (time.ratio(), self.tick.ratio());
        let (num, den) = (
            u128::from(n) * u128::from(td),
            u128::from(d) * u128::from(tn),
        );
        ensure!(
            num % den == 0,
            "{time} is not a whole number of fast-mode ticks"
        );
        u64::try_from(num / den).context("fast mode time overflows")
    }
    /// Makes the tick divide `time`, rescaling every time kept in ticks.
    fn refine(&mut self, time: Time) -> Result<()> {
        let ((n1, d1), (n2, d2)) = (self.tick.ratio(), time.ratio());
        let (d1, d2) = (u128::from(d1), u128::from(d2));
        let lcm = d1 / gcd(d1, d2) * d2;
        let g = gcd(u128::from(n1) * (lcm / d1), u128::from(n2) * (lcm / d2));
        let tick = Time::new(
            u64::try_from(g).context("fast mode tick overflows")?,
            u64::try_from(lcm).context("fast mode tick overflows")?,
        )?;
        if tick == self.tick {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.tick, tick);
        let factor = self.ticks(old)?;
        let scale = |t: &mut u64| -> Result<()> {
            *t = t.checked_mul(factor).context("fast mode time overflows")?;
            Ok(())
        };
        scale(&mut self.now)?;
        for wave in &mut self.waves {
            scale(&mut wave.next)?;
            scale(&mut wave.half)?;
        }
        for pll in &mut self.plls {
            for t in pll.last_rise.iter_mut().chain(pll.period.iter_mut()) {
                scale(t)?;
            }
        }
        for dtr in &mut self.dtrs {
            if let Some(time) = &mut dtr.completion {
                scale(time)?;
            }
        }
        Ok(())
    }
    /// Starts a clock on the input `driver` now: low for half a period, then
    /// high for half a period, as the board oscillator is driven in tests.
    pub fn clock(&mut self, driver: DriverId, period: Time) -> Result<()> {
        ensure!(self.clock.is_none(), "fast mode runs one clock");
        let slot = self
            .loaded
            .program
            .input_slot(driver)
            .context("clock is not an input of the fast program")?;
        let (n, d) = period.ratio();
        let half = Time::new(n, 2 * d)?;
        self.refine(half)?;
        // A PLL on this clock steps in 1/16 b of its period.
        let steps: Vec<u64> = self
            .loaded
            .program
            .plls
            .iter()
            .filter(|p| p.reference.0 == slot)
            .map(|p| p.b)
            .collect();
        for b in steps {
            self.refine(Time::new(n, 16 * b * d)?)?;
        }
        if self.values[slot] != 0 {
            self.values[slot] = 0;
            self.step()?;
        }
        let half = self.ticks(half)?;
        self.clock = Some(self.waves.len());
        self.waves.push(Wave {
            slot,
            next: self.now + half,
            half,
            value: 1,
        });
        Ok(())
    }
    /// Runs the next instant with edges if it is not after `until`; returns
    /// whether there was one and the clock's new level if it had an edge.
    fn next_point(&mut self, until: u64) -> Result<Option<Option<u8>>> {
        match self
            .waves
            .iter()
            .map(|w| w.next)
            .chain(self.dtrs.iter().filter_map(|d| d.completion))
            .min()
        {
            Some(t) if t <= until => self.instant(t, &[]).map(Some),
            _ => Ok(None),
        }
    }
    /// The edges due at `t` and the `extra` input values, set together;
    /// returns the clock's new level if it had an edge.
    fn instant(&mut self, t: u64, extra: &[(u32, u8)]) -> Result<Option<u8>> {
        self.advance_dcs_time(t);
        self.slots.clear();
        self.levels.clear();
        for &(slot, value) in extra {
            self.slots.push(slot);
            self.levels.push(value);
        }
        for (dtr, state) in self.loaded.program.dtrs.iter().zip(&mut self.dtrs) {
            if state.completion == Some(t) {
                state.completion = None;
                for (bit, &slot) in dtr.outputs.iter().enumerate() {
                    self.slots.push(slot as u32);
                    self.levels.push((dtr.code >> bit) & 1);
                }
            }
        }
        let mut edge = None;
        for (i, wave) in self.waves.iter_mut().enumerate() {
            if wave.next == t {
                self.slots.push(wave.slot as u32);
                self.levels.push(wave.value);
                if Some(i) == self.clock {
                    edge = Some(wave.value);
                }
                wave.value ^= 1;
                wave.next += wave.half;
            }
        }
        self.now = t;
        // SAFETY: the slots are below `program.slots`, and both arrays have
        // `slots.len()` entries.
        let status = unsafe {
            (self.loaded.point)(
                self.values.as_mut_ptr(),
                self.slots.as_ptr(),
                self.levels.as_ptr(),
                self.slots.len() as u32,
            )
        };
        self.finish_dtrs(status)?;
        self.track_plls()?;
        if !self.trace_signals.is_empty() {
            self.trace_layout()?;
            self.trace_buffer.times_ps.push(self.trace_ps(self.now));
            for &(slot, inv) in &self.trace_slots {
                self.trace_buffer
                    .rows
                    .push(trace_value(self.values[slot as usize], inv));
            }
        }
        Ok(edge)
    }
    /// Handshake stages may consume at most one edge at a physical timestamp,
    /// including across DTR pauses and multiple set_at/step calls. Generated
    /// run/schedule loops clear these same slots at each new instant.
    fn advance_dcs_time(&mut self, t: u64) {
        if t > self.now {
            for &slot in &self.loaded.program.dcs_blocked {
                self.values[slot] = 0;
            }
        }
    }
    fn input_level(&self, driver: DriverId, value: Option<bool>) -> Result<(usize, u8)> {
        let slot = self
            .loaded
            .program
            .input_slot(driver)
            .context("not an input of the fast program")?;
        ensure!(
            self.waves.iter().all(|w| w.slot != slot),
            "the clock drives this input"
        );
        let level = match value {
            Some(b) => u8::from(b),
            None => {
                ensure!(
                    self.loaded.program.allows_z(slot),
                    "fast mode is two-valued; this input cannot be set to Z"
                );
                3
            }
        };
        Ok((slot, level))
    }
    /// Sets inputs at `time`, in one instant with any edge due then, after
    /// every edge before it.
    pub fn set_at(&mut self, time: Time, inputs: &[(DriverId, bool)]) -> Result<()> {
        self.set_at_levels_iter(time, inputs.iter().map(|&(d, v)| (d, Some(v))))
    }
    /// As `set_at`, with `None` for Z on a tristate pad's external driver.
    pub fn set_at_levels(&mut self, time: Time, inputs: &[(DriverId, Option<bool>)]) -> Result<()> {
        self.set_at_levels_iter(time, inputs.iter().copied())
    }
    fn set_at_levels_iter(
        &mut self,
        time: Time,
        inputs: impl IntoIterator<Item = (DriverId, Option<bool>)>,
    ) -> Result<()> {
        self.refine(time)?;
        let t = self.ticks(time)?;
        ensure!(t >= self.now, "cannot set inputs in the past");
        let mut extra = Vec::new();
        for (driver, value) in inputs {
            let (slot, level) = self.input_level(driver, value)?;
            extra.push((slot as u32, level));
        }
        if t > self.now {
            while self.next_point(t - 1)?.is_some() {}
        }
        self.instant(t, &extra)?;
        Ok(())
    }
    /// Reference edges of the PLLs at the current time; the second rising edge
    /// starts the output edges, and a later edge off the period faults.
    fn track_plls(&mut self) -> Result<()> {
        for p in 0..self.plls.len() {
            let pll = &self.loaded.program.plls[p];
            let level = self.values[pll.reference.0] ^ u8::from(pll.reference.1);
            let state = &mut self.plls[p];
            let rising = state.previous == 0 && level == 1;
            state.previous = level;
            if !rising {
                continue;
            }
            let now = self.now;
            match (state.last_rise, state.period) {
                (None, _) => {}
                (Some(last), None) => {
                    let period = now - last;
                    state.period = Some(period);
                    self.start_outputs(p, now, period)?;
                }
                (Some(last), Some(period)) => ensure!(
                    now == last + period,
                    "{} reference period changed; relock is not modelled",
                    pll.label
                ),
            }
            self.plls[p].last_rise = Some(now);
        }
        Ok(())
    }
    /// The output edge trains of PLL `p`, laid out from the reference edge at
    /// `anchor` and continued past now.
    fn start_outputs(&mut self, p: usize, anchor: u64, period: u64) -> Result<()> {
        let pll = &self.loaded.program.plls[p];
        ensure!(
            period.is_multiple_of(16 * pll.b),
            "{} reference period is not a whole number of fast-mode steps",
            pll.label
        );
        let step = period / (16 * pll.b);
        for &(slot, divider, phase) in &pll.outputs {
            // As schedule_pll_window: in steps from the anchor, a rising edge
            // at the phase offset, a falling edge half an output period later,
            // both after the anchor.
            let cycle = 16 * pll.a * divider;
            let offset = (2 * pll.a as i64 * phase).rem_euclid(cycle as i64) as u64;
            let rise = if offset > 0 { offset } else { cycle };
            let fall = offset + cycle / 2;
            let (first, value) = if rise < fall { (rise, 1) } else { (fall, 0) };
            let mut wave = Wave {
                slot,
                next: anchor + first * step,
                half: cycle / 2 * step,
                value,
            };
            wave.pass(self.now);
            self.waves.push(wave);
        }
        Ok(())
    }
    /// Runs every edge up to and including `time`.
    pub fn run_until(&mut self, time: Time) -> Result<()> {
        self.refine(time)?;
        let until = self.ticks(time)?;
        ensure!(until >= self.now, "cannot run backwards");
        while self.now < until {
            let to = match self.tuning.is_some() {
                true => self.now + self.tune_ticks(until - self.now)?,
                false => until,
            };
            let (started, from) = (Instant::now(), self.now_ps());
            while self.next_point(to)?.is_some() {}
            self.advance_dcs_time(to);
            self.now = to;
            if self.tuning.is_some() {
                self.tune_ran(self.now_ps() - from, started.elapsed())?;
            }
        }
        Ok(())
    }
    /// Runs until the clock has fallen `cycles` times and returns the value of
    /// `probe` after each falling edge.
    pub fn run_cycles(&mut self, cycles: u64, probe: SignalId) -> Result<Vec<u8>> {
        self.run_cycles_sampled(cycles, &[probe])
    }
    /// As `run_cycles` with several probes: one row of `probes.len()` values
    /// per falling edge.
    pub fn run_cycles_sampled(&mut self, cycles: u64, probes: &[SignalId]) -> Result<Vec<u8>> {
        self.sampled(cycles, probes, false, false)
    }
    /// Runs `edges` clock edges, rising or falling, with a row of probe
    /// values after each.
    pub fn run_edges_sampled(&mut self, edges: u64, probes: &[SignalId]) -> Result<Vec<u8>> {
        self.sampled(edges, probes, true, false)
    }
    /// As `run_edges_sampled`, for signals that need not be probes: where
    /// the program that ran an edge does not keep a signal up to date
    /// (`FastProgram::fresh`), its value after that edge is `NOT_SAMPLED`.
    /// Tuning may change program between pieces of the run, and with it
    /// what is folded.
    pub fn run_edges_watched(&mut self, edges: u64, signals: &[SignalId]) -> Result<Vec<u8>> {
        self.sampled(edges, signals, true, true)
    }
    fn sampled(
        &mut self,
        count: u64,
        probes: &[SignalId],
        every: bool,
        watched: bool,
    ) -> Result<Vec<u8>> {
        let rows = usize::try_from(count)?;
        let width = probes.len();
        let mut out = vec![0_u8; rows * width];
        let mut done = 0;
        // Tuned, the run goes in pieces, each on the program `tune` picks.
        while done < count {
            let chunk = match self.tuning.is_some() {
                true => self.tune_chunk(count - done, every)?,
                false => count - done,
            };
            let mut slots = Vec::new();
            let mut inverted = Vec::new();
            for &p in probes {
                let (slot, inv) = self
                    .loaded
                    .program
                    .slot(p)
                    .context("probe is not in the fast program")?;
                slots.push(slot as u32);
                inverted.push(u8::from(inv));
            }
            let rows = &mut out[done as usize * width..(done + chunk) as usize * width];
            let (started, from) = (Instant::now(), self.now_ps());
            self.run_sampled(chunk, &slots, rows, every)?;
            if inverted.contains(&1) {
                for row in rows.chunks_exact_mut(width) {
                    for (b, inv) in row.iter_mut().zip(&inverted) {
                        // An inverted X or Z (2, 3) is X.
                        *b = if *b < 2 {
                            *b ^ inv
                        } else if *inv == 1 {
                            2
                        } else {
                            *b
                        };
                    }
                }
            }
            if watched {
                let program = &self.loaded.program;
                let stale: Vec<usize> = (0..width).filter(|&k| !program.fresh(probes[k])).collect();
                if !stale.is_empty() {
                    for row in rows.chunks_exact_mut(width) {
                        for &k in &stale {
                            row[k] = NOT_SAMPLED;
                        }
                    }
                }
            }
            if self.tuning.is_some() {
                self.tune_ran(self.now_ps() - from, started.elapsed())?;
            }
            done += chunk;
        }
        Ok(out)
    }
    /// `count` falling edges, or clock edges when `every`, with a row of
    /// probes after each.
    fn run_sampled(
        &mut self,
        count: u64,
        probes: &[u32],
        out: &mut [u8],
        every: bool,
    ) -> Result<()> {
        if !self.dtrs.is_empty() {
            return self.run_dtr_sampled(count, probes, out, every);
        }
        let c = self.clock.context("start a clock first")?;
        let clock = self.waves[c];
        if self.waves.len() == 1
            && self.plls.is_empty()
            && clock.value == 1
            && (!every || count.is_multiple_of(2))
        {
            let cycles = if every { count / 2 } else { count };
            // Only the clock, next rising: whole cycles inside the program.
            // SAFETY: the probes are slots of the program and `out` has a row
            // of them per cycle.
            let tracing = !self.trace_signals.is_empty();
            let mut combined = Vec::new();
            let mut raw = Vec::new();
            if tracing {
                self.trace_layout()?;
                combined.extend_from_slice(probes);
                combined.extend(self.trace_slots.iter().map(|&(slot, _)| slot));
                raw.resize(2 * cycles as usize * combined.len(), 0);
            }
            let done = unsafe {
                (self.loaded.run)(
                    self.values.as_mut_ptr(),
                    clock.slot as u32,
                    cycles,
                    if tracing {
                        combined.as_ptr()
                    } else {
                        probes.as_ptr()
                    },
                    if tracing {
                        combined.len()
                    } else {
                        probes.len()
                    } as u32,
                    u32::from(every || tracing),
                    if tracing {
                        raw.as_mut_ptr()
                    } else {
                        out.as_mut_ptr()
                    },
                )
            };
            if done != cycles {
                return Err(self.failure());
            }
            if tracing {
                self.trace_split(&raw, probes.len(), out, |i| {
                    (clock.next + i as u64 * clock.half, every || i % 2 == 1)
                });
            }
            if cycles > 0 {
                self.now = clock.next + (2 * cycles - 1) * clock.half;
                self.waves[c].next = clock.next + 2 * cycles * clock.half;
            }
            return Ok(());
        }
        if count >= SCHEDULE_CYCLES {
            if let Some(mut table) = self.table(c, every)? {
                if !self.trace_signals.is_empty() {
                    self.trace_layout()?;
                    let flags = table.sample.clone();
                    let per = flags.iter().map(|&v| u64::from(v)).sum::<u64>();
                    ensure!(per > 0, "the schedule has no clock edge");
                    let full = (count - 1) / per;
                    let nth = ((count - 1) % per) as usize;
                    let last = flags
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| **v == 1)
                        .nth(nth)
                        .context("schedule sample missing")?
                        .0;
                    let instants = full * flags.len() as u64 + last as u64 + 1;
                    table.sample.fill(1);
                    let mut combined = probes.to_vec();
                    combined.extend(self.trace_slots.iter().map(|&(slot, _)| slot));
                    let mut raw = vec![0; instants as usize * combined.len()];
                    let start = self.now;
                    self.run_table(&table, instants, &combined, &mut raw)?;
                    self.trace_split(&raw, probes.len(), out, |i| {
                        (
                            start
                                + (i / flags.len()) as u64 * table.period
                                + table.offsets[i % flags.len()],
                            flags[i % flags.len()] == 1,
                        )
                    });
                    return Ok(());
                }
                return self.run_table(&table, count, probes, out);
            }
        }
        let mut done = 0;
        while done < count {
            let edge = self.next_point(u64::MAX)?.context("no edges left")?;
            if edge == Some(0) || (every && edge.is_some()) {
                let row = done as usize * probes.len();
                for (k, &p) in probes.iter().enumerate() {
                    out[row + k] = self.values[p as usize];
                }
                done += 1;
            }
        }
        Ok(())
    }
    pub fn program(&self) -> &FastProgram {
        &self.loaded.program
    }
    /// Every slot as the program has it now (`program().slots` of them),
    /// for diagnostics: how many change in a cycle, say.
    pub fn raw_values(&self) -> &[u8] {
        &self.values
    }
    /// The values of `reader`'s signals, in its order, into `out`, as
    /// `value` gives them: 0, 1, or 2 for X and for a signal the program does
    /// not have. The slots are looked up once, and again only after tuning
    /// has changed program.
    pub fn read(&self, reader: &mut Reader, out: &mut Vec<u8>) {
        if reader
            .place
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(p, &self.loaded.place))
        {
            let program = &self.loaded.program;
            reader.slots = reader
                .signals
                .iter()
                .map(|s| match s.and_then(|s| program.slot(s)) {
                    Some((slot, inverted)) => (slot as u32, u8::from(inverted)),
                    None => (u32::MAX, 0),
                })
                .collect();
            reader.place = Some(Arc::clone(&self.loaded.place));
        }
        out.clear();
        out.extend(reader.slots.iter().map(
            |&(slot, inverted)| match self.values.get(slot as usize) {
                Some(&b) if b < 2 => b ^ inverted,
                _ => 2,
            },
        ));
    }
    /// The state now, to come back to with `restore`.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            values: self.values.clone(),
            place: Arc::clone(&self.loaded.place),
            tick: self.tick,
            now: self.now,
            waves: self.waves.clone(),
            plls: self.plls.clone(),
            dtrs: self.dtrs.clone(),
        }
    }
    /// Goes back (or on) to a state taken with `snapshot` of this
    /// simulation, also when it has changed program since (`tune`).
    pub fn restore(&mut self, s: &Snapshot) {
        if !self.trace_signals.is_empty() {
            self.trace_buffer.rows.clear();
            self.trace_buffer.times_ps.clear();
            self.trace_buffer.gap = true;
        }
        self.tick = s.tick;
        self.now = s.now;
        self.waves.clone_from(&s.waves);
        self.plls.clone_from(&s.plls);
        self.dtrs.clone_from(&s.dtrs);
        // Copy directly only when `s` comes from this exact program. Two
        // programs of one kernel can lay the slots out alike and still fold
        // or evaluate on demand different nets (another folding width tried
        // by the tuner, or a program built to keep watched nets); a plain
        // copy would leave those nets stale. A snapshot of another program
        // is therefore always carried and settled, even when the layouts
        // match; this costs a full settle (about 1.3 ms on `life_full`) when
        // the tuner goes back to a program it had, which is rare, and
        // comparing layouts instead would be wrong, not just faster.
        if s.values.len() == self.values.len() && Arc::ptr_eq(&s.place, &self.loaded.place) {
            self.values.copy_from_slice(&s.values);
        } else {
            self.values = carry(&s.values, &s.place, &self.loaded);
            for wave in &mut self.waves {
                wave.slot = carry_slot(wave.slot, &s.place, &self.loaded.place);
            }
            // Bring every net up to date; `step` captures nothing, the state
            // being one between instants.
            let _ = self.step();
        }
    }
    /// Sets an input the kernel drove through an external driver.
    pub fn set(&mut self, driver: DriverId, value: bool) -> Result<()> {
        let (slot, level) = self.input_level(driver, Some(value))?;
        self.values[slot] = level;
        Ok(())
    }
    /// As `set`, with `None` for Z on a tristate pad's external driver.
    pub fn set_levels(&mut self, inputs: &[(DriverId, Option<bool>)]) -> Result<()> {
        let mut updates = Vec::with_capacity(inputs.len());
        for &(driver, value) in inputs {
            updates.push(self.input_level(driver, value)?);
        }
        for (slot, level) in updates {
            self.values[slot] = level;
        }
        Ok(())
    }
    /// Settles after input changes, at the current time, and captures the
    /// registers they clock.
    pub fn step(&mut self) -> Result<()> {
        // SAFETY: the array has `program.slots` entries, as the program expects.
        let status = unsafe { (self.loaded.step)(self.values.as_mut_ptr()) };
        self.finish_dtrs(status)?;
        self.track_plls()
    }
    /// Return 2 pauses after a wave's captures, before the host publishes
    /// DTROUT. STARTPULSE was latched before those captures, so a register
    /// changing it cannot hide a pulse or change what that wave sampled.
    fn finish_dtrs(&mut self, mut status: c_int) -> Result<()> {
        for _ in 0..10000 {
            if status == 0 {
                return Ok(());
            }
            if status != 2 || self.dtrs.is_empty() {
                return Err(self.failure());
            }
            let duration = self.ticks(Time::from_ps(DTR_CONVERSION_PS))?;
            for (dtr, state) in self.loaded.program.dtrs.iter().zip(&mut self.dtrs) {
                let start = self.values[dtr.captured];
                if start == state.previous {
                    continue;
                }
                state.completion = if state.previous == 1 && start == 0 {
                    Some(
                        self.now
                            .checked_add(duration)
                            .context("DTR completion overflows")?,
                    )
                } else {
                    None
                };
                state.previous = start;
                for &slot in &dtr.outputs {
                    self.values[slot] = 0;
                }
            }
            // SAFETY: values has the loaded program's complete slot layout.
            status = unsafe {
                (self.loaded.resume.context("DTR resume is missing")?)(self.values.as_mut_ptr())
            };
        }
        bail!("fast mode DTR did not converge at {}", self.now()?)
    }

    /// DTR-only batching. Stop strictly before a pending completion; instant()
    /// publishes that completion together with any coincident clock edges.
    /// C returns (completed instants << 2) | reason, leaving a paused instant
    /// unsampled until finish_dtrs has closed all its capture waves.
    fn run_dtr_sampled(
        &mut self,
        count: u64,
        probes: &[u32],
        out: &mut [u8],
        every: bool,
    ) -> Result<()> {
        let c = self.clock.context("start a clock first")?;
        let mut done = 0;
        self.trace_layout()?;
        let mut combined = probes.to_vec();
        combined.extend(self.trace_slots.iter().map(|&(slot, _)| slot));
        while done < count {
            let deadline = self
                .dtrs
                .iter()
                .filter_map(|d| d.completion)
                .min()
                .unwrap_or(u64::MAX);
            let next = self
                .waves
                .iter()
                .map(|w| w.next)
                .min()
                .context("no edges left")?;
            let single = self.waves.len() == 1 && self.plls.is_empty();
            let table = if !single && next < deadline {
                self.table(c, every)?
            } else {
                None
            };
            if next >= deadline || (!single && table.is_none()) {
                let edge = self.next_point(u64::MAX)?.context("no instants left")?;
                if edge == Some(0) || (every && edge.is_some()) {
                    let row = done as usize * probes.len();
                    for (k, &p) in probes.iter().enumerate() {
                        out[row + k] = self.values[p as usize];
                    }
                    done += 1;
                }
                continue;
            }
            let clock = self.waves[c];
            let mut times = Vec::new();
            let mut samples = Vec::new();
            let mut taken = 0;
            for i in 0..65536_usize {
                let (t, sample) = if let Some(table) = &table {
                    let n = table.offsets.len();
                    (
                        self.now + (i / n) as u64 * table.period + table.offsets[i % n],
                        table.sample[i % n] != 0,
                    )
                } else {
                    (
                        clock.next + i as u64 * clock.half,
                        every || (clock.value ^ (i as u8 & 1)) == 0,
                    )
                };
                if t >= deadline {
                    break;
                }
                times.push(t);
                samples.push(sample);
                taken += u64::from(sample);
                if taken == count - done {
                    break;
                }
            }
            let mut raw = vec![0; times.len() * combined.len()];
            // SAFETY: all slots, table indices and output rows belong to the
            // loaded program. DTR programs use the instant-count ABI above.
            let result = unsafe {
                if let Some(table) = &table {
                    (self.loaded.schedule)(
                        self.values.as_mut_ptr(),
                        table.first.as_ptr(),
                        table.slots.as_ptr(),
                        table.values.as_ptr(),
                        table.sample.as_ptr(),
                        table.offsets.len() as u32,
                        times.len() as u64,
                        combined.as_ptr(),
                        combined.len() as u32,
                        raw.as_mut_ptr(),
                    )
                } else {
                    (self.loaded.run)(
                        self.values.as_mut_ptr(),
                        clock.slot as u32,
                        times.len() as u64,
                        combined.as_ptr(),
                        combined.len() as u32,
                        1,
                        raw.as_mut_ptr(),
                    )
                }
            };
            let completed = (result >> 2) as usize;
            let status = (result & 3) as c_int;
            let ran = completed + usize::from(status != 0);
            self.now = times[ran - 1];
            for wave in &mut self.waves {
                wave.pass(self.now);
            }
            self.finish_dtrs(status)?;
            if status != 0 {
                for (k, &p) in combined.iter().enumerate() {
                    raw[completed * combined.len() + k] = self.values[p as usize];
                }
            }
            if table.is_some() {
                // table() only batches PLLs already locked to this clock.
                let clock = self.waves[c];
                for (p, state) in self.loaded.program.plls.iter().zip(&mut self.plls) {
                    state.previous = self.values[p.reference.0] ^ u8::from(p.reference.1);
                    let edge = clock.next - clock.half;
                    state.last_rise = Some(if (clock.value ^ 1) ^ u8::from(p.reference.1) == 1 {
                        edge
                    } else {
                        edge - clock.half
                    });
                }
            }
            for i in 0..ran {
                let row = &raw[i * combined.len()..(i + 1) * combined.len()];
                if samples[i] {
                    let offset = done as usize * probes.len();
                    out[offset..offset + probes.len()].copy_from_slice(&row[..probes.len()]);
                    done += 1;
                }
                if !self.trace_signals.is_empty() {
                    self.trace_buffer.times_ps.push(self.trace_ps(times[i]));
                    for (value, &(_, inv)) in row[probes.len()..].iter().zip(&self.trace_slots) {
                        self.trace_buffer.rows.push(trace_value(*value, inv));
                    }
                }
            }
        }
        Ok(())
    }
    /// Why the program stopped, at the current time.
    fn failure(&self) -> anyhow::Error {
        // SAFETY: fast_error reads the diagnostic slots reserved by this
        // program in this simulation's value array.
        let error = unsafe { (self.loaded.error)(self.values.as_ptr()) };
        let (code, unit) = (error >> 16, (error & 0xffff) as usize);
        let at = self
            .now()
            .map_or_else(|_| "an unrepresentable time".into(), |t| t.to_string());
        // A pad conflict is flagged in this simulation's own slots (unit + 1).
        if let Some(flag) = self.loaded.program.pad_conflict {
            if let Some(&unit) = self.values.get(flag).filter(|&&u| u != 0) {
                let pad = self
                    .loaded
                    .program
                    .pads
                    .get(usize::from(unit) - 1)
                    .map_or("?", String::as_str);
                return anyhow::anyhow!(
                    "fast mode stopped at {at}: pad {pad} is driven by the design and from \
                     outside with different levels; the event kernel makes it X"
                );
            }
        }
        let ebr = self
            .loaded
            .program
            .ebrs
            .get(unit)
            .map_or("?", String::as_str);
        match code {
            7 => anyhow::anyhow!(
                "fast mode is two-valued; DCSC {unit} has an ambiguous target, unknown selected clock, or uncertain handshake edge at {at}; the event kernel makes DCSOUT X"
            ),
            6 => anyhow::anyhow!(
                "fast mode is two-valued; STARTPULSE of DTR {unit} is unknown at {at}"
            ),
            5 => anyhow::anyhow!(
                "fast mode stopped at {at}: asynchronous PRLD register {unit} released LSR \
                 while M changed in the same instant; the event kernel makes Q unknown"
            ),
            2 => anyhow::anyhow!(
                "fast mode stopped at {at}: the logic reads an output of EBR {ebr} that the event \
                 kernel makes unknown here (a cross-port collision on one row, a control \
                 combination the board has not shown, or a read of unknown memory); fast mode is \
                 two-valued, so run this with the event kernel"
            ),
            3 => anyhow::anyhow!(
                "fast mode stopped at {at}: EBR {ebr} has an active local reset with synchronous \
                 release, which the event kernel rejects"
            ),
            _ => anyhow::anyhow!(
                "fast mode did not converge at {at}: 10000 capture waves without a stable state"
            ),
        }
    }
    /// The edges of one hyperperiod from now, when they repeat: every PLL has
    /// learnt its period from the clock, and the instants are few enough.
    /// Samples follow falling edges of the clock, or every edge when `every`.
    fn table(&self, clock: usize, every: bool) -> Result<Option<Table>> {
        let clock_slot = self.waves[clock].slot;
        let periodic = self
            .loaded
            .program
            .plls
            .iter()
            .zip(&self.plls)
            .all(|(p, s)| p.reference.0 == clock_slot && s.period.is_some());
        if !periodic {
            return Ok(None);
        }
        let mut period: u128 = 1;
        let mut instants: u128 = 0;
        for wave in &self.waves {
            let p = u128::from(2 * wave.half);
            period = period / gcd(period, p) * p;
            if period > u128::from(u64::MAX / 2) {
                return Ok(None);
            }
        }
        for wave in &self.waves {
            instants += period / u128::from(wave.half);
        }
        if instants > SCHEDULE_INSTANTS {
            return Ok(None);
        }
        let period = period as u64;
        let mut waves = self.waves.clone();
        let mut table = Table {
            offsets: Vec::new(),
            first: vec![0],
            slots: Vec::new(),
            values: Vec::new(),
            sample: Vec::new(),
            period,
        };
        loop {
            let t = waves.iter().map(|w| w.next).min().context("no edges")?;
            if t > self.now + period {
                break;
            }
            let mut sample = false;
            for (i, wave) in waves.iter_mut().enumerate() {
                if wave.next == t {
                    table.slots.push(wave.slot as u32);
                    table.values.push(wave.value);
                    sample |= i == clock && (every || wave.value == 0);
                    wave.value ^= 1;
                    wave.next += wave.half;
                }
            }
            table.offsets.push(t - self.now);
            table.first.push(table.slots.len() as u32);
            table.sample.push(u8::from(sample));
        }
        Ok(Some(table))
    }
    /// Runs until `samples` sampled instants of a table built now, then moves
    /// every wave and PLL to the time of the last instant run.
    fn run_table(
        &mut self,
        table: &Table,
        samples: u64,
        probes: &[u32],
        out: &mut [u8],
    ) -> Result<()> {
        let n = table.offsets.len() as u64;
        ensure!(table.sample.contains(&1), "the schedule has no clock edge");
        // SAFETY: the table arrays are consistent (first has n + 1 entries
        // into slots and values), the slots are below `program.slots`, and
        // `out` has a row of probes per cycle.
        let run = unsafe {
            (self.loaded.schedule)(
                self.values.as_mut_ptr(),
                table.first.as_ptr(),
                table.slots.as_ptr(),
                table.values.as_ptr(),
                table.sample.as_ptr(),
                n as u32,
                samples,
                probes.as_ptr(),
                probes.len() as u32,
                out.as_mut_ptr(),
            )
        };
        if run == u64::MAX {
            return Err(self.failure());
        }
        let last = run - 1;
        let end = self.now + (last / n) * table.period + table.offsets[(last % n) as usize];
        for wave in &mut self.waves {
            wave.pass(end);
        }
        self.now = end;
        // The PLL references are the clock: their last rising edge is the
        // clock's last edge to their rising level at or before now.
        let clock = self.waves[self.clock.context("no clock")?];
        for (p, state) in self.loaded.program.plls.iter().zip(&mut self.plls) {
            let level = self.values[p.reference.0] ^ u8::from(p.reference.1);
            state.previous = level;
            let edge = clock.next - clock.half;
            state.last_rise = Some(if (clock.value ^ 1) ^ u8::from(p.reference.1) == 1 {
                edge
            } else {
                edge - clock.half
            });
        }
        Ok(())
    }
    /// The array of EBR `b` (in `program().ebrs` order): cell 9·row + bit of
    /// the canonical 2048x9 view, 0, 1 or 2 for unknown.
    pub fn memory(&self, b: usize) -> Option<&[u8]> {
        let first = *self.loaded.program.memories.get(b)?;
        Some(&self.values[first..first + 2048 * 9])
    }
    /// The sixteen cells of LUTRAM column `w` (in the kernel's writer order),
    /// in physical LUT bit order: cell A + 2B + 4C + 8D.
    pub fn lutram(&self, w: usize) -> Option<&[u8]> {
        let first = *self.loaded.program.lutrams.get(w)?;
        Some(&self.values[first..first + 16])
    }
    /// The value of a signal. A probed signal is always up to date. Another
    /// net may be left from an earlier cycle until `step`: one that only the
    /// data input of a register with an enable reads, which fast mode
    /// evaluates only for a capture that uses it, or one folded into the table
    /// of the gate (with `FAST_FOLD=cones`, the gates) that reads it
    /// (docs/FAST_MODE.md).
    pub fn value(&self, signal: SignalId) -> Option<Logic> {
        self.loaded
            .program
            .slot(signal)
            .map(|(s, inverted)| match (self.values[s], inverted) {
                // 3 is Z, held only by a tristate net that nothing reads but
                // probes (kernel/fast.rs).
                (3, false) => Logic::Z,
                (2.., _) => Logic::X,
                (b, _) => Logic::from_bool((b ^ u8::from(inverted)) == 1),
            })
    }
}

/// A watched signal's value after an edge that the program running it did
/// not keep up to date (`FastSim::run_edges_watched`).
pub const NOT_SAMPLED: u8 = u8::MAX;

/// Tracing: the pads (or any signals) after every instant the program runs,
/// for decoding what a design sends faster than the board clock (a TMDS lane
/// on a 125 MHz PLL clock). Off unless `trace` is given signals; then every
/// path that runs instants (`instant`, `fast_run`, `fast_schedule`) appends
/// a row and its time, and the caller's own samples are the same as without
/// a trace. `take_trace` drains the buffer: it grows by one row of the traced
/// signals plus 8 bytes of time per instant, about 3 GB a second of board time
/// for four pads on a 125 MHz DDR serializer (5 instants per edge of the
/// 25 MHz clock), so take it after every run. A tuned run (`tune`) may switch
/// programs while tracing; the slots are found again after each switch. Its
/// measurements then include the trace, so callers that trace do not tune.
impl FastSim {
    /// Trace these signals after every instant. An empty slice turns tracing
    /// off. Setting it clears the buffer; signals must be compiled probes if
    /// their values would otherwise be folded or evaluated on demand.
    pub fn trace(&mut self, signals: &[SignalId]) -> Result<()> {
        for &signal in signals {
            self.loaded
                .program
                .slot(signal)
                .context("trace signal is not in the fast program")?;
        }
        self.trace_signals = signals.to_vec();
        self.trace_place = None;
        self.trace_buffer = Trace {
            width: signals.len(),
            ..Trace::default()
        };
        self.trace_layout()
    }
    /// Drain the trace, keeping the selected signals for the next chunk.
    pub fn take_trace(&mut self) -> Trace {
        std::mem::replace(
            &mut self.trace_buffer,
            Trace {
                width: self.trace_signals.len(),
                ..Trace::default()
            },
        )
    }
    fn trace_layout(&mut self) -> Result<()> {
        if self
            .trace_place
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(p, &self.loaded.place))
        {
            self.trace_slots = self
                .trace_signals
                .iter()
                .map(|&s| {
                    self.loaded
                        .program
                        .slot(s)
                        .map(|(slot, inv)| (slot as u32, u8::from(inv)))
                        .context("trace signal is not in the fast program")
                })
                .collect::<Result<_>>()?;
            self.trace_place = Some(Arc::clone(&self.loaded.place));
        }
        Ok(())
    }
    fn trace_ps(&self, tick: u64) -> u64 {
        let (n, d) = self.tick.ratio();
        (u128::from(tick) * u128::from(n) / u128::from(d)) as u64
    }
    /// Split C rows into the caller's samples and the instant trace. Caller
    /// inversions are still applied by `sampled`, exactly as without a trace.
    fn trace_split(
        &mut self,
        raw: &[u8],
        probes: usize,
        out: &mut [u8],
        mut time_and_sample: impl FnMut(usize) -> (u64, bool),
    ) {
        let width = probes + self.trace_slots.len();
        let mut sample = 0;
        for (i, row) in raw.chunks_exact(width).enumerate() {
            let (tick, wanted) = time_and_sample(i);
            self.trace_buffer.times_ps.push(self.trace_ps(tick));
            for (&value, &(_, inv)) in row[probes..].iter().zip(&self.trace_slots) {
                self.trace_buffer.rows.push(trace_value(value, inv));
            }
            if wanted {
                out[sample * probes..(sample + 1) * probes].copy_from_slice(&row[..probes]);
                sample += 1;
            }
        }
    }
}

/// Match sampled probe inversion, including X/Z.
fn trace_value(value: u8, inverted: u8) -> u8 {
    if value < 2 {
        value ^ inverted
    } else if inverted != 0 {
        2
    } else {
        value
    }
}

/// `values` in the layout `place` carried into the layout of `to`: every
/// slot of the common numbering moves to where `to` keeps it; slots `to`
/// keeps nowhere else (the old places of moved slots, padding) take its
/// initial values.
fn carry(values: &[u8], place: &[usize], to: &Loaded) -> Vec<u8> {
    let mut out = to.program.initial.clone();
    for (&from, &into) in place.iter().zip(to.place.iter()) {
        out[into] = values[from];
    }
    out
}

/// Slot `slot` of the layout `from` in the layout `to`.
fn carry_slot(slot: usize, from: &[usize], to: &[usize]) -> usize {
    let base = match from.get(slot) {
        Some(&s) if s == slot => slot,
        _ => from.iter().position(|&s| s == slot).unwrap_or(slot),
    };
    to.get(base).copied().unwrap_or(slot)
}

/// How long each candidate runs when choosing fast mode's settings.
const TRIAL: Duration = Duration::from_millis(250);

/// Startup choices need no C generation: bind the configuration and selected
/// kernel plan, generator, generation flags, cores and pinned base options.
pub fn choice_memory(kernel: &Kernel) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        "fast-choice-v2",
        kernel.fast_choice_identity(),
        include_str!("kernel/fast.rs"),
        include_str!("kernel/fast_dsp.rs"),
        include_str!("kernel/fast_dcs.rs"),
        compile_flags(),
        [
            "FAST_FOLD",
            "FAST_ACTIVITY",
            "FAST_CODE_BUDGET",
            "FAST_FUNCTION_LINES",
            "FAST_THREAD_STEAL",
            "FAST_THREAD_STATS",
        ]
        .map(std::env::var_os),
    )
        .hash(&mut hasher);
    choice_path(hasher.finish())
}

// Old source-based records are migrated after the first default generation.
fn legacy_choice_memory(default_source: &str) -> PathBuf {
    choice_path(default_source)
}

fn choice_path(identity: impl Hash) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        identity,
        performance_cores(),
        FastOptions::pinned(),
        FastOptions::from_env(),
    )
        .hash(&mut hasher);
    std::env::temp_dir()
        .join("ecp5_emu_fast")
        .join(format!("{:016x}.choice", hasher.finish()))
}

fn read_choices(path: &Path, base: FastOptions) -> Vec<(FastOptions, f64)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w.len() != 3 && w.len() != 5 {
                return None;
            }
            // The original three columns still work. The tuner can also
            // change the split threshold and meeting cost: keep those too.
            let options = FastOptions {
                threads: w[0].parse().ok()?,
                collapse: w[1].parse().ok()?,
                thread_min: w.get(3).map_or(Some(base.thread_min), |w| w.parse().ok())?,
                barrier_lines: w
                    .get(4)
                    .map_or(Some(base.barrier_lines), |w| w.parse().ok())?,
            };
            let speed: f64 = w[2].parse().ok()?;
            (speed.is_finite() && speed > 0.0).then_some((options, speed))
        })
        .collect()
}

fn remembered_choices(path: &Path, base: FastOptions) -> Vec<(FastOptions, f64)> {
    let pinned = FastOptions::pinned();
    let cores = performance_cores();
    read_choices(path, base)
        .into_iter()
        .filter(|(o, _)| {
            let valid_threads = if pinned[0] {
                o.threads == base.threads
            } else {
                (1..=cores).contains(&o.threads)
            };
            valid_threads
                && (!pinned[1] || o.thread_min == base.thread_min)
                && (!pinned[2] || o.collapse == base.collapse)
                && (!pinned[3] || o.barrier_lines == base.barrier_lines)
        })
        .collect()
}

fn write_choices(path: &Path, table: &[(FastOptions, f64)]) {
    let text: String = table
        .iter()
        .map(|(o, speed)| {
            format!(
                "{} {} {speed:.0} {} {}\n",
                o.threads, o.collapse, o.thread_min, o.barrier_lines
            )
        })
        .collect();
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(path, text))
            .is_err()
        {
            eprintln!("fast mode: could not keep the choice in {}", path.display());
        }
    }
}

fn starting_options(gates: usize, base: FastOptions, pinned: bool, cores: usize) -> FastOptions {
    let threads = if pinned {
        base.threads
    } else if gates < base.thread_min {
        1
    } else if gates >= 50_000 {
        // docs/FAST_MODE.md, Speed / The full-chip Life: Life (~10k LUTs,
        // 14k gates) likes the default 3 threads; life_full (~79k LUTs,
        // 108k gates) likes 8-10 on 12 performance cores. Use a conservative
        // 50k raw-gate boundary, then let the in-run tuner refine the guess.
        cores.min(10)
    } else {
        base.threads
    };
    FastOptions { threads, ..base }
}

/// Chooses a simulation for `kernel` on this machine, reusing `initial`
/// when its options win. A measured winner is restored to the kernel's
/// starting state, including its clocks, before being handed to the caller.
/// With no memory, returns the simulation already guessed by `start` without
/// generating C or running trials; `FastSim::tune` refines it in the run.
/// With memory, measures only the last best and returns it at half its
/// remembered speed or better. Otherwise tries its neighbours, the second
/// best and defaults, then searches everything if still below half.
/// `FAST_CHOOSE=full` always searches thread counts and foldings. Trials use
/// throwaway simulations without advancing the kernel; environment options
/// stay pinned. Reports startup and trials through `progress`; cancellation
/// prevents further trials and fails the choice, including a guessed choice.
pub fn choose_options(
    kernel: &Kernel,
    initial: FastSim,
    clock: DriverId,
    period: Time,
    probes: &[SignalId],
    progress: &dyn Fn(String),
    cancelled: &dyn Fn() -> bool,
) -> Result<FastSim> {
    let base = FastOptions::from_env();
    let pinned = FastOptions::pinned();
    let cores = performance_cores();
    let threads: Vec<usize> = if pinned[0] {
        vec![base.threads]
    } else {
        [1, 2, 3, 4, 6, 8, 10, 12, 14, 16]
            .into_iter()
            .filter(|&t| t <= cores)
            .collect()
    };
    let foldings: Vec<usize> = if pinned[2] {
        Vec::new()
    } else {
        vec![0, 6, 8, 12]
    };
    let memory = choice_memory(kernel);
    let full = std::env::var("FAST_CHOOSE").is_ok_and(|v| v.trim() == "full");
    let remembered: Vec<(FastOptions, f64)> = if full {
        Vec::new()
    } else {
        let table = remembered_choices(&memory, base);
        if table.is_empty() && initial.program().options == base {
            remembered_choices(&legacy_choice_memory(&initial.program().source), base)
        } else {
            table
        }
    };
    ensure!(!cancelled(), "choosing fast mode's settings was cancelled");
    if remembered.is_empty() && !full {
        progress(format!(
            "Starting with {}; tuning in the background",
            describe_fast(&initial.program().options)
        ));
        ensure!(!cancelled(), "choosing fast mode's settings was cancelled");
        return Ok(initial);
    }
    // Each program measured: its settings, its source's hash, its speeds.
    let mut measured: Vec<(FastOptions, u64, Vec<f64>)> = Vec::new();
    let tried = std::cell::Cell::new(0);
    let best = |measured: &[(FastOptions, u64, Vec<f64>)]| {
        measured
            .iter()
            .max_by(|a, b| top(&a.2).total_cmp(&top(&b.2)))
            .map(|m| m.0)
    };
    let mut initial = Some(initial);
    let mut winner = None;
    let mut trial =
        |options: FastOptions, measured: &mut Vec<(FastOptions, u64, Vec<f64>)>, total: usize| {
            if cancelled() {
                return;
            }
            tried.set(tried.get() + 1);
            progress(format!(
                "Choosing fast mode's settings: {} of about {total} ({})",
                tried.get(),
                describe_fast(&options)
            ));
            let repeated = measured.iter().position(|m| m.0 == options);
            let result = (|| -> Result<Option<(f64, u64, FastSim)>> {
                let mut sim = if initial
                    .as_ref()
                    .is_some_and(|s| s.program().options == options)
                {
                    initial.take().context("initial simulation already used")?
                } else {
                    FastSim::with_options(kernel, &options)?
                };
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                sim.program().source.hash(&mut hasher);
                let source = hasher.finish();
                // Another setting made this program: nothing new to measure.
                // The fastest programs are deliberately measured again below.
                if repeated.is_none() && measured.iter().any(|m| m.1 == source) {
                    return Ok(None);
                }
                let start = sim.snapshot();
                let start_clock = sim.clock;
                let speed = speed_of(&mut sim, clock, period, probes)?;
                sim.restore(&start);
                // Undo the clock installed by this private trial here;
                // leave the public snapshot/restore contract unchanged.
                sim.clock = start_clock;
                Ok(Some((speed, source, sim)))
            })();
            match result {
                Ok(Some((speed, source, sim))) => {
                    if let Some(i) = repeated {
                        measured[i].2.push(speed);
                    } else {
                        measured.push((options, source, vec![speed]));
                    }
                    if best(measured) == Some(options) {
                        winner = Some(sim);
                    }
                }
                Ok(None) => {}
                Err(e) if repeated.is_none() => {
                    eprintln!("fast mode: {} failed: {e:#}", describe_fast(&options));
                }
                Err(_) => {}
            }
        };
    let mut search = remembered.is_empty();
    let confirmed = remembered.first().is_some_and(|&(last, was)| {
        trial(last, &mut measured, 1);
        measured.first().is_some_and(|m| top(&m.2) >= was / 2.0)
    });
    if let Some(&(last, was)) = remembered.first().filter(|_| !confirmed) {
        // The best of last time, a thread count either side, the second,
        // and the defaults (a busy machine can make many threads very slow).
        let mut round = vec![last];
        for k in [
            threads.iter().rposition(|&t| t < last.threads),
            threads.iter().position(|&t| t > last.threads),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(&t) = threads.get(k) {
                round.push(FastOptions { threads: t, ..last });
            }
        }
        round.extend(remembered.get(1).map(|r| r.0));
        round.push(base);
        let mut seen = std::collections::HashSet::new();
        round.retain(|o| seen.insert(*o));
        let total = tried.get() + round.len();
        for o in round {
            trial(o, &mut measured, total);
        }
        // Far slower than last time: the machine is busier now, or the
        // design's programs changed; everything is measured again.
        let now = measured.iter().map(|m| top(&m.2)).fold(0.0, f64::max);
        if now < was / 2.0 {
            eprintln!(
                "fast mode: {:.1} kHz at best against {:.1} last time; measuring every setting again",
                now / 2e3,
                was / 2e3
            );
            search = true;
        }
    }
    if search {
        let total = tried.get() + threads.len() + foldings.len() + 3;
        for &t in &threads {
            trial(FastOptions { threads: t, ..base }, &mut measured, total);
        }
        let fastest = best(&measured).unwrap_or(base);
        for &c in &foldings {
            if c != fastest.collapse {
                trial(
                    FastOptions {
                        collapse: c,
                        ..fastest
                    },
                    &mut measured,
                    total,
                );
            }
        }
        // The three fastest once more: a busy machine slows a run now and then.
        let mut order: Vec<FastOptions> = measured.iter().map(|m| m.0).collect();
        order.sort_by(|a, b| {
            let speed = |o: &FastOptions| {
                measured
                    .iter()
                    .find(|m| m.0 == *o)
                    .map_or(0.0, |m| top(&m.2))
            };
            speed(b).total_cmp(&speed(a))
        });
        for o in order.into_iter().take(3) {
            trial(o, &mut measured, total);
        }
    }
    ensure!(!cancelled(), "choosing fast mode's settings was cancelled");
    let chosen = best(&measured).context("no program of fast mode ran")?;
    // Kept for next time: this opening's speeds, and the rest of last time's.
    let mut table: Vec<(FastOptions, f64)> = measured.iter().map(|m| (m.0, top(&m.2))).collect();
    for &(o, speed) in &remembered {
        if !table.iter().any(|t| t.0 == o) {
            table.push((o, speed));
        }
    }
    table.sort_by(|a, b| b.1.total_cmp(&a.1));
    write_choices(&memory, &table);
    eprintln!(
        "fast mode: chose {} — {}",
        describe_fast(&chosen),
        table
            .iter()
            .map(|(o, speed)| format!("{} {:.1} kHz", describe_fast(o), speed / 2e3))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let sim = winner.context("no chosen simulation was kept")?;
    debug_assert_eq!(sim.program().options, chosen);
    Ok(sim)
}

/// The best of a program's speeds measured.
fn top(speeds: &[f64]) -> f64 {
    speeds.iter().copied().fold(0.0, f64::max)
}

/// Fast mode's settings in a few words.
fn describe_fast(o: &FastOptions) -> String {
    format!(
        "{} thread{}, folding {}",
        o.threads,
        if o.threads == 1 { "" } else { "s" },
        o.collapse
    )
}

/// How fast `sim` runs from its current state, in clock edges a second.
fn speed_of(sim: &mut FastSim, clock: DriverId, period: Time, probes: &[SignalId]) -> Result<f64> {
    sim.clock(clock, period)?;
    // Warmed up (caches, the threads started), then timed.
    let mut piece = 64_u64;
    let warm = Instant::now();
    while warm.elapsed() < TRIAL / 5 {
        sim.run_edges_sampled(piece, probes)?;
        piece = (piece * 2).min(1 << 14);
    }
    let start = Instant::now();
    let mut edges = 0_u64;
    while start.elapsed() < TRIAL {
        sim.run_edges_sampled(piece, probes)?;
        edges += piece;
    }
    Ok(edges as f64 / start.elapsed().as_secs_f64())
}

/// The cores a thread of a settle can use at full speed: on Apple silicon
/// the performance cores (`hw.perflevel0.physicalcpu`), since a part on an
/// efficiency core takes twice as long and the settle waits for it
/// (life_full: 12 threads slower than 10 on 10 performance cores); else
/// every core.
pub(crate) fn performance_cores() -> usize {
    let all = std::thread::available_parallelism().map_or(1, |n| n.get());
    #[cfg(target_os = "macos")]
    {
        let mut n: c_int = 0;
        let mut size = std::mem::size_of::<c_int>();
        // SAFETY: a valid name, and an int with its size for the answer.
        let ok = unsafe {
            sysctlbyname(
                c"hw.perflevel0.physicalcpu".as_ptr(),
                (&mut n as *mut c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        if ok && n > 0 {
            return (n as usize).min(all);
        }
    }
    all
}

/// Wall time of a piece of a tuned run, outside a measurement.
const TUNE_PIECE: Duration = Duration::from_millis(100);
/// A program being measured runs this long to warm up (its threads wake,
/// its code comes into the caches), then this long measured.
const TUNE_WARM: Duration = Duration::from_millis(4);
const TUNE_MEASURE: Duration = Duration::from_millis(15);
/// Pieces with more edge trains than the clock's are never shorter than this
/// many cycles or edges (below it they leave the program for the host loop),
/// unless the run is.
const TUNE_MIN_PIECE: u64 = SCHEDULE_CYCLES;
/// Another program takes over when it measured this much faster. Programs
/// closer than this are not told apart reliably on a busy machine (one
/// measurement can be off by half or double, and one in turns can favour
/// the program with fewer threads), and changing between them gains little.
const TUNE_MARGIN: f64 = 1.10;

/// A program `tune` measures: other options than the running one's.
struct Candidate {
    options: FastOptions,
    /// The program, until it is first loaded.
    program: Option<FastProgram>,
    /// Its library, once compiled (at once when it is in the cache).
    library: Option<PathBuf>,
    loaded: Option<Loaded>,
    failed: Option<String>,
    /// Simulated picoseconds per wall second, smoothed over its runs, and
    /// when it last ran.
    speed: Option<f64>,
    /// Median speed of real home-program run pieces, never a short probe.
    run_speed: Option<f64>,
    measured: Option<Instant>,
    /// Its speed over that of the running program, measured in turns with
    /// it: the last few measurements.
    ratios: std::collections::VecDeque<f64>,
    /// Trials it lost (`Tuning::trial`), and until when it is not measured.
    lost: u32,
    barred: Option<Instant>,
}

impl Candidate {
    fn generated(
        program: FastProgram,
        library: Option<PathBuf>,
        known: Option<(f64, u32)>,
    ) -> Self {
        let now = Instant::now();
        Self {
            options: program.options,
            program: Some(program),
            library,
            loaded: None,
            failed: None,
            speed: None,
            run_speed: None,
            measured: known.map(|_| now),
            ratios: known.map(|k| k.0).into_iter().collect(),
            lost: known.map_or(0, |k| k.1),
            barred: known
                .filter(|k| k.1 > 0)
                .map(|k| now + Duration::from_secs(60 << k.1.min(6))),
        }
    }
}

enum TuningBuild {
    Program(Box<FastProgram>, Option<PathBuf>, Option<(f64, u32)>),
    Library(usize, std::result::Result<PathBuf, String>),
    Generated,
}

/// Apply the generation budget independently of compilation, as before.
/// A duplicate layout is not a candidate. Returning false stops abandoned work.
fn tuning_programs(
    options: Vec<FastOptions>,
    source: String,
    slots: usize,
    checks: bool,
    mut generate: impl FnMut(&FastOptions) -> Result<FastProgram>,
    mut ready: impl FnMut(FastProgram) -> bool,
) {
    let started = Instant::now();
    let mut sources = std::collections::HashSet::from([source]);
    for options in options {
        if sources.len() > 1 && !checks && started.elapsed() > TUNE_BUILD {
            break;
        }
        let Ok(program) = generate(&options) else {
            continue;
        };
        if program.place.len() == slots && sources.insert(program.source.clone()) && !ready(program)
        {
            break;
        }
    }
}

fn tuning_priority() {
    // The generator itself, as well as clang, leaves the interactive runner
    // first choice of cores. Linux nice applies to this thread and its children.
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
        }
        pthread_set_qos_class_self_np(0x11, 0); // QOS_CLASS_UTILITY
    }
    #[cfg(target_os = "linux")]
    unsafe {
        unsafe extern "C" {
            fn nice(increment: i32) -> i32;
        }
        nice(10);
    }
}

/// The median of the ratios kept.
fn median(ratios: &std::collections::VecDeque<f64>) -> Option<f64> {
    let mut r: Vec<f64> = ratios.iter().copied().collect();
    r.sort_by(f64::total_cmp);
    (!r.is_empty()).then(|| {
        let n = r.len();
        if n % 2 == 1 {
            r[n / 2]
        } else {
            0.5 * (r[n / 2 - 1] + r[n / 2])
        }
    })
}

/// A candidate takes over after this many measurements whose median says it
/// is `TUNE_MARGIN` faster; it is measured again soon until then while it is
/// at least `TUNE_CLOSE` as fast. One measurement on a busy machine can be
/// off by half or double: the threads land on other cores after each change
/// of program.
const TUNE_CONFIRM: usize = 3;
const TUNE_CLOSE: f64 = 0.85;
/// The running program's speed over its last pieces (of 50 ms or more) has
/// a spread (median absolute deviation over the median); above this only
/// programs with fewer threads are measured, once a second (a quarter of a
/// second when the running one has fallen to half its best: a loaded
/// machine that leaves fewer cores than threads can cut a settle split over
/// many threads by ten times or more, life_full on 10 threads from 95 to 4
/// kHz), and a candidate must beat the running one by twice the spread, at
/// most `TUNE_MARGIN_MOST`.
const TUNE_NOISE: f64 = 0.1;
const TUNE_MARGIN_MOST: f64 = 3.0;
/// Time on candidates: this much at first, then this share of the run.
const TUNE_ALLOWANCE: Duration = Duration::from_millis(250);
const TUNE_SHARE: f64 = 0.02;
/// The load is taken to have changed at most this often.
const TUNE_MOVE: Duration = Duration::from_secs(10);

/// A measurement of candidate `candidate` against `home`, the program
/// running before it: the candidate, then `home` again, each warmed up then
/// measured; their ratio goes to the candidate's `ratios`.
struct Probe {
    candidate: usize,
    home: usize,
    /// Whether the candidate is running, and whether the piece now is the
    /// warm-up.
    on_candidate: bool,
    warming: bool,
    /// The candidate's speed, once measured.
    candidate_speed: f64,
    warmed: Duration,
    ps: f64,
    seconds: f64,
}

/// Building candidate programs stops after this long (`tune`).
const TUNE_BUILD: Duration = Duration::from_millis(1500);
/// The first wall time of a tuned run warms up, unmeasured.
const TUNE_START: Duration = Duration::from_millis(50);
/// Form speed samples from actual home-program work, even when the window
/// asks for pieces shorter than this (its publish target is 30 ms).
const TUNE_SAMPLE: Duration = Duration::from_millis(50);
/// A clean stop can keep the measured home before all candidates are ready.
const TUNE_SAVE_ON_STOP: Duration = Duration::from_secs(2);

#[derive(Default)]
struct RunPiece {
    ps: f64,
    wall: Duration,
}

impl RunPiece {
    fn add(&mut self, ps: f64, wall: Duration) -> Option<f64> {
        if ps <= 0.0 || wall.is_zero() {
            return None;
        }
        self.ps += ps;
        self.wall += wall;
        if self.wall < TUNE_SAMPLE {
            return None;
        }
        let speed = self.ps / self.wall.as_secs_f64();
        *self = Self::default();
        Some(speed)
    }
}

struct Tuning {
    /// The running program is `candidates[current]`; its `loaded` is the
    /// simulation's own.
    candidates: Vec<Candidate>,
    current: usize,
    /// Programs generated and libraries compiled by the background worker.
    compiled: mpsc::Receiver<TuningBuild>,
    building: bool,
    probe: Option<Probe>,
    next_probe: Instant,
    /// The speed of the last piece, for the size of the next, and the wall
    /// time run so far until `TUNE_START`.
    last: Option<f64>,
    started: Duration,
    /// Measurements since the running program last changed, and its speed
    /// then: when its speed moves away from that by a fifth (the load on the
    /// machine changed), measuring starts again as often as at first.
    stable: u32,
    level: Option<f64>,
    /// Where the options chosen for this design are kept (`read_memory`).
    memory: PathBuf,
    /// Startup memory and simulated picoseconds per edge of the user's clock.
    choice_memory: Option<(PathBuf, f64)>,
    saved_choice: Option<FastOptions>,
    /// Accumulate short home pieces; probe timings never enter these samples.
    run_piece: RunPiece,
    /// Measurements made, programs that took over, times the running
    /// program's speed moved by a fifth, and the wall time run on candidates.
    probes: u32,
    takeovers: u32,
    moves: u32,
    away: Duration,
    /// Wall time of the whole tuned run, when the load last moved, and the
    /// running program's last piece speeds (pieces of 50 ms or more).
    total: Duration,
    moved: Option<Instant>,
    recent: std::collections::VecDeque<f64>,
    /// A program that has just taken over, on trial: the one before it, the
    /// median speed of that one's last pieces, and the ratio it took over
    /// with. Once the new one has run
    /// as many pieces it stays if their median is not lower, else the one
    /// before comes back. A measurement in turns is short and can favour a
    /// program; the pieces are what the run gets.
    trial: Option<(usize, f64, f64)>,
    reverts: u32,
    /// `tune_for_checks`: changes program at every piece.
    switch: bool,
    report: bool,
    /// The best median of the running program's last pieces since it took
    /// over: when it falls to half of that, the load has taken its cores.
    peak: f64,
}

impl Tuning {
    /// Summarize the accepted choice, not a temporary probe or takeover trial.
    fn summary(&self, edge_ps: Option<f64>) -> String {
        let home = self.settled();
        let start = self.candidates[0].options;
        let options = |o: FastOptions| {
            let mut text = format!(
                "{} thread{}, folding {}",
                o.threads,
                if o.threads == 1 { "" } else { "s" },
                if o.collapse > 1 {
                    o.collapse.to_string()
                } else {
                    "off".into()
                }
            );
            if o.thread_min != start.thread_min {
                text.push_str(&format!(", split from {} lines", o.thread_min));
            }
            if o.barrier_lines != start.barrier_lines {
                text.push_str(&format!(", meeting {} lines", o.barrier_lines));
            }
            text
        };
        let mut text = if home == 0 {
            format!("kept the start: {}", options(start))
        } else {
            format!(
                "switched {} -> {}",
                options(start),
                options(self.candidates[home].options)
            )
        };
        if let Some((speed, edge)) = self.candidates[home].run_speed.zip(edge_ps) {
            if speed.is_finite() && speed > 0.0 && edge > 0.0 {
                text.push_str(&format!(", {:.1} kHz", speed / edge / 2e3));
            }
        }
        if home == 0 && self.takeovers > 0 {
            text.push_str(&format!(
                "; {} switch{} tried",
                self.takeovers,
                if self.takeovers == 1 { "" } else { "es" }
            ));
        }
        text
    }

    /// Remember only a program that has run ten steady pieces: never a
    /// candidate being measured or a takeover whose trial could revert.
    /// Keeping the initial stable program also covers a guess already best.
    fn keep_choice(&mut self, index: usize, speed: f64) {
        let options = self.candidates[index].options;
        if self.switch || self.saved_choice == Some(options) {
            return;
        }
        if let Some((path, edge_ps)) = &self.choice_memory {
            let edges = speed / edge_ps;
            let mut table = read_choices(path, FastOptions::from_env());
            table.retain(|(o, _)| *o != options);
            // Accepted on real run pieces: lead with this choice rather
            // than an older option's possibly inflated short-trial speed.
            table.insert(0, (options, edges));
            write_choices(path, &table);
            self.saved_choice = Some(options);
            if self.report {
                eprintln!(
                    "fast mode tune: kept {}, {:.1} kHz; remembered for startup",
                    describe(&options),
                    edges / 2e3
                );
            }
        }
    }

    /// The start stays best once each candidate was confirmed or was clearly
    /// slower. A promising candidate still awaiting confirmation is not done.
    fn keep_start_if_ready(&mut self) {
        if self.building || self.probe.is_some() || self.trial.is_some() || self.recent.len() < 10 {
            return;
        }
        let checked = self.candidates.iter().enumerate().all(|(k, c)| {
            k == self.current
                || c.failed.is_some()
                || c.lost > 0
                || c.ratios.len() >= TUNE_CONFIRM
                || median(&c.ratios).is_some_and(|r| r < TUNE_CLOSE)
        });
        if checked {
            if let Some(speed) = self.candidates[self.current].run_speed {
                self.keep_choice(self.current, speed);
            }
        }
    }

    /// A probe or an unconfirmed takeover is not the accepted home program.
    fn settled(&self) -> usize {
        self.trial
            .map(|(old, _, _)| old)
            .or_else(|| self.probe.as_ref().map(|p| p.home))
            .unwrap_or(self.current)
    }

    /// The spread of the running program's last piece speeds (median
    /// absolute deviation over the median), once there are five.
    fn noise(&self) -> Option<f64> {
        if self.recent.len() < 5 {
            return None;
        }
        let m = median(&self.recent)?;
        let deviations: std::collections::VecDeque<f64> =
            self.recent.iter().map(|v| (v - m).abs()).collect();
        Some(median(&deviations)? / m)
    }
}

/// What a tuning memory file keeps: the options chosen, on the first line,
/// then candidates measured clearly slower than them or that lost a trial,
/// each with its ratio and its trials lost.
fn read_memory(path: &Path) -> (Option<FastOptions>, Vec<(FastOptions, f64, u32)>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, Vec::new());
    };
    let options = |w: &[&str]| -> Option<FastOptions> {
        let n: Vec<usize> = w.iter().map(|w| w.parse().ok()).collect::<Option<_>>()?;
        Some(FastOptions {
            threads: n[0],
            thread_min: n[1],
            collapse: n[2],
            barrier_lines: n[3],
        })
    };
    let mut lines = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>());
    let choice = lines
        .next()
        .filter(|w| w.len() == 4)
        .and_then(|w| options(&w));
    let slow = lines
        .filter(|w| w.len() == 6)
        .filter_map(|w| Some((options(&w[..4])?, w[4].parse().ok()?, w[5].parse().ok()?)))
        .collect();
    (choice, slow)
}

impl Drop for Tuning {
    /// Keeps the choice, and the candidates clearly slower than it, for the
    /// next run of this design.
    fn drop(&mut self) {
        if self.switch {
            return;
        }
        let home = self.settled();
        if self.saved_choice.is_none() && self.total >= TUNE_SAVE_ON_STOP {
            if let Some(speed) = self.candidates[home].run_speed {
                self.keep_choice(home, speed);
            }
        }
        let line = |o: &FastOptions| {
            format!(
                "{} {} {} {}",
                o.threads, o.thread_min, o.collapse, o.barrier_lines
            )
        };
        let mut text = format!("{}\n", line(&self.candidates[home].options));
        let scale = self.trial.map_or(1.0, |(_, _, ratio)| ratio);
        for (k, c) in self.candidates.iter().enumerate() {
            match median(&c.ratios).map(|r| r * scale) {
                Some(r) if k != home && (r < 0.6 || c.lost > 0) => {
                    text.push_str(&format!("{} {r:.3} {}\n", line(&c.options), c.lost))
                }
                _ => {}
            }
        }
        let _ = std::fs::write(&self.memory, text);
    }
}

fn describe(o: &FastOptions) -> String {
    format!(
        "{} thread{}, split from {} lines, folding {}, meeting {} lines",
        o.threads,
        if o.threads == 1 { "" } else { "s" },
        o.thread_min,
        if o.collapse > 1 {
            format!("to {} inputs", o.collapse)
        } else {
            "off".into()
        },
        o.barrier_lines
    )
}

impl FastSim {
    /// Connect the in-run tuner's accepted choices to the default program's
    /// startup memory. Set this before `tune`, after any startup replacement.
    pub fn set_choice_memory(&mut self, path: PathBuf) {
        self.choice_memory = Some(path);
    }

    /// The current time in picoseconds, for speeds.
    fn now_ps(&self) -> f64 {
        let (n, d) = self.tick.ratio();
        self.now as f64 * n as f64 / d as f64
    }
    /// Measures, as the design runs, programs of `kernel` (the kernel this
    /// simulation was built from) with options other than the running one's,
    /// and runs the fastest; it goes on measuring them now and then, so a
    /// change of load on the machine changes the choice
    /// (docs/FAST_MODE.md, *Tuning*).
    ///
    /// The candidates are the running options with one changed: the threads
    /// (1, 2, 3, 4, 6, 8, up to the cores; none above 1 when no settle has
    /// 300 lines), the folding width (0, 6, 8, 12), the fewest lines split
    /// when a smaller settle exists, the meeting cost when a settle is split.
    /// Options the environment sets stay as they are. Libraries not in the
    /// cache generate and compile in the background, one at a time at a low
    /// priority. If the input snapshot cannot be allocated, generation instead
    /// borrows the live kernel synchronously; compilation stays in the background.
    /// Measured in turns with the running program, a candidate clearly faster
    /// runs on trial and stays if its own pieces are not slower. What a run
    /// chose is kept for the next run of the design. `FAST_TUNE=0` turns
    /// tuning off; with `FAST_THREAD_REPORT` every change is printed.
    pub fn tune(&mut self, kernel: &Kernel) -> Result<()> {
        if std::env::var("FAST_TUNE").is_ok_and(|m| m.trim() == "0") {
            return Ok(());
        }
        self.start_tuning(kernel, false)
    }
    /// As `tune`, for the checks: the candidates (with settles split across 2
    /// and 4 threads however small they are, too) compile now, and every
    /// piece of a run (97 cycles or edges) goes to the next program, so a
    /// trace shows whether it depends on the program. Returns how many
    /// programs take turns.
    pub fn tune_for_checks(&mut self, kernel: &Kernel) -> Result<usize> {
        self.start_tuning(kernel, true)?;
        let flags = compile_flags();
        let t = self.tuning.as_mut().context("not tuning")?;
        for c in &mut t.candidates {
            if let (Some(program), None) = (&c.program, &c.library) {
                match compile(&program.source, &flags, false) {
                    Ok(library) => c.library = Some(library),
                    Err(e) => c.failed = Some(format!("{e:#}")),
                }
            }
        }
        Ok(1 + t.candidates.iter().filter(|c| c.library.is_some()).count())
    }
    fn start_tuning(&mut self, kernel: &Kernel, switch: bool) -> Result<()> {
        if self.tuning.is_some() {
            return Ok(());
        }
        let report = std::env::var_os("FAST_THREAD_REPORT").is_some();
        let flags = compile_flags();
        let home = self.loaded.program.options;
        let (n, d) = self.tick.ratio();
        let choice_memory = self.choice_memory.clone().zip(
            self.clock
                .map(|k| self.waves[k].half as f64 * n as f64 / d as f64),
        );
        let key = cache_paths(&self.loaded.program.source, &flags)?.2;
        let memory = std::env::temp_dir()
            .join("ecp5_emu_fast")
            .join(format!("{key}.tune"));
        let (remembered, slow) = read_memory(&memory);
        let pinned = FastOptions::pinned();
        let cores = performance_cores();
        let lines = &self.loaded.program.settle_lines;
        let largest = lines.iter().copied().max().unwrap_or(0);
        // The likeliest first: half the threads (where a loaded machine
        // sends a large design), a thread more or fewer, every performance
        // core, then folding a size smaller, one thread, the rest. Building
        // a program of a design of 100 000 lines takes about a second, so
        // building stops after `TUNE_BUILD` (at least one is built); those
        // not measured on an earlier run come before those that were, so
        // further runs of a large design get to the rest.
        // Around the choice of the last run, which runs from the start when
        // it is ready, else around the running options.
        let anchor = remembered.unwrap_or(home);
        let mut options: Vec<FastOptions> = Vec::new();
        let threads = |t: usize| {
            (t <= cores && (t == 1 || largest >= 300)).then_some(FastOptions { threads: t, ..home })
        };
        let collapse = |c: usize| FastOptions {
            collapse: c,
            ..home
        };
        if !pinned[0] {
            let half = (anchor.threads >= 4).then_some(anchor.threads / 2);
            options.extend(
                half.into_iter()
                    .chain([
                        anchor.threads.saturating_sub(1).max(1),
                        anchor.threads + 1,
                        cores,
                    ])
                    .filter_map(threads),
            );
        }
        if !pinned[2] {
            options.push(collapse(home.collapse.saturating_sub(2)));
        }
        if !pinned[0] {
            options.extend(threads(1));
        }
        if !pinned[2] {
            options.extend([0, 6, 8, 12].map(collapse));
        }
        if !pinned[0] {
            options.extend([2, 3, 4, 6, 8, 10, 12, 16].into_iter().filter_map(threads));
        }
        if !pinned[1] && home.threads > 1 {
            if let Some(&m) = lines
                .iter()
                .filter(|&&l| (300..home.thread_min).contains(&l))
                .min()
            {
                options.push(FastOptions {
                    thread_min: m,
                    ..home
                });
            }
        }
        if !pinned[3] && home.threads > 1 {
            let phases = &self.loaded.program.settle_phases;
            if phases.contains(&2) {
                options.push(FastOptions {
                    barrier_lines: 1 << 30,
                    ..home
                });
            } else if phases.contains(&1) {
                options.push(FastOptions {
                    barrier_lines: home.barrier_lines / 3,
                    ..home
                });
            }
        }
        if switch {
            // The checks split even small settles, so that every design has
            // programs of other layouts to change to.
            for t in [2, 4] {
                options.push(FastOptions {
                    threads: t,
                    thread_min: 1,
                    ..home
                });
            }
        }
        let (known, unknown): (Vec<FastOptions>, Vec<FastOptions>) = options
            .into_iter()
            .partition(|o| slow.iter().any(|(s, ..)| s == o));
        let mut seen = std::collections::HashSet::from([home]);
        let options: Vec<FastOptions> = remembered
            .into_iter()
            .chain(unknown)
            .chain(known)
            .filter(|o| seen.insert(*o))
            .collect();
        let have_candidates = !options.is_empty();
        let mut candidates = vec![Candidate {
            options: home,
            program: None,
            library: None,
            loaded: None,
            failed: None,
            speed: None,
            run_speed: None,
            measured: None,
            ratios: Default::default(),
            lost: 0,
            barred: None,
        }];
        let (send, compiled) = mpsc::channel();
        let slots = self.loaded.place.len();
        // An empty round still measures the running program so its honest
        // speed can be remembered, without copying generation inputs or source.
        let source = if have_candidates {
            self.loaded.program.source.clone()
        } else {
            String::new()
        };
        let mut building = false;
        if switch {
            tuning_programs(
                options,
                source,
                slots,
                true,
                |o| kernel.fast_program_with(o),
                |program| {
                    candidates.push(Candidate::generated(program, None, None));
                    true
                },
            );
        } else if have_candidates {
            match kernel.fast_generator() {
                Ok(generator) => {
                    building = true;
                    std::thread::Builder::new()
                        .name("fast mode compile".into())
                        .spawn(move || {
                            tuning_priority();
                            let mut to_compile = Vec::new();
                            let mut index = 1;
                            tuning_programs(
                                options,
                                source,
                                slots,
                                false,
                                |o| generator.program(o),
                                |program| {
                                    let library =
                                        cache_paths(&program.source, &flags).ok().map(|p| p.1);
                                    let library = library.filter(|p| p.exists());
                                    if library.is_none() {
                                        to_compile.push((index, program.source.clone()));
                                    }
                                    let known = slow
                                        .iter()
                                        .find(|(o, ..)| *o == program.options)
                                        .map(|&(_, ratio, lost)| (ratio, lost));
                                    index += 1;
                                    send.send(TuningBuild::Program(
                                        Box::new(program),
                                        library,
                                        known,
                                    ))
                                    .is_ok()
                                },
                            );
                            // Free the copied inputs before announcing generation's
                            // completion or compiling a library. They never live in Tuning.
                            drop(generator);
                            if send.send(TuningBuild::Generated).is_err() {
                                return;
                            }
                            for (index, source) in to_compile {
                                let library =
                                    compile(&source, &flags, true).map_err(|e| format!("{e:#}"));
                                if send.send(TuningBuild::Library(index, library)).is_err() {
                                    break;
                                }
                            }
                        })?;
                }
                Err(error) => {
                    if report {
                        eprintln!("fast mode tune: snapshot allocation failed ({error}); generating candidates synchronously");
                    }
                    // The failed snapshot's partial buffers are already gone.
                    // Borrow the live kernel as before background generation;
                    // only compilation needs owned source text on its worker.
                    let mut to_compile = Vec::new();
                    tuning_programs(
                        options,
                        source,
                        slots,
                        false,
                        |o| kernel.fast_program_with(o),
                        |program| {
                            let library = cache_paths(&program.source, &flags)
                                .ok()
                                .map(|p| p.1)
                                .filter(|p| p.exists());
                            if library.is_none() {
                                to_compile.push((candidates.len(), program.source.clone()));
                            }
                            let known = slow
                                .iter()
                                .find(|(o, ..)| *o == program.options)
                                .map(|&(_, ratio, lost)| (ratio, lost));
                            candidates.push(Candidate::generated(program, library, known));
                            true
                        },
                    );
                    if !to_compile.is_empty() {
                        std::thread::Builder::new()
                            .name("fast mode compile".into())
                            .spawn(move || {
                                tuning_priority();
                                for (index, source) in to_compile {
                                    let library = compile(&source, &flags, true)
                                        .map_err(|e| format!("{e:#}"));
                                    if send.send(TuningBuild::Library(index, library)).is_err() {
                                        break;
                                    }
                                }
                            })?;
                    }
                }
            }
        }
        if report {
            eprintln!(
                "fast mode tune: running {}; candidates {}",
                describe(&home),
                if !have_candidates {
                    "none to build; measuring the running program"
                } else if building {
                    "generate and compile in the background"
                } else if switch {
                    "ready for checks"
                } else {
                    "generated synchronously; libraries compile in the background"
                }
            );
        }
        self.tuning = Some(Box::new(Tuning {
            candidates,
            current: 0,
            compiled,
            building,
            probe: None,
            next_probe: Instant::now(),
            last: None,
            started: Duration::ZERO,
            probes: 0,
            takeovers: 0,
            moves: 0,
            away: Duration::ZERO,
            total: Duration::ZERO,
            moved: None,
            recent: Default::default(),
            trial: None,
            reverts: 0,
            stable: 0,
            level: None,
            memory,
            choice_memory,
            saved_choice: None,
            run_piece: RunPiece::default(),
            switch,
            report,
            peak: 0.0,
        }));
        Ok(())
    }
    /// The accepted options and their median speed on real run pieces. A
    /// pending probe or unconfirmed takeover is not reported as the choice.
    pub fn tuning_summary(&self) -> Option<String> {
        let t = self.tuning.as_ref()?;
        let (n, d) = self.tick.ratio();
        let edge_ps = self
            .clock
            .map(|k| self.waves[k].half as f64 * n as f64 / d as f64);
        Some(t.summary(edge_ps))
    }

    /// What tuning found: the options running and the speed measured for
    /// each program, fastest first.
    pub fn tuning_report(&self) -> Option<String> {
        let t = self.tuning.as_ref()?;
        let ratio = |k: usize| {
            if k == t.current {
                Some(1.0)
            } else {
                median(&t.candidates[k].ratios)
            }
        };
        let mut rows: Vec<usize> = (0..t.candidates.len()).collect();
        rows.sort_by(|&a, &b| ratio(b).unwrap_or(0.0).total_cmp(&ratio(a).unwrap_or(0.0)));
        let mut text = format!(
            "running {}; {} measurements, {} changes of program ({} taken back), {} moves of its speed, {:.2} s of {:.2} on others, spread of its speed {}; the others, by their speed over its (median of the last measurements):",
            describe(&t.candidates[t.current].options),
            t.probes,
            t.takeovers,
            t.reverts,
            t.moves,
            t.away.as_secs_f64(),
            t.total.as_secs_f64(),
            t.noise().map_or("unknown".into(), |n| format!("{:.1}%", 100.0 * n))
        );
        if t.building {
            text.push_str("\n  generating candidates in the background");
        }
        for k in rows {
            if k == t.current {
                continue;
            }
            let c = &t.candidates[k];
            let speed = match (&c.failed, ratio(k), &c.library, c.program.is_some()) {
                (Some(e), ..) => format!("failed: {}", e.lines().next().unwrap_or("")),
                (None, Some(r), ..) => format!("{:.0}% ({} measured)", 100.0 * r, c.ratios.len()),
                (None, None, None, true) => "compiling".into(),
                _ => "not measured".into(),
            };
            text.push_str(&format!("\n  {}: {speed}", describe(&c.options)));
        }
        Some(text)
    }
    /// Takes in libraries compiled since and starts a measurement when one is
    /// due; returns how long the next piece should run.
    fn tune_plan(&mut self) -> Result<Duration> {
        let now = Instant::now();
        let t = self.tuning.as_mut().context("not tuning")?;
        while let Ok(build) = t.compiled.try_recv() {
            match build {
                TuningBuild::Program(program, library, known) => {
                    t.candidates
                        .push(Candidate::generated(*program, library, known));
                }
                TuningBuild::Library(index, Ok(path)) => t.candidates[index].library = Some(path),
                TuningBuild::Library(index, Err(e)) => t.candidates[index].failed = Some(e),
                TuningBuild::Generated => t.building = false,
            }
        }
        if t.switch {
            // Every piece on the next program that can run.
            let n = t.candidates.len();
            let next = (1..n).map(|k| (t.current + k) % n).find(|&k| {
                t.candidates[k].failed.is_none()
                    && (t.candidates[k].loaded.is_some() || t.candidates[k].library.is_some())
            });
            if let Some(k) = next {
                self.switch_to(k)?;
            }
            return Ok(Duration::ZERO);
        }
        if let Some(p) = &t.probe {
            let ms = Duration::from_millis(2);
            return Ok(match p.warming {
                true => TUNE_WARM.saturating_sub(p.warmed).max(ms),
                false => TUNE_MEASURE
                    .saturating_sub(Duration::from_secs_f64(p.seconds))
                    .max(ms),
            });
        }
        if t.started < TUNE_START {
            return Ok(Duration::from_millis(10));
        }
        if now < t.next_probe {
            return Ok((t.next_probe - now).min(TUNE_PIECE));
        }
        let home = t.current;
        if t.trial.is_some() {
            return Ok(TUNE_PIECE);
        }
        let Some(home_speed) = t.candidates[home].speed else {
            // The running program is measured first, on its own pieces.
            return Ok(TUNE_PIECE / 4);
        };
        // Not measured enough yet, or enough time spent on others already.
        let budget = TUNE_ALLOWANCE.as_secs_f64() + TUNE_SHARE * t.total.as_secs_f64();
        let Some(noise) = t.noise().filter(|_| t.away.as_secs_f64() <= budget) else {
            t.next_probe = now + TUNE_PIECE;
            return Ok(TUNE_PIECE);
        };
        // Too noisy to tell close programs apart: only fewer threads, which
        // a loaded machine may run many times faster.
        let noisy = noise > TUNE_NOISE;
        let fallen = median(&t.recent).is_some_and(|m| m < 0.5 * t.peak);
        let home_threads = t.candidates[home].options.threads;
        // What to measure: a program not measured yet, else the one most
        // overdue, a slow one seldom; the more measurements have not
        // changed the program, the seldomer.
        let backoff = 2f64.powi(t.stable.min(4) as i32);
        let mut pick: Option<(usize, f64)> = None;
        let mut unmeasured = false;
        for (k, c) in t.candidates.iter().enumerate() {
            if k == home
                || c.failed.is_some()
                || (c.loaded.is_none() && c.library.is_none())
                || c.barred.is_some_and(|until| now < until)
                || (noisy && c.options.threads >= home_threads)
            {
                continue;
            }
            let _ = home_speed;
            let due = match (c.measured, median(&c.ratios)) {
                (Some(at), Some(r)) => {
                    let age = now.duration_since(at).as_secs_f64();
                    if c.ratios.len() < TUNE_CONFIRM && r >= TUNE_CLOSE {
                        unmeasured = true;
                        age / 0.25
                    } else {
                        age / (if r < 0.6 { 30.0 } else { 2.0 } * backoff)
                    }
                }
                _ => {
                    unmeasured = true;
                    f64::INFINITY
                }
            };
            if due >= 1.0 && pick.is_none_or(|(_, d)| due > d) {
                pick = Some((k, due));
            }
        }
        t.next_probe = now
            + Duration::from_millis(match (noisy, fallen) {
                (true, true) => 250,
                (true, false) => 1000,
                (false, _) if unmeasured => 50,
                _ => 250,
            });
        let Some((k, _)) = pick else {
            return Ok(TUNE_PIECE);
        };
        t.probe = Some(Probe {
            candidate: k,
            home,
            on_candidate: true,
            warming: true,
            candidate_speed: 0.0,
            warmed: Duration::ZERO,
            ps: 0.0,
            seconds: 0.0,
        });
        if let Err(e) = self.switch_to(k) {
            let t = self.tuning.as_mut().context("not tuning")?;
            t.candidates[k].failed = Some(format!("{e:#}"));
            t.probe = None;
            return Ok(TUNE_PIECE);
        }
        Ok(TUNE_WARM)
    }
    /// The cycles or edges of the next piece of a tuned run of `left` more.
    fn tune_chunk(&mut self, left: u64, every: bool) -> Result<u64> {
        let wall = self.tune_plan()?;
        let t = self.tuning.as_ref().context("not tuning")?;
        let c = self.clock.context("start a clock first")?;
        let (n, d) = self.tick.ratio();
        let unit = self.waves[c].half as f64 * n as f64 / d as f64 * if every { 1.0 } else { 2.0 };
        // Only the clock: any number of cycles runs inside the program.
        let least = match self.waves.len() == 1 && self.plls.is_empty() {
            true => 64,
            false => TUNE_MIN_PIECE,
        };
        let mut chunk = match (t.switch, t.last) {
            (true, _) => 97,
            (false, Some(v)) => ((v * wall.as_secs_f64() / unit) as u64).max(least),
            (false, None) => least,
        };
        if every {
            chunk += chunk & 1;
        }
        Ok(chunk.min(left).max(1))
    }
    /// The ticks of the next piece of a tuned `run_until` of `left` more.
    fn tune_ticks(&mut self, left: u64) -> Result<u64> {
        let wall = self.tune_plan()?;
        let t = self.tuning.as_ref().context("not tuning")?;
        let (n, d) = self.tick.ratio();
        let ps_per_tick = n as f64 / d as f64;
        let ticks = match (t.switch, t.last) {
            (false, Some(v)) => (v * wall.as_secs_f64() / ps_per_tick) as u64,
            _ => left / 64,
        };
        Ok(ticks.clamp(1, left))
    }
    /// Takes the speed of a piece that ran `ps` of simulated time in `wall`,
    /// and moves a measurement on.
    fn tune_ran(&mut self, ps: f64, wall: Duration) -> Result<()> {
        let now = Instant::now();
        let t = self.tuning.as_mut().context("not tuning")?;
        let seconds = wall.as_secs_f64();
        t.total += wall;
        if seconds > 0.0 && ps > 0.0 {
            t.last = Some(ps / seconds);
        }
        let Some(p) = t.probe.as_mut() else {
            if t.started < TUNE_START {
                t.started += wall;
            } else if let Some(v) = t.run_piece.add(ps, wall) {
                let c = &mut t.candidates[t.current];
                let speed = c.speed.map_or(v, |s| 0.7 * s + 0.3 * v);
                c.speed = Some(speed);
                c.measured = Some(now);
                t.recent.push_back(v);
                if t.recent.len() > 10 {
                    t.recent.pop_front();
                }
                c.run_speed = median(&t.recent);
                if t.recent.len() >= 3 {
                    t.peak = t.peak.max(median(&t.recent).unwrap_or(0.0));
                }
                if let (Some((old, before, r)), 10) = (t.trial, t.recent.len()) {
                    t.trial = None;
                    let after = median(&t.recent).unwrap_or(0.0);
                    let new = t.current;
                    if after < 0.98 * before {
                        t.reverts += 1;
                        if t.report {
                            eprintln!(
                                "fast mode tune: {} ran at {:.0}% of the speed of {} over its first pieces: that one comes back",
                                describe(&t.candidates[new].options),
                                100.0 * after / before,
                                describe(&t.candidates[old].options)
                            );
                        }
                        // Measured in turns it looked faster, so it would
                        // be put forward again: it waits, longer each time.
                        for c in &mut t.candidates {
                            for x in &mut c.ratios {
                                *x *= r;
                            }
                        }
                        let c = &mut t.candidates[new];
                        c.ratios = [after / before].into();
                        c.lost += 1;
                        c.barred = Some(now + Duration::from_secs(60 << c.lost.min(6)));
                        t.candidates[old].ratios.clear();
                        t.recent.clear();
                        t.run_piece = RunPiece::default();
                        t.peak = 0.0;
                        t.stable += 1;
                        return self.switch_to(old);
                    }
                    t.keep_choice(new, after);
                }
                t.keep_start_if_ready();
                let quiet = t.moved.is_none_or(|at| now.duration_since(at) >= TUNE_MOVE);
                match t.level {
                    Some(level) if quiet && (speed / level - 1.0).abs() > 0.2 => {
                        t.stable = 0;
                        t.level = Some(speed);
                        t.moves += 1;
                        t.moved = Some(now);
                    }
                    None => t.level = Some(speed),
                    _ => {}
                }
            }
            return Ok(());
        };
        if p.on_candidate {
            t.away += wall;
        }
        if p.warming {
            p.warmed += wall;
            if p.warmed >= TUNE_WARM {
                p.warming = false;
            }
            return Ok(());
        }
        p.ps += ps;
        p.seconds += seconds;
        if p.seconds < TUNE_MEASURE.as_secs_f64() {
            return Ok(());
        }
        let (k, home, v) = (p.candidate, p.home, p.ps / p.seconds);
        (p.warming, p.warmed, p.ps, p.seconds) = (true, Duration::ZERO, 0.0, 0.0);
        if p.on_candidate {
            // The candidate measured: now the program before it again.
            p.candidate_speed = v;
            p.on_candidate = false;
            return self.switch_to(home);
        }
        let ratio = p.candidate_speed / v;
        t.probe = None;
        t.probes += 1;
        let margin = TUNE_MARGIN
            .max(1.0 + 2.0 * t.noise().unwrap_or(1.0))
            .min(TUNE_MARGIN_MOST);
        let running = &mut t.candidates[home];
        running.speed = Some(running.speed.map_or(v, |s| 0.5 * s + 0.5 * v));
        running.measured = Some(now);
        let c = &mut t.candidates[k];
        c.speed = Some(c.speed.map_or(v * ratio, |s| 0.5 * s + 0.5 * v * ratio));
        c.measured = Some(now);
        c.ratios.push_back(ratio);
        if c.ratios.len() > 5 {
            c.ratios.pop_front();
        }
        let n = c.ratios.len();
        let r = median(&c.ratios).unwrap_or(0.0);
        let above = c
            .ratios
            .iter()
            .rev()
            .take(TUNE_CONFIRM)
            .filter(|&&x| x > margin)
            .count();
        if n < TUNE_CONFIRM || r <= margin || above + 1 < TUNE_CONFIRM {
            if n >= TUNE_CONFIRM || r < TUNE_CLOSE {
                t.stable += 1;
            }
            t.keep_start_if_ready();
            return Ok(());
        }
        t.stable = 0;
        t.level = None;
        t.takeovers += 1;
        if t.report {
            eprintln!(
                "fast mode tune: {} runs at {:.0}% of the speed of {} (median of {n}): it takes over",
                describe(&t.candidates[k].options),
                100.0 * r,
                describe(&t.candidates[home].options)
            );
        }
        // Every ratio was to the program that stops now: over the new one
        // they are about r times smaller; the old one's is 1/r.
        for c in &mut t.candidates {
            for x in &mut c.ratios {
                *x /= r;
            }
        }
        t.candidates[k].ratios.clear();
        t.trial = Some((home, median(&t.recent).unwrap_or(0.0), r));
        t.recent.clear();
        t.run_piece = RunPiece::default();
        t.peak = 0.0;
        t.candidates[home].ratios = [1.0 / r].into();
        self.switch_to(k)
    }
    /// Runs candidate `k` from the current state on.
    fn switch_to(&mut self, k: usize) -> Result<()> {
        let t = self.tuning.as_mut().context("not tuning")?;
        if k == t.current {
            return Ok(());
        }
        let c = &mut t.candidates[k];
        let loaded = match c.loaded.take() {
            Some(l) => l,
            None => {
                let program = c.program.take().context("program already used")?;
                let library = c.library.clone().context("not compiled yet")?;
                open(program, &library)?
            }
        };
        let old = std::mem::replace(&mut self.loaded, loaded);
        self.values = carry(&self.values, &old.place, &self.loaded);
        for wave in &mut self.waves {
            wave.slot = carry_slot(wave.slot, &old.place, &self.loaded.place);
        }
        let t = self.tuning.as_mut().context("not tuning")?;
        let previous = t.current;
        t.candidates[previous].loaded = Some(old);
        t.current = k;
        // Bring the nets this program folds differently up to date.
        self.step()
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::{
        check_compiler, compiler_identity_with, read_choices, starting_options, units_of,
        write_choices, Candidate, FastOptions, Prepared, Probe, RunPiece, Tuning,
    };
    use std::{
        collections::HashMap,
        ffi::OsStr,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
        time::{Duration, Instant},
    };

    #[test]
    fn compiler_identity_is_probed_once_per_value_even_concurrently() {
        let identities = Mutex::new(HashMap::new());
        for (compiler, expected) in [
            (
                "clang",
                Some("clang version\nInstalledDir: /compiler/one\n"),
            ),
            (
                "/compiler with spaces",
                Some("clang version\nzig version two\n"),
            ),
            ("missing-compiler", None),
        ] {
            let calls = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let (identities, calls) = (&identities, &calls);
                    scope.spawn(move || {
                        for _ in 0..3 {
                            let identity =
                                compiler_identity_with(OsStr::new(compiler), identities, |value| {
                                    assert_eq!(value, compiler);
                                    calls.fetch_add(1, Ordering::Relaxed);
                                    expected.map(String::from)
                                });
                            assert_eq!(identity, expected.unwrap_or_default());
                        }
                    });
                }
            });
            assert_eq!(calls.load(Ordering::Relaxed), 1, "{compiler}");
        }
    }

    #[test]
    fn compiler_check_handles_paths_and_missing_compilers_without_env_changes() {
        let dir = std::env::temp_dir().join(format!("ecp5 compiler check {}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let clang = dir.join("clang");
        let missing = dir.join("missing compiler");
        std::fs::write(&clang, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&clang, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = std::env::join_paths([&missing, &dir]).unwrap();
        let unavailable =
            "fast mode needs a C compiler: install clang on PATH or set ECP5_CLANG to one";
        for path in [None, Some(dir.as_os_str())] {
            assert_eq!(
                check_compiler(None, path).unwrap_err().to_string(),
                unavailable
            );
        }
        for value in [&missing, &clang, &dir] {
            assert_eq!(
                check_compiler(Some(value.as_os_str()), Some(&path))
                    .unwrap_err()
                    .to_string(),
                format!(
                    "fast mode: ECP5_CLANG={} is not an executable file",
                    value.display()
                )
            );
        }
        std::fs::set_permissions(&clang, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            check_compiler(Some(clang.as_os_str()), None).unwrap(),
            clang.as_os_str()
        );
        for name in ["clang-21", "compiler with spaces"] {
            std::fs::copy(&clang, dir.join(name)).unwrap();
            assert_eq!(
                check_compiler(Some(OsStr::new(name)), Some(&path)).unwrap(),
                name
            );
        }
        assert_eq!(
            check_compiler(Some(OsStr::new("missing-compiler")), Some(&path))
                .unwrap_err()
                .to_string(),
            "fast mode: ECP5_CLANG=missing-compiler is not an executable file"
        );
        for value in [None, Some(OsStr::new(""))] {
            assert_eq!(check_compiler(value, Some(&path)).unwrap(), "clang");
            assert_eq!(
                check_compiler(value, None).unwrap_err().to_string(),
                unavailable
            );
        }
        // An invalid override must not silently fall back to a working PATH.
        assert!(check_compiler(Some(missing.as_os_str()), Some(&path)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_guess_uses_raw_gate_count_and_respects_pinned_threads() {
        let base = FastOptions {
            threads: 3,
            thread_min: 2000,
            collapse: 12,
            barrier_lines: 300,
        };
        for (gates, cores, threads) in [
            (1999, 12, 1),
            (2000, 12, 3),
            (15_225, 12, 3),
            (49_999, 12, 3),
            (50_000, 12, 10),
            (108_444, 8, 8),
        ] {
            assert_eq!(
                starting_options(gates, base, false, cores),
                FastOptions { threads, ..base }
            );
        }
        assert_eq!(starting_options(0, base, true, 12), base);
        assert_eq!(starting_options(108_444, base, true, 12), base);
    }

    #[test]
    fn short_run_pieces_make_time_weighted_speed_samples() {
        let mut piece = RunPiece::default();
        assert_eq!(piece.add(10.0, Duration::from_millis(10)), None);
        assert_eq!(piece.add(90.0, Duration::from_millis(30)), None);
        // 200 ps in 60 ms, rather than an unweighted mean of the speeds.
        assert_eq!(
            piece.add(100.0, Duration::from_millis(20)),
            Some(200.0 / 0.060)
        );
        assert_eq!(piece.add(0.0, Duration::from_secs(1)), None);
        assert_eq!(piece.add(100.0, Duration::ZERO), None);
        assert_eq!(piece.add(50.0, Duration::from_millis(50)), Some(1000.0));
    }

    fn memory_tuning(label: &str) -> (Tuning, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("ecp5-choice-{}-{label}.choice", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let options = FastOptions {
            threads: 3,
            thread_min: 2000,
            collapse: 12,
            barrier_lines: 300,
        };
        let candidates = [3, 4]
            .into_iter()
            .map(|threads| Candidate {
                options: FastOptions { threads, ..options },
                program: None,
                library: None,
                loaded: None,
                failed: None,
                speed: Some(1e12),
                run_speed: (threads == 3).then_some(2e9),
                measured: None,
                ratios: Default::default(),
                lost: 0,
                barred: None,
            })
            .collect();
        let (_, compiled) = std::sync::mpsc::channel();
        let tuning = Tuning {
            candidates,
            current: 0,
            compiled,
            building: false,
            probe: None,
            next_probe: Instant::now(),
            last: None,
            started: Duration::ZERO,
            stable: 0,
            level: None,
            memory: path.with_extension("tune"),
            choice_memory: Some((path.clone(), 20_000.0)),
            saved_choice: None,
            run_piece: RunPiece::default(),
            probes: 0,
            takeovers: 0,
            moves: 0,
            away: Duration::ZERO,
            total: Duration::from_secs(3),
            moved: None,
            recent: [2e9; 10].into(),
            trial: None,
            reverts: 0,
            switch: false,
            report: false,
            peak: 2e9,
        };
        (tuning, path)
    }

    #[test]
    fn tuning_summary_uses_the_accepted_choice_and_real_run_speed() {
        let (mut t, _) = memory_tuning("summary");
        assert_eq!(
            t.summary(Some(20_000.0)),
            "kept the start: 3 threads, folding 12, 50.0 kHz"
        );
        t.current = 1;
        t.candidates[1].run_speed = Some(4e9);
        t.probe = Some(Probe {
            candidate: 1,
            home: 0,
            on_candidate: true,
            warming: false,
            candidate_speed: 1e12,
            warmed: Duration::ZERO,
            ps: 0.0,
            seconds: 0.0,
        });
        assert_eq!(
            t.summary(Some(20_000.0)),
            "kept the start: 3 threads, folding 12, 50.0 kHz"
        );
        t.probe = None;
        t.takeovers = 1;
        t.trial = Some((0, 2e9, 2.0));
        assert_eq!(
            t.summary(Some(20_000.0)),
            "kept the start: 3 threads, folding 12, 50.0 kHz; 1 switch tried"
        );
        t.trial = None;
        t.candidates[1].options = FastOptions {
            threads: 1,
            collapse: 0,
            thread_min: 512,
            barrier_lines: 1000,
        };
        assert_eq!(t.summary(Some(20_000.0)),
            "switched 3 threads, folding 12 -> 1 thread, folding off, split from 512 lines, meeting 1000 lines, 100.0 kHz");
        t.current = 0;
        t.takeovers = 2;
        t.candidates[0].run_speed = None;
        assert_eq!(
            t.summary(Some(20_000.0)),
            "kept the start: 3 threads, folding 12; 2 switches tried"
        );
        // This fixture owns no compiled program and need not persist a run.
        t.switch = true;
    }

    fn assert_home_memory(path: &std::path::Path) {
        let rows = read_choices(path, FastOptions::from_env());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.threads, 3);
        assert_eq!(
            rows[0].1, 100_000.0,
            "use home run speed, not the inflated probe speed"
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(path.with_extension("tune")).unwrap();
    }

    #[test]
    fn starting_program_is_kept_after_the_first_candidate_round() {
        let (mut t, path) = memory_tuning("round");
        t.keep_start_if_ready();
        assert!(!path.exists(), "wait for the compiling candidate");
        t.candidates[1].ratios = [0.95].into();
        t.keep_start_if_ready();
        assert!(!path.exists(), "wait for confirmation of a close candidate");
        t.candidates[1].ratios = [0.95; 3].into();
        t.building = true;
        t.keep_start_if_ready();
        assert!(!path.exists(), "wait for generation to finish");
        t.building = false;
        t.keep_start_if_ready();
        assert!(path.exists(), "write while running, without a takeover");
        drop(t);
        assert_home_memory(&path);
    }

    #[test]
    fn clean_stop_keeps_a_measured_start_with_candidates_still_pending() {
        let (t, path) = memory_tuning("stop");
        drop(t);
        assert_home_memory(&path);
    }

    #[test]
    fn starting_program_is_kept_after_a_replacement_was_rejected() {
        let (mut t, path) = memory_tuning("rejected");
        t.candidates[1].ratios = [0.99].into();
        t.candidates[1].lost = 1;
        t.keep_start_if_ready();
        assert!(path.exists());
        drop(t);
        assert_home_memory(&path);
    }

    #[test]
    fn clean_stop_never_promotes_a_probe_or_unconfirmed_takeover() {
        for (label, trial) in [("probe", false), ("trial", true)] {
            let (mut t, path) = memory_tuning(label);
            t.current = 1;
            t.candidates[1].run_speed = Some(3e9);
            if trial {
                t.trial = Some((0, 2e9, 1.5));
            } else {
                t.probe = Some(Probe {
                    candidate: 1,
                    home: 0,
                    on_candidate: true,
                    warming: false,
                    candidate_speed: 1e12,
                    warmed: Duration::ZERO,
                    ps: 0.0,
                    seconds: 0.0,
                });
            }
            drop(t);
            assert_home_memory(&path);
        }
    }

    #[test]
    fn clean_stop_needs_two_seconds_and_a_real_run_speed() {
        for label in ["brief", "unmeasured"] {
            let (mut t, path) = memory_tuning(label);
            if label == "brief" {
                t.total = Duration::from_millis(1900);
            } else {
                t.candidates[0].run_speed = None;
            }
            drop(t);
            assert!(!path.exists());
            std::fs::remove_file(path.with_extension("tune")).unwrap();
        }
    }

    #[test]
    fn choice_memory_keeps_all_tuned_options_and_reads_older_choices() {
        let path = std::env::temp_dir().join(format!(
            "ecp5_emu_choice_format_test_{}.choice",
            std::process::id()
        ));
        let base = FastOptions::from_env();
        let tuned = FastOptions {
            threads: 9,
            thread_min: 512,
            collapse: 8,
            barrier_lines: 1000,
        };
        write_choices(&path, &[(tuned, 123_456.0)]);
        assert_eq!(read_choices(&path, base), vec![(tuned, 123_456.0)]);
        std::fs::write(&path, "3 12 654321\n3 12 NaN\n3 12 0\n").unwrap();
        assert_eq!(
            read_choices(&path, base),
            vec![(
                FastOptions {
                    threads: 3,
                    collapse: 12,
                    ..base
                },
                654_321.0
            )]
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn prepared_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Prepared>();
    }

    #[test]
    fn the_unit_count_is_found_after_a_long_prelude() {
        let prelude: String = (0..40)
            .map(|k| format!("/* prelude line {k} */\n"))
            .collect();
        let source = format!("// header\n{prelude}typedef uint8_t u8;\n/* ecp5_emu units: 23 */\n#if TU == 0\n/* ecp5_emu units: 5 */\n#endif\n");
        assert_eq!(units_of(&source), 23);
        assert_eq!(units_of("// one unit\ntypedef uint8_t u8;\nint x;\n"), 1);
    }
}
