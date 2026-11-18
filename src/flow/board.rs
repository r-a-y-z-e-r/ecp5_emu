//! Volatile SRAM loading for a ULX3S. No flash programming is exposed.
//!
//! One board is often shared by several runs (a terminal on its UART, a test
//! that loads it, the window). A lock file says who has it: created
//! atomically before openFPGALoader runs, holding `<who> <unix seconds>`, and
//! removed when the load is over. A load finds it there and stops with whose
//! it is and how old, instead of loading over another run.
use super::{tools, Tools};
use anyhow::{bail, ensure, Context, Result};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// The complete loader argument list. The default operation writes SRAM.
pub fn load_args(bitstream: &Path) -> Vec<String> {
    vec![
        "-b".into(),
        "ulx3s".into(),
        bitstream.to_string_lossy().into_owned(),
    ]
}

/// Where the board's lock is: `ECP5_BOARD_LOCK`, else `out/board.lock` of
/// this checkout (of the main checkout when this one is a git worktree under
/// `.claude/worktrees/`, so that every worktree shares one lock), else in the
/// temporary folder when the checkout is gone (an installed binary).
pub fn lock_path() -> PathBuf {
    if let Some(path) = std::env::var_os("ECP5_BOARD_LOCK") {
        return path.into();
    }
    let manifest = env!("CARGO_MANIFEST_DIR");
    let repo = manifest
        .find("/.claude/worktrees/")
        .map_or(manifest, |at| &manifest[..at]);
    let repo = Path::new(repo);
    if repo.is_dir() {
        repo.join("out/board.lock")
    } else {
        std::env::temp_dir().join("ecp5_emu_board.lock")
    }
}

/// The board, held while the lock file exists; dropping it removes the file.
pub struct BoardLock {
    path: PathBuf,
}
impl BoardLock {
    /// Takes the lock, or says whose it is when another run holds it.
    pub fn take(path: &Path, who: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                writeln!(file, "{who} {now}")?;
                Ok(Self { path: path.into() })
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                let held = fs::read_to_string(path).unwrap_or_default();
                let mut fields = held.split_whitespace();
                let owner = fields.next().unwrap_or("another run");
                let age = fields
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map_or_else(String::new, |since| {
                        format!(", since {} min ago", now.saturating_sub(since) / 60)
                    });
                bail!(
                    "the board is busy: {owner} holds {}{age}. Wait for it, or remove that file if that run is over",
                    path.display()
                )
            }
            Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
}
impl Drop for BoardLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Loads a bitstream into SRAM, streaming both output pipes on the caller
/// thread. With `lock` (normally [`lock_path`]) the board is held for the
/// load and a load already running elsewhere is not disturbed.
pub fn load(
    bitstream: &Path,
    tools: &Tools,
    lock: Option<&Path>,
    on_line: &mut dyn FnMut(&str),
) -> Result<()> {
    ensure!(
        bitstream.is_file(),
        "missing bitstream {}",
        bitstream.display()
    );
    let bitstream = fs::canonicalize(bitstream)?;
    let _held = lock
        .map(|path| BoardLock::take(path, &format!("ecp5_emu-load-{}", std::process::id())))
        .transpose()?;
    let mut output = String::new();
    let result = tools::run(
        &tools.openfpgaloader,
        &load_args(&bitstream),
        bitstream.parent().unwrap(),
        None,
        &mut |line| {
            output.push_str(line);
            output.push('\n');
            on_line(line);
        },
    )
    .map_err(|e| {
        if tools::is_missing_tool_error(&e) {
            anyhow::anyhow!(tools::missing_tool_message(
                &tools.openfpgaloader,
                "openFPGALoader",
                "OPENFPGALOADER",
            ))
        } else {
            e
        }
    })?;
    if !result.status.success() {
        let lower = output.to_lowercase();
        if [
            "unable to open ftdi device",
            "no device found",
            "jtag init failed",
            "cable not found",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            bail!("no ULX3S found on USB; plug it in by the US1 port and close any serial terminal open on it (the FT231X carries both JTAG and the UART)");
        }
        bail!(
            "openFPGALoader failed ({}): {}",
            result.status,
            output.trim()
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sram_only() {
        let args = load_args(Path::new("top.bit"));
        assert_eq!(args, ["-b", "ulx3s", "top.bit"]);
        assert!(!args
            .iter()
            .any(|s| ["-f", "--write-flash", "--flash"].contains(&s.as_str())));
    }
    #[test]
    fn missing_loader_names_the_attempted_path() {
        let dir = crate::flow::test_support::Folder::new();
        let bitstream = dir.0.join("top.bit");
        fs::write(&bitstream, []).unwrap();
        let tools = Tools {
            openfpgaloader: dir.0.join("missing loader"),
            ..Tools::default()
        };
        let error = load(&bitstream, &tools, None, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            format!(
                "openFPGALoader not found at {}: install it or set ECP5_OPENFPGALOADER",
                tools.openfpgaloader.display()
            )
        );
    }
    /// The lock is taken once, says whose it is to a second taker, and is
    /// gone when dropped.
    #[test]
    fn lock_is_exclusive() {
        let dir = crate::flow::test_support::Folder::new();
        let path = dir.0.join("out/board.lock");
        let held = BoardLock::take(&path, "first").unwrap();
        assert!(fs::read_to_string(&path).unwrap().starts_with("first "));
        let error = BoardLock::take(&path, "second").err().unwrap().to_string();
        assert!(error.contains("board is busy: first holds"), "{error}");
        assert!(error.contains("min ago"), "{error}");
        drop(held);
        assert!(!path.exists());
        BoardLock::take(&path, "second").unwrap();
    }
    /// A worktree shares the main checkout's lock.
    #[test]
    fn lock_path_of_this_checkout() {
        if std::env::var_os("ECP5_BOARD_LOCK").is_none() {
            let path = lock_path();
            assert!(path.ends_with("out/board.lock"), "{}", path.display());
            assert!(!path.to_string_lossy().contains("/.claude/worktrees/"));
        }
    }
}
