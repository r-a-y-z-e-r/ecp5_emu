//! Parallel MULT18X18D with operand forwarding and optional register stages.
use super::*;
use crate::state::MultiplierState;

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct MultiplierRegister {
    pub clock: Input,
    pub enable: Input,
    pub reset: Input,
    pub reset_kind: ResetKind,
}
/// The output stage (HD01 on the board): it captures at a rising edge of its
/// clock (both edges with CLK_DIV disabled, HD09) while CE0 is high,
/// whatever REG_OUTPUT_CE says; the reset is the one
/// REG_OUTPUT_RST selects on MULT18 0/4 and RST0 on 1/5, synchronous or
/// asynchronous as RESETMODE says. Global reset must be disabled (not
/// measured). SIGNEDP after this stage is unmodelled X.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct MultiplierOutputRegister {
    pub clock: Input,
    pub enable: Input,
    pub reset: Input,
    pub reset_kind: ResetKind,
}
#[derive(Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct MultiplierPipeline {
    /// HD09: disabled clock dividers capture non-C stages on both edges.
    pub both_edges: bool,
    pub input_a: Option<MultiplierRegister>,
    pub input_b: Option<MultiplierRegister>,
    pub product: Option<MultiplierRegister>,
    pub output: Option<MultiplierOutputRegister>,
    /// Effective active-high global reset, independent of local RESETMODE.
    pub global_reset: Option<Input>,
}
impl MultiplierPipeline {
    fn stages(&self) -> [Option<MultiplierRegister>; 4] {
        [
            self.input_a,
            self.input_b,
            self.product,
            self.output.map(|s| MultiplierRegister {
                clock: s.clock,
                enable: s.enable,
                reset: s.reset,
                reset_kind: s.reset_kind,
            }),
        ]
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Multiplier18 {
    pub resource: ResourceId,
    pub a: [Input; 18],
    pub b: [Input; 18],
    pub signed_a: Input,
    pub signed_b: Input,
    /// This profile requires parallel inputs (both known zero). Shift/highspeed
    /// paths and the unlocated matching registers have no invented behavior.
    pub source_a: Input,
    pub source_b: Input,
}
pub struct MultiplierOutputs {
    pub product: [SignalId; 36],
    pub signed: SignalId,
    /// Parallel operands after the optional INPUTA/B stages, before the
    /// multiplier's PIPELINE/OUTPUT stages. No separate forwarding storage.
    pub roa: [SignalId; 18],
    pub rob: [SignalId; 18],
}
/// Only operand wires are driven; no P/SIGNEDP or shared output bank ownership.
pub struct MultiplierOperandOutputs {
    pub roa: [SignalId; 18],
    pub rob: [SignalId; 18],
}
type ProductSignals = ([SignalId; 36], SignalId);
#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Multiplier {
    config: Multiplier18,
    pipeline: MultiplierPipeline,
    previous_clock: [Logic; 4],
    pub(super) global_at_rise: [Logic; 4],
    #[cfg_attr(
        test,
        serde(serialize_with = "crate::kernel::cache::snapshot::optional_array")
    )]
    product_drivers: Option<[usize; 37]>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    operand_drivers: [usize; 36],
}
impl Multiplier {
    pub(super) fn config(&self) -> &Multiplier18 {
        &self.config
    }
    pub(super) fn pipeline(&self) -> &MultiplierPipeline {
        &self.pipeline
    }
    /// The drivers of P0..P35 and SIGNEDP, when the product is bound.
    pub(super) fn product_drivers(&self) -> Option<&[usize; 37]> {
        self.product_drivers.as_ref()
    }
    /// The drivers of ROA0..ROA17 and ROB0..ROB17.
    pub(super) fn operand_drivers(&self) -> &[usize; 36] {
        &self.operand_drivers
    }
}
pub(super) struct MultiplierCapture {
    index: usize,
    state: MultiplierState,
    output: Option<[Logic; 36]>,
}
/// The captures of one wave: the multipliers' stages and the ALUs' registers.
pub(super) struct DspCaptures {
    multipliers: Vec<MultiplierCapture>,
    alus: Vec<super::alu::AluCapture>,
}
impl Multiplier {
    fn operands(
        &self,
        state: &MultiplierState,
        signals: &[Signal],
    ) -> ([Logic; 18], [Logic; 18], Logic, Logic) {
        let (a, sa) = if self.pipeline.input_a.is_some() {
            (state.input_a, state.signed_a)
        } else {
            (
                self.config.a.map(|v| binary(v.read(signals))),
                binary(self.config.signed_a.read(signals)),
            )
        };
        let (b, sb) = if self.pipeline.input_b.is_some() {
            (state.input_b, state.signed_b)
        } else {
            (
                self.config.b.map(|v| binary(v.read(signals))),
                binary(self.config.signed_b.read(signals)),
            )
        };
        (a, b, sa, sb)
    }
}
fn binary(value: Logic) -> Logic {
    if value == Logic::Z {
        Logic::X
    } else {
        value
    }
}

fn capture<const N: usize>(
    old: [Logic; N],
    data: [Logic; N],
    control: MultiplierRegister,
    edge: Activity,
    signals: &[Signal],
    global_reset: Logic,
) -> [Logic; N] {
    std::array::from_fn(|bit| {
        let mut next = Logic::mux(control.enable.read(signals), old[bit], binary(data[bit]));
        if control.reset_kind == ResetKind::Synchronous {
            next = Logic::mux(control.reset.read(signals), next, Logic::Zero);
        }
        let mut next = match edge {
            Activity::None => old[bit],
            Activity::Definite => next,
            Activity::Possible => Logic::mux(Logic::X, old[bit], next),
        };
        if control.reset_kind == ResetKind::Asynchronous {
            next = Logic::mux(control.reset.read(signals), next, Logic::Zero);
        }
        Logic::mux(global_reset, next, Logic::Zero)
    })
}
fn unsigned(bits: &[Logic; 18]) -> Option<u64> {
    bits.iter().enumerate().try_fold(0, |word, (bit, value)| {
        Some(word | ((value.bit()? as u64) << bit))
    })
}
fn operand(word: u64, signed: bool) -> i64 {
    word as i64
        - if signed && word & (1 << 17) != 0 {
            1 << 18
        } else {
            0
        }
}
/// Exact known arithmetic. Unknown data uses a conservative four-state
/// shift/add network; unknown signed controls on known operands merge all
/// possible interpretations. Z is unknown data, never a numeric level.
pub fn multiply18(
    a: &[Logic; 18],
    b: &[Logic; 18],
    signed_a: Logic,
    signed_b: Logic,
) -> [Logic; 36] {
    if let (Some(a), Some(b)) = (unsigned(a), unsigned(b)) {
        let mut result = None;
        for sa in [false, true]
            .into_iter()
            .filter(|s| signed_a.bit().is_none_or(|v| v == *s))
        {
            for sb in [false, true]
                .into_iter()
                .filter(|s| signed_b.bit().is_none_or(|v| v == *s))
            {
                let product = (operand(a, sa) * operand(b, sb)) as u64;
                let bits = std::array::from_fn(|bit| Logic::from_bool(product & (1 << bit) != 0));
                result = Some(result.map_or(bits, |old: [Logic; 36]| {
                    std::array::from_fn(|bit| Logic::mux(Logic::X, old[bit], bits[bit]))
                }));
            }
        }
        return result.unwrap();
    }
    let binary = |v: Logic| if v == Logic::Z { Logic::X } else { v };
    let a: [Logic; 36] = std::array::from_fn(|bit| {
        if bit < 18 {
            binary(a[bit])
        } else {
            a[17].and(signed_a)
        }
    });
    let b: [Logic; 36] = std::array::from_fn(|bit| {
        if bit < 18 {
            binary(b[bit])
        } else {
            b[17].and(signed_b)
        }
    });
    let mut result = [Logic::Zero; 36];
    for (offset, &multiplier) in b.iter().enumerate() {
        if multiplier == Logic::Zero {
            continue;
        }
        let mut carry = Logic::Zero;
        for bit in offset..36 {
            let x = result[bit];
            let y = a[bit - offset].and(multiplier);
            result[bit] = x.xor(y).xor(carry);
            carry = x.and(y).or(x.and(carry)).or(y.and(carry));
        }
    }
    result
}
impl Builder {
    pub fn multiplier18(&mut self, config: Multiplier18) -> Result<MultiplierOutputs> {
        self.multiplier18_with_pipeline(config, MultiplierPipeline::default())
    }
    /// D05 execution contract: data/sign share input controls; product/SIGNEDP
    /// share pipeline controls; reset precedes CE. Physical/model validation is
    /// pending and must remain explicit in plans accepting registered profiles.
    pub fn multiplier18_with_pipeline(
        &mut self,
        config: Multiplier18,
        pipeline: MultiplierPipeline,
    ) -> Result<MultiplierOutputs> {
        let (product, operands) = self.bind_multiplier(config, pipeline, true)?;
        let (product, signed) = product.unwrap();
        Ok(MultiplierOutputs {
            product,
            signed,
            roa: operands.roa,
            rob: operands.rob,
        })
    }
    /// Observe parallel INPUTA/B independently of an unmodelled ALU. The caller
    /// must establish the routing mode; D05 register assumptions still apply.
    /// Sets the output register of a bound MULT18 to 0, as the planner does
    /// where the board showed it 0 after configuration (HD07s).
    pub fn clear_multiplier_output(&mut self, id: ResourceId) -> Result<()> {
        ensure!(
            self.bound_multipliers.contains(&id),
            "MULT18 has no execution owner"
        );
        self.state.set_multiplier_output(id, [Logic::Zero; 36])
    }
    /// HD09ps: the enabled INPUTA/B and PIPELINE stages start at zero with
    /// GSR enabled. The planner calls this only for configured startup.
    pub fn clear_multiplier_stages(&mut self, id: ResourceId) -> Result<()> {
        let pipeline = &self
            .multipliers
            .iter()
            .find(|m| m.config.resource == id)
            .context("MULT18 has no execution owner")?
            .pipeline;
        let mut state = self.state.multiplier(id)?.clone();
        if pipeline.input_a.is_some() {
            state.input_a = [Logic::Zero; 18];
        }
        if pipeline.input_b.is_some() {
            state.input_b = [Logic::Zero; 18];
        }
        if pipeline.product.is_some() {
            state.product = [Logic::Zero; 36];
        }
        // HD09 did not record startup sign storage; leave it unknown.
        self.state.set_multiplier(id, state)
    }
    pub fn multiplier18_operands(
        &mut self,
        config: Multiplier18,
        pipeline: MultiplierPipeline,
    ) -> Result<MultiplierOperandOutputs> {
        ensure!(
            pipeline.product.is_none() && pipeline.output.is_none(),
            "MULT18 operand-only view cannot bind product/output register stages"
        );
        Ok(self.bind_multiplier(config, pipeline, false)?.1)
    }
    fn bind_multiplier(
        &mut self,
        config: Multiplier18,
        pipeline: MultiplierPipeline,
        product: bool,
    ) -> Result<(Option<ProductSignals>, MultiplierOperandOutputs)> {
        let resource = self.device.model().resource(config.resource);
        ensure!(resource.family == "MULT18", "resource is not a MULT18");
        ensure!(
            !self.bound_multipliers.contains(&config.resource),
            "MULT18 already has an execution owner"
        );
        self.state.dsp_slice(config.resource)?;
        if pipeline.output.is_some() {
            self.state.multiplier_output_view(config.resource)?;
        }
        let inputs = config
            .a
            .into_iter()
            .chain(config.b)
            .chain([
                config.signed_a,
                config.signed_b,
                config.source_a,
                config.source_b,
            ])
            .chain(
                pipeline
                    .stages()
                    .into_iter()
                    .flatten()
                    .flat_map(|s| [s.clock, s.enable, s.reset]),
            )
            .chain(pipeline.global_reset)
            .chain(
                pipeline
                    .output
                    .into_iter()
                    .flat_map(|s| [s.enable, s.reset]),
            )
            .collect::<Vec<_>>();
        for &input in &inputs {
            self.validate_input(input)?;
        }
        let key = resource.key.clone();
        let ports = self.device.model().resource_ports(config.resource);
        let mut signals = Vec::new();
        let mut drivers = Vec::new();
        let product_ports = (0..36)
            .map(|bit| format!("P{bit}"))
            .chain(["SIGNEDP".into()])
            .filter(|_| product);
        for name in product_ports
            .chain((0..18).map(|bit| format!("ROA{bit}")))
            .chain((0..18).map(|bit| format!("ROB{bit}")))
        {
            let port = ports
                .iter()
                .find(|p| p.name == name && p.direction == Direction::Output)
                .with_context(|| format!("missing MULT18 output {name}"))?;
            let signal = self.signal(port.wire.clone())?;
            let label = format!("mult18:{key}:{name}");
            self.label(&label)?;
            drivers.push(self.driver(signal, label, Strength::Strong, Logic::X, false));
            signals.push(signal);
        }
        let index = self.multipliers.len();
        for input in inputs {
            if let Some(signal) = input.signal() {
                self.signals[signal.index].multipliers.push(index);
            }
        }
        self.bound_multipliers.insert(config.resource);
        let operand_offset = if product { 37 } else { 0 };
        self.multipliers.push(Multiplier {
            config,
            pipeline,
            previous_clock: [Logic::X; 4],
            global_at_rise: [Logic::Zero; 4],
            product_drivers: product.then(|| drivers[..37].try_into().unwrap()),
            operand_drivers: drivers[operand_offset..].try_into().unwrap(),
        });
        Ok((
            product.then(|| (signals[..36].try_into().unwrap(), signals[36])),
            MultiplierOperandOutputs {
                roa: signals[operand_offset..operand_offset + 18]
                    .try_into()
                    .unwrap(),
                rob: signals[operand_offset + 18..].try_into().unwrap(),
            },
        ))
    }
}
impl Kernel {
    pub(super) fn initialize_multiplier_clocks(&mut self) {
        for m in &mut self.multipliers {
            m.previous_clock = m
                .pipeline
                .stages()
                .map(|s| s.map_or(Logic::Zero, |s| s.clock.read(&self.signals)));
        }
    }
    pub(super) fn capture_multipliers(&mut self) -> Result<DspCaptures> {
        let mut captures = Vec::new();
        let mut alus = Vec::new();
        for index in self.dirty_multiplier_registers.take() {
            // The ALUs follow the multipliers in these sets (Builder::build).
            if index >= self.multipliers.len() {
                alus.push(index - self.multipliers.len());
                continue;
            }
            let m = &mut self.multipliers[index];
            let old = self.state.multiplier(m.config.resource)?;
            let mut next = old.clone();
            let mut output = None;
            let global = m
                .pipeline
                .global_reset
                .map_or(Logic::Zero, |v| v.read(&self.signals));
            let (a, b, sa, sb) = m.operands(old, &self.signals);
            // Compute every stage from old state before committing any stage.
            for (stage, control) in m.pipeline.stages().into_iter().enumerate() {
                let Some(control) = control else {
                    continue;
                };
                self.stats.multiplier_register_evaluations += 1;
                let clock = control.clock.read(&self.signals);
                let global = dsp_global(
                    m.pipeline.both_edges,
                    m.previous_clock[stage],
                    clock,
                    global,
                    &mut m.global_at_rise[stage],
                );
                let edge = dsp_edge(m.pipeline.both_edges, m.previous_clock[stage], clock);
                m.previous_clock[stage] = clock;
                if edge != Activity::None {
                    self.stats.multiplier_register_edges += 1;
                }
                if edge == Activity::Possible {
                    self.stats.possible_clock_edges += 1;
                }
                match stage {
                    0 | 1 => {
                        let (previous, data, sign, sign_data) = if stage == 0 {
                            (old.input_a, m.config.a, old.signed_a, m.config.signed_a)
                        } else {
                            (old.input_b, m.config.b, old.signed_b, m.config.signed_b)
                        };
                        let q = capture(
                            previous,
                            data.map(|v| v.read(&self.signals)),
                            control,
                            edge,
                            &self.signals,
                            global,
                        );
                        let s = capture(
                            [sign],
                            [sign_data.read(&self.signals)],
                            control,
                            edge,
                            &self.signals,
                            global,
                        )[0];
                        if stage == 0 {
                            next.input_a = q;
                            next.signed_a = s;
                        } else {
                            next.input_b = q;
                            next.signed_b = s;
                        }
                    }
                    2 => {
                        next.product = capture(
                            old.product,
                            multiply18(&a, &b, sa, sb),
                            control,
                            edge,
                            &self.signals,
                            global,
                        );
                        next.signed_product = capture(
                            [old.signed_product],
                            [sa.or(sb)],
                            control,
                            edge,
                            &self.signals,
                            global,
                        )[0];
                    }
                    3 => {
                        let previous = self.state.multiplier_output(m.config.resource)?;
                        let data = if m.pipeline.product.is_some() {
                            old.product
                        } else {
                            multiply18(&a, &b, sa, sb)
                        };
                        // GSR clears it at once and holds it (HD07).
                        let value = capture(previous, data, control, edge, &self.signals, global);
                        if value != previous {
                            output = Some(value);
                        }
                    }
                    _ => unreachable!(),
                }
            }
            if &next != old || output.is_some() {
                captures.push(MultiplierCapture {
                    index,
                    state: next,
                    output,
                });
            }
        }
        Ok(DspCaptures {
            multipliers: captures,
            alus: self.capture_alus(&alus)?,
        })
    }
    pub(super) fn commit_multipliers(&mut self, captures: DspCaptures) -> Result<()> {
        for alu in self.commit_alus(captures.alus) {
            self.dirty_multipliers.insert(self.multipliers.len() + alu);
        }
        for MultiplierCapture {
            index,
            state: next,
            output,
        } in captures.multipliers
        {
            let id = self.multipliers[index].config.resource;
            self.stats.multiplier_register_bit_changes += self
                .state
                .multiplier(id)?
                .bits()
                .zip(next.bits())
                .filter(|(a, b)| a != b)
                .count() as u64;
            self.state.set_multiplier(id, next)?;
            if let Some(value) = output {
                let changed = self
                    .state
                    .multiplier_output(id)?
                    .into_iter()
                    .zip(value)
                    .filter(|(a, b)| a != b)
                    .count() as u64;
                self.stats.multiplier_register_bit_changes += changed;
                self.stats.dsp_output_register_bit_changes += changed;
                // Update only this product's view, preserving a peer capture
                // into the other half of the same bank in this capture wave.
                self.state.set_multiplier_output(id, value)?;
            }
            self.dirty_multipliers.insert(index);
            // Revisit uncertain asynchronous controls against the new data.
            self.dirty_multiplier_registers.insert(index);
        }
        Ok(())
    }
    pub(super) fn evaluate_multipliers(&mut self) -> Result<Vec<Update>> {
        let mut updates = self.update_buffer();
        let dirty = self.dirty_multipliers.take();
        for &index in &dirty {
            if index >= self.multipliers.len() {
                self.evaluate_alu(index - self.multipliers.len(), &mut updates);
                continue;
            }
            let multiplier = &self.multipliers[index];
            let config = &multiplier.config;
            ensure!(config.source_a.read(&self.signals)==Logic::Zero && config.source_b.read(&self.signals)==Logic::Zero,
                "MULT18 {} uses unknown/nonparallel SOURCEA/B; shift, C and highspeed paths are not implemented",self.device.model().resource(config.resource).key);
            self.stats.multiplier_evaluations += 1;
            let state = self.state.multiplier(config.resource)?;
            let (a, b, sa, sb) = multiplier.operands(state, &self.signals);
            let mut values = Vec::new();
            if let Some(drivers) = &multiplier.product_drivers {
                let (product, signed) = if multiplier.pipeline.output.is_some() {
                    (self.state.multiplier_output(config.resource)?, Logic::X)
                } else if multiplier.pipeline.product.is_some() {
                    (state.product, state.signed_product)
                } else {
                    (multiply18(&a, &b, sa, sb), sa.or(sb))
                };
                values.extend(
                    drivers
                        .iter()
                        .copied()
                        .zip(product.into_iter().chain([signed])),
                );
            }
            values.extend(
                multiplier
                    .operand_drivers
                    .iter()
                    .copied()
                    .zip(a.into_iter().chain(b)),
            );
            for (driver, value) in values {
                updates.push(Update {
                    driver,
                    value,
                    reason: Reason::Static("parallel MULT18: ROA/ROB follow INPUTA/B; P follows selected input/pipeline/output stages and the shared output bank"),
                });
            }
        }
        self.dirty_multipliers.recycle(dirty);
        Ok(updates)
    }
}

/// Both polarities count with CLK_DIV disabled (HD09). An uncertain
/// transition remains uncertain, including transitions between X and Z.
pub(super) fn dsp_edge(both: bool, previous: Logic, current: Logic) -> Activity {
    let rise = clock_edge(Edge::Rising, previous, current);
    if !both {
        return rise;
    }
    let fall = clock_edge(Edge::Falling, previous, current);
    match (rise, fall) {
        (Activity::Definite, _) | (_, Activity::Definite) => Activity::Definite,
        (Activity::Possible, _) | (_, Activity::Possible) => Activity::Possible,
        _ => Activity::None,
    }
}

/// HD07/HD09: GSR active at a rising edge holds through that clock period.
/// This matters with disabled dividers: release after the rise must not let
/// the following falling edge reload a stage that GSR just cleared.
pub(super) fn dsp_global(
    both: bool,
    previous: Logic,
    clock: Logic,
    global: Logic,
    held: &mut Logic,
) -> Logic {
    if !both {
        return global;
    }
    *held = match clock_edge(Edge::Rising, previous, clock) {
        Activity::Definite => global,
        Activity::Possible => Logic::mux(Logic::X, *held, global),
        Activity::None => *held,
    };
    global.or(*held)
}
