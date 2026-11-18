//! IOLOGIC registers, DDR paths and digitally modelled input/output delays.
use super::*;
use crate::kernel::{IoLogicConfig, IoPath};

impl Planner {
    /// The PIO data and tristate inputs: the fabric (`I`, `T`) unless a
    /// DATAMUX/TRIMUX field selects the IOLOGIC (`IOLDO`, `IOLTO`).
    pub(super) fn pio_output_sources(
        &self,
        id: ResourceId,
    ) -> Result<(&'static str, &'static str)> {
        let key = &self.device.model().resource(id).key;
        let names = self
            .device
            .resource_settings(id)
            .context("PIO without composed settings")?
            .fields
            .iter()
            .filter(|f| f.name.contains(".DATAMUX_") || f.name.contains(".TRIMUX_"))
            .map(|f| f.name.clone())
            .collect::<Vec<_>>();
        // DATAMUX_MDDR's IOLDO is the bits of DATAMUX_ODDR and DATAMUX_OREG
        // together (bits.db): when it is selected, as for the X2 gearbox,
        // the two decode as IOLDO too, as parts of it.
        let mddr = names.iter().any(|n| n.ends_with(".DATAMUX_MDDR"))
            && self.qualified_fallback(id, ".DATAMUX_MDDR", "PADDO")? == "IOLDO";
        let (mut data, mut tristate) = ("I", "T");
        for name in names {
            if mddr && (name.ends_with(".DATAMUX_ODDR") || name.ends_with(".DATAMUX_OREG")) {
                continue;
            }
            let suffix = format!(".{}", name.split_once('.').context("PIO field")?.1);
            let data_path = name.contains(".DATAMUX_");
            let (fabric, iologic) = if data_path {
                ("PADDO", "IOLDO")
            } else {
                ("PADDT", "IOLTO")
            };
            let value = self.qualified_fallback(id, &suffix, fabric)?;
            if value == fabric {
                continue;
            }
            ensure!(
                value == iologic,
                "unsupported PIO path {key}:{name}={value}"
            );
            ensure!(
                ["_ODDR", "_OREG", "_TSREG", "_MDDR"]
                    .iter()
                    .any(|s| name.ends_with(s)),
                "IOLOGIC memory path is not implemented: {key}:{name}"
            );
            let port = if data_path { &mut data } else { &mut tristate };
            ensure!(
                ["I", "T"].contains(port),
                "two PIO paths select the IOLOGIC: {key}"
            );
            *port = iologic;
        }
        // An output delay also uses the IOLOGIC with its register bypassed
        // (MODE=NONE). nextpnr leaves DATAMUX_OREG at PADDO in that case;
        // follow IOLDOD so its settings are checked and the delay is recorded.
        if data == "I" {
            let (site, name) = key.rsplit_once('/').context("PIO key")?;
            let letter = name.chars().last().context("PIO name")?;
            if let Some(iologic) = self
                .device
                .model()
                .resource_id(&format!("{site}/IOLOGIC{letter}"))
                .or_else(|| {
                    self.device
                        .model()
                        .resource_id(&format!("{site}/SIOLOGIC{letter}"))
                })
            {
                if self.iologic_output_delay_enabled(iologic)? {
                    data = "IOLDO";
                }
            }
        }
        Ok((data, tristate))
    }
    /// A wire with fixed arcs from two IOLOGIC outputs: the 7:1 lanes
    /// RXDATA4..6 of A/C share the CIB wires of RXDATA1..3 of B/D, and the
    /// output delay IOLDOD shares the PIO input with IOLDO. Returns the source
    /// that is active in its IOLOGIC's mode, or None for any other overlap.
    pub(super) fn shared_iologic_source(
        &self,
        fixed: &[(Option<Wire>, RouteUse, Vec<BitConstraint>)],
    ) -> Result<Option<usize>> {
        let (mut special, mut plain) = (Vec::new(), Vec::new());
        for (n, (source, _, _)) in fixed.iter().enumerate() {
            let Some(source) = source else {
                return Ok(None);
            };
            let key = self.index.canonical(self.device.model(), source)?;
            let Some([Producer::Resource(id, port)]) = self.producers.get(&key).map(Vec::as_slice)
            else {
                return Ok(None);
            };
            if !self
                .device
                .model()
                .resource(*id)
                .family
                .ends_with("IOLOGIC")
            {
                return Ok(None);
            }
            match port.as_str() {
                "RXDATA4" | "RXDATA5" | "RXDATA6" => {
                    special.push((n, self.iologic_setting(*id, "IDDRXN.MODE")? == "IDDR71"))
                }
                "IOLDOD" => special.push((n, self.iologic_output_delay_enabled(*id)?)),
                _ => plain.push(n),
            }
        }
        ensure!(
            special.len() == 1 && plain.len() == 1,
            "unqualified shared IOLOGIC sources: {:?}",
            fixed.iter().map(|f| &f.1.source).collect::<Vec<_>>()
        );
        Ok(Some(if special[0].1 { special[0].0 } else { plain[0] }))
    }
    /// The candidates of an IOLOGIC enum, or None when the resource has no
    /// such field. Top/bottom SIOLOGIC settings are named IOLOGIC too.
    fn iologic_candidates(&self, id: ResourceId, name: &str) -> Result<Option<Vec<String>>> {
        let resource = self.device.model().resource(id);
        let name = format!(
            "IOLOGIC{}.{name}",
            resource.name.chars().last().context("IOLOGIC name")?
        );
        Ok(self
            .device
            .resource_settings(id)
            .and_then(|r| r.fields.iter().find(|f| f.name == name))
            .map(|f| f.candidates.clone()))
    }
    /// Absence is harmless only if the bound database has no OUTDEL field,
    /// or an entirely undecoded resource occupies database-default tiles.
    fn iologic_output_delay_enabled(&self, id: ResourceId) -> Result<bool> {
        let model = self.device.model();
        let resource = model.resource(id);
        let name = format!(
            "IOLOGIC{}.DELAY.OUTDEL",
            resource.name.chars().last().context("IOLOGIC name")?
        );
        // Top-bank settings live in PICT fragments, not the PIOT anchor.
        let mut tiles = vec![resource.anchor];
        for site in model.site_references().iter().filter(|site| {
            site.resource == Some(id) && site.binding == crate::model::Binding::Instance
        }) {
            if !tiles.contains(&site.tile) {
                tiles.push(site.tile);
            }
        }
        let mut definitions = Vec::new();
        for &tile in &tiles {
            let tile = model.tile(tile);
            let kind = &model.database().types[&tile.kind];
            ensure!(
                !kind.words.contains_key(&name),
                "{} DELAY.OUTDEL is not a database enum",
                resource.key
            );
            if let Some(definition) = kind.enums.get(&name) {
                definitions.push(definition);
            }
        }
        let values = self.iologic_candidates(id, "DELAY.OUTDEL")?;
        if definitions.is_empty() {
            ensure!(
                values.is_none(),
                "{} DELAY.OUTDEL decode exists without a database field",
                resource.key
            );
            return Ok(false);
        }
        if self.device.resource_settings(id).is_none() {
            for tile in tiles {
                ensure!(
                    self.iologic_tile_is_default(tile).with_context(|| format!(
                        "{} DELAY.OUTDEL default tile check",
                        resource.key
                    ))?,
                    "{} DELAY.OUTDEL has no decoded settings on a configured tile",
                    resource.key
                );
            }
            ensure!(
                definitions
                    .iter()
                    .all(|d| d.default.as_deref() == Some("DISABLED")),
                "{} DELAY.OUTDEL database default must be DISABLED: {:?}",
                resource.key,
                definitions.iter().map(|d| &d.default).collect::<Vec<_>>()
            );
            return Ok(false);
        }
        let values =
            values.with_context(|| format!("{} has no DELAY.OUTDEL setting", resource.key))?;
        ensure!(
            values.len() == 1,
            "{} DELAY.OUTDEL is unresolved or ambiguous: {:?}",
            resource.key,
            values
        );
        match values[0].as_str() {
            "ENABLED" => Ok(true),
            "DISABLED" => Ok(false),
            value => bail!(
                "{} DELAY.OUTDEL has unsupported value {value}",
                resource.key
            ),
        }
    }
    /// Compare native CRAM with libtrellis' empty-tile writer: zero routing,
    /// word defaults first, then enum defaults in database name order. This
    /// also checks unused bits, and works for both text and bitstream images.
    fn iologic_tile_is_default(&self, id: TileId) -> Result<bool> {
        let tile = self.device.model().tile(id);
        let kind = &self.device.model().database().types[&tile.kind];
        let mut defaults = vec![false; tile.cram.frames * tile.cram.bits];
        for (name, value) in &kind.words {
            for (bits, value) in kind.word_bits[name].iter().zip(value.bytes().rev()) {
                for bit in bits {
                    defaults[bit.frame * tile.cram.bits + bit.bit] = bit.value == (value == b'1');
                }
            }
        }
        for definition in kind.enums.values() {
            if let Some(value) = &definition.default {
                let bits = definition
                    .patterns
                    .get(value)
                    .context("database enum default has no pattern")?;
                for bit in bits {
                    defaults[bit.frame * tile.cram.bits + bit.bit] = bit.value;
                }
            }
        }
        Ok(defaults.iter().enumerate().all(|(index, &value)| {
            self.device.image().cram().bit(
                tile.cram.start_frame + index / tile.cram.bits,
                tile.cram.start_bit + index % tile.cram.bits,
            ) == value
        }))
    }
    /// One IOLOGIC setting by exact name with a single candidate.
    fn iologic_setting(&self, id: ResourceId, name: &str) -> Result<String> {
        let resource = self.device.model().resource(id);
        let prefix = format!(
            "IOLOGIC{}",
            resource.name.chars().last().context("IOLOGIC name")?
        );
        let field = self
            .device
            .resource_settings(id)
            .and_then(|r| {
                r.fields
                    .iter()
                    .find(|f| f.name == format!("{prefix}.{name}"))
            })
            .with_context(|| format!("{} has no {name} setting", resource.key))?;
        if field.kind == "word" {
            return field
                .value
                .clone()
                .with_context(|| format!("{} {name} is {}", resource.key, field.status));
        }
        ensure!(
            field.candidates.len() == 1,
            "{} {name} is ambiguous: {:?}",
            resource.key,
            field.candidates
        );
        Ok(field.candidates[0].clone())
    }
    /// A control input through a CLK/LSR mux: the port as routed, or inverted.
    fn iologic_control(&mut self, id: ResourceId, port: &str, mux: &str) -> Result<Input> {
        let key = self.device.model().resource(id).key.clone();
        let wire = self.resource_port(id, port)?;
        let input = self.input(&wire)?;
        match self.iologic_setting(id, mux)?.as_str() {
            "CLK" | "LSR" | "CE" => Ok(input),
            "INV" => Ok(Input::Inverted(self.signal(&wire)?)),
            value => bail!("unsupported IOLOGIC {mux}={value}: {key}"),
        }
    }
    /// IOLOGIC output `port` (kernel/iologic.rs). The CLK, CE and LSR inputs
    /// are shared; CLKIMUX/CEIMUX/LSRIMUX apply them to the input registers and
    /// CLKOMUX/CEOMUX/LSROMUX to the output and tristate registers.
    pub(super) fn execute_iologic(&mut self, id: ResourceId, port: &str) -> Result<()> {
        let resource = self.device.model().resource(id);
        let key = resource.key.clone();
        // Top/bottom SIOLOGIC (HI02c) has no X2/7:1, memory or IOLTOMUX fields.
        let side = resource.family == "IOLOGIC";
        let letter = resource.name.chars().last().context("IOLOGIC name")?;
        let path = match port {
            "RXDATA0" | "RXDATA1" | "RXDATA2" | "RXDATA3" | "RXDATA4" | "RXDATA5" | "RXDATA6" => {
                "RXDATA"
            }
            "IOLDO" => "IOLDO",
            "IOLDOD" => "IOLDOD",
            "IOLTO" => "IOLTO",
            "INFF" => "INFF",
            "INDD" => "INDD",
            "CFLAG" => "CFLAG",
            _ => bail!("IOLOGIC output {port} is not implemented: {key}"),
        };
        if !self.done_iologic.insert((id, path)) {
            return Ok(());
        }
        let modes = self
            .iologic_candidates(id, "MODE")?
            .with_context(|| format!("{key} has no MODE setting"))?;
        let pio = key
            .rsplit_once('/')
            .and_then(|(site, _)| {
                self.device
                    .model()
                    .resource_id(&format!("{site}/PIO{letter}"))
            })
            .with_context(|| format!("{key} has no PIO"))?;
        // On SIOLOGIC, MODE=IREG_OREG has the same bits as NONE. An output
        // delay can bypass the register: DATAMUX_OREG decides that case.
        let mode = if modes.len() == 1 {
            modes[0].clone()
        } else if path != "RXDATA" && modes.iter().all(|m| m == "IREG_OREG" || m == "NONE") {
            if matches!(path, "IOLDO" | "IOLDOD")
                && self.qualified_fallback(pio, ".DATAMUX_OREG", "PADDO")? == "PADDO"
            {
                "NONE".to_string()
            } else {
                "IREG_OREG".to_string()
            }
        } else {
            bail!("{key} MODE is ambiguous: {modes:?}");
        };
        // IDDR71's bits include F2B2, the bit that tells IDDRX1_ODDRX1 from
        // IDDRXN in MODE (bits.db, IOLOGIC A and C), so with IDDRXN.MODE=IDDR71
        // the MODE decodes as IDDRX1_ODDRX1. The bit belongs to IDDR71.
        let mode = if mode == "IDDRX1_ODDRX1"
            && self.iologic_candidates(id, "IDDRXN.MODE")?.as_deref()
                == Some(&["IDDR71".to_string()])
        {
            "IDDRXN".to_string()
        } else {
            mode
        };
        // The gearbox ratio: X2 (HI03) or 7:1 (HI07).
        let seven = match mode.as_str() {
            "ODDRXN" => self.iologic_setting(id, "ODDRXN.MODE")? == "ODDR71",
            "IDDRXN" => self.iologic_setting(id, "IDDRXN.MODE")? == "IDDR71",
            _ => false,
        };
        // The PIO must select this path for the same mode. ODDRX2 reaches it
        // through DATAMUX_MDDR, ODDR71 through DATAMUX_ODDR.
        let pio_field = match (mode.as_str(), path) {
            ("IDDRX1_ODDRX1", "IOLDO") => Some((".DATAMUX_ODDR", "PADDO", "IOLDO")),
            ("IREG_OREG", "IOLDO") => Some((".DATAMUX_OREG", "PADDO", "IOLDO")),
            ("ODDRXN", "IOLDO") if seven => Some((".DATAMUX_ODDR", "PADDO", "IOLDO")),
            ("ODDRXN", "IOLDO") => Some((".DATAMUX_MDDR", "PADDO", "IOLDO")),
            ("IREG_OREG", "IOLTO") => Some((".TRIMUX_TSREG", "PADDT", "IOLTO")),
            _ => None,
        };
        if let Some((field, fabric, selected)) = pio_field {
            ensure!(
                self.qualified_fallback(pio, field, fabric)? == selected,
                "{key} MODE={mode} drives {path}, but the PIO does not select it with {field}"
            );
        }
        let setting = |name: &str| self.iologic_setting(id, name);
        for name in [
            "IDDRXN.MODE",
            "ODDRXN.MODE",
            "MIDDRX.MODE",
            "MODDRX.MODE",
            "MTDDRX.MODE",
        ] {
            let values = self.iologic_candidates(id, name)?;
            // The gearboxes are IDDRXN/ODDRXN with IDDRX2/ODDRX2 (HI03) or
            // IDDR71/ODDR71 (HI07).
            let expected = match (mode.as_str(), name, seven) {
                ("IDDRXN", "IDDRXN.MODE", false) => "IDDRX2",
                ("IDDRXN", "IDDRXN.MODE", true) => "IDDR71",
                ("ODDRXN", "ODDRXN.MODE", false) => "ODDRX2",
                ("ODDRXN", "ODDRXN.MODE", true) => "ODDR71",
                _ => "NONE",
            };
            // ODDRX2's bits hold those of MODDRX2 and MTSHX2 (bits.db: e.g.
            // ODDRX2 = F8B3 F8B4, MODDRX2 = MTSHX2 = F8B3 on IOLOGIC C), so
            // with ODDRXN those decode from the same bits. ODDR71's bits
            // (F8B3 F8B4 F9B3 among them) also hold MOSHX2 (F8B4 F9B3).
            let implied: &[&[&str]] = match (mode.as_str(), name, seven) {
                ("ODDRXN", "MODDRX.MODE", false) => &[&["MODDRX2"]],
                ("ODDRXN", "MODDRX.MODE", true) => {
                    &[&["MODDRX2"], &["MOSHX2"], &["MODDRX2", "MOSHX2"]]
                }
                ("ODDRXN", "MTDDRX.MODE", _) => &[&["MTSHX2"]],
                _ => &[],
            };
            ensure!(
                values.as_ref().map_or(!side, |v| v == &[expected]
                    || implied.iter().any(|i| v == i)),
                "IOLOGIC {name} is not implemented: {key} (MODE {mode}, candidates {values:?})"
            );
        }
        // HI05 measured CE, LSR_OVER_CE and GSR=ENABLED on the IO registers,
        // HI09 RST and GSR=ENABLED on the X1 DDR paths; the gearbox paths ran
        // only with GSR=DISABLED, ASYNC and no CE.
        let registers = mode == "IREG_OREG";
        let gsr = setting("GSR")?;
        let srmode = setting("SRMODE")?;
        ensure!(
            gsr == "DISABLED" || ((registers || mode == "IDDRX1_ODDRX1") && gsr == "ENABLED"),
            "IOLOGIC GSR={gsr} is not measured for MODE={mode}: {key}"
        );
        ensure!(
            srmode == "ASYNC" || (registers && srmode == "LSR_OVER_CE"),
            "IOLOGIC SRMODE={srmode} is not measured for MODE={mode}: {key}"
        );
        // DELAYG/DELAYF change timing by up to 127 taps, not data. Both paths
        // use zero propagation delay (HI08 input and HI11 output measurements).
        // DELAYF's CFLAG is a separate, path-neutral counter output.
        ensure!(
            setting("DELAY.WAIT_FOR_EDGE")? == "DISABLED",
            "IOLOGIC DELAY.WAIT_FOR_EDGE is not implemented (requires DISABLED): {key}"
        );
        if path == "CFLAG" {
            return self.execute_delay_flag(id);
        }
        if path == "INDD" {
            // The delayed pad to the fabric.
            let pad = self.input(&self.resource_port(id, "PADDI")?)?;
            let output = self.signal(&self.resource_port(id, "INDD")?)?;
            self.builder.buffer(&format!("indd:{key}"), pad, output)?;
            self.record_resource(id, "IOLOGIC input delay to INDD with zero delay, as every propagation delay of the emulator (HI08: DEL_VALUE and LOADN/MOVE change the timing by up to 127 taps, not the data)");
            return Ok(());
        }
        if path == "IOLDOD" {
            ensure!(
                self.iologic_output_delay_enabled(id)?,
                "IOLOGIC IOLDOD requires DELAY.OUTDEL=ENABLED: {key}"
            );
            self.execute_iologic(id, "IOLDO")?;
            let data = self.signal(&self.resource_port(id, "IOLDO")?)?;
            let output = self.signal(&self.resource_port(id, "IOLDOD")?)?;
            self.builder
                .buffer(&format!("ioldod:{key}"), data.into(), output)?;
            self.record_resource(id, "IOLOGIC output delay to IOLDOD with zero propagation delay: IOLDO from the output register or bypassed TXDATA0 passes unchanged and uninverted (HI08 input-delay model; HI11 measured OFS1P3BX/DELAYF, OFS1P3BX/DELAYG and fabric FF/DELAYG UART output on left-bank L4; other banks assumed to preserve data too)");
            return Ok(());
        }
        if mode == "NONE" && path == "IOLDO" {
            let data = self.input(&self.resource_port(id, "TXDATA0")?)?;
            let output = self.signal(&self.resource_port(id, "IOLDO")?)?;
            self.builder.buffer(&format!("ioldo:{key}"), data, output)?;
            return Ok(());
        }
        let direction = if path == "RXDATA" || path == "INFF" {
            "I"
        } else {
            "O"
        };
        let ce_mux = self.iologic_setting(id, &format!("CE{direction}MUX"))?;
        let enable = match ce_mux.as_str() {
            "1" => Input::Constant(Logic::One),
            "CEMUX" if registers => match self.iologic_setting(id, "CEMUX")?.as_str() {
                "1" => Input::Constant(Logic::One),
                "0" => Input::Constant(Logic::Zero),
                _ => self.iologic_control(id, "CE", "CEMUX")?,
            },
            value => bail!("IOLOGIC clock enable CE{direction}MUX={value} is not measured for MODE={mode}: {key}"),
        };
        let lsr_mux = self.iologic_setting(id, &format!("LSR{direction}MUX"))?;
        let clock = self.iologic_control(id, "CLK", &format!("CLK{direction}MUX"))?;
        let lsr = match lsr_mux.as_str() {
            "0" => None,
            "LSRMUX" => Some(self.iologic_control(id, "LSR", "LSRMUX")?),
            value => bail!("unsupported IOLOGIC LSR{direction}MUX={value}: {key}"),
        };
        // ASYNC: the path's reset; LSR_OVER_CE: a synchronous one.
        let (reset, sync_reset) = if srmode == "ASYNC" {
            (lsr, None)
        } else {
            (None, lsr)
        };
        let global_reset = if gsr == "ENABLED" {
            Some(self.system_input(SystemPort::Gsr)?)
        } else {
            None
        };
        let data = |planner: &mut Self, name: &str| -> Result<Input> {
            planner.input(&planner.resource_port(id, name)?)
        };
        let output = |planner: &mut Self, name: &str| -> Result<SignalId> {
            planner.signal(&planner.resource_port(id, name)?)
        };
        let regset = |planner: &Self, name: &str, mode: &str| -> Result<Logic> {
            ensure!(
                planner.iologic_setting(id, mode)? == "FF",
                "IOLOGIC latch is not implemented: {key} {mode}"
            );
            match planner.iologic_setting(id, name)?.as_str() {
                "RESET" => Ok(Logic::Zero),
                "SET" => Ok(Logic::One),
                value => bail!("unsupported IOLOGIC {name}={value}: {key}"),
            }
        };
        if mode == "ODDRXN" || mode == "IDDRXN" {
            // The gearbox drives IOLDO (ODDRXN) or RXDATA (IDDRXN) only; any
            // other path of the same IOLOGIC would rebuild it.
            ensure!(
                matches!(
                    (mode.as_str(), path),
                    ("ODDRXN", "IOLDO") | ("IDDRXN", "RXDATA")
                ),
                "IOLOGIC MODE={mode} output {path} is not implemented: {key}"
            );
            return self.execute_gearbox(id, &mode, seven, clock, reset);
        }
        // HI10: an input or output register in latch mode.
        if registers && matches!(path, "INFF" | "IOLDO") {
            let (mode_field, regset_field, data_port, name) = if path == "INFF" {
                ("FF.INREGMODE", "FF.REGSET", "PADDI", "ILATCH")
            } else {
                ("OUTREG.OUTREGMODE", "OUTREG.REGSET", "TXDATA0", "OLATCH")
            };
            if self.iologic_setting(id, mode_field)? == "LATCH" {
                ensure!(
                    srmode == "ASYNC"
                        && global_reset.is_none()
                        && self.iologic_setting(id, regset_field)? == "RESET",
                    "IOLOGIC latch is measured only with SRMODE=ASYNC, GSR=DISABLED and REGSET=RESET (HI10): {key}"
                );
                let io_path = IoPath::Latch {
                    data: data(self, data_port)?,
                    enable,
                    output: output(self, path)?,
                };
                self.builder.iologic(IoLogicConfig {
                    resource: id,
                    name,
                    path: io_path,
                    clock,
                    reset,
                })?;
                self.report.counts.iologic_paths += 1;
                if self.done_resources.insert(id) {
                    self.record_resource(id, "IOLOGIC IO register in latch mode as observed on the board (HI10): transparent while its clock is low and CE high, holding its output otherwise, LSR (ASYNC) clearing at once");
                }
                return Ok(());
            }
        }
        let (name, io_path) = match (mode.as_str(), path) {
            ("IDDRX1_ODDRX1", "IOLDO") => (
                "ODDR",
                IoPath::OutputDdr {
                    data: [data(self, "TXDATA0")?, data(self, "TXDATA1")?],
                    output: output(self, "IOLDO")?,
                    global_reset,
                },
            ),
            ("IDDRX1_ODDRX1", "RXDATA") => (
                "IDDR",
                IoPath::InputDdr {
                    data: data(self, "PADDI")?,
                    outputs: [output(self, "RXDATA0")?, output(self, "RXDATA1")?],
                    global_reset,
                },
            ),
            ("IREG_OREG", "IOLDO") => (
                "OREG",
                IoPath::Register {
                    data: data(self, "TXDATA0")?,
                    output: output(self, "IOLDO")?,
                    value: regset(self, "OUTREG.REGSET", "OUTREG.OUTREGMODE")?,
                    enable,
                    sync_reset,
                    global_reset,
                },
            ),
            ("IREG_OREG", "IOLTO") => {
                // NONE and TS have identical bits; TDDR would select DDR
                // tristate. SIOLOGIC has no IOLTOMUX.
                let values = self.iologic_candidates(id, "IOLTOMUX")?;
                ensure!(
                    values.as_ref().map_or(!side, |v| !v.is_empty()
                        && v.iter().all(|v| v == "NONE" || v == "TS")),
                    "IOLOGIC IOLTOMUX {values:?} is not implemented: {key}"
                );
                (
                    "TSREG",
                    IoPath::Register {
                        data: data(self, "TSDATA0")?,
                        output: output(self, "IOLTO")?,
                        value: regset(self, "TSREG.REGSET", "TSREG.OUTREGMODE")?,
                        enable,
                        sync_reset,
                        global_reset,
                    },
                )
            }
            ("IREG_OREG", "INFF") => (
                "IREG",
                IoPath::Register {
                    data: data(self, "PADDI")?,
                    output: output(self, "INFF")?,
                    value: regset(self, "FF.REGSET", "FF.INREGMODE")?,
                    enable,
                    sync_reset,
                    global_reset,
                },
            ),
            (mode, path) => bail!("IOLOGIC MODE={mode} output {path} is not implemented: {key}"),
        };
        self.builder.iologic(IoLogicConfig {
            resource: id,
            name,
            path: io_path,
            clock,
            reset,
        })?;
        self.report.counts.iologic_paths += 1;
        if self.done_resources.insert(id) {
            self.record_resource(id, "IOLOGIC/SIOLOGIC ODDRX1F/IDDRX1F and IREG/OREG/TSREG as observed on the board (HI02, HI02c, HI05, HI09): DDR stages start at 0, and RST and GSR=ENABLED clear them and the outputs asynchronously; IO registers start at REGSET; IO registers take CE, a synchronous LSR (LSR_OVER_CE) that wins over CE, an ASYNC LSR, and with GSR=ENABLED the global reset, asynchronously; SIOLOGIC MODE IREG_OREG/NONE (identical bits) qualified by the used path and PIO selection");
        }
        Ok(())
    }
    /// DELAYF's CFLAG (HI08, HI11): the delay counter, an IOLOGIC path clocked by
    /// MOVE with LOADN loading DEL_VALUE, and CFLAG = DIRECTION ? counter == 0
    /// : counter == 127 as plan muxes, so it settles in dependency order.
    /// LOADNMUX=1 (Trellis' other value) ties LOADN high.
    fn execute_delay_flag(&mut self, id: ResourceId) -> Result<()> {
        let resource = self.device.model().resource(id);
        let (key, location, name) = (
            resource.key.clone(),
            resource.location,
            resource.name.clone(),
        );
        let side = resource.family == "IOLOGIC";
        let value = u8::from_str_radix(&self.iologic_setting(id, "DELAY.DEL_VALUE")?, 2)?;
        // HI11 measured left-bank IOLOGIC; HI08/HI11 measured bottom-bank
        // SIOLOGIC. Apply each kind's startup to unmeasured right/top banks.
        let initial = if side { value & 0x3e } else { 0 };
        let loadn = match self.iologic_candidates(id, "LOADNMUX")?.as_deref() {
            Some([v]) if v == "LOADN" => self.input(&self.resource_port(id, "LOADN")?)?,
            _ => Input::Constant(Logic::One),
        };
        let direction = self.input(&self.resource_port(id, "DIRECTION")?)?;
        let clock = self.input(&self.resource_port(id, "MOVE")?)?;
        let net = |planner: &mut Self, label: String| {
            planner.signal(&Wire {
                location,
                name: format!("DELAY_{label}_{name}"),
            })
        };
        let mut bits = Vec::new();
        for k in 0..7 {
            bits.push(net(self, format!("C{k}"))?);
        }
        self.builder.iologic(IoLogicConfig {
            resource: id,
            name: "DELAY_COUNT",
            path: IoPath::DelayCounter {
                loadn,
                direction,
                value,
                initial,
                outputs: bits.clone().try_into().unwrap(),
            },
            clock,
            reset: None,
        })?;
        // All ones (127) and all zeros (0) as chains of muxes, then DIRECTION.
        let (mut ones, mut zeros): (Input, Input) = (bits[0].into(), Input::Inverted(bits[0]));
        for (k, &bit) in bits.iter().enumerate().skip(1) {
            let o = net(self, format!("ONES{k}"))?;
            self.builder.mux(
                &format!("cflag:{key}:ones{k}"),
                bit.into(),
                Input::Constant(Logic::Zero),
                ones,
                o,
            )?;
            let z = net(self, format!("ZEROS{k}"))?;
            self.builder.mux(
                &format!("cflag:{key}:zeros{k}"),
                bit.into(),
                zeros,
                Input::Constant(Logic::Zero),
                z,
            )?;
            (ones, zeros) = (o.into(), z.into());
        }
        let flag = self.signal(&self.resource_port(id, "CFLAG")?)?;
        self.builder
            .mux(&format!("cflag:{key}"), direction, ones, zeros, flag)?;
        self.record_resource(id, "DELAYF counter and CFLAG (HI08, HI11): after configuration IOLOGIC starts at DEL_VALUE & 0x3E (bits 1..5, measured on left-bank output L4 and input A2); SIOLOGIC starts at 0 (measured on bottom-bank FIRE inputs, including R1). Right IOLOGIC and top SIOLOGIC use their kind's rule as an unmeasured assumption. LOADN low loads all seven DEL_VALUE bits and blocks MOVE; subsequent counts saturate at 0..127, stepped at MOVE's falling edge by DIRECTION taken at its rise (0 up). CFLAG = DIRECTION ? counter == 0 : counter == 127 at once. DIRECTION changes while MOVE is high remain outside the protocol and differ from the board in some cases (HI08b, HI08c)");
        Ok(())
    }
}
