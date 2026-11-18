//! A design over the fabric (src/fabric.rs): which of the die's drawn
//! wires, arcs and links the design uses, the value each carries, and how
//! far along its net each piece lies from the net's driver, so that a new
//! value can run along the wires as it would on the chip.
//!
//! A wire of the fabric is a tap of a track at a position, or a name of the
//! position's tile types; the design's wires are the model's (src/model),
//! where a track is named once, at one of its taps. Both are taken to the
//! same identity: a track by the position of its tap 0, a wire of the clock
//! tree as its segment (`G_HPBX0000` of a tile is the branch of its tap),
//! any other wire as the model places it. An arc is in use when the
//! design's plan routes it (`Report::routes`), and a fixed arc when a wire
//! it joins is in use; a wire is in use when an arc in use reaches it.
//!
//! A net is the wires that arcs in use join, and the clock buffers (DCC)
//! join their input and output. Of a net only the way from its drivers to
//! the sites it feeds is on its route: the rest of a conductor carries the
//! value too, but on to nothing. Each net knows its pieces, where its
//! drivers leave their sites and where it enters the sites it feeds.
//!
//! A connection inside a slice is in use when the plan uses both its ends:
//! the carry between two CCU2 halves when the second takes the carry in, a
//! LUT into the F5 mux when the mux output is routed, the M input into a
//! flip-flop the plan loads from M, a control into a flip-flop in use. It
//! carries the value of the port it comes from, and it joins the net of the
//! pin it continues, so a route through a slice reaches the part it feeds.
use crate::{
    compile::{ConstantUse, FieldUse, RouteUse},
    fabric::{
        parse_track, split_prefix, step, this_die, BoxKind, Fabric, MacroPiece, Owner, Part,
        TrackId, WireKey, TILE_W,
    },
    layout::LogicTile,
    logic::Logic,
    model::{
        topology::{Endpoint, Location, Wire},
        DeviceModel,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    collections::{hash_map::Entry, BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

/// What a piece draws, for the zoom it shows from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Track,
    Local,
    Jumper,
    Pin,
    /// An arc to a neighbour's wire, drawn between the tiles.
    Link,
    /// A wire of a DSP block.
    Block,
    /// The clock tree: a branch along a row, a spine down a column.
    Clock,
    /// A connection inside a slice.
    Internal,
}

impl From<Part> for Kind {
    fn from(p: Part) -> Self {
        match p {
            Part::Track => Kind::Track,
            Part::Local => Kind::Local,
            Part::Jumper => Kind::Jumper,
            Part::Pin => Kind::Pin,
            Part::Clock => Kind::Clock,
            Part::Internal => Kind::Internal,
        }
    }
}

/// A straight piece of a wire in use, in die units, within one tile: the
/// value it carries, how far its ends are from its net's driver, and
/// whether it is on the way from the driver to a site the net feeds (the
/// rest of a conductor carries the value too, but on to nothing).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub value: u32,
    pub d0: f32,
    pub d1: f32,
    pub kind: Kind,
    pub route: bool,
}

/// The dot of an arc in use, or of a wire in use where a link arrives.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dot {
    pub at: [f32; 2],
    pub value: u32,
    pub d: f32,
    pub route: bool,
}

/// A LUT or a flip-flop in use, with the values of its outputs. `value`
/// is the one it shows: a flip-flop's Q, a LUT's F (also where no route
/// takes it: into a wide mux, or a CCU2 half that only makes the carry);
/// u32::MAX for none. `also` has its other outputs a
/// change of which it flashes with too: the carry out of a CCU2 half, and
/// in slice C as RAMW (the write port of the RAM in slices A and B, where
/// the LUTs have no F) the write address (WADO) or data (WDO) it passes on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub value: u32,
    pub also: Vec<u32>,
    pub label: String,
}

/// What half k of slice C as RAMW passes on: the write address from LUT4
/// (WADO0–3 from its D, B, C, A), the write data from LUT5 (WDO0–3 from its
/// C, A, D, B).
pub fn ramw_outputs(k: usize) -> [(String, char); 4] {
    let (name, from) = if k == 0 {
        ("WADO", ['D', 'B', 'C', 'A'])
    } else {
        ("WDO", ['C', 'A', 'D', 'B'])
    };
    std::array::from_fn(|i| (format!("{name}{i}C_SLICE"), from[i]))
}

/// A site other than a LUT or a flip-flop that the design uses (an EBR, a
/// pad, a part of a DSP block, a slice's wide muxes or controls): the way
/// of some net enters or leaves it. The values of those pins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsedBox {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub values: Vec<u32>,
    pub label: String,
    /// A site of its own (its label at its right), not a part of a slice.
    pub site: bool,
}

#[derive(PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    /// The design's wires whose values are shown, as the model names them.
    #[serde(with = "crate::design_cache::cached_wires")]
    pub values: Vec<Wire>,
    /// The net of each, as the index of one of its wires.
    pub nets: Vec<u32>,
    /// By position (row major), as ranges into `pieces`, `dots`, `sites`.
    pub pieces: Vec<Piece>,
    pub dots: Vec<Dot>,
    pub sites: Vec<Site>,
    pub boxes: Vec<UsedBox>,
    pieces_at: Vec<(u32, u32)>,
    boxes_at: Vec<(u32, u32)>,
    dots_at: Vec<(u32, u32)>,
    sites_at: Vec<(u32, u32)>,
    /// The furthest any piece is from its net's driver.
    pub longest: f32,
    /// Routes of the plan, those drawn, and those not drawn by why.
    pub routes: usize,
    pub drawn: usize,
    pub not_drawn: BTreeMap<String, usize>,
    /// Of the drawn, those the fabric shows with no dot, jumper or link:
    /// left out because they would join a wire to nothing else; by the
    /// families of their wires and the tile types.
    pub hidden: usize,
    pub hidden_by: BTreeMap<String, usize>,
    /// Pieces whose net has no driver drawn: their value starts from a
    /// loose end.
    pub undriven: usize,
    /// Wires in use only through fixed arcs (a site's pins, mostly).
    pub carried: usize,
    /// Pieces of nets that feed nothing in use: not shown.
    pub unfed: usize,
    /// Where nets start that are not a site's pin (a constant's at the
    /// CIB's): their driver is not drawn.
    pub loose: Vec<(u32, [f32; 2])>,
    /// The pieces of each net (by index into `pieces`), where its drivers
    /// leave their sites and where it enters the sites it feeds.
    #[serde(serialize_with = "ordered_map")]
    nets_at: HashMap<u32, (u32, u32)>,
    by_net: Vec<u32>,
    #[serde(serialize_with = "ordered_map")]
    drivers: HashMap<u32, Vec<[f32; 2]>>,
    #[serde(serialize_with = "ordered_map")]
    fed_at: HashMap<u32, Vec<[f32; 2]>>,
    /// Nets a constant of a CIB drives (an unused input tied off), and to
    /// what.
    #[serde(serialize_with = "ordered_map")]
    tied: HashMap<u32, bool>,
    /// Each wire of the design's conductors, by value.
    #[serde(serialize_with = "ordered_map")]
    index: HashMap<Ident, u32>,
}

impl Overlay {
    pub fn pieces_in(&self, position: usize) -> &[Piece] {
        &self.pieces[self.piece_range(position)]
    }
    /// The pieces at a position, as indices into `pieces`.
    pub fn piece_range(&self, position: usize) -> std::ops::Range<usize> {
        let (a, b) = self.pieces_at[position];
        a as usize..b as usize
    }
    pub fn dots_in(&self, position: usize) -> &[Dot] {
        let (a, b) = self.dots_at[position];
        &self.dots[a as usize..b as usize]
    }
    pub fn sites_in(&self, position: usize) -> &[Site] {
        let (a, b) = self.sites_at[position];
        &self.sites[a as usize..b as usize]
    }
    /// The other sites in use at a position.
    pub fn boxes_in(&self, position: usize) -> &[UsedBox] {
        let (a, b) = self.boxes_at[position];
        &self.boxes[a as usize..b as usize]
    }
    /// The pieces of a net, by index into `pieces`.
    pub fn pieces_of(&self, net: u32) -> &[u32] {
        let (a, b) = self.nets_at.get(&net).copied().unwrap_or((0, 0));
        &self.by_net[a as usize..b as usize]
    }
    /// Where a net's drivers leave their sites.
    pub fn drivers_of(&self, net: u32) -> &[[f32; 2]] {
        self.drivers.get(&net).map_or(&[], Vec::as_slice)
    }
    /// The constant a net is tied to, when a CIB drives it with one: its
    /// start is then where the wire leaves the CIB, not a site.
    pub fn tied(&self, net: u32) -> Option<bool> {
        self.tied.get(&net).copied()
    }
    /// The nets tied to a constant.
    pub fn tied_nets(&self) -> impl Iterator<Item = (u32, bool)> + '_ {
        self.tied.iter().map(|(&n, &v)| (n, v))
    }
    /// Where a net enters the sites it feeds.
    pub fn fed_of(&self, net: u32) -> &[[f32; 2]] {
        self.fed_at.get(&net).map_or(&[], Vec::as_slice)
    }
    /// The value (index into `values`) of a wire of the design, as the model
    /// names it: none for a wire the design does not use.
    pub fn value_of(&self, model: &DeviceModel, wire: &Wire) -> Option<u32> {
        self.index
            .get(&ident(&canonical(model, wire.clone())))
            .copied()
    }
}

/// A conductor: a track by where its tap 0 is, any other wire as placed.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Ident {
    Track(TrackId, i16, i16),
    Wire(#[serde(with = "crate::design_cache::CachedWire")] Wire),
}

// Hash table seeds are process-local; cache bytes must not depend on them.
fn ordered_map<S, K, V>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    K: Ord + Serialize,
    V: Serialize,
{
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
    serializer.collect_map(entries)
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Track(a, r, c), Self::Track(b, s, d)) => (a, r, c).cmp(&(b, s, d)),
            (Self::Wire(a), Self::Wire(b)) => (a.location.row, a.location.col, &a.name).cmp(&(
                b.location.row,
                b.location.col,
                &b.name,
            )),
            (Self::Track(..), Self::Wire(_)) => std::cmp::Ordering::Less,
            (Self::Wire(_), Self::Track(..)) => std::cmp::Ordering::Greater,
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The same performance-core count used by fast mode, with a portable fallback.
fn thread_count() -> usize {
    if let Some(n) = std::env::var("ECP5_OVERLAY_THREADS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
    {
        return n;
    }
    crate::fast::performance_cores()
}

// Contiguous ranges, joined in input order. No worker assigns global indices
// or accumulates floating-point results across ranges.
fn ranges<T: Send>(len: usize, threads: usize, work: impl Fn(Range<usize>) -> T + Sync) -> Vec<T> {
    let workers = threads.min(len.div_ceil(64)).max(1);
    if workers == 1 {
        return vec![work(0..len)];
    }
    std::thread::scope(|scope| {
        let work = &work;
        let chunk = len.div_ceil(workers);
        let handles: Vec<_> = (0..len)
            .step_by(chunk)
            .map(|start| scope.spawn(move || work(start..(start + chunk).min(len))))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

const CACHE_MAGIC: &[u8; 8] = b"ECP5OVL1";
const CACHE_VERSION: u32 = 2;
const CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;
const HEADER_LEN: usize = 8 + 4 + 8 + 32;

/// The compiled design's inputs and the overlay format.
#[derive(Debug, PartialEq, Eq)]
pub struct OverlayKey([u8; 32]);

impl OverlayKey {
    /// Uses the identity returned by [`crate::compile::cache_key`].
    pub fn new(compile_key: &[u8; 32]) -> Self {
        Self(crate::design_cache::key(compile_key, CACHE_VERSION))
    }

    pub fn cache_path(&self, dir: &Path) -> PathBuf {
        crate::design_cache::path(&self.0, dir)
    }
}

/// Encodes every field, including the lookup tables, with a version,
/// payload length and SHA-256 checksum.
pub fn encode_cache(overlay: &Overlay) -> postcard::Result<Vec<u8>> {
    let mut buf = postcard::to_extend(overlay, vec![0u8; HEADER_LEN])?;
    let payload_len = buf.len() - HEADER_LEN;
    let checksum: [u8; 32] = Sha256::digest(&buf[HEADER_LEN..]).into();
    buf[0..8].copy_from_slice(CACHE_MAGIC);
    buf[8..12].copy_from_slice(&CACHE_VERSION.to_le_bytes());
    buf[12..20].copy_from_slice(&(payload_len as u64).to_le_bytes());
    buf[20..52].copy_from_slice(&checksum);
    Ok(buf)
}

/// Reads an overlay only after its complete header and payload check out.
pub fn decode_cache(bytes: &[u8]) -> Option<Overlay> {
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
    let (overlay, rest) = postcard::take_from_bytes(payload).ok()?;
    rest.is_empty().then_some(overlay)
}

/// Keeps the newest cache files within 1 GiB, or `ECP5_OVERLAY_CACHE_MB`
/// MiB. The newest always stays, even when it alone exceeds the cap. Once
/// a file exceeds the remaining budget, it and all older files are removed.
/// Files that cannot be read or removed stay.
pub fn prune_cache(dir: &Path) {
    let limit = crate::design_cache::limit("ECP5_OVERLAY_CACHE_MB", CACHE_MAX_BYTES);
    crate::design_cache::prune(dir, limit);
}

/// Like [`cached`], using `dir` as the cache directory.
#[allow(clippy::too_many_arguments)]
pub fn cached_in(
    f: &Fabric,
    model: &DeviceModel,
    routes: &[RouteUse],
    constants: &[ConstantUse],
    logic: &[LogicTile],
    fields: &[FieldUse],
    key: Option<&OverlayKey>,
    dir: &Path,
) -> Overlay {
    if std::env::var("ECP5_OVERLAY_CACHE").is_ok_and(|v| v.trim() == "0") {
        return build(f, model, routes, constants, logic, fields);
    }
    let Some(key) = key else {
        return build(f, model, routes, constants, logic, fields);
    };
    let path = key.cache_path(dir);
    if let Some(overlay) = std::fs::read(&path).ok().and_then(|b| decode_cache(&b)) {
        return overlay;
    }
    let profile = std::env::var_os("ECP5_OVERLAY_TIMES").is_some();
    let since = std::time::Instant::now();
    let overlay = build(f, model, routes, constants, logic, fields);
    if profile {
        eprintln!("overlay build_total {:.6} s", since.elapsed().as_secs_f64());
    }
    let since = std::time::Instant::now();
    if let Ok(bytes) = encode_cache(&overlay) {
        if profile {
            eprintln!(
                "overlay cache_encode {:.6} s",
                since.elapsed().as_secs_f64()
            );
        }
        let since = std::time::Instant::now();
        if crate::design_cache::write_cache_atomic(&path, &bytes).is_ok() {
            prune_cache(dir);
        }
        if profile {
            eprintln!(
                "overlay cache_write_prune {:.6} s",
                since.elapsed().as_secs_f64()
            );
        }
    }
    overlay
}

/// Returns a checked overlay from disk, else builds it and writes it for
/// the next run. `ECP5_OVERLAY_CACHE=0` turns off reads and writes.
pub fn cached(
    f: &Fabric,
    model: &DeviceModel,
    routes: &[RouteUse],
    constants: &[ConstantUse],
    logic: &[LogicTile],
    fields: &[FieldUse],
    key: Option<&OverlayKey>,
) -> Overlay {
    cached_in(
        f,
        model,
        routes,
        constants,
        logic,
        fields,
        key,
        &std::env::temp_dir().join("ecp5_emu_overlay"),
    )
}

fn ident(w: &Wire) -> Ident {
    match parse_track(&w.name) {
        Some((id, tap)) => {
            let s = step(&id);
            Ident::Track(
                id,
                w.location.row - tap as i16 * s.0,
                w.location.col - tap as i16 * s.1,
            )
        }
        None => Ident::Wire(w.clone()),
    }
}

/// A wire as the plan has it: a clock branch, spine or quadrant wire is
/// its segment of the clock tree (`G_HPBX0000` of a tile is the
/// `L_HPBX0000` of its tap).
fn canonical(model: &DeviceModel, w: Wire) -> Wire {
    model.clock_segment(&w).ok().flatten().unwrap_or(w)
}

/// A name of a template that is not the wire of its own position with its
/// own name: a neighbour's, the clock tree's, a renamed PCS wire.
fn placed_elsewhere(name: &str) -> bool {
    let (dr, dc, _) = split_prefix(name);
    (dr, dc) != (0, 0) || name.starts_with("G_") || name.contains("PCSA")
}

/// The wires of a template by key, and its names placed elsewhere.
struct WireIndex {
    tracks: HashMap<(TrackId, u8), u32>,
    /// A name may be two wires of a template: a tap's local wire of the
    /// clock and the branch it drives.
    names: HashMap<String, Vec<u32>>,
    elsewhere: Vec<(u32, String)>,
    segments: Vec<Vec<u32>>,
    dots: Vec<Vec<u32>>,
    drawn_arcs: Vec<bool>,
    pins: Vec<(u32, Vec<[f32; 2]>)>,
}

impl WireIndex {
    fn new(t: &crate::fabric::Template) -> Self {
        let wires = &t.wires;
        let mut index = Self {
            tracks: HashMap::new(),
            names: HashMap::new(),
            elsewhere: Vec::new(),
            segments: vec![Vec::new(); wires.len() + t.arcs.len()],
            dots: vec![Vec::new(); wires.len() + t.arcs.len()],
            drawn_arcs: vec![false; t.arcs.len()],
            pins: Vec::new(),
        };
        for (k, w) in wires.iter().enumerate() {
            match w {
                WireKey::Track(id, tap) => {
                    index.tracks.insert((*id, *tap), k as u32);
                }
                WireKey::Name(n) if placed_elsewhere(n) => {
                    index.elsewhere.push((k as u32, n.clone()))
                }
                WireKey::Name(n) => {
                    index.names.entry(n.clone()).or_default().push(k as u32);
                }
            }
        }
        let owner = |o: &Owner| match o {
            Owner::Wire(k) => *k as usize,
            Owner::Arc(j) => wires.len() + *j as usize,
        };
        let mut pins: BTreeMap<u32, Vec<[f32; 2]>> = BTreeMap::new();
        for (i, (seg, o)) in t.segments.iter().zip(&t.owners).enumerate() {
            index.segments[owner(o)].push(i as u32);
            if let Owner::Arc(j) = o {
                index.drawn_arcs[*j as usize] = true;
            }
            if let (Part::Pin, Owner::Wire(k)) = (seg.2, o) {
                let ends = pins.entry(*k).or_default();
                ends.extend(
                    [seg.0, seg.1]
                        .into_iter()
                        .filter(|q| t.site_ends.contains(q)),
                );
            }
        }
        index.pins = pins.into_iter().collect();
        for (i, o) in t.dot_owners.iter().enumerate() {
            index.dots[owner(o)].push(i as u32);
            if let Owner::Arc(j) = o {
                index.drawn_arcs[*j as usize] = true;
            }
        }
        index
    }
}

/// Union-find over value indices: the nets.
struct Nets(Vec<u32>);

impl Nets {
    fn grow(&mut self, n: usize) {
        while self.0.len() < n {
            self.0.push(self.0.len() as u32);
        }
    }
    fn find(&mut self, mut i: u32) -> u32 {
        while self.0[i as usize] != i {
            let up = self.0[self.0[i as usize] as usize];
            self.0[i as usize] = up;
            i = up;
        }
        i
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a as usize] = b;
        }
    }
}

/// A piece in use before it is split where other pieces of its net meet it.
struct Raw {
    a: [f32; 2],
    b: [f32; 2],
    value: u32,
    kind: Kind,
    /// A pin that a fixed arc feeds: where its net ends, not where it
    /// starts, though it carries its driver's value.
    fed: bool,
}

/// Points closer than this are one.
const TOUCH: f32 = 2e-4;
/// The cell of the grid points are found by.
const GRID: f32 = 0.25;

pub fn build(
    f: &Fabric,
    model: &DeviceModel,
    routes: &[RouteUse],
    constants: &[ConstantUse],
    logic: &[LogicTile],
    fields: &[FieldUse],
) -> Overlay {
    let mut since = std::time::Instant::now();
    let profile = std::env::var_os("ECP5_OVERLAY_TIMES").is_some();
    let mut phase = |name: &str| {
        if profile {
            eprintln!("overlay {name} {:.6} s", since.elapsed().as_secs_f64());
            since = std::time::Instant::now();
        }
    };
    let threads = thread_count();
    if profile {
        eprintln!("overlay threads {threads}");
    }
    let cols = f.cols as usize;
    let positions = f.rows as usize * cols;
    let inside = |r: i16, c: i16| r >= 0 && c >= 0 && (r as u16) < f.rows && (c as u16) < f.cols;
    let position = |l: Location| l.row as usize * cols + l.col as usize;
    let location = |p: usize| Location {
        row: (p / cols) as i16,
        col: (p % cols) as i16,
    };

    // The design's wires, the nets they make, and the arcs in use.
    let mut values: Vec<Wire> = Vec::new();
    let mut value_of: HashMap<Ident, u32> = HashMap::new();
    let mut value = |w: Wire| -> u32 {
        let key = ident(&w);
        *value_of.entry(key).or_insert_with(|| {
            values.push(w);
            (values.len() - 1) as u32
        })
    };
    let mut nets = Nets(Vec::new());
    let mut sinks: HashSet<u32> = HashSet::new();
    let mut arc_index: HashMap<usize, HashMap<(String, String), u32>> = HashMap::new();
    // (position, arc of its template) → value of the arc's source.
    let mut active: HashMap<(u32, u32), u32> = HashMap::new();
    let mut not_drawn: BTreeMap<String, usize> = BTreeMap::new();
    // The arcs of the plan's routes, of those in use.
    let mut routed: HashSet<(u32, u32)> = HashSet::new();
    let mut drawn = 0;
    for route in routes {
        if let Some(tile) = model.tile_id(&route.tile).map(|t| model.tile(t)) {
            let t = f.template(tile.row as u16, tile.col as u16);
            arc_index
                .entry(Arc::as_ptr(&t.arcs) as usize)
                .or_insert_with(|| {
                    t.arcs
                        .iter()
                        .enumerate()
                        .map(|(j, a)| (a.clone(), j as u32))
                        .collect()
                });
        }
    }
    phase("route_indices");
    struct WalkedRoute {
        wires: Option<(Wire, Wire)>,
        arc: Option<(u32, u32)>,
        missing: Option<String>,
    }
    for part in ranges(routes.len(), threads, |range| {
        routes[range]
            .iter()
            .map(|route| {
                let missing = |reason: &str| WalkedRoute {
                    wires: None,
                    arc: None,
                    missing: Some(reason.into()),
                };
                let Some(tile) = model.tile_id(&route.tile).map(|t| model.tile(t)) else {
                    return missing("tile not in the model");
                };
                let at = Location {
                    row: tile.row,
                    col: tile.col,
                };
                let (Ok(Endpoint::Wire(source)), Ok(Endpoint::Wire(sink))) =
                    (model.wire(at, &route.source), model.wire(at, &route.sink))
                else {
                    return missing("wire outside the die");
                };
                let t = f.template(at.row as u16, at.col as u16);
                let arcs = &arc_index[&(Arc::as_ptr(&t.arcs) as usize)];
                let names = (
                    this_die(&route.source).unwrap_or(&route.source),
                    this_die(&route.sink).unwrap_or(&route.sink),
                );
                let arc = arcs
                    .get(&(names.0.to_string(), names.1.to_string()))
                    .map(|&j| (position(at) as u32, j));
                let reason = arc.is_none().then(|| {
                    let family = |n: &str| -> String {
                        split_prefix(n)
                            .2
                            .chars()
                            .filter(|c| !c.is_ascii_digit())
                            .collect()
                    };
                    format!("{} → {} in {}", family(names.0), family(names.1), tile.kind)
                });
                WalkedRoute {
                    wires: Some((canonical(model, source), canonical(model, sink))),
                    arc,
                    missing: reason,
                }
            })
            .collect::<Vec<_>>()
    }) {
        // Assign value ids and union roots in the plan's original route order.
        for route in part {
            if let Some(reason) = route.missing {
                *not_drawn.entry(reason).or_default() += 1;
            }
            if let Some((source, sink)) = route.wires {
                let (vs, vt) = (value(source), value(sink));
                nets.grow(vs.max(vt) as usize + 1);
                nets.join(vs, vt);
                sinks.insert(vt);
                if let Some(arc) = route.arc {
                    active.insert(arc, vs);
                    routed.insert(arc);
                    drawn += 1;
                }
            }
        }
    }
    phase("routes_nets");
    // The LUTs and flip-flops the plan uses, whether a route takes their
    // outputs or not (a carry's sums go on into the flip-flops inside the
    // slice).
    let mut used_logic = vec![(0_u8, 0_u8); positions];
    let mut slices_at: HashMap<usize, u8> = HashMap::new();
    for t in logic {
        let p = t.row as usize * cols + t.col as usize;
        slices_at.insert(p, t.slices);
        used_logic[p].0 |= t.luts;
        used_logic[p].1 |= t.flip_flops;
    }
    // The settings of the slices the plan reads: their mode, and which
    // input each flip-flop takes (`SLICEA.MODE`, `SLICEA.REG0.SD`: 1 for
    // DI, 0 for M).
    let mut setting: HashMap<(usize, String), String> = HashMap::new();
    for field in fields {
        if !field.name.starts_with("SLICE") || field.candidates.len() != 1 {
            continue;
        }
        if let Some(tile) = model.tile_id(&field.tile).map(|t| model.tile(t)) {
            let p = tile.row as usize * cols + tile.col as usize;
            setting.insert((p, field.name.clone()), field.candidates[0].clone());
        }
    }
    let letter = |s: usize| (b'A' + s as u8) as char;

    let ramw = |p: usize, s: usize| {
        s == 2
            && setting
                .get(&(p, "SLICEC.MODE".to_string()))
                .is_some_and(|m| m == "RAMW")
    };
    let ccu2 = |p: usize, s: usize| {
        setting
            .get(&(p, format!("SLICE{}.MODE", letter(s))))
            .is_some_and(|m| m == "CCU2")
    };
    // Whether half k of a CCU2 slice takes the carry in (`INJECT1_k` NO);
    // with YES it starts a chain and ignores it.
    let takes_carry = |p: usize, s: usize, k: u32| {
        setting
            .get(&(p, format!("SLICE{}.CCU2.INJECT1_{k}", letter(s))))
            .is_some_and(|i| i == "NO")
    };
    // The slice before slice s in a carry chain (slice D of the tile to the
    // west before slice A), and the one after it.
    let before = |p: usize, s: usize| match s {
        0 => (!p.is_multiple_of(cols)).then(|| (p - 1, 3)),
        _ => Some((p, s - 1)),
    };
    let after = |p: usize, s: usize| match s {
        3 => (p % cols + 1 < cols).then(|| (p + 1, 0)),
        _ => Some((p, s + 1)),
    };
    // A carry runs between two CCU2 slices whose second takes it.
    let chained = |(p, s): (usize, usize), (q, t): (usize, usize)| {
        ccu2(p, s) && ccu2(q, t) && takes_carry(q, t, 0)
    };
    let takes_m = |p: usize, n: u32| {
        setting
            .get(&(
                p,
                format!("SLICE{}.REG{}.SD", letter(n as usize / 2), n % 2),
            ))
            .is_some_and(|sd| sd == "0")
    };
    // The carry into, through and out of slice s, as its ports are named.
    let carry_in = |s: usize| match s {
        0 => "FCI_SLICE".to_string(),
        _ => format!("FCI{}_SLICE", letter(s)),
    };
    let carry_out = |s: usize| match s {
        3 => "FCO_SLICE".to_string(),
        _ => format!("FCO{}_SLICE", letter(s)),
    };
    // The ports inside the slices the plan uses that no route may take:
    // the carry of a CCU2 slice, the LUT outputs into an F5 mux in use, and
    // the output of every LUT in use, which may go only into the slice (a
    // wide mux, the carry), so that each LUT has a value to show.
    let mut inner: Vec<Wire> = Vec::new();
    for t in logic {
        let at = Location {
            row: t.row as i16,
            col: t.col as i16,
        };
        let p = position(at);
        for s in (0..4).filter(|s| t.slices >> s & 1 == 1) {
            let wire = |name: String| Wire { location: at, name };
            if ccu2(p, s) {
                inner.extend([
                    wire(carry_in(s)),
                    wire(format!("FCI{}1_SLICE", letter(s))),
                    wire(carry_out(s)),
                ]);
            }
            if value_of.contains_key(&ident(&wire(format!("F5{}_SLICE", letter(s))))) {
                inner.extend([
                    wire(format!("F{}_SLICE", 2 * s)),
                    wire(format!("F{}_SLICE", 2 * s + 1)),
                ]);
            }
        }
        for n in (0..8).filter(|n| t.luts >> n & 1 == 1) {
            if ramw(p, n / 2) {
                for (name, _) in ramw_outputs(n % 2) {
                    inner.push(Wire { location: at, name });
                }
            } else {
                inner.push(Wire {
                    location: at,
                    name: format!("F{n}_SLICE"),
                });
            }
        }
    }
    for w in inner {
        if let Entry::Vacant(e) = value_of.entry(ident(&w)) {
            e.insert(values.len() as u32);
            values.push(w);
        }
    }
    phase("inner_values");
    // The wires a CIB ties to a constant.
    let tied_values: HashSet<u32> = constants
        .iter()
        .filter_map(|c| {
            value_of
                .get(&ident(&canonical(model, c.wire.clone())))
                .copied()
        })
        .collect();
    // A fixed arc is always joined: its two wires carry one value. The plan
    // routes up to a site's routing wire and leaves the rest to the site
    // (`A0` → `A0_SLICE`, `Q0_SLICE` → `Q0`), so a value is carried on
    // along fixed arcs: forwards, and backwards to a wire's only fixed
    // driver when no route drives it. The CIB's test access (`…_CIBTEST`)
    // is inactive after loading, as the plan has it, so nothing comes out
    // of it; but each J wire the CIB passes on (to a pad, an EBR, a DSP)
    // has its pin at the CIB, and goes there: a J wire in use ends at it
    // too, and one the CIB ties to a constant starts at it, where the CIB
    // makes the constant.
    type Fixed = (Vec<u32>, HashMap<String, usize>, Vec<u32>);
    let mut fixed_of: HashMap<usize, Fixed> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut queued = vec![false; positions];
    let near = |l: Location, queue: &mut VecDeque<usize>, queued: &mut [bool]| {
        for (dr, dc) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (r, c) = (l.row + dr, l.col + dc);
            if inside(r, c) {
                let p = r as usize * cols + c as usize;
                if !queued[p] {
                    queued[p] = true;
                    queue.push_back(p);
                }
            }
        }
    };
    let mut initial: Vec<_> = value_of.keys().collect();
    initial.sort_unstable();
    for key in initial {
        if let Ident::Wire(w) = key {
            near(w.location, &mut queue, &mut queued);
        }
    }
    let mut carried = 0;
    // The wires a fixed arc feeds (`A0` → `A0_SLICE`).
    let mut downstream: HashSet<Ident> = HashSet::new();
    for t in &f.templates {
        fixed_of
            .entry(Arc::as_ptr(&t.arcs) as usize)
            .or_insert_with(|| {
                let to_cib: Vec<u32> = (0..t.arcs.len())
                    .filter(|&j| {
                        t.fixed[j]
                            && t.arcs[j].1.ends_with("_CIBTEST")
                            && !t.arcs[j].0.ends_with("_CIBTEST")
                    })
                    .map(|j| j as u32)
                    .collect();
                let fixed: Vec<u32> = (0..t.arcs.len())
                    .filter(|&j| {
                        t.fixed[j]
                            && !t.arcs[j].0.ends_with("_CIBTEST")
                            && !t.arcs[j].1.ends_with("_CIBTEST")
                    })
                    .map(|j| j as u32)
                    .collect();
                let mut drivers: HashMap<String, usize> = HashMap::new();
                for &j in &fixed {
                    *drivers.entry(t.arcs[j as usize].1.clone()).or_default() += 1;
                }
                (fixed, drivers, to_cib)
            });
    }
    struct FixedArc {
        arc: u32,
        from: Ident,
        to: Ident,
        from_at: Location,
        to_at: Location,
        cib: bool,
        only_driver: bool,
    }
    let mut fixed_at: Vec<Option<Vec<FixedArc>>> = (0..positions).map(|_| None).collect();
    while !queue.is_empty() {
        // Resolve each position once, independently of the propagation wave.
        // Propagation itself keeps the sequential queue and insertion order.
        let pending: Vec<_> = queue
            .iter()
            .copied()
            .filter(|&p| fixed_at[p].is_none())
            .collect();
        for part in ranges(pending.len(), threads, |range| {
            pending[range]
                .iter()
                .map(|&p| {
                    let t = &f.templates[f.template_at[p] as usize];
                    let (fixed, drivers, to_cib) = &fixed_of[&(Arc::as_ptr(&t.arcs) as usize)];
                    let mut arcs = Vec::new();
                    for (j, cib) in to_cib
                        .iter()
                        .map(|&j| (j, true))
                        .chain(fixed.iter().map(|&j| (j, false)))
                    {
                        let (a, b) = &t.arcs[j as usize];
                        if b.starts_with('=') {
                            continue;
                        }
                        let (Ok(Endpoint::Wire(wa)), Ok(Endpoint::Wire(wb))) =
                            (model.wire(location(p), a), model.wire(location(p), b))
                        else {
                            continue;
                        };
                        let cib_at = wb.location;
                        let (wa, wb) = (canonical(model, wa), canonical(model, wb));
                        arcs.push(FixedArc {
                            arc: j,
                            from: ident(&wa),
                            to: ident(&wb),
                            from_at: wa.location,
                            to_at: if cib { cib_at } else { wb.location },
                            cib,
                            only_driver: !cib && drivers[b] == 1,
                        });
                    }
                    (p, arcs)
                })
                .collect::<Vec<_>>()
        }) {
            for (p, arcs) in part {
                fixed_at[p] = Some(arcs);
            }
        }
        for _ in 0..queue.len() {
            let p = queue.pop_front().unwrap();
            queued[p] = false;
            for arc in fixed_at[p].as_ref().unwrap() {
                let (ia, ib) = (&arc.from, &arc.to);
                if arc.cib {
                    let Some(v) = value_of.get(ia).copied() else {
                        continue;
                    };
                    active.entry((p as u32, arc.arc)).or_insert(v);
                    if let Entry::Vacant(e) = value_of.entry(ib.clone()) {
                        if !tied_values.contains(&v) {
                            downstream.insert(ib.clone());
                        }
                        e.insert(v);
                        carried += 1;
                        near(arc.to_at, &mut queue, &mut queued);
                    }
                    continue;
                }
                let (va, vb) = (value_of.get(ia).copied(), value_of.get(ib).copied());
                let (v, new, forward) = match (va, vb) {
                    (Some(v), Some(_)) => (v, None, false),
                    (Some(v), None) => (v, Some((ib, arc.to_at)), true),
                    (None, Some(v)) if arc.only_driver && !sinks.contains(&v) => {
                        (v, Some((ia, arc.from_at)), false)
                    }
                    _ => continue,
                };
                active.entry((p as u32, arc.arc)).or_insert(v);
                if let Some((i, at)) = new {
                    if forward {
                        downstream.insert(i.clone());
                    }
                    value_of.insert(i.clone(), v);
                    carried += 1;
                    near(at, &mut queue, &mut queued);
                }
            }
        }
    }
    phase("fixed_arcs");
    let lookup = |w: &Wire| value_of.get(&ident(w)).copied();
    let value_at = |p: usize, name: &str| -> Option<u32> {
        match model.wire(location(p), name) {
            Ok(Endpoint::Wire(w)) => lookup(&canonical(model, w)),
            _ => None,
        }
    };
    // A link of the clock tree between two positions that name one
    // conductor carries it when the plan uses it.
    for l in &f.links {
        let p = l.from as usize;
        let (r, c) = ((p / cols) as u16, (p % cols) as u16);
        let (name, other) = &f.template(r, c).arcs[l.arc as usize];
        if other.starts_with('=') {
            if let Some(v) = value_at(p, name) {
                active.insert((l.from, l.arc), v);
            }
        }
    }

    phase("clock_links");
    // The wires in use where the fabric draws them.
    let indices: Vec<_> = ranges(f.templates.len(), threads, |range| {
        range
            .map(|i| WireIndex::new(&f.templates[i]))
            .collect::<Vec<_>>()
    })
    .into_iter()
    .flatten()
    .collect();
    // (position, wire of its template) → value.
    let mut lit: HashMap<(u32, u32), u32> = HashMap::new();
    for (key, &v) in &value_of {
        match key {
            Ident::Track(id, r0, c0) => {
                let s = step(id);
                for tap in 0..=id.span {
                    let (r, c) = (r0 + tap as i16 * s.0, c0 + tap as i16 * s.1);
                    if !inside(r, c) {
                        continue;
                    }
                    let p = r as usize * cols + c as usize;
                    if let Some(&k) = indices[f.template_at[p] as usize].tracks.get(&(*id, tap)) {
                        lit.insert((p as u32, k), v);
                    }
                }
            }
            Ident::Wire(w) => {
                if !inside(w.location.row, w.location.col) {
                    continue;
                }
                let p = position(w.location);
                for &k in indices[f.template_at[p] as usize]
                    .names
                    .get(&w.name)
                    .into_iter()
                    .flatten()
                {
                    lit.insert((p as u32, k), v);
                }
            }
        }
    }
    for entries in ranges(positions, threads, |range| {
        let mut entries = Vec::new();
        for p in range {
            for (k, name) in &indices[f.template_at[p] as usize].elsewhere {
                if let Some(v) = value_at(p, name) {
                    entries.push(((p as u32, *k), v));
                }
            }
        }
        entries
    }) {
        lit.extend(entries);
    }
    phase("value_of_positions");
    // The connections inside the slices (`~FROM>TO`, src/fabric.rs) the
    // plan uses, each with the value of the port it carries: the carry
    // through a CCU2 slice, in from the CCU2 slice before it and out to the
    // one after it where those take it, and between its halves where the
    // second takes it; a LUT into its
    // slice's F5 mux in use; M into a
    // flip-flop that takes M; the clock, reset and enable into a flip-flop
    // in use, where they are routed. Where one leaves a box that its pin
    // enters (a control, M), it runs on from that pin's end.
    let lut_used = |p: usize, n: u32| used_logic[p].0 >> n & 1 == 1;
    let ff_used = |p: usize, n: u32| used_logic[p].1 >> n & 1 == 1;
    let inside_slice = |p: usize, name: &str| -> Option<(u32, Option<String>)> {
        let (from, to) = name.strip_prefix('~')?.split_once('>')?;
        let number = |x: &str, prefix: &str| x.strip_prefix(prefix)?.parse::<u32>().ok();
        let routed = |name: String| value_at(p, &name);
        match (from, to) {
            ("FCI", _) => {
                let n = number(to, "LUT")?;
                let s = n as usize / 2;
                before(p, s)
                    .is_some_and(|b| chained(b, (p, s)))
                    .then(|| routed(carry_in(s)))?
                    .map(|v| (v, None))
            }
            (_, "FCO") => {
                let s = number(from, "LUT")? as usize / 2;
                after(p, s)
                    .is_some_and(|a| chained((p, s), a))
                    .then(|| routed(carry_out(s)))?
                    .map(|v| (v, None))
            }
            (_, "MUX") => {
                let n = number(from, "LUT")?;
                let slice = letter(n as usize / 2);
                (lut_used(p, n) && routed(format!("F5{slice}_SLICE")).is_some())
                    .then(|| routed(format!("F{n}_SLICE")))?
                    .map(|v| (v, None))
            }
            ("MUX", _) => {
                let n = number(to, "FF")?;
                let pin = format!("M{n}_SLICE");
                (ff_used(p, n) && takes_m(p, n))
                    .then(|| routed(pin.clone()))?
                    .map(|v| (v, Some(pin)))
            }
            ("CLK" | "LSR" | "CE", _) => {
                let n = number(to, "FF")?;
                let pin = format!("{from}{}_SLICE", n / 2);
                ff_used(p, n)
                    .then(|| routed(pin.clone()))?
                    .map(|v| (v, Some(pin)))
            }
            _ => {
                let (a, b) = (number(from, "LUT")?, number(to, "LUT")?);
                let s = a as usize / 2;
                (b == a + 1 && ccu2(p, s) && takes_carry(p, s, 1))
                    .then(|| routed(format!("FCI{}1_SLICE", letter(s))))?
                    .map(|v| (v, None))
            }
        }
    };
    // Where a lit one goes (in die units), and from where on a pin's end
    // one that runs on through a box goes on: (pin's end, its start, value).
    let mut internal_to: Vec<[f32; 2]> = Vec::new();
    let mut through: Vec<([f32; 2], [f32; 2], u32)> = Vec::new();
    let mut slice_positions: Vec<_> = slices_at.keys().copied().collect();
    slice_positions.sort_unstable();
    for (entries, ends, connections) in ranges(slice_positions.len(), threads, |range| {
        let mut entries = Vec::new();
        let mut internal_to = Vec::new();
        let mut through = Vec::new();
        for &p in &slice_positions[range] {
            let (r, c) = ((p / cols) as u16, (p % cols) as u16);
            let base = [c as f32 * TILE_W, f.row_top[r as usize]];
            let die = |q: [f32; 2]| [base[0] + q[0], base[1] + q[1]];
            let t = f.template(r, c);
            for &(k, from, to) in &t.internals {
                let WireKey::Name(name) = &t.wires[k as usize] else {
                    continue;
                };
                let Some((v, pin)) = inside_slice(p, name) else {
                    continue;
                };
                entries.push(((p as u32, k), v));
                internal_to.push(die(to));
                if let Some(end) = pin.and_then(|pin| pin_end(t, &pin)) {
                    through.push((die(end), die(from), v));
                }
            }
        }

        (entries, internal_to, through)
    }) {
        lit.extend(entries);
        internal_to.extend(ends);
        through.extend(connections);
    }

    phase("slices");
    // Pieces and dots of the templates, then links and DSP blocks.
    let mut raw: Vec<Raw> = Vec::new();
    let mut dots: Vec<([f32; 2], u32)> = Vec::new();
    let mut sites: Vec<(usize, Site)> = Vec::new();
    let mut shown: HashSet<(u32, u32)> = HashSet::new();
    // Where pins end on their sites, where a driver's value starts.
    let key = |p: [f32; 2]| ((p[0] / TOUCH).round() as i32, (p[1] / TOUCH).round() as i32);
    let mut site_ends: HashSet<(i32, i32)> = HashSet::new();
    // The pins of the parts of slices the plan does not use: no net ends
    // there, so no way runs to a part that does nothing.
    let mut blocked: HashSet<(i32, i32)> = HashSet::new();
    const EDGE: f32 = 1e-3;
    let on_edge = |q: [f32; 2], min: [f32; 2], max: [f32; 2]| {
        let inside = q[0] >= min[0] - EDGE
            && q[0] <= max[0] + EDGE
            && q[1] >= min[1] - EDGE
            && q[1] <= max[1] + EDGE;
        inside
            && ((q[0] - min[0]).abs() <= EDGE
                || (q[0] - max[0]).abs() <= EDGE
                || (q[1] - min[1]).abs() <= EDGE
                || (q[1] - max[1]).abs() <= EDGE)
    };
    let mut lit_at = vec![Vec::new(); positions];
    let mut active_at = vec![Vec::new(); positions];
    for (&(p, k), &v) in &lit {
        lit_at[p as usize].push((k, v));
    }
    for (&(p, j), &v) in &active {
        active_at[p as usize].push((j, v));
    }
    let mut busy: Vec<u32> = lit.keys().chain(active.keys()).map(|k| k.0).collect();
    busy.sort_unstable();
    busy.dedup();
    for (pieces, points, arcs, ends, unused) in ranges(busy.len(), threads, |range| {
        let mut raw = Vec::new();
        let mut dots = Vec::new();
        let mut shown = HashSet::new();
        let mut site_ends = HashSet::new();
        let mut blocked = HashSet::new();
        for &p in &busy[range] {
            let p = p as usize;
            let (r, c) = ((p / cols) as u16, (p % cols) as u16);
            let base = [c as f32 * TILE_W, f.row_top[r as usize]];
            let die = |q: [f32; 2]| [base[0] + q[0], base[1] + q[1]];
            let t = f.template(r, c);
            site_ends.extend(t.site_ends.iter().map(|q| key(die(*q))));
            let index = &indices[f.template_at[p] as usize];
            let mut segments = Vec::new();
            let mut points = Vec::new();
            for (owner, value) in lit_at[p].iter().map(|&(k, v)| (k as usize, v)).chain(
                active_at[p]
                    .iter()
                    .map(|&(j, v)| (t.wires.len() + j as usize, v)),
            ) {
                segments.extend(index.segments[owner].iter().map(|&i| (i, value)));
                points.extend(index.dots[owner].iter().map(|&i| (i, value)));
            }
            segments.sort_unstable_by_key(|x| x.0);
            points.sort_unstable_by_key(|x| x.0);
            for &(j, _) in &active_at[p] {
                if index.drawn_arcs[j as usize] {
                    shown.insert((p as u32, j));
                }
            }
            let fed: HashSet<u32> = index
                .pins
                .iter()
                .filter_map(|(k, _)| {
                    let WireKey::Name(name) = &t.wires[*k as usize] else {
                        return None;
                    };
                    if !lit.contains_key(&(p as u32, *k)) {
                        return None;
                    }
                    match model.wire(location(p), name) {
                        Ok(Endpoint::Wire(w)) => downstream
                            .contains(&ident(&canonical(model, w)))
                            .then_some(*k),
                        _ => None,
                    }
                })
                .collect();
            for (i, v) in segments {
                let seg = &t.segments[i as usize];
                raw.push(Raw {
                    a: die(seg.0),
                    b: die(seg.1),
                    value: v,
                    kind: seg.2.into(),
                    fed: seg.2 == Part::Pin
                        && matches!(&t.owners[i as usize], Owner::Wire(k) if fed.contains(k)),
                });
            }
            for (i, v) in points {
                dots.push((die(t.dots[i as usize]), v));
            }
            // A pin of a slice ends where the plan uses it: a LUT's inputs and
            // output at a LUT in use, a flip-flop's at one in use, M at its
            // flip-flop or at the wide mux it selects (F5 for M0, FX for M1 of
            // a slice), the FX mux's inputs at a FX mux in use (its output
            // routed on), the controls, the carry and the LUTRAM's writes at a
            // slice in use. A pin elsewhere is where no net ends, so no way
            // runs to a part that does nothing.
            let slices = slices_at.get(&p).copied();
            let lut = |n: u32| used_logic[p].0 >> n & 1 == 1;
            let ff = |n: u32| used_logic[p].1 >> n & 1 == 1;
            let routed = |name: String| value_at(p, &name).is_some();
            let pin_used = |name: &str| -> bool {
                let Some(base) = name.strip_suffix("_SLICE") else {
                    return true;
                };
                let Some(slices) = slices else {
                    return false;
                };
                let digits = base.find(|c: char| c.is_ascii_digit());
                let number = digits.and_then(|d| base[d..].parse::<u32>().ok());
                let letter = |l: &str| l.bytes().next().filter(|b| (b'A'..=b'D').contains(b));
                match (digits.map(|d| &base[..d]), number) {
                    (Some("A" | "B" | "C" | "D" | "F"), Some(n)) if n < 8 => lut(n),
                    (Some("Q"), Some(n)) if n < 8 => ff(n),
                    (Some("DI"), Some(n)) if n < 8 => ff(n) && !takes_m(p, n),
                    (Some("M"), Some(n)) if n < 8 => {
                        let slice = (b'A' + (n / 2) as u8) as char;
                        let mux = if n % 2 == 0 { "F5" } else { "FX" };
                        (ff(n) && takes_m(p, n)) || routed(format!("{mux}{slice}_SLICE"))
                    }
                    _ => match base
                        .strip_prefix("FXA")
                        .or_else(|| base.strip_prefix("FXB"))
                        .and_then(letter)
                    {
                        Some(l) => routed(format!("FX{}_SLICE", l as char)),
                        None => slices != 0,
                    },
                }
            };
            for (k, ends) in &index.pins {
                let WireKey::Name(name) = &t.wires[*k as usize] else {
                    continue;
                };
                if !pin_used(name) {
                    blocked.extend(ends.iter().map(|e| key(die(*e))));
                }
            }
        }
        (raw, dots, shown, site_ends, blocked)
    }) {
        raw.extend(pieces);
        dots.extend(points);
        shown.extend(arcs);
        site_ends.extend(ends);
        blocked.extend(unused);
    }

    phase("templates");
    for l in &f.links {
        if let Some(&v) = active.get(&(l.from, l.arc)) {
            shown.insert((l.from, l.arc));
            polyline(&mut raw, &l.points, v, Kind::Link);
        }
    }
    let place = |n: &(usize, String)| value_at(n.0, &n.1);
    for m in &f.macros {
        let pieces = m
            .wire_pieces
            .iter()
            .zip(&m.wires)
            .chain(m.inner_pieces.iter().zip(&m.inner));
        for (piece, points) in pieces {
            let v = match piece {
                MacroPiece::Trunk(a) | MacroPiece::Stub(a) => place(a),
                MacroPiece::Feed(a, b) | MacroPiece::Join(a, b) => {
                    place(a).filter(|_| place(b).is_some())
                }
            };
            if let Some(v) = v {
                polyline(&mut raw, points, v, Kind::Block);
            }
        }
    }

    // A wire of a DSP block ends at a part's port: where its net is used.
    for r in raw.iter().filter(|r| r.kind == Kind::Block) {
        for q in [r.a, r.b] {
            if f.macros.iter().any(|m| {
                m.parts.iter().any(|b| {
                    (q[0] >= b.min[0] - EDGE && q[0] <= b.max[0] + EDGE)
                        && ((q[1] - b.min[1]).abs() <= EDGE || (q[1] - b.max[1]).abs() <= EDGE)
                })
            }) {
                site_ends.insert(key(q));
            }
        }
    }
    // A clock buffer (DCC) passes its input on: the clock from its pad and
    // the clock it drives over the die are one net to follow.
    nets.grow(values.len());
    for (v, w) in values.iter().enumerate() {
        if !(w.name.contains("DCC") && w.name.contains("CLKI")) {
            continue;
        }
        let out = Wire {
            location: w.location,
            name: w.name.replace("JCLKI", "CLKO").replace("CLKI", "CLKO"),
        };
        if let Some(&o) = value_of.get(&ident(&out)) {
            nets.join(v as u32, o);
        }
    }
    phase("links_macros_nets");
    // The points where pieces of a net meet: their ends and the dots.
    let net: Vec<u32> = (0..values.len() as u32).map(|v| nets.find(v)).collect();
    let tied: HashMap<u32, bool> = constants
        .iter()
        .filter_map(|c| {
            let v = value_of.get(&ident(&canonical(model, c.wire.clone())))?;
            Some((net[*v as usize], c.value == Logic::One))
        })
        .collect();
    // Nets share no graph edges. Keep raw/dot order within each net, so
    // local node ids preserve every Dijkstra tie from the global graph.
    let mut raw_by_net = vec![Vec::new(); values.len()];
    let mut dots_by_net = vec![Vec::new(); values.len()];
    let mut through_by_net = vec![Vec::new(); values.len()];
    for (i, r) in raw.iter().enumerate() {
        raw_by_net[net[r.value as usize] as usize].push((i, r));
    }
    for (i, &(at, v)) in dots.iter().enumerate() {
        dots_by_net[net[v as usize] as usize].push((i, (at, v)));
    }
    for &(a, b, v) in &through {
        through_by_net[net[v as usize] as usize].push((a, b, v));
    }
    let busy_nets: Vec<_> = (0..values.len())
        .filter(|&n| !raw_by_net[n].is_empty() || !dots_by_net[n].is_empty())
        .collect();
    let internal_ends: HashSet<_> = internal_to.iter().map(|q| key(*q)).collect();
    phase("group_nets");
    let drawings = ranges(busy_nets.len(), threads, |range| {
        busy_nets[range]
            .iter()
            .map(|&n| {
                draw_net(
                    f,
                    &net,
                    &raw_by_net[n],
                    &dots_by_net[n],
                    &through_by_net[n],
                    raw.len(),
                    &sinks,
                    &site_ends,
                    &internal_ends,
                    &blocked,
                )
            })
            .collect::<Vec<_>>()
    });
    let mut out = Vec::new();
    phase("draw_nets");
    let mut drawn_dots = Vec::new();
    let mut loose = Vec::new();
    let mut drivers = HashMap::new();
    let mut fed_at = HashMap::new();
    let mut longest = 0.0_f32;
    let mut undriven = 0;
    let mut unfed = 0;
    for drawing in drawings.into_iter().flatten() {
        out.extend(drawing.pieces);
        drawn_dots.extend(drawing.dots);
        loose.extend(drawing.loose);
        drivers.extend(drawing.drivers);
        fed_at.extend(drawing.fed_at);
        longest = longest.max(drawing.longest);
        undriven += drawing.undriven;
        unfed += drawing.unfed;
    }
    phase("merge_nets");
    // Restore the original global traversal order before stable grouping.
    out.sort_by_key(|x| x.0);
    drawn_dots.sort_unstable_by_key(|x| x.0);
    loose.sort_unstable_by_key(|x| x.0);
    let out = out.into_iter().map(|x| x.1).collect();
    let loose = loose.into_iter().map(|x| x.1).collect();
    phase("net_graphs_pieces");
    // Arcs in use that the fabric left out, by their wires' families.
    let hidden: Vec<&(u32, u32)> = routed.iter().filter(|k| !shown.contains(k)).collect();
    let mut hidden_by: BTreeMap<String, usize> = BTreeMap::new();
    for &&(p, j) in &hidden {
        let (r, c) = ((p as usize / cols) as u16, (p as usize % cols) as u16);
        let (a, b) = &f.template(r, c).arcs[j as usize];
        let family = |n: &str| -> String { n.chars().filter(|c| !c.is_ascii_digit()).collect() };
        *hidden_by
            .entry(format!(
                "{} → {} in {}",
                family(a),
                family(b),
                f.types_at(r, c).join("+")
            ))
            .or_default() += 1;
    }
    let row_at = |y: f32| f.row_at(y) as usize;
    let at_of = |p: [f32; 2]| {
        let c = ((p[0] / TILE_W).floor().max(0.0) as usize).min(cols - 1);
        row_at(p[1]) * cols + c
    };
    let dots = drawn_dots
        .into_iter()
        .map(|(_, d)| (at_of(d.at), d))
        .collect();
    let (pieces, pieces_at) = by_position(out, positions);
    phase("pieces_by_position");
    // The sites the ways of nets enter or leave, and only those, are in
    // use: a pin of a route ends on the edge of a site of its tile (a LUT,
    // a flip-flop, a part of a slice, an EBR, a pad …), a wire of a DSP
    // block on a part's. A LUT or a flip-flop shows the value of its
    // output where a route takes it.
    let mut lit_logic: BTreeSet<(usize, usize)> = BTreeSet::new();
    // (position, 0 for a tile's site or 1 + its block for a DSP part, its
    // index) → the site.
    let mut used: BTreeMap<(usize, usize, usize), UsedBox> = BTreeMap::new();
    for (p, &(a, b)) in pieces_at.iter().enumerate() {
        let list = &pieces[a as usize..b as usize];
        if list.is_empty() {
            continue;
        }
        let (r, c) = ((p / cols) as u16, (p % cols) as u16);
        let base = [c as f32 * TILE_W, f.row_top[r as usize]];
        let t = f.template(r, c);
        for piece in list
            .iter()
            .filter(|q| q.route && matches!(q.kind, Kind::Pin | Kind::Internal))
        {
            for q in [piece.a, piece.b] {
                let q = [q[0] - base[0], q[1] - base[1]];
                for (k, b) in t.boxes.iter().enumerate() {
                    if b.kind == BoxKind::Slice || !on_edge(q, b.min, b.max) {
                        continue;
                    }
                    if matches!(b.kind, BoxKind::Lut | BoxKind::FlipFlop) {
                        lit_logic.insert((p, k));
                        continue;
                    }
                    {
                        let die = |x: [f32; 2]| [base[0] + x[0], base[1] + x[1]];
                        used.entry((p, 0, k))
                            .or_insert_with(|| UsedBox {
                                min: die(b.min),
                                max: die(b.max),
                                values: Vec::new(),
                                label: b.label.clone(),
                                site: b.kind == BoxKind::Site,
                            })
                            .values
                            .push(piece.value);
                    }
                }
            }
        }
    }
    for (m, block) in f.macros.iter().enumerate() {
        for piece in pieces.iter().filter(|q| q.route && q.kind == Kind::Block) {
            for q in [piece.a, piece.b] {
                for (k, b) in block.parts.iter().enumerate() {
                    if on_edge(q, b.min, b.max) {
                        let c = ((b.min[0] / TILE_W) as usize).min(cols - 1);
                        let p = block.row as usize * cols + c;
                        used.entry((p, 1 + m, k))
                            .or_insert_with(|| UsedBox {
                                min: b.min,
                                max: b.max,
                                values: Vec::new(),
                                label: b.label.clone(),
                                site: false,
                            })
                            .values
                            .push(piece.value);
                    }
                }
            }
        }
    }
    for &(p, k) in &lit_logic {
        let (r, c) = ((p / cols) as u16, (p % cols) as u16);
        let base = [c as f32 * TILE_W, f.row_top[r as usize]];
        let die = |x: [f32; 2]| [base[0] + x[0], base[1] + x[1]];
        let b = &f.template(r, c).boxes[k];
        let output = match b.kind {
            BoxKind::Lut => b.label.strip_prefix("LUT").map(|n| format!("F{n}")),
            _ => b.label.strip_prefix("FF").map(|n| format!("Q{n}")),
        };
        // The carry out of a CCU2 half: into the slice's second half from
        // its first, out of the slice from its second. A half that only
        // makes the carry (its F not taken) shows that. A RAMW half has no
        // F: it passes the write address or data on.
        let n = b
            .label
            .strip_prefix("LUT")
            .and_then(|n| n.parse::<usize>().ok());
        let also: Vec<u32> = match n {
            Some(n) if ccu2(p, n / 2) => value_at(
                p,
                &if n % 2 == 0 {
                    format!("FCI{}1_SLICE", letter(n / 2))
                } else {
                    carry_out(n / 2)
                },
            )
            .into_iter()
            .collect(),
            Some(n) if ramw(p, n / 2) => ramw_outputs(n % 2)
                .iter()
                .filter_map(|(name, _)| value_at(p, name))
                .collect(),
            _ => Vec::new(),
        };
        let ramw_half = n.is_some_and(|n| ramw(p, n / 2));
        // Its own output, as the slice names it: the tile's wire of the same
        // name may carry another (`F1` the FX mux's, `F0` the F5 mux's).
        let value = if ramw_half {
            None
        } else {
            output
                .as_ref()
                .and_then(|o| value_at(p, &format!("{o}_SLICE")).or_else(|| value_at(p, o)))
        };
        sites.push((
            p,
            Site {
                min: die(b.min),
                max: die(b.max),
                value: value.unwrap_or(u32::MAX),
                also,
                label: b.label.clone(),
            },
        ));
    }
    let boxes: Vec<(usize, UsedBox)> = used
        .into_iter()
        .map(|((p, _, _), mut b)| {
            b.values.sort_unstable();
            b.values.dedup();
            (p, b)
        })
        .collect();
    let (boxes, boxes_at) = by_position(boxes, positions);
    phase("boxes_sites");
    // Stable counting by net avoids sorting millions of piece indices.
    let mut counts = vec![0_u32; net.len()];
    for piece in &pieces {
        counts[net[piece.value as usize] as usize] += 1;
    }
    let mut nets_at = HashMap::new();
    let mut offsets = vec![0_u32; net.len()];
    let mut end = 0_u32;
    for (n, &count) in counts.iter().enumerate() {
        if count != 0 {
            offsets[n] = end;
            nets_at.insert(n as u32, (end, end + count));
            end += count;
        }
    }
    let mut by_net = vec![0_u32; pieces.len()];
    for (k, piece) in pieces.iter().enumerate() {
        let offset = &mut offsets[net[piece.value as usize] as usize];
        by_net[*offset as usize] = k as u32;
        *offset += 1;
    }
    let (dots, dots_at) = by_position(dots, positions);
    let (sites, sites_at) = by_position(sites, positions);
    phase("indices_by_position");
    Overlay {
        values,
        nets: net,
        pieces,
        dots,
        sites,
        boxes,
        pieces_at,
        boxes_at,
        dots_at,
        sites_at,
        longest,
        routes: routes.len(),
        drawn,
        not_drawn,
        hidden: hidden.len(),
        hidden_by,
        undriven,
        carried,
        unfed,
        loose,
        nets_at,
        by_net,
        drivers,
        fed_at,
        tied,
        index: value_of,
    }
}

struct NetDrawing {
    pieces: Vec<(usize, (usize, Piece))>,
    dots: Vec<(usize, Dot)>,
    loose: Vec<(usize, (u32, [f32; 2]))>,
    drivers: HashMap<u32, Vec<[f32; 2]>>,
    fed_at: HashMap<u32, Vec<[f32; 2]>>,
    longest: f32,
    undriven: usize,
    unfed: usize,
}

#[allow(clippy::too_many_arguments)]
fn draw_net(
    f: &Fabric,
    net: &[u32],
    raw: &[(usize, &Raw)],
    dots: &[(usize, ([f32; 2], u32))],
    through: &[([f32; 2], [f32; 2], u32)],
    raw_count: usize,
    sinks: &HashSet<u32>,
    site_ends: &HashSet<(i32, i32)>,
    internal_ends: &HashSet<(i32, i32)>,
    blocked: &HashSet<(i32, i32)>,
) -> NetDrawing {
    let cols = f.cols as usize;
    let key = |p: [f32; 2]| ((p[0] / TOUCH).round() as i32, (p[1] / TOUCH).round() as i32);
    let mut nodes: Vec<([f32; 2], u32)> = Vec::new();
    let mut node_of: HashMap<(i32, i32, u32), u32> = HashMap::new();
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    let cell = |p: [f32; 2]| ((p[0] / GRID).floor() as i32, (p[1] / GRID).floor() as i32);
    let mut order_of = Vec::new();
    let mut node = |p: [f32; 2], n: u32, order: usize| -> u32 {
        let key = (
            (p[0] / TOUCH).round() as i32,
            (p[1] / TOUCH).round() as i32,
            n,
        );
        *node_of.entry(key).or_insert_with(|| {
            nodes.push((p, n));
            order_of.push(order);
            let id = (nodes.len() - 1) as u32;
            grid.entry(cell(p)).or_default().push(id);
            id
        })
    };
    // The ends of pins a fixed arc feeds.
    let mut fed_pin: HashSet<u32> = HashSet::new();
    for &(order, r) in raw {
        let n = net[r.value as usize];
        let (a, b) = (node(r.a, n, order * 2), node(r.b, n, order * 2 + 1));
        if r.fed {
            fed_pin.extend([a, b]);
        }
    }
    let dot_nodes: Vec<u32> = dots
        .iter()
        .map(|&(order, (p, v))| node(p, net[v as usize], raw_count * 2 + order))
        .collect();
    // Each piece split at every point of its net on it.
    struct Edge {
        to: u32,
        length: f32,
        split: u32,
    }
    let mut adjacent: Vec<Vec<Edge>> = (0..nodes.len()).map(|_| Vec::new()).collect();
    // (from node, to node, value, kind) for every split piece.
    let mut split: Vec<(u32, u32, u32, Kind)> = Vec::new();
    let mut split_raw = Vec::new();
    for &(order, r) in raw {
        let n = net[r.value as usize];
        let (lo, hi) = (
            [r.a[0].min(r.b[0]) - TOUCH, r.a[1].min(r.b[1]) - TOUCH],
            [r.a[0].max(r.b[0]) + TOUCH, r.a[1].max(r.b[1]) + TOUCH],
        );
        let (c0, c1) = (cell(lo), cell(hi));
        let horizontal = (r.a[1] - r.b[1]).abs() <= (r.a[0] - r.b[0]).abs();
        let mut on: Vec<(f32, u32)> = Vec::new();
        for gx in c0.0..=c1.0 {
            for gy in c0.1..=c1.1 {
                for &id in grid.get(&(gx, gy)).into_iter().flatten() {
                    let (q, qn) = nodes[id as usize];
                    if qn != n || q[0] < lo[0] || q[0] > hi[0] || q[1] < lo[1] || q[1] > hi[1] {
                        continue;
                    }
                    let along = if horizontal {
                        ((q[1] - r.a[1]).abs() <= TOUCH).then(|| (q[0] - r.a[0]).abs())
                    } else {
                        ((q[0] - r.a[0]).abs() <= TOUCH).then(|| (q[1] - r.a[1]).abs())
                    };
                    if let Some(t) = along {
                        on.push((t, id));
                    }
                }
            }
        }
        on.sort_by(|x, y| x.0.total_cmp(&y.0));
        on.dedup_by_key(|x| x.1);
        for w in on.windows(2) {
            let length = (w[1].0 - w[0].0).abs();
            let k = split.len() as u32;
            adjacent[w[0].1 as usize].push(Edge {
                to: w[1].1,
                length,
                split: k,
            });
            adjacent[w[1].1 as usize].push(Edge {
                to: w[0].1,
                length,
                split: k,
            });
            split.push((w[0].1, w[1].1, r.value, r.kind));
            split_raw.push(order);
        }
    }
    // A connection inside a slice that runs on through a box from a pin's
    // end: joined to it there, not drawn.
    let drawn_splits = split.len();
    for &(a, b, v) in through {
        let n = net[v as usize];
        let at = |q: [f32; 2]| {
            node_of
                .get(&(
                    (q[0] / TOUCH).round() as i32,
                    (q[1] / TOUCH).round() as i32,
                    n,
                ))
                .copied()
        };
        if let (Some(na), Some(nb)) = (at(a), at(b)) {
            let k = split.len() as u32;
            adjacent[na as usize].push(Edge {
                to: nb,
                length: 0.0,
                split: k,
            });
            adjacent[nb as usize].push(Edge {
                to: na,
                length: 0.0,
                split: k,
            });
            split.push((na, nb, v, Kind::Internal));
        }
    }

    // Where the connections inside slices end: never where a value starts.

    // How far each point is from its net's driver: a driver is a wire no
    // arc in use leads to; it starts where its pins leave their sites, or
    // else from its own loose ends, or from anywhere on it.
    let mut distance = vec![f32::INFINITY; nodes.len()];
    // The piece each point was reached by from its driver's side.
    let mut via = vec![u32::MAX; nodes.len()];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    let mut of_root: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(a, b, v, _) in &split {
        if !sinks.contains(&v) {
            of_root.entry(v).or_default().extend([a, b]);
        }
    }
    // The nets with a driver drawn.
    let driven: HashSet<u32> = of_root.keys().map(|v| net[*v as usize]).collect();
    let mut seeds: Vec<u32> = Vec::new();
    for (_, mut ends) in of_root {
        ends.sort_unstable();
        ends.dedup();
        let at_site: Vec<u32> = ends
            .iter()
            .copied()
            .filter(|&e| {
                let at = key(nodes[e as usize].0);
                site_ends.contains(&at) && !fed_pin.contains(&e) && !internal_ends.contains(&at)
            })
            .collect();
        let loose: Vec<u32> = ends
            .iter()
            .copied()
            .filter(|&e| adjacent[e as usize].len() == 1)
            .collect();
        seeds.extend(if !at_site.is_empty() {
            at_site
        } else if !loose.is_empty() {
            loose
        } else {
            ends
        });
    }
    let run = |seeds: &[u32],
               distance: &mut Vec<f32>,
               via: &mut Vec<u32>,
               heap: &mut BinaryHeap<Reverse<(u32, u32)>>| {
        for &s in seeds {
            distance[s as usize] = 0.0;
            heap.push(Reverse((0, s)));
        }
        while let Some(Reverse((d, u))) = heap.pop() {
            let d = f32::from_bits(d);
            if d > distance[u as usize] {
                continue;
            }
            for e in &adjacent[u as usize] {
                let nd = d + e.length;
                if nd < distance[e.to as usize] {
                    distance[e.to as usize] = nd;
                    via[e.to as usize] = e.split;
                    heap.push(Reverse((nd.to_bits(), e.to)));
                }
            }
        }
    };
    run(&seeds, &mut distance, &mut via, &mut heap);
    // Nets whose driver is not drawn start from a loose end of their own.
    let mut order: Vec<u32> = (0..nodes.len() as u32)
        .filter(|&i| distance[i as usize].is_infinite())
        .collect();
    order.sort_by_key(|&i| adjacent[i as usize].len() != 1);
    for i in order {
        if distance[i as usize].is_infinite() {
            run(&[i], &mut distance, &mut via, &mut heap);
        }
    }
    // The way from each driver to the sites its net feeds: back from every
    // pin end at a site that is not a driver's. A net with no such end
    // drawn is shown whole.
    let mut route = vec![false; split.len()];
    let mut fed: HashSet<u32> = HashSet::new();
    for (n, &(p, net)) in nodes.iter().enumerate() {
        if via[n] == u32::MAX || !site_ends.contains(&key(p)) || blocked.contains(&key(p)) {
            continue;
        }
        fed.insert(net);
        let mut at = n as u32;
        while via[at as usize] != u32::MAX {
            let k = via[at as usize];
            if route[k as usize] {
                break;
            }
            route[k as usize] = true;
            let (a, b, _, _) = split[k as usize];
            at = if a == at { b } else { a };
        }
    }
    // A net that feeds nothing in use carries its value on to nothing:
    // none of it is on a route.
    let mut unfed = 0;
    for s in &split {
        if !fed.contains(&nodes[s.0 as usize].1) {
            unfed += 1;
        }
    }
    let mut node_route = vec![false; nodes.len()];
    for (k, s) in split.iter().enumerate() {
        if route[k] {
            node_route[s.0 as usize] = true;
            node_route[s.1 as usize] = true;
        }
    }
    // Where a net's route starts, and where it enters the sites it feeds.
    let mut drivers: HashMap<u32, Vec<[f32; 2]>> = HashMap::new();
    let mut fed_at: HashMap<u32, Vec<[f32; 2]>> = HashMap::new();
    // Starts that are not a site's pin (a constant starts at the CIB's).
    let mut loose = Vec::new();
    for (n, &(p, net)) in nodes.iter().enumerate() {
        if via[n] == u32::MAX && distance[n] == 0.0 && node_route[n] {
            drivers.entry(net).or_default().push(p);
            if !site_ends.contains(&key(p)) {
                loose.push((order_of[n], (net, p)));
            }
        } else if via[n] != u32::MAX && site_ends.contains(&key(p)) && !blocked.contains(&key(p)) {
            fed_at.entry(net).or_default().push(p);
        }
    }

    // Pieces cut at the tiles they pass, by position.
    let row_at = |y: f32| f.row_at(y) as usize;
    let mut out = Vec::new();
    let mut undriven = 0;
    for (k, &(a, b, v, kind)) in split.iter().enumerate().take(drawn_splits) {
        let (pa, pb) = (nodes[a as usize].0, nodes[b as usize].0);
        let (da, db) = (distance[a as usize], distance[b as usize]);
        if !driven.contains(&net[v as usize]) {
            undriven += 1;
        }
        let horizontal = (pa[1] - pb[1]).abs() <= (pa[0] - pb[0]).abs();
        let axis = usize::from(!horizontal);
        let (lo, hi) = (pa[axis].min(pb[axis]), pa[axis].max(pb[axis]));
        // Borders of tiles across the piece.
        let mut cuts: Vec<f32> = vec![lo, hi];
        if horizontal {
            let mut k = (lo / TILE_W).floor() + 1.0;
            while k * TILE_W < hi {
                cuts.push(k * TILE_W);
                k += 1.0;
            }
        } else {
            for r in row_at(lo) + 1..=row_at(hi) {
                let y = f.row_top[r];
                if y > lo && y < hi {
                    cuts.push(y);
                }
            }
        }
        cuts.sort_by(f32::total_cmp);
        cuts.dedup();
        let at = |x: f32| -> ([f32; 2], f32) {
            let t = if (pb[axis] - pa[axis]).abs() > 0.0 {
                (x - pa[axis]) / (pb[axis] - pa[axis])
            } else {
                0.0
            };
            let mut p = pa;
            p[axis] = x;
            (p, da + (db - da) * t)
        };
        for w in cuts.windows(2) {
            let ((p0, d0), (p1, d1)) = (at(w[0]), at(w[1]));
            let mid = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
            let c = ((mid[0] / TILE_W).floor().max(0.0) as usize).min(cols - 1);
            let position = row_at(mid[1]) * cols + c;
            out.push((
                split_raw[k],
                (
                    position,
                    Piece {
                        a: p0,
                        b: p1,
                        value: v,
                        d0,
                        d1,
                        kind,
                        route: route[k],
                    },
                ),
            ));
        }
    }
    let longest = distance
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .fold(0.0, f32::max);
    let dots = dots
        .iter()
        .zip(&dot_nodes)
        .map(|(&(order, (at, value)), &n)| {
            (
                order,
                Dot {
                    at,
                    value,
                    d: distance[n as usize],
                    route: node_route[n as usize],
                },
            )
        })
        .collect();
    NetDrawing {
        pieces: out,
        dots,
        loose,
        drivers,
        fed_at,
        longest,
        undriven,
        unfed,
    }
}

/// Where a pin of a template, by name, ends on its site.
fn pin_end(t: &crate::fabric::Template, name: &str) -> Option<[f32; 2]> {
    t.segments
        .iter()
        .zip(&t.owners)
        .filter(|(s, o)| {
            s.2 == Part::Pin
                && matches!(o, Owner::Wire(k) if t.wires[*k as usize] == WireKey::Name(name.to_string()))
        })
        .flat_map(|(s, _)| [s.0, s.1])
        .find(|e| t.site_ends.contains(e))
}

/// The straight pieces of a polyline.
fn polyline(raw: &mut Vec<Raw>, points: &[[f32; 2]], value: u32, kind: Kind) {
    for w in points.windows(2) {
        if w[0] != w[1] {
            raw.push(Raw {
                a: w[0],
                b: w[1],
                value,
                kind,
                fed: false,
            });
        }
    }
}

fn by_position<T>(items: Vec<(usize, T)>, positions: usize) -> (Vec<T>, Vec<(u32, u32)>) {
    let mut buckets: Vec<Vec<T>> = (0..positions).map(|_| Vec::new()).collect();
    for (p, item) in items {
        buckets[p].push(item);
    }
    let mut out = Vec::new();
    let mut at = vec![(0, 0); positions];
    for (p, bucket) in buckets.into_iter().enumerate() {
        if !bucket.is_empty() {
            let start = out.len() as u32;
            out.extend(bucket);
            at[p] = (start, out.len() as u32);
        }
    }
    (out, at)
}
