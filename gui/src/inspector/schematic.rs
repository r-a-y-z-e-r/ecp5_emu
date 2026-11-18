//! The schematic of the element the inspector shows: its insides drawn as a
//! datasheet draws them, live. Every wire has the colour of its value, as
//! the map's 0/1 has it (1 bright, 0 dim tan, not known grey-violet, not
//! used grey); the way a value takes through the muxes now is bold with a
//! cyan edge; a change flashes orange as on the map.
//!
//! - a LUT: its 16 cells of INIT and the tree of 2:1 muxes that A, B, C and
//!   D select, down to F; in CCU2 mode also the xor with the carry in and
//!   the carry mux (propagate or generate); as RAMW, the buffers that pass
//!   its inputs on as the RAM's write address or data;
//! - a flip-flop: the D mux (REG.SD), the enable mux, the set/reset mux and
//!   the register with its clock;
//! - the wide muxes F5 and FX; a slice's control set; a whole slice;
//! - a CIB: each J wire in use with its input mux;
//! - a pad: the tristate driver, the pad and the input buffer;
//! - any other site: a chip with its pins in use.
use super::{
    Control, Details, FlipFlop, Input, Inside, JWire, Lut, Mux, Pin, Pio, SliceView, LETTERS,
};
use crate::fabric_view::{CHANGE, NET, ONE, SITE, SITE_EDGE, SITE_TEXT, UNKNOWN, USED, ZERO};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};

/// A value on the sheet: as the emulator has it, or none where the design
/// does not use the wire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    Zero,
    One,
    X,
    Off,
}

impl V {
    fn of(b: Option<bool>) -> V {
        match b {
            Some(true) => V::One,
            Some(false) => V::Zero,
            None => V::X,
        }
    }
    fn bit(self) -> Option<bool> {
        match self {
            V::One => Some(true),
            V::Zero => Some(false),
            _ => None,
        }
    }
    fn not(self) -> V {
        match self {
            V::One => V::Zero,
            V::Zero => V::One,
            v => v,
        }
    }
}

/// A 2:1 mux: `high` where `sel` is 1, `low` where 0; with `sel` not known,
/// their value where they agree.
fn mux(sel: V, low: V, high: V) -> V {
    match sel.bit() {
        Some(false) => low,
        Some(true) => high,
        None if low == high && low.bit().is_some() => low,
        None => V::X,
    }
}

fn xor(a: V, b: V) -> V {
    match (a.bit(), b.bit()) {
        (Some(x), Some(y)) => V::of(Some(x ^ y)),
        _ => V::X,
    }
}

/// A wire's value now and how much it glows for its last change.
#[derive(Clone, Copy)]
struct W {
    v: V,
    glow: f32,
}

impl W {
    /// A value worked out on the sheet: it does not glow.
    fn c(v: V) -> W {
        W { v, glow: 0.0 }
    }
    fn not(self) -> W {
        W {
            v: self.v.not(),
            glow: self.glow,
        }
    }
}

const FONT: f32 = 11.0;
const SMALL: f32 = 9.5;
const BACK: Color32 = Color32::from_rgb(14, 15, 18);
const BODY: Color32 = Color32::from_rgb(38, 40, 46);
const EDGE: Color32 = Color32::from_gray(150);
const TEXT: Color32 = Color32::from_gray(210);
const DIM: Color32 = Color32::from_gray(125);

/// What a sheet shows live: the values of the overlay's wires, how much
/// each glows for its last change, and whether a wire has the colour of its
/// value (the map's 0/1) or of being used (beige, grey where not).
pub(super) struct Live<'a> {
    pub lit: &'a [u8],
    pub glow: &'a dyn Fn(u32) -> f32,
    pub values: bool,
}

/// A sheet 100 units wide, as wide as the panel, and `height` units high.
struct Sheet<'a> {
    painter: egui::Painter,
    rect: Rect,
    k: f32,
    live: &'a Live<'a>,
}

impl<'a> Sheet<'a> {
    fn new(ui: &mut egui::Ui, height: f32, live: &'a Live<'a>) -> Self {
        let width = ui.available_width().max(240.0);
        let k = width / 100.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height * k), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, BACK);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, Color32::from_gray(55)),
            StrokeKind::Inside,
        );
        Self {
            painter,
            rect,
            k,
            live,
        }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.rect.min + Vec2::new(x, y) * self.k
    }

    /// A wire of the overlay: its value now, or not used.
    fn wire(&self, w: Option<u32>) -> W {
        match w {
            None => W::c(V::Off),
            Some(i) => W {
                v: match self.live.lit.get(i as usize) {
                    Some(0) => V::Zero,
                    Some(1) => V::One,
                    _ => V::X,
                },
                glow: (self.live.glow)(i),
            },
        }
    }

    fn input(&self, i: Input) -> W {
        match i {
            Input::Tied(v) => W::c(V::of(Some(v))),
            Input::Wire(w) => self.wire(w),
        }
    }

    /// A wire's colour: with 0/1 its value's, else beige where it is used;
    /// grey where it is not; orange as it changes.
    fn colour(&self, w: W) -> Color32 {
        let c = match (w.v, self.live.values) {
            (V::Off, _) => Color32::from_gray(70),
            (_, false) => USED,
            (V::One, true) => ONE,
            (V::Zero, true) => ZERO,
            (V::X, true) => UNKNOWN,
        };
        c.lerp_to_gamma(CHANGE, w.glow)
    }

    /// Text on a LUT or flip-flop that the design uses, drawn as `site` does.
    fn ink(&self, out: W) -> Color32 {
        if self.live.values && out.v == V::Zero {
            Color32::from_rgb(245, 236, 216)
        } else {
            SITE_TEXT
        }
    }

    /// A LUT or flip-flop: grey where the design does not use it; else as
    /// the map draws it, beige, or with 0/1 the colour of its output `out`.
    fn site(&self, (x0, y0, x1, y1): (f32, f32, f32, f32), used: bool, out: W, label: &str) {
        if !used {
            self.block(x0, y0, x1, y1, true);
            self.text(
                (x0 + x1) / 2.0,
                (y0 + y1) / 2.0,
                label,
                Align2::CENTER_CENTER,
                DIM,
            );
            return;
        }
        let (fill, text) = match (self.live.values, out.v) {
            (true, V::Zero) => (ZERO, Color32::from_rgb(245, 236, 216)),
            (true, V::One) => (ONE, SITE_TEXT),
            (true, _) => (UNKNOWN, SITE_TEXT),
            (false, _) => (SITE, SITE_TEXT),
        };
        let rect = Rect::from_min_max(self.p(x0, y0), self.p(x1, y1));
        self.painter
            .rect_filled(rect, 2.0, fill.lerp_to_gamma(CHANGE, out.glow));
        self.painter
            .rect_stroke(rect, 2.0, Stroke::new(1.5, SITE_EDGE), StrokeKind::Inside);
        self.text(
            (x0 + x1) / 2.0,
            (y0 + y1) / 2.0,
            label,
            Align2::CENTER_CENTER,
            text,
        );
    }

    /// A wire through `points`; `bold` where the value takes it now.
    fn line(&self, points: &[(f32, f32)], w: W, bold: bool) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        if bold {
            self.painter.add(Shape::line(
                points.clone(),
                Stroke::new(6.0, NET.gamma_multiply(0.4)),
            ));
            self.painter
                .add(Shape::line(points, Stroke::new(2.6, self.colour(w))));
        } else {
            self.painter.add(Shape::line(
                points,
                Stroke::new(1.5 + w.glow, self.colour(w)),
            ));
        }
    }

    /// A connection that is cut: dashed, grey.
    fn cut(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.extend(Shape::dashed_line(
            &points,
            Stroke::new(1.2, Color32::from_gray(90)),
            4.0,
            3.0,
        ));
    }

    fn text(&self, x: f32, y: f32, s: &str, align: Align2, colour: Color32) {
        self.painter
            .text(self.p(x, y), align, s, FontId::monospace(FONT), colour);
    }

    fn small(&self, x: f32, y: f32, s: &str, align: Align2, colour: Color32) {
        self.painter
            .text(self.p(x, y), align, s, FontId::monospace(SMALL), colour);
    }

    /// A value as a small chip: 1, 0, x, or a dash where it is not used.
    fn tag(&self, x: f32, y: f32, w: W, align: Align2) {
        // Without 0/1 the value is only written: on beige, as what is used.
        let (text, back, fore) = match (w.v, self.live.values) {
            (V::Off, _) => ("–", Color32::from_gray(50), Color32::from_gray(150)),
            (V::One, true) => ("1", ONE, SITE_TEXT),
            (V::Zero, true) => ("0", ZERO, Color32::from_rgb(245, 236, 216)),
            (V::X, true) => ("x", UNKNOWN, Color32::from_rgb(30, 24, 50)),
            (V::One, false) => ("1", USED, SITE_TEXT),
            (V::Zero, false) => ("0", USED, SITE_TEXT),
            (V::X, false) => ("x", USED, SITE_TEXT),
        };
        let rect = align.anchor_size(self.p(x, y), Vec2::splat(14.0));
        self.painter
            .rect_filled(rect, 3.0, back.lerp_to_gamma(CHANGE, w.glow));
        self.painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(10.5),
            fore,
        );
    }

    fn dot(&self, x: f32, y: f32, w: W) {
        self.painter
            .circle_filled(self.p(x, y), 2.8, self.colour(w));
    }

    fn block(&self, x0: f32, y0: f32, x1: f32, y1: f32, dim: bool) {
        let rect = Rect::from_min_max(self.p(x0, y0), self.p(x1, y1));
        let (fill, edge) = if dim {
            (Color32::from_rgb(26, 27, 31), Color32::from_gray(80))
        } else {
            (BODY, EDGE)
        };
        self.painter.rect_filled(rect, 2.0, fill);
        self.painter
            .rect_stroke(rect, 2.0, Stroke::new(1.2, edge), StrokeKind::Inside);
    }

    /// A mux: its inputs on its left edge (x0, from y0 to y1) at `ins`, the
    /// output on its right edge; the chosen input's number lit. Returns the
    /// select point at its bottom and the output.
    fn mux(
        &self,
        (x0, x1): (f32, f32),
        (y0, y1): (f32, f32),
        ins: &[f32],
        chosen: Option<usize>,
    ) -> ((f32, f32), (f32, f32)) {
        // Long sides stay steep on a tall mux.
        let inset = ((y1 - y0) * 0.22).min(2.5);
        let points = vec![
            self.p(x0, y0),
            self.p(x1, y0 + inset),
            self.p(x1, y1 - inset),
            self.p(x0, y1),
        ];
        self.painter
            .add(Shape::convex_polygon(points, BODY, Stroke::new(1.2, EDGE)));
        // Each input's number beside it, inside the slanted edges: moved in
        // where it would cross one, left out where it has no room.
        let (tw, th) = (6.0 / self.k, 10.0 / self.k);
        let reach = (0.5 + tw) / (x1 - x0);
        let (top, bottom) = (y0 + inset * reach + th / 2.0, y1 - inset * reach - th / 2.0);
        if (x1 - x0) * self.k >= 14.0 && top <= bottom {
            for (i, &y) in ins.iter().enumerate() {
                self.painter.text(
                    self.p(x0 + 0.5, y.clamp(top, bottom)),
                    Align2::LEFT_CENTER,
                    i.to_string(),
                    FontId::monospace(8.5),
                    if chosen == Some(i) { TEXT } else { DIM },
                );
            }
        }
        (((x0 + x1) / 2.0, y1 - inset / 2.0), (x1, (y0 + y1) / 2.0))
    }

    /// A buffer: a triangle from x0 to its point at x1, `h` high at `y`.
    fn buffer(&self, x0: f32, x1: f32, y: f32, h: f32) {
        let points = vec![
            self.p(x0, y - h / 2.0),
            self.p(x1, y),
            self.p(x0, y + h / 2.0),
        ];
        self.painter
            .add(Shape::convex_polygon(points, BODY, Stroke::new(1.2, EDGE)));
    }

    /// An inverting circle at a pin.
    fn bubble(&self, x: f32, y: f32) {
        self.painter
            .circle(self.p(x, y), 0.8 * self.k, BACK, Stroke::new(1.2, EDGE));
    }

    /// An xor gate from x0 to x1, y0 to y1.
    fn xor(&self, (x0, x1): (f32, f32), (y0, y1): (f32, f32)) {
        let ym = (y0 + y1) / 2.0;
        let bez = |a: (f32, f32), c: (f32, f32), b: (f32, f32)| -> Vec<Pos2> {
            (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let u = 1.0 - t;
                    self.p(
                        u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
                        u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
                    )
                })
                .collect()
        };
        let back = |dx: f32| -> Vec<Pos2> {
            (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    self.p(
                        x0 + dx + 1.2 * (t * std::f32::consts::PI).sin(),
                        y0 + (y1 - y0) * t,
                    )
                })
                .collect()
        };
        let w = x1 - x0;
        self.painter.add(Shape::convex_polygon(
            vec![self.p(x0 + 1.2, y0), self.p(x1, ym), self.p(x0 + 1.2, y1)],
            BODY,
            Stroke::NONE,
        ));
        let edge = Stroke::new(1.2, EDGE);
        self.painter.add(Shape::line(
            bez((x0, y0), (x0 + 0.6 * w, y0), (x1, ym)),
            edge,
        ));
        self.painter.add(Shape::line(
            bez((x0, y1), (x0 + 0.6 * w, y1), (x1, ym)),
            edge,
        ));
        self.painter.add(Shape::line(back(0.0), edge));
        self.painter.add(Shape::line(back(-1.0), edge));
    }
}

/// The schematic of the element, with a line on how to read it.
pub(super) fn draw(ui: &mut egui::Ui, d: &Details, live: &Live) {
    match &d.inside {
        Inside::Lut(l) => lut(ui, l, live),
        Inside::Ramw(outputs) => ramw(ui, outputs, live),
        Inside::FlipFlop(f) => flip_flop(ui, f, live),
        Inside::Mux(m) => wide(ui, m, live),
        Inside::Control(c) => control(ui, c, live),
        Inside::Slice(v) => slice(ui, v, live),
        Inside::Cib(rows) => cib(ui, rows, live),
        Inside::Pio(p) => pio(ui, p, live),
        Inside::Nothing => chip(ui, d, live),
    }
    let caption = if live.values {
        "Wires: 1 bright, 0 dim, x not known, grey not used. Bold: the way the value takes now. A change flashes orange."
    } else {
        "Wires: beige where the design uses them, grey where not; the values are in the small boxes, and 0/1 colours the wires by them. Bold: the way the value takes now. A change flashes orange."
    };
    ui.add(egui::Label::new(egui::RichText::new(caption).small().weak()).wrap());
}

/// A name with its numbers padded, so that `ADA10` sorts after `ADA9`.
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

/// A name cut to `n` characters.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Logic.

/// A LUT: 16 cells and a tree of muxes; in CCU2 mode the adder's xor and
/// carry mux beside it.
fn lut(ui: &mut egui::Ui, l: &Lut, live: &Live) {
    let ccu2 = l.carry.is_some();
    let h = if ccu2 { 118.0 } else { 104.0 };
    let sh = Sheet::new(ui, h, live);
    let ins: Vec<W> = l.inputs.iter().map(|&i| sh.input(i)).collect();
    let tied: Vec<bool> = l
        .inputs
        .iter()
        .map(|i| matches!(i, Input::Tied(_)))
        .collect();
    let cell_y = |i: usize| 8.0 + 5.0 * i as f32;
    let cells: Vec<V> = (0..16)
        .map(|i| match l.init {
            Some(m) => V::of(Some(m >> i & 1 == 1)),
            None => V::X,
        })
        .collect();
    let readable = |row: usize| {
        l.inputs.iter().enumerate().all(|(i, x)| match x {
            Input::Tied(v) => (row >> i & 1 == 1) == *v,
            Input::Wire(_) => true,
        })
    };
    let address = ins
        .iter()
        .enumerate()
        .try_fold(0usize, |a, (i, w)| Some(a | usize::from(w.v.bit()?) << i));
    let xs: [f32; 4] = if ccu2 {
        [20.0, 31.0, 42.0, 53.0]
    } else {
        [22.0, 38.0, 54.0, 70.0]
    };
    const MW: f32 = 4.0;
    // Each level's outputs: where and what.
    let mut ys: Vec<Vec<f32>> = vec![(0..16).map(cell_y).collect()];
    let mut vals: Vec<Vec<V>> = vec![cells.clone()];
    for level in 1..=4 {
        let sel = ins[level - 1].v;
        let n = 16 >> level;
        let (py, pv) = (&ys[level - 1], &vals[level - 1]);
        let y: Vec<f32> = (0..n).map(|j| (py[2 * j] + py[2 * j + 1]) / 2.0).collect();
        let v: Vec<V> = (0..n).map(|j| mux(sel, pv[2 * j], pv[2 * j + 1])).collect();
        ys.push(y);
        vals.push(v);
    }
    let bottom = h - 19.0;
    sh.small(
        9.0,
        3.0,
        if l.ram { "RAM" } else { "INIT" },
        Align2::CENTER_CENTER,
        DIM,
    );
    // The selects, behind the muxes they run through.
    for level in 1..=4 {
        let x = xs[level - 1] + MW / 2.0;
        let w = ins[level - 1];
        sh.line(&[(x, bottom), (x, ys[level][0])], w, false);
        sh.text(
            x,
            bottom + 1.5,
            &LETTERS[level - 1].to_string(),
            Align2::CENTER_TOP,
            TEXT,
        );
        sh.tag(x, bottom + 8.5, w, Align2::CENTER_CENTER);
        if tied[level - 1] {
            sh.small(x, bottom + 13.0, "tied", Align2::CENTER_TOP, DIM);
        }
    }
    // The cells.
    for (i, &v) in cells.iter().enumerate() {
        let y = cell_y(i);
        sh.line(&[(12.0, y), (xs[0], y)], W::c(v), address == Some(i));
        let rect = Rect::from_min_max(sh.p(6.0, y - 2.0), sh.p(12.0, y + 2.0));
        // A bit of INIT: with 0/1 the colour of its value, else beige as
        // what the LUT uses.
        let (fill, fore) = if sh.live.values {
            (
                sh.colour(W::c(v)),
                if v == V::One {
                    SITE_TEXT
                } else {
                    Color32::from_rgb(245, 236, 216)
                },
            )
        } else {
            (SITE, SITE_TEXT)
        };
        let fill = if readable(i) {
            fill
        } else {
            fill.gamma_multiply(0.3)
        };
        sh.painter.rect_filled(rect, 1.5, fill);
        if address == Some(i) {
            sh.painter
                .rect_stroke(rect, 1.5, Stroke::new(2.0, NET), StrokeKind::Outside);
        }
        let text = match v {
            V::One => "1",
            V::Zero => "0",
            _ => "?",
        };
        sh.painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(10.0),
            if readable(i) {
                fore
            } else {
                fore.gamma_multiply(0.5)
            },
        );
        sh.small(5.0, y, &i.to_string(), Align2::RIGHT_CENTER, DIM);
    }
    // The muxes, level by level, and the wires out of them.
    let end = if ccu2 { 57.0 } else { 80.0 };
    for level in 1..=4 {
        let chosen = ins[level - 1].v.bit().map(usize::from);
        for j in 0..(16 >> level) {
            let (a, b) = (ys[level - 1][2 * j], ys[level - 1][2 * j + 1]);
            let (_, out) = sh.mux(
                (xs[level - 1], xs[level - 1] + MW),
                (a - 1.8, b + 1.8),
                &[a, b],
                chosen,
            );
            let to = if level < 4 { xs[level] } else { end };
            let on = address.is_some_and(|x| x >> level == j);
            sh.line(&[out, (to, out.1)], W::c(vals[level][j]), on);
        }
    }
    let y_f = ys[4][0];
    let p = vals[4][0];
    let f = match sh.wire(l.out) {
        w if w.v == V::Off => W::c(p),
        w => w,
    };
    if !ccu2 {
        sh.text(82.0, y_f, "F", Align2::LEFT_CENTER, TEXT);
        sh.tag(92.0, y_f, f, Align2::CENTER_CENTER);
        if !l.taken {
            sh.small(82.0, y_f + 5.0, "no route", Align2::LEFT_CENTER, DIM);
        }
        return;
    }
    // CCU2: F = P xor the carry in; the carry out is the carry in where P is
    // 1, else G, INIT's low four that B A read.
    let Some((cin_wire, inject)) = l.carry else {
        return;
    };
    let cin = sh.wire(cin_wire);
    let cin_v = if cin.v == V::Off { V::X } else { cin.v };
    sh.small(58.5, y_f - 1.0, "P", Align2::CENTER_BOTTOM, DIM);
    // P into the xor, and down to the carry mux's select.
    sh.line(
        &[
            (57.0, y_f),
            (60.0, y_f),
            (60.0, y_f - 1.8),
            (66.0, y_f - 1.8),
        ],
        W::c(p),
        true,
    );
    let (cx0, cx1, cy0, cy1) = (82.0, 86.0, 59.0, 75.0);
    let sel = (
        (cx0 + cx1) / 2.0,
        cy1 - ((cy1 - cy0) * 0.22f32).min(2.5) / 2.0,
    );
    sh.line(
        &[(60.0, y_f), (60.0, 81.0), (sel.0, 81.0), sel],
        W::c(p),
        false,
    );
    sh.dot(60.0, y_f, W::c(p));
    // The carry in: up to the xor, and to the carry mux.
    let cin_x = 63.0;
    sh.line(
        &[(cin_x, bottom), (cin_x, 71.0), (cx0, 71.0)],
        cin,
        p.bit() == Some(true),
    );
    if inject {
        sh.cut(&[(cin_x, 71.0), (cin_x, y_f + 1.8), (66.0, y_f + 1.8)]);
        sh.small(
            cin_x + 1.0,
            y_f + 6.0,
            "cut: INJECT1",
            Align2::LEFT_CENTER,
            DIM,
        );
    } else {
        sh.line(
            &[(cin_x, 71.0), (cin_x, y_f + 1.8), (66.0, y_f + 1.8)],
            cin,
            false,
        );
    }
    sh.dot(cin_x, 71.0, cin);
    sh.text(cin_x, bottom + 1.5, "Cin", Align2::CENTER_TOP, TEXT);
    sh.tag(cin_x, bottom + 8.5, cin, Align2::CENTER_CENTER);
    sh.xor((66.0, 72.0), (y_f - 3.4, y_f + 3.4));
    let sum = if inject { p } else { xor(p, cin_v) };
    let f = if f.v == V::Off || l.out.is_none() {
        W::c(sum)
    } else {
        f
    };
    sh.line(&[(72.0, y_f), (84.0, y_f)], f, true);
    sh.text(86.0, y_f, "F", Align2::LEFT_CENTER, TEXT);
    sh.tag(95.0, y_f, f, Align2::CENTER_CENTER);
    if !l.taken {
        sh.small(86.0, y_f + 5.0, "no route", Align2::LEFT_CENTER, DIM);
    }
    // G: the generate, INIT[B A] (0 with INJECT1).
    let g = if inject {
        V::Zero
    } else {
        match (ins[0].v.bit(), ins[1].v.bit()) {
            (Some(a), Some(b)) => cells[usize::from(a) | usize::from(b) << 1],
            _ => V::X,
        }
    };
    sh.block(66.0, 60.0, 76.0, 66.0, false);
    sh.text(71.0, 63.0, "G", Align2::CENTER_CENTER, TEXT);
    sh.small(71.0, 67.0, "INIT[BA]", Align2::CENTER_TOP, DIM);
    let take = p.bit().map(usize::from);
    sh.line(&[(76.0, 63.0), (cx0, 63.0)], W::c(g), take == Some(0));
    sh.mux((cx0, cx1), (cy0, cy1), &[63.0, 71.0], take);
    let cout = match sh.wire(l.carry_out) {
        w if w.v == V::Off => W::c(mux(p, g, cin_v)),
        w => w,
    };
    sh.line(&[(cx1, 67.0), (92.0, 67.0)], cout, true);
    sh.small(89.0, 63.5, "Cout", Align2::CENTER_BOTTOM, TEXT);
    sh.tag(96.0, 67.0, cout, Align2::CENTER_CENTER);
}

/// A RAMW half: four buffers passing its inputs on.
fn ramw(ui: &mut egui::Ui, outputs: &[(String, Option<u32>, char, Input)], live: &Live) {
    let sh = Sheet::new(ui, 46.0, live);
    for (i, (name, wire, from, input)) in outputs.iter().enumerate() {
        let y = 8.0 + 10.0 * i as f32;
        let a = sh.input(*input);
        let o = sh.wire(*wire);
        sh.text(2.0, y, &from.to_string(), Align2::LEFT_CENTER, TEXT);
        sh.tag(10.0, y, a, Align2::CENTER_CENTER);
        sh.line(&[(14.0, y), (40.0, y)], a, false);
        sh.buffer(40.0, 47.0, y, 6.0);
        sh.line(&[(47.0, y), (68.0, y)], o, false);
        sh.tag(72.0, y, o, Align2::CENTER_CENTER);
        sh.small(
            76.0,
            y,
            name.trim_end_matches("C_SLICE"),
            Align2::LEFT_CENTER,
            TEXT,
        );
    }
}

/// A flip-flop: the D, enable and set/reset muxes and the register.
fn flip_flop(ui: &mut egui::Ui, f: &FlipFlop, live: &Live) {
    let sh = Sheet::new(ui, 66.0, live);
    let known = |v: V| if v == V::Off { V::X } else { v };
    let (m, di, q, clk) = (sh.wire(f.m), sh.wire(f.di), sh.wire(f.q), sh.wire(f.clock));
    let sd = usize::from(!f.from_m);
    let d = if f.from_m { m } else { di };
    let ce = sh.input(f.enable);
    let ce = if f.invert_enable { ce.not() } else { ce };
    // An LSR the design leaves alone reads 0.
    let lsr = match f.lsr {
        None => W::c(V::Zero),
        Some(_) => sh.wire(f.lsr),
    };
    let lsr = if f.invert_lsr { lsr.not() } else { lsr };
    let set = match f.preload {
        Some(w) => sh.wire(w),
        None => W::c(V::of(Some(f.set))),
    };
    let hold = mux(ce.v, known(q.v), known(d.v));
    let next = mux(lsr.v, hold, known(set.v));
    let (take_ce, take_lsr) = (ce.v.bit().map(usize::from), lsr.v.bit().map(usize::from));
    // REG.SD.
    sh.text(1.0, 12.0, "M", Align2::LEFT_CENTER, TEXT);
    sh.text(1.0, 22.0, "DI", Align2::LEFT_CENTER, TEXT);
    sh.line(
        &[(7.0, 12.0), (18.0, 12.0)],
        m,
        sd == 0 && take_ce == Some(1) && take_lsr == Some(0),
    );
    sh.line(
        &[(7.0, 22.0), (18.0, 22.0)],
        di,
        sd == 1 && take_ce == Some(1) && take_lsr == Some(0),
    );
    let (_, d_out) = sh.mux((18.0, 22.0), (8.0, 26.0), &[12.0, 22.0], Some(sd));
    sh.small(20.0, 28.0, &format!("SD={sd}"), Align2::CENTER_TOP, DIM);
    // CE: D, or Q again.
    let (ce_sel, ce_out) = sh.mux((36.0, 40.0), (6.0, 24.0), &[10.0, 20.0], take_ce);
    sh.line(
        &[d_out, (30.0, 17.0), (30.0, 20.0), (36.0, 20.0)],
        d,
        take_ce == Some(1) && take_lsr == Some(0),
    );
    sh.line(
        &[
            (86.0, 22.0),
            (86.0, 3.0),
            (32.0, 3.0),
            (32.0, 10.0),
            (36.0, 10.0),
        ],
        q,
        take_ce == Some(0) && take_lsr == Some(0),
    );
    sh.dot(86.0, 22.0, q);
    sh.line(&[(ce_sel.0, 54.0), ce_sel], ce, false);
    sh.text(ce_sel.0, 55.0, "CE", Align2::CENTER_TOP, TEXT);
    sh.tag(ce_sel.0, 62.0, ce, Align2::CENTER_CENTER);
    if matches!(f.enable, Input::Tied(_)) {
        sh.small(ce_sel.0 + 2.5, 62.0, "tied", Align2::LEFT_CENTER, DIM);
    }
    // LSR: what CE gives, or REGSET's value (M with PRLD).
    let (lsr_sel, lsr_out) = sh.mux((50.0, 54.0), (12.0, 32.0), &[15.0, 28.0], take_lsr);
    sh.line(&[ce_out, (50.0, 15.0)], W::c(hold), take_lsr == Some(0));
    sh.block(42.0, 25.5, 47.5, 30.5, false);
    sh.text(
        44.75,
        28.0,
        if f.preload.is_some() {
            "M"
        } else if f.set {
            "1"
        } else {
            "0"
        },
        Align2::CENTER_CENTER,
        TEXT,
    );
    sh.line(&[(47.5, 28.0), (50.0, 28.0)], set, take_lsr == Some(1));
    sh.line(&[(lsr_sel.0, 54.0), lsr_sel], lsr, false);
    sh.text(lsr_sel.0, 55.0, "LSR", Align2::CENTER_TOP, TEXT);
    sh.tag(lsr_sel.0, 62.0, lsr, Align2::CENTER_CENTER);
    if f.asynchronous {
        sh.small(lsr_sel.0 + 2.5, 62.0, "async", Align2::LEFT_CENTER, DIM);
    }
    // The register and its clock.
    sh.line(&[lsr_out, (62.0, 22.0)], W::c(next), true);
    sh.site((62.0, 10.0, 78.0, 34.0), true, q, "");
    let ink = sh.ink(q);
    sh.small(63.0, 22.0, "D", Align2::LEFT_CENTER, ink);
    sh.small(77.0, 22.0, "Q", Align2::RIGHT_CENTER, ink);
    sh.painter.add(Shape::line(
        vec![sh.p(62.0, 28.0), sh.p(64.5, 30.0), sh.p(62.0, 32.0)],
        Stroke::new(1.2, ink),
    ));
    sh.line(&[(58.0, 54.0), (58.0, 30.0), (62.0, 30.0)], clk, false);
    if f.invert_clock {
        sh.bubble(61.0, 30.0);
    }
    sh.text(58.0, 55.0, "CLK", Align2::CENTER_TOP, TEXT);
    sh.tag(58.0, 62.0, clk, Align2::CENTER_CENTER);
    sh.small(
        70.0,
        37.0,
        &format!(
            "next {}: {}",
            if f.invert_clock { "fall" } else { "rise" },
            match next {
                V::One => "1",
                V::Zero => "0",
                _ => "x",
            }
        ),
        Align2::CENTER_TOP,
        DIM,
    );
    sh.line(&[(78.0, 22.0), (92.0, 22.0)], q, false);
    sh.text(93.0, 17.5, "Q", Align2::CENTER_BOTTOM, TEXT);
    sh.tag(95.0, 22.0, q, Align2::CENTER_CENTER);
}

/// The wide muxes of a slice.
fn wide(ui: &mut egui::Ui, m: &Mux, live: &Live) {
    let sh = Sheet::new(ui, 72.0, live);
    let (n0, n1) = (2 * m.slice, 2 * m.slice + 1);
    let letter = LETTERS[m.slice];
    for (top, wires, names) in [
        (
            5.0,
            m.f5,
            [
                format!("F{n0} (LUT{n0})"),
                format!("F{n1} (LUT{n1})"),
                format!("M{n0}"),
                "F5".to_string(),
            ],
        ),
        (
            39.0,
            m.fx,
            [
                format!("FXA{letter}"),
                format!("FXB{letter}"),
                format!("M{n1}"),
                "FX".to_string(),
            ],
        ),
    ] {
        let [sel, low, high, o] = wires.map(|w| sh.wire(w));
        let take = sel.v.bit().map(usize::from);
        let (a, b) = (top + 5.0, top + 19.0);
        sh.small(2.0, a, &names[0], Align2::LEFT_CENTER, TEXT);
        sh.small(2.0, b, &names[1], Align2::LEFT_CENTER, TEXT);
        sh.line(&[(28.0, a), (44.0, a)], low, take == Some(0));
        sh.line(&[(28.0, b), (44.0, b)], high, take == Some(1));
        let (s, out) = sh.mux((44.0, 50.0), (top, top + 24.0), &[a, b], take);
        sh.line(&[(s.0, top + 28.0), s], sel, false);
        sh.small(s.0 + 1.5, top + 28.0, &names[2], Align2::LEFT_CENTER, TEXT);
        sh.tag(s.0 + 10.0, top + 28.0, sel, Align2::LEFT_CENTER);
        let shown = if o.v == V::Off {
            W::c(mux(sel.v, low.v, high.v))
        } else {
            o
        };
        sh.line(&[out, (84.0, out.1)], shown, take.is_some());
        sh.text(86.0, out.1, &names[3], Align2::LEFT_CENTER, TEXT);
        sh.tag(96.0, out.1, shown, Align2::CENTER_CENTER);
    }
}

/// A slice's control set: the clock and set/reset the slice takes of the
/// tile's two, whether it inverts them, and its clock enable.
fn control(ui: &mut egui::Ui, c: &Control, live: &Live) {
    let sh = Sheet::new(ui, 88.0, live);
    // Clock and set/reset: two of the tile's, one picked by an arc.
    for (top, ins, choice, out, invert, names) in [
        (
            3.0,
            c.clk,
            c.clk_choice,
            c.clk_out,
            c.clk_inv,
            ["CLK0", "CLK1", "MUXCLK", "CLK", "INV: falling edge"],
        ),
        (
            31.0,
            c.lsr,
            c.lsr_choice,
            c.lsr_out,
            c.lsr_inv,
            ["LSR0", "LSR1", "MUXLSR", "LSR", "INV"],
        ),
    ] {
        let (a, b) = (top + 4.0, top + 14.0);
        for (k, y) in [(0, a), (1, b)] {
            let w = sh.wire(ins[k]);
            sh.text(1.0, y, names[k], Align2::LEFT_CENTER, TEXT);
            sh.tag(13.0, y, w, Align2::CENTER_CENTER);
            sh.line(&[(16.0, y), (26.0, y)], w, choice == Some(k));
        }
        let (_, o) = sh.mux((26.0, 30.0), (top, top + 18.0), &[a, b], choice);
        sh.small(
            28.0,
            top + 19.5,
            &format!("{} (arc)", names[2]),
            Align2::CENTER_TOP,
            DIM,
        );
        // A set/reset the design leaves alone reads 0.
        let loose = out.is_none() && names[3] == "LSR";
        let w = if loose { W::c(V::Zero) } else { sh.wire(out) };
        if loose {
            sh.small(82.0, o.1 + 4.5, "not connected", Align2::LEFT_CENTER, DIM);
        }
        if invert {
            sh.line(&[o, (44.0, o.1)], w, true);
            sh.buffer(44.0, 49.5, o.1, 5.0);
            sh.bubble(50.3, o.1);
            sh.line(&[(51.1, o.1), (80.0, o.1)], w.not(), true);
            sh.small(47.0, o.1 - 3.6, names[4], Align2::CENTER_BOTTOM, DIM);
        } else {
            sh.line(&[o, (80.0, o.1)], w, true);
        }
        let shown = if invert { w.not() } else { w };
        sh.text(82.0, o.1, names[3], Align2::LEFT_CENTER, TEXT);
        sh.tag(96.0, o.1, shown, Align2::CENTER_CENTER);
    }
    if c.lsr_async {
        sh.small(82.0, 36.0, "async", Align2::LEFT_CENTER, DIM);
    }
    // CE: 1, 0, its wire, or the wire inverted, as CEMUX says.
    let ce = sh.wire(c.ce);
    let take = match c.ce_mode.as_str() {
        "1" => Some(0),
        "0" => Some(1),
        "CE" => Some(2),
        "INV" => Some(3),
        _ => None,
    };
    let ys = [60.0, 66.0, 72.0, 78.0];
    for (k, v) in [(0, true), (1, false)] {
        sh.block(19.0, ys[k] - 2.2, 23.0, ys[k] + 2.2, false);
        sh.small(
            21.0,
            ys[k],
            if v { "1" } else { "0" },
            Align2::CENTER_CENTER,
            TEXT,
        );
        sh.line(
            &[(23.0, ys[k]), (26.0, ys[k])],
            W::c(V::of(Some(v))),
            take == Some(k),
        );
    }
    sh.text(1.0, ys[2], "CE", Align2::LEFT_CENTER, TEXT);
    sh.tag(9.0, ys[2], ce, Align2::CENTER_CENTER);
    sh.line(&[(12.0, ys[2]), (26.0, ys[2])], ce, take == Some(2));
    sh.line(
        &[(15.0, ys[2]), (15.0, ys[3]), (18.0, ys[3])],
        ce,
        take == Some(3),
    );
    sh.dot(15.0, ys[2], ce);
    sh.buffer(18.0, 22.4, ys[3], 3.6);
    sh.bubble(23.2, ys[3]);
    sh.line(&[(24.0, ys[3]), (26.0, ys[3])], ce.not(), take == Some(3));
    let (_, o) = sh.mux((26.0, 30.0), (57.0, 81.0), &ys, take);
    sh.small(
        28.0,
        82.5,
        &format!("CEMUX={}", c.ce_mode),
        Align2::CENTER_TOP,
        DIM,
    );
    let out = match take {
        Some(0) => W::c(V::One),
        Some(1) => W::c(V::Zero),
        Some(3) => ce.not(),
        _ => ce,
    };
    sh.line(&[o, (80.0, o.1)], out, true);
    sh.text(82.0, o.1, "CE", Align2::LEFT_CENTER, TEXT);
    sh.tag(96.0, o.1, out, Align2::CENTER_CENTER);
}

/// A whole slice: its LUTs, the F5 mux, the FX mux, its flip-flops, the
/// carry along it and its control set.
fn slice(ui: &mut egui::Ui, v: &SliceView, live: &Live) {
    let sh = Sheet::new(ui, 100.0, live);
    let (n0, n1) = (2 * v.slice, 2 * v.slice + 1);
    if v.mode == "CCU2" {
        let (fci, fco) = (sh.wire(v.fci), sh.wire(v.fco));
        sh.line(&[(6.0, 2.0), (6.0, 24.0)], fci, false);
        sh.line(&[(6.0, 24.0), (6.0, 48.0)], fco, false);
        sh.small(7.5, 3.0, "FCI", Align2::LEFT_CENTER, DIM);
        sh.small(7.5, 47.0, "FCO", Align2::LEFT_CENTER, DIM);
    }
    // The LUTs into F5.
    let m = [sh.wire(v.m[0]), sh.wire(v.m[1])];
    let take5 = m[0].v.bit().map(usize::from);
    for (k, top) in [(0usize, 6.0), (1, 28.0)] {
        let n = 2 * v.slice + k;
        let f = sh.wire(v.f[k]);
        sh.site((12.0, top, 30.0, top + 14.0), v.luts[k], f, "");
        sh.text(
            21.0,
            top + 5.0,
            &format!("LUT{n}"),
            Align2::CENTER_CENTER,
            if v.luts[k] { sh.ink(f) } else { DIM },
        );
        sh.tag(21.0, top + 10.5, f, Align2::CENTER_CENTER);
        for i in 0..4 {
            let y = top + 2.5 + 3.0 * i as f32;
            sh.line(&[(9.0, y), (12.0, y)], W::c(V::Off), false);
        }
        sh.line(&[(30.0, top + 7.0), (40.0, top + 7.0)], f, take5 == Some(k));
    }
    let f5 = sh.wire(v.f5);
    let (s5, o5) = sh.mux((40.0, 44.0), (9.0, 39.0), &[13.0, 35.0], take5);
    sh.line(&[(s5.0, 47.0), s5], m[0], false);
    sh.small(
        s5.0 + 1.0,
        46.5,
        &format!("M{n0}"),
        Align2::LEFT_CENTER,
        TEXT,
    );
    sh.line(&[o5, (50.0, o5.1)], f5, take5.is_some() && f5.v != V::Off);
    sh.small(51.0, o5.1, "F5", Align2::LEFT_CENTER, TEXT);
    sh.tag(57.5, o5.1, f5, Align2::LEFT_CENTER);
    // FX from the neighbours' F5 and FX.
    let (fxa, fxb, fx) = (sh.wire(v.fxa), sh.wire(v.fxb), sh.wire(v.fx));
    let takex = m[1].v.bit().map(usize::from);
    sh.small(14.0, 58.0, "FXA", Align2::LEFT_CENTER, TEXT);
    sh.small(14.0, 70.0, "FXB", Align2::LEFT_CENTER, TEXT);
    sh.line(&[(22.0, 58.0), (40.0, 58.0)], fxa, takex == Some(0));
    sh.line(&[(22.0, 70.0), (40.0, 70.0)], fxb, takex == Some(1));
    let (sx, ox) = sh.mux((40.0, 44.0), (54.0, 74.0), &[58.0, 70.0], takex);
    sh.line(&[(sx.0, 80.0), sx], m[1], false);
    sh.small(
        sx.0 + 1.0,
        79.5,
        &format!("M{n1}"),
        Align2::LEFT_CENTER,
        TEXT,
    );
    sh.line(&[ox, (50.0, ox.1)], fx, takex.is_some() && fx.v != V::Off);
    sh.small(51.0, ox.1, "FX", Align2::LEFT_CENTER, TEXT);
    sh.tag(57.5, ox.1, fx, Align2::LEFT_CENTER);
    // The flip-flops, clocked from the control set.
    let clk = sh.wire(v.clk);
    sh.line(&[(63.0, 86.0), (63.0, 15.0)], clk, false);
    for (k, top) in [(0usize, 6.0), (1, 28.0)] {
        let n = 2 * v.slice + k;
        let out = sh.wire(v.q[k]);
        sh.site((68.0, top, 84.0, top + 14.0), v.ffs[k], out, "");
        sh.text(
            76.0,
            top + 4.5,
            &format!("FF{n}"),
            Align2::CENTER_CENTER,
            if v.ffs[k] { sh.ink(out) } else { DIM },
        );
        let (di, q) = (sh.wire(v.di[k]), sh.wire(v.q[k]));
        sh.small(66.0, top + 3.0, "DI", Align2::RIGHT_CENTER, DIM);
        sh.tag(66.5, top + 7.5, di, Align2::RIGHT_CENTER);
        sh.line(&[(66.5, top + 7.5), (68.0, top + 7.5)], di, false);
        sh.line(&[(63.0, top + 11.0), (68.0, top + 11.0)], clk, false);
        sh.line(&[(84.0, top + 7.5), (90.0, top + 7.5)], q, false);
        sh.tag(91.0, top + 7.5, q, Align2::LEFT_CENTER);
        sh.small(91.0, top + 2.0, "Q", Align2::LEFT_CENTER, DIM);
    }
    let _ = n1;
    // The control set.
    sh.block(12.0, 86.0, 84.0, 96.0, false);
    for (x, name, w) in [
        (20.0, "CLK", clk),
        (44.0, "LSR", sh.wire(v.lsr)),
        (68.0, "CE", sh.wire(v.ce)),
    ] {
        sh.small(x, 91.0, name, Align2::RIGHT_CENTER, TEXT);
        sh.tag(x + 1.5, 91.0, w, Align2::LEFT_CENTER);
    }
}

// ---------------------------------------------------------------------------
// Sites.

/// A CIB: each J wire in use, with the input mux that drives it or the
/// block that does, and where it goes.
fn cib(ui: &mut egui::Ui, rows: &[JWire], live: &Live) {
    if rows.is_empty() {
        ui.label("No J wire of this CIB is in use.");
        return;
    }
    let shown = rows.len().min(24);
    let h = 6.0 + shown as f32 * 8.0 + if rows.len() > shown { 6.0 } else { 0.0 };
    let sh = Sheet::new(ui, h, live);
    for (i, j) in rows.iter().take(shown).enumerate() {
        let y = 7.0 + i as f32 * 8.0;
        let w = sh.wire(j.wire);
        if j.input {
            match j.tied {
                Some(v) => {
                    sh.block(25.0, y - 2.2, 30.0, y + 2.2, false);
                    sh.text(
                        27.5,
                        y,
                        if v { "1" } else { "0" },
                        Align2::CENTER_CENTER,
                        TEXT,
                    );
                    sh.line(&[(30.0, y), (34.0, y)], W::c(V::of(Some(v))), true);
                    sh.small(24.0, y, "CIB", Align2::RIGHT_CENTER, DIM);
                }
                None => {
                    sh.small(30.0, y, &cut(&j.from, 20), Align2::RIGHT_CENTER, TEXT);
                    sh.line(&[(31.0, y), (34.0, y)], w, true);
                }
            }
            // The tracks it does not pass.
            for dy in [-2.2, 2.2] {
                sh.line(&[(32.0, y + dy), (34.0, y + dy)], W::c(V::Off), false);
            }
            sh.mux((34.0, 38.0), (y - 3.3, y + 3.3), &[], Some(0));
            if j.choices > 0 {
                sh.small(
                    39.0,
                    y + 0.8,
                    &format!(
                        "1 of {}{}",
                        j.choices,
                        if j.tied.is_some() { "+0/1" } else { "" }
                    ),
                    Align2::LEFT_TOP,
                    DIM,
                );
            }
            sh.line(&[(38.0, y), (64.0, y)], w, false);
        } else {
            sh.small(30.0, y, &cut(&j.from, 20), Align2::RIGHT_CENTER, TEXT);
            sh.line(&[(31.0, y), (64.0, y)], w, false);
            sh.small(39.0, y + 0.8, "from the block", Align2::LEFT_TOP, DIM);
        }
        sh.small(51.0, y - 0.6, &j.name, Align2::CENTER_BOTTOM, TEXT);
        sh.tag(64.0, y, w, Align2::LEFT_CENTER);
        sh.small(69.0, y, &cut(&j.to, 21), Align2::LEFT_CENTER, TEXT);
    }
    if rows.len() > shown {
        sh.small(
            50.0,
            h - 3.0,
            &format!("and {} more", rows.len() - shown),
            Align2::CENTER_CENTER,
            DIM,
        );
    }
}

/// A pad: the tristate driver from I, enabled while T is 0, the pad, and
/// the input buffer back to O.
fn pio(ui: &mut egui::Ui, p: &Pio, live: &Live) {
    let sh = Sheet::new(ui, 56.0, live);
    let (i, t, o) = (sh.wire(p.i), sh.wire(p.t), sh.wire(p.o));
    // The driver drives while T is 0; a T the design leaves alone is 1.
    let drives = match t.v {
        V::Off => false,
        v => v.bit() == Some(false),
    };
    sh.text(1.0, 14.0, "I", Align2::LEFT_CENTER, TEXT);
    sh.small(4.0, 14.0, "PADDO", Align2::LEFT_CENTER, DIM);
    sh.tag(17.0, 14.0, i, Align2::CENTER_CENTER);
    sh.line(&[(20.0, 14.0), (34.0, 14.0)], i, drives);
    sh.text(1.0, 30.0, "T", Align2::LEFT_CENTER, TEXT);
    sh.small(4.0, 30.0, "PADDT", Align2::LEFT_CENTER, DIM);
    sh.tag(17.0, 30.0, t, Align2::CENTER_CENTER);
    sh.line(&[(20.0, 30.0), (39.0, 30.0), (39.0, 18.4)], t, false);
    sh.buffer(34.0, 44.0, 14.0, 10.0);
    sh.bubble(39.0, 17.6);
    let pad = if drives { i } else { o };
    sh.line(
        &[(44.0, 14.0), (70.0, 14.0), (70.0, 24.0)],
        if drives { i } else { W::c(V::Off) },
        drives,
    );
    sh.painter.circle(
        sh.p(70.0, 28.0),
        4.0 * sh.k,
        sh.colour(pad).gamma_multiply(0.5),
        Stroke::new(1.5, EDGE),
    );
    sh.small(
        70.0,
        28.0,
        p.ball.as_deref().unwrap_or("pad"),
        Align2::CENTER_CENTER,
        TEXT,
    );
    if let Some(role) = &p.role {
        sh.small(76.0, 25.0, &cut(role, 18), Align2::LEFT_CENTER, TEXT);
    }
    sh.small(
        76.0,
        31.0,
        if drives { "driven" } else { "not driven" },
        Align2::LEFT_CENTER,
        DIM,
    );
    sh.line(
        &[(70.0, 32.0), (70.0, 42.0), (46.0, 42.0)],
        o,
        !drives && o.v != V::Off,
    );
    sh.buffer(46.0, 36.0, 42.0, 10.0);
    sh.line(&[(36.0, 42.0), (20.0, 42.0)], o, !drives && o.v != V::Off);
    sh.text(1.0, 42.0, "O", Align2::LEFT_CENTER, TEXT);
    sh.small(4.0, 42.0, "PADDI", Align2::LEFT_CENTER, DIM);
    sh.tag(17.0, 42.0, o, Align2::CENTER_CENTER);
}

/// A line of a chip: whether it is an output, its name without the bit
/// number, and its bits with their wires (one, numbered u32::MAX, for a
/// pin with no number).
type Bus = (bool, String, Vec<(u32, Option<u32>)>);

/// Any other site: a chip, its inputs on the left and its outputs on the
/// right, those in use (or the first of them when none is), the pins of a
/// bus on one line as a datasheet draws them (`ADA[13:3]`) with its value.
fn chip(ui: &mut egui::Ui, d: &Details, live: &Live) {
    let mut used: Vec<&Pin> = d.pins.iter().filter(|p| p.wire.is_some()).collect();
    let in_use = used.len();
    if used.is_empty() {
        used = d.pins.iter().take(12).collect();
    }
    used.sort_by_key(|p| natural(&p.name));
    let output = |p: &Pin| p.role == "out" || p.role == "in/out";
    // Pins by side and bus: the name without its number, and each bit.
    let mut rows: Vec<Bus> = Vec::new();
    for p in &used {
        let base = p.name.trim_end_matches(|c: char| c.is_ascii_digit());
        let bit = p.name[base.len()..].parse::<u32>().ok();
        let out = output(p);
        match bit {
            Some(n) if !base.is_empty() => {
                match rows.iter_mut().find(|r| r.0 == out && r.1 == base) {
                    Some(r) => r.2.push((n, p.wire)),
                    None => rows.push((out, base.to_string(), vec![(n, p.wire)])),
                }
            }
            _ => rows.push((out, p.name.clone(), vec![(u32::MAX, p.wire)])),
        }
    }
    let left: Vec<_> = rows.iter().filter(|r| !r.0).collect();
    let right: Vec<_> = rows.iter().filter(|r| r.0).collect();
    const MAX: usize = 28;
    let lines = left.len().min(MAX).max(right.len().min(MAX)).max(3);
    let pitch = 4.6;
    let body = (8.0, 12.0 + lines as f32 * pitch);
    let shown_bits: usize = left
        .iter()
        .take(MAX)
        .chain(right.iter().take(MAX))
        .map(|r| r.2.len())
        .sum();
    let more_used = in_use.saturating_sub(shown_bits);
    let unused = d.pins.len() - in_use - if in_use == 0 { shown_bits } else { 0 };
    let h = body.1 + 8.0;
    let sh = Sheet::new(ui, h, live);
    // A site in use is beige, as the map draws it, and flashes with its
    // pins; its pin names dark on it.
    let ink = if in_use > 0 {
        let glow = d
            .pins
            .iter()
            .filter_map(|q| q.wire)
            .fold(0.0f32, |g, w| g.max((live.glow)(w)));
        let rect = Rect::from_min_max(sh.p(32.0, body.0), sh.p(68.0, body.1));
        sh.painter
            .rect_filled(rect, 2.0, SITE.lerp_to_gamma(CHANGE, glow));
        sh.painter
            .rect_stroke(rect, 2.0, Stroke::new(1.5, SITE_EDGE), StrokeKind::Inside);
        SITE_TEXT
    } else {
        sh.block(32.0, body.0, 68.0, body.1, false);
        TEXT
    };
    sh.painter
        .circle(sh.p(50.0, body.0), 1.6 * sh.k, BACK, Stroke::new(1.2, EDGE));
    sh.painter.rect_filled(
        Rect::from_min_max(sh.p(47.0, body.0 - 2.0), sh.p(53.0, body.0 - 0.1)),
        0.0,
        BACK,
    );
    let title = d.title.split("  ·").next().unwrap_or(&d.title);
    sh.small(50.0, 4.0, title, Align2::CENTER_CENTER, TEXT);
    for (side, list) in [(0, &left), (1, &right)] {
        for (k, (_, base, bits)) in list.iter().take(MAX).enumerate() {
            let y = body.0 + 4.5 + k as f32 * pitch;
            let (stub, name_at, name_align, value_at, value_align) = if side == 0 {
                (
                    (25.0, 32.0),
                    33.0,
                    Align2::LEFT_CENTER,
                    24.0,
                    Align2::RIGHT_CENTER,
                )
            } else {
                (
                    (68.0, 75.0),
                    67.0,
                    Align2::RIGHT_CENTER,
                    76.0,
                    Align2::LEFT_CENTER,
                )
            };
            if bits.len() == 1 {
                let w = sh.wire(bits[0].1);
                let name = if bits[0].0 == u32::MAX {
                    base.clone()
                } else {
                    format!("{base}{}", bits[0].0)
                };
                sh.line(&[(stub.0, y), (stub.1, y)], w, false);
                sh.small(name_at, y, &cut(&name, 10), name_align, ink);
                sh.tag(value_at, y, w, value_align);
                continue;
            }
            // A bus: its bits, the highest first.
            let mut bits = bits.clone();
            bits.sort_by_key(|b| std::cmp::Reverse(b.0));
            let ws: Vec<W> = bits.iter().map(|b| sh.wire(b.1)).collect();
            let (hi, lo) = (bits[0].0, bits[bits.len() - 1].0);
            let label = if (hi - lo) as usize + 1 == bits.len() {
                format!("{base}[{hi}:{lo}]")
            } else {
                format!("{base} ×{}", bits.len())
            };
            let known: Option<Vec<bool>> = ws.iter().map(|w| w.v.bit()).collect();
            let value = match known {
                Some(v) => {
                    let n = v.iter().fold(0u64, |a, &b| a << 1 | u64::from(b));
                    format!("0x{n:0w$X}", w = v.len().div_ceil(4))
                }
                None => ws
                    .iter()
                    .map(|w| match w.v {
                        V::One => '1',
                        V::Zero => '0',
                        V::X => 'x',
                        V::Off => '-',
                    })
                    .collect(),
            };
            let glow = ws.iter().fold(0.0f32, |g, w| g.max(w.glow));
            let first = ws[0].v;
            let colour = if ws.iter().all(|w| w.v == first) {
                sh.colour(W { v: first, glow })
            } else {
                // Bits of several values: as used without 0/1.
                if sh.live.values {
                    Color32::from_gray(185)
                } else {
                    USED
                }
                .lerp_to_gamma(CHANGE, glow)
            };
            sh.painter
                .line_segment([sh.p(stub.0, y), sh.p(stub.1, y)], Stroke::new(3.5, colour));
            let mid = (stub.0 + stub.1) / 2.0;
            sh.painter.line_segment(
                [sh.p(mid - 0.6, y + 1.2), sh.p(mid + 0.6, y - 1.2)],
                Stroke::new(1.2, TEXT),
            );
            sh.small(name_at, y, &cut(&label, 11), name_align, ink);
            sh.small(value_at, y, &cut(&value, 12), value_align, colour);
        }
    }
    let note = match (more_used, unused) {
        (0, 0) => String::new(),
        (0, u) => format!("{u} more pins, not used"),
        (m, 0) => format!("{m} more pins in use"),
        (m, u) => format!("{m} more pins in use, {u} not used"),
    };
    if !note.is_empty() {
        sh.small(50.0, h - 3.5, &note, Align2::CENTER_CENTER, DIM);
    }
}

/// The schematic of an element as an SVG image, drawn by egui without a
/// window: a way to look at the sheets in tests.
#[cfg(test)]
pub(super) fn svg(d: &Details, lit: &[u8], values: bool) -> String {
    use egui::epaint::ColorMode;
    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::dark());
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 1000.0))),
        ..Default::default()
    };
    // The first pass loads the fonts.
    let live = Live {
        lit,
        glow: &|_| 0.0,
        values,
    };
    let _ = ctx.run_ui(input(), |ui| draw(ui, d, &live));
    let out = ctx.run_ui(input(), |ui| draw(ui, d, &live));
    let colour = |c: Color32| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        format!("rgb({r},{g},{b})\" {{}}-opacity=\"{:.3}", a as f32 / 255.0)
    };
    let paint = |what: &str, c: Color32| colour(c).replace("{}", what);
    let mut body = String::new();
    let mut bottom: f32 = 0.0;
    fn walk(shape: &Shape, f: &mut dyn FnMut(&Shape)) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, f)),
            s => f(s),
        }
    }
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut |s| match s {
            Shape::LineSegment { points, stroke } => {
                body.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                    points[0].x, points[0].y, points[1].x, points[1].y, paint("stroke", stroke.color), stroke.width
                ));
            }
            Shape::Path(p) => {
                let pts: Vec<String> = p
                    .points
                    .iter()
                    .map(|q| format!("{},{}", q.x, q.y))
                    .collect();
                let stroke = match p.stroke.color {
                    ColorMode::Solid(c) => c,
                    _ => Color32::GRAY,
                };
                bottom = p.points.iter().fold(bottom, |m, q| m.max(q.y));
                body.push_str(&format!(
                    "<{} points=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>\n",
                    if p.closed { "polygon" } else { "polyline" },
                    pts.join(" "),
                    if p.closed { paint("fill", p.fill) } else { "none\" fill-opacity=\"0".into() },
                    paint("stroke", stroke),
                    p.stroke.width
                ));
            }
            Shape::Rect(r) => {
                bottom = bottom.max(r.rect.max.y);
                body.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                    r.rect.min.x, r.rect.min.y, r.rect.width(), r.rect.height(), r.corner_radius.nw, paint("fill", r.fill), paint("stroke", r.stroke.color), r.stroke.width
                ));
            }
            Shape::Circle(c) => {
                body.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                    c.center.x, c.center.y, c.radius, paint("fill", c.fill), paint("stroke", c.stroke.color), c.stroke.width
                ));
            }
            Shape::Text(t) => {
                let job = &t.galley.job;
                let (size, c) = job.sections.first().map_or((11.0, t.fallback_color), |s| {
                    (s.format.font_id.size, s.format.color)
                });
                let c = t
                    .override_text_color
                    .unwrap_or(if c == Color32::PLACEHOLDER {
                        t.fallback_color
                    } else {
                        c
                    });
                let text = job
                    .text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                bottom = bottom.max(t.pos.y + size);
                body.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" font-family=\"Menlo, monospace\" font-size=\"{}\" fill=\"{}\" xml:space=\"preserve\">{}</text>\n",
                    t.pos.x, t.pos.y + size * 0.8, size * 0.92, paint("fill", c), text
                ));
            }
            _ => {}
        });
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"420\" height=\"{}\">\n<rect width=\"100%\" height=\"100%\" fill=\"rgb(27,27,27)\"/>\n{body}</svg>\n",
        bottom + 12.0
    )
}
