//! The inspector: what a site picked on the map is and how it works, and
//! what it does now in the design. Its pins with their values and the HDL
//! names of their nets, the settings the bitstream gives it, and its
//! insides: a LUT's truth table with the row its inputs read now, what a
//! flip-flop takes at the next clock edge, which input each wide mux
//! passes, and for a CIB each J wire with the track its input mux picks or
//! the constant it ties, and where the wire goes. Above it all, the
//! element's schematic (src/inspector/schematic.rs).
mod schematic;

use crate::fabric_view::{Picked, ONE, UNKNOWN, ZERO};
use ecp5_emu::{
    board,
    fabric::{self, BoxKind, TILE_W},
    fabric_overlay,
    live::Static,
    logic::Logic,
    model::{
        topology::{Endpoint, Location, Wire},
        Direction,
    },
};
use eframe::egui::{self, Color32, RichText};
use std::collections::HashMap;

/// What the inspector asks of the window.
pub enum Action {
    Close,
    /// Keep this net shown on the map.
    Net(u32),
}

/// The element shown, worked out once for each pick.
#[derive(Default)]
pub struct Inspector {
    shown: Option<(Picked, Details)>,
}

/// A pin: its name, what it does, and its wire among the design's (none
/// when the design does not use it).
struct Pin {
    name: String,
    role: String,
    wire: Option<u32>,
}

struct Details {
    title: String,
    kind: String,
    about: Vec<String>,
    pins: Vec<Pin>,
    settings: Vec<(String, String)>,
    inside: Inside,
    notes: Vec<String>,
}

enum Inside {
    Nothing,
    Lut(Lut),
    /// A half of slice C as RAMW: each output it passes on, its wire, and
    /// the input it takes.
    Ramw(Vec<(String, Option<u32>, char, Input)>),
    FlipFlop(FlipFlop),
    Mux(Mux),
    Cib(Vec<JWire>),
    /// For the schematic alone: a slice's control set, a whole slice, a
    /// pad's buffer.
    Control(Control),
    Slice(SliceView),
    Pio(Pio),
}

/// A slice's control set: the tile's two clocks and the one the arc into
/// MUXCLK picks, the slice's clock, and whether CLKMUX inverts it; the
/// same for LSR; its CE wire and CEMUX (1, 0, CE, INV).
struct Control {
    clk: [Option<u32>; 2],
    clk_choice: Option<usize>,
    clk_out: Option<u32>,
    clk_inv: bool,
    lsr: [Option<u32>; 2],
    lsr_choice: Option<usize>,
    lsr_out: Option<u32>,
    lsr_inv: bool,
    lsr_async: bool,
    ce: Option<u32>,
    ce_mode: String,
}

/// A whole slice: which of its parts the design uses, and the wires
/// between them.
struct SliceView {
    slice: usize,
    mode: String,
    luts: [bool; 2],
    ffs: [bool; 2],
    f: [Option<u32>; 2],
    q: [Option<u32>; 2],
    di: [Option<u32>; 2],
    m: [Option<u32>; 2],
    f5: Option<u32>,
    fx: Option<u32>,
    fxa: Option<u32>,
    fxb: Option<u32>,
    fci: Option<u32>,
    fco: Option<u32>,
    clk: Option<u32>,
    lsr: Option<u32>,
    ce: Option<u32>,
}

/// A pad's buffer: I (the value it drives), T (the driver off at 1), O
/// (what it reads), and its ball on the board.
struct Pio {
    i: Option<u32>,
    t: Option<u32>,
    o: Option<u32>,
    ball: Option<String>,
    role: Option<String>,
}

/// An input of a LUT or a flip-flop's enable: a constant its mux ties, or
/// its wire.
#[derive(Clone, Copy)]
enum Input {
    Tied(bool),
    Wire(Option<u32>),
}

struct Lut {
    /// INIT, bit i the value at address i (A the lowest bit); none when a
    /// bit is not known.
    init: Option<u16>,
    /// A, B, C, D.
    inputs: [Input; 4],
    out: Option<u32>,
    /// In CCU2 mode: the carry in, and whether INJECT1 keeps it from F.
    carry: Option<(Option<u32>, bool)>,
    /// In CCU2 mode: the carry out of this half.
    carry_out: Option<u32>,
    /// In DPRAM mode: the LUT is RAM, which the design writes; INIT is its
    /// content at configuration.
    ram: bool,
    /// Whether a route takes F out of the slice; without one it goes only
    /// into the slice (a wide mux), or nowhere (a CCU2 half that only
    /// makes the carry).
    taken: bool,
}

struct FlipFlop {
    data: Option<u32>,
    from_m: bool,
    /// Both inputs REG.SD picks from.
    di: Option<u32>,
    m: Option<u32>,
    clock: Option<u32>,
    invert_clock: bool,
    lsr: Option<u32>,
    invert_lsr: bool,
    asynchronous: bool,
    enable: Input,
    invert_enable: bool,
    /// What LSR at 1 gives: REGSET's value, or M with PRLD.
    set: bool,
    preload: Option<Option<u32>>,
    q: Option<u32>,
}

/// The two wide muxes of a slice: select, low input, high input, output.
struct Mux {
    slice: usize,
    f5: [Option<u32>; 4],
    fx: [Option<u32>; 4],
}

/// A J wire of a CIB: whether it goes into the block beside the CIB, its
/// wire, and what drives it and what it drives, as the tiles name them.
struct JWire {
    name: String,
    input: bool,
    wire: Option<u32>,
    from: String,
    to: String,
    /// The constant its mux ties it to, and how many tracks the mux has.
    tied: Option<bool>,
    choices: usize,
}

impl Inspector {
    /// The panel for `picked`, with `lit` the values of the overlay's wires.
    /// `glow` says how much a change of a wire glows now, as on the map.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        s: &Static,
        lit: &[u8],
        picked: &Picked,
        glow: &dyn Fn(u32) -> f32,
        values: bool,
    ) -> Option<Action> {
        if self.shown.as_ref().is_none_or(|(p, _)| p != picked) {
            self.shown = Some((picked.clone(), details(s, picked)));
        }
        let d = &self.shown.as_ref()?.1;
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading(&d.title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").on_hover_text("Close (Esc)").clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        ui.label(RichText::new(&d.kind).weak());
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Schematic")
                    .default_open(true)
                    .show(ui, |ui| {
                        schematic::draw(ui, d, &schematic::Live { lit, glow, values })
                    });
                egui::CollapsingHeader::new("How it works")
                    .default_open(true)
                    .show(ui, |ui| {
                        for p in &d.about {
                            ui.add(egui::Label::new(p).wrap());
                        }
                    });
                match &d.inside {
                    Inside::Nothing | Inside::Control(_) | Inside::Slice(_) | Inside::Pio(_) => {}
                    Inside::Lut(l) => lut(ui, l, lit),
                    Inside::Ramw(outputs) => ramw(ui, outputs, lit),
                    Inside::FlipFlop(f) => flip_flop(ui, f, lit),
                    Inside::Mux(m) => mux(ui, m, lit),
                    Inside::Cib(rows) => {
                        if let Some(n) = cib(ui, s, rows, lit) {
                            action = Some(Action::Net(n));
                        }
                    }
                }
                if !d.pins.is_empty() {
                    egui::CollapsingHeader::new(format!("Pins ({})", d.pins.len()))
                        .default_open(true)
                        .show(ui, |ui| {
                            if let Some(n) = pins(ui, s, &d.pins, lit) {
                                action = Some(Action::Net(n));
                            }
                        });
                }
                if !d.settings.is_empty() {
                    egui::CollapsingHeader::new(format!("Settings ({})", d.settings.len()))
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::Grid::new("settings").striped(true).show(ui, |ui| {
                                for (name, value) in &d.settings {
                                    ui.label(RichText::new(name).monospace());
                                    ui.label(RichText::new(value).monospace().strong());
                                    ui.end_row();
                                }
                            });
                        });
                }
                for n in &d.notes {
                    ui.add(egui::Label::new(RichText::new(n).small().weak()).wrap());
                }
            });
        action
    }
}

// ---------------------------------------------------------------------------
// Values.

/// A value as a small coloured chip: 1, 0, x (not known), or a dash where
/// the design does not use the wire.
fn chip(ui: &mut egui::Ui, value: Option<u8>) {
    let (text, back, fore) = match value {
        Some(1) => ("1", ONE, Color32::from_rgb(40, 32, 20)),
        Some(0) => ("0", ZERO, Color32::from_rgb(245, 236, 216)),
        Some(_) => ("x", UNKNOWN, Color32::from_rgb(30, 24, 50)),
        None => ("–", Color32::from_gray(50), Color32::from_gray(150)),
    };
    ui.label(
        RichText::new(format!(" {text} "))
            .monospace()
            .strong()
            .background_color(back)
            .color(fore),
    )
    .on_hover_text(match value {
        Some(1) | Some(0) => "its value now",
        Some(_) => "not known: the design does not drive it, or it has not settled",
        None => "the design does not use this wire",
    });
}

fn read(lit: &[u8], wire: Option<u32>) -> Option<u8> {
    wire.and_then(|w| lit.get(w as usize).copied())
}

fn bit(lit: &[u8], wire: Option<u32>) -> Option<bool> {
    match read(lit, wire) {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

fn input(lit: &[u8], i: Input) -> Option<bool> {
    match i {
        Input::Tied(v) => Some(v),
        Input::Wire(w) => bit(lit, w),
    }
}

fn b(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "x",
    }
}

/// The HDL names of a wire's net, a few of them.
fn names(s: &Static, wire: u32) -> Option<String> {
    let net = *s.overlay.nets.get(wire as usize)?;
    let list = s.net_names.get(&net)?;
    let shown: Vec<&str> = list.iter().take(3).map(String::as_str).collect();
    Some(format!(
        "{}{}",
        shown.join(", "),
        if list.len() > 3 {
            format!(" +{}", list.len() - 3)
        } else {
            String::new()
        }
    ))
}

/// A link to a wire's net: its HDL names, or "net" without any. A click
/// keeps the net shown on the map. The names are cut to the width left in
/// the row, the whole of them on hover: a large design's names (a cell of
/// a generated grid, say) would widen the panel past the window.
fn net_link(ui: &mut egui::Ui, s: &Static, wire: Option<u32>) -> Option<u32> {
    let wire = wire?;
    let net = *s.overlay.nets.get(wire as usize)?;
    let text = names(s, wire).unwrap_or_else(|| "net".into());
    let wrap = ui.style().wrap_mode;
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    let link = ui.link(RichText::new(&text).small());
    ui.style_mut().wrap_mode = wrap;
    link.on_hover_text(format!("{text}\n\nShow this net on the map"))
        .clicked()
        .then_some(net)
}

fn pins(ui: &mut egui::Ui, s: &Static, pins: &[Pin], lit: &[u8]) -> Option<u32> {
    let mut picked = None;
    // The count of columns bounds the last, the net, to the panel.
    egui::Grid::new("pins")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            for p in pins {
                ui.label(RichText::new(&p.name).monospace());
                chip(ui, read(lit, p.wire));
                ui.label(RichText::new(&p.role).small());
                if let Some(n) = net_link(ui, s, p.wire) {
                    picked = Some(n);
                }
                ui.end_row();
            }
        });
    picked
}

// ---------------------------------------------------------------------------
// The insides, live.

fn ramw(ui: &mut egui::Ui, outputs: &[(String, Option<u32>, char, Input)], lit: &[u8]) {
    egui::CollapsingHeader::new("Now")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("ramw").striped(true).show(ui, |ui| {
                for (name, wire, from, i) in outputs {
                    ui.label(RichText::new(name).monospace());
                    chip(ui, read(lit, *wire));
                    ui.label(
                        RichText::new(format!("← input {from} = {}", b(input(lit, *i)))).small(),
                    );
                    ui.end_row();
                }
            });
        });
}

fn lut(ui: &mut egui::Ui, l: &Lut, lit: &[u8]) {
    let title = if l.ram {
        "Content at configuration"
    } else {
        "Truth table"
    };
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            if l.ram {
                ui.add(
                    egui::Label::new(
                        RichText::new("This LUT is RAM: the table is what the bitstream stored, and the design writes it while it runs. F is the bit stored at the address now, which may no longer be the one shown.")
                            .small(),
                    )
                    .wrap(),
                );
            }
            let Some(init) = l.init else {
                ui.label("INIT is not known: some of its bits are not in the bitstream.");
                return;
            };
            let values: Vec<Option<bool>> = l.inputs.iter().map(|&i| input(lit, i)).collect();
            let address = values
                .iter()
                .enumerate()
                .try_fold(0usize, |a, (i, v)| Some(a | (usize::from((*v)?) << i)));
            // Rows a tied input never lets the LUT read.
            let readable = |row: usize| {
                l.inputs.iter().enumerate().all(|(i, x)| match x {
                    Input::Tied(v) => (row >> i & 1 == 1) == *v,
                    Input::Wire(_) => true,
                })
            };
            ui.label(
                RichText::new(format!("INIT = 0x{init:04X}"))
                    .monospace()
                    .strong(),
            );
            if let Some(f) = function(init, &l.inputs) {
                ui.label(if l.carry.is_some() {
                    format!("INIT as a LUT4: {f} (F is that xor the carry in)")
                } else {
                    format!("F = {f}")
                });
            }
            egui::Grid::new("truth").spacing([10.0, 0.0]).show(ui, |ui| {
                for h in ["D", "C", "B", "A", "", "INIT"] {
                    ui.label(RichText::new(h).monospace().weak());
                }
                ui.end_row();
                // Rows the LUT cannot reach, quieter than the text on
                // either theme.
                let faint = if ui.visuals().dark_mode { Color32::from_gray(80) } else { Color32::from_gray(175) };
                for row in (0..16).rev() {
                    let now = address == Some(row);
                    let dim = !readable(row);
                    let style = |t: String| {
                        let t = RichText::new(t).monospace();
                        if now {
                            t.strong().color(Color32::BLACK).background_color(ONE)
                        } else if dim {
                            t.color(faint)
                        } else {
                            t
                        }
                    };
                    for i in (0..4).rev() {
                        ui.label(style(format!("{}", row >> i & 1)));
                    }
                    ui.label(style(format!("{row:>2}")));
                    ui.label(style(format!("{}", init >> row & 1)));
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Grey rows: an input tied to a constant never reads them. The bright row: the one D C B A read now.").small().weak());
            let inputs: Vec<String> = ["A", "B", "C", "D"]
                .iter()
                .zip(&l.inputs)
                .zip(&values)
                .map(|((n, i), v)| match i {
                    Input::Tied(_) => format!("{n}={} (tied)", b(*v)),
                    Input::Wire(_) => format!("{n}={}", b(*v)),
                })
                .collect();
            let f = bit(lit, l.out);
            let reads = match address {
                Some(a) => format!("address {a}: INIT[{a}] = {}", init >> a & 1),
                None => "an address not known".to_string(),
            };
            ui.label(format!(
                "Now {} → {reads}; F = {}{}",
                inputs.join(" "),
                b(f),
                if l.taken { "" } else { " (no route takes it)" }
            ));
            if let Some((carry, inject)) = l.carry {
                let c = bit(lit, carry);
                ui.label(if inject {
                    format!("Carry in {} does not reach F: INJECT1 = YES.", b(c))
                } else {
                    format!("Carry in {}: F = INIT[address] xor carry in.", b(c))
                });
                // The carry out: the carry in where INIT's bit is 1
                // (propagate), else INIT's low four read by B A (generate).
                let generate = match (values[0], values[1]) {
                    (Some(a), Some(bb)) if !inject => {
                        let g = usize::from(a) | usize::from(bb) << 1;
                        format!("INIT[{g}] = {}", init >> g & 1)
                    }
                    (Some(_), Some(_)) => "0 (INJECT1 = YES)".to_string(),
                    _ => "not known".to_string(),
                };
                let how = match address.map(|a| init >> a & 1) {
                    Some(1) => "INIT's bit is 1: it passes the carry in".to_string(),
                    Some(_) => format!("INIT's bit is 0: it generates {generate}"),
                    None => "how is not known".to_string(),
                };
                ui.label(format!("Carry out {} ({how}).", b(bit(lit, l.carry_out))));
                if !l.taken {
                    ui.label(
                        RichText::new("No route takes F: this half works for its carry. Its box shows F, and flashes when F or the carry changes.")
                            .strong(),
                    );
                }
            }
        });
}

/// A plain name for a LUT's function of the inputs it reads, when it has
/// one: a constant, a wire, its inverse, or an AND, OR or XOR of them all
/// (or the inverse of one).
fn function(init: u16, inputs: &[Input; 4]) -> Option<String> {
    let names = ["A", "B", "C", "D"];
    let free: Vec<usize> = (0..4)
        .filter(|&i| matches!(inputs[i], Input::Wire(_)))
        .collect();
    let at = |bits: usize| -> bool {
        let mut a = 0;
        for (i, x) in inputs.iter().enumerate() {
            let v = match x {
                Input::Tied(v) => *v,
                Input::Wire(_) => bits >> free.iter().position(|&f| f == i).unwrap() & 1 == 1,
            };
            a |= usize::from(v) << i;
        }
        init >> a & 1 == 1
    };
    let n = free.len();
    let table: Vec<bool> = (0..1 << n).map(at).collect();
    // The inputs F depends on.
    let used: Vec<usize> = (0..n)
        .filter(|&k| (0..1 << n).any(|x| table[x] != table[x ^ (1 << k)]))
        .collect();
    if used.is_empty() {
        return Some(if table[0] { "1".into() } else { "0".into() });
    }
    let vars: Vec<&str> = used.iter().map(|&k| names[free[k]]).collect();
    // The function of the inputs it depends on alone.
    let of = |x: usize| {
        let mut full = 0;
        for (j, &k) in used.iter().enumerate() {
            full |= (x >> j & 1) << k;
        }
        table[full]
    };
    let m = used.len();
    let all = (1usize << m) - 1;
    let f: Vec<bool> = (0..=all).map(of).collect();
    let and = (0..=all).all(|x| f[x] == (x == all));
    let or = (0..=all).all(|x| f[x] == (x != 0));
    let xor = (0..=all).all(|x| f[x] == (x.count_ones() % 2 == 1));
    let join = |op: &str| vars.join(op);
    let name = if m == 1 {
        if f[1] {
            vars[0].to_string()
        } else {
            format!("NOT {}", vars[0])
        }
    } else if and {
        join(" AND ")
    } else if (0..=all).all(|x| f[x] != (x == all)) {
        format!("NOT ({})", join(" AND "))
    } else if or {
        join(" OR ")
    } else if (0..=all).all(|x| f[x] != (x != 0)) {
        format!("NOT ({})", join(" OR "))
    } else if xor {
        join(" XOR ")
    } else if (0..=all).all(|x| f[x] != (x.count_ones() % 2 == 1)) {
        format!("NOT ({})", join(" XOR "))
    } else {
        return Some(format!("a function of {}", vars.join(", ")));
    };
    Some(name)
}

fn flip_flop(ui: &mut egui::Ui, f: &FlipFlop, lit: &[u8]) {
    egui::CollapsingHeader::new("Now")
        .default_open(true)
        .show(ui, |ui| {
            let d = bit(lit, f.data);
            let q = bit(lit, f.q);
            // An LSR or clock wire the design leaves alone reads 0.
            let lsr = match f.lsr {
                Some(_) => bit(lit, f.lsr),
                None => Some(false),
            }
            .map(|v| v ^ f.invert_lsr);
            let ce = input(lit, f.enable).map(|v| v ^ f.invert_enable);
            let set = match f.preload {
                Some(m) => bit(lit, m),
                None => Some(f.set),
            };
            egui::Grid::new("ff").show(ui, |ui| {
                let mut row = |name: &str, v: Option<bool>, note: String| {
                    ui.label(RichText::new(name).monospace());
                    chip(ui, v.map(u8::from).or(Some(2)));
                    ui.label(RichText::new(note).small());
                    ui.end_row();
                };
                row(
                    "D",
                    d,
                    if f.from_m {
                        "from M (SD = 0)".into()
                    } else {
                        "from DI (SD = 1)".into()
                    },
                );
                row(
                    "CLK",
                    bit(lit, f.clock),
                    if f.invert_clock {
                        "falling edges (INV)".into()
                    } else {
                        "rising edges".into()
                    },
                );
                row(
                    "CE",
                    ce,
                    match f.enable {
                        Input::Tied(true) if !f.invert_enable => {
                            "always enabled (CEMUX = 1)".into()
                        }
                        Input::Tied(_) => "tied".into(),
                        Input::Wire(_) => "as its wire says".into(),
                    },
                );
                row(
                    "LSR",
                    lsr,
                    format!(
                        "{}{}",
                        if f.lsr.is_none() {
                            "not connected: 0; "
                        } else {
                            ""
                        },
                        if f.asynchronous {
                            "acts at once (ASYNC)"
                        } else {
                            "acts at the edge"
                        }
                    ),
                );
                row("Q", q, "its value now".into());
            });
            let next = match (lsr, ce) {
                (Some(true), _) if f.asynchronous => {
                    format!("LSR is 1: Q is held at {}.", b(set))
                }
                (Some(true), _) => format!("At the next edge Q becomes {} (LSR is 1).", b(set)),
                (Some(false), Some(false)) => {
                    format!("At the next edge Q keeps {} (CE is 0).", b(q))
                }
                (Some(false), Some(true)) => format!("At the next edge Q takes D: {}.", b(d)),
                _ => "What the next edge gives is not known.".to_string(),
            };
            ui.label(RichText::new(next).strong());
        });
}

fn mux(ui: &mut egui::Ui, m: &Mux, lit: &[u8]) {
    egui::CollapsingHeader::new("Now")
        .default_open(true)
        .show(ui, |ui| {
            for (name, [sel, low, high, out], (l, h)) in
                [("F5", m.f5, ("F0", "F1")), ("FX", m.fx, ("FXA", "FXB"))]
            {
                let s = bit(lit, sel);
                let passes = match s {
                    Some(true) => h,
                    Some(false) => l,
                    None => "?",
                };
                ui.label(format!(
                    "{name}: select {} → passes {passes} ({}); {l} = {}, {h} = {}; {name} = {}",
                    b(s),
                    if sel.is_none() {
                        "select not used"
                    } else {
                        "from M"
                    },
                    b(bit(lit, low)),
                    b(bit(lit, high)),
                    b(bit(lit, out)),
                ));
            }
        });
}

fn cib(ui: &mut egui::Ui, s: &Static, rows: &[JWire], lit: &[u8]) -> Option<u32> {
    let mut picked = None;
    egui::CollapsingHeader::new(format!("J wires in use ({})", rows.len()))
        .default_open(true)
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.label("The design uses none of this CIB's J wires.");
                return;
            }
            egui::Grid::new("cib")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for h in ["J wire", "", "from", "to", "net"] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for j in rows {
                        ui.label(RichText::new(&j.name).monospace())
                            .on_hover_text(if j.input {
                                "into the block beside the CIB"
                            } else {
                                "out of the block, onto the tracks"
                            });
                        chip(ui, read(lit, j.wire));
                        ui.label(RichText::new(&j.from).monospace().small());
                        ui.label(RichText::new(&j.to).monospace().small());
                        if let Some(n) = net_link(ui, s, j.wire) {
                            picked = Some(n);
                        }
                        ui.end_row();
                    }
                });
        });
    picked
}

// ---------------------------------------------------------------------------
// Working out what an element is.

const LETTERS: [char; 4] = ['A', 'B', 'C', 'D'];

/// The wire a name of a position's tile types is.
fn resolve(s: &Static, (r, c): (u16, u16), name: &str) -> Option<Wire> {
    match s.model.wire(
        Location {
            row: r as i16,
            col: c as i16,
        },
        name,
    ) {
        Ok(Endpoint::Wire(w)) => Some(w),
        _ => None,
    }
}

fn wire_of(s: &Static, at: (u16, u16), name: &str) -> Option<u32> {
    let w = resolve(s, at, name)?;
    s.overlay.value_of(&s.model, &w)
}

/// What the plan sets at a position, by name: its value, or the values it
/// can be.
fn settings_at(s: &Static, at: (u16, u16)) -> HashMap<&str, String> {
    s.settings
        .get(&at)
        .into_iter()
        .flatten()
        .map(|(n, v)| (n.as_str(), v.join(" | ")))
        .collect()
}

fn details(s: &Static, p: &Picked) -> Details {
    let (r, c) = p.at;
    let mut d = match p.kind {
        BoxKind::Lut | BoxKind::FlipFlop | BoxKind::Mux | BoxKind::Control | BoxKind::Slice
            if !p.dsp =>
        {
            logic(s, p)
        }
        _ if p.label == "CIB" => cib_details(s, p),
        _ => site(s, p),
    };
    d.title = format!("{}  ·  R{r}C{c}", d.title);
    d
}

/// Which slice a part of a logic tile is in, by where it is.
fn slice_of(s: &Static, p: &Picked) -> usize {
    let base = [p.at.1 as f32 * TILE_W, s.fabric.row_top[p.at.0 as usize]];
    let at = [p.min[0] - base[0], p.min[1] - base[1]];
    let near = |q: [f32; 2]| (q[0] - at[0]).abs() < 1e-4 && (q[1] - at[1]).abs() < 1e-4;
    (0..4)
        .find(|&k| {
            near(fabric::mux_rect(k).0)
                || near(fabric::ctrl_rect(k).0)
                || near(fabric::slice_rect(k).0)
        })
        .unwrap_or(0)
}

/// A part of a slice: a LUT, a flip-flop, the wide muxes, the control set
/// or the whole slice.
fn logic(s: &Static, p: &Picked) -> Details {
    let at = p.at;
    let number = |prefix: &str| {
        p.label
            .strip_prefix(prefix)
            .and_then(|n| n.parse::<usize>().ok())
    };
    let (slice, n) = match p.kind {
        BoxKind::Lut => {
            let n = number("LUT").unwrap_or(0);
            (n / 2, Some(n))
        }
        BoxKind::FlipFlop => {
            let n = number("FF").unwrap_or(0);
            (n / 2, Some(n))
        }
        _ => (slice_of(s, p), None),
    };
    let letter = LETTERS[slice];
    let set = settings_at(s, at);
    let get = |name: String| set.get(name.as_str()).cloned();
    let mode = get(format!("SLICE{letter}.MODE")).unwrap_or_else(|| "LOGIC".into());
    let pin = |name: String, role: &str| Pin {
        wire: wire_of(s, at, &name),
        name,
        role: role.into(),
    };
    // The tile's clock and set/reset the slice takes (arcs into MUXCLK and
    // MUXLSR), and how the slice's flip-flops take them.
    let chosen = |sink: String| {
        s.transfers
            .get(&at)
            .and_then(|t| t.iter().find(|(_, z)| *z == sink))
            .map(|(a, _)| a.clone())
    };
    let clk = chosen(format!("MUXCLK{slice}"));
    let lsr = chosen(format!("MUXLSR{slice}"));
    let mut shared: Vec<(String, String)> = Vec::new();
    if let Some(k) = &clk {
        shared.push((format!("MUXCLK{slice} ←"), k.clone()));
        if let Some(v) = get(format!("{k}.CLKMUX")) {
            shared.push((format!("{k}.CLKMUX"), v));
        }
    }
    if let Some(l) = &lsr {
        shared.push((format!("MUXLSR{slice} ←"), l.clone()));
        for f in ["LSRMUX", "SRMODE"] {
            if let Some(v) = get(format!("{l}.{f}")) {
                shared.push((format!("{l}.{f}"), v));
            }
        }
    }
    // The slice's settings whose name (after `SLICEx.`) passes `keep`.
    let of_slice = |keep: &dyn Fn(&str) -> bool| -> Vec<(String, String)> {
        let prefix = format!("SLICE{letter}.");
        let mut out: Vec<(String, String)> = set
            .iter()
            .filter(|(k, _)| k.strip_prefix(&prefix).is_some_and(keep))
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        out.sort();
        out
    };
    let used = s
        .geometry
        .logic
        .iter()
        .find(|t| (t.row, t.col) == at)
        .map(|t| (t.luts, t.flip_flops));
    let in_use = |mask: u8, n: usize| used.is_some_and(|u| [u.0, u.1][mask as usize] >> n & 1 == 1);
    let mut notes = Vec::new();
    match (p.kind, n) {
        (BoxKind::Lut, Some(n)) if mode == "RAMW" => {
            let k = n % 2;
            let tied = |port: char| -> Input {
                match get(format!("SLICE{letter}.{port}{k}MUX")).as_deref() {
                    Some("0") => Input::Tied(false),
                    Some("1") => Input::Tied(true),
                    _ => Input::Wire(wire_of(s, at, &format!("{port}{n}_SLICE"))),
                }
            };
            let outputs: Vec<(String, Option<u32>, char, Input)> = fabric_overlay::ramw_outputs(k)
                .into_iter()
                .map(|(name, from)| (name.clone(), wire_of(s, at, &name), from, tied(from)))
                .collect();
            let mut pins: Vec<Pin> = LETTERS
                .iter()
                .map(|q| pin(format!("{q}{n}_SLICE"), &format!("input {q}")))
                .collect();
            for (i, (name, _, from, _)) in outputs.iter().enumerate() {
                pins.push(pin(
                    name.clone(),
                    &format!(
                        "{} bit {i}, from input {from}",
                        if k == 0 {
                            "write address"
                        } else {
                            "write data"
                        }
                    ),
                ));
            }
            Details {
                title: format!("LUT{n}"),
                kind: format!(
                    "the {} half of slice C as RAMW: the {} of the RAM in slices A and B",
                    if k == 0 { "first" } else { "second" },
                    if k == 0 {
                        "write address"
                    } else {
                        "write data"
                    }
                ),
                about: vec![RAMW.to_string()],
                pins,
                settings: of_slice(&|r: &str| r == "MODE" || r.starts_with(&format!("K{k}."))),
                inside: Inside::Ramw(outputs),
                notes,
            }
        }
        (BoxKind::Lut, Some(n)) => {
            let k = n % 2;
            let tied = |port: char| -> Input {
                match get(format!("SLICE{letter}.{port}{k}MUX")).as_deref() {
                    Some("0") => Input::Tied(false),
                    Some("1") => Input::Tied(true),
                    _ => Input::Wire(wire_of(s, at, &format!("{port}{n}_SLICE"))),
                }
            };
            let init = s
                .model
                .tile_id(&format!("R{}C{}:PLC2", at.0, at.1))
                .and_then(|t| s.device.word(t, &format!("SLICE{letter}.K{k}.INIT")).ok())
                .and_then(|w| {
                    w.iter().enumerate().try_fold(0u16, |m, (i, v)| match v {
                        Logic::One => Some(m | 1 << i),
                        Logic::Zero => Some(m),
                        _ => None,
                    })
                });
            let carry = (mode == "CCU2").then(|| {
                let name = match (slice, k) {
                    (0, 0) => "FCI_SLICE".to_string(),
                    (_, 0) => format!("FCI{letter}_SLICE"),
                    _ => format!("FCI{letter}1_SLICE"),
                };
                let inject =
                    get(format!("SLICE{letter}.CCU2.INJECT1_{k}")).as_deref() == Some("YES");
                (wire_of(s, at, &name), inject)
            });
            // The carry out of the half: into the second half from the
            // first, out of the slice from the second.
            let carry_out = match (slice, k) {
                (_, 0) => format!("FCI{letter}1_SLICE"),
                (3, _) => "FCO_SLICE".to_string(),
                _ => format!("FCO{letter}_SLICE"),
            };
            let mut pins: Vec<Pin> = LETTERS
                .iter()
                .map(|q| {
                    let role = match tied(*q) {
                        Input::Tied(v) => {
                            format!("input {q}, tied to {} by {q}{k}MUX", u8::from(v))
                        }
                        Input::Wire(_) => format!("input {q}"),
                    };
                    pin(format!("{q}{n}_SLICE"), &role)
                })
                .collect();
            // A route from F out of the slice (the tile's `F{n}` may carry a
            // wide mux's output instead).
            let taken = s
                .transfers
                .get(&at)
                .is_some_and(|t| t.iter().any(|(a, _)| *a == format!("F{n}_SLICE")));
            pins.push(pin(
                format!("F{n}_SLICE"),
                if taken {
                    "output F"
                } else {
                    "output F, which no route takes"
                },
            ));
            if mode == "CCU2" {
                if let Some(name) = match (slice, k) {
                    (0, 0) => Some("FCI_SLICE".to_string()),
                    (_, 0) => Some(format!("FCI{letter}_SLICE")),
                    _ => None,
                } {
                    pins.push(pin(name, "carry in, from the slice before"));
                }
                pins.push(pin(
                    carry_out.clone(),
                    if k == 0 {
                        "carry out, into the second half"
                    } else {
                        "carry out, to the next slice"
                    },
                ));
            }
            if !in_use(0, n) {
                notes.push(format!("The design does not use LUT{n}."));
            }
            let mut about = vec![LUT.to_string()];
            if mode == "CCU2" {
                about.push(CCU2.to_string());
            }
            if mode == "DPRAM" {
                about.push(DPRAM.to_string());
            }
            let mut settings = of_slice(&|r: &str| {
                r == "MODE"
                    || r.starts_with(&format!("K{k}."))
                    || r == format!("CCU2.INJECT1_{k}")
                    || LETTERS.iter().any(|q| r == format!("{q}{k}MUX"))
            });
            if let Some(i) = init {
                settings.insert(
                    0,
                    (format!("SLICE{letter}.K{k}.INIT"), format!("0x{i:04X}")),
                );
            }
            Details {
                title: format!("LUT{n}"),
                kind: format!(
                    "a 4-input look-up table, the {} of slice {letter}; mode {mode}",
                    if k == 0 { "first" } else { "second" }
                ),
                about,
                pins,
                settings,
                inside: Inside::Lut(Lut {
                    init,
                    inputs: LETTERS.map(tied),
                    out: wire_of(s, at, &format!("F{n}_SLICE")),
                    carry_out: carry.and_then(|_| wire_of(s, at, &carry_out)),
                    ram: mode == "DPRAM",
                    taken,
                    carry,
                }),
                notes,
            }
        }
        (BoxKind::FlipFlop, Some(n)) => {
            let k = n % 2;
            let from_m = get(format!("SLICE{letter}.REG{k}.SD")).as_deref() == Some("0");
            let enable = match get(format!("SLICE{letter}.CEMUX")).as_deref() {
                Some("0") => Input::Tied(false),
                Some("1") => Input::Tied(true),
                _ => Input::Wire(wire_of(s, at, &format!("CE{slice}_SLICE"))),
            };
            let m = wire_of(s, at, &format!("M{n}_SLICE"));
            let prld = get(format!("SLICE{letter}.REG{k}.LSRMODE")).as_deref() == Some("PRLD");
            let clock_mux = clk.as_ref().and_then(|c| get(format!("{c}.CLKMUX")));
            let lsr_mux = lsr.as_ref().and_then(|l| get(format!("{l}.LSRMUX")));
            let sr_mode = lsr.as_ref().and_then(|l| get(format!("{l}.SRMODE")));
            let data_pin = if from_m {
                pin(format!("M{n}_SLICE"), "D, from M (REG.SD = 0)")
            } else {
                pin(format!("DI{n}_SLICE"), "D, from DI (REG.SD = 1)")
            };
            let pins = vec![
                data_pin,
                pin(format!("CLK{slice}_SLICE"), "clock, the slice's"),
                pin(format!("LSR{slice}_SLICE"), "set/reset, the slice's"),
                pin(format!("CE{slice}_SLICE"), "clock enable, the slice's"),
                pin(format!("Q{n}_SLICE"), "output Q"),
            ];
            if !in_use(1, n) {
                notes.push(format!("The design does not use FF{n}."));
            }
            let mut settings = of_slice(&|r: &str| {
                r.starts_with(&format!("REG{k}."))
                    || ["CEMUX", "GSR"].contains(&r)
                    || r == format!("M{k}MUX")
            });
            settings.extend(shared);
            Details {
                title: format!("FF{n}"),
                kind: format!(
                    "a D flip-flop, the {} of slice {letter}",
                    if k == 0 { "first" } else { "second" }
                ),
                about: vec![FF.to_string()],
                settings,
                inside: Inside::FlipFlop(FlipFlop {
                    data: if from_m {
                        m
                    } else {
                        wire_of(s, at, &format!("DI{n}_SLICE"))
                    },
                    from_m,
                    di: wire_of(s, at, &format!("DI{n}_SLICE")),
                    m,
                    clock: wire_of(s, at, &format!("CLK{slice}_SLICE")),
                    invert_clock: clock_mux.as_deref() == Some("INV"),
                    lsr: wire_of(s, at, &format!("LSR{slice}_SLICE")),
                    invert_lsr: lsr_mux.as_deref() == Some("INV"),
                    asynchronous: sr_mode.as_deref() == Some("ASYNC"),
                    invert_enable: get(format!("SLICE{letter}.CEMUX")).as_deref() == Some("INV"),
                    enable,
                    set: get(format!("SLICE{letter}.REG{k}.REGSET")).as_deref() != Some("RESET"),
                    preload: prld.then_some(m),
                    q: wire_of(s, at, &format!("Q{n}_SLICE")),
                }),
                pins,
                notes,
            }
        }
        (BoxKind::Mux, _) => {
            let (n0, n1) = (2 * slice, 2 * slice + 1);
            let w = |name: String| wire_of(s, at, &name);
            let pins = vec![
                pin(format!("M{n0}_SLICE"), "selects F5"),
                pin(format!("F5{letter}_SLICE"), "F5 = M ? F1 : F0"),
                pin(format!("M{n1}_SLICE"), "selects FX"),
                pin(format!("FXA{letter}_SLICE"), "FX's low input"),
                pin(format!("FXB{letter}_SLICE"), "FX's high input"),
                pin(format!("FX{letter}_SLICE"), "FX = M ? FXB : FXA"),
            ];
            Details {
                title: format!("F5 and FX of slice {letter}"),
                kind: format!("the wide muxes of slice {letter}; mode {mode}"),
                about: vec![MUX.to_string()],
                pins,
                settings: of_slice(&|r: &str| r == "MODE" || r == "M0MUX" || r == "M1MUX"),
                inside: Inside::Mux(Mux {
                    slice,
                    f5: [
                        w(format!("M{n0}_SLICE")),
                        w(format!("F{n0}_SLICE")),
                        w(format!("F{n1}_SLICE")),
                        w(format!("F5{letter}_SLICE")),
                    ],
                    fx: [
                        w(format!("M{n1}_SLICE")),
                        w(format!("FXA{letter}_SLICE")),
                        w(format!("FXB{letter}_SLICE")),
                        w(format!("FX{letter}_SLICE")),
                    ],
                }),
                notes,
            }
        }
        (BoxKind::Control, _) => {
            // Longer prefixes first: WADO before WAD.
            let roles = |name: &str| -> &'static str {
                [
                    ("CLK", "clock"),
                    ("LSR", "set/reset"),
                    ("CE", "clock enable"),
                    ("WRE", "RAM write enable"),
                    ("WCK", "RAM write clock"),
                    ("WADO", "RAM write address, to slices A and B"),
                    ("WDO", "RAM write data, to slices A and B"),
                    ("WAD", "RAM write address"),
                    ("WD", "RAM write data"),
                ]
                .iter()
                .find(|(p, _)| name.starts_with(p))
                .map_or("", |r| r.1)
            };
            let pins = p.pins.iter().map(|n| pin(n.clone(), roles(n))).collect();
            let mut settings = of_slice(&|r: &str| ["CEMUX", "GSR", "WREMUX"].contains(&r));
            settings.extend(shared);
            let choice = |c: &Option<String>, first: &str, second: &str| match c.as_deref() {
                Some(x) if x == first => Some(0),
                Some(x) if x == second => Some(1),
                _ => None,
            };
            let control = Control {
                clk: [wire_of(s, at, "CLK0"), wire_of(s, at, "CLK1")],
                clk_choice: choice(&clk, "CLK0", "CLK1"),
                clk_out: wire_of(s, at, &format!("CLK{slice}_SLICE")),
                clk_inv: clk
                    .as_ref()
                    .and_then(|c| get(format!("{c}.CLKMUX")))
                    .as_deref()
                    == Some("INV"),
                lsr: [wire_of(s, at, "LSR0"), wire_of(s, at, "LSR1")],
                lsr_choice: choice(&lsr, "LSR0", "LSR1"),
                lsr_out: wire_of(s, at, &format!("LSR{slice}_SLICE")),
                lsr_inv: lsr
                    .as_ref()
                    .and_then(|l| get(format!("{l}.LSRMUX")))
                    .as_deref()
                    == Some("INV"),
                lsr_async: lsr
                    .as_ref()
                    .and_then(|l| get(format!("{l}.SRMODE")))
                    .as_deref()
                    == Some("ASYNC"),
                ce: wire_of(s, at, &format!("CE{slice}_SLICE")),
                ce_mode: get(format!("SLICE{letter}.CEMUX")).unwrap_or_else(|| "CE".into()),
            };
            Details {
                title: format!("Control set of slice {letter}"),
                kind: "clock, set/reset, clock enable and the RAM write port of a slice".into(),
                about: vec![CONTROL.to_string()],
                pins,
                settings,
                inside: Inside::Control(control),
                notes,
            }
        }
        _ => {
            let parts: Vec<String> = (0..2)
                .map(|k| {
                    let n = 2 * slice + k;
                    format!(
                        "LUT{n} {}, FF{n} {}",
                        if in_use(0, n) { "in use" } else { "not used" },
                        if in_use(1, n) { "in use" } else { "not used" }
                    )
                })
                .collect();
            let mut settings = of_slice(&|_| true);
            settings.extend(shared);
            notes.push(parts.join("; "));
            let w = |name: String| wire_of(s, at, &name);
            let (n0, n1) = (2 * slice, 2 * slice + 1);
            let view = SliceView {
                slice,
                mode: mode.clone(),
                luts: [in_use(0, n0), in_use(0, n1)],
                ffs: [in_use(1, n0), in_use(1, n1)],
                f: [w(format!("F{n0}_SLICE")), w(format!("F{n1}_SLICE"))],
                q: [w(format!("Q{n0}_SLICE")), w(format!("Q{n1}_SLICE"))],
                di: [w(format!("DI{n0}_SLICE")), w(format!("DI{n1}_SLICE"))],
                m: [w(format!("M{n0}_SLICE")), w(format!("M{n1}_SLICE"))],
                f5: w(format!("F5{letter}_SLICE")),
                fx: w(format!("FX{letter}_SLICE")),
                fxa: w(format!("FXA{letter}_SLICE")),
                fxb: w(format!("FXB{letter}_SLICE")),
                fci: w(if slice == 0 {
                    "FCI_SLICE".into()
                } else {
                    format!("FCI{letter}_SLICE")
                }),
                fco: w(if slice == 3 {
                    "FCO_SLICE".into()
                } else {
                    format!("FCO{letter}_SLICE")
                }),
                clk: w(format!("CLK{slice}_SLICE")),
                lsr: w(format!("LSR{slice}_SLICE")),
                ce: w(format!("CE{slice}_SLICE")),
            };
            Details {
                title: format!("Slice {letter}"),
                kind: format!("a slice of a logic tile (PLC); mode {mode}"),
                about: vec![SLICE.to_string()],
                pins: Vec::new(),
                settings,
                inside: Inside::Slice(view),
                notes,
            }
        }
    }
}

/// A routing transfer at one end of a wire: the name at its other end,
/// the position, and whether the wire is its source.
type End = (String, (u16, u16), bool);

/// A CIB: each J wire the design uses, with what drives it and what it
/// drives within a few tiles.
fn cib_details(s: &Static, p: &Picked) -> Details {
    let at = p.at;
    let set = settings_at(s, at);
    // The transfers near the CIB by the wire at each end: (the other end's
    // name, its position, whether the wire is the source).
    let mut ends: HashMap<Wire, Vec<End>> = HashMap::new();
    for dr in -3i32..=3 {
        for dc in -3i32..=3 {
            let (r, c) = (at.0 as i32 + dr, at.1 as i32 + dc);
            if r < 0 || c < 0 {
                continue;
            }
            let q = (r as u16, c as u16);
            for (a, z) in s.transfers.get(&q).into_iter().flatten() {
                if let Some(w) = resolve(s, q, a) {
                    ends.entry(w).or_default().push((z.clone(), q, true));
                }
                if let Some(w) = resolve(s, q, z) {
                    ends.entry(w).or_default().push((a.clone(), q, false));
                }
            }
        }
    }
    let place = |name: &str, q: (u16, u16)| {
        if q == at {
            name.to_string()
        } else {
            format!("{name} (R{}C{})", q.0, q.1)
        }
    };
    // The CIB's tile, for the size of each J wire's input mux.
    let tile = s
        .model
        .tiles()
        .find(|(_, t)| t.row as u16 == at.0 && t.col as u16 == at.1 && t.kind.starts_with("CIB"))
        .map(|(id, _)| id);
    let mut rows = Vec::new();
    let mut seen = 0;
    for pin in &p.pins {
        let j = pin.trim_end_matches("_CIBTEST");
        seen += 1;
        let input = !(j.starts_with("JF") || j.starts_with("JQ"));
        let w = resolve(s, at, j);
        let list = w.as_ref().and_then(|w| ends.get(w));
        let side = |source: bool| -> Vec<String> {
            list.into_iter()
                .flatten()
                .filter(|e| e.2 == source)
                .map(|e| place(&e.0, e.1))
                .collect()
        };
        let mut from = side(false);
        let mut tied = None;
        if let Some(v) = set.get(format!("CIB.{j}MUX").as_str()) {
            if v == "0" || v == "1" {
                from.insert(0, format!("tied to {v} (CIB.{j}MUX)"));
                tied = Some(v == "1");
            }
        }
        let to = side(true);
        if from.is_empty() && to.is_empty() {
            continue;
        }
        rows.push(JWire {
            name: j.to_string(),
            input,
            wire: w.and_then(|w| s.overlay.value_of(&s.model, &w)),
            from: if from.is_empty() {
                "the block".into()
            } else {
                from.join(", ")
            },
            to: if to.is_empty() {
                "the block".into()
            } else {
                to.join(", ")
            },
            tied,
            choices: tile.map_or(0, |t| s.model.connections_to(t, j).count()),
        });
    }
    let mut settings: Vec<(String, String)> = set
        .iter()
        .filter(|(k, _)| k.starts_with("CIB."))
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    settings.sort();
    let kinds = s.fabric.types_at(at.0, at.1).join(", ");
    Details {
        title: "CIB".into(),
        kind: format!("connection interface block ({kinds}): {} J wires, {} in use", seen, rows.len()),
        about: vec![CIB.to_string()],
        pins: Vec::new(),
        settings,
        inside: Inside::Cib(rows),
        notes: vec![
            "\"the block\": the site beside the CIB, through a fixed connection the plan does not route.".into(),
        ],
    }
}

/// Any other site: a pad, its IOLOGIC, a block RAM, a part of a DSP block,
/// a clock site. Its ports and decoded settings come from the device
/// model, found by its name near where it is drawn.
fn site(s: &Static, p: &Picked) -> Details {
    let at = p.at;
    let distance = |r: &ecp5_emu::model::Resource| {
        (r.location.row as i32 - at.0 as i32).abs() + (r.location.col as i32 - at.1 as i32).abs()
    };
    // The site whose ports are the wires of the box's pins (sites of one
    // name are told apart so); a part of a DSP block, drawn without pins,
    // by its name.
    let wires: std::collections::HashSet<Wire> =
        p.pins.iter().filter_map(|n| resolve(s, at, n)).collect();
    let by_pins = s
        .model
        .resources()
        .filter(|(_, r)| r.family != "PLC2" && r.family != "SLICE" && distance(r) <= 8)
        .map(|(id, r)| {
            let shared = s
                .model
                .resource_ports(id)
                .iter()
                .filter(|q| wires.contains(&q.wire))
                .count();
            (shared, id, r)
        })
        .filter(|(n, ..)| *n > 0)
        .max_by_key(|(n, _, r)| (*n, -distance(r)))
        .map(|(_, id, r)| (distance(r), id, r));
    let name = p.label.split(' ').next().unwrap_or(&p.label);
    let found = by_pins.or_else(|| {
        s.model
            .resources()
            .filter(|(_, r)| r.name == name)
            .map(|(id, r)| (distance(r), id, r))
            .filter(|(d, ..)| *d <= 4)
            .min_by_key(|(d, ..)| *d)
    });
    let Some((_, id, r)) = found else {
        // A part with no site of its own: its pins as drawn.
        let pins = p
            .pins
            .iter()
            .map(|n| Pin {
                wire: wire_of(s, at, n),
                name: n.clone(),
                role: String::new(),
            })
            .collect();
        return Details {
            title: p.label.clone(),
            kind: "a part of a site, named by its ports".into(),
            about: vec![about(&p.label).to_string()],
            pins,
            settings: Vec::new(),
            inside: Inside::Nothing,
            notes: Vec::new(),
        };
    };
    let pins: Vec<Pin> = s
        .model
        .resource_ports(id)
        .into_iter()
        .map(|port| Pin {
            role: match port.direction {
                Direction::Input => "in",
                Direction::Output => "out",
                Direction::InOut => "in/out",
            }
            .into(),
            wire: s.overlay.value_of(&s.model, &port.wire),
            name: port.name,
        })
        .collect();
    let decoded = s.device.resource_settings(id);
    let settings = decoded
        .map(|d| {
            d.fields
                .iter()
                .filter_map(|f| f.value.as_ref().map(|v| (f.name.clone(), v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let mut notes = Vec::new();
    let mut kind = format!("{}, {}", r.family, r.key);
    let mut inside = Inside::Nothing;
    if r.family == "PIO" {
        if let Ok(modes) = s.device.pio_modes(id) {
            if let Some(m) = modes.get(&r.key) {
                kind.push_str(&format!(
                    "; {}{}",
                    m.direction.as_deref().unwrap_or("direction not known"),
                    if m.base_types.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", m.base_types.join(" | "))
                    }
                ));
            }
        }
        let ball = s
            .model
            .package_pins()
            .find(|q| q.package == board::PACKAGE && q.pio == Some(id));
        let port = |name: &str| pins.iter().find(|q| q.name == name).and_then(|q| q.wire);
        inside = Inside::Pio(Pio {
            i: port("I"),
            t: port("T"),
            o: port("O"),
            ball: ball.map(|b| b.ball.clone()),
            role: ball.and_then(|b| board_role(&b.ball)),
        });
        if let Some(pin) = ball {
            notes.push(format!(
                "Ball {} of the {}, bank {}{}",
                pin.ball,
                board::PACKAGE,
                pin.bank,
                board_role(&pin.ball)
                    .map(|r| format!(": on the ULX3S, {r}"))
                    .unwrap_or_default()
            ));
        }
    }
    // A site reached at its inputs alone passes nothing on: a fixed
    // connection brings a value to it, but nothing takes what it gives.
    let reached: Vec<&str> = pins
        .iter()
        .filter(|q| q.role == "in" && q.wire.is_some())
        .map(|q| q.name.as_str())
        .collect();
    let gives = pins.iter().any(|q| q.role != "in" && q.wire.is_some());
    if !reached.is_empty() && !gives {
        notes.push(format!(
            "Only its input{} {} {} reached; none of its outputs is used, so it passes nothing on.",
            if reached.len() > 1 { "s" } else { "" },
            reached.join(", "),
            if reached.len() > 1 { "are" } else { "is" }
        ));
    }
    if let Some(e) = decoded.and_then(|d| d.enabled) {
        notes.push(if e {
            "The bitstream enables it.".into()
        } else {
            "The bitstream leaves it disabled.".into()
        });
    }
    Details {
        // `DLLDEL R48C0` where sites of one name are drawn together.
        title: if p.label.split(' ').next() == Some(r.name.as_str()) {
            p.label.clone()
        } else {
            r.name.clone()
        },
        kind,
        about: vec![about(&r.family).to_string()],
        pins,
        settings,
        inside,
        notes,
    }
}

/// What a ball of the package is on the ULX3S.
fn board_role(ball: &str) -> Option<String> {
    if let Some(i) = board::LEDS.iter().position(|b| *b == ball) {
        return Some(format!("LED D{i} (led[{i}])"));
    }
    if let Some(b) = board::BUTTONS.iter().find(|b| b.ball == ball) {
        return Some(format!("the {} button", b.name));
    }
    match ball {
        _ if ball == board::CLOCK => Some("the 25 MHz oscillator (clk_25mhz)".into()),
        _ if ball == board::UART_TX => Some("the UART line to the host (ftdi_rxd)".into()),
        _ if ball == board::UART_RX => Some("the UART line from the host (ftdi_txd)".into()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// How each kind of element works.

const LUT: &str = "A LUT4 is a 16-bit memory that its four inputs read: D C B A make an address from 0 to 15, and F is the bit of INIT stored there. Any function of four inputs is one INIT value, which the bitstream writes. An input the design does not use is tied to a constant by the slice's input mux (A0MUX = 1 …), and then only the rows with that value are ever read.";

const DPRAM: &str = "In DPRAM mode the LUTs of slices A and B are 16 × 1 bits of RAM each: D C B A are the read address, and F is the bit stored there. Slice C, as RAMW, gives all four the write address and their data, and the write enable and clock (WRE, WCK) write them. INIT is their content at configuration.";

const RAMW: &str = "In RAMW mode slice C is not logic: it is the write port of the distributed RAM in slices A and B of its tile. Its first LUT's inputs D, B, C, A go on as the write address WAD0–3 (WADO0–3) to all four RAM LUTs; its second LUT's inputs C, A, D, B go on as the data WDO0–3, two bits for each of slices A and B. Its INIT is not read.";

const CCU2: &str = "In CCU2 mode the slice's two LUTs are a 2-bit adder on the carry chain. Each half reads its INIT as a LUT4 does, and F is that bit xor the carry in. Where the bit is 1 (propagate) the carry passes on; where it is 0 the carry out is the bit of INIT's low four that B A read (generate). With INJECT1 = YES the carry in does not reach F, and the half generates no carry of its own.";

const FF: &str = "A D flip-flop. At a rising edge of its clock (a falling one with CLKMUX = INV) Q takes D if CE is 1, and keeps its value if CE is 0. While LSR is 1 the edge gives Q REGSET's value whatever CE is (SRMODE LSR_OVER_CE), or LSR gives it at once (ASYNC); with LSRMODE = PRLD it loads M instead. REG.SD picks D: 1 takes DI, routed in the tile from this LUT's F or the slice's wide muxes; 0 takes the M input. After configuration Q starts at REGSET's value.";

const MUX: &str = "The two wide muxes of the slice. F5 = M0 ? F1 : F0: with M0 as a fifth input, the slice's two LUT4s make any function of five inputs (a LUT5). FX = M1 ? FXB : FXA joins the F5 and FX outputs of neighbouring slices, for functions of six, seven and eight inputs. The M inputs also reach the flip-flops (REG.SD = 0). In CCU2 mode the muxes are not used.";

const CONTROL: &str = "The control set of the slice, which its two flip-flops share. The clock is one of the tile's two clocks, CLK0 and CLK1, which an arc into MUXCLK picks; CLKMUX = INV takes its falling edge. LSR, the set/reset, is one of LSR0 and LSR1 in the same way. CE enables the flip-flops: CEMUX 1 always, 0 never, CE as its wire says, INV its inverse. WRE, WCK, WAD and WD are the write port of distributed RAM: slices A and B hold the RAM, and slice C passes them the write address and data (WADO, WDO).";

const SLICE: &str = "A slice: two LUT4s, two flip-flops, the wide muxes F5 and FX, a stage of the carry chain and a control set. MODE says what its LUTs are: LOGIC (two LUT4s), CCU2 (a 2-bit adder on the carry chain) or a distributed RAM. A logic tile (PLC2) has four slices. Click a part of the slice for more.";

const CIB: &str = "A CIB (connection interface block) is the routing of a logic tile without its slices. One stands beside every block that is not logic: the I/O, block RAM, DSP and the clocks. Its switch box is a logic tile's, with the same tracks and arcs, and in place of the slices' pins it has J wires. Each input mux (JA–JD, JM, JCE, JCLK, JLSR) passes one track to the block beside it, or ties its J wire to 0 or 1 (CIB.JxMUX); the block's outputs come back on JF and JQ, and the switch box takes them onto the tracks. The database draws this interface as a site, CIBTEST, whose pins are the J wires.";

fn about(family: &str) -> &'static str {
    match family {
        "PIO" | "PIOA" | "PIOB" | "PIOC" | "PIOD" => "One pad's I/O buffer. I (the wire PADDO) is the value the pad drives, and T (PADDT) turns the driver off: at 1 the pad drives nothing and only listens. O (PADDI) is what the pad reads, back into the fabric. DRIVE (mA), SLEWRATE, PULLMODE and HYSTERESIS are its electrical settings. When the IOLOGIC beside it is used (registers, DDR), it drives the pad through IOLDO and IOLTO instead.",
        "IOLOGIC" => "The logic between a pad's buffer and the fabric: registers on the input, the output and the output enable, DDR (two bits per clock, IDDRX1F and ODDRX1F), gearing, and a delay on the input. MODE says what it is set to; with NONE the pad's signals pass it by.",
        "SIOLOGIC" => "A simpler IOLOGIC, on the top and bottom edges: registers and DDR between a pad's buffer and the fabric, without the gearing. MODE says what it is set to; with NONE the pad's signals pass it by.",
        "EBR" => "A block of 18 Kbit RAM. As DP16KD it has two ports, each with its clock, enable, write enable, address and data, from 16K × 1 to 1K × 18; as PDPW16KD one write port and one read port up to 36 bits wide. The bitstream sets its shape and its first content (INITVAL), and the emulator runs it as written.",
        "MULT18" => "An 18 × 18 multiplier of a DSP slice: A × B, signed or unsigned, with optional registers on its inputs, a pipeline register and an output register.",
        "ALU54" => "The 54-bit adder of a DSP slice: it adds, subtracts or accumulates the products of the slice's two MULT18s, and passes a cascade to the next slice.",
        "PRADD" => "The ports of a DSP slice's pre-adder, which the database names apart from the multiplier.",
        "DSP_SLICE" | "DSP_BLOCK" => "The frame of a DSP block: its multipliers and adders are drawn as its parts.",
        "DCC_MID" | "DCC_CENTRE" => "A clock enable on the clock tree (DCC): it passes a global clock on into the tree, or stops it while CE is 0.",
        "DCS" => "A clock switch: it picks one of two clocks for the tree, without glitches when it switches.",
        "PLL" => "A phase-locked loop: it makes new clocks from a reference clock. With its feedback from CLKOP, CLKOP runs at the reference × CLKFB_DIV / CLKI_DIV; the VCO runs CLKOP_DIV times faster, and each other output divides the VCO by its own divider. LOCK goes to 1 when the outputs are stable.",
        "OSCG" => "The on-chip oscillator: about 310 MHz divided by DIV. Its frequency varies from chip to chip and with temperature.",
        "CLKDIVF" => "Divides an edge clock by 2 or 3.5, for the gearing of the IOLOGIC.",
        "ECLKSYNCB" => "Starts and stops an edge clock cleanly, for the gearing of the IOLOGIC.",
        "TRELLIS_ECLKBUF" => "A buffer onto an edge clock.",
        "BRGECLKSYNC" | "ECLKBRIDGECS" => "The bridge that passes an edge clock between banks.",
        "DQSBUFM" => "The strobe logic of a DDR memory interface (DQS): it aligns the clock of the data read.",
        "DLLDELD" => "A delay on a clock input, which a DLL keeps at a fixed share of the clock period.",
        "DDRDLL" => "A DLL that measures a clock's period, for the delays of the DLLDELD and DQS logic.",
        "GSR" => "The global set/reset: it puts every flip-flop whose slice has GSR ENABLED at its REGSET value.",
        "JTAGG" => "The JTAG port's access into the fabric.",
        "SEDGA" => "Soft error detection: it reads the configuration back and checks its CRC.",
        "USRMCLK" => "The fabric's access to the clock pin of the configuration flash.",
        "DTR" => "The die's temperature readout.",
        _ => "A site of the device; the settings below are those the bitstream gives it.",
    }
}

#[cfg(test)]
mod tests {
    //! The inspector over a running design (`target/reference`, built by
    //! examples/ulx3s/build.sh): every LUT, flip-flop and other site the
    //! design uses opens, and what it shows agrees with the emulator. The
    //! bit of INIT that a LUT's inputs address now is its output now (xor
    //! the carry in CCU2 mode); a flip-flop has its clock and output; a CIB
    //! in use has J wires in use; a pad in use has pins with values.
    use super::*;
    use crate::fabric_view::pins_of;
    use ecp5_emu::live::{self, Command, Design, Shared};
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    /// The element the map picks where the overlay draws one in use.
    fn picked(s: &Static, min: [f32; 2], max: [f32; 2], label: &str) -> Option<Picked> {
        let f = &s.fabric;
        let same =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        let (r, c) = (
            f.row_at((min[1] + max[1]) / 2.0),
            ((min[0] + max[0]) / 2.0 / TILE_W) as u16,
        );
        let base = [c as f32 * TILE_W, f.row_top[r as usize]];
        let t = f.template(r, c);
        if let Some(b) = t
            .boxes
            .iter()
            .find(|b| b.label == label && same([base[0] + b.min[0], base[1] + b.min[1]], min))
        {
            return Some(Picked {
                at: (r, c),
                kind: b.kind,
                label: b.label.clone(),
                min,
                max,
                pins: pins_of(t, b),
                dsp: false,
            });
        }
        f.macros.iter().find_map(|m| {
            let b = m
                .parts
                .iter()
                .find(|b| b.label == label && same(b.min, min))?;
            Some(Picked {
                at: (m.row, ((b.min[0] + b.max[0]) / 2.0 / TILE_W) as u16),
                kind: b.kind,
                label: b.label.clone(),
                min: b.min,
                max: b.max,
                pins: Vec::new(),
                dsp: true,
            })
        })
    }

    fn inspect(design: &str) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference")
            .join(design);
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
        let start = Instant::now();
        let s = loop {
            {
                let sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{}", sh.error.as_ref().unwrap());
                if let Some(r) = &sh.ready {
                    break Arc::clone(r);
                }
            }
            assert!(start.elapsed() < Duration::from_secs(600), "not ready");
            std::thread::sleep(Duration::from_millis(50));
        };
        let o = &s.overlay;
        // A few states, each a few cycles on.
        let (mut luts, mut known, mut ffs, mut others, mut carries, mut ramw) = (0, 0, 0, 0, 0, 0);
        let mut shows = 0;
        for round in 0..4 {
            let before = shared.lock().unwrap().cycles;
            commands.send(Command::Step(3 + round)).unwrap();
            let start = Instant::now();
            let lit = loop {
                std::thread::sleep(Duration::from_millis(10));
                let sh = shared.lock().unwrap();
                if !sh.running && sh.cycles > before && sh.lit.len() == o.values.len() {
                    break sh.lit.clone();
                }
                assert!(
                    start.elapsed() < Duration::from_secs(60),
                    "the step did not end"
                );
            };
            for site in &o.sites {
                let p = picked(&s, site.min, site.max, &site.label)
                    .unwrap_or_else(|| panic!("{} at {:?} cannot be picked", site.label, site.min));
                let d = details(&s, &p);
                match &d.inside {
                    Inside::Lut(l) => {
                        luts += 1;
                        let init = l
                            .init
                            .unwrap_or_else(|| panic!("{}: INIT not known", d.title));
                        let address = l.inputs.iter().enumerate().try_fold(0usize, |a, (i, x)| {
                            Some(a | usize::from(input(&lit, *x)?) << i)
                        });
                        let carry = match l.carry {
                            None | Some((_, true)) => Some(false),
                            Some((c, false)) => bit(&lit, c),
                        };
                        // The box shows F.
                        assert_eq!(
                            read(&lit, Some(site.value)),
                            read(&lit, l.out),
                            "{}: its box shows another value than F; routes here: {:?}",
                            d.title,
                            s.transfers.get(&p.at).map(|t| t
                                .iter()
                                .filter(|(a, z)| a.starts_with('F') || z.starts_with('F'))
                                .collect::<Vec<_>>())
                        );
                        shows += 1;
                        // RAM keeps its own content, which the design writes.
                        if l.ram {
                            continue;
                        }
                        if let (Some(a), Some(f), Some(c)) = (address, bit(&lit, l.out), carry) {
                            known += 1;
                            assert_eq!(
                                (init >> a & 1 == 1) ^ c,
                                f,
                                "{}: INIT 0x{init:04X} at address {a} (carry {c}) but F is {f}",
                                d.title
                            );
                        }
                        // The carry out: the carry in on a 1 of INIT (propagate),
                        // else INIT's low four read by B A (generate).
                        if let (Some(a), Some((cin, inject)), Some(out)) =
                            (address, l.carry, bit(&lit, l.carry_out))
                        {
                            let g = a & 3;
                            let expected = if init >> a & 1 == 1 {
                                bit(&lit, cin)
                            } else {
                                Some(!inject && init >> g & 1 == 1)
                            };
                            if let Some(e) = expected {
                                carries += 1;
                                assert_eq!(
                                    e, out,
                                    "{}: INIT 0x{init:04X} at {a}, carry out {out}",
                                    d.title
                                );
                            }
                        }
                    }
                    Inside::Ramw(outputs) => {
                        // Each output is the input it passes on.
                        for (name, wire, from, i) in outputs {
                            if let (Some(o), Some(v)) = (bit(&lit, *wire), input(&lit, *i)) {
                                ramw += 1;
                                assert_eq!(o, v, "{}: {name} is not its input {from}", d.title);
                            }
                        }
                    }
                    Inside::FlipFlop(f) => {
                        ffs += 1;
                        assert_eq!(
                            read(&lit, Some(site.value)),
                            read(&lit, f.q),
                            "{}: its box shows another value than Q",
                            d.title
                        );
                        assert!(f.q.is_some(), "{}: no Q", d.title);
                        assert!(f.clock.is_some(), "{}: no clock", d.title);
                        assert!(f.data.is_some(), "{}: no D", d.title);
                    }
                    _ => panic!("{} opens as neither a LUT nor a flip-flop", d.title),
                }
            }
            for b in &o.boxes {
                let p = picked(&s, b.min, b.max, &b.label)
                    .unwrap_or_else(|| panic!("{} at {:?} cannot be picked", b.label, b.min));
                let d = details(&s, &p);
                others += 1;
                // What the map lights, the inspector shows in use.
                let in_use = match &d.inside {
                    Inside::Cib(rows) => !rows.is_empty(),
                    Inside::Pio(q) => q.i.is_some() || q.t.is_some() || q.o.is_some(),
                    _ => d.pins.iter().any(|q| q.wire.is_some()),
                };
                assert!(
                    in_use,
                    "{}: lit on the map, but no pin of it in use",
                    d.title
                );
                if let Inside::Cib(rows) = &d.inside {
                    assert!(
                        !rows.is_empty(),
                        "{}: a CIB in use with no J wire in use",
                        d.title
                    );
                    assert!(
                        rows.iter().any(|j| j.wire.is_some()),
                        "{}: no J wire has a value",
                        d.title
                    );
                    if round == 0 {
                        for j in rows {
                            println!(
                                "  {} {} = {:?}: from {} to {}",
                                d.title,
                                j.name,
                                read_value(&lit, j.wire),
                                j.from,
                                j.to
                            );
                        }
                    }
                }
                if p.label.starts_with("PIO") {
                    assert!(
                        d.pins.iter().any(|q| q.wire.is_some()),
                        "{}: a pad in use with no pin in use",
                        d.title
                    );
                    if round == 0 {
                        println!("  {}: {}; {}", d.title, d.kind, d.notes.join(" "));
                    }
                }
            }
        }
        println!(
            "{design}: {luts} LUT views ({shows} boxes showing F; {known} with every input known, each F its INIT bit; {carries} carry outs as INIT says; {ramw} RAMW outputs equal to their inputs), {ffs} flip-flop views, {others} other sites, over 4 states"
        );
        assert!(known > 0, "no LUT had all its inputs known");
    }

    fn read_value(lit: &[u8], w: Option<u32>) -> Option<u8> {
        read(lit, w)
    }

    /// The schematics of one element of each kind the designs use, as SVG
    /// images in target/schematics, to look at.
    #[test]
    #[ignore]
    fn schematics_drawn() {
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/schematics");
        std::fs::create_dir_all(&out).unwrap();
        for design in ["blinky_cycle", "uart_hello", "picorv32_console"] {
            let (s, shared, commands) = load(design);
            let before = shared.lock().unwrap().cycles;
            commands.send(Command::Step(5)).unwrap();
            let lit = loop {
                std::thread::sleep(Duration::from_millis(10));
                let sh = shared.lock().unwrap();
                if !sh.running && sh.cycles > before && sh.lit.len() == s.overlay.values.len() {
                    break sh.lit.clone();
                }
            };
            let o = &s.overlay;
            let mut seen = std::collections::BTreeSet::new();
            let mut elements: Vec<Picked> = Vec::new();
            for site in &o.sites {
                elements.extend(picked(&s, site.min, site.max, &site.label));
            }
            for b in &o.boxes {
                elements.extend(picked(&s, b.min, b.max, &b.label));
            }
            // A whole slice where a LUT is lit.
            if let Some(site) = o.sites.first() {
                let p = picked(&s, site.min, site.max, &site.label).unwrap();
                let t = s.fabric.template(p.at.0, p.at.1);
                let base = [p.at.1 as f32 * TILE_W, s.fabric.row_top[p.at.0 as usize]];
                if let Some(b) = t.boxes.iter().find(|b| b.kind == BoxKind::Slice) {
                    let min = [base[0] + b.min[0], base[1] + b.min[1]];
                    let max = [base[0] + b.max[0], base[1] + b.max[1]];
                    elements.extend(picked(&s, min, max, &b.label));
                }
            }
            for p in elements {
                let d = details(&s, &p);
                let kind = match &d.inside {
                    Inside::Lut(l) if l.ram => "lut_dpram".to_string(),
                    Inside::Lut(l) if l.carry.is_some() => "lut_ccu2".to_string(),
                    Inside::Lut(_) => "lut".to_string(),
                    Inside::Ramw(_) => "ramw".to_string(),
                    Inside::FlipFlop(_) => "ff".to_string(),
                    Inside::Mux(_) => "mux".to_string(),
                    Inside::Control(_) => "control".to_string(),
                    Inside::Slice(_) => "slice".to_string(),
                    Inside::Cib(_) => "cib".to_string(),
                    Inside::Pio(_) => "pio".to_string(),
                    Inside::Nothing => {
                        format!("site_{}", p.label.trim_end_matches(char::is_numeric))
                    }
                };
                if !seen.insert(kind.clone()) {
                    continue;
                }
                let file = out.join(format!("{design}_{}.svg", kind.replace(' ', "_")));
                std::fs::write(&file, schematic::svg(&d, &lit, false)).unwrap();
                let values = out.join(format!("{design}_{}_01.svg", kind.replace(' ', "_")));
                std::fs::write(&values, schematic::svg(&d, &lit, true)).unwrap();
                println!("{}: {}", d.title, file.display());
            }
        }
    }

    /// A design loaded live, ready.
    fn load(
        design: &str,
    ) -> (
        Arc<Static>,
        Arc<Mutex<Shared>>,
        std::sync::mpsc::Sender<Command>,
    ) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference")
            .join(design);
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
        let start = Instant::now();
        loop {
            {
                let sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{}", sh.error.as_ref().unwrap());
                if let Some(r) = &sh.ready {
                    let r = Arc::clone(r);
                    drop(sh);
                    return (r, shared, commands);
                }
            }
            assert!(start.elapsed() < Duration::from_secs(600), "not ready");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    #[ignore]
    fn blinky_cycle_inspected() {
        inspect("blinky_cycle");
    }

    #[test]
    #[ignore]
    fn uart_hello_inspected() {
        inspect("uart_hello");
    }

    #[test]
    #[ignore]
    fn picorv32_console_inspected() {
        inspect("picorv32_console");
    }
}
