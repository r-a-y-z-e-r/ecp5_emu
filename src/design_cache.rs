//! Shared inputs and storage for design view caches.
use crate::model::topology::{Location, Wire};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Reuse the compile identity, which covers the validated model manifest,
/// decoded configuration, catalogues, request, board and executable.
pub(crate) fn key(compile_key: &[u8; 32], version: u32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ecp5-design-view-v2\0");
    hasher.update(version.to_le_bytes());
    hasher.update(compile_key);
    hasher.finalize().into()
}

pub(crate) fn path(key: &[u8; 32], dir: &Path) -> PathBuf {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.bin"))
}

pub(crate) fn limit(variable: &str, default: u64) -> u64 {
    std::env::var(variable)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .and_then(|mb| mb.checked_mul(1024 * 1024))
        .unwrap_or(default)
}

// The model's wire types only serialize. These adapters read them back
// without changing the model's public types.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Location")]
struct CachedLocation {
    row: i16,
    col: i16,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Wire")]
pub(crate) struct CachedWire {
    #[serde(with = "CachedLocation")]
    location: Location,
    name: String,
}

pub(crate) mod cached_wires {
    use super::*;

    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "CachedWire")] &'a Wire);

    #[derive(Deserialize)]
    struct Owned(#[serde(with = "CachedWire")] Wire);

    pub fn serialize<S: serde::Serializer>(
        wires: &[Wire],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(wires.iter().map(Borrowed))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Wire>, D::Error> {
        Ok(Vec::<Owned>::deserialize(deserializer)?
            .into_iter()
            .map(|w| w.0)
            .collect())
    }
}

pub(crate) fn write_cache_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Keeps the newest prefix within the budget, always keeping one file.
pub(crate) fn prune(dir: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .filter_map(|p| {
            let metadata = std::fs::metadata(&p).ok()?;
            metadata
                .is_file()
                .then_some((metadata.modified().ok()?, p, metadata.len()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut total = 0u64;
    let mut pruning = false;
    for (i, (_, path, size)) in files.into_iter().enumerate() {
        pruning |= i > 0 && (total > limit || size > limit.saturating_sub(total));
        if pruning {
            let _ = std::fs::remove_file(path);
        } else {
            total = total.saturating_add(size);
        }
    }
}
