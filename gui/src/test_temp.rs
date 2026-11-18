//! Temporary directories for tests in this process.
use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static TEMPORARY_NANOS: Cell<Option<u128>> = const { Cell::new(None) };
    static HOME_FIXTURE: TestHome = TestHome(unique_dir("ecp5-gui-home-"));
}

struct TestHome(PathBuf);

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A home shared within one test thread, without changing the process environment.
pub(crate) fn home_dir() -> PathBuf {
    HOME_FIXTURE.with(|home| home.0.clone())
}

pub(crate) fn unique_dir(prefix: &str) -> PathBuf {
    let nanos = TEMPORARY_NANOS.with(|time| {
        time.get().unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        })
    });
    let path = std::env::temp_dir().join(format!(
        "{prefix}{}-{}-{nanos}",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

/// A curated demo and a hardware test, without touching the repository catalogue.
pub(crate) fn demo_fixture() -> PathBuf {
    let root = unique_dir("ecp5-dev-mode-");
    for name in ["hero", "hardware"] {
        let project = root.join("examples/ulx3s").join(name);
        fs::create_dir_all(project.join("build")).unwrap();
        fs::write(project.join("ecp5.toml"), "").unwrap();
        fs::write(project.join("top.v"), "module top; endmodule").unwrap();
        fs::write(project.join("build/top.config"), "").unwrap();
    }
    fs::write(
        root.join("examples/showcase.toml"),
        "showcase = ['ulx3s/hero']\n",
    )
    .unwrap();
    root
}

pub(crate) fn painted_text(output: &eframe::egui::FullOutput) -> Vec<(String, eframe::egui::Rect)> {
    use eframe::egui;
    fn collect(shape: &egui::Shape, text: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => text.push((
                t.galley.text().into(),
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

pub(crate) fn click(pos: eframe::egui::Pos2, pressed: bool) -> Vec<eframe::egui::Event> {
    use eframe::egui;
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

#[test]
fn consecutive_temporary_folders_are_distinct_even_with_the_same_timestamp() {
    use std::collections::HashSet;

    // Filesystem calls can outlast a clock tick; hold time still so they
    // cannot hide collisions in names that use only the process and time.
    TEMPORARY_NANOS.with(|time| time.set(Some(0)));
    let paths: HashSet<_> = (0..100).map(|_| unique_dir("ecp5-test-temp-")).collect();
    TEMPORARY_NANOS.with(|time| time.set(None));
    let distinct = paths.len();
    for path in paths {
        fs::remove_dir_all(path).unwrap();
    }
    assert_eq!(distinct, 100);
}
