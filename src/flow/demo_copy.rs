//! Portable copies of the bundled demos, with shared sources materialized and
//! their notices retained. Creation never runs firmware scripts or build tools.
//! `ECP5_EXAMPLES` overrides package/checkout discovery; the build-host fallback is debug-only.
//!
//! An optional `examples/showcase.toml`, next to `ulx3s/` and `gallery/`, selects
//! which demos are visible by default:
//! ```toml
//! # Examples shown by default; every other example appears in Dev mode.
//! showcase = ["ulx3s/blinky_cycle", "gallery/riscv_shell"]
//! ```
//! Without the file, every demo is showcase. With it, only listed demos are
//! showcase; an empty list selects none. Unknown keys and malformed TOML are
//! errors naming the file. Listed names that are not copyable demos are reported
//! in [`Catalogue::unknown_showcase`] without failing the catalogue.
//!
//! [`catalogue`] and [`catalogue_in`] return all copyable [`Demo`] entries.
//! Use [`Catalogue::visible`] with `false` for showcase demos or `true` for every
//! demo in Dev mode. [`create`] and [`create_in`] accept any demo by name,
//! regardless of its showcase flag.
use super::{new_project, Project};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const APACHE: &str = include_str!("../../LICENSE");
const ULX3S: &str = include_str!("../../assets/ulx3s/LICENSE.md");

/// Demo selection and destination, independent of a window or CLI.
#[derive(Clone, Debug)]
pub struct Options {
    pub demo: String,
    pub dir: PathBuf,
    pub name: String,
}
impl Options {
    /// Uses the destination leaf as the display name.
    pub fn new(dir: impl Into<PathBuf>, demo: impl Into<String>) -> Self {
        let options = new_project::Options::new(dir);
        Self {
            demo: demo.into(),
            dir: options.dir,
            name: options.name,
        }
    }
}

/// A copyable bundled demo and its default visibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demo {
    /// Group-qualified name, such as `ulx3s/blinky_cycle`.
    pub name: String,
    /// Source group: `ulx3s` or `gallery`.
    pub group: String,
    /// Whether this demo is visible outside Dev mode.
    pub showcase: bool,
}

/// All copyable demos, sorted by name, and catalogue diagnostics.
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    pub demos: Vec<Demo>,
    /// Excluded folders and the reasons they cannot be copied.
    pub excluded: Vec<(String, String)>,
    /// Sorted, distinct table entries that do not name a copyable demo.
    pub unknown_showcase: Vec<String>,
}

impl Catalogue {
    /// Iterates over showcase demos by default, or every demo in Dev mode.
    pub fn visible(&self, dev: bool) -> impl Iterator<Item = &Demo> {
        self.demos.iter().filter(move |demo| dev || demo.showcase)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ShowcaseTable {
    showcase: BTreeSet<String>,
}

/// Finds bundled examples, returning `None` if discovery fails.
pub fn examples_root() -> Option<PathBuf> {
    examples_root_checked().ok()
}

/// Finds examples with both group README markers, or reports the tried locations.
/// An explicit `ECP5_EXAMPLES` takes precedence and never falls back on failure.
/// Otherwise searches package resources, executable/current folder ancestors,
/// and, in debug builds only, the compile-time repository.
pub fn examples_root_checked() -> Result<PathBuf> {
    let executable = std::env::current_exe().ok();
    let current = std::env::current_dir().ok();
    let override_root = std::env::var_os("ECP5_EXAMPLES").map(PathBuf::from);
    find_examples_root(
        executable.as_deref(),
        current.as_deref(),
        override_root.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
        cfg!(debug_assertions),
    )
}

fn find_examples_root(
    executable: Option<&Path>,
    current: Option<&Path>,
    override_root: Option<&Path>,
    fallback: &Path,
    debug: bool,
) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = override_root {
        candidates.push(root.to_path_buf());
    } else {
        let executable_dir = executable.and_then(Path::parent);
        if let Some(package) = executable_dir.and_then(Path::parent) {
            candidates.push(package.join("Resources/examples"));
            candidates.push(package.join("share/examples"));
        }
        for start in [executable_dir, current].into_iter().flatten() {
            candidates.extend(start.ancestors().map(|root| root.join("examples")));
        }
        if debug {
            candidates.push(fallback.join("examples"));
        }
    }
    for root in &candidates {
        if !root.as_os_str().is_empty()
            && root.join("ulx3s/README.md").is_file()
            && root.join("gallery/README.md").is_file()
        {
            return Ok(root.clone());
        }
    }
    anyhow::bail!(
        "Demo sources unavailable: {}; tried: {}; required files: ulx3s/README.md and gallery/README.md",
        if override_root.is_some() {
            "ECP5_EXAMPLES is set but invalid (no fallback)"
        } else {
            "ECP5_EXAMPLES is not set"
        },
        if candidates.is_empty() {
            "none".to_owned()
        } else {
            candidates
                .iter()
                .map(|root| format!("\"{}\"", root.display()))
                .collect::<Vec<_>>()
                .join(", ")
        }
    )
}

/// Shared creation-summary text for demos with external SD or flash input.
/// These runtime images are never imported into the copied project.
pub fn runtime_inputs(demo: &str) -> Option<&'static str> {
    match demo {
        "ulx3s/sd_read" => {
            Some("Runtime SD input uses ECP5_SD_IMAGE; no personal image was copied.")
        }
        "ulx3s/flash_dump" | "ulx3s/picorv32_flash" => {
            Some("Runtime flash input uses ECP5_FLASH_IMAGE; no personal image was copied.")
        }
        _ => None,
    }
}

/// Sorted `gallery/name` and `ulx3s/name` entries found by [`examples_root`].
/// Reads the optional `showcase.toml` in that examples directory as documented
/// in this module. Visibility does not affect which demos can be copied.
/// An installation without examples reports `Demo sources unavailable`.
pub fn catalogue() -> Result<Catalogue> {
    catalogue_in(&examples_root_checked()?)
}

/// Lists demos in an explicit examples directory containing `ulx3s/` and/or
/// `gallery/`, reading `root/showcase.toml` when present. Malformed tables are
/// errors naming that path; unknown demo names are diagnostic only.
/// Excluded folders carry a reason; no sources reports `Demo sources unavailable`.
/// Does not perform a root search or write files.
pub fn catalogue_in(root: &Path) -> Result<Catalogue> {
    let table = root.join("showcase.toml");
    let showcase = match fs::read_to_string(&table) {
        Ok(text) => Some(
            toml::from_str::<ShowcaseTable>(&text)
                .with_context(|| format!("reading showcase table {}", table.display()))?
                .showcase,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading showcase table {}", table.display()));
        }
    };
    let mut catalogue = Catalogue::default();
    for group in ["ulx3s", "gallery"] {
        let folder = root.join(group);
        if !folder.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&folder)? {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let verilog = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
            if path.join("ecp5.toml").is_file()
                || verilog
                    .iter()
                    .any(|e| e.path().extension().is_some_and(|s| s == "v") && e.path().is_file())
            {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .with_context(|| format!("demo name is not UTF-8: {}", path.display()))?;
                let demo = format!("{group}/{name}");
                let lpfs = root_lpfs(&path)?;
                if !path.join("ecp5.toml").is_file()
                    && lpfs.len() > 1
                    && !lpfs.contains(&PathBuf::from("top.lpf"))
                {
                    catalogue.excluded.push((
                        demo,
                        format!(
                            "no ecp5.toml and several root LPFs without top.lpf: {}",
                            lpfs.iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                } else {
                    catalogue.demos.push(Demo {
                        showcase: showcase.as_ref().is_none_or(|names| names.contains(&demo)),
                        name: demo,
                        group: group.into(),
                    });
                }
            }
        }
    }
    catalogue.demos.sort_by(|a, b| a.name.cmp(&b.name));
    catalogue.excluded.sort();
    if let Some(showcase) = showcase {
        catalogue.unknown_showcase = showcase
            .into_iter()
            .filter(|name| !catalogue.demos.iter().any(|demo| &demo.name == name))
            .collect();
    }
    ensure!(
        !catalogue.demos.is_empty() || !catalogue.excluded.is_empty(),
        "Demo sources unavailable"
    );
    Ok(catalogue)
}

fn root_lpfs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut lpfs = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "lpf") && path.is_file() {
            lpfs.push(PathBuf::from(
                path.file_name().context("LPF has no filename")?,
            ));
        }
    }
    lpfs.sort();
    Ok(lpfs)
}

/// Copies a demo, validates it and publishes without overwriting an occupied
/// destination. An existing empty directory is allowed, as for New project.
/// Any copyable demo name is accepted, including demos hidden outside Dev mode.
pub fn create(options: &Options) -> Result<Project> {
    create_in(&examples_root_checked()?, options)
}

struct Entry {
    source: PathBuf,
    relative: PathBuf,
    directory: bool,
}

fn resolve(path: &Path, root: &Path) -> Result<PathBuf> {
    let resolved = fs::canonicalize(path).with_context(|| {
        format!(
            "resolving demo source (broken link or cycle): {}",
            path.display()
        )
    })?;
    ensure!(
        resolved.starts_with(root),
        "demo source outside bundled examples: {} -> {}",
        path.display(),
        resolved.display()
    );
    Ok(resolved)
}

fn cache(name: &std::ffi::OsStr) -> bool {
    [
        "build",
        "target",
        "obj_dir",
        "node_modules",
        "__pycache__",
        ".cache",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".flow",
        ".Xil",
    ]
    .iter()
    .any(|s| name == *s)
}

fn metadata(name: &std::ffi::OsStr) -> bool {
    [
        ".git",
        ".hg",
        ".svn",
        ".bzr",
        ".jj",
        "_darcs",
        "CVS",
        ".DS_Store",
    ]
    .iter()
    .any(|s| name == *s)
}

fn visit(
    path: &Path,
    relative: &Path,
    root: &Path,
    build: &Path,
    active: &mut BTreeSet<PathBuf>,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    if path.file_name().is_some_and(metadata) {
        return Ok(());
    }
    let source = resolve(path, root)?;
    let info = fs::metadata(&source)?;
    if info.is_dir()
        && (path.file_name().is_some_and(cache)
            || source
                .strip_prefix(root)?
                .components()
                .any(|c| cache(c.as_os_str()))
            || source.starts_with(build))
    {
        return Ok(());
    }
    ensure!(
        info.is_dir() || info.is_file(),
        "unsupported special demo file: {}",
        path.display()
    );
    entries.push(Entry {
        source: source.clone(),
        relative: relative.into(),
        directory: info.is_dir(),
    });
    if info.is_dir() {
        ensure!(
            active.insert(source.clone()),
            "demo symlink cycle: {}",
            path.display()
        );
        let mut children = fs::read_dir(&source)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            visit(
                &child.path(),
                &relative.join(child.file_name()),
                root,
                build,
                active,
                entries,
            )?;
        }
        active.remove(&source);
    }
    Ok(())
}

fn patch_name(text: &str, name: &str) -> Result<String> {
    let document = toml_edit::Document::parse(text).context("reading demo manifest")?;
    let value = toml_edit::Value::from(name);
    if let Some(span) = document
        .get("project")
        .and_then(|p| p.get("name"))
        .and_then(toml_edit::Item::span)
    {
        // Use the parser's value span so even CRLF and unusual spacing outside
        // this one string remain byte-for-byte identical.
        let mut patched = text.to_owned();
        patched.replace_range(span, &value.to_string());
        return Ok(patched);
    }
    let mut document = document.into_mut();
    if !document.contains_key("project") {
        document["project"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    document["project"]["name"] = toml_edit::value(name);
    Ok(document.to_string())
}

fn generated_manifest(name: &str, lpfs: &[PathBuf]) -> Result<String> {
    let mut text = patch_name("", name)?;
    if lpfs.len() > 1 {
        let alternatives = lpfs
            .iter()
            .filter(|p| *p != Path::new("top.lpf"))
            .map(|p| toml_edit::Value::from(p.to_string_lossy().as_ref()).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "\n[sources]\n# Alternative constraint variants: {alternatives}.\n\
             # Set sources.constraints to one of these filenames to switch variants.\n\
             constraints = \"top.lpf\"\n"
        ));
    }
    Ok(text)
}

fn materialized_comments(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            if line.trim_start().starts_with('#') {
                line.replace("is a symlink to", "is a copy of")
                    .replace("are symlinks to", "are copies of")
                    .replace("are symlinks into", "are copies from")
            } else {
                line.to_owned()
            }
        })
        .collect()
}

fn notices(demo: &str, entries: &[Entry]) -> Result<String> {
    let mut text = format!(
        "Source demo: examples/{demo}\n\nCopied material:\n\
         - Repository demo RTL, firmware, scripts and documentation: Apache-2.0.\n\
         - ULX3S pin constraints (*.lpf): copyright 2016-2018 EMARD, ULX3S licence.\n"
    );
    let mut core_notices = String::new();
    for entry in entries.iter().filter(|e| !e.directory) {
        if entry
            .relative
            .file_name()
            .is_some_and(|n| n == "picorv32.v")
        {
            let source = fs::read_to_string(&entry.source)?;
            let end = source.find("*/").context("PicoRV32 ISC header missing")? + 2;
            ensure!(source.starts_with("/*"), "PicoRV32 ISC header missing");
            text.push_str(&format!("- {}: PicoRV32, ISC.\n", entry.relative.display()));
            core_notices.push_str(&format!("\nPicoRV32 ISC notice\n{}\n", &source[..end]));
        }
        if entry.relative == Path::new("serv/LICENSE") {
            text.push_str("- serv/ and materialized serv_*.v: SERV, ISC.\n");
            core_notices.push_str(&format!(
                "\nSERV ISC notice\n{}\n",
                fs::read_to_string(&entry.source)?
            ));
        }
    }
    text.push_str(&format!(
        "\nRepository Apache-2.0 licence\n{APACHE}\n\nULX3S constraints notice\n{ULX3S}\n"
    ));
    text.push_str(&core_notices);
    Ok(text)
}

/// Copies from an explicit examples directory containing `ulx3s/` and/or
/// `gallery/`, validating before publication under the same rules as [`create`].
/// Shared source links must stay within this directory. Does not search for
/// another root, run tools or import runtime SD/flash images.
/// The demo does not have to be listed in `showcase.toml`.
pub fn create_in(examples: &Path, options: &Options) -> Result<Project> {
    let mut destination = new_project::Options::new(&options.dir);
    destination.name.clone_from(&options.name);
    destination.validate()?;
    let catalogue = catalogue_in(examples)?;
    if let Some((_, reason)) = catalogue
        .excluded
        .iter()
        .find(|(demo, _)| demo == &options.demo)
    {
        anyhow::bail!("demo {:?} unavailable: {reason}", options.demo);
    }
    ensure!(
        catalogue.demos.iter().any(|demo| demo.name == options.demo),
        "unknown demo {:?}",
        options.demo
    );
    let root = fs::canonicalize(examples)?;
    let source = resolve(&root.join(&options.demo), &root)?;
    for name in ["ecp5.toml", "ecpbram.txt"] {
        let path = source.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            resolve(&path, &root)?;
        }
    }
    let build = if source.join("ecp5.toml").exists() {
        Project::open(&source)
            .context("opening source demo")?
            .build_dir
    } else {
        let build = source.join("build");
        if build.exists() {
            fs::canonicalize(build)?
        } else {
            build
        }
    };
    let mut entries = Vec::new();
    visit(
        &source,
        Path::new(""),
        &root,
        &build,
        &mut BTreeSet::new(),
        &mut entries,
    )?;
    let notices = notices(&options.demo, &entries)?;
    let parent = options
        .dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).context("creating project parent folder")?;
    let stage = new_project::Staging::new(parent)?;
    for entry in &entries {
        let path = stage.path.join(&entry.relative);
        if entry.directory {
            fs::create_dir_all(&path)?;
        } else {
            fs::copy(&entry.source, &path)
                .with_context(|| format!("copying demo source {}", entry.source.display()))?;
        }
    }
    let manifest = stage.path.join("ecp5.toml");
    let text = if manifest.exists() {
        patch_name(&fs::read_to_string(&manifest)?, &options.name)?
    } else {
        generated_manifest(&options.name, &root_lpfs(&source)?)?
    };
    fs::write(&manifest, materialized_comments(&text))?;
    fs::write(stage.path.join("THIRD_PARTY_NOTICES.txt"), notices)?;
    Project::open(&stage.path).context("validating copied demo")?;
    stage.publish(&options.dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;

    fn fixture(folder: &Folder) -> (PathBuf, PathBuf, Options) {
        let examples = folder.0.join("examples");
        let source = examples.join("ulx3s/demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("top.v"), "module top; endmodule\n").unwrap();
        fs::write(source.join("top.lpf"), "").unwrap();
        let options = Options::new(folder.0.join("copy with spaces"), "ulx3s/demo");
        (examples, source, options)
    }

    fn names(catalogue: &Catalogue, dev: bool) -> Vec<&str> {
        catalogue
            .visible(dev)
            .map(|demo| demo.name.as_str())
            .collect()
    }

    fn examples_at(root: &Path) {
        for marker in ["ulx3s/README.md", "gallery/README.md"] {
            let path = root.join(marker);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "# Examples\n").unwrap();
        }
    }

    #[test]
    fn root_search_prefers_executable_then_current_then_fallback() {
        let folder = Folder::new();
        let installed = folder.0.join("executable checkout");
        let current = folder.0.join("current checkout");
        let fallback = folder.0.join("compile-time checkout");
        for root in [&installed, &current, &fallback] {
            examples_at(&root.join("examples"));
        }
        let executable = installed.join("bin/tools/ecp5_emu");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "").unwrap();
        let working = current.join("nested/folder");
        fs::create_dir_all(&working).unwrap();
        assert_eq!(
            find_examples_root(Some(&executable), Some(&working), None, &fallback, true).unwrap(),
            installed.join("examples")
        );
        assert_eq!(
            find_examples_root(None, Some(&working), None, &fallback, true).unwrap(),
            current.join("examples")
        );
        assert_eq!(
            find_examples_root(None, None, None, &fallback, true).unwrap(),
            fallback.join("examples")
        );
        assert!(find_examples_root(None, None, None, &fallback, false).is_err());
        assert!(find_examples_root(None, None, None, &folder.0.join("missing"), true).is_err());
    }

    #[test]
    fn root_search_prefers_explicit_override() {
        let folder = Folder::new();
        let override_root = folder.0.join("explicit examples");
        examples_at(&override_root);
        let package = folder.0.join("package");
        for path in ["Resources/examples", "share/examples", "examples"] {
            examples_at(&package.join(path));
        }
        let fallback = folder.0.join("compile-time checkout");
        examples_at(&fallback.join("examples"));
        let executable = package.join("bin/ecp5_emu");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        assert_eq!(
            find_examples_root(
                Some(&executable),
                Some(&fallback),
                Some(&override_root),
                &fallback,
                true,
            )
            .unwrap(),
            override_root
        );
    }

    #[test]
    fn root_search_invalid_override_never_falls_back() {
        let folder = Folder::new();
        let package = folder.0.join("package");
        for path in ["Resources/examples", "share/examples", "examples"] {
            examples_at(&package.join(path));
        }
        let executable = package.join("bin/ecp5_emu");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let stale = folder.0.join("stale examples");
        fs::create_dir_all(stale.join("ulx3s")).unwrap();
        for invalid in [folder.0.join("missing"), stale, PathBuf::new()] {
            let error = find_examples_root(
                Some(&executable),
                Some(&package),
                Some(&invalid),
                &package,
                true,
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("ECP5_EXAMPLES is set but invalid (no fallback)"),
                "{error}"
            );
            assert!(
                error.contains(&format!("tried: \"{}\"", invalid.display())),
                "{error}"
            );
            assert!(!error.contains(&package.display().to_string()), "{error}");
        }
    }

    #[test]
    fn root_search_macos_resources_precede_share_and_checkout() {
        let folder = Folder::new();
        let contents = folder.0.join("ECP5 Emu.app/Contents");
        let resources = contents.join("Resources/examples");
        for root in [
            &resources,
            &contents.join("share/examples"),
            &folder.0.join("examples"),
        ] {
            examples_at(root);
        }
        let executable = contents.join("MacOS/ecp5_gui");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        assert_eq!(
            find_examples_root(Some(&executable), Some(&folder.0), None, &folder.0, false).unwrap(),
            resources
        );
    }

    #[test]
    fn root_search_linux_share_precedes_checkout() {
        let folder = Folder::new();
        let share = folder.0.join("share/examples");
        examples_at(&share);
        examples_at(&folder.0.join("examples"));
        let executable = folder.0.join("bin/ecp5_emu");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        assert_eq!(
            find_examples_root(Some(&executable), Some(&folder.0), None, &folder.0, false).unwrap(),
            share
        );
    }

    #[test]
    fn root_search_skips_stale_examples_and_requires_both_marker_files() {
        let folder = Folder::new();
        let examples = folder.0.join("examples");
        examples_at(&examples);
        let executable = folder.0.join("target/release/ecp5_emu");
        fs::create_dir_all(executable.parent().unwrap().join("examples/ulx3s")).unwrap();
        let stale = folder.0.join("target/examples");
        examples_at(&stale);
        for missing in ["ulx3s/README.md", "gallery/README.md"] {
            fs::remove_file(stale.join(missing)).unwrap();
            assert_eq!(
                find_examples_root(Some(&executable), None, None, &folder.0, false).unwrap(),
                examples
            );
            fs::create_dir(stale.join(missing)).unwrap();
            assert_eq!(
                find_examples_root(Some(&executable), None, None, &folder.0, false).unwrap(),
                examples
            );
            fs::remove_dir(stale.join(missing)).unwrap();
            fs::write(stale.join(missing), "# Examples\n").unwrap();
        }
    }

    #[test]
    fn root_search_error_lists_tried_locations_and_override_variable() {
        let folder = Folder::new();
        let executable = folder.0.join("package/bin/ecp5_emu");
        let working = folder.0.join("checkout/nested");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(&working).unwrap();
        let fallback = folder.0.join("compile-time checkout");
        let error = find_examples_root(Some(&executable), Some(&working), None, &fallback, true)
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("Demo sources unavailable: ECP5_EXAMPLES is not set; tried: "),
            "{error}"
        );
        let mut position = 0;
        for path in [
            "package/Resources/examples",
            "package/share/examples",
            "package/bin/examples",
            "package/examples",
            "checkout/nested/examples",
            "checkout/examples",
            "compile-time checkout/examples",
        ] {
            let location = format!("\"{}\"", folder.0.join(path).display());
            position += error[position..]
                .find(&location)
                .unwrap_or_else(|| panic!("missing {location}: {error}"))
                + location.len();
        }
        assert!(
            error.contains("required files: ulx3s/README.md and gallery/README.md"),
            "{error}"
        );
        let release_error =
            find_examples_root(Some(&executable), Some(&working), None, &fallback, false)
                .unwrap_err()
                .to_string();
        assert!(
            !release_error.contains(&fallback.display().to_string()),
            "{release_error}"
        );
    }

    #[test]
    fn runtime_inputs_identify_sd_and_flash_demos() {
        assert_eq!(
            runtime_inputs("ulx3s/sd_read"),
            Some("Runtime SD input uses ECP5_SD_IMAGE; no personal image was copied.")
        );
        for demo in ["ulx3s/flash_dump", "ulx3s/picorv32_flash"] {
            assert_eq!(
                runtime_inputs(demo),
                Some("Runtime flash input uses ECP5_FLASH_IMAGE; no personal image was copied.")
            );
        }
        for demo in [
            "ulx3s/uart_hello",
            "gallery/riscv_shell",
            "sd_read",
            "unknown",
        ] {
            assert_eq!(runtime_inputs(demo), None);
        }
    }

    #[test]
    fn catalogue_without_table_makes_both_groups_showcase() {
        let folder = Folder::new();
        let (examples, _, _) = fixture(&folder);
        fs::create_dir_all(examples.join("gallery/manifest")).unwrap();
        fs::write(examples.join("gallery/manifest/ecp5.toml"), "").unwrap();
        fs::create_dir_all(examples.join("gallery/docs")).unwrap();
        let catalogue = catalogue_in(&examples).unwrap();
        assert_eq!(names(&catalogue, false), ["gallery/manifest", "ulx3s/demo"]);
        assert_eq!(names(&catalogue, true), names(&catalogue, false));
        assert!(catalogue.demos.iter().all(|demo| demo.showcase));
        assert_eq!(catalogue.demos[0].group, "gallery");
        assert_eq!(catalogue.demos[1].group, "ulx3s");
        assert!(catalogue.unknown_showcase.is_empty());
        assert_eq!(
            catalogue_in(&folder.0.join("absent"))
                .unwrap_err()
                .to_string(),
            "Demo sources unavailable"
        );
        assert!(!folder.0.join("absent").exists());
    }

    #[test]
    fn showcase_table_selects_two_demos_and_dev_shows_every_demo() {
        let folder = Folder::new();
        let (examples, _, _) = fixture(&folder);
        for demo in ["gallery/featured", "ulx3s/development"] {
            let source = examples.join(demo);
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("top.v"), "module top; endmodule\n").unwrap();
        }
        fs::write(
            examples.join("showcase.toml"),
            "# Default demos\nshowcase = ['ulx3s/demo', 'gallery/featured']\n",
        )
        .unwrap();
        let catalogue = catalogue_in(&examples).unwrap();
        assert_eq!(names(&catalogue, false), ["gallery/featured", "ulx3s/demo"]);
        assert_eq!(
            names(&catalogue, true),
            ["gallery/featured", "ulx3s/demo", "ulx3s/development"]
        );
        assert!(!catalogue.demos[2].showcase);
        assert!(catalogue.unknown_showcase.is_empty());
    }

    #[test]
    fn unknown_showcase_names_are_diagnostics_without_hiding_valid_demos() {
        let folder = Folder::new();
        let (examples, _, _) = fixture(&folder);
        fs::create_dir_all(examples.join("gallery/docs")).unwrap();
        fs::write(
            examples.join("showcase.toml"),
            "showcase = ['ulx3s/missing', 'ulx3s/demo', 'gallery/docs']\n",
        )
        .unwrap();
        let catalogue = catalogue_in(&examples).unwrap();
        assert_eq!(
            catalogue.unknown_showcase,
            ["gallery/docs", "ulx3s/missing"]
        );
        assert_eq!(names(&catalogue, false), ["ulx3s/demo"]);
        assert_eq!(names(&catalogue, true), ["ulx3s/demo"]);
    }

    #[test]
    fn malformed_showcase_tables_name_the_file() {
        let folder = Folder::new();
        let (examples, _, _) = fixture(&folder);
        let table = examples.join("showcase.toml");
        for text in [
            "showcase = [",
            "showcase = []\nunknown = true\n",
            "showcase = 'ulx3s/demo'\n",
            "showcase = [42]\n",
            "",
        ] {
            fs::write(&table, text).unwrap();
            let error = catalogue_in(&examples).unwrap_err().to_string();
            assert!(
                error.contains(&table.display().to_string()),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn non_showcase_demo_can_still_be_copied() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        fs::write(examples.join("showcase.toml"), "showcase = []\n").unwrap();
        let catalogue = catalogue_in(&examples).unwrap();
        assert!(names(&catalogue, false).is_empty());
        assert_eq!(names(&catalogue, true), ["ulx3s/demo"]);
        let project = create_in(&examples, &options).unwrap();
        assert_eq!(
            fs::read(project.root.join("top.v")).unwrap(),
            fs::read(source.join("top.v")).unwrap()
        );
    }

    #[test]
    fn several_lpfs_select_top_and_document_alternatives() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        fs::write(source.join("top_pullup.lpf"), "# alternate\n").unwrap();
        assert_eq!(
            names(&catalogue_in(&examples).unwrap(), true),
            ["ulx3s/demo"]
        );
        let project = create_in(&examples, &options).unwrap();
        assert_eq!(project.constraints, Path::new("top.lpf"));
        let manifest = fs::read_to_string(project.root.join("ecp5.toml")).unwrap();
        assert!(manifest.contains("# Alternative constraint variants: \"top_pullup.lpf\"."));
        assert!(manifest
            .contains("# Set sources.constraints to one of these filenames to switch variants."));
        assert_eq!(
            fs::read(project.root.join("top_pullup.lpf")).unwrap(),
            b"# alternate\n"
        );
        assert!(!source.join("ecp5.toml").exists());
    }

    #[test]
    fn several_lpfs_without_top_are_excluded_with_reason() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        fs::rename(source.join("top.lpf"), source.join("down.lpf")).unwrap();
        fs::write(source.join("up.lpf"), "").unwrap();
        let catalogue = catalogue_in(&examples).unwrap();
        assert!(catalogue.demos.is_empty());
        assert_eq!(catalogue.excluded.len(), 1);
        let (demo, reason) = &catalogue.excluded[0];
        assert_eq!(demo, "ulx3s/demo");
        assert!(reason.contains("without top.lpf"), "{reason}");
        assert!(reason.contains("down.lpf, up.lpf"), "{reason}");
        assert!(create_in(&examples, &options)
            .unwrap_err()
            .to_string()
            .contains(reason));
        assert!(!options.dir.exists());
    }

    #[test]
    fn manifest_changes_only_name_bytes() {
        for original in [
            "# before\n[project] # table\nname  = 'old'  # name\ntop = 'custom'\n\n[sources]\n# keep\nverilog = [\n  'top.v', # source\n]\n",
            "# before\r\n[project]\r\nname = \"old\" # after\r\ntop='top'\r\n",
            "project = { name = 'old', top = 'top' } # inline\n",
            "project.name = 'old' # dotted\nproject.top = 'top'\n",
        ] {
            let name = "My \"quoted\" \\ project";
            let patched = patch_name(original, name).unwrap();
            let old = if original.contains("'old'") { "'old'" } else { "\"old\"" };
            assert_eq!(patched, original.replacen(old, &toml_edit::Value::from(name).to_string(), 1));
            let parsed: toml::Value = toml::from_str(&patched).unwrap();
            assert_eq!(parsed["project"]["name"].as_str(), Some(name));
        }
        assert_eq!(
            patch_name("", "new").unwrap(),
            "[project]\nname = \"new\"\n"
        );
        let original = "# existing\n[sources]\nverilog = ['top.v'] # keep\n";
        assert!(patch_name(original, "new").unwrap().starts_with(original));
    }

    #[test]
    fn only_symlink_comments_are_updated() {
        let original = "# picorv32.v is a symlink to shared/core.v.\n# keep this comment\nname = 'is a symlink to' # keep\n# serv_*.v are symlinks into serv/.\n";
        assert_eq!(materialized_comments(original), "# picorv32.v is a copy of shared/core.v.\n# keep this comment\nname = 'is a symlink to' # keep\n# serv_*.v are copies from serv/.\n");
    }

    #[test]
    fn skips_build_custom_outputs_and_caches_but_keeps_images() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        fs::write(
            source.join("ecp5.toml"),
            "[build]\ndir = 'outputs/routed'\n",
        )
        .unwrap();
        for dir in [
            "build",
            "sim/build",
            "outputs/routed",
            "firmware/__pycache__",
            ".git",
            "obj_dir",
            ".flow",
        ] {
            fs::create_dir_all(source.join(dir)).unwrap();
            fs::write(source.join(dir).join("cached"), "stale").unwrap();
        }
        for file in [
            "payload.bin",
            "firmware.hex",
            "ram.mem",
            "rom.init",
            "README.md",
            "check.json",
            "gui.script",
        ] {
            fs::write(source.join(file), "committed input\n").unwrap();
        }
        for dir in ["expected", "sim", "repro", "firmware"] {
            fs::create_dir_all(source.join(dir)).unwrap();
            fs::write(source.join(dir).join("input"), "keep").unwrap();
        }
        let project = create_in(&examples, &options).unwrap();
        for dir in [
            "build",
            "sim/build",
            "outputs/routed",
            "firmware/__pycache__",
            ".git",
            "obj_dir",
            ".flow",
        ] {
            assert!(!project.root.join(dir).exists(), "{dir}");
        }
        for file in [
            "payload.bin",
            "firmware.hex",
            "ram.mem",
            "rom.init",
            "README.md",
            "check.json",
            "gui.script",
            "expected/input",
            "sim/input",
            "repro/input",
            "firmware/input",
        ] {
            assert_eq!(
                fs::read(project.root.join(file)).unwrap(),
                fs::read(source.join(file)).unwrap()
            );
        }
    }

    #[test]
    fn occupied_destination_is_refused_and_empty_is_allowed() {
        let folder = Folder::new();
        let (examples, _, options) = fixture(&folder);
        fs::create_dir(&options.dir).unwrap();
        fs::write(options.dir.join("mine"), "keep").unwrap();
        assert!(create_in(&examples, &options)
            .unwrap_err()
            .to_string()
            .contains("not empty"));
        assert_eq!(
            fs::read_to_string(options.dir.join("mine")).unwrap(),
            "keep"
        );
        fs::remove_file(options.dir.join("mine")).unwrap();
        assert_eq!(
            create_in(&examples, &options).unwrap().name,
            "copy with spaces"
        );
    }

    #[cfg(unix)]
    fn link(target: impl AsRef<Path>, path: impl AsRef<Path>) -> bool {
        if let Err(error) = std::os::unix::fs::symlink(target, path) {
            eprintln!("skipping symlink fixture: filesystem refused it: {error}");
            return false;
        }
        true
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_become_real_files_and_directories_with_executable_bits() {
        use std::os::unix::fs::PermissionsExt;
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        let shared = examples.join("ulx3s/shared");
        fs::create_dir(&shared).unwrap();
        fs::write(shared.join("script.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(shared.join("script.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        if !link("../shared/script.sh", source.join("script.sh"))
            || !link("../shared", source.join("shared"))
        {
            return;
        }
        let project = create_in(&examples, &options).unwrap();
        for file in ["script.sh", "shared/script.sh"] {
            let info = fs::symlink_metadata(project.root.join(file)).unwrap();
            assert!(info.is_file());
            assert_eq!(info.permissions().mode() & 0o111, 0o111);
            assert_eq!(
                fs::read_to_string(project.root.join(file)).unwrap(),
                "#!/bin/sh\n"
            );
        }
        assert!(fs::symlink_metadata(project.root.join("shared"))
            .unwrap()
            .is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_cycles_broken_links_and_outside_targets_before_publication() {
        for (target, expected) in [
            (".", "cycle"),
            ("loop", "cycle"),
            ("missing", "broken link"),
            ("../../../top.v", "outside bundled examples"),
        ] {
            let folder = Folder::new();
            let (examples, source, mut options) = fixture(&folder);
            options.dir = folder.0.join("absent parent/copy");
            if !link(target, source.join("loop")) {
                return;
            }
            let error = format!("{:#}", create_in(&examples, &options).unwrap_err());
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("loop"), "{error}");
            assert!(!options.dir.parent().unwrap().exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn skips_alias_of_resolved_custom_build_directory() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        fs::create_dir(source.join("outputs")).unwrap();
        fs::write(source.join("outputs/top.config"), "cached").unwrap();
        if !link("outputs", source.join("route")) || !link("outputs", source.join("alias")) {
            return;
        }
        fs::write(source.join("ecp5.toml"), "[build]\ndir = 'route'\n").unwrap();
        let project = create_in(&examples, &options).unwrap();
        for path in ["outputs", "route", "alias"] {
            assert!(!project.root.join(path).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_special_files_before_publication() {
        let folder = Folder::new();
        let (examples, source, options) = fixture(&folder);
        let socket = source.join("socket");
        let _listener = match std::os::unix::net::UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("skipping socket fixture: filesystem refused it: {error}");
                return;
            }
        };
        let error = create_in(&examples, &options).unwrap_err().to_string();
        assert!(error.contains("special demo file"), "{error}");
        assert!(error.contains("socket"), "{error}");
        assert!(!options.dir.exists());
    }
}
