//! Tool paths and a pipe runner shared by builds and SRAM loading.
use super::messages::{Message, MessageKind, Severity};
use anyhow::{bail, ensure, Context, Result};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

/// Executables, either bare PATH names or explicit paths.
#[derive(Clone, Debug)]
pub struct Tools {
    pub yosys: PathBuf,
    pub nextpnr: PathBuf,
    pub ecppack: PathBuf,
    pub ecpbram: PathBuf,
    pub openfpgaloader: PathBuf,
}
impl Default for Tools {
    fn default() -> Self {
        Self {
            yosys: "yosys".into(),
            nextpnr: "nextpnr-ecp5".into(),
            ecppack: "ecppack".into(),
            ecpbram: "ecpbram".into(),
            openfpgaloader: "openFPGALoader".into(),
        }
    }
}
impl Tools {
    /// Applies ECP5_YOSYS, ECP5_NEXTPNR, ECP5_ECPPACK, ECP5_ECPBRAM and ECP5_OPENFPGALOADER.
    pub fn from_env() -> Self {
        let mut tools = Self::default();
        for (name, path) in [
            ("YOSYS", &mut tools.yosys),
            ("NEXTPNR", &mut tools.nextpnr),
            ("ECPPACK", &mut tools.ecppack),
            ("ECPBRAM", &mut tools.ecpbram),
            ("OPENFPGALOADER", &mut tools.openfpgaloader),
        ] {
            if let Some(value) = std::env::var_os(format!("ECP5_{name}")) {
                *path = value.into();
            }
        }
        tools
    }
}

/// Only Yosys needs this override: its abc step does not quote temporary paths.
pub(crate) fn yosys_tmpdir(build_dir: &Path) -> Result<Option<PathBuf>> {
    let temp_dir = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(target_os = "macos")]
    let cache_dir = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Caches/ecp5_emu/tmp"));
    #[cfg(target_os = "linux")]
    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|cache| cache.join("ecp5_emu/tmp"));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cache_dir: Option<PathBuf> = None;
    choose_yosys_tmpdir(&temp_dir, build_dir, cache_dir.as_deref())
}

fn choose_yosys_tmpdir(
    temp_dir: &Path,
    build_dir: &Path,
    cache_dir: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let whitespace = |path: &Path| path.to_string_lossy().chars().any(char::is_whitespace);
    if !whitespace(temp_dir) {
        return Ok(None);
    }
    let build_dir = std::path::absolute(build_dir)?;
    if !whitespace(&build_dir) {
        return Ok(Some(build_dir.join(".tmp")));
    }
    if let Some(cache_dir) = cache_dir {
        let cache_dir = std::path::absolute(cache_dir)?;
        if !whitespace(&cache_dir) {
            return Ok(Some(cache_dir));
        }
    }
    bail!(
        "TMPDIR ({}) contains a space, which Yosys's abc step cannot handle; set TMPDIR to a folder without spaces",
        temp_dir.display()
    )
}

/// A suite is selected only for an explicit slang pin or VHDL roots.
#[derive(Clone, Debug)]
pub(crate) struct Suite {
    pub root: PathBuf,
    pub source: String,
}
impl Suite {
    pub fn resolve(slang: bool) -> Result<Self> {
        Self::find(
            std::env::var_os("ECP5_OSS_CAD_SUITE"),
            std::env::var_os("PATH"),
            slang,
        )
    }
    fn find(env: Option<OsString>, path: Option<OsString>, slang: bool) -> Result<Self> {
        let requirement = if slang {
            "sv_frontend = \"slang\" needs OSS CAD Suite"
        } else {
            "VHDL needs the GHDL Yosys plugin from OSS CAD Suite"
        };
        let valid = |p: &Path| !p.as_os_str().is_empty() && p.join("bin/yosys").is_file();
        if let Some(value) = env {
            let root = PathBuf::from(value);
            ensure!(valid(&root), "{requirement}: invalid ECP5_OSS_CAD_SUITE root {} (expected bin/yosys); no fallback", root.display());
            return Ok(Self {
                root: std::fs::canonicalize(root)?,
                source: "ECP5_OSS_CAD_SUITE".into(),
            });
        }
        let yosys = path.and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("yosys"))
                .find(|p| {
                    let Ok(metadata) = p.metadata() else {
                        return false;
                    };
                    if !metadata.is_file() {
                        return false;
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    true
                })
        });
        let plugin = if slang { "slang" } else { "ghdl" };
        let discover = |yosys: &Path| -> Result<PathBuf> {
            let resolved = std::fs::canonicalize(yosys)
                .with_context(|| format!("resolving symlinks for {}", yosys.display()))?;
            let bin = resolved
                .parent()
                .filter(|p| p.file_name().is_some_and(|name| name == "bin"))
                .context("resolved yosys is not in a bin directory")?;
            let root = bin.parent().context("bin has no suite root")?;
            let missing: Vec<_> = [
                "bin/yosys".to_owned(),
                "libexec/yosys".to_owned(),
                format!("share/yosys/plugins/{plugin}.so"),
            ]
            .into_iter()
            .filter(|p| !root.join(p).is_file())
            .collect();
            ensure!(
                missing.is_empty(),
                "{} is missing {}",
                root.display(),
                missing.join(", ")
            );
            Ok(root.to_path_buf())
        };
        let reason = if let Some(yosys) = &yosys {
            match discover(yosys) {
                Ok(root) => {
                    return Ok(Self {
                        source: format!("yosys on PATH ({})", root.display()),
                        root,
                    });
                }
                Err(error) => format!("{error:#}"),
            }
        } else {
            "no executable yosys found on PATH".into()
        };
        let location = yosys
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "none".into());
        bail!("{requirement}: ECP5_OSS_CAD_SUITE is not set; tried PATH yosys ({location}): not an OSS CAD Suite with the needed plugin share/yosys/plugins/{plugin}.so ({reason}); set ECP5_OSS_CAD_SUITE to the suite root, or run \"source <suite>/environment\"")
    }
}
/// Only an unavailable executable permits the tour's prebuilt fallback.
pub fn is_missing_tool(message: &Message) -> bool {
    message.kind == MessageKind::MissingTool && message.severity == Severity::Error
}
pub(crate) fn is_missing_tool_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
        )
    })
}
pub(crate) fn missing_tool_message(program: &Path, tool: &str, env: &str) -> String {
    let variable = format!("ECP5_{env}");
    if std::env::var_os(&variable).is_some_and(|value| Path::new(&value) == program) {
        return format!(
            "{tool} not found at {} (from {variable}): reinstall the package or point {variable} at a working {tool}",
            program.display()
        );
    }
    let location = if program == Path::new(tool) {
        "on PATH".into()
    } else {
        format!("at {}", program.display())
    };
    let install = if tool == "openFPGALoader" {
        "install it"
    } else {
        "install the open FPGA toolchain (yosys, nextpnr-ecp5, prjtrellis: ecppack/ecpbram)"
    };
    format!("{tool} not found {location}: {install} or set {variable}")
}
pub(crate) struct RunResult {
    pub status: ExitStatus,
    pub cancelled: bool,
}
pub(crate) fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}
fn reader(
    pipe: impl Read + Send + 'static,
    tx: mpsc::Sender<std::io::Result<String>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    })
}
/// Readers only send lines; callbacks always run on the caller's thread.
pub(crate) fn run(
    program: &Path,
    args: &[String],
    cwd: &Path,
    cancel: Option<&AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<RunResult> {
    run_with_env(program, args, cwd, &[], cancel, on_line)
}
pub(crate) fn run_with_env(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(OsString, OsString)],
    cancel: Option<&AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<RunResult> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().cloned())
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Kill the whole group on cancellation: shell wrappers may have a sleeping child
    // holding the pipes open after the shell exits.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let (tx, rx) = mpsc::channel();
    let stdout = reader(
        child.stdout.take().context("missing stdout pipe")?,
        tx.clone(),
    );
    let stderr = reader(child.stderr.take().context("missing stderr pipe")?, tx);
    let mut was_cancelled = false;
    let mut read_error = None;
    loop {
        if cancelled(cancel) && !was_cancelled {
            was_cancelled = true;
            #[cfg(unix)]
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(line)) => on_line(&line),
            Ok(Err(error)) => {
                read_error = Some(error);
                let _ = child.kill();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    // Pipes can close before the process exits, so keep cancellation responsive.
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancelled(cancel) {
            was_cancelled = true;
            let _ = child.kill();
        }
        thread::sleep(Duration::from_millis(10));
    };
    let _ = stdout.join();
    let _ = stderr.join();
    if let Some(error) = read_error {
        return Err(error.into());
    }
    Ok(RunResult {
        status,
        cancelled: was_cancelled,
    })
}
pub(crate) fn command_line(program: &Path, args: &[String]) -> String {
    std::iter::once(program.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|s| {
            if s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c))
            {
                s
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;

    #[test]
    fn yosys_tmpdir_chooses_build_then_cache_or_errors() {
        let temp = Path::new("/tmp/with spaces");
        let build = Path::new("/build/clean");
        let spaced_build = Path::new("/build/with spaces");
        let cache = Path::new("/cache/ecp5_emu/tmp");
        assert_eq!(
            choose_yosys_tmpdir(temp, build, Some(cache)).unwrap(),
            Some(build.join(".tmp"))
        );
        assert_eq!(
            choose_yosys_tmpdir(temp, spaced_build, Some(cache)).unwrap(),
            Some(cache.to_path_buf())
        );
        for cache in [
            None,
            Some(Path::new("/home/with spaces/.cache/ecp5_emu/tmp")),
        ] {
            let error = choose_yosys_tmpdir(temp, spaced_build, cache)
                .unwrap_err()
                .to_string();
            assert_eq!(error, "TMPDIR (/tmp/with spaces) contains a space, which Yosys's abc step cannot handle; set TMPDIR to a folder without spaces");
        }
        assert_eq!(
            choose_yosys_tmpdir(Path::new("/tmp/clean"), spaced_build, None).unwrap(),
            None
        );
        for temp in [
            "/tmp/with\ttab",
            "/tmp/with\nnewline",
            "/tmp/with\u{2003}space",
        ] {
            assert_eq!(
                choose_yosys_tmpdir(Path::new(temp), build, None).unwrap(),
                Some(build.join(".tmp"))
            );
        }
    }

    fn executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Discovery must not run yosys, even if it is a wrapper script.
        std::fs::write(path, "#!/bin/sh\nexit 99\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn fake_suite(root: &Path, plugin: &str) {
        executable(&root.join("bin/yosys"));
        executable(&root.join("libexec/yosys"));
        let plugin = root.join(format!("share/yosys/plugins/{plugin}.so"));
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(plugin, "plugin").unwrap();
    }

    #[test]
    fn suite_env_takes_precedence_and_invalid_env_never_falls_back() {
        let f = Folder::new();
        let root = f.0.join("explicit suite");
        // Keep the explicit override's existing bin/yosys validation.
        executable(&root.join("bin/yosys"));
        let fallback = f.0.join("path suite");
        fake_suite(&fallback, "slang");
        let path = Some(fallback.join("bin").into_os_string());
        for slang in [true, false] {
            let suite =
                Suite::find(Some(root.clone().into_os_string()), path.clone(), slang).unwrap();
            assert_eq!(suite.root, std::fs::canonicalize(&root).unwrap());
            assert_eq!(suite.source, "ECP5_OSS_CAD_SUITE");
            for invalid in [OsString::new(), f.0.join("invalid").into_os_string()] {
                let error = Suite::find(Some(invalid), path.clone(), slang)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("invalid ECP5_OSS_CAD_SUITE root"), "{error}");
                assert!(error.contains("expected bin/yosys"), "{error}");
                assert!(error.contains("no fallback"), "{error}");
            }
        }
    }

    #[test]
    fn suite_from_path_requires_the_projects_plugin() {
        let f = Folder::new();
        for (slang, plugin, other) in [(true, "slang", "ghdl"), (false, "ghdl", "slang")] {
            let root = f.0.join(format!("suite {plugin}"));
            fake_suite(&root, other);
            let path = Some(root.join("bin").into_os_string());
            let error = Suite::find(None, path.clone(), slang)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("is missing share/yosys/plugins/{plugin}.so")),
                "{error}"
            );
            assert!(error.contains(&root.join("bin/yosys").display().to_string()));
            fake_suite(&root, plugin);
            let suite = Suite::find(None, path, slang).unwrap();
            assert_eq!(suite.root, std::fs::canonicalize(root).unwrap());
            assert_eq!(
                suite.source,
                format!("yosys on PATH ({})", suite.root.display())
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn suite_from_path_follows_symlink_chains() {
        use std::os::unix::fs::symlink;
        let f = Folder::new();
        let root = f.0.join("suite with spaces");
        fake_suite(&root, "slang");
        let links = f.0.join("links");
        std::fs::create_dir(&links).unwrap();
        symlink("../suite with spaces/bin/yosys", links.join("intermediate")).unwrap();
        symlink("intermediate", links.join("yosys")).unwrap();
        let suite = Suite::find(None, Some(links.into_os_string()), true).unwrap();
        assert_eq!(suite.root, std::fs::canonicalize(root).unwrap());
        assert_eq!(
            suite.source,
            format!("yosys on PATH ({})", suite.root.display())
        );
    }

    #[test]
    fn suite_from_path_rejects_homebrew_and_does_not_search_past_it() {
        let f = Folder::new();
        let brew = f.0.join("homebrew/Cellar/yosys/version");
        executable(&brew.join("bin/yosys"));
        let suite = f.0.join("suite");
        fake_suite(&suite, "slang");
        let path = std::env::join_paths([brew.join("bin"), suite.join("bin")]).unwrap();
        let error = Suite::find(None, Some(path), true).unwrap_err().to_string();
        assert!(error.contains(&brew.join("bin/yosys").display().to_string()));
        assert!(
            error.contains("is missing libexec/yosys, share/yosys/plugins/slang.so"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn suite_from_path_rejects_homebrew_symlink_and_non_executable_yosys() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Folder::new();
        let brew = f.0.join("homebrew");
        let cellar = brew.join("Cellar/yosys/version/bin/yosys");
        executable(&cellar);
        std::fs::create_dir(brew.join("bin")).unwrap();
        symlink("../Cellar/yosys/version/bin/yosys", brew.join("bin/yosys")).unwrap();
        let path = Some(brew.join("bin").into_os_string());
        let error = Suite::find(None, path.clone(), false)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("is missing libexec/yosys, share/yosys/plugins/ghdl.so"),
            "{error}"
        );
        std::fs::set_permissions(cellar, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = Suite::find(None, path, false).unwrap_err().to_string();
        assert!(error.contains("PATH yosys (none)"), "{error}");
    }

    #[test]
    fn suite_from_path_rejects_yosys_outside_bin() {
        let f = Folder::new();
        executable(&f.0.join("yosys"));
        let error = Suite::find(None, Some(f.0.clone().into_os_string()), true)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("resolved yosys is not in a bin directory"),
            "{error}"
        );
    }

    #[test]
    fn suite_without_path_yosys_has_actionable_error() {
        let f = Folder::new();
        for path in [None, Some(f.0.clone().into_os_string())] {
            for (slang, requirement, plugin) in [
                (true, "sv_frontend = \"slang\" needs OSS CAD Suite", "slang"),
                (
                    false,
                    "VHDL needs the GHDL Yosys plugin from OSS CAD Suite",
                    "ghdl",
                ),
            ] {
                let error = Suite::find(None, path.clone(), slang)
                    .unwrap_err()
                    .to_string();
                assert_eq!(error, format!("{requirement}: ECP5_OSS_CAD_SUITE is not set; tried PATH yosys (none): not an OSS CAD Suite with the needed plugin share/yosys/plugins/{plugin}.so (no executable yosys found on PATH); set ECP5_OSS_CAD_SUITE to the suite root, or run \"source <suite>/environment\""));
            }
        }
    }
}
