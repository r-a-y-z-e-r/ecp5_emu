//! Paths shown in the start screen, project window and prerequisite messages.
use std::path::{Path, PathBuf};

/// Formats paths and paths embedded in tool output without exposing the home folder.
/// The filesystem paths remain untouched; only strings passed to widgets use this.
#[derive(Clone)]
pub struct DisplayPaths {
    cwd: Option<PathBuf>,
    home: Option<PathBuf>,
    root: Option<PathBuf>,
    prefixes: Vec<(String, &'static str)>,
}

impl DisplayPaths {
    pub fn new(root: Option<&Path>) -> Self {
        #[cfg(not(test))]
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        #[cfg(test)]
        let home = Some(crate::test_temp::home_dir());
        Self::with_environment(std::env::current_dir().ok(), home, root)
    }

    pub(crate) fn with_environment(
        cwd: Option<PathBuf>,
        home: Option<PathBuf>,
        root: Option<&Path>,
    ) -> Self {
        let mut paths = Self {
            cwd,
            home,
            root: None,
            prefixes: Vec::new(),
        };
        paths.set_root(root);
        paths
    }

    pub(crate) fn set_root(&mut self, root: Option<&Path>) {
        self.root = root.map(Path::to_path_buf);
        self.prefixes.clear();
        for (base, replacement) in [(&self.cwd, "."), (&self.home, "~"), (&self.root, ".")] {
            let Some(base) = base else { continue };
            for base in [Some(base.clone()), base.canonicalize().ok()]
                .into_iter()
                .flatten()
            {
                let prefix = base.to_string_lossy().into_owned();
                if !prefix.is_empty() && !self.prefixes.iter().any(|(p, _)| p == &prefix) {
                    self.prefixes.push((prefix, replacement));
                }
            }
        }
    }

    pub fn path(&self, path: &Path) -> String {
        self.text(&path.to_string_lossy())
    }

    pub fn text(&self, text: &str) -> String {
        let mut text = text.to_owned();
        // Prefixes are ordered by cwd, home, then project root. Replace only
        // complete path components, also in tool output with spaces in paths.
        for (prefix, replacement) in &self.prefixes {
            let mut rest = text.as_str();
            let mut result = String::new();
            while let Some(at) = rest.find(prefix.as_str()) {
                let tail = &rest[at + prefix.len()..];
                result.push_str(&rest[..at]);
                if let Some(tail) = tail.strip_prefix(std::path::MAIN_SEPARATOR) {
                    if *replacement == "~" {
                        result.push_str("~/");
                    }
                    rest = tail;
                } else {
                    let boundary = tail
                        .chars()
                        .next()
                        .is_none_or(|c| c.is_whitespace() || ":'\"),;]}`".contains(c));
                    result.push_str(if boundary { replacement } else { prefix });
                    rest = tail;
                }
            }
            result.push_str(rest);
            text = result;
        }
        text
    }

    pub(crate) fn expand(&self, input: &str) -> PathBuf {
        self.expand_path(Path::new(input.trim()))
    }

    pub(crate) fn expand_path(&self, input: &Path) -> PathBuf {
        if let (Some(home), Ok(relative)) = (&self.home, input.strip_prefix("~")) {
            return home.join(relative);
        }
        input.to_path_buf()
    }
}
