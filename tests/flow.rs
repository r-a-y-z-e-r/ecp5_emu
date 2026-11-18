//! Project builds with isolated tool stubs and optional real-tool comparisons.
#![cfg(unix)]
use ecp5_emu::flow::{
    self, BuildOptions, Event, Outcome, Project, Severity, Step, StepStatus, SvFrontend, Tools,
};
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "../src/command_flow.rs"]
mod command_flow;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "flow test {} {}",
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
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}
struct Fixture {
    folder: Folder,
    project: Project,
    options: BuildOptions,
    calls: PathBuf,
}
impl Fixture {
    fn new(memory: bool) -> Self {
        let folder = Folder::new();
        let root = folder.0.join("project with spaces");
        write(&root.join("top.v"), "module top; endmodule\n");
        write(&root.join("top.lpf"), "# constraints\n");
        write(&root.join("message.hex"), "12\n34\n");
        if memory {
            write(&root.join("fw.hex"), "01\n02\n03\n04\n");
            write(&root.join("ecpbram.txt"), "# swapped firmware\nfw.hex\n");
        }
        let project = Project::open(&root).unwrap();
        let stubs = folder.0.join("stub tools");
        fs::create_dir_all(&stubs).unwrap();
        let calls = folder.0.join("calls");
        let script = format!(
            "#!/bin/sh\nset -eu\nstubdir={}\ncalls={}\n{}",
            quote(&stubs),
            quote(&calls),
            STUB.replace(r"\t", "\t")
        );
        for name in [
            "yosys",
            "nextpnr-ecp5",
            "ecppack",
            "ecpbram",
            "openFPGALoader",
        ] {
            let path = stubs.join(name);
            write(&path, &script);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let options = BuildOptions {
            tools: Tools {
                yosys: stubs.join("yosys"),
                nextpnr: stubs.join("nextpnr-ecp5"),
                ecppack: stubs.join("ecppack"),
                ecpbram: stubs.join("ecpbram"),
                openfpgaloader: stubs.join("openFPGALoader"),
            },
            ..BuildOptions::default()
        };
        Self {
            folder,
            project,
            options,
            calls,
        }
    }
    fn marker(&self, kind: &str, tool: &str) {
        write(&self.folder.0.join("stub tools").join(kind), tool);
    }
    fn clear(&self, kind: &str) {
        fs::remove_file(self.folder.0.join("stub tools").join(kind)).unwrap();
    }
    fn build(&self) -> Outcome {
        let o = flow::build(&self.project, &self.options, &mut |_| {});
        assert!(o.ok, "{:#?}", o.messages);
        o
    }
    fn calls(&self) -> Vec<(String, String, Vec<String>)> {
        let data = fs::read_to_string(&self.calls).unwrap();
        let mut lines = data.lines();
        let mut result = Vec::new();
        while let Some(tool) = lines.next() {
            let cwd = lines.next().unwrap().to_string();
            let n: usize = lines.next().unwrap().parse().unwrap();
            let args = lines.by_ref().take(n).map(str::to_string).collect();
            assert_eq!(lines.next(), Some("END"));
            result.push((tool.into(), cwd, args));
        }
        result
    }
}
const STUB: &str = r#"
tool=$(basename "$0")
printf '%s\n' "$tool" "$PWD" "$#" "$@" 'END' >> "$calls"
if [ "${1:-}" = '-V' ]; then echo 'Yosys 0.70+post (git sha1 stub)'; exit 0; fi
if [ "${1:-}" = '--version' ]; then echo '"nextpnr-ecp5" -- Next Generation Place and Route (Version nextpnr-0.11.1-77-g6b438a1c)' >&2; exit 0; fi
# Per-fixture controls avoid changing this test process's global environment.
FLOW_FAIL=$(cat "$stubdir/fail" 2>/dev/null || :)
FLOW_SLEEP=$(cat "$stubdir/sleep" 2>/dev/null || :)
if [ "$FLOW_SLEEP" = "$tool" ]; then echo 'waiting for cancellation'; sleep 10; fi
if [ "$FLOW_FAIL" = "$tool" ]; then
  case "$tool" in
    yosys) echo 'top.v:4: ERROR: syntax error, unexpected TOK_ASSIGN' >&2;;
    nextpnr-ecp5) echo 'ERROR: unexpected input following LOCATE clause (on line 2)' >&2; echo "ERROR: Failed to parse LPF file 'top.lpf'" >&2;;
    ecppack) echo 'Failed to open input file' >&2;;
    openFPGALoader) echo 'unable to open ftdi device' >&2;;
    *) echo 'ERROR: stub failed' >&2;;
  esac
  exit 1
fi
case "$tool" in
  yosys)
    log=''; script=''
    while [ "$#" -gt 0 ]; do case "$1" in -l) log=$2; shift;; -p) script=$2; shift;; esac; shift; done
    json=${script##* -json }
    # The netlist depends on the sources, as yosys's does; with a
    # `same-netlist` marker it does not (a comment edited, say).
    if [ -e "$stubdir/same-netlist" ]; then
      printf '%s\n' '{"stub":"netlist"}' > "$json"
    else
      cat ./*.v ./*.hex 2>/dev/null | cksum > "$json"
    fi
    cat > "$log" <<'LOG'
=== top ===
  5 CCU2C
  8 TRELLIS_FF
LOG
    echo 'top.v:4: Warning: Identifier `\undeclared_thing'"'"' is implicitly declared.'
    echo 'Warning: Wire top.\undeclared_thing is used but has no driver.' >&2
    echo 'Warning: Wire top.\undeclared_thing is used but has no driver.' >&2
    ;;
  nextpnr-ecp5)
    while [ "$#" -gt 0 ]; do
      case "$1" in --textcfg) config=$2; shift;; --write) routed=$2; shift;; -l) log=$2; shift;; esac
      shift
    done
    # The configuration depends on the netlist and the constraints.
    cat top.json ./*.lpf | cksum > "$config"; echo '{}' > "$routed"
    cat > "$log" <<'LOG'
Info: Device utilisation:
Info: 	          TRELLIS_IO:      16/    365     4%
Info: 	                DCCA:       1/     56     1%
Info: 	              DP16KD:       0/    208     0%
Info: 	          MULT18X18D:       0/    156     0%
Info: 	              ALU54B:       0/     78     0%
Info: 	             EHXPLLL:       0/      4     0%
Info: 	          TRELLIS_FF:     775/  83640     0%
Info: 	        TRELLIS_COMB:   10859/  83640    12%

Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 448.83 MHz (PASS at 25.00 MHz)
Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)
Info: Program finished normally.
LOG
    cat "$log" >&2
    ;;
  ecppack) cp "$2" "$3";;
  ecpbram)
    mode=$1
    while [ "$#" -gt 0 ]; do
      case "$1" in -g) gen=$2;; -w) width=$2;; -d) depth=$2;; -s) seed=$2;; -i) input=$2;; -o) output=$2;; -t) image=$2;; esac
      shift 2
    done
    if [ "$mode" = '-g' ]; then
      awk -v w="$width" -v d="$depth" -v s="$seed" 'BEGIN { for(i=0;i<d;i++){ for(j=0;j<w/4;j++)printf "%x",s; print "" } }' > "$gen"
    else
      cat "$input" "$image" > "$output"
    fi
    ;;
  openFPGALoader) echo 'SRAM loaded';;
esac
"#;
fn statuses(o: &Outcome) -> Vec<&'static str> {
    o.steps
        .iter()
        .map(|(_, s)| match s {
            StepStatus::Ran { .. } => "ran",
            StepStatus::Skipped => "skipped",
            StepStatus::Failed => "failed",
            StepStatus::NotRun => "not run",
        })
        .collect()
}

#[test]
fn yosys_tmpdir_is_per_child() {
    if let Ok(case) = env::var("ECP5_TEST_TMPDIR_CASE") {
        let root = PathBuf::from(env::var_os("ECP5_TEST_PROJECT").unwrap());
        let mut project = Project::open(&root).unwrap();
        project
            .set_build_dir(env::var_os("ECP5_TEST_BUILD_DIR").unwrap())
            .unwrap();
        let original = env::var_os("TMPDIR").unwrap();
        let records = root.join("child-tmpdirs");
        let expected = PathBuf::from(env::var_os("ECP5_TEST_TMPDIR_EXPECTED").unwrap());
        let mut redirects = Vec::new();
        let mut options = BuildOptions::default();
        let outcome = flow::build(&project, &options, &mut |event| {
            if let Event::Output { step, line } = event {
                if line.starts_with("TMPDIR redirected to ") {
                    assert_eq!(step, Step::Synthesis);
                    redirects.push(line);
                }
            }
        });
        assert_eq!(env::var_os("TMPDIR"), Some(original.clone()));
        if case == "error" {
            assert!(!outcome.ok);
            assert_eq!(
                statuses(&outcome),
                ["failed", "not run", "not run", "not run"]
            );
            assert!(outcome.messages.iter().any(|m| m.step == Step::Synthesis
                && m.text == format!("TMPDIR ({}) contains a space, which Yosys's abc step cannot handle; set TMPDIR to a folder without spaces", Path::new(&original).display())));
            assert!(
                !records.exists(),
                "a synthesis tool ran before TMPDIR validation"
            );
            return;
        }
        assert!(outcome.ok, "{:?}", outcome.messages);
        assert_eq!(statuses(&outcome), ["ran"; 4]);
        assert_eq!(redirects.len(), usize::from(case != "clean"));
        let records = fs::read_to_string(&records).unwrap();
        let mut lines = records.lines();
        let mut yosys = 0;
        while let Some(tool) = lines.next() {
            let tmpdir = lines.next().unwrap();
            if tool == "yosys" {
                yosys += 1;
                assert_eq!(
                    fs::canonicalize(tmpdir).unwrap(),
                    fs::canonicalize(&expected).unwrap()
                );
                if case != "clean" {
                    assert_eq!(
                        redirects,
                        [format!(
                            "TMPDIR redirected to {tmpdir} for Yosys's abc step"
                        )]
                    );
                }
                assert!(!tmpdir.chars().any(char::is_whitespace), "{tmpdir}");
                assert!(Path::new(tmpdir).is_dir());
            } else {
                assert_eq!(Path::new(tmpdir), Path::new(&original), "{tool}");
            }
        }
        assert_eq!(yosys, 1);
        if case == "build" {
            assert!(!expected.join("stale").exists());
            write(&expected.join("stale"), "clean this on the next synthesis");
            options.force = true;
            assert!(flow::build(&project, &options, &mut |_| {}).ok);
            assert!(!expected.join("stale").exists());
        } else if case != "clean" {
            assert!(expected.join("stale").exists(), "shared cache was cleared");
        }
        return;
    }

    // A fixed space-free base keeps these cases independent of the test runner's
    // own TMPDIR, HOME and checkout path. No process-wide environment is changed.
    let clean = Folder(PathBuf::from("/tmp").join(format!(
        "ecp5-flow-tmpdir-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    for case in ["build", "cache", "suite", "error", "clean"] {
        let f = Fixture::new(true);
        let root = &f.project.root;
        let build_dir = if case == "build" {
            clean.0.join("build")
        } else {
            root.join("build")
        };
        let home = if case == "error" {
            f.folder.0.join("home with spaces")
        } else {
            clean.0.join("home")
        };
        let cache = home.join(if cfg!(target_os = "macos") {
            "Library/Caches/ecp5_emu/tmp"
        } else {
            ".cache/ecp5_emu/tmp"
        });
        let temp = if case == "clean" {
            clean.0.join("original-tmpdir")
        } else {
            f.folder.0.join("tmp with spaces")
        };
        fs::create_dir_all(&temp).unwrap();
        let expected = match case {
            "build" => build_dir.join(".tmp"),
            "clean" => temp.clone(),
            _ => cache,
        };
        write(&expected.join("stale"), "left by an earlier build");
        for path in [
            &f.options.tools.yosys,
            &f.options.tools.nextpnr,
            &f.options.tools.ecppack,
            &f.options.tools.ecpbram,
        ] {
            let script = fs::read_to_string(path).unwrap().replace(
                "# Per-fixture controls",
                &format!(
                    "printf '%s\\n' \"$tool\" \"${{TMPDIR-unset}}\" >> {}\n# Per-fixture controls",
                    quote(&root.join("child-tmpdirs"))
                ),
            );
            write(path, script);
        }
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "yosys_tmpdir_is_per_child", "--nocapture"])
            .env("ECP5_TEST_TMPDIR_CASE", case)
            .env("ECP5_TEST_PROJECT", root)
            .env("ECP5_TEST_BUILD_DIR", &build_dir)
            .env("ECP5_TEST_TMPDIR_EXPECTED", &expected)
            .env("TMPDIR", &temp)
            .env("HOME", home)
            .env_remove("XDG_CACHE_HOME")
            .env("ECP5_YOSYS", &f.options.tools.yosys)
            .env("ECP5_NEXTPNR", &f.options.tools.nextpnr)
            .env("ECP5_ECPPACK", &f.options.tools.ecppack)
            .env("ECP5_ECPBRAM", &f.options.tools.ecpbram);
        if case == "suite" {
            let suite = f.folder.0.join("suite with spaces");
            write(
                &root.join("ecp5.toml"),
                "[synthesis]\nsv_frontend='slang'\n",
            );
            write(&suite.join("share/yosys/plugins/slang.so"), "stub plugin");
            let yosys = suite.join("bin/yosys");
            write(&yosys, fs::read(&f.options.tools.yosys).unwrap());
            fs::set_permissions(&yosys, fs::Permissions::from_mode(0o755)).unwrap();
            command.env("ECP5_OSS_CAD_SUITE", suite);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn ordered_hdl_scripts_frontend_and_cache() {
    let mut f = Fixture::new(false);
    for name in ["leaf.v", "types.SV", "top.sv", "tail.V"] {
        write(&f.project.root.join(name), "module stub; endmodule\n");
    }
    write(
        &f.project.root.join("include/settings.svh"),
        "`define WIDTH 8\n",
    );
    fs::create_dir_all(f.project.root.join("shared")).unwrap();
    let manifest = f.project.root.join("ecp5.toml");
    let sources = "[sources]\nhdl=['leaf.v','types.SV','top.sv','tail.V']\ninclude_dirs=['include','shared']\nfiles=['include/settings.svh']\n";
    write(&manifest, sources);
    f.project = Project::open(&f.project.root).unwrap();
    let first = f.build();
    let calls = f.calls();
    assert_eq!(calls[2].0, "yosys");
    assert_eq!(calls[2].2, [
        "-l", "yosys.log", "-p",
        "read_verilog -Iinclude -Ishared leaf.v; read_verilog -sv -Iinclude -Ishared types.SV top.sv; read_verilog -Iinclude -Ishared tail.V; synth_ecp5 -top top -json top.json"
    ]);
    assert_eq!(
        fs::read(
            f.project
                .build_dir
                .join(".hdl-synthesis/include/settings.svh")
        )
        .unwrap(),
        b"`define WIDTH 8\n"
    );
    let report = first.report.unwrap();
    let frontend = report.frontend.as_ref().unwrap();
    assert_eq!(frontend.requested, SvFrontend::Auto);
    assert_eq!(frontend.resolved, flow::report::FrontendKind::Builtin);
    assert_eq!(
        frontend.reason,
        "auto: Yosys built-in reader (read_verilog -sv)"
    );
    assert_eq!(
        frontend.yosys_path,
        fs::canonicalize(&f.options.tools.yosys).unwrap()
    );
    assert_eq!(frontend.yosys_version, "Yosys 0.70+post (git sha1 stub)");
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(&first.outputs.report).unwrap()).unwrap();
    assert_eq!(json["frontend"]["requested"], "auto");
    assert_eq!(json["frontend"]["resolved"], "builtin");
    assert_eq!(
        json["frontend"]["reason"],
        "auto: Yosys built-in reader (read_verilog -sv)"
    );
    assert_eq!(
        json["frontend"]["yosys_path"],
        frontend.yosys_path.to_str().unwrap()
    );
    assert_eq!(json["frontend"]["yosys_version"], frontend.yosys_version);
    assert_eq!(statuses(&f.build()), ["skipped"; 3]);
    // Pinning builtin has the same argv/netlist but changes the frontend key.
    write(
        &manifest,
        format!("{sources}[synthesis]\nsv_frontend='builtin'\n"),
    );
    f.project = Project::open(&f.project.root).unwrap();
    let pinned = f.build();
    assert_eq!(statuses(&pinned), ["ran", "skipped", "skipped"]);
    let pinned_frontend = pinned.report.unwrap().frontend.unwrap();
    assert_eq!(pinned_frontend.requested, SvFrontend::Builtin);
    assert_eq!(
        pinned_frontend.reason,
        "builtin: pinned Yosys built-in reader"
    );
    for changed in [
        sources.replace("'leaf.v','types.SV'", "'types.SV','leaf.v'"),
        sources.replace("['include','shared']", "['shared','include']"),
    ] {
        write(&manifest, sources);
        f.project = Project::open(&f.project.root).unwrap();
        f.build();
        write(&manifest, changed);
        f.project = Project::open(&f.project.root).unwrap();
        assert_eq!(statuses(&f.build()), ["ran", "skipped", "skipped"]);
    }
    write(
        &f.project.root.join("include/settings.svh"),
        "`define WIDTH 16\n",
    );
    assert_eq!(statuses(&f.build()), ["ran", "skipped", "skipped"]);
    write(
        &manifest,
        sources.replace("files=['include/settings.svh']", "files=[]"),
    );
    f.project = Project::open(&f.project.root).unwrap();
    f.build();
    assert!(!f
        .project
        .build_dir
        .join(".hdl-synthesis/include/settings.svh")
        .exists());
}
#[test]
fn hdl_default_includes_and_single_language_runs() {
    for (roots, script) in [
        (
            "['top.sv']",
            "read_verilog -sv -I. top.sv; synth_ecp5 -top top -json top.json",
        ),
        (
            "['top.v']",
            "read_verilog -I. top.v; synth_ecp5 -top top -json top.json",
        ),
    ] {
        let mut f = Fixture::new(false);
        write(&f.project.root.join("top.sv"), "module top; endmodule\n");
        write(
            &f.project.root.join("ecp5.toml"),
            format!("[sources]\nhdl={roots}\n"),
        );
        f.project = Project::open(&f.project.root).unwrap();
        f.build();
        assert_eq!(f.calls()[2].2, ["-l", "yosys.log", "-p", script]);
    }
}
#[test]
fn hdl_keeps_the_selected_yosys_symlink() {
    let mut f = Fixture::new(false);
    write(
        &f.project.root.join("ecp5.toml"),
        "[sources]\nhdl=['top.v']",
    );
    f.project = Project::open(&f.project.root).unwrap();
    let driver = f.options.tools.yosys.with_file_name("driver");
    fs::rename(&f.options.tools.yosys, &driver).unwrap();
    std::os::unix::fs::symlink(&driver, &f.options.tools.yosys).unwrap();
    let o = f.build();
    assert_eq!(f.calls()[2].0, "yosys");
    assert_eq!(o.report.unwrap().frontend.unwrap().yosys_path, driver);
}
#[test]
fn hdl_mode_changes_cache_even_with_the_legacy_script() {
    let mut f = Fixture::new(false);
    f.marker("same-netlist", "yosys");
    f.build();
    let legacy = f
        .calls()
        .into_iter()
        .find(|c| c.0 == "yosys" && c.2.first().is_some_and(|a| a == "-l"))
        .unwrap()
        .2;
    write(
        &f.project.root.join("ecp5.toml"),
        "[sources]\nhdl=['top.v']\ninclude_dirs=[]\n",
    );
    f.project = Project::open(&f.project.root).unwrap();
    assert_eq!(statuses(&f.build()), ["ran", "skipped", "skipped"]);
    let calls = f.calls();
    let script = &calls
        .iter()
        .rev()
        .find(|c| c.0 == "yosys" && c.2.first().is_some_and(|a| a == "-l"))
        .unwrap()
        .2;
    assert_eq!(script, &legacy);
}
#[test]
fn hdl_workspace_cannot_contain_the_project() {
    let mut f = Fixture::new(false);
    let root = f.folder.0.join(".hdl-synthesis/project");
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::rename(&f.project.root, &root).unwrap();
    write(&root.join("ecp5.toml"), "[sources]\nhdl=['top.v']\n");
    f.project = Project::open(&root).unwrap();
    f.project.set_build_dir(&f.folder.0).unwrap();
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert!(!o.ok);
    assert!(o.messages[0]
        .text
        .contains("must not contain the project folder"));
    assert!(root.join("top.v").is_file());
    assert!(!f.calls.exists());
}
#[test]
fn sv_diagnostics_keep_original_source_locations() {
    let mut f = Fixture::new(false);
    fs::rename(f.project.root.join("top.v"), f.project.root.join("top.sv")).unwrap();
    write(
        &f.project.root.join("ecp5.toml"),
        "[sources]\nhdl=['top.sv']",
    );
    f.project = Project::open(&f.project.root).unwrap();
    let stub = &f.options.tools.yosys;
    write(
        stub,
        fs::read_to_string(stub)
            .unwrap()
            .replace("top.v:", "top.sv:"),
    );
    f.marker("fail", "yosys");
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert!(!o.ok);
    let m = o
        .messages
        .iter()
        .find(|m| m.severity == Severity::Error)
        .unwrap();
    assert_eq!(m.file, Some(f.project.root.join("top.sv")));
    assert_eq!(m.line, Some(4));
}
#[test]
fn cli_progress_keeps_diagnostics_on_separate_lines() {
    for memory in [false, true] {
        for verbose in [false, true] {
            let f = Fixture::new(memory);
            let out = f.folder.0.join("cli output");
            for state in ["done", "skipped", "failed"] {
                let mut command = Command::new("sh");
                // Both streams go to one pipe, preserving the binary's write order.
                command
                    .args(["-c", "exec \"$0\" \"$@\" 2>&1"])
                    .arg(env!("CARGO_BIN_EXE_ecp5_emu"))
                    .arg("build")
                    .arg(&f.project.root)
                    .arg("--out")
                    .arg(&out)
                    .env("ECP5_YOSYS", &f.options.tools.yosys)
                    .env("ECP5_NEXTPNR", &f.options.tools.nextpnr)
                    .env("ECP5_ECPPACK", &f.options.tools.ecppack)
                    .env("ECP5_ECPBRAM", &f.options.tools.ecpbram);
                if verbose {
                    command.arg("--verbose");
                }
                if state == "failed" {
                    f.marker("fail", "yosys");
                    command.arg("--force");
                }
                let output = command.output().unwrap();
                let text = String::from_utf8(output.stdout).unwrap();
                assert_eq!(output.status.success(), state != "failed", "{text}");
                assert!(output.stderr.is_empty());
                let memory_notes: Vec<_> = text
                    .lines()
                    .filter(|line| line.starts_with("Memory images (ecpbram):"))
                    .collect();
                if memory {
                    assert!(memory_notes.is_empty(), "{text}");
                } else {
                    assert_eq!(
                    memory_notes,
                    ["Memory images (ecpbram): none listed — $readmemh files are built in by Yosys"],
                    "{text}"
                );
                }
                if state == "done" {
                    assert!(text.contains("warning:"), "{text}");
                    assert!(text.contains("Frontend: auto -> builtin:"), "{text}");
                    assert!(
                        text.contains("auto: Yosys built-in reader (read_verilog -sv)"),
                        "{text}"
                    );
                    assert!(text.contains("Yosys 0.70+post (git sha1 stub)"), "{text}");
                    if !verbose {
                        println!("{text}");
                    }
                } else if state == "failed" {
                    assert!(text.contains("error:"), "{text}");
                }
                let mut completed = Vec::new();
                for line in text.lines() {
                    let lower = line.to_ascii_lowercase();
                    assert!(
                        !(line.starts_with('[')
                            && (lower.contains("warning:") || lower.contains("error:"))),
                        "diagnostic shares a progress line: {text}"
                    );
                    if let Some((prefix, status)) = line.split_once(" … ") {
                        let done = status
                            .strip_prefix("done in ")
                            .and_then(|s| s.strip_suffix(" s"))
                            .is_some_and(|s| s.parse::<f64>().is_ok());
                        if done || status == "skipped: inputs unchanged" || status == "failed" {
                            assert!(
                                match state {
                                    "done" => done,
                                    "skipped" => status == "skipped: inputs unchanged",
                                    _ => status == "failed",
                                },
                                "unexpected step result: {text}"
                            );
                            completed.push(prefix);
                        }
                    }
                }
                let expected: &[&str] = if memory {
                    &[
                        "[1/4] Synthesis (yosys)",
                        "[2/4] Place and route (nextpnr-ecp5)",
                        "[3/4] Memory images (ecpbram)",
                        "[4/4] Bitstream (ecppack)",
                    ]
                } else {
                    &[
                        "[1/3] Synthesis (yosys)",
                        "[2/3] Place and route (nextpnr-ecp5)",
                        "[3/3] Bitstream (ecppack)",
                    ]
                };
                let count = if state == "failed" { 1 } else { expected.len() };
                assert_eq!(completed, expected[..count], "{text}");
            }
        }
    }
}
#[test]
fn exact_commands_events_and_incremental_rebuilds() {
    let f = Fixture::new(false);
    let mut events = Vec::new();
    let o = flow::build(&f.project, &f.options, &mut |event| events.push(event));
    assert!(o.ok, "{:?}", o.messages);
    assert_eq!(statuses(&o), ["ran", "ran", "ran"]);
    let calls = f.calls();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["yosys", "nextpnr-ecp5", "yosys", "nextpnr-ecp5", "ecppack"]
    );
    assert_eq!(calls[0].2, ["-V"]);
    assert_eq!(calls[1].2, ["--version"]);
    assert_eq!(
        calls[2].2,
        [
            "-l",
            "yosys.log",
            "-p",
            "read_verilog top.v; synth_ecp5 -top top -json top.json"
        ]
    );
    assert_eq!(
        calls[3].2,
        [
            "--85k",
            "--package",
            "CABGA381",
            "--json",
            "top.json",
            "--lpf",
            "top.lpf",
            "--textcfg",
            "top.config",
            "--write",
            "top_routed.json",
            "--seed",
            "1",
            "-l",
            "nextpnr.log"
        ]
    );
    assert_eq!(calls[4].2, ["--compress", "top.config", "top.bit"]);
    assert!(calls.iter().all(|c| Path::new(&c.1) == f.project.build_dir));
    assert_eq!(
        events
            .iter()
            .filter_map(|e| match e {
                Event::Started { step, .. } => Some((*step, true)),
                Event::Finished { step, ok, .. } => {
                    assert!(ok);
                    Some((*step, false))
                }
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            (Step::Synthesis, true),
            (Step::Synthesis, false),
            (Step::PlaceRoute, true),
            (Step::PlaceRoute, false),
            (Step::Bitstream, true),
            (Step::Bitstream, false)
        ]
    );
    let first_output = events
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Output {
                    step: Step::Synthesis,
                    ..
                }
            )
        })
        .unwrap();
    let synth_finished = events
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Finished {
                    step: Step::Synthesis,
                    ..
                }
            )
        })
        .unwrap();
    assert!(first_output > 0 && first_output < synth_finished);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Message(_)))
            .count(),
        2
    );
    assert_eq!(
        o.messages.iter().find(|m| m.file.is_some()).unwrap().file,
        Some(f.project.root.join("top.v"))
    );
    assert_eq!(
        o.messages
            .iter()
            .find(|m| m.text.contains("no driver"))
            .unwrap()
            .count,
        2
    );
    let report = o.report.unwrap();
    assert_eq!(report.clocks[0].achieved_mhz, 354.99);
    let mut skip_events = Vec::new();
    let o = flow::build(&f.project, &f.options, &mut |e| skip_events.push(e));
    assert_eq!(statuses(&o), ["skipped"; 3]);
    assert_eq!(o.report, Some(report));
    assert!(skip_events
        .iter()
        .all(|e| matches!(e, Event::Skipped { .. })));
    write(
        &f.project.root.join("top.v"),
        "module top; endmodule // changed\n",
    );
    assert_eq!(statuses(&f.build()), ["ran"; 3]);
    write(&f.project.root.join("top.lpf"), "# edited constraints\n");
    assert_eq!(statuses(&f.build()), ["skipped", "ran", "ran"]);
    write(&f.project.root.join("message.hex"), "ab\ncd\n");
    assert_eq!(statuses(&f.build()), ["ran"; 3]);
    let mut force = f.options.clone();
    force.force = true;
    assert_eq!(
        statuses(&flow::build(&f.project, &force, &mut |_| {})),
        ["ran"; 3]
    );
    // A corrupt or missing output reruns its step; a step after it runs
    // only when what it reads came out different.
    write(&f.project.outputs().bit, "corrupt");
    assert_eq!(statuses(&f.build()), ["skipped", "skipped", "ran"]);
    fs::remove_file(f.project.outputs().json).unwrap();
    assert_eq!(statuses(&f.build()), ["ran", "skipped", "skipped"]);
    // An edit that leaves the netlist as it was (a comment) runs yosys
    // only: place and route reads the same top.json.
    f.marker("same-netlist", "");
    write(&f.project.root.join("top.v"), "module top; endmodule\n");
    assert_eq!(statuses(&f.build()), ["ran", "ran", "ran"]);
    write(
        &f.project.root.join("top.v"),
        "module top; endmodule // a comment edited\n",
    );
    assert_eq!(statuses(&f.build()), ["ran", "skipped", "skipped"]);
}
#[test]
fn memory_swap_and_parameters() {
    let f = Fixture::new(true);
    assert_eq!(statuses(&f.build()), ["ran"; 4]);
    let calls = f.calls();
    assert_eq!(calls[2].0, "ecpbram");
    assert_eq!(
        calls[2].2,
        ["-g", "placeholder_fw.hex", "-w", "8", "-d", "4", "-s", "1"]
    );
    assert!(calls[4]
        .2
        .windows(2)
        .any(|a| a == ["--textcfg", "top_placeholder.config"]));
    assert_eq!(
        calls[5].2,
        [
            "-i".to_string(),
            "top.config".into(),
            "-o".into(),
            "top.swap.config".into(),
            "-f".into(),
            "placeholder_fw.hex".into(),
            "-t".into(),
            f.project.root.join("fw.hex").display().to_string()
        ]
    );
    assert_eq!(
        fs::read(f.project.build_dir.join("fw.hex")).unwrap(),
        fs::read(f.project.root.join("fw.hex")).unwrap()
    );
    assert_eq!(statuses(&f.build()), ["skipped"; 4]);
    write(&f.project.root.join("fw.hex"), "11\n22\n33\n44\n");
    assert_eq!(statuses(&f.build()), ["skipped", "skipped", "ran", "ran"]);
    write(&f.project.root.join("fw.hex"), "1111\n2222\n3333\n4444\n");
    assert_eq!(statuses(&f.build()), ["ran"; 4]);
    write(&f.project.root.join("top.lpf"), "# changed\n");
    assert_eq!(statuses(&f.build()), ["skipped", "ran", "ran", "ran"]);
    let mut force = f.options.clone();
    force.force = true;
    assert_eq!(
        statuses(&flow::build(&f.project, &force, &mut |_| {})),
        ["ran"; 4]
    );
}
#[test]
fn hdl_memory_images_use_staged_synthesis_and_build_placeholders() {
    let mut f = Fixture::new(true);
    fs::rename(f.project.root.join("top.v"), f.project.root.join("top.sv")).unwrap();
    write(
        &f.project.root.join("ecp5.toml"),
        "[sources]\nhdl=['top.sv']\n[memory]\nimages=['fw.hex']\n",
    );
    f.project = Project::open(&f.project.root).unwrap();
    assert!(f.project.root.join("ecpbram.txt").is_file());
    let o = f.build();
    assert_eq!(statuses(&o), ["ran"; 4]);
    let calls = f.calls();
    let workspace = f.project.build_dir.join(".hdl-synthesis");
    assert_eq!(calls[2].0, "ecpbram");
    assert_eq!(Path::new(&calls[2].1), workspace);
    assert_eq!(
        calls[2].2,
        ["-g", "placeholder_fw.hex", "-w", "8", "-d", "4", "-s", "1"]
    );
    assert_eq!(calls[3].0, "yosys");
    assert_eq!(Path::new(&calls[3].1), workspace);
    assert_eq!(
        calls[3].2,
        [
            "-l",
            "yosys.log",
            "-p",
            "read_verilog -sv -I. top.sv; synth_ecp5 -top top -json top.json"
        ]
    );
    let placeholder = fs::read(f.project.build_dir.join("placeholder_fw.hex")).unwrap();
    assert_eq!(
        placeholder,
        fs::read(workspace.join("placeholder_fw.hex")).unwrap()
    );
    assert_eq!(placeholder, fs::read(workspace.join("fw.hex")).unwrap());
    assert!(o.outputs.placeholder_config.as_ref().unwrap().is_file());
    assert_eq!(calls[5].0, "ecpbram");
    assert_eq!(Path::new(&calls[5].1), f.project.build_dir);
    assert_eq!(
        calls[5].2,
        [
            "-i".to_string(),
            "top.config".into(),
            "-o".into(),
            "top.swap.config".into(),
            "-f".into(),
            "placeholder_fw.hex".into(),
            "-t".into(),
            f.project.root.join("fw.hex").display().to_string(),
        ]
    );
    let image = fs::read(f.project.root.join("fw.hex")).unwrap();
    assert_eq!(fs::read(f.project.build_dir.join("fw.hex")).unwrap(), image);
    assert!(fs::read(&o.outputs.config).unwrap().ends_with(&image));
    assert_eq!(
        fs::read(&o.outputs.bit).unwrap(),
        fs::read(&o.outputs.config).unwrap()
    );
    assert_eq!(statuses(&f.build()), ["skipped"; 4]);
    write(&f.project.root.join("fw.hex"), "aa\nbb\ncc\ndd\n");
    let swapped = f.build();
    assert_eq!(statuses(&swapped), ["skipped", "skipped", "ran", "ran"]);
    assert_eq!(
        fs::read(f.project.build_dir.join("placeholder_fw.hex")).unwrap(),
        placeholder
    );
    assert!(fs::read(&swapped.outputs.config)
        .unwrap()
        .ends_with(b"aa\nbb\ncc\ndd\n"));
}
#[test]
fn relative_paths_with_spaces_and_device_options() {
    let mut f = Fixture::new(false);
    write(
        &f.project.root.join("rtl/cpu core.v"),
        "module cpu; endmodule\n",
    );
    write(&f.project.root.join("ecp5.toml"), "[project]\ntop='cpu'\n[sources]\nverilog=['rtl/cpu core.v', 'top.v']\n[device]\npart='LFE5UM-25F'\nspeed=7\n[build]\nseed=42\nallow_timing_failure=true\n");
    f.project = Project::open(&f.project.root).unwrap();
    f.build();
    let calls = f.calls();
    assert_eq!(
        calls[2].2[3],
        "read_verilog \"rtl/cpu core.v\" top.v; synth_ecp5 -top cpu -json top.json"
    );
    assert_eq!(calls[3].2[0], "--um-25k");
    assert!(calls[3].2.windows(2).any(|s| s == ["--speed", "7"]));
    assert!(calls[3].2.windows(2).any(|s| s == ["--seed", "42"]));
    assert!(calls[3].2.contains(&"--timing-allow-fail".into()));
    assert_eq!(
        fs::read(f.project.build_dir.join("rtl/cpu core.v")).unwrap(),
        fs::read(f.project.root.join("rtl/cpu core.v")).unwrap()
    );
}
#[test]
fn failure_retry_fallback_and_missing_tool() {
    let mut f = Fixture::new(false);
    f.marker("fail", "yosys");
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert!(!o.ok);
    assert_eq!(statuses(&o), ["failed", "not run", "not run"]);
    assert_eq!(o.messages[0].severity, Severity::Error);
    assert_eq!(o.messages[0].file, Some(f.project.root.join("top.v")));
    assert_eq!(o.messages[0].line, Some(4));
    f.clear("fail");
    assert_eq!(statuses(&f.build()), ["ran"; 3]);
    f.marker("fail", "nextpnr-ecp5");
    write(&f.project.root.join("top.lpf"), "changed");
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert_eq!(statuses(&o), ["skipped", "failed", "not run"]);
    assert_eq!(o.messages[0].file, Some(f.project.root.join("top.lpf")));
    assert_eq!(o.messages[0].line, Some(2));
    f.clear("fail");
    assert_eq!(statuses(&f.build()), ["skipped", "ran", "ran"]);
    f.marker("fail", "ecppack");
    fs::remove_file(f.project.outputs().bit).unwrap();
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert!(o
        .messages
        .iter()
        .any(|m| m.text == "ecppack failed (exit 1): Failed to open input file"));
    assert_eq!(statuses(&o), ["skipped", "skipped", "failed"]);
    f.clear("fail");
    assert_eq!(statuses(&f.build()), ["skipped", "skipped", "ran"]);
    f.options.tools.yosys = f.folder.0.join("absent yosys");
    let o = flow::build(&f.project, &f.options, &mut |_| {});
    assert!(!o.ok);
    let expected = format!(
        "yosys not found at {}: install the open FPGA toolchain (yosys, nextpnr-ecp5, prjtrellis: ecppack/ecpbram) or set ECP5_YOSYS",
        f.options.tools.yosys.display()
    );
    assert!(o
        .messages
        .iter()
        .any(|m| flow::tools::is_missing_tool(m) && m.text == expected));
}
/// Child processes isolate environment overrides and an empty PATH from other tests.
#[test]
fn missing_tools_report_attempted_executables() {
    if let Ok(expected) = env::var("ECP5_TEST_MISSING_TOOL") {
        let project = Project::open(env::var_os("ECP5_TEST_PROJECT").unwrap()).unwrap();
        let tools = Tools::from_env();
        match env::var("ECP5_TEST_TOOL").unwrap().as_str() {
            "openFPGALoader" => {
                let error =
                    flow::board::load(&project.root.join("top.bit"), &tools, None, &mut |_| {})
                        .unwrap_err();
                assert_eq!(error.to_string(), expected);
                return;
            }
            "image reader" => {
                let path = ecp5_emu::image::reader_path(None);
                let explicit =
                    (env::var("ECP5_TEST_CASE").unwrap() == "explicit").then_some(path.as_path());
                let error = ecp5_emu::image::ConfigurationImage::load(
                    &project.root.join("top.bit"),
                    &project.root,
                    explicit,
                )
                .err()
                .unwrap();
                assert_eq!(error.to_string(), expected);
                return;
            }
            _ => {}
        }
        let mut events = Vec::new();
        let options = BuildOptions {
            tools,
            ..BuildOptions::default()
        };
        let outcome = flow::build(&project, &options, &mut |e| events.push(e));
        assert!(!outcome.ok);
        let error = outcome
            .messages
            .iter()
            .find(|m| m.severity == Severity::Error)
            .unwrap();
        assert_eq!(error.text, expected);
        assert!(flow::tools::is_missing_tool(error));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Message(m) if flow::tools::is_missing_tool(m))));
        return;
    }
    for (tool, variable) in [
        ("yosys", "ECP5_YOSYS"),
        ("nextpnr-ecp5", "ECP5_NEXTPNR"),
        ("ecppack", "ECP5_ECPPACK"),
        ("ecpbram", "ECP5_ECPBRAM"),
        ("openFPGALoader", "ECP5_OPENFPGALOADER"),
        ("image reader", "ECP5_IMAGE_READER"),
    ] {
        for case in [
            "missing",
            "not executable",
            "PATH",
            "bare override",
            "explicit",
        ] {
            // These two tools run first, so no stub needs external commands on PATH.
            if ["PATH", "bare override"].contains(&case)
                && !["yosys", "ecpbram", "openFPGALoader"].contains(&tool)
                || case == "explicit" && tool != "image reader"
            {
                continue;
            }
            let f = Fixture::new(tool == "ecpbram");
            write(&f.project.root.join("top.bit"), []);
            let path = if case == "bare override" {
                PathBuf::from(tool)
            } else {
                f.folder.0.join("bundled tools").join(tool)
            };
            if case == "not executable" {
                write(&path, "#!/bin/sh\nexit 0\n");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            let mut command = Command::new(env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "missing_tools_report_attempted_executables",
                    "--nocapture",
                ])
                .env("ECP5_TEST_PROJECT", &f.project.root)
                .env("ECP5_TEST_TOOL", tool)
                .env("ECP5_TEST_CASE", case)
                .env("ECP5_YOSYS", &f.options.tools.yosys)
                .env("ECP5_NEXTPNR", &f.options.tools.nextpnr)
                .env("ECP5_ECPPACK", &f.options.tools.ecppack)
                .env("ECP5_ECPBRAM", &f.options.tools.ecpbram);
            let expected = if case == "PATH" {
                command.env_remove(variable).env("PATH", "");
                let install = if tool == "openFPGALoader" {
                    "install it"
                } else {
                    "install the open FPGA toolchain (yosys, nextpnr-ecp5, prjtrellis: ecppack/ecpbram)"
                };
                format!("{tool} not found on PATH: {install} or set {variable}")
            } else {
                command.env(variable, &path);
                if case == "bare override" {
                    command.env("PATH", "");
                }
                if case == "explicit" {
                    format!("starting {}; build it with python3 tools/build_image_reader.py (--trellis PATH or TRELLIS_ROOT / parent of ECP5_DB; --prefix PATH or TRELLIS_PREFIX)", path.display())
                } else {
                    format!("{tool} not found at {} (from {variable}): reinstall the package or point {variable} at a working {tool}", path.display())
                }
            };
            let output = command
                .env("ECP5_TEST_MISSING_TOOL", expected)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{tool}, {case}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
#[test]
fn cancel_kills_a_sleeping_tool_promptly() {
    let mut f = Fixture::new(false);
    f.marker("sleep", "yosys");
    let cancel = Arc::new(AtomicBool::new(false));
    f.options.cancel = Some(cancel.clone());
    let mut cancel_started = None;
    let mut saw_output = false;
    let o = flow::build(&f.project, &f.options, &mut |e| {
        if let Event::Output { line, .. } = e {
            if line == "waiting for cancellation" {
                saw_output = true;
                cancel_started = Some(Instant::now());
                cancel.store(true, Ordering::Relaxed);
            }
        }
    });
    assert!(saw_output);
    assert!(o.cancelled);
    assert!(!o.ok);
    assert!(
        cancel_started.unwrap().elapsed() < Duration::from_secs(1),
        "cancellation took {:?}",
        cancel_started.unwrap().elapsed()
    );
    assert_eq!(statuses(&o), ["failed", "not run", "not run"]);
    cancel.store(false, Ordering::Relaxed);
    f.clear("sleep");
    assert_eq!(statuses(&f.build()), ["ran"; 3]);
}
/// `COMMAND --help` and `-h` answer for every command, also for run and
/// load, whose first argument is otherwise a project or a .bit/.config.
#[test]
fn every_command_has_help() {
    for command in [
        "build", "run", "load", "device", "emulate", "inspect", "decode",
    ] {
        for flag in ["--help", "-h"] {
            let out = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
                .args([command, flag])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "{command} {flag}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                text.contains(&format!("ecp5_emu {command} ")),
                "{command} {flag}: {text}"
            );
        }
    }
    let load = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .args(["load", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&load.stdout);
    assert!(
        text.contains("load PROJECT_DIR")
            && text.contains("load DESIGN.bit")
            && text.contains("never into its flash")
    );
}
#[test]
fn background_thread_and_stubbed_sram_loader() {
    let f = Fixture::new(false);
    let project = f.project.clone();
    let options = f.options.clone();
    let o = thread::spawn(move || flow::build(&project, &options, &mut |_| {}))
        .join()
        .unwrap();
    assert!(o.ok);
    let mut lines = Vec::new();
    // The board's lock lives in the fixture, not in the lab's out/.
    let lock = f.folder.0.join("board.lock");
    flow::board::load(&o.outputs.bit, &f.options.tools, Some(&lock), &mut |s| {
        lines.push(s.to_string())
    })
    .unwrap();
    assert_eq!(lines, ["SRAM loaded"]);
    let calls = f.calls();
    let args = &calls.last().unwrap().2;
    assert_eq!(
        args,
        &[
            "-b".to_string(),
            "ulx3s".into(),
            fs::canonicalize(&o.outputs.bit)
                .unwrap()
                .display()
                .to_string()
        ]
    );
    assert!(!args
        .iter()
        .any(|s| ["-f", "--write-flash", "--flash"].contains(&s.as_str())));
    f.marker("fail", "openFPGALoader");
    assert!(!lock.exists(), "the lock is left after a load");
    let held = flow::board::BoardLock::take(&lock, "another-run").unwrap();
    let busy = flow::board::load(&o.outputs.bit, &f.options.tools, Some(&lock), &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(busy.contains("board is busy: another-run"), "{busy}");
    drop(held);
    let error = flow::board::load(
        &f.project.outputs().bit,
        &f.options.tools,
        Some(&lock),
        &mut |_| {},
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("no ULX3S found on USB")
            && error.contains("US1")
            && error.contains("FT231X")
    );
}
fn copy_design(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let to = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_design(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}
/// Builds `examples/ulx3s/<name>` twice on this machine, with the flow and
/// with examples/ulx3s/build.sh (swap.sh for memory images), and requires the
/// same top.config from both. The two are kept apart on purpose: build.sh
/// makes target/reference, the oracle the emulator is checked against, so a
/// wrong command line in the flow must show here instead of producing both
/// sides of a comparison. Both run at once (place and route is single
/// threaded), which halves life_full's time.
fn real_comparison(name: &str, check_incremental: bool) {
    let folder = Folder::new();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = folder.0.join("real project");
    copy_design(&repo.join("examples/ulx3s").join(name), &design);
    let p = Project::open(&design).unwrap();
    let script_out = folder.0.join("shell build");
    let script = {
        let (script, design, out) = (
            repo.join("examples/ulx3s/build.sh"),
            design.clone(),
            script_out.clone(),
        );
        thread::spawn(move || Command::new(script).args([&design, &out]).output().unwrap())
    };
    let o = flow::build(&p, &BuildOptions::default(), &mut |e| {
        if let Event::Message(m) = e {
            if m.severity == Severity::Error {
                eprintln!("{m}");
            }
        }
    });
    assert!(o.ok, "{:?}", o.messages);
    let output = script.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let flow_config = fs::read(&o.outputs.config).unwrap();
    let shell_config = fs::read(script_out.join("top.config")).unwrap();
    assert!(
        flow_config == shell_config,
        "top.config differs from build.sh for {name}"
    );
    if check_incremental {
        let r = o.report.unwrap();
        assert_eq!(
            r.utilisation
                .iter()
                .find(|r| r.cell == "TRELLIS_FF")
                .unwrap()
                .used,
            8
        );
        assert!(r.clocks.iter().any(|c| c.net == "clk_25mhz" && c.pass));
        let again = flow::build(&p, &BuildOptions::default(), &mut |_| {});
        assert_eq!(statuses(&again), ["skipped"; 3]);
        // A comment after the module leaves yosys's netlist byte for byte as
        // it was, so place and route and ecppack are skipped.
        let source = p.root.join("top.v");
        let original = fs::read_to_string(&source).unwrap();
        write(
            &source,
            format!("{original}\n// Incremental rebuild check.\n"),
        );
        let edited = flow::build(&p, &BuildOptions::default(), &mut |_| {});
        assert!(edited.ok);
        assert_eq!(statuses(&edited), ["ran", "skipped", "skipped"]);
        // A counter that counts by two is another design: every step runs.
        write(&source, original.replace("count + 1", "count + 2"));
        let changed = flow::build(&p, &BuildOptions::default(), &mut |_| {});
        assert!(changed.ok);
        assert_eq!(statuses(&changed), ["ran"; 3]);
        assert_ne!(fs::read(&changed.outputs.config).unwrap(), shell_config);
    }
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
fn real_blinky_matches_build_script() {
    real_comparison("blinky_cycle", true);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack"]
fn real_blinky_with_spaced_tmpdir() {
    if let Some(root) = env::var_os("ECP5_TEST_REAL_TMPDIR_PROJECT") {
        let original = env::var_os("TMPDIR").unwrap();
        assert!(Path::new(&original)
            .to_string_lossy()
            .chars()
            .any(char::is_whitespace));
        let project = Project::open(root).unwrap();
        let mut redirects = 0;
        let outcome = flow::build(&project, &BuildOptions::default(), &mut |event| {
            if let Event::Output { step, line } = event {
                if line.starts_with("TMPDIR redirected to ") {
                    assert_eq!(step, Step::Synthesis);
                    redirects += 1;
                }
            }
        });
        assert!(outcome.ok, "{:?}", outcome.messages);
        assert_eq!(statuses(&outcome), ["ran"; 3]);
        assert!(outcome.outputs.bit.is_file());
        assert_eq!(redirects, 1);
        assert_eq!(env::var_os("TMPDIR"), Some(original));
        return;
    }
    let folder = Folder::new();
    let design = folder.0.join("blinky project");
    copy_design(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/blinky_cycle"),
        &design,
    );
    let temp = folder.0.join("tmp with spaces");
    fs::create_dir_all(&temp).unwrap();
    let output = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "real_blinky_with_spaced_tmpdir",
            "--ignored",
            "--nocapture",
        ])
        .env("ECP5_TEST_REAL_TMPDIR_PROJECT", &design)
        .env("TMPDIR", &temp)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and the Trellis database"]
fn real_blinky_sv_sources_and_emulation() {
    use ecp5_emu::sources::{Collected, Files};
    let folder = Folder::new();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = folder.0.join("sv project");
    copy_design(&repo.join("examples/ulx3s/blinky_sv"), &design);
    let p = Project::open(&design).unwrap();
    let o = flow::build(&p, &BuildOptions::default(), &mut |_| {});
    assert!(o.ok, "{:?}", o.messages);
    assert_eq!(
        o.report
            .as_ref()
            .unwrap()
            .frontend
            .as_ref()
            .unwrap()
            .resolved,
        flow::report::FrontendKind::Builtin
    );
    let routed: serde_json::Value =
        serde_json::from_slice(&fs::read(&o.outputs.routed_json).unwrap()).unwrap();
    let top = &routed["modules"][&p.top];
    assert!(top["cells"].as_object().unwrap().values().any(|c| {
        c["attributes"]["src"]
            .as_str()
            .is_some_and(|s| s.contains("top.sv:"))
    }));
    let files = Files::project(
        p.root.clone(),
        p.hdl_roots().iter().chain(&p.files).cloned(),
    );
    // Routed bit IDs stand in for overlay nets, exercising both source-probe
    // directions with the same index used by the window.
    let index = Collected::from_top(top, &files).build(|name| {
        top["netnames"][name]["bits"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b.as_u64().map(|n| n as u32))
            .collect()
    });
    let nets = index.resolve("top.sv", 2, 60, "led");
    assert_eq!(nets.len(), 8);
    for net in nets {
        let locations = index.net_source_choices(net);
        assert!(!locations.is_empty());
        assert!(locations
            .iter()
            .all(|l| l.span.file.as_ref() == "top.sv" && l.span.line > 0));
    }
    for cycles in [3, 8] {
        let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
            .arg("emulate")
            .arg(&o.outputs.config)
            .arg("--names")
            .arg(&o.outputs.routed_json)
            .args([
                "--startup",
                "configured",
                "--scope",
                "observed",
                "--clock",
                "net:clk_25mhz",
                "--cycles",
                &cycles.to_string(),
                "--probe",
                "net:led",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let last = text.lines().last().unwrap();
        for bit in 0..8 {
            assert!(
                last.contains(&format!("led[{bit}]={}", (cycles >> bit) & 1)),
                "{text}"
            );
        }
    }
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
fn real_memory_matches_build_script_and_swap() {
    real_comparison("picorv32_console", false);
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5 and ecppack"]
fn real_mixed_hdl_and_tracked_includes() {
    let folder = Folder::new();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = folder.0.join("mixed project");
    write(
        &design.join("top.lpf"),
        fs::read(repo.join("examples/ulx3s/blinky_cycle/top.lpf")).unwrap(),
    );
    write(
        &design.join("leaf.v"),
        "module leaf(input [7:0] a, output [7:0] y); assign y = a; endmodule\n",
    );
    write(
        &design.join("types.sv"),
        "package widths; localparam W = 8; endpackage\n",
    );
    write(&design.join("top.sv"), "`include \"settings.svh\"\nmodule top(input logic clk_25mhz, output [7:0] led);\nlogic [widths::W-1:0] count = 0;\nalways_ff @(posedge clk_25mhz) count <= count + `STEP;\nleaf child(count, led);\nendmodule\n");
    write(&design.join("include/settings.svh"), "`define STEP 1\n");
    let manifest = "[sources]\nhdl=['leaf.v','types.sv','top.sv']\ninclude_dirs=['include']\nfiles=['include/settings.svh']\n";
    write(&design.join("ecp5.toml"), manifest);
    let p = Project::open(&design).unwrap();
    let o = flow::build(&p, &BuildOptions::default(), &mut |_| {});
    assert!(o.ok, "{:?}", o.messages);
    // The header remains in the project and was staged by the previous build,
    // but removing it from files must fail rather than use stale contents.
    write(
        &design.join("ecp5.toml"),
        manifest.replace("files=['include/settings.svh']", "files=[]"),
    );
    let p = Project::open(&design).unwrap();
    let o = flow::build(&p, &BuildOptions::default(), &mut |_| {});
    assert!(!o.ok);
    assert_eq!(statuses(&o), ["failed", "not run", "not run"]);
    assert!(
        o.messages.iter().any(|m| m.severity == Severity::Error
            && m.text.contains("include file")
            && m.text.contains("sources.files")),
        "{:?}",
        o.messages
    );
}
#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack and ecpbram"]
fn real_uart_matches_build_script() {
    real_comparison("uart_hello", false);
}
/// The full chip: about 40 minutes of place and route, both builds at once.
#[test]
#[ignore = "requires the open toolchain and about 40 minutes (the full chip placed and routed twice, at once)"]
fn real_life_full_matches_build_script() {
    real_comparison("life_full", false);
}

#[test]
fn builtin_slang_hint_requires_an_sv_hdl_root() {
    for (manifest, file, token, hint) in [
        ("", "top.v", "TOK_ID", false),
        ("[sources]\nhdl=['top.v']", "top.v", "TOK_ID", false),
        (
            "[sources]\nverilog=['top.sv']",
            "top.sv",
            "TOK_IMPORT",
            false,
        ),
        (
            "[sources]\nhdl=['top.v']\nfiles=['top.sv']",
            "top.sv",
            "TOK_IMPORT",
            false,
        ),
        ("[sources]\nhdl=['top.sv']", "top.sv", "TOK_IMPORT", true),
        ("[sources]\nhdl=['top.SV']", "top.SV", "TOK_ID", true),
    ] {
        let mut f = Fixture::new(false);
        write(&f.project.root.join(file), "");
        write(&f.project.root.join("ecp5.toml"), manifest);
        f.project = Project::open(&f.project.root).unwrap();
        let original = fs::read_to_string(&f.options.tools.yosys).unwrap();
        write(
            &f.options.tools.yosys,
            original.replace(
                "tool=$(basename",
                &format!("if [ \"${{1:-}}\" != '-V' ]; then echo '{file}:2: ERROR: syntax error, unexpected {token}'; exit 1; fi\ntool=$(basename"),
            ),
        );
        let o = flow::build(&f.project, &f.options, &mut |_| {});
        assert!(!o.ok);
        let m = o
            .messages
            .iter()
            .find(|m| m.severity == Severity::Error)
            .unwrap();
        assert_eq!(m.file, Some(f.project.root.join(file)));
        assert_eq!(
            m.text.contains("set [synthesis] sv_frontend = \"slang\""),
            hint,
            "{manifest}: {m}"
        );
    }
}

/// Each case runs in a child test process so suite and GHDL environment overrides
/// cannot leak into other builds (or the test runner).
#[test]
fn suite_frontends_and_cache() {
    if let Ok(case) = env::var("ECP5_TEST_SUITE_CASE") {
        let root = PathBuf::from(env::var_os("ECP5_TEST_PROJECT").unwrap());
        let suite = PathBuf::from(env::var_os("ECP5_TEST_SUITE_ROOT").unwrap());
        let calls = PathBuf::from(env::var_os("ECP5_TEST_CALLS").unwrap());
        let options = BuildOptions::default();
        let p = Project::open(&root).unwrap();
        let o = flow::build(&p, &options, &mut |_| {});
        if ["invalid", "none", "missing-plugin"].contains(&case.as_str()) {
            assert!(!o.ok);
            assert_eq!(o.messages.len(), 1);
            assert!(!calls.exists(), "tool ran before suite validation");
            let text = &o.messages[0].text;
            assert!(text.contains("OSS CAD Suite"), "{text}");
            if case == "invalid" {
                assert!(text.contains("invalid ECP5_OSS_CAD_SUITE root"));
            }
            if case == "none" {
                assert!(
                    text.contains("tried")
                        && text.contains("ECP5_OSS_CAD_SUITE")
                        && text.contains("slang")
                );
            }
            if case == "missing-plugin" {
                assert!(text.contains("slang.so"));
            }
            return;
        }
        if case == "failure" || case == "hint" {
            assert!(!o.ok);
            let diagnostics: Vec<_> = o.messages.iter().filter(|m| m.file.is_some()).collect();
            assert_eq!(diagnostics.len(), 1, "{:?}", o.messages);
            let m = diagnostics[0];
            assert_eq!(m.file, Some(root.join("top.sv")));
            assert_eq!(m.line, Some(2));
            if case == "hint" {
                assert!(m.text.contains(
                    "set [synthesis] sv_frontend = \"slang\" in ecp5.toml (needs OSS CAD Suite)"
                ));
            } else {
                assert!(m
                    .text
                    .contains("expected expression\n  assign led = ;\n               ^"));
                assert!(!m.text.contains("sv_frontend"));
            }
            let calls = fs::read_to_string(calls).unwrap();
            assert_eq!(
                calls.lines().filter(|s| *s == "-p").count(),
                1,
                "compile retried: {calls}"
            );
            return;
        }
        assert!(o.ok, "{:?}", o.messages);
        let report = o.report.unwrap();
        if case == "builtin-suite" {
            let frontend = report.frontend.unwrap();
            assert_eq!(frontend.resolved, flow::report::FrontendKind::Builtin);
            assert!(frontend.suite_root.is_none() && frontend.plugin.is_none());
            assert_eq!(
                frontend.yosys_path,
                fs::canonicalize(suite.join("bin/yosys")).unwrap()
            );
            let calls = fs::read_to_string(calls).unwrap();
            assert!(!calls.lines().any(|line| line == "-m"));
            assert_eq!(calls.lines().filter(|line| *line == "-p").count(), 1);
            return;
        }
        let frontend = report.frontend.as_ref().unwrap();
        assert_eq!(frontend.suite_root, Some(fs::canonicalize(&suite).unwrap()));
        assert_eq!(frontend.suite_source.as_deref(), Some("ECP5_OSS_CAD_SUITE"));
        assert_eq!(
            frontend.yosys_path,
            fs::canonicalize(suite.join("bin/yosys")).unwrap()
        );
        assert!(frontend.yosys_version.contains("Yosys 0.70"));
        let plugin = if case == "vhdl" || case == "prefix" {
            "ghdl"
        } else {
            "slang"
        };
        assert_eq!(frontend.plugin.as_deref(), Some(plugin));
        assert_eq!(
            frontend.plugin_identity.as_ref().unwrap().path,
            PathBuf::from(format!("share/yosys/plugins/{plugin}.so"))
        );
        assert_eq!(frontend.plugin_identity.as_ref().unwrap().sha256.len(), 64);
        let prefixes = fs::read_to_string(suite.join("child-prefixes")).unwrap();
        let expected = if case == "prefix" {
            PathBuf::from("/already/set/ghdl")
        } else {
            suite.join("lib/ghdl")
        };
        assert_eq!(frontend.ghdl_prefix, Some(expected.clone()));
        assert!(prefixes
            .lines()
            .any(|line| line == expected.to_string_lossy()));
        let downstream =
            fs::read_to_string(root.parent().unwrap().join("downstream-prefixes")).unwrap();
        assert!(
            downstream.lines().all(|line| line
                == if case == "prefix" {
                    "/already/set/ghdl"
                } else {
                    "unset"
                }),
            "{downstream}"
        );
        assert_eq!(
            env::var_os("GHDL_PREFIX"),
            if case == "prefix" {
                Some("/already/set/ghdl".into())
            } else {
                None
            }
        );
        let calls_text = fs::read_to_string(&calls).unwrap();
        if plugin == "ghdl" {
            assert!(
                calls_text.contains(
                    "ghdl --std=08 types.vhd top.vhd -e top; synth_ecp5 -top top -json top.json"
                ),
                "{calls_text}"
            );
            assert_eq!(frontend.vhdl_standard.as_deref(), Some("2008"));
            assert_eq!(report.notes, [flow::report::VHDL_SOURCE_LIMIT]);
        } else {
            assert!(calls_text.contains("read_slang --top top -I. -Iinclude types.sv leaf.v top.sv; synth_ecp5 -top top -json top.json"), "{calls_text}");
            assert_eq!(frontend.resolved, flow::report::FrontendKind::Slang);
            assert!(report.notes.is_empty());
        }
        assert_eq!(
            serde_json::from_slice::<flow::Report>(&fs::read(p.outputs().report).unwrap()).unwrap(),
            report
        );
        assert_eq!(
            statuses(&flow::build(&p, &options, &mut |_| {})),
            ["skipped"; 3]
        );
        write(
            &suite.join(format!("share/yosys/plugins/{plugin}.so")),
            "changed plugin bytes",
        );
        assert_eq!(
            statuses(&flow::build(&p, &options, &mut |_| {})),
            ["ran", "skipped", "skipped"]
        );
        let alternate = suite.with_file_name("alternate suite");
        // Changing the root with the same plugin bytes must also invalidate synthesis.
        copy_design(&suite, &alternate);
        let output = Command::new(env::current_exe().unwrap())
            .args(["--exact", "suite_cache_new_root"])
            .env("ECP5_TEST_PROJECT", &root)
            .env("ECP5_OSS_CAD_SUITE", &alternate)
            .env(
                "PATH",
                env::join_paths([
                    options.tools.yosys.parent().unwrap(),
                    Path::new("/usr/bin"),
                    Path::new("/bin"),
                ])
                .unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    for case in [
        "builtin-suite",
        "slang",
        "vhdl",
        "prefix",
        "invalid",
        "none",
        "missing-plugin",
        "failure",
        "hint",
    ] {
        let f = Fixture::new(false);
        let root = &f.project.root;
        let suite = f.folder.0.join("suite with spaces");
        for plugin in ["slang", "ghdl"] {
            write(
                &suite.join(format!("share/yosys/plugins/{plugin}.so")),
                plugin,
            );
        }
        for name in ["types.sv", "top.sv", "leaf.v", "types.vhd", "top.vhd"] {
            write(&root.join(name), "");
        }
        fs::create_dir(root.join("include")).unwrap();
        let manifest = if case == "vhdl" || case == "prefix" {
            "[project]\ntop='ToP'\n[sources]\nhdl=['types.vhd','top.vhd']\n"
        } else if case == "hint" || case == "builtin-suite" {
            "[sources]\nhdl=['top.sv']\n"
        } else {
            "[sources]\nhdl=['types.sv','leaf.v','top.sv']\ninclude_dirs=['.','include']\n[synthesis]\nsv_frontend='slang'\n"
        };
        write(&root.join("ecp5.toml"), manifest);
        let original = fs::read_to_string(&f.options.tools.yosys).unwrap();
        let extra = if case == "failure" {
            "if [ \"${1:-}\" != '-V' ]; then echo 'top.sv:2: ERROR: expected expression'; echo '  assign led = ;'; echo '               ^'; exit 1; fi\n".to_string()
        } else {
            String::new()
        };
        let script = original.replace(
            "tool=$(basename",
            &format!(
                "printf '%s\\n' \"${{GHDL_PREFIX-unset}}\" >> {}\n{extra}tool=$(basename",
                quote(&suite.join("child-prefixes"))
            ),
        );
        // For the failure case, log the argv before rejecting the compile.
        let script = if case == "failure" {
            script.replace(
                "if [ \"${1:-}\" != '-V' ]; then",
                &format!(
                    "if [ \"${{1:-}}\" != '-V' ]; then printf '%s\\n' \"$@\" >> {};",
                    quote(&f.calls)
                ),
            )
        } else {
            script
        };
        write(&suite.join("bin/yosys"), script);
        fs::set_permissions(suite.join("bin/yosys"), fs::Permissions::from_mode(0o755)).unwrap();
        for path in [&f.options.tools.nextpnr, &f.options.tools.ecppack] {
            let script = fs::read_to_string(path).unwrap().replace(
                "tool=$(basename",
                &format!(
                    "printf '%s\\n' \"${{GHDL_PREFIX-unset}}\" >> {}\ntool=$(basename",
                    quote(&f.folder.0.join("downstream-prefixes"))
                ),
            );
            write(path, script);
        }
        if case == "hint" {
            let script = original.replace("tool=$(basename", &format!("if [ \"${{1:-}}\" != '-V' ]; then printf '%s\\n' \"$@\" >> {}; echo 'top.sv:2: ERROR: syntax error, unexpected TOK_IMPORT'; exit 1; fi\ntool=$(basename", quote(&f.calls)));
            write(&f.options.tools.yosys, script);
        }
        if case == "missing-plugin" {
            fs::remove_file(suite.join("share/yosys/plugins/slang.so")).unwrap();
        }
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "suite_frontends_and_cache", "--nocapture"])
            .env("ECP5_TEST_SUITE_CASE", case)
            .env("ECP5_TEST_PROJECT", root)
            .env("ECP5_TEST_SUITE_ROOT", &suite)
            .env("ECP5_TEST_CALLS", &f.calls)
            // Keep suite discovery independent of the runner's installed tools.
            .env(
                "PATH",
                env::join_paths([
                    f.options.tools.yosys.parent().unwrap(),
                    Path::new("/usr/bin"),
                    Path::new("/bin"),
                ])
                .unwrap(),
            )
            .env(
                "ECP5_OSS_CAD_SUITE",
                if case == "invalid" || case == "builtin-suite" {
                    f.folder.0.join("invalid")
                } else {
                    suite.clone()
                },
            )
            .env("ECP5_YOSYS", &f.options.tools.yosys)
            .env("ECP5_NEXTPNR", &f.options.tools.nextpnr)
            .env("ECP5_ECPPACK", &f.options.tools.ecppack)
            .env("ECP5_ECPBRAM", &f.options.tools.ecpbram)
            .env_remove("GHDL_PREFIX");
        if case == "builtin-suite" {
            command.env("ECP5_YOSYS", suite.join("bin/yosys"));
        }
        if case == "none" {
            command
                .env_remove("ECP5_OSS_CAD_SUITE")
                .env("HOME", &f.folder.0);
        }
        if case == "prefix" {
            command.env("GHDL_PREFIX", "/already/set/ghdl");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn suite_cache_new_root() {
    if let Some(root) =
        env::var_os("ECP5_TEST_PROJECT").filter(|_| env::var_os("ECP5_TEST_SUITE_CASE").is_some())
    {
        let p = Project::open(root).unwrap();
        let o = flow::build(&p, &BuildOptions::default(), &mut |_| {});
        assert!(o.ok, "{:?}", o.messages);
        assert_eq!(statuses(&o), ["ran", "skipped", "skipped"]);
    }
}

fn real_suite_counter(name: &str, plugin: &str) {
    let folder = Folder::new();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = folder.0.join("suite project");
    copy_design(&repo.join("examples/ulx3s").join(name), &design);
    let p = Project::open(&design).unwrap();
    let options = BuildOptions::default();
    let mut starts = Vec::new();
    let o = flow::build(&p, &options, &mut |event| {
        if let Event::Started { step, command } = event {
            starts.push((step, command));
        }
    });
    assert!(o.ok, "{:?}", o.messages);
    let report = o.report.as_ref().unwrap();
    let frontend = report.frontend.as_ref().unwrap();
    let suite = frontend.suite_root.as_ref().unwrap();
    assert_eq!(frontend.plugin.as_deref(), Some(plugin));
    assert_eq!(
        frontend.yosys_path,
        fs::canonicalize(suite.join("bin/yosys")).unwrap()
    );
    assert!(starts[0].1.contains(&format!("-m {plugin}")));
    assert!(starts.iter().any(|(s, command)| *s == Step::PlaceRoute
        && command.starts_with(&quote_if_needed(&options.tools.nextpnr))));
    assert!(starts.iter().any(|(s, command)| *s == Step::Bitstream
        && command.starts_with(&quote_if_needed(&options.tools.ecppack))));
    let routed: serde_json::Value =
        serde_json::from_slice(&fs::read(&o.outputs.routed_json).unwrap()).unwrap();
    let routed_text = serde_json::to_string(&routed).unwrap();
    if plugin == "slang" {
        assert!(routed_text.contains("top.sv:"));
    } else {
        assert!(!routed_text.contains("top.vhd:"));
        assert_eq!(report.notes, [flow::report::VHDL_SOURCE_LIMIT]);
    }
    // Exercise rollover as well as individual clocks, using the routed names.
    let output = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"))
        .arg("emulate")
        .arg(&o.outputs.config)
        .arg("--names")
        .arg(&o.outputs.routed_json)
        .args([
            "--startup",
            "configured",
            "--scope",
            "observed",
            "--clock",
            "net:clk_25mhz",
            "--cycles",
            "320",
            "--probe",
            "net:led",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let samples: Vec<_> = text
        .lines()
        .filter(|line| line.contains("led[0]="))
        .collect();
    assert!(!samples.is_empty(), "{text}");
    let last = samples.last().unwrap();
    for bit in 0..8 {
        assert!(
            last.contains(&format!("led[{bit}]={}", (320 >> bit) & 1)),
            "{text}"
        );
    }
    assert_eq!(
        statuses(&flow::build(&p, &options, &mut |_| {})),
        ["skipped"; 3]
    );
}
fn quote_if_needed(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c))
    {
        s.into_owned()
    } else {
        quote(path)
    }
}
#[test]
#[ignore = "requires OSS CAD Suite: set ECP5_OSS_CAD_SUITE or run source <suite>/environment"]
fn real_suite_blinky_vhdl() {
    real_suite_counter("blinky_vhdl", "ghdl");
}
#[test]
#[ignore = "requires OSS CAD Suite: set ECP5_OSS_CAD_SUITE or run source <suite>/environment"]
fn real_suite_sv_package() {
    real_suite_counter("sv_package", "slang");
}
#[test]
#[ignore = "requires OSS CAD Suite: set ECP5_OSS_CAD_SUITE or run source <suite>/environment"]
fn real_suite_slang_error_once() {
    let folder = Folder::new();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = folder.0.join("broken slang project");
    copy_design(&repo.join("examples/ulx3s/sv_package"), &design);
    write(
        &design.join("top.sv"),
        "module top(input logic clk_25mhz, output logic led);\n  assign led = ;\nendmodule\n",
    );
    let p = Project::open(&design).unwrap();
    let mut starts = 0;
    let o = flow::build(&p, &BuildOptions::default(), &mut |e| {
        if matches!(
            e,
            Event::Started {
                step: Step::Synthesis,
                ..
            }
        ) {
            starts += 1;
        }
    });
    assert!(!o.ok);
    assert_eq!(starts, 1, "{:?}", o.messages);
    assert_eq!(statuses(&o), ["failed", "not run", "not run"]);
    assert!(
        o.messages.iter().any(|m| m.severity == Severity::Error
            && m.file == Some(p.root.join("top.sv"))
            && m.line == Some(2)
            && m.text.contains("expected expression")),
        "{:?}",
        o.messages
    );
}
