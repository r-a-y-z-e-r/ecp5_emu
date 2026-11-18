//! Shared creation dialogs and commands; publication belongs to flow, builds to ProjectApp.
use crate::start::{self, display_paths::DisplayPaths};
use anyhow::{bail, ensure, Context, Result};
use ecp5_emu::flow::{
    self, demo_copy,
    new_project::{Resource, DEPENDENCIES},
};
use eframe::egui;
use std::{collections::BTreeSet, path::PathBuf};

pub const RESOURCES: [(Resource, &str, &str); 14] = [
    (Resource::Clock, "clock", "Clock"),
    (Resource::Leds, "leds", "LEDs"),
    (Resource::Buttons, "buttons", "Buttons"),
    (Resource::Switches, "switches", "DIP switches"),
    (Resource::Uart, "uart", "UART"),
    (Resource::Oled, "oled", "OLED (SSD1331 SPI)"),
    (Resource::Hdmi, "hdmi", "HDMI / GPDI"),
    (Resource::Sd, "sd", "SD card"),
    (Resource::Audio, "audio", "Audio jack"),
    (Resource::Sdram, "sdram", "SDRAM"),
    (Resource::Adc, "adc", "ADC (MAX11125)"),
    (Resource::Rtc, "rtc", "RTC (MCP7940N)"),
    (Resource::Flash, "flash", "SPI flash"),
    (Resource::Header, "header", "Header pins (inputs only)"),
];

const HEADER_NOTE: &str = "J1/J2 inputs only, never driven by default: safe with nothing connected";

fn resource_description(resource: Resource) -> Option<&'static str> {
    Some(match resource {
        Resource::Oled => "Adds OLED SPI and control ports, held reset and idle. See examples/ulx3s/oled_pattern.",
        Resource::Hdmi => "Adds GPDI video outputs, held low. See examples/ulx3s/hdmi_bars.",
        Resource::Sd => "Adds SD SPI ports with the card deselected. See examples/ulx3s/sd_read.",
        Resource::Audio => "Adds audio DAC outputs, held at zero. See examples/ulx3s/audio_tone.",
        Resource::Sdram => "Adds inactive SDRAM control ports and a released data bus. See examples/ulx3s/sdram_test.",
        Resource::Adc => "Adds ADC SPI ports and analog pad inputs, with the ADC deselected. See examples/ulx3s/adc_read.",
        Resource::Rtc => "Adds a released, open-drain RTC I2C bus. See examples/ulx3s/rtc_read.",
        Resource::Flash => "Adds deselected SPI flash ports and its USRMCLK clock. See examples/ulx3s/flash_dump.",
        Resource::Header => "Adds J1/J2 GP/GN inputs with GP pull-downs and GN pull-ups. See examples/ulx3s/header_probe.",
        _ => return None,
    })
}

/// A comment begins outside JSON strings, including strings with escaped quotes.
pub fn uncomment(line: &str) -> &str {
    let (mut quoted, mut escaped) = (false, false);
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c == '#' && !quoted {
            return &line[..i];
        }
    }
    line
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub name: Option<String>,
    pub dir: Option<String>,
    pub resources: Option<BTreeSet<Resource>>,
    pub demo: Option<String>,
}

impl Fields {
    fn parse(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let object = value.as_object().context("expected a JSON object")?;
        let mut fields = Self::default();
        for (key, value) in object {
            let string = || {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("expected a string")
            };
            match key.as_str() {
                "name" => fields.name = Some(string()?),
                "dir" => fields.dir = Some(string()?),
                "demo" => fields.demo = Some(string()?),
                "run" => bail!("project option 'run' was removed; Create only opens the project; use Build and Emulate afterwards"),
                "resources" => {
                    fields.resources = Some(
                        value
                            .as_array()
                            .context("resources must be an array")?
                            .iter()
                            .map(|v| {
                                Resource::parse(v.as_str().context("resource must be a string")?)
                            })
                            .collect::<Result<_>>()?,
                    );
                }
                _ => bail!("unknown project option {key:?}"),
            }
        }
        Ok(fields)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    New(Fields),
    Demo(Fields),
    Options(Fields),
    Create,
    Cancel,
}

impl Command {
    pub fn parse(text: &str) -> Result<Option<Self>> {
        let text = uncomment(text).trim();
        let (word, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        Ok(Some(match word {
            "new-project" => Self::New(Fields::parse(args.trim())?),
            "start-demo" => Self::Demo(Fields::parse(args.trim())?),
            "project-options" => Self::Options(Fields::parse(args.trim())?),
            "create-project" | "cancel-project" => {
                ensure!(args.trim().is_empty(), "{word} takes no arguments");
                if word == "create-project" {
                    Self::Create
                } else {
                    Self::Cancel
                }
            }
            _ => return Ok(None),
        }))
    }
}

pub fn atom(text: &str) -> String {
    let text = start::display_text(text);
    if text.is_empty()
        || text
            .chars()
            .any(|c| c.is_whitespace() || "\"\\#".contains(c))
    {
        serde_json::to_string(&text).unwrap()
    } else {
        text
    }
}

pub struct Dialog {
    pub name: String,
    pub folder: String,
    pub resources: BTreeSet<Resource>,
    pub demo: Option<String>,
    pub error: Option<String>,
    parent: PathBuf,
    folder_edited: bool,
    demos: Vec<(String, String)>,
    catalogue: demo_copy::Catalogue,
    dev_mode: bool,
    catalogue_error: Option<String>,
}

impl Dialog {
    pub fn new(parent: Option<PathBuf>, demo: bool, dev_mode: bool) -> Self {
        let paths = DisplayPaths::new(None);
        let parent = parent.unwrap_or_else(|| paths.expand("~/ecp5-projects"));
        let catalogue = demo_copy::catalogue();
        let demos: Vec<_> = catalogue
            .as_ref()
            .map(|c| {
                c.visible(true)
                    .map(|d| {
                        let description = demo_copy::examples_root()
                            .map(|root| start::demo_description(&root, &d.name))
                            .unwrap_or_default();
                        (d.name.clone(), description)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let name = if demo {
            catalogue
                .as_ref()
                .ok()
                .and_then(|c| c.visible(dev_mode).next())
                .and_then(|d| d.name.rsplit('/').next())
                .unwrap_or("my_project")
        } else {
            "my_project"
        }
        .to_string();
        let folder = paths.path(&parent.join(&name));
        Self {
            name,
            folder,
            resources: flow::new_project::Options::new("").resources,
            demo: demo.then(|| {
                catalogue
                    .as_ref()
                    .ok()
                    .and_then(|c| c.visible(dev_mode).next())
                    .map(|d| d.name.clone())
                    .unwrap_or_default()
            }),
            error: None,
            parent,
            folder_edited: false,
            demos,
            catalogue: catalogue.as_ref().cloned().unwrap_or_default(),
            dev_mode,
            catalogue_error: catalogue
                .err()
                .map(|e| start::display_text(&format!("{e:#}"))),
        }
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
        if !self.folder_edited {
            self.folder = DisplayPaths::new(None).path(&self.parent.join(&self.name));
        }
    }

    pub fn set_folder(&mut self, folder: String) {
        self.folder = folder;
        self.folder_edited = true;
    }

    pub fn dir(&self) -> PathBuf {
        DisplayPaths::new(None).expand(&self.folder)
    }

    pub fn apply(&mut self, fields: Fields) -> Result<()> {
        ensure!(
            self.demo.is_none() || fields.resources.is_none(),
            "demo copies do not accept resources"
        );
        ensure!(
            self.demo.is_some() || fields.demo.is_none(),
            "New project does not accept demo"
        );
        if let Some(name) = fields.name {
            self.set_name(name);
        }
        if let Some(dir) = fields.dir {
            self.set_folder(dir);
        }
        if let Some(resources) = fields.resources {
            self.resources = resources;
        }
        if let Some(demo) = fields.demo {
            self.demo = Some(demo);
        }
        self.error = None;
        Ok(())
    }

    pub fn resources_text(&self) -> String {
        RESOURCES
            .iter()
            .filter(|(r, _, _)| self.resources.contains(r))
            .map(|(_, id, _)| *id)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn required_reason(&self, resource: Resource) -> Option<&'static str> {
        if let Some(dependency) = DEPENDENCIES.iter().find(|dependency| {
            dependency.requires == resource && self.resources.contains(&dependency.resource)
        }) {
            return Some(dependency.reason);
        }
        let has_peripheral = RESOURCES[5..]
            .iter()
            .any(|(resource, _, _)| self.resources.contains(resource));
        match resource {
            Resource::Clock => Some("Clock is required by every recipe"),
            Resource::Leds if !has_peripheral && !self.resources.contains(&Resource::Uart) => {
                Some("At least one of LEDs or UART is required")
            }
            Resource::Uart if !has_peripheral && !self.resources.contains(&Resource::Leds) => {
                Some("At least one of LEDs or UART is required")
            }
            _ => None,
        }
    }

    pub fn select(&mut self, resource: Resource, selected: bool) {
        if selected {
            self.resources.insert(resource);
            for dependency in DEPENDENCIES.iter().filter(|d| d.resource == resource) {
                self.resources.insert(dependency.requires);
            }
            self.resources.insert(Resource::Clock);
        } else if self.required_reason(resource).is_none() {
            self.resources.remove(&resource);
        }
    }

    pub fn name_error(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            Some("Project name must not be blank")
        } else if self.name.chars().any(char::is_control) {
            Some("Project name must not contain control characters")
        } else {
            None
        }
    }

    pub fn folder_error(&self) -> Option<String> {
        let mut options = flow::new_project::Options::new(self.dir());
        options.name = "validation".into();
        options
            .validate()
            .err()
            .map(|e| start::display_text(&format!("{e:#}")))
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(error) = self.name_error() {
            bail!(error);
        }
        if let Some(error) = self.folder_error() {
            bail!(error);
        }
        if let Some(demo) = &self.demo {
            if let Some(error) = &self.catalogue_error {
                bail!("{error}");
            }
            ensure!(
                self.demos.iter().any(|(id, _)| id == demo),
                "Select an available demo"
            );
        } else {
            let mut options = flow::new_project::Options::new(self.dir());
            options.name = self.name.clone();
            options.resources = self.resources.clone();
            options.validate()?;
        }
        Ok(())
    }

    pub fn create(&self) -> Result<flow::Project> {
        self.validate()?;
        if let Some(demo) = &self.demo {
            let mut options = demo_copy::Options::new(self.dir(), demo);
            options.name = self.name.clone();
            demo_copy::create(&options)
        } else {
            let mut options = flow::new_project::Options::new(self.dir());
            options.name = self.name.clone();
            options.resources = self.resources.clone();
            flow::new_project::create(&options)
        }
    }

    pub fn action_name(&self) -> &'static str {
        if self.demo.is_some() {
            "copy-demo"
        } else {
            "new-project"
        }
    }

    fn resource_checkbox(&mut self, ui: &mut egui::Ui, resource: Resource, label: &str) {
        let required = self.required_reason(resource);
        let mut selected = self.resources.contains(&resource);
        ui.horizontal(|ui| {
            let mut response = ui.add_enabled(
                required.is_none() || !selected,
                egui::Checkbox::new(&mut selected, label),
            );
            if let Some(description) = resource_description(resource) {
                response = response.on_hover_text(description);
            }
            if response.changed() {
                self.select(resource, selected);
            }
            let reason = required.or_else(|| {
                DEPENDENCIES
                    .iter()
                    .find(|d| d.resource == resource && selected)
                    .map(|d| d.reason)
            });
            if let Some(reason) = reason {
                ui.small(reason);
            }
        });
    }

    pub fn draw(&mut self, ctx: &egui::Context, dev_mode: bool) -> Option<Command> {
        // Filter the choices without changing the demo the user chose to copy.
        self.dev_mode = dev_mode;
        let mut action = None;
        egui::Modal::new(egui::Id::new("create_project")).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.heading(if self.demo.is_some() {
                "Start from a demo…"
            } else {
                "New project…"
            });
            if self.demo.is_some() {
                ui.label("Make a personal copy. The original demo stays in the library.");
            }
            ui.label("Name");
            let mut name = self.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                self.set_name(name);
            }
            if let Some(error) = self.name_error() {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.label("Project folder");
            ui.horizontal(|ui| {
                let mut folder = self.folder.clone();
                if ui
                    .add_sized([430.0, 24.0], egui::TextEdit::singleline(&mut folder))
                    .changed()
                {
                    self.set_folder(folder);
                }
                if ui.button("Browse…").clicked() {
                    if let Some(parent) = rfd::FileDialog::new()
                        .set_directory(self.dir().parent().unwrap_or(&self.parent))
                        .pick_folder()
                    {
                        self.parent = parent;
                        self.set_folder(
                            DisplayPaths::new(None).path(&self.parent.join(&self.name)),
                        );
                    }
                }
            });
            if let Some(error) = self.folder_error() {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(demo) = &mut self.demo {
                if !self.dev_mode && self.catalogue.visible(false).next().is_none() {
                    ui.label(start::NO_SHOWCASE);
                }
                let selected = self.catalogue.demos.iter().find(|d| d.name == *demo);
                let selected_text = selected.map_or_else(String::new, |d| {
                    if d.showcase {
                        d.name.clone()
                    } else {
                        format!("{} · dev", d.name)
                    }
                });
                egui::ComboBox::from_label("Demo to copy")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for entry in self.catalogue.visible(self.dev_mode) {
                            ui.horizontal(|ui| {
                                ui.selectable_value(demo, entry.name.clone(), &entry.name);
                                if !entry.showcase {
                                    start::dev_tag(ui);
                                }
                            });
                        }
                    });
                if let Some((_, description)) = self.demos.iter().find(|(id, _)| id == demo) {
                    ui.label(description);
                }
                if let Some(error) = &self.catalogue_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            } else {
                ui.label("ULX3S — LFE5U-85F / CABGA381, 25 MHz");
                for &(resource, _, label) in &RESOURCES[..5] {
                    self.resource_checkbox(ui, resource, label);
                }
                ui.collapsing("Advanced", |ui| {
                    for &(resource, _, label) in &RESOURCES[5..] {
                        self.resource_checkbox(ui, resource, label);
                    }
                    ui.small(HEADER_NOTE);
                    ui.label("Explore the existing examples with Start from a demo…");
                });
            }
            let valid = self.validate();
            if self.name_error().is_none() && self.folder_error().is_none() {
                if let Err(error) = &valid {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        start::display_text(&format!("{error:#}")),
                    );
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(valid.is_ok(), egui::Button::new("Create"))
                    .clicked()
                    || (valid.is_ok() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    action = Some(Command::Create);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = Some(Command::Cancel);
                }
            });
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn frame(
        ctx: &egui::Context,
        dialog: &mut Dialog,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 900.0),
                )),
                events,
                time: Some(ctx.input(|i| i.time) + 0.1),
                ..Default::default()
            },
            |ui| {
                dialog.draw(ui.ctx(), false);
            },
        )
    }

    fn click(
        ctx: &egui::Context,
        dialog: &mut Dialog,
        output: &egui::FullOutput,
        label: &str,
    ) -> egui::FullOutput {
        let at = crate::test_temp::painted_text(output)
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("missing {label}"))
            .1
            .center();
        for pressed in [true, false] {
            frame(ctx, dialog, crate::test_temp::click(at, pressed));
        }
        frame(ctx, dialog, vec![]);
        frame(ctx, dialog, vec![])
    }

    #[test]
    fn advanced_checkboxes_show_help_and_dependencies_and_create_each_peripheral() {
        let root = crate::test_temp::unique_dir("new-project-advanced-");
        let cases: [(&str, &[&str]); 9] = [
            (
                "oled_pattern",
                &["oled_clk", "oled_mosi", "oled_dc", "oled_resn", "oled_csn"],
            ),
            ("hdmi_bars", &["gpdi_dp"]),
            ("sd_read", &["sd_clk", "sd_cmd", "sd_d"]),
            ("audio_tone", &["audio_l", "audio_r", "audio_v"]),
            (
                "sdram_test",
                &[
                    "sdram_clk",
                    "sdram_cke",
                    "sdram_csn",
                    "sdram_wen",
                    "sdram_rasn",
                    "sdram_casn",
                    "sdram_ba",
                    "sdram_dqm",
                    "sdram_a",
                    "sdram_d",
                ],
            ),
            (
                "adc_read",
                &["adc_csn", "adc_sclk", "adc_mosi", "adc_miso", "adc_ain"],
            ),
            ("rtc_read", &["rtc_scl", "rtc_sda"]),
            (
                "flash_dump",
                &[
                    "flash_csn",
                    "flash_mosi",
                    "flash_miso",
                    "flash_wpn",
                    "flash_holdn",
                ],
            ),
            ("header_probe", &["gp", "gn"]),
        ];
        for (&(resource, name, label), (example, ports)) in RESOURCES[5..].iter().zip(cases) {
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| {
                style.animation_time = 0.0;
                style.interaction.tooltip_delay = 0.0;
            });
            let mut dialog = Dialog::new(Some(root.clone()), false, false);
            dialog.set_name(name.into());
            frame(&ctx, &mut dialog, vec![]);
            let output = frame(&ctx, &mut dialog, vec![]);
            let text = crate::test_temp::painted_text(&output);
            assert_eq!(dialog.resources_text(), "clock,leds");
            for (_, _, label) in &RESOURCES[5..] {
                assert!(!text.iter().any(|(text, _)| text == label));
            }
            let output = click(&ctx, &mut dialog, &output, "Advanced");
            let text = crate::test_temp::painted_text(&output);
            assert!(text.iter().any(|(text, _)| text == HEADER_NOTE));
            assert!(!text.iter().any(|(text, _)| text.contains("Deferred:")));
            let at = text
                .iter()
                .find(|(text, _)| text == label)
                .unwrap()
                .1
                .center();
            frame(&ctx, &mut dialog, vec![egui::Event::PointerMoved(at)]);
            frame(&ctx, &mut dialog, vec![]);
            let output = frame(&ctx, &mut dialog, vec![]);
            let description = resource_description(resource).unwrap();
            assert!(description.contains(&format!("examples/ulx3s/{example}")));
            assert!(
                crate::test_temp::painted_text(&output)
                    .iter()
                    .any(|(text, _)| text == description),
                "{name} tooltip"
            );

            let output = click(&ctx, &mut dialog, &output, label);
            assert!(dialog.resources.contains(&resource), "{name} checkbox");
            let dependency = DEPENDENCIES
                .iter()
                .find(|d| d.resource == resource)
                .unwrap();
            assert!(crate::test_temp::painted_text(&output)
                .iter()
                .any(|(text, _)| text == dependency.reason));
            let output = click(&ctx, &mut dialog, &output, "Clock");
            assert!(dialog.resources.contains(&Resource::Clock));
            let output = click(&ctx, &mut dialog, &output, "LEDs");
            assert_eq!(dialog.resources, [Resource::Clock, resource].into());
            assert_eq!(dialog.resources_text(), format!("clock,{name}"));
            let output = click(&ctx, &mut dialog, &output, label);
            assert!(
                !dialog.resources.contains(&resource),
                "{name} can be unchecked"
            );
            click(&ctx, &mut dialog, &output, label);
            assert_eq!(dialog.resources, [Resource::Clock, resource].into());
            let project = dialog.create().unwrap();
            let manifest = fs::read_to_string(project.root.join("ecp5.toml")).unwrap();
            assert!(manifest.contains(&format!("name = \"{name}\"")));
            assert!(manifest.contains("verilog = [\"top.v\"]"));
            assert!(manifest.contains("constraints = \"top.lpf\""));
            let top = fs::read_to_string(project.root.join("top.v")).unwrap();
            let declarations = top.split(");").next().unwrap();
            let lpf = fs::read_to_string(project.root.join("top.lpf")).unwrap();
            let reference = fs::read_to_string(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../examples/ulx3s")
                    .join(example)
                    .join("top.lpf"),
            )
            .unwrap();
            for port in ports {
                assert!(declarations.contains(port), "{name}: {port}");
                let lines: Vec<_> = reference
                    .lines()
                    .map(str::trim)
                    .filter(|line| {
                        !line.starts_with('#')
                            && line
                                .split('"')
                                .nth(1)
                                .is_some_and(|p| p.split('[').next() == Some(port))
                    })
                    .collect();
                assert!(!lines.is_empty(), "{name}: {port}");
                for line in lines {
                    assert!(
                        lpf.lines().any(|generated| generated == line),
                        "{name}: {line}"
                    );
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_script_parsers_accept_and_record_all_resources_and_create_a_valid_project() {
        let root = crate::test_temp::unique_dir("new-project-advanced-script-");
        let names =
            "clock,leds,buttons,switches,uart,oled,hdmi,sd,audio,sdram,adc,rtc,flash,header";
        let resources: Vec<_> = names.split(',').collect();
        let json = serde_json::json!({"resources": resources}).to_string();
        let script = format!("0 new-project {json}\n0 project-options {json}\n");
        let path = root.join("window.script");
        fs::write(&path, &script).unwrap();
        let window = crate::trace::script(&path).unwrap();
        let project = crate::project::script::parse_script(&script).unwrap();
        assert_eq!(window.len(), 2);
        assert_eq!(project.len(), 2);
        let mut dialog = Dialog::new(Some(root.clone()), false, false);
        for ((_, text), project) in window.iter().zip(&project) {
            let command = Command::parse(text).unwrap().unwrap();
            assert_eq!(
                project.command,
                crate::project::script::ScriptCommand::Creation(command.clone())
            );
            let (Command::New(fields) | Command::Options(fields)) = command else {
                panic!("expected resource fields");
            };
            dialog.apply(fields).unwrap();
            assert_eq!(dialog.resources, Resource::ALL.into());
            assert_eq!(dialog.resources_text(), names);
        }
        let created = dialog.create().unwrap();
        let reopened = flow::Project::open(&created.root).unwrap();
        assert_eq!(reopened.name, dialog.name);
        assert_eq!(reopened.verilog, [PathBuf::from("top.v")]);
        assert_eq!(reopened.constraints, PathBuf::from("top.lpf"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checking_each_resource_adds_shared_dependencies_and_reports_missing_script_dependencies() {
        let root = crate::test_temp::unique_dir("new-project-dependencies-");
        let mut dialog = Dialog::new(Some(root.clone()), false, false);
        for dependency in DEPENDENCIES {
            dialog.resources = [dependency.resource].into();
            let error = dialog.validate().unwrap_err().to_string();
            assert!(error.contains(dependency.reason), "{error}");
            dialog.select(dependency.resource, true);
            assert!(dialog.resources.contains(&dependency.requires));
            assert_eq!(
                dialog.required_reason(dependency.requires),
                Some(dependency.reason)
            );
            dialog.select(dependency.requires, false);
            assert!(dialog.resources.contains(&dependency.requires));
            assert!(dialog.validate().is_ok());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn turning_dev_mode_off_keeps_the_selected_demo_tag_and_allows_creation() {
        let root = crate::test_temp::unique_dir("keep-selected-dev-demo-");
        let ctx = egui::Context::default();
        let mut dialog = Dialog::new(Some(root.clone()), true, true);
        let demo = "ulx3s/uart_hello";
        dialog
            .catalogue
            .demos
            .iter_mut()
            .find(|d| d.name == demo)
            .expect("bundled UART demo")
            .showcase = false;
        dialog.demo = Some(demo.into());
        dialog.set_name("kept_dev_demo".into());
        let destination = dialog.dir();

        for dev in [true, false] {
            // The modal's first frame measures its layout before painting it.
            let mut text = Vec::new();
            for _ in 0..2 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        dialog.draw(ui.ctx(), dev);
                    },
                );
                text = crate::test_temp::painted_text(&output);
            }
            assert!(!text.iter().any(|(s, _)| s.contains("Build and run")));
            assert_eq!(dialog.demo.as_deref(), Some(demo));
            assert_eq!(dialog.name, "kept_dev_demo");
            assert_eq!(dialog.dir(), destination);
            assert!(
                text.iter().any(|(s, _)| s == &format!("{demo} · dev")),
                "{text:?}"
            );
            assert!(dialog.validate().is_ok());
        }

        let project = dialog
            .create()
            .expect("flow accepts a selected dev demo with Dev mode off");
        assert_eq!(
            project.root.canonicalize().unwrap(),
            destination.canonicalize().unwrap()
        );
        assert!(destination.join("ecp5.toml").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn demo_picker_filters_tags_and_explains_an_empty_showcase() {
        use crate::test_temp;
        let root = test_temp::demo_fixture();
        for dev in [false, true] {
            let ctx = egui::Context::default();
            let mut dialog = Dialog::new(Some(root.clone()), true, dev);
            dialog.catalogue = demo_copy::catalogue_in(&root.join("examples")).unwrap();
            dialog.demos = dialog
                .catalogue
                .demos
                .iter()
                .map(|d| (d.name.clone(), "fixture".into()))
                .collect();
            dialog.catalogue_error = None;
            dialog.demo = Some("ulx3s/hero".into());
            let frame = |dialog: &mut Dialog, events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        dialog.draw(ui.ctx(), dev);
                    },
                )
            };
            frame(&mut dialog, vec![]);
            let out = frame(&mut dialog, vec![]);
            let at = test_temp::painted_text(&out)
                .iter()
                .find(|(s, _)| s == "ulx3s/hero")
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                frame(&mut dialog, test_temp::click(at, pressed));
            }
            let out = frame(&mut dialog, vec![]);
            let text = test_temp::painted_text(&out);
            assert!(text.iter().any(|(s, _)| s == "ulx3s/hero"));
            assert_eq!(
                text.iter().any(|(s, _)| s == "ulx3s/hardware"),
                dev,
                "{text:?}"
            );
            assert_eq!(text.iter().any(|(s, _)| s == "dev"), dev, "{text:?}");
        }
        let ctx = egui::Context::default();
        let mut dialog = Dialog::new(Some(root.clone()), true, false);
        dialog.catalogue = demo_copy::catalogue_in(&root.join("examples")).unwrap();
        for demo in &mut dialog.catalogue.demos {
            demo.showcase = false;
        }
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            dialog.draw(ui.ctx(), false);
        });
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            dialog.draw(ui.ctx(), false);
        });
        assert!(test_temp::painted_text(&out)
            .iter()
            .any(|(s, _)| s == start::NO_SHOWCASE));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn defaults_and_suggestions_follow_name_until_folder_is_edited() {
        let parent = crate::test_temp::unique_dir("new-project-parent-");
        let mut dialog = Dialog::new(Some(parent.clone()), false, false);
        assert_eq!(dialog.resources_text(), "clock,leds");
        assert_eq!(dialog.dir(), parent.join("my_project"));
        dialog.set_name("Counter #1".into());
        assert_eq!(dialog.dir(), parent.join("Counter #1"));
        dialog.set_folder(parent.join("independent").to_string_lossy().into_owned());
        dialog.set_name("Display name".into());
        assert_eq!(dialog.dir(), parent.join("independent"));
        assert!(dialog.validate().is_ok());
        assert!(!dialog.dir().exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn checklist_prevents_removing_dependencies_and_explains_them() {
        let parent = crate::test_temp::unique_dir("new-project-checklist-");
        let mut dialog = Dialog::new(Some(parent.clone()), false, false);
        dialog.select(Resource::Clock, false);
        assert!(dialog.resources.contains(&Resource::Clock));
        assert!(dialog
            .required_reason(Resource::Clock)
            .unwrap()
            .contains("required"));
        dialog.select(Resource::Leds, false);
        assert!(dialog.resources.contains(&Resource::Leds));
        dialog.select(Resource::Uart, true);
        dialog.select(Resource::Leds, false);
        assert!(!dialog.resources.contains(&Resource::Leds));
        assert!(dialog
            .required_reason(Resource::Uart)
            .unwrap()
            .contains("LEDs or UART"));
        for resource in [Resource::Buttons, Resource::Switches] {
            dialog.select(resource, true);
            assert!(dialog.resources.contains(&Resource::Leds));
            assert!(dialog
                .required_reason(Resource::Leds)
                .unwrap()
                .contains("require LEDs"));
            dialog.select(Resource::Leds, false);
            assert!(dialog.resources.contains(&Resource::Leds));
            dialog.select(resource, false);
        }
        dialog.resources = BTreeSet::from([Resource::Clock]);
        assert!(dialog
            .validate()
            .unwrap_err()
            .to_string()
            .contains("observable output"));
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn validation_rejects_bad_fields_and_occupied_destinations_without_writing() {
        let parent = crate::test_temp::unique_dir("new-project-validation-");
        let mut dialog = Dialog::new(Some(parent.clone()), false, false);
        dialog.set_name("  ".into());
        assert!(dialog.name_error().is_some());
        dialog.set_name("bad\0name".into());
        assert!(dialog.name_error().is_some());
        dialog.set_name("A valid name".into());
        for leaf in ["", ".", ".."] {
            dialog.set_folder(if leaf.is_empty() {
                "".into()
            } else {
                format!("{}/{leaf}", parent.display())
            });
            assert!(dialog.folder_error().is_some(), "{leaf}");
        }
        let occupied = parent.join("occupied");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("keep"), "original").unwrap();
        dialog.set_folder(occupied.to_string_lossy().into_owned());
        assert!(dialog.create().is_err());
        assert_eq!(
            fs::read_to_string(occupied.join("keep")).unwrap(),
            "original"
        );
        let file = parent.join("file");
        fs::write(&file, "original").unwrap();
        dialog.set_folder(file.to_string_lossy().into_owned());
        assert!(dialog.folder_error().unwrap().contains("not a directory"));
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn both_script_parsers_accept_all_creation_commands_and_json_hashes() {
        let root = crate::test_temp::unique_dir("creation-parser-");
        let text = r#"0 new-project {"dir":"a folder/#1","name":"A \"quote\" #1","resources":["clock","leds"]} # comment
0 project-options {"name":"Name #2"}
0 create-project
0 start-demo {"demo":"ulx3s/uart_hello","dir":"copy #1","name":"Copy #1"}
0 cancel-project
"#;
        let path = root.join("window.script");
        fs::write(&path, text).unwrap();
        let gui = crate::trace::script(&path).unwrap();
        let project = crate::project::script::parse_script(text).unwrap();
        assert_eq!(gui.len(), 5);
        assert_eq!(project.len(), 5);
        for ((_, gui), project) in gui.iter().zip(&project) {
            assert_eq!(
                project.command,
                crate::project::script::ScriptCommand::Creation(
                    Command::parse(gui).unwrap().unwrap()
                )
            );
        }
        assert!(gui[0].1.contains("#1"));
        assert!(!gui[0].1.contains("# comment"));
        for text in [
            "new-project []",
            "new-project {\"run\":3}",
            "project-options {\"resources\":[\"pins\"]}",
            "create-project extra",
            "cancel-project extra",
        ] {
            assert!(Command::parse(text).is_err(), "{text}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_script_parsers_reject_the_removed_run_option() {
        let root = crate::test_temp::unique_dir("removed-run-");
        let path = root.join("window.script");
        for command in ["new-project", "start-demo", "project-options"] {
            for value in ["true", "false"] {
                let text = format!("0 {command} {{\"run\":{value}}}\n");
                fs::write(&path, &text).unwrap();
                let window_error = crate::trace::script(&path).unwrap_err();
                let project_error = crate::project::script::parse_script(&text).unwrap_err();
                for error in [window_error, project_error] {
                    assert!(
                        format!("{error:#}").contains("'run' was removed"),
                        "{error:#}"
                    );
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn demo_fields_do_not_mix_copying_with_resource_generation() {
        let parent = crate::test_temp::unique_dir("new-project-demo-fields-");
        let mut copy = Dialog::new(Some(parent.clone()), true, false);
        assert!(copy
            .apply(Fields {
                resources: Some(BTreeSet::new()),
                ..Default::default()
            })
            .is_err());
        let mut new = Dialog::new(Some(parent.clone()), false, false);
        assert!(new
            .apply(Fields {
                demo: Some("ulx3s/uart_hello".into()),
                ..Default::default()
            })
            .is_err());
        fs::remove_dir_all(parent).unwrap();
    }
}
