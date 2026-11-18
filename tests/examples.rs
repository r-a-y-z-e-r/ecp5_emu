//! Every example is an explicit project; legacy inputs also match the shell flow.
use ecp5_emu::flow::Project;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn matching(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry.path().is_file()
                && !entry.file_name().to_string_lossy().starts_with('.')
                && entry
                    .path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| extensions.contains(&s))
        })
        .map(|entry| PathBuf::from(entry.file_name()))
        .collect();
    paths.sort();
    paths
}

fn design_folders(root: &Path, designs: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        // Simulations, toolchain reproducers, firmware, vendored SERV sources
        // and generated outputs are support folders, not example designs.
        if matches!(
            entry.file_name().to_str(),
            Some("sim" | "repro" | "firmware" | "serv" | "build" | "expected")
        ) {
            continue;
        }
        let path = entry.path();
        if path.join("ecp5.toml").is_file() || !matching(&path, &["v", "sv"]).is_empty() {
            designs.push(path.clone());
        }
        design_folders(&path, designs);
    }
}

fn assert_hdl_manifest_inputs(project: &Project, manifest: &Path) {
    #[derive(serde::Deserialize)]
    struct Manifest {
        sources: Sources,
        #[serde(default)]
        memory: Memory,
    }
    #[derive(serde::Deserialize)]
    struct Sources {
        hdl: Vec<PathBuf>,
        constraints: Option<PathBuf>,
    }
    #[derive(Default, serde::Deserialize)]
    struct Memory {
        images: Option<Vec<PathBuf>>,
    }

    let expected: Manifest = toml::from_str(&fs::read_to_string(manifest).unwrap()).unwrap();
    let label = manifest.display();
    assert_eq!(project.hdl_roots(), expected.sources.hdl, "{label}");
    let constraints = expected.sources.constraints.unwrap_or_else(|| {
        let paths = matching(&project.root, &["lpf"]);
        assert_eq!(
            paths.len(),
            1,
            "{label}: default constraints must be unique"
        );
        paths[0].clone()
    });
    assert_eq!(project.constraints, constraints, "{label}");
    let images = expected.memory.images.unwrap_or_else(|| {
        let list = project.root.join("ecpbram.txt");
        if list.is_file() {
            fs::read_to_string(list)
                .unwrap()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(PathBuf::from)
                .collect()
        } else {
            Vec::new()
        }
    });
    assert_eq!(project.memory_images, images, "{label}");
}

#[test]
fn every_example_opens_and_ulx3s_inputs_match_build_script() {
    let repo = repo();
    for category in ["ulx3s", "gallery"] {
        let mut designs = Vec::new();
        design_folders(&repo.join("examples").join(category), &mut designs);
        designs.sort();
        assert!(!designs.is_empty(), "no {category} examples found");
        // Keep the nested variants covered even if discovery is later changed.
        if category == "ulx3s" {
            for variant in ["header_probe/driven", "switches/pullup"] {
                assert!(designs.contains(&repo.join("examples/ulx3s").join(variant)));
            }
        }
        for design in designs {
            let manifest = design.join("ecp5.toml");
            assert!(manifest.is_file(), "missing {}", manifest.display());
            // Both the folder and the manifest are accepted project entry points.
            let project = Project::open(&design)
                .unwrap_or_else(|error| panic!("{}: {error:#}", design.display()));
            let explicit = Project::open(&manifest)
                .unwrap_or_else(|error| panic!("{}: {error:#}", manifest.display()));
            assert_eq!(project.root, explicit.root);
            assert_eq!(project.file.as_ref(), Some(&manifest));
            // Ordered HDL roots opt into the manifest's inputs and defaults;
            // build.sh's legacy *.v-only oracle does not describe this flow.
            if project.hdl.is_some() {
                assert_hdl_manifest_inputs(&project, &manifest);
                continue;
            }
            if category != "ulx3s" {
                continue;
            }
            // build.sh copies *.v and top.lpf, reads Verilog in sorted order,
            // and uses the default device and seed without a timing override.
            let label = design.display();
            assert_eq!(project.verilog, matching(&design, &["v"]), "{label}");
            assert_eq!(project.constraints, Path::new("top.lpf"), "{label}");
            assert_eq!(project.top, "top", "{label}");
            assert_eq!(project.device.part, "LFE5U-85F", "{label}");
            assert_eq!(project.device.package, "CABGA381", "{label}");
            assert_eq!(project.device.speed, None, "{label}");
            assert_eq!(project.seed, 1, "{label}");
            assert!(!project.allow_timing_failure, "{label}");
            let image_list = design.join("ecpbram.txt");
            let images: Vec<PathBuf> = if image_list.is_file() {
                // Match grep -v '^#' followed by shell word splitting in
                // build.sh and swap.sh, retaining the image/placeholder order.
                fs::read_to_string(image_list)
                    .unwrap()
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .flat_map(str::split_whitespace)
                    .map(PathBuf::from)
                    .collect()
            } else {
                Vec::new()
            };
            assert_eq!(project.memory_images, images, "{label}");
            // All other *.hex files are copied as synthesis inputs, including
            // picorv32_flash's boot lanes, which build.sh does not swap.
            let files: Vec<_> = matching(&design, &["hex"])
                .into_iter()
                .filter(|path| !images.contains(path))
                .collect();
            assert_eq!(project.files, files, "{label}");
        }
    }
}

#[cfg(unix)]
mod real {
    use super::*;
    use ecp5_emu::flow::{self, BuildOptions};
    use std::{
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "examples test {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn copy_design(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let to = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_design(&entry.path(), &to);
            } else {
                // Dereference shared HDL symlinks before moving the design;
                // driven/ must keep its own wrapper and its parent's helpers.
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }

    // Same independent-oracle pattern as tests/flow.rs: copy the project into
    // a path with spaces, run both builds concurrently, compare config bytes.
    fn real_comparison(name: &str) {
        let folder = Folder::new();
        let repo = repo();
        let design = folder.0.join("real project");
        copy_design(&repo.join("examples/ulx3s").join(name), &design);
        let project = Project::open(&design).unwrap();
        if project.hdl.is_some() {
            eprintln!("{name}: skipping build.sh comparison for sources.hdl");
            return;
        }
        let script_out = folder.0.join("shell build");
        let script = {
            let (script, design, out) = (
                repo.join("examples/ulx3s/build.sh"),
                design.clone(),
                script_out.clone(),
            );
            thread::spawn(move || Command::new(script).args([&design, &out]).output().unwrap())
        };
        let outcome = flow::build(&project, &BuildOptions::default(), &mut |_| {});
        // Always join before checking either result, so cleanup cannot race a
        // still-running shell build when the flow reports a failure.
        let output = script.join().unwrap();
        assert!(outcome.ok, "{name}: {:#?}", outcome.messages);
        assert!(
            output.status.success(),
            "{name}: build.sh failed ({})\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            fs::read(&outcome.outputs.config).unwrap()
                == fs::read(script_out.join("top.config")).unwrap(),
            "top.config differs from build.sh for {name}"
        );
    }

    macro_rules! real_examples {
        ($($test:ident => $design:literal),+ $(,)?) => {
            $(
                #[test]
                #[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
                fn $test() {
                    real_comparison($design);
                }
            )+
        };
    }

    real_examples! {
        adc_read => "adc_read",
        audio_tone => "audio_tone",
        blinky_sv => "blinky_sv",
        edid_read => "edid_read",
        flash_dump => "flash_dump",
        hdmi_bars => "hdmi_bars",
        hdmi_moving => "hdmi_moving",
        header_probe => "header_probe",
        header_probe_driven => "header_probe/driven",
        oled_pattern => "oled_pattern",
        picorv32_flash => "picorv32_flash",
        picorv32_sdram => "picorv32_sdram",
        rtc_read => "rtc_read",
        sd_read => "sd_read",
        sdram_test => "sdram_test",
        switches => "switches",
        switches_pullup => "switches/pullup",
    }
}
