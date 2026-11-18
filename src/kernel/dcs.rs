//! DCSC clock selection (HC02). POS/NEG finish the old active edge, hold
//! high/low through TWO later active edges of the new source, then follow it.
//! A coincident new edge cannot consume a stage launched at the same time.
//! Bypass and the static table are immediate. Startup, rapid retargeting and
//! unmeasured constant-selection transitions use the policies in CLOCK_NETWORK.md.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub enum DcsMode {
    Pos,
    Neg,
    Clk0Low,
    Clk0High,
    Clk1Low,
    Clk1High,
    Clk0,
    Clk1,
    Low,
    High,
}

impl std::str::FromStr for DcsMode {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "POS" => Self::Pos,
            // Trellis gives NONE exactly the NEG bit pattern, not another mode.
            "NEG" | "NONE" => Self::Neg,
            "CLK0_LOW" => Self::Clk0Low,
            "CLK0_HIGH" => Self::Clk0High,
            "CLK1_LOW" => Self::Clk1Low,
            "CLK1_HIGH" => Self::Clk1High,
            "CLK0" => Self::Clk0,
            "CLK1" => Self::Clk1,
            "LOW" => Self::Low,
            "HIGH" => Self::High,
            _ => bail!("unsupported DCSMODE {value}"),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Clock(usize),
    Level(Logic),
}

impl DcsMode {
    fn target(self, sel: usize, bypass: bool) -> Target {
        if bypass || matches!(self, Self::Pos | Self::Neg) {
            return match sel {
                1 => Target::Clock(0),
                2 => Target::Clock(1),
                _ => Target::Level(Logic::from_bool(!bypass && self == Self::Pos)),
            };
        }
        match self {
            Self::Clk0 => Target::Clock(0),
            Self::Clk1 => Target::Clock(1),
            Self::Clk0Low | Self::Clk0High if sel == 1 => Target::Clock(0),
            Self::Clk1Low | Self::Clk1High if sel == 2 => Target::Clock(1),
            _ => Target::Level(Logic::from_bool(matches!(
                self,
                Self::High | Self::Clk0High | Self::Clk1High
            ))),
        }
    }
    fn hold(self) -> Logic {
        Logic::from_bool(self == Self::Pos)
    }
    fn edge(self) -> Edge {
        if self == Self::Pos {
            Edge::Rising
        } else {
            Edge::Falling
        }
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct DcsConfig {
    pub resource: ResourceId,
    pub mode: DcsMode,
    pub clocks: [Input; 2],
    /// SEL0, SEL1, least significant first.
    pub select: [Input; 2],
    pub mode_select: Input,
    pub output: SignalId,
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) enum State {
    Initial,
    Unknown,
    Follow(usize),
    Held,
    WaitOld {
        old: usize,
        new: Option<usize>,
    },
    WaitNew {
        new: usize,
        remaining: u8,
        after: Time,
    },
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Dcs {
    pub(super) config: DcsConfig,
    pub(super) previous: [Logic; 2],
    pub(super) control: Option<[Logic; 3]>,
    pub(super) state: State,
    pub(super) driver: usize,
}

impl Builder {
    pub fn dcs(&mut self, config: DcsConfig) -> Result<()> {
        self.validate_signal(config.output)?;
        let label = format!("dcs:{}", self.device.model().resource(config.resource).key);
        self.label(&label)?;
        let index = self.dcs.len();
        for input in config
            .clocks
            .into_iter()
            .chain(config.select)
            .chain([config.mode_select])
        {
            self.validate_input(input)?;
            if let Some(s) = input.signal() {
                if !self.signals[s.index].dcs.contains(&index) {
                    self.signals[s.index].dcs.push(index);
                }
            }
        }
        let driver = self.driver(config.output, label, Strength::Strong, Logic::X, false);
        self.dcs.push(Dcs {
            config,
            previous: [Logic::X; 2],
            control: None,
            state: State::Initial,
            driver,
        });
        Ok(())
    }
}

impl Dcs {
    fn evaluate(&mut self, signals: &[Signal], now: Time, initializing: bool) -> Logic {
        let clocks = self.config.clocks.map(|c| c.read(signals));
        let control = [
            self.config.select[0].read(signals),
            self.config.select[1].read(signals),
            self.config.mode_select.read(signals),
        ];
        // Unknown controls retain a target only if every interpretation agrees.
        let mut targets = Vec::new();
        for bypass in [false, true] {
            for sel in 0..4 {
                if control[2].bit().is_some_and(|v| v != bypass)
                    || (0..2).any(|bit| {
                        control[bit]
                            .bit()
                            .is_some_and(|v| v != (sel & (1 << bit) != 0))
                    })
                {
                    continue;
                }
                targets.push(self.config.mode.target(sel, bypass));
            }
        }
        let target = targets
            .first()
            .copied()
            .filter(|t| targets.iter().all(|v| v == t));
        let protected =
            matches!(self.config.mode, DcsMode::Pos | DcsMode::Neg) && control[2] == Logic::Zero;
        let changed = self.control != Some(control);
        self.control = Some(control);
        let mut edges = [Activity::None; 2];
        if !initializing {
            for n in 0..2 {
                edges[n] = clock_edge(self.config.mode.edge(), self.previous[n], clocks[n]);
            }
        }
        self.previous = clocks;
        let Some(target) = target else {
            self.state = State::Unknown;
            return Logic::X;
        };
        if initializing || !protected {
            self.state = match target {
                Target::Clock(n) => State::Follow(n),
                Target::Level(_) => State::Held,
            };
            return match target {
                Target::Clock(n) => digital(clocks[n]),
                Target::Level(v) => v,
            };
        }
        if changed {
            let old = match self.state {
                State::Follow(n) | State::WaitOld { old: n, .. } => Some(n),
                _ => None,
            };
            self.state = match (old, target) {
                (Some(old), Target::Clock(new)) if old == new => State::Follow(old),
                (Some(old), Target::Clock(new)) => State::WaitOld {
                    old,
                    new: Some(new),
                },
                (Some(old), Target::Level(_)) => State::WaitOld { old, new: None },
                (None, Target::Clock(new)) => State::WaitNew {
                    new,
                    remaining: 2,
                    after: now,
                },
                (None, Target::Level(_)) => State::Held,
            };
        }
        match self.state {
            State::WaitOld { old, new } => match edges[old] {
                Activity::Definite => {
                    self.state = new.map_or(State::Held, |new| State::WaitNew {
                        new,
                        remaining: 2,
                        after: now,
                    });
                }
                Activity::Possible => self.state = State::Unknown,
                Activity::None => {}
            },
            State::WaitNew {
                new,
                remaining,
                after,
            } if now > after => match edges[new] {
                Activity::Definite => {
                    self.state = if remaining == 1 {
                        State::Follow(new)
                    } else {
                        State::WaitNew {
                            new,
                            remaining: remaining - 1,
                            after: now,
                        }
                    };
                }
                Activity::Possible => self.state = State::Unknown,
                Activity::None => {}
            },
            _ => {}
        }
        match self.state {
            State::Follow(n) | State::WaitOld { old: n, .. } => digital(clocks[n]),
            State::Held | State::WaitNew { .. } => self.config.mode.hold(),
            State::Initial | State::Unknown => Logic::X,
        }
    }
}

fn digital(value: Logic) -> Logic {
    value.bit().map_or(Logic::X, Logic::from_bool)
}

impl Kernel {
    pub(super) fn initialize_dcs(&mut self) -> Result<()> {
        self.dirty_dcs.extend(0..self.dcs.len());
        while !self.dirty_dcs.is_empty() {
            self.next_delta()?;
            let updates = self.capture_dcs(true);
            self.publish(updates, Phase::Initial)?;
            self.settle_inputs()?;
        }
        Ok(())
    }
    pub(super) fn capture_dcs(&mut self, initializing: bool) -> Vec<Update> {
        let dirty = self.dirty_dcs.take();
        let mut updates = Vec::new();
        for &index in &dirty {
            let dcs = &mut self.dcs[index];
            updates.push(Update {
                driver: dcs.driver,
                value: dcs.evaluate(&self.signals, self.now, initializing),
                reason: Reason::Static("DCS clock/control change (HC02)"),
            });
        }
        self.dirty_dcs.recycle(dirty);
        updates
    }
}
