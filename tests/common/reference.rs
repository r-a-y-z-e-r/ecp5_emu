//! Routed reference designs, shared safely by independent test processes.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
};

/// Locate a conventional reference fixture prepared by CI or the user.
#[allow(dead_code)] // src/test_designs.rs also includes this module and only uses build.
pub fn prepared(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/reference")
        .join(name);
    assert!(
        root.join("top.config").is_file(),
        "target/reference/{name} is not prepared: bash packaging/build_test_fixtures.sh"
    );
    root
}

/// Build a reference on demand, except the explicitly prepared full-chip Life.
#[allow(dead_code)] // src/test_designs.rs also includes this module and only uses build.
pub fn design(name: &str) -> PathBuf {
    if name != "life_full" {
        return build(name);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reference/life_full");
    assert!(
        ["top.config", "top_routed.json"]
            .iter()
            .all(|name| root.join(name).is_file()),
        "target/reference/life_full is not built (about 40 minutes of place and route): examples/ulx3s/build.sh examples/ulx3s/life_full target/reference/life_full"
    );
    root
}

/// Build a name under examples/ulx3s, or a design folder with prepared inputs.
pub fn build(design: impl AsRef<Path>) -> PathBuf {
    static BUILT: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let design = design.as_ref();
    let design = if design.components().count() == 1 {
        root.join("examples/ulx3s").join(design)
    } else {
        root.join(design)
    };
    let name = design.file_name().unwrap().to_str().unwrap();
    // Hold the lock through the build and publication, so tests in this
    // binary build each set of inputs only once.
    let mut built = BUILT.get_or_init(Mutex::default).lock().unwrap();
    // Hash every file by its relative name and contents, including memory
    // images, constraints and any patched inputs in a prepared folder.
    let mut inputs = Vec::new();
    collect(&design, &mut inputs);
    inputs.sort();
    let mut hash = Sha256::new();
    for path in inputs {
        hash_file(&mut hash, path.strip_prefix(&design).unwrap(), &path);
    }
    // build.sh calls swap.sh for designs with ecpbram.txt.
    for script in ["examples/ulx3s/build.sh", "examples/ulx3s/swap.sh"] {
        hash_file(&mut hash, Path::new(script), &root.join(script));
    }
    let hash: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let reference = root.join("target/reference");
    let out = reference.join(format!("{name}-{hash}"));
    if built.contains(&out) && complete(&out) {
        return out;
    }
    if !complete(&out) {
        fs::create_dir_all(&reference).unwrap();
        let temporary = loop {
            let path = reference.join(format!(
                "{name}-{hash}.tmp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("creating {}: {e}", path.display()),
            }
        };
        let log_path = temporary.join("build.log");
        let log = fs::File::create(&log_path).unwrap();
        let status = Command::new(root.join("examples/ulx3s/build.sh"))
            .arg(&design)
            .arg(&temporary)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .status()
            .unwrap_or_else(|e| panic!("building {name}: {e}, logs in {}", log_path.display()));
        assert!(
            status.success() && complete(&temporary),
            "building {name}, logs in {}",
            log_path.display()
        );
        // Another process can finish the same inputs first. Its directory
        // is complete too; only the private, losing build is removed.
        if let Err(e) = fs::rename(&temporary, &out) {
            assert!(
                complete(&out),
                "publishing {name}: {e}, logs in {}",
                log_path.display()
            );
            fs::remove_dir_all(&temporary).unwrap();
        }
    }
    let log = fs::read_to_string(out.join("nextpnr.log")).unwrap_or_default();
    for line in log.lines().filter(|line| {
        [
            "Max frequency",
            "TRELLIS_SLICE:",
            "TRELLIS_IO:",
            "DP16KD:",
            "DCCA:",
            "Total LUT4s:",
            "TRELLIS_FF:",
            "TRELLIS_COMB:",
            "SIOLOGIC:",
            "EHXPLLL:",
        ]
        .iter()
        .any(|summary| line.contains(summary))
    }) {
        println!("{name}: {line}");
    }
    built.insert(out.clone());
    out
}

fn complete(path: &Path) -> bool {
    ["top.config", "top_routed.json", "top.bit"]
        .iter()
        .all(|name| path.join(name).is_file())
}

fn collect(dir: &Path, inputs: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        // Path metadata and reads follow symlinks, including shared HDL.
        if path.is_dir() {
            collect(&path, inputs);
        } else {
            inputs.push(path);
        }
    }
}

fn hash_file(hash: &mut Sha256, name: &Path, path: &Path) {
    let contents = fs::read(path).unwrap();
    hash.update(name.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update((contents.len() as u64).to_le_bytes());
    hash.update(contents);
}
