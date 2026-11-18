//! Die geometry for views, in tile units: tile (row r, column c) covers
//! x ∈ [c, c + 1], y ∈ [r, r + 1]. Every tile has the same plan:
//!
//! ```text
//!  y=0 ┌─────────────────────────────────────────┐
//!      │ horizontal channel: H wires, one lane   │
//!      │ per conductor that passes the row       │
//!  0.28├─────────────────────────────────────────┤
//!      │   (jumpers between vertical wires)      │
//!  0.33├────────┬──────────┬──┬──────────────────┤
//!      │ pin    │ vertical │j │ block: the sites │
//!      │ lanes  │ channel  │u │ (four slices of  │
//!      │        │ V wires  │m │ two LUTs and two │
//!      │        │          │p │ flip-flops, or   │
//!      │        │          │  │ EBR, DSP, PIO …) │
//!  y=1 └────────┴──────────┴──┴──────────────────┘
//!     x=0     0.17       0.35 0.40              1
//! ```
//!
//! The wires are the design's own: the inter-tile wires and local wires that
//! the plan's routing transfers (`Report::routes`, decoded from the
//! bitstream) use. An inter-tile wire is one straight conductor over the
//! tiles it spans, in its own lane of the row's or the column's channel.
//! Trellis names them `<H|V><span><direction><track><tap>`: `V06N0303` is
//! tap 3 of a wire that runs six tiles north, so the same conductor is
//! `V06N0300` three tiles south and `V06N0306` three tiles north. A local
//! wire of a tile is an L: from its pin at the block's edge left to its own
//! lane, then up through the horizontal channel. Pins are laid out as a
//! staircase (a pin higher up has a lane further left), so any two local
//! wires of a tile cross exactly once, and every local wire crosses every
//! horizontal and vertical wire that passes the tile.
//!
//! A routing transfer (a pip) is therefore a dot where its two wires cross.
//! Two horizontal or two vertical wires have no crossing; a short jumper in
//! the tile joins them. Each net is a tree of segments from its driver, and
//! every segment and dot knows its distance from the driver along the net,
//! so a view can show a new value running from the driver to the sinks.
use crate::{
    compile::Report,
    model::{
        topology::{Endpoint, Location, Wire},
        DeviceModel,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap},
    path::{Path, PathBuf},
};

/// Bottom of the horizontal channel.
pub const H_CHANNEL: f32 = 0.28;
const V_JUMPERS: (f32, f32) = (0.29, 0.32);
/// Top of the band of pins and sites.
pub const BAND_TOP: f32 = 0.33;
const PIN_LANES: (f32, f32) = (0.01, 0.16);
const V_LANES: (f32, f32) = (0.17, 0.35);
const H_JUMPERS: (f32, f32) = (0.355, 0.395);
/// Left edge of the block of sites; pins meet their local wires here.
pub const BLOCK: f32 = 0.40;
/// A LUT and its flip-flop share a row; two rows and a strip of controls
/// make a slice.
const ROW: f32 = 0.066;
const STRIP: f32 = 0.033;
const SLICE: f32 = 2.0 * ROW + STRIP;
const LUT_X: (f32, f32) = (BLOCK + 0.03, BLOCK + 0.30);
const FF_X: (f32, f32) = (BLOCK + 0.38, 0.97);

/// Index into `Geometry::values` meaning "no value".
pub const NO_VALUE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SegmentKind {
    /// Part of an inter-tile wire.
    Track,
    /// Part of a local wire of a tile.
    Local,
    /// Joins two wires that do not cross.
    Jumper,
}

/// A straight piece of a net. `d0` and `d1` are the distances of `a` and
/// `b` from the net's driver, along the net.
#[derive(PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub kind: SegmentKind,
    pub value: u32,
    pub net: u32,
    pub d0: f32,
    pub d1: f32,
}

/// A pip: the switch where a routing transfer joins its two wires.
#[derive(PartialEq, Serialize, Deserialize)]
pub struct Dot {
    pub at: [f32; 2],
    pub value: u32,
    pub net: u32,
    pub d: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SiteKind {
    Ebr,
    Dsp,
    Pio,
    IoLogic,
    Clock,
    Other,
}

/// A site other than a slice.
#[derive(PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub kind: SiteKind,
    pub name: String,
    pub used: bool,
}

/// A logic tile with a slice in use. Bit s of `slices` is slice s (A..D);
/// bit n of `luts`/`flip_flops` is LUT or flip-flop n (slice n / 2); `f`
/// and `q` are the values of their outputs. A slice can be in use for its
/// carry alone.
#[derive(PartialEq, Serialize, Deserialize)]
pub struct LogicTile {
    pub row: u16,
    pub col: u16,
    pub slices: u8,
    pub luts: u8,
    pub flip_flops: u8,
    pub f: [u32; 8],
    pub q: [u32; 8],
}

#[derive(PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub rows: u16,
    pub cols: u16,
    pub segments: Vec<Segment>,
    pub dots: Vec<Dot>,
    pub sites: Vec<Site>,
    pub logic: Vec<LogicTile>,
    /// The wire behind each value index.
    #[serde(with = "crate::design_cache::cached_wires")]
    pub values: Vec<Wire>,
    /// Length of the longest net from its driver, in tile units.
    pub longest: f32,
}

const CACHE_MAGIC: &[u8; 8] = b"ECP5GEO1";
const CACHE_VERSION: u32 = 2;
// A life_full geometry is about 147 MiB, leaving room for smaller designs.
const CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const HEADER_LEN: usize = 8 + 4 + 8 + 32;

/// The compiled design's inputs and the geometry format.
#[derive(Debug, PartialEq, Eq)]
pub struct GeometryKey([u8; 32]);

impl GeometryKey {
    /// Uses the identity returned by [`crate::compile::cache_key`].
    pub fn new(compile_key: &[u8; 32]) -> Self {
        Self(crate::design_cache::key(compile_key, CACHE_VERSION))
    }

    pub fn cache_path(&self, dir: &Path) -> PathBuf {
        crate::design_cache::path(&self.0, dir)
    }
}

/// Encodes every geometry field with a version, payload length and checksum.
pub fn encode_cache(geometry: &Geometry) -> postcard::Result<Vec<u8>> {
    let mut buf = postcard::to_extend(geometry, vec![0u8; HEADER_LEN])?;
    let payload_len = buf.len() - HEADER_LEN;
    let checksum: [u8; 32] = Sha256::digest(&buf[HEADER_LEN..]).into();
    buf[0..8].copy_from_slice(CACHE_MAGIC);
    buf[8..12].copy_from_slice(&CACHE_VERSION.to_le_bytes());
    buf[12..20].copy_from_slice(&(payload_len as u64).to_le_bytes());
    buf[20..52].copy_from_slice(&checksum);
    Ok(buf)
}

/// Reads geometry only after its complete header and payload check out.
pub fn decode_cache(bytes: &[u8]) -> Option<Geometry> {
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
    let (geometry, rest) = postcard::take_from_bytes(payload).ok()?;
    rest.is_empty().then_some(geometry)
}

/// Keeps the newest files within 256 MiB, or `ECP5_GEOMETRY_CACHE_MB` MiB.
/// The newest always stays, even when it alone exceeds the cap.
pub fn prune_cache(dir: &Path) {
    let limit = crate::design_cache::limit("ECP5_GEOMETRY_CACHE_MB", CACHE_MAX_BYTES);
    crate::design_cache::prune(dir, limit);
}

/// Like [`cached`], using `dir` as the cache directory.
pub fn cached_in(
    model: &DeviceModel,
    report: &Report,
    key: Option<&GeometryKey>,
    dir: &Path,
) -> Geometry {
    if std::env::var("ECP5_GEOMETRY_CACHE").is_ok_and(|v| v.trim() == "0") {
        return build_uncached(model, report);
    }
    let Some(key) = key else {
        return build_uncached(model, report);
    };
    let path = key.cache_path(dir);
    if let Some(geometry) = std::fs::read(&path).ok().and_then(|b| decode_cache(&b)) {
        prune_cache(dir);
        return geometry;
    }
    let geometry = build_uncached(model, report);
    if let Ok(bytes) = encode_cache(&geometry) {
        if crate::design_cache::write_cache_atomic(&path, &bytes).is_ok() {
            prune_cache(dir);
        }
    }
    geometry
}

/// Returns checked geometry from disk, else builds and writes it for the
/// next run. `ECP5_GEOMETRY_CACHE=0` turns off reads and writes.
pub fn cached(model: &DeviceModel, report: &Report, key: Option<&GeometryKey>) -> Geometry {
    cached_in(
        model,
        report,
        key,
        &std::env::temp_dir().join("ecp5_emu_geometry"),
    )
}

fn row_top(n: usize) -> f32 {
    BAND_TOP + (n / 2) as f32 * SLICE + (n % 2) as f32 * ROW
}

/// LUT n of a logic tile, as (min, max) in the tile.
pub fn lut_rect(n: usize) -> ([f32; 2], [f32; 2]) {
    let top = row_top(n);
    ([LUT_X.0, top + 0.06 * ROW], [LUT_X.1, top + 0.72 * ROW])
}

/// Flip-flop n of a logic tile.
pub fn ff_rect(n: usize) -> ([f32; 2], [f32; 2]) {
    let (min, max) = lut_rect(n);
    ([FF_X.0, min[1]], [FF_X.1, max[1]])
}

/// Slice s (A..D) of a logic tile.
pub fn slice_rect(s: usize) -> ([f32; 2], [f32; 2]) {
    let top = BAND_TOP + s as f32 * SLICE;
    ([BLOCK + 0.01, top + 0.003], [0.995, top + SLICE - 0.003])
}

/// An inter-tile routing wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    pub horizontal: bool,
    /// Tiles it spans, at least 1.
    pub span: i16,
    /// Direction of travel as (rows, columns) per tile.
    pub step: (i16, i16),
    /// Kind, direction and track number, unique per orientation.
    pub id: u16,
    /// Tiles from the wire's start to this name's tile.
    pub tap: i16,
}

/// `H06E0103` and the like; none for any other name.
pub fn track(name: &str) -> Option<Track> {
    let b = name.as_bytes();
    if b.len() != 8 || !(b[0] == b'H' || b[0] == b'V') {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i16> { name.get(r)?.parse().ok() };
    let (span, track, tap) = (digits(1..3)?, digits(4..6)?, digits(6..8)?);
    let horizontal = b[0] == b'H';
    let (step, direction) = match (horizontal, b[3]) {
        (true, b'E' | b'R') => ((0, 1), 0),
        (true, b'W' | b'L') => ((0, -1), 1),
        (false, b'N' | b'T') => ((-1, 0), 0),
        (false, b'S' | b'B') => ((1, 0), 1),
        _ => return None,
    };
    Some(Track {
        horizontal,
        span: span.max(1),
        step,
        id: (span as u16) << 8 | direction << 7 | track as u16,
        tap,
    })
}

/// The conductor of a track named at `location`: its row (horizontal) or
/// column, and the first and last tile along it.
fn extent(t: &Track, location: Location) -> (i16, i16, i16) {
    let start = (
        location.row - t.tap * t.step.0,
        location.col - t.tap * t.step.1,
    );
    let end = (start.0 + t.span * t.step.0, start.1 + t.span * t.step.1);
    if t.horizontal {
        (location.row, start.1.min(end.1), start.1.max(end.1))
    } else {
        (location.col, start.0.min(end.0), start.0.max(end.0))
    }
}

/// Where a pin of a logic tile's slices is: its height in the tile, the x
/// of its end on the slice side, and whether it is that side (`_SLICE`).
fn slice_pin(name: &str) -> Option<(f32, f32, bool)> {
    let (base, slice_side) = match name.strip_suffix("_SLICE") {
        Some(b) => (b, true),
        None => (name, false),
    };
    let split = base.find(|c: char| c.is_ascii_digit())?;
    let (letters, number) = base.split_at(split);
    let n: usize = number.parse().ok()?;
    let lut = |at: f32| -> Option<(f32, f32, bool)> {
        (n < 8).then(|| (row_top(n) + at * ROW, LUT_X.0, slice_side))
    };
    let ff = |at: f32, x: f32| -> Option<(f32, f32, bool)> {
        (n < 8).then(|| (row_top(n) + at * ROW, x, slice_side))
    };
    let control = |slot: f32| -> Option<(f32, f32, bool)> {
        let s = n % 4;
        let y = BAND_TOP + s as f32 * SLICE + 2.0 * ROW + STRIP * slot;
        Some((y, FF_X.0 + 0.02 + 0.06 * slot, slice_side))
    };
    match letters {
        "A" => lut(0.14),
        "B" => lut(0.27),
        "C" => lut(0.40),
        "D" => lut(0.53),
        "F" => lut(0.66),
        "M" => ff(0.80, FF_X.0 + 0.015),
        "DI" => ff(0.87, FF_X.0 + 0.015),
        "Q" => ff(0.94, FF_X.0 + 0.03),
        "CLK" => control(0.15),
        "MUXCLK" => control(0.32),
        "LSR" => control(0.49),
        "MUXLSR" => control(0.66),
        "CE" => control(0.83),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// A conductor: horizontal at y, or vertical at x, within its tiles.
    Track { horizontal: bool, at: f32 },
    /// A local wire of the tile at (row, col): from the block's edge at
    /// height y left to its lane at x, then up.
    Local { row: i16, col: i16, x: f32, y: f32 },
    /// The slice side of a pin: from the site to the block's edge at y.
    Pin { row: i16, col: i16, y: f32 },
}

struct WireShape {
    shape: Shape,
    /// The driver's end first: the site for a pin.
    points: Vec<[f32; 2]>,
    value: u32,
}

enum Join {
    Dot([f32; 2]),
    /// From a point on the source to a point on the sink.
    Jumper([f32; 2], [f32; 2]),
}

fn site_kind(family: &str) -> SiteKind {
    match family {
        "EBR" => SiteKind::Ebr,
        "MULT18" | "ALU54" | "DSP_SLICE" | "DSP_BLOCK" => SiteKind::Dsp,
        "PIO" => SiteKind::Pio,
        "IOLOGIC" | "SIOLOGIC" => SiteKind::IoLogic,
        "PLL" | "DCC_MID" | "DCC_CENTRE" | "DCS" | "CLKDIVF" | "ECLKSYNCB" | "ECLKBRIDGECS"
        | "BRGECLKSYNC" | "TRELLIS_ECLKBUF" | "OSCG" | "DDRDLL" | "DLLDELD" => SiteKind::Clock,
        _ => SiteKind::Other,
    }
}

/// Evenly spaced positions in `range`.
fn lane(range: (f32, f32), k: usize, n: usize) -> f32 {
    range.0 + (range.1 - range.0) * (k as f32 + 0.5) / n.max(1) as f32
}

/// Lanes for intervals [start, end] of tiles: the first free lane for each,
/// in order of start. Returns the lane of each interval and the count.
fn colour(intervals: &[(i16, i16)]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..intervals.len()).collect();
    order.sort_by_key(|&i| intervals[i]);
    let mut ends: Vec<i16> = Vec::new();
    let mut lanes = vec![0; intervals.len()];
    for i in order {
        let (start, end) = intervals[i];
        match ends.iter().position(|&e| e < start) {
            Some(k) => {
                ends[k] = end;
                lanes[i] = k;
            }
            None => {
                ends.push(end);
                lanes[i] = ends.len() - 1;
            }
        }
    }
    (lanes, ends.len())
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
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = length(a, b);
        let t = if len > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (len * len)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + d[0] * t, a[1] + d[1] * t];
        let e = length(p, q);
        if e < best.0 {
            best = (e, walked + t * len);
        }
        walked += len;
    }
    best.1
}

/// The point at distance `s` along a polyline.
fn at_distance(points: &[[f32; 2]], s: f32) -> [f32; 2] {
    let mut left = s;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = length(a, b);
        if left <= len || len == 0.0 {
            let t = if len > 0.0 { left / len } else { 0.0 };
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        left -= len;
    }
    *points.last().unwrap()
}

/// Where a transfer at `at` joins wires `s` and `t`, given their shapes.
/// `jumper` gives the next jumper lane of the tile.
fn join(at: Location, s: &WireShape, t: &WireShape, jumper: &mut dyn FnMut(bool) -> f32) -> Join {
    let (r, c) = (at.row as f32, at.col as f32);
    let here = |row: i16, col: i16| (row, col) == (at.row, at.col);
    match (s.shape, t.shape) {
        (
            Shape::Track {
                horizontal: true,
                at: y,
            },
            Shape::Track {
                horizontal: false,
                at: x,
            },
        )
        | (
            Shape::Track {
                horizontal: false,
                at: x,
            },
            Shape::Track {
                horizontal: true,
                at: y,
            },
        ) => Join::Dot([c + x, r + y]),
        (Shape::Track { horizontal, at: a }, Shape::Track { at: b, .. }) => {
            let lane = jumper(horizontal);
            if horizontal {
                Join::Jumper([c + lane, r + a], [c + lane, r + b])
            } else {
                Join::Jumper([c + a, r + lane], [c + b, r + lane])
            }
        }
        (
            Shape::Track {
                horizontal: true,
                at: y,
            },
            Shape::Local { row, col, x, .. },
        )
        | (
            Shape::Local { row, col, x, .. },
            Shape::Track {
                horizontal: true,
                at: y,
            },
        ) if here(row, col) => Join::Dot([c + x, r + y]),
        (
            Shape::Track {
                horizontal: false,
                at: x,
            },
            Shape::Local { row, col, y, .. },
        )
        | (
            Shape::Local { row, col, y, .. },
            Shape::Track {
                horizontal: false,
                at: x,
            },
        ) if here(row, col) => Join::Dot([c + x, r + y]),
        (
            Shape::Local {
                row: r1,
                col: c1,
                x: x1,
                y: y1,
            },
            Shape::Local {
                row: r2,
                col: c2,
                x: x2,
                y: y2,
            },
        ) if here(r1, c1) && (r1, c1) == (r2, c2) && y1 != y2 => {
            // The lower wire's lane crosses the upper wire's row.
            let (x, y) = if y1 < y2 { (x2, y1) } else { (x1, y2) };
            Join::Dot([c + x, r + y])
        }
        (
            Shape::Local {
                row, col, y: y1, ..
            }
            | Shape::Pin { row, col, y: y1 },
            Shape::Pin { y: y2, .. } | Shape::Local { y: y2, .. },
        ) if here(row, col) && (y1 - y2).abs() < 1e-6 => Join::Dot([c + BLOCK, r + y1]),
        _ => {
            // No crossing: join the points of each wire nearest the tile's
            // switchbox.
            let centre = [c + 0.2, r + 0.15];
            let near = |w: &WireShape| at_distance(&w.points, along(&w.points, centre));
            Join::Jumper(near(s), near(t))
        }
    }
}

fn location_of(model: &DeviceModel, tile: &str) -> Option<Location> {
    let t = model.tile(model.tile_id(tile)?);
    Some(Location {
        row: t.row,
        col: t.col,
    })
}

fn canonical(model: &DeviceModel, at: Location, name: &str) -> Option<Wire> {
    match model.wire(at, name).ok()? {
        Endpoint::Wire(w) => Some(w),
        _ => None,
    }
}

/// Slice, LUT and flip-flop use, independent of the routing geometry. The
/// output indices stay `NO_VALUE`: only a complete Geometry owns values.
/// The fabric overlay uses the positions and masks, not those indices.
pub fn logic_usage(model: &DeviceModel, report: &Report) -> Vec<LogicTile> {
    let mut used_f: BTreeMap<(i16, i16), u8> = BTreeMap::new();
    for route in &report.routes {
        // Canonicalization preserves this suffix, including relative names
        // and density prefixes. Other routes cannot name a slice output.
        if !route.source.ends_with("_SLICE") {
            continue;
        }
        let Some(at) = location_of(model, &route.tile) else {
            continue;
        };
        let Some(source) = canonical(model, at, &route.source) else {
            continue;
        };
        if let Some(n) = source
            .name
            .strip_prefix('F')
            .and_then(|x| x.strip_suffix("_SLICE"))
            .and_then(|x| x.parse::<u8>().ok())
        {
            if canonical(model, at, &route.sink).is_some() {
                *used_f.entry((at.row, at.col)).or_default() |= 1 << n;
            }
        }
    }
    logic_tiles(model, report, used_f)
}

fn logic_tiles(
    model: &DeviceModel,
    report: &Report,
    mut used_f: BTreeMap<(i16, i16), u8>,
) -> Vec<LogicTile> {
    let mut flip_flops: BTreeMap<(i16, i16), u8> = BTreeMap::new();
    let mut slices: BTreeMap<(i16, i16), u8> = BTreeMap::new();
    for field in &report.fields {
        let Some(at) = location_of(model, &field.tile) else {
            continue;
        };
        let Some(rest) = field.name.strip_prefix("SLICE") else {
            continue;
        };
        let Some(s @ 'A'..='D') = rest.chars().next() else {
            continue;
        };
        let s = (s as u8 - b'A') as usize;
        let tail = rest[1..].strip_prefix('.').unwrap_or("");
        *slices.entry((at.row, at.col)).or_default() |= 1 << s;
        if let Some(k) = tail
            .strip_prefix("REG")
            .and_then(|r| r.chars().next())
            .and_then(|c| c.to_digit(10))
        {
            *flip_flops.entry((at.row, at.col)).or_default() |= 1 << (2 * s + k as usize);
        }
        // A RAM slice uses both its LUTs. A CCU2 slice uses the halves the plan
        // computes, and the plan reads the carry setting of each half it
        // computes: the last slice of a chain may compute only its first half.
        if tail == "MODE" && field.candidates.iter().any(|c| c != "LOGIC" && c != "CCU2") {
            *used_f.entry((at.row, at.col)).or_default() |= 0b11 << (2 * s);
        }
        if let Some(k @ (b'0' | b'1')) = tail
            .strip_prefix("CCU2.INJECT1_")
            .and_then(|k| k.bytes().next())
        {
            *used_f.entry((at.row, at.col)).or_default() |= 1 << (2 * s + usize::from(k - b'0'));
        }
        // `SLICEA.B1MUX`: an input mux of LUT 1 of slice A is in the plan.
        if let [b'A'..=b'D', k @ (b'0' | b'1'), b'M', b'U', b'X'] = tail.as_bytes() {
            *used_f.entry((at.row, at.col)).or_default() |= 1 << (2 * s + usize::from(*k - b'0'));
        }
    }
    let mut logic = Vec::new();
    for (&(row, col), &mask) in &slices {
        let in_use: u8 = (0..4)
            .filter(|s| mask >> s & 1 == 1)
            .fold(0, |m, s| m | 0b11 << (2 * s));
        logic.push(LogicTile {
            row: row as u16,
            col: col as u16,
            slices: mask,
            luts: used_f.get(&(row, col)).copied().unwrap_or(0) & in_use,
            flip_flops: flip_flops.get(&(row, col)).copied().unwrap_or(0) & in_use,
            f: [NO_VALUE; 8],
            q: [NO_VALUE; 8],
        });
    }

    logic
}

/// The geometry of a compiled design on the die.
pub fn build(model: &DeviceModel, report: &Report) -> Geometry {
    build_uncached(model, report)
}

/// Builds the geometry without reading or writing a cache.
pub fn build_uncached(model: &DeviceModel, report: &Report) -> Geometry {
    let mut since = std::time::Instant::now();
    let profile = std::env::var_os("ECP5_LAYOUT_TIMES").is_some();
    let mut phase = |name: &str| {
        if profile {
            eprintln!("layout {name} {:.6} s", since.elapsed().as_secs_f64());
            since = std::time::Instant::now();
        }
    };
    let (mut rows, mut cols) = (0, 0);

    for (_, tile) in model.tiles() {
        rows = rows.max(tile.row as u16 + 1);
        cols = cols.max(tile.col as u16 + 1);
    }
    let location_of = |tile: &str| location_of(model, tile);
    let canonical = |at: Location, name: &str| canonical(model, at, name);

    phase("tiles");

    // The transfers, in canonical wires.
    let mut transfers: Vec<(Location, Wire, Wire)> = Vec::new();
    for route in &report.routes {
        let Some(at) = location_of(&route.tile) else {
            continue;
        };
        if let (Some(s), Some(t)) = (canonical(at, &route.source), canonical(at, &route.sink)) {
            transfers.push((at, s, t));
        }
    }

    phase("routes");

    // Conductors, one per inter-tile wire whatever tap names it, and local
    // wires per tile.
    type Conductor = (bool, i16, i16, i16, u16);
    let conductor_of = |w: &Wire| -> Option<Conductor> {
        let t = track(&w.name)?;
        let (fixed, a, b) = extent(&t, w.location);
        Some((t.horizontal, fixed, a, b, t.id))
    };
    let mut conductors: BTreeSet<Conductor> = BTreeSet::new();
    // The first tap of each conductor the transfers name, in their order.
    let mut first_tap: HashMap<Conductor, Wire> = HashMap::new();
    let mut locals: BTreeMap<(i16, i16), BTreeSet<String>> = BTreeMap::new();
    for (_, s, t) in &transfers {
        for w in [s, t] {
            match conductor_of(w) {
                Some(c) => {
                    conductors.insert(c);
                    first_tap.entry(c).or_insert_with(|| w.clone());
                }
                None => {
                    locals
                        .entry((w.location.row, w.location.col))
                        .or_default()
                        .insert(w.name.clone());
                }
            }
        }
    }
    let mut by_line: BTreeMap<(bool, i16), Vec<Conductor>> = BTreeMap::new();
    for &c in &conductors {
        by_line.entry((c.0, c.1)).or_default().push(c);
    }
    let mut lane_of: HashMap<Conductor, f32> = HashMap::new();
    for ((horizontal, _), list) in &by_line {
        let intervals: Vec<(i16, i16)> = list.iter().map(|c| (c.2, c.3)).collect();
        let (lanes, n) = colour(&intervals);
        let range = if *horizontal {
            (0.01, H_CHANNEL - 0.01)
        } else {
            V_LANES
        };
        for (c, k) in list.iter().zip(lanes) {
            lane_of.insert(*c, lane(range, k, n));
        }
    }

    phase("conductors_lanes");

    // Shapes of all wires.
    let mut values: Vec<Wire> = Vec::new();
    let mut value_index: HashMap<Wire, u32> = HashMap::new();
    let mut value_of = |w: &Wire| -> u32 {
        *value_index.entry(w.clone()).or_insert_with(|| {
            values.push(w.clone());
            (values.len() - 1) as u32
        })
    };
    let mut shapes: Vec<WireShape> = Vec::new();
    let mut shape_of_conductor: HashMap<Conductor, usize> = HashMap::new();
    for &c in &conductors {
        let at = lane_of[&c];
        let (horizontal, fixed, a, b, id) = c;
        let points = if horizontal {
            let y = fixed as f32 + at;
            vec![[a as f32 + 0.005, y], [b as f32 + 0.995, y]]
        } else {
            let x = fixed as f32 + at;
            vec![[x, a as f32 + 0.005], [x, b as f32 + 0.995]]
        };
        // Its value is that of any tap: they are one conductor.
        let tap = first_tap.get(&c).cloned().unwrap_or(Wire {
            location: Location { row: 0, col: 0 },
            name: format!("{id}"),
        });
        shapes.push(WireShape {
            shape: Shape::Track { horizontal, at },
            points,
            value: value_of(&tap),
        });
        shape_of_conductor.insert(c, shapes.len() - 1);
    }
    phase("tracks");

    // Local wires: pins of the slices where they are known, the rest spread
    // over the band; lanes as a staircase, higher pins further left.
    let mut shape_of_local: HashMap<Wire, usize> = HashMap::new();
    for (&(row, col), names) in &locals {
        let (r, c) = (row as f32, col as f32);
        let mut placed: Vec<(String, f32, Option<f32>, bool)> = Vec::new();
        let mut others: Vec<&String> = Vec::new();
        for name in names {
            match slice_pin(name) {
                Some((y, x, side)) => placed.push((name.clone(), y, Some(x), side)),
                None => others.push(name),
            }
        }
        // Heights away from the slice pins, or any when a full tile has none.
        let taken: Vec<f32> = placed.iter().map(|p| p.1).collect();
        let slots = (others.len() * 4).max(240);
        let all: Vec<f32> = (0..slots)
            .map(|k| lane((BAND_TOP + 0.005, 0.995), k, slots))
            .collect();
        let mut free: Vec<f32> = all
            .iter()
            .copied()
            .filter(|y| taken.iter().all(|t| (t - y).abs() > 0.0015))
            .collect();
        if free.len() < others.len() {
            free = all;
        }
        for (k, name) in others.iter().enumerate() {
            let y = free[k * free.len() / others.len().max(1)];
            placed.push(((*name).clone(), y, None, name.ends_with("_SLICE")));
        }
        let mut heights: Vec<f32> = placed.iter().filter(|p| !p.3).map(|p| p.1).collect();
        heights.sort_by(f32::total_cmp);
        heights.dedup();
        for (name, y, pin_x, side) in placed {
            let wire = Wire {
                location: Location { row, col },
                name,
            };
            let (shape, points) = if side {
                let x = pin_x.unwrap_or(BLOCK + 0.05);
                let mut points = vec![[c + x, r + y], [c + BLOCK, r + y]];
                if wire.name.starts_with('Q') {
                    // Down from the flip-flop it comes from.
                    points.insert(0, [c + x, r + y - 0.22 * ROW]);
                }
                (Shape::Pin { row, col, y }, points)
            } else {
                let k = heights.iter().position(|h| *h == y).unwrap_or(0);
                let x = lane(PIN_LANES, k, heights.len());
                (
                    Shape::Local { row, col, x, y },
                    vec![[c + BLOCK, r + y], [c + x, r + y], [c + x, r + 0.005]],
                )
            };
            shapes.push(WireShape {
                shape,
                points,
                value: value_of(&wire),
            });
            shape_of_local.insert(wire, shapes.len() - 1);
        }
    }
    let index = |w: &Wire| -> usize {
        match conductor_of(w) {
            Some(c) => shape_of_conductor[&c],
            None => shape_of_local[w],
        }
    };

    phase("locals");

    // Joins, with jumper lanes counted per tile first.
    let pairs: Vec<(Location, usize, usize)> = transfers
        .iter()
        .map(|(at, s, t)| (*at, index(s), index(t)))
        .collect();
    let mut jumper_count: BTreeMap<(i16, i16, bool), usize> = BTreeMap::new();
    for (at, s, t) in &pairs {
        if let (Shape::Track { horizontal: a, .. }, Shape::Track { horizontal: b, .. }) =
            (shapes[*s].shape, shapes[*t].shape)
        {
            if a == b {
                *jumper_count.entry((at.row, at.col, a)).or_default() += 1;
            }
        }
    }
    let mut jumper_next: BTreeMap<(i16, i16, bool), usize> = BTreeMap::new();
    let mut joins: Vec<(usize, usize, Join)> = Vec::new();
    for (at, s, t) in pairs {
        let mut jumper = |horizontal: bool| -> f32 {
            let key = (at.row, at.col, horizontal);
            let k = jumper_next.entry(key).or_default();
            *k += 1;
            let n = jumper_count.get(&key).copied().unwrap_or(1);
            lane(if horizontal { H_JUMPERS } else { V_JUMPERS }, *k - 1, n)
        };
        let j = join(at, &shapes[s], &shapes[t], &mut jumper);
        joins.push((s, t, j));
    }

    phase("joins");

    // Nets: wires joined by transfers.
    let mut parent: Vec<usize> = (0..shapes.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (s, t, _) in &joins {
        let (a, b) = (find(&mut parent, *s), find(&mut parent, *t));
        if a != b {
            parent[a] = b;
        }
    }
    let mut net_ids: HashMap<usize, u32> = HashMap::new();
    let net: Vec<u32> = (0..shapes.len())
        .map(|i| {
            let root = find(&mut parent, i);
            let next = net_ids.len() as u32;
            *net_ids.entry(root).or_insert(next)
        })
        .collect();

    phase("nets");

    // The graph: every wire split where transfers join it.
    let mut stops: Vec<Vec<f32>> = vec![Vec::new(); shapes.len()];
    for (s, t, j) in &joins {
        let (ps, pt) = match j {
            Join::Dot(p) => (*p, *p),
            Join::Jumper(a, b) => (*a, *b),
        };
        stops[*s].push(along(&shapes[*s].points, ps));
        stops[*t].push(along(&shapes[*t].points, pt));
    }
    let mut node_of: HashMap<(i64, i64, u32), usize> = HashMap::new();
    let mut nodes: Vec<[f32; 2]> = Vec::new();
    let mut node = |p: [f32; 2], n: u32, nodes: &mut Vec<[f32; 2]>| -> usize {
        let key = (
            (p[0] * 4096.0).round() as i64,
            (p[1] * 4096.0).round() as i64,
            n,
        );
        *node_of.entry(key).or_insert_with(|| {
            nodes.push(p);
            nodes.len() - 1
        })
    };
    let mut edges: Vec<(usize, usize, SegmentKind, u32, u32)> = Vec::new();
    for (i, shape) in shapes.iter().enumerate() {
        let kind = match shape.shape {
            Shape::Track { .. } => SegmentKind::Track,
            _ => SegmentKind::Local,
        };
        let mut cuts: Vec<f32> = stops[i].clone();
        let mut walked = 0.0;
        cuts.push(0.0);
        for w in shape.points.windows(2) {
            walked += length(w[0], w[1]);
            cuts.push(walked);
        }
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
        let points: Vec<usize> = cuts
            .iter()
            .map(|&s| node(at_distance(&shape.points, s), net[i], &mut nodes))
            .collect();
        for w in points.windows(2) {
            if w[0] != w[1] {
                edges.push((w[0], w[1], kind, shape.value, net[i]));
            }
        }
    }
    let mut dot_nodes = Vec::new();
    for (s, t, j) in &joins {
        let n = net[*s];
        let value = shapes[*t].value;
        match j {
            Join::Dot(p) => dot_nodes.push((node(*p, n, &mut nodes), value, n)),
            Join::Jumper(a, b) => {
                let (u, v) = (node(*a, n, &mut nodes), node(*b, n, &mut nodes));
                if u != v {
                    edges.push((u, v, SegmentKind::Jumper, value, n));
                }
                dot_nodes.push((u, value, n));
                dot_nodes.push((v, value, n));
            }
        }
    }

    phase("graph");

    // Distances from each net's driver: the start of a wire that no
    // transfer drives, usually a pin of a site.
    let mut driven = vec![false; shapes.len()];
    for (_, t, _) in &joins {
        driven[*t] = true;
    }
    let mut starts: Vec<usize> = Vec::new();
    let mut has_driver: BTreeSet<u32> = BTreeSet::new();
    for (i, shape) in shapes.iter().enumerate() {
        if !driven[i] {
            starts.push(node(shape.points[0], net[i], &mut nodes));
            has_driver.insert(net[i]);
        }
    }
    // A net whose every wire is driven (a loop) starts anywhere.
    for (i, shape) in shapes.iter().enumerate() {
        if has_driver.insert(net[i]) {
            starts.push(node(shape.points[0], net[i], &mut nodes));
        }
    }
    let mut adjacency: Vec<Vec<(usize, f32)>> = vec![Vec::new(); nodes.len()];
    for &(u, v, ..) in &edges {
        let d = length(nodes[u], nodes[v]);
        adjacency[u].push((v, d));
        adjacency[v].push((u, d));
    }
    let mut distance = vec![f32::INFINITY; nodes.len()];
    let mut heap = BinaryHeap::new();
    for s in starts {
        distance[s] = 0.0;
        heap.push(Reverse((0_u32, s)));
    }
    while let Some(Reverse((d, u))) = heap.pop() {
        let d = f32::from_bits(d);
        if d > distance[u] {
            continue;
        }
        for &(v, w) in &adjacency[u] {
            let nd = d + w;
            if nd < distance[v] {
                distance[v] = nd;
                heap.push(Reverse((nd.to_bits(), v)));
            }
        }
    }
    let dist = |u: usize| {
        if distance[u].is_finite() {
            distance[u]
        } else {
            0.0
        }
    };
    let longest = distance
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .fold(0.0, f32::max);
    phase("distances");

    let segments: Vec<Segment> = edges
        .iter()
        .map(|&(u, v, kind, value, net)| Segment {
            a: nodes[u],
            b: nodes[v],
            kind,
            value,
            net,
            d0: dist(u),
            d1: dist(v),
        })
        .collect();
    phase("segments");

    let mut seen = BTreeSet::new();
    let dots: Vec<Dot> = dot_nodes
        .into_iter()
        .filter(|(u, ..)| seen.insert(*u))
        .map(|(u, value, net)| Dot {
            at: nodes[u],
            value,
            net,
            d: dist(u),
        })
        .collect();

    phase("dots");

    // Logic tiles: slices, LUTs and flip-flops in use from the plan.
    let mut used_f: BTreeMap<(i16, i16), u8> = BTreeMap::new();
    for (at, s, _) in &transfers {
        if let Some(n) = s
            .name
            .strip_prefix('F')
            .and_then(|x| x.strip_suffix("_SLICE"))
            .and_then(|x| x.parse::<u8>().ok())
        {
            *used_f.entry((at.row, at.col)).or_default() |= 1 << n;
        }
    }
    let mut logic = logic_tiles(model, report, used_f);
    // Assign output indices after the route shapes, in the original order.
    for tile in &mut logic {
        let at = Location {
            row: tile.row as i16,
            col: tile.col as i16,
        };
        let mut value = |name: String| canonical(at, &name).map_or(NO_VALUE, |w| value_of(&w));
        // The outputs as the slices name them: the tile's F1 may carry the
        // FX mux's output, F0 the F5 mux's.
        tile.f = std::array::from_fn(|n| value(format!("F{n}_SLICE")));
        tile.q = std::array::from_fn(|n| value(format!("Q{n}_SLICE")));
    }

    phase("logic");

    // Other sites, stacked in the block of their tile; an EBR spans to the
    // next one.
    let used: BTreeSet<&str> = report.resources.iter().map(|r| r.id.as_str()).collect();
    type Named<'a> = (&'a str, &'a str, &'a str);
    let mut by_position: BTreeMap<(i16, i16), Vec<Named>> = BTreeMap::new();
    for (_, r) in model.resources() {
        if matches!(r.family.as_str(), "PLC2" | "SLICE") {
            continue;
        }
        by_position
            .entry((r.location.row, r.location.col))
            .or_default()
            .push((r.family.as_str(), r.name.as_str(), r.key.as_str()));
    }
    let mut ebr_columns: BTreeMap<i16, Vec<i16>> = BTreeMap::new();
    for (&(r, c), list) in &by_position {
        if list.iter().any(|(f, _, _)| *f == "EBR") {
            ebr_columns.entry(r).or_default().push(c);
        }
    }
    let mut sites = Vec::new();
    for (&(row, col), list) in &by_position {
        let n = list.len() as f32;
        for (k, &(family, name, key)) in list.iter().enumerate() {
            let kind = site_kind(family);
            let (r, c) = (row as f32, col as f32);
            let (min, max) = if kind == SiteKind::Ebr {
                let next = ebr_columns[&row]
                    .iter()
                    .find(|&&x| x > col)
                    .copied()
                    .unwrap_or(col + 3);
                let span = (next - col).clamp(1, 3) as f32;
                (
                    [c + BLOCK + 0.02, r + BAND_TOP + 0.03],
                    [c + span - 0.02, r + 0.97],
                )
            } else {
                let band = (1.0 - BAND_TOP - 0.02) / n;
                (
                    [
                        c + BLOCK + 0.02,
                        r + BAND_TOP + 0.01 + band * k as f32 + 0.005,
                    ],
                    [
                        c + 0.98,
                        r + BAND_TOP + 0.01 + band * (k as f32 + 1.0) - 0.005,
                    ],
                )
            };
            sites.push(Site {
                min,
                max,
                kind,
                name: name.into(),
                used: used.contains(key),
            });
        }
    }
    phase("sites");

    Geometry {
        rows,
        cols,
        segments,
        dots,
        sites,
        logic,
        values,
        longest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_parse_span_direction_and_tap() {
        let t = track("V06N0303").unwrap();
        assert_eq!(
            (t.horizontal, t.span, t.step, t.tap),
            (false, 6, (-1, 0), 3)
        );
        let t = track("H02E0101").unwrap();
        assert_eq!((t.horizontal, t.span, t.step, t.tap), (true, 2, (0, 1), 1));
        assert_eq!(track("H00R0000").unwrap().span, 1);
        assert!(
            track("A0").is_none() && track("G_HPBX0000").is_none() && track("H06X0103").is_none()
        );
        assert_ne!(track("H02E0100").unwrap().id, track("H02W0100").unwrap().id);
    }

    #[test]
    fn every_tap_of_a_conductor_has_the_same_extent() {
        // V06N03xx: tap 0 at R20, tap 3 at R17, tap 6 at R14.
        let e = |row, name: &str| extent(&track(name).unwrap(), Location { row, col: 20 });
        assert_eq!(e(20, "V06N0300"), (20, 14, 20));
        assert_eq!(e(17, "V06N0303"), e(20, "V06N0300"));
        assert_eq!(e(14, "V06N0306"), e(20, "V06N0300"));
        let h = |col, name: &str| extent(&track(name).unwrap(), Location { row: 5, col });
        assert_eq!(h(10, "H02E0100"), (5, 10, 12));
        assert_eq!(h(12, "H02E0102"), h(10, "H02E0100"));
        assert_eq!(h(10, "H02W0100"), (5, 8, 10));
    }

    #[test]
    fn overlapping_conductors_get_their_own_lanes() {
        let (lanes, n) = colour(&[(0, 2), (1, 3), (3, 5), (6, 8)]);
        assert_eq!(n, 2);
        assert_ne!(lanes[0], lanes[1]);
        assert_ne!(lanes[1], lanes[2]);
        assert_eq!(lanes[3], lanes[0]);
    }

    #[test]
    fn slice_pins_sit_on_their_site() {
        for n in 0..8 {
            let (min, max) = lut_rect(n);
            for letter in ["A", "B", "C", "D", "F"] {
                let (y, x, _) = slice_pin(&format!("{letter}{n}")).unwrap();
                assert!(y > min[1] && y < max[1] && x == min[0], "{letter}{n}");
            }
            // M, DI and Q run below the LUT to the flip-flop.
            for letter in ["M", "DI", "Q"] {
                let (y, x, _) = slice_pin(&format!("{letter}{n}")).unwrap();
                assert!(
                    y > max[1] && y < row_top(n) + ROW && x > ff_rect(n).0[0],
                    "{letter}{n}"
                );
            }
        }
        assert!(slice_pin("Q3_SLICE").unwrap().2);
        assert!(slice_pin("X0").is_none());
    }

    /// Every join lies on both of its wires: a dot where they cross, or a
    /// jumper whose ends are on each.
    #[test]
    fn joins_lie_on_their_wires() {
        let at = Location { row: 3, col: 7 };
        let local = |x: f32, y: f32| WireShape {
            shape: Shape::Local {
                row: 3,
                col: 7,
                x,
                y,
            },
            points: vec![[7.0 + BLOCK, 3.0 + y], [7.0 + x, 3.0 + y], [7.0 + x, 3.005]],
            value: 0,
        };
        let h = WireShape {
            shape: Shape::Track {
                horizontal: true,
                at: 0.1,
            },
            points: vec![[5.005, 3.1], [9.995, 3.1]],
            value: 0,
        };
        let v = WireShape {
            shape: Shape::Track {
                horizontal: false,
                at: 0.2,
            },
            points: vec![[7.2, 1.005], [7.2, 4.995]],
            value: 0,
        };
        let h2 = WireShape {
            shape: Shape::Track {
                horizontal: true,
                at: 0.2,
            },
            points: vec![[6.005, 3.2], [8.995, 3.2]],
            value: 0,
        };
        let (upper, lower) = (local(0.05, 0.5), local(0.10, 0.7));
        let mut jumper = |h: bool| if h { 0.37 } else { 0.30 };
        let on = |w: &WireShape, p: [f32; 2]| {
            length(at_distance(&w.points, along(&w.points, p)), p) < 1e-5
        };
        for (s, t) in [
            (&h, &v),
            (&v, &h),
            (&h, &upper),
            (&lower, &v),
            (&upper, &lower),
            (&lower, &upper),
            (&h, &h2),
        ] {
            match join(at, s, t, &mut jumper) {
                Join::Dot(p) => {
                    assert!(on(s, p) && on(t, p), "{:?} {:?} at {p:?}", s.shape, t.shape)
                }
                Join::Jumper(a, b) => assert!(on(s, a) && on(t, b), "{:?} {:?}", s.shape, t.shape),
            }
        }
    }

    #[test]
    fn polyline_distances() {
        let l = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert_eq!(along(&l, [1.0, 0.5]), 1.5);
        assert_eq!(at_distance(&l, 1.5), [1.0, 0.5]);
    }
}
