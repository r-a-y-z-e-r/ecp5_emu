//! The window: the Design menu (open, recent, demos, close, theme) and the
//! run controls on top, a status line at the bottom (the run, its speed,
//! fast mode's settings, the window's frames a second), the board on the
//! left (the ULX3S in 3D,
//! as tall as the window, its LEDs lit and its buttons to press,
//! src/board_view.rs; its UART console under it), the die in the middle
//! (its whole fabric with the design's wires, or the grid: its tiles and
//! sites without the wires), the
//! inspector of a site clicked on the map beside it, and the selected
//! position below.
#[path = "loading.rs"]
pub(crate) mod loading;
#[cfg(test)]
#[path = "open_tests.rs"]
mod open_tests;
#[path = "toolbar.rs"]
pub(crate) mod toolbar;

use crate::{
    board_view::{BoardHdmi, BoardInputs, BoardScreen, BoardView},
    changes::Motion,
    fabric_view::{self, FabricView},
    inspector::{self, Inspector},
    monitor_view::{Monitor, Panel},
    start::{self, display_paths::DisplayPaths, Action, Demos, Preferences},
    theme,
    trace::{self, Shown, Trace},
};
use ecp5_emu::{
    board,
    fabric::TILE_W,
    live::{self, Class, Command, Shared},
    sources::{self, SourceLocation, Span},
    timing_path::{MappedHop, MappedPath, Published},
};
use eframe::egui::{self, RichText};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

/// How the die is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Map {
    /// Every wire and site of the fabric, the design's in colour.
    Fabric,
    /// The tiles and their sites without the wires, those in use lit.
    Grid,
}

/// Lever positions requested by the window, until the runner reports them.
#[derive(Default)]
struct SwitchRequests {
    requested: [Option<bool>; 4],
}

impl SwitchRequests {
    fn drawn(&self, reported: [bool; 4]) -> [bool; 4] {
        std::array::from_fn(|i| self.requested[i].unwrap_or(reported[i]))
    }

    fn report(&mut self, reported: [bool; 4]) {
        for (request, on) in self.requested.iter_mut().zip(reported) {
            if *request == Some(on) {
                *request = None;
            }
        }
    }

    fn command(&mut self, command: &Command) {
        match *command {
            Command::Switch(i, on) => self.requested[i] = Some(on),
            // Both the cycle field and the waves seek with GoTo.
            Command::Back(_) | Command::GoTo(_) => *self = Self::default(),
            _ => {}
        }
    }
}

pub fn class_name(class: Class) -> &'static str {
    match class {
        Class::Logic => "logic (PLC)",
        Class::Memory => "block RAM (EBR)",
        Class::Dsp => "DSP",
        Class::Io => "IO (PIO/PIC)",
        Class::Clock => "clocks and system",
        Class::Routing => "routing (CIB)",
        Class::Filler => "filler",
    }
}

pub struct App {
    pub(crate) new_file_available: bool,
    pub(crate) new_file_requested: bool,
    pub(crate) creation_command: Option<crate::new_project::Command>,
    pub(crate) creation_busy: bool,
    pub(crate) creation_available: bool,
    pub(crate) project_request: Option<std::path::PathBuf>,
    pub(crate) preferred_rate: Option<Option<f64>>,
    title: String,
    shared: Arc<Mutex<Shared>>,
    commands: Option<mpsc::Sender<Command>>,
    preferences: Preferences,
    load_times: start::LoadTimes,
    loading: Option<loading::Loading>,
    demos: Demos,
    browser_error: Option<String>,
    reader: Option<start::reader::Check>,
    /// Creates or refreshes the reader prerequisite for an open.
    pub(crate) reader_check: fn() -> start::reader::Check,
    pending_design: Option<live::Design>,
    open_notes: Vec<String>,
    gpu: bool,
    frames: VecDeque<std::time::Instant>,
    opened_at: std::time::Instant,
    /// The die, with its wires or (the grid) without; one view, so both
    /// show the same place.
    fabric: FabricView,
    /// The site clicked on the map: what it is, how it works, what it does.
    inspector: Inspector,
    map: Map,
    /// The cell of each position, for the fabric's hovered tile, and the
    /// logic tile the design uses there.
    cell_at: HashMap<(u16, u16), usize>,
    logic_at: HashMap<(u16, u16), usize>,
    /// An HDL name whose net to show once the design is loaded.
    pending_net: Option<String>,
    console: String,
    line: String,
    held: [bool; 7],
    debounce: DebounceHint,
    step_leds: StepLedHint,
    /// The board in 3D, and the buttons a script holds down.
    board: BoardView,
    board_uart: BoardUartSplit,
    script_held: [bool; 7],
    /// The switch positions last seen for this design, for the record.
    seen_switches: Option<[bool; 4]>,
    switch_requests: SwitchRequests,
    /// How many cycles a step goes, back or on: a preset, or (none) the
    /// number typed under Custom.
    step_choice: Option<u64>,
    step_text: String,
    /// The rate typed under Custom, when that is chosen.
    rate_custom: bool,
    rate_text: String,
    /// A cycle to go to, as typed.
    goto_text: String,
    /// Commands to give by the second after the design is ready, and when
    /// it was (src/trace.rs).
    script: Vec<(f64, String)>,
    ready_at: Option<std::time::Instant>,
    /// What keeping the record took last frame, milliseconds.
    trace_ms: f64,
    /// Where the script holds the pointer, in points (`point X Y`).
    pointer: Option<egui::Pos2>,
    /// Where the board and the die were last written to the record.
    layout: Option<(egui::Rect, egui::Rect)>,
    layout_evidence: Option<MapLayout>,
    /// The board's HDMI monitor and OLED, each a window of its own.
    monitor: Monitor,
    /// For starting another design's run.
    ctx: egui::Context,
    /// With no design: the start screen to open one from (the window on
    /// its own), or the run's stage (in a project's window, which opens the
    /// design it builds).
    start_screen: bool,
    /// A design the script opens or closes, at the end of the frame.
    scripted_action: Option<Action>,
    /// The wires' values as the window last took them from the run.
    taken: Taken,
    source_values: SourceValues,
    /// The nets watched and their values after every edge, under the map.
    waves: crate::waves::Waves,
    script_pointed: usize,
    source_files: Option<sources::Files>,
    source_root: Option<std::path::PathBuf>,
    paths: DisplayPaths,
    #[cfg(test)]
    defer_native_load: bool,
    #[cfg(test)]
    board_publication: Option<BoardPublication>,
    source_index_recorded: bool,
    probe_notes: Vec<String>,
    source_jump: Option<Span>,
    chip_jump: bool,
    probe_status: Option<String>,
    probed: Vec<u32>,
    watch_id: Option<u64>,
    wave_rows_received: bool,
    waves_drawn: bool,
    script_sources: VecDeque<(f32, f32)>,
    critical: Critical,
    toolbar: toolbar::State,
}

/// The wires' values as the run last published them (src/live.rs
/// `Shared::published`), as the window took them: copied only when the run
/// has published since, so the frames drawn between two publishes (the
/// glow fading, every vsync) take nothing from the run and hold its lock
/// only for a moment.
#[derive(Default)]
struct Taken {
    /// The publish last taken; none: whatever the run has, next frame.
    published: Option<u64>,
    lit: Arc<Vec<u8>>,
    values: Arc<Vec<u8>>,
}

/// The exact publication and LED inputs handed to the board, without a GPU.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct BoardPublication {
    published: u64,
    edges: u64,
    running: bool,
    used: [bool; 8],
    leds: [Option<bool>; 8],
    lit: [bool; 8],
}

impl Taken {
    /// Takes the values `s` holds if the run has published them since the
    /// last taken; whether it had. A frame still holding the last copy
    /// keeps it as it was.
    fn take(&mut self, s: &Shared) -> bool {
        if self.published == Some(s.published) {
            return false;
        }
        self.published = Some(s.published);
        Arc::make_mut(&mut self.lit).clone_from(&s.lit);
        Arc::make_mut(&mut self.values).clone_from(&s.values);
        true
    }
}

/// A seek is released only by a later publication at this design's target.
#[derive(Default)]
struct ValueSeek {
    pending: Option<(Arc<live::Static>, u64, u64)>,
}

impl ValueSeek {
    fn dispatch(&mut self, s: &Shared, command: &Command) {
        let target = match *command {
            Command::Back(n) => s.cycles.saturating_sub(n).max(s.earliest),
            Command::GoTo(n) => n.max(s.earliest),
            _ => return,
        };
        self.pending = s
            .ready
            .as_ref()
            .map(|design| (Arc::clone(design), s.published, target));
    }

    fn seeking(&mut self, s: &Shared) -> bool {
        if let Some((design, publication, target)) = &self.pending {
            if s.ready
                .as_ref()
                .is_none_or(|ready| !Arc::ptr_eq(ready, design))
                || (s.published != *publication && s.cycles == *target)
            {
                self.pending = None;
            }
        }
        self.pending.is_some()
    }
}

#[derive(Clone)]
struct ValueTarget {
    name: String,
    bits: Vec<ValueWire>,
}

#[derive(Clone)]
struct ValueWire {
    name: String,
    index: Option<u32>,
    alternatives: Vec<(u32, Option<usize>)>,
}

enum SourceUnavailable {
    Loading,
    Failed(String),
    Seeking,
    FirstPublication,
    NoNet,
}

#[derive(Default)]
struct SourceValues {
    design: Option<Arc<live::Static>>,
    index: Option<Arc<sources::Index>>,
    mappings: HashMap<(String, String), Vec<ValueTarget>>,
    formatted: HashMap<(String, String), crate::project::editor::ValueView>,
    visible: std::collections::HashSet<(String, String)>,
    seek: ValueSeek,
    last_note: Option<(u64, u64, String)>,
}

impl SourceValues {
    fn bind(&mut self, design: &Arc<live::Static>, index: &Arc<sources::Index>) {
        if self
            .design
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, design))
            || self
                .index
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, index))
        {
            self.design = Some(Arc::clone(design));
            self.index = Some(Arc::clone(index));
            self.mappings.clear();
            self.formatted.clear();
            self.last_note = None;
        }
    }

    /// Called under Shared's mutex: metadata and sparse bytes are coherent.
    /// Mapping and formatting are deliberately performed outside that lock.
    fn read(s: &Shared, targets: &[ValueTarget]) -> Vec<crate::project::editor::ValueRow> {
        use crate::project::editor::{ValueBit, ValueRow};
        targets
            .iter()
            .map(|row| ValueRow {
                name: row.name.clone(),
                bits: row
                    .bits
                    .iter()
                    .map(|bit| ValueBit {
                        name: bit.name.clone(),
                        index: bit.index,
                        alternatives: bit
                            .alternatives
                            .iter()
                            .map(|(net, wire)| {
                                (*net, wire.and_then(|wire| s.lit.get(wire).copied()))
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect()
    }
}

impl App {
    /// The window on its own: a design running, or (none) the start screen
    /// to open one from.
    pub fn start(cc: &eframe::CreationContext<'_>, design: Option<live::Design>) -> Self {
        let mut app = Self::new(cc, "Open a design".into(), None);
        app.start_screen = true;
        app.shared.lock().unwrap().stage = String::new();
        if let Some(design) = design {
            let ctx = app.ctx.clone();
            app.switch(&ctx, Some(design));
        }
        app
    }

    /// The window with a design running, or with none yet (a project not
    /// built: src/project.rs opens one later with `open`).
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        title: String,
        design: Option<live::Design>,
    ) -> Self {
        let given = title.clone();
        // The chip's model and fabric, the same for every design, start now.
        live::prepare_chip();
        let mut browser_error = None;
        let preferences = Preferences::load().unwrap_or_else(|e| {
            let error = format!("Reading preferences: {e}");
            eprintln!("{error}");
            browser_error = Some(error);
            Preferences::defaults()
        });
        theme::install(&cc.egui_ctx, preferences.theme);
        let demos = start::demos(&start::repository_root()).unwrap_or_else(|e| {
            let error = format!("Reading demos: {e}");
            eprintln!("{error}");
            browser_error = Some(error);
            Demos::default()
        });
        let mut app = Self {
            new_file_available: false,
            new_file_requested: false,
            creation_command: None,
            creation_busy: false,
            creation_available: true,
            project_request: None,
            preferred_rate: None,
            title,
            shared: Arc::new(Mutex::new(Shared::default())),
            commands: None,
            preferences,
            load_times: start::LoadTimes::load(),
            loading: None,
            demos,
            browser_error,
            reader: None,
            reader_check: start::reader::Check::new,
            pending_design: None,
            open_notes: Vec::new(),
            gpu: false,
            frames: VecDeque::new(),
            opened_at: std::time::Instant::now(),
            fabric: FabricView::new(None, None, (0.5, 0.5)),
            inspector: Inspector::default(),
            map: Map::Fabric,
            cell_at: HashMap::new(),
            logic_at: HashMap::new(),
            pending_net: None,
            console: String::new(),
            line: String::new(),
            held: [false; 7],
            debounce: DebounceHint::default(),
            step_leds: StepLedHint::default(),
            board: BoardView::new(cc),
            board_uart: BoardUartSplit::default(),
            script_held: [false; 7],
            seen_switches: None,
            switch_requests: SwitchRequests::default(),
            step_choice: Some(1),
            step_text: String::new(),
            rate_custom: false,
            rate_text: String::new(),
            goto_text: String::new(),
            script: Vec::new(),
            ready_at: None,
            trace_ms: 0.0,
            pointer: None,
            layout: None,
            layout_evidence: None,
            monitor: Monitor::default(),
            ctx: cc.egui_ctx.clone(),
            start_screen: false,
            scripted_action: None,
            taken: Taken::default(),
            source_values: SourceValues::default(),
            waves: crate::waves::Waves::default(),
            script_pointed: 0,
            source_files: None,
            source_root: None,
            paths: DisplayPaths::new(None),
            #[cfg(test)]
            defer_native_load: false,
            #[cfg(test)]
            board_publication: None,
            source_index_recorded: false,
            probe_notes: Vec::new(),
            source_jump: None,
            chip_jump: false,
            probe_status: None,
            probed: Vec::new(),
            watch_id: None,
            wave_rows_received: false,
            waves_drawn: false,
            script_sources: VecDeque::new(),
            critical: Critical::default(),
            toolbar: toolbar::State::default(),
        };
        // The GPU draws the design's wires where it can (ECP5_GUI_CPU=1: the
        // CPU, as before).
        if std::env::var_os("ECP5_GUI_CPU").is_none() {
            if let Some(state) = cc.wgpu_render_state.as_ref() {
                if crate::gpu_map::GpuMap::install(state) {
                    app.gpu = true;
                    app.fabric.use_gpu();
                }
            }
        }
        match design {
            Some(design) => {
                let ctx = app.ctx.clone();
                app.switch(&ctx, Some(design));
                app.title = given;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "ecp5_emu — {}",
                    app.title
                )));
            }
            None => {
                app.shared.lock().unwrap().stage =
                    "No design loaded: build the project, then Emulate".into()
            }
        }
        app
    }
    /// What the run is doing: the stage of loading the design, whether it
    /// is ready, and the error that stopped it (src/project.rs shows these
    /// while the window shows the project, not the chip).
    pub fn state(&self) -> (String, bool, Option<String>) {
        let s = self.shared.lock().unwrap();
        (s.stage.clone(), s.ready.is_some(), s.error.clone())
    }

    pub fn ready(&self) -> Option<Arc<live::Static>> {
        self.shared.lock().unwrap().ready.clone()
    }

    pub fn has_design(&self) -> bool {
        self.commands.is_some()
    }

    pub fn project_sources(
        &mut self,
        root: std::path::PathBuf,
        paths: impl IntoIterator<Item = std::path::PathBuf>,
    ) {
        self.fabric.vhdl_sources = ecp5_emu::flow::Project::open(&root).is_ok_and(|p| p.has_vhdl());
        self.source_files = Some(sources::Files::project(root.clone(), paths));
        self.paths.set_root(Some(&root));
        self.source_root = Some(root);
    }

    pub(crate) fn set_project_title(&mut self, title: &str) {
        self.title = self.paths.text(title);
    }

    pub(crate) fn set_display_paths(&mut self, paths: DisplayPaths) {
        self.paths = paths;
    }

    pub fn record_note(&mut self, note: &str) {
        if let Some(record) = self.fabric.trace.get_mut() {
            let _ = record.note(&self.paths.text(note));
        }
    }

    fn update_debounce_hint(&mut self, now: std::time::Instant, cycles: u64, rate: Option<f64>) {
        if let Some(note) = self.debounce.update(self.held, now, cycles, rate) {
            self.record_note(&note);
        }
    }

    pub fn has_trace(&self) -> bool {
        self.fabric.trace.borrow().is_some()
    }

    pub fn drawing_progress(&self) -> (u32, u64) {
        self.fabric
            .trace
            .borrow()
            .as_ref()
            .map_or((0, 0), Trace::progress)
    }

    pub fn tour_evidence(&self) -> (bool, bool, bool) {
        let shared = self.shared.lock().unwrap();
        (
            shared.running && shared.rate == Some(4.0),
            !self.probed.is_empty(),
            self.waves.open && self.waves_drawn,
        )
    }

    pub fn map_rect(&self) -> egui::Rect {
        self.fabric.map_rect
    }

    pub fn probed_point(&self) -> Option<egui::Pos2> {
        let s = self.ready()?;
        self.fabric.probed_point(&s.fabric, &s.overlay)
    }

    pub fn show_source_at(&mut self, point: egui::Pos2) -> bool {
        let Some(s) = self.ready() else {
            return false;
        };
        self.fabric.show_source_at(&s.fabric, &s.overlay, point)
    }

    pub fn run(&mut self) {
        self.send(Command::Run);
    }

    fn probe_note(&mut self, note: String) {
        let note = self.paths.text(&note);
        if let Some(record) = self.fabric.trace.get_mut() {
            let _ = record.note(&note);
        }
        if !self.start_screen {
            self.probe_notes.push(format!("# {note}"));
        }
    }

    /// Also polled while the project editor is visible, without drawing the map.
    pub fn poll_sources(&mut self) {
        self.fabric.project_sources = !self.start_screen;
        self.waves.project_sources = !self.start_screen;
        if let Some(s) = self.ready() {
            if let Some(index) = s.sources.get() {
                self.fabric.sources = Some(Arc::clone(index));
                self.waves.sources = Some(Arc::clone(index));
                if !self.source_index_recorded {
                    self.source_index_recorded = true;
                    self.probe_note(format!(
                        "probe index {:.6} {} {}",
                        s.source_seconds.get().copied().unwrap_or_default(),
                        index.nets.len(),
                        index.locations
                    ));
                    self.probe_note(format!(
                        "probe load-pass {:.6}",
                        index.collect_time.as_secs_f64()
                    ));
                }
            }
            if let Some(timing) = s.timing.get() {
                if !self.critical.recorded {
                    self.critical.recorded = true;
                    self.probe_note(format!(
                        "critical cost {:.6} {}",
                        timing.seconds, timing.bytes
                    ));
                    match &timing.result {
                        Ok(paths) => {
                            for path in paths {
                                self.probe_note(path.path.note());
                            }
                        }
                        Err(error) => self.probe_note(error.clone()),
                    }
                }
            }
        }
    }

    /// Used by both the report's button and window scripts; selection waits
    /// for the after-ready publication when the design is still loading.
    pub fn critical_path(&mut self, clock: &str) {
        self.critical.command(clock);
        self.ctx.request_repaint();
    }

    pub fn take_probe_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.probe_notes)
    }
    pub fn take_source_jump(&mut self) -> Option<Span> {
        self.source_jump.take()
    }

    pub fn take_chip_jump(&mut self) -> bool {
        std::mem::take(&mut self.chip_jump)
    }

    fn waves_action(&mut self, action: crate::waves::RowAction) {
        match action {
            crate::waves::RowAction::Chip { name, nets } => {
                self.probe_note(format!("waves chip {name} -> {} nets", nets.len()));
                self.probe_status = Some(format!("{name}: {} nets on the chip", nets.len()));
                self.fabric.keep_nets(nets.clone());
                self.probed = nets;
                self.map = Map::Fabric;
                self.chip_jump = true;
            }
            crate::waves::RowAction::Source {
                name,
                locations,
                chosen,
            } => {
                self.probe_note(format!(
                    "waves source {name} -> {}",
                    source_labels(&locations)
                ));
                self.source_jump = source_jump_or_copy(&self.ctx, !self.start_screen, chosen);
            }
        }
        self.ctx.request_repaint();
    }

    fn waves_outcome(
        &mut self,
        source: bool,
        name: &str,
        result: Result<crate::waves::RowAction, crate::waves::RowError>,
    ) {
        match result {
            Ok(action) => self.waves_action(action),
            Err(error) => {
                let kind = if source { "source" } else { "chip" };
                let outcome = match error {
                    crate::waves::RowError::NotRow => "not a Waves row".into(),
                    crate::waves::RowError::Unavailable(reason) => format!("unavailable: {reason}"),
                };
                let note = format!("waves {kind} {name} -> {outcome}");
                eprintln!("script: {note}");
                self.probe_note(note);
            }
        }
    }

    pub fn begin_source_values(&mut self) {
        self.source_values.visible.clear();
    }

    pub fn end_source_values(&mut self) {
        self.source_values
            .formatted
            .retain(|key, _| self.source_values.visible.contains(key));
    }

    /// Sparse read-only lookup. Passive callers cannot send runner commands.
    pub fn source_value(&mut self, file: &str, name: &str) -> crate::project::editor::ValueView {
        use crate::project::editor::{format_values, ValueView};
        if name.is_empty() {
            return ValueView::unavailable("No signal identifier here");
        }
        let (design, stage, error) = {
            let s = self.shared.lock().unwrap();
            (s.ready.clone(), s.stage.clone(), s.error.clone())
        };
        if let Some(error) = error {
            return ValueView::unavailable(format!("Design failed to load: {error}"));
        }
        if !self.has_design() {
            return ValueView::unavailable("Emulate this project to see values");
        }
        let Some(design) = design else {
            return ValueView::unavailable(format!("Design is loading: {stage}"));
        };
        let Some(index) = design.sources.get() else {
            return ValueView::unavailable("Source index is being prepared");
        };
        self.source_values.bind(&design, index);
        let key = (file.to_owned(), name.to_owned());
        self.source_values.visible.insert(key.clone());
        let targets =
            self.source_values
                .mappings
                .entry(key.clone())
                .or_insert_with(|| {
                    let rows = match index.name_only(file, name) {
                        sources::NameLookup::Absent => return Vec::new(),
                        sources::NameLookup::Unrouted(rows)
                        | sources::NameLookup::Resolved(rows) => rows,
                    };
                    rows.iter()
                        .map(|row| ValueTarget {
                            name: row.name.clone(),
                            bits: row
                                .bits
                                .iter()
                                .map(|bit| ValueWire {
                                    name: bit.name.clone(),
                                    index: bit.index,
                                    alternatives: bit
                                        .targets
                                        .iter()
                                        .map(|net| {
                                            let wire = design
                                                .overlay
                                                .pieces_of(*net)
                                                .first()
                                                .and_then(|piece| {
                                                    design.overlay.pieces.get(*piece as usize)
                                                })
                                                .map(|piece| piece.value as usize);
                                            (*net, wire)
                                        })
                                        .collect(),
                                })
                                .collect(),
                        })
                        .collect()
                });
        let (unavailable, rows, cached, cycle, edges, published, running) = {
            let s = self.shared.lock().unwrap();
            let unavailable = if s
                .ready
                .as_ref()
                .is_none_or(|ready| !Arc::ptr_eq(ready, &design))
            {
                Some(SourceUnavailable::Loading)
            } else if let Some(error) = &s.error {
                Some(SourceUnavailable::Failed(error.clone()))
            } else if self.source_values.seek.seeking(&s) {
                Some(SourceUnavailable::Seeking)
            } else if s.published == 0 {
                Some(SourceUnavailable::FirstPublication)
            } else if !targets
                .iter()
                .flat_map(|row| &row.bits)
                .any(|bit| !bit.alternatives.is_empty())
            {
                Some(SourceUnavailable::NoNet)
            } else {
                None
            };
            let cached = self
                .source_values
                .formatted
                .get(&key)
                .is_some_and(|v| v.published == s.published);
            let rows = if unavailable.is_none() && !cached {
                SourceValues::read(&s, targets)
            } else {
                Vec::new()
            };
            (
                unavailable,
                rows,
                cached,
                s.cycles,
                s.edges,
                s.published,
                s.running,
            )
        };
        if let Some(unavailable) = unavailable {
            let message = match unavailable {
                SourceUnavailable::Loading => "Design is loading".into(),
                SourceUnavailable::Failed(error) => format!("Design failed to load: {error}"),
                SourceUnavailable::Seeking => "seeking…".into(),
                SourceUnavailable::FirstPublication => "Values are not available yet".into(),
                SourceUnavailable::NoNet => format!(
                    "`{name}` has no net on the chip: optimised away, a constant, or inside a cell"
                ),
            };
            let mut view = ValueView::unavailable(message);
            view.published = published;
            view.edges = edges;
            view.running = running;
            return view;
        }
        if cached {
            let mut view = self.source_values.formatted[&key].clone();
            view.running = running;
            return view;
        }
        let (display, detail) = format_values(&rows);
        let mut nets: Vec<_> = rows
            .iter()
            .flat_map(|row| &row.bits)
            .flat_map(|bit| bit.alternatives.iter().map(|(net, _)| *net))
            .collect();
        nets.sort_unstable();
        nets.dedup();
        let view = ValueView {
            rows: display,
            detail,
            cycle: Some(cycle),
            edges,
            published,
            running,
            targets: nets,
            warning: None,
        };
        self.source_values.formatted.insert(key, view.clone());
        view
    }

    pub fn source_value_note(&mut self, note: String, view: &crate::project::editor::ValueView) {
        let key = (view.published, view.edges, note.clone());
        if self.source_values.last_note.as_ref() != Some(&key) {
            self.source_values.last_note = Some(key);
            self.probe_note(note);
        }
    }

    pub fn probe_source(
        &mut self,
        file: &str,
        line: u32,
        column: u32,
        identifier: &str,
        warning: Option<&str>,
    ) -> Option<String> {
        self.poll_sources();
        let s = self.ready()?;
        let index = s.sources.get()?;
        let mut probe = index.probe(file, line, column, identifier);
        let message = probe.message.clone();
        if let Some(warning) = warning {
            probe.message.push_str(&format!("; {warning}"));
        }
        self.probe_note(probe.note(file, line, column));
        self.fabric.keep_nets(probe.nets.clone());
        self.probed = probe.nets;
        self.probe_status = Some(probe.message.clone());
        self.map = Map::Fabric;
        self.ctx.request_repaint();
        Some(message)
    }

    pub fn watch_probed(&mut self) {
        self.watch_source_targets(&self.probed.clone());
    }

    pub fn watch_source_targets(&mut self, targets: &[u32]) {
        if let Some(s) = self.ready() {
            for &net in targets {
                let name = s
                    .sources
                    .get()
                    .and_then(|i| i.nets.get(&net))
                    .and_then(|n| n.names.iter().find(|n| !n.contains('$')))
                    .cloned();
                self.waves.add_net_from(&s, net, name, "source");
            }
            if let Some(command) = self.waves.command() {
                self.send(command);
            }
        }
    }

    fn show_source(&mut self, target: String, locations: Vec<SourceLocation>, chosen: Span) {
        self.probe_note(map_probe_note(&target, &locations));
        self.source_jump = source_jump_or_copy(&self.ctx, !self.start_screen, chosen);
    }
    /// Runs another design in the window in place of the one there: the old
    /// run ends (its thread stops when its commands are dropped), and what
    /// was shown of it goes. The view's place on the die, the GPU and the
    /// record (ECP5_GUI_TRACE) stay, so one record covers both designs.
    pub fn open(&mut self, title: String, design: live::Design) {
        let ctx = self.ctx.clone();
        self.switch(&ctx, Some(design));
        self.title = title;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "ecp5_emu — {}",
            self.title
        )));
    }

    fn save_preferences(&mut self) {
        if let Err(e) = self.preferences.save() {
            let error = format!("Saving preferences: {e}");
            eprintln!("{error}");
            self.browser_error = Some(error);
        }
    }

    pub(crate) fn dev_mode(&self) -> bool {
        self.preferences.dev_mode
    }

    fn set_dev_mode(&mut self, dev: bool) {
        if self.preferences.dev_mode != dev {
            self.preferences.dev_mode = dev;
            self.save_preferences();
            self.record_note(if dev { "dev on" } else { "dev off" });
        }
    }

    /// Start the shared prerequisite check before any native design loading.
    pub fn prepare_reader(&mut self) -> Result<(), String> {
        if self.reader.as_ref().is_some_and(|check| {
            matches!(check.status(), start::reader::Status::Missing(_))
                || (check.status() == start::reader::Status::Ready
                    && !ecp5_emu::image::reader_path(None).is_file())
        }) {
            self.reader = None;
        }
        let check = self.reader.get_or_insert_with(self.reader_check);
        let status = check.status();
        let notes = check.notes();
        for note in notes {
            if self.has_trace() {
                self.record_note(&note);
            } else {
                self.open_notes.push(note);
            }
        }
        match status {
            start::reader::Status::Missing(reason) => {
                self.browser_error = Some(reason.clone());
                Err(reason)
            }
            _ => Ok(()),
        }
    }

    fn switch(&mut self, ctx: &egui::Context, design: Option<live::Design>) {
        self.pending_design = None;
        if design.is_some() {
            if let Err(reason) = self.prepare_reader() {
                self.shared.lock().unwrap().error = Some(reason);
                return;
            }
            let check = self.reader.as_mut().unwrap();
            self.open_notes.extend(check.notes());
            if check.status() == start::reader::Status::Building {
                self.pending_design = design;
                self.browser_error = Some(start::reader::BUILDING.into());
                self.shared.lock().unwrap().stage = start::reader::BUILDING.into();
                ctx.request_repaint();
                return;
            }
        }
        self.browser_error = None;
        self.switch_ready(ctx, design);
    }

    /// Continue pending opens and demo builds on the window thread.
    fn poll_open(&mut self, ctx: &egui::Context) {
        let mut actions = Vec::new();
        for demo in self.demos.iter_mut() {
            if let Some((action, note)) = demo.poll_build() {
                self.open_notes.push(note);
                actions.extend(action);
            }
        }
        if let Some(check) = &mut self.reader {
            self.open_notes.extend(check.notes());
            if self.pending_design.is_some() {
                match check.status() {
                    start::reader::Status::Ready => {
                        let design = self.pending_design.take();
                        self.browser_error = None;
                        self.switch_ready(ctx, design);
                    }
                    start::reader::Status::Missing(reason) => {
                        self.pending_design = None;
                        self.browser_error = Some(reason.clone());
                        let mut shared = self.shared.lock().unwrap();
                        shared.stage.clear();
                        shared.error = Some(reason);
                    }
                    start::reader::Status::Building => {
                        ctx.request_repaint_after(Duration::from_millis(100))
                    }
                }
            }
        }
        for note in std::mem::take(&mut self.open_notes) {
            self.record_note(&note);
        }
        for action in actions {
            self.action(ctx, action);
        }
    }

    fn switch_ready(&mut self, ctx: &egui::Context, design: Option<live::Design>) {
        self.commands.take();
        // Retire pending wire measurements before replacing their indices.
        // The same record continues with a note marking the new design.
        let mut trace = self.fabric.trace.get_mut().take();
        if let Some(t) = trace.as_mut() {
            t.drawn.sight(std::iter::empty());
            t.drawn.drawn(f64::MAX);
            let shown = Shown {
                ui_ms: 0.0,
                update_ms: 0.0,
                trace_ms: 0.0,
                running: false,
                rate: None,
                cycles: 0,
                mhz: 0.0,
                values: false,
                zoom: 1.0,
                map_height: 0.0,
                pointing: false,
                dropped: 0,
                gpu: Default::default(),
            };
            if let Err(e) = t.frame(0.0, &shown) {
                eprintln!("trace: {e:#}");
            }
            t.take_resets();
            t.wires(0);
            let name = design.as_ref().map_or_else(
                || "closed".into(),
                |d| {
                    let path = self
                        .source_root
                        .as_ref()
                        .and_then(|root| d.config.strip_prefix(root).ok())
                        .unwrap_or(&d.config);
                    self.paths.path(path)
                },
            );
            if let Err(e) = t.note(&format!("design\t{name}")) {
                eprintln!("trace: {e:#}");
            }
        }
        let start = self.fabric.start;
        let glow = self.fabric.glow;
        self.fabric = FabricView::new(None, None, (0.5, 0.5));
        self.fabric.start = start;
        self.fabric.glow = glow;
        if self.gpu {
            self.fabric.use_gpu();
        }
        *self.fabric.trace.get_mut() = trace;
        self.inspector = Inspector::default();
        // The HDMI monitor and the OLED show the design that ran.
        self.monitor = Monitor::default();
        self.cell_at.clear();
        self.logic_at.clear();
        self.pending_net = None;
        self.map = Map::Fabric;
        self.console.clear();
        self.line.clear();
        self.held = [false; 7];
        self.debounce = DebounceHint::default();
        self.step_leds = StepLedHint::default();
        self.script_held = [false; 7];
        self.seen_switches = None;
        self.switch_requests = SwitchRequests::default();
        self.ready_at = None;
        self.loading = None;
        self.pointer = None;
        self.layout = None;
        self.layout_evidence = None;
        self.trace_ms = 0.0;
        self.goto_text.clear();
        self.rate_custom = false;
        self.rate_text.clear();
        self.step_choice = Some(1);
        self.step_text.clear();
        self.board.take_notes();
        self.shared = Arc::new(Mutex::new(Shared::default()));
        self.taken = Taken::default();
        self.source_values = SourceValues::default();
        self.waves.reset();
        self.script_pointed = 0;
        self.source_index_recorded = false;
        self.critical = Critical::default();
        self.reset_toolbar_editors();
        self.source_jump = None;
        self.chip_jump = false;
        self.probe_status = None;
        self.probed.clear();
        self.watch_id = None;
        self.wave_rows_received = false;
        self.waves_drawn = false;
        self.script_sources.clear();
        self.probe_notes.clear();
        self.title = "Open a design".into();
        // Closed: a script's next lines on the start screen count from now
        // (`scripted`), not from when the window opened.
        if design.is_none() {
            self.opened_at = std::time::Instant::now();
        }
        // Headless record tests exercise opening without native workers or preferences writes.
        #[cfg(test)]
        if self.defer_native_load {
            return;
        }
        if let Some(design) = design {
            self.title = design_title(&design.config);
            self.preferences.add(&design.config);
            self.save_preferences();
            let path = self.preferences.recent[0].clone();
            self.loading = Some(loading::Loading::new(
                path.clone(),
                self.load_times.get(&path),
                std::time::Instant::now(),
            ));
            let ctx = ctx.clone();
            self.commands = Some(live::spawn_with_sources(
                design,
                live::RunOptions::default(),
                self.source_files.take(),
                Arc::clone(&self.shared),
                move || ctx.request_repaint(),
            ));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "ecp5_emu — {}",
            self.title
        )));
        ctx.request_repaint();
    }

    pub(crate) fn action(&mut self, ctx: &egui::Context, action: Action) {
        let config = match action {
            Action::DevMode(dev) => {
                self.set_dev_mode(dev);
                return;
            }
            Action::NewFile => {
                self.new_file_requested = self.new_file_available;
                return;
            }
            Action::NewProject | Action::StartFromDemo => {
                self.creation_command = Some(if matches!(action, Action::NewProject) {
                    crate::new_project::Command::New(Default::default())
                } else {
                    crate::new_project::Command::Demo(Default::default())
                });
                self.creation_busy = true;
                return;
            }
            Action::Build(name) => {
                if let Some(demo) = self.demos.iter_mut().find(|d| d.name == name) {
                    demo.start_build(ctx);
                }
                return;
            }
            Action::Tour => {
                if let Err(e) = start::launch_tour(ctx) {
                    self.browser_error = Some(format!("Could not start the tour: {e}"));
                }
                return;
            }
            Action::Close => {
                self.switch(ctx, None);
                return;
            }
            Action::Open => rfd::FileDialog::new()
                .add_filter("ECP5 designs", &["config", "bit"])
                .pick_file(),
            Action::Load(path) if path.is_dir() => {
                self.project_request = Some(path);
                return;
            }
            Action::Load(path) => Some(path),
        };
        if let Some(config) = config {
            self.source_files = None;
            self.fabric.vhdl_sources = false;
            self.source_root = None;
            self.paths.set_root(None);
            if let Some(demo) = self
                .demos
                .iter()
                .find(|d| d.built_config().as_ref() == Some(&config))
            {
                match ecp5_emu::flow::Project::open(&demo.project) {
                    Ok(project) => self.project_sources(
                        project.root.clone(),
                        project.hdl_roots().iter().chain(&project.files).cloned(),
                    ),
                    Err(error) => {
                        self.browser_error = Some(start::display_text(&format!("{error:#}")));
                        return;
                    }
                }
            }
            let routed = config.with_file_name("top_routed.json");
            let names = routed.is_file().then_some(routed);
            self.switch(
                ctx,
                Some(live::Design {
                    config,
                    names,
                    run: false,
                }),
            );
        }
    }

    pub(crate) fn design_menu(
        &mut self,
        ui: &mut egui::Ui,
        mut project: Option<&mut crate::project::Toolbar>,
    ) -> Option<Action> {
        let mut action = None;
        let menu = ui.menu_button("Design ▾", |ui| {
            ui.set_min_width(210.0);
            for (label, choice) in [
                ("New project…", Action::NewProject),
                ("Start from a demo…", Action::StartFromDemo),
            ] {
                let response = ui
                    .add_enabled(self.creation_available, egui::Button::new(label))
                    .on_disabled_hover_text("Wait for the current build or emulation to finish");
                self.remember(label, &response);
                if response.clicked() {
                    action = Some(choice);
                    ui.close();
                }
            }
            if ui
                .add_enabled(self.new_file_available, egui::Button::new("New file…"))
                .on_disabled_hover_text("Open an idle project to create a file")
                .clicked()
            {
                action = Some(Action::NewFile);
                ui.close();
            }
            if ui.button("Open…").clicked() {
                action = Some(Action::Open);
                ui.close();
            }
            ui.menu_button("Recent", |ui| {
                ui.set_max_width(540.0);
                if let Some(a) = start::recent_choices(ui, &self.preferences.recent) {
                    action = Some(a);
                }
            });
            ui.menu_button("Demos", |ui| {
                ui.set_max_width(540.0);
                if let Some(a) = start::demo_choices(ui, &self.demos, false, self.dev_mode()) {
                    action = Some(a);
                }
            });
            if ui
                .add_enabled(self.commands.is_some(), egui::Button::new("Close"))
                .clicked()
            {
                action = Some(Action::Close);
                ui.close();
            }
            ui.separator();
            ui.menu_button("Theme", |ui| {
            for (preference, name) in [
                (egui::ThemePreference::System, "System"),
                (egui::ThemePreference::Light, "Light"),
                (egui::ThemePreference::Dark, "Dark"),
            ] {
                if ui
                    .selectable_value(&mut self.preferences.theme, preference, name)
                    .changed()
                {
                    ui.ctx().set_theme(preference);
                    self.save_preferences();
                }
            }
            });
            let mut dev = self.dev_mode();
            let response = ui.checkbox(&mut dev, "Dev mode")
                .on_hover_text(start::DEV_MODE_TOOLTIP);
            self.remember("Dev mode", &response);
            if response.changed() {
                self.set_dev_mode(dev);
            }
            if let Some(project) = project.as_deref_mut() {
                ui.menu_button("Build", |ui| project.build_menu(ui));
            }
            ui.menu_button("Run", |ui| self.run_menu(ui));
            ui.menu_button("View", |ui| {
                if let Some(project) = project.as_deref_mut() { project.tabs(ui); ui.separator(); }
                self.view_menu(ui);
            });
            ui.menu_button("Help", |ui| {
                ui.set_max_width(340.0);
                ui.label("Run/Pause controls the current emulator. Back (←) and Step (→) use the selected step size. Click the cycle readout and press Enter to seek; Escape cancels.");
                ui.label("Going back replays the same button presses and UART input until a new input starts a run from that cycle. W over a wire watches it in Waves; click its trace to seek.");
                if project.is_some() { ui.label("Build: Cmd/Ctrl+B. Emulate opens the build, or queues the running build's result."); }
            });
        });
        self.remember("Design", &menu.response);
        action
    }

    fn peripherals_menu(&mut self, ui: &mut egui::Ui) {
        let menu = ui.menu_button("Peripherals", |ui| {
            for (panel, available, mut open) in self.monitor.panels() {
                if ui
                    .add_enabled(available, egui::Checkbox::new(&mut open, panel.label()))
                    .changed()
                {
                    if let Err(error) = self.monitor.set_open(panel, open) {
                        eprintln!("peripherals: {error}");
                    }
                }
            }
        });
        self.remember("Peripherals", &menu.response);
    }

    #[cfg(test)]
    pub(crate) fn isolated_preferences(&mut self, path: std::path::PathBuf) {
        let parent = path.parent().unwrap().to_path_buf();
        self.preferences = Preferences::read(Some(path)).unwrap();
        self.preferences.project_parent.get_or_insert(parent);
    }

    pub(crate) fn project_parent(&self) -> Option<std::path::PathBuf> {
        self.preferences.project_parent.clone()
    }

    pub(crate) fn remember_project(&mut self, dir: &Path) {
        self.preferences.add(dir);
        self.preferences.project_parent = dir.parent().map(Path::to_path_buf);
        self.save_preferences();
    }

    pub(crate) fn reset_for_project(&mut self, ctx: &egui::Context) {
        self.switch(ctx, None);
        self.start_screen = false;
        self.source_root = None;
        self.source_files = None;
        self.ready_at = Some(std::time::Instant::now());
    }

    pub(crate) fn creation_clock(&mut self) {
        self.ready_at = Some(std::time::Instant::now());
    }

    pub(crate) fn creation_complete(&mut self) {
        self.creation_busy = false;
    }

    /// Keeps a record of what the map draws (src/trace.rs).
    pub fn trace(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        *self.fabric.trace.get_mut() = Some(Trace::new(path)?);
        self.layout = None;
        self.layout_evidence = None;
        self.reset_toolbar_recording();
        Ok(())
    }

    fn load_die_shown(&mut self) {
        if let Some(load) = self.loading.as_mut() {
            if !load.die_shown {
                load.die_shown = true;
                if let Some(record) = self.fabric.trace.get_mut() {
                    let _ = record.note(&format!(
                        "load die-shown {:.3}",
                        load.elapsed(std::time::Instant::now())
                    ));
                }
            }
        }
    }
    /// Gives the commands of a script (src/trace.rs).
    pub fn script(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.script = trace::script(path)?;
        Ok(())
    }

    /// A guided tour owns playback even when started inside a scripted window.
    pub fn clear_script(&mut self) {
        self.script.clear();
        self.scripted_action = None;
        self.script_sources.clear();
        self.script_pointed = 0;
        self.pointer = None;
    }
    /// Window scripts also advance while the project editor is visible.
    pub(crate) fn script_in_project(&mut self, ui: &egui::Ui) {
        self.poll_open(ui.ctx());
        if self.has_design() && self.ready().is_none() && self.state().2.is_none() {
            return;
        }
        self.ready_at.get_or_insert_with(std::time::Instant::now);
        self.scripted(ui);
        if !self.script.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }

    /// The script's commands due by now.
    fn scripted(&mut self, ui: &egui::Ui) {
        if self.creation_busy {
            return;
        }
        if self.commands.is_none()
            && self.start_screen
            && (self.pending_design.is_some() || self.demos.iter().any(|d| d.build.running()))
        {
            return;
        }
        // From when the design is ready; on the start screen, with none
        // yet, from when the window opened (to `open` one).
        let at = match (self.ready_at, self.commands.is_none() && self.start_screen) {
            (Some(at), _) => at,
            (None, true) => self.opened_at,
            (None, false) => return,
        };
        let t = at.elapsed().as_secs_f64();
        while self.script.first().is_some_and(|c| c.0 <= t) {
            let (_, command) = self.script.remove(0);
            if let Some(record) = self.fabric.trace.borrow_mut().as_mut() {
                let _ = record.note(&self.paths.text(&format!("script\t{t:.3}\t{command}")));
            }
            match crate::new_project::Command::parse(&command) {
                Ok(Some(command)) => {
                    self.creation_command = Some(command);
                    self.creation_busy = true;
                    break;
                }
                Err(error) => {
                    self.record_note(&format!("new-project failed {error:#}"));
                    continue;
                }
                Ok(None) => {}
            }
            let mut words = command.split_whitespace();
            let (word, arg) = (words.next().unwrap_or(""), words.next());
            // Opening or building a design takes effect later in the frame;
            // subsequent lines wait for it and count from when it is ready.
            let mut switching = matches!(word, "open" | "close");
            match (word, arg) {
                ("run", _) => self.send(Command::Run),
                ("pause", _) => self.send(Command::Pause),
                ("step", Some(n)) => self.send(Command::Step(n.parse().unwrap_or(1))),
                ("rate", Some("max")) => self.send(Command::Rate(None)),
                ("rate", Some("real")) => self.send(Command::Rate(Some(live::REAL_TIME))),
                ("rate", Some(r)) => self.send(Command::Rate(r.parse().ok())),
                ("values", Some(on)) => self.fabric.values = on == "on",
                ("critical", Some(_)) => self.critical_path(command["critical".len()..].trim()),
                ("glow", Some(mode)) => {
                    if !self.fabric.set_glow(mode) {
                        eprintln!("script: {command:?} not known");
                    }
                }
                ("zoom", Some(f)) => {
                    let view = ui.ctx().content_rect().size();
                    self.fabric.zoom(f.parse().unwrap_or(1.0), view);
                }
                ("fit", _) => self.fabric.fit(),
                // Nets watched in the waves: an HDL name (a bus's name: every
                // bit), N: the design's first N HDL names, or none.
                ("waves", Some(state @ ("open" | "close"))) => {
                    self.waves.set_open(state == "open");
                }
                ("waves-chip" | "waves-source", _) => {
                    if let Some((source, name)) = waves_command(&command) {
                        self.poll_sources();
                        let result = self.waves.action_for_row(name, source);
                        self.waves_outcome(source, name, result);
                    } else {
                        eprintln!("script: expected waves-chip NAME or waves-source NAME");
                    }
                }
                ("watch", Some("pointed")) => self.script_pointed += 1,
                ("watch", Some(what)) => {
                    let ready = self.shared.lock().unwrap().ready.clone();
                    if let Some(s) = ready {
                        match (what, what.parse::<usize>()) {
                            ("none", _) => self.waves.clear(),
                            ("shown", _) => self.waves.add_shown(&s, self.fabric.net, "script"),
                            (_, Ok(n)) => {
                                self.waves.add_first(&s, n);
                            }
                            (_, Err(_)) => {
                                self.waves.add_name_from(&s, what, "script");
                            }
                        }
                    }
                }
                // The map's middle at shares X, Y of the die, at ZOOM times
                // the zoom that fits it.
                ("view", Some(x)) => {
                    let mut number = || words.next().and_then(|w| w.parse::<f32>().ok());
                    if let (Ok(x), Some(y), Some(zoom)) = (x.parse::<f32>(), number(), number()) {
                        self.fabric.aim(x, y, zoom);
                    }
                }
                // The die covered view by view at ZOOM, a view every SECONDS
                // from now: `view` lines put among the script's own.
                ("sweep", Some(zoom)) => {
                    let every = words
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(2.0);
                    let zoom = zoom.parse::<f32>().unwrap_or(4.0);
                    let views =
                        fabric_view::sweep(self.fabric.die, self.fabric.map_rect.size(), zoom);
                    let lines = views
                        .iter()
                        .enumerate()
                        .map(|(k, (x, y))| {
                            (t + k as f64 * every, format!("view {x:.4} {y:.4} {zoom}"))
                        })
                        .collect();
                    crate::trace::insert(&mut self.script, lines);
                }
                ("why", _) => self.fabric.why.set(true),
                // The pointer held still at a point of the map as it is now,
                // as shares of its width and height; `point off` lets go.
                ("point", Some("off")) => self.pointer = None,
                ("show-source", _) => match source_point(&command) {
                    Some(point) => self.script_sources.push_back(point),
                    None => eprintln!("script: expected show-source X Y (shares of the map)"),
                },
                ("point", Some(x)) => {
                    let y = words
                        .next()
                        .and_then(|y| y.parse::<f32>().ok())
                        .unwrap_or(0.5);
                    let m = self.fabric.map_rect;
                    self.pointer = Some(
                        m.min
                            + egui::vec2(
                                x.parse::<f32>().unwrap_or(0.5) * m.width(),
                                y * m.height(),
                            ),
                    );
                }
                // A button held down or let go, as from the board.
                ("button", Some(n)) => {
                    if let Ok(i) = n.parse::<usize>() {
                        if i < 7 {
                            self.script_held[i] = words.next() != Some("up");
                        }
                    }
                }
                ("switch", n) => {
                    let used = self
                        .shared
                        .lock()
                        .unwrap()
                        .ready
                        .as_ref()
                        .map_or([false; 4], |s| s.switches);
                    match switch_command(n, words.next(), &used) {
                        Ok(command) => self.send(command),
                        Err(e) => eprintln!("script: {command:?}: {e}"),
                    }
                }
                ("panel", _) => {
                    if let Err(error) = panel_command(&command)
                        .and_then(|(panel, open)| self.monitor.set_open(panel, open))
                    {
                        eprintln!("script: {command:?}: {error}");
                    }
                }
                ("board", Some(view)) => self.board.view(view),
                // The window's size, in points (its surface made anew).
                ("size", Some(w)) => {
                    if let (Ok(w), Some(Ok(h))) =
                        (w.parse::<f32>(), words.next().map(str::parse::<f32>))
                    {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                    }
                }
                // The window minimised and back, as when it is out of sight
                // (minimised it draws no frames, so `show` waits for one).
                ("hide", _) => ui
                    .ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                ("show", _) => {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                // Text typed into the UART console, sent as its box sends a
                // line: the rest of the line after `type `, where `\r`, `\n`,
                // `\\` and `\xHH` stand for those bytes (`#` starts a comment,
                // so it is `\x23`). No CR is added: end a line with `\r`.
                ("type", Some(_)) => {
                    let text = command.trim_start()["type".len()..].trim_start().as_bytes();
                    let mut bytes = Vec::with_capacity(text.len());
                    let mut i = 0;
                    while i < text.len() {
                        let hex = text
                            .get(i + 2..i + 4)
                            .and_then(|h| std::str::from_utf8(h).ok())
                            .and_then(|h| u8::from_str_radix(h, 16).ok());
                        let (byte, len) = match (text[i], text.get(i + 1)) {
                            (b'\\', Some(b'r')) => (b'\r', 2),
                            (b'\\', Some(b'n')) => (b'\n', 2),
                            (b'\\', Some(b'\\')) => (b'\\', 2),
                            (b'\\', Some(b'x')) if hex.is_some() => (hex.unwrap_or(0), 4),
                            (b, _) => (b, 1),
                        };
                        bytes.push(byte);
                        i += len;
                    }
                    self.send(Command::Keys(bytes));
                }
                ("quit", _) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                // Another design, as from the Design menu (the rest of the
                // line is its path; later times count from when it is
                // ready), or none.
                ("open", Some(_)) => {
                    let path = command["open".len()..].trim();
                    self.scripted_action = Some(Action::Load(std::path::PathBuf::from(path)));
                }
                ("build-demo", Some(name)) => {
                    let reason = if !self.start_screen || self.commands.is_some() {
                        Some("only available on the start screen")
                    } else {
                        match self.demos.iter().find(|demo| demo.name == name) {
                            None => Some("unknown demo"),
                            Some(demo) if demo.built_config().is_some() => Some("already built"),
                            Some(_) => None,
                        }
                    };
                    if let Some(reason) = reason {
                        self.record_note(&format!("script build-demo {name}: {reason}"));
                    } else {
                        self.scripted_action = Some(Action::Build(name.into()));
                        switching = true;
                    }
                }
                ("close", _) => self.scripted_action = Some(Action::Close),
                // The theme, as from the Design menu (not kept).
                ("theme", Some(name)) => {
                    let preference = match name {
                        "light" => egui::ThemePreference::Light,
                        "dark" => egui::ThemePreference::Dark,
                        _ => egui::ThemePreference::System,
                    };
                    self.preferences.theme = preference;
                    ui.ctx().set_theme(preference);
                }
                ("dev", Some(name @ ("on" | "off"))) if words.next().is_none() => {
                    self.set_dev_mode(name == "on");
                }
                _ => eprintln!("script: {command:?} not known"),
            }
            if switching {
                break;
            }
        }
    }
    /// Where the map opens: a tile in the middle, and pixels per tile.
    pub fn view(&mut self, focus: Option<(u16, u16)>, cell: Option<f32>) {
        self.fabric.start = (focus, cell.map(|c| c / TILE_W));
    }
    /// Opens on the grid of tiles instead of the fabric.
    pub fn grid(&mut self) {
        self.map = Map::Grid;
    }
    /// Shows the net of an HDL name (`clk_25mhz`, `led[0]`) once loaded.
    pub fn show_net(&mut self, name: String) {
        self.pending_net = Some(name);
    }
    /// Cycles per second; none for as fast as possible.
    pub fn rate(&mut self, rate: Option<f64>) {
        self.preferred_rate = Some(rate);
        self.send(Command::Rate(rate));
    }
    fn send(&mut self, command: Command) {
        if let Command::Rate(rate) = &command {
            self.preferred_rate = Some(*rate);
        }
        self.switch_requests.command(&command);
        if let Command::Watch { id, .. } = &command {
            self.watch_id = Some(*id);
            self.wave_rows_received = false;
            self.waves_drawn = false;
        }
        if let Command::Switch(i, on) = command {
            let seconds = self.ready_at.map_or(0.0, |r| r.elapsed().as_secs_f64());
            let cycle = self.shared.lock().unwrap().cycles;
            if let Some(record) = self.fabric.trace.borrow_mut().as_mut() {
                let _ = record.note(&switch_request_note(seconds, cycle, i, on));
            }
            self.ctx.request_repaint();
        }
        let mut step_led_note = None;
        if let Some(commands) = &self.commands {
            // Keep dispatch and its publication baseline in one critical section.
            // A delayed command cannot be acknowledged by a pre-dispatch sample.
            let s = self.shared.lock().unwrap();
            let used = s.ready.as_ref().map_or(self.step_leds.used, |s| s.leds);
            step_led_note = self
                .step_leds
                .dispatch(&s, used, &command, std::time::Instant::now());
            self.source_values.seek.dispatch(&s, &command);
            if commands.send(command).is_err() {
                self.source_values.seek.pending = None;
                self.step_leds.pending = None;
            }
        }
        if let Some(note) = step_led_note {
            self.record_note(&note);
        }
    }
    /// How many cycles a step goes.
    fn steps(&self) -> u64 {
        match self.step_choice {
            Some(n) => n,
            None => amount(&self.step_text).map_or(1, |n| (n.round() as u64).max(1)),
        }
    }
}

pub fn design_title(path: &Path) -> String {
    let file = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    if matches!(file.as_ref(), "top.config" | "top.bit") {
        path.parent()
            .and_then(|p| p.file_name())
            .map_or_else(|| file.to_string(), |p| p.to_string_lossy().into_owned())
    } else {
        file.to_string()
    }
}

fn status_field(ui: &mut egui::Ui, width: f32, text: &str) {
    ui.add_sized(
        [width, 20.0],
        egui::Label::new(RichText::new(text).monospace()).truncate(),
    )
    .on_hover_text(text);
}

struct ButtonHold {
    since: std::time::Instant,
    cycle: u64,
    noted: bool,
}

/// A paused navigation is acknowledged by a later publication at its target.
struct LedStep {
    publication: u64,
    from: u64,
    target: u64,
    leds: [Option<bool>; 8],
    changed: bool,
}

#[derive(Default)]
struct StepLedHint {
    used: [bool; 8],
    seen: Option<(u64, [Option<bool>; 8])>,
    loaded_at: Option<u64>,
    changed_at: Option<u64>,
    pending: Option<LedStep>,
    text: Option<String>,
    until: Option<std::time::Instant>,
}

impl StepLedHint {
    fn clear(&mut self) {
        self.text = None;
        self.until = None;
    }

    fn differs(&self, a: [Option<bool>; 8], b: [Option<bool>; 8]) -> bool {
        (0..8).any(|i| self.used[i] && a[i] != b[i])
    }

    fn observe(&mut self, s: &Shared, used: [bool; 8], now: std::time::Instant) -> Option<String> {
        if self.used != used {
            *self = Self {
                used,
                ..Self::default()
            };
        }
        if s.running {
            self.clear();
            self.pending = None;
        } else if self.until.is_some_and(|until| now >= until) {
            self.clear();
        }
        if s.published == 0 || self.seen.is_some_and(|(p, _)| p == s.published) {
            return None;
        }
        self.loaded_at.get_or_insert(s.cycles);
        let changed = self
            .seen
            .is_some_and(|(_, leds)| self.differs(leds, s.leds));
        if changed {
            self.changed_at = Some(s.cycles);
            self.clear();
        }
        self.seen = Some((s.published, s.leds));
        let different = self
            .pending
            .as_ref()
            .is_some_and(|step| self.differs(step.leds, s.leds));
        let step = self.pending.as_mut()?;
        step.changed |= changed || different;
        if s.running || s.published == step.publication || s.cycles != step.target {
            return None;
        }
        let step = self.pending.take().unwrap();
        if step.changed || !self.used.iter().any(|&used| used) {
            return None;
        }
        let count = step.from.abs_diff(s.cycles);
        let since = s
            .cycles
            .saturating_sub(self.changed_at.or(self.loaded_at).unwrap());
        let history = self.changed_at.map_or_else(
            || {
                format!(
                    "The LEDs have not changed since cycle {}",
                    self.loaded_at.unwrap()
                )
            },
            |_| format!("They last changed {since} cycles ago"),
        );
        self.text = Some(format!(
            "No LED changed in this {count}-cycle step. {history}; try a larger step (1M)."
        ));
        self.until = Some(now + Duration::from_secs(8));
        Some(format!("hint step-leds {count} {since}"))
    }

    fn dispatch(
        &mut self,
        s: &Shared,
        used: [bool; 8],
        command: &Command,
        now: std::time::Instant,
    ) -> Option<String> {
        let note = self.observe(s, used, now);
        if matches!(command, Command::Run) {
            self.clear();
            self.pending = None;
        } else if !s.running {
            let target = match *command {
                Command::Step(n) => Some(s.cycles.saturating_add(n)),
                Command::Back(n) => Some(s.cycles.saturating_sub(n).max(s.earliest)),
                Command::GoTo(n) => Some(n.max(s.earliest)),
                _ => None,
            };
            if let Some(target) = target {
                self.pending = Some(LedStep {
                    publication: s.published,
                    from: s.cycles,
                    target,
                    leds: s.leds,
                    changed: false,
                });
            }
        }
        note
    }
}

fn status_hint_height(ui: &egui::Ui, text: Option<&str>) -> f32 {
    text.map_or(0.0, |text| {
        ui.painter()
            .layout(
                text.to_owned(),
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().text_color(),
                (ui.available_width() - 16.0).max(1.0),
            )
            .size()
            .y
            + ui.spacing().item_spacing.y
    })
}

/// An explanation of a common design delay, without changing its inputs.
#[derive(Default)]
struct DebounceHint {
    holds: [Option<ButtonHold>; 7],
    text: Option<String>,
}

impl DebounceHint {
    const CYCLES: f64 = 65_536.0;

    fn height(&self, ui: &egui::Ui) -> f32 {
        status_hint_height(ui, self.text.as_deref())
    }

    fn update(
        &mut self,
        held: [bool; 7],
        now: std::time::Instant,
        cycles: u64,
        rate: Option<f64>,
    ) -> Option<String> {
        for (hold, down) in self.holds.iter_mut().zip(held) {
            if down {
                hold.get_or_insert(ButtonHold {
                    since: now,
                    cycle: cycles,
                    noted: false,
                });
            } else {
                *hold = None;
            }
        }
        self.text = None;
        let rate = rate.filter(|r| r.is_finite() && *r > 0.0 && Self::CYCLES / r > 10.0)?;
        let (i, hold) = self.holds.iter_mut().enumerate().find_map(|(i, hold)| {
            hold.as_mut()
                .filter(|h| now.saturating_duration_since(h.since) >= Duration::from_millis(1500))
                .map(|h| (i, h))
        })?;
        let name = board::BUTTONS[i].name;
        let cycles = cycles.saturating_sub(hold.cycle);
        let seconds = (Self::CYCLES / rate).ceil() as u64;
        let wait = if seconds >= 3600 {
            format!("{} h {} min", seconds / 3600, seconds % 3600 / 60)
        } else if seconds >= 60 {
            format!("{} min {} s", seconds / 60, seconds % 60)
        } else {
            format!("{seconds} s")
        };
        self.text = Some(format!(
            "{name} held for {cycles} cycles. Designs often debounce a button for about 65 536 cycles (2.6 ms at 25 MHz); at {rate} cycles/s that takes {wait}. Raise the rate (real time) while holding."
        ));
        if hold.noted {
            None
        } else {
            hold.noted = true;
            Some(format!("hint debounce {name} {cycles} {rate}"))
        }
    }
}

/// The status line: what the run is doing and how fast, in fields of fixed
/// width so that nothing moves as the numbers change.
#[allow(clippy::too_many_arguments)]
fn status_bar(
    ui: &mut egui::Ui,
    stage: &str,
    error: Option<&str>,
    cycles: u64,
    mhz: f64,
    rate: Option<f64>,
    fast: Option<&live::FastChoice>,
    watch_status: Option<&str>,
    fps: f64,
) {
    ui.horizontal(|ui| {
        status_field(ui, 170.0, error.map_or(stage, |_| "Error"));
        status_field(ui, 180.0, &format!("cycle {cycles:>14}"));
        let seconds = cycles as f64 * board::CLOCK_PERIOD_NS as f64 * 1e-9;
        status_field(ui, 185.0, &format!("board {seconds:>13.6} s"));
        let speed = if mhz >= 0.01 {
            format!("{mhz:>10.2} MHz")
        } else {
            format!("{:>10.0} cycles/s", mhz * 1e6)
        };
        status_field(ui, 150.0, &speed);
        let rate = rate.map_or_else(
            || "rate          max".into(),
            |r| format!("rate {r:>12.3} /s"),
        );
        status_field(ui, 165.0, &rate);
        let fast = fast.map_or_else(
            || "fast: preparing".into(),
            |f| {
                format!(
                    "fast mode: {:>2} threads, folding {:>2}",
                    f.threads, f.folding
                )
            },
        );
        status_field(ui, 265.0, &fast);
        status_field(ui, 200.0, watch_status.unwrap_or(""));
        status_field(ui, 80.0, &format!("{fps:>5.1} fps"));
    });
}

/// Width-to-height aspect of the 3D board viewport. The ULX3S PCB is
/// 94 × 51 mm (about 1.84), and the tilted 3D camera wants a bit more
/// vertical room, so 1.6 fits the board snugly without empty gaps.
const BOARD_VIEW_ASPECT: f32 = 1.6;

/// Height in points for the 3D board view given the panel's `width` and the
/// `available_height` left under the header.
fn board_view_height(width: f32, available_height: f32) -> f32 {
    if !width.is_finite()
        || !available_height.is_finite()
        || width <= 0.0
        || available_height <= 0.0
    {
        return 0.0;
    }
    (width / BOARD_VIEW_ASPECT).min(available_height)
}

/// The board's share of the space above the UART, kept for this session.
/// Preferences store themes and recent paths, rather than UI sizes.
#[derive(Default)]
struct BoardUartSplit {
    share: Option<f32>,
    drag_start: f32,
}

impl BoardUartSplit {
    const BOARD_MIN: f32 = 160.0;
    const UART_MIN: f32 = 160.0;
    const HANDLE: f32 = 8.0;

    fn show(&mut self, ui: &mut egui::Ui, fallback_height: f32) -> (egui::Rect, egui::Rect) {
        let rect = ui.available_rect_before_wrap();
        let height = (rect.height() - Self::HANDLE).max(0.0);
        // At very small window sizes, share the space without overflowing it.
        let scale = (height / (Self::BOARD_MIN + fallback_height + Self::UART_MIN)).min(1.0);
        let min = (Self::BOARD_MIN + fallback_height) * scale;
        let max = (height - Self::UART_MIN * scale).max(min);
        let default = board_view_height(rect.width(), height) + fallback_height;
        let mut board = self
            .share
            .map_or(default, |share| height * share)
            .clamp(min, max);
        let handle = |board| {
            egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + board),
                egui::vec2(rect.width(), Self::HANDLE),
            )
        };
        let response = ui
            .interact(
                handle(board),
                ui.id().with("board_uart_split"),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag to resize the board and UART; double-click to reset");
        if response.double_clicked() {
            self.share = None;
            board = default.clamp(min, max);
        } else if response.dragged() && height > 0.0 {
            if response.drag_started() {
                self.drag_start = board;
            }
            board = (self.drag_start + response.total_drag_delta().unwrap_or_default().y)
                .clamp(min, max);
            self.share = Some(board / height);
        }
        let handle = handle(board);
        let stroke = ui.style().interact(&response).bg_stroke;
        ui.painter()
            .hline(handle.x_range(), handle.center().y, stroke);
        (
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), handle.top())),
            egui::Rect::from_min_max(egui::pos2(rect.left(), handle.bottom()), rect.max),
        )
    }
}

/// Received UART text scrolls within its pane and follows new output until scrolled back.
fn uart_text(ui: &mut egui::Ui, text: &str) -> egui::scroll_area::ScrollAreaOutput<egui::Response> {
    egui::ScrollArea::vertical()
        .id_salt("uart_console")
        .stick_to_bottom(true)
        .min_scrolled_height(0.0)
        .max_height(ui.available_height())
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace()).wrap())
        })
}

/// Compact summary card shown under the board when the design uses no UART.
fn design_card(
    ui: &mut egui::Ui,
    design: &ecp5_emu::live::Static,
    oled: Option<(u32, u32)>,
    hdmi: Option<&str>,
) {
    let muted = theme::colours(ui.visuals().dark_mode).muted;
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.strong("Design");
        ui.add(egui::Label::new(RichText::new(&design.summary).small()).wrap());
        let leds: Vec<String> = (0..8)
            .filter(|&i| design.leds[i])
            .map(|i| format!("D{i}"))
            .collect();
        let buttons: Vec<&str> = board::BUTTONS
            .iter()
            .enumerate()
            .filter(|(i, _)| design.buttons[*i])
            .map(|(_, b)| b.name)
            .collect();
        let mut parts = Vec::new();
        if !leds.is_empty() {
            parts.push(format!("LEDs: {}", leds.join(", ")));
        }
        if !buttons.is_empty() {
            parts.push(format!("Buttons: {}", buttons.join(", ")));
        }
        if let Some((w, h)) = oled {
            parts.push(format!("OLED: {w}×{h}"));
        }
        if let Some(mode) = hdmi {
            parts.push(format!("HDMI: {mode}"));
        }
        let parts_text = if parts.is_empty() {
            "Board parts: none".to_owned()
        } else {
            parts.join(" · ")
        };
        ui.add(egui::Label::new(RichText::new(parts_text).small().color(muted)).wrap());
    });
}

/// The board's LEDs and buttons as a row each, for when the 3D board cannot
/// be shown; returns the buttons held down here.
fn led_and_button_rows(
    ui: &mut egui::Ui,
    used: &[bool; 8],
    buttons: &[bool; 7],
    leds: &[Option<bool>; 8],
    held: &[bool; 7],
) -> [bool; 7] {
    ui.separator();
    ui.label("LEDs (D0 … D7)");
    ui.horizontal(|ui| {
        for (i, led) in leds.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
            let colour = match (used[i], led) {
                (false, _) => theme::colours(ui.visuals().dark_mode).muted,
                (true, Some(true)) => theme::colours(ui.visuals().dark_mode).error,
                (true, Some(false)) => theme::colours(ui.visuals().dark_mode).zero,
                (true, None) => theme::colours(ui.visuals().dark_mode).unknown,
            };
            ui.painter().circle_filled(rect.center(), 8.0, colour);
            response.on_hover_text(format!(
                "led[{i}] on {}{}",
                board::LEDS[i],
                if used[i] {
                    ""
                } else {
                    ": not used by the design"
                }
            ));
        }
    });
    ui.label("Buttons (hold to press)");
    let mut down = [false; 7];
    ui.horizontal_wrapped(|ui| {
        for (i, button) in board::BUTTONS.iter().enumerate() {
            let response = ui
                .add_enabled(buttons[i], egui::Button::new(button.name).selected(held[i]))
                .on_hover_text(format!("btn[{i}] on {}", button.ball))
                .on_disabled_hover_text(format!(
                    "btn[{i}] on {}: not used by the design",
                    button.ball
                ));
            down[i] = response.is_pointer_button_down_on();
        }
    });
    down
}

/// SW1 beside the fallback rows. Labels and space stay the same while the
/// run reports new positions; pending requests move the levers at once.
fn switch_rows(ui: &mut egui::Ui, used: &[bool; 4], on: &[bool; 4]) -> Vec<usize> {
    let mut clicks = Vec::new();
    ui.vertical(|ui| {
        ui.label("SW1 (click to toggle)");
        for i in 0..4 {
            let label = format!("SW1 switch {}: sw[{i}]", i + 1);
            let hint = format!(
                "{label}, {}{}",
                if on[i] { "ON" } else { "OFF" },
                if used[i] {
                    "; click to toggle"
                } else {
                    ", not read by the design"
                }
            );
            let response = ui
                .add_enabled_ui(used[i], |ui| {
                    ui.add_sized(
                        [170.0, 22.0],
                        egui::Button::new(label).small().selected(on[i]),
                    )
                })
                .inner
                .on_hover_text(&hint)
                .on_disabled_hover_text(&hint);
            if response.clicked() {
                clicks.push(i);
            }
        }
    });
    clicks
}

/// Board lever numbers start at one; the runner's switch indices start at
/// zero. Scripts may only change levers read by the ready design.
fn switch_command(
    n: Option<&str>,
    state: Option<&str>,
    used: &[bool; 4],
) -> anyhow::Result<Command> {
    use anyhow::{bail, Context};
    let n = n
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=4).contains(n))
        .context("switch number must be 1..4")?;
    let on = match state {
        Some("on") => true,
        Some("off") => false,
        _ => bail!("switch position must be on or off"),
    };
    if !used[n - 1] {
        bail!("SW1 switch {n}: sw[{}] is not read by the design", n - 1);
    }
    Ok(Command::Switch(n - 1, on))
}

fn switch_click(i: usize, on: &[bool; 4]) -> Command {
    Command::Switch(i, !on[i])
}

fn switch_request_note(seconds: f64, cycle: u64, i: usize, on: bool) -> String {
    format!(
        "switch requested\t{seconds:.3}\t{cycle}\t{}\t{}",
        i + 1,
        if on { "on" } else { "off" }
    )
}

/// Panel scripts use exact names and a single open/close action.
fn panel_command(text: &str) -> anyhow::Result<(Panel, bool)> {
    use anyhow::{bail, Context};
    let words: Vec<_> = text.split_whitespace().collect();
    let ["panel", name, action] = words.as_slice() else {
        bail!("expected panel NAME open|close");
    };
    let panel =
        Panel::from_name(name).with_context(|| format!("unknown peripheral panel {name:?}"))?;
    let open = match *action {
        "open" => true,
        "close" => false,
        _ => bail!("panel action must be open or close"),
    };
    Ok((panel, open))
}

/// A positive number as typed: `1500`, `1 500`, `1_500`, `2.5k`, `3M`,
/// `1G`.
fn amount(text: &str) -> Option<f64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect::<String>()
        .replace(',', ".");
    let (number, scale) = match t.chars().last()? {
        'k' | 'K' => (&t[..t.len() - 1], 1e3),
        'm' | 'M' => (&t[..t.len() - 1], 1e6),
        'g' | 'G' => (&t[..t.len() - 1], 1e9),
        _ => (t.as_str(), 1.0),
    };
    let v = number.parse::<f64>().ok()? * scale;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// A cycle number as typed: 0, or a number as `amount` takes it.
fn cycle_number(text: &str) -> Option<u64> {
    if text.trim() == "0" {
        return Some(0);
    }
    amount(text).map(|v| v.round() as u64)
}

/// A number as one would type it: `25M`, `2.5k`, `3`.
fn short(v: f64) -> String {
    let (n, unit) = if v >= 1e9 {
        (v / 1e9, "G")
    } else if v >= 1e6 {
        (v / 1e6, "M")
    } else if v >= 1e3 {
        (v / 1e3, "k")
    } else {
        (v, "")
    };
    let n = format!("{n:.6}");
    format!("{}{unit}", n.trim_end_matches('0').trim_end_matches('.'))
}

/// UART bytes as terminal text: CR LF is a line break, a lone CR is dropped.
fn append_text(console: &mut String, bytes: &[u8]) {
    for &b in bytes {
        match b {
            b'\r' => {}
            b'\n' | b'\t' | 0x20..=0x7e => console.push(b as char),
            _ => console.push_str(&format!("\\x{b:02x}")),
        }
    }
    const KEEP: usize = 1 << 16;
    if console.len() > KEEP {
        let cut = console.len() - KEEP;
        let cut = (cut..console.len())
            .find(|&i| console.is_char_boundary(i))
            .unwrap_or(0);
        console.drain(..cut);
    }
}

/// UART bytes as the record writes them: printable ASCII as it is, and
/// `\r`, `\n`, `\t`, `\\` and `\xHH` for the rest, so a line holds them all.
fn record_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'\r' => text.push_str("\\r"),
            b'\n' => text.push_str("\\n"),
            b'\t' => text.push_str("\\t"),
            b'\\' => text.push_str("\\\\"),
            0x20..=0x7e => text.push(b as char),
            _ => text.push_str(&format!("\\x{b:02x}")),
        }
    }
    text
}

impl eframe::App for App {
    /// The script's pointer, as if the mouse were there.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(at) = self.pointer {
            raw_input.events.push(egui::Event::PointerMoved(at));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_open(ui.ctx());
        let started = std::time::Instant::now();
        self.waves.recording = self.fabric.trace.borrow().is_some();
        // A scripted point reaches egui through the next raw input, so its
        // watch waits for that frame's map to find the wire there.
        let script_pointed = std::mem::take(&mut self.script_pointed);
        self.frames.push_back(started);
        while self
            .frames
            .front()
            .is_some_and(|t| started.duration_since(*t).as_secs_f64() > 1.0)
        {
            self.frames.pop_front();
        }
        let fps = self.frames.len() as f64
            / started
                .duration_since(self.opened_at)
                .as_secs_f64()
                .clamp(0.001, 1.0);
        ui.ctx().request_repaint_after(Duration::from_millis(200));
        if self.commands.is_none() && self.start_screen {
            self.scripted(ui);
            if !self.script.is_empty() {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            let mut action = self.scripted_action.take();
            egui::Panel::top("start_controls").show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(a) = self.design_menu(ui, None) {
                        action = Some(a);
                    }
                    self.peripherals_segment(ui);
                    if let Some(a) = self.new_project_segment(ui) {
                        action = Some(a);
                    }
                });
            });
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(a) = start::screen(
                    ui,
                    &self.preferences,
                    &self.demos,
                    self.browser_error.as_deref(),
                ) {
                    action = Some(a);
                }
            });
            if let Some(action) = action {
                self.action(ui.ctx(), action);
            }
            return;
        }
        // A copy of what the emulator published; the console text is taken.
        let (
            stage,
            error,
            ready,
            running,
            rate,
            cycles,
            mhz,
            fast,
            leds,
            switches,
            new,
            (earliest, ahead),
            toggled,
            (wave_runs, edges),
            watch_status,
            step_led_note,
        ) = {
            let mut s = self.shared.lock().unwrap();
            // After going back the console starts again from the run's text
            // up to there.
            if let Some(all) = s.console.take() {
                self.console.clear();
                append_text(&mut self.console, &all);
            }
            let text = std::mem::take(&mut s.text);
            append_text(&mut self.console, &text);
            // The record's console lines: the seconds since the design was
            // ready, the run's cycle when the window took the bytes, the bytes.
            if !text.is_empty() {
                if let Some(t) = self.fabric.trace.borrow_mut().as_mut() {
                    let at = self.ready_at.map_or(0.0, |r| r.elapsed().as_secs_f64());
                    let _ = t.note(&format!(
                        "uart\t{at:.3}\t{}\t{}",
                        s.cycles,
                        self.paths.text(&record_text(&text))
                    ));
                }
            }
            (
                s.stage.clone(),
                s.error.clone(),
                s.ready.clone(),
                s.running,
                s.rate,
                s.cycles,
                s.mhz,
                s.fast.clone(),
                s.leds,
                s.switches,
                self.taken.take(&s),
                (s.earliest, s.ahead),
                std::mem::take(&mut s.toggled),
                (std::mem::take(&mut s.waves), s.edges),
                s.watch_status.clone(),
                self.step_leds.observe(
                    &s,
                    s.ready.as_ref().map_or(self.step_leds.used, |s| s.leds),
                    started,
                ),
            )
        };
        if let Some(note) = step_led_note {
            self.record_note(&note);
        }
        if ready.is_some() {
            self.switch_requests.report(switches);
        }
        if ready.is_none() && error.is_none() {
            if let Some(load) = self.loading.as_mut() {
                if let Some(note) = load.observe(&stage, started) {
                    if let Some(record) = self.fabric.trace.get_mut() {
                        let _ = record.note(&self.paths.text(&note));
                    }
                }
            }
        }
        // The watched nets' rows: the edges recorded ahead after going back
        // are kept while the run has them (`ahead` is in cycles).
        self.wave_rows_received |= wave_runs.iter().any(|run| {
            Some(run.id) == self.watch_id
                && run.width == self.waves.nets.len()
                && run.width > 0
                && run.rows.iter().any(|&value| value <= 3)
        });
        self.waves
            .take(wave_runs, edges, if ahead > 0 { ahead * 2 + 1 } else { 0 });
        let (values, lit) = (Arc::clone(&self.taken.values), Arc::clone(&self.taken.lit));
        // What the board's peripherals show (src/peripherals): the pictures
        // are shared, not copied.
        let peripherals = self.shared.lock().unwrap().peripherals.clone();
        // ← and → go back and on by a step, where no text is typed.
        if ready.is_some() && !running && !ui.ctx().egui_wants_keyboard_input() {
            let (back, on) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                )
            });
            if back && cycles > earliest {
                self.send(Command::Back(self.steps()));
            }
            if on {
                self.send(Command::Step(self.steps()));
            }
        }
        if let Some(s) = &ready {
            self.poll_sources();
            if let Some(name) = self.pending_net.take() {
                // A port's name is on each net at its pad: the largest.
                self.fabric.net = s
                    .net_names
                    .iter()
                    .filter(|(_, names)| {
                        names
                            .iter()
                            .any(|n| *n == name || n.starts_with(&format!("{name} (")))
                    })
                    .map(|(net, _)| *net)
                    .max_by_key(|&net| s.overlay.pieces_of(net).len());
            }
            if self.cell_at.is_empty() {
                self.cell_at = s
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ((c.row, c.col), i))
                    .collect();
                self.logic_at = s
                    .geometry
                    .logic
                    .iter()
                    .enumerate()
                    .map(|(i, t)| ((t.row, t.col), i))
                    .collect();
            }
        }
        if running {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        if ready.is_some() && self.ready_at.is_none() {
            self.ready_at = Some(std::time::Instant::now());
        }
        if ready.is_some() && self.seen_switches != Some(switches) {
            self.seen_switches = Some(switches);
            if let Some(record) = self.fabric.trace.borrow_mut().as_mut() {
                let at = self.ready_at.map_or(0.0, |r| r.elapsed().as_secs_f64());
                let _ = record.switches(at, cycles, switches);
            }
        }
        self.scripted(ui);
        if !self.script.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        // Every change spreads from its net's driver at the pace of the run
        // and glows on (src/changes.rs).
        // A frame with nothing new from the run takes nothing in.
        let motion = Motion::of(running, rate);
        let updating = std::time::Instant::now();
        if new || !toggled.is_empty() {
            self.fabric.update(&lit, &toggled, motion);
        }
        let update_ms = updating.elapsed().as_secs_f64() * 1e3;
        self.fabric.wires = self.map == Map::Fabric;
        // A change is drawn every frame (every vsync) while it spreads or
        // glows, so its glow fades smoothly. As fast as it goes the run
        // publishes about 30 times a second (each publish asks for a frame):
        // the frames between take nothing from it and cost the window little.
        if self.fabric.animating() {
            ui.ctx().request_repaint();
        }

        let mut action = if self.toolbar.hosted {
            None
        } else {
            self.plain_toolbar(ui)
        };
        self.update_debounce_hint(started, cycles, rate);
        let hint_height =
            self.debounce.height(ui) + status_hint_height(ui, self.step_leds.text.as_deref());
        // Reserve the status area now, then draw it after this frame's board
        // input so a release also removes the hint in this frame.
        let mut status_ui = egui::Panel::bottom("status")
            .exact_size(if self.probe_status.is_some() {
                52.0 + hint_height
            } else {
                28.0 + hint_height
            })
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new()))
            .inner;

        self.board.recording = self.fabric.trace.borrow().is_some();
        self.board.ready = self.ready_at;
        // The board, left of the die and as tall as the window: the ULX3S in
        // 3D (its LEDs lit, its buttons to hold down) over its UART console
        // or the design card. 600 of the window's 1500 points at the start
        // (the first frame's window is not yet its full size, so not a share
        // of it).
        // The buttons that went down or up this frame, for the record.
        let mut pressed: Vec<(usize, bool)> = Vec::new();
        egui::Panel::left("board")
            .default_size(600.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("ULX3S");
                    ui.separator();
                    self.board.toolbar(ui);
                });
                let (leds_used, buttons_used) = ready
                    .as_ref()
                    .map_or(([false; 8], [false; 7]), |s| (s.leds, s.buttons));
                let switches_used = ready.as_ref().map_or([false; 4], |s| s.switches);
                let switches_on = if ready.is_some() {
                    self.switch_requests.drawn(switches)
                } else {
                    [false; 4]
                };
                self.board.set_switches(switches_used, switches_on);
                let oled = peripherals.oled_screen();
                let hdmi = peripherals.hdmi_mode();
                // The OLED module and the HDMI cable on the 3D board, while the
                // design drives them.
                self.board
                    .set_oled(
                        oled.as_ref()
                            .map(|(width, height, rgba, serial)| BoardScreen {
                                width: *width,
                                height: *height,
                                rgba: Arc::clone(rgba),
                                serial: *serial,
                            }),
                    );
                self.board
                    .set_hdmi(hdmi.as_ref().map(|mode| BoardHdmi { mode: mode.clone() }));
                // Keep fallback controls above the divider when no 3D renderer is available.
                let fallback_height = if self.board.available() { 0.0 } else { 128.0 };
                let split = ready
                    .as_ref()
                    .filter(|s| s.uart_tx || s.uart_rx)
                    .map(|_| self.board_uart.show(ui, fallback_height));
                let board_rect = split.map_or_else(|| ui.available_rect_before_wrap(), |s| s.0);
                let (on_board, in_row, switch_clicks) = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(board_rect), |ui| {
                        ui.set_clip_rect(ui.clip_rect().intersect(board_rect));
                        // The renderer fits its camera to this viewport's aspect.
                        let board_size = egui::vec2(
                            ui.available_width(),
                            split.map_or_else(
                                || board_view_height(ui.available_width(), ui.available_height()),
                                |(board, _)| (board.height() - fallback_height).max(0.0),
                            ),
                        );
                        let on_board = ui
                            .allocate_ui(board_size, |ui| {
                                let inputs = BoardInputs {
                                    lit: std::array::from_fn(|i| {
                                        leds_used[i] && leds[i] == Some(true)
                                    }),
                                    values: leds,
                                    leds_used,
                                    buttons_used,
                                    held: self.held,
                                };
                                #[cfg(test)]
                                {
                                    self.board_publication = Some(BoardPublication {
                                        published: self.taken.published.unwrap_or(0),
                                        edges,
                                        running,
                                        used: leds_used,
                                        leds,
                                        lit: inputs.lit,
                                    });
                                }
                                self.board.show(ui, &inputs)
                            })
                            .inner;
                        // Without the 3D board (no model built, no GPU), the LEDs and
                        // buttons as a row each.
                        let mut in_row = [false; 7];
                        let mut switch_clicks = self.board.take_switch_clicks();
                        if !self.board.available() {
                            ui.horizontal(|ui| {
                                let width =
                                    (ui.available_width() - 170.0 - ui.spacing().item_spacing.x)
                                        .max(0.0);
                                in_row = ui
                                    .allocate_ui_with_layout(
                                        egui::vec2(width, 0.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            led_and_button_rows(
                                                ui,
                                                &leds_used,
                                                &buttons_used,
                                                &leds,
                                                &self.held,
                                            )
                                        },
                                    )
                                    .inner;
                                switch_clicks.extend(switch_rows(ui, &switches_used, &switches_on));
                            });
                        }
                        (on_board, in_row, switch_clicks)
                    })
                    .inner;
                for i in switch_clicks {
                    let on = self.switch_requests.drawn(switches);
                    self.send(switch_click(i, &on));
                }
                // Held on the board, in the rows or by the script.
                for i in 0..7 {
                    let down = buttons_used[i]
                        && (on_board == Some(i) || in_row[i] || self.script_held[i]);
                    if down != self.held[i] {
                        self.held[i] = down;
                        self.send(Command::Button(i, down));
                        pressed.push((i, down));
                    }
                }
                let console_rect = split.map_or_else(|| ui.available_rect_before_wrap(), |s| s.1);
                ui.scope_builder(egui::UiBuilder::new().max_rect(console_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(console_rect));
                    match &ready {
                        None => {
                            let status = if error.is_some() {
                                "Could not load design"
                            } else {
                                "Loading design…"
                            };
                            ui.add_sized(
                                [ui.available_width(), 18.0],
                                egui::Label::new(RichText::new(status).small()).truncate(),
                            )
                            .on_hover_text(status);
                        }
                        Some(s) if s.uart_tx || s.uart_rx => {
                            // The design in a line (all of it on hover), then its
                            // UART console in the rest of the panel.
                            ui.add_sized(
                                [ui.available_width(), 18.0],
                                egui::Label::new(RichText::new(&s.summary).small()).truncate(),
                            )
                            .on_hover_text(&s.summary);
                            ui.separator();
                            ui.label(format!(
                                "UART {} baud (TX {}, RX {})",
                                board::BAUD,
                                if s.uart_tx { board::UART_TX } else { "unused" },
                                if s.uart_rx { board::UART_RX } else { "unused" }
                            ));
                            let input = ui.add_enabled(
                                s.uart_rx,
                                egui::TextEdit::singleline(&mut self.line)
                                    .hint_text("type, Enter sends it with CR"),
                            );
                            if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                let mut bytes = std::mem::take(&mut self.line).into_bytes();
                                bytes.push(b'\r');
                                self.send(Command::Keys(bytes));
                                input.request_focus();
                            }
                            uart_text(ui, &self.console);
                        }
                        Some(s) => {
                            ui.add_space(4.0);
                            design_card(
                                ui,
                                s,
                                oled.as_ref().map(|(w, h, _, _)| (*w, *h)),
                                hdmi.as_deref(),
                            );
                        }
                    }
                });
            });

        // The HDMI monitor and the OLED, as windows over the rest.
        for command in self.monitor.show(ui.ctx(), &peripherals) {
            self.send(command);
        }
        // The board's lines of the record: its LEDs as the GPU drew them;
        // then the monitor's and the OLED's pictures.
        let mut notes = self.board.take_notes();
        notes.extend(self.monitor.take_notes());
        // The buttons held or let go (by the board, the rows or the script):
        // `# button N down|up <seconds since ready> <cycle>`.
        let at = self.ready_at.map_or(0.0, |r| r.elapsed().as_secs_f64());
        notes.extend(pressed.iter().map(|&(i, down)| {
            format!(
                "button\t{i}\t{}\t{at:.3}\t{cycles}",
                if down { "down" } else { "up" }
            )
        }));
        if !notes.is_empty() {
            if let Some(t) = self.fabric.trace.borrow_mut().as_mut() {
                for n in notes {
                    let _ = t.note(&n);
                }
            }
        }

        self.update_debounce_hint(started, cycles, rate);
        if let Some(text) = &self.debounce.text {
            status_ui.add(egui::Label::new(text).wrap());
        }
        if let Some(text) = &self.step_leds.text {
            status_ui.add(egui::Label::new(text).wrap());
        }
        if let Some(status) = self.probe_status.clone() {
            status_ui.horizontal(|ui| {
                ui.add_sized(
                    [(ui.available_width() - 150.0).max(50.0), 22.0],
                    egui::Label::new(&status).truncate(),
                )
                .on_hover_text(&status);
                if !self.probed.is_empty() && ui.button("Watch in Waves").clicked() {
                    self.watch_probed();
                }
            });
        }
        status_bar(
            &mut status_ui,
            &stage,
            error.as_deref(),
            cycles,
            mhz,
            rate,
            fast.as_ref(),
            watch_status.as_deref(),
            fps,
        );

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.fabric.picked = None;
        }
        if let (Some(s), Some(picked)) = (&ready, self.fabric.picked.clone()) {
            egui::Panel::right("inspector")
                .default_size(400.0)
                .show(ui, |ui| {
                    match self.inspector.show(
                        ui,
                        s,
                        &lit,
                        &picked,
                        &|v| self.fabric.glow(v),
                        self.fabric.values,
                    ) {
                        Some(inspector::Action::Close) => self.fabric.picked = None,
                        Some(inspector::Action::Net(net)) => {
                            self.fabric.net = Some(net);
                            self.map = Map::Fabric;
                        }
                        None => {}
                    }
                });
        }

        egui::Panel::bottom("tile").default_size(120.0).show(ui, |ui| {
            // What is pointed at scrolls within the panel: the panel keeps its
            // height, or a long net's text would grow it over the pointer, the
            // pointer leave the wire, the text shrink back, and so on in a loop.
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                fabric_view::legend(ui, ready.as_deref(), self.fabric.values, self.fabric.wires);
                ui.separator();
                let Some(s) = &ready else {
                    return;
                };
                // The net pointed at or kept: its HDL names, where it starts
                // and what it feeds.
                if let (Map::Fabric, Some(net)) = (self.map, self.fabric.shown_net()) {
                    let o = &s.overlay;
                    let f = &s.fabric;
                    let tile = |p: &[f32; 2]| (f.row_at(p[1]), (p[0] / TILE_W) as u16);
                    let names = s.net_names.get(&net);
                    let shown: Vec<&str> = names
                        .map(|n| n.iter().take(4).map(String::as_str).collect())
                        .unwrap_or_default();
                    let more = names.map_or(0, |n| n.len().saturating_sub(4));
                    let from: Vec<String> = o
                        .drivers_of(net)
                        .iter()
                        .map(|p| {
                            let (r, c) = tile(p);
                            format!("R{r}C{c}")
                        })
                        .collect();
                    let mut tiles: Vec<(u16, u16)> = o.fed_of(net).iter().map(tile).collect();
                    tiles.sort();
                    tiles.dedup();
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            theme::colours(ui.visuals().dark_mode).net,
                            if shown.is_empty() {
                                "net (no HDL name)".to_string()
                            } else {
                                format!(
                                    "net {}{}",
                                    shown.join(", "),
                                    if more > 0 { format!(" … and {more} more") } else { String::new() }
                                )
                            },
                        );
                        let start = match (o.tied(net), from.is_empty()) {
                            (Some(v), _) => format!("tied to {} by a CIB", u8::from(v)),
                            (None, true) => "from a wire not drawn".to_string(),
                            (None, false) => format!("from {}", from.join(", ")),
                        };
                        ui.label(format!(
                            "{start}, to {} site pins in {} tiles{}",
                            o.fed_of(net).len(),
                            tiles.len(),
                            if self.fabric.net == Some(net) { "  (kept; click off a wire to let go)" } else { "" }
                        ));
                    });
                    ui.separator();
                }
                let at = self
                    .fabric
                    .hovered
                    .or(self.fabric.selected)
                    .and_then(|p| self.cell_at.get(&p).copied());
                let Some(i) = at else {
                    ui.label("Point at the map for a position; click to keep it. Scroll to zoom, drag to pan, double-click to fit.");
                    return;
                };
                let c = &s.cells[i];
                ui.horizontal(|ui| {
                    ui.strong(format!("R{}C{}", c.row, c.col));
                    ui.label(class_name(c.class));
                    ui.label(format!("{} settings and {} routing transfers in use", c.fields, c.routes));
                });
                ui.label(RichText::new(c.tiles.join(", ")).small());
                if !c.blocks.is_empty() {
                    ui.label(format!("executes: {}", c.blocks.join(", ")));
                }
                if let Some(&k) = self.logic_at.get(&(c.row, c.col)) {
                    let t = &s.geometry.logic[k];
                    let bits = |used: u8, of: &[u32; 8]| -> String {
                        (0..8)
                            .rev()
                            .map(|n| match (used >> n & 1, values.get(of[n] as usize)) {
                                (0, _) => '.',
                                (_, Some(1)) => '1',
                                (_, Some(0)) => '0',
                                _ => 'x',
                            })
                            .collect()
                    };
                    ui.monospace(format!(
                        "LUT7..0 out {}   FF7..0 Q {}   (. = not used)",
                        bits(t.luts, &t.f),
                        bits(t.flip_flops, &t.q)
                    ));
                }
                if !c.names.is_empty() {
                    let shown: Vec<&str> = c.names.iter().take(12).map(String::as_str).collect();
                    ui.label(RichText::new(format!(
                        "HDL names driven here: {}{}",
                        shown.join(", "),
                        if c.names.len() > 12 { format!(" … and {} more", c.names.len() - 12) } else { String::new() }
                    ))
                    .small());
                }
            });
        });

        // The waves above the tile's line, under the map: its cursor goes to
        // a cycle, and the map shows it.
        if self.waves.open {
            let shown_net = self.fabric.net;
            self.waves.publish(edges, &lit);
            let panel = egui::Panel::bottom("waves")
                .resizable(true)
                .default_size(240.0)
                .min_size(96.0)
                .show(ui, |ui| {
                    self.waves.show(
                        ui,
                        ready.as_deref(),
                        edges,
                        shown_net,
                        watch_status.as_deref(),
                    )
                });
            self.waves_drawn = self.wave_rows_received
                && panel.response.rect.width() > 220.0
                && panel.response.rect.height() > 40.0;
            let goto = panel.inner;
            if let Some(cycle) = goto {
                self.send(Command::GoTo(cycle));
            }
        }
        if let Some(action) = self.waves.row_action.take() {
            self.waves_action(action);
        }
        egui::CentralPanel::default().show(ui, |ui| match (&ready, &error) {
            (_, Some(e)) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading("Could not load the design");
                    ui.colored_label(theme::colours(ui.visuals().dark_mode).error, e);
                    if ui.button("Open another design…").clicked() {
                        action = Some(Action::Open);
                    }
                });
            }
            (Some(s), None) => {
                let timing = s.timing.get();
                let path = self.critical.path(timing);
                self.fabric.critical = path.clone();
                let shown = path.as_ref().map(|p| p.path.domain.label());
                if self.critical.shown != shown {
                    self.critical.shown = shown;
                    if let Some(path) = &path {
                        for (n, hop) in path.hops.iter().enumerate() {
                            self.probe_note(hop.note(n + 1));
                        }
                    }
                }
                let central = ui.available_rect_before_wrap();
                let (map, shelf) = timing_layout(central, self.critical.on);
                ui.scope_builder(egui::UiBuilder::new().max_rect(map), |ui| {
                    self.fabric
                        .show(ui, &s.fabric, Some((&s.overlay, &lit)), Some(&s.net_names));
                });
                let (centre, cell) = self.fabric.camera_centre();
                let evidence = (central, shelf, self.fabric.map_rect, centre, cell);
                if let Some(note) = map_layout_note(&mut self.layout_evidence, evidence) {
                    self.record_note(&note);
                }
                if let Some(shelf) = shelf {
                    let mut action = TimingAction::default();
                    ui.scope_builder(egui::UiBuilder::new().max_rect(shelf), |ui| {
                        action =
                            timing_panel(ui, &mut self.critical, timing, self.fabric.vhdl_sources);
                    });
                    if let Some(hop) = action.focus {
                        self.fabric.centre_hop(&hop);
                        ui.ctx().request_repaint();
                    }
                    if let Some((name, locations, chosen)) = action.source {
                        self.show_source(name, locations, chosen);
                    }
                }
                self.load_die_shown();
            }
            (None, None) => {
                if self.loading.is_some() {
                    let map = ui.available_rect_before_wrap();
                    if let Some(fabric) = live::chip_fabric() {
                        self.fabric.show(ui, &fabric, None, None);
                        ui.painter()
                            .rect_filled(map, 0.0, egui::Color32::from_black_alpha(42));
                        self.load_die_shown();
                    }
                    if let Some(load) = &self.loading {
                        load.card(ui, map, &self.title, std::time::Instant::now());
                    }
                } else {
                    ui.centered_and_justified(|ui| ui.label(format!("{stage} …")));
                }
            }
        });
        if ready.is_some() {
            if let Some(mut load) = self.loading.take() {
                if let Some(record) = self.fabric.trace.get_mut() {
                    let _ = record.note(
                        &self
                            .paths
                            .text(&format!("load {:.3} {stage}", load.elapsed(started))),
                    );
                }
                let durations = load.finish(started);
                self.load_times.remember(load.path, durations);
                if let Err(e) = self.load_times.save() {
                    eprintln!("Saving load times: {e}");
                }
            }
        } else if error.is_some() {
            self.loading = None;
        }
        // Both W and the script use the net this frame's map found, and
        // the keyboard check after text fields have had their turn.
        if let Some(s) = &ready {
            if s.sources.get().is_some() {
                self.poll_sources();
                while let Some((x, y)) = self.script_sources.pop_front() {
                    let (target, locations) = self.fabric.source_at(&s.fabric, &s.overlay, x, y);
                    if let Some(chosen) = locations.first().map(|location| location.span.clone()) {
                        self.show_source(target, locations, chosen);
                    } else {
                        self.probe_note("probe map none -> no source at this point".into());
                    }
                }
            }
            if let Some((target, locations, chosen)) = self.fabric.show_source.take() {
                self.show_source(target, locations, chosen);
            }
            if let Some(net) = self.fabric.watch_net.take() {
                self.waves.add_menu(s, net);
            }
            let keyboard = ui.ctx().egui_wants_keyboard_input();
            let net = self.fabric.pointed_net();
            if ui.input(|i| i.key_pressed(egui::Key::W)) {
                if !keyboard && net.is_none() && self.fabric.has_probe() {
                    self.watch_probed();
                } else {
                    self.waves.add_pointed(s, net, keyboard, "W");
                }
            }
            for _ in 0..script_pointed {
                self.waves.add_pointed(s, net, keyboard, "script");
            }
        }
        if let Some(watch) = self.waves.command() {
            self.send(watch);
        }
        let seconds = self.ready_at.map_or(0.0, |r| r.elapsed().as_secs_f64());
        let notes = self.waves.notes(seconds, watch_status.as_deref(), edges);
        if let Some(record) = self.fabric.trace.borrow_mut().as_mut() {
            for line in notes {
                let _ = record.note(&self.paths.text(&format!("waves\t{seconds:.3}\t{line}")));
            }
        }
        if let Some(action) = action.or_else(|| self.scripted_action.take()) {
            self.action(ui.ctx(), action);
            return;
        }
        if ready.is_some() {
            let shown = Shown {
                ui_ms: started.elapsed().as_secs_f64() * 1e3,
                update_ms,
                trace_ms: self.trace_ms,
                running,
                rate,
                cycles,
                mhz,
                values: self.fabric.values,
                zoom: self.fabric.zoom_level(),
                map_height: self.fabric.map_rect.height(),
                pointing: self.fabric.shown_net().is_some(),
                dropped: self.fabric.dropped(),
                gpu: self.fabric.gpu_measurements(),
            };
            let draw_ms = self.fabric.draw_ms;
            let tracing = std::time::Instant::now();
            if let Some(t) = self.fabric.trace.get_mut() {
                if let Err(e) = t.frame(draw_ms, &shown) {
                    eprintln!("trace: {e:#}");
                }
            }
            self.trace_ms = tracing.elapsed().as_secs_f64() * 1e3;
            // Where the board and the die are, when that changes.
            if let Some(board) = self.board.rect {
                let now = (board, self.fabric.map_rect);
                if self.layout != Some(now) {
                    self.layout = Some(now);
                    let r = |r: egui::Rect| {
                        format!(
                            "x {:.0}..{:.0} y {:.0}..{:.0}",
                            r.min.x, r.max.x, r.min.y, r.max.y
                        )
                    };
                    let window = ui.ctx().content_rect();
                    if let Some(t) = self.fabric.trace.get_mut() {
                        let _ = t.note(&format!(
                            "layout\twindow {}\tboard {}\tdie map {}",
                            r(window),
                            r(board),
                            r(self.fabric.map_rect)
                        ));
                    }
                }
            }
        }
    }
}

fn map_probe_note(target: &str, locations: &[SourceLocation]) -> String {
    format!(
        "probe map {target} -> {}",
        locations
            .iter()
            .map(|location| location.span.label())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn source_labels(locations: &[SourceLocation]) -> String {
    let mut labels = Vec::new();
    for location in locations {
        let label = location.span.label();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels.join(",")
}

fn waves_command(command: &str) -> Option<(bool, &str)> {
    let (verb, name) = command.trim().split_once(char::is_whitespace)?;
    let source = match verb {
        "waves-chip" => false,
        "waves-source" => true,
        _ => return None,
    };
    let name = name.trim();
    (!name.is_empty()).then_some((source, name))
}

fn source_jump_or_copy(ctx: &egui::Context, project: bool, chosen: Span) -> Option<Span> {
    if project {
        Some(chosen)
    } else {
        ctx.copy_text(chosen.label());
        None
    }
}

/// The report visibility and selected clock for the current design.
#[derive(Default)]
struct Critical {
    on: bool,
    clock: Option<String>,
    recorded: bool,
    shown: Option<String>,
}
impl Critical {
    fn command(&mut self, value: &str) {
        match value {
            "off" => self.on = false,
            "on" => self.on = true,
            clock => {
                self.on = true;
                self.clock = Some(clock.into());
            }
        }
    }
    fn path(&self, timing: Option<&Published>) -> Option<Arc<MappedPath>> {
        if !self.on {
            return None;
        }
        let paths = timing?.result.as_ref().ok()?;
        paths
            .iter()
            .find(|p| {
                self.clock.as_ref().is_none_or(|c| {
                    p.path.domain.label() == *c
                        || p.path.domain.clock() == Some(c.as_str())
                        || p.label == *c
                })
            })
            .cloned()
    }
}
#[cfg(test)]
fn critical_toggle(ui: &mut egui::Ui, critical: &mut Critical) {
    ui.toggle_value(&mut critical.on, "Critical path")
        .on_hover_text("Show nextpnr's estimated critical path and its source locations.");
}
type MapLayout = (egui::Rect, Option<egui::Rect>, egui::Rect, egui::Vec2, f32);

/// Keep the immediately preceding camera even when no geometry note is needed.
fn map_layout_note(previous: &mut Option<MapLayout>, current: MapLayout) -> Option<String> {
    let prior = previous.replace(current);
    let (central, shelf, map, centre, cell) = current;
    if prior.is_some_and(|p| (p.0, p.1, p.2) == (central, shelf, map)) {
        return None;
    }
    let before = prior.unwrap_or(current);
    let rect = |r: egui::Rect| {
        format!(
            "{:.3},{:.3},{:.3},{:.3}",
            r.min.x, r.min.y, r.max.x, r.max.y
        )
    };
    Some(format!(
        "map-layout report={} central={} shelf={} map={} resized={} before_centre={:.6},{:.6} before_cell={:.6} centre={:.6},{:.6} cell={:.6}",
        if shelf.is_some() { "on" } else { "off" }, rect(central),
        shelf.map_or_else(|| "-".into(), rect), rect(map), u8::from(prior.is_some()),
        before.3.x, before.3.y, before.4, centre.x, centre.y, cell,
    ))
}

fn timing_layout(rect: egui::Rect, report: bool) -> (egui::Rect, Option<egui::Rect>) {
    if !report {
        return (rect, None);
    }
    let width = 340.0_f32.min(rect.width() * 0.45);
    let map = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - width - 8.0, rect.max.y));
    let shelf = egui::Rect::from_min_max(egui::pos2(map.max.x + 8.0, rect.min.y), rect.max);
    (map, Some(shelf))
}
#[derive(Default)]
struct TimingAction {
    focus: Option<MappedHop>,
    source: Option<(String, Vec<SourceLocation>, Span)>,
}
fn timing_row(n: usize, hop: &MappedHop) -> String {
    format!(
        "{n:>2}  {:<8}  {:.2} ns  total {:.2} ns\n{}\n{}",
        hop.hop.kind.word(),
        hop.hop.curr,
        hop.hop.total,
        hop.name,
        hop.locations
            .first()
            .map_or_else(|| "source: -".into(), Span::label)
    )
}
fn timing_panel(
    ui: &mut egui::Ui,
    critical: &mut Critical,
    timing: Option<&Published>,
    vhdl: bool,
) -> TimingAction {
    let mut action = TimingAction::default();
    let mut close = ui.rect_contains_pointer(ui.max_rect())
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    ui.horizontal(|ui| {
        ui.heading("nextpnr timing report");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= ui
                .button("×")
                .on_hover_text("Close the timing report (turns Critical path off)")
                .clicked();
        });
    });
    if close {
        critical.command("off");
        ui.ctx().request_repaint();
        return action;
    }
    ui.label("nextpnr's estimate, not measured by the emulator");
    let Some(timing) = timing else {
        ui.label("Waiting for nextpnr.log…");
        return action;
    };
    let paths = match &timing.result {
        Ok(paths) => paths,
        Err(error) => {
            ui.label(error);
            return action;
        }
    };
    if paths.len() > 1 {
        egui::ComboBox::from_id_salt("critical_clock")
            .width((ui.available_width() - 30.0).max(40.0))
            .selected_text(
                critical
                    .path(Some(timing))
                    .map_or_else(|| "Choose clock / domain".into(), |p| p.label.clone()),
            )
            .show_ui(ui, |ui| {
                for path in paths {
                    ui.selectable_value(
                        &mut critical.clock,
                        Some(path.path.domain.label()),
                        &path.label,
                    );
                }
            });
    }
    let Some(path) = critical.path(Some(timing)) else {
        ui.label("No critical path for this clock / domain");
        return action;
    };
    ui.label(&path.label)
        .on_hover_text(path.path.domain.label());
    if let Some(f) = path.path.frequency {
        ui.label(format!("Fmax {:.2} MHz", f.mhz));
        ui.label(f.constraint.map_or_else(
            || "constraint: -; slack: -".into(),
            |c| {
                format!(
                    "constraint {c:.2} MHz; slack {:+.2} ns",
                    path.path.slack().unwrap()
                )
            },
        ));
    } else {
        ui.label("Fmax / constraint / slack: - (cross-domain path)");
    }
    ui.label(format!(
        "total {:.2} ns; {:.2} logic + {:.2} routing",
        path.path.total(),
        path.path.logic,
        path.path.routing
    ))
    .on_hover_text("nextpnr rounds each displayed value independently to 0.01 ns.");
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("critical_hops")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, hop) in path.hops.iter().enumerate() {
                let mut text = timing_row(i + 1, hop);
                if let Some(reason) = hop.route_not_shown() {
                    text.push_str("\nroute not shown: ");
                    text.push_str(reason);
                }
                let lines = text.lines().count() as f32;
                let height = lines * ui.text_style_height(&egui::TextStyle::Small)
                    + (lines - 1.0) * ui.spacing().item_spacing.y
                    + 8.0;
                let (id, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
                let background = ui.painter().add(egui::Shape::Noop);
                // Truncate each line independently, so even a generated
                // name thousands of characters long keeps its source row.
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(4.0, 3.0))),
                    |ui| {
                        for line in text.lines() {
                            ui.add(
                                egui::Label::new(RichText::new(line).monospace().small())
                                    .truncate()
                                    .selectable(false),
                            );
                        }
                    },
                );
                let response = ui.interact(rect, id, egui::Sense::click());
                if response.hovered() {
                    ui.painter().set(
                        background,
                        egui::Shape::rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill),
                    );
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &text)
                });
                let response = response.on_hover_text(&text);
                if response.clicked() {
                    action.focus = Some(hop.clone());
                }
                response.context_menu(|ui| {
                    let span = hop.locations.first();
                    if ui
                        .add_enabled(span.is_some(), egui::Button::new("Show in source"))
                        .on_disabled_hover_text(if vhdl {
                            ecp5_emu::flow::report::VHDL_SOURCE_LIMIT
                        } else {
                            "no source line for this timing hop"
                        })
                        .clicked()
                    {
                        let locations = hop
                            .locations
                            .iter()
                            .map(|span| SourceLocation {
                                span: span.clone(),
                                kind: sources::SourceKind::Cell,
                            })
                            .collect();
                        action.source = Some((hop.name.clone(), locations, span.unwrap().clone()));
                        ui.close();
                    }
                });
            }
        });
    action
}

fn source_point(command: &str) -> Option<(f32, f32)> {
    let mut words = command.split_whitespace();
    if words.next()? != "show-source" {
        return None;
    }
    let x = words.next()?.parse::<f32>().ok()?;
    let y = words.next()?.parse::<f32>().ok()?;
    (words.next().is_none()
        && x.is_finite()
        && y.is_finite()
        && (0.0..=1.0).contains(&x)
        && (0.0..=1.0).contains(&y))
    .then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::{
        amount, board_view_height, cycle_number, panel_command, short, status_bar, switch_click,
        switch_command, switch_request_note, switch_rows, Panel, SwitchRequests, Taken,
        BOARD_VIEW_ASPECT,
    };
    use ecp5_emu::live::{self, Command, FastChoice, Shared};
    use eframe::egui;
    use std::sync::Arc;

    #[test]
    fn dev_script_persists_and_records_only_explicit_changes() {
        use crate::{start, test_temp};
        let root = test_temp::demo_fixture();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Dev script".into(), None);
        let prefs = root.join("recent.txt");
        app.isolated_preferences(prefs.clone());
        app.start_screen = true;
        let record = root.join("window.tsv");
        app.trace(&record).unwrap();
        for (commands, expected) in [
            ("0 theme light\n0 values on\n", false),
            ("0 dev on\n", true),
            ("0 dev on\n0 theme dark\n", true),
            ("0 dev maybe\n0 dev off extra\n", true),
            ("0 dev off\n", false),
        ] {
            let script = root.join("window.script");
            std::fs::write(&script, commands).unwrap();
            app.script(&script).unwrap();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
            assert_eq!(app.dev_mode(), expected, "{commands}");
            for _ in 0..3 {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
                assert_eq!(app.dev_mode(), expected);
            }
            if prefs.exists() {
                assert_eq!(
                    start::Preferences::read(Some(prefs.clone()))
                        .unwrap()
                        .dev_mode,
                    expected
                );
            }
        }
        drop(app);
        let text = std::fs::read_to_string(record).unwrap();
        assert_eq!(
            text.lines()
                .filter(|s| s.starts_with("# dev "))
                .collect::<Vec<_>>(),
            ["# dev on", "# dev off"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_dev_controls_save_immediately_and_record_changes() {
        use crate::{start, test_temp};
        use eframe::App as _;
        for via_menu in [false, true] {
            let root = test_temp::demo_fixture();
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = super::App::new(&cc, "Dev controls".into(), None);
            let prefs = root.join("recent.txt");
            app.isolated_preferences(prefs.clone());
            app.demos = start::demos(&root).unwrap();
            app.start_screen = true;
            let record = root.join("window.tsv");
            app.trace(&record).unwrap();
            let frame = |app: &mut super::App, events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.ui(ui, &mut eframe::Frame::_new_kittest()),
                )
            };
            let position = |out: &egui::FullOutput, label: &str| {
                test_temp::painted_text(out)
                    .iter()
                    .find(|(s, _)| s == label)
                    .unwrap_or_else(|| {
                        panic!("missing {label}: {:?}", test_temp::painted_text(out))
                    })
                    .1
                    .center()
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            for expected in [true, false] {
                let mut out = frame(&mut app, vec![]);
                if via_menu {
                    let at = position(&out, "Design ▾");
                    frame(&mut app, test_temp::click(at, true));
                    frame(&mut app, test_temp::click(at, false));
                    out = frame(&mut app, vec![]);
                }
                // The menu entry is painted after the gallery toggle; choose its
                // last matching label when both are present.
                let at = if via_menu {
                    test_temp::painted_text(&out)
                        .iter()
                        .rev()
                        .find(|(s, _)| s == "Dev mode")
                        .unwrap()
                        .1
                        .center()
                } else {
                    position(&out, "Dev mode")
                };
                frame(&mut app, test_temp::click(at, true));
                frame(&mut app, test_temp::click(at, false));
                assert_eq!(app.dev_mode(), expected, "menu={via_menu}");
                assert_eq!(
                    start::Preferences::read(Some(prefs.clone()))
                        .unwrap()
                        .dev_mode,
                    expected
                );
                for _ in 0..3 {
                    frame(&mut app, vec![]);
                }
                assert_eq!(app.dev_mode(), expected);
            }
            drop(app);
            let text = std::fs::read_to_string(record).unwrap();
            assert_eq!(
                text.lines()
                    .filter(|s| s.starts_with("# dev "))
                    .collect::<Vec<_>>(),
                ["# dev on", "# dev off"]
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn design_records_use_injected_home_outside_the_project_root() {
        design_record_paths(false);
    }

    #[test]
    fn design_records_keep_project_relative_configs() {
        design_record_paths(true);
    }

    fn design_record_paths(under_root: bool) {
        use crate::start::{display_paths::DisplayPaths, reader};
        use std::path::Path;
        let temp = crate::test_temp::unique_dir("ecp5-design-record-");
        let home = temp.join("private home");
        let root = home.join("work/project");
        let cwd = home.join("work");
        let cases = if under_root {
            vec![(
                Some(root.clone()),
                root.join("build/top.config"),
                "build/top.config",
            )]
        } else {
            vec![
                (
                    None,
                    home.join("reference/top.config"),
                    "~/reference/top.config",
                ),
                (
                    Some(root.clone()),
                    home.join("reference/top.config"),
                    "~/reference/top.config",
                ),
                (
                    Some(root.clone()),
                    cwd.join("reference/top.config"),
                    "reference/top.config",
                ),
            ]
        };
        for (i, (source_root, config, label)) in cases.into_iter().enumerate() {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx);
            let mut app = super::App::new(&cc, "test".into(), None);
            app.defer_native_load = true;
            app.reader_check = || reader::fixed(reader::Status::Ready);
            app.set_display_paths(DisplayPaths::with_environment(
                Some(cwd.clone()),
                Some(home.clone()),
                None,
            ));
            if let Some(root) = source_root {
                app.project_sources(root, []);
            }
            let record = temp.join(format!("window-{i}.tsv"));
            app.trace(&record).unwrap();
            app.open(
                "test".into(),
                ecp5_emu::live::Design {
                    config,
                    names: None,
                    run: false,
                },
            );
            drop(app);
            let text = std::fs::read_to_string(record).unwrap();
            assert!(
                text.lines()
                    .any(|line| line == format!("# design\t{label}")),
                "{text}"
            );
            assert!(!text.contains(home.to_str().unwrap()), "{text}");
            assert!(!text.contains(temp.to_str().unwrap()), "{text}");
            assert!(!text.contains("prebuilt/top.config"), "{text}");
            let design = text
                .lines()
                .find_map(|line| line.strip_prefix("# design\t"))
                .unwrap();
            assert!(!Path::new(design).is_absolute());
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn window_records_hide_paths_in_scripts_probes_loading_uart_and_waves() {
        use crate::start::display_paths::DisplayPaths;
        let temp = crate::test_temp::unique_dir("ecp5-window-record-paths-");
        let home = temp.join("private home");
        let file = home.join("shared/tool.log");
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "test".into(), None);
        app.set_display_paths(DisplayPaths::with_environment(
            Some(home.join("work")),
            Some(home.clone()),
            None,
        ));
        let record = temp.join("window.tsv");
        app.trace(&record).unwrap();
        for marker in ["reader", "start build", "tour failed"] {
            app.record_note(&format!("{marker} reading '{}'", file.display()));
        }
        for marker in [
            "probe source",
            "probe map",
            "critical path",
            "critical hop",
            "value",
        ] {
            app.probe_note(format!("{marker} reading '{}'", file.display()));
        }
        let notes = app.take_probe_notes();
        assert_eq!(notes.len(), 5);
        assert!(notes.iter().all(|line| line.contains("~/shared/tool.log")));
        app.ready_at = Some(std::time::Instant::now());
        app.script = vec![(0.0, format!("waves-source {}", file.display()))];
        app.loading = Some(super::loading::Loading::new(
            file.clone(),
            Vec::new(),
            std::time::Instant::now(),
        ));
        {
            let mut shared = app.shared.lock().unwrap();
            shared.stage = format!("reading '{}'", file.display());
            shared.text = format!("reading '{}'", file.display()).into_bytes();
            shared.watch_status = Some(format!("reading '{}'", file.display()));
        }
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            eframe::App::ui(&mut app, ui, &mut frame);
        });
        assert!(app.script.is_empty());
        drop(app);
        let text = std::fs::read_to_string(record).unwrap();
        assert!(!text.contains(home.to_str().unwrap()), "{text}");
        assert!(!text.contains(temp.to_str().unwrap()), "{text}");
        for marker in [
            "# reader",
            "# start build",
            "# tour failed",
            "# probe source",
            "# probe map",
            "# critical path",
            "# critical hop",
            "# value",
            "# script\t",
            "# uart\t",
            "# load ",
            "# waves\t",
        ] {
            assert!(
                text.lines()
                    .any(|line| line.starts_with(marker) && line.contains("~/shared/tool.log")),
                "{marker}: {text}"
            );
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    fn timing_fixture() -> ecp5_emu::timing_path::Published {
        use ecp5_emu::timing_path::{self, MappedHop, MappedPath};
        let path = timing_path::parse(
            include_str!("../../tests/fixtures/timing_path/blinky_cycle/creator.txt"),
            include_str!("../../tests/fixtures/timing_path/blinky_cycle/nextpnr.log"),
        )
        .unwrap()
        .paths
        .remove(0);
        let hops = path
            .hops
            .iter()
            .map(|hop| MappedHop {
                hop: hop.clone(),
                name: hop.name(),
                nets: vec![],
                sites: vec![],
                locations: vec![ecp5_emu::sources::Span {
                    file: "top.v".into(),
                    line: 7,
                    column: Some(1),
                    end_line: 7,
                    end_column: Some(4),
                }],
            })
            .collect();
        timing_path::Published {
            result: Ok(vec![Arc::new(MappedPath {
                label: "clk_25mhz".into(),
                path,
                hops,
            })]),
            seconds: 0.0,
            bytes: 0,
        }
    }

    #[test]
    fn source_values_loading_failure_and_records_are_read_only() {
        use crate::project::editor::{Probe, ValueView};
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = super::App::new(&cc, "Values".into(), None);
        assert_eq!(
            app.source_value("top.v", "count").detail,
            ["Emulate this project to see values"]
        );
        let (commands, received) = std::sync::mpsc::channel();
        app.commands = Some(commands);
        app.shared.lock().unwrap().stage = "reading configuration".into();
        assert_eq!(
            app.source_value("top.v", "count").detail,
            ["Design is loading: reading configuration"]
        );
        app.shared.lock().unwrap().error = Some("fixture failed".into());
        assert_eq!(
            app.source_value("top.v", "count").detail,
            ["Design failed to load: fixture failed"]
        );
        let probe = Probe {
            path: "top.v".into(),
            line: 4,
            column: 2,
            identifier: "count".into(),
        };
        let mut view = ValueView::unavailable("seeking…");
        for _ in 0..3 {
            app.source_value_note(view.note("top.v", &probe), &view);
        }
        assert_eq!(
            app.take_probe_notes(),
            ["# value top.v:4:2 count -> seeking…"]
        );
        view.rows = vec!["count = 1".into()];
        view.cycle = Some(7);
        view.published = 8;
        view.edges = 14;
        let path =
            std::env::temp_dir().join(format!("ecp5-source-value-note-{}.tsv", std::process::id()));
        app.trace(&path).unwrap();
        for _ in 0..3 {
            app.source_value_note(view.note("top.v", &probe), &view);
        }
        view.published += 1;
        view.edges += 1;
        app.source_value_note(view.note("top.v", &probe), &view);
        drop(app.fabric.trace.get_mut().take());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.lines()
                .filter(|s| *s == "# value top.v:4:2 count = 1 @ 7")
                .count(),
            2
        );
        assert!(!text.contains("# #"));
        std::fs::remove_file(path).unwrap();
        assert!(received.try_recv().is_err());
        assert!(app.probed.is_empty() && app.waves.nets.is_empty());
        assert!(app.taken.published.is_none() && app.taken.lit.is_empty());
    }

    #[test]
    fn sparse_source_reads_preserve_missing_entries_and_waves_sampling_limits() {
        use super::{SourceValues, ValueTarget, ValueWire};
        use crate::{
            project::editor::format_values,
            waves::{Watched, Waves},
        };
        use ecp5_emu::{fast::NOT_SAMPLED, live::WaveRun};
        let targets = vec![ValueTarget {
            name: "signal".into(),
            bits: vec![ValueWire {
                name: "signal".into(),
                index: None,
                alternatives: vec![(17, Some(3))],
            }],
        }];
        let ctx = egui::Context::default();
        let mut waves = Waves::default();
        waves.nets = vec![Watched {
            name: "signal".into(),
            net: 17,
            wire: 3,
        }];
        waves.set_open(true);
        let id = match waves.command() {
            Some(Command::Watch { id, .. }) => id,
            _ => 0,
        };
        // Program switch at edge 3: the historical folded row at edge 2
        // stays NOT_SAMPLED, even after a full snapshot is available.
        waves.take(
            vec![WaveRun {
                id,
                first: 1,
                width: 1,
                rows: vec![0, NOT_SAMPLED, 1],
            }],
            3,
            0,
        );
        let mut s = Shared {
            published: 1,
            cycles: 1,
            edges: 3,
            lit: vec![2, 2, 2, 1],
            ..Default::default()
        };
        let rows = SourceValues::read(&s, &targets);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bits.len(), 1);
        assert_eq!(rows[0].bits[0].alternatives, [(17, Some(1))]);
        let render = |waves: &mut Waves, edge| {
            let frame = |waves: &mut Waves, events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 500.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        waves.show(ui, None, edge, None, None);
                    },
                )
            };
            let mut out = frame(waves, vec![egui::Event::PointerGone]);
            if edge == 2 {
                let shape = out
                    .shapes
                    .iter()
                    .find(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "signal"))
                    .expect("watched signal row");
                let egui::Shape::Text(t) = &shape.shape else {
                    unreachable!()
                };
                // With no cursor the column reads edge 2, which was folded,
                // rather than the newer retained sample (1 at edge 3).
                let value = out
                    .shapes
                    .iter()
                    .find(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "?"))
                    .expect("current edge 2 is NOT_SAMPLED");
                // Default scale is two points per edge. This is edge 2,
                // one edge left of the retained edge 3, after the switch.
                let pointer = egui::pos2(value.clip_rect.right() - 2.0, t.pos.y + 5.0);
                out = frame(waves, vec![egui::Event::PointerMoved(pointer)]);
            }
            out.shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(render(&mut waves, 3).contains(&"1".into()));
        assert_eq!(format_values(&rows).0, ["signal = 1"]);
        waves.take(Vec::new(), 2, 1);
        let historical = render(&mut waves, 2);
        assert!(historical.contains(&"Cursor @ 1".into()), "{historical:?}");
        assert!(historical.contains(&"?".into()), "{historical:?}");
        // Between edges a new input can change the publication at edge 3;
        // its Waves sample remains the value from that edge.
        s.lit[3] = 0;
        s.published += 1;
        let rows = SourceValues::read(&s, &targets);
        assert_eq!(format_values(&rows).0, ["signal = 0"]);
        assert!(render(&mut waves, 3).contains(&"1".into()));
        // Publication encoding merges X/Z/unavailable to x; Waves retains Z.
        s.lit[3] = 2;
        s.published += 1;
        s.edges = 4;
        s.cycles = 2;
        waves.take(
            vec![WaveRun {
                id,
                first: 4,
                width: 1,
                rows: vec![3],
            }],
            4,
            0,
        );
        assert!(render(&mut waves, 4).contains(&"Z".into()));
        assert_eq!(
            format_values(&SourceValues::read(&s, &targets)).0,
            ["signal = x"]
        );
        let missing = vec![ValueTarget {
            name: "missing".into(),
            bits: vec![ValueWire {
                name: "missing".into(),
                index: None,
                alternatives: vec![(18, None), (19, Some(1000))],
            }],
        }];
        assert_eq!(
            SourceValues::read(&s, &missing)[0].bits[0].alternatives,
            [(18, None), (19, None)]
        );
    }

    #[test]
    #[ignore = "headless header_probe_driven and blinky_cycle references, native reader and database"]
    fn released_header_pull_up_reaches_board_led_input_while_running() {
        use std::{
            collections::HashSet,
            sync::Mutex,
            time::{Duration, Instant},
        };
        let references = std::env::var_os("ECP5_TEST_REFERENCE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/reference")
            });
        // An empty ECP5_HEADER and an unset one both release every pin. Use
        // explicit per-run levels too, so another test's env cannot drive GN0.
        let released = ecp5_emu::board::parse_header(Some("")).unwrap();
        assert_eq!(released, ecp5_emu::board::parse_header(None).unwrap());
        for name in ["header_probe_driven", "blinky_cycle"] {
            let root = references.join(name);
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = super::App::new(&cc, name.into(), None);
            let shared = Arc::new(Mutex::new(Shared::default()));
            let repaint = ctx.clone();
            app.commands = Some(live::spawn_with(
                live::Design {
                    config: root.join("top.config"),
                    names: Some(root.join("top_routed.json")),
                    run: false,
                },
                live::RunOptions::default().header(released),
                shared.clone(),
                move || repaint.request_repaint(),
            ));
            app.shared = shared.clone();
            let deadline = Instant::now() + Duration::from_secs(240);
            loop {
                let s = shared.lock().unwrap();
                assert!(s.error.is_none(), "{:?}", s.error);
                if s.published > 0 {
                    let design = s.ready.as_ref().unwrap();
                    assert!(design.leds[0], "{name}: B2 must be a used LED");
                    println!(
                        "{name} initial: used={:?} shared LEDs={:?}",
                        design.leds, s.leds
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "{}", s.stage);
                drop(s);
                std::thread::sleep(Duration::from_millis(10));
            }
            app.rate(None);
            app.run();
            let mut frame = eframe::Frame::_new_kittest();
            let started = Instant::now();
            let (mut frames, mut publications) = (0, HashSet::new());
            while started.elapsed() < Duration::from_secs(1) {
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1500.0, 950.0),
                        )),
                        ..Default::default()
                    },
                    |ui| eframe::App::ui(&mut app, ui, &mut frame),
                );
                let shown = app.board_publication.expect("App must call board.show");
                if shown.running {
                    assert!(shown.used[0]);
                    assert_eq!(
                        shown.lit,
                        std::array::from_fn(|i| shown.used[i] && shown.leds[i] == Some(true))
                    );
                    if name == "header_probe_driven" {
                        assert_eq!(shared.lock().unwrap().leds[0], Some(true));
                        assert_eq!(shown.leds[0], Some(true), "snapshot {shown:?}");
                        assert!(shown.lit[0], "board input {shown:?}");
                    }
                    if publications.insert(shown.published) {
                        println!("{name} board input: {shown:?}");
                    }
                    assert!(shown.edges > 0);
                    frames += 1;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                frames >= 2 && publications.len() >= 2,
                "{name}: {frames} frames, {} publications",
                publications.len()
            );
            app.send(Command::Pause);
        }
    }

    /// Exercise real publications and the same name lookup used by source tabs.
    #[test]
    #[ignore = "headless blinky reference design, native reader and device database"]
    fn source_port_values_and_waves_match_board_reference() {
        use super::App;
        use crate::waves::Waves;
        use ecp5_emu::{live, sources::NameLookup};
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/blinky_cycle");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn(
            live::Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let design = loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if sh.published > 0 && sh.ready.as_ref().is_some_and(|d| d.sources.get().is_some()) {
                break sh.ready.clone().unwrap();
            }
            assert!(Instant::now() < deadline, "{}", sh.stage);
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        };
        let baseline = {
            let sh = shared.lock().unwrap();
            (sh.published, sh.cycles)
        };
        commands.send(Command::Step(1)).unwrap();
        loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if sh.published != baseline.0 && !sh.running && sh.cycles == baseline.1 + 1 {
                break;
            }
            assert!(Instant::now() < deadline);
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        }
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, "Port values".into(), None);
        app.shared = shared.clone();
        app.commands = Some(commands.clone());
        let mut waves = Waves::default();
        assert_eq!(waves.add_name(&design, "led"), 8);
        {
            let sh = shared.lock().unwrap();
            println!(
                "port evidence cycle={} edges={} board={:?}",
                sh.cycles, sh.edges, sh.leds
            );
            let NameLookup::Resolved(rows) =
                design.sources.get().unwrap().name_only("top.v", "led")
            else {
                panic!("led layout")
            };
            for bit in &rows[0].bits {
                for &net in &bit.targets {
                    let wire = design
                        .overlay
                        .pieces_of(net)
                        .first()
                        .map(|p| design.overlay.pieces[*p as usize].value as usize);
                    let geometry = wire.and_then(|v| {
                        design
                            .geometry
                            .values
                            .iter()
                            .position(|w| *w == design.overlay.values[v])
                    });
                    println!(
                        "{} -> net {net}, labels={:?}, wire={:?}, lit={:?}, values={:?}",
                        bit.name,
                        design.net_names.get(&net),
                        wire.map(|v| &design.overlay.values[v]),
                        wire.and_then(|v| sh.lit.get(v)),
                        geometry.and_then(|v| sh.values.get(v))
                    );
                }
            }
            for watched in &waves.nets {
                println!(
                    "watch {} -> net {} wire {} lit={:?}",
                    watched.name,
                    watched.net,
                    watched.wire,
                    sh.lit.get(watched.wire as usize)
                );
            }
        }
        let mut count_waves = Waves::default();
        assert_eq!(count_waves.add_name(&design, "count"), 8);
        assert_eq!(
            waves
                .nets
                .iter()
                .map(|w| (w.net, w.wire))
                .collect::<Vec<_>>(),
            count_waves
                .nets
                .iter()
                .map(|w| (w.net, w.wire))
                .collect::<Vec<_>>(),
            "watch led must select exactly count's data nets, in the same bit order"
        );
        let Command::Watch { id, wires, keep } = waves.command().unwrap() else {
            panic!("led watch")
        };
        let Command::Watch {
            wires: count_wires, ..
        } = count_waves.command().unwrap()
        else {
            panic!("count watch")
        };
        assert_eq!(wires, count_wires);
        commands.send(Command::Watch { id, wires, keep }).unwrap();
        let snapshot = |app: &mut App| {
            let (published, cycles, edges, running, leds) = {
                let sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{:?}", sh.error);
                (sh.published, sh.cycles, sh.edges, sh.running, sh.leds)
            };
            let led = app.source_value("top.v", "led");
            let count = app.source_value("top.v", "count");
            let input = app.source_value("top.v", "clk_25mhz");
            if [led.published, count.published, input.published]
                .iter()
                .any(|p| *p != published)
            {
                return None; // Retry if a publication crossed the two queries.
            }
            let expected = leds.iter().enumerate().fold(0u64, |value, (bit, high)| {
                value | (u64::from(high.expect("blinky LED probe")) << bit)
            });
            assert_eq!(led.displayed(), count.displayed());
            assert_eq!(
                led.displayed(),
                format!("0x{expected:02x} ({expected}) (bits 7..0 found)")
            );
            assert_eq!(
                input.displayed(),
                (edges % 2).to_string(),
                "incoming oscillator level"
            );
            assert_eq!((led.cycle, led.edges), (Some(cycles), edges));
            Some((published, cycles, edges, running, expected))
        };
        let initial = snapshot(&mut app).unwrap();
        assert!(!initial.3, "paused after Step");
        commands.send(Command::Rate(Some(4.0))).unwrap();
        commands.send(Command::Run).unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        let mut last = initial.0;
        let mut publications = 0;
        let mut numbers = std::collections::HashSet::new();
        loop {
            if let Some((published, cycles, _, running, number)) = snapshot(&mut app) {
                if published != last {
                    assert_eq!(published, last + 1, "check every publication at 4 cycles/s");
                    last = published;
                    publications += 1;
                    numbers.insert(number);
                }
                if running && publications >= 16 && cycles >= initial.1 + 6 {
                    break;
                }
            }
            assert!(Instant::now() < until, "4 cycles/s publications");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            numbers.len() >= 6,
            "numeric values must change, not just agree at zero"
        );
        commands.send(Command::Pause).unwrap();
        let paused = loop {
            if let Some(view) = snapshot(&mut app) {
                if !view.3 {
                    break view;
                }
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        };
        commands.send(Command::Step(5)).unwrap();
        loop {
            if let Some(view) = snapshot(&mut app) {
                if !view.3 && view.0 > paused.0 && view.1 == paused.1 + 5 {
                    break;
                }
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        let sh = shared.lock().unwrap();
        let mut next = initial.2 + 1;
        for run in sh.waves.iter().filter(|run| run.id == id) {
            assert_eq!(run.width, 8);
            assert_eq!(run.first, next, "contiguous watched edges");
            for (offset, row) in run.rows.chunks_exact(8).enumerate() {
                let edge = run.first + offset as u64;
                let count = (initial.4 + edge.div_ceil(2) - initial.2.div_ceil(2)) & 255;
                for (column, value) in row.iter().enumerate() {
                    assert_eq!(
                        *value,
                        ((count >> (7 - column)) & 1) as u8,
                        "led/count bit {} at edge {edge}",
                        7 - column
                    );
                }
            }
            next += run.edges();
        }
        assert_eq!(next, sh.edges + 1);
        assert!(next > initial.2 + 16);
        println!("port regression: {publications} consecutive running publications, {} changing values, {} matching watched edges; paused Step and incoming clock match", numbers.len(), next - initial.2 - 1);
    }

    #[test]
    #[ignore = "headless debounce reference design, native reader and device database"]
    fn source_button_port_matches_board_press_and_release_reference() {
        use super::App;
        use crate::waves::Waves;
        use ecp5_emu::{board, live};
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/debounce");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn(
            live::Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let design = loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if sh.published > 0 && sh.ready.as_ref().is_some_and(|d| d.sources.get().is_some()) {
                break sh.ready.clone().unwrap();
            }
            assert!(Instant::now() < deadline, "{}", sh.stage);
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        };
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, "Input port".into(), None);
        app.shared = shared.clone();
        app.commands = Some(commands.clone());
        let mut waves = Waves::default();
        assert_eq!(waves.add_name(&design, "btn[1]"), 1);
        commands.send(waves.command().unwrap()).unwrap();
        for pressed in [false, true, false] {
            let (publication, cycle) = {
                let sh = shared.lock().unwrap();
                (sh.published, sh.cycles)
            };
            // The board's FIRE1 lever sends this same command and raw level.
            commands.send(Command::Button(1, pressed)).unwrap();
            commands.send(Command::Step(2)).unwrap();
            loop {
                let sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{:?}", sh.error);
                if sh.published != publication && !sh.running && sh.cycles == cycle + 2 {
                    let expected = u8::from(pressed != board::BUTTONS[1].idle);
                    assert_eq!(sh.lit[waves.nets[0].wire as usize], expected);
                    break;
                }
                assert!(Instant::now() < deadline);
                drop(sh);
                std::thread::sleep(Duration::from_millis(10));
            }
            let value = app.source_value("top.v", "btn[1]");
            let expected = u8::from(pressed != board::BUTTONS[1].idle);
            assert_eq!(
                value.displayed(),
                format!("0x{expected} ({expected}) (bits 1..1 found)")
            );
        }
    }

    /// Native reader/reference data, but no window, frame renderer or GPU.
    #[test]
    #[ignore = "headless reference design, native reader and device database"]
    fn source_values_reference_states_seek_replacement_and_coherent_publications() {
        use super::App;
        use ecp5_emu::{layout::Geometry, live};
        use std::{
            sync::{
                atomic::{AtomicU64, Ordering},
                Mutex, OnceLock,
            },
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/blinky_cycle");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let repaints = Arc::new(AtomicU64::new(0));
        let count = repaints.clone();
        let runner = live::spawn(
            live::Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            move || {
                count.fetch_add(1, Ordering::Relaxed);
            },
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let (design, lit) = loop {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{:?}", s.error);
            if s.published > 0 {
                break (s.ready.clone().unwrap(), s.lit.clone());
            }
            assert!(Instant::now() < deadline, "{}", s.stage);
            drop(s);
            std::thread::sleep(Duration::from_millis(10));
        };
        let ecp5_emu::sources::NameLookup::Resolved(rows) =
            design.sources.get().unwrap().name_only("top.v", "count[0]")
        else {
            panic!("counter")
        };
        let net = rows[0].bits[0].targets[0];
        let wire = design.overlay.pieces[design.overlay.pieces_of(net)[0] as usize].value;
        let start_cycle = shared.lock().unwrap().cycles;
        runner
            .send(Command::Watch {
                id: 71,
                wires: vec![wire],
                keep: 64,
            })
            .unwrap();
        runner.send(Command::Step(4)).unwrap();
        loop {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{:?}", s.error);
            if s.cycles == start_cycle + 4 && !s.running {
                let bit = s
                    .waves
                    .iter()
                    .find_map(|run| {
                        (run.id == 71 && run.first <= s.edges)
                            .then(|| run.rows.get((s.edges - run.first) as usize).copied())
                            .flatten()
                    })
                    .expect("sample at the published edge");
                assert!(bit <= 1);
                assert_eq!(bit, s.lit[wire as usize], "same edge and input state");
                break;
            }
            assert!(Instant::now() < deadline);
            drop(s);
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(runner);
        assert!(
            repaints.load(Ordering::Relaxed) > 0,
            "publications repaint with the chip hidden too"
        );
        let make_design = |indexed| {
            let source = OnceLock::new();
            if indexed {
                source
                    .set(design.sources.get().unwrap().clone())
                    .ok()
                    .unwrap();
            }
            Arc::new(live::Static {
                rows: design.rows,
                cols: design.cols,
                cells: Vec::new(),
                geometry: Geometry {
                    rows: 0,
                    cols: 0,
                    segments: Vec::new(),
                    dots: Vec::new(),
                    sites: Vec::new(),
                    logic: Vec::new(),
                    values: Vec::new(),
                    longest: 0.0,
                },
                fabric: design.fabric.clone(),
                overlay: serde_json::from_value(serde_json::json!({
                    "values": [], "nets": [], "pieces": design.overlay.pieces,
                    "dots": [], "sites": [], "boxes": [], "pieces_at": [],
                    "boxes_at": [], "dots_at": [], "sites_at": [], "longest": 0.0,
                    "routes": 0, "drawn": 0, "not_drawn": {}, "hidden": 0,
                    "hidden_by": {}, "undriven": 0, "carried": 0, "unfed": 0,
                    "loose": [], "nets_at": {net.to_string(): [0, design.overlay.pieces_of(net).len()]},
                    "by_net": design.overlay.pieces_of(net), "drivers": {}, "fed_at": {}, "tied": {}, "index": {}
                })).unwrap(),
                net_names: design.net_names.clone(),
                sources: source,
                source_seconds: OnceLock::new(),
                timing: OnceLock::new(),
                model: design.model.clone(),
                device: design.device.clone(),
                settings: Default::default(),
                transfers: Default::default(),
                leds: [false; 8],
                buttons: [false; 7],
                switches: [false; 4],
                header: Vec::new(),
                uart_tx: false,
                uart_rx: false,
                summary: String::new(),
                routes: 0,
            })
        };
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, "Values".into(), None);
        let (commands, received) = std::sync::mpsc::channel();
        app.commands = Some(commands);
        let initial = make_design(false);
        *app.shared.lock().unwrap() = Shared {
            ready: Some(initial.clone()),
            lit,
            ..Default::default()
        };
        assert_eq!(
            app.source_value("top.v", "count[0]").detail,
            ["Source index is being prepared"]
        );
        initial
            .sources
            .set(design.sources.get().unwrap().clone())
            .ok()
            .unwrap();
        assert_eq!(
            app.source_value("top.v", "count[0]").detail,
            ["Values are not available yet"]
        );
        {
            let mut s = app.shared.lock().unwrap();
            s.published = 10;
            s.cycles = 7;
            s.edges = 14;
            s.earliest = 2;
        }
        let unavailable = app.source_value("top.v", "count[1]");
        assert!(unavailable.displayed().contains("0x?"));
        assert!(unavailable
            .detail
            .iter()
            .any(|s| s.contains("missing: count[1]")));
        let first = app.source_value("top.v", "count[0]");
        assert_eq!(first.cycle, Some(7));
        assert!(!first.running);
        assert_eq!(first.targets.len(), 1);
        let net = first.targets[0];
        let wire =
            initial.overlay.pieces[initial.overlay.pieces_of(net)[0] as usize].value as usize;
        let before = app.shared.lock().unwrap().lit[wire];
        {
            let mut s = app.shared.lock().unwrap();
            s.lit[wire] = 1 - before;
            s.running = true;
        }
        let cached = app.source_value("top.v", "count[0]");
        assert_eq!(
            cached.rows, first.rows,
            "no reformat/read between publications"
        );
        assert!(cached.running);
        app.shared.lock().unwrap().published += 1;
        assert_ne!(app.source_value("top.v", "count[0]").rows, first.rows);
        assert!(
            received.try_recv().is_err(),
            "passive values send no commands"
        );
        assert!(app.probed.is_empty() && app.waves.nets.is_empty());
        assert!(
            app.taken.published.is_none()
                && app.taken.lit.is_empty()
                && app.taken.values.is_empty(),
            "no full vector copy for project-only values"
        );
        app.send(Command::Back(3));
        assert!(matches!(received.recv().unwrap(), Command::Back(3)));
        assert_eq!(app.source_value("top.v", "count[0]").detail, ["seeking…"]);
        {
            let mut s = app.shared.lock().unwrap();
            s.published += 1;
            s.cycles = 6;
        }
        assert_eq!(
            app.source_value("top.v", "count[0]").detail,
            ["seeking…"],
            "unrelated publication while a command is delayed"
        );
        {
            let mut s = app.shared.lock().unwrap();
            s.published += 1;
            s.cycles = 4;
        }
        assert_eq!(app.source_value("top.v", "count[0]").cycle, Some(4));
        app.send(Command::GoTo(99));
        app.send(Command::GoTo(0));
        {
            let mut s = app.shared.lock().unwrap();
            s.published += 1;
            s.cycles = 99;
        }
        assert_eq!(
            app.source_value("top.v", "count[0]").detail,
            ["seeking…"],
            "superseding target wins"
        );
        {
            let mut s = app.shared.lock().unwrap();
            s.published += 1;
            s.cycles = 2;
        }
        assert_eq!(app.source_value("top.v", "count[0]").cycle, Some(2));
        app.send(Command::Back(99));
        assert_eq!(
            app.source_value("top.v", "count[0]").detail,
            ["seeking…"],
            "a seek to the current earliest cycle still needs a later publication"
        );
        app.shared.lock().unwrap().published += 1;
        assert_eq!(app.source_value("top.v", "count[0]").cycle, Some(2));
        let old = app.source_value("top.v", "count[0]");
        {
            let mut s = app.shared.lock().unwrap();
            s.ready = Some(make_design(true));
            s.lit[wire] ^= 1;
        }
        let replaced = app.source_value("top.v", "count[0]");
        assert_eq!(replaced.published, old.published);
        assert_ne!(
            replaced.rows, old.rows,
            "same IDs and publish counter in a new design invalidate caches"
        );
        let cloned = app.shared.clone();
        let publisher = std::thread::spawn(move || {
            for cycle in 100..500 {
                let mut s = cloned.lock().unwrap();
                s.cycles = cycle;
                s.edges = cycle * 2 + cycle % 2;
                s.published += 1;
                s.lit[wire] = (cycle % 2) as u8;
                drop(s);
                std::thread::yield_now();
            }
        });
        for _ in 0..500 {
            let view = app.source_value("top.v", "count[0]");
            if let Some(cycle) = view.cycle.filter(|cycle| *cycle >= 100) {
                assert_eq!(view.edges, cycle * 2 + cycle % 2);
                assert!(view.displayed().starts_with(&format!("0x{}", cycle % 2)));
            }
        }
        publisher.join().unwrap();
        assert_eq!(app.source_value("top.v", "removed").cycle, None);
        // A rebuild report alone has not replaced this running Static/index.
        assert!(Arc::ptr_eq(
            app.source_values.design.as_ref().unwrap(),
            app.shared.lock().unwrap().ready.as_ref().unwrap()
        ));
        let _ = received.try_iter().collect::<Vec<_>>();
        app.probed = vec![u32::MAX];
        app.watch_source_targets(&replaced.targets);
        let Command::Watch { wires, .. } = received.recv().unwrap() else {
            panic!("explicit watch")
        };
        assert_eq!(wires, [wire as u32]);
        assert_eq!(
            app.probed,
            [u32::MAX],
            "watching a hover never re-probes or changes map selection"
        );
    }

    #[test]
    fn critical_panel_toggle_rows_and_source_menu_in_both_themes() {
        use super::{critical_toggle, timing_layout, timing_panel, Critical};
        let timing = timing_fixture();
        for dark in [false, true] {
            for size in [egui::vec2(1000.0, 700.0), egui::vec2(1600.0, 1000.0)] {
                let ctx = egui::Context::default();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut critical = Critical::default();
                let mut map_rect = egui::Rect::NOTHING;
                let mut frame = |events| {
                    let mut action = super::TimingAction::default();
                    let out = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::top("controls")
                                .show(ui, |ui| critical_toggle(ui, &mut critical));
                            egui::CentralPanel::default().show(ui, |ui| {
                                let (map, shelf) =
                                    timing_layout(ui.available_rect_before_wrap(), critical.on);
                                map_rect = map;
                                if let Some(shelf) = shelf {
                                    ui.scope_builder(
                                        egui::UiBuilder::new().max_rect(shelf),
                                        |ui| {
                                            action = timing_panel(
                                                ui,
                                                &mut critical,
                                                Some(&timing),
                                                false,
                                            );
                                        },
                                    );
                                }
                            });
                        },
                    );
                    (out, action, map_rect)
                };
                let find = |out: &egui::FullOutput, text: &str| {
                    out.shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(t) if t.galley.text().contains(text) => {
                                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("missing {text:?}"))
                };
                let click = |pos, button, pressed| {
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
                let (off, _, before) = frame(vec![]);
                let toggle = find(&off, "Critical path").center();
                frame(click(toggle, egui::PointerButton::Primary, true));
                frame(click(toggle, egui::PointerButton::Primary, false));
                let (on, _, after) = frame(vec![]);
                assert_eq!(before.min, after.min);
                assert_eq!(before.height(), after.height());
                assert_eq!(before.width() - after.width(), 348.0);
                for text in [
                    "nextpnr timing report",
                    "nextpnr's estimate, not measured by the emulator",
                    "Fmax 354.99 MHz",
                    "constraint 25.00 MHz; slack +37.18 ns",
                    "total 2.82 ns",
                    "clk-to-q",
                    "top.v:7",
                ] {
                    find(&on, text);
                }
                let delay = find(&on, "clk-to-q");
                let source_line = find(&on, "top.v:7");
                assert!(
                    source_line.min.y > delay.max.y,
                    "source is a separately drawn line"
                );
                find(&on, "led_TRELLIS_FF_Q_7.Q");
                let row = find(&on, "clk-to-q").center();
                frame(click(row, egui::PointerButton::Primary, true));
                let (_, action, _) = frame(click(row, egui::PointerButton::Primary, false));
                assert_eq!(action.focus.unwrap().hop.kind.word(), "clk-to-q");
                frame(click(row, egui::PointerButton::Secondary, true));
                frame(click(row, egui::PointerButton::Secondary, false));
                let (menu, _, _) = frame(vec![]);
                let source = find(&menu, "Show in source").center();
                frame(click(source, egui::PointerButton::Primary, true));
                let (_, action, _) = frame(click(source, egui::PointerButton::Primary, false));
                assert_eq!(action.source.unwrap().2.label(), "top.v:7");
                frame(click(toggle, egui::PointerButton::Primary, true));
                frame(click(toggle, egui::PointerButton::Primary, false));
                let (closed, _, last) = frame(vec![]);
                assert_eq!(before, last);
                assert!(!closed.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "nextpnr timing report")));
                frame(click(toggle, egui::PointerButton::Primary, true));
                frame(click(toggle, egui::PointerButton::Primary, false));
                let (on, _, _) = frame(vec![]);
                let close = find(&on, "×").center();
                frame(click(close, egui::PointerButton::Primary, true));
                frame(click(close, egui::PointerButton::Primary, false));
                let (closed, _, last) = frame(vec![]);
                assert_eq!(before, last);
                assert!(!closed.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "nextpnr timing report")));
                frame(click(toggle, egui::PointerButton::Primary, true));
                frame(click(toggle, egui::PointerButton::Primary, false));
                let escape = |pos| {
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                };
                frame(escape(after.center()));
                let (on, _, map) = frame(vec![]);
                assert_eq!(map, after, "Escape over the map keeps the report open");
                find(&on, "nextpnr timing report");
                frame(escape(egui::pos2(close.x, before.bottom() - 5.0)));
                let (closed, _, last) = frame(vec![]);
                assert_eq!(before, last);
                assert!(!closed.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "nextpnr timing report")));
                assert!(!critical.on);
            }
        }
    }

    /// The user's complete loaded layout, with CPU shapes and no native window or GPU.
    #[test]
    #[ignore = "headless reference design, native reader and device database"]
    fn critical_shelf_loaded_layout_paints_full_map_and_records_resizes() {
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/blinky_cycle");
        let runner = ecp5_emu::live::spawn(
            ecp5_emu::live::Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            || {},
        );
        let started = Instant::now();
        loop {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{:?}", s.error);
            if s.ready.as_ref().is_some_and(|s| s.timing.get().is_some()) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(180), "{}", s.stage);
            drop(s);
            std::thread::sleep(Duration::from_millis(10));
        }
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Layout".into(), None);
        app.shared = shared;
        app.commands = Some(runner);
        // CPU frames have no board renderer; supply its readback rectangle for layout notes.
        app.board.rect = Some(egui::Rect::from_min_size(
            egui::pos2(8.0, 90.0),
            egui::vec2(584.0, 450.0),
        ));
        let temp = crate::test_temp::unique_dir("ecp5-map-layout-");
        let record = temp.join("window.tsv");
        app.trace(&record).unwrap();
        let mut frame = eframe::Frame::_new_kittest();
        let mut draw = |app: &mut super::App, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1500.0, 950.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| eframe::App::ui(app, ui, &mut frame),
            )
        };
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        let full = app.fabric.map_rect;
        assert!((full.min.x - 608.0).abs() < 1.0, "{full:?}");
        assert!((full.max.x - 1492.0).abs() < 1.0, "{full:?}");
        app.fabric.aim(0.4, 0.6, 19.0);
        let off = draw(&mut app, vec![]);
        assert!((app.fabric.zoom_level() - 19.0).abs() < 0.01);
        assert_eq!(app.fabric.map_rect, full);
        assert!(
            off.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(12, 13, 16)
                    && r.rect == full && shape.clip_rect.contains_rect(full)
            )),
            "fabric background paints across the full central width"
        );
        assert!(!off.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(t) if t.galley.text() == "nextpnr timing report"
        )));
        app.critical.command("on");
        let on = draw(&mut app, vec![]);
        let (narrow, shelf) = super::timing_layout(full, true);
        let shelf = shelf.unwrap();
        assert_eq!(app.fabric.map_rect, narrow);
        assert_eq!(narrow.max.x + 8.0, shelf.min.x);
        assert!(on.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(t) if t.galley.text() == "nextpnr timing report" && shelf.contains(t.pos)
        )));
        let button = |out: &egui::FullOutput, text: &str| {
            out.shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == text => {
                        Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {text}"))
        };
        let centre = app.fabric.camera_centre();
        for close in ["×", "Critical path", "Escape"] {
            let on = draw(&mut app, vec![]);
            if close == "Escape" {
                draw(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(shelf.center()),
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            } else {
                let pos = button(&on, close);
                for pressed in [true, false] {
                    draw(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
            }
            assert!(!app.critical.on);
            assert!(app
                .critical
                .path(app.ready().unwrap().timing.get())
                .is_none());
            draw(&mut app, vec![]);
            assert_eq!(app.fabric.map_rect, full);
            let (after, cell) = app.fabric.camera_centre();
            assert!((after - centre.0).length() < 0.01, "{close}");
            assert!((cell - centre.1).abs() < 0.01, "{close}");
            app.critical_path("on");
            draw(&mut app, vec![]);
        }
        app.critical.command("off");
        draw(&mut app, vec![]);
        assert_eq!(app.fabric.map_rect, full);
        app.fabric.fit();
        draw(&mut app, vec![]);
        assert!((app.fabric.zoom_level() - 1.0).abs() < 1e-5);
        app.critical.command("on");
        draw(&mut app, vec![]);
        assert!((app.fabric.zoom_level() - 1.0).abs() < 1e-5);
        app.critical.command("off");
        draw(&mut app, vec![]);
        assert!((app.fabric.zoom_level() - 1.0).abs() < 1e-5);
        drop(app);
        let text = std::fs::read_to_string(record).unwrap();
        let layouts: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("# layout\t"))
            .map(|line| line.split("die map ").nth(1).unwrap())
            .collect();
        let label = |rect: egui::Rect| {
            format!(
                "x {:.0}..{:.0} y {:.0}..{:.0}",
                rect.min.x, rect.max.x, rect.min.y, rect.max.y
            )
        };
        let full = label(full);
        let narrow = label(narrow);
        assert_eq!(
            layouts,
            [
                &full, &narrow, &full, &narrow, &full, &narrow, &full, &narrow, &full, &narrow,
                &full,
            ]
        );
        let closing_notes: Vec<_> = text
            .lines()
            .filter(|line| {
                line.starts_with("# map-layout report=off") && line.contains("resized=1")
            })
            .take(3)
            .collect();
        assert_eq!(closing_notes.len(), 3);
        assert_eq!(closing_notes[0], closing_notes[1]);
        assert_eq!(closing_notes[1], closing_notes[2]);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn critical_clock_selector_waiting_and_parser_errors() {
        use super::{timing_panel, Critical};
        let mut timing = timing_fixture();
        let paths = timing.result.as_mut().unwrap();
        let mut second = (*paths[0]).clone();
        second.label = "second_clock".into();
        second.path.domain = ecp5_emu::timing_path::Domain::Clock {
            name: "second_clock".into(),
            edges: "posedge -> posedge".into(),
        };
        paths.push(Arc::new(second));
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut state = Critical {
                on: true,
                ..Default::default()
            };
            let mut frame = |events, timing| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(380.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        timing_panel(ui, &mut state, timing, false);
                    },
                )
            };
            let find = |out: &egui::FullOutput, text: &str| {
                out.shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == text => {
                            Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {text}"))
            };
            let click = |pos, pressed| {
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
            find(&frame(vec![], None), "Waiting for nextpnr.log…");
            let first = frame(vec![], Some(&timing));
            let combo = find(&first, "clk_25mhz");
            frame(click(combo, true), Some(&timing));
            frame(click(combo, false), Some(&timing));
            let popup = frame(vec![], Some(&timing));
            let second = find(&popup, "second_clock");
            frame(click(second, true), Some(&timing));
            frame(click(second, false), Some(&timing));
            let selected = frame(vec![], Some(&timing));
            find(&selected, "second_clock");
            let error = ecp5_emu::timing_path::Published {
                result: Err("critical path: nextpnr 0.12 not supported".into()),
                seconds: 0.0,
                bytes: 0,
            };
            find(
                &frame(vec![], Some(&error)),
                "critical path: nextpnr 0.12 not supported",
            );
            assert_eq!(state.path(Some(&timing)).unwrap().label, "second_clock");
        }
    }

    #[test]
    fn map_layout_notes_only_geometry_and_carries_the_previous_frames_camera() {
        let full = egui::Rect::from_min_max(egui::pos2(608.0, 57.0), egui::pos2(1492.0, 794.0));
        let mut previous = None;
        let initial = (full, None, full, egui::vec2(100.0, 200.0), 40.0);
        let note = super::map_layout_note(&mut previous, initial).unwrap();
        assert!(note.contains("resized=0"));
        let mut camera = initial.3;
        let mut cell = initial.4;
        for i in 0..400 {
            camera = egui::vec2(i as f32, 300.0);
            cell = 40.0 + i as f32;
            assert!(
                super::map_layout_note(&mut previous, (full, None, full, camera, cell)).is_none()
            );
        }
        let (narrow, shelf) = super::timing_layout(full, true);
        let note =
            super::map_layout_note(&mut previous, (full, shelf, narrow, camera, cell)).unwrap();
        assert!(note.contains("resized=1"));
        assert!(
            note.contains("before_centre=399.000000,300.000000 before_cell=439.000000"),
            "{note}"
        );
        assert!(
            note.contains("centre=399.000000,300.000000 cell=439.000000"),
            "{note}"
        );
        assert!(
            super::map_layout_note(&mut previous, (full, shelf, narrow, camera, cell)).is_none()
        );
    }

    #[test]
    fn critical_commands_select_and_wait_for_publication() {
        let timing = timing_fixture();
        let mut state = super::Critical::default();
        state.command("on");
        assert!(state.path(None).is_none());
        assert_eq!(state.path(Some(&timing)).unwrap().label, "clk_25mhz");
        state.command("off");
        assert!(state.path(Some(&timing)).is_none());
        state.command("clk_25mhz");
        assert!(state.path(Some(&timing)).is_some());
        state.command("unknown");
        assert!(state.path(Some(&timing)).is_none());
    }

    #[test]
    fn source_menu_context_follows_the_window_before_index_publication() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx);
        let mut app = super::App::new(&cc, "Sources".into(), None);
        for plain in [true, false] {
            app.start_screen = plain;
            app.poll_sources();
            assert_eq!(app.fabric.project_sources, !plain);
            assert_eq!(app.waves.project_sources, !plain);
        }
    }

    #[test]
    fn waves_scripts_record_unavailable_and_unknown_rows_in_both_windows() {
        use crate::waves::{RowError, Watched};
        use ecp5_emu::sources::Index;
        for plain in [false, true] {
            for state in 0..3 {
                let ctx = egui::Context::default();
                let cc = eframe::CreationContext::_new_kittest(ctx.clone());
                let mut app = super::App::new(&cc, "Waves outcomes".into(), None);
                app.start_screen = plain;
                app.ready_at = Some(std::time::Instant::now());
                app.waves.nets = vec![Watched {
                    name: "count[3]".into(),
                    net: 3,
                    wire: 3,
                }];
                app.waves.sources = match state {
                    0 => None,
                    _ => {
                        let mut index = Index::default();
                        index.locations = usize::from(state == 2);
                        Some(std::sync::Arc::new(index))
                    }
                };
                let reason = match state {
                    0 => "design has no source index yet",
                    1 if plain => {
                        "no source files found for this design (looked beside its top.config)"
                    }
                    1 => "no project source matched the build's src attributes",
                    _ => "no source line for this net (synthesis left it only a library location)",
                };
                let path = std::env::temp_dir().join(format!(
                    "ecp5-waves-outcomes-{}-{plain}-{state}.tsv",
                    std::process::id()
                ));
                app.trace(&path).unwrap();
                app.script = [
                    "waves-source count[3]",
                    "waves-source reset_count",
                    "waves-chip reset_count",
                ]
                .into_iter()
                .map(|command| (0.0, command.into()))
                .collect();
                let out = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
                assert!(app.script.is_empty());
                assert!(app.take_source_jump().is_none());
                assert!(!app.take_chip_jump());
                assert!(app.probed.is_empty());
                assert!(!out
                    .platform_output
                    .commands
                    .iter()
                    .any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
                // The shared reporter also preserves an unavailable chip outcome.
                app.waves_outcome(
                    false,
                    "count[3]",
                    Err(RowError::Unavailable("no wire on the map for this row")),
                );
                let expected = [
                    format!("# waves source count[3] -> unavailable: {reason}"),
                    "# waves source reset_count -> not a Waves row".into(),
                    "# waves chip reset_count -> not a Waves row".into(),
                    "# waves chip count[3] -> unavailable: no wire on the map for this row".into(),
                ];
                let notes = app.take_probe_notes();
                if plain {
                    assert!(notes.is_empty());
                } else {
                    assert_eq!(notes, expected);
                }
                drop(app.fabric.trace.get_mut().take());
                let record = std::fs::read_to_string(&path).unwrap();
                std::fs::remove_file(&path).unwrap();
                assert_eq!(
                    record
                        .lines()
                        .filter(|line| line.starts_with("# waves "))
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }

    #[test]
    fn waves_commands_parse_row_names_and_dispatch_the_menu_actions() {
        use crate::waves::{RowAction, Watched};
        use ecp5_emu::sources::{Index, NetSources};
        assert_eq!(
            super::waves_command("waves-chip count[3]"),
            Some((false, "count[3]"))
        );
        assert_eq!(
            super::waves_command("waves-source count"),
            Some((true, "count"))
        );
        assert_eq!(
            super::waves_command(" waves-chip  net 42 "),
            Some((false, "net 42"))
        );
        for command in ["waves-chip", "waves-source ", "waves count", "watch count"] {
            assert_eq!(super::waves_command(command), None);
        }
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Waves".into(), None);
        app.ready_at = Some(std::time::Instant::now());
        app.waves.nets = (0..4)
            .map(|net| Watched {
                name: format!("count[{net}]"),
                net,
                wire: net,
            })
            .collect();
        let span = super::Span {
            file: "rtl/counter.v".into(),
            line: 19,
            column: Some(3),
            end_line: 19,
            end_column: Some(8),
        };
        let declaration = super::Span {
            line: 4,
            end_line: 4,
            ..span.clone()
        };
        let mut index = Index::default();
        index.locations = 8;
        for net in 0..4 {
            index.nets.insert(
                net,
                NetSources {
                    driven: vec![span.clone()],
                    declared: vec![declaration.clone()],
                    ..Default::default()
                },
            );
        }
        app.waves.sources = Some(std::sync::Arc::new(index));
        let script_path =
            std::env::temp_dir().join(format!("ecp5-waves-script-{}.txt", std::process::id()));
        std::fs::write(
            &script_path,
            "0 waves-chip count # all bits\n0 waves-source count[3]\n",
        )
        .unwrap();
        app.script(&script_path).unwrap();
        std::fs::remove_file(&script_path).unwrap();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
        assert_eq!(app.probed, [0, 1, 2, 3]);
        assert!(app.fabric.has_probe());
        assert_eq!(app.fabric.net, Some(0));
        assert!(app.map == super::Map::Fabric);
        assert!(app.take_chip_jump());
        assert!(!app.take_chip_jump());
        assert_eq!(app.take_source_jump(), Some(span.clone()));
        assert!(app.take_source_jump().is_none());
        assert_eq!(
            app.take_probe_notes(),
            [
                "# waves chip count -> 4 nets",
                "# waves source count[3] -> rtl/counter.v:19,rtl/counter.v:4"
            ]
        );
        app.waves_action(app.waves.chip_action("count[3]").unwrap());
        assert_eq!(app.probed, [3]);
        assert_eq!(app.fabric.net, Some(3));
        // In a plain window both a menu choice and the script copy the span.
        app.start_screen = true;
        let action = app
            .waves
            .source_action("count", Some(declaration.clone()))
            .unwrap();
        assert!(matches!(&action, RowAction::Source { chosen, .. } if chosen == &declaration));
        let mut action = Some(action);
        let out = ctx.run_ui(egui::RawInput::default(), |_| {
            if let Some(action) = action.take() {
                app.waves_action(action);
            }
        });
        assert!(app.take_source_jump().is_none());
        assert!(out
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s == "rtl/counter.v:4")));
    }

    #[test]
    fn show_source_commands_take_the_same_map_shares_as_point() {
        assert_eq!(
            super::source_point("show-source 0.25 0.75"),
            Some((0.25, 0.75))
        );
        for command in [
            "show-source",
            "show-source 0.5",
            "show-source -0.1 0.5",
            "show-source 0.5 NaN",
            "show-source 0.5 1.1",
            "show-source 0 0 extra",
        ] {
            assert!(super::source_point(command).is_none(), "{command}");
        }
    }

    #[test]
    fn map_notes_name_aliases_and_both_windows_use_the_chosen_source() {
        use ecp5_emu::sources::{Collected, Files};
        let top = serde_json::json!({"netnames":{
            "cpu.reg_pc[8]":{"bits":[8],"attributes":{"src":"picorv32.v:159.13-159.19"}},
            "z_alias":{"bits":[8],"attributes":{"src":"picorv32.v:159.20-159.28"}}
        },"cells":{
            "ff":{"attributes":{"src":"picorv32.v:1374.2-1946.5"},"port_directions":{"Q":"output"},"connections":{"Q":[8]}}
        }});
        let files = Files::project("project".into(), ["picorv32.v".into()]);
        let index = Collected::from_top(&top, &files).build(|_| vec![21826]);
        let choices = index.net_source_choices(21826);
        assert_eq!(
            super::map_probe_note(&index.net_label(21826), &choices),
            "probe map net 21826 (cpu.reg_pc[8] +1) -> picorv32.v:1374,picorv32.v:159"
        );
        for chosen in &choices {
            let ctx = egui::Context::default();
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(super::source_jump_or_copy(ui.ctx(), false, chosen.span.clone()).is_none());
            });
            assert!(out.platform_output.commands.iter().any(|command| matches!(command,egui::OutputCommand::CopyText(text) if *text == chosen.span.label())));
            assert_eq!(
                super::source_jump_or_copy(&ctx, true, chosen.span.clone()),
                Some(chosen.span.clone())
            );
        }
        assert_eq!(
            choices[0].span.line, 1374,
            "the script's default is assigned"
        );
    }

    #[test]
    fn script_switch_numbers_and_positions() {
        for n in 1..=4 {
            for (word, expected) in [("on", true), ("off", false)] {
                let text = n.to_string();
                let command = switch_command(Some(&text), Some(word), &[true; 4]).unwrap();
                assert!(matches!(command, Command::Switch(i, on) if i == n - 1 && on == expected));
                let mut used = [true; 4];
                used[n - 1] = false;
                assert!(switch_command(Some(&text), Some(word), &used).is_err());
            }
        }
        for n in [
            None,
            Some("0"),
            Some("5"),
            Some("-1"),
            Some("one"),
            Some("18446744073709551616"),
        ] {
            assert!(switch_command(n, Some("on"), &[true; 4]).is_err(), "{n:?}");
        }
        for word in [None, Some("up"), Some("ON"), Some("true"), Some("")] {
            assert!(
                switch_command(Some("1"), word, &[true; 4]).is_err(),
                "{word:?}"
            );
        }
    }

    #[test]
    fn panel_scripts_parse_every_name_and_reject_invalid_commands() {
        for panel in Panel::ALL {
            for (action, open) in [("open", true), ("close", false)] {
                assert_eq!(
                    panel_command(&format!("panel {} {action}", panel.name())).unwrap(),
                    (panel, open)
                );
            }
        }
        for command in [
            "panel",
            "panel oled",
            "panel unknown open",
            "panel OLED open",
            "panel edid open",
            "panel adc toggle",
            "panel sd Open",
            "panel rtc open extra",
        ] {
            assert!(panel_command(command).is_err(), "{command}");
        }
    }

    #[test]
    fn panel_scripts_reopen_seen_panels_record_commands_and_leave_unused_ones_alone() {
        use ecp5_emu::peripherals::{adc::Adc, View};
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Panel scripts".into(), None);
        let view = View {
            adc: Some(Adc::new().status()),
            ..View::default()
        };
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.monitor.show(ui.ctx(), &view);
        });
        let root = crate::test_temp::unique_dir("panel-scripts-");
        let record = root.join("record.tsv");
        app.trace(&record).unwrap();
        for (command, expected) in [
            ("panel adc close", false),
            ("panel adc open", true),
            ("panel oled open", true),
            ("panel adc toggle", true),
        ] {
            app.script = vec![(0.0, command.into())];
            app.ready_at = Some(std::time::Instant::now());
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
            assert!(app.script.is_empty());
            assert_eq!(
                app.monitor.panels().iter().find(|p| p.0 == Panel::Adc),
                Some(&(Panel::Adc, true, expected))
            );
            assert_eq!(
                app.monitor.panels().iter().find(|p| p.0 == Panel::Oled),
                Some(&(Panel::Oled, false, false))
            );
        }
        app.fabric.trace.get_mut().take();
        let text = std::fs::read_to_string(&record).unwrap();
        for command in [
            "panel adc close",
            "panel adc open",
            "panel oled open",
            "panel adc toggle",
        ] {
            assert!(
                text.lines()
                    .any(|line| line.starts_with("# script\t") && line.ends_with(command)),
                "{text}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn peripherals_menu_disables_unseen_panels_and_toggles_a_seen_one() {
        use ecp5_emu::peripherals::{adc::Adc, View};
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Panel menu".into(), None);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.monitor.show(
                ui.ctx(),
                &View {
                    adc: Some(Adc::new().status()),
                    ..View::default()
                },
            );
        });
        let frame = |app: &mut super::App, events| {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.horizontal(|ui| {
                        app.design_menu(ui, None);
                        app.peripherals_menu(ui);
                    });
                },
            )
        };
        let position = |out: &egui::FullOutput, text: &str| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            })
        };
        let click = |pos, pressed| {
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
        frame(&mut app, vec![]);
        for (panel, expected) in [
            (Panel::Oled, false),
            (Panel::Adc, false),
            (Panel::Adc, true),
        ] {
            let mut out = frame(&mut app, vec![]);
            if position(&out, panel.label()).is_none() {
                let menu = position(&out, "Peripherals").unwrap();
                frame(&mut app, click(menu, true));
                frame(&mut app, click(menu, false));
                out = frame(&mut app, vec![]);
            }
            for entry in Panel::ALL {
                assert!(position(&out, entry.label()).is_some(), "{entry:?}");
            }
            let at = position(&out, panel.label()).unwrap();
            frame(&mut app, click(at, true));
            frame(&mut app, click(at, false));
            assert_eq!(
                app.monitor.panels().iter().find(|p| p.0 == panel),
                Some(&(panel, panel == Panel::Adc, expected))
            );
        }
    }

    #[test]
    fn switch_clicks_toggle_the_reported_position() {
        for bits in 0..16 {
            let on = std::array::from_fn(|i| bits & (1 << i) != 0);
            for i in 0..4 {
                assert!(matches!(switch_click(i, &on), Command::Switch(index, value)
                    if index == i && value != on[i]));
            }
        }
    }

    #[test]
    fn switches_draw_requested_positions_over_reported_positions() {
        for bits in 0..16 {
            let reported = std::array::from_fn(|i| bits & (1 << i) != 0);
            for requests in 0..81 {
                let switches = SwitchRequests {
                    requested: std::array::from_fn(|i| {
                        match requests / 3_usize.pow(i as u32) % 3 {
                            0 => None,
                            1 => Some(false),
                            _ => Some(true),
                        }
                    }),
                };
                let drawn = switches.drawn(reported);
                for i in 0..4 {
                    assert_eq!(drawn[i], switches.requested[i].unwrap_or(reported[i]));
                }
            }
        }
    }

    #[test]
    fn switch_requests_end_only_on_matching_reports_for_each_lever() {
        let mut switches = SwitchRequests {
            requested: [Some(true), Some(false), Some(true), None],
        };
        switches.report([false, true, true, false]);
        assert_eq!(switches.requested, [Some(true), Some(false), None, None]);
        switches.report([true, true, false, true]);
        assert_eq!(switches.requested, [None, Some(false), None, None]);
        // An acknowledged request no longer hides later reported changes.
        assert_eq!(
            switches.drawn([false, true, false, true]),
            [false, false, false, true]
        );
        switches.report([false; 4]);
        assert_eq!(switches.requested, [None; 4]);
    }

    #[test]
    fn back_and_all_seeks_clear_switch_requests() {
        for command in [
            Command::Back(0),
            Command::Back(12),
            Command::GoTo(0),
            Command::GoTo(42),
            Command::GoTo(u64::MAX),
        ] {
            let mut switches = SwitchRequests {
                requested: [Some(true); 4],
            };
            switches.command(&command);
            assert_eq!(switches.requested, [None; 4]);
            assert_eq!(switches.drawn([false; 4]), [false; 4]);
        }
    }

    #[test]
    fn two_switch_toggles_while_paused_request_on_then_off() {
        let reported = [false; 4];
        for i in 0..4 {
            let mut switches = SwitchRequests::default();
            for expected in [true, false] {
                // The paused runner keeps reporting off between clicks.
                switches.report(reported);
                let command = switch_click(i, &switches.drawn(reported));
                assert!(matches!(command, Command::Switch(index, on)
                    if index == i && on == expected));
                switches.command(&command);
                assert_eq!(switches.requested[i], Some(expected));
                assert_eq!(switches.drawn(reported)[i], expected);
            }
            // Running or stepping must preserve a request awaiting its report.
            for command in [Command::Run, Command::Pause, Command::Step(1)] {
                switches.command(&command);
                assert_eq!(switches.requested[i], Some(false));
            }
        }
    }

    #[test]
    fn switch_requests_are_recorded_with_board_numbers_and_positions() {
        let path = std::env::temp_dir().join(format!(
            "ecp5_switch_request_trace_{}.tsv",
            std::process::id()
        ));
        let mut trace = crate::trace::Trace::new(&path).unwrap();
        trace
            .note(&switch_request_note(1.23456, 42, 3, true))
            .unwrap();
        trace.note(&switch_request_note(2.0, 42, 0, false)).unwrap();
        drop(trace);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let notes: Vec<_> = text.lines().filter(|line| line.starts_with('#')).collect();
        assert_eq!(
            notes,
            [
                "# switch requested\t1.235\t42\t4\ton",
                "# switch requested\t2.000\t42\t1\toff",
            ]
        );
    }

    #[test]
    fn switch_rows_keep_fixed_space_when_loaded_or_changed() {
        for preference in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, preference);
            let mut initial = None;
            for used in 0..16 {
                for bits in 0..16 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 950.0),
                        )),
                        ..Default::default()
                    };
                    let _ = ctx.run_ui(input, |ui| {
                        let rows = ui.scope(|ui| {
                            assert!(switch_rows(
                                ui,
                                &std::array::from_fn(|i| used & (1 << i) != 0),
                                &std::array::from_fn(|i| bits & (1 << i) != 0),
                            )
                            .is_empty());
                        });
                        let after = ui.label("UART").rect;
                        let geometry = (rows.response.rect, after);
                        assert_eq!(*initial.get_or_insert(geometry), geometry);
                        assert_eq!(rows.response.rect.width(), 170.0);
                    });
                }
            }
        }
    }

    #[test]
    fn a_script_selects_all_glow_modes() {
        use crate::{fabric_view::FabricView, gpu_map::GlowChoice, trace};
        let path =
            std::env::temp_dir().join(format!("ecp5_glow_script_{}.txt", std::process::id()));
        std::fs::write(
            &path,
            "0 glow quads\n1 glow top\n2 glow mask\n3 glow auto # back to default\n",
        )
        .unwrap();
        let script = trace::script(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let mut view = FabricView::new(None, None, (0.5, 0.5));
        assert_eq!(view.glow, GlowChoice::Auto);
        assert_eq!(script.len(), 4);
        for ((_, command), expected) in script.into_iter().zip([
            GlowChoice::Quads,
            GlowChoice::Top,
            GlowChoice::Mask,
            GlowChoice::Auto,
        ]) {
            let (verb, mode) = command.split_once(' ').unwrap();
            assert_eq!(verb, "glow");
            assert!(view.set_glow(mode));
            assert_eq!(view.glow, expected);
        }
        assert!(!view.set_glow("unknown"));
        assert_eq!(view.glow, GlowChoice::Auto);
    }

    #[test]
    fn build_demo_scripts_dispatch_the_build_control_and_record_skipped_names() {
        use crate::start;
        use std::{
            fs,
            time::{Duration, Instant},
        };

        let root =
            std::env::temp_dir().join(format!("ecp5-build-demo-script-{}", std::process::id()));
        let examples = root.join("examples/ulx3s");
        for name in ["built", "unbuilt"] {
            fs::create_dir_all(examples.join(name)).unwrap();
            fs::write(examples.join(name).join("top.v"), "module top(); endmodule").unwrap();
        }
        // Parsing the project fails before any external FPGA tool can start.
        fs::write(examples.join("unbuilt/ecp5.toml"), "invalid = [").unwrap();
        fs::write(examples.join("README.md"), "").unwrap();
        fs::create_dir_all(root.join("target/reference/built")).unwrap();
        fs::write(root.join("target/reference/built/top.config"), "").unwrap();
        let path = root.join("script.txt");
        fs::write(&path, "0 build-demo unknown\n1 build-demo built\n5 build-demo unbuilt # build as from the row\n0.1 values on\n").unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Open a design".into(), None);
        app.reader_check = || start::reader::fixed(start::reader::Status::Ready);
        app.start_screen = true;
        app.demos = start::demos(&root).unwrap();
        app.opened_at = Instant::now() - Duration::from_secs(10);
        app.fabric.values = false;
        let record = root.join("window.tsv");
        app.trace(&record).unwrap();
        app.script(&path).unwrap();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
        assert!(
            matches!(&app.scripted_action, Some(start::Action::Build(name)) if name == "unbuilt")
        );
        assert!(
            !app.fabric.values,
            "later commands belong to the built design"
        );
        assert_eq!(app.script, [(0.1, "values on".into())]);
        let action = app.scripted_action.take().unwrap();
        app.action(&ctx, action);
        assert!(
            !app.demos[0].build.running(),
            "an already-built demo is untouched"
        );
        assert!(
            app.demos[1].build.running(),
            "the script uses the row's build worker"
        );
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
        assert!(
            !app.fabric.values,
            "scripts wait while the build is running"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.demos[1].build.running() {
            app.poll_open(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(app.demos[1].build.failed());
        assert!(app.commands.is_none());
        assert!(app.scripted_action.is_none());
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.scripted(ui));
        assert!(
            app.fabric.values,
            "a failed build leaves the script on the start screen"
        );
        drop(app);
        let text = fs::read_to_string(record).unwrap();
        assert!(text
            .lines()
            .any(|s| s == "# script build-demo unknown: unknown demo"));
        assert!(text
            .lines()
            .any(|s| s == "# script build-demo built: already built"));
        let build: Vec<_> = text
            .lines()
            .filter(|s| s.starts_with("# start build "))
            .collect();
        assert_eq!(build.len(), 1, "{text}");
        assert!(
            build[0]
                .strip_prefix("# start build unbuilt failed ")
                .unwrap()
                .parse::<f64>()
                .unwrap()
                >= 0.0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn board_view_height_matches_aspect_and_caps() {
        assert!((board_view_height(600.0, 880.0) - 600.0 / BOARD_VIEW_ASPECT).abs() < 1e-4);
        assert!((board_view_height(480.0, 880.0) - 300.0).abs() < 1e-4);
        assert!((board_view_height(800.0, 900.0) - 500.0).abs() < 1e-4);
        assert_eq!(board_view_height(600.0, 240.0), 240.0);
        assert_eq!(board_view_height(600.0, 375.0), 375.0);
        assert_eq!(board_view_height(0.0, 500.0), 0.0);
        assert_eq!(board_view_height(600.0, 0.0), 0.0);
        assert_eq!(board_view_height(-10.0, 500.0), 0.0);
        assert_eq!(board_view_height(600.0, -5.0), 0.0);
        assert_eq!(board_view_height(f32::NAN, 500.0), 0.0);
        assert_eq!(board_view_height(600.0, f32::INFINITY), 0.0);
    }

    #[test]
    fn board_uart_split_drag_limits_reset_share_and_following_text() {
        use super::BoardUartSplit;
        let path = crate::board_model::default_path();
        let model = path
            .exists()
            .then(|| crate::board_model::load(&path).unwrap());
        for fallback in [0.0, 128.0] {
            let ctx = egui::Context::default();
            let mut split = BoardUartSplit::default();
            let size = egui::vec2(1000.0, 800.0);
            let mut text = "received line\n".repeat(100);
            let frame = |split: &mut BoardUartSplit, time, events, size, text: &str| {
                let mut panes = None;
                let mut scroll = None;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::left("board")
                            .default_size(600.0)
                            .show(ui, |ui| {
                                ui.heading("ULX3S");
                                let available = ui.available_rect_before_wrap();
                                let (board, uart) = split.show(ui, fallback);
                                ui.scope_builder(egui::UiBuilder::new().max_rect(board), |ui| {
                                    ui.allocate_rect(board, egui::Sense::hover());
                                });
                                ui.scope_builder(egui::UiBuilder::new().max_rect(uart), |ui| {
                                    ui.label("UART received text");
                                    scroll = Some(super::uart_text(ui, text));
                                });
                                assert!(ui.min_rect().bottom() <= available.bottom() + 0.01);
                                panes = Some((board, uart));
                            });
                    },
                );
                (out, panes.unwrap(), scroll.unwrap())
            };
            let handle = |panes: (egui::Rect, egui::Rect)| {
                egui::pos2(
                    panes.0.center().x,
                    panes.0.bottom() + BoardUartSplit::HANDLE / 2.0,
                )
            };
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(&mut split, 0.0, vec![], size, &text);
            let (_, initial, _) = frame(&mut split, 0.1, vec![], size, &text);
            assert!(
                (initial.0.height() - initial.0.width() / BOARD_VIEW_ASPECT - fallback).abs()
                    < 0.01
            );
            let pos = handle(initial);
            let (hover, _, _) = frame(
                &mut split,
                0.2,
                vec![egui::Event::PointerMoved(pos)],
                size,
                &text,
            );
            assert_eq!(
                hover.platform_output.cursor_icon,
                egui::CursorIcon::ResizeVertical
            );
            frame(&mut split, 1.0, vec![button(pos, true)], size, &text);
            let end = pos + egui::vec2(0.0, -70.0);
            frame(
                &mut split,
                1.1,
                vec![egui::Event::PointerMoved(end)],
                size,
                &text,
            );
            let (_, resized, _) = frame(&mut split, 1.2, vec![button(end, false)], size, &text);
            assert!((initial.0.height() - resized.0.height() - 70.0).abs() < 0.01);
            assert!((resized.1.height() - initial.1.height() - 70.0).abs() < 0.01);
            let share = split.share.unwrap();
            let (_, larger, _) = frame(
                &mut split,
                1.3,
                vec![],
                size + egui::vec2(0.0, 200.0),
                &text,
            );
            assert!(
                (larger.0.height() / (larger.0.height() + larger.1.height()) - share).abs() < 0.001
            );
            let (_, mut panes, _) = frame(&mut split, 1.4, vec![], size, &text);
            let mut board_rects = vec![initial.0, resized.0, larger.0];
            for (time, y) in [(2.0, -2000.0), (3.0, 2000.0)] {
                let pos = handle(panes);
                frame(
                    &mut split,
                    time,
                    vec![egui::Event::PointerMoved(pos), button(pos, true)],
                    size,
                    &text,
                );
                let end = egui::pos2(pos.x, y);
                frame(
                    &mut split,
                    time + 0.1,
                    vec![egui::Event::PointerMoved(end)],
                    size,
                    &text,
                );
                (_, panes, _) = frame(
                    &mut split,
                    time + 0.2,
                    vec![button(end, false)],
                    size,
                    &text,
                );
                assert!(panes.0.height() >= BoardUartSplit::BOARD_MIN + fallback - 0.01);
                assert!(panes.1.height() >= BoardUartSplit::UART_MIN - 0.01);
                if y < 0.0 {
                    assert!((panes.0.height() - BoardUartSplit::BOARD_MIN - fallback).abs() < 0.01);
                } else {
                    assert!((panes.1.height() - BoardUartSplit::UART_MIN).abs() < 0.01);
                }
                board_rects.push(panes.0);
            }
            let pos = handle(panes);
            for time in [4.0, 4.1] {
                frame(
                    &mut split,
                    time,
                    vec![
                        egui::Event::PointerMoved(pos),
                        button(pos, true),
                        button(pos, false),
                    ],
                    size,
                    &text,
                );
            }
            let (_, reset, before) = frame(&mut split, 4.2, vec![], size, &text);
            assert!(split.share.is_none());
            assert_eq!(reset, initial);
            text.push_str(&"new received line\n".repeat(50));
            frame(&mut split, 4.3, vec![], size, &text);
            let (_, _, after) = frame(&mut split, 4.4, vec![], size, &text);
            assert!(after.state.offset.y > before.state.offset.y);
            assert!(
                (after.state.offset.y - (after.content_size.y - after.inner_rect.height())).abs()
                    < 1.0
            );
            assert!(reset.1.contains_rect(after.inner_rect));

            // The renderer uses this camera fit: actual LED lenses stay inside
            // every viewport reached by the drag, including both limits.
            if let Some(model) = &model {
                for mut rect in board_rects {
                    rect.max.y -= fallback;
                    let aspect = rect.width() / rect.height();
                    for camera in [
                        crate::board_model::Camera::PHOTO,
                        crate::board_model::Camera::TOP,
                    ] {
                        let matrix = camera.view_proj(camera.fit(&model.outline, aspect), aspect);
                        for led in model.leds {
                            let p = matrix.point(led.expect("board LED lens"));
                            let at = egui::pos2(
                                rect.center().x + p[0] * rect.width() / 2.0,
                                rect.center().y - p[1] * rect.height() / 2.0,
                            );
                            assert!(rect.contains(at), "LED {at:?} outside {rect:?}");
                        }
                    }
                }
            }
        }
    }

    /// The window copies the wires' values only when the run has published
    /// since: a frame between two publishes takes nothing.
    #[test]
    fn values_are_taken_once_per_publish() {
        let mut s = Shared {
            lit: vec![0, 1, 2],
            values: vec![1, 1],
            ..Default::default()
        };
        let mut taken = Taken::default();
        // The first look takes whatever the run has, published or not.
        assert!(taken.take(&s));
        assert_eq!((*taken.lit).clone(), [0, 1, 2]);
        assert_eq!((*taken.values).clone(), [1, 1]);
        // Nothing published since: nothing taken, the copy as it was.
        s.lit[0] = 1;
        assert!(!taken.take(&s));
        assert_eq!((*taken.lit).clone(), [0, 1, 2]);
        // A frame holds the copy while the run publishes again: it keeps
        // what it had, the next frame has the new values.
        let frame = Arc::clone(&taken.lit);
        s.lit = vec![1, 1, 0, 2];
        s.published += 1;
        assert!(taken.take(&s));
        assert_eq!((*frame).clone(), [0, 1, 2]);
        assert_eq!((*taken.lit).clone(), [1, 1, 0, 2]);
        assert!(!taken.take(&s));
        // Another design (a new `Shared`, counting from 0 again): taken
        // whatever its count, once.
        let other = Shared {
            lit: vec![2; 5],
            published: 1,
            ..Default::default()
        };
        taken = Taken::default();
        assert!(taken.take(&other));
        assert_eq!((*taken.lit).clone(), [2; 5]);
        assert!(!taken.take(&other));
    }

    /// Numbers typed for a step or a rate, and how a rate is shown to edit.
    #[test]
    fn typed_numbers() {
        for (text, value) in [
            ("1500", 1500.0),
            ("1 500", 1500.0),
            ("1_500", 1500.0),
            ("2.5k", 2500.0),
            ("2,5k", 2500.0),
            ("3M", 3e6),
            ("1G", 1e9),
            ("0.5", 0.5),
        ] {
            assert_eq!(amount(text), Some(value), "{text}");
        }
        for text in ["", "k", "-3", "0", "abc", "1.2.3"] {
            assert_eq!(amount(text), None, "{text}");
        }
        assert_eq!(cycle_number("0"), Some(0));
        assert_eq!(cycle_number("2.5M"), Some(2_500_000));
        assert_eq!(cycle_number("12 345"), Some(12_345));
        assert_eq!(cycle_number("x"), None);
        assert_eq!(short(25e6), "25M");
        assert_eq!(short(2500.0), "2.5k");
        assert_eq!(short(3.0), "3");
        assert!((amount(&short(1234.5)).unwrap() - 1234.5).abs() < 1e-6);
    }

    #[test]
    fn step_led_hint_uses_publications_and_clears_on_change_run_and_timeout() {
        use super::StepLedHint;
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let used = [true, false, false, false, false, false, false, false];
        let mut hint = StepLedHint::default();
        let mut s = Shared {
            cycles: 100,
            published: 1,
            leds: [Some(false); 8],
            ..Shared::default()
        };
        assert!(hint.observe(&s, used, now).is_none());
        hint.dispatch(&s, used, &Command::Step(1000), now);
        s.cycles = 1100;
        s.leds[1] = Some(true); // An unused LED is irrelevant.
        assert!(
            hint.observe(&s, used, now).is_none(),
            "no later publication yet"
        );
        assert!(hint.text.is_none());
        s.published += 1;
        assert_eq!(
            hint.observe(&s, used, now).as_deref(),
            Some("hint step-leds 1000 1000")
        );
        assert_eq!(hint.text.as_deref(), Some("No LED changed in this 1000-cycle step. The LEDs have not changed since cycle 100; try a larger step (1M)."));
        assert!(hint
            .observe(&s, used, now + Duration::from_secs(7))
            .is_none());
        assert!(hint.text.is_some());
        hint.observe(&s, used, now + Duration::from_secs(8));
        assert!(hint.text.is_none());
        hint.dispatch(&s, used, &Command::Back(500), now);
        s.published += 1;
        s.cycles = 600;
        assert_eq!(
            hint.observe(&s, used, now).as_deref(),
            Some("hint step-leds 500 500")
        );
        hint.dispatch(&s, used, &Command::GoTo(900), now);
        s.published += 1;
        s.cycles = 900;
        assert_eq!(
            hint.observe(&s, used, now).as_deref(),
            Some("hint step-leds 300 800")
        );
        hint.dispatch(&s, used, &Command::Step(1), now);
        s.published += 1;
        s.cycles = 901;
        s.leds[0] = Some(true);
        assert!(hint.observe(&s, used, now).is_none());
        assert!(hint.text.is_none());
        hint.dispatch(&s, used, &Command::Step(1000), now);
        s.published += 1;
        s.cycles = 1901;
        assert_eq!(
            hint.observe(&s, used, now).as_deref(),
            Some("hint step-leds 1000 1000")
        );
        assert!(hint
            .text
            .as_deref()
            .unwrap()
            .contains("They last changed 1000 cycles ago"));
        hint.dispatch(&s, used, &Command::Run, now);
        assert!(hint.text.is_none() && hint.pending.is_none());
        // A design with no used LEDs needs no LED advice.
        hint.observe(&s, [false; 8], now);
        hint.dispatch(&s, [false; 8], &Command::Step(1), now);
        s.published += 1;
        s.cycles += 1;
        assert!(hint.observe(&s, [false; 8], now).is_none());
        assert!(hint.text.is_none());
    }

    #[test]
    fn step_led_hint_is_shown_in_status_and_recorded_once_per_step() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Step LED hint".into(), None);
        let (commands, received) = std::sync::mpsc::channel();
        app.commands = Some(commands);
        // Fake Shared has no loaded fabric; supply its one used LED directly.
        app.step_leds.used[0] = true;
        let temp = crate::test_temp::unique_dir("ecp5-step-led-hint-");
        let record = temp.join("window.tsv");
        app.trace(&record).unwrap();
        let publish = |app: &super::App, cycle, running, value| {
            let mut s = app.shared.lock().unwrap();
            s.cycles = cycle;
            s.edges = cycle * 2;
            s.running = running;
            s.leds[0] = Some(value);
            s.published += 1;
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut draw = |app: &mut super::App| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1500.0, 950.0),
                    )),
                    ..Default::default()
                },
                |ui| eframe::App::ui(app, ui, &mut frame),
            )
        };
        publish(&app, 1000, false, false);
        draw(&mut app);
        app.send(Command::Step(1000));
        assert!(matches!(received.try_recv(), Ok(Command::Step(1000))));
        publish(&app, 2000, false, false);
        let out = draw(&mut app);
        let text = app.step_leds.text.clone().unwrap();
        assert!(text.contains("1000-cycle step") && text.contains("since cycle 1000"));
        assert!(
            out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(t) if t.galley.text() == text
            )),
            "the real App status area draws the complete hint"
        );
        draw(&mut app);
        publish(&app, 2000, false, false);
        draw(&mut app); // Neither redraws nor idle publications repeat the note.
        app.send(Command::Step(1000));
        publish(&app, 3000, false, true);
        draw(&mut app);
        assert!(app.step_leds.text.is_none());
        publish(&app, 1_600_000, true, false);
        draw(&mut app); // Running publications update the last-change cycle.
        publish(&app, 1_600_100, false, false);
        draw(&mut app);
        app.send(Command::Step(1000));
        publish(&app, 1_601_100, false, false);
        draw(&mut app);
        assert_eq!(app.step_leds.text.as_deref(), Some("No LED changed in this 1000-cycle step. They last changed 1100 cycles ago; try a larger step (1M)."));
        app.send(Command::Run);
        assert!(
            app.step_leds.text.is_none(),
            "Run clears before its next publication"
        );
        draw(&mut app);
        assert!(app.step_leds.text.is_none());
        drop(app);
        let text = std::fs::read_to_string(record).unwrap();
        let notes: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("# hint step-leds "))
            .collect();
        assert_eq!(
            notes,
            ["# hint step-leds 1000 1000", "# hint step-leds 1000 1100"]
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn debounce_hint_counts_cycles_waits_wall_time_and_records_once_per_hold() {
        use std::time::{Duration, Instant};
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = super::App::new(&cc, "Debounce hint".into(), None);
        let (commands, received) = std::sync::mpsc::channel();
        app.commands = Some(commands);
        let temp = crate::test_temp::unique_dir("ecp5-debounce-hint-");
        let record = temp.join("window.tsv");
        app.trace(&record).unwrap();
        let now = Instant::now();
        app.held[1] = true;
        app.update_debounce_hint(now, 100, Some(4.0));
        app.update_debounce_hint(now + Duration::from_millis(1499), 105, Some(4.0));
        assert!(app.debounce.text.is_none());
        app.update_debounce_hint(now + Duration::from_millis(1500), 106, Some(4.0));
        let text = app.debounce.text.as_deref().unwrap();
        assert_eq!(text, "FIRE1 held for 6 cycles. Designs often debounce a button for about 65 536 cycles (2.6 ms at 25 MHz); at 4 cycles/s that takes 4 h 33 min. Raise the rate (real time) while holding.");
        // The complete explanation wraps in the window's status area.
        for width in [600.0, 1500.0] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 950.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::bottom("hint")
                        .exact_size(28.0 + app.debounce.height(ui))
                        .show(ui, |ui| {
                            let label = ui.add(egui::Label::new(text).wrap());
                            assert!(
                                ui.clip_rect().contains_rect(label.rect),
                                "{:?}: {:?}",
                                ui.clip_rect(),
                                label.rect
                            );
                            status_bar(
                                ui,
                                "Running",
                                None,
                                106,
                                0.000004,
                                Some(4.0),
                                None,
                                None,
                                30.0,
                            );
                        });
                },
            );
            assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(t) if t.galley.text() == text
            )));
        }
        for (seconds, cycles) in [(2, 108), (3, 112), (4, 116)] {
            app.update_debounce_hint(now + Duration::from_secs(seconds), cycles, Some(4.0));
            assert!(app
                .debounce
                .text
                .as_deref()
                .unwrap()
                .starts_with(&format!("FIRE1 held for {} cycles.", cycles - 100)));
        }
        app.update_debounce_hint(now + Duration::from_secs(5), 120, Some(6000.0));
        assert!(app
            .debounce
            .text
            .as_deref()
            .unwrap()
            .contains("at 6000 cycles/s that takes 11 s."));
        app.update_debounce_hint(now + Duration::from_secs(6), 200, Some(live::REAL_TIME));
        assert!(app.debounce.text.is_none());
        app.update_debounce_hint(now + Duration::from_secs(7), 204, Some(4.0));
        assert!(app.debounce.text.is_some());
        app.held[1] = false;
        app.update_debounce_hint(now + Duration::from_secs(8), 208, Some(4.0));
        assert!(app.debounce.text.is_none());
        app.held[1] = true;
        app.update_debounce_hint(now + Duration::from_secs(9), 212, Some(4.0));
        assert!(app.debounce.text.is_none());
        app.update_debounce_hint(now + Duration::from_millis(10_500), 218, Some(4.0));
        assert!(app.debounce.text.is_some());
        assert!(
            received.try_iter().next().is_none(),
            "a hint never drives the design"
        );
        drop(app);
        let text = std::fs::read_to_string(record).unwrap();
        let notes: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("# hint debounce "))
            .collect();
        assert_eq!(
            notes,
            ["# hint debounce FIRE1 6 4", "# hint debounce FIRE1 6 4"]
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn debounce_hint_ignores_real_time_max_and_ten_second_waits() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        for rate in [
            Some(live::REAL_TIME),
            None,
            Some(65_536.0 / 10.0),
            Some(0.0),
            Some(f64::NAN),
        ] {
            let mut hint = super::DebounceHint::default();
            let held = [true; 7];
            assert!(hint.update(held, now, 10, rate).is_none());
            assert!(hint
                .update(held, now + Duration::from_secs(2), 50, rate)
                .is_none());
            assert!(hint.text.is_none(), "rate {rate:?}");
        }
    }

    #[test]
    fn status_line_keeps_fixed_width_and_fits_1500_points() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, egui::ThemePreference::Dark);
        let fast = FastChoice {
            threads: 10,
            folding: 4,
        };
        let mut widths = Vec::new();
        for (stage, error, cycles, mhz, rate, fast_opt, watch, fps) in [
            (
                "Paused",
                None,
                0_u64,
                0.0_f64,
                Some(25_000_000.0),
                None,
                None,
                0.0_f64,
            ),
            (
                "Running",
                None,
                123_456_789,
                12.34,
                None,
                Some(&fast),
                Some("Building a program that keeps 3 watched nets…"),
                60.0,
            ),
            (
                "Error",
                Some("bad"),
                42,
                0.0005,
                Some(2.0),
                Some(&fast),
                Some("Running a program that keeps 3 watched nets the default folds: less folding, its speed may differ"),
                30.0,
            ),
        ] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 950.0),
                )),
                ..Default::default()
            };
            let mut max_x = 0.0_f32;
            let _ = ctx.run_ui(input, |ui| {
                egui::Panel::bottom("status")
                    .exact_size(28.0)
                    .show(ui, |ui| {
                        let resp = ui.horizontal(|ui| {
                            status_bar(ui, stage, error, cycles, mhz, rate, fast_opt, watch, fps);
                        });
                        max_x = resp.response.rect.max.x;
                    });
            });
            assert!(max_x <= 1500.0, "status bar overflowed 1500pt: {max_x}");
            widths.push(max_x);
        }
        assert!(
            widths.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01),
            "widths changed: {widths:?}"
        );
    }
}
