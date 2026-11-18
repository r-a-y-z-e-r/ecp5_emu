//! Opening designs and keeping the user's recent paths, theme and load times.
#[path = "start_build.rs"]
mod build;
#[path = "display_paths.rs"]
pub(crate) mod display_paths;
#[path = "reader.rs"]
pub(crate) mod reader;

use ecp5_emu::flow::demo_copy::Catalogue;
use eframe::egui::{self, RichText};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Demo {
    pub name: String,
    pub description: String,
    pub luts: Option<usize>,
    pub config: PathBuf,
    pub project: PathBuf,
    pub build: build::State,
}

pub const DEV_MODE_TOOLTIP: &str =
    "Show every example, including developer and hardware-test designs";
pub const NO_SHOWCASE: &str = "No showcase demos. Turn on Dev mode to see every example.";

/// Keep all build states while visibility changes through the shared catalogue.
#[derive(Default)]
pub struct Demos {
    catalogue: Catalogue,
    entries: Vec<Demo>,
}

impl std::ops::Deref for Demos {
    type Target = [Demo];
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for Demos {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

impl Demos {
    fn visible(&self, dev: bool) -> impl Iterator<Item = (&Demo, bool)> {
        self.catalogue.visible(dev).filter_map(|entry| {
            let name = entry.name.strip_prefix("ulx3s/").unwrap_or(&entry.name);
            self.entries
                .iter()
                .find(|demo| demo.name == name)
                .map(|demo| (demo, entry.showcase))
        })
    }

    #[cfg(test)]
    fn from_entries(entries: Vec<Demo>) -> Self {
        Self {
            catalogue: Catalogue {
                demos: entries
                    .iter()
                    .map(|demo| ecp5_emu::flow::demo_copy::Demo {
                        name: demo.name.clone(),
                        group: "ulx3s".into(),
                        showcase: true,
                    })
                    .collect(),
                ..Default::default()
            },
            entries,
        }
    }
}

pub(crate) fn dev_tag(ui: &mut egui::Ui) {
    ui.label(RichText::new("dev").small().weak());
}

pub fn display_text(text: &str) -> String {
    display_paths::DisplayPaths::new(Some(&repository_root())).text(text)
}

impl Demo {
    pub fn built_config(&self) -> Option<PathBuf> {
        [self.config.clone(), self.project.join("build/top.config")]
            .into_iter()
            .find(|p| p.is_file())
    }

    pub fn start_build(&mut self, ctx: &egui::Context) {
        self.build.start(self.project.clone(), ctx);
    }

    pub fn poll_build(&mut self) -> Option<(Option<Action>, String)> {
        let (result, seconds) = self.build.poll()?;
        let ok = result.is_ok();
        let action = result.ok().map(|config| {
            self.config = config.clone();
            Action::Load(config)
        });
        Some((
            action,
            format!(
                "start build {} {} {seconds:.2}",
                self.name,
                if ok { "ok" } else { "failed" }
            ),
        ))
    }

    pub fn build_command(&self) -> String {
        format!(
            "ecp5_emu build examples/{}",
            if self.name.contains('/') {
                self.name.clone()
            } else {
                format!("ulx3s/{}", self.name)
            }
        )
    }
}

pub fn repository_root() -> PathBuf {
    if let Some(root) = ecp5_emu::flow::demo_copy::examples_root()
        .and_then(|examples| examples.parent().map(Path::to_path_buf))
    {
        return root;
    }
    let executable = std::env::current_exe().ok();
    let current = std::env::current_dir().ok();
    for start in [
        executable.as_deref().and_then(Path::parent),
        current.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(root) = start
            .ancestors()
            .find(|p| p.join("examples/ulx3s").is_dir())
        {
            return root.to_path_buf();
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The README's inline code and link labels as plain text in the window. A
/// code span with no letter or digit (`*`, `<<`) keeps quotes: bare, it would
/// read as punctuation or markup. Brackets that are not a link stay.
fn plain(markdown: &str) -> String {
    let chars: Vec<char> = markdown.chars().collect();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '`' => {
                if let Some(length) = chars[i + 1..].iter().position(|&c| c == '`') {
                    let code: String = chars[i + 1..i + 1 + length].iter().collect();
                    if code.chars().any(char::is_alphanumeric) {
                        text.push_str(&code);
                    } else {
                        text.push_str(&format!("\"{code}\""));
                    }
                    i += length + 2;
                    continue;
                }
            }
            '[' => {
                if let Some((label, end)) = link(&chars, i) {
                    text.push_str(&plain(&label));
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        text.push(chars[i]);
        i += 1;
    }
    text
}

/// A link `[label](target)` opening at `chars[at]`: its label, and where the
/// text after it starts.
fn link(chars: &[char], at: usize) -> Option<(String, usize)> {
    let close = matching(chars, at, '[', ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = matching(chars, close + 1, '(', ')')?;
    Some((chars[at + 1..close].iter().collect(), end + 1))
}

/// Where the bracket opening at `chars[at]` closes.
fn matching(chars: &[char], at: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(at) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

pub(crate) fn demo_description(examples: &Path, id: &str) -> String {
    let (group, name) = id.split_once('/').unwrap_or(("ulx3s", id));
    let readme = fs::read_to_string(examples.join(group).join("README.md")).unwrap_or_default();
    readme
        .lines()
        .find_map(|line| {
            let mut columns = line.split('|').skip(1);
            let design = columns.next()?.trim();
            (design.starts_with(&format!("[{name}]({name}/")) || design == format!("`{name}`"))
                .then(|| columns.next().map(|s| plain(s.trim())))
                .flatten()
        })
        .unwrap_or_else(|| format!("{group} reference design"))
}

pub fn demos(root: &Path) -> io::Result<Demos> {
    let examples = root.join("examples");
    let catalogue = ecp5_emu::flow::demo_copy::catalogue_in(&examples)
        .map_err(|e| io::Error::other(display_text(&format!("{e:#}"))))?;
    let entries = catalogue
        .visible(true)
        .map(|demo| {
            let name = demo
                .name
                .strip_prefix("ulx3s/")
                .unwrap_or(&demo.name)
                .to_string();
            let description = demo_description(&examples, &demo.name);
            let luts = description.find("LUT4").and_then(|at| {
                let digits: String = description[..at]
                    .trim_end()
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_digit() || c.is_whitespace())
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                digits.chars().rev().collect::<String>().parse().ok()
            });
            Demo {
                config: root.join("target/reference").join(&name).join("top.config"),
                project: examples.join(&demo.name),
                name,
                description,
                luts,
                build: build::State::default(),
            }
        })
        .collect();
    Ok(Demos { catalogue, entries })
}

#[cfg(test)]
pub(crate) fn settings_path() -> Option<PathBuf> {
    Some(crate::test_temp::home_dir().join("config/ecp5_emu/recent.txt"))
}

#[cfg(not(test))]
pub(crate) fn settings_path() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let directory = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|p| p.join(".config")))?
    };
    Some(directory.join("ecp5_emu/recent.txt"))
}

/// Seconds spent in each stage, in the order the window saw them.
pub type StageTimes = Vec<(String, f64)>;

/// The last successful load of each design, newest first. JSON lines keep
/// paths and stage names with spaces, tabs or newlines unambiguous.
pub struct LoadTimes {
    designs: Vec<(PathBuf, StageTimes)>,
    path: Option<PathBuf>,
}

impl LoadTimes {
    pub fn load() -> Self {
        Self::read(settings_path().map(|p| p.with_file_name("load-times.txt")))
    }

    fn read(path: Option<PathBuf>) -> Self {
        let mut times = Self {
            designs: Vec::new(),
            path,
        };
        let text = times.path.as_ref().and_then(|p| fs::read_to_string(p).ok());
        for line in text.unwrap_or_default().lines() {
            if let Ok((path, stages)) = serde_json::from_str::<(PathBuf, StageTimes)>(line) {
                if !stages.is_empty()
                    && stages.iter().all(|(name, seconds)| {
                        !name.is_empty() && seconds.is_finite() && *seconds >= 0.0
                    })
                    && !times.designs.iter().any(|(p, _)| *p == path)
                {
                    times.designs.push((path, stages));
                    if times.designs.len() == 32 {
                        break;
                    }
                }
            }
        }
        times
    }

    pub fn get(&self, path: &Path) -> StageTimes {
        self.designs
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, stages)| stages.clone())
            .unwrap_or_default()
    }

    pub fn remember(&mut self, path: PathBuf, stages: StageTimes) {
        self.designs.retain(|(p, _)| *p != path);
        self.designs.insert(0, (path, stages));
        self.designs.truncate(32);
    }

    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut text = String::new();
        for design in &self.designs {
            text.push_str(&serde_json::to_string(design)?);
            text.push('\n');
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, text)?;
        fs::rename(temporary, path)
    }
}

pub struct Preferences {
    pub recent: Vec<PathBuf>,
    pub theme: egui::ThemePreference,
    pub dev_mode: bool,
    pub project_parent: Option<PathBuf>,
    path: Option<PathBuf>,
}

impl Preferences {
    pub fn load() -> io::Result<Self> {
        Self::read(settings_path())
    }

    pub(crate) fn read(path: Option<PathBuf>) -> io::Result<Self> {
        let mut prefs = Self {
            recent: Vec::new(),
            theme: egui::ThemePreference::System,
            dev_mode: false,
            project_parent: None,
            path,
        };
        let text = match prefs.path.as_ref().map(fs::read_to_string).transpose() {
            Ok(text) => text.unwrap_or_default(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        for line in text.lines() {
            match line.strip_prefix("theme=") {
                Some("light") => prefs.theme = egui::ThemePreference::Light,
                Some("dark") => prefs.theme = egui::ThemePreference::Dark,
                Some(_) => {}
                None => {
                    if let Some(dev) = line.strip_prefix("dev_mode=") {
                        prefs.dev_mode = dev == "true";
                    }
                    if let Some(parent) = line.strip_prefix("project_parent=") {
                        prefs.project_parent = serde_json::from_str::<PathBuf>(parent).ok();
                    }
                    if let Some(path) = line.strip_prefix("path=") {
                        if let Ok(path) = serde_json::from_str::<String>(path) {
                            let path = PathBuf::from(path);
                            if !prefs.recent.contains(&path) && prefs.recent.len() < 8 {
                                prefs.recent.push(path);
                            }
                        }
                    }
                }
            }
        }
        Ok(prefs)
    }

    pub fn defaults() -> Self {
        Self {
            recent: Vec::new(),
            theme: egui::ThemePreference::System,
            dev_mode: false,
            project_parent: None,
            path: settings_path(),
        }
    }

    pub fn add(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            }
        });
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(8);
    }

    pub fn save(&self) -> io::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| io::Error::other("could not find the user's configuration directory"))?;
        let theme = match self.theme {
            egui::ThemePreference::System => "system",
            egui::ThemePreference::Light => "light",
            egui::ThemePreference::Dark => "dark",
        };
        let mut text = format!("theme={theme}\ndev_mode={}\n", self.dev_mode);
        if let Some(parent) = &self.project_parent {
            text.push_str(&format!(
                "project_parent={}\n",
                serde_json::to_string(parent)?
            ));
        }
        for path in &self.recent {
            text.push_str(&format!(
                "path={}\n",
                serde_json::to_string(&path.to_string_lossy())?
            ));
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, text)?;
        fs::rename(temporary, path)
    }
}

#[derive(Clone)]
pub enum Action {
    NewFile,
    NewProject,
    StartFromDemo,
    Open,
    Load(PathBuf),
    Build(String),
    Close,
    Tour,
    DevMode(bool),
}

/// Inherit the environment, including architecture and record paths.
pub fn launch_tour(ctx: &egui::Context) -> io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg("--tour")
        .spawn()?;
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    Ok(())
}

pub fn recent_choices(ui: &mut egui::Ui, recent: &[PathBuf]) -> Option<Action> {
    let mut action = None;
    if recent.is_empty() {
        ui.label("No recent designs");
    }
    for path in recent {
        let exists = path.is_file() || path.is_dir();
        let label = display_paths::DisplayPaths::new(Some(&repository_root())).path(path);
        if ui
            .add_enabled(exists, egui::Button::new(&label).truncate())
            .on_hover_text(&label)
            .on_disabled_hover_text(format!("Project or file not found: {label}"))
            .clicked()
        {
            action = Some(Action::Load(path.clone()));
            ui.close();
        }
    }
    action
}

pub fn demo_choices(ui: &mut egui::Ui, demos: &Demos, detailed: bool, dev: bool) -> Option<Action> {
    let mut action = None;
    if !dev && demos.visible(false).next().is_none() {
        ui.label(NO_SHOWCASE);
    }
    for (demo, showcase) in demos.visible(dev) {
        ui.push_id(&demo.name, |ui| {
            let config = demo.built_config();
            let title = match demo.luts {
                Some(luts) => format!("{} · {luts} LUT4", demo.name),
                None => demo.name.clone(),
            };
            let (row, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), if detailed { 76.0 } else { 54.0 }),
                egui::Sense::hover(),
            );
            let controls = egui::Rect::from_min_size(row.min, egui::vec2(row.width(), 24.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(controls));
            child.horizontal(|ui| {
                let title_width =
                    (controls.width() - 72.0 - if showcase { 0.0 } else { 26.0 }).max(20.0);
                if ui
                    .add_enabled_ui(config.is_some(), |ui| {
                        ui.add_sized([title_width, 24.0], egui::Button::new(title).truncate())
                    })
                    .inner
                    .on_hover_text(display_text(&demo.description))
                    .clicked()
                {
                    action = config.clone().map(Action::Load);
                    ui.close();
                }
                if !showcase {
                    dev_tag(ui);
                }
                if config.is_none()
                    && ui
                        .add_enabled(
                            !demo.build.running(),
                            egui::Button::new("Build").min_size(egui::vec2(64.0, 24.0)),
                        )
                        .on_hover_text(demo.build_command())
                        .clicked()
                {
                    action = Some(Action::Build(demo.name.clone()));
                }
            });
            let status = display_text(&demo.build.caption());
            let colour = if demo.build.failed() {
                crate::theme::colours(ui.visuals().dark_mode).error
            } else {
                ui.visuals().text_color()
            };
            ui.put(
                egui::Rect::from_min_size(
                    row.min + egui::vec2(0.0, 28.0),
                    egui::vec2(row.width(), 20.0),
                ),
                egui::Label::new(RichText::new(&status).small().color(colour)).truncate(),
            )
            .on_hover_text(&status);
            if detailed {
                let description = display_text(&demo.description);
                ui.put(
                    egui::Rect::from_min_size(
                        row.min + egui::vec2(0.0, 50.0),
                        egui::vec2(row.width(), 20.0),
                    ),
                    egui::Label::new(RichText::new(&description).small()).truncate(),
                )
                .on_hover_text(description);
            }
        });
    }
    action
}

pub fn screen(
    ui: &mut egui::Ui,
    prefs: &Preferences,
    demos: &Demos,
    error: Option<&str>,
) -> Option<Action> {
    let mut action = None;
    let width = ui.available_width().min(960.0);
    let height = ui.available_height().min(690.0);
    let rect = egui::Rect::from_center_size(ui.max_rect().center(), egui::vec2(width, height));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("ECP5 emulator");
                ui.label("Open a design to explore the LFE5U-85F and run it on the ULX3S.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized([180.0, 36.0], egui::Button::new("New project…"))
                        .clicked()
                    {
                        action = Some(Action::NewProject);
                    }
                    if ui
                        .add_sized([200.0, 36.0], egui::Button::new("Start from a demo…"))
                        .on_hover_text("Make a personal copy of a demo")
                        .clicked()
                    {
                        action = Some(Action::StartFromDemo);
                    }
                });
                ui.label("Create your own project or make a copy of a demo.");
                if ui
                    .button("Open…")
                    .on_hover_text("Open a Trellis configuration (.config) or bitstream (.bit)")
                    .clicked()
                {
                    action = Some(Action::Open);
                }
                ui.add_space(12.0);
                if ui.button("Take the tour").clicked() {
                    action = Some(Action::Tour);
                }
                if let Some(error) = error {
                    ui.colored_label(
                        crate::theme::colours(ui.visuals().dark_mode).error,
                        display_text(error),
                    );
                }
                ui.columns(2, |columns| {
                    columns[0].heading("Recent designs");
                    if let Some(a) = recent_choices(&mut columns[0], &prefs.recent) {
                        action = Some(a);
                    }
                    columns[1].horizontal(|ui| {
                        ui.heading("Demo designs");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut dev = prefs.dev_mode;
                            if ui
                                .checkbox(&mut dev, RichText::new("Dev mode").small().weak())
                                .on_hover_text(DEV_MODE_TOOLTIP)
                                .changed()
                            {
                                action = Some(Action::DevMode(dev));
                            }
                        });
                    });
                    columns[1].label("Choose a demo, or build it here.");
                    if let Some(a) = demo_choices(&mut columns[1], demos, true, prefs.dev_mode) {
                        action = Some(a);
                    }
                });
            });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> PathBuf {
        crate::test_temp::unique_dir("ecp5-gui-")
    }

    #[test]
    fn dev_preference_round_trips_and_old_preferences_default_to_off() {
        let root = temporary();
        let path = root.join("recent.txt");
        fs::write(&path, "theme=dark\npath=\"/old/hardware\"\n").unwrap();
        let mut prefs = Preferences::read(Some(path.clone())).unwrap();
        assert!(!prefs.dev_mode);
        assert!(!Preferences::defaults().dev_mode);
        let recent = prefs.recent.clone();
        for dev in [true, false] {
            prefs.dev_mode = dev;
            prefs.save().unwrap();
            let restored = Preferences::read(Some(path.clone())).unwrap();
            assert_eq!(restored.dev_mode, dev);
            assert_eq!(restored.recent, recent);
            assert_eq!(restored.theme, egui::ThemePreference::Dark);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gallery_and_menu_filter_tag_and_explain_empty_showcase_without_filtering_recent() {
        let root = crate::test_temp::demo_fixture();
        let mut demos = demos(&root).unwrap();
        let mut prefs = Preferences::read(None).unwrap();
        prefs.add(&root.join("examples/ulx3s/hardware"));
        let ctx = egui::Context::default();
        for dev in [false, true, false] {
            prefs.dev_mode = dev;
            for detailed in [true, false] {
                let mut text = Vec::new();
                for _ in 0..3 {
                    let out = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1100.0, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            if detailed {
                                screen(ui, &prefs, &demos, None);
                            } else {
                                demo_choices(ui, &demos, false, dev);
                            }
                        },
                    );
                    text = crate::test_temp::painted_text(&out);
                }
                assert!(text.iter().any(|(s, _)| s == "hero"), "{text:?}");
                assert_eq!(text.iter().any(|(s, _)| s == "hardware"), dev, "{text:?}");
                assert_eq!(text.iter().any(|(s, _)| s == "dev"), dev, "{text:?}");
                if detailed {
                    assert!(
                        text.iter().any(|(s, _)| s.ends_with("/ulx3s/hardware")),
                        "Recent: {text:?}"
                    );
                }
            }
        }
        for demo in &mut demos.catalogue.demos {
            demo.showcase = false;
        }
        for detailed in [true, false] {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                demo_choices(ui, &demos, detailed, false);
            });
            assert!(crate::test_temp::painted_text(&out)
                .iter()
                .any(|(s, _)| s == NO_SHOWCASE));
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Every character of every demo's description is in the window's
    /// font (a missing one is drawn as a box).
    #[test]
    fn demo_descriptions_have_every_glyph() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, egui::ThemePreference::Dark);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let font = egui::TextStyle::Small.resolve(&ctx.global_style());
        let missing: Vec<(String, char)> = demos(&repository_root())
            .unwrap()
            .entries
            .into_iter()
            .flat_map(|d| {
                let name = d.name.clone();
                d.description
                    .chars()
                    .map(move |c| (name.clone(), c))
                    .collect::<Vec<_>>()
            })
            .filter(|&(_, c)| !crate::theme::draws(&ctx, &font, c))
            .collect();
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    #[test]
    fn demos_follow_directories_and_readme_table() {
        let root = temporary();
        let examples = root.join("examples/ulx3s");
        for name in ["life", "blinky", "unlisted", "not_a_design"] {
            fs::create_dir_all(examples.join(name)).unwrap();
        }
        for name in ["life", "blinky", "unlisted"] {
            fs::write(examples.join(name).join("top.v"), "").unwrap();
        }
        fs::write(examples.join("README.md"), "| Design | What it shows | Board |\n| [life](life/top.v) | A [grid](life/top.v) with about 10 850 LUT4 | — |\n| [blinky](blinky/top.v) | an 8-bit `counter`: `*` on `led[0]`, led[7] lit | — |\n").unwrap();
        fs::create_dir_all(root.join("target/reference/blinky")).unwrap();
        fs::write(root.join("target/reference/blinky/top.config"), "").unwrap();
        let found = demos(&root).unwrap();
        assert_eq!(
            found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["blinky", "life", "unlisted"]
        );
        assert_eq!(
            found[0].description,
            "an 8-bit counter: \"*\" on led[0], led[7] lit"
        );
        assert!(found[0].config.is_file());
        assert_eq!(found[1].luts, Some(10_850));
        assert_eq!(found[1].description, "A grid with about 10 850 LUT4");
        assert!(!found[1].config.is_file());
        assert_eq!(
            found[1].build_command(),
            "ecp5_emu build examples/ulx3s/life"
        );
        fs::create_dir_all(found[1].project.join("build")).unwrap();
        fs::write(found[1].project.join("build/top.config"), "").unwrap();
        assert_eq!(
            found[1].built_config(),
            Some(found[1].project.join("build/top.config"))
        );
        assert_eq!(found[0].built_config(), Some(found[0].config.clone()));
        fs::remove_dir_all(root).unwrap();
    }

    /// Code spans and links as the window shows them.
    #[test]
    fn plain_keeps_what_markup_would_lose() {
        for (markdown, shown) in [
            ("`*` in the DSP blocks", "\"*\" in the DSP blocks"),
            ("`<<` and `+`", "\"<<\" and \"+\""),
            ("`led[0]` toggles", "led[0] toggles"),
            ("led[7] lights", "led[7] lights"),
            ("an `ODDRX1F` per lane", "an ODDRX1F per lane"),
            ("[HR02 (`mac`)](a/(b).md): ok", "HR02 (mac): ok"),
            (
                "frames ([PERIPHERALS.md](../x.md))",
                "frames (PERIPHERALS.md)",
            ),
            ("a lone ` stays", "a lone ` stays"),
            ("[not a link] and ] alone", "[not a link] and ] alone"),
        ] {
            assert_eq!(plain(markdown), shown, "{markdown}");
        }
    }

    #[test]
    fn repository_demos_have_the_readme_descriptions() {
        let found = demos(&repository_root()).unwrap();
        assert!(found.len() >= 11);
        assert!(found
            .iter()
            .all(|d| d.description != "ULX3S reference design"));
        assert_eq!(
            found.iter().find(|d| d.name == "life_full").unwrap().luts,
            Some(78_700)
        );
        let description = |name: &str| &found.iter().find(|d| d.name == name).unwrap().description;
        assert!(description("mac").starts_with("\"*\" in the DSP blocks"));
        assert!(description("blinky_cycle").contains("led[0] toggles"));
    }

    fn painted(shape: &egui::Shape, text: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(shape) => text.push((
                shape.galley.text().into(),
                egui::Rect::from_min_size(shape.pos, shape.galley.size()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    painted(shape, text);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn demo_rows_keep_their_space_and_hide_home_paths_in_both_themes_and_widths() {
        let home = crate::test_temp::home_dir();
        let root = temporary();
        let make_demo = |name: &str| Demo {
            name: name.into(),
            description: format!("Source {}", home.join("private/top.v").display()),
            luts: Some(8),
            config: root.join(name).join("top.config"),
            project: root.join("examples/ulx3s").join(name),
            build: build::State::default(),
        };
        let prefs = Preferences {
            recent: vec![home.join("private/top.config")],
            theme: egui::ThemePreference::Dark,
            dev_mode: false,
            project_parent: None,
            path: None,
        };
        for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            for width in [1500.0, 600.0] {
                let ctx = egui::Context::default();
                crate::theme::install(&ctx, theme);
                let mut positions = None;
                for state in 0..4 {
                    let mut demos =
                        Demos::from_entries(vec![make_demo("alpha"), make_demo("beta")]);
                    demos[0].build = match state {
                        1 => build::State::preview("Place and route", true, false),
                        2 => build::State::preview(
                            &format!("error: {}", home.join("private/top.v").display()),
                            false,
                            true,
                        ),
                        3 => {
                            fs::create_dir_all(demos[0].config.parent().unwrap()).unwrap();
                            fs::write(&demos[0].config, "").unwrap();
                            build::State::preview("Built · 3.0 s", false, false)
                        }
                        _ => build::State::default(),
                    };
                    let mut text = Vec::new();
                    for _ in 0..3 {
                        let output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 898.0),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                egui::CentralPanel::default().show(ui, |ui| {
                                    screen(ui, &prefs, &demos, None);
                                });
                            },
                        );
                        text.clear();
                        for shape in &output.shapes {
                            painted(&shape.shape, &mut text);
                        }
                    }
                    for (shown, _) in &text {
                        assert!(
                            !shown.contains(home.to_str().unwrap()),
                            "home path painted: {shown}"
                        );
                    }
                    let stable: Vec<_> = text
                        .iter()
                        .filter(|(s, _)| {
                            s.contains("alpha")
                                || s.contains("beta")
                                || s.starts_with("Source ")
                                || s == "Recent designs"
                        })
                        .cloned()
                        .collect();
                    assert!(stable.len() >= 5, "the demo rows are drawn: {text:?}");
                    if let Some(before) = &positions {
                        assert_eq!(
                            &stable, before,
                            "rows moved in state {state} at width {width}"
                        );
                    } else {
                        positions = Some(stable);
                    }
                }
                // The same screen paints a reader failure, including tool diagnostics.
                let error = reader::script_error(&format!("usage: build_image_reader.py [-h]\nbuild_image_reader.py: error: prjtrellis source not found (invalid path: {})\n", home.join("FPGA/prjtrellis").display()));
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 898.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        screen(
                            ui,
                            &prefs,
                            &Demos::from_entries(vec![make_demo("reader")]),
                            Some(&error),
                        );
                    },
                );
                let mut text = Vec::new();
                for shape in &output.shapes {
                    painted(&shape.shape, &mut text);
                }
                assert!(text
                    .iter()
                    .any(|(s, _)| s.contains("prjtrellis source not found")));
                assert!(text
                    .iter()
                    .any(|(s, _)| s.contains("See README Requirements.")));
                assert!(
                    text.iter()
                        .all(|(s, _)| !s.contains(home.to_str().unwrap())),
                    "{text:?}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_times_round_trip_deduplicate_and_keep_the_last_32_designs() {
        let root = temporary();
        let path = root.join("load-times.txt");
        let mut times = LoadTimes::read(Some(path.clone()));
        assert!(times.designs.is_empty());
        let stages = vec![
            ("Reading the configuration".into(), 6.25),
            ("Choosing fast mode's settings".into(), 7.9),
        ];
        let design = |n| root.join(format!("design {n}\t\n.config"));
        for n in 0..35 {
            times.remember(design(n), stages.clone());
        }
        times.remember(design(10), vec![("Reading the configuration".into(), 4.5)]);
        times.save().unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 32);
        let restored = LoadTimes::read(Some(path));
        assert_eq!(restored.designs, times.designs);
        assert_eq!(restored.designs[0].0, design(10));
        assert_eq!(restored.get(&design(34)), stages);
        assert!(restored.get(&design(0)).is_empty());
        assert_eq!(restored.get(&design(10))[0].1, 4.5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_unreadable_and_invalid_load_times_have_no_estimate() {
        let root = temporary();
        assert!(LoadTimes::read(Some(root.join("missing")))
            .designs
            .is_empty());
        assert!(LoadTimes::read(Some(root.clone())).designs.is_empty());
        let path = root.join("load-times.txt");
        fs::write(&path, "not json\n[\"x\",[[\"stage\",-2.0]]]\n[\"y\",[]]\n").unwrap();
        assert!(LoadTimes::read(Some(path.clone())).designs.is_empty());
        let text: String = (0..40)
            .map(|n| format!("[\"{n}\",[[\"stage\",1.0]]]\n"))
            .collect();
        fs::write(&path, text).unwrap();
        assert_eq!(LoadTimes::read(Some(path)).designs.len(), 32);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_add_front_deduplicate_cap_and_round_trip() {
        let root = temporary();
        let path = root.join("recent.txt");
        let mut prefs = Preferences::read(Some(path.clone())).unwrap();
        assert_eq!(prefs.theme, egui::ThemePreference::System);
        for n in 0..10 {
            prefs.add(&root.join(format!("design {n}.config")));
        }
        assert_eq!(prefs.recent.len(), 8);
        assert_eq!(prefs.recent[0], root.join("design 9.config"));
        prefs.add(&root.join("design 5.config"));
        prefs.add(&root.join("design 5.config"));
        assert_eq!(prefs.recent.len(), 8);
        assert_eq!(prefs.recent[0], root.join("design 5.config"));
        prefs.theme = egui::ThemePreference::Light;
        prefs.save().unwrap();
        let restored = Preferences::read(Some(path)).unwrap();
        assert_eq!(restored.recent, prefs.recent);
        assert_eq!(restored.theme, prefs.theme);
        assert!(restored.recent.iter().all(|p| !p.exists()));
        fs::remove_dir_all(root).unwrap();
    }
}
