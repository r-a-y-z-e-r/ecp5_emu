//! The design overlay survives disk storage with all its private indices.
mod common;
use common::reference;
use ecp5_emu::{
    board,
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fabric,
    fabric_overlay::{self, OverlayKey},
    image::ConfigurationImage,
    kernel, layout, live,
    logic::Logic,
    model::{topology::Wire, DeviceModel},
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = Self(std::env::temp_dir().join(format!(
            "ecp5_emu_overlay_test_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        std::fs::create_dir(&dir.0).unwrap();
        dir
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Each override runs in its own process so parallel tests do not share it.
fn run_with_cache_limit(test: &str, limit: Option<&str>) -> bool {
    if std::env::var_os("ECP5_OVERLAY_CACHE_TEST_CHILD").is_some() {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test])
        .env("ECP5_OVERLAY_CACHE_TEST_CHILD", "1");
    if let Some(limit) = limit {
        command.env("ECP5_OVERLAY_CACHE_MB", limit);
    } else {
        command.env_remove("ECP5_OVERLAY_CACHE_MB");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    true
}

fn cache_file(dir: &TempDir, age: u64, size: usize) -> PathBuf {
    let path = dir.0.join(format!("{age}.bin"));
    std::fs::write(&path, vec![0u8; size]).unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(age))
        .unwrap();
    path
}

#[test]
fn overlay_cache_keeps_newest_files_under_size_cap() {
    if run_with_cache_limit("overlay_cache_keeps_newest_files_under_size_cap", Some("1")) {
        return;
    }
    let dir = TempDir::new();
    let oldest = cache_file(&dir, 1, 1);
    let older = cache_file(&dir, 2, 1);
    let middle = cache_file(&dir, 3, 624 * 1024);
    let newest = cache_file(&dir, 4, 400 * 1024);
    let unrelated = dir.0.join("notes.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    fabric_overlay::prune_cache(&dir.0);
    assert!(newest.exists() && middle.exists());
    assert!(!older.exists() && !oldest.exists());
    assert!(unrelated.exists());
    // An older small file cannot displace a newer file that did not fit.
    std::fs::write(&middle, vec![0u8; 625 * 1024]).unwrap();
    cache_file(&dir, 1, 1);
    fabric_overlay::prune_cache(&dir.0);
    assert!(middle.exists());
    assert!(!newest.exists() && !oldest.exists());
}

#[test]
fn overlay_cache_keeps_one_oversized_newest_file() {
    if run_with_cache_limit("overlay_cache_keeps_one_oversized_newest_file", Some("1")) {
        return;
    }
    let dir = TempDir::new();
    let older = cache_file(&dir, 1, 1);
    let empty = cache_file(&dir, 2, 0);
    let newest = cache_file(&dir, 3, 1024 * 1024 + 1);
    fabric_overlay::prune_cache(&dir.0);
    assert!(newest.exists());
    assert!(!older.exists() && !empty.exists());
}

#[test]
fn overlay_cache_size_limit_override() {
    if std::env::var_os("ECP5_OVERLAY_CACHE_TEST_CHILD").is_none() {
        for limit in [
            None,
            Some("2"),
            Some("0"),
            Some("invalid"),
            Some("18446744073709551615"),
        ] {
            assert!(run_with_cache_limit(
                "overlay_cache_size_limit_override",
                limit
            ));
        }
        return;
    }
    let dir = TempDir::new();
    for i in 0..6 {
        cache_file(&dir, i, 256 * 1024);
    }
    fabric_overlay::prune_cache(&dir.0);
    let expected = if std::env::var("ECP5_OVERLAY_CACHE_MB").as_deref() == Ok("0") {
        1
    } else {
        6
    };
    assert!(dir.0.join("5.bin").exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), expected);
}

#[test]
fn overlay_key_tracks_compile_identity() {
    let dir = TempDir::new();
    let first = OverlayKey::new(&[1; 32]);
    assert_eq!(first, OverlayKey::new(&[1; 32]));
    let changed = OverlayKey::new(&[2; 32]);
    assert_ne!(first, changed);
    assert_ne!(first.cache_path(&dir.0), changed.cache_path(&dir.0));
}

fn round_trip(test: &str, design: &str, root: PathBuf, rebuild: bool) {
    if std::env::var_os("ECP5_OVERLAY_COMPARE_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--include-ignored", "--nocapture"])
            .env("ECP5_OVERLAY_COMPARE_CHILD", "1")
            .status()
            .unwrap()
            .success());
        return;
    }
    let config = root.join("top.config");
    let architecture = live::architecture();
    let db = Database::discover(None).unwrap();
    let model = Arc::new(DeviceModel::load(&db, &architecture).unwrap());
    let image = Arc::new(ConfigurationImage::load(&config, &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &architecture).unwrap(),
        )
        .unwrap(),
    );
    let used = board::used_balls(&device).unwrap();
    let mut drives = vec![(Reference::Pin(board::CLOCK.into()), Logic::Zero)];
    for button in board::BUTTONS {
        if used.iter().any(|(b, d)| b == button.ball && d.receives()) {
            drives.push((
                Reference::Pin(button.ball.into()),
                Logic::from_bool(button.idle),
            ));
        }
    }
    if used
        .iter()
        .any(|(b, d)| b == board::UART_RX && d.receives())
    {
        drives.push((Reference::Pin(board::UART_RX.into()), Logic::One));
    }
    let request = Request {
        package: board::PACKAGE.into(),
        scope: Scope::Observed,
        startup: Startup::Configured,
        drives,
        probes: used
            .into_iter()
            .filter(|(_, d)| d.drives())
            .map(|(b, _)| Reference::Pin(b))
            .collect(),
        options: kernel::Options {
            trace_capacity: 0,
            ..kernel::Options::default()
        },
    };
    let board = compile::Board::default();
    let compile_key = compile::cache_key(&device, &request, &board).unwrap();
    let compiled = compile::compile_on_board(device, request, &board).unwrap();
    let geometry = layout::build(&model, &compiled.report);
    let fabric = fabric::cached(&model, &architecture);
    let report = &compiled.report;
    let dir = TempDir::new();
    let key = OverlayKey::new(&compile_key);
    let path = key.cache_path(&dir.0);

    // These fixture tests run in an isolated child, so overrides cannot race
    // another overlay or a live loader in this process.
    std::env::set_var("ECP5_OVERLAY_THREADS", "1");
    std::env::remove_var("ECP5_OVERLAY_CACHE");
    let t0 = Instant::now();
    let original = fabric_overlay::build(
        &fabric,
        &model,
        &report.routes,
        &report.constants,
        &geometry.logic,
        &report.fields,
    );
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;
    std::env::remove_var("ECP5_OVERLAY_THREADS");
    assert!(!original.pieces.is_empty());
    if let Some(path) = std::env::var_os("ECP5_OVERLAY_REFERENCE") {
        let reference = fabric_overlay::decode_cache(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            reference == original,
            "overlay differs from original global graph build"
        );
        println!("overlay {design}: matches the original global graph build field by field");
    }
    let t1 = Instant::now();
    let encoded = fabric_overlay::encode_cache(&original).unwrap();
    std::fs::write(&path, &encoded).unwrap();
    let write_ms = t1.elapsed().as_secs_f64() * 1e3;
    let t2 = Instant::now();
    let bytes = std::fs::read(&path).unwrap();
    let decoded = fabric_overlay::decode_cache(&bytes).expect("valid overlay cache");
    let read_ms = t2.elapsed().as_secs_f64() * 1e3;
    assert!(decoded == original, "round trip changed an overlay field");
    drop(decoded);
    drop(bytes);

    let cached = || {
        fabric_overlay::cached_in(
            &fabric,
            &model,
            &report.routes,
            &report.constants,
            &geometry.logic,
            &report.fields,
            Some(&key),
            &dir.0,
        )
    };
    std::fs::remove_file(&path).unwrap();
    let logic = layout::logic_usage(&model, report);
    let parallel_since = Instant::now();
    let parallel = fabric_overlay::build(
        &fabric,
        &model,
        &report.routes,
        &report.constants,
        &logic,
        &report.fields,
    );
    let parallel_ms = parallel_since.elapsed().as_secs_f64() * 1e3;
    assert!(
        parallel == original,
        "parallel build with early logic use changed an overlay field"
    );
    assert!(
        postcard::to_stdvec(&parallel).unwrap() == postcard::to_stdvec(&original).unwrap(),
        "parallel postcard differs"
    );
    let written = cached();
    assert!(written == original, "cache miss changed an overlay field");
    assert!(
        std::fs::read(&path).unwrap() == encoded,
        "parallel cache file differs"
    );
    drop(written);
    println!("overlay {design}: sequential={build_ms:.1} ms, parallel={parallel_ms:.1} ms; fields, postcard and cache bytes identical");
    drop(parallel);
    let t3 = Instant::now();
    let hit = cached();
    let hit_ms = t3.elapsed().as_secs_f64() * 1e3;
    assert!(hit == original, "cache hit changed an overlay field");
    drop(hit);
    println!(
        "overlay cache {design}: pieces={}, size={} bytes ({:.2} MiB), build={build_ms:.1} ms, write={write_ms:.1} ms, read={read_ms:.1} ms, cached_hit={hit_ms:.1} ms",
        original.pieces.len(), encoded.len(), encoded.len() as f64 / (1024.0 * 1024.0),
    );

    for byte in [0, 8, 12, 20, encoded.len() - 1] {
        let mut bad = encoded.clone();
        bad[byte] ^= 0xff;
        assert!(fabric_overlay::decode_cache(&bad).is_none());
    }
    for len in [0, 8, 51, encoded.len() / 2, encoded.len() - 1] {
        assert!(fabric_overlay::decode_cache(&encoded[..len]).is_none());
    }
    // A valid checksum cannot make an invalid encoding or trailing data valid.
    for mut bad in [encoded[..52].to_vec(), encoded.clone()] {
        bad.push(0xff);
        let len = (bad.len() - 52) as u64;
        bad[12..20].copy_from_slice(&len.to_le_bytes());
        let checksum: [u8; 32] = Sha256::digest(&bad[52..]).into();
        bad[20..52].copy_from_slice(&checksum);
        assert!(fabric_overlay::decode_cache(&bad).is_none());
    }
    if matches!(design, "blinky_cycle" | "picorv32_console") {
        check_live(design, &root, &original);
    }
    if rebuild {
        for truncated in [false, true] {
            let mut bad = encoded.clone();
            if truncated {
                bad.truncate(bad.len() / 2);
            } else {
                let last = bad.len() - 1;
                bad[last] ^= 0xff;
            }
            std::fs::write(&path, bad).unwrap();
            let rebuilt = cached();
            assert!(rebuilt == original, "rebuild changed an overlay field");
            let disk = std::fs::read(&path).unwrap();
            assert!(
                fabric_overlay::decode_cache(&disk).is_some_and(|o| o == rebuilt),
                "rebuilt overlay was not fully rewritten"
            );
            println!(
                "overlay cache {design}: rejected and rebuilt {} file",
                if truncated { "truncated" } else { "corrupted" }
            );
        }
    }
}

#[test]
#[ignore = "requires the life reference design, native image reader and local Trellis database"]
fn overlay_cache_round_trips_and_rebuilds_bad_files() {
    round_trip(
        "overlay_cache_round_trips_and_rebuilds_bad_files",
        "life",
        reference::design("life"),
        true,
    );
}

#[test]
#[ignore = "requires the life_full reference design, native image reader and local Trellis database"]
fn life_full_overlay_cache_timings() {
    round_trip(
        "life_full_overlay_cache_timings",
        "life_full",
        reference::design("life_full"),
        false,
    );
}

#[test]
fn blinky_parallel_overlay_matches_sequential() {
    round_trip(
        "blinky_parallel_overlay_matches_sequential",
        "blinky_cycle",
        reference::prepared("blinky_cycle"),
        false,
    );
}

#[test]
#[ignore = "requires the PicoRV32 reference design, clang, native image reader and local Trellis database"]
fn picorv32_parallel_overlay_matches_sequential() {
    round_trip(
        "picorv32_parallel_overlay_matches_sequential",
        "picorv32_console",
        reference::design("picorv32_console"),
        false,
    );
}

fn check_live(design: &str, root: &Path, expected: &fabric_overlay::Overlay) {
    use std::{collections::HashMap, sync::Mutex, time::Duration};
    let shared = Arc::new(Mutex::new(live::Shared::default()));
    let commands = live::spawn(
        live::Design {
            config: root.join("top.config"),
            names: Some(root.join("top_routed.json")),
            run: false,
        },
        Arc::clone(&shared),
        || {},
    );
    let since = Instant::now();
    let statics = loop {
        let s = shared.lock().unwrap();
        assert!(s.error.is_none(), "live error: {:?}", s.error);
        if s.fast.is_some() && !s.lit.is_empty() {
            break s.ready.clone().expect("ready static");
        }
        assert!(
            since.elapsed() < Duration::from_secs(120),
            "live timeout: {}",
            s.stage
        );
        drop(s);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(statics.overlay == *expected, "live Static overlay differs");
    let names = Names::load(
        &root.join("top_routed.json"),
        &statics.model,
        board::PACKAGE,
    )
    .unwrap();
    let mut net_names: HashMap<u32, Vec<String>> = HashMap::new();
    for (name, reference) in names.observations() {
        let wires: Vec<(Wire, String)> = match reference {
            Reference::Wire(w) => vec![(w.clone(), name.into())],
            Reference::Pin(ball) => statics
                .model
                .package_pin(board::PACKAGE, ball)
                .and_then(|p| p.pio)
                .map(|id| {
                    statics
                        .model
                        .resource_ports(id)
                        .into_iter()
                        .map(|p| (p.wire, format!("{name} ({})", p.name)))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for (wire, label) in wires {
            if let Some(v) = expected.value_of(&statics.model, &wire) {
                net_names
                    .entry(expected.nets[v as usize])
                    .or_default()
                    .push(label);
            }
        }
    }
    for list in net_names.values_mut() {
        list.sort();
        list.dedup();
    }
    assert_eq!(statics.net_names, net_names, "live Static net_names differ");
    drop(commands);
    let since = Instant::now();
    while Arc::strong_count(&shared) != 1 {
        assert!(
            since.elapsed() < Duration::from_secs(30),
            "live runner did not stop"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "live {design}: ready, Static net_names/overlay identical, fast running, stopped cleanly"
    );
}
