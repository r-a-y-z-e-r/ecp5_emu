//! Mode-qualified execution planning from native CRAM, with explicit boundaries.
mod cache;
mod config_ports;
mod dcs;
mod dsp;
mod dtr;
mod ebr;
mod eclk;
mod index;
mod iologic;
mod lutram;
mod oscillator;
mod route_decode;
mod system;
#[cfg(test)]
pub(crate) mod timing;
mod usrmclk;
pub use config_ports::ConfigurationPortUse;
pub use dsp::MultiplierUse;
pub use dtr::ULX3S_DTR_CODE;
pub use ebr::EbrUse;
pub use lutram::LutRamUse;
pub use oscillator::{NOMINAL_OSCILLATOR_HZ, OSCILLATOR_DIVIDERS};
pub use system::SystemPlan;
pub use usrmclk::{cclk_pad, CclkPad};

use crate::{
    database::{ArcKind, BitConstraint},
    kernel::{
        self, Builder, DriverId, FlipFlop, Input, Kernel, Reset, ResetKind, SignalId, Strength,
        SystemPort,
    },
    logic::Logic,
    model::{
        topology::{Endpoint, Wire},
        Direction, NodeId, ResourceId, TileId,
    },
    prepared::ConfiguredDevice,
};
use anyhow::{bail, ensure, Context, Result};
use index::WireId;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Reference {
    Pin(String),
    Wire(Wire),
    System(SystemPort),
}
impl Reference {
    pub fn parse(model: &crate::model::DeviceModel, text: &str) -> Result<Self> {
        if let Some(port) = text.strip_prefix("sys:") {
            return Ok(Self::System(SystemPort::parse(port)?));
        }
        if let Some(ball) = text.strip_prefix("pin:") {
            ensure!(!ball.is_empty(), "empty pin reference");
            return Ok(Self::Pin(ball.into()));
        }
        let (position, name) = text
            .split_once(':')
            .context("expected pin:G2 or R38C2:Q0")?;
        ensure!(
            !name.starts_with("$SYS."),
            "use sys: references for logical system controls"
        );
        let (row, col) = crate::database::coordinates(position)?;
        match model.wire(crate::model::topology::Location { row, col }, name)? {
            Endpoint::Wire(wire) => Ok(Self::Wire(wire)),
            _ => bail!("reference is outside this device"),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Pin(ball) => format!("pin:{ball}"),
            Self::Wire(w) => format!("R{}C{}:{}", w.location.row, w.location.col, w.name),
            Self::System(port) => format!("sys:{}", port.name()),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Observed,
    AllPlcRegisters,
}
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Startup {
    ReadyUnknown,
    /// User mode right after configuration: executed PLC flip-flops start at
    /// their REGSET value, whatever SLICE GSR says (hardware series HS01).
    Configured,
    WakePhases,
}
#[derive(Clone, Serialize)]
pub struct Request {
    pub package: String,
    pub scope: Scope,
    /// Ready environment or explicit wake phase boundaries after loading.
    /// No profile executes POR/transport or infers physical START timing.
    pub startup: Startup,
    pub drives: Vec<(Reference, Logic)>,
    pub probes: Vec<Reference>,
    pub options: kernel::Options,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct FieldUse {
    pub tile: String,
    pub name: String,
    pub candidates: Vec<String>,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct ConstantUse {
    pub tile: String,
    pub field: String,
    pub wire: Wire,
    pub encoded_value: String,
    pub value: Logic,
    pub rule: String,
}
#[derive(Clone, Serialize, serde::Deserialize)]
pub struct RouteUse {
    pub tile: String,
    pub source: String,
    pub sink: String,
    pub source_line: usize,
    pub rule: String,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct ClockAlias {
    pub raw: Wire,
    pub segment: Wire,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct ResourceField {
    pub name: String,
    pub status: String,
    pub candidates: Vec<String>,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct ResourceUse {
    pub id: String,
    pub family: String,
    pub rule: String,
    pub fields: Vec<ResourceField>,
}
#[derive(Default, Serialize, serde::Deserialize, Clone)]
pub struct Counts {
    pub lut4: usize,
    pub lutram_columns: usize,
    pub ramw: usize,
    pub ebr_blocks: usize,
    pub ebr_read_ports: usize,
    pub multiplier18: usize,
    pub alu54: usize,
    pub carry_halves: usize,
    pub wide_muxes: usize,
    pub flip_flops: usize,
    pub pio: usize,
    pub iologic_paths: usize,
    pub dcc_bypasses: usize,
    /// DCCA clock gates with a dynamic CE.
    pub dcc_gates: usize,
    /// ECLKSYNCB edge-clock synchronisers, BRGECLKSYNC included.
    pub eclk_syncs: usize,
    /// ECLKBRIDGECS edge-clock bridges.
    pub eclk_bridges: usize,
    /// CLKDIVF edge-clock dividers.
    pub clock_dividers: usize,
    pub constants: usize,
    pub routes: usize,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct Report {
    pub schema_version: u32,
    pub configuration_sha256: String,
    pub scope: Scope,
    pub startup: Startup,
    pub roots: Vec<String>,
    pub counts: Counts,
    pub fields: Vec<FieldUse>,
    pub constants: Vec<ConstantUse>,
    pub routes: Vec<RouteUse>,
    pub excluded_routes: Vec<RouteUse>,
    pub resources: Vec<ResourceUse>,
    pub clock_aliases: Vec<ClockAlias>,
    pub lutram: Vec<LutRamUse>,
    pub ebr: Vec<EbrUse>,
    pub multipliers: Vec<MultiplierUse>,
    pub system: SystemPlan,
    pub undriven: Vec<Wire>,
    /// Jointly decoded mode of every executed PIO (decode/pio.rs).
    pub pio_modes: BTreeMap<String, crate::decode::PioMode>,
    pub limitations: Vec<String>,
}
pub struct Compiled {
    pub kernel: Kernel,
    pub drivers: BTreeMap<String, DriverId>,
    pub probes: BTreeMap<String, SignalId>,
    pub report: Report,
}
impl Compiled {
    pub fn value(&self, reference: &Reference) -> Result<Logic> {
        if let Some(&signal) = self.probes.get(&reference.label()) {
            return self.kernel.value(signal);
        }
        if let Reference::Wire(wire) = reference {
            let wire = index::canonical_ref(self.kernel.device().model(), wire)?;
            return self.kernel.value(
                self.kernel
                    .signal(wire.as_ref())
                    .context("wire outside execution plan")?,
            );
        }
        bail!("reference is outside execution plan")
    }
}
#[derive(Clone)]
struct Node {
    id: NodeId,
    owner: ResourceId,
    tile: TileId,
    name: String,
}
#[derive(Clone)]
enum Producer {
    Node(usize, String),
    Resource(ResourceId, String),
}
struct Pad {
    signal: SignalId,
    driver: DriverId,
    receiver_enabled: bool,
    /// Some consistent PIO modes have a receiver and others do not.
    receiver_ambiguous: bool,
}
/// Feedback qualified jointly with the overlapping physical route bits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PllFeedback {
    Internal(&'static str),
    External,
    RawVco,
}
/// A PLL whose settings are checked but whose feedback root is not yet known.
struct PendingPll {
    reference: Input,
    feedback: SignalId,
    feedback_kind: PllFeedback,
    feedback_description: String,
    clki: u64,
    clkfb: u64,
    /// divider, CPHASE*8+FPHASE, enabled
    outputs: BTreeMap<&'static str, (u64, i64, bool)>,
    /// Output ports the plan reaches.
    used: BTreeMap<&'static str, SignalId>,
    /// RST and STDBY where PLLRST_ENA and STDBY_ENABLE enable them.
    controls: Vec<Input>,
}
struct Planner {
    device: Arc<ConfiguredDevice>,
    builder: Builder,
    index: index::Index,
    routes: route_decode::Decoded,
    nodes: Vec<Node>,
    /// Split-node ports relative to their PLC, shared by every placed node.
    node_ports: HashMap<String, Vec<crate::model::Port>>,
    producers: HashMap<WireId, Vec<Producer>>,
    signals: Vec<Option<SignalId>>,
    boundaries: HashSet<WireId>,
    queue: VecDeque<WireId>,
    seen: Vec<bool>,
    done_nodes: HashSet<NodeId>,
    done_lutram: HashSet<TileId>,
    done_resources: HashSet<ResourceId>,
    dsp_divider_mode: Option<bool>,
    /// IOLOGIC paths already planned: IOLDO, IOLTO, INFF or RXDATA.
    done_iologic: HashSet<(ResourceId, &'static str)>,
    fields: HashSet<(TileId, String)>,
    clock_aliases: HashSet<WireId>,
    pads: HashMap<ResourceId, Pad>,
    /// Joint PIO modes by resource key; one decode covers a whole pair.
    pio_modes: HashMap<String, crate::decode::PioMode>,
    plls: HashMap<ResourceId, PendingPll>,
    /// OSCG base frequency in Hz (Board::oscillator_hz).
    oscillator_hz: u64,
    dtr_code: u8,
    report: Report,
    package: String,
    system: Option<kernel::SystemHandles>,
}

/// A connection on the board outside the chip, such as a jumper wire from an
/// output pin to an input pin: the `to` pad follows the `from` pad, including
/// high impedance. It is one-directional.
#[derive(Clone, Debug, Serialize)]
pub struct BoardWire {
    pub from: Reference,
    pub to: Reference,
}

/// The physical surroundings of the chip that the configuration does not
/// describe.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Board {
    pub wires: Vec<BoardWire>,
    /// OSCG base frequency in Hz, before the divider. `None` takes the nominal
    /// 310 MHz; the HC06 board measured 311.8 MHz.
    pub oscillator_hz: Option<u64>,
    /// Full DTR byte, including valid bit 7. None uses HY04 room-temperature
    /// ULX3S measurement 0x9E; this is not a Celsius scale.
    pub dtr_code: Option<u8>,
}

pub fn compile(device: Arc<ConfiguredDevice>, request: Request) -> Result<Compiled> {
    compile_on_board(device, request, &Board::default())
}

/// The plan cache's identity, also used by geometry and overlay caches.
/// It binds source bytes, the decoded device and catalogues, every request
/// and board field, and the executable hashed once per process. Cache
/// disable flags do not prevent making an identity for another cache.
pub fn cache_key(device: &ConfiguredDevice, request: &Request, board: &Board) -> Option<[u8; 32]> {
    cache::key(device, request, board)
}

/// `compile` for a chip on a board: wires between package pins and the
/// chip's own oscillator frequency.
pub fn compile_on_board(
    device: Arc<ConfiguredDevice>,
    request: Request,
    board: &Board,
) -> Result<Compiled> {
    compile_on_board_phases(device, request, board, |_| {})
}

// The ignored timing harness observes phase boundaries without adding a CLI
// option or timing overhead to the window's monomorphisation.
fn compile_on_board_phases(
    device: Arc<ConfiguredDevice>,
    request: Request,
    board: &Board,
    mut phase: impl FnMut(&str),
) -> Result<Compiled> {
    let cache =
        cache::directory().and_then(|dir| Some((dir, cache_key(&device, &request, board)?)));
    if let Some((dir, key)) = &cache {
        if let Some(compiled) = cache::read(dir, key, Arc::clone(&device), request.options) {
            phase("cache_hit");
            return Ok(compiled);
        }
    }
    phase("cache_lookup");
    let mut planner = Planner::new(device, &request)?;
    phase("planner_new");
    planner.oscillator_hz = board.oscillator_hz.unwrap_or(NOMINAL_OSCILLATOR_HZ);
    planner.dtr_code = board.dtr_code.unwrap_or(ULX3S_DTR_CODE);
    ensure!(
        planner.oscillator_hz > 0,
        "oscillator frequency must be positive"
    );
    planner.configure_configuration_ports()?;
    planner.configure_system(request.startup)?;
    let mut drivers = BTreeMap::new();
    let mut assigned = HashSet::new();
    for (reference, value) in &request.drives {
        if let Reference::System(port) = reference {
            ensure!(
                planner.system.as_ref().unwrap().inputs.contains_key(port),
                "{} is read-only or unavailable in this startup profile",
                reference.label()
            );
        }
        if let Reference::Wire(wire) = reference {
            planner
                .boundaries
                .insert(planner.index.canonical(planner.device.model(), wire)?);
        }
        let (signal, existing) = planner.reference(reference)?;
        ensure!(
            assigned.insert(signal),
            "two initial drivers name the same boundary; use one reference"
        );
        // Existing pad drivers start at Z. Their initial value is installed as
        // a construction-time override, never a fabricated timestamp-0 edge.
        let driver = if let Some(driver) = existing {
            planner.builder.initial_external_value(driver, *value)?;
            driver
        } else {
            planner.builder.external_driver(
                signal,
                &format!("boundary:{}", reference.label()),
                Strength::Strong,
                *value,
            )?
        };
        drivers.insert(reference.label(), driver);
    }
    for wire in &board.wires {
        ensure!(
            matches!(
                (&wire.from, &wire.to),
                (Reference::Pin(_), Reference::Pin(_))
            ),
            "board wires connect package pins"
        );
        let (from, _) = planner.reference(&wire.from)?;
        let (to, _) = planner.reference(&wire.to)?;
        let label = format!("board:{}->{}", wire.from.label(), wire.to.label());
        planner.builder.buffer(&label, from.into(), to)?;
    }
    let mut probes = BTreeMap::new();
    for reference in &request.probes {
        let (signal, _) = planner.reference(reference)?;
        planner.builder.watch(signal)?;
        planner.enqueue_reference(reference)?;
        probes.insert(reference.label(), signal);
        planner.report.roots.push(reference.label());
    }
    if matches!(request.scope, Scope::AllPlcRegisters) {
        for node in planner
            .nodes
            .clone()
            .into_iter()
            .filter(|n| n.name.contains(".FF"))
        {
            planner.execute_node(&node, "Q")?;
        }
        let ebr = planner
            .device
            .model()
            .resources()
            .filter(|(_, r)| r.family == "EBR")
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        for id in ebr {
            if planner.ebr_mode_name(id)? != "NONE" {
                planner.execute_ebr(id, None)?;
                planner.report.roots.push(format!(
                    "{}:memory",
                    planner.device.model().resource(id).key
                ));
            }
        }
        let dsp = planner
            .device
            .model()
            .resources()
            .filter(|(_, r)| ["MULT18", "ALU54"].contains(&r.family.as_str()))
            .map(|(id, r)| (id, r.family.clone()))
            .collect::<Vec<_>>();
        for (id, family) in dsp {
            if planner.qualified_fallback(id, ".MODE", "NONE")? != "NONE" {
                let root = if family == "ALU54" {
                    planner.execute_alu(id, "R0")?;
                    "result"
                } else {
                    planner.execute_multiplier(id, None)?;
                    "product"
                };
                planner.report.roots.push(format!(
                    "{}:{root}",
                    planner.device.model().resource(id).key
                ));
            }
        }
        // Memory writes must continue even when none of their asynchronous
        // outputs currently feeds a FF or an explicitly observed signal.
        for node in planner
            .nodes
            .clone()
            .into_iter()
            .filter(|n| n.name == "SLICEC.RAMW")
        {
            if planner
                .field(node.tile, "SLICEC.MODE")?
                .iter()
                .any(|v| v == "RAMW")
            {
                planner.execute_lutram(&node)?;
                planner.report.roots.push(format!(
                    "{}:SLICEC.RAMW",
                    planner.device.model().tile(node.tile).name
                ));
            }
        }
    }
    ensure!(
        !probes.is_empty() || matches!(request.scope, Scope::AllPlcRegisters),
        "observed scope requires at least one probe"
    );
    phase("roots_and_system");
    while !planner.queue.is_empty() {
        let batch = planner.queue.len().min(route_decode::BATCH);
        planner.routes.prepare(
            &planner.device,
            &planner.index,
            &planner.queue,
            &planner.seen,
            &planner.producers,
        );
        for _ in 0..batch {
            let wire = planner.queue.pop_front().unwrap();
            planner
                .seen
                .resize(planner.seen.len().max(wire.0 + 1), false);
            if std::mem::replace(&mut planner.seen[wire.0], true) {
                continue;
            }
            planner.execute_wire(wire).with_context(|| {
                let wire = planner.index.wire(wire);
                format!(
                    "planning R{}C{}:{}",
                    wire.location.row, wire.location.col, wire.name
                )
            })?;
        }
    }
    phase("backward_planning");
    planner.finish_plls()?;
    planner.finish_configuration_ports()?;
    phase("finish_plls");
    planner.report.undriven.sort();
    planner
        .report
        .constants
        .sort_by(|a, b| (&a.wire, &a.tile, &a.field).cmp(&(&b.wire, &b.tile, &b.field)));
    planner.report.lutram.sort_by(|a, b| a.plc.cmp(&b.plc));
    planner
        .report
        .ebr
        .sort_by(|a, b| a.resource.cmp(&b.resource));
    planner.report.resources.sort_by(|a, b| a.id.cmp(&b.id));
    planner
        .report
        .multipliers
        .sort_by(|a, b| a.resource.cmp(&b.resource));
    planner
        .report
        .clock_aliases
        .sort_by(|a, b| a.raw.cmp(&b.raw));
    planner
        .report
        .fields
        .sort_by(|a, b| (&a.tile, &a.name).cmp(&(&b.tile, &b.name)));
    phase("sort_report");
    let kernel = planner.builder.build()?;
    phase("kernel_build");
    let compiled = Compiled {
        kernel,
        drivers,
        probes,
        report: planner.report,
    };
    if let Some((dir, key)) = cache {
        cache::write(&dir, &key, &compiled);
    }
    phase("cache_write");
    Ok(compiled)
}

impl Planner {
    fn new(device: Arc<ConfiguredDevice>, request: &Request) -> Result<Self> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::new");
        ensure!(
            device
                .model()
                .package_pins()
                .any(|p| p.package == request.package),
            "unknown package {}",
            request.package
        );
        let mut builder = Builder::new(Arc::clone(&device))?;
        builder.options(request.options)?;
        let index = index::Index::new(device.model())?;
        let mut nodes = Vec::new();
        let mut node_ports = HashMap::new();
        let mut producers: HashMap<WireId, Vec<Producer>> = HashMap::new();
        for (id, resource) in device.model().resources() {
            if resource.family == "PLC2" {
                for node in device.model().nodes(id) {
                    let node_index = nodes.len();
                    let ports = node_ports
                        .entry(node.native_name.to_owned())
                        .or_insert_with(|| {
                            let mut ports = device.model().node_ports(node.id);
                            for port in &mut ports {
                                port.wire.location.row -= resource.location.row;
                                port.wire.location.col -= resource.location.col;
                            }
                            ports
                        });
                    for port in ports.iter().filter(|p| p.direction == Direction::Output) {
                        let mut wire = port.wire.clone();
                        wire.location.row += resource.location.row;
                        wire.location.col += resource.location.col;
                        producers
                            .entry(index.canonical(device.model(), &wire)?)
                            .or_default()
                            .push(Producer::Node(node_index, port.name.clone()));
                    }
                    nodes.push(Node {
                        id: node.id,
                        owner: node.owner,
                        tile: resource.anchor,
                        name: node.native_name.into(),
                    });
                }
            } else if resource.family != "SLICE" {
                for port in device
                    .model()
                    .resource_ports(id)
                    .into_iter()
                    .filter(|p| p.direction == Direction::Output)
                {
                    producers
                        .entry(index.canonical(device.model(), &port.wire)?)
                        .or_default()
                        .push(Producer::Resource(id, port.name));
                }
            }
        }
        Ok(Self {report:Report {schema_version:1,configuration_sha256:device.fingerprint().into(),scope:request.scope,startup:request.startup,
            roots:Vec::new(),counts:Counts::default(),fields:Vec::new(),constants:Vec::new(),routes:Vec::new(),excluded_routes:Vec::new(),resources:Vec::new(),clock_aliases:Vec::new(),lutram:Vec::new(),ebr:Vec::new(),multipliers:Vec::new(),system:SystemPlan::default(),undriven:Vec::new(),pio_modes:BTreeMap::new(),
            limitations:vec!["image already loaded; no physical POR, transport, START binding or inferred wake-clock timing; user GSR ASYNC/NONE and HY01 ACTIVE_LOW SYNC; zero propagation delay",
                "unsupported reachable modes fail; full behavioral coverage is not claimed",
                "observed scope does not execute unobserved sequential state; all storage remains present",
                "undriven controls remain unknown, without inferred pulldowns",
                "basic binary single-ended PIO; no electrical timing, thresholds or bank-power model",
                "LUTRAM: standard A/B DPRAM + C RAMW with zero C INIT; atomic writes, conservative X/Z policy; no hardware collision validation; C FF SD=1 data explicitly X/unmodelled",
                "EBR: DP widths 1/2/4/9/18; PDP36 write with a 1/2/4/9/18/36-bit read (36: both read paths, order and controls per ebr[].assumptions); synchronous core plus optional output stage; conservative INIT9 collision/control policy; active ASYNC reset with SYNC release rejected; no board validation",
                "DSP: parallel MULT18 with optional logical input/pipeline stages; data/sign reset, CE priority and divider semantics remain explicit D05 assumptions awaiting hardware/vendor-model checks; MULT_BYPASS/CAS_MATCH_REG/HIGHSPEED_CLK assumed at standard nextpnr defaults; arbitrary Diamond/9/36-bit modes uncertified; shared output product profile requires all effective CE=1/RST=0 and GSR disabled; registered SIGNEDP and ALU/flag/framing aliases remain unmodelled",
                "post-load profiles treat unmodelled CIBTEST access as inactive; retained separately, not hardware-validated"].into_iter().map(String::from).collect()},
            // DSP execution is qualified separately; missing MULT_BYPASS,
            // CAS_MATCH_REG and HIGHSPEED_CLK use the explicit nextpnr profile.
            device,builder,index,routes:route_decode::Decoded::default(),nodes,node_ports,producers,signals:Vec::new(),boundaries:HashSet::new(),queue:VecDeque::new(),seen:Vec::new(),clock_aliases:HashSet::new(),
            done_nodes:HashSet::new(),done_lutram:HashSet::new(),done_resources:HashSet::new(),dsp_divider_mode:None,done_iologic:HashSet::new(),fields:HashSet::new(),pads:HashMap::new(),pio_modes:HashMap::new(),plls:HashMap::new(),oscillator_hz:NOMINAL_OSCILLATOR_HZ,dtr_code:ULX3S_DTR_CODE,package:request.package.clone(),system:None})
    }
    fn signal(&mut self, raw: &Wire) -> Result<SignalId> {
        let wire = self.index.canonical(self.device.model(), raw)?;
        self.signal_id(raw, wire)
    }
    fn signal_id(&mut self, raw: &Wire, wire: WireId) -> Result<SignalId> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::signal");
        if !self.index.matches(wire, raw) && self.clock_aliases.insert(self.index.id(raw)) {
            self.report.clock_aliases.push(ClockAlias {
                raw: raw.clone(),
                segment: self.index.wire(wire),
            });
        }
        if let Some(Some(signal)) = self.signals.get(wire.0) {
            return Ok(*signal);
        }
        let signal = self.builder.signal(self.index.wire(wire))?;
        self.signals
            .resize(self.signals.len().max(wire.0 + 1), None);
        self.signals[wire.0] = Some(signal);
        Ok(signal)
    }
    fn input(&mut self, raw: &Wire) -> Result<Input> {
        let wire = self.index.canonical(self.device.model(), raw)?;
        self.input_id(raw, wire)
    }
    fn input_id(&mut self, raw: &Wire, wire: WireId) -> Result<Input> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::input");
        let signal = self.signal_id(raw, wire)?;
        self.queue.push_back(wire);
        Ok(signal.into())
    }
    fn field(&mut self, tile: TileId, name: &str) -> Result<Vec<String>> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::field");
        let values = self.device.enum_candidates(tile, name);
        self.decoded_field(tile, name, values)
    }
    fn decoded_field(
        &mut self,
        tile: TileId,
        name: &str,
        values: Result<Vec<String>>,
    ) -> Result<Vec<String>> {
        let values = values?;
        if self.fields.insert((tile, name.into())) {
            self.report.fields.push(FieldUse {
                tile: self.device.model().tile(tile).name.clone(),
                name: name.into(),
                candidates: values.clone(),
            });
        }
        Ok(values)
    }
    fn one(&mut self, tile: TileId, name: &str) -> Result<String> {
        let values = self.field(tile, name)?;
        ensure!(
            values.len() == 1,
            "ambiguous/unmatched {}:{name}: {values:?}",
            self.device.model().tile(tile).name
        );
        Ok(values[0].clone())
    }
    fn node_port(&self, node: &Node, name: &str) -> Result<Wire> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::node_port");
        self.node_ports[&node.name]
            .iter()
            .find(|p| p.name == name)
            .map(|p| {
                let mut wire = p.wire.clone();
                let location = self.device.model().resource(node.owner).location;
                wire.location.row += location.row;
                wire.location.col += location.col;
                wire
            })
            .with_context(|| format!("missing native node port {name}"))
    }
    fn resource_port(&self, id: ResourceId, name: &str) -> Result<Wire> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::resource_port");
        self.device
            .model()
            .resource_ports(id)
            .into_iter()
            .find(|p| p.name == name)
            .map(|p| p.wire)
            .with_context(|| format!("missing resource port {name}"))
    }
    fn reference(&mut self, reference: &Reference) -> Result<(SignalId, Option<DriverId>)> {
        match reference {
            Reference::System(port) => {
                let handles = self
                    .system
                    .as_ref()
                    .context("system controller not constructed")?;
                Ok((
                    *handles
                        .signals
                        .get(port)
                        .context("system reference unavailable in this startup profile")?,
                    handles.inputs.get(port).copied(),
                ))
            }
            Reference::Wire(wire) => Ok((self.signal(wire)?, None)),
            Reference::Pin(ball) => {
                let pin = self
                    .device
                    .model()
                    .package_pin(&self.package, ball)
                    .context("unknown package ball")?;
                let id = if let Some(id) = pin.pio {
                    self.pio(id)?;
                    id
                } else if pin.function == "CCLK" {
                    let id = usrmclk::resource(&self.device)?;
                    self.execute_usrmclk(id)?;
                    id
                } else {
                    bail!("package ball has no confirmed GPIO or CCLK binding");
                };
                let pad = &self.pads[&id];
                Ok((pad.signal, Some(pad.driver)))
            }
        }
    }
    fn enqueue_reference(&mut self, reference: &Reference) -> Result<()> {
        if let Reference::Wire(wire) = reference {
            self.queue
                .push_back(self.index.canonical(self.device.model(), wire)?);
        }
        Ok(())
    }
    fn selected_control(
        &mut self,
        node: &Node,
        selector: &str,
        port: &str,
        attribute: &str,
    ) -> Result<(Input, Option<String>)> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::selected_control");
        let selection = self.device.raw_mux(node.tile, selector)?;
        ensure!(
            selection.candidate_sources.len() <= 1,
            "ambiguous control selector {selector}"
        );
        let wire = self.node_port(node, port)?;
        if port == "LSR"
            && selection.candidate_sources.is_empty()
            && ["LSR0", "LSR1"]
                .iter()
                .map(|lsr| self.one(node.tile, &format!("{lsr}.LSRMUX")))
                .collect::<Result<Vec<_>>>()?
                .iter()
                .all(|mux| mux == "LSR")
        {
            // nextpnr drops a constant-0 LSR net with LSRMUX=LSR and leaves the
            // selector unrouted; on the board such flip-flops (HS01) toggled
            // normally, so the unselected local reset is inactive.
            self.report.constants.push(ConstantUse {
                tile: self.device.model().tile(node.tile).name.clone(),
                field: selector.into(),
                wire,
                encoded_value: "no selector arc; LSR0/LSR1.LSRMUX=LSR".into(),
                value: Logic::Zero,
                rule: "unselected PLC local reset is inactive: nextpnr contract, observed on the board (HS01)".into(),
            });
            return Ok((Input::Constant(Logic::Zero), None));
        }
        let input = self.input(&wire)?;
        let Some(source) = selection.candidate_sources.first() else {
            return Ok((input, None));
        };
        let value = self.one(node.tile, &format!("{source}.{attribute}"))?;
        match value.as_str() {
            "INV" => Ok((Input::Inverted(self.signal(&wire)?), Some(source.clone()))),
            "CLK" | "LSR" => Ok((input, Some(source.clone()))),
            _ => bail!("unsupported control inversion {attribute}={value}"),
        }
    }
    fn tied_input(&mut self, node: &Node, slice: &str, local: &str, port: &str) -> Result<Input> {
        match self
            .one(node.tile, &format!("{slice}.{local}MUX"))?
            .as_str()
        {
            "0" => Ok(Input::Constant(Logic::Zero)),
            "1" => Ok(Input::Constant(Logic::One)),
            value if value == local => self.input(&self.node_port(node, port)?),
            value => bail!("unsupported input mux {slice}.{local}MUX={value}"),
        }
    }
    fn execute_node(&mut self, node: &Node, port: &str) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::execute_node");
        let (slice, local) = node.name.split_once('.').context("invalid node name")?;
        let z = usize::from(slice.as_bytes()[5] - b'A');
        if local.starts_with("FF") {
            if !self.done_nodes.insert(node.id) {
                return Ok(());
            }
            let k = local.strip_prefix("FF").unwrap();
            let base = format!("{slice}.REG{k}");
            let preload = match self.one(node.tile, &format!("{base}.LSRMODE"))?.as_str() {
                "LSR" => None,
                "PRLD" => Some(self.tied_input(node, slice, &format!("M{k}"), "M")?),
                value => bail!("unsupported LSRMODE {value}: {base}"),
            };
            let data = match self.one(node.tile, &format!("{base}.SD"))?.as_str() {
                "1" if slice == "SLICEC" && self.one(node.tile, "SLICEC.MODE")? == "RAMW" => {
                    // C's COMB positions are reserved by RAMW. We have no
                    // established DI function for their FFs in this mode.
                    // Keep the physical FF/reset/CE behavior, with explicit X
                    // data and a recorded boundary, rather than executing the
                    // reserved C INIT as an ordinary LUT. SD=0 still uses M.
                    self.execute_lutram(node)?;
                    let plc = &self.device.model().tile(node.tile).name;
                    self.report
                        .lutram
                        .iter_mut()
                        .find(|r| &r.plc == plc)
                        .unwrap()
                        .unmodelled_c_ff_di
                        .push(node.name.clone());
                    Input::Constant(Logic::X)
                }
                "1" => self.input(&self.node_port(node, "DI")?)?,
                "0" => self.tied_input(node, slice, &format!("M{k}"), "M")?,
                value => bail!("unknown SD {value}"),
            };
            let (clock, _) = self.selected_control(node, &format!("MUXCLK{z}"), "CLK", "CLKMUX")?;
            let (reset, selected_lsr) =
                self.selected_control(node, &format!("MUXLSR{z}"), "LSR", "LSRMUX")?;
            let enable = match self.one(node.tile, &format!("{slice}.CEMUX"))?.as_str() {
                "0" => Input::Constant(Logic::Zero),
                "1" => Input::Constant(Logic::One),
                "CE" => self.input(&self.node_port(node, "CE")?)?,
                "INV" => {
                    let wire = self.node_port(node, "CE")?;
                    self.input(&wire)?;
                    Input::Inverted(self.signal(&wire)?)
                }
                value => bail!("unsupported CE {value}"),
            };
            let kind = if let Some(lsr) = selected_lsr {
                match self.one(node.tile, &format!("{lsr}.SRMODE"))?.as_str() {
                    "ASYNC" => ResetKind::Asynchronous,
                    "LSR_OVER_CE" => ResetKind::Synchronous,
                    value => bail!("unsupported SRMODE {value}"),
                }
            } else {
                ResetKind::Synchronous
            };
            let value = match self.one(node.tile, &format!("{base}.REGSET"))?.as_str() {
                "SET" => Logic::One,
                "RESET" => Logic::Zero,
                value => bail!("unsupported REGSET {value}"),
            };
            let global_reset = match self.one(node.tile, &format!("{slice}.GSR"))?.as_str() {
                "ENABLED" => {
                    self.report.system.gsr_enabled_ff += 1;
                    Some(kernel::GlobalReset {
                        input: self.system_input(SystemPort::Gsr)?,
                        value,
                    })
                }
                "DISABLED" => {
                    self.report.system.gsr_disabled_ff += 1;
                    None
                }
                value => bail!("unsupported slice GSR {value}"),
            };
            if preload.is_none() && matches!(self.report.startup, Startup::Configured) {
                // LSR FFs start at REGSET. PRLD stays unknown until M settles,
                // so a feedback M cannot acquire an invented REGSET value.
                self.builder.initial_register(node.id, value)?;
            }
            self.builder.flip_flop_with_global_reset(
                FlipFlop {
                    preload,
                    node: node.id,
                    data,
                    clock,
                    enable,
                    edge: kernel::Edge::Rising,
                    reset: Some(Reset {
                        input: reset,
                        kind,
                        value,
                    }),
                },
                global_reset,
            )?;
            if preload.is_some() && matches!(self.report.startup, Startup::Configured) {
                self.builder.initial_preload_register(node.id)?;
            }
            self.report.counts.flip_flops += 1;
        } else if let Some(k) = local.strip_prefix('K') {
            let mode = self.one(node.tile, &format!("{slice}.MODE"))?;
            if mode == "DPRAM" && port == "F" {
                return self.execute_lutram(node);
            }
            ensure!(
                ["LOGIC", "CCU2"].contains(&mode.as_str()),
                "unsupported PLC mode {slice}={mode}"
            );
            if port == "OFX" {
                ensure!(
                    mode == "LOGIC",
                    "OFX in CCU2 mode needs a separate qualified model"
                );
                let output = self.signal(&self.node_port(node, "OFX")?)?;
                let select = self.tied_input(node, slice, &format!("M{k}"), "M")?;
                let (low, high) = if k == "0" {
                    (
                        self.input(&self.node_port(node, "F")?)?,
                        self.input(&self.node_port(node, "F1")?)?,
                    )
                } else {
                    (
                        self.input(&self.resource_port(node.owner, "FXA")?)?,
                        self.input(&self.resource_port(node.owner, "FXB")?)?,
                    )
                };
                let label = format!(
                    "wide:{}:{}",
                    self.device.model().tile(node.tile).name,
                    node.name
                );
                if self.builder.has_source(&label) {
                    return Ok(());
                }
                self.builder.mux(&label, select, low, high, output)?;
                self.report.counts.wide_muxes += 1;
                return Ok(());
            }
            if port == "FCO" && mode == "LOGIC" {
                // S04: no carry function is established for a LOGIC slice.
                // nextpnr starts chains in the next PLC with a CCU2 half whose
                // LUT output selects the generate term, so CIN is not used;
                // an explicit X keeps any real use of it visible.
                let wire = self.node_port(node, "FCO")?;
                let output = self.signal(&wire)?;
                self.builder.static_driver(
                    output,
                    &format!(
                        "unknown-carry:R{}C{}:{}",
                        wire.location.row, wire.location.col, wire.name
                    ),
                    Strength::Strong,
                    Logic::X,
                )?;
                self.report.constants.push(ConstantUse {
                    tile: self.device.model().tile(node.tile).name.clone(),
                    field: format!("{}.FCO", node.name),
                    wire,
                    encoded_value: "LOGIC mode".into(),
                    value: Logic::X,
                    rule:
                        "carry out of a LOGIC-mode slice: function not established (S04), driven X"
                            .into(),
                });
                return Ok(());
            }
            ensure!(
                port == "F" || (port == "FCO" && mode == "CCU2"),
                "unsupported node output {}.{port} in {mode}",
                node.name
            );
            if !self.done_nodes.insert(node.id) {
                return Ok(());
            }
            let mut inputs = [Input::Constant(Logic::X); 4];
            for (index, p) in ["A", "B", "C", "D"].into_iter().enumerate() {
                inputs[index] = self.tied_input(node, slice, &format!("{p}{k}"), p)?;
            }
            if mode == "LOGIC" {
                self.builder.lut4_with_inputs(node.id, inputs)?;
                self.report.counts.lut4 += 1;
            } else {
                let inject = self.one(node.tile, &format!("{slice}.CCU2.INJECT1_{k}"))?;
                ensure!(
                    ["YES", "NO"].contains(&inject.as_str()),
                    "unsupported INJECT1"
                );
                let cin = self.input(&self.node_port(node, "FCI")?)?;
                self.builder.carry(node.id, inputs, cin, inject == "YES")?;
                self.report.counts.carry_halves += 1;
            }
        } else if local == "RAMW" {
            self.execute_lutram(node)?;
        } else {
            bail!("unsupported node {}", node.name);
        }
        Ok(())
    }
    fn composed(&self, id: ResourceId, suffix: &str) -> Result<Vec<String>> {
        let result = self
            .device
            .resource_settings(id)
            .context("resource has no composed configuration")?;
        let fields = result
            .fields
            .iter()
            .filter(|f| f.name.ends_with(suffix))
            .collect::<Vec<_>>();
        ensure!(
            fields.len() == 1,
            "missing or multiple composed fields: {suffix}"
        );
        let f = fields[0];
        ensure!(
            ["resolved", "ambiguous"].contains(&f.status.as_str()) && !f.candidates.is_empty(),
            "unresolved resource field {}: {}",
            f.name,
            f.status
        );
        Ok(f.candidates.clone())
    }
    fn record_resource(&mut self, id: ResourceId, rule: &'static str) {
        let resource = self.device.model().resource(id);
        let fields = self
            .device
            .resource_settings(id)
            .map(|r| {
                r.fields
                    .iter()
                    .map(|f| ResourceField {
                        name: f.name.clone(),
                        status: f.status.clone(),
                        candidates: f.candidates.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.report.resources.push(ResourceUse {
            id: resource.key.clone(),
            family: resource.family.clone(),
            rule: rule.into(),
            fields,
        });
    }
    fn composed_one(&self, id: ResourceId, suffix: &str) -> Result<String> {
        let values = self.composed(id, suffix)?;
        ensure!(values.len() == 1, "ambiguous {suffix}: {values:?}");
        Ok(values[0].clone())
    }
    fn qualified_fallback(&self, id: ResourceId, suffix: &str, fallback: &str) -> Result<String> {
        let result = self
            .device
            .resource_settings(id)
            .context("missing composed settings")?;
        let fields = result
            .fields
            .iter()
            .filter(|f| f.name.ends_with(suffix))
            .collect::<Vec<_>>();
        ensure!(fields.len() == 1, "missing or multiple qualified fields");
        let f = fields[0];
        if f.status == "unobserved" && !f.observed_bits.is_empty() && f.candidates == [fallback] {
            return Ok(fallback.into());
        }
        self.composed_one(id, suffix)
    }
    fn pio(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let resource = self.device.model().resource(id);
        let key = resource.key.clone();
        // BASE_TYPE shares bits with DRIVE/PULLMODE/OPENDRAIN/HYSTERESIS/CLAMP;
        // the decoder inverts the nextpnr/libtrellis writer (decode/pio.rs).
        if !self.pio_modes.contains_key(&key) {
            let modes = self.device.pio_modes(id)?;
            self.pio_modes.extend(modes);
        }
        let mode = self
            .pio_modes
            .get(&key)
            .cloned()
            .with_context(|| format!("PIO without a decoded mode: {key}"))?;
        self.report.pio_modes.insert(key.clone(), mode.clone());
        if let Some(true_key) = &mode.complement_of {
            return self.pio_complement(id, true_key);
        }
        let values = &mode.base_types;
        ensure!(
            ["resolved", "ambiguous"].contains(&mode.status.as_str()),
            "{} PIO mode {key}: {values:?}",
            mode.status
        );
        // Identical bits can stand for several writer inputs. A driver or
        // receiver present in every consistent mode is built; one present in
        // only some is ambiguous and fails only when the plan uses it.
        let directions: BTreeSet<&str> = values
            .iter()
            .map(|v| v.split_once('_').map_or("NONE", |(d, _)| d))
            .collect();
        let all = |set: [&str; 2]| directions.iter().all(|d| set.contains(d));
        let none = |set: [&str; 2]| directions.iter().all(|d| !set.contains(d));
        let drives = all(["OUTPUT", "BIDIR"]);
        let receives = all(["INPUT", "BIDIR"]);
        ensure!(
            !directions.contains("NONE") && (drives || none(["OUTPUT", "BIDIR"])),
            "inactive/unknown or ambiguous BASE_TYPE {key}: {values:?}"
        );
        let receiver_ambiguous = !receives && !none(["INPUT", "BIDIR"]);
        let standards: Vec<&str> = values
            .iter()
            .filter_map(|v| v.split_once('_').map(|(_, s)| s))
            .collect();
        let differential = self.pio_differential(&key, &mode)?;
        ensure!(
            differential
                || standards.iter().any(|s| [
                    "LVCMOS12", "LVCMOS15", "LVCMOS18", "LVCMOS25", "LVCMOS33", "LVTTL33"
                ]
                .contains(s)),
            "unsupported electrical BASE_TYPE: {values:?}"
        );
        let pullmode = receives
            .then(|| self.pio_setting(id, &mode, "PULLMODE", None))
            .transpose()?;
        // nextpnr writes OPENDRAIN only on request. Unwritten, the output
        // stays push-pull: HS01 LEDs with DRIVE=4 drove high on the board.
        let opendrain = drives
            .then(|| self.pio_setting(id, &mode, "OPENDRAIN", Some("OFF")))
            .transpose()?;
        ensure!(
            !differential || opendrain.as_deref().is_none_or(|v| v == "OFF"),
            "unsupported differential IO_TYPE {standards:?} on {key}: open-drain pair has no complementary push-pull digital model"
        );
        let location = resource.location;
        let pad = self.pio_pad(id, receives, receiver_ambiguous)?;
        if differential {
            let (site, name) = key.rsplit_once('/').context("PIO key")?;
            let other = match name {
                "PIOA" => "PIOB",
                "PIOC" => "PIOD",
                _ => bail!("unsupported differential IO_TYPE {standards:?} on {key}: positive pad must be A or C"),
            };
            let other_key = format!("{site}/{other}");
            ensure!(
                self.pio_modes.get(&other_key).and_then(|m| m.complement_of.as_deref()) == Some(key.as_str()),
                "unsupported differential IO_TYPE {standards:?} on {key}: ambiguous or missing complement {other_key}"
            );
            let other_id = self
                .device
                .model()
                .resource_id(&other_key)
                .context("missing complement PIO")?;
            if drives {
                self.pio(other_id)?;
            }
        }
        if receives {
            let output = self.signal(&self.resource_port(id, "O")?)?;
            self.builder
                .receiver(&format!("receiver:{key}"), pad.into(), output)?;
            match pullmode.as_deref().unwrap_or_default() {
                "UP" => self.builder.static_driver(
                    pad,
                    &format!("pull:{key}"),
                    Strength::Pull,
                    Logic::One,
                )?,
                "DOWN" => self.builder.static_driver(
                    pad,
                    &format!("pull:{key}"),
                    Strength::Pull,
                    Logic::Zero,
                )?,
                "NONE" => {}
                value => bail!("unsupported PULLMODE {value}"),
            }
        }
        if drives {
            let (data, tristate) = self.pio_driver_inputs(id)?;
            let data = match opendrain.as_deref().unwrap_or_default() {
                "OFF" => data,
                "ON" => {
                    let internal = self.signal(&Wire {
                        location,
                        name: format!("OPEN_DRAIN_{}", self.device.model().resource(id).name),
                    })?;
                    self.builder.mux(
                        &format!("open-drain:{key}"),
                        data,
                        Input::Constant(Logic::Zero),
                        Input::Constant(Logic::Z),
                        internal,
                    )?;
                    internal.into()
                }
                value => bail!("unsupported OPENDRAIN {value}"),
            };
            self.pad_output_driver(id, data, tristate, pad)?;
        }
        ensure!(drives || receives, "PIO {key} has no digital direction");
        self.report.counts.pio += 1;
        self.record_resource(id, if differential {
            "digital differential PIO: output P=data and N=inverted data with shared tristate/GOE; input reads P only; negative leg and invalid-pair behavior are not modelled; electrical standard, voltage, termination and swing have no digital effect; joint BASE_TYPE aliases retain pair ownership"
        } else {
            "binary single-ended PIO; BASE_TYPE decoded jointly with the settings written over it (pio_mode; the single-ended IO standard may stay ambiguous); unique PADDO/PADDT empty fallbacks are qualified by mode; raw field statuses retained"
        });
        Ok(())
    }
    /// The pad of a PIO with its external driver, which starts at Z.
    fn pio_pad(
        &mut self,
        id: ResourceId,
        receiver_enabled: bool,
        receiver_ambiguous: bool,
    ) -> Result<SignalId> {
        let name = format!(
            "PAD_{}",
            self.device
                .model()
                .resource(id)
                .name
                .strip_prefix("PIO")
                .context("PIO name")?
        );
        self.pad(id, &name, receiver_enabled, receiver_ambiguous)
    }
    /// A digital pad with an external driver, initially released.
    fn pad(
        &mut self,
        id: ResourceId,
        name: &str,
        receiver_enabled: bool,
        receiver_ambiguous: bool,
    ) -> Result<SignalId> {
        self.check_configuration_pad(id)?;
        let resource = self.device.model().resource(id);
        let key = resource.key.clone();
        let pad_wire = Wire {
            location: resource.location,
            name: name.into(),
        };
        let pad = self.signal(&pad_wire)?;
        let driver = self.builder.external_driver(
            pad,
            &format!("external:{key}"),
            Strength::Strong,
            Logic::Z,
        )?;
        self.pads.insert(
            id,
            Pad {
                signal: pad,
                driver,
                receiver_enabled,
                receiver_ambiguous,
            },
        );
        Ok(pad)
    }
    /// Data and tristate inputs of a driving PIO: from the fabric or, per
    /// DATAMUX/TRIMUX, from the IOLOGIC registers (compile/iologic.rs).
    fn pio_driver_inputs(&mut self, id: ResourceId) -> Result<(Input, Input)> {
        let key = self.device.model().resource(id).key.clone();
        let (data_port, tristate_port) = self.pio_output_sources(id)?;
        let port = |name: &str| {
            self.resource_port(id, name)
                .with_context(|| format!("PIO {key} output path {name}"))
        };
        let (data, tristate) = (port(data_port)?, port(tristate_port)?);
        Ok((self.input(&data)?, self.input(&tristate)?))
    }
    /// Drives `pad` with `data` unless `tristate` is high, and only while the
    /// global output enable is active.
    fn pad_output_driver(
        &mut self,
        id: ResourceId,
        data: Input,
        tristate: Input,
        pad: SignalId,
    ) -> Result<()> {
        let resource = self.device.model().resource(id);
        let (key, location, name) = (
            resource.key.clone(),
            resource.location,
            resource.name.clone(),
        );
        let user_output = self.signal(&Wire {
            location,
            name: format!("USER_OUTPUT_{name}"),
        })?;
        self.builder.mux(
            &format!("output:{key}"),
            tristate,
            data,
            Input::Constant(Logic::Z),
            user_output,
        )?;
        self.builder.mux(
            &format!("goe:{key}"),
            self.system_input(SystemPort::Goe)?,
            Input::Constant(Logic::Z),
            user_output.into(),
            pad,
        )
    }
    /// Qualify all bit-identical IO_TYPE alternatives before assigning a pair.
    fn pio_differential(&self, key: &str, mode: &crate::decode::PioMode) -> Result<bool> {
        let standards: Vec<_> = mode
            .base_types
            .iter()
            .filter_map(|b| b.split_once('_'))
            .collect();
        let any = standards
            .iter()
            .any(|(_, s)| crate::decode::differential(s));
        ensure!(
            !any || standards.iter().all(|(_, s)| crate::decode::differential(s)),
            "unsupported IO_TYPE alternatives {:?} on {key}: ambiguous single-ended/differential pair ownership", mode.base_types
        );
        ensure!(
            !any || standards.iter().all(|(d, _)| ["INPUT", "OUTPUT"].contains(d)),
            "unsupported differential IO_TYPE {:?} on {key}: bidirectional differential receiver/driver switching is not implemented (including pseudo-differential bidir)", mode.base_types
        );
        ensure!(
            !any || matches!(mode.direction.as_deref(), Some("INPUT" | "OUTPUT")),
            "unsupported differential IO_TYPE {:?} on {key}: ambiguous input/output pair direction",
            mode.base_types
        );
        Ok(any)
    }
    /// The complement PIO (B/D) belongs to the true PIO (A/C). Inputs expose
    /// its external pad only; outputs share data, tristate and GOE with P.
    fn pio_complement(&mut self, id: ResourceId, true_key: &str) -> Result<()> {
        let key = self.device.model().resource(id).key.clone();
        let true_id = self
            .device
            .model()
            .resource_id(true_key)
            .with_context(|| format!("no true PIO {true_key} for {key}"))?;
        let true_mode = self
            .pio_modes
            .get(true_key)
            .cloned()
            .with_context(|| format!("PIO without a decoded mode: {true_key}"))?;
        ensure!(
            self.pio_differential(true_key, &true_mode)?,
            "complement without differential IO_TYPE: {true_key}"
        );
        let drives = true_mode.direction.as_deref() == Some("OUTPUT");
        ensure!(
            !drives || self.pio_setting(true_id, &true_mode, "OPENDRAIN", Some("OFF"))? == "OFF",
            "unsupported differential IO_TYPE {:?} on {true_key}: open-drain pair has no complementary push-pull digital model", true_mode.base_types
        );
        let pad = self.pio_pad(id, false, false)?;
        if drives {
            let (data, tristate) = self.pio_driver_inputs(true_id)?;
            self.pad_output_driver(id, data.inverted(), tristate, pad)?;
        }
        self.report.counts.pio += 1;
        self.record_resource(id, "complement PIO of a differential pair: outputs invert the true PIO data with shared tristate/GOE; inputs leave the negative pad unmodelled; nextpnr leaves its BASE_TYPE unwritten; independent receiver unused");
        Ok(())
    }
    /// A PIO setting from the joint mode when it is written over BASE_TYPE,
    /// otherwise from the independently decoded field. `unwritten` names the
    /// value a writer that never set the enum leaves behind.
    fn pio_setting(
        &self,
        id: ResourceId,
        mode: &crate::decode::PioMode,
        name: &str,
        unwritten: Option<&str>,
    ) -> Result<String> {
        let Some(values) = mode.overlays.get(name) else {
            return self.composed_one(id, &format!(".{name}"));
        };
        let values: BTreeSet<String> = values
            .iter()
            .map(|v| match (v.as_str(), unwritten) {
                ("unwritten", Some(u)) => u.to_string(),
                _ => v.clone(),
            })
            .collect();
        ensure!(
            values.len() == 1,
            "ambiguous PIO {} {name}: {values:?}",
            self.device.model().resource(id).key
        );
        Ok(values.into_iter().next().unwrap())
    }
    fn resource(&mut self, id: ResourceId, port: &str) -> Result<()> {
        let resource = self.device.model().resource(id);
        if resource.family == "PIO" {
            ensure!(port == "O", "unknown PIO output");
            self.pio(id)?;
            ensure!(
                self.pads[&id].receiver_enabled,
                "PIO receiver is {} in this mode",
                if self.pads[&id].receiver_ambiguous {
                    "ambiguous (some consistent modes have none)"
                } else {
                    "disabled"
                }
            );
            Ok(())
        } else if resource.family == "USRMCLK" {
            self.execute_usrmclk(id)
        } else if resource.family == "EBR" {
            self.execute_ebr(id, Some(port))
        } else if resource.family == "MULT18" {
            self.execute_multiplier(id, Some(port))
        } else if resource.family == "ALU54" {
            self.execute_alu(id, port)
        } else if resource.family == "PLL" {
            self.execute_pll(id, port)
        } else if resource.family.ends_with("IOLOGIC") {
            self.execute_iologic(id, port)
        } else if resource.family == "DTR" {
            self.execute_dtr(id)
        } else if resource.family == "OSCG" {
            self.execute_oscillator(id, port)
        } else if resource.family == "ECLKSYNCB" || resource.family == "BRGECLKSYNC" {
            // BRGECLKSYNC is an ECLKSYNCB after the bridge (MODE ECLKSYNCB).
            self.execute_eclksync(id)
        } else if resource.family == "ECLKBRIDGECS" {
            self.execute_eclkbridge(id)
        } else if resource.family == "TRELLIS_ECLKBUF" {
            self.execute_eclkbuf(id)
        } else if resource.family == "CLKDIVF" {
            self.execute_clkdiv(id)
        } else if resource.family == "DCS" {
            self.execute_dcs(id)
        } else if resource.family.starts_with("DCC") {
            if !self.done_resources.insert(id) {
                return Ok(());
            }
            let mode = self.qualified_fallback(id, ".MODE", "NONE")?;
            if mode == "DCCA" {
                // CE taken at each rising edge of CLKI decides whether that
                // high phase passes, whole; CLKO falls with CLKI (HC01).
                let clock = self.input(&self.resource_port(id, "CLKI")?)?;
                let enable = self.input(&self.resource_port(id, "CE")?)?;
                let output = self.signal(&self.resource_port(id, "CLKO")?)?;
                self.builder.iologic(crate::kernel::IoLogicConfig {
                    resource: id,
                    name: "DCCA",
                    path: crate::kernel::IoPath::ClockGate { enable, output },
                    clock,
                    reset: None,
                })?;
                self.report.counts.dcc_gates += 1;
                self.record_resource(id, "MODE=DCCA clock gate: CE taken at each CLKI rising edge passes or stops that whole high phase, CLKO falls with CLKI (HC01, docs/inventory/LFE5U-85F/hc01_dcca); starts closed until the first rising edge (not measured)");
                return Ok(());
            }
            ensure!(mode == "NONE", "DCC MODE {mode} is not implemented");
            let label = format!("dcc-bypass:{}", resource.key);
            let input = self.input(&self.resource_port(id, "CLKI")?)?;
            let output = self.signal(&self.resource_port(id, "CLKO")?)?;
            self.builder.buffer(&label, input, output)?;
            self.report.counts.dcc_bypasses += 1;
            self.record_resource(id,"MODE=NONE static bypass; unique empty NONE fallback is qualified by DCC mode contract");
            Ok(())
        } else {
            bail!(
                "unsupported reachable resource {} ({}) output {port}",
                resource.key,
                resource.family
            );
        }
    }
    /// One decoded PLL setting by exact name; an enum must have a single
    /// candidate and a word must be fully resolved (returned as an integer).
    fn pll_setting(&self, id: ResourceId, name: &str) -> Result<String> {
        let key = &self.device.model().resource(id).key;
        let field = self
            .device
            .resource_settings(id)
            .and_then(|r| r.fields.iter().find(|f| f.name == name))
            .with_context(|| format!("PLL {key} has no {name} setting"))?;
        if field.kind == "word" {
            let value = field
                .value
                .as_deref()
                .with_context(|| format!("PLL {key} {name} is {}", field.status))?;
            return Ok(u64::from_str_radix(value, 2)?.to_string());
        }
        ensure!(
            field.candidates.len() == 1,
            "PLL {key} {name} is ambiguous: {:?}",
            field.candidates
        );
        Ok(field.candidates[0].clone())
    }
    /// Interpret the complete shared selector, not a subset enum match. HC08
    /// qualifies two INT_*/fabric unions; canonical INT_OS3 also sets mux bits
    /// but remains internal. Derive physical coordinates from each site's DB.
    fn pll_feedback(&self, id: ResourceId) -> Result<(PllFeedback, String)> {
        let model = self.device.model();
        let key = &model.resource(id).key;
        let field = self.device.resource_settings(id)
            .and_then(|r| r.fields.iter().find(|f| f.name == "FEEDBK_PATH"))
            .with_context(|| format!("PLL {key} FEEDBK_PATH missing, CLKFB source unknown; HC08 requires a complete selector"))?;
        let observed: BTreeSet<_> = field
            .observed_bits
            .iter()
            .map(|&(f, b, _)| (f, b))
            .collect();
        for (_, tile) in model.tiles() {
            let tt = &model.database().types[&tile.kind];
            let Some(def) = tt.enums.get("FEEDBK_PATH") else {
                continue;
            };
            let mask: BTreeSet<_> = def
                .patterns
                .values()
                .flatten()
                .map(|b| (b.frame, b.bit))
                .collect();
            let global: BTreeSet<_> = mask
                .iter()
                .map(|&(f, b)| (tile.cram.start_frame + f, tile.cram.start_bit + b))
                .collect();
            if mask.is_empty() || global != observed {
                continue;
            }
            let ones: BTreeSet<_> = mask
                .iter()
                .copied()
                .filter(|&(f, b)| {
                    self.device
                        .image()
                        .cram()
                        .bit(tile.cram.start_frame + f, tile.cram.start_bit + b)
                })
                .collect();
            let asserted = |bits: &[BitConstraint]| -> BTreeSet<_> {
                bits.iter()
                    .filter(|b| b.value)
                    .map(|b| (b.frame, b.bit))
                    .collect()
            };
            let sink = tt
                .muxes
                .keys()
                .find(|s| s.ends_with("_CLKFB"))
                .with_context(|| {
                    format!(
                        "PLL {key} FEEDBK_PATH {:?}, CLKFB source unknown; HC08 missing mux",
                        field.candidates
                    )
                })?;
            let tile_id = model
                .tile_id(&tile.name)
                .context("PLL selector tile missing")?;
            let mux = self.device.raw_mux(tile_id, sink)?;
            let description = format!(
                "FEEDBK_PATH {:?}, selector bits {ones:?}, CLKFB source {:?}",
                field.candidates, mux.candidate_sources
            );
            for (selector, output) in [
                ("INT_OP", "CLKOP"),
                ("INT_OS", "CLKOS"),
                ("INT_OS2", "CLKOS2"),
                ("INT_OS3", "CLKOS3"),
            ] {
                if def
                    .patterns
                    .get(selector)
                    .is_some_and(|bits| asserted(bits) == ones)
                {
                    return Ok((PllFeedback::Internal(output), description));
                }
            }
            if mux.candidate_sources.len() == 1 {
                let source = &mux.candidate_sources[0];
                if let Some(arc) = tt
                    .routing
                    .iter()
                    .find(|a| a.sink == *sink && a.source == *source)
                {
                    let route = asserted(&arc.bits);
                    if ones == route && source.contains("_JCLKFB") {
                        return Ok((PllFeedback::External, description));
                    }
                    if source.ends_with("_JCLKFB3")
                        && ["INT_OP", "INT_OS"].iter().any(|selector| {
                            def.patterns.get(*selector).is_some_and(|bits| {
                                asserted(bits)
                                    .union(&route)
                                    .copied()
                                    .collect::<BTreeSet<_>>()
                                    == ones
                            })
                        })
                    {
                        return Ok((PllFeedback::RawVco, description));
                    }
                }
            }
            bail!("PLL {key} {description}: unmeasured feedback combination; HC08 qualifies only INT_OP/INT_OS with fabric JCLKFB3 returned from the same PLL's CLKOP");
        }
        bail!("PLL {key} FEEDBK_PATH {:?}, CLKFB source unknown: unresolved selector bits; HC08 requires a complete physical pattern", field.candidates)
    }
    /// EHXPLLL with static settings and a periodic reference (kernel/pll.rs).
    /// Settings are checked and outputs created while planning; the generator
    /// is built in `finish_plls`, once the external CLKFB route is known.
    fn execute_pll(&mut self, id: ResourceId, port: &str) -> Result<()> {
        let key = self.device.model().resource(id).key.clone();
        if !self.plls.contains_key(&id) {
            let setting = |name: &str| self.pll_setting(id, name);
            let number = |name: &str| -> Result<u64> { Ok(setting(name)?.parse()?) };
            ensure!(
                setting("MODE")? == "EHXPLLL",
                "PLL {key} MODE is not EHXPLLL"
            );
            for (name, expected) in [
                ("DPHASE_SOURCE", "DISABLED"),
                ("SYNC_ENABLE", "DISABLED"),
                ("REFIN_RESET", "DISABLED"),
                ("INTFB_WAKE", "DISABLED"),
                ("OUTDIVIDER_MUXA", "DIVA"),
                ("OUTDIVIDER_MUXB", "DIVB"),
                ("OUTDIVIDER_MUXC", "DIVC"),
                ("OUTDIVIDER_MUXD", "DIVD"),
                ("CLKOP_TRIM_POL", "RISING"),
                ("CLKOP_TRIM_DELAY", "0"),
                ("CLKOS_TRIM_POL", "RISING"),
                ("CLKOS_TRIM_DELAY", "0"),
            ] {
                let value = setting(name)?;
                ensure!(
                    value == expected,
                    "PLL {key} {name}={value} is not implemented (only {expected})"
                );
            }
            let (feedback_kind, feedback_description) = self.pll_feedback(id)?;
            // nextpnr stores dividers minus one and phases directly.
            let mut outputs = BTreeMap::new();
            for output in ["CLKOP", "CLKOS", "CLKOS2", "CLKOS3"] {
                let eighths =
                    8 * number(&format!("{output}_CPHASE"))? + number(&format!("{output}_FPHASE"))?;
                let enabled = setting(&format!("{output}_ENABLE"))? == "ENABLED";
                outputs.insert(
                    output,
                    (
                        number(&format!("{output}_DIV"))? + 1,
                        eighths as i64,
                        enabled,
                    ),
                );
            }
            let (clki, clkfb) = (number("CLKI_DIV")? + 1, number("CLKFB_DIV")? + 1);
            // RST and STDBY act only where enabled; disabled, a pulsed input
            // changed nothing on the board (HC07).
            let enabled: Vec<&str> = [("PLLRST_ENA", "RST"), ("STDBY_ENABLE", "STDBY")]
                .into_iter()
                .map(|(enable, port)| Ok((setting(enable)? == "ENABLED").then_some(port)))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect();
            let mut controls = Vec::new();
            for port in enabled {
                let control = self.input(&self.resource_port(id, port)?)?;
                if !matches!(control, Input::Constant(Logic::Zero)) {
                    controls.push(control);
                }
            }
            ensure!(
                feedback_kind != PllFeedback::RawVco || controls.is_empty(),
                "PLL {key} {feedback_description}: HC08 raw-VCO RST/STDBY recovery is unmeasured"
            );
            let reference = self.input(&self.resource_port(id, "CLKI")?)?;
            // Only exact canonical internal selectors use CLKINTFB. HC08's
            // combined patterns trace CLKFB to qualify the measured return.
            let feedback_port = if matches!(feedback_kind, PllFeedback::Internal(_)) {
                "CLKINTFB"
            } else {
                "CLKFB"
            };
            let feedback_wire = self.resource_port(id, feedback_port)?;
            let feedback = self.signal(&feedback_wire)?;
            self.plls.insert(
                id,
                PendingPll {
                    reference,
                    feedback,
                    feedback_kind,
                    feedback_description,
                    clki,
                    clkfb,
                    outputs,
                    used: BTreeMap::new(),
                    controls,
                },
            );
            self.input(&feedback_wire).with_context(|| {
                format!(
                    "PLL {key} {}: HC08 could not resolve the CLKFB return",
                    self.plls[&id].feedback_description
                )
            })?;
        }
        match port {
            "CLKOP" | "CLKOS" | "CLKOS2" | "CLKOS3" => {
                let port = ["CLKOP", "CLKOS", "CLKOS2", "CLKOS3"]
                    .into_iter()
                    .find(|p| *p == port)
                    .unwrap();
                ensure!(
                    self.plls[&id].outputs[port].2,
                    "PLL {key} output {port} is disabled"
                );
                let signal = self.signal(&self.resource_port(id, port)?)?;
                self.plls.get_mut(&id).unwrap().used.insert(port, signal);
            }
            "LOCK" | "INTLOCK" | "CLKINTFB" => {
                let port = ["LOCK", "INTLOCK", "CLKINTFB"]
                    .into_iter()
                    .find(|p| *p == port)
                    .unwrap();
                let signal = self.signal(&self.resource_port(id, port)?)?;
                self.plls.get_mut(&id).unwrap().used.insert(port, signal);
            }
            _ => bail!("PLL {key} output {port} is not implemented"),
        }
        Ok(())
    }
    /// Builds every planned PLL now that its CLKFB route is known.
    fn finish_plls(&mut self) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::finish_plls");
        let mut plls: Vec<_> = std::mem::take(&mut self.plls).into_iter().collect();
        plls.sort_by(|a, b| {
            let key = |id| &self.device.model().resource(id).key;
            key(a.0).cmp(key(b.0))
        });
        for (id, pll) in plls {
            let key = self.device.model().resource(id).key.clone();
            // External CLKFB follows directed route buffers; internal feedback
            // starts directly at CLKINTFB, qualified by FEEDBK_PATH above.
            let root = self.builder.buffer_root(pll.feedback)?;
            let source = pll
                .used
                .iter()
                .find(|(_, &signal)| signal == root)
                .map(|(&port, _)| port);
            let feedback = match (source, pll.feedback_kind) {
                (Some("CLKINTFB"), PllFeedback::Internal(output)) => output,
                (Some(output), PllFeedback::External) if pll.outputs.contains_key(output) => output,
                (Some("CLKOP"), PllFeedback::RawVco) => kernel::PllConfig::VCO_FEEDBACK,
                _ => bail!(
                    "PLL {key} {}: CLKFB is not fed by a qualified output ({source:?}); HC08 raw-VCO feedback requires the same PLL's CLKOP; user feedback is not implemented", pll.feedback_description
                ),
            };
            let (feedback_divider, feedback_eighths) = if pll.feedback_kind == PllFeedback::RawVco {
                (1, 0)
            } else {
                let (divider, phase, _) = pll.outputs[feedback];
                (divider, phase)
            };
            let (a, b) = (pll.clki, pll.clkfb * feedback_divider);
            let common = gcd(a, b);
            let mut outputs = Vec::new();
            let mut lock = Vec::new();
            for (&port, &signal) in &pll.used {
                match port {
                    "LOCK" | "INTLOCK" => lock.push((port, signal)),
                    "CLKINTFB" => {}
                    _ => {
                        let (divider, eighths, _) = pll.outputs[port];
                        outputs.push(kernel::PllOutput {
                            port,
                            signal,
                            divider,
                            phase_eighths: eighths - feedback_eighths,
                        });
                    }
                }
            }
            self.builder.pll(kernel::PllConfig {
                resource: id,
                reference: pll.reference,
                vco_per_reference: (a / common, b / common),
                outputs,
                lock,
                controls: pll.controls,
                reference_divider: pll.clki,
                feedback_output: feedback,
            })?;
            self.record_resource(id, "EHXPLLL static settings: dividers minus one and CPHASE/FPHASE as nextpnr writes them; exact canonical INT_* selects CLKINTFB (HC07); HC08 INT_OP/INT_OS plus fabric JCLKFB3 returned from the same PLL's CLKOP selects raw-VCO feedback, Fvco = Fin x CLKFB_DIV / CLKI_DIV, independent of output dividers and zero versus 12/8/1/2 analog words; other external output feedback is traced through CLKFB and multiplies Fvco by Dfb; each Fout = Fvco / Dout; phase relative to feedback, or a synthetic zero VCO phase origin for HC08 (not measured), and assumed 50% duty; LOCK high from the start is the existing configured-startup approximation (HC05; HC08 synchronized high by 10 ms), static enables override ENCLK; reference period learnt from two rising edges; enabled RST/STDBY holds outputs and LOCK low, with HC07 restart/relock timing for output feedback only (raw-VCO recovery refused); dynamic phase and REFCLK bypass not executed");
        }
        Ok(())
    }
    fn execute_wire(&mut self, wire_id: WireId) -> Result<()> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/mod::execute_wire");
        let wire = self.index.wire(wire_id);
        let wire = &wire;
        if let Some(producers) = self.producers.get(&wire_id).cloned() {
            ensure!(
                producers.len() == 1,
                "multiple native producers for {wire:?}"
            );
            match &producers[0] {
                Producer::Node(index, port) => {
                    self.execute_node(&self.nodes[*index].clone(), port)?
                }
                Producer::Resource(id, port) => self.resource(*id, port)?,
            }
            return Ok(());
        }
        let mut incoming = if let Some(incoming) = self.routes.take(wire_id) {
            incoming?
        } else {
            self.index
                .incoming(self.device.model(), wire_id)?
                .into_iter()
                .map(route_decode::Incoming::lazy)
                .collect()
        };
        let mut constants = HashSet::new();
        for decoded in &mut incoming {
            let item = &decoded.item;
            if let Some(field) = &item.constant {
                let values = decoded
                    .constant
                    .take()
                    .unwrap_or_else(|| self.device.enum_candidates(item.tile, field));
                let mut values = self.decoded_field(item.tile, field, values)?;
                // Both constants written (nextpnr ties two cells' pins on one
                // CIB wire to different constants and writes the mux for
                // each): the chip takes 0 (HD08, the ALU's C pins on the
                // MULT18 C wires).
                let both = values.len() == 2 && values.iter().all(|v| v == "0" || v == "1");
                if both {
                    values = vec!["0".into()];
                }
                ensure!(
                    values.len() == 1,
                    "ambiguous CIB constant selection {field}"
                );
                if ["0", "1"].contains(&values[0].as_str()) {
                    let clock_control =
                        matches!(item.raw.as_str(), "JCLK0" | "JCLK1" | "JLSR0" | "JLSR1");
                    let (value, rule) = if clock_control {
                        let tile = self.device.model().tile(item.tile);
                        let def = &self.device.model().database().types[&tile.kind].enums[field];
                        ensure!(
                            values[0] == "0"
                                && def.patterns.len() == 2
                                && def.patterns.get(&item.raw).is_some_and(Vec::is_empty)
                                && def.patterns.get("0").is_some_and(|bits| !bits.is_empty()),
                            "unqualified CIB clock/LSR constant encoding {field}"
                        );
                        (Logic::One,"CIB CLK/LSR label 0 was fuzzed at the EBR pin after consumer inversion; nextpnr tie_cib_signal drives raw high and then applies the consumer mux once")
                    } else if both {
                        (
                            Logic::Zero,
                            "CIB data constant written as 0 and as 1: the chip takes 0 (HD08)",
                        )
                    } else {
                        (
                            Logic::from_bool(values[0] == "1"),
                            "direct CIB data/CE constant",
                        )
                    };
                    constants.insert(value == Logic::One);
                    self.report.constants.push(ConstantUse {
                        tile: self.device.model().tile(item.tile).name.clone(),
                        field: field.clone(),
                        wire: wire.clone(),
                        encoded_value: values[0].clone(),
                        value,
                        rule: rule.into(),
                    });
                }
            }
        }
        ensure!(
            constants.len() <= 1,
            "conflicting physical CIB constant fragments at {wire:?}"
        );
        if let Some(value) = constants.iter().next() {
            let output = self.signal_id(wire, wire_id)?;
            self.builder.static_driver(
                output,
                &format!(
                    "constant:R{}C{}:{}",
                    wire.location.row, wire.location.col, wire.name
                ),
                Strength::Strong,
                Logic::from_bool(*value),
            )?;
            self.report.counts.constants += 1;
            return Ok(());
        }
        let mut fixed = Vec::new();
        let mut muxes = Vec::new();
        for mut decoded in incoming.into_iter().filter(|i| i.item.constant.is_none()) {
            let item = &decoded.item;
            let override_carry = if let Some(field) =
                route_decode::carry_field(&self.device, item.tile, &item.raw)
            {
                let values = decoded
                    .carry
                    .take()
                    .unwrap_or_else(|| self.device.enum_candidates(item.tile, &field));
                let values = self.decoded_field(item.tile, &field, values)?;
                ensure!(
                    values.len() == 1,
                    "ambiguous/unmatched {}:{field}: {values:?}",
                    self.device.model().tile(item.tile).name
                );
                values[0] == "CCU2"
            } else {
                false
            };
            let connections = decoded.connections.take().unwrap_or_else(|| {
                route_decode::connections(&self.device, item.tile, &item.raw, override_carry)
            });
            for connection in connections {
                let Some(c) = connection? else { continue };
                ensure!(
                    !(wire.name.starts_with("JMULTA") && c.candidate.1.source.contains("JPO")),
                    "DSP preadder path is not implemented at {wire:?}"
                );
                let sink = &c.sink;
                let candidate = c.candidate;
                if c.kind == ArcKind::Fixed {
                    if let Some(rule) = self.excluded_dsp_source(&candidate.0, wire)? {
                        let (_, mut evidence, _) = candidate;
                        evidence.rule = rule.into();
                        self.report.excluded_routes.push(evidence);
                        continue;
                    }
                }
                if c.kind == ArcKind::Fixed && candidate.1.source.ends_with("_CIBTEST") {
                    let (_, mut evidence, _) = candidate;
                    evidence.rule =
                        "post-load profile: CIBTEST access inactive (not hardware-validated)"
                            .into();
                    self.report.excluded_routes.push(evidence);
                    continue;
                }
                if c.kind == ArcKind::Fixed {
                    fixed.push(candidate);
                } else {
                    muxes.push(candidate);
                }
                ensure!(
                    self.index.canonical(self.device.model(), sink)? == wire_id,
                    "incoming index mismatch"
                );
            }
        }
        let best = muxes
            .iter()
            .filter(|a| {
                !muxes
                    .iter()
                    .any(|b| subset(&a.2, &b.2) && !subset(&b.2, &a.2))
            })
            .collect::<Vec<_>>();
        ensure!(
            best.len() <= 1,
            "ambiguous physical mux candidates: {:?}",
            best.iter()
                .map(|c| (&c.1.tile, &c.1.source))
                .collect::<Vec<_>>()
        );
        // A DSP C line JDSPCn: the fixed JMUICn path from the CIB, and a mux
        // from JROC whose one bit (F35 of the tile, in Trellis) nextpnr sets
        // when it routes any C pin through JROC. On the board the MULT18 C
        // lines (C[17:0], C[44:27]) kept their JMUIC values with that bit set
        // (HD08c o5); an ALU's own C pins (C[26:18], C[53:45]) take the JROC
        // route nextpnr gave them (not measured: the record has R[15:0]).
        let dsp_c_line = wire
            .name
            .strip_prefix("JDSPC")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|_| {
                best.first().is_some_and(|b| b.1.source.contains("JROC"))
                    && !fixed.is_empty()
                    && fixed.iter().all(|f| f.1.source.contains("JMUIC"))
            });
        let selected = if let Some(n) = dsp_c_line {
            if (18..27).contains(&n) || (45..54).contains(&n) {
                let mut chosen = (*best[0]).clone();
                chosen.1.rule =
                    "DSP C pin of the ALU: the JROC route nextpnr gave it (not measured)".into();
                self.report.excluded_routes.extend(fixed.iter().map(|f| {
                    let mut evidence = f.1.clone();
                    evidence.rule = "DSP C pin of the ALU routed through JROC".into();
                    evidence
                }));
                vec![chosen]
            } else {
                let mut evidence = best[0].1.clone();
                evidence.rule =
                    "DSP C line of a MULT18: JMUIC, whatever the shared JROC bit (HD08c)".into();
                self.report.excluded_routes.push(evidence);
                fixed
            }
        } else if let Some(best) = best.first() {
            if !fixed.is_empty() {
                let same_source = fixed.iter().all(|f| f.0 == best.0);
                ensure!(
                    same_source
                        || (wire.name.starts_with("JDI")
                            && best.1.source.contains("INDD")
                            && fixed.iter().all(|f| f.1.source.contains("JPADDI")))
                        || (wire.name == "JINCK"
                            && best.1.source.contains("DLLDEL")
                            && fixed.iter().all(|f| f.1.source.contains("JPADDI"))),
                    "unqualified fixed/mux overlap at {wire:?}"
                );
            }
            vec![(*best).clone()]
        } else {
            if fixed.iter().map(|f| &f.0).collect::<HashSet<_>>().len() > 1 {
                if let Some(index) = self.shared_iologic_source(&fixed)? {
                    let mut chosen = fixed.swap_remove(index);
                    chosen.1.rule =
                        "fixed; shared IOLOGIC sink, the other source is inactive in its mode"
                            .into();
                    fixed = vec![chosen];
                }
            }
            fixed
        };
        let mut sources = HashSet::new();
        for (source, evidence, _) in selected {
            let source = source
                .as_ref()
                .context("selected mux source leaves the device")?;
            // The fixed internal arc is a block, not an unconditional route.
            // Only its qualified NONE profile may take this fallback path.
            let eclksync_arc = (evidence.source.contains("ECLKI_ECLKSYNC")
                && evidence.sink.contains("ECLKO_ECLKSYNC"))
                || (evidence.source.contains("ECLKI_BRGECLKSYNC")
                    && evidence.sink.contains("ECLKO_BRGECLKSYNC"))
                || (evidence.source.rsplit('_').next() == Some("ECLKI")
                    && evidence.sink.rsplit('_').next() == Some("ECLKO"));
            let source_key = self.index.canonical(self.device.model(), source)?;
            if !sources.insert(source_key) {
                continue;
            }
            ensure!(
                sources.len() == 1,
                "unqualified multiple fixed sources at {wire:?}"
            );
            if eclksync_arc {
                let mut owners = Vec::new();
                for (id, resource) in self
                    .device
                    .model()
                    .resources()
                    .filter(|(_, r)| r.family == "ECLKSYNCB" || r.family == "BRGECLKSYNC")
                {
                    if resource.location == wire.location
                        && self
                            .index
                            .canonical(self.device.model(), &self.resource_port(id, "ECLKI")?)?
                            == source_key
                        && self
                            .index
                            .canonical(self.device.model(), &self.resource_port(id, "ECLKO")?)?
                            == wire_id
                    {
                        owners.push(id);
                    }
                }
                ensure!(
                    owners.len() == 1,
                    "ECLKSYNC bypass has no unique resource owner ({} {} -> {})",
                    evidence.tile,
                    evidence.source,
                    evidence.sink
                );
                self.qualify_eclksync_none(owners[0])?;
                self.execute_eclksync(owners[0])?;
                self.report.routes.push(evidence);
                self.report.counts.routes += 1;
                continue;
            }
            let input = self.input_id(source, source_key)?;
            let output = self.signal_id(wire, wire_id)?;
            let label = format!(
                "route:{}:{}:{}",
                evidence.tile, evidence.source, evidence.sink
            );
            self.builder.buffer(&label, input, output)?;
            self.report.routes.push(evidence);
            self.report.counts.routes += 1;
        }
        if sources.is_empty() {
            if let Some(source) = self.bank_eclk_source(wire)? {
                // The bank's edge clock reaches each IO tile's BNK_ECLK<n>
                // through a bank-membership relation, not a route record
                // (IO_ROUTING.md); nextpnr's ECLK router stops there too.
                let input = self.input(&source)?;
                let output = self.signal_id(wire, wire_id)?;
                self.builder.buffer(
                    &format!(
                        "bank-eclk:{}:R{}C{}:{}",
                        source.name, wire.location.row, wire.location.col, wire.name
                    ),
                    input,
                    output,
                )?;
                self.report.routes.push(RouteUse {
                    tile: format!("R{}C{}", wire.location.row, wire.location.col),
                    source: source.name.clone(),
                    sink: wire.name.clone(),
                    source_line: 0,
                    rule: "bank edge clock: G_BANK<bank>ECLK<n> to the IO tile's BNK_ECLK<n>, bank of its PIOs (IO_ROUTING.md)".into(),
                });
                self.report.counts.routes += 1;
                return Ok(());
            }
        }
        if sources.is_empty() && !self.boundaries.contains(&wire_id) {
            self.report.undriven.push(wire.clone());
        }
        Ok(())
    }
}
impl Planner {
    /// For an IO tile's BNK_ECLK<n>, the bank edge clock G_BANK<bank>ECLK<n>
    /// that feeds it: the bank of the tile's PIOs, from the package data.
    fn bank_eclk_source(&self, wire: &Wire) -> Result<Option<Wire>> {
        let Some(n) = wire.name.strip_prefix("BNK_ECLK") else {
            return Ok(None);
        };
        let model = self.device.model();
        let banks: BTreeSet<&str> = model
            .package_pins()
            .filter(|p| {
                p.pio
                    .is_some_and(|id| model.resource(id).location == wire.location)
            })
            .map(|p| p.bank.as_str())
            .collect();
        ensure!(
            banks.len() <= 1,
            "IO tile R{}C{} has PIOs in banks {banks:?}",
            wire.location.row,
            wire.location.col
        );
        Ok(banks.into_iter().next().map(|bank| Wire {
            location: crate::model::topology::Location { row: 0, col: 0 },
            name: format!("G_BANK{bank}ECLK{n}"),
        }))
    }
}
fn subset(a: &[BitConstraint], b: &[BitConstraint]) -> bool {
    a.iter().all(|bit| b.contains(bit))
}
fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}
