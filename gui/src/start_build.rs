//! Build a start-screen demo without blocking the window.
use ecp5_emu::flow::{self, BuildOptions, Event, Severity, Tools};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Instant,
};

#[derive(Debug, Default)]
pub struct State {
    task: Option<Task>,
    pub status: String,
    failed: bool,
}

#[derive(Debug)]
struct Task {
    events: mpsc::Receiver<Event>,
    done: mpsc::Receiver<Result<PathBuf, String>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    step: String,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn caption(&self) -> String {
        self.task.as_ref().map_or_else(
            || self.status.clone(),
            |task| {
                format!(
                    "{} · {:.1} s",
                    task.step,
                    task.started.elapsed().as_secs_f64()
                )
            },
        )
    }

    pub fn start(&mut self, root: PathBuf, ctx: &egui::Context) {
        if self.running() {
            return;
        }
        let (send, events) = mpsc::channel();
        let (finish, done) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let options = BuildOptions {
            force: false,
            tools: Tools::from_env(),
            cancel: Some(Arc::clone(&cancel)),
        };
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = flow::Project::open(root)
                .map_err(|e| format!("{e:#}"))
                .and_then(|project| {
                    let outcome = flow::build(&project, &options, &mut |event| {
                        let _ = send.send(event);
                        ctx.request_repaint();
                    });
                    build_result(outcome)
                });
            let _ = finish.send(result);
            ctx.request_repaint();
        });
        self.failed = false;
        self.status.clear();
        self.task = Some(Task {
            events,
            done,
            cancel,
            started: Instant::now(),
            step: "Opening project".into(),
        });
    }

    pub fn poll(&mut self) -> Option<(Result<PathBuf, String>, f64)> {
        let task = self.task.as_mut()?;
        for event in task.events.try_iter() {
            if let Event::Started { step, .. } | Event::Skipped { step } = event {
                task.step = match step {
                    flow::Step::Synthesis => "Synthesis",
                    flow::Step::PlaceRoute => "Place and route",
                    flow::Step::Swap => "Memory images",
                    flow::Step::Bitstream => "Bitstream",
                }
                .into();
            }
        }
        let result = match task.done.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("Build worker stopped".into()),
        };
        let seconds = task.started.elapsed().as_secs_f64();
        self.failed = result.is_err();
        self.status = result
            .as_ref()
            .map_or_else(|error| error.clone(), |_| format!("Built · {seconds:.1} s"));
        self.task = None;
        Some((result, seconds))
    }

    #[cfg(test)]
    pub fn preview(status: &str, running: bool, failed: bool) -> Self {
        let mut state = Self {
            status: status.into(),
            failed,
            task: None,
        };
        if running {
            let (_, events) = mpsc::channel();
            let (_, done) = mpsc::channel();
            state.task = Some(Task {
                events,
                done,
                cancel: Arc::new(AtomicBool::new(false)),
                started: Instant::now(),
                step: status.into(),
            });
        }
        state
    }
}

fn build_result(outcome: flow::Outcome) -> Result<PathBuf, String> {
    if outcome.ok {
        Ok(outcome.outputs.config)
    } else {
        Err(outcome
            .messages
            .iter()
            .find(|m| m.severity == Severity::Error)
            .map(ToString::to_string)
            .unwrap_or_else(|| "Build failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    #[test]
    fn the_first_build_error_keeps_its_location_and_success_uses_the_flow_output() {
        let project = flow::Project::open(
            crate::start::repository_root().join("examples/ulx3s/blinky_cycle"),
        )
        .unwrap();
        let message = |severity, text: &str| flow::Message {
            kind: Default::default(),
            severity,
            text: text.into(),
            file: Some(project.root.join("top.v")),
            line: Some(7),
            column: Some(3),
            count: 1,
            step: flow::Step::Synthesis,
        };
        let mut outcome = flow::Outcome {
            ok: false,
            cancelled: false,
            steps: Vec::new(),
            report: None,
            messages: vec![
                message(Severity::Warning, "warning"),
                message(Severity::Error, "first error"),
                message(Severity::Error, "second error"),
            ],
            outputs: project.outputs(),
        };
        let error = build_result(outcome.clone()).unwrap_err();
        assert!(error.ends_with("top.v:7:3: error: first error"));
        let shown = crate::start::display_text(&error);
        assert!(!shown.contains(&crate::start::repository_root().display().to_string()));
        outcome.ok = true;
        assert_eq!(build_result(outcome).unwrap(), project.outputs().config);
    }

    #[test]
    fn an_invalid_demo_finishes_on_the_worker_and_can_be_retried() {
        let root = std::env::temp_dir().join(format!("ecp5-start-worker-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ecp5.toml"), "invalid = [").unwrap();
        let mut state = State::default();
        for _ in 0..2 {
            state.start(root.clone(), &egui::Context::default());
            assert!(state.running());
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                if let Some(result) = state.poll() {
                    break result;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            };
            assert!(result.0.is_err());
            assert!(result.1 >= 0.0);
            assert!(!state.running());
            assert!(state.failed());
            assert!(!state.caption().is_empty());
            assert!(state.poll().is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
