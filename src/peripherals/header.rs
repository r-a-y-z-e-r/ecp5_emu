//! Ideal logic probes on J1/J2. No electrical drive strength or contention model.
use crate::board::{self, Direction};
use std::{collections::BTreeMap, collections::VecDeque};
const SECOND: u64 = 1_000_000_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub index: usize,
    pub direction: Direction,
    /// Resolved pad: 0, 1, X=2, Z=3.
    pub level: u8,
    pub hz: Option<f64>,
    /// Fraction of known time spent high, 0..1.
    pub duty: Option<f64>,
    pub transitions: u64,
    pub external: Option<bool>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub time_ps: u64,
    pub pins: Vec<Pin>,
}
#[derive(Clone)]
struct Probe {
    pin: Pin,
    changes: VecDeque<(u64, u8)>,
}
/// Change intervals retain the predecessor of the last-second window.
/// Cloning preserves levels, counters, external settings and measurement history.
#[derive(Clone)]
pub struct Header {
    time: u64,
    probes: Vec<Probe>,
}
impl Header {
    pub fn new(used: &BTreeMap<String, Direction>) -> Option<Self> {
        let probes: Vec<_> = board::HEADER
            .iter()
            .enumerate()
            .filter_map(|(index, p)| {
                used.get(p.ball).map(|&direction| Probe {
                    pin: Pin {
                        index,
                        direction,
                        level: 3,
                        hz: None,
                        duty: None,
                        transitions: 0,
                        external: None,
                    },
                    changes: VecDeque::new(),
                })
            })
            .collect();
        (!probes.is_empty()).then_some(Self { time: 0, probes })
    }
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.probes.iter().map(|p| p.pin.index)
    }
    pub fn set_level(&mut self, index: usize, level: Option<bool>) -> bool {
        if let Some(p) = self
            .probes
            .iter_mut()
            .find(|p| p.pin.index == index && p.pin.direction.receives())
        {
            p.pin.external = level;
            true
        } else {
            false
        }
    }
    pub fn levels(&self) -> Vec<Option<bool>> {
        let mut levels = vec![None; 56];
        for p in &self.probes {
            levels[p.pin.index] = p.pin.external;
        }
        levels
    }
    pub fn feed(&mut self, ps: u64, levels: impl Iterator<Item = u8>) {
        self.time = ps;
        let cutoff = ps.saturating_sub(SECOND);
        for (p, level) in self.probes.iter_mut().zip(levels) {
            if p.changes.is_empty() || p.pin.level != level {
                if !p.changes.is_empty() {
                    p.pin.transitions += 1;
                }
                p.pin.level = level;
                p.changes.push_back((ps, level));
            }
            while p.changes.get(1).is_some_and(|c| c.0 <= cutoff) {
                p.changes.pop_front();
            }
        }
    }
    pub fn status(&self) -> Status {
        Status {
            time_ps: self.time,
            pins: self
                .probes
                .iter()
                .map(|p| {
                    let mut pin = p.pin.clone();
                    if !pin.direction.drives() {
                        return pin;
                    }
                    let from = self
                        .time
                        .saturating_sub(SECOND)
                        .max(p.changes.front().map_or(self.time, |c| c.0));
                    let mut high = 0u64;
                    let mut unknown = false;
                    let mut rises = Vec::new();
                    for (i, &(at, level)) in p.changes.iter().enumerate() {
                        let end = p.changes.get(i + 1).map_or(self.time, |c| c.0);
                        let start = at.max(from);
                        if end > start {
                            if level == 1 {
                                high += end - start;
                            }
                            unknown |= level > 1;
                        }
                        if at >= from && i > 0 && level == 1 && p.changes[i - 1].1 == 0 {
                            rises.push(at);
                        }
                    }
                    if self.time > from && !unknown {
                        pin.duty = Some(high as f64 / (self.time - from) as f64);
                        pin.hz = if rises.len() >= 2 {
                            Some(
                                (rises.len() - 1) as f64 * SECOND as f64
                                    / (rises.last().unwrap() - rises[0]) as f64,
                            )
                        } else if p.changes.len() == 1 {
                            Some(0.0)
                        } else {
                            None
                        };
                    }
                    pin
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pwm_window_duty_unknown_and_snapshot() {
        let mut h = Header::new(&[("B11".into(), Direction::Output)].into()).unwrap();
        for cycle in 0..1500u64 {
            h.feed(cycle * 1_000_000_000, [1].into_iter());
            h.feed(cycle * 1_000_000_000 + 250_000_000, [0].into_iter());
        }
        h.feed(1_500_000_000_000, [1].into_iter());
        let s = h.status();
        assert!((s.pins[0].hz.unwrap() - 1000.0).abs() < 1e-6);
        assert!((s.pins[0].duty.unwrap() - 0.25).abs() < 1e-6);
        assert_eq!(s.pins[0].transitions, 3000);
        let saved = h.clone();
        h.feed(1_500_100_000_000, [2].into_iter());
        h.feed(1_500_200_000_000, [2].into_iter());
        assert!(h.status().pins[0].duty.is_none());
        h = saved;
        assert_eq!(h.status(), s);
    }
}
