//! The changes of a design's wires as the map shows them.
//!
//! Every change the window learns of is kept with the moment it came: a new
//! value, or a wire the run says changed between two frames (a clock, an
//! even number of times). A change spreads from its net's driver to the
//! net's far end in `spread` seconds, the same for a short net and a long
//! one, whatever the zoom and however long a frame takes: it reaches the
//! point a share `u` of the net's length from the driver at `at + spread·u`.
//! Every point it has reached glows, from bright to out in `fade` seconds.
//! While running, repeated changes shorten that trail to 40% of their
//! observed interval, at least one 60 Hz frame and at most 0.3 seconds.
//! The bright travelling front then stands apart from the fading trail.
//! So each frame draws all the changes kept, each where its time has taken
//! it. Of each wire the last two changes are kept, in one record of a fixed
//! size (a wire's older changes have spread all the way, under the later
//! ones): taking a frame's changes in touches only the wires that changed,
//! and nothing has to be let go of later.
//! Running as fast as it goes, `spread` is nought: the wires that change
//! glow at once. Each change keeps the pace it came at: changing the rate
//! leaves the changes already made as they were.
use std::time::Instant;

/// How changes are shown, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// From the driver to the far end of the net.
    pub spread: f32,
    /// A point reached, from bright to out. While running this is the upper
    /// bound; each wire's observed interval can shorten it.
    pub fade: f32,
}

impl Motion {
    /// For a run paused (a step), at a set rate, or as fast as it goes.
    pub fn of(running: bool, rate: Option<f64>) -> Self {
        match (running, rate) {
            // A step: slow enough to follow.
            (false, _) => Motion {
                spread: 1.5,
                fade: 0.8,
            },
            // A change spreads within the half cycle before the next edge.
            // Quiet wires get the full trail; repeated changes shorten it.
            (true, Some(r)) => Motion {
                spread: (0.4 / r).min(1.5) as f32,
                fade: RUN_FADE,
            },
            (true, None) => Motion {
                spread: 0.0,
                fade: RUN_FADE,
            },
        }
    }
}

/// A change of a wire: when (on the window's clock), its value after, and
/// the pace it came at.
#[derive(Clone, Copy, Debug)]
struct Change {
    at: f64,
    value: u8,
    spread: f64,
    fade: f64,
}

impl Change {
    /// When it reaches the share `u` of the net's length.
    fn reaches(&self, u: f32) -> f64 {
        self.at + self.spread * u as f64
    }

    /// How much it glows at `u` now: from 1 when it reaches it to 0.
    fn glow(&self, u: f32, now: f64) -> f64 {
        let reached = self.reaches(u);
        if now < reached {
            0.0
        } else {
            (1.0 - (now - reached) / self.fade).max(0.0)
        }
    }

    /// When it is out everywhere.
    fn out(&self) -> f64 {
        self.at + self.spread + self.fade
    }
}

/// No change: its time.
pub const NONE: f32 = -1e30;

const RUN_FADE: f32 = 0.3;
/// Keep a fast change visible for at least one nominal 60 Hz frame.
const FRAME: f64 = 1.0 / 60.0;

/// How busy a wire is: it rises by `BUSY_STEP` with each change and decays
/// over `BUSY_TAU` seconds, so a wire changing in every one of the ~30
/// samples a second of a run as fast as it goes stays at 1, one changing
/// once a second near 0.03. A change at a fast rate (`FAST`: its spread
/// shorter than that, so many changes a second) glows by it (`glow_weight`):
/// from `BUSY_LEAST` for a quiet wire to fully for one changing in half the
/// samples or more (`BUSY_FULL`), so a full chip as fast as it goes shows
/// where the activity is instead of one orange slab. Every change still
/// glows, so it is still drawn lit. The GPU does the same (src/gpu_map.rs
/// `busy_of`).
pub const BUSY_STEP: f32 = 1.0 / 30.0;
pub const BUSY_TAU: f32 = 1.0;
pub const BUSY_LEAST: f32 = 0.2;
pub const BUSY_FULL: f32 = 0.5;
pub const FAST: f32 = 1e-3;

/// How much a change at a fast rate glows on a wire this busy.
pub fn glow_weight(busy: f32) -> f32 {
    BUSY_LEAST + (1.0 - BUSY_LEAST) * (busy / BUSY_FULL).clamp(0.0, 1.0)
}

/// What is kept of a wire: its last two changes, the older first (when each
/// came, how long it takes to spread and to fade; none: a time of `NONE`),
/// and four values: now, after the older, after the later, and before the
/// older. That is all a drawing needs: a change spreads within the half
/// cycle before the next edge, so by the time a wire changes again the
/// change before has spread all the way, and the one before that is
/// nowhere the last to have reached. Last, how busy the wire was at its
/// later change (f32 bits, `BUSY_STEP`). The GPU takes it as it is
/// (src/gpu_map.rs).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Record {
    pub changes: [[f32; 3]; 2],
    pub values: u32,
    pub busy: u32,
}

impl Record {
    /// A wire with value `v` and no change.
    fn new(v: u8) -> Self {
        Self {
            changes: [[NONE, 0.0, 1.0]; 2],
            values: u32::from(v) * 0x0101_0101,
            busy: 0,
        }
    }

    /// How busy the wire is at `now`: as it was at its later change,
    /// decayed since.
    fn busy_at(&self, now: f64) -> f32 {
        let last = self.changes[1][0];
        if last < -1e29 {
            return 0.0;
        }
        f32::from_bits(self.busy) * (-((now as f32 - last).max(0.0)) / BUSY_TAU).exp()
    }

    /// How much a change of this wire at a fast rate glows, from
    /// `BUSY_LEAST` for a quiet wire to 1 for a busy one.
    fn weight(&self, c: &Change, now: f64) -> f64 {
        if c.spread as f32 >= FAST {
            return 1.0;
        }
        f64::from(glow_weight(self.busy_at(now)))
    }

    fn value_byte(&self, k: u32) -> u8 {
        (self.values >> (8 * k)) as u8
    }

    /// The older change and the later, if any.
    fn two(&self) -> [Option<Change>; 2] {
        [0, 1].map(|k| {
            let [at, spread, fade] = self.changes[k];
            (at > -1e29).then(|| Change {
                at: at as f64,
                value: self.value_byte(1 + k as u32),
                spread: spread as f64,
                fade: fade as f64,
            })
        })
    }

    /// A change come in: the later becomes the older, and the value before
    /// it the value before them.
    fn push(&mut self, c: Change) {
        self.busy = (self.busy_at(c.at) + BUSY_STEP).min(1.0).to_bits();
        let [older, later] = self.two();
        let before = match (older, later) {
            (Some(_), _) => self.value_byte(1),
            (None, Some(_)) => self.value_byte(3),
            (None, None) => self.value_byte(0),
        };
        self.changes = [
            self.changes[1],
            [c.at as f32, c.spread as f32, c.fade as f32],
        ];
        self.values = u32::from(c.value)
            | u32::from(self.value_byte(2)) << 8
            | u32::from(c.value) << 16
            | u32::from(before) << 24;
    }

    /// Learn the trail from successive observations at the same run rate.
    /// Only running motions opt in with RUN_FADE; steps keep their longer
    /// trail. Existing fields carry the result unchanged to the GPU.
    fn take(&mut self, mut c: Change) {
        let [at, spread, fade] = self.changes[1];
        if c.fade == f64::from(RUN_FADE)
            && at > -1e29
            && spread == c.spread as f32
            && fade <= RUN_FADE
        {
            c.fade = (0.4 * (c.at - f64::from(at))).clamp(FRAME, c.fade);
            // The previous observation used an interval not yet known. Do
            // not let that longer trail cover this change's leading front.
            self.changes[1][2] = fade.min(c.fade as f32);
        }
        self.push(c);
    }
}

/// The changes of the wires of a design's overlay (by value index).
pub struct Changes {
    clock: Instant,
    pub motion: Motion,
    /// The values last taken.
    seen: Vec<u8>,
    /// What is kept of each wire.
    records: Vec<Record>,
    /// The wires that changed with the values last taken.
    found: Vec<usize>,
    /// When the last change is out everywhere, and when the last stops
    /// spreading.
    out: f64,
    spread: f64,
    /// Counts the changes of value: what a drawing of the values is of.
    revision: u64,
}

impl Default for Changes {
    fn default() -> Self {
        Self {
            clock: Instant::now(),
            motion: Motion::of(false, None),
            seen: Vec::new(),
            records: Vec::new(),
            found: Vec::new(),
            out: f64::NEG_INFINITY,
            spread: f64::NEG_INFINITY,
            revision: 0,
        }
    }
}

/// Calls `f` with each wire whose value differs from that seen, or that
/// `toggled` (empty, or one a wire) says changed. Eight wires at a time:
/// most are as they were, and a word compared tells.
pub fn changed(values: &[u8], seen: &[u8], toggled: &[u8], mut f: impl FnMut(usize)) {
    let n = values.len().min(seen.len());
    let toggled = if toggled.len() >= n { toggled } else { &[] };
    let word = |b: &[u8], k: usize| u64::from_ne_bytes(b[k..k + 8].try_into().unwrap());
    let mut i = 0;
    while i < n {
        if i + 8 <= n {
            let same = word(values, i) == word(seen, i);
            if same && (toggled.is_empty() || word(toggled, i) == 0) {
                i += 8;
                continue;
            }
            for k in i..i + 8 {
                if values[k] != seen[k] || toggled.get(k) == Some(&1) {
                    f(k);
                }
            }
            i += 8;
        } else {
            if values[i] != seen[i] || toggled.get(i) == Some(&1) {
                f(i);
            }
            i += 1;
        }
    }
}

/// The changes of the wires `found` (a run, in order) taken into their
/// records and values, which start at wire `first`; how many changed value.
fn take_in(
    found: &[usize],
    first: usize,
    records: &mut [Record],
    seen: &mut [u8],
    values: &[u8],
    change: impl Fn(u8) -> Change,
) -> u64 {
    let mut revisions = 0;
    for &i in found {
        let (v, k) = (values[i], i - first);
        if v != seen[k] {
            revisions += 1;
        }
        records[k].take(change(v));
        seen[k] = v;
    }
    revisions
}

impl Changes {
    /// Seconds on the window's clock.
    pub fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// Takes the values of the wires now, and those the run says changed
    /// since the last taken (`toggled`, 1 for each; may be empty): each
    /// that changed is a change from now.
    pub fn update(&mut self, values: &[u8], toggled: &[u8]) {
        self.update_at(values, toggled, self.now());
    }

    fn update_at(&mut self, values: &[u8], toggled: &[u8], now: f64) {
        self.found.clear();
        if self.seen.len() != values.len() {
            self.seen = values.to_vec();
            self.records = values.iter().map(|&v| Record::new(v)).collect();
            self.revision += 1;
            return;
        }
        let n = values.len();
        let toggled = if toggled.len() == n { toggled } else { &[] };
        let mut found = std::mem::take(&mut self.found);
        changed(values, &self.seen, toggled, |i| found.push(i));
        let (spread, fade) = (self.motion.spread as f64, self.motion.fade as f64);
        let change = move |value| Change {
            at: now,
            value,
            spread,
            fade,
        };
        if let Some(&i) = found.first() {
            let c = change(values[i]);
            self.out = self.out.max(c.out());
            self.spread = self.spread.max(c.reaches(1.0));
        }
        // A full chip changes a few hundred thousand wires a frame: they are
        // taken in by a few threads, each a run of them (found in order, so
        // the runs' records do not overlap).
        let parts = if found.len() >= 1 << 16 { 4 } else { 1 };
        let chunk = found.len().div_ceil(parts).max(1);
        self.revision += std::thread::scope(|scope| {
            let (mut records, mut seen) = (&mut self.records[..], &mut self.seen[..]);
            let mut base = 0;
            let mut parts = Vec::new();
            for run in found.chunks(chunk) {
                let end = run[run.len() - 1] + 1;
                let (r, rest) = std::mem::take(&mut records).split_at_mut(end - base);
                let (s, rest_seen) = std::mem::take(&mut seen).split_at_mut(end - base);
                (records, seen) = (rest, rest_seen);
                let first = base;
                let mut take = move || take_in(run, first, r, s, values, change);
                if parts.is_empty() && chunk == found.len() {
                    return take();
                }
                parts.push(scope.spawn(take));
                base = end;
            }
            parts
                .into_iter()
                .map(|p| p.join().expect("taking changes in"))
                .sum()
        });
        self.found = found;
    }

    /// The wires that changed with the values last taken.
    pub fn found(&self) -> &[usize] {
        &self.found
    }

    /// What is kept of each wire.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Whether a change is spreading or glowing anywhere.
    pub fn animating(&self) -> bool {
        self.now() <= self.out
    }

    /// Whether a change of a wire may still spread, so that the values the
    /// wires show differ along them.
    pub fn spreading(&self, now: f64) -> bool {
        now < self.spread
    }

    /// Counts the changes of value taken.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether a wire has a change in sight now.
    pub fn lit(&self, w: u32, now: f64) -> bool {
        self.records
            .get(w as usize)
            .is_some_and(|r| r.two().iter().flatten().any(|c| now <= c.out()))
    }

    /// How much a wire glows at `u`, the share of its net's length from the
    /// driver (0 at the driver, 1 at the far end): from 1 when a change
    /// reaches it to 0.
    pub fn glow(&self, w: u32, u: f32, now: f64) -> f32 {
        self.records.get(w as usize).map_or(0.0, |r| {
            r.two()
                .iter()
                .flatten()
                .map(|c| c.glow(u, now) * r.weight(c, now))
                .fold(0.0, f64::max) as f32
        })
    }

    /// How busy a wire is now (0 to 1, `BUSY_STEP`).
    pub fn busy(&self, w: u32, now: f64) -> f32 {
        self.records.get(w as usize).map_or(0.0, |r| r.busy_at(now))
    }

    /// The value a wire shows at `u`: that of the last change that has
    /// reached it, else the value before them.
    pub fn value(&self, w: u32, u: f32, now: f64) -> u8 {
        let Some(r) = self.records.get(w as usize) else {
            return 2;
        };
        match r.two() {
            [_, Some(c)] if now >= c.reaches(u) => c.value,
            [Some(c), _] if now >= c.reaches(u) => c.value,
            [None, None] => r.value_byte(0),
            _ => r.value_byte(3),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When the later change of wire `w` came.
    fn at(c: &Changes, w: usize) -> f64 {
        c.records[w].changes[1][0] as f64
    }

    #[test]
    fn a_change_spreads_then_fades() {
        let mut c = Changes {
            motion: Motion {
                spread: 1.0,
                fade: 0.5,
            },
            ..Default::default()
        };
        c.update(&[0], &[]);
        c.update(&[1], &[]);
        let t0 = at(&c, 0);
        // Half way: the front at the middle, bright; behind it fading (out
        // at the driver, reached half a second ago); ahead not reached.
        let now = t0 + 0.5;
        assert!((c.glow(0, 0.5, now) - 1.0).abs() < 1e-6);
        assert!((c.glow(0, 0.25, now) - 0.5).abs() < 1e-6);
        assert!(c.glow(0, 0.0, now) < 1e-6);
        assert_eq!(c.glow(0, 0.6, now), 0.0);
        assert_eq!(c.value(0, 0.4, now), 1);
        assert_eq!(c.value(0, 0.6, now), 0);
        // At the far end at 1 s, out half a second after.
        assert!((c.glow(0, 1.0, t0 + 1.0) - 1.0).abs() < 1e-6);
        assert!((c.glow(0, 1.0, t0 + 1.25) - 0.5).abs() < 1e-6);
        assert_eq!(c.glow(0, 1.0, t0 + 1.6), 0.0);
    }

    #[test]
    fn the_change_before_glows_on_under_the_next() {
        let mut c = Changes {
            motion: Motion {
                spread: 0.1,
                fade: 0.3,
            },
            ..Default::default()
        };
        c.update(&[0], &[]);
        c.update(&[1], &[]);
        let t0 = at(&c, 0);
        // A second change 0.125 s on (a clock at 4 cycles a second): the far
        // end, not reached by it yet, still glows from the first.
        c.records[0].push(Change {
            at: t0 + 0.125,
            value: 0,
            spread: 0.1,
            fade: 0.3,
        });
        let now = t0 + 0.15;
        assert!(c.glow(0, 1.0, now) > 0.8, "{}", c.glow(0, 1.0, now));
        assert!(c.glow(0, 0.1, now) > 0.9);
        assert_eq!(c.value(0, 0.1, now), 0);
        assert_eq!(c.value(0, 1.0, now), 1);
    }

    #[test]
    fn four_cycles_per_second_has_a_distinct_front_and_a_dark_interval() {
        // Both the board clock (125 ms between changes) and a clock with a
        // two-cycle period (250 ms) must visibly alternate at 4 cycles/s.
        for interval in [0.125, 0.25] {
            let mut c = Changes {
                motion: Motion::of(true, Some(4.0)),
                ..Default::default()
            };
            c.update_at(&[0], &[], 0.0);
            for k in 0..4 {
                c.update_at(&[(k % 2) as u8], &[1], 1.0 + k as f64 * interval);
            }
            let t = at(&c, 0);
            let fade = f64::from(c.records()[0].changes[1][2]);
            assert!((fade - 0.4 * interval).abs() < 1e-6);
            for u in [0.0, 0.5, 1.0] {
                let reaches = t + f64::from(c.motion.spread) * f64::from(u);
                assert!(c.glow(0, u, reaches + 1e-6) > 0.99);
                assert!(c.glow(0, u, reaches + 0.9 * interval) < 0.3);
            }
            // Near the far end: a bright front, a dark tail, and an end
            // the new value has not yet reached.
            let now = t + 0.09;
            assert!(c.glow(0, 0.9, now + 1e-6) > 0.99);
            assert!(c.glow(0, 0.0, now) < 0.3);
            assert_eq!(c.glow(0, 1.0, now), 0.0);
            assert_eq!(c.value(0, 0.9, now + 1e-6), 1);
            assert_eq!(c.value(0, 1.0, now), 0);
        }
    }

    #[test]
    fn fast_trails_shorten_only_for_repeated_changes_and_keep_one_frame() {
        let mut c = Changes {
            motion: Motion::of(true, Some(10_000.0)),
            ..Default::default()
        };
        c.update_at(&[0, 0], &[], 0.0);
        c.update_at(&[1, 1], &[], 1.0);
        c.update_at(&[0, 1], &[], 1.03);
        let frequent = c.records()[0];
        let quiet = c.records()[1];
        assert!((f64::from(frequent.changes[1][2]) - FRAME).abs() < 1e-6);
        assert_eq!(frequent.changes[0][2], frequent.changes[1][2]);
        assert_eq!(quiet.changes[1][2], RUN_FADE);
        assert_eq!(c.glow(0, 0.0, 1.05), 0.0);
        assert!(c.glow(1, 0.0, 1.05) > 0.0);
        c.update_at(&[0, 0], &[], 2.0);
        assert_eq!(c.records()[1].changes[1][2], RUN_FADE);
        assert_eq!(std::mem::size_of::<Record>(), 32);
    }

    #[test]
    fn steps_and_rate_changes_do_not_inherit_a_fast_trail() {
        let mut c = Changes {
            motion: Motion::of(true, Some(10_000.0)),
            ..Default::default()
        };
        c.update_at(&[0], &[], 0.0);
        c.update_at(&[1], &[], 1.0);
        c.update_at(&[0], &[], 1.03);
        c.motion = Motion::of(true, Some(4.0));
        c.update_at(&[1], &[], 1.04);
        assert_eq!(c.records()[0].changes[1][2], RUN_FADE);
        c.motion = Motion::of(false, None);
        c.update_at(&[0], &[], 1.05);
        assert_eq!(c.records()[0].changes[1][2], 0.8);
    }

    #[test]
    fn a_wire_that_changed_twice_between_frames_glows() {
        let mut c = Changes {
            motion: Motion::of(true, Some(100.0)),
            ..Default::default()
        };
        c.update(&[0, 0], &[]);
        c.update(&[0, 0], &[1, 0]);
        // 10 ms on, the change (spreading in 4 ms) has reached all of it.
        let now = c.now() + 0.01;
        assert!(c.glow(0, 0.5, now) > 0.9);
        assert_eq!(c.glow(1, 0.5, now), 0.0);
        assert_eq!(c.value(0, 0.5, now), 0);
    }

    #[test]
    fn a_wire_keeps_its_last_two_changes() {
        let mut c = Changes {
            motion: Motion {
                spread: 1.0,
                fade: 0.3,
            },
            ..Default::default()
        };
        c.update(&[0], &[]);
        assert_eq!(c.value(0, 0.5, c.now()), 0);
        // One change: before it reached, the value before it.
        c.update(&[1], &[]);
        let t1 = at(&c, 0);
        assert_eq!(c.value(0, 0.5, t1 + 0.1), 0);
        assert_eq!(c.value(0, 0.5, t1 + 0.6), 1);
        for v in [0, 1, 2] {
            c.update(&[v], &[]);
        }
        // The last two (to 1, then to 2), and the value before them, 0.
        let t = at(&c, 0);
        let r = c.records[0];
        assert_eq!(r.values, 2 | 1 << 8 | 2 << 16);
        assert!(r.changes[0][0] > -1e29 && r.changes[0][0] <= r.changes[1][0]);
        assert_eq!(c.value(0, 1.0, t - 1.0), 0);
        assert_eq!(c.value(0, 0.5, t + 0.6), 2);
        // Out everywhere a fade after the far end: nothing lit, the value
        // the last one's.
        assert!(c.lit(0, t + 1.01));
        assert!(!c.lit(0, t + 1.1));
        assert_eq!(c.glow(0, 1.0, t + 1.1), 0.0);
        assert_eq!(c.value(0, 1.0, t + 1.1), 2);
    }

    #[test]
    fn the_wires_changed_are_found_eight_at_a_time_and_one_by_one() {
        let seen = vec![0u8; 21];
        let mut values = seen.clone();
        let mut toggled = vec![0u8; 21];
        values[3] = 1;
        values[8] = 2;
        toggled[15] = 1;
        values[20] = 1;
        let mut found = Vec::new();
        changed(&values, &seen, &toggled, |i| found.push(i));
        assert_eq!(found, [3, 8, 15, 20]);
        found.clear();
        changed(&values, &seen, &[], |i| found.push(i));
        assert_eq!(found, [3, 8, 20]);
    }

    /// As fast as it goes, a wire changing in every sample glows fully and
    /// one that changed once dimly, but still glows; at a set rate every
    /// change glows fully, however often its wire changes.
    #[test]
    fn a_quiet_wire_glows_less_than_a_busy_one_at_full_speed() {
        let fast = Motion::of(true, None);
        let mut r = Record::new(0);
        let mut q = Record::new(0);
        let at = |k: u32| 10.0 + f64::from(k) / 30.0;
        for k in 0..60 {
            r.push(Change {
                at: at(k),
                value: (k % 2) as u8,
                spread: fast.spread as f64,
                fade: fast.fade as f64,
            });
        }
        q.push(Change {
            at: at(59),
            value: 1,
            spread: fast.spread as f64,
            fade: fast.fade as f64,
        });
        let now = at(59);
        let glow = |r: &Record| {
            r.two()
                .iter()
                .flatten()
                .map(|c| c.glow(0.0, now) * r.weight(c, now))
                .fold(0.0, f64::max)
        };
        assert!(glow(&r) > 0.95, "busy {}", glow(&r));
        let quiet = glow(&q);
        assert!(quiet > 0.0 && quiet < 0.5, "quiet {quiet}");
        // At 4 cycles a second the same change glows fully.
        let slow = Motion::of(true, Some(4.0));
        let mut s = Record::new(0);
        s.push(Change {
            at: now,
            value: 1,
            spread: slow.spread as f64,
            fade: slow.fade as f64,
        });
        let c = s.two()[1].unwrap();
        assert_eq!(s.weight(&c, now), 1.0);
    }

    /// Hundreds of thousands of changes a frame are taken in by several
    /// threads: each wire's record as if taken in one by one.
    #[test]
    fn many_changes_are_taken_in_as_few() {
        let n = 400_000;
        let before: Vec<u8> = (0..n).map(|i| (i % 3) as u8).collect();
        let after: Vec<u8> = (0..n)
            .map(|i| {
                if i % 5 == 0 {
                    before[i]
                } else {
                    (before[i] + 1) % 3
                }
            })
            .collect();
        let toggled: Vec<u8> = (0..n).map(|i| u8::from(i % 10 == 0)).collect();
        let mut many = Changes::default();
        many.update(&before, &[]);
        many.update(&after, &toggled);
        assert!(many.found().len() >= 1 << 16, "the threaded way");
        for i in 0..n {
            let mut one = Changes {
                clock: many.clock,
                ..Default::default()
            };
            if i % 997 != 0 {
                continue;
            }
            one.update(&before[i..=i], &[]);
            one.update(&after[i..=i], &toggled[i..=i]);
            assert_eq!(
                many.records()[i].values,
                one.records()[0].values,
                "wire {i}"
            );
            assert_eq!(
                many.records()[i].changes[1][0] > -1e29,
                one.records()[0].changes[1][0] > -1e29,
                "wire {i}"
            );
            assert_eq!(
                many.found().binary_search(&i).is_ok(),
                !one.found().is_empty(),
                "wire {i}"
            );
        }
        let changed = (0..n).filter(|&i| before[i] != after[i]).count() as u64;
        assert_eq!(many.revision(), 1 + changed);
    }

    #[test]
    fn a_change_keeps_the_pace_it_came_at() {
        let mut c = Changes {
            motion: Motion::of(true, None),
            ..Default::default()
        };
        c.update(&[0], &[]);
        c.update(&[1], &[]);
        let t0 = at(&c, 0);
        // At once everywhere, as fast as it went; then a slow pace is set:
        // the change made before stays as it was.
        c.motion = Motion {
            spread: 1.0,
            fade: 0.6,
        };
        c.update(&[1], &[]);
        let now = t0 + f64::from(Motion::of(true, None).fade) / 2.0;
        // Half faded, as fast as it went; and by how busy its wire is, a
        // change made as fast as it goes (`BUSY_STEP`).
        let r = c.records()[0];
        let weight = r.weight(&r.two()[1].unwrap(), now);
        assert!(weight < 1.0);
        assert!((c.glow(0, 1.0, now) - 0.5 * weight as f32).abs() < 1e-6);
        assert_eq!(c.value(0, 1.0, now), 1);
    }
}
