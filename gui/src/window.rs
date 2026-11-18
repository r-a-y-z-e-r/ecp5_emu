//! One native window: move its App into a ProjectApp after publication.
use crate::{
    app::App,
    new_project::{atom, Command, Dialog},
    project::{
        script::{self, ProjectTrace, ScriptCommand, ScriptRunner, WaitCondition},
        ProjectApp,
    },
    start::{self, display_paths::DisplayPaths},
};
use anyhow::Result;
use eframe::egui;
use std::{
    path::Path,
    time::{Duration, Instant},
};

enum Surface {
    Plain(Box<App>),
    Project(Box<ProjectApp>),
}

pub struct Window {
    surface: Option<Surface>,
    dialog: Option<Dialog>,
    trace: Option<ProjectTrace>,
    script: Option<ScriptRunner>,
    summary: Option<String>,
}

impl Window {
    pub fn plain(app: App) -> Result<Self> {
        let trace = std::env::var_os("ECP5_PROJECT_TRACE")
            .map(|p| ProjectTrace::new(Path::new(&p)))
            .transpose()?;
        let script = std::env::var_os("ECP5_PROJECT_SCRIPT")
            .map(|p| script::parse_script_file(Path::new(&p)))
            .transpose()?
            .map(ScriptRunner::from_steps);
        Ok(Self {
            surface: Some(Surface::Plain(Box::new(app))),
            dialog: None,
            trace,
            script,
            summary: None,
        })
    }

    pub fn project(project: ProjectApp) -> Self {
        Self {
            surface: Some(Surface::Project(Box::new(project))),
            dialog: None,
            trace: None,
            script: None,
            summary: None,
        }
    }

    fn app(&mut self) -> &mut App {
        match self.surface.as_mut().unwrap() {
            Surface::Plain(app) => app,
            Surface::Project(project) => project.app(),
        }
    }

    fn note(&mut self, note: &str) {
        let note = start::display_text(note);
        match self.surface.as_mut().unwrap() {
            Surface::Plain(app) => {
                app.record_note(&note);
                if let Some(trace) = &mut self.trace {
                    trace.record(&note);
                }
            }
            Surface::Project(project) => project.creation_note(&note),
        }
    }

    fn complete(&mut self) {
        match self.surface.as_mut().unwrap() {
            Surface::Plain(app) => {
                app.creation_complete();
                if let Some(script) = &mut self.script {
                    if script.waiting() == Some(WaitCondition::Creation) {
                        script.complete_wait(Instant::now());
                    }
                }
            }
            Surface::Project(project) => project.complete_creation_command(),
        }
    }

    fn allowed(&self) -> bool {
        match self.surface.as_ref().unwrap() {
            Surface::Plain(_) => true,
            Surface::Project(project) => project.creation_allowed(),
        }
    }

    fn open_project(&mut self, dir: &Path, ctx: &egui::Context) {
        let surface = self.surface.take().unwrap();
        let mut project = match surface {
            Surface::Plain(mut app) => {
                app.reset_for_project(ctx);
                Box::new(ProjectApp::from_app(
                    *app,
                    dir.to_path_buf(),
                    None,
                    false,
                    DisplayPaths::new(None),
                    self.trace.take(),
                    self.script.take(),
                    ctx,
                ))
            }
            Surface::Project(project) => project,
        };
        project.adopt_created(dir, ctx);
        self.surface = Some(Surface::Project(project));
    }

    fn handle(&mut self, command: Command, ctx: &egui::Context) {
        let failed_action = match &command {
            Command::Options(_) => "project-options".to_string(),
            Command::Demo(_) => "start-demo".to_string(),
            Command::Create => self.dialog.as_ref().map_or_else(
                || "new-project".into(),
                |d| {
                    if let Some(demo) = &d.demo {
                        format!("copy-demo {} {}", atom(demo), atom(&d.name))
                    } else {
                        format!("new-project {}", atom(&d.name))
                    }
                },
            ),
            _ => "new-project".to_string(),
        };
        let result = self.apply(command, ctx);
        if let Err(error) = result {
            let reason = start::display_text(&format!("{error:#}"));
            self.note(&format!("{failed_action} failed {reason}"));
            if let Some(dialog) = &mut self.dialog {
                dialog.error = Some(reason);
            }
        }
        self.complete();
    }

    fn apply(&mut self, command: Command, ctx: &egui::Context) -> Result<()> {
        match command {
            Command::New(fields) => self.begin(false, fields)?,
            Command::Demo(fields) => self.begin(true, fields)?,
            Command::Options(fields) => {
                let dialog = self
                    .dialog
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("No project dialog is open"))?;
                dialog.apply(fields)?;
                let note = format!("project-options ok resources={}", dialog.resources_text());
                self.note(&note);
            }
            Command::Cancel => {
                let dialog = self
                    .dialog
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("No project dialog is open"))?;
                self.note(&format!("{} cancelled", dialog.action_name()));
            }
            Command::Create => {
                anyhow::ensure!(
                    self.allowed(),
                    "Wait for the current build or emulation to finish"
                );
                let dialog = self
                    .dialog
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No project dialog is open"))?;
                dialog.validate()?;
                if let Some(Surface::Project(project)) = &mut self.surface {
                    project.prepare_creation()?;
                }
                let dialog = self.dialog.as_ref().unwrap();
                let since = Instant::now();
                let project = dialog.create()?;
                let note = if let Some(demo) = &dialog.demo {
                    format!(
                        "copy-demo {} {} ok {:.3}",
                        atom(demo),
                        atom(&project.name),
                        since.elapsed().as_secs_f64()
                    )
                } else {
                    format!(
                        "new-project {} ok {:.3}",
                        atom(&project.name),
                        since.elapsed().as_secs_f64()
                    )
                };
                self.summary = dialog
                    .demo
                    .as_deref()
                    .and_then(ecp5_emu::flow::demo_copy::runtime_inputs)
                    .map(str::to_owned);
                self.note(&note);
                self.dialog = None;
                self.open_project(&project.root, ctx);
                self.app().creation_clock();
            }
        }
        Ok(())
    }

    fn begin(&mut self, demo: bool, fields: crate::new_project::Fields) -> Result<()> {
        anyhow::ensure!(
            self.allowed(),
            "Wait for the current build or emulation to finish"
        );
        let parent = self.app().project_parent();
        let mut dialog = Dialog::new(parent, demo, self.app().dev_mode());
        dialog.apply(fields)?;
        let note = if let Some(demo) = &dialog.demo {
            format!("start-demo dialog {} {}", atom(demo), atom(&dialog.name))
        } else {
            format!(
                "new-project dialog {} resources={}",
                atom(&dialog.name),
                dialog.resources_text()
            )
        };
        self.dialog = Some(dialog);
        self.note(&note);
        Ok(())
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(command) = self.app().creation_command.take() {
            self.handle(command, ctx);
        }
        if let Some(dir) = self.app().project_request.take() {
            if self.allowed() {
                let prepared = match self.surface.as_mut().unwrap() {
                    Surface::Project(p) => p.prepare_creation(),
                    Surface::Plain(_) => Ok(()),
                };
                if prepared.is_ok() {
                    self.open_project(&dir, ctx);
                }
            }
        }
        // Before the first project exists, a project script can still create it.
        if matches!(self.surface, Some(Surface::Plain(_))) && !self.app().creation_busy {
            let command = self.script.as_mut().map(|s| s.poll_next(Instant::now()));
            match command {
                Some(Ok(Some(ScriptCommand::Creation(command)))) => {
                    self.script
                        .as_mut()
                        .unwrap()
                        .set_waiting(WaitCondition::Creation);
                    self.handle(command, ctx);
                    ctx.request_repaint();
                }
                Some(Ok(Some(ScriptCommand::Quit))) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                }
                Some(Ok(Some(_))) => {
                    self.note("project-script failed create or open a project first")
                }
                Some(Err(seconds)) => {
                    ctx.request_repaint_after(Duration::from_secs_f64(seconds.min(0.25)))
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for Window {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        match self.surface.as_mut().unwrap() {
            Surface::Plain(app) => app.raw_input_hook(ctx, input),
            Surface::Project(project) => project.raw_input_hook(ctx, input),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::N,
        );
        if self.dialog.is_none()
            && self.allowed()
            && ui.input_mut(|i| i.consume_shortcut(&shortcut))
        {
            self.handle(Command::New(Default::default()), &ctx);
        }
        self.poll(&ctx);
        self.app().creation_available = self.allowed();
        match self.surface.as_mut().unwrap() {
            Surface::Plain(app) => app.ui(ui, frame),
            Surface::Project(project) => project.ui(ui, frame),
        }
        // Process exactly the command just dispatched, before any later script line.
        if let Some(command) = self.app().creation_command.take() {
            self.handle(command, &ctx);
            ctx.request_repaint();
        }
        let dev_mode = self.app().dev_mode();
        if let Some(command) = self.dialog.as_mut().and_then(|d| d.draw(&ctx, dev_mode)) {
            self.handle(command, &ctx);
        }
        if let Some(summary) = self.summary.clone() {
            egui::Window::new("Demo copy created")
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.label(summary);
                    if ui.button("OK").clicked() {
                        self.summary = None;
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_project::Fields;
    use std::{fs, path::PathBuf};

    fn setup(root: &Path) -> (Window, egui::Context) {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = App::start(&cc, None);
        app.isolated_preferences(root.join("recent.txt"));
        app.trace(&root.join("window.tsv")).unwrap();
        let window = Window {
            surface: Some(Surface::Plain(Box::new(app))),
            dialog: None,
            trace: Some(ProjectTrace::new(&root.join("project.tsv")).unwrap()),
            script: None,
            summary: None,
        };
        (window, ctx)
    }

    fn frame(
        window: &mut Window,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        use eframe::App as _;
        let mut frame = eframe::Frame::_new_kittest();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 950.0),
                )),
                events,
                ..Default::default()
            },
            |ui| window.ui(ui, &mut frame),
        )
    }

    fn fields(dir: PathBuf) -> Fields {
        Fields {
            dir: Some(dir.to_string_lossy().into_owned()),
            name: Some("Counter #1".into()),
            ..Default::default()
        }
    }

    fn assert_editor(window: &Window, dir: &Path) {
        let Some(Surface::Project(project)) = &window.surface else {
            panic!("same host must now hold a project");
        };
        assert_eq!(
            project.creation_editor_path(),
            Some(dir.join("top.v").as_path())
        );
        assert!(!project.creation_build_started());
        assert!(!dir.join("build").exists());
        for file in ["ecp5.toml", "top.v", "top.lpf"] {
            assert!(dir.join(file).is_file(), "{file}");
        }
        assert!(ecp5_emu::flow::Project::open(dir).is_ok());
    }

    #[test]
    fn gui_script_creates_in_place_keeps_trace_writer_and_remaining_commands() {
        let root = crate::test_temp::unique_dir("window-create-")
            .canonicalize()
            .unwrap();
        let dir = root.join("Counter #1");
        let (mut window, ctx) = setup(&root);
        window.app().rate(Some(4.0));
        window.note("before creation");
        let script = root.join("gui.script");
        fs::write(&script, format!("0 new-project {}\n0 project-options {{\"resources\":[\"clock\",\"leds\",\"buttons\"]}}\n0 create-project\n0 theme light\n",
            serde_json::json!({"dir":dir,"name":"Counter #1"}))).unwrap();
        window.app().script(&script).unwrap();
        for _ in 0..8 {
            frame(&mut window, &ctx, Vec::new());
        }
        assert_editor(&window, &dir);
        assert_eq!(window.app().preferred_rate, Some(Some(4.0)));
        window.note("after transition");
        let prefs = start::Preferences::read(Some(root.join("recent.txt"))).unwrap();
        assert_eq!(prefs.project_parent, Some(root.clone()));
        assert_eq!(prefs.recent[0], dir.canonicalize().unwrap());
        assert_eq!(
            Dialog::new(prefs.project_parent, false, prefs.dev_mode).dir(),
            root.join("my_project")
        );
        drop(window);
        let gui = fs::read_to_string(root.join("window.tsv")).unwrap();
        let project = fs::read_to_string(root.join("project.tsv")).unwrap();
        for payload in [
            "before creation",
            "new-project dialog \"Counter #1\" resources=clock,leds",
            "project-options ok resources=clock,leds,buttons",
            "new-project \"Counter #1\" ok ",
            "after transition",
        ] {
            assert!(gui.contains(&format!("# {payload}")), "{payload}: {gui}");
            assert!(
                project
                    .lines()
                    .any(|s| s.split_once('\t').unwrap().1.starts_with(payload)),
                "{payload}: {project}"
            );
        }
        assert_eq!(gui.matches("# new-project \"Counter #1\" ok ").count(), 1);
        assert!(gui.contains("theme light"), "pending GUI commands continue");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_script_can_create_from_start_screen_then_copy_a_demo() {
        let root = crate::test_temp::unique_dir("project-script-create-")
            .canonicalize()
            .unwrap();
        let dir = root.join("new");
        let copy = root.join("copy");
        let (mut window, ctx) = setup(&root);
        let text = format!("0 new-project {}\n0 create-project\n0 start-demo {}\n0 project-options {{\"name\":\"Copied UART\"}}\n0 create-project\n0 open top.v\n",
            serde_json::json!({"dir":dir,"name":"First"}),
            serde_json::json!({"dir":copy,"demo":"ulx3s/uart_hello"}));
        window.script = Some(ScriptRunner::from_steps(
            script::parse_script(&text).unwrap(),
        ));
        for _ in 0..10 {
            frame(&mut window, &ctx, Vec::new());
        }
        assert_editor(&window, &copy);
        assert!(dir.join("top.v").is_file());
        assert!(copy.join("THIRD_PARTY_NOTICES.txt").is_file());
        assert_eq!(
            ecp5_emu::flow::Project::open(&copy).unwrap().name,
            "Copied UART"
        );
        // The existing catalogue also contains an SV demo; select its actual top source.
        let sv_dir = root.join("sv-copy");
        let mut sv = fields(sv_dir.clone());
        sv.demo = Some("ulx3s/blinky_sv".into());
        window.handle(Command::Demo(sv), &ctx);
        window.handle(Command::Create, &ctx);
        let Some(Surface::Project(project)) = &window.surface else {
            unreachable!()
        };
        assert_eq!(
            project.creation_editor_path(),
            Some(sv_dir.join("top.sv").as_path())
        );
        drop(window);
        for file in ["window.tsv", "project.tsv"] {
            let trace = fs::read_to_string(root.join(file)).unwrap();
            for note in [
                "start-demo dialog ulx3s/uart_hello",
                "copy-demo ulx3s/uart_hello \"Copied UART\" ok ",
            ] {
                assert!(trace.contains(note), "{trace}");
            }
            assert_eq!(
                trace
                    .matches("copy-demo ulx3s/uart_hello \"Copied UART\" ok ")
                    .count(),
                1
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancel_and_validation_failures_leave_destination_and_preferences_untouched() {
        let root = crate::test_temp::unique_dir("window-cancel-")
            .canonicalize()
            .unwrap();
        let dir = root.join("absent/child");
        let (mut window, ctx) = setup(&root);
        window.handle(Command::New(fields(dir.clone())), &ctx);
        window.handle(Command::Cancel, &ctx);
        assert!(!dir.parent().unwrap().exists());
        assert!(!root.join("recent.txt").exists());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep"), "unchanged").unwrap();
        window.handle(Command::New(fields(dir.clone())), &ctx);
        window.handle(Command::Create, &ctx);
        assert!(window
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("not empty"));
        assert!(matches!(window.surface, Some(Surface::Plain(_))));
        assert_eq!(fs::read_to_string(dir.join("keep")).unwrap(), "unchanged");
        window.handle(Command::Cancel, &ctx);
        let mut demo = fields(root.join("copy"));
        demo.demo = Some("ulx3s/uart_hello".into());
        window.handle(Command::Demo(demo), &ctx);
        window.handle(Command::Cancel, &ctx);
        drop(window);
        for file in ["window.tsv", "project.tsv"] {
            let trace = fs::read_to_string(root.join(file)).unwrap();
            assert!(trace.contains("new-project cancelled"));
            assert!(trace.contains("copy-demo cancelled"));
            assert!(!trace.contains(" ok "));
        }
        assert!(!root.join("copy").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shortcut_draws_checklist_escape_cancels_and_enter_creates() {
        let root = crate::test_temp::unique_dir("window-shortcut-")
            .canonicalize()
            .unwrap();
        let (mut window, ctx) = setup(&root);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(
            &mut window,
            &ctx,
            vec![key(
                egui::Key::N,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            )],
        );
        assert!(window.dialog.is_some());
        frame(
            &mut window,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(window.dialog.is_none());
        assert!(!root.join("recent.txt").exists());
        let dir = root.join("keyboard-created");
        window.handle(Command::New(fields(dir.clone())), &ctx);
        frame(&mut window, &ctx, Vec::new());
        let drawn = frame(&mut window, &ctx, Vec::new());
        fn text(shape: &egui::Shape, labels: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(s) => labels.push(s.galley.text().into()),
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        text(s, labels);
                    }
                }
                _ => {}
            }
        }
        let mut labels = Vec::new();
        for shape in &drawn.shapes {
            text(&shape.shape, &mut labels);
        }
        for label in [
            "Name",
            "Project folder",
            "ULX3S — LFE5U-85F / CABGA381, 25 MHz",
            "Clock",
            "LEDs",
            "Buttons",
            "DIP switches",
            "UART",
        ] {
            assert!(labels.iter().any(|s| s == label), "{label}: {labels:?}");
        }
        assert!(!labels.iter().any(|s| s.contains("Build and run")));
        assert!(labels.iter().any(|s| s.contains("Clock is required")));
        assert!(labels.iter().any(|s| s.contains("LEDs or UART")));
        frame(
            &mut window,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_editor(&window, &dir);
        assert!(window.dialog.is_none());
        drop(window);
        fs::remove_dir_all(root).unwrap();
    }
}
