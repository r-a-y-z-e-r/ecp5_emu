//! ULX3S resistor DACs, from the pad levels and emulated picoseconds.
//! Voltages are schematic-derived predictions, never measurements. Nothing
//! is plugged into the jack: no load, filtering, playback or DAC glitches.
use crate::board;
use std::{collections::VecDeque, sync::Arc};

pub const VCC: f64 = 3.3;
pub const SAMPLE_HZ: u64 = 48_000;
pub const SCOPE_SAMPLES: usize = 2400;
const SECOND_PS: u64 = 1_000_000_000_000;
const SCOPE_PS: u64 = SECOND_PS / 20;

/// 0/1 are driven rails, 3 is Z (resistor disconnected), 2 is unknown.
pub type Pins = [[u8; 4]; 3];

/// Open-circuit voltage and source impedance. An X makes both unknown;
/// an entirely disconnected ladder has no defined voltage or impedance.
pub fn ladder(pins: [u8; 4]) -> (Option<f64>, Option<f64>) {
    let (mut high, mut total) = (0.0, 0.0);
    for (pin, ohms) in pins.into_iter().zip(board::AUDIO_OHMS) {
        match pin {
            0 | 1 => {
                total += 1.0 / ohms;
                high += f64::from(pin) / ohms;
            }
            3 => {}
            _ => return (None, None),
        }
    }
    if total == 0.0 {
        (None, None)
    } else {
        (Some(VCC * high / total), Some(1.0 / total))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelStatus {
    pub attached: bool,
    /// Known high bits, with separate masks for driven and unknown bits.
    pub code: u8,
    pub driven: u8,
    pub unknown: u8,
    pub volts: Option<f64>,
    pub source_ohms: Option<f64>,
    pub min_volts: Option<f64>,
    pub max_volts: Option<f64>,
    pub vpp: Option<f64>,
    pub hz: Option<f64>,
    /// Changes of the settled pad word, excluding the first observation.
    pub transitions: u64,
}
impl ChannelStatus {
    /// The four pad bits, most significant first, including X and Z.
    pub fn code_text(&self) -> String {
        (0..4)
            .rev()
            .map(|bit| {
                let mask = 1 << bit;
                if self.unknown & mask != 0 {
                    'x'
                } else if self.driven & mask == 0 {
                    'z'
                } else if self.code & mask != 0 {
                    '1'
                } else {
                    '0'
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeSample {
    pub ps: u64,
    pub volts: [Option<f32>; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub time_ps: u64,
    pub channels: [ChannelStatus; 3],
    /// Last 50 ms, at exactly 48 kHz in emulated time, with zero-order hold.
    pub scope: Arc<[ScopeSample]>,
    pub transitions: u64,
}

#[derive(Clone)]
struct Channel {
    attached: bool,
    pins: [u8; 4],
    volts: Option<f64>,
    ohms: Option<f64>,
    transitions: u64,
    // Piecewise-constant voltages. Keep the predecessor covering the cut,
    // so extrema and the time-weighted mean do not alias narrow pulses.
    history: VecDeque<(u64, Option<f64>)>,
}
impl Channel {
    fn new(attached: bool) -> Self {
        Self {
            attached,
            pins: [3; 4],
            volts: None,
            ohms: None,
            transitions: 0,
            history: VecDeque::new(),
        }
    }
    fn update(&mut self, ps: u64, pins: [u8; 4], first: bool) {
        if !self.attached || (!first && self.pins == pins) {
            return;
        }
        if !first {
            self.transitions += 1;
        }
        self.pins = pins;
        (self.volts, self.ohms) = ladder(pins);
        if self.history.back().is_none_or(|p| p.1 != self.volts) {
            self.history.push_back((ps, self.volts));
        }
        let cut = ps.saturating_sub(SECOND_PS);
        while self.history.get(1).is_some_and(|p| p.0 <= cut) {
            self.history.pop_front();
        }
    }
    fn status(&self, now: u64) -> ChannelStatus {
        let cut = now.saturating_sub(SECOND_PS);
        let mut min = None::<f64>;
        let mut max = None::<f64>;
        let (mut area, mut duration) = (0.0, 0u64);
        for (i, &(at, volts)) in self.history.iter().enumerate() {
            let end = self.history.get(i + 1).map_or(now, |p| p.0);
            if end < cut || (end == cut && at != now) {
                continue;
            }
            if let Some(v) = volts {
                min = Some(min.map_or(v, |a| a.min(v)));
                max = Some(max.map_or(v, |a| a.max(v)));
                let span = end.saturating_sub(at.max(cut));
                area += v * span as f64;
                duration += span;
            }
        }
        let vpp = min.zip(max).map(|(lo, hi)| hi - lo);
        let mean = if duration > 0 {
            area / duration as f64
        } else {
            self.volts.unwrap_or(0.0)
        };
        let mut hz = None;
        if let Some(vpp) = vpp.filter(|v| *v > 1e-6) {
            let (lo, hi) = (mean - vpp * 0.05, mean + vpp * 0.05);
            let (mut armed, mut first, mut last, mut crossings) = (false, 0, 0, 0u64);
            for (i, &(at, volts)) in self.history.iter().enumerate() {
                let end = self.history.get(i + 1).map_or(now, |p| p.0);
                if end < cut || (end == cut && at != now) {
                    continue;
                }
                match volts {
                    Some(v) if v <= lo => armed = true,
                    Some(v) if v >= hi && armed => {
                        if crossings == 0 {
                            first = at;
                        }
                        last = at;
                        crossings += 1;
                        armed = false;
                    }
                    None => {
                        armed = false;
                        first = 0;
                        last = 0;
                        crossings = 0;
                    }
                    _ => {}
                }
            }
            if crossings >= 2 && last > first {
                let period = (last - first) / (crossings - 1);
                // A stopped tone loses its estimate before its old extrema
                // leave the one-second window. Low tones get three periods.
                if now - last <= (period * 3).max(SECOND_PS / 20) {
                    hz = Some((crossings - 1) as f64 * SECOND_PS as f64 / (last - first) as f64);
                }
            }
        }
        let mask = |level: fn(u8) -> bool| {
            self.pins
                .iter()
                .enumerate()
                .fold(0u8, |m, (i, &p)| m | (u8::from(level(p)) << i))
        };
        ChannelStatus {
            attached: self.attached,
            code: mask(|p| p == 1),
            driven: mask(|p| p <= 1),
            unknown: mask(|p| p == 2),
            volts: self.volts,
            source_ohms: self.ohms,
            min_volts: min,
            max_volts: max,
            vpp,
            hz,
            transitions: self.transitions,
        }
    }
}

/// Cloning keeps the full analog history and resampling phase for replay.
#[derive(Clone)]
pub struct Audio {
    channels: [Channel; 3],
    now: u64,
    observed: bool,
    next_index: u64,
    next_ps: u128,
    scope: VecDeque<ScopeSample>,
}
impl Audio {
    pub fn new(attached: [bool; 3]) -> Self {
        Self {
            channels: attached.map(Channel::new),
            now: 0,
            observed: false,
            next_index: 0,
            next_ps: 0,
            scope: VecDeque::with_capacity(SCOPE_SAMPLES),
        }
    }
    fn seek_sample(&mut self, ps: u64) {
        self.next_index =
            (u128::from(ps) * u128::from(SAMPLE_HZ)).div_ceil(u128::from(SECOND_PS)) as u64;
        self.next_ps = u128::from(self.next_index) * u128::from(SECOND_PS) / u128::from(SAMPLE_HZ);
    }
    fn resample(&mut self) {
        if self.scope.len() == SCOPE_SAMPLES {
            self.scope.pop_front();
        }
        self.scope.push_back(ScopeSample {
            ps: self.next_ps as u64,
            volts: std::array::from_fn(|i| self.channels[i].volts.map(|v| v as f32)),
        });
        self.next_index += 1;
        self.next_ps = u128::from(self.next_index) * u128::from(SECOND_PS) / u128::from(SAMPLE_HZ);
    }
    /// Samples must be monotonic; restore a clone before replaying earlier time.
    /// Z disconnects a resistor. X makes that channel's voltage unknown.
    pub fn feed(&mut self, ps: u64, pins: Pins) {
        if self.observed && ps < self.now {
            return;
        }
        let first = !self.observed;
        if first {
            self.seek_sample(ps);
        } else if u128::from(ps.saturating_sub(SCOPE_PS)) > self.next_ps {
            self.seek_sample(ps.saturating_sub(SCOPE_PS));
        }
        // Old levels hold up to the new instant; a sample exactly on a
        // change belongs to the new levels, independently of chunk sizes.
        while self.next_ps < u128::from(ps) {
            self.resample();
        }
        for (channel, pins) in self.channels.iter_mut().zip(pins) {
            channel.update(ps, pins, first);
        }
        while self.next_ps <= u128::from(ps) {
            self.resample();
        }
        self.now = ps;
        self.observed = true;
    }
    pub fn status(&self) -> Status {
        let channels = std::array::from_fn(|i| self.channels[i].status(self.now));
        Status {
            time_ps: self.now,
            transitions: channels.iter().map(|c| c.transitions).sum(),
            channels,
            scope: self.scope.iter().copied().collect::<Vec<_>>().into(),
        }
    }
}
