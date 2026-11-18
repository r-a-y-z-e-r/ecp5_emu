use super::*;
use crate::start::reader;
use eframe::App as _;

fn ready_reader() -> reader::Check {
    reader::fixed(reader::Status::Ready)
}

struct Temporary(PathBuf);

impl Temporary {
    fn new() -> Self {
        Self(
            crate::test_temp::unique_dir("ecp5_project_privacy_")
                .canonicalize()
                .unwrap(),
        )
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("ecp5.toml"), "").unwrap();
    std::fs::write(
        root.join("top.v"),
        include_str!("../../../examples/ulx3s/blinky_cycle/top.v"),
    )
    .unwrap();
    std::fs::write(root.join("top.lpf"), "").unwrap();
}

#[test]
fn display_paths_follow_precedence_and_expand_home_without_changing_files() {
    let paths = DisplayPaths::with_environment(
        Some("/home/visitor/work".into()),
        Some("/home/visitor".into()),
        Some(Path::new("/opt/project")),
    );
    for (input, expected) in [
        ("/home/visitor/work/demo/top.v", "demo/top.v"),
        ("/home/visitor/work", "."),
        ("/home/visitor/Library/tour/top.v", "~/Library/tour/top.v"),
        ("/home/visitor", "~"),
        ("/opt/project/rtl/top.v", "rtl/top.v"),
        ("/opt/project", "."),
        ("rtl/top.v", "rtl/top.v"),
        ("/opt/other/top.v", "/opt/other/top.v"),
        ("/home/visitor2/top.v", "/home/visitor2/top.v"),
    ] {
        assert_eq!(paths.path(Path::new(input)), expected);
    }
    assert_eq!(
        paths.expand(" ~/Library/tour "),
        Path::new("/home/visitor/Library/tour")
    );
    assert_eq!(paths.expand("~"), Path::new("/home/visitor"));
    assert_eq!(paths.expand("~someone/top.v"), Path::new("~someone/top.v"));
    assert_eq!(
        paths.text(
            "reading '/home/visitor/work/project with spaces/top.v': /home/visitor/Library/tour"
        ),
        "reading 'project with spaces/top.v': ~/Library/tour"
    );
}

fn collect_text(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(shape) => text.push(shape.galley.text().to_owned()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, text);
            }
        }
        _ => {}
    }
}

fn render(window: &mut ProjectApp, ctx: &egui::Context, home: &Path) -> Vec<String> {
    let mut text = Vec::new();
    let mut frame = eframe::Frame::_new_kittest();
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1800.0, 1200.0),
                )),
                ..Default::default()
            },
            |ui| window.ui(ui, &mut frame),
        );
        for shape in &output.shapes {
            collect_text(&shape.shape, &mut text);
        }
        for viewport in output.viewport_output.values() {
            for command in &viewport.commands {
                if let egui::ViewportCommand::Title(title) = command {
                    assert!(
                        !title.contains(home.to_str().unwrap()),
                        "title leaks home: {title}"
                    );
                }
            }
        }
    }
    assert!(!text.is_empty());
    for text in &text {
        assert!(
            !text.contains(home.to_str().unwrap()),
            "painted text leaks home: {text}"
        );
    }
    text
}

#[test]
fn project_window_painted_text_hides_home_including_canonical_tour_roots() {
    let temp = Temporary::new();
    let home = temp.0.join("private home");
    let ordinary = home.join("work/project");
    let bundle = home.join("Applications/Emulator.app/Contents/Resources/tour/blinky_cycle");
    fixture(&ordinary);
    fixture(&bundle);
    let located = tour::TourProject::locate(
        Some(&bundle.join("../blinky_cycle")),
        None,
        &temp.0.join("ecp5_gui"),
        &temp.0,
        &home.join("Library/Application Support/ecp5_emu"),
    )
    .unwrap();
    assert_eq!(located.root, bundle.canonicalize().unwrap());
    assert!(located.root.is_absolute());

    let mut projects = vec![(ordinary, None), (located.root.clone(), Some(located))];
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let source = home.join("read-only bundle");
        fixture(&source);
        std::fs::create_dir_all(source.join("prebuilt")).unwrap();
        for name in ["top.config", "top_routed.json"] {
            std::fs::write(source.join("prebuilt").join(name), "prebuilt").unwrap();
        }
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o555)).unwrap();
        let config = home.join("Library/Application Support/ecp5_emu");
        let copied = tour::TourProject::locate(
            Some(&source),
            None,
            &temp.0.join("ecp5_gui"),
            &temp.0,
            &config,
        );
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        let copied = copied.unwrap();
        assert_eq!(
            copied.root,
            config.join("tour/blinky_cycle").canonicalize().unwrap()
        );
        projects.push((copied.root.clone(), Some(copied)));
    }

    for (root, touring) in projects {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let paths = DisplayPaths::with_environment(
            Some(temp.0.join("elsewhere")),
            Some(home.clone()),
            None,
        );
        let mut window =
            ProjectApp::new_with_paths(&cc, root.clone(), None, false, false, paths).unwrap();
        window.app().reader_check = ready_reader;
        assert!(window.path_input.starts_with("~/"));
        assert_eq!(window.project.as_ref().unwrap().root, root);
        // Reopening the displayed ~/ path must still resolve the real project.
        window.open_project(Path::new(&window.path_input.clone()), &ctx);
        assert_eq!(window.project.as_ref().unwrap().root, root);
        window.status = format!("status: reading {}", root.join("top.v").display());
        window.messages.push(Message {
            kind: Default::default(),
            severity: Severity::Error,
            step: Step::Synthesis,
            text: format!("diagnostic: {}", home.join("shared/include.v").display()),
            file: Some(root.join("top.v")),
            line: Some(2),
            column: None,
            count: 1,
        });
        window.log.push(
            LogSource::Step(Step::Synthesis),
            format!("tool-log: {}", root.join("top.v").display()),
        );
        window.last = Some(Outcome {
            ok: false,
            cancelled: false,
            steps: Vec::new(),
            messages: Vec::new(),
            report: Some(flow::Report {
                max_delays: vec![format!(
                    "report-delay: {}",
                    home.join("shared/top.v").display()
                )],
                ..Default::default()
            }),
            outputs: window.project.as_ref().unwrap().outputs(),
        });
        let log = root.join("build.log");
        std::fs::write(
            &log,
            format!("editor-log: {}", root.join("top.v").display()),
        )
        .unwrap();
        assert!(window.open_in_editor(&log, log.to_string_lossy().into_owned(), true, None));
        if let Some(project) = &touring {
            let mut session = tour::Session::with_reader(project.clone(), ready_reader());
            session.machine.step = tour::Step::Source;
            session
                .machine
                .fail(0.0, &format!("caption: {}", root.display()));
            session.freeze_at(0.5);
            window.tour = Some(session);
        }
        for (tab, marker) in [
            (OutputTab::Messages, "diagnostic:"),
            (OutputTab::Log, "tool-log:"),
            (OutputTab::Report, "report-delay:"),
        ] {
            window.output.tab = tab;
            let text = render(&mut window, &ctx, &home);
            for marker in [marker, "Open project", "top.v", "status:", "editor-log:"] {
                assert!(
                    text.iter().any(|t| t.contains(marker)),
                    "missing {marker}: {text:?}"
                );
            }
            if touring.is_some() {
                assert!(text.iter().any(|t| t.contains("caption:")));
            }
        }
        assert!(!window.open_in_editor(&root.join("missing.v"), "missing.v".into(), false, None));
        assert!(render(&mut window, &ctx, &home)
            .iter()
            .any(|t| t.contains("Open failed:")));
        window.editor.clear();
        window.project = Err(format!("project-error: {}", root.display()));
        assert!(render(&mut window, &ctx, &home)
            .iter()
            .any(|t| t.contains("project-error:")));
    }
}

#[test]
fn failed_tour_keeps_its_waiting_caption_and_stop_button_without_a_deadline() {
    let temp = Temporary::new();
    let home = temp.0.join("private home");
    let root = home.join("project");
    fixture(&root);
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let paths = DisplayPaths::with_environment(None, Some(home.clone()), None);
    let mut window =
        ProjectApp::new_with_paths(&cc, root.clone(), None, false, false, paths).unwrap();
    window.app().reader_check = ready_reader;
    let mut session = tour::Session::with_reader(
        tour::TourProject {
            prebuilt: root.join("prebuilt"),
            root,
        },
        ready_reader(),
    );
    session.machine = tour::Tour::new(0.0);
    session.machine.fail(0.0, "caption: clock regression");
    window.tour = Some(session);

    for (seconds, visible) in [(0.5, true), (2.5, true), (10000.0, true)] {
        window.tour.as_mut().unwrap().freeze_at(seconds);
        let text = render(&mut window, &ctx, &home);
        assert_eq!(
            text.iter().any(|t| t.contains("caption: clock regression")),
            visible,
            "failed caption at {seconds} s: {text:?}"
        );
        assert!(text.iter().any(|t| t == "Stop"));
        assert!(text.iter().any(|t| t.contains("Waiting for")));
        assert!(window.tour.as_ref().unwrap().machine.active());
    }
}

/// Exercise the real card and input hook with a frozen clock and no native window.
#[test]
fn tour_next_clicks_wait_for_evidence_and_record_the_next_note() {
    for preference in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
        for ready in [false, true] {
            let temp = Temporary::new();
            let root = temp.0.join("project");
            fixture(&root);
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, preference);
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let paths = DisplayPaths::with_environment(None, Some(temp.0.clone()), None);
            let mut window =
                ProjectApp::new_with_paths(&cc, root.clone(), None, false, false, paths).unwrap();
            let record = temp.0.join("tour.tsv");
            window.trace = Some(ProjectTrace::new(&record).unwrap());
            let mut session = tour::Session::with_reader(
                tour::TourProject {
                    prebuilt: root.join("prebuilt"),
                    root,
                },
                ready_reader(),
            );
            session.machine = tour::Tour::new(0.0);
            session.machine.step = tour::Step::Run;
            session.machine.take_entry();
            session.machine.notes();
            if ready {
                session.machine.observe(
                    0.5,
                    &tour::Evidence {
                        running_at_four: true,
                        changes_drawn: true,
                        ..Default::default()
                    },
                );
            }
            session.freeze_at(0.5);
            window.tour = Some(session);
            let mut draw = |time, events| {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1800.0, 1200.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                window.raw_input_hook(&ctx, &mut input);
                ctx.run_ui(input, |ui| window.tour_card(ui))
            };
            let _ = draw(0.0, vec![]);
            let _ = draw(0.1, vec![]);
            let next = tour::next_button_rect(&ctx)
                .expect("the caption has Next")
                .center();
            let _ = draw(0.2, vec![egui::Event::PointerMoved(next)]);
            // A disabled Next explains which evidence the tour still needs.
            let output = draw(1.0, vec![]);
            let output_after_delay = draw(1.5, vec![]);
            if !ready {
                let mut painted = Vec::new();
                for output in [output, output_after_delay] {
                    for shape in output.shapes {
                        collect_text(&shape.shape, &mut painted);
                    }
                }
                assert!(
                    painted
                        .iter()
                        .any(|text| text.contains("Waiting for running at 4 cycles/s")),
                    "{painted:?}"
                );
            }
            for (time, pressed) in [(1.6, true), (1.7, false)] {
                let _ = draw(
                    time,
                    vec![egui::Event::PointerButton {
                        pos: next,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            let machine = &window.tour.as_ref().unwrap().machine;
            assert!(machine.active(), "Next must not stop the tour");
            assert_eq!(
                machine.step,
                if ready {
                    tour::Step::Source
                } else {
                    tour::Step::Run
                }
            );
            let text = std::fs::read_to_string(&record).unwrap();
            assert_eq!(
                text.matches("# tour next 3").count(),
                usize::from(ready),
                "{text}"
            );
            assert!(!text.contains("tour stopped"));
            // A physical click elsewhere leaves the tour active.
            let mut input = egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos: egui::pos2(1.0, 1.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            window.raw_input_hook(&ctx, &mut input);
            assert!(window.tour.as_ref().unwrap().machine.active());
            let text = std::fs::read_to_string(record).unwrap();
            assert!(!text.contains("# tour stopped at "));
        }
    }
}

/// A small chip surface using the real source-menu widget and project jump
/// handler. It exercises input, pacing and file navigation without a device
/// model, FPGA tools or a native window.
struct TourGesture {
    temporary: Temporary,
    window: ProjectApp,
    ctx: egui::Context,
    wire: egui::Pos2,
}

impl TourGesture {
    fn new(step: tour::Step) -> Self {
        let temporary = Temporary::new();
        let root = temporary.0.join("project");
        fixture(&root);
        std::fs::write(
            root.join("ecp5.toml"),
            "[project]\nname = \"tour\"\ntop = \"top\"\n[sources]\nverilog = [\"top.v\"]\n",
        )
        .unwrap();
        let prebuilt = temporary.0.join("prebuilt");
        std::fs::create_dir_all(&prebuilt).unwrap();
        std::fs::write(prebuilt.join("top.config"), "headless fixture").unwrap();
        std::fs::write(prebuilt.join("top_routed.json"), "{}").unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, egui::ThemePreference::Dark);
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut window = ProjectApp::new_with_paths(
            &cc,
            root.clone(),
            None,
            false,
            false,
            DisplayPaths::new(None),
        )
        .unwrap();
        window.app.reader_check = ready_reader;
        window.emulated_root = Some(root.clone());
        window.trace = Some(ProjectTrace::new(&temporary.0.join("tour.tsv")).unwrap());
        let mut session = tour::Session::with_reader(
            tour::TourProject {
                root,
                prebuilt: temporary.0.join("prebuilt"),
            },
            ready_reader(),
        );
        session.machine = tour::Tour::new(0.0);
        session.machine.step = step;
        session.machine.notes();
        // Build was explicitly requested by starting the tour; this fixture
        // represents its wait, without launching a native toolchain.
        if step == tour::Step::Build {
            session.machine.take_entry();
        }
        let wire = egui::pos2(300.0, 300.0);
        if step == tour::Step::Back {
            session.target = Some((wire, 0.0));
        }
        window.tour = Some(session);
        Self {
            temporary,
            window,
            ctx,
            wire,
        }
    }

    fn frame(&mut self, time: f64, events: Vec<egui::Event>) -> egui::FullOutput {
        let ctx = self.ctx.clone();
        self.window.tour.as_mut().unwrap().freeze_at(time);
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1800.0, 1200.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        self.window.raw_input_hook(&ctx, &mut input);
        ctx.run_ui(input, |ui| {
            self.window.poll_tour(&ctx);
            egui::Panel::top("tour_test_tabs").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (label, tab) in [("Project", ViewTab::Project), ("Chip", ViewTab::Chip)] {
                        if ui.selectable_label(self.window.tab == tab, label).clicked() {
                            self.window.set_tab(tab);
                        }
                    }
                    ui.label("Emulate");
                    ui.label("top.v");
                });
            });
            let mut chosen = None;
            egui::CentralPanel::default().show(ui, |ui| {
                if self.window.tab == ViewTab::Chip {
                    let rect = egui::Rect::from_center_size(self.wire, egui::vec2(140.0, 24.0));
                    ui.painter().line_segment(
                        [rect.left_center(), rect.right_center()],
                        egui::Stroke::new(3.0, crate::fabric_view::CHANGE),
                    );
                    let response = ui.interact(
                        rect,
                        egui::Id::new("tour_test_lit_wire"),
                        egui::Sense::click(),
                    );
                    response.context_menu(|ui| {
                        let location = ecp5_emu::sources::SourceLocation {
                            span: Span {
                                file: "top.v".into(),
                                line: 7,
                                end_line: 7,
                                column: Some(7),
                                end_column: Some(12),
                            },
                            kind: ecp5_emu::sources::SourceKind::Driven,
                        };
                        chosen = crate::fabric_view::source_menu(ui, &[location], None);
                        if chosen.is_some() {
                            ui.close();
                        }
                    });
                }
            });
            if let Some(span) = chosen {
                self.window.show_source(span);
            }
            if self.window.tab == ViewTab::Project && self.window.editor.active_path().is_some() {
                self.window.tour.as_mut().unwrap().editor_shown = true;
            }
            self.window.poll_tour(&ctx);
            self.window.tour_card(ui);
        })
    }

    fn click(&mut self, time: f64, point: egui::Pos2, button: egui::PointerButton) {
        for (offset, pressed) in [(0.0, true), (0.1, false)] {
            let _ = self.frame(
                time + offset,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    fn record(&self) -> String {
        std::fs::read_to_string(self.temporary.0.join("tour.tsv")).unwrap()
    }
}

fn tour_step_does_not_navigate(step: tour::Step, tab: ViewTab) {
    let mut fixture = TourGesture::new(step);
    fixture.window.tab = tab;
    fixture.window.tour.as_mut().unwrap().machine.prebuilt =
        step == tour::Step::Build || step == tour::Step::Chip;
    for time in [0.0, 0.1, 1.6, 5.0, 10.1] {
        let _ = fixture.frame(time, vec![]);
        assert!(fixture.window.tab == tab, "{step:?} switched the view");
        assert!(
            fixture.window.editor.active_path().is_none(),
            "{step:?} opened a file"
        );
        assert!(!fixture.window.tour.as_ref().unwrap().jump_requested);
    }
    if step == tour::Step::Back {
        assert!(fixture.window.tour.as_ref().unwrap().machine.active());
        assert!(!fixture.window.tour.as_ref().unwrap().machine.next_enabled());
    }
    let record = fixture.record();
    assert!(
        !record.contains("view ") && !record.contains("open "),
        "{record}"
    );
}

macro_rules! no_tour_jump {
    ($name:ident, $step:ident, $tab:ident) => {
        #[test]
        fn $name() {
            tour_step_does_not_navigate(tour::Step::$step, ViewTab::$tab);
        }
    };
}
no_tour_jump!(
    tour_build_does_not_navigate_or_load_the_fallback,
    Build,
    Project
);
no_tour_jump!(tour_chip_does_not_switch_to_the_chip, Chip, Project);
no_tour_jump!(tour_run_does_not_navigate, Run, Project);
no_tour_jump!(tour_source_does_not_open_the_editor, Source, Chip);
no_tour_jump!(
    tour_back_waits_on_the_chip_for_more_than_ten_seconds,
    Back,
    Chip
);
no_tour_jump!(tour_waves_does_not_switch_to_the_chip, Waves, Project);
no_tour_jump!(tour_done_does_not_navigate, Done, Chip);

#[test]
fn tour_back_waits_for_the_right_click_and_source_menu_choice_then_next() {
    let mut fixture = TourGesture::new(tour::Step::Back);
    fixture.window.tab = ViewTab::Chip;
    for time in [0.0, 0.1, 10.0] {
        let _ = fixture.frame(time, vec![]);
    }
    assert_eq!(
        fixture.window.tour.as_ref().unwrap().machine.caption(),
        "Right-click this lit wire → Show in source"
    );
    fixture.click(10.2, fixture.wire, egui::PointerButton::Secondary);
    for time in [10.4, 10.5, 10.6] {
        let _ = fixture.frame(time, vec![]);
    }
    assert!(fixture.window.editor.active_path().is_none());
    let menu = fixture
        .window
        .tour
        .as_ref()
        .unwrap()
        .menu
        .expect("the real source menu is open");
    fixture.click(10.8, menu.center(), egui::PointerButton::Primary);
    assert!(fixture.window.tab == ViewTab::Project);
    assert!(fixture
        .window
        .editor
        .active_path()
        .is_some_and(|p| p.ends_with("top.v")));
    let machine = &fixture.window.tour.as_ref().unwrap().machine;
    assert!(machine.active());
    assert_eq!(machine.step, tour::Step::Back);
    assert!(machine.next_enabled());
    let next = tour::next_button_rect(&fixture.ctx).unwrap().center();
    fixture.click(11.0, next, egui::PointerButton::Primary);
    assert_eq!(
        fixture.window.tour.as_ref().unwrap().machine.step,
        tour::Step::Waves
    );
    assert!(
        fixture.window.tab == ViewTab::Project,
        "Next never navigates"
    );
    assert!(!fixture.record().contains("tour show-me"));
}

#[test]
fn tour_show_me_points_opens_the_real_menu_slowly_and_records_the_request() {
    let mut fixture = TourGesture::new(tour::Step::Back);
    fixture.window.tab = ViewTab::Chip;
    for time in [0.0, 0.1] {
        let _ = fixture.frame(time, vec![]);
    }
    let show = fixture
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("tour_show_me_rect")))
        .unwrap()
        .center();
    fixture.click(0.2, show, egui::PointerButton::Primary);
    assert_eq!(fixture.record().matches("# tour show-me 5").count(), 1);
    let mut saw_menu = false;
    let mut saw_pointer = false;
    for tick in 4..=70 {
        let time = tick as f64 / 10.0;
        let output = fixture.frame(time, vec![]);
        saw_menu |= fixture.window.tour.as_ref().unwrap().menu.is_some();
        saw_pointer |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.stroke.color == crate::theme::colours(true).tour));
        if time < 3.0 {
            assert!(
                fixture.window.tab == ViewTab::Chip,
                "the gesture must be slow enough to follow"
            );
            assert!(fixture.window.editor.active_path().is_none());
        }
    }
    assert!(saw_menu && saw_pointer);
    assert!(fixture.window.tab == ViewTab::Project);
    assert!(fixture.window.editor.active_path().is_some());
    assert!(fixture.window.tour.as_ref().unwrap().machine.next_enabled());
    assert_eq!(
        fixture.window.tour.as_ref().unwrap().machine.step,
        tour::Step::Back
    );
    assert_eq!(fixture.record().matches("# tour show-me 5").count(), 1);
}

#[test]
fn tour_source_show_me_opens_the_real_editor_only_after_the_request() {
    let mut fixture = TourGesture::new(tour::Step::Source);
    fixture.window.tab = ViewTab::Chip;
    let mut frame = eframe::Frame::_new_kittest();
    let mut draw = |fixture: &mut TourGesture, time, events| {
        fixture.window.tour.as_mut().unwrap().freeze_at(time);
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1800.0, 1200.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        fixture.window.raw_input_hook(&fixture.ctx, &mut input);
        let _ = fixture
            .ctx
            .run_ui(input, |ui| fixture.window.ui(ui, &mut frame));
    };
    for time in [0.0, 0.1, 10.0] {
        draw(&mut fixture, time, vec![]);
        assert!(fixture.window.tab == ViewTab::Chip);
        assert!(fixture.window.editor.active_path().is_none());
    }
    let show = fixture
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("tour_show_me_rect")))
        .unwrap()
        .center();
    for (time, pressed) in [(10.2, true), (10.3, false)] {
        draw(
            &mut fixture,
            time,
            vec![
                egui::Event::PointerMoved(show),
                egui::Event::PointerButton {
                    pos: show,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for tick in 104..=165 {
        draw(&mut fixture, tick as f64 / 10.0, vec![]);
    }
    assert!(fixture.window.tab == ViewTab::Project);
    assert!(fixture
        .window
        .editor
        .active_path()
        .is_some_and(|p| p.ends_with("top.v")));
    assert_eq!(fixture.record().matches("# tour show-me 4").count(), 1);
    assert!(fixture.window.tour.as_ref().unwrap().machine.active());
}

#[test]
fn tour_build_failure_does_not_jump_to_a_diagnostic_file() {
    let mut fixture = TourGesture::new(tour::Step::Build);
    fixture.window.tab = ViewTab::Chip;
    let (_events_tx, events) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    done_tx
        .send(Outcome {
            ok: false,
            cancelled: false,
            steps: vec![],
            report: None,
            messages: vec![Message {
                kind: ecp5_emu::flow::messages::MessageKind::Diagnostic,
                severity: Severity::Error,
                step: Step::Synthesis,
                text: "syntax error in top.v:7".into(),
                file: Some("top.v".into()),
                line: Some(7),
                column: Some(7),
                count: 1,
            }],
            outputs: fixture.window.project.as_ref().unwrap().outputs(),
        })
        .unwrap();
    fixture.window.active = Some(ActiveTask::Build {
        events,
        done,
        cancel: Arc::new(AtomicBool::new(false)),
        started: Instant::now(),
        step_started: None,
        last_line: None,
    });
    fixture.window.poll();
    fixture.window.poll_tour(&fixture.ctx);
    assert!(fixture.window.tab == ViewTab::Chip);
    assert!(fixture.window.editor.active_path().is_none());
    assert!(fixture.window.tour.as_ref().unwrap().machine.active());
    assert!(!fixture.window.tour.as_ref().unwrap().machine.next_enabled());
    assert!(fixture.record().contains("# tour failed build"));
    for time in [100.0, 10000.0] {
        let _ = fixture.frame(time - 0.1, vec![]);
        let output = fixture.frame(time, vec![]);
        let mut text = Vec::new();
        for shape in output.shapes {
            collect_text(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|t| t == "Stop"));
        assert!(text.iter().any(|t| t.contains("Waiting for")));
        assert!(fixture.window.tour.as_ref().unwrap().machine.active());
    }
}

fn painted_label_point(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing painted {label}"))
}

#[test]
fn navigation_and_keys_preserve_every_active_step_until_the_stop_button_is_clicked() {
    for step in tour::Step::ALL
        .into_iter()
        .filter(|s| *s != tour::Step::Done)
    {
        let mut fixture = TourGesture::new(step);
        fixture.window.tab = ViewTab::Chip;
        for time in [0.0, 0.1] {
            let _ = fixture.frame(time, vec![]);
        }
        // Press, move and release: the real input hook sees a map drag.
        fixture.click(0.2, egui::pos2(100.0, 200.0), egui::PointerButton::Primary);
        let events = [
            egui::Event::PointerButton {
                pos: egui::pos2(100.0, 200.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerMoved(egui::pos2(250.0, 270.0)),
            egui::Event::PointerButton {
                pos: egui::pos2(250.0, 270.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, 100.0),
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Zoom(1.2),
            egui::Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(0),
                phase: egui::TouchPhase::Start,
                pos: egui::pos2(100.0, 200.0),
                force: None,
            },
            egui::Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(0),
                phase: egui::TouchPhase::End,
                pos: egui::pos2(100.0, 200.0),
                force: None,
            },
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        for (i, event) in events.into_iter().enumerate() {
            let _ = fixture.frame(1.0 + i as f64, vec![event]);
            assert!(
                fixture.window.tour.as_ref().unwrap().machine.active(),
                "{step:?}"
            );
            assert_eq!(fixture.window.tour.as_ref().unwrap().machine.step, step);
        }
        for (i, name) in ["Project", "Chip"].into_iter().enumerate() {
            let output = fixture.frame(19.0 + 2.0 * i as f64, vec![]);
            let point = painted_label_point(&output, name);
            fixture.click(20.0 + 2.0 * i as f64, point, egui::PointerButton::Primary);
            assert!(fixture.window.tour.as_ref().unwrap().machine.active());
        }
        let output = fixture.frame(10000.0, vec![]);
        let stop = painted_label_point(&output, "Stop");
        let mut text = Vec::new();
        for shape in output.shapes {
            collect_text(&shape.shape, &mut text);
        }
        assert!(
            text.iter().any(|s| s.contains("Waiting for")),
            "{step:?}: {text:?}"
        );
        assert!(fixture.window.tour.as_ref().unwrap().machine.active());
        assert!(!fixture.record().contains("tour stopped"));
        fixture.click(10001.0, stop, egui::PointerButton::Primary);
        assert!(!fixture.window.tour.as_ref().unwrap().machine.active());
        assert_eq!(fixture.record().matches("# tour stopped at ").count(), 1);
    }
}

#[test]
fn every_tour_step_names_exactly_the_buttons_it_renders_before_and_after_evidence() {
    let mut machine = tour::Tour::new(0.0);
    let ctx = egui::Context::default();
    for step in tour::Step::ALL {
        assert_eq!(machine.step, step);
        for ready in [false, true] {
            if ready {
                machine.observe(
                    step.number() as f64,
                    &tour::Evidence {
                        build: Some(Ok(())),
                        chip_ready: true,
                        chip: true,
                        running_at_four: true,
                        changes_drawn: true,
                        source_probed: true,
                        source_jumped: true,
                        waves: true,
                        ..Default::default()
                    },
                );
            }
            let mut text = Vec::new();
            for _ in 0..2 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        tour::card(
                            ui,
                            ui.max_rect(),
                            machine.caption(),
                            step,
                            machine.active(),
                            None,
                            machine.waiting(),
                        );
                    },
                );
                text.clear();
                for shape in output.shapes {
                    collect_text(&shape.shape, &mut text);
                }
            }
            let captions = text
                .iter()
                .filter(|s| !["Stop", "Next", "Show me"].contains(&s.as_str()))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            for button in ["Stop", "Next", "Show me"] {
                assert_eq!(
                    captions.contains(button),
                    text.iter().any(|s| s == button),
                    "{step:?}, ready={ready}, {button}: {text:?}"
                );
            }
            assert_eq!(
                text.iter().any(|s| s == "Show me"),
                machine.active() && step.needs_gesture()
            );
        }
        machine.next(step.number() as f64);
    }
}
