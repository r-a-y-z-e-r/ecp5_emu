//! Parallel MULT18 profile; D01/D04/D05 keep unverified modes explicit.
use super::*;
use crate::kernel::{
    Alu54, AluRegister, Multiplier18, MultiplierOutputRegister, MultiplierPipeline,
    MultiplierRegister, ALU_FLAGS,
};

/// What the planner takes from an executable ALU54B's settings.
struct AluSettings {
    both_edges: bool,
    /// CLK0..CLK3 of the registered bits of R, if any is registered.
    clock: Option<String>,
    rndpat: u64,
    /// Bits of R that leave before the register (a CIBOUT_BYP half).
    unregistered: u64,
    /// Bits of R whose register has no clock.
    stuck: u64,
    /// Bits of R whose reset is asynchronous.
    async_reset: u64,
    /// RST0..RST3: the reset of R[17:0] and R[44:27] (REG_OUTPUT0/1_RST, the
    /// same bits as the first MULT18's REG_OUTPUT_RST). The other half takes
    /// RST0 (HD07).
    first_reset: String,
    /// The slice's GSR is enabled: GSR clears R (HD07).
    gsr: bool,
    /// The opcode register stages of IN, OP0, OP1 (HD08): clock, enable and
    /// reset port names.
    opcode: [[Option<[String; 3]>; 2]; 3],
    /// Their reset is asynchronous: RESETMODE of the ALU (HD08 o2, o6).
    opcode_async: bool,
    /// The clock of the C registers C0 and C1, where they are on (HD08).
    c_registers: [Option<String>; 2],
    /// Their reset is asynchronous (RESETMODE, the same on the ALU and the
    /// MULT18 of each, HD08 c1, c2).
    c_async: [bool; 2],
    /// The flags as constants, and their register's clock.
    flags: [Logic; 8],
    flag_clock: Option<String>,
    /// False for registered flags beside a bypass of their half: not measured.
    flags_measured: bool,
}
/// The four groups of R on the board (HD03, HD04, HD06): their bits, whether
/// REG_OUTPUT1_CLK (else REG_OUTPUT0_CLK) clocks them, and whether they
/// belong to the second MULT18's half of the output bank (else the first's):
/// that MULT18's CIBOUT_BYP takes them before the register, and its RESETMODE
/// makes their reset asynchronous (the ALU's own RESETMODE does that for the
/// first half only).
const ALU_GROUPS: [(u64, bool, bool); 4] = [
    (0x3_FFFF, false, false),
    (0x7FC_0000, false, true),
    (0x1FFF_F800_0000, true, false),
    (0x3F_E000_0000_0000, false, true),
];

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct MultiplierStageUse {
    pub stage: String,
    pub clock: String,
    pub enable: String,
    pub reset: String,
    pub reset_mode: String,
}

// These two labels belong to the executable, never to the cache input buffer.
type ExecutionView = &'static str;

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct MultiplierUse {
    pub resource: String,
    pub slice: String,
    #[serde(deserialize_with = "deserialize_execution_view")]
    pub execution_view: ExecutionView,
    pub register_candidates: BTreeMap<String, Vec<String>>,
    pub cibout_bypass: String,
    pub output_path_decode: Option<crate::decode::DspOutputPath>,
    pub implicit_unmodelled_parameters: Vec<String>,
    pub stages: Vec<MultiplierStageUse>,
    pub gsr_enabled: bool,
    pub behavioural_assumptions: Vec<String>,
    pub output_register: Option<MultiplierOutputUse>,
    pub rule: String,
}
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct MultiplierOutputUse {
    pub clock: String,
    pub logical_bit_start: usize,
    pub width: usize,
    pub rule: String,
}

fn deserialize_execution_view<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<&'static str, D::Error> {
    let text = <String as serde::Deserialize>::deserialize(d)?;
    match text.as_str() {
        "operands_only" => Ok("operands_only"),
        "product_and_operands" => Ok("product_and_operands"),
        _ => Err(serde::de::Error::custom(
            "unknown multiplier execution view",
        )),
    }
}
impl Planner {
    pub(super) fn execute_multiplier(
        &mut self,
        id: ResourceId,
        output: Option<&str>,
    ) -> Result<()> {
        if let Some(output) = output {
            ensure!(
                output == "SIGNEDP"
                    || [("P", 36), ("ROA", 18), ("ROB", 18)]
                        .iter()
                        .any(|(prefix, width)| output
                            .strip_prefix(prefix)
                            .is_some_and(|bit| bit.parse::<usize>().is_ok_and(|bit| bit < *width))),
                "unsupported MULT18 output {output}; shift/C forwarding is not implemented"
            );
        }
        ensure!(
            self.composed_one(id, ".MODE")? == "MULT18X18D",
            "inactive/unsupported multiplier mode"
        );
        let slice = self
            .device
            .model()
            .resource(id)
            .parent
            .context("MULT18 has no slice")?;
        let mut active_alu = None;
        for &peer in self.device.model().children(slice) {
            if self.device.model().resource(peer).family == "ALU54" {
                match self.qualified_fallback(peer, ".MODE", "NONE")?.as_str() {
                    "NONE" => {}
                    "ALU54B" => active_alu = Some(peer),
                    _ => bail!("unsupported ALU54 mode sharing the DSP slice"),
                }
            }
        }
        // An ALU54B the planner can execute takes the products (MA, MB); the
        // output bank is the ALU's then. With any other active ALU only the
        // operands can be observed.
        let alu_view = match active_alu {
            Some(alu) => self.alu_settings(alu).is_ok(),
            None => false,
        };
        let operands_only = active_alu.is_some() && !alu_view;
        ensure!(!operands_only || output.is_some_and(|p| p.starts_with("ROA") || p.starts_with("ROB")),
            "active ALU54 outside the measured profile ({:#}): only ROA/ROB operand observation is implemented",
            self.alu_settings(active_alu.unwrap()).err().unwrap());
        ensure!(
            self.composed_one(slice, ".CIBOUT")? == "ON",
            "DSP slice CIB output is disabled"
        );
        let bypass = if operands_only {
            "not_used_operand_view".into()
        } else if alu_view {
            "product_to_alu".into()
        } else {
            self.multiplier_output_path(id)?
        };
        ensure!(
            operands_only || alu_view || ["ON", "OFF"].contains(&bypass.as_str()),
            "unknown MULT18 output path"
        );
        ensure!(
            operands_only || alu_view || bypass == "ON" || output != Some("SIGNEDP"),
            "registered MULT18 SIGNEDP is unresolved D01/D05; the product output stage does not model SIGNEDP"
        );
        // Check each requested output even when an earlier product/forwarding
        // probe already bound the primitive (notably unresolved SIGNEDP).
        if self.done_resources.contains(&id) {
            return Ok(());
        }
        let mut registers = BTreeMap::new();
        for stage in ["INPUTA", "INPUTB", "INPUTC", "PIPELINE", "OUTPUT"] {
            if operands_only && !["INPUTA", "INPUTB"].contains(&stage) {
                continue;
            }
            // In an executed ALU slice the output bank is the ALU's, and the C
            // register is the ALU's C register (the same bits, HD08).
            if alu_view && (stage == "OUTPUT" || stage == "INPUTC") {
                continue;
            }
            let suffix = format!(".REG_{stage}_CLK");
            let raw = self
                .device
                .resource_settings(id)
                .unwrap()
                .fields
                .iter()
                .find(|f| f.name.ends_with(&suffix))
                .context("missing DSP register field")?;
            let choices = if stage == "OUTPUT"
                && raw.status == "unobserved"
                && !raw.observed_bits.is_empty()
                && raw.candidates == ["NONE"]
            {
                raw.candidates.clone()
            } else {
                self.composed(id, &suffix)?
            };
            // ON CIBOUT_BYP selects the combinational path even if NONE and
            // CLK3 share an empty clock code in the inactive output stage.
            let clocks = ["CLK0", "CLK1", "CLK2", "CLK3"];
            let valid = match stage {
                "INPUTC" => choices == ["NONE"],
                "OUTPUT" if bypass == "OFF" => {
                    choices.len() == 1 && clocks.contains(&choices[0].as_str())
                }
                "OUTPUT" => {
                    choices.iter().any(|s| s == "NONE")
                        && choices
                            .iter()
                            .all(|s| ["NONE", "CLK3"].contains(&s.as_str()))
                }
                _ => {
                    choices.len() == 1
                        && (choices[0] == "NONE" || clocks.contains(&choices[0].as_str()))
                }
            };
            ensure!(
                valid,
                "MULT18 unsupported/unresolved {stage} stage: {choices:?}"
            );
            registers.insert(format!("REG_{stage}_CLK"), choices);
        }
        ensure!(
            self.composed_one(id, ".SOURCEB_MODE")? == "B_SHIFT",
            "MULT18 C/dynamic/highspeed source mode is not implemented"
        );
        let mut a = [Input::Constant(Logic::X); 18];
        let mut b = a;
        for bit in 0..18 {
            a[bit] = self.input(&self.resource_port(id, &format!("A{bit}"))?)?;
            b[bit] = self.input(&self.resource_port(id, &format!("B{bit}"))?)?;
        }
        let signed_a = self.input(&self.resource_port(id, "SIGNEDA")?)?;
        let signed_b = self.input(&self.resource_port(id, "SIGNEDB")?)?;
        let source_a = self.input(&self.resource_port(id, "SOURCEA")?)?;
        let source_b = self.input(&self.resource_port(id, "SOURCEB")?)?;
        let clocks: Vec<_> = registers
            .values()
            .filter(|v| v.len() == 1 && v[0] != "NONE")
            .map(|v| v[0].as_str())
            .collect();
        let mode = self.dsp_clock_mode(id, &clocks)?;
        if !clocks.is_empty() {
            self.dsp_divider_profile(mode)?;
        }
        let mut pipeline = MultiplierPipeline {
            both_edges: mode,
            ..Default::default()
        };
        let mut stages = Vec::new();
        for (stage, target) in [
            ("INPUTA", &mut pipeline.input_a),
            ("INPUTB", &mut pipeline.input_b),
            ("PIPELINE", &mut pipeline.product),
        ] {
            let Some(choices) = registers.get(&format!("REG_{stage}_CLK")) else {
                continue;
            };
            let clock = &choices[0];
            if clock == "NONE" {
                continue;
            }
            let enable = self.dsp_control(id, &format!(".REG_{stage}_CE"), "CE")?;
            let reset = self.dsp_control(id, &format!(".REG_{stage}_RST"), "RST")?;
            ensure!(
                ["CE0", "CE1", "CE2", "CE3"].contains(&enable.as_str()),
                "invalid DSP CE selection"
            );
            ensure!(
                ["RST0", "RST1", "RST2", "RST3"].contains(&reset.as_str()),
                "invalid DSP reset selection"
            );
            let reset_mode = self.qualified_fallback(id, ".RESETMODE", "SYNC")?;
            let reset_kind = match reset_mode.as_str() {
                "SYNC" => ResetKind::Synchronous,
                "ASYNC" => ResetKind::Asynchronous,
                _ => bail!("unsupported DSP RESETMODE"),
            };
            *target = Some(MultiplierRegister {
                clock: self.multiplier_mux(id, clock)?,
                enable: self.multiplier_mux(id, &enable)?,
                reset: self.multiplier_mux(id, &reset)?,
                reset_kind,
            });
            stages.push(MultiplierStageUse {
                stage: stage.into(),
                clock: clock.clone(),
                enable,
                reset,
                reset_mode,
            });
        }
        let mut gsr_enabled = false;
        if !stages.is_empty() || bypass == "OFF" {
            gsr_enabled = match self.qualified_fallback(id, ".GSR", "ENABLED")?.as_str() {
                "ENABLED" => true,
                "DISABLED" => false,
                _ => bail!("unsupported DSP GSR"),
            };
            if gsr_enabled {
                pipeline.global_reset = Some(self.system_input(SystemPort::Gsr)?);
            }
        }
        let output_register = if bypass == "OFF" {
            let clock = registers["REG_OUTPUT_CLK"][0].clone();
            // HD01 on the board: the output stage captures under CE0 whatever
            // REG_OUTPUT_CE says (Trellis has no such field); its reset is the
            // REG_OUTPUT_RST selection where the field exists (MULT18 0/4) and
            // RST0 where it does not (1/5).
            let has_reset_field = self
                .device
                .resource_settings(id)
                .is_some_and(|r| r.fields.iter().any(|f| f.name.ends_with(".REG_OUTPUT_RST")));
            let reset = if has_reset_field {
                self.qualified_fallback(id, ".REG_OUTPUT_RST", "RST0")?
            } else {
                "RST0".to_string()
            };
            ensure!(
                ["RST0", "RST1", "RST2", "RST3"].contains(&reset.as_str()),
                "invalid DSP output reset selection"
            );
            let reset_kind = match self.qualified_fallback(id, ".RESETMODE", "SYNC")?.as_str() {
                "SYNC" => ResetKind::Synchronous,
                "ASYNC" => ResetKind::Asynchronous,
                _ => bail!("unsupported DSP RESETMODE"),
            };
            pipeline.output = Some(MultiplierOutputRegister {
                clock: self.multiplier_mux(id, &clock)?,
                enable: self.multiplier_mux(id, "CE0")?,
                reset: self.multiplier_mux(id, &reset)?,
                reset_kind,
            });
            let mut peers = self
                .device
                .model()
                .children(slice)
                .iter()
                .copied()
                .filter(|&p| self.device.model().resource(p).family == "MULT18")
                .collect::<Vec<_>>();
            peers.sort_by_key(|&p| self.device.model().resource(p).location.col);
            let logical_bit_start = peers
                .iter()
                .position(|&p| p == id)
                .context("missing MULT18 product view")?
                * 36;
            Some(MultiplierOutputUse {clock,logical_bit_start,width:36,rule:"shared 72-bit bank: logical native-order 36-bit product view; output stage under CE0 and REG_OUTPUT_RST (RST0 on MULT18 1/5) as measured on the board (HD01, HD07), RESETMODE ASYNC asynchronous, an enabled GSR clearing it at once (HD07); output SIGNEDP unmodelled, ALU/flag/framing mapping remains D01".into()})
        } else {
            None
        };
        let config = Multiplier18 {
            resource: id,
            a,
            b,
            signed_a,
            signed_b,
            source_a,
            source_b,
        };
        let registered_output = pipeline.output.is_some();
        let disabled_divider = pipeline.both_edges;
        if operands_only {
            self.builder.multiplier18_operands(config, pipeline)?;
        } else {
            self.builder.multiplier18_with_pipeline(config, pipeline)?;
        }
        // With the slice's GSR enabled the output register read 0 at the
        // first clock after configuration (HD07s); with it disabled that is
        // not measured, and the register stays unknown until it captures.
        if registered_output && gsr_enabled && matches!(self.report.startup, Startup::Configured) {
            self.builder.clear_multiplier_output(id)?;
        }
        if disabled_divider && gsr_enabled && matches!(self.report.startup, Startup::Configured) {
            self.builder.clear_multiplier_stages(id)?;
        }
        self.done_resources.insert(id);
        if self.done_resources.insert(slice) {
            self.record_resource(slice, if operands_only {
                "active ALU54 slice: only parallel MULT INPUTA/B operand taps execute; product/ALU/flag/output-bank execution remains unbound"
            } else {"shared DSP slice CIBOUT=ON; one 72-bit OR/FR bank with two logical 36-bit MULT product views; ALU/flag/framing/physical mapping remains D01"});
        }
        self.report.counts.multiplier18 += 1;
        let mut behavioural_assumptions = if stages.is_empty() {
            vec![]
        } else {
            vec![
            "D05: input SIGNEDA/B share their operand's clock/CE/reset; pipeline SIGNEDP shares product controls",
            "D05: reset clears data/sign to zero and precedes CE; effective GSR clears enabled stages asynchronously",
            "D05: parallel mode with standard ENABLED CLK_DIV uses selected rising clock without a highspeed divider",
        ]
        };
        if output_register.is_some() {
            behavioural_assumptions.push("D05: registered output captures on the selected rising clock with standard ENABLED CLK_DIV; physical timing remains unverified");
        }
        self.report.multipliers.push(MultiplierUse {resource:self.device.model().resource(id).key.clone(),slice:self.device.model().resource(slice).key.clone(),
            execution_view:if operands_only {"operands_only"} else {"product_and_operands"},register_candidates:registers,cibout_bypass:bypass,
            output_path_decode:self.device.resource_settings(id).and_then(|r|r.dsp_output_path.clone()),
            implicit_unmodelled_parameters:vec!["MULT_BYPASS=DISABLED","CAS_MATCH_REG=FALSE","HIGHSPEED_CLK=NONE"].into_iter().map(String::from).collect(),
            stages,gsr_enabled,behavioural_assumptions:behavioural_assumptions.into_iter().map(String::from).collect(),output_register,
            rule:(if operands_only {"parallel ROA/ROB taps in active ALU54 slice; only INPUTA/B and their controls execute; no P/SIGNEDP drivers or output-bank captures; D05 remains explicit"} else {"parallel A/B, SOURCEA/B=0 required; ROA/ROB follow INPUTA/B, before PIPELINE/OUTPUT; shared product output stage has restricted controls; D01/D05/D06 remain explicit"}).into()});
        self.record_resource(id,if operands_only {"MULT18 operand-only view for active ALU54; INPUTA/B settings qualified; remaining fields preserved raw, not applied; P/SIGNEDP and ALU execution remain unsupported"} else {"MULT18 input/pipeline/output profile with ROA/ROB operand forwarding; one shared product output view, restricted output controls; unique empty control codes qualified against observed bits; inactive output-clock NONE/CLK3 ambiguity only with bypass; D01/D05/D06 retained"});
        Ok(())
    }
    fn dsp_divider_profile(&mut self, disabled: bool) -> Result<()> {
        ensure!(
            self.dsp_divider_mode.is_none_or(|old| old == disabled),
            "mixed ENABLED/DISABLED DSP register cells are not measured (HD09)"
        );
        self.dsp_divider_mode = Some(disabled);
        Ok(())
    }
    /// An empty selector code still identifies a unique CE/RST when all of
    /// its bits were read. RST3 in HD09p is such a code, not the RST0 fallback.
    fn dsp_control(&self, id: ResourceId, suffix: &str, prefix: &str) -> Result<String> {
        if let Some(f) = self
            .device
            .resource_settings(id)
            .and_then(|r| r.fields.iter().find(|f| f.name.ends_with(suffix)))
        {
            if f.status == "unobserved"
                && !f.observed_bits.is_empty()
                && f.candidates.len() == 1
                && (0..4).any(|i| f.candidates[0] == format!("{prefix}{i}"))
            {
                return Ok(f.candidates[0].clone());
            }
        }
        self.qualified_fallback(id, suffix, &format!("{prefix}0"))
    }
    /// Refuse unmeasured divider mixtures, including shared ALU/MULT fields.
    fn dsp_clock_mode(&self, id: ResourceId, clocks: &[&str]) -> Result<bool> {
        let mut mode = None;
        let slice = self
            .device
            .model()
            .resource(id)
            .parent
            .context("DSP has no slice")?;
        for clock in clocks {
            let value = self.qualified_fallback(id, &format!(".{clock}_DIV"), "ENABLED")?;
            ensure!(
                ["ENABLED", "DISABLED"].contains(&value.as_str()),
                "unsupported DSP {clock}_DIV={value}"
            );
            let disabled = value == "DISABLED";
            ensure!(
                mode.is_none_or(|old| old == disabled),
                "mixed ENABLED/DISABLED DSP register clocks are not measured (HD09)"
            );
            mode = Some(disabled);
            for &peer in self.device.model().children(slice) {
                if peer == id
                    || !["ALU54", "MULT18"]
                        .contains(&self.device.model().resource(peer).family.as_str())
                {
                    continue;
                }
                if self.qualified_fallback(peer, ".MODE", "NONE")? == "NONE" {
                    continue;
                }
                let settings = self
                    .device
                    .resource_settings(peer)
                    .context("missing peer DSP settings")?;
                if settings
                    .fields
                    .iter()
                    .any(|f| f.name.ends_with(&format!(".{clock}_DIV")))
                {
                    let peer_value =
                        self.qualified_fallback(peer, &format!(".{clock}_DIV"), "ENABLED")?;
                    ensure!(
                        peer_value == value,
                        "ALU/MULT18 CLK_DIV disagreement in one DSP slice is not measured (HD09)"
                    );
                } else {
                    ensure!(
                        !disabled,
                        "ALU/MULT18 CLK_DIV disagreement cannot be resolved (HD09)"
                    );
                }
                for f in &settings.fields {
                    if !f.name.contains(".REG_")
                        || !f.name.ends_with("_CLK")
                        || f.candidates.len() != 1
                    {
                        continue;
                    }
                    let selected = &f.candidates[0];
                    if !["CLK0", "CLK1", "CLK2", "CLK3"].contains(&selected.as_str()) {
                        continue;
                    }
                    let peer_mode =
                        self.qualified_fallback(peer, &format!(".{selected}_DIV"), "ENABLED")?;
                    ensure!(peer_mode==value,"mixed ENABLED/DISABLED DSP register clocks in one slice are not measured (HD09)");
                }
            }
        }
        Ok(mode.unwrap_or(false))
    }
    /// The two MULT18 of a DSP slice, in native order.
    fn slice_multipliers(&self, slice: ResourceId) -> Result<[ResourceId; 2]> {
        let mut peers = self
            .device
            .model()
            .children(slice)
            .iter()
            .copied()
            .filter(|&p| self.device.model().resource(p).family == "MULT18")
            .collect::<Vec<_>>();
        peers.sort_by_key(|&p| self.device.model().resource(p).location.col);
        peers
            .try_into()
            .map_err(|_| anyhow::anyhow!("DSP slice without two MULT18"))
    }
    /// Checks an ALU54B's settings against the measured profile (HD02..HD07):
    /// both output registers on one clock, every input and opcode register
    /// bypassed, static patterns.
    fn alu_settings(&self, id: ResourceId) -> Result<AluSettings> {
        ensure!(
            self.qualified_fallback(id, ".MODE", "NONE")? == "ALU54B",
            "ALU54 is not in MODE ALU54B"
        );
        let clocks = ["CLK0", "CLK1", "CLK2", "CLK3"];
        let register_clock = |field: &str| -> Result<Option<String>> {
            let value = self.qualified_fallback(id, &format!(".{field}"), "NONE")?;
            ensure!(
                value == "NONE" || clocks.contains(&value.as_str()),
                "ALU54B {field}={value}"
            );
            Ok((value != "NONE").then_some(value))
        };
        let outputs = [
            register_clock("REG_OUTPUT0_CLK")?,
            register_clock("REG_OUTPUT1_CLK")?,
        ];
        let flag_clock = register_clock("REG_FLAG_CLK")?;
        // The opcode registers (HD08): IN holds OP[6:0], OP0 OP7 and OP1
        // OP[10:8], each in a first and a second stage. OP1 has no CE or RST
        // field: it takes OP0's (o3, o6). IN_1 alone gave values no register
        // model explains (o4, 6% of the clocks), so it is refused.
        // An input register's clock: its select bits may be shared (C with
        // the MULT18's REG_INPUTC_CLK, OP1 with OP0) while its bypass bit is
        // its own. Where the bypass is set beside a clock that the sharer
        // selected, the decoder sees both values and the register is off
        // (HD08 c4, o3; HD03).
        let input_clock = |field: &str| -> Result<Option<String>> {
            let values = self.composed(id, &format!(".{field}"))?;
            Ok(match values.as_slice() {
                [one] if one == "NONE" => None,
                [one] if clocks.contains(&one.as_str()) => Some(one.clone()),
                [a, b] if (a == "NONE") != (b == "NONE") => None,
                _ => bail!("ALU54B {field}: {values:?}"),
            })
        };
        // A CE or RST selection. CE3 and RST3 have no bits: with every bit
        // clear the field is unobserved, its one candidate the line (HD08 o3).
        let line = |field: &str, fallback: &str| -> Result<String> {
            let unobserved = self.device.resource_settings(id).and_then(|r| {
                r.fields
                    .iter()
                    .find(|f| f.name.ends_with(&format!(".{field}")))
                    .filter(|f| f.status == "unobserved" && f.candidates.len() == 1)
                    .map(|f| f.candidates[0].clone())
            });
            let value = match unobserved {
                Some(v) => v,
                None => self.qualified_fallback(id, &format!(".{field}"), fallback)?,
            };
            ensure!(
                value.len() == 3 || value.len() == 4,
                "ALU54B {field}={value}"
            );
            Ok(value)
        };
        let mut opcode: [[Option<[String; 3]>; 2]; 3] = Default::default();
        for (stages, (group, controls)) in
            opcode
                .iter_mut()
                .zip([("IN", "IN"), ("OP0", "OP0"), ("OP1", "OP0")])
        {
            for (stage, slot) in stages.iter_mut().enumerate() {
                let Some(clock) = input_clock(&format!("REG_OPCODE{group}_{stage}_CLK"))? else {
                    continue;
                };
                ensure!(
                    !(group == "IN" && stage == 1),
                    "ALU54B REG_OPCODEIN_1_CLK={clock} is not understood: on the board (HD08 o4) its values follow no register model in 6% of the clocks"
                );
                *slot = Some([
                    clock,
                    line(&format!("REG_OPCODE{controls}_{stage}_CE"), "CE0")?,
                    line(&format!("REG_OPCODE{controls}_{stage}_RST"), "RST0")?,
                ]);
            }
        }
        // The C registers: on where the ALU's field and the MULT18's (the same
        // bits) both ask for a clock.
        let c_registers = [
            input_clock("REG_INPUTC0_CLK")?,
            input_clock("REG_INPUTC1_CLK")?,
        ];
        // The reset line of R[17:0] and R[44:27]: REG_OUTPUT0_RST and
        // REG_OUTPUT1_RST, the bits the first MULT18's REG_OUTPUT_RST sets
        // together (HD07 ec, ed). Different lines for the two are not measured.
        let resets = ["REG_OUTPUT0_RST", "REG_OUTPUT1_RST"]
            .map(|field| self.qualified_fallback(id, &format!(".{field}"), "RST0"));
        let [first_reset, other_reset] = resets;
        let (first_reset, other_reset) = (first_reset?, other_reset?);
        ensure!(
            ["RST0", "RST1", "RST2", "RST3"].contains(&first_reset.as_str()),
            "ALU54B REG_OUTPUT0_RST={first_reset}"
        );
        ensure!(
            first_reset == other_reset,
            "ALU54B REG_OUTPUT0_RST={first_reset} and REG_OUTPUT1_RST={other_reset} differ: not measured"
        );
        // GSR: the slice's two GSR bits, shared with both MULT18. Enabled, it
        // clears R (HD07); disabled, a user GSR does nothing (HD06).
        let gsr = match self.qualified_fallback(id, ".GSR", "ENABLED")?.as_str() {
            "ENABLED" => true,
            "DISABLED" => false,
            other => bail!("unsupported ALU54B GSR {other}"),
        };
        for (field, fallback) in [
            ("MCPAT_SOURCE", "STATIC"),
            ("MASKPAT_SOURCE", "STATIC"),
            ("LEGACY", "DISABLED"),
            ("FORCE_ZERO_BARREL_SHIFT", "DISABLED"),
        ] {
            let value = self.qualified_fallback(id, &format!(".{field}"), fallback)?;
            ensure!(
                value == fallback,
                "ALU54B {field}={value} is not measured (only {fallback})"
            );
        }
        let input_clocks: Vec<&String> = opcode
            .iter()
            .flatten()
            .flatten()
            .map(|s| &s[0])
            .chain(c_registers.iter().flatten())
            .collect();
        let clocks: Vec<_> = outputs
            .iter()
            .chain([&flag_clock])
            .flatten()
            .chain(input_clocks)
            .map(String::as_str)
            .collect();
        let both_edges = self.dsp_clock_mode(id, &clocks)?;
        // The patterns are 56-bit words in Trellis with 54 bits on the chip.
        let word = |name: &str| -> Result<u64> {
            let field = self
                .device
                .resource_settings(id)
                .and_then(|r| r.fields.iter().find(|f| f.name.ends_with(name)))
                .with_context(|| format!("ALU54B has no {name}"))?;
            ensure!(field.word_bits_lsb0.len() >= 54, "ALU54B {name} is short");
            let mut value = 0_u64;
            for (k, bit) in field.word_bits_lsb0.iter().take(54).enumerate() {
                value |=
                    (bit.with_context(|| format!("ALU54B {name} bit not decoded"))? as u64) << k;
            }
            Ok(value)
        };
        let rndpat = word(".RNDPAT")?;
        // The flags never followed R on the board: they are what a zero word
        // gives against the patterns (HD04 sa, sb): EQZ and EQZM high, EQOM
        // low, EQPAT when every unmasked bit of MCPAT is 0, EQPATB when every
        // unmasked bit of it is 1; OVER, UNDER and OVERUNDER low.
        let (mcpat, unmasked) = (word(".MCPAT")?, !word(".MASKPAT")? & ((1 << 54) - 1));
        let flags = [
            true,
            true,
            false,
            mcpat & unmasked == 0,
            !mcpat & unmasked == 0,
            false,
            false,
            false,
        ]
        .map(Logic::from_bool);
        // The output bank: each group of R is unregistered with its MULT18's
        // CIBOUT_BYP, else on its register clock, else without a clock.
        let slice = self
            .device
            .model()
            .resource(id)
            .parent
            .context("ALU54 has no slice")?;
        let [first, second] = self.slice_multipliers(slice)?;
        let bypass = |m: ResourceId| -> Result<bool> {
            let decoded = self
                .device
                .resource_settings(m)
                .and_then(|r| r.dsp_output_path.as_ref())
                .context("missing DSP output path decode")?;
            // Beside an ALU the MULT18's own output clock says nothing: the
            // two selector bits alone decide.
            let on = decoded.selector_bits.iter().filter(|b| b.2).count();
            match decoded.value.as_deref() {
                Some("ON") => Ok(true),
                Some("OFF") => Ok(false),
                _ if !decoded.selector_bits.is_empty() && on == 0 => Ok(false),
                _ => bail!(
                    "unresolved CIBOUT_BYP beside an ALU54B: {} (raw {})",
                    decoded.status,
                    decoded.raw_status
                ),
            }
        };
        let bypasses = [bypass(first)?, bypass(second)?];
        // RESETMODE of the ALU and of each MULT18 (HD06): the first half is
        // asynchronous with the ALU's or the first MULT18's, the second half
        // with the second MULT18's.
        let asynchronous = |id: ResourceId| -> Result<bool> {
            match self.qualified_fallback(id, ".RESETMODE", "SYNC")?.as_str() {
                "SYNC" => Ok(false),
                "ASYNC" => Ok(true),
                other => bail!("unsupported DSP RESETMODE {other}"),
            }
        };
        let halves = [
            asynchronous(id)? || asynchronous(first)?,
            asynchronous(second)?,
        ];
        let opcode_async = asynchronous(id)?;
        let mut c_async = [false; 2];
        for (n, m) in [first, second].into_iter().enumerate() {
            if c_registers[n].is_some() {
                c_async[n] = asynchronous(id)?;
                ensure!(
                    asynchronous(m)? == c_async[n],
                    "ALU54B C register with RESETMODE differing between the ALU and its MULT18 is not measured"
                );
            }
        }
        let (mut unregistered, mut stuck, mut clock, mut async_reset) = (0, 0, None, 0);
        for (bits, second_clock, second_bypass) in ALU_GROUPS {
            if halves[usize::from(second_bypass)] {
                async_reset |= bits;
            }
            if bypasses[usize::from(second_bypass)] {
                unregistered |= bits;
            } else if let Some(c) = &outputs[usize::from(second_clock)] {
                ensure!(
                    clock.as_ref().is_none_or(|old| old == c),
                    "ALU54B output registers on two clocks are not measured"
                );
                clock = Some(c.clone());
            } else {
                stuck |= bits;
            }
        }
        Ok(AluSettings {
            both_edges,
            clock,
            rndpat,
            unregistered,
            stuck,
            async_reset,
            first_reset,
            gsr,
            opcode,
            opcode_async,
            c_registers,
            c_async,
            flags,
            flags_measured: !(bypasses[1] && flag_clock.is_some()),
            flag_clock,
        })
    }
    /// A cascade input (CIN, SIGNEDCIN): the pin of the ALU before this one,
    /// or zero where the chip has none there (the first ALU of a DSP row
    /// read zero on the board, HD02).
    fn cascade_input(&mut self, wire: &Wire) -> Result<Input> {
        let mut driven = false;
        for item in self
            .index
            .incoming(self.device.model(), self.index.id(wire))?
        {
            for connection in self.device.model().connections_to(item.tile, &item.raw) {
                let c = connection?;
                if let Endpoint::Wire(ref source) = c.source {
                    driven |= c.definition.kind == ArcKind::Fixed
                        && self.producers.contains_key(&self.index.id(source));
                }
            }
        }
        if driven {
            self.input(wire)
        } else {
            Ok(Input::Constant(Logic::Zero))
        }
    }
    /// ALU54B as measured on the board (HD02, HD03): see kernel/alu.rs.
    pub(super) fn execute_alu(&mut self, id: ResourceId, port: &str) -> Result<()> {
        let flag = ALU_FLAGS.contains(&port);
        ensure!(
            flag || port == "SIGNEDR"
                || port
                    .strip_prefix('R')
                    .is_some_and(|k| k.parse::<usize>().is_ok_and(|k| k < 54)),
            "unsupported ALU54 output {port} (CO is not measured)"
        );
        if self.done_resources.contains(&id) {
            return Ok(());
        }
        let key = self.device.model().resource(id).key.clone();
        if self.qualified_fallback(id, ".MODE", "NONE")? == "NONE" {
            // The ALU after an unused one read zero through the cascade
            // (HD03 a0): R and SIGNEDR of an inactive ALU are low.
            ensure!(!flag, "flag {port} of an inactive ALU54: {key}");
            for name in (0..54)
                .map(|k| format!("R{k}"))
                .chain(["SIGNEDR".to_string()])
            {
                let signal = self.signal(&self.resource_port(id, &name)?)?;
                self.builder.static_driver(
                    signal,
                    &format!("alu54-inactive:{key}:{name}"),
                    crate::kernel::Strength::Strong,
                    Logic::Zero,
                )?;
            }
            self.done_resources.insert(id);
            self.record_resource(id, "inactive ALU54: R and SIGNEDR read low through the cascade, as the ALU after an unused one saw on the board (HD03 a0)");
            return Ok(());
        }
        let settings = self
            .alu_settings(id)
            .with_context(|| format!("ALU54B {key}"))?;
        if settings.clock.is_some()
            || settings.flag_clock.is_some()
            || settings.opcode.iter().flatten().any(Option::is_some)
            || settings.c_registers.iter().any(Option::is_some)
        {
            self.dsp_divider_profile(settings.both_edges)?;
        }

        ensure!(
            !flag || settings.flags_measured,
            "ALU54B {key}: registered flags beside CIBOUT_BYP of the second MULT18 are not measured"
        );
        let slice = self
            .device
            .model()
            .resource(id)
            .parent
            .context("ALU54 has no slice")?;
        let [first, second] = self.slice_multipliers(slice)?;
        self.done_resources.insert(id);
        let bus = |planner: &mut Self,
                   owner: ResourceId,
                   name: &str,
                   from: usize,
                   n: usize|
         -> Result<Vec<Input>> {
            (from..from + n)
                .map(|k| planner.input(&planner.resource_port(owner, &format!("{name}{k}"))?))
                .collect()
        };
        let array = |v: Vec<Input>| -> [Input; 54] { v.try_into().unwrap() };
        // A = {C input of the first MULT18, ROB, ROA}; B the same of the second.
        let a = array([bus(self, id, "A", 0, 36)?, bus(self, first, "C", 0, 18)?].concat());
        let b = array([bus(self, id, "B", 0, 36)?, bus(self, second, "C", 0, 18)?].concat());
        // C = {C[53:45], second MULT18's C, C[26:18], first MULT18's C}: the
        // ALU's own pins C[17:0] and C[44:27] do nothing.
        let c = array(
            [
                bus(self, first, "C", 0, 18)?,
                bus(self, id, "C", 18, 9)?,
                bus(self, second, "C", 0, 18)?,
                bus(self, id, "C", 45, 9)?,
            ]
            .concat(),
        );
        let config = Alu54 {
            both_edges: settings.both_edges,
            resource: id,
            op: bus(self, id, "OP", 0, 11)?.try_into().unwrap(),
            ma: bus(self, id, "MA", 0, 36)?.try_into().unwrap(),
            mb: bus(self, id, "MB", 0, 36)?.try_into().unwrap(),
            signed_ma: self.input(&self.resource_port(id, "SIGNEDIA")?)?,
            signed_mb: self.input(&self.resource_port(id, "SIGNEDIB")?)?,
            a,
            b,
            c,
            cin: array(
                (0..54)
                    .map(|k| self.cascade_input(&self.resource_port(id, &format!("CIN{k}"))?))
                    .collect::<Result<Vec<_>>>()?,
            ),
            rndpat: settings.rndpat,
            clock: match &settings.clock {
                Some(clock) => self.input(&self.resource_port(id, clock)?)?,
                None => Input::Constant(Logic::Zero),
            },
            // The registers take the block's CE0 line, which is the ALU's own
            // CE0 pin as routed. (nextpnr puts one net on each of a block's
            // four CE lines, the first slice's first: a second ALU whose
            // netlist CE0 differs ends up on the first's, HD03 b2, HD04 rb.)
            enable: self.input(&self.resource_port(id, "CE0")?)?,
            resets: [
                self.input(&self.resource_port(id, &settings.first_reset)?)?,
                self.input(&self.resource_port(id, "RST0")?)?,
            ],
            global_reset: if settings.gsr {
                Some(self.system_input(SystemPort::Gsr)?)
            } else {
                None
            },
            opcode: {
                let mut stages: [[Option<AluRegister>; 2]; 3] = Default::default();
                for (g, group) in settings.opcode.iter().enumerate() {
                    for (s, ports) in group.iter().enumerate() {
                        if let Some([clock, enable, reset]) = ports {
                            stages[g][s] = Some(AluRegister {
                                clock: self.input(&self.resource_port(id, clock)?)?,
                                enable: self.input(&self.resource_port(id, enable)?)?,
                                reset: self.input(&self.resource_port(id, reset)?)?,
                                asynchronous: settings.opcode_async,
                            });
                        }
                    }
                }
                stages
            },
            // The C registers take CE0 and RST0 (HD08 c1..c3).
            c_registers: {
                let mut registers: [Option<AluRegister>; 2] = Default::default();
                for (n, clock) in settings.c_registers.iter().enumerate() {
                    if let Some(clock) = clock {
                        registers[n] = Some(AluRegister {
                            clock: self.input(&self.resource_port(id, clock)?)?,
                            enable: self.input(&self.resource_port(id, "CE0")?)?,
                            reset: self.input(&self.resource_port(id, "RST0")?)?,
                            asynchronous: settings.c_async[n],
                        });
                    }
                }
                registers
            },
            async_reset: settings.async_reset,
            unregistered: settings.unregistered,
            stuck: settings.stuck,
            flags: settings.flags,
            flag_clock: match &settings.flag_clock {
                Some(clock) => Some(self.input(&self.resource_port(id, clock)?)?),
                None => None,
            },
        };
        self.builder.alu54(config)?;
        self.report.counts.alu54 += 1;
        self.record_resource(id, "ALU54B as measured on the board (HD02, HD03, HD04): X/Y/Z by OP[6:0], sum or bitwise by OP[10:7]; A, B and C take the MULT18 C inputs; CIN is the R before it, its >> 18 filled with CIN[53] & SIGNEDIB; registers enabled by the block's CE0 line (the ALU's CE0 pin as routed); each group of R unregistered with its MULT18's CIBOUT_BYP, else on REG_OUTPUT0/1_CLK, else 0; reset over the enable, R[17:0] and R[44:27] by the line REG_OUTPUT0/1_RST selects and R[26:18], R[53:45] by RST0 (HD07), synchronous or asynchronous per half by RESETMODE of the ALU and its MULT18 (HD06, HD07); GSR clears R at once where the slice has it enabled and does nothing where disabled (HD07, HD06); flags are those of a zero word, constant, their register starting at 0 (its reset is not measured); SIGNEDR unknown");
        Ok(())
    }
    fn multiplier_mux(&mut self, id: ResourceId, port: &str) -> Result<Input> {
        let choice = self.qualified_fallback(id, &format!(".{port}MUX"), port)?;
        ensure!(
            choice == port || choice == "INV",
            "unsupported MULT18 {port}MUX"
        );
        let wire = self.resource_port(id, port)?;
        let input = self.input(&wire)?;
        Ok(if choice == "INV" {
            match input {
                Input::Constant(value) => Input::Constant(value.invert()),
                Input::Signal(s) => Input::Inverted(s),
                Input::Inverted(s) => Input::Signal(s),
            }
        } else {
            input
        })
    }
    /// The active ALU54B of the DSP slice that owns `wire`, a pin of one of
    /// its MULT18 (or the same pin under its MULT9 or ALU24 name).
    fn active_alu_of_pin(&self, wire: &Wire) -> Result<Option<ResourceId>> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/dsp::active_alu_of_pin");
        let wide = ["_MULT9", "_ALU24"]
            .iter()
            .find_map(|suffix| {
                let prefix = wire.name.strip_suffix(suffix)?;
                Some(Wire {
                    location: wire.location,
                    name: format!(
                        "{prefix}{}",
                        if *suffix == "_MULT9" {
                            "_MULT18"
                        } else {
                            "_ALU54"
                        }
                    ),
                })
            })
            .unwrap_or_else(|| wire.clone());
        let Some([Producer::Resource(id, _)]) =
            self.producers.get(&self.index.id(&wide)).map(Vec::as_slice)
        else {
            return Ok(None);
        };
        let resource = self.device.model().resource(*id);
        if !["MULT18", "ALU54"].contains(&resource.family.as_str()) {
            return Ok(None);
        }
        let slice = resource.parent.context("DSP primitive has no slice")?;
        for &peer in self.device.model().children(slice) {
            if self.device.model().resource(peer).family == "ALU54"
                && self.qualified_fallback(peer, ".MODE", "NONE")? == "ALU54B"
            {
                return Ok(Some(peer));
            }
        }
        Ok(None)
    }
    pub(super) fn excluded_dsp_source(
        &self,
        source: &Option<Wire>,
        sink: &Wire,
    ) -> Result<Option<&'static str>> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/dsp::excluded_dsp_source");
        let Some(source) = source else {
            return Ok(None);
        };
        // An ALU24 pin beside the pin of an active ALU54B on the same wire.
        if source.name.ends_with("_ALU24") {
            for item in self
                .index
                .incoming(self.device.model(), self.index.id(sink))?
            {
                for connection in self.device.model().connections_to(item.tile, &item.raw) {
                    let c = connection?;
                    let Endpoint::Wire(ref other) = c.source else {
                        continue;
                    };
                    if c.definition.kind == ArcKind::Fixed
                        && other.name.ends_with("_ALU54")
                        && self.active_alu_of_pin(other)?.is_some()
                    {
                        return Ok(Some(
                            "active ALU54B slice: exclude the ALU24 description of the same output",
                        ));
                    }
                }
            }
        }
        // A slice with an active ALU54B (HD02, HD03): its CIB outputs carry R
        // and the flags, and its products go to MA/MB only.
        if self.active_alu_of_pin(source)?.is_some() {
            let name = &source.name;
            if name.ends_with("_MULT9") || name.ends_with("_ALU24") {
                return Ok(Some(
                    "active ALU54B slice: exclude the MULT9/ALU24 description of the same pin",
                ));
            }
            if name.ends_with("_MULT18") && name.starts_with("JP") && !sink.name.ends_with("_ALU54")
            {
                return Ok(Some("active ALU54B slice: the CIB outputs carry the ALU's R and flags, not the product"));
            }
        }
        if let Some([Producer::Resource(id, _)]) = self
            .producers
            .get(&self.index.id(source))
            .map(Vec::as_slice)
        {
            if self.device.model().resource(*id).family == "ALU54"
                && self.inactive_alu_multiplier_profile(*id)?
            {
                return Ok(Some("parallel MULT18 CIB profile: exclude inactive ALU54/ALU24 flag/output description"));
            }
        }
        if let Some(bit) = source
            .name
            .strip_prefix("JP")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&b| b < 54)
        {
            let registered = Wire {
                location: source.location,
                name: format!("JR{bit}"),
            };
            let mut connected = false;
            for item in self
                .index
                .incoming(self.device.model(), self.index.id(source))?
            {
                for connection in self.device.model().connections_to(item.tile, &item.raw) {
                    let c = connection?;
                    connected |= c.definition.kind == ArcKind::Fixed
                        && c.source == Endpoint::Wire(registered.clone())
                        && c.sink == Endpoint::Wire(source.clone());
                }
            }
            if !connected {
                return Ok(None);
            }
            let mut owners = HashSet::new();
            for item in self
                .index
                .incoming(self.device.model(), self.index.id(&registered))?
            {
                for connection in self.device.model().connections_to(item.tile, &item.raw) {
                    let c = connection?;
                    if c.definition.kind != ArcKind::Fixed {
                        continue;
                    }
                    let Endpoint::Wire(ref wide) = c.source else {
                        continue;
                    };
                    if !wide.name.ends_with("_ALU54") {
                        continue;
                    }
                    if let Some([Producer::Resource(id, _)]) =
                        self.producers.get(&self.index.id(wide)).map(Vec::as_slice)
                    {
                        owners.insert(*id);
                    }
                }
            }
            if owners.len() == 1 {
                let alu = *owners.iter().next().unwrap();
                if self.inactive_alu_multiplier_profile(alu)? {
                    return Ok(Some("parallel MULT18 CIB profile: exclude alternate JP/JR ALU54/ALU24 description; product execution owns the selected bypass or shared output view"));
                }
            }
        }
        let Some(prefix) = source.name.strip_suffix("_MULT9") else {
            return Ok(None);
        };
        let wide = Wire {
            location: source.location,
            name: format!("{prefix}_MULT18"),
        };
        let Some(producers) = self.producers.get(&self.index.id(&wide)) else {
            return Ok(None);
        };
        let [Producer::Resource(id, _)] = producers.as_slice() else {
            return Ok(None);
        };
        if self.device.model().resource(*id).family == "MULT18"
            && self.multiplier_cib_output(*id)?
        {
            return Ok(Some(
                "parallel MULT18 CIB profile: exclude alternative MULT9 fixed output description",
            ));
        }
        Ok(None)
    }
    fn multiplier_cib_output(&self, id: ResourceId) -> Result<bool> {
        if self.qualified_fallback(id, ".MODE", "NONE")? != "MULT18X18D" {
            return Ok(false);
        }
        match self.multiplier_output_path(id)?.as_str() {
            "ON" => Ok(true),
            "OFF" => {
                let clocks = self.composed(id, ".REG_OUTPUT_CLK")?;
                Ok(clocks.len() == 1
                    && ["CLK0", "CLK1", "CLK2", "CLK3"].contains(&clocks[0].as_str()))
            }
            _ => Ok(false),
        }
    }
    fn multiplier_output_path(&self, id: ResourceId) -> Result<String> {
        let decoded = self
            .device
            .resource_settings(id)
            .and_then(|r| r.dsp_output_path.as_ref())
            .context("missing DSP output path decode")?;
        decoded.value.clone().with_context(|| {
            format!(
                "unresolved DSP output path: {} (raw {})",
                decoded.status, decoded.raw_status
            )
        })
    }
    fn inactive_alu_multiplier_profile(&self, alu: ResourceId) -> Result<bool> {
        let slice = self
            .device
            .model()
            .resource(alu)
            .parent
            .context("ALU has no slice")?;
        if self.qualified_fallback(alu, ".MODE", "NONE")? != "NONE"
            || self.qualified_fallback(slice, ".CIBOUT", "OFF")? != "ON"
        {
            return Ok(false);
        }
        for &peer in self.device.model().children(slice) {
            if self.device.model().resource(peer).family == "MULT18"
                && self.multiplier_cib_output(peer)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
