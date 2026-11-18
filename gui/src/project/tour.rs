//! A first run driven by observed window state and the visitor's Next button.
//! Evidence waits and reading have no deadline; only Stop or finishing ends a tour.
//!
//! Record notes keep `# tour N STEP CAPTION`, `# tour done SECONDS`,
//! `# tour stopped at STEP`, `# tour failed STEP REASON` and
//! `# tour skip build REASON`. Pressing Next adds `# tour next N`;
//! requesting a demonstration adds `# tour show-me N`.
use crate::start::reader::{self, Check, Status};
use anyhow::{bail, Context, Result};
use eframe::egui::{self, Rect, RichText};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Build,
    Chip,
    Run,
    Source,
    Back,
    Waves,
    Done,
}

impl Step {
    pub const ALL: [Self; 7] = [
        Self::Build,
        Self::Chip,
        Self::Run,
        Self::Source,
        Self::Back,
        Self::Waves,
        Self::Done,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Chip => "chip",
            Self::Run => "run",
            Self::Source => "source",
            Self::Back => "back",
            Self::Waves => "waves",
            Self::Done => "done",
        }
    }

    pub fn number(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap().min(5) + 1
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Build => "Building blinky_cycle: yosys synthesizes the Verilog, nextpnr places and routes it on the LFE5U-85F",
            Self::Chip => "Click Emulate to open the chip and see the design's wires, or press Show me",
            Self::Run => "Running at 4 cycles per second: a wire glows when its value changes",
            Self::Source => "Click Project, open top.v, then click count in count <= count + 1 to light its nets, or press Show me",
            Self::Back => "Right-click this lit wire → Show in source",
            Self::Waves => "Click Chip, then Watch in Waves to see count's bits after every clock edge, or press Show me",
            Self::Done => "That's the tour. You are in control: open your own project or a demo",
        }
    }

    pub fn needs_gesture(self) -> bool {
        matches!(self, Self::Chip | Self::Source | Self::Back | Self::Waves)
    }

    fn waiting(self) -> &'static str {
        match self {
            Self::Build => "Waiting for the build to succeed",
            Self::Chip => "Waiting for the ready chip's die to be drawn",
            Self::Run => "Waiting for running at 4 cycles/s and changes drawn lit",
            Self::Source => "Waiting for the editor probe to find nets",
            Self::Back => "Waiting for Show in source to open the editor",
            Self::Waves => "Waiting for Waves rows to be drawn",
            Self::Done => "The tour is complete",
        }
    }
}

pub const SKIP: &str =
    "The FPGA toolchain is not installed here: opening the prebuilt design instead";
pub const CHIP_READY: &str = "The chip: the design's wires on the ECP5's die";

#[derive(Default)]
pub struct Evidence {
    pub build: Option<Result<(), String>>,
    pub missing_tool: bool,
    pub chip_ready: bool,
    pub chip: bool,
    pub running_at_four: bool,
    pub changes_drawn: bool,
    pub source_probed: bool,
    pub source_jumped: bool,
    pub waves: bool,
}

pub struct Tour {
    pub step: Step,
    pub prebuilt: bool,
    since: f64,
    started: f64,
    skip_since: Option<f64>,
    chip_ready: bool,
    ready: bool,
    entry: bool,
    terminal: Option<(f64, String)>,
    failure: Option<String>,
    notes: Vec<String>,
    reader: Option<Check>,
    reader_caption: Option<String>,
}

impl Tour {
    pub fn new(now: f64) -> Self {
        let mut tour = Self {
            step: Step::Build,
            prebuilt: false,
            since: now,
            started: now,
            skip_since: None,
            chip_ready: false,
            ready: false,
            entry: true,
            terminal: None,
            failure: None,
            notes: Vec::new(),
            reader: None,
            reader_caption: None,
        };
        tour.note_step();
        tour
    }

    fn with_reader(now: f64, check: Check) -> Self {
        let mut tour = Self::new(now);
        tour.notes.clear();
        tour.entry = false;
        if check.build_requested {
            tour.notes.push("tour reader building".into());
        }
        tour.reader = Some(check);
        tour.poll_reader(now);
        tour
    }

    fn poll_reader(&mut self, now: f64) {
        let Some(check) = &mut self.reader else {
            return;
        };
        self.notes.extend(check.notes());
        match check.status() {
            Status::Building => self.reader_caption = Some(reader::BUILDING.into()),
            Status::Missing(reason) => {
                if self.failure.as_deref() != Some(&reason) {
                    self.notes.push(format!("tour failed reader {reason}"));
                    self.failure = Some(reason.clone());
                }
                self.reader_caption = Some(format!("Waiting for the image reader: {reason}"));
            }
            Status::Ready => {
                self.reader = None;
                self.reader_caption = None;
                self.failure = None;
                self.since = now;
                self.started = now;
                self.entry = true;
                self.note_step();
            }
        }
    }

    pub fn active(&self) -> bool {
        self.terminal.is_none() && self.step != Step::Done
    }

    pub fn caption(&self) -> &str {
        if let Some(caption) = &self.reader_caption {
            caption
        } else if let Some((_, text)) = &self.terminal {
            text
        } else if let Some(reason) = &self.failure {
            reason
        } else if self.skip_since.is_some() {
            SKIP
        } else if self.step == Step::Chip && self.chip_ready {
            CHIP_READY
        } else {
            self.step.caption()
        }
    }

    pub fn visible(&self, now: f64) -> bool {
        if self.reader_caption.is_some() {
            true
        } else if let Some((at, _)) = self.terminal {
            now - at < 2.0
        } else {
            self.step != Step::Done || now - self.since < 4.0
        }
    }

    pub fn take_entry(&mut self) -> Option<Step> {
        std::mem::take(&mut self.entry).then_some(self.step)
    }

    fn note_step(&mut self) {
        self.notes.push(format!(
            "tour {} {} {}",
            self.step.number(),
            self.step.name(),
            self.caption()
        ));
    }

    pub fn notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    pub fn stop(&mut self, now: f64) {
        if self.active() {
            let name = if self.reader.is_some() {
                "reader"
            } else {
                self.step.name()
            };
            self.notes.push(format!("tour stopped at {name}"));
            self.terminal = Some((now, "Tour stopped: you are in control".into()));
            self.entry = false;
            self.reader = None;
            self.reader_caption = None;
        }
    }

    pub fn fail(&mut self, _now: f64, reason: &str) {
        if self.active() && self.failure.as_deref() != Some(reason) {
            self.notes
                .push(format!("tour failed {} {reason}", self.step.name()));
            self.failure = Some(reason.into());
            self.ready = false;
            self.entry = false;
        }
    }

    pub fn observe(&mut self, now: f64, seen: &Evidence) {
        if self.reader.is_some() {
            self.poll_reader(now);
            return;
        }
        if !self.active() {
            return;
        }
        let success = match self.step {
            Step::Build => match &seen.build {
                Some(Ok(())) => true,
                Some(Err(reason)) if seen.missing_tool => {
                    if self.skip_since.is_none() {
                        self.prebuilt = true;
                        self.skip_since = Some(now);
                        self.notes.push(format!("tour skip build {reason}"));
                        self.notes.push(format!("tour 1 build {SKIP}"));
                    }
                    true
                }
                Some(Err(reason)) => {
                    self.fail(now, reason);
                    return;
                }
                None => false,
            },
            Step::Chip => seen.chip,
            Step::Run => seen.running_at_four && seen.changes_drawn,
            Step::Source => seen.source_probed,
            Step::Back => seen.source_jumped,
            Step::Waves => seen.waves,
            Step::Done => false,
        };
        // Evidence is an observed event, not a condition the visitor must
        // keep true throughout an arbitrarily long wait for Next.
        self.ready |= success;
        let chip_caption_changed = self.step == Step::Chip && self.chip_ready != seen.chip_ready;
        self.chip_ready = seen.chip_ready;
        if chip_caption_changed {
            self.notes.push(format!(
                "tour caption {} {}",
                self.step.number(),
                self.caption()
            ));
        }
        if success {
            self.failure = None;
        }
    }

    pub fn next_enabled(&self) -> bool {
        self.active() && self.reader.is_none() && self.ready
    }

    pub fn waiting(&self) -> Option<&str> {
        if self.next_enabled() {
            None
        } else if self.reader.is_some() {
            Some(self.caption())
        } else {
            Some(self.step.waiting())
        }
    }

    pub fn show_me(&mut self) -> bool {
        if !self.active() || self.reader.is_some() || !self.step.needs_gesture() {
            return false;
        }
        self.notes
            .push(format!("tour show-me {}", self.step.number()));
        true
    }

    pub fn next(&mut self, now: f64) {
        if !self.next_enabled() {
            return;
        }
        self.notes.push(format!("tour next {}", self.step.number()));
        self.step = Step::ALL[self.step.number()];
        // Emulate opens a new design after a successful build. A skipped
        // build has already opened the prebuilt, which may now be ready.
        self.chip_ready = self.step == Step::Chip && self.prebuilt && self.chip_ready;
        self.since = now;
        self.skip_since = None;
        self.failure = None;
        self.ready = false;
        self.entry = true;
        self.note_step();
        if self.step == Step::Done {
            self.notes
                .push(format!("tour done {:.2}", now - self.started));
        }
    }
}

/// Search the assignment, not its declaration or a fixed line number.
pub fn counter_location(text: &str) -> Option<(u32, u32)> {
    for (line, text) in text.lines().enumerate() {
        let code = text.split("//").next()?;
        if let Some(at) = code.find("count <= count + 1") {
            return Some((line as u32 + 1, code[..at].chars().count() as u32 + 1));
        }
    }
    None
}

#[cfg(test)]
pub fn next_button_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp(egui::Id::new("tour_next_rect")))
}

/// A fixed card, over unused lower space in the editor or away from a map target.
pub fn card(
    ui: &mut egui::Ui,
    area: Rect,
    text: &str,
    step: Step,
    running: bool,
    target: Option<egui::Pos2>,
    waiting: Option<&str>,
) -> (Rect, bool, bool, bool) {
    let size = egui::vec2(400.0, 220.0);
    let mut rect = Rect::from_min_size(
        egui::pos2(
            (area.right() - size.x - 14.0).max(area.left()),
            (area.bottom() - size.y - 14.0).max(area.top()),
        ),
        size,
    );
    if target.is_some_and(|p| rect.expand(24.0).contains(p)) {
        rect = Rect::from_min_size(area.left_top() + egui::vec2(14.0, 14.0), size);
    }
    // An Area registers this foreground card for hit testing and tooltips;
    // a child UI with only a foreground layer paints but cannot receive input.
    let response = egui::Area::new(egui::Id::new("tour_caption_area"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .movable(false)
        .constrain(false)
        .default_size(rect.size())
        .show(ui.ctx(), |ui| {
            ui.set_min_size(rect.size());
            let mut card = crate::app::loading::card_surface(ui, "tour_caption", rect);
            let inner = rect.shrink(14.0);
            let colours = crate::theme::colours(card.visuals().dark_mode);
            card.put(
                Rect::from_min_size(inner.min, egui::vec2(270.0, 22.0)),
                egui::Label::new(
                    RichText::new(if reader::is_caption(text) {
                        "Take the tour · image reader".into()
                    } else {
                        format!("Take the tour · step {} of 6", step.number())
                    })
                    .small()
                    .color(colours.tour),
                ),
            );
            let stopped = running
                && card
                    .put(
                        Rect::from_min_size(
                            egui::pos2(inner.right() - 58.0, inner.top()),
                            egui::vec2(58.0, 22.0),
                        ),
                        egui::Button::new("Stop"),
                    )
                    .clicked();
            card.put(
                Rect::from_min_size(
                    inner.min + egui::vec2(0.0, 32.0),
                    egui::vec2(inner.width(), 88.0),
                ),
                egui::Label::new(RichText::new(text).size(14.0)).wrap(),
            );
            if running {
                card.put(
                    Rect::from_min_size(
                        inner.min + egui::vec2(0.0, 120.0),
                        egui::vec2(inner.width(), 40.0),
                    ),
                    egui::Label::new(format!(
                        "{}. Stop anytime.",
                        waiting.unwrap_or("Evidence seen; press Next when ready")
                    ))
                    .wrap(),
                );
            }
            let next_rect = Rect::from_min_size(
                egui::pos2(inner.right() - 58.0, inner.bottom() - 22.0),
                egui::vec2(58.0, 22.0),
            );
            card.ctx().data_mut(|data| {
                let id = egui::Id::new("tour_next_rect");
                if running {
                    data.insert_temp(id, next_rect);
                } else {
                    data.remove::<Rect>(id);
                }
            });
            let next = running
                && card
                    .scope_builder(egui::UiBuilder::new().max_rect(next_rect), |ui| {
                        if waiting.is_some() {
                            ui.disable();
                        }
                        let response = ui.put(next_rect, egui::Button::new("Next"));
                        if let Some(reason) = waiting {
                            response.on_disabled_hover_text(reason).clicked()
                        } else {
                            response.clicked()
                        }
                    })
                    .inner;
            let show_rect = Rect::from_min_size(
                egui::pos2(inner.right() - 154.0, next_rect.top()),
                egui::vec2(86.0, 22.0),
            );
            let can_show = running && step.needs_gesture() && !reader::is_caption(text);
            card.ctx().data_mut(|data| {
                let id = egui::Id::new("tour_show_me_rect");
                if can_show {
                    data.insert_temp(id, show_rect);
                } else {
                    data.remove::<Rect>(id);
                }
            });
            let show_me = can_show && card.put(show_rect, egui::Button::new("Show me")).clicked();
            if running {
                card.put(
                    Rect::from_min_size(
                        egui::pos2(inner.left(), next_rect.top()),
                        egui::vec2(
                            if can_show {
                                200.0
                            } else {
                                inner.width() - 70.0
                            },
                            22.0,
                        ),
                    ),
                    egui::Label::new(if can_show {
                        "Next when ready, or Show me."
                    } else {
                        "Look around, then press Next."
                    })
                    .selectable(false),
                );
            }
            (stopped, next, show_me)
        });
    let (stopped, next, show_me) = response.inner;
    (rect, stopped, next, show_me)
}

/// A demonstration only runs after Show me. Stages are paced by the tour
/// clock; pointer events go through the same widgets as a physical gesture.
pub struct Demo {
    pub phase: usize,
    pub since: f64,
    pub pointer: Option<egui::Pos2>,
    pub events: Vec<egui::Event>,
}

impl Demo {
    pub fn new(now: f64) -> Self {
        Self {
            phase: 0,
            since: now,
            pointer: None,
            events: Vec::new(),
        }
    }

    pub fn point(&mut self, point: egui::Pos2) {
        self.pointer = Some(point);
        self.events.push(egui::Event::PointerMoved(point));
    }

    pub fn button(&mut self, button: egui::PointerButton, pressed: bool) {
        if let Some(pos) = self.pointer {
            self.events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    pub fn advance(&mut self, now: f64) {
        self.phase += 1;
        self.since = now;
    }
}

/// Find the labels actually painted by the normal toolbar, editor and menus.
/// This keeps the tour out of those controls' implementation and follows
/// their layout in compact windows and both themes.
pub fn labelled_rects(ctx: &egui::Context, matches: impl Fn(&str) -> bool) -> Vec<Rect> {
    text_rects(ctx, |text| {
        matches(text.galley.text())
            .then(|| Rect::from_min_size(text.pos, text.galley.size()).expand(4.0))
    })
}

pub fn counter_point(ctx: &egui::Context) -> Option<egui::Pos2> {
    text_rects(ctx, |text| {
        let source = text.galley.text();
        let byte = source.find("count <= count + 1")?;
        let start = source[..byte].chars().count();
        let a = text.galley.pos_from_cursor(egui::text::CCursor::new(start));
        let b = text
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start + 5));
        Some(Rect::from_min_max(a.min, b.max).translate(text.pos.to_vec2()))
    })
    .first()
    .map(Rect::center)
}

fn text_rects(
    ctx: &egui::Context,
    find: impl Fn(&egui::epaint::TextShape) -> Option<Rect>,
) -> Vec<Rect> {
    fn collect(
        shape: &egui::Shape,
        find: &impl Fn(&egui::epaint::TextShape) -> Option<Rect>,
        rects: &mut Vec<Rect>,
    ) {
        match shape {
            egui::Shape::Text(text) => {
                if let Some(rect) = find(text) {
                    rects.push(rect);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, find, rects);
                }
            }
            _ => {}
        }
    }
    let layers = ctx.memory(|m| m.areas().visible_layer_ids());
    let mut rects = Vec::new();
    ctx.graphics(|graphics| {
        for layer in layers {
            if let Some(shapes) = graphics.get(layer) {
                for shape in shapes.all_entries() {
                    collect(&shape.shape, &find, &mut rects);
                }
            }
        }
    });
    rects
}

#[derive(Clone, Debug)]
pub struct TourProject {
    pub root: PathBuf,
    pub prebuilt: PathBuf,
}

pub struct Session {
    pub machine: Tour,
    pub project: TourProject,
    pub clock: std::time::Instant,
    #[cfg(test)]
    frozen_seconds: Option<f64>,
    pub lit_before: u64,
    pub editor_shown: bool,
    pub jumped: bool,
    pub target: Option<(egui::Pos2, f64)>,
    pub jump_requested: bool,
    pub prebuilt_opened: bool,
    pub demo: Option<Demo>,
    pub menu: Option<Rect>,
}

impl Session {
    pub fn new(project: TourProject) -> Self {
        Self::with_reader(project, Check::new())
    }

    pub fn with_reader(project: TourProject, reader: Check) -> Self {
        Self {
            machine: Tour::with_reader(0.0, reader),
            project,
            clock: std::time::Instant::now(),
            #[cfg(test)]
            frozen_seconds: None,
            lit_before: 0,
            editor_shown: false,
            jumped: false,
            target: None,
            jump_requested: false,
            prebuilt_opened: false,
            demo: None,
            menu: None,
        }
    }

    pub fn seconds(&self) -> f64 {
        #[cfg(test)]
        if let Some(seconds) = self.frozen_seconds {
            return seconds;
        }
        self.clock.elapsed().as_secs_f64()
    }

    #[cfg(test)]
    pub(super) fn freeze_at(&mut self, seconds: f64) {
        self.frozen_seconds = Some(seconds);
    }

    pub fn relative(&self, text: &str) -> String {
        text.replace(&self.project.prebuilt.display().to_string(), "prebuilt")
            .replace(&self.project.root.display().to_string(), ".")
            .replace(
                &crate::start::repository_root().display().to_string(),
                "checkout",
            )
    }
}

impl TourProject {
    pub fn find() -> Result<Self> {
        let config =
            crate::start::settings_path().context("could not find the configuration folder")?;
        Self::locate(
            std::env::var_os("ECP5_TOUR").map(PathBuf::from).as_deref(),
            std::env::var_os("ECP5_ARCHITECTURE")
                .map(PathBuf::from)
                .as_deref(),
            &std::env::current_exe()?,
            &crate::start::repository_root(),
            config.parent().unwrap(),
        )
    }

    pub(super) fn locate(
        env: Option<&Path>,
        architecture: Option<&Path>,
        executable: &Path,
        repo: &Path,
        config: &Path,
    ) -> Result<Self> {
        let beside = executable.parent().context("executable has no directory")?;
        let candidates = env.map_or_else(
            || {
                architecture
                    .and_then(Path::parent)
                    .map(|parent| parent.join("tour/blinky_cycle"))
                    .into_iter()
                    .chain([
                        beside.join("../Resources/tour/blinky_cycle"),
                        beside.join("../share/tour/blinky_cycle"),
                        repo.join("examples/ulx3s/blinky_cycle"),
                    ])
                    .collect()
            },
            |path| vec![path.to_path_buf()],
        );
        let root = candidates
            .into_iter()
            .find(|p| sources_exist(p))
            .context("tour project not found (set ECP5_TOUR to its folder)")?;
        let root = root.canonicalize()?;
        let prebuilt = if root.join("prebuilt/top.config").is_file() {
            root.join("prebuilt")
        } else {
            repo.join("target/reference/blinky_cycle")
        };
        let project = Self { root, prebuilt };
        project.writable_copy(config)
    }

    fn writable_copy(self, config: &Path) -> Result<Self> {
        let probe = self
            .root
            .join(format!(".tour-write-{}", std::process::id()));
        let writable = !fs::metadata(&self.root)?.permissions().readonly()
            && fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)
                .is_ok();
        if writable {
            fs::remove_file(probe)?;
            return Ok(self);
        }
        let root = config.join("tour/blinky_cycle");
        if !sources_exist(&root) {
            let temporary = config.join(format!("tour/.blinky_cycle-{}", std::process::id()));
            fs::create_dir_all(&temporary)?;
            for name in ["ecp5.toml", "top.v", "top.lpf"] {
                let target = temporary.join(name);
                fs::copy(self.root.join(name), &target)?;
                make_writable(&target)?;
            }
            fs::create_dir_all(temporary.join("prebuilt"))?;
            for name in ["top.config", "top_routed.json"] {
                fs::copy(
                    self.prebuilt.join(name),
                    temporary.join("prebuilt").join(name),
                )
                .with_context(|| format!("copying tour prebuilt/{name}"))?;
            }
            fs::rename(temporary, &root)?;
        }
        let root = root.canonicalize()?;
        Ok(Self {
            prebuilt: root.join("prebuilt"),
            root,
        })
    }

    pub fn check_prebuilt(&self) -> Result<()> {
        if ["top.config", "top_routed.json"]
            .iter()
            .all(|name| self.prebuilt.join(name).is_file())
        {
            Ok(())
        } else {
            bail!("the tour's prebuilt/top.config or prebuilt/top_routed.json is missing")
        }
    }
}

fn sources_exist(root: &Path) -> bool {
    ["ecp5.toml", "top.v", "top.lpf"]
        .iter()
        .all(|name| root.join(name).is_file())
}

fn make_writable(path: &Path) -> std::io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_events() -> Evidence {
        Evidence {
            build: Some(Ok(())),
            missing_tool: false,
            chip_ready: true,
            chip: true,
            running_at_four: true,
            changes_drawn: true,
            source_probed: true,
            source_jumped: true,
            waves: true,
        }
    }

    fn at(step: Step) -> Tour {
        let mut tour = Tour::new(0.0);
        tour.step = step;
        tour.notes.clear();
        tour
    }

    #[test]
    fn next_advances_in_order_with_the_real_captions_and_notes() {
        let mut tour = Tour::new(0.0);
        let mut now = 0.0;
        let mut observed = Vec::new();
        let mut expected = Vec::new();
        for step in Step::ALL {
            assert_eq!(tour.step, step);
            assert_eq!(tour.take_entry(), Some(step));
            assert_eq!(tour.take_entry(), None);
            assert_eq!(tour.caption(), step.caption());
            observed.extend(tour.notes());
            expected.push(format!(
                "tour {} {} {}",
                step.number(),
                step.name(),
                step.caption()
            ));
            if step == Step::Done {
                break;
            }
            assert!(!tour.next_enabled());
            tour.observe(now, &all_events());
            if step == Step::Chip {
                expected.push(format!("tour caption 2 {CHIP_READY}"));
            }
            assert!(tour.next_enabled());
            assert_eq!(tour.step, step, "evidence alone never advances");
            now += 1.0;
            tour.next(now);
            expected.push(format!("tour next {}", step.number()));
        }
        expected.push(format!("tour done {now:.2}"));
        assert_eq!(observed, expected);
        assert!(!tour.active());
        assert!(!tour.next_enabled());
        assert!(tour.visible(now + 3.9));
        assert!(!tour.visible(now + 4.0));
        tour.next(now + 5.0);
        assert!(tour.notes().is_empty());
        assert_eq!(
            Step::ALL.map(Step::name),
            ["build", "chip", "run", "source", "back", "waves", "done"]
        );
    }

    #[test]
    fn chip_caption_changes_when_ready_without_bypassing_the_drawing_event() {
        for prebuilt in [false, true] {
            let mut tour = Tour::new(0.0);
            let mut seen = Evidence {
                missing_tool: prebuilt,
                build: Some(if prebuilt {
                    Err("yosys not found on PATH: install the open FPGA toolchain".into())
                } else {
                    Ok(())
                }),
                ..Default::default()
            };
            tour.observe(0.25, &seen);
            let entered = if prebuilt { 2.25 } else { 0.25 };
            if prebuilt {
                tour.observe(entered, &seen);
            }
            tour.next(entered);
            assert_eq!(tour.step, Step::Chip);
            assert_eq!(tour.caption(), Step::Chip.caption());
            assert_eq!(
                tour.notes().last().unwrap(),
                &format!("tour 2 chip {}", Step::Chip.caption())
            );

            seen.chip_ready = true;
            tour.observe(entered + 1.0, &seen);
            assert_eq!(tour.step, Step::Chip);
            assert_eq!(tour.caption(), CHIP_READY);
            assert_eq!(tour.notes(), [format!("tour caption 2 {CHIP_READY}")]);
            tour.observe(entered + 1.5, &seen);
            assert!(
                tour.notes().is_empty(),
                "unchanged captions are not repeated"
            );
            tour.observe(entered + 3.1, &seen);
            assert_eq!(
                tour.step,
                Step::Chip,
                "readiness alone cannot advance the step"
            );
            assert!(tour.notes().is_empty());

            seen.chip = true;
            tour.observe(entered + 3.2, &seen);
            assert_eq!(tour.step, Step::Chip);
            tour.next(entered + 3.2);
            assert_eq!(tour.step, Step::Run);
            assert_eq!(
                tour.notes(),
                [
                    "tour next 2".into(),
                    format!("tour 3 run {}", Step::Run.caption())
                ]
            );
        }
    }

    #[test]
    fn prebuilt_ready_during_skip_enters_chip_with_the_ready_caption() {
        let mut tour = Tour::new(0.0);
        let mut seen = Evidence {
            missing_tool: true,
            build: Some(Err(
                "yosys not found on PATH: install the open FPGA toolchain".into(),
            )),
            ..Default::default()
        };
        tour.observe(0.0, &seen);
        tour.notes();
        seen.chip_ready = true;
        seen.chip = true;
        tour.observe(1.49, &seen);
        assert_eq!(tour.step, Step::Build);
        assert_eq!(tour.caption(), SKIP);
        assert!(tour.notes().is_empty());

        tour.observe(2.0, &seen);
        assert_eq!(tour.step, Step::Build);
        tour.next(2.0);
        assert_eq!(tour.step, Step::Chip);
        assert_eq!(tour.caption(), CHIP_READY);
        assert_eq!(
            tour.notes(),
            ["tour next 1".into(), format!("tour 2 chip {CHIP_READY}")]
        );
        tour.observe(3.0, &seen);
        assert_eq!(tour.step, Step::Chip);
        assert_eq!(tour.caption(), CHIP_READY);
        assert!(
            tour.notes().is_empty(),
            "the entry caption needs no change note"
        );
    }

    #[test]
    fn ready_steps_wait_indefinitely_for_next_even_after_evidence_disappears() {
        for step in &Step::ALL[..6] {
            let mut tour = at(*step);
            tour.take_entry();
            tour.next(0.0);
            assert_eq!(tour.step, *step);
            assert!(!tour.next_enabled());
            assert!(tour.notes().is_empty());
            let mut seen = all_events();
            seen.chip_ready = false;
            tour.observe(0.1, &seen);
            assert!(tour.next_enabled());
            assert_eq!(tour.waiting(), None);
            tour.observe(10000.0, &Evidence::default());
            assert!(
                tour.active(),
                "no deadline while waiting for Next: {step:?}"
            );
            assert_eq!(tour.step, *step);
            assert!(tour.next_enabled());
            assert_eq!(tour.take_entry(), None);
            assert!(tour.notes().is_empty());
            tour.next(10000.0);
            assert_eq!(tour.step, Step::ALL[step.number()]);
            assert_eq!(tour.notes()[0], format!("tour next {}", step.number()));
            assert!(!tour.next_enabled(), "the next step needs its own evidence");
        }
    }

    #[test]
    fn missing_evidence_waits_indefinitely_without_advancing_or_ending() {
        for step in &Step::ALL[..6] {
            let mut tour = at(*step);
            for now in [0.0, 16.0, 90.0, 120.0, 10000.0] {
                tour.observe(now, &Evidence::default());
                tour.next(now);
                assert_eq!(tour.step, *step);
                assert!(tour.active());
                assert!(tour.visible(now));
                assert!(!tour.next_enabled());
                assert_eq!(tour.waiting(), Some(step.waiting()));
                assert!(tour.notes().is_empty());
            }
            tour.observe(10001.0, &all_events());
            assert!(tour.next_enabled(), "late evidence still enables Next");
        }
        let mut tour = at(Step::Run);
        let mut seen = all_events();
        seen.changes_drawn = false;
        tour.observe(8.0, &seen);
        assert!(!tour.next_enabled());
        seen.changes_drawn = true;
        seen.running_at_four = false;
        tour.observe(9.0, &seen);
        assert!(!tour.next_enabled());
        seen.running_at_four = true;
        tour.observe(9.1, &seen);
        assert_eq!(tour.step, Step::Run);
        assert!(tour.next_enabled());
        tour.next(9.1);
        assert_eq!(tour.step, Step::Source);
    }

    #[test]
    fn missing_executables_skip_the_build_but_other_errors_wait_for_recovery() {
        use ecp5_emu::flow::{self, messages::MessageKind};
        for reason in [
            "yosys not found on PATH: install the open FPGA toolchain",
            "nextpnr-ecp5 not found at /opt/bundle/nextpnr (from ECP5_NEXTPNR): reinstall the package",
            "ecppack not found on PATH: install the open FPGA toolchain",
            "ecpbram not found at /opt/bundle/ecpbram (from ECP5_ECPBRAM): reinstall the package",
            "An entirely different diagnostic wording",
        ] {
            let mut tour = Tour::new(0.0);
            tour.notes();
            let diagnostic = flow::Message {
                kind: MessageKind::MissingTool,
                severity: flow::Severity::Error,
                step: flow::Step::Synthesis,
                text: reason.into(),
                file: None,
                line: None,
                column: None,
                count: 1,
            };
            let seen = Evidence {
                missing_tool: flow::tools::is_missing_tool(&diagnostic),
                build: Some(Err(diagnostic.text)),
                ..Default::default()
            };
            tour.observe(0.5, &seen);
            assert!(tour.prebuilt);
            assert_eq!(tour.caption(), SKIP);
            assert_eq!(
                tour.notes(),
                [
                    format!("tour skip build {reason}"),
                    format!("tour 1 build {SKIP}")
                ]
            );
            tour.observe(2.0, &seen);
            assert_eq!(tour.step, Step::Build);
            tour.observe(2.5, &seen);
            assert!(tour.next_enabled());
            tour.next(2.5);
            assert_eq!(tour.step, Step::Chip);
            assert_eq!(tour.take_entry(), Some(Step::Chip));
        }
        for reason in [
            "yosys not found on PATH: install the open FPGA toolchain",
            "syntax error in top.v:7",
            "ERROR: Module `missing' not found!",
            "Build stopped",
            "yosys exited with status 1",
        ] {
            let mut tour = Tour::new(0.0);
            tour.observe(
                1.0,
                &Evidence {
                    build: Some(Err(reason.into())),
                    ..Default::default()
                },
            );
            assert!(tour.active());
            assert!(tour.visible(10000.0));
            assert!(!tour.next_enabled());
            assert_eq!(tour.waiting(), Some(Step::Build.waiting()));
            assert!(!tour.prebuilt);
            assert!(tour.caption().contains(reason));
            assert!(tour
                .notes()
                .contains(&format!("tour failed build {reason}")));
        }
    }

    #[test]
    fn errors_on_every_step_wait_with_one_record_note_and_allow_later_evidence() {
        for step in &Step::ALL[..6] {
            let mut tour = at(*step);
            tour.fail(1.0, "evidence unavailable");
            tour.fail(10000.0, "evidence unavailable");
            tour.observe(10000.0, &Evidence::default());
            assert!(tour.active());
            assert!(tour.visible(10000.0));
            assert!(!tour.next_enabled());
            assert_eq!(tour.waiting(), Some(step.waiting()));
            assert_eq!(
                tour.notes(),
                [format!("tour failed {} evidence unavailable", step.name())]
            );
            tour.observe(10001.0, &all_events());
            assert!(tour.next_enabled());
            assert_eq!(
                tour.caption(),
                if *step == Step::Chip {
                    CHIP_READY
                } else {
                    step.caption()
                }
            );
            tour.next(10002.0);
            assert_eq!(tour.step, Step::ALL[step.number()]);
        }
    }

    #[test]
    fn explicit_stop_ends_each_step_once_and_cannot_advance_again() {
        for step in &Step::ALL[..6] {
            let mut tour = at(*step);
            tour.stop(1.0);
            assert_eq!(tour.caption(), "Tour stopped: you are in control");
            assert_eq!(tour.notes(), [format!("tour stopped at {}", step.name())]);
            tour.observe(50.0, &all_events());
            tour.stop(50.0);
            assert_eq!(tour.step, *step);
            assert!(tour.notes().is_empty());
            assert!(tour.take_entry().is_none());
            assert!(tour.visible(2.99));
            assert!(!tour.visible(3.0));
        }
    }

    #[test]
    fn show_me_is_explicit_and_does_not_bypass_next_evidence() {
        for step in Step::ALL {
            let mut tour = at(step);
            assert_eq!(tour.show_me(), step.needs_gesture());
            let expected: Vec<_> = if step.needs_gesture() {
                vec![format!("tour show-me {}", step.number())]
            } else {
                vec![]
            };
            assert_eq!(tour.notes(), expected);
            assert!(!tour.next_enabled());
            tour.stop(1.0);
            tour.notes();
            assert!(!tour.show_me());
            assert!(tour.notes().is_empty());
        }
    }

    #[test]
    fn counter_is_found_in_the_assignment_including_moved_and_unicode_lines() {
        let original = include_str!("../../../examples/ulx3s/blinky_cycle/top.v");
        let (line, column) = counter_location(original).unwrap();
        assert_eq!(
            ecp5_emu::sources::identifier_at(original, line, column),
            "count"
        );
        let moved = format!("// count <= count + 1;\n\n{original}");
        assert_eq!(counter_location(&moved), Some((line + 2, column)));
        assert_eq!(
            counter_location("reg [7:0] count;\n/* λ */ count <= count + 1;"),
            Some((2, 9))
        );
        assert!(counter_location("reg [7:0] count; // count <= count + 1").is_none());
    }

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            Self(crate::test_temp::unique_dir("ecp5-tour-"))
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(path: &Path) {
        fs::create_dir_all(path.join("prebuilt")).unwrap();
        for name in [
            "ecp5.toml",
            "top.v",
            "top.lpf",
            "prebuilt/top.config",
            "prebuilt/top_routed.json",
        ] {
            fs::write(path.join(name), name).unwrap();
        }
    }

    #[test]
    fn lookup_prefers_env_then_architecture_sibling_then_packaged_locations_then_checkout() {
        let temp = Temporary::new();
        let root = &temp.0;
        let checkout = root.join("repo/examples/ulx3s/blinky_cycle");
        let repo = root.join("repo");
        let config = root.join("config");
        fixture(&checkout);
        fs::remove_dir_all(checkout.join("prebuilt")).unwrap();
        let reference = repo.join("target/reference/blinky_cycle");
        fs::create_dir_all(&reference).unwrap();
        fs::write(reference.join("top.config"), "config").unwrap();
        fs::write(reference.join("top_routed.json"), "json").unwrap();

        for (platform, executable, resources, launcher) in [
            (
                "macos",
                "ecp5.app/Contents/MacOS/ecp5_gui",
                "ecp5.app/Contents/Resources",
                None,
            ),
            (
                "linux",
                "linux/libexec/ecp5_gui",
                "linux/share",
                Some("linux/bin/ecp5_gui"),
            ),
        ] {
            let exe = root.join(executable);
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, "").unwrap();
            if let Some(launcher) = launcher {
                let launcher = root.join(launcher);
                fs::create_dir_all(launcher.parent().unwrap()).unwrap();
                fs::write(launcher, "#!/bin/sh\nexec ../libexec/ecp5_gui \"$@\"\n").unwrap();
            }
            let architecture = root.join(resources).join("architecture");
            fs::create_dir_all(&architecture).unwrap();
            let packaged = root.join(resources).join("tour/blinky_cycle");
            let located =
                TourProject::locate(None, Some(&architecture), &exe, &repo, &config).unwrap();
            assert_eq!(located.root, checkout.canonicalize().unwrap());
            assert_eq!(located.prebuilt, reference);

            fixture(&packaged);
            for architecture in [None, Some(architecture.as_path())] {
                let located =
                    TourProject::locate(None, architecture, &exe, &repo, &config).unwrap();
                assert_eq!(located.root, packaged.canonicalize().unwrap(), "{platform}");
                assert_eq!(
                    located.prebuilt,
                    packaged.canonicalize().unwrap().join("prebuilt")
                );
            }

            let relocated_architecture = root.join(platform).join("relocated/share/architecture");
            fs::create_dir_all(&relocated_architecture).unwrap();
            let sibling = relocated_architecture
                .parent()
                .unwrap()
                .join("tour/blinky_cycle");
            fixture(&sibling);
            let located =
                TourProject::locate(None, Some(&relocated_architecture), &exe, &repo, &config)
                    .unwrap();
            assert_eq!(located.root, sibling.canonicalize().unwrap());

            let env = root.join(platform).join("override");
            fixture(&env);
            let located = TourProject::locate(
                Some(&env),
                Some(&relocated_architecture),
                &exe,
                &repo,
                &config,
            )
            .unwrap();
            assert_eq!(located.root, env.canonicalize().unwrap());
            assert_eq!(
                located.prebuilt,
                env.canonicalize().unwrap().join("prebuilt")
            );
            assert!(TourProject::locate(
                Some(&root.join("missing")),
                Some(&architecture),
                &exe,
                &repo,
                &config
            )
            .is_err());
            assert_eq!(
                fs::read_dir(&env).unwrap().count(),
                4,
                "the writability probe was removed"
            );
        }
        assert!(!config.exists(), "a writable project needs no copy");
    }

    #[test]
    #[cfg(unix)]
    fn read_only_project_is_copied_once_and_sources_in_the_copy_are_writable() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temporary::new();
        let source = temp.0.join("read-only");
        fixture(&source);
        fs::set_permissions(source.join("top.v"), fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
        let config = temp.0.join("config");
        let find = || {
            TourProject::locate(
                Some(&source),
                None,
                &temp.0.join("ecp5_gui"),
                &temp.0,
                &config,
            )
            .unwrap()
        };
        let copied = find();
        assert_eq!(
            copied.root,
            config.join("tour/blinky_cycle").canonicalize().unwrap()
        );
        copied.check_prebuilt().unwrap();
        assert!(!fs::metadata(copied.root.join("top.v"))
            .unwrap()
            .permissions()
            .readonly());
        fs::write(copied.root.join("top.v"), "user's edits").unwrap();
        fs::write(source.join("top.lpf"), "changed original").unwrap();
        assert_eq!(find().root, copied.root);
        assert_eq!(
            fs::read_to_string(copied.root.join("top.v")).unwrap(),
            "user's edits"
        );
        assert_eq!(
            fs::read_to_string(copied.root.join("top.lpf")).unwrap(),
            "top.lpf"
        );
        fs::set_permissions(source.join("top.v"), fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The shipped design and source list, drawn by egui without a native window.
    #[test]
    #[ignore = "needs the blinky_cycle prebuilt, architecture catalogues and clang"]
    fn prebuilt_counter_probes_a_visible_wire_returns_to_source_and_delivers_waves() {
        use crate::{fabric_view::FabricView, project::editor::Editor, waves::Waves};
        use ecp5_emu::{
            live::{self, Command, Design, RunOptions, Shared},
            sources::Files,
        };
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        let repo = crate::start::repository_root();
        let root = repo
            .join("examples/ulx3s/blinky_cycle")
            .canonicalize()
            .unwrap();
        let prebuilt = repo.join("target/reference/blinky_cycle");
        let files = Files::project(root.clone(), ["top.v".into()]);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn_with_sources(
            Design {
                config: prebuilt.join("top.config"),
                names: Some(prebuilt.join("top_routed.json")),
                run: false,
            },
            RunOptions::default(),
            Some(files),
            Arc::clone(&shared),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(120);
        let loaded = loop {
            let shared = shared.lock().unwrap();
            assert!(shared.error.is_none(), "{:?}", shared.error);
            if let Some(ready) = shared.ready.as_ref().filter(|s| s.sources.get().is_some()) {
                break Arc::clone(ready);
            }
            assert!(Instant::now() < deadline, "loading: {}", shared.stage);
            drop(shared);
            std::thread::sleep(Duration::from_millis(20));
        };
        let path = root.join("top.v");
        let mut editor = Editor::default();
        editor.open(&path, "top.v".into(), false, None).unwrap();
        let (line, column) = counter_location(editor.text(&path).unwrap()).unwrap();
        let probe = editor.select_probe(&path, line, column).unwrap();
        let index = loaded.sources.get().unwrap();
        let found = index.probe("top.v", probe.line, probe.column, &probe.identifier);
        assert!(!found.nets.is_empty(), "{}", found.message);
        let mut map = FabricView::new(None, None, (0.5, 0.5));
        map.sources = Some(Arc::clone(index));
        map.keep_nets(found.nets.clone());
        let ctx = egui::Context::default();
        let values = shared.lock().unwrap().lit.clone();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::pos2(600.0, 40.0),
                    egui::vec2(900.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                map.show(
                    ui,
                    &loaded.fabric,
                    Some((&loaded.overlay, &values)),
                    Some(&loaded.net_names),
                );
            },
        );
        let point = map
            .probed_point(&loaded.fabric, &loaded.overlay)
            .expect("a visible and pickable piece of count");
        assert!(map.show_source_at(&loaded.fabric, &loaded.overlay, point));
        let (_, _, span) = map
            .show_source
            .take()
            .expect("the map menu's source action");
        assert_eq!(span.file.as_ref(), "top.v");
        editor
            .open(
                &root.join(span.file.as_ref()),
                span.file.to_string(),
                false,
                Some(span.line),
            )
            .unwrap();
        editor.select_span(span);
        let mut waves = Waves::default();
        for net in found.nets {
            let name = index
                .nets
                .get(&net)
                .and_then(|n| n.names.iter().find(|n| !n.contains('$')))
                .cloned();
            waves.add_net_from(&loaded, net, name, "source");
        }
        assert!(waves.open);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| {
                waves.show(ui, Some(&loaded), 0, None, None);
            },
        );
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        for bit in 0..8 {
            assert!(
                labels.contains(&format!("count[{bit}]").as_str()),
                "{labels:?}"
            );
        }
        assert!(!labels.contains(&"count[8]"), "the bus must be expanded");
        let watch = waves.command().unwrap();
        let (id, width) = match &watch {
            Command::Watch { id, wires, .. } => (*id, wires.len()),
            _ => unreachable!(),
        };
        commands.send(watch).unwrap();
        commands.send(Command::Rate(Some(4.0))).unwrap();
        commands.send(Command::Run).unwrap();
        loop {
            let mut shared = shared.lock().unwrap();
            assert!(shared.error.is_none(), "{:?}", shared.error);
            if shared
                .waves
                .iter()
                .any(|run| run.id == id && run.width == width && run.rows.iter().any(|&v| v <= 3))
            {
                let edges = shared.edges;
                waves.take(std::mem::take(&mut shared.waves), edges, 0);
                break;
            }
            assert!(Instant::now() < deadline, "waiting for counter waves");
            drop(shared);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn constructing_a_tour_with_an_injected_ready_reader_spawns_no_build() {
        let temp = Temporary::new();
        let reader_path = temp.0.join("target/trellis_image_reader");
        assert!(!reader_path.exists());
        let check = reader::fixed(Status::Ready);
        assert_eq!(check.status(), Status::Ready);
        assert!(!check.build_requested);
        let mut session = Session::with_reader(
            TourProject {
                root: temp.0.clone(),
                prebuilt: temp.0.join("prebuilt"),
            },
            check,
        );
        assert!(session.machine.reader.is_none(), "there is no Building job");
        assert_eq!(session.machine.take_entry(), Some(Step::Build));
        assert_eq!(
            session.machine.notes(),
            [
                "reader found".into(),
                format!("tour 1 build {}", Step::Build.caption())
            ]
        );
        assert!(
            !reader_path.exists(),
            "constructing a tour creates no reader file"
        );
    }

    #[test]
    fn reader_failure_has_its_own_caption_and_note_before_step_one() {
        let failure = reader::script_error("build_image_reader.py: error: prjtrellis source not found; use --trellis PATH or TRELLIS_ROOT");
        let mut tour = Tour::with_reader(0.0, reader::fixed(Status::Missing(failure.clone())));
        assert!(tour.caption().contains(&failure));
        assert!(tour.caption().ends_with("See README Requirements."));
        assert!(tour.active());
        assert!(tour.visible(10000.0));
        assert_eq!(tour.waiting(), Some(tour.caption()));
        assert_eq!(tour.take_entry(), None);
        assert_eq!(
            tour.notes(),
            [
                "reader missing".to_owned(),
                format!("tour failed reader {failure}")
            ]
        );
        tour.observe(100.0, &all_events());
        assert!(tour.notes().is_empty());

        let mut tour = Tour::with_reader(0.0, reader::fixed(Status::Building));
        tour.notes();
        tour.reader.as_ref().unwrap().finish(Err(failure.clone()));
        tour.observe(0.25, &all_events());
        assert!(tour.caption().contains(&failure));
        assert_eq!(tour.notes(), [format!("tour failed reader {failure}")]);
        assert!(tour.active());
        assert_eq!(tour.waiting(), Some(tour.caption()));
        assert_eq!(tour.take_entry(), None);
        tour.stop(10000.0);
        assert_eq!(tour.notes(), ["tour stopped at reader"]);
        assert!(!tour.active());

        let mut tour = Tour::with_reader(0.0, reader::fixed(Status::Building));
        assert_eq!(tour.caption(), reader::BUILDING);
        assert!(tour.caption().contains("tools/build_image_reader.py"));
        assert_eq!(tour.take_entry(), None);
        assert_eq!(tour.notes(), ["tour reader building", "reader missing"]);
        tour.observe(1000.0, &all_events());
        assert!(
            tour.active(),
            "reader work does not consume the build's deadline"
        );
        assert_eq!(tour.take_entry(), None);
        tour.reader.as_ref().unwrap().finish(Ok(()));
        tour.observe(1001.0, &all_events());
        assert_eq!(tour.take_entry(), Some(Step::Build));
        assert_eq!(tour.caption(), Step::Build.caption());
        assert_eq!(
            tour.notes(),
            [
                "reader built".into(),
                format!("tour 1 build {}", Step::Build.caption())
            ]
        );
        assert_eq!(tour.since, 1001.0);

        let mut tour = Tour::with_reader(2.0, reader::fixed(Status::Ready));
        assert_eq!(tour.take_entry(), Some(Step::Build));
        assert_eq!(
            tour.notes(),
            [
                "reader found".into(),
                format!("tour 1 build {}", Step::Build.caption())
            ]
        );
    }

    #[test]
    fn reader_card_has_no_numbered_step_and_no_home_in_painted_strings() {
        let home = crate::test_temp::home_dir().display().to_string();
        for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, theme);
            for width in [600.0, 1500.0] {
                let text = reader::script_error(&format!("usage: build_image_reader.py [-h]\nbuild_image_reader.py: error: prjtrellis source not found (invalid path: {home}/FPGA/prjtrellis)\n"));
                let draw = || {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 898.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            card(ui, ui.max_rect(), &text, Step::Build, false, None, None);
                        },
                    )
                };
                // The Area measures itself on its first frame.
                let _ = draw();
                let output = draw();
                fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(t) => text.push(t.galley.text().into()),
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                collect(shape, text);
                            }
                        }
                        _ => {}
                    }
                }
                let mut painted = Vec::new();
                for shape in &output.shapes {
                    collect(&shape.shape, &mut painted);
                }
                assert!(painted
                    .iter()
                    .any(|s| s.contains("prjtrellis source not found")));
                assert!(painted
                    .iter()
                    .any(|s| s.contains("See README Requirements.")));
                assert!(painted.iter().any(|s| s == "Take the tour · image reader"));
                assert!(
                    painted
                        .iter()
                        .all(|s| !s.contains(&home) && !s.contains("step 1")),
                    "{painted:?}"
                );
            }
        }
    }

    #[test]
    fn every_caption_has_a_fixed_card_in_both_themes_without_moving_the_content() {
        for preference in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, preference);
            for width in [700.0, 1500.0] {
                for text in Step::ALL.map(Step::caption).into_iter().chain([SKIP, CHIP_READY, reader::MISSING, reader::BUILDING, "Tour stopped: you are in control", "Tour ended at run: timed out after 16 s waiting for running at 4 cycles/s and changes drawn lit"]) {
                    let output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0))), ..Default::default() }, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            let area = ui.available_rect_before_wrap();
                            ui.allocate_exact_size(area.size(), egui::Sense::hover());
                            let before = ui.min_rect();
                            let (rect, _, _, _) = card(ui, area, text, Step::Build, true, None, Some(Step::Build.waiting()));
                            assert_eq!(rect.size(), egui::vec2(400.0, 220.0));
                            assert_eq!(ui.min_rect(), before);
                            let (other, _, _, _) = card(ui, area, text, Step::Build, true, Some(rect.center()), None);
                            assert_eq!(other.size(), rect.size());
                            assert!(!other.contains(rect.center()));
                        });
                    });
                    let found = output.shapes.iter().find_map(|shape| match &shape.shape { egui::Shape::Text(label) if label.galley.text() == text => Some(label), _ => None }).expect("the complete caption is drawn");
                    assert!(found.galley.size().y <= 88.0, "caption must fit: {text}");
                }
            }
        }
    }
}
