//! The design's wires drawn by the GPU. Every piece of every wire is put in
//! GPU memory once a design (its ends on the die, the share of its net's
//! length at each end, and its wire); each frame only the changes of the
//! wires that changed since are sent, the journal's record of each (its last
//! two changes with their pace, src/changes.rs), and a shader colours every point: the value there
//! with 0/1, and how much a change glows there, by the same rule as the
//! journal. So a frame costs the same for a design of ten thousand LUTs and
//! one of eighty, and moving the view costs nothing on the CPU, which stays
//! with the run.
//!
//! The wires are drawn twice: their colours, then the changes over them,
//! wider the more they glow. The colours come from an image of which piece
//! is on top at each pixel (its wire, and the share of its net's length
//! there), drawn over all the pieces in sight only when the view moves; each
//! frame a pass over the map's pixels colours each from its wire's record,
//! the value there with 0/1. So with 0/1 a frame costs the map's pixels
//! however many values change, not the millions of pieces of a full chip.
//! Of the changes only the pieces a change may light are drawn: a compute
//! pass goes over the pieces in sight once a frame and lists those, and the
//! changes are drawn from the list (the GPU told how many by the pass
//! itself), not over all the pieces.
//! From afar, when hundreds of thousands of pieces glow, those shorter than
//! a uniform threshold in physical pixels splat their exact thin-strip
//! coverage into a buffer with atomic MAX; only longer pieces are
//! rasterized into a single-channel mask with MAX blending. The default is
//! 24 physical pixels: on the Apple M3 Max with life_full at fit, the
//! benchmark's combined splat and strips time fell from 8.441 ms at 1.5 px
//! to 2.385 ms at 24 px, and the window's busy late phase rose from
//! 40.5-42.5 to 57.9-58.4 fps. Pieces longer than 24 px are rare from afar,
//! so the strips pass is nearly empty there. ECP5_SPLAT_MAX overrides the
//! threshold at startup for A/B. The splat walks the major axis, testing
//! three pixels across each step, so work is linear in length and bounded
//! by the threshold. Positive half-float bits keep the same precision in
//! both. The colour pass reads their maximum directly and mixes it into the
//! value on top, so covered wires still show activity without a
//! buffer-to-texture copy. A top-wire-only glow and the old quads can be
//! chosen too. A four-byte read of the list's count chooses the dense way a
//! few frames late; close up the quads keep their wider glow.
//!
//! What a frame asks of the GPU before the window's own drawing (the wires
//! sent, the image of pieces, the list, the record's marks) goes in a
//! command buffer of its own, submitted at once: egui throws its frame's
//! commands away when the window is covered, and these must not be lost
//! with it (a wire sent and dropped would stay stale on the GPU).
//!
//! With a record kept (src/trace.rs), the shader marks each wire it drew lit,
//! and the least and greatest value each wire was drawn with on the screen
//! in the frame read (a wire under others everywhere: none); these are read
//! back now and then, a few frames late.
use crate::changes::Record;
use bytemuck::{Pod, Zeroable};
use eframe::{
    egui::{self, Color32},
    egui_wgpu::{
        self, wgpu, wgpu::util::DeviceExt, CallbackResources, CallbackTrait, ScreenDescriptor,
    },
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};

/// A piece of a wire: its ends on the die, the shares of its net's length
/// there, and its wire (the overlay's value index).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Segment {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub share: [f32; 2],
    pub wire: u32,
}

/// A site, a box or a tile of the grid as the GPU draws it while it is a
/// few pixels across: where it is on the die, its colour with no change
/// (premultiplied RGBA, red in the low byte), the wires whose change makes
/// it glow (`count` of `Spots::wires` from `first`; a site's output first),
/// and what it is (`SPOT_BOX`, `SPOT_SITE`, `SPOT_TILE`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Spot {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub base: u32,
    pub first: u32,
    pub count: u32,
    pub kind: u32,
}

/// A box (an EBR, a pad, a part of a slice): its base colour, lit by a change.
pub const SPOT_BOX: u32 = 0;
/// A LUT or flip-flop: with 0/1 the colour of its output's value.
pub const SPOT_SITE: u32 = 1;
/// A tile of the grid (drawn whatever its size, only when asked for).
pub const SPOT_TILE: u32 = 2;

/// A design's spots, and the wires they glow with.
pub struct Spots {
    pub spots: Vec<Spot>,
    pub wires: Vec<u32>,
}

/// Whether the CPU leaves a site or box from `min` to `max` (die units) to
/// the GPU at `cell` points a die unit: less than 4 points across, as the
/// vertex shader has it.
pub fn spot_small(min: [f32; 2], max: [f32; 2], cell: f32) -> bool {
    ((max[0] - min[0]) * cell).min((max[1] - min[1]) * cell) < 4.0
}

/// The wires' changes as the GPU is to have them, and those changed since
/// it took them last, as the patches that send them (one a wire, its
/// latest state).
#[derive(Default)]
pub struct WireStates {
    pub states: Vec<Record>,
    patches: Vec<Patch>,
    /// Each wire's patch + 1, or 0.
    slot: Vec<u32>,
    /// All of them are to be sent (a new design).
    all: bool,
}

impl WireStates {
    /// The wires of a new design.
    pub fn set_all(&mut self, states: Vec<Record>) {
        self.slot = vec![0; states.len()];
        self.states = states;
        self.patches.clear();
        self.all = true;
    }

    /// A wire changed.
    pub fn set(&mut self, w: u32, state: Record) {
        let i = w as usize;
        self.states[i] = state;
        match self.slot[i] {
            0 => {
                self.patches.push(Patch { index: w, state });
                self.slot[i] = self.patches.len() as u32;
            }
            k => self.patches[k as usize - 1].state = state,
        }
    }

    /// What is to be sent: all, or the patches of the wires changed.
    fn take(&mut self) -> (bool, Vec<Patch>) {
        let patches = std::mem::take(&mut self.patches);
        for p in &patches {
            self.slot[p.index as usize] = 0;
        }
        (std::mem::replace(&mut self.all, false), patches)
    }

    /// The patches taken are sent: their room is kept for the next.
    fn give_back(&mut self, mut patches: Vec<Patch>) {
        if self.patches.is_empty() {
            patches.clear();
            self.patches = patches;
        }
    }
}

/// A wire's state, sent on its own.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Patch {
    index: u32,
    state: Record,
}

const SCATTER: &str = r#"
struct Wire {
    at1: f32, spread1: f32, fade1: f32,
    at2: f32, spread2: f32, fade2: f32,
    values: u32,
    pad: u32,
};
struct Patch {
    index: u32,
    at1: f32, spread1: f32, fade1: f32,
    at2: f32, spread2: f32, fade2: f32,
    values: u32,
    pad: u32,
};
@group(0) @binding(0) var<storage, read> patches: array<Patch>;
@group(0) @binding(1) var<storage, read_write> wires: array<Wire>;
@group(0) @binding(2) var<uniform> count: vec4<u32>;

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= count.x) {
        return;
    }
    let p = patches[i];
    wires[p.index] = Wire(p.at1, p.spread1, p.fade1, p.at2, p.spread2, p.fade2, p.values, p.pad);
}

// The record's marks of the wires listed start again (src/trace.rs).
@group(0) @binding(3) var<storage, read> resets: array<u32>;
@group(0) @binding(4) var<storage, read_write> lit: array<u32>;

@compute @workgroup_size(64)
fn reset(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= count.y) {
        return;
    }
    lit[resets[i]] = 0u;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    /// The die's (0, 0) from the map's top left, and the map's size, in
    /// points; points per die unit; a wire's width in points; the clock;
    /// pixels per point.
    origin: [f32; 2],
    size: [f32; 2],
    cell: f32,
    width: f32,
    now: f32,
    ppp: f32,
    values: u32,
    layer: u32,
    record: u32,
    /// 1: pieces no change can light are dropped before their pixels.
    cull: u32,
    /// The frame drawn, for the record's marks.
    frame: u32,
    flat: u32,
    glow_mode: u32,
    record_lit: u32,
    mask_size: [u32; 2],
    splat_max: f32,
    mask_pad: u32,
    used: [f32; 4],
    one: [f32; 4],
    zero: [f32; 4],
    unknown: [f32; 4],
    change: [f32; 4],
}

const SHADER: &str = r#"
struct Uniforms {
    origin: vec2<f32>,
    size: vec2<f32>,
    cell: f32,
    width: f32,
    now: f32,
    ppp: f32,
    values: u32,
    layer: u32,
    record: u32,
    cull: u32,
    frame: u32,
    // 1: the changes drawn flat, for measuring (ECP5_GUI_FLAT_CHANGES).
    flat: u32,
    glow_mode: u32,
    record_lit: u32,
    mask_size: vec2<u32>,
    splat_max: f32,
    mask_pad: u32,
    used: vec4<f32>,
    one: vec4<f32>,
    zero: vec4<f32>,
    unknown: vec4<f32>,
    change: vec4<f32>,
};
struct Wire {
    at1: f32, spread1: f32, fade1: f32,
    at2: f32, spread2: f32, fade2: f32,
    values: u32,
    // How busy the wire was at its later change (f32 bits; src/changes.rs).
    busy: u32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> wires: array<Wire>;
@group(0) @binding(2) var<storage, read_write> lit: array<atomic<u32>>;
// The values each wire is drawn with on the screen, a bit each: 0, 1, and
// any other (unknown).
@group(0) @binding(3) var<storage, read_write> seen: array<atomic<u32>>;

// The pieces as stored (seven words, as `Segment`), and the list of those
// a change may light (CULL), for drawing the changes.
struct Seg {
    ax: f32, ay: f32, bx: f32, by: f32,
    s0: f32, s1: f32,
    wire: u32,
};
@group(1) @binding(0) var<storage, read> segs: array<Seg>;
@group(1) @binding(1) var<storage, read> list: array<u32>;

struct Piece {
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) share: vec2<f32>,
    @location(3) wire: u32,
};
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) along: f32,
    @location(1) across: f32,
    @location(2) @interpolate(flat) wire: u32,
    @location(3) @interpolate(flat) share: vec2<f32>,
    @location(4) @interpolate(flat) length: f32,
    // The wire's changes and values, read once a piece, not a pixel (the
    // changes only: the colours read them a pixel, from the image of pieces).
    @location(6) @interpolate(flat) c1: vec3<f32>,
    @location(7) @interpolate(flat) c2: vec3<f32>,
    @location(8) @interpolate(flat) values: u32,
    // 1: the record waits on this wire's being drawn lit (the pass that
    // lists the pieces knows; drawn over all the pieces, always 1).
    @location(9) @interpolate(flat) mark: u32,
    // 1: a change lights the piece alike all along it (one value, one glow
    // at both ends), in `colour`; its quad is then just the line drawn.
    @location(10) @interpolate(flat) even: u32,
    @location(11) @interpolate(flat) colour: vec4<f32>,
    // How much a change at a fast rate glows on this wire (`busy_of`).
    @location(12) @interpolate(flat) weight: f32,
};

// How much a change at a fast rate glows on wire `w`: from BUSY_LEAST for a
// quiet wire to 1 for a busy one, by how busy it is now (as it was at its
// later change, decayed since), as src/changes.rs has it.
const BUSY_TAU: f32 = 1.0;
const BUSY_LEAST: f32 = 0.2;
const BUSY_FULL: f32 = 0.5;
const FAST: f32 = 1e-3;
fn busy_of(w: Wire) -> f32 {
    if (w.at2 < -1e29) {
        return 1.0;
    }
    let busy = clamp(bitcast<f32>(w.busy) * exp(-max(u.now - w.at2, 0.0) / BUSY_TAU), 0.0, 1.0);
    return BUSY_LEAST + (1.0 - BUSY_LEAST) * clamp(busy / BUSY_FULL, 0.0, 1.0);
}

// A change's glow, by `weight` when it came at a fast rate.
fn glow_by(at: f32, spread: f32, fade: f32, s: f32, weight: f32) -> f32 {
    let g = glow_of(at, spread, fade, s);
    return select(g, g * weight, spread < FAST);
}

// The most a wire glows at share s: the greater of its two changes'.
fn glow2(w: Wire, s: f32, weight: f32) -> f32 {
    return max(glow_by(w.at1, w.spread1, w.fade1, s, weight), glow_by(w.at2, w.spread2, w.fade2, s, weight));
}

fn glow_of(at: f32, spread: f32, fade: f32, s: f32) -> f32 {
    if (at < -1e29) {
        return 0.0;
    }
    let reached = at + spread * s;
    if (u.now < reached) {
        return 0.0;
    }
    return max(1.0 - (u.now - reached) / fade, 0.0);
}

// The most a change glows on a piece from share s0 to s1 now: where it
// reached last, its front or the far end once past it.
fn peak(at: f32, spread: f32, fade: f32, s0: f32, s1: f32, weight: f32) -> f32 {
    if (at < -1e29) {
        return 0.0;
    }
    var s = s1;
    if (spread > 0.0) {
        s = clamp((u.now - at) / spread, s0, s1);
    }
    return glow_by(at, spread, fade, s, weight);
}

// Whether a change may light some point of a piece from share s0 to s1:
// it has reached the near end, and the far end not long enough ago to be
// out.
fn may_light(at: f32, spread: f32, fade: f32, s0: f32, s1: f32) -> bool {
    return at > -1e29 && u.now >= at + spread * s0 && u.now <= at + spread * s1 + fade;
}

fn value_at(w: Wire, s: f32) -> u32 {
    if (w.at2 > -1e29 && u.now >= w.at2 + w.spread2 * s) {
        return (w.values >> 16u) & 255u;
    }
    if (w.at1 > -1e29 && u.now >= w.at1 + w.spread1 * s) {
        return (w.values >> 8u) & 255u;
    }
    if (w.at1 > -1e29 || w.at2 > -1e29) {
        return (w.values >> 24u) & 255u;
    }
    return w.values & 255u;
}

fn colour_of(v: u32) -> vec4<f32> {
    if (u.values == 0u) {
        return u.used;
    }
    if (v == 1u) {
        return u.one;
    }
    if (v == 0u) {
        return u.zero;
    }
    return u.unknown;
}

// The same mix for a quad and for a pixel of the cached image.
fn glowing_colour(v: u32, glow: f32) -> vec4<f32> {
    return mix(colour_of(v), u.change, glow);
}

fn mark_lit(wire: u32) {
    let mark = 0xFFFFFFFFu - u.frame;
    // Only the first fragments after a reset contend for this wire.
    if (atomicLoad(&lit[wire]) == 0u) {
        atomicMax(&lit[wire], mark);
    }
}

// At the switch to pixels the extra width is zero. It grows smoothly to
// the usual close-up width over the next factor of two in zoom. The lane
// spacing there is still far below a pixel (0.0022 die units).
fn change_width(glow: f32) -> f32 {
    return u.width * (1.0 + 1.2 * glow * smoothstep(16.0, 32.0, u.cell * u.ppp));
}

fn mask_point(point: vec2<f32>) -> vec2<f32> {
    return (u.origin + point * u.cell) / u.size * vec2<f32>(u.mask_size);
}

// Corner `k` of the quad of a piece from `a` to `b` (die units), with the
// shares of its net's length at its ends; `cull`: nothing to draw where no
// change may light it. The image of pieces (layer 0) needs no record of the
// wire: only where the piece is.
fn place(k: u32, a: vec2<f32>, b: vec2<f32>, share: vec2<f32>, wire: u32, cull: bool) -> Out {
    var o: Out;
    var w = Wire(-1e30, 0.0, 1.0, -1e30, 0.0, 1.0, 0u, 0u);
    if (u.layer == 1u || u.layer == 3u) {
        w = wires[wire];
    }
    o.wire = wire;
    o.share = share;
    o.c1 = vec3<f32>(w.at1, w.spread1, w.fade1);
    o.c2 = vec3<f32>(w.at2, w.spread2, w.fade2);
    o.values = w.values;
    o.weight = 1.0;
    if (u.layer == 1u || u.layer == 3u) {
        o.weight = busy_of(w);
    }
    o.mark = 1u;
    o.even = 0u;
    o.colour = vec4<f32>(0.0);
    let s0 = min(share.x, share.y);
    let s1 = max(share.x, share.y);
    // The changes: only the pieces a change may light, the others nothing
    // to draw.
    if (cull) {
        if (!may_light(w.at1, w.spread1, w.fade1, s0, s1) && !may_light(w.at2, w.spread2, w.fade2, s0, s1)) {
            o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
            return o;
        }
    }
    let end = f32(k / 2u);
    let side = select(-1.0, 1.0, (k & 1u) == 1u);
    var pa = u.origin + a * u.cell;
    var pb = u.origin + b * u.cell;
    var source_a = pa;
    var source_b = pb;
    if (u.layer == 3u) {
        // A sub-pixel piece must still put activity into its nearest
        // pixel. Snapped endpoints also give a zero-length piece a pixel.
        source_a = mask_point(a);
        source_b = mask_point(b);
        pa = floor(source_a) + vec2<f32>(0.5);
        pb = floor(source_b) + vec2<f32>(0.5);
    }
    let d = pb - pa;
    let len = length(d);
    var dir = vec2<f32>(1.0, 0.0);
    if (len > 1e-4) {
        dir = d / len;
    }
    let n = vec2<f32>(-dir.y, dir.x);
    // Room for the line as drawn, wider for a change the more it glows (at
    // the most it glows on the piece now): the pixels whose centres the
    // fragments keep (no more: those past it would only be thrown away).
    var half = max(u.width * 0.5, 0.5 / u.ppp);
    if (u.layer == 3u) {
        half = 0.5;
        let g0 = glow2(w, s0, o.weight);
        let g1 = glow2(w, s1, o.weight);
        let front = (u.now >= w.at1 + w.spread1 * s0) != (u.now >= w.at1 + w.spread1 * s1)
            || (u.now >= w.at2 + w.spread2 * s0) != (u.now >= w.at2 + w.spread2 * s1);
        if (!front && abs(g0 - g1) < 1.0 / 65536.0) {
            o.even = 1u;
            o.colour.x = max(g0, g1);
        }
    }
    if (u.layer == 1u) {
        let k = max(peak(w.at1, w.spread1, w.fade1, s0, s1, o.weight), peak(w.at2, w.spread2, w.fade2, s0, s1, o.weight));
        half = max(change_width(k) * 0.5, 0.5 / u.ppp);
        // Lit alike all along (no change's front on it, as nearly always
        // from afar or at a fast rate): its colour once a piece, not a
        // pixel, as the fragments would have it; nothing where it is out.
        let g0 = glow2(w, s0, o.weight);
        let g1 = glow2(w, s1, o.weight);
        let v0 = value_at(w, s0);
        // (No front within it: each change has reached both ends, or
        // neither: a front between ends both out may glow behind it.)
        let front = (u.now >= w.at1 + w.spread1 * s0) != (u.now >= w.at1 + w.spread1 * s1)
            || (u.now >= w.at2 + w.spread2 * s0) != (u.now >= w.at2 + w.spread2 * s1);
        if (!front && v0 == value_at(w, s1) && abs(g0 - g1) < 1.0 / 255.0) {
            let g = max(g0, g1);
            if (g <= 0.0) {
                o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
                return o;
            }
            o.even = 1u;
            o.colour = glowing_colour(v0, g);
            half = max(change_width(g) * 0.5, 0.5 / u.ppp);
        }
    }
    let along = select(-half, len + half, end > 0.5);
    let q = pa + dir * along + n * side * half;
    let draw_size = select(u.size, vec2<f32>(u.mask_size), u.layer == 3u);
    o.pos = vec4<f32>(q.x / draw_size.x * 2.0 - 1.0, 1.0 - q.y / draw_size.y * 2.0, 0.0, 1.0);
    o.along = along;
    o.across = side * half;
    o.length = len;
    if (u.layer == 3u) {
        // Coverage is snapped, but the glow is still evaluated at this
        // point of the original piece, with its original share and time.
        let source_d = source_b - source_a;
        o.length = length(source_d);
        if (o.length > 1e-4) {
            o.along = dot(q - source_a, source_d / o.length);
        }
    }
    return o;
}

// The pieces, a quad an instance: the image of pieces, and the changes when
// they are not drawn from the list.
@vertex
fn vs(@builtin(vertex_index) k: u32, p: Piece) -> Out {
    return place(k, p.a, p.b, p.share, p.wire, u.layer == 1u && u.cull == 1u);
}

// The changes over the pieces listed: instance i is the list's i-th.
@vertex
fn vs_listed(@builtin(vertex_index) k: u32, @builtin(instance_index) i: u32) -> Out {
    let e = list[i];
    let p = segs[e & 0x7fffffffu];
    var o = place(k, vec2<f32>(p.ax, p.ay), vec2<f32>(p.bx, p.by), vec2<f32>(p.s0, p.s1), p.wire, false);
    o.mark = e >> 31u;
    return o;
}

// The same geometry, with a one-physical-pixel width (layer 3). Separate
// entry points let the mask use either the list or the fallback ranges.
@vertex
fn vs_mask(@builtin(vertex_index) k: u32, p: Piece) -> Out {
    return place(k, p.a, p.b, p.share, p.wire, u.cull == 1u);
}

@vertex
fn vs_mask_listed(@builtin(vertex_index) k: u32, @builtin(instance_index) i: u32) -> Out {
    let e = list[i];
    let p = segs[e & 0x7fffffffu];
    var o = place(k, vec2<f32>(p.ax, p.ay), vec2<f32>(p.bx, p.by), vec2<f32>(p.s0, p.s1), p.wire, false);
    o.mark = e >> 31u;
    return o;
}

// Where along its net a pixel of a piece is, as a share of the net's length.
fn share_at(i: Out) -> f32 {
    var t = 0.5;
    if (i.length > 1e-4) {
        t = clamp(i.along / i.length, 0.0, 1.0);
    }
    return mix(i.share.x, i.share.y, t);
}

// The image of pieces: at each pixel the piece drawn last (on top, as the
// colours were drawn), as its wire + 1 (0: none) and the share there.
@fragment
fn fs_id(i: Out) -> @location(0) vec2<u32> {
    let half = max(u.width * 0.5, 0.5 / u.ppp);
    if (abs(i.across) > half || i.along < -half || i.along > i.length + half) {
        discard;
    }
    return vec2<u32>(i.wire + 1u, bitcast<u32>(share_at(i)));
}

// The colours, a pass over the map's pixels: each the colour of its piece's
// wire, the value there now with 0/1.
@group(1) @binding(2) var ids: texture_2d<u32>;
@group(1) @binding(4) var glow_mask: texture_2d<f32>;
// Positive half-float bits are ordered as integers. Matching R16Float's
// precision lets the colour pass take MAX without a texture copy.
@group(1) @binding(5) var<storage, read_write> mask_pixels: array<atomic<u32>>;
@group(1) @binding(6) var<storage, read> mask_args: array<u32>;
@group(1) @binding(7) var<storage, read_write> mask_long: array<u32>;
@group(1) @binding(8) var<storage, read_write> mask_long_args: array<atomic<u32>>;

struct Full {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Full {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Full;
    o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs_colours(i: Full) -> @location(0) vec4<f32> {
    let size = textureDimensions(ids);
    let p = min(vec2<u32>(i.uv * vec2<f32>(size)), size - vec2<u32>(1u, 1u));
    let id = textureLoad(ids, p, 0).xy;
    var mask = 0.0;
    if (u.glow_mode == 2u) {
        mask = max(textureLoad(glow_mask, p, 0).x,
            unpack2x16float(atomicLoad(&mask_pixels[p.y * size.x + p.x])).x);
    }
    if (id.x == 0u) {
        if (mask > 0.0) {
            return mix(u.used, u.change, mask);
        }
        discard;
    }
    if (u.values == 0u && u.glow_mode == 0u && u.record == 0u) {
        return u.used;
    }
    let wire = id.x - 1u;
    let w = wires[wire];
    let share = bitcast<f32>(id.y);
    let v = value_at(w, share);
    // The values the wire is drawn with on the screen, in the frames whose
    // marks are read back; most pixels of a wire find theirs marked already.
    if (u.record == 1u) {
        let bit = 1u << min(v, 2u);
        if ((atomicLoad(&seen[wire]) & bit) == 0u) {
            atomicOr(&seen[wire], bit);
        }
    }
    if (u.glow_mode == 1u) {
        let glow = glow2(w, share, busy_of(w));
        if (glow > 0.0 && u.record_lit == 1u) {
            mark_lit(wire);
        }
        if (u.flat == 1u && glow > 0.0) {
            return u.change;
        }
        return glowing_colour(v, glow);
    }
    if (u.glow_mode == 2u) {
        return glowing_colour(v, mask);
    }
    return colour_of(v);
}

// No colour or widening in the mask. Usually a piece glows alike all
// along it; only a front or a changing glow needs the per-point value.
@fragment
fn fs_mask(i: Out) -> @location(0) f32 {
    var glow = i.colour.x;
    if (i.even == 0u) {
        let w = Wire(i.c1.x, i.c1.y, i.c1.z, i.c2.x, i.c2.y, i.c2.z, i.values, 0u);
        glow = glow2(w, share_at(i), i.weight);
    }
    if (glow <= 0.0) {
        discard;
    }
    if (u.record == 1u && i.mark == 1u) {
        mark_lit(i.wire);
    }
    return glow;
}

fn mask_share(share: vec2<f32>, source_a: vec2<f32>, source_b: vec2<f32>, point: vec2<f32>) -> f32 {
    let d = source_b - source_a;
    let len = length(d);
    var t = 0.5;
    if (len > 1e-4) {
        t = clamp(dot(point - source_a, d / len) / len, 0.0, 1.0);
    }
    return mix(share.x, share.y, t);
}

// The same glow as fs_mask, with the same constant-piece shortcut.
fn mask_glow(w: Wire, share: vec2<f32>, source_a: vec2<f32>, source_b: vec2<f32>, point: vec2<f32>, weight: f32) -> f32 {
    let s0 = min(share.x, share.y);
    let s1 = max(share.x, share.y);
    let g0 = glow2(w, s0, weight);
    let g1 = glow2(w, s1, weight);
    let front = (u.now >= w.at1 + w.spread1 * s0) != (u.now >= w.at1 + w.spread1 * s1)
        || (u.now >= w.at2 + w.spread2 * s0) != (u.now >= w.at2 + w.spread2 * s1);
    if (!front && abs(g0 - g1) < 1.0 / 65536.0) {
        return max(g0, g1);
    }
    return glow2(w, mask_share(share, source_a, source_b, point), weight);
}

fn splat_pixel(w: Wire, p: Seg, a: vec2<f32>, b: vec2<f32>, pixel: vec2<i32>, weight: f32) -> bool {
    let glow = mask_glow(w, vec2<f32>(p.s0, p.s1), a, b, vec2<f32>(pixel) + vec2<f32>(0.5), weight);
    if (glow <= 0.0) {
        return false;
    }
    let at = u32(pixel.y) * u.mask_size.x + u32(pixel.x);
    let bits = pack2x16float(vec2<f32>(glow, 0.0)) & 65535u;
    if (atomicLoad(&mask_pixels[at]) < bits) {
        atomicMax(&mask_pixels[at], bits);
    }
    return true;
}

// Short pieces cover very few pixels. Most have both ends in one pixel;
// the others test the thin strip's exact coverage, including rotated caps.
// Endpoints alone would miss pixels of a snapped (2,1) diagonal.
fn splat_piece(p: Seg, a: vec2<f32>, b: vec2<f32>) {
    let pa = floor(a) + vec2<f32>(0.5);
    let pb = floor(b) + vec2<f32>(0.5);
    let w = wires[p.wire];
    let weight = busy_of(w);
    var contributed = false;
    if (all(pa == pb)) {
        let pixel = vec2<i32>(floor(pa));
        if (all(pixel >= vec2<i32>(0)) && all(pixel < vec2<i32>(u.mask_size))) {
            contributed = splat_pixel(w, p, a, b, pixel, weight);
        }
    } else {
        let d = pb - pa;
        let len = length(d);
        let dir = d / len;
        let normal = vec2<f32>(-dir.y, dir.x);
        let reach = (abs(dir) + abs(normal)) * 0.5;
        let lo = max(vec2<i32>(floor(min(pa, pb) - reach)), vec2<i32>(0));
        let hi = min(vec2<i32>(ceil(max(pa, pb) + reach)) - vec2<i32>(1), vec2<i32>(u.mask_size) - vec2<i32>(1));
        // At a major-axis pixel centre the strip reaches at most sqrt(2)/2
        // across its centre line. These three neighbours include the rotated
        // caps too; the same along/across test keeps only covered centres.
        let major = select(1u, 0u, abs(d.x) >= abs(d.y));
        let minor = 1u - major;
        for (var step = lo[major]; step <= hi[major]; step = step + 1) {
            let centre = pa[minor] + (f32(step) + 0.5 - pa[major]) * d[minor] / d[major];
            let across = i32(floor(centre));
            for (var side = max(across - 1, lo[minor]); side <= min(across + 1, hi[minor]); side = side + 1) {
                var pixel: vec2<i32>;
                pixel[major] = step;
                pixel[minor] = side;
                let q = vec2<f32>(pixel) + vec2<f32>(0.5) - pa;
                let along = dot(q, dir);
                if (along >= -0.5 && along <= len + 0.5 && abs(dot(q, normal)) <= 0.5) {
                    contributed = splat_pixel(w, p, a, b, pixel, weight) || contributed;
                }
            }
        }
    }
    if (contributed && u.record == 1u) {
        mark_lit(p.wire);
    }
}

var<workgroup> mask_kept: atomic<u32>;
var<workgroup> mask_base: u32;

@compute @workgroup_size(256)
fn splat(@builtin(workgroup_id) g: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>, @builtin(local_invocation_index) l: u32) {
    if (l == 0u) { atomicStore(&mask_kept, 0u); }
    workgroupBarrier();
    let i = (g.y * n.x + g.x) * 256u + l;
    var keep = false;
    var slot = 0u;
    if (i < mask_args[1]) {
        let p = segs[list[i] & 0x7fffffffu];
        let a = mask_point(vec2<f32>(p.ax, p.ay));
        let b = mask_point(vec2<f32>(p.bx, p.by));
        if (length(b - a) < u.splat_max) {
            splat_piece(p, a, b);
        } else {
            keep = true;
            slot = atomicAdd(&mask_kept, 1u);
        }
    }
    workgroupBarrier();
    if (l == 0u) {
        let count = atomicLoad(&mask_kept);
        mask_base = 0u;
        if (count > 0u) { mask_base = atomicAdd(&mask_long_args[1], count); }
    }
    workgroupBarrier();
    if (keep) { mask_long[mask_base + slot] = list[i]; }
}

// The sites, boxes and grid tiles a few pixels across (`Spot`): a quad an
// instance, filled with its colour now; one closer is drawn by the CPU,
// with its outline and label, so it is nothing here.
@group(1) @binding(3) var<storage, read> spot_wires: array<u32>;

struct SpotIn {
    @location(0) min: vec2<f32>,
    @location(1) max: vec2<f32>,
    @location(2) base: u32,
    @location(3) first: u32,
    @location(4) count: u32,
    @location(5) kind: u32,
};
struct SpotOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) colour: vec4<f32>,
};

@vertex
fn vs_spot(@builtin(vertex_index) k: u32, s: SpotIn) -> SpotOut {
    var o: SpotOut;
    var lo = u.origin + s.min * u.cell;
    var hi = u.origin + s.max * u.cell;
    let size = (s.max - s.min) * u.cell;
    if (s.kind == 2u) {
        // A tile of the grid keeps a gap to the next from 3 points a die
        // unit on.
        if (u.cell >= 3.0) {
            lo = lo + vec2<f32>(0.5, 0.5);
            hi = hi - vec2<f32>(0.5, 0.5);
        }
    } else if (min(size.x, size.y) >= 4.0) {
        o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return o;
    }
    let x = select(lo.x, hi.x, (k & 1u) == 1u);
    let y = select(lo.y, hi.y, k >= 2u);
    o.pos = vec4<f32>(x / u.size.x * 2.0 - 1.0, 1.0 - y / u.size.y * 2.0, 0.0, 1.0);
    // It glows with a change of any of its wires at the driver; a site
    // with 0/1 has the colour of its output's value.
    var glow = 0.0;
    for (var i = 0u; i < s.count; i = i + 1u) {
        let w = wires[spot_wires[s.first + i]];
        glow = max(glow, glow2(w, 0.0, busy_of(w)));
    }
    var fill = unpack4x8unorm(s.base);
    if (s.kind == 1u && u.values == 1u && s.count > 0u) {
        fill = colour_of(value_at(wires[spot_wires[s.first]], 0.0));
    }
    o.colour = mix(fill, u.change, glow);
    return o;
}

@fragment
fn fs_spot(i: SpotOut) -> @location(0) vec4<f32> {
    return i.colour;
}

// The changes over the colours.
@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    if (u.flat == 1u) {
        return u.change;
    }
    // Lit alike all along: the quad is the line drawn.
    if (i.even == 1u) {
        if (u.record == 1u && i.mark == 1u) {
            mark_lit(i.wire);
        }
        return i.colour;
    }
    let s = share_at(i);
    let w = Wire(i.c1.x, i.c1.y, i.c1.z, i.c2.x, i.c2.y, i.c2.z, i.values, 0u);
    let k = glow2(w, s, i.weight);
    if (k <= 0.0) {
        discard;
    }
    let half = max(change_width(k) * 0.5, 0.5 / u.ppp);
    if (abs(i.across) > half || i.along < -half || i.along > i.length + half) {
        discard;
    }
    // The first frame since the wire changed: the greatest mark.
    if (u.record == 1u && i.mark == 1u) {
        mark_lit(i.wire);
    }
    return glowing_colour(value_at(w, s), k);
}
"#;

/// The pieces a change may light, listed: the pieces in sight come as runs
/// (the first piece of each and the threads before it); each thread takes
/// one piece, and those kept are put in the list, a workgroup's together,
/// with how many in the draw's arguments (vertices, instances, first
/// vertex, first instance).
const CULL: &str = r#"
struct Seg {
    ax: f32, ay: f32, bx: f32, by: f32,
    s0: f32, s1: f32,
    wire: u32,
};
struct Wire {
    at1: f32, spread1: f32, fade1: f32,
    at2: f32, spread2: f32, fade2: f32,
    values: u32,
    pad: u32,
};
struct Cull {
    now: f32,
    runs: u32,
    total: u32,
    record: u32,
};
@group(0) @binding(0) var<uniform> c: Cull;
@group(0) @binding(1) var<storage, read> segs: array<Seg>;
@group(0) @binding(2) var<storage, read> wires: array<Wire>;
@group(0) @binding(3) var<storage, read> runs: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> list: array<u32>;
@group(0) @binding(5) var<storage, read_write> args: array<atomic<u32>>;
// The record's marks of being drawn lit (src/trace.rs): a piece of a wire
// not marked since it changed is listed with its top bit set, and only
// those mark it as they are drawn.
@group(0) @binding(6) var<storage, read> lit: array<u32>;

var<workgroup> kept: atomic<u32>;
var<workgroup> base: u32;

fn may_light(at: f32, spread: f32, fade: f32, s0: f32, s1: f32) -> bool {
    return at > -1e29 && c.now >= at + spread * s0 && c.now <= at + spread * s1 + fade;
}

@compute @workgroup_size(256)
fn cull(
    @builtin(workgroup_id) g: vec3<u32>,
    @builtin(num_workgroups) n: vec3<u32>,
    @builtin(local_invocation_index) l: u32,
) {
    if (l == 0u) {
        atomicStore(&kept, 0u);
    }
    workgroupBarrier();
    let t = (g.y * n.x + g.x) * 256u + l;
    var piece = 0u;
    var slot = 0u;
    var keep = false;
    if (t < c.total) {
        // The run the thread is in: the last starting at or before it.
        var lo = 0u;
        var hi = c.runs;
        while (hi - lo > 1u) {
            let mid = (lo + hi) / 2u;
            if (runs[mid].y <= t) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        piece = runs[lo].x + (t - runs[lo].y);
        let p = segs[piece];
        let w = wires[p.wire];
        let s0 = min(p.s0, p.s1);
        let s1 = max(p.s0, p.s1);
        if (may_light(w.at1, w.spread1, w.fade1, s0, s1) || may_light(w.at2, w.spread2, w.fade2, s0, s1)) {
            keep = true;
            slot = atomicAdd(&kept, 1u);
        }
    }
    workgroupBarrier();
    if (l == 0u) {
        base = atomicAdd(&args[1], atomicLoad(&kept));
    }
    workgroupBarrier();
    if (keep) {
        var flag = 0u;
        if (c.record == 1u && lit[segs[piece].wire] == 0u) {
            flag = 1u << 31u;
        }
        list[base + slot] = piece | flag;
    }
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullUniforms {
    now: f32,
    runs: u32,
    total: u32,
    /// 1: a record is kept (pieces are flagged for its marks).
    record: u32,
}

/// Runs of pieces the list can be made of in one frame: a row of tiles is
/// one, so a few hundred at most.
const RUNS: u64 = 4096;

/// The image of pieces: a pixel's wire + 1 (0: no piece) and the share of
/// its net's length there (f32 bits).
const PIECES_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
// R16Float is blendable without optional features and keeps glows below
// 1/255, which an R8 mask would round away while still marking them lit.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// What the changes are drawn from: the list of the pieces a change may
/// light, the draw's arguments, the runs of pieces in sight; the group the
/// pass makes the list with, and the group the drawing reads it with.
struct Listed {
    _list: wgpu::Buffer,
    args: wgpu::Buffer,
    /// How many pieces the frame's pass listed, for the record.
    count: wgpu::Buffer,
    runs: wgpu::Buffer,
    cull_group: wgpu::BindGroup,
    draw_group: wgpu::BindGroup,
    total: u32,
    mask: Option<MaskList>,
}

/// Only the long pieces go through the rasterizer. The compute group is
/// kept with the pixel buffer it writes, and rebuilt when the view resizes.
struct MaskList {
    _list: wgpu::Buffer,
    args: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    pixels: wgpu::Buffer,
    splat_group: wgpu::BindGroup,
}

/// The image of pieces: at each pixel of the map the piece on top, as its
/// wire and the share of its net's length there; kept while the view
/// stays, whatever the values do.
struct Pieces {
    texture: wgpu::Texture,
    mask: wgpu::Texture,
    splats: wgpu::Buffer,
    group: wgpu::BindGroup,
    size: [u32; 2],
    /// What it was drawn for: the design, the view, the width.
    key: Vec<u64>,
}

/// What the GPU drew, read back: for each wire whether it was drawn lit
/// (since the read before), and the least and greatest value it was drawn
/// with in the frame read (none: 255 and 0).
pub struct Readback {
    /// The time and the number of the frame read (on the changes' clock).
    pub at: f64,
    pub of: u32,
    /// Each wire's mark (`trace::mark`): the first frame it was drawn lit
    /// since it changed.
    pub lit: Vec<u32>,
    /// Each wire's values drawn on the screen in the frame read, a bit
    /// each: 0, 1, other (`trace::Drawn::values_of`).
    pub seen: Vec<u32>,
    /// The pieces listed to draw changes on in the frame read.
    pub listed: u32,
    /// Whether this drawing gave only the cached image's top wires their
    /// glow. Wires with no pixel cannot be judged as lit in this mode.
    pub pixel_glow: bool,
}

/// Where a read back is on its way.
enum Reading {
    Idle,
    Copied,
    Mapping(Arc<AtomicBool>),
}

/// Small measurements for the record, independent of the wire marks.
#[derive(Clone, Copy, Default)]
pub struct Measurements {
    pub upload_frame: Option<u32>,
    pub upload_bytes: u64,
    pub full_upload: bool,
    pub gpu: Option<GpuTime>,
    pub glow: Option<GlowMode>,
}

/// The mode and the actual listed count of a drawing, read without waiting.
#[derive(Clone, Copy)]
pub struct GlowMode {
    pub frame: u32,
    pub choice: GlowChoice,
    pub mode: Glow,
    pub listed: u32,
}

/// A script's requested way of drawing changes. Dense choices still keep
/// the wider quads close up and where only a few pieces glow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlowChoice {
    #[default]
    Auto,
    Quads,
    Top,
    Mask,
}

impl GlowChoice {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(Self::Auto),
            "quads" => Some(Self::Quads),
            "top" => Some(Self::Top),
            "mask" => Some(Self::Mask),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Quads => "quads",
            Self::Top => "top",
            Self::Mask => "mask",
        }
    }

    fn choose(self, was: Glow, count: u32, cell: f32, ppp: f32) -> Glow {
        if self == Self::Quads || !pixel_glow(was != Glow::Quads, count, cell, ppp) {
            return Glow::Quads;
        }
        match self {
            Self::Auto => AUTO_DENSE_GLOW,
            Self::Quads => Glow::Quads,
            Self::Top => Glow::Top,
            Self::Mask => Glow::Mask,
        }
    }
}

/// What is actually drawn this frame; the values are the shader's modes.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glow {
    Quads = 0,
    Top = 1,
    Mask = 2,
}

impl Glow {
    pub fn name(self) -> &'static str {
        match self {
            Self::Quads => "quads",
            Self::Top => "top",
            Self::Mask => "mask",
        }
    }
}

// The dense way used by auto, kept in one place for the window's A/B runs.
const AUTO_DENSE_GLOW: Glow = Glow::Mask;

// A quarter million quads already cost a million vertices, before their
// overlapping pixels. Below 16 physical pixels a die unit the 0.0022-unit
// lanes are less than 0.04 pixels apart: extra width cannot resolve them.
// Half the entry count to leave keeps activity near the limit from flapping.
const PIXEL_GLOW_PIECES: u32 = 250_000;
const PIXEL_GLOW_CELL: f32 = 16.0;

fn pixel_glow(was: bool, count: u32, cell: f32, ppp: f32) -> bool {
    cell * ppp <= PIXEL_GLOW_CELL
        && count
            >= if was {
                PIXEL_GLOW_PIECES / 2
            } else {
                PIXEL_GLOW_PIECES
            }
}

/// Full writes only past three quarters: patches still use less space
/// (36 bytes each, against 32 for a wire), with room for the scatter's cost.
fn full_upload(patches: u64, wires: u64) -> bool {
    patches > wires * 3 / 4
}

/// Two four-byte counts in flight, also when no record is kept and on
/// adapters without timestamps. No wire readback or GPU wait is needed to
/// choose the drawing. Each count stays with its view and its drawing mode.
struct ListingReads {
    slots: [ListingRead; 2],
    latest: Option<(Vec<u64>, GlowMode)>,
}

struct ListingRead {
    buffer: wgpu::Buffer,
    done: Arc<AtomicU32>,
    pending: Option<(Vec<u64>, GlowMode)>,
    target: Option<Arc<Mutex<Measurements>>>,
}

impl ListingReads {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            slots: std::array::from_fn(|_| ListingRead {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("map listed count read back"),
                    size: 8,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                done: Arc::new(AtomicU32::new(0)),
                pending: None,
                target: None,
            }),
            latest: None,
        }
    }

    fn poll(&mut self, device: &wgpu::Device) {
        if self.slots.iter().all(|s| s.pending.is_none()) {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.slots {
            let done = slot.done.load(Ordering::Acquire);
            if done == 0 {
                continue;
            }
            if let Some((key, mut mode)) = slot.pending.take() {
                if done == 1 {
                    {
                        let view = slot.buffer.slice(..).get_mapped_range();
                        mode.listed = bytemuck::cast_slice::<u8, u32>(&view)[0];
                    }
                    if self
                        .latest
                        .as_ref()
                        .is_none_or(|(old_key, old)| *old_key != key || mode.frame > old.frame)
                    {
                        self.latest = Some((key, mode));
                        if let Some(target) = &slot.target {
                            target.lock().unwrap().glow = Some(mode);
                        }
                    }
                }
                slot.buffer.unmap();
            }
            slot.target = None;
            slot.done.store(0, Ordering::Relaxed);
        }
    }

    fn copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        count: &wgpu::Buffer,
        key: Vec<u64>,
        mode: GlowMode,
        target: Option<&Arc<Mutex<Measurements>>>,
    ) {
        let Some(slot) = self.slots.iter_mut().find(|s| s.pending.is_none()) else {
            return;
        };
        encoder.copy_buffer_to_buffer(count, 0, &slot.buffer, 0, 4);
        slot.pending = Some((key, mode));
        slot.target = target.cloned();
        let done = Arc::clone(&slot.done);
        encoder.map_buffer_on_submit(&slot.buffer, wgpu::MapMode::Read, .., move |result| {
            done.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
        });
    }
}

/// Times of the map's own passes, read a few frames after `frame`.
#[derive(Clone, Copy)]
pub struct GpuTime {
    pub frame: u32,
    pub ms: f64,
    pub upload_ms: f64,
    pub list_ms: f64,
    pub draw_ms: f64,
    pub mask_ms: f64,
    pub splat_ms: f64,
    pub strips_ms: f64,
}

impl GpuTime {
    /// Durations belong to individual passes. Idle time between them, or
    /// between submissions, is never included in a sum.
    fn from_pairs(frame: u32, pairs: [[u64; 2]; PARTS], used: [bool; PARTS], period: f32) -> Self {
        let ms: [f64; PARTS] = std::array::from_fn(|part| {
            if used[part] {
                pass_ms(pairs[part], period)
            } else {
                0.0
            }
        });
        Self {
            frame,
            ms: sum_ms(&ms),
            upload_ms: ms[UPLOAD],
            list_ms: ms[LIST],
            draw_ms: sum_ms(&[ms[DRAW], ms[MASK], ms[SPLAT]]),
            mask_ms: sum_ms(&[ms[MASK], ms[SPLAT]]),
            splat_ms: ms[SPLAT],
            strips_ms: ms[MASK],
        }
    }
}

/// Subtract only a pass's own start from its end. A missing or reversed
/// pair is invalid: sorting it could turn a stale sample into a duration
/// spanning several frames.
fn pass_ms(times: [u64; 2], period: f32) -> f64 {
    // Metal resolves an unavailable counter to MTLCounterErrorValue (MAX).
    if times.contains(&0) || times.contains(&u64::MAX) {
        return -1.0;
    }
    times[1]
        .checked_sub(times[0])
        .map_or(-1.0, |ticks| ticks as f64 * f64::from(period) / 1e6)
}

fn sum_ms(times: &[f64]) -> f64 {
    if times.iter().any(|&ms| ms < 0.0) {
        -1.0
    } else {
        times.iter().sum()
    }
}

/// CPU writes only: copies between GPU buffers and readbacks do not count.
#[derive(Default)]
struct Uploads {
    bytes: u64,
    full: bool,
}

impl Uploads {
    fn write(&mut self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        queue.write_buffer(buffer, 0, bytes);
    }

    fn init(
        &mut self,
        device: &wgpu::Device,
        desc: &wgpu::util::BufferInitDescriptor<'_>,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer_init(desc);
        self.bytes += buffer.size();
        buffer
    }
}

const UPLOAD: usize = 0;
const RESET: usize = 1;
const DRAW: usize = 2;
const LIST: usize = 3;
const MASK: usize = 4;
const SPLAT: usize = 5;
const PARTS: usize = 6;
const TIMING_QUERIES: u32 = (PARTS * 2) as u32;
const TIMING_BYTES: u64 = TIMING_QUERIES as u64 * std::mem::size_of::<u64>() as u64;

/// Four small readbacks in flight. A busy ring loses a timing sample, not
/// a frame; no wait for the GPU, and no extra read of the wire buffers.
struct Timing {
    slots: [TimingSlot; 4],
    active: Option<usize>,
    next: usize,
    serial: u64,
    read: u64,
}

struct TimingSlot {
    /// One counter sample buffer per slot on Metal. Each pass owns two
    /// indices in it, without multiplying counter buffers across devices.
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// 0: on its way, 1: mapped, 2: mapping failed.
    done: Arc<AtomicU32>,
    target: Option<Arc<Mutex<Measurements>>>,
    used: [bool; PARTS],
    frame: u32,
    serial: u64,
}

impl Timing {
    fn new(device: &wgpu::Device) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(Self {
            slots: std::array::from_fn(|_| TimingSlot {
                queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("map pass times"),
                    ty: wgpu::QueryType::Timestamp,
                    count: TIMING_QUERIES,
                }),
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("map pass times resolved"),
                    size: TIMING_BYTES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                staging: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("map pass times read back"),
                    size: TIMING_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                done: Arc::new(AtomicU32::new(0)),
                target: None,
                used: [false; PARTS],
                frame: 0,
                serial: 0,
            }),
            active: None,
            next: 0,
            serial: 0,
            read: 0,
        })
    }

    fn poll(&mut self, device: &wgpu::Device, period: f32) {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.slots {
            let done = slot.done.load(Ordering::Acquire);
            if done == 0 {
                continue;
            }
            if let Some(target) = slot.target.take() {
                if done == 1 {
                    let gpu = {
                        let view = slot.staging.slice(..).get_mapped_range();
                        let pairs = std::array::from_fn(|k| {
                            let offset = k * 2 * std::mem::size_of::<u64>();
                            let times: &[u64] = bytemuck::cast_slice(&view[offset..offset + 16]);
                            [times[0], times[1]]
                        });
                        GpuTime::from_pairs(slot.frame, pairs, slot.used, period)
                    };
                    // Several completions can arrive together; keep the latest.
                    if slot.serial > self.read {
                        self.read = slot.serial;
                        target.lock().unwrap().gpu = Some(gpu);
                    }
                }
                slot.staging.unmap();
            }
            slot.done.store(0, Ordering::Relaxed);
        }
    }

    fn begin(&mut self, frame: u32, target: &Arc<Mutex<Measurements>>) {
        // An unfinished frame (no wire callback) has no pending readback.
        if let Some(k) = self.active.take() {
            self.slots[k].target = None;
        }
        for step in 0..self.slots.len() {
            let k = (self.next + step) % self.slots.len();
            let slot = &mut self.slots[k];
            if slot.target.is_none() {
                slot.target = Some(Arc::clone(target));
                slot.used.fill(false);
                slot.frame = frame;
                self.serial += 1;
                slot.serial = self.serial;
                self.active = Some(k);
                self.next = (k + 1) % self.slots.len();
                break;
            }
        }
    }

    fn compute(&mut self, part: usize) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let slot = self.slots.get_mut(self.active?)?;
        slot.used[part] = true;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &slot.queries,
            beginning_of_pass_write_index: Some((part * 2) as u32),
            end_of_pass_write_index: Some((part * 2 + 1) as u32),
        })
    }

    fn render(&mut self, part: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let writes = self.compute(part)?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: writes.query_set,
            beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
            end_of_pass_write_index: writes.end_of_pass_write_index,
        })
    }

    fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(k) = self.active.take() else {
            return;
        };
        self.resolve(encoder, k);
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder, k: usize) {
        let slot = &self.slots[k];
        encoder.resolve_query_set(&slot.queries, 0..TIMING_QUERIES, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.staging, 0, TIMING_BYTES);
        let done = Arc::clone(&slot.done);
        encoder.map_buffer_on_submit(&slot.staging, wgpu::MapMode::Read, .., move |result| {
            done.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
        });
    }
}

/// The GPU's side: the pipeline and the buffers, kept among egui's paint
/// callback resources.
pub struct GpuMap {
    uploads: Uploads,
    measured: Option<(usize, u32)>,
    timing: Option<Timing>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The uniforms of the image of pieces and the colours, of the
    /// changes, of the spots, and of the mask; a group over each with the wires.
    uniforms: [wgpu::Buffer; 4],
    groups: Option<[wgpu::BindGroup; 4]>,
    /// The last frame prepared, and the last painted: a frame prepared and
    /// not painted was dropped by egui (the window covered).
    prepared: Option<u32>,
    painted: AtomicU32,
    /// The wires last sent and the frame (by whichever callback of the
    /// frame came first).
    synced: Option<(usize, u32)>,
    /// The spots: their drawing, the layout of the group over their wires;
    /// the design they are of, their instances and that group.
    spots_pipeline: wgpu::RenderPipeline,
    spots_layout: wgpu::BindGroupLayout,
    spots_design: usize,
    spots: Option<(wgpu::Buffer, u32, wgpu::Buffer, wgpu::BindGroup)>,
    design: usize,
    segments: Option<(wgpu::Buffer, u32)>,
    wires: Option<(wgpu::Buffer, u64)>,
    /// Sending the wires changed: the scatter pass, its patches (and how
    /// many fit), their count, and the group over them and the wires.
    scatter: wgpu::ComputePipeline,
    scatter_layout: wgpu::BindGroupLayout,
    patches: Option<(wgpu::Buffer, u64)>,
    count: wgpu::Buffer,
    scatter_group: Option<wgpu::BindGroup>,
    /// The record's marks: each wire's first frame drawn lit since it
    /// changed, and the values it was drawn with (`seen`).
    marks: Option<[wgpu::Buffer; 2]>,
    staging: Option<wgpu::Buffer>,
    reading: Reading,
    /// The time of the last frame drawn, and of the frame whose marks are
    /// on their way back.
    drawn_at: f32,
    copied_at: f32,
    drawn_frame: u32,
    copied_frame: u32,
    drawn_pixel_glow: bool,
    copied_pixel_glow: bool,
    /// The image of pieces, drawn once a view, its drawing, and the
    /// colours drawn from it every frame (and the layout of its group).
    pieces: Option<Pieces>,
    pieces_pipeline: wgpu::RenderPipeline,
    colours: wgpu::RenderPipeline,
    colours_layout: wgpu::BindGroupLayout,
    mask_pipeline: wgpu::RenderPipeline,
    mask_listed_pipeline: wgpu::RenderPipeline,
    splat_pipeline: wgpu::ComputePipeline,
    splat_layout: wgpu::BindGroupLayout,
    /// Physical pixels; read once at startup for an A/B run.
    splat_max: f32,
    /// Drawing the changes from a list of the pieces a change may light:
    /// the pass that lists them, the drawing, their groups' layouts, the
    /// pass's uniforms; the list and the rest for the design, when the GPU
    /// can hold them; whether this frame's changes are drawn from it.
    cull: Option<wgpu::ComputePipeline>,
    cull_layout: wgpu::BindGroupLayout,
    cull_uniforms: wgpu::Buffer,
    listed_pipeline: wgpu::RenderPipeline,
    listed_layout: wgpu::BindGroupLayout,
    listed: Option<Listed>,
    from_list: bool,
    glow: Glow,
    record_lit: bool,
    listing_reads: ListingReads,
    /// Starting the record's marks of some wires again: the pass, its
    /// layout, how many, the wires (and how many fit), its group.
    reset: wgpu::ComputePipeline,
    reset_layout: wgpu::BindGroupLayout,
    reset_count: wgpu::Buffer,
    resets: Option<(wgpu::Buffer, u64)>,
    reset_group: Option<wgpu::BindGroup>,
}

impl GpuMap {
    /// Sets the GPU's side up, if the GPU can record what it draws from a
    /// fragment shader (else the CPU draws the wires).
    pub fn install(state: &egui_wgpu::RenderState) -> bool {
        let caps = state.adapter.get_downlevel_capabilities();
        let device = &state.device;
        // Errors said, not left to a logger nobody set up.
        device.on_uncaptured_error(std::sync::Arc::new(|e| eprintln!("wgpu: {e}")));
        device.set_device_lost_callback(|reason, message| {
            eprintln!("wgpu: the GPU device is lost ({reason:?}): {message}")
        });
        match Self::new(device, &caps, state.target_format) {
            Some(map) => {
                state.renderer.write().callback_resources.insert(map);
                true
            }
            None => false,
        }
    }

    /// The pipelines and buffers for drawing into `format`, if the GPU can
    /// record what it draws from a fragment shader.
    fn new(
        device: &wgpu::Device,
        caps: &wgpu::DownlevelCapabilities,
        format: wgpu::TextureFormat,
    ) -> Option<Self> {
        if !caps
            .flags
            .contains(wgpu::DownlevelFlags::FRAGMENT_WRITABLE_STORAGE)
        {
            return None;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map wires"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT
                | if binding == 2 {
                    wgpu::ShaderStages::COMPUTE
                } else {
                    wgpu::ShaderStages::empty()
                },
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wires"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(2, false),
                storage(3, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map wires"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map wires"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Segment>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let read_only = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let listed_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wires listed"),
            entries: &[
                read_only(0, wgpu::ShaderStages::VERTEX),
                read_only(1, wgpu::ShaderStages::VERTEX),
            ],
        });
        let listed_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map wires listed"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("map wires listed"),
                    bind_group_layouts: &[Some(&layout), Some(&listed_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_listed"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let cull_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let cull_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wires cull"),
            entries: &[
                cull_entry(0, wgpu::BufferBindingType::Uniform),
                cull_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                cull_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                cull_entry(3, wgpu::BufferBindingType::Storage { read_only: true }),
                cull_entry(4, wgpu::BufferBindingType::Storage { read_only: false }),
                cull_entry(5, wgpu::BufferBindingType::Storage { read_only: false }),
                cull_entry(6, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        // Without indirect draws the changes are drawn over all the pieces.
        let cull = caps
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
            .then(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("map wires cull"),
                    source: wgpu::ShaderSource::Wgsl(CULL.into()),
                });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("map wires cull"),
                    layout: Some(
                        &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("map wires cull"),
                            bind_group_layouts: &[Some(&cull_layout)],
                            immediate_size: 0,
                        }),
                    ),
                    module: &module,
                    entry_point: Some("cull"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
        let cull_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map wires cull"),
            size: std::mem::size_of::<CullUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let scatter_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map wires scatter"),
            source: wgpu::ShaderSource::Wgsl(SCATTER.into()),
        });
        let compute_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let scatter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wires scatter"),
            entries: &[
                compute_entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
                compute_entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
                compute_entry(2, wgpu::BufferBindingType::Uniform),
            ],
        });
        let scatter = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("map wires scatter"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("map wires scatter"),
                    bind_group_layouts: &[Some(&scatter_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &scatter_module,
            entry_point: Some("scatter"),
            compilation_options: Default::default(),
            cache: None,
        });
        let count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map wires scatter"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let reset_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wires reset"),
            entries: &[
                compute_entry(2, wgpu::BufferBindingType::Uniform),
                compute_entry(3, wgpu::BufferBindingType::Storage { read_only: true }),
                compute_entry(4, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let reset = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("map wires reset"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("map wires reset"),
                    bind_group_layouts: &[Some(&reset_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &scatter_module,
            entry_point: Some("reset"),
            compilation_options: Default::default(),
            cache: None,
        });
        let reset_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map wires reset"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The image of pieces: a wire and a share a pixel, the piece drawn
        // last on top.
        let pieces_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map wire pieces image"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Segment>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_id"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PIECES_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // The colours from it, a pass over the map's pixels.
        let colours_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map wire colours"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // Long pieces use raster coverage and MAX blending; the mask
        // carries only a glow, never values or widened strokes.
        let mask_pipeline = |listed| {
            let layouts = [Some(&layout), Some(&listed_layout)];
            let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32];
            let buffers = [wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Segment>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attributes,
            }];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("map glow mask"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("map glow mask"),
                        bind_group_layouts: &layouts[..if listed { 2 } else { 1 }],
                        immediate_size: 0,
                    }),
                ),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(if listed { "vs_mask_listed" } else { "vs_mask" }),
                    compilation_options: Default::default(),
                    buffers: if listed { &[] } else { &buffers },
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_mask"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: MASK_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Max,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Max,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::RED,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mask_listed_pipeline = mask_pipeline(true);
        let mask_pipeline = mask_pipeline(false);
        let splat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map mask splats"),
            entries: &[0, 1, 5, 6, 7, 8].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage {
                        read_only: matches!(binding, 0 | 1 | 6),
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let splat_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("map mask splats"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("map mask splats"),
                    bind_group_layouts: &[Some(&layout), Some(&splat_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("splat"),
            compilation_options: Default::default(),
            cache: None,
        });
        let colours = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map wire colours"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("map wire colours"),
                    bind_group_layouts: &[Some(&layout), Some(&colours_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_colours"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // The spots: a quad an instance, their wires read from a buffer of
        // their own.
        let spots_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map spots"),
            entries: &[read_only(3, wgpu::ShaderStages::VERTEX)],
        });
        let spots_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map spots"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("map spots"),
                bind_group_layouts: &[Some(&layout), Some(&spots_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_spot"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Spot>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32, 3 => Uint32, 4 => Uint32, 5 => Uint32],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_spot"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let map = GpuMap {
            uploads: Uploads::default(),
            measured: None,
            timing: Timing::new(device),
            pipeline,
            layout,
            uniforms: [uniform(), uniform(), uniform(), uniform()],
            groups: None,
            synced: None,
            prepared: None,
            painted: AtomicU32::new(0),
            spots_pipeline,
            spots_layout,
            spots_design: 0,
            spots: None,
            design: 0,
            segments: None,
            wires: None,
            scatter,
            scatter_layout,
            patches: None,
            count,
            scatter_group: None,
            marks: None,
            staging: None,
            reading: Reading::Idle,
            drawn_at: 0.0,
            copied_at: 0.0,
            drawn_frame: 0,
            copied_frame: 0,
            drawn_pixel_glow: false,
            copied_pixel_glow: false,
            pieces: None,
            pieces_pipeline,
            colours,
            colours_layout,
            mask_pipeline,
            mask_listed_pipeline,
            splat_pipeline,
            splat_layout,
            splat_max: splat_max(std::env::var("ECP5_SPLAT_MAX").ok().as_deref()),
            cull,
            cull_layout,
            cull_uniforms,
            listed_pipeline,
            listed_layout,
            listed: None,
            from_list: false,
            glow: Glow::Quads,
            record_lit: false,
            listing_reads: ListingReads::new(device),
            reset,
            reset_layout,
            reset_count,
            resets: None,
            reset_group: None,
        };
        Some(map)
    }

    /// Both callbacks share the frame's writes and queries; the spots may
    /// send the wires first, before the wire callback lists their pieces.
    fn begin_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        states: &Mutex<WireStates>,
        frame: u32,
        measure: Option<&Arc<Mutex<Measurements>>>,
    ) {
        let key = (states as *const Mutex<WireStates> as usize, frame);
        if self.measured == Some(key) {
            return;
        }
        self.measured = Some(key);
        self.uploads.bytes = 0;
        self.uploads.full = false;
        if let (Some(timing), Some(target)) = (&mut self.timing, measure) {
            timing.poll(device, queue.get_timestamp_period());
            timing.begin(frame, target);
        }
    }

    fn end_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        measure: Option<&Arc<Mutex<Measurements>>>,
    ) {
        if let Some(target) = measure {
            let mut shown = target.lock().unwrap();
            shown.upload_frame = self.measured.map(|(_, frame)| frame);
            shown.upload_bytes = self.uploads.bytes;
            shown.full_upload = self.uploads.full;
        }
        if let Some(timing) = &mut self.timing {
            timing.finish(encoder);
        }
    }

    /// The buffers for `n` wires, and the bind groups over them; whether
    /// they are new (and the wires all to be sent).
    fn size(&mut self, device: &wgpu::Device, n: u64) -> bool {
        if self.wires.as_ref().is_some_and(|w| w.1 == n) {
            return false;
        }
        let bytes = n.max(1) * std::mem::size_of::<Record>() as u64;
        let wires = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map wires"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mark = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires drawn"),
                size: n.max(1) * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let marks = [mark(), mark()];
        let group = |k: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wires"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms[k].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wires.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: marks[0].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: marks[1].as_entire_binding(),
                    },
                ],
            })
        };
        self.groups = Some([group(0), group(1), group(2), group(3)]);
        self.staging = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map wires read back"),
            size: n.max(1) * 8 + 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.marks = Some(marks);
        self.wires = Some((wires, n));
        self.scatter_group = None;
        self.listed = None;
        self.reset_group = None;
        self.reading = Reading::Idle;
        true
    }

    /// The wires' changes since they were last sent, once a frame (the
    /// first callback of frame `frame` sends them), into `encoder`.
    fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        states: &Mutex<WireStates>,
        frame: u32,
    ) {
        let at = (states as *const Mutex<WireStates> as usize, frame);
        if self.synced == Some(at) && self.wires.is_some() {
            return;
        }
        self.synced = Some(at);
        let mut wires = states.lock().unwrap();
        let n = wires.states.len() as u64;
        let new = self.size(device, n);
        let (all, patches) = wires.take();
        if all || new || full_upload(patches.len() as u64, n) || knob("ECP5_GUI_FULL_UPLOAD") {
            if let Some((buffer, _)) = &self.wires {
                self.uploads.full = true;
                self.uploads
                    .write(queue, buffer, bytemuck::cast_slice(&wires.states));
            }
        } else if !patches.is_empty() {
            self.send(device, queue, encoder, &patches);
        }
        wires.give_back(patches);
    }

    /// The uniforms of drawing `k` (0: the image of pieces and the colours,
    /// 1: the changes, 2: the spots, 3: the glow mask).
    #[allow(clippy::too_many_arguments)]
    fn uniforms(
        &mut self,
        queue: &wgpu::Queue,
        k: usize,
        view: (egui::Vec2, egui::Vec2, f32, f32),
        now: f32,
        ppp: f32,
        values: bool,
        record: bool,
        frame: u32,
    ) {
        let (origin, size, cell, width) = view;
        let u = Uniforms {
            origin: [origin.x, origin.y],
            size: [size.x, size.y],
            cell,
            width,
            now,
            ppp,
            values: u32::from(values),
            layer: k as u32,
            record: u32::from(record),
            cull: u32::from(!knob("ECP5_GUI_NOCULL")),
            frame,
            flat: u32::from(knob("ECP5_GUI_FLAT_CHANGES")),
            glow_mode: if knob("ECP5_GUI_NO_CHANGES") {
                0
            } else {
                self.glow as u32
            },
            record_lit: u32::from(self.record_lit),
            mask_size: self.pieces.as_ref().map_or([1, 1], |p| p.size),
            splat_max: self.splat_max,
            mask_pad: 0,
            used: rgba(crate::fabric_view::USED),
            one: rgba(crate::fabric_view::ONE),
            zero: rgba(crate::fabric_view::ZERO),
            unknown: rgba(crate::fabric_view::UNKNOWN),
            change: rgba(crate::fabric_view::CHANGE),
        };
        self.uploads
            .write(queue, &self.uniforms[k], bytemuck::bytes_of(&u));
    }

    /// Sends the states of some wires: their patches, scattered into place
    /// by a compute pass.
    fn send(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        patches: &[Patch],
    ) {
        let Some((wires, _)) = &self.wires else {
            return;
        };
        let n = patches.len() as u64;
        if self.patches.as_ref().is_none_or(|p| p.1 < n) {
            let room = n.next_power_of_two().max(1024);
            self.patches = Some((
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("map wires patches"),
                    size: room * std::mem::size_of::<Patch>() as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                room,
            ));
            self.scatter_group = None;
        }
        let Some((buffer, _)) = &self.patches else {
            return;
        };
        if self.scatter_group.is_none() {
            self.scatter_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wires scatter"),
                layout: &self.scatter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wires.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.count.as_entire_binding(),
                    },
                ],
            }));
        }
        self.uploads
            .write(queue, buffer, bytemuck::cast_slice(patches));
        self.uploads.write(
            queue,
            &self.count,
            bytemuck::cast_slice(&[n as u32, 0, 0, 0]),
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("map wires scatter"),
            timestamp_writes: self.timing.as_mut().and_then(|t| t.compute(UPLOAD)),
        });
        pass.set_pipeline(&self.scatter);
        pass.set_bind_group(0, self.scatter_group.as_ref(), &[]);
        pass.dispatch_workgroups(n.div_ceil(64) as u32, 1, 1);
    }

    /// Starts the record's marks of the wires `resets` again.
    fn reset(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        resets: &[u32],
    ) {
        let Some(marks) = &self.marks else {
            return;
        };
        let n = resets.len() as u64;
        if n == 0 {
            return;
        }
        if self.resets.as_ref().is_none_or(|r| r.1 < n) {
            let room = n.next_power_of_two().max(1024);
            self.resets = Some((
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("map wires reset"),
                    size: room * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                room,
            ));
            self.reset_group = None;
        }
        let Some((buffer, _)) = &self.resets else {
            return;
        };
        if self.reset_group.is_none() {
            self.reset_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wires reset"),
                layout: &self.reset_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.reset_count.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: marks[0].as_entire_binding(),
                    },
                ],
            }));
        }
        self.uploads
            .write(queue, buffer, bytemuck::cast_slice(resets));
        self.uploads.write(
            queue,
            &self.reset_count,
            bytemuck::cast_slice(&[0, n as u32, 0, 0]),
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("map wires reset"),
            timestamp_writes: self.timing.as_mut().and_then(|t| t.compute(RESET)),
        });
        pass.set_pipeline(&self.reset);
        pass.set_bind_group(0, self.reset_group.as_ref(), &[]);
        pass.dispatch_workgroups(n.div_ceil(64) as u32, 1, 1);
    }

    /// Draws the image of pieces (made anew when its size changes): which
    /// piece is on top at each pixel.
    fn draw_pieces(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        key: Vec<u64>,
        ranges: &[std::ops::Range<u32>],
    ) {
        let (Some((segments, n)), Some(groups)) = (&self.segments, &self.groups) else {
            return;
        };
        if self.pieces.as_ref().is_none_or(|c| c.size != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("map wire pieces image"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PIECES_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let mask = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("map glow mask"),
                size: texture.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let splats = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map mask pixels"),
                size: u64::from(size[0]) * u64::from(size[1]) * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mask_view = mask.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wire colours"),
                layout: &self.colours_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: splats.as_entire_binding(),
                    },
                ],
            });
            self.pieces = Some(Pieces {
                texture,
                mask,
                splats,
                group,
                size,
                key: Vec::new(),
            });
        }
        let Some(pieces) = &mut self.pieces else {
            return;
        };
        let view = pieces.texture.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("map wire pieces image"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.timing.as_mut().and_then(|t| t.render(DRAW)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pieces_pipeline);
            pass.set_vertex_buffer(0, segments.slice(..));
            pass.set_bind_group(0, &groups[0], &[]);
            for range in ranges {
                pass.draw(0..4, range.start.min(*n)..range.end.min(*n));
            }
        }
        pieces.key = key;
    }

    /// The greatest glow at each pixel, including pieces hidden beneath
    /// the image's top wire. Both paths mark a contributing wire, even
    /// when another piece puts a greater glow there too. The strips-only
    /// path remains a reference for the windowless comparison.
    fn draw_mask(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        ranges: &[std::ops::Range<u32>],
        hybrid: bool,
    ) {
        let hybrid = hybrid && self.from_list;
        if let Some(pieces) = &self.pieces {
            encoder.clear_buffer(&pieces.splats, 0, None);
        }
        if hybrid {
            self.splat_mask(device, encoder);
        }
        let (Some(pieces), Some(groups), Some((segments, n))) =
            (&self.pieces, &self.groups, &self.segments)
        else {
            return;
        };
        let view = pieces.mask.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map glow mask"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: self.timing.as_mut().and_then(|t| t.render(MASK)),
            ..Default::default()
        });
        pass.set_bind_group(0, &groups[3], &[]);
        if let (true, Some(listed)) = (self.from_list, &self.listed) {
            pass.set_pipeline(&self.mask_listed_pipeline);
            if let (true, Some(mask)) = (hybrid, &listed.mask) {
                pass.set_bind_group(1, &mask.draw_group, &[]);
                pass.draw_indirect(&mask.args, 0);
            } else {
                pass.set_bind_group(1, &listed.draw_group, &[]);
                pass.draw_indirect(&listed.args, 0);
            }
        } else {
            pass.set_pipeline(&self.mask_pipeline);
            pass.set_vertex_buffer(0, segments.slice(..));
            for range in ranges {
                pass.draw(0..4, range.start.min(*n)..range.end.min(*n));
            }
        }
    }

    /// One compute thread per listed piece, rather than four vertices and
    /// a primitive per speck. Compact only the long pieces for the draw.
    fn splat_mask(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let (Some(listed), Some(pieces), Some((segments, n)), Some(groups)) =
            (&mut self.listed, &self.pieces, &self.segments, &self.groups)
        else {
            return;
        };
        if listed
            .mask
            .as_ref()
            .is_none_or(|m| m.pixels != pieces.splats)
        {
            let list = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map mask long pieces"),
                size: u64::from((*n).max(1)) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let args = self.uploads.init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("map mask long arguments"),
                    contents: bytemuck::cast_slice(&[4u32, 0, 0, 0]),
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::INDIRECT
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                },
            );
            let draw_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map mask long pieces"),
                layout: &self.listed_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: segments.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: list.as_entire_binding(),
                    },
                ],
            });
            let splat_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map mask splats"),
                layout: &self.splat_layout,
                entries: &[
                    (0, segments),
                    (1, &listed._list),
                    (5, &pieces.splats),
                    (6, &listed.args),
                    (7, &list),
                    (8, &args),
                ]
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding,
                    resource: buffer.as_entire_binding(),
                }),
            });
            listed.mask = Some(MaskList {
                _list: list,
                args,
                draw_group,
                pixels: pieces.splats.clone(),
                splat_group,
            });
        }
        let mask = listed.mask.as_ref().unwrap();
        encoder.clear_buffer(&mask.args, 4, Some(4));
        if listed.total > 0 {
            let count = listed.total.div_ceil(256);
            let x = count.min(65535);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("map mask splats"),
                timestamp_writes: self.timing.as_mut().and_then(|t| t.compute(SPLAT)),
            });
            pass.set_pipeline(&self.splat_pipeline);
            pass.set_bind_group(0, &groups[3], &[]);
            pass.set_bind_group(1, &mask.splat_group, &[]);
            pass.dispatch_workgroups(x, count.div_ceil(x), 1);
        }
    }

    /// Lists the pieces in `ranges` a change may light at `now`, for the
    /// changes to be drawn from the list; false where it cannot be (no
    /// indirect draws, or the pieces too many for one buffer the shaders
    /// read).
    fn list(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        now: f32,
        ranges: &[std::ops::Range<u32>],
        record: bool,
    ) -> bool {
        let (Some(cull), Some((segments, n)), Some((wires, _)), Some(marks)) =
            (&self.cull, &self.segments, &self.wires, &self.marks)
        else {
            return false;
        };
        let n = *n;
        let bytes = n as u64 * std::mem::size_of::<Segment>() as u64;
        if bytes > device.limits().max_storage_buffer_binding_size || ranges.len() as u64 > RUNS {
            return false;
        }
        if self.listed.is_none() {
            let list = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires listed"),
                size: n.max(1) as u64 * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let args = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires listed"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let runs = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires listed"),
                size: RUNS * 8,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let cull_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wires cull"),
                layout: &self.cull_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.cull_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: segments.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wires.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: runs.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: list.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: args.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: marks[0].as_entire_binding(),
                    },
                ],
            });
            let draw_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map wires listed"),
                layout: &self.listed_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: segments.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: list.as_entire_binding(),
                    },
                ],
            });
            let count = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map wires listed count"),
                size: 4,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.listed = Some(Listed {
                _list: list,
                args,
                count,
                runs,
                cull_group,
                draw_group,
                total: 0,
                mask: None,
            });
        }
        let Some(listed) = &mut self.listed else {
            return false;
        };
        // The runs: each one's first piece and the threads before it.
        let mut runs: Vec<[u32; 2]> = Vec::with_capacity(ranges.len().max(1));
        let mut total = 0u32;
        for range in ranges {
            let (start, end) = (range.start.min(n), range.end.min(n));
            if start < end {
                runs.push([start, total]);
                total += end - start;
            }
        }
        if runs.is_empty() {
            runs.push([0, 0]);
        }
        listed.total = total;
        self.uploads
            .write(queue, &listed.runs, bytemuck::cast_slice(&runs));
        self.uploads.write(
            queue,
            &self.cull_uniforms,
            bytemuck::bytes_of(&CullUniforms {
                now,
                runs: runs.len() as u32,
                total,
                record: u32::from(record),
            }),
        );
        // Four corners a piece, none listed yet.
        self.uploads
            .write(queue, &listed.args, bytemuck::cast_slice(&[4u32, 0, 0, 0]));
        if total > 0 {
            let groups = total.div_ceil(256);
            let x = groups.min(65535);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("map wires cull"),
                timestamp_writes: self.timing.as_mut().and_then(|t| t.compute(LIST)),
            });
            pass.set_pipeline(cull);
            pass.set_bind_group(0, &listed.cull_group, &[]);
            pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
        }
        encoder.copy_buffer_to_buffer(&listed.args, 4, &listed.count, 0, 4);
        true
    }

    /// One step of reading back what was drawn: a copy of the marks made
    /// (then cleared), mapped once the copy has gone, read once mapped.
    fn read_back(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<Readback> {
        let (Some(marks), Some(staging), Some((_, n))) = (&self.marks, &self.staging, &self.wires)
        else {
            return None;
        };
        let n = *n as usize;
        let size = (n * 4) as u64;
        match &self.reading {
            Reading::Idle => {
                for (k, m) in marks.iter().enumerate() {
                    encoder.copy_buffer_to_buffer(m, 0, staging, k as u64 * size, size);
                }
                // The pieces the frame read listed (its pass's count, kept
                // by its commands after the pass).
                if let Some(listed) = &self.listed {
                    encoder.copy_buffer_to_buffer(&listed.count, 0, staging, 2 * size, 4);
                }
                self.copied_at = self.drawn_at;
                self.copied_frame = self.drawn_frame;
                self.copied_pixel_glow = self.drawn_pixel_glow;
                self.reading = Reading::Copied;
            }
            Reading::Copied => {
                let done = Arc::new(AtomicBool::new(false));
                let flag = Arc::clone(&done);
                staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    if r.is_ok() {
                        flag.store(true, Ordering::Release);
                    }
                });
                self.reading = Reading::Mapping(done);
            }
            Reading::Mapping(done) => {
                let _ = device.poll(wgpu::PollType::Poll);
                if done.load(Ordering::Acquire) {
                    let read = {
                        let view = staging.slice(..).get_mapped_range();
                        let words: &[u32] = bytemuck::cast_slice(&view);
                        Readback {
                            at: self.copied_at as f64,
                            of: self.copied_frame,
                            lit: words[..n].to_vec(),
                            seen: words[n..2 * n].to_vec(),
                            listed: words[2 * n],
                            pixel_glow: self.copied_pixel_glow,
                        }
                    };
                    staging.unmap();
                    self.reading = Reading::Idle;
                    return Some(read);
                }
            }
        }
        None
    }
}

/// What a frame asks of the GPU.
pub struct MapCallback {
    pub design: usize,
    /// The pieces, sent with the first frame of a design.
    pub segments: Option<Arc<Vec<Segment>>>,
    /// The pieces to draw: those of the tiles in sight, a run of them a
    /// row of tiles.
    pub ranges: Vec<std::ops::Range<u32>>,
    /// The wires' changes; those changed since are sent.
    pub wires: Arc<Mutex<WireStates>>,
    /// Where the map is, in points.
    pub rect: egui::Rect,
    /// The die's (0, 0) from the map's top left and the map's size, in
    /// points; points per die unit; a wire's width; the clock; 0/1.
    pub origin: egui::Vec2,
    pub size: egui::Vec2,
    pub cell: f32,
    pub width: f32,
    pub now: f32,
    pub values: bool,
    pub glow: GlowChoice,
    /// The number of the frame, for the record's marks, and the wires
    /// whose marks start again (the record waits on a change of theirs).
    pub frame: u32,
    pub resets: Vec<u32>,
    /// Where to put what was drawn, read back, when a record is kept.
    pub record: Option<Arc<Mutex<Option<Readback>>>>,
    pub measure: Option<Arc<Mutex<Measurements>>>,
    /// Counts the frames prepared that egui then did not draw.
    pub dropped: Arc<AtomicU32>,
}

// Chosen from the life_full fit benchmark and window records on the M3 Max.
const SPLAT_MAX: f32 = 24.0;

fn splat_max(value: Option<&str>) -> f32 {
    value
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(SPLAT_MAX)
}

/// A switch for measuring one way of drawing against another.
fn knob(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn rgba(c: Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_array();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

impl CallbackTrait for MapCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(map) = resources.get_mut::<GpuMap>() else {
            return Vec::new();
        };
        map.begin_frame(
            device,
            queue,
            &self.wires,
            self.frame,
            self.measure.as_ref(),
        );
        // The map's commands, submitted here: egui drops its own encoder,
        // and what it holds, when the window cannot be drawn this frame.
        let mut own = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("map wires"),
        });
        let encoder = &mut own;
        if let Some(p) = map.prepared {
            if map.painted.load(Ordering::Relaxed) != p {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        map.prepared = Some(self.frame);
        if map.design != self.design {
            map.segments = None;
        }
        if let (None, Some(segments)) = (&map.segments, &self.segments) {
            let buffer = map.uploads.init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("map wire pieces"),
                    contents: bytemuck::cast_slice(segments),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
                },
            );
            map.segments = Some((buffer, segments.len() as u32));
            map.design = self.design;
            map.listed = None;
        }
        map.sync(device, queue, encoder, &self.wires, self.frame);
        // The image of pieces is drawn again when the view or the width
        // change; the values only change the colours drawn from it.
        let px = egui::epaint::ViewportInPixels::from_points(
            &self.rect,
            screen.pixels_per_point,
            screen.size_in_pixels,
        );
        let size = [px.width_px.max(1) as u32, px.height_px.max(1) as u32];
        let key = vec![
            self.design as u64,
            u64::from(self.origin.x.to_bits()) << 32 | u64::from(self.origin.y.to_bits()),
            u64::from(self.size.x.to_bits()) << 32 | u64::from(self.size.y.to_bits()),
            u64::from(self.cell.to_bits()) << 32 | u64::from(self.width.to_bits()),
            u64::from(screen.pixels_per_point.to_bits()),
        ];
        let stale = map
            .pieces
            .as_ref()
            .is_none_or(|c| c.key != key || c.size != size)
            || knob("ECP5_GUI_NO_COLOUR_CACHE");
        map.listing_reads.poll(device);
        // The latest count for this view, or its visible pieces as a cheap
        // upper bound until the first count arrives. No scan of wire states.
        let count = map
            .listing_reads
            .latest
            .as_ref()
            .filter(|(view, _)| *view == key)
            .map_or_else(
                || {
                    let n = map.segments.as_ref().map_or(0, |(_, n)| *n);
                    self.ranges
                        .iter()
                        .map(|r| r.end.min(n).saturating_sub(r.start.min(n)))
                        .sum()
                },
                |(_, mode)| mode.listed,
            );
        map.glow = self
            .glow
            .choose(map.glow, count, self.cell, screen.pixels_per_point);
        // For measuring the frames alone: the record without what it reads.
        let record = self.record.as_ref().filter(|_| !knob("ECP5_GUI_NO_MARKS"));
        map.record_lit = record.is_some() && !knob("ECP5_GUI_NO_LIT_MARKS");
        if record.is_some() {
            map.reset(device, queue, encoder, &self.resets);
        }
        if let Some(slot) = record {
            // The marks of the frame before (its colours and changes were
            // drawn after this encoder's commands of then), copied first.
            if let Some(read) = map.read_back(device, encoder) {
                *slot.lock().unwrap() = Some(read);
            }
            // The values drawn are marked in the frames whose marks are
            // copied next (in the next frame's commands, when no read is on
            // its way), anew each time; a wire's mark of being drawn lit
            // starts again when it changes (`reset`).
            if matches!(map.reading, Reading::Idle) {
                if let Some(marks) = &map.marks {
                    encoder.clear_buffer(&marks[1], 0, None);
                }
            }
        }
        let read_next = record.is_some() && matches!(map.reading, Reading::Idle);
        map.drawn_at = self.now;
        map.drawn_frame = self.frame;
        map.drawn_pixel_glow = map.glow == Glow::Top;
        for k in 0..2 {
            map.uniforms(
                queue,
                k,
                (self.origin, self.size, self.cell, self.width),
                self.now,
                screen.pixels_per_point,
                self.values,
                if k == 0 {
                    read_next && (map.glow == Glow::Top || !knob("ECP5_GUI_NO_VALUE_MARKS"))
                } else {
                    record.is_some() && !knob("ECP5_GUI_NO_LIT_MARKS")
                },
                self.frame,
            );
        }
        if stale {
            map.draw_pieces(device, encoder, size, key.clone(), &self.ranges);
        }
        map.from_list = !knob("ECP5_GUI_NO_LIST")
            && map.list(
                device,
                queue,
                encoder,
                self.now,
                &self.ranges,
                record.is_some(),
            );
        if map.glow == Glow::Mask && !knob("ECP5_GUI_NO_CHANGES") {
            map.uniforms(
                queue,
                3,
                (self.origin, self.size, self.cell, self.width),
                self.now,
                screen.pixels_per_point,
                self.values,
                map.record_lit,
                self.frame,
            );
            map.draw_mask(device, encoder, &self.ranges, true);
        }
        if let (true, Some(listed)) = (map.from_list, &map.listed) {
            map.listing_reads.copy(
                encoder,
                &listed.count,
                key,
                GlowMode {
                    frame: self.frame,
                    choice: self.glow,
                    mode: map.glow,
                    listed: 0,
                },
                self.measure.as_ref(),
            );
        } else if let Some(target) = &self.measure {
            // No indirect listing on this adapter: the estimate still
            // identifies the mode, with no readback to wait on.
            target.lock().unwrap().glow = Some(GlowMode {
                frame: self.frame,
                choice: self.glow,
                mode: map.glow,
                listed: count,
            });
        }
        map.end_frame(encoder, self.measure.as_ref());
        queue.submit([own.finish()]);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(map) = resources.get::<GpuMap>() else {
            return;
        };
        map.painted.store(self.frame, Ordering::Relaxed);
        let (Some((segments, n)), Some(groups)) = (&map.segments, &map.groups) else {
            return;
        };
        // The colours, from the image of pieces drawn once a view; then the
        // changes over them.
        if let Some(pieces) = &map.pieces {
            pass.set_pipeline(&map.colours);
            pass.set_bind_group(0, &groups[0], &[]);
            pass.set_bind_group(1, &pieces.group, &[]);
            pass.draw(0..3, 0..1);
        }
        // For measuring: the colours only.
        if map.glow != Glow::Quads || knob("ECP5_GUI_NO_CHANGES") {
            return;
        }
        if let (true, Some(listed)) = (map.from_list, &map.listed) {
            pass.set_pipeline(&map.listed_pipeline);
            pass.set_bind_group(0, &groups[1], &[]);
            pass.set_bind_group(1, &listed.draw_group, &[]);
            pass.draw_indirect(&listed.args, 0);
            return;
        }
        pass.set_pipeline(&map.pipeline);
        pass.set_vertex_buffer(0, segments.slice(..));
        pass.set_bind_group(0, &groups[1], &[]);
        for range in &self.ranges {
            pass.draw(0..4, range.start.min(*n)..range.end.min(*n));
        }
    }
}

/// What a frame asks of the GPU for the spots (`Spot`): drawn under the
/// sites the CPU draws closer, and under the wires.
pub struct SpotsCallback {
    pub design: usize,
    /// The design's spots, sent with the first frame of a design.
    pub spots: Option<Arc<Spots>>,
    /// The spots to draw: a run of them a row of tiles, for each kind.
    pub ranges: Vec<std::ops::Range<u32>>,
    /// The wires' changes, sent by whichever of the frame's callbacks
    /// comes first.
    pub wires: Arc<Mutex<WireStates>>,
    /// The die's (0, 0) from the map's top left and the map's size, in
    /// points; points per die unit; the clock; 0/1; the frame.
    pub origin: egui::Vec2,
    pub size: egui::Vec2,
    pub cell: f32,
    pub now: f32,
    pub values: bool,
    pub frame: u32,
    pub measure: Option<Arc<Mutex<Measurements>>>,
    /// With no wires drawn, this is the frame's last map callback.
    pub last: bool,
}

impl CallbackTrait for SpotsCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(map) = resources.get_mut::<GpuMap>() else {
            return Vec::new();
        };
        map.begin_frame(
            device,
            queue,
            &self.wires,
            self.frame,
            self.measure.as_ref(),
        );
        let mut own = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("map spots"),
        });
        map.sync(device, queue, &mut own, &self.wires, self.frame);
        if map.spots_design != self.design {
            map.spots = None;
        }
        if let (None, Some(spots)) = (&map.spots, &self.spots) {
            let none = [Spot::zeroed()];
            let instances = map.uploads.init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("map spots"),
                    contents: bytemuck::cast_slice(if spots.spots.is_empty() {
                        &none[..]
                    } else {
                        &spots.spots[..]
                    }),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            );
            let wires = map.uploads.init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("map spots wires"),
                    contents: bytemuck::cast_slice(if spots.wires.is_empty() {
                        &[0u32]
                    } else {
                        &spots.wires[..]
                    }),
                    usage: wgpu::BufferUsages::STORAGE,
                },
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map spots"),
                layout: &map.spots_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wires.as_entire_binding(),
                }],
            });
            map.spots = Some((instances, spots.spots.len() as u32, wires, group));
            map.spots_design = self.design;
        }
        map.uniforms(
            queue,
            2,
            (self.origin, self.size, self.cell, 1.0),
            self.now,
            screen.pixels_per_point,
            self.values,
            false,
            self.frame,
        );
        if self.last {
            map.end_frame(&mut own, self.measure.as_ref());
        }
        queue.submit([own.finish()]);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(map) = resources.get::<GpuMap>() else {
            return;
        };
        let (Some((instances, n, _, group)), Some(groups)) = (&map.spots, &map.groups) else {
            return;
        };
        pass.set_pipeline(&map.spots_pipeline);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.set_bind_group(0, &groups[2], &[]);
        pass.set_bind_group(1, group, &[]);
        for range in &self.ranges {
            let (a, b) = (range.start.min(*n), range.end.min(*n));
            if a < b {
                pass.draw(0..4, a..b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The comparison's pieces each have their wire's index. Record the share
    // error before half conversion, independently of the computed glow.
    fn mask_share_diagnostics() -> String {
        SHADER.replace(
            "    var glow = i.colour.x;",
            r#"    let p = segs[i.wire];
    let a = mask_point(vec2<f32>(p.ax, p.ay));
    let b = mask_point(vec2<f32>(p.bx, p.by));
    if (i.even == 0u && length(b - a) < u.splat_max) {
        let projected = mask_share(i.share, a, b, i.pos.xy);
        let error = abs(share_at(i) - projected);
        let pixel = vec2<u32>(i.pos.xy);
        atomicMax(&mask_pixels[pixel.y * u.mask_size.x + pixel.x], bitcast<u32>(error));
    }
    var glow = i.colour.x;"#,
        )
    }

    fn half_glow(bits: u16) -> f64 {
        let exponent = (bits >> 10) & 31;
        let mantissa = f64::from(bits & 1023);
        if exponent == 0 {
            mantissa * 2.0f64.powi(-24)
        } else {
            (1024.0 + mantissa) * 2.0f64.powi(i32::from(exponent) - 25)
        }
    }

    fn glow_error_bound(a: u16, b: u16, share_error: f32, front: bool) -> f64 {
        // On each lit side of this test's front, dg/ds = spread/fade = 0.4/0.6.
        // There is a jump at the front: the separate exact coverage assertion
        // must reject a lit/unlit mismatch. The duplicate pieces have the
        // same front, so overlap cannot hide it. MAX is non-expansive: use the
        // largest measured share error of all contributions at this pixel.
        // This measures interpolation precision rather than assuming a
        // backend's subpixel grid or picking a fixed half-float ULP count.
        let propagation = if front {
            (0.4 / 0.6) * f64::from(share_error)
        } else {
            0.0
        };
        // Four add/subtract/multiply operations and a division per glow,
        // with intermediates <= 2; allow WGSL's 2.5 ULP division accuracy.
        // Two evaluations give 2 * (4 + 2.5) * 2 * epsilon / fade.
        // With no front both paths use the identical constant-piece shortcut.
        let arithmetic = if front {
            2.0 * (4.0 + 2.5) * 2.0 * f64::from(f32::EPSILON) / 0.6
        } else {
            0.0
        };
        // WGSL conversion permits either adjacent half value, not just
        // round-to-nearest: each conversion costs at most one spacing.
        // https://www.w3.org/TR/WGSL/#floating-point-conversion
        // With the identical constant input, the two results can differ by
        // only one spacing in total. Use the larger neighbour at a binade edge.
        let spacing = |bits: u16| {
            (half_glow(bits + 1) - half_glow(bits))
                .max(half_glow(bits) - half_glow(bits.saturating_sub(1)))
        };
        let conversion = if front {
            spacing(a) + spacing(b)
        } else {
            spacing(a).max(spacing(b))
        };
        propagation + arithmetic + conversion
    }

    #[test]
    fn mask_glow_bound_propagates_share_error_and_half_rounding() {
        assert_eq!(half_glow(0), 0.0);
        assert_eq!(half_glow(1), 2.0f64.powi(-24));
        assert_eq!(half_glow(0x3c00), 1.0);
        assert_eq!(
            glow_error_bound(15066, 15068, 1.0, false),
            glow_error_bound(15066, 15068, 0.0, false)
        );
        let rounded = glow_error_bound(15066, 15068, 0.0, true);
        assert!(half_glow(15067) - half_glow(15066) <= rounded);
        assert!(half_glow(15068) - half_glow(15066) <= rounded);
        assert!(half_glow(15069) - half_glow(15066) > rounded);
        let delta = 0.002f32;
        assert!(
            (glow_error_bound(15066, 15068, delta, true)
                - rounded
                - (0.4 / 0.6) * f64::from(delta))
            .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn map_shaders_validate_without_a_gpu() {
        for source in [SCATTER, SHADER, CULL, &mask_share_diagnostics()] {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn splat_threshold_defaults_and_parses_physical_pixels() {
        assert_eq!(SPLAT_MAX, 24.0);
        assert_eq!(splat_max(None), SPLAT_MAX);
        for invalid in ["", "pixels", "0", "-1", "NaN", "inf", "-inf", "1e39"] {
            assert_eq!(splat_max(Some(invalid)), SPLAT_MAX, "{invalid}");
        }
        for (value, expected) in [("1.5", 1.5), ("3", 3.0), (" 6 ", 6.0), ("24", 24.0)] {
            assert_eq!(splat_max(Some(value)), expected);
        }
    }

    #[test]
    fn timestamps_reject_missing_or_reversed_pairs() {
        assert_eq!(pass_ms([20, 10], 1e6), -1.0);
        assert_eq!(pass_ms([10, 20], 1e6), 10.0);
        assert_eq!(pass_ms([10, 10], 1e6), 0.0);
        assert_eq!(pass_ms([0, 20], 1e6), -1.0);
        assert_eq!(pass_ms([20, 0], 1e6), -1.0);
        assert_eq!(pass_ms([0, 0], 1e6), -1.0);
        assert_eq!(pass_ms([10, u64::MAX], 1e6), -1.0);
        assert_eq!(pass_ms([u64::MAX, 20], 1e6), -1.0);
        // These passes overlap and arrive out of order. Subtracting the
        // last endpoint from the first would wrap or measure the wrong work.
        let parts = [[1000, 1020], [40, 60], [800, 900]].map(|pair| pass_ms(pair, 1e6));
        assert_eq!(sum_ms(&parts), 140.0);
        assert_eq!(sum_ms(&[0.0, -1.0, 10.0]), -1.0);
        assert_eq!(sum_ms(&[0.0, 0.0]), 0.0);
    }

    #[test]
    fn timing_totals_exclude_gaps_between_passes() {
        let mut pairs = [[0; 2]; PARTS];
        let mut used = [false; PARTS];
        pairs[UPLOAD] = [10, 11];
        pairs[LIST] = [70, 74];
        pairs[SPLAT] = [100, 103];
        pairs[MASK] = [150, 152];
        for part in [UPLOAD, LIST, SPLAT, MASK] {
            used[part] = true;
        }
        let gpu = GpuTime::from_pairs(42, pairs, used, 1e6);
        assert_eq!(gpu.splat_ms, 3.0);
        assert_eq!(gpu.strips_ms, 2.0);
        assert_eq!(gpu.mask_ms, 5.0); // Not 52 ms: the gap is not mask work.
        assert_eq!(gpu.draw_ms, 5.0);
        assert_eq!(gpu.ms, 10.0); // Not 142 ms from the first to last pass.
        for invalid in [[0, 152], [150, 0], [152, 150]] {
            pairs[MASK] = invalid;
            let gpu = GpuTime::from_pairs(43, pairs, used, 1e6);
            assert_eq!(gpu.splat_ms, 3.0);
            assert_eq!(gpu.strips_ms, -1.0);
            assert_eq!(gpu.mask_ms, -1.0);
            assert_eq!(gpu.ms, -1.0);
        }
        used[MASK] = false;
        let gpu = GpuTime::from_pairs(44, pairs, used, 1e6);
        assert_eq!(gpu.strips_ms, 0.0);
        assert_eq!(gpu.mask_ms, gpu.splat_ms);
    }

    #[test]
    fn dense_glow_switches_only_from_afar_and_has_hysteresis() {
        assert!(!pixel_glow(false, 249_999, 2.0, 2.0));
        assert!(pixel_glow(false, 250_000, 2.0, 2.0));
        assert!(pixel_glow(true, 125_000, 2.0, 2.0));
        assert!(!pixel_glow(true, 124_999, 2.0, 2.0));
        assert!(pixel_glow(false, 1_000_000, 8.0, 2.0));
        assert!(!pixel_glow(true, 1_000_000, 8.01, 2.0));
        // The reported fit and 8x views, including a display at 1x.
        assert!(pixel_glow(false, 5_800_000, 2.2, 2.0));
        assert!(!pixel_glow(true, 5_800_000, 17.6, 1.0));
        assert!(!pixel_glow(false, 1, 2.2, 2.0));
        for (choice, dense) in [
            (GlowChoice::Auto, Glow::Mask),
            (GlowChoice::Mask, Glow::Mask),
            (GlowChoice::Top, Glow::Top),
            (GlowChoice::Quads, Glow::Quads),
        ] {
            assert_eq!(choice.choose(Glow::Quads, 250_000, 2.0, 2.0), dense);
            assert_eq!(choice.choose(dense, 125_000, 2.0, 2.0), dense);
            assert_eq!(choice.choose(dense, 124_999, 2.0, 2.0), Glow::Quads);
            assert_eq!(choice.choose(dense, 5_800_000, 17.6, 1.0), Glow::Quads);
        }
    }

    #[test]
    fn falling_behind_keeps_uploading_patches() {
        let wires = 1_350_000;
        assert!(!full_upload(380_000, wires));
        assert!(!full_upload(wires * 3 / 4, wires));
        assert!(full_upload(wires * 3 / 4 + 1, wires));
        assert!(
            380_000 * std::mem::size_of::<Patch>() < wires as usize * std::mem::size_of::<Record>()
        );
        assert!(
            wires as usize * 3 / 4 * std::mem::size_of::<Patch>()
                < wires as usize * std::mem::size_of::<Record>()
        );
    }

    /// Render both paths over the same pixels. The duplicate wire is
    /// completely covered in the image: only the top wire may be marked
    /// lit by the colour pass, while both still have quads in quad mode.
    #[test]
    fn pixel_glow_matches_quads_and_marks_the_visible_wire() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("glow rendering unavailable: no GPU adapter ({error})");
                return;
            }
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut map = GpuMap::new(
            &device,
            &adapter.get_downlevel_capabilities(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        map.size(&device, 2);
        let segments = [0, 1].map(|wire| Segment {
            a: [1.0, 8.5],
            b: [15.0, 8.5],
            share: [0.0, 1.0],
            wire,
        });
        map.segments = Some((
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("glow test pieces"),
                contents: bytemuck::cast_slice(&segments),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            }),
            2,
        ));
        let view = (egui::Vec2::ZERO, egui::vec2(16.0, 16.0), 1.0, 1.0);
        map.uniforms(&queue, 0, view, 1.0, 1.0, true, false, 1);
        let mut encoder = device.create_command_encoder(&Default::default());
        map.draw_pieces(
            &device,
            &mut encoder,
            [16, 16],
            Vec::new(),
            &std::iter::once(0..2).collect::<Vec<_>>(),
        );
        queue.submit([encoder.finish()]);
        let picture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glow test picture"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glow test pixels and marks"),
            size: 16 * 256 + 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let states = Mutex::new(WireStates::default());
        let none = crate::changes::NONE;
        // Before a front, across a slow front, behind it, expired, and at
        // 1000 c/s and max, with quiet/busy wires and a change of rate.
        let cases = [
            ([[none, 0.0, 1.0], [1.0, 0.4, 0.8]], 0.9, 1.0, false),
            ([[none, 0.0, 1.0], [1.0, 0.4, 0.8]], 1.2, 1.0, true),
            ([[none, 0.0, 1.0], [1.0, 0.4, 0.8]], 1.5, 1.0, true),
            ([[none, 0.0, 1.0], [1.0, 0.4, 0.8]], 2.3, 1.0, false),
            ([[none, 0.0, 1.0], [1.0, 0.0004, 0.3]], 1.1, 0.05, true),
            ([[none, 0.0, 1.0], [1.0, 0.0004, 0.3]], 1.1, 1.0, true),
            ([[none, 0.0, 1.0], [1.0, 0.0, 0.6]], 1.2, 0.05, true),
            ([[0.8, 0.4, 0.8], [1.0, 0.0004, 0.3]], 1.1, 0.05, true),
        ];
        let mut frame = 0;
        for values in [false, true] {
            for (changes, now, busy, lit) in cases {
                let mut pictures = Vec::new();
                for pixel in [false, true] {
                    frame += 1;
                    states.lock().unwrap().set_all(vec![
                        Record {
                            changes,
                            values: 0x0001_0001,
                            busy: f32::to_bits(busy),
                        };
                        2
                    ]);
                    map.glow = if pixel { Glow::Top } else { Glow::Quads };
                    map.record_lit = true;
                    let mut encoder = device.create_command_encoder(&Default::default());
                    map.sync(&device, &queue, &mut encoder, &states, frame);
                    for k in 0..2 {
                        map.uniforms(&queue, k, view, now, 1.0, values, true, frame);
                    }
                    let groups = map.groups.as_ref().unwrap();
                    let marks = map.marks.as_ref().unwrap();
                    for buffer in marks {
                        encoder.clear_buffer(buffer, 0, None);
                    }
                    let target = picture.create_view(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("glow equivalence"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &target,
                                resolve_target: None,
                                depth_slice: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                        pass.set_pipeline(&map.colours);
                        pass.set_bind_group(0, &groups[0], &[]);
                        pass.set_bind_group(1, &map.pieces.as_ref().unwrap().group, &[]);
                        pass.draw(0..3, 0..1);
                        if !pixel {
                            pass.set_pipeline(&map.pipeline);
                            pass.set_bind_group(0, &groups[1], &[]);
                            pass.set_vertex_buffer(0, map.segments.as_ref().unwrap().0.slice(..));
                            pass.draw(0..4, 0..2);
                        }
                    }
                    encoder.copy_texture_to_buffer(
                        picture.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(16),
                            },
                        },
                        picture.size(),
                    );
                    encoder.copy_buffer_to_buffer(&marks[0], 0, &readback, 4096, 8);
                    encoder.copy_buffer_to_buffer(&marks[1], 0, &readback, 4104, 8);
                    let done = Arc::new(AtomicBool::new(false));
                    let flag = Arc::clone(&done);
                    encoder.map_buffer_on_submit(
                        &readback,
                        wgpu::MapMode::Read,
                        ..,
                        move |result| {
                            result.unwrap();
                            flag.store(true, Ordering::Release);
                        },
                    );
                    queue.submit([encoder.finish()]);
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: None,
                            timeout: Some(std::time::Duration::from_secs(10)),
                        })
                        .unwrap();
                    assert!(done.load(Ordering::Acquire));
                    let data = readback.slice(..).get_mapped_range();
                    let words: &[u32] = bytemuck::cast_slice(&data[4096..]);
                    let mark = if lit { crate::trace::mark(frame) } else { 0 };
                    assert_eq!(
                        &words[..2],
                        &[if pixel { 0 } else { mark }, mark],
                        "pixel={pixel}, now={now}"
                    );
                    assert_eq!(words[2], 0, "covered wire has no value pixel");
                    assert_ne!(words[3], 0, "top wire has value pixels, even with 0/1 off");
                    pictures.push(
                        (0..16)
                            .flat_map(|row| data[row * 256..row * 256 + 64].to_vec())
                            .collect::<Vec<_>>(),
                    );
                    drop(data);
                    readback.unmap();
                }
                assert!(
                    pictures[0]
                        .iter()
                        .zip(&pictures[1])
                        .all(|(&a, &b)| a.abs_diff(b) <= 1),
                    "glow differs: values={values}, now={now}, busy={busy}"
                );
            }
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
    }

    /// Hidden activity survives, the brightest contribution wins, the
    /// mask stays one pixel wide, and a later empty mask clears the glow.
    #[test]
    fn a_mask_keeps_hidden_activity_and_uses_max_not_sum() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("mask rendering unavailable: no GPU adapter ({error})");
                return;
            }
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut map = GpuMap::new(
            &device,
            &adapter.get_downlevel_capabilities(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        let picture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mask test picture"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mask test pixels and marks"),
            size: 4096 + 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let states = Mutex::new(WireStates::default());
        let none = crate::changes::NONE;
        let still = Record {
            changes: [[none, 0.0, 1.0]; 2],
            values: 0,
            busy: 0,
        };
        let active = Record {
            changes: [[none, 0.0, 1.0], [1.0, 0.0, 0.6]],
            values: 0x0001_0001,
            busy: 1.0f32.to_bits(),
        };
        let quiet = Record {
            values: 0,
            busy: 0.05f32.to_bits(),
            ..active
        };
        let ranges: Vec<_> = std::iter::once(0..2).collect();
        let mut frame = 0;
        for ppp in [1.0, 2.0] {
            let view = (
                egui::Vec2::ZERO,
                egui::vec2(16.0 / ppp, 16.0 / ppp),
                1.0 / ppp,
                1.0 / ppp,
            );
            for values in [false, true] {
                // Hidden activity, MAX, a sub-pixel splat and the same
                // piece through the fallback, then an empty mask.
                for phase in 0..5 {
                    for mode in [Glow::Top, Glow::Mask] {
                        frame += 1;
                        let segments = [0, 1].map(|wire| Segment {
                            a: if matches!(phase, 2 | 3) && wire == 0 {
                                [8.2, 8.2]
                            } else {
                                [1.0, 8.5]
                            },
                            b: if matches!(phase, 2 | 3) && wire == 0 {
                                [8.2, 8.2]
                            } else {
                                [15.0, 8.5]
                            },
                            share: [0.0, 1.0],
                            wire,
                        });
                        map.segments = Some((
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("hidden mask test pieces"),
                                contents: bytemuck::cast_slice(&segments),
                                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
                            }),
                            2,
                        ));
                        // Any list must refer to these pieces, including
                        // when this iteration replaces their vertex buffer.
                        map.listed = None;
                        states.lock().unwrap().set_all(if phase == 4 {
                            vec![still; 2]
                        } else {
                            vec![active, if phase == 1 { quiet } else { still }]
                        });
                        map.glow = mode;
                        map.record_lit = true;
                        let mut encoder = device.create_command_encoder(&Default::default());
                        map.sync(&device, &queue, &mut encoder, &states, frame);
                        map.uniforms(&queue, 0, view, 1.1, ppp, values, true, frame);
                        for mark in map.marks.as_ref().unwrap() {
                            encoder.clear_buffer(mark, 0, None);
                        }
                        map.draw_pieces(&device, &mut encoder, [16, 16], Vec::new(), &ranges);
                        map.uniforms(&queue, 3, view, 1.1, ppp, values, true, frame);
                        let listed = map.list(&device, &queue, &mut encoder, 1.1, &ranges, true);
                        // Exercise both the compute-to-colour path and
                        // the fallback on adapters without indirect lists.
                        map.from_list = listed && phase != 3;
                        if map.cull.is_some() {
                            assert_eq!(map.from_list, phase != 3);
                        }
                        if mode == Glow::Mask {
                            map.draw_mask(&device, &mut encoder, &ranges, true);
                        }
                        let target = picture.create_view(&Default::default());
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("mask test colours"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target,
                                    resolve_target: None,
                                    depth_slice: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                ..Default::default()
                            });
                            pass.set_pipeline(&map.colours);
                            pass.set_bind_group(0, &map.groups.as_ref().unwrap()[0], &[]);
                            pass.set_bind_group(1, &map.pieces.as_ref().unwrap().group, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        encoder.copy_texture_to_buffer(
                            picture.as_image_copy(),
                            wgpu::TexelCopyBufferInfo {
                                buffer: &readback,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(256),
                                    rows_per_image: Some(16),
                                },
                            },
                            picture.size(),
                        );
                        let marks = map.marks.as_ref().unwrap();
                        encoder.copy_buffer_to_buffer(&marks[0], 0, &readback, 4096, 8);
                        encoder.copy_buffer_to_buffer(&marks[1], 0, &readback, 4104, 8);
                        let done = Arc::new(AtomicBool::new(false));
                        let flag = Arc::clone(&done);
                        encoder.map_buffer_on_submit(
                            &readback,
                            wgpu::MapMode::Read,
                            ..,
                            move |result| {
                                result.unwrap();
                                flag.store(true, Ordering::Release);
                            },
                        );
                        queue.submit([encoder.finish()]);
                        device
                            .poll(wgpu::PollType::Wait {
                                submission_index: None,
                                timeout: Some(std::time::Duration::from_secs(10)),
                            })
                            .unwrap();
                        assert!(done.load(Ordering::Acquire));
                        let data = readback.slice(..).get_mapped_range();
                        let words: &[u32] = bytemuck::cast_slice(&data[4096..]);
                        assert_eq!(
                            words[0],
                            if mode == Glow::Mask && phase != 4 {
                                crate::trace::mark(frame)
                            } else {
                                0
                            },
                            "hidden mark: mode={mode:?} phase={phase} ppp={ppp}"
                        );
                        assert_eq!(
                            words[1],
                            if phase == 1 {
                                crate::trace::mark(frame)
                            } else {
                                0
                            }
                        );
                        assert_eq!(&words[2..], &[0, 1], "only the top wire's 0 value is seen");
                        let pixel = &data[8 * 256 + 8 * 4..8 * 256 + 9 * 4];
                        let glow = if mode == Glow::Mask && phase != 4 {
                            1.0 - 0.1 / 0.6
                        } else if phase == 1 {
                            (1.0 - 0.1 / 0.6) * crate::changes::glow_weight(0.05 * (-0.1f32).exp())
                        } else {
                            0.0
                        };
                        let base = if values {
                            crate::fabric_view::ZERO
                        } else {
                            crate::fabric_view::USED
                        };
                        let expected = rgba(base)
                            .into_iter()
                            .zip(rgba(crate::fabric_view::CHANGE))
                            .map(|(base, change)| {
                                ((base + (change - base) * glow) * 255.0).round() as u8
                            })
                            .collect::<Vec<_>>();
                        assert!(
                            pixel.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1),
                            "mask mix/MAX: mode={mode:?} phase={phase} ppp={ppp} pixel={pixel:?}"
                        );
                        // A dense mask is exactly one physical pixel wide.
                        assert_eq!(&data[7 * 256 + 8 * 4..7 * 256 + 9 * 4], &[0, 0, 0, 0]);
                        assert_eq!(&data[9 * 256 + 8 * 4..9 * 256 + 9 * 4], &[0, 0, 0, 0]);
                        drop(data);
                        readback.unmap();
                    }
                }
            }
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
    }

    /// Compare the combined buffer/texture mask with strips alone, including
    /// sub-pixel pieces, diagonal caps, overlap/MAX, clipping, and fronts.
    #[test]
    fn splats_and_long_strips_match_the_complete_strip_mask() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("splat comparison unavailable: no GPU adapter ({error})");
                return;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }))
        .unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut map = GpuMap::new(
            &device,
            &adapter.get_downlevel_capabilities(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        if map.cull.is_none() {
            eprintln!("splat comparison unavailable: no indirect listing");
            return;
        }
        let diagnostic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mask share comparison"),
            entries: &[0, 1, 5].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: match binding {
                    0 => wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    1 => wgpu::ShaderStages::VERTEX,
                    _ => wgpu::ShaderStages::FRAGMENT,
                },
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage {
                        read_only: binding != 5,
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mask share comparison"),
            source: wgpu::ShaderSource::Wgsl(mask_share_diagnostics().into()),
        });
        let diagnostic_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mask share comparison"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("mask share comparison"),
                    bind_group_layouts: &[Some(&map.layout), Some(&diagnostic_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_mask_listed"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_mask"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: MASK_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::RED,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mut geometry = vec![
            ([2.1, 2.1], [2.2, 2.3]),
            ([4.2, 2.4], [4.2, 2.4]),
            ([6.8, 2.8], [7.8, 3.8]),
            ([8.99, 2.99], [10.05, 3.75]), // Snaps to (2,1): four pixels.
            ([10.05, 3.75], [8.99, 2.99]),
            ([2.99, 5.99], [4.04, 7.04]),
            ([4.04, 7.04], [2.99, 5.99]),
            ([-0.4, 9.4], [0.4, 9.4]),
            ([13.9, 9.4], [15.2, 9.7]),
            ([1.0, 12.5], [14.0, 12.5]),
            ([12.0, 1.0], [12.0, 14.0]),
            ([1.0, 14.0], [14.0, 1.0]),
            ([2.1, 2.1], [2.2, 2.3]), // A weaker glow under another splat.
            ([6.1, 9.1], [7.6, 9.1]), // Exactly at the length threshold.
            ([6.1, 10.1], [7.599, 10.1]),
            ([3.2, 3.4], [20.6, 12.1]),
            ([3.2, 3.4], [20.6, 12.1]), // A weaker multi-pixel contribution.
        ];
        // Each piece has its own pixels so overlap cannot hide a missing
        // diagonal or cap. The two weak duplicates share their stronger
        // contributions' tiles.
        for threshold in [1.5, 3.0, 6.0, 12.0, 24.0] {
            for direction in [
                [1.0f32, 0.0],
                [0.0, 1.0],
                [1.0, 1.0],
                [2.0, 1.0],
                [1.0, 3.0],
                [2.0, -1.0],
            ] {
                let norm = direction[0].hypot(direction[1]);
                for length in [threshold - 0.001, threshold, threshold + 0.001] {
                    for offset in [[0.13, 0.87], [0.87, 0.13]] {
                        let d = direction.map(|v| v / norm * length);
                        let a = [16.0 + offset[0] - d[0] * 0.5, 16.0 + offset[1] - d[1] * 0.5];
                        let b = [a[0] + d[0], a[1] + d[1]];
                        geometry.extend([(a, b), (b, a)]);
                    }
                }
            }
        }
        let mut segments: Vec<_> = geometry
            .into_iter()
            .enumerate()
            .map(|(wire, (a, b))| {
                let tile = match wire {
                    12 => 0,
                    16 => 15,
                    _ => wire,
                };
                let offset = [(tile % 16) as f32 * 32.0, (tile / 16) as f32 * 32.0];
                Segment {
                    a: [a[0] + offset[0], a[1] + offset[1]],
                    b: [b[0] + offset[0], b[1] + offset[1]],
                    share: [0.0, 1.0],
                    wire: wire as u32,
                }
            })
            .collect();
        segments.push(Segment {
            a: [-0.4, 9.4],
            b: [0.4, 9.4],
            share: [0.0, 1.0],
            wire: segments.len() as u32,
        });
        segments.push(Segment {
            a: [-2.0, -2.0],
            b: [-2.0, -2.0],
            share: [0.0, 1.0],
            wire: segments.len() as u32,
        });
        let pixels = [512, (segments.len() as u32).div_ceil(16) * 32];
        let row_bytes = (pixels[0] * 2).next_multiple_of(256);
        let texture_bytes = u64::from(row_bytes) * u64::from(pixels[1]);
        let pixel_count = (pixels[0] * pixels[1]) as usize;
        let splat_bytes = pixel_count as u64 * 4;
        let marks_at = texture_bytes + splat_bytes;
        let wires = Arc::new(Mutex::new(WireStates::default()));
        let measured = Arc::new(Mutex::new(Measurements::default()));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("combined mask test"),
            size: (marks_at + segments.len() as u64 * 4).next_multiple_of(8),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ranges: Vec<_> = std::iter::once(0..segments.len() as u32).collect();
        let mut frame = 0;
        let mut glow_failures = Vec::new();
        for threshold in [1.5, 3.0, 6.0, 12.0, 24.0] {
            map.splat_max = threshold;
            for ppp in [1.0, 2.0] {
                for fractional in [false, true] {
                    let size = if fractional {
                        egui::vec2(pixels[0] as f32 - 0.25, pixels[1] as f32 + 0.25)
                    } else {
                        egui::vec2(pixels[0] as f32, pixels[1] as f32)
                    } / ppp;
                    let view = (egui::Vec2::ZERO, size, 1.0 / ppp, 1.0 / ppp);
                    for front in [false, true] {
                        let mut reference = None;
                        for hybrid in [false, true] {
                            frame += 1;
                            let records = (0..segments.len())
                                .map(|wire| Record {
                                    changes: [
                                        [crate::changes::NONE, 0.0, 1.0],
                                        [1.0, if front { 0.4 } else { 0.0 }, 0.6],
                                    ],
                                    values: 0x0001_0001,
                                    busy: if wire == 12 || wire == 16 {
                                        0.05f32.to_bits()
                                    } else {
                                        1.0f32.to_bits()
                                    },
                                })
                                .collect();
                            wires.lock().unwrap().set_all(records);
                            map.begin_frame(&device, &queue, &wires, frame, Some(&measured));
                            map.glow = Glow::Mask;
                            map.record_lit = true;
                            let mut encoder = device.create_command_encoder(&Default::default());
                            map.sync(&device, &queue, &mut encoder, &wires, frame);
                            if map.segments.is_none() {
                                map.segments = Some((
                                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                        label: Some("short and long test pieces"),
                                        contents: bytemuck::cast_slice(&segments),
                                        usage: wgpu::BufferUsages::VERTEX
                                            | wgpu::BufferUsages::STORAGE,
                                    }),
                                    segments.len() as u32,
                                ));
                            }
                            for mark in map.marks.as_ref().unwrap() {
                                encoder.clear_buffer(mark, 0, None);
                            }
                            map.uniforms(&queue, 0, view, 1.213, ppp, true, true, frame);
                            map.draw_pieces(&device, &mut encoder, pixels, Vec::new(), &ranges);
                            map.uniforms(&queue, 3, view, 1.213, ppp, true, true, frame);
                            map.from_list =
                                map.list(&device, &queue, &mut encoder, 1.213, &ranges, true);
                            assert!(map.from_list);
                            if hybrid {
                                map.draw_mask(&device, &mut encoder, &ranges, true);
                            } else {
                                let listed = map.listed.as_mut().unwrap();
                                let diagnostic_group =
                                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("mask share comparison"),
                                        layout: &diagnostic_layout,
                                        entries: &[
                                            (0, &map.segments.as_ref().unwrap().0),
                                            (1, &listed._list),
                                            (5, &map.pieces.as_ref().unwrap().splats),
                                        ]
                                        .map(
                                            |(binding, buffer)| wgpu::BindGroupEntry {
                                                binding,
                                                resource: buffer.as_entire_binding(),
                                            },
                                        ),
                                    });
                                let pipeline = std::mem::replace(
                                    &mut map.mask_listed_pipeline,
                                    diagnostic_pipeline.clone(),
                                );
                                let group =
                                    std::mem::replace(&mut listed.draw_group, diagnostic_group);
                                map.draw_mask(&device, &mut encoder, &ranges, false);
                                map.mask_listed_pipeline = pipeline;
                                map.listed.as_mut().unwrap().draw_group = group;
                            }
                            map.end_frame(&mut encoder, Some(&measured));
                            let pieces = map.pieces.as_ref().unwrap();
                            encoder.copy_texture_to_buffer(
                                pieces.mask.as_image_copy(),
                                wgpu::TexelCopyBufferInfo {
                                    buffer: &readback,
                                    layout: wgpu::TexelCopyBufferLayout {
                                        offset: 0,
                                        bytes_per_row: Some(row_bytes),
                                        rows_per_image: Some(pixels[1]),
                                    },
                                },
                                pieces.mask.size(),
                            );
                            encoder.copy_buffer_to_buffer(
                                &pieces.splats,
                                0,
                                &readback,
                                texture_bytes,
                                splat_bytes,
                            );
                            encoder.copy_buffer_to_buffer(
                                &map.marks.as_ref().unwrap()[0],
                                0,
                                &readback,
                                marks_at,
                                segments.len() as u64 * 4,
                            );
                            let done = Arc::new(AtomicBool::new(false));
                            let flag = Arc::clone(&done);
                            encoder.map_buffer_on_submit(
                                &readback,
                                wgpu::MapMode::Read,
                                ..,
                                move |result| {
                                    result.unwrap();
                                    flag.store(true, Ordering::Release);
                                },
                            );
                            queue.submit([encoder.finish()]);
                            device
                                .poll(wgpu::PollType::Wait {
                                    submission_index: None,
                                    timeout: Some(std::time::Duration::from_secs(10)),
                                })
                                .unwrap();
                            assert!(done.load(Ordering::Acquire));
                            let data = readback.slice(..).get_mapped_range();
                            let strips: &[u16] =
                                bytemuck::cast_slice(&data[..texture_bytes as usize]);
                            let splats: &[u32] = bytemuck::cast_slice(
                                &data[texture_bytes as usize..marks_at as usize],
                            );
                            let lit: Vec<_> = bytemuck::cast_slice::<u8, u32>(
                                &data[marks_at as usize..marks_at as usize + segments.len() * 4],
                            )
                            .iter()
                            .map(|&mark| mark != 0)
                            .collect();
                            let combined: Vec<_> = (0..pixel_count)
                                .map(|i| {
                                    strips[(i / pixels[0] as usize) * (row_bytes / 2) as usize
                                        + i % pixels[0] as usize]
                                        .max(if hybrid { splats[i] as u16 } else { 0 })
                                })
                                .collect();
                            if hybrid {
                                assert!(
                                    splats.iter().any(|&v| v > 0),
                                    "the compute path wrote no splats"
                                );
                                assert!(
                                    strips.iter().any(|&v| v > 0),
                                    "the strips path drew nothing"
                                );
                            }
                            if !front {
                                assert!(
                                    lit[..lit.len() - 1].iter().all(|&v| v),
                                    "an active wire was not marked"
                                );
                            }
                            assert!(!lit[lit.len() - 1], "an off-screen wire was marked");
                            if let Some((pixels, marks, share_errors)) = &reference {
                                let pixels: &Vec<u16> = pixels;
                                assert_eq!(&lit, marks, "splat/strip lit marks differ");
                                let share_errors: &Vec<f32> = share_errors;
                                let mut worst = (0u16, 0usize, 0u16, 0u16, 0.0f64);
                                let mut failures = 0;
                                for (i, (&a, &b)) in pixels.iter().zip(&combined).enumerate() {
                                    assert_eq!(
                                        a == 0,
                                        b == 0,
                                        "coverage at pixel {i}, threshold={threshold} ppp={ppp} \
                                         front={front} fractional={fractional}"
                                    );
                                    let bound = glow_error_bound(a, b, share_errors[i], front);
                                    let ulps = a.abs_diff(b);
                                    if ulps > worst.0 {
                                        worst = (ulps, i, a, b, bound);
                                    }
                                    if (half_glow(a) - half_glow(b)).abs() > bound {
                                        failures += 1;
                                    }
                                }
                                let (ulps, pixel, a, b, bound) = worst;
                                eprintln!("threshold={threshold} ppp={ppp} front={front} fractional={fractional}: max={ulps} ULP at ({}, {}) pixel={pixel}, {a} != {b}, glow bound={bound:.8}, share error={:.8}, failures={failures}", pixel % map.pieces.as_ref().unwrap().size[0] as usize, pixel / map.pieces.as_ref().unwrap().size[0] as usize, share_errors[pixel]);
                                if failures > 0 {
                                    glow_failures
                                        .push((threshold, ppp, front, fractional, failures));
                                }
                            } else {
                                reference = Some((
                                    combined,
                                    lit,
                                    splats
                                        .iter()
                                        .map(|&v| f32::from_bits(v))
                                        .collect::<Vec<_>>(),
                                ));
                            }
                            drop(data);
                            readback.unmap();
                            if let Some(timing) = &mut map.timing {
                                timing.poll(&device, queue.get_timestamp_period());
                                let gpu = measured
                                    .lock()
                                    .unwrap()
                                    .gpu
                                    .expect("timestamps did not resolve");
                                assert_eq!(gpu.frame, frame);
                                for ms in [
                                    gpu.ms,
                                    gpu.draw_ms,
                                    gpu.mask_ms,
                                    gpu.splat_ms,
                                    gpu.strips_ms,
                                ] {
                                    assert!(
                                        ms == -1.0 || (0.0..10_000.0).contains(&ms),
                                        "timestamp wrapped: {ms}"
                                    );
                                }
                                if gpu.mask_ms >= 0.0 {
                                    assert_eq!(gpu.mask_ms, gpu.splat_ms + gpu.strips_ms);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            glow_failures.is_empty(),
            "MAX/glow cases outside the derived bound: {glow_failures:?}"
        );
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
    }

    /// Compare the mask split on the full design at fit, without a window.
    /// The same deterministic 75% of wires glow throughout every run.
    #[test]
    #[ignore]
    fn benchmark_life_full_splat_thresholds() {
        use ecp5_emu::live::{self, Design, Shared};
        use std::time::{Duration, Instant};

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("the benchmark needs a GPU adapter");
        assert!(
            adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY),
            "the benchmark needs timestamp queries"
        );
        let most = adapter.limits();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: most.max_storage_buffer_binding_size,
            max_buffer_size: most.max_buffer_size,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TIMESTAMP_QUERY,
            required_limits: limits,
            ..Default::default()
        }))
        .unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut map = GpuMap::new(
            &device,
            &adapter.get_downlevel_capabilities(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .expect("the GPU cannot record what it draws");
        assert!(map.cull.is_some(), "the benchmark needs indirect listing");

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/life_full");
        assert!(
            root.join("top.config").is_file(),
            "build target/reference/life_full first"
        );
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            Arc::clone(&shared),
            || {},
        );
        let started = Instant::now();
        let design = loop {
            {
                let state = shared.lock().unwrap();
                assert!(state.error.is_none(), "{}", state.error.as_ref().unwrap());
                if let Some(ready) = &state.ready {
                    break Arc::clone(ready);
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(180),
                "design loading timed out"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        drop(commands);
        let overlay = &design.overlay;
        // The same routed pieces and net shares as FabricView sends.
        let mut lengths = vec![0.0f32; overlay.values.len()];
        for piece in overlay.pieces.iter().filter(|p| p.route) {
            let net = overlay.nets[piece.value as usize] as usize;
            for distance in [piece.d0, piece.d1].into_iter().filter(|d| d.is_finite()) {
                lengths[net] = lengths[net].max(distance);
            }
        }
        let segments: Vec<_> = overlay
            .pieces
            .iter()
            .filter(|p| p.route)
            .map(|p| {
                let length = lengths[overlay.nets[p.value as usize] as usize];
                Segment {
                    a: p.a,
                    b: p.b,
                    share: [p.d0, p.d1].map(|d| {
                        if d.is_finite() && length > 0.0 {
                            (d / length).clamp(0.0, 1.0)
                        } else {
                            0.0
                        }
                    }),
                    wire: p.value,
                }
            })
            .collect();
        let wires = Arc::new(Mutex::new(WireStates::default()));
        wires.lock().unwrap().set_all(
            (0..overlay.values.len())
                .map(|wire| {
                    let hash = (wire as u32)
                        .wrapping_mul(747_796_405)
                        .wrapping_add(2_891_336_453);
                    let active = !((hash >> 16) ^ hash).is_multiple_of(4);
                    Record {
                        changes: [
                            [crate::changes::NONE, 0.0, 1.0],
                            [if active { 1.0 } else { crate::changes::NONE }, 0.0, 0.6],
                        ],
                        values: 0x0001_0001,
                        busy: 1.0f32.to_bits(),
                    }
                })
                .collect(),
        );
        let measured = Arc::new(Mutex::new(Measurements::default()));
        let size = egui::vec2(1600.0, 1000.0);
        let die = egui::Vec2::from(design.fabric.size());
        let cell = (size.x / die.x).min(size.y / die.y);
        let view = ((size - die * cell) / 2.0, size, cell, 1.3);
        let pixels = [3200, 2000];
        let ranges: Vec<_> = std::iter::once(0..segments.len() as u32).collect();
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("benchmark mask counts"),
            size: 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        map.segments = Some((
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("life_full benchmark pieces"),
                contents: bytemuck::cast_slice(&segments),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            }),
            segments.len() as u32,
        ));
        map.glow = Glow::Mask;
        eprintln!(
            "life_full: {} pieces, 75% active wires, 1600x1000 points at 2 pixels/point, {} ({:?})",
            segments.len(),
            adapter.get_info().name,
            adapter.get_info().backend
        );
        eprintln!("5 warmup frames, medians of 15 frames; sum is the sum of pass medians");
        eprintln!(
            "threshold px | splat ms | strips ms | sum ms | pieces splatted | pieces stripped"
        );
        let mut frame = 0;
        for threshold in [1.5, 3.0, 6.0, 12.0, 24.0] {
            map.splat_max = threshold;
            let mut splat_times = Vec::new();
            let mut strip_times = Vec::new();
            let mut piece_counts = None;
            for sample in 0..20 {
                frame += 1;
                map.begin_frame(&device, &queue, &wires, frame, Some(&measured));
                let mut encoder = device.create_command_encoder(&Default::default());
                map.sync(&device, &queue, &mut encoder, &wires, frame);
                // A fixed fit view keeps the image of pieces, as in the window.
                if map.pieces.is_none() {
                    map.uniforms(&queue, 0, view, 1.213, 2.0, true, false, frame);
                    map.draw_pieces(&device, &mut encoder, pixels, Vec::new(), &ranges);
                }
                map.uniforms(&queue, 3, view, 1.213, 2.0, true, false, frame);
                map.from_list = map.list(&device, &queue, &mut encoder, 1.213, &ranges, false);
                assert!(map.from_list, "the GPU cannot hold life_full's piece list");
                map.draw_mask(&device, &mut encoder, &ranges, true);
                // Metal may resolve counters before the sampling command
                // buffer completes, even when its copy is mapped afterwards.
                // Keep this slot, submit the passes, then resolve in a second
                // submission only after sampling has completed. Waiting on
                // the already-resolved staging buffer cannot repair stale data.
                // https://github.com/gfx-rs/wgpu/issues/9414
                let slot_index = map.timing.as_mut().unwrap().active.take().unwrap();
                map.end_frame(&mut encoder, Some(&measured));
                let listed = map.listed.as_ref().unwrap();
                encoder.copy_buffer_to_buffer(&listed.args, 4, &counts, 0, 4);
                encoder.copy_buffer_to_buffer(
                    &listed.mask.as_ref().unwrap().args,
                    4,
                    &counts,
                    4,
                    4,
                );
                let counts_done = Arc::new(AtomicU32::new(0));
                let done = Arc::clone(&counts_done);
                encoder.map_buffer_on_submit(&counts, wgpu::MapMode::Read, .., move |result| {
                    done.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
                });
                let submission = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(Duration::from_secs(30)),
                    })
                    .unwrap();
                assert_eq!(
                    counts_done.load(Ordering::Acquire),
                    1,
                    "piece count readback failed"
                );
                let data = counts.slice(..).get_mapped_range();
                let words: &[u32] = bytemuck::cast_slice(&data);
                let (total, stripped) = (words[0], words[1]);
                assert!(total > 0 && stripped <= total);
                if let Some(previous) = piece_counts {
                    assert_eq!(previous, (total, stripped));
                }
                piece_counts = Some((total, stripped));
                drop(data);
                counts.unmap();

                let timing = map.timing.as_mut().unwrap();
                let started = Instant::now();
                let (splat_ms, strips_ms) = loop {
                    let mut resolve = device.create_command_encoder(&Default::default());
                    timing.resolve(&mut resolve, slot_index);
                    let submission = queue.submit([resolve.finish()]);
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: Some(Duration::from_secs(30)),
                        })
                        .unwrap();
                    let slot = &timing.slots[slot_index];
                    assert_eq!(slot.frame, frame);
                    assert!(
                        slot.used[SPLAT] && slot.used[MASK],
                        "mask timestamp writes were not attached"
                    );
                    // Wait for this frame's mapping callback, not a previous
                    // Measurements sample. A mapped invalid pair is different
                    // from a late readback: resolve the same counters again.
                    while slot.done.load(Ordering::Acquire) == 0 {
                        assert!(
                            started.elapsed() < Duration::from_secs(30),
                            "timestamp mapping timed out"
                        );
                        device
                            .poll(wgpu::PollType::Wait {
                                submission_index: None,
                                timeout: Some(Duration::from_secs(1)),
                            })
                            .unwrap();
                    }
                    assert_eq!(
                        slot.done.load(Ordering::Acquire),
                        1,
                        "timestamp readback failed"
                    );
                    let data = slot.staging.slice(..).get_mapped_range();
                    let words: &[u64] = bytemuck::cast_slice(&data);
                    let pairs = std::array::from_fn(|part| [words[part * 2], words[part * 2 + 1]]);
                    let gpu =
                        GpuTime::from_pairs(frame, pairs, slot.used, queue.get_timestamp_period());
                    // The render pass is always encoded, with its timestamps
                    // attached even for an indirect draw. Zero instances run
                    // no strip vertices/fragments; Metal may leave that pair
                    // unavailable. Count that empty strip workload as zero,
                    // never turn a missing nonempty measurement into zero.
                    let strips_ms = if stripped == 0 { 0.0 } else { gpu.strips_ms };
                    let splat_ms = gpu.splat_ms;
                    drop(data);
                    if splat_ms >= 0.0 && strips_ms >= 0.0 {
                        timing.poll(&device, queue.get_timestamp_period());
                        break (splat_ms, strips_ms);
                    }
                    slot.staging.unmap();
                    slot.done.store(0, Ordering::Relaxed);
                    assert!(started.elapsed() < Duration::from_secs(30),
                        "completed nonempty passes still have unavailable counters: frame={frame}, used={:?}, splat={:?}, strips={:?}, total={total}, stripped={stripped}",
                        slot.used, pairs[SPLAT], pairs[MASK]);
                };
                if sample >= 5 {
                    splat_times.push(splat_ms);
                    strip_times.push(strips_ms);
                }
            }
            splat_times.sort_by(f64::total_cmp);
            strip_times.sort_by(f64::total_cmp);
            let splat = splat_times[splat_times.len() / 2];
            let strips = strip_times[strip_times.len() / 2];
            let (total, stripped) = piece_counts.unwrap();
            eprintln!(
                "{threshold:12.1} | {splat:8.3} | {strips:9.3} | {:6.3} | {:15} | {:15}",
                splat + strips,
                total - stripped,
                stripped
            );
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
    }

    /// Every pipeline builds, including both mask paths, without a window.
    #[test]
    fn the_map_builds_on_the_gpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()
        else {
            eprintln!("no GPU adapter: the map's pipelines not checked");
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let map = GpuMap::new(
            &device,
            &adapter.get_downlevel_capabilities(),
            wgpu::TextureFormat::Bgra8Unorm,
        );
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
        assert!(map.is_some(), "the GPU cannot record what it draws");
        assert!(map.unwrap().timing.is_none(), "timestamps were not enabled");
    }

    /// Exercise both callbacks, full writes, scatter, cull, the image and
    /// the asynchronous query resolve without opening a window.
    #[test]
    fn the_map_times_and_counts_uploads_on_the_gpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        ) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("map timing unavailable: no GPU adapter ({error})");
                return;
            }
        };
        let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        eprintln!(
            "map timing: {} ({:?}), TIMESTAMP_QUERY={}",
            adapter.get_info().name,
            adapter.get_info().backend,
            !features.is_empty(),
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut resources = CallbackResources::default();
        resources.insert(
            GpuMap::new(
                &device,
                &adapter.get_downlevel_capabilities(),
                wgpu::TextureFormat::Bgra8Unorm,
            )
            .expect("the GPU cannot record what it draws"),
        );
        let wires = Arc::new(Mutex::new(WireStates::default()));
        wires.lock().unwrap().set_all(vec![Record::zeroed(); 8]);
        let measured = Arc::new(Mutex::new(Measurements::default()));
        let screen = ScreenDescriptor {
            size_in_pixels: [16, 16],
            pixels_per_point: 1.0,
        };
        let segments = Arc::new(vec![Segment {
            a: [0.0, 0.0],
            b: [1.0, 1.0],
            share: [0.0, 1.0],
            wire: 0,
        }]);
        let mut callback = MapCallback {
            design: 1,
            segments: Some(Arc::clone(&segments)),
            ranges: std::iter::once(0..1).collect(),
            wires: Arc::clone(&wires),
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(16.0, 16.0)),
            origin: egui::Vec2::ZERO,
            size: egui::vec2(16.0, 16.0),
            cell: 8.0,
            width: 1.0,
            now: 1.0,
            values: true,
            glow: GlowChoice::Auto,
            frame: 1,
            resets: vec![0],
            record: Some(Arc::default()),
            measure: Some(Arc::clone(&measured)),
            dropped: Arc::default(),
        };
        let mut spots = SpotsCallback {
            design: 1,
            spots: Some(Arc::new(Spots {
                spots: vec![Spot::zeroed()],
                wires: vec![0],
            })),
            ranges: Vec::new(),
            wires: Arc::clone(&wires),
            origin: callback.origin,
            size: callback.size,
            cell: callback.cell,
            now: callback.now,
            values: true,
            frame: 1,
            measure: Some(Arc::clone(&measured)),
            last: false,
        };
        // Run enough frames to reuse every slot. The waits here are only
        // for the test; the window only polls and skips a busy slot.
        for frame in 1..=8 {
            if frame == 2 {
                wires.lock().unwrap().set(0, Record::zeroed());
            } else if frame == 4 {
                for wire in 0..3 {
                    wires.lock().unwrap().set(wire, Record::zeroed());
                }
            } else if frame == 6 {
                for wire in 0..7 {
                    wires.lock().unwrap().set(wire, Record::zeroed());
                }
            }
            callback.frame = frame;
            spots.frame = frame;
            let mut egui_encoder = device.create_command_encoder(&Default::default());
            spots.prepare(&device, &queue, &screen, &mut egui_encoder, &mut resources);
            callback.prepare(&device, &queue, &screen, &mut egui_encoder, &mut resources);
            let upload = *measured.lock().unwrap();
            assert_eq!(upload.upload_frame, Some(frame));
            assert_eq!(upload.full_upload, frame == 1 || frame == 6);
            // Three drawing uniforms, runs/cull uniforms/args, resets/count.
            let fixed = 3 * std::mem::size_of::<Uniforms>() + 8 + 16 + 16 + 4 + 16;
            let changing = match frame {
                1 => {
                    8 * std::mem::size_of::<Record>()
                        + std::mem::size_of::<Segment>()
                        + std::mem::size_of::<Spot>()
                        + 4
                }
                2 => std::mem::size_of::<Patch>() + 16,
                4 => 3 * std::mem::size_of::<Patch>() + 16,
                6 => 8 * std::mem::size_of::<Record>(),
                _ => 0,
            };
            assert_eq!(upload.upload_bytes, (fixed + changing) as u64);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .unwrap();
            let map = resources.get_mut::<GpuMap>().unwrap();
            if map.from_list {
                map.listing_reads.poll(&device);
                let (_, mode) = map
                    .listing_reads
                    .latest
                    .as_ref()
                    .expect("list count not read");
                assert_eq!(mode.frame, frame);
                assert_eq!(mode.listed, 0);
                assert_eq!(mode.mode, Glow::Quads);
                assert_eq!(measured.lock().unwrap().glow.unwrap().frame, frame);
            }
            if let Some(timing) = &mut map.timing {
                timing.poll(&device, queue.get_timestamp_period());
                let gpu = measured
                    .lock()
                    .unwrap()
                    .gpu
                    .expect("query results not read");
                assert_eq!(gpu.frame, frame);
                assert!([
                    gpu.ms,
                    gpu.upload_ms,
                    gpu.list_ms,
                    gpu.draw_ms,
                    gpu.mask_ms,
                    gpu.splat_ms,
                    gpu.strips_ms
                ]
                .into_iter()
                .all(|ms| ms.is_finite() && (ms == -1.0 || ms >= 0.0)));
                if gpu.ms >= 0.0 {
                    assert!(gpu.ms >= gpu.upload_ms + gpu.list_ms + gpu.draw_ms - 1e-9);
                }
                if frame > 1 {
                    assert_eq!(gpu.draw_ms, 0.0, "the image was cached");
                }
                if frame != 2 && frame != 4 {
                    assert_eq!(gpu.upload_ms, 0.0, "no scatter was needed");
                }
            } else {
                assert!(features.is_empty());
                assert!(measured.lock().unwrap().gpu.is_none());
            }
        }
        if let Some(timing) = &mut resources.get_mut::<GpuMap>().unwrap().timing {
            let mut commands = Vec::new();
            for frame in 9..=12 {
                timing.begin(frame, &measured);
                assert!(timing.active.is_some());
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("map timing ring test"),
                        timestamp_writes: timing.compute(UPLOAD),
                    });
                }
                timing.finish(&mut encoder);
                commands.push(encoder.finish());
            }
            // All four copies are still in flight: measuring skips this
            // frame without waiting, and the earlier samples still resolve.
            timing.begin(13, &measured);
            assert!(timing.active.is_none());
            queue.submit(commands);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .unwrap();
            timing.poll(&device, queue.get_timestamp_period());
            assert_eq!(measured.lock().unwrap().gpu.unwrap().frame, 12);
            assert!(timing.slots.iter().all(|slot| slot.target.is_none()));
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{}", error.unwrap());
        eprintln!("map timing: upload counts checked; queries resolved or unsupported");
    }
}
