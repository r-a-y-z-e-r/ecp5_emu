//! The die: every tile with every wire, arc and site of its type
//! (src/fabric.rs), in grey. Tracks show from a few tiles across, local
//! wires, pins and jumpers closer, and a dot for every arc closer still.
//! With a design (src/fabric_overlay.rs) the wires, arcs, LUTs and
//! flip-flops it uses are beige, and every change spreads in orange from
//! the net's driver to its far end at the pace of the run, glowing on where
//! it has been (src/changes.rs). Their values (1, 0) can be coloured
//! instead. Scroll or pinch to zoom, drag to pan, click a wire to keep its
//! net, a site or a part of a slice to inspect it (src/inspector.rs),
//! double-click to fit.
//!
//! The design's wires in sight are found again only when the view moves. A
//! large design from afar (hundreds of thousands of pieces, most a pixel or
//! two long) has them drawn into an image, a band of rows a thread, kept
//! while neither the view nor the colours change; a change is drawn over a
//! copy of it, on every piece it lights, however many share a pixel. Where
//! the GPU draws the wires (src/gpu_map.rs), it also draws the design's
//! sites, boxes and grid tiles while they are a few pixels across, from the
//! wires' records: the CPU then only draws those seen closer, with their
//! outlines and labels, and a frame of a full chip costs it next to nothing.
use crate::{
    changes::{Changes, Motion},
    gpu_map::{self, MapCallback, Readback, Segment, Spot, Spots, SpotsCallback, WireStates},
    trace::{Drawn, Trace},
};
use ecp5_emu::{
    database::Database,
    fabric::{self, BoxKind, Fabric, Part, TILE_W},
    fabric_overlay::Overlay,
    live,
    model::DeviceModel,
    sources::{Index, SourceKind, SourceLocation, Span},
    timing_path::{Kind as HopKind, MappedHop, MappedPath, Site, Tile},
};
use eframe::{
    egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2},
    egui_wgpu,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
struct Loaded {
    fabric: Option<Arc<Fabric>>,
    error: Option<String>,
}

pub struct FabricApp {
    loaded: Arc<Mutex<Loaded>>,
    view: FabricView,
}

/// The die's fabric with its camera, and a design's values over it.
pub struct FabricView {
    /// Pixels per die unit (a tile is TILE_W wide).
    cell: f32,
    offset: Vec2,
    fitted: bool,
    /// A tile to centre and the pixels per die unit for the first frame.
    pub start: (Option<(u16, u16)>, Option<f32>),
    /// Where in the focused tile to centre, as shares of the tile.
    inside: (f32, f32),
    pub hovered: Option<(u16, u16)>,
    pub selected: Option<(u16, u16)>,
    /// The net of the wire pointed at, and the one clicked (kept).
    hover_net: Option<u32>,
    pub net: Option<u32>,
    kept_nets: Vec<u32>,
    focus_nets: bool,
    /// A menu's net is kept while the pointer moves onto its button.
    menu_net: Option<(u32, String)>,
    /// A menu choice for App to take after this frame's map.
    pub watch_net: Option<u32>,
    /// Source actions keep their original target while the menu is open.
    menu_sources: Vec<SourceLocation>,
    menu_source_target: String,
    menu_source_reason: Option<&'static str>,
    pub show_source: Option<(String, Vec<SourceLocation>, Span)>,
    pub sources: Option<Arc<Index>>,
    pub project_sources: bool,
    pub vhdl_sources: bool,
    pub critical: Option<Arc<MappedPath>>,
    focus_hop: Option<MappedHop>,
    /// The report selection outlives the one-frame camera request.
    critical_focus: Option<(Arc<MappedPath>, usize)>,
    /// Links drawn this frame, by the frame they were drawn in.
    drawn: Vec<u64>,
    frame: u64,
    /// The changes of the design's wires: where they have spread and how
    /// much they glow.
    changes: Changes,
    /// Each net's length from its driver to its far end (by net), and the
    /// design they are of: where a change has spread to is a share of it.
    lengths: RefCell<(usize, Vec<f32>)>,
    /// Each position's tile colour (by its tile types), and the fabric it
    /// is of: found once, not every frame.
    tile_colours: (usize, Vec<Color32>),
    /// When the record last had how busy the changing wires are.
    busy_noted: Option<Instant>,
    /// The design's wires in sight, kept while the view stays, and the
    /// image they are drawn as from afar.
    sight: RefCell<InSight>,
    image: RefCell<WireImage>,
    /// From how many pieces in sight they are drawn as an image.
    raster_from: usize,
    /// The record of what is drawn, when kept (src/trace.rs); how long the
    /// last drawing of the map took, in milliseconds; the zoom it was
    /// fitted at.
    pub trace: RefCell<Option<Trace>>,
    pub draw_ms: f64,
    /// Where the map was last drawn, in points.
    pub map_rect: Rect,
    /// The next drawing explains the wires the record missed most.
    pub why: std::cell::Cell<bool>,
    /// Drawings of the design's map so far: the frame a change is first
    /// drawn in, for the record.
    draws: std::cell::Cell<u32>,
    fit_cell: f32,
    /// A place to show from the next drawing (`aim`), and the die's size in
    /// its units as last drawn (for a script's `sweep`).
    aim: Option<(f32, f32, f32)>,
    pub die: Vec2,
    /// The GPU's share, when it draws the design's wires (src/gpu_map.rs).
    gpu: RefCell<Option<GpuView>>,
    /// Colour the design's wires by their values instead.
    pub values: bool,
    /// How the GPU draws changes; a dense choice keeps quads close up.
    pub glow: gpu_map::GlowChoice,
    /// This view's number among those made in the window (a design each).
    generation: usize,
    /// Draw the wires; without them the die is its tiles and sites, those
    /// in use lit (the grid).
    pub wires: bool,
    /// The site or part of a slice under the pointer, and the one clicked,
    /// which the inspector shows.
    hover_element: Option<Picked>,
    pub picked: Option<Picked>,
}

/// A site, a part of a slice or a part of a DSP block, as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    /// The position whose template draws it; for a part of a DSP block, the
    /// block's row and the part's column.
    pub at: (u16, u16),
    pub kind: BoxKind,
    pub label: String,
    /// Where it is on the die.
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// The wires that end on it, as the position's tile types name them.
    pub pins: Vec<String>,
    /// A part of a DSP block, drawn apart from the tiles.
    pub dsp: bool,
}

impl FabricApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        focus: Option<(u16, u16)>,
        cell: Option<f32>,
        inside: (f32, f32),
    ) -> Self {
        let preference = crate::start::Preferences::load().map_or_else(
            |e| {
                eprintln!("Reading preferences: {e}");
                egui::ThemePreference::System
            },
            |p| p.theme,
        );
        crate::theme::install(&cc.egui_ctx, preference);
        let loaded = Arc::new(Mutex::new(Loaded::default()));
        let shared = Arc::clone(&loaded);
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Fabric> {
                let model = DeviceModel::load(&Database::discover(None)?, &live::architecture())?;
                Ok(fabric::cached(&model, &live::architecture()))
            })();
            let mut l = shared.lock().unwrap();
            match result {
                Ok(f) => l.fabric = Some(Arc::new(f)),
                Err(e) => {
                    eprintln!("fabric: {e:#}");
                    l.error = Some(format!("{e:#}"));
                }
            }
            ctx.request_repaint();
        });
        Self {
            loaded,
            view: FabricView::new(focus, cell, inside),
        }
    }
}

impl FabricView {
    /// A script can compare the ways of drawing in one running window.
    pub fn set_glow(&mut self, name: &str) -> bool {
        let Some(choice) = gpu_map::GlowChoice::parse(name) else {
            return false;
        };
        self.glow = choice;
        true
    }

    pub fn new(focus: Option<(u16, u16)>, cell: Option<f32>, inside: (f32, f32)) -> Self {
        Self {
            cell: 8.0,
            offset: Vec2::ZERO,
            fitted: false,
            start: (focus, cell),
            inside,
            hovered: None,
            selected: None,
            hover_net: None,
            net: None,
            kept_nets: Vec::new(),
            focus_nets: false,
            menu_net: None,
            watch_net: None,
            menu_sources: Vec::new(),
            menu_source_target: String::new(),
            menu_source_reason: None,
            show_source: None,
            sources: None,
            project_sources: false,
            vhdl_sources: false,
            critical: None,
            focus_hop: None,
            critical_focus: None,
            drawn: Vec::new(),
            frame: 0,
            changes: Changes::default(),
            lengths: RefCell::default(),
            tile_colours: (0, Vec::new()),
            busy_noted: None,
            sight: RefCell::default(),
            image: RefCell::default(),
            raster_from: RASTER_FROM,
            trace: RefCell::new(None),
            draw_ms: 0.0,
            map_rect: Rect::NOTHING,
            why: Default::default(),
            draws: Default::default(),
            fit_cell: 0.0,
            aim: None,
            die: Vec2::ZERO,
            gpu: RefCell::new(None),
            values: false,
            glow: gpu_map::GlowChoice::Auto,
            generation: {
                static VIEWS: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(1);
                VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xffff
            },
            wires: true,
            hover_element: None,
            picked: None,
        }
    }

    /// Takes the values of the overlay's wires now, and those the run says
    /// changed since the last taken (src/live.rs), to be shown as `motion`
    /// has it (src/changes.rs).
    pub fn update(&mut self, values: &[u8], toggled: &[u8], motion: Motion) {
        self.changes.motion = motion;
        self.changes.update(values, toggled);
        let found = self.changes.found();
        if let Some(t) = self.trace.get_mut() {
            if t.wires(values.len()) {
                self.sight.get_mut().key = None;
            }
            let now = self.changes.now();
            // The next drawing is the first to have them.
            let frame = self.draws.get() + 1;
            for &i in found {
                t.came(i as u32, values[i], now, motion, frame);
            }
            // At a fast rate, every two seconds: how busy the wires that
            // changed are, and so how much their changes glow (src/changes.rs
            // `BUSY_STEP`), as quantiles: a `# busy` line.
            let due = self
                .busy_noted
                .is_none_or(|at| at.elapsed().as_secs_f64() >= 2.0);
            if motion.spread < crate::changes::FAST && due && !found.is_empty() {
                self.busy_noted = Some(Instant::now());
                let step = (found.len() / 20_000).max(1);
                let mut busy: Vec<f32> = found
                    .iter()
                    .step_by(step)
                    .map(|&i| self.changes.busy(i as u32, now))
                    .collect();
                busy.sort_by(f32::total_cmp);
                let q = |p: f64| busy[((busy.len() - 1) as f64 * p) as usize];
                let weight = crate::changes::glow_weight;
                let _ = t.note(&format!(
                    "busy\t{now:.3}\t{} wires changed\tbusy p10 {:.2} p50 {:.2} p90 {:.2}\tglow p10 {:.2} p50 {:.2} p90 {:.2}",
                    found.len(),
                    q(0.1),
                    q(0.5),
                    q(0.9),
                    weight(q(0.1)),
                    weight(q(0.5)),
                    weight(q(0.9)),
                ));
            }
        }
        // The GPU's copy of the wires' changes, where they changed.
        if let Some(g) = self.gpu.get_mut() {
            let records = self.changes.records();
            let mut wires = g.wires.lock().unwrap();
            if wires.states.len() != records.len() {
                wires.set_all(records.to_vec());
            } else {
                for &i in found {
                    wires.set(i as u32, records[i]);
                }
            }
        }
    }

    /// The GPU draws the design's wires from now on (the GPU's side set up
    /// by `gpu_map::GpuMap::install`).
    pub fn use_gpu(&mut self) {
        *self.gpu.get_mut() = Some(GpuView {
            design: 0,
            segments: None,
            starts: Vec::new(),
            spots_design: 0,
            spots: None,
            spot_starts: Default::default(),
            largest: (0.0, 0.0),
            wires: Arc::default(),
            readback: Arc::default(),
            measurements: Arc::default(),
            dropped: Arc::default(),
        });
    }

    /// How many frames of the GPU's map egui dropped (the window covered):
    /// prepared, then not drawn.
    pub fn dropped(&self) -> u32 {
        self.gpu
            .borrow()
            .as_ref()
            .map_or(0, |g| g.dropped.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// The last preparation's uploads and any new, late GPU times.
    pub fn gpu_measurements(&self) -> gpu_map::Measurements {
        self.gpu
            .borrow()
            .as_ref()
            .map_or_else(Default::default, |g| {
                std::mem::take(&mut *g.measurements.lock().unwrap())
            })
    }

    /// Zooms by `factor` about the middle of the map.
    pub fn zoom(&mut self, factor: f32, view: Vec2) {
        let middle = view / 2.0;
        self.offset = middle - (middle - self.offset) * factor;
        self.cell *= factor;
    }

    fn watch_menu(&mut self, response: &egui::Response, names: Option<&HashMap<u32, Vec<String>>>) {
        if response.secondary_clicked() {
            self.menu_net = self.hover_net.map(|net| {
                let name = names
                    .and_then(|names| names.get(&net))
                    .and_then(|names| names.first())
                    .cloned()
                    .unwrap_or_else(|| format!("net {net}"));
                (net, name)
            });
            (self.menu_source_target, self.menu_sources) =
                self.source_target(self.hover_net, self.hover_element.as_ref());
            self.menu_source_reason = (self.menu_sources.is_empty()
                && (self.hover_net.is_some() || self.hover_element.is_some()))
            .then(|| {
                if self.vhdl_sources {
                    ecp5_emu::flow::report::VHDL_SOURCE_LIMIT
                } else {
                    source_unavailable(self.sources.as_deref(), self.project_sources)
                }
            });
        }
        if self.menu_net.is_some()
            || !self.menu_sources.is_empty()
            || self.menu_source_reason.is_some()
        {
            response.context_menu(|ui| {
                if let Some((net, name)) = &self.menu_net {
                    if ui.button(format!("Watch this net ({name})")).clicked() {
                        self.watch_net = Some(*net);
                        ui.close();
                    }
                }
                let chosen = source_menu(ui, &self.menu_sources, self.menu_source_reason);
                if let Some(span) = chosen {
                    self.choose_source(
                        self.menu_source_target.clone(),
                        self.menu_sources.clone(),
                        span,
                    );
                    ui.close();
                }
            });
        }
    }

    fn source_target(
        &self,
        net: Option<u32>,
        site: Option<&Picked>,
    ) -> (String, Vec<SourceLocation>) {
        let Some(index) = self.sources.as_deref() else {
            return (String::new(), Vec::new());
        };
        if let Some(net) = net {
            return (index.net_label(net), index.net_source_choices(net));
        }
        if let Some(site) = site {
            return (
                format!("R{}C{}/{}", site.at.0, site.at.1, site.label),
                source_choices(&index.site_locations(site.at.0, site.at.1, &site.label)),
            );
        }
        (String::new(), Vec::new())
    }

    /// The script takes the same menu action, using shares of the map just
    /// as `point` does, without depending on the pointer's previous frame.
    pub fn source_at(
        &self,
        f: &Fabric,
        o: &Overlay,
        x: f32,
        y: f32,
    ) -> (String, Vec<SourceLocation>) {
        let p = (self.map_rect.size() * Vec2::new(x, y) - self.offset) / self.cell;
        let net = self.wires.then(|| self.pick(f, o, [p.x, p.y])).flatten();
        let site = net
            .is_none()
            .then(|| self.element_at(f, [p.x, p.y], true))
            .flatten();
        self.source_target(net, site.as_ref())
    }

    /// A visible piece of a kept net that the ordinary map picker agrees on.
    /// Prefer a long piece so the pointer is easy to see in a recording.
    pub fn probed_point(&self, f: &Fabric, o: &Overlay) -> Option<Pos2> {
        let mut best = None;
        let mut length = 0.0;
        let view = self.map_rect.shrink(24.0);
        for &net in self.kept() {
            for &i in o.pieces_of(net) {
                let piece = &o.pieces[i as usize];
                if !piece.route {
                    continue;
                }
                let a = Pos2::from(piece.a);
                let b = Pos2::from(piece.b);
                let size = a.distance(b) * self.cell;
                if size <= length {
                    continue;
                }
                for share in [0.5, 0.25, 0.75] {
                    let p = a.lerp(b, share);
                    let screen = self.map_rect.min + self.offset + p.to_vec2() * self.cell;
                    if view.contains(screen)
                        && self.pick(f, o, [p.x, p.y]) == Some(net)
                        && !self.source_target(Some(net), None).1.is_empty()
                    {
                        best = Some(screen);
                        length = size;
                        break;
                    }
                }
            }
        }
        best
    }

    /// The menu and automation dispatch the same chosen source location.
    fn choose_source(&mut self, target: String, locations: Vec<SourceLocation>, chosen: Span) {
        self.show_source = Some((target, locations, chosen));
    }

    pub fn show_source_at(&mut self, f: &Fabric, o: &Overlay, point: Pos2) -> bool {
        let share = (point - self.map_rect.min) / self.map_rect.size();
        let (target, locations) = self.source_at(f, o, share.x, share.y);
        let Some(chosen) = locations.first().map(|location| location.span.clone()) else {
            return false;
        };
        self.choose_source(target, locations, chosen);
        true
    }

    pub fn keep_nets(&mut self, nets: Vec<u32>) {
        self.net = nets.first().copied();
        self.kept_nets = nets;
        self.focus_nets = true;
    }

    fn kept(&self) -> &[u32] {
        if self.net == self.kept_nets.first().copied() {
            &self.kept_nets
        } else {
            &[]
        }
    }

    pub fn has_probe(&self) -> bool {
        !self.kept().is_empty()
    }

    /// The net of the wire under the pointer, if any.
    pub fn pointed_net(&self) -> Option<u32> {
        self.hover_net
    }

    /// Fits the die to the map again.
    pub fn fit(&mut self) {
        self.fitted = false;
    }

    /// From the next drawing, the point at shares `(x, y)` of the die in
    /// the middle of the map, at `zoom` times the zoom that fits the die.
    pub fn aim(&mut self, x: f32, y: f32, zoom: f32) {
        self.aim = Some((x, y, zoom));
    }

    pub fn centre_hop(&mut self, hop: &MappedHop) {
        self.focus_hop = Some(hop.clone());
    }

    /// Read-only camera evidence for the window record, in die units and pixels per unit.
    pub(crate) fn camera_centre(&self) -> (Vec2, f32) {
        (
            (self.map_rect.size() / 2.0 - self.offset) / self.cell,
            self.cell,
        )
    }

    /// The zoom, as a share of the zoom that fits the die.
    pub fn zoom_level(&self) -> f32 {
        if self.fit_cell > 0.0 {
            self.cell / self.fit_cell
        } else {
            1.0
        }
    }

    /// Whether a change is spreading or glowing.
    pub fn animating(&self) -> bool {
        self.changes.animating()
    }
}

/// What the design uses; a change spreading along it.
pub const USED: Color32 = Color32::from_rgb(238, 218, 172);
pub const CHANGE: Color32 = Color32::from_rgb(255, 112, 0);
/// A LUT or flip-flop in use, its edge and its label.
pub const SITE: Color32 = Color32::from_rgb(214, 192, 146);
pub const SITE_EDGE: Color32 = Color32::from_rgb(255, 240, 205);
pub const SITE_TEXT: Color32 = Color32::from_rgb(40, 32, 20);

/// With 0/1, a wire at 1 is lit, one at 0 a dim tan, of the same warm
/// family as what is in use; one whose value is not known grey-violet.
pub const ONE: Color32 = Color32::from_rgb(255, 238, 196);
pub const ZERO: Color32 = Color32::from_rgb(142, 116, 76);
pub const UNKNOWN: Color32 = Color32::from_rgb(150, 128, 200);
/// Text on a site at 0.
const SITE_TEXT_LIGHT: Color32 = Color32::from_rgb(245, 236, 216);

/// The camera that shows the point at shares `(x, y)` of a die `size`
/// units big in the middle of a map `map` points big, at `zoom` times the
/// zoom that fits the die: pixels per die unit (no nearer than the wheel
/// goes), and where the die's corner is in the map. None for an empty die
/// or map.
pub fn aimed(size: Vec2, map: Vec2, (x, y, zoom): (f32, f32, f32)) -> Option<(f32, Vec2)> {
    let fit = (map.x / size.x).min(map.y / size.y);
    if !(fit.is_finite() && fit > 0.0 && zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    let cell = (fit * zoom).min(12000.0);
    Some((cell, map / 2.0 - Vec2::new(x * size.x, y * size.y) * cell))
}

/// Where the views of a sweep of the die at `zoom` (a script's `sweep`) are
/// centred, as shares of the die: as many across and down as cover it, row
/// by row, each row the other way, so each view is next to the one before.
pub fn sweep(size: Vec2, map: Vec2, zoom: f32) -> Vec<(f32, f32)> {
    let Some((cell, _)) = aimed(size, map, (0.5, 0.5, zoom)) else {
        return Vec::new();
    };
    // A view shows `map / cell` of the die; a hair over a whole number of
    // views is not one more.
    let views = |die: f32, seen: f32| ((die * cell / seen - 1e-3).ceil() as usize).max(1);
    let (across, down) = (views(size.x, map.x), views(size.y, map.y));
    let share = |k: usize, n: usize| (k as f32 + 0.5) / n as f32;
    (0..down)
        .flat_map(|j| {
            (0..across).map(move |i| {
                let i = if j % 2 == 1 { across - 1 - i } else { i };
                (share(i, across), share(j, down))
            })
        })
        .collect()
}

fn colour(v: Option<&u8>) -> Color32 {
    match v {
        Some(1) => ONE,
        Some(0) => ZERO,
        _ => UNKNOWN,
    }
}

const BACKGROUND: Color32 = Color32::from_rgb(12, 13, 16);

#[derive(Clone, Copy)]
struct LegendCounts {
    drawn: usize,
    hidden: usize,
    routes: usize,
    sites: usize,
}

/// The full explanations shown when hovering the legend's `?` button.
fn legend_help(counts: Option<LegendCounts>, values: bool, wires: bool) -> String {
    let mut parts = Vec::new();
    if values {
        parts.push("Orange: a change spreading from its driver.".to_owned());
    } else {
        parts.push(
            "Used: used by the design. Change: a change spreading from its driver.".to_owned(),
        );
    }
    if let Some(c) = counts {
        let hidden = if c.hidden > 0 {
            format!(" ({} left out by the fabric)", c.hidden)
        } else {
            String::new()
        };
        parts.push(format!(
            "{} of the plan's {} routing transfers drawn as arcs of the fabric{}; {} LUTs and flip-flops.",
            c.drawn - c.hidden,
            c.routes,
            hidden,
            c.sites
        ));
    }
    parts.push(
        if wires {
            "A 0 or 1 in a small box: a CIB ties the wire to that constant. Point at a wire for its whole net (a ring where it starts, a square where a constant ties it, a dot where it ends); click to keep it. The design's wires show at any zoom; the empty fabric's tracks from ~100 px per tile, its local wires from ~500, dots from ~1300."
        } else {
            "The tiles and their sites, without the wires: those the design uses lit, and a change of what they drive flashes."
        }
        .to_owned(),
    );
    parts.join("\n\n")
}

fn legend_with_counts(ui: &mut egui::Ui, counts: Option<LegendCounts>, values: bool, wires: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.visuals_mut().override_text_color =
            Some(crate::theme::colours(ui.visuals().dark_mode).muted);
        let swatch = |ui: &mut egui::Ui, colour: Color32, text: &str| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().rect_filled(rect, 1.0, colour);
            ui.label(text);
        };
        if values {
            swatch(ui, ONE, "1");
            swatch(ui, ZERO, "0");
            swatch(ui, UNKNOWN, "unknown");
            swatch(ui, CHANGE, "change");
        } else {
            swatch(ui, USED, "used");
            swatch(ui, CHANGE, "change");
        }
        swatch(ui, Color32::from_gray(60), "not used");
        if let Some(c) = counts {
            ui.separator();
            ui.label(format!(
                "{} arcs drawn, {} sites",
                c.drawn - c.hidden,
                c.sites
            ));
        }
        ui.small_button("?")
            .on_hover_text(legend_help(counts, values, wires));
    });
}

/// The colour key shown under the fabric, with what the design uses of it.
pub fn legend(
    ui: &mut egui::Ui,
    design: Option<&ecp5_emu::live::Static>,
    values: bool,
    wires: bool,
) {
    let counts = design.map(|s| {
        let o = &s.overlay;
        LegendCounts {
            drawn: o.drawn,
            hidden: o.hidden,
            routes: o.routes,
            sites: o.sites.len(),
        }
    });
    legend_with_counts(ui, counts, values, wires);
}

fn tile_colour(kinds: &[String]) -> Color32 {
    let has = |p: &str| kinds.iter().any(|k| k.contains(p));
    if has("PLC2") {
        Color32::from_rgb(30, 31, 35)
    } else if has("EBR") || has("DSP") {
        Color32::from_rgb(34, 33, 36)
    } else if has("PIC") || has("PIO") {
        Color32::from_rgb(32, 35, 34)
    } else if has("CIB") {
        Color32::from_rgb(26, 27, 30)
    } else {
        Color32::from_rgb(20, 21, 24)
    }
}

fn part_stroke(part: Part, width: f32, dim: bool) -> Stroke {
    let grey: u8 = match part {
        Part::Track => 105,
        Part::Local => 88,
        Part::Jumper => 125,
        Part::Pin => 140,
        Part::Clock => 115,
        Part::Internal => 140,
    };
    // Under a design what it does not use is a little quieter, still grey.
    let grey = if dim {
        (grey as u16 * 4 / 5) as u8
    } else {
        grey
    };
    Stroke::new(width, Color32::from_gray(grey))
}

impl eframe::App for FabricApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (fabric, error) = {
            let l = self.loaded.lock().unwrap();
            (l.fabric.clone(), l.error.clone())
        };
        egui::Panel::top("info").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("LFE5U-85F, empty");
                ui.separator();
                match (&fabric, &error) {
                    (_, Some(e)) => {
                        ui.colored_label(Color32::LIGHT_RED, e);
                    }
                    (None, None) => {
                        ui.label("Loading the device model …");
                    }
                    (Some(f), None) => {
                        let (h, v) = f.lanes.counts();
                        ui.label(format!(
                            "{} × {} tiles, {} sites, {} arcs, {} links between tiles ({} to a wire not found), lanes: {h} horizontal, {v} vertical  |  {:.0} px per tile",
                            f.rows,
                            f.cols,
                            f.sites,
                            f.arcs,
                            f.links.len(),
                            f.broken_links,
                            self.view.cell * TILE_W
                        ));
                    }
                }
            });
        });
        // The text scrolls within the panel, which keeps its height whatever
        // is pointed at (else it grows over the pointer and back, in a loop).
        egui::Panel::bottom("tile").default_size(48.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match (&fabric, self.view.hovered) {
                (Some(f), Some((r, c))) => {
                    let t = f.template(r, c);
                    ui.label(format!(
                        "R{r}C{c}: {}  —  {} segments, {} dots, {} links out, {} arcs left out (they join a wire to nothing else)",
                        f.types_at(r, c).join(", "),
                        t.segments.len(),
                        t.dots.len(),
                        f.links_from(r, c).len(),
                        t.left_out,
                    ));
                }
                _ => {
                    ui.label("Scroll to zoom (tracks from ~30 px per tile, local wires from ~150, a dot per arc from ~400), drag to pan, double-click to fit.");
                }
            })
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(f) = fabric else {
                return;
            };
            self.view.show(ui, &f, None, None);
        });
    }
}

impl FabricView {
    /// Draws the die, and a design's values over it: `live` is the overlay
    /// and the values of its wires.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        f: &Fabric,
        live: Option<(&Overlay, &[u8])>,
        names: Option<&HashMap<u32, Vec<String>>>,
    ) {
        let started = Instant::now();
        let dim = live.is_some();
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let view = response.rect;
        let size = Vec2::from(f.size());
        if self.fitted && self.map_rect.is_finite() && view != self.map_rect {
            let fit_offset = (self.map_rect.size() - size * self.fit_cell) / 2.0;
            let at_fit = (self.cell - self.fit_cell).abs() <= self.fit_cell * 1e-5
                && (self.offset - fit_offset).length_sq() < 1e-4;
            if at_fit {
                self.fitted = false;
            } else {
                // Offset is local to the map. Move the scene with its centre,
                // accounting for a moved map origin as well as a changed size.
                self.offset +=
                    (view.center() - self.map_rect.center()) - (view.min - self.map_rect.min);
            }
        }
        self.map_rect = view;
        self.fit_cell = (view.width() / size.x).min(view.height() / size.y);
        if !self.fitted && view.width() > 0.0 {
            self.cell = self.fit_cell;
            self.offset = (view.size() - size * self.cell) / 2.0;
            if let (focus, Some(cell)) = std::mem::take(&mut self.start) {
                self.cell = cell;
                let (r, c) = focus.unwrap_or((f.rows / 2, f.cols / 2));
                let centre = Vec2::new(
                    (c as f32 + self.inside.0) * TILE_W,
                    f.row_top[r as usize] + self.inside.1 * f.row_height(r),
                );
                self.offset = view.size() / 2.0 - centre * cell;
            }
            self.fitted = true;
            ui.ctx().request_repaint();
        }
        self.die = size;
        if self.critical_focus.as_ref().is_some_and(|(path, _)| {
            self.critical
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(path, current))
        }) {
            self.critical_focus = None;
        }
        if let Some(aim) = self.aim.take() {
            if let Some((cell, offset)) = aimed(size, view.size(), aim) {
                (self.cell, self.offset) = (cell, offset);
            }
        }
        if self.focus_nets {
            if let Some((overlay, _)) = live {
                let mut bounds = Rect::NOTHING;
                for &net in self.kept() {
                    for &piece in overlay.pieces_of(net) {
                        let piece = &overlay.pieces[piece as usize];
                        bounds.extend_with(Pos2::from(piece.a));
                        bounds.extend_with(Pos2::from(piece.b));
                    }
                }
                if bounds.is_finite() {
                    let shown = Rect::from_min_max(
                        view.min + self.offset + bounds.min.to_vec2() * self.cell,
                        view.min + self.offset + bounds.max.to_vec2() * self.cell,
                    );
                    if !view.shrink(16.0).contains_rect(shown) {
                        self.cell = self
                            .cell
                            .min((view.width() - 32.0).max(1.0) / bounds.width().max(0.02))
                            .min((view.height() - 32.0).max(1.0) / bounds.height().max(0.02));
                        self.offset = view.size() / 2.0 - bounds.center().to_vec2() * self.cell;
                    }
                }
            }
            self.focus_nets = false;
        }
        let focused = ui.input(|i| i.focused);
        if let Some(pointer) = response.hover_pos().filter(|_| focused) {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zoom * (scroll / 300.0).exp();
            if factor != 1.0 {
                let new = (self.cell * factor).clamp(2.0, 12000.0);
                let at = pointer - view.min;
                self.offset = at - (at - self.offset) * (new / self.cell);
                self.cell = new;
            }
        }
        if response.dragged() {
            self.offset += response.drag_delta();
        }
        if response.double_clicked() {
            self.fitted = false;
        }
        if let Some(hop) = self.focus_hop.take() {
            self.critical_focus = self.critical.as_ref().and_then(|path| {
                path.hops
                    .iter()
                    .position(|candidate| {
                        candidate.hop.kind == hop.hop.kind
                            && candidate.hop.source == hop.hop.source
                            && candidate.hop.sink == hop.hop.sink
                            && candidate.hop.net == hop.hop.net
                            && candidate.hop.total == hop.hop.total
                    })
                    .map(|i| (Arc::clone(path), i + 1))
            });
            let mut bounds = Rect::NOTHING;
            for site in &hop.sites {
                if let Some(rect) = timing_site(f, site) {
                    bounds = bounds.union(rect);
                }
            }
            if let Some((a, b)) = hop.hop.tiles {
                for tile in [a, b] {
                    if let Some(p) = timing_tile(f, tile) {
                        bounds.extend_with(p);
                    }
                }
            }
            if let Some((overlay, _)) = live {
                for route in &hop.hop.route {
                    let piece = &overlay.pieces[route.piece as usize];
                    bounds.extend_with(Pos2::from(piece.a));
                    bounds.extend_with(Pos2::from(piece.b));
                }
            }
            if bounds.is_finite() {
                self.cell = self
                    .cell
                    .max(self.fit_cell * 8.0)
                    .min((view.width() - 40.0).max(1.0) / bounds.width().max(0.1))
                    .min((view.height() - 40.0).max(1.0) / bounds.height().max(0.1));
                self.offset = view.size() / 2.0 - bounds.center().to_vec2() * self.cell;
            }
        }
        let origin = view.min + self.offset;
        let cell = self.cell;
        // The net of the wire under the pointer; a click keeps it, or lets
        // it go off a wire.
        self.hover_net = match (live.filter(|_| self.wires), response.hover_pos()) {
            (Some((o, _)), Some(p)) => {
                let g = (p - origin) / cell;
                self.pick(f, o, [g.x, g.y])
            }
            _ => None,
        };
        // The site or part under the pointer, where no wire of the design
        // is: a click opens it in the inspector, a click off any closes it.
        // Only over a design: the empty die has no inspector.
        self.hover_element = match (dim, self.hover_net, response.hover_pos()) {
            (true, None, Some(p)) => {
                let g = (p - origin) / cell;
                self.element_at(f, [g.x, g.y], dim)
            }
            _ => None,
        };
        if self.hover_element.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            self.selected = self.hovered;
            self.net = self.hover_net;
            self.kept_nets.clear();
            if self.hover_net.is_none() {
                self.picked = self.hover_element.clone();
            }
        }
        if live.is_some() {
            self.watch_menu(&response, names);
        }
        let screen = |p: [f32; 2]| origin + Vec2::new(p[0], p[1]) * cell;
        painter.rect_filled(view, 0.0, BACKGROUND);

        // The tiles in sight.
        let first = (view.min - origin) / cell;
        let last = (view.max - origin) / cell;
        let (c0, c1) = (
            (first.x / TILE_W).floor().max(0.0) as u16,
            ((last.x / TILE_W).ceil().max(0.0) as u16).min(f.cols),
        );
        let (r0, r1) = (
            f.row_at(first.y.max(0.0)),
            (f.row_at(last.y.max(0.0)) + 1).min(f.rows),
        );
        // The grid keeps its tiles apart from further out.
        let gap = if cell >= if self.wires { 6.0 } else { 3.0 } {
            0.5
        } else {
            0.0
        };
        let (tracks, locals, dots) = (cell >= 30.0, cell >= 150.0, cell >= 400.0);
        let width = (cell * 0.0005).clamp(0.4, 1.0);
        let radius = (cell * 0.0008).clamp(0.6, 2.2);
        let dot = Color32::from_gray(if dim { 120 } else { 150 });
        // The tiles as one mesh: some ten thousand shapes from afar take
        // longer to lay out than to draw.
        let fabric_key = f as *const Fabric as usize;
        if self.tile_colours.0 != fabric_key {
            let colours = (0..f.rows)
                .flat_map(|r| (0..f.cols).map(move |c| (r, c)))
                .map(|(r, c)| tile_colour(f.types_at(r, c)))
                .collect();
            self.tile_colours = (fabric_key, colours);
        }
        let mut tiles = egui::Mesh::default();
        for r in r0..r1 {
            for c in c0..c1 {
                let tile = Rect::from_min_size(
                    screen([c as f32 * TILE_W, f.row_top[r as usize]]),
                    Vec2::new(TILE_W, f.row_height(r)) * cell,
                );
                let colour = self.tile_colours.1[r as usize * f.cols as usize + c as usize];
                tiles.add_colored_rect(tile.shrink(gap), colour);
            }
        }
        painter.add(egui::Shape::mesh(tiles));
        // Sites: the slices of the templates, then the others; under a
        // design the unused ones a little quieter, still grey.
        let (fill, edge, slice, text) = if dim {
            (
                Color32::from_rgb(40, 41, 46),
                Stroke::new(1.0, Color32::from_gray(100)),
                Color32::from_gray(68),
                Color32::from_gray(135),
            )
        } else {
            (
                Color32::from_rgb(44, 45, 50),
                Stroke::new(1.0, Color32::from_gray(120)),
                Color32::from_gray(80),
                Color32::from_gray(160),
            )
        };
        let font = egui::FontId::monospace((cell / 110.0).clamp(6.0, 12.0));
        // Without the wires the sites show from further out.
        let sites_from = if self.wires { 12.0 } else { 4.0 };
        if cell >= sites_from {
            for r in r0..r1 {
                for c in c0..c1 {
                    let base = [c as f32 * TILE_W, f.row_top[r as usize]];
                    for b in &f.template(r, c).boxes {
                        let site = b.kind == BoxKind::Site;
                        // Under a design from as close as its LUTs show.
                        if !site && cell < if dim { 30.0 } else { 60.0 } {
                            continue;
                        }
                        let rect = Rect::from_min_max(
                            screen([base[0] + b.min[0], base[1] + b.min[1]]),
                            screen([base[0] + b.max[0], base[1] + b.max[1]]),
                        );
                        // A site a few pixels across is only its fill: an
                        // outline that small reads as a glyph.
                        let outlined = rect.width().min(rect.height()) >= 5.0;
                        if b.kind == BoxKind::Slice {
                            if outlined {
                                let outline = Stroke::new(1.0, slice);
                                painter.rect_stroke(rect, 1.0, outline, StrokeKind::Inside);
                            }
                        } else {
                            painter.rect_filled(rect, if outlined { 1.0 } else { 0.0 }, fill);
                            if outlined {
                                painter.rect_stroke(rect, 1.0, edge, StrokeKind::Inside);
                            }
                        }
                        let (at, align, size) = match b.kind {
                            BoxKind::Slice => (
                                rect.right_top() + Vec2::new(-3.0, 2.0),
                                egui::Align2::RIGHT_TOP,
                                font.size,
                            ),
                            BoxKind::Site => (
                                rect.right_center() - Vec2::new(4.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                (rect.height() * 0.5).min(12.0),
                            ),
                            _ => (rect.center(), egui::Align2::CENTER_CENTER, font.size),
                        };
                        let shown = if site {
                            cell >= 60.0 && size >= 6.0
                        } else {
                            cell >= 300.0 && rect.height() >= 8.0
                        };
                        if shown {
                            let font = egui::FontId::monospace(size);
                            painter.text(at, align, &b.label, font, text);
                        }
                    }
                }
            }
        }
        // The DSP blocks of the rows in sight: their outline and parts, then
        // the wires from the CIB into their ports and the fixed connections
        // between the parts' ports.
        let seen = |min: [f32; 2], max: [f32; 2]| {
            view.intersects(Rect::from_min_max(screen(min), screen(max)))
        };
        for m in f.macros.iter().filter(|m| m.row >= r0 && m.row < r1) {
            for b in &m.frames {
                let rect = Rect::from_min_max(screen(b.min), screen(b.max));
                if !view.intersects(rect) {
                    continue;
                }
                painter.rect_stroke(
                    rect,
                    2.0,
                    Stroke::new(1.5, Color32::from_gray(110)),
                    StrokeKind::Inside,
                );
                if cell >= 30.0 {
                    painter.text(
                        rect.left_bottom() + Vec2::new(4.0, -3.0),
                        egui::Align2::LEFT_BOTTOM,
                        &b.label,
                        egui::FontId::proportional((cell / 40.0).clamp(9.0, 14.0)),
                        Color32::from_gray(170),
                    );
                }
            }
            if cell >= sites_from {
                for b in &m.parts {
                    if !seen(b.min, b.max) {
                        continue;
                    }
                    let rect = Rect::from_min_max(screen(b.min), screen(b.max));
                    painter.rect_filled(rect, 1.0, fill);
                    painter.rect_stroke(rect, 1.0, edge, StrokeKind::Inside);
                    let size = (rect.height() * 0.3).min(12.0).min(rect.width() / 5.0);
                    if cell >= 60.0 && size >= 6.0 {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &b.label,
                            egui::FontId::monospace(size),
                            Color32::from_gray(170),
                        );
                    }
                }
            }
            // The blocks' wires show from further out than the tiles': the
            // rows are high and the channels spread.
            if self.wires && cell >= 40.0 {
                let bound = |l: &[[f32; 2]]| {
                    let (mut min, mut max) = (l[0], l[0]);
                    for p in l {
                        min = [min[0].min(p[0]), min[1].min(p[1])];
                        max = [max[0].max(p[0]), max[1].max(p[1])];
                    }
                    seen(min, max)
                };
                for w in m.wires.iter().filter(|w| bound(w)) {
                    let points: Vec<Pos2> = w.iter().map(|p| screen(*p)).collect();
                    painter.line(points, part_stroke(Part::Local, width, dim));
                }
                for w in m.inner.iter().filter(|w| bound(w)) {
                    let points: Vec<Pos2> = w.iter().map(|p| screen(*p)).collect();
                    painter.line(points, part_stroke(Part::Jumper, width, dim));
                }
                if dots {
                    for d in m.dots.iter().filter(|d| seen(**d, **d)) {
                        painter.circle_filled(screen(*d), radius, dot);
                    }
                }
            }
        }
        // Wires and arcs over the sites.
        self.frame += 1;
        if self.drawn.len() != f.links.len() {
            self.drawn = vec![0; f.links.len()];
        }
        if self.wires && tracks {
            // The fabric's wires as one mesh: tens of thousands of shapes,
            // each laid out on its own, cost more than the frame. A wire
            // thinner than a pixel is a pixel as faint as it is thin.
            let mut grey = Lines::new();
            let mut add = |a: Pos2, b: Pos2, stroke: Stroke| {
                let colour = if stroke.width < 1.0 {
                    stroke.color.gamma_multiply(stroke.width)
                } else {
                    stroke.color
                };
                grey.add(a, b, stroke.width.max(1.0), colour);
            };
            for r in r0..r1 {
                for c in c0..c1 {
                    let base = [c as f32 * TILE_W, f.row_top[r as usize]];
                    let at = |p: [f32; 2]| screen([base[0] + p[0], base[1] + p[1]]);
                    let t = f.template(r, c);
                    for &(a, b, part) in &t.segments {
                        if !matches!(part, Part::Track | Part::Clock) && !locals {
                            continue;
                        }
                        add(at(a), at(b), part_stroke(part, width, dim));
                    }
                    // Every link that passes this tile, once, wherever it
                    // starts.
                    if locals {
                        for &k in f.links_through(r, c) {
                            if self.drawn[k as usize] == self.frame {
                                continue;
                            }
                            self.drawn[k as usize] = self.frame;
                            let link = &f.links[k as usize];
                            for pair in link.points.windows(2) {
                                add(
                                    screen(pair[0]),
                                    screen(pair[1]),
                                    part_stroke(Part::Jumper, width, dim),
                                );
                            }
                        }
                    }
                }
            }
            grey.paint(&painter);
            // The dots of the arcs over the wires.
            if dots {
                for r in r0..r1 {
                    for c in c0..c1 {
                        let base = [c as f32 * TILE_W, f.row_top[r as usize]];
                        for &p in &f.template(r, c).dots {
                            painter.circle_filled(
                                screen([base[0] + p[0], base[1] + p[1]]),
                                radius,
                                dot,
                            );
                        }
                    }
                }
            }
        }
        if let Some((o, values)) = live {
            self.overlay(&painter, view, f, o, values, (r0, r1, c0, c1), &screen);
            if self.wires {
                self.ties(&painter, view, o, &screen);
            }
            if let Some(net) = self.shown_net().filter(|_| self.wires) {
                self.highlight(&painter, view, o, net, &screen);
            }
            if self.wires {
                for &net in self.kept() {
                    if Some(net) != self.shown_net() {
                        self.highlight(&painter, view, o, net, &screen);
                    }
                }
            }
            if let Some(path) = &self.critical {
                let focused = self.critical_focus.as_ref().map(|(_, number)| *number);
                draw_critical(&painter, view, f, o, path, &screen, focused);
            }
        }
        // The element pointed at, thin; the one in the inspector, bold.
        for (e, stroke) in [
            (&self.hover_element, Stroke::new(1.5, Color32::WHITE)),
            (&self.picked, Stroke::new(3.0, NET)),
        ] {
            if let Some(e) = e {
                let rect = Rect::from_min_max(screen(e.min), screen(e.max));
                painter.rect_stroke(rect, 1.0, stroke, StrokeKind::Outside);
            }
        }
        self.hovered = response.hover_pos().and_then(|p| {
            let g = (p - origin) / cell;
            (g.x >= 0.0 && g.y >= 0.0 && g.x < size.x && g.y < size.y)
                .then_some((f.row_at(g.y), (g.x / TILE_W) as u16))
        });
        for (at, c) in [
            (self.hovered, Color32::WHITE),
            (self.selected, Color32::YELLOW),
        ] {
            if let Some((r, col)) = at {
                let tile = Rect::from_min_size(
                    screen([col as f32 * TILE_W, f.row_top[r as usize]]),
                    Vec2::new(TILE_W, f.row_height(r)) * cell,
                );
                painter.rect_stroke(tile, 0.0, Stroke::new(1.5, c), StrokeKind::Outside);
            }
        }
        if cell >= 14.0 {
            let font = egui::FontId::monospace(9.0);
            for col in c0..c1 {
                let x = origin.x + (col as f32 + 0.5) * TILE_W * cell;
                painter.text(
                    Pos2::new(x, view.min.y + 1.0),
                    egui::Align2::CENTER_TOP,
                    format!("C{col}"),
                    font.clone(),
                    Color32::GRAY,
                );
            }
            for row in r0..r1 {
                let y = origin.y + (f.row_top[row as usize] + f.row_height(row) / 2.0) * cell;
                painter.text(
                    Pos2::new(view.min.x + 2.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("R{row}"),
                    font.clone(),
                    Color32::GRAY,
                );
            }
        }
        self.draw_ms = started.elapsed().as_secs_f64() * 1e3;
    }
}

fn source_menu_label(span: &Span) -> String {
    format!("Show in source ({})", span.label())
}

/// Explain the design-wide state before attributing a missing line to a net.
pub(crate) fn source_unavailable(index: Option<&Index>, project: bool) -> &'static str {
    match index {
        None => "design has no source index yet",
        Some(index) if index.locations == 0 => {
            if project {
                "no project source matched the build's src attributes"
            } else {
                "no source files found for this design (looked beside its top.config)"
            }
        }
        Some(_) => "no source line for this net (synthesis left it only a library location)",
    }
}

pub(crate) fn source_menu(
    ui: &mut egui::Ui,
    locations: &[SourceLocation],
    unavailable: Option<&str>,
) -> Option<Span> {
    let mut chosen = None;
    if let Some(location) = locations.first() {
        if ui.button(source_menu_label(&location.span)).clicked() {
            chosen = Some(location.span.clone());
        }
    }
    if locations.is_empty() {
        if let Some(reason) = unavailable {
            ui.add_enabled(false, egui::Button::new("Show in source"))
                .on_disabled_hover_text(reason);
            ui.add(egui::Label::new(egui::RichText::new(reason).small().weak()).wrap());
        }
    }
    if locations.len() > 1 {
        ui.menu_button("Source locations", |ui| {
            for location in locations {
                if ui.button(location.label()).clicked() {
                    chosen = Some(location.span.clone());
                }
            }
        });
    }
    chosen
}

/// Several expressions on one line share one menu entry and record label.
fn source_choices(spans: &[Span]) -> Vec<SourceLocation> {
    let mut seen = HashSet::new();
    spans
        .iter()
        .filter(|s| seen.insert((s.file.clone(), s.line)))
        .map(|span| SourceLocation {
            span: span.clone(),
            kind: SourceKind::Cell,
        })
        .collect()
}

/// A design's sites, boxes and grid tiles as the GPU draws them from afar,
/// as the CPU would draw them (`FabricView::overlay`): where each position's
/// start among them (tiles, sites, boxes), and the largest of the sites' and
/// of the boxes' smaller sides.
fn spots_of(f: &Fabric, o: &Overlay) -> (Spots, [Vec<u32>; 3], (f32, f32)) {
    let positions = f.rows as usize * f.cols as usize;
    let pack = |c: Color32| u32::from_le_bytes(c.to_array());
    let side = |min: [f32; 2], max: [f32; 2]| (max[0] - min[0]).min(max[1] - min[1]);
    let mut spots = Vec::new();
    let mut wires: Vec<u32> = Vec::new();
    let mut starts: [Vec<u32>; 3] = Default::default();
    let mut largest = (0.0f32, 0.0f32);
    for (kind, starts) in starts.iter_mut().enumerate() {
        starts.reserve(positions + 1);
        for p in 0..positions {
            starts.push(spots.len() as u32);
            let (sites, boxes) = (o.sites_in(p), o.boxes_in(p));
            match kind {
                0 => {
                    if sites.is_empty() && boxes.is_empty() {
                        continue;
                    }
                    let first = wires.len() as u32;
                    for s in sites {
                        wires.push(s.value);
                        wires.extend(&s.also);
                    }
                    for b in boxes {
                        wires.extend(&b.values);
                    }
                    let (r, c) = ((p / f.cols as usize) as u16, (p % f.cols as usize) as f32);
                    let min = [c * TILE_W, f.row_top[r as usize]];
                    let share = ((sites.len() + boxes.len()) as f32 / 16.0).min(1.0);
                    spots.push(Spot {
                        min,
                        max: [min[0] + TILE_W, min[1] + f.row_height(r)],
                        base: pack(SITE.gamma_multiply(0.35 + 0.65 * share)),
                        first,
                        count: wires.len() as u32 - first,
                        kind: gpu_map::SPOT_TILE,
                    });
                }
                1 => {
                    for s in sites {
                        let first = wires.len() as u32;
                        wires.push(s.value);
                        wires.extend(&s.also);
                        largest.0 = largest.0.max(side(s.min, s.max));
                        spots.push(Spot {
                            min: s.min,
                            max: s.max,
                            base: pack(SITE),
                            first,
                            count: wires.len() as u32 - first,
                            kind: gpu_map::SPOT_SITE,
                        });
                    }
                }
                _ => {
                    for b in boxes {
                        let first = wires.len() as u32;
                        wires.extend(&b.values);
                        largest.1 = largest.1.max(side(b.min, b.max));
                        spots.push(Spot {
                            min: b.min,
                            max: b.max,
                            base: pack(SITE),
                            first,
                            count: wires.len() as u32 - first,
                            kind: gpu_map::SPOT_BOX,
                        });
                    }
                }
            }
        }
        starts.push(spots.len() as u32);
    }
    (Spots { spots, wires }, starts, largest)
}

/// Where line segments go: quads of a mesh, or pixels of an image.
trait Sink {
    fn add(&mut self, a: Pos2, b: Pos2, width: f32, colour: Color32);
}

impl Sink for Lines {
    fn add(&mut self, a: Pos2, b: Pos2, width: f32, colour: Color32) {
        Lines::add(self, a, b, width, colour);
    }
}

/// From how many pieces in sight the design's wires are drawn as an image:
/// a large design from afar has hundreds of thousands, most a pixel or two
/// long, and laying them out as quads takes longer than drawing the pixels.
const RASTER_FROM: usize = 40_000;

/// The GPU's share of the map: the pieces of the design's wires (once a
/// design), the wires' changes as the GPU has them and how often they have
/// changed, and what it drew, read back for a record.
struct GpuView {
    design: usize,
    segments: Option<Arc<Vec<Segment>>>,
    /// Where each tile's pieces start among them (row major, and the end).
    starts: Vec<u32>,
    /// The design's sites, boxes and grid tiles as the GPU draws them from
    /// afar (`gpu_map::Spot`), the design they are of, where each
    /// position's start among them (tiles, sites, boxes: row major, and the
    /// end), and the largest of the sites' and of the boxes' smaller sides
    /// (die units): from afar the CPU need not look at them at all.
    spots_design: usize,
    spots: Option<Arc<Spots>>,
    spot_starts: [Vec<u32>; 3],
    largest: (f32, f32),
    wires: Arc<Mutex<WireStates>>,
    readback: Arc<Mutex<Option<Readback>>>,
    measurements: Arc<Mutex<gpu_map::Measurements>>,
    /// Frames of the map egui dropped (prepared, not drawn).
    dropped: Arc<std::sync::atomic::AtomicU32>,
}

/// The design's wires in sight for one view: the pieces (by index into the
/// overlay's) with their ends on the screen. Found again only when the view
/// moves, so that a frame of a run only draws what changed.
#[derive(Default)]
struct InSight {
    /// The view, the zoom, the pan and the design.
    key: Option<(Rect, f32, Vec2, usize)>,
    /// A list a band of tile rows.
    pieces: Vec<Vec<Seen>>,
}

/// A piece of a wire in sight: its index, its wire's (the value's) and its
/// ends on the screen.
#[derive(Clone, Copy)]
struct Seen {
    i: u32,
    v: u32,
    a: Pos2,
    b: Pos2,
}

/// The design's wires drawn as an image of the view, a pixel to a point.
#[derive(Default)]
struct WireImage {
    texture: Option<egui::TextureHandle>,
    /// The wires, without a change over them, and the image's size.
    base: Vec<Color32>,
    size: [usize; 2],
    /// What `base` shows: whether 0/1, and the values' revision; none when
    /// it is to be drawn again.
    state: Option<(bool, u64)>,
    /// Whether the texture shows `base` alone, no change lit over it.
    plain: bool,
}

/// Line segments into the pixels of an image whose top left is `origin`.
struct Raster<'a> {
    pixels: &'a mut [Color32],
    size: [usize; 2],
    origin: Pos2,
}

impl Sink for Raster<'_> {
    fn add(&mut self, a: Pos2, b: Pos2, width: f32, colour: Color32) {
        let [sw, sh] = self.size;
        let (w, h) = (sw as f32, sh as f32);
        let (mut a, mut b) = (a - self.origin.to_vec2(), b - self.origin.to_vec2());
        // A wide line a pixel or two more across its run.
        let thick: i64 = if width >= 2.5 {
            3
        } else if width >= 1.8 {
            2
        } else {
            1
        };
        let mut set = |x: f32, y: f32| {
            if x >= 0.0 && y >= 0.0 && (x as usize) < sw && (y as usize) < sh {
                self.pixels[y as usize * sw + x as usize] = colour;
            }
        };
        // Most pieces from afar: a pixel.
        if thick == 1 && (a.x - b.x).abs() < 1.0 && (a.y - b.y).abs() < 1.0 {
            set(a.x.floor(), a.y.floor());
            return;
        }
        // Clipped to the image (Liang-Barsky), where it crosses an edge.
        let inside = |p: Pos2| p.x >= 0.0 && p.y >= 0.0 && p.x < w && p.y < h;
        if !(inside(a) && inside(b)) {
            let d = b - a;
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            for (p, q) in [
                (-d.x, a.x + 2.0),
                (d.x, w + 2.0 - a.x),
                (-d.y, a.y + 2.0),
                (d.y, h + 2.0 - a.y),
            ] {
                if p == 0.0 {
                    if q < 0.0 {
                        return;
                    }
                } else if p < 0.0 {
                    t0 = t0.max(q / p);
                } else {
                    t1 = t1.min(q / p);
                }
            }
            if t0 > t1 {
                return;
            }
            (a, b) = (a + d * t0, a + d * t1);
        }
        let d = b - a;
        let n = d.x.abs().max(d.y.abs()).ceil().max(1.0);
        let step = d / n;
        let steep = d.y.abs() > d.x.abs();
        let mut p = a;
        for _ in 0..=n as usize {
            let (x, y) = (p.x.floor(), p.y.floor());
            for k in 0..thick {
                let off = (k - (thick - 1) / 2) as f32;
                if steep {
                    set(x + off, y);
                } else {
                    set(x, y + off);
                }
            }
            p += step;
        }
    }
}

/// The threads that find and draw a large design's wires.
fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(4)
}

/// Draws `pieces` into the pixels of an image `size` whose top left is
/// `origin`, a thread a band of rows, each band taking the pieces that
/// reach into it. Whether `draw` drew any.
fn bands(
    pixels: &mut [Color32],
    size: [usize; 2],
    origin: Pos2,
    pieces: &[Vec<Seen>],
    draw: &(dyn Fn(&mut dyn Sink, &Seen) -> bool + Sync),
) -> bool {
    let [w, h] = size;
    let n = threads().min(h).max(1);
    let mut rest = pixels;
    let mut parts = Vec::new();
    for k in 0..n {
        let (y0, y1) = (h * k / n, h * (k + 1) / n);
        let (band, tail) = rest.split_at_mut((y1 - y0) * w);
        parts.push((y0, y1, band));
        rest = tail;
    }
    std::thread::scope(|scope| {
        let jobs: Vec<_> = parts
            .into_iter()
            .map(|(y0, y1, band)| {
                scope.spawn(move || {
                    let mut r = Raster {
                        pixels: band,
                        size: [w, y1 - y0],
                        origin: origin + Vec2::new(0.0, y0 as f32),
                    };
                    let (top, bottom) = (origin.y + y0 as f32 - 3.0, origin.y + y1 as f32 + 3.0);
                    let mut drew = false;
                    for q in pieces.iter().flatten() {
                        if q.a.y.max(q.b.y) >= top && q.a.y.min(q.b.y) < bottom {
                            drew |= draw(&mut r, q);
                        }
                    }
                    drew
                })
            })
            .collect();
        jobs.into_iter().fold(false, |d, j| j.join().unwrap() | d)
    })
}

/// How the pieces of the wires are drawn this frame: the changes and the
/// moment, each net's length, whether by value, and the width.
struct Paint<'a> {
    o: &'a Overlay,
    changes: &'a Changes,
    lengths: &'a [f32],
    now: f64,
    values: bool,
    width: f32,
    /// Where what is drawn is recorded, when it is.
    drawn: Option<&'a Drawn>,
}

impl Paint<'_> {
    /// Where a point `d` from its net's driver is: the share of the net's
    /// length (0 for a net with no length, or no driver drawn).
    fn share(&self, v: u32, d: f32) -> f32 {
        let length = self.lengths[self.o.nets[v as usize] as usize];
        if d.is_finite() && length > 0.0 {
            (d / length).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// A wire's colour at a share of its net: what the design uses, or with
    /// 0/1 the value there.
    fn colour(&self, v: u32, u: f32) -> Color32 {
        if self.values {
            colour(Some(&self.changes.value(v, u, self.now)))
        } else {
            USED
        }
    }

    /// As `colour`, for the wires' own colours: with 0/1 the value drawn is
    /// recorded.
    fn base(&self, v: u32, u: f32) -> Color32 {
        if !self.values {
            return USED;
        }
        let x = self.changes.value(v, u, self.now);
        if let Some(d) = self.drawn {
            d.value(v, x);
        }
        colour(Some(&x))
    }

    /// A piece in steps along it, the near end (to the driver) first, each
    /// with its middle's share of the net: no more than a step a pixel, and
    /// sixteen at most.
    fn steps(&self, q: &Seen) -> impl Iterator<Item = (Pos2, Pos2, f32)> {
        let p = &self.o.pieces[q.i as usize];
        let (u0, u1) = (self.share(q.v, p.d0), self.share(q.v, p.d1));
        let n = (q.b - q.a).length().ceil().clamp(1.0, 16.0) as usize;
        let (a, b) = (q.a, q.b);
        (0..n).map(move |k| {
            let (s0, s1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
            (
                a + (b - a) * s0,
                a + (b - a) * s1,
                u0 + (u1 - u0) * (s0 + s1) / 2.0,
            )
        })
    }

    /// A piece of a wire; with 0/1 in the colours of the values along it,
    /// where a change has not reached everywhere yet.
    fn wire(&self, sink: &mut dyn Sink, q: &Seen) {
        if !self.values || !self.changes.lit(q.v, self.now) {
            sink.add(q.a, q.b, self.width, self.base(q.v, 0.0));
            return;
        }
        for (a, b, u) in self.steps(q) {
            sink.add(a, b, self.width, self.base(q.v, u));
        }
    }

    /// The changes over a piece of a wire, orange, brighter and wider the
    /// more they glow; whether any lit it.
    fn change(&self, sink: &mut dyn Sink, q: &Seen) -> bool {
        if !self.changes.lit(q.v, self.now) {
            return false;
        }
        let mut lit = false;
        for (a, b, u) in self.steps(q) {
            let k = self.changes.glow(q.v, u, self.now);
            if k > 0.0 {
                sink.add(
                    a,
                    b,
                    self.width * (1.0 + 1.2 * k),
                    self.colour(q.v, u).lerp_to_gamma(CHANGE, k),
                );
                lit = true;
            }
        }
        if let (true, Some(d)) = (lit, self.drawn) {
            d.lit(q.v);
        }
        lit
    }
}

/// Line segments drawn as one mesh of quads, with square ends that close
/// the corners where two meet: a shape each would cost far more when a
/// large design has hundreds of thousands in sight.
struct Lines(egui::Mesh);

impl Lines {
    fn new() -> Self {
        Lines(egui::Mesh::default())
    }

    fn add(&mut self, a: Pos2, b: Pos2, width: f32, colour: Color32) {
        let d = b - a;
        let length = d.length();
        let along = if length > 1e-3 { d / length } else { Vec2::X };
        let (e, n) = (along * (width / 2.0), along.rot90() * (width / 2.0));
        let i = self.0.vertices.len() as u32;
        for p in [a - e + n, a - e - n, b + e - n, b + e + n] {
            self.0.colored_vertex(p, colour);
        }
        self.0.add_triangle(i, i + 1, i + 2);
        self.0.add_triangle(i, i + 2, i + 3);
    }

    fn paint(self, painter: &egui::Painter) {
        if !self.0.is_empty() {
            painter.add(egui::Shape::mesh(self.0));
        }
    }
}

impl FabricView {
    /// The design's wires in use over the fabric, in the tiles in sight:
    /// LUTs and flip-flops, then the wires, then the dots of the arcs, each
    /// lit by the changes where they have spread (src/changes.rs). With
    /// `values`, the wires have the colour of their value, the old one where
    /// a change has not reached yet.
    #[allow(clippy::too_many_arguments)]
    fn overlay(
        &self,
        painter: &egui::Painter,
        view: Rect,
        f: &Fabric,
        o: &Overlay,
        values: &[u8],
        (r0, r1, c0, c1): (u16, u16, u16, u16),
        screen: &dyn Fn([f32; 2]) -> Pos2,
    ) {
        let cell = self.cell;
        let changes = &self.changes;
        let now = changes.now();
        let frame = self.draws.get() + 1;
        self.draws.set(frame);
        let mut resets = Vec::new();
        if let Some(t) = self.trace.borrow_mut().as_mut() {
            t.drawing(frame, now);
            resets = t.take_resets();
            // What the GPU drew, read back a few frames late.
            if let Some(read) = self
                .gpu
                .borrow()
                .as_ref()
                .and_then(|g| g.readback.lock().unwrap().take())
            {
                t.drawn.load(read);
            }
        }
        let position = |r: u16, c: u16| r as usize * f.cols as usize + c as usize;
        // Each net's length, once a design.
        // (With this view's number: a design opened after another may be
        // given the same memory, and the GPU must not keep the old pieces.)
        let design = (o as *const Overlay as usize) ^ (self.generation << 48);
        if self.lengths.borrow().0 != design {
            let mut lengths = vec![0.0f32; o.values.len()];
            for p in o.pieces.iter().filter(|p| p.route) {
                let n = o.nets[p.value as usize] as usize;
                for d in [p.d0, p.d1].into_iter().filter(|d| d.is_finite()) {
                    lengths[n] = lengths[n].max(d);
                }
            }
            *self.lengths.borrow_mut() = (design, lengths);
        }
        let lengths = self.lengths.borrow();
        // Every wire in use in sight, found again only when the view moves:
        // a band of tile rows a thread.
        // With the GPU drawing them, only for a record (what is in sight).
        let gpu = self.gpu.borrow().is_some();
        let tracing = self.trace.borrow().is_some();
        let mut sight = self.sight.borrow_mut();
        let key = (view, cell, self.offset, design);
        if (!gpu || tracing) && sight.key != Some(key) {
            let origin = screen([0.0, 0.0]);
            let at = |p: [f32; 2]| origin + Vec2::new(p[0], p[1]) * cell;
            let rows = (r1 - r0) as usize;
            let n = rows.min(threads()).max(1);
            let s = &mut *sight;
            // The lists of the last view, kept for their memory.
            s.pieces.resize_with(n, Vec::new);
            std::thread::scope(|scope| {
                for (k, pieces) in s.pieces.iter_mut().enumerate() {
                    let (ra, rb) = (r0 as usize + rows * k / n, r0 as usize + rows * (k + 1) / n);
                    scope.spawn(move || {
                        pieces.clear();
                        for r in ra..rb {
                            for c in c0 as usize..c1 as usize {
                                // Only the way from a driver to what it
                                // feeds: the rest of a conductor stays grey,
                                // as the empty fabric draws it.
                                for i in o.piece_range(r * f.cols as usize + c) {
                                    let p = &o.pieces[i];
                                    if !p.route {
                                        continue;
                                    }
                                    let (a, b) = (at(p.a), at(p.b));
                                    if view.intersects(Rect::from_two_pos(a, b).expand(2.0)) {
                                        pieces.push(Seen {
                                            i: i as u32,
                                            v: p.value,
                                            a,
                                            b,
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
            });
            s.key = Some(key);
            self.image.borrow_mut().state = None;
            // For the record, in sight is on the map: the pieces kept just
            // past its edges (for their width) cannot be seen.
            if let Some(t) = self.trace.borrow_mut().as_mut() {
                // With where across the map each is, and the share of its
                // net's length a change reaches it at.
                let share = |q: &Seen| {
                    let p = &o.pieces[q.i as usize];
                    let length = lengths.1[o.nets[q.v as usize] as usize];
                    let at = |d: f32| {
                        if d.is_finite() && length > 0.0 {
                            (d / length).clamp(0.0, 1.0)
                        } else {
                            0.0
                        }
                    };
                    at(p.d0).min(at(p.d1))
                };
                t.drawn.sight(
                    s.pieces
                        .iter()
                        .flatten()
                        .filter(|q| view.intersects(Rect::from_two_pos(q.a, q.b)))
                        .map(|q| {
                            (
                                q.v,
                                ((q.a.x + q.b.x) / 2.0 - view.min.x) / view.width(),
                                share(q),
                            )
                        }),
                );
            }
        }
        if self.why.take() {
            if let Some(t) = self.trace.borrow_mut().as_mut() {
                let pieces: Vec<&Seen> = sight.pieces.iter().flatten().collect();
                let _ = t.note(&format!(
                    "why: the map is {view:?}, {cell} points a die unit"
                ));
                for (v, times) in t.most_missed(40) {
                    let r = changes.records()[v as usize];
                    let net = lengths.1[o.nets[v as usize] as usize];
                    let mine: Vec<&&Seen> = pieces.iter().filter(|q| q.v == v).collect();
                    let longest = mine
                        .iter()
                        .map(|q| (q.b - q.a).length())
                        .fold(0.0, f32::max);
                    let _ = t.note(&format!(
                        "why wire {v}: missed {times}, net length {net:.3}, changes {:?}, values {:08x}, now {now:.3}; {} pieces in sight, longest {longest:.2} points",
                        r.changes,
                        r.values,
                        mine.len()
                    ));
                    for q in mine.iter().take(6) {
                        let p = &o.pieces[q.i as usize];
                        let _ = t.note(&format!(
                            "why   piece {}: ({:.1}, {:.1}) to ({:.1}, {:.1}), {:.2} points, from the driver {:.3} to {:.3} of the net, {:?}",
                            q.i,
                            q.a.x,
                            q.a.y,
                            q.b.x,
                            q.b.y,
                            (q.b - q.a).length(),
                            p.d0,
                            p.d1,
                            p.kind
                        ));
                    }
                }
            }
        }
        let trace = self.trace.borrow();
        let drawn = trace.as_ref().map(|t| &t.drawn);
        // A site glows with a change of its output, at the driver.
        let glow = |v: u32| changes.glow(v, 0.0, now);
        // With the GPU, the sites, boxes and grid tiles a few pixels across
        // are drawn by it from the wires' records, under the rest; the CPU
        // draws those closer, with their outlines and labels.
        let mut far = (false, false);
        if gpu {
            let mut g = self.gpu.borrow_mut();
            let g = g.as_mut().expect("the GPU's share");
            if g.spots_design != design || g.spots.is_none() {
                let (spots, starts, largest) = spots_of(f, o);
                g.spots = Some(Arc::new(spots));
                g.spot_starts = starts;
                g.largest = largest;
                g.spots_design = design;
            }
            far = (g.largest.0 * cell < 4.0, g.largest.1 * cell < 4.0);
            // The grid's tiles, then the sites, then the boxes, as the CPU
            // draws them; a run a row of tiles in sight.
            let kinds: &[usize] = if !self.wires && cell < 12.0 {
                &[0, 1, 2]
            } else {
                &[1, 2]
            };
            let mut ranges = Vec::new();
            for &k in kinds {
                let starts = &g.spot_starts[k];
                for r in r0..r1 {
                    let at = |c: u16| starts[r as usize * f.cols as usize + c as usize];
                    if at(c0) < at(c1) {
                        ranges.push(at(c0)..at(c1));
                    }
                }
            }
            painter.add(egui_wgpu::Callback::new_paint_callback(
                view,
                SpotsCallback {
                    design,
                    spots: g.spots.clone(),
                    ranges,
                    wires: Arc::clone(&g.wires),
                    origin: self.offset,
                    size: view.size(),
                    cell,
                    now: now as f32,
                    values: self.values,
                    frame,
                    measure: tracing.then(|| Arc::clone(&g.measurements)),
                    last: !self.wires,
                },
            ));
        }
        // The grid from afar: a tile the design uses filled the more the more
        // of its sites it uses, flashing with their changes.
        if !gpu && !self.wires && cell < 12.0 {
            for r in r0..r1 {
                for c in c0..c1 {
                    let p = position(r, c);
                    let (sites, boxes) = (o.sites_in(p), o.boxes_in(p));
                    if sites.is_empty() && boxes.is_empty() {
                        continue;
                    }
                    let k = sites
                        .iter()
                        .map(|s| s.also.iter().fold(glow(s.value), |k, &v| k.max(glow(v))))
                        .chain(boxes.iter().flat_map(|b| b.values.iter().map(|&v| glow(v))))
                        .fold(0.0, f32::max);
                    let share = ((sites.len() + boxes.len()) as f32 / 16.0).min(1.0);
                    let tile = Rect::from_min_size(
                        screen([c as f32 * TILE_W, f.row_top[r as usize]]),
                        Vec2::new(TILE_W, f.row_height(r)) * cell,
                    );
                    let fill = SITE
                        .gamma_multiply(0.35 + 0.65 * share)
                        .lerp_to_gamma(CHANGE, k);
                    painter.rect_filled(
                        tile.shrink(if cell >= 3.0 { 0.5 } else { 0.0 }),
                        0.0,
                        fill,
                    );
                }
            }
        }
        // Sites a few pixels across, filled only, as one mesh.
        let mut small = egui::Mesh::default();
        {
            let font = egui::FontId::monospace((cell / 110.0).clamp(6.0, 12.0));
            for r in (r0..r1).filter(|_| !far.0) {
                for c in c0..c1 {
                    for s in o.sites_in(position(r, c)) {
                        if gpu && gpu_map::spot_small(s.min, s.max, cell) {
                            continue;
                        }
                        let rect = Rect::from_min_max(screen(s.min), screen(s.max));
                        // With 0/1 the value of its output, where a route
                        // takes it; and a change flashes, either way.
                        let (fill, text) = match (self.values, values.get(s.value as usize)) {
                            (true, Some(0)) => (ZERO, SITE_TEXT_LIGHT),
                            (true, Some(1)) => (ONE, SITE_TEXT),
                            (true, Some(_)) => (UNKNOWN, SITE_TEXT),
                            _ => (SITE, SITE_TEXT),
                        };
                        // A CCU2 half flashes with its sum or its carry, a RAMW half
                        // with what it passes on.
                        let fill = fill.lerp_to_gamma(
                            CHANGE,
                            s.also.iter().fold(glow(s.value), |k, &v| k.max(glow(v))),
                        );
                        let edge = SITE_EDGE;
                        if rect.width().min(rect.height()) < 4.0 {
                            small.add_colored_rect(rect, fill);
                            continue;
                        }
                        painter.rect_filled(rect, 1.0, fill);
                        if rect.width().min(rect.height()) >= 4.0 {
                            painter.rect_stroke(
                                rect,
                                1.0,
                                Stroke::new(1.5, edge),
                                StrokeKind::Inside,
                            );
                        }
                        if cell >= 300.0 && rect.height() >= 8.0 {
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                &s.label,
                                font.clone(),
                                text,
                            );
                        }
                    }
                }
            }
        }
        // The other sites in use: an EBR, a pad, a part of a DSP block, a
        // slice's wide muxes or controls.
        for r in (r0..r1).filter(|_| !far.1) {
            for c in c0..c1 {
                for b in o.boxes_in(position(r, c)) {
                    if gpu && gpu_map::spot_small(b.min, b.max, cell) {
                        continue;
                    }
                    let rect = Rect::from_min_max(screen(b.min), screen(b.max));
                    if !view.intersects(rect) {
                        continue;
                    }
                    let k = b.values.iter().map(|&v| glow(v)).fold(0.0, f32::max);
                    if rect.width().min(rect.height()) < 4.0 {
                        small.add_colored_rect(rect, SITE.lerp_to_gamma(CHANGE, k));
                        continue;
                    }
                    painter.rect_filled(rect, 1.0, SITE.lerp_to_gamma(CHANGE, k));
                    if rect.width().min(rect.height()) >= 4.0 {
                        painter.rect_stroke(
                            rect,
                            1.0,
                            Stroke::new(1.5, SITE_EDGE),
                            StrokeKind::Inside,
                        );
                    }
                    let (at, align, size) = if b.site {
                        (
                            rect.right_center() - Vec2::new(4.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            (rect.height() * 0.5).min(12.0),
                        )
                    } else {
                        (
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            (cell / 110.0).clamp(6.0, 12.0),
                        )
                    };
                    let shown = if b.site {
                        cell >= 60.0 && size >= 6.0
                    } else {
                        cell >= 300.0 && rect.height() >= 8.0
                    };
                    if shown {
                        painter.text(
                            at,
                            align,
                            &b.label,
                            egui::FontId::monospace(size),
                            SITE_TEXT,
                        );
                    }
                }
            }
        }
        if !small.is_empty() {
            painter.add(egui::Shape::mesh(small));
        }
        // Without the wires, that is all.
        if !self.wires {
            return;
        }
        let width = (cell * 0.0013).clamp(1.3, 3.0);
        let paint = Paint {
            o,
            changes,
            lengths: &lengths.1,
            now,
            values: self.values,
            width,
            drawn,
        };
        let any = changes.animating();
        let count: usize = sight.pieces.iter().map(Vec::len).sum();
        if gpu {
            let mut g = self.gpu.borrow_mut();
            let g = g.as_mut().expect("the GPU's share");
            if g.design != design || g.segments.is_none() {
                let mut segments: Vec<Segment> = Vec::new();
                let mut starts = Vec::new();
                for position in 0..f.rows as usize * f.cols as usize {
                    starts.push(segments.len() as u32);
                    for p in o.pieces_in(position).iter().filter(|p| p.route) {
                        let length = lengths.1[o.nets[p.value as usize] as usize];
                        let share = |d: f32| {
                            if d.is_finite() && length > 0.0 {
                                (d / length).clamp(0.0, 1.0)
                            } else {
                                0.0
                            }
                        };
                        segments.push(Segment {
                            a: p.a,
                            b: p.b,
                            share: [share(p.d0), share(p.d1)],
                            wire: p.value,
                        });
                    }
                }
                starts.push(segments.len() as u32);
                if tracing {
                    eprintln!(
                        "map: {} pieces of wire, {} MB, {} wires",
                        segments.len(),
                        (segments.len() * std::mem::size_of::<Segment>()) >> 20,
                        o.values.len()
                    );
                }
                g.segments = Some(Arc::new(segments));
                g.starts = starts;
                g.design = design;
            }
            // The tiles in sight, and a few more each way for the long
            // tracks that start beyond it: a run a row.
            const BEYOND: u16 = 6;
            let (ra, rb) = (r0.saturating_sub(BEYOND), (r1 + BEYOND).min(f.rows));
            let (ca, cb) = (c0.saturating_sub(BEYOND), (c1 + BEYOND).min(f.cols));
            let ranges: Vec<std::ops::Range<u32>> = (ra..rb)
                .map(|r| {
                    let at = |c: u16| g.starts[r as usize * f.cols as usize + c as usize];
                    at(ca)..at(cb)
                })
                .filter(|range| !range.is_empty())
                .collect();
            painter.add(egui_wgpu::Callback::new_paint_callback(
                view,
                MapCallback {
                    design,
                    segments: g.segments.clone(),
                    ranges,
                    rect: view,
                    wires: Arc::clone(&g.wires),
                    origin: self.offset,
                    size: view.size(),
                    cell,
                    width,
                    now: now as f32,
                    values: self.values,
                    frame,
                    resets,
                    glow: self.glow,
                    record: tracing.then(|| Arc::clone(&g.readback)),
                    measure: tracing.then(|| Arc::clone(&g.measurements)),
                    dropped: Arc::clone(&g.dropped),
                },
            ));
        } else if count >= self.raster_from {
            // An image of the wires, drawn again when the view moves or the
            // colours do; the changes over a copy of it.
            let mut image = self.image.borrow_mut();
            let im = &mut *image;
            let size = [
                view.width().ceil().max(1.0) as usize,
                view.height().ceil().max(1.0) as usize,
            ];
            // Without 0/1 the wires are all one colour, whatever the values;
            // with it a change spreading moves the colours every frame.
            let state = (
                self.values,
                if self.values { changes.revision() } else { 0 },
            );
            let moving = self.values && changes.spreading(now);
            let fresh = im.size != size || im.state != Some(state) || moving;
            if fresh {
                if let Some(d) = drawn {
                    d.colours_again();
                }
                im.size = size;
                im.base.clear();
                im.base.resize(size[0] * size[1], Color32::TRANSPARENT);
                bands(&mut im.base, size, view.min, &sight.pieces, &|sink, q| {
                    paint.wire(sink, q);
                    true
                });
                im.state = (!moving).then_some(state);
            }
            let mut over = Vec::new();
            if any {
                over = im.base.clone();
                if !bands(&mut over, size, view.min, &sight.pieces, &|sink, q| {
                    paint.change(sink, q)
                }) {
                    over.clear();
                }
            }
            if fresh || !over.is_empty() || !im.plain {
                im.plain = over.is_empty();
                let pixels = if over.is_empty() {
                    im.base.clone()
                } else {
                    over
                };
                let image = egui::ColorImage::new(size, pixels);
                match &mut im.texture {
                    Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                    None => {
                        im.texture = Some(painter.ctx().load_texture(
                            "design wires",
                            image,
                            egui::TextureOptions::NEAREST,
                        ))
                    }
                }
            }
            if let Some(t) = &im.texture {
                painter.image(
                    t.id(),
                    Rect::from_min_size(view.min, Vec2::new(size[0] as f32, size[1] as f32)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            // Of the pieces shorter than a pixel, one a pixel: more would
            // only draw it over.
            let (pw, ph) = (view.width() as usize + 8, view.height() as usize + 8);
            let pixel = |a: Pos2, b: Pos2| -> Option<usize> {
                let (x, y) = (
                    (a.x - view.min.x + 4.0) as usize,
                    (a.y - view.min.y + 4.0) as usize,
                );
                ((a - b).length() < 1.0 && x < pw && y < ph).then_some(y * pw + x)
            };
            let mut speck = vec![false; pw * ph];
            let mut lines = Lines::new();
            if let Some(d) = drawn {
                d.colours_again();
            }
            for q in sight.pieces.iter().flatten() {
                if let Some(k) = pixel(q.a, q.b) {
                    if speck[k] {
                        continue;
                    }
                    speck[k] = true;
                }
                paint.wire(&mut lines, q);
            }
            lines.paint(painter);
            // The changes on every piece they light: of the short ones, one
            // a pixel of those lit, whichever drew the pixel above.
            speck.fill(false);
            let mut lines = Lines::new();
            for q in sight.pieces.iter().flatten().filter(|_| any) {
                let k = pixel(q.a, q.b);
                if k.is_some_and(|k| speck[k]) {
                    continue;
                }
                if paint.change(&mut lines, q) {
                    if let Some(k) = k {
                        speck[k] = true;
                    }
                }
            }
            lines.paint(painter);
        }
        // The CPU drew it all this frame: the record has it.
        if let (false, Some(d)) = (gpu, drawn) {
            d.drawn(now);
        }
        if cell >= 150.0 {
            let radius = (cell * 0.0013).clamp(1.6, 3.6);
            for r in r0..r1 {
                for c in c0..c1 {
                    for d in o.dots_in(position(r, c)).iter().filter(|d| d.route) {
                        let q = screen(d.at);
                        if !view.expand(radius).contains(q) {
                            continue;
                        }
                        let u = paint.share(d.value, d.d);
                        let colour = paint
                            .colour(d.value, u)
                            .lerp_to_gamma(CHANGE, changes.glow(d.value, u, now));
                        painter.circle_filled(q, radius, colour);
                    }
                }
            }
        }
    }
}

/// A net pointed at or kept, drawn over everything at any zoom.
pub const NET: Color32 = Color32::from_rgb(70, 225, 255);

impl FabricView {
    /// The net shown whole: the one pointed at, else the one kept.
    pub fn shown_net(&self) -> Option<u32> {
        self.hover_net.or(self.net)
    }

    /// How much a wire of the overlay glows for its changes at its driver,
    /// from 1 when it changes to 0, for the inspector's schematic.
    pub fn glow(&self, v: u32) -> f32 {
        self.changes.glow(v, 0.0, self.changes.now())
    }

    /// The net of the piece on the way from a driver nearest to a point of
    /// the die, a few pixels at most.
    fn pick(&self, f: &Fabric, o: &Overlay, p: [f32; 2]) -> Option<u32> {
        let size = f.size();
        if p[0] < 0.0 || p[1] < 0.0 || p[0] >= size[0] || p[1] >= size[1] {
            return None;
        }
        let tolerance = 6.0 / self.cell;
        let (r, c) = (f.row_at(p[1]) as i32, (p[0] / TILE_W) as i32);
        let mut best: Option<(f32, u32)> = None;
        for rr in (r - 1).max(0)..=(r + 1).min(f.rows as i32 - 1) {
            for cc in (c - 1).max(0)..=(c + 1).min(f.cols as i32 - 1) {
                let position = rr as usize * f.cols as usize + cc as usize;
                for q in o.pieces_in(position).iter().filter(|q| q.route) {
                    let d = to_segment(p, q.a, q.b);
                    if d <= tolerance && best.is_none_or(|b| d < b.0) {
                        best = Some((d, o.nets[q.value as usize]));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The smallest site, part of a slice or part of a DSP block at a point
    /// of the die, of those drawn at this zoom (`dim`: under a design).
    fn element_at(&self, f: &Fabric, p: [f32; 2], dim: bool) -> Option<Picked> {
        let size = f.size();
        if p[0] < 0.0 || p[1] < 0.0 || p[0] >= size[0] || p[1] >= size[1] {
            return None;
        }
        let cell = self.cell;
        if cell < if self.wires { 12.0 } else { 4.0 } {
            return None;
        }
        let inside = |min: [f32; 2], max: [f32; 2]| {
            (min[0]..=max[0]).contains(&p[0]) && (min[1]..=max[1]).contains(&p[1])
        };
        let area = |min: [f32; 2], max: [f32; 2]| (max[0] - min[0]) * (max[1] - min[1]);
        let (r, c) = (f.row_at(p[1]), (p[0] / TILE_W) as u16);
        let base = [c as f32 * TILE_W, f.row_top[r as usize]];
        let t = f.template(r, c);
        let mut best: Option<(f32, Picked)> = None;
        for b in &t.boxes {
            // The parts of slices show from as close as the drawing has them.
            if b.kind != BoxKind::Site && cell < if dim { 30.0 } else { 60.0 } {
                continue;
            }
            let (min, max) = (
                [base[0] + b.min[0], base[1] + b.min[1]],
                [base[0] + b.max[0], base[1] + b.max[1]],
            );
            if inside(min, max) && best.as_ref().is_none_or(|(a, _)| area(min, max) < *a) {
                let picked = Picked {
                    at: (r, c),
                    kind: b.kind,
                    label: b.label.clone(),
                    min,
                    max,
                    pins: pins_of(t, b),
                    dsp: false,
                };
                best = Some((area(min, max), picked));
            }
        }
        for m in &f.macros {
            for b in m.parts.iter().filter(|b| inside(b.min, b.max)) {
                if best.as_ref().is_none_or(|(a, _)| area(b.min, b.max) < *a) {
                    let col = ((b.min[0] + b.max[0]) / 2.0 / TILE_W) as u16;
                    let picked = Picked {
                        at: (m.row, col),
                        kind: b.kind,
                        label: b.label.clone(),
                        min: b.min,
                        max: b.max,
                        pins: Vec::new(),
                        dsp: true,
                    };
                    best = Some((area(b.min, b.max), picked));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// A whole net over the die: its way from the drivers to what it
    /// feeds, a ring where each driver leaves its site and a dot where the
    /// net enters each site it feeds.
    fn highlight(
        &self,
        painter: &egui::Painter,
        view: Rect,
        o: &Overlay,
        net: u32,
        screen: &dyn Fn([f32; 2]) -> Pos2,
    ) {
        let width = (self.cell * 0.0013).clamp(1.3, 3.0) * 2.0;
        for &k in o.pieces_of(net) {
            let p = &o.pieces[k as usize];
            if !p.route {
                continue;
            }
            let (a, b) = (screen(p.a), screen(p.b));
            if view.intersects(Rect::from_two_pos(a, b).expand(4.0)) {
                painter.line_segment([a, b], Stroke::new(width, NET));
            }
        }
        for &p in o.fed_of(net) {
            painter.circle_filled(screen(p), (width * 1.6).max(3.5), NET);
        }
        // A net a CIB ties to a constant starts at no site: a square where
        // its wire leaves the CIB.
        let tied = o.tied(net).is_some();
        for &p in o.drivers_of(net) {
            let q = screen(p);
            if tied {
                let side = (width * 3.0).max(7.0);
                let rect = Rect::from_center_size(q, Vec2::splat(side * 2.0));
                painter.rect_stroke(rect, 0.0, Stroke::new(2.0, NET), StrokeKind::Middle);
                painter.rect_filled(rect.shrink(side * 0.55), 0.0, Color32::WHITE);
            } else {
                painter.circle_stroke(q, (width * 3.0).max(7.0), Stroke::new(2.0, NET));
                painter.circle_filled(q, (width * 1.6).max(3.5), Color32::WHITE);
            }
        }
    }
}

const CRITICAL_STROKE: f32 = 2.5;
const CRITICAL_ALL_BADGES_ZOOM: f32 = 12.0;

/// Keep screen decorations sparse until there is room between their anchors.
/// At medium zoom, increase the hop stride until the badges fit locally;
/// endpoints and the report's focused number always have priority.
fn critical_badge_subset(
    painter: &egui::Painter,
    view: Rect,
    die: Rect,
    mut labels: Vec<CriticalLabel>,
    zoom: f32,
    focused: Option<usize>,
    compact: bool,
) -> Vec<CriticalLabel> {
    if !view.is_positive() {
        return Vec::new();
    }
    let visible = |label: &CriticalLabel| label.outlines.iter().any(|r| view.intersects(*r));
    labels.retain(|label| visible(label) || focused.is_some_and(|n| label.numbers.contains(&n)));
    for label in &mut labels {
        if !visible(label) {
            label.numbers.retain(|n| Some(*n) == focused);
        }
    }
    if let Some(n) = focused {
        if let Some(i) = labels
            .iter()
            .position(|label| label.numbers.contains(&n) && label.numbers.len() > 1)
        {
            let outlines = labels[i].outlines.clone();
            labels[i].numbers.retain(|&number| number != n);
            labels.push(CriticalLabel {
                outlines,
                numbers: vec![n],
            });
        }
    }
    labels.sort_by_key(|label| {
        (
            !focused.is_some_and(|n| label.numbers.contains(&n)),
            label.numbers[0],
        )
    });
    if labels.is_empty() || zoom >= CRITICAL_ALL_BADGES_ZOOM {
        return labels;
    }
    let keep_focus = |mut labels: Vec<CriticalLabel>| {
        for label in &mut labels {
            label.numbers.retain(|n| Some(*n) == focused);
        }
        labels.retain(|label| !label.numbers.is_empty());
        labels
    };
    let positions: Vec<_> = labels
        .iter()
        .filter(|label| visible(label))
        .flat_map(|label| {
            std::iter::repeat_n(
                label.outlines[0].intersect(view).center(),
                label.numbers.len(),
            )
        })
        .collect();
    let spacing = if positions.len() < 2 {
        f32::INFINITY
    } else {
        positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                positions
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, q)| p.distance(*q))
                    .fold(f32::INFINITY, f32::min)
            })
            .sum::<f32>()
            / positions.len() as f32
    };
    let font = egui::FontId::monospace(11.0);
    let padding = Vec2::splat(2.0).min(view.size() * 0.1);
    let colour =
        crate::theme::colours(painter.ctx().global_style().visuals.dark_mode).critical_text;
    let first = labels
        .iter()
        .filter(|label| visible(label))
        .flat_map(|label| &label.numbers)
        .min()
        .copied();
    let last = labels
        .iter()
        .filter(|label| visible(label))
        .flat_map(|label| &label.numbers)
        .max()
        .copied();
    let Some(last) = last else {
        return keep_focus(labels);
    };
    let width = painter
        .layout_no_wrap(last.to_string(), font.clone(), colour)
        .size()
        .x
        + padding.x;
    if zoom < 4.0 && spacing < 1.5 * width {
        return keep_focus(labels);
    }
    let max_stride = last.max(2);
    let stride = ((1.5 * width / spacing).ceil() as usize).clamp(2, max_stride);
    for step in stride..=max_stride {
        let mut selected = labels.clone();
        for label in &mut selected {
            label.numbers.retain(|&n| {
                Some(n) == focused || Some(n) == first || n == last || (n - 1) % step == 0
            });
        }
        selected.retain(|label| !label.numbers.is_empty());
        let sizes: Vec<_> = selected
            .iter()
            .map(|label| {
                painter
                    .layout(
                        critical_text(&label.numbers, compact),
                        font.clone(),
                        colour,
                        (view.width() - padding.x).max(1.0),
                    )
                    .size()
                    + padding
            })
            .collect();
        let mut placed = Vec::new();
        let local = selected.iter().zip(&sizes).all(|(label, &size)| {
            if let Some(rect) = critical_badge(view, label.outlines[0], size, &placed, true) {
                placed.push(rect);
                true
            } else {
                false
            }
        });
        // At fit, even a sparse path must leave almost all of the die clear.
        let within_budget = zoom >= 4.0
            || sizes.iter().map(|s| s.x * s.y).sum::<f32>()
                < die.intersect(view).area().max(0.0) * 0.045;
        if within_budget && (local || step == max_stride) {
            return selected;
        }
        if step == max_stride {
            return keep_focus(labels);
        }
    }
    keep_focus(labels)
}

fn critical_sites(hops: &[MappedHop]) -> Vec<(&Site, Vec<usize>)> {
    let mut sites: Vec<(&Site, Vec<usize>)> = Vec::new();
    for (i, hop) in hops.iter().enumerate() {
        for site in &hop.sites {
            if let Some((_, numbers)) = sites.iter_mut().find(|(s, _)| *s == site) {
                numbers.push(i + 1);
            } else {
                sites.push((site, vec![i + 1]));
            }
        }
    }
    for (_, numbers) in &mut sites {
        numbers.sort_unstable();
        numbers.dedup();
    }
    sites
}

fn critical_text(numbers: &[usize], compact: bool) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < numbers.len() {
        let mut end = i;
        if compact {
            while end + 1 < numbers.len() && numbers[end + 1] == numbers[end] + 1 {
                end += 1;
            }
        }
        if end >= i + 2 {
            parts.push(format!("{}–{}", numbers[i], numbers[end]));
        } else {
            parts.extend(numbers[i..=end].iter().map(usize::to_string));
        }
        i = end + 1;
    }
    parts.join(",")
}

/// Try the four sides first, then stack below the nearest label. At an
/// edge, search the remaining gaps rather than clipping or hiding a hop.
fn critical_badge(
    view: Rect,
    outline: Rect,
    size: Vec2,
    placed: &[Rect],
    nearby_only: bool,
) -> Option<Rect> {
    let gap = 3.0;
    if size.x > view.width() || size.y > view.height() {
        return None;
    }
    let bounded = |p: Pos2| {
        Rect::from_min_size(
            Pos2::new(
                p.x.clamp(view.left(), view.right() - size.x),
                p.y.clamp(view.top(), view.bottom() - size.y),
            ),
            size,
        )
    };
    let free = |r: Rect| !placed.iter().any(|p| p.expand(1.0).intersects(r));
    for p in [
        outline.left_top() - Vec2::new(0.0, size.y + gap),
        outline.left_bottom() + Vec2::new(0.0, gap),
        outline.right_top() + Vec2::new(gap, 0.0),
        outline.left_top() - Vec2::new(size.x + gap, 0.0),
    ] {
        let r = bounded(p);
        if free(r) {
            return Some(r);
        }
    }
    if nearby_only {
        return None;
    }
    let nearest = placed
        .iter()
        .min_by(|a, b| {
            a.center()
                .distance_sq(outline.center())
                .total_cmp(&b.center().distance_sq(outline.center()))
        })
        .copied()?;
    let mut p = bounded(nearest.left_bottom() + Vec2::new(0.0, gap)).min;
    while p.y + size.y <= view.bottom() {
        let r = Rect::from_min_size(p, size);
        let bottom = placed
            .iter()
            .filter(|other| other.expand(1.0).intersects(r))
            .map(Rect::bottom)
            .max_by(f32::total_cmp);
        match bottom {
            None => return Some(r),
            Some(bottom) => p.y = bottom + gap,
        }
    }
    let mut xs = vec![view.left()];
    let mut ys = vec![view.top()];
    for r in placed {
        xs.extend([r.right() + gap, r.left() - size.x - gap]);
        ys.extend([r.bottom() + gap, r.top() - size.y - gap]);
    }
    ys.into_iter()
        .flat_map(|y| xs.iter().map(move |&x| bounded(Pos2::new(x, y))))
        .filter(|&r| free(r))
        .min_by(|a, b| {
            a.center()
                .distance_sq(nearest.center())
                .total_cmp(&b.center().distance_sq(nearest.center()))
        })
}

#[derive(Clone)]
struct CriticalLabel {
    outlines: Vec<Rect>,
    numbers: Vec<usize>,
}

fn draw_critical_labels(
    painter: &egui::Painter,
    view: Rect,
    mut labels: Vec<CriticalLabel>,
    mut compact: bool,
) {
    const MIN_FONT_SIZE: f32 = 6.0;
    if labels.is_empty() || !view.is_positive() {
        return;
    }
    let palette = crate::theme::colours(painter.ctx().global_style().visuals.dark_mode);
    let mut font_size = 11.0;
    let padding = Vec2::splat(2.0).min(view.size() * 0.1);
    // Reject collapsed maps before laying out or merging every hop.
    let minimum = painter
        .layout_no_wrap(
            "1".into(),
            egui::FontId::monospace(MIN_FONT_SIZE),
            palette.critical_text,
        )
        .size()
        + padding;
    if minimum.x > view.width() || minimum.y > view.height() {
        return;
    }
    loop {
        let mut placed = Vec::new();
        let mut galleys = Vec::new();
        let mut failed = None;
        for (i, label) in labels.iter().enumerate() {
            let galley = painter.layout(
                critical_text(&label.numbers, compact),
                egui::FontId::monospace(font_size),
                palette.critical_text,
                (view.width() - padding.x).max(1.0),
            );
            let Some(badge) = critical_badge(
                view,
                label.outlines[0],
                galley.size() + padding,
                &placed,
                false,
            ) else {
                failed = Some(i);
                break;
            };
            placed.push(badge);
            galleys.push(galley);
        }
        if let Some(i) = failed {
            // A crowded, small map can share a badge. Merging frees space
            // without losing numbers.
            if labels.len() > 1 {
                let label = labels.remove(i);
                let nearest = labels
                    .iter_mut()
                    .min_by(|a, b| {
                        a.outlines[0]
                            .center()
                            .distance_sq(label.outlines[0].center())
                            .total_cmp(
                                &b.outlines[0]
                                    .center()
                                    .distance_sq(label.outlines[0].center()),
                            )
                    })
                    .unwrap();
                nearest.outlines.extend(label.outlines);
                nearest.numbers.extend(label.numbers);
                nearest.numbers.sort_unstable();
                nearest.numbers.dedup();
                continue;
            }
            // Once every hop shares a badge, shrink only as far as the
            // minimum readable font, then omit labels that still cannot fit.
            if font_size <= MIN_FONT_SIZE {
                return;
            }
            compact = true;
            font_size = (font_size * 0.8).max(MIN_FONT_SIZE);
            continue;
        }
        // Only real route pieces draw line segments. Badges have no leaders
        // from approximate positions that could be mistaken for routed wires.
        for (badge, galley) in placed.into_iter().zip(galleys) {
            painter.rect_filled(badge, 2.0, palette.critical);
            painter.galley(badge.min + padding / 2.0, galley, palette.critical_text);
        }
        break;
    }
}

fn timing_tile(f: &Fabric, tile: Tile) -> Option<Pos2> {
    (tile.row < f.rows && tile.col < f.cols).then(|| {
        Pos2::new(
            (tile.col as f32 + 0.5) * TILE_W,
            f.row_top[tile.row as usize] + f.row_height(tile.row) / 2.0,
        )
    })
}

fn timing_site(f: &Fabric, site: &Site) -> Option<Rect> {
    timing_tile(f, site.tile)?;
    let base = Vec2::new(
        site.tile.col as f32 * TILE_W,
        f.row_top[site.tile.row as usize],
    );
    let b = f
        .template(site.tile.row, site.tile.col)
        .boxes
        .iter()
        .find(|b| b.label == site.label);
    Some(b.map_or_else(
        || {
            Rect::from_center_size(
                timing_tile(f, site.tile).unwrap(),
                Vec2::new(TILE_W, f.row_height(site.tile.row)),
            )
        },
        |b| Rect::from_min_max(Pos2::from(b.min) + base, Pos2::from(b.max) + base),
    ))
}

/// Halfway along the painted pieces, measured by length rather than by
/// their bounds: the anchor stays on a wire even when the route bends.
fn critical_midpoint(pieces: &[(Pos2, Pos2)]) -> Option<Pos2> {
    let mut left = pieces.iter().map(|(a, b)| a.distance(*b)).sum::<f32>() / 2.0;
    for &(a, b) in pieces {
        let length = a.distance(b);
        if left <= length {
            return Some(a + (b - a) * if length > 0.0 { left / length } else { 0.0 });
        }
        left -= length;
    }
    pieces.last().map(|(_, b)| *b)
}

/// Clip for badge anchors only. Painted route endpoints remain untouched.
fn critical_visible_segment(view: Rect, a: Pos2, b: Pos2) -> Option<(Pos2, Pos2)> {
    let delta = b - a;
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for (at, step, min, max) in [
        (a.x, delta.x, view.left(), view.right()),
        (a.y, delta.y, view.top(), view.bottom()),
    ] {
        if step == 0.0 {
            if at < min || at > max {
                return None;
            }
        } else {
            let (a, b) = ((min - at) / step, (max - at) / step);
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
            if lo > hi {
                return None;
            }
        }
    }
    Some((view.clamp(a + delta * lo), view.clamp(a + delta * hi)))
}

fn draw_critical(
    painter: &egui::Painter,
    view: Rect,
    f: &Fabric,
    overlay: &Overlay,
    path: &MappedPath,
    screen: &dyn Fn([f32; 2]) -> Pos2,
    focused: Option<usize>,
) {
    let stroke = Stroke::new(
        CRITICAL_STROKE,
        crate::theme::colours(painter.ctx().global_style().visuals.dark_mode).critical,
    );
    let mut labels = Vec::new();
    for (i, hop) in path.hops.iter().enumerate() {
        let mut pieces = Vec::new();
        let mut visible = Vec::new();
        for route in &hop.hop.route {
            let p = &overlay.pieces[route.piece as usize];
            let (a, b) = if route.reverse {
                (p.b, p.a)
            } else {
                (p.a, p.b)
            };
            let (a, b) = (screen(a), screen(b));
            pieces.push((a, b));
            if let Some(piece) = critical_visible_segment(view, a, b) {
                visible.push(piece);
            }
            if view.intersects(Rect::from_two_pos(a, b).expand(4.0)) {
                painter.line_segment([a, b], stroke);
            }
        }
        if let Some(at) = critical_midpoint(&visible).or_else(|| critical_midpoint(&pieces)) {
            labels.push(CriticalLabel {
                outlines: vec![Rect::from_center_size(at, Vec2::ZERO)],
                numbers: vec![i + 1],
            });
        }
    }
    let sites = critical_sites(&path.hops);
    let outlines: Vec<_> = sites
        .iter()
        .map(|(site, _)| {
            timing_site(f, site)
                .map(|rect| Rect::from_min_max(screen(rect.min.into()), screen(rect.max.into())))
        })
        .collect();
    // Cell hops belong to their source site. Routing hops have a badge on
    // the route when resolved. Unresolved hops retain both site outlines and
    // their number at the driver; only the report explains the missing route.
    let owners: Vec<_> = path
        .hops
        .iter()
        .map(|hop| {
            let candidates: Vec<_> = hop
                .sites
                .iter()
                .filter_map(|site| {
                    sites
                        .iter()
                        .position(|(s, _)| *s == site)
                        .filter(|&i| outlines[i].is_some())
                })
                .collect();
            candidates
                .iter()
                .copied()
                .find(|&i| view.intersects(outlines[i].unwrap()))
                .or_else(|| candidates.first().copied())
        })
        .collect();
    for (i, (_, numbers)) in sites.into_iter().enumerate() {
        if let Some(rect) = outlines[i] {
            if numbers.iter().any(|&n| {
                path.hops[n - 1].hop.kind != HopKind::Routing
                    || path.hops[n - 1].route_not_shown().is_some()
            }) && view.intersects(rect)
            {
                painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Middle);
            }
            let numbers: Vec<_> = numbers
                .into_iter()
                .filter(|&n| {
                    (path.hops[n - 1].hop.kind != HopKind::Routing
                        || path.hops[n - 1].route_not_shown().is_some())
                        && owners[n - 1] == Some(i)
                })
                .collect();
            if !numbers.is_empty() {
                labels.push(CriticalLabel {
                    outlines: vec![rect],
                    numbers,
                });
            }
        }
    }
    for (i, hop) in path
        .hops
        .iter()
        .enumerate()
        .filter(|(i, hop)| hop.hop.kind != HopKind::Routing && owners[*i].is_none())
    {
        let at = hop
            .hop
            .tiles
            .and_then(|(tile, _)| timing_tile(f, tile))
            .map_or(view.center(), |p| screen(p.into()));
        labels.push(CriticalLabel {
            outlines: vec![Rect::from_center_size(at, Vec2::ZERO)],
            numbers: vec![i + 1],
        });
    }
    if let Some(n) = focused.filter(|&n| n > 0 && n <= path.hops.len()) {
        if !labels.iter().any(|label| label.numbers.contains(&n)) {
            // A report selection can lack both a resolved route and a site.
            // Keep its number visible without inventing wire geometry.
            labels.push(CriticalLabel {
                outlines: vec![Rect::from_center_size(view.center(), Vec2::ZERO)],
                numbers: vec![n],
            });
        }
    }
    labels.sort_by_key(|label| label.numbers[0]);
    let compact = screen([TILE_W, 0.0]).distance(screen([0.0, 0.0])) < 20.0;
    let size = f.size();
    let fit = (view.width() / size[0]).min(view.height() / size[1]);
    let zoom = screen([TILE_W, 0.0]).distance(screen([0.0, 0.0])) / TILE_W / fit;
    let die = Rect::from_min_max(screen([0.0; 2]), screen(size));
    let labels = critical_badge_subset(painter, view, die, labels, zoom, focused, compact);
    draw_critical_labels(painter, view, labels, compact);
}

/// The wires of a template whose pins end on a box, as the tile types name
/// them, sorted.
pub(crate) fn pins_of(t: &fabric::Template, b: &fabric::SiteBox) -> Vec<String> {
    let on = |e: [f32; 2]| {
        const E: f32 = 1e-4;
        e[0] >= b.min[0] - E && e[0] <= b.max[0] + E && e[1] >= b.min[1] - E && e[1] <= b.max[1] + E
    };
    let mut pins: Vec<String> = t
        .segments
        .iter()
        .zip(&t.owners)
        .filter(|((a, z, part), _)| *part == Part::Pin && (on(*a) || on(*z)))
        .filter_map(|(_, owner)| match owner {
            fabric::Owner::Wire(w) => match &t.wires[*w as usize] {
                fabric::WireKey::Name(n) => Some(n.clone()),
                fabric::WireKey::Track(..) => None,
            },
            fabric::Owner::Arc(_) => None,
        })
        .collect();
    pins.sort();
    pins.dedup();
    pins
}

/// How far a point is from a segment.
fn to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    (x * x + y * y).sqrt()
}

impl FabricView {
    /// Where a CIB ties a net to a constant: a small site of its own with
    /// the constant in it, where the net's wire leaves the CIB, so that the
    /// net starts somewhere, as every other starts at a site.
    fn ties(
        &self,
        painter: &egui::Painter,
        view: Rect,
        o: &Overlay,
        screen: &dyn Fn([f32; 2]) -> Pos2,
    ) {
        if self.cell < 30.0 {
            return;
        }
        let side = (self.cell * 0.07).clamp(12.0, 24.0);
        for (net, value) in o.tied_nets() {
            for &p in o.drivers_of(net) {
                let q = screen(p);
                if !view.expand(side).contains(q) {
                    continue;
                }
                let rect = Rect::from_center_size(q, Vec2::splat(side));
                painter.rect_filled(rect, 1.0, SITE);
                painter.rect_stroke(rect, 1.0, Stroke::new(1.0, SITE_EDGE), StrokeKind::Inside);
                painter.text(
                    q,
                    egui::Align2::CENTER_CENTER,
                    if value { "1" } else { "0" },
                    egui::FontId::monospace(side * 0.8),
                    SITE_TEXT,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The map of a large design (`life`, 10 000 LUTs, built into
    //! target/reference by examples/ulx3s/build.sh), drawn by egui without a
    //! window: how long a frame takes, and whether a change shows.
    use super::*;
    use ecp5_emu::live::{self, Command, Design, Shared};
    use std::time::{Duration, Instant};

    fn camera_fabric() -> Fabric {
        let positions = 96 * 127;
        serde_json::from_value(serde_json::json!({
            "rows": 96, "cols": 127, "kinds": vec![vec!["PLC2"]; positions],
            "templates": [fabric::Template::default()], "template_at": vec![0; positions],
            "links": [], "links_at": vec![[0, 0]; positions],
            "through_at": vec![[0, 0]; positions], "through": [],
            "row_top": (0..=96).map(|r| r as f32 * TILE_W).collect::<Vec<_>>(),
            "macros": [], "dsp_inside": 0, "dsp_lost": 0, "broken_links": 0,
            "broken": {}, "sites": 0, "lanes": {"h": {}, "v": {}}, "arcs": 0,
        }))
        .unwrap()
    }

    fn camera_frame(
        ctx: &egui::Context,
        view: &mut FabricView,
        fabric: &Fabric,
        rect: Rect,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 950.0))),
                ..Default::default()
            },
            |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    view.show(ui, fabric, None, None);
                });
            },
        )
    }

    #[test]
    fn map_resizes_keep_zoomed_centre_and_refit_at_fit() {
        let f = camera_fabric();
        let ctx = egui::Context::default();
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        let full = Rect::from_min_max(Pos2::new(608.0, 60.0), Pos2::new(1492.0, 860.0));
        let rects = [
            Rect::from_min_max(full.min, Pos2::new(1144.0, 860.0)), // Report.
            Rect::from_min_max(full.min, Pos2::new(1000.0, 860.0)), // Inspector.
            Rect::from_min_max(full.min, Pos2::new(1000.0, 620.0)), // Waves.
            Rect::from_min_size(Pos2::new(650.0, 85.0), Vec2::new(350.0, 500.0)),
            Rect::from_min_size(Pos2::new(620.0, 100.0), Vec2::new(350.0, 500.0)),
            full,
        ];
        camera_frame(&ctx, &mut view, &f, full);
        for zoom in [19.0, 1.0] {
            // A panned view at the fit scale must also keep its chosen centre.
            view.aim(0.4, 0.6, zoom);
            let out = camera_frame(&ctx, &mut view, &f, full);
            assert!((view.zoom_level() - zoom).abs() < 1e-4);
            assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(r) if r.fill == BACKGROUND && r.rect == full && shape.clip_rect.contains_rect(full)
            )), "fabric background covers the whole map, including its right edge");
            let centre = (view.map_rect.size() / 2.0 - view.offset) / view.cell;
            let cell = view.cell;
            for rect in rects {
                camera_frame(&ctx, &mut view, &f, rect);
                assert_eq!(view.map_rect, rect);
                assert_eq!(view.cell, cell);
                let point = view.map_rect.min + view.offset + centre * view.cell;
                assert!(
                    point.distance(view.map_rect.center()) < 1.0,
                    "{rect:?}: {point:?}"
                );
            }
        }
        view.fit();
        camera_frame(&ctx, &mut view, &f, full);
        for rect in rects {
            camera_frame(&ctx, &mut view, &f, rect);
            let size = Vec2::from(f.size());
            let cell = (rect.width() / size.x).min(rect.height() / size.y);
            assert!((view.cell - cell).abs() < 1e-5);
            assert!((view.offset - (rect.size() - size * cell) / 2.0).length() < 0.01);
            assert!((view.zoom_level() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn critical_hop_focus_centres_in_the_resized_map() {
        let f = camera_fabric();
        let ctx = egui::Context::default();
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        let full = Rect::from_min_max(Pos2::new(608.0, 60.0), Pos2::new(1492.0, 860.0));
        camera_frame(&ctx, &mut view, &f, full);
        view.aim(0.4, 0.6, 19.0);
        camera_frame(&ctx, &mut view, &f, full);
        let mut hop = ecp5_emu::timing_path::parse(
            include_str!("../../tests/fixtures/timing_path/blinky_cycle/creator.txt"),
            include_str!("../../tests/fixtures/timing_path/blinky_cycle/nextpnr.log"),
        )
        .unwrap()
        .paths[0]
            .hops[0]
            .clone();
        hop.tiles = None;
        let hop = MappedHop {
            name: hop.name(),
            hop,
            nets: Vec::new(),
            locations: Vec::new(),
            sites: vec![Site {
                tile: Tile { row: 33, col: 12 },
                label: "FF0".into(),
            }],
        };
        let centre = timing_site(&f, &hop.sites[0]).unwrap().center();
        view.centre_hop(&hop);
        let narrow = Rect::from_min_max(full.min, Pos2::new(1144.0, 860.0));
        camera_frame(&ctx, &mut view, &f, narrow);
        assert!(view.focus_hop.is_none());
        let screen = view.map_rect.min + view.offset + centre.to_vec2() * view.cell;
        assert!(screen.distance(narrow.center()) < 1.0);
    }

    fn near(c: Color32) -> bool {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        r > 200 && g < 190 && b < 120
    }

    /// A used wire glowing at all: moved from its colour towards that of a
    /// change (a quiet wire's change as fast as it goes glows by only
    /// `BUSY_LEAST` or a little more, src/changes.rs).
    fn tinted(c: Color32) -> bool {
        let [r, _, b, _] = c.to_srgba_unmultiplied();
        r > 200 && b + 12 <= USED.b()
    }

    /// Pixels in the colour of a change, or near it, of the images sent to
    /// the screen this frame (the wires of a large design from afar).
    fn orange_pixels(delta: &egui::TexturesDelta) -> usize {
        delta
            .set
            .iter()
            .map(|(_, d)| {
                let egui::ImageData::Color(image) = &d.image;
                image.pixels.iter().filter(|&&c| near(c)).count()
            })
            .sum()
    }

    /// Shapes drawn in the colour of a change, or near it.
    fn orange(shapes: &[egui::epaint::ClippedShape]) -> usize {
        fn walk(s: &egui::Shape) -> usize {
            match s {
                egui::Shape::Vec(v) => v.iter().map(walk).sum(),
                egui::Shape::LineSegment { stroke, .. } => usize::from(near(stroke.color)),
                egui::Shape::Mesh(m) => m.vertices.iter().filter(|v| near(v.color)).count() / 4,
                egui::Shape::Path(p) => match p.stroke.color {
                    egui::epaint::ColorMode::Solid(c) => usize::from(near(c)),
                    _ => 0,
                },
                _ => 0,
            }
        }
        shapes.iter().map(|c| walk(&c.shape)).sum()
    }

    /// The image of the wires a frame sent to the screen, if it sent one.
    fn sent(out: &egui::FullOutput) -> Option<Arc<egui::ColorImage>> {
        out.textures_delta
            .set
            .iter()
            .map(|(_, d)| {
                let egui::ImageData::Color(image) = &d.image;
                Arc::clone(image)
            })
            .next()
    }

    fn is_critical(colour: Color32) -> bool {
        [false, true]
            .into_iter()
            .any(|dark| colour == crate::theme::colours(dark).critical)
    }

    fn critical_painted_labels(out: &egui::FullOutput) -> Vec<(Rect, String)> {
        fn flatten<'a>(shape: &'a egui::Shape, shapes: &mut Vec<&'a egui::Shape>) {
            if let egui::Shape::Vec(children) = shape {
                for child in children {
                    flatten(child, shapes);
                }
            } else {
                shapes.push(shape);
            }
        }
        let mut shapes = Vec::new();
        for shape in &out.shapes {
            flatten(&shape.shape, &mut shapes);
        }
        shapes
            .windows(2)
            .filter_map(|pair| match (pair[0], pair[1]) {
                (egui::Shape::Rect(r), egui::Shape::Text(t)) if is_critical(r.fill) => {
                    assert!(r
                        .rect
                        .contains_rect(Rect::from_min_size(t.pos, t.galley.size())));
                    Some((r.rect, t.galley.text().to_string()))
                }
                _ => None,
            })
            .collect()
    }

    fn critical_label_numbers(text: &str) -> Vec<usize> {
        text.split(',')
            .flat_map(|part| {
                if let Some((a, b)) = part.split_once('–') {
                    a.parse().unwrap()..=b.parse().unwrap()
                } else {
                    let n = part.parse().unwrap();
                    n..=n
                }
            })
            .collect()
    }

    fn critical_badge_numbers(out: &egui::FullOutput, map: Rect) -> Vec<usize> {
        let labels = critical_painted_labels(out);
        let mut numbers = Vec::new();
        for (i, (rect, text)) in labels.iter().enumerate() {
            assert!(map.contains_rect(*rect), "outside map: {text} {rect:?}");
            for (other, other_text) in &labels[..i] {
                assert!(!rect.intersects(*other), "overlap: {text} and {other_text}");
            }
            let ns = critical_label_numbers(text);
            assert!(ns.windows(2).all(|w| w[0] < w[1]), "unsorted: {text}");
            numbers.extend(ns);
        }
        numbers.sort_unstable();
        assert!(
            numbers.windows(2).all(|pair| pair[0] < pair[1]),
            "duplicate badge number"
        );
        numbers
    }

    fn assert_critical_labels(out: &egui::FullOutput, map: Rect, hops: usize) {
        assert_eq!(
            critical_badge_numbers(out, map),
            (1..=hops).collect::<Vec<_>>()
        );
    }

    fn critical_on_screen_hops(
        f: &Fabric,
        overlay: &Overlay,
        path: &MappedPath,
        view: Rect,
        screen: &dyn Fn([f32; 2]) -> Pos2,
    ) -> Vec<usize> {
        path.hops
            .iter()
            .enumerate()
            .filter_map(|(i, hop)| {
                let visible = if !hop.hop.route.is_empty() {
                    hop.hop.route.iter().any(|route| {
                        let p = &overlay.pieces[route.piece as usize];
                        critical_visible_segment(view, screen(p.a), screen(p.b)).is_some()
                    })
                } else {
                    hop.sites
                        .iter()
                        .filter_map(|site| timing_site(f, site))
                        .any(|r| {
                            view.intersects(Rect::from_min_max(
                                screen(r.min.into()),
                                screen(r.max.into()),
                            ))
                        })
                };
                visible.then_some(i + 1)
            })
            .collect()
    }

    #[test]
    fn critical_same_site_hops_are_collected_once() {
        let report = ecp5_emu::timing_path::parse(
            include_str!("../../tests/fixtures/timing_path/picorv32_console/creator.txt"),
            include_str!("../../tests/fixtures/timing_path/picorv32_console/nextpnr.log"),
        )
        .unwrap();
        let site = Site {
            tile: Tile { row: 33, col: 12 },
            label: "FF0".into(),
        };
        let mut hops: Vec<_> = report.paths[0]
            .hops
            .iter()
            .map(|hop| MappedHop {
                hop: hop.clone(),
                name: hop.name(),
                nets: Vec::new(),
                sites: Vec::new(),
                locations: Vec::new(),
            })
            .collect();
        assert_eq!(hops[17].hop.kind, HopKind::Routing);
        hops[17].sites = vec![site.clone(), site.clone()];
        hops[18].sites = vec![site.clone()];
        let sites = critical_sites(&hops);
        assert_eq!(sites, vec![(&site, vec![18, 19])]);
        assert_eq!(critical_text(&[3, 4, 5, 6, 7, 8, 9, 12], true), "3–9,12");
        assert_eq!(critical_text(&[10, 11, 12], false), "10,11,12");
    }

    #[test]
    fn critical_midpoint_stays_on_the_route() {
        let a = Pos2::new(10.0, 20.0);
        let b = Pos2::new(50.0, 20.0);
        let c = Pos2::new(50.0, 80.0);
        assert_eq!(
            critical_midpoint(&[(a, b), (b, c)]),
            Some(Pos2::new(50.0, 30.0))
        );
        assert_eq!(critical_midpoint(&[(a, a)]), Some(a));
        assert_eq!(critical_midpoint(&[]), None);
    }

    #[test]
    fn critical_labels_skip_maps_too_small_for_one_badge() {
        for map in [
            Rect::ZERO,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 4.0)),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 180.0)),
        ] {
            for hops in [1, 19, 200] {
                let ctx = egui::Context::default();
                let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let before = ctx.fonts(|f| f.num_galleys_in_cache());
                    draw_critical_labels(
                        ui.painter(),
                        map,
                        (1..=hops)
                            .map(|n| CriticalLabel {
                                outlines: vec![map],
                                numbers: vec![n],
                            })
                            .collect(),
                        false,
                    );
                    let layouts = ctx.fonts(|f| f.num_galleys_in_cache()) - before;
                    assert!(layouts <= 1, "{map:?}: {layouts} layouts for {hops} hops");
                });
                assert!(critical_painted_labels(&out).is_empty());
                assert!(!out.shapes.iter().any(|s| matches!(
                    &s.shape,
                    egui::Shape::Text(_) | egui::Shape::LineSegment { .. }
                )));
            }
        }
    }

    #[test]
    fn critical_labels_stop_shrinking_at_minimum_font() {
        let ctx = egui::Context::default();
        let map = Rect::from_min_size(Pos2::ZERO, Vec2::new(30.0, 20.0));
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let before = ctx.fonts(|f| f.num_galleys_in_cache());
            draw_critical_labels(
                ui.painter(),
                map,
                vec![CriticalLabel {
                    outlines: vec![map],
                    numbers: (1..=199).step_by(2).collect(),
                }],
                false,
            );
            let layouts = ctx.fonts(|f| f.num_galleys_in_cache()) - before;
            assert!(layouts <= 5, "{layouts} layouts while shrinking one badge");
        });
        assert!(critical_painted_labels(&out).is_empty());
    }

    #[test]
    fn critical_labels_stack_and_keep_every_number() {
        let ctx = egui::Context::default();
        let map = Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 180.0));
        let outline = Rect::from_center_size(map.center(), Vec2::splat(12.0));
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(map),
                ..Default::default()
            },
            |ui| {
                draw_critical_labels(
                    ui.painter(),
                    map,
                    (1..=19)
                        .map(|n| CriticalLabel {
                            outlines: vec![outline],
                            numbers: vec![n],
                        })
                        .collect(),
                    true,
                );
            },
        );
        assert_critical_labels(&out, map, 19);
        let labels = critical_painted_labels(&out);
        assert_eq!(labels.len(), 19);
        // Four local positions fill first; the fifth stacks under the
        // nearest badge without adding synthetic wire segments.
        let nearest = labels[..4]
            .iter()
            .min_by(|(a, _), (b, _)| {
                a.center()
                    .distance_sq(outline.center())
                    .total_cmp(&b.center().distance_sq(outline.center()))
            })
            .unwrap()
            .0;
        assert!(labels[4].0.top() > nearest.bottom());
        assert_eq!(labels[4].0.left(), nearest.left());
        assert!(!out
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::LineSegment { .. })));
    }

    #[test]
    fn critical_crowded_map_shares_badges_without_losing_hops() {
        let ctx = egui::Context::default();
        let map = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(70.0, 35.0));
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0))),
                ..Default::default()
            },
            |ui| {
                draw_critical_labels(
                    ui.painter(),
                    map,
                    (1..=19)
                        .map(|n| CriticalLabel {
                            outlines: vec![Rect::from_center_size(Pos2::ZERO, Vec2::ZERO)],
                            numbers: vec![n],
                        })
                        .collect(),
                    true,
                );
            },
        );
        assert_critical_labels(&out, map, 19);
        let labels = critical_painted_labels(&out);
        assert!(labels.len() < 19);
        assert!(labels.iter().any(|(_, text)| text.contains('–')));
    }

    #[test]
    fn critical_badges_thin_fifty_seven_hops_and_keep_focus() {
        let map = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let die = map.shrink(30.0);
        for zoom in [1.0, 4.0, 19.0] {
            for focused in [None, Some(17)] {
                let ctx = egui::Context::default();
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(map),
                        ..Default::default()
                    },
                    |ui| {
                        let mut labels: Vec<_> = (1..=57)
                            .map(|n| CriticalLabel {
                                outlines: vec![Rect::from_center_size(
                                    map.center() + Vec2::splat((n as f32 - 29.0) * 0.3 * zoom),
                                    Vec2::ZERO,
                                )],
                                numbers: vec![n],
                            })
                            .collect();
                        // The selected number must get its own badge even when
                        // several hops share the same physical site.
                        labels[15].numbers.push(17);
                        labels.remove(16);
                        let selected = critical_badge_subset(
                            ui.painter(),
                            map,
                            die,
                            labels,
                            zoom,
                            focused,
                            false,
                        );
                        draw_critical_labels(ui.painter(), map, selected, false);
                    },
                );
                let numbers = critical_badge_numbers(&out, map);
                let labels = critical_painted_labels(&out);
                let coverage = labels.iter().map(|(rect, _)| rect.area()).sum::<f32>() / die.area();
                if zoom == 1.0 {
                    assert_eq!(numbers, focused.into_iter().collect::<Vec<_>>());
                    assert!(coverage < 0.05);
                } else if zoom == 4.0 {
                    assert!(numbers.contains(&1) && numbers.contains(&57));
                    assert!(numbers.len() < 57);
                    assert!((2..=57).any(|step| numbers
                        == (1..=57)
                            .filter(|&n| n == 1
                                || n == 57
                                || Some(n) == focused
                                || (n - 1) % step == 0)
                            .collect::<Vec<_>>()));
                } else {
                    assert_eq!(numbers, (1..=57).collect::<Vec<_>>());
                }
                if let Some(n) = focused {
                    assert!(labels.iter().any(|(_, text)| text == &n.to_string()));
                }
                critical_geometry(&out, 1.0, Pos2::ZERO);
            }
        }
    }

    #[test]
    fn critical_visible_anchors_clip_to_the_route() {
        let view = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 100.0));
        assert_eq!(
            critical_visible_segment(view, Pos2::new(-10.0, 50.0), Pos2::new(120.0, 50.0)),
            Some((Pos2::new(10.0, 50.0), Pos2::new(110.0, 50.0)))
        );
        assert_eq!(
            critical_visible_segment(view, Pos2::new(-10.0, 90.0), Pos2::new(20.0, 120.0)),
            None
        );
        let point = Pos2::new(30.0, 40.0);
        assert_eq!(
            critical_visible_segment(view, point, point),
            Some((point, point))
        );
    }

    #[test]
    #[ignore = "needs local reference designs, Trellis and clang"]
    fn critical_picorv32_labels_at_fit_four_and_nineteen_times_zoom() {
        let (commands, s) = critical_reference("picorv32_console");
        let path = &s.timing.get().unwrap().result.as_ref().unwrap()[0];
        let hop_count = path.path.hops.len();
        assert!(hop_count > 0);
        assert_eq!(path.hops.len(), hop_count);
        let mut bounds = Rect::NOTHING;
        for hop in &path.hops {
            for site in &hop.sites {
                if let Some(rect) = timing_site(&s.fabric, site) {
                    bounds = bounds.union(rect);
                }
            }
            for route in &hop.hop.route {
                let p = &s.overlay.pieces[route.piece as usize];
                bounds.extend_with(Pos2::from(p.a));
                bounds.extend_with(Pos2::from(p.b));
            }
        }
        let mut fallback = (**path).clone();
        for hop in &mut fallback.hops {
            hop.nets.clear();
            hop.hop.route.clear();
        }
        let hop = fallback
            .hops
            .iter_mut()
            .rev()
            .find(|hop| hop.hop.kind == HopKind::Routing && !hop.sites.is_empty())
            .unwrap();
        hop.sites.push(hop.sites[0].clone());
        let fallback = Arc::new(fallback);
        let values = vec![2; s.overlay.values.len()];
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut view = FabricView::new(None, None, (0.5, 0.5));
            view.critical = Some(Arc::clone(path));
            let frame = |view: &mut FabricView| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(1500.0, 898.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            view.show(
                                ui,
                                &s.fabric,
                                Some((&s.overlay, &values)),
                                Some(&s.net_names),
                            );
                        });
                    },
                )
            };
            let check = |out: &egui::FullOutput,
                         view: &FabricView,
                         path: &MappedPath,
                         zoom: f32| {
                let screen = |p| view.map_rect.min + view.offset + Vec2::from(p) * view.cell;
                let expected =
                    critical_on_screen_hops(&s.fabric, &s.overlay, path, view.map_rect, &screen);
                let numbers = critical_badge_numbers(out, view.map_rect);
                assert!(numbers.iter().all(|n| expected.contains(n)));
                let labels = critical_painted_labels(out);
                let die = Rect::from_min_max(screen([0.0; 2]), screen(s.fabric.size()))
                    .intersect(view.map_rect);
                let coverage = labels.iter().map(|(rect, _)| rect.area()).sum::<f32>() / die.area();
                if zoom == 1.0 {
                    assert!(coverage < 0.05, "fit badges cover {:.2}%", coverage * 100.0);
                }
                if zoom == 4.0 && !expected.is_empty() {
                    assert!(numbers.contains(expected.first().unwrap()));
                    assert!(numbers.contains(expected.last().unwrap()));
                }
                if zoom == 19.0 {
                    assert!(!expected.is_empty(), "test camera must see some hops");
                    assert_eq!(numbers, expected);
                    assert!(
                        labels.iter().all(|(rect, _)| rect.height() >= 13.0),
                        "readable badges"
                    );
                }
                println!(
                    "# critical badges zoom={zoom} dark={dark} visible={} shown={} coverage={:.3}%",
                    expected.len(),
                    numbers.len(),
                    coverage * 100.0
                );
            };
            frame(&mut view);
            let fit = (view.cell, view.offset);
            for zoom in [1.0, 4.0, 19.0] {
                view.cell = fit.0 * zoom;
                view.offset = if zoom == 1.0 {
                    fit.1
                } else {
                    view.map_rect.size() / 2.0 - bounds.center().to_vec2() * view.cell
                };
                view.critical = Some(Arc::clone(path));
                check(&frame(&mut view), &view, path, zoom);
                view.critical = Some(Arc::clone(&fallback));
                check(&frame(&mut view), &view, &fallback, zoom);
            }
            view.critical = Some(Arc::clone(path));
            let (i, hop) = path
                .hops
                .iter()
                .enumerate()
                .skip(hop_count / 2)
                .find(|(_, hop)| hop.hop.kind != HopKind::Routing && !hop.sites.is_empty())
                .unwrap();
            view.centre_hop(hop);
            frame(&mut view);
            assert!(view.focus_hop.is_none());
            let site = timing_site(&s.fabric, &hop.sites[0]).unwrap();
            let at = view.map_rect.min + view.offset + site.center().to_vec2() * view.cell;
            assert!(
                at.distance(view.map_rect.center()) < 2.0,
                "report click still centres the hop"
            );
            for zoom in [1.0, 4.0, 19.0] {
                view.cell = fit.0 * zoom;
                view.offset = if zoom == 1.0 {
                    fit.1
                } else {
                    view.map_rect.size() / 2.0 - bounds.center().to_vec2() * view.cell
                };
                for _ in 0..2 {
                    let out = frame(&mut view);
                    critical_badge_numbers(&out, view.map_rect);
                    assert!(
                        critical_painted_labels(&out)
                            .iter()
                            .any(|(_, text)| text == &(i + 1).to_string()),
                        "focused hop survives zoom={zoom}"
                    );
                }
            }
            view.offset -= Vec2::splat(10000.0);
            let out = frame(&mut view);
            assert!(
                critical_painted_labels(&out)
                    .iter()
                    .any(|(_, text)| text == &(i + 1).to_string()),
                "focused off-screen hop stays labelled"
            );
            view.critical = Some(Arc::new((**path).clone()));
            frame(&mut view);
            assert!(
                view.critical_focus.is_none(),
                "a different report clears the focus override"
            );
            let mut unplaced = (**path).clone();
            let i = unplaced
                .hops
                .iter()
                .position(|hop| hop.hop.kind == HopKind::Routing)
                .unwrap();
            unplaced.hops[i].sites.clear();
            unplaced.hops[i].hop.route.clear();
            let unplaced = Arc::new(unplaced);
            view.critical = Some(Arc::clone(&unplaced));
            view.centre_hop(&unplaced.hops[i]);
            frame(&mut view);
            view.fit();
            let out = frame(&mut view);
            assert!(
                critical_painted_labels(&out)
                    .iter()
                    .any(|(_, text)| text == &(i + 1).to_string()),
                "a focused unresolved hop without a site still has its badge"
            );
            assert_eq!(
                path.hops.len(),
                hop_count,
                "the report keeps its full index"
            );
        }
        drop(commands);
    }

    /// Measure submitted centerlines/outlines independently of stroke footprints
    /// and screen decorations. A fixed-pixel stroke itself covers more die units
    /// at fit; it must never change the geometry to which it is applied.
    fn critical_geometry(out: &egui::FullOutput, cell: f32, origin: Pos2) -> (Rect, usize) {
        fn measure(shape: &egui::Shape, cell: f32, origin: Pos2, geometry: &mut (Rect, usize)) {
            let die = |p: Pos2| Pos2::ZERO + (p - origin) / cell;
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        measure(child, cell, origin, geometry);
                    }
                }
                egui::Shape::LineSegment { points, stroke } if is_critical(stroke.color) => {
                    assert_eq!(stroke.width, 2.5, "route width in screen pixels");
                    for &p in points {
                        geometry.0.extend_with(die(p));
                    }
                    geometry.1 += 1;
                }
                egui::Shape::Rect(r) if is_critical(r.stroke.color) => {
                    assert_eq!(r.stroke.width, 2.5, "site width in screen pixels");
                    for p in [r.rect.min, r.rect.max] {
                        geometry.0.extend_with(die(p));
                    }
                }
                egui::Shape::Circle(c) if is_critical(c.stroke.color) => {
                    panic!("no synthetic markers for unresolved hops");
                }
                egui::Shape::Text(t) => {
                    assert!(t
                        .galley
                        .job
                        .sections
                        .iter()
                        .all(|s| s.format.font_id.size == 11.0));
                }
                _ => {}
            }
        }
        let mut geometry = (Rect::NOTHING, 0);
        for shape in &out.shapes {
            measure(&shape.shape, cell, origin, &mut geometry);
        }
        geometry
    }

    fn assert_critical_hop_chain(
        out: &egui::FullOutput,
        hop: &MappedHop,
        f: &Fabric,
        overlay: &Overlay,
        cell: f32,
        origin: Pos2,
    ) {
        fn segments(shape: &egui::Shape, drawn: &mut Vec<[Pos2; 2]>) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        segments(child, drawn);
                    }
                }
                egui::Shape::LineSegment { points, stroke } if is_critical(stroke.color) => {
                    drawn.push(*points);
                }
                _ => {}
            }
        }
        critical_geometry(out, cell, origin);
        let mut drawn = Vec::new();
        for shape in &out.shapes {
            segments(&shape.shape, &mut drawn);
        }
        assert_eq!(drawn.len(), hop.hop.route.len(), "{}", hop.name);
        let drawn: Vec<_> = drawn
            .into_iter()
            .map(|points| points.map(|p| Pos2::ZERO + (p - origin) / cell))
            .collect();
        for (points, route) in drawn.iter().zip(&hop.hop.route) {
            assert!(hop
                .nets
                .iter()
                .any(|&net| overlay.pieces_of(net).contains(&route.piece)));
            let p = &overlay.pieces[route.piece as usize];
            assert!(p.route, "{}: the ordinary map draws this wire", hop.name);
            let (a, b) = if route.reverse {
                (p.b, p.a)
            } else {
                (p.a, p.b)
            };
            assert!(points[0].distance(Pos2::from(a)) < 0.001);
            assert!(points[1].distance(Pos2::from(b)) < 0.001);
        }
        if drawn.is_empty() {
            assert!(hop.route_not_shown().is_some());
            // Both unresolved endpoints remain outlined; a badge is a fill,
            // so it cannot satisfy this stroke check by accident.
            for site in &hop.sites {
                let rect = timing_site(f, site).unwrap();
                assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(r) if is_critical(r.stroke.color)
                        && r.rect.min.distance(origin + rect.min.to_vec2() * cell) < 0.001
                        && r.rect.max.distance(origin + rect.max.to_vec2() * cell) < 0.001)));
            }
        } else {
            assert_eq!(hop.route_not_shown(), None);
            for pair in drawn.windows(2) {
                assert!(
                    pair[0][1].distance(pair[1][0]) < 0.001,
                    "{}: broken chain",
                    hop.name
                );
            }
            let bounds = |site: &Site, port: &str| {
                let position = site.tile.row as usize * f.cols as usize + site.tile.col as usize;
                let mux = site
                    .label
                    .strip_prefix("LUT")
                    .filter(|_| matches!(port, "F1" | "OFX" | "FXA" | "FXB" | "M"))
                    .and_then(|n| n.parse::<usize>().ok());
                let (min, max) = if let Some(n) = mux {
                    // These TRELLIS_COMB ports belong to the slice's mux,
                    // not its LUT. Check the exact physical mux box from
                    // the fabric independently of the mapper's wire lookup.
                    let (min, max) = fabric::mux_rect(n / 2);
                    let base = Vec2::new(
                        site.tile.col as f32 * TILE_W,
                        f.row_top[site.tile.row as usize],
                    );
                    let expected =
                        Rect::from_min_max(Pos2::from(min) + base, Pos2::from(max) + base);
                    let b = overlay
                        .boxes_in(position)
                        .iter()
                        .find(|b| {
                            b.label == "F5 FX M"
                                && Pos2::from(b.min).distance(expected.min) < 0.001
                                && Pos2::from(b.max).distance(expected.max) < 0.001
                        })
                        .expect("the port's actual mux box is in the overlay");
                    (b.min, b.max)
                } else {
                    overlay
                        .sites_in(position)
                        .iter()
                        .find(|b| b.label == site.label)
                        .map(|b| (b.min, b.max))
                        .or_else(|| {
                            overlay
                                .boxes_in(position)
                                .iter()
                                .find(|b| b.label == site.label)
                                .map(|b| (b.min, b.max))
                        })
                        .expect("resolved endpoints have actual overlay site boxes")
                };
                Rect::from_min_max(Pos2::from(min), Pos2::from(max)).expand(0.001)
            };
            assert!(
                bounds(hop.sites.first().unwrap(), &hop.hop.source.port).contains(drawn[0][0]),
                "{}: driver",
                hop.name
            );
            assert!(
                bounds(
                    hop.sites.last().unwrap(),
                    &hop.hop.sink.as_ref().unwrap().port
                )
                .contains(drawn.last().unwrap()[1]),
                "{}: sink",
                hop.name
            );
        }
    }

    #[test]
    fn critical_badges_use_the_theme_palette() {
        let view = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        for dark in [false, true] {
            let palette = crate::theme::colours(dark);
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(view),
                    ..Default::default()
                },
                |ui| {
                    draw_critical_labels(
                        ui.painter(),
                        view,
                        vec![CriticalLabel {
                            outlines: vec![Rect::from_center_size(view.center(), Vec2::ZERO)],
                            numbers: vec![1],
                        }],
                        false,
                    );
                },
            );
            assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(r) if r.fill == palette.critical)));
            assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(t) if t.galley.job.sections.iter()
                    .all(|section| section.format.color == palette.critical_text))));
        }
    }

    #[test]
    fn critical_crowded_and_offscreen_labels_have_no_fake_segments() {
        let view = Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 898.0));
        for zoom in [1.0, 4.0, 19.0] {
            let ctx = egui::Context::default();
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(view),
                    ..Default::default()
                },
                |ui| {
                    draw_critical_labels(
                        ui.painter(),
                        view,
                        (1..=19)
                            .map(|n| CriticalLabel {
                                outlines: vec![Rect::from_center_size(
                                    view.center()
                                        + Vec2::new(
                                            if n == 19 { -1000.0 * zoom } else { 0.0 },
                                            n as f32 * zoom,
                                        ),
                                    Vec2::ZERO,
                                )],
                                numbers: vec![n],
                            })
                            .collect(),
                        false,
                    );
                },
            );
            assert_critical_labels(&out, view, 19);
            let (geometry, pieces) = critical_geometry(&out, 1.0, Pos2::ZERO);
            assert_eq!(geometry, Rect::NOTHING);
            assert_eq!(pieces, 0);
        }
    }

    #[test]
    #[ignore = "needs local reference designs, Trellis and clang"]
    fn critical_reference_geometry_at_fit_four_and_nineteen_times_zoom() {
        for design in ["blinky_cycle", "picorv32_console"] {
            let (commands, s) = critical_reference(design);
            let paths = s.timing.get().unwrap().result.as_ref().unwrap();
            assert!(!paths.is_empty());
            let (mut resolved, mut unresolved) = (0, 0);
            let mut reasons = std::collections::BTreeMap::new();
            let view = Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 898.0));
            let fit = (view.width() / s.fabric.size()[0]).min(view.height() / s.fabric.size()[1]);
            for (i, path) in paths.iter().enumerate() {
                // The original path is parsed from this build's nextpnr.log;
                // another machine's seed can produce a different hop count.
                assert_eq!(path.hops.len(), path.path.hops.len());
                assert_eq!(path.hops.last().unwrap().hop.total, path.path.total());
                for (hop, expected) in path.hops.iter().zip(&path.path.hops) {
                    assert_eq!(hop.hop.source, expected.source);
                    assert_eq!(hop.hop.sink, expected.sink);
                    assert_eq!(hop.hop.kind, expected.kind);
                    assert_eq!(hop.hop.total, expected.total);
                    if hop.hop.kind == HopKind::Routing {
                        if let Some(reason) = hop.route_not_shown() {
                            assert!(hop.hop.route_error.is_some(), "missing route explanation");
                            unresolved += 1;
                            *reasons.entry(reason).or_insert(0) += 1;
                            println!("# {design} path={i} unresolved {}: {reason}", hop.name);
                        } else {
                            resolved += 1;
                        }
                    }
                }
                let mut expected = Rect::NOTHING;
                let mut piece_count = 0;
                for hop in &path.hops {
                    for piece in &hop.hop.route {
                        let piece = &s.overlay.pieces[piece.piece as usize];
                        expected.extend_with(Pos2::from(piece.a));
                        expected.extend_with(Pos2::from(piece.b));
                        piece_count += 1;
                    }
                    if hop.hop.kind != HopKind::Routing || hop.route_not_shown().is_some() {
                        for site in &hop.sites {
                            if let Some(rect) = timing_site(&s.fabric, site) {
                                expected = expected.union(rect);
                            }
                        }
                    }
                }
                for dark in [false, true] {
                    let mut at_fit = None;
                    for zoom in [1.0, 4.0, 19.0] {
                        let cell = fit * zoom;
                        let origin = view.center() - expected.center().to_vec2() * cell;
                        let screen = |p| origin + Vec2::from(p) * cell;
                        let ctx = egui::Context::default();
                        ctx.set_visuals(if dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                        // First use an actual fixed-size viewport: off-screen badges
                        // must not add fake segments as the camera zooms or pans.
                        let out = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(view),
                                ..Default::default()
                            },
                            |ui| {
                                draw_critical(
                                    ui.painter(),
                                    view,
                                    &s.fabric,
                                    &s.overlay,
                                    path,
                                    &screen,
                                    None,
                                );
                            },
                        );
                        let numbers = critical_badge_numbers(&out, view);
                        let visible =
                            critical_on_screen_hops(&s.fabric, &s.overlay, path, view, &screen);
                        assert!(numbers.iter().all(|n| visible.contains(n)));
                        if zoom >= CRITICAL_ALL_BADGES_ZOOM {
                            assert_eq!(numbers, visible);
                        }
                        critical_geometry(&out, cell, origin);
                        // Include the projected die for the geometry pass so viewport
                        // culling cannot discard a path's extrema at high zoom.
                        let uncropped = view.union(Rect::from_min_max(
                            screen([0.0; 2]),
                            screen(s.fabric.size()),
                        ));
                        let out = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(uncropped),
                                ..Default::default()
                            },
                            |ui| {
                                draw_critical(
                                    ui.painter(),
                                    uncropped,
                                    &s.fabric,
                                    &s.overlay,
                                    path,
                                    &screen,
                                    None,
                                );
                            },
                        );
                        let (bounds, pieces) = critical_geometry(&out, cell, origin);
                        assert_eq!(pieces, piece_count, "no coarsened or fallback wire spans");
                        assert!(
                            bounds.min.distance(expected.min) < 0.001
                                && bounds.max.distance(expected.max) < 0.001,
                            "{design} path={i} zoom={zoom}: {bounds:?} != {expected:?}"
                        );
                        let fit_bounds = *at_fit.get_or_insert(bounds);
                        assert!(
                            bounds.min.distance(fit_bounds.min) < 0.001
                                && bounds.max.distance(fit_bounds.max) < 0.001
                        );
                        for hop in path
                            .hops
                            .iter()
                            .filter(|hop| hop.hop.kind == HopKind::Routing)
                        {
                            let single = MappedPath {
                                hops: vec![hop.clone()],
                                ..(**path).clone()
                            };
                            let out = ctx.run_ui(
                                egui::RawInput {
                                    screen_rect: Some(uncropped),
                                    ..Default::default()
                                },
                                |ui| {
                                    draw_critical(
                                        ui.painter(),
                                        uncropped,
                                        &s.fabric,
                                        &s.overlay,
                                        &single,
                                        &screen,
                                        None,
                                    )
                                },
                            );
                            assert_critical_hop_chain(
                                &out, hop, &s.fabric, &s.overlay, cell, origin,
                            );
                        }
                        println!("# {design} path={i} zoom={zoom} dark={dark} bbox={bounds:?} width=2.5px pieces={pieces}");
                    }
                }
            }
            println!("# {design} routing hops: {resolved} resolved, {unresolved} unresolved; {reasons:?}");
            drop(commands);
        }
    }

    fn critical_reference(design: &str) -> (std::sync::mpsc::Sender<Command>, Arc<live::Static>) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if let Some(s) = sh.ready.as_ref().filter(|s| s.timing.get().is_some()) {
                break Arc::clone(s);
            }
            assert!(start.elapsed() < Duration::from_secs(180));
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        };
        (commands, s)
    }

    /// Real routed data and CPU drawing only: this never creates a window.
    #[test]
    #[ignore = "needs local reference designs, Trellis and clang"]
    fn critical_blinky_routes_trim_fanout_to_a_connected_chain() {
        let (commands, s) = critical_reference("blinky_cycle");
        let paths = s.timing.get().unwrap().result.as_ref().unwrap();
        let mut trimmed_fanout = false;
        let mut routed = 0;
        for path in paths {
            let expected = &path.path;
            assert_eq!(path.hops.len(), expected.hops.len());
            for (hop, expected) in path.hops.iter().zip(&expected.hops) {
                assert_eq!(hop.hop.source, expected.source);
                assert_eq!(hop.hop.sink, expected.sink);
                if matches!(
                    expected.net.as_deref(),
                    Some("count[0]" | "count[7]" | "led_TRELLIS_FF_Q_DI")
                ) {
                    assert!(!hop.hop.route.is_empty(), "{}", hop.name);
                }
                if hop.hop.route.is_empty() {
                    continue;
                }
                routed += 1;
                let mut points = Vec::new();
                for route in &hop.hop.route {
                    assert!(hop
                        .nets
                        .iter()
                        .any(|&n| s.overlay.pieces_of(n).contains(&route.piece)));
                    let p = &s.overlay.pieces[route.piece as usize];
                    assert!(p.route);
                    let (a, b) = if route.reverse {
                        (p.b, p.a)
                    } else {
                        (p.a, p.b)
                    };
                    if let Some(&last) = points.last() {
                        assert!(Pos2::from(a).distance(last) < 0.001, "{}", hop.name);
                    }
                    points.push(Pos2::from(a));
                    points.push(Pos2::from(b));
                }
                let start = timing_site(&s.fabric, &hop.sites[0]).unwrap();
                let end = timing_site(&s.fabric, hop.sites.last().unwrap()).unwrap();
                assert!(start.expand(0.001).contains(points[0]), "{}", hop.name);
                assert!(
                    end.expand(0.001).contains(*points.last().unwrap()),
                    "{}",
                    hop.name
                );
                for &net in &hop.nets {
                    let other_sinks: Vec<_> = s
                        .overlay
                        .fed_of(net)
                        .iter()
                        .filter(|&&p| !end.expand(0.001).contains(Pos2::from(p)))
                        // Carry pins also mark pass-through points on the
                        // chain. Only terminal pins are other branches.
                        .filter(|&&p| {
                            s.overlay
                                .pieces_of(net)
                                .iter()
                                .filter(|&&i| {
                                    let piece = &s.overlay.pieces[i as usize];
                                    piece.route
                                        && [piece.a, piece.b]
                                            .into_iter()
                                            .any(|q| Pos2::from(q).distance(Pos2::from(p)) < 0.001)
                                })
                                .count()
                                == 1
                        })
                        .collect();
                    if !other_sinks.is_empty() {
                        assert!(
                            other_sinks.iter().all(|&&p| points
                                .iter()
                                .all(|&q| q.distance(Pos2::from(p)) > 0.001)),
                            "{} includes another sink's branch",
                            hop.name
                        );
                        assert!(hop.hop.route.len() < s.overlay.pieces_of(net).len());
                        trimmed_fanout = true;
                    }
                }
            }
        }
        assert!(
            routed >= 3,
            "expected the routed FF-to-LUT, LUT-to-FF and FF-to-pad connections"
        );
        assert!(
            trimmed_fanout,
            "the blinky counter net must omit its other sinks"
        );
        drop(commands);
    }

    /// Real routed data and CPU drawing only: this never creates a window.
    #[test]
    #[ignore = "needs local reference designs, Trellis and clang"]
    fn critical_path_draws_without_a_window() {
        let design = std::env::var("ECP5_PROBE_DESIGN").unwrap_or_else(|_| "blinky_cycle".into());
        let (commands, s) = critical_reference(&design);
        let path = s.timing.get().unwrap().result.as_ref().unwrap()[0].clone();
        let values = vec![2; s.overlay.values.len()];
        for dark in [false, true] {
            for size in [Vec2::new(1000.0, 700.0), Vec2::new(1600.0, 1000.0)] {
                let ctx = egui::Context::default();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut view = FabricView::new(None, None, (0.5, 0.5));
                let frame = |view: &mut FabricView| {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                view.show(
                                    ui,
                                    &s.fabric,
                                    Some((&s.overlay, &values)),
                                    Some(&s.net_names),
                                )
                            });
                        },
                    )
                };
                frame(&mut view);
                frame(&mut view);
                let off_ms = view.draw_ms;
                let before = view.map_rect;
                view.critical = Some(Arc::clone(&path));
                let out = frame(&mut view);
                assert_eq!(before, view.map_rect);
                let purple = out
                    .shapes
                    .iter()
                    .filter(|shape| match &shape.shape {
                        egui::Shape::LineSegment { stroke, .. } => is_critical(stroke.color),
                        egui::Shape::Rect(rect) => is_critical(rect.stroke.color),
                        _ => false,
                    })
                    .count();
                assert!(purple > 0);
                critical_badge_numbers(&out, view.map_rect);
                println!(
                    "# critical draw {design} {size:?} dark={dark} off={off_ms:.3}ms on={:.3}ms",
                    view.draw_ms
                );
                view.centre_hop(&path.hops[0]);
                let out = frame(&mut view);
                assert!(
                    critical_badge_numbers(&out, view.map_rect).contains(&1),
                    "the focused hop has a badge"
                );
                assert!(view.focus_hop.is_none());
                let centre = timing_site(&s.fabric, &path.hops[0].sites[0])
                    .unwrap()
                    .center();
                let screen = view.map_rect.min + view.offset + centre.to_vec2() * view.cell;
                assert!(screen.distance(view.map_rect.center()) < 2.0);
                let mut fallback = (*path).clone();
                for hop in &mut fallback.hops {
                    hop.nets.clear();
                    hop.hop.route.clear();
                }
                view.fit();
                view.critical = Some(Arc::new(fallback));
                let out = frame(&mut view);
                assert!(!out.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if is_critical(stroke.color))));
                assert!(out.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if is_critical(rect.stroke.color))));
            }
        }
        drop(commands);
    }

    #[test]
    fn the_watch_menu_requests_the_right_clicked_net() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, egui::ThemePreference::Dark);
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        let names = HashMap::from([(7, vec!["data[2]".to_string()])]);
        let mut text = String::new();
        let mut first = true;
        let mut frame = |view: &mut FabricView, events: Vec<egui::Event>, net| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let field = ui.text_edit_singleline(&mut text);
                    if first {
                        field.request_focus();
                        first = false;
                    }
                    let response =
                        ui.allocate_response(Vec2::new(700.0, 400.0), Sense::click_and_drag());
                    view.hover_net = net;
                    view.watch_menu(&response, Some(&names));
                },
            )
        };
        let button = |pos, button, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&mut view, Vec::new(), None);
        assert!(ctx.egui_wants_keyboard_input());
        let point = Pos2::new(100.0, 100.0);
        for pressed in [true, false] {
            frame(
                &mut view,
                button(point, egui::PointerButton::Secondary, pressed),
                Some(7),
            );
        }
        assert_eq!(view.menu_net, Some((7, "data[2]".to_string())));
        assert!(view.watch_net.is_none());
        let mut target = None;
        for _ in 0..3 {
            let out = frame(&mut view, Vec::new(), None);
            target = out.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == "Watch this net (data[2])" => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            });
            if target.is_some() {
                break;
            }
        }
        let target = target.expect("the watch menu button drawn");
        for pressed in [true, false] {
            frame(
                &mut view,
                button(target, egui::PointerButton::Primary, pressed),
                Some(99),
            );
        }
        assert_eq!(view.watch_net.take(), Some(7));
        assert!(view.watch_net.take().is_none());
    }

    #[test]
    fn map_source_menu_explains_missing_index_files_and_net_locations() {
        use ecp5_emu::sources::{Collected, Files};
        let top = serde_json::json!({"netnames": {
            "other": {"bits": [8], "attributes": {"src": "rtl/top.v:13"}}
        }});
        let files = Files::project("project".into(), ["rtl/top.v".into()]);
        let populated = Arc::new(Collected::from_top(&top, &files).build(|_| vec![8]));
        let missing =
            std::env::temp_dir().join(format!("ecp5-no-source-files-{}", std::process::id()));
        let beside = Files::beside(&missing.join("top_routed.json"));
        let empty = Arc::new(Collected::from_top(&top, &beside).build(|_| vec![8]));
        assert!(populated.locations > 0);
        assert_eq!(empty.locations, 0);
        for (project, vhdl) in [(false, false), (true, false), (true, true)] {
            for state in 0..3 {
                for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
                    let ctx = egui::Context::default();
                    crate::theme::install(&ctx, theme);
                    let mut view = FabricView::new(None, None, (0.5, 0.5));
                    view.project_sources = project;
                    view.vhdl_sources = vhdl;
                    view.sources = match state {
                        0 => None,
                        1 => Some(Arc::clone(&empty)),
                        _ => Some(Arc::clone(&populated)),
                    };
                    let reason = match state {
                        _ if vhdl => ecp5_emu::flow::report::VHDL_SOURCE_LIMIT,
                        0 => "design has no source index yet",
                        1 if project => "no project source matched the build's src attributes",
                        1 => "no source files found for this design (looked beside its top.config)",
                        _ => "no source line for this net (synthesis left it only a library location)",
                    };
                    let frame = |view: &mut FabricView, events| {
                        ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(
                                    Pos2::ZERO,
                                    Vec2::new(1000.0, 500.0),
                                )),
                                events,
                                ..Default::default()
                            },
                            |ui| {
                                let response = ui.allocate_response(
                                    Vec2::new(900.0, 400.0),
                                    Sense::click_and_drag(),
                                );
                                view.hover_net = Some(7);
                                view.watch_menu(&response, None);
                            },
                        )
                    };
                    let button = |pos, button, pressed| {
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    };
                    frame(&mut view, vec![]);
                    let point = Pos2::new(100.0, 100.0);
                    for pressed in [true, false] {
                        frame(
                            &mut view,
                            button(point, egui::PointerButton::Secondary, pressed),
                        );
                    }
                    frame(&mut view, vec![]);
                    let out = frame(&mut view, vec![]);
                    let text_at = |text| {
                        out.shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::Shape::Text(t) if t.galley.text() == text => {
                                    Some(Rect::from_min_size(t.pos, t.galley.size()).center())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("missing menu text: {text}"))
                    };
                    text_at(reason);
                    let target = text_at("Show in source");
                    for pressed in [true, false] {
                        frame(
                            &mut view,
                            button(target, egui::PointerButton::Primary, pressed),
                        );
                    }
                    assert!(
                        view.show_source.is_none(),
                        "disabled source action cannot dispatch"
                    );
                    assert!(view.watch_net.is_none());
                }
            }
        }
    }

    #[test]
    fn the_source_menu_shows_a_relative_location_and_keeps_the_jump_target() {
        let ctx = egui::Context::default();
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        let top = serde_json::json!({"netnames":{"data":{"bits":[1],"attributes":{"src":"top.v:13.4-13.8|/share/yosys/map.v:99"}}}});
        let files = ecp5_emu::sources::Files::project("project".into(), ["top.v".into()]);
        view.sources = Some(Arc::new(
            ecp5_emu::sources::Collected::from_top(&top, &files).build(|_| vec![7]),
        ));
        let frame = |view: &mut FabricView, events, net| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response =
                        ui.allocate_response(Vec2::new(700.0, 400.0), Sense::click_and_drag());
                    view.hover_net = net;
                    view.watch_menu(&response, None);
                },
            )
        };
        let button = |pos, button, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&mut view, Vec::new(), None);
        for pressed in [true, false] {
            frame(
                &mut view,
                button(
                    Pos2::new(100.0, 100.0),
                    egui::PointerButton::Secondary,
                    pressed,
                ),
                Some(7),
            );
        }
        let mut target = None;
        for _ in 0..3 {
            let out = frame(&mut view, Vec::new(), None);
            target = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Show in source (top.v:13)" => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            });
            if target.is_some() {
                break;
            }
        }
        let target = target.expect("Show in source menu entry");
        for pressed in [true, false] {
            frame(
                &mut view,
                button(target, egui::PointerButton::Primary, pressed),
                Some(99),
            );
        }
        let (net, locations, span) = view.show_source.take().unwrap();
        assert_eq!(net, "net 7 (data)");
        assert_eq!(locations.len(), 1);
        assert_eq!(span.label(), "top.v:13");
        assert_eq!(span.column, Some(4));
        assert!(view.show_source.is_none());
    }

    #[test]
    fn source_menu_keeps_the_assigned_default_and_offers_labelled_declarations() {
        let top = serde_json::json!({"netnames":{
            "cpu.reg_pc[8]":{"bits":[8],"attributes":{"src":"picorv32.v:159.13-159.19"}},
            "z_alias":{"bits":[8],"attributes":{"src":"picorv32.v:159.20-159.28"}}
        },"cells":{
            "ff":{"attributes":{"src":"picorv32.v:1374.2-1946.5"},"port_directions":{"Q":"output"},"connections":{"Q":[8]}}
        }});
        let files = ecp5_emu::sources::Files::project("project".into(), ["picorv32.v".into()]);
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        view.sources = Some(Arc::new(
            ecp5_emu::sources::Collected::from_top(&top, &files).build(|_| vec![21826]),
        ));
        let (target, locations) = view.source_target(Some(21826), None);
        assert_eq!(target, "net 21826 (cpu.reg_pc[8] +1)");
        let text_rect = |out: &egui::FullOutput, text: &str| {
            out.shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == text => {
                        Some(Rect::from_min_size(t.pos, t.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing menu text {text}"))
        };
        let button = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        for declaration in [false, true] {
            let ctx = egui::Context::default();
            let frame = |events| {
                let mut chosen = None;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        chosen = source_menu(ui, &locations, None);
                    },
                );
                (out, chosen)
            };
            let (out, _) = frame(Vec::new());
            let target = if declaration {
                let menu = text_rect(&out, "Source locations").center();
                frame(button(menu, true));
                frame(button(menu, false));
                let (out, _) = frame(Vec::new());
                let assigned = text_rect(&out, "assigned in picorv32.v:1374");
                let declared = text_rect(&out, "declared in picorv32.v:159");
                assert!(assigned.min.y < declared.min.y);
                declared.center()
            } else {
                text_rect(&out, "Show in source (picorv32.v:1374)").center()
            };
            frame(button(target, true));
            let (_, chosen) = frame(button(target, false));
            assert_eq!(chosen.unwrap().line, if declaration { 159 } else { 1374 });
        }
    }

    #[test]
    #[ignore]
    fn a_large_design_draws_and_flashes() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/reference/life");
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
        let s = loop {
            {
                let sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{}", sh.error.as_ref().unwrap());
                if let Some(r) = &sh.ready {
                    break Arc::clone(r);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let o = &s.overlay;
        // The values once paused past `cycle`, read with the cycle.
        let lit = |shared: &Mutex<Shared>, cycle: u64| loop {
            {
                let sh = shared.lock().unwrap();
                if !sh.running && sh.cycles >= cycle && sh.lit.len() == o.values.len() {
                    break sh.lit.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            ..Default::default()
        };
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        // A frame as the window draws it: how long it took, its shapes,
        // what is orange, and the image of the wires it sent, if any.
        let frame = |view: &mut FabricView, values: &[u8]| {
            let t = Instant::now();
            let out = ctx.run_ui(input(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    view.show(ui, &s.fabric, Some((o, values)), Some(&s.net_names))
                });
            });
            let built = t.elapsed();
            let image = sent(&out);
            let n = out.shapes.len();
            // Orange quads, and orange pixels of the wires' image where a
            // large design from afar draws them as one.
            let orange = orange(&out.shapes) + orange_pixels(&out.textures_delta);
            let meshes = ctx.tessellate(out.shapes, 1.0);
            let vertices: usize = meshes
                .iter()
                .map(|m| match &m.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                    _ => 0,
                })
                .sum();
            println!("  built {built:?}, with tessellation {:?}; {n} shapes, {vertices} vertices, {orange} orange", t.elapsed());
            (orange, image)
        };
        let step = Motion::of(false, None);
        let before = lit(&shared, 0);
        view.update(&before, &[], step);
        println!("frames:");
        for _ in 0..3 {
            frame(&mut view, &before);
        }
        // Moving the view: the wires are found and drawn again.
        println!("pan, then zoom:");
        for k in 0..6 {
            if k < 3 {
                view.offset += Vec2::new(5.0, 3.0);
            } else {
                view.cell *= 1.05;
            }
            frame(&mut view, &before);
        }
        let cycles = shared.lock().unwrap().cycles;
        commands.send(Command::Step(1)).unwrap();
        let after = lit(&shared, cycles + 1);
        let changed: Vec<bool> = before.iter().zip(&after).map(|(a, b)| a != b).collect();
        // A step: the change, frame after frame, while it lasts.
        view.update(&after, &[], step);
        let mut seen = Vec::new();
        while view.animating() {
            seen.push(frame(&mut view, &after).0);
        }
        let shown = seen.iter().filter(|&&o| o > 0).count();
        println!("a step showed in {shown} of {} frames", seen.len());
        assert!(shown >= 6, "a step shows in {shown} frames");

        // Where each piece of a wire that changed is lit, from afar to near,
        // the image drawn whatever the number of pieces: the pixel at its
        // middle (or any within `around` of it: a dim change is drawn
        // narrower, and may miss that pixel), with the share of its net there.
        let share = |view: &FabricView, i: u32, v: u32| {
            let p = &o.pieces[i as usize];
            let length = view.lengths.borrow().1[o.nets[v as usize] as usize];
            let u = |d: f32| {
                if d.is_finite() && length > 0.0 {
                    (d / length).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            };
            (u(p.d0) + u(p.d1)) / 2.0
        };
        let pieces = |view: &FabricView,
                      image: &egui::ColorImage,
                      lit: fn(Color32) -> bool,
                      around: i64|
         -> Vec<(f32, bool)> {
            let sight = view.sight.borrow();
            let (rect, ..) = sight.key.expect("the wires in sight");
            let [w, h] = image.size;
            sight
                .pieces
                .iter()
                .flatten()
                .filter(|q| changed[q.v as usize])
                .filter_map(|q| {
                    let m = ((q.a + q.b.to_vec2()) / 2.0) - rect.min.to_vec2();
                    (m.x >= 0.0 && m.y >= 0.0 && m.x < w as f32 && m.y < h as f32).then(|| {
                        (
                            share(view, q.i, q.v),
                            (-around..=around)
                                .flat_map(|dy| (-around..=around).map(move |dx| (dx, dy)))
                                .any(|(dx, dy)| {
                                    let (x, y) = (m.x as i64 + dx, m.y as i64 + dy);
                                    (0..w as i64).contains(&x)
                                        && (0..h as i64).contains(&y)
                                        && lit(image.pixels[y as usize * w + x as usize])
                                }),
                        )
                    })
                })
                .collect()
        };
        view.raster_from = 0;
        let toggled: Vec<u8> = changed.iter().map(|&c| u8::from(c)).collect();

        // As fast as it goes: every piece of a wire that changed glows, a
        // quiet wire's (one change) dimly, a busy one's (a change in every
        // sample) fully.
        view.fitted = false;
        frame(&mut view, &after);
        let fast = Motion::of(true, None);
        for (busy, changes, lit, around) in [
            ("quiet", 1, tinted as fn(Color32) -> bool, 1),
            ("busy", 20, near, 0),
        ] {
            for _ in 0..changes {
                view.update(&after, &toggled, fast);
            }
            let image = frame(&mut view, &after).1.expect("the changes drawn");
            let all = pieces(&view, &image, lit, around);
            let dark = all.iter().filter(|p| !p.1).count();
            println!(
                "as fast as it goes, {busy}: {} pieces of the wires that changed glow, {dark} do not",
                all.len() - dark
            );
            assert!(all.len() > 1000);
            assert_eq!(
                dark, 0,
                "{busy}: {dark} pieces of wires that changed do not glow"
            );
        }

        // Spreading, from afar to near: half way through, what the change
        // has reached glows and what it has not does not, at any zoom.
        let fit = (view.cell, view.offset);
        let rect = view.sight.borrow().key.expect("the wires in sight").0;
        let (min, max) = o
            .sites
            .iter()
            .fold(([f32::MAX; 2], [f32::MIN; 2]), |(a, b), s| {
                (
                    [a[0].min(s.min[0]), a[1].min(s.min[1])],
                    [b[0].max(s.max[0]), b[1].max(s.max[1])],
                )
            });
        let centre = Vec2::new((min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0);
        let spread = Motion {
            spread: 1.0,
            fade: 0.6,
        };
        // Each change on its own: the one before (the same wires) out first,
        // else it would still glow where it reached.
        let settle = |view: &FabricView| {
            while view.animating() {
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let mut now = after.clone();
        for zoom in [1.0, 4.0, 16.0, 64.0] {
            view.cell = fit.0 * zoom;
            view.offset = if zoom == 1.0 {
                fit.1
            } else {
                rect.size() / 2.0 - centre * view.cell
            };
            settle(&view);
            frame(&mut view, &now);
            now = if now == after {
                before.clone()
            } else {
                after.clone()
            };
            view.update(&now, &[], spread);
            std::thread::sleep(Duration::from_millis(500));
            let image = frame(&mut view, &now).1.expect("the changes drawn");
            let all = pieces(&view, &image, near, 0);
            let reached: Vec<bool> = all
                .iter()
                .filter(|p| (0.3..0.45).contains(&p.0))
                .map(|p| p.1)
                .collect();
            let ahead: Vec<bool> = all.iter().filter(|p| p.0 >= 0.7).map(|p| p.1).collect();
            let lit = |v: &[bool]| v.iter().filter(|&&l| l).count();
            println!(
                "x{zoom}: half way, {} of {} pieces reached glow, {} of {} ahead",
                lit(&reached),
                reached.len(),
                lit(&ahead),
                ahead.len()
            );
            assert!(
                reached.len() >= 20,
                "x{zoom}: only {} pieces reached in sight",
                reached.len()
            );
            // What the drawing makes of each piece, whatever its pixels: every
            // piece the change has reached lit, none it has not.
            let (mut on, mut off) = ((0, 0), (0, 0));
            {
                struct Count;
                impl Sink for Count {
                    fn add(&mut self, _: Pos2, _: Pos2, _: f32, _: Color32) {}
                }
                let lengths = view.lengths.borrow();
                let paint = Paint {
                    o,
                    changes: &view.changes,
                    lengths: &lengths.1,
                    now: view.changes.now(),
                    values: false,
                    width: 1.3,
                    drawn: None,
                };
                let sight = view.sight.borrow();
                for q in sight
                    .pieces
                    .iter()
                    .flatten()
                    .filter(|q| changed[q.v as usize])
                {
                    let p = &o.pieces[q.i as usize];
                    let (u0, u1) = (paint.share(q.v, p.d0), paint.share(q.v, p.d1));
                    let lit = paint.change(&mut Count, q);
                    if u0.min(u1) >= 0.1 && u0.max(u1) <= 0.45 {
                        on.0 += 1;
                        on.1 += usize::from(lit);
                    } else if u0.min(u1) >= 0.6 {
                        off.0 += 1;
                        off.1 += usize::from(lit);
                    }
                }
            }
            println!(
                "  drawn: {} of {} pieces reached lit, {} of {} ahead",
                on.1, on.0, off.1, off.0
            );
            // On the screen, where the image is: what was reached glows.
            // Not that nothing ahead does: the tracks lie a pixel or so apart
            // even close up, and a lit one, drawn wider, covers its
            // neighbour.
            assert!(
                lit(&reached) * 100 >= reached.len() * 80,
                "x{zoom}: what the change reached does not glow"
            );
        }

        // A clock at 4 cycles a second changes every 125 ms. Its 100 ms
        // travelling front must stand apart from the shortened trailing
        // glow, rather than leaving the entire net continuously lit.
        view.cell = fit.0;
        view.offset = fit.1;
        settle(&view);
        let four = Motion::of(true, Some(4.0));
        now = if now == after {
            before.clone()
        } else {
            after.clone()
        };
        view.update(&now, &[], four);
        std::thread::sleep(Duration::from_millis(125));
        now = if now == after {
            before.clone()
        } else {
            after.clone()
        };
        view.update(&now, &[], four);
        for &v in view.changes.found() {
            let record = view.changes.records()[v];
            let at = f64::from(record.changes[1][0]);
            let fade = f64::from(record.changes[1][2]);
            assert!(fade < 0.1, "the clock's trail stayed long: {fade}");
            let now = at + 0.09;
            assert!(view.changes.glow(v as u32, 0.9, now + 1e-6) > 0.99);
            assert!(view.changes.glow(v as u32, 0.0, now) < 0.3);
        }
    }

    /// A script's `view` and `sweep`: where the camera goes, and that a
    /// sweep's views cover the die, each next to the one before.
    #[test]
    fn views_aim_and_a_sweep_covers_the_die() {
        let die = Vec2::new(1260.0, 900.0);
        let map = Vec2::new(1000.0, 800.0);
        let fit = (map.x / die.x).min(map.y / die.y);
        // The middle at zoom 1 is the fit, the die centred.
        let (cell, offset) = aimed(die, map, (0.5, 0.5, 1.0)).unwrap();
        assert!((cell - fit).abs() < 1e-4);
        assert!((offset - (map - die * fit) / 2.0).length() < 1e-3);
        // A corner at zoom 4: that corner in the middle of the map.
        let (cell, offset) = aimed(die, map, (0.0, 0.0, 4.0)).unwrap();
        assert!((cell - 4.0 * fit).abs() < 1e-4);
        assert!((offset - map / 2.0).length() < 1e-3);
        // No die or map yet, or no zoom: nowhere to aim.
        assert!(aimed(Vec2::ZERO, map, (0.5, 0.5, 1.0)).is_none());
        assert!(aimed(die, Vec2::ZERO, (0.5, 0.5, 1.0)).is_none());
        assert!(aimed(die, map, (0.5, 0.5, 0.0)).is_none());
        assert!(sweep(Vec2::ZERO, map, 4.0).is_empty());
        // At the fit one view; nearer, enough to cover the die.
        assert_eq!(sweep(die, map, 1.0), [(0.5, 0.5)]);
        for zoom in [2.0, 3.0, 4.0, 6.5, 8.0] {
            let views = sweep(die, map, zoom);
            let cell = aimed(die, map, (0.5, 0.5, zoom)).unwrap().0;
            let half = map / cell / 2.0;
            let shows = |(x, y): (f32, f32), p: Vec2| {
                let centre = Vec2::new(x * die.x, y * die.y);
                (p.x - centre.x).abs() <= half.x + 1e-3 && (p.y - centre.y).abs() <= half.y + 1e-3
            };
            for i in 0..=40 {
                for j in 0..=40 {
                    let p = Vec2::new(i as f32 / 40.0 * die.x, j as f32 / 40.0 * die.y);
                    assert!(
                        views.iter().any(|&v| shows(v, p)),
                        "zoom {zoom}: {p:?} in no view"
                    );
                }
            }
            // No more views than it takes: one fewer across or down would not do.
            let across = views.iter().filter(|v| v.1 == views[0].1).count();
            let down = views.len() / across;
            assert!((across as f32 - 1.0) * 2.0 * half.x < die.x - 1e-3);
            assert!((down as f32 - 1.0) * 2.0 * half.y < die.y - 1e-3);
            // Each view next to the one before: a step across or down.
            for w in views.windows(2) {
                let (dx, dy) = ((w[1].0 - w[0].0).abs(), (w[1].1 - w[0].1).abs());
                assert!(
                    (dx > 0.0) != (dy > 0.0) && dx <= 1.0 / across as f32 + 1e-4,
                    "zoom {zoom}: {w:?}"
                );
            }
        }
    }

    #[test]
    fn legend_stays_one_row_at_1500_points() {
        let sample = LegendCounts {
            drawn: 48_120,
            hidden: 312,
            routes: 50_000,
            sites: 12_345,
        };
        for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, theme);
            for counts in [None, Some(sample)] {
                for values in [false, true] {
                    for wires in [false, true] {
                        let input = egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                Vec2::new(1500.0, 980.0),
                            )),
                            ..Default::default()
                        };
                        let mut legend_rect = Rect::NOTHING;
                        let mut one_row_h = 0.0_f32;
                        let _ = ctx.run_ui(input, |ui| {
                            egui::Panel::left("board")
                                .default_size(600.0)
                                .show(ui, |_| {});
                            egui::Panel::bottom("tile")
                                .default_size(120.0)
                                .show(ui, |ui| {
                                    one_row_h = ui
                                        .horizontal(|ui| {
                                            let _ = ui.small_button("?");
                                        })
                                        .response
                                        .rect
                                        .height();
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            legend_rect = ui
                                                .scope(|ui| {
                                                    legend_with_counts(ui, counts, values, wires);
                                                })
                                                .response
                                                .rect;
                                        });
                                });
                        });
                        assert!(
                            legend_rect.height() <= one_row_h + 1.0,
                            "legend wrapped (theme={theme:?}, counts={}, values={values}, wires={wires}): height {} > {one_row_h}",
                            counts.is_some(),
                            legend_rect.height()
                        );
                        assert!(
                            legend_rect.max.x <= 1500.0,
                            "legend overflowed 1500pt: {}",
                            legend_rect.max.x
                        );
                    }
                }
            }
        }
        // Keep the full explanations verbatim in the "?" tooltip.
        let with_wires = legend_help(Some(sample), false, true);
        assert!(with_wires.contains("used by the design"));
        assert!(with_wires.contains("a change spreading from its driver"));
        assert!(with_wires.contains(
            "47808 of the plan's 50000 routing transfers drawn as arcs of the fabric (312 left out by the fabric); 12345 LUTs and flip-flops."
        ));
        assert!(
            with_wires.contains("A 0 or 1 in a small box: a CIB ties the wire to that constant.")
        );
        let without_wires = legend_help(None, true, false);
        assert!(without_wires.contains(
            "The tiles and their sites, without the wires: those the design uses lit, and a change of what they drive flashes."
        ));
    }
}
