//! Geometry on disk retains every field and rebuilds damaged files.
mod common;
use common::reference;
use ecp5_emu::{
    board,
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fabric_overlay::OverlayKey,
    image::ConfigurationImage,
    kernel,
    layout::{self, Dot, Geometry, GeometryKey, LogicTile, Segment, SegmentKind, Site, SiteKind},
    live,
    logic::Logic,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    peripherals::Attached,
    prepared::{load_catalogue, ConfiguredDevice},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = Self(std::env::temp_dir().join(format!(
            "ecp5_emu_geometry_test_{}_{}",
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

fn sample() -> Geometry {
    Geometry {
        rows: 3,
        cols: 4,
        segments: vec![Segment {
            a: [1.0, 2.0],
            b: [2.0, 2.0],
            kind: SegmentKind::Track,
            value: 0,
            net: 0,
            d0: 0.0,
            d1: 1.0,
        }],
        dots: vec![Dot {
            at: [2.0, 2.0],
            value: 0,
            net: 0,
            d: 1.0,
        }],
        sites: vec![Site {
            min: [1.4, 2.3],
            max: [2.0, 3.0],
            kind: SiteKind::Pio,
            name: "PIO".into(),
            used: true,
        }],
        logic: vec![LogicTile {
            row: 2,
            col: 1,
            slices: 1,
            luts: 1,
            flip_flops: 1,
            f: [0; 8],
            q: [layout::NO_VALUE; 8],
        }],
        values: vec![Wire {
            location: Location { row: 2, col: 1 },
            name: "H01E0000".into(),
        }],
        longest: 1.0,
    }
}

#[test]
fn geometry_cache_round_trips_every_field_and_checks_the_envelope() {
    let dir = TempDir::new();
    let path = dir.0.join("geometry.bin");
    let original = sample();
    let encoded = layout::encode_cache(&original).unwrap();
    std::fs::write(&path, &encoded).unwrap();
    let decoded = layout::decode_cache(&std::fs::read(path).unwrap()).unwrap();
    assert!(decoded == original);
    assert_eq!(layout::encode_cache(&decoded).unwrap(), encoded);
    for at in [0, 8, 12, 20, encoded.len() - 1] {
        let mut bad = encoded.clone();
        bad[at] ^= 0xff;
        assert!(layout::decode_cache(&bad).is_none());
    }
    for len in [0, 8, 51, encoded.len() / 2, encoded.len() - 1] {
        assert!(layout::decode_cache(&encoded[..len]).is_none());
    }
    // A matching checksum does not make invalid encoding or trailing bytes valid.
    for mut bad in [encoded[..52].to_vec(), encoded] {
        bad.push(0xff);
        let len = (bad.len() - 52) as u64;
        bad[12..20].copy_from_slice(&len.to_le_bytes());
        let checksum: [u8; 32] = Sha256::digest(&bad[52..]).into();
        bad[20..52].copy_from_slice(&checksum);
        assert!(layout::decode_cache(&bad).is_none());
    }
}

#[test]
fn geometry_key_tracks_compile_identity_and_format() {
    let dir = TempDir::new();
    let first = GeometryKey::new(&[1; 32]);
    assert_eq!(first, GeometryKey::new(&[1; 32]));
    let changed = GeometryKey::new(&[2; 32]);
    assert_ne!(first, changed);
    assert_ne!(first.cache_path(&dir.0), changed.cache_path(&dir.0));
    assert_eq!(
        first.cache_path(&dir.0),
        OverlayKey::new(&[1; 32]).cache_path(&dir.0)
    );
    let mut hash = Sha256::new();
    hash.update(b"ecp5-design-view-v2\0");
    hash.update(2u32.to_le_bytes());
    hash.update([1u8; 32]);
    let expected: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        first.cache_path(&dir.0),
        dir.0.join(format!("{expected}.bin"))
    );
}

fn cache_file(dir: &TempDir, timestamp: u64, size: u64) -> PathBuf {
    let path = dir.0.join(format!("{timestamp}.bin"));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(size).unwrap();
    file.set_modified(
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(timestamp),
    )
    .unwrap();
    path
}

#[test]
fn geometry_cache_prunes_by_size_and_keeps_oversized_newest() {
    // Isolate environment overrides from the other tests in this process.
    if std::env::var_os("ECP5_GEOMETRY_CACHE_TEST_CHILD").is_none() {
        for limit in [
            None,
            Some("1"),
            Some("0"),
            Some("invalid"),
            Some("18446744073709551615"),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "geometry_cache_prunes_by_size_and_keeps_oversized_newest",
                ])
                .env("ECP5_GEOMETRY_CACHE_TEST_CHILD", "1");
            if let Some(limit) = limit {
                command.env("ECP5_GEOMETRY_CACHE_MB", limit);
            } else {
                command.env_remove("ECP5_GEOMETRY_CACHE_MB");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        return;
    }
    let dir = TempDir::new();
    let older = cache_file(&dir, 1, 1);
    let middle = cache_file(&dir, 2, 624 * 1024);
    let newest = cache_file(&dir, 3, 400 * 1024);
    let unrelated = dir.0.join("notes.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    layout::prune_cache(&dir.0);
    let limit = std::env::var("ECP5_GEOMETRY_CACHE_MB").unwrap_or_default();
    assert!(newest.exists() && unrelated.exists());
    assert_eq!(middle.exists(), limit != "0");
    assert_eq!(older.exists(), limit != "0" && limit != "1");
    // Sparse files test the default cap without allocating that disk space.
    if limit != "0" && limit != "1" {
        cache_file(&dir, 2, 124 * 1024 * 1024);
        cache_file(&dir, 3, 132 * 1024 * 1024);
        layout::prune_cache(&dir.0);
        assert!(newest.exists() && middle.exists());
        assert!(!older.exists());
    }
    let oversized = cache_file(&dir, 4, 256 * 1024 * 1024 + 1);
    layout::prune_cache(&dir.0);
    assert!(oversized.exists() && unrelated.exists());
    assert!(!newest.exists() && !middle.exists() && !older.exists());
}

#[test]
fn blinky_geometry_matches_sequential() {
    round_trip(
        "blinky_geometry_matches_sequential",
        "blinky_cycle",
        reference::prepared("blinky_cycle"),
    );
}

#[test]
#[ignore = "builds picorv32_console on demand (place and route, minutes); requires clang, native image reader and local Trellis database"]
fn picorv32_geometry_matches_sequential() {
    round_trip(
        "picorv32_geometry_matches_sequential",
        "picorv32_console",
        reference::design("picorv32_console"),
    );
}

#[test]
#[ignore = "life_full geometry comparison; release only, native image reader and Trellis database"]
fn life_full_geometry_cache_round_trip_rebuild_and_timings() {
    round_trip(
        "life_full_geometry_cache_round_trip_rebuild_and_timings",
        "life_full",
        reference::design("life_full"),
    );
}

fn round_trip(test: &str, design: &str, root: PathBuf) {
    // Each fixture owns its environment overrides in a separate process.
    if std::env::var_os("ECP5_GEOMETRY_COMPARE_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--include-ignored", "--nocapture"])
            .env("ECP5_GEOMETRY_COMPARE_CHILD", "1")
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
    let used: BTreeMap<_, _> = board::used_balls(&device).unwrap().into_iter().collect();
    let pin = |ball: &str| Reference::Pin(ball.into());
    let attached = Attached::new(&used);
    let mut drives = vec![(pin(board::CLOCK), Logic::Zero)];
    for button in board::BUTTONS {
        if used.get(button.ball).is_some_and(|d| d.receives()) {
            drives.push((pin(button.ball), Logic::from_bool(button.idle)));
        }
    }
    if used.get(board::UART_RX).is_some_and(|d| d.receives()) {
        drives.push((pin(board::UART_RX), Logic::One));
    }
    for (ball, level) in attached.input_balls() {
        drives.push((pin(ball), level.map_or(Logic::Z, Logic::from_bool)));
    }
    let request = Request {
        package: board::PACKAGE.into(),
        scope: Scope::Observed,
        startup: Startup::Configured,
        drives,
        probes: used
            .iter()
            .filter(|(_, d)| d.drives())
            .map(|(b, _)| pin(b))
            .chain(attached.balls().into_iter().map(pin))
            .collect(),
        options: kernel::Options {
            trace_capacity: 0,
            ..kernel::Options::default()
        },
    };
    let board = compile::Board::default();
    let compile_key = compile::cache_key(&device, &request, &board).unwrap();
    let mut key_times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let identity = compile::cache_key(&device, &request, &board).unwrap();
        std::hint::black_box((GeometryKey::new(&identity), OverlayKey::new(&identity)));
        key_times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    key_times.sort_by(f64::total_cmp);
    println!(
        "{design} compile identity+view keys: median={:.3} ms",
        key_times[2]
    );
    for change_drive in [false, true] {
        let mut changed = request.clone();
        if change_drive {
            changed.drives[0].1 = Logic::One;
        } else {
            changed.probes.push(Reference::Pin(board::CLOCK.into()));
        }
        let other = compile::cache_key(&device, &changed, &board).unwrap();
        assert_ne!(GeometryKey::new(&compile_key), GeometryKey::new(&other));
        assert_ne!(OverlayKey::new(&compile_key), OverlayKey::new(&other));
    }
    let compiled = compile::compile_on_board(device, request, &board).unwrap();
    let mut hash_times = Vec::new();
    let mut report_bytes = 0;
    for _ in 0..5 {
        let start = Instant::now();
        let bytes = postcard::to_stdvec(&compiled.report).unwrap();
        std::hint::black_box(Sha256::digest(&bytes));
        hash_times.push(start.elapsed().as_secs_f64() * 1e3);
        report_bytes = bytes.len();
    }
    hash_times.sort_by(f64::total_cmp);
    println!("{design} report postcard+SHA256: bytes={report_bytes}, median={:.3} ms, range={:.3}..{:.3} ms", hash_times[2], hash_times[0], hash_times[4]);
    let dir = TempDir::new();
    let key = GeometryKey::new(&compile_key);
    let path = key.cache_path(&dir.0);
    std::env::remove_var("ECP5_GEOMETRY_CACHE");
    let t0 = Instant::now();
    let original = layout::build_uncached(&model, &compiled.report);
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;
    let logic_since = Instant::now();
    let logic = layout::logic_usage(&model, &compiled.report);
    let logic_ms = logic_since.elapsed().as_secs_f64() * 1e3;
    assert_eq!(logic.len(), original.logic.len());
    for (usage, full) in logic.iter().zip(&original.logic) {
        assert_eq!(
            (
                usage.row,
                usage.col,
                usage.slices,
                usage.luts,
                usage.flip_flops
            ),
            (full.row, full.col, full.slices, full.luts, full.flip_flops)
        );
        assert_eq!(usage.f, [layout::NO_VALUE; 8]);
        assert_eq!(usage.q, [layout::NO_VALUE; 8]);
    }
    assert!(!original.segments.is_empty() && !original.logic.is_empty());
    let t1 = Instant::now();
    let encoded = layout::encode_cache(&original).unwrap();
    std::fs::write(&path, &encoded).unwrap();
    let write_ms = t1.elapsed().as_secs_f64() * 1e3;
    if let Some(reference) = std::env::var_os("ECP5_GEOMETRY_REFERENCE") {
        let bytes = std::fs::read(reference).unwrap();
        assert!(
            layout::decode_cache(&bytes).is_some_and(|g| g == original),
            "geometry differs from the original sequential build"
        );
        assert!(
            bytes == encoded,
            "geometry cache differs from the original build"
        );
    }
    let concurrent_since = Instant::now();
    let concurrent = std::thread::scope(|scope| {
        let geometry = scope.spawn(|| layout::build_uncached(&model, &compiled.report));
        let usage = layout::logic_usage(&model, &compiled.report);
        assert!(usage == logic, "concurrent early logic use differs");
        geometry.join().unwrap()
    });
    let concurrent_ms = concurrent_since.elapsed().as_secs_f64() * 1e3;
    assert!(
        concurrent == original,
        "concurrent build changed a geometry field"
    );
    assert!(
        layout::encode_cache(&concurrent).unwrap() == encoded,
        "concurrent cache encoding differs"
    );
    drop(concurrent);
    println!("geometry {design}: sequential={build_ms:.1} ms, concurrent={concurrent_ms:.1} ms, logic_use={logic_ms:.1} ms; fields and cache bytes identical");
    std::fs::remove_file(&path).unwrap();
    let written = layout::cached_in(&model, &compiled.report, Some(&key), &dir.0);
    assert!(written == original, "cache miss changed a geometry field");
    assert!(
        std::fs::read(&path).unwrap() == encoded,
        "cache miss file differs"
    );
    drop(written);
    let t2 = Instant::now();
    let bytes = std::fs::read(&path).unwrap();
    let decoded = layout::decode_cache(&bytes).unwrap();
    let read_ms = t2.elapsed().as_secs_f64() * 1e3;
    assert!(decoded == original, "round trip changed a geometry field");
    drop(decoded);
    drop(bytes);
    let cached = || layout::cached_in(&model, &compiled.report, Some(&key), &dir.0);
    let t3 = Instant::now();
    let hit = cached();
    let hit_ms = t3.elapsed().as_secs_f64() * 1e3;
    assert!(hit == original, "cache hit changed a geometry field");
    drop(hit);
    println!(
        "geometry cache {design}: size={} bytes ({:.2} MiB), build={build_ms:.1} ms, write={write_ms:.1} ms, read={read_ms:.1} ms, cached_hit={hit_ms:.1} ms",
        encoded.len(), encoded.len() as f64 / (1024.0 * 1024.0),
    );
    for truncated in [false, true] {
        let mut bad = encoded.clone();
        if truncated {
            bad.truncate(bad.len() / 2);
        } else {
            let last = bad.len() - 1;
            bad[last] ^= 0xff;
        }
        assert!(layout::decode_cache(&bad).is_none());
        std::fs::write(&path, bad).unwrap();
        let rebuilt = cached();
        assert!(rebuilt == original, "rebuild changed a geometry field");
        let disk = std::fs::read(&path).unwrap();
        assert!(layout::decode_cache(&disk).is_some_and(|g| g == original));
        assert!(disk == encoded, "rebuilt cache file differs");
    }
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        1,
        "temporary files left behind"
    );
}
