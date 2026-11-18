//! HY04 DTR: a board-profile code, not a temperature calculation from CRAM.
use super::*;
use crate::kernel::DtrConfig;

/// Initial and final full DTR byte measured on this ULX3S at room temperature.
pub const ULX3S_DTR_CODE: u8 = 0x9E;

impl Planner {
    pub(super) fn execute_dtr(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let mode = self.qualified_fallback(id, ".MODE", "NONE")?;
        let outputs = (0..8)
            .map(|bit| self.signal(&self.resource_port(id, &format!("DTROUT{bit}"))?))
            .collect::<Result<Vec<_>>>()?;
        match mode.as_str() {
            "DTR" => {
                let start = self.input(&self.resource_port(id, "STARTPULSE")?)?;
                self.builder.dtr(DtrConfig {
                    resource: id,
                    start,
                    outputs: outputs.try_into().expect("eight DTR outputs"),
                    code: self.dtr_code,
                })?;
                self.record_resource(id, "HY04: board-profile initial/final full byte (ULX3S default 0x9E); STARTPULSE high/retrigger invalidates all bits to X; latest definite fall completes after pooled median 32.740 us; variable binary-count intermediate codes are not replayed; no Celsius scale; fast mode refused");
            }
            "NONE" => {
                for (bit, output) in outputs.into_iter().enumerate() {
                    self.builder.static_driver(
                        output,
                        &format!("dtr-inactive:{id:?}:{bit}"),
                        kernel::Strength::Strong,
                        Logic::X,
                    )?;
                }
                self.record_resource(
                    id,
                    "DTR.MODE NONE: inactive, outputs X; no sensor conversion",
                );
            }
            _ => bail!("unsupported DTR.MODE {mode}"),
        }
        Ok(())
    }
}
