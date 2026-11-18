//! HY04 DTR. Count offsets are not uniform from the falling edge, so active
//! conversion drives 00 until completion after deassertion. Never emits X.
//! A new assertion cancels the old completion; nothing queues or repeats
//! while high. The profile supplies the whole initial/final byte.
use super::*;

/// Median of 1995 isolated HY04 windows, measured from the falling edge:
/// (821 observer ticks - 2.5 synchronizer ticks) * 40 ns = 32.740 us.
/// This fixed policy does not reproduce the measured phase/width variation.
pub const DTR_CONVERSION_PS: u64 = 32_740_000;

#[cfg_attr(test, derive(serde::Serialize))]
pub struct DtrConfig {
    pub resource: ResourceId,
    pub start: Input,
    /// DTROUT0 through DTROUT7, least significant bit first.
    pub outputs: [SignalId; 8],
    pub code: u8,
}

#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Dtr {
    pub(super) config: DtrConfig,
    pub(super) previous: Logic,
    pub(super) drivers: [usize; 8],
    pub(super) timer: Option<TimerId>,
    label: String,
}

impl Dtr {
    /// Only C generation uses this copy; event diagnostics stay on the live kernel.
    pub(super) fn generation_snapshot(&self) -> Self {
        Self {
            config: DtrConfig {
                resource: self.config.resource,
                start: self.config.start,
                outputs: self.config.outputs,
                code: self.config.code,
            },
            previous: self.previous,
            drivers: self.drivers,
            timer: self.timer,
            label: String::new(),
        }
    }
}

impl Builder {
    pub fn dtr(&mut self, config: DtrConfig) -> Result<()> {
        self.validate_input(config.start)?;
        for output in config.outputs {
            self.validate_signal(output)?;
        }
        let label = format!("dtr:{}", self.device.model().resource(config.resource).key);
        self.label(&label)?;
        let index = self.dtrs.len();
        if let Some(signal) = config.start.signal() {
            self.signals[signal.index].dtrs.push(index);
        }
        let drivers = std::array::from_fn(|bit| {
            self.driver(
                config.outputs[bit],
                format!("{label}:{bit}"),
                Strength::Strong,
                Logic::from_bool(config.code & (1 << bit) != 0),
                false,
            )
        });
        self.dtrs.push(Dtr {
            config,
            previous: Logic::X,
            drivers,
            timer: None,
            label,
        });
        Ok(())
    }
}

impl Kernel {
    pub(super) fn initialize_dtrs(&mut self) -> Result<()> {
        for index in 0..self.dtrs.len() {
            let label = self.dtrs[index].label.clone();
            let timer = self.new_timer(&label)?;
            self.dtrs[index].timer = Some(timer);
        }
        self.dirty_dtrs.extend(0..self.dtrs.len());
        let updates = self.capture_dtrs(true)?;
        self.publish(updates, Phase::Initial)?;
        self.settle_inputs()
    }

    pub(super) fn capture_dtrs(&mut self, initializing: bool) -> Result<Vec<Update>> {
        let dirty = self.dirty_dtrs.take();
        let mut updates = Vec::new();
        for &index in &dirty {
            let dtr = &mut self.dtrs[index];
            let start = dtr.config.start.read(&self.signals);
            let previous = std::mem::replace(&mut dtr.previous, start);
            if !initializing && start == previous {
                continue;
            }
            let timer = dtr.timer.context("DTR has no completion timer")?;
            let drivers = dtr.drivers;
            let code = dtr.config.code;
            self.cancel_timer(timer)?;
            if initializing && start == Logic::Zero {
                // Initial drivers already hold the environment code; no edge.
                continue;
            }
            // Initial high and uncertain controls clear the bus and cancel work.
            // X/Z -> low cannot establish a conversion; known high -> low can.
            updates.extend(drivers.into_iter().map(|driver| Update {
                driver,
                value: Logic::Zero,
                reason: Reason::Static("DTR conversion/retrigger (HY04)"),
            }));
            if !initializing && previous == Logic::One && start == Logic::Zero {
                let complete = self.now.checked_add(Time::from_ps(DTR_CONVERSION_PS))?;
                for (bit, driver) in drivers.into_iter().enumerate() {
                    self.schedule_generated(
                        complete,
                        driver,
                        Logic::from_bool(code & (1 << bit) != 0),
                        timer.index,
                    )?;
                }
            }
        }
        self.dirty_dtrs.recycle(dirty);
        Ok(updates)
    }
}
