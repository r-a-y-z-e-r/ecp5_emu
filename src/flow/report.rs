//! Resource and timing reports parsed from Yosys and nextpnr logs.
use super::project::SvFrontend;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::PathBuf};

pub const VHDL_SOURCE_LIMIT: &str =
    "Source probing is not available for VHDL yet: the GHDL plugin writes no source locations";

/// The actual synthesis reader, separate from the manifest's SV preference.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FrontendKind {
    Builtin,
    Slang,
    Ghdl,
}
impl fmt::Display for FrontendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Builtin => "builtin",
            Self::Slang => "slang",
            Self::Ghdl => "ghdl",
        })
    }
}
/// Plugin bytes shipped by the selected suite; path is relative to its root.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PluginIdentity {
    pub path: PathBuf,
    pub sha256: String,
}

/// Frontend and default Yosys used to produce the synthesis artifact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Frontend {
    pub requested: SvFrontend,
    pub resolved: FrontendKind,
    pub reason: String,
    pub yosys_path: PathBuf,
    pub yosys_version: String,
    #[serde(default)]
    pub suite_root: Option<PathBuf>,
    #[serde(default)]
    pub suite_source: Option<String>,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub plugin_identity: Option<PluginIdentity>,
    #[serde(default)]
    pub vhdl_standard: Option<String>,
    #[serde(default)]
    pub ghdl_prefix: Option<PathBuf>,
}
impl Frontend {
    /// Selection and executable provenance for the CLI report and build log.
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "{} -> {}: {}; yosys {} ({})",
            self.requested,
            self.resolved,
            self.reason,
            self.yosys_path.display(),
            self.yosys_version
        );
        if let (Some(root), Some(source), Some(plugin), Some(identity)) = (
            &self.suite_root,
            &self.suite_source,
            &self.plugin,
            &self.plugin_identity,
        ) {
            summary.push_str(&format!(
                "; suite {} ({source}); {plugin} {} sha256:{}",
                root.display(),
                identity.path.display(),
                identity.sha256.chars().take(12).collect::<String>()
            ));
        }
        if let Some(standard) = &self.vhdl_standard {
            summary.push_str(&format!("; VHDL-{standard}"));
        }
        summary
    }
}

/// One nextpnr device resource row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Resource {
    pub cell: String,
    pub label: String,
    pub used: u64,
    pub available: u64,
}
/// Final achieved frequency for a clock, with its original and friendly names.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clock {
    pub name: String,
    pub net: String,
    pub achieved_mhz: f64,
    pub target_mhz: Option<f64>,
    pub pass: bool,
}
/// A reusable build report, also written to report.json.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    #[serde(default)]
    pub frontend: Option<Frontend>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub utilisation: Vec<Resource>,
    pub clocks: Vec<Clock>,
    pub max_delays: Vec<String>,
    pub synthesis_cells: Vec<(String, u64)>,
}
fn label(cell: &str) -> &str {
    match cell {
        "TRELLIS_COMB" => "LUT4",
        "TRELLIS_FF" => "Flip-flops",
        "DP16KD" => "Block RAM (EBR)",
        "MULT18X18D" => "DSP multipliers",
        "ALU54B" => "DSP ALUs",
        "TRELLIS_IO" => "IO pins",
        "EHXPLLL" => "PLLs",
        "TRELLIS_RAMW" => "Distributed RAM (RAMW)",
        "DCCA" => "Global clock buffers",
        _ => cell,
    }
}
/// Removes nextpnr's global-net wrapper while preserving the HDL signal name.
pub fn friendly_clock(name: &str) -> String {
    let name = name.strip_prefix("$glbnet$").unwrap_or(name);
    name.rsplit_once('$')
        .filter(|(_, suffix)| suffix.starts_with("TRELLIS_"))
        .map_or(name, |(net, _)| net)
        .into()
}
fn clock(line: &str) -> Option<Clock> {
    let rest = line.split_once("Max frequency for clock '")?.1;
    let (name, rest) = rest.split_once("': ")?;
    let achieved_mhz = rest.split_whitespace().next()?.parse().ok()?;
    let target_mhz = rest
        .split_once(" at ")
        .and_then(|(_, s)| s.split_whitespace().next()?.parse().ok());
    Some(Clock {
        name: name.into(),
        net: friendly_clock(name),
        achieved_mhz,
        target_mhz,
        pass: !rest.contains("FAIL"),
    })
}
fn resource(line: &str) -> Option<Resource> {
    let (cell, values) = line.split_once(':')?;
    let (used, rest) = values.trim().split_once('/')?;
    let available = rest.split_whitespace().next()?.parse().ok()?;
    let cell = cell.trim();
    Some(Resource {
        cell: cell.into(),
        label: label(cell).into(),
        used: used.trim().parse().ok()?,
        available,
    })
}
impl Report {
    /// Takes the last utilisation block and final timing result for each clock.
    pub fn parse(yosys: &str, nextpnr: &str, top: &str) -> Self {
        let mut report = Self::default();
        let mut in_utilisation = false;
        let mut clocks = BTreeMap::new();
        for line in nextpnr.lines() {
            let text = line
                .trim()
                .strip_prefix("Info:")
                .unwrap_or(line.trim())
                .trim();
            if text == "Device utilisation:" {
                report.utilisation.clear();
                in_utilisation = true;
                continue;
            }
            if in_utilisation {
                if let Some(row) = resource(text) {
                    report.utilisation.push(row);
                } else {
                    in_utilisation = false;
                }
            }
            if let Some(clock) = clock(line) {
                clocks.insert(clock.name.clone(), clock);
            }
            if text.starts_with("Max delay ") && !report.max_delays.iter().any(|s| s == text) {
                report.max_delays.push(text.into());
            }
        }
        report.clocks = clocks.into_values().collect();
        let header = format!("=== {top} ===");
        let mut in_top = false;
        for line in yosys.lines() {
            let text = line.trim();
            if text.starts_with("=== ") {
                in_top = text == header;
                if in_top {
                    report.synthesis_cells.clear();
                }
                continue;
            }
            if in_top {
                // Statistics rows have a count and an uppercase primitive name.
                let fields: Vec<_> = text.split_whitespace().collect();
                if fields.len() == 2
                    && fields[1]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase() || c == '$')
                {
                    if let Ok(count) = fields[0].parse() {
                        report.synthesis_cells.push((fields[1].into(), count));
                    }
                }
                if text.starts_with("End of script") || text.starts_with("JSON backend") {
                    in_top = false;
                }
            }
        }
        report
    }
    /// A compact device utilisation line, in the usual FPGA resource order.
    pub fn summary(&self) -> String {
        let get = |cell: &str| self.utilisation.iter().find(|r| r.cell == cell);
        let ratio = |cell: &str, percent: bool| {
            get(cell)
                .map(|r| {
                    let base = format!("{}/{}", r.used, r.available);
                    if percent && r.available != 0 {
                        format!("{base} ({}%)", r.used * 100 / r.available)
                    } else {
                        base
                    }
                })
                .unwrap_or_else(|| "n/a".into())
        };
        format!(
            "LUT4 {}, FF {}, EBR {}, DSP {} mult + {} ALU, IO {}, PLL {}",
            ratio("TRELLIS_COMB", true),
            ratio("TRELLIS_FF", true),
            ratio("DP16KD", false),
            ratio("MULT18X18D", false),
            ratio("ALU54B", false),
            ratio("TRELLIS_IO", false),
            ratio("EHXPLLL", false)
        )
    }
    /// One display line per clock, using the final timing result.
    pub fn timing_lines(&self) -> Vec<String> {
        self.clocks
            .iter()
            .map(|c| {
                let pass = if c.pass { "PASS" } else { "FAIL" };
                match c.target_mhz {
                    Some(target) => format!(
                        "{}: {:.2} MHz (target {:.2} MHz, {pass})",
                        c.net, c.achieved_mhz, target
                    ),
                    None => format!("{}: {:.2} MHz ({pass})", c.net, c.achieved_mhz),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::test_support::{CLOCK, UTIL, YOSYS};
    #[test]
    fn real_reports_and_last_results_win() {
        let yosys = format!(
            "=== top ===\n 1 OLD\n=== child ===\n 2 CHILD\n{YOSYS}\n=== child ===\n 3 OTHER\n"
        );
        let nextpnr = format!("{}\n{UTIL}{CLOCK}", UTIL.replace("10859", "1"));
        let r = Report::parse(&yosys, &nextpnr, "top");
        assert_eq!(r.utilisation.len(), 10);
        assert_eq!(
            r.utilisation
                .iter()
                .find(|r| r.cell == "TRELLIS_COMB")
                .unwrap()
                .label,
            "LUT4"
        );
        assert_eq!(
            r.synthesis_cells,
            [
                ("CCU2C".into(), 5),
                ("L6MUX21".into(), 788),
                ("LUT4".into(), 10843),
                ("PFUMX".into(), 3553),
                ("TRELLIS_FF".into(), 775)
            ]
        );
        assert_eq!(r.summary(), "LUT4 10859/83640 (12%), FF 775/83640 (0%), EBR 0/208, DSP 0/156 mult + 0/78 ALU, IO 16/365, PLL 0/4");
        assert_eq!(r.clocks[0].achieved_mhz, 354.99);
        assert_eq!(r.clocks[0].net, "clk_25mhz");
        assert_eq!(
            r.timing_lines(),
            ["clk_25mhz: 354.99 MHz (target 25.00 MHz, PASS)"]
        );
        assert_eq!(r.max_delays, ["Max delay <async>                                 -> posedge $glbnet$clk_25mhz$TRELLIS_IO_IN: 0.82 ns"]);
        assert_eq!(
            serde_json::from_str::<Report>(&serde_json::to_string(&r).unwrap()).unwrap(),
            r
        );
        let failing = "Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 448.83 MHz (FAIL at 900.09 MHz)\nERROR: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (FAIL at 900.09 MHz)";
        let r = Report::parse(YOSYS, failing, "top");
        assert_eq!(r.clocks[0].achieved_mhz, 354.99);
        assert!(!r.clocks[0].pass);
        assert_eq!(r.clocks[0].target_mhz, Some(900.09));
        assert_eq!(friendly_clock("plain"), "plain");
    }
}
