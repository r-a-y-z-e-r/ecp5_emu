//! Immutable input image. Native service values are preserved separately from CRAM.
use crate::{config::Config, decode::Cram};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

/// The native image bridge selected by an explicit path, environment, or checkout.
pub fn reader_path(explicit: Option<&Path>) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ECP5_IMAGE_READER").map(PathBuf::from))
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/trellis_image_reader")
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Bitstream,
    TextConfig,
}

impl SourceKind {
    fn origin(self) -> &'static str {
        match self {
            Self::Bitstream => "bitstream",
            Self::TextConfig => "text_config",
        }
    }
    fn argument(self) -> &'static str {
        match self {
            Self::Bitstream => "bit",
            Self::TextConfig => "config",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NativeRegisters {
    pub ctrl0: u32,
    pub ctrl1: u32,
    pub sed_reference: u32,
    pub usercode: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceMetadata {
    pub native_registers: Option<NativeRegisters>,
    /// These are values after the native parser, not proof that each command was present.
    pub parser_defaults_may_apply: bool,
    pub native_comments: Vec<Vec<u8>>,
    pub text_comments: Vec<String>,
    pub sysconfig_directives: BTreeMap<String, String>,
    pub text_variant: Option<String>,
}

#[derive(Deserialize)]
struct ReaderEnvelope {
    image_reader_schema: u32,
    native_registers: Option<NativeRegisters>,
    parser_defaults_may_apply: bool,
    native_metadata_hex: Vec<String>,
}

#[derive(Serialize)]
pub struct ImageSummary {
    pub source_kind: SourceKind,
    pub source_sha256: String,
    pub decoded_sha256: String,
    pub source_bytes: usize,
    pub cram_fingerprint: String,
    pub frames: usize,
    pub bits_per_frame: usize,
    pub ebr_init_images: usize,
    pub service: ServiceMetadata,
}

pub struct ConfigurationImage {
    source: Vec<u8>,
    kind: SourceKind,
    sha256: String,
    decoded_sha256: String,
    cram: Cram,
    service: ServiceMetadata,
}

/// Hash the immutable mapped executable once per process, shared by caches.
pub(crate) fn executable_sha256() -> Option<[u8; 32]> {
    static DIGEST: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    *DIGEST.get_or_init(|| {
        let path = std::env::current_exe().ok()?;
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter(
            "identity.executable_sha256",
            std::fs::metadata(&path).map_or(0, |m| m.len()),
        );
        hash_file(&path).ok()
    })
}

fn hash_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct TextMetadata {
    device: String,
    comments: Vec<String>,
    sysconfig: BTreeMap<String, String>,
    variant: Option<String>,
}
impl TextMetadata {
    fn parse(source: &[u8]) -> Result<Self> {
        // This view extracts service metadata, not routing. nextpnr can emit
        // both an empty/default DCS arc and its selected arc at one sink.
        // libtrellis decodes the complete, unchanged source into CRAM; the
        // legacy single-source Config map must not reject it here (HC02).
        let text = std::str::from_utf8(source).context("config is not UTF-8")?;
        let metadata = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("arc:"))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = Config::parse(&metadata)?;
        Ok(Self {
            device: parsed.device,
            comments: parsed.comments,
            sysconfig: parsed.sysconfig,
            variant: parsed.variant,
        })
    }
}

mod image_cache {
    use super::*;

    const MAGIC: &[u8; 8] = b"ECP5IMG1";
    const HEADER: usize = 84;
    const VERSION: u32 = 1;
    // CRAM and INIT9, not the much larger textual routing maps/source bytes.
    const MAX_FILE: u64 = 32 * 1024 * 1024;

    fn directory() -> PathBuf {
        std::env::temp_dir().join("ecp5_emu_image")
    }
    fn path(key: &[u8; 32]) -> PathBuf {
        directory().join(format!("{}.bin", hex_digest(key)))
    }

    fn database_digest(database: &Path) -> std::io::Result<[u8; 32]> {
        let mut files = Vec::new();
        let root_file = database.join("devices.json");
        if root_file.is_file() {
            files.push(root_file);
        }
        let mut folders = vec![database.join("ECP5")];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|x| x == "json" || x == "db") {
                    files.push(path);
                }
            }
        }
        files.sort();
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter(
            "image.database_sha256",
            files
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok())
                .map(|m| m.len())
                .sum(),
        );
        let mut hash = Sha256::new();
        for file in files {
            let name = file
                .strip_prefix(database)
                .expect("database source")
                .to_string_lossy();
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(hash_file(&file)?);
        }
        Ok(hash.finalize().into())
    }

    /// Include the actual native decoder library, not only its small launcher.
    /// Unsupported dynamic-library inspection disables this optional cache.
    fn reader_libraries(reader: &Path) -> Option<Vec<PathBuf>> {
        let mut prefix = [0; 2];
        std::fs::File::open(reader)
            .ok()?
            .read_exact(&mut prefix)
            .ok()?;
        // Portable scripted readers (including fixtures) have no linked Trellis.
        if &prefix == b"#!" {
            return Some(Vec::new());
        }
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("otool").arg("-L").arg(reader).output().ok()?;
            if !output.status.success() {
                return None;
            }
            let libraries = String::from_utf8(output.stdout).ok()?;
            let rpaths = Command::new("otool").arg("-l").arg(reader).output().ok()?;
            let rpaths = String::from_utf8(rpaths.stdout).ok()?;
            let mut roots: Vec<PathBuf> = std::env::var_os("DYLD_LIBRARY_PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default();
            roots.extend(rpaths.lines().filter_map(|line| {
                line.trim()
                    .strip_prefix("path ")?
                    .split_once(" (offset")
                    .map(|p| PathBuf::from(p.0))
            }));
            let mut result = Vec::new();
            for line in libraries.lines().filter(|l| l.contains("libtrellis")) {
                let library = line.split_whitespace().next()?;
                let path = if let Some(name) = library.strip_prefix("@rpath/") {
                    roots
                        .iter()
                        .map(|root| root.join(name))
                        .find(|p| p.is_file())?
                } else if let Some(name) = library.strip_prefix("@loader_path/") {
                    reader.parent()?.join(name)
                } else {
                    PathBuf::from(library)
                };
                result.push(path);
            }
            Some(result)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let output = Command::new("ldd").arg(reader).output().ok()?;
            if !output.status.success() {
                let message = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                return (message.contains("not a dynamic executable")
                    || message.contains("statically linked"))
                .then(Vec::new);
            }
            let text = String::from_utf8(output.stdout).ok()?;
            text.lines()
                .filter(|l| l.contains("libtrellis"))
                .map(|line| {
                    let path = line.split_once("=>")?.1.split_whitespace().next()?;
                    Path::new(path).is_absolute().then(|| PathBuf::from(path))
                })
                .collect()
        }
    }

    pub(super) fn key(
        source: &str,
        kind: SourceKind,
        database: &Path,
        reader: &Path,
    ) -> Option<[u8; 32]> {
        if ["ECP5_IMAGE_CACHE", "ECP5_FABRIC_CACHE"]
            .iter()
            .any(|v| std::env::var(v).is_ok_and(|v| v.trim() == "0"))
        {
            return None;
        }
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter("image.cache_key", 0);
        let mut hash = Sha256::new();
        hash.update(MAGIC);
        hash.update(VERSION.to_le_bytes());
        hash.update(source.as_bytes());
        hash.update(kind.argument());
        hash.update(executable_sha256()?);
        hash.update(database_digest(database).ok()?);
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter(
            "image.reader_identity",
            std::fs::metadata(reader).map_or(0, |m| m.len()),
        );
        hash.update(hash_file(reader).ok()?);
        for library in reader_libraries(reader)? {
            hash.update(hash_file(&library).ok()?);
        }
        Some(hash.finalize().into())
    }

    fn payload<'a>(bytes: &'a [u8], key: &[u8; 32]) -> Option<&'a [u8]> {
        if bytes.len() < HEADER
            || bytes.len() as u64 > MAX_FILE
            || &bytes[..8] != MAGIC
            || bytes[8..12] != VERSION.to_le_bytes()
            || &bytes[52..HEADER] != key
        {
            return None;
        }
        let length = usize::try_from(u64::from_le_bytes(bytes[12..20].try_into().ok()?)).ok()?;
        if HEADER.checked_add(length)? != bytes.len() {
            return None;
        }
        let data = &bytes[HEADER..];
        let digest: [u8; 32] = Sha256::digest(data).into();
        (digest == bytes[20..52]).then_some(data)
    }

    pub(super) fn read(
        key: &[u8; 32],
        source_hash: &str,
        kind: SourceKind,
        source: &[u8],
    ) -> Option<ConfigurationImage> {
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter("image.cache_read_decode", 0);
        let file = path(key);
        if std::fs::metadata(&file).ok()?.len() > MAX_FILE {
            return None;
        }
        let bytes = std::fs::read(file).ok()?;
        decode(&bytes, key, source_hash, kind, source)
    }

    type Cached = (SourceKind, String, String, Cram, ServiceMetadata);
    fn decode(
        bytes: &[u8],
        key: &[u8; 32],
        source_hash: &str,
        kind: SourceKind,
        source: &[u8],
    ) -> Option<ConfigurationImage> {
        let (cached, rest): (Cached, _) = postcard::take_from_bytes(payload(bytes, key)?).ok()?;
        let (cached_kind, sha256, decoded_sha256, cram, service) = cached;
        if !rest.is_empty()
            || cached_kind != kind
            || sha256 != source_hash
            || cram.origin != kind.origin()
            || cram.frames == 0
            || cram.bits_per_frame == 0
            || !cram.bits_per_frame.is_multiple_of(8)
            || cram.frames.checked_mul(cram.bits_per_frame / 8)? != cram.bytes().len()
            || !cram.bram.values().flatten().all(|&w| w <= 0x1ff)
            || service.native_registers.is_some() != (kind == SourceKind::Bitstream)
            || service.parser_defaults_may_apply != (kind == SourceKind::Bitstream)
        {
            return None;
        }
        Some(ConfigurationImage {
            source: source.to_vec(),
            kind,
            sha256,
            decoded_sha256,
            cram,
            service,
        })
    }

    pub(super) fn write(key: &[u8; 32], image: &ConfigurationImage) -> Option<()> {
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter("image.cache_write", 0);
        let bytes = encode(key, image)?;
        let dir = directory();
        std::fs::create_dir_all(&dir).ok()?;
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let temporary = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        if std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(&temporary, path(key)))
            .is_err()
        {
            let _ = std::fs::remove_file(temporary);
            return None;
        }
        // Each entry is bounded by MAX_FILE, so this bounds the folder to 96 MiB.
        crate::fabric::prune_cache(&dir, 3);
        Some(())
    }

    fn encode(key: &[u8; 32], image: &ConfigurationImage) -> Option<Vec<u8>> {
        let data = postcard::to_stdvec(&(
            image.kind,
            &image.sha256,
            &image.decoded_sha256,
            &image.cram,
            &image.service,
        ))
        .ok()?;
        if data.len() as u128 + HEADER as u128 > u128::from(MAX_FILE) {
            return None;
        }
        let mut bytes = Vec::with_capacity(HEADER + data.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&Sha256::digest(&data));
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&data);
        Some(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct Folder(PathBuf);
        impl Folder {
            fn new() -> Self {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let root = std::env::temp_dir().join(format!(
                    "ecp5-image-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                std::fs::create_dir_all(root.join("ECP5")).unwrap();
                std::fs::write(root.join("ECP5/bits.db"), b"database version 1").unwrap();
                Self(root)
            }
        }
        impl Drop for Folder {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn snapshot(kind: SourceKind) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F",
                "frames":1,"bits_per_frame":8,"encoding":"frame_byte_lsb0","cram_hex":"a5",
                "bram_data":{"2":[0,511,256]},"origin":kind.origin(),
                "native_registers":if kind == SourceKind::Bitstream { serde_json::json!({"ctrl0":1,"ctrl1":2,"sed_reference":3,"usercode":4}) } else { serde_json::Value::Null },
                "parser_defaults_may_apply":kind == SourceKind::Bitstream,"native_metadata_hex":["00ff"]
            })).unwrap()
        }

        #[test]
        fn cache_preserves_source_cram_and_native_or_text_metadata() {
            for kind in [SourceKind::TextConfig, SourceKind::Bitstream] {
                let source = if kind == SourceKind::TextConfig {
                    b".device LFE5U-85F\n.variant test\n.comment exact comment\n.sysconfig USER 123\n".to_vec()
                } else {
                    vec![0xff, 0, 1, 3]
                };
                let image =
                    ConfigurationImage::from_snapshot(source.clone(), kind, &snapshot(kind))
                        .unwrap();
                let bytes = encode(&[7; 32], &image).unwrap();
                let restored =
                    decode(&bytes, &[7; 32], image.source_sha256(), kind, &source).unwrap();
                assert_eq!(
                    serde_json::to_vec(&image.summary()).unwrap(),
                    serde_json::to_vec(&restored.summary()).unwrap()
                );
                assert_eq!(image.source_bytes(), restored.source_bytes());
                assert_eq!(image.cram().bytes(), restored.cram().bytes());
                assert_eq!(image.cram().bram, restored.cram().bram);
                assert!(decode(&bytes, &[8; 32], image.source_sha256(), kind, &source).is_none());
                assert!(decode(&bytes, &[7; 32], "different source", kind, &source).is_none());
                for i in 0..bytes.len() {
                    let mut corrupt = bytes.clone();
                    corrupt[i] ^= 1;
                    assert!(payload(&corrupt, &[7; 32]).is_none());
                    assert!(payload(&bytes[..i], &[7; 32]).is_none());
                }
                let mut trailing = bytes;
                trailing.push(0);
                assert!(payload(&trailing, &[7; 32]).is_none());
            }
        }

        #[test]
        fn cache_key_tracks_contents_even_when_size_and_mtime_are_preserved() {
            let folder = Folder::new();
            let reader = folder.0.join("reader");
            std::fs::write(&reader, b"#!/bin/sh\n# decoder 1").unwrap();
            let first = key("source", SourceKind::TextConfig, &folder.0, &reader).unwrap();
            assert_ne!(
                Some(first),
                key("other source", SourceKind::TextConfig, &folder.0, &reader)
            );
            assert_ne!(
                Some(first),
                key("source", SourceKind::Bitstream, &folder.0, &reader)
            );
            let file = folder.0.join("ECP5/bits.db");
            let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
            std::fs::write(&file, b"database version 2").unwrap();
            std::fs::File::open(file)
                .unwrap()
                .set_modified(modified)
                .unwrap();
            let second = key("source", SourceKind::TextConfig, &folder.0, &reader).unwrap();
            assert_ne!(first, second);
            std::fs::write(&reader, b"#!/bin/sh\n# decoder 2").unwrap();
            assert_ne!(
                Some(second),
                key("source", SourceKind::TextConfig, &folder.0, &reader)
            );
        }

        #[cfg(unix)]
        #[test]
        fn second_load_skips_reader_and_corruption_rebuilds() -> Result<()> {
            use std::os::unix::fs::PermissionsExt;
            let folder = Folder::new();
            let source = folder.0.join("top.config");
            let reader = folder.0.join("reader");
            let marker = folder.0.join("called");
            let marker_text = marker.to_string_lossy().replace('\'', "'\\''");
            let snap = String::from_utf8(snapshot(SourceKind::TextConfig))?;
            let script = format!("#!/bin/sh\ncat >/dev/null\n[ ! -e '{marker_text}' ] || exit 17\ntouch '{marker_text}'\nprintf '%s\\n' '{snap}'\n");
            std::fs::write(&reader, script)?;
            std::fs::set_permissions(&reader, std::fs::Permissions::from_mode(0o755))?;
            std::fs::write(&source, b".device LFE5U-85F\n.comment image cache test\n")?;
            let first = ConfigurationImage::load(&source, &folder.0, Some(&reader))?;
            let second = ConfigurationImage::load(&source, &folder.0, Some(&reader))?;
            assert_eq!(
                serde_json::to_vec(&first.summary())?,
                serde_json::to_vec(&second.summary())?
            );
            let key = key(
                first.source_sha256(),
                SourceKind::TextConfig,
                &folder.0,
                &reader,
            )
            .unwrap();
            let file = path(&key);
            let mut bytes = std::fs::read(&file)?;
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            std::fs::write(&file, bytes)?;
            std::fs::remove_file(&marker)?;
            let rebuilt = ConfigurationImage::load(&source, &folder.0, Some(&reader))?;
            assert!(marker.exists());
            assert_eq!(
                serde_json::to_vec(&first.summary())?,
                serde_json::to_vec(&rebuilt.summary())?
            );
            std::fs::write(&source, b".device LFE5U-85F\n.comment changed source\n")?;
            assert!(ConfigurationImage::load(&source, &folder.0, Some(&reader)).is_err());
            std::fs::remove_file(file)?;
            Ok(())
        }
    }
}

impl ConfigurationImage {
    pub fn load(path: &Path, database: &Path, reader: Option<&Path>) -> Result<Self> {
        #[cfg(feature = "compile-timing")]
        let _timer = crate::prepared::load_timing::enter("image.load", 0);
        let kind = if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("bit"))
        {
            SourceKind::Bitstream
        } else {
            SourceKind::TextConfig
        };
        #[cfg(feature = "compile-timing")]
        let read_timer = crate::prepared::load_timing::enter(
            "image.source_read",
            std::fs::metadata(path).map_or(0, |m| m.len()),
        );
        let source = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        #[cfg(feature = "compile-timing")]
        drop(read_timer);
        let executable = reader_path(reader);
        #[cfg(feature = "compile-timing")]
        let hash_timer =
            crate::prepared::load_timing::enter("image.source_sha256", source.len() as u64);
        let source_sha256 = hex_digest(&Sha256::digest(&source));
        #[cfg(feature = "compile-timing")]
        drop(hash_timer);
        let cache = image_cache::key(&source_sha256, kind, database, &executable);
        if let Some(key) = &cache {
            if let Some(image) = image_cache::read(key, &source_sha256, kind, &source) {
                return Ok(image);
            }
        }
        // Validate before native parsing, retaining metadata rather than route maps.
        let text = if kind == SourceKind::TextConfig {
            #[cfg(feature = "compile-timing")]
            let _timer =
                crate::prepared::load_timing::enter("image.validate_text", source.len() as u64);
            Some(TextMetadata::parse(&source)?)
        } else {
            None
        };
        #[cfg(feature = "compile-timing")]
        let native_timer =
            crate::prepared::load_timing::enter("image.native_reader", source.len() as u64);
        let mut child = Command::new(&executable)
            .arg(database)
            .arg(kind.argument())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                let message = if reader.is_none()
                    && std::env::var_os("ECP5_IMAGE_READER")
                        .is_some_and(|path| Path::new(&path) == executable)
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                    )
                {
                    format!(
                        "image reader not found at {} (from ECP5_IMAGE_READER): reinstall the package or point ECP5_IMAGE_READER at a working image reader",
                        executable.display()
                    )
                } else {
                    format!(
                        "starting {}; build it with python3 tools/build_image_reader.py (--trellis PATH or TRELLIS_ROOT / parent of ECP5_DB; --prefix PATH or TRELLIS_PREFIX)",
                        executable.display()
                    )
                };
                anyhow::Error::new(error).context(message)
            })?;
        let write = child
            .stdin
            .take()
            .context("image reader stdin unavailable")?
            .write_all(&source);
        let output = child
            .wait_with_output()
            .context("waiting for image reader")?;
        ensure!(
            output.status.success(),
            "native image import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        write.context("sending original bytes to image reader")?;
        #[cfg(feature = "compile-timing")]
        drop(native_timer);
        let image =
            Self::from_snapshot_inner(source, kind, &output.stdout, text, Some(source_sha256))?;
        if let Some(key) = cache {
            image_cache::write(&key, &image);
        }
        Ok(image)
    }

    /// Also used by portable fixtures and future in-process native bindings.
    pub fn from_snapshot(source: Vec<u8>, kind: SourceKind, snapshot: &[u8]) -> Result<Self> {
        Self::from_snapshot_inner(source, kind, snapshot, None, None)
    }

    fn from_snapshot_inner(
        source: Vec<u8>,
        kind: SourceKind,
        snapshot: &[u8],
        text: Option<TextMetadata>,
        source_sha256: Option<String>,
    ) -> Result<Self> {
        #[cfg(feature = "compile-timing")]
        let _timer =
            crate::prepared::load_timing::enter("image.snapshot_decode", snapshot.len() as u64);
        let envelope: ReaderEnvelope =
            serde_json::from_slice(snapshot).context("reading native image metadata")?;
        ensure!(
            envelope.image_reader_schema == 1,
            "unsupported image reader schema"
        );
        ensure!(
            envelope.native_registers.is_some() == (kind == SourceKind::Bitstream),
            "source kind and native register evidence disagree"
        );
        ensure!(
            envelope.parser_defaults_may_apply == (kind == SourceKind::Bitstream),
            "source kind and native default provenance disagree"
        );
        let cram = Cram::from_json(snapshot)?;
        ensure!(
            cram.origin == kind.origin(),
            "source kind and CRAM origin disagree"
        );
        let text = if kind == SourceKind::TextConfig {
            #[cfg(feature = "compile-timing")]
            let _timer =
                crate::prepared::load_timing::enter("image.parse_text_again", source.len() as u64);
            let parsed = match text {
                Some(text) => text,
                None => TextMetadata::parse(&source)?,
            };
            ensure!(
                parsed.device == cram.device,
                "text and CRAM device disagree"
            );
            Some(parsed)
        } else {
            None
        };
        let comments = envelope
            .native_metadata_hex
            .into_iter()
            .map(|s| {
                ensure!(s.len().is_multiple_of(2), "odd native comment encoding");
                s.as_bytes()
                    .chunks_exact(2)
                    .map(|bytes| {
                        let text = std::str::from_utf8(bytes).context("invalid comment hex")?;
                        u8::from_str_radix(text, 16).context("invalid comment hex")
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let service = ServiceMetadata {
            native_registers: envelope.native_registers,
            parser_defaults_may_apply: envelope.parser_defaults_may_apply,
            native_comments: comments,
            text_comments: text
                .as_ref()
                .map(|c| c.comments.clone())
                .unwrap_or_default(),
            sysconfig_directives: text
                .as_ref()
                .map(|c| c.sysconfig.clone())
                .unwrap_or_default(),
            text_variant: text.and_then(|c| c.variant),
        };
        #[cfg(feature = "compile-timing")]
        let hash_timer =
            crate::prepared::load_timing::enter("image.source_sha256", source.len() as u64);
        let sha256 = source_sha256.unwrap_or_else(|| hex_digest(&Sha256::digest(&source)));
        #[cfg(feature = "compile-timing")]
        drop(hash_timer);
        let mut decoded = Sha256::new();
        decoded.update(b"ecp5-configuration-image-v1\0");
        decoded.update(cram.device.as_bytes());
        decoded.update((cram.frames as u64).to_le_bytes());
        decoded.update((cram.bits_per_frame as u64).to_le_bytes());
        decoded.update(cram.bytes());
        decoded.update((cram.bram.len() as u64).to_le_bytes());
        for (id, words) in &cram.bram {
            decoded.update(id.to_le_bytes());
            decoded.update((words.len() as u64).to_le_bytes());
            for word in words {
                decoded.update(word.to_le_bytes());
            }
        }
        let metadata = serde_json::to_vec(&service)?;
        decoded.update((metadata.len() as u64).to_le_bytes());
        decoded.update(metadata);
        let decoded_sha256 = decoded
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self {
            source,
            kind,
            sha256,
            decoded_sha256,
            cram,
            service,
        })
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }
    pub fn source_kind(&self) -> SourceKind {
        self.kind
    }
    pub fn source_sha256(&self) -> &str {
        &self.sha256
    }
    pub fn decoded_sha256(&self) -> &str {
        &self.decoded_sha256
    }
    pub fn cram(&self) -> &Cram {
        &self.cram
    }
    pub fn service(&self) -> &ServiceMetadata {
        &self.service
    }
    pub fn summary(&self) -> ImageSummary {
        ImageSummary {
            source_kind: self.kind,
            source_sha256: self.sha256.clone(),
            decoded_sha256: self.decoded_sha256.clone(),
            source_bytes: self.source.len(),
            cram_fingerprint: self.cram.fingerprint(),
            frames: self.cram.frames,
            bits_per_frame: self.cram.bits_per_frame,
            ebr_init_images: self.cram.bram.len(),
            service: self.service.clone(),
        }
    }
}
