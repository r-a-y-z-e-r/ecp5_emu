//! The edge-clock chain of the side banks: ECLKSYNCB (and BRGECLKSYNC),
//! ECLKBRIDGECS, TRELLIS_ECLKBUF and CLKDIVF (CLOCK_NETWORK.md; board runs
//! HI03, HI04, HI06, HI12 and HI13 under docs/inventory/LFE5U-85F).
use super::*;
use crate::kernel::{IoLogicConfig, IoPath};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigurationClock {
    PrimaryPin,
    ConstantOne,
    Other,
}

impl Planner {
    /// Follow the actual planned buffers, rather than guessing from a route
    /// name or from the clock's value at time zero. Planning route wires here
    /// uses the normal mux/constant qualification and marks them seen exactly
    /// as the outer work queue does; fabric is qualified only for constant-one CE.
    fn eclksync_configuration_clock(
        &mut self,
        raw: &Wire,
        visiting: &mut HashSet<WireId>,
    ) -> Result<ConfigurationClock> {
        let wire = self.index.canonical(self.device.model(), raw)?;
        if !visiting.insert(wire) {
            return Ok(ConfigurationClock::Other);
        }
        let result = self.eclksync_configuration_source(wire, visiting);
        visiting.remove(&wire);
        result
    }

    fn eclksync_configuration_source(
        &mut self,
        wire: WireId,
        visiting: &mut HashSet<WireId>,
    ) -> Result<ConfigurationClock> {
        if let Some(producers) = self.producers.get(&wire).cloned() {
            // nextpnr can drive a tied CE through a physical constant LUT.
            // Only a LOGIC-mode LUT with every table entry 1 is constant 1;
            // registers, RAM, carry and data-dependent fabric remain dynamic.
            if let [Producer::Node(index, port)] = producers.as_slice() {
                let node = &self.nodes[*index];
                if port == "F" && node.name.contains(".K") {
                    let slice = node.name.split('.').next().unwrap();
                    if self
                        .device
                        .enum_candidates(node.tile, &format!("{slice}.MODE"))?
                        == ["LOGIC"]
                        && self
                            .device
                            .word(node.tile, &format!("{}.INIT", node.name))?
                            .iter()
                            .all(|v| *v == Logic::One)
                    {
                        return Ok(ConfigurationClock::ConstantOne);
                    }
                }
                return Ok(ConfigurationClock::Other);
            }
            let [Producer::Resource(id, port)] = producers.as_slice() else {
                return Ok(ConfigurationClock::Other);
            };
            let resource = self.device.model().resource(*id);
            if resource.family == "PIO" && port == "O" {
                let primary = self.device.model().package_pins().any(|pin| {
                    pin.package == self.package
                        && pin.pio == Some(*id)
                        && pin.dual_function.contains("PCLK")
                });
                let input = self
                    .pio_modes
                    .get(&resource.key)
                    .is_some_and(|mode| mode.direction.as_deref() == Some("INPUT"));
                return Ok(if primary && input {
                    ConfigurationClock::PrimaryPin
                } else {
                    ConfigurationClock::Other
                });
            }
            let input_port = if resource.family.starts_with("DCC") && port == "CLKO" {
                let mode = self.qualified_fallback(*id, ".MODE", "NONE")?;
                if mode == "DCCA" {
                    let enable = self.resource_port(*id, "CE")?;
                    if self.eclksync_configuration_clock(&enable, visiting)?
                        != ConfigurationClock::ConstantOne
                    {
                        return Ok(ConfigurationClock::Other);
                    }
                } else if mode != "NONE" {
                    return Ok(ConfigurationClock::Other);
                }
                "CLKI"
            } else if resource.family == "TRELLIS_ECLKBUF" && port == "ECLKO" {
                "ECLKI"
            } else {
                // Includes fabric, PLL and OSCG: configuration-time activity
                // of PLL/OSCG outputs has not been measured by HI04/12/13.
                return Ok(ConfigurationClock::Other);
            };
            let input = self.resource_port(*id, input_port)?;
            return self.eclksync_configuration_clock(&input, visiting);
        }

        self.seen.resize(self.seen.len().max(wire.0 + 1), false);
        if !std::mem::replace(&mut self.seen[wire.0], true) {
            self.execute_wire(wire)?;
        }
        let raw = self.index.wire(wire);
        if let Some(constant) = self.report.constants.iter().find(|c| c.wire == raw) {
            return Ok(if constant.value == Logic::One {
                ConfigurationClock::ConstantOne
            } else {
                ConfigurationClock::Other
            });
        }
        let Some(signal) = self.signals.get(wire.0).copied().flatten() else {
            return Ok(ConfigurationClock::Other);
        };
        let root = self.builder.buffer_root(signal)?;
        if root != signal {
            if let Some(source) = self.signals.iter().position(|s| *s == Some(root)) {
                let raw = self.index.wire(WireId(source));
                return self.eclksync_configuration_clock(&raw, visiting);
            }
        }
        Ok(ConfigurationClock::Other)
    }

    /// Limit the HI12 NONE profile to the unused STOP path nextpnr
    /// leaves on an automatically promoted edge clock. Follow fixed aliases
    /// to the CIB; a selected mux, constant or native producer means STOP is
    /// connected, even when the driver happens to be constant at startup.
    pub(super) fn qualify_eclksync_none(&self, id: ResourceId) -> Result<()> {
        let key = &self.device.model().resource(id).key;
        let mode = self.qualified_fallback(id, ".MODE", "NONE")?;
        ensure!(
            mode == "NONE",
            "ECLKSYNC {key} bypass MODE={mode} is not qualified; requires MODE=NONE (HI12 qualifies only unrouted STOP)"
        );
        let mut pending = vec![self.resource_port(id, "STOP")?];
        let mut seen = HashSet::new();
        while let Some(raw) = pending.pop() {
            let wire = self.index.canonical(self.device.model(), &raw)?;
            if !seen.insert(wire) {
                continue;
            }
            ensure!(
                !self.producers.contains_key(&wire),
                "ECLKSYNC {key} MODE=NONE with routed STOP is not measured (HI12 qualifies only unrouted STOP)"
            );
            for item in self.index.incoming(self.device.model(), wire)? {
                if let Some(field) = &item.constant {
                    let values = self.device.enum_candidates(item.tile, field)?;
                    ensure!(!values.iter().any(|v| v == "0" || v == "1"), "ECLKSYNC {key} MODE=NONE with constant STOP ({field}={values:?}) is not measured (HI12 qualifies only unrouted STOP)");
                } else {
                    for connection in self
                        .device
                        .matching_connections_to(item.tile, &item.raw, None)
                    {
                        let connection = connection?;
                        ensure!(connection.definition.kind == ArcKind::Fixed, "ECLKSYNC {key} MODE=NONE with routed STOP ({} {} -> {}) is not measured (HI12 qualifies only unrouted STOP)", connection.tile.name, connection.definition.source, connection.definition.sink);
                        if let Endpoint::Wire(source) = connection.source {
                            pending.push(source);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// ECLKSYNCB (HI04): STOP passes four stages clocked by the falling edges
    /// of ECLKI, and ECLKO = ECLKI while the last one is 0. ECLKO stops low
    /// after the fourth falling edge that sees STOP high and restarts after
    /// the fourth that sees it low. HI13/HI12's pre-running primary pin path
    /// starts cleared, including constant-enabled DCCs; other sources retain
    /// HI04's stages at 1 and fifth-rising-edge start (C05).
    pub(super) fn execute_eclksync(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let key = self.device.model().resource(id).key.clone();
        let mode = self.qualified_fallback(id, ".MODE", "NONE")?;
        if mode == "NONE" {
            self.qualify_eclksync_none(id)?;
            let input = self.input(&self.resource_port(id, "ECLKI")?)?;
            let output = self.signal(&self.resource_port(id, "ECLKO")?)?;
            self.builder
                .buffer(&format!("eclksync:{key}"), input, output)?;
            self.report.counts.eclk_syncs += 1;
            self.record_resource(id, "HI12: MODE=NONE with unrouted STOP is a plain buffer ECLKO = ECLKI, with no deficit against pin-clock fabric counters at /2 or /3.5; STOP is unused and not sampled. Enabled ECLKSYNCB uses the HI04 STOP pipeline with HI12/HI13 source-dependent initial stages");
            return Ok(());
        }
        ensure!(
            mode == "ECLKSYNCB",
            "ECLKSYNC {key} MODE {mode} is not implemented"
        );
        let input_wire = self.resource_port(id, "ECLKI")?;
        let initial = if self.eclksync_configuration_clock(&input_wire, &mut HashSet::new())?
            == ConfigurationClock::PrimaryPin
        {
            Logic::Zero
        } else {
            Logic::One
        };
        let input = self.input(&input_wire)?;
        let stop = self.input(&self.resource_port(id, "STOP")?)?;
        let output = self.signal(&self.resource_port(id, "ECLKO")?)?;
        let resource = self.device.model().resource(id);
        let gate = self.signal(&Wire {
            location: resource.location,
            name: format!("STOPSYNC_{}", resource.name),
        })?;
        self.builder.iologic(IoLogicConfig {
            resource: id,
            name: "ECLKSYNC_STOP",
            path: IoPath::StopSync {
                stop,
                gate,
                initial,
            },
            clock: input,
            reset: None,
        })?;
        self.builder.mux(
            &format!("eclksync:{key}"),
            gate.into(),
            input,
            Input::Constant(Logic::Zero),
            output,
        )?;
        self.report.counts.eclk_syncs += 1;
        self.record_resource(id, if initial == Logic::Zero {
            "ECLKSYNCB (HI04, HI12, HI13): STOP through four stages on ECLKI falling edges, ECLKO = ECLKI while the last is 0; stages initially 0 for a primary clock input pin through routes/buffers and constant-CE=1 DCC/DCCA (HI12 sync_2, HI13 P), with no startup deficit"
        } else {
            "ECLKSYNCB (HI04, HI12, HI13): STOP through four stages on ECLKI falling edges, ECLKO = ECLKI while the last is 0; stages initially 1 for fabric or DCC/DCCA with CE other than constant 1 (HI04, HI13 G), losing four rising edges after ECLKI starts (occasional extra HI04 edge C05 not modelled). PLL/OSCG configuration-time activity is unmeasured and retains the HI04 stages at 1"
        });
        Ok(())
    }

    /// ECLKBRIDGECS (HI06). With MODE=ECLKBRIDGECS, ECSOUT = SEL ? CLK1 :
    /// CLK0 at once: a plain mux, so a switch can cut or add a pulse. With
    /// MODE=NONE, which nextpnr writes (it sets the mode only for a cell with
    /// a STOP port, which the bridge lacks), ECSOUT is CLK0 whatever SEL is.
    pub(super) fn execute_eclkbridge(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let key = self.device.model().resource(id).key.clone();
        let mode = self.qualified_fallback(id, ".MODE", "NONE")?;
        let clk0 = self.input(&self.resource_port(id, "CLK0")?)?;
        let output = self.signal(&self.resource_port(id, "ECSOUT")?)?;
        let label = format!("eclkbridge:{key}");
        match mode.as_str() {
            "ECLKBRIDGECS" => {
                let clk1 = self.input(&self.resource_port(id, "CLK1")?)?;
                let sel = self.input(&self.resource_port(id, "SEL")?)?;
                self.builder.mux(&label, sel, clk0, clk1, output)?;
                self.record_resource(id, "ECLKBRIDGECS as measured on the board (HI06): MODE=ECLKBRIDGECS makes ECSOUT = SEL ? CLK1 : CLK0 at once, a plain mux that can cut or add a pulse when SEL changes");
            }
            "NONE" => {
                self.builder.buffer(&label, clk0, output)?;
                self.record_resource(id, "ECLKBRIDGECS with MODE=NONE (what nextpnr writes, HI06): ECSOUT = CLK0 and SEL has no effect, as on the board");
            }
            _ => bail!("ECLKBRIDGECS {key} MODE {mode} is not implemented"),
        }
        self.report.counts.eclk_bridges += 1;
        Ok(())
    }

    /// TRELLIS_ECLKBUF: the bank's edge-clock buffer onto G_BANK<n>ECLK<m>.
    pub(super) fn execute_eclkbuf(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let key = self.device.model().resource(id).key.clone();
        let input = self.input(&self.resource_port(id, "ECLKI")?)?;
        let output = self.signal(&self.resource_port(id, "ECLKO")?)?;
        self.builder
            .buffer(&format!("eclkbuf:{key}"), input, output)?;
        self.record_resource(id, "edge-clock buffer: ECLKO = ECLKI");
        Ok(())
    }

    /// CLKDIVF /2 and /3.5 with RST and ALIGNWD, as HI04 measured them on
    /// the board (`IoPath::Divider`). It starts low and rises at the first
    /// edge of CLKI; the late start that HI03 saw is ECLKSYNCB's. GSR=ENABLED
    /// is not measured.
    pub(super) fn execute_clkdiv(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let key = self.device.model().resource(id).key.clone();
        let div = self.qualified_fallback(id, ".DIV", "2.0")?;
        let seven_halves = match div.as_str() {
            "2.0" => false,
            "3.5" => true,
            _ => bail!("CLKDIVF {key} DIV {div} is not a CLKDIVF ratio"),
        };
        let gsr = self.qualified_fallback(id, ".GSR", "DISABLED")?;
        ensure!(gsr == "DISABLED", "CLKDIVF {key} GSR={gsr} is not measured");
        let clock = self.input(&self.resource_port(id, "CLKI")?)?;
        let reset = self.input(&self.resource_port(id, "RST")?)?;
        let align = self.input(&self.resource_port(id, "ALIGNWD")?)?;
        let output = self.signal(&self.resource_port(id, "CDIVX")?)?;
        self.builder.iologic(IoLogicConfig {
            resource: id,
            name: "CLKDIVF",
            path: IoPath::Divider {
                align,
                output,
                seven_halves,
            },
            clock,
            reset: Some(reset),
        })?;
        self.report.counts.clock_dividers += 1;
        self.record_resource(id, "CLKDIVF as measured on the board (HI04): /2 toggles CDIVX at each CLKI rising edge, /3.5 counts both edges with CDIVX high for 4 of 7; RST clears it at once and the first rising edge after release is consumed; ALIGNWD sampled at rising edges slips one CLKI period two rising edges later, every time at /3.5 and every second time since reset at /2; starts low, rising at the first edge");
        Ok(())
    }

    /// The X2 gearbox of a side IOLOGIC (HI03): ODDRX2F or IDDRX2F, from the
    /// SCLK clock `sclk` (CLKOMUX/CLKIMUX), the ECLK port and the LSR reset.
    /// The SCLK side is three (ODDR) or one (IDDR) rank of ordinary IO
    /// registers on internal nets; the ECLK side is `GearOut`/`GearIn`
    /// (kernel/iologic.rs). The reset, which HI03 did not exercise, clears
    /// every stage as HI02 showed for X1.
    pub(super) fn execute_gearbox(
        &mut self,
        id: ResourceId,
        mode: &str,
        seven: bool,
        sclk: Input,
        reset: Option<Input>,
    ) -> Result<()> {
        if seven {
            return self.execute_gearbox7(id, mode, sclk, reset);
        }
        const ODDR_STAGES: [[&str; 4]; 3] = [
            ["ODDRX2_A0", "ODDRX2_A1", "ODDRX2_A2", "ODDRX2_A3"],
            ["ODDRX2_B0", "ODDRX2_B1", "ODDRX2_B2", "ODDRX2_B3"],
            ["ODDRX2_C0", "ODDRX2_C1", "ODDRX2_C2", "ODDRX2_C3"],
        ];
        const IDDR_OUTPUTS: [&str; 4] = ["IDDRX2_Q0", "IDDRX2_Q1", "IDDRX2_Q2", "IDDRX2_Q3"];
        let resource = self.device.model().resource(id);
        let (location, name) = (resource.location, resource.name.clone());
        let eclk = self.input(&self.resource_port(id, "ECLK")?)?;
        let net = |planner: &mut Self, label: &str| {
            planner.signal(&Wire {
                location,
                name: format!("X2_{label}_{name}"),
            })
        };
        let register = |planner: &mut Self,
                        label: &'static str,
                        data: Input,
                        output: SignalId|
         -> Result<()> {
            planner.builder.iologic(IoLogicConfig {
                resource: id,
                name: label,
                path: IoPath::Register {
                    data,
                    output,
                    value: Logic::Zero,
                    enable: Input::Constant(Logic::One),
                    sync_reset: None,
                    global_reset: None,
                },
                clock: sclk,
                reset,
            })
        };
        match mode {
            "ODDRXN" => {
                let mut word = [Input::Constant(Logic::Zero); 4];
                for (k, w) in word.iter_mut().enumerate() {
                    *w = self.input(&self.resource_port(id, &format!("TXDATA{k}"))?)?;
                }
                for labels in ODDR_STAGES {
                    let mut next = word;
                    for (k, label) in labels.into_iter().enumerate() {
                        let output = net(self, label)?;
                        register(self, label, word[k], output)?;
                        next[k] = output.into();
                    }
                    word = next;
                }
                let output = self.signal(&self.resource_port(id, "IOLDO")?)?;
                self.builder.iologic(IoLogicConfig {
                    resource: id,
                    name: "ODDRX2",
                    path: IoPath::GearOut {
                        data: word,
                        sclk,
                        output,
                    },
                    clock: eclk,
                    reset,
                })?;
            }
            "IDDRXN" => {
                let pad = self.input(&self.resource_port(id, "PADDI")?)?;
                let mut holding = Vec::new();
                for k in 0..4 {
                    holding.push(net(self, &format!("H{k}"))?);
                }
                self.builder.iologic(IoLogicConfig {
                    resource: id,
                    name: "IDDRX2",
                    path: IoPath::GearIn {
                        data: pad,
                        sclk,
                        outputs: holding.clone().try_into().unwrap(),
                    },
                    clock: eclk,
                    reset,
                })?;
                for (k, label) in IDDR_OUTPUTS.into_iter().enumerate() {
                    let q = self.signal(&self.resource_port(id, &format!("RXDATA{k}"))?)?;
                    register(self, label, holding[k].into(), q)?;
                }
            }
            _ => unreachable!(),
        }
        self.report.counts.iologic_paths += 1;
        if self.done_resources.insert(id) {
            self.record_resource(id, "IOLOGIC X2 gearbox as observed on the board (HI03): ODDRX2F puts D0..D3 on the pad from the ECLK rising edge where SCLK falls, after a word taken at SCLK rising edge S_m leaves after S_{m+2}; IDDRX2F samples the pad at four ECLK edges from that mid edge and publishes them one SCLK edge after the next mid edge; stages start at 0; the reset (not exercised by HI03) clears every stage, as HI02 showed for X1");
        }
        Ok(())
    }

    /// The 7:1 gearbox of a side IOLOGIC (HI07): ODDR71B or IDDR71B, from
    /// the SCLK clock `sclk` (CLKOMUX/CLKIMUX), the ECLK port and the LSR
    /// reset. ODDR71B: two ranks of SCLK registers, then `GearOut7`, which
    /// loads the word at the ECLK edge after SCLK rises and shows D0..D6 one
    /// per ECLK half. IDDR71B: `GearIn7` hands the pad at the last seven ECLK
    /// edges to holding nets at that same edge, oldest first into Q4, Q5, Q6,
    /// Q0..Q3, and SCLK registers publish them at the next SCLK rising edge.
    /// The reset, which HI07 did not exercise, clears every stage as HI02
    /// showed for X1.
    fn execute_gearbox7(
        &mut self,
        id: ResourceId,
        mode: &str,
        sclk: Input,
        reset: Option<Input>,
    ) -> Result<()> {
        const ODDR_STAGES: [[&str; 7]; 2] = [
            [
                "ODDR71_A0",
                "ODDR71_A1",
                "ODDR71_A2",
                "ODDR71_A3",
                "ODDR71_A4",
                "ODDR71_A5",
                "ODDR71_A6",
            ],
            [
                "ODDR71_B0",
                "ODDR71_B1",
                "ODDR71_B2",
                "ODDR71_B3",
                "ODDR71_B4",
                "ODDR71_B5",
                "ODDR71_B6",
            ],
        ];
        const IDDR_OUTPUTS: [&str; 7] = [
            "IDDR71_Q0",
            "IDDR71_Q1",
            "IDDR71_Q2",
            "IDDR71_Q3",
            "IDDR71_Q4",
            "IDDR71_Q5",
            "IDDR71_Q6",
        ];
        let resource = self.device.model().resource(id);
        let (location, name) = (resource.location, resource.name.clone());
        let eclk = self.input(&self.resource_port(id, "ECLK")?)?;
        let net = |planner: &mut Self, label: &str| {
            planner.signal(&Wire {
                location,
                name: format!("X71_{label}_{name}"),
            })
        };
        let register = |planner: &mut Self,
                        label: &'static str,
                        data: Input,
                        output: SignalId|
         -> Result<()> {
            planner.builder.iologic(IoLogicConfig {
                resource: id,
                name: label,
                path: IoPath::Register {
                    data,
                    output,
                    value: Logic::Zero,
                    enable: Input::Constant(Logic::One),
                    sync_reset: None,
                    global_reset: None,
                },
                clock: sclk,
                reset,
            })
        };
        match mode {
            "ODDRXN" => {
                let mut word = [Input::Constant(Logic::Zero); 7];
                for (k, w) in word.iter_mut().enumerate() {
                    *w = self.input(&self.resource_port(id, &format!("TXDATA{k}"))?)?;
                }
                for labels in ODDR_STAGES {
                    let mut next = word;
                    for (k, label) in labels.into_iter().enumerate() {
                        let output = net(self, label)?;
                        register(self, label, word[k], output)?;
                        next[k] = output.into();
                    }
                    word = next;
                }
                let output = self.signal(&self.resource_port(id, "IOLDO")?)?;
                self.builder.iologic(IoLogicConfig {
                    resource: id,
                    name: "ODDR71",
                    path: IoPath::GearOut7 {
                        data: word,
                        sclk,
                        output,
                    },
                    clock: eclk,
                    reset,
                })?;
            }
            "IDDRXN" => {
                let pad = self.input(&self.resource_port(id, "PADDI")?)?;
                let mut holding = Vec::new();
                for k in 0..7 {
                    holding.push(net(self, &format!("H{k}"))?);
                }
                self.builder.iologic(IoLogicConfig {
                    resource: id,
                    name: "IDDR71",
                    path: IoPath::GearIn7 {
                        data: pad,
                        sclk,
                        outputs: holding.clone().try_into().unwrap(),
                    },
                    clock: eclk,
                    reset,
                })?;
                for (k, label) in IDDR_OUTPUTS.into_iter().enumerate() {
                    let q = self.signal(&self.resource_port(id, &format!("RXDATA{k}"))?)?;
                    register(self, label, holding[k].into(), q)?;
                }
            }
            _ => unreachable!(),
        }
        self.report.counts.iologic_paths += 1;
        if self.done_resources.insert(id) {
            self.record_resource(id, "IOLOGIC 7:1 gearbox as observed on the board (HI07): ODDR71B shows D0..D6 one per ECLK half from the ECLK edge after the SCLK rise, for the word captured at the SCLK rise before; IDDR71B publishes at an SCLK rise the pad at the seven ECLK edges up to the edge after the previous SCLK rise, oldest first into Q4, Q5, Q6, Q0..Q3; stages start at 0; the reset (not exercised by HI07) clears every stage");
        }
        Ok(())
    }
}
