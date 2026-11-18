//! EHXPLLL driven by a periodic reference clock.
//!
//! Frequencies and phases follow the divider settings (the nextpnr/ecppll
//! relation; HC05 qualifies output feedback, HC08 qualifies raw-VCO feedback
//! for measured INT_OP/INT_OS plus fabric-return selector unions). The planner
//! supplies the VCO period ratio; output dividers apply in both cases. The
//! board also showed the PLL already locked when user logic starts,
//! so LOCK is high from the start. The reference period is learnt from its
//! first two rising edges, and the outputs stay low until then. A reference
//! edge off that period faults the kernel: relock is not modelled. The output
//! duty cycle is assumed to be 50%.
//!
//! RST and STDBY act only where PLLRST_ENA and STDBY_ENABLE enable them, and
//! they act alike (HC07). While one is high the outputs and LOCK are low.
//! After it falls the outputs restart on their old phase grid, at their first
//! rising edge after a short recovery delay for holds under 1 µs, or 2.2 µs
//! later after a longer one. The short delay depends on CLKI_DIV and feedback.
//! On the board the output then first runs at about
//! 60% of its rate and reaches it within about 0.4 ms; here it runs at its
//! rate. LOCK rises 98 304 phase-detector periods after the fall, plus the
//! feedback latency and an acquisition time that grows with the hold
//! ([`acquisition`]).
use super::*;

/// A hold this long stops the VCO: the outputs restart later (HC07).
const LONG_HOLD: Time = Time::from_ps(1_000_000);
/// Restart after a long hold. The observer saw the first output edge 51 to
/// 61 reference cycles of 40 ns after the fall, including its latency.
const LONG_RESTART: Time = Time::from_ps(2_200_000);
/// The fixed LOCK counter in phase-detector periods (HC07, HC07b). Recovery
/// adds a feedback latency in reference periods, separately in `pll_arm`.
const LOCK_COUNT: u64 = 98_304;
/// Acquisition time after a hold, both in ps: the medians over six PLLs of
/// how much later LOCK rose than after a short hold (HC07, HC07b; each PLL
/// within about 15% of them). Linear between the points, constant past the
/// last.
const ACQUISITION: [(u64, u64); 6] = [
    (320_000, 0),
    (640_000, 4_000_000),
    (1_280_000, 43_000_000),
    (4_000_000, 134_000_000),
    (10_240_000, 300_000_000),
    (40_000_000, 424_000_000),
];

/// How much later LOCK rises after a hold of `hold` ps than after a short one.
pub fn acquisition(hold: u64) -> u64 {
    let (first, last) = (ACQUISITION[0], ACQUISITION[ACQUISITION.len() - 1]);
    if hold <= first.0 {
        return first.1;
    }
    for pair in ACQUISITION.windows(2) {
        let ((w0, a0), (w1, a1)) = (pair[0], pair[1]);
        if hold <= w1 {
            return a0 + (u128::from(a1 - a0) * u128::from(hold - w0) / u128::from(w1 - w0)) as u64;
        }
    }
    last.1
}

/// Whole picoseconds in `time`, rounded down.
fn whole_ps(time: Time) -> u64 {
    let (numerator, denominator) = time.ratio();
    numerator / denominator
}

pub struct PllConfig {
    pub resource: ResourceId,
    /// CLKI after routing.
    pub reference: Input,
    /// VCO period over reference period: CLKI_DIV / (CLKFB_DIV × feedback divider).
    pub vco_per_reference: (u64, u64),
    pub outputs: Vec<PllOutput>,
    /// LOCK and INTLOCK.
    pub lock: Vec<(&'static str, SignalId)>,
    /// The RST and STDBY inputs their enables let act; any high holds the PLL.
    pub controls: Vec<Input>,
    /// CLKI_DIV: the phase detector runs at the reference rate over it.
    pub reference_divider: u64,
    /// The feedback output, or VCO_FEEDBACK for HC08 (no live controls).
    /// HC07 measures recovery for CLKOP/CLKOS3 only.
    pub feedback_output: &'static str,
}
impl PllConfig {
    /// Distinct from any output divider: HC08's measured raw-VCO return.
    pub const VCO_FEEDBACK: &'static str = "VCO";
}
#[derive(Clone, Copy)]
pub struct PllOutput {
    pub port: &'static str,
    pub signal: SignalId,
    pub divider: u64,
    /// Rising edge relative to the feedback output, in eighths of a VCO period.
    pub phase_eighths: i64,
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Pll {
    pub(super) label: String,
    pub(super) reference: Input,
    /// VCO period = reference period × a / b.
    pub(super) a: u64,
    pub(super) b: u64,
    /// (driver, divider, phase in eighths of a VCO period)
    pub(super) outputs: Vec<(usize, u64, i64)>,
    previous: Logic,
    pub(super) last_rise: Option<Time>,
    /// First reference edge with a known period; output edges are laid out from it.
    pub(super) anchor: Option<Time>,
    pub(super) period: Option<Time>,
    /// Reference periods scheduled since the anchor.
    windows: u64,
    timer: Option<usize>,
    /// The RST and STDBY inputs their enables let act.
    pub(super) controls: Vec<Input>,
    control_previous: Logic,
    reference_divider: u64,
    feedback_output: &'static str,
    locks: Vec<usize>,
    /// Since when the controls hold the PLL.
    held_since: Option<Time>,
    /// A fall whose restart waits for the reference period: (time, hold).
    pending_release: Option<(Time, Time)>,
    /// Outputs restart at their first rising edge at or after this time.
    restart: Option<Time>,
}

impl Builder {
    pub fn pll(&mut self, config: PllConfig) -> Result<()> {
        self.validate_input(config.reference)?;
        let (a, b) = config.vco_per_reference;
        ensure!(a > 0 && b > 0, "PLL frequency ratio must be positive");
        ensure!(
            config.reference_divider > 0,
            "PLL reference divider must be positive"
        );
        let key = self.device.model().resource(config.resource).key.clone();
        ensure!(config.feedback_output != PllConfig::VCO_FEEDBACK || config.controls.is_empty(),
            "PLL {key} FEEDBK_PATH raw-VCO, CLKFB source fabric: HC08 RST/STDBY recovery is unmeasured");
        let index = self.plls.len();
        if let Some(signal) = config.reference.signal() {
            self.signals[signal.index].plls.push(index);
        }
        for &control in &config.controls {
            self.validate_input(control)?;
            if let Some(signal) = control.signal() {
                if !self.signals[signal.index].plls.contains(&index) {
                    self.signals[signal.index].plls.push(index);
                }
            }
        }
        let mut outputs = Vec::new();
        for output in &config.outputs {
            self.validate_signal(output.signal)?;
            ensure!(output.divider > 0, "PLL output divider must be positive");
            let label = format!("pll:{key}:{}", output.port);
            self.label(&label)?;
            let driver = self.driver(output.signal, label, Strength::Strong, Logic::Zero, false);
            outputs.push((driver, output.divider, output.phase_eighths));
        }
        let mut locks = Vec::new();
        for &(port, signal) in &config.lock {
            self.validate_signal(signal)?;
            let label = format!("pll:{key}:{port}");
            self.label(&label)?;
            locks.push(self.driver(signal, label, Strength::Strong, Logic::One, false));
        }
        self.plls.push(Pll {
            label: format!("pll:{key}"),
            reference: config.reference,
            a,
            b,
            outputs,
            previous: Logic::X,
            last_rise: None,
            anchor: None,
            period: None,
            windows: 0,
            timer: None,
            controls: config.controls,
            control_previous: Logic::X,
            reference_divider: config.reference_divider,
            feedback_output: config.feedback_output,
            locks,
            held_since: None,
            pending_release: None,
            restart: None,
        });
        Ok(())
    }
}

impl Kernel {
    pub(super) fn initialize_plls(&mut self) -> Result<()> {
        for index in 0..self.plls.len() {
            let label = self.plls[index].label.clone();
            self.plls[index].timer = Some(self.new_timer(&label)?.index);
            self.plls[index].previous = self.plls[index].reference.read(&self.signals);
            self.pll_control(index)?;
        }
        Ok(())
    }
    /// Any control high holds the PLL; all low lets it run.
    fn pll_control_value(&self, index: usize) -> Logic {
        let mut value = Logic::Zero;
        for control in &self.plls[index].controls {
            match control.read(&self.signals) {
                Logic::One => return Logic::One,
                Logic::Zero => {}
                _ => value = Logic::X,
            }
        }
        value
    }
    fn pll_control(&mut self, index: usize) -> Result<()> {
        let value = self.pll_control_value(index);
        let previous = std::mem::replace(&mut self.plls[index].control_previous, value);
        match (previous, value) {
            (previous, value) if previous == value => Ok(()),
            (_, Logic::One) => self.pll_hold(index),
            (Logic::One, Logic::Zero) => self.pll_release(index),
            // A control that settles from unknown at the start.
            (Logic::X, Logic::Zero) => Ok(()),
            (previous, value) => bail!(
                "{} RST/STDBY is not binary: {previous} -> {value}",
                self.plls[index].label
            ),
        }
    }
    /// Outputs and LOCK fall now; pending edges and a pending LOCK are dropped.
    fn pll_hold(&mut self, index: usize) -> Result<()> {
        let now = self.now;
        let timer = self.plls[index].timer.context("PLL without a timer")?;
        self.timers[timer].generation = self.timers[timer]
            .generation
            .checked_add(1)
            .context("timer generation overflow")?;
        let pll = &mut self.plls[index];
        pll.held_since = Some(now);
        pll.pending_release = None;
        pll.restart = None;
        let drivers: Vec<usize> = pll
            .outputs
            .iter()
            .map(|&(driver, _, _)| driver)
            .chain(pll.locks.iter().copied())
            .collect();
        let at = now.checked_add(Time::from_ps(1))?;
        for driver in drivers {
            self.schedule_generated(at, driver, Logic::Zero, timer)?;
        }
        Ok(())
    }
    fn pll_release(&mut self, index: usize) -> Result<()> {
        let now = self.now;
        let pll = &mut self.plls[index];
        let since = pll
            .held_since
            .take()
            .context("PLL released without a hold")?;
        pll.pending_release = Some((now, now.checked_sub(since)?));
        self.pll_arm(index, true)
    }
    /// Once the reference period is known, sets the restart of a fall and
    /// schedules LOCK; `current` schedules the rest of the current window.
    fn pll_arm(&mut self, index: usize, current: bool) -> Result<()> {
        let pll = &mut self.plls[index];
        let (Some((release, hold)), Some(period), Some(timer)) =
            (pll.pending_release, pll.period, pll.timer)
        else {
            return Ok(());
        };
        pll.pending_release = None;
        // HC07b short holds: the old common delay lost 14 fewer edge counts
        // than the board with CLKOS3 at CLKI_DIV 5, and six fewer with CLKOP.
        // LOCK was one reference cycle early for CLKOS3 and three for CLKOP
        // at CLKI_DIV 1; CLKI_DIV 5 already matched. Keep the counter itself
        // in phase-detector periods and the measured latency in reference
        // periods. Other profiles retain the earlier unmeasured assumption.
        let (short_delay, lock_latency) = match (pll.reference_divider, pll.feedback_output) {
            (1, "CLKOS3") => ((1, 1), 6),
            (1, "CLKOP") => ((1, 1), 8),
            (5, "CLKOS3") => ((9, 2), 25),
            (5, "CLKOP") => ((2, 1), 25),
            (divider, _) => ((1, 1), 5 * divider),
        };
        let delay = if hold < LONG_HOLD {
            period.scaled(short_delay.0, short_delay.1)?
        } else {
            LONG_RESTART
        };
        pll.restart = Some(release.checked_add(delay)?);
        let lock = release
            .checked_add(Time::from_ps(acquisition(whole_ps(hold))))?
            .checked_add(period.checked_mul(pll.reference_divider * LOCK_COUNT + lock_latency)?)?;
        for driver in pll.locks.clone() {
            self.schedule_generated(lock, driver, Logic::One, timer)?;
        }
        if current && self.plls[index].anchor.is_some() {
            self.schedule_pll_window(index)?;
        }
        Ok(())
    }
    pub(super) fn evaluate_plls(&mut self) -> Result<()> {
        let dirty = self.dirty_plls.take();
        for &index in &dirty {
            self.pll_control(index)?;
            let pll = &mut self.plls[index];
            let value = pll.reference.read(&self.signals);
            let previous = std::mem::replace(&mut pll.previous, value);
            match clock_edge(Edge::Rising, previous, value) {
                Activity::None => {}
                Activity::Possible => {
                    bail!(
                        "{} reference is not a binary clock: {previous} -> {value}",
                        pll.label
                    )
                }
                Activity::Definite => self.pll_reference_edge(index)?,
            }
        }
        self.dirty_plls.recycle(dirty);
        Ok(())
    }
    fn pll_reference_edge(&mut self, index: usize) -> Result<()> {
        let now = self.now;
        let pll = &mut self.plls[index];
        match (pll.last_rise, pll.period) {
            (None, _) => {
                pll.last_rise = Some(now);
                return Ok(());
            }
            (Some(last), None) => {
                pll.period = Some(now.checked_sub(last)?);
                pll.anchor = Some(now);
                pll.last_rise = Some(now);
                self.pll_arm(index, false)?;
                return self.schedule_pll_window(index);
            }
            (Some(last), Some(period)) => {
                ensure!(
                    now == last.checked_add(period)?,
                    "{} reference period changed at {now} (expected {}); relock is not modelled",
                    pll.label,
                    last.checked_add(period)?
                );
                pll.windows += 1;
            }
        }
        pll.last_rise = Some(now);
        self.schedule_pll_window(index)
    }
    /// Output edges in (now, now + reference period], on a grid of
    /// reference period / (16 b): a VCO period is 16 a steps there, so phase
    /// eighths and half output periods fall on whole steps.
    fn schedule_pll_window(&mut self, index: usize) -> Result<()> {
        let now = self.now;
        let pll = &self.plls[index];
        let (Some(period), Some(anchor), Some(timer)) = (pll.period, pll.anchor, pll.timer) else {
            bail!("{} window without a reference period", pll.label);
        };
        // Held, or released before the period was known: no edges yet.
        if pll.held_since.is_some() || pll.pending_release.is_some() {
            return Ok(());
        }
        let step = period.scaled(1, 16 * pll.b)?;
        let window = 16 * pll.b;
        let start = pll.windows * window;
        let end = start + window;
        // Grid steps from the anchor to the restart, rounded up.
        let restart = match pll.restart {
            Some(restart) if restart > anchor => {
                let (dn, dd) = restart.checked_sub(anchor)?.ratio();
                let (sn, sd) = step.ratio();
                let (num, den) = (
                    u128::from(dn) * u128::from(sd),
                    u128::from(dd) * u128::from(sn),
                );
                u64::try_from(num.div_ceil(den)).context("PLL restart is too far")?
            }
            _ => 0,
        };
        let mut events = Vec::new();
        for &(driver, divider, phase) in &pll.outputs {
            let cycle = 16 * pll.a * divider;
            let offset = (2 * pll.a as i64 * phase).rem_euclid(cycle as i64) as u64;
            // An output restarts at its first rising edge at or after the restart.
            let gate = if restart > offset {
                offset + (restart - offset).div_ceil(cycle) * cycle
            } else {
                offset
            };
            for (first, value) in [(offset, Logic::One), (offset + cycle / 2, Logic::Zero)] {
                let mut position = if first > start {
                    first
                } else {
                    first + ((start - first) / cycle + 1) * cycle
                };
                while position <= end {
                    let time = anchor.checked_add(step.checked_mul(position)?)?;
                    if position >= gate && time > now {
                        events.push((time, driver, value));
                    }
                    position += cycle;
                }
            }
        }
        for (time, driver, value) in events {
            self.schedule_generated(time, driver, value, timer)?;
        }
        Ok(())
    }
}
