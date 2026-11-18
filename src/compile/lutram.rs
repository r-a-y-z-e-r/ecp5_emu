//! Standard distributed RAM binding from the shared A/B/C mode bit.
use super::*;
use crate::kernel::LutRam;

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct LutRamUse {
    pub plc: String,
    pub columns: Vec<String>,
    pub write_clock_mux: String,
    pub write_enable_mux: String,
    pub unmodelled_c_ff_di: Vec<String>,
    pub rule: String,
}

impl Planner {
    fn peer_node(&self, node: &Node, name: &str) -> Result<Node> {
        let plc = self
            .device
            .model()
            .resource(node.owner)
            .parent
            .context("slice has no PLC parent")?;
        let peer = self
            .device
            .model()
            .nodes(plc)
            .find(|n| n.native_name == name)
            .with_context(|| format!("missing LUTRAM peer {name}"))?;
        Ok(Node {
            id: peer.id,
            owner: peer.owner,
            tile: node.tile,
            name: peer.native_name.into(),
        })
    }

    pub(super) fn execute_lutram(&mut self, node: &Node) -> Result<()> {
        if self.done_lutram.contains(&node.tile) {
            return Ok(());
        }
        for (slice, expected) in [("SLICEA", "DPRAM"), ("SLICEB", "DPRAM"), ("SLICEC", "RAMW")] {
            ensure!(
                self.one(node.tile, &format!("{slice}.MODE"))? == expected,
                "unsupported mixed LUTRAM modes: {slice} must be {expected}"
            );
        }
        for k in 0..2 {
            ensure!(
                self.device
                    .word(node.tile, &format!("SLICEC.K{k}.INIT"))?
                    .iter()
                    .all(|&b| b == Logic::Zero),
                "nonzero/unknown C INIT in RAMW needs a separate model"
            );
        }
        let clock_mux = self.one(node.tile, "CLK1.CLKMUX")?;
        ensure!(
            ["CLK", "INV"].contains(&clock_mux.as_str()),
            "unsupported LUTRAM CLK1 polarity"
        );
        let enable_mux = self.one(node.tile, "SLICEA.WREMUX")?;
        ensure!(
            ["0", "1", "WRE", "INV"].contains(&enable_mux.as_str()),
            "unsupported LUTRAM WREMUX"
        );
        // The same physical inversion bit also belongs to LSR1. Reading both
        // owners records that overlap; apply the inversion only once to WRE.
        let lsr_mux = self.one(node.tile, "LSR1.LSRMUX")?;
        ensure!(
            lsr_mux
                == if ["INV", "0"].contains(&enable_mux.as_str()) {
                    "INV"
                } else {
                    "LSR"
                },
            "inconsistent shared WREMUX/LSR1 inversion bit"
        );
        self.done_lutram.insert(node.tile);
        let ramw = self.peer_node(node, "SLICEC.RAMW")?;
        for (output, input) in [
            ("WADO0", "D0"),
            ("WADO1", "B0"),
            ("WADO2", "C0"),
            ("WADO3", "A0"),
            ("WDO0", "C1"),
            ("WDO1", "A1"),
            ("WDO2", "D1"),
            ("WDO3", "B1"),
        ] {
            let input = self.tied_input(&ramw, "SLICEC", input, input)?;
            let signal = self.signal(&self.node_port(&ramw, output)?)?;
            self.builder.buffer(
                &format!("ramw:{}:{output}", self.device.model().tile(node.tile).name),
                input,
                signal,
            )?;
        }
        self.done_nodes.insert(ramw.id);
        self.report.counts.ramw += 1;
        let mut columns = Vec::new();
        for name in ["SLICEA.K0", "SLICEA.K1", "SLICEB.K0", "SLICEB.K1"] {
            let column = self.peer_node(node, name)?;
            let (slice, k) = name.split_once(".K").unwrap();
            let mut read_address = [Input::Constant(Logic::X); 4];
            for (i, p) in ["A", "B", "C", "D"].into_iter().enumerate() {
                read_address[i] = self.tied_input(&column, slice, &format!("{p}{k}"), p)?;
            }
            // Logical RAM bits 0 and 3 swap at the physical LUT boundary.
            let mut write_address = [Input::Constant(Logic::X); 4];
            for (i, bit) in [3, 1, 2, 0].into_iter().enumerate() {
                write_address[i] = self.input(&self.node_port(&column, &format!("WAD{bit}"))?)?;
            }
            let clock_wire = self.node_port(&column, "WCK")?;
            let clock = self.input(&clock_wire)?;
            let clock = if clock_mux == "INV" {
                Input::Inverted(self.signal(&clock_wire)?)
            } else {
                clock
            };
            let enable = match enable_mux.as_str() {
                "0" => Input::Constant(Logic::Zero),
                "1" => Input::Constant(Logic::One),
                "WRE" => self.input(&self.node_port(&column, "WRE")?)?,
                "INV" => {
                    let wire = self.node_port(&column, "WRE")?;
                    self.input(&wire)?;
                    Input::Inverted(self.signal(&wire)?)
                }
                _ => unreachable!(),
            };
            let data = self.input(&self.node_port(&column, "WD")?)?;
            self.builder.lut_ram_with_inhibit(
                LutRam {
                    node: column.id,
                    read_address,
                    write_address,
                    data,
                    clock,
                    enable,
                    edge: kernel::Edge::Rising,
                },
                self.system_input(SystemPort::Gwdis)?,
            )?;
            ensure!(
                self.done_nodes.insert(column.id),
                "LUTRAM column already has an execution owner"
            );
            self.report.counts.lutram_columns += 1;
            columns.push(name.into());
        }
        self.report.lutram.push(LutRamUse {plc:self.device.model().tile(node.tile).name.clone(), columns,
            write_clock_mux:clock_mux, write_enable_mux:enable_mux, unmodelled_c_ff_di:Vec::new(),
            rule:"shared A/B DPRAM + C RAMW; physical INIT and live cells; WAD[3,1,2,0]; atomic edge write; read/FF collision policy unvalidated on hardware".into()});
        Ok(())
    }
}
