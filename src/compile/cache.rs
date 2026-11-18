//! Best-effort disk cache, following fabric's temp-directory/disable rules.
use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ECP5PLN1";
const VERSION: u32 = 4;
const HEADER: usize = 84;
const KEEP: usize = 3;
/// Limit completed plan files to 1 GB as well as at most three entries.
const MAX_BYTES: u64 = 1_000_000_000;

pub(super) fn key(device: &ConfiguredDevice, request: &Request, board: &Board) -> Option<[u8; 32]> {
    #[cfg(feature = "compile-timing")]
    let _timer = crate::prepared::load_timing::enter("plan.cache_key", 0);
    let executable = crate::image::executable_sha256()?;
    Some(key_for(
        device.image().source_sha256(),
        device.fingerprint(),
        request,
        board,
        &executable,
    ))
}
fn key_for(
    source_sha256: &str,
    fingerprint: &str,
    request: &Request,
    board: &Board,
    executable: &[u8; 32],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(MAGIC);
    hash.update(VERSION.to_le_bytes());
    hash.update(source_sha256);
    // Also binds the decoded image, architecture and configuration catalogue.
    hash.update(fingerprint);
    hash.update(executable);
    hash.update(serde_json::to_vec(&(request, board)).expect("serializable compile request"));
    hash.finalize().into()
}

pub(super) fn directory() -> Option<PathBuf> {
    if ["ECP5_FABRIC_CACHE", "ECP5_COMPILE_CACHE"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|v| v.trim() == "0"))
    {
        return None;
    }
    Some(std::env::temp_dir().join("ecp5_emu_compile"))
}
fn path(dir: &Path, key: &[u8; 32]) -> PathBuf {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.bin"))
}
fn envelope(key: &[u8; 32], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&Sha256::digest(payload));
    bytes.extend_from_slice(key);
    bytes.extend_from_slice(payload);
    bytes
}
fn payload<'a>(key: &[u8; 32], bytes: &'a [u8]) -> Option<&'a [u8]> {
    if bytes.len() < HEADER
        || &bytes[..8] != MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().ok()?) != VERSION
        || &bytes[52..HEADER] != key
    {
        return None;
    }
    let len = usize::try_from(u64::from_le_bytes(bytes[12..20].try_into().ok()?)).ok()?;
    if bytes.len() != HEADER.checked_add(len)? {
        return None;
    }
    let payload = &bytes[HEADER..];
    let digest: [u8; 32] = Sha256::digest(payload).into();
    (digest == bytes[20..52]).then_some(payload)
}

pub(super) fn read(
    dir: &Path,
    key: &[u8; 32],
    device: Arc<ConfiguredDevice>,
    options: kernel::Options,
) -> Option<Compiled> {
    #[cfg(feature = "compile-timing")]
    let _timer = crate::prepared::load_timing::enter("plan.cache_read_restore", 0);
    // Enforce the limits for existing caches too, including a warm-only session.
    prune(dir, KEEP, MAX_BYTES);
    #[cfg(feature = "compile-timing")]
    let read_timer = crate::prepared::load_timing::enter(
        "plan.file_read",
        std::fs::metadata(path(dir, key)).map_or(0, |m| m.len()),
    );
    let bytes = std::fs::read(path(dir, key)).ok()?;
    #[cfg(feature = "compile-timing")]
    drop(read_timer);
    #[cfg(feature = "compile-timing")]
    let checksum_timer =
        crate::prepared::load_timing::enter("plan.payload_sha256", bytes.len() as u64);
    let payload = payload(key, &bytes)?;
    #[cfg(feature = "compile-timing")]
    drop(checksum_timer);
    let compiled = kernel::cache::decode(Arc::clone(&device), options, payload).ok()?;
    (compiled.report.configuration_sha256 == device.fingerprint()).then_some(compiled)
}
pub(super) fn write(dir: &Path, key: &[u8; 32], compiled: &Compiled) -> Option<()> {
    let payload = kernel::cache::encode(compiled)?;
    if payload.len() as u128 + HEADER as u128 > u128::from(MAX_BYTES) {
        return None;
    }
    let bytes = envelope(key, &payload);
    write_atomic(&path(dir, key), &bytes).ok()?;
    prune(dir, KEEP, MAX_BYTES);
    Some(())
}

/// Remove oldest completed entries until both limits hold. Like the fabric
/// cache, unreadable or unremovable files are left alone; atomic temporary
/// files belong to writers and are not pruned here.
fn prune(dir: &Path, keep: usize, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .filter_map(|path| {
            let metadata = std::fs::metadata(&path).ok()?;
            metadata
                .is_file()
                .then_some((metadata.modified().ok()?, path, metadata.len()))
        })
        .collect();
    files.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut count = files.len();
    let mut bytes: u128 = files.iter().map(|(_, _, size)| u128::from(*size)).sum();
    for (_, path, size) in files {
        if count <= keep && bytes <= u128::from(max_bytes) {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            count -= 1;
            bytes -= u128::from(size);
        }
    }
}
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().expect("cache directory");
    std::fs::create_dir_all(dir)?;
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn header_checks_all_metadata_and_payload() {
        let key = [7; 32];
        let bytes = envelope(&key, b"plan");
        assert_eq!(payload(&key, &bytes), Some(&b"plan"[..]));
        assert!(payload(&[8; 32], &bytes).is_none());
        for i in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[i] ^= 1;
            assert!(payload(&key, &corrupt).is_none(), "accepted byte {i}");
            assert!(payload(&key, &bytes[..i]).is_none());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(payload(&key, &trailing).is_none());
    }

    #[test]
    fn key_binds_configuration_executable_and_every_request_and_board_field() {
        let request = Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![(Reference::Pin("G2".into()), Logic::Zero)],
            probes: vec![Reference::Pin("B2".into())],
            options: kernel::Options::default(),
        };
        let board = Board::default();
        let key = key_for("source", "bound", &request, &board, &[1; 32]);
        let differs = |other: [u8; 32]| {
            assert_ne!(key, other);
            assert_ne!(
                crate::layout::GeometryKey::new(&key),
                crate::layout::GeometryKey::new(&other)
            );
            assert_ne!(
                crate::fabric_overlay::OverlayKey::new(&key),
                crate::fabric_overlay::OverlayKey::new(&other)
            );
        };
        differs(key_for("other", "bound", &request, &board, &[1; 32]));
        differs(key_for("source", "other", &request, &board, &[1; 32]));
        differs(key_for("source", "bound", &request, &board, &[2; 32]));
        for change in 0..7 {
            let mut other = request.clone();
            match change {
                0 => other.package = "other".into(),
                1 => other.scope = Scope::AllPlcRegisters,
                2 => other.startup = Startup::WakePhases,
                3 => other.drives[0].1 = Logic::One,
                4 => other.probes.push(Reference::Pin("C2".into())),
                5 => other.options.trace_capacity += 1,
                _ => other.options.max_delta_cycles += 1,
            }
            differs(key_for("source", "bound", &other, &board, &[1; 32]));
        }
        let mut board = board;
        board.oscillator_hz = Some(300_000_000);
        differs(key_for("source", "bound", &request, &board, &[1; 32]));
        board.oscillator_hz = None;
        board.dtr_code = Some(0xA1);
        assert_ne!(key, key_for("source", "bound", &request, &board, &[1; 32]));
        board.dtr_code = None;
        board.wires.push(BoardWire {
            from: Reference::Pin("B2".into()),
            to: Reference::Pin("C2".into()),
        });
        differs(key_for("source", "bound", &request, &board, &[1; 32]));
    }

    #[test]
    fn writes_are_atomic_and_only_the_newest_three_files_remain() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ecp5-plan-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        for i in 0..5 {
            let file = path(&dir, &[i; 32]);
            write_atomic(&file, &[i])?;
            std::fs::File::open(file)?.set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::from(i)),
            )?;
        }
        prune(&dir, KEEP, MAX_BYTES);
        for i in 0..5 {
            assert_eq!(path(&dir, &[i; 32]).exists(), i >= 2);
        }
        write_atomic(&path(&dir, &[4; 32]), b"replacement")?;
        assert_eq!(std::fs::read(path(&dir, &[4; 32]))?, b"replacement");
        assert_eq!(std::fs::read_dir(&dir)?.count(), KEEP);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn size_limit_removes_oldest_even_below_the_entry_limit() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("ecp5-plan-cache-size-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // Small real files exercise the byte accounting without allocating GB.
        for (i, size) in [(0, 6), (1, 4), (2, 6)] {
            let file = path(&dir, &[i; 32]);
            write_atomic(&file, &vec![i; size])?;
            std::fs::File::open(file)?.set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::from(i)),
            )?;
        }
        let temporary = dir.join(".tmp-active-writer");
        std::fs::write(&temporary, b"in progress")?;
        prune(&dir, KEEP, 10);
        assert!(!path(&dir, &[0; 32]).exists());
        assert!(path(&dir, &[1; 32]).exists());
        assert!(path(&dir, &[2; 32]).exists());
        assert!(temporary.exists());
        // One oversized entry must not be retained just because it is newest.
        prune(&dir, KEEP, 5);
        assert!(!path(&dir, &[1; 32]).exists());
        assert!(!path(&dir, &[2; 32]).exists());
        assert!(temporary.exists());
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
