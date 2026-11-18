//! Multi-tab source and log editor with line-number gutter, message line
//! highlighting, syntax colours, dirty tracking, Save (Cmd/Ctrl+S), and Revert.
use anyhow::{Context, Result};
use ecp5_emu::{
    flow::{
        new_project::{lpf_lines, port_naming, ports, Naming, Port, PortNaming, Resource},
        Project,
    },
    sources::{self, Span},
};
use eframe::egui::{self, Color32, FontId, RichText};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Tab {
    path: PathBuf,
    label: String,
    text: String,
    saved_text: String,
    read_only: bool,
    highlight_line: Option<u32>,
    scroll_to_line: Option<u32>,
    select_span: Option<Span>,
    cursor: Option<egui::text::CCursorRange>,
    syntax: Option<Syntax>,
    highlighting: HighlightCache,
}

impl Tab {
    fn syntax(&mut self) -> &Syntax {
        if self.syntax.is_none() {
            self.highlighting.update(&self.text);
        }
        self.syntax
            .get_or_insert_with(|| Syntax::from_lines(&self.highlighting.lines))
    }

    fn dirty(&self) -> bool {
        !self.read_only && self.text != self.saved_text
    }

    fn save(&mut self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        fs::write(&self.path, &self.text)
            .with_context(|| format!("writing {}", self.path.display()))?;
        self.saved_text = self.text.clone();
        Ok(())
    }

    fn revert(&mut self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        self.text = text.clone();
        self.saved_text = text;
        self.syntax = None;
        Ok(())
    }
}

#[derive(Default)]
pub struct Editor {
    tabs: Vec<Tab>,
    active: usize,
    error: Option<String>,
    pub probe_status: Option<String>,
    pub watch_probe: bool,
    pub(super) inline_values: bool,
    /// Sum of layouter calls during the last show_values frame.
    pub(super) layouter_ms: f64,
    /// The layouter saw a change to a buffer rendered in an earlier frame.
    pub(super) laid_out_edited: bool,
    scripted_hover: Option<(PathBuf, u32, u32)>,
    pointer_hover: Option<Probe>,
    /// Complete top-module port names, including bus bits, when an index supplies them.
    /// The current source index does not retain this metadata; None means unknown.
    pub(super) lpf_ports: Option<BTreeSet<String>>,
    pin_insert: PinInsert,
    notes: Vec<String>,
    pub watch_values: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub identifier: String,
}

impl Editor {
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.active = 0;
        self.error = None;
        self.probe_status = None;
        self.watch_probe = false;
        self.scripted_hover = None;
        self.pointer_hover = None;
        self.lpf_ports = None;
        self.pin_insert = PinInsert::default();
        drop(self.take_notes());
        self.watch_values.clear();
    }

    pub fn is_dirty(&self, path: &Path) -> bool {
        self.tabs
            .iter()
            .any(|t| same_file(&t.path, path) && t.dirty())
    }

    /// Reload a clean open tab after a library action patches its file; keep focus and edits elsewhere.
    pub(super) fn refresh_clean(&mut self, path: &Path) -> Result<()> {
        if let Some(tab) = self.tabs.iter_mut().find(|t| same_file(&t.path, path)) {
            anyhow::ensure!(!tab.dirty(), "Save {} before refreshing it", tab.label);
            tab.revert()?;
        }
        Ok(())
    }

    /// Drain after show_values: `for note in self.editor.take_notes() { self.record(&note); }`.
    /// ProjectApp wires this after its editor panel; editor actions never write records directly.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    fn pin_ports(&mut self) -> Vec<TopPort> {
        let path = &self.tabs[self.active].path;
        let Some(parent) = path.parent() else {
            return Vec::new();
        };
        let mut project = None;
        for root in parent.ancestors() {
            let manifest = root.join("ecp5.toml");
            if root == parent || manifest.is_file() {
                if let Ok(candidate) = Project::open(root) {
                    if same_file(&candidate.root.join(&candidate.constraints), path) {
                        project = Some(candidate);
                        break;
                    }
                }
                if manifest.is_file() {
                    break;
                }
            }
        }
        let Some(project) = project else {
            return Vec::new();
        };
        for file in &project.verilog {
            let path = project.root.join(file);
            if !is_verilog(&path) {
                continue;
            }
            let ports =
                if let Some(tab) = self.tabs.iter_mut().find(|tab| same_file(&tab.path, &path)) {
                    tab.highlighting.update(&tab.text);
                    top_ports(&tab.highlighting, &project.top)
                } else if let Ok(text) = fs::read_to_string(&path) {
                    let mut cache = HighlightCache::new(Language::for_path(&path));
                    cache.update(&text);
                    top_ports(&cache, &project.top)
                } else {
                    None
                };
            if let Some(ports) = ports {
                return ports;
            }
        }
        Vec::new()
    }

    fn lpf_strip(&mut self, ui: &mut egui::Ui) -> Option<PinRequest> {
        if self.tabs[self.active].highlighting.language != Language::Lpf {
            return None;
        }
        let mut request = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            let button = ui.add_enabled(
                !self.tabs[self.active].read_only,
                egui::Button::new("Insert pin…"),
            );
            if button.clicked() {
                self.pin_insert.ports = self.pin_ports();
                self.pin_insert.cursor = self.tabs[self.active].cursor;
            }
            egui::Popup::menu(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(520.0)
                .show(|ui| {
                    ui.label(RichText::new("ULX3S pins (Radiona)").strong());
                    for resource in PIN_RESOURCES {
                        if ui
                            .selectable_label(
                                self.pin_insert.resource == resource,
                                resource_label(resource),
                            )
                            .clicked()
                        {
                            self.pin_insert.resource = resource;
                            self.pin_insert.port = resource_port(resource).into();
                        }
                    }
                    ui.separator();
                    self.pin_insert.show_ports(ui);
                    let tab = &mut self.tabs[self.active];
                    let lines = lpf_lines(
                        self.pin_insert.resource,
                        pin_naming(self.pin_insert.resource, &self.pin_insert.port),
                    );
                    let mut allowed = false;
                    match lines {
                        Ok(lines) => {
                            let bits = lpf_balls(&lines).len();
                            if let Some(warning) = self.pin_insert.width_warning(bits) {
                                ui.colored_label(ui.visuals().warn_fg_color, warning);
                            }
                            match check_pin_insert(tab, &lines) {
                                Ok(()) => allowed = true,
                                Err(error) => {
                                    ui.colored_label(ui.visuals().warn_fg_color, error.to_string());
                                }
                            }
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    ui.monospace(lines);
                                });
                        }
                        Err(error) => {
                            ui.weak(error.to_string());
                        }
                    }
                    if ui
                        .add_enabled(allowed, egui::Button::new("Insert"))
                        .clicked()
                    {
                        request = Some(PinRequest {
                            resource: self.pin_insert.resource,
                            port: self.pin_insert.port.trim().into(),
                            cursor: self.pin_insert.cursor,
                        });
                        ui.close();
                    }
                });
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                ui.add(egui::Label::new(RichText::new(LPF_STRIP).small().weak()).wrap());
            });
        });
        ui.separator();
        request
    }

    pub fn active_path(&self) -> Option<&Path> {
        self.tabs.get(self.active).map(|t| t.path.as_path())
    }

    pub(super) fn active_language(&self) -> Option<Language> {
        self.tabs.get(self.active).map(|t| t.highlighting.language)
    }

    /// Opens a file (or switches to its existing tab), optionally highlighting
    /// and scrolling to a 1-based line number.
    pub fn open(
        &mut self,
        path: &Path,
        label: String,
        read_only: bool,
        line: Option<u32>,
    ) -> Result<()> {
        if let Some(index) = self.tabs.iter().position(|t| same_file(&t.path, path)) {
            self.active = index;
            let tab = &mut self.tabs[index];
            if read_only && !tab.dirty() {
                if let Ok(fresh) = fs::read_to_string(&tab.path) {
                    tab.text = fresh.clone();
                    tab.saved_text = fresh;
                    tab.syntax = None;
                }
            }
            if let Some(line) = line {
                tab.highlight_line = Some(line);
                tab.scroll_to_line = Some(line);
            }
            self.error = None;
            return Ok(());
        }
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        self.tabs.push(Tab {
            path: path.to_path_buf(),
            label,
            saved_text: text.clone(),
            text,
            read_only,
            highlight_line: line,
            scroll_to_line: line,
            select_span: None,
            cursor: None,
            syntax: None,
            highlighting: HighlightCache::new(Language::for_path(path)),
        });
        self.active = self.tabs.len() - 1;
        self.error = None;
        Ok(())
    }

    pub fn select_span(&mut self, span: Span) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.highlight_line = Some(span.line);
            tab.scroll_to_line = Some(span.line);
            tab.select_span = Some(span);
        }
    }

    /// Script clicks read the editor buffer, including unsaved changes.
    pub fn probe_at(&self, path: &Path, line: u32, column: u32) -> Option<Probe> {
        let tab = self.tabs.iter().find(|tab| same_file(&tab.path, path))?;
        Some(Probe {
            path: tab.path.clone(),
            line,
            column,
            identifier: sources::identifier_at(&tab.text, line, column),
        })
    }

    pub fn text(&self, path: &Path) -> Option<&str> {
        self.tabs
            .iter()
            .find(|tab| same_file(&tab.path, path))
            .map(|tab| tab.text.as_str())
    }

    /// Select an identifier and probe the buffer through the script's path.
    pub fn select_probe(&mut self, path: &Path, line: u32, column: u32) -> Option<Probe> {
        let probe = self.probe_at(path, line, column)?;
        self.select_span(Span {
            file: path.file_name()?.to_string_lossy().into_owned().into(),
            line,
            column: Some(column),
            end_line: line,
            end_column: Some(column + probe.identifier.chars().count() as u32),
        });
        Some(probe)
    }

    /// Replaces a 1-based line in the editor buffer (opening the file first if
    /// needed) and marks the buffer dirty.
    pub fn replace_line(
        &mut self,
        path: &Path,
        label: String,
        line: u32,
        replacement: &str,
    ) -> Result<()> {
        self.open(path, label, false, Some(line))?;
        let tab = &mut self.tabs[self.active];
        let had_trailing_newline = tab.text.ends_with('\n');
        let mut lines: Vec<String> = tab.text.split('\n').map(str::to_string).collect();
        if had_trailing_newline && lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let idx = line.saturating_sub(1) as usize;
        if idx >= lines.len() {
            lines.resize(idx + 1, String::new());
        }
        lines[idx] = replacement.to_string();
        tab.text = lines.join("\n");
        tab.syntax = None;
        if had_trailing_newline || !tab.text.is_empty() {
            tab.text.push('\n');
        }
        Ok(())
    }

    /// Saves the currently active tab if dirty and writable. Returns the path
    /// saved, if any.
    pub fn save_active(&mut self) -> Result<Option<PathBuf>> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Ok(None);
        };
        if !tab.dirty() {
            return Ok(None);
        }
        tab.save()?;
        self.error = None;
        Ok(Some(tab.path.clone()))
    }

    /// Saves every dirty editable tab before a build. Returns how many files
    /// were written and whether `ecp5.toml` was among them.
    pub fn save_all_dirty(&mut self) -> Result<(usize, bool)> {
        let mut count = 0;
        let mut manifest_saved = false;
        for tab in &mut self.tabs {
            if tab.dirty() {
                tab.save()?;
                count += 1;
                if tab.path.file_name().is_some_and(|n| n == "ecp5.toml") {
                    manifest_saved = true;
                }
            }
        }
        self.error = None;
        Ok((count, manifest_saved))
    }

    #[cfg(test)]
    pub fn show(&mut self, ui: &mut egui::Ui, paths: &super::DisplayPaths) -> Option<Probe> {
        self.show_values(ui, paths, |_, _, _| {
            ValueView::unavailable("Emulate this project to see values")
        })
    }

    pub fn script_hover(&mut self, path: &Path, line: u32, column: u32) {
        self.scripted_hover = Some((path.into(), line, column));
        self.pointer_hover = None;
    }

    pub fn value_probe_at(&mut self, path: &Path, line: u32, column: u32) -> Option<Probe> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| same_file(&tab.path, path))?;
        Some(Self::hover_at(tab, line, column).unwrap_or_else(|| Probe {
            path: path.into(),
            line,
            column,
            identifier: String::new(),
        }))
    }

    /// Both scripted and pointer hovers read the cached lexer for this buffer.
    fn hover_at(tab: &mut Tab, line: u32, column: u32) -> Option<Probe> {
        let language = tab.highlighting.language;
        let mut column = column;
        let identifier = if matches!(language, Language::Vhdl | Language::Lpf) {
            tab.highlighting.update(&tab.text);
            let source_line = tab.highlighting.lines.get(line.checked_sub(1)? as usize)?;
            let character = column.checked_sub(1)? as usize;
            let token = source_line.tokens.iter().find(|t| {
                (t.class == TokenClass::Identifier
                    || (language == Language::Lpf
                        && matches!(t.class, TokenClass::Keyword | TokenClass::String)))
                    && t.start <= character
                    && character < t.end
            })?;
            if language == Language::Lpf {
                column = token.start as u32 + 1;
            }
            source_line.text[token.bytes.clone()]
                .trim_matches('\\')
                .to_owned()
        } else {
            tab.syntax().identifier(line, column)?
        };
        Some(Probe {
            path: tab.path.clone(),
            line,
            column,
            identifier,
        })
    }

    pub fn show_values(
        &mut self,
        ui: &mut egui::Ui,
        paths: &super::DisplayPaths,
        mut value: impl FnMut(&Probe, bool, bool) -> ValueView,
    ) -> Option<Probe> {
        self.layouter_ms = 0.0;
        self.laid_out_edited = false;
        if let Some(err) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, paths.text(err));
        }
        if self.tabs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Select a source or log file on the left to open it.").weak(),
                );
            });
            return None;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }

        // Save shortcut (Cmd+S / Ctrl+S).
        let save_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
        if ui.input_mut(|i| i.consume_shortcut(&save_shortcut)) {
            if let Err(e) = self.save_active() {
                self.error = Some(format!("{e:#}"));
            }
        }

        // Tab strip.
        let mut close_idx = None;
        ui.horizontal_wrapped(|ui| {
            for (i, tab) in self.tabs.iter().enumerate() {
                let title = if tab.dirty() {
                    format!("{} *", tab.label)
                } else {
                    tab.label.clone()
                };
                let selected = i == self.active;
                let (tab_response, close_response) = tab_button(ui, &paths.text(&title), selected);
                if tab_response.clicked() {
                    self.active = i;
                }
                if close_response.clicked()
                    || tab_response.clicked_by(egui::PointerButton::Middle)
                    || close_response.clicked_by(egui::PointerButton::Middle)
                {
                    close_idx = Some(i);
                }
                (tab_response | close_response).on_hover_text(paths.path(&tab.path));
                ui.separator();
            }
        });
        if let Some(idx) = close_idx {
            self.tabs.remove(idx);
            if self.tabs.is_empty() {
                self.active = 0;
                return None;
            }
            if idx < self.active {
                self.active -= 1;
            } else if self.active >= self.tabs.len() {
                self.active = self.tabs.len() - 1;
            }
        }
        ui.separator();

        if let Some(status) = &self.probe_status {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [(ui.available_width() - 150.0).max(50.0), 22.0],
                    egui::Label::new(paths.text(status)).truncate(),
                )
                .on_hover_text(paths.text(status));
                if ui
                    .button("Watch in Waves")
                    .on_hover_text("Watch the probed nets (W when not editing text)")
                    .clicked()
                {
                    self.watch_probe = true;
                }
            });
        }
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::W)) {
            self.watch_probe = true;
        }

        // Active file toolbar.
        let tab = &mut self.tabs[self.active];
        let dirty = tab.dirty();
        ui.horizontal(|ui| {
            ui.label(RichText::new(paths.text(&tab.label)).strong());
            if tab.read_only {
                ui.label(RichText::new("(read-only)").weak().small());
            } else {
                if ui
                    .add_enabled(dirty, egui::Button::new("Save"))
                    .on_hover_text("Save file (Cmd/Ctrl+S)")
                    .clicked()
                {
                    if let Err(e) = tab.save() {
                        self.error = Some(format!("{e:#}"));
                    } else {
                        self.error = None;
                    }
                }
                if ui
                    .add_enabled(dirty, egui::Button::new("Revert"))
                    .on_hover_text("Discard edits and reload from disk")
                    .clicked()
                {
                    if let Err(e) = tab.revert() {
                        self.error = Some(format!("{e:#}"));
                    } else {
                        self.error = None;
                    }
                }
            }
            if is_verilog(&tab.path) {
                ui.separator();
                ui.checkbox(&mut self.inline_values, "Inline values")
                    .on_hover_text(
                        "Current published values of assignment targets; not expression results",
                    );
            }
            if let Some(line) = tab.highlight_line {
                ui.separator();
                ui.label(RichText::new(format!("line {line}")).weak().small());
            }
        });
        ui.separator();

        // Code view with line-number gutter.
        let pin_request = self.lpf_strip(ui);
        let tab = &mut self.tabs[self.active];
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font_id)).max(14.0);
        let line_count = tab.text.split('\n').count().max(1);
        let digits = line_count.to_string().len().max(3);
        let gutter_width = (digits as f32) * (font_id.size * 0.65) + 16.0;
        let highlight = tab.highlight_line;
        let pending_scroll = tab.scroll_to_line.take();
        let text_color = ui.visuals().text_color();
        let highlight_bg = Color32::from_rgba_unmultiplied(180, 130, 40, 65);
        let gutter_bg = ui.visuals().faint_bg_color;
        let gutter_fg = ui.visuals().weak_text_color();
        let mut probe = None;

        egui::ScrollArea::both()
            .id_salt(("editor_scroll", &tab.path))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let total_height = (line_count as f32) * row_height + 16.0;
                let avail_w = ui.available_width();
                ui.horizontal_top(|ui| {
                    let (gutter_rect, _) = ui.allocate_exact_size(
                        egui::vec2(gutter_width, total_height.max(ui.available_height())),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(gutter_rect, 0.0, gutter_bg);

                    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                        let started = std::time::Instant::now();
                        self.laid_out_edited |= tab
                            .highlighting
                            .rendered
                            .as_ref()
                            .is_some_and(|cached| cached.galley.text() != buf.as_str());
                        let galley = if tab.read_only {
                            let job = build_layout_job(
                                buf.as_str(),
                                &font_id,
                                text_color,
                                highlight,
                                highlight_bg,
                            );
                            ui.fonts_mut(|f| f.layout_job(job))
                        } else {
                            tab.highlighting.galley(
                                ui,
                                buf.as_str(),
                                &font_id,
                                text_color,
                                highlight.map(|line| (line, highlight_bg)),
                            )
                        };
                        self.layouter_ms += started.elapsed().as_secs_f64() * 1000.0;
                        galley
                    };

                    let readonly_text = paths.text(&tab.text);
                    let mut readonly_buf = readonly_text.as_str();
                    let edit = if tab.read_only {
                        egui::TextEdit::multiline(&mut readonly_buf)
                    } else {
                        egui::TextEdit::multiline(&mut tab.text)
                    }
                    .id_salt(("editor_text", &tab.path))
                    .font(font_id.clone())
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .desired_width((avail_w - gutter_width - 8.0).max(400.0))
                    .layouter(&mut layouter);

                    let mut output = edit.show(ui);
                    if let Some(request) = &pin_request {
                        match insert_pin(tab, &mut output.state, request, ui.input(|i| i.time)) {
                            Ok(note) => {
                                self.notes.push(note);
                                output.cursor_range = tab.cursor;
                                output.response.mark_changed();
                                output.state.clone().store(ui.ctx(), output.response.id);
                                output.response.request_focus();
                                ui.ctx().request_repaint();
                            }
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                    if let Some(span) = tab.select_span.take() {
                        let (start, end) = span_range(&tab.text, &span);
                        output
                            .state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(start),
                                egui::text::CCursor::new(end),
                            )));
                        tab.cursor = output.state.cursor.char_range();
                        output.state.store(ui.ctx(), output.response.id);
                        output.response.request_focus();
                        ui.ctx().request_repaint();
                    } else {
                        let range = output.cursor_range;
                        let moved = range != tab.cursor;
                        let clicked = output.response.clicked() || output.response.drag_stopped();
                        if !output.response.changed()
                            && (clicked || (moved && output.response.has_focus()))
                        {
                            if let Some(range) = range {
                                let cursor = if range.is_empty() {
                                    range.primary.index
                                } else {
                                    range.sorted_cursors()[0].index
                                };
                                let (line, column) = line_column(&tab.text, cursor.into());
                                tab.highlight_line = Some(line);
                                probe = Some(Probe {
                                    path: tab.path.clone(),
                                    line,
                                    column,
                                    identifier: sources::identifier_at(&tab.text, line, column),
                                });
                            }
                        }
                        tab.cursor = range;
                    }
                    if output.response.changed() {
                        tab.syntax = None;
                    }
                    let source = matches!(
                        tab.highlighting.language,
                        Language::Verilog | Language::SystemVerilog | Language::Vhdl
                    );
                    let mut hover = None;
                    let lpf = tab.highlighting.language == Language::Lpf;
                    if source || lpf {
                        if let Some(pointer) = output.response.hover_pos() {
                            self.scripted_hover = None;
                            self.pointer_hover = pointer_coordinates(
                                &tab.text,
                                &output.galley,
                                output.galley_pos,
                                pointer,
                            )
                            .and_then(|(line, column)| Self::hover_at(tab, line, column));
                            hover = self.pointer_hover.clone();
                        } else if let Some((path, line, column)) = &self.scripted_hover {
                            if same_file(&tab.path, path) {
                                hover = Self::hover_at(tab, *line, *column);
                                if hover.is_none() && source {
                                    let probe = Probe {
                                        path: path.clone(),
                                        line: *line,
                                        column: *column,
                                        identifier: String::new(),
                                    };
                                    value(&probe, tab.dirty(), true);
                                }
                            }
                        }
                    }
                    if hover.is_none()
                        && self.scripted_hover.is_none()
                        && output.response.is_tooltip_open()
                    {
                        hover = self
                            .pointer_hover
                            .as_ref()
                            .filter(|probe| same_file(&probe.path, &tab.path))
                            .cloned();
                    }
                    if let Some(hover) = hover {
                        let help = if lpf {
                            lpf_help_at(tab, hover.line, hover.column, self.lpf_ports.as_ref())
                        } else {
                            None
                        };
                        let view = source.then(|| value(&hover, tab.dirty(), true));
                        if help.is_some() || view.is_some() {
                            let mut anchor = <egui::Response as Clone>::clone(&output.response);
                            let start =
                                character_offset(&tab.text, hover.line, Some(hover.column), false);
                            anchor.rect = output
                                .galley
                                .pos_from_cursor(egui::text::CCursor::new(start))
                                .translate(output.galley_pos.to_vec2());
                            if lpf {
                                anchor.rect |= output
                                    .galley
                                    .pos_from_cursor(egui::text::CCursor::new(
                                        start + hover.identifier.chars().count(),
                                    ))
                                    .translate(output.galley_pos.to_vec2());
                            }
                            let tooltip = if self.scripted_hover.is_some() {
                                egui::Tooltip::for_widget(&anchor)
                            } else {
                                egui::Tooltip::for_enabled(&anchor)
                            };
                            tooltip.show(|ui| {
                                if let Some(help) = &help {
                                    help.show(ui);
                                } else if let Some(view) = &view {
                                    view.show(ui);
                                    if !view.targets.is_empty()
                                        && ui.button("Watch in Waves").clicked()
                                    {
                                        self.watch_values = view.targets.clone();
                                    }
                                }
                            });
                        }
                    }
                    let galley = &output.galley;
                    let origin = output.galley_pos;

                    // Draw line numbers aligned with each laid-out row.
                    let first = galley
                        .rows
                        .partition_point(|row| origin.y + row.rect().max.y < ui.clip_rect().min.y);
                    let last = galley
                        .rows
                        .partition_point(|row| origin.y + row.rect().min.y <= ui.clip_rect().max.y);
                    for (idx, placed_row) in galley.rows.iter().enumerate().take(last).skip(first) {
                        let line_no = idx + 1;
                        if is_verilog(&tab.path) && self.inline_values {
                            let syntax = tab.syntax();
                            if let Some(assigned) =
                                syntax.assignments.get(&(line_no as u32)).cloned()
                            {
                                let mut labels = Vec::new();
                                for (name, line, column) in assigned {
                                    let target = Probe {
                                        path: tab.path.clone(),
                                        line,
                                        column,
                                        identifier: name.clone(),
                                    };
                                    let view = value(&target, tab.dirty(), false);
                                    labels.push(format!("{name}: {}", view.inline()));
                                }
                                let position = inline_position(origin, placed_row.rect());
                                let annotation = ui.painter().layout_no_wrap(
                                    labels.join("; "),
                                    font_id.clone(),
                                    gutter_fg,
                                );
                                ui.expand_to_include_rect(egui::Rect::from_min_size(
                                    position,
                                    annotation.size(),
                                ));
                                ui.painter().galley(position, annotation, gutter_fg);
                            }
                        }
                        let y = origin.y + placed_row.rect().min.y;
                        let row_h = placed_row.rect().height().max(row_height);
                        let is_hl = highlight == Some(line_no as u32);
                        if is_hl {
                            let hl_gutter = egui::Rect::from_min_size(
                                egui::pos2(gutter_rect.min.x, y),
                                egui::vec2(gutter_rect.width(), row_h),
                            );
                            ui.painter().rect_filled(hl_gutter, 0.0, highlight_bg);
                        }
                        let num_pos = egui::pos2(gutter_rect.max.x - 6.0, y);
                        ui.painter().text(
                            num_pos,
                            egui::Align2::RIGHT_TOP,
                            line_no.to_string(),
                            font_id.clone(),
                            if is_hl { text_color } else { gutter_fg },
                        );
                    }

                    if let Some(target) = pending_scroll {
                        let idx = target.saturating_sub(1) as usize;
                        let target_rect = galley
                            .rows
                            .get(idx)
                            .map(|r| r.rect().translate(origin.to_vec2()))
                            .unwrap_or_else(|| {
                                let y = origin.y + (idx as f32) * row_height;
                                egui::Rect::from_min_size(
                                    egui::pos2(origin.x, y),
                                    egui::vec2(100.0, row_height),
                                )
                            });
                        ui.scroll_to_rect(target_rect, Some(egui::Align::Center));
                    }
                });
            });
        probe
    }
}

const LPF_STRIP: &str = "An LPF connects your top module's ports to the ULX3S pins. Hover a keyword for help. Pin names come from the ULX3S board (Radiona); use Insert pin… to add them.";
const PIN_RESOURCES: [Resource; 14] = [
    Resource::Clock,
    Resource::Leds,
    Resource::Buttons,
    Resource::Switches,
    Resource::Uart,
    Resource::Oled,
    Resource::Hdmi,
    Resource::Sd,
    Resource::Audio,
    Resource::Sdram,
    Resource::Adc,
    Resource::Rtc,
    Resource::Flash,
    Resource::Header,
];

fn resource_label(resource: Resource) -> &'static str {
    match resource {
        Resource::Clock => "25 MHz clock",
        Resource::Leds => "LEDs D0..D7",
        Resource::Buttons => "Buttons (PWR, FIRE1, FIRE2, UP, DOWN, LEFT, RIGHT)",
        Resource::Switches => "DIP switches",
        Resource::Uart => "UART TX/RX",
        Resource::Oled => "OLED (SSD1331 SPI)",
        Resource::Hdmi => "HDMI / GPDI",
        Resource::Sd => "SD card",
        Resource::Audio => "Audio jack",
        Resource::Sdram => "SDRAM",
        Resource::Adc => "ADC (MAX11125)",
        Resource::Rtc => "RTC (MCP7940N)",
        Resource::Flash => "SPI flash",
        Resource::Header => "Header pins (inputs only)",
    }
}

fn resource_port(resource: Resource) -> &'static str {
    match port_naming(resource) {
        PortNaming::Single { default } => default,
        PortNaming::Prefix { common } => common.unwrap_or(""),
    }
}

fn resource_name(resource: Resource) -> &'static str {
    match resource {
        Resource::Clock => "clock",
        Resource::Leds => "leds",
        Resource::Buttons => "buttons",
        Resource::Switches => "switches",
        Resource::Uart => "uart",
        Resource::Oled => "oled",
        Resource::Hdmi => "hdmi",
        Resource::Sd => "sd",
        Resource::Audio => "audio",
        Resource::Sdram => "sdram",
        Resource::Adc => "adc",
        Resource::Rtc => "rtc",
        Resource::Flash => "flash",
        Resource::Header => "header",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TopPort {
    name: String,
    width: Option<usize>,
}

struct PinInsert {
    resource: Resource,
    port: String,
    ports: Vec<TopPort>,
    cursor: Option<egui::text::CCursorRange>,
}

impl Default for PinInsert {
    fn default() -> Self {
        Self {
            resource: Resource::Clock,
            port: resource_port(Resource::Clock).into(),
            ports: Vec::new(),
            cursor: None,
        }
    }
}

impl PinInsert {
    fn inserted_ports(&self) -> Vec<Port> {
        let mut ports = ports(self.resource);
        let rename = match pin_naming(self.resource, &self.port) {
            Naming::Default => None,
            Naming::Base(new) | Naming::Prefix(new) => Some((resource_port(self.resource), new)),
        };
        if let Some((old, new)) = rename {
            for port in &mut ports {
                port.name = format!("{new}{}", port.name.strip_prefix(old).unwrap());
            }
        }
        ports
    }

    fn port_status(&self, port: &Port) -> (bool, String) {
        let width = port.width.unwrap_or(1);
        let Some(top) = self.ports.iter().find(|top| top.name == port.name) else {
            return (false, format!("× {} ({width} bits; missing)", port.name));
        };
        let status = match top.width {
            Some(actual) if actual == width => format!("✔ {} ({width} bits)", port.name),
            Some(actual) => format!(
                "⚠ {} (needs {width} bits; top has {actual} bits)",
                port.name
            ),
            None => format!("⚠ {} (needs {width} bits; top width unknown)", port.name),
        };
        (true, status)
    }

    fn show_ports(&mut self, ui: &mut egui::Ui) {
        match port_naming(self.resource) {
            PortNaming::Single { .. } => {
                ui.horizontal(|ui| {
                    ui.label("Port base name");
                    ui.text_edit_singleline(&mut self.port);
                });
                if self.ports.is_empty() {
                    ui.weak("No top-module ports found. Enter a port base name.");
                } else {
                    ui.weak("Top-module ports (choose one):");
                    ui.horizontal_wrapped(|ui| {
                        for port in &self.ports {
                            let width = port
                                .width
                                .map_or_else(|| "width unknown".into(), |n| format!("{n} bits"));
                            if ui
                                .selectable_label(
                                    self.port == port.name,
                                    format!("{} ({width})", port.name),
                                )
                                .clicked()
                            {
                                self.port = port.name.clone();
                            }
                        }
                    });
                }
            }
            PortNaming::Prefix { common } => {
                if common.is_some() {
                    ui.horizontal(|ui| {
                        ui.label("Prefix");
                        ui.text_edit_singleline(&mut self.port);
                    });
                }
                let ports = self.inserted_ports();
                let present = ports.iter().filter(|port| self.port_status(port).0).count();
                ui.weak(format!("{present} of {} ports are in top", ports.len()));
                egui::ScrollArea::vertical()
                    .id_salt("insert_pin_ports")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for port in &ports {
                            let (_, status) = self.port_status(port);
                            ui.monospace(status);
                        }
                    });
            }
        }
    }

    fn width_warning(&self, bits: usize) -> Option<String> {
        if !matches!(port_naming(self.resource), PortNaming::Single { .. }) {
            return None;
        }
        let port = self.ports.iter().find(|p| p.name == self.port.trim())?;
        match port.width {
            Some(width) if width != bits => Some(format!(
                "Width mismatch: {} needs {bits} bits, but port {} has {width} bits.",
                resource_label(self.resource),
                port.name
            )),
            None => Some(format!(
                "The width of port {} is unknown; {} needs {bits} bits.",
                port.name,
                resource_label(self.resource)
            )),
            _ => None,
        }
    }
}

/// Share the naming choice between the live preview and the queued insertion.
fn pin_naming(resource: Resource, value: &str) -> Naming<'_> {
    let value = value.trim();
    match port_naming(resource) {
        PortNaming::Single { .. } => Naming::Base(value),
        PortNaming::Prefix {
            common: Some(common),
        } if value != common => Naming::Prefix(value),
        PortNaming::Prefix { .. } => Naming::Default,
    }
}

struct PinRequest {
    resource: Resource,
    port: String,
    cursor: Option<egui::text::CCursorRange>,
}

/// Use the shared lexer for both previews and duplicate detection, ignoring comments.
fn lpf_balls(text: &str) -> BTreeSet<String> {
    let mut cache = HighlightCache::new(Language::Lpf);
    cache.update(text);
    let tokens: Vec<_> = cache
        .lines
        .iter()
        .flat_map(|line| {
            line.tokens
                .iter()
                .filter(|t| t.class != TokenClass::Comment)
                .map(|t| &line.text[t.bytes.clone()])
        })
        .collect();
    tokens
        .windows(2)
        .filter_map(|pair| {
            (pair[0] == "SITE")
                .then(|| {
                    pair[1]
                        .strip_prefix('"')?
                        .strip_suffix('"')
                        .map(str::to_owned)
                })
                .flatten()
        })
        .collect()
}

fn check_pin_insert(tab: &Tab, lines: &str) -> Result<()> {
    anyhow::ensure!(!tab.read_only, "This file is read-only.");
    let existing = lpf_balls(&tab.text);
    let balls = lpf_balls(lines);
    let duplicates: Vec<_> = existing.intersection(&balls).cloned().collect();
    anyhow::ensure!(
        duplicates.is_empty(),
        "Already assigned: {}. No lines inserted.",
        duplicates.join(", ")
    );
    Ok(())
}

/// Commit one buffer edit and bracket it with explicit egui undo points.
fn insert_pin(
    tab: &mut Tab,
    state: &mut egui::text_edit::TextEditState,
    request: &PinRequest,
    time: f64,
) -> Result<String> {
    let lines = lpf_lines(
        request.resource,
        pin_naming(request.resource, &request.port),
    )?;
    check_pin_insert(tab, &lines)?;
    let index = request
        .cursor
        .map_or_else(
            || tab.text.chars().count(),
            |range| usize::from(range.primary.index),
        )
        .min(tab.text.chars().count());
    let cursor = request
        .cursor
        .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(index)));
    let mut undoer = state.undoer();
    undoer.add_undo(&(cursor, tab.text.clone()));
    let byte = tab
        .text
        .char_indices()
        .nth(index)
        .map_or(tab.text.len(), |(byte, _)| byte);
    let prefix = if byte > 0 && !tab.text[..byte].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let inserted = format!("{prefix}{lines}");
    tab.text.insert_str(byte, &inserted);
    let cursor =
        egui::text::CCursorRange::one(egui::text::CCursor::new(index + inserted.chars().count()));
    tab.cursor = Some(cursor);
    tab.select_span = None;
    tab.syntax = None;
    state.cursor.set_char_range(Some(cursor));
    undoer.feed_state(time, &(cursor, tab.text.clone()));
    undoer.add_undo(&(cursor, tab.text.clone()));
    state.set_undoer(undoer);
    Ok(format!(
        "# lpf insert {} {} {}",
        resource_name(request.resource),
        request.port,
        lines.lines().count()
    ))
}

/// Extract names and packed widths from one module's cached Verilog tokens.
/// Unresolved expressions stay unknown rather than guessing a scalar width.
fn top_ports(cache: &HighlightCache, top: &str) -> Option<Vec<TopPort>> {
    let tokens: Vec<_> = cache
        .lines
        .iter()
        .flat_map(|line| {
            line.tokens
                .iter()
                .filter(|t| {
                    !matches!(
                        t.class,
                        TokenClass::Comment | TokenClass::Directive | TokenClass::String
                    )
                })
                .map(|t| (t.class, &line.text[t.bytes.clone()]))
        })
        .collect();
    let module = tokens
        .windows(2)
        .position(|pair| pair[0].1 == "module" && pair[1].1.trim_matches('\\') == top)?;
    let end = tokens[module + 2..]
        .iter()
        .position(|t| t.1 == "endmodule")
        .map(|i| module + 2 + i)?;
    let header_end = tokens[module..end].iter().position(|t| t.1 == ";")? + module;
    let mut start = module + 2;
    if tokens.get(start).is_some_and(|t| t.1 == "#") {
        start += 1;
        let mut depth = 0;
        while start < header_end {
            match tokens[start].1 {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        start += 1;
                        break;
                    }
                }
                _ => {}
            }
            start += 1;
        }
    }
    let header = &tokens[start..header_end];
    let ansi = header
        .iter()
        .any(|t| matches!(t.1, "input" | "output" | "inout"));
    let mut ports = Vec::new();
    if ansi {
        port_declarations(header, &mut ports);
    } else {
        let names: BTreeSet<_> = header
            .iter()
            .filter(|t| t.0 == TokenClass::Identifier)
            .map(|t| t.1.trim_matches('\\'))
            .collect();
        let body_start = header_end + 1;
        let mut statement = body_start;
        let mut nested = false;
        for i in body_start..end {
            match tokens[i].1 {
                "task" | "function" => nested = true,
                "endtask" | "endfunction" => {
                    nested = false;
                    statement = i + 1;
                }
                ";" => {
                    if !nested
                        && tokens
                            .get(statement)
                            .is_some_and(|t| matches!(t.1, "input" | "output" | "inout"))
                    {
                        port_declarations(&tokens[statement..i], &mut ports);
                    }
                    statement = i + 1;
                }
                _ => {}
            }
        }
        ports.retain(|p| names.contains(p.name.as_str()));
    }
    Some(ports)
}

fn port_declarations(tokens: &[(TokenClass, &str)], ports: &mut Vec<TopPort>) {
    let mut width = Some(1_usize);
    let mut direction = false;
    let mut name = None;
    let mut i = usize::from(tokens.first().is_some_and(|t| t.1 == "("));
    while i < tokens.len() {
        match tokens[i].1 {
            "input" | "output" | "inout" => {
                direction = true;
                width = Some(1);
            }
            "[" => {
                if let Some(end) = tokens[i + 1..]
                    .iter()
                    .position(|t| t.1 == "]")
                    .map(|n| i + 1 + n)
                {
                    let range = &tokens[i + 1..end];
                    let dimension = if range.len() == 3 && range[1].1 == ":" {
                        range[0]
                            .1
                            .replace('_', "")
                            .parse::<usize>()
                            .ok()
                            .zip(range[2].1.replace('_', "").parse::<usize>().ok())
                            .and_then(|(a, b)| a.abs_diff(b).checked_add(1))
                    } else {
                        None
                    };
                    width = width.zip(dimension).and_then(|(a, b)| a.checked_mul(b));
                    i = end;
                } else {
                    width = None;
                }
            }
            "," | ")" | ";" => {
                if direction {
                    if let Some(name) = name.take() {
                        ports.push(TopPort { name, width });
                    }
                }
            }
            "wire" | "reg" | "logic" | "signed" | "unsigned" | "tri" | "var" => {}
            "=" => {
                while i + 1 < tokens.len() && !matches!(tokens[i + 1].1, "," | ")" | ";") {
                    i += 1;
                }
            }
            _ if direction && tokens[i].0 == TokenClass::Identifier => {
                name = Some(tokens[i].1.trim_matches('\\').into())
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(name) = name {
        if direction {
            ports.push(TopPort { name, width });
        }
    }
}

// Keep these sentences identical to the explanatory header in project LPFs.
const LPF_OVERVIEW: &str =
    "LPF (Lattice Preference File) maps top-level ports to package pins and sets I/O.";
const LPF_LOCATE: &str =
    "LOCATE COMP \"port\" SITE \"ball\"; connects a port to a physical package ball.";
const LPF_BALL: &str = "A ball name is a CABGA381 coordinate: row letter plus column number, as seen on the package underside; for example, G2 is row G, column 2.";
const LPF_IOBUF: &str = "IOBUF PORT \"port\" IO_TYPE=... PULLMODE=... DRIVE=...;";
const LPF_IO_TYPE: &str = "IO_TYPE selects the electrical standard: LVCMOS33 means 3.3 V on ULX3S.";
const LPF_PULLMODE: &str =
    "PULLMODE selects a weak internal pull: UP to 1, DOWN to 0, NONE disables it.";
const LPF_DRIVE: &str = "DRIVE sets output drive strength in mA; it does not set an input's level.";
const LPF_FREQUENCY: &str =
    "FREQUENCY PORT \"clk_25mhz\" 25 MHZ; tells timing analysis the clock rate.";
const LPF_BUS: &str = "Bus bits are separate ports, written in quotes, for example \"led[0]\".";

const LPF_LOCATE_EXAMPLE: &str = "LOCATE COMP \"led[0]\" SITE \"G2\";";
const LPF_IOBUF_EXAMPLE: &str = "IOBUF PORT \"led[0]\" IO_TYPE=LVCMOS33 PULLMODE=NONE DRIVE=4;";
const LPF_FREQUENCY_EXAMPLE: &str = "FREQUENCY PORT \"clk_25mhz\" 25 MHZ;";

struct LpfHelp {
    sentences: Vec<String>,
    example: String,
}

impl LpfHelp {
    fn new(sentences: &[&str], example: impl Into<String>) -> Self {
        Self {
            sentences: sentences.iter().map(|s| (*s).into()).collect(),
            example: example.into(),
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        for sentence in &self.sentences {
            ui.label(sentence);
        }
        ui.monospace(&self.example);
    }
}

/// Read the highlighting cache, including preceding lines for split statements.
/// `ports` is a complete top-module snapshot; an absent index cannot rule out a port.
fn lpf_help_at(
    tab: &mut Tab,
    line: u32,
    column: u32,
    ports: Option<&BTreeSet<String>>,
) -> Option<LpfHelp> {
    if tab.highlighting.language != Language::Lpf {
        return None;
    }
    tab.highlighting.update(&tab.text);
    let line_index = line.checked_sub(1)? as usize;
    let source_line = tab.highlighting.lines.get(line_index)?;
    let character = column.checked_sub(1)? as usize;
    let index = source_line
        .tokens
        .iter()
        .position(|t| t.start <= character && character < t.end)?;
    let token = &source_line.tokens[index];
    let word = &source_line.text[token.bytes.clone()];
    let previous = tab.highlighting.lines[..line_index]
        .iter()
        .flat_map(|line| line.tokens.iter().map(move |token| (line, token)))
        .chain(
            source_line.tokens[..index]
                .iter()
                .map(|token| (source_line, token)),
        )
        .rev()
        .find(|(_, token)| token.class != TokenClass::Comment)
        .map(|(line, token)| &line.text[token.bytes.clone()]);
    if token.class == TokenClass::String {
        let name = word.strip_prefix('"')?.strip_suffix('"')?;
        return match previous {
            Some("SITE") => lpf_ball_help(name),
            Some("COMP" | "PORT") => {
                let mut help = LpfHelp::new(&[LPF_BUS], LPF_LOCATE_EXAMPLE);
                if let Some(ports) = ports {
                    help.sentences.push(if ports.contains(name) {
                        format!("\"{name}\" is a port in the project's top module.")
                    } else {
                        format!("\"{name}\" is not a port in the project's top module.")
                    });
                }
                Some(help)
            }
            _ => None,
        };
    }
    if !matches!(token.class, TokenClass::Keyword | TokenClass::Identifier) {
        return None;
    }
    Some(match word {
        "LOCATE" | "COMP" => LpfHelp::new(&[LPF_OVERVIEW, LPF_LOCATE], LPF_LOCATE_EXAMPLE),
        "SITE" => LpfHelp::new(&[LPF_LOCATE, LPF_BALL], LPF_LOCATE_EXAMPLE),
        "IOBUF" => LpfHelp::new(&[LPF_OVERVIEW, LPF_IOBUF], LPF_IOBUF_EXAMPLE),
        "PORT" => LpfHelp::new(&[LPF_IOBUF, LPF_FREQUENCY], LPF_IOBUF_EXAMPLE),
        "IO_TYPE" => LpfHelp::new(&[LPF_IO_TYPE], LPF_IOBUF_EXAMPLE),
        "PULLMODE" => LpfHelp::new(&[LPF_PULLMODE], LPF_IOBUF_EXAMPLE),
        "DRIVE" => LpfHelp::new(&[LPF_DRIVE], LPF_IOBUF_EXAMPLE),
        "FREQUENCY" | "MHZ" => LpfHelp::new(&[LPF_FREQUENCY], LPF_FREQUENCY_EXAMPLE),
        "UP" | "DOWN" | "NONE" => LpfHelp::new(
            &[LPF_PULLMODE],
            format!("IOBUF PORT \"button\" IO_TYPE=LVCMOS33 PULLMODE={word};"),
        ),
        "BLOCK" => LpfHelp::new(
            &["BLOCK excludes selected paths from timing analysis; nextpnr accepts reset and asynchronous path forms."],
            "BLOCK RESETPATHS;",
        ),
        "SYSCONFIG" => LpfHelp::new(
            &["SYSCONFIG sets device configuration options; rarely needed in ULX3S projects."],
            "SYSCONFIG COMPRESS_CONFIG=ON;",
        ),
        "BANK" => LpfHelp::new(
            &["BANK selects an I/O bank for preferences; nextpnr infers bank voltage from I/O types instead."],
            "BANK 6 VCCIO 3.3 V;",
        ),
        "VCCIO" => LpfHelp::new(
            &["VCCIO specifies an I/O bank's supply voltage; it does not change the board's physical power supply."],
            "BANK 6 VCCIO 3.3 V;",
        ),
        _ if word.starts_with("LVCMOS") || matches!(word, "LVTTL33" | "LVDS" | "LVDS25") => {
            LpfHelp::new(&[LPF_IO_TYPE], format!("IOBUF PORT \"led[0]\" IO_TYPE={word};"))
        }
        _ => return None,
    })
}

/// Package-coordinate help is separate so the board LPF can later add pin labels here.
/// A coordinate's shape alone does not establish its function on a board.
fn lpf_ball_help(ball: &str) -> Option<LpfHelp> {
    let split = ball.find(|c: char| c.is_ascii_digit())?;
    let (row, column) = ball.split_at(split);
    if row.is_empty() || !row.bytes().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let column = column.parse::<u32>().ok().filter(|&n| n > 0)?;
    let mut help = LpfHelp::new(&[LPF_BALL], LPF_LOCATE_EXAMPLE);
    help.sentences
        .push(format!("{ball}: row {row}, column {column}."));
    Some(help)
}

/// Publication text is retained across redraws; only requested bits are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValueView {
    pub rows: Vec<String>,
    pub detail: Vec<String>,
    pub cycle: Option<u64>,
    pub edges: u64,
    pub published: u64,
    pub running: bool,
    pub targets: Vec<u32>,
    pub warning: Option<String>,
}

pub(crate) const X_LABEL: &str = "unknown, high-impedance or not available here";

impl ValueView {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            rows: Vec::new(),
            detail: vec![message.into()],
            cycle: None,
            edges: 0,
            published: 0,
            running: false,
            targets: Vec::new(),
            warning: None,
        }
    }

    pub fn displayed(&self) -> String {
        if let [row] = self.rows.as_slice() {
            row.split_once(" = ")
                .map_or(row.as_str(), |(_, value)| value)
                .to_owned()
        } else {
            self.rows.join("; ")
        }
    }

    fn inline(&self) -> String {
        let mut text = if let Some(cycle) = self.cycle {
            format!("{} (at cycle {cycle}, as published)", self.displayed())
        } else {
            self.detail.join("; ")
        };
        if let Some(warning) = &self.warning {
            text.push_str(&format!("; {warning}; values from the running build"));
        }
        text
    }

    fn show(&self, ui: &mut egui::Ui) {
        for row in &self.rows {
            ui.monospace(row);
        }
        if let Some(cycle) = self.cycle {
            ui.label(format!("at cycle {cycle}, as published"));
            ui.weak(format!(
                "{}; edge {} (half-cycle {})",
                if self.running { "running" } else { "paused" },
                self.edges,
                self.edges % 2
            ));
        }
        for detail in &self.detail {
            ui.label(detail);
        }
        if let Some(warning) = &self.warning {
            ui.colored_label(Color32::YELLOW, warning);
            ui.weak("Values belong to the running build; edited code is not being simulated.");
        }
        if self.cycle.is_some() {
            ui.weak("Watch in Waves for edge-by-edge values.");
        }
    }

    /// Trace::note owns the '# ' prefix. An unavailable state has no '=' or cycle.
    pub fn note(&self, file: &str, probe: &Probe) -> String {
        let name = if probe.identifier.is_empty() {
            "-"
        } else {
            &probe.identifier
        };
        let mut note = if let Some(cycle) = self.cycle {
            format!(
                "value {file}:{}:{} {name} = {} @ {cycle}",
                probe.line,
                probe.column,
                self.displayed()
            )
        } else {
            format!(
                "value {file}:{}:{} {name} -> {}",
                probe.line,
                probe.column,
                self.detail.join("; ")
            )
        };
        if let Some(warning) = &self.warning {
            note.push_str(&format!("; {warning}"));
        }
        note
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValueBit {
    pub name: String,
    pub index: Option<u32>,
    /// One entry per physical target. None means a missing piece/value entry.
    pub alternatives: Vec<(u32, Option<u8>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValueRow {
    pub name: String,
    pub bits: Vec<ValueBit>,
}

/// Descending bits, one nibble at a time: there is no whole-bus integer limit.
fn bus_value(bits: &[Option<u8>]) -> (String, Option<u64>) {
    let mut digits = Vec::new();
    for nibble in bits.rchunks(4) {
        let digit = if nibble.iter().any(Option::is_none) {
            '?'
        } else if nibble.iter().any(|b| b.is_some_and(|b| b > 1)) {
            'x'
        } else {
            let n = nibble.iter().fold(0, |n, b| (n << 1) | b.unwrap());
            char::from_digit(u32::from(n), 16).unwrap()
        };
        digits.push(digit);
    }
    digits.reverse();
    let mut hex = String::from("0x");
    for (i, digit) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(4) {
            hex.push('_');
        }
        hex.push(*digit);
    }
    let decimal = (bits.len() <= 64 && bits.iter().all(|b| matches!(b, Some(0 | 1)))).then(|| {
        bits.iter()
            .fold(0u64, |n, b| (n << 1) | u64::from(b.unwrap()))
    });
    (hex, decimal)
}

pub(crate) fn format_values(rows: &[ValueRow]) -> (Vec<String>, Vec<String>) {
    let mut display = Vec::new();
    let mut detail = Vec::new();
    let mut has_x = false;
    for row in rows {
        let mut bits = Vec::new();
        let mut missing = Vec::new();
        for bit in &row.bits {
            // Distinct physical targets are explicit, even when their bytes agree.
            let value = match bit.alternatives.as_slice() {
                [(_, value)] => *value,
                _ => None,
            };
            if bit.alternatives.is_empty() || bit.alternatives.iter().any(|(_, v)| v.is_none()) {
                missing.push(bit.name.clone());
            }
            for (_, value) in &bit.alternatives {
                has_x |= value.is_some_and(|v| v > 1);
            }
            if bit.alternatives.len() > 1 {
                detail.push(format!(
                    "{} alternatives: {}",
                    bit.name,
                    bit.alternatives
                        .iter()
                        .map(|(net, value)| {
                            format!(
                                "net {net} = {}",
                                match value {
                                    Some(0) => "0",
                                    Some(1) => "1",
                                    Some(_) => "x",
                                    None => "?",
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            bits.push(value);
        }
        if let (Some(high), Some(low)) = (
            row.bits.first().and_then(|b| b.index),
            row.bits.last().and_then(|b| b.index),
        ) {
            let (hex, decimal) = bus_value(&bits);
            display.push(format!(
                "{} = {hex}{} (bits {high}..{low} found)",
                row.name,
                decimal.map_or_else(String::new, |n| format!(" ({n})"))
            ));
        } else {
            let value = match bits.first().copied().flatten() {
                Some(0) => "0",
                Some(1) => "1",
                Some(_) => "x",
                None => "?",
            };
            display.push(format!("{} = {value}", row.name));
        }
        if !missing.is_empty() {
            detail.push(format!("missing: {}", missing.join(", ")));
        }
    }
    if has_x {
        detail.push(format!("x: {X_LABEL}"));
    }
    (display, detail)
}

fn is_verilog(path: &Path) -> bool {
    matches!(
        Language::for_path(path),
        Language::Verilog | Language::SystemVerilog
    )
}

fn pointer_coordinates(
    text: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    pointer: egui::Pos2,
) -> Option<(u32, u32)> {
    let position = pointer - origin;
    let row = galley.rows.get(
        galley
            .rows
            .partition_point(|row| row.rect().max.y < position.y),
    )?;
    if !row.rect().contains(position.to_pos2()) {
        return None;
    }
    let cursor = galley.cursor_from_pos(position);
    let mut index = usize::from(cursor.index);
    // cursor_from_pos rounds to a caret; hover follows the glyph beneath it.
    if galley.pos_from_cursor(cursor).min.x > position.x && index > 0 {
        index -= 1;
    }
    Some(line_column(text, index))
}

fn inline_position(origin: egui::Pos2, row: egui::Rect) -> egui::Pos2 {
    origin + egui::vec2(row.max.x + 20.0, row.min.y)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenClass {
    Plain,
    Keyword,
    Identifier,
    Number,
    String,
    Comment,
    Directive,
    SystemTask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Language {
    Verilog,
    SystemVerilog,
    Vhdl,
    Lpf,
    Toml,
    Plain,
}

impl Language {
    pub(super) fn for_path(path: &Path) -> Self {
        match path.extension().and_then(|s| s.to_str()) {
            Some("v") => Self::Verilog,
            Some("sv" | "svh") => Self::SystemVerilog,
            Some("vhd" | "vhdl") => Self::Vhdl,
            Some("lpf") => Self::Lpf,
            Some("toml") => Self::Toml,
            _ => Self::Plain,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LexerState {
    #[default]
    Normal,
    BlockComment,
    String {
        quote: char,
        triple: bool,
        escaped: bool,
    },
    BasedDigits,
}

struct HighlightToken {
    bytes: std::ops::Range<usize>,
    start: usize,
    end: usize,
    class: TokenClass,
}

struct SyntaxLine {
    text: String,
    incoming: LexerState,
    outgoing: LexerState,
    tokens: Vec<HighlightToken>,
    layout: Option<LineLayout>,
}

impl SyntaxLine {
    /// The single lexer used by both colours and the live-value analysis.
    /// Spans use bytes for egui and characters for source probe coordinates.
    fn lex(text: &str, incoming: LexerState, language: Language) -> Self {
        let chars: Vec<_> = text.chars().collect();
        let bytes: Vec<_> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        let mut state = incoming;
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() && language != Language::Plain {
            let start = i;
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            let class;
            if state == LexerState::BlockComment
                || (state == LexerState::Normal && c == '/' && next == Some('*'))
            {
                if state != LexerState::BlockComment {
                    i += 2;
                }
                state = LexerState::BlockComment;
                while i < chars.len() {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        i += 2;
                        state = LexerState::Normal;
                        break;
                    }
                    i += 1;
                }
                class = TokenClass::Comment;
            } else if matches!(state, LexerState::String { .. })
                || c == '"'
                || (language == Language::Toml && c == '\'')
            {
                let (quote, triple, mut escaped) = match state {
                    LexerState::String {
                        quote,
                        triple,
                        escaped,
                    } => (quote, triple, escaped),
                    _ => {
                        let triple =
                            language == Language::Toml && chars.get(i..i + 3) == Some(&[c, c, c]);
                        i += if triple { 3 } else { 1 };
                        (c, triple, false)
                    }
                };
                state = LexerState::String {
                    quote,
                    triple,
                    escaped,
                };
                while i < chars.len() {
                    if escaped {
                        escaped = false;
                        i += 1;
                    } else if chars[i] == '\n' && !triple {
                        state = LexerState::Normal;
                        break;
                    } else if chars[i] == '\\'
                        && language != Language::Vhdl
                        && (language != Language::Toml || quote != '\'')
                    {
                        escaped = true;
                        i += 1;
                    } else if chars[i] == quote
                        && (!triple || chars.get(i..i + 3) == Some(&[quote, quote, quote]))
                    {
                        if language == Language::Vhdl && chars.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        i += if triple { 3 } else { 1 };
                        state = LexerState::Normal;
                        break;
                    } else {
                        i += 1;
                    }
                }
                if state != LexerState::Normal && triple {
                    state = LexerState::String {
                        quote,
                        triple,
                        escaped,
                    };
                } else {
                    state = LexerState::Normal;
                }
                class = TokenClass::String;
            } else if state == LexerState::BasedDigits {
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i += 1;
                }
                if i < chars.len() {
                    state = LexerState::Normal;
                    while chars.get(i).is_some_and(|c| {
                        c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | 'z' | 'Z' | '?' | '_')
                    }) {
                        i += 1;
                    }
                }
                if i == start {
                    continue;
                }
                class = TokenClass::Number;
            } else if c.is_whitespace() {
                i += 1;
                continue;
            } else if (language != Language::Vhdl && c == '/' && next == Some('/'))
                || (language == Language::Vhdl && c == '-' && next == Some('-'))
                || (matches!(language, Language::Lpf | Language::Toml) && c == '#')
            {
                i = chars.len() - usize::from(chars.last() == Some(&'\n'));
                class = TokenClass::Comment;
            } else if language == Language::Toml
                && c == '['
                && text[..bytes[start]].trim().is_empty()
            {
                i = chars
                    .iter()
                    .enumerate()
                    .skip(i)
                    .find(|(_, c)| **c == '\n' || **c == '#')
                    .map_or(chars.len(), |(i, _)| i);
                class = TokenClass::Keyword;
            } else if language == Language::Vhdl && c == '\'' {
                let attribute = tokens
                    .iter()
                    .rev()
                    .find(|t: &&HighlightToken| t.class != TokenClass::Comment)
                    .is_some_and(|t| {
                        t.class == TokenClass::Identifier
                            || matches!(&text[t.bytes.clone()], ")" | "]")
                    });
                if !attribute && chars.get(i + 2) == Some(&'\'') && chars.get(i + 1) != Some(&'\n')
                {
                    i += 3;
                    class = TokenClass::String;
                } else {
                    i += 1;
                    class = TokenClass::Plain;
                }
            } else if let Some(end) = vhdl_bit_string(language, &chars, i) {
                i = end;
                class = TokenClass::Number;
            } else if c == '`' && language != Language::Vhdl {
                i += 1;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
                {
                    i += 1;
                }
                class = TokenClass::Directive;
            } else if c == '\\' {
                i += 1;
                if language == Language::Vhdl {
                    while chars.get(i).is_some_and(|c| !matches!(c, '\\' | '\n')) {
                        i += 1;
                    }
                    i += usize::from(chars.get(i) == Some(&'\\'));
                } else {
                    while chars.get(i).is_some_and(|c| !c.is_whitespace()) {
                        i += 1;
                    }
                }
                class = TokenClass::Identifier;
            } else if c.is_ascii_digit() || c == '\'' {
                i += 1;
                while chars.get(i).is_some_and(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '_' | '\'' | '?' | '.')
                        || (matches!(c, '+' | '-')
                            && matches!(chars.get(i - 1), Some('e' | 'E'))
                            && !chars[start..i].contains(&'\''))
                }) {
                    i += 1;
                }
                if language == Language::Vhdl && chars.get(i) == Some(&'#') {
                    i += 1;
                    while chars
                        .get(i)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
                    {
                        i += 1;
                    }
                    i += usize::from(chars.get(i) == Some(&'#'));
                    if chars.get(i).is_some_and(|c| matches!(c, 'e' | 'E')) {
                        i += 1;
                        i += usize::from(chars.get(i).is_some_and(|c| matches!(c, '+' | '-')));
                        while chars
                            .get(i)
                            .is_some_and(|c| c.is_ascii_digit() || *c == '_')
                        {
                            i += 1;
                        }
                    }
                } else if let Some(quote) = chars[start..i].iter().position(|c| *c == '\'') {
                    let mut radix = start + quote + 1;
                    if chars.get(radix).is_some_and(|c| matches!(c, 's' | 'S')) {
                        radix += 1;
                    }
                    if radix + 1 == i
                        && chars.get(radix).is_some_and(|c| {
                            matches!(c, 'b' | 'B' | 'o' | 'O' | 'd' | 'D' | 'h' | 'H')
                        })
                    {
                        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                            i += 1;
                        }
                        if i == chars.len() {
                            state = LexerState::BasedDigits;
                        }
                        while chars.get(i).is_some_and(|c| {
                            c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | 'z' | 'Z' | '?' | '_')
                        }) {
                            i += 1;
                        }
                    }
                }
                class = TokenClass::Number;
            } else if c.is_ascii_alphabetic() || matches!(c, '_' | '$') {
                i += 1;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
                {
                    i += 1;
                }
                let word = &text[bytes[start]..bytes[i]];
                class = if c == '$' {
                    TokenClass::SystemTask
                } else if language_keyword(language, word)
                    || (language == Language::Lpf
                        && matches!(
                            word,
                            "LOCATE"
                                | "COMP"
                                | "SITE"
                                | "IOBUF"
                                | "PORT"
                                | "IO_TYPE"
                                | "FREQUENCY"
                                | "BLOCK"
                                | "SYSCONFIG"
                                | "BANK"
                                | "VCCIO"
                        ))
                    || (language == Language::Toml && matches!(word, "true" | "false"))
                {
                    TokenClass::Keyword
                } else {
                    TokenClass::Identifier
                };
            } else {
                i += 1;
                if let Some(n) = next {
                    if matches!(
                        (c, n),
                        ('<', '=')
                            | ('>', '=')
                            | ('=', '=')
                            | ('!', '=')
                            | ('&', '&')
                            | ('|', '|')
                            | ('<', '<')
                            | ('>', '>')
                            | ('+', '=')
                            | ('-', '=')
                            | ('*', '=')
                            | ('/', '=')
                            | (':', ':')
                            | (':', '=')
                            | ('=', '>')
                            | ('/', '>')
                    ) {
                        i += 1;
                        if matches!(c, '=' | '!') && chars.get(i) == Some(&'=') {
                            i += 1;
                        }
                    }
                }
                class = TokenClass::Plain;
            }
            tokens.push(HighlightToken {
                bytes: bytes[start]..bytes[i],
                start,
                end: i,
                class,
            });
        }
        Self {
            text: text.into(),
            incoming,
            outgoing: state,
            tokens,
            layout: None,
        }
    }
}

struct LineLayout {
    background: Color32,
    galley: Arc<egui::Galley>,
}

#[derive(PartialEq)]
struct LayoutStyle {
    font: FontId,
    color: Color32,
    dark: bool,
    pixels_per_point: f32,
}

struct CachedGalley {
    style: LayoutStyle,
    selected: Option<(u32, Color32)>,
    font_epoch: Arc<egui::Galley>,
    galley: Arc<egui::Galley>,
}

impl SyntaxLine {
    fn append_sections(
        &self,
        job: &mut egui::text::LayoutJob,
        style: &LayoutStyle,
        background: Color32,
        offset: usize,
        len: usize,
    ) {
        let palette = crate::theme::syntax_colours(style.dark);
        let normal = egui::TextFormat {
            font_id: style.font.clone(),
            color: style.color,
            background,
            ..Default::default()
        };
        let mut end = 0;
        for token in &self.tokens {
            let color = match token.class {
                TokenClass::Keyword => palette.keyword,
                TokenClass::Number => palette.number,
                TokenClass::String => palette.string,
                TokenClass::Comment => palette.comment,
                TokenClass::Directive => palette.directive,
                TokenClass::SystemTask => palette.system_task,
                TokenClass::Identifier | TokenClass::Plain => continue,
            };
            let start = token.bytes.start.min(len);
            layout_section(job, offset + end..offset + start, normal.clone());
            end = token.bytes.end.min(len);
            layout_section(
                job,
                offset + start..offset + end,
                egui::TextFormat {
                    color,
                    ..normal.clone()
                },
            );
        }
        layout_section(job, offset + end..offset + len, normal);
    }
}

struct HighlightCache {
    language: Language,
    lines: Vec<SyntaxLine>,
    rendered: Option<CachedGalley>,
    #[cfg(test)]
    relexed: usize,
    #[cfg(test)]
    laid_out: usize,
}

impl HighlightCache {
    fn new(language: Language) -> Self {
        Self {
            language,
            lines: Vec::new(),
            rendered: None,
            #[cfg(test)]
            relexed: 0,
            #[cfg(test)]
            laid_out: 0,
        }
    }

    /// Reuse the matching prefix and shifted suffix. Only changed lines and
    /// the suffix whose incoming lexer state changed need lexing again.
    fn update(&mut self, text: &str) -> bool {
        #[cfg(test)]
        {
            self.relexed = 0;
        }
        let mut lines: Vec<_> = text.split_inclusive('\n').collect();
        if text.is_empty() || text.ends_with('\n') {
            lines.push("");
        }
        let prefix = self
            .lines
            .iter()
            .zip(&lines)
            .take_while(|(old, new)| old.text == **new)
            .count();
        if prefix == lines.len() && prefix == self.lines.len() {
            return false;
        }
        let suffix = self.lines[prefix..]
            .iter()
            .rev()
            .zip(lines[prefix..].iter().rev())
            .take_while(|(old, new)| old.text == **new)
            .count();
        let tail = self.lines.split_off(self.lines.len() - suffix);
        self.lines.truncate(prefix);
        let mut state = self.lines.last().map_or(LexerState::Normal, |l| l.outgoing);
        for text in &lines[prefix..lines.len() - suffix] {
            let line = SyntaxLine::lex(text, state, self.language);
            state = line.outgoing;
            self.lines.push(line);
            #[cfg(test)]
            {
                self.relexed += 1;
            }
        }
        let mut tail = tail.into_iter();
        while let Some(line) = tail.next() {
            if state == line.incoming {
                self.lines.push(line);
                self.lines.extend(tail);
                break;
            }
            let line = SyntaxLine::lex(&line.text, state, self.language);
            state = line.outgoing;
            self.lines.push(line);
            #[cfg(test)]
            {
                self.relexed += 1;
            }
        }
        true
    }

    fn galley(
        &mut self,
        ui: &egui::Ui,
        text: &str,
        font: &FontId,
        color: Color32,
        selected: Option<(u32, Color32)>,
    ) -> Arc<egui::Galley> {
        #[cfg(test)]
        {
            self.relexed = 0;
            self.laid_out = 0;
        }
        let style = LayoutStyle {
            font: font.clone(),
            color,
            dark: ui.visuals().dark_mode,
            pixels_per_point: ui.ctx().pixels_per_point(),
        };
        // Touch a tiny egui galley each frame. Its identity changes when egui
        // resets the font/atlas cache, invalidating our retained line meshes too.
        let font_epoch =
            ui.fonts_mut(|f| f.layout_no_wrap("M".into(), font.clone(), Color32::WHITE));
        if let Some(cached) = &self.rendered {
            if cached.style == style
                && cached.selected == selected
                && Arc::ptr_eq(&cached.font_epoch, &font_epoch)
                && cached.galley.text() == text
            {
                return Arc::clone(&cached.galley);
            }
        }
        self.update(text);
        if self.rendered.as_ref().is_none_or(|cached| {
            cached.style != style || !Arc::ptr_eq(&cached.font_epoch, &font_epoch)
        }) {
            for line in &mut self.lines {
                line.layout = None;
            }
        }
        if text.is_empty() {
            let galley = ui.fonts_mut(|f| f.layout_job(egui::text::LayoutJob::default()));
            self.rendered = Some(CachedGalley {
                style,
                selected,
                font_epoch,
                galley: Arc::clone(&galley),
            });
            return galley;
        }
        let mut children = Vec::with_capacity(self.lines.len());
        let mut job = egui::text::LayoutJob {
            text: text.into(),
            ..Default::default()
        };
        let mut offset = 0;
        for (index, line) in self.lines.iter_mut().enumerate() {
            let background = selected
                .filter(|(n, _)| *n as usize == index + 1)
                .map_or(Color32::TRANSPARENT, |(_, bg)| bg);
            if line
                .layout
                .as_ref()
                .is_none_or(|l| l.background != background)
            {
                // No wrapping: each source line has exactly one reusable row.
                let body = line.text.strip_suffix('\n').unwrap_or(&line.text);
                let mut line_job = egui::text::LayoutJob {
                    text: body.into(),
                    ..Default::default()
                };
                line.append_sections(&mut line_job, &style, background, 0, body.len());
                if line_job.sections.is_empty() {
                    line_job.sections.push(egui::text::LayoutSection {
                        leading_space: 0.0,
                        byte_range: 0.into()..0.into(),
                        format: egui::TextFormat {
                            font_id: font.clone(),
                            color,
                            background,
                            ..Default::default()
                        },
                    });
                }
                let galley = ui.fonts_mut(|f| f.layout_job(line_job));
                line.layout = Some(LineLayout { background, galley });
                #[cfg(test)]
                {
                    self.laid_out += 1;
                }
            }
            let child = &line.layout.as_ref().unwrap().galley;
            // The combined job is metadata for TextEdit's text and selection.
            // Rebase sections on edits, but never reshape/hash unchanged lines.
            for section in &child.job.sections {
                layout_section(
                    &mut job,
                    offset + usize::from(section.byte_range.start)
                        ..offset + usize::from(section.byte_range.end),
                    section.format.clone(),
                );
            }
            let body_end = offset + child.job.text.len();
            offset += line.text.len();
            if body_end < offset {
                layout_section(
                    &mut job,
                    body_end..offset,
                    child.job.sections.last().unwrap().format.clone(),
                );
            }
            children.push(Arc::clone(child));
        }
        let galley = Arc::new(egui::Galley::concat(
            Arc::new(job),
            &children,
            style.pixels_per_point,
        ));
        self.rendered = Some(CachedGalley {
            style,
            selected,
            font_epoch,
            galley: Arc::clone(&galley),
        });
        galley
    }

    #[cfg(test)]
    fn layout_job(
        &mut self,
        text: &str,
        font: &FontId,
        color: Color32,
        selected: Option<(u32, Color32)>,
        dark: bool,
    ) -> egui::text::LayoutJob {
        self.update(text);
        let style = LayoutStyle {
            font: font.clone(),
            color,
            dark,
            pixels_per_point: 1.0,
        };
        let mut job = egui::text::LayoutJob {
            text: text.into(),
            ..Default::default()
        };
        let mut offset = 0;
        for (index, line) in self.lines.iter().enumerate() {
            let background = selected
                .filter(|(n, _)| *n as usize == index + 1)
                .map_or(Color32::TRANSPARENT, |(_, bg)| bg);
            line.append_sections(&mut job, &style, background, offset, line.text.len());
            offset += line.text.len();
        }
        job
    }
}

fn layout_section(
    job: &mut egui::text::LayoutJob,
    bytes: std::ops::Range<usize>,
    format: egui::TextFormat,
) {
    if bytes.is_empty() {
        return;
    }
    if let Some(previous) = job.sections.last_mut() {
        if previous.byte_range.end == bytes.start.into() && previous.format == format {
            previous.byte_range.end = bytes.end.into();
            return;
        }
    }
    job.sections.push(egui::text::LayoutSection {
        leading_space: 0.0,
        byte_range: bytes.start.into()..bytes.end.into(),
        format,
    });
}

#[derive(Clone, Debug)]
struct Token {
    start: usize,
    end: usize,
    line: u32,
    column: u32,
    end_line: u32,
    text: String,
    signal: bool,
}

#[derive(Default)]
struct Syntax {
    tokens: Vec<Token>,
    assignments: BTreeMap<u32, Vec<(String, u32, u32)>>,
}

impl Syntax {
    fn identifier(&self, line: u32, column: u32) -> Option<String> {
        self.tokens
            .get(
                self.tokens
                    .partition_point(|t| (t.line, t.column) <= (line, column))
                    .checked_sub(1)?,
            )
            .filter(|t| {
                t.line == line
                    && t.column <= column
                    && column < t.column + (t.end - t.start) as u32
                    && t.signal
            })
            .map(|t| t.text.clone())
    }

    #[cfg(test)]
    fn parse(text: &str) -> Self {
        let mut cache = HighlightCache::new(Language::Verilog);
        cache.update(text);
        Self::from_lines(&cache.lines)
    }

    fn from_lines(lines: &[SyntaxLine]) -> Self {
        let mut tokens = Vec::new();
        let mut offset = 0;
        for (line_index, line) in lines.iter().enumerate() {
            let chars: Vec<_> = line.text.chars().collect();
            let mut consumed = 0;
            for token in &line.tokens {
                if token.start < consumed || token.class == TokenClass::Comment {
                    continue;
                }
                let raw = &line.text[token.bytes.clone()];
                let signal = token.class == TokenClass::Identifier
                    && raw != "\\"
                    && (raw.starts_with('\\') || !keyword(raw));
                let mut end = token.end;
                let mut name = raw.trim_start_matches('\\').to_string();
                if signal {
                    if let Some((selected_end, index)) = constant_select(&chars, end) {
                        name = format!("{name}[{index}]");
                        end = selected_end;
                    }
                }
                consumed = end;
                tokens.push(Token {
                    start: offset + token.start,
                    end: offset + end,
                    line: line_index as u32 + 1,
                    column: token.start as u32 + 1,
                    end_line: line_index as u32 + 1,
                    text: name,
                    signal,
                });
            }
            offset += chars.len();
        }
        let mut syntax = Self {
            tokens,
            assignments: BTreeMap::new(),
        };
        syntax.find_assignments();
        syntax
    }

    fn find_assignments(&mut self) {
        let tokens = &self.tokens;
        let mut nesting = 0i32;
        let mut assigned_until = 0;
        let mut assignment_commas = Vec::new();
        let mut question = false;
        for (i, token) in tokens.iter().enumerate() {
            match token.text.as_str() {
                "(" | "[" | "{" => nesting += 1,
                ")" | "]" | "}" => nesting -= 1,
                "?" => question = true,
                ";" => question = false,
                _ => {}
            }
            if nesting != 0 || !matches!(token.text.as_str(), "=" | "<=") || i == 0 {
                continue;
            }
            let mut start = i - 1;
            let mut targets = Vec::new();
            if tokens[start].signal {
                targets.push(&tokens[start]);
            } else if tokens[start].text == "}" {
                let mut want_name = true;
                while start > 0 {
                    start -= 1;
                    let t = &tokens[start];
                    if t.text == "{" {
                        break;
                    }
                    if want_name && t.signal {
                        targets.push(t);
                    } else if want_name || t.text != "," {
                        targets.clear();
                        break;
                    }
                    want_name = !want_name;
                }
                if tokens[start].text != "{" || want_name {
                    targets.clear();
                }
            }
            if targets.is_empty() {
                continue;
            }
            if i < assigned_until && (start == 0 || !assignment_commas.contains(&(start - 1))) {
                continue;
            }
            if i >= assigned_until {
                assignment_commas.clear();
            }
            if start > 0 {
                let previous = tokens[start - 1].text.as_str();
                if previous == ":" && question {
                    continue;
                }
                if !matches!(
                    previous,
                    "assign"
                        | "begin"
                        | "end"
                        | "else"
                        | ";"
                        | ":"
                        | ")"
                        | ","
                        | "wire"
                        | "reg"
                        | "logic"
                        | "integer"
                        | "signed"
                        | "unsigned"
                        | "]"
                ) {
                    continue;
                }
                if previous == "]" {
                    let declaration = tokens[..start]
                        .iter()
                        .rposition(|t| matches!(t.text.as_str(), ";" | "begin" | "end"))
                        .map_or(0, |p| p + 1);
                    if !tokens[declaration..start]
                        .iter()
                        .any(|t| matches!(t.text.as_str(), "wire" | "reg" | "logic"))
                    {
                        continue;
                    }
                }
            }
            let mut depth = 0;
            let mut rhs_questions = 0;
            for (offset, t) in tokens[i + 1..].iter().enumerate() {
                match t.text.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    "begin" | "end" => break,
                    "?" if depth == 0 => rhs_questions += 1,
                    ":" if depth == 0 => {
                        if rhs_questions == 0 {
                            break;
                        }
                        rhs_questions -= 1;
                    }
                    "," if depth == 0 => assignment_commas.push(i + 1 + offset),
                    ";" if depth == 0 => {
                        // Relational operators in an RHS (including a ternary
                        // branch) are not assignments to their left operand.
                        assigned_until = i + 1 + offset;
                        let assigned = self.assignments.entry(t.end_line).or_default();
                        for target in &targets {
                            let entry = (target.text.clone(), target.line, target.column);
                            if !assigned.contains(&entry) {
                                assigned.push(entry);
                            }
                        }
                        break;
                    }
                    _ => {}
                }
                if depth < 0 {
                    break;
                }
            }
        }
    }
}

/// Dynamic and part selects are deliberately left to the lexer as symbols.
fn constant_select(chars: &[char], mut i: usize) -> Option<(usize, u32)> {
    while chars.get(i).is_some_and(|c| matches!(c, ' ' | '\t')) {
        i += 1;
    }
    if chars.get(i) != Some(&'[') {
        return None;
    }
    i += 1;
    while chars.get(i).is_some_and(|c| matches!(c, ' ' | '\t')) {
        i += 1;
    }
    let start = i;
    while chars
        .get(i)
        .is_some_and(|c| c.is_ascii_digit() || *c == '_')
    {
        i += 1;
    }
    let digits: String = chars[start..i].iter().filter(|c| **c != '_').collect();
    let bit = digits.parse::<u32>().ok()?;
    while chars.get(i).is_some_and(|c| matches!(c, ' ' | '\t')) {
        i += 1;
    }
    (chars.get(i) == Some(&']')).then_some((i + 1, bit))
}

fn verilog_keywords() -> &'static std::collections::HashMap<&'static str, bool> {
    // The flag identifies words added by SystemVerilog, without duplicating the
    // Verilog table. Value lookup retains the original combined reserved set.
    const VERILOG: &str = "always and assign automatic begin buf bufif0 bufif1 case casex casez cell cmos config deassign default defparam design disable edge else end endcase endconfig endfunction endgenerate endmodule endprimitive endspecify endtable endtask event for force forever fork function generate genvar highz0 highz1 if ifnone incdir include initial inout input instance integer join large liblist library localparam macromodule medium module nand negedge nmos nor noshowcancelled not notif0 notif1 or output parameter pmos posedge primitive pull0 pull1 pulldown pullup pulsestyle_ondetect pulsestyle_onevent rcmos real realtime reg release repeat rnmos rpmos rtran rtranif0 rtranif1 scalared showcancelled signed small specify specparam strong0 strong1 supply0 supply1 table task time tran tranif0 tranif1 tri tri0 tri1 triand trior trireg unsigned use uwire vectored wait wand weak0 weak1 while wire wor xnor xor";
    const SYSTEM_VERILOG: &str = "accept_on alias always_comb always_ff always_latch assert assume before bind bins binsof bit break byte chandle checker class clocking const constraint context continue cover covergroup coverpoint cross dist do endchecker endclass endclocking endgroup endinterface endpackage endprogram endproperty endsequence enum eventually expect export extends extern final first_match foreach forkjoin global iff ignore_bins illegal_bins implements implies import inside int interconnect interface intersect join_any join_none let local logic longint matches modport nettype new nexttime null package packed priority program property protected pure rand randc randcase randsequence ref reject_on restrict return s_always s_eventually s_nexttime s_until s_until_with sequence shortint shortreal soft solve static string strong struct super sync_accept_on sync_reject_on tagged this throughout timeprecision timeunit type typedef union unique unique0 until until_with untyped var virtual void wait_order weak wildcard with within";
    static WORDS: std::sync::OnceLock<std::collections::HashMap<&str, bool>> =
        std::sync::OnceLock::new();
    WORDS.get_or_init(|| {
        VERILOG
            .split_ascii_whitespace()
            .map(|s| (s, false))
            .chain(SYSTEM_VERILOG.split_ascii_whitespace().map(|s| (s, true)))
            .collect()
    })
}

fn keyword(word: &str) -> bool {
    verilog_keywords().contains_key(word)
}

fn language_keyword(language: Language, word: &str) -> bool {
    match language {
        Language::Verilog => verilog_keywords().get(word) == Some(&false),
        Language::SystemVerilog => keyword(word),
        Language::Vhdl => {
            const VHDL: &str = "abs access after alias all and architecture array assert assume assume_guarantee attribute begin block body buffer bus case component configuration constant context cover default disconnect downto else elsif end entity exit fair fairness file for force function generate generic group guarded if impure in inertial inout is label library linkage literal loop map mod nand new next nor not null of on open or others out package parameter port postponed procedure process property protected pure range record register reject release rem report restrict restrict_guarantee return rol ror select sequence severity shared signal sla sll sra srl strong subtype then to transport type unaffected units until use variable vmode vprop vunit wait when while with xnor xor";
            static WORDS: std::sync::OnceLock<std::collections::HashSet<&str>> =
                std::sync::OnceLock::new();
            WORDS
                .get_or_init(|| VHDL.split_ascii_whitespace().collect())
                .contains(word.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

fn vhdl_bit_string(language: Language, chars: &[char], start: usize) -> Option<usize> {
    if language != Language::Vhdl {
        return None;
    }
    let mut i = start;
    while chars
        .get(i)
        .is_some_and(|c| c.is_ascii_digit() || *c == '_')
    {
        i += 1;
    }
    let prefix = i;
    while chars.get(i).is_some_and(char::is_ascii_alphabetic) {
        i += 1;
    }
    let radix = &chars[prefix..i];
    let valid = matches!(
        radix,
        ['x' | 'X' | 'b' | 'B' | 'o' | 'O' | 'd' | 'D']
            | ['u' | 'U' | 's' | 'S', 'x' | 'X' | 'b' | 'B' | 'o' | 'O']
    );
    if !valid || chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    while chars.get(i).is_some_and(|c| !matches!(c, '"' | '\n')) {
        i += 1;
    }
    Some(i + usize::from(chars.get(i) == Some(&'"')))
}

fn line_column(text: &str, index: usize) -> (u32, u32) {
    text.chars().take(index).fold((1, 1), |(line, col), c| {
        if c == '\n' {
            (line + 1, 1)
        } else {
            (line, col + 1)
        }
    })
}

fn character_offset(text: &str, line: u32, column: Option<u32>, end: bool) -> usize {
    let mut offset = 0;
    for (i, text) in text.split('\n').enumerate() {
        if i + 1 == line as usize {
            return offset
                + column.map_or(if end { text.chars().count() } else { 0 }, |c| {
                    (c.saturating_sub(1) as usize).min(text.chars().count())
                });
        }
        offset += text.chars().count() + 1;
    }
    text.chars().count()
}

fn span_range(text: &str, span: &Span) -> (usize, usize) {
    (
        character_offset(text, span.line, span.column, false),
        character_offset(text, span.end_line, span.end_column, true),
    )
}

fn build_layout_job(
    text: &str,
    font_id: &FontId,
    color: Color32,
    highlight_line: Option<u32>,
    highlight_bg: Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let normal = egui::TextFormat {
        font_id: font_id.clone(),
        color,
        ..Default::default()
    };
    let highlighted = egui::TextFormat {
        font_id: font_id.clone(),
        color,
        background: highlight_bg,
        ..Default::default()
    };
    let mut start = 0usize;
    let mut line_no = 1u32;
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let fmt = if highlight_line == Some(line_no) {
                highlighted.clone()
            } else {
                normal.clone()
            };
            job.append(&text[start..=idx], 0.0, fmt);
            start = idx + 1;
            line_no += 1;
        }
    }
    if start < text.len() {
        let fmt = if highlight_line == Some(line_no) {
            highlighted
        } else {
            normal
        };
        job.append(&text[start..], 0.0, fmt);
    }
    job
}

/// The tab body and close control occupy disjoint parts of one padded rectangle.
fn tab_button(ui: &mut egui::Ui, title: &str, selected: bool) -> (egui::Response, egui::Response) {
    let padding = egui::vec2(8.0, 4.0);
    let galley = egui::WidgetText::from(title).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let height = (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y);
    let size = egui::vec2(galley.size().x + padding.x * 2.0 + height, height);
    let (id, rect) = ui.allocate_space(size);
    let close_rect =
        egui::Rect::from_min_max(egui::pos2(rect.right() - height, rect.top()), rect.max);
    let body_rect = egui::Rect::from_min_max(rect.min, close_rect.left_bottom());
    let body = ui.interact(body_rect, id, egui::Sense::click());
    let background = ui.painter().add(egui::Shape::Noop);
    let close = ui.put(close_rect, egui::Button::new("×").small().frame(false));
    let response = body.clone() | close.clone();
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().set(
            background,
            egui::Shape::rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill),
        );
    }
    ui.painter().galley(
        egui::pos2(
            rect.left() + padding.x,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        visuals.text_color(),
    );
    body.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, title)
    });
    (body, close)
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || (fs::canonicalize(a).ok() == fs::canonicalize(b).ok() && a.exists() && b.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin_frame(
        ctx: &egui::Context,
        editor: &mut Editor,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                editor.show(ui, &super::super::DisplayPaths::new(None));
            },
        )
    }

    fn pin_click(
        ctx: &egui::Context,
        editor: &mut Editor,
        output: &egui::FullOutput,
        text: &str,
    ) -> egui::FullOutput {
        let pos = tab_text_rect(output, text).center();
        for pressed in [true, false] {
            pin_frame(
                ctx,
                editor,
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
        pin_frame(ctx, editor, vec![])
    }

    fn pin_texts(output: &egui::FullOutput) -> Vec<&str> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pin_strip_is_lpf_only_and_visible_in_both_themes() {
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, theme);
            let mut editor = lpf_editor("");
            let output = pin_frame(&ctx, &mut editor, vec![]);
            assert!(pin_texts(&output).contains(&LPF_STRIP));
            assert!(pin_texts(&output).contains(&"Insert pin…"));
            editor.tabs[0].path = "top.v".into();
            editor.tabs[0].highlighting = HighlightCache::new(Language::Verilog);
            let output = pin_frame(&ctx, &mut editor, vec![]);
            assert!(!pin_texts(&output).contains(&LPF_STRIP));
            assert!(!pin_texts(&output).contains(&"Insert pin…"));
        }
    }

    #[test]
    fn pin_insert_matches_wrapper_at_cursor_or_end_and_is_one_undoable_edit() {
        for resource in PIN_RESOURCES {
            let mut editor = lpf_editor("");
            let mut state = egui::text_edit::TextEditState::default();
            let request = PinRequest {
                resource,
                port: match port_naming(resource) {
                    PortNaming::Single { .. } => "chosen",
                    PortNaming::Prefix { common: Some(_) } => "chosen_",
                    PortNaming::Prefix { common: None } => "",
                }
                .into(),
                cursor: None,
            };
            let expected = lpf_lines(
                resource,
                match port_naming(resource) {
                    PortNaming::Single { .. } => Naming::Base("chosen"),
                    PortNaming::Prefix { common: Some(_) } => Naming::Prefix("chosen_"),
                    PortNaming::Prefix { common: None } => Naming::Default,
                },
            )
            .unwrap();
            let tab = &mut editor.tabs[0];
            let note = insert_pin(tab, &mut state, &request, 0.0).unwrap();
            assert_eq!(tab.text, expected);
            assert!(tab.dirty());
            assert_eq!(
                note,
                format!(
                    "# lpf insert {} {} {}",
                    resource_name(resource),
                    request.port,
                    expected.lines().count()
                )
            );
            let current = (tab.cursor.unwrap(), tab.text.clone());
            let mut undoer = state.undoer();
            let before = undoer.undo(&current).unwrap().clone();
            assert_eq!(before.1, "");
            assert!(
                undoer.undo(&before).is_none(),
                "the insert is a single undo step"
            );
            assert_eq!(undoer.redo(&before).unwrap(), &current);
            let original = tab.text.clone();
            assert!(insert_pin(tab, &mut state, &request, 1.0)
                .unwrap_err()
                .to_string()
                .contains("Already assigned"));
            assert_eq!(tab.text, original);
        }
        let mut editor = lpf_editor("# λ before\n# after\n");
        let mut state = egui::text_edit::TextEditState::default();
        let cursor =
            egui::text::CCursorRange::one(egui::text::CCursor::new("# λ before\n".chars().count()));
        let request = PinRequest {
            resource: Resource::Clock,
            port: "clk".into(),
            cursor: Some(cursor),
        };
        let note = insert_pin(&mut editor.tabs[0], &mut state, &request, 0.0).unwrap();
        assert_eq!(
            editor.tabs[0].text,
            format!(
                "# λ before\n{}# after\n",
                lpf_lines(Resource::Clock, Naming::Base("clk")).unwrap()
            )
        );
        editor.notes.push(note);
        assert_eq!(editor.take_notes(), ["# lpf insert clock clk 3"]);
        assert!(editor.take_notes().is_empty());
        let tab = &mut editor.tabs[0];
        tab.text =
            "# LOCATE COMP \"old\" SITE \"G2\";\nLOCATE COMP \"old\"\nSITE /* comment */ \"G2\";\n"
                .into();
        assert!(check_pin_insert(
            tab,
            &lpf_lines(Resource::Clock, Naming::Base("clk")).unwrap()
        )
        .is_err());
        tab.text = "# LOCATE COMP \"old\" SITE \"G2\";\n".into();
        assert!(check_pin_insert(
            tab,
            &lpf_lines(Resource::Clock, Naming::Base("clk")).unwrap()
        )
        .is_ok());
    }

    #[test]
    fn pin_resources_match_generator_ports_preview_and_insertion() {
        use ecp5_emu::flow::new_project::{self, Options};

        let root = crate::test_temp::unique_dir("ecp5-pin-resource-widths-");
        assert_eq!(PIN_RESOURCES, Resource::ALL);
        for resource in PIN_RESOURCES {
            let mut options = Options::new(root.join(resource_name(resource)));
            options.resources.insert(resource);
            let project = new_project::create(&options).unwrap();
            // Exercise both the defaults and every editable naming field.
            for custom in [false, true] {
                let mut expected_ports = ports(resource);
                let (value, naming) = match port_naming(resource) {
                    PortNaming::Single { default } => {
                        let value = if custom { "chosen" } else { default };
                        expected_ports[0].name = value.into();
                        (value, Naming::Base(value))
                    }
                    PortNaming::Prefix {
                        common: Some(common),
                    } if custom => {
                        for port in &mut expected_ports {
                            port.name =
                                format!("chosen_{}", port.name.strip_prefix(common).unwrap());
                        }
                        ("chosen_", Naming::Prefix("chosen_"))
                    }
                    PortNaming::Prefix { common } => (common.unwrap_or(""), Naming::Default),
                };
                let expected = lpf_lines(resource, naming).unwrap();
                let mut editor = Editor::default();
                editor
                    .open(&project.root.join("top.lpf"), "top.lpf".into(), false, None)
                    .unwrap();
                editor.tabs[0].text.clear();
                let ctx = egui::Context::default();
                let output = pin_frame(&ctx, &mut editor, vec![]);
                let output = pin_click(&ctx, &mut editor, &output, "Insert pin…");
                pin_click(&ctx, &mut editor, &output, resource_label(resource));
                assert_eq!(editor.pin_insert.resource, resource);
                assert_eq!(editor.pin_insert.port, resource_port(resource));
                if custom && !value.is_empty() {
                    // Type into the actual field so the next frame must update
                    // both the listed names and the LPF preview.
                    let output = pin_frame(&ctx, &mut editor, vec![]);
                    pin_click(&ctx, &mut editor, &output, resource_port(resource));
                    pin_frame(
                        &ctx,
                        &mut editor,
                        vec![
                            egui::Event::Key {
                                key: egui::Key::A,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::COMMAND,
                            },
                            egui::Event::Text(value.into()),
                        ],
                    );
                }
                let output = pin_frame(&ctx, &mut editor, vec![]);
                assert_eq!(editor.pin_insert.port, value);
                assert_eq!(editor.pin_insert.inserted_ports(), expected_ports);
                let texts = pin_texts(&output);
                assert!(
                    texts.contains(&expected.as_str()),
                    "{resource:?}: LPF preview"
                );
                match port_naming(resource) {
                    PortNaming::Single { .. } => {
                        assert!(texts.contains(&"Port base name"));
                        assert!(!texts.contains(&"Prefix"));
                        for port in ports(resource) {
                            let width = port.width.unwrap_or(1);
                            assert!(
                                texts.contains(&format!("{} ({width} bits)", port.name).as_str())
                            );
                        }
                    }
                    PortNaming::Prefix { common } => {
                        assert!(!texts.contains(&"Port base name"));
                        assert!(!texts.contains(&"Top-module ports (choose one):"));
                        assert_eq!(texts.contains(&"Prefix"), common.is_some());
                        let present = if custom && common.is_some() {
                            0
                        } else {
                            expected_ports.len()
                        };
                        assert!(texts.contains(
                            &format!("{present} of {} ports are in top", expected_ports.len())
                                .as_str()
                        ));
                        for port in &expected_ports {
                            let width = port.width.unwrap_or(1);
                            let status = if present == 0 {
                                format!("× {} ({width} bits; missing)", port.name)
                            } else {
                                format!("✔ {} ({width} bits)", port.name)
                            };
                            assert!(texts.contains(&status.as_str()), "{resource:?}: {status}");
                        }
                        assert!(editor
                            .pin_insert
                            .width_warning(lpf_balls(&expected).len())
                            .is_none());
                    }
                }
                pin_click(&ctx, &mut editor, &output, "Insert");
                assert_eq!(editor.tabs[0].text, expected, "{resource:?} insertion");
                assert_eq!(
                    editor.take_notes(),
                    [format!(
                        "# lpf insert {} {value} {}",
                        resource_name(resource),
                        expected.lines().count()
                    )]
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pin_prefix_checks_matching_different_unknown_and_missing_top_ports() {
        let mut cache = HighlightCache::new(Language::Verilog);
        cache.update("module chosen(input oled_clk, output [3:0] oled_mosi, input [N-1:0] oled_dc); endmodule");
        let mut editor = lpf_editor("");
        let ctx = egui::Context::default();
        let output = pin_frame(&ctx, &mut editor, vec![]);
        let output = pin_click(&ctx, &mut editor, &output, "Insert pin…");
        pin_click(&ctx, &mut editor, &output, resource_label(Resource::Oled));
        editor.pin_insert.ports = top_ports(&cache, "chosen").unwrap();
        let output = pin_frame(&ctx, &mut editor, vec![]);
        let texts = pin_texts(&output);
        for status in [
            "3 of 5 ports are in top",
            "✔ oled_clk (1 bits)",
            "⚠ oled_mosi (needs 1 bits; top has 4 bits)",
            "⚠ oled_dc (needs 1 bits; top width unknown)",
            "× oled_resn (1 bits; missing)",
            "× oled_csn (1 bits; missing)",
        ] {
            assert!(texts.contains(&status), "missing {status}");
        }
        pin_click(&ctx, &mut editor, &output, "Insert");
        assert_eq!(
            editor.tabs[0].text,
            lpf_lines(Resource::Oled, Naming::Default).unwrap()
        );
        assert_eq!(editor.take_notes(), ["# lpf insert oled oled_ 10"]);
    }

    #[test]
    fn pin_prefix_checks_bus_widths_and_escaped_scalar_names() {
        let mut cache = HighlightCache::new(Language::Verilog);
        cache.update(r"module top(input sd_clk, input [3:0] sd_cmd, input \sd_d[0] ); endmodule");
        let insert = PinInsert {
            resource: Resource::Sd,
            port: "sd_".into(),
            ports: top_ports(&cache, "top").unwrap(),
            cursor: None,
        };
        let statuses: Vec<_> = insert
            .inserted_ports()
            .iter()
            .map(|port| insert.port_status(port))
            .collect();
        assert_eq!(
            statuses,
            [
                (true, "✔ sd_clk (1 bits)".into()),
                (true, "⚠ sd_cmd (needs 1 bits; top has 4 bits)".into()),
                (true, "✔ sd_d[0] (1 bits)".into()),
                (false, "× sd_d[3] (1 bits; missing)".into()),
            ]
        );
        cache.update("module top(output [3:0] audio_l, output [2:0] audio_r); endmodule");
        let insert = PinInsert {
            resource: Resource::Audio,
            port: "audio_".into(),
            ports: top_ports(&cache, "top").unwrap(),
            cursor: None,
        };
        let statuses: Vec<_> = insert
            .inserted_ports()
            .iter()
            .map(|port| insert.port_status(port))
            .collect();
        assert_eq!(
            statuses,
            [
                (true, "✔ audio_l (4 bits)".into()),
                (true, "⚠ audio_r (needs 4 bits; top has 3 bits)".into()),
                (false, "× audio_v (4 bits; missing)".into()),
            ]
        );
    }

    #[test]
    fn top_port_suggestions_use_named_module_and_ansi_or_body_declarations() {
        for source in [
            "module other(input wrong); endmodule\n// module chosen(input fake);\nmodule chosen #(parameter N=4)(input clk, output wire [7:0] led, other_led, input [0:3] sw, input [N-1:0] dynamic); endmodule",
            "module chosen(clk, led, other_led, sw, dynamic); input clk; output [7:0] led, other_led; input [0:3] sw; input [N-1:0] dynamic; wire internal; endmodule",
        ] {
            let mut cache = HighlightCache::new(Language::Verilog);
            cache.update(source);
            let ports = top_ports(&cache, "chosen").unwrap();
            assert_eq!(ports, [
                TopPort { name: "clk".into(), width: Some(1) },
                TopPort { name: "led".into(), width: Some(8) },
                TopPort { name: "other_led".into(), width: Some(8) },
                TopPort { name: "sw".into(), width: Some(4) },
                TopPort { name: "dynamic".into(), width: None },
            ]);
            assert!(top_ports(&cache, "absent").is_none());
        }
    }

    #[test]
    fn pin_popup_suggests_unsaved_project_ports_warns_and_queues_insert() {
        let root = crate::test_temp::unique_dir("ecp5-pin-popup-");
        fs::write(
            root.join("ecp5.toml"),
            "[project]\ntop='chosen'\n[sources]\nverilog=['top.v']\nconstraints='top.lpf'\n",
        )
        .unwrap();
        fs::write(root.join("top.v"), "module chosen(input clk); endmodule\n").unwrap();
        fs::write(root.join("top.lpf"), "").unwrap();
        let mut editor = Editor::default();
        editor
            .open(&root.join("top.v"), "top.v".into(), false, None)
            .unwrap();
        editor.tabs[0].text =
            "module chosen(input clk, output [7:0] led, input [3:0] sw); endmodule\n".into();
        editor
            .open(&root.join("top.lpf"), "top.lpf".into(), false, None)
            .unwrap();
        let ctx = egui::Context::default();
        let output = pin_frame(&ctx, &mut editor, vec![]);
        let output = pin_click(&ctx, &mut editor, &output, "Insert pin…");
        let output = pin_click(&ctx, &mut editor, &output, "DIP switches");
        assert!(pin_texts(&output).contains(&"led (8 bits)"));
        assert!(pin_texts(&output).contains(&"sw (4 bits)"));
        let output = pin_click(&ctx, &mut editor, &output, "clk (1 bits)");
        assert!(pin_texts(&output)
            .iter()
            .any(|s| s.contains("Width mismatch") && s.contains("4 bits") && s.contains("1 bits")));
        let output = pin_click(&ctx, &mut editor, &output, "sw (4 bits)");
        assert!(!pin_texts(&output)
            .iter()
            .any(|s| s.contains("Width mismatch")));
        pin_click(&ctx, &mut editor, &output, "Insert");
        assert_eq!(
            editor.tabs[1].text,
            lpf_lines(Resource::Switches, Naming::Base("sw")).unwrap()
        );
        assert!(editor.tabs[1].dirty());
        assert_eq!(editor.take_notes(), ["# lpf insert switches sw 8"]);
        assert!(editor.take_notes().is_empty());
        pin_frame(
            &ctx,
            &mut editor,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert!(
            editor.tabs[1].text.is_empty(),
            "one undo removes the whole insertion"
        );
        assert!(!editor.tabs[1].dirty());
        pin_frame(
            &ctx,
            &mut editor,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    shift: true,
                    ..egui::Modifiers::COMMAND
                },
            }],
        );
        assert_eq!(
            editor.tabs[1].text,
            lpf_lines(Resource::Switches, Naming::Base("sw")).unwrap()
        );
        let output = pin_frame(&ctx, &mut editor, vec![]);
        let output = pin_click(&ctx, &mut editor, &output, "Insert pin…");
        assert!(pin_texts(&output)
            .iter()
            .any(|s| s.contains("Already assigned")));
        assert!(editor.take_notes().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    const LPF_SAMPLE: &str = "  LOCATE COMP \"led[0]\" SITE \"G2\";\nIOBUF PORT \"led[0]\" IO_TYPE=LVCMOS33 PULLMODE=UP DRIVE=4;\nIOBUF PORT \"button\" IO_TYPE=LVCMOS25 PULLMODE=DOWN;\nIOBUF PORT \"other\" IO_TYPE=LVCMOS18 PULLMODE=NONE;\nFREQUENCY PORT \"clk_25mhz\" 25 MHZ;\nBLOCK RESETPATHS;\nSYSCONFIG COMPRESS_CONFIG=ON;\nBANK 6 VCCIO 3.3 V;\n# IO_TYPE=LVCMOS33 is a comment\n// PULLMODE=UP is also a comment\n/* LOCATE COMP \"comment\" SITE \"G2\"; */\n";

    fn lpf_editor(text: &str) -> Editor {
        let mut editor = value_editor(text);
        editor.tabs[0].path = "top.lpf".into();
        editor.tabs[0].label = "top.lpf".into();
        editor.tabs[0].highlighting = HighlightCache::new(Language::Lpf);
        // LPF help must never request inline or hover live values, even if enabled.
        editor.inline_values = true;
        editor
    }

    fn lpf_hover_frames(editor: &mut Editor, byte: usize, scripted: bool) -> Vec<Vec<String>> {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.5);
        let text = editor.tabs[0].text.clone();
        let index = text[..byte].chars().count();
        if scripted {
            let (line, column) = line_column(&text, index);
            editor.script_hover(Path::new("top.lpf"), line, column);
        }
        let mut pointer = None;
        let mut frames = Vec::new();
        for time in [0.0, 0.1, 0.2, 1.0, 1.1] {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 600.0),
                    )),
                    time: Some(time),
                    events: if scripted {
                        vec![egui::Event::PointerGone]
                    } else if time == 0.1 {
                        pointer.map(egui::Event::PointerMoved).into_iter().collect()
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ui| {
                    editor.show_values(ui, &super::super::DisplayPaths::new(None), |_, _, _| {
                        panic!("LPF help must not read or record live values")
                    });
                },
            );
            let mut texts: Vec<String> = Vec::new();
            for shape in &output.shapes {
                if let egui::Shape::Text(t) = &shape.shape {
                    if t.galley.text() == text {
                        let cursor = t.galley.pos_from_cursor(egui::text::CCursor::new(index));
                        pointer = Some(t.pos + egui::vec2(cursor.left() + 1.0, cursor.center().y));
                    }
                    texts.push(t.galley.text().into());
                }
            }
            frames.push(texts);
        }
        assert!(
            editor.tabs[0].syntax.is_none(),
            "LPF never runs Verilog analysis"
        );
        assert_eq!(
            editor.tabs[0].highlighting.relexed, 0,
            "hover reuses cached tokens"
        );
        assert_eq!(editor.tabs[0].text, text);
        frames
    }

    #[test]
    fn lpf_pointer_hovers_share_the_value_tooltip_delay_and_cover_keywords_and_values() {
        for (token, sentence, example) in [
            ("LOCATE", LPF_LOCATE, LPF_LOCATE_EXAMPLE),
            ("COMP", LPF_LOCATE, LPF_LOCATE_EXAMPLE),
            ("SITE", LPF_BALL, LPF_LOCATE_EXAMPLE),
            ("IOBUF", LPF_IOBUF, LPF_IOBUF_EXAMPLE),
            ("PORT", LPF_IOBUF, LPF_IOBUF_EXAMPLE),
            ("IO_TYPE", LPF_IO_TYPE, LPF_IOBUF_EXAMPLE),
            ("LVCMOS33", LPF_IO_TYPE, "IOBUF PORT \"led[0]\" IO_TYPE=LVCMOS33;"),
            ("LVCMOS25", LPF_IO_TYPE, "IOBUF PORT \"led[0]\" IO_TYPE=LVCMOS25;"),
            ("LVCMOS18", LPF_IO_TYPE, "IOBUF PORT \"led[0]\" IO_TYPE=LVCMOS18;"),
            ("PULLMODE", LPF_PULLMODE, LPF_IOBUF_EXAMPLE),
            ("UP", LPF_PULLMODE, "IOBUF PORT \"button\" IO_TYPE=LVCMOS33 PULLMODE=UP;"),
            ("DOWN", LPF_PULLMODE, "IOBUF PORT \"button\" IO_TYPE=LVCMOS33 PULLMODE=DOWN;"),
            ("NONE", LPF_PULLMODE, "IOBUF PORT \"button\" IO_TYPE=LVCMOS33 PULLMODE=NONE;"),
            ("DRIVE", LPF_DRIVE, LPF_IOBUF_EXAMPLE),
            ("FREQUENCY", LPF_FREQUENCY, LPF_FREQUENCY_EXAMPLE),
            ("MHZ", LPF_FREQUENCY, LPF_FREQUENCY_EXAMPLE),
            ("BLOCK", "BLOCK excludes selected paths from timing analysis; nextpnr accepts reset and asynchronous path forms.", "BLOCK RESETPATHS;"),
            ("SYSCONFIG", "SYSCONFIG sets device configuration options; rarely needed in ULX3S projects.", "SYSCONFIG COMPRESS_CONFIG=ON;"),
            ("BANK", "BANK selects an I/O bank for preferences; nextpnr infers bank voltage from I/O types instead.", "BANK 6 VCCIO 3.3 V;"),
            ("VCCIO", "VCCIO specifies an I/O bank's supply voltage; it does not change the board's physical power supply.", "BANK 6 VCCIO 3.3 V;"),
            ("\"G2\"", LPF_BALL, LPF_LOCATE_EXAMPLE),
            ("\"led[0]\"", LPF_BUS, LPF_LOCATE_EXAMPLE),
        ] {
            let mut editor = lpf_editor(LPF_SAMPLE);
            let frames = lpf_hover_frames(&mut editor, LPF_SAMPLE.find(token).unwrap(), false);
            assert!(!frames[2].iter().any(|s| s == sentence), "early tooltip for {token}");
            let shown = frames.last().unwrap();
            assert!(shown.iter().any(|s| s == sentence), "{token}: {shown:?}; hover: {:?}", editor.pointer_hover);
            assert!(shown.iter().any(|s| s == example), "example for {token}: {shown:?}");
            if token == "\"G2\"" {
                assert!(shown.iter().any(|s| s == "G2: row G, column 2."));
                assert!(!shown.iter().any(|s| s.contains("LED D0")));
            }
            if token == "\"led[0]\"" {
                assert!(shown.iter().all(|s| {
                    !s.contains("port membership") && !s.contains("project's top module")
                }));
            }
        }
    }

    #[test]
    fn lpf_whitespace_comments_and_unrelated_strings_do_not_show_help() {
        let text = format!("{LPF_SAMPLE}UNKNOWN \"G2\";\n");
        for byte in [
            0,
            text.find(" COMP").unwrap(),
            text.find("# IO_TYPE").unwrap() + 3,
            text.find("// PULLMODE").unwrap() + 4,
            text.find("/* LOCATE").unwrap() + 4,
            text.find("UNKNOWN").unwrap(),
            text.rfind("\"G2\"").unwrap(),
        ] {
            for scripted in [false, true] {
                let mut editor = lpf_editor(&text);
                let frames = lpf_hover_frames(&mut editor, byte, scripted);
                assert!(frames.last().unwrap().iter().all(|s| {
                    !s.contains("maps top-level ports")
                        && !s.contains("selects the electrical standard")
                        && !s.contains("selects a weak internal pull")
                        && !s.contains("CABGA381 coordinate")
                }));
            }
        }
    }

    #[test]
    fn lpf_split_statements_and_port_membership_use_cached_tokens() {
        let text = "LOCATE COMP\n# comment between tokens\n\"led[0]\" SITE\n\"G2\";\n";
        for known in [true, false] {
            let mut editor = lpf_editor(text);
            editor.lpf_ports = Some(if known {
                BTreeSet::from(["led[0]".into()])
            } else {
                BTreeSet::from(["led[1]".into()])
            });
            let frames = lpf_hover_frames(&mut editor, text.find("\"led[0]\"").unwrap(), true);
            let expected = if known {
                "\"led[0]\" is a port in the project's top module."
            } else {
                "\"led[0]\" is not a port in the project's top module."
            };
            assert!(frames.last().unwrap().iter().any(|s| s == expected));
            let frames = lpf_hover_frames(&mut editor, text.find("\"G2\"").unwrap(), true);
            assert!(frames
                .last()
                .unwrap()
                .iter()
                .any(|s| s == "G2: row G, column 2."));
            assert!(lpf_ball_help("A0").is_none());
            assert!(lpf_ball_help("led[0]").is_none());
        }
    }

    fn tab_editor() -> Editor {
        let mut editor = Editor::default();
        for name in ["first.v", "second.v", "third.v"] {
            let mut tab = value_editor("// source\n").tabs.remove(0);
            tab.path = Path::new("/opt/tab-project/rtl").join(name);
            tab.label = name.into();
            editor.tabs.push(tab);
        }
        editor.tabs[0].text.push_str("// unsaved\n");
        editor.active = 2;
        editor
    }

    fn tab_frame(
        ctx: &egui::Context,
        editor: &mut Editor,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                editor.show(
                    ui,
                    &super::super::DisplayPaths::with_environment(
                        None,
                        Some("/home/visitor".into()),
                        Some(Path::new("/opt/tab-project")),
                    ),
                );
            },
        )
    }

    fn tab_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing text: {text}"))
    }

    fn tab_click(
        ctx: &egui::Context,
        editor: &mut Editor,
        pos: egui::Pos2,
        button: egui::PointerButton,
    ) {
        for pressed in [true, false] {
            tab_frame(
                ctx,
                editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn tab_padding_selects_and_keeps_the_dirty_marker() {
        let ctx = egui::Context::default();
        let mut editor = tab_editor();
        let output = tab_frame(&ctx, &mut editor, vec![]);
        let label = tab_text_rect(&output, "first.v *");
        let padding = egui::pos2(label.left() - 4.0, label.center().y);
        assert!(!label.contains(padding));
        tab_click(&ctx, &mut editor, padding, egui::PointerButton::Primary);
        assert_eq!(
            editor.active_path(),
            Some(Path::new("/opt/tab-project/rtl/first.v"))
        );
        assert!(editor.tabs[0].dirty());
        let output = tab_frame(&ctx, &mut editor, vec![]);
        tab_text_rect(&output, "first.v *");
    }

    #[test]
    fn tab_padding_hover_highlights_the_whole_tab_and_shows_relative_path() {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let mut editor = tab_editor();
        let output = tab_frame(&ctx, &mut editor, vec![]);
        let label = tab_text_rect(&output, "first.v *");
        let close = tab_text_rect(&output, "×");
        let padding = egui::pos2(label.left() - 4.0, label.center().y);
        tab_frame(&ctx, &mut editor, vec![egui::Event::PointerMoved(padding)]);
        tab_frame(&ctx, &mut editor, vec![]);
        let output = tab_frame(&ctx, &mut editor, vec![]);
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r)
                if r.fill != Color32::TRANSPARENT
                    && r.rect.height() <= label.height() + 16.0
                    && r.rect.contains(padding)
                    && r.rect.contains_rect(label)
                    && r.rect.contains_rect(close))
        }));
        tab_text_rect(&output, "rtl/first.v");
        assert!(output.shapes.iter().all(|shape| {
            !matches!(&shape.shape, egui::Shape::Text(t)
                if t.galley.text().contains("/opt/tab-project")
                    || t.galley.text().contains("/home/visitor"))
        }));
        assert_eq!(editor.active, 2);
    }

    #[test]
    fn tab_close_and_middle_click_preserve_the_other_active_file() {
        for (close_control, button) in [
            (true, egui::PointerButton::Primary),
            (true, egui::PointerButton::Middle),
            (false, egui::PointerButton::Middle),
        ] {
            let ctx = egui::Context::default();
            let mut editor = tab_editor();
            editor.active = 1;
            let output = tab_frame(&ctx, &mut editor, vec![]);
            let label = tab_text_rect(&output, "first.v *");
            let pos = if close_control {
                tab_text_rect(&output, "×").center()
            } else {
                egui::pos2(label.left() - 4.0, label.center().y)
            };
            tab_click(&ctx, &mut editor, pos, button);
            assert_eq!(editor.tabs.len(), 2);
            assert_eq!(editor.tabs[0].label, "second.v");
            assert_eq!(
                editor.active_path(),
                Some(Path::new("/opt/tab-project/rtl/second.v"))
            );
        }
    }

    fn assert_token_spans(language: Language, text: &str, expected: &[(&str, TokenClass)]) {
        let mut cache = HighlightCache::new(language);
        cache.update(text);
        let mut offset = 0;
        let mut actual = Vec::new();
        for line in &cache.lines {
            for token in &line.tokens {
                assert_eq!(line.text[..token.bytes.start].chars().count(), token.start);
                assert_eq!(line.text[..token.bytes.end].chars().count(), token.end);
                actual.push((
                    offset + token.bytes.start..offset + token.bytes.end,
                    token.class,
                ));
            }
            offset += line.text.len();
        }
        assert_eq!(actual.len(), expected.len(), "{language:?}: {actual:?}");
        let mut end = 0;
        for ((span, class), &(raw, expected_class)) in actual.iter().zip(expected) {
            let start = end + text[end..].find(raw).unwrap();
            assert!(text[end..start].chars().all(char::is_whitespace));
            assert_eq!(*span, start..start + raw.len(), "{language:?}: {raw}");
            assert_eq!(*class, expected_class, "{language:?}: {raw}");
            end = span.end;
        }
        assert!(text[end..].chars().all(char::is_whitespace));
    }

    #[test]
    fn language_detection_and_system_verilog_keywords_are_distinct() {
        for (path, language) in [
            ("top.v", Language::Verilog),
            ("header.vh", Language::Plain),
            ("top.sv", Language::SystemVerilog),
            ("header.svh", Language::SystemVerilog),
            ("top.vhd", Language::Vhdl),
            ("top.vhdl", Language::Vhdl),
            ("pins.lpf", Language::Lpf),
            ("ecp5.toml", Language::Toml),
            ("top.log", Language::Plain),
            ("notes.txt", Language::Plain),
        ] {
            assert_eq!(Language::for_path(Path::new(path)), language);
        }
        let words = "logic bit byte int always_ff always_comb always_latch typedef enum struct union packed package endpackage import interface endinterface modport unique priority";
        for language in [Language::Verilog, Language::SystemVerilog] {
            let line = SyntaxLine::lex(words, LexerState::Normal, language);
            assert!(line.tokens.iter().all(|t| t.class
                == if language == Language::Verilog {
                    TokenClass::Identifier
                } else {
                    TokenClass::Keyword
                }));
            // Value lookup keeps the original reserved-word exclusions in .v too.
            assert!(Syntax::from_lines(&[line]).tokens.iter().all(|t| !t.signal));
        }
    }

    #[test]
    fn system_verilog_and_verilog_keep_identical_live_value_lookup() {
        let text = "module top;\nlogic [7:0] data;\nalways_ff @(posedge clk) begin\n data <= source[2] + '1;\nend\nendmodule\n";
        let mut syntax = Vec::new();
        for language in [Language::Verilog, Language::SystemVerilog] {
            let mut cache = HighlightCache::new(language);
            cache.update(text);
            syntax.push(Syntax::from_lines(&cache.lines));
        }
        assert_eq!(syntax[0].assignments, syntax[1].assignments);
        for (line_index, line) in text.lines().enumerate() {
            for column in 1..=line.chars().count() as u32 {
                assert_eq!(
                    syntax[0].identifier(line_index as u32 + 1, column),
                    syntax[1].identifier(line_index as u32 + 1, column)
                );
            }
        }
        assert_eq!(syntax[1].identifier(4, 10).as_deref(), Some("source[2]"));
        let mut editor = value_editor("signal logic : bit;\nlogic <= '1';\n");
        editor.tabs[0].path = "top.vhdl".into();
        editor.tabs[0].highlighting = HighlightCache::new(Language::Vhdl);
        assert_eq!(
            editor
                .value_probe_at(Path::new("top.vhdl"), 1, 8)
                .unwrap()
                .identifier,
            "logic"
        );
        assert!(
            editor.tabs[0].syntax.is_none(),
            "VHDL never runs the Verilog value analysis"
        );
    }

    #[test]
    fn system_verilog_classes_and_spans_include_fill_literals_and_scope() {
        use TokenClass::*;
        assert_token_spans(Language::SystemVerilog,
            "`ifdef TEST\nmodule top; logic x;\nalways_ff @(posedge clk) x <= '0;\nassign y = pkg::data | '1; $display(\"a\\\"b\"); // λ\n/* block\n end */ function int f; endfunction task t; endtask endmodule\n",
            &[
                ("`ifdef", Directive), ("TEST", Identifier), ("module", Keyword), ("top", Identifier), (";", Plain),
                ("logic", Keyword), ("x", Identifier), (";", Plain), ("always_ff", Keyword), ("@", Plain),
                ("(", Plain), ("posedge", Keyword), ("clk", Identifier), (")", Plain), ("x", Identifier),
                ("<=", Plain), ("'0", Number), (";", Plain), ("assign", Keyword), ("y", Identifier),
                ("=", Plain), ("pkg", Identifier), ("::", Plain), ("data", Identifier), ("|", Plain),
                ("'1", Number), (";", Plain), ("$display", SystemTask), ("(", Plain), ("\"a\\\"b\"", String),
                (")", Plain), (";", Plain), ("// λ", Comment), ("/* block\n", Comment), (" end */", Comment),
                ("function", Keyword), ("int", Keyword), ("f", Identifier), (";", Plain), ("endfunction", Keyword),
                ("task", Keyword), ("t", Identifier), (";", Plain), ("endtask", Keyword), ("endmodule", Keyword),
            ]);
    }

    #[test]
    fn vhdl_classes_and_spans_distinguish_attributes_characters_and_numbers() {
        use TokenClass::*;
        assert_token_spans(Language::Vhdl,
            "EnTiTy Top IS\nSIGNAL clk : bit := 'a';\nBEGin\nif clk'event and (s)'range = 16#FF# then\ns <= x\"FF\" & b\"1010\" & o\"17\" & 8x\"FF\";\nreport \"a\"\"b\"; -- λ comment\n/* block\n comment */ END;\n",
            &[
                ("EnTiTy", Keyword), ("Top", Identifier), ("IS", Keyword), ("SIGNAL", Keyword),
                ("clk", Identifier), (":", Plain), ("bit", Identifier), (":=", Plain), ("'a'", String), (";", Plain),
                ("BEGin", Keyword), ("if", Keyword), ("clk", Identifier), ("'", Plain), ("event", Identifier),
                ("and", Keyword), ("(", Plain), ("s", Identifier), (")", Plain), ("'", Plain), ("range", Keyword),
                ("=", Plain), ("16#FF#", Number), ("then", Keyword), ("s", Identifier), ("<=", Plain),
                ("x\"FF\"", Number), ("&", Plain), ("b\"1010\"", Number), ("&", Plain), ("o\"17\"", Number),
                ("&", Plain), ("8x\"FF\"", Number), (";", Plain), ("report", Keyword), ("\"a\"\"b\"", String),
                (";", Plain), ("-- λ comment", Comment), ("/* block\n", Comment), (" comment */", Comment),
                ("END", Keyword), (";", Plain),
            ]);
        assert_token_spans(
            Language::Vhdl,
            "s'range 'x' 16#F.F#E+1 \\odd name\\",
            &[
                ("s", Identifier),
                ("'", Plain),
                ("range", Keyword),
                ("'x'", String),
                ("16#F.F#E+1", Number),
                ("\\odd name\\", Identifier),
            ],
        );
    }

    #[test]
    fn vhdl_block_comment_edits_propagate_through_the_shared_cache() {
        let text = "signal s : bit;\n/* open */\ns <= '1';\n-- end\n";
        let mut cache = HighlightCache::new(Language::Vhdl);
        cache.update(text);
        let opened = text.replace("/* open */", "/* open");
        cache.update(&opened);
        assert_eq!(cache.relexed, 4);
        assert_eq!(cache.lines[2].tokens[0].class, TokenClass::Comment);
        assert_same_as_fresh(&cache, &opened);
        let closed = opened.replace("-- end", "*/ -- end");
        cache.update(&closed);
        assert_eq!(cache.relexed, 2);
        assert_eq!(cache.lines[3].tokens[1].class, TokenClass::Comment);
        assert_same_as_fresh(&cache, &closed);
    }

    #[test]
    fn vhdl_hover_displays_and_records_the_not_yet_reason() {
        let root = crate::test_temp::unique_dir("ecp5-vhdl-values-")
            .canonicalize()
            .unwrap();
        fs::write(
            root.join("ecp5.toml"),
            "[sources]\nfiles = ['top.vhd', 'top.vhdl']\n",
        )
        .unwrap();
        fs::write(root.join("top.v"), "module top; endmodule\n").unwrap();
        fs::write(root.join("top.lpf"), "").unwrap();
        for file in ["top.vhd", "top.vhdl"] {
            fs::write(root.join(file), "signal clk : bit;\n").unwrap();
        }
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut window =
            super::super::ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
        let record = root.join("vhdl.tsv");
        window.app.trace(&record).unwrap();
        window.editor.inline_values = true;
        let reason = "Live values are not available for VHDL yet";
        for file in ["top.vhd", "top.vhdl"] {
            let path = root.join(file);
            window.editor.open(&path, file.into(), false, None).unwrap();
            window.editor.script_hover(&path, 1, 8);
            let mut calls = 0;
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut source = super::super::SourceView {
                    app: &mut window.app,
                    project: window.project.as_ref().ok(),
                    emulated_root: Some(&root),
                    paths: &window.paths,
                    built: None,
                    modified: Default::default(),
                };
                window
                    .editor
                    .show_values(ui, &window.paths, |probe, dirty, record| {
                        assert!(record, "VHDL never requests inline live values");
                        let view = source.value(probe, dirty, record);
                        assert_eq!(probe.identifier, "clk");
                        assert_eq!(view.detail, [reason]);
                        assert!(view.targets.is_empty());
                        calls += 1;
                        view
                    });
            });
            assert!(calls > 0);
            assert!(window
                .app
                .take_probe_notes()
                .iter()
                .any(|note| note == &format!("# value {file}:1:8 clk -> {reason}")));
        }
        let probe = Probe {
            path: root.join("outside.vhd"),
            line: 1,
            column: 1,
            identifier: "outside".into(),
        };
        let mut source = super::super::SourceView {
            app: &mut window.app,
            project: None,
            emulated_root: None,
            paths: &window.paths,
            built: None,
            modified: Default::default(),
        };
        assert_eq!(source.value(&probe, false, true).detail, [reason]);
        drop(window);
        let notes = fs::read_to_string(record).unwrap();
        assert!(notes.contains(&format!("# value top.vhd:1:8 clk -> {reason}")));
        assert!(notes.contains(&format!("# value top.vhdl:1:8 clk -> {reason}")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn galley_cache_reuses_idle_and_only_reshapes_changed_lines() {
        let ctx = egui::Context::default();
        let mut cache = HighlightCache::new(Language::Verilog);
        let text = "module top;\nwire clk;\nendmodule\n";
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = FontId::monospace(12.5);
            let color = ui.visuals().text_color();
            let before = cache.galley(ui, text, &font, color, None);
            assert_eq!(cache.laid_out, 4);
            let idle = cache.galley(ui, text, &font, color, None);
            assert!(Arc::ptr_eq(&before, &idle));
            assert_eq!((cache.relexed, cache.laid_out), (0, 0));
            let edited = text.replace("clk", "clock");
            let after = cache.galley(ui, &edited, &font, color, None);
            assert_eq!((cache.relexed, cache.laid_out), (1, 1));
            assert!(Arc::ptr_eq(&before.rows[0].row, &after.rows[0].row));
            assert!(Arc::ptr_eq(&before.rows[2].row, &after.rows[2].row));
            let background = Color32::from_rgba_unmultiplied(180, 130, 40, 65);
            let selected = Some((2, background));
            let selected_galley = cache.galley(ui, &edited, &font, color, selected);
            assert_eq!((cache.relexed, cache.laid_out), (0, 1));
            assert!(!Arc::ptr_eq(&after, &selected_galley));
            cache.galley(ui, &edited, &font, color, Some((3, background)));
            assert_eq!((cache.relexed, cache.laid_out), (0, 2));
            cache.galley(ui, &edited, &font, Color32::RED, selected);
            assert_eq!(cache.laid_out, 4);
            cache.galley(ui, &edited, &FontId::monospace(14.0), color, None);
            assert_eq!(cache.laid_out, 4);
            ui.visuals_mut().dark_mode = !ui.visuals().dark_mode;
            cache.galley(ui, &edited, &FontId::monospace(14.0), color, None);
            assert_eq!(cache.laid_out, 4);
        });
    }

    #[test]
    fn concatenated_cached_lines_match_full_layout_in_both_themes() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::install(
                &ctx,
                if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                },
            );
            for language in [
                Language::Verilog,
                Language::SystemVerilog,
                Language::Vhdl,
                Language::Lpf,
                Language::Toml,
                Language::Plain,
            ] {
                let mut cache = HighlightCache::new(language);
                for text in ["", "\n", "\n\n", "module top;\n\treg [7:0] data = 8'h0d;\r\n/* λ\ncomment */ assign x = data;\nendmodule\n", "last line   "] {
                    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let font = egui::TextStyle::Monospace.resolve(ui.style());
                        let color = ui.visuals().text_color();
                        let selected = Some((2, Color32::from_rgba_unmultiplied(180, 130, 40, 65)));
                        let actual = cache.galley(ui, text, &font, color, selected);
                        let job = cache.layout_job(text, &font, color, selected, dark);
                        let expected = ui.fonts_mut(|f| f.layout_job(job));
                        assert_eq!(actual.size(), expected.size(), "{language:?} {dark} {text:?}");
                        assert_eq!(actual.intrinsic_size(), expected.intrinsic_size());
                        assert_eq!(actual.rows.len(), expected.rows.len());
                        for (a, b) in actual.rows.iter().zip(&expected.rows) {
                            assert_eq!(a.rect(), b.rect());
                            assert_eq!(a.ends_with_newline, b.ends_with_newline);
                            assert_eq!(a.glyphs, b.glyphs);
                            assert_eq!(a.visuals, b.visuals);
                        }
                        for index in 0..=text.chars().count() {
                            assert_eq!(actual.pos_from_cursor(egui::text::CCursor::new(index)),
                                expected.pos_from_cursor(egui::text::CCursor::new(index)));
                        }
                    });
                }
            }
        }
    }

    #[test]
    fn highlighting_classes_and_spans_cover_verilog_constructs() {
        use TokenClass::*;
        let text = "`define WIDTH 8\n`ifdef TEST\nmodule top;\nreg [7:0] data; wire clk;\nalways @(posedge clk) data <= 8'h0d + 4'b1010 + 32'd5 + 1.25e-3 + 8'he-1;\nassign out = data & 'z;\n$display(\"a\\\"b\\\\c\"); // λ comment\n/* block\n comment */ endmodule\n";
        let expected = [
            ("`define", Directive),
            ("WIDTH", Identifier),
            ("8", Number),
            ("`ifdef", Directive),
            ("TEST", Identifier),
            ("module", Keyword),
            ("top", Identifier),
            (";", Plain),
            ("reg", Keyword),
            ("[", Plain),
            ("7", Number),
            (":", Plain),
            ("0", Number),
            ("]", Plain),
            ("data", Identifier),
            (";", Plain),
            ("wire", Keyword),
            ("clk", Identifier),
            (";", Plain),
            ("always", Keyword),
            ("@", Plain),
            ("(", Plain),
            ("posedge", Keyword),
            ("clk", Identifier),
            (")", Plain),
            ("data", Identifier),
            ("<=", Plain),
            ("8'h0d", Number),
            ("+", Plain),
            ("4'b1010", Number),
            ("+", Plain),
            ("32'd5", Number),
            ("+", Plain),
            ("1.25e-3", Number),
            ("+", Plain),
            ("8'he", Number),
            ("-", Plain),
            ("1", Number),
            (";", Plain),
            ("assign", Keyword),
            ("out", Identifier),
            ("=", Plain),
            ("data", Identifier),
            ("&", Plain),
            ("'z", Number),
            (";", Plain),
            ("$display", SystemTask),
            ("(", Plain),
            ("\"a\\\"b\\\\c\"", String),
            (")", Plain),
            (";", Plain),
            ("// λ comment", Comment),
            ("/* block\n", Comment),
            (" comment */", Comment),
            ("endmodule", Keyword),
        ];
        let mut cache = HighlightCache::new(Language::Verilog);
        cache.update(text);
        let mut byte_offset = 0;
        let actual: Vec<_> = cache
            .lines
            .iter()
            .flat_map(|line| {
                let offset = byte_offset;
                byte_offset += line.text.len();
                line.tokens.iter().map(move |t| {
                    assert_eq!(line.text[..t.bytes.start].chars().count(), t.start);
                    assert_eq!(line.text[..t.bytes.end].chars().count(), t.end);
                    (offset + t.bytes.start..offset + t.bytes.end, t.class)
                })
            })
            .collect();
        assert_eq!(actual.len(), expected.len());
        let mut end = 0;
        for ((span, class), (raw, expected_class)) in actual.iter().zip(expected) {
            let start = end + text[end..].find(raw).unwrap();
            assert!(text[end..start].chars().all(char::is_whitespace));
            assert_eq!(*span, start..start + raw.len(), "{raw}");
            assert_eq!(*class, expected_class, "{raw}");
            end = span.end;
        }
    }

    fn assert_same_as_fresh(cache: &HighlightCache, text: &str) {
        let mut fresh = HighlightCache::new(cache.language);
        fresh.update(text);
        assert_eq!(cache.lines.len(), fresh.lines.len());
        for (a, b) in cache.lines.iter().zip(&fresh.lines) {
            assert_eq!(a.text, b.text);
            assert_eq!(a.incoming, b.incoming);
            assert_eq!(a.outgoing, b.outgoing);
            let spans = |l: &SyntaxLine| {
                l.tokens
                    .iter()
                    .map(|t| (t.bytes.clone(), t.start, t.end, t.class))
                    .collect::<Vec<_>>()
            };
            assert_eq!(spans(a), spans(b));
        }
    }

    #[test]
    fn incremental_comments_propagate_until_the_cached_state_matches() {
        let original = "module top;\nwire a;\n/* mark */\nassign x = a;\n// stop\nendmodule\n";
        let mut cache = HighlightCache::new(Language::Verilog);
        cache.update(original);
        assert_eq!(cache.relexed, 7);
        assert!(!cache.update(original));
        assert_eq!(cache.relexed, 0);
        let opened = original.replace("/* mark */", "/* mark");
        cache.update(&opened);
        assert_eq!(cache.relexed, 5);
        assert!(cache.lines[3..]
            .iter()
            .flat_map(|l| &l.tokens)
            .all(|t| t.class == TokenClass::Comment));
        assert_same_as_fresh(&cache, &opened);
        let closed = opened.replace("// stop", "// stop */");
        cache.update(&closed);
        assert_eq!(cache.relexed, 3);
        assert_eq!(cache.lines[5].tokens[0].class, TokenClass::Keyword);
        assert_same_as_fresh(&cache, &closed);
        let edited = closed.replace("wire a;", "wire ab;");
        cache.update(&edited);
        assert_eq!(cache.relexed, 1);
        assert_same_as_fresh(&cache, &edited);
        let inserted = edited.replace("wire ab;\n", "wire ab;\nwire added;\n");
        cache.update(&inserted);
        assert_eq!(cache.relexed, 1, "shifted cached lines are reused");
        assert_same_as_fresh(&cache, &inserted);
        cache.update(&edited);
        assert_eq!(
            cache.relexed, 0,
            "deleting a state-neutral line needs no lexing"
        );
        assert_same_as_fresh(&cache, &edited);
    }

    #[test]
    fn incremental_strings_and_comment_delimiters_keep_their_state() {
        for language in [
            Language::Verilog,
            Language::SystemVerilog,
            Language::Vhdl,
            Language::Lpf,
            Language::Toml,
        ] {
            let text = "wire x = data;\n/* comment */\nwire y;\n";
            let mut cache = HighlightCache::new(language);
            cache.update(text);
            let opened = text.replace("data", "\"data");
            cache.update(&opened);
            assert_eq!(
                cache.relexed, 1,
                "{language:?}: an unclosed string ends at EOL"
            );
            assert_eq!(cache.lines[1].tokens[0].class, TokenClass::Comment);
            assert_eq!(
                cache.lines[2].tokens.last().unwrap().class,
                TokenClass::Plain
            );
            assert_eq!(cache.lines[0].outgoing, LexerState::Normal);
            assert_same_as_fresh(&cache, &opened);
        }
        let text = "text = data\n# remains a string\nlast\n# comment\n";
        let mut cache = HighlightCache::new(Language::Toml);
        cache.update(text);
        let opened = text.replace("data", "\"\"\"data");
        cache.update(&opened);
        assert_eq!(cache.relexed, 5);
        assert_eq!(cache.lines[1].tokens[0].class, TokenClass::String);
        assert_same_as_fresh(&cache, &opened);
        let closed = opened.replace("last", "last\"\"\"");
        cache.update(&closed);
        assert_eq!(cache.relexed, 3);
        assert_eq!(cache.lines[3].tokens[0].class, TokenClass::Comment);
        assert_same_as_fresh(&cache, &closed);
        let mut cache = HighlightCache::new(Language::Verilog);
        let based = "assign x = 16'h\n FF; wire y;";
        cache.update(based);
        assert_eq!(cache.lines[1].tokens[0].class, TokenClass::Number);
        assert_eq!(Syntax::from_lines(&cache.lines).identifier(2, 2), None);
    }

    #[test]
    fn lpf_and_toml_colour_comments_strings_and_headers() {
        for (language, text, expected) in [
            (
                Language::Lpf,
                "LOCATE COMP \"led[0]\" SITE \"A1\"; # pin\n",
                vec![
                    ("LOCATE", TokenClass::Keyword),
                    ("COMP", TokenClass::Keyword),
                    ("\"led[0]\"", TokenClass::String),
                    ("SITE", TokenClass::Keyword),
                    ("\"A1\"", TokenClass::String),
                    (";", TokenClass::Plain),
                    ("# pin", TokenClass::Comment),
                ],
            ),
            (
                Language::Toml,
                "[project] # settings\nname = \"a#b\"\npath = 'top.v'\n",
                vec![
                    ("[project] ", TokenClass::Keyword),
                    ("# settings", TokenClass::Comment),
                    ("name", TokenClass::Identifier),
                    ("=", TokenClass::Plain),
                    ("\"a#b\"", TokenClass::String),
                    ("path", TokenClass::Identifier),
                    ("=", TokenClass::Plain),
                    ("'top.v'", TokenClass::String),
                ],
            ),
        ] {
            let mut cache = HighlightCache::new(language);
            cache.update(text);
            let actual: Vec<_> = cache
                .lines
                .iter()
                .flat_map(|l| l.tokens.iter().map(|t| (&l.text[t.bytes.clone()], t.class)))
                .collect();
            assert_eq!(actual, expected);
        }
        let mut cache = HighlightCache::new(Language::Toml);
        let text = "text = \"\"\"first\n# string\nlast\"\"\"\n# comment\n";
        cache.update(text);
        assert_eq!(cache.lines[1].tokens[0].class, TokenClass::String);
        assert_eq!(cache.lines[3].tokens[0].class, TokenClass::Comment);
    }

    #[test]
    fn headless_highlighting_keeps_text_edit_geometry_and_probe_backgrounds() {
        let text = "module top;\n\treg [7:0] data = 8'h0d;\n/* λ\ncomment */ assign x = data;\n$display(\"a\\\"b\");\nendmodule\n";
        for dark in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::install(
                &ctx,
                if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                },
            );
            let mut cache = HighlightCache::new(Language::Verilog);
            let mut geometry = Vec::new();
            for coloured in [false, true] {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let color = ui.visuals().text_color();
                    let bg = Color32::from_rgba_unmultiplied(180, 130, 40, 65);
                    let mut layouter =
                        |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _: f32| {
                            let job = if coloured {
                                cache.layout_job(buf.as_str(), &font, color, Some((4, bg)), dark)
                            } else {
                                build_layout_job(buf.as_str(), &font, color, Some(4), bg)
                            };
                            if coloured {
                                let start = text.find("comment */").unwrap();
                                let end = start + text[start..].find('\n').unwrap() + 1;
                                for section in &job.sections {
                                    let range = usize::from(section.byte_range.start)
                                        ..usize::from(section.byte_range.end);
                                    if range.start < end && range.end > start {
                                        assert_eq!(section.format.background, bg);
                                    }
                                }
                                assert!(job.sections.iter().any(|s| s.format.color
                                    == crate::theme::syntax_colours(dark).keyword));
                            }
                            ui.fonts_mut(|f| f.layout_job(job))
                        };
                    let mut buffer = text.to_string();
                    let output = egui::TextEdit::multiline(&mut buffer)
                        .font(font.clone())
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .desired_width(600.0)
                        .layouter(&mut layouter)
                        .show(ui);
                    geometry.push((
                        output.response.rect,
                        output.galley.size(),
                        (0..=text.chars().count())
                            .map(|i| output.galley.pos_from_cursor(egui::text::CCursor::new(i)))
                            .collect::<Vec<_>>(),
                    ));
                });
            }
            assert_eq!(geometry[0], geometry[1], "dark={dark}");
            let mut editor = value_editor(text);
            editor.tabs[0].read_only = true;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                editor.show(ui, &super::super::DisplayPaths::new(None));
            });
            assert!(editor.tabs[0].highlighting.lines.is_empty());
            let galley = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == text => Some(&t.galley),
                    _ => None,
                })
                .unwrap();
            let theme = if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            };
            let color = ctx.style_of(theme).visuals.text_color();
            assert!(galley.job.sections.iter().all(|s| s.format.color == color));
        }
    }

    /// CPU-only comparison with the original plain layouter. Run in release
    /// mode explicitly, so routine tests never include a timing workload.
    #[test]
    #[ignore = "short headless typing timing; run in release with --ignored --nocapture"]
    fn picorv32_typing_layout_timings() {
        use std::time::Instant;
        let source = include_str!("../../../examples/ulx3s/picorv32_console/picorv32.v");
        let middle = source.len() / 2;
        let middle = middle + source[middle..].find('\n').unwrap() + 1;
        let positions = [
            ("idle", None),
            ("middle", Some(middle)),
            ("block opener", Some(source.find("/*").unwrap() + 1)),
            ("end", Some(source.len())),
        ];
        for dark in [false, true] {
            for (name, position) in positions {
                let mut medians = Vec::new();
                for coloured in [false, true] {
                    let ctx = egui::Context::default();
                    crate::theme::install(
                        &ctx,
                        if dark {
                            egui::ThemePreference::Dark
                        } else {
                            egui::ThemePreference::Light
                        },
                    );
                    let mut cache = HighlightCache::new(Language::Verilog);
                    let mut samples = Vec::new();
                    for sample in 0..19 {
                        let mut edited = source.to_string();
                        if let Some(position) = position {
                            edited.insert(position, (b'a' + sample) as char);
                        }
                        // Restore an open editor before each independent insertion.
                        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                            let font = egui::TextStyle::Monospace.resolve(ui.style());
                            let color = ui.visuals().text_color();
                            if coloured {
                                std::hint::black_box(cache.galley(ui, source, &font, color, None));
                            } else {
                                let job = build_layout_job(
                                    source,
                                    &font,
                                    color,
                                    None,
                                    Color32::TRANSPARENT,
                                );
                                std::hint::black_box(ui.fonts_mut(|f| f.layout_job(job)));
                            }
                        });
                        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                            let font = egui::TextStyle::Monospace.resolve(ui.style());
                            let color = ui.visuals().text_color();
                            let started = Instant::now();
                            if coloured {
                                std::hint::black_box(cache.galley(ui, &edited, &font, color, None));
                            } else {
                                let job = build_layout_job(
                                    &edited,
                                    &font,
                                    color,
                                    None,
                                    Color32::TRANSPARENT,
                                );
                                std::hint::black_box(ui.fonts_mut(|f| f.layout_job(job)));
                            }
                            if sample >= 3 {
                                samples.push(started.elapsed().as_secs_f64() * 1_000.0);
                            }
                        });
                        if coloured {
                            let limit = if name == "idle" {
                                0
                            } else if name == "block opener" {
                                20
                            } else {
                                1
                            };
                            assert!(cache.relexed <= limit, "lexed {}", cache.relexed);
                            assert!(cache.laid_out <= limit, "laid out {}", cache.laid_out);
                        }
                    }
                    samples.sort_by(f64::total_cmp);
                    medians.push(samples[samples.len() / 2]);
                }
                eprintln!("picorv32 {} {name}: plain {:.3}, highlighted {:.3} ms/frame (median, 16 samples, {})", if dark { "dark" } else { "light" }, medians[0], medians[1], if cfg!(debug_assertions) { "debug" } else { "release" });
                if !cfg!(debug_assertions) {
                    assert!(medians[1] <= medians[0] + 1.0, "{name}: {medians:?}");
                }
            }
        }
    }

    fn value_editor(text: &str) -> Editor {
        let mut editor = Editor::default();
        editor.tabs.push(Tab {
            path: "top.v".into(),
            label: "top.v".into(),
            text: text.into(),
            saved_text: text.into(),
            read_only: false,
            highlight_line: None,
            scroll_to_line: None,
            select_span: None,
            cursor: None,
            syntax: None,
            highlighting: HighlightCache::new(Language::Verilog),
        });
        editor
    }

    #[test]
    fn verilog_value_lexer_excludes_keywords_literals_comments_and_strings() {
        let text = "// λ signal\n/* multiline\n signal */ assign result = data[ 12 ] ^ 65'h1_ffff ^ 'z;\n$display(\"data \\\" result\"); \\escaped.name[3]  = foo_bar; // data\nwire x = 8'shff + 128'bxz01 + 'hdead;\n";
        let syntax = Syntax::parse(text);
        for excluded in [
            "assign",
            "wire",
            "65'h1_ffff",
            "'z",
            "8'shff",
            "128'bxz01",
            "'hdead",
            "$display",
        ] {
            let at = text.find(excluded).unwrap();
            for offset in 0..excluded.chars().count() {
                let (line, column) = line_column(text, text[..at].chars().count() + offset);
                assert_eq!(
                    syntax.identifier(line, column),
                    None,
                    "{excluded} at {offset}"
                );
            }
        }
        assert_eq!(syntax.identifier(1, 7), None);
        assert_eq!(syntax.identifier(3, 3), None);
        let strings = text.find("\"data").unwrap();
        let (line, column) = line_column(text, text[..strings].chars().count() + 2);
        assert_eq!(syntax.identifier(line, column), None);
        for (raw, expected) in [
            ("result", "result"),
            ("data[ 12 ]", "data[12]"),
            ("\\escaped.name[3]", "escaped.name[3]"),
            ("foo_bar", "foo_bar"),
        ] {
            let at = text.find(raw).unwrap();
            let (line, column) = line_column(text, text[..at].chars().count());
            assert_eq!(syntax.identifier(line, column).as_deref(), Some(expected));
        }
        assert_eq!(
            Syntax::parse("assign a = \\wire ;")
                .identifier(1, 13)
                .as_deref(),
            Some("wire")
        );
        let spaced = Syntax::parse("assign \\escaped.name [ 12 ] = 16'h FF; `MACRO");
        assert_eq!(spaced.identifier(1, 9).as_deref(), Some("escaped.name[12]"));
        assert_eq!(spaced.identifier(1, 36), None);
        assert_eq!(spaced.identifier(1, 41), None);
        assert_eq!(
            Syntax::parse("a[7:0] = b; array[idx] = c;")
                .assignments
                .len(),
            0
        );
    }

    #[test]
    fn inline_assignment_targets_are_cached_at_the_multiline_statement_end() {
        let text = "assign {high, low[2]} =\n  rhs + other;\nalways @(posedge clk) begin\n  if (enable && (test <= limit)) count <=\n    count + 1;\n  array[index] = rhs;\n  object.member = rhs;\n  for (i = 0; i < 4; i = i+1) array[i] = rhs;\nend\nwire [7:0] init = 8'hff;\nassign y = select ? a : b <= c;\nassign array[index] = select ? a : b <= c;\nassign array[index] = {a, b <= c};\nassign first = rhs, second[3] = other;\n";
        let syntax = Syntax::parse(text);
        let names = |line| {
            syntax
                .assignments
                .get(&line)
                .unwrap()
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(2), ["low[2]", "high"]);
        assert_eq!(names(5), ["count"]);
        assert_eq!(names(10), ["init"]);
        assert_eq!(names(11), ["y"]);
        assert_eq!(names(14), ["first", "second[3]"]);
        assert_eq!(
            syntax.assignments.len(),
            5,
            "unsupported LHS and relational <= are skipped"
        );
    }

    #[test]
    fn any_width_values_keep_high_bits_partial_nibbles_and_decimal_limits() {
        for (width, hex, decimal) in [
            (8, "0x81", Some(129)),
            (64, "0x8000_0000_0000_0001", Some((1u64 << 63) + 1)),
            (65, "0x1_0000_0000_0000_0001", None),
            (128, "0x8000_0000_0000_0000_0000_0000_0000_0001", None),
        ] {
            let mut bits = vec![Some(0); width];
            bits[0] = Some(1);
            bits[width - 1] = Some(1);
            assert_eq!(bus_value(&bits), (hex.into(), decimal), "{width} bits");
        }
        assert_eq!(
            bus_value(&[Some(1), Some(0), Some(1), Some(0), Some(1)]),
            ("0x15".into(), Some(21))
        );
        for x in [2, 3, 255] {
            assert_eq!(
                bus_value(&[Some(1), Some(x), Some(0), Some(0)]),
                ("0xx".into(), None)
            );
        }
        assert_eq!(
            bus_value(&[Some(2), None, Some(1), Some(0), Some(0)]),
            ("0xx?".into(), None)
        );
    }

    #[test]
    fn found_ranges_gaps_and_physical_alternatives_are_explicit() {
        let row = ValueRow {
            name: "cpu.x".into(),
            bits: (2..=8)
                .rev()
                .map(|i| ValueBit {
                    name: format!("cpu.x[{i}]"),
                    index: Some(i),
                    alternatives: match i {
                        8 => vec![(80, Some(1))],
                        6 | 2 => vec![],
                        5 => vec![(50, Some(2))],
                        _ => vec![(i, Some(0))],
                    },
                })
                .collect(),
        };
        let (values, details) = format_values(&[row]);
        assert_eq!(values, ["cpu.x = 0x?? (bits 8..2 found)"]);
        assert!(details.contains(&"missing: cpu.x[6], cpu.x[2]".into()));
        assert!(details.contains(&format!("x: {X_LABEL}")));
        assert!(!details.join(" ").contains("cpu.x[1]"));
        let row = ValueRow {
            name: "pin".into(),
            bits: vec![ValueBit {
                name: "pin".into(),
                index: None,
                alternatives: vec![(9, Some(0)), (4, Some(1)), (3, None)],
            }],
        };
        let (values, details) = format_values(&[row]);
        assert_eq!(values, ["pin = ?"]);
        assert_eq!(
            details[0],
            "pin alternatives: net 9 = 0, net 4 = 1, net 3 = ?"
        );
        assert_eq!(details[1], "missing: pin");
    }

    #[test]
    fn pointer_and_script_hover_share_current_buffer_lexer_without_text_changes() {
        let text = "// λ\nassign count = source;\n";
        let mut editor = value_editor(text);
        let paths = super::super::DisplayPaths::new(None);
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 300.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(raw(), |ui| {
            editor.show_values(ui, &paths, |_, _, _| ValueView::unavailable("no values"));
        });
        let (origin, galley) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some((t.pos, t.galley.clone())),
                _ => None,
            })
            .unwrap();
        let target = character_offset(text, 2, Some(9), false);
        let cursor = galley.pos_from_cursor(egui::text::CCursor::new(target));
        let pointer = origin + egui::vec2(cursor.min.x + 1.0, cursor.center().y);
        let mut seen = Vec::new();
        let mut input = raw();
        input.events = vec![egui::Event::PointerMoved(pointer)];
        let _ = ctx.run_ui(input, |ui| {
            editor.show_values(ui, &paths, |probe, _, hover| {
                if hover {
                    seen.push((probe.line, probe.column, probe.identifier.clone()));
                }
                ValueView::unavailable("no values")
            });
        });
        assert_eq!(seen, [(2, 9, "count".into())]);
        editor.script_hover(Path::new("top.v"), 2, 9);
        let mut input = raw();
        input.events = vec![egui::Event::PointerGone];
        let _ = ctx.run_ui(input, |ui| {
            editor.show_values(ui, &paths, |probe, _, hover| {
                if hover {
                    seen.push((probe.line, probe.column, probe.identifier.clone()));
                }
                ValueView::unavailable("no values")
            });
        });
        assert_eq!(seen[0], seen[1]);
        assert_eq!(editor.tabs[0].text, text);
        assert_eq!(editor.tabs[0].saved_text, text);
        assert!(!editor.tabs[0].dirty());
        assert!(!editor.inline_values);
        editor.tabs[0].text = text.replace("count", "renamed");
        editor.tabs[0].syntax = None;
        assert_eq!(
            Editor::hover_at(&mut editor.tabs[0], 2, 9)
                .unwrap()
                .identifier,
            "renamed"
        );
        assert!(editor.tabs[0].dirty());
    }

    #[test]
    fn inline_only_requests_visible_assignments_and_follows_scroll_coordinates() {
        let text = (0..200)
            .map(|i| format!("assign n{i} = {}rhs;\n", " ".repeat(120)))
            .collect::<String>();
        let mut editor = value_editor(&text);
        let ctx = egui::Context::default();
        let paths = super::super::DisplayPaths::new(None);
        ctx.global_style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        let frame = |editor: &mut Editor, seen: &mut Vec<String>, horizontal: bool| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 220.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    if horizontal {
                        let id = ui.make_persistent_id(egui::IdSalt::new((
                            "editor_scroll",
                            &editor.tabs[0].path,
                        )));
                        let mut state = egui::scroll_area::State::load(ui.ctx(), id).unwrap();
                        state.offset.x += 200.0;
                        state.store(ui.ctx(), id);
                    }
                    editor.show_values(ui, &paths, |probe, _, hover| {
                        assert!(!hover);
                        seen.push(probe.identifier.clone());
                        ValueView::unavailable("seeking…")
                    });
                },
            )
        };
        let mut seen = Vec::new();
        let _ = frame(&mut editor, &mut seen, false);
        assert!(seen.is_empty(), "toggle is off initially");
        editor.inline_values = true;
        let _ = frame(&mut editor, &mut seen, false);
        assert!(
            !seen.is_empty() && seen.len() < 15,
            "visible rows only: {seen:?}"
        );
        assert!(seen.iter().all(|s| s != "rhs"));
        editor.tabs[0].scroll_to_line = Some(150);
        let _ = frame(&mut editor, &mut Vec::new(), false);
        let _ = frame(&mut editor, &mut Vec::new(), false);
        seen.clear();
        let before = frame(&mut editor, &mut seen, false);
        assert!(
            seen.iter().any(|s| s == "n149"),
            "scroll invalidates visible targets: {seen:?}"
        );
        assert!(!seen.iter().any(|s| s == "n0"));
        assert_eq!(editor.tabs[0].text, text);
        assert!(!editor.tabs[0].dirty());
        let after = frame(&mut editor, &mut Vec::new(), true);
        let annotation = |out: &egui::FullOutput| {
            out.shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text().starts_with("n149:") => Some(t.pos),
                    _ => None,
                })
                .unwrap()
        };
        let before = annotation(&before);
        let after = annotation(&after);
        assert!(
            (before.x - after.x - 200.0).abs() < 0.1,
            "inline paint follows horizontal scrolling: {before:?} -> {after:?}"
        );
        assert_eq!(before.y, after.y);
    }

    #[test]
    fn annotations_leave_saved_source_and_character_coordinates_unchanged() {
        let root = std::env::temp_dir().join(format!("ecp5-inline-save-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("top.v");
        fs::write(&path, "assign count = source;\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&path, "top.v".into(), false, None).unwrap();
        editor.inline_values = true;
        editor
            .replace_line(&path, "top.v".into(), 1, "assign count = edited;")
            .unwrap();
        let ctx = egui::Context::default();
        let mut view = ValueView::unavailable("Values are not available yet");
        view.rows = vec!["count = 1".into()];
        view.cycle = Some(7);
        view.warning = Some("built from an older top.v: locations may be off".into());
        assert!(view.inline().contains("values from the running build"));
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            editor.show_values(ui, &super::super::DisplayPaths::new(None), |_, dirty, _| {
                assert!(dirty);
                view.clone()
            });
        });
        assert_eq!(
            editor.value_probe_at(&path, 1, 8).unwrap().identifier,
            "count"
        );
        assert_eq!(editor.save_active().unwrap(), Some(path.clone()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "assign count = edited;\n"
        );
        assert!(!editor.is_dirty(&path));
        assert_eq!(editor.save_active().unwrap(), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn value_records_use_exact_display_and_no_invented_number_when_unavailable() {
        let probe = Probe {
            path: "rtl/a file.v".into(),
            line: 4,
            column: 9,
            identifier: "count".into(),
        };
        let mut view = ValueView::unavailable("seeking…");
        assert_eq!(
            view.note("rtl/a file.v", &probe),
            "value rtl/a file.v:4:9 count -> seeking…"
        );
        view.rows = vec!["count = 0x81 (129) (bits 7..0 found)".into()];
        view.cycle = Some(42);
        assert_eq!(
            view.note("rtl/a file.v", &probe),
            "value rtl/a file.v:4:9 count = 0x81 (129) (bits 7..0 found) @ 42"
        );
    }

    #[test]
    fn tour_selection_uses_the_editor_buffer_and_selects_exactly_count() {
        let root = std::env::temp_dir().join(format!("ecp5_tour_editor_{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("top.v");
        fs::write(
            &path,
            include_str!("../../../examples/ulx3s/blinky_cycle/top.v"),
        )
        .unwrap();
        let mut editor = Editor::default();
        editor.open(&path, "top.v".into(), false, None).unwrap();
        let (line, _) = super::super::tour::counter_location(editor.text(&path).unwrap()).unwrap();
        editor
            .replace_line(&path, "top.v".into(), line, "    count <= count + 1;")
            .unwrap();
        let (line, column) =
            super::super::tour::counter_location(editor.text(&path).unwrap()).unwrap();
        let probe = editor.select_probe(&path, line, column).unwrap();
        assert_eq!(probe.identifier, "count");
        assert_eq!(probe.column, 5);
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(editor
                    .show(ui, &super::super::DisplayPaths::new(None))
                    .is_none());
            });
        }
        let tab = &editor.tabs[editor.active];
        let [start, end] = tab.cursor.unwrap().sorted_cursors();
        let selected: String = tab
            .text
            .chars()
            .skip(usize::from(start.index))
            .take(usize::from(end.index) - usize::from(start.index))
            .collect();
        assert_eq!(selected, "count");
        assert!(
            editor.is_dirty(&path),
            "probing must not save the user's edits"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_source_jump_switches_tabs_scrolls_highlights_and_selects_the_span() {
        let root = std::env::temp_dir().join(format!("ecp5_source_editor_{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let top = root.join("top.v");
        let cpu = root.join("cpu file.v");
        fs::write(&top, "module top;\nendmodule\n").unwrap();
        fs::write(&cpu, "module cpu;\nassign a = b & c;\nendmodule\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&cpu, "cpu file.v".into(), false, None).unwrap();
        editor.open(&top, "top.v".into(), false, None).unwrap();
        let span = Span {
            file: "cpu file.v".into(),
            line: 2,
            column: Some(12),
            end_line: 2,
            end_column: Some(13),
        };
        editor
            .open(&cpu, span.file.to_string(), false, Some(span.line))
            .unwrap();
        editor.select_span(span.clone());
        assert_eq!(editor.active_path(), Some(cpu.as_path()));
        assert_eq!(editor.tabs[editor.active].highlight_line, Some(2));
        assert_eq!(editor.tabs[editor.active].scroll_to_line, Some(2));
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor
                .show(ui, &super::super::DisplayPaths::new(None))
                .is_none());
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(
                editor
                    .show(ui, &super::super::DisplayPaths::new(None))
                    .is_none(),
                "a source jump must not probe back"
            );
        });
        assert!(
            ctx.egui_wants_keyboard_input(),
            "the selected span is visible in the focused editor"
        );
        let tab = &editor.tabs[editor.active];
        let range = tab.cursor.unwrap();
        let [start, end] = range.sorted_cursors();
        assert_eq!(
            tab.text
                .chars()
                .skip(usize::from(start.index))
                .take(usize::from(end.index) - usize::from(start.index))
                .collect::<String>(),
            "b"
        );
        assert!(tab.scroll_to_line.is_none());
        assert_eq!(editor.probe_at(&cpu, 2, 12).unwrap().identifier, "b");
        editor
            .replace_line(&cpu, "cpu file.v".into(), 2, "assign a = renamed;")
            .unwrap();
        assert!(editor.is_dirty(&cpu));
        assert_eq!(editor.probe_at(&cpu, 2, 12).unwrap().identifier, "renamed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coordinates_use_characters_and_line_only_spans_select_the_line() {
        let text = "// λ\nassign a = b;\n";
        assert_eq!(line_column(text, 5), (2, 1));
        let span = Span {
            file: "top.v".into(),
            line: 2,
            column: None,
            end_line: 2,
            end_column: None,
        };
        let (start, end) = span_range(text, &span);
        assert_eq!(
            text.chars()
                .skip(start)
                .take(end - start)
                .collect::<String>(),
            "assign a = b;"
        );
    }

    #[test]
    fn editor_tokens_use_ranked_locations_and_show_the_recorded_no_match_message() {
        use ecp5_emu::sources::{Collected, Files};
        let root = std::env::temp_dir().join(format!("ecp5_editor_probe_{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("top.v");
        fs::write(&path,"// comment_word\n\n\n\nmodule top;\n  reg [7:0] count = 0;\n  always @(posedge clk_25mhz) count <= count + 1;\nwire folded;\n").unwrap();
        let mut top = serde_json::json!({"netnames":{
            "folded":{"bits":[100],"attributes":{"src":"top.v:8.6-8.12"}}
        },"cells":{
            "ff":{"attributes":{"src":"top.v:7.3-7.50"},"port_directions":{"Q":"output"},"connections":{"Q":[1,2,3,4,5,6,7,8]}}
        }});
        for bit in 0..8 {
            top["netnames"][format!("count[{bit}]")] =
                serde_json::json!({"bits":[bit+1],"attributes":{"src":"top.v:6.13-6.18"}});
        }
        let cpu = root.join("picorv32.v");
        fs::write(
            &cpu,
            format!("{}\t\t\t\treg_pc <= current_pc;\n", "\n".repeat(1503)),
        )
        .unwrap();
        top["netnames"]["cpu.reg_pc[8]"] =
            serde_json::json!({"bits":[9],"attributes":{"src":"picorv32.v:159.13-159.19"}});
        top["netnames"]["process"] =
            serde_json::json!({"bits":[10],"attributes":{"src":"picorv32.v:1374.2-1946.5"}});
        top["cells"]["expression"] = serde_json::json!({"attributes":{"src":"picorv32.v:1504.5-1504.24"},"port_directions":{"Q":"output"},"connections":{"Q":[9]}});
        let index = Collected::from_top(
            &top,
            &Files::project(root.clone(), ["top.v".into(), "picorv32.v".into()]),
        )
        .build(|name| {
            if name == "folded" {
                vec![]
            } else {
                vec![top["netnames"][name]["bits"][0].as_u64().unwrap() as u32]
            }
        });
        let mut editor = Editor::default();
        editor.open(&path, "top.v".into(), false, None).unwrap();
        for (line, column, token) in [(6, 3, "reg"), (7, 3, "always")] {
            let click = editor.probe_at(&path, line, column).unwrap();
            assert_eq!(click.identifier, token);
            let probe = index.probe("top.v", line, column, &click.identifier);
            assert_eq!(probe.nets.len(), 8);
        }
        editor.open(&cpu, "picorv32.v".into(), false, None).unwrap();
        let click = editor.probe_at(&cpu, 1504, 12).unwrap();
        assert!(click.identifier.is_empty());
        assert_eq!(
            index.probe("picorv32.v", 1504, 12, &click.identifier).nets,
            [9]
        );
        editor.open(&path, "top.v".into(), false, None).unwrap();
        for (line, column, expected) in [
            (1, 4, "no net at top.v:1:4"),
            (
                8,
                6,
                "`folded` has no net on the chip (a constant, or inside a cell)",
            ),
        ] {
            let click = editor.probe_at(&path, line, column).unwrap();
            let probe = index.probe("top.v", line, column, &click.identifier);
            editor.probe_status = Some(probe.message.clone());
            let ctx = egui::Context::default();
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    editor.show(ui, &super::super::DisplayPaths::new(None));
                },
            );
            assert!(out
                .shapes
                .iter()
                .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text() == expected)));
            let record_path = root.join("trace.tsv");
            let mut trace = crate::trace::Trace::new(&record_path).unwrap();
            trace.note(&probe.note("top.v", line, column)).unwrap();
            drop(trace);
            assert!(fs::read_to_string(record_path)
                .unwrap()
                .lines()
                .any(|s| s == format!("# probe source top.v:{line}:{column} -> {expected}")));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
