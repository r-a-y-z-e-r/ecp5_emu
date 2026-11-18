//! A project is a folder with HDL, an LPF, and optionally `ecp5.toml`.
//! All keys are optional; unknown keys are errors. Paths are relative to the
//! project folder, may name subfolders, and cannot be absolute or contain `..`.
//! Source paths exposed by [`Project`] retain that relative layout.
//!
//! ```toml
//! [project]
//! name = "blinky_cycle" # default: folder name
//! top = "top"           # default: top
//! [sources]
//! verilog = ["top.v"]   # default: sorted *.v in the folder
//! constraints = "top.lpf" # default: the single *.lpf
//! files = ["message.hex"] # default: *.hex *.mem *.vh *.svh *.init, minus images
//! # hdl = ["top.sv"]    # opt-in ordered roots, replaces verilog
//! # include_dirs = ["."] # only with hdl; listed headers go in files
//! [synthesis]
//! sv_frontend = "auto"  # auto | builtin | slang (explicit suite pin)
//! [memory]
//! images = ["fw0.hex"]  # default: non-comment lines of ecpbram.txt, else none
//! [device]
//! part = "LFE5U-85F"    # default: LFE5U-85F
//! package = "CABGA381"  # default: CABGA381
//! speed = 6             # omitted by default; passed to nextpnr only if given
//! [build]
//! dir = "build"         # default: build; cannot be the project folder
//! seed = 1              # default: 1
//! allow_timing_failure = false # default: false
//! ```
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

/// Language of a compilation root in the ordered HDL list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HdlLanguage {
    Verilog,
    SystemVerilog,
    Vhdl,
}
impl HdlLanguage {
    /// Classifies a root by suffix without changing the path's spelling.
    pub fn from_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "v" => Ok(Self::Verilog),
            "sv" => Ok(Self::SystemVerilog),
            "vhd" | "vhdl" => Ok(Self::Vhdl),
            _ => bail!(
                "unknown HDL root suffix: {} (expected .v, .sv, .vhd or .vhdl)",
                path.display()
            ),
        }
    }
}
/// Requested SystemVerilog frontend; auto uses Yosys's built-in reader.
/// Auto and builtin always use Yosys's built-in reader; slang is an explicit pin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SvFrontend {
    #[default]
    Auto,
    Builtin,
    Slang,
}
impl std::fmt::Display for SvFrontend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Builtin => "builtin",
            Self::Slang => "slang",
        })
    }
}

/// Target device understood by nextpnr-ecp5.
#[derive(Clone, Debug)]
pub struct Device {
    pub part: String,
    pub package: String,
    pub speed: Option<u32>,
}
impl Device {
    /// The nextpnr device selection flag.
    pub fn flag(&self) -> Result<&'static str> {
        Ok(match self.part.as_str() {
            "LFE5U-12F" => "--12k",
            "LFE5U-25F" => "--25k",
            "LFE5U-45F" => "--45k",
            "LFE5U-85F" => "--85k",
            "LFE5UM-25F" => "--um-25k",
            "LFE5UM-45F" => "--um-45k",
            "LFE5UM-85F" => "--um-85k",
            "LFE5UM5G-25F" => "--um5g-25k",
            "LFE5UM5G-45F" => "--um5g-45k",
            "LFE5UM5G-85F" => "--um5g-85k",
            _ => bail!("unsupported ECP5 part {}", self.part),
        })
    }
}
/// Validated project inputs. Source paths are relative to `root`.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub file: Option<PathBuf>,
    pub name: String,
    pub top: String,
    /// Selected roots, also exposed here for existing editor/diagnostic consumers.
    pub verilog: Vec<PathBuf>,
    /// Ordered opt-in HDL roots; absent for the unchanged legacy read path.
    pub hdl: Option<Vec<PathBuf>>,
    pub include_dirs: Vec<PathBuf>,
    pub sv_frontend: SvFrontend,
    pub constraints: PathBuf,
    pub files: Vec<PathBuf>,
    pub memory_images: Vec<PathBuf>,
    pub device: Device,
    pub build_dir: PathBuf,
    pub seed: u64,
    pub allow_timing_failure: bool,
}
/// Absolute output paths, with a placeholder configuration only for swapped RAM.
#[derive(Clone, Debug)]
pub struct Outputs {
    pub json: PathBuf,
    pub config: PathBuf,
    pub placeholder_config: Option<PathBuf>,
    pub routed_json: PathBuf,
    pub bit: PathBuf,
    pub yosys_log: PathBuf,
    pub nextpnr_log: PathBuf,
    pub report: PathBuf,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Manifest {
    project: Identity,
    sources: Sources,
    synthesis: Synthesis,
    memory: Memory,
    device: Target,
    build: Build,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Identity {
    name: Option<String>,
    top: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Sources {
    verilog: Option<Vec<PathBuf>>,
    hdl: Option<Vec<PathBuf>>,
    include_dirs: Option<Vec<PathBuf>>,
    constraints: Option<PathBuf>,
    files: Option<Vec<PathBuf>>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Synthesis {
    sv_frontend: SvFrontend,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Memory {
    images: Option<Vec<PathBuf>>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Target {
    part: Option<String>,
    package: Option<String>,
    speed: Option<u32>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Build {
    dir: Option<PathBuf>,
    seed: Option<u64>,
    allow_timing_failure: bool,
}

fn relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "project path must be relative and cannot contain '..': {}",
        path.display()
    );
    Ok(())
}
fn matching(root: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        // `Path::is_file` follows a symbolic link, so a source shared with
        // another project through a link is taken like any other file.
        if entry.path().is_file()
            && entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| extensions.contains(&s))
        {
            paths.push(PathBuf::from(entry.file_name()));
        }
    }
    paths.sort();
    Ok(paths)
}
impl Project {
    /// Opens a folder or its `ecp5.toml`, applying defaults and validating inputs.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        ensure!(path.exists(), "no project at {}", path.display());
        let (root, file) = if path.is_dir() {
            let root = fs::canonicalize(path)
                .with_context(|| format!("opening project {}", path.display()))?;
            let manifest = root.join("ecp5.toml");
            let file = manifest.exists().then_some(manifest);
            (root, file)
        } else {
            ensure!(
                path.file_name().is_some_and(|n| n == "ecp5.toml"),
                "expected a project folder or ecp5.toml: {}",
                path.display()
            );
            let file =
                fs::canonicalize(path).with_context(|| format!("opening {}", path.display()))?;
            (
                file.parent()
                    .context("manifest has no parent")?
                    .to_path_buf(),
                Some(file),
            )
        };
        let m: Manifest = match &file {
            Some(file) => toml::from_str(&fs::read_to_string(file)?)
                .with_context(|| format!("reading {}", file.display()))?,
            None => Manifest::default(),
        };
        let memory_images = match m.memory.images {
            Some(images) => images,
            None if root.join("ecpbram.txt").exists() => {
                fs::read_to_string(root.join("ecpbram.txt"))?
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
                    .map(PathBuf::from)
                    .collect()
            }
            None => Vec::new(),
        };
        let constraints = match m.sources.constraints {
            Some(path) => path,
            None => {
                let paths = matching(&root, &["lpf"])?;
                ensure!(
                    paths.len() == 1,
                    "{} has {} .lpf files; name one with sources.constraints in ecp5.toml",
                    root.display(),
                    paths.len()
                );
                paths[0].clone()
            }
        };
        ensure!(
            m.sources.hdl.is_none() || m.sources.verilog.is_none(),
            "sources.verilog and sources.hdl cannot be supplied together"
        );
        ensure!(
            m.sources.hdl.is_some() || m.sources.include_dirs.is_none(),
            "sources.include_dirs is only valid with sources.hdl"
        );
        let include_dirs = if m.sources.hdl.is_some() {
            m.sources.include_dirs.unwrap_or_else(|| vec![".".into()])
        } else {
            Vec::new()
        };
        let verilog = match &m.sources.hdl {
            Some(hdl) => {
                ensure!(!hdl.is_empty(), "no HDL roots in {}", root.display());
                let mut seen = HashSet::new();
                let mut vhdl = false;
                let mut other = false;
                for path in hdl {
                    relative(path)?;
                    let normalized: PathBuf = path
                        .components()
                        .filter(|c| !matches!(c, Component::CurDir))
                        .collect();
                    ensure!(
                        seen.insert(normalized),
                        "duplicate HDL root: {}",
                        path.display()
                    );
                    if HdlLanguage::from_path(path)? == HdlLanguage::Vhdl {
                        vhdl = true;
                    } else {
                        other = true;
                    }
                }
                ensure!(
                    !vhdl || !other,
                    "Cross-language VHDL hierarchy is not supported yet"
                );
                hdl.clone()
            }
            None => m.sources.verilog.unwrap_or(matching(&root, &["v"])?),
        };
        ensure!(
            !verilog.is_empty(),
            "no Verilog sources in {}",
            root.display()
        );
        let files = m.sources.files.unwrap_or(
            matching(&root, &["hex", "mem", "vh", "svh", "init"])?
                .into_iter()
                .filter(|p| !memory_images.contains(p))
                .collect(),
        );
        let mut top = m.project.top.unwrap_or_else(|| "top".into());
        ensure!(
            !top.is_empty()
                && top
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$'),
            "invalid top module {top:?}"
        );
        if m.sources.hdl.as_ref().is_some_and(|roots| {
            roots
                .iter()
                .any(|p| HdlLanguage::from_path(p).ok() == Some(HdlLanguage::Vhdl))
        }) {
            ensure!(
                top.starts_with(|c: char| c.is_ascii_alphabetic())
                    && top.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !top.ends_with('_')
                    && !top.contains("__"),
                "invalid VHDL basic entity identifier {top:?}"
            );
            top.make_ascii_lowercase();
        }
        let mut project = Self {
            name: m.project.name.unwrap_or_else(|| {
                root.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
            root,
            file,
            top,
            verilog,
            hdl: m.sources.hdl,
            include_dirs,
            sv_frontend: m.synthesis.sv_frontend,
            constraints,
            files,
            memory_images,
            device: Device {
                part: m.device.part.unwrap_or_else(|| "LFE5U-85F".into()),
                package: m.device.package.unwrap_or_else(|| "CABGA381".into()),
                speed: m.device.speed,
            },
            build_dir: PathBuf::new(),
            seed: m.build.seed.unwrap_or(1),
            allow_timing_failure: m.build.allow_timing_failure,
        };
        project.device.flag()?;
        for path in project
            .hdl_roots()
            .iter()
            .chain(&project.files)
            .chain(&project.memory_images)
            .chain(std::iter::once(&project.constraints))
        {
            // A path stays inside the folder by its spelling; a symbolic link
            // in the folder may still point elsewhere (a core shared between
            // projects without a second copy).
            relative(path)?;
            let full = project.root.join(path);
            ensure!(full.is_file(), "missing project file {}", full.display());
        }
        for path in &project.include_dirs {
            relative(path)?;
            // The built-in reader does not unquote its attached -Idir option.
            ensure!(
                path.to_string_lossy().chars().all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c)),
                "built-in Yosys include directory must use ASCII letters, digits or _./- (quoted -Idir options are unsupported): {}",
                path.display()
            );
            let full = project.root.join(path);
            ensure!(
                full.is_dir(),
                "missing include directory {}",
                full.display()
            );
        }
        for image in &project.memory_images {
            ensure!(
                !project.files.contains(image) && !project.hdl_roots().contains(image),
                "memory image {} must not also be a synthesis source",
                image.display()
            );
        }
        let dir = m.build.dir.unwrap_or_else(|| "build".into());
        relative(&dir)?;
        project.set_build_dir(project.root.join(dir))?;
        Ok(project)
    }
    /// Whether this project requires the GHDL plugin.
    pub fn has_vhdl(&self) -> bool {
        self.hdl.as_ref().is_some_and(|roots| {
            roots
                .iter()
                .any(|p| HdlLanguage::from_path(p).ok() == Some(HdlLanguage::Vhdl))
        })
    }
    /// All compilation roots, in build order, for source registration and probing.
    pub fn hdl_roots(&self) -> &[PathBuf] {
        self.hdl.as_deref().unwrap_or(&self.verilog)
    }
    /// Overrides the output folder; relative overrides are relative to the caller.
    pub fn set_build_dir(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = std::path::absolute(path)?;
        // Resolve existing ancestors too, so a symlink cannot select the project itself.
        fn resolve(path: &Path) -> Result<PathBuf> {
            if path.exists() {
                return Ok(fs::canonicalize(path)?);
            }
            let parent = resolve(path.parent().context("output folder has no parent")?)?;
            Ok(parent.join(path.file_name().context("output folder has no name")?))
        }
        let path = resolve(&path)?;
        ensure!(
            path != self.root,
            "build directory must not be the project folder itself"
        );
        self.build_dir = path;
        Ok(())
    }
    /// Returns all fixed output names, regardless of the selected top module.
    pub fn outputs(&self) -> Outputs {
        let p = |name| self.build_dir.join(name);
        Outputs {
            json: p("top.json"),
            config: p("top.config"),
            placeholder_config: (!self.memory_images.is_empty())
                .then(|| p("top_placeholder.config")),
            routed_json: p("top_routed.json"),
            bit: p("top.bit"),
            yosys_log: p("yosys.log"),
            nextpnr_log: p("nextpnr.log"),
            report: p("report.json"),
        }
    }
    /// Explains whether the emulator models the selected part and package.
    pub fn emulatable(&self) -> std::result::Result<(), String> {
        if self.device.part == "LFE5U-85F" && self.device.package == "CABGA381" {
            Ok(())
        } else {
            Err(format!(
                "the emulator models only LFE5U-85F/CABGA381; this project selects {}/{}",
                self.device.part, self.device.package
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;
    #[test]
    fn implicit_and_toml_defaults() {
        let f = Folder::new();
        fs::write(f.0.join("message.hex"), "12\n").unwrap();
        fs::write(f.0.join("fw.hex"), "34\n").unwrap();
        fs::write(f.0.join("ecpbram.txt"), "# firmware\n\nfw.hex\n").unwrap();
        let p = Project::open(&f.0).unwrap();
        assert!(p.root.is_absolute());
        assert!(p.file.is_none());
        assert_eq!(p.verilog, [PathBuf::from("top.v")]);
        assert_eq!(p.hdl_roots(), p.verilog);
        assert!(p.hdl.is_none());
        assert!(p.include_dirs.is_empty());
        assert_eq!(p.sv_frontend, SvFrontend::Auto);
        assert_eq!(p.files, [PathBuf::from("message.hex")]);
        assert_eq!(p.memory_images, [PathBuf::from("fw.hex")]);
        assert_eq!(p.seed, 1);
        assert_eq!(p.top, "top");
        assert_eq!(p.device.flag().unwrap(), "--85k");
        assert_eq!(p.build_dir, p.root.join("build"));
        assert!(p.outputs().placeholder_config.is_some());
        assert!(p.emulatable().is_ok());
        fs::write(f.0.join("ecp5.toml"), "[project]\n[device]\n[build]\n").unwrap();
        let explicit = Project::open(f.0.join("ecp5.toml")).unwrap();
        assert!(explicit.file.is_some());
        assert_eq!(explicit.verilog, p.verilog);
    }
    #[test]
    fn ordered_hdl_frontend_and_includes() {
        let f = Folder::new();
        fs::create_dir(f.0.join("include")).unwrap();
        for name in ["types.SV", "leaf.V", "top.sv", "types.VHD", "top.VhDl"] {
            fs::write(f.0.join(name), "").unwrap();
        }
        fs::write(f.0.join("include/settings.svh"), "").unwrap();
        for frontend in ["auto", "builtin", "slang"] {
            fs::write(f.0.join("ecp5.toml"), format!(
                "[sources]\nhdl=['types.SV','leaf.V','top.sv']\ninclude_dirs=['include']\nfiles=['include/settings.svh']\n[synthesis]\nsv_frontend='{frontend}'\n"
            )).unwrap();
            let p = Project::open(&f.0).unwrap();
            assert_eq!(
                p.hdl_roots(),
                ["types.SV", "leaf.V", "top.sv"].map(PathBuf::from)
            );
            assert_eq!(p.include_dirs, [PathBuf::from("include")]);
            assert_eq!(p.files, [PathBuf::from("include/settings.svh")]);
            assert_eq!(p.sv_frontend.to_string(), frontend);
        }
        for (name, language) in [
            ("leaf.V", HdlLanguage::Verilog),
            ("types.SV", HdlLanguage::SystemVerilog),
            ("types.VHD", HdlLanguage::Vhdl),
            ("top.VhDl", HdlLanguage::Vhdl),
        ] {
            assert_eq!(HdlLanguage::from_path(Path::new(name)).unwrap(), language);
        }
        fs::write(
            f.0.join("ecp5.toml"),
            "[sources]\nhdl=['types.VHD','top.VhDl']",
        )
        .unwrap();
        let p = Project::open(&f.0).unwrap();
        assert_eq!(p.include_dirs, [PathBuf::from(".")]);
        assert_eq!(p.sv_frontend, SvFrontend::Auto);
        // New suffixes never enter an old project's sorted *.v default.
        fs::write(f.0.join("ecp5.toml"), "").unwrap();
        assert_eq!(
            Project::open(&f.0).unwrap().hdl_roots(),
            [PathBuf::from("top.v")]
        );
    }
    #[test]
    fn hdl_validation() {
        let f = Folder::new();
        fs::write(f.0.join("top.sv"), "").unwrap();
        fs::write(f.0.join("top.vhd"), "").unwrap();
        for (text, expected) in [
            (
                "[sources]\nhdl=['top.sv']\nverilog=[]",
                "cannot be supplied together",
            ),
            ("[sources]\ninclude_dirs=[]", "only valid with sources.hdl"),
            ("[sources]\nhdl=['top.txt']", "unknown HDL root suffix"),
            ("[sources]\nhdl=['top.sv','./top.sv']", "duplicate HDL root"),
            ("[sources]\nhdl=[]", "no HDL roots"),
            ("[sources]\nhdl=['../top.sv']", "cannot contain '..'"),
            (
                "[sources]\nhdl=['top.sv']\ninclude_dirs=['../include']",
                "cannot contain '..'",
            ),
            (
                "[sources]\nhdl=['top.sv']\ninclude_dirs=['/tmp']",
                "must be relative",
            ),
            (
                "[sources]\nhdl=['top.sv']\ninclude_dirs=['missing']",
                "missing include directory",
            ),
            (
                "[sources]\nhdl=['top.sv']\ninclude_dirs=['include with spaces']",
                "quoted -Idir options are unsupported",
            ),
            ("[sources]\nhdl=['missing.sv']", "missing project file"),
            (
                "[sources]\nhdl=['top.vhd','top.sv']",
                "Cross-language VHDL hierarchy is not supported yet",
            ),
            ("[synthesis]\nsv_frontend='other'", "unknown variant"),
        ] {
            fs::write(f.0.join("ecp5.toml"), text).unwrap();
            let error = format!("{:#}", Project::open(&f.0).unwrap_err());
            assert!(error.contains(expected), "{text}: {error}");
        }
        // Legacy input lists keep Verilog mode even for explicitly named .sv.
        fs::write(f.0.join("ecp5.toml"), "[sources]\nverilog=['top.sv']").unwrap();
        let p = Project::open(&f.0).unwrap();
        assert!(p.hdl.is_none());
        assert_eq!(p.hdl_roots(), [PathBuf::from("top.sv")]);
    }
    #[test]
    fn manifest_validation_and_overrides() {
        let f = Folder::new();
        for (text, expected) in [
            ("typo = 1", "unknown field"),
            ("[project]\ntopp = 'x'", "unknown field"),
            ("[sources]\nverilog = ['../top.v']", "cannot contain '..'"),
            ("[sources]\nconstraints = '/top.lpf'", "must be relative"),
            ("[sources]\nfiles = ['missing.hex']", "missing.hex"),
            ("[build]\ndir = '.'", "must not be the project folder"),
        ] {
            fs::write(f.0.join("ecp5.toml"), text).unwrap();
            let error = format!("{:#}", Project::open(&f.0).unwrap_err());
            assert!(error.contains(expected), "{error}");
        }
        fs::create_dir(f.0.join("rtl")).unwrap();
        fs::write(f.0.join("rtl/cpu.v"), "module cpu; endmodule").unwrap();
        fs::write(f.0.join("ecp5.toml"), "[project]\nname='cpu'\ntop='cpu'\n[sources]\nverilog=['rtl/cpu.v', 'top.v']\nfiles=[]\n[memory]\nimages=[]\n[device]\npart='LFE5UM5G-45F'\npackage='CABGA554'\nspeed=8\n[build]\ndir='output/nested'\nseed=7\nallow_timing_failure=true\n").unwrap();
        let p = Project::open(&f.0).unwrap();
        assert_eq!(p.name, "cpu");
        assert_eq!(p.top, "cpu");
        assert_eq!(p.verilog[0], PathBuf::from("rtl/cpu.v"));
        assert_eq!(p.device.flag().unwrap(), "--um5g-45k");
        assert_eq!(p.device.speed, Some(8));
        assert!(p.emulatable().unwrap_err().contains("LFE5U-85F/CABGA381"));
        assert!(p.allow_timing_failure);
        assert_eq!(p.seed, 7);
    }
    /// A Verilog file linked from another project is a source by default and
    /// by name, as examples/gallery shares the PicoRV32 core.
    #[cfg(unix)]
    #[test]
    fn linked_sources_count() {
        let shared = Folder::new();
        fs::write(shared.0.join("core.v"), "module core; endmodule\n").unwrap();
        let f = Folder::new();
        std::os::unix::fs::symlink(shared.0.join("core.v"), f.0.join("core.v")).unwrap();
        let p = Project::open(&f.0).unwrap();
        assert_eq!(p.verilog, [PathBuf::from("core.v"), PathBuf::from("top.v")]);
        fs::write(
            f.0.join("ecp5.toml"),
            "[sources]\nverilog = ['top.v', 'core.v']\n",
        )
        .unwrap();
        assert_eq!(Project::open(&f.0).unwrap().verilog.len(), 2);
    }
    #[test]
    fn constraints_must_be_unambiguous() {
        let f = Folder::new();
        fs::remove_file(f.0.join("top.lpf")).unwrap();
        assert!(Project::open(&f.0)
            .unwrap_err()
            .to_string()
            .contains("sources.constraints"));
        fs::write(f.0.join("a.lpf"), "").unwrap();
        fs::write(f.0.join("b.lpf"), "").unwrap();
        assert!(Project::open(&f.0)
            .unwrap_err()
            .to_string()
            .contains("2 .lpf"));
    }
}
