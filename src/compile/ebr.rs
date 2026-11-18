//! Composed EBR fields span MIB/spine tiles; no single-tile mode guessing.
use super::*;
use crate::kernel::{Ebr, EbrMode, EbrPort, WriteMode};

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct EbrUse {
    pub resource: String,
    pub mode: EbrMode,
    pub mode_decode: crate::decode::EbrModeResult,
    pub widths: [usize; 2],
    pub output_registers: [bool; 2],
    pub write_modes: [WriteMode; 2],
    pub raw_cs_decode: [u8; 2],
    pub reset_mode: String,
    pub asynchronous_release: String,
    pub gsr_enabled: bool,
    pub initialization: String,
    pub rule: String,
    /// What a 36-bit PDP read relies on beyond the decoded fields (HB01).
    pub assumptions: Vec<String>,
}
/// The order of a 36-bit PDP read on DOA/DOB (kernel::PDP36_LOW_PATH).
const PDP36_ORDER: &str = "36-bit PDP read: bits 17:0 (written through DIA) on DOA, 35:18 on DOB, as the yosys PDP wrapper reads them; measured on the ULX3S (HB01, B01); nextpnr's public PDPW16KD packing assumes the reverse";
/// The controls of both read paths (kernel::PDP36_A_PATH_OWN_CONTROLS).
const PDP36_CONTROLS: &str = "36-bit PDP read: the read port's CLKB, CEB/CSB, OCEB and RSTB act on both read paths, RSTA/OCEA/CLKA on neither (measured, HB01); REGMODE_A must equal REGMODE_B (not measured otherwise)";
impl Planner {
    pub(super) fn ebr_mode_name(&self, id: ResourceId) -> Result<String> {
        let mode = self
            .device
            .resource_settings(id)
            .and_then(|r| r.ebr_mode.as_ref())
            .context("missing composed EBR mode")?;
        mode.value.clone().with_context(|| {
            format!(
                "unresolved effective EBR MODE: {} {:?}",
                mode.status, mode.candidates
            )
        })
    }
    fn ebr_mux(&mut self, id: ResourceId, port: &str) -> Result<Input> {
        let choice = self.composed_one(id, &format!(".{port}MUX"))?;
        ensure!(
            choice == port || choice == "INV",
            "unsupported EBR {port}MUX: {choice}"
        );
        let wire = self.resource_port(id, port)?;
        let input = self.input(&wire)?;
        Ok(if choice == "INV" {
            Input::Inverted(self.signal(&wire)?)
        } else {
            input
        })
    }
    fn ebr_decode(&self, id: ResourceId, side: char) -> Result<u8> {
        let suffix = format!(".CSDECODE_{side}");
        let field = self
            .device
            .resource_settings(id)
            .context("missing EBR settings")?
            .fields
            .iter()
            .find(|f| f.name.ends_with(&suffix))
            .context("missing EBR CSDECODE")?;
        ensure!(
            field.status == "resolved" && field.word_bits_lsb0.len() == 3,
            "unresolved EBR CSDECODE"
        );
        field
            .word_bits_lsb0
            .iter()
            .enumerate()
            .try_fold(0, |value, (bit, v)| {
                Ok(value | ((v.context("unknown CSDECODE bit")? as u8) << (2 - bit)))
            })
    }
    pub(super) fn execute_ebr(&mut self, id: ResourceId, output: Option<&str>) -> Result<()> {
        let mode = self.ebr_mode_name(id)?;
        let mode = match mode.as_str() {
            "DP16KD" => EbrMode::Dp,
            "PDPW16KD" => EbrMode::Pdp,
            _ => bail!("reachable inactive/unsupported EBR mode {mode}"),
        };
        let widths = if mode == EbrMode::Dp {
            [
                self.composed_one(id, ".DP16KD.DATA_WIDTH_A")?.parse()?,
                self.composed_one(id, ".DP16KD.DATA_WIDTH_B")?.parse()?,
            ]
        } else {
            [
                36,
                self.composed_one(id, ".PDPW16KD.DATA_WIDTH_R")?.parse()?,
            ]
        };
        // A PDP read of 36 bits drives both read paths, DOA and DOB (HB01).
        let wide_read = mode == EbrMode::Pdp && widths[1] == 36;
        ensure!(
            ([1, 2, 4, 9, 18].contains(&widths[1]) || wide_read)
                && (mode == EbrMode::Pdp || [1, 2, 4, 9, 18].contains(&widths[0])),
            "unsupported EBR width {widths:?}"
        );
        if let Some(output) = output {
            let (side, bit) = if let Some(bit) = output.strip_prefix("DOA") {
                (0, bit)
            } else if let Some(bit) = output.strip_prefix("DOB") {
                (1, bit)
            } else {
                bail!("unknown EBR output {output}")
            };
            let bit: usize = bit.parse()?;
            let pins = match (mode, side) {
                (EbrMode::Dp, _) => widths[side],
                (EbrMode::Pdp, 0) => 18 * usize::from(wide_read),
                (EbrMode::Pdp, _) => widths[1].min(18),
            };
            ensure!(
                bit < pins,
                "EBR output {output} outside validated data view"
            );
        }
        if self.done_resources.contains(&id) {
            return Ok(());
        }
        let reset_mode = self.composed_one(id, ".RESETMODE")?;
        let reset_kind = match reset_mode.as_str() {
            "SYNC" => ResetKind::Synchronous,
            "ASYNC" => ResetKind::Asynchronous,
            _ => bail!("unsupported EBR reset mode"),
        };
        let release = self.composed_one(id, ".ASYNC_RESET_RELEASE")?;
        ensure!(
            ["SYNC", "ASYNC"].contains(&release.as_str()),
            "unsupported EBR reset release"
        );
        let gsr = self.composed_one(id, ".GSR")?;
        ensure!(
            ["ENABLED", "DISABLED"].contains(&gsr.as_str()),
            "unsupported EBR GSR"
        );
        let mut ports = Vec::new();
        for (side, letter) in ['A', 'B'].into_iter().enumerate() {
            let writing_pdp = mode == EbrMode::Pdp && side == 0;
            let write_mode = if mode == EbrMode::Dp {
                match self
                    .composed_one(id, &format!(".DP16KD.WRITEMODE_{letter}"))?
                    .as_str()
                {
                    "NORMAL" => WriteMode::Normal,
                    "WRITETHROUGH" => WriteMode::WriteThrough,
                    "READBEFOREWRITE" => WriteMode::ReadBeforeWrite,
                    _ => bail!("unsupported EBR write mode"),
                }
            } else {
                WriteMode::Normal
            };
            let reg = self.composed_one(id, &format!(".REGMODE_{letter}"))?;
            ensure!(
                ["NOREG", "OUTREG"].contains(&reg.as_str()),
                "unsupported EBR output register mode"
            );
            // Both writers set REGMODE_A = REGMODE_B for a PDP; a 36-bit read
            // with two different modes is not measured.
            ensure!(
                !(wide_read && side == 0) || reg == self.composed_one(id, ".REGMODE_B")?,
                "36-bit PDP read with REGMODE_A != REGMODE_B is not measured"
            );
            // Path A of a 36-bit read keeps its output register and enable.
            let reads = !writing_pdp || wide_read;
            let mut address = [Input::Constant(Logic::Zero); 14];
            let shift = match widths[side] {
                1 => 0,
                2 => 1,
                4 => 2,
                9 => 3,
                18 => 4,
                36 => 5,
                _ => unreachable!(),
            };
            // Group enables share the low address pins of a wide write port;
            // a 36-bit read port has none, and ADB4..0 address nothing.
            let enables = if mode == EbrMode::Pdp && side == 1 && widths[1] == 36 {
                0
            } else if widths[side] >= 18 {
                widths[side] / 9
            } else {
                0
            };
            for (bit, input) in address.iter_mut().enumerate() {
                if bit < shift && bit >= enables {
                    continue;
                }
                let name = format!("AD{letter}{bit}");
                *input = if (letter == 'A' && bit < 4) || (letter == 'B' && bit < 2) {
                    self.ebr_mux(id, &name)?
                } else {
                    self.input(&self.resource_port(id, &name)?)?
                };
            }
            let mut data = [Input::Constant(Logic::Zero); 18];
            let count = if mode == EbrMode::Pdp {
                18
            } else {
                widths[side]
            };
            for (bit, input) in data.iter_mut().enumerate().take(count) {
                *input = self.input(&self.resource_port(id, &format!("DI{letter}{bit}"))?)?;
            }
            let mut select = [Input::Constant(Logic::Zero); 3];
            for (bit, input) in select.iter_mut().enumerate() {
                *input = self.input(&self.resource_port(id, &format!("CS{letter}{bit}"))?)?;
            }
            ports.push(EbrPort {
                width: widths[side],
                output_register: reads && reg == "OUTREG",
                write_mode,
                clock: self.ebr_mux(id, &format!("CLK{letter}"))?,
                enable: self.ebr_mux(id, &format!("CE{letter}"))?,
                output_enable: if !reads {
                    Input::Constant(Logic::One)
                } else {
                    self.ebr_mux(id, &format!("OCE{letter}"))?
                },
                write_enable: if mode == EbrMode::Pdp {
                    Input::Constant(Logic::Zero)
                } else {
                    self.ebr_mux(id, &format!("WE{letter}"))?
                },
                select,
                decode: self.ebr_decode(id, letter)?,
                reset: self.ebr_mux(id, &format!("RST{letter}"))?,
                address,
                data,
            });
        }
        let ports: [EbrPort; 2] = ports.try_into().ok().unwrap();
        let usage = EbrUse {
            resource: self.device.model().resource(id).key.clone(),
            mode,
            mode_decode: self.device.resource_settings(id).unwrap().ebr_mode.as_ref().unwrap().clone(),
            widths,
            output_registers: ports.each_ref().map(|p| p.output_register),
            write_modes: ports.each_ref().map(|p| p.write_mode),
            raw_cs_decode: ports.each_ref().map(|p| p.decode),
            reset_mode,
            asynchronous_release: release.clone(),
            gsr_enabled: gsr == "ENABLED",
            initialization: self.device.resource_settings(id)
                .and_then(|s| s.memory.as_ref()).map(|m| m.status.clone())
                .unwrap_or_else(|| "missing binding; live array X".into()),
            rule: "one INIT9 array; synchronous independent clocks; pre-write capture; HB09 resolves same-clock equal-width/mode full-word DP accesses; HB09P resolves same-clock full-word PDP36/NOREG/NORMAL reads with measured controls to new data; other same-row cross-port collisions and uncertain controls use conservative X; see EBR_EXECUTION.md".into(),
            assumptions: if wide_read {
                vec![PDP36_ORDER.into(), PDP36_CONTROLS.into()]
            } else {
                Vec::new()
            },
        };
        // Under the configured startup every read latch and output register
        // reads 0 until its port reads: on the board for every width, both
        // ports, NOREG and OUTREG, GSR on and off, either RESETMODE (HB02).
        if matches!(self.report.startup, Startup::Configured) {
            self.builder.clear_ebr_pipeline(id)?;
        }
        self.builder.ebr(Ebr {
            resource: id,
            mode,
            ports,
            reset_kind,
            asynchronous_release: release == "ASYNC",
            global_reset: if gsr == "ENABLED" {
                Some(self.system_input(SystemPort::Gsr)?)
            } else {
                None
            },
            write_inhibit: self.system_input(SystemPort::Gwdis)?,
        })?;
        self.done_resources.insert(id);
        self.report.counts.ebr_blocks += 1;
        self.report.counts.ebr_read_ports += if mode == EbrMode::Dp { 2 } else { 1 };
        self.report.ebr.push(usage);
        self.record_resource(id,"composed multi-tile EBR mode/width/muxes; CS word reversal applied once; live shared storage; write-capable EBRs rooted in all-plc scope");
        Ok(())
    }
}
