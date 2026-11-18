//! A LUTRAM column owns the existing sixteen physical LUT cells. A write
//! invalidates its asynchronous reader even when no address input changed.
use super::*;

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct LutRam {
    pub node: NodeId,
    /// Both addresses use physical LUT bit order: A + 2B + 4C + 8D.
    /// The planner maps logical WAD pins to this order exactly once.
    pub read_address: [Input; 4],
    pub write_address: [Input; 4],
    pub data: Input,
    pub clock: Input,
    pub enable: Input,
    pub edge: Edge,
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Writer {
    pub config: LutRam,
    pub previous_clock: Logic,
    pub(super) reader: usize,
    pub(super) inhibit: Input,
}
pub(super) struct CellUpdate {
    pub node: NodeId,
    pub address: usize,
    pub value: Logic,
    pub reader: usize,
}

impl Builder {
    /// One synchronous write port and an asynchronous read of the same LUT.
    /// Startup/global write inhibition must be supplied by a qualified plan.
    pub fn lut_ram(&mut self, config: LutRam) -> Result<SignalId> {
        self.lut_ram_with_inhibit(config, Input::Constant(Logic::Zero))
    }
    pub fn lut_ram_with_inhibit(&mut self, config: LutRam, inhibit: Input) -> Result<SignalId> {
        for input in config.write_address.into_iter().chain([
            config.data,
            config.clock,
            config.enable,
            inhibit,
        ]) {
            self.validate_input(input)?;
        }
        let reader = self.combinational.len();
        let output = self.lut4_with_inputs(config.node, config.read_address)?;
        let index = self.lut_writes.len();
        for input in config.write_address.into_iter().chain([
            config.data,
            config.clock,
            config.enable,
            inhibit,
        ]) {
            if let Some(signal) = input.signal() {
                self.signals[signal.index].lut_writes.push(index);
            }
        }
        self.lut_writes.push(Writer {
            config,
            previous_clock: Logic::X,
            reader,
            inhibit,
        });
        Ok(output)
    }
}

impl Kernel {
    pub(super) fn capture_lut_writes(&mut self) -> Result<Vec<CellUpdate>> {
        let mut updates = Vec::new();
        for index in self.dirty_lut_writes.take() {
            self.stats.lut_write_evaluations += 1;
            let writer = &mut self.lut_writes[index];
            let config = &writer.config;
            let clock = config.clock.read(&self.signals);
            let edge = clock_edge(config.edge, writer.previous_clock, clock);
            writer.previous_clock = clock;
            if edge == Activity::Possible {
                self.stats.possible_clock_edges += 1;
            }
            let enable = config
                .enable
                .read(&self.signals)
                .and(writer.inhibit.read(&self.signals).invert());
            if edge == Activity::None || enable == Logic::Zero {
                continue;
            }
            self.stats.lut_write_edges += 1;
            let address = config.write_address.map(|i| i.read(&self.signals));
            let certain = edge == Activity::Definite
                && enable == Logic::One
                && address.iter().all(|v| v.bit().is_some());
            let data = match config.data.read(&self.signals) {
                Logic::Z => Logic::X,
                v => v,
            };
            // An uncertain address may select each compatible cell, but does
            // not certainly select any one of them. Merge with holding that
            // cell. This deliberately loses correlations between different
            // cells; it never invents a known value from an uncertain write.
            for (cell, &old) in self.state.lut(config.node)?.iter().enumerate() {
                if address
                    .iter()
                    .enumerate()
                    .any(|(bit, v)| v.bit().is_some_and(|v| v != (cell & (1 << bit) != 0)))
                {
                    continue;
                }
                let value = if certain {
                    data
                } else {
                    Logic::mux(Logic::X, old, data)
                };
                if value != old {
                    updates.push(CellUpdate {
                        node: config.node,
                        address: cell,
                        value,
                        reader: writer.reader,
                    });
                }
            }
        }
        Ok(updates)
    }
}
