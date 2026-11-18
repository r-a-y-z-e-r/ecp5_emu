//! The internal oscillator OSCG: a free-running clock at the base frequency
//! over the divider.
//!
//! The base frequency belongs to the chip, not to the configuration (HC06
//! measured 311.8 MHz on the ULX3S, 0.58% above the nominal 310 MHz), so the
//! plan supplies the period. On the board the oscillator already runs when
//! user logic starts, at an arbitrary phase; here the output is low at time 0
//! and rises after half a period. The ±0.1% wander seen on the board is not
//! modelled.
//!
//! Each oscillator keeps exactly one future edge in the queue and schedules
//! the next one when it fires, so long runs do not fill the queue.
use super::*;

pub struct OscillatorConfig {
    pub resource: ResourceId,
    pub output: SignalId,
    /// Half of the output period.
    pub half_period: Time,
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Oscillator {
    label: String,
    pub(super) driver: usize,
    pub(super) half_period: Time,
    timer: Option<usize>,
}

impl Builder {
    pub fn oscillator(&mut self, config: OscillatorConfig) -> Result<()> {
        self.validate_signal(config.output)?;
        ensure!(
            config.half_period > Time::ZERO,
            "oscillator period must be positive"
        );
        let key = self.device.model().resource(config.resource).key.clone();
        let label = format!("osc:{key}");
        self.label(&label)?;
        let driver = self.driver(
            config.output,
            label.clone(),
            Strength::Strong,
            Logic::Zero,
            false,
        );
        self.oscillators.push(Oscillator {
            label,
            driver,
            half_period: config.half_period,
            timer: None,
        });
        Ok(())
    }
}

impl Kernel {
    pub(super) fn initialize_oscillators(&mut self) -> Result<()> {
        for index in 0..self.oscillators.len() {
            let label = self.oscillators[index].label.clone();
            let timer = self.new_timer(&label)?.index;
            self.oscillators[index].timer = Some(timer);
            self.oscillator_timers.insert(timer, index);
            let (driver, half) = (
                self.oscillators[index].driver,
                self.oscillators[index].half_period,
            );
            self.schedule_generated(self.now.checked_add(half)?, driver, Logic::One, timer)?;
        }
        Ok(())
    }
    /// The oscillator driven by `timer`, if any.
    pub(super) fn oscillator_of(&self, timer: usize) -> Option<usize> {
        self.oscillator_timers.get(&timer).copied()
    }
    /// Schedules the edge that follows one to `value` that just fired.
    pub(super) fn next_oscillator_edge(&mut self, index: usize, value: Logic) -> Result<()> {
        let oscillator = &self.oscillators[index];
        let (driver, half) = (oscillator.driver, oscillator.half_period);
        let timer = oscillator
            .timer
            .with_context(|| format!("{} has no timer", oscillator.label))?;
        self.schedule_generated(self.now.checked_add(half)?, driver, value.invert(), timer)
    }
}
