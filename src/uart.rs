//! A UART line at the edge of the board, 8N1: a receiver that decodes a
//! design's output from samples of its level, and a transmitter that lays out
//! the levels that send bytes to a design's input. Times are integer
//! nanoseconds. Bit k of a frame (0 the start bit, 1..=8 the data, LSB first,
//! 9 the stop bit) begins k·10⁹/baud ns after the frame, rounded down.
use std::collections::VecDeque;

/// Start of bit `k` of a frame, in ns after the frame.
fn bit(baud: u64, k: u64) -> u64 {
    k * 1_000_000_000 / baud
}

/// Middle of bit `k` of a frame, in ns after the frame.
fn centre(baud: u64, k: u64) -> u64 {
    (2 * k + 1) * 1_000_000_000 / (2 * baud)
}

/// Decodes frames from samples of a line, given in time order.
/// The line is assumed idle high before the first sample, so a low first
/// sample starts a frame. A low stop bit drops the byte as a framing error.
#[derive(Clone)]
pub struct Receiver {
    baud: u64,
    /// The level of the last sample, initially idle high.
    level: bool,
    /// Start of the frame being received and the next bit to sample.
    frame: Option<(u64, u64)>,
    byte: u8,
    /// Frames whose stop bit was low; their bytes are dropped.
    pub framing_errors: u64,
}

impl Receiver {
    pub fn new(baud: u64) -> Self {
        assert!(
            (1..=1_000_000_000).contains(&baud),
            "baud rate out of range"
        );
        Self {
            baud,
            level: true,
            frame: None,
            byte: 0,
            framing_errors: 0,
        }
    }
    /// The line is at `level` from `ns` on. Each bit is taken at the first
    /// sample at or after its middle, so samples must come at least every
    /// half bit. Returns the byte when its stop bit has been sampled.
    pub fn sample(&mut self, ns: u64, level: bool) -> Option<u8> {
        let previous = std::mem::replace(&mut self.level, level);
        let Some((start, k)) = self.frame else {
            // A falling edge starts a frame.
            if previous && !level {
                self.frame = Some((ns, 0));
                self.byte = 0;
            }
            return None;
        };
        if ns < start + centre(self.baud, k) {
            return None;
        }
        match k {
            // A start bit that is high again in its middle was a glitch.
            0 if level => self.frame = None,
            0..=8 => {
                if k > 0 {
                    self.byte |= u8::from(level) << (k - 1);
                }
                self.frame = Some((start, k + 1));
            }
            _ => {
                self.frame = None;
                if level {
                    return Some(self.byte);
                }
                self.framing_errors += 1;
            }
        }
        None
    }
}

/// Lays out the level changes that send queued bytes, one frame after
/// another.
pub struct Transmitter {
    baud: u64,
    queue: VecDeque<u8>,
    /// Level changes of the frame being sent, earliest first.
    changes: VecDeque<(u64, bool)>,
    /// When the line is free for the next start bit.
    free: u64,
}

impl Transmitter {
    pub fn new(baud: u64) -> Self {
        assert!(
            (1..=1_000_000_000).contains(&baud),
            "baud rate out of range"
        );
        Self {
            baud,
            queue: VecDeque::new(),
            changes: VecDeque::new(),
            free: 0,
        }
    }
    pub fn push(&mut self, byte: u8) {
        self.queue.push_back(byte);
    }
    /// The next level change, as (ns, level). A queued byte starts its frame
    /// at `earliest` or when the line is free, whichever is later.
    pub fn next(&mut self, earliest: u64) -> Option<(u64, bool)> {
        if self.changes.is_empty() {
            let byte = self.queue.pop_front()?;
            let start = self.free.max(earliest);
            for k in 0..10 {
                let level = match k {
                    0 => false,
                    9 => true,
                    _ => byte >> (k - 1) & 1 == 1,
                };
                self.changes.push_back((start + bit(self.baud, k), level));
            }
            self.free = start + bit(self.baud, 10);
        }
        self.changes.front().copied()
    }
    /// Takes the change `next` returned.
    pub fn pop(&mut self) -> Option<(u64, bool)> {
        self.changes.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line a transmitter drives from `start` ns on, sampled every `step` ns
    /// up to `end`.
    fn line(tx: &mut Transmitter, start: u64, step: u64, end: u64) -> Vec<(u64, bool)> {
        let mut level = true;
        let mut out = Vec::new();
        let mut t = 0;
        while t <= end {
            while let Some((_, l)) = tx.next(start).filter(|&(ns, _)| ns <= t) {
                level = l;
                tx.pop();
            }
            out.push((t, level));
            t += step;
        }
        out
    }

    #[test]
    fn a_frame_is_start_bit_data_lsb_first_and_stop_bit() {
        let mut tx = Transmitter::new(115_200);
        tx.push(0x41);
        let changes: Vec<_> = std::iter::from_fn(|| {
            let c = tx.next(1000)?;
            tx.pop();
            Some(c)
        })
        .collect();
        let levels: Vec<bool> = changes.iter().map(|&(_, l)| l).collect();
        assert_eq!(
            levels,
            [false, true, false, false, false, false, false, true, false, true]
        );
        // 8680.55 ns per bit, from the frame at 1000 ns.
        assert_eq!(changes[0].0, 1000);
        assert_eq!(changes[1].0, 1000 + 8680);
        assert_eq!(changes[9].0, 1000 + 78125);
    }

    #[test]
    fn frames_follow_back_to_back() {
        let mut tx = Transmitter::new(115_200);
        tx.push(1);
        tx.push(2);
        for _ in 0..10 {
            tx.next(0);
            tx.pop();
        }
        // The second start bit comes ten bits after the first frame.
        assert_eq!(tx.next(0), Some((86_805, false)));
    }

    #[test]
    fn the_receiver_decodes_what_the_transmitter_sends() {
        let mut tx = Transmitter::new(115_200);
        for &b in b"Hi\r\n\x00\xff" {
            tx.push(b);
        }
        let mut rx = Receiver::new(115_200);
        let bytes: Vec<u8> = line(&mut tx, 1000, 20, 600_000)
            .into_iter()
            .filter_map(|(t, l)| rx.sample(t, l))
            .collect();
        assert_eq!(bytes, b"Hi\r\n\x00\xff");
        assert_eq!(rx.framing_errors, 0);
    }

    #[test]
    fn a_short_low_pulse_is_not_a_start_bit() {
        let mut rx = Receiver::new(115_200);
        let mut out = Vec::new();
        for t in (0..200_000).step_by(20) {
            // Low for 1 µs only, at 10 µs.
            let level = !(10_000..11_000).contains(&t);
            out.extend(rx.sample(t, level));
        }
        assert!(out.is_empty());
        assert_eq!(rx.framing_errors, 0);
    }

    #[test]
    fn a_low_stop_bit_is_a_framing_error() {
        let mut rx = Receiver::new(115_200);
        let mut out = Vec::new();
        // A break: the line low for twelve bit times.
        for t in (0..200_000).step_by(20) {
            out.extend(rx.sample(t, !(10_000..114_000).contains(&t)));
        }
        assert!(out.is_empty());
        assert_eq!(rx.framing_errors, 1);
    }

    #[test]
    fn a_start_bit_at_the_first_sample_decodes() {
        let mut tx = Transmitter::new(115_200);
        tx.push(b'H');
        let mut rx = Receiver::new(115_200);
        let bytes: Vec<u8> = line(&mut tx, 0, 20, 100_000)
            .into_iter()
            .filter_map(|(t, l)| rx.sample(t, l))
            .collect();
        assert_eq!(bytes, b"H");
        assert_eq!(rx.framing_errors, 0);
    }

    #[test]
    fn back_to_back_frames_from_the_first_sample_decode_in_order() {
        let mut tx = Transmitter::new(115_200);
        for &b in b"Hello" {
            tx.push(b);
        }
        let mut rx = Receiver::new(115_200);
        let bytes: Vec<u8> = line(&mut tx, 0, 20, 500_000)
            .into_iter()
            .filter_map(|(t, l)| rx.sample(t, l))
            .collect();
        assert_eq!(bytes, b"Hello");
        assert_eq!(rx.framing_errors, 0);
    }

    #[test]
    fn a_line_low_from_the_first_sample_is_a_framing_error() {
        let mut rx = Receiver::new(115_200);
        let mut out = Vec::new();
        // A break from the first sample, with no preceding idle sample.
        for t in (0..200_000).step_by(20) {
            out.extend(rx.sample(t, t >= bit(115_200, 10)));
        }
        assert!(out.is_empty());
        assert_eq!(rx.framing_errors, 1);
    }
}
