//! Deterministic digital event kernel on the full architecture's storage.
//!
//! The execution plan is explicit. This module does not infer enabled resources,
//! physical mux modes, startup, or global controls from raw bit-pattern matches.
mod alu;
pub(crate) mod cache;
mod dcs;
mod dsp;
mod dtr;
mod ebr;
mod fast;
mod fast_dcs;
mod fast_dsp;
mod iologic;
mod lutram;
mod osc;
mod pll;
mod system;
mod time;
pub use alu::{
    operate as alu54_operate, trailing_zeros_mask, Alu54, Alu54Outputs, AluRegister, ALU_C_GROUPS,
    ALU_FIRST_HALF, ALU_FLAGS, ALU_OPCODE_GROUPS,
};
pub use dcs::{DcsConfig, DcsMode};
pub use dsp::{
    multiply18, Multiplier18, MultiplierOperandOutputs, MultiplierOutputRegister,
    MultiplierOutputs, MultiplierPipeline, MultiplierRegister,
};
pub use dtr::{DtrConfig, DTR_CONVERSION_PS};
pub use ebr::{
    ebr_cell, Ebr, EbrMode, EbrNotice, EbrPort, WriteMode, PDP36_A_PATH_OWN_CONTROLS,
    PDP36_LOW_PATH,
};
pub use fast::{FastOptions, FastProgram, StartsUnknown};
pub use iologic::{IoLogicConfig, IoPath};
pub use lutram::LutRam;
pub use osc::OscillatorConfig;
pub use pll::{PllConfig, PllOutput};
pub use system::{GlobalReset, SystemConfig, SystemHandles, SystemPort, WakeSequence};
pub use time::Time;

use crate::{
    logic::Logic,
    model::{topology::Wire, Direction, NodeId, ResourceId},
    prepared::ConfiguredDevice,
    state::DeviceState,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);

macro_rules! handle {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name {
            scope: u64,
            index: usize,
        }
    };
}
handle!(SignalId);
#[cfg(test)]
impl SignalId {
    /// A signal of no kernel, for tests of what carries signals.
    pub(crate) fn for_tests(index: usize) -> Self {
        Self { scope: 0, index }
    }
}
handle!(DriverId);
handle!(TimerId);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Input {
    Constant(Logic),
    Signal(SignalId),
    Inverted(SignalId),
}
impl From<SignalId> for Input {
    fn from(signal: SignalId) -> Self {
        Self::Signal(signal)
    }
}
impl Input {
    /// The logical complement of this input.
    pub fn inverted(self) -> Self {
        match self {
            Self::Constant(v) => Self::Constant(v.invert()),
            Self::Signal(s) => Self::Inverted(s),
            Self::Inverted(s) => Self::Signal(s),
        }
    }
    fn signal(self) -> Option<SignalId> {
        match self {
            Self::Constant(_) => None,
            Self::Signal(s) | Self::Inverted(s) => Some(s),
        }
    }
    fn read(self, signals: &[Signal]) -> Logic {
        match self {
            Self::Constant(v) => v,
            Self::Signal(s) => signals[s.index].value,
            Self::Inverted(s) => signals[s.index].value.invert(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strength {
    Strong,
    Pull,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Rising,
    Falling,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetKind {
    Synchronous,
    Asynchronous,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Reset {
    /// Active high after optional Input inversion. X/Z merges both possibilities.
    pub input: Input,
    pub kind: ResetKind,
    pub value: Logic,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FlipFlop {
    #[serde(with = "cache::node")]
    pub node: NodeId,
    /// Mode-qualified DI/M selection must be supplied by the plan compiler.
    pub data: Input,
    /// Some(M) for LSRMODE=PRLD; reset selects M instead of REGSET.
    pub preload: Option<Input>,
    pub clock: Input,
    pub enable: Input,
    pub edge: Edge,
    pub reset: Option<Reset>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Options {
    /// Total combinational/capture waves at one time, including generated clocks.
    pub max_delta_cycles: u32,
    /// A bounded trace ring. Zero disables retention; counters still advance.
    pub trace_capacity: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_delta_cycles: 4096,
            trace_capacity: 65_536,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initial,
    Stimulus,
    Combinational,
    Capture,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Cause {
    pub source: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Transition {
    pub time: Time,
    pub delta: u32,
    pub phase: Phase,
    pub wire: Wire,
    pub previous: Logic,
    pub value: Logic,
    pub causes: Vec<Cause>,
}
#[derive(Default, Debug, Serialize, Clone, Deserialize)]
pub struct Statistics {
    pub time_batches: u64,
    pub driver_changes: u64,
    pub signal_changes: u64,
    pub combinational_evaluations: u64,
    pub register_evaluations: u64,
    pub lut_write_evaluations: u64,
    pub lut_write_edges: u64,
    pub lut_bit_changes: u64,
    pub system_evaluations: u64,
    pub multiplier_evaluations: u64,
    pub multiplier_register_evaluations: u64,
    pub multiplier_register_edges: u64,
    pub multiplier_register_bit_changes: u64,
    pub dsp_output_register_bit_changes: u64,
    pub ebr_evaluations: u64,
    pub ebr_access_edges: u64,
    pub ebr_bit_changes: u64,
    pub ebr_collision_rows: u64,
    pub ebr_control_ambiguities: u64,
    pub ebr_notice_overwrites: u64,
    pub possible_clock_edges: u64,
    pub discarded_timer_events: u64,
    pub trace_overwrites: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Signal {
    wire: Wire,
    value: Logic,
    drivers: Vec<usize>,
    combinational: Vec<usize>,
    /// Registers that read this signal as data, enable or reset.
    sequential: Vec<usize>,
    /// Clock groups clocked by this signal.
    clock_groups: Vec<usize>,
    /// False when nothing reads this signal (set once the kernel is built).
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
#[derive(Clone, Serialize, Deserialize)]
struct Driver {
    signal: usize,
    value: Logic,
    strength: Strength,
    label: String,
    external: bool,
}
#[derive(Clone, Serialize, Deserialize)]
enum Function {
    Buffer(Input),
    Receiver(Input),
    Mux {
        select: Input,
        low: Input,
        high: Input,
    },
    Lut4 {
        #[serde(with = "cache::node")]
        node: NodeId,
        inputs: [Input; 4],
    },
    Carry {
        #[serde(with = "cache::node")]
        node: NodeId,
        inputs: [Input; 4],
        cin: Input,
        inject: bool,
        carry_output: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
struct Combinational {
    function: Function,
    driver: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct Register {
    config: RegisterConfig,
    driver: usize,
    /// The clock group, unless the clock is a constant.
    group: Option<usize>,
    global_reset: Option<GlobalReset>,
    initialize_preload: bool,
    previous_preload: Logic,
    previous_lsr: Logic,
    #[serde(skip)]
    preload_changed_at: Option<Time>,
    #[serde(skip)]
    released_at: Option<Time>,
}
/// The same capture contract also serves internal resource registers which
/// have no physical slice node (e.g. the HY01 SGSR synchronizer).
#[derive(Clone, Serialize, Deserialize)]
struct RegisterConfig {
    #[serde(with = "cache::optional_node")]
    node: Option<NodeId>,
    data: Input,
    preload: Option<Input>,
    clock: Input,
    enable: Input,
    edge: Edge,
    reset: Option<Reset>,
}
/// Registers sharing one clock input. Their previous clock level is shared,
/// so a clock change is one group update, and a change that is no edge for a
/// register costs nothing for it.
#[derive(Clone, Serialize, Deserialize)]
struct ClockGroup {
    input: Input,
    previous: Logic,
    members: Vec<usize>,
    /// Edges of the change seen in the current capture wave; None otherwise.
    rising: Activity,
    falling: Activity,
}
#[cfg_attr(test, derive(serde::Serialize))]
struct Timer {
    label: String,
    generation: u64,
}
#[cfg_attr(test, derive(serde::Serialize))]
struct Event {
    driver: usize,
    value: Logic,
    timer: Option<(usize, u64)>,
}
/// Why a driver changed; the source is always the updated driver itself.
/// Kept compact on the hot path and expanded into a `Cause` only for traces.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(serde::Serialize))]
enum Reason {
    Static(&'static str),
    Timer { index: usize, generation: u64 },
}
#[cfg_attr(test, derive(serde::Serialize))]
struct Update {
    driver: usize,
    value: Logic,
    reason: Reason,
}
/// Resolution work item: signal rank, signal, and the changed driver with its reason.
type Affected = (usize, usize, Option<(usize, Reason)>);

/// Index set iterated in ascending order, without per-insert allocation.
#[derive(Default)]
#[cfg_attr(test, derive(serde::Serialize))]
struct DirtySet {
    items: Vec<usize>,
    marked: Vec<bool>,
    /// Buffer returned by `recycle`, reused by the next `take`.
    spare: Vec<usize>,
}
impl DirtySet {
    fn insert(&mut self, index: usize) {
        if index >= self.marked.len() {
            self.marked.resize(index + 1, false);
        }
        if !self.marked[index] {
            self.marked[index] = true;
            self.items.push(index);
        }
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Removes and returns every member in ascending order.
    fn take(&mut self) -> Vec<usize> {
        let mut items = std::mem::replace(&mut self.items, std::mem::take(&mut self.spare));
        for &index in &items {
            self.marked[index] = false;
        }
        items.sort_unstable();
        items
    }
    /// Gives back a buffer from `take` so the next wave does not allocate.
    fn recycle(&mut self, mut items: Vec<usize>) {
        items.clear();
        self.spare = items;
    }
}
impl Extend<usize> for DirtySet {
    fn extend<I: IntoIterator<Item = usize>>(&mut self, iter: I) {
        for index in iter {
            self.insert(index);
        }
    }
}
impl<'a> Extend<&'a usize> for DirtySet {
    fn extend<I: IntoIterator<Item = &'a usize>>(&mut self, iter: I) {
        self.extend(iter.into_iter().copied());
    }
}

pub struct Builder {
    device: Arc<ConfiguredDevice>,
    state: DeviceState,
    scope: u64,
    signals: Vec<Signal>,
    signal_ids: HashMap<Wire, SignalId>,
    drivers: Vec<Driver>,
    combinational: Vec<Combinational>,
    registers: Vec<Register>,
    lut_writes: Vec<lutram::Writer>,
    system: Option<system::Controller>,
    ebr: Vec<ebr::Block>,
    bound_ebr: HashSet<ResourceId>,
    multipliers: Vec<dsp::Multiplier>,
    bound_multipliers: HashSet<ResourceId>,
    alus: Vec<alu::Alu>,
    bound_alus: HashSet<ResourceId>,
    /// (signal, ALU) of every ALU input: the signals' reader lists get them
    /// at build(), numbered after the multipliers.
    alu_readers: Vec<(usize, usize)>,
    plls: Vec<pll::Pll>,
    iologic: Vec<iologic::IoLogic>,
    dcs: Vec<dcs::Dcs>,
    dtrs: Vec<dtr::Dtr>,
    oscillators: Vec<osc::Oscillator>,
    clock_groups: Vec<ClockGroup>,
    /// (signal index, inverted) to clock group.
    clock_group_ids: HashMap<(usize, bool), usize>,
    labels: HashSet<String>,
    bound_nodes: HashSet<NodeId>,
    options: Options,
}

impl Builder {
    pub fn new(device: Arc<ConfiguredDevice>) -> Result<Self> {
        let state = DeviceState::new(&device)?;
        Self::with_state(device, state)
    }
    /// Accepts initial LUT/FF storage, not a resumed simulation or cached wires.
    pub fn with_state(device: Arc<ConfiguredDevice>, state: DeviceState) -> Result<Self> {
        ensure!(
            state.belongs_to(&device),
            "state belongs to a different configuration"
        );
        ensure!(
            state.observed_wire_count() == 0,
            "cannot resume a wire snapshot without driver/timer state"
        );
        let scope = NEXT_SCOPE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| anyhow::anyhow!("kernel handle scopes exhausted"))?;
        Ok(Self {
            device,
            state,
            scope,
            signals: Vec::new(),
            signal_ids: HashMap::new(),
            drivers: Vec::new(),
            combinational: Vec::new(),
            registers: Vec::new(),
            lut_writes: Vec::new(),
            system: None,
            ebr: Vec::new(),
            bound_ebr: HashSet::new(),
            multipliers: Vec::new(),
            bound_multipliers: HashSet::new(),
            alus: Vec::new(),
            bound_alus: HashSet::new(),
            alu_readers: Vec::new(),
            plls: Vec::new(),
            iologic: Vec::new(),
            dcs: Vec::new(),
            dtrs: Vec::new(),
            oscillators: Vec::new(),
            clock_groups: Vec::new(),
            clock_group_ids: HashMap::new(),
            labels: HashSet::new(),
            bound_nodes: HashSet::new(),
            options: Options::default(),
        })
    }
    pub fn options(&mut self, options: Options) -> Result<()> {
        ensure!(options.max_delta_cycles > 0, "delta limit must be positive");
        self.options = options;
        Ok(())
    }
    pub fn signal(&mut self, wire: Wire) -> Result<SignalId> {
        if let Some(id) = self.signal_ids.get(&wire) {
            return Ok(*id);
        }
        let db = self.device.model().database();
        ensure!(
            !wire.name.is_empty()
                && (0..=db.max_row).contains(&wire.location.row)
                && (0..=db.max_col).contains(&wire.location.col),
            "invalid kernel wire"
        );
        let id = SignalId {
            scope: self.scope,
            index: self.signals.len(),
        };
        self.signal_ids.insert(wire.clone(), id);
        self.signals.push(Signal {
            wire,
            value: Logic::Z,
            drivers: Vec::new(),
            combinational: Vec::new(),
            sequential: Vec::new(),
            clock_groups: Vec::new(),
            read: true,
            lut_writes: Vec::new(),
            system: false,
            ebr: Vec::new(),
            multipliers: Vec::new(),
            plls: Vec::new(),
            iologic: Vec::new(),
            dcs: Vec::new(),
            dtrs: Vec::new(),
            watched: false,
        });
        Ok(id)
    }
    fn validate_signal(&self, signal: SignalId) -> Result<()> {
        ensure!(
            signal.scope == self.scope && signal.index < self.signals.len(),
            "signal belongs to a different kernel"
        );
        Ok(())
    }
    fn validate_input(&self, input: Input) -> Result<()> {
        if let Some(signal) = input.signal() {
            self.validate_signal(signal)?;
        }
        Ok(())
    }
    fn label(&mut self, label: &str) -> Result<()> {
        ensure!(
            !label.is_empty() && self.labels.insert(label.into()),
            "empty or duplicate execution source: {label}"
        );
        Ok(())
    }
    fn driver(
        &mut self,
        signal: SignalId,
        label: String,
        strength: Strength,
        value: Logic,
        external: bool,
    ) -> usize {
        let index = self.drivers.len();
        self.drivers.push(Driver {
            signal: signal.index,
            label,
            strength,
            value,
            external,
        });
        self.signals[signal.index].drivers.push(index);
        index
    }
    /// Each pad/device output gets its own driver, even when sharing one wire.
    pub fn external_driver(
        &mut self,
        signal: SignalId,
        label: &str,
        strength: Strength,
        initial: Logic,
    ) -> Result<DriverId> {
        self.validate_signal(signal)?;
        self.label(label)?;
        let index = self.driver(signal, label.into(), strength, initial, true);
        Ok(DriverId {
            scope: self.scope,
            index,
        })
    }
    pub(crate) fn initial_external_value(&mut self, id: DriverId, value: Logic) -> Result<()> {
        ensure!(id.scope == self.scope, "driver belongs to another builder");
        let driver = self.drivers.get_mut(id.index).context("unknown driver")?;
        ensure!(
            driver.external,
            "only an external driver can receive initial stimulus"
        );
        driver.value = value;
        Ok(())
    }
    /// Flip-flop storage before execution, e.g. its value right after
    /// configuration. Must come before the flip-flop is bound.
    pub(crate) fn initial_register(&mut self, node: NodeId, value: Logic) -> Result<()> {
        ensure!(value.bit().is_some(), "initial FF value must be 0 or 1");
        ensure!(
            !self.bound_nodes.contains(&node),
            "node already has an execution owner"
        );
        self.state.write_register(node, value)
    }
    /// The signal a chain of plain directed buffers (routes, DCC bypasses)
    /// starts from; the planner uses it to find which output feeds a PLL CLKFB.
    pub(crate) fn buffer_root(&self, signal: SignalId) -> Result<SignalId> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::buffer_root");
        self.validate_signal(signal)?;
        let mut index = signal.index;
        for _ in 0..self.signals.len() {
            let [driver] = self.signals[index].drivers[..] else {
                break;
            };
            match self.combinational.iter().find(|g| g.driver == driver) {
                Some(Combinational {
                    function: Function::Buffer(Input::Signal(source)),
                    ..
                }) => index = source.index,
                _ => break,
            }
        }
        Ok(SignalId {
            scope: self.scope,
            index,
        })
    }
    pub(crate) fn has_source(&self, label: &str) -> bool {
        self.labels.contains(label)
    }
    pub fn static_driver(
        &mut self,
        signal: SignalId,
        label: &str,
        strength: Strength,
        value: Logic,
    ) -> Result<()> {
        self.validate_signal(signal)?;
        self.label(label)?;
        self.driver(signal, label.into(), strength, value, false);
        Ok(())
    }
    pub fn watch(&mut self, signal: SignalId) -> Result<()> {
        self.validate_signal(signal)?;
        self.signals[signal.index].watched = true;
        Ok(())
    }
    fn add_combinational(
        &mut self,
        function: Function,
        inputs: &[Input],
        output: SignalId,
        label: String,
    ) {
        let index = self.combinational.len();
        for input in inputs {
            if let Some(signal) = input.signal() {
                self.signals[signal.index].combinational.push(index);
            }
        }
        let driver = self.driver(output, label, Strength::Strong, Logic::X, false);
        self.combinational.push(Combinational { function, driver });
    }
    /// Explicit directed transfer, never a union of source and sink identities.
    /// The caller owns mode qualification; this is not raw-mux auto-activation.
    pub fn buffer(&mut self, label: &str, input: Input, output: SignalId) -> Result<()> {
        self.validate_input(input)?;
        self.validate_signal(output)?;
        self.label(label)?;
        self.add_combinational(Function::Buffer(input), &[input], output, label.into());
        Ok(())
    }
    pub fn receiver(&mut self, label: &str, input: Input, output: SignalId) -> Result<()> {
        self.validate_input(input)?;
        self.validate_signal(output)?;
        self.label(label)?;
        self.add_combinational(Function::Receiver(input), &[input], output, label.into());
        Ok(())
    }
    /// Generic digital mux, also usable for an explicit tristate plan with Z.
    pub fn mux(
        &mut self,
        label: &str,
        select: Input,
        low: Input,
        high: Input,
        output: SignalId,
    ) -> Result<()> {
        for input in [select, low, high] {
            self.validate_input(input)?;
        }
        self.validate_signal(output)?;
        self.label(label)?;
        self.add_combinational(
            Function::Mux { select, low, high },
            &[select, low, high],
            output,
            label.into(),
        );
        Ok(())
    }
    fn node_port(&mut self, node: NodeId, name: &str, direction: Direction) -> Result<SignalId> {
        let port = self
            .device
            .model()
            .node_ports(node)
            .into_iter()
            .find(|p| p.name == name && p.direction == direction)
            .with_context(|| format!("missing native {name} port"))?;
        self.signal(port.wire)
    }
    /// Execute this physical LUT's live table with native A/B/C/D bit order.
    /// Wide LUT, carry and RAM modes need their own qualified plan operations.
    pub fn lut4(&mut self, node: NodeId) -> Result<SignalId> {
        self.state.lut(node)?;
        let mut inputs = [Input::Constant(Logic::X); 4];
        for (i, name) in ["A", "B", "C", "D"].into_iter().enumerate() {
            inputs[i] = self.node_port(node, name, Direction::Input)?.into();
        }
        self.lut4_with_inputs(node, inputs)
    }
    pub fn lut4_with_inputs(&mut self, node: NodeId, inputs: [Input; 4]) -> Result<SignalId> {
        self.state.lut(node)?;
        ensure!(
            !self.bound_nodes.contains(&node),
            "node already has an execution owner"
        );
        for input in inputs {
            self.validate_input(input)?;
        }
        let output = self.node_port(node, "F", Direction::Output)?;
        let label = format!(
            "lut:R{}C{}:{}",
            self.signals[output.index].wire.location.row,
            self.signals[output.index].wire.location.col,
            self.signals[output.index].wire.name
        );
        self.label(&label)?;
        self.bound_nodes.insert(node);
        self.add_combinational(Function::Lut4 { node, inputs }, &inputs, output, label);
        Ok(output)
    }
    /// One physical CCU2 half uses its own LUT table for p and the low four
    /// entries for g. INJECT1 suppresses g/sum injection, not propagation at p=1.
    pub fn carry(
        &mut self,
        node: NodeId,
        inputs: [Input; 4],
        cin: Input,
        inject: bool,
    ) -> Result<(SignalId, SignalId)> {
        self.state.lut(node)?;
        ensure!(
            !self.bound_nodes.contains(&node),
            "node already has an execution owner"
        );
        for input in inputs.into_iter().chain([cin]) {
            self.validate_input(input)?;
        }
        let sum = self.node_port(node, "F", Direction::Output)?;
        let cout = self.node_port(node, "FCO", Direction::Output)?;
        let wire = &self.signals[sum.index].wire;
        let key = format!(
            "carry:R{}C{}:{}",
            wire.location.row, wire.location.col, wire.name
        );
        self.label(&format!("{key}:sum"))?;
        self.label(&format!("{key}:cout"))?;
        self.bound_nodes.insert(node);
        let mut fanin = inputs.to_vec();
        fanin.push(cin);
        for (output, carry_output) in [(sum, false), (cout, true)] {
            self.add_combinational(
                Function::Carry {
                    node,
                    inputs,
                    cin,
                    inject,
                    carry_output,
                },
                &fanin,
                output,
                format!("{key}:{}", if carry_output { "cout" } else { "sum" }),
            );
        }
        Ok((sum, cout))
    }
    /// Local FF contract: reset precedes CE; initial Q comes from DeviceState.
    /// Global reset is optional and supplied by a mode-qualified planner.
    pub fn flip_flop(&mut self, config: FlipFlop) -> Result<SignalId> {
        self.flip_flop_with_global_reset(config, None)
    }
    pub fn flip_flop_with_global_reset(
        &mut self,
        config: FlipFlop,
        global_reset: Option<GlobalReset>,
    ) -> Result<SignalId> {
        let initial = self.state.register(config.node)?;
        ensure!(
            initial != Logic::Z,
            "FF storage cannot start in high impedance; use X"
        );
        ensure!(
            !self.bound_nodes.contains(&config.node),
            "node already has an execution owner"
        );
        let output = self.node_port(config.node, "Q", Direction::Output)?;
        let wire = &self.signals[output.index].wire;
        let label = format!(
            "ff:R{}C{}:{}",
            wire.location.row, wire.location.col, wire.name
        );
        self.bound_nodes.insert(config.node);
        self.bind_register(
            RegisterConfig {
                node: Some(config.node),
                data: config.data,
                preload: config.preload,
                clock: config.clock,
                enable: config.enable,
                edge: config.edge,
                reset: config.reset,
            },
            global_reset,
            output,
            initial,
            label,
        )
    }
    /// Reset-free, rising-edge internal resource FF; uses the ordinary kernel
    /// and fast-mode register machinery without claiming a physical slice FF.
    pub(crate) fn internal_flip_flop(
        &mut self,
        label: &str,
        data: Input,
        clock: Input,
        initial: Logic,
    ) -> Result<SignalId> {
        ensure!(
            initial.bit().is_some(),
            "internal FF initial value must be 0 or 1"
        );
        let output = self.signal(Wire {
            location: crate::model::topology::Location { row: 0, col: 0 },
            name: format!("$FF.{label}"),
        })?;
        self.bind_register(
            RegisterConfig {
                node: None,
                data,
                preload: None,
                clock,
                enable: Input::Constant(Logic::One),
                edge: Edge::Rising,
                reset: None,
            },
            None,
            output,
            initial,
            label.into(),
        )
    }
    fn bind_register(
        &mut self,
        config: RegisterConfig,
        global_reset: Option<GlobalReset>,
        output: SignalId,
        initial: Logic,
        label: String,
    ) -> Result<SignalId> {
        let mut inputs = vec![config.data, config.enable];
        inputs.extend(config.preload);
        if let Some(reset) = config.reset {
            ensure!(
                reset.value.bit().is_some(),
                "local reset value must be 0 or 1"
            );
            inputs.push(reset.input);
        }
        if let Some(reset) = global_reset {
            ensure!(
                reset.value.bit().is_some(),
                "global reset value must be 0 or 1"
            );
            inputs.push(reset.input);
        }
        for &input in inputs.iter().chain([&config.clock]) {
            self.validate_input(input)?;
        }
        self.label(&label)?;
        let index = self.registers.len();
        // Data/CE changes alone do not clock a register, but are relevant to
        // conservative merging while an asynchronous reset is uncertain.
        for input in inputs {
            if let Some(signal) = input.signal() {
                self.signals[signal.index].sequential.push(index);
            }
        }
        let key = match config.clock {
            Input::Constant(_) => None,
            Input::Signal(s) => Some((s.index, false)),
            Input::Inverted(s) => Some((s.index, true)),
        };
        let group = key.map(|key| {
            let groups = &mut self.clock_groups;
            let signals = &mut self.signals;
            *self.clock_group_ids.entry(key).or_insert_with(|| {
                signals[key.0].clock_groups.push(groups.len());
                groups.push(ClockGroup {
                    input: config.clock,
                    previous: Logic::X,
                    members: Vec::new(),
                    rising: Activity::None,
                    falling: Activity::None,
                });
                groups.len() - 1
            })
        });
        if let Some(group) = group {
            self.clock_groups[group].members.push(index);
        }
        let driver = self.driver(output, label, Strength::Strong, initial, false);
        self.registers.push(Register {
            config,
            driver,
            group,
            global_reset,
            initialize_preload: false,
            previous_preload: Logic::X,
            previous_lsr: Logic::X,
            preload_changed_at: None,
            released_at: None,
        });
        Ok(output)
    }
    /// Configured startup of a PRLD FF, sampled after the initial fabric settles.
    /// HS02 contains one startup observation of this behavior.
    pub fn initial_preload_register(&mut self, node: NodeId) -> Result<()> {
        let r = self
            .registers
            .iter_mut()
            .find(|r| r.config.node == Some(node))
            .context("unbound preload FF")?;
        ensure!(r.config.preload.is_some(), "initial preload requires PRLD");
        r.initialize_preload = true;
        Ok(())
    }
    pub fn build(mut self) -> Result<Kernel> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::build");
        // ALUs share the multipliers' dirty sets: index = multipliers + ALU.
        for (signal, alu) in std::mem::take(&mut self.alu_readers) {
            let index = self.multipliers.len() + alu;
            self.signals[signal].multipliers.push(index);
        }
        // Ranks reproduce the stable readable orders (wire, then driver label)
        // that keep traces independent of dense handles, without string
        // comparisons on every wave.
        let rank = |order: Vec<usize>| {
            let mut rank = vec![0; order.len()];
            for (position, index) in order.into_iter().enumerate() {
                rank[index] = position;
            }
            rank
        };
        let mut order: Vec<usize> = (0..self.signals.len()).collect();
        order.sort_by(|&a, &b| self.signals[a].wire.cmp(&self.signals[b].wire));
        let signal_rank = rank(order);
        let mut order: Vec<usize> = (0..self.drivers.len()).collect();
        order.sort_by(|&a, &b| self.drivers[a].label.cmp(&self.drivers[b].label));
        let driver_rank = rank(order);
        let mut kernel = Kernel {
            device: self.device,
            state: self.state,
            scope: self.scope,
            signals: self.signals,
            signal_ids: self.signal_ids,
            drivers: self.drivers,
            combinational: self.combinational,
            register_seen: vec![false; self.registers.len()],
            register_order: Vec::new(),
            registers: self.registers,
            clock_groups: self.clock_groups,
            dirty_clock_groups: DirtySet::default(),
            aliases: Vec::new(),
            lut_writes: self.lut_writes,
            system: self.system,
            ebr: self.ebr,
            multipliers: self.multipliers,
            alus: self.alus,
            plls: self.plls,
            dirty_plls: DirtySet::default(),
            iologic: self.iologic,
            dirty_iologic: DirtySet::default(),
            dcs: self.dcs,
            dtrs: self.dtrs,
            dirty_dtrs: DirtySet::default(),
            dirty_dcs: DirtySet::default(),
            oscillators: self.oscillators,
            oscillator_timers: HashMap::new(),
            signal_rank,
            driver_rank,
            affected: Vec::new(),
            spare_updates: Vec::new(),
            dirty_combinational: DirtySet::default(),
            dirty_registers: DirtySet::default(),
            dirty_lut_writes: DirtySet::default(),
            dirty_system: true,
            dirty_ebr: DirtySet::default(),
            dirty_multipliers: DirtySet::default(),
            dirty_multiplier_registers: DirtySet::default(),
            timers: Vec::new(),
            timer_labels: HashSet::new(),
            queue: BTreeMap::new(),
            now: Time::ZERO,
            delta: 0,
            options: self.options,
            trace: VecDeque::new(),
            ebr_notices: VecDeque::new(),
            stats: Statistics::default(),
            recent_activity: VecDeque::new(),
            fault: None,
        };
        for signal in 0..kernel.signals.len() {
            let slot = kernel
                .state
                .register_wire(kernel.signals[signal].wire.clone(), Logic::Z)?;
            // with_state() requires an empty wire snapshot, so slots follow signals.
            ensure!(slot == signal, "wire state slot differs from kernel signal");
        }
        kernel.resolve_all_initial()?;
        kernel
            .dirty_combinational
            .extend(0..kernel.combinational.len());
        kernel.initialize_system_clock()?;
        kernel
            .dirty_multipliers
            .extend(0..kernel.multipliers.len() + kernel.alus.len());
        kernel.settle_inputs()?;
        kernel.initialize_plls()?;
        kernel.initialize_oscillators()?;
        let mut startup = Vec::new();
        for r in &mut kernel.registers {
            r.previous_preload = r
                .config
                .preload
                .map_or(Logic::X, |m| m.read(&kernel.signals));
            r.previous_lsr = r
                .config
                .reset
                .map_or(Logic::Zero, |s| s.input.read(&kernel.signals));
            if r.initialize_preload {
                let value = match r.previous_preload {
                    Logic::Z => Logic::X,
                    value => value,
                };
                if let Some(node) = r.config.node {
                    kernel.state.write_register(node, value)?;
                }
                startup.push(Update {
                    driver: r.driver,
                    value,
                    reason: Reason::Static("configured preload"),
                });
            }
        }
        kernel.publish(startup, Phase::Initial)?;
        kernel.settle_inputs()?;
        // Establish initial clock levels only after the input fabric settles.
        kernel.initialize_dcs()?;
        kernel.initialize_dtrs()?;
        // There is no invented X->initial clock edge during construction.
        for group in &mut kernel.clock_groups {
            group.previous = group.input.read(&kernel.signals);
        }
        kernel.initialize_multiplier_clocks();
        kernel.initialize_alu_clocks();
        kernel.initialize_iologic_clocks();
        for writer in &mut kernel.lut_writes {
            writer.previous_clock = writer.config.clock.read(&kernel.signals);
        }
        for block in &mut kernel.ebr {
            block.previous_clock = block
                .config
                .ports
                .each_ref()
                .map(|p| p.clock.read(&kernel.signals));
        }
        kernel.dirty_registers.extend(0..kernel.registers.len());
        kernel.dirty_ebr.extend(0..kernel.ebr.len());
        kernel
            .dirty_multiplier_registers
            .extend(0..kernel.multipliers.len() + kernel.alus.len());
        kernel.dirty_iologic.extend(0..kernel.iologic.len());
        kernel.settle()?;
        kernel.collapse_buffers();
        kernel.state.mark_event_kernel_run();
        Ok(kernel)
    }
}

#[cfg_attr(test, derive(serde::Serialize))]
pub struct Kernel {
    #[cfg_attr(test, serde(skip))]
    device: Arc<ConfiguredDevice>,
    state: DeviceState,
    #[cfg_attr(test, serde(skip))]
    scope: u64,
    signals: Vec<Signal>,
    #[cfg_attr(test, serde(serialize_with = "cache::snapshot::map"))]
    signal_ids: HashMap<Wire, SignalId>,
    drivers: Vec<Driver>,
    combinational: Vec<Combinational>,
    registers: Vec<Register>,
    /// Signals that follow each signal within the same delta: routing
    /// segments whose buffer was collapsed (`collapse_buffers`).
    aliases: Vec<Vec<usize>>,
    clock_groups: Vec<ClockGroup>,
    dirty_clock_groups: DirtySet,
    /// Scratch for one capture wave: registers already queued, and their order.
    register_seen: Vec<bool>,
    register_order: Vec<usize>,
    lut_writes: Vec<lutram::Writer>,
    system: Option<system::Controller>,
    ebr: Vec<ebr::Block>,
    multipliers: Vec<dsp::Multiplier>,
    alus: Vec<alu::Alu>,
    plls: Vec<pll::Pll>,
    dirty_plls: DirtySet,
    iologic: Vec<iologic::IoLogic>,
    dirty_iologic: DirtySet,
    dcs: Vec<dcs::Dcs>,
    dtrs: Vec<dtr::Dtr>,
    dirty_dcs: DirtySet,
    dirty_dtrs: DirtySet,
    oscillators: Vec<osc::Oscillator>,
    /// Timer index to oscillator index.
    #[cfg_attr(test, serde(serialize_with = "cache::snapshot::map"))]
    oscillator_timers: HashMap<usize, usize>,
    signal_rank: Vec<usize>,
    driver_rank: Vec<usize>,
    affected: Vec<Affected>,
    spare_updates: Vec<Update>,
    dirty_combinational: DirtySet,
    dirty_registers: DirtySet,
    dirty_lut_writes: DirtySet,
    dirty_system: bool,
    dirty_ebr: DirtySet,
    dirty_multipliers: DirtySet,
    dirty_multiplier_registers: DirtySet,
    timers: Vec<Timer>,
    #[cfg_attr(test, serde(serialize_with = "cache::snapshot::set"))]
    timer_labels: HashSet<String>,
    queue: BTreeMap<Time, Vec<Event>>,
    now: Time,
    delta: u32,
    options: Options,
    trace: VecDeque<Transition>,
    ebr_notices: VecDeque<EbrNotice>,
    stats: Statistics,
    recent_activity: VecDeque<(usize, Logic)>,
    fault: Option<String>,
}

impl Kernel {
    pub fn device(&self) -> &Arc<ConfiguredDevice> {
        &self.device
    }
    pub fn state(&self) -> &DeviceState {
        &self.state
    }
    pub fn now(&self) -> Time {
        self.now
    }
    pub fn statistics(&self) -> &Statistics {
        &self.stats
    }
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }
    pub fn signal(&self, wire: &Wire) -> Option<SignalId> {
        self.signal_ids.get(wire).copied()
    }
    /// Keeps `signal` computed in the fast-mode programs built from now on,
    /// as a probe is (`FastProgram::fresh`): a net to sample after every
    /// edge. The event kernel runs as before (a routing segment that follows
    /// its buffer chain's root still does; the root is what is kept).
    pub fn watch(&mut self, signal: SignalId) -> Result<()> {
        ensure!(
            signal.scope == self.scope && signal.index < self.signals.len(),
            "signal belongs to a different kernel"
        );
        self.signals[signal.index].watched = true;
        Ok(())
    }
    pub fn value(&self, signal: SignalId) -> Result<Logic> {
        ensure!(
            signal.scope == self.scope && signal.index < self.signals.len(),
            "signal belongs to a different kernel"
        );
        Ok(self.signals[signal.index].value)
    }
    pub fn drain_trace(&mut self) -> Vec<Transition> {
        self.trace.drain(..).collect()
    }
    fn healthy(&self) -> Result<()> {
        ensure!(
            self.fault.is_none(),
            "kernel is stopped after a fault: {}",
            self.fault.as_deref().unwrap_or_default()
        );
        Ok(())
    }
    pub fn new_timer(&mut self, label: &str) -> Result<TimerId> {
        self.healthy()?;
        ensure!(
            !label.is_empty() && self.timer_labels.insert(label.into()),
            "empty or duplicate timer label"
        );
        let id = TimerId {
            scope: self.scope,
            index: self.timers.len(),
        };
        self.timers.push(Timer {
            label: label.into(),
            generation: 0,
        });
        Ok(id)
    }
    fn timer(&self, timer: TimerId) -> Result<&Timer> {
        ensure!(
            timer.scope == self.scope,
            "timer belongs to a different kernel"
        );
        self.timers.get(timer.index).context("unknown timer")
    }
    /// Invalidates already queued events without searching/reordering the queue.
    pub fn cancel_timer(&mut self, timer: TimerId) -> Result<()> {
        self.healthy()?;
        let generation = self
            .timer(timer)?
            .generation
            .checked_add(1)
            .context("timer generation overflow")?;
        self.timers[timer.index].generation = generation;
        Ok(())
    }
    pub fn schedule(&mut self, time: Time, driver: DriverId, value: Logic) -> Result<()> {
        self.schedule_batch(time, &[(driver, value)], None)
    }
    /// A future edge of an internal generator (PLL), tagged with its timer.
    fn schedule_generated(
        &mut self,
        time: Time,
        driver: usize,
        value: Logic,
        timer: usize,
    ) -> Result<()> {
        ensure!(time > self.now, "generated edge is not in the future");
        let generation = self.timers[timer].generation;
        self.queue.entry(time).or_default().push(Event {
            driver,
            value,
            timer: Some((timer, generation)),
        });
        Ok(())
    }
    /// Batches targeting the same timestamp are collected before any resolution.
    /// Two different values for one driver at one time are an error, not priority.
    pub fn schedule_batch(
        &mut self,
        time: Time,
        updates: &[(DriverId, Logic)],
        timer: Option<TimerId>,
    ) -> Result<()> {
        self.healthy()?;
        ensure!(
            time > self.now,
            "timestamp is already closed: {time} <= {}",
            self.now
        );
        let timer = timer
            .map(|id| self.timer(id).map(|t| (id.index, t.generation)))
            .transpose()?;
        let mut values = BTreeMap::new();
        for &(driver, value) in updates {
            ensure!(
                driver.scope == self.scope
                    && self.drivers.get(driver.index).is_some_and(|d| d.external),
                "invalid external driver or different kernel"
            );
            if let Some(previous) = values.insert(driver.index, value) {
                ensure!(
                    previous == value,
                    "conflicting values in one stimulus batch"
                );
            }
        }
        if !values.is_empty() {
            self.queue
                .entry(time)
                .or_default()
                .extend(values.into_iter().map(|(driver, value)| Event {
                    driver,
                    value,
                    timer,
                }));
        }
        Ok(())
    }
    /// Advances through a closed timestamp. On execution error the kernel is
    /// faulted; partial delta history is retained and cannot be resumed silently.
    pub fn run_until(&mut self, until: Time) -> Result<()> {
        self.healthy()?;
        ensure!(until >= self.now, "cannot run backwards");
        while self
            .queue
            .first_key_value()
            .is_some_and(|(time, _)| *time <= until)
        {
            let (time, events) = self.queue.pop_first().expect("nonempty event queue");
            if let Err(error) = self.advance(time, events) {
                let message = format!("event kernel at {time}: {error:#}");
                self.fault = Some(message.clone());
                bail!(message);
            }
        }
        self.now = until;
        Ok(())
    }
    /// An empty update buffer, reusing the one the last `publish` released.
    fn update_buffer(&mut self) -> Vec<Update> {
        std::mem::take(&mut self.spare_updates)
    }
    fn advance(&mut self, time: Time, events: Vec<Event>) -> Result<()> {
        let mut values = BTreeMap::new();
        let mut updates = self.update_buffer();
        let mut oscillator_edges = Vec::new();
        for event in events {
            let reason = if let Some((index, generation)) = event.timer {
                if self.timers[index].generation != generation {
                    self.stats.discarded_timer_events += 1;
                    continue;
                }
                if let Some(oscillator) = self.oscillator_of(index) {
                    oscillator_edges.push((oscillator, event.value));
                }
                Reason::Timer { index, generation }
            } else {
                Reason::Static("external stimulus")
            };
            if let Some(previous) = values.insert(event.driver, event.value) {
                ensure!(
                    previous == event.value,
                    "conflicting events for driver {} at one timestamp",
                    self.drivers[event.driver].label
                );
            }
            updates.push(Update {
                driver: event.driver,
                value: event.value,
                reason,
            });
        }
        self.now = time;
        self.delta = 0;
        self.stats.time_batches += 1;
        self.publish(updates, Phase::Stimulus)?;
        self.settle()?;
        for (oscillator, value) in oscillator_edges {
            self.next_oscillator_edge(oscillator, value)?;
        }
        Ok(())
    }
    fn next_delta(&mut self) -> Result<()> {
        if self.delta >= self.options.max_delta_cycles {
            let activity = self
                .recent_activity
                .iter()
                .map(|&(index, value)| {
                    let driver = &self.drivers[index];
                    let wire = &self.signals[driver.signal].wire;
                    format!(
                        "{} -> R{}C{}:{}={value}",
                        driver.label, wire.location.row, wire.location.col, wire.name
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            bail!("did not settle after {} delta waves (possible zero-time feedback); recent source changes: {activity}",self.options.max_delta_cycles);
        }
        self.delta += 1;
        Ok(())
    }
    fn resolve_all_initial(&mut self) -> Result<()> {
        let mut affected = std::mem::take(&mut self.affected);
        for (signal, rank) in self.signal_rank.iter().copied().enumerate() {
            let drivers = &self.signals[signal].drivers;
            if drivers.is_empty() {
                affected.push((rank, signal, None));
            }
            affected.extend(drivers.iter().map(|&driver| {
                (
                    rank,
                    signal,
                    Some((driver, Reason::Static("initial driver"))),
                )
            }));
        }
        self.resolve(affected, Phase::Initial)
    }
    fn cause(&self, driver: usize, reason: Reason) -> Cause {
        Cause {
            source: self.drivers[driver].label.clone(),
            reason: match reason {
                Reason::Static(reason) => reason.into(),
                Reason::Timer { index, generation } => {
                    format!("timer:{}#{generation}", self.timers[index].label)
                }
            },
        }
    }
    fn publish(&mut self, mut updates: Vec<Update>, phase: Phase) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::publish");
        // Stable sort: updates of one driver become adjacent in arrival order.
        updates.sort_by_key(|update| self.driver_rank[update.driver]);
        for range in driver_runs(&updates) {
            ensure!(
                updates[range.clone()]
                    .iter()
                    .all(|update| update.value == updates[range.start].value),
                "conflicting internal driver updates"
            );
        }
        let mut affected = std::mem::take(&mut self.affected);
        // Apply the entire wave before resolving even the first destination.
        for range in driver_runs(&updates) {
            let index = updates[range.start].driver;
            let value = updates[range.start].value;
            let driver = &mut self.drivers[index];
            if driver.value != value {
                driver.value = value;
                self.stats.driver_changes += 1;
                if self.recent_activity.len() == 16 {
                    self.recent_activity.pop_front();
                }
                self.recent_activity.push_back((index, value));
                let signal = driver.signal;
                let rank = self.signal_rank[signal];
                affected.extend(
                    updates[range]
                        .iter()
                        .map(|update| (rank, signal, Some((index, update.reason)))),
                );
            }
        }
        updates.clear();
        self.spare_updates = updates;
        self.resolve(affected, phase)
    }
    fn resolve(&mut self, mut affected: Vec<Affected>, phase: Phase) -> Result<()> {
        // Stable readable wire order makes traces independent of dense handles.
        affected.sort_by_key(|&(rank, _, _)| rank);
        let mut start = 0;
        while start < affected.len() {
            let index = affected[start].1;
            let end = start
                + affected[start..]
                    .iter()
                    .take_while(|&&(_, signal, _)| signal == index)
                    .count();
            let range = start..end;
            start = end;
            let signal = &self.signals[index];
            let strong = Logic::resolve(
                signal
                    .drivers
                    .iter()
                    .filter(|&&d| self.drivers[d].strength == Strength::Strong)
                    .map(|&d| self.drivers[d].value),
            );
            let value = if strong == Logic::Z {
                Logic::resolve(
                    signal
                        .drivers
                        .iter()
                        .filter(|&&d| self.drivers[d].strength == Strength::Pull)
                        .map(|&d| self.drivers[d].value),
                )
            } else {
                strong
            };
            if signal.value == value {
                continue;
            }
            let causes = (signal.watched && self.options.trace_capacity > 0).then(|| {
                let mut causes: Vec<Cause> = affected[range]
                    .iter()
                    .filter_map(|&(_, _, cause)| cause)
                    .map(|(driver, reason)| self.cause(driver, reason))
                    .collect();
                causes.sort();
                causes.dedup();
                causes
            });
            let previous = self.signals[index].value;
            self.set_signal(index, value)?;
            if !self.aliases.is_empty() && !self.aliases[index].is_empty() {
                let aliases = std::mem::take(&mut self.aliases[index]);
                for &alias in &aliases {
                    self.set_signal(alias, value)?;
                }
                self.aliases[index] = aliases;
            }
            let signal = &self.signals[index];
            if let Some(causes) = causes {
                if self.trace.len() == self.options.trace_capacity {
                    self.trace.pop_front();
                    self.stats.trace_overwrites += 1;
                }
                self.trace.push_back(Transition {
                    time: self.now,
                    delta: self.delta,
                    phase,
                    wire: signal.wire.clone(),
                    previous,
                    value,
                    causes,
                });
            }
        }
        affected.clear();
        self.affected = affected;
        Ok(())
    }
    /// Stores a new signal value and marks everything that reads it.
    fn set_signal(&mut self, index: usize, value: Logic) -> Result<()> {
        let signal = &mut self.signals[index];
        signal.value = value;
        self.state.set_wire_slot(index, value)?;
        self.stats.signal_changes += 1;
        if !signal.read {
            return Ok(());
        }
        self.dirty_combinational.extend(&signal.combinational);
        self.dirty_registers.extend(&signal.sequential);
        self.dirty_clock_groups.extend(&signal.clock_groups);
        self.dirty_lut_writes.extend(&signal.lut_writes);
        self.dirty_system |= signal.system;
        self.dirty_ebr.extend(&signal.ebr);
        self.dirty_multipliers.extend(&signal.multipliers);
        self.dirty_multiplier_registers.extend(&signal.multipliers);
        self.dirty_plls.extend(&signal.plls);
        self.dirty_iologic.extend(&signal.iologic);
        self.dirty_dcs.extend(&signal.dcs);
        self.dirty_dtrs.extend(&signal.dtrs);
        Ok(())
    }
    /// Routing segments are buffers from one signal to the next. A signal
    /// whose only driver is such a buffer, and that no probe watches, follows
    /// the root of its buffer chain: once the initial settle has made them
    /// equal, the buffer is dropped and the segment takes the root's value in
    /// the same delta, without a gate evaluation or a wave of its own. Values
    /// at every settled time stay the same; transitions of watched signals can
    /// arrive in fewer delta cycles. A loop of buffers is left as it is.
    fn collapse_buffers(&mut self) {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::collapse_buffers");
        const NONE: usize = usize::MAX;
        let mut gate_of_driver = vec![NONE; self.drivers.len()];
        for (g, gate) in self.combinational.iter().enumerate() {
            gate_of_driver[gate.driver] = g;
        }
        // (source signal, buffer gate) of each collapsible signal.
        let source: Vec<Option<(usize, usize)>> = self
            .signals
            .iter()
            .enumerate()
            .map(|(s, signal)| {
                if signal.watched || signal.drivers.len() != 1 {
                    return None;
                }
                let g = gate_of_driver[signal.drivers[0]];
                match self.combinational.get(g)?.function {
                    Function::Buffer(Input::Signal(src)) if src.index != s => Some((src.index, g)),
                    _ => None,
                }
            })
            .collect();
        let mut aliases = vec![Vec::new(); self.signals.len()];
        let mut collapsed = 0_usize;
        for s in 0..self.signals.len() {
            let Some((first, gate)) = source[s] else {
                continue;
            };
            let (mut root, mut steps) = (first, 0);
            while let Some((next, _)) = source[root] {
                root = next;
                steps += 1;
                if steps > 100_000 {
                    root = NONE;
                    break;
                }
            }
            if root == NONE || self.signals[s].value != self.signals[root].value {
                continue;
            }
            self.signals[first].combinational.retain(|&g| g != gate);
            aliases[root].push(s);
            collapsed += 1;
        }
        // A signal and its aliases change together, so whatever reads an alias
        // is marked from the root; an alias update is then just its value.
        for (root, list) in aliases.iter().enumerate() {
            for &alias in list {
                let moved = &mut self.signals[alias];
                let lists = [
                    std::mem::take(&mut moved.combinational),
                    std::mem::take(&mut moved.sequential),
                    std::mem::take(&mut moved.clock_groups),
                    std::mem::take(&mut moved.lut_writes),
                    std::mem::take(&mut moved.ebr),
                    std::mem::take(&mut moved.multipliers),
                    std::mem::take(&mut moved.plls),
                    std::mem::take(&mut moved.iologic),
                    std::mem::take(&mut moved.dcs),
                    std::mem::take(&mut moved.dtrs),
                ];
                let system = std::mem::take(&mut moved.system);
                let target = &mut self.signals[root];
                target.system |= system;
                for (into, from) in [
                    &mut target.combinational,
                    &mut target.sequential,
                    &mut target.clock_groups,
                    &mut target.lut_writes,
                    &mut target.ebr,
                    &mut target.multipliers,
                    &mut target.plls,
                    &mut target.iologic,
                    &mut target.dcs,
                    &mut target.dtrs,
                ]
                .into_iter()
                .zip(lists)
                {
                    into.extend(from);
                }
            }
            if !list.is_empty() {
                let target = &mut self.signals[root];
                for v in [
                    &mut target.combinational,
                    &mut target.sequential,
                    &mut target.clock_groups,
                    &mut target.lut_writes,
                    &mut target.ebr,
                    &mut target.multipliers,
                    &mut target.plls,
                    &mut target.iologic,
                    &mut target.dcs,
                    &mut target.dtrs,
                ] {
                    v.sort_unstable();
                    v.dedup();
                }
            }
        }
        if collapsed > 0 {
            self.aliases = aliases;
        }
        for signal in &mut self.signals {
            signal.read = !(signal.combinational.is_empty()
                && signal.sequential.is_empty()
                && signal.clock_groups.is_empty()
                && signal.lut_writes.is_empty()
                && !signal.system
                && signal.ebr.is_empty()
                && signal.multipliers.is_empty()
                && signal.plls.is_empty()
                && signal.iologic.is_empty()
                && signal.dcs.is_empty()
                && signal.dtrs.is_empty());
        }
    }
    fn settle_combinational(&mut self) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::settle_combinational");
        while !self.dirty_combinational.is_empty() || !self.dirty_multipliers.is_empty() {
            self.next_delta()?;
            if self.dirty_combinational.is_empty() {
                let updates = self.evaluate_multipliers()?;
                self.publish(updates, Phase::Combinational)?;
                continue;
            }
            let dirty = self.dirty_combinational.take();
            let mut updates = self.update_buffer();
            for &index in &dirty {
                let gate = &self.combinational[index];
                self.stats.combinational_evaluations += 1;
                let value = match &gate.function {
                    Function::Buffer(input) => input.read(&self.signals),
                    Function::Receiver(input) => input
                        .read(&self.signals)
                        .bit()
                        .map(Logic::from_bool)
                        .unwrap_or(Logic::X),
                    Function::Mux { select, low, high } => Logic::mux(
                        select.read(&self.signals),
                        low.read(&self.signals),
                        high.read(&self.signals),
                    ),
                    Function::Lut4 { node, inputs } => {
                        let address = inputs.map(|input| input.read(&self.signals));
                        Logic::table(self.state.lut(*node)?, &address)
                    }
                    Function::Carry {
                        node,
                        inputs,
                        cin,
                        inject,
                        carry_output,
                    } => {
                        let address = inputs.map(|i| i.read(&self.signals));
                        let table = self.state.lut(*node)?;
                        let p = Logic::table(table, &address);
                        let cin = cin.read(&self.signals);
                        if *carry_output {
                            let g = if *inject {
                                Logic::Zero
                            } else {
                                Logic::table(
                                    table,
                                    &[address[0], address[1], Logic::Zero, Logic::Zero],
                                )
                            };
                            Logic::mux(p, g, cin)
                        } else {
                            p.xor(if *inject { Logic::Zero } else { cin })
                        }
                    }
                };
                updates.push(Update {
                    driver: gate.driver,
                    value,
                    reason: Reason::Static("combinational evaluation"),
                });
            }
            self.dirty_combinational.recycle(dirty);
            self.publish(updates, Phase::Combinational)?;
        }
        Ok(())
    }
    fn settle(&mut self) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("kernel/mod::settle");
        loop {
            self.settle_inputs()?;
            // PLLs only schedule future edges, so they never reopen this wave.
            if !self.dirty_plls.is_empty() {
                self.evaluate_plls()?;
            }
            if self.dirty_registers.is_empty()
                && self.dirty_clock_groups.is_empty()
                && self.dirty_lut_writes.is_empty()
                && self.dirty_ebr.is_empty()
                && self.dirty_multiplier_registers.is_empty()
                && self.dirty_iologic.is_empty()
                && self.dirty_dcs.is_empty()
                && self.dirty_dtrs.is_empty()
            {
                return Ok(());
            }
            self.next_delta()?;
            // Clock groups whose clock changed: one edge test per group.
            let groups = self.dirty_clock_groups.take();
            for &g in &groups {
                let group = &mut self.clock_groups[g];
                let clock = group.input.read(&self.signals);
                let previous = std::mem::replace(&mut group.previous, clock);
                group.rising = clock_edge(Edge::Rising, previous, clock);
                group.falling = clock_edge(Edge::Falling, previous, clock);
            }
            // Registers whose data or controls changed, then the members of
            // groups that saw an edge of their kind; each once.
            let dirty = self.dirty_registers.take();
            let mut order = std::mem::take(&mut self.register_order);
            for &index in &dirty {
                self.register_seen[index] = true;
                order.push(index);
            }
            for &g in &groups {
                let group = &self.clock_groups[g];
                if group.rising == Activity::None && group.falling == Activity::None {
                    continue;
                }
                for &index in &group.members {
                    let activity = match self.registers[index].config.edge {
                        Edge::Rising => group.rising,
                        Edge::Falling => group.falling,
                    };
                    if activity != Activity::None && !self.register_seen[index] {
                        self.register_seen[index] = true;
                        order.push(index);
                    }
                }
            }
            let mut captures = Vec::new();
            for &index in &order {
                self.register_seen[index] = false;
                let register = &mut self.registers[index];
                self.stats.register_evaluations += 1;
                let config = &register.config;
                let edge = register.group.map_or(Activity::None, |g| {
                    let group = &self.clock_groups[g];
                    match config.edge {
                        Edge::Rising => group.rising,
                        Edge::Falling => group.falling,
                    }
                });
                if edge == Activity::Possible {
                    self.stats.possible_clock_edges += 1;
                }
                // The output driver holds the stored value.
                let old = self.drivers[register.driver].value;
                let data = match config.data.read(&self.signals) {
                    Logic::Z => Logic::X,
                    value => value,
                };
                let preload = config.preload.map(|m| match m.read(&self.signals) {
                    Logic::Z => Logic::X,
                    v => v,
                });
                let mut capture = Logic::mux(config.enable.read(&self.signals), old, data);
                if let Some(reset) = config.reset.filter(|r| r.kind == ResetKind::Synchronous) {
                    capture = Logic::mux(
                        reset.input.read(&self.signals),
                        capture,
                        preload.unwrap_or(reset.value),
                    );
                }
                let mut value = match edge {
                    Activity::None => old,
                    Activity::Definite => capture,
                    Activity::Possible => Logic::mux(Logic::X, old, capture),
                };
                if let Some(reset) = config.reset.filter(|r| r.kind == ResetKind::Asynchronous) {
                    value = Logic::mux(
                        reset.input.read(&self.signals),
                        value,
                        preload.unwrap_or(reset.value),
                    );
                }
                if let (Some(m), Some(reset)) = (preload, config.reset) {
                    let lsr = reset.input.read(&self.signals);
                    if m != register.previous_preload {
                        register.preload_changed_at = Some(self.now);
                    }
                    if register.previous_lsr == Logic::One && lsr == Logic::Zero {
                        register.released_at = Some(self.now);
                    }
                    if reset.kind == ResetKind::Asynchronous
                        && lsr == Logic::Zero
                        && register.released_at == Some(self.now)
                        && register.preload_changed_at == Some(self.now)
                    {
                        value = Logic::X;
                    }
                    register.previous_preload = m;
                    register.previous_lsr = lsr;
                }
                if let Some(reset) = register.global_reset {
                    value = Logic::mux(
                        reset.input.read(&self.signals),
                        value,
                        preload.unwrap_or(reset.value),
                    );
                }
                if value != old {
                    captures.push((config.node, register.driver, value));
                }
            }
            for &g in &groups {
                let group = &mut self.clock_groups[g];
                group.rising = Activity::None;
                group.falling = Activity::None;
            }
            order.clear();
            self.register_order = order;
            self.dirty_registers.recycle(dirty);
            self.dirty_clock_groups.recycle(groups);
            // RAM writes and FF captures sample the same settled inputs. RAM
            // read outputs are invalidated only after every capture is computed.
            let memory_updates = self.capture_lut_writes()?;
            let ebr_captures = self.capture_ebr()?;
            // DSP stages and fabric FFs sample the same pre-publication state.
            let multiplier_captures = self.capture_multipliers()?;
            let iologic_captures = self.capture_iologic()?;
            let dcs_updates = self.capture_dcs(false);
            let dtr_updates = self.capture_dtrs(false)?;
            // Every register samples the same pre-publication state, including
            // independent clocks and cascaded registers on coincident edges.
            let mut updates = self.update_buffer();
            for (node, driver, value) in captures {
                if let Some(node) = node {
                    self.state.write_register(node, value)?;
                }
                updates.push(Update {
                    driver,
                    value,
                    reason: Reason::Static("simultaneous register capture"),
                });
            }
            for update in memory_updates {
                self.state
                    .write_lut(update.node, update.address, update.value)?;
                self.dirty_combinational.insert(update.reader);
                self.stats.lut_bit_changes += 1;
            }
            self.commit_ebr(ebr_captures, &mut updates)?;
            self.commit_multipliers(multiplier_captures)?;
            self.commit_iologic(iologic_captures, &mut updates);
            updates.extend(dcs_updates);
            updates.extend(dtr_updates);
            self.publish(updates, Phase::Capture)?;
        }
    }
}

/// Ranges of consecutive updates that target the same driver.
fn driver_runs(updates: &[Update]) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let driver = updates.get(start)?.driver;
        let end = start
            + updates[start..]
                .iter()
                .take_while(|update| update.driver == driver)
                .count();
        let range = start..end;
        start = end;
        Some(range)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Activity {
    None,
    Definite,
    Possible,
}
fn clock_edge(edge: Edge, previous: Logic, current: Logic) -> Activity {
    if previous == current {
        return Activity::None;
    }
    let (from, to) = match edge {
        Edge::Rising => (false, true),
        Edge::Falling => (true, false),
    };
    if previous.bit() == Some(from) && current.bit() == Some(to) {
        Activity::Definite
    } else if previous.bit().is_none_or(|v| v == from) && current.bit().is_none_or(|v| v == to) {
        Activity::Possible
    } else {
        Activity::None
    }
}
