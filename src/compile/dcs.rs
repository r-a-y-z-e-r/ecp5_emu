//! HC02 DCS planning: the decoded configuration bits select the mode.
use super::*;
use crate::kernel::{DcsConfig, DcsMode};

impl Planner {
    pub(super) fn execute_dcs(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let modes = self.composed(id, ".DCSMODE")?;
        let decoded = modes
            .iter()
            .map(|s| s.parse::<DcsMode>())
            .collect::<Result<Vec<_>>>()?;
        let mode = decoded[0];
        ensure!(
            decoded.iter().all(|m| *m == mode),
            "ambiguous DCSMODE bits: {modes:?}"
        );
        let clocks = [
            self.input(&self.resource_port(id, "CLK0")?)?,
            self.input(&self.resource_port(id, "CLK1")?)?,
        ];
        let select = [
            self.input(&self.resource_port(id, "SEL0")?)?,
            self.input(&self.resource_port(id, "SEL1")?)?,
        ];
        let mode_select = self.input(&self.resource_port(id, "MODESEL")?)?;
        let output = self.signal(&self.resource_port(id, "DCSOUT")?)?;
        self.builder.dcs(DcsConfig {
            resource: id,
            mode,
            clocks,
            select,
            mode_select,
            output,
        })?;
        self.record_resource(id, "HC02 DCSC: decoded DCSMODE bits (NONE aliases NEG); POS/NEG hold through two new active edges after the old active edge; MODESEL=1 is an immediate one-hot mux; static table otherwise; stopped new clock holds indefinitely, no timeout; event and two-valued fast modes");
        Ok(())
    }
}
