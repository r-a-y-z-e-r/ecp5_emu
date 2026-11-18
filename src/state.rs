//! Mutable storage snapshot. Clock/reset/write-port semantics belong to the kernel.
use crate::{
    logic::Logic,
    model::{topology::Wire, NodeId, ResourceId},
    prepared::ConfiguredDevice,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
struct EbrStorage {
    words: Vec<[Logic; 9]>,
    init_status: String,
    init_coverage: &'static str,
    pipeline: EbrPipeline,
}

/// Two physical 18-bit read paths and their optional output registers. These
/// are shared by DP/PDP views, never additional copies of the SRAM array.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EbrPipeline {
    pub read_latches: [[Logic; 18]; 2],
    pub output_registers: [[Logic; 18]; 2],
}

/// One shared OR/FR bank. Parallel MULT mode has two logical 36-bit product
/// views, in native multiplier order. ALU/flag/framing aliases and physical FF
/// coordinates remain D01; this is not a complete DSP register census.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct DspSliceState {
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    shared_output: [Logic; 72],
}
impl DspSliceState {
    pub fn shared_output(&self) -> &[Logic; 72] {
        &self.shared_output
    }
}

/// Logical views for the parallel MULT18 profile. These do not assign physical
/// FF coordinates, C/shift storage or any bits of the shared OR/FR bank (D01/D05).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct MultiplierState {
    pub input_a: [Logic; 18],
    pub input_b: [Logic; 18],
    pub signed_a: Logic,
    pub signed_b: Logic,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub product: [Logic; 36],
    pub signed_product: Logic,
}
impl MultiplierState {
    pub fn bits(&self) -> impl Iterator<Item = Logic> + '_ {
        self.input_a
            .into_iter()
            .chain(self.input_b)
            .chain([self.signed_a, self.signed_b])
            .chain(self.product)
            .chain([self.signed_product])
    }
}

/// Logical system strobes after configuration has been loaded. Reset/disable
/// values are active high here, independently of physical GSRN/GWDISn polarity.
#[derive(Clone, Debug, Serialize)]
pub struct SystemState {
    pub profile: &'static str,
    pub phase: Option<u8>,
    pub goe: Logic,
    pub gwdis: Logic,
    pub startup_gsr: Logic,
    pub user_gsr: Logic,
    pub gsr: Logic,
    pub done: Logic,
    pub ready: Logic,
    pub power_on_reset: &'static str,
}

#[derive(Serialize)]
pub struct StateSummary {
    pub configuration_sha256: String,
    pub lut_arrays: usize,
    pub lut_bits: usize,
    pub known_lut_bits: usize,
    pub flip_flops: usize,
    pub known_flip_flops: usize,
    pub ebr_arrays: usize,
    pub known_ebr_bits: usize,
    pub known_ebr_latch_bits: usize,
    pub known_ebr_output_register_bits: usize,
    pub ebr_initialization: std::collections::BTreeMap<String, usize>,
    pub dsp_slices: usize,
    pub dsp_shared_output_bits: usize,
    pub known_dsp_shared_output_bits: usize,
    pub dsp_storage_mapping: &'static str,
    pub dsp_multiplier_views: usize,
    pub known_dsp_multiplier_bits: usize,
    pub observed_wire_values: usize,
    pub execution: &'static str,
    pub system: Option<SystemState>,
}

#[cfg_attr(test, derive(serde::Serialize))]
pub struct DeviceState {
    configuration: String,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    luts: HashMap<NodeId, [Logic; 16]>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    registers: HashMap<NodeId, Logic>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    ebr: HashMap<ResourceId, EbrStorage>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    dsp: HashMap<ResourceId, DspSliceState>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    dsp_owners: HashMap<ResourceId, ResourceId>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    dsp_output_offsets: HashMap<ResourceId, usize>,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    dsp_multipliers: HashMap<ResourceId, MultiplierState>,
    /// Wire values live in a dense vector; the map only resolves names to slots.
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::map"))]
    wire_slots: HashMap<Wire, usize>,
    wire_values: Vec<Logic>,
    max_row: i16,
    max_col: i16,
    execution: &'static str,
    system: Option<SystemState>,
}

impl DeviceState {
    /// Storage read by C generation. Wire-name lookup and event bookkeeping
    /// stay with the live state; the worker only reads LUT, EBR and DSP cells.
    pub(crate) fn fast_snapshot(&self) -> crate::snapshot::AllocationResult<Self> {
        use crate::snapshot;
        let mut ebr = HashMap::new();
        ebr.try_reserve(self.ebr.len())?;
        for (&resource, block) in &self.ebr {
            ebr.insert(
                resource,
                EbrStorage {
                    words: snapshot::cloned(&block.words)?,
                    init_status: snapshot::string(&block.init_status)?,
                    init_coverage: block.init_coverage,
                    pipeline: block.pipeline.clone(),
                },
            );
        }
        Ok(Self {
            configuration: snapshot::string(&self.configuration)?,
            luts: snapshot::map(&self.luts)?,
            registers: snapshot::map(&self.registers)?,
            ebr,
            dsp: snapshot::map(&self.dsp)?,
            dsp_owners: snapshot::map(&self.dsp_owners)?,
            dsp_output_offsets: snapshot::map(&self.dsp_output_offsets)?,
            dsp_multipliers: snapshot::map(&self.dsp_multipliers)?,
            wire_slots: HashMap::new(),
            wire_values: Vec::new(),
            max_row: self.max_row,
            max_col: self.max_col,
            execution: self.execution,
            system: self.system.clone(),
        })
    }
    pub fn new(device: &ConfiguredDevice) -> Result<Self> {
        let model = device.model();
        let mut state = Self {
            configuration: device.fingerprint().into(),
            luts: HashMap::new(),
            registers: HashMap::new(),
            ebr: HashMap::new(),
            dsp: HashMap::new(),
            dsp_owners: HashMap::new(),
            dsp_output_offsets: HashMap::new(),
            dsp_multipliers: HashMap::new(),
            wire_slots: HashMap::new(),
            wire_values: Vec::new(),
            max_row: model.database().max_row,
            max_col: model.database().max_col,
            execution: "storage_snapshot;event_kernel_not_run",
            system: None,
        };
        for (id, r) in model.resources() {
            if r.family == "MULT18" {
                state.dsp_multipliers.insert(
                    id,
                    MultiplierState {
                        input_a: [Logic::X; 18],
                        input_b: [Logic::X; 18],
                        signed_a: Logic::X,
                        signed_b: Logic::X,
                        product: [Logic::X; 36],
                        signed_product: Logic::X,
                    },
                );
            }
            if r.family == "DSP_SLICE" {
                let mut products = model
                    .children(id)
                    .iter()
                    .copied()
                    .filter(|&p| model.resource(p).family == "MULT18")
                    .collect::<Vec<_>>();
                products.sort_by_key(|&p| model.resource(p).location.col);
                ensure!(
                    products.len() == 2,
                    "DSP slice must contain two native MULT18 product views"
                );
                for (lane, product) in products.into_iter().enumerate() {
                    state.dsp_output_offsets.insert(product, lane * 36);
                }
                state.dsp.insert(
                    id,
                    DspSliceState {
                        shared_output: [Logic::X; 72],
                    },
                );
            }
            if ["MULT18", "ALU54"].contains(&r.family.as_str()) {
                let parent = r.parent.context("DSP primitive has no slice owner")?;
                ensure!(
                    model.resource(parent).family == "DSP_SLICE",
                    "DSP primitive owner is not a slice"
                );
                state.dsp_owners.insert(id, parent);
            }
            if r.family == "PLC2" {
                for node in model.nodes(id) {
                    match node.kind {
                        "TRELLIS_COMB" => {
                            let bits =
                                device.word(r.anchor, &format!("{}.INIT", node.native_name))?;
                            let bits: [Logic; 16] = bits
                                .try_into()
                                .map_err(|_| anyhow::anyhow!("LUT INIT must be 16 bits"))?;
                            ensure!(
                                state.luts.insert(node.id, bits).is_none(),
                                "duplicate LUT storage owner"
                            );
                        }
                        "TRELLIS_FF" => {
                            // REGSET alone does not establish the startup/PUR state (S01).
                            ensure!(
                                state.registers.insert(node.id, Logic::X).is_none(),
                                "duplicate FF storage owner"
                            );
                        }
                        _ => {} // RAMW is a view/control function, not a second LUT RAM array.
                    }
                }
            } else if r.family == "EBR" {
                let mut words = vec![[Logic::X; 9]; 2048];
                let memory = device.resource_settings(id).and_then(|r| r.memory.as_ref());
                let status = memory
                    .map(|m| m.status.as_str())
                    .unwrap_or("no_configuration_entry");
                if let Some(init) = memory
                    .and_then(|m| m.image_id)
                    .and_then(|wid| device.image().cram().bram.get(&wid))
                {
                    ensure!(init.len() == 2048, "bound EBR image is not complete length");
                    for (dest, &value) in words.iter_mut().zip(init) {
                        for (bit, out) in dest.iter_mut().enumerate() {
                            *out = Logic::from_bool(value & (1 << bit) != 0);
                        }
                    }
                }
                state.ebr.insert(
                    id,
                    EbrStorage {
                        words,
                        init_status: status.into(),
                        init_coverage: if device.image().cram().origin == "text_config" {
                            "explicit_text_words"
                        } else {
                            "native_decoded_without_write_mask"
                        },
                        pipeline: EbrPipeline {
                            read_latches: [[Logic::X; 18]; 2],
                            output_registers: [[Logic::X; 18]; 2],
                        },
                    },
                );
            }
        }
        Ok(state)
    }
    pub fn belongs_to(&self, device: &ConfiguredDevice) -> bool {
        self.configuration == device.fingerprint()
    }
    pub fn multiplier(&self, id: ResourceId) -> Result<&MultiplierState> {
        self.dsp_multipliers
            .get(&id)
            .context("resource has no MULT18 state")
    }
    pub(crate) fn set_multiplier(&mut self, id: ResourceId, value: MultiplierState) -> Result<()> {
        *self
            .dsp_multipliers
            .get_mut(&id)
            .context("resource has no MULT18 state")? = value;
        Ok(())
    }
    /// Logical product coordinates, not silicon FF numbering or ALU aliases.
    pub fn multiplier_output_view(
        &self,
        id: ResourceId,
    ) -> Result<(ResourceId, std::ops::Range<usize>)> {
        let owner = *self
            .dsp_owners
            .get(&id)
            .context("MULT18 has no DSP slice")?;
        let offset = *self
            .dsp_output_offsets
            .get(&id)
            .context("resource has no MULT18 output view")?;
        Ok((owner, offset..offset + 36))
    }
    pub fn multiplier_output(&self, id: ResourceId) -> Result<[Logic; 36]> {
        let (owner, range) = self.multiplier_output_view(id)?;
        Ok(self.dsp[&owner].shared_output[range].try_into().unwrap())
    }
    pub(crate) fn set_multiplier_output(
        &mut self,
        id: ResourceId,
        value: [Logic; 36],
    ) -> Result<()> {
        let (owner, range) = self.multiplier_output_view(id)?;
        self.dsp
            .get_mut(&owner)
            .context("missing shared DSP bank")?
            .shared_output[range]
            .copy_from_slice(&value);
        Ok(())
    }
    /// MULT18, ALU54 and the slice group address the same bank, never copies.
    pub fn dsp_slice(&self, id: ResourceId) -> Result<(ResourceId, &DspSliceState)> {
        let owner = self.dsp_owners.get(&id).copied().unwrap_or(id);
        Ok((
            owner,
            self.dsp
                .get(&owner)
                .context("resource has no DSP slice storage")?,
        ))
    }
    pub fn system(&self) -> Option<&SystemState> {
        self.system.as_ref()
    }
    pub(crate) fn set_system(&mut self, state: SystemState) {
        self.system = Some(state);
    }
    pub(crate) fn mark_event_kernel_run(&mut self) {
        self.execution = "event_kernel;explicit_execution_plan";
    }
    pub fn lut(&self, id: NodeId) -> Result<&[Logic; 16]> {
        self.luts.get(&id).context("node has no LUT storage")
    }
    pub fn write_lut(&mut self, id: NodeId, address: usize, value: Logic) -> Result<()> {
        *self
            .luts
            .get_mut(&id)
            .context("node has no LUT storage")?
            .get_mut(address)
            .context("LUT address outside 16 bits")? = value;
        Ok(())
    }
    pub fn register(&self, id: NodeId) -> Result<Logic> {
        self.registers
            .get(&id)
            .copied()
            .context("node has no FF storage")
    }
    pub fn write_register(&mut self, id: NodeId, value: Logic) -> Result<()> {
        *self
            .registers
            .get_mut(&id)
            .context("node has no FF storage")? = value;
        Ok(())
    }
    pub fn ebr_word(&self, id: ResourceId, address: usize) -> Result<&[Logic; 9]> {
        self.ebr
            .get(&id)
            .context("resource has no EBR storage")?
            .words
            .get(address)
            .context("EBR address outside 2048 words")
    }
    pub fn write_ebr_word(
        &mut self,
        id: ResourceId,
        address: usize,
        value: [Logic; 9],
    ) -> Result<()> {
        *self
            .ebr
            .get_mut(&id)
            .context("resource has no EBR storage")?
            .words
            .get_mut(address)
            .context("EBR address outside 2048 words")? = value;
        Ok(())
    }
    pub fn ebr_init_evidence(&self, id: ResourceId) -> Result<(&str, &str)> {
        let m = self.ebr.get(&id).context("resource has no EBR storage")?;
        Ok((&m.init_status, m.init_coverage))
    }
    pub fn ebr_pipeline(&self, id: ResourceId) -> Result<&EbrPipeline> {
        Ok(&self
            .ebr
            .get(&id)
            .context("resource has no EBR storage")?
            .pipeline)
    }
    pub(crate) fn set_ebr_pipeline(&mut self, id: ResourceId, value: EbrPipeline) -> Result<()> {
        self.ebr
            .get_mut(&id)
            .context("resource has no EBR storage")?
            .pipeline = value;
        Ok(())
    }
    pub fn wire_value(&self, wire: &Wire) -> Option<Logic> {
        self.wire_slots
            .get(wire)
            .map(|&slot| self.wire_values[slot])
    }
    pub(crate) fn observed_wire_count(&self) -> usize {
        self.wire_values.len()
    }
    pub fn set_wire_value(&mut self, wire: Wire, value: Logic) -> Result<()> {
        self.register_wire(wire, value).map(|_| ())
    }
    /// Returns the wire's slot; new wires get consecutive slots from zero.
    pub(crate) fn register_wire(&mut self, wire: Wire, value: Logic) -> Result<usize> {
        ensure!(
            !wire.name.is_empty()
                && (0..=self.max_row).contains(&wire.location.row)
                && (0..=self.max_col).contains(&wire.location.col),
            "invalid wire state key"
        );
        if let Some(&slot) = self.wire_slots.get(&wire) {
            self.wire_values[slot] = value;
            return Ok(slot);
        }
        let slot = self.wire_values.len();
        self.wire_slots.insert(wire, slot);
        self.wire_values.push(value);
        Ok(slot)
    }
    pub(crate) fn set_wire_slot(&mut self, slot: usize, value: Logic) -> Result<()> {
        *self
            .wire_values
            .get_mut(slot)
            .context("unknown wire state slot")? = value;
        Ok(())
    }
    pub fn summary(&self) -> StateSummary {
        let mut init = std::collections::BTreeMap::new();
        for m in self.ebr.values() {
            *init.entry(m.init_status.clone()).or_insert(0) += 1;
        }
        StateSummary {
            configuration_sha256: self.configuration.clone(),
            lut_arrays: self.luts.len(),
            lut_bits: self.luts.len() * 16,
            known_lut_bits: self
                .luts
                .values()
                .flatten()
                .filter(|v| v.bit().is_some())
                .count(),
            flip_flops: self.registers.len(),
            known_flip_flops: self
                .registers
                .values()
                .filter(|v| v.bit().is_some())
                .count(),
            ebr_arrays: self.ebr.len(),
            known_ebr_bits: self
                .ebr
                .values()
                .flat_map(|m| &m.words)
                .flatten()
                .filter(|v| v.bit().is_some())
                .count(),
            known_ebr_latch_bits: self
                .ebr
                .values()
                .flat_map(|m| m.pipeline.read_latches.iter().flatten())
                .filter(|v| v.bit().is_some())
                .count(),
            known_ebr_output_register_bits: self
                .ebr
                .values()
                .flat_map(|m| m.pipeline.output_registers.iter().flatten())
                .filter(|v| v.bit().is_some())
                .count(),
            ebr_initialization: init,
            dsp_slices:self.dsp.len(),
            dsp_shared_output_bits:self.dsp.len()*72,
            known_dsp_shared_output_bits:self.dsp.values().flat_map(|s|&s.shared_output).filter(|v|v.bit().is_some()).count(),
            dsp_storage_mapping:"one OR/FR 72-bit bank per slice; parallel MULT products occupy two logical 36-bit views; ALU/flag/framing/physical mappings remain D01/D05",
            dsp_multiplier_views: self.dsp_multipliers.len(),
            known_dsp_multiplier_bits: self.dsp_multipliers.values().flat_map(MultiplierState::bits).filter(|v| v.bit().is_some()).count(),
            observed_wire_values: self.wire_values.len(),
            execution: self.execution,
            system: self.system.clone(),
        }
    }
}
