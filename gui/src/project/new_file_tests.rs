//! New-file dialogs, script dispatch, publication and project refresh, without a display.
use super::*;
use new_file::{Kind, Request};
use std::fs;

fn fixture(vhdl: bool) -> (PathBuf, ProjectApp, egui::Context) {
    let root = crate::test_temp::unique_dir("ecp5-new-file-")
        .canonicalize()
        .unwrap();
    if vhdl {
        fs::write(
            root.join("top.vhd"),
            "entity top is end entity; architecture rtl of top is begin end architecture;",
        )
        .unwrap();
        fs::write(root.join("top.lpf"), "").unwrap();
        fs::write(root.join("ecp5.toml"), "[sources]\nhdl = ['top.vhd']\n").unwrap();
    } else {
        flow::new_project::create(&flow::new_project::Options::new(&root)).unwrap();
    }
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
    window.app.isolated_preferences(root.join("recent.txt"));
    window.trace = Some(ProjectTrace::new(&root.join("project.tsv")).unwrap());
    window.app.trace(&root.join("window.tsv")).unwrap();
    (root, window, ctx)
}

fn frame(
    window: &mut ProjectApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
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
        |ui| eframe::App::ui(window, ui, &mut frame),
    )
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn text(output: &egui::FullOutput, needle: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(s) if s.galley.text().contains(needle)))
}

#[test]
fn dialog_creates_every_kind_refreshes_tree_and_opens_tab() {
    for (kind, extension, expected) in [
        (Kind::Verilog, "v", "module counter"),
        (Kind::SystemVerilog, "sv", "module counter"),
        (Kind::Vhdl, "vhd", "entity counter is"),
        (Kind::Constraints, "lpf", "# New constraints"),
        (Kind::Hex, "hex", "// Hex data"),
    ] {
        let (root, mut window, ctx) = fixture(kind == Kind::Vhdl);
        window.open_relative(Path::new("ecp5.toml"), None);
        let original_manifest = window
            .editor
            .text(&root.join("ecp5.toml"))
            .unwrap()
            .to_owned();
        window.begin_new_file(Some(PathBuf::from("counter.v")));
        let dialog = window.new_file_dialog.as_mut().unwrap();
        dialog.set_kind(kind);
        assert_eq!(dialog.path, format!("counter.{extension}"));
        assert_eq!(dialog.name, "counter");
        frame(&mut window, &ctx, Vec::new());
        let path = root.join(format!("counter.{extension}"));
        frame(
            &mut window,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(window.new_file_dialog.is_none());
        assert!(fs::read_to_string(&path).unwrap().contains(expected));
        assert_eq!(window.editor.active_path(), Some(path.as_path()));
        assert!(window.status.contains("rebuild needed"));
        let refreshed = fs::read_to_string(root.join("ecp5.toml")).unwrap();
        assert_eq!(
            window.editor.text(&root.join("ecp5.toml")),
            Some(refreshed.as_str())
        );
        if kind == Kind::SystemVerilog {
            assert_ne!(original_manifest, refreshed);
            assert!(window
                .status
                .contains("sources.hdl created from sources.verilog"));
        }
        let painted = frame(&mut window, &ctx, Vec::new());
        assert!(painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(s) if s.pos.x < 260.0 && s.galley.text() == format!("counter.{extension}"))), "new file is visible in the tree");
        if matches!(
            kind,
            Kind::Verilog | Kind::SystemVerilog | Kind::Vhdl | Kind::Hex
        ) {
            assert!(
                text(&painted, &format!("counter.{extension}")),
                "new input appears in the tree/editor"
            );
            let p = window.project.as_ref().unwrap();
            assert!(p
                .hdl_roots()
                .iter()
                .chain(&p.files)
                .any(|p| p == Path::new(&format!("counter.{extension}"))));
        } else {
            assert_eq!(
                window.project.as_ref().unwrap().constraints,
                PathBuf::from("top.lpf")
            );
        }
        drop(window);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn name_follows_filename_and_kind_but_cancel_and_existing_files_are_safe() {
    let (root, mut window, ctx) = fixture(false);
    window.begin_new_file(Some(PathBuf::from("counter.v")));
    let dialog = window.new_file_dialog.as_mut().unwrap();
    dialog.set_path("rtl/timer.v".into());
    assert_eq!(dialog.name, "timer");
    dialog.set_kind(Kind::SystemVerilog);
    assert_eq!(dialog.path, "rtl/timer.sv");
    frame(&mut window, &ctx, Vec::new());
    frame(
        &mut window,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(!root.join("rtl").exists());
    assert!(window.new_file_dialog.is_none());
    let original = fs::read(root.join("top.v")).unwrap();
    let manifest = fs::read(root.join("ecp5.toml")).unwrap();
    window.begin_new_file(Some(PathBuf::from("top.v")));
    frame(&mut window, &ctx, Vec::new());
    frame(
        &mut window,
        &ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert!(window
        .new_file_dialog
        .as_ref()
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("already exists"));
    let drawn = frame(&mut window, &ctx, Vec::new());
    assert!(text(&drawn, "already exists"));
    assert_eq!(fs::read(root.join("top.v")).unwrap(), original);
    assert_eq!(fs::read(root.join("ecp5.toml")).unwrap(), manifest);
    drop(window);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dirty_manifest_is_refused_without_saving_other_edits_then_clean_tab_is_refreshed() {
    let (root, mut window, ctx) = fixture(false);
    let manifest = root.join("ecp5.toml");
    let saved = fs::read_to_string(&manifest).unwrap();
    window
        .editor
        .replace_line(
            &manifest,
            "ecp5.toml".into(),
            1,
            "[project] # unsaved manifest",
        )
        .unwrap();
    window
        .editor
        .replace_line(&root.join("top.v"), "top.v".into(), 1, "// unsaved HDL")
        .unwrap();
    let request = || Request {
        path: "rtl/counter.sv".into(),
        name: None,
    };
    assert!(window
        .create_file(request(), &ctx)
        .unwrap_err()
        .contains("Save ecp5.toml"));
    assert!(!root.join("rtl").exists());
    assert_eq!(fs::read_to_string(&manifest).unwrap(), saved);
    window.open_relative(Path::new("ecp5.toml"), None);
    window.editor.save_active().unwrap();
    window.create_file(request(), &ctx).unwrap();
    assert!(window.editor.is_dirty(&root.join("top.v")));
    assert_eq!(
        window.editor.text(&manifest),
        Some(fs::read_to_string(&manifest).unwrap().as_str())
    );
    assert!(window
        .status
        .contains("sources.hdl created from sources.verilog"));
    assert!(!fs::read_to_string(root.join("top.v"))
        .unwrap()
        .contains("unsaved HDL"));
    drop(window);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_file_drops_build_emulator_and_source_associations_and_requires_rebuild() {
    let (root, mut window, ctx) = fixture(false);
    let outputs = window.project.as_ref().unwrap().outputs();
    fs::create_dir_all(&window.project.as_ref().unwrap().build_dir).unwrap();
    fs::write(&outputs.config, "preexisting config").unwrap();
    fs::write(&outputs.routed_json, "{}").unwrap();
    window.emulated_root = Some(root.clone());
    window.last = Some(Outcome {
        ok: true,
        cancelled: false,
        steps: Vec::new(),
        messages: Vec::new(),
        report: None,
        outputs,
    });
    assert!(window.emulate_blocker().is_none());
    window
        .create_file(
            Request {
                path: "counter.v".into(),
                name: None,
            },
            &ctx,
        )
        .unwrap();
    assert!(window.last.is_none());
    assert!(window.emulated_root.is_none());
    assert!(window.app.ready().is_none());
    assert!(!window.app.has_design());
    assert!(window.emulate_blocker().unwrap().contains("inputs changed"));
    assert_eq!(window.tab, ViewTab::Project);
    drop(window);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_script_parses_names_hashes_entity_alias_and_records_both_outcomes() {
    let (root, mut window, ctx) = fixture(false);
    let script = "0 new-file {\"path\":\"rtl #1/counter.sv\",\"module\":\"custom_counter\"} # comment\n0 new-file {\"path\":\"rtl #1/counter.sv\"}\n";
    let steps = script::parse_script(script).unwrap();
    assert_eq!(steps.len(), 2);
    window.script = Some(ScriptRunner::from_steps(steps));
    window.poll_script(&ctx);
    assert!(fs::read_to_string(root.join("rtl #1/counter.sv"))
        .unwrap()
        .contains("module custom_counter"));
    let vhdl =
        script::parse_script("0 new-file {\"path\":\"counter.vhd\",\"entity\":\"counter_entity\"}")
            .unwrap();
    assert!(
        matches!(&vhdl[0].command, ScriptCommand::NewFile(Request { name: Some(name), .. }) if name == "counter_entity")
    );
    for invalid in [
        "[]",
        "{}",
        "{\"path\":true}",
        "{\"path\":\"a.v\",\"module\":1}",
        "{\"path\":\"a.v\",\"module\":\"a\",\"entity\":\"b\"}",
    ] {
        assert!(script::parse_script(&format!("0 new-file {invalid}")).is_err());
    }
    drop(window);
    let gui = fs::read_to_string(root.join("window.tsv")).unwrap();
    let project = fs::read_to_string(root.join("project.tsv")).unwrap();
    for payload in [
        "file new \"rtl #1/counter.sv\" ok",
        "file new \"rtl #1/counter.sv\" failed new file already exists:",
        "manifest sources.hdl created from sources.verilog ok",
    ] {
        assert!(gui.contains(&format!("# {payload}")), "{gui}");
        assert!(
            project
                .lines()
                .any(|l| l.split_once('\t').unwrap().1.starts_with(payload)),
            "{project}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_file_shortcut_requires_an_idle_project_and_no_text_focus() {
    let (root, mut window, ctx) = fixture(false);
    frame(
        &mut window,
        &ctx,
        vec![key(egui::Key::N, egui::Modifiers::COMMAND)],
    );
    assert!(window.new_file_dialog.is_some());
    frame(
        &mut window,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(window.new_file_dialog.is_none());
    let id = egui::Id::new("focused-text");
    ctx.memory_mut(|m| m.request_focus(id));
    frame(
        &mut window,
        &ctx,
        vec![key(egui::Key::N, egui::Modifiers::COMMAND)],
    );
    assert!(window.new_file_dialog.is_none());
    drop(window);
    fs::remove_dir_all(root).unwrap();
}
