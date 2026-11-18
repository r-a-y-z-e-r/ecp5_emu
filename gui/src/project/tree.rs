//! Left project tree: open-project bar, example picker, sources/outputs tree,
//! and build step progress rows.
use super::editor::Editor;
use ecp5_emu::flow::{Project, Step, StepStatus};
use eframe::egui::{self, AtomExt as _, Color32, RichText};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug)]
pub enum StepProgress {
    Pending,
    Running { started: Instant },
    Done { seconds: f64 },
    Skipped,
    Failed { seconds: Option<f64> },
    NotRun,
}

pub enum TreeAction {
    NewFile(PathBuf),
    MemoryImage {
        relative: PathBuf,
        use_as_image: bool,
    },
    OpenProject(PathBuf),
    OpenFile {
        path: PathBuf,
        label: String,
        read_only: bool,
    },
}

/// Lists the example projects of the repository: the folders of
/// `examples/ulx3s` (the reference designs) and `examples/gallery` (the
/// demos), each shown as `group/name`, those with Verilog only.
pub fn examples_list() -> Vec<(String, PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(repo) = manifest.parent() else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for group in ["ulx3s", "gallery"] {
        let Ok(entries) = fs::read_dir(repo.join("examples").join(group)) else {
            continue;
        };
        let mut found: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && (p.join("ecp5.toml").is_file()
                        || fs::read_dir(p).is_ok_and(|mut d| {
                            d.any(|e| {
                                e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "v"))
                            })
                        }))
            })
            .map(|p| {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                (format!("{group}/{name}"), p)
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        dirs.extend(found);
    }
    dirs
}

/// Executable steps only; projects without swapped images get a separate note.
pub fn default_steps(project: &Project) -> Vec<(Step, StepProgress)> {
    let mut steps = vec![
        (Step::Synthesis, StepProgress::Pending),
        (Step::PlaceRoute, StepProgress::Pending),
    ];
    if !project.memory_images.is_empty() {
        steps.push((Step::Swap, StepProgress::Pending));
    }
    steps.push((Step::Bitstream, StepProgress::Pending));
    steps
}

pub fn sync_step_status(steps: &mut Vec<(Step, StepProgress)>, final_steps: &[(Step, StepStatus)]) {
    for (step, status) in final_steps {
        let entry = steps.iter_mut().find(|(s, _)| s == step);
        let mapped = match status {
            StepStatus::Ran { seconds } => StepProgress::Done { seconds: *seconds },
            StepStatus::Skipped => StepProgress::Skipped,
            StepStatus::Failed => {
                let prev = entry.as_ref().and_then(|(_, p)| match p {
                    StepProgress::Failed { seconds } => *seconds,
                    StepProgress::Running { started } => Some(started.elapsed().as_secs_f64()),
                    _ => None,
                });
                StepProgress::Failed { seconds: prev }
            }
            StepStatus::NotRun => StepProgress::NotRun,
        };
        if let Some((_, existing)) = entry {
            *existing = mapped;
        } else {
            steps.push((*step, mapped));
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    path_input: &mut String,
    project: &Result<Project, String>,
    steps: &[(Step, StepProgress)],
    examples: &[(String, PathBuf)],
    dev_mode: bool,
    editor: &Editor,
    inactive_constraints: &[PathBuf],
    busy: bool,
    paths: &super::DisplayPaths,
) -> Option<TreeAction> {
    let mut action = None;

    ui.heading("Project");
    ui.add_enabled_ui(!busy, |ui| {
        ui.label(RichText::new("Open project").small().weak());
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(path_input)
                    .desired_width(ui.available_width() - 56.0)
                    .hint_text("project folder or ecp5.toml"),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Open").clicked() || enter) && !path_input.trim().is_empty() {
                let dir = project
                    .as_ref()
                    .ok()
                    .filter(|p| path_input.trim() == paths.path(&p.root))
                    .map_or_else(|| paths.expand(path_input), |p| p.root.clone());
                action = Some(TreeAction::OpenProject(dir));
            }
        });
        if !examples.is_empty() {
            ui.menu_button("Examples ▾", |ui| {
                let catalogue = ecp5_emu::flow::demo_copy::catalogue().unwrap_or_default();
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        if let Some(path) =
                            example_choices(ui, examples, &catalogue, dev_mode, paths)
                        {
                            *path_input = paths.path(&path);
                            action = Some(TreeAction::OpenProject(path));
                        }
                    });
            });
        }
    });
    ui.separator();

    let project = match project {
        Ok(p) => p,
        Err(err) => {
            ui.colored_label(Color32::LIGHT_RED, paths.text(err));
            return action;
        }
    };

    egui::ScrollArea::vertical()
        .id_salt("project_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let root_row = ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{} ({}/{})",
                        paths.text(&project.top),
                        paths.text(&project.device.part),
                        paths.text(&project.device.package)
                    ))
                    .small()
                    .weak(),
                )
                .sense(egui::Sense::click()),
            );
            new_file_menu(
                &root_row,
                busy,
                super::new_file::suggested_hdl(project),
                &mut action,
            );
            ui.add_space(4.0);

            if let Some(file) = &project.file {
                egui::CollapsingHeader::new("Project file")
                    .default_open(true)
                    .show(ui, |ui| {
                        source_row(ui, file, "ecp5.toml", editor, &mut action);
                    });
            }

            let sources =
                egui::CollapsingHeader::new(format!("Verilog ({})", project.verilog.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        for rel in &project.verilog {
                            let full = project.root.join(rel);
                            source_row(ui, &full, &paths.path(rel), editor, &mut action);
                        }
                    });

            new_file_menu(
                &sources.header_response,
                busy,
                super::new_file::suggested_hdl(project),
                &mut action,
            );

            let constraints = egui::CollapsingHeader::new("Constraints")
                .default_open(true)
                .show(ui, |ui| {
                    let full = project.root.join(&project.constraints);
                    source_row(
                        ui,
                        &full,
                        &paths.path(&project.constraints),
                        editor,
                        &mut action,
                    );
                    for relative in inactive_constraints
                        .iter()
                        .filter(|p| *p != &project.constraints)
                    {
                        source_row(
                            ui,
                            &project.root.join(relative),
                            &paths.path(relative),
                            editor,
                            &mut action,
                        );
                    }
                    if !inactive_constraints.is_empty() {
                        ui.small("New LPFs are inactive until selected in ecp5.toml.");
                    }
                });

            new_file_menu(
                &constraints.header_response,
                busy,
                project.constraints.with_file_name("constraints.lpf"),
                &mut action,
            );

            if !project.files.is_empty() {
                let files = egui::CollapsingHeader::new(format!("Files ({})", project.files.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        for rel in &project.files {
                            let full = project.root.join(rel);
                            let row = source_row(ui, &full, &paths.path(rel), editor, &mut action);
                            memory_image_menu(&row, rel, false, busy, &mut action);
                        }
                    });
                new_file_menu(
                    &files.header_response,
                    busy,
                    PathBuf::from("data.hex"),
                    &mut action,
                );
            }

            if !project.memory_images.is_empty() {
                let images = egui::CollapsingHeader::new(format!(
                    "Memory images ({})",
                    project.memory_images.len()
                ))
                .default_open(true)
                .show(ui, |ui| {
                    for rel in &project.memory_images {
                        let full = project.root.join(rel);
                        let row = source_row(ui, &full, &paths.path(rel), editor, &mut action);
                        memory_image_menu(&row, rel, true, busy, &mut action);
                    }
                });
                new_file_menu(
                    &images.header_response,
                    busy,
                    PathBuf::from("data.hex"),
                    &mut action,
                );
            }

            let out = project.outputs();
            let output_path = |path: &Path| paths.path(&project_relative_path(&project.root, path));
            egui::CollapsingHeader::new("Outputs")
                .default_open(true)
                .show(ui, |ui| {
                    output_row(
                        ui,
                        "top.config",
                        &out.config,
                        &output_path(&out.config),
                        false,
                        editor,
                        &mut action,
                    );
                    output_row(
                        ui,
                        "top_routed.json",
                        &out.routed_json,
                        &output_path(&out.routed_json),
                        false,
                        editor,
                        &mut action,
                    );
                    output_row(
                        ui,
                        "top.bit",
                        &out.bit,
                        &output_path(&out.bit),
                        false,
                        editor,
                        &mut action,
                    );
                    output_row(
                        ui,
                        "yosys.log",
                        &out.yosys_log,
                        &output_path(&out.yosys_log),
                        true,
                        editor,
                        &mut action,
                    );
                    output_row(
                        ui,
                        "nextpnr.log",
                        &out.nextpnr_log,
                        &output_path(&out.nextpnr_log),
                        true,
                        editor,
                        &mut action,
                    );
                });

            ui.separator();
            ui.label(RichText::new("Build steps").strong());
            ui.add_space(2.0);
            build_step_rows(ui, project, steps, paths);
        });

    action
}

fn example_choices(
    ui: &mut egui::Ui,
    examples: &[(String, PathBuf)],
    catalogue: &ecp5_emu::flow::demo_copy::Catalogue,
    dev_mode: bool,
    paths: &super::DisplayPaths,
) -> Option<PathBuf> {
    let mut action = None;
    if !dev_mode && catalogue.visible(false).next().is_none() {
        ui.label(crate::start::NO_SHOWCASE);
    }
    for demo in catalogue.visible(dev_mode) {
        let Some((name, path)) = examples.iter().find(|(name, _)| *name == demo.name) else {
            continue;
        };
        ui.horizontal(|ui| {
            if ui.button(paths.text(name)).clicked() {
                action = Some(path.clone());
                ui.close();
            }
            if !demo.showcase {
                crate::start::dev_tag(ui);
            }
        });
    }
    action
}

fn new_file_menu(
    response: &egui::Response,
    busy: bool,
    suggested: PathBuf,
    action: &mut Option<TreeAction>,
) {
    response.context_menu(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("New file…"))
            .on_disabled_hover_text("Wait for the current build or emulation to finish")
            .clicked()
        {
            *action = Some(TreeAction::NewFile(suggested.clone()));
            ui.close();
        }
    });
}

fn memory_image_menu(
    response: &egui::Response,
    relative: &Path,
    is_image: bool,
    busy: bool,
    action: &mut Option<TreeAction>,
) {
    if !matches!(
        relative.extension().and_then(|s| s.to_str()),
        Some("hex" | "mem" | "init")
    ) {
        return;
    }
    response.context_menu(|ui| {
        let label = if is_image {
            "Stop using as BRAM image"
        } else {
            "Use as BRAM image (ecpbram)…"
        };
        if ui
            .add_enabled(!busy, egui::Button::new(label))
            .on_disabled_hover_text("Wait for the current build or emulation to finish")
            .clicked()
        {
            *action = Some(TreeAction::MemoryImage {
                relative: relative.to_owned(),
                use_as_image: !is_image,
            });
            ui.close();
        }
    });
}

/// Output overrides may be outside the project; show those paths relatively too.
fn project_relative_path(root: &Path, path: &Path) -> PathBuf {
    let mut base = root;
    let mut relative = PathBuf::new();
    loop {
        if let Ok(tail) = path.strip_prefix(base) {
            relative.push(tail);
            return relative;
        }
        let Some(parent) = base.parent() else {
            return path.to_path_buf();
        };
        relative.push("..");
        base = parent;
    }
}

fn source_row(
    ui: &mut egui::Ui,
    full: &Path,
    label: &str,
    editor: &Editor,
    action: &mut Option<TreeAction>,
) -> egui::Response {
    let dirty = editor.is_dirty(full);
    let active = editor.active_path() == Some(full);
    let display = if dirty {
        format!("{label} *")
    } else {
        label.to_string()
    };
    let text = RichText::new(display).monospace();
    let response = file_row(ui, text, active, None).on_hover_text(label);
    if response.clicked() {
        *action = Some(TreeAction::OpenFile {
            path: full.to_path_buf(),
            label: label.to_string(),
            read_only: false,
        });
    }
    response
}

fn output_row(
    ui: &mut egui::Ui,
    name: &str,
    path: &Path,
    tooltip: &str,
    clickable_log: bool,
    editor: &Editor,
    action: &mut Option<TreeAction>,
) {
    let meta = fs::metadata(path).ok();
    let size_text = match &meta {
        Some(m) => format_bytes(m.len()),
        None => "missing".to_string(),
    };
    let present = meta.is_some();
    if clickable_log && present {
        let active = editor.active_path() == Some(path);
        if file_row(
            ui,
            RichText::new(name).monospace(),
            active,
            Some(RichText::new(size_text).small().weak()),
        )
        .on_hover_text(format!("{tooltip}\nOpen log (read-only)"))
        .clicked()
        {
            *action = Some(TreeAction::OpenFile {
                path: path.to_path_buf(),
                label: name.to_string(),
                read_only: true,
            });
        }
    } else {
        // Keep these labels selectable as text, including double-click and copy.
        let background = ui.painter().add(egui::Shape::Noop);
        let row = ui.horizontal(|ui| {
            let color = if present {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, RichText::new(name).monospace())
                .on_hover_text(tooltip);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(size_text).small().weak())
                    .on_hover_text(tooltip);
            });
        });
        let mut rect = row.response.rect;
        rect.min.x = ui.clip_rect().left();
        rect.max.x = ui.clip_rect().right();
        let response = ui
            .interact(rect, row.response.id, egui::Sense::hover())
            .on_hover_text(tooltip);
        if response.contains_pointer() {
            let visuals = &ui.visuals().widgets.hovered;
            ui.painter().set(
                background,
                egui::Shape::rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill),
            );
        }
    }
}

/// Use the visible panel width for interaction and painting, while retaining
/// the label's indentation inside a collapsing header and the size column.
fn file_row(
    ui: &mut egui::Ui,
    text: RichText,
    active: bool,
    size: Option<RichText>,
) -> egui::Response {
    let available = ui.available_rect_before_wrap();
    let mut rect = available;
    rect.min.x = ui.clip_rect().left();
    rect.max.x = ui.clip_rect().right();
    let indent = available.left() - rect.left();
    let mut atoms = egui::Atoms::new(text);
    if indent > 0.0 {
        atoms.push_left(egui::Atom::default().atom_size(egui::vec2(
            (indent - ui.spacing().icon_spacing).max(0.0),
            0.0,
        )));
    }
    if let Some(size) = size {
        atoms.push_right(egui::Atom::grow());
        atoms.push_right(size);
    }
    let mut row_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let row =
        row_ui.add(egui::Button::selectable(active, atoms).min_size(egui::vec2(rect.width(), 0.0)));
    // Advancing the indented parent must not move the following labels left.
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
        available.min,
        egui::vec2(available.width(), row.rect.height()),
    ));
    row
}

const NO_MEMORY_IMAGES: &str =
    "Memory images (ecpbram): none listed — $readmemh files are built in by Yosys";
const MEMORY_IMAGES_RULE: &str = "ecpbram swaps only the images listed in ecp5.toml [memory].images, or in ecpbram.txt when that setting is absent. The swap happens after place and route, so new firmware can reuse the placed design. $readmemh files that are not listed are built in by Yosys; changing them reruns synthesis.";

fn build_step_rows(
    ui: &mut egui::Ui,
    project: &Project,
    steps: &[(Step, StepProgress)],
    paths: &super::DisplayPaths,
) {
    let mut memory_tooltip = MEMORY_IMAGES_RULE.to_string();
    if project.memory_images.is_empty() {
        ui.label(RichText::new(NO_MEMORY_IMAGES).small().weak())
            .on_hover_text(&memory_tooltip);
    } else {
        memory_tooltip.push_str("\n\nSwaps:\n");
        memory_tooltip.push_str(
            &project
                .memory_images
                .iter()
                .map(|path| paths.path(path))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    for (step, state) in steps {
        step_row(
            ui,
            *step,
            state,
            (*step == Step::Swap).then_some(memory_tooltip.as_str()),
        );
    }
}

fn step_row(
    ui: &mut egui::Ui,
    step: Step,
    state: &StepProgress,
    details: Option<&str>,
) -> egui::InnerResponse<(egui::Response, Option<egui::Response>)> {
    let name = step.to_string();
    let status = match state {
        StepProgress::Running { started } => Some(
            RichText::new(format!("{:.1} s", started.elapsed().as_secs_f64()))
                .monospace()
                .small(),
        ),
        StepProgress::Done { seconds }
        | StepProgress::Failed {
            seconds: Some(seconds),
        } => Some(RichText::new(format!("{seconds:.1} s")).monospace().small()),
        StepProgress::Skipped => Some(RichText::new("inputs unchanged").small().weak()),
        StepProgress::NotRun => Some(RichText::new("not run").small().weak()),
        StepProgress::Pending | StepProgress::Failed { seconds: None } => None,
    };
    let mut tooltip = format!(
        "{name}\n{}",
        status.as_ref().map_or(
            if matches!(state, StepProgress::Pending) {
                "pending"
            } else {
                "failed"
            },
            RichText::text,
        )
    );
    if let Some(details) = details {
        tooltip.push('\n');
        tooltip.push_str(details);
    }
    let mut row = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            // Reserve the status first. At the panel's minimum width it also
            // elides, leaving room for the icon and the step name.
            let status = status.map(|text| {
                let galley = egui::WidgetText::from(text).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    ui.available_width() * 0.5,
                    egui::FontSelection::Default,
                );
                ui.label(galley).on_hover_text(&tooltip)
            });
            let name = ui
                .with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let mut text = RichText::new(&name);
                    match state {
                        StepProgress::Pending => {
                            ui.label(RichText::new("○").weak());
                        }
                        StepProgress::Running { .. } => {
                            ui.spinner();
                            text = text.strong();
                        }
                        StepProgress::Done { .. } => {
                            ui.colored_label(Color32::LIGHT_GREEN, "✔");
                        }
                        StepProgress::Skipped => {
                            ui.colored_label(Color32::from_gray(160), "–");
                        }
                        StepProgress::Failed { .. } => {
                            ui.colored_label(Color32::LIGHT_RED, "✖");
                            text = text.color(Color32::LIGHT_RED);
                        }
                        StepProgress::NotRun => {
                            ui.label(RichText::new("·").weak());
                            text = text.weak();
                        }
                    }
                    ui.add(
                        egui::Label::new(text)
                            .truncate()
                            .show_tooltip_when_elided(false),
                    )
                    .on_hover_text(&tooltip)
                })
                .inner;
            (name, status)
        },
    );
    row.response = row.response.on_hover_text(tooltip);
    row
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
