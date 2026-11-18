//! Manual release-mode benchmark of the window's request, with plan snapshots.
//!
//! Uncached phase timings without per-call instrumentation (`cargo test` sets
//! `cfg(test)`; no Cargo feature is needed):
//! ```sh
//! ECP5_COMPILE_CACHE=0 cargo test --release --lib compile::timing::window_compile -- --ignored --nocapture
//! ```
//! Opt into the detailed function profiler:
//! ```sh
//! ECP5_COMPILE_CACHE=0 ECP5_TIMING_PROFILE=1 cargo test --release --features compile-timing --lib compile::timing::window_compile -- --ignored --nocapture
//! ```
//! `ECP5_TIMING_DESIGNS=life_full,blinky_cycle,blinky_pll,ram32` selects designs;
//! `ECP5_TIMING_OUTPUT=/tmp/compile-before` saves reports, cache payloads and
//! complete test-only kernel snapshots. Compare two folders with
//! `python3 tools/compare_compile.py /tmp/compile-before /tmp/compile-after`.
//! Normal builds exclude this module. In test builds the per-call sites and
//! thread-local profiler exist only with `compile-timing`; the default benchmark
//! retains just the phase-boundary clocks.
use super::*;
use crate::{
    board, database::Database, image::ConfigurationImage, model::DeviceModel,
    peripherals::Attached, prepared::load_catalogue,
};
use std::{path::PathBuf, time::Instant};

// All callers and the profiler itself are absent without the measurement feature.
#[cfg(feature = "compile-timing")]
mod profile {
    use super::*;
    use std::{cell::RefCell, time::Duration};
    #[derive(Default)]
    struct Profile {
        enabled: bool,
        stack: Vec<Duration>,
        times: BTreeMap<&'static str, (Duration, usize)>,
    }
    thread_local! { static PROFILE: RefCell<Profile> = RefCell::default(); }
    pub(crate) struct Timer(Option<(&'static str, Instant)>);
    pub(crate) fn enter(name: &'static str) -> Timer {
        Timer(PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            if !p.enabled {
                return None;
            }
            p.stack.push(Duration::ZERO);
            Some((name, Instant::now()))
        }))
    }
    impl Drop for Timer {
        fn drop(&mut self) {
            let Some((name, start)) = self.0 else {
                return;
            };
            let elapsed = start.elapsed();
            PROFILE.with(|p| {
                let mut p = p.borrow_mut();
                let children = p.stack.pop().unwrap();
                if let Some(parent) = p.stack.last_mut() {
                    *parent += elapsed;
                }
                let entry = p.times.entry(name).or_default();
                entry.0 += elapsed.saturating_sub(children);
                entry.1 += 1;
            });
        }
    }

    pub(super) fn start() {
        PROFILE.with(|p| {
            *p.borrow_mut() = Profile {
                enabled: std::env::var_os("ECP5_TIMING_PROFILE").is_some(),
                ..Profile::default()
            }
        });
    }
    pub(super) fn finish(design: &str) {
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            p.enabled = false;
            let mut times: Vec<_> = p.times.iter().collect();
            times.sort_by_key(|(_, (time, _))| std::cmp::Reverse(*time));
            for (name, (time, calls)) in times.iter().take(10) {
                eprintln!(
                    "{design} hotspot {name} {:.6} {calls} calls",
                    time.as_secs_f64()
                );
            }
        });
    }
}
#[cfg(feature = "compile-timing")]
pub(crate) use profile::enter;

#[test]
#[ignore = "loads full reference designs; run alone in release mode"]
fn window_compile() -> Result<()> {
    let evidence = crate::live::architecture();
    let db = Database::discover(None)?;
    let start = Instant::now();
    let model = Arc::new(DeviceModel::load(&db, &evidence)?);
    eprintln!("model_load {:.6}", start.elapsed().as_secs_f64());
    let designs = std::env::var("ECP5_TIMING_DESIGNS").unwrap_or_else(|_| "life_full".into());
    for design in designs.split(',') {
        let root = crate::test_designs::reference_design(design);
        let start = Instant::now();
        let image = Arc::new(ConfigurationImage::load(
            &root.join("top.config"),
            &db,
            None,
        )?);
        eprintln!("{design} image_load {:.6}", start.elapsed().as_secs_f64());
        let start = Instant::now();
        let catalogue = load_catalogue(&model, &evidence)?;
        eprintln!(
            "{design} catalogue_load {:.6}",
            start.elapsed().as_secs_f64()
        );
        let start = Instant::now();
        let device = Arc::new(ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            catalogue,
        )?);
        eprintln!("{design} bind_decode {:.6}", start.elapsed().as_secs_f64());
        // Keep this request in step with live::work, including peripheral inputs.
        let start = Instant::now();
        let used: BTreeMap<_, _> = board::used_balls(&device)?.into_iter().collect();
        let receives = |ball: &str| used.get(ball).is_some_and(|d| d.receives());
        let pin = |ball: &str| Reference::Pin(ball.into());
        ensure!(receives(board::CLOCK), "design does not receive the clock");
        let mut drives = vec![(pin(board::CLOCK), Logic::Zero)];
        for button in &board::BUTTONS {
            if receives(button.ball) {
                drives.push((pin(button.ball), Logic::from_bool(button.idle)));
            }
        }
        if receives(board::UART_RX) {
            drives.push((pin(board::UART_RX), Logic::One));
        }
        for (ball, level) in Attached::new(&used).input_balls() {
            drives.push((pin(ball), level.map_or(Logic::Z, Logic::from_bool)));
        }
        let probes = used
            .iter()
            .filter(|(_, d)| d.drives())
            .map(|(b, _)| pin(b))
            .collect();
        let request = Request {
            package: board::PACKAGE.into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives,
            probes,
            options: kernel::Options {
                trace_capacity: 0,
                ..kernel::Options::default()
            },
        };
        eprintln!("{design} pins {:.6}", start.elapsed().as_secs_f64());
        let start = Instant::now();
        let mut last = start;
        #[cfg(feature = "compile-timing")]
        profile::start();
        let options = request.options;
        let mut compiled =
            compile_on_board_phases(Arc::clone(&device), request, &Board::default(), |phase| {
                let now = Instant::now();
                eprintln!(
                    "{design} {phase} {:.6}",
                    now.duration_since(last).as_secs_f64()
                );
                last = now;
            })?;
        eprintln!(
            "{design} compile_total {:.6}",
            start.elapsed().as_secs_f64()
        );
        #[cfg(feature = "compile-timing")]
        profile::finish(design);
        if let Some(folder) = std::env::var_os("ECP5_TIMING_OUTPUT") {
            let folder = PathBuf::from(folder);
            std::fs::create_dir_all(&folder)?;
            std::fs::write(
                folder.join(format!("{design}.json")),
                serde_json::to_vec(&compiled.report)?,
            )?;
            std::fs::write(
                folder.join(format!("{design}.kernel.bin")),
                kernel::cache::snapshot::encode(&compiled)?,
            )?;
            if let Some(bytes) = kernel::cache::encode(&compiled) {
                std::fs::write(folder.join(format!("{design}.plan.bin")), bytes)?;
                let _ = std::fs::remove_file(folder.join(format!("{design}.plan.unavailable")));
            } else {
                std::fs::write(
                    folder.join(format!("{design}.plan.unavailable")),
                    "production cache encoder does not support this kernel; compare the full test snapshot\n",
                )?;
                let _ = std::fs::remove_file(folder.join(format!("{design}.plan.bin")));
            }
        }
        if std::env::var_os("ECP5_TIMING_VERIFY_CACHE").is_some() {
            if let Some(bytes) = kernel::cache::encode(&compiled) {
                let mut restored = kernel::cache::decode(device, options, &bytes)?;
                let foreign = *compiled.probes.values().next().context("no probes")?;
                ensure!(
                    restored.kernel.value(foreign).is_err(),
                    "cached signal reused a foreign scope"
                );
                let foreign = *compiled.drivers.values().next().context("no drives")?;
                ensure!(
                    restored
                        .kernel
                        .schedule(kernel::Time::from_ps(1), foreign, Logic::Zero)
                        .is_err(),
                    "cached driver reused a foreign scope"
                );
                ensure!(
                    kernel::cache::encode(&restored).as_deref() == Some(&bytes),
                    "kernel snapshot differs after cache read"
                );
                eprintln!("{design} kernel_snapshot_identical {} bytes", bytes.len());
                // Compare evolving state as well as the complete initial plan.
                for edge in 1..=8 {
                    for c in [&mut compiled, &mut restored] {
                        c.kernel.schedule(
                            kernel::Time::from_ps(edge * 20_000),
                            c.drivers["pin:G2"],
                            Logic::from_bool(edge % 2 == 1),
                        )?;
                        c.kernel.run_until(kernel::Time::from_ps(edge * 20_000))?;
                    }
                    ensure!(
                        serde_json::to_vec(&compiled.kernel.state().summary())?
                            == serde_json::to_vec(&restored.kernel.state().summary())?,
                        "cache state diverged"
                    );
                    for (name, &signal) in &compiled.probes {
                        ensure!(
                            compiled.kernel.value(signal)?
                                == restored.kernel.value(restored.probes[name])?,
                            "cache probe diverged at {name}"
                        );
                    }
                }
            } else {
                eprintln!("{design} cache_fallback");
            }
        }
    }
    Ok(())
}
