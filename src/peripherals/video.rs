//! A digital DVI/HDMI receiver at the GPDI pads. Time is in picoseconds;
//! a row holds until the next row, including across feed calls.
use super::tmds::{data_byte, decode, Symbol};
use crate::fast::Trace;
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub width: usize,
    pub height: usize,
    pub htotal: usize,
    pub vtotal: usize,
    pub hsync_width: usize,
    pub vsync_width: usize,
    pub hsync_positive: bool,
    pub vsync_positive: bool,
    pub pixel_clock_hz: f64,
    pub refresh_hz: f64,
    pub regular: bool,
}

#[derive(Debug)]
pub struct Frame {
    pub number: u64,
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub crc32: u32,
    pub timing: Timing,
    pub end_ps: u64,
}

#[derive(Clone, Debug, Default)]
pub struct VideoStatus {
    pub locked: bool,
    pub pixel_clock_hz: f64,
    pub timing: Option<Timing>,
    pub last_frame: Option<Arc<Frame>>,
    pub frames_decoded: u64,
    pub invalid_symbols: u64,
    pub relocks: u64,
    pub covered_ps: u64,
    pub note: String,
}

#[derive(Default)]
struct Lane {
    word: u16,
    streak: [u8; 10],
    phase: Option<usize>,
    invalid: u8,
    queue: VecDeque<u16>,
}
impl Lane {
    fn bit(&mut self, bit: u8, n: u64) -> bool {
        self.word = self.word >> 1 | u16::from(bit) << 9;
        if n < 9 {
            return false;
        }
        let phase = (n % 10) as usize;
        if self.phase.is_none() {
            let symbol = decode(self.word);
            self.streak[phase] = if matches!(symbol, Symbol::Control(_)) {
                self.streak[phase].saturating_add(1)
            } else {
                0
            };
            if self.streak[phase] >= 8 {
                self.phase = Some(phase);
            }
        }
        if self.phase == Some(phase) {
            self.queue.push_back(self.word);
            return true;
        }
        false
    }
}

/// Complete held-level runs, in pixel clocks. Taking the longer observation
/// of each level excludes partial runs at acquisition and frame boundaries.
#[derive(Default)]
struct Sync {
    level: Option<bool>,
    start: u64,
    held: [u64; 2],
}
impl Sync {
    fn sample(&mut self, n: u64, level: bool) {
        if self.level != Some(level) {
            if let Some(old) = self.level {
                self.held[old as usize] = self.held[old as usize].max(n - self.start);
            }
            self.start = n;
            self.level = Some(level);
        }
    }
    fn pulse(&self) -> (u64, bool) {
        let positive = self.held[1] < self.held[0];
        (self.held[positive as usize], positive)
    }
}

pub struct VideoDecoder {
    status: VideoStatus,
    previous: Option<(u64, [u8; 4])>,
    rise: Option<u64>,
    periods: [u64; 8],
    stable: usize,
    period: u64,
    anchor: u64,
    cell: u64,
    next_sample: Option<u64>,
    bits: u64,
    lanes: [Lane; 3],
    acquired: bool,
    pixel: u64,
    hs: Sync,
    vs: Sync,
    preamble: u8,
    preamble_count: u8,
    island: bool,
    guards: u8,
    in_video: bool,
    last_video: Option<u64>,
    line_start: Option<u64>,
    line_width: usize,
    htotal: u64,
    frame_start: Option<u64>,
    frame_number: u64,
    complete_start: bool,
    widths: Vec<usize>,
    rgb: Vec<u8>,
}

impl Default for VideoDecoder {
    fn default() -> Self {
        Self::new()
    }
}
impl VideoDecoder {
    pub fn new() -> Self {
        Self {
            status: VideoStatus {
                note: "waiting for a stable clock".into(),
                ..VideoStatus::default()
            },
            previous: None,
            rise: None,
            periods: [0; 8],
            stable: 0,
            period: 0,
            anchor: 0,
            cell: 0,
            next_sample: None,
            bits: 0,
            lanes: std::array::from_fn(|_| Lane::default()),
            acquired: false,
            pixel: 0,
            hs: Sync::default(),
            vs: Sync::default(),
            preamble: 0,
            preamble_count: 0,
            island: false,
            guards: 0,
            in_video: false,
            last_video: None,
            line_start: None,
            line_width: 0,
            htotal: 0,
            frame_start: None,
            frame_number: 0,
            complete_start: false,
            widths: Vec::new(),
            rgb: Vec::new(),
        }
    }
    pub fn status(&self) -> VideoStatus {
        self.status.clone()
    }
    pub fn last_frame(&self) -> Option<Arc<Frame>> {
        self.status.last_frame.clone()
    }

    /// The trace has columns D0 (blue), D1 (green), D2 (red), CLK.
    pub fn feed(&mut self, trace: &Trace) {
        if trace.gap {
            self.gap();
        }
        if trace.width != 4 || trace.rows.len() != trace.times_ps.len() * 4 {
            self.lose("expected four GPDI columns");
            return;
        }
        let (rows, _) = trace.rows.as_chunks::<4>();
        for (&ps, &row) in trace.times_ps.iter().zip(rows) {
            self.sample(ps, row);
        }
    }
    pub fn gap(&mut self) {
        self.lose("gap in pad samples");
        self.previous = None;
    }
    fn lose(&mut self, note: &str) {
        if self.status.locked || self.acquired {
            self.status.relocks += 1;
        }
        self.status.locked = false;
        if self.status.note != note {
            self.status.note = note.into();
        }
        self.stable = 0;
        self.period = 0;
        self.rise = None;
        self.next_sample = None;
        self.bits = 0;
        self.lanes = std::array::from_fn(|_| Lane::default());
        self.acquired = false;
        self.complete_start = false;
        self.in_video = false;
        self.last_video = None;
        self.line_start = None;
        self.frame_start = None;
        self.line_width = 0;
        self.widths.clear();
        self.rgb.clear();
        self.hs = Sync::default();
        self.vs = Sync::default();
        self.island = false;
        self.guards = 0;
        self.preamble_count = 0;
    }
    /// Sample the held pads at every bit-cell centre reached by this row.
    #[inline]
    pub fn sample(&mut self, ps: u64, pads: [u8; 4]) {
        if let Some((last, held)) = self.previous {
            if ps <= last {
                self.gap();
            } else {
                self.status.covered_ps += ps - last;
                // A stopped clock must not extrapolate an unbounded bit train.
                if self.period != 0 && ps.saturating_sub(self.rise.unwrap_or(ps)) > 2 * self.period
                {
                    self.lose("clock stopped or samples missing");
                }
                while self.next_sample.is_some_and(|t| t < ps) {
                    self.take_bit(held);
                }
            }
        }
        if pads.iter().any(|&p| p > 1) {
            self.lose("unknown or undriven GPDI pad");
            self.previous = Some((ps, pads));
            return;
        }
        let rising = self.previous.is_some_and(|(_, p)| p[3] == 0) && pads[3] == 1;
        if rising {
            if let Some(last) = self.rise {
                let p = ps - last;
                if self.period != 0 && p.abs_diff(self.period) * 100 > self.period {
                    self.lose("pixel clock changed");
                }
                self.periods[self.stable % 8] = p;
                self.stable += 1;
                if self.stable >= 8 {
                    let min = *self.periods.iter().min().unwrap();
                    let max = *self.periods.iter().max().unwrap();
                    if min > 0 && (max - min) * 100 <= min {
                        self.period = self.periods.iter().sum::<u64>() / 8;
                        self.status.pixel_clock_hz = 1e12 / self.period as f64;
                        if !self.acquired && self.status.note != "searching for control symbols" {
                            self.status.note = "searching for control symbols".into();
                        }
                    }
                }
            }
            self.rise = Some(ps);
            if self.period != 0 {
                self.anchor = ps;
                self.cell = 0;
                self.next_sample = Some(ps + self.period / 20);
            }
        }
        while self.next_sample == Some(ps) {
            self.take_bit(pads);
        }
        self.previous = Some((ps, pads));
    }
    #[inline]
    fn take_bit(&mut self, pads: [u8; 4]) {
        let n = self.bits;
        self.bits += 1;
        let mut ready = false;
        for (lane, bit) in self.lanes.iter_mut().zip(pads) {
            ready |= lane.bit(bit, n);
        }
        self.cell += 1;
        self.next_sample = if self.cell < 10 {
            Some(self.anchor + (2 * self.cell + 1) * self.period / 20)
        } else {
            None
        };
        if !ready {
            return;
        }
        if !self.acquired && self.lanes.iter().all(|l| l.phase.is_some()) {
            self.acquired = true;
            self.status.locked = true;
            self.status.note.clear();
        }
        while self.lanes.iter().all(|l| !l.queue.is_empty()) {
            let words: [u16; 3] = std::array::from_fn(|i| self.lanes[i].queue[0]);
            let heads = words.map(decode);
            let controls = heads
                .iter()
                .filter(|s| matches!(s, Symbol::Control(_)))
                .count();
            // Acquisition can start each lane one word apart. Keep its first
            // data word until all lanes have reached the end of blanking.
            if controls > 0 && controls < 3 && !self.in_video && !self.island {
                for lane in &mut self.lanes {
                    if matches!(decode(lane.queue[0]), Symbol::Control(_)) {
                        lane.queue.pop_front();
                    }
                }
                continue;
            }
            for lane in &mut self.lanes {
                lane.queue.pop_front();
            }
            let ps = self.anchor + (self.cell.saturating_sub(1)) * self.period / 10;
            self.symbols(heads, words, ps);
            if !self.acquired {
                break;
            }
        }
        // An absent lane must not make an unbounded queue.
        if self.lanes.iter().any(|l| l.queue.len() > 32) {
            self.lose("lane alignment lost");
        }
    }
    fn symbols(&mut self, s: [Symbol; 3], words: [u16; 3], ps: u64) {
        self.pixel += 1;
        if let [Symbol::Control(c), Symbol::Control(a), Symbol::Control(b)] = s {
            self.end_line();
            self.island = false;
            self.guards = 0;
            self.hs.sample(self.pixel, c & 1 != 0);
            self.vs.sample(self.pixel, c & 2 != 0);
            let preamble = a | b << 2;
            self.preamble_count = if self.preamble == preamble {
                self.preamble_count.saturating_add(1)
            } else {
                1
            };
            self.preamble = preamble;
            for l in &mut self.lanes {
                l.invalid = 0;
            }
            return;
        }
        if self.preamble_count >= 8 {
            if self.preamble == 5 {
                self.island = true;
            }
            if self.preamble == 1 {
                self.guards = 2;
            }
        }
        self.preamble_count = 0;
        if self.island {
            return;
        }
        if self.guards != 0 {
            self.guards -= 1;
            if s != [Symbol::Guard; 3] {
                self.lose("invalid HDMI video guard band");
            }
            return;
        }
        if let [Some(b), Some(g), Some(r)] = words.map(data_byte) {
            for l in &mut self.lanes {
                l.invalid = 0;
            }
            if !self.in_video {
                self.start_line(ps);
            }
            self.in_video = true;
            self.last_video = Some(self.pixel);
            self.line_width += 1;
            self.rgb.extend_from_slice(&[r, g, b]);
            if self.rgb.len() > 64 * 1024 * 1024 {
                self.lose("video period exceeds buffer limit");
            }
        } else {
            self.complete_start = false;
            for (lane, symbol) in self.lanes.iter_mut().zip(s) {
                if symbol == Symbol::Invalid {
                    self.status.invalid_symbols += 1;
                    lane.invalid = lane.invalid.saturating_add(1);
                } else {
                    lane.invalid = 0;
                }
            }
            if self.lanes.iter().any(|l| l.invalid >= 16) {
                self.lose("invalid TMDS symbols");
            }
        }
    }
    fn end_line(&mut self) {
        if self.in_video {
            self.widths.push(self.line_width);
            self.line_width = 0;
        }
        self.in_video = false;
    }
    fn start_line(&mut self, ps: u64) {
        let vertical = self
            .last_video
            .is_some_and(|p| self.htotal > 0 && 2 * (self.pixel - p) > 3 * self.htotal);
        if vertical {
            if self.complete_start {
                self.finish_frame(ps);
            }
            if self.frame_start.is_some() {
                self.frame_number += 1;
            }
            self.frame_start = Some(self.pixel);
            self.complete_start = true;
            self.rgb.clear();
            self.widths.clear();
        } else {
            if let Some(last) = self.line_start {
                self.htotal = self.pixel - last;
            }
            if self.frame_start.is_none() {
                self.frame_start = Some(self.pixel);
            }
        }
        self.line_start = Some(self.pixel);
    }
    fn finish_frame(&mut self, ps: u64) {
        let Some(&width) = self.widths.first() else {
            return;
        };
        let height = self.widths.len();
        let clocks = self.pixel - self.frame_start.unwrap();
        let (hwidth, hp) = self.hs.pulse();
        let (vwidth, vp) = self.vs.pulse();
        let timing = Timing {
            width,
            height,
            htotal: self.htotal as usize,
            vtotal: (clocks / self.htotal) as usize,
            hsync_width: hwidth as usize,
            vsync_width: (vwidth / self.htotal) as usize,
            hsync_positive: hp,
            vsync_positive: vp,
            pixel_clock_hz: self.status.pixel_clock_hz,
            refresh_hz: self.status.pixel_clock_hz / clocks as f64,
            regular: self.widths.iter().all(|&w| w == width),
        };
        let rgb = std::mem::take(&mut self.rgb);
        let frame = Arc::new(Frame {
            number: self.frame_number,
            width,
            height,
            crc32: crc32(&rgb),
            rgb,
            timing: timing.clone(),
            end_ps: ps,
        });
        self.status.frames_decoded += 1;
        self.status.timing = Some(timing);
        self.status.last_frame = Some(frame);
    }
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut b = 0;
        while b < 8 {
            c = (c >> 1) ^ if c & 1 != 0 { 0xedb88320 } else { 0 };
            b += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}
const CRC: [u32; 256] = crc_table();
/// CRC-32/ISO-HDLC, over RGB bytes in raster order (the zlib convention).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0;
    for &b in bytes {
        c = c >> 8 ^ CRC[((c ^ u32::from(b)) & 255) as usize];
    }
    !c
}
