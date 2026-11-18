//! The window's actual headless runner, through ready and the first values.
//! Run one release process at a time on the Mac:
//! ECP5_LOAD_TIMES=1 ECP5_OPEN_OUTPUT=/tmp/open-after cargo test --release --features compile-timing --test open_sequence -- --ignored --nocapture
//! A new executable identity gives the first-open cache misses of a new build.
//! ECP5_OPEN_DESIGNS=life_full,life_full selects first and repeated opens.
//! ECP5_OPEN_EXT=bit selects bitstreams; config is the default.
#![cfg(feature = "compile-timing")]

use anyhow::{ensure, Result};
use ecp5_emu::live::{self, Design, Shared};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "full window load sequence; release only, one process at a time"]
fn window_open_sequence() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Like the start screen, keep the chip resident across design opens.
    live::prepare_chip();
    let chip = Instant::now();
    while live::chip_fabric().is_none() {
        ensure!(
            chip.elapsed() < Duration::from_secs(120),
            "chip load timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("chip resident {:.6}", chip.elapsed().as_secs_f64());
    let designs =
        std::env::var("ECP5_OPEN_DESIGNS").unwrap_or_else(|_| "life_full,life_full".into());
    let extension = std::env::var("ECP5_OPEN_EXT").unwrap_or_else(|_| "config".into());
    for (open, design) in designs.split(',').enumerate() {
        let config = root.join(format!("target/reference/{design}/top.{extension}"));
        let routed = config.with_file_name("top_routed.json");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let observed = Arc::clone(&shared);
        let (updates, receive) = mpsc::channel();
        let since = Instant::now();
        let commands = live::spawn(
            Design {
                names: routed.is_file().then_some(routed),
                config,
                run: false,
            },
            Arc::clone(&shared),
            move || {
                let s = observed.lock().unwrap();
                let _ = updates.send((since.elapsed().as_secs_f64(), s.stage.clone()));
            },
        );
        let mut stages = Vec::new();
        let mut previous = String::new();
        let elapsed = loop {
            if let Ok((seconds, stage)) = receive.recv_timeout(Duration::from_millis(20)) {
                if stage != previous {
                    eprintln!("{design}/{extension}/{open} # load {seconds:.6} {stage}");
                    previous.clone_from(&stage);
                    stages.push((seconds, stage));
                }
            }
            let s = shared.lock().unwrap();
            ensure!(s.error.is_none(), "live runner: {:?}", s.error);
            if s.ready.is_some() && s.fast.is_some() && !s.lit.is_empty() {
                break since.elapsed().as_secs_f64();
            }
            ensure!(
                since.elapsed() < Duration::from_secs(300),
                "load timed out: {}",
                s.stage
            );
        };
        let drawing = stages
            .iter()
            .position(|(_, s)| s.starts_with("Drawing the die's fabric"))
            .expect("drawing stage");
        for (i, (_, stage)) in stages.iter().enumerate() {
            if stage.starts_with("Choosing fast mode's settings")
                || stage.starts_with("Starting with")
            {
                ensure!(i > drawing, "settings started before drawing");
            }
        }
        eprintln!("{design}/{extension}/{open} # load settings/tuner deferred until geometry and overlay completed; see ECP5_LOAD_TIMES boundary");
        let statics = shared.lock().unwrap().ready.clone().unwrap();
        eprintln!("{design}/{extension}/{open} image_to_ready {elapsed:.6}");
        if let Some(folder) = std::env::var_os("ECP5_OPEN_OUTPUT") {
            let folder = PathBuf::from(folder);
            std::fs::create_dir_all(&folder)?;
            let prefix = format!("{design}-{extension}-{open}");
            std::fs::write(
                folder.join(format!("{prefix}-times.json")),
                serde_json::to_vec_pretty(&serde_json::json!({"stages":stages,"total":elapsed}))?,
            )?;
            std::fs::write(
                folder.join(format!("{prefix}-geometry.bin")),
                ecp5_emu::layout::encode_cache(&statics.geometry)?,
            )?;
            std::fs::write(
                folder.join(format!("{prefix}-overlay.bin")),
                postcard::to_stdvec(&statics.overlay)?,
            )?;
            let names: std::collections::BTreeMap<_, _> = statics.net_names.iter().collect();
            std::fs::write(
                folder.join(format!("{prefix}-net-names.json")),
                serde_json::to_vec(&names)?,
            )?;
        }
        drop(commands);
        // The next open must not compete with the preceding runner's teardown.
        let stopping = Instant::now();
        while Arc::strong_count(&shared) != 1 {
            ensure!(
                stopping.elapsed() < Duration::from_secs(30),
                "runner did not stop"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}
