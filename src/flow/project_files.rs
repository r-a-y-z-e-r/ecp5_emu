//! Project file creation shared by windows and scripts. Membership follows the
//! loader's discovery rules; generated HDL is never instantiated automatically.
use super::{HdlLanguage, Project};
use anyhow::{bail, ensure, Context, Result};
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Component, Path, PathBuf},
};
use toml_edit::{Document, Item, Value};

/// Optional name for a new HDL module/entity. Otherwise use the filename stem.
/// A supplied name for a non-HDL file is an error.
#[derive(Clone, Debug, Default)]
pub struct NewFile {
    pub name: Option<String>,
}

/// Published file and human-readable membership changes, suitable for records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewFileOutcome {
    /// Absolute path inside the canonical project root.
    pub path: PathBuf,
    /// Module/entity identifier; absent for constraints and auxiliary data.
    pub name: Option<String>,
    /// Every manifest change, in the order applied. Empty for default discovery.
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Verilog,
    SystemVerilog,
    Vhdl,
    Constraints,
    Hex,
}
impl Kind {
    fn from_path(path: &Path) -> Result<Self> {
        Ok(match path.extension().and_then(|s| s.to_str()) {
            Some("v") => Self::Verilog,
            Some("sv") => Self::SystemVerilog,
            Some("vhd" | "vhdl") => Self::Vhdl,
            Some("lpf") => Self::Constraints,
            Some("hex") => Self::Hex,
            _ => bail!(
                "unsupported new file extension (expected .v, .sv, .vhd, .vhdl, .lpf or .hex)"
            ),
        })
    }

    fn is_hdl(self) -> bool {
        matches!(self, Self::Verilog | Self::SystemVerilog | Self::Vhdl)
    }
}

/// Creates a project-relative `.v`, `.sv`, `.vhd`/`.vhdl`, `.lpf` or `.hex`.
///
/// Missing parent directories are created. Absolute paths, `..`, escaping
/// symlink parents, build output paths and occupied names are refused. HDL names
/// must be ordinary identifiers, not reserved words. LPFs remain inactive; hex
/// data joins auxiliary files, never memory images. Legacy roots are promoted
/// to ordered `sources.hdl` when adding SystemVerilog. VHDL roots cannot mix with
/// Verilog/SystemVerilog. An empty project may acquire its first HDL root, but
/// must satisfy all other requirements of [`Project::open`] after creation.
///
/// Reads a fresh manifest and patches only affected spans with `toml_edit`.
/// Writes the file and any manifest changes, then reopens the project. Ordinary
/// failures restore the original manifest bytes and remove the new file and
/// directories. This is not a crash-atomic transaction. Callers must resolve a
/// dirty manifest buffer first, reload it afterwards, and serialize file actions.
/// On success, reopen inputs and invalidate any previous build association.
pub fn new_file(project_root: &Path, relative: &Path, options: &NewFile) -> Result<NewFileOutcome> {
    ensure!(project_root.is_dir(), "expected a project folder");
    let root = fs::canonicalize(project_root).context("opening project folder")?;
    let relative = local_path(relative)?;
    let kind = Kind::from_path(&relative)?;
    let name = if kind.is_hdl() {
        let name = options
            .name
            .as_deref()
            .or_else(|| relative.file_stem().and_then(|s| s.to_str()))
            .context("HDL filename must have a UTF-8 identifier")?;
        identifier(name, kind)?;
        Some(name.to_owned())
    } else {
        ensure!(
            options.name.is_none(),
            "only HDL files have a module/entity name"
        );
        None
    };
    let manifest = root.join("ecp5.toml");
    let original = read_optional(&manifest)?;
    let text = std::str::from_utf8(original.as_deref().unwrap_or_default())
        .context("manifest is not UTF-8")?;
    let document = Document::parse(text).context("reading project manifest")?;
    let project = match Project::open(&root) {
        Ok(project) => Some(project),
        // Project::open requires roots. Permit bootstrapping only when the
        // selected list/discovery is actually empty, then validate everything
        // with the real loader once the first root is published.
        Err(_) if kind.is_hdl() && roots_empty(&root, &document)? => None,
        Err(error) => return Err(error),
    };
    let build = match &project {
        Some(project) => project.build_dir.clone(),
        None => resolve(
            &root.join(
                document
                    .get("build")
                    .and_then(|b| b.get("dir"))
                    .and_then(Item::as_str)
                    .unwrap_or("build"),
            ),
        )?,
    };
    let path = root.join(&relative);
    check_destination(&root, &path, &build)?;
    let roots = project.as_ref().map_or(&[][..], |p| p.verilog.as_slice());
    let has_vhdl = roots
        .iter()
        .any(|p| HdlLanguage::from_path(p).is_ok_and(|l| l == HdlLanguage::Vhdl));
    if kind == Kind::Vhdl {
        ensure!(
            roots.is_empty() || has_vhdl,
            "a VHDL file cannot join Verilog/SystemVerilog sources yet (cross-language hierarchy is not supported)"
        );
    } else if kind.is_hdl() {
        ensure!(
            !has_vhdl,
            "a Verilog/SystemVerilog file cannot join VHDL sources yet (cross-language hierarchy is not supported)"
        );
    }

    let mut patched = text.to_owned();
    let mut notes = Vec::new();
    let root_level = relative.parent() == Some(Path::new(""));
    // A local symlink to the root also participates in root LPF discovery.
    let lpf_at_root = kind == Kind::Constraints
        && resolve(path.parent().context("new file has no parent")?)? == root;
    let explicit = |key| document.get("sources").and_then(|s| s.get(key)).is_some();
    let relative_text = relative.to_str().context("new file path must be UTF-8")?;
    match kind {
        Kind::Verilog if explicit("hdl") => {
            append(&mut patched, "hdl", relative_text)?;
            notes.push("sources.hdl appended".into());
        }
        Kind::Verilog if explicit("verilog") => {
            append(&mut patched, "verilog", relative_text)?;
            notes.push("sources.verilog appended".into());
        }
        Kind::Verilog if !root_level => {
            set_list(&mut patched, "verilog", roots, relative_text)?;
            notes.push("sources.verilog made explicit".into());
        }
        Kind::SystemVerilog | Kind::Vhdl if explicit("hdl") => {
            append(&mut patched, "hdl", relative_text)?;
            notes.push("sources.hdl appended".into());
        }
        Kind::SystemVerilog | Kind::Vhdl => {
            if explicit("verilog") {
                rename_verilog(&mut patched)?;
                append(&mut patched, "hdl", relative_text)?;
                notes.push("sources.hdl created from sources.verilog".into());
            } else {
                set_list(&mut patched, "hdl", roots, relative_text)?;
                notes.push("sources.hdl created from default Verilog discovery".into());
            }
        }
        Kind::Constraints if lpf_at_root && !explicit("constraints") => {
            let current = &project
                .as_ref()
                .context("missing project inputs")?
                .constraints;
            set_source(&mut patched, "constraints", &path_value(current)?)?;
            notes.push("sources.constraints made explicit".into());
        }
        Kind::Hex if explicit("files") => {
            append(&mut patched, "files", relative_text)?;
            notes.push("sources.files appended".into());
        }
        Kind::Hex if !root_level => {
            let files = &project.as_ref().context("missing project inputs")?.files;
            set_list(&mut patched, "files", files, relative_text)?;
            notes.push("sources.files made explicit".into());
        }
        _ => {}
    }

    let changed = patched != text;
    if changed && original.is_some() {
        ensure!(
            fs::canonicalize(&manifest)?.starts_with(&root),
            "cannot edit a manifest symlink outside the project; make a local copy first"
        );
    }
    ensure!(
        read_optional(&manifest)? == original,
        "manifest changed during file creation"
    );
    let contents = template(kind, name.as_deref().unwrap_or_default());
    let mut directories = Vec::new();
    let mut file_created = false;
    let mut manifest_written = false;
    let result = (|| -> Result<()> {
        create_parents(
            path.parent().context("new file has no parent")?,
            &mut directories,
        )?;
        check_destination(&root, &path, &build)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        file_created = true;
        file.write_all(contents.as_bytes())?;
        drop(file);
        ensure!(
            read_optional(&manifest)? == original,
            "manifest changed during file creation"
        );
        if changed {
            let mut options = OpenOptions::new();
            options.write(true);
            if original.is_some() {
                options.truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options
                .open(&manifest)
                .context("writing project manifest")?;
            manifest_written = true;
            file.write_all(patched.as_bytes())?;
            drop(file);
        }
        let reopened = Project::open(&root).context("validating new project file")?;
        ensure!(
            !resolve(&path)?.starts_with(&reopened.build_dir),
            "new file cannot be inside the build output folder"
        );
        if kind == Kind::Hex {
            ensure!(
                !reopened.memory_images.contains(&relative),
                "new hex data must not be a memory image"
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        if manifest_written {
            let rollback = match &original {
                Some(bytes) => fs::write(&manifest, bytes),
                None => fs::remove_file(&manifest),
            };
            if let Err(error) = rollback {
                failures.push(format!("restoring manifest: {error}"));
            }
        }
        if file_created {
            if let Err(error) = fs::remove_file(&path) {
                failures.push(format!("removing new file: {error}"));
            }
        }
        for directory in directories.iter().rev() {
            if let Err(error) = fs::remove_dir(directory) {
                failures.push(format!("removing {}: {error}", directory.display()));
            }
        }
        return if failures.is_empty() {
            Err(error)
        } else {
            Err(error.context(format!("rollback failed: {}", failures.join("; "))))
        };
    }
    Ok(NewFileOutcome { path, name, notes })
}

/// Membership changes for an existing BRAM image, suitable for UI status and records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub notes: Vec<String>,
}

/// Marks a local `.hex`, `.mem` or `.init` as a post-route ecpbram image,
/// or returns it to synthesis inputs. Explicit source lists and the legacy
/// `ecpbram.txt` list are kept in sync. Other manifest bytes and legacy comments
/// are preserved. Validate with the loader and roll back both files on failure.
/// Callers must resolve dirty manifest/legacy buffers first, refresh clean tabs
/// afterwards, reload project inputs and invalidate the previous build.
pub fn set_memory_image(
    project_root: &Path,
    relative: &Path,
    use_as_image: bool,
) -> Result<Outcome> {
    let root = fs::canonicalize(project_root).context("opening project folder")?;
    ensure!(root.is_dir(), "expected a project folder");
    let relative = local_path(relative)?;
    ensure!(
        matches!(
            relative.extension().and_then(|s| s.to_str()),
            Some("hex" | "mem" | "init")
        ),
        "BRAM images require a .hex, .mem or .init file"
    );
    let path = root.join(&relative);
    ensure!(path.is_file(), "memory image must be an existing file");
    ensure!(
        resolve(&path)?.starts_with(&root),
        "memory image symlink leaves the project"
    );
    let manifest = root.join("ecp5.toml");
    let legacy = root.join("ecpbram.txt");
    let original = read_optional(&manifest)?;
    let legacy_original = read_optional(&legacy)?;
    let text = std::str::from_utf8(original.as_deref().unwrap_or_default())?;
    let legacy_text = std::str::from_utf8(legacy_original.as_deref().unwrap_or_default())?;
    let doc = Document::parse(text)?;
    let project = Project::open(&root)?;
    ensure!(
        !resolve(&path)?.starts_with(&project.build_dir),
        "memory image cannot be inside the build output folder"
    );
    let explicit_images = doc.get("memory").and_then(|s| s.get("images")).is_some();
    let explicit_files = doc.get("sources").and_then(|s| s.get("files")).is_some();
    let relative_text = relative.to_str().context("project path must be UTF-8")?;
    ensure!(
        relative_text.trim() == relative_text
            && !relative_text.starts_with('#')
            && !relative_text.contains(['\r', '\n']),
        "image path cannot be represented in ecpbram.txt"
    );
    let mut patched = text.to_owned();
    let mut notes = Vec::new();
    let mut images = project.memory_images.clone();
    if !explicit_images {
        let mut array = toml_edit::Array::new();
        for image in &images {
            array.push(path_value(image)?);
        }
        set_table_value(&mut patched, "memory", "images", &Value::Array(array))?;
        notes.push(if legacy_original.is_some() {
            "memory.images made explicit from ecpbram.txt".into()
        } else {
            "memory.images made explicit".into()
        });
    }
    if use_as_image {
        if !images.contains(&relative) {
            append_to(&mut patched, "memory", "images", relative_text)?;
            images.push(relative.clone());
            notes.push(format!("memory.images appended {relative_text}"));
        }
        if explicit_files && remove_from(&mut patched, "sources", "files", &relative)? {
            notes.push(format!("sources.files removed {relative_text}"));
        }
    } else {
        if remove_from(&mut patched, "memory", "images", &relative)? {
            notes.push(format!("memory.images removed {relative_text}"));
        }
        images.retain(|p| p != &relative);
        if explicit_files && !project.files.contains(&relative) {
            append_to(&mut patched, "sources", "files", relative_text)?;
            notes.push(format!("sources.files appended {relative_text}"));
        } else if !explicit_files && project.memory_images.contains(&relative) {
            notes.push(if relative.parent() == Some(Path::new("")) {
                format!("{relative_text} rejoins default sources.files discovery")
            } else {
                format!("{relative_text} is no longer an image; subfolders are not discovered as sources")
            });
        }
    }
    let legacy_patched = image_list(legacy_text, &images)?;
    let updates = [
        (&manifest, &original, patched.as_bytes()),
        (&legacy, &legacy_original, legacy_patched.as_bytes()),
    ];
    for (path, original, candidate) in updates {
        if candidate != original.as_deref().unwrap_or_default() && original.is_some() {
            ensure!(resolve(path)?.starts_with(&root), "cannot edit a manifest or image-list symlink outside the project; make a local copy first");
        }
        ensure!(
            read_optional(path)? == *original,
            "project membership changed during edit"
        );
    }
    if legacy_patched != legacy_text {
        notes.push("ecpbram.txt updated".into());
    }
    let mut written = Vec::new();
    let result = (|| -> Result<()> {
        for (path, original, candidate) in updates {
            ensure!(
                read_optional(path)? == *original,
                "project membership changed during edit"
            );
            if candidate == original.as_deref().unwrap_or_default() {
                continue;
            }
            let mut options = OpenOptions::new();
            options.write(true);
            if original.is_some() {
                options.truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options.open(path).context("writing project membership")?;
            written.push((path, original));
            file.write_all(candidate)?;
        }
        Project::open(&root).context("validating memory image membership")?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for (path, original) in written.into_iter().rev() {
            let result = match original {
                Some(bytes) => fs::write(path, bytes),
                None => fs::remove_file(path),
            };
            if let Err(error) = result {
                failures.push(format!("restoring {}: {error}", path.display()));
            }
        }
        return if failures.is_empty() {
            Err(error)
        } else {
            Err(error.context(format!("rollback failed: {}", failures.join("; "))))
        };
    }
    Ok(Outcome { notes })
}

/// Keep matching legacy lines and all comments verbatim, inserting missing
/// images in manifest order. Remove only obsolete/duplicate image lines.
fn image_list(original: &str, images: &[PathBuf]) -> Result<String> {
    let nl = newline(original);
    let mut remaining = images.iter();
    let mut next = remaining.next();
    let mut text = String::new();
    for line in original.split_inclusive('\n') {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') {
            text.push_str(line);
        } else if let Some(index) = next
            .into_iter()
            .chain(remaining.clone())
            .position(|p| p == Path::new(value))
        {
            for _ in 0..index {
                text.push_str(
                    next.unwrap()
                        .to_str()
                        .context("project path must be UTF-8")?,
                );
                text.push_str(nl);
                next = remaining.next();
            }
            text.push_str(line);
            next = remaining.next();
        }
    }
    for image in next.into_iter().chain(remaining) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push_str(nl);
        }
        text.push_str(image.to_str().context("project path must be UTF-8")?);
        text.push_str(nl);
    }
    Ok(text)
}

/// Remove value tokens and their commas, leaving all comments and other bytes.
fn remove_from(text: &mut String, table: &str, key: &str, path: &Path) -> Result<bool> {
    let mut changed = false;
    loop {
        let doc = Document::parse(text.as_str())?;
        let array = doc
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(Item::as_array)
            .context("expected a path array")?;
        let Some((index, value)) = array
            .iter()
            .enumerate()
            .find(|(_, v)| v.as_str().is_some_and(|s| Path::new(s) == path))
        else {
            return Ok(changed);
        };
        let span = value.span().context("path value has no span")?;
        let gap = if index > 0 {
            array.get(index - 1).unwrap().span().unwrap().end..span.start
        } else {
            span.end
                ..array
                    .get(1)
                    .and_then(Value::span)
                    .map_or(array.span().unwrap().end - 1, |s| s.start)
        };
        let mut comment = false;
        let comma = text[gap.clone()].char_indices().find_map(|(i, c)| {
            if c == '\n' {
                comment = false;
            } else if c == '#' {
                comment = true;
            }
            (!comment && c == ',').then_some(gap.start + i)
        });
        let mut edits = vec![span];
        if let Some(comma) = comma {
            edits.push(comma..comma + 1);
        }
        edits.sort_by_key(|r| std::cmp::Reverse(r.start));
        for edit in edits {
            text.replace_range(edit, "");
        }
        changed = true;
    }
}

fn local_path(path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "project file path must be relative and cannot contain '..'"
    );
    let path: PathBuf = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    ensure!(
        path.file_name().is_some(),
        "project file path needs a filename"
    );
    Ok(path)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(fs::read(path)?)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Resolve existing ancestors even if the leaf or several parents are absent.
fn resolve(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(fs::canonicalize(path)?),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(resolve(
            path.parent().context("path has no existing ancestor")?,
        )?
        .join(path.file_name().context("path has no filename")?)),
        Err(error) => Err(error.into()),
    }
}

fn check_destination(root: &Path, path: &Path, build: &Path) -> Result<()> {
    let resolved = resolve(path.parent().context("new file has no parent")?)?
        .join(path.file_name().context("new file has no filename")?);
    ensure!(
        resolved.starts_with(root),
        "symlinked parent leaves the project"
    );
    ensure!(
        !resolved.starts_with(build),
        "new file cannot be inside the build output folder"
    );
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("new file already exists: {}", path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn create_parents(path: &Path, created: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    create_parents(path.parent().context("directory has no parent")?, created)?;
    fs::create_dir(path)?;
    created.push(path.to_owned());
    Ok(())
}

fn roots_empty(root: &Path, document: &Document<&str>) -> Result<bool> {
    for key in ["hdl", "verilog"] {
        if let Some(item) = document.get("sources").and_then(|s| s.get(key)) {
            return Ok(item.as_array().is_some_and(|a| a.is_empty()));
        }
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|s| s == "v") {
            return Ok(false);
        }
    }
    Ok(true)
}

fn path_value(path: &Path) -> Result<Value> {
    Ok(Value::from(
        path.to_str().context("project path must be UTF-8")?,
    ))
}

fn set_list(text: &mut String, key: &str, paths: &[PathBuf], addition: &str) -> Result<()> {
    let mut array = toml_edit::Array::new();
    for path in paths {
        array.push(path_value(path)?);
    }
    array.push(addition);
    set_source(text, key, &Value::Array(array))
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Insert before an array/inline-table closing delimiter, keeping every old
/// byte, including trailing comments. Add a missing comma after the last value.
fn insert_element(text: &mut String, end: usize, last: Option<usize>, comma: bool, element: &str) {
    let close = end - 1;
    let line = text[..close].rfind('\n').map_or(0, |n| n + 1);
    let indent = &text[line..close];
    let (at, inserted) = if indent.chars().all(|c| c == ' ' || c == '\t') {
        (line, format!("{indent}  {element},{}", newline(text)))
    } else {
        (close, format!(" {element}"))
    };
    text.insert_str(at, &inserted);
    if !comma {
        if let Some(last) = last {
            text.insert(last, ',');
        }
    }
}

fn append(text: &mut String, key: &str, path: &str) -> Result<()> {
    append_to(text, "sources", key, path)
}

fn append_to(text: &mut String, table: &str, key: &str, path: &str) -> Result<()> {
    let document = Document::parse(text.as_str())?;
    let array = document
        .get(table)
        .and_then(|s| s.get(key))
        .and_then(Item::as_array)
        .context("expected a path array")?;
    let span = array.span().context("path array has no span")?;
    let last = array.iter().last().and_then(Value::span).map(|s| s.end);
    let comma = array.trailing_comma();
    let quoted = Value::from(path).to_string();
    insert_element(text, span.end, last, comma, &quoted);
    Ok(())
}

fn rename_verilog(text: &mut String) -> Result<()> {
    let document = Document::parse(text.as_str())?;
    let span = document
        .get("sources")
        .and_then(Item::as_table_like)
        .and_then(|s| s.get_key_value("verilog"))
        .and_then(|(k, _)| k.span())
        .context("sources.verilog has no key span")?;
    text.replace_range(span, "hdl");
    Ok(())
}

/// Add one key in its existing table style without serializing other items.
fn set_source(text: &mut String, key: &str, value: &Value) -> Result<()> {
    set_table_value(text, "sources", key, value)
}

fn set_table_value(text: &mut String, table_name: &str, key: &str, value: &Value) -> Result<()> {
    let document = Document::parse(text.as_str())?;
    let entry = format!("{key} = {value}");
    let nl = newline(text);
    match document.get(table_name) {
        Some(Item::Value(Value::InlineTable(table))) => {
            let span = table.span().context("table has no span")?;
            let last = table
                .iter()
                .filter_map(|(_, v)| v.span())
                .map(|s| s.end)
                .max();
            let comma = table.trailing_comma();
            insert_element(text, span.end, last, comma, &entry);
        }
        Some(Item::Table(table)) => {
            let end = table
                .iter()
                .filter_map(|(_, v)| v.span())
                .map(|s| s.end)
                .max()
                .or_else(|| table.span().map(|s| s.start))
                .context("table has no span")?;
            let at = text[end..].find('\n').map_or(text.len(), |n| end + n + 1);
            let prefix = if table.is_dotted() {
                format!("{table_name}.")
            } else {
                String::new()
            };
            let separator = if at > 0 && !text[..at].ends_with('\n') {
                nl
            } else {
                ""
            };
            text.insert_str(at, &format!("{separator}{prefix}{entry}{nl}"));
        }
        None => {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push_str(nl);
            }
            text.push_str(&format!("[{table_name}]{nl}{entry}{nl}"));
        }
        _ => bail!("expected {table_name} to be a table"),
    }
    Ok(())
}

fn template(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Verilog | Kind::SystemVerilog => format!(
            "// New module; instantiate it from the design when needed.\nmodule {name};\nendmodule\n"
        ),
        Kind::Vhdl => format!(
            "-- New entity; instantiate it from the design when needed.\nlibrary ieee;\nuse ieee.std_logic_1164.all;\n\nentity {name} is\nend entity {name};\n\narchitecture rtl of {name} is\nbegin\nend architecture rtl;\n"
        ),
        Kind::Constraints => {
            let mut text = String::from("# New constraints; see the LPF syntax reference below.\n");
            for line in include_str!("../../assets/ulx3s/board.lpf").lines().take(12) {
                text.push_str(line);
                text.push('\n');
            }
            text
        }
        // Yosys $readmemh accepts // line comments in hexadecimal data.
        Kind::Hex => "// Hex data; add hexadecimal words below.\n".into(),
    }
}

// Reserved words across Verilog and SystemVerilog (including assertion,
// configuration and coverage syntax), even for a new .v file.
const VERILOG_KEYWORDS: &str = "
accept_on alias always always_comb always_ff always_latch and assert assign assume automatic
before begin bind bins binsof bit break buf bufif0 bufif1 byte
case casex casez cell chandle checker class clocking cmos config const constraint context
continue cover covergroup coverpoint cross deassign default defparam design disable dist do
edge else end endcase endchecker endclass endclocking endconfig endfunction endgenerate
endgroup endinterface endmodule endpackage endprimitive endprogram endproperty endspecify
endsequence endtable endtask enum event eventually expect export extends extern
final first_match for force foreach forever fork forkjoin function generate genvar global
highz0 highz1 if iff ifnone ignore_bins illegal_bins implements implies import incdir include
initial inout input inside instance int integer interconnect interface intersect
join join_any join_none large let liblist library local localparam logic longint
macromodule matches medium modport module nand negedge nettype new nexttime nmos nor
noshowcancelled not notif0 notif1 null or output package packed parameter pmos posedge
primitive priority program property protected pull0 pull1 pulldown pullup pulsestyle_ondetect
pulsestyle_onevent pure rand randc randcase randsequence rcmos real realtime ref reg reject_on
release repeat restrict return rnmos rpmos rtran rtranif0 rtranif1 s_always s_eventually
s_nexttime s_until s_until_with scalared sequence shortint shortreal showcancelled signed
small soft solve specify specparam static string strong strong0 strong1 struct super supply0
supply1 sync_accept_on sync_reject_on table tagged task this throughout time timeprecision
timeunit tran tranif0 tranif1 tri tri0 tri1 triand trior trireg type typedef union unique
unique0 unsigned until until_with untyped use uwire var vectored virtual void wait wait_order
wand weak weak0 weak1 while wildcard wire with within wor xnor xor
";
// Includes VHDL-2019's private/view; see GHDL's vhdl-tokens.ads.
const VHDL_KEYWORDS: &str = "
abs access after alias all and architecture array assert assume assume_guarantee attribute
begin block body buffer bus case component configuration constant context cover default
disconnect downto else elsif end entity exit fairness file for force function generate generic
group guarded if impure in inertial inout is label library linkage literal loop map mod nand
new next nor not null of on open or others out package parameter port postponed private
procedure process property protected pure range record register reject release rem report
restrict restrict_guarantee return rol ror select sequence severity shared signal sla sll sra
srl strong subtype then to transport type unaffected units until use variable view vmode
vprop vunit wait when while with xnor xor
";

fn identifier(name: &str, kind: Kind) -> Result<()> {
    let mut chars = name.chars();
    let first = chars.next();
    let valid = if kind == Kind::Vhdl {
        first.is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.ends_with('_')
            && !name.contains("__")
    } else {
        first.is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    };
    ensure!(valid, "invalid HDL identifier {name:?}");
    let keywords = if kind == Kind::Vhdl {
        VHDL_KEYWORDS
    } else {
        VERILOG_KEYWORDS
    };
    ensure!(
        !keywords
            .split_ascii_whitespace()
            .any(|word| if kind == Kind::Vhdl {
                word.eq_ignore_ascii_case(name)
            } else {
                word == name
            }),
        "HDL identifier {name:?} is a reserved keyword"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;
    use std::collections::BTreeMap;

    fn create(root: &Path, path: &str) -> Result<NewFileOutcome> {
        new_file(root, Path::new(path), &NewFile::default())
    }

    fn manifest(root: &Path) -> String {
        fs::read_to_string(root.join("ecp5.toml")).unwrap()
    }

    fn write_manifest(root: &Path, text: &str) {
        fs::write(root.join("ecp5.toml"), text).unwrap();
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    // Include empty directories and link targets, so failed operations cannot
    // quietly leave parent folders or change the manifest/file tree.
    fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, folder: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                let info = fs::symlink_metadata(&path).unwrap();
                let bytes = if info.is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec()
                } else if info.is_dir() {
                    visit(root, &path, out);
                    Vec::new()
                } else {
                    fs::read(&path).unwrap()
                };
                out.insert(path.strip_prefix(root).unwrap().to_owned(), bytes);
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }

    fn refused(root: &Path, path: &str, expected: &str) {
        let before = tree(root);
        let error = format!("{:#}", create(root, path).unwrap_err());
        assert!(error.contains(expected), "{path}: {error}");
        assert_eq!(tree(root), before, "{path}");
    }

    #[test]
    fn kinds_in_default_explicit_verilog_and_hdl_projects() {
        for initial in [None, Some("verilog"), Some("hdl")] {
            for extension in ["v", "sv", "vhd", "vhdl", "lpf", "hex"] {
                let f = Folder::new();
                if let Some(key) = initial {
                    write_manifest(
                        &f.0,
                        &format!("# project\n[sources]\n{key} = ['top.v'] # roots\n"),
                    );
                }
                assert!(f.0.to_str().unwrap().contains(' '));
                if matches!(extension, "vhd" | "vhdl") {
                    refused(
                        &f.0,
                        &format!("new_unit.{extension}"),
                        "a VHDL file cannot join",
                    );
                    continue;
                }
                let outcome = create(&f.0, &format!("new_unit.{extension}")).unwrap();
                assert_eq!(
                    outcome.path,
                    fs::canonicalize(&f.0)
                        .unwrap()
                        .join(format!("new_unit.{extension}"))
                );
                let p = Project::open(&f.0).unwrap();
                match extension {
                    "v" | "sv" => {
                        assert_eq!(outcome.name.as_deref(), Some("new_unit"));
                        assert!(p
                            .verilog
                            .contains(&PathBuf::from(format!("new_unit.{extension}"))));
                        assert_eq!(
                            fs::read_to_string(f.0.join("top.v")).unwrap(),
                            "module top; endmodule\n"
                        );
                        assert_eq!(fs::read_to_string(&outcome.path).unwrap(),
                            "// New module; instantiate it from the design when needed.\nmodule new_unit;\nendmodule\n");
                        if extension == "sv" || initial == Some("hdl") {
                            assert!(p.hdl.is_some());
                        }
                    }
                    "lpf" => {
                        assert!(outcome.name.is_none());
                        assert_eq!(p.constraints, Path::new("top.lpf"));
                        assert_eq!(outcome.notes, ["sources.constraints made explicit"]);
                        let contents = fs::read_to_string(&outcome.path).unwrap();
                        assert!(contents.lines().all(|line| line.starts_with('#')));
                        assert_eq!(
                            contents.lines().skip(1).collect::<Vec<_>>(),
                            include_str!("../../assets/ulx3s/board.lpf")
                                .lines()
                                .take(12)
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(
                            contents.lines().skip(1).collect::<Vec<_>>(),
                            include_str!("../../examples/ulx3s/blinky_cycle/top.lpf")
                                .lines()
                                .take(12)
                                .collect::<Vec<_>>()
                        );
                    }
                    "hex" => {
                        assert!(outcome.name.is_none());
                        assert!(outcome.notes.is_empty());
                        assert_eq!(p.files, paths(&["new_unit.hex"]));
                        assert!(p.memory_images.is_empty());
                        assert_eq!(
                            fs::read_to_string(&outcome.path).unwrap(),
                            "// Hex data; add hexadecimal words below.\n"
                        );
                    }
                    _ => unreachable!(),
                }
                if initial.is_none() && matches!(extension, "v" | "hex") {
                    assert!(!f.0.join("ecp5.toml").exists());
                    assert!(outcome.notes.is_empty());
                }
            }
        }
    }

    #[test]
    fn subfolder_verilog_materializes_sorted_roots_and_appends_to_explicit_lists() {
        for key in [None, Some("verilog"), Some("hdl")] {
            let f = Folder::new();
            fs::write(f.0.join("alpha.v"), "module alpha; endmodule\n").unwrap();
            if let Some(key) = key {
                write_manifest(&f.0, &format!("[sources]\n{key} = ['top.v', 'alpha.v']\n"));
            }
            let outcome = create(&f.0, "rtl with spaces/nested/counter.v").unwrap();
            let p = Project::open(&f.0).unwrap();
            let expected = if key.is_none() {
                paths(&["alpha.v", "top.v", "rtl with spaces/nested/counter.v"])
            } else {
                paths(&["top.v", "alpha.v", "rtl with spaces/nested/counter.v"])
            };
            assert_eq!(p.verilog, expected);
            assert_eq!(
                outcome.notes,
                [match key {
                    None => "sources.verilog made explicit",
                    Some("verilog") => "sources.verilog appended",
                    _ => "sources.hdl appended",
                }]
            );
        }
    }

    #[test]
    fn sv_promotes_legacy_roots_preserving_comments_bytes_and_order() {
        for original in [
            "# heading\n[project] # identity\nname = 'A project'\n\n[sources] # inputs\n# roots\nverilog  = [\n  'z.v', # first\n  'top.v' # second\n] # array\nconstraints = 'top.lpf' # active\n\n[build]\nseed = 17 # stable\n",
            "# heading\r\n[sources]\r\nverilog=['z.v', 'top.v',] # roots\r\n[build]\r\nseed=17 # stable\r\n",
            "sources = { verilog = ['z.v','top.v'], constraints='top.lpf' } # inline\n[build]\nseed=17\n",
            "# dotted\nsources.verilog = ['z.v','top.v'] # roots\nsources.constraints='top.lpf'\n[build]\nseed=17\n",
            "[sources]\n'verilog' = ['z.v','top.v'] # quoted key\n",
        ] {
            let f = Folder::new();
            fs::write(f.0.join("z.v"), "module z; endmodule\n").unwrap();
            write_manifest(&f.0, original);
            let outcome = create(&f.0, "rtl/extra.sv").unwrap();
            assert_eq!(outcome.notes, ["sources.hdl created from sources.verilog"]);
            let p = Project::open(&f.0).unwrap();
            assert_eq!(p.hdl, Some(paths(&["z.v", "top.v", "rtl/extra.sv"])));
            let updated = manifest(&f.0);
            let doc = Document::parse(updated.as_str()).unwrap();
            let sources = doc["sources"].as_table_like().unwrap();
            assert!(sources.get("verilog").is_none());
            let key = sources.get_key_value("hdl").unwrap().0.span().unwrap();
            let array = sources.get("hdl").unwrap().as_array().unwrap();
            let last = array.iter().last().unwrap().span().unwrap();
            let mut restored = updated.clone();
            // Remove just the inserted bytes and undo the key rename; all
            // other bytes, including comments within the array, must survive.
            let close = array.span().unwrap().end - 1;
            if original.contains("'top.v' # second") {
                let line = restored[..last.start].rfind('\n').unwrap() + 1;
                restored.replace_range(line..close, "");
                let comma = restored.find("'top.v', # second").unwrap() + "'top.v'".len();
                restored.remove(comma);
            } else {
                let start = restored[..last.start].rfind(' ').unwrap();
                restored.replace_range(start..close, "");
                if !original.contains("'top.v',]") {
                    let comma = restored.find("'top.v',").unwrap() + "'top.v'".len();
                    restored.remove(comma);
                }
            }
            restored.replace_range(key, if original.contains("'verilog'") { "'verilog'" } else { "verilog" });
            assert_eq!(restored, original);
        }
        let f = Folder::new();
        fs::write(f.0.join("a.v"), "").unwrap();
        let outcome = create(&f.0, "extra.sv").unwrap();
        assert_eq!(
            outcome.notes,
            ["sources.hdl created from default Verilog discovery"]
        );
        assert_eq!(
            Project::open(&f.0).unwrap().hdl,
            Some(paths(&["a.v", "top.v", "extra.sv"]))
        );
    }

    #[test]
    fn vhdl_bootstraps_empty_projects_and_appends_to_vhdl_roots() {
        for initial in [
            "",
            "[sources]\nverilog = [] # deliberately empty\n",
            "[sources]\nhdl = []\n",
        ] {
            for extension in ["vhd", "vhdl"] {
                let f = Folder::new();
                fs::remove_file(f.0.join("top.v")).unwrap();
                if !initial.is_empty() {
                    write_manifest(&f.0, initial);
                }
                let outcome = create(&f.0, &format!("rtl/first_entity.{extension}")).unwrap();
                assert_eq!(outcome.name.as_deref(), Some("first_entity"));
                let text = fs::read_to_string(&outcome.path).unwrap();
                assert!(text.starts_with("-- "));
                assert!(text.contains("library ieee;\nuse ieee.std_logic_1164.all;"));
                assert!(text.contains("entity first_entity is\nend entity first_entity;"));
                assert!(text
                    .contains("architecture rtl of first_entity is\nbegin\nend architecture rtl;"));
                let second = create(&f.0, "second_entity.vhd").unwrap();
                assert_eq!(second.notes, ["sources.hdl appended"]);
                assert_eq!(
                    Project::open(&f.0).unwrap().hdl,
                    Some(paths(&[
                        &format!("rtl/first_entity.{extension}"),
                        "second_entity.vhd"
                    ]))
                );
                refused(&f.0, "mixed.v", "cannot join VHDL");
                refused(&f.0, "mixed.sv", "cannot join VHDL");
            }
        }
        for path in ["first.v", "first.sv"] {
            let f = Folder::new();
            fs::remove_file(f.0.join("top.v")).unwrap();
            create(&f.0, path).unwrap();
            assert_eq!(Project::open(&f.0).unwrap().verilog, paths(&[path]));
        }
    }

    #[test]
    fn constraints_preserve_active_selection_and_only_pin_default_root_discovery() {
        for initial in [
            None,
            Some("[sources]\nconstraints = 'top.lpf' # selected\n"),
        ] {
            for name in ["alternate.lpf", "constraints/alternate.lpf"] {
                let f = Folder::new();
                if let Some(initial) = initial {
                    write_manifest(&f.0, initial);
                }
                let outcome = create(&f.0, name).unwrap();
                let p = Project::open(&f.0).unwrap();
                assert_eq!(p.constraints, Path::new("top.lpf"));
                if let Some(initial) = initial {
                    assert_eq!(manifest(&f.0), initial);
                    assert!(outcome.notes.is_empty());
                } else if name.contains('/') {
                    assert!(!f.0.join("ecp5.toml").exists());
                    assert!(outcome.notes.is_empty());
                } else {
                    assert_eq!(manifest(&f.0), "[sources]\nconstraints = \"top.lpf\"\n");
                }
            }
        }
    }

    #[test]
    fn hex_membership_keeps_auxiliary_order_and_excludes_memory_images() {
        for explicit in [false, true] {
            for new in ["new_data.hex", "data with spaces/new_data.hex"] {
                let f = Folder::new();
                for name in ["z.hex", "a.mem", "firmware.hex"] {
                    fs::write(f.0.join(name), "00\n").unwrap();
                }
                fs::write(f.0.join("ecpbram.txt"), "# runtime\nfirmware.hex\n").unwrap();
                if explicit {
                    write_manifest(&f.0, "[sources]\nfiles = ['z.hex', 'a.mem'] # aux\n");
                }
                let outcome = create(&f.0, new).unwrap();
                let p = Project::open(&f.0).unwrap();
                assert_eq!(p.memory_images, paths(&["firmware.hex"]));
                let expected = if explicit {
                    paths(&["z.hex", "a.mem", new])
                } else if new.contains('/') {
                    paths(&["a.mem", "z.hex", new])
                } else {
                    paths(&["a.mem", new, "z.hex"])
                };
                assert_eq!(p.files, expected);
                assert_eq!(
                    fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
                    "# runtime\nfirmware.hex\n"
                );
                assert_eq!(
                    outcome.notes,
                    if explicit {
                        vec!["sources.files appended"]
                    } else if new.contains('/') {
                        vec!["sources.files made explicit"]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }

    #[test]
    fn refuses_unsafe_paths_names_keywords_and_unknown_extensions_without_changes() {
        let f = Folder::new();
        for (path, expected) in [
            ("top.v", "already exists"),
            ("/absolute.v", "relative"),
            ("../outside.v", "'..'"),
            ("rtl/../outside.v", "'..'"),
            ("build/unit.v", "build output"),
            ("build/deep/new.hex", "build output"),
            ("9bad.v", "invalid HDL identifier"),
            ("bad-name.v", "invalid HDL identifier"),
            ("bad name.sv", "invalid HDL identifier"),
            ("$bad.v", "invalid HDL identifier"),
            ("module.v", "keyword"),
            ("endcase.v", "keyword"),
            ("instance.sv", "keyword"),
            ("always_ff.v", "keyword"),
            ("checker.sv", "keyword"),
            ("data.txt", "expected .v, .sv, .vhd, .vhdl, .lpf or .hex"),
            ("", "relative"),
            (".", "filename"),
        ] {
            refused(&f.0, path, expected);
        }
        fs::create_dir(f.0.join("occupied.v")).unwrap();
        refused(&f.0, "occupied.v", "already exists");
        write_manifest(&f.0, "[build]\ndir = 'route output/nested'\n");
        refused(&f.0, "route output/nested/unit.v", "build output");
    }

    #[test]
    fn validates_language_identifiers_and_accepts_an_explicit_name() {
        let f = Folder::new();
        for name in [
            "",
            "123",
            "bad-name",
            "has space",
            "module",
            "interface",
            "randsequence",
        ] {
            let before = tree(&f.0);
            assert!(new_file(
                &f.0,
                Path::new("valid.v"),
                &NewFile {
                    name: Some(name.into())
                }
            )
            .is_err());
            assert_eq!(tree(&f.0), before);
        }
        let outcome = new_file(
            &f.0,
            Path::new("bad-name.v"),
            &NewFile {
                name: Some("_valid$1".into()),
            },
        )
        .unwrap();
        assert_eq!(outcome.name.as_deref(), Some("_valid$1"));
        assert!(fs::read_to_string(outcome.path)
            .unwrap()
            .contains("module _valid$1;"));
        assert!(new_file(
            &f.0,
            Path::new("data.hex"),
            &NewFile {
                name: Some("data".into())
            }
        )
        .is_err());
        fs::remove_file(f.0.join("top.v")).unwrap();
        fs::remove_file(f.0.join("bad-name.v")).unwrap();
        for path in [
            "_bad.vhd",
            "bad_.vhd",
            "two__words.vhd",
            "bad$dollar.vhd",
            "Entity.vhd",
            "ARCHITECTURE.vhdl",
            "private.vhdl",
            "view.vhd",
            "9bad.vhd",
        ] {
            refused(
                &f.0,
                path,
                if ["Entity", "ARCHITECTURE", "private", "view"]
                    .iter()
                    .any(|prefix| path.starts_with(prefix))
                {
                    "keyword"
                } else {
                    "invalid HDL identifier"
                },
            );
        }
        create(&f.0, "Good_entity2.vhdl").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_or_alias_build_output_or_overwrite_a_broken_link() {
        use std::os::unix::fs::symlink;
        let f = Folder::new();
        let outside = Folder::new();
        symlink(&outside.0, f.0.join("external")).unwrap();
        let outside_before = tree(&outside.0);
        refused(&f.0, "external/unit.v", "symlinked parent leaves");
        assert_eq!(tree(&outside.0), outside_before);
        fs::create_dir(f.0.join("outputs")).unwrap();
        symlink("outputs", f.0.join("route")).unwrap();
        symlink("outputs", f.0.join("alias")).unwrap();
        write_manifest(&f.0, "[build]\ndir = 'route'\n");
        refused(&f.0, "outputs/unit.v", "build output");
        refused(&f.0, "alias/unit.v", "build output");
        symlink("missing", f.0.join("taken.v")).unwrap();
        refused(&f.0, "taken.v", "already exists");
        fs::create_dir(f.0.join("rtl")).unwrap();
        symlink("rtl", f.0.join("local")).unwrap();
        create(&f.0, "local/allowed.v").unwrap();
        assert!(f.0.join("rtl/allowed.v").is_file());
        symlink(".", f.0.join("root_alias")).unwrap();
        // Default constraints also stay selected when adding through the alias.
        let outcome = create(&f.0, "root_alias/alternative.lpf").unwrap();
        assert_eq!(outcome.notes, ["sources.constraints made explicit"]);
        assert_eq!(
            Project::open(&f.0).unwrap().constraints,
            Path::new("top.lpf")
        );
    }

    #[cfg(unix)]
    #[test]
    fn manifest_symlink_to_an_external_file_is_never_mutated() {
        let f = Folder::new();
        let outside = Folder::new();
        write_manifest(&outside.0, "# keep\n[sources]\nverilog = ['top.v']\n");
        std::os::unix::fs::symlink(outside.0.join("ecp5.toml"), f.0.join("ecp5.toml")).unwrap();
        let before = tree(&outside.0);
        refused(&f.0, "new_unit.sv", "manifest symlink outside");
        assert_eq!(tree(&outside.0), before);
    }

    #[test]
    fn rollback_restores_manifest_bytes_and_removes_created_parents_and_manifest() {
        // A first root can be staged despite the loader's initial no-roots
        // error. Final loader validation then fails on the invalid top name.
        for initial in [None, Some("# preserve\r\n[project]\r\ntop = 'bad-name' # invalid\r\n[sources]\r\nverilog = [] # empty\r\n")] {
            let f = Folder::new();
            fs::remove_file(f.0.join("top.v")).unwrap();
            if let Some(initial) = initial {
                write_manifest(&f.0, initial);
            } else {
                // With no manifest, fail final validation on missing LPF.
                fs::remove_file(f.0.join("top.lpf")).unwrap();
            }
            refused(&f.0, "new parents/nested/unit.sv", "validating new project file");
            if let Some(initial) = initial { assert_eq!(manifest(&f.0), initial); }
            else { assert!(!f.0.join("ecp5.toml").exists()); }
            assert!(!f.0.join("new parents").exists());
        }
    }

    #[test]
    fn adding_keys_preserves_every_unrelated_byte_in_all_table_styles() {
        for original in [
            "# before\r\n[project] # id\r\nname='unchanged'\r\n[sources] # inputs\r\nverilog = ['top.v'] # root\r\n# tail\r\n[build]\r\nseed=21 # stable\r\n",
            "# before\n[sources] # empty table\n# tail\n[build]\nseed=21 # stable\n",
            "sources = { verilog=['top.v'] } # inline\n[build]\nseed=21\n",
            "sources.verilog=['top.v'] # dotted\n[build]\nseed=21\n",
            "# no sources\n[build]\nseed = 21 # stable", "# only a comment",
        ] {
            let f = Folder::new();
            write_manifest(&f.0, original);
            let outcome = create(&f.0, "alt.lpf").unwrap();
            assert_eq!(outcome.notes, ["sources.constraints made explicit"]);
            assert_eq!(Project::open(&f.0).unwrap().constraints, Path::new("top.lpf"));
            let updated = manifest(&f.0);
            let restored = if original.contains("sources = {") {
                updated.replacen(",  constraints = \"top.lpf\"", " ", 1)
            } else if original.contains("sources.verilog") {
                updated.replacen("sources.constraints = \"top.lpf\"\n", "", 1)
            } else if original.contains("[sources]") {
                updated.replacen(&format!("constraints = \"top.lpf\"{}", newline(original)), "", 1)
            } else {
                updated[..original.len()].to_owned()
            };
            assert_eq!(restored, original);
        }
    }

    #[test]
    fn appends_to_empty_arrays_and_arrays_with_closing_indent_and_comments() {
        for list in [
            "[]",
            "[ ]",
            "[\n # empty\n  ]",
            "[\n 'top.v', # root\n    ]",
            "['top.v' # root\n]",
            "[\r\n 'top.v', # root\r\n]\r\n",
        ] {
            let f = Folder::new();
            write_manifest(&f.0, &format!("[sources]\nfiles = {list} # list\n"));
            create(&f.0, "data.hex").unwrap();
            let p = Project::open(&f.0).unwrap();
            assert_eq!(p.files.last().unwrap(), Path::new("data.hex"));
            let updated = manifest(&f.0);
            if list.contains("# root") {
                assert!(updated.contains("# root"));
            }
            if list.contains("# empty") {
                assert!(updated.contains("# empty"));
            }
        }
    }
    #[test]
    fn memory_image_adds_explicit_images_and_removes_all_explicit_source_references() {
        let f = Folder::new();
        for file in ["old.hex", "fw.hex", "keep.mem"] {
            fs::write(f.0.join(file), "00\n").unwrap();
        }
        let original = "# heading\r\n[sources] # inputs\r\nfiles = [\r\n 'fw.hex', # firmware, comment\r\n 'keep.mem', # keep\r\n 'fw.hex' # repeated\r\n] # sources\r\n[memory] # images\r\nimages = ['old.hex'] # order\r\n[build]\r\nseed = 21 # unchanged\r\n";
        write_manifest(&f.0, original);
        let before = fs::read(f.0.join("fw.hex")).unwrap();
        let outcome = set_memory_image(&f.0, Path::new("fw.hex"), true).unwrap();
        let p = Project::open(&f.0).unwrap();
        assert_eq!(p.memory_images, paths(&["old.hex", "fw.hex"]));
        assert_eq!(p.files, paths(&["keep.mem"]));
        assert_eq!(fs::read(f.0.join("fw.hex")).unwrap(), before);
        let updated = manifest(&f.0);
        assert!(updated.starts_with("# heading\r\n[sources] # inputs\r\n"));
        assert!(updated.contains("# firmware, comment\r\n 'keep.mem'"));
        assert!(updated.contains("# repeated\r\n] # sources\r\n[memory] # images\r\n"));
        assert!(updated.ends_with("# order\r\n[build]\r\nseed = 21 # unchanged\r\n"));
        assert_eq!(
            outcome.notes,
            [
                "memory.images appended fw.hex",
                "sources.files removed fw.hex",
                "ecpbram.txt updated"
            ]
        );
        assert_eq!(
            fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
            "old.hex\nfw.hex\n"
        );
        assert!(set_memory_image(&f.0, Path::new("fw.hex"), true)
            .unwrap()
            .notes
            .is_empty());
    }

    #[test]
    fn memory_image_materializes_legacy_images_and_preserves_the_legacy_comments() {
        let f = Folder::new();
        for file in ["old.hex", "fw.mem"] {
            fs::write(f.0.join(file), "00\n").unwrap();
        }
        let legacy = "# firmware\r\n\r\n  old.hex  \r\n# keep this footer";
        fs::write(f.0.join("ecpbram.txt"), legacy).unwrap();
        let outcome = set_memory_image(&f.0, Path::new("fw.mem"), true).unwrap();
        assert_eq!(
            Project::open(&f.0).unwrap().memory_images,
            paths(&["old.hex", "fw.mem"])
        );
        assert!(Project::open(&f.0).unwrap().files.is_empty());
        assert_eq!(
            fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
            format!("{legacy}\r\nfw.mem\r\n")
        );
        assert!(outcome
            .notes
            .iter()
            .any(|n| n == "memory.images made explicit from ecpbram.txt"));
        assert!(manifest(&f.0).contains("images = [\"old.hex\", \"fw.mem\"]"));
        set_memory_image(&f.0, Path::new("old.hex"), false).unwrap();
        assert_eq!(Project::open(&f.0).unwrap().files, paths(&["old.hex"]));
        assert_eq!(
            fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
            "# firmware\r\n\r\n# keep this footer\r\nfw.mem\r\n"
        );
    }

    #[test]
    fn memory_image_removal_restores_only_explicit_source_lists_and_default_root_discovery() {
        for explicit in [false, true] {
            for file in ["fw.hex", "data/fw.init"] {
                let f = Folder::new();
                fs::create_dir_all(f.0.join("data")).unwrap();
                fs::write(f.0.join(file), "00\n").unwrap();
                let initial = format!(
                    "{}[memory]\nimages = ['{file}'] # selected\n",
                    if explicit {
                        "[sources]\nfiles = [] # explicit\n"
                    } else {
                        ""
                    }
                );
                write_manifest(&f.0, &initial);
                fs::write(
                    f.0.join("ecpbram.txt"),
                    format!("# firmware\n{file}\n# end\n"),
                )
                .unwrap();
                let outcome = set_memory_image(&f.0, Path::new(file), false).unwrap();
                let p = Project::open(&f.0).unwrap();
                assert!(p.memory_images.is_empty());
                assert_eq!(
                    p.files,
                    if explicit || !file.contains('/') {
                        paths(&[file])
                    } else {
                        vec![]
                    }
                );
                assert_eq!(
                    fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
                    "# firmware\n# end\n"
                );
                assert!(manifest(&f.0).contains("# selected"));
                assert!(outcome
                    .notes
                    .iter()
                    .any(|n| n == &format!("memory.images removed {file}")));
                if explicit {
                    assert!(outcome
                        .notes
                        .iter()
                        .any(|n| n == &format!("sources.files appended {file}")));
                } else {
                    assert!(!manifest(&f.0).contains("[sources]"));
                }
            }
        }
    }

    #[test]
    fn memory_image_removal_materializes_legacy_list_and_explicit_images_override_stale_legacy() {
        let f = Folder::new();
        for file in ["fw.hex", "other.hex", "old.hex"] {
            fs::write(f.0.join(file), "00\n").unwrap();
        }
        fs::write(
            f.0.join("ecpbram.txt"),
            "# old\nfw.hex\n# other\nother.hex\n",
        )
        .unwrap();
        let outcome = set_memory_image(&f.0, Path::new("fw.hex"), false).unwrap();
        assert_eq!(
            Project::open(&f.0).unwrap().memory_images,
            paths(&["other.hex"])
        );
        assert!(outcome
            .notes
            .iter()
            .any(|n| n == "memory.images made explicit from ecpbram.txt"));
        assert_eq!(
            fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
            "# old\n# other\nother.hex\n"
        );
        write_manifest(&f.0, "[memory]\nimages = ['old.hex', 'other.hex']\n");
        set_memory_image(&f.0, Path::new("fw.hex"), true).unwrap();
        assert_eq!(
            fs::read_to_string(f.0.join("ecpbram.txt")).unwrap(),
            "# old\n# other\nold.hex\nother.hex\nfw.hex\n"
        );
    }

    #[test]
    fn memory_image_edits_preserve_unrelated_bytes_in_all_table_styles() {
        for original in [
            "# heading\n[sources]\nfiles=['fw.hex', 'other.mem'] # inputs\n[memory] # runtime\nimages=[] # tail\n[build]\nseed=21\n",
            "sources = { files=['fw.hex', 'other.mem'] } # inputs\nmemory = { images=[] } # runtime\n[build]\nseed=21\n",
            "sources.files=['fw.hex', 'other.mem'] # inputs\nmemory.images=[] # runtime\n[build]\nseed=21\n",
            "[sources]\nfiles=['fw.hex', 'other.mem']\n[memory]\n# empty\n[build]\nseed=21\n",
            "memory = {} # inline empty\nsources.files=['fw.hex', 'other.mem']\n[build]\nseed=21\n",
            "sources.files=['fw.hex', 'other.mem']\nmemory.other = 'invalid'\n",
        ] {
            let f = Folder::new();
            for file in ["fw.hex", "other.mem"] { fs::write(f.0.join(file), "00\n").unwrap(); }
            write_manifest(&f.0, original);
            if original.contains("invalid") {
                assert!(set_memory_image(&f.0, Path::new("fw.hex"), true).is_err());
                assert_eq!(manifest(&f.0), original);
                continue;
            }
            set_memory_image(&f.0, Path::new("fw.hex"), true).unwrap();
            assert_eq!(Project::open(&f.0).unwrap().memory_images, paths(&["fw.hex"]));
            assert_eq!(Project::open(&f.0).unwrap().files, paths(&["other.mem"]));
            assert!(manifest(&f.0).ends_with("[build]\nseed=21\n"));
            if original.contains("# runtime") { assert!(manifest(&f.0).contains("# runtime")); }
            set_memory_image(&f.0, Path::new("fw.hex"), false).unwrap();
            assert_eq!(Project::open(&f.0).unwrap().files, paths(&["other.mem", "fw.hex"]));
        }
    }

    #[test]
    fn memory_image_refuses_non_memory_missing_unsafe_and_build_paths_without_changes() {
        let f = Folder::new();
        fs::create_dir(f.0.join("build")).unwrap();
        fs::write(f.0.join("build/fw.hex"), "00\n").unwrap();
        for path in [
            "top.v",
            "top.lpf",
            "missing.hex",
            "../fw.hex",
            "/absolute.hex",
            "build/fw.hex",
        ] {
            let before = tree(&f.0);
            assert!(
                set_memory_image(&f.0, Path::new(path), true).is_err(),
                "{path}"
            );
            assert_eq!(tree(&f.0), before);
        }
    }

    #[test]
    fn memory_image_rolls_back_both_files_on_final_loader_failure() {
        for legacy in [None, Some("# preserve\r\n# footer\r\n")] {
            let f = Folder::new();
            fs::write(f.0.join("fw.hex"), "00\n").unwrap();
            // Legacy verilog permits an arbitrary existing root; assigning it
            // as an image must fail final validation rather than corrupt inputs.
            write_manifest(
                &f.0,
                "# preserve\r\n[sources]\r\nverilog = ['top.v', 'fw.hex'] # root\r\n",
            );
            if let Some(text) = legacy {
                fs::write(f.0.join("ecpbram.txt"), text).unwrap();
            }
            Project::open(&f.0).unwrap();
            let before = tree(&f.0);
            let error = set_memory_image(&f.0, Path::new("fw.hex"), true).unwrap_err();
            assert!(
                format!("{error:#}").contains("validating memory image membership"),
                "{error:#}"
            );
            assert_eq!(tree(&f.0), before);
        }
    }

    #[cfg(unix)]
    #[test]
    fn memory_image_refuses_external_symlinks_without_mutating_either_project() {
        use std::os::unix::fs::symlink;
        for target in ["fw.hex", "ecp5.toml", "ecpbram.txt"] {
            let f = Folder::new();
            let outside = Folder::new();
            fs::write(f.0.join("fw.hex"), "00\n").unwrap();
            fs::write(
                outside.0.join(target),
                if target == "ecp5.toml" {
                    "# external\n"
                } else if target == "fw.hex" {
                    "00\n"
                } else {
                    "# external\n"
                },
            )
            .unwrap();
            let local = f.0.join(target);
            if local.exists() {
                fs::remove_file(&local).unwrap();
            }
            symlink(outside.0.join(target), local).unwrap();
            let before = tree(&f.0);
            let external_before = tree(&outside.0);
            assert!(
                set_memory_image(&f.0, Path::new("fw.hex"), true).is_err(),
                "{target}"
            );
            assert_eq!(tree(&f.0), before);
            assert_eq!(tree(&outside.0), external_before);
        }
    }
}
