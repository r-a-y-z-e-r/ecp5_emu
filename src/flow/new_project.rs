//! New project creates a portable ULX3S counter and safe peripheral defaults.
//! Creation validates the choices and stages three
//! self-contained files beside the destination before publishing the project;
//! it never builds, downloads tools or opens a window.
use super::Project;
use crate::board;
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fmt::Write,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

/// Board resources available to every project creation client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resource {
    Clock,
    Leds,
    Buttons,
    Switches,
    Uart,
    Oled,
    Hdmi,
    Sd,
    Audio,
    Sdram,
    Adc,
    Rtc,
    Flash,
    Header,
}

/// A required selection and its user-facing explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub resource: Resource,
    pub requires: Resource,
    pub reason: &'static str,
}

/// Shared by validation and creation windows; selections are never added silently.
pub const DEPENDENCIES: &[Dependency] = &[
    Dependency {
        resource: Resource::Buttons,
        requires: Resource::Leds,
        reason: "buttons require LEDs",
    },
    Dependency {
        resource: Resource::Switches,
        requires: Resource::Leds,
        reason: "switches require LEDs",
    },
    Dependency {
        resource: Resource::Uart,
        requires: Resource::Clock,
        reason: "UART requires clock",
    },
    Dependency {
        resource: Resource::Oled,
        requires: Resource::Clock,
        reason: "OLED requires clock for SPI timing",
    },
    Dependency {
        resource: Resource::Hdmi,
        requires: Resource::Clock,
        reason: "HDMI requires clock for pixel and serial timing",
    },
    Dependency {
        resource: Resource::Sd,
        requires: Resource::Clock,
        reason: "SD requires clock for SPI timing",
    },
    Dependency {
        resource: Resource::Audio,
        requires: Resource::Clock,
        reason: "audio requires clock for sample timing",
    },
    Dependency {
        resource: Resource::Sdram,
        requires: Resource::Clock,
        reason: "SDRAM requires clock for controller timing",
    },
    Dependency {
        resource: Resource::Adc,
        requires: Resource::Clock,
        reason: "ADC requires clock for SPI timing",
    },
    Dependency {
        resource: Resource::Rtc,
        requires: Resource::Clock,
        reason: "RTC requires clock for I2C timing",
    },
    Dependency {
        resource: Resource::Flash,
        requires: Resource::Clock,
        reason: "Flash requires clock for SPI timing",
    },
    Dependency {
        resource: Resource::Header,
        requires: Resource::Clock,
        reason: "header requires clock for input sampling",
    },
];

impl Resource {
    pub const ALL: [Self; 14] = [
        Self::Clock,
        Self::Leds,
        Self::Buttons,
        Self::Switches,
        Self::Uart,
        Self::Oled,
        Self::Hdmi,
        Self::Sd,
        Self::Audio,
        Self::Sdram,
        Self::Adc,
        Self::Rtc,
        Self::Flash,
        Self::Header,
    ];

    /// Resource names shared by the CLI and other creation clients.
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "clock" => Self::Clock,
            "leds" => Self::Leds,
            "buttons" => Self::Buttons,
            "switches" => Self::Switches,
            "uart" => Self::Uart,
            "oled" => Self::Oled,
            "hdmi" => Self::Hdmi,
            "sd" => Self::Sd,
            "audio" => Self::Audio,
            "sdram" => Self::Sdram,
            "adc" => Self::Adc,
            "rtc" => Self::Rtc,
            "flash" => Self::Flash,
            "header" => Self::Header,
            _ => bail!("unknown or unavailable resource {name:?}"),
        })
    }
}

/// A top-level signal's direction in the generated HDL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
    Inout,
}

/// One resource's top-level port, in declaration order, without dependencies.
/// Escaped scalar names (such as SD's `sd_d[0]`) omit the Verilog backslash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    /// `None` denotes a scalar, including an escaped scalar name.
    pub width: Option<usize>,
    pub direction: Direction,
}

/// The ports that the generator declares for this resource alone.
pub fn ports(resource: Resource) -> Vec<Port> {
    resource
        .port_declarations()
        .iter()
        .map(|declaration| {
            let (name, width, direction) = port_parts(declaration);
            Port {
                name: name.into(),
                width,
                direction,
            }
        })
        .collect()
}

/// Whether a resource names one signal or a group of distinct signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortNaming {
    Single { default: &'static str },
    Prefix { common: Option<&'static str> },
}

/// Derive naming from the declarations, using their shared `namespace_` prefix.
pub fn port_naming(resource: Resource) -> PortNaming {
    let declarations = resource.port_declarations();
    let (first, _, _) = port_parts(declarations[0]);
    if declarations.len() == 1 {
        return PortNaming::Single { default: first };
    }
    let common = first.find('_').map(|i| &first[..=i]).filter(|prefix| {
        declarations
            .iter()
            .all(|declaration| port_parts(declaration).0.starts_with(prefix))
    });
    PortNaming::Prefix { common }
}

/// Keep generated names, rename a single signal, or replace a shared prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming<'a> {
    Default,
    Base(&'a str),
    Prefix(&'a str),
}

fn validate_identifier(name: &str) -> Result<()> {
    let mut chars = name.chars();
    ensure!(
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$')),
        "invalid Verilog identifier {name:?}: start with a letter or underscore, then use letters, digits, underscores or $"
    );
    Ok(())
}

/// The generator's LPF commands for one resource, with optional signal naming.
/// Preserve board.lpf command order, formatting and settings. GPDI DN pads
/// follow DP electrically; Flash SCK needs USRMCLK, not an LPF port.
pub fn lpf_lines(resource: Resource, naming: Naming<'_>) -> Result<String> {
    let rename = match (naming, port_naming(resource)) {
        (Naming::Default, _) => None,
        (Naming::Base(name), PortNaming::Single { default }) => Some((default, name)),
        (
            Naming::Prefix(name),
            PortNaming::Prefix {
                common: Some(prefix),
            },
        ) => Some((prefix, name)),
        (Naming::Base(_), PortNaming::Prefix { .. }) => {
            bail!("Base naming requires a single signal or bus; {resource:?} has multiple signals")
        }
        (Naming::Prefix(_), PortNaming::Single { .. }) => {
            bail!("Prefix naming requires multiple signals with a common prefix; {resource:?} is a single signal or bus")
        }
        (Naming::Prefix(_), PortNaming::Prefix { common: None }) => {
            bail!("{resource:?} ports have no common prefix to replace")
        }
    };
    if let Some((_, name)) = rename {
        validate_identifier(name)?;
    }
    let resources = BTreeSet::from([resource]);
    // Reuse the complete generator, including its clock frequency and board
    // settings. Only remove comments and substitute quoted port names.
    let mut text = String::new();
    for line in lpf_commands(&constraints(&resources)) {
        if let Some((old, new)) = rename {
            let port = line.split('"').nth(1).unwrap();
            let suffix = port
                .strip_prefix(old)
                .expect("command belongs to a resource port");
            // Rename each original command once, even if the new name matches
            // another original port (for example audio_r or sd_d).
            writeln!(
                text,
                "{}",
                line.replacen(&format!("\"{port}\""), &format!("\"{new}{suffix}\""), 1)
            )
            .unwrap();
        } else {
            writeln!(text, "{line}").unwrap();
        }
    }

    Ok(text)
}

/// Explicit choices; dependencies are reported rather than added silently.
#[derive(Clone, Debug)]
pub struct Options {
    pub dir: PathBuf,
    pub name: String,
    pub resources: BTreeSet<Resource>,
}

impl Options {
    /// Defaults to the folder's leaf name and the clock/LED demonstration.
    /// A UI can suggest this leaf below `~/ecp5-projects` before calling Create.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let name = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            dir,
            name,
            resources: BTreeSet::from([Resource::Clock, Resource::Leds]),
        }
    }

    /// Checks fields, dependencies and the current destination without writing.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty(),
            "project name must not be blank"
        );
        let path = self.dir.as_os_str().as_encoded_bytes();
        let separator = std::path::MAIN_SEPARATOR as u8;
        let end = path
            .iter()
            .rposition(|b| *b != separator)
            .map_or(0, |i| i + 1);
        let leaf = path[..end]
            .rsplit(|b| *b == separator)
            .next()
            .unwrap_or(b"");
        ensure!(
            self.dir.file_name().is_some()
                && !String::from_utf8_lossy(leaf).trim().is_empty()
                && leaf != b"."
                && leaf != b".."
                && !path.contains(&0),
            "invalid project folder leaf: {}",
            self.dir.display()
        );
        let has = |r| self.resources.contains(&r);
        let mut missing = Vec::new();
        if !has(Resource::Clock) {
            missing.push("clock is required by every recipe");
        }
        for dependency in DEPENDENCIES {
            if has(dependency.resource) && !has(dependency.requires) {
                missing.push(dependency.reason);
            }
        }
        // The initial demonstrations need an observable output. Peripheral
        // starters may deliberately only hold a device idle or sample inputs.
        if !has(Resource::Leds)
            && !has(Resource::Uart)
            && !self.resources.iter().any(|r| r.template().is_some())
        {
            missing.push("at least one observable output is required (LEDs or UART)");
        }
        ensure!(missing.is_empty(), "{}", missing.join("; "));
        let mut balls = BTreeSet::new();
        for pin in pins(&self.resources) {
            ensure!(
                balls.insert(pin.ball),
                "duplicate ball ownership: {}",
                pin.ball
            );
        }
        destination_available(&self.dir)
    }
}

fn destination_available(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(meta) => {
            ensure!(
                meta.is_dir(),
                "destination is not a directory: {}",
                dir.display()
            );
            ensure!(
                fs::read_dir(dir)?.next().transpose()?.is_none(),
                "destination directory is not empty: {}",
                dir.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("checking {}", dir.display())),
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(super) struct Staging {
    pub(super) path: PathBuf,
    published: bool,
}
impl Staging {
    pub(super) fn new(parent: &Path) -> Result<Self> {
        loop {
            let dir = parent.join(format!(
                ".ecp5-new-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    return Ok(Self {
                        path: dir,
                        published: false,
                    })
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).context("creating temporary project folder"),
            }
        }
    }
    pub(super) fn publish(mut self, destination: &Path) -> Result<Project> {
        destination_available(destination)?;
        // A directory rename cannot replace a file, symlink or nonempty
        // directory, including one occupied after the check above. An empty
        // destination directory is deliberately allowed by the creation policy.
        fs::rename(&self.path, destination).context("publishing new project")?;
        self.published = true;
        Project::open(destination)
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Writes a new project and returns its validated, published inputs.
pub fn create(options: &Options) -> Result<Project> {
    options.validate()?;
    let parent = options
        .dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).context("creating project parent folder")?;
    let stage = Staging::new(parent)?;
    for (name, text) in files(options) {
        fs::write(stage.path.join(name), text).with_context(|| format!("writing {name}"))?;
    }
    Project::open(&stage.path).context("validating generated project")?;
    stage.publish(&options.dir)
}

fn files(options: &Options) -> [(&'static str, String); 3] {
    // Serialize just the new string value, never an existing user manifest.
    let name = toml::Value::String(options.name.clone());
    let manifest = format!(
        "[project]\nname = {name}\ntop = \"top\"\n\n[sources]\nverilog = [\"top.v\"]\nconstraints = \"top.lpf\"\n\n[device]\npart = \"LFE5U-85F\"\npackage = \"CABGA381\"\n"
    );
    [
        ("ecp5.toml", manifest),
        ("top.v", verilog(&options.resources)),
        ("top.lpf", constraints(&options.resources)),
    ]
}

const BOARD_LPF: &str = include_str!("../../assets/ulx3s/board.lpf");

const LPF_HEADER: &str = r#"# LPF (Lattice Preference File) maps top-level ports to package pins and sets I/O.
# LOCATE COMP "port" SITE "ball"; connects a port to a physical package ball.
# A ball name is a CABGA381 coordinate: row letter plus column number,
# as seen on the package underside; for example, G2 is row G, column 2.
# IOBUF PORT "port" IO_TYPE=... PULLMODE=... DRIVE=...;
# IO_TYPE selects the electrical standard: LVCMOS33 means 3.3 V on ULX3S.
# PULLMODE selects a weak internal pull: UP to 1, DOWN to 0, NONE disables it.
# DRIVE sets output drive strength in mA; it does not set an input's level.
# FREQUENCY PORT "clk_25mhz" 25 MHZ; tells timing analysis the clock rate.
# Bus bits are separate ports, written in quotes, for example "led[0]".
# Ball names: Radiona ULX3S, EMARD's doc/constraints/ulx3s_v20.lpf.
# Pin-constraint licence: assets/ulx3s/LICENSE.md.
"#;

fn lpf_commands(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

struct Pin {
    port: String,
    ball: &'static str,
    group: &'static str,
    provenance: &'static str,
}

impl Resource {
    fn port_declarations(self) -> &'static [&'static str] {
        match self {
            Self::Clock => &["input clk_25mhz"],
            Self::Leds => &["output [7:0] led"],
            Self::Buttons => &["input [6:0] btn"],
            Self::Switches => &["input [3:0] sw"],
            Self::Uart => &["output ftdi_rxd", "input ftdi_txd"],
            _ => self.template().unwrap().0,
        }
    }

    fn port_bases(self) -> &'static [&'static str] {
        match self {
            Self::Clock => &["clk_25mhz"],
            Self::Leds => &["led"],
            Self::Buttons => &["btn"],
            Self::Switches => &["sw"],
            Self::Uart => &["ftdi_rxd", "ftdi_txd"],
            Self::Oled => &["oled_clk", "oled_mosi", "oled_dc", "oled_resn", "oled_csn"],
            Self::Hdmi => &["gpdi_dp"],
            Self::Sd => &["sd_clk", "sd_cmd", "sd_d"],
            Self::Audio => &["audio_l", "audio_r", "audio_v"],
            Self::Sdram => &[
                "sdram_clk",
                "sdram_cke",
                "sdram_csn",
                "sdram_wen",
                "sdram_rasn",
                "sdram_casn",
                "sdram_ba",
                "sdram_dqm",
                "sdram_a",
                "sdram_d",
            ],
            Self::Adc => &["adc_csn", "adc_sclk", "adc_mosi", "adc_miso", "adc_ain"],
            Self::Rtc => &["rtc_scl", "rtc_sda"],
            Self::Flash => &[
                "flash_csn",
                "flash_mosi",
                "flash_miso",
                "flash_wpn",
                "flash_holdn",
            ],
            Self::Header => &["gn", "gp"],
        }
    }

    fn group(self) -> &'static str {
        match self {
            Self::Clock => "25 MHz oscillator",
            Self::Leds => "LEDs D0..D7",
            Self::Buttons => "Buttons PWR, FIRE1, FIRE2, UP, DOWN, LEFT, RIGHT",
            Self::Switches => "DIP switches",
            Self::Uart => "FTDI UART",
            Self::Oled => "OLED SPI and control",
            Self::Hdmi => "GPDI video outputs (differential pairs)",
            Self::Sd => "SD card SPI",
            Self::Audio => "Audio DAC",
            Self::Sdram => "SDRAM",
            Self::Adc => "ADC SPI and analog inputs",
            Self::Rtc => "RTC I2C bus",
            Self::Flash => "SPI flash (SCK uses USRMCLK, never LOCATE U3)",
            Self::Header => "Expansion headers GP/GN: inputs only",
        }
    }
}

fn pins(resources: &BTreeSet<Resource>) -> Vec<Pin> {
    let mut pins = Vec::new();
    for resource in resources {
        for locate in lpf_commands(BOARD_LPF).filter(|l| l.starts_with("LOCATE ")) {
            let mut fields = locate.split('"');
            let port = fields.nth(1).unwrap();
            let ball = fields.nth(1).unwrap();
            let base = port.split('[').next().unwrap();
            if !resource.port_bases().contains(&base) {
                continue;
            }
            // AIN0..7 are GN/GP14..17. One physical input buffer and pull
            // serves both resources; header owns the pads when both are chosen.
            if base == "adc_ain" && resources.contains(&Resource::Header) {
                continue;
            }
            pins.push(Pin {
                port: port.into(),
                ball,
                group: resource.group(),
                provenance: "Reference: assets/ulx3s/board.lpf; balls checked against src/board.rs",
            });
        }
    }
    pins
}

fn constraints(resources: &BTreeSet<Resource>) -> String {
    let mut text = String::from(LPF_HEADER);
    text.push_str("\n# ULX3S constraints: assets/ulx3s/board.lpf.\n# Licence: assets/ulx3s/LICENSE.md; catalogue: licenses/README.md.\n");
    for line in include_str!("../../assets/ulx3s/LICENSE.md").lines() {
        writeln!(text, "# {line}").unwrap();
    }
    let pins = pins(resources);
    let mut previous = "";
    let mut previous_group = "";
    // Preserve the reference command order as well as every command's bytes.
    for line in lpf_commands(BOARD_LPF) {
        let Some(pin) = pins
            .iter()
            .find(|pin| line.split('"').nth(1) == Some(pin.port.as_str()))
        else {
            continue;
        };
        if pin.group != previous_group {
            writeln!(text, "\n# {}", pin.group).unwrap();
            previous_group = pin.group;
        }
        if pin.provenance != previous {
            writeln!(text, "# {}", pin.provenance).unwrap();
            previous = pin.provenance;
        }
        if line.starts_with("LOCATE ") {
            if let Some(button) = board::BUTTONS.iter().find(|b| b.ball == pin.ball) {
                writeln!(
                    text,
                    "# {}: released level {}",
                    button.name,
                    u8::from(button.idle)
                )
                .unwrap();
            }
        }
        writeln!(text, "{line}").unwrap();
    }

    text
}

impl Resource {
    fn template(self) -> Option<(&'static [&'static str], &'static str)> {
        Some(match self {
            Self::Oled => (
                &[
                    r"output oled_clk",
                    r"output oled_mosi",
                    r"output oled_dc",
                    r"output oled_resn",
                    r"output oled_csn",
                ],
                r#"
  // See examples/ulx3s/oled_pattern.
  assign oled_clk = 1'b0;
  assign oled_mosi = 1'b0;
  assign oled_dc = 1'b0;
  assign oled_resn = 1'b0;
  assign oled_csn = 1'b1;
"#,
            ),
            Self::Hdmi => (
                &[r"output [3:0] gpdi_dp"],
                r#"
  // See examples/ulx3s/hdmi_bars.
  // LVCMOS33D supplies the complementary GPDI_DN pads automatically.
  assign gpdi_dp = 4'b0;
"#,
            ),
            Self::Sd => (
                &[
                    r"output sd_clk",
                    r"output sd_cmd",
                    r"(* keep *) input \sd_d[0] ",
                    r"output \sd_d[3] ",
                ],
                r#"
  // See examples/ulx3s/sd_read.
  assign sd_clk = 1'b0;
  assign sd_cmd = 1'b1;
  assign \sd_d[3] = 1'b1;
  (* keep *) wire sd_input = \sd_d[0] ;
"#,
            ),
            Self::Audio => (
                &[
                    r"output [3:0] audio_l",
                    r"output [3:0] audio_r",
                    r"output [3:0] audio_v",
                ],
                r#"
  // See examples/ulx3s/audio_tone.
  assign audio_l = 4'b0;
  assign audio_r = 4'b0;
  assign audio_v = 4'b0;
"#,
            ),
            Self::Sdram => (
                &[
                    r"output sdram_clk",
                    r"output sdram_cke",
                    r"output sdram_csn",
                    r"output sdram_rasn",
                    r"output sdram_casn",
                    r"output sdram_wen",
                    r"output [1:0] sdram_ba",
                    r"output [12:0] sdram_a",
                    r"output [1:0] sdram_dqm",
                    r"inout [15:0] sdram_d",
                ],
                r#"
  // See examples/ulx3s/sdram_test.
  assign sdram_clk = 1'b0;
  assign sdram_cke = 1'b0;
  assign sdram_csn = 1'b1;
  assign sdram_rasn = 1'b1;
  assign sdram_casn = 1'b1;
  assign sdram_wen = 1'b1;
  assign sdram_ba = 2'b0;
  assign sdram_a = 13'b0;
  assign sdram_dqm = 2'b11;
  // Explicit input buffers keep the data bus released after synthesis.
  (* keep *) wire [15:0] sdram_inputs;
  genvar bit_index;
  generate for (bit_index = 0; bit_index < 16; bit_index = bit_index + 1) begin: data_pads
    BB pad(.I(1'b0), .T(1'b1), .O(sdram_inputs[bit_index]), .B(sdram_d[bit_index]));
  end endgenerate
"#,
            ),
            Self::Adc => (
                &[
                    r"output adc_csn",
                    r"output adc_sclk",
                    r"output adc_mosi",
                    r"(* keep *) input adc_miso",
                    r"(* keep *) input [7:0] adc_ain",
                ],
                r#"
  // See examples/ulx3s/adc_read.
  assign adc_csn = 1'b1;
  assign adc_sclk = 1'b0;
  assign adc_mosi = 1'b0;
  (* keep *) wire adc_input = adc_miso;
  (* keep *) wire [7:0] analog_pads = adc_ain;
"#,
            ),
            Self::Rtc => (
                &[r"inout rtc_scl", r"inout rtc_sda"],
                r#"
  // See examples/ulx3s/rtc_read.
  // Release the open-drain bus; never drive either line high.
  (* keep *) wire [1:0] rtc_inputs;
  BB scl_pad(.I(1'b0), .T(1'b1), .O(rtc_inputs[0]), .B(rtc_scl));
  BB sda_pad(.I(1'b0), .T(1'b1), .O(rtc_inputs[1]), .B(rtc_sda));
"#,
            ),
            Self::Flash => (
                &[
                    r"output flash_csn",
                    r"output flash_mosi",
                    r"(* keep *) input flash_miso",
                    r"output flash_wpn",
                    r"output flash_holdn",
                ],
                r#"
  // See examples/ulx3s/flash_dump.
  assign flash_csn = 1'b1;
  assign flash_mosi = 1'b1;
  assign flash_wpn = 1'b1;
  assign flash_holdn = 1'b1;
  (* keep *) wire flash_input = flash_miso;
  wire sck = 1'b0;
  USRMCLK clock_pin(.USRMCLKI(sck), .USRMCLKTS(1'b0));
"#,
            ),
            Self::Header => (
                &[r"(* keep *) input [27:0] gp", r"(* keep *) input [27:0] gn"],
                r#"
  // See examples/ulx3s/header_probe.
  // J1/J2 inputs only, never driven by default; GN pull-up, GP pull-down.
  (* keep *) wire [27:0] header_gp = gp;
  (* keep *) wire [27:0] header_gn = gn;
"#,
            ),
            _ => return None,
        })
    }
}

// Read only the generator's own fixed declarations, including keep attributes
// and escaped SD scalar identifiers; this is not a general Verilog parser.
fn port_parts(declaration: &'static str) -> (&'static str, Option<usize>, Direction) {
    let mut words = declaration
        .split_whitespace()
        .skip_while(|word| !matches!(*word, "input" | "output" | "inout"));
    let direction = match words.next().unwrap() {
        "input" => Direction::Input,
        "output" => Direction::Output,
        "inout" => Direction::Inout,
        _ => unreachable!(),
    };
    let mut name = words.next().unwrap();
    let width = if name.starts_with('[') {
        let (high, low) = name[1..name.len() - 1].split_once(':').unwrap();
        let width = high.parse::<usize>().unwrap() - low.parse::<usize>().unwrap() + 1;
        name = words.next().unwrap();
        Some(width)
    } else {
        None
    };
    (name.strip_prefix('\\').unwrap_or(name), width, direction)
}

fn verilog(resources: &BTreeSet<Resource>) -> String {
    let has = |r| resources.contains(&r);
    let mut ports = Resource::Clock.port_declarations().to_vec();
    let mut text = String::from("// ULX3S: a 28-bit counter at 25 MHz.\n");
    if has(Resource::Leds) {
        text.push_str("// LEDs show count[27:20]");
        if has(Resource::Buttons) {
            text.push_str("; led[0] shows any pressed button");
        }
        if has(Resource::Switches) {
            text.push_str("; led[4:1] shows sw[3:0]");
        }
        text.push_str(".\n");
        ports.extend(Resource::Leds.port_declarations());
    }
    if has(Resource::Buttons) {
        ports.extend(Resource::Buttons.port_declarations());
    }
    if has(Resource::Switches) {
        ports.extend(Resource::Switches.port_declarations());
    }
    if has(Resource::Uart) {
        text.push_str(
            "// UART greets at startup, every 0.1 s, and on idle RX activity (not echo).\n",
        );
        ports.extend(Resource::Uart.port_declarations());
    }
    for resource in resources {
        if let Some((peripheral_ports, _)) = resource.template() {
            ports.extend(peripheral_ports.iter().copied().filter(|p| {
                !(resource == &Resource::Adc && has(Resource::Header) && p.ends_with("adc_ain"))
            }));
        }
    }
    writeln!(text, "module top(\n  {}\n);", ports.join(",\n  ")).unwrap();
    text.push_str(
        "  reg [27:0] count = 0;\n  always @(posedge clk_25mhz)\n    count <= count + 1;\n",
    );
    if has(Resource::Buttons) {
        let idle = board::BUTTONS
            .iter()
            .enumerate()
            .fold(0_u8, |m, (i, b)| m | (u8::from(b.idle) << i));
        writeln!(text, "\n  // Released levels from src/board.rs::BUTTONS; PWR is active low.\n  localparam [6:0] BUTTON_IDLE = 7'b{idle:07b};").unwrap();
        text.push_str("  reg [6:0] btn_meta = BUTTON_IDLE;\n  reg [6:0] btn_sync = BUTTON_IDLE;\n  always @(posedge clk_25mhz) begin\n    btn_meta <= btn;\n    btn_sync <= btn_meta;\n  end\n  wire any_pressed = |(btn_sync ^ BUTTON_IDLE);\n");
    }
    if has(Resource::Switches) {
        text.push_str("\n  reg [3:0] sw_meta = 0;\n  reg [3:0] sw_sync = 0;\n  always @(posedge clk_25mhz) begin\n    sw_meta <= sw;\n    sw_sync <= sw_meta;\n  end\n");
    }
    if has(Resource::Leds) {
        let bit0 = if has(Resource::Buttons) {
            "any_pressed"
        } else {
            "count[20]"
        };
        let middle = if has(Resource::Switches) {
            "sw_sync"
        } else {
            "count[24:21]"
        };
        if !has(Resource::Buttons) && !has(Resource::Switches) {
            text.push_str("\n  assign led = count[27:20];\n");
        } else {
            writeln!(text, "\n  assign led = {{count[27:25], {middle}, {bit0}}};").unwrap();
        }
    }
    if has(Resource::Uart) {
        let hz = 1_000_000_000 / board::CLOCK_PERIOD_NS;
        writeln!(text, "\n  // Framing adapted from examples/ulx3s/uart_hello/top.v (Apache-2.0).\n  // Copyright and licence: ecp5_emu contributors, repository LICENSE.\n  localparam BIT_CYCLES = {};\n  localparam PAUSE_CYCLES = {};", hz / board::BAUD, hz / 10).unwrap();
        text.push_str(UART_VERILOG);
    }
    if resources.iter().any(|r| r.template().is_some()) {
        // Retain the clock input even in a starter containing only idle outputs.
        text.push_str("\n  (* keep *) wire clock_input = clk_25mhz;\n");
    }
    for resource in resources {
        if let Some((_, block)) = resource.template() {
            text.push_str(block);
            if *resource == Resource::Adc && has(Resource::Header) {
                text.push_str("  // AIN0..7 share GN/GP14..17: use the header input buffers and pulls.\n  wire [7:0] adc_ain = {gp[17], gn[17], gp[16], gn[16], gp[15], gn[15], gp[14], gn[14]};\n");
            }
        }
    }
    text.push_str("endmodule\n");
    text
}

const UART_VERILOG: &str = r#"  localparam LENGTH = 15;
  reg rx_meta = 1;
  reg rx_sync = 1;
  reg rx_previous = 1;
  always @(posedge clk_25mhz) begin
    rx_meta <= ftdi_txd;
    rx_sync <= rx_meta;
    rx_previous <= rx_sync;
  end
  wire rx_falling = rx_previous & ~rx_sync;

  reg [3:0] index = 0;
  reg [7:0] char;
  always @*
    case (index)
      0: char = "H"; 1: char = "e"; 2: char = "l"; 3: char = "l"; 4: char = "o";
      5: char = ","; 6: char = " "; 7: char = "U"; 8: char = "L"; 9: char = "X";
      10: char = "3"; 11: char = "S"; 12: char = "!"; 13: char = 8'h0d;
      default: char = 8'h0a;
    endcase

  reg [9:0] frame = 10'h3ff; // Stop, eight data bits LSB first, start.
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  reg [21:0] pause = 0;
  always @(posedge clk_25mhz)
    if (bits_left != 0) begin
      if (baud == BIT_CYCLES - 1) begin
        baud <= 0;
        frame <= {1'b1, frame[9:1]};
        bits_left <= bits_left - 1;
      end else
        baud <= baud + 1;
    end else if (pause != 0) begin
      // Only accept activity between complete greetings, never between bytes.
      if (rx_falling) pause <= 0;
      else pause <= pause - 1;
    end else begin
      frame <= {1'b1, char, 1'b0};
      bits_left <= 10;
      if (index == LENGTH - 1) begin
        index <= 0;
        pause <= PAUSE_CYCLES;
      end else
        index <= index + 1;
    end
  assign ftdi_rxd = frame[0];
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::Folder;

    fn all() -> BTreeSet<Resource> {
        Resource::ALL.into()
    }

    fn initial() -> BTreeSet<Resource> {
        [
            Resource::Clock,
            Resource::Leds,
            Resource::Buttons,
            Resource::Switches,
            Resource::Uart,
        ]
        .into()
    }

    #[test]
    fn default_lpf_preserves_resource_commands_and_board_balls() {
        for resource in Resource::ALL {
            let generated = constraints(&[resource].into());
            let expected = lpf_commands(&generated)
                .map(|line| format!("{line}\n"))
                .collect::<String>();
            let actual = lpf_lines(resource, Naming::Default).unwrap();
            assert_eq!(actual, expected, "{resource:?}");
            assert_eq!(
                actual
                    .lines()
                    .filter(|line| line.starts_with("LOCATE "))
                    .map(|line| line.split('"').nth(3).unwrap())
                    .collect::<Vec<_>>(),
                pins(&[resource].into())
                    .iter()
                    .map(|pin| pin.ball)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn resource_naming_and_renames_preserve_signal_suffixes_and_settings() {
        for (resource, expected) in [
            (
                Resource::Clock,
                PortNaming::Single {
                    default: "clk_25mhz",
                },
            ),
            (Resource::Leds, PortNaming::Single { default: "led" }),
            (Resource::Buttons, PortNaming::Single { default: "btn" }),
            (Resource::Switches, PortNaming::Single { default: "sw" }),
            (Resource::Hdmi, PortNaming::Single { default: "gpdi_dp" }),
            (
                Resource::Uart,
                PortNaming::Prefix {
                    common: Some("ftdi_"),
                },
            ),
            (
                Resource::Oled,
                PortNaming::Prefix {
                    common: Some("oled_"),
                },
            ),
            (
                Resource::Sd,
                PortNaming::Prefix {
                    common: Some("sd_"),
                },
            ),
            (
                Resource::Sdram,
                PortNaming::Prefix {
                    common: Some("sdram_"),
                },
            ),
            (
                Resource::Audio,
                PortNaming::Prefix {
                    common: Some("audio_"),
                },
            ),
            (
                Resource::Adc,
                PortNaming::Prefix {
                    common: Some("adc_"),
                },
            ),
            (
                Resource::Rtc,
                PortNaming::Prefix {
                    common: Some("rtc_"),
                },
            ),
            (
                Resource::Flash,
                PortNaming::Prefix {
                    common: Some("flash_"),
                },
            ),
            (Resource::Header, PortNaming::Prefix { common: None }),
        ] {
            assert_eq!(port_naming(resource), expected, "{resource:?}");
            let old = match expected {
                PortNaming::Single { default } => default,
                PortNaming::Prefix {
                    common: Some(prefix),
                } => prefix,
                PortNaming::Prefix { common: None } => continue,
            };
            // Include names overlapping existing ports, and legal $ characters.
            for new in ["chosen", "disp_", "audio_r", "sd_d", "_pin9$"] {
                let naming = match expected {
                    PortNaming::Single { .. } => Naming::Base(new),
                    PortNaming::Prefix { .. } => Naming::Prefix(new),
                };
                let generated = constraints(&[resource].into());
                let original: Vec<_> = lpf_commands(&generated).collect();
                let renamed = lpf_lines(resource, naming).unwrap();
                let actual: Vec<_> = renamed.lines().collect();
                assert_eq!(actual.len(), original.len());
                for (before, after) in original.iter().zip(actual) {
                    let mut before = before.split('"');
                    let mut after = after.split('"');
                    assert_eq!(before.next(), after.next());
                    let suffix = before.next().unwrap().strip_prefix(old).unwrap();
                    assert_eq!(after.next().unwrap(), format!("{new}{suffix}"));
                    assert_eq!(before.collect::<Vec<_>>(), after.collect::<Vec<_>>());
                }
            }
        }
    }

    #[test]
    fn invalid_naming_combinations_and_identifiers_explain_the_error() {
        for resource in Resource::ALL {
            let (naming, reason) = match port_naming(resource) {
                PortNaming::Single { .. } => {
                    (Naming::Prefix("chosen_"), "requires multiple signals")
                }
                PortNaming::Prefix { .. } => {
                    (Naming::Base("chosen"), "requires a single signal or bus")
                }
            };
            assert!(lpf_lines(resource, naming)
                .unwrap_err()
                .to_string()
                .contains(reason));
        }
        assert!(lpf_lines(Resource::Header, Naming::Prefix("chosen_"))
            .unwrap_err()
            .to_string()
            .contains("no common prefix"));
        for invalid in [
            "",
            "led[0]",
            "bad name",
            "\";",
            "1led",
            "x\ny",
            "a-b",
            "$pin",
            "é",
            "\\escaped",
        ] {
            for resource in Resource::ALL {
                let naming = match port_naming(resource) {
                    PortNaming::Single { .. } => Naming::Base(invalid),
                    PortNaming::Prefix { common: Some(_) } => Naming::Prefix(invalid),
                    PortNaming::Prefix { common: None } => continue,
                };
                assert!(
                    lpf_lines(resource, naming)
                        .unwrap_err()
                        .to_string()
                        .contains("invalid Verilog identifier"),
                    "{resource:?}: {invalid:?}"
                );
            }
        }
    }

    #[test]
    fn all_resource_combinations_and_dependencies() {
        let folder = Folder::new();
        let resources: Vec<_> = initial().into_iter().collect();
        let mut legal = 0;
        for mask in 0..32 {
            let mut options = Options::new(folder.0.join(format!("combination {mask}")));
            options.resources = resources
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, r)| *r)
                .collect();
            let has = |r| options.resources.contains(&r);
            let valid = has(Resource::Clock)
                && (has(Resource::Leds) || has(Resource::Uart))
                && (!(has(Resource::Buttons) || has(Resource::Switches)) || has(Resource::Leds));
            assert_eq!(options.validate().is_ok(), valid, "{mask}");
            if valid {
                legal += 1;
                let project = create(&options).unwrap();
                assert_eq!(project.name, options.name);
                assert_eq!(fs::read_dir(&project.root).unwrap().count(), 3);
            } else {
                assert!(create(&options).is_err());
                assert!(!options.dir.exists());
            }
        }
        assert_eq!(legal, 9);
        let mut options = Options::new(folder.0.join("missing dependencies"));
        options.resources = [Resource::Buttons, Resource::Switches, Resource::Uart].into();
        let error = options.validate().unwrap_err().to_string();
        for reason in [
            "clock is required",
            "UART requires clock",
            "buttons require LEDs",
            "switches require LEDs",
        ] {
            assert!(error.contains(reason), "{error}");
        }
        assert_eq!(options.resources.len(), 3);
        assert!(Resource::parse("unknown").is_err());
    }

    #[test]
    fn names_paths_and_toml_escaping() {
        let folder = Folder::new();
        let mut options = Options::new(
            folder
                .0
                .join("parent with spaces")
                .join("a 'quoted' \"folder\""),
        );
        options.name = "My \"counter\" \\ demo\n\t\r\u{1} café".into();
        let project = create(&options).unwrap();
        assert_eq!(project.name, options.name);
        assert_eq!(project.top, "top");
        assert_eq!(project.verilog, [PathBuf::from("top.v")]);
        assert_eq!(project.constraints, PathBuf::from("top.lpf"));
        assert_eq!(project.device.part, "LFE5U-85F");
        assert_eq!(project.device.package, board::PACKAGE);
        assert!(project.device.speed.is_none());
        let trailing = folder.0.join("trailing separator");
        let options = Options::new(format!("{}/", trailing.display()));
        assert_eq!(create(&options).unwrap().name, "trailing separator");
    }

    #[test]
    fn rejects_invalid_fields_and_occupied_destinations() {
        let folder = Folder::new();
        for leaf in ["", ".", "..", "/", "  ", "bad\0leaf"] {
            let mut options = Options::new(leaf);
            options.name = "valid display name".into();
            assert!(options.validate().is_err(), "{leaf:?}");
        }
        for suffix in ["/.", "/.."] {
            let mut options = Options::new(format!("{}{suffix}", folder.0.display()));
            options.name = "valid".into();
            assert!(options.validate().is_err());
        }
        let mut options = Options::new(folder.0.join("new"));
        options.name = " \t\n".into();
        assert!(create(&options).is_err());
        assert!(!options.dir.exists());
        options.name = "valid".into();
        fs::write(&options.dir, "keep this file").unwrap();
        assert!(create(&options).is_err());
        assert_eq!(fs::read_to_string(&options.dir).unwrap(), "keep this file");
        fs::remove_file(&options.dir).unwrap();
        fs::create_dir(&options.dir).unwrap();
        fs::write(options.dir.join("user file"), "keep").unwrap();
        assert!(create(&options).is_err());
        assert_eq!(
            fs::read_to_string(options.dir.join("user file")).unwrap(),
            "keep"
        );
        fs::remove_file(options.dir.join("user file")).unwrap();
        assert!(
            create(&options).is_ok(),
            "an existing empty directory is allowed"
        );
    }

    #[test]
    fn publication_rechecks_destination_and_cleans_only_its_staging_folder() {
        let folder = Folder::new();
        let unrelated = folder.0.join(".ecp5-new-unrelated");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("keep"), "other operation").unwrap();
        for as_file in [false, true] {
            let destination = folder.0.join(format!("racing destination {as_file}"));
            let options = Options::new(&destination);
            options.validate().unwrap();
            let stage = Staging::new(&folder.0).unwrap();
            let temporary = stage.path.clone();
            for (name, text) in files(&options) {
                fs::write(stage.path.join(name), text).unwrap();
            }
            Project::open(&stage.path).unwrap();
            if as_file {
                fs::write(&destination, "user data").unwrap();
            } else {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("user data"), "keep").unwrap();
            }
            assert!(stage.publish(&destination).is_err());
            assert!(!temporary.exists());
            assert_eq!(
                fs::read_to_string(if as_file {
                    destination
                } else {
                    destination.join("user data")
                })
                .unwrap(),
                if as_file { "user data" } else { "keep" }
            );
        }
        let partial = Staging::new(&folder.0).unwrap();
        let temporary = partial.path.clone();
        fs::write(partial.path.join("partial"), "incomplete").unwrap();
        drop(partial);
        assert!(!temporary.exists());
        assert_eq!(
            fs::read_to_string(unrelated.join("keep")).unwrap(),
            "other operation"
        );
        assert_eq!(
            fs::read_to_string(folder.0.join("top.v")).unwrap(),
            "module top; endmodule\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn destination_symlinks_are_not_followed() {
        let folder = Folder::new();
        let target = folder.0.join("empty target");
        fs::create_dir(&target).unwrap();
        let link = folder.0.join("destination link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(create(&Options::new(&link)).is_err());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_dir(&target).unwrap();
        assert!(
            create(&Options::new(&link)).is_err(),
            "broken symlinks also remain untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_folder_with_an_explicit_display_name() {
        use std::os::unix::ffi::OsStringExt;
        let folder = Folder::new();
        let mut options = Options::new(
            folder
                .0
                .join(std::ffi::OsString::from_vec(b"project \xff".to_vec())),
        );
        match fs::create_dir(&options.dir) {
            Ok(()) => fs::remove_dir(&options.dir).unwrap(),
            // EILSEQ is 84 on Linux and 92 on macOS. Some filesystems report
            // an invalid filename or argument instead of an encoding error.
            Err(error)
                if matches!(error.raw_os_error(), Some(84 | 92))
                    || matches!(
                        error.kind(),
                        ErrorKind::InvalidFilename | ErrorKind::InvalidInput
                    ) =>
            {
                eprintln!(
                    "skipping non-UTF-8 folder test: filesystem refuses such names ({error})"
                );
                return;
            }
            Err(error) => panic!("probing non-UTF-8 folder support: {error}"),
        }
        options.name = "My counter".into();
        assert_eq!(create(&options).unwrap().name, options.name);
    }

    #[test]
    fn generation_is_deterministic_and_uses_board_sites_and_reference_settings() {
        let folder = Folder::new();
        let mut options = Options::new(folder.0.join("first"));
        options.name = "same display name".into();
        options.resources = all();
        let expected = files(&options);
        assert_eq!(expected, files(&options));
        create(&options).unwrap();
        options.dir = folder.0.join("elsewhere");
        create(&options).unwrap();
        for (name, text) in &expected {
            assert_eq!(
                fs::read_to_string(folder.0.join("first").join(name)).unwrap(),
                *text
            );
            assert_eq!(fs::read_to_string(options.dir.join(name)).unwrap(), *text);
        }
        let lpf = &expected[2].1;
        assert!(lpf.starts_with(LPF_HEADER));
        for group in [
            "25 MHz oscillator",
            "LEDs D0..D7",
            "Buttons PWR, FIRE1, FIRE2, UP, DOWN, LEFT, RIGHT",
            "DIP switches",
            "FTDI UART",
        ] {
            assert_eq!(lpf.matches(&format!("\n# {group}\n")).count(), 1);
        }
        assert_eq!(
            lpf_commands("# comment\n  # indented comment\n\n  FREQUENCY PORT \"clk\" 25 MHZ;\n")
                .collect::<Vec<_>>(),
            ["FREQUENCY PORT \"clk\" 25 MHZ;"]
        );
        assert!(BOARD_LPF.starts_with(LPF_HEADER));
        assert!(lpf.contains("# ULX3S constraints: assets/ulx3s/board.lpf."));
        assert!(!lpf.contains("examples/"));
        let all_pins = pins(&all());
        assert_eq!(
            lpf_commands(lpf)
                .filter(|l| l.starts_with("LOCATE "))
                .count(),
            all_pins.len()
        );
        for pin in all_pins {
            assert!(lpf.contains(&format!(
                "LOCATE COMP \"{}\" SITE \"{}\";",
                pin.port, pin.ball
            )));
            let prefix = format!("IOBUF PORT \"{}\" ", pin.port);
            let line = lpf_commands(BOARD_LPF)
                .find(|line| line.starts_with(&prefix))
                .unwrap();
            assert!(lpf.lines().any(|generated| generated == line), "{line}");
            assert!(lpf.contains(&format!("# {}", pin.provenance)));
        }
        for line in include_str!("../../assets/ulx3s/LICENSE.md").lines() {
            assert!(lpf.contains(&format!("# {line}\n")));
        }
        assert_eq!(expected[1].1.matches("assign led =").count(), 1);
    }

    #[test]
    fn every_resource_combination_preserves_demo_backed_constraints() {
        // Snapshot of the demo-backed generator before the board.lpf migration;
        // buttons include all seven pre-P9 constraints from dev's debounce LPF.
        let before = [
            (
                Resource::Clock,
                r#"LOCATE COMP "clk_25mhz" SITE "G2";
IOBUF PORT "clk_25mhz" PULLMODE=NONE IO_TYPE=LVCMOS33;
FREQUENCY PORT "clk_25mhz" 25 MHZ;
"#,
            ),
            (
                Resource::Leds,
                r#"LOCATE COMP "led[0]" SITE "B2";
IOBUF PORT "led[0]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[1]" SITE "C2";
IOBUF PORT "led[1]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[2]" SITE "C1";
IOBUF PORT "led[2]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[3]" SITE "D2";
IOBUF PORT "led[3]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[4]" SITE "D1";
IOBUF PORT "led[4]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[5]" SITE "E2";
IOBUF PORT "led[5]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[6]" SITE "E1";
IOBUF PORT "led[6]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "led[7]" SITE "H3";
IOBUF PORT "led[7]" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;
"#,
            ),
            (
                Resource::Buttons,
                r#"LOCATE COMP "btn[0]" SITE "D6";
IOBUF PORT "btn[0]" PULLMODE=UP IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[1]" SITE "R1";
IOBUF PORT "btn[1]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[2]" SITE "T1";
IOBUF PORT "btn[2]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[3]" SITE "R18";
IOBUF PORT "btn[3]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[4]" SITE "V1";
IOBUF PORT "btn[4]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[5]" SITE "U1";
IOBUF PORT "btn[5]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "btn[6]" SITE "H16";
IOBUF PORT "btn[6]" PULLMODE=DOWN IO_TYPE=LVCMOS33 DRIVE=4;
"#,
            ),
            (
                Resource::Switches,
                r#"LOCATE COMP "sw[0]" SITE "E8";
IOBUF PORT "sw[0]" PULLMODE=DOWN IO_TYPE=LVCMOS33;
LOCATE COMP "sw[1]" SITE "D8";
IOBUF PORT "sw[1]" PULLMODE=DOWN IO_TYPE=LVCMOS33;
LOCATE COMP "sw[2]" SITE "D7";
IOBUF PORT "sw[2]" PULLMODE=DOWN IO_TYPE=LVCMOS33;
LOCATE COMP "sw[3]" SITE "E7";
IOBUF PORT "sw[3]" PULLMODE=DOWN IO_TYPE=LVCMOS33;
"#,
            ),
            (
                Resource::Uart,
                r#"LOCATE COMP "ftdi_rxd" SITE "L4";
IOBUF PORT "ftdi_rxd" PULLMODE=UP IO_TYPE=LVCMOS33 DRIVE=4;
LOCATE COMP "ftdi_txd" SITE "M1";
IOBUF PORT "ftdi_txd" PULLMODE=UP IO_TYPE=LVCMOS33;
"#,
            ),
        ];
        for mask in 0..32 {
            let resources: BTreeSet<_> = before
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, (resource, _))| *resource)
                .collect();
            let expected: Vec<_> = before
                .iter()
                .filter(|(resource, _)| resources.contains(resource))
                .flat_map(|(_, commands)| commands.lines())
                .collect();
            let generated = constraints(&resources);
            let actual: Vec<_> = generated
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
                .collect();
            assert_eq!(actual, expected, "resource mask {mask:05b}");
        }
    }

    #[test]
    fn board_reference_balls_match_board_constants() {
        let mut sites = vec![
            ("clk_25mhz".to_string(), board::CLOCK),
            ("ftdi_rxd".into(), board::UART_TX),
            ("ftdi_txd".into(), board::UART_RX),
        ];
        sites.extend(
            board::LEDS
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("led[{i}]"), *b)),
        );
        sites.extend(
            board::BUTTONS
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("btn[{i}]"), b.ball)),
        );
        sites.extend(
            board::SWITCHES
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("sw[{i}]"), *b)),
        );
        sites.extend([
            ("oled_clk".into(), board::OLED_CLK),
            ("oled_mosi".into(), board::OLED_MOSI),
            ("oled_dc".into(), board::OLED_DC),
            ("oled_resn".into(), board::OLED_RESN),
            ("oled_csn".into(), board::OLED_CSN),
            ("sd_clk".into(), board::SD_CLK),
            ("sd_cmd".into(), board::SD_CMD),
            ("sd_d[0]".into(), board::SD_D[0]),
            ("sd_d[3]".into(), board::SD_D[3]),
            ("sdram_clk".into(), board::SDRAM_CLK),
            ("sdram_cke".into(), board::SDRAM_CKE),
            ("sdram_csn".into(), board::SDRAM_CSN),
            ("sdram_wen".into(), board::SDRAM_WEN),
            ("sdram_rasn".into(), board::SDRAM_RASN),
            ("sdram_casn".into(), board::SDRAM_CASN),
            ("adc_csn".into(), board::ADC_CSN),
            ("adc_sclk".into(), board::ADC_SCLK),
            ("adc_mosi".into(), board::ADC_MOSI),
            ("adc_miso".into(), board::ADC_MISO),
            ("rtc_scl".into(), board::RTC_SCL),
            ("rtc_sda".into(), board::RTC_SDA),
            ("flash_csn".into(), board::FLASH_CSN),
            ("flash_mosi".into(), board::FLASH_MOSI),
            ("flash_miso".into(), board::FLASH_MISO),
            ("flash_wpn".into(), board::FLASH_WPN),
            ("flash_holdn".into(), board::FLASH_HOLDN),
        ]);
        for (base, balls) in [
            ("gpdi_dp", board::GPDI_DP.as_slice()),
            ("audio_l", board::AUDIO_L.as_slice()),
            ("audio_r", board::AUDIO_R.as_slice()),
            ("audio_v", board::AUDIO_V.as_slice()),
            ("sdram_ba", board::SDRAM_BA.as_slice()),
            ("sdram_dqm", board::SDRAM_DQM.as_slice()),
            ("sdram_a", board::SDRAM_A.as_slice()),
            ("sdram_d", board::SDRAM_D.as_slice()),
            ("adc_ain", board::ADC_AIN.as_slice()),
        ] {
            sites.extend(
                balls
                    .iter()
                    .enumerate()
                    .map(|(i, ball)| (format!("{base}[{i}]"), *ball)),
            );
        }
        sites.extend(board::HEADER.iter().map(|pin| {
            (
                format!("{}[{}]", pin.name[..2].to_lowercase(), &pin.name[2..]),
                pin.ball,
            )
        }));
        let expected: std::collections::BTreeMap<_, _> = sites.into_iter().collect();
        let mut actual = std::collections::BTreeMap::new();
        for line in lpf_commands(BOARD_LPF).filter(|line| line.starts_with("LOCATE ")) {
            let (port, ball) = line
                .strip_prefix("LOCATE COMP \"")
                .unwrap()
                .split_once("\" SITE \"")
                .unwrap();
            let ball = ball.strip_suffix("\";").unwrap();
            assert!(
                actual.insert(port.to_string(), ball).is_none(),
                "duplicate {port}"
            );
        }
        assert_eq!(actual, expected);
    }
    #[test]
    fn every_peripheral_uses_verbatim_reference_commands() {
        for (resource, source) in [
            (
                Resource::Oled,
                include_str!("../../examples/ulx3s/oled_pattern/top.lpf"),
            ),
            (
                Resource::Hdmi,
                include_str!("../../examples/ulx3s/hdmi_bars/top.lpf"),
            ),
            (
                Resource::Sd,
                include_str!("../../examples/ulx3s/sd_read/top.lpf"),
            ),
            (
                Resource::Audio,
                include_str!("../../examples/ulx3s/audio_tone/top.lpf"),
            ),
            (
                Resource::Sdram,
                include_str!("../../examples/ulx3s/sdram_test/top.lpf"),
            ),
            (
                Resource::Adc,
                include_str!("../../examples/ulx3s/adc_read/top.lpf"),
            ),
            (
                Resource::Rtc,
                include_str!("../../examples/ulx3s/rtc_read/top.lpf"),
            ),
            (
                Resource::Flash,
                include_str!("../../examples/ulx3s/flash_dump/top.lpf"),
            ),
            (
                Resource::Header,
                include_str!("../../examples/ulx3s/header_probe/top.lpf"),
            ),
        ] {
            let generated = constraints(&[resource].into());
            let actual: Vec<_> = lpf_commands(&generated).collect();
            let expected: Vec<_> = lpf_commands(source)
                .filter(|line| {
                    let port = line.split('"').nth(1).unwrap();
                    resource
                        .port_bases()
                        .contains(&port.split('[').next().unwrap())
                })
                .collect();
            assert_eq!(actual, expected, "{resource:?}");
            for line in actual {
                assert!(
                    BOARD_LPF.lines().any(|reference| reference == line),
                    "{line}"
                );
            }
        }
        assert!(!BOARD_LPF.contains("SITE \"U3\""));
        let flash = verilog(&[Resource::Clock, Resource::Flash].into());
        assert!(flash.contains("USRMCLK clock_pin(.USRMCLKI(sck), .USRMCLKTS(1'b0));"));
        assert!(flash.contains("wire sck = 1'b0;"));
    }

    #[test]
    fn dependencies_are_exposed_and_explained_without_adding_selections() {
        let folder = Folder::new();
        for dependency in DEPENDENCIES {
            let mut options = Options::new(folder.0.join(format!("{:?}", dependency.resource)));
            options.resources = [dependency.resource].into();
            assert!(options
                .validate()
                .unwrap_err()
                .to_string()
                .contains(dependency.reason));
            assert_eq!(options.resources, [dependency.resource].into());
        }
        for resource in all().difference(&initial()) {
            let mut options = Options::new(folder.0.join(format!("alone {resource:?}")));
            options.resources = [Resource::Clock, *resource].into();
            assert!(options.validate().is_ok(), "{resource:?}");
            create(&options).unwrap();
        }
    }

    #[test]
    fn generated_header_is_inputs_only_with_reviewed_pulls_and_shared_adc_inputs() {
        for resources in [[Resource::Clock, Resource::Header].into(), all()] {
            let top = verilog(&resources);
            assert!(top.contains("(* keep *) input [27:0] gp"));
            assert!(top.contains("(* keep *) input [27:0] gn"));
            let block = top
                .split("// See examples/ulx3s/header_probe.")
                .nth(1)
                .unwrap();
            assert!(block.contains("inputs only, never driven by default"));
            assert!(!block.contains("assign") && !block.contains("inout"));
            assert_eq!(
                block
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.trim().starts_with("//"))
                    .collect::<Vec<_>>(),
                [
                    "  (* keep *) wire [27:0] header_gp = gp;",
                    "  (* keep *) wire [27:0] header_gn = gn;",
                    "endmodule",
                ]
            );
            let lpf = constraints(&resources);
            for pin in board::HEADER {
                let port = format!("{}[{}]", pin.name[..2].to_lowercase(), &pin.name[2..]);
                let pull = if pin.name.starts_with("GN") {
                    "UP"
                } else {
                    "DOWN"
                };
                assert!(lpf.contains(&format!(
                    "IOBUF PORT \"{port}\" PULLMODE={pull} IO_TYPE=LVCMOS33;"
                )));
            }
            let balls: BTreeSet<_> = pins(&resources).iter().map(|p| p.ball).collect();
            assert_eq!(balls.len(), pins(&resources).len());
            if resources.contains(&Resource::Adc) {
                assert!(!top.contains("input [7:0] adc_ain"));
                assert!(top.contains("wire [7:0] adc_ain = {gp[17], gn[17], gp[16], gn[16], gp[15], gn[15], gp[14], gn[14]}"));
            }
        }
    }
}
