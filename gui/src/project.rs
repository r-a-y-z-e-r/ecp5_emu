//! A Quartus-like project window (`ecp5_gui --project DIR`): sources,
//! incremental builds (yosys, nextpnr-ecp5, ecpbram, ecppack), messages,
//! log, utilisation/timing report, and live emulation in
//! the embedded `App` (`src/app.rs`).
pub(crate) mod editor;
mod new_file;
mod output;
pub(crate) mod script;
pub(crate) mod tour;
mod tree;

use crate::{app::App, start::display_paths::DisplayPaths};
use ecp5_emu::{
    flow::{self, BuildOptions, Event, Message, Outcome, Project, Severity, Step, Tools},
    live,
    sources::Span,
};
use editor::{Editor, Probe};
use eframe::egui::{self, Color32};
use output::{LogBuffer, LogSource, OutputState, OutputTab};
use script::{ProjectTrace, ScriptCommand, ScriptRunner, ViewTab, WaitCondition};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use tree::{StepProgress, TreeAction};

/// Background build; only one runs at a time.
enum ActiveTask {
    Build {
        events: mpsc::Receiver<Event>,
        done: mpsc::Receiver<Outcome>,
        cancel: Arc<AtomicBool>,
        started: Instant,
        step_started: Option<(Step, Instant)>,
        last_line: Option<String>,
    },
}

/// One snapshot supplies both the visible build controls and Design's submenus.
#[derive(Clone, Copy)]
enum ToolbarAction {
    Tab(ViewTab),
    Build(bool),
    Stop,
    Emulate,
}

pub(crate) struct Toolbar {
    tab: ViewTab,
    idle: bool,
    building: bool,
    emulate_blocker: Option<String>,
    action: Option<ToolbarAction>,
    #[cfg(test)]
    controls: Vec<(String, egui::Rect, bool)>,
}
impl Toolbar {
    pub(crate) fn tabs(&mut self, ui: &mut egui::Ui) {
        for (label, tab) in [("Project", ViewTab::Project), ("Chip", ViewTab::Chip)] {
            let response = ui.selectable_label(self.tab == tab, label);
            #[cfg(test)]
            self.controls
                .push((label.into(), response.rect, response.enabled()));
            if response.clicked() {
                self.action = Some(ToolbarAction::Tab(tab));
            }
        }
    }
    fn build_action(&mut self, ui: &mut egui::Ui, clean: bool) {
        let label = if clean { "Rebuild" } else { "Build" };
        let response = ui
            .add_enabled(self.idle, egui::Button::new(label))
            .on_hover_text(if clean {
                "Force clean rebuild of all steps"
            } else {
                "Incremental build (Cmd/Ctrl+B)"
            })
            .on_disabled_hover_text(
                "Open a valid project and wait for the running build to finish.",
            );
        #[cfg(test)]
        self.controls
            .push((label.into(), response.rect, response.enabled()));
        if response.clicked() {
            self.action = Some(ToolbarAction::Build(clean));
            if egui::containers::menu::is_in_menu(ui) {
                ui.close();
            }
        }
    }
    fn emulate(&mut self, ui: &mut egui::Ui) {
        let response = ui.add_enabled(self.emulate_blocker.is_none(), egui::Button::new("Emulate"))
            .on_hover_text("Open this project's build in the emulator; during a build, queue its successful result.");
        let response = if let Some(reason) = &self.emulate_blocker {
            response.on_disabled_hover_text(reason)
        } else {
            response
        };
        #[cfg(test)]
        self.controls
            .push(("Emulate".into(), response.rect, response.enabled()));
        if response.clicked() {
            self.action = Some(ToolbarAction::Emulate);
            if egui::containers::menu::is_in_menu(ui) {
                ui.close();
            }
        }
    }
    pub(crate) fn build_menu(&mut self, ui: &mut egui::Ui) {
        self.build_action(ui, false);
        self.build_action(ui, true);
        if ui
            .add_enabled(self.building, egui::Button::new("Stop build"))
            .on_disabled_hover_text("No build is running.")
            .clicked()
        {
            self.action = Some(ToolbarAction::Stop);
            if egui::containers::menu::is_in_menu(ui) {
                ui.close();
            }
        }
        self.emulate(ui);
    }
}

const MEMORY_IMAGE_CONFIRMATION: &str = "Swapped into the placed design by ecpbram after place and route, so new firmware needs no new place and route. A $readmemh file that is not listed here is built in by Yosys; changing it reruns synthesis.";

struct MemoryImageChange {
    relative: PathBuf,
    use_as_image: bool,
    error: Option<String>,
}

pub struct ProjectApp {
    app: App,
    project: Result<Project, String>,
    out_override: Option<PathBuf>,
    path_input: String,
    paths: DisplayPaths,
    examples: Vec<(String, PathBuf)>,
    tab: ViewTab,
    /// Cycles per second to give each design opened (`--rate`).
    rate: Option<Option<f64>>,
    active: Option<ActiveTask>,
    last: Option<Outcome>,
    needs_rebuild: bool,
    new_file_dialog: Option<new_file::Dialog>,
    memory_image_change: Option<MemoryImageChange>,
    inactive_constraints: Vec<PathBuf>,
    /// Open the design in the Chip tab as soon as the running build succeeds.
    emulate_after: bool,
    status: String,
    steps: Vec<(Step, StepProgress)>,
    messages: Vec<Message>,
    log: LogBuffer,
    output: OutputState,
    editor: Editor,
    trace: Option<ProjectTrace>,
    script: Option<ScriptRunner>,
    typing: Option<Typing>,
    editor_phase: u64,
    editor_key: String,
    editor_samples: Vec<(f64, f64, bool, usize)>,
    /// A design opened in the emulator and not yet ready (it takes seconds:
    /// the device model, the configuration, the die's drawing, fast mode).
    chip_loading: Option<Instant>,
    pending_probe: Option<Probe>,
    probe_summary: Option<(Probe, String, String)>,
    emulated_root: Option<PathBuf>,
    tour: Option<tour::Session>,
}

impl ProjectApp {
    /// Opens the project in `dir`, its outputs in `out` when given; with
    /// `run`, builds it and opens the design at once.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        dir: PathBuf,
        out: Option<PathBuf>,
        run: bool,
        tour: bool,
    ) -> anyhow::Result<Self> {
        Self::new_with_paths(cc, dir, out, run, tour, DisplayPaths::new(None))
    }

    fn new_with_paths(
        cc: &eframe::CreationContext<'_>,
        dir: PathBuf,
        out: Option<PathBuf>,
        run: bool,
        tour: bool,
        paths: DisplayPaths,
    ) -> anyhow::Result<Self> {
        let trace = std::env::var_os("ECP5_PROJECT_TRACE")
            .map(|p| ProjectTrace::new(Path::new(&p)))
            .transpose()?;
        let script = std::env::var_os("ECP5_PROJECT_SCRIPT")
            .filter(|_| !tour)
            .map(|p| script::parse_script_file(Path::new(&p)))
            .transpose()?
            .map(ScriptRunner::from_steps);
        let app = App::new(cc, "Project".into(), None);
        Ok(Self::from_app(
            app,
            dir,
            out,
            run,
            paths,
            trace,
            script,
            &cc.egui_ctx,
        ))
    }

    /// Move the existing window app, including its trace and pending GUI commands.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_app(
        mut app: App,
        dir: PathBuf,
        out: Option<PathBuf>,
        run: bool,
        mut paths: DisplayPaths,
        mut trace: Option<ProjectTrace>,
        script: Option<ScriptRunner>,
        ctx: &egui::Context,
    ) -> Self {
        let dir = paths.expand_path(&dir);
        let project = open_project_with_out(&dir, out.as_deref());
        paths.set_root(project.as_ref().ok().map(|p| p.root.as_path()));
        let path_input = paths.path(&dir);
        let steps = project
            .as_ref()
            .map_or_else(|_| Vec::new(), tree::default_steps);
        if let Some(t) = &mut trace {
            let event = match &project {
                Ok(p) => format!("project {} .", crate::new_project::atom(&p.name)),
                Err(error) => format!("project error {error:#}"),
            };
            t.record(&paths.text(&event));
        }
        app.set_display_paths(paths.clone());
        if let Ok(project) = &project {
            app.set_project_title(&project.name);
        }
        let rate = app.preferred_rate;
        app.set_toolbar_hosted();
        let mut window = Self {
            app,
            project: project.map_err(|e| format!("{e:#}")),
            out_override: out,
            path_input,
            paths,
            examples: tree::examples_list(),
            tab: ViewTab::Project,
            rate,
            active: None,
            last: None,
            needs_rebuild: false,
            new_file_dialog: None,
            memory_image_change: None,
            inactive_constraints: Vec::new(),
            emulate_after: false,
            status: "Idle".into(),
            steps,
            messages: Vec::new(),
            log: LogBuffer::default(),
            output: OutputState::default(),
            editor: Editor::default(),
            trace,
            script,
            typing: None,
            editor_phase: 0,
            editor_key: String::new(),
            editor_samples: Vec::new(),
            chip_loading: None,
            pending_probe: None,
            probe_summary: None,
            emulated_root: None,
            tour: None,
        };
        if run && window.project.is_ok() {
            window.build(false, true, ctx);
        }
        window
    }

    pub(crate) fn creation_allowed(&self) -> bool {
        !self.is_busy() && self.chip_loading.is_none()
    }

    pub(crate) fn prepare_creation(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.creation_allowed(),
            "Wait for the current build or emulation to finish"
        );
        self.editor.save_all_dirty()?;
        Ok(())
    }

    pub(crate) fn creation_note(&mut self, note: &str) {
        self.record(note);
        self.app.record_note(note);
    }

    pub(crate) fn complete_creation_command(&mut self) {
        self.app.creation_complete();
        if let Some(runner) = &mut self.script {
            if runner.waiting() == Some(WaitCondition::Creation) {
                runner.complete_wait(Instant::now());
            }
        }
    }

    pub(crate) fn adopt_created(&mut self, dir: &Path, ctx: &egui::Context) {
        self.app.reset_for_project(ctx);
        self.out_override = None;
        self.rate = self.app.preferred_rate;
        self.open_project(dir, ctx);
        let source = self
            .project
            .as_ref()
            .ok()
            .and_then(|p| {
                p.hdl_roots()
                    .iter()
                    .find(|path| path.file_stem().is_some_and(|stem| stem == p.top.as_str()))
                    .cloned()
            })
            .unwrap_or_else(|| PathBuf::from("top.v"));
        self.open_relative(&source, None);
        self.set_tab(ViewTab::Project);
        self.app.remember_project(dir);
    }

    #[cfg(test)]
    pub(crate) fn creation_build_started(&self) -> bool {
        assert!(self.project.is_ok(), "{:?}", self.project);
        self.is_building()
    }

    #[cfg(test)]
    pub(crate) fn creation_editor_path(&self) -> Option<&Path> {
        self.editor.active_path()
    }

    /// The window inside, for the options main.rs applies to it.
    pub fn app(&mut self) -> &mut App {
        &mut self.app
    }

    /// Cycles per second for every design this window opens.
    pub fn rate(&mut self, rate: Option<f64>) {
        self.rate = Some(rate);
        self.app.rate(rate);
    }

    pub fn start_tour(&mut self, project: tour::TourProject) -> anyhow::Result<()> {
        let directory = crate::start::settings_path()
            .and_then(|p| p.parent().map(|p| p.join("tour")))
            .ok_or_else(|| anyhow::anyhow!("could not find the configuration folder"))?;
        std::fs::create_dir_all(&directory)?;
        if self.trace.is_none() {
            self.trace = Some(ProjectTrace::new(&directory.join("last-project.tsv"))?);
        }
        if !self.app.has_trace() {
            self.app.trace(&directory.join("last-window.tsv"))?;
        }
        self.script = None;
        self.app.clear_script();
        self.tour = Some(tour::Session::new(project));
        self.record("project blinky_cycle .");
        self.flush_tour_notes();
        Ok(())
    }

    fn flush_tour_notes(&mut self) {
        let Some(tour) = &mut self.tour else {
            return;
        };
        let notes: Vec<_> = tour
            .machine
            .notes()
            .iter()
            .map(|n| tour.relative(n))
            .collect();
        for note in notes {
            self.record(&format!("# {note}"));
            self.app.record_note(&note);
        }
    }

    fn stop_tour(&mut self) {
        if let Some(tour) = &mut self.tour {
            tour.machine.stop(tour.seconds());
            tour.target = None;
        }
        self.flush_tour_notes();
    }

    fn fail_tour(&mut self, reason: &str) {
        if let Some(tour) = &mut self.tour {
            tour.machine.fail(tour.seconds(), &tour.relative(reason));
        }
        self.status = reason.to_owned();
        self.flush_tour_notes();
    }

    fn poll_tour(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.tour else {
            return;
        };
        if !session.machine.active() {
            return;
        }
        let (running_at_four, probed, waves) = self.app.tour_evidence();
        let (map_frame, lit) = self.app.drawing_progress();
        let seen = tour::Evidence {
            missing_tool: self.last.as_ref().is_some_and(|o| {
                !o.cancelled && o.messages.iter().any(flow::tools::is_missing_tool)
            }),
            build: (!self.is_building())
                .then(|| {
                    self.last.as_ref().map(|o| {
                        if o.cancelled {
                            Err("Build stopped".to_owned())
                        } else if o.ok {
                            Ok(())
                        } else {
                            let errors = o
                                .messages
                                .iter()
                                .filter(|m| m.severity == Severity::Error)
                                .map(|m| m.text.as_str())
                                .collect::<Vec<_>>()
                                .join("; ");
                            Err(if errors.is_empty() {
                                self.status.clone()
                            } else {
                                errors
                            })
                        }
                    })
                })
                .flatten(),
            chip: self.tab == ViewTab::Chip && self.app.state().1 && map_frame > 0,
            chip_ready: self.app.state().1,
            running_at_four: self.tab == ViewTab::Chip && running_at_four,
            changes_drawn: lit > session.lit_before,
            source_probed: probed && self.pending_probe.is_none() && session.editor_shown,
            source_jumped: session.jumped && session.editor_shown && self.tab == ViewTab::Project,
            waves: self.tab == ViewTab::Chip && waves,
        };
        if session.machine.step != tour::Step::Build {
            if let Some(error) = self.app.state().2 {
                self.fail_tour(&error);
                return;
            }
        }
        let session = self.tour.as_mut().unwrap();
        session.machine.observe(session.seconds(), &seen);
        let entry = session.machine.take_entry();
        self.flush_tour_notes();
        match entry {
            Some(tour::Step::Build) => {
                self.last = None;
                self.build(false, false, ctx);
                if !self.is_building() {
                    self.fail_tour("the project build could not start");
                }
            }
            Some(tour::Step::Run) => {
                self.tour.as_mut().unwrap().lit_before = lit;
                self.app.rate(Some(4.0));
                self.app.run();
            }
            // Entering a step never navigates. The visitor uses the normal
            // controls, or explicitly asks Show me to demonstrate them.
            Some(tour::Step::Back) => {
                self.tour.as_mut().unwrap().jumped = false;
            }
            _ => {}
        }
        let session = self.tour.as_ref().unwrap();
        if session.machine.step == tour::Step::Back && self.tab == ViewTab::Chip {
            if let Some(point) = self.app.probed_point() {
                let session = self.tour.as_mut().unwrap();
                if session.target.is_none() {
                    session.target = Some((point, session.seconds()));
                    let map = self.app.map_rect();
                    let share = (point - map.min) / map.size();
                    let note = format!("tour pointer on probed wire {:.4} {:.4}", share.x, share.y);
                    self.record(&format!("# {note}"));
                    self.app.record_note(&note);
                } else {
                    session.target = Some((point, session.seconds()));
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    /// Only an explicit Show me request reaches this driver. Navigation
    /// waits between pointing and acting; Back uses the real map menu.
    fn demonstrate_tour(&mut self, ctx: &egui::Context) {
        let Some(session) = self.tour.as_mut() else {
            return;
        };
        let Some(mut demo) = session.demo.take() else {
            return;
        };
        if !session.machine.active() {
            return;
        }
        let now = session.seconds();
        let step = session.machine.step;
        let elapsed = now - demo.since;
        let label = |text: &str| tour::labelled_rects(ctx, |s| s == text).first().copied();
        let mut finished = false;
        match (step, demo.phase) {
            (tour::Step::Chip, 0) => {
                if let Some(rect) = label("Emulate") {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Chip, 1) if elapsed >= 1.0 => {
                let session = self.tour.as_mut().unwrap();
                if session.machine.prebuilt {
                    if let Err(error) = session.project.check_prebuilt() {
                        self.fail_tour(&error.to_string());
                        return;
                    }
                    let prebuilt = session.project.prebuilt.clone();
                    session.prebuilt_opened = true;
                    self.open_emulation(
                        prebuilt.join("top.config"),
                        prebuilt.join("top_routed.json"),
                    );
                } else {
                    self.emulate();
                }
                finished = true;
            }
            (tour::Step::Source | tour::Step::Waves, 0) => {
                if let Some(rect) = label(if step == tour::Step::Source {
                    "Project"
                } else {
                    "Chip"
                }) {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Source | tour::Step::Waves, 1) if elapsed >= 1.0 => {
                self.set_tab(if step == tour::Step::Source {
                    ViewTab::Project
                } else {
                    ViewTab::Chip
                });
                demo.advance(now);
            }
            (tour::Step::Source, 2) => {
                if let Some(rect) = label("top.v") {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Source, 3) if elapsed >= 1.0 => {
                self.open_relative(Path::new("top.v"), None);
                demo.advance(now);
            }
            (tour::Step::Source, 4) => {
                if let Some(point) = tour::counter_point(ctx) {
                    demo.point(point);
                    demo.advance(now);
                }
            }
            (tour::Step::Source, 5) if elapsed >= 1.0 => {
                let path = self.tour.as_ref().unwrap().project.root.join("top.v");
                if let Some((line, column)) =
                    self.editor.text(&path).and_then(tour::counter_location)
                {
                    if let Some(probe) = self.editor.select_probe(&path, line, column) {
                        self.probe(probe);
                    }
                } else {
                    self.fail_tour("count <= count + 1 was not found in top.v");
                }
                finished = true;
            }
            (tour::Step::Waves, 2) => {
                if let Some(rect) = label("Watch in Waves") {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Waves, 3) if elapsed >= 1.0 => {
                self.app.watch_probed();
                finished = true;
            }
            (tour::Step::Back, 0) => {
                if self.tab == ViewTab::Chip {
                    demo.phase = 2;
                } else if let Some(rect) = label("Chip") {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Back, 1) if elapsed >= 1.0 => {
                self.set_tab(ViewTab::Chip);
                demo.advance(now);
            }
            (tour::Step::Back, 2) => {
                if let Some((point, _)) = self.tour.as_ref().unwrap().target {
                    demo.point(point);
                    demo.advance(now);
                }
            }
            (tour::Step::Back, 3) if elapsed >= 1.0 => {
                self.tour.as_mut().unwrap().jump_requested = true;
                demo.button(egui::PointerButton::Secondary, true);
                demo.advance(now);
            }
            (tour::Step::Back, 4) if elapsed >= 0.25 => {
                demo.button(egui::PointerButton::Secondary, false);
                demo.advance(now);
            }
            (tour::Step::Back, 5) if elapsed >= 1.0 => {
                if let Some(rect) = self.tour.as_ref().unwrap().menu {
                    demo.point(rect.center());
                    demo.advance(now);
                }
            }
            (tour::Step::Back, 6) if elapsed >= 1.0 => {
                demo.button(egui::PointerButton::Primary, true);
                demo.advance(now);
            }
            (tour::Step::Back, 7) if elapsed >= 0.25 => {
                demo.button(egui::PointerButton::Primary, false);
                demo.advance(now);
            }
            (tour::Step::Back, 8) if elapsed >= 0.25 => {
                // A menu sizing pass can swallow a synthetic click. The
                // visitor requested this visible choice with Show me, so
                // finish its action if the normal click has not dispatched.
                if self.tab == ViewTab::Chip {
                    if let Some((point, _)) = self.tour.as_ref().unwrap().target {
                        self.app.show_source_at(point);
                    }
                }
                finished = true;
            }
            _ => {}
        }
        if !finished {
            self.tour.as_mut().unwrap().demo = Some(demo);
        }
    }

    fn tour_card(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.tour else {
            return;
        };
        if !session.machine.visible(session.seconds()) {
            return;
        }
        let area = if self.tab == ViewTab::Chip && self.app.map_rect().is_positive() {
            self.app.map_rect()
        } else {
            ui.ctx().content_rect().shrink2(egui::vec2(16.0, 240.0))
        };
        let step = session.machine.step;
        let menu = tour::labelled_rects(ui.ctx(), |text| text.starts_with("Show in source ("))
            .first()
            .copied();
        let guidance = match step {
            tour::Step::Chip => Some("Emulate"),
            tour::Step::Source if self.tab == ViewTab::Chip => Some("Project"),
            tour::Step::Source
                if self.editor.active_path()
                    != Some(session.project.root.join("top.v").as_path()) =>
            {
                Some("top.v")
            }
            tour::Step::Back | tour::Step::Waves if self.tab == ViewTab::Project => Some("Chip"),
            tour::Step::Waves => Some("Watch in Waves"),
            _ => None,
        };
        let guide_point = guidance
            .and_then(|label| {
                tour::labelled_rects(ui.ctx(), |text| text == label)
                    .first()
                    .map(egui::Rect::center)
            })
            .or_else(|| {
                (step == tour::Step::Source)
                    .then(|| tour::counter_point(ui.ctx()))
                    .flatten()
            });
        let session = self.tour.as_mut().unwrap();
        session.menu = menu;
        self.demonstrate_tour(ui.ctx());
        let session = self.tour.as_ref().unwrap();
        let target = session
            .demo
            .as_ref()
            .and_then(|d| d.pointer)
            .or_else(|| {
                if self.tab == ViewTab::Chip {
                    session
                        .menu
                        .map(|rect| rect.center())
                        .or_else(|| session.target.map(|(point, _)| point))
                } else {
                    None
                }
            })
            .or(guide_point);
        let mut overlay = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("tour_overlay")
                .layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("tour_overlay"),
                ))
                .max_rect(ui.ctx().content_rect()),
        );
        let (_, stop, next, show_me) = tour::card(
            &mut overlay,
            area,
            &self.paths.text(session.machine.caption()),
            session.machine.step,
            session.machine.active(),
            target,
            session.machine.waiting(),
        );
        if let Some(point) = target {
            let colour = crate::theme::colours(ui.visuals().dark_mode).tour;
            overlay
                .painter()
                .circle_stroke(point, 11.0, egui::Stroke::new(2.0, colour));
            overlay.painter().arrow(
                point + egui::vec2(26.0, -30.0),
                egui::vec2(-20.0, 24.0),
                egui::Stroke::new(3.0, colour),
            );
        }
        if stop {
            self.stop_tour();
        } else if next {
            let session = self.tour.as_mut().unwrap();
            session.machine.next(session.seconds());
            session.target = None;
            session.demo = None;
            self.flush_tour_notes();
        }
        if show_me {
            let session = self.tour.as_mut().unwrap();
            if session.machine.show_me() {
                session.demo = Some(tour::Demo::new(session.seconds()));
            }
            self.flush_tour_notes();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    fn is_building(&self) -> bool {
        matches!(self.active, Some(ActiveTask::Build { .. }))
    }

    fn record(&mut self, event: &str) {
        if let Some(t) = &mut self.trace {
            let event = self
                .tour
                .as_ref()
                .map_or_else(|| event.to_owned(), |tour| tour.relative(event));
            t.record(&self.paths.text(&event));
        }
    }

    fn set_tab(&mut self, tab: ViewTab) {
        if self.tab != tab {
            self.tab = tab;
            self.record(&format!("view {}", tab.as_str()));
        }
    }

    fn begin_new_file(&mut self, suggested: Option<PathBuf>) {
        if !self.creation_allowed()
            || self.new_file_dialog.is_some()
            || self.memory_image_change.is_some()
        {
            return;
        }
        if let Ok(project) = &self.project {
            self.new_file_dialog = Some(new_file::Dialog::new(
                suggested.unwrap_or_else(|| new_file::suggested_hdl(project)),
            ));
        }
    }

    fn new_file_ui(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.app.new_file_requested) {
            self.begin_new_file(None);
        }
        let Some(mut dialog) = self.new_file_dialog.take() else {
            return;
        };
        match dialog.draw(ctx) {
            Some(new_file::Action::Cancel) => {}
            Some(new_file::Action::Create(request)) => {
                if let Err(reason) = self.create_file(request, ctx) {
                    dialog.error = Some(reason);
                    self.new_file_dialog = Some(dialog);
                }
            }
            None => self.new_file_dialog = Some(dialog),
        }
    }

    fn create_file(
        &mut self,
        request: new_file::Request,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let label = crate::new_project::atom(&self.paths.path(&request.path));
        let result = self.publish_file(&request, ctx);
        match result {
            Ok(notes) => {
                self.creation_note(&format!("file new {label} ok"));
                for note in &notes {
                    self.creation_note(&format!("manifest {note} ok"));
                }
                self.status = format!(
                    "Created {label}; rebuild needed{}",
                    if notes.is_empty() {
                        String::new()
                    } else {
                        format!("; {}", notes.join("; "))
                    }
                );
                Ok(())
            }
            Err(error) => {
                let reason = self.paths.text(&format!("{error:#}"));
                let record_reason = reason.replace(['\n', '\r', '\t'], " ");
                self.creation_note(&format!("file new {label} failed {record_reason}"));
                self.status = format!("New file failed: {reason}");
                Err(reason)
            }
        }
    }

    fn publish_file(
        &mut self,
        request: &new_file::Request,
        ctx: &egui::Context,
    ) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(
            self.creation_allowed(),
            "Wait for the current build or emulation to finish"
        );
        let root = self
            .project
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?
            .root
            .clone();
        let manifest = root.join("ecp5.toml");
        anyhow::ensure!(
            !self.editor.is_dirty(&manifest),
            "Save ecp5.toml before creating a file"
        );
        // The CLI output override is also a build directory, even when absent from the manifest.
        if let Some(project) = self
            .project
            .as_ref()
            .ok()
            .filter(|_| self.out_override.is_some())
        {
            let mut parent = root.join(&request.path);
            let mut tail = Vec::new();
            while !parent.exists() {
                if let Some(leaf) = parent.file_name() {
                    tail.push(leaf.to_os_string());
                }
                let Some(ancestor) = parent.parent() else {
                    break;
                };
                parent = ancestor.to_path_buf();
            }
            if let Ok(mut resolved) = parent.canonicalize() {
                for leaf in tail.into_iter().rev() {
                    resolved.push(leaf);
                }
                anyhow::ensure!(
                    !resolved.starts_with(&project.build_dir),
                    "new file cannot be inside the build output folder"
                );
            }
        }
        let outcome = flow::project_files::new_file(
            &root,
            &request.path,
            &flow::project_files::NewFile {
                name: request.name.clone(),
            },
        )?;
        self.refresh_project_inputs(&root, ctx)?;
        if request.path.extension().is_some_and(|e| e == "lpf") {
            self.inactive_constraints
                .push(outcome.path.strip_prefix(&root)?.to_path_buf());
        }
        self.open_in_editor(&outcome.path, self.paths.path(&request.path), false, None);
        Ok(outcome.notes)
    }

    fn begin_memory_image(&mut self, relative: PathBuf, use_as_image: bool) {
        if self.creation_allowed()
            && self.new_file_dialog.is_none()
            && self.memory_image_change.is_none()
        {
            self.memory_image_change = Some(MemoryImageChange {
                relative,
                use_as_image,
                error: None,
            });
        }
    }

    fn memory_image_ui(&mut self, ctx: &egui::Context) {
        let Some(mut change) = self.memory_image_change.take() else {
            return;
        };
        let Some(root) = self.project.as_ref().ok().map(|p| p.root.clone()) else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("memory_image_change")).show(ctx, |ui| {
            ui.set_width(580.0);
            ui.heading(if change.use_as_image { "Use as BRAM image (ecpbram)?" } else { "Stop using as BRAM image?" });
            ui.label(self.paths.path(&change.relative));
            ui.label(MEMORY_IMAGE_CONFIRMATION);
            let mut dirty = false;
            for name in ["ecp5.toml", "ecpbram.txt"] {
                let path = root.join(name);
                if self.editor.is_dirty(&path) {
                    dirty = true;
                    ui.label(format!("{name} has unsaved edits. Save them here, or Cancel to review or revert them."));
                    if ui.button(format!("Save {name}")).clicked() {
                        let result = self.editor.open(&path, name.into(), false, None)
                            .and_then(|()| self.editor.save_active());
                        if let Err(error) = result { change.error = Some(self.paths.text(&format!("{error:#}"))); }
                    }
                }
            }
            if let Some(error) = &change.error { ui.colored_label(ui.visuals().error_fg_color, error); }
            ui.horizontal(|ui| {
                let enabled = !dirty && self.creation_allowed();
                apply = ui.add_enabled(enabled, egui::Button::new("Apply")).clicked()
                    || (enabled && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel {
            return;
        }
        if apply {
            if let Err(reason) =
                self.change_memory_image(&change.relative, change.use_as_image, ctx)
            {
                change.error = Some(reason);
            } else {
                return;
            }
        }
        self.memory_image_change = Some(change);
    }

    fn change_memory_image(
        &mut self,
        relative: &Path,
        use_as_image: bool,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let label = crate::new_project::atom(&self.paths.path(relative));
        let state = if use_as_image { "on" } else { "off" };
        match self.publish_memory_image(relative, use_as_image, ctx) {
            Ok(notes) => {
                self.creation_note(&format!("file memory-image {label} {state} ok"));
                for note in &notes {
                    self.creation_note(&format!("manifest {note} ok"));
                }
                self.status = format!(
                    "BRAM image {label} {state}; rebuild needed{}",
                    if notes.is_empty() {
                        String::new()
                    } else {
                        format!("; {}", notes.join("; "))
                    }
                );
                Ok(())
            }
            Err(error) => {
                let reason = self.paths.text(&format!("{error:#}"));
                self.creation_note(&format!(
                    "file memory-image {label} {state} failed {}",
                    reason.replace(['\n', '\r', '\t'], " ")
                ));
                self.status = format!("BRAM image change failed: {reason}");
                Err(reason)
            }
        }
    }

    fn publish_memory_image(
        &mut self,
        relative: &Path,
        use_as_image: bool,
        ctx: &egui::Context,
    ) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(
            self.creation_allowed(),
            "Wait for the current build or emulation to finish"
        );
        let project = self
            .project
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?;
        let root = project.root.clone();
        for name in ["ecp5.toml", "ecpbram.txt"] {
            anyhow::ensure!(
                !self.editor.is_dirty(&root.join(name)),
                "Save {name} before changing BRAM images"
            );
        }
        // Include the CLI output override, which is not part of the manifest.
        anyhow::ensure!(
            !root
                .join(relative)
                .canonicalize()?
                .starts_with(&project.build_dir),
            "memory image cannot be inside the build output folder"
        );
        let outcome = flow::project_files::set_memory_image(&root, relative, use_as_image)?;
        self.refresh_project_inputs(&root, ctx)?;
        self.editor.refresh_clean(&root.join("ecpbram.txt"))?;
        Ok(outcome.notes)
    }

    /// Inputs changed on disk: retire the previous build and emulator, retain
    /// other editor buffers and refresh only the clean manifest tab.
    fn refresh_project_inputs(&mut self, root: &Path, ctx: &egui::Context) -> anyhow::Result<()> {
        self.app.reset_for_project(ctx);
        self.app.set_display_paths(self.paths.clone());
        self.last = None;
        self.needs_rebuild = true;
        self.emulated_root = None;
        self.emulate_after = false;
        self.chip_loading = None;
        self.pending_probe = None;
        self.probe_summary = None;
        self.editor.probe_status = None;
        self.editor.watch_values.clear();
        self.project =
            open_project_with_out(root, self.out_override.as_deref()).map_err(|e| format!("{e:#}"));
        self.project
            .as_ref()
            .map_err(|e| anyhow::anyhow!("Project inputs changed, but refresh failed: {e}"))?;
        self.editor.refresh_clean(&root.join("ecp5.toml"))?;
        if let Ok(project) = &self.project {
            self.app.set_project_title(&project.name);
        }
        self.steps = self
            .project
            .as_ref()
            .map_or_else(|_| Vec::new(), tree::default_steps);
        self.messages.clear();
        self.log.clear();
        self.output.critical_clock = None;
        self.set_tab(ViewTab::Project);
        Ok(())
    }

    fn open_project(&mut self, dir: &Path, ctx: &egui::Context) {
        if self.is_busy() {
            return;
        }
        let dir = self.paths.expand_path(dir);
        self.paths.set_root(None);
        self.path_input = self.paths.path(&dir);
        self.last = None;
        self.needs_rebuild = false;
        self.new_file_dialog = None;
        self.memory_image_change = None;
        self.inactive_constraints.clear();
        self.messages.clear();
        self.log.clear();
        self.emulate_after = false;
        self.editor.clear();
        self.pending_probe = None;
        self.probe_summary = None;
        self.emulated_root = None;
        match open_project_with_out(&dir, self.out_override.as_deref()) {
            Ok(p) => {
                self.paths.set_root(Some(&p.root));
                self.path_input = self.paths.path(&p.root);
                self.steps = tree::default_steps(&p);
                self.status = "Idle".into();
                self.record(&format!("project {} .", p.name));
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "ecp5_emu — {}",
                    self.paths.text(&p.name)
                )));
                self.project = Ok(p);
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.steps.clear();
                self.status = "Project error".into();
                self.project = Err(msg);
            }
        }
        self.app.set_display_paths(self.paths.clone());
        if let Ok(project) = &self.project {
            self.app.set_project_title(&project.name);
        }
    }

    fn build(&mut self, force: bool, emulate: bool, ctx: &egui::Context) {
        if let Some(ActiveTask::Build { .. }) = self.active {
            self.emulate_after |= emulate;
            return;
        }
        let Ok(mut project) = self.project.clone() else {
            return;
        };

        // Save every dirty editor buffer before launching the toolchain.
        let saved_prefix = match self.editor.save_all_dirty() {
            Ok((0, _)) => None,
            Ok((n, manifest_saved)) => {
                if manifest_saved {
                    match open_project_with_out(&project.root, self.out_override.as_deref()) {
                        Ok(reloaded) => {
                            project = reloaded.clone();
                            self.project = Ok(reloaded);
                        }
                        Err(e) => {
                            let msg = format!("{e:#}");
                            self.status = format!("Save/project error: {msg}");
                            self.project = Err(msg);
                            return;
                        }
                    }
                }
                Some(format!(
                    "Saved {} {}; ",
                    n,
                    if n == 1 { "file" } else { "files" }
                ))
            }
            Err(e) => {
                self.status = format!("Save failed: {e:#}");
                return;
            }
        };

        self.steps = tree::default_steps(&project);
        self.messages.clear();
        self.emulate_after = emulate;
        self.probe_summary = None;
        self.editor.probe_status = None;
        self.record(&format!("build start force={force}"));

        let (send, events) = mpsc::channel();
        let (finish, done) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let options = BuildOptions {
            force,
            tools: Tools::from_env(),
            cancel: Some(Arc::clone(&cancel)),
        };
        let ctx_bg = ctx.clone();
        std::thread::spawn(move || {
            let outcome = flow::build(&project, &options, &mut |event| {
                let _ = send.send(event);
                ctx_bg.request_repaint();
            });
            let _ = finish.send(outcome);
            ctx_bg.request_repaint();
        });

        self.active = Some(ActiveTask::Build {
            events,
            done,
            cancel,
            started: Instant::now(),
            step_started: None,
            last_line: None,
        });
        self.status = format!("{}Building …", saved_prefix.as_deref().unwrap_or(""));
    }

    /// Why Emulate cannot open a build or queue the running build's result.
    /// Reopened projects need both emulator outputs; newer sources are allowed.
    fn emulate_blocker(&self) -> Option<String> {
        let Ok(project) = &self.project else {
            return Some("Open a valid project first".into());
        };
        if let Err(reason) = project.emulatable() {
            return Some(reason);
        }
        if self.is_building() {
            return None;
        }
        if self.needs_rebuild {
            return Some("Project inputs changed; build the project before emulating".into());
        }
        match &self.last {
            Some(outcome) => (!outcome.ok).then(|| "Last build did not succeed".into()),
            None => {
                let outputs = project.outputs();
                (!(outputs.config.is_file() && outputs.routed_json.is_file()))
                    .then(|| "Build the project first".into())
            }
        }
    }

    /// Opens this session's successful build or existing outputs in Chip.
    /// Returns whether the request was accepted, including queuing a running build.
    fn emulate(&mut self) -> bool {
        if let Some(reason) = self.emulate_blocker() {
            self.record(&format!("# emulate blocked {reason}"));
            self.status = reason;
            return false;
        }
        if self.is_building() {
            self.emulate_after = true;
            return true;
        }
        let Ok(project) = &self.project else {
            return false;
        };
        let outputs = self
            .last
            .as_ref()
            .map_or_else(|| project.outputs(), |outcome| outcome.outputs.clone());
        self.open_emulation(outputs.config, outputs.routed_json);
        true
    }

    /// Built and shipped designs share source files, loading and cross-probing.
    fn open_emulation(&mut self, config: PathBuf, names: PathBuf) {
        if let Err(reason) = self.app.prepare_reader() {
            self.record(&format!("chip error {reason}"));
            self.status = reason;
            return;
        }
        let Ok(project) = &self.project else {
            return;
        };
        self.app.project_sources(
            project.root.clone(),
            project.hdl_roots().iter().chain(&project.files).cloned(),
        );
        self.pending_probe = None;
        self.probe_summary = None;
        self.editor.probe_status = None;
        self.emulated_root = Some(project.root.clone());
        self.app.open(
            self.paths.text(&project.name),
            live::Design {
                config: config.clone(),
                names: Some(names),
                run: !self.tour.as_ref().is_some_and(|t| t.machine.active()),
            },
        );
        if let Some(rate) = self.rate {
            self.app.rate(rate);
        }
        let path = config.strip_prefix(&project.root).unwrap_or(&config);
        self.record(&format!("emulate {}", self.paths.path(path)));
        self.chip_loading = Some(Instant::now());
        self.set_tab(ViewTab::Chip);
    }
    /// Follows the design opened in the emulator until it is ready or
    /// failed: the status says how far loading is, the record when it ended.
    fn poll_chip(&mut self) {
        let Some(since) = self.chip_loading else {
            return;
        };
        let (stage, ready, error) = self.app.state();
        let secs = since.elapsed().as_secs_f64();
        let ended = if let Some(error) = error {
            self.record(&format!("chip error {secs:.2} {error}"));
            self.status = format!("Emulator: {error}");
            true
        } else if ready {
            self.record(&format!("chip ready {secs:.2}"));
            if !self.is_busy() {
                self.status = format!("Emulating (ready in {secs:.1} s)");
            }
            true
        } else {
            if !self.is_busy() {
                self.status = format!("Loading the design: {stage} … {secs:.0} s");
            }
            false
        };
        if ended {
            self.chip_loading = None;
            if let Some(runner) = &mut self.script {
                if runner.waiting() == Some(WaitCondition::Ready) {
                    runner.complete_wait(Instant::now());
                }
            }
        }
    }

    fn open_in_editor(
        &mut self,
        path: &Path,
        label: String,
        read_only: bool,
        line: Option<u32>,
    ) -> bool {
        if let Err(e) = self
            .editor
            .open(path, self.paths.text(&label), read_only, line)
        {
            self.status = format!("Open failed: {e:#}");
            self.record(&format!("open failed {e:#}"));
            return false;
        }
        let line_no = line.unwrap_or(1);
        self.record(&format!(
            "open {label}:{line_no} language={}",
            language_label(
                self.editor
                    .active_language()
                    .unwrap_or_else(|| editor::Language::for_path(path))
            )
        ));
        true
    }

    fn open_relative(&mut self, rel: &Path, line: Option<u32>) {
        let Ok(project) = &self.project else {
            return;
        };
        let full = project.root.join(rel);
        let label = self.paths.path(rel);
        self.open_in_editor(&full, label, false, line);
    }

    fn probe(&mut self, probe: Probe) {
        self.pending_probe = None;
        let Ok(project) = &self.project else { return };
        let Some(relative) = project
            .hdl_roots()
            .iter()
            .chain(&project.files)
            .find(|p| project.root.join(p) == probe.path)
        else {
            return;
        };
        let file = relative.to_string_lossy().into_owned();
        if !self.app.has_design() || self.emulated_root.as_ref() != Some(&project.root) {
            self.status = "Emulate this project to probe its sources".into();
            return;
        }
        let warning = self.stale_warning(&probe.path, &file);
        if let Some(mut status) = self.app.probe_source(
            &file,
            probe.line,
            probe.column,
            &probe.identifier,
            warning.as_deref(),
        ) {
            self.probe_summary = Some((probe.clone(), file.clone(), status.clone()));
            if let Some(warning) = warning {
                status.push_str(&format!("; {warning}"));
            }
            self.status = status.clone();
            self.editor.probe_status = Some(status);
        } else if self.app.state().2.is_none() {
            self.status = "Source index is being prepared".into();
            self.pending_probe = Some(probe);
        } else {
            self.status = "Source probing is unavailable: the design failed to load".into();
        }
    }

    fn stale_warning(&self, path: &Path, file: &str) -> Option<String> {
        let built = self
            .app
            .ready()
            .and_then(|s| s.sources.get().and_then(|i| i.built_at));
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        source_warning(file, self.editor.is_dirty(path), modified, built)
    }

    fn show_source(&mut self, span: Span) {
        let Ok(project) = &self.project else { return };
        if self.emulated_root.as_ref() != Some(&project.root) {
            return;
        }
        let relative = Path::new(span.file.as_ref());
        if !project
            .hdl_roots()
            .iter()
            .chain(&project.files)
            .any(|p| p == relative)
        {
            return;
        }
        let path = project.root.join(relative);
        if !self.open_in_editor(&path, span.file.to_string(), false, Some(span.line)) {
            return;
        }
        self.editor.select_span(span.clone());
        self.probe_summary = None;
        self.editor.probe_status = None;
        self.status = span.label();
        if let Some(warning) = self.stale_warning(&path, &span.file) {
            self.status = warning;
        }
        self.set_tab(ViewTab::Project);
        if let Some(tour) = &mut self.tour {
            if tour.machine.step == tour::Step::Back && tour.jump_requested {
                tour.jumped = true;
                tour.editor_shown = false;
                tour.target = None;
            }
        }
    }

    fn poll_probes(&mut self) {
        self.app.poll_sources();
        if let Some(probe) = self.pending_probe.take() {
            self.probe(probe);
        }
        if self.app.take_chip_jump() {
            self.set_tab(ViewTab::Chip);
        }
        if let Some(span) = self.app.take_source_jump() {
            self.show_source(span);
        }
        for note in self.app.take_probe_notes() {
            self.record(&note);
        }
        if let Some((probe, file, summary)) = &self.probe_summary {
            let mut status = summary.clone();
            if let Some(warning) = self.stale_warning(&probe.path, file) {
                status.push_str(&format!("; {warning}"));
            }
            self.status = status.clone();
            self.editor.probe_status = Some(status);
        }
        if self.pending_probe.is_none() {
            if let Some(runner) = &mut self.script {
                if runner.waiting() == Some(WaitCondition::Sources) {
                    runner.complete_wait(Instant::now());
                }
            }
        }
    }

    fn poll(&mut self) {
        let mut finished_build: Option<(Outcome, f64)> = None;
        let mut trace_events: Vec<String> = Vec::new();
        let mut live_status: Option<String> = None;

        if let Some(ActiveTask::Build {
            events,
            done,
            started,
            step_started,
            last_line,
            ..
        }) = &mut self.active
        {
            while let Ok(event) = events.try_recv() {
                match event {
                    Event::Started { step, .. } => {
                        *step_started = Some((step, Instant::now()));
                        *last_line = None;
                        if let Some((_, s)) = self.steps.iter_mut().find(|(k, _)| *k == step) {
                            *s = StepProgress::Running {
                                started: Instant::now(),
                            };
                        }
                        trace_events.push(format!("step {} start", step_key(step)));
                    }
                    Event::Skipped { step } => {
                        if let Some((_, s)) = self.steps.iter_mut().find(|(k, _)| *k == step) {
                            *s = StepProgress::Skipped;
                        }
                        trace_events.push(format!("step {} skipped", step_key(step)));
                    }
                    Event::Output { step, line } => {
                        if !line.trim().is_empty() {
                            *last_line = Some(line.trim().to_string());
                        }
                        self.log.push(LogSource::Step(step), line);
                    }
                    Event::Message(msg) => {
                        trace_events.push(format_trace_message(self.project.as_ref().ok(), &msg));
                        if let Some(existing) = self.messages.iter_mut().find(|m| {
                            m.severity == msg.severity
                                && m.step == msg.step
                                && m.text == msg.text
                                && m.file == msg.file
                                && m.line == msg.line
                                && m.column == msg.column
                        }) {
                            existing.count += 1;
                        } else {
                            self.messages.push(msg);
                        }
                    }
                    Event::Finished { step, ok, seconds } => {
                        if let Some((_, s)) = self.steps.iter_mut().find(|(k, _)| *k == step) {
                            *s = if ok {
                                StepProgress::Done { seconds }
                            } else {
                                StepProgress::Failed {
                                    seconds: Some(seconds),
                                }
                            };
                        }
                        if ok {
                            trace_events.push(format!("step {} done {seconds:.2}", step_key(step)));
                        } else {
                            trace_events.push(format!("step {} failed", step_key(step)));
                        }
                    }
                }
            }
            if let Some((step, st)) = step_started {
                let secs = st.elapsed().as_secs_f64();
                live_status = Some(match last_line {
                    Some(line) => {
                        format!(
                            "Building: {step} {secs:.1} s — {}",
                            truncate(&self.paths.text(line), 64)
                        )
                    }
                    None => format!("Building: {step} {secs:.1} s"),
                });
            }
            if let Ok(outcome) = done.try_recv() {
                finished_build = Some((outcome, started.elapsed().as_secs_f64()));
            }
        }

        for ev in trace_events {
            self.record(&ev);
        }
        if let Some(status) = live_status {
            self.status = status;
        }

        if let Some((outcome, total_secs)) = finished_build {
            self.active = None;
            tree::sync_step_status(&mut self.steps, &outcome.steps);
            for (step, st) in &outcome.steps {
                if *st == ecp5_emu::flow::StepStatus::NotRun {
                    self.record(&format!("step {} not-run", step_key(*step)));
                }
            }
            self.messages = outcome.messages.clone();
            let errors = self
                .messages
                .iter()
                .filter(|m| m.severity == Severity::Error)
                .count();
            let warnings = self
                .messages
                .iter()
                .filter(|m| m.severity == Severity::Warning)
                .count();

            self.status = if outcome.cancelled {
                "Build stopped".into()
            } else if outcome.ok {
                format!(
                    "Built in {total_secs:.1} s: {errors} {}, {warnings} {}",
                    plural(errors, "error"),
                    plural(warnings, "warning"),
                )
            } else {
                format!("Build failed: {errors} {}", plural(errors, "error"))
            };

            let tag = if outcome.cancelled {
                "cancelled"
            } else if outcome.ok {
                "ok"
            } else {
                "failed"
            };
            self.record(&format!(
                "build {tag} {total_secs:.2} errors={errors} warnings={warnings}"
            ));
            if let Some(report) = &outcome.report {
                self.record(&format!("report {}", report.summary()));
                for line in report.timing_lines() {
                    self.record(&format!("timing {line}"));
                }
            }

            let ok = outcome.ok && errors == 0;
            if ok {
                self.needs_rebuild = false;
            }
            self.last = Some(outcome);

            if ok && std::mem::take(&mut self.emulate_after) {
                self.emulate();
            } else if !ok
                && !self.last.as_ref().is_some_and(|o| o.cancelled)
                && !self.tour.as_ref().is_some_and(|t| t.machine.active())
            {
                self.emulate_after = false;
                self.set_tab(ViewTab::Project);
                self.output.tab = OutputTab::Messages;
                if let Some(loc) = self
                    .messages
                    .iter()
                    .filter(|m| m.severity == Severity::Error)
                    .find_map(|m| output::resolve_message_location(self.project.as_ref().ok(), m))
                {
                    self.open_in_editor(&loc.path, loc.label, false, loc.line);
                }
            } else {
                self.emulate_after = false;
            }

            let opened = self.chip_loading.is_some();
            if let Some(runner) = &mut self.script {
                match runner.waiting() {
                    // Emulate waits for the design to be ready, not only built.
                    Some(WaitCondition::Emulate) if opened => {
                        runner.set_waiting(WaitCondition::Ready)
                    }
                    Some(WaitCondition::Build) | Some(WaitCondition::Emulate) => {
                        runner.complete_wait(Instant::now());
                    }
                    _ => {}
                }
            }
        }
    }

    fn flush_editor_frames(&mut self) {
        let samples = std::mem::take(&mut self.editor_samples);
        for typing in [false, true] {
            let pairs: Vec<_> = samples.iter().filter(|s| s.2 == typing).collect();
            if pairs.is_empty() {
                continue;
            }
            let inline: usize = pairs.iter().map(|s| s.3).sum();
            let frame: Vec<_> = pairs.iter().map(|s| s.0).collect();
            let layout: Vec<_> = pairs.iter().map(|s| s.1).collect();
            self.record(&format!(
                "# editor frames {} kind={} n={} frame_p50_ms={:.4} frame_p95_ms={:.4} frame_max_ms={:.4} layout_p50_ms={:.4} layout_p95_ms={:.4} layout_max_ms={:.4} inline_values={inline}",
                self.editor_key, if typing { "typing" } else { "idle" }, pairs.len(),
                quantile(&frame, 0.5), quantile(&frame, 0.95), quantile(&frame, 1.0),
                quantile(&layout, 0.5), quantile(&layout, 0.95), quantile(&layout, 1.0),
            ));
        }
    }

    fn type_character(&mut self, ctx: &egui::Context) {
        let Some(typing) = &mut self.typing else {
            return;
        };
        let Some(character) = typing.text.pop_front() else {
            return;
        };
        let line = self
            .editor
            .text(&typing.path)
            .and_then(|s| s.split('\n').nth(typing.line as usize - 1));
        let result = line.and_then(|line| {
            let offset = line
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(line.len()))
                .nth(typing.column as usize - 1)?;
            let mut replacement = line.to_owned();
            replacement.insert(offset, character);
            Some(replacement)
        });
        if let Some(line) = result {
            if let Err(error) =
                self.editor
                    .replace_line(&typing.path, typing.label.clone(), typing.line, &line)
            {
                self.status = format!("Script type failed: {error:#}");
                typing.text.clear();
            }
            typing.column += 1;
        } else {
            self.status = "Script type failed: coordinates outside the buffer".into();
            typing.text.clear();
        }
        if self.status.starts_with("Script type failed:") {
            self.record(&format!("# script error {}", self.status));
        }
        ctx.request_repaint();
    }

    fn poll_script(&mut self, ctx: &egui::Context) {
        if self.app.creation_busy
            || self.new_file_dialog.is_some()
            || self.memory_image_change.is_some()
        {
            return;
        }
        let now = Instant::now();
        loop {
            let Some(runner) = &mut self.script else {
                return;
            };
            match runner.poll_next(now) {
                Ok(None) => return,
                Err(wait_secs) => {
                    ctx.request_repaint_after(Duration::from_secs_f64(wait_secs.min(0.25)));
                    return;
                }
                Ok(Some(cmd)) => {
                    self.flush_editor_frames();
                    self.editor_phase += 1;
                    self.record(&format!("# script phase={} {cmd:?}", self.editor_phase));
                    match cmd {
                        ScriptCommand::NewFile(request) => {
                            let _ = self.create_file(request, ctx);
                        }
                        ScriptCommand::MemoryImage { path, use_as_image } => {
                            let _ = self.change_memory_image(&path, use_as_image, ctx);
                        }
                        ScriptCommand::Creation(command) => {
                            self.app.creation_command = Some(command);
                            self.app.creation_busy = true;
                            self.script
                                .as_mut()
                                .unwrap()
                                .set_waiting(WaitCondition::Creation);
                            return;
                        }
                        ScriptCommand::Build => {
                            self.build(false, false, ctx);
                            if self.is_building() {
                                if let Some(r) = &mut self.script {
                                    r.set_waiting(WaitCondition::Build);
                                }
                                return;
                            }
                        }
                        ScriptCommand::Rebuild => {
                            self.build(true, false, ctx);
                            if self.is_building() {
                                if let Some(r) = &mut self.script {
                                    r.set_waiting(WaitCondition::Build);
                                }
                                return;
                            }
                        }
                        ScriptCommand::Emulate => {
                            if !self.emulate() {
                                self.record(&format!("# script error {}", self.status));
                                continue;
                            }
                            if self.is_building() {
                                if let Some(r) = &mut self.script {
                                    r.set_waiting(WaitCondition::Emulate);
                                }
                                return;
                            }
                            if self.chip_loading.is_some() {
                                if let Some(r) = &mut self.script {
                                    r.set_waiting(WaitCondition::Ready);
                                }
                                return;
                            }
                        }
                        ScriptCommand::View(tab) => self.set_tab(tab),
                        ScriptCommand::Open { file, line } => self.open_relative(&file, line),
                        ScriptCommand::Source { file, line, column } => {
                            let allowed = self.project.as_ref().is_ok_and(|p| {
                                p.hdl_roots().iter().chain(&p.files).any(|p| p == &file)
                            });
                            if allowed {
                                let path = self.project.as_ref().unwrap().root.join(&file);
                                self.open_relative(&file, Some(line));
                                if let Some(probe) = self.editor.probe_at(&path, line, column) {
                                    self.probe(probe);
                                }
                                if self.pending_probe.is_some() {
                                    if let Some(runner) = &mut self.script {
                                        runner.set_waiting(WaitCondition::Sources);
                                    }
                                    return;
                                }
                            }
                        }
                        ScriptCommand::Hover { file, line, column } => {
                            let allowed = self.project.as_ref().is_ok_and(|p| {
                                p.verilog.iter().chain(&p.files).any(|p| p == &file)
                            });
                            if allowed {
                                let path = self.project.as_ref().unwrap().root.join(&file);
                                self.open_relative(&file, Some(line));
                                self.editor.script_hover(&path, line, column);
                                if let Some(probe) = self.editor.value_probe_at(&path, line, column)
                                {
                                    let built = self
                                        .app
                                        .ready()
                                        .and_then(|s| s.sources.get().and_then(|i| i.built_at));
                                    let mut values = SourceView {
                                        app: &mut self.app,
                                        project: self.project.as_ref().ok(),
                                        emulated_root: self.emulated_root.as_deref(),
                                        paths: &self.paths,
                                        built,
                                        modified: Default::default(),
                                    };
                                    values.value(&probe, self.editor.is_dirty(&path), true);
                                }
                                self.set_tab(ViewTab::Project);
                            }
                        }
                        ScriptCommand::ClickMessage(index) => {
                            let visible =
                                output::visible_messages(&self.messages, &self.output.filter);
                            if let Some((_, msg)) = visible.get(index) {
                                if let Some(loc) = output::resolve_message_location(
                                    self.project.as_ref().ok(),
                                    msg,
                                ) {
                                    self.open_in_editor(&loc.path, loc.label, false, loc.line);
                                }
                            }
                        }
                        ScriptCommand::Edit { file, line, text } => {
                            if let Ok(project) = &self.project {
                                let full = project.root.join(&file);
                                let label = self.paths.path(&file);
                                if let Err(e) = self.editor.replace_line(&full, label, line, &text)
                                {
                                    self.status = format!("Script edit failed: {e:#}");
                                }
                            }
                        }
                        ScriptCommand::Inline(on) => {
                            self.editor.inline_values = on;
                            self.record(if on { "# inline on" } else { "# inline off" });
                        }
                        ScriptCommand::Type {
                            file,
                            line,
                            column,
                            text,
                        } => {
                            self.open_relative(&file, Some(line));
                            self.set_tab(ViewTab::Project);
                            if let Ok(project) = &self.project {
                                self.typing = Some(Typing {
                                    path: project.root.join(&file),
                                    label: self.paths.path(&file),
                                    line,
                                    column,
                                    text: text.chars().collect(),
                                });
                                self.script
                                    .as_mut()
                                    .unwrap()
                                    .set_waiting(WaitCondition::Typing);
                                ctx.request_repaint();
                                return;
                            }
                        }
                        ScriptCommand::Save => {
                            if let Err(e) = self.editor.save_active() {
                                self.status = format!("Save failed: {e:#}");
                            }
                        }
                        ScriptCommand::Quit => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl Drop for ProjectApp {
    fn drop(&mut self) {
        self.flush_editor_frames();
    }
}

struct Typing {
    path: PathBuf,
    label: String,
    line: u32,
    column: u32,
    text: std::collections::VecDeque<char>,
}

fn language_label(language: editor::Language) -> &'static str {
    match language {
        editor::Language::Verilog => "verilog",
        editor::Language::SystemVerilog => "systemverilog",
        editor::Language::Vhdl => "vhdl",
        editor::Language::Lpf => "lpf",
        editor::Language::Toml => "toml",
        editor::Language::Plain => "plain",
    }
}

fn quantile(values: &[f64], share: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let pos = (values.len() - 1) as f64 * share;
    let low = pos.floor() as usize;
    values[low] + (values[pos.ceil() as usize] - values[low]) * pos.fract()
}

impl ProjectApp {
    pub(crate) fn project_tabs(ui: &mut egui::Ui, state: &mut Toolbar) {
        state.tabs(ui);
    }
    pub(crate) fn build_segment(ui: &mut egui::Ui, state: &mut Toolbar) -> egui::Rect {
        ui.horizontal(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                state.build_action(ui, false);
                ui.menu_button("▾", |ui| state.build_menu(ui))
                    .response
                    .on_hover_text("Build, Rebuild, Stop build and Emulate");
            });
            Self::emulate_control(ui, state);
        })
        .response
        .rect
    }
    pub(crate) fn emulate_control(ui: &mut egui::Ui, state: &mut Toolbar) {
        state.emulate(ui);
    }
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        use crate::app::toolbar::{self, Rects};
        let compact = ui.ctx().content_rect().width() < 1500.0;
        let mut state = Toolbar {
            tab: self.tab,
            idle: !self.is_busy() && self.project.is_ok(),
            building: self.is_building(),
            emulate_blocker: self
                .emulate_blocker()
                .map(|reason| self.paths.text(&reason)),
            action: None,
            #[cfg(test)]
            controls: Vec::new(),
        };
        let snapshot = self.app.toolbar_snapshot();
        let name = self
            .project
            .as_ref()
            .map_or_else(|_| "No project".into(), |p| self.paths.text(&p.name));
        let path = self
            .project
            .as_ref()
            .map_or_else(|e| self.paths.text(e), |p| self.paths.path(&p.root));
        let mut action = None;
        let mut rects = None;
        self.app.begin_toolbar();
        egui::Panel::top("controls")
            .exact_size(46.0)
            .show(ui, |ui| {
                toolbar::prepare(ui);
                let row = ui.horizontal_centered(|ui| {
                    let navigation = ui
                        .horizontal(|ui| {
                            action = self.app.design_menu(ui, Some(&mut state));
                            self.app.peripherals_segment(ui);
                            if let Some(a) = self.app.new_project_segment(ui) {
                                action = Some(a);
                            }
                            self.app.toolbar_name(ui, &name, &path, compact);
                            Self::project_tabs(ui, &mut state);
                        })
                        .response
                        .rect;
                    toolbar::divider(ui);
                    let build = Self::build_segment(ui, &mut state);
                    toolbar::divider(ui);
                    let transport = self.app.transport_segment(ui, snapshot, compact);
                    toolbar::divider(ui);
                    let time = self.app.time_segment(ui, snapshot, compact);
                    let view = self.app.right_view_segment(ui, snapshot, compact);
                    (navigation, build, transport, time, view)
                });
                rects = Some(Rects {
                    row: row.response.rect,
                    navigation: row.inner.0,
                    build: Some(row.inner.1),
                    transport: row.inner.2,
                    time: row.inner.3,
                    view: row.inner.4,
                });
            });
        self.app.finish_toolbar(ui, rects.unwrap());
        #[cfg(test)]
        self.app.toolbar_test_project_controls(state.controls);
        if let Some(action) = action {
            self.app.action(ui.ctx(), action);
        }
        if let Some(action) = state.action {
            match action {
                ToolbarAction::Tab(tab) => self.set_tab(tab),
                ToolbarAction::Build(clean) => self.build(clean, false, ui.ctx()),
                ToolbarAction::Stop => {
                    if let Some(ActiveTask::Build { cancel, .. }) = &self.active {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                ToolbarAction::Emulate => {
                    self.emulate();
                }
            }
        }
    }
}

impl eframe::App for ProjectApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Navigation never ends the tour. Observe only the source-menu
        // gesture needed as evidence, before adding a requested demonstration.
        if let Some(session) = self.tour.as_mut().filter(|s| s.machine.active()) {
            for event in &raw_input.events {
                if let egui::Event::PointerButton {
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    ..
                } = event
                {
                    if session.machine.step == tour::Step::Back {
                        session.jump_requested = true;
                    }
                }
            }
        }
        eframe::App::raw_input_hook(&mut self.app, ctx, raw_input);
        if let Some(demo) = self
            .tour
            .as_mut()
            .filter(|s| s.machine.active())
            .and_then(|s| s.demo.as_mut())
        {
            if let Some(point) = demo.pointer {
                raw_input.events.push(egui::Event::PointerMoved(point));
            }
            raw_input.events.append(&mut demo.events);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = Instant::now();
        let ctx = ui.ctx().clone();
        self.app.new_file_available = self.project.is_ok()
            && self.creation_allowed()
            && self.new_file_dialog.is_none()
            && self.memory_image_change.is_none();
        let new_file_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N);
        if self.app.new_file_available
            && !ctx.egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_shortcut(&new_file_shortcut))
        {
            self.begin_new_file(None);
        }
        self.type_character(&ctx);
        self.poll();
        self.poll_chip();
        self.poll_probes();
        self.poll_script(&ctx);
        self.poll_tour(&ctx);
        if self.tab == ViewTab::Project {
            self.app.script_in_project(ui);
        }
        if self.is_busy() || self.chip_loading.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Build shortcut (Cmd+B / Ctrl+B).
        let build_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::B);
        if !self.is_busy()
            && self.project.is_ok()
            && ui.input_mut(|i| i.consume_shortcut(&build_shortcut))
        {
            self.build(false, false, &ctx);
        }

        self.toolbar(ui);
        // Build progress belongs to the bottom on both tabs, above the output pane.
        egui::Panel::bottom("project_status")
            .exact_size(28.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [(ui.available_width() * 0.55).max(80.0), 22.0],
                        egui::Label::new(self.paths.text(&self.status)).truncate(),
                    )
                    .on_hover_text(self.paths.text(&self.status));
                    if let Some(ActiveTask::Build { cancel, .. }) = &self.active {
                        ui.spinner();
                        if ui
                            .button("Stop")
                            .on_hover_text("Stop the running build")
                            .clicked()
                        {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    if self.tab == ViewTab::Project {
                        self.app.emulator_status(ui);
                    }
                });
            });

        // (2) Chip tab: delegate exclusively to inner App.
        if self.tab == ViewTab::Chip {
            eframe::App::ui(&mut self.app, ui, frame);
            self.poll_probes();
            self.tour_card(ui);
            self.new_file_ui(&ctx);
            self.memory_image_ui(&ctx);
            return;
        }

        // (3) Project tab: left tree, bottom output, central editor.
        let busy = !self.creation_allowed() || self.memory_image_change.is_some();
        let mut tree_action = None;
        egui::Panel::left("project_tree")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                tree_action = tree::show(
                    ui,
                    &mut self.path_input,
                    &self.project,
                    &self.steps,
                    &self.examples,
                    self.app.dev_mode(),
                    &self.editor,
                    &self.inactive_constraints,
                    busy,
                    &self.paths,
                );
            });
        if let Some(action) = tree_action {
            match action {
                TreeAction::NewFile(path) => self.begin_new_file(Some(path)),
                TreeAction::MemoryImage {
                    relative,
                    use_as_image,
                } => self.begin_memory_image(relative, use_as_image),
                TreeAction::OpenProject(dir) => self.open_project(&dir, &ctx),
                TreeAction::OpenFile {
                    path,
                    label,
                    read_only,
                } => {
                    self.open_in_editor(&path, label, read_only, None);
                }
            }
        }

        let mut msg_clicked = None;
        egui::Panel::bottom("project_output")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                let report = self.last.as_ref().and_then(|o| o.report.as_ref());
                msg_clicked = output::show(
                    ui,
                    &mut self.output,
                    self.project.as_ref().ok(),
                    &self.messages,
                    &self.log,
                    report,
                    busy,
                    &self.paths,
                );
            });
        if let Some(loc) = msg_clicked {
            self.open_in_editor(&loc.path, loc.label, false, loc.line);
        }
        if let Some(clock) = self.output.critical_clock.take() {
            // The report belongs to the last build, which can be newer
            // than the design currently running in the chip tab.
            if self.last.as_ref().is_some_and(|o| o.ok) {
                self.emulate();
                self.app.critical_path(&clock);
            }
        }

        let key = format!(
            "phase={} file={} theme={} inline={}",
            self.editor_phase,
            self.editor
                .active_path()
                .map(|p| self.paths.path(p))
                .unwrap_or_else(|| "-".into()),
            if ui.visuals().dark_mode {
                "dark"
            } else {
                "light"
            },
            if self.editor.inline_values {
                "on"
            } else {
                "off"
            }
        );
        if key != self.editor_key {
            self.flush_editor_frames();
            self.editor_key = key;
        }
        let mut inline_shown = 0;
        let mut probe = None;
        self.app.begin_source_values();
        let built = self
            .app
            .ready()
            .and_then(|s| s.sources.get().and_then(|i| i.built_at));
        egui::CentralPanel::default().show(ui, |ui| {
            if let Err(err) = &self.project {
                if self.editor.active_path().is_none() {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        self.paths.text(&format!("Project failed to open:\n{err}")),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        "Enter a project folder or ecp5.toml on the left, or pick an example.",
                    );
                    if ui.button("Take the tour").clicked() {
                        match tour::TourProject::find() {
                            Ok(project) => {
                                self.out_override = None;
                                self.open_project(&project.root, &ctx);
                                if let Err(e) = self.start_tour(project) {
                                    self.status = format!("Tour: {e:#}");
                                }
                            }
                            Err(e) => self.status = format!("Tour: {e:#}"),
                        }
                    }
                    return;
                }
            }
            let mut values = SourceView {
                app: &mut self.app,
                project: self.project.as_ref().ok(),
                emulated_root: self.emulated_root.as_deref(),
                paths: &self.paths,
                built,
                modified: Default::default(),
            };
            probe = self
                .editor
                .show_values(ui, &self.paths, |probe, dirty, record| {
                    let view = values.value(probe, dirty, record);
                    if !record
                        && view.cycle.is_some()
                        && view.rows.iter().any(|row| !row.contains('?'))
                    {
                        inline_shown += 1;
                    }
                    view
                });
            if let Some(tour) = &mut self.tour {
                if self.editor.active_path() == Some(tour.project.root.join("top.v").as_path()) {
                    tour.editor_shown = true;
                }
            }
        });
        self.app.end_source_values();
        for note in self.editor.take_notes() {
            self.record(&note);
        }
        if !self.editor.watch_values.is_empty() {
            self.app
                .watch_source_targets(&std::mem::take(&mut self.editor.watch_values));
        }
        if let Some(probe) = probe {
            self.probe(probe);
        }
        if std::mem::take(&mut self.editor.watch_probe) {
            self.app.watch_probed();
        }
        self.poll_probes();
        self.tour_card(ui);
        if self.trace.is_some() && self.editor.active_path().is_some() {
            self.editor_samples.push((
                started.elapsed().as_secs_f64() * 1000.0,
                self.editor.layouter_ms,
                self.editor.laid_out_edited,
                inline_shown,
            ));
            if self.editor_samples.len() >= 600 {
                self.flush_editor_frames();
            }
        }
        self.new_file_ui(&ctx);
        self.memory_image_ui(&ctx);
        if self.typing.as_ref().is_some_and(|t| t.text.is_empty()) {
            self.typing = None;
            if let Some(runner) = &mut self.script {
                runner.complete_wait(Instant::now());
            }
            ctx.request_repaint();
        }
    }
}

struct SourceView<'a> {
    app: &'a mut App,
    project: Option<&'a Project>,
    emulated_root: Option<&'a Path>,
    paths: &'a DisplayPaths,
    built: Option<std::time::SystemTime>,
    modified: std::collections::HashMap<PathBuf, Option<std::time::SystemTime>>,
}

impl SourceView<'_> {
    fn value(&mut self, probe: &Probe, dirty: bool, record: bool) -> editor::ValueView {
        use editor::ValueView;
        if editor::Language::for_path(&probe.path) == editor::Language::Vhdl {
            let view = ValueView::unavailable("Live values are not available for VHDL yet");
            if record {
                let path = self
                    .project
                    .and_then(|p| probe.path.strip_prefix(&p.root).ok())
                    .unwrap_or(&probe.path);
                let file = self.paths.path(path);
                let note = self.paths.text(&view.note(&file, probe));
                self.app.source_value_note(note, &view);
            }
            return view;
        }
        let Some((project, relative)) = self.project.and_then(|p| {
            p.verilog
                .iter()
                .chain(&p.files)
                .find(|f| p.root.join(f) == probe.path)
                .map(|f| (p, f))
        }) else {
            return ValueView::unavailable("Values are available in project source files");
        };
        let file = relative.to_string_lossy();
        let mut view = if probe.identifier.is_empty() {
            ValueView::unavailable("No signal identifier here")
        } else if self.emulated_root == Some(project.root.as_path()) {
            self.app.source_value(&file, &probe.identifier)
        } else {
            ValueView::unavailable("Emulate this project to see values")
        };
        // One mtime lookup per requested file, outside Shared's lock.
        let mtime = *self.modified.entry(probe.path.clone()).or_insert_with(|| {
            std::fs::metadata(&probe.path)
                .and_then(|m| m.modified())
                .ok()
        });
        view.warning = source_warning(&file, dirty, mtime, self.built);
        view.rows = view.rows.iter().map(|s| self.paths.text(s)).collect();
        view.detail = view.detail.iter().map(|s| self.paths.text(s)).collect();
        if record {
            let note = self.paths.text(&view.note(&file, probe));
            self.app.source_value_note(note, &view);
        }
        view
    }
}

fn open_project_with_out(dir: &Path, out: Option<&Path>) -> anyhow::Result<Project> {
    let mut p = Project::open(dir)?;
    if let Some(out) = out {
        p.set_build_dir(out)?;
    }
    Ok(p)
}

fn source_warning(
    file: &str,
    dirty: bool,
    modified: Option<std::time::SystemTime>,
    built: Option<std::time::SystemTime>,
) -> Option<String> {
    (dirty
        || modified
            .zip(built)
            .is_some_and(|(source, built)| source > built))
    .then(|| format!("built from an older {file}: locations may be off"))
}

fn step_key(step: Step) -> &'static str {
    match step {
        Step::Synthesis => "synthesis",
        Step::PlaceRoute => "place_route",
        Step::Swap => "swap",
        Step::Bitstream => "bitstream",
    }
}

fn format_trace_message(project: Option<&Project>, msg: &Message) -> String {
    let loc = match (output::resolve_message_location(project, msg), msg.line) {
        (Some(l), Some(line)) => format!("{}:{line}", l.label),
        (Some(l), None) => format!("{}:0", l.label),
        (None, _) => "-".to_string(),
    };
    format!("message {} {} {}", msg.severity, loc, msg.text)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
#[path = "project/memory_image_tests.rs"]
mod memory_image_tests;

#[cfg(test)]
#[path = "project/new_file_tests.rs"]
mod new_file_tests;

#[cfg(test)]
#[path = "project/privacy_tests.rs"]
mod privacy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use output::{resolve_message_location, LogBuffer};
    use script::{parse_script, ScriptCommand, ViewTab};

    #[test]
    fn publication_kit_scripts_parse_without_opening_a_window() {
        let kit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/gui_scripts/v010");
        for name in [
            "shelf",
            "toolbar",
            "live_blinky",
            "live_life_full",
            "syntax",
            "new_project",
        ] {
            assert!(
                !crate::trace::script(&kit.join(format!("{name}.window.script")))
                    .unwrap()
                    .is_empty()
            );
            if !["shelf", "toolbar", "new_project"].contains(&name) {
                assert!(
                    !script::parse_script_file(&kit.join(format!("{name}.project.script")))
                        .unwrap()
                        .is_empty()
                );
            }
        }
        assert_eq!(
            language_label(editor::Language::for_path(&kit.join("hdl/sample.sv"))),
            "systemverilog"
        );
        assert_eq!(
            language_label(editor::Language::for_path(&kit.join("hdl/sample.vhd"))),
            "vhdl"
        );
        assert!(
            Project::open(kit.join("hdl")).is_ok(),
            "samples are a valid unbuilt project"
        );
        assert!(
            !kit.join("hdl/picorv32_plain.txt").exists(),
            "baseline is prepared outside the source tree"
        );
    }

    #[test]
    fn scripted_typing_inserts_one_character_per_frame_and_records_editor_costs() {
        let root = crate::test_temp::unique_dir("ecp5-typing-record-");
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "module top;\n// text\nendmodule\n").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
        let record = root.join("typing.project.tsv");
        window.trace = Some(ProjectTrace::new(&record).unwrap());
        window.open_relative(Path::new("top.v"), None);
        let window_script = root.join("window.script");
        std::fs::write(&window_script, "0 theme light\n0 size 1500 950\n0 pause\n").unwrap();
        let main_record = root.join("window.tsv");
        window.app.trace(&main_record).unwrap();
        window.app.script(&window_script).unwrap();
        let mut frame = eframe::Frame::_new_kittest();
        let mut draw = |window: &mut ProjectApp| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1500.0, 950.0),
                    )),
                    ..Default::default()
                },
                |ui| eframe::App::ui(window, ui, &mut frame),
            )
        };
        for _ in 0..3 {
            draw(&mut window);
        }
        window.script = Some(ScriptRunner::from_steps(
            parse_script("0 inline on\n0 type top.v 2 4 λab").unwrap(),
        ));
        draw(&mut window);
        assert_eq!(
            window
                .editor
                .text(&root.join("top.v"))
                .unwrap()
                .lines()
                .nth(1),
            Some("// text")
        );
        for expected in ["// λtext", "// λatext", "// λabtext"] {
            draw(&mut window);
            assert_eq!(
                window
                    .editor
                    .text(&root.join("top.v"))
                    .unwrap()
                    .lines()
                    .nth(1),
                Some(expected)
            );
        }
        assert!(window.typing.is_none());
        for _ in 0..3 {
            draw(&mut window);
        }
        drop(window);
        let main = std::fs::read_to_string(main_record).unwrap();
        assert!(
            main.contains("theme light")
                && main.contains("size 1500 950")
                && main.contains("pause"),
            "{main}"
        );
        let text = std::fs::read_to_string(record).unwrap();
        assert!(text.contains("theme=light"), "{text}");
        assert!(text.contains("language=verilog"), "{text}");
        assert!(text.contains("# inline on"), "{text}");
        assert!(text.contains("kind=typing n=3"), "{text}");
        assert!(text.contains("kind=idle n="), "{text}");
        assert!(text.contains("layout_p95_ms="), "{text}");
        assert_eq!(
            std::fs::read_to_string(root.join("top.v")).unwrap(),
            "module top;\n// text\nendmodule\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn emulation_window(prefix: &str) -> (PathBuf, ProjectApp) {
        let root = crate::test_temp::unique_dir(prefix).canonicalize().unwrap();
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "module top; endmodule").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
        window.app.reader_check =
            || crate::start::reader::fixed(crate::start::reader::Status::Building);
        (root, window)
    }

    #[test]
    fn cli_run_starts_a_build_and_queues_emulation() {
        let (root, window) = emulation_window("ecp5-cli-run-");
        drop(window);
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx);
        let window = ProjectApp::new(&cc, root.clone(), None, true, false).unwrap();
        assert!(
            window.is_building(),
            "--project DIR --run must spawn a build"
        );
        assert!(
            window.emulate_after,
            "successful build must open in the emulator"
        );
        let Some(ActiveTask::Build { cancel, done, .. }) = &window.active else {
            unreachable!()
        };
        cancel.store(true, Ordering::Relaxed);
        let result = done
            .recv_timeout(Duration::from_secs(10))
            .expect("build worker must respond");
        assert!(result.cancelled, "worker must reach the build pipeline");
        drop(window);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn emulate_blocker_requires_a_valid_emulatable_project() {
        let (root, mut window) = emulation_window("ecp5-emulate-invalid-");
        window.project.as_mut().unwrap().device.part = "LFE5U-45F".into();
        let reason = window.project.as_ref().unwrap().emulatable().unwrap_err();
        assert_eq!(window.emulate_blocker(), Some(reason.clone()));
        assert!(!window.emulate());
        assert_eq!(window.status, reason);
        window.project = Err("invalid project".into());
        assert_eq!(
            window.emulate_blocker().as_deref(),
            Some("Open a valid project first")
        );
        assert!(!window.emulate());
        assert_eq!(window.status, "Open a valid project first");
        assert!(window.chip_loading.is_none());
        drop(window);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn emulate_blocker_requires_both_outputs_without_a_session_outcome() {
        let (root, mut window) = emulation_window("ecp5-emulate-missing-");
        window.project.as_mut().unwrap().build_dir = root.join("custom build");
        let outputs = window.project.as_ref().unwrap().outputs();
        std::fs::create_dir(&window.project.as_ref().unwrap().build_dir).unwrap();
        for (config, names) in [(false, false), (true, false), (false, true), (true, true)] {
            for (path, present) in [(&outputs.config, config), (&outputs.routed_json, names)] {
                if present {
                    std::fs::write(path, "").unwrap();
                } else if path.is_file() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            assert_eq!(
                window.emulate_blocker().as_deref(),
                (!(config && names)).then_some("Build the project first"),
                "config={config}, names={names}"
            );
        }
        drop(window);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn emulate_blocker_uses_the_last_outcome_even_with_existing_outputs() {
        let (root, mut window) = emulation_window("ecp5-emulate-outcome-");
        let record = root.join("project.tsv");
        window.trace = Some(ProjectTrace::new(&record).unwrap());
        let outputs = window.project.as_ref().unwrap().outputs();
        window.last = Some(Outcome {
            ok: true,
            cancelled: false,
            steps: Vec::new(),
            messages: Vec::new(),
            report: None,
            outputs: outputs.clone(),
        });
        assert!(!outputs.config.exists());
        assert!(!outputs.routed_json.exists());
        assert_eq!(window.emulate_blocker(), None);
        window.last.as_mut().unwrap().ok = false;
        for existing_outputs in [false, true] {
            if existing_outputs {
                std::fs::create_dir(root.join("build")).unwrap();
                std::fs::write(&outputs.config, "").unwrap();
                std::fs::write(&outputs.routed_json, "{}").unwrap();
            }
            assert_eq!(
                window.emulate_blocker().as_deref(),
                Some("Last build did not succeed")
            );
            assert!(!window.emulate());
            assert_eq!(window.status, "Last build did not succeed");
        }
        let text = std::fs::read_to_string(&record).unwrap();
        assert_eq!(
            text.matches("# emulate blocked Last build did not succeed")
                .count(),
            2,
            "{text}"
        );
        assert!(!text.contains("# script error"), "{text}");
        window.script = Some(ScriptRunner::from_steps(
            parse_script("0 emulate\n0 view chip").unwrap(),
        ));
        window.poll_script(&egui::Context::default());
        assert_eq!(window.tab, ViewTab::Chip);
        assert_eq!(window.script.as_ref().unwrap().waiting(), None);
        assert!(window.chip_loading.is_none());
        drop(window);
        let text = std::fs::read_to_string(record).unwrap();
        let notes: Vec<_> = text
            .lines()
            .filter_map(|line| line.split_once('\t').map(|(_, note)| note))
            .filter(|note| {
                note.starts_with("# emulate blocked") || note.starts_with("# script error")
            })
            .collect();
        assert_eq!(
            notes,
            [
                "# emulate blocked Last build did not succeed",
                "# emulate blocked Last build did not succeed",
                "# emulate blocked Last build did not succeed",
                "# script error Last build did not succeed",
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn emulate_queues_a_running_build_without_outputs_or_after_failure() {
        let (root, mut window) = emulation_window("ecp5-emulate-queue-");
        let (_events_tx, events) = mpsc::channel();
        let (_done_tx, done) = mpsc::channel();
        window.active = Some(ActiveTask::Build {
            events,
            done,
            cancel: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            step_started: None,
            last_line: None,
        });
        assert_eq!(window.emulate_blocker(), None);
        assert!(window.emulate());
        assert!(window.emulate_after);
        assert!(window.chip_loading.is_none());
        window.last = Some(Outcome {
            ok: false,
            cancelled: false,
            steps: Vec::new(),
            messages: Vec::new(),
            report: None,
            outputs: window.project.as_ref().unwrap().outputs(),
        });
        window.emulate_after = false;
        assert_eq!(window.emulate_blocker(), None);
        window.script = Some(ScriptRunner::from_steps(parse_script("0 emulate").unwrap()));
        window.poll_script(&egui::Context::default());
        assert!(window.emulate_after);
        assert_eq!(
            window.script.as_ref().unwrap().waiting(),
            Some(WaitCondition::Emulate)
        );
        assert!(window.chip_loading.is_none());
        window.emulate_after = false;
        window.project.as_mut().unwrap().device.package = "CABGA756".into();
        let reason = window.project.as_ref().unwrap().emulatable().unwrap_err();
        assert_eq!(window.emulate_blocker(), Some(reason.clone()));
        assert!(!window.emulate());
        assert_eq!(window.status, reason);
        assert!(!window.emulate_after);
        window.project = Err("invalid project".into());
        assert_eq!(
            window.emulate_blocker().as_deref(),
            Some("Open a valid project first")
        );
        drop(window);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn emulate_reuses_existing_outputs_without_building_or_starting_native_loading() {
        let (root, mut window) = emulation_window("ecp5-emulate-existing-");
        std::fs::create_dir(root.join("build")).unwrap();
        let outputs = window.project.as_ref().unwrap().outputs();
        let built = std::time::UNIX_EPOCH + Duration::from_secs(1000);
        for (path, text) in [(&outputs.config, ""), (&outputs.routed_json, "{}")] {
            std::fs::write(path, text).unwrap();
            std::fs::File::open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(built))
                .unwrap();
        }
        let modified = std::fs::metadata(root.join("top.v"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(super::source_warning("top.v", false, Some(modified), Some(built)).is_some());
        assert!(window.last.is_none());
        assert_eq!(window.emulate_blocker(), None);
        assert!(window.emulate());
        assert!(window.chip_loading.is_some());
        assert_eq!(window.emulated_root.as_ref(), Some(&root));
        assert!(!window.is_building());
        drop(window);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vhdl_value_records_are_project_relative_with_injected_home_and_cwd() {
        let temp = crate::test_temp::unique_dir("ecp5-vhdl-record-")
            .canonicalize()
            .unwrap();
        let home = temp.join("private home");
        let root = home.join("work/project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx);
        let paths =
            DisplayPaths::with_environment(Some(home.join("work")), Some(home.clone()), None);
        let mut window =
            ProjectApp::new_with_paths(&cc, root.clone(), None, false, false, paths).unwrap();
        let main = temp.join("window.tsv");
        let project = temp.join("project.tsv");
        window.app.trace(&main).unwrap();
        window.trace = Some(ProjectTrace::new(&project).unwrap());
        for path in [
            root.join("top.vhd"),
            root.join("top.vhdl"),
            home.join("library/outside.vhd"),
        ] {
            let probe = Probe {
                path,
                line: 1,
                column: 8,
                identifier: "clk".into(),
            };
            let mut source = SourceView {
                app: &mut window.app,
                project: window.project.as_ref().ok(),
                emulated_root: None,
                paths: &window.paths,
                built: None,
                modified: Default::default(),
            };
            assert_eq!(
                source.value(&probe, false, true).detail,
                ["Live values are not available for VHDL yet"]
            );
        }
        window.poll_probes();
        drop(window);
        for record in [main, project] {
            let text = std::fs::read_to_string(record).unwrap();
            for file in ["top.vhd", "top.vhdl", "~/library/outside.vhd"] {
                assert!(
                    text.contains(&format!(
                        "# value {file}:1:8 clk -> Live values are not available for VHDL yet"
                    )),
                    "{text}"
                );
            }
            assert!(!text.contains(home.to_str().unwrap()), "{text}");
            assert!(!text.contains("project/top.vhd"), "{text}");
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn consecutive_script_hovers_record_each_current_buffer_result_once() {
        let root = std::env::temp_dir().join(format!("ecp5-hover-script-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "assign original = other;\n").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
        let record = root.join("project.tsv");
        window.trace = Some(ProjectTrace::new(&record).unwrap());
        window.script = Some(ScriptRunner::from_steps(parse_script(
            "0 edit top.v 1 assign edited_signal = other;\n0 hover top.v 1 10\n0 hover top.v 1 25"
        ).unwrap()));
        window.poll_script(&ctx);
        assert_eq!(
            window.editor.text(&root.join("top.v")),
            Some("assign edited_signal = other;\n")
        );
        assert!(!window.app.has_design());
        let mut frame = eframe::Frame::_new_kittest();
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                eframe::App::ui(&mut window, ui, &mut frame)
            });
        }
        drop(window);
        let text = std::fs::read_to_string(record).unwrap();
        let notes: Vec<_> = text.lines().filter(|s| s.contains("# value ")).collect();
        assert_eq!(
            notes.len(),
            2,
            "zero-delay commands are not overwritten and redraws do not duplicate: {text}"
        );
        assert!(notes[0]
            .contains("# value top.v:1:10 edited_signal -> Emulate this project to see values"));
        assert!(notes[1].contains("# value top.v:1:25 other -> Emulate this project to see values"));
        assert!(notes
            .iter()
            .all(|s| !s.contains(" = ") && !s.contains(" @ ")));
        assert_eq!(
            std::fs::read_to_string(root.join("top.v")).unwrap(),
            "assign original = other;\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_pointer_hover_records_once_in_both_traces_and_matches_script() {
        fn source_galley(
            shape: &egui::Shape,
            text: &str,
        ) -> Option<(egui::Pos2, Arc<egui::Galley>)> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some((t.pos, t.galley.clone())),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| source_galley(s, text)),
                _ => None,
            }
        }
        let root = crate::test_temp::unique_dir("ecp5-pointer-record-");
        let text = "assign original = other;\n";
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), text).unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let mut scripted_note = None;
        for (scripted, main_record) in [(true, true), (false, false), (false, true)] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
            let record = root.join(format!("project-{scripted}-{main_record}.tsv"));
            let main = root.join(format!("window-{scripted}-{main_record}.tsv"));
            window.trace = Some(ProjectTrace::new(&record).unwrap());
            if main_record {
                window.app.trace(&main).unwrap();
            }
            window.open_relative(Path::new("top.v"), None);
            let raw = |time| egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1800.0, 1200.0),
                )),
                ..Default::default()
            };
            let mut frame = eframe::Frame::_new_kittest();
            let mut galley = None;
            for time in [0.0, 0.1, 0.2] {
                let output = ctx.run_ui(raw(time), |ui| {
                    eframe::App::ui(&mut window, ui, &mut frame);
                });
                galley = output
                    .shapes
                    .iter()
                    .find_map(|s| source_galley(&s.shape, text));
            }
            let (origin, galley) = galley.expect("the real project editor draws its source buffer");
            let cursor = galley.pos_from_cursor(egui::text::CCursor::new(9));
            let pointer = origin + egui::vec2(cursor.min.x + 1.0, cursor.center().y);
            if scripted {
                window.script = Some(ScriptRunner::from_steps(
                    parse_script("0 hover top.v 1 10").unwrap(),
                ));
                window.poll_script(&ctx);
            }
            // Move only once, pass the tooltip delay, then redraw with the pointer at rest.
            for (i, time) in [0.3, 1.5, 2.0, 2.5, 3.0, 3.5].into_iter().enumerate() {
                let mut input = raw(time);
                if !scripted && i == 0 {
                    input.events.push(egui::Event::PointerMoved(pointer));
                }
                let _ = ctx.run_ui(input, |ui| {
                    eframe::App::ui(&mut window, ui, &mut frame);
                });
            }
            drop(window);
            let project_text = std::fs::read_to_string(record).unwrap();
            let notes: Vec<_> = project_text
                .lines()
                .filter_map(|s| s.split_once("# value ").map(|(_, n)| n))
                .collect();
            assert_eq!(
                notes.len(),
                1,
                "scripted={scripted} main={main_record}: {project_text}"
            );
            assert_eq!(
                notes[0],
                "top.v:1:10 original -> Emulate this project to see values"
            );
            if scripted {
                scripted_note = Some(notes[0].to_owned());
            } else {
                assert_eq!(Some(notes[0]), scripted_note.as_deref());
            }
            if main_record {
                let main_text = std::fs::read_to_string(main).unwrap();
                let main_notes: Vec<_> = main_text
                    .lines()
                    .filter_map(|s| s.strip_prefix("# value "))
                    .collect();
                assert_eq!(main_notes, notes, "{main_text}");
            }
        }
        assert_eq!(std::fs::read_to_string(root.join("top.v")).unwrap(), text);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_records_hide_paths_in_events_and_embedded_diagnostics() {
        let temp = crate::test_temp::unique_dir("ecp5-project-record-paths-");
        let home = temp.join("private home");
        let root = home.join("work/project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(home.join("shared")).unwrap();
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "assign original = other;\n").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        let paths =
            DisplayPaths::with_environment(Some(home.join("work")), Some(home.clone()), None);
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut window =
            ProjectApp::new_with_paths(&cc, root.clone(), None, false, false, paths).unwrap();
        let record = temp.join("project.tsv");
        window.trace = Some(ProjectTrace::new(&record).unwrap());
        window.app.reader_check =
            || crate::start::reader::fixed(crate::start::reader::Status::Building);
        let config = home.join("shared/top.config");
        window.open_emulation(config.clone(), config.with_file_name("top_routed.json"));
        let source = home.join("shared/tool.log");
        std::fs::write(&source, "tool output").unwrap();
        assert!(window.open_in_editor(&source, source.display().to_string(), true, None));
        for event in [
            "project",
            "chip error 0.01",
            "script",
            "message error -",
            "report",
            "timing",
            "# probe source",
            "# critical hop",
            "# tour failed",
        ] {
            window.record(&format!("{event} reading '{}'", source.display()));
        }
        drop(window);
        let text = std::fs::read_to_string(record).unwrap();
        assert!(text.contains("emulate ~/shared/top.config"), "{text}");
        assert!(text.contains("open ~/shared/tool.log:1"), "{text}");
        assert!(!text.contains("prebuilt/top.config"), "{text}");
        assert!(!text.contains(home.to_str().unwrap()), "{text}");
        assert!(!text.contains(temp.to_str().unwrap()), "{text}");
        for event in [
            "project",
            "chip error",
            "script",
            "message error",
            "report",
            "timing",
            "# probe source",
            "# critical hop",
            "# tour failed",
        ] {
            assert!(
                text.lines()
                    .any(|line| line.contains(event) && line.contains("~/shared/tool.log")),
                "{event}: {text}"
            );
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn source_locations_warn_for_newer_files_or_dirty_buffers() {
        let built = std::time::UNIX_EPOCH + Duration::from_secs(1000);
        assert!(super::source_warning("top.v", false, Some(built), Some(built)).is_none());
        assert_eq!(
            super::source_warning("top.v", true, Some(built), Some(built)).as_deref(),
            Some("built from an older top.v: locations may be off")
        );
        assert!(super::source_warning(
            "rtl/cpu.v",
            false,
            Some(built + Duration::from_secs(1)),
            Some(built)
        )
        .is_some());
        assert!(super::source_warning("top.v", false, None, Some(built)).is_none());
    }

    #[test]
    fn script_parser_commands_and_comments() {
        let input = "\
# leading comment
0 build
2.5 rebuild # inline comment
1 emulate
0.5 view chip
0 view project
1 open rtl/cpu.v 42
0 open top.lpf
0 click-message 3
0 edit top.v 12 assign led = 8'hAA;
0 save
1 quit
";
        let steps = parse_script(input).unwrap();
        assert_eq!(steps.len(), 11);
        assert_eq!(steps[0].delay_secs, 0.0);
        assert_eq!(steps[0].command, ScriptCommand::Build);
        assert_eq!(steps[1].delay_secs, 2.5);
        assert_eq!(steps[1].command, ScriptCommand::Rebuild);
        assert_eq!(steps[2].command, ScriptCommand::Emulate);
        assert_eq!(steps[3].command, ScriptCommand::View(ViewTab::Chip));
        assert_eq!(steps[4].command, ScriptCommand::View(ViewTab::Project));
        assert_eq!(
            steps[5].command,
            ScriptCommand::Open {
                file: PathBuf::from("rtl/cpu.v"),
                line: Some(42)
            }
        );
        assert_eq!(
            steps[6].command,
            ScriptCommand::Open {
                file: PathBuf::from("top.lpf"),
                line: None
            }
        );
        assert_eq!(steps[7].command, ScriptCommand::ClickMessage(3));
        assert_eq!(
            steps[8].command,
            ScriptCommand::Edit {
                file: PathBuf::from("top.v"),
                line: 12,
                text: "assign led = 8'hAA;".to_string()
            }
        );
        assert_eq!(steps[9].command, ScriptCommand::Save);
        assert_eq!(steps[10].command, ScriptCommand::Quit);
    }

    #[test]
    fn script_parser_rejects_bad_lines() {
        assert!(parse_script("-1 build").is_err());
        assert!(parse_script("0 unknown").is_err());
        assert_eq!(
            parse_script("0 load").unwrap_err().to_string(),
            "line 1: unknown script command \"load\""
        );
        assert!(parse_script("0 view nowhere").is_err());
        assert!(parse_script("0 open").is_err());
    }

    #[test]
    fn message_location_maps_to_project_file() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/ulx3s/blinky_cycle");
        let project = Project::open(&example).unwrap();
        let msg = Message {
            kind: Default::default(),
            severity: Severity::Error,
            step: Step::Synthesis,
            text: "syntax error".into(),
            file: Some(project.root.join("top.v")),
            line: Some(7),
            column: Some(3),
            count: 1,
        };
        let loc = resolve_message_location(Some(&project), &msg).unwrap();
        assert_eq!(loc.path, project.root.join("top.v"));
        assert_eq!(
            loc.label,
            DisplayPaths::new(Some(&project.root)).path(&project.root.join("top.v"))
        );
        assert_eq!(loc.line, Some(7));

        let no_file = Message { file: None, ..msg };
        assert!(resolve_message_location(Some(&project), &no_file).is_none());
    }

    #[test]
    fn log_buffer_enforces_cap() {
        let mut buf = LogBuffer::with_cap(3);
        for i in 0..5 {
            buf.push(LogSource::Step(Step::Synthesis), format!("line {i}"));
        }
        assert_eq!(buf.len(), 3);
        let texts: Vec<_> = buf.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["line 2", "line 3", "line 4"]);
        assert_eq!(output::LOG_CAP, 20_000);
    }
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;
    use crate::app::toolbar::tests::{assert_layout, painted, snapshot};

    #[test]
    fn toolbar_project_both_tabs_widths_themes_states_and_emulate_blocker() {
        let root = crate::test_temp::unique_dir("ecp5-project-toolbar-");
        std::fs::write(
            root.join("ecp5.toml"),
            "[project]\nname = \"a_very_long_project_name_that_must_truncate\"\n",
        )
        .unwrap();
        std::fs::write(root.join("top.v"), "module top; endmodule").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
            crate::theme::install(&ctx, theme);
            for width in [1500.0, 1499.0, 1400.0, 1200.0, 1100.0] {
                let size = egui::vec2(width, if width == 1500.0 { 950.0 } else { 800.0 });
                for tab in [ViewTab::Project, ViewTab::Chip] {
                    window.tab = tab;
                    for (running, cycles, ahead) in [
                        (false, 1, 0),
                        (false, 1_234_567, 240),
                        (true, u64::MAX, u64::MAX),
                    ] {
                        let mut s = snapshot();
                        s.running = running;
                        s.cycles = cycles;
                        s.ahead = ahead;
                        window.app.toolbar_test_snapshot(s);
                        let mut output = None;
                        for _ in 0..3 {
                            output = Some(ctx.run_ui(
                                egui::RawInput {
                                    screen_rect: Some(egui::Rect::from_min_size(
                                        egui::Pos2::ZERO,
                                        size,
                                    )),
                                    ..Default::default()
                                },
                                |ui| {
                                    eframe::App::ui(
                                        &mut window,
                                        ui,
                                        &mut eframe::Frame::_new_kittest(),
                                    )
                                },
                            ));
                        }
                        let (layout, controls) = window.app.toolbar_test_evidence();
                        assert_layout(layout, controls, size, true);
                        let enabled = controls.iter().find(|c| c.0 == "Emulate").unwrap().2;
                        assert_eq!(enabled, window.emulate_blocker().is_none());
                        let mut texts = Vec::new();
                        for shape in output.unwrap().shapes {
                            if let egui::Shape::Text(text) = &shape.shape {
                                if text.pos.y < 46.0
                                    && (text.galley.text() == "New project"
                                        || (width == 1500.0
                                            && [
                                                "Values",
                                                "Wires",
                                                "Tiles",
                                                "Waves",
                                                "Critical path",
                                            ]
                                            .contains(&text.galley.text())))
                                {
                                    assert!(
                                        !text.galley.elided,
                                        "{} must keep its label",
                                        text.galley.text()
                                    );
                                }
                            }
                            let mut text = Vec::new();
                            painted(&shape.shape, &mut text);
                            text.retain(|(_, rect)| rect.center().y <= 46.0);
                            for (_, rect) in &text {
                                assert!(
                                    shape.clip_rect.expand(0.5).contains_rect(*rect),
                                    "clipped text: {text:?}; clip {:?}",
                                    shape.clip_rect
                                );
                            }
                            texts.extend(text);
                        }
                        assert_eq!(
                            texts.iter().filter(|t| t.0.starts_with('+')).count(),
                            usize::from(ahead > 0)
                        );
                        assert_eq!(
                            texts
                                .iter()
                                .filter(|t| t.0.contains("a_very_long_project"))
                                .count(),
                            1,
                            "{texts:?}"
                        );
                    }
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    fn frame(
        window: &mut ProjectApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 950.0),
                )),
                events,
                ..Default::default()
            },
            |ui| eframe::App::ui(window, ui, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn text_point(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        let mut texts = Vec::new();
        for shape in &output.shapes {
            painted(&shape.shape, &mut texts);
        }
        texts
            .iter()
            .find(|t| t.0 == label)
            .unwrap_or_else(|| panic!("missing {label}: {texts:?}"))
            .1
            .center()
    }
    fn click(window: &mut ProjectApp, ctx: &egui::Context, width: f32, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                window,
                ctx,
                width,
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
        for _ in 0..3 {
            frame(window, ctx, width, vec![]);
        }
    }
    fn fixture(root: &Path, ctx: &egui::Context) -> ProjectApp {
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "module top; endmodule").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        ProjectApp::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            root.into(),
            None,
            false,
            false,
        )
        .unwrap()
    }
    #[test]
    fn toolbar_new_project_button_dispatches_on_both_project_tabs() {
        let root = crate::test_temp::unique_dir("toolbar-project-creation-");
        for width in [1500.0, 1100.0] {
            for tab in [ViewTab::Project, ViewTab::Chip] {
                let ctx = egui::Context::default();
                let mut window = fixture(&root, &ctx);
                window.tab = tab;
                window.app.creation_available = false;
                for _ in 0..3 {
                    frame(&mut window, &ctx, width, vec![]);
                }
                let control = window
                    .app
                    .toolbar_test_evidence()
                    .1
                    .iter()
                    .find(|c| c.0 == "New project")
                    .unwrap()
                    .clone();
                assert!(!control.2);
                click(&mut window, &ctx, width, control.1.center());
                assert!(window.app.creation_command.is_none());
                window.app.creation_available = true;
                for _ in 0..3 {
                    frame(&mut window, &ctx, width, vec![]);
                }
                let control = window
                    .app
                    .toolbar_test_evidence()
                    .1
                    .iter()
                    .find(|c| c.0 == "New project")
                    .unwrap()
                    .clone();
                assert!(control.2);
                click(&mut window, &ctx, width, control.1.center());
                assert!(matches!(
                    window.app.creation_command,
                    Some(crate::new_project::Command::New(_))
                ));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn toolbar_project_records_lpf_insert_notes_once_after_the_editor() {
        let root = crate::test_temp::unique_dir("toolbar-lpf-notes-");
        let ctx = egui::Context::default();
        let mut window = fixture(&root, &ctx);
        let path = root.join("project.tsv");
        window.trace = Some(ProjectTrace::new(&path).unwrap());
        window.open_relative(Path::new("top.lpf"), None);
        for _ in 0..3 {
            frame(&mut window, &ctx, 1500.0, vec![]);
        }
        let output = frame(&mut window, &ctx, 1500.0, vec![]);
        click(
            &mut window,
            &ctx,
            1500.0,
            text_point(&output, "Insert pin…"),
        );
        let output = frame(&mut window, &ctx, 1500.0, vec![]);
        click(&mut window, &ctx, 1500.0, text_point(&output, "Insert"));
        assert!(window
            .editor
            .text(&root.join("top.lpf"))
            .unwrap()
            .contains("LOCATE COMP"));
        assert!(window.editor.take_notes().is_empty());
        drop(window);
        let record = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            record
                .lines()
                .filter(|line| line.ends_with("# lpf insert clock clk_25mhz 3"))
                .count(),
            1,
            "{record}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
