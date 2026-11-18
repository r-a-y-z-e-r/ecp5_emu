//! Demo portability and real-tool comparisons against the original projects.
use ecp5_emu::flow::{
    self,
    demo_copy::{self, Options},
    BuildOptions, Project, Tools,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "demo copy test {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_no_links(path: &Path) {
    let info = fs::symlink_metadata(path).unwrap();
    assert!(!info.file_type().is_symlink(), "{}", path.display());
    if info.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            assert_no_links(&entry.unwrap().path());
        }
    }
}

#[test]
fn every_demo_opens_without_symlinks() {
    let folder = Folder::new();
    for demo in demo_copy::catalogue().unwrap().demos {
        let demo = demo.name;
        let original = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(&demo);
        let manifest = fs::read(original.join("ecp5.toml")).ok();
        let mut options = Options::new(folder.0.join(&demo), &demo);
        options.name = format!("My {demo}");
        let copied =
            demo_copy::create(&options).unwrap_or_else(|error| panic!("{demo}: {error:#}"));
        assert_eq!(Project::open(&copied.root).unwrap().name, options.name);
        assert_no_links(&copied.root);
        assert!(!copied.build_dir.exists());
        for input in copied
            .verilog
            .iter()
            .chain(&copied.files)
            .chain(&copied.memory_images)
            .chain(std::iter::once(&copied.constraints))
        {
            assert_eq!(
                fs::read(copied.root.join(input)).unwrap(),
                fs::read(original.join(input)).unwrap(),
                "{demo}: {}",
                input.display()
            );
        }
        assert_eq!(fs::read(original.join("ecp5.toml")).ok(), manifest);
        let notices = fs::read_to_string(copied.root.join("THIRD_PARTY_NOTICES.txt")).unwrap();
        assert!(notices.contains(&format!("Source demo: examples/{demo}")));
        assert!(notices.contains(include_str!("../LICENSE")));
        assert!(notices.contains(include_str!("../assets/ulx3s/LICENSE.md")));
        if copied.root.join("picorv32.v").exists() {
            let core = fs::read_to_string(copied.root.join("picorv32.v")).unwrap();
            assert!(notices.contains(&core[..core.find("*/").unwrap() + 2]));
        }
        if copied.root.join("serv/LICENSE").exists() {
            assert!(
                notices.contains(&fs::read_to_string(copied.root.join("serv/LICENSE")).unwrap())
            );
        }
        for path in [
            "README.md",
            "check.json",
            "gui.script",
            "expected",
            "sim",
            "repro",
            "firmware",
        ] {
            if original.join(path).exists() {
                assert!(copied.root.join(path).exists(), "{demo}: {path}");
            }
        }
    }
}

#[test]
fn repository_showcase_table_names_real_demos_when_present() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let table = examples.join("showcase.toml");
    if !table.exists() {
        eprintln!("{} is absent; all demos are showcase", table.display());
        return;
    }
    let catalogue = demo_copy::catalogue_in(&examples).unwrap();
    assert!(
        catalogue.unknown_showcase.is_empty(),
        "{} lists unknown demos: {:?}",
        table.display(),
        catalogue.unknown_showcase
    );
    let parsed: toml::Value = toml::from_str(&fs::read_to_string(&table).unwrap()).unwrap();
    for name in parsed["showcase"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(
            catalogue
                .demos
                .iter()
                .any(|demo| demo.name == name && demo.showcase),
            "{}: {name} is not a showcase demo",
            table.display()
        );
    }
}

#[test]
fn cli_lists_and_copies_demos() {
    let binary = env!("CARGO_BIN_EXE_ecp5_emu");
    let catalogue = demo_copy::catalogue().unwrap();
    for dev in [false, true] {
        let mut command = Command::new(binary);
        command.args(["new", "--list-demos"]);
        if dev {
            command.arg("--dev");
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            catalogue
                .visible(dev)
                .map(|demo| demo.name.as_str())
                .collect::<Vec<_>>()
        );
    }
    let folder = Folder::new();
    let destination = folder.0.join("CLI copied project");
    let output = Command::new(binary)
        .arg("new")
        .arg(&destination)
        .args(["--from-demo", "gallery/riscv_shell", "--name", "My shell"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project = Project::open(&destination).unwrap();
    assert_eq!(project.name, "My shell");
    assert_no_links(&project.root);
    assert!(!project.build_dir.exists());
    let output = Command::new(binary)
        .args(["new", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--list-demos"));
    assert!(help.contains("--dev"));
    assert!(help.contains("showcase.toml"));
    assert!(!help.contains("not available yet"));
}

#[test]
fn cli_filters_showcase_and_reports_diagnostics_in_temporary_installation() {
    let folder = Folder::new();
    let examples = folder.0.join("examples");
    for demo in [
        "gallery/featured",
        "ulx3s/featured",
        "ulx3s/development",
        "ulx3s/excluded",
    ] {
        let source = examples.join(demo);
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("top.v"), "module top; endmodule\n").unwrap();
    }
    for lpf in ["up.lpf", "down.lpf"] {
        fs::write(examples.join("ulx3s/excluded").join(lpf), "").unwrap();
    }
    fs::write(examples.join("ulx3s/README.md"), "").unwrap();
    fs::write(examples.join("gallery/README.md"), "").unwrap();
    fs::write(
        examples.join("showcase.toml"),
        "showcase = ['ulx3s/featured', 'gallery/featured', 'ulx3s/missing']\n",
    )
    .unwrap();
    // Root discovery prefers the executable's ancestors, so install a copy
    // beside the temporary examples rather than changing the process cwd.
    let binary = folder.0.join(
        Path::new(env!("CARGO_BIN_EXE_ecp5_emu"))
            .file_name()
            .unwrap(),
    );
    fs::copy(env!("CARGO_BIN_EXE_ecp5_emu"), &binary).unwrap();
    for dev in [false, true] {
        let mut command = Command::new(&binary);
        command.args(["new", "--list-demos"]);
        if dev {
            command.arg("--dev");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stderr}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            if dev {
                "gallery/featured\nulx3s/development\nulx3s/featured\n"
            } else {
                "gallery/featured\nulx3s/featured\n"
            }
        );
        assert!(stderr.contains("Excluded ulx3s/excluded:"), "{stderr}");
        assert!(
            stderr.contains("Unknown showcase demo: ulx3s/missing"),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 2);
    }
}

fn same_build(demo: &str) {
    let folder = Folder::new();
    let copied = demo_copy::create(&Options::new(folder.0.join("copied project"), demo)).unwrap();
    let mut original = Project::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(demo),
    )
    .unwrap();
    original
        .set_build_dir(folder.0.join("original build"))
        .unwrap();
    let options = BuildOptions {
        tools: Tools::from_env(),
        force: true,
        ..BuildOptions::default()
    };
    for project in [&original, &copied] {
        let outcome = flow::build(project, &options, &mut |_| {});
        assert!(
            outcome.ok,
            "{demo}: {}: {:?}",
            project.root.display(),
            outcome.messages
        );
    }
    assert_eq!(
        fs::read(original.outputs().config).unwrap(),
        fs::read(copied.outputs().config).unwrap(),
        "{demo}: copied and original top.config differ"
    );
}

#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
fn uart_hello_build_matches_original() {
    same_build("ulx3s/uart_hello");
}

#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
fn riscv_shell_build_matches_original() {
    same_build("gallery/riscv_shell");
}
