use crate::{
    config::{Config, TileConfig},
    database::{Database, Tile, TileType},
    logic::Logic,
    routing::Routing,
    sim::{FlipFlop, Function, Input, Simulation},
};
use anyhow::{ensure, Context, Result};
use std::collections::{BTreeMap, BTreeSet};

pub struct Port {
    pub direction: String,
    pub net: usize,
    pub driver: usize,
}

pub struct Chip {
    pub routing: Routing,
    pub simulation: Simulation,
    pub ports: BTreeMap<String, Port>,
}

#[derive(Debug, Default)]
pub struct Coverage {
    pub unsupported: Vec<String>,
    pub unknown_bits: usize,
    pub logic_tiles: usize,
}

pub fn expand_groups(db: &Database, config: &Config) -> Result<Config> {
    let mut expanded = config.clone();
    for (names, group) in &config.groups {
        ensure!(
            group.arcs.is_empty() && group.unknown.is_empty(),
            "tile groups may only contain words and enums"
        );
        let mut matched = BTreeSet::new();
        for name in names {
            let tile = db
                .tiles
                .get(name)
                .with_context(|| format!("unknown group tile {name}"))?;
            let tt = &db.types[&tile.kind];
            let tc = expanded.tiles.entry(name.clone()).or_default();
            for (key, value) in &group.words {
                if tt.words.contains_key(key) {
                    tc.words.insert(key.clone(), value.clone());
                    matched.insert(key);
                }
            }
            for (key, value) in &group.enums {
                if tt.enums.contains_key(key) {
                    tc.enums.insert(key.clone(), value.clone());
                    matched.insert(key);
                }
            }
        }
        for key in group.words.keys().chain(group.enums.keys()) {
            ensure!(
                matched.contains(key),
                "group setting {key} matches no member tile"
            );
        }
    }
    expanded.groups.clear();
    db.validate(&expanded)?;
    Ok(expanded)
}

pub fn coverage(db: &Database, config: &Config) -> Coverage {
    let mut report = Coverage {
        logic_tiles: db.tiles.values().filter(|t| t.kind == "PLC2").count(),
        ..Coverage::default()
    };
    for (name, tc) in &config.tiles {
        let tile = &db.tiles[name];
        report.unknown_bits += tc.unknown.len();
        if tile.kind == "PLC2" && !tc.unknown.is_empty() {
            report
                .unsupported
                .push(format!("{name}: unknown logic configuration bits"));
        }
        for (key, value) in &tc.enums {
            let pio_attribute = key
                .split_once('.')
                .map(|(_, attribute)| attribute)
                .unwrap_or("");
            let basic_pio = key.starts_with("PIO")
                && [
                    "BASE_TYPE",
                    "PULLMODE",
                    "OPENDRAIN",
                    "CLAMP",
                    "DRIVE",
                    "HYSTERESIS",
                    "SLEWRATE",
                    "DIFFRESISTOR",
                    "TERMINATION_1V35",
                    "TERMINATION_1V5",
                    "TERMINATION_1V8",
                ]
                .contains(&pio_attribute);
            let supported = key.starts_with("SLICE")
                || key.starts_with("CLK")
                || key.starts_with("LSR")
                || key.starts_with("CIB.")
                || basic_pio
                || key.starts_with("BANK.")
                || (key.starts_with("DCC_") && value == "NONE");
            let default = db.types[&tile.kind]
                .enums
                .get(key)
                .and_then(|d| d.default.as_ref());
            if !supported && default != Some(value) && value != "_NONE_" {
                report.unsupported.push(format!("{name}: {key}={value}"));
            }
            if key.starts_with("SLICE")
                && key.ends_with(".MODE")
                && !["LOGIC", "CCU2"].contains(&value.as_str())
            {
                report.unsupported.push(format!("{name}: {key}={value}"));
            }
            if key.ends_with(".PULLMODE") && !["NONE", "UP", "DOWN"].contains(&value.as_str()) {
                report.unsupported.push(format!("{name}: {key}={value}"));
            }
        }
        if tile.kind != "PLC2" {
            for key in tc.words.keys() {
                report
                    .unsupported
                    .push(format!("{name}: unmodelled word {key}"));
            }
        }
        for source in tc.arcs.values() {
            if ["G_DCS0", "G_DCS1"].contains(&source.as_str()) {
                report
                    .unsupported
                    .push(format!("{name}: dynamic clock selector {source}"));
            }
        }
    }
    if !config.bram.is_empty() {
        report
            .unsupported
            .push("block RAM initialisation is not modelled yet".into());
    }
    report.unsupported.sort();
    report.unsupported.dedup();
    report
}

type GateSpec = (Function, Vec<Input>, Vec<usize>);
struct Builder {
    routing: Routing,
    gates: Vec<GateSpec>,
    registers: Vec<FlipFlop>,
    constants: Vec<(usize, Logic)>,
}

impl Builder {
    fn net(&mut self, tile: &Tile, name: &str) -> Result<usize> {
        self.routing.net(tile.row, tile.col, name)
    }
    fn input(&mut self, tile: &Tile, name: &str) -> Result<Input> {
        Ok(Input::Net(self.net(tile, name)?))
    }
    fn mux_input(
        &mut self,
        tile: &Tile,
        tt: &TileType,
        tc: &TileConfig,
        slice: &str,
        local: &str,
        physical: &str,
    ) -> Result<Input> {
        let value = tt
            .enum_value(tc, &format!("{slice}.{local}MUX"))
            .unwrap_or(local);
        match value {
            "0" => Ok(Input::Constant(Logic::Zero)),
            "1" => Ok(Input::Constant(Logic::One)),
            _ => self.input(tile, physical),
        }
    }

    fn logic_tile(&mut self, tile: &Tile, tt: &TileType, tc: &TileConfig) -> Result<()> {
        for (z, letter) in ['A', 'B', 'C', 'D'].iter().enumerate() {
            let slice = format!("SLICE{letter}");
            let mode = tt
                .enum_value(tc, &format!("{slice}.MODE"))
                .unwrap_or("LOGIC");
            ensure!(
                ["LOGIC", "CCU2"].contains(&mode),
                "{} {slice}: mode {mode} is not modelled",
                tile.name
            );
            let cin_name = if z == 0 {
                "FCI_SLICE".into()
            } else {
                format!("FCI{letter}_SLICE")
            };
            let mut carry = self.input(tile, &cin_name)?;
            let mut f = Vec::new();
            let mut m = Vec::new();
            for k in 0..2 {
                let index = 2 * z + k;
                let inputs = ['A', 'B', 'C', 'D']
                    .iter()
                    .map(|p| {
                        self.mux_input(
                            tile,
                            tt,
                            tc,
                            &slice,
                            &format!("{p}{k}"),
                            &format!("{p}{index}_SLICE"),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                m.push(self.mux_input(
                    tile,
                    tt,
                    tc,
                    &slice,
                    &format!("M{k}"),
                    &format!("M{index}_SLICE"),
                )?);
                let mask = tt.word_u16(tc, &format!("{slice}.K{k}.INIT"))?;
                let output = self.net(tile, &format!("F{index}_SLICE"))?;
                f.push(output);
                if mode == "CCU2" {
                    let cout = self.net(
                        tile,
                        &if k == 0 {
                            format!("FCO{letter}1_SLICE")
                        } else if z == 3 {
                            "FCO_SLICE".into()
                        } else {
                            format!("FCO{letter}_SLICE")
                        },
                    )?;
                    let mut inputs = inputs;
                    inputs.push(carry);
                    let inject =
                        tt.enum_value(tc, &format!("{slice}.CCU2.INJECT1_{k}")) == Some("YES");
                    self.gates
                        .push((Function::Carry { mask, inject }, inputs, vec![output, cout]));
                    carry = Input::Net(cout);
                } else {
                    self.gates.push((Function::Lut(mask), inputs, vec![output]));
                }
            }
            if mode == "LOGIC" {
                let f5 = self.net(tile, &format!("F5{letter}_SLICE"))?;
                self.gates.push((
                    Function::Mux,
                    vec![m[0], Input::Net(f[0]), Input::Net(f[1])],
                    vec![f5],
                ));
                let fxa = self.input(tile, &format!("FXA{letter}_SLICE"))?;
                let fxb = self.input(tile, &format!("FXB{letter}_SLICE"))?;
                let fx = self.net(tile, &format!("FX{letter}_SLICE"))?;
                self.gates
                    .push((Function::Mux, vec![m[1], fxa, fxb], vec![fx]));
            }
            let selected_clk = tc
                .arcs
                .get(&format!("MUXCLK{z}"))
                .map_or("CLK0", String::as_str);
            let selected_lsr = tc
                .arcs
                .get(&format!("MUXLSR{z}"))
                .map_or("LSR0", String::as_str);
            let clock = Input::Pulldown(self.net(tile, &format!("CLK{z}_SLICE"))?);
            let reset = Input::Pulldown(self.net(tile, &format!("LSR{z}_SLICE"))?);
            let ce = tt.enum_value(tc, &format!("{slice}.CEMUX")).unwrap_or("CE");
            let enable = match ce {
                "0" => Input::Constant(Logic::Zero),
                "1" => Input::Constant(Logic::One),
                _ => self.input(tile, &format!("CE{z}_SLICE"))?,
            };
            for (k, &m_input) in m.iter().enumerate() {
                let index = 2 * z + k;
                let sd = tt
                    .enum_value(tc, &format!("{slice}.REG{k}.SD"))
                    .unwrap_or("1");
                let data = if sd == "0" {
                    m_input
                } else {
                    self.input(tile, &format!("DI{index}_SLICE"))?
                };
                let output = self.net(tile, &format!("Q{index}_SLICE"))?;
                self.registers.push(FlipFlop {
                    clock,
                    reset,
                    enable,
                    data,
                    output,
                    preload: (tt.enum_value(tc, &format!("{slice}.REG{k}.LSRMODE"))
                        == Some("PRLD"))
                    .then_some(m_input),
                    reset_value: Logic::from_bool(
                        tt.enum_value(tc, &format!("{slice}.REG{k}.REGSET")) != Some("RESET"),
                    ),
                    invert_clock: tt.enum_value(tc, &format!("{selected_clk}.CLKMUX"))
                        == Some("INV"),
                    invert_reset: tt.enum_value(tc, &format!("{selected_lsr}.LSRMUX"))
                        == Some("INV"),
                    invert_enable: ce == "INV",
                    asynchronous: tt.enum_value(tc, &format!("{selected_lsr}.SRMODE"))
                        == Some("ASYNC"),
                    name: format!("{}:{slice}.REG{k}", tile.name),
                });
            }
        }
        Ok(())
    }
}

impl Chip {
    pub fn compile(db: &Database, original: &Config, package: &str) -> Result<Self> {
        let config = expand_groups(db, original)?;
        let report = coverage(db, &config);
        ensure!(
            report.unsupported.is_empty(),
            "unmodelled active configuration:\n{}",
            report.unsupported.join("\n")
        );
        let mut b = Builder {
            routing: Routing::build(db, &config)?,
            gates: Vec::new(),
            registers: Vec::new(),
            constants: Vec::new(),
        };
        let empty = TileConfig::default();
        for tile in db.tiles.values() {
            let tt = &db.types[&tile.kind];
            let tc = config.tiles.get(&tile.name).unwrap_or(&empty);
            if tile.kind == "PLC2" {
                b.logic_tile(tile, tt, tc)?;
            }
            for (name, def) in &tt.enums {
                let value = tt.enum_value(tc, name).or(def.default.as_deref());
                if name.starts_with("CIB.")
                    && name.ends_with("MUX")
                    && matches!(value, Some("0" | "1"))
                {
                    let net = b.net(tile, &name[4..name.len() - 3])?;
                    b.constants
                        .push((net, Logic::from_bool(value == Some("1"))));
                }
                if name.starts_with("DCC_") && name.ends_with(".MODE") {
                    let id = name[4..name.len() - 5].replacen('_', "DCC", 1);
                    // Names are DCC_L0.MODE -> LDCC0.
                    let id = format!("{}DCC{}", &id[..1], &id[1..]);
                    let input = b.input(tile, &format!("G_CLKI_{id}"))?;
                    let output = b.net(tile, &format!("G_CLKO_{id}"))?;
                    b.gates.push((Function::Buffer, vec![input], vec![output]));
                }
            }
        }
        let pins = db
            .packages
            .get(package)
            .with_context(|| format!("unknown package {package}"))?;
        let mut port_specs = Vec::new();
        let mut pulls = Vec::new();
        for (name, pin) in pins {
            let (pr, pc) = if pin.row == 0 || pin.row == db.max_row {
                (pin.row, pin.col + i16::from(pin.pio == "B"))
            } else {
                (pin.row + 1, pin.col)
            };
            let pio_tile = db.tiles.values().find(|t| {
                t.row == pr
                    && t.col == pc
                    && db.types[&t.kind]
                        .enums
                        .contains_key(&format!("PIO{}.BASE_TYPE", pin.pio))
            });
            let Some(tile) = pio_tile else {
                continue;
            };
            let tt = &db.types[&tile.kind];
            let tc = config.tiles.get(&tile.name).unwrap_or(&empty);
            let base = tt
                .enum_value(tc, &format!("PIO{}.BASE_TYPE", pin.pio))
                .unwrap_or("NONE");
            let direction = base.split('_').next().unwrap_or("NONE");
            if !["INPUT", "OUTPUT", "BIDIR"].contains(&direction) {
                continue;
            }
            let pad = b
                .routing
                .net(pin.row, pin.col, &format!("PAD_{}", pin.pio))?;
            match tt.enum_value(tc, &format!("PIO{}.PULLMODE", pin.pio)) {
                Some("UP") => pulls.push((pad, Logic::One)),
                Some("DOWN") => pulls.push((pad, Logic::Zero)),
                _ => {}
            }
            if direction == "INPUT" || direction == "BIDIR" {
                let output = b
                    .routing
                    .net(pin.row, pin.col, &format!("JPADDI{}_PIO", pin.pio))?;
                b.gates
                    .push((Function::Buffer, vec![Input::Net(pad)], vec![output]));
            }
            if direction == "OUTPUT" || direction == "BIDIR" {
                let data = b
                    .routing
                    .net(pin.row, pin.col, &format!("PADDO{}_PIO", pin.pio))?;
                let tristate = b
                    .routing
                    .net(pin.row, pin.col, &format!("PADDT{}_PIO", pin.pio))?;
                b.gates.push((
                    if tt.enum_value(tc, &format!("PIO{}.OPENDRAIN", pin.pio)) == Some("ON") {
                        Function::OpenDrain
                    } else {
                        Function::TriState
                    },
                    vec![Input::Pulldown(tristate), Input::Net(data)],
                    vec![pad],
                ));
            }
            port_specs.push((name.clone(), direction.to_string(), pad));
        }
        let mut simulation = Simulation::new(b.routing.wire_count());
        for (net, value) in b.constants {
            simulation.add_driver(net, value);
        }
        for (net, value) in pulls {
            simulation.add_pull(net, value);
        }
        for (function, inputs, outputs) in b.gates {
            simulation.add_gate(function, inputs, &outputs);
        }
        for register in b.registers {
            simulation.add_register(register);
        }
        let mut ports = BTreeMap::new();
        for (name, direction, net) in port_specs {
            let driver = simulation.add_driver(net, Logic::Z);
            ports.insert(
                name,
                Port {
                    direction,
                    net,
                    driver,
                },
            );
        }
        simulation.initialise()?;
        Ok(Self {
            routing: b.routing,
            simulation,
            ports,
        })
    }

    pub fn probe_net(&self, reference: &str) -> Result<usize> {
        if let Some(pin) = reference.strip_prefix("pin:") {
            return Ok(self
                .ports
                .get(pin)
                .with_context(|| format!("unknown package pin {pin}"))?
                .net);
        }
        let (tile, wire) = reference
            .split_once(':')
            .context("expected pin:G2 or R38C2:Q0")?;
        let (row, col) = crate::database::coordinates(tile)?;
        self.routing
            .find(row, col, wire)
            .with_context(|| format!("unknown wire {reference}"))
    }

    pub fn drive_pins(&mut self, values: &[(String, Logic)]) -> Result<()> {
        let mut updates = Vec::new();
        for (name, value) in values {
            let name = name.strip_prefix("pin:").unwrap_or(name);
            let port = self
                .ports
                .get(name)
                .with_context(|| format!("unknown package pin {name}"))?;
            ensure!(
                port.direction != "OUTPUT" || *value == Logic::Z,
                "pin {name} is configured as OUTPUT"
            );
            updates.push((port.driver, *value));
        }
        self.simulation.drive(&updates)
    }
}
