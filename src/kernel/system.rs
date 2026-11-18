//! Explicit logical wake phases and global strobes. This is not a POR,
//! programming transport, oscillator or physical START-clock timing model.
use super::*;
use crate::{model::topology::Location, state::SystemState};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GlobalReset {
    pub input: Input,
    pub value: Logic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPort {
    Step,
    Done,
    Goe,
    Gwdis,
    StartupGsr,
    UserGsr,
    Gsr,
    Ready,
}
impl SystemPort {
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "step" => Ok(Self::Step),
            "done" => Ok(Self::Done),
            "goe" => Ok(Self::Goe),
            "gwdis" => Ok(Self::Gwdis),
            "startup_gsr" => Ok(Self::StartupGsr),
            "user_gsr" => Ok(Self::UserGsr),
            "gsr" => Ok(Self::Gsr),
            "ready" => Ok(Self::Ready),
            _ => bail!("unknown system reference sys:{name}"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Done => "done",
            Self::Goe => "goe",
            Self::Gwdis => "gwdis",
            Self::StartupGsr => "startup_gsr",
            Self::UserGsr => "user_gsr",
            Self::Gsr => "gsr",
            Self::Ready => "ready",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum WakeSequence {
    Sequence4,
    Sequence21,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    /// None selects an already ready environment with unknown FF state.
    pub wake: Option<WakeSequence>,
    /// Effective active-high asynchronous user GSR, distinct from startup GSR.
    pub user_gsr: Input,
}
pub struct SystemHandles {
    pub signals: BTreeMap<SystemPort, SignalId>,
    pub inputs: BTreeMap<SystemPort, DriverId>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Controller {
    config: SystemConfig,
    signals: BTreeMap<SystemPort, SignalId>,
    outputs: BTreeMap<SystemPort, usize>,
    previous_step: Logic,
}

impl Builder {
    pub fn system_controls(&mut self, config: SystemConfig) -> Result<SystemHandles> {
        ensure!(self.system.is_none(), "system controller already exists");
        ensure!(
            self.state.system().is_none(),
            "cannot resume system state without its controller history"
        );
        self.validate_input(config.user_gsr)?;
        let mut signals = BTreeMap::new();
        let mut outputs = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        for port in [
            SystemPort::Step,
            SystemPort::Done,
            SystemPort::Goe,
            SystemPort::Gwdis,
            SystemPort::StartupGsr,
            SystemPort::UserGsr,
            SystemPort::Gsr,
            SystemPort::Ready,
        ] {
            if port == SystemPort::Step && config.wake.is_none() {
                continue;
            }
            let signal = self.signal(Wire {
                location: Location { row: 0, col: 0 },
                name: format!("$SYS.{}", port.name()),
            })?;
            signals.insert(port, signal);
            if port == SystemPort::Step {
                let driver = self.external_driver(
                    signal,
                    "system:phase-step",
                    Strength::Strong,
                    Logic::Zero,
                )?;
                inputs.insert(port, driver);
                self.signals[signal.index].system = true;
            } else {
                let label = format!("system:{}", port.name());
                self.label(&label)?;
                let value = if port == SystemPort::Done {
                    if config.wake.is_some() {
                        Logic::Zero
                    } else {
                        Logic::One
                    }
                } else {
                    Logic::X
                };
                outputs.insert(
                    port,
                    self.driver(signal, label, Strength::Strong, value, false),
                );
            }
        }
        if config.wake == Some(WakeSequence::Sequence4) {
            let done = signals[&SystemPort::Done];
            let driver =
                self.external_driver(done, "system:external-DONE", Strength::Strong, Logic::Z)?;
            inputs.insert(SystemPort::Done, driver);
            self.static_driver(done, "system:DONE-pullup", Strength::Pull, Logic::One)?;
            self.signals[done.index].system = true;
        }
        if let Some(signal) = config.user_gsr.signal() {
            self.signals[signal.index].system = true;
        }
        self.state.set_system(SystemState {
            profile: if config.wake.is_some() {
                "wake-phases"
            } else {
                "ready-unknown"
            },
            phase: None,
            goe: Logic::X,
            gwdis: Logic::X,
            startup_gsr: Logic::X,
            user_gsr: Logic::X,
            gsr: Logic::X,
            done: Logic::X,
            ready: Logic::X,
            power_on_reset: "unmodelled; image already loaded",
        });
        self.system = Some(Controller {
            config,
            signals: signals.clone(),
            outputs,
            previous_step: Logic::Zero,
        });
        Ok(SystemHandles { signals, inputs })
    }
}

impl Kernel {
    /// The input of the GSR primitive, when the plan has a system controller:
    /// a constant, or the net that drives a user GSR from the fabric.
    pub(super) fn user_gsr(&self) -> Option<Input> {
        self.system.as_ref().map(|c| c.config.user_gsr)
    }
    /// The signal of a system controller output, if the plan has one.
    pub(super) fn system_signal(&self, port: SystemPort) -> Option<SignalId> {
        self.system.as_ref()?.signals.get(&port).copied()
    }
    pub(super) fn initialize_system_clock(&mut self) -> Result<()> {
        self.settle_combinational()?;
        if let Some(controller) = &mut self.system {
            if let Some(signal) = controller.signals.get(&SystemPort::Step) {
                let value = self.signals[signal.index].value;
                ensure!(
                    value.bit().is_some(),
                    "sys:step must start at a binary level"
                );
                controller.previous_step = value;
            }
        }
        Ok(())
    }
    pub(super) fn settle_inputs(&mut self) -> Result<()> {
        loop {
            self.settle_combinational()?;
            if !self.dirty_system {
                return Ok(());
            }
            self.dirty_system = false;
            if self.system.is_none() {
                continue;
            }
            self.next_delta()?;
            self.stats.system_evaluations += 1;
            let controller = self.system.as_mut().unwrap();
            let mut state = self.state.system().context("missing system state")?.clone();
            if let Some(signal) = controller.signals.get(&SystemPort::Step) {
                let step = self.signals[signal.index].value;
                ensure!(
                    step.bit().is_some(),
                    "sys:step must remain binary; uncertain phase progression is unsupported"
                );
                if clock_edge(Edge::Rising, controller.previous_step, step) == Activity::Definite {
                    state.phase = Some(state.phase.map_or(0, |p| (p + 1).min(3)));
                }
                controller.previous_step = step;
            }
            let wake = controller.config.wake;
            let released = |phase: u8| wake.is_none() || state.phase.is_some_and(|p| p >= phase);
            state.goe = Logic::from_bool(released(1));
            state.startup_gsr = Logic::from_bool(!released(2));
            state.gwdis = Logic::from_bool(!released(2));
            state.user_gsr = controller
                .config
                .user_gsr
                .read(&self.signals)
                .bit()
                .map(Logic::from_bool)
                .unwrap_or(Logic::X);
            state.gsr = state.startup_gsr.or(state.user_gsr);
            let done_phase = if wake == Some(WakeSequence::Sequence4) {
                0
            } else {
                3
            };
            let done_release = released(done_phase);
            let done_drive = if wake == Some(WakeSequence::Sequence4) && done_release {
                Logic::Z
            } else {
                Logic::from_bool(done_release)
            };
            state.done = if wake == Some(WakeSequence::Sequence4) {
                self.signals[controller.signals[&SystemPort::Done].index].value
            } else {
                done_drive
            };
            // DONE can delay completion; pulling it down after user mode does
            // not restart wake-up. X preserves the possibility of an earlier
            // completion until a known released DONE makes readiness certain.
            state.ready = if released(3) {
                if wake == Some(WakeSequence::Sequence4) {
                    state
                        .ready
                        .or(state.done.bit().map(Logic::from_bool).unwrap_or(Logic::X))
                } else {
                    Logic::One
                }
            } else {
                Logic::Zero
            };
            let mut updates = Vec::new();
            for (port, value) in [
                (SystemPort::Done, done_drive),
                (SystemPort::Goe, state.goe),
                (SystemPort::Gwdis, state.gwdis),
                (SystemPort::StartupGsr, state.startup_gsr),
                (SystemPort::UserGsr, state.user_gsr),
                (SystemPort::Gsr, state.gsr),
                (SystemPort::Ready, state.ready),
            ] {
                let driver = controller.outputs[&port];
                updates.push(Update {
                    driver,
                    value,
                    reason: Reason::Static("logical system strobes before resource capture"),
                });
            }
            self.state.set_system(state);
            self.publish(updates, Phase::Combinational)?;
        }
    }
}
