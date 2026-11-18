//! IOLOGIC registers and generic DDR X1, as observed on the board (HI02).
//!
//! - ODDRX1F: TXDATA0/TXDATA1 captured at an SCLK rising edge leave the pad
//!   two rising edges later, TXDATA0 in the high phase and TXDATA1 in the
//!   following low phase.
//! - IDDRX1F: the pad at a rising edge (RXDATA0) and at the following falling
//!   edge (RXDATA1) are published together at the next rising edge.
//! - IREG/OREG/TSREG: a rising-edge flip-flop.
//!
//! - DCCA (compile/mod.rs), a clock gate: CE taken at a rising edge of CLKI
//!   decides whether that high phase passes, whole; CLKO falls with CLKI
//!   (HC01). It shares this rise/fall capture machinery, as do the X2
//!   gearbox (HI03) and the edge-clock blocks ECLKSYNCB and CLKDIVF (HI04).
//!
//! The DDR stages start at 0 after configuration; an IO register starts at its
//! REGSET value. An asynchronous reset holds the DDR stages at 0 and an IO
//! register at REGSET, and capture restarts at the first edge after release.
use super::*;

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub enum IoPath {
    /// ODDRX1F: TXDATA0, TXDATA1 to IOLDO. `global_reset` is the GSR with
    /// GSR=ENABLED: like the path's reset it clears every stage and the
    /// output at once while active (HI09).
    OutputDdr {
        data: [Input; 2],
        output: SignalId,
        global_reset: Option<Input>,
    },
    /// An IO register in latch mode (HI10): transparent while its clock is
    /// low and `enable` (CE) high, holding its output otherwise; the path's
    /// reset (LSR, ASYNC) clears it at once. It starts at 0.
    Latch {
        data: Input,
        enable: Input,
        output: SignalId,
    },
    /// IDDRX1F: PADDI to RXDATA0, RXDATA1; `global_reset` as for ODDRX1F.
    InputDdr {
        data: Input,
        outputs: [SignalId; 2],
        global_reset: Option<Input>,
    },
    /// ODDRX2F, edge-clock side (HI03), clocked by ECLK on both edges. Its
    /// SCLK-side stages are ordinary `Register` paths whose last stage feeds
    /// `data`. A rising edge with SCLK high (the mid edge, where SCLK falls)
    /// loads the word and puts D0 out; the falling edge after it D1; the next
    /// rising edge (SCLK low, where SCLK rises) D2; the falling edge after it
    /// D3. Stages: the word w0..w3 and m (the last rising edge was a mid one).
    GearOut {
        data: [Input; 4],
        sclk: Input,
        output: SignalId,
    },
    /// IDDRX2F, edge-clock side (HI03), clocked by ECLK on both edges: the
    /// pad at the mid rising edge, the falling edge after it, the next rising
    /// edge and the falling edge after it are samples 0..3; the next mid
    /// rising edge moves them to `outputs`, which SCLK-side `Register` paths
    /// publish at the next SCLK rising edge. Stages: s0..s3 and m.
    GearIn {
        data: Input,
        sclk: Input,
        outputs: [SignalId; 4],
    },
    /// ODDR71B, edge-clock side (HI07), clocked by ECLK, acting the same on
    /// both edges. At the edge after SCLK rises (SCLK read high now, low at
    /// the edge before) it loads the word and puts D0 out; each later edge
    /// puts out the next bit, D1..D6, then holds. The SCLK side is two ranks
    /// of `Register` paths. Stages: w0..w6, the bit index (3 bits), and SCLK
    /// as read at the previous edge.
    GearOut7 {
        data: [Input; 7],
        sclk: Input,
        output: SignalId,
    },
    /// IDDR71B, edge-clock side (HI07), clocked by ECLK on both edges: it
    /// keeps the pad at the last seven edges; at the edge after SCLK rises
    /// (the same edge as GearOut7's load) it hands them to `outputs`, oldest
    /// first into Q4, Q5, Q6, Q0, Q1, Q2, Q3. SCLK-side `Register` paths
    /// publish them at the next SCLK rising edge. Stages: the seven samples,
    /// oldest first, and SCLK as read at the previous edge.
    GearIn7 {
        data: Input,
        sclk: Input,
        outputs: [SignalId; 7],
    },
    /// DELAYF's delay counter (HI08), clocked by MOVE: at a falling edge of
    /// MOVE it steps by DIRECTION as taken at MOVE's rise (0 up, 1 down),
    /// saturating at 0 and 127;
    /// while LOADN is low it holds `value` (DEL_VALUE) at once and MOVE does
    /// nothing. `initial` is the configured counter (HI11: IOLOGIC bits 1..5
    /// of DEL_VALUE, SIOLOGIC zero). `outputs` are its seven bits, LSB first;
    /// CFLAG is plan logic over them and DIRECTION.
    DelayCounter {
        loadn: Input,
        direction: Input,
        value: u8,
        initial: u8,
        outputs: [SignalId; 7],
    },
    /// CLKDIVF (HI04). /2 toggles CDIVX at every rising edge of the clock;
    /// /3.5 (`seven_halves`) counts both edges, CDIVX high for 4 of every 7.
    /// RST (the path's reset) clears everything; after its release the first
    /// rising edge is consumed and the second raises CDIVX. ALIGNWD is
    /// sampled at rising edges: the first sample that sees it high starts a
    /// slip two rising edges later, where the divider ignores that rising edge
    /// and the falling edge after it. /3.5 slips on every ALIGNWD, /2 on every
    /// second one since the last reset. It starts low, rising at the first
    /// edge. Stages: run, armed (the first edge after a reset was consumed),
    /// the /3.5 count c0..c2, the last ALIGNWD sample, the slip pipeline
    /// p1, p2, the falling edge to skip, and the /2 ALIGNWD parity. All zero is
    /// the reset state.
    Divider {
        align: Input,
        output: SignalId,
        seven_halves: bool,
    },
    /// ECLKSYNCB's STOP synchronizer (HI04): STOP passes four stages clocked
    /// by the falling edges of ECLKI; `gate` is the last one, and ECLKO =
    /// ECLKI while it is 0 (a mux in the plan). HI13 initializes all four
    /// stages to `initial`: 0 for a pre-running primary pin clock, otherwise
    /// 1 for the HI04 fifth-rising-edge start (PLL/OSCG history unmeasured).
    StopSync {
        stop: Input,
        gate: SignalId,
        initial: Logic,
    },
    /// DCCA: CLKO is CE taken at the rising edge of the clock, and 0 from
    /// the falling edge. It starts low (closed) until the first rising edge,
    /// an assumption: the board run did not see the start.
    ClockGate { enable: Input, output: SignalId },
    /// IREG, OREG or TSREG (HI02, HI05): a rising-edge flip-flop with a
    /// clock enable, a synchronous set/reset that wins over the enable
    /// (SRMODE=LSR_OVER_CE; the path's own `reset` is the ASYNC one), and
    /// the global reset when GSR=ENABLED, which acts asynchronously. Every
    /// reset sets REGSET.
    Register {
        data: Input,
        output: SignalId,
        /// REGSET: the value after configuration and under reset.
        value: Logic,
        /// CE after CEMUX and CEIMUX/CEOMUX; constant 1 when not used.
        enable: Input,
        /// LSR when SRMODE=LSR_OVER_CE.
        sync_reset: Option<Input>,
        /// The device GSR when GSR=ENABLED.
        global_reset: Option<Input>,
    },
}

impl IoPath {
    /// The GSR a path takes, if any: an IO register's or an X1 DDR path's
    /// with GSR=ENABLED. It acts at once, like the path's reset; an IO
    /// register takes its REGSET value, a DDR path clears.
    pub fn global_reset(&self) -> Option<Input> {
        match self {
            IoPath::Register { global_reset, .. }
            | IoPath::OutputDdr { global_reset, .. }
            | IoPath::InputDdr { global_reset, .. } => *global_reset,
            _ => None,
        }
    }
}
#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct IoLogicConfig {
    pub resource: ResourceId,
    /// Path name for labels, e.g. "ODDR" or "TSREG".
    pub name: &'static str,
    pub path: IoPath,
    /// Clock after the CLKIMUX/CLKOMUX selection.
    pub clock: Input,
    /// Active-high asynchronous reset after LSRMUX and LSRIMUX/LSROMUX.
    pub reset: Option<Input>,
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct IoLogic {
    pub(super) config: IoLogicConfig,
    pub(super) drivers: Vec<usize>,
    /// ODDR: [D0, D1] of the first and second stage, then the D1 waiting for
    /// the falling edge. IDDR: rising sample, falling sample.
    pub(super) stages: Vec<Logic>,
    previous_clock: Logic,
}

impl Builder {
    pub fn iologic(&mut self, config: IoLogicConfig) -> Result<()> {
        let key = self.device.model().resource(config.resource).key.clone();
        let zeros = |n: usize| vec![Logic::Zero; n];
        let (inputs, outputs, initial, stages): (Vec<Input>, Vec<SignalId>, Logic, Vec<Logic>) =
            match &config.path {
                IoPath::OutputDdr {
                    data,
                    output,
                    global_reset,
                } => (
                    data.iter().chain(global_reset).copied().collect(),
                    vec![*output],
                    Logic::Zero,
                    zeros(5),
                ),
                IoPath::InputDdr {
                    data,
                    outputs,
                    global_reset,
                } => (
                    std::iter::once(data).chain(global_reset).copied().collect(),
                    outputs.to_vec(),
                    Logic::Zero,
                    zeros(2),
                ),
                IoPath::Latch {
                    data,
                    enable,
                    output,
                } => (vec![*data, *enable], vec![*output], Logic::Zero, zeros(0)),
                IoPath::ClockGate { enable, output } => {
                    (vec![*enable], vec![*output], Logic::Zero, zeros(0))
                }
                IoPath::Divider {
                    align,
                    output,
                    seven_halves,
                } => {
                    // Running; /3.5 at count 6, so the first edge raises
                    // CDIVX as for /2.
                    let mut stages = zeros(DIVIDER_STAGES);
                    stages[0] = Logic::One;
                    if *seven_halves {
                        stages[3] = Logic::One;
                        stages[4] = Logic::One;
                    }
                    (vec![*align], vec![*output], Logic::Zero, stages)
                }
                IoPath::StopSync {
                    stop,
                    gate,
                    initial,
                } => (vec![*stop], vec![*gate], *initial, vec![*initial; 3]),
                IoPath::GearOut { data, sclk, output } => {
                    let mut inputs = data.to_vec();
                    inputs.push(*sclk);
                    (inputs, vec![*output], Logic::Zero, zeros(5))
                }
                IoPath::GearIn {
                    data,
                    sclk,
                    outputs,
                } => (vec![*data, *sclk], outputs.to_vec(), Logic::Zero, zeros(5)),
                IoPath::GearOut7 { data, sclk, output } => {
                    let mut inputs = data.to_vec();
                    inputs.push(*sclk);
                    (inputs, vec![*output], Logic::Zero, zeros(GEAR_OUT7_STAGES))
                }
                IoPath::DelayCounter {
                    loadn,
                    direction,
                    outputs,
                    ..
                } => (
                    vec![*loadn, *direction],
                    outputs.to_vec(),
                    Logic::Zero,
                    zeros(1),
                ),
                IoPath::GearIn7 {
                    data,
                    sclk,
                    outputs,
                } => (
                    vec![*data, *sclk],
                    outputs.to_vec(),
                    Logic::Zero,
                    zeros(GEAR_IN7_STAGES),
                ),
                IoPath::Register {
                    data,
                    output,
                    value,
                    enable,
                    sync_reset,
                    global_reset,
                } => {
                    ensure!(value.bit().is_some(), "IO register REGSET must be 0 or 1");
                    let mut inputs = vec![*data, *enable];
                    inputs.extend(sync_reset.iter().chain(global_reset));
                    (inputs, vec![*output], *value, zeros(0))
                }
            };
        let index = self.iologic.len();
        for input in inputs.into_iter().chain([config.clock]).chain(config.reset) {
            self.validate_input(input)?;
            if let Some(signal) = input.signal() {
                self.signals[signal.index].iologic.push(index);
            }
        }
        let mut drivers = Vec::new();
        for (n, signal) in outputs.into_iter().enumerate() {
            self.validate_signal(signal)?;
            let label = format!("iologic:{key}:{}{n}", config.name);
            self.label(&label)?;
            let initial = match &config.path {
                IoPath::DelayCounter { initial, .. } => Logic::from_bool((initial >> n) & 1 != 0),
                _ => initial,
            };
            drivers.push(self.driver(signal, label, Strength::Strong, initial, false));
        }
        self.iologic.push(IoLogic {
            config,
            drivers,
            stages,
            previous_clock: Logic::X,
        });
        Ok(())
    }
}

pub(super) struct IoLogicCapture {
    index: usize,
    stages: Vec<Logic>,
    outputs: Vec<Logic>,
}

impl Kernel {
    pub(super) fn initialize_iologic_clocks(&mut self) {
        for io in &mut self.iologic {
            io.previous_clock = io.config.clock.read(&self.signals);
        }
    }
    /// Computes every dirty IOLOGIC from the pre-publication state; nothing is
    /// committed here, so cascaded registers on one edge see old values.
    pub(super) fn capture_iologic(&mut self) -> Result<Vec<IoLogicCapture>> {
        let dirty = self.dirty_iologic.take();
        let mut captures = Vec::new();
        for &index in &dirty {
            let io = &mut self.iologic[index];
            let clock = io.config.clock.read(&self.signals);
            let previous = std::mem::replace(&mut io.previous_clock, clock);
            let rising = clock_edge(Edge::Rising, previous, clock);
            let falling = clock_edge(Edge::Falling, previous, clock);
            if rising == Activity::Possible || falling == Activity::Possible {
                self.stats.possible_clock_edges += 1;
            }
            let read = |input: Input| match input.read(&self.signals) {
                Logic::Z => Logic::X,
                value => value,
            };
            let old_outputs: Vec<Logic> =
                io.drivers.iter().map(|&d| self.drivers[d].value).collect();
            let (mut stages, mut outputs) = (io.stages.clone(), old_outputs.clone());
            // A possible edge merges the captured and the held value.
            let merge = |activity: Activity, old: Logic, new: Logic| match activity {
                Activity::None => old,
                Activity::Definite => new,
                Activity::Possible => Logic::mux(Logic::X, old, new),
            };
            let reset_value = match &io.config.path {
                IoPath::OutputDdr { data, .. } => {
                    let s = &io.stages;
                    let d = data.map(read);
                    // Rising: out <- second-stage D0, waiting D1 <- second-stage
                    // D1, second stage <- first stage, first stage <- data.
                    let next = [d[0], d[1], s[0], s[1], s[3]];
                    for (n, value) in next.into_iter().enumerate() {
                        stages[n] = merge(rising, s[n], value);
                    }
                    outputs[0] = merge(rising, outputs[0], s[2]);
                    outputs[0] = merge(falling, outputs[0], s[4]);
                    Logic::Zero
                }
                IoPath::InputDdr { data, .. } => {
                    let s = &io.stages;
                    let d = read(*data);
                    // Rising: outputs <- (rising sample, falling sample), then
                    // rising sample <- pad. Falling: falling sample <- pad.
                    outputs[0] = merge(rising, outputs[0], s[0]);
                    outputs[1] = merge(rising, outputs[1], s[1]);
                    stages[0] = merge(rising, s[0], d);
                    stages[1] = merge(falling, s[1], d);
                    Logic::Zero
                }
                IoPath::GearOut { data, sclk, .. } => {
                    let s = &io.stages;
                    let d = data.map(read);
                    let mid = read(*sclk);
                    for k in 0..4 {
                        stages[k] = merge(rising, s[k], Logic::mux(mid, s[k], d[k]));
                    }
                    stages[4] = merge(rising, s[4], mid);
                    outputs[0] = merge(rising, outputs[0], Logic::mux(mid, s[2], d[0]));
                    outputs[0] = merge(falling, outputs[0], Logic::mux(s[4], s[3], s[1]));
                    Logic::Zero
                }
                IoPath::GearIn { data, sclk, .. } => {
                    let s = &io.stages;
                    let pad = read(*data);
                    let mid = read(*sclk);
                    for k in 0..4 {
                        outputs[k] = merge(rising, outputs[k], Logic::mux(mid, outputs[k], s[k]));
                    }
                    stages[0] = merge(rising, s[0], Logic::mux(mid, s[0], pad));
                    stages[2] = merge(rising, s[2], Logic::mux(mid, pad, s[2]));
                    stages[4] = merge(rising, s[4], mid);
                    stages[1] = merge(falling, s[1], Logic::mux(s[4], s[1], pad));
                    stages[3] = merge(falling, s[3], Logic::mux(s[4], pad, s[3]));
                    Logic::Zero
                }
                IoPath::DelayCounter {
                    loadn,
                    direction,
                    value,
                    ..
                } => {
                    // DIRECTION is taken at MOVE's rise and the step made at
                    // its fall: in the HI08 protocol the same as DIRECTION at
                    // the fall, and outside it (DIRECTION changed while MOVE
                    // is high) the board's direction in two cases of three.
                    // LOADN low clears the taken DIRECTION too (fast mode's
                    // reset clears every stage of a path).
                    let load = read(*loadn).invert();
                    let s = &io.stages;
                    stages[0] =
                        Logic::mux(load, merge(rising, s[0], read(*direction)), Logic::Zero);
                    let stepped = delay_step(&old_outputs, s[0]);
                    for k in 0..7 {
                        let stepped = merge(falling, old_outputs[k], stepped[k]);
                        let loaded = Logic::from_bool((value >> k) & 1 == 1);
                        outputs[k] = Logic::mux(load, stepped, loaded);
                    }
                    Logic::Zero
                }
                IoPath::Latch { data, enable, .. } => {
                    // Level-sensitive: evaluated whenever D, CE, CLK or LSR
                    // changes, from their values after the change.
                    let open = Logic::mux(clock, read(*enable), Logic::Zero);
                    outputs[0] = Logic::mux(open, outputs[0], read(*data));
                    Logic::Zero
                }
                IoPath::GearOut7 { data, sclk, .. } => {
                    let edge = either_edge(rising, falling);
                    let (next, out) =
                        gear_out7_next(&io.stages, outputs[0], &data.map(read), read(*sclk));
                    for (k, value) in next.into_iter().enumerate() {
                        stages[k] = merge(edge, io.stages[k], value);
                    }
                    outputs[0] = merge(edge, outputs[0], out);
                    Logic::Zero
                }
                IoPath::GearIn7 { data, sclk, .. } => {
                    let edge = either_edge(rising, falling);
                    let (next, held) =
                        gear_in7_next(&io.stages, &old_outputs, read(*data), read(*sclk));
                    for (k, value) in next.into_iter().enumerate() {
                        stages[k] = merge(edge, io.stages[k], value);
                    }
                    for (k, value) in held.into_iter().enumerate() {
                        outputs[k] = merge(edge, outputs[k], value);
                    }
                    Logic::Zero
                }
                IoPath::Divider {
                    align,
                    seven_halves,
                    ..
                } => {
                    let s = &io.stages;
                    let next = divider_next(s, old_outputs[0], read(*align), *seven_halves);
                    for k in 0..DIVIDER_STAGES {
                        stages[k] = merge(falling, merge(rising, s[k], next.rise[k]), next.fall[k]);
                    }
                    outputs[0] =
                        merge(falling, merge(rising, outputs[0], next.rise_q), next.fall_q);
                    Logic::Zero
                }
                IoPath::StopSync { stop, .. } => {
                    let s = &io.stages;
                    stages[0] = merge(falling, s[0], read(*stop));
                    stages[1] = merge(falling, s[1], s[0]);
                    stages[2] = merge(falling, s[2], s[1]);
                    outputs[0] = merge(falling, outputs[0], s[2]);
                    Logic::Zero
                }
                IoPath::ClockGate { enable, .. } => {
                    outputs[0] = merge(rising, outputs[0], read(*enable));
                    outputs[0] = merge(falling, outputs[0], Logic::Zero);
                    Logic::Zero
                }
                IoPath::Register {
                    data,
                    value,
                    enable,
                    sync_reset,
                    global_reset,
                    ..
                } => {
                    let mut next = Logic::mux(read(*enable), outputs[0], read(*data));
                    if let Some(reset) = sync_reset {
                        next = Logic::mux(read(*reset), next, *value);
                    }
                    outputs[0] = merge(rising, outputs[0], next);
                    if let Some(reset) = global_reset {
                        outputs[0] = Logic::mux(read(*reset), outputs[0], *value);
                    }
                    *value
                }
            };
            // The path's reset, and an X1 DDR path's global reset (HI09),
            // clear every stage and output while active.
            let ddr_global = match &io.config.path {
                IoPath::OutputDdr { global_reset, .. } | IoPath::InputDdr { global_reset, .. } => {
                    *global_reset
                }
                _ => None,
            };
            for reset in io.config.reset.into_iter().chain(ddr_global) {
                let reset = reset.read(&self.signals);
                for value in stages.iter_mut().chain(outputs.iter_mut()) {
                    *value = Logic::mux(reset, *value, reset_value);
                }
            }
            if stages != io.stages || outputs != old_outputs {
                captures.push(IoLogicCapture {
                    index,
                    stages,
                    outputs,
                });
            }
        }
        self.dirty_iologic.recycle(dirty);
        Ok(captures)
    }
    pub(super) fn commit_iologic(
        &mut self,
        captures: Vec<IoLogicCapture>,
        updates: &mut Vec<Update>,
    ) {
        for capture in captures {
            let io = &mut self.iologic[capture.index];
            io.stages = capture.stages;
            for (&driver, value) in io.drivers.iter().zip(capture.outputs) {
                updates.push(Update {
                    driver,
                    value,
                    reason: Reason::Static("IOLOGIC capture"),
                });
            }
        }
    }
}

/// Stages of a `Divider` path.
pub(super) const DIVIDER_STAGES: usize = 10;

/// A divider's stages and CDIVX after a rising and after a falling edge of
/// its clock, from the stages `s` and CDIVX `q` before it (see `Divider`).
pub(super) struct DividerNext {
    pub rise: [Logic; DIVIDER_STAGES],
    pub rise_q: Logic,
    pub fall: [Logic; DIVIDER_STAGES],
    pub fall_q: Logic,
}

/// The /3.5 count after one more edge: 0..6, then 0.
fn count7(c: [Logic; 3]) -> [Logic; 3] {
    match (c[0].bit(), c[1].bit(), c[2].bit()) {
        (Some(a), Some(b), Some(d)) => {
            let v = u8::from(a) | u8::from(b) << 1 | u8::from(d) << 2;
            let n = if v >= 6 { 0 } else { v + 1 };
            [0, 1, 2].map(|k| Logic::from_bool((n >> k) & 1 == 1))
        }
        _ => [Logic::X; 3],
    }
}

pub(super) fn divider_next(s: &[Logic], q: Logic, align: Logic, seven: bool) -> DividerNext {
    use Logic::{One, Zero};
    let (run, armed, c) = (s[0], s[1], [s[2], s[3], s[4]]);
    let (sampled, p1, p2, skip, parity) = (s[5], s[6], s[7], s[8], s[9]);
    let next = count7(c);
    let (counted, counted_q) = if seven {
        (next, next[2].invert())
    } else {
        (c, q.invert())
    };
    // Rising edge: a reset's wait, a slip, or one step.
    let trigger = align.and(sampled.invert());
    let mut rise = [Zero; DIVIDER_STAGES];
    rise[0] = run.or(armed);
    rise[1] = run.invert().and(armed.invert());
    for k in 0..3 {
        rise[2 + k] = Logic::mux(
            run,
            Logic::mux(armed, c[k], Zero),
            Logic::mux(p2, counted[k], c[k]),
        );
    }
    let rise_q = Logic::mux(run, Logic::mux(armed, q, One), Logic::mux(p2, counted_q, q));
    rise[5] = align;
    rise[6] = if seven { trigger } else { trigger.and(parity) };
    rise[7] = p1;
    rise[8] = run.and(p2);
    rise[9] = if seven { parity } else { parity.xor(trigger) };
    // Falling edge: /3.5 steps unless a slip skips it.
    let mut fall = [Zero; DIVIDER_STAGES];
    fall.copy_from_slice(&s[..DIVIDER_STAGES]);
    fall[8] = Zero;
    let mut fall_q = q;
    if seven {
        let step = run.and(skip.invert());
        for k in 0..3 {
            fall[2 + k] = Logic::mux(step, c[k], next[k]);
        }
        fall_q = Logic::mux(step, q, next[2].invert());
    }
    DividerNext {
        rise,
        rise_q,
        fall,
        fall_q,
    }
}

/// Stages of a `GearOut7` path: w0..w6, the bit index i0..i2, SCLK last read.
pub(super) const GEAR_OUT7_STAGES: usize = 11;
/// Stages of a `GearIn7` path: seven pad samples, oldest first, SCLK last read.
pub(super) const GEAR_IN7_STAGES: usize = 8;
/// For each of Q0..Q6, which of the seven samples (oldest first) it takes.
pub(super) const GEAR_IN7_ORDER: [usize; 7] = [3, 4, 5, 6, 0, 1, 2];

/// A capture on either clock edge: the gearboxes of 7:1 act the same on both.
fn either_edge(rising: Activity, falling: Activity) -> Activity {
    match (rising, falling) {
        (Activity::Definite, _) | (_, Activity::Definite) => Activity::Definite,
        (Activity::Possible, _) | (_, Activity::Possible) => Activity::Possible,
        _ => Activity::None,
    }
}

/// GearOut7 after an ECLK edge: the stages and the output, from the stages
/// `s`, the output `out`, the word `d` and SCLK.
fn gear_out7_next(s: &[Logic], out: Logic, d: &[Logic; 7], sclk: Logic) -> (Vec<Logic>, Logic) {
    let load = sclk.and(s[10].invert());
    let index = match (s[7].bit(), s[8].bit(), s[9].bit()) {
        (Some(a), Some(b), Some(c)) => {
            Some(usize::from(a) | usize::from(b) << 1 | usize::from(c) << 2)
        }
        _ => None,
    };
    // Without a load: the next bit, then hold after D6.
    let (shown, counted) = match index {
        Some(i) if i <= 6 => (s[i], i + 1),
        Some(i) => (out, i),
        None => (Logic::X, 8),
    };
    let mut next: Vec<Logic> = (0..7).map(|k| Logic::mux(load, s[k], d[k])).collect();
    for b in 0..3 {
        let counted = if counted > 7 {
            Logic::X
        } else {
            Logic::from_bool((counted >> b) & 1 == 1)
        };
        next.push(Logic::mux(load, counted, Logic::from_bool(b == 0)));
    }
    next.push(sclk);
    (next, Logic::mux(load, shown, d[0]))
}

/// GearIn7 after an ECLK edge: the stages and the holding outputs, from the
/// stages `s`, the outputs `held`, the pad and SCLK.
fn gear_in7_next(s: &[Logic], held: &[Logic], pad: Logic, sclk: Logic) -> (Vec<Logic>, Vec<Logic>) {
    let load = sclk.and(s[7].invert());
    let mut samples: Vec<Logic> = s[1..7].to_vec();
    samples.push(pad);
    let outputs = (0..7)
        .map(|q| Logic::mux(load, held[q], samples[GEAR_IN7_ORDER[q]]))
        .collect();
    samples.push(sclk);
    (samples, outputs)
}

/// A DELAYF counter after one MOVE step: the seven bits `c` (LSB first)
/// plus one when DIRECTION is 0 or minus one when it is 1, saturating at 127
/// and 0; unknown bits or DIRECTION make every bit unknown.
fn delay_step(c: &[Logic], direction: Logic) -> Vec<Logic> {
    let mut count = 0_u8;
    for (k, bit) in c.iter().enumerate().take(7) {
        match bit.bit() {
            Some(b) => count |= u8::from(b) << k,
            None => return vec![Logic::X; 7],
        }
    }
    let next = match direction.bit() {
        Some(false) => count.saturating_add(1).min(127),
        Some(true) => count.saturating_sub(1),
        None => return vec![Logic::X; 7],
    };
    (0..7)
        .map(|k| Logic::from_bool((next >> k) & 1 == 1))
        .collect()
}
