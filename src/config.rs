use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeMap, path::Path, process::Command};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileConfig {
    /// Trellis writes arcs in sink, source order.
    pub arcs: BTreeMap<String, String>,
    pub words: BTreeMap<String, String>,
    /// The first value of each enum.
    pub enums: BTreeMap<String, String>,
    /// Later writes of an enum with another value, as (name, value). nextpnr
    /// writes a CIB constant mux once for each cell pin on the wire, and
    /// ecppack sets the bits of every value written (HD08).
    pub rewritten_enums: Vec<(String, String)>,
    pub unknown: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub device: String,
    pub variant: Option<String>,
    pub comments: Vec<String>,
    pub sysconfig: BTreeMap<String, String>,
    pub tiles: BTreeMap<String, TileConfig>,
    pub bram: BTreeMap<u16, Vec<u16>>,
    pub groups: Vec<(Vec<String>, TileConfig)>,
}

fn insert(map: &mut BTreeMap<String, String>, key: &str, value: &str) -> Result<()> {
    if let Some(old) = map.insert(key.into(), value.into()) {
        ensure!(
            old == value,
            "conflicting values for {key}: {old} and {value}"
        );
    }
    Ok(())
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let mut result = Self::default();
        let mut tile: Option<String> = None;
        let mut group: Option<usize> = None;
        let mut bram: Option<u16> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<_> = line.split_whitespace().collect();
            let parse_line = || -> Result<()> {
                if line.starts_with('.') {
                    tile = None;
                    group = None;
                    bram = None;
                    match parts[0] {
                        ".device" => {
                            ensure!(
                                parts.len() == 2 && result.device.is_empty(),
                                "expected a single .device declaration"
                            );
                            result.device = parts[1].into();
                        }
                        ".variant" => {
                            ensure!(
                                parts.len() == 2 && result.variant.is_none(),
                                "invalid .variant"
                            );
                            result.variant = Some(parts[1].into());
                        }
                        ".comment" => result.comments.push(line[8..].trim().into()),
                        ".sysconfig" => {
                            ensure!(parts.len() == 3, "expected .sysconfig key value");
                            insert(&mut result.sysconfig, parts[1], parts[2])?;
                        }
                        ".tile" => {
                            ensure!(
                                parts.len() == 2 && parts[1].contains(':'),
                                "expected .tile name:type"
                            );
                            tile = Some(parts[1].into());
                            result.tiles.entry(parts[1].into()).or_default();
                        }
                        ".tile_group" => {
                            ensure!(parts.len() > 1, "empty tile group");
                            group = Some(result.groups.len());
                            result.groups.push((
                                parts[1..].iter().map(|s| (*s).into()).collect(),
                                TileConfig::default(),
                            ));
                        }
                        ".bram_init" => {
                            ensure!(parts.len() == 2, "expected .bram_init block_id");
                            let id = parts[1].parse()?;
                            ensure!(!result.bram.contains_key(&id), "duplicate BRAM block {id}");
                            result.bram.insert(id, Vec::new());
                            bram = Some(id);
                        }
                        other => bail!("unknown directive {other}"),
                    }
                } else if let Some(id) = bram {
                    for value in &parts {
                        let value = u16::from_str_radix(value, 16)?;
                        ensure!(
                            value <= 0x1ff,
                            "ECP5 BRAM initialisation words must be nine bits"
                        );
                        result.bram.get_mut(&id).unwrap().push(value);
                    }
                } else {
                    let target = if let Some(ref name) = tile {
                        result.tiles.get_mut(name).unwrap()
                    } else if let Some(i) = group {
                        &mut result.groups[i].1
                    } else {
                        bail!("configuration entry outside a tile or tile group");
                    };
                    match parts[0] {
                        "arc:" => {
                            ensure!(parts.len() == 3, "expected arc: sink source");
                            insert(&mut target.arcs, parts[1], parts[2])?;
                        }
                        "word:" => {
                            ensure!(
                                parts.len() == 3
                                    && parts[2].bytes().all(|b| b == b'0' || b == b'1'),
                                "expected word: name binary_value"
                            );
                            insert(&mut target.words, parts[1], parts[2])?;
                        }
                        "enum:" => {
                            ensure!(parts.len() == 3, "expected enum: name value");
                            match target.enums.get(parts[1]) {
                                Some(old) if old != parts[2] => target
                                    .rewritten_enums
                                    .push((parts[1].into(), parts[2].into())),
                                Some(_) => {}
                                None => {
                                    target.enums.insert(parts[1].into(), parts[2].into());
                                }
                            }
                        }
                        "unknown:" => {
                            ensure!(parts.len() == 2, "expected unknown: F<frame>B<bit>");
                            let (frame, bit) = parts[1]
                                .strip_prefix('F')
                                .context("missing frame prefix")?
                                .split_once('B')
                                .context("missing bit prefix")?;
                            target.unknown.push((frame.parse()?, bit.parse()?));
                        }
                        other => bail!("unknown tile entry {other}"),
                    }
                }
                Ok(())
            };
            let mut parse_line = parse_line;
            parse_line().with_context(|| format!("config line {}: {line}", index + 1))?;
        }
        ensure!(!result.device.is_empty(), "missing .device declaration");
        Ok(result)
    }

    pub fn load(path: &Path, database: &Path) -> Result<Self> {
        let text = if path.extension().is_some_and(|x| x == "bit") {
            let output =
                Command::new(std::env::var_os("ECP5_PYTHON").unwrap_or_else(|| "python3".into()))
                    .args(["-c", include_str!("../tools/trellis_bridge.py")])
                    .arg(database)
                    .arg(path)
                    .output()
                    .context("cannot run Python/Trellis bitstream decoder")?;
            ensure!(
                output.status.success(),
                "Trellis decoder failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .context("decoder did not return UTF-8 configuration")?
        } else {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        };
        Self::parse(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_device_tile_identity_and_both_luts() {
        let c = Config::parse(".device LFE5U-85F\n.tile CIB_R1C2:CIB\narc: JA0 E1_Q0 # sink then source\n.tile R1C2:PLC2\nword: SLICEA.K0.INIT 0000000000000001\nword: SLICEA.K1.INIT 1000000000000000\n").unwrap();
        assert_eq!(c.tiles.len(), 2);
        assert_eq!(c.tiles["CIB_R1C2:CIB"].arcs["JA0"], "E1_Q0");
        assert_eq!(c.tiles["R1C2:PLC2"].words.len(), 2);
    }
    #[test]
    fn retains_bram_and_groups() {
        let c = Config::parse(".device LFE5U-85F\n.bram_init 4\n000 1ff 112\n.tile_group A:EBR B:EBR\nenum: EBR.MODE DP16KD\n").unwrap();
        assert_eq!(c.bram[&4], [0, 511, 0x112]);
        assert_eq!(c.groups[0].0.len(), 2);
    }
    #[test]
    fn keeps_a_rewritten_enum_as_ecppack_takes_it() {
        let c = Config::parse(
            ".device LFE5U-85F\n.tile CIB_R1C2:CIB_DSP\nenum: CIB.JA1MUX 1\nenum: CIB.JA1MUX 0\n",
        )
        .unwrap();
        let t = &c.tiles["CIB_R1C2:CIB_DSP"];
        assert_eq!(t.enums["CIB.JA1MUX"], "1");
        assert_eq!(t.rewritten_enums, [("CIB.JA1MUX".into(), "0".into())]);
    }
    #[test]
    fn malformed_input_is_an_error() {
        for input in [
            ".tile R1C1:PLC2",
            ".device LFE5U-85F\nword: x 01",
            ".device LFE5U-85F\n.tile R1C1:PLC2\nword: x 02",
            ".device LFE5U-85F\n.tile R1C1:PLC2\narc: a b\narc: a c",
            ".device LFE5U-85F\n.bram_init 3\n200",
        ] {
            assert!(Config::parse(input).is_err(), "{input}");
        }
    }
}
