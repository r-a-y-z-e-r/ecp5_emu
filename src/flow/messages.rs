//! Live diagnostics with locations mapped back to original project files.
use super::{pipeline::Step, project::Project};
use std::{fmt, path::PathBuf};

/// Diagnostic importance, independent of a tool's exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}
impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        })
    }
}
/// The diagnostic's meaning, independent of its displayed wording.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageKind {
    #[default]
    Diagnostic,
    MissingTool,
}
/// A distinct diagnostic. `count` includes repeated occurrences in both pipes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: MessageKind,
    pub severity: Severity,
    pub step: Step,
    pub text: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub count: u32,
}
impl Message {
    pub(crate) fn new(step: Step, severity: Severity, text: impl Into<String>) -> Self {
        Self {
            kind: MessageKind::default(),
            severity,
            step,
            text: text.into(),
            file: None,
            line: None,
            column: None,
            count: 1,
        }
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.severity == other.severity
            && self.step == other.step
            && self.text == other.text
            && self.file == other.file
            && self.line == other.line
            && self.column == other.column
    }
}
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.file {
            write!(f, "{}:", path.display())?;
            if let Some(line) = self.line {
                write!(f, "{line}:")?;
                if let Some(column) = self.column {
                    write!(f, "{column}:")?;
                }
            }
            write!(f, " ")?;
        }
        write!(f, "{}: {}", self.severity, self.text)
    }
}
fn numbers(s: &str) -> Option<(u32, Option<u32>)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let line = s[..end].parse().ok()?;
    let column = s[end..]
        .strip_prefix(['.', ':'])
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok());
    Some((line, column))
}
fn location(project: &Project, text: &str) -> Option<(PathBuf, u32, Option<u32>)> {
    for path in project
        .hdl_roots()
        .iter()
        .chain(&project.files)
        .chain(&project.memory_images)
        .chain(std::iter::once(&project.constraints))
    {
        for spelling in [
            path.clone(),
            project.build_dir.join(path),
            project.build_dir.join(".hdl-synthesis").join(path),
            project.root.join(path),
        ] {
            let needle = format!("{}:", spelling.display());
            for (start, _) in text.match_indices(&needle) {
                // Do not match top.v inside another filename or an external absolute path.
                if start > 0 && !matches!(text.as_bytes()[start - 1], b' ' | b'(' | b'\'' | b'"') {
                    continue;
                }
                if let Some((line, column)) = numbers(&text[start + needle.len()..]) {
                    return Some((project.root.join(path), line, column));
                }
            }
        }
    }
    None
}
/// Parses one output line; normal progress output has no diagnostic.
pub fn parse(project: &Project, step: Step, line: &str) -> Option<Message> {
    let line = line.trim();
    if line.starts_with("Warnings:") {
        return None;
    }
    let mut fields = line.splitn(4, ':');
    let _file = fields.next();
    let ghdl_body = fields
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .and_then(|_| fields.next()?.parse::<u32>().ok())
        .and_then(|_| fields.next())
        .filter(|_| location(project, line).is_some());
    let (severity, text) = if let Some(body) = ghdl_body.filter(|_| step == Step::Synthesis) {
        let body = body.trim();
        if let Some(text) = body.strip_prefix("warning:") {
            (Severity::Warning, text.trim())
        } else if let Some(text) = body.strip_prefix("note:") {
            (Severity::Info, text.trim())
        } else {
            (
                Severity::Error,
                body.strip_prefix("error:").unwrap_or(body).trim(),
            )
        }
    } else if step == Step::Synthesis
        && (line.starts_with("Lexer warning:") || line.starts_with("ABC: Warning:"))
    {
        let user = location(project, line);
        let mut m = Message::new(
            step,
            if line.starts_with("Lexer") && user.is_some() {
                Severity::Warning
            } else {
                Severity::Info
            },
            line,
        );
        if let Some((file, line, column)) = user {
            m.file = Some(file);
            m.line = Some(line);
            m.column = column;
        }
        return Some(m);
    } else if let Some(s) = line
        .strip_prefix("ERROR:")
        .or_else(|| line.strip_prefix("Error:"))
    {
        (Severity::Error, s.trim())
    } else if let Some(s) = line
        .strip_prefix("Warning:")
        .or_else(|| line.strip_prefix("WARNING:"))
    {
        (Severity::Warning, s.trim())
    } else if step == Step::Synthesis {
        if let Some((_, s)) = line.split_once(": ERROR:") {
            (Severity::Error, s.trim())
        } else if let Some((_, s)) = line.split_once(": Warning:") {
            (Severity::Warning, s.trim())
        } else {
            return None;
        }
    } else {
        return None;
    };
    let mut m = Message::new(step, severity, text);
    if let Some((file, line, column)) = location(project, line) {
        m.file = Some(file);
        m.line = Some(line);
        m.column = column;
    }
    if step == Step::PlaceRoute {
        if let Some((_, rest)) = text.split_once("(on line ") {
            m.file = Some(project.root.join(&project.constraints));
            m.line = numbers(rest).map(|n| n.0);
        } else if text.contains("is unconstrained in LPF")
            || text.contains("Failed to parse LPF file")
        {
            m.file = Some(project.root.join(&project.constraints));
        }
    }
    Some(m)
}

/// Source excerpts, carets and indented frontend notes belong to the preceding diagnostic.
pub(crate) fn is_continuation(line: &str) -> bool {
    !line.trim().is_empty() && (line.starts_with(char::is_whitespace) || line.starts_with('^'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;
    #[test]
    fn missing_tool_kind_is_independent_of_text_and_deduplication() {
        let diagnostic = Message::new(Step::Synthesis, Severity::Error, "any wording");
        let mut missing = diagnostic.clone();
        missing.kind = MessageKind::MissingTool;
        assert!(crate::flow::tools::is_missing_tool(&missing));
        assert!(!crate::flow::tools::is_missing_tool(&diagnostic));
        assert!(!missing.same(&diagnostic));
        missing.severity = Severity::Warning;
        assert!(!crate::flow::tools::is_missing_tool(&missing));
    }
    #[test]
    fn real_diagnostic_fixtures() {
        let f = Folder::new();
        std::fs::write(f.0.join("syn.v"), "").unwrap();
        std::fs::write(f.0.join("warn.v"), "").unwrap();
        let p = Project::open(&f.0).unwrap();
        let cases = [
            (Step::Synthesis, "syn.v:4: ERROR: syntax error, unexpected TOK_ASSIGN", Severity::Error, Some("syn.v"), Some(4)),
            (Step::Synthesis, "warn.v:4: Warning: Identifier `\\undeclared_thing' is implicitly declared.", Severity::Warning, Some("warn.v"), Some(4)),
            (Step::Synthesis, "Warning: Wire top.\\undeclared_thing is used but has no driver.", Severity::Warning, None, None),
            (Step::Synthesis, "Lexer warning: The SystemVerilog keyword `bit' (at /opt/homebrew/Cellar/yosys/0.70/bin/../share/yosys/abc9_map.v:35) is not recognized unless read_verilog is called with -sv!", Severity::Info, None, None),
            (Step::Synthesis, "ABC: Warning: The network is combinational.", Severity::Info, None, None),
            (Step::Synthesis, "ERROR: Module `nosuch' not found!", Severity::Error, None, None),
            (Step::PlaceRoute, "ERROR: IO pin 'clk_25mhz$tr_io' constrained to pin 'ZZ9', which does not exist for package 'CABGA381'.", Severity::Error, None, None),
            (Step::PlaceRoute, "ERROR: Packing design failed.", Severity::Error, None, None),
            (Step::PlaceRoute, "ERROR: IO 'led[7]' is unconstrained in LPF (override this error with --lpf-allow-unconstrained)", Severity::Error, Some("top.lpf"), None),
            (Step::PlaceRoute, "ERROR: unexpected input following LOCATE clause (on line 2)", Severity::Error, Some("top.lpf"), Some(2)),
            (Step::PlaceRoute, "ERROR: Failed to parse LPF file 'syn.lpf'", Severity::Error, Some("top.lpf"), None),
            (Step::PlaceRoute, "ERROR: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (FAIL at 900.09 MHz)", Severity::Error, None, None),
            (Step::Bitstream, "Error: example", Severity::Error, None, None),
        ];
        for (step, line, severity, file, number) in cases {
            let m = parse(&p, step, line).unwrap();
            assert_eq!(m.severity, severity, "{line}");
            assert_eq!(m.file, file.map(|s| p.root.join(s)), "{line}");
            assert_eq!(m.line, number, "{line}");
        }
        assert!(parse(&p, Step::Synthesis, "Warnings: 2 unique messages, 3 total").is_none());
        assert!(parse(&p, Step::Bitstream, "Failed to open input file").is_none());
        assert!(parse(&p, Step::PlaceRoute, "Info: Program finished normally.").is_none());
        for text in ["syn.v:4.2-5.3: ERROR: bad", "Warning: bad (at syn.v:4.2)"] {
            let m = parse(&p, Step::Synthesis, text).unwrap();
            assert_eq!(m.file, Some(p.root.join("syn.v")));
            assert_eq!(m.line, Some(4));
            assert_eq!(m.column, Some(2));
        }
        let m = parse(
            &p,
            Step::Synthesis,
            &format!("{}:4: ERROR: bad", p.build_dir.join("syn.v").display()),
        )
        .unwrap();
        assert_eq!(m.file, Some(p.root.join("syn.v")));
        assert_eq!(
            m.to_string(),
            format!("{}:4: error: bad", p.root.join("syn.v").display())
        );
    }
}

#[cfg(test)]
mod suite_tests {
    use super::*;
    use crate::flow::test_support::Folder;
    #[test]
    fn ghdl_and_slang_diagnostics() {
        let f = Folder::new();
        for name in ["bad.vhd", "bad.sv"] {
            std::fs::write(f.0.join(name), "").unwrap();
        }
        for (name, text, severity, column) in [
            (
                "bad.vhd",
                "bad.vhd:8:13:error: primary expression expected",
                Severity::Error,
                Some(13),
            ),
            (
                "bad.vhd",
                "bad.vhd:8:13: primary expression expected",
                Severity::Error,
                Some(13),
            ),
            (
                "bad.vhd",
                "bad.vhd:8:13:warning: unused signal",
                Severity::Warning,
                Some(13),
            ),
            (
                "bad.sv",
                "bad.sv:2: ERROR: expected expression",
                Severity::Error,
                None,
            ),
        ] {
            std::fs::write(
                f.0.join("ecp5.toml"),
                format!("[sources]\nhdl=['{name}']\n"),
            )
            .unwrap();
            let p = Project::open(&f.0).unwrap();
            let m = parse(&p, Step::Synthesis, text).unwrap();
            assert_eq!(m.file, Some(p.root.join(name)));
            assert_eq!(m.line, Some(if name == "bad.vhd" { 8 } else { 2 }));
            assert_eq!(m.column, column);
            assert_eq!(m.severity, severity);
            let absolute = text.replacen(
                name,
                &p.build_dir
                    .join(".hdl-synthesis")
                    .join(name)
                    .to_string_lossy(),
                1,
            );
            assert_eq!(parse(&p, Step::Synthesis, &absolute).unwrap(), m);
        }
        assert!(is_continuation("  broken <= ;"));
        assert!(is_continuation("            ^"));
        assert!(!is_continuation("ERROR: vhdl import failed."));
    }
}
