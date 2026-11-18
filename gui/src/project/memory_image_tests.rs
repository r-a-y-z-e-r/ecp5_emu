//! BRAM membership menus, confirmation, refresh and script records without a display.
use super::*;
use std::fs;

fn fixture(file: &str) -> (PathBuf, ProjectApp, egui::Context) {
    let root = crate::test_temp::unique_dir("ecp5-memory-image-")
        .canonicalize()
        .unwrap();
    flow::new_project::create(&flow::new_project::Options::new(&root)).unwrap();
    fs::write(root.join(file), "00\n").unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut window = ProjectApp::new(&cc, root.clone(), None, false, false).unwrap();
    ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
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
    let mut native = eframe::Frame::_new_kittest();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1500.0, 1100.0),
            )),
            events,
            time: Some(ctx.input(|i| i.time) + 0.1),
            ..Default::default()
        },
        |ui| eframe::App::ui(window, ui, &mut native),
    )
}

fn drawn(window: &mut ProjectApp, ctx: &egui::Context) -> egui::FullOutput {
    frame(window, ctx, vec![]);
    frame(window, ctx, vec![])
}

fn rect(output: &egui::FullOutput, label: &str, tree: bool) -> egui::Rect {
    crate::test_temp::painted_text(output)
        .into_iter()
        .find(|(text, rect)| text == label && (!tree || rect.min.x < 260.0))
        .unwrap_or_else(|| {
            panic!(
                "missing {label}: {:?}",
                crate::test_temp::painted_text(output)
            )
        })
        .1
}

fn click(window: &mut ProjectApp, ctx: &egui::Context, pos: egui::Pos2) {
    for pressed in [true, false] {
        frame(window, ctx, crate::test_temp::click(pos, pressed));
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn menu(window: &mut ProjectApp, ctx: &egui::Context, file: &str, label: &str) {
    let pos = rect(&drawn(window, ctx), file, true).center();
    for pressed in [true, false] {
        frame(
            window,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let pos = rect(&drawn(window, ctx), label, false).center();
    click(window, ctx, pos);
}

#[test]
fn memory_image_context_confirmation_refreshes_tree_steps_manifest_and_build_association() {
    let (root, mut window, ctx) = fixture("fw.hex");
    let manifest = root.join("ecp5.toml");
    let original = fs::read(&manifest).unwrap();
    window.open_relative(Path::new("ecp5.toml"), None);
    window.emulated_root = Some(root.clone());
    window.last = Some(Outcome {
        ok: true,
        cancelled: false,
        steps: vec![],
        messages: vec![],
        report: None,
        outputs: window.project.as_ref().unwrap().outputs(),
    });
    menu(&mut window, &ctx, "fw.hex", "Use as BRAM image (ecpbram)…");
    assert!(window.memory_image_change.is_some());
    let confirmation = drawn(&mut window, &ctx);
    rect(&confirmation, MEMORY_IMAGE_CONFIRMATION, false);
    assert_eq!(fs::read(&manifest).unwrap(), original);
    assert!(window.project.as_ref().unwrap().memory_images.is_empty());
    frame(&mut window, &ctx, vec![key(egui::Key::Escape)]);
    assert!(window.memory_image_change.is_none());
    assert_eq!(fs::read(&manifest).unwrap(), original);
    menu(&mut window, &ctx, "fw.hex", "Use as BRAM image (ecpbram)…");
    let apply = rect(&drawn(&mut window, &ctx), "Apply", false).center();
    click(&mut window, &ctx, apply);
    assert!(window.memory_image_change.is_none());
    let project = window.project.as_ref().unwrap();
    assert_eq!(project.memory_images, [PathBuf::from("fw.hex")]);
    assert!(!project.files.contains(&PathBuf::from("fw.hex")));
    assert!(window
        .steps
        .iter()
        .any(|(s, p)| *s == Step::Swap && matches!(p, StepProgress::Pending)));
    assert!(window.last.is_none());
    assert!(window.emulated_root.is_none());
    assert!(!window.app.has_design());
    assert!(window.emulate_blocker().unwrap().contains("inputs changed"));
    assert_eq!(
        window.editor.text(&manifest),
        Some(fs::read_to_string(&manifest).unwrap().as_str())
    );
    assert!(window.status.contains("memory.images appended fw.hex"));
    assert!(window.status.contains("ecpbram.txt updated"));
    let output = drawn(&mut window, &ctx);
    rect(&output, "Memory images (1)", true);
    rect(&output, "fw.hex", true);
    let swap = rect(&output, "Memory images (ecpbram)", true).center();
    let mut tooltip = egui::FullOutput::default();
    for _ in 0..4 {
        tooltip = frame(&mut window, &ctx, vec![egui::Event::PointerMoved(swap)]);
    }
    let text = crate::test_temp::painted_text(&tooltip);
    assert!(
        text.iter().any(|(s, _)| s.contains("Swaps:\nfw.hex")),
        "{text:?}"
    );
    menu(&mut window, &ctx, "fw.hex", "Stop using as BRAM image");
    rect(&drawn(&mut window, &ctx), MEMORY_IMAGE_CONFIRMATION, false);
    let apply = rect(&drawn(&mut window, &ctx), "Apply", false).center();
    click(&mut window, &ctx, apply);
    assert!(window.project.as_ref().unwrap().memory_images.is_empty());
    assert_eq!(
        window.project.as_ref().unwrap().files,
        [PathBuf::from("fw.hex")]
    );
    assert!(!window.steps.iter().any(|(s, _)| *s == Step::Swap));
    let output = drawn(&mut window, &ctx);
    assert!(crate::test_temp::painted_text(&output)
        .iter()
        .any(|(s, _)| s.contains("none listed — $readmemh")));
    drop(window);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn memory_image_confirmation_resolves_only_dirty_membership_tabs_and_preserves_other_edits() {
    let (root, mut window, ctx) = fixture("fw.mem");
    let manifest = root.join("ecp5.toml");
    let original = fs::read(&manifest).unwrap();
    window
        .editor
        .replace_line(
            &manifest,
            "ecp5.toml".into(),
            1,
            "[project] # unsaved identity",
        )
        .unwrap();
    window
        .editor
        .replace_line(
            &root.join("top.v"),
            "top.v".into(),
            1,
            "// keep this unsaved HDL",
        )
        .unwrap();
    let error = window
        .change_memory_image(Path::new("fw.mem"), true, &ctx)
        .unwrap_err();
    assert!(error.contains("Save ecp5.toml"));
    assert_eq!(fs::read(&manifest).unwrap(), original);
    window.begin_memory_image("fw.mem".into(), true);
    frame(&mut window, &ctx, vec![]);
    frame(&mut window, &ctx, vec![key(egui::Key::Enter)]);
    assert!(window.memory_image_change.is_some());
    assert_eq!(fs::read(&manifest).unwrap(), original);
    let output = drawn(&mut window, &ctx);
    let save = rect(&output, "Save ecp5.toml", false).center();
    click(&mut window, &ctx, save);
    assert!(!window.editor.is_dirty(&manifest));
    assert!(window.editor.is_dirty(&root.join("top.v")));
    let apply = rect(&drawn(&mut window, &ctx), "Apply", false).center();
    click(&mut window, &ctx, apply);
    assert!(window.memory_image_change.is_none());
    assert!(fs::read_to_string(&manifest)
        .unwrap()
        .contains("# unsaved identity"));
    assert_eq!(
        window.project.as_ref().unwrap().memory_images,
        [PathBuf::from("fw.mem")]
    );
    assert_eq!(
        window.editor.text(&manifest),
        Some(fs::read_to_string(&manifest).unwrap().as_str())
    );
    assert!(window.editor.is_dirty(&root.join("top.v")));
    assert!(!fs::read_to_string(root.join("top.v"))
        .unwrap()
        .contains("unsaved HDL"));
    window.open_relative(Path::new("ecpbram.txt"), None);
    window
        .editor
        .replace_line(
            &root.join("ecpbram.txt"),
            "ecpbram.txt".into(),
            1,
            "# unsaved legacy",
        )
        .unwrap();
    assert!(window
        .change_memory_image(Path::new("fw.mem"), false, &ctx)
        .unwrap_err()
        .contains("Save ecpbram.txt"));
    drop(window);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn memory_image_script_parses_strict_json_and_records_success_reverse_and_failure_in_both_traces() {
    let (root, mut window, ctx) = fixture("fw #1.init");
    let steps = script::parse_script("0 memory-image {\"path\":\"fw #1.init\",\"use\":true} # comment\n0 memory-image {\"path\":\"fw #1.init\",\"use\":false}\n0 memory-image {\"path\":\"top.v\",\"use\":true}\n").unwrap();
    assert_eq!(steps.len(), 3);
    assert!(
        matches!(&steps[0].command, ScriptCommand::MemoryImage { path, use_as_image: true } if path == Path::new("fw #1.init"))
    );
    window.script = Some(ScriptRunner::from_steps(steps));
    window.poll_script(&ctx);
    assert!(window.project.as_ref().unwrap().memory_images.is_empty());
    assert!(window
        .project
        .as_ref()
        .unwrap()
        .files
        .contains(&PathBuf::from("fw #1.init")));
    for invalid in [
        "[]",
        "{}",
        "{\"path\":\"fw.hex\"}",
        "{\"path\":true,\"use\":true}",
        "{\"path\":\"fw.hex\",\"use\":1}",
        "{\"path\":\"fw.hex\",\"use\":true,\"extra\":0}",
    ] {
        assert!(
            script::parse_script(&format!("0 memory-image {invalid}")).is_err(),
            "{invalid}"
        );
    }
    drop(window);
    for trace in ["project.tsv", "window.tsv"] {
        let text = fs::read_to_string(root.join(trace)).unwrap();
        for payload in [
            "file memory-image \"fw #1.init\" on ok",
            "file memory-image \"fw #1.init\" off ok",
            "file memory-image top.v on failed BRAM images require",
            "manifest memory.images appended fw #1.init ok",
        ] {
            assert_eq!(text.matches(payload).count(), 1, "{trace}: {text}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}
