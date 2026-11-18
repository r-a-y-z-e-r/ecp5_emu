//! Bottom output panel: Messages, Log (capped at 20 000 lines), and Report
//! (utilisation table, timing table, synthesis cells, summary).
use ecp5_emu::flow::{Message, Project, Report, Severity, Step};
use eframe::egui::{self, Color32, RichText};
use std::{collections::VecDeque, path::PathBuf};

pub const LOG_CAP: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputTab {
    Messages,
    Log,
    Report,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Step(Step),
}

impl LogSource {
    pub fn short(self) -> &'static str {
        match self {
            Self::Step(Step::Synthesis) => "yosys",
            Self::Step(Step::PlaceRoute) => "nextpnr",
            Self::Step(Step::Swap) => "ecpbram",
            Self::Step(Step::Bitstream) => "ecppack",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFilter {
    All,
    Source(LogSource),
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub source: LogSource,
    pub text: String,
}

/// Ring buffer keeping at most the last `LOG_CAP` lines.
pub struct LogBuffer {
    lines: VecDeque<LogLine>,
    cap: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::with_cap(LOG_CAP)
    }
}

impl LogBuffer {
    pub fn with_cap(cap: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(cap.min(1024)),
            cap,
        }
    }

    pub fn push(&mut self, source: LogSource, text: impl Into<String>) {
        if self.cap == 0 {
            return;
        }
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(LogLine {
            source,
            text: text.into(),
        });
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }
}

#[derive(Clone, Debug)]
pub struct MessageFilter {
    pub errors: bool,
    pub warnings: bool,
    pub info: bool,
}

impl Default for MessageFilter {
    fn default() -> Self {
        Self {
            errors: true,
            warnings: true,
            info: false,
        }
    }
}

impl MessageFilter {
    pub fn matches(&self, severity: Severity) -> bool {
        match severity {
            Severity::Error => self.errors,
            Severity::Warning => self.warnings,
            Severity::Info => self.info,
        }
    }
}

/// Resolved editor target for a diagnostic message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageLocation {
    pub path: PathBuf,
    pub label: String,
    pub line: Option<u32>,
}

/// Maps a diagnostic `Message` to a file path and label within the project.
pub fn resolve_message_location(
    project: Option<&Project>,
    msg: &Message,
) -> Option<MessageLocation> {
    let file = msg.file.as_ref()?;
    let label = super::DisplayPaths::new(project.map(|p| p.root.as_path())).path(file);
    Some(MessageLocation {
        path: file.clone(),
        label,
        line: msg.line,
    })
}

pub fn visible_messages<'a>(
    messages: &'a [Message],
    filter: &MessageFilter,
) -> Vec<(usize, &'a Message)> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| filter.matches(m.severity))
        .collect()
}

pub struct OutputState {
    pub tab: OutputTab,
    pub filter: MessageFilter,
    pub log_filter: LogFilter,
    pub show_all_resources: bool,
    pub critical_clock: Option<String>,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            tab: OutputTab::Messages,
            filter: MessageFilter::default(),
            log_filter: LogFilter::All,
            show_all_resources: false,
            critical_clock: None,
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut OutputState,
    project: Option<&Project>,
    messages: &[Message],
    log: &LogBuffer,
    report: Option<&Report>,
    busy: bool,
    paths: &super::DisplayPaths,
) -> Option<MessageLocation> {
    let errors = messages
        .iter()
        .filter(|m| m.severity == Severity::Error)
        .count();
    let warnings = messages
        .iter()
        .filter(|m| m.severity == Severity::Warning)
        .count();

    ui.horizontal(|ui| {
        let msg_label = if errors > 0 || warnings > 0 {
            format!("Messages ({errors}E, {warnings}W)")
        } else {
            "Messages".to_string()
        };
        ui.selectable_value(&mut state.tab, OutputTab::Messages, msg_label);
        ui.selectable_value(
            &mut state.tab,
            OutputTab::Log,
            format!("Log ({})", log.len()),
        );
        ui.selectable_value(&mut state.tab, OutputTab::Report, "Report");
    });
    ui.separator();

    match state.tab {
        OutputTab::Messages => show_messages(ui, &mut state.filter, project, messages, paths),
        OutputTab::Log => {
            show_log(ui, &mut state.log_filter, project, log, busy, paths);
            None
        }
        OutputTab::Report => {
            state.critical_clock = show_report(ui, &mut state.show_all_resources, report, paths);
            None
        }
    }
}

fn show_messages(
    ui: &mut egui::Ui,
    filter: &mut MessageFilter,
    project: Option<&Project>,
    messages: &[Message],
    paths: &super::DisplayPaths,
) -> Option<MessageLocation> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.checkbox(&mut filter.errors, "Errors");
        ui.checkbox(&mut filter.warnings, "Warnings");
        ui.checkbox(&mut filter.info, "Info");
    });
    ui.separator();

    let visible = visible_messages(messages, filter);
    if visible.is_empty() {
        ui.label(RichText::new("No matching messages.").weak());
        return None;
    }
    egui::ScrollArea::vertical()
        .id_salt("messages_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (_, msg) in visible {
                let color = match msg.severity {
                    Severity::Error => Color32::LIGHT_RED,
                    Severity::Warning => Color32::from_rgb(245, 195, 70),
                    Severity::Info => Color32::from_gray(175),
                };
                let loc = resolve_message_location(project, msg).map(|mut loc| {
                    loc.label = paths.path(&loc.path);
                    loc
                });
                let loc_prefix = match (&loc, msg.line) {
                    (Some(l), Some(line)) => format!("{}:{line}: ", l.label),
                    (Some(l), None) => format!("{}: ", l.label),
                    (None, _) => String::new(),
                };
                let count_suffix = if msg.count > 1 {
                    format!("  ({} times)", msg.count)
                } else {
                    String::new()
                };
                let line = format!("{loc_prefix}{}: {}{count_suffix}", msg.severity, msg.text);
                let resp = ui.add(
                    egui::Label::new(RichText::new(paths.text(&line)).color(color).monospace())
                        .sense(if loc.is_some() {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        })
                        .wrap(),
                );
                if let Some(loc) = loc {
                    let resp = resp.on_hover_text("Click to open in editor");
                    if resp.clicked() {
                        clicked = Some(loc);
                    }
                }
            }
        });
    clicked
}

fn show_log(
    ui: &mut egui::Ui,
    filter: &mut LogFilter,
    project: Option<&Project>,
    log: &LogBuffer,
    busy: bool,
    paths: &super::DisplayPaths,
) {
    let has_swap = project.is_some_and(|p| !p.memory_images.is_empty())
        || log.iter().any(|l| l.source == LogSource::Step(Step::Swap));
    ui.horizontal(|ui| {
        ui.selectable_value(filter, LogFilter::All, "All");
        ui.selectable_value(
            filter,
            LogFilter::Source(LogSource::Step(Step::Synthesis)),
            "Synthesis",
        );
        ui.selectable_value(
            filter,
            LogFilter::Source(LogSource::Step(Step::PlaceRoute)),
            "Place & Route",
        );
        if has_swap {
            ui.selectable_value(
                filter,
                LogFilter::Source(LogSource::Step(Step::Swap)),
                "Swap",
            );
        }
        ui.selectable_value(
            filter,
            LogFilter::Source(LogSource::Step(Step::Bitstream)),
            "Bitstream",
        );
    });
    ui.separator();

    let filtered: Vec<&LogLine> = log
        .iter()
        .filter(|l| match *filter {
            LogFilter::All => true,
            LogFilter::Source(src) => l.source == src,
        })
        .collect();

    let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(14.0);
    egui::ScrollArea::both()
        .id_salt("log_scroll")
        .stick_to_bottom(busy)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, filtered.len(), |ui, range| {
            for entry in &filtered[range] {
                ui.add(
                    egui::Label::new(
                        RichText::new(paths.text(&format!(
                            "[{}] {}",
                            entry.source.short(),
                            entry.text
                        )))
                        .monospace(),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true),
                );
            }
        });
}

fn show_report(
    ui: &mut egui::Ui,
    show_all: &mut bool,
    report: Option<&Report>,
    paths: &super::DisplayPaths,
) -> Option<String> {
    let mut critical = None;
    let Some(report) = report else {
        ui.label(RichText::new("No build report yet.").weak());
        return None;
    };

    egui::ScrollArea::vertical()
        .id_salt("report_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(
                RichText::new(paths.text(&report.summary()))
                    .monospace()
                    .strong(),
            );
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.strong("Utilisation");
                ui.checkbox(show_all, "Show all");
            });
            egui::Grid::new("utilisation_grid")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    for res in &report.utilisation {
                        if !*show_all && res.used == 0 {
                            continue;
                        }
                        let frac = if res.available > 0 {
                            (res.used as f32 / res.available as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let pct = if res.available > 0 {
                            res.used as f64 * 100.0 / res.available as f64
                        } else {
                            0.0
                        };
                        ui.label(paths.text(&res.label));
                        ui.label(
                            RichText::new(format!("{} / {}", res.used, res.available)).monospace(),
                        );
                        thin_bar(ui, frac);
                        ui.label(RichText::new(format!("{pct:.1}%")).monospace());
                        ui.end_row();
                    }
                });

            if !report.clocks.is_empty() {
                ui.add_space(8.0);
                ui.strong("Timing");
                egui::Grid::new("timing_grid")
                    .striped(true)
                    .num_columns(5)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Clock").weak().small());
                        ui.label(RichText::new("Achieved").weak().small());
                        ui.label(RichText::new("Target").weak().small());
                        ui.label(RichText::new("Result").weak().small());
                        ui.label("");
                        ui.end_row();
                        for clock in &report.clocks {
                            ui.label(RichText::new(paths.text(&clock.net)).monospace());
                            ui.label(
                                RichText::new(format!("{:.2} MHz", clock.achieved_mhz)).monospace(),
                            );
                            let target = clock
                                .target_mhz
                                .map_or_else(|| "n/a".to_string(), |t| format!("{t:.2} MHz"));
                            ui.label(RichText::new(target).monospace());
                            if clock.pass {
                                ui.colored_label(Color32::LIGHT_GREEN, "PASS");
                            } else {
                                ui.colored_label(Color32::LIGHT_RED, "FAIL");
                            }
                            if ui.button("Show on the chip").clicked() {
                                critical = Some(clock.name.clone());
                            }
                            ui.end_row();
                        }
                    });
            }

            if !report.max_delays.is_empty() {
                ui.add_space(6.0);
                for delay in &report.max_delays {
                    ui.label(RichText::new(paths.text(delay)).monospace().small().weak());
                }
            }

            if !report.synthesis_cells.is_empty() {
                ui.add_space(8.0);
                ui.strong("Synthesis cells");
                egui::Grid::new("cells_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (cell, count) in &report.synthesis_cells {
                            ui.label(RichText::new(paths.text(cell)).monospace());
                            ui.label(RichText::new(count.to_string()).monospace());
                            ui.end_row();
                        }
                    });
            }
        });
    critical
}

fn thin_bar(ui: &mut egui::Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(100.0, 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(45));
    if fraction > 0.0 {
        let filled = egui::Rect::from_min_size(
            rect.min,
            egui::vec2((rect.width() * fraction).max(2.0), rect.height()),
        );
        let color = if fraction > 0.9 {
            Color32::LIGHT_RED
        } else if fraction > 0.7 {
            Color32::from_rgb(245, 195, 70)
        } else {
            Color32::from_rgb(90, 175, 120)
        };
        ui.painter().rect_filled(filled, 2.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_button_returns_its_clock_in_both_themes() {
        let report = Report::parse(
            "",
            include_str!("../../../tests/fixtures/timing_path/blinky_cycle/nextpnr.log"),
            "top",
        );
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut state = OutputState {
                tab: OutputTab::Report,
                ..Default::default()
            };
            let mut frame = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui,
                            &mut state,
                            None,
                            &[],
                            &LogBuffer::default(),
                            Some(&report),
                            false,
                            &super::super::DisplayPaths::new(None),
                        );
                    },
                )
            };
            let out = frame(vec![]);
            let pos = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Show on the chip" => {
                        Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                    }
                    _ => None,
                })
                .unwrap();
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            assert_eq!(
                state.critical_clock.as_deref(),
                Some("$glbnet$clk_25mhz$TRELLIS_IO_IN")
            );
        }
    }
}
