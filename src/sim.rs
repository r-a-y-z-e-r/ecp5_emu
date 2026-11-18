use crate::logic::Logic;
use anyhow::{bail, ensure, Result};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub enum Input {
    Net(usize),
    Constant(Logic),
    Pulldown(usize),
}

impl Input {
    fn read(self, values: &[Logic]) -> Logic {
        match self {
            Self::Constant(v) => v,
            Self::Net(n) => values[n],
            Self::Pulldown(n) => {
                if values[n] == Logic::Z {
                    Logic::Zero
                } else {
                    values[n]
                }
            }
        }
    }
    fn net(self) -> Option<usize> {
        match self {
            Self::Net(n) | Self::Pulldown(n) => Some(n),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Function {
    Lut(u16),
    Carry { mask: u16, inject: bool },
    Mux,
    Buffer,
    And,
    TriState,
    OpenDrain,
}

struct Gate {
    function: Function,
    inputs: Vec<Input>,
    outputs: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct FlipFlop {
    pub clock: Input,
    pub reset: Input,
    pub enable: Input,
    pub data: Input,
    pub preload: Option<Input>,
    pub reset_value: Logic,
    pub invert_clock: bool,
    pub invert_reset: bool,
    pub invert_enable: bool,
    pub asynchronous: bool,
    pub output: usize,
    pub name: String,
}

struct Register {
    config: FlipFlop,
    driver: usize,
    previous_clock: Logic,
}

/// Four-state, zero-delay digital engine. Combinational logic settles before
/// registers sample, and all registers publish their new states together.
pub struct Simulation {
    values: Vec<Logic>,
    drivers: Vec<(usize, Logic, bool)>,
    net_drivers: Vec<Vec<usize>>,
    fanout: Vec<Vec<usize>>,
    register_fanout: Vec<Vec<usize>>,
    gates: Vec<Gate>,
    registers: Vec<Register>,
    dirty: BTreeSet<usize>,
    dirty_registers: BTreeSet<usize>,
    pub delta_limit: usize,
}

impl Simulation {
    pub fn new(nets: usize) -> Self {
        Self {
            values: vec![Logic::Z; nets],
            drivers: Vec::new(),
            net_drivers: vec![Vec::new(); nets],
            fanout: vec![Vec::new(); nets],
            register_fanout: vec![Vec::new(); nets],
            gates: Vec::new(),
            registers: Vec::new(),
            dirty: BTreeSet::new(),
            dirty_registers: BTreeSet::new(),
            delta_limit: 4096,
        }
    }

    pub fn add_driver(&mut self, net: usize, value: Logic) -> usize {
        let id = self.drivers.len();
        self.drivers.push((net, value, false));
        self.net_drivers[net].push(id);
        id
    }

    pub fn add_pull(&mut self, net: usize, value: Logic) {
        let driver = self.add_driver(net, value);
        self.drivers[driver].2 = true;
    }

    pub fn add_gate(&mut self, function: Function, inputs: Vec<Input>, outputs: &[usize]) {
        let id = self.gates.len();
        for input in &inputs {
            if let Some(net) = input.net() {
                self.fanout[net].push(id);
            }
        }
        let outputs = outputs
            .iter()
            .map(|&n| self.add_driver(n, Logic::X))
            .collect();
        self.gates.push(Gate {
            function,
            inputs,
            outputs,
        });
        self.dirty.insert(id);
    }

    pub fn add_register(&mut self, config: FlipFlop) {
        let id = self.registers.len();
        for input in [config.clock, config.reset]
            .into_iter()
            .chain(config.preload)
        {
            if let Some(net) = input.net() {
                self.register_fanout[net].push(id);
            }
        }
        let driver = self.add_driver(config.output, config.reset_value);
        self.registers.push(Register {
            config,
            driver,
            previous_clock: Logic::X,
        });
    }

    fn resolve_net(&mut self, net: usize) {
        let strong = Logic::resolve(
            self.net_drivers[net]
                .iter()
                .filter(|&&d| !self.drivers[d].2)
                .map(|&d| self.drivers[d].1),
        );
        let value = if strong == Logic::Z {
            Logic::resolve(
                self.net_drivers[net]
                    .iter()
                    .filter(|&&d| self.drivers[d].2)
                    .map(|&d| self.drivers[d].1),
            )
        } else {
            strong
        };
        if self.values[net] != value {
            self.values[net] = value;
            self.dirty.extend(&self.fanout[net]);
            self.dirty_registers.extend(&self.register_fanout[net]);
        }
    }

    pub fn initialise(&mut self) -> Result<()> {
        for net in 0..self.values.len() {
            if !self.net_drivers[net].is_empty() {
                self.resolve_net(net);
            }
        }
        self.combinational()?;
        let preload: Vec<_> = self
            .registers
            .iter()
            .filter_map(|r| r.config.preload.map(|i| (r.driver, i.read(&self.values))))
            .collect();
        self.publish(&preload);
        self.combinational()?;
        // Establish initial clock levels without manufacturing a clock edge.
        for reg in &mut self.registers {
            let clk = reg.config.clock.read(&self.values);
            reg.previous_clock = if reg.config.invert_clock {
                clk.invert()
            } else {
                clk
            };
        }
        self.dirty_registers.clear();
        self.dirty_registers.extend(0..self.registers.len());
        self.settle()
    }

    fn combinational(&mut self) -> Result<()> {
        for _ in 0..self.delta_limit {
            if self.dirty.is_empty() {
                return Ok(());
            }
            let current = std::mem::take(&mut self.dirty);
            let mut updates = Vec::new();
            for id in current {
                let gate = &self.gates[id];
                let v: Vec<_> = gate.inputs.iter().map(|i| i.read(&self.values)).collect();
                let outputs = match gate.function {
                    Function::Lut(mask) => vec![Logic::lut(mask, &v)],
                    Function::Carry { mask, inject } => {
                        let l4 = Logic::lut(mask, &v[..4]);
                        let l2 = if inject {
                            Logic::Zero
                        } else {
                            Logic::lut(mask & 0xf, &v[..2])
                        };
                        vec![
                            l4.xor(if inject { Logic::Zero } else { v[4] }),
                            l4.invert().and(l2).or(l4.and(v[4])),
                        ]
                    }
                    Function::Mux => vec![Logic::mux(v[0], v[1], v[2])],
                    Function::Buffer => vec![v[0]],
                    Function::And => vec![v[0].and(v[1])],
                    Function::TriState => vec![Logic::mux(v[0], v[1], Logic::Z)],
                    Function::OpenDrain => vec![Logic::mux(
                        v[0],
                        Logic::mux(v[1], Logic::Zero, Logic::Z),
                        Logic::Z,
                    )],
                };
                updates.extend(gate.outputs.iter().copied().zip(outputs));
            }
            self.publish(&updates);
        }
        bail!(
            "combinational logic did not settle after {} delta cycles (possible oscillator)",
            self.delta_limit
        )
    }

    fn publish(&mut self, updates: &[(usize, Logic)]) {
        let mut changed = BTreeSet::new();
        for &(driver, value) in updates {
            if self.drivers[driver].1 != value {
                self.drivers[driver].1 = value;
                changed.insert(self.drivers[driver].0);
            }
        }
        for net in changed {
            self.resolve_net(net);
        }
    }

    pub fn drive(&mut self, updates: &[(usize, Logic)]) -> Result<()> {
        for &(driver, _) in updates {
            ensure!(driver < self.drivers.len(), "unknown driver {driver}");
        }
        self.publish(updates);
        self.settle()
    }

    pub fn settle(&mut self) -> Result<()> {
        for _ in 0..self.delta_limit {
            self.combinational()?;
            if self.dirty_registers.is_empty() {
                return Ok(());
            }
            let current = std::mem::take(&mut self.dirty_registers);
            let mut updates = Vec::new();
            for id in current {
                let reg = &mut self.registers[id];
                let c = &reg.config;
                let mut clk = c.clock.read(&self.values);
                if c.invert_clock {
                    clk = clk.invert();
                }
                let edge = (reg.previous_clock == Logic::Zero && clk != Logic::Zero)
                    || (reg.previous_clock.bit().is_none() && clk == Logic::One);
                reg.previous_clock = clk;
                let mut reset = c.reset.read(&self.values);
                if c.invert_reset {
                    reset = reset.invert();
                }
                let reset_value = c.preload.map_or(c.reset_value, |i| i.read(&self.values));
                if !(edge || c.asynchronous && reset == Logic::One) {
                    continue;
                }
                let old = self.drivers[reg.driver].1;
                let mut enable = c.enable.read(&self.values);
                if c.invert_enable {
                    enable = enable.invert();
                }
                let captured = Logic::mux(enable, old, c.data.read(&self.values));
                let next = if c.asynchronous && reset == Logic::One {
                    reset_value
                } else {
                    Logic::mux(reset, captured, reset_value)
                };
                updates.push((reg.driver, next));
            }
            self.publish(&updates);
        }
        bail!(
            "sequential logic did not settle after {} delta cycles",
            self.delta_limit
        )
    }

    pub fn value(&self, net: usize) -> Logic {
        self.values[net]
    }
    pub fn gate_count(&self) -> usize {
        self.gates.len()
    }
    pub fn register_count(&self) -> usize {
        self.registers.len()
    }
    pub fn driver_count(&self, net: usize) -> usize {
        self.net_drivers[net].len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registers_sample_old_values_on_the_same_edge() {
        let mut sim = Simulation::new(4);
        let clk = sim.add_driver(0, Logic::Zero);
        let data = sim.add_driver(1, Logic::One);
        for (d, q) in [(1, 2), (2, 3)] {
            sim.add_register(FlipFlop {
                clock: Input::Net(0),
                reset: Input::Constant(Logic::Zero),
                enable: Input::Constant(Logic::One),
                data: Input::Net(d),
                preload: None,
                reset_value: Logic::Zero,
                invert_clock: false,
                invert_reset: false,
                invert_enable: false,
                asynchronous: false,
                output: q,
                name: format!("q{q}"),
            });
        }
        sim.initialise().unwrap();
        sim.drive(&[(clk, Logic::One)]).unwrap();
        assert_eq!((sim.value(2), sim.value(3)), (Logic::One, Logic::Zero));
        sim.drive(&[(clk, Logic::Zero), (data, Logic::Zero)])
            .unwrap();
        sim.drive(&[(clk, Logic::One)]).unwrap();
        assert_eq!((sim.value(2), sim.value(3)), (Logic::Zero, Logic::One));
    }
    #[test]
    fn reports_an_oscillator_instead_of_hanging() {
        let mut sim = Simulation::new(1);
        sim.add_gate(Function::Lut(0x5555), vec![Input::Net(0)], &[0]);
        sim.drivers[0].1 = Logic::Zero;
        sim.delta_limit = 8;
        assert!(sim.initialise().is_err());
    }
    #[test]
    fn resolves_contention_and_release() {
        let mut sim = Simulation::new(1);
        let a = sim.add_driver(0, Logic::Zero);
        let b = sim.add_driver(0, Logic::One);
        sim.initialise().unwrap();
        assert_eq!(sim.value(0), Logic::X);
        sim.drive(&[(b, Logic::Z)]).unwrap();
        assert_eq!(sim.value(0), Logic::Zero);
        sim.drive(&[(a, Logic::Z)]).unwrap();
        assert_eq!(sim.value(0), Logic::Z);
    }

    #[test]
    fn external_input_overrides_a_weak_pull() {
        let mut sim = Simulation::new(1);
        sim.add_pull(0, Logic::One);
        let external = sim.add_driver(0, Logic::Z);
        sim.initialise().unwrap();
        assert_eq!(sim.value(0), Logic::One);
        sim.drive(&[(external, Logic::Zero)]).unwrap();
        assert_eq!(sim.value(0), Logic::Zero);
        sim.drive(&[(external, Logic::Z)]).unwrap();
        assert_eq!(sim.value(0), Logic::One);
    }

    #[test]
    fn two_bit_ccu2_matches_integer_addition() {
        let mut sim = Simulation::new(9);
        let drivers: Vec<_> = (0..5).map(|n| sim.add_driver(n, Logic::Zero)).collect();
        // Yosys arith_map_ccu2c uses 0x96aa with C=0, D=1 for addition.
        sim.add_gate(
            Function::Carry {
                mask: 0x96aa,
                inject: false,
            },
            vec![
                Input::Net(0),
                Input::Net(2),
                Input::Constant(Logic::Zero),
                Input::Constant(Logic::One),
                Input::Net(4),
            ],
            &[5, 6],
        );
        sim.add_gate(
            Function::Carry {
                mask: 0x96aa,
                inject: false,
            },
            vec![
                Input::Net(1),
                Input::Net(3),
                Input::Constant(Logic::Zero),
                Input::Constant(Logic::One),
                Input::Net(6),
            ],
            &[7, 8],
        );
        sim.initialise().unwrap();
        for a in 0..4_u32 {
            for b in 0..4_u32 {
                for cin in 0..2_u32 {
                    let bits = [a & 1, (a >> 1) & 1, b & 1, (b >> 1) & 1, cin];
                    let updates: Vec<_> = drivers
                        .iter()
                        .zip(bits)
                        .map(|(&d, v)| (d, Logic::from_bool(v != 0)))
                        .collect();
                    sim.drive(&updates).unwrap();
                    let actual = [5, 7, 8].iter().enumerate().fold(0, |sum, (i, &net)| {
                        sum | ((sim.value(net).bit().unwrap() as u32) << i)
                    });
                    assert_eq!(actual, a + b + cin, "{a} + {b} + {cin}");
                }
            }
        }
    }
}
