//! Synchronous EBR ports on one canonical 2048x9 array. Unmeasured control
//! combinations and collision domains use an explicitly conservative policy.
use super::*;
use crate::state::EbrPipeline;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum EbrMode {
    Dp,
    Pdp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum WriteMode {
    Normal,
    WriteThrough,
    ReadBeforeWrite,
}
#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct EbrPort {
    pub width: usize,
    pub output_register: bool,
    pub write_mode: WriteMode,
    pub clock: Input,
    pub enable: Input,
    pub output_enable: Input,
    pub write_enable: Input,
    pub select: [Input; 3],
    /// Raw CS pin match, after reversing the stored CSDECODE word once.
    pub decode: u8,
    pub reset: Input,
    pub address: [Input; 14],
    /// DP data pins; in PDP, DIA and DIB form the lower/upper write halves.
    pub data: [Input; 18],
}
#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Ebr {
    pub resource: ResourceId,
    pub mode: EbrMode,
    pub ports: [EbrPort; 2],
    pub reset_kind: ResetKind,
    /// Synchronous release has no established internal stage count. Such a
    /// configuration is accepted only while local reset remains known inactive.
    pub asynchronous_release: bool,
    pub global_reset: Option<Input>,
    pub write_inhibit: Input,
}
/// The read path (0 = A, pins DOA; 1 = B, pins DOB) that carries bits 17:0 of
/// a 36-bit PDP read, the half written through DIA; bits 35:18 take the other.
/// Measured on the board (HB01, docs/inventory/LFE5U-85F/hb01_pdp36): A, the
/// order of the yosys PDP wrapper; nextpnr's public PDPW16KD packing assumes B.
pub const PDP36_LOW_PATH: usize = 0;
/// Whether the A path of a 36-bit PDP read takes its output enable and local
/// reset from port A (OCEA, RSTA) rather than from the read port B. Measured
/// (HB01): no. RSTB clears both halves, RSTA neither, OCEB moves both output
/// registers, OCEA neither, and a CLKA edge moves none.
pub const PDP36_A_PATH_OWN_CONTROLS: bool = false;

impl Ebr {
    /// Static part of HB09P's raw PDP36 collision profile. Dynamic controls,
    /// masks, definite shared edges and addresses are checked at capture.
    pub(super) fn hb09p_profile(&self) -> bool {
        self.mode == EbrMode::Pdp
            && self.reset_kind == ResetKind::Asynchronous
            && self.asynchronous_release
            && self.global_reset.is_none()
            && self
                .ports
                .iter()
                .all(|p| p.width == 36 && p.write_mode == WriteMode::Normal && !p.output_register)
    }
    /// A PDP block whose read port B is 36 bits wide: one read drives both
    /// 18-bit read paths, DOA and DOB.
    pub fn wide_read(&self) -> bool {
        self.mode == EbrMode::Pdp && self.ports[1].width == 36
    }
    /// The number of output pins of read path `side` (DOA or DOB).
    pub fn read_pins(&self, side: usize) -> usize {
        match (self.mode, side) {
            (EbrMode::Dp, _) => self.ports[side].width,
            (EbrMode::Pdp, 0) => 18 * usize::from(self.wide_read()),
            (EbrMode::Pdp, _) => self.ports[1].width.min(18),
        }
    }
    /// The read path and pin of bit `bit` of the word read by port `side`.
    pub fn read_pin(&self, side: usize, bit: usize) -> (usize, usize) {
        if self.wide_read() {
            if bit < 18 {
                (PDP36_LOW_PATH, bit)
            } else {
                (1 - PDP36_LOW_PATH, bit - 18)
            }
        } else {
            (side, bit)
        }
    }
    /// The port whose output enable, local reset and register mode act on
    /// read path `path`. In a 36-bit PDP read the read port is B; HB01
    /// decides whether path A follows it or port A.
    pub fn path_controls(&self, path: usize) -> &EbrPort {
        if self.wide_read() && !(path == 0 && PDP36_A_PATH_OWN_CONTROLS) {
            &self.ports[1]
        } else {
            &self.ports[path]
        }
    }
}
#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Block {
    pub config: Ebr,
    pub previous_clock: [Logic; 2],
    /// Filled on the first capture, after all routes have been built. Port
    /// clock pins can be different endpoints of the same buffer chain.
    pub(super) same_clock: Option<bool>,
    pub(super) outputs: [[Option<usize>; 18]; 2],
}
pub(super) struct Capture {
    resource: ResourceId,
    pipeline: EbrPipeline,
    writes: BTreeMap<usize, Logic>,
    updates: Vec<Update>,
}
#[derive(Clone, Serialize)]
pub struct EbrNotice {
    pub time: Time,
    pub delta: u32,
    pub resource: String,
    pub kind: &'static str,
    /// Bounded preview of conservative canonical nine-bit collision domains.
    pub rows9: Vec<usize>,
    pub row_count: usize,
}
fn shift(width: usize) -> usize {
    match width {
        1 => 0,
        2 => 1,
        4 => 2,
        9 => 3,
        18 => 4,
        36 => 5,
        _ => unreachable!(),
    }
}
/// Address/data views from the collected INIT layout, including preserved
/// ninth bits invisible to widths 1/2/4.
pub fn ebr_cell(width: usize, address: usize, bit: usize) -> Result<(usize, usize)> {
    ensure!(
        [1, 2, 4, 9, 18, 36].contains(&width)
            && address < (1 << (14 - shift(width)))
            && bit < width,
        "invalid EBR data coordinate"
    );
    Ok(if width < 9 {
        (address / (8 / width), (address % (8 / width)) * width + bit)
    } else {
        (address * (width / 9) + bit / 9, bit % 9)
    })
}
fn cell(width: usize, address: usize, bit: usize) -> usize {
    let (row, bit) = ebr_cell(width, address, bit).expect("validated EBR coordinate");
    row * 9 + bit
}
fn binary(value: Logic) -> Logic {
    if value == Logic::Z {
        Logic::X
    } else {
        value
    }
}
fn merge(a: Logic, b: Logic) -> Logic {
    Logic::mux(Logic::X, a, b)
}
fn addresses(port: &EbrPort, signals: &[Signal]) -> Vec<usize> {
    let bits = port.address[shift(port.width)..]
        .iter()
        .map(|i| i.read(signals))
        .collect::<Vec<_>>();
    if let Some(a) = bits
        .iter()
        .enumerate()
        .try_fold(0, |a, (i, v)| Some(a | ((v.bit()? as usize) << i)))
    {
        return vec![a];
    }
    (0..1 << bits.len())
        .filter(|a| {
            bits.iter()
                .enumerate()
                .all(|(i, v)| v.bit().is_none_or(|v| v == (a & (1 << i) != 0)))
        })
        .collect()
}
fn access(port: &EbrPort, signals: &[Signal]) -> Logic {
    port.select
        .iter()
        .enumerate()
        .fold(port.enable.read(signals), |result, (bit, input)| {
            let value = input.read(signals);
            result.and(if port.decode & (1 << bit) == 0 {
                value.invert()
            } else {
                binary(value)
            })
        })
}
fn group_mask(port: &EbrPort, signals: &[Signal], bit: usize) -> Logic {
    if port.width >= 18 {
        binary(port.address[bit / 9].read(signals))
    } else {
        Logic::One
    }
}
fn apply_edge(edge: Activity, old: Logic, new: Logic) -> Logic {
    match edge {
        Activity::None => old,
        Activity::Definite => new,
        Activity::Possible => merge(old, new),
    }
}

/// A clock's source and effective polarity through plain routing buffers.
/// Do not equate unrelated clocks merely because their edges coincide.
fn clock_source(
    mut input: Input,
    signals: &[Signal],
    combinational: &[Combinational],
) -> Option<(SignalId, bool)> {
    let mut inverted = false;
    for _ in 0..signals.len() {
        let source = match input {
            Input::Signal(s) => s,
            Input::Inverted(s) => {
                inverted = !inverted;
                s
            }
            Input::Constant(_) => return None,
        };
        let [driver] = signals[source.index].drivers[..] else {
            return Some((source, inverted));
        };
        match combinational.iter().find(|g| g.driver == driver) {
            Some(Combinational {
                function: Function::Buffer(next),
                ..
            }) => input = *next,
            _ => return Some((source, inverted)),
        }
    }
    None // A buffer loop is not an established clock source.
}

impl Builder {
    /// Sets the read latches and output registers of an EBR to 0, as the
    /// planner does under the configured startup (HB02). Call it before
    /// [`Builder::ebr`] binds the block.
    pub fn clear_ebr_pipeline(&mut self, id: ResourceId) -> Result<()> {
        self.state.set_ebr_pipeline(
            id,
            EbrPipeline {
                read_latches: [[Logic::Zero; 18]; 2],
                output_registers: [[Logic::Zero; 18]; 2],
            },
        )
    }
    pub fn ebr(&mut self, config: Ebr) -> Result<[Vec<SignalId>; 2]> {
        ensure!(
            self.device.model().resource(config.resource).family == "EBR",
            "resource is not an EBR"
        );
        ensure!(
            !self.bound_ebr.contains(&config.resource),
            "EBR already has an execution owner"
        );
        let pipeline = self.state.ebr_pipeline(config.resource)?.clone();
        for (index, port) in config.ports.iter().enumerate() {
            let widths = match (config.mode, index) {
                (EbrMode::Pdp, 0) => &[36][..],
                (EbrMode::Pdp, _) => &[1, 2, 4, 9, 18, 36][..],
                _ => &[1, 2, 4, 9, 18][..],
            };
            ensure!(
                widths.contains(&port.width),
                "unsupported EBR port width {}",
                port.width
            );
            ensure!(port.decode < 8, "CSDECODE exceeds three bits");
            for input in port
                .address
                .into_iter()
                .chain(port.data)
                .chain(port.select)
                .chain([
                    port.clock,
                    port.enable,
                    port.output_enable,
                    port.write_enable,
                    port.reset,
                ])
            {
                self.validate_input(input)?;
            }
        }
        self.validate_input(config.write_inhibit)?;
        if let Some(reset) = config.global_reset {
            self.validate_input(reset)?;
        }
        let index = self.ebr.len();
        let ports = self.device.model().resource_ports(config.resource);
        let key = self.device.model().resource(config.resource).key.clone();
        let mut outputs = [[None; 18]; 2];
        let mut signals: [Vec<SignalId>; 2] = [Vec::new(), Vec::new()];
        for side in 0..2 {
            for (bit, output) in outputs[side]
                .iter_mut()
                .enumerate()
                .take(config.read_pins(side))
            {
                let name = format!("DO{}{bit}", if side == 0 { 'A' } else { 'B' });
                let native = ports
                    .iter()
                    .find(|p| p.name == name && p.direction == Direction::Output)
                    .with_context(|| format!("missing EBR output {name}"))?;
                let signal = self.signal(native.wire.clone())?;
                let label = format!("ebr:{key}:{name}");
                self.label(&label)?;
                let initial = if config.path_controls(side).output_register {
                    pipeline.output_registers[side][bit]
                } else {
                    pipeline.read_latches[side][bit]
                };
                *output = Some(self.driver(signal, label, Strength::Strong, initial, false));
                signals[side].push(signal);
            }
        }
        for port in &config.ports {
            for input in port
                .address
                .into_iter()
                .chain(port.data)
                .chain(port.select)
                .chain([
                    port.clock,
                    port.enable,
                    port.output_enable,
                    port.write_enable,
                    port.reset,
                ])
            {
                if let Some(signal) = input.signal() {
                    self.signals[signal.index].ebr.push(index);
                }
            }
        }
        for input in [Some(config.write_inhibit), config.global_reset]
            .into_iter()
            .flatten()
        {
            if let Some(signal) = input.signal() {
                self.signals[signal.index].ebr.push(index);
            }
        }
        self.bound_ebr.insert(config.resource);
        self.ebr.push(Block {
            config,
            previous_clock: [Logic::X; 2],
            same_clock: None,
            outputs,
        });
        Ok(signals)
    }
}

impl Kernel {
    fn ebr_notice(&mut self, resource: ResourceId, kind: &'static str, rows: &BTreeSet<usize>) {
        let capacity = self.options.trace_capacity.min(1024);
        if capacity == 0 {
            return;
        }
        if self.ebr_notices.len() == capacity {
            self.ebr_notices.pop_front();
            self.stats.ebr_notice_overwrites += 1;
        }
        self.ebr_notices.push_back(EbrNotice {
            time: self.now,
            delta: self.delta,
            resource: self.device.model().resource(resource).key.clone(),
            kind,
            rows9: rows.iter().copied().take(16).collect(),
            row_count: rows.len(),
        });
    }
    pub fn ebr_notices(&self) -> impl Iterator<Item = &EbrNotice> {
        self.ebr_notices.iter()
    }
    pub(super) fn capture_ebr(&mut self) -> Result<Vec<Capture>> {
        let mut captures = Vec::new();
        for index in self.dirty_ebr.take() {
            self.stats.ebr_evaluations += 1;
            let block = &mut self.ebr[index];
            let config = block.config.clone();
            let outputs = block.outputs;
            let same_clock = *block.same_clock.get_or_insert_with(|| {
                let sources = config
                    .ports
                    .each_ref()
                    .map(|p| clock_source(p.clock, &self.signals, &self.combinational));
                sources[0].is_some() && sources[0] == sources[1]
            });
            let clocks = config.ports.each_ref().map(|p| p.clock.read(&self.signals));
            let edges = std::array::from_fn::<_, 2, _>(|p| {
                clock_edge(Edge::Rising, block.previous_clock[p], clocks[p])
            });
            block.previous_clock = clocks;
            let global = config
                .global_reset
                .map(|r| binary(r.read(&self.signals)))
                .unwrap_or(Logic::Zero);
            let inhibit = binary(config.write_inhibit.read(&self.signals));
            let old = self.state.ebr_pipeline(config.resource)?.clone();
            let mut pipeline = old.clone();
            let addresses = config.ports.each_ref().map(|p| addresses(p, &self.signals));
            let access = config.ports.each_ref().map(|p| access(p, &self.signals));
            let resets = config
                .ports
                .each_ref()
                .map(|p| binary(p.reset.read(&self.signals)));
            let mut writes: [BTreeMap<usize, Logic>; 2] = [BTreeMap::new(), BTreeMap::new()];
            let mut control_ambiguity = false;
            for side in 0..2 {
                let port = &config.ports[side];
                ensure!(
                    config.reset_kind != ResetKind::Asynchronous
                        || config.asynchronous_release
                        || resets[side] == Logic::Zero,
                    "EBR {} has active/unknown local reset with unsupported synchronous release",
                    self.device.model().resource(config.resource).key
                );
                if edges[side] == Activity::Possible {
                    self.stats.possible_clock_edges += 1;
                }
                if edges[side] != Activity::None && access[side] != Logic::Zero {
                    self.stats.ebr_access_edges += 1;
                }
                if edges[side] == Activity::None || (config.mode == EbrMode::Pdp && side == 1) {
                    continue;
                }
                let we = if config.mode == EbrMode::Pdp {
                    Logic::One
                } else {
                    port.write_enable.read(&self.signals)
                };
                let enabled = access[side].and(we).and(inhibit.invert());
                if enabled == Logic::Zero {
                    continue;
                }
                let reset_ambiguous = resets[side] != Logic::Zero || global != Logic::Zero;
                control_ambiguity |= reset_ambiguous;
                let certain = edges[side] == Activity::Definite
                    && enabled == Logic::One
                    && addresses[side].len() == 1
                    && !reset_ambiguous;
                for &address in &addresses[side] {
                    for bit in 0..port.width {
                        let mask = group_mask(port, &self.signals, bit);
                        if mask == Logic::Zero {
                            continue;
                        }
                        let data = if config.mode == EbrMode::Pdp {
                            config.ports[bit / 18].data[bit % 18]
                        } else {
                            port.data[bit]
                        };
                        let value = binary(data.read(&self.signals));
                        let cell = cell(port.width, address, bit);
                        let stored = self.state.ebr_word(config.resource, cell / 9)?[cell % 9];
                        writes[side].insert(
                            cell,
                            if certain && mask == Logic::One {
                                value
                            } else {
                                merge(stored, value)
                            },
                        );
                    }
                }
            }
            let rows = writes
                .each_ref()
                .map(|w| w.keys().map(|c| c / 9).collect::<BTreeSet<_>>());
            // HB09 measured equal-width/mode full-word accesses on one clock,
            // with CE/OCE active and no reset or write inhibit. In this scope
            // different narrow words in one INIT9 row are independent too.
            let hb09 = config.mode == EbrMode::Dp
                && same_clock
                && config.ports[0].width == config.ports[1].width
                && config.ports[0].write_mode == config.ports[1].write_mode
                && global == Logic::Zero
                && inhibit == Logic::Zero
                && (0..2).all(|side| {
                    let port = &config.ports[side];
                    edges[side] == Activity::Definite
                        && access[side] == Logic::One
                        && resets[side] == Logic::Zero
                        && port.output_enable.read(&self.signals) == Logic::One
                        && port.write_enable.read(&self.signals).bit().is_some()
                        && addresses[side].len() == 1
                        && (0..port.width)
                            .all(|bit| group_mask(port, &self.signals, bit) == Logic::One)
                });
            // HB09P measured the raw Yosys PDP36/NOREG/NORMAL profile:
            // one shared clock/edge, active CE/CS, OCE=0/1,
            // ASYNC reset/release with resets/GSR/GWDIS inactive and all
            // four write lanes enabled. No narrow/registered PDP inference.
            let hb09p = config.hb09p_profile()
                && same_clock
                && global == Logic::Zero
                && inhibit == Logic::Zero
                && (0..2).all(|side| {
                    let port = &config.ports[side];
                    edges[side] == Activity::Definite
                        && access[side] == Logic::One
                        && resets[side] == Logic::Zero
                        && port.output_enable.read(&self.signals)
                            == if side == 0 { Logic::Zero } else { Logic::One }
                        && addresses[side].len() == 1
                })
                && (0..36)
                    .all(|bit| group_mask(&config.ports[0], &self.signals, bit) == Logic::One);
            let same_word = addresses[0] == addresses[1];
            let double_write = if hb09 && !same_word {
                BTreeSet::new()
            } else {
                rows[0].intersection(&rows[1]).copied().collect()
            };
            let mut collision_rows = double_write.clone();
            if hb09 && config.ports[0].width < 18 {
                collision_rows.clear();
            }
            for side in 0..2 {
                if config.mode == EbrMode::Pdp && side == 0 {
                    continue;
                }
                let port = &config.ports[side];
                let edge = &edges[side];
                if *edge != Activity::None && access[side] != Logic::Zero {
                    for &a in &addresses[side] {
                        for b in 0..port.width {
                            let row = cell(port.width, a, b) / 9;
                            if !(hb09 || hb09p && same_word) && rows[1 - side].contains(&row) {
                                collision_rows.insert(row);
                            }
                        }
                    }
                }
                let we = if config.mode == EbrMode::Pdp {
                    Logic::Zero
                } else {
                    binary(port.write_enable.read(&self.signals))
                };
                if *edge != Activity::None
                    && access[side] != Logic::Zero
                    && we != Logic::Zero
                    && port.write_mode != WriteMode::Normal
                    && port.width >= 18
                    && (0..port.width / 9)
                        .any(|group| group_mask(port, &self.signals, group * 9) != Logic::One)
                {
                    control_ambiguity = true;
                }
                // The read paths this port drives (both in a 36-bit PDP
                // read), with the output enable and reset of each.
                let mut oce = [Logic::X; 2];
                let mut reset = [Logic::X; 2];
                for path in 0..2 {
                    if !(path == side || config.wide_read()) {
                        continue;
                    }
                    let controls = config.path_controls(path);
                    oce[path] = binary(controls.output_enable.read(&self.signals));
                    reset[path] = binary(controls.reset.read(&self.signals));
                    if *edge != Activity::None
                        && ((controls.output_register
                            && oce[path] != Logic::Zero
                            && access[side] != Logic::One)
                            || (!controls.output_register
                                && oce[path] != Logic::One
                                && access[side] != Logic::Zero))
                    {
                        control_ambiguity = true;
                    }
                }
                for bit in 0..port.width {
                    let (path, pin) = config.read_pin(side, bit);
                    let registered = config.path_controls(path).output_register;
                    let mut memory = None;
                    for &a in &addresses[side] {
                        let pos = cell(port.width, a, bit);
                        let value = if hb09 {
                            if we == Logic::Zero
                                && same_word
                                && port.write_mode != WriteMode::ReadBeforeWrite
                            {
                                writes[1 - side].get(&pos).copied().unwrap_or(
                                    self.state.ebr_word(config.resource, pos / 9)?[pos % 9],
                                )
                            } else {
                                self.state.ebr_word(config.resource, pos / 9)?[pos % 9]
                            }
                        } else if hb09p && same_word {
                            writes[0]
                                .get(&pos)
                                .copied()
                                .unwrap_or(self.state.ebr_word(config.resource, pos / 9)?[pos % 9])
                        } else if rows[1 - side].contains(&(pos / 9)) {
                            Logic::X
                        } else {
                            self.state.ebr_word(config.resource, pos / 9)?[pos % 9]
                        };
                        memory = Some(memory.map_or(value, |old| merge(old, value)));
                    }
                    let memory = memory.unwrap_or(Logic::X);
                    let held = old.read_latches[path][pin];
                    let mut next = memory;
                    if we != Logic::Zero {
                        let mask = group_mask(port, &self.signals, bit);
                        let data = binary(port.data[bit].read(&self.signals));
                        let mut write_result = match port.write_mode {
                            WriteMode::Normal => held,
                            WriteMode::WriteThrough => {
                                Logic::mux(mask, merge(merge(held, memory), data), data)
                            }
                            WriteMode::ReadBeforeWrite => {
                                Logic::mux(mask, merge(held, memory), memory)
                            }
                        };
                        if inhibit != Logic::Zero {
                            write_result = merge(write_result, memory);
                            control_ambiguity = true;
                        }
                        next = Logic::mux(we, memory, write_result);
                    }
                    if !registered && oce[path] != Logic::One {
                        next = merge(held, next);
                    }
                    next = Logic::mux(access[side], held, next);
                    if config.reset_kind == ResetKind::Synchronous {
                        next = Logic::mux(reset[path], next, Logic::Zero);
                    }
                    let mut latch = apply_edge(*edge, held, next);
                    let out_old = old.output_registers[path][pin];
                    let out_candidate = if access[side] == Logic::One {
                        held
                    } else {
                        merge(out_old, held)
                    };
                    let mut out_next = Logic::mux(oce[path], out_old, out_candidate);
                    if config.reset_kind == ResetKind::Synchronous {
                        out_next = Logic::mux(reset[path], out_next, Logic::Zero);
                    }
                    let mut output = if registered {
                        apply_edge(*edge, out_old, out_next)
                    } else {
                        out_old
                    };
                    if config.reset_kind == ResetKind::Asynchronous {
                        latch = Logic::mux(reset[path], latch, Logic::Zero);
                        if registered {
                            output = Logic::mux(reset[path], output, Logic::Zero);
                        }
                    }
                    latch = Logic::mux(global, latch, Logic::Zero);
                    if registered {
                        output = Logic::mux(global, output, Logic::Zero);
                    }
                    pipeline.read_latches[path][pin] = latch;
                    pipeline.output_registers[path][pin] = output;
                }
            }
            let mut combined = BTreeMap::new();
            for w in writes {
                combined.extend(w);
            }
            // Extending A then B above implements HB09's B winner at widths
            // 1/2/4/9, without disturbing other narrow words or the ninth bit.
            // At width 18 the board sometimes mixed bits: keep the array X,
            // while each writing port still outputs its own write-mode result.
            if !hb09 || config.ports[0].width == 18 {
                for &row in &double_write {
                    for bit in 0..9 {
                        combined.insert(row * 9 + bit, Logic::X);
                    }
                }
            }
            if !collision_rows.is_empty() {
                self.stats.ebr_collision_rows += collision_rows.len() as u64;
                self.ebr_notice(
                    config.resource,
                    "possible cross-port collision; conservative INIT9 row domain",
                    &collision_rows,
                );
            }
            if control_ambiguity {
                self.stats.ebr_control_ambiguities += 1;
                self.ebr_notice(
                    config.resource,
                    "unmeasured reset/write, mask, inhibit or OCE gating policy",
                    &BTreeSet::new(),
                );
            }
            let mut updates = Vec::new();
            for (side, outputs) in outputs.iter().enumerate() {
                for (bit, &driver) in outputs.iter().enumerate() {
                    if let Some(driver) = driver {
                        updates.push(Update {
                            driver,
                            value: if config.path_controls(side).output_register {
                                pipeline.output_registers[side][bit]
                            } else {
                                pipeline.read_latches[side][bit]
                            },
                            reason: Reason::Static("EBR capture; shared pre-write state and conservative collision policy"),
                        });
                    }
                }
            }
            captures.push(Capture {
                resource: config.resource,
                pipeline,
                writes: combined,
                updates,
            });
        }
        Ok(captures)
    }
    pub(super) fn commit_ebr(
        &mut self,
        captures: Vec<Capture>,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        for capture in captures {
            self.state
                .set_ebr_pipeline(capture.resource, capture.pipeline)?;
            let mut words: BTreeMap<usize, [Logic; 9]> = BTreeMap::new();
            for (cell, value) in capture.writes {
                let row = cell / 9;
                if let std::collections::btree_map::Entry::Vacant(entry) = words.entry(row) {
                    entry.insert(*self.state.ebr_word(capture.resource, row)?);
                }
                let word = words.get_mut(&row).unwrap();
                if word[cell % 9] != value {
                    word[cell % 9] = value;
                    self.stats.ebr_bit_changes += 1;
                }
            }
            for (row, word) in words {
                self.state.write_ebr_word(capture.resource, row, word)?;
            }
            updates.extend(capture.updates);
        }
        Ok(())
    }
}
