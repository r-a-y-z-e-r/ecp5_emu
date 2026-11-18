//! Opening a demo preserves its sources while waiting for the native reader.
use super::*;
use crate::start::reader::{self, Status};
use std::fs;

#[test]
fn demo_opens_wait_for_the_reader_with_names_and_project_sources() {
    let root = std::env::temp_dir().join(format!("ecp5-demo-open-{}", std::process::id()));
    let project = root.join("examples/ulx3s/demo");
    fs::create_dir_all(project.join("build")).unwrap();
    for (name, text) in [
        ("top.v", "module top(); endmodule"),
        ("ecp5.toml", ""),
        ("top.lpf", ""),
        ("build/top.config", ""),
        ("build/top_routed.json", "{}"),
    ] {
        fs::write(project.join(name), text).unwrap();
    }
    fs::write(root.join("examples/ulx3s/README.md"), "").unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, "Open a design".into(), None);
    app.demos = start::demos(&root).unwrap();
    app.reader_check = || reader::fixed(Status::Building);
    let record = root.join("window.tsv");
    app.trace(&record).unwrap();
    let config = app.demos[0].built_config().unwrap();
    app.action(&ctx, Action::Load(config.clone()));
    assert!(
        app.commands.is_none(),
        "native loading waits for the reader"
    );
    let pending = app.pending_design.as_ref().unwrap();
    assert_eq!(pending.config, config);
    assert_eq!(pending.names, Some(project.join("build/top_routed.json")));
    assert!(!pending.run);
    assert_eq!(app.source_root, Some(project.canonicalize().unwrap()));
    assert!(
        app.source_files.is_some(),
        "cross-probing keeps the project's inputs"
    );
    assert_eq!(app.browser_error.as_deref(), Some(reader::BUILDING));
    app.reader
        .as_ref()
        .unwrap()
        .finish(Err(reader::MISSING.into()));
    app.poll_open(&ctx);
    assert!(app.pending_design.is_none());
    assert!(app.commands.is_none());
    assert_eq!(app.browser_error.as_deref(), Some(reader::MISSING));
    assert_eq!(app.state().2.as_deref(), Some(reader::MISSING));
    drop(app);
    assert!(fs::read_to_string(record)
        .unwrap()
        .lines()
        .any(|line| line == "# reader missing"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepare_reader_uses_injected_checks_for_initial_and_refreshed_opens() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut app = App::new(&cc, "Open a design".into(), None);
    app.reader_check = || reader::fixed(Status::Building);
    assert!(app.prepare_reader().is_ok());
    assert_eq!(app.reader.as_ref().unwrap().status(), Status::Building);
    assert_eq!(std::mem::take(&mut app.open_notes), ["reader missing"]);

    app.reader
        .as_ref()
        .unwrap()
        .finish(Err("injected failure".into()));
    app.reader_check = || reader::fixed(Status::Ready);
    assert!(app.prepare_reader().is_ok());
    assert_eq!(app.reader.as_ref().unwrap().status(), Status::Ready);
    assert!(!app.reader.as_ref().unwrap().build_requested);
    assert_eq!(app.open_notes, ["reader found"]);
}
