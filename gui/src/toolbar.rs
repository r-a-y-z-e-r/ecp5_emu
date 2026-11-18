//! Layout A: one row, shared by the config viewer and both project tabs.
use super::*;

const REPLAY: &str = "You went back: stepping on runs these cycles again with the same inputs. A button press or a key here starts a new run from this cycle.";
const VALUES: &str = "Colour the design's wires, LUTs and flip-flops by their values: 1 bright, 0 dim tan, unknown grey-violet; a change still runs as an orange stripe.";
const WAVES: &str = "Nets' values after every clock edge over the last cycles, under the map: add them by HDL name, with W over a wire, or the net kept on the map. A click on the traces goes to that cycle.";
const RATE: &str = "Target rate: how fast the design runs, not measured throughput. At a few cycles per second each new value runs along its wires.";
const RATES: [(&str, Option<f64>); 6] = [
    ("1 cycle/s", Some(1.0)),
    ("4 cycles/s", Some(4.0)),
    ("100 cycles/s", Some(100.0)),
    ("10k cycles/s", Some(1e4)),
    ("real time (25 MHz)", Some(live::REAL_TIME)),
    ("as fast as possible", None),
];

#[derive(Clone, Copy, PartialEq)]
enum Editor {
    Step,
    Rate,
}

#[derive(Default)]
pub(super) struct State {
    editor: Option<Editor>,
    focus: bool,
    popup_text: String,
    goto: bool,
    goto_focus: bool,
    goto_error: bool,
    size: Option<egui::Vec2>,
    pub(crate) hosted: bool,
    #[cfg(test)]
    pub(crate) controls: Vec<(String, egui::Rect, bool)>,
    #[cfg(test)]
    pub(crate) layout: Option<Rects>,
    #[cfg(test)]
    snapshot: Option<Snapshot>,
}

#[derive(Clone, Copy)]
pub(crate) struct Snapshot {
    pub ready: bool,
    pub running: bool,
    pub cycles: u64,
    pub earliest: u64,
    pub ahead: u64,
    pub rate: Option<f64>,
}

#[derive(Clone, Copy)]
pub(crate) struct Rects {
    pub row: egui::Rect,
    pub navigation: egui::Rect,
    pub build: Option<egui::Rect>,
    pub transport: egui::Rect,
    pub time: egui::Rect,
    pub view: Option<egui::Rect>,
}

pub(crate) fn prepare(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = if ui.ctx().content_rect().width() < 1500.0 {
        2.0
    } else {
        4.0
    };
    ui.spacing_mut().button_padding = egui::vec2(4.0, 4.0);
    ui.spacing_mut().interact_size.y = 26.0;
    ui.set_height(30.0);
}

/// Match native menu buttons, including the start screen's inherited row style.
fn control_height(ui: &egui::Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y)
}

fn editor_margin(ui: &egui::Ui) -> egui::Margin {
    let vertical = (control_height(ui) - ui.text_style_height(&egui::TextStyle::Body))
        .max(0.0)
        .round() as i8;
    let horizontal = ui.spacing().button_padding.x.round() as i8;
    egui::Margin {
        left: horizontal,
        right: horizontal,
        top: vertical / 2,
        bottom: vertical - vertical / 2,
    }
}

pub(crate) fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 26.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    NewProject,
    Back,
    Run,
    Pause,
    Step,
    Values,
    Wires,
    Tiles,
    Waves,
    Critical,
}

/// Vector marks keep the same geometry with every font and platform.
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    width: f32,
    selected: bool,
    text: bool,
) -> egui::Response {
    let response = ui.add_sized(
        [width, control_height(ui)],
        egui::Button::new("").selected(selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    let c = egui::pos2(response.rect.left() + 12.0, response.rect.center().y);
    let p = ui.painter();
    let colour = ui
        .style()
        .interact_selectable(&response, selected)
        .fg_stroke
        .color;
    if text {
        let galley = egui::WidgetText::from(label).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (response.rect.width() - 26.0).max(1.0),
            egui::TextStyle::Button,
        );
        p.galley(
            egui::pos2(
                response.rect.left() + 23.0,
                response.rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            colour,
        );
    }
    let stroke = egui::Stroke::new(1.4, colour);
    let pos = |x, y| c + egui::vec2(x, y);
    match icon {
        Icon::NewProject => {
            p.line_segment([pos(-5.0, 0.0), pos(5.0, 0.0)], stroke);
            p.line_segment([pos(0.0, -5.0), pos(0.0, 5.0)], stroke);
        }
        Icon::Back | Icon::Run | Icon::Step => {
            let d = if matches!(icon, Icon::Back) {
                -1.0
            } else {
                1.0
            };
            p.add(egui::Shape::convex_polygon(
                vec![pos(-4.0 * d, -6.0), pos(5.0 * d, 0.0), pos(-4.0 * d, 6.0)],
                colour,
                egui::Stroke::NONE,
            ));
            if matches!(icon, Icon::Step) {
                p.line_segment([pos(7.0, -6.0), pos(7.0, 6.0)], stroke);
            }
        }
        Icon::Pause => {
            for x in [-3.0, 3.0] {
                p.line_segment([pos(x, -6.0), pos(x, 6.0)], egui::Stroke::new(3.0, colour));
            }
        }
        Icon::Values => {
            p.rect_stroke(
                egui::Rect::from_min_max(pos(-6.0, -4.0), pos(-1.0, 4.0)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            p.line_segment([pos(4.0, -4.0), pos(4.0, 4.0)], stroke);
        }
        Icon::Tiles => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12.0, 12.0)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            p.line_segment([pos(-6.0, 0.0), pos(6.0, 0.0)], stroke);
            p.line_segment([pos(0.0, -6.0), pos(0.0, 6.0)], stroke);
        }
        Icon::Wires | Icon::Waves | Icon::Critical => {
            let points = match icon {
                Icon::Wires => vec![
                    pos(-6.0, 5.0),
                    pos(-2.0, -4.0),
                    pos(2.0, 4.0),
                    pos(6.0, -6.0),
                ],
                Icon::Waves => vec![
                    pos(-7.0, 4.0),
                    pos(-3.0, 4.0),
                    pos(-3.0, -4.0),
                    pos(3.0, -4.0),
                    pos(3.0, 4.0),
                    pos(7.0, 4.0),
                ],
                _ => vec![
                    pos(-6.0, 6.0),
                    pos(-2.0, 0.0),
                    pos(2.0, 0.0),
                    pos(7.0, -6.0),
                ],
            };
            p.add(egui::Shape::line(points, stroke));
        }
    }
    response
}

impl App {
    /// P6 integration slot immediately after Design in every navigation group.
    pub(crate) fn peripherals_segment(&mut self, ui: &mut egui::Ui) {
        self.peripherals_menu(ui);
    }
    pub(crate) fn new_project_segment(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let response = ui
            .add_enabled_ui(self.creation_available, |ui| {
                icon_button(ui, Icon::NewProject, "New project", 108.0, false, true)
            })
            .inner
            .on_disabled_hover_text("Wait for the current build or emulation to finish");
        self.remember("New project", &response);
        response.clicked().then_some(Action::NewProject)
    }
    pub(crate) fn reset_toolbar_editors(&mut self) {
        self.toolbar.editor = None;
        self.toolbar.goto = false;
        self.toolbar.goto_error = false;
        self.toolbar.goto_focus = false;
        self.toolbar.focus = false;
    }
    pub(crate) fn reset_toolbar_recording(&mut self) {
        self.toolbar.size = None;
    }

    pub(crate) fn set_toolbar_hosted(&mut self) {
        self.toolbar.hosted = true;
    }
    pub(crate) fn emulator_status(&self, ui: &mut egui::Ui) {
        let s = self.shared.lock().unwrap();
        let text = s.error.as_deref().unwrap_or(&s.stage);
        ui.add(egui::Label::new(text).truncate())
            .on_hover_text(text);
        ui.label(format!("cycle {}", s.cycles));
    }

    pub(crate) fn toolbar_snapshot(&self) -> Snapshot {
        #[cfg(test)]
        if let Some(snapshot) = self.toolbar.snapshot {
            return snapshot;
        }
        let s = self.shared.lock().unwrap();
        Snapshot {
            ready: s.ready.is_some(),
            running: s.running,
            cycles: s.cycles,
            earliest: s.earliest,
            ahead: s.ahead,
            rate: s.rate,
        }
    }
    pub(crate) fn toolbar_name(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        path: &str,
        compact: bool,
    ) {
        let response = ui
            .add_sized(
                [if compact { 64.0 } else { 96.0 }, 26.0],
                egui::Label::new(RichText::new(name).strong()).truncate(),
            )
            .on_hover_text(path);
        self.remember("Design name", &response);
    }
    pub(crate) fn remember(&mut self, label: &str, response: &egui::Response) {
        #[cfg(test)]
        self.toolbar
            .controls
            .push((label.into(), response.rect, response.enabled()));
        #[cfg(not(test))]
        let _ = (label, response);
    }
    #[cfg(test)]
    pub(crate) fn toolbar_test_snapshot(&mut self, snapshot: Snapshot) {
        self.toolbar.snapshot = Some(snapshot);
    }
    #[cfg(test)]
    pub(crate) fn toolbar_test_evidence(&self) -> (Rects, &[(String, egui::Rect, bool)]) {
        (self.toolbar.layout.unwrap(), &self.toolbar.controls)
    }
    #[cfg(test)]
    pub(crate) fn toolbar_test_project_controls(
        &mut self,
        controls: Vec<(String, egui::Rect, bool)>,
    ) {
        self.toolbar.controls.extend(controls);
    }
    pub(crate) fn begin_toolbar(&mut self) {
        #[cfg(test)]
        self.toolbar.controls.clear();
    }
    pub(crate) fn finish_toolbar(&mut self, ui: &egui::Ui, rects: Rects) {
        #[cfg(test)]
        {
            self.toolbar.layout = Some(rects);
        }
        let size = ui.ctx().content_rect().size();
        if self.toolbar.size == Some(size) {
            return;
        }
        // A stable row has no layout changes when counts, running state or badges change.
        self.toolbar.size = Some(size);
        let rect = |r: egui::Rect| {
            format!(
                "{:.3},{:.3},{:.3},{:.3}",
                r.min.x, r.min.y, r.max.x, r.max.y
            )
        };
        self.record_note(&format!("toolbar size={:.0},{:.0} row={} navigation={} build={} transport={} time={} view={} compact={}",
            size.x,size.y,rect(rects.row),rect(rects.navigation),rects.build.map_or_else(|| "-".into(),rect),rect(rects.transport),rect(rects.time),rects.view.map_or_else(|| "-".into(),rect),u8::from(size.x < 1500.0)));
    }
    pub(crate) fn plain_toolbar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let compact = ui.ctx().content_rect().width() < 1500.0;
        let s = self.toolbar_snapshot();
        let mut action = None;
        let mut rects = None;
        self.begin_toolbar();
        egui::Panel::top("controls")
            .exact_size(46.0)
            .show(ui, |ui| {
                prepare(ui);
                let row = ui.horizontal_centered(|ui| {
                    let navigation = ui
                        .horizontal(|ui| {
                            action = self.design_menu(ui, None);
                            self.peripherals_segment(ui);
                            if let Some(a) = self.new_project_segment(ui) {
                                action = Some(a);
                            }
                            let title = self.title.clone();
                            self.toolbar_name(ui, &title, &title, compact);
                        })
                        .response
                        .rect;
                    divider(ui);
                    let transport = self.transport_segment(ui, s, compact);
                    divider(ui);
                    let time = self.time_segment(ui, s, compact);
                    let view = self.right_view_segment(ui, s, compact);
                    (navigation, transport, time, view)
                });
                rects = Some(Rects {
                    row: row.response.rect,
                    navigation: row.inner.0,
                    build: None,
                    transport: row.inner.1,
                    time: row.inner.2,
                    view: row.inner.3,
                });
            });
        self.finish_toolbar(ui, rects.unwrap());
        action
    }
    fn transport_reason(s: Snapshot) -> &'static str {
        if !s.ready {
            "Emulate a design first; wait until it is ready."
        } else {
            "Pause first."
        }
    }
    fn back(&mut self, ui: &mut egui::Ui, s: Snapshot, text: bool) {
        let steps = self.steps();
        let response = ui.add_enabled_ui(s.ready && !s.running && s.cycles > s.earliest, |ui| icon_button(ui,Icon::Back,"Back",if text {80.0} else {26.0},false,text)).inner
            .on_hover_text(format!("Back {steps} {} (←). The run is as it was there; stepping on takes the same button presses and UART input again, until you give a new one.",if steps == 1 {"cycle"} else {"cycles"}))
            .on_disabled_hover_text(format!("{} Back as far as cycle {}, where fast mode starts.",Self::transport_reason(s),s.earliest));
        self.remember("Back", &response);
        if response.clicked() {
            self.send(Command::Back(steps));
        }
    }
    fn run_pause(&mut self, ui: &mut egui::Ui, s: Snapshot, toggle: bool, width: f32) {
        for running in [false, true] {
            if toggle && running != s.running {
                continue;
            }
            let label = if running { "Pause" } else { "Run" };
            let response = ui
                .add_enabled_ui(s.ready && (toggle || s.running == running), |ui| {
                    icon_button(
                        ui,
                        if running { Icon::Pause } else { Icon::Run },
                        label,
                        width,
                        false,
                        true,
                    )
                })
                .inner
                .on_hover_text(if running {
                    "Pause the run."
                } else {
                    "Run the design at the target rate."
                })
                .on_disabled_hover_text(Self::transport_reason(s));
            self.remember(label, &response);
            if response.clicked() {
                self.send(if running {
                    Command::Pause
                } else {
                    Command::Run
                });
            }
        }
    }
    fn step(&mut self, ui: &mut egui::Ui, s: Snapshot, width: f32) {
        let steps = self.steps();
        let label = format!("Step {}", compact_count(steps));
        let response = ui
            .add_enabled_ui(s.ready && !s.running, |ui| {
                icon_button(ui, Icon::Step, &label, width, false, true)
            })
            .inner
            .on_hover_text(format!(
                "Run {steps} {} and pause (→).",
                if steps == 1 { "cycle" } else { "cycles" }
            ))
            .on_disabled_hover_text(Self::transport_reason(s));
        self.remember("Step", &response);
        if response.clicked() {
            self.send(Command::Step(steps));
        }
    }
    fn open_editor(&mut self, editor: Editor, rate: Option<f64>) {
        self.toolbar.editor = Some(editor);
        self.toolbar.focus = true;
        match editor {
            Editor::Step => self.toolbar.popup_text = self.steps().to_string(),
            Editor::Rate => self.toolbar.popup_text = rate.map(short).unwrap_or_default(),
        }
    }
    fn step_choices(&mut self, ui: &mut egui::Ui) {
        for (label, n) in [
            ("1 cycle", 1),
            ("1k cycles", 1000),
            ("1M cycles", 1_000_000),
        ] {
            let response = ui.selectable_label(self.step_choice == Some(n), label);
            self.remember(label, &response);
            if response.clicked() {
                self.step_choice = Some(n);
                ui.close();
            }
        }
        let response = ui
            .button("Custom…")
            .on_hover_text("Steps of a number of cycles you type");
        self.remember("Custom step", &response);
        if response.clicked() {
            self.open_editor(Editor::Step, None);
            ui.close();
        }
    }
    pub(crate) fn transport_segment(
        &mut self,
        ui: &mut egui::Ui,
        s: Snapshot,
        compact: bool,
    ) -> egui::Rect {
        ui.horizontal(|ui| {
            self.back(ui, s, false);
            self.run_pause(ui, s, true, if compact { 60.0 } else { 64.0 });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                self.step(ui, s, if compact { 72.0 } else { 76.0 });
                let arrow = ui
                    .menu_button("▾", |ui| self.step_choices(ui))
                    .response
                    .on_hover_text("Step size: 1, 1k, 1M or Custom…");
                self.remember("Step size", &arrow);
                self.numeric_popup(ui, &arrow, Editor::Step);
            });
        })
        .response
        .rect
    }
    fn numeric_popup(&mut self, ui: &egui::Ui, anchor: &egui::Response, editor: Editor) {
        let mut open = self.toolbar.editor == Some(editor);
        let mut done = false;
        egui::Popup::from_response(anchor)
            .id(ui.id().with(if editor == Editor::Step {
                "custom_step"
            } else {
                "custom_rate"
            }))
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(220.0);
                ui.label(if editor == Editor::Step {
                    "Cycles per step"
                } else {
                    "Custom rate"
                });
                let text = &mut self.toolbar.popup_text;
                let parsed = amount(text);
                let valid = parsed.is_some_and(|n| n.is_finite() && n > 0.0);
                let field = ui
                    .horizontal(|ui| {
                        let field = ui.add(egui::TextEdit::singleline(text).desired_width(125.0));
                        ui.label(if editor == Editor::Step {
                            "cycles"
                        } else {
                            "cycles/s"
                        });
                        field
                    })
                    .inner
                    .on_hover_text(if editor == Editor::Step {
                        "Cycles a step goes: 1500, 1 500, 2.5k, 3M, 1G."
                    } else {
                        "Cycles per second: 3, 0.5, 2500, 2.5k, 12M."
                    });
                self.remember("Custom input", &field);
                if std::mem::take(&mut self.toolbar.focus) {
                    field.request_focus();
                }
                if !valid {
                    ui.colored_label(
                        theme::colours(ui.visuals().dark_mode).error,
                        "Enter a positive number",
                    );
                }
                ui.horizontal(|ui| {
                    let apply = ui.add_enabled(valid, egui::Button::new("Apply"));
                    self.remember("Apply", &apply);
                    if apply.clicked() {
                        if editor == Editor::Step {
                            self.step_text = self.toolbar.popup_text.clone();
                            self.step_choice = None;
                        } else {
                            self.rate_text = self.toolbar.popup_text.clone();
                            self.rate_custom = true;
                            self.send(Command::Rate(parsed));
                        }
                        done = true;
                    }
                    let cancel = ui.button("Cancel");
                    self.remember("Cancel", &cancel);
                    if cancel.clicked() {
                        done = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = true;
                }
            });
        if self.toolbar.editor == Some(editor) && (!open || done) {
            self.toolbar.editor = None;
        }
    }
    fn open_goto(&mut self, cycles: u64) {
        self.goto_text = cycles.to_string();
        self.toolbar.goto = true;
        self.toolbar.goto_focus = true;
        self.toolbar.goto_error = false;
    }
    fn rate_choices(&mut self, ui: &mut egui::Ui, rate: Option<f64>) {
        for (label, value) in RATES {
            let response = ui.selectable_label(!self.rate_custom && value == rate, label);
            self.remember(label, &response);
            if response.clicked() {
                self.rate_custom = false;
                self.send(Command::Rate(value));
                ui.close();
            }
        }
        let response = ui.button("Custom…");
        self.remember("Custom rate", &response);
        if response.clicked() {
            self.open_editor(Editor::Rate, rate);
            ui.close();
        }
    }
    pub(crate) fn time_segment(
        &mut self,
        ui: &mut egui::Ui,
        s: Snapshot,
        compact: bool,
    ) -> egui::Rect {
        ui.horizontal(|ui| {
            let width = if compact {152.0} else {214.0};
            if self.toolbar.goto {
                let margin = editor_margin(ui);
                let field = ui.add_enabled(s.ready && !s.running,egui::TextEdit::singleline(&mut self.goto_text).desired_width(width - 8.0).margin(margin).vertical_align(egui::Align::Center))
                    .on_hover_text(format!("A cycle to go to, back or on, and pause there (Enter): 120000, 2.5M. Back as far as cycle {}.",s.earliest));
                self.remember("Go to cycle",&field);
                let opening = std::mem::take(&mut self.toolbar.goto_focus);
                if opening { field.request_focus(); }
                // A menu can open this editor on the same click; don't dismiss that opening click.
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape))
                    || (!opening && field.clicked_elsewhere());
                if cancel {
                    self.toolbar.goto = false;
                    self.toolbar.goto_error = false;
                    field.surrender_focus();
                    ui.ctx().request_repaint();
                } else if (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) && s.ready && !s.running {
                    if let Some(cycle) = cycle_number(&self.goto_text) { self.send(Command::GoTo(cycle)); self.toolbar.goto = false; field.surrender_focus(); } else { self.toolbar.goto_error = true; field.request_focus(); }
                }
                if self.toolbar.goto && self.toolbar.goto_error {
                    egui::Popup::from_response(&field).id(ui.id().with("goto_error")).show(|ui| {ui.colored_label(theme::colours(ui.visuals().dark_mode).error,"Invalid cycle; use 120000 or 2.5M");});
                }
            } else {
                let response = ui.add_enabled(s.ready && !s.running,egui::Button::new("").min_size(egui::vec2(width,26.0)))
                    .on_hover_text(format!("Cycle {}. Go to cycle…; click to type, Enter goes there. Back as far as cycle {}.",s.cycles,s.earliest))
                    .on_disabled_hover_text(format!("Cycle {}. Go to cycle…; click to type, Enter goes there. Back as far as cycle {}. {}",s.cycles,s.earliest,Self::transport_reason(s)));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button,response.enabled(),"Go to cycle…"));
                self.remember("Cycle",&response);
                let badge_width = if s.ahead > 0 {if compact {53.0} else {84.0}} else {0.0};
                let text = format!("cycle {}",if compact || s.cycles >= 1_000_000_000 {compact_count(s.cycles)} else {grouped(s.cycles)});
                let text_rect = egui::Rect::from_min_max(response.rect.min + egui::vec2(6.0,0.0),response.rect.max - egui::vec2(badge_width + 3.0,0.0));
                let galley = ui.painter().layout(text,egui::FontId::monospace(12.0),ui.visuals().text_color(),f32::INFINITY);
                ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(egui::pos2(text_rect.left(),text_rect.center().y-galley.size().y/2.0),galley,ui.visuals().text_color());
                if s.ahead > 0 {
                    let badge = egui::Rect::from_min_max(egui::pos2(response.rect.right()-badge_width,response.rect.top()+4.0),response.rect.max-egui::vec2(3.0,4.0));
                    ui.painter().rect_filled(badge,4.0,ui.visuals().selection.bg_fill);
                    let label = if compact {format!("+{}",compact_count(s.ahead))} else {format!("+{} ahead",compact_count(s.ahead))};
                    ui.painter().text(badge.center(),egui::Align2::CENTER_CENTER,label,egui::FontId::proportional(11.0),ui.visuals().selection.stroke.color);
                    ui.interact(badge,ui.id().with("ahead"),egui::Sense::hover()).on_hover_text(format!("{} cycles recorded ahead. {REPLAY}",s.ahead));
                }
                if response.clicked() { self.open_goto(s.cycles); }
            }
            let current = RATES.iter().find(|(_,r)| *r == s.rate).map_or_else(|| format!("{} cycles/s",short(s.rate.unwrap_or(0.0))),|(label,_)|label.to_string());
            let response = ui.add_enabled_ui(s.ready,|ui| egui::ComboBox::from_id_salt("rate").width(if compact {86.0} else {112.0}).truncate().selected_text(&current).show_ui(ui,|ui| self.rate_choices(ui,s.rate)).response).inner.on_hover_text(format!("{current}. {RATE}")).on_disabled_hover_text(Self::transport_reason(s));
            self.remember("Rate",&response);
            self.numeric_popup(ui,&response,Editor::Rate);
        }).response.rect
    }
    pub(crate) fn right_view_segment(
        &mut self,
        ui: &mut egui::Ui,
        s: Snapshot,
        compact: bool,
    ) -> Option<egui::Rect> {
        if ui.ctx().content_rect().width() < 1100.0 {
            return None;
        }
        let width = if compact { 136.0 } else { 360.0 };
        // Keep a right inset after fitting the navigation entry points.
        let inset = if compact { 6.0 } else { 8.0 };
        ui.add_space((ui.available_width() - width - inset).max(0.0));
        Some(self.view_segment(ui, s, compact))
    }
    pub(crate) fn view_segment(
        &mut self,
        ui: &mut egui::Ui,
        s: Snapshot,
        compact: bool,
    ) -> egui::Rect {
        ui.add_enabled_ui(s.ready,|ui| ui.horizontal(|ui| {
            self.view_control(ui,s,compact,(Icon::Values,"Values",self.fabric.values,VALUES,66.0));
            // One exclusive Wires | Tiles segment, without an internal gap.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                self.view_control(ui,s,compact,(Icon::Wires,"Wires",self.map == Map::Fabric,"Wires and sites of the die; those the design uses lit, its changes running along them.",60.0));
                self.view_control(ui,s,compact,(Icon::Tiles,"Tiles",self.map == Map::Grid,"Tiles and sites without wires; those the design uses lit.",56.0));
            });
            self.view_control(ui,s,compact,(Icon::Waves,"Waves",self.waves.open,WAVES,66.0));
            self.view_control(ui,s,compact,(Icon::Critical,"Critical path",self.critical.on,"Show nextpnr's estimated critical path and its source locations.",100.0));
        }).response.rect).inner
    }
    fn view_control(
        &mut self,
        ui: &mut egui::Ui,
        s: Snapshot,
        compact: bool,
        control: (Icon, &str, bool, &str, f32),
    ) {
        let (icon, label, selected, hint, width) = control;
        let response = icon_button(
            ui,
            icon,
            label,
            if compact { 26.0 } else { width },
            selected,
            !compact,
        )
        .on_hover_text(hint)
        .on_disabled_hover_text(format!("{hint} {}", Self::transport_reason(s)));
        self.remember(label, &response);
        if response.clicked() {
            match icon {
                Icon::Values => self.fabric.values = !self.fabric.values,
                Icon::Wires => self.map = Map::Fabric,
                Icon::Tiles => self.map = Map::Grid,
                Icon::Waves => self.waves.set_open(!self.waves.open),
                Icon::Critical => self.critical.on = !self.critical.on,
                _ => unreachable!(),
            }
        }
    }
    pub(crate) fn run_menu(&mut self, ui: &mut egui::Ui) {
        let s = self.toolbar_snapshot();
        self.run_pause(ui, s, false, 90.0);
        self.back(ui, s, true);
        self.step(ui, s, 110.0);
        ui.menu_button("Step size", |ui| self.step_choices(ui));
        if ui
            .add_enabled(s.ready && !s.running, egui::Button::new("Go to cycle…"))
            .on_disabled_hover_text(Self::transport_reason(s))
            .clicked()
        {
            self.open_goto(s.cycles);
            ui.close();
        }
        ui.add_enabled_ui(s.ready, |ui| {
            ui.menu_button("Rate", |ui| self.rate_choices(ui, s.rate));
        });
    }
    pub(crate) fn view_menu(&mut self, ui: &mut egui::Ui) {
        let ready = self.toolbar_snapshot().ready;
        ui.add_enabled_ui(ready,|ui| {
            ui.checkbox(&mut self.fabric.values,"Values").on_hover_text(VALUES);
            ui.selectable_value(&mut self.map,Map::Fabric,"Wires").on_hover_text("Wires and sites of the die; those the design uses lit, its changes running along them.");
            ui.selectable_value(&mut self.map,Map::Grid,"Tiles").on_hover_text("Tiles and sites without wires; those the design uses lit.");
            let mut open = self.waves.open;
            ui.checkbox(&mut open,"Waves").on_hover_text(WAVES);
            self.waves.set_open(open);
            ui.checkbox(&mut self.critical.on,"Critical path").on_hover_text("Show nextpnr's estimated critical path and its source locations.");
            ui.menu_button("Critical path clock",|ui| {
                let ready = self.ready();
                let timing = ready.as_ref().and_then(|s|s.timing.get());
                if let Some(Ok(paths)) = timing.map(|t|&t.result) {
                    if ui.selectable_label(self.critical.clock.is_none(),"Automatic").clicked() { self.critical.clock = None; }
                    for path in paths { if ui.selectable_label(self.critical.clock.as_deref() == Some(&path.label),&path.label).clicked() { self.critical.clock = Some(path.label.clone()); self.critical.on = true; } }
                } else { ui.label("Timing report is being prepared"); }
            });
        });
    }
}

fn compact_count(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let (scale, suffix) = [
        (1e18, "E"),
        (1e15, "P"),
        (1e12, "T"),
        (1e9, "G"),
        (1e6, "M"),
        (1e3, "k"),
    ]
    .into_iter()
    .find(|(scale, _)| n as f64 >= *scale)
    .unwrap();
    let value = format!("{:.2}", n as f64 / scale);
    format!(
        "{}{suffix}",
        value.trim_end_matches('0').trim_end_matches('.')
    )
}
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut text = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(' ');
        }
        text.push(c);
    }
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const LABELS: [&str; 14] = [
        "Design",
        "Peripherals",
        "New project",
        "Design name",
        "Back",
        "Run",
        "Step",
        "Step size",
        "Cycle",
        "Rate",
        "Values",
        "Wires",
        "Tiles",
        "Waves",
    ];
    pub(crate) fn snapshot() -> Snapshot {
        Snapshot {
            ready: true,
            running: false,
            cycles: 1_234_567,
            earliest: 1,
            ahead: 240,
            rate: Some(4.0),
        }
    }
    pub(crate) fn painted(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    painted(shape, texts);
                }
            }
            egui::Shape::Text(text) => texts.push((
                text.galley.text().into(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    pub(crate) fn assert_layout(
        layout: Rects,
        controls: &[(String, egui::Rect, bool)],
        size: egui::Vec2,
        project: bool,
    ) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert_row_control_geometry(controls);
        assert!(
            screen.contains_rect(layout.row),
            "row {:?} outside {screen:?}",
            layout.row
        );
        assert!(layout.row.height() <= 46.01, "{:?}", layout.row);
        let groups: Vec<_> = [
            Some(layout.navigation),
            layout.build,
            Some(layout.transport),
            Some(layout.time),
            layout.view,
        ]
        .into_iter()
        .flatten()
        .collect();
        for r in &groups {
            assert!(
                layout.row.contains_rect(*r),
                "group {r:?} outside {:?}",
                layout.row
            );
        }
        for pair in groups.windows(2) {
            assert!(pair[0].right() <= pair[1].left(), "overlap {pair:?}");
        }
        assert_eq!(layout.build.is_some(), project);
        assert_eq!(layout.view.is_some(), size.x >= 1100.0);
        for label in LABELS.into_iter().chain(["Critical path"]) {
            let matching: Vec<_> = controls
                .iter()
                .filter(|c| c.0 == label || (label == "Run" && c.0 == "Pause"))
                .collect();
            assert_eq!(matching.len(), 1, "{label}: {controls:?}");
            assert!(
                screen.contains_rect(matching[0].1),
                "clipped {label}: {:?}",
                matching[0]
            );
            if size.x == 1100.0
                && ["Values", "Wires", "Tiles", "Waves", "Critical path"].contains(&label)
            {
                assert_eq!(matching[0].1.width(), 26.0);
            }
        }
        for label in ["Project", "Chip", "Build", "Emulate"] {
            assert_eq!(
                controls.iter().filter(|c| c.0 == label).count(),
                usize::from(project),
                "{label}: {controls:?}"
            );
        }
        assert!(!controls.iter().any(|c|["Go to cycle","Custom input","Apply","Cancel"].contains(&c.0.as_str())),"unused fields: {controls:?}");
        for (i, a) in controls.iter().enumerate() {
            for b in &controls[i + 1..] {
                assert!(
                    !a.1.shrink(0.1).intersects(b.1.shrink(0.1)),
                    "control overlap: {a:?} {b:?}"
                );
            }
        }
    }

    fn assert_row_control_geometry(controls: &[(String, egui::Rect, bool)]) {
        let design = controls.iter().find(|c| c.0 == "Design").unwrap().1;
        let centre = design.center().y;
        for (label, rect, _) in controls {
            assert!(
                (rect.height() - design.height()).abs() <= 0.5,
                "{label} has height {}, expected {} pt: {controls:?}",
                rect.height(),
                design.height()
            );
            assert!(
                (rect.center().y - centre).abs() <= 0.5,
                "{label} has centre {}, expected {centre}: {controls:?}",
                rect.center().y
            );
        }
    }

    fn assert_row_painted_geometry(output: &egui::FullOutput, centre: f32) {
        fn frames(shape: &egui::Shape, rects: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        frames(shape, rects);
                    }
                }
                egui::Shape::Rect(shape)
                    if shape.rect.top() >= 0.0
                        && shape.rect.bottom() <= 46.0
                        && shape.rect.height() > 20.0
                        && shape.rect.height() < 40.0
                        && shape.rect.width() > 10.0 =>
                {
                    rects.push(shape.rect)
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        for shape in &output.shapes {
            frames(&shape.shape, &mut rects);
        }
        assert!(!rects.is_empty());
        for rect in &rects {
            assert!(
                (rect.height() - 26.0).abs() <= 0.5,
                "painted control height {}: {rects:?}",
                rect.height()
            );
            assert!(
                (rect.center().y - centre).abs() <= 0.5,
                "painted control centre {} != {centre}: {rects:?}",
                rect.center().y
            );
        }
    }

    struct Harness {
        app: App,
        ctx: egui::Context,
        size: egui::Vec2,
        time: f64,
        commands: mpsc::Receiver<Command>,
    }
    impl Harness {
        fn new(width: f32, theme: egui::ThemePreference) -> Self {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = App::new(
                &cc,
                "a_very_long_design_name_that_must_truncate".into(),
                None,
            );
            theme::install(&ctx, theme);
            app.toolbar_test_snapshot(snapshot());
            let (sender, commands) = mpsc::channel();
            app.commands = Some(sender);
            Self {
                app,
                ctx,
                size: egui::vec2(width, if width == 1500.0 { 950.0 } else { 800.0 }),
                time: 0.0,
                commands,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 0.1;
            self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    eframe::App::ui(&mut self.app, ui, &mut eframe::Frame::_new_kittest());
                },
            )
        }
        fn settle(&mut self) -> egui::FullOutput {
            self.frame(vec![]);
            self.frame(vec![]);
            self.frame(vec![])
        }
        fn control(&self, label: &str) -> (egui::Rect, bool) {
            let found = self
                .app
                .toolbar
                .controls
                .iter()
                .rev()
                .find(|c| c.0 == label)
                .unwrap_or_else(|| panic!("missing {label}: {:?}", self.app.toolbar.controls));
            (found.1, found.2)
        }
        fn click(&mut self, label: &str) {
            let pos = self.control(label).0.center();
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            self.settle();
        }
        fn key(&mut self, key: egui::Key) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        fn texts(&mut self) -> Vec<(String, egui::Rect)> {
            let mut texts = Vec::new();
            for shape in self.settle().shapes {
                painted(&shape.shape, &mut texts);
            }
            texts
        }
    }

    #[test]
    fn p29_row_controls_share_height_and_centre_in_both_hosts_and_themes() {
        let root = crate::test_temp::unique_dir("toolbar-p29-");
        std::fs::write(root.join("ecp5.toml"), "").unwrap();
        std::fs::write(root.join("top.v"), "module top; endmodule").unwrap();
        std::fs::write(root.join("top.lpf"), "").unwrap();
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            for width in [1500.0, 1100.0] {
                let mut h = Harness::new(width, theme);
                h.settle();
                for label in [
                    "Design",
                    "Peripherals",
                    "New project",
                    "Run",
                    "Step size",
                    "Rate",
                    "Values",
                ] {
                    let pos = h.control(label).0.center();
                    h.frame(vec![egui::Event::PointerMoved(pos)]);
                    let output = h.settle();
                    assert_row_control_geometry(&h.app.toolbar.controls);
                    assert_row_painted_geometry(&output, h.control("Design").0.center().y);
                }
                h.click("Cycle");
                let output = h.settle();
                assert_row_control_geometry(&h.app.toolbar.controls);
                assert_row_painted_geometry(&output, h.control("Design").0.center().y);

                let ctx = egui::Context::default();
                let cc = eframe::CreationContext::_new_kittest(ctx.clone());
                let mut project =
                    crate::project::ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
                theme::install(&ctx, theme);
                project.app().toolbar_test_snapshot(snapshot());
                let draw = |project: &mut crate::project::ProjectApp, events| {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                            events,
                            ..Default::default()
                        },
                        |ui| eframe::App::ui(project, ui, &mut eframe::Frame::_new_kittest()),
                    )
                };
                for _ in 0..3 {
                    draw(&mut project, vec![]);
                }
                // Change tabs through the actual controls; both use the shared row style.
                for tab in ["Project", "Chip"] {
                    let pos = project
                        .app()
                        .toolbar_test_evidence()
                        .1
                        .iter()
                        .find(|c| c.0 == tab)
                        .unwrap()
                        .1
                        .center();
                    for pressed in [true, false] {
                        draw(
                            &mut project,
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
                    for label in [
                        "Design",
                        "Peripherals",
                        "New project",
                        "Build",
                        "Rate",
                        "Step size",
                        tab,
                    ] {
                        let pos = project
                            .app()
                            .toolbar_test_evidence()
                            .1
                            .iter()
                            .find(|c| c.0 == label)
                            .unwrap()
                            .1
                            .center();
                        draw(&mut project, vec![egui::Event::PointerMoved(pos)]);
                        draw(&mut project, vec![]);
                        let output = draw(&mut project, vec![]);
                        let (_, controls) = project.app().toolbar_test_evidence();
                        assert_row_control_geometry(controls);
                        let centre = controls
                            .iter()
                            .find(|c| c.0 == "Design")
                            .unwrap()
                            .1
                            .center()
                            .y;
                        // Painted frames include the otherwise unlabelled Build split arrow.
                        assert_row_painted_geometry(&output, centre);
                    }
                    project.app().open_goto(snapshot().cycles);
                    draw(&mut project, vec![]);
                    let output = draw(&mut project, vec![]);
                    let (_, controls) = project.app().toolbar_test_evidence();
                    assert_row_control_geometry(controls);
                    let centre = controls
                        .iter()
                        .find(|c| c.0 == "Design")
                        .unwrap()
                        .1
                        .center()
                        .y;
                    assert_row_painted_geometry(&output, centre);
                    project.app().reset_toolbar_editors();
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn p29_start_navigation_inherits_the_same_button_style_and_height() {
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            for width in [1500.0, 1100.0] {
                let mut h = Harness::new(width, theme);
                h.app.commands = None;
                h.app.start_screen = true;
                for _ in 0..3 {
                    h.app.toolbar.controls.clear();
                    h.frame(vec![]);
                    assert_row_control_geometry(&h.app.toolbar.controls);
                    for label in ["Design", "Peripherals", "New project"] {
                        h.control(label);
                    }
                }
            }
        }
    }

    #[test]
    fn toolbar_plain_layout_widths_themes_states_counts_and_record_resizes() {
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let mut h = Harness::new(1500.0, theme);
            let temp = crate::test_temp::unique_dir("ecp5-toolbar-layout-");
            let path = temp.join("window.tsv");
            h.app.trace(&path).unwrap();
            for width in [1500.0, 1100.0] {
                h.size = egui::vec2(width, if width == 1500.0 { 950.0 } else { 800.0 });
                for (running, cycles, ahead) in [
                    (false, 1, 0),
                    (false, 1_234_567, 240),
                    (true, u64::MAX, u64::MAX),
                ] {
                    let mut s = snapshot();
                    s.running = running;
                    s.cycles = cycles;
                    s.ahead = ahead;
                    h.app.toolbar_test_snapshot(s);
                    let output = h.settle();
                    let (layout, controls) = h.app.toolbar_test_evidence();
                    assert_layout(layout, controls, h.size, false);
                    let mut texts = Vec::new();
                    for shape in output.shapes {
                        let mut text = Vec::new();
                        painted(&shape.shape, &mut text);
                        text.retain(|(_, rect)| rect.center().y <= 46.0);
                        for (_, rect) in &text {
                            assert!(
                                shape.clip_rect.expand(0.5).contains_rect(*rect),
                                "text clipped: {text:?}; clip {:?}",
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
                            .filter(|t| t.0.contains("a_very_long_design"))
                            .count(),
                        1
                    );
                    assert_eq!(
                        texts
                            .iter()
                            .filter(|t| ["Values", "Wires", "Tiles", "Waves", "Critical path"]
                                .contains(&t.0.trim()))
                            .count(),
                        if width == 1100.0 { 0 } else { 5 }
                    );
                }
            }
            drop(h);
            let record = std::fs::read_to_string(path).unwrap();
            assert_eq!(
                record
                    .lines()
                    .filter(|line| line.starts_with("# toolbar "))
                    .count(),
                2,
                "{record}"
            );
            std::fs::remove_dir_all(temp).unwrap();
        }
    }

    #[test]
    fn toolbar_split_step_custom_apply_cancel_and_goto_enter_escape() {
        let mut h = Harness::new(1100.0, egui::ThemePreference::Dark);
        h.settle();
        h.click("Step size");
        h.click("1k cycles");
        assert_eq!(h.app.steps(), 1000);
        assert!(h.commands.try_recv().is_err());
        h.click("Step");
        assert!(matches!(h.commands.try_recv(), Ok(Command::Step(1000))));
        h.click("Back");
        assert!(matches!(h.commands.try_recv(), Ok(Command::Back(1000))));
        h.click("Step size");
        h.click("Custom step");
        assert!(h.control("Custom input").1);
        h.app.toolbar.popup_text = "2.5k".into();
        h.settle();
        h.click("Apply");
        assert_eq!(h.app.steps(), 2500);
        assert!(h.commands.try_recv().is_err());
        h.click("Step size");
        h.click("Custom step");
        h.app.toolbar.popup_text = "77".into();
        h.settle();
        h.click("Cancel");
        assert_eq!(h.app.steps(), 2500);
        assert!(h.commands.try_recv().is_err());
        h.click("Cycle");
        assert_eq!(h.app.goto_text, "1234567");
        h.app.goto_text = "bad".into();
        h.key(egui::Key::Enter);
        assert!(h.app.toolbar.goto);
        assert!(h.app.toolbar.goto_error);
        assert!(h.commands.try_recv().is_err());
        assert!(h.texts().iter().any(|t| t.0.contains("Invalid cycle")));
        h.app.goto_text = "2.5M".into();
        h.key(egui::Key::Enter);
        assert!(matches!(
            h.commands.try_recv(),
            Ok(Command::GoTo(2_500_000))
        ));
        assert!(!h.app.toolbar.goto);
        h.settle();
        h.click("Cycle");
        h.key(egui::Key::Escape);
        assert!(!h.app.toolbar.goto);
        assert!(h.commands.try_recv().is_err());
    }

    #[test]
    fn p37_goto_click_away_and_escape_cancel_only_enter_seeks() {
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            for width in [1500.0, 1100.0] {
                let mut h = Harness::new(width, theme);
                h.settle();
                h.click("Cycle");
                h.app.goto_text = "99".into();
                h.settle();
                // Empty space is deliberately not focusable; focus loss alone misses it.
                let pos = egui::pos2(width - 12.0, h.size.y - 60.0);
                for pressed in [true, false] {
                    h.frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                h.settle();
                assert!(!h.app.toolbar.goto, "click-away must restore the readout");
                h.control("Cycle");
                assert!(h.commands.try_recv().is_err());

                h.click("Cycle");
                h.app.goto_text = "123".into();
                h.key(egui::Key::Escape);
                h.settle();
                assert!(!h.app.toolbar.goto);
                h.control("Cycle");
                assert!(h.commands.try_recv().is_err());

                h.click("Cycle");
                // Type into the focused editor instead of mutating its saved text.
                h.frame(vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }]);
                h.frame(vec![egui::Event::Text("2.5k".into())]);
                h.key(egui::Key::Enter);
                h.settle();
                assert!(matches!(h.commands.try_recv(), Ok(Command::GoTo(2500))));
                assert!(h.commands.try_recv().is_err());
                assert!(!h.app.toolbar.goto);
                h.control("Cycle");
            }
        }
    }

    #[test]
    fn toolbar_rate_custom_popup_preserves_rate_until_apply_while_running() {
        let mut h = Harness::new(1500.0, egui::ThemePreference::Light);
        let mut s = snapshot();
        s.running = true;
        h.app.toolbar_test_snapshot(s);
        h.settle();
        assert!(h.control("Rate").1);
        h.click("Rate");
        for (label, _) in RATES {
            assert!(h.control(label).1);
        }
        h.click("Custom rate");
        h.app.toolbar.popup_text = "0.5".into();
        h.settle();
        let texts = h.texts();
        assert_eq!(texts.iter().filter(|t| t.0 == "cycles/s").count(), 1);
        assert!(h.commands.try_recv().is_err());
        h.click("Apply");
        assert!(matches!(
            h.commands.try_recv(),
            Ok(Command::Rate(Some(0.5)))
        ));
        h.click("Rate");
        h.click("Custom rate");
        h.app.toolbar.popup_text = "12M".into();
        h.settle();
        h.click("Cancel");
        assert!(h.commands.try_recv().is_err());
    }

    #[test]
    fn toolbar_compact_view_toggles_tooltips_and_narrow_menu_fallback() {
        let mut h = Harness::new(1100.0, egui::ThemePreference::Dark);
        h.settle();
        h.click("Tiles");
        assert!(h.app.map == Map::Grid);
        h.click("Tiles");
        assert!(h.app.map == Map::Grid);
        h.click("Wires");
        assert!(h.app.map == Map::Fabric);
        h.click("Values");
        assert!(h.app.fabric.values);
        for (label, part) in [
            ("Values", "unknown grey-violet"),
            ("Wires", "Wires and sites"),
            ("Tiles", "without wires"),
            ("Waves", "W over a wire"),
            ("Critical path", "nextpnr"),
        ] {
            let rect = h.control(label).0;
            h.frame(vec![egui::Event::PointerMoved(rect.center())]);
            h.time += 1.0;
            assert!(
                h.texts().iter().any(|t| t.0.contains(part)),
                "{label}: {:?}",
                h.texts()
            );
        }
        h.size.x = 1000.0;
        h.settle();
        assert!(h.app.toolbar.layout.unwrap().view.is_none());
        assert!(!h.app.toolbar.controls.iter().any(|c| c.0 == "Values"));
        h.click("Design");
        assert!(h.texts().iter().any(|t| t.0 == "View"));
    }

    #[test]
    fn toolbar_new_project_button_emits_real_creation_and_respects_availability() {
        for width in [1500.0, 1100.0] {
            let mut h = Harness::new(width, egui::ThemePreference::Dark);
            h.settle();
            let design = h.control("Design").0;
            let peripherals = h.control("Peripherals").0;
            let new = h.control("New project").0;
            assert!(design.right() <= peripherals.left() && peripherals.right() <= new.left());
            let output = h.settle();
            assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::LineSegment {points,..} if new.contains(points[0]) && new.contains(points[1]))),"New project needs its vector icon");
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()=="New project" && !text.galley.elided)),"New project must keep its full visible label");
            assert!(h.control("New project").1);
            h.click("New project");
            assert!(matches!(
                h.app.creation_command.take(),
                Some(crate::new_project::Command::New(_))
            ));
            assert!(h.app.creation_busy);
            h.app.creation_available = false;
            h.settle();
            assert!(!h.control("New project").1);
            h.click("New project");
            assert!(h.app.creation_command.is_none());
            h.frame(vec![egui::Event::PointerMoved(egui::Pos2::ZERO)]);
            h.frame(vec![egui::Event::PointerMoved(new.center())]);
            h.time += 1.0;
            let texts = h.texts();
            assert!(
                texts
                    .iter()
                    .any(|t| t.0 == "Wait for the current build or emulation to finish"),
                "{texts:?}"
            );
        }
    }

    #[test]
    fn toolbar_design_creation_disabled_tooltip_and_plain_has_no_build_menu() {
        let mut h = Harness::new(1100.0, egui::ThemePreference::Light);
        h.app.creation_available = false;
        h.settle();
        h.click("Design");
        for label in ["New project…", "Start from a demo…"] {
            let (rect, enabled) = h.control(label);
            assert!(!enabled);
            h.frame(vec![egui::Event::PointerMoved(rect.center())]);
            h.time += 1.0;
            assert!(h
                .texts()
                .iter()
                .any(|t| t.0 == "Wait for the current build or emulation to finish"));
        }
        let texts = h.texts();
        let shown: Vec<_> = texts.iter().map(|t| t.0.trim()).collect();
        for label in [
            "Open…", "Recent", "Demos", "Close", "Theme", "Run", "View", "Help",
        ] {
            assert!(shown.contains(&label), "{shown:?}");
        }
        assert!(!shown.contains(&"Build"));
        assert!(!shown.iter().any(|t| t.contains("Shift+N")));
    }
}
