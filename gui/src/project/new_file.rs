//! New-file choices shared by the project dialog and script dispatcher.
use anyhow::{ensure, Context, Result};
use ecp5_emu::flow::Project;
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Verilog,
    SystemVerilog,
    Vhdl,
    Constraints,
    Hex,
}

impl Kind {
    const ALL: [Self; 5] = [
        Self::Verilog,
        Self::SystemVerilog,
        Self::Vhdl,
        Self::Constraints,
        Self::Hex,
    ];

    pub fn from_path(path: &Path) -> Option<Self> {
        Some(match path.extension()?.to_str()? {
            "v" => Self::Verilog,
            "sv" => Self::SystemVerilog,
            "vhd" | "vhdl" => Self::Vhdl,
            "lpf" => Self::Constraints,
            "hex" => Self::Hex,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Verilog => "Verilog (.v)",
            Self::SystemVerilog => "SystemVerilog (.sv)",
            Self::Vhdl => "VHDL (.vhd)",
            Self::Constraints => "Constraints (.lpf)",
            Self::Hex => "Hex data (.hex)",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Verilog => "v",
            Self::SystemVerilog => "sv",
            Self::Vhdl => "vhd",
            Self::Constraints => "lpf",
            Self::Hex => "hex",
        }
    }

    fn is_hdl(self) -> bool {
        matches!(self, Self::Verilog | Self::SystemVerilog | Self::Vhdl)
    }
}

pub fn suggested_hdl(project: &Project) -> PathBuf {
    let first = project.hdl_roots().first();
    let kind = first
        .and_then(|p| Kind::from_path(p))
        .unwrap_or(Kind::Verilog);
    first
        .and_then(|p| p.parent())
        .unwrap_or(Path::new(""))
        .join("counter")
        .with_extension(kind.extension())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub path: PathBuf,
    pub name: Option<String>,
}

impl Request {
    pub fn parse(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let object = value
            .as_object()
            .context("new-file expects a JSON object")?;
        ensure!(
            object
                .keys()
                .all(|k| matches!(k.as_str(), "path" | "module" | "entity")),
            "unknown new-file option"
        );
        let path = object
            .get("path")
            .and_then(|v| v.as_str())
            .context("new-file needs a path string")?;
        let name = |key| {
            object
                .get(key)
                .map(|v| v.as_str().context("module/entity must be a string"))
                .transpose()
        };
        let module = name("module")?;
        let entity = name("entity")?;
        ensure!(
            module.is_none() || entity.is_none() || module == entity,
            "module and entity names disagree"
        );
        Ok(Self {
            path: PathBuf::from(path),
            name: module.or(entity).map(str::to_owned),
        })
    }
}

pub struct Dialog {
    pub path: String,
    pub name: String,
    pub kind: Kind,
    pub error: Option<String>,
    name_edited: bool,
    focus: bool,
}

pub enum Action {
    Create(Request),
    Cancel,
}

impl Dialog {
    pub fn new(path: PathBuf) -> Self {
        let mut dialog = Self {
            path: String::new(),
            name: String::new(),
            kind: Kind::Verilog,
            error: None,
            name_edited: false,
            focus: true,
        };
        dialog.set_path(path.to_string_lossy().into_owned());
        dialog
    }

    pub fn set_path(&mut self, path: String) {
        self.path = path;
        if let Some(kind) = Kind::from_path(Path::new(&self.path)) {
            self.kind = kind;
        }
        if !self.name_edited {
            self.name = Path::new(&self.path)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
        self.error = None;
    }

    pub fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.set_path(
            Path::new(&self.path)
                .with_extension(kind.extension())
                .to_string_lossy()
                .into_owned(),
        );
    }

    fn request(&self) -> Request {
        Request {
            path: PathBuf::from(&self.path),
            name: (self.kind.is_hdl() && !self.name.trim().is_empty()).then(|| self.name.clone()),
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        egui::Modal::new(egui::Id::new("new_file")).show(ctx, |ui| {
            ui.set_width(450.0);
            ui.heading("New file…");
            ui.label("Path (relative to project)");
            let mut path = self.path.clone();
            let field = ui.text_edit_singleline(&mut path);
            if self.focus {
                field.request_focus();
                self.focus = false;
            }
            if field.changed() {
                self.set_path(path);
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let mut kind = self.kind;
            egui::ComboBox::from_label("Kind")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for choice in Kind::ALL {
                        ui.selectable_value(&mut kind, choice, choice.label());
                    }
                });
            if kind != self.kind {
                self.set_kind(kind);
            }
            if self.kind.is_hdl() {
                ui.label(if self.kind == Kind::Vhdl {
                    "Entity name (optional)"
                } else {
                    "Module name (optional)"
                });
                if ui.text_edit_singleline(&mut self.name).changed() {
                    self.name_edited = true;
                    self.error = None;
                }
                ui.small("Defaults to the filename; instantiate it from your design when needed.");
            } else if self.kind == Kind::Hex {
                ui.small("Hex data joins auxiliary files, not memory images.");
            } else {
                ui.small("New constraints remain inactive until selected in ecp5.toml.");
            }
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    action = Some(Action::Create(self.request()));
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = Some(Action::Cancel);
                }
            });
        });
        action
    }
}
