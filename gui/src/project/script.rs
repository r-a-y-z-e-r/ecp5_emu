//! Recording (`ECP5_PROJECT_TRACE`) and scripted playback (`ECP5_PROJECT_SCRIPT`)
//! for the project window.
//! `hover FILE LINE COL` reads published values without probing or watching.
//! `source FILE LINE COL` probes the editor buffer at one-based character
//! coordinates. FILE is relative to the project and may contain spaces.
//! It waits for the after-ready index independently of `emulate`'s ready wait.
//!
//! The editor queues `# lpf insert RESOURCE PORT LINES` for a successful Insert
//! pin action. RESOURCE is clock/leds/buttons/switches/uart, PORT is the chosen
//! base name (UART bit 0 is TX, bit 1 is RX), and LINES counts inserted LPF
//! commands. Rejected duplicates queue nothing. After the editor panel's
//! show_values call, ProjectApp drains them with:
//! `for note in self.editor.take_notes() { self.record(&note); }`.
//!
//! `--tour` uses window state rather than script delays. Both records receive
//! `# tour N STEP CAPTION` (six steps, then `6 done`), `# tour skip build REASON`,
//! `# tour done SECONDS`, `# tour stopped at STEP`, or `# tour failed STEP REASON`.
//! A skip also records the fallback caption as `# tour 1 build CAPTION`.
//! Step notes contain the entry caption; changes within a step add
//! `# tour caption N CAPTION`, including when the chip becomes ready.
//! `# tour pointer on probed wire X Y` marks the map pointer's 1.5 second hold,
//! at shares of the map's width and height.
//! Paths in tour notes are relative. A timeout ends the tour without advancing.
//! Without explicit record paths, tours write `tour/last-project.tsv` and
//! `tour/last-window.tsv` beside recent.txt. ECP5_PROJECT_TRACE and ECP5_GUI_TRACE
//! override those destinations; tour playback ignores both script variables.
use anyhow::{bail, Context, Result};
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

/// Active tab in the top project bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewTab {
    Project,
    Chip,
}

impl ViewTab {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Chip => "chip",
        }
    }
}

/// Tab-separated event log written when `ECP5_PROJECT_TRACE` is set.
pub struct ProjectTrace {
    out: BufWriter<File>,
    opened_at: Instant,
}

impl ProjectTrace {
    pub fn new(path: &Path) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self {
            out: BufWriter::new(file),
            opened_at: Instant::now(),
        })
    }

    pub fn record(&mut self, event: &str) {
        let t = self.opened_at.elapsed().as_secs_f64();
        let clean: String = event
            .chars()
            .map(|c| {
                if matches!(c, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let _ = writeln!(self.out, "{t:.3}\t{clean}");
        let _ = self.out.flush();
    }
}

/// A single command parsed from `ECP5_PROJECT_SCRIPT`.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptCommand {
    NewFile(super::new_file::Request),
    MemoryImage {
        path: PathBuf,
        use_as_image: bool,
    },
    Creation(crate::new_project::Command),
    Build,
    Rebuild,
    Emulate,
    View(ViewTab),
    Open {
        file: PathBuf,
        line: Option<u32>,
    },
    Source {
        file: PathBuf,
        line: u32,
        column: u32,
    },
    Hover {
        file: PathBuf,
        line: u32,
        column: u32,
    },
    ClickMessage(usize),
    Edit {
        file: PathBuf,
        line: u32,
        text: String,
    },
    Inline(bool),
    Type {
        file: PathBuf,
        line: u32,
        column: u32,
        text: String,
    },
    Save,
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptStep {
    pub delay_secs: f64,
    pub command: ScriptCommand,
}

/// Parses `<delay-seconds> <command>`; `#` starts a comment outside quoted strings.
pub fn parse_script(text: &str) -> Result<Vec<ScriptStep>> {
    let mut steps = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = crate::new_project::uncomment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let line_no = idx + 1;
        let (delay_str, rest) = line.split_once(char::is_whitespace).with_context(|| {
            format!("line {line_no}: expected '<delay> <command>', got {line:?}")
        })?;
        let delay_secs: f64 = delay_str
            .parse()
            .with_context(|| format!("line {line_no}: invalid delay {delay_str:?}"))?;
        if !delay_secs.is_finite() || delay_secs < 0.0 {
            bail!("line {line_no}: delay must be non-negative, got {delay_secs}");
        }
        let rest = rest.trim();
        let (word, args) = rest
            .split_once(char::is_whitespace)
            .map_or((rest, ""), |(w, a)| (w, a.trim_start()));
        let command = if let Some(command) =
            crate::new_project::Command::parse(rest).with_context(|| format!("line {line_no}"))?
        {
            ScriptCommand::Creation(command)
        } else {
            match word {
                "new-file" => ScriptCommand::NewFile(
                    super::new_file::Request::parse(args)
                        .with_context(|| format!("line {line_no}"))?,
                ),
                "memory-image" => {
                    let value: serde_json::Value =
                        serde_json::from_str(args).with_context(|| {
                            format!("line {line_no}: memory-image expects a JSON object")
                        })?;
                    let object = value
                        .as_object()
                        .context("memory-image expects a JSON object")?;
                    for key in object.keys() {
                        if key != "path" && key != "use" {
                            bail!("line {line_no}: unknown memory-image option {key:?}");
                        }
                    }
                    let path = object
                        .get("path")
                        .and_then(|v| v.as_str())
                        .context("memory-image needs a string path")?;
                    let use_as_image = object
                        .get("use")
                        .and_then(|v| v.as_bool())
                        .context("memory-image needs a boolean use")?;
                    ScriptCommand::MemoryImage {
                        path: PathBuf::from(path),
                        use_as_image,
                    }
                }
                "build" => ScriptCommand::Build,
                "rebuild" => ScriptCommand::Rebuild,
                "emulate" => ScriptCommand::Emulate,
                "view" => match args.trim() {
                    "project" => ScriptCommand::View(ViewTab::Project),
                    "chip" => ScriptCommand::View(ViewTab::Chip),
                    other => bail!("line {line_no}: expected 'view project|chip', got {other:?}"),
                },
                "open" => {
                    let mut parts = args.split_whitespace();
                    let file = parts
                        .next()
                        .with_context(|| format!("line {line_no}: 'open' needs a relative file"))?;
                    let line = parts
                        .next()
                        .map(|s| {
                            s.parse::<u32>()
                                .with_context(|| format!("line {line_no}: invalid line {s:?}"))
                        })
                        .transpose()?;
                    ScriptCommand::Open {
                        file: PathBuf::from(file),
                        line,
                    }
                }
                "source" | "hover" => {
                    let (before_column, column) = args
                        .trim()
                        .rsplit_once(char::is_whitespace)
                        .with_context(|| format!("line {line_no}: {word} needs FILE LINE COL"))?;
                    let (file, line) = before_column
                        .trim()
                        .rsplit_once(char::is_whitespace)
                        .with_context(|| format!("line {line_no}: {word} needs FILE LINE COL"))?;
                    let column =
                        column
                            .parse::<u32>()
                            .ok()
                            .filter(|n| *n > 0)
                            .with_context(|| {
                                format!("line {line_no}: {word} needs FILE LINE COL (one-based)")
                            })?;
                    let line = line
                        .parse::<u32>()
                        .ok()
                        .filter(|n| *n > 0)
                        .with_context(|| {
                            format!("line {line_no}: {word} needs FILE LINE COL (one-based)")
                        })?;
                    let file = PathBuf::from(file.trim().trim_matches('"'));
                    if file.as_os_str().is_empty()
                        || file.is_absolute()
                        || file
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                    {
                        bail!("line {line_no}: {word} needs a relative project file");
                    }
                    if word == "hover" {
                        ScriptCommand::Hover { file, line, column }
                    } else {
                        ScriptCommand::Source { file, line, column }
                    }
                }
                "click-message" => {
                    let index = args.trim().parse::<usize>().with_context(|| {
                        format!("line {line_no}: 'click-message' needs an index")
                    })?;
                    ScriptCommand::ClickMessage(index)
                }
                "edit" => {
                    let (file, after_file) =
                        args.split_once(char::is_whitespace).with_context(|| {
                            format!("line {line_no}: 'edit' needs '<file> <line> <text>'")
                        })?;
                    let after_file = after_file.trim_start();
                    let (line_str, text) = after_file
                        .split_once(char::is_whitespace)
                        .map_or((after_file, ""), |(l, t)| (l, t));
                    let target_line = line_str.parse::<u32>().with_context(|| {
                        format!("line {line_no}: invalid edit line {line_str:?}")
                    })?;
                    ScriptCommand::Edit {
                        file: PathBuf::from(file),
                        line: target_line,
                        text: text.to_string(),
                    }
                }
                "inline" => match args.trim() {
                    "on" => ScriptCommand::Inline(true),
                    "off" => ScriptCommand::Inline(false),
                    _ => bail!("line {line_no}: inline needs on|off"),
                },
                "type" => {
                    let take = |input: &str| -> (String, String) {
                        input
                            .trim_start()
                            .split_once(char::is_whitespace)
                            .map_or_else(
                                || (input.trim().into(), String::new()),
                                |(a, b)| (a.into(), b.into()),
                            )
                    };
                    let (file, rest) = take(args);
                    let (line, rest) = take(&rest);
                    let (column, text) = take(&rest);
                    let file = PathBuf::from(file);
                    let line = line.parse::<u32>().ok().filter(|n| *n > 0);
                    let column = column.parse::<u32>().ok().filter(|n| *n > 0);
                    let text = (!text.is_empty()).then_some(text);
                    if file.as_os_str().is_empty()
                        || file.is_absolute()
                        || file
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                    {
                        bail!("line {line_no}: type needs a relative project file");
                    }
                    ScriptCommand::Type {
                        file,
                        line: line.context("type needs FILE LINE COL TEXT (one-based)")?,
                        column: column.context("type needs FILE LINE COL TEXT (one-based)")?,
                        text: text.context("type needs FILE LINE COL TEXT")?,
                    }
                }
                "save" => ScriptCommand::Save,
                "quit" => ScriptCommand::Quit,
                other => bail!("line {line_no}: unknown script command {other:?}"),
            }
        };
        steps.push(ScriptStep {
            delay_secs,
            command,
        });
    }
    Ok(steps)
}

pub fn parse_script_file(path: &Path) -> Result<Vec<ScriptStep>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_script(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_and_type_commands_validate_coordinates_and_keep_text() {
        let steps =
            parse_script("0 inline on\n1 inline off\n0 type top.v   12   3 // λ comment").unwrap();
        assert_eq!(steps[0].command, ScriptCommand::Inline(true));
        assert_eq!(steps[1].command, ScriptCommand::Inline(false));
        assert_eq!(
            steps[2].command,
            ScriptCommand::Type {
                file: "top.v".into(),
                line: 12,
                column: 3,
                text: "// λ comment".into(),
            }
        );
        for line in [
            "0 inline yes",
            "0 type top.v 0 1 x",
            "0 type top.v 1 0 x",
            "0 type top.v 1 1",
            "0 type /tmp/top.v 1 1 x",
            "0 type ../top.v 1 1 x",
        ] {
            assert!(parse_script(line).is_err(), "{line}");
        }
    }

    #[test]
    fn hover_commands_share_source_coordinates_and_keep_paths_with_spaces() {
        let steps =
            parse_script("0 hover rtl/a file.v 42 9\n1 hover \"other file.v\" 2 3").unwrap();
        assert_eq!(
            steps[0].command,
            ScriptCommand::Hover {
                file: "rtl/a file.v".into(),
                line: 42,
                column: 9,
            }
        );
        assert_eq!(
            steps[1].command,
            ScriptCommand::Hover {
                file: "other file.v".into(),
                line: 2,
                column: 3,
            }
        );
        for args in [
            "top.v",
            "top.v 1 0",
            "top.v 0 1",
            "/tmp/top.v 1 2",
            "../top.v 1 2",
        ] {
            assert!(parse_script(&format!("0 hover {args}")).is_err());
        }
    }

    #[test]
    fn source_commands_keep_relative_files_with_spaces_and_one_based_coordinates() {
        let steps = parse_script("0 source top.v 13 12\n1 source rtl/cpu file.v  42   9 # comment")
            .unwrap();
        assert_eq!(
            steps[0].command,
            ScriptCommand::Source {
                file: "top.v".into(),
                line: 13,
                column: 12
            }
        );
        assert_eq!(
            steps[1].command,
            ScriptCommand::Source {
                file: "rtl/cpu file.v".into(),
                line: 42,
                column: 9
            }
        );
        for line in [
            "0 source top.v",
            "0 source top.v 1 0",
            "0 source top.v 0 1",
            "0 source /tmp/top.v 1 2",
            "0 source ../top.v 1 2",
        ] {
            assert!(parse_script(line).is_err(), "{line}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitCondition {
    Creation,
    Build,
    /// Emulate given during a build: the build, then the design's loading.
    Emulate,
    /// A design opened: until it is ready in the emulator, or failed.
    Ready,
    Sources,
    Typing,
}

/// Drives scripted commands, counting each command's delay from when the
/// previous command completed.
pub struct ScriptRunner {
    queue: VecDeque<ScriptStep>,
    completed_at: Instant,
    waiting: Option<WaitCondition>,
}

impl ScriptRunner {
    pub fn from_steps(steps: Vec<ScriptStep>) -> Self {
        Self {
            queue: steps.into(),
            completed_at: Instant::now(),
            waiting: None,
        }
    }

    pub fn waiting(&self) -> Option<WaitCondition> {
        self.waiting
    }

    pub fn set_waiting(&mut self, condition: WaitCondition) {
        self.waiting = Some(condition);
    }

    /// Marks the currently awaited build/emulate command complete so the
    /// next command's delay begins now.
    pub fn complete_wait(&mut self, now: Instant) {
        if self.waiting.take().is_some() {
            self.completed_at = now;
        }
    }

    /// Returns the next due command if not waiting and its delay has elapsed,
    /// or the remaining duration until the next command becomes due.
    pub fn poll_next(&mut self, now: Instant) -> std::result::Result<Option<ScriptCommand>, f64> {
        if self.waiting.is_some() {
            return Ok(None);
        }
        let Some(front) = self.queue.front() else {
            return Ok(None);
        };
        let elapsed = now.duration_since(self.completed_at).as_secs_f64();
        if elapsed >= front.delay_secs {
            let step = self.queue.pop_front().unwrap();
            self.completed_at = now;
            Ok(Some(step.command))
        } else {
            Err(front.delay_secs - elapsed)
        }
    }
}
