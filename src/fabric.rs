//! The whole fabric of the die for views: every wire and every arc (pip or
//! fixed connection) of every tile, from the Trellis tile types, each wire
//! drawn only between the things it connects and every piece of it
//! horizontal or vertical. A tile is `TILE_W` wide and `TILE_H` high.
//!
//! ```text
//! y/YS=0 ┌───────────────────────────────────────┬────────────────────────┐
//!        │ the clock's 16 branches of the row    │                        │
//!        │ horizontal channel: a lane for every  │ jumpers between two    │
//!        │ (wire, tap) of the H wires            │ horizontal wires       │
//!   0.25 ├────────┬──────────┬────────┬────────┬─┴────────────────────────┤
//!        │ local  │ vertical │ risers │through │ block: the sites          │
//!        │ wire   │ channel  │        │ lanes  │ (slices: LUTs, FFs, …;    │
//!        │ lanes  │          │        │        │ EBR, PIO …), their ports  │
//!        │        │          │        │        │ on the left edge          │
//!   1.0  │ ─ ─ ─ ─ ─ ─ ─ ─ lanes of links along the row ─ ─ ─ ─ ─ ─ ─ ─ ─ │
//!   1.25 └────────┴──────────┴────────┴────────┴──────────────────────────┘
//!       x=0     0.50       0.78     1.20     1.46                       3.2
//! ```
//!
//! Heights are drawn stretched by `YS`, so that the die is square. Every row
//! of the band lies on one grid `ROW_GAP` apart, and the lanes of links too;
//! the through lanes are `LANE_GAP` apart. No two wires side by side come
//! closer than they do in the plain logic tiles, anywhere.
//!
//! An inter-tile wire `<H|V><span><direction><track><tap>` is a conductor
//! that spans `span` tiles; at tap k it runs in the lane of (wire, k), and
//! just before the side it travels to it steps to the lane of (wire, k + 1),
//! as the conductors of the chip are staggered. A tile names a neighbour's
//! wire with a prefix (`N1_V02S0201` is `V02S0201` one tile north); that is
//! the same conductor at another tap here. The lanes are the same
//! everywhere, so conductors continue from tile to tile; a conductor is
//! drawn from its first arc to its last, across its tiles. Names for another
//! die of the family (`25K_…`, `45K_…`) are left out, `85K_` is dropped.
//!
//! The tiles at one position share their local wires. A local wire is an L:
//! a row from the block's edge to its lane, then up through the horizontal
//! channel. Rows higher up have lanes further left, so two local wires cross
//! exactly once, and every local wire crosses every lane of both channels.
//! A site's pin (`A0_SLICE`, `JADA0_EBR`) runs from the edge of its LUT,
//! flip-flop, control or site to the block's edge, where the local wire it
//! is fixed to takes its row; a pin with more than one arc runs on past the
//! edge. A site is drawn where the tile that names its ports' wires is (the
//! ports and their wires come from the device model's catalogue), so a site
//! whose wires only a neighbour names (a PLL, the DDRDLL) is drawn there.
//! An arc is a dot where its two wires cross; where they do not, a jumper:
//! between two horizontal wires a vertical piece above the block, between
//! two vertical wires a row of its own in the band, and between a wire and a
//! pin at another row a riser at the block's edge.
//!
//! Inside each slice of a logic tile the fabric also draws the connections
//! no tile type names: the carry from the slice's carry in through its two
//! LUTs to its carry out, each LUT into the slice's F5 mux, the M input past
//! the mux into each flip-flop, and the clock, reset and enable from the
//! controls into both flip-flops. Each is a wire of its own, named
//! `~FROM>TO` (`~LUT0>MUX`, `~CLK>FF1`), from one site part to another.
//!
//! A DSP block is drawn whole, under the interconnect of its row: its parts
//! (multipliers, pre-adders, adders, inner muxes) side by side in the blocks
//! of their columns, each CIB wire that feeds a port running on down its
//! own lane into a channel above the parts and along a lane of it to the
//! port, and every fixed connection between two ports a piece along a lane
//! of the channel below the parts, between stubs down from the ports; the
//! vertical conductors pass between the parts. The DSP rows share the height
//! the other rows leave of a square die, and the channels and parts spread
//! over it.
//!
//! An arc to a neighbour's wire (`E1_HFIE0000`) is a link: down a riser here
//! to the lanes of this row, along them, and up a riser of its own to that
//! wire; to another row, through a lane of the target's column kept for
//! that. A neighbour's wire that no tile there names itself (`S1W2_JPADDI`
//! of an ECLK tile, the pad's way into the clocks) is drawn as a local wire
//! of the position nearest to it of those that name it, and the others link
//! to it there. Arcs that would join a wire to nothing else (a wire whose
//! other arcs are on another die) are left out, and so are groups of wires
//! that reach no site, tile or link.
//!
//! The clock tree. Each row has 16 branches from each tap column, left and
//! right (`L_HPBX…`, `R_HPBX…` of the tap; `G_HPBX…` of every tile they
//! drive), in a band of their own at the top of its tiles, below the jogs of
//! the vertical tracks and above the first horizontal track; a dot where a
//! branch meets the local wire of a tile. Each tap column has 16 spines in
//! each quadrant (`G_VPTX…`), in through lanes kept for them, with a dot
//! where a spine meets the local wire of a tap tile. The rest of the tree,
//! conductors that several positions name (a quadrant's clocks from the
//! centre to its spines, `G_HPRX…` to `G_ULPCLK…`; the centre's own wires;
//! the PLLs' outputs), are links from each position to the next over a
//! spanning tree, as an arc to a neighbour's wire would be (`G_HPRX0000` to
//! `=G_HPRX0000`).
use crate::{database::Database, model::DeviceModel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::Read as _,
    path::{Path, PathBuf},
};

/// Width of a tile in die units: wide, so that the sites in its block, a DSP
/// block's parts above all, have room for their ports.
pub const TILE_W: f32 = 3.2;
/// The die is drawn square: the DSP rows are `DSP_H` high, and the other
/// rows share the height left, a tile's rows and lanes stretched from 1.25
/// by `YS` (for the LFE5U-85F's 127 columns and 93 other rows).
const DSP_H: f32 = 29.0;
const YS: f32 = (127.0 * TILE_W - 3.0 * DSP_H) / 93.0 / 1.25;
pub const TILE_H: f32 = 1.25 * YS;
pub const H_CHANNEL: f32 = 0.25 * YS;
const H_LANES: (f32, f32) = (0.024 * YS, 0.247 * YS);
pub const BAND_TOP: f32 = 0.255 * YS;
/// The lowest row of the band; the link lanes run below it.
pub const BAND_BOTTOM: f32 = YS;
/// Below the band, above where vertical conductors step to their next lane.
const LINK_LANES: (f32, f32) = (1.004 * YS, TILE_H - 0.021);
const LOCAL_LANES: (f32, f32) = (0.025, 0.50);
const V_LANES: (f32, f32) = (0.51, 0.775);
const RISERS: (f32, f32) = (0.785, 1.20);
/// Where links between rows cross the rows on their way.
const THROUGH: (f32, f32) = (1.205, 1.45);
pub const BLOCK: f32 = 1.46;
const H_JUMPERS: (f32, f32) = (BLOCK + 0.01, TILE_W - 0.025);
/// How far before the tile's edge a conductor at tap 0 steps to its next
/// lane; each tap further on steps `JOG_STEP` earlier, so the steps of one
/// wire's taps never touch. The last 0.02 at each edge holds only steps.
const JOG: f32 = 0.004;
const JOG_STEP: f32 = 0.003;
/// The sites of a position other than a logic tile: boxes stacked at the
/// block's right, their ports on the left edge; between the block's edge
/// and the boxes, the risers of the arcs between two ports.
const SITE_X: (f32, f32) = (BLOCK + 0.32, TILE_W - 0.012);
const INNER_RISERS: (f32, f32) = (BLOCK + 0.02, BLOCK + 0.30);
/// Rows of the band closer than this would read as one.
const ROW_GAP: f32 = 0.0011 * YS;
/// Lanes side by side across a tile, the through lanes, closer than this
/// would read as one: the vertical channel's own pitch.
const LANE_GAP: f32 = 0.0022;
/// The clock tree's channels. The branches of a row run in a band of their
/// own at the top of its tiles, LANE_GAP apart: below the vertical tracks'
/// jogs (down to JOG + 6 JOG_STEP) and above the first horizontal track.
/// A spine runs down through lanes of its tap column, right of those that
/// links take there.
const CLOCK_CHANNELS: u8 = 16;
const CLOCK_BAND: f32 = 0.0262;
const THROUGH_TAP: (f32, f32) = (
    THROUGH.0,
    THROUGH.1 - (CLOCK_CHANNELS as f32 + 0.5) * LANE_GAP,
);
fn branch_y(ch: u8) -> f32 {
    CLOCK_BAND + ch as f32 * LANE_GAP
}
fn spine_x(ch: u8) -> f32 {
    THROUGH.1 - (ch as f32 + 0.5) * LANE_GAP
}

mod arc_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S: Serializer, T: Serialize>(v: &Arc<T>, s: S) -> Result<S::Ok, S::Error> {
        (**v).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Arc<T>, D::Error> {
        T::deserialize(d).map(Arc::new)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Part {
    Track,
    Local,
    Jumper,
    /// From a site to the block's edge.
    Pin,
    /// The clock tree: a branch along a row, a spine down a column.
    Clock,
    /// A connection inside a slice that no tile type names: a LUT into its
    /// slice's F5 mux, the carry through the slice, the controls and the M
    /// input into a flip-flop.
    Internal,
}

/// A piece of a wire: its ends and what it is part of.
pub type Segment = ([f32; 2], [f32; 2], Part);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxKind {
    Slice,
    Lut,
    FlipFlop,
    /// The wide muxes of a slice (F5, FX).
    Mux,
    /// Clock, reset, enable and LUTRAM writes of a slice.
    Control,
    /// Any other site: an EBR, a multiplier, an I/O pad or its logic …
    Site,
}

/// A site drawn by a template: a slice or a part of one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SiteBox {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub kind: BoxKind,
    pub label: String,
}

/// The wires, arcs and sites of one position, in its tile.
#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub segments: Vec<([f32; 2], [f32; 2], Part)>,
    /// A dot per arc where its wires cross, one at each end of a jumper.
    pub dots: Vec<[f32; 2]>,
    pub boxes: Vec<SiteBox>,
    /// Where pins end on their sites.
    pub site_ends: Vec<[f32; 2]>,
    /// Where links from neighbours arrive, by the name of the wire.
    pub anchors: Vec<([f32; 2], String)>,
    /// Links that leave: the point here, the neighbour's offset, the name
    /// of its wire and the arc.
    pub links: Vec<([f32; 2], i16, i16, String, usize)>,
    /// Links (by arc) and arriving links (by wire) whose wire here has
    /// nothing else: they would end in nothing.
    weak_links: BTreeSet<usize>,
    weak_anchors: BTreeSet<String>,
    /// Pins of a slice that have no place on it.
    pub unplaced: Vec<String>,
    /// Arcs left out because they would join a wire to nothing else.
    pub left_out: usize,
    /// The height of its tile.
    pub height: f32,
    /// What each segment draws: a wire of `wires` or the jumper of an arc
    /// of `arcs`; and what each dot marks: an arc, or the wire a link
    /// arrives on.
    pub owners: Vec<Owner>,
    pub dot_owners: Vec<Owner>,
    /// The wires it draws, as the tile types name them.
    pub wires: Vec<WireKey>,
    /// The arcs of the position's tile types: (source, sink), and whether
    /// each is fixed.
    #[serde(with = "arc_serde")]
    pub arcs: std::sync::Arc<Vec<(String, String)>>,
    #[serde(with = "arc_serde")]
    pub fixed: std::sync::Arc<Vec<bool>>,
    /// The connections inside the slices of a logic tile: their wire in
    /// `wires` (named `~FROM>TO`), where their value comes from and where
    /// it goes, both on a site's edge.
    pub internals: Vec<(u32, [f32; 2], [f32; 2])>,
}

/// What a segment of a template draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owner {
    /// A wire, by its index in the template's `wires`.
    Wire(u32),
    /// The jumper of an arc, by its index in `arcs`.
    Arc(u32),
}

/// A wire of a template: a tap of a track here, or a name of the position's
/// tile types (with a neighbour's prefix for a site's port named so).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireKey {
    Track(TrackId, u8),
    Name(String),
}

/// An arc between a wire here and a neighbour's wire, as a polyline in die
/// units.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub points: Vec<[f32; 2]>,
    /// The position it leaves and its arc there.
    pub from: u32,
    pub arc: u32,
}

/// An inter-tile wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TrackId {
    pub horizontal: bool,
    pub span: u8,
    /// Travels west or north (or left, top).
    pub backwards: bool,
    pub track: u8,
    /// A wire of span 0 (`H00L0000`, `V00T0100`): to the next tile, as one
    /// of span 1 goes, but another conductor than the span-1 wire of the
    /// same number (`H01W0000`).
    pub zero: bool,
}

/// `H06E0103` → the wire and tap 3; none for any other name.
pub fn parse_track(name: &str) -> Option<(TrackId, u8)> {
    let b = name.as_bytes();
    if b.len() != 8 || !(b[0] == b'H' || b[0] == b'V') {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u8> { name.get(r)?.parse().ok() };
    let (span, track, tap) = (digits(1..3)?, digits(4..6)?, digits(6..8)?);
    let horizontal = b[0] == b'H';
    let backwards = match (horizontal, b[3]) {
        (true, b'E' | b'R') | (false, b'S' | b'B') => false,
        (true, b'W' | b'L') | (false, b'N' | b'T') => true,
        _ => return None,
    };
    let zero = span == 0;
    let span = span.max(1);
    (tap <= span).then_some((
        TrackId {
            horizontal,
            span,
            backwards,
            track,
            zero,
        },
        tap,
    ))
}

/// A neighbour prefix: `N1_X` is X one tile north; `S1E1_X` one south-east.
pub fn split_prefix(name: &str) -> (i16, i16, &str) {
    let Some((prefix, base)) = name.split_once('_') else {
        return (0, 0, name);
    };
    let bytes = prefix.as_bytes();
    if bytes.is_empty() {
        return (0, 0, name);
    }
    let (mut dr, mut dc) = (0, 0);
    let mut i = 0;
    while i < bytes.len() {
        let direction = bytes[i];
        let digits = bytes[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if !b"NSEW".contains(&direction) || digits == 0 {
            return (0, 0, name);
        }
        let n: i16 = prefix[i + 1..i + 1 + digits].parse().unwrap_or(0);
        match direction {
            b'N' => dr -= n,
            b'S' => dr += n,
            b'E' => dc += n,
            _ => dc -= n,
        }
        i += 1 + digits;
    }
    (dr, dc, base)
}

/// The die the fabric is drawn for: a tile type names wires that differ
/// between the dies of the family with the die's size first, `85K_S1_JC1`.
const DIE: &str = "85";

/// A name as this die has it: `85K_S1_JC1` is `S1_JC1`; the names for the
/// other dies (`25K_…`, `45K_…`) are none.
pub(crate) fn this_die(name: &str) -> Option<&str> {
    match name.split_once("K_") {
        Some((size, rest)) if !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) => {
            (size == DIE).then_some(rest)
        }
        _ => Some(name),
    }
}

/// The arcs of a tile type on this die, as (source, sink).
fn routing<'a>(db: &'a Database, kind: &str) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
    db.types[kind]
        .routing
        .iter()
        .filter_map(|a| Some((this_die(&a.source)?, this_die(&a.sink)?)))
}

/// The arcs of a tile type on this die, as (source, sink, fixed).
fn routing_kind<'a>(
    db: &'a Database,
    kind: &str,
) -> impl Iterator<Item = (&'a str, &'a str, bool)> + 'a {
    db.types[kind].routing.iter().filter_map(|a| {
        Some((
            this_die(&a.source)?,
            this_die(&a.sink)?,
            a.kind == crate::database::ArcKind::Fixed,
        ))
    })
}

pub(crate) fn step(id: &TrackId) -> (i16, i16) {
    match (id.horizontal, id.backwards) {
        (true, false) => (0, 1),
        (true, true) => (0, -1),
        (false, false) => (1, 0),
        (false, true) => (-1, 0),
    }
}

/// The tap of a track, as seen from a tile `(dr, dc)` away from where it is
/// named: the same conductor, further along or back.
fn tap_here(id: &TrackId, tap: u8, dr: i16, dc: i16) -> Option<u8> {
    let s = step(id);
    let t = tap as i16 - (dr * s.0 + dc * s.1);
    (0..=id.span as i16).contains(&t).then_some(t as u8)
}

/// The lanes of both channels: every (wire, tap) of every tile type.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Lanes {
    h: BTreeMap<(TrackId, u8), usize>,
    v: BTreeMap<(TrackId, u8), usize>,
}

impl Lanes {
    fn new(db: &Database) -> Self {
        let mut ids = BTreeSet::new();
        for kind in db.types.keys() {
            for (source, sink) in routing(db, kind) {
                for name in [source, sink] {
                    if let Some((id, _)) = parse_track(split_prefix(name).2) {
                        ids.insert(id);
                    }
                }
            }
        }
        let (mut h, mut v) = (BTreeMap::new(), BTreeMap::new());
        for id in ids {
            let map = if id.horizontal { &mut h } else { &mut v };
            for tap in 0..=id.span {
                let n = map.len();
                map.insert((id, tap), n);
            }
        }
        Self { h, v }
    }
    /// Position of a lane across its channel, in the tile.
    fn at(&self, id: &TrackId, tap: u8) -> f32 {
        let (map, range) = if id.horizontal {
            (&self.h, H_LANES)
        } else {
            (&self.v, V_LANES)
        };
        lane(range, map[&(*id, tap)], map.len())
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.h.len(), self.v.len())
    }
}

fn lane(range: (f32, f32), k: usize, n: usize) -> f32 {
    range.0 + (range.1 - range.0) * (k as f32 + 0.5) / n.max(1) as f32
}

/// The sites of a logic tile. A slice has two rows, each a LUT and its
/// flip-flop, and below them a strip with the wide muxes and the controls;
/// the carry runs down the right side, between the slices.
mod slices {
    use super::YS;
    use super::{BAND_BOTTOM, BAND_TOP, BLOCK, TILE_W};
    pub const ROW: f32 = 0.05 * YS;
    pub const STRIP: f32 = 0.06 * YS;
    pub const GAP: f32 = 0.009 * YS;
    pub const SLICE: f32 = 2.0 * ROW + STRIP + GAP;
    pub const TOP: f32 = BAND_TOP + 0.015 * YS;
    pub const X: (f32, f32) = (BLOCK + 0.015, TILE_W - 0.008);
    pub const LUT_X: (f32, f32) = (BLOCK + 0.06, BLOCK + 0.80);
    /// Where a LUT's output turns down to its row.
    pub const F_X: f32 = LUT_X.1 + 0.05;
    pub const FF_X: (f32, f32) = (BLOCK + 1.0, TILE_W - 0.12);
    pub const CARRY_X: f32 = TILE_W - 0.06;
    /// The row of the carry into the tile, above slice A.
    pub const FCI_Y: f32 = (BAND_TOP + TOP) / 2.0;
    pub fn slice_top(s: usize) -> f32 {
        TOP + s as f32 * SLICE
    }
    pub fn slice_bottom(s: usize) -> f32 {
        slice_top(s) + SLICE - GAP
    }
    pub fn row_top(n: usize) -> f32 {
        slice_top(n / 2) + (n % 2) as f32 * ROW
    }
    pub fn strip_top(s: usize) -> f32 {
        slice_top(s) + 2.0 * ROW
    }
    /// The row of the carry out of the tile, below slice D.
    pub fn fco_y() -> f32 {
        (slice_bottom(3) + BAND_BOTTOM) / 2.0
    }
    /// Between slice s and the next.
    pub fn gap(s: usize) -> f32 {
        (slice_bottom(s) + slice_top(s + 1)) / 2.0
    }
}

/// Slice s (A..D) of a logic tile, as (min, max) in the tile.
pub fn slice_rect(s: usize) -> ([f32; 2], [f32; 2]) {
    use slices::*;
    ([X.0, slice_top(s)], [X.1, slice_bottom(s)])
}

/// LUT n of a logic tile.
pub fn lut_rect(n: usize) -> ([f32; 2], [f32; 2]) {
    use slices::*;
    let top = row_top(n);
    ([LUT_X.0, top + 0.04 * ROW], [LUT_X.1, top + 0.50 * ROW])
}

/// Flip-flop n of a logic tile.
pub fn ff_rect(n: usize) -> ([f32; 2], [f32; 2]) {
    let (min, max) = lut_rect(n);
    ([slices::FF_X.0, min[1]], [slices::FF_X.1, max[1]])
}

/// The wide muxes of slice s.
pub fn mux_rect(s: usize) -> ([f32; 2], [f32; 2]) {
    use slices::*;
    let top = strip_top(s);
    ([LUT_X.0, top + 0.05 * STRIP], [LUT_X.1, top + 0.32 * STRIP])
}

/// The controls of slice s.
pub fn ctrl_rect(s: usize) -> ([f32; 2], [f32; 2]) {
    use slices::*;
    let top = strip_top(s);
    ([LUT_X.0, top + 0.40 * STRIP], [FF_X.1, top + 0.95 * STRIP])
}

/// A pin of a slice's wide muxes (`F5A`, `FXB`, `FXAC`) or of its controls
/// (`CLK2`, `WAD0A`, `WDO1C`): its slice, whether it is a mux pin, and its
/// order in its box.
fn strip_pin(base: &str) -> Option<(usize, bool, u8)> {
    let slice = |c: u8| (b'A'..=b'D').contains(&c).then(|| (c - b'A') as usize);
    let b = base.as_bytes();
    let last = *b.last()?;
    match (b.len(), base.get(..2), base.get(..3)) {
        (3, Some("F5"), _) => return Some((slice(last)?, true, 0)),
        (3, Some("FX"), _) => return Some((slice(last)?, true, 1)),
        (4, _, Some("FXA")) => return Some((slice(last)?, true, 2)),
        (4, _, Some("FXB")) => return Some((slice(last)?, true, 3)),
        _ => {}
    }
    let digits = base.find(|c: char| c.is_ascii_digit())?;
    let (family, rest) = base.split_at(digits);
    let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let number: u8 = rest[..rest.len() - suffix.len()].parse().ok()?;
    let order = |family: u8| family * 10 + number;
    match (family, suffix) {
        // M0 selects slice A's F5 mux, M1 its FX mux; a flip-flop takes M
        // too, past its LUT.
        ("M", "") if number < 8 => Some(((number / 2) as usize, true, 4 + number % 2)),
        ("CLK", "") if number < 4 => Some((number as usize, false, order(0))),
        ("LSR", "") if number < 4 => Some((number as usize, false, order(1))),
        ("CE", "") if number < 4 => Some((number as usize, false, order(2))),
        // LUTRAM write clock and enable of slices A and B.
        ("WCK", "") if number < 2 => Some((number as usize, false, order(3))),
        ("WRE", "") if number < 2 => Some((number as usize, false, order(4))),
        // Write address and data: into A and B, out of C.
        ("WAD", s) if s.len() == 1 => Some((slice(s.as_bytes()[0])?, false, order(5))),
        ("WD", s) if s.len() == 1 => Some((slice(s.as_bytes()[0])?, false, order(6))),
        ("WADO", s) if s.len() == 1 => Some((slice(s.as_bytes()[0])?, false, order(7))),
        ("WDO", s) if s.len() == 1 => Some((slice(s.as_bytes()[0])?, false, order(8))),
        _ => None,
    }
}

/// The site side of a logic tile's pin: its row at the block's edge, and
/// its polyline from the site to there (or, for the carry between slices,
/// to the gap between them). `strip` has the rows of the strip pins.
fn logic_pin(base: &str, strip: &BTreeMap<String, f32>) -> Option<(f32, Vec<[f32; 2]>)> {
    use slices::*;
    if let Some(&y) = strip.get(base) {
        return Some((y, vec![[LUT_X.0, y], [BLOCK, y]]));
    }
    match base {
        "FCI" => {
            let path = vec![[CARRY_X, slice_top(0)], [CARRY_X, FCI_Y], [BLOCK, FCI_Y]];
            return Some((FCI_Y, path));
        }
        "FCO" => {
            let y = fco_y();
            return Some((
                y,
                vec![[CARRY_X, slice_bottom(3)], [CARRY_X, y], [BLOCK, y]],
            ));
        }
        _ => {}
    }
    let index = |l: &str, of: [&str; 3]| of.iter().position(|x| *x == l);
    if let Some(s) = base
        .strip_prefix("FCO")
        .and_then(|l| index(l, ["A", "B", "C"]))
    {
        return Some((gap(s), vec![[CARRY_X, slice_bottom(s)], [CARRY_X, gap(s)]]));
    }
    if let Some(s) = base
        .strip_prefix("FCI")
        .and_then(|l| index(l, ["B", "C", "D"]))
    {
        return Some((gap(s), vec![[CARRY_X, slice_top(s + 1)], [CARRY_X, gap(s)]]));
    }
    let split = base.find(|c: char| c.is_ascii_digit())?;
    let (letters, number) = base.split_at(split);
    let n: usize = number.parse().ok().filter(|n| *n < 8)?;
    let y = |at: f32| row_top(n) + at * ROW;
    let input = |at: f32| Some((y(at), vec![[LUT_X.0, y(at)], [BLOCK, y(at)]]));
    // Under the LUT, then up into the flip-flop's bottom.
    let to_ff = |at: f32, x: f32| {
        let bottom = ff_rect(n).1[1];
        Some((y(at), vec![[x, bottom], [x, y(at)], [BLOCK, y(at)]]))
    };
    match letters {
        "A" => input(0.10),
        "B" => input(0.20),
        "C" => input(0.30),
        "D" => input(0.40),
        // Out of the LUT's right side, down and back under it.
        "F" => Some((
            y(0.60),
            vec![
                [LUT_X.1, y(0.27)],
                [F_X, y(0.27)],
                [F_X, y(0.60)],
                [BLOCK, y(0.60)],
            ],
        )),
        "DI" => to_ff(0.80, FF_X.0 + 0.10),
        "Q" => to_ff(0.90, FF_X.0 + 0.15),
        _ => None,
    }
}

/// The connections inside each slice of a logic tile that no tile type
/// names, as the plan executes a slice (src/compile): (name `~FROM>TO`,
/// polyline from where the value comes to where it goes). In slice s with
/// LUTs n0 = 2s and n1 = 2s + 1:
///
/// - the carry, in CCU2 mode: from the slice's carry in (`FCI_SLICE` for
///   A, `FCIB_SLICE` …) along its top into LUT n0, from LUT n0 down into
///   LUT n1 (`FCIA1_SLICE` …), and from LUT n1 under it to the slice's
///   carry out (`FCOA_SLICE` …, `FCO_SLICE` for D);
/// - each LUT's output into the slice's F5 mux, which M0 selects;
/// - the M input from the mux box (where M ends) into its flip-flop, which
///   takes M or DI as `REGn.SD` says;
/// - the slice's clock, reset and enable from its control box into both
///   flip-flops: into the lower one straight up, into the upper one round
///   its right side.
///
/// They run in the gap between the LUTs and the flip-flops, under the LUTs
/// and right of the flip-flops, apart from the pins and each other.
fn slice_internals() -> Vec<(String, Vec<[f32; 2]>)> {
    use slices::*;
    let mut out = Vec::new();
    for s in 0..4 {
        let (n0, n1) = (2 * s, 2 * s + 1);
        let y = |n: usize, at: f32| row_top(n) + at * ROW;
        let strip = |at: f32| strip_top(s) + at * STRIP;
        let (lut, ff) = (LUT_X.1, FF_X.0);
        let mut push = |name: String, points: Vec<[f32; 2]>| out.push((name, points));
        // The carry.
        let carry = lut - 0.06;
        push(
            format!("~FCI>LUT{n0}"),
            vec![
                [CARRY_X, slice_top(s)],
                [CARRY_X, y(n0, 0.02)],
                [carry, y(n0, 0.02)],
                [carry, y(n0, 0.04)],
            ],
        );
        push(
            format!("~LUT{n0}>LUT{n1}"),
            vec![[lut - 0.11, y(n0, 0.50)], [lut - 0.11, y(n1, 0.04)]],
        );
        push(
            format!("~LUT{n1}>FCO"),
            vec![
                [carry, y(n1, 0.50)],
                [carry, y(n1, 0.55)],
                [CARRY_X, y(n1, 0.55)],
                [CARRY_X, slice_bottom(s)],
            ],
        );
        // Each LUT into the F5 mux.
        for (n, x, at) in [(n0, lut + 0.14, 0.24), (n1, lut + 0.10, 0.12)] {
            push(
                format!("~LUT{n}>MUX"),
                vec![
                    [lut, y(n, 0.40)],
                    [x, y(n, 0.40)],
                    [x, strip(at)],
                    [lut, strip(at)],
                ],
            );
        }
        // M into its flip-flop.
        for (n, x, at) in [(n0, ff - 0.03, 0.28), (n1, ff - 0.01, 0.31)] {
            push(
                format!("~MUX>FF{n}"),
                vec![
                    [lut, strip(at)],
                    [x, strip(at)],
                    [x, y(n, 0.15)],
                    [ff, y(n, 0.15)],
                ],
            );
        }
        // The controls into both flip-flops.
        for (k, control) in ["CLK", "LSR", "CE"].into_iter().enumerate() {
            let k = k as f32;
            let x = ff + 0.34 + 0.06 * k;
            push(
                format!("~{control}>FF{n1}"),
                vec![[x, strip(0.40)], [x, y(n1, 0.50)]],
            );
            let x = FF_X.1 + 0.015 * (k + 1.0);
            push(
                format!("~{control}>FF{n0}"),
                vec![
                    [FF_X.1, strip(0.50 + 0.15 * k)],
                    [x, strip(0.50 + 0.15 * k)],
                    [x, y(n0, 0.35 - 0.10 * k)],
                    [FF_X.1, y(n0, 0.35 - 0.10 * k)],
                ],
            );
        }
    }
    out
}

/// The boxes of a logic tile.
fn logic_boxes() -> Vec<SiteBox> {
    let mut out = Vec::new();
    let mut push = |(min, max): ([f32; 2], [f32; 2]), kind, label: String| {
        out.push(SiteBox {
            min,
            max,
            kind,
            label,
        })
    };
    for s in 0..4 {
        let letter = (b'A' + s as u8) as char;
        push(slice_rect(s), BoxKind::Slice, format!("SLICE{letter}"));
        push(mux_rect(s), BoxKind::Mux, "F5 FX M".into());
        push(ctrl_rect(s), BoxKind::Control, "CLK LSR CE WRE".into());
    }
    for n in 0..8 {
        push(lut_rect(n), BoxKind::Lut, format!("LUT{n}"));
        push(ff_rect(n), BoxKind::FlipFlop, format!("FF{n}"));
    }
    out
}

/// A wire of a position.
#[derive(Clone, Debug)]
enum Wire {
    /// A local wire: its row and its lane.
    Local { y: f32, x: f32 },
    /// The site side of a pin: its row at the block's edge and its path.
    Pin {
        y: f32,
        path: Vec<[f32; 2]>,
        /// A pin with more than one arc runs on past the block's edge, so
        /// that each arc meets it with a dot of its own.
        long: bool,
    },
}

impl Wire {
    fn y(&self) -> f32 {
        match self {
            Wire::Local { y, .. } | Wire::Pin { y, .. } => *y,
        }
    }
}

/// The wires, tracks and arcs of the tile types at a position.
struct Layout {
    logic: bool,
    tracks: BTreeSet<TrackId>,
    /// (wire, tap) of the tracks that an arc of this position touches.
    attached: HashSet<(TrackId, u8)>,
    wires: BTreeMap<String, Wire>,
    arcs: Vec<(String, String)>,
    /// The same, shared with the templates, and whether each is fixed.
    shared_arcs: std::sync::Arc<Vec<(String, String)>>,
    fixed: std::sync::Arc<Vec<bool>>,
    /// The row of each jumper between two vertical tracks, by arc.
    vv_rows: HashMap<usize, f32>,
    unplaced: Vec<String>,
    boxes: Vec<SiteBox>,
}

/// The suffixes of the ports of sites: `JADA0_EBR`, `JP3_MULT18`,
/// `JCEA_SIOLOGIC`.
const PORTS: [&str; 15] = [
    "EBR", "MULT", "ALU", "PRADD", "IOLOGIC", "SIOLOGIC", "PIO", "CIBTEST", "DCU", "PLLREFCS",
    "EXTREF", "APIO", "OSC", "DQS", "DLLDEL",
];

/// The box a site's port belongs to: its suffix and, at the I/O, the
/// letter of its pad.
fn port_box(name: &str) -> Option<String> {
    if name.starts_with("G_") {
        return None;
    }
    // A neighbour's prefix is not a port's name: `W2_DLLDEL` is the wire
    // `DLLDEL` (a DLLDEL's way to the clock input), not a port of one.
    let (base, suffix) = split_prefix(name).2.rsplit_once('_')?;
    let family = suffix.trim_end_matches(|c: char| c.is_ascii_digit());
    if base.is_empty() || !PORTS.contains(&family) {
        return None;
    }
    let io = matches!(family, "PIO" | "IOLOGIC" | "SIOLOGIC");
    match base
        .chars()
        .last()
        .filter(|c| io && ('A'..='D').contains(c))
    {
        Some(letter) => Some(format!("{suffix}{letter}")),
        None => Some(suffix.to_string()),
    }
}

/// A name with its numbers padded, so that `A10` sorts after `A9`.
fn natural(name: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    for c in name.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            out.push_str(&format!("{digits:0>6}"));
            digits.clear();
        }
        out.push(c);
    }
    out
}

/// A site at a position: where the site is (`R48C0`) and the names of its
/// ports here.
type Placed = (String, Vec<String>);

/// The sites drawn at a position other than a logic tile, as boxes
/// stacked in the block, each with its ports as pins on its left edge, in
/// order. `sites` are the sites whose ports are wires here, by name, with
/// those wires (a site with none is drawn bare); the ports of parts of sites
/// that have no site of their own (a multiplier's pre-adder, a CIB's test
/// inputs) are found by the suffix of their wires.
fn site_ports(
    names: &BTreeSet<String>,
    sites: &[(String, Vec<String>)],
    wires: &mut BTreeMap<String, Wire>,
    boxes: &mut Vec<SiteBox>,
) {
    let mut shown: Vec<(String, Vec<&str>)> = Vec::new();
    let mut claimed: HashSet<&str> = HashSet::new();
    for (label, ports) in sites {
        let list: Vec<&str> = ports
            .iter()
            .map(String::as_str)
            .filter(|p| names.contains(*p) && claimed.insert(p))
            .collect();
        shown.push((label.clone(), list));
    }
    let mut parts: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for name in names {
        if claimed.contains(name.as_str()) {
            continue;
        }
        if let Some(key) = port_box(name) {
            parts.entry(key).or_default().push(name);
        }
    }
    shown.extend(parts);
    if shown.is_empty() {
        return;
    }
    let weight = |n: usize| n.max(4) as f32;
    let total: f32 = shown.iter().map(|s| weight(s.1.len())).sum();
    let (top, bottom) = (BAND_TOP + 0.01, BAND_BOTTOM - 0.006);
    let gap = (0.006f32).min((bottom - top) * 0.2 / shown.len() as f32);
    let room = bottom - top - gap * (shown.len() - 1) as f32;
    let mut y = top;
    for (label, mut list) in shown {
        let h = room * weight(list.len()) / total;
        let (min, max) = ([SITE_X.0, y], [SITE_X.1, y + h]);
        list.sort_by_key(|n| natural(n));
        for (k, name) in list.iter().enumerate() {
            let py = lane((min[1], max[1]), k, list.len());
            let path = vec![[min[0], py], [BLOCK, py]];
            let pin = Wire::Pin {
                y: py,
                path,
                long: false,
            };
            wires.insert(name.to_string(), pin);
        }
        // The CIB's test site stands for the CIB: its pins are the CIB's J
        // wires, which the CIB's input muxes feed, or tie to a constant.
        let label = if label == "CIBTEST" {
            "CIB".to_string()
        } else {
            label
        };
        boxes.push(SiteBox {
            min,
            max,
            kind: BoxKind::Site,
            label,
        });
        y += h + gap;
    }
}

/// `foreign` has the names that some tile names in a neighbour.
fn layout(
    db: &Database,
    kinds: &[String],
    sites: &[(String, Vec<String>)],
    hosted: &[String],
    foreign: &HashSet<String>,
    inside_dsp: &HashSet<String>,
) -> Layout {
    let logic = kinds.iter().any(|k| k == "PLC2");
    let mut tracks = BTreeSet::new();
    let mut attached = HashSet::new();
    let mut names = BTreeSet::new();
    let mut arcs = Vec::new();
    let mut vv = Vec::new();
    let mut fixed = Vec::new();
    for kind in kinds {
        for (source, sink, is_fixed) in routing_kind(db, kind) {
            // The DSP blocks' own wires are drawn with the blocks.
            if [source, sink]
                .iter()
                .any(|n| inside_dsp.contains(split_prefix(n).2))
            {
                continue;
            }
            let mut vertical = 0;
            for name in [source, sink] {
                let (dr, dc, base) = split_prefix(name);
                match parse_track(base) {
                    Some((id, tap)) => {
                        tracks.insert(id);
                        if let Some(t) = tap_here(&id, tap, dr, dc) {
                            attached.insert((id, t));
                            vertical += usize::from(!id.horizontal);
                        }
                    }
                    None if (dr, dc) == (0, 0) => {
                        names.insert(name.to_string());
                    }
                    None => {}
                }
            }
            if vertical == 2 {
                vv.push(arcs.len());
            }
            arcs.push((source.to_string(), sink.to_string()));
            fixed.push(is_fixed);
        }
    }
    // Wires of a neighbour that it does not name itself, drawn here.
    names.extend(hosted.iter().cloned());
    // The ports of sites here that tiles name with a prefix.
    for (_, ports) in sites {
        for port in ports {
            if arcs.iter().any(|(s, t)| s == port || t == port) {
                names.insert(port.clone());
            }
        }
    }
    // The pins of the slices: the strip pins spread over their boxes.
    let mut wires: BTreeMap<String, Wire> = BTreeMap::new();
    let mut unplaced = Vec::new();
    let mut boxes = Vec::new();
    if !logic {
        site_ports(&names, sites, &mut wires, &mut boxes);
    } else {
        boxes = logic_boxes();
        let mut boxes: BTreeMap<(usize, bool), Vec<(u8, &str)>> = BTreeMap::new();
        for name in &names {
            if let Some((s, mux, order)) = name.strip_suffix("_SLICE").and_then(strip_pin) {
                boxes
                    .entry((s, mux))
                    .or_default()
                    .push((order, name.strip_suffix("_SLICE").unwrap()));
            }
        }
        let mut strip = BTreeMap::new();
        for ((s, mux), mut list) in boxes {
            list.sort();
            let (min, max) = if mux { mux_rect(s) } else { ctrl_rect(s) };
            for (k, (_, base)) in list.iter().enumerate() {
                strip.insert(base.to_string(), lane((min[1], max[1]), k, list.len()));
            }
        }
        for name in &names {
            if let Some(base) = name.strip_suffix("_SLICE") {
                match logic_pin(base, &strip) {
                    Some((y, path)) => {
                        let pin = Wire::Pin {
                            y,
                            path,
                            long: false,
                        };
                        wires.insert(name.clone(), pin);
                    }
                    None => unplaced.push(name.clone()),
                }
            }
        }
    }
    // Pins with more than one arc, or that a neighbour links to, run on.
    let mut uses: HashMap<String, usize> = HashMap::new();
    for (s, t) in &arcs {
        for n in [s, t] {
            if matches!(wires.get(n), Some(Wire::Pin { .. })) {
                *uses.entry(n.clone()).or_default() += 1;
            }
        }
    }
    for (name, w) in wires.iter_mut() {
        if let Wire::Pin { y, path, long } = w {
            if uses.get(name).copied().unwrap_or(0) >= 2 || foreign.contains(name) {
                *long = true;
                path.push([LOCAL_LANES.0, *y]);
            }
        }
    }
    // A local wire fixed to a pin with that arc alone takes the pin's row:
    // the pin of the same name first, then the one that drives it, then any.
    let mut partners: BTreeMap<&str, Vec<(u8, &str)>> = BTreeMap::new();
    for (s, t) in &arcs {
        for (pin, local, drives) in [(s, t, true), (t, s, false)] {
            let is_pin = matches!(wires.get(pin), Some(Wire::Pin { long: false, .. }));
            if is_pin && names.contains(local) && !wires.contains_key(local) {
                let same = pin.strip_suffix("_SLICE") == Some(local.as_str());
                let rank = if same { 0 } else { 2 - u8::from(drives) };
                partners
                    .entry(local.as_str())
                    .or_default()
                    .push((rank, pin.as_str()));
            }
        }
    }
    let mut claimed: HashSet<String> = HashSet::new();
    let mut homes: Vec<(String, String)> = Vec::new();
    for (local, mut list) in partners {
        list.sort();
        if let Some(&(_, pin)) = list.iter().find(|(_, p)| !claimed.contains(*p)) {
            claimed.insert(pin.to_string());
            homes.push((local.to_string(), pin.to_string()));
        }
    }
    let homed: HashSet<&str> = homes.iter().map(|p| p.0.as_str()).collect();
    // In a tap column the left branches of the clock end where the right
    // ones begin: their local wires (`L_HPBX…`) come first, so their lanes
    // are left of those of the tile's own (`G_HPBX…`) and the right ones.
    let mut others: Vec<String> = names
        .iter()
        .filter(|n| !wires.contains_key(*n) && !homed.contains(n.as_str()))
        .cloned()
        .collect();
    others.sort_by_key(|n| !n.starts_with("L_HPBX"));
    // Every row of the band is a slot of one grid, ROW_GAP apart (closer
    // only where a position has more rows than the band has slots). A pin
    // that reaches the block's edge moves to the nearest free slot; the
    // other local wires and the jumpers between vertical tracks share the
    // slots left, interleaved.
    let to_block = |w: &Wire| matches!(w, Wire::Pin { path, .. } if path.iter().any(|p| (p[0] - BLOCK).abs() < 1e-6));
    let mut pins: Vec<(f32, String)> = wires
        .iter()
        .filter(|(_, w)| to_block(w))
        .map(|(n, w)| (w.y(), n.clone()))
        .collect();
    pins.sort_by(|a, b| a.0.total_cmp(&b.0));
    let need = others.len() + vv.len();
    let low = BAND_TOP + 0.002;
    let slots = (((BAND_BOTTOM - low) / ROW_GAP) as usize + 1).max(pins.len() + need);
    let pitch = (BAND_BOTTOM - low) / (slots - 1) as f32;
    let mut used = vec![false; slots];
    for (old, name) in &pins {
        let k0 = (((old - low) / pitch).round().max(0.0) as usize).min(slots - 1);
        let k = (0..slots)
            .flat_map(|d| [k0.checked_add(d), k0.checked_sub(d)])
            .flatten()
            .find(|k| *k < slots && !used[*k])
            .unwrap();
        used[k] = true;
        let new = low + k as f32 * pitch;
        if let Some(Wire::Pin { y, path, .. }) = wires.get_mut(name) {
            for p in path.iter_mut() {
                if p[1] == *old {
                    p[1] = new;
                }
            }
            *y = new;
        }
    }
    let mut placed: Vec<(String, f32)> = homes
        .iter()
        .map(|(local, pin)| (local.clone(), wires[pin].y()))
        .collect();
    let free: Vec<f32> = (0..slots)
        .filter(|k| !used[*k])
        .map(|k| low + k as f32 * pitch)
        .collect();
    let mut vv_rows = HashMap::new();
    let (mut a, mut b) = (0, 0);
    for k in 0..need {
        let y = free[k * free.len() / need];
        let local = b >= vv.len()
            || (a < others.len() && (2 * a + 1) * vv.len() <= (2 * b + 1) * others.len());
        if local {
            placed.push((others[a].clone(), y));
            a += 1;
        } else {
            vv_rows.insert(vv[b], y);
            b += 1;
        }
    }
    // Lanes as a staircase: higher rows further left.
    let mut ys: Vec<f32> = placed.iter().map(|p| p.1).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    for (name, y) in placed {
        // LANE_GAP apart from the left, closer only where too many.
        let pitch = LANE_GAP.min((LOCAL_LANES.1 - LOCAL_LANES.0) / ys.len().max(1) as f32);
        let x = LOCAL_LANES.0 + (ys.partition_point(|h| *h < y) as f32 + 0.5) * pitch;
        wires.insert(name, Wire::Local { y, x });
    }
    Layout {
        logic,
        tracks,
        attached,
        wires,
        shared_arcs: std::sync::Arc::new(arcs.clone()),
        fixed: std::sync::Arc::new(fixed),
        arcs,
        vv_rows,
        unplaced,
        boxes,
    }
}

/// A wire of a template: its polyline, the distances along it where things
/// attach, and whether its ends must stay (a site, or a conductor that
/// continues past the tile).
struct Draw {
    points: Vec<[f32; 2]>,
    part: Part,
    stops: Vec<f32>,
    keep_start: bool,
    keep_end: bool,
}

fn length(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Distance along a polyline to its point nearest `p`.
fn along(points: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let mut best = (f32::MAX, 0.0);
    let mut walked = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = length(a, b);
        let t = if len > 0.0 {
            (((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / (len * len))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let e = length(p, q);
        if e < best.0 {
            best = (e, walked + t * len);
        }
        walked += len;
    }
    best.1
}

/// The part of a polyline between distances `from` and `to`.
fn cut(points: &[[f32; 2]], from: f32, to: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut walked = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = length(a, b);
        let (s0, s1) = (walked, walked + len);
        // Exactly on the piece's line, and exactly at its ends near them.
        let at = |s: f32| {
            let t = if len > 0.0 {
                ((s - s0) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if t < 1e-5 {
                a
            } else if t > 1.0 - 1e-5 {
                b
            } else {
                let x = if a[0] == b[0] {
                    a[0]
                } else {
                    a[0] + (b[0] - a[0]) * t
                };
                let y = if a[1] == b[1] {
                    a[1]
                } else {
                    a[1] + (b[1] - a[1]) * t
                };
                [x, y]
            }
        };
        if s1 >= from && s0 <= to {
            let p = at(from.max(s0));
            if out.last() != Some(&p) {
                out.push(p);
            }
            out.push(at(to.min(s1)));
        }
        walked = s1;
    }
    out.dedup();
    out
}

/// A conductor at one tap across its tile, from the side it comes from;
/// with `next`, it steps to that lane just before the side it goes to.
fn track_points(id: &TrackId, tap: u8, at: f32, next: Option<f32>, height: f32) -> Vec<[f32; 2]> {
    let len = if id.horizontal { TILE_W } else { height };
    let jog = JOG + tap as f32 * JOG_STEP;
    let p = |s: f32, lane: f32| {
        let s = if id.backwards { len - s } else { s };
        if id.horizontal {
            [s, lane]
        } else {
            [lane, s]
        }
    };
    match next {
        Some(n) => vec![p(0.0, at), p(len - jog, at), p(len - jog, n), p(len, n)],
        None => vec![p(0.0, at), p(len, at)],
    }
}

/// An arc's end: a track's lane, a local wire, a pin, or a neighbour's
/// wire.
#[derive(Clone, Copy, Debug)]
enum End {
    Track { id: TrackId, at: f32, wire: usize },
    Local { y: f32, x: f32, wire: usize },
    Pin { y: f32, wire: usize, long: bool },
    Foreign { dr: i16, dc: i16 },
}

/// A piece of the clock tree through a tile: a branch of its row, or a
/// spine of its tap column. Its channel; whether it runs on into the tile
/// before (left, above) and after; the local wires here it meets; and the
/// wire of the device it is, as this tile names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ClockRun {
    spine: bool,
    ch: u8,
    before: bool,
    after: bool,
    meets: Vec<String>,
    owner: String,
}

/// What a template depends on besides its layout: for each (wire, tap) of
/// its tracks, whether the conductor has an arc behind (in a tile it came
/// from) and ahead; the names of its wires that neighbours link to; and its
/// arcs to a neighbour's wire that the neighbour has.
struct Key<'a> {
    here: &'a Layout,
    reach: &'a BTreeMap<(TrackId, u8), (bool, bool)>,
    /// How many links arrive at each wire: each gets a riser of its own.
    anchored: &'a BTreeMap<String, usize>,
    linked: &'a BTreeSet<usize>,
    /// The height of the row.
    height: f32,
    /// Local wires that run on down into a DSP block below.
    tapped: &'a BTreeSet<String>,
    /// The clock tree through the tile.
    clock: &'a [ClockRun],
}

/// Lanes of a run handed out one by one.
struct Run {
    range: (f32, f32),
    n: usize,
    next: usize,
    taken: HashSet<u32>,
}

impl Run {
    fn new(range: (f32, f32), n: usize) -> Self {
        Self {
            range,
            n,
            next: 0,
            taken: HashSet::new(),
        }
    }
    fn take(&mut self) -> f32 {
        let x = lane(self.range, self.next.min(self.n.saturating_sub(1)), self.n);
        self.next += 1;
        self.taken.insert(x.to_bits());
        x
    }
    /// Spreads the lanes the live joins use over the run, in order: lanes
    /// were handed out for every arc before those that join a wire to
    /// nothing else were left out. `channel` limits it to points in the
    /// horizontal channel.
    fn compact(&self, joins: &mut [Join], alive: &[bool], channel: bool) {
        let ours =
            |p: &[f32; 2]| self.taken.contains(&p[0].to_bits()) && (!channel || p[1] < H_CHANNEL);
        let mut used: Vec<f32> = Vec::new();
        for (j, _) in joins.iter().zip(alive).filter(|(_, a)| **a) {
            for p in j.points() {
                if ours(p) {
                    used.push(p[0]);
                }
            }
        }
        used.sort_by(f32::total_cmp);
        used.dedup();
        let new: HashMap<u32, f32> = used
            .iter()
            .enumerate()
            .map(|(k, x)| (x.to_bits(), lane(self.range, k, used.len())))
            .collect();
        for (j, _) in joins.iter_mut().zip(alive).filter(|(_, a)| **a) {
            for p in j.points_mut() {
                if ours(p) {
                    p[0] = new[&p[0].to_bits()];
                }
            }
        }
    }
}

/// How an arc (or an arriving link) is drawn: where it attaches to its
/// wires, its dots, its jumper, and the link it starts or ends.
#[derive(Default)]
struct Join {
    stops: Vec<(usize, [f32; 2])>,
    dots: Vec<[f32; 2]>,
    jumper: Vec<[f32; 2]>,
    link: Option<([f32; 2], i16, i16, String, usize)>,
    anchor: Option<([f32; 2], String)>,
    /// A wire that runs on into a DSP block.
    tap: bool,
    /// Where a local wire meets the clock tree.
    clock: bool,
    /// The arc it draws (`u32::MAX` for none).
    arc: u32,
}

impl Join {
    fn points(&self) -> impl Iterator<Item = &[f32; 2]> {
        self.stops
            .iter()
            .map(|s| &s.1)
            .chain(&self.dots)
            .chain(&self.jumper)
            .chain(self.link.iter().map(|l| &l.0))
            .chain(self.anchor.iter().map(|a| &a.0))
    }
    fn points_mut(&mut self) -> impl Iterator<Item = &mut [f32; 2]> {
        self.stops
            .iter_mut()
            .map(|s| &mut s.1)
            .chain(self.dots.iter_mut())
            .chain(self.jumper.iter_mut())
            .chain(self.link.iter_mut().map(|l| &mut l.0))
            .chain(self.anchor.iter_mut().map(|a| &mut a.0))
    }
    fn dot(stops: &[(usize, [f32; 2])], p: [f32; 2]) -> Self {
        Self {
            stops: stops.to_vec(),
            dots: vec![p],
            ..Self::default()
        }
    }
    /// A jumper along horizontal and vertical pieces, a dot at each end.
    fn jumper(stops: &[(usize, [f32; 2])], path: &[[f32; 2]]) -> Self {
        Self {
            stops: stops.to_vec(),
            dots: vec![path[0], *path.last().unwrap()],
            jumper: path.to_vec(),
            ..Self::default()
        }
    }
    /// Links lead out of the tile, so they never leave a wire dangling.
    fn lasting(&self) -> bool {
        self.link.is_some() || self.anchor.is_some() || self.tap || self.clock
    }
}

/// For each wire, how many distinct points its live joins and its
/// continuing ends hold it at: a wire held at fewer than two is a point.
fn points(wires: &[Draw], joins: &[Join], alive: &[bool]) -> Vec<usize> {
    let mut at: Vec<Vec<f32>> = wires
        .iter()
        .map(|w| {
            let total: f32 = w.points.windows(2).map(|p| length(p[0], p[1])).sum();
            let mut v = Vec::new();
            if w.keep_start {
                v.push(0.0);
            }
            if w.keep_end {
                v.push(total);
            }
            v
        })
        .collect();
    for (j, _) in joins.iter().zip(alive).filter(|(_, a)| **a) {
        for (w, p) in &j.stops {
            at[*w].push(along(&wires[*w].points, *p));
        }
    }
    at.into_iter()
        .map(|mut v| {
            v.sort_by(f32::total_cmp);
            v.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
            v.len()
        })
        .collect()
}

fn template(key: &Key, lanes: &Lanes) -> Template {
    let here = key.here;
    let mut out = Template {
        unplaced: here.unplaced.clone(),
        height: key.height,
        ..Template::default()
    };
    let mut wires: Vec<Draw> = Vec::new();
    // Tracks: every tap of every wire.
    let mut track_wire: HashMap<(TrackId, u8), usize> = HashMap::new();
    for id in &here.tracks {
        for tap in 0..=id.span {
            let at = lanes.at(id, tap);
            let (enters, leaves) = key
                .reach
                .get(&(*id, tap))
                .copied()
                .unwrap_or((false, false));
            track_wire.insert((*id, tap), wires.len());
            wires.push(Draw {
                points: track_points(
                    id,
                    tap,
                    at,
                    leaves.then(|| lanes.at(id, tap + 1)),
                    key.height,
                ),
                part: Part::Track,
                stops: Vec::new(),
                keep_start: enters,
                keep_end: leaves,
            });
        }
    }
    // Local wires and pins.
    let mut wire_of_name: HashMap<&str, usize> = HashMap::new();
    for (name, w) in &here.wires {
        wire_of_name.insert(name.as_str(), wires.len());
        wires.push(match w {
            Wire::Local { y, x } => Draw {
                points: vec![[BLOCK, *y], [*x, *y], [*x, 0.004]],
                part: Part::Local,
                stops: Vec::new(),
                keep_start: false,
                keep_end: false,
            },
            Wire::Pin { path, .. } => Draw {
                points: path.clone(),
                part: Part::Pin,
                stops: Vec::new(),
                keep_start: true,
                keep_end: false,
            },
        });
    }
    // The clock tree: each piece from where it comes in, or the first local
    // wire it meets, to where it goes on, or the last.
    let mut clock_joins: Vec<Join> = Vec::new();
    for run in key.clock {
        let meets: Vec<(usize, f32, f32)> = run
            .meets
            .iter()
            .filter_map(|n| match here.wires.get(n) {
                Some(Wire::Local { y, x }) => Some((wire_of_name[n.as_str()], *x, *y)),
                _ => None,
            })
            .collect();
        let at = |m: &(usize, f32, f32)| if run.spine { m.2 } else { m.1 };
        let (len, lane) = if run.spine {
            (key.height, spine_x(run.ch))
        } else {
            (TILE_W, branch_y(run.ch))
        };
        let from = if run.before {
            0.0
        } else {
            meets.iter().map(at).fold(f32::MAX, f32::min)
        };
        let to = if run.after {
            len
        } else {
            meets.iter().map(at).fold(f32::MIN, f32::max)
        };
        let point = |s: f32| if run.spine { [lane, s] } else { [s, lane] };
        let w = wires.len();
        wires.push(Draw {
            points: vec![point(from), point(to)],
            part: Part::Clock,
            stops: Vec::new(),
            keep_start: run.before,
            keep_end: run.after,
        });
        for m in &meets {
            let p = point(at(m));
            clock_joins.push(Join {
                stops: vec![(m.0, p), (w, p)],
                dots: vec![p],
                clock: true,
                arc: u32::MAX,
                ..Join::default()
            });
        }
    }
    let end = |name: &str| -> Option<End> {
        // The same conductor at another position (`=G_HPRX0000`).
        if name.starts_with('=') {
            return Some(End::Foreign { dr: 0, dc: 0 });
        }
        let (dr, dc, base) = split_prefix(name);
        if let Some((id, tap)) = parse_track(base) {
            let t = tap_here(&id, tap, dr, dc)?;
            return Some(End::Track {
                id,
                at: lanes.at(&id, t),
                wire: *track_wire.get(&(id, t))?,
            });
        }
        // A site's port here may be named with a prefix.
        if (dr, dc) != (0, 0) && !wire_of_name.contains_key(name) {
            return Some(End::Foreign { dr, dc });
        }
        let wire = *wire_of_name.get(name)?;
        Some(match &here.wires[name] {
            Wire::Local { y, x } => End::Local { y: *y, x: *x, wire },
            Wire::Pin { y, long, .. } => End::Pin {
                y: *y,
                wire,
                long: *long,
            },
        })
    };
    // Arcs to a neighbour's wire the neighbour lacks are left out.
    let ends: Vec<Option<(End, End)>> = here
        .arcs
        .iter()
        .enumerate()
        .map(|(i, (s, t))| {
            let e = (end(s)?, end(t)?);
            let foreign = matches!(e, (End::Foreign { .. }, _) | (_, End::Foreign { .. }));
            (!foreign || key.linked.contains(&i)).then_some(e)
        })
        .collect();
    let pin_end = |w: usize| *wires[w].points.last().unwrap();
    let meet = |a: usize, b: usize| length(pin_end(a), pin_end(b)) < 1e-5;
    // Risers and jumpers above the block, counted first.
    let (mut risers, mut above) = (key.anchored.values().sum::<usize>(), 0);
    for (s, t) in ends.iter().flatten() {
        use End::*;
        let riser = match (s, t) {
            (Foreign { .. }, Foreign { .. }) => false,
            (Foreign { .. }, Track { id, .. }) | (Track { id, .. }, Foreign { .. }) => {
                id.horizontal
            }
            (Foreign { .. }, _) | (_, Foreign { .. }) => true,
            (Track { id: a, .. }, Track { id: b, .. }) => {
                above += usize::from(a.horizontal && b.horizontal);
                false
            }
            (Track { id, .. }, Pin { .. }) | (Pin { .. }, Track { id, .. }) => id.horizontal,
            (Local { y: a, .. }, Pin { y: b, long, .. })
            | (Pin { y: b, long, .. }, Local { y: a, .. }) => {
                if *long {
                    b > a
                } else {
                    (a - b).abs() > 1e-6
                }
            }
            (Pin { wire: a, .. }, Pin { wire: b, .. }) => here.logic && !meet(*a, *b),
            _ => false,
        };
        risers += usize::from(riser);
    }
    // Between two ports of the sites here, a riser inside the block.
    let inner = ends
        .iter()
        .flatten()
        .filter(
            |e| matches!(e, (End::Pin { wire: a, .. }, End::Pin { wire: b, .. }) if !meet(*a, *b)),
        )
        .count();
    let mut inner = Run::new(INNER_RISERS, if here.logic { 0 } else { inner });
    let mut risers = Run::new(RISERS, risers);
    let mut above = Run::new(H_JUMPERS, above);
    let mut joins: Vec<Join> = Vec::new();
    for (i, e) in ends.iter().enumerate() {
        use End::*;
        let Some((s, t)) = *e else {
            continue;
        };
        let mut join = match (s, t) {
            (Foreign { .. }, Foreign { .. }) => continue,
            (Foreign { dr, dc }, x) | (x, Foreign { dr, dc }) => {
                // A link leaves from a riser on the wire here, or from a
                // vertical track where it crosses the link lanes.
                let mut j = match x {
                    Track { id, at, wire } if !id.horizontal => {
                        Join::dot(&[(wire, [at, LINK_LANES.0])], [at, LINK_LANES.0])
                    }
                    Track { at, wire, .. } => {
                        let p = [risers.take(), at];
                        Join::dot(&[(wire, p)], p)
                    }
                    Local { y, wire, .. } => {
                        let p = [risers.take(), y];
                        Join::dot(&[(wire, p)], p)
                    }
                    Pin {
                        y,
                        wire,
                        long: true,
                    } => {
                        let p = [risers.take(), y];
                        Join::dot(&[(wire, p)], p)
                    }
                    Pin { y, wire, .. } => {
                        Join::jumper(&[(wire, [BLOCK, y])], &[[BLOCK, y], [risers.take(), y]])
                    }
                    Foreign { .. } => unreachable!(),
                };
                let p = *j.dots.last().unwrap();
                let name = if matches!(s, Foreign { .. }) {
                    &here.arcs[i].0
                } else {
                    &here.arcs[i].1
                };
                j.link = Some((p, dr, dc, split_prefix(name).2.to_string(), i));
                j
            }
            (
                Track {
                    id: a,
                    at: p,
                    wire: wa,
                },
                Track {
                    id: b,
                    at: q,
                    wire: wb,
                },
            ) => {
                if a.horizontal != b.horizontal {
                    let dot = if a.horizontal { [q, p] } else { [p, q] };
                    Join::dot(&[(wa, dot), (wb, dot)], dot)
                } else if a.horizontal {
                    let x = above.take();
                    Join::jumper(&[(wa, [x, p]), (wb, [x, q])], &[[x, p], [x, q]])
                } else {
                    let y = here.vv_rows.get(&i).copied().unwrap_or(BAND_BOTTOM);
                    Join::jumper(&[(wa, [p, y]), (wb, [q, y])], &[[p, y], [q, y]])
                }
            }
            (Track { id, at, wire: wt }, Local { y, x, wire: wl })
            | (Local { y, x, wire: wl }, Track { id, at, wire: wt }) => {
                let dot = if id.horizontal { [x, at] } else { [at, y] };
                Join::dot(&[(wt, dot), (wl, dot)], dot)
            }
            (Track { id, at, wire: wt }, Pin { y, wire: wp, long })
            | (Pin { y, wire: wp, long }, Track { id, at, wire: wt }) => {
                if long && id.horizontal {
                    let x = risers.take();
                    Join::jumper(&[(wt, [x, at]), (wp, [x, y])], &[[x, at], [x, y]])
                } else if long {
                    Join::dot(&[(wt, [at, y]), (wp, [at, y])], [at, y])
                } else if id.horizontal {
                    let x = risers.take();
                    let path = [[x, at], [x, y], [BLOCK, y]];
                    Join::jumper(&[(wt, [x, at]), (wp, [BLOCK, y])], &path)
                } else {
                    Join::jumper(&[(wt, [at, y]), (wp, [BLOCK, y])], &[[at, y], [BLOCK, y]])
                }
            }
            (
                Local {
                    y: y1,
                    x: x1,
                    wire: w1,
                },
                Local {
                    y: y2,
                    x: x2,
                    wire: w2,
                },
            ) => {
                // The lower wire's lane meets the upper wire's row.
                let dot = if (y1 - y2).abs() < 1e-6 {
                    [x1.max(x2), y1]
                } else if y1 < y2 {
                    [x2, y1]
                } else {
                    [x1, y2]
                };
                Join::dot(&[(w1, dot), (w2, dot)], dot)
            }
            (
                Local {
                    y: yl,
                    x: xl,
                    wire: wl,
                },
                Pin {
                    y: yp,
                    wire: wp,
                    long,
                },
            )
            | (
                Pin {
                    y: yp,
                    wire: wp,
                    long,
                },
                Local {
                    y: yl,
                    x: xl,
                    wire: wl,
                },
            ) => {
                if long && yp < yl {
                    // The wire's lane crosses the pin's row.
                    Join::dot(&[(wl, [xl, yp]), (wp, [xl, yp])], [xl, yp])
                } else if long {
                    let x = risers.take();
                    Join::jumper(&[(wl, [x, yl]), (wp, [x, yp])], &[[x, yl], [x, yp]])
                } else if (yl - yp).abs() <= 1e-6 {
                    Join::dot(&[(wl, [BLOCK, yl]), (wp, [BLOCK, yp])], [BLOCK, yl])
                } else {
                    let x = risers.take();
                    let path = [[x, yl], [x, yp], [BLOCK, yp]];
                    Join::jumper(&[(wl, [x, yl]), (wp, [BLOCK, yp])], &path)
                }
            }
            (Pin { wire: a, .. }, Pin { wire: b, .. }) => {
                let (ea, eb) = (pin_end(a), pin_end(b));
                if meet(a, b) {
                    Join::dot(&[(a, ea), (b, eb)], ea)
                } else if !here.logic {
                    let x = inner.take();
                    let (pa, pb) = ([x, ea[1]], [x, eb[1]]);
                    Join::jumper(&[(a, pa), (b, pb)], &[pa, pb])
                } else {
                    let x = risers.take();
                    Join::jumper(&[(a, ea), (b, eb)], &[ea, [x, ea[1]], [x, eb[1]], eb])
                }
            }
        };
        join.arc = i as u32;
        joins.push(join);
    }
    // Where neighbours' links arrive: a riser on the wire for each.
    for name in key
        .anchored
        .iter()
        .flat_map(|(name, n)| std::iter::repeat_n(name, *n))
    {
        let Some(&w) = wire_of_name.get(name.as_str()) else {
            continue;
        };
        let x = risers.take();
        let y = here.wires[name].y();
        let mut j = match &here.wires[name] {
            Wire::Local { .. } | Wire::Pin { long: true, .. } => Join {
                stops: vec![(w, [x, y])],
                arc: u32::MAX,
                ..Join::default()
            },
            Wire::Pin { .. } => {
                let mut j = Join::jumper(&[(w, [BLOCK, y])], &[[BLOCK, y], [x, y]]);
                j.dots.pop();
                j.arc = u32::MAX;
                j
            }
        };
        j.anchor = Some(([x, y], name.clone()));
        joins.push(j);
    }
    // Where local wires run on down into a DSP block: their corner.
    for name in key.tapped {
        if let (Some(&w), Some(Wire::Local { y, x })) =
            (wire_of_name.get(name.as_str()), here.wires.get(name))
        {
            joins.push(Join {
                stops: vec![(w, [*x, *y])],
                dots: vec![[*x, *y]],
                tap: true,
                arc: u32::MAX,
                ..Join::default()
            });
        }
    }
    joins.extend(clock_joins);
    // An arc that joins a wire to nothing else is left out, and so on
    // until every wire left joins two things.
    let mut alive = vec![true; joins.len()];
    loop {
        let count = points(&wires, &joins, &alive);
        let mut changed = false;
        for (j, a) in joins.iter().zip(alive.iter_mut()) {
            if *a && !j.lasting() && j.stops.iter().any(|(w, _)| count[*w] < 2) {
                *a = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Groups of wires joined only to each other, reaching no site, no other
    // tile and no link, are left out too.
    let mut group: Vec<usize> = (0..wires.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for (j, _) in joins.iter().zip(&alive).filter(|(_, a)| **a) {
        for w in j.stops.windows(2) {
            let (a, b) = (root(&mut group, w[0].0), root(&mut group, w[1].0));
            group[a] = b;
        }
    }
    let mut reaches = vec![false; wires.len()];
    for (w, d) in wires.iter().enumerate() {
        if d.part == Part::Pin || d.keep_start || d.keep_end {
            let r = root(&mut group, w);
            reaches[r] = true;
        }
    }
    for (j, _) in joins.iter().zip(&alive).filter(|(_, a)| **a) {
        if j.lasting() {
            let r = root(&mut group, j.stops[0].0);
            reaches[r] = true;
        }
    }
    for (j, a) in joins.iter().zip(alive.iter_mut()) {
        if *a
            && j.stops
                .first()
                .is_some_and(|s| !reaches[root(&mut group, s.0)])
        {
            *a = false;
        }
    }
    out.left_out = alive.iter().filter(|a| !**a).count();
    let count = points(&wires, &joins, &alive);
    for (j, _) in joins.iter().zip(&alive).filter(|(_, a)| **a) {
        if j.stops.iter().any(|(w, _)| count[*w] < 2) {
            if let Some(l) = &j.link {
                out.weak_links.insert(l.4);
            }
            if let Some(a) = &j.anchor {
                out.weak_anchors.insert(a.1.clone());
            }
        }
    }
    risers.compact(&mut joins, &alive, false);
    inner.compact(&mut joins, &alive, false);
    above.compact(&mut joins, &alive, true);
    for (j, _) in joins.into_iter().zip(alive).filter(|(_, a)| *a) {
        // A join with no arc (where a link arrives, where a wire runs on
        // into a DSP block) belongs to its one wire.
        let owner = if j.arc == u32::MAX {
            Owner::Wire(j.stops[0].0 as u32)
        } else {
            Owner::Arc(j.arc)
        };
        for (w, p) in j.stops {
            let d = along(&wires[w].points, p);
            wires[w].stops.push(d);
        }
        out.dot_owners
            .extend(std::iter::repeat_n(owner, j.dots.len()));
        out.dots.extend(j.dots);
        for w in j.jumper.windows(2) {
            if w[0] != w[1] {
                out.segments.push((w[0], w[1], Part::Jumper));
                out.owners.push(owner);
            }
        }
        out.links.extend(j.link);
        out.anchors.extend(j.anchor);
    }
    // Each wire between its outermost attachments; ends that continue stay.
    for (k, w) in wires.iter().enumerate() {
        if w.part == Part::Pin && w.stops.is_empty() {
            continue;
        }
        if w.part == Part::Pin {
            out.site_ends.push(w.points[0]);
        }
        let total: f32 = w.points.windows(2).map(|p| length(p[0], p[1])).sum();
        let mut stops = w.stops.clone();
        if w.keep_start {
            stops.push(0.0);
        }
        if w.keep_end {
            stops.push(total);
        }
        let (Some(from), Some(to)) = (
            stops.iter().copied().reduce(f32::min),
            stops.iter().copied().reduce(f32::max),
        ) else {
            continue;
        };
        // Without the slivers rounding leaves at a corner.
        let mut kept = cut(&w.points, from, to);
        kept.dedup_by(|b, a| length(*a, *b) < 1e-5);
        for p in kept.windows(2) {
            out.segments.push((p[0], p[1], w.part));
            out.owners.push(Owner::Wire(k as u32));
        }
    }
    out.boxes = here.boxes.clone();
    out.wires = here
        .tracks
        .iter()
        .flat_map(|id| (0..=id.span).map(|tap| WireKey::Track(*id, tap)))
        .chain(here.wires.keys().map(|n| WireKey::Name(n.clone())))
        .chain(key.clock.iter().map(|r| WireKey::Name(r.owner.clone())))
        .collect();
    out.arcs = here.shared_arcs.clone();
    out.fixed = here.fixed.clone();
    // Inside the slices of a logic tile, the connections no tile type
    // names, drawn whole from where their value comes to where it goes.
    if here.logic {
        for (name, points) in slice_internals() {
            let k = out.wires.len() as u32;
            for w in points.windows(2) {
                out.segments.push((w[0], w[1], Part::Internal));
                out.owners.push(Owner::Wire(k));
            }
            let (from, to) = (points[0], *points.last().unwrap());
            out.site_ends.extend([from, to]);
            out.internals.push((k, from, to));
            out.wires.push(WireKey::Name(name));
        }
    }
    out
}

/// The fabric of the die.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Fabric {
    pub rows: u16,
    pub cols: u16,
    /// Tile types at each position, row major.
    pub kinds: Vec<Vec<String>>,
    pub templates: Vec<Template>,
    /// The template of each position.
    pub template_at: Vec<u32>,
    pub links: Vec<Link>,
    /// Links that leave each position, as ranges into `links`.
    pub links_at: Vec<(u32, u32)>,
    /// The links through each position, as ranges into `through`.
    through_at: Vec<(u32, u32)>,
    through: Vec<u32>,
    /// Where each row starts, and after the last where the die ends.
    pub row_top: Vec<f32>,
    /// The DSP blocks, drawn whole under the interconnect of their rows.
    pub macros: Vec<Macro>,
    /// Their fixed connections within one part (drawn under it), and those
    /// that lead out of the blocks of a row (not drawn).
    pub dsp_inside: usize,
    pub dsp_lost: usize,
    /// Arcs to a neighbour's wire that the neighbour does not have.
    pub broken_links: usize,
    /// Those arcs, counted by (tile types here, wire, tile types there).
    pub broken: BTreeMap<(String, String, String), usize>,
    /// Sites of the die, slices included.
    pub sites: usize,
    pub lanes: Lanes,
    /// Arcs of the die.
    pub arcs: usize,
}

impl Fabric {
    fn index(&self, row: u16, col: u16) -> usize {
        row as usize * self.cols as usize + col as usize
    }
    pub fn types_at(&self, row: u16, col: u16) -> &[String] {
        &self.kinds[self.index(row, col)]
    }
    pub fn template(&self, row: u16, col: u16) -> &Template {
        &self.templates[self.template_at[self.index(row, col)] as usize]
    }
    pub fn links_from(&self, row: u16, col: u16) -> &[Link] {
        let (a, b) = self.links_at[self.index(row, col)];
        &self.links[a as usize..b as usize]
    }
    /// The links that pass through a tile, by index into `links`.
    pub fn links_through(&self, row: u16, col: u16) -> &[u32] {
        let (a, b) = self.through_at[self.index(row, col)];
        &self.through[a as usize..b as usize]
    }
    pub fn row_height(&self, row: u16) -> f32 {
        self.row_top[row as usize + 1] - self.row_top[row as usize]
    }
    /// The row a height of the die lies in.
    pub fn row_at(&self, y: f32) -> u16 {
        (self.row_top.partition_point(|t| *t <= y).max(1) - 1).min(self.rows as usize - 1) as u16
    }
    /// Size of the die in die units.
    pub fn size(&self) -> [f32; 2] {
        [self.cols as f32 * TILE_W, self.row_top[self.rows as usize]]
    }
}

/// The pieces of a polyline of horizontal and vertical pieces in die units,
/// cut at the tiles they pass: (position, piece in its tile's coordinates).
fn clip(row_top: &[f32], cols: u16, points: &[[f32; 2]]) -> Vec<(usize, Segment)> {
    let rows = row_top.len() - 1;
    let row_at = |y: f32| (row_top.partition_point(|t| *t <= y).max(1) - 1).min(rows - 1);
    let mut out = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a[1] - b[1]).abs() <= EPS {
            let r = row_at(a[1]);
            let (lo, hi) = (a[0].min(b[0]), a[0].max(b[0]));
            let mut at = lo;
            while at < hi - EPS {
                let c = (at / TILE_W + 1e-6).floor();
                let end = ((c + 1.0) * TILE_W).min(hi);
                if c >= 0.0 && c < cols as f32 {
                    let y = a[1] - row_top[r];
                    let piece = ([at - c * TILE_W, y], [end - c * TILE_W, y], Part::Jumper);
                    out.push((r * cols as usize + c as usize, piece));
                }
                at = end;
            }
        } else {
            let c = (a[0] / TILE_W).floor();
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            let mut at = lo;
            while at < hi - EPS {
                let r = row_at(at + 1e-6);
                let end = row_top[r + 1].min(hi);
                if c >= 0.0 && c < cols as f32 {
                    let x = a[0] - c * TILE_W;
                    let piece = ([x, at - row_top[r]], [x, end - row_top[r]], Part::Jumper);
                    out.push((r * cols as usize + c as usize, piece));
                }
                at = end;
            }
        }
    }
    out
}

/// A DSP block drawn as one site, under the interconnect of its row, in die
/// units: its parts, the wires from the CIB into their ports, and every
/// fixed connection between its parts' ports.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub row: u16,
    /// The blocks' outlines, with labels.
    pub frames: Vec<SiteBox>,
    pub parts: Vec<SiteBox>,
    /// From each CIB wire into the block: a trunk down along the wire's lane
    /// and a branch along a lane of the port channel to each of its ports.
    pub wires: Vec<Vec<[f32; 2]>>,
    /// The fixed connections between ports of the parts (one part's, or two
    /// parts'): a stub down from each port on a part's bottom edge, and for
    /// each connection a piece along a lane of the channel below the parts
    /// from one stub to the other.
    pub inner: Vec<Vec<[f32; 2]>>,
    pub dots: Vec<[f32; 2]>,
    /// What each of `wires` and `inner` draws.
    pub wire_pieces: Vec<MacroPiece>,
    pub inner_pieces: Vec<MacroPiece>,
}

/// A wire of a position: its index and name there.
pub type PlaceName = (usize, String);

/// What a piece of a DSP block draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MacroPiece {
    /// A CIB wire running down into the block.
    Trunk(PlaceName),
    /// From a CIB wire to the port it feeds.
    Feed(PlaceName, PlaceName),
    /// A fixed connection from one port to another.
    Join(PlaceName, PlaceName),
    /// A port's stub below its part.
    Stub(PlaceName),
}

/// The part of a DSP block a wire of it belongs to: a multiplier, adder or
/// pre-adder by its suffix (`JA3_MULT18`), else the family of an inner node
/// (`JMULTA4` → `JMULTA`).
fn dsp_part(name: &str) -> String {
    for suffix in ["MULT18", "MULT9", "PRADD18", "PRADD9", "ALU54", "ALU24"] {
        if name.ends_with(&format!("_{suffix}")) {
            return suffix.to_string();
        }
    }
    name.trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

/// A part of a row's DSP blocks, or a CIB wire: its column and name.
type PartKey = (i32, String);
/// A port of a part.
type PortKey = (PartKey, String);
/// Where a local wire of a position is: its lane and row.
type LocalAt<'a> = dyn Fn(usize, &str) -> Option<(f32, f32)> + 'a;

/// The DSP blocks of a row laid out in tile coordinates (y from the row's
/// top, x in die units), and the local wires that run into them.
struct DspPlan {
    macros: Vec<Macro>,
    tapped: Vec<(usize, String)>,
    /// The least height the row needs.
    needed: f32,
    /// Fixed connections within one part, and those that lead out of the
    /// row's blocks (not drawn).
    inside_parts: usize,
    lost: usize,
}

/// What the parts of a row's DSP blocks are and what joins them.
struct DspNets {
    /// The ports of each part fed from the CIB, and those joined to ports of
    /// parts.
    top: BTreeMap<PartKey, BTreeSet<String>>,
    bottom: BTreeMap<PartKey, BTreeSet<String>>,
    /// (part, CIB wire, port).
    feeds: BTreeSet<(PartKey, PartKey, String)>,
    /// (part, port) to (part, port).
    joins: BTreeSet<(PortKey, PortKey)>,
    inside_parts: usize,
    lost: usize,
}

fn dsp_nets(
    db: &Database,
    row: u16,
    cols: u16,
    kinds: &[Vec<String>],
    local_at: &LocalAt<'_>,
    inside_dsp: &HashSet<String>,
) -> DspNets {
    let index = |c: i32| row as usize * cols as usize + c as usize;
    let dsp_col = |c: i32| {
        c >= 0 && c < cols as i32 && kinds[index(c)].iter().any(|k| k.starts_with("MIB_DSP"))
    };
    let mut nets = DspNets {
        top: BTreeMap::new(),
        bottom: BTreeMap::new(),
        feeds: BTreeSet::new(),
        joins: BTreeSet::new(),
        inside_parts: 0,
        lost: 0,
    };
    for c in 0..cols as i32 {
        if !dsp_col(c) {
            continue;
        }
        for kind in kinds[index(c)].iter().filter(|k| k.starts_with("MIB_DSP")) {
            for (s, t) in routing(db, kind) {
                let ends = [split_prefix(s), split_prefix(t)];
                let inner = ends.map(|e| inside_dsp.contains(e.2));
                if !inner[0] && !inner[1] {
                    continue;
                }
                if ends.iter().any(|e| e.0 != 0) {
                    nets.lost += 1;
                    continue;
                }
                let part = |e: &(i16, i16, &str)| (c + e.1 as i32, dsp_part(e.2));
                if inner[0] && inner[1] {
                    let (a, b) = (part(&ends[0]), part(&ends[1]));
                    if !dsp_col(a.0) || !dsp_col(b.0) {
                        nets.lost += 1;
                        continue;
                    }
                    nets.inside_parts += usize::from(a == b);
                    for (k, e) in [(&a, &ends[0]), (&b, &ends[1])] {
                        nets.top.entry(k.clone()).or_default();
                        nets.bottom
                            .entry(k.clone())
                            .or_default()
                            .insert(e.2.to_string());
                    }
                    nets.joins
                        .insert(((a, ends[0].2.to_string()), (b, ends[1].2.to_string())));
                } else {
                    let (p, j) = if inner[0] {
                        (&ends[0], &ends[1])
                    } else {
                        (&ends[1], &ends[0])
                    };
                    let at = part(p);
                    let jc = c + j.1 as i32;
                    if !dsp_col(at.0)
                        || jc < 0
                        || jc >= cols as i32
                        || local_at(index(jc), j.2).is_none()
                    {
                        nets.lost += 1;
                        continue;
                    }
                    nets.bottom.entry(at.clone()).or_default();
                    nets.top
                        .entry(at.clone())
                        .or_default()
                        .insert(p.2.to_string());
                    nets.feeds
                        .insert((at, (jc, j.2.to_string()), p.2.to_string()));
                }
            }
        }
    }
    nets
}

/// Lanes handed out over spans, a lane free again past its last span's end:
/// the lane of each span and how many lanes.
fn pack_spans(spans: &[(f32, f32)]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a].0.total_cmp(&spans[b].0));
    let mut ends: Vec<f32> = Vec::new();
    let mut lane_of = vec![0; spans.len()];
    for k in order {
        let (lo, hi) = spans[k];
        match ends.iter().position(|&e| e < lo - 1e-4) {
            Some(l) => {
                ends[l] = hi;
                lane_of[k] = l;
            }
            None => {
                ends.push(hi);
                lane_of[k] = ends.len() - 1;
            }
        }
    }
    (lane_of, ends.len())
}

/// The height of a DSP block's parts, at the least.
const PART_H: f32 = 0.15;

/// Lays out a row's DSP blocks in a row `height` high (at least what they
/// need; the room left over spreads the channels and the parts).
fn plan_dsp(
    model: &DeviceModel,
    row: u16,
    cols: u16,
    kinds: &[Vec<String>],
    local_at: &LocalAt<'_>,
    nets: &DspNets,
    height: Option<f32>,
) -> DspPlan {
    let index = |c: i32| row as usize * cols as usize + c as usize;
    let dsp_col = |c: i32| {
        c >= 0 && c < cols as i32 && kinds[index(c)].iter().any(|k| k.starts_with("MIB_DSP"))
    };
    // The parts side by side in the block of their column, at the width
    // their ports need.
    let mut x_of: BTreeMap<PartKey, (f32, f32)> = BTreeMap::new();
    let mut by_col: BTreeMap<i32, Vec<PartKey>> = BTreeMap::new();
    for key in nets.top.keys() {
        by_col.entry(key.0).or_default().push(key.clone());
    }
    // Parts fed from the CIB first: their ports stay within the block, clear
    // of the next column's lanes, whose wires run down to the port channel.
    // Below that channel the parts reach on into those lanes.
    for list in by_col.values_mut() {
        list.sort_by_key(|k| (nets.top[k].is_empty(), k.1.clone()));
    }
    let limit = |c: i32| (c + 1) as f32 * TILE_W - 0.005;
    for (c, list) in &by_col {
        let (lo, hi) = (
            *c as f32 * TILE_W + BLOCK + 0.01,
            (*c + 1) as f32 * TILE_W + LOCAL_LANES.1,
        );
        let weight = |k: &PartKey| nets.top[k].len().max(nets.bottom[k].len()).max(6) as f32;
        let gap = 0.006;
        // Spread over [from, to] at the widths their weights ask.
        let mut spread = |parts: &[PartKey], from: f32, to: f32| {
            let total: f32 = parts.iter().map(weight).sum();
            let room = to - from - gap * parts.len().saturating_sub(1) as f32;
            let mut x = from;
            for k in parts {
                let w = room * weight(k) / total;
                x_of.insert(k.clone(), (x, x + w));
                x += w + gap;
            }
        };
        let fed = list.iter().take_while(|k| !nets.top[*k].is_empty()).count();
        let all: f32 = list.iter().map(weight).sum();
        let fed_share: f32 = list[..fed].iter().map(weight).sum::<f32>() / all;
        if fed == list.len() || lo + (hi - lo) * fed_share <= limit(*c) {
            // Those fed from the CIB end within the block anyway.
            spread(list, lo, if fed == list.len() { limit(*c) } else { hi });
        } else {
            spread(&list[..fed], lo, limit(*c));
            spread(&list[fed..], limit(*c) + gap, hi);
        }
    }
    // Ports along the parts' edges: from the CIB on top, to other ports
    // below.
    let along = |edge: &BTreeMap<PartKey, BTreeSet<String>>, clear: bool| {
        let mut at: HashMap<(PartKey, String), f32> = HashMap::new();
        for (k, list) in edge {
            let (x0, mut x1) = x_of[k];
            if clear {
                x1 = x1.min(limit(k.0));
            }
            let mut names: Vec<&String> = list.iter().collect();
            names.sort_by_key(|n| natural(n));
            for (i, n) in names.iter().enumerate() {
                at.insert((k.clone(), (*n).clone()), lane((x0, x1), i, names.len()));
            }
        }
        at
    };
    let (top_x, bottom_x) = (along(&nets.top, true), along(&nets.bottom, false));
    // The lanes of the port channel, one for each branch from a trunk to a
    // port, and of the channel below, one for each connection.
    let feeds: Vec<&(PartKey, PartKey, String)> = nets.feeds.iter().collect();
    let trunk_x = |jc: i32, j: &str| jc as f32 * TILE_W + local_at(index(jc), j).unwrap().0;
    let branch: Vec<(f32, f32)> = feeds
        .iter()
        .map(|(part, (jc, j), port)| {
            let (t, p) = (trunk_x(*jc, j), top_x[&(part.clone(), port.clone())]);
            (t.min(p), t.max(p))
        })
        .collect();
    let (branch_lane, ports_lanes) = pack_spans(&branch);
    let joins: Vec<&(PortKey, PortKey)> = nets.joins.iter().collect();
    let link: Vec<(f32, f32)> = joins
        .iter()
        .map(|(a, b)| {
            let (xa, xb) = (bottom_x[a], bottom_x[b]);
            (xa.min(xb), xa.max(xb))
        })
        .collect();
    let (join_lane, join_lanes) = pack_spans(&link);
    // The room: fixed margins, then the channels and parts, spread over
    // what the row has beyond its need.
    let margins = 0.006 + 0.004 + 0.006 + 0.004 + 0.025;
    let base = (ports_lanes + join_lanes) as f32 * ROW_GAP + PART_H;
    let needed = TILE_H + margins + base;
    let scale = height.map_or(1.0, |h| ((h - TILE_H - margins) / base).max(1.0));
    let pitch = ROW_GAP * scale;
    let top = TILE_H + 0.006;
    let part_top = top + ports_lanes as f32 * pitch + 0.004;
    let part_bottom = part_top + PART_H * scale;
    let join_top = part_bottom + 0.006;
    let bottom = join_top + join_lanes as f32 * pitch + 0.004;
    // From the CIB: trunks and branches.
    let mut wires = Vec::new();
    let mut wire_pieces = Vec::new();
    let mut inner_pieces = Vec::new();
    let mut dots = Vec::new();
    let mut tapped = Vec::new();
    let mut trunks: BTreeMap<PartKey, (f32, f32, f32)> = BTreeMap::new();
    for (k, (part, (jc, j), port)) in feeds.iter().enumerate() {
        let (lane_x, row_y) = local_at(index(*jc), j).unwrap();
        let t = *jc as f32 * TILE_W + lane_x;
        let y = top + (branch_lane[k] as f32 + 0.5) * pitch;
        let px = top_x[&(part.clone(), port.clone())];
        wires.push(vec![[t, y], [px, y], [px, part_top]]);
        wire_pieces.push(MacroPiece::Feed(
            (index(*jc), j.clone()),
            (index(part.0), port.clone()),
        ));
        dots.push([t, y]);
        let e = trunks.entry((*jc, j.clone())).or_insert((t, row_y, y));
        e.2 = e.2.max(y);
    }
    for ((jc, j), (x, y0, y1)) in trunks {
        wires.push(vec![[x, y0], [x, y1]]);
        wire_pieces.push(MacroPiece::Trunk((index(jc), j.clone())));
        tapped.push((index(jc), j));
    }
    // Between ports: a stub from each port down to its deepest connection,
    // and a piece along the lane of each connection with a dot at each end.
    let mut inner = Vec::new();
    let mut stubs: BTreeMap<(PartKey, String), f32> = BTreeMap::new();
    for (k, (a, b)) in joins.iter().enumerate() {
        let y = join_top + (join_lane[k] as f32 + 0.5) * pitch;
        let (xa, xb) = (bottom_x[a], bottom_x[b]);
        inner.push(vec![[xa, y], [xb, y]]);
        inner_pieces.push(MacroPiece::Join(
            (index(a.0 .0), a.1.clone()),
            (index(b.0 .0), b.1.clone()),
        ));
        dots.push([xa, y]);
        dots.push([xb, y]);
        for end in [a, b] {
            let deepest = stubs.entry(end.clone()).or_insert(y);
            *deepest = deepest.max(y);
        }
    }
    for (end, y) in stubs {
        let x = bottom_x[&end];
        inner.push(vec![[x, part_bottom], [x, y]]);
        inner_pieces.push(MacroPiece::Stub((index(end.0 .0), end.1.clone())));
    }
    // The parts, named by the site they are or the node they stand for.
    let mut names_at: HashMap<i32, Vec<(String, String)>> = HashMap::new();
    for (_, r) in model.resources() {
        if r.location.row as u16 == row {
            names_at
                .entry(r.location.col as i32)
                .or_default()
                .push((r.family.clone(), r.name.clone()));
        }
    }
    let parts = x_of
        .iter()
        .map(|(k, (x0, x1))| {
            let label = names_at
                .get(&k.0)
                .and_then(|l| l.iter().find(|(f, _)| *f == k.1))
                .map_or_else(
                    || k.1.trim_start_matches('J').to_string(),
                    |(_, n)| n.clone(),
                );
            SiteBox {
                min: [*x0, part_top],
                max: [*x1, part_bottom],
                kind: BoxKind::Site,
                label,
            }
        })
        .collect();
    // A block from each column with its first part.
    let mut frames = Vec::new();
    let mut c = 0;
    while c < cols as i32 {
        if kinds[index(c)].iter().any(|k| k == "MIB_DSP0") {
            let last = (c..c + 9).filter(|&k| dsp_col(k)).max().unwrap_or(c);
            frames.push(SiteBox {
                min: [c as f32 * TILE_W + 0.004, TILE_H + 0.002],
                max: [(last + 1) as f32 * TILE_W - 0.004, bottom],
                kind: BoxKind::Slice,
                label: format!("DSP R{row}C{c}"),
            });
            c = last + 1;
        } else {
            c += 1;
        }
    }
    DspPlan {
        macros: vec![Macro {
            row,
            frames,
            parts,
            wires,
            inner,
            dots,
            wire_pieces,
            inner_pieces,
        }],
        tapped,
        needed,
        inside_parts: nets.inside_parts,
        lost: nets.lost,
    }
}

/// A name of a neighbour's wire other than a track: its offset and name.
fn away(name: &str) -> Option<(i16, i16, &str)> {
    let (dr, dc, base) = split_prefix(name);
    ((dr, dc) != (0, 0) && parse_track(base).is_none()).then_some((dr, dc, base))
}

/// The tile types of a position, its sites other than slices, as
/// (family, name), and the wires of its neighbours it draws as its own
/// (those their own tiles do not name).
type Shape = (Vec<String>, Vec<(String, Vec<String>)>, Vec<String>);

/// The clock tree's taps: their column and the columns their left and
/// right branches drive; and its quadrants: their rows and columns.
type Taps = Vec<(i32, (i32, i32), (i32, i32))>;
type Quadrants = Vec<((i32, i32), (i32, i32))>;

fn clock_topology(db: &Database) -> (Taps, Quadrants) {
    let g = &db.globals;
    let num = |v: &serde_json::Value, k: &str| v[k].as_i64().unwrap_or(-1) as i32;
    let taps = g["taps"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(name, t)| {
                    Some((
                        name.strip_prefix('C')?.parse().ok()?,
                        (num(t, "lx0"), num(t, "lx1")),
                        (num(t, "rx0"), num(t, "rx1")),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let quadrants = g["quadrants"]
        .as_object()
        .map(|m| {
            m.values()
                .map(|q| ((num(q, "y0"), num(q, "y1")), (num(q, "x0"), num(q, "x1"))))
                .collect()
        })
        .unwrap_or_default();
    (taps, quadrants)
}

/// The fabric of the whole die.
pub fn build(model: &DeviceModel) -> Fabric {
    let db = model.database();
    let lanes = Lanes::new(db);
    let (mut rows, mut cols) = (0, 0);
    for (_, tile) in model.tiles() {
        rows = rows.max(tile.row as u16 + 1);
        cols = cols.max(tile.col as u16 + 1);
    }
    let positions = rows as usize * cols as usize;
    // The DSP blocks' own wires: those of their tile types that no CIB of
    // theirs names.
    let mut inside_dsp: HashSet<String> = HashSet::new();
    let mut cib_dsp: HashSet<String> = HashSet::new();
    for kind in db.types.keys() {
        let set = if kind.starts_with("MIB_DSP") {
            &mut inside_dsp
        } else if kind.starts_with("CIB_DSP") {
            &mut cib_dsp
        } else {
            continue;
        };
        for (source, sink) in routing(db, kind) {
            for name in [source, sink] {
                let base = split_prefix(name).2;
                if parse_track(base).is_none() {
                    set.insert(base.to_string());
                }
            }
        }
    }
    inside_dsp.retain(|n| !cib_dsp.contains(n));
    let mut kinds = vec![Vec::new(); positions];
    for (_, tile) in model.tiles() {
        kinds[tile.row as usize * cols as usize + tile.col as usize].push(tile.kind.clone());
    }
    for k in &mut kinds {
        k.sort();
    }
    // The wires other than tracks each position's tile types name, here or
    // (with a prefix) in a neighbour.
    let mut names_of: BTreeMap<&Vec<String>, BTreeSet<&str>> = BTreeMap::new();
    for k in &kinds {
        names_of.entry(k).or_insert_with(|| {
            let mut names = BTreeSet::new();
            for kind in k {
                for (source, sink) in routing(db, kind) {
                    for name in [source, sink] {
                        if parse_track(split_prefix(name).2).is_none() {
                            names.insert(name);
                        }
                    }
                }
            }
            names
        });
    }
    // The sites other than slices, drawn where the wires of their ports are:
    // a port's wire is a local wire of the position that names it as the
    // same wire of the device. Sites with no ports (the frames of a DSP)
    // are not drawn; those whose ports no tile names, bare.
    let mut sites = 0;
    let mut ports_by_wire: HashMap<(i16, i16, String), Vec<(usize, String)>> = HashMap::new();
    let mut wanted: HashSet<&str> = HashSet::new();
    let mut resources = Vec::new();
    for (id, r) in model.resources() {
        sites += usize::from(r.family != "PLC2");
        // DSP blocks are drawn whole, apart.
        let dsp = r.family == "MULT18" || r.family == "ALU54" || r.family.starts_with("DSP");
        if r.family == "PLC2" || r.family == "SLICE" || dsp {
            continue;
        }
        let ports = model.resource_ports(id);
        if ports.is_empty() {
            continue;
        }
        resources.push((r, ports));
    }
    for (_, ports) in &resources {
        for p in ports {
            wanted.insert(p.wire.name.as_str());
        }
    }
    let index_of = |r: usize, c: usize| r * cols as usize + c;
    for r in 0..rows as usize {
        for c in 0..cols as usize {
            let at = crate::model::topology::Location {
                row: r as i16,
                col: c as i16,
            };
            for name in &names_of[&kinds[index_of(r, c)]] {
                if !wanted.contains(split_prefix(name).2) {
                    continue;
                }
                if let Ok(crate::model::topology::Endpoint::Wire(w)) = model.wire(at, name) {
                    ports_by_wire
                        .entry((w.location.row, w.location.col, w.name))
                        .or_default()
                        .push((index_of(r, c), name.to_string()));
                }
            }
        }
    }
    // The sites drawn at each position, by name and then by resource (four
    // DLLDEL of one edge have one name and are four sites), with where each
    // is and the names of its ports here.
    let mut here_sites: Vec<BTreeMap<(String, String), Placed>> = vec![BTreeMap::new(); positions];
    let site = |r: &crate::model::Resource| {
        (
            (r.name.clone(), r.key.clone()),
            (
                format!("R{}C{}", r.location.row, r.location.col),
                Vec::new(),
            ),
        )
    };
    // The names that are a site's port at each position, and the other
    // names of those wires elsewhere, which link to them.
    let mut hosted: Vec<BTreeSet<String>> = vec![BTreeSet::new(); positions];
    let mut redirect: HashMap<(usize, String), (usize, String)> = HashMap::new();
    for (r, ports) in &resources {
        let home = index_of(r.location.row as usize, r.location.col as usize);
        let mut placed = false;
        for p in ports {
            let key = (
                p.wire.location.row,
                p.wire.location.col,
                p.wire.name.clone(),
            );
            let Some(found) = ports_by_wire.get(&key) else {
                continue;
            };
            // The site's own position if it names the wire, else the nearest;
            // a name without a prefix first.
            let (i, name) = found
                .iter()
                .min_by_key(|(i, name)| {
                    let (fr, fc) = ((i / cols as usize) as i32, (i % cols as usize) as i32);
                    let (hr, hc) = ((home / cols as usize) as i32, (home % cols as usize) as i32);
                    let prefixed = split_prefix(name).0 != 0 || split_prefix(name).1 != 0;
                    ((fr - hr).abs() + (fc - hc).abs(), prefixed)
                })
                .unwrap();
            let (key, empty) = site(r);
            here_sites[*i]
                .entry(key)
                .or_insert(empty)
                .1
                .push(name.clone());
            hosted[*i].insert(name.clone());
            for (j, other) in found {
                let prefixed = split_prefix(other).0 != 0 || split_prefix(other).1 != 0;
                if *j != *i && prefixed {
                    redirect.insert((*j, other.clone()), (*i, name.clone()));
                }
            }
            placed = true;
        }
        if !placed && home < positions {
            let (key, empty) = site(r);
            here_sites[home].entry(key).or_insert(empty);
        }
    }
    let mut shapes: Vec<Shape> = kinds
        .iter()
        .zip(here_sites)
        .map(|(k, s)| {
            // A name that more sites here have is told apart by where each
            // site is: `DLLDEL R48C0`.
            let mut count: HashMap<String, usize> = HashMap::new();
            for (name, _) in s.keys() {
                *count.entry(name.clone()).or_default() += 1;
            }
            let s = s
                .into_iter()
                .map(|((name, _), (at, mut ports))| {
                    ports.sort();
                    ports.dedup();
                    let label = if count[&name] > 1 {
                        format!("{name} {at}")
                    } else {
                        name
                    };
                    (label, ports)
                })
                .collect();
            (k.clone(), s, Vec::new())
        })
        .collect();
    // Wires that tiles name only in a neighbour whose own tiles do not
    // name them (`S1W2_JPADDI` of an ECLK tile, the pad's way into the
    // clocks): each is drawn as a local wire of the position nearest to it
    // of those that name it, and the others link to it there.
    {
        let first: BTreeMap<&Shape, Layout> = shapes
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|shape| {
                let l = layout(db, &shape.0, &shape.1, &[], &HashSet::new(), &inside_dsp);
                (shape, l)
            })
            .collect();
        // (position, name) of each such wire, and (distance, position,
        // name) of each that names it.
        type Refs = Vec<(i32, usize, String)>;
        let mut phantoms: BTreeMap<(usize, String), Refs> = BTreeMap::new();
        for r in 0..rows as i32 {
            for c in 0..cols as i32 {
                let i = index_of(r as usize, c as usize);
                for name in &names_of[&kinds[i]] {
                    let Some((dr, dc, base)) = away(name) else {
                        continue;
                    };
                    let (rr, cc) = (r + dr as i32, c + dc as i32);
                    if wanted.contains(base)
                        || inside_dsp.contains(base)
                        || redirect.contains_key(&(i, name.to_string()))
                        || rr < 0
                        || cc < 0
                        || rr >= rows as i32
                        || cc >= cols as i32
                    {
                        continue;
                    }
                    let j = index_of(rr as usize, cc as usize);
                    if first[&shapes[j]].wires.contains_key(base) {
                        continue;
                    }
                    phantoms.entry((j, base.to_string())).or_default().push((
                        (dr.abs() + dc.abs()) as i32,
                        i,
                        name.to_string(),
                    ));
                }
            }
        }
        let mut hosted: Vec<BTreeSet<String>> = vec![BTreeSet::new(); positions];
        for (_, mut refs) in phantoms {
            refs.sort();
            let (_, host, name) = refs[0].clone();
            hosted[host].insert(name.clone());
            for (_, i, other) in refs.into_iter().skip(1) {
                redirect.insert((i, other), (host, name.clone()));
            }
        }
        drop(first);
        for (shape, h) in shapes.iter_mut().zip(hosted) {
            shape.2 = h.into_iter().collect();
        }
    }
    // The wires some tile names in a neighbour.
    let mut named_away: HashSet<String> = HashSet::new();
    for kind in db.types.keys() {
        for (source, sink) in routing(db, kind) {
            for name in [source, sink] {
                if let Some((_, _, base)) = away(name) {
                    named_away.insert(base.to_string());
                }
            }
        }
    }
    for (i, name) in redirect.values() {
        let _ = i;
        named_away.insert(name.clone());
    }
    let mut layouts: BTreeMap<&Shape, Layout> = BTreeMap::new();
    for shape in &shapes {
        if !layouts.contains_key(shape) {
            layouts.insert(
                shape,
                layout(db, &shape.0, &shape.1, &shape.2, &named_away, &inside_dsp),
            );
        }
    }
    // The clock tree. Its branches along the rows and its spines down the
    // tap columns are drawn as runs of their own (below); the rest of it
    // (the quadrants' clocks from the centre to the spines, the centre's
    // own wires, the PLLs' outputs) are conductors that several positions
    // name: each position links to the next in a tree over them, as an arc
    // to a neighbour's wire would (`G_HPRX0000` to `=G_HPRX0000`).
    let (taps, quadrants) = clock_topology(db);
    let tap_cols: HashSet<i32> = taps.iter().map(|t| t.0).collect();
    let mut groups: BTreeMap<(i16, i16, String), Vec<(usize, String)>> = BTreeMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let at = crate::model::topology::Location {
            row: (i / cols as usize) as i16,
            col: (i % cols as usize) as i16,
        };
        for name in layouts[shape].wires.keys() {
            if !name.starts_with("G_") || name.starts_with("G_HPBX") {
                continue;
            }
            let Ok(crate::model::topology::Endpoint::Wire(w)) = model.wire(at, name) else {
                continue;
            };
            let w = model.clock_segment(&w).ok().flatten().unwrap_or(w);
            groups
                .entry((w.location.row, w.location.col, w.name))
                .or_default()
                .push((i, name.clone()));
        }
    }
    // Where each link of the clock goes: (position, name) to (position,
    // name), over a spanning tree of the positions, each to its parent.
    let mut bridge: HashMap<(usize, String), (usize, String)> = HashMap::new();
    for (_, mut nodes) in groups {
        nodes.sort();
        nodes.dedup_by_key(|n| n.0);
        // A spine is drawn down its tap column: only the spine's own tile
        // off the column links, to the column in its row.
        let on_tap = |i: usize| tap_cols.contains(&((i % cols as usize) as i32));
        if nodes.iter().any(|n| n.1.starts_with("G_VPTX")) {
            for (i, name) in nodes.iter().filter(|n| !on_tap(n.0)) {
                let row = i / cols as usize;
                if let Some(n) = nodes
                    .iter()
                    .find(|n| on_tap(n.0) && n.0 / cols as usize == row)
                {
                    bridge.insert((*i, name.clone()), n.clone());
                }
            }
            continue;
        }
        if nodes.len() < 2 {
            continue;
        }
        let rc = |i: usize| ((i / cols as usize) as i32, (i % cols as usize) as i32);
        let far = |a: usize, b: usize| {
            let ((ra, ca), (rb, cb)) = (rc(a), rc(b));
            (ra - rb).abs() + (ca - cb).abs()
        };
        let mut joined = vec![false; nodes.len()];
        joined[0] = true;
        for _ in 1..nodes.len() {
            let (k, parent) = (0..nodes.len())
                .filter(|&k| !joined[k])
                .flat_map(|k| (0..nodes.len()).filter(|&p| joined[p]).map(move |p| (k, p)))
                .min_by_key(|&(k, p)| (far(nodes[k].0, nodes[p].0), k, p))
                .unwrap();
            joined[k] = true;
            bridge.insert(nodes[k].clone(), nodes[parent].clone());
        }
    }
    let mut bridged: HashMap<&Shape, BTreeSet<String>> = HashMap::new();
    for (i, name) in bridge.keys() {
        bridged.entry(&shapes[*i]).or_default().insert(name.clone());
    }
    for (shape, names) in bridged {
        let l = layouts.get_mut(shape).unwrap();
        let mut fixed = (*l.fixed).clone();
        for name in names {
            l.arcs.push((name.clone(), format!("={name}")));
            fixed.push(true);
        }
        l.shared_arcs = std::sync::Arc::new(l.arcs.clone());
        l.fixed = std::sync::Arc::new(fixed);
    }
    // The DSP blocks, under the interconnect of their rows, which are as
    // high as they need.
    let local_at = |i: usize, name: &str| match layouts[&shapes[i]].wires.get(name) {
        Some(Wire::Local { y, x }) => Some((*x, *y)),
        _ => None,
    };
    let mut heights: Vec<f32> = vec![TILE_H; rows as usize];
    let mut tapped: Vec<BTreeSet<String>> = vec![BTreeSet::new(); positions];
    let dsp_rows: Vec<u16> = (0..rows)
        .filter(|&r| {
            (0..cols).any(|c| {
                kinds[r as usize * cols as usize + c as usize]
                    .iter()
                    .any(|k| k.starts_with("MIB_DSP"))
            })
        })
        .collect();
    let nets: Vec<DspNets> = dsp_rows
        .iter()
        .map(|&r| dsp_nets(db, r, cols, &kinds, &local_at, &inside_dsp))
        .collect();
    // The die square: the DSP rows share the height the others leave, or
    // are as high as they need.
    let width = cols as f32 * TILE_W;
    let share =
        (width - (rows as usize - dsp_rows.len()) as f32 * TILE_H) / dsp_rows.len().max(1) as f32;
    let mut plans = Vec::new();
    let (mut dsp_inside, mut dsp_lost) = (0, 0);
    for (&r, n) in dsp_rows.iter().zip(&nets) {
        let need = plan_dsp(model, r, cols, &kinds, &local_at, n, None).needed;
        let h = share.max(need);
        let plan = plan_dsp(model, r, cols, &kinds, &local_at, n, Some(h));
        heights[r as usize] = h;
        for (i, name) in &plan.tapped {
            tapped[*i].insert(name.clone());
        }
        dsp_inside += plan.inside_parts;
        dsp_lost += plan.lost;
        plans.push(plan);
    }
    let mut row_top = vec![0.0f32];
    for h in &heights {
        row_top.push(row_top.last().unwrap() + h);
    }
    let index = |r: i32, c: i32| r as usize * cols as usize + c as usize;
    let inside = |r: i32, c: i32| r >= 0 && c >= 0 && r < rows as i32 && c < cols as i32;
    let at = |r: i32, c: i32| -> Option<&Layout> {
        inside(r, c).then(|| &layouts[&shapes[index(r, c)]])
    };
    // The runs of the clock tree through each tile: a branch of each row
    // from each tap, left and right, over the tiles whose local wire of its
    // channel it meets, the tap's own (`L_HPBX…`, `R_HPBX…`) included; a
    // spine down each tap column in each quadrant, over the tap tiles.
    let mut clock: Vec<Vec<ClockRun>> = vec![Vec::new(); positions];
    let local = |r: i32, c: i32, name: &str| {
        at(r, c).is_some_and(|l| matches!(l.wires.get(name), Some(Wire::Local { .. })))
    };
    for r in 0..rows as i32 {
        for &(t, left, right) in &taps {
            for (side, (x0, x1)) in [('L', left), ('R', right)] {
                for ch in 0..CLOCK_CHANNELS {
                    let own = format!("G_HPBX{ch:02}00");
                    let tap = format!("{side}_HPBX{ch:02}00");
                    let mut meets: BTreeMap<i32, Vec<String>> = BTreeMap::new();
                    for c in x0..=x1 {
                        if local(r, c, &own) {
                            meets.entry(c).or_default().push(own.clone());
                        }
                    }
                    if local(r, t, &tap) {
                        meets.entry(t).or_default().push(tap.clone());
                    }
                    let (Some(&lo), Some(&hi)) = (meets.keys().next(), meets.keys().last()) else {
                        continue;
                    };
                    for c in (lo..=hi).filter(|_| lo < hi) {
                        clock[index(r, c)].push(ClockRun {
                            spine: false,
                            ch,
                            before: c > lo,
                            after: c < hi,
                            meets: meets.get(&c).cloned().unwrap_or_default(),
                            owner: if c == t { tap.clone() } else { own.clone() },
                        });
                    }
                }
            }
        }
    }
    for &((y0, y1), (x0, x1)) in &quadrants {
        for &(t, _, _) in taps.iter().filter(|t| (x0..=x1).contains(&t.0)) {
            for ch in 0..CLOCK_CHANNELS {
                let own = format!("G_VPTX{ch:02}00");
                let meets: Vec<i32> = (y0..=y1).filter(|&r| local(r, t, &own)).collect();
                let (Some(&lo), Some(&hi)) = (meets.first(), meets.last()) else {
                    continue;
                };
                for r in (lo..=hi).filter(|_| lo < hi) {
                    clock[index(r, t)].push(ClockRun {
                        spine: true,
                        ch,
                        before: r > lo,
                        after: r < hi,
                        meets: if meets.contains(&r) {
                            vec![own.clone()]
                        } else {
                            Vec::new()
                        },
                        owner: own.clone(),
                    });
                }
            }
        }
    }
    for runs in &mut clock {
        runs.sort();
    }
    // The arcs of each position to a neighbour's wire: those that find it,
    // and those that do not.
    let mut foreign: Vec<Vec<(usize, i32, i32, String)>> = vec![Vec::new(); positions];
    let mut linked: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); positions];
    let mut broken_links = 0;
    let mut broken: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    let mut arcs = 0;
    for r in 0..rows as i32 {
        for c in 0..cols as i32 {
            let here = at(r, c).unwrap();
            arcs += here.arcs.iter().filter(|a| !a.1.starts_with('=')).count();
            let i = index(r, c);
            // Where a name of a wire elsewhere leads: a site's port that
            // another tile names, or the neighbour the prefix says.
            let target = |name: &str| -> Option<(i32, i32, String, String)> {
                if here.wires.contains_key(name) {
                    return None;
                }
                if let Some(base) = name.strip_prefix('=') {
                    let (j, there) = bridge.get(&(i, base.to_string()))?;
                    let (jr, jc) = ((j / cols as usize) as i32, (j % cols as usize) as i32);
                    return Some((jr, jc, there.clone(), format!("{base} = R{jr}C{jc}")));
                }
                if let Some((j, host)) = redirect.get(&(i, name.to_string())) {
                    let (jr, jc) = ((j / cols as usize) as i32, (j % cols as usize) as i32);
                    return Some((jr, jc, host.clone(), format!("{} -> R{jr}C{jc}", name)));
                }
                let (dr, dc, base) = away(name)?;
                Some((
                    r + dr as i32,
                    c + dc as i32,
                    base.to_string(),
                    format!("{dr},{dc} {base}"),
                ))
            };
            for (k, (s, t)) in here.arcs.iter().enumerate() {
                let (rr, cc, base, what) = match (target(s), target(t)) {
                    (Some(e), None) | (None, Some(e)) => e,
                    _ => continue,
                };
                if at(rr, cc).is_some_and(|l| l.wires.contains_key(&base)) {
                    foreign[i].push((k, rr, cc, base));
                    linked[i].insert(k);
                } else {
                    broken_links += 1;
                    let there = if inside(rr, cc) {
                        kinds[index(rr, cc)].join("+")
                    } else {
                        "outside the die".into()
                    };
                    *broken
                        .entry((kinds[index(r, c)].join("+"), what, there))
                        .or_default() += 1;
                }
            }
        }
    }
    // For each conductor through each position: an arc behind it, and one
    // ahead, along the tiles it spans that have it.
    let reach_at = |r: i32, c: i32| -> BTreeMap<(TrackId, u8), (bool, bool)> {
        let here = at(r, c).unwrap();
        let mut out = BTreeMap::new();
        for id in &here.tracks {
            let s = step(id);
            for tap in 0..=id.span {
                let look = |forward: bool| -> bool {
                    let dir: i32 = if forward { 1 } else { -1 };
                    let steps = if forward { id.span - tap } else { tap };
                    for j in 1..=steps as i32 {
                        let (rr, cc) = (r + dir * j * s.0 as i32, c + dir * j * s.1 as i32);
                        let Some(l) = at(rr, cc).filter(|l| l.tracks.contains(id)) else {
                            return false;
                        };
                        if l.attached.contains(&(*id, (tap as i32 + dir * j) as u8)) {
                            return true;
                        }
                    }
                    false
                };
                out.insert((*id, tap), (look(false), look(true)));
            }
        }
        out
    };
    let reach: Vec<BTreeMap<(TrackId, u8), (bool, bool)>> = (0..rows as i32)
        .flat_map(|r| (0..cols as i32).map(move |c| (r, c)))
        .map(|(r, c)| reach_at(r, c))
        .collect();
    // A template per distinct layout, reach, anchors and links; links that
    // would end in a wire with nothing else at either end are left out,
    // until none is.
    let layout_ids: HashMap<&Shape, usize> =
        layouts.keys().enumerate().map(|(i, k)| (*k, i)).collect();
    type TemplateKey = (
        usize,
        Vec<(bool, bool)>,
        BTreeMap<String, usize>,
        BTreeSet<usize>,
        u32,
        BTreeSet<String>,
        Vec<ClockRun>,
    );
    let (templates, template_at) = loop {
        let mut anchored: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); positions];
        for (i, list) in foreign.iter().enumerate() {
            for (k, rr, cc, base) in list {
                if linked[i].contains(k) {
                    *anchored[index(*rr, *cc)].entry(base.clone()).or_default() += 1;
                }
            }
        }
        let mut by_key: HashMap<TemplateKey, u32> = HashMap::new();
        let mut templates = Vec::new();
        let mut template_at = vec![0u32; positions];
        for r in 0..rows as i32 {
            for c in 0..cols as i32 {
                let i = index(r, c);
                let k = (
                    layout_ids[&shapes[i]],
                    reach[i].values().copied().collect(),
                    anchored[i].clone(),
                    linked[i].clone(),
                    heights[r as usize].to_bits(),
                    tapped[i].clone(),
                    clock[i].clone(),
                );
                template_at[i] = *by_key.entry(k).or_insert_with(|| {
                    let key = Key {
                        here: at(r, c).unwrap(),
                        reach: &reach[i],
                        anchored: &anchored[i],
                        linked: &linked[i],
                        height: heights[r as usize],
                        tapped: &tapped[i],
                        clock: &clock[i],
                    };
                    templates.push(template(&key, &lanes));
                    (templates.len() - 1) as u32
                });
            }
        }
        let mut weak = Vec::new();
        for (i, list) in foreign.iter().enumerate() {
            let t: &Template = &templates[template_at[i] as usize];
            for (k, rr, cc, base) in list {
                let there: &Template = &templates[template_at[index(*rr, *cc)] as usize];
                if linked[i].contains(k)
                    && (t.weak_links.contains(k) || there.weak_anchors.contains(base))
                {
                    weak.push((i, *k));
                }
            }
        }
        if weak.is_empty() {
            break (templates, template_at);
        }
        for (i, k) in weak {
            linked[i].remove(&k);
        }
    };
    // Links: from their riser here down to the lanes of this row, along
    // them, and up to the anchor there. A link to another row runs along
    // the lanes of this row to the column of the anchor, up or down a
    // through lane of that column to the lanes of the anchor's row, and
    // along those to the anchor.
    struct Pending {
        from: usize,
        arc: u32,
        start: [f32; 2],
        end: [f32; 2],
        row: i32,
        end_row: i32,
        end_col: i32,
    }
    let mut pending: Vec<Pending> = Vec::new();
    // Links arrive at their wire's anchors one each, in the order of their
    // tiles.
    let mut arrived: HashMap<(usize, String), usize> = HashMap::new();
    for r in 0..rows as i32 {
        for c in 0..cols as i32 {
            let i = index(r, c);
            for (p, _, _, _, k) in &templates[template_at[i] as usize].links {
                let Some((_, rr, cc, base)) = foreign[i].iter().find(|f| f.0 == *k) else {
                    continue;
                };
                let (rr, cc) = (*rr, *cc);
                let anchor = inside(rr, cc).then_some(()).and_then(|_| {
                    let nth = arrived.entry((index(rr, cc), base.clone())).or_default();
                    *nth += 1;
                    templates[template_at[index(rr, cc)] as usize]
                        .anchors
                        .iter()
                        .filter(|(_, n)| n == base)
                        .nth(*nth - 1)
                        .map(|(a, _)| *a)
                });
                match anchor {
                    Some(a) => pending.push(Pending {
                        from: i,
                        arc: *k as u32,
                        start: [c as f32 * TILE_W + p[0], row_top[r as usize] + p[1]],
                        end: [cc as f32 * TILE_W + a[0], row_top[rr as usize] + a[1]],
                        row: r,
                        end_row: rr,
                        end_col: cc,
                    }),
                    None => {
                        // The wire there was found but left out: a bug.
                        broken_links += 1;
                        *broken
                            .entry((kinds[i].join("+"), base.clone(), "no anchor".into()))
                            .or_default() += 1;
                    }
                }
            }
        }
    }
    // Lanes handed out over spans, a lane free again past its last span's
    // end: (index of the item, lane) and how many lanes.
    fn pack(mut items: Vec<(usize, f32, f32)>, gap: f32) -> (Vec<(usize, usize)>, usize) {
        items.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut ends: Vec<f32> = Vec::new();
        let mut out = Vec::with_capacity(items.len());
        for (k, lo, hi) in items {
            match ends.iter().position(|&e| e < lo - gap) {
                Some(l) => {
                    ends[l] = hi;
                    out.push((k, l));
                }
                None => {
                    ends.push(hi);
                    out.push((k, ends.len() - 1));
                }
            }
        }
        (out, ends.len())
    }
    // `gap` apart from the start of a range, closer only where too many.
    let place = |range: (f32, f32), l: usize, n: usize, gap: f32| {
        range.0 + (l as f32 + 0.5) * gap.min((range.1 - range.0) / n as f32)
    };
    // The through lanes of each column, over the rows their links span.
    let mut through = vec![0.0f32; pending.len()];
    let mut by_col: BTreeMap<i32, Vec<(usize, f32, f32)>> = BTreeMap::new();
    for (k, p) in pending.iter().enumerate() {
        if p.row != p.end_row {
            let (lo, hi) = (p.row.min(p.end_row) as f32, p.row.max(p.end_row) as f32);
            by_col.entry(p.end_col).or_default().push((k, lo, hi));
        }
    }
    for (col, items) in by_col {
        let (lanes, n) = pack(items, 0.5);
        // A tap column keeps the right of its through lanes for the spines.
        let range = if tap_cols.contains(&col) {
            THROUGH_TAP
        } else {
            THROUGH
        };
        for (k, l) in lanes {
            through[k] = col as f32 * TILE_W + place(range, l, n, LANE_GAP);
        }
    }
    // The pieces along the lanes of a row: (link, second piece), row, span.
    let mut pieces: BTreeMap<i32, Vec<(usize, f32, f32)>> = BTreeMap::new();
    let mut piece_of: Vec<(usize, bool)> = Vec::new();
    for (k, p) in pending.iter().enumerate() {
        let turn = if p.row == p.end_row {
            p.end[0]
        } else {
            through[k]
        };
        let n = piece_of.len();
        piece_of.push((k, false));
        pieces
            .entry(p.row)
            .or_default()
            .push((n, p.start[0].min(turn), p.start[0].max(turn)));
        if p.row != p.end_row {
            let n = piece_of.len();
            piece_of.push((k, true));
            pieces
                .entry(p.end_row)
                .or_default()
                .push((n, turn.min(p.end[0]), turn.max(p.end[0])));
        }
    }
    let mut lane_y = vec![0.0f32; piece_of.len()];
    for (row, items) in pieces {
        let (lanes, n) = pack(items, 1e-4);
        for (piece, l) in lanes {
            lane_y[piece] = row_top[row as usize] + place(LINK_LANES, l, n, ROW_GAP);
        }
    }
    let mut lanes_of = vec![(0.0f32, 0.0f32); pending.len()];
    for (piece, &(k, second)) in piece_of.iter().enumerate() {
        if second {
            lanes_of[k].1 = lane_y[piece];
        } else {
            lanes_of[k].0 = lane_y[piece];
        }
    }
    let mut links = Vec::with_capacity(pending.len());
    let mut links_at = vec![(0, 0); positions];
    for (k, p) in pending.iter().enumerate() {
        let (y, y2) = lanes_of[k];
        let mut points = if p.row == p.end_row {
            vec![p.start, [p.start[0], y], [p.end[0], y], p.end]
        } else {
            let x = through[k];
            vec![
                p.start,
                [p.start[0], y],
                [x, y],
                [x, y2],
                [p.end[0], y2],
                p.end,
            ]
        };
        points.dedup();
        if links_at[p.from] == (0, 0) {
            links_at[p.from] = (links.len() as u32, links.len() as u32);
        }
        links.push(Link {
            points,
            from: p.from as u32,
            arc: p.arc,
        });
        links_at[p.from].1 = links.len() as u32;
    }
    // The links through each tile, for views that draw a part of the die.
    let mut through_lists: Vec<Vec<u32>> = vec![Vec::new(); positions];
    for (k, link) in links.iter().enumerate() {
        for (i, _) in clip(&row_top, cols, &link.points) {
            if through_lists[i].last() != Some(&(k as u32)) {
                through_lists[i].push(k as u32);
            }
        }
    }
    let mut through = Vec::new();
    let mut through_at = Vec::with_capacity(positions);
    for list in through_lists {
        let start = through.len() as u32;
        through.extend(list);
        through_at.push((start, through.len() as u32));
    }
    // The DSP blocks in die units.
    let mut macros = Vec::new();
    for plan in plans {
        for mut m in plan.macros {
            let dy = row_top[m.row as usize];
            let shift = |p: &mut [f32; 2]| p[1] += dy;
            for b in m.frames.iter_mut().chain(m.parts.iter_mut()) {
                shift(&mut b.min);
                shift(&mut b.max);
            }
            for w in m.wires.iter_mut().chain(m.inner.iter_mut()) {
                w.iter_mut().for_each(shift);
            }
            m.dots.iter_mut().for_each(shift);
            macros.push(m);
        }
    }
    Fabric {
        rows,
        cols,
        kinds,
        templates,
        template_at,
        links,
        links_at,
        through_at,
        through,
        row_top,
        macros,
        dsp_inside,
        dsp_lost,
        broken_links,
        broken,
        sites,
        lanes,
        arcs,
    }
}

/// The cache file's format: version 2 is postcard (version 1, bincode, is
/// read as no file).
const CACHE_MAGIC: &[u8; 8] = b"ECP5FAB2";
const CACHE_VERSION: u32 = 2;
/// Cache files kept in the folder: each build of the code writes its own,
/// and the oldest beyond these are removed after a write.
const CACHE_KEEP: usize = 3;
const HEADER_LEN: usize = 8 + 4 + 8 + 32;

fn cache_disabled() -> bool {
    std::env::var("ECP5_FABRIC_CACHE").is_ok_and(|v| v.trim() == "0")
}

fn file_sha256(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Path to the cache file in `dir` for `architecture` and the running
/// executable.
pub fn cache_path(architecture: &Path, dir: &Path) -> Option<PathBuf> {
    let manifest = std::fs::read(architecture.join("model_manifest.json")).ok()?;
    let exe = std::env::current_exe().ok()?;
    let exe_digest = file_sha256(&exe).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(CACHE_VERSION.to_le_bytes());
    hasher.update((manifest.len() as u64).to_le_bytes());
    hasher.update(&manifest);
    hasher.update(exe_digest);
    let digest: [u8; 32] = hasher.finalize().into();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Some(dir.join(format!("{hex}.bin")))
}

/// Encodes `fabric` with a header holding the format version, payload length
/// and SHA-256 checksum.
pub fn encode_cache(fabric: &Fabric) -> postcard::Result<Vec<u8>> {
    let buf = postcard::to_extend(fabric, vec![0u8; HEADER_LEN])?;
    let mut buf = buf;
    let payload_len = buf.len() - HEADER_LEN;
    let checksum: [u8; 32] = Sha256::digest(&buf[HEADER_LEN..]).into();
    buf[0..8].copy_from_slice(CACHE_MAGIC);
    buf[8..12].copy_from_slice(&CACHE_VERSION.to_le_bytes());
    buf[12..20].copy_from_slice(&(payload_len as u64).to_le_bytes());
    buf[20..52].copy_from_slice(&checksum);
    Ok(buf)
}

/// Decodes a cached fabric after checking the header's magic, version,
/// length and checksum.
pub fn decode_cache(bytes: &[u8]) -> Option<Fabric> {
    if bytes.len() < HEADER_LEN || &bytes[0..8] != CACHE_MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    if version != CACHE_VERSION {
        return None;
    }
    let payload_len = usize::try_from(u64::from_le_bytes(bytes[12..20].try_into().ok()?)).ok()?;
    if bytes.len() != HEADER_LEN.checked_add(payload_len)? {
        return None;
    }
    let payload = &bytes[HEADER_LEN..];
    let checksum: [u8; 32] = Sha256::digest(payload).into();
    if checksum != bytes[20..52] {
        return None;
    }
    postcard::from_bytes(payload).ok()
}

/// Removes the cache files in `dir` (`*.bin`) beyond the `keep` written
/// last, by their times; whatever cannot be read or removed stays.
pub fn prune_cache(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

fn write_cache_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = format!(
        ".tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let tmp = dir.join(unique);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Like [`cached`], using `dir` as the cache directory.
pub fn cached_in(model: &DeviceModel, architecture: &Path, dir: &Path) -> Fabric {
    if cache_disabled() {
        return build(model);
    }
    let Some(path) = cache_path(architecture, dir) else {
        return build(model);
    };
    if let Some(fabric) = std::fs::read(&path).ok().and_then(|b| decode_cache(&b)) {
        return fabric;
    }
    let fabric = build(model);
    if let Ok(bytes) = encode_cache(&fabric) {
        if write_cache_atomic(&path, &bytes).is_ok() {
            prune_cache(dir, CACHE_KEEP);
        }
    }
    fabric
}

/// Returns the die's fabric from the on-disk cache when valid, else builds it
/// and writes the cache file for the next run.
pub fn cached(model: &DeviceModel, architecture: &Path) -> Fabric {
    cached_in(
        model,
        architecture,
        &std::env::temp_dir().join("ecp5_emu_fabric"),
    )
}

/// What the check of the drawing found.
#[derive(Default, Debug)]
pub struct Check {
    /// Segment ends that touch nothing, by part.
    pub lonely: BTreeMap<String, usize>,
    /// Segments and pieces of links neither horizontal nor vertical.
    pub diagonal: usize,
    /// Segments through the inside of a site (other than a slice's box,
    /// which its own pins cross).
    pub through_sites: usize,
    /// Pins whose end is not on the edge of a site.
    pub pins_off_site: usize,
    /// Pins of a slice with no place on it.
    pub unplaced: usize,
    /// Link ends that miss their dot or their anchor.
    pub links: usize,
    /// Dots that do not sit on the wires they join: on fewer than two
    /// segments, or for a link's start on none.
    pub dots_off_wires: usize,
    /// Pieces of two wires that run over each other.
    pub overlaps: usize,
    /// Sites no wire comes to.
    pub bare_sites: usize,
    /// Groups of wires joined to each other and to nothing else: no site,
    /// no other tile.
    pub islands: usize,
    /// Pairs of parallel pieces side by side closer than the spacing asked
    /// for, and the closest such pair seen.
    pub tight: usize,
    pub closest: f32,
    /// Positions and ends checked.
    pub positions: usize,
    pub ends: usize,
    /// Findings per tile types.
    pub by_types: BTreeMap<String, usize>,
    pub examples: Vec<String>,
}

impl Check {
    pub fn total(&self) -> usize {
        self.lonely.values().sum::<usize>()
            + self.diagonal
            + self.through_sites
            + self.pins_off_site
            + self.unplaced
            + self.links
            + self.dots_off_wires
            + self.overlaps
            + self.bare_sites
            + self.islands
            + self.tight
    }
}

/// How close parallel pieces run side by side: horizontal pieces whose
/// spans overlap, by how far apart their rows are, and so for vertical
/// ones. Pieces closer than `SCAN` count; returns the closest distance, the
/// pairs closer than `limit` and a description of the closest pair.
fn spacing(segments: &[([f32; 2], [f32; 2], Part)], limit: f32) -> (f32, usize, String) {
    const SCAN: f32 = 0.01;
    let (mut min, mut below, mut example) = (SCAN, 0, String::new());
    for horizontal in [true, false] {
        let (a, b) = if horizontal { (1, 0) } else { (0, 1) };
        let mut list: Vec<(f32, f32, f32, Part)> = segments
            .iter()
            .filter(|(p, q, _)| (p[a] - q[a]).abs() <= EPS && (p[b] - q[b]).abs() > EPS)
            .map(|(p, q, part)| (p[a], p[b].min(q[b]), p[b].max(q[b]), *part))
            .collect();
        list.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (i, &(at, lo, hi, part)) in list.iter().enumerate() {
            for &(at2, lo2, hi2, part2) in &list[i + 1..] {
                let d = at2 - at;
                if d > SCAN {
                    break;
                }
                if d <= EPS || hi.min(hi2) - lo.max(lo2) <= EPS {
                    continue;
                }
                if d < limit * 0.98 {
                    below += 1;
                }
                if d < min {
                    min = d;
                    let along = if horizontal { "rows" } else { "columns" };
                    example = format!(
                        "closest {part:?} and {part2:?} {along} {at:.5} and {at2:.5} over {:.4}..{:.4}",
                        lo.max(lo2),
                        hi.min(hi2)
                    );
                }
            }
        }
    }
    (min, below, example)
}

/// Pairs of collinear pieces that run over each other.
fn overlap_count(segments: &[([f32; 2], [f32; 2], Part)]) -> (usize, String) {
    const Q: f32 = 2.0e4;
    let mut lines: HashMap<(bool, i64), Vec<(f32, f32)>> = HashMap::new();
    for &(a, b, _) in segments {
        let (k, lo, hi) = if (a[1] - b[1]).abs() <= EPS {
            (
                (true, (a[1] * Q).round() as i64),
                a[0].min(b[0]),
                a[0].max(b[0]),
            )
        } else {
            (
                (false, (a[0] * Q).round() as i64),
                a[1].min(b[1]),
                a[1].max(b[1]),
            )
        };
        lines.entry(k).or_default().push((lo, hi));
    }
    let mut n = 0;
    let mut example = String::new();
    for (k, list) in lines.iter_mut() {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach = f32::MIN;
        for &(lo, hi) in list.iter() {
            if lo < reach - EPS {
                n += 1;
                if example.is_empty() {
                    let line = if k.0 { "row" } else { "column" };
                    example = format!("{line} {:.5} over {lo:.4}..{reach:.4}", k.1 as f32 / Q);
                }
            }
            reach = reach.max(hi);
        }
    }
    (n, example)
}

/// Groups of a template's segments joined (at a shared end or a dot) to
/// each other only: not to a site, not to a link, not to the next tile.
fn islands(t: &Template, on_site: &[[f32; 2]]) -> (usize, Option<[f32; 2]>) {
    const Q: f32 = 2.0e4;
    let key = |p: [f32; 2]| ((p[0] * Q).round() as i64, (p[1] * Q).round() as i64);
    let n = t.segments.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, (a, b, _)) in t.segments.iter().enumerate() {
        ends.entry(key(*a)).or_default().push(i);
        ends.entry(key(*b)).or_default().push(i);
    }
    // The segments through a point: ends there, or lying across it.
    let mut rows: HashMap<i64, Vec<(f32, f32, usize)>> = HashMap::new();
    let mut cols: HashMap<i64, Vec<(f32, f32, usize)>> = HashMap::new();
    for (i, (a, b, _)) in t.segments.iter().enumerate() {
        if (a[1] - b[1]).abs() <= EPS {
            rows.entry(key(*a).1)
                .or_default()
                .push((a[0].min(b[0]), a[0].max(b[0]), i));
        } else {
            cols.entry(key(*a).0)
                .or_default()
                .push((a[1].min(b[1]), a[1].max(b[1]), i));
        }
    }
    let through = |p: [f32; 2]| -> Vec<usize> {
        let (kx, ky) = key(p);
        let mut out = Vec::new();
        for d in -1..=1 {
            for dx in -1..=1 {
                out.extend(ends.get(&(kx + dx, ky + d)).into_iter().flatten().copied());
            }
            for &(lo, hi, i) in rows.get(&(ky + d)).into_iter().flatten() {
                if p[0] >= lo - EPS && p[0] <= hi + EPS {
                    out.push(i);
                }
            }
            for &(lo, hi, i) in cols.get(&(kx + d)).into_iter().flatten() {
                if p[1] >= lo - EPS && p[1] <= hi + EPS {
                    out.push(i);
                }
            }
        }
        out
    };
    let join = |list: Vec<usize>, parent: &mut Vec<usize>| {
        for w in list.windows(2) {
            let (x, y) = (root(parent, w[0]), root(parent, w[1]));
            parent[x] = y;
        }
    };
    for list in ends.values() {
        join(list.clone(), &mut parent);
    }
    for d in &t.dots {
        join(through(*d), &mut parent);
    }
    let mut outside = vec![false; n];
    let at_edge =
        |p: &[f32; 2]| p[0] <= EPS || p[0] >= TILE_W - EPS || p[1] <= EPS || p[1] >= t.height - EPS;
    let mut marks: Vec<usize> = Vec::new();
    for (i, (a, b, _)) in t.segments.iter().enumerate() {
        if at_edge(a) || at_edge(b) {
            marks.push(i);
        }
    }
    for p in on_site
        .iter()
        .chain(t.links.iter().map(|l| &l.0))
        .chain(t.anchors.iter().map(|a| &a.0))
    {
        marks.extend(through(*p));
    }
    for i in marks {
        let r = root(&mut parent, i);
        outside[r] = true;
    }
    let mut lost = HashSet::new();
    let mut example = None;
    for i in 0..n {
        let r = root(&mut parent, i);
        if !outside[r] && lost.insert(r) {
            example.get_or_insert(t.segments[i].0);
        }
    }
    (lost.len(), example)
}

const EPS: f32 = 1e-5;

fn diagonal(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() > EPS && (a[1] - b[1]).abs() > EPS
}

/// Whether `p` lies on the boundary of a rectangle.
fn on_edge(p: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    let within = p[0] >= min[0] - EPS
        && p[0] <= max[0] + EPS
        && p[1] >= min[1] - EPS
        && p[1] <= max[1] + EPS;
    let edge = (p[0] - min[0]).abs() < EPS
        || (p[0] - max[0]).abs() < EPS
        || (p[1] - min[1]).abs() < EPS
        || (p[1] - max[1]).abs() < EPS;
    within && edge
}

/// Whether a horizontal or vertical segment runs through the open inside of
/// a rectangle.
fn crosses(a: [f32; 2], b: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    let (lo, hi) = (
        [a[0].min(b[0]), a[1].min(b[1])],
        [a[0].max(b[0]), a[1].max(b[1])],
    );
    lo[0] < max[0] - EPS && hi[0] > min[0] + EPS && lo[1] < max[1] - EPS && hi[1] > min[1] + EPS
}

impl Fabric {
    /// Checks the drawing of every position `only` accepts: every segment is
    /// horizontal or vertical, none runs through a site, every pin ends on
    /// its site's edge, every segment end touches something (another
    /// segment's end, a dot, a pin's site, a link's anchor, or across the
    /// tile's edge the next tile's segment), and every link is horizontal
    /// and vertical pieces from its dot to its anchor.
    /// The closest two parallel pieces come in each template.
    pub fn closest(&self) -> Vec<f32> {
        self.templates
            .iter()
            .map(|t| spacing(&t.segments, 0.0).0)
            .collect()
    }

    /// `min_gap` is the least distance between two parallel pieces side by
    /// side that the drawing must keep.
    pub fn check(&self, only: impl Fn(u16, u16, &[String]) -> bool, min_gap: f32) -> Check {
        const Q: f32 = 2.0e4;
        let key = |p: [f32; 2]| ((p[0] * Q).round() as i64, (p[1] * Q).round() as i64);
        let near = |set: &HashSet<(i64, i64)>, p: [f32; 2]| {
            let (x, y) = key(p);
            (-1..=1).any(|dx| (-1..=1).any(|dy| set.contains(&(x + dx, y + dy))))
        };
        let edge_points: Vec<HashSet<(i64, i64)>> = self
            .templates
            .iter()
            .map(|t| {
                let at_edge = |p: [f32; 2]| {
                    p[0] <= EPS || p[0] >= TILE_W - EPS || p[1] <= EPS || p[1] >= t.height - EPS
                };
                t.segments
                    .iter()
                    .flat_map(|&(a, b, _)| [a, b])
                    .filter(|p| at_edge(*p))
                    .map(key)
                    .collect()
            })
            .collect();
        // What each template has wrong in itself, and what touches its ends.
        struct Own {
            touch: HashSet<(i64, i64)>,
            diagonal: usize,
            through: usize,
            pins_off: usize,
            dots_off: usize,
            overlaps: usize,
            bare: usize,
            islands: usize,
            tight: usize,
            closest: f32,
            examples: Vec<String>,
        }
        let own: Vec<Own> = self
            .templates
            .iter()
            .map(|t| {
                let mut count: HashMap<(i64, i64), usize> = HashMap::new();
                for &(a, b, _) in &t.segments {
                    *count.entry(key(a)).or_default() += 1;
                    *count.entry(key(b)).or_default() += 1;
                }
                let on_site: Vec<[f32; 2]> = t
                    .site_ends
                    .iter()
                    .copied()
                    .filter(|p| t.boxes.iter().any(|b| on_edge(*p, b.min, b.max)))
                    .collect();
                let touch = t
                    .dots
                    .iter()
                    .chain(&on_site)
                    .copied()
                    .chain(t.anchors.iter().map(|a| a.0))
                    .map(key)
                    .chain(count.iter().filter(|(_, n)| **n > 1).map(|(k, _)| *k))
                    .collect();
                let mut examples = Vec::new();
                let diagonal = t
                    .segments
                    .iter()
                    .filter(|(a, b, part)| {
                        let d = diagonal(*a, *b);
                        if d && examples.len() < 3 {
                            examples.push(format!("diagonal {part:?} {a:?} {b:?}"));
                        }
                        d
                    })
                    .count();
                let through = t
                    .segments
                    .iter()
                    .filter(|(a, b, part)| {
                        let hit = t
                            .boxes
                            .iter()
                            .find(|x| x.kind != BoxKind::Slice && crosses(*a, *b, x.min, x.max));
                        if let Some(x) = hit {
                            if examples.len() < 6 {
                                examples.push(format!("{part:?} {a:?}-{b:?} through {}", x.label));
                            }
                        }
                        hit.is_some()
                    })
                    .count();
                // Segments by the line they lie on: horizontal ones by row,
                // vertical ones by column.
                let mut lines: HashMap<(bool, i64), Vec<(f32, f32)>> = HashMap::new();
                for &(a, b, _) in &t.segments {
                    let (k, lo, hi) = if (a[1] - b[1]).abs() <= EPS {
                        ((true, key([0.0, a[1]]).1), a[0].min(b[0]), a[0].max(b[0]))
                    } else {
                        ((false, key([a[0], 0.0]).0), a[1].min(b[1]), a[1].max(b[1]))
                    };
                    lines.entry(k).or_default().push((lo, hi));
                }
                let mut overlaps = 0;
                for (k, list) in &mut lines {
                    list.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut reach = f32::MIN;
                    for &(lo, hi) in list.iter() {
                        if lo < reach - EPS {
                            overlaps += 1;
                            if examples.len() < 9 {
                                examples.push(format!("overlap on {k:?} at {lo:.4}..{hi:.4}"));
                            }
                        }
                        reach = reach.max(hi);
                    }
                }
                let on_line = |p: [f32; 2]| {
                    let (kx, ky) = key(p);
                    let mut n = 0;
                    for d in -1..=1 {
                        n += lines.get(&(true, ky + d)).map_or(0, |l| {
                            l.iter()
                                .filter(|(lo, hi)| p[0] >= lo - EPS && p[0] <= hi + EPS)
                                .count()
                        });
                        n += lines.get(&(false, kx + d)).map_or(0, |l| {
                            l.iter()
                                .filter(|(lo, hi)| p[1] >= lo - EPS && p[1] <= hi + EPS)
                                .count()
                        });
                    }
                    n
                };
                let starts: HashSet<(i64, i64)> = t.links.iter().map(|l| key(l.0)).collect();
                let dots_off = t
                    .dots
                    .iter()
                    .filter(|&&d| {
                        let need = if starts.contains(&key(d)) { 1 } else { 2 };
                        let off = on_line(d) < need;
                        if off && examples.len() < 9 {
                            examples.push(format!("dot off its wires at {d:?}"));
                        }
                        off
                    })
                    .count();
                let bare: Vec<&SiteBox> = t
                    .boxes
                    .iter()
                    .filter(|b| b.kind != BoxKind::Slice)
                    .filter(|b| !on_site.iter().any(|p| on_edge(*p, b.min, b.max)))
                    .collect();
                for b in bare.iter().take(3) {
                    examples.push(format!("bare site {}", b.label));
                }
                let (islands, island) = islands(t, &on_site);
                if let Some(p) = island {
                    examples.push(format!("island of wires at {p:?}"));
                }
                let (closest, tight, pair) = spacing(&t.segments, min_gap);
                if tight > 0 {
                    examples.push(format!("tight: {pair}"));
                }
                Own {
                    touch,
                    diagonal,
                    through,
                    pins_off: t.site_ends.len() - on_site.len(),
                    dots_off,
                    overlaps,
                    bare: bare.len(),
                    islands,
                    tight,
                    closest,
                    examples,
                }
            })
            .collect();
        // The pieces of links in each tile they pass, in its coordinates.
        let mut passing: HashMap<usize, Vec<Segment>> = HashMap::new();
        for link in &self.links {
            for (i, piece) in clip(&self.row_top, self.cols, &link.points) {
                passing.entry(i).or_default().push(piece);
            }
        }
        // And the wires of the DSP blocks, and their parts.
        let mut parts_in: HashMap<usize, Vec<&SiteBox>> = HashMap::new();
        for m in &self.macros {
            for w in m.wires.iter().chain(m.inner.iter()) {
                for (i, piece) in clip(&self.row_top, self.cols, w) {
                    passing.entry(i).or_default().push(piece);
                }
            }
            for b in &m.parts {
                let c = (b.min[0] / TILE_W).floor() as u16;
                parts_in.entry(self.index(m.row, c)).or_default().push(b);
            }
        }
        let mut check = Check {
            closest: f32::MAX,
            ..Check::default()
        };
        self.check_macros(&only, &mut check);
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        for r in 0..self.rows {
            for c in 0..self.cols {
                let i = self.index(r, c);
                if !only(r, c, &self.kinds[i]) {
                    continue;
                }
                check.positions += 1;
                let t = self.template_at[i] as usize;
                let template = &self.templates[t];
                let found = &own[t];
                let types = self.kinds[i].join("+");
                // With the links that pass: how close they come to the wires
                // here and each other, and whether they run over them.
                let (mut closest, mut tight, mut overlaps) =
                    (found.closest, found.tight, found.overlaps);
                if let Some(pieces) = passing.get(&i) {
                    let mut all = template.segments.clone();
                    all.extend(pieces.iter().copied());
                    let (near, t, pair) = spacing(&all, min_gap);
                    let seen = check
                        .examples
                        .iter()
                        .filter(|e| e.contains(&format!(" {types}: ")))
                        .count();
                    if t > found.tight && seen < 2 && check.examples.len() < 300 {
                        check
                            .examples
                            .push(format!("R{r}C{c} {types}: tight with links: {pair}"));
                    }
                    closest = near;
                    tight = t;
                    let (n, what) = overlap_count(&all);
                    if n > found.overlaps && check.examples.len() < 300 {
                        check
                            .examples
                            .push(format!("R{r}C{c} {types}: overlap with links on {what}"));
                    }
                    overlaps = overlaps.max(n);
                }
                let mut findings = found.diagonal
                    + found.through
                    + found.pins_off
                    + found.dots_off
                    + overlaps
                    + found.bare
                    + found.islands
                    + tight
                    + template.unplaced.len();
                check.bare_sites += found.bare;
                check.islands += found.islands;
                check.tight += tight;
                check.closest = check.closest.min(closest);
                check.dots_off_wires += found.dots_off;
                check.overlaps += overlaps;
                check.diagonal += found.diagonal;
                check.through_sites += found.through;
                check.pins_off_site += found.pins_off;
                check.unplaced += template.unplaced.len();
                let example = |check: &mut Check, s: String| {
                    let kind = s.split(' ').next().unwrap_or("");
                    let seen = check
                        .examples
                        .iter()
                        .filter(|e| e.contains(&format!(" {types}: {kind} ")))
                        .count();
                    if seen < 2 && check.examples.len() < 300 {
                        check.examples.push(format!("R{r}C{c} {types}: {s}"));
                    }
                };
                if findings > 0 {
                    for e in &found.examples {
                        example(&mut check, e.clone());
                    }
                    if !template.unplaced.is_empty() {
                        example(&mut check, format!("unplaced pins {:?}", template.unplaced));
                    }
                }
                let base = [c as f32 * TILE_W, self.row_top[r as usize]];
                for &(a, b, part) in &template.segments {
                    for p in [a, b] {
                        check.ends += 1;
                        if near(&found.touch, p) {
                            continue;
                        }
                        let dr = if p[1] <= EPS {
                            -1
                        } else if p[1] >= template.height - EPS {
                            1
                        } else {
                            0
                        };
                        let dc = if p[0] <= EPS {
                            -1
                        } else if p[0] >= TILE_W - EPS {
                            1
                        } else {
                            0
                        };
                        let (rr, cc) = (r as i32 + dr, c as i32 + dc);
                        let across = (dr, dc) != (0, 0)
                            && rr >= 0
                            && cc >= 0
                            && rr < self.rows as i32
                            && cc < self.cols as i32;
                        if across {
                            // The same point in the next tile's coordinates.
                            let y = match dr {
                                1 => p[1] - template.height,
                                -1 => p[1] + self.row_height(rr as u16),
                                _ => p[1],
                            };
                            let q = [p[0] - dc as f32 * TILE_W, y];
                            let n = self.template_at[self.index(rr as u16, cc as u16)] as usize;
                            if near(&edge_points[n], q) {
                                continue;
                            }
                        }
                        *check.lonely.entry(format!("{part:?}")).or_default() += 1;
                        findings += 1;
                        example(
                            &mut check,
                            format!(
                                "lonely {part:?} end at ({:.5}, {:.5}); nearest dot {:?}",
                                p[0],
                                p[1],
                                template
                                    .dots
                                    .iter()
                                    .min_by(|a, b| length(**a, p).total_cmp(&length(**b, p)))
                            ),
                        );
                    }
                }
                for link in self.links_from(r, c) {
                    check.ends += 2;
                    let bent = link
                        .points
                        .windows(2)
                        .filter(|w| diagonal(w[0], w[1]))
                        .count();
                    check.diagonal += bent;
                    let (from, to) = (link.points[0], *link.points.last().unwrap());
                    let f = [from[0] - base[0], from[1] - base[1]];
                    let (tr, tc) = (self.row_at(to[1]) as f32, (to[0] / TILE_W).floor());
                    let e = [to[0] - tc * TILE_W, to[1] - self.row_top[tr as usize]];
                    let there = self.template(tr as u16, tc as u16);
                    let bad = usize::from(!template.dots.iter().any(|d| close(*d, f)))
                        + usize::from(!there.anchors.iter().any(|a| close(a.0, e)));
                    check.links += bad;
                    findings += bad + bent;
                    if bad + bent > 0 {
                        example(
                            &mut check,
                            format!("link {:?}: {bad} loose ends, {bent} diagonal", link.points),
                        );
                    }
                }
                // No wire of the tile runs through a part of a DSP block.
                for b in parts_in.get(&i).into_iter().flatten() {
                    let (min, max) = (
                        [b.min[0] - base[0], b.min[1] - base[1]],
                        [b.max[0] - base[0], b.max[1] - base[1]],
                    );
                    for (a, z, part) in &template.segments {
                        if crosses(*a, *z, min, max) {
                            check.through_sites += 1;
                            findings += 1;
                            example(
                                &mut check,
                                format!("{part:?} {a:?}-{z:?} through {}", b.label),
                            );
                        }
                    }
                }
                if findings > 0 {
                    *check.by_types.entry(types.clone()).or_default() += findings;
                }
            }
        }
        check
    }

    /// The DSP blocks of the rows `only` accepts: every piece square, no
    /// wire or bus through a part, every end on a dot, a part's edge or the
    /// wire it runs on from, and no part without a wire or bus.
    fn check_macros(&self, only: &impl Fn(u16, u16, &[String]) -> bool, check: &mut Check) {
        const Q: f32 = 1.0e4;
        let key = |p: [f32; 2]| ((p[0] * Q).round() as i64, (p[1] * Q).round() as i64);
        for m in &self.macros {
            if !(0..self.cols).any(|c| only(m.row, c, &self.kinds[self.index(m.row, c)])) {
                continue;
            }
            let lines: Vec<&Vec<[f32; 2]>> = m.wires.iter().chain(m.inner.iter()).collect();
            // The parts by where they start: they do not overlap.
            let mut parts: Vec<&SiteBox> = m.parts.iter().collect();
            parts.sort_by(|a, b| a.min[0].total_cmp(&b.min[0]));
            let parts = &parts;
            let over = |lo: f32, hi: f32| {
                let i = parts.partition_point(|b| b.max[0] < lo - EPS);
                (i..parts.len()).take_while(move |&k| parts[k].min[0] <= hi + EPS)
            };
            let dots: HashSet<(i64, i64)> = m.dots.iter().map(|d| key(*d)).collect();
            let near_dot = |p: [f32; 2]| {
                let (x, y) = key(p);
                (-1..=1).any(|dx| (-1..=1).any(|dy| dots.contains(&(x + dx, y + dy))))
            };
            let mut rows: HashMap<i64, Vec<(f32, f32)>> = HashMap::new();
            let mut cols: HashMap<i64, Vec<(f32, f32)>> = HashMap::new();
            for l in &lines {
                for w in l.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if (a[1] - b[1]).abs() <= EPS {
                        rows.entry(key(a).1)
                            .or_default()
                            .push((a[0].min(b[0]), a[0].max(b[0])));
                    } else {
                        cols.entry(key(a).0)
                            .or_default()
                            .push((a[1].min(b[1]), a[1].max(b[1])));
                    }
                }
            }
            let on_line = |p: [f32; 2]| {
                let (x, y) = key(p);
                (-1..=1).any(|d| {
                    rows.get(&(y + d)).is_some_and(|l| {
                        l.iter()
                            .any(|(lo, hi)| p[0] >= lo - 1e-4 && p[0] <= hi + 1e-4)
                    }) || cols.get(&(x + d)).is_some_and(|l| {
                        l.iter()
                            .any(|(lo, hi)| p[1] >= lo - 1e-4 && p[1] <= hi + 1e-4)
                    })
                })
            };
            let push = |check: &mut Check, s: String| {
                if check.examples.len() < 300 {
                    check.examples.push(format!("DSP R{}: {s}", m.row));
                }
            };
            let mut used = vec![false; parts.len()];
            for l in &lines {
                for w in l.windows(2) {
                    if diagonal(w[0], w[1]) {
                        check.diagonal += 1;
                    }
                    for k in over(w[0][0].min(w[1][0]), w[0][0].max(w[1][0])) {
                        let b = parts[k];
                        if crosses(w[0], w[1], b.min, b.max) {
                            check.through_sites += 1;
                            push(
                                check,
                                format!("wire {:?}-{:?} through {}", w[0], w[1], b.label),
                            );
                        }
                    }
                }
                for p in [l[0], *l.last().unwrap()] {
                    check.ends += 1;
                    let mut on_part = false;
                    for k in over(p[0], p[0]) {
                        if on_edge(p, parts[k].min, parts[k].max) {
                            used[k] = true;
                            on_part = true;
                        }
                    }
                    // A trunk starts on the corner of its wire, where the
                    // tile draws a dot.
                    let corner = || {
                        let r = self.row_at(p[1]);
                        let c = (p[0] / TILE_W).floor();
                        let q = [p[0] - c * TILE_W, p[1] - self.row_top[r as usize]];
                        c >= 0.0
                            && (c as u16) < self.cols
                            && self
                                .template(r, c as u16)
                                .dots
                                .iter()
                                .any(|d| length(*d, q) < 1e-4)
                    };
                    if !(on_part || near_dot(p) || corner()) {
                        *check.lonely.entry("DSP".into()).or_default() += 1;
                        push(check, format!("lonely end at {p:?}"));
                    }
                }
            }
            for d in &m.dots {
                if !on_line(*d) {
                    check.dots_off_wires += 1;
                }
            }
            for (k, b) in parts.iter().enumerate() {
                if !used[k] {
                    check.bare_sites += 1;
                    push(check, format!("bare part {}", b.label));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_taps() {
        assert_eq!(split_prefix("N1_V02S0201"), (-1, 0, "V02S0201"));
        assert_eq!(split_prefix("S1E1_JF0"), (1, 1, "JF0"));
        assert_eq!(split_prefix("A0_SLICE"), (0, 0, "A0_SLICE"));
        assert_eq!(split_prefix("G_HPBX0000"), (0, 0, "G_HPBX0000"));
        // One tile north is tap 1 of a southward wire: here it is tap 2.
        let (id, tap) = parse_track("V02S0201").unwrap();
        assert_eq!(tap_here(&id, tap, -1, 0), Some(2));
        // Three tiles east is tap 3 of an eastward span-6 wire: tap 0 here.
        let (id, tap) = parse_track("H06E0103").unwrap();
        assert_eq!(tap_here(&id, tap, 0, 3), Some(0));
        assert_eq!(tap_here(&id, tap, 0, 4), None);
    }

    #[test]
    fn a_cut_keeps_the_part_between_two_distances() {
        let l = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert_eq!(cut(&l, 0.5, 1.5), vec![[0.5, 0.0], [1.0, 0.0], [1.0, 0.5]]);
        assert_eq!(along(&l, [1.0, 0.25]), 1.25);
    }

    #[test]
    fn a_conductor_steps_to_its_next_lane_square() {
        let (id, _) = parse_track("H02W0101").unwrap();
        let p = track_points(&id, 1, 0.1, Some(0.11), TILE_H);
        let jog = JOG + JOG_STEP;
        let want = [[TILE_W, 0.1], [jog, 0.1], [jog, 0.11], [0.0, 0.11]];
        for (a, b) in p.iter().zip(want) {
            assert!(
                (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
                "{p:?}"
            );
        }
        assert!(p.windows(2).all(|w| !diagonal(w[0], w[1])));
    }

    #[test]
    fn slice_pins_end_on_their_sites_and_cross_none() {
        let boxes = logic_boxes();
        let parts: Vec<&SiteBox> = boxes.iter().filter(|b| b.kind != BoxKind::Slice).collect();
        let mut strip = BTreeMap::new();
        let mut names = vec![
            "FCI".to_string(),
            "FCO".into(),
            "FCOA".into(),
            "FCIB".into(),
        ];
        for n in 0..8 {
            for l in ["A", "B", "C", "D", "F", "DI", "Q"] {
                names.push(format!("{l}{n}"));
            }
        }
        for s in 0..4 {
            strip.insert(
                format!("CLK{s}"),
                lane((ctrl_rect(s).0[1], ctrl_rect(s).1[1]), 0, 3),
            );
            strip.insert(
                format!("F5{}", (b'A' + s as u8) as char),
                lane((mux_rect(s).0[1], mux_rect(s).1[1]), 0, 6),
            );
            for k in 0..2 {
                strip.insert(
                    format!("M{}", 2 * s + k),
                    lane((mux_rect(s).0[1], mux_rect(s).1[1]), 4 + k, 6),
                );
            }
        }
        names.extend(strip.keys().cloned());
        for name in &names {
            let (y, path) = logic_pin(name, &strip).unwrap_or_else(|| panic!("{name}"));
            assert!(
                boxes.iter().any(|b| on_edge(path[0], b.min, b.max)),
                "{name} starts off its site"
            );
            for w in path.windows(2) {
                assert!(!diagonal(w[0], w[1]), "{name}");
                for b in &parts {
                    assert!(
                        !crosses(w[0], w[1], b.min, b.max),
                        "{name} through {}",
                        b.label
                    );
                }
            }
            let last = *path.last().unwrap();
            assert!(
                last[0] == BLOCK || name.len() == 4,
                "{name} ends at {last:?}"
            );
            assert!(last[0] != BLOCK || last[1] == y);
        }
        assert_eq!(strip_pin("WAD2B"), Some((1, false, 52)));
        assert_eq!(strip_pin("FXAC"), Some((2, true, 2)));
        assert_eq!(strip_pin("FXC"), Some((2, true, 1)));
        assert_eq!(strip_pin("F5D"), Some((3, true, 0)));
        assert_eq!(strip_pin("M3"), Some((1, true, 5)));
        assert_eq!(strip_pin("WDO1C"), Some((2, false, 81)));
        assert_eq!(strip_pin("WCK1"), Some((1, false, 31)));
        assert_eq!(strip_pin("F5"), None);
        assert_eq!(strip_pin("A5"), None);
    }

    /// After a write only the newest cache files stay: the others go, and
    /// files of other kinds are left alone.
    #[test]
    fn the_cache_folder_keeps_the_newest_files() {
        let dir = std::env::temp_dir().join(format!("ecp5_emu_prune_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let base = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for k in 0..5u64 {
            let path = dir.join(format!("{k}.bin"));
            let file = std::fs::File::create(&path).unwrap();
            file.set_modified(base + std::time::Duration::from_secs(60 * k))
                .unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "kept").unwrap();
        prune_cache(&dir, 3);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(left, ["2.bin", "3.bin", "4.bin", "notes.txt"]);
    }

    #[test]
    fn cache_header_rejects_corruption_and_truncation() {
        let mut h = BTreeMap::new();
        h.insert(
            (
                TrackId {
                    horizontal: true,
                    span: 2,
                    backwards: false,
                    track: 1,
                    zero: false,
                },
                1,
            ),
            0,
        );
        let mut broken = BTreeMap::new();
        broken.insert(("PLC2".into(), "0,1 H02E0001".into(), "MIB".into()), 3);
        let mut weak_links = BTreeSet::new();
        weak_links.insert(4);
        let mut weak_anchors = BTreeSet::new();
        weak_anchors.insert("W0".into());
        let fabric = Fabric {
            rows: 1,
            cols: 1,
            kinds: vec![vec!["PLC2".into()]],
            templates: vec![Template {
                weak_links,
                weak_anchors,
                height: 1.25,
                arcs: std::sync::Arc::new(vec![("A".into(), "B".into())]),
                fixed: std::sync::Arc::new(vec![true]),
                ..Template::default()
            }],
            template_at: vec![0],
            links: vec![Link {
                points: vec![[0.0, 0.0], [1.0, 0.0]],
                from: 0,
                arc: 0,
            }],
            links_at: vec![(0, 1)],
            through_at: vec![(0, 1)],
            through: vec![0],
            row_top: vec![0.0, 1.25],
            macros: Vec::new(),
            dsp_inside: 1,
            dsp_lost: 2,
            broken_links: 3,
            broken,
            sites: 4,
            lanes: Lanes {
                h,
                v: BTreeMap::new(),
            },
            arcs: 5,
        };
        let encoded = encode_cache(&fabric).unwrap();
        assert!(decode_cache(&encoded).as_ref() == Some(&fabric));

        // Truncated header and truncated payload.
        assert!(decode_cache(&encoded[..HEADER_LEN - 1]).is_none());
        assert!(decode_cache(&encoded[..encoded.len() - 1]).is_none());
        // Trailing garbage.
        let mut extra = encoded.clone();
        extra.push(0);
        assert!(decode_cache(&extra).is_none());
        // Bad magic, version, length, checksum, and payload byte.
        for idx in [0, 8, 12, 20, encoded.len() - 1] {
            let mut bad = encoded.clone();
            bad[idx] ^= 0xff;
            assert!(decode_cache(&bad).is_none(), "accepted corruption at {idx}");
        }
    }
}
