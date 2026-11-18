use crate::config::{Config, TileConfig};
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Tile {
    pub name: String,
    pub kind: String,
    pub row: i16,
    pub col: i16,
    pub cram: CramWindow,
    pub sites: Vec<Site>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CramWindow {
    pub start_frame: usize,
    pub start_bit: usize,
    pub frames: usize,
    pub bits: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Site {
    pub name: String,
    pub pos_row: String,
    pub pos_col: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitConstraint {
    pub frame: usize,
    pub bit: usize,
    pub value: bool,
}

impl BitConstraint {
    fn parse(token: &str) -> Result<Self> {
        let (value, token) = match token.strip_prefix('!') {
            Some(token) => (false, token),
            None => (true, token),
        };
        let (frame, bit) = token
            .strip_prefix('F')
            .context("missing F in routing bit")?
            .split_once('B')
            .context("missing B in routing bit")?;
        Ok(Self {
            frame: frame.parse()?,
            bit: bit.parse()?,
            value,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArcKind {
    Fixed,
    Mux,
}

/// A directed database description, without assuming that it is active.
#[derive(Clone, Debug)]
pub struct ArcDef {
    pub source: String,
    pub sink: String,
    pub kind: ArcKind,
    pub bits: Vec<BitConstraint>,
    pub source_line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct EnumDef {
    pub default: Option<String>,
    pub options: BTreeSet<String>,
    pub patterns: BTreeMap<String, Vec<BitConstraint>>,
}

#[derive(Clone, Debug, Default)]
pub struct TileType {
    pub routing: Vec<ArcDef>,
    pub incoming: BTreeMap<String, Vec<usize>>,
    pub fixed: Vec<(String, String)>,
    pub muxes: BTreeMap<String, BTreeSet<String>>,
    pub default_arcs: BTreeMap<String, String>,
    pub words: BTreeMap<String, String>,
    pub word_bits: BTreeMap<String, Vec<Vec<BitConstraint>>>,
    pub enums: BTreeMap<String, EnumDef>,
}

impl TileType {
    pub fn parse(text: &str) -> Result<Self> {
        let mut result = Self::default();
        let mut section = ("", "");
        for (line_index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let p: Vec<_> = line.split_whitespace().collect();
            if p.is_empty() {
                section = ("", "");
                continue;
            }
            if p[0].starts_with('.') {
                ensure!(p.len() >= 2, "invalid database header: {line}");
                section = (p[0], p[1]);
                match p[0] {
                    ".fixed_conn" => {
                        ensure!(p.len() == 3, "invalid fixed connection");
                        result.fixed.push((p[1].into(), p[2].into()));
                        result.routing.push(ArcDef {
                            source: p[2].into(),
                            sink: p[1].into(),
                            kind: ArcKind::Fixed,
                            bits: Vec::new(),
                            source_line: line_index + 1,
                        });
                    }
                    ".mux" => {
                        result.muxes.entry(p[1].into()).or_default();
                    }
                    ".config" => {
                        ensure!(p.len() == 3, "configuration word has no default");
                        result.words.insert(p[1].into(), p[2].into());
                        result.word_bits.insert(p[1].into(), Vec::new());
                    }
                    ".config_enum" => {
                        result.enums.insert(
                            p[1].into(),
                            EnumDef {
                                default: p.get(2).map(|s| (*s).into()),
                                options: BTreeSet::new(),
                                patterns: BTreeMap::new(),
                            },
                        );
                    }
                    other => bail!("unsupported database section {other}"),
                }
            } else if section.0 == ".mux" {
                ensure!(p.len() >= 2, "mux source has no bit constraints: {line}");
                let bits = if p[1..] == ["-"] {
                    Vec::new()
                } else {
                    p[1..]
                        .iter()
                        .map(|s| BitConstraint::parse(s))
                        .collect::<Result<Vec<_>>>()?
                };
                result.routing.push(ArcDef {
                    source: p[0].into(),
                    sink: section.1.into(),
                    kind: ArcKind::Mux,
                    bits,
                    source_line: line_index + 1,
                });
                result.muxes.get_mut(section.1).unwrap().insert(p[0].into());
                if p.get(1) == Some(&"-") {
                    ensure!(
                        result
                            .default_arcs
                            .insert(section.1.into(), p[0].into())
                            .is_none(),
                        "multiple default mux sources"
                    );
                }
            } else if section.0 == ".config" {
                let group = if p == ["-"] {
                    Vec::new()
                } else {
                    p.iter()
                        .map(|s| BitConstraint::parse(s))
                        .collect::<Result<Vec<_>>>()?
                };
                result.word_bits.get_mut(section.1).unwrap().push(group);
            } else if section.0 == ".config_enum" {
                ensure!(p.len() >= 2, "enum value has no bit constraints: {line}");
                let bits = if p[1..] == ["-"] {
                    Vec::new()
                } else {
                    p[1..]
                        .iter()
                        .map(|s| BitConstraint::parse(s))
                        .collect::<Result<Vec<_>>>()?
                };
                let def = result.enums.get_mut(section.1).unwrap();
                ensure!(def.options.insert(p[0].into()), "duplicate enum value");
                def.patterns.insert(p[0].into(), bits);
            }
        }
        for (index, arc) in result.routing.iter().enumerate() {
            result
                .incoming
                .entry(arc.sink.clone())
                .or_default()
                .push(index);
        }
        Ok(result)
    }

    pub fn enum_value<'a>(&'a self, config: &'a TileConfig, key: &str) -> Option<&'a str> {
        config
            .enums
            .get(key)
            .filter(|s| s.as_str() != "_NONE_")
            .map(String::as_str)
            .or_else(|| self.enums.get(key)?.default.as_deref())
    }

    pub fn word_u16(&self, config: &TileConfig, key: &str) -> Result<u16> {
        let value = config
            .words
            .get(key)
            .or_else(|| self.words.get(key))
            .with_context(|| format!("unknown word {key}"))?;
        u16::from_str_radix(value, 2).with_context(|| format!("word {key} is not a 16-bit value"))
    }

    pub fn validate(&self, config: &TileConfig) -> Result<()> {
        for (sink, source) in &config.arcs {
            ensure!(
                self.muxes.get(sink).is_some_and(|s| s.contains(source)),
                "unknown arc {source} -> {sink}"
            );
        }
        for (name, value) in &config.words {
            let default = self
                .words
                .get(name)
                .with_context(|| format!("unknown configuration word {name}"))?;
            ensure!(
                default.len() == value.len(),
                "{name}: expected {} bits, got {}",
                default.len(),
                value.len()
            );
        }
        for (name, value) in &config.enums {
            let def = self
                .enums
                .get(name)
                .with_context(|| format!("unknown configuration enum {name}"))?;
            // nextpnr uses _NONE_ to leave this setting's configuration bits alone.
            ensure!(
                value == "_NONE_" || def.options.contains(value),
                "{name}: unknown option {value}"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct Pin {
    pub row: i16,
    pub col: i16,
    pub pio: String,
}

pub struct Database {
    pub root: PathBuf,
    pub device: String,
    pub tiles: BTreeMap<String, Tile>,
    pub types: BTreeMap<String, TileType>,
    pub packages: BTreeMap<String, BTreeMap<String, Pin>>,
    pub globals: serde_json::Value,
    pub max_row: i16,
    pub max_col: i16,
}

pub fn coordinates(name: &str) -> Result<(i16, i16)> {
    let name = name.split(':').next().unwrap_or(name);
    let start = name.rfind('R').context("tile has no row coordinate")?;
    let (row, col) = name[start + 1..]
        .split_once('C')
        .context("tile has no column coordinate")?;
    Ok((row.parse()?, col.parse()?))
}

impl Database {
    pub fn discover(explicit: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = explicit {
            return Ok(path.into());
        }
        if let Some(path) = std::env::var_os("ECP5_DB") {
            return Ok(path.into());
        }
        let mut candidates = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join("FPGA/prjtrellis/database"));
        }
        candidates.extend(
            [
                "/usr/local/share/trellis/database",
                "/usr/local/share/trellis",
                "/opt/homebrew/share/trellis/database",
                "/opt/homebrew/share/trellis",
            ]
            .map(PathBuf::from),
        );
        candidates
            .into_iter()
            .find(|p| p.join("ECP5/LFE5U-85F/tilegrid.json").is_file())
            .context("Trellis database not found; use --db PATH or ECP5_DB")
    }

    pub fn load(root: &Path, device: &str) -> Result<Self> {
        ensure!(
            device == "LFE5U-85F",
            "device {device} is not supported; current target is LFE5U-85F"
        );
        let family = root.join("ECP5");
        let device_dir = family.join(device);
        let grid: BTreeMap<String, serde_json::Value> =
            serde_json::from_slice(&std::fs::read(device_dir.join("tilegrid.json"))?)?;
        let mut types = BTreeMap::new();
        let mut tiles = BTreeMap::new();
        let (mut max_row, mut max_col) = (0, 0);
        for (name, value) in grid {
            let kind = value["type"]
                .as_str()
                .context("tilegrid entry has no type")?
                .to_string();
            let (row, col) = coordinates(&name).with_context(|| name.clone())?;
            max_row = max_row.max(row);
            max_col = max_col.max(col);
            if !types.contains_key(&kind) {
                let path = family.join("tiledata").join(&kind).join("bits.db");
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                types.insert(
                    kind.clone(),
                    TileType::parse(&text)
                        .with_context(|| format!("parsing {}", path.display()))?,
                );
            }
            tiles.insert(
                name.clone(),
                Tile {
                    name,
                    kind,
                    row,
                    col,
                    cram: CramWindow {
                        start_frame: value["start_frame"]
                            .as_u64()
                            .context("missing start_frame")?
                            as usize,
                        start_bit: value["start_bit"].as_u64().context("missing start_bit")?
                            as usize,
                        frames: value["cols"].as_u64().context("missing frame count")? as usize,
                        bits: value["rows"].as_u64().context("missing bit count")? as usize,
                    },
                    sites: serde_json::from_value(value["sites"].clone())
                        .context("reading tile sites")?,
                },
            );
        }
        #[derive(Deserialize)]
        struct IoDb {
            packages: BTreeMap<String, BTreeMap<String, Pin>>,
        }
        let io: IoDb = serde_json::from_slice(&std::fs::read(device_dir.join("iodb.json"))?)?;
        let globals = serde_json::from_slice(&std::fs::read(device_dir.join("globals.json"))?)?;
        ensure!(!tiles.is_empty(), "empty tilegrid");
        Ok(Self {
            root: root.into(),
            device: device.into(),
            tiles,
            types,
            packages: io.packages,
            globals,
            max_row,
            max_col,
        })
    }

    pub fn validate(&self, config: &Config) -> Result<()> {
        ensure!(
            config.device == self.device,
            "configuration/database device mismatch"
        );
        for (name, tc) in &config.tiles {
            let tile = self
                .tiles
                .get(name)
                .with_context(|| format!("unknown tile {name}"))?;
            self.types[&tile.kind]
                .validate(tc)
                .with_context(|| format!("tile {name}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_defaults_and_fixed_arc_order() {
        let db = TileType::parse(".mux F0\nF0_SLICE -\nF5A_SLICE F8B10\n\n.config SLICEA.K0.INIT 1111111111111111\n!F0B0\n\n.config_enum SLICEA.MODE LOGIC\nLOGIC -\nCCU2 F9B10\n\n.fixed_conn A0_SLICE A0\n").unwrap();
        assert_eq!(db.default_arcs["F0"], "F0_SLICE");
        assert_eq!(db.fixed[0], ("A0_SLICE".into(), "A0".into()));
        assert_eq!(
            db.word_u16(&TileConfig::default(), "SLICEA.K0.INIT")
                .unwrap(),
            0xffff
        );
        assert_eq!(coordinates("CIB_R37C1:CIB_LR").unwrap(), (37, 1));
    }
}
