//! One native reader check shared by the tour and every design opening.
use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, OnceLock},
};

pub const MISSING: &str = "Image reader missing. See README Requirements.";
pub const BUILDING: &str =
    "Building the missing image reader with tools/build_image_reader.py before opening a design";
const REQUIREMENTS: &str = " See README Requirements.";

pub fn is_caption(text: &str) -> bool {
    text == BUILDING || text.ends_with(REQUIREMENTS.trim_start())
}

fn message(detail: &str) -> String {
    let punctuation = if detail.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    super::display_text(&format!("{detail}{punctuation}{REQUIREMENTS}"))
}

pub fn script_error(stderr: &str) -> String {
    let last = stderr.lines().rev().find(|line| line.contains("error:"));
    let detail = last
        .or_else(|| {
            stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with("usage:"))
        })
        .map(str::trim)
        .unwrap_or("Image reader build failed");
    message(
        detail
            .strip_prefix("build_image_reader.py: error: ")
            .unwrap_or(detail),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Building,
    Ready,
    Missing(String),
}

struct Job {
    status: Status,
    built: bool,
    attempted: bool,
}

pub struct Check {
    job: Arc<Mutex<Job>>,
    notes: Vec<String>,
    reported: bool,
    pub build_requested: bool,
}

struct Builder {
    root: PathBuf,
    reader: PathBuf,
    program: PathBuf,
    script: PathBuf,
}

impl Builder {
    fn run(&self) -> Result<(), String> {
        let output = Command::new(&self.program)
            .arg(&self.script)
            .arg("--output")
            .arg(&self.reader)
            .current_dir(&self.root)
            .output()
            .map_err(|error| message(&format!("python3 not found: {error}")))?;
        if !output.status.success() {
            return Err(script_error(&String::from_utf8_lossy(&output.stderr)));
        }
        if !self.reader.is_file() {
            return Err(message("Image reader build did not produce the executable"));
        }
        Ok(())
    }
}

impl Check {
    pub fn new() -> Self {
        let reader = ecp5_emu::image::reader_path(None);
        type SharedJob = Arc<Mutex<Job>>;
        static JOB: OnceLock<Mutex<Option<SharedJob>>> = OnceLock::new();
        let mut slot = JOB.get_or_init(|| Mutex::new(None)).lock().unwrap();
        if let Some(job) = slot
            .as_ref()
            .filter(|job| job.lock().unwrap().status == Status::Building)
        {
            return Self::watch(Arc::clone(job));
        }
        if reader.is_file() {
            return Self::watch(Arc::new(Mutex::new(Job {
                status: Status::Ready,
                built: false,
                attempted: false,
            })));
        }
        // Only the build is shared. A later open also sees a reader the user
        // built by hand after following the missing-reader message.
        if slot
            .as_ref()
            .is_none_or(|job| !job.lock().unwrap().attempted)
        {
            *slot = Some(Self::start(Builder {
                root: super::repository_root(),
                reader: reader.clone(),
                program: "python3".into(),
                script: "tools/build_image_reader.py".into(),
            }));
        }
        let job = slot.as_ref().unwrap();
        {
            let mut state = job.lock().unwrap();
            if state.status == Status::Ready && !reader.is_file() {
                state.status = Status::Missing(MISSING.into());
                state.built = false;
            }
        }
        Self::watch(Arc::clone(job))
    }

    fn start(builder: Builder) -> Arc<Mutex<Job>> {
        let found = builder.reader.is_file();
        let job = Arc::new(Mutex::new(Job {
            status: if found {
                Status::Ready
            } else {
                Status::Building
            },
            built: false,
            attempted: !found,
        }));
        if !found {
            let job = Arc::clone(&job);
            std::thread::spawn(move || {
                let result = builder.run();
                let mut job = job.lock().unwrap();
                job.built = result.is_ok();
                job.status = result.map_or_else(Status::Missing, |_| Status::Ready);
            });
        }
        job
    }

    fn watch(job: Arc<Mutex<Job>>) -> Self {
        let state = job.lock().unwrap();
        let building = state.status == Status::Building;
        let initial = if state.status == Status::Ready {
            if state.built {
                "reader built"
            } else {
                "reader found"
            }
        } else {
            "reader missing"
        };
        let reported = !building;
        let notes = if state.built {
            vec!["reader missing".into(), "reader built".into()]
        } else {
            vec![initial.into()]
        };
        let build_requested = state.attempted;
        drop(state);
        Self {
            job,
            notes,
            reported,
            build_requested,
        }
    }

    pub fn status(&self) -> Status {
        self.job.lock().unwrap().status.clone()
    }

    #[cfg(test)]
    pub fn finish(&self, result: Result<(), String>) {
        let mut job = self.job.lock().unwrap();
        job.built = result.is_ok();
        job.status = result.map_or_else(Status::Missing, |_| Status::Ready);
    }

    pub fn notes(&mut self) -> Vec<String> {
        let state = self.job.lock().unwrap();
        if !self.reported && state.status != Status::Building {
            if state.built {
                self.notes.push("reader built".into());
            }
            self.reported = true;
        }
        std::mem::take(&mut self.notes)
    }
}

#[cfg(test)]
pub fn fixed(status: Status) -> Check {
    Check::watch(Arc::new(Mutex::new(Job {
        attempted: status == Status::Building,
        status,
        built: false,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::Path,
        time::{Duration, Instant},
    };

    const ERROR: &str = "prjtrellis source not found; use --trellis PATH or TRELLIS_ROOT (or ECP5_DB pointing into a source checkout); configure --trellis/TRELLIS_ROOT (or ECP5_DB) and --prefix/TRELLIS_PREFIX";

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            Self(
                crate::test_temp::unique_dir("ecp5-reader-")
                    .canonicalize()
                    .unwrap(),
            )
        }
        fn builder(&self, program: &str, script: &str) -> Builder {
            Builder {
                root: self.0.clone(),
                reader: self.0.join("reader"),
                program: program.into(),
                script: script.into(),
            }
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn finished(check: &Check) -> Status {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = check.status();
            if status != Status::Building {
                return status;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn explicit_reader_uses_the_emulators_resolution() {
        let path = Path::new("chosen/reader");
        assert_eq!(ecp5_emu::image::reader_path(Some(path)), path);
    }

    #[test]
    #[cfg(unix)]
    fn the_worker_runs_the_injected_builder_from_the_repository_with_only_output() {
        let root = Temporary::new();
        fs::write(
            root.0.join("success.sh"),
            r#"printf '%s\n' "$@" > args.txt
printf '%s\n' "$PWD" > cwd.txt
printf reader > "$2"
"#,
        )
        .unwrap();
        let job = Check::start(root.builder("/bin/sh", "success.sh"));
        let mut check = Check::watch(Arc::clone(&job));
        let mut other = Check::watch(job);
        assert_eq!(finished(&check), Status::Ready);
        assert_eq!(check.notes(), ["reader missing", "reader built"]);
        assert!(check.notes().is_empty());
        assert_eq!(other.notes(), ["reader missing", "reader built"]);
        assert_eq!(
            fs::read_to_string(root.0.join("args.txt")).unwrap(),
            format!("--output\n{}\n", root.0.join("reader").display())
        );
        assert_eq!(
            fs::read_to_string(root.0.join("cwd.txt")).unwrap().trim(),
            root.0.to_str().unwrap()
        );
        let mut found = Check::watch(Check::start(
            root.builder("missing-python3", "must-not-run.py"),
        ));
        assert_eq!(found.status(), Status::Ready);
        assert_eq!(found.notes(), ["reader found"]);
        assert!(!found.build_requested);
    }

    #[test]
    #[cfg(unix)]
    fn the_worker_preserves_the_last_script_error_and_rejects_nonzero_exit_with_output() {
        let root = Temporary::new();
        fs::write(root.0.join("stderr.txt"), format!("usage: build_image_reader.py [-h]\nbuild_image_reader.py: error: earlier error\nbuild_image_reader.py: error: {ERROR}\n")).unwrap();
        fs::write(
            root.0.join("failure.sh"),
            r#"cat stderr.txt >&2
printf partial > "$2"
exit 2
"#,
        )
        .unwrap();
        let mut check = Check::watch(Check::start(root.builder("/bin/sh", "failure.sh")));
        assert_eq!(
            finished(&check),
            Status::Missing(format!("{ERROR}.{REQUIREMENTS}"))
        );
        assert_eq!(check.notes(), ["reader missing"]);
        assert!(
            root.0.join("reader").is_file(),
            "non-zero exit fails even if a file was produced"
        );
    }

    #[test]
    fn a_missing_python_is_reported_with_the_requirements_pointer() {
        let root = Temporary::new();
        let mut check = Check::watch(Check::start(
            root.builder("/nonexistent/ecp5-python3", "tools/build_image_reader.py"),
        ));
        let Status::Missing(error) = finished(&check) else {
            panic!("the program cannot start");
        };
        assert!(error.starts_with("python3 not found: "), "{error}");
        assert!(error.ends_with(REQUIREMENTS));
        assert_eq!(check.notes(), ["reader missing"]);
    }

    #[test]
    fn the_requirements_pointer_follows_a_complete_sentence() {
        for (detail, expected) in [
            ("missing prefix", "missing prefix. See README Requirements."),
            (
                "missing prefix.",
                "missing prefix. See README Requirements.",
            ),
            (
                "missing prefix!",
                "missing prefix! See README Requirements.",
            ),
            (
                "missing prefix?",
                "missing prefix? See README Requirements.",
            ),
        ] {
            assert_eq!(
                script_error(&format!("build_image_reader.py: error: {detail}")),
                expected
            );
        }
    }

    #[test]
    fn script_errors_drop_usage_and_the_argparse_prefix_and_hide_home_paths() {
        let home = crate::test_temp::home_dir().display().to_string();
        let error = script_error(&format!("usage: build_image_reader.py [-h]\nbuild_image_reader.py: error: earlier error\nbuild_image_reader.py: error: {ERROR} (invalid path: {home}/FPGA/prjtrellis)\n"));
        assert!(error.starts_with(ERROR));
        assert!(error.ends_with(REQUIREMENTS));
        assert!(error.contains("~/FPGA/prjtrellis"));
        assert!(!error.contains(&home));
        assert!(!error.contains("usage:") && !error.contains("build_image_reader.py: error:"));
        assert!(is_caption(&error));
    }
}
