//! Synchronous build orchestration suitable for a background thread.
use super::{
    cache::{self, Cache},
    messages::{self, Message, MessageKind, Severity},
    project::{HdlLanguage, Outputs, Project, SvFrontend},
    report::{Frontend, FrontendKind, PluginIdentity, Report, VHDL_SOURCE_LIMIT},
    tools::{self, Tools},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

/// A build stage, in execution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Step {
    Synthesis,
    PlaceRoute,
    Swap,
    Bitstream,
}
impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Synthesis => "Synthesis (yosys)",
            Self::PlaceRoute => "Place and route (nextpnr-ecp5)",
            Self::Swap => "Memory images (ecpbram)",
            Self::Bitstream => "Bitstream (ecppack)",
        })
    }
}
/// Build policy and executables. Cancellation can be requested from another thread.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub force: bool,
    pub tools: Tools,
    pub cancel: Option<Arc<AtomicBool>>,
}
impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            force: false,
            tools: Tools::from_env(),
            cancel: None,
        }
    }
}
/// Owned notifications, delivered on the thread calling [`build`].
#[derive(Clone, Debug)]
pub enum Event {
    Started { step: Step, command: String },
    Skipped { step: Step },
    Output { step: Step, line: String },
    Message(Message),
    Finished { step: Step, ok: bool, seconds: f64 },
}
/// Execution result for a single stage; later stages remain NotRun after failure.
#[derive(Clone, Debug, PartialEq)]
pub enum StepStatus {
    Ran { seconds: f64 },
    Skipped,
    Failed,
    NotRun,
}
/// Complete build result, including partial diagnostics on failure or cancellation.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub ok: bool,
    pub cancelled: bool,
    pub steps: Vec<(Step, StepStatus)>,
    pub messages: Vec<Message>,
    pub report: Option<Report>,
    pub outputs: Outputs,
}
struct Image {
    path: PathBuf,
    placeholder: PathBuf,
    width: usize,
    depth: usize,
    seed: usize,
}
struct Invocation {
    program: PathBuf,
    args: Vec<String>,
    tool: &'static str,
    env: &'static str,
    child_env: Vec<(OsString, OsString)>,
}
impl Invocation {
    fn new(program: &Path, args: Vec<String>, tool: &'static str, env: &'static str) -> Self {
        Self {
            program: program.into(),
            args,
            tool,
            env,
            child_env: Vec::new(),
        }
    }
    fn display(&self) -> String {
        tools::command_line(&self.program, &self.args)
    }
}
fn string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn yosys_path(path: &Path) -> String {
    let s = string(path);
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
    {
        s
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
fn synthesis_script(project: &Project) -> Result<String> {
    let mut reads = Vec::new();
    if project.has_vhdl() {
        reads.push(format!(
            "ghdl --std=08 {} -e {}",
            project
                .hdl_roots()
                .iter()
                .map(|p| yosys_path(p))
                .collect::<Vec<_>>()
                .join(" "),
            project.top
        ));
    } else if project.sv_frontend == SvFrontend::Slang {
        let mut args = vec!["read_slang".into(), "--top".into(), project.top.clone()];
        args.extend(
            project
                .include_dirs
                .iter()
                .map(|p| format!("-I{}", yosys_path(p))),
        );
        args.extend(project.hdl_roots().iter().map(|p| yosys_path(p)));
        reads.push(args.join(" "));
    } else if let Some(roots) = &project.hdl {
        let includes = project
            .include_dirs
            .iter()
            .map(|p| format!("-I{}", yosys_path(p)))
            .collect::<Vec<_>>();
        let mut start = 0;
        while start < roots.len() {
            let language = HdlLanguage::from_path(&roots[start])?;
            let mut end = start + 1;
            while end < roots.len() && HdlLanguage::from_path(&roots[end])? == language {
                end += 1;
            }
            let mut args = vec!["read_verilog".to_string()];
            if language == HdlLanguage::SystemVerilog {
                args.push("-sv".into());
            }
            args.extend(includes.iter().cloned());
            args.extend(roots[start..end].iter().map(|p| yosys_path(p)));
            reads.push(args.join(" "));
            start = end;
        }
    } else {
        reads.push(format!(
            "read_verilog {}",
            project
                .verilog
                .iter()
                .map(|p| yosys_path(p))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    reads.push(format!("synth_ecp5 -top {} -json top.json", project.top));
    Ok(reads.join("; "))
}
fn suite_plugin(project: &Project) -> Option<&'static str> {
    if project.has_vhdl() {
        Some("ghdl")
    } else if project.sv_frontend == SvFrontend::Slang {
        Some("slang")
    } else {
        None
    }
}
fn select_frontend(project: &Project, tools: &mut Tools) -> Result<Frontend> {
    let plugin = suite_plugin(project);
    let suite = plugin
        .map(|plugin| tools::Suite::resolve(plugin == "slang"))
        .transpose()?;
    let identity = if let (Some(suite), Some(plugin)) = (&suite, plugin) {
        tools.yosys = suite.root.join("bin/yosys");
        let path = PathBuf::from(format!("share/yosys/plugins/{plugin}.so"));
        Some(PluginIdentity {
            sha256: cache::file_hash(&suite.root.join(&path)).with_context(|| {
                format!(
                    "OSS CAD Suite {} ({}) needs plugin {}",
                    suite.root.display(),
                    suite.source,
                    path.display()
                )
            })?,
            path,
        })
    } else {
        None
    };
    let ghdl_prefix = suite.as_ref().map(|suite| {
        std::env::var_os("GHDL_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|| suite.root.join("lib/ghdl"))
    });
    Ok(Frontend {
        requested: project.sv_frontend,
        resolved: match plugin {
            Some("ghdl") => FrontendKind::Ghdl,
            Some("slang") => FrontendKind::Slang,
            _ => FrontendKind::Builtin,
        },
        reason: match plugin {
            Some("ghdl") => "VHDL roots: GHDL plugin (work)",
            Some("slang") => "explicit slang pin",
            _ => match project.sv_frontend {
                SvFrontend::Auto => "auto: Yosys built-in reader (read_verilog -sv)",
                _ => "builtin: pinned Yosys built-in reader",
            },
        }
        .into(),
        yosys_path: resolved_program(&tools.yosys, &project.build_dir),
        yosys_version: String::new(),
        suite_root: suite.as_ref().map(|s| s.root.clone()),
        suite_source: suite.map(|s| s.source),
        plugin: plugin.map(str::to_owned),
        plugin_identity: identity,
        vhdl_standard: project.has_vhdl().then(|| "2008".into()),
        ghdl_prefix,
    })
}
fn program_path(program: &Path, cwd: &Path) -> PathBuf {
    if program.is_absolute() {
        program.to_path_buf()
    } else if program.components().count() > 1 {
        cwd.join(program)
    } else {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| {
                        if dir.is_absolute() {
                            dir
                        } else {
                            cwd.join(dir)
                        }
                        .join(program)
                    })
                    .find(|p| {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            p.is_file()
                                && p.metadata()
                                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                        }
                        #[cfg(not(unix))]
                        p.is_file()
                    })
            })
            .unwrap_or_else(|| program.to_path_buf())
    }
}
fn resolved_program(program: &Path, cwd: &Path) -> PathBuf {
    let path = program_path(program, cwd);
    fs::canonicalize(&path).unwrap_or(path)
}
fn copy_changed(from: &Path, to: &Path) -> Result<()> {
    let bytes = fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    if fs::read(to).is_ok_and(|b| b == bytes) {
        return Ok(());
    }
    fs::create_dir_all(to.parent().context("destination has no parent")?)?;
    fs::write(to, bytes).with_context(|| format!("writing {}", to.display()))?;
    Ok(())
}
fn synthesis_dir(project: &Project) -> PathBuf {
    if project.hdl.is_some() {
        project.build_dir.join(".hdl-synthesis")
    } else {
        project.build_dir.clone()
    }
}
fn images(project: &Project) -> Result<Vec<Image>> {
    project
        .memory_images
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let bytes = fs::read(project.root.join(path))?;
            let first = bytes.split(|&b| b == b'\n').next().unwrap_or_default();
            let width = first.iter().filter(|b| !b.is_ascii_whitespace()).count() * 4;
            let depth = bytes.iter().filter(|&&b| b == b'\n').count();
            ensure!(
                width > 0 && depth > 0,
                "memory image {} needs a non-empty first line and newline-terminated rows",
                path.display()
            );
            Ok(Image {
                path: path.clone(),
                placeholder: PathBuf::from(format!("placeholder_{}", path.display())),
                width,
                depth,
                seed: i + 1,
            })
        })
        .collect()
}
fn invocations(
    project: &Project,
    step: Step,
    images: &[Image],
    tools: &Tools,
    frontend: &Frontend,
) -> Result<Vec<Invocation>> {
    let one = |program: &Path, args, tool, env| vec![Invocation::new(program, args, tool, env)];
    Ok(match step {
        Step::Synthesis => {
            let mut commands: Vec<_> = images
                .iter()
                .map(|i| {
                    Invocation::new(
                        &tools.ecpbram,
                        vec![
                            "-g".into(),
                            string(&i.placeholder),
                            "-w".into(),
                            i.width.to_string(),
                            "-d".into(),
                            i.depth.to_string(),
                            "-s".into(),
                            i.seed.to_string(),
                        ],
                        "ecpbram",
                        "ECPBRAM",
                    )
                })
                .collect();
            let mut args = Vec::new();
            if let Some(plugin) = &frontend.plugin {
                args.extend(["-m".into(), plugin.clone()]);
            }
            args.extend([
                "-l".into(),
                "yosys.log".into(),
                "-p".into(),
                synthesis_script(project)?,
            ]);
            let mut yosys = Invocation::new(&tools.yosys, args, "yosys", "YOSYS");
            if let Some(prefix) = &frontend.ghdl_prefix {
                yosys
                    .child_env
                    .push(("GHDL_PREFIX".into(), prefix.as_os_str().into()));
            }
            commands.push(yosys);
            commands
        }
        Step::PlaceRoute => {
            let mut args = vec![
                project.device.flag()?.into(),
                "--package".into(),
                project.device.package.clone(),
            ];
            if let Some(speed) = project.device.speed {
                args.extend(["--speed".into(), speed.to_string()]);
            }
            args.extend([
                "--json".into(),
                "top.json".into(),
                "--lpf".into(),
                string(&project.constraints),
                "--textcfg".into(),
                if images.is_empty() {
                    "top.config"
                } else {
                    "top_placeholder.config"
                }
                .into(),
                "--write".into(),
                "top_routed.json".into(),
                "--seed".into(),
                project.seed.to_string(),
            ]);
            if project.allow_timing_failure {
                args.push("--timing-allow-fail".into());
            }
            args.extend(["-l".into(), "nextpnr.log".into()]);
            one(&tools.nextpnr, args, "nextpnr-ecp5", "NEXTPNR")
        }
        Step::Swap => images
            .iter()
            .map(|i| {
                Invocation::new(
                    &tools.ecpbram,
                    vec![
                        "-i".into(),
                        "top.config".into(),
                        "-o".into(),
                        "top.swap.config".into(),
                        "-f".into(),
                        string(&i.placeholder),
                        "-t".into(),
                        string(&project.root.join(&i.path)),
                    ],
                    "ecpbram",
                    "ECPBRAM",
                )
            })
            .collect(),
        Step::Bitstream => one(
            &tools.ecppack,
            vec!["--compress".into(), "top.config".into(), "top.bit".into()],
            "ecppack",
            "ECPPACK",
        ),
    })
}
fn inputs(project: &Project, step: Step, images: &[Image]) -> Vec<(PathBuf, PathBuf)> {
    let original = |p: &PathBuf| (p.clone(), project.root.join(p));
    let built = |name: &str| (PathBuf::from(name), project.build_dir.join(name));
    match step {
        Step::Synthesis => project
            .hdl_roots()
            .iter()
            .chain(&project.files)
            .map(original)
            .collect(),
        Step::PlaceRoute => vec![built("top.json"), original(&project.constraints)],
        Step::Swap => std::iter::once(built("top_placeholder.config"))
            .chain(images.iter().map(|i| {
                (
                    i.placeholder.clone(),
                    project.build_dir.join(&i.placeholder),
                )
            }))
            .chain(project.memory_images.iter().map(original))
            .collect(),
        Step::Bitstream => vec![built("top.config")],
    }
}
fn outputs(project: &Project, step: Step, images: &[Image]) -> Vec<PathBuf> {
    let names: &[&str] = match step {
        Step::Synthesis => &["top.json", "yosys.log"],
        Step::PlaceRoute if images.is_empty() => &["top.config", "top_routed.json", "nextpnr.log"],
        Step::PlaceRoute => &["top_placeholder.config", "top_routed.json", "nextpnr.log"],
        Step::Swap => &["top.config"],
        Step::Bitstream => &["top.bit"],
    };
    let mut paths: Vec<_> = names.iter().map(|p| project.build_dir.join(p)).collect();
    if step == Step::Synthesis {
        paths.extend(
            images
                .iter()
                .map(|i| project.build_dir.join(&i.placeholder)),
        );
    }
    paths
}
fn message(outcome: &mut Outcome, on_event: &mut dyn FnMut(Event), m: Message) {
    if let Some(existing) = outcome.messages.iter_mut().find(|old| old.same(&m)) {
        existing.count += 1;
    } else {
        on_event(Event::Message(m.clone()));
        outcome.messages.push(m);
    }
}
fn version(program: &Path, arg: &str, project: &Project, options: &BuildOptions) -> String {
    let mut lines = Vec::new();
    // A missing executable is diagnosed at its actual stage, with the install hint.
    let _ = tools::run(
        program,
        &[arg.into()],
        &project.build_dir,
        options.cancel.as_deref(),
        &mut |line| lines.push(line.to_owned()),
    );
    lines.join("\n")
}
fn execute(
    project: &Project,
    options: &BuildOptions,
    step: Step,
    images: &[Image],
    commands: &[Invocation],
    outcome: &mut Outcome,
    on_event: &mut dyn FnMut(Event),
) -> Result<bool> {
    let cwd = if step == Step::Synthesis {
        synthesis_dir(project)
    } else {
        project.build_dir.clone()
    };
    if step == Step::Swap {
        copy_changed(
            &project.build_dir.join("top_placeholder.config"),
            &project.build_dir.join("top.config"),
        )?;
    }
    let yosys_tmpdir = if step == Step::Synthesis {
        tools::yosys_tmpdir(&project.build_dir)?
    } else {
        None
    };
    if let Some(dir) = &yosys_tmpdir {
        // The cache is shared by builds; only clear our build-owned directory.
        if *dir == project.build_dir.join(".tmp") && dir.exists() {
            fs::remove_dir_all(dir)
                .with_context(|| format!("cleaning Yosys temporary directory {}", dir.display()))?;
        }
        fs::create_dir_all(dir)
            .with_context(|| format!("creating Yosys temporary directory {}", dir.display()))?;
        on_event(Event::Output {
            step,
            line: format!(
                "TMPDIR redirected to {} for Yosys's abc step",
                dir.display()
            ),
        });
    }
    for (index, command) in commands.iter().enumerate() {
        if tools::cancelled(options.cancel.as_deref()) {
            outcome.cancelled = true;
            return Ok(false);
        }
        if step == Step::Synthesis && index < images.len() {
            fs::create_dir_all(cwd.join(&images[index].placeholder).parent().unwrap())?;
        }
        let mut last = String::new();
        let errors_before = outcome
            .messages
            .iter()
            .filter(|m| m.step == step && m.severity == Severity::Error)
            .count();
        let program = if step == Step::Synthesis && project.hdl.is_some() {
            program_path(&command.program, &project.build_dir)
        } else {
            command.program.clone()
        };
        let mut child_env = command.child_env.clone();
        if command.tool == "yosys" {
            if let Some(dir) = &yosys_tmpdir {
                child_env.push(("TMPDIR".into(), dir.as_os_str().into()));
            }
        }
        let mut pending: Option<Message> = None;
        let result = tools::run_with_env(
            &program,
            &command.args,
            &cwd,
            &child_env,
            options.cancel.as_deref(),
            &mut |line| {
                on_event(Event::Output {
                    step,
                    line: line.into(),
                });
                if !line.trim().is_empty() {
                    last = line.trim().into();
                }
                if let Some(mut m) = messages::parse(project, step, line) {
                    if let Some(previous) = pending.take() {
                        message(outcome, on_event, previous);
                    }
                    if step == Step::Synthesis
                        && suite_plugin(project).is_none()
                        && m.severity == Severity::Error
                        && project.hdl.as_ref().is_some_and(|roots| {
                            roots.iter().any(|root| {
                                HdlLanguage::from_path(root).ok()
                                    == Some(HdlLanguage::SystemVerilog)
                                    && m.file.as_ref() == Some(&project.root.join(root))
                            })
                        })
                        && (line.contains("unexpected TOK_IMPORT")
                            || line.contains("unexpected TOK_ID"))
                    {
                        m.text.push_str("; set [synthesis] sv_frontend = \"slang\" in ecp5.toml (needs OSS CAD Suite)");
                    }
                    if project.hdl.is_some()
                        && m.severity == Severity::Error
                        && line.contains("include file")
                    {
                        m.text.push_str("; list included headers in sources.files (include_dirs does not copy directory contents)");
                    }
                    pending = Some(m);
                } else if messages::is_continuation(line) {
                    if let Some(m) = pending.as_mut().filter(|m| m.file.is_some()) {
                        m.text.push('\n');
                        m.text.push_str(line);
                    }
                } else if let Some(previous) = pending.take() {
                    message(outcome, on_event, previous);
                }
            },
        );
        if let Some(previous) = pending.take() {
            message(outcome, on_event, previous);
        }
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let missing = tools::is_missing_tool_error(&error);
                let text = if missing {
                    tools::missing_tool_message(&command.program, command.tool, command.env)
                } else {
                    format!("{}: {error:#}", command.tool)
                };
                let mut diagnostic = Message::new(step, Severity::Error, text);
                if missing {
                    diagnostic.kind = MessageKind::MissingTool;
                }
                message(outcome, on_event, diagnostic);
                return Ok(false);
            }
        };
        if result.cancelled {
            outcome.cancelled = true;
            return Ok(false);
        }
        if !result.status.success() {
            if outcome
                .messages
                .iter()
                .filter(|m| m.step == step && m.severity == Severity::Error)
                .count()
                == errors_before
            {
                let exit = result
                    .status
                    .code()
                    .map_or_else(|| "signal".into(), |n| n.to_string());
                message(
                    outcome,
                    on_event,
                    Message::new(
                        step,
                        Severity::Error,
                        format!("{} failed (exit {exit}): {last}", command.tool),
                    ),
                );
            }
            return Ok(false);
        }
        if step == Step::Synthesis && index < images.len() {
            copy_changed(
                &cwd.join(&images[index].placeholder),
                &cwd.join(&images[index].path),
            )?;
        }
        if step == Step::Swap {
            fs::rename(
                project.build_dir.join("top.swap.config"),
                project.build_dir.join("top.config"),
            )?;
            copy_changed(
                &project.root.join(&images[index].path),
                &project.build_dir.join(&images[index].path),
            )?;
        }
    }
    Ok(true)
}
/// Builds a project, streaming events, and returns partial results even on failure.
/// The caller may move the project/options to a worker thread; no global cwd or
/// environment is changed. Do not build the same output folder concurrently.
pub fn build(
    project: &Project,
    options: &BuildOptions,
    on_event: &mut dyn FnMut(Event),
) -> Outcome {
    let mut steps = vec![Step::Synthesis, Step::PlaceRoute];
    if !project.memory_images.is_empty() {
        steps.push(Step::Swap);
    }
    steps.push(Step::Bitstream);
    let mut outcome = Outcome {
        ok: false,
        cancelled: false,
        steps: steps.iter().map(|s| (*s, StepStatus::NotRun)).collect(),
        messages: Vec::new(),
        report: None,
        outputs: project.outputs(),
    };
    let mut selected_tools = options.tools.clone();
    let prepared = (|| -> Result<(Vec<Image>, Frontend)> {
        // Resolve the suite and plugin before any tool, including version probes.
        let frontend = select_frontend(project, &mut selected_tools)?;
        ensure!(
            project.build_dir != project.root,
            "build directory must not be the project folder itself"
        );
        fs::create_dir_all(&project.build_dir)?;
        let source_dir = synthesis_dir(project);
        if project.hdl.is_some() {
            let workspace = if source_dir.exists() {
                fs::canonicalize(&source_dir)?
            } else {
                source_dir.clone()
            };
            ensure!(
                !project.root.starts_with(&workspace),
                "HDL synthesis workspace must not contain the project folder"
            );
            if source_dir.exists() {
                fs::remove_dir_all(&source_dir)?;
            }
        }
        fs::create_dir_all(&source_dir)?;
        // A fresh HDL workspace contains only declared inputs. Stale copied
        // headers cannot turn an unlisted include into an untracked dependency.
        for path in project.hdl_roots().iter().chain(&project.files) {
            copy_changed(&project.root.join(path), &source_dir.join(path))?;
        }
        copy_changed(
            &project.root.join(&project.constraints),
            &project.build_dir.join(&project.constraints),
        )?;
        for path in &project.include_dirs {
            fs::create_dir_all(source_dir.join(path))?;
        }
        Ok((images(project)?, frontend))
    })();
    let (images, mut frontend) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            outcome.steps[0].1 = StepStatus::Failed;
            message(
                &mut outcome,
                on_event,
                Message::new(
                    Step::Synthesis,
                    Severity::Error,
                    format!("preparing project: {error:#}"),
                ),
            );
            return outcome;
        }
    };
    let yosys_version = version(&selected_tools.yosys, "-V", project, options);
    let nextpnr_version = version(&selected_tools.nextpnr, "--version", project, options);
    frontend.yosys_version = yosys_version.clone();
    let mut cache = Cache::read(&project.build_dir);
    for (index, step) in steps.iter().copied().enumerate() {
        if tools::cancelled(options.cancel.as_deref()) {
            outcome.cancelled = true;
            break;
        }
        let start = Instant::now();
        let result = (|| -> Result<bool> {
            let commands = invocations(project, step, &images, &selected_tools, &frontend)?;
            let mut parts = Vec::new();
            for c in &commands {
                parts.push(string(&c.program));
                parts.extend(c.args.iter().cloned());
            }
            parts.push(match step {
                Step::Synthesis => yosys_version.clone(),
                Step::PlaceRoute => nextpnr_version.clone(),
                _ => String::new(),
            });
            if step == Step::Synthesis {
                parts.push(
                    if project.hdl.is_some() {
                        "ordered-hdl"
                    } else {
                        "legacy-verilog"
                    }
                    .into(),
                );
                parts.push(serde_json::to_string(&frontend)?);
                // The script preserves runs and include options; record each
                // language too, including legacy Verilog-mode roots.
                for path in project.hdl_roots() {
                    let language = if project.hdl.is_some() {
                        HdlLanguage::from_path(path)?
                    } else {
                        HdlLanguage::Verilog
                    };
                    parts.push(serde_json::to_string(&(path, language))?);
                }
            }
            let key = cache::key(&parts, &inputs(project, step, &images))?;
            let outputs = outputs(project, step, &images);
            // A step is skipped when what it reads is unchanged, whether or
            // not the step before it ran: a comment edited in a source gives
            // yosys the same netlist, and then place and route (40 minutes
            // for life_full) is not run again. The key holds the hashes of
            // the step's inputs, outputs of the steps before it included.
            if !options.force && cache.matches(step, &key, &outputs) {
                outcome.steps[index].1 = StepStatus::Skipped;
                on_event(Event::Skipped { step });
                return Ok(true);
            }
            // Forgotten before the step runs, so that a step stopped half way
            // (failed, cancelled, killed) runs again next time.
            cache.remove(step);
            cache.save(&project.build_dir)?;
            on_event(Event::Started {
                step,
                command: commands
                    .iter()
                    .map(Invocation::display)
                    .collect::<Vec<_>>()
                    .join(" && "),
            });
            if step == Step::Synthesis {
                on_event(Event::Output {
                    step,
                    line: format!("Frontend: {}", frontend.summary()),
                });
            }
            let ok = execute(
                project,
                options,
                step,
                &images,
                &commands,
                &mut outcome,
                on_event,
            )?;
            if step == Step::Synthesis && project.hdl.is_some() {
                for output in &outputs {
                    let relative = output.strip_prefix(&project.build_dir)?;
                    let staged = synthesis_dir(project).join(relative);
                    if staged.is_file() {
                        copy_changed(&staged, output)?;
                    }
                }
            }
            if !ok {
                return Ok(false);
            }
            cache.record(step, key, &outputs)?;
            cache.save(&project.build_dir)?;
            Ok(true)
        })();
        let ok = match result {
            Ok(ok) => ok,
            Err(error) => {
                message(
                    &mut outcome,
                    on_event,
                    Message::new(step, Severity::Error, format!("{error:#}")),
                );
                false
            }
        };
        if outcome.steps[index].1 == StepStatus::Skipped {
            continue;
        }
        let seconds = start.elapsed().as_secs_f64();
        outcome.steps[index].1 = if ok {
            StepStatus::Ran { seconds }
        } else {
            StepStatus::Failed
        };
        on_event(Event::Finished { step, ok, seconds });
        if !ok {
            break;
        }
    }
    if outcome.cancelled {
        let step = outcome
            .steps
            .iter()
            .find(|(_, s)| matches!(s, StepStatus::Failed | StepStatus::NotRun))
            .map_or(Step::Synthesis, |(s, _)| *s);
        message(
            &mut outcome,
            on_event,
            Message::new(step, Severity::Info, "build cancelled"),
        );
        return outcome;
    }
    if outcome
        .steps
        .iter()
        .all(|(_, s)| matches!(s, StepStatus::Ran { .. } | StepStatus::Skipped))
    {
        let report = (|| -> Result<Report> {
            let mut report = Report::parse(
                &fs::read_to_string(&outcome.outputs.yosys_log)?,
                &fs::read_to_string(&outcome.outputs.nextpnr_log)?,
                &project.top,
            );
            report.frontend = Some(frontend);
            if project.has_vhdl() {
                report.notes.push(VHDL_SOURCE_LIMIT.into());
            }
            fs::write(&outcome.outputs.report, serde_json::to_vec_pretty(&report)?)?;
            Ok(report)
        })();
        match report {
            Ok(report) => {
                outcome.report = Some(report);
                outcome.ok = true;
            }
            Err(error) => message(
                &mut outcome,
                on_event,
                Message::new(
                    Step::Bitstream,
                    Severity::Error,
                    format!("writing report: {error:#}"),
                ),
            ),
        }
    }
    outcome
}
