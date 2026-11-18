//! Decode complete resource fields from shared CRAM, independently of simulation.
mod dsp;
mod ebr;
mod pio;
use crate::config::Config;
use anyhow::{bail, ensure, Context, Result};
pub use dsp::DspOutputPath;
pub use ebr::{EbrModeResult, ModeOverride};
pub(crate) use pio::differential;
pub use pio::PioMode;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

pub type Bit = (usize, usize, bool);

#[derive(Debug, Deserialize, Serialize)]
pub struct Catalog {
    schema_version: u32,
    pub device: String,
    pub frames: usize,
    pub bits_per_frame: usize,
    pub resources: Vec<Resource>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Resource {
    pub id: String,
    pub family: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Field {
    kind: Kind,
    pub name: String,
    pub fragments: Vec<Fragment>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Enum,
    Word,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Fragment {
    tile: String,
    source: String,
    default: Option<String>,
    #[serde(default)]
    options: BTreeMap<String, Vec<Bit>>,
    #[serde(default)]
    word_bits: Vec<Vec<Bit>>,
}

#[derive(Deserialize)]
struct Snapshot {
    schema_version: u32,
    device: String,
    frames: usize,
    bits_per_frame: usize,
    encoding: String,
    cram_hex: String,
    bram_data: BTreeMap<u16, Vec<u16>>,
    origin: String,
}

#[derive(Deserialize, Serialize)]
pub struct Cram {
    pub device: String,
    pub frames: usize,
    pub bits_per_frame: usize,
    data: Vec<u8>,
    pub bram: BTreeMap<u16, Vec<u16>>,
    pub origin: String,
}

impl Cram {
    pub fn from_json(json: &[u8]) -> Result<Self> {
        let s: Snapshot = serde_json::from_slice(json).context("reading CRAM snapshot")?;
        ensure!(
            s.schema_version == 1 && s.encoding == "frame_byte_lsb0",
            "unsupported CRAM snapshot format"
        );
        ensure!(
            s.frames > 0 && s.bits_per_frame > 0 && s.bits_per_frame.is_multiple_of(8),
            "invalid CRAM dimensions"
        );
        let size = s
            .frames
            .checked_mul(s.bits_per_frame / 8)
            .context("CRAM size overflow")?;
        ensure!(
            size.checked_mul(2) == Some(s.cram_hex.len()),
            "CRAM snapshot length differs from dimensions"
        );
        let nibble = |c: u8| -> Result<u8> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => bail!("invalid CRAM hex digit"),
            }
        };
        let data = s
            .cram_hex
            .as_bytes()
            .chunks_exact(2)
            .map(|c| Ok(nibble(c[0])? * 16 + nibble(c[1])?))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            s.bram_data.values().flatten().all(|&v| v <= 0x1ff),
            "ECP5 BRAM data exceeds nine bits"
        );
        Ok(Self {
            device: s.device,
            frames: s.frames,
            bits_per_frame: s.bits_per_frame,
            data,
            bram: s.bram_data,
            origin: s.origin,
        })
    }

    pub fn load(path: &Path, database: &Path, catalog: &Path) -> Result<Self> {
        if path
            .extension()
            .is_none_or(|s| !s.eq_ignore_ascii_case("bit"))
        {
            // Validate textual values before asking the native library to apply them.
            Config::parse(&std::fs::read_to_string(path)?)?;
        }
        let output =
            Command::new(std::env::var_os("ECP5_PYTHON").unwrap_or_else(|| "python3".into()))
                .args(["-c", include_str!("../tools/trellis_bridge.py")])
                .arg(database)
                .arg(path)
                .arg("cram")
                .arg(catalog)
                .output()
                .context("loading CRAM through libtrellis")?;
        ensure!(
            output.status.success(),
            "CRAM import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self::from_json(&output.stdout)
    }

    pub fn bit(&self, frame: usize, bit: usize) -> bool {
        self.data[frame * (self.bits_per_frame / 8) + bit / 8] & (1 << (bit % 8)) != 0
    }

    pub fn fingerprint(&self) -> String {
        let mut hash = 0xcbf29ce484222325_u64;
        for b in &self.data {
            hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        format!("fnv1a64:{hash:016x}")
    }
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FieldResult {
    pub name: String,
    pub kind: &'static str,
    pub status: String,
    pub value: Option<String>,
    pub candidates: Vec<String>,
    pub matching_values: Vec<String>,
    pub specificity: usize,
    pub fragments: Vec<Source>,
    pub observed_bits: Vec<Bit>,
    pub word_bits_lsb0: Vec<Option<bool>>,
    pub conflicting_word_bits: Vec<usize>,
    pub applicable: Option<bool>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Source {
    tile: String,
    source: String,
    default: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ResourceResult {
    pub id: String,
    pub family: String,
    pub enabled: Option<bool>,
    pub fields: Vec<FieldResult>,
    pub memory: Option<MemoryBinding>,
    /// Effective EBR mode, preserving the raw MODE field and any overlay evidence.
    pub ebr_mode: Option<EbrModeResult>,
    pub dsp_output_path: Option<DspOutputPath>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct MemoryBinding {
    pub wid: Option<u16>,
    pub status: String,
    pub image_id: Option<u16>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct InitImage {
    pub status: String,
    pub coverage: String,
    pub words_9bit: Vec<u16>,
    pub potential_owners: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    schema_version: u32,
    pub device: String,
    pub origin: String,
    pub cram_fingerprint: String,
    pub resources: Vec<ResourceResult>,
    pub images: BTreeMap<u16, InitImage>,
    pub field_status_counts: BTreeMap<String, usize>,
    pub limitations: Vec<&'static str>,
}

impl Field {
    fn constraints(&self) -> BTreeSet<Bit> {
        self.fragments
            .iter()
            .flat_map(|f| f.options.values().chain(f.word_bits.iter()))
            .flatten()
            .copied()
            .collect()
    }

    fn decode(&self, cram: &Cram) -> FieldResult {
        let observed: BTreeSet<_> = self
            .constraints()
            .into_iter()
            .map(|(f, b, _)| (f, b, cram.bit(f, b)))
            .collect();
        let mut r = FieldResult {
            name: self.name.clone(),
            kind: if self.kind == Kind::Enum {
                "enum"
            } else {
                "word"
            },
            status: String::new(),
            value: None,
            candidates: vec![],
            matching_values: vec![],
            specificity: 0,
            fragments: self
                .fragments
                .iter()
                .map(|f| Source {
                    tile: f.tile.clone(),
                    source: f.source.clone(),
                    default: f.default.clone(),
                })
                .collect(),
            observed_bits: observed.into_iter().collect(),
            word_bits_lsb0: vec![],
            conflicting_word_bits: vec![],
            applicable: None,
        };
        if self.kind == Kind::Enum {
            let values: BTreeSet<_> = self
                .fragments
                .iter()
                .flat_map(|f| f.options.keys())
                .collect();
            let mut matches = Vec::new();
            for value in values {
                let bits: BTreeSet<_> = self
                    .fragments
                    .iter()
                    .filter_map(|f| f.options.get(value))
                    .flatten()
                    .copied()
                    .collect();
                if bits.iter().all(|&(f, b, v)| cram.bit(f, b) == v) {
                    matches.push((
                        value.clone(),
                        bits,
                        self.fragments.iter().all(|f| f.options.contains_key(value)),
                    ));
                }
            }
            r.matching_values = matches.iter().map(|m| m.0.clone()).collect();
            if !matches.is_empty() {
                // Only a strict superset of the same observations can rule out
                // a weaker pattern. Incomparable patterns remain ambiguous.
                let best: Vec<_> = matches
                    .iter()
                    .filter(|m| !matches.iter().any(|n| m.1 != n.1 && m.1.is_subset(&n.1)))
                    .collect();
                r.specificity = best.iter().map(|m| m.1.len()).max().unwrap();
                r.candidates = best.iter().map(|m| m.0.clone()).collect();
                r.status = if best.len() > 1 {
                    "ambiguous"
                } else if best[0].1.is_empty() {
                    "unobserved"
                } else if !best[0].2 {
                    "partial"
                } else {
                    r.value = Some(best[0].0.clone());
                    "resolved"
                }
                .into();
            } else {
                r.status = "unmatched".into();
            }
        } else {
            let width = self.fragments[0].word_bits.len();
            for i in 0..width {
                let mut observations = BTreeSet::new();
                let mut conflict = false;
                for fragment in &self.fragments {
                    let group = &fragment.word_bits[i];
                    if group.is_empty() {
                        continue;
                    }
                    let ones = group
                        .iter()
                        .filter(|&&(f, b, v)| cram.bit(f, b) == v)
                        .count();
                    if ones == group.len() {
                        observations.insert(true);
                    } else if ones == 0 {
                        observations.insert(false);
                    } else {
                        conflict = true;
                    }
                }
                if conflict || observations.len() > 1 {
                    r.conflicting_word_bits.push(i);
                }
                r.word_bits_lsb0
                    .push(if !conflict && observations.len() == 1 {
                        observations.first().copied()
                    } else {
                        None
                    });
            }
            r.status = if !r.conflicting_word_bits.is_empty() {
                "conflict"
            } else if r.word_bits_lsb0.iter().any(Option::is_none) {
                "partial"
            } else {
                "resolved"
            }
            .into();
            if r.status == "resolved" {
                r.value = Some(
                    r.word_bits_lsb0
                        .iter()
                        .rev()
                        .map(|v| if *v == Some(true) { '1' } else { '0' })
                        .collect(),
                );
            }
        }
        r
    }
}

impl Catalog {
    /// Merge pinned research catalogues, preserving every distinct fragment.
    pub fn merge_documents(
        documents: &[Vec<u8>],
        device: &str,
        frames: usize,
        bits_per_frame: usize,
    ) -> Result<Self> {
        let mut resources: BTreeMap<String, Resource> = BTreeMap::new();
        for bytes in documents {
            let mut document: serde_json::Value = serde_json::from_slice(bytes)?;
            let object = document
                .as_object_mut()
                .context("configuration catalogue is not an object")?;
            for (key, value) in [
                ("schema_version", serde_json::json!(1)),
                ("frames", serde_json::json!(frames)),
                ("bits_per_frame", serde_json::json!(bits_per_frame)),
            ] {
                object.entry(key).or_insert(value);
            }
            let part: Self = serde_json::from_value(document)?;
            part.validate()?;
            ensure!(
                part.device == device
                    && part.frames == frames
                    && part.bits_per_frame == bits_per_frame,
                "configuration catalogue belongs to another device/layout"
            );
            for incoming in part.resources {
                let target = resources
                    .entry(incoming.id.clone())
                    .or_insert_with(|| Resource {
                        id: incoming.id.clone(),
                        family: incoming.family.clone(),
                        fields: Vec::new(),
                    });
                ensure!(
                    target.family == incoming.family,
                    "conflicting resource family {}",
                    target.id
                );
                for field in incoming.fields {
                    if let Some(existing) = target.fields.iter_mut().find(|f| f.name == field.name)
                    {
                        ensure!(
                            existing.kind == field.kind,
                            "conflicting field kind {}",
                            field.name
                        );
                        for fragment in field.fragments {
                            if let Some(prior) = existing
                                .fragments
                                .iter()
                                .find(|f| f.tile == fragment.tile && f.source == fragment.source)
                            {
                                ensure!(
                                    *prior == fragment,
                                    "conflicting fragment {}:{}",
                                    target.id,
                                    field.name
                                );
                            } else {
                                existing.fragments.push(fragment);
                            }
                        }
                    } else {
                        target.fields.push(field);
                    }
                }
            }
        }
        let merged = Self {
            schema_version: 1,
            device: device.into(),
            frames,
            bits_per_frame,
            resources: resources.into_values().collect(),
        };
        merged.validate()?;
        Ok(merged)
    }

    pub fn validate_tiles(&self, database: &crate::database::Database) -> Result<()> {
        self.validate()?;
        for r in &self.resources {
            for field in &r.fields {
                for f in &field.fragments {
                    let tile = database
                        .tiles
                        .get(&f.tile)
                        .with_context(|| format!("unknown configuration tile {}", f.tile))?;
                    for &(frame, bit, _) in f.options.values().chain(f.word_bits.iter()).flatten() {
                        ensure!(
                            (tile.cram.start_frame..tile.cram.start_frame + tile.cram.frames)
                                .contains(&frame)
                                && (tile.cram.start_bit..tile.cram.start_bit + tile.cram.bits)
                                    .contains(&bit),
                            "configuration fragment exceeds owner tile {}",
                            f.tile
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let catalog: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.device == "LFE5U-85F",
            "unsupported decode catalogue"
        );
        ensure!(
            self.frames > 0 && self.bits_per_frame > 0,
            "invalid catalogue dimensions"
        );
        let mut ids = BTreeSet::new();
        for r in &self.resources {
            ensure!(ids.insert(&r.id), "duplicate resource ID {}", r.id);
            let mut names = BTreeSet::new();
            for f in &r.fields {
                ensure!(names.insert(&f.name), "duplicate field {}:{}", r.id, f.name);
                ensure!(!f.fragments.is_empty(), "field {} has no fragments", f.name);
                if f.kind == Kind::Word {
                    let width = f.fragments[0].word_bits.len();
                    ensure!(
                        width > 0 && f.fragments.iter().all(|x| x.word_bits.len() == width),
                        "inconsistent word width {}",
                        f.name
                    );
                }
                for (frame, bit, _) in f.constraints() {
                    ensure!(
                        frame < self.frames && bit < self.bits_per_frame,
                        "catalogue bit outside CRAM: {frame}:{bit}"
                    );
                }
            }
        }
        Ok(())
    }

    /// The PIO resources only, kept for the lazy joint PIO decoder.
    pub fn pio_subset(&self) -> Catalog {
        Catalog {
            schema_version: self.schema_version,
            device: self.device.clone(),
            frames: self.frames,
            bits_per_frame: self.bits_per_frame,
            resources: self
                .resources
                .iter()
                .filter(|r| r.family == "PIO")
                .cloned()
                .collect(),
        }
    }
    /// Effective modes of PIO `id` and of every PIO sharing writer bits with
    /// it (normally its pair), decoded together; see decode/pio.rs.
    pub fn pio_modes(&self, cram: &Cram, id: &str) -> Result<BTreeMap<String, PioMode>> {
        let pios: Vec<&Resource> = self
            .resources
            .iter()
            .filter(|r| r.family == "PIO")
            .collect();
        ensure!(
            pios.iter().any(|r| r.id == id),
            "no PIO decode catalogue entry: {id}"
        );
        let tiles = |r: &Resource| -> BTreeSet<String> {
            r.fields
                .iter()
                .flat_map(|f| f.fragments.iter().map(|fr| fr.tile.clone()))
                .collect()
        };
        let mut group: Vec<&Resource> = pios.iter().filter(|r| r.id == id).copied().collect();
        let mut bits = pio::writer_positions(group[0]);
        while let Some(&next) = pios.iter().find(|r| {
            !group.iter().any(|g| g.id == r.id)
                && group.iter().any(|g| !tiles(g).is_disjoint(&tiles(r)))
                && !pio::writer_positions(r).is_disjoint(&bits)
        }) {
            bits.extend(pio::writer_positions(next));
            group.push(next);
        }
        Ok(pio::group_modes(&group, cram))
    }
    pub fn decode(&self, cram: &Cram, selected: &[String]) -> Result<Report> {
        ensure!(
            cram.device == self.device
                && cram.frames == self.frames
                && cram.bits_per_frame == self.bits_per_frame,
            "CRAM/catalogue mismatch"
        );
        for id in selected {
            ensure!(
                self.resources.iter().any(|r| &r.id == id),
                "resource has no decode catalogue entry: {id}"
            );
        }
        // Decode and bind all EBRs before applying the display filter: hidden
        // resources can share WID with a visible one.
        let mut resources: Vec<_> = self
            .resources
            .iter()
            .map(|r| {
                let mut fields: Vec<_> = r.fields.iter().map(|f| f.decode(cram)).collect();
                let raw_mode = fields
                    .iter()
                    .find(|f| f.name.split('.').count() == 2 && f.name.ends_with(".MODE"))
                    .and_then(|f| f.value.clone());
                let ebr_mode = if r.family == "EBR" {
                    ebr::mode(r, &fields, cram)
                } else {
                    None
                };
                let dsp_output_path = if r.family == "MULT18" {
                    dsp::output_path(r, &fields, cram)
                } else {
                    None
                };
                let mode = ebr_mode.as_ref().and_then(|m| m.value.clone()).or(raw_mode);
                let enabled = mode.as_ref().map(|m| m != "NONE");
                if r.family == "EBR" {
                    for f in &mut fields {
                        let required = if f.name.contains(".DP16KD.")
                            || f.name.ends_with(".WEAMUX")
                            || f.name.ends_with(".WEBMUX")
                        {
                            Some("DP16KD")
                        } else if f.name.contains(".PDPW16KD.") {
                            Some("PDPW16KD")
                        } else {
                            None
                        };
                        f.applicable =
                            required.and_then(|required| mode.as_ref().map(|m| m == required));
                    }
                }
                ResourceResult {
                    id: r.id.clone(),
                    family: r.family.clone(),
                    enabled,
                    fields,
                    memory: None,
                    ebr_mode,
                    dsp_output_path,
                }
            })
            .collect();
        let mut owners: BTreeMap<u16, Vec<String>> = BTreeMap::new();
        for r in resources.iter_mut().filter(|r| r.family == "EBR") {
            let wid = r
                .fields
                .iter()
                .find(|f| f.name.ends_with(".WID"))
                .and_then(|f| f.value.as_ref())
                .and_then(|v| u16::from_str_radix(v, 2).ok())
                .map(|v| v.reverse_bits() >> 7);
            if r.enabled != Some(false) {
                if let Some(w) = wid {
                    owners.entry(w).or_default().push(r.id.clone());
                }
            }
            r.memory = Some(MemoryBinding {
                wid,
                status: String::new(),
                image_id: None,
            });
        }
        let images: BTreeMap<_, _> = cram
            .bram
            .iter()
            .map(|(&id, words)| {
                (
                    id,
                    InitImage {
                        status: if words.len() == 2048 {
                            "complete_length"
                        } else {
                            "invalid_length"
                        }
                        .into(),
                        coverage: if cram.origin == "text_config" {
                            "explicit_text_words"
                        } else {
                            "libtrellis_decoded_without_write_mask"
                        }
                        .into(),
                        words_9bit: words.clone(),
                        potential_owners: owners.get(&id).cloned().unwrap_or_default(),
                    },
                )
            })
            .collect();
        for r in resources.iter_mut().filter(|r| r.family == "EBR") {
            let m = r.memory.as_mut().unwrap();
            m.status = match (r.enabled, m.wid) {
                (Some(false), _) => "inactive",
                (None, _) | (_, None) => "indeterminate",
                (Some(true), Some(w)) if owners[&w].len() > 1 => "duplicate_wid",
                (Some(true), Some(w)) => match images.get(&w) {
                    None => "missing_init",
                    Some(i) if i.status == "invalid_length" => "invalid_image_length",
                    Some(_) => {
                        m.image_id = Some(w);
                        "bound"
                    }
                },
            }
            .into();
        }
        resources.retain(|r| selected.is_empty() || selected.contains(&r.id));
        let mut counts = BTreeMap::new();
        for f in resources.iter().flat_map(|r| &r.fields) {
            *counts.entry(f.status.clone()).or_default() += 1;
        }
        Ok(Report { schema_version:1,device:self.device.clone(),origin:cram.origin.clone(),cram_fingerprint:cram.fingerprint(),resources,images,
            field_status_counts:counts,limitations:vec![
                "Decoded fields are bit-pattern interpretations, not executable resource behaviour.",
                "Enum candidates discard only strict subsets of matching constraints; aliases and incomparable patterns remain explicit.",
                "EBR effective mode may qualify the single verified WEAMUX override for DP/NONE; raw MODE observations and status remain unchanged.",
                "Partial configuration streams and dynamic reconfiguration are not validated.",
                "libtrellis does not expose an EBR write-coverage mask; decoded bitstream images retain that limitation.",
                "Missing or invalid INIT does not imply zero-filled physical memory."] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field(value: serde_json::Value) -> Field {
        serde_json::from_value(value).unwrap()
    }
    fn cram(bits: u16) -> Cram {
        Cram {
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 16,
            data: bits.to_le_bytes().to_vec(),
            bram: BTreeMap::new(),
            origin: "text_config".into(),
        }
    }
    fn mode() -> Field {
        field(json!({"kind":"enum","name":"EBR0.MODE","fragments":[
            {"tile":"A","source":"a:1","default":"PDPW16KD","options":{"NONE":[[0,0,false]],"DP16KD":[[0,0,false]],"PDPW16KD":[[0,0,true]]}},
            {"tile":"B","source":"b:1","default":"DP16KD","options":{"NONE":[[0,1,false]],"DP16KD":[[0,1,true]],"PDPW16KD":[[0,1,true]]}}]}))
    }
    fn ebr(id: &str, enable_bit: usize) -> Resource {
        let word: Vec<_> = (0..9).map(|i| json!([[0, i, true]])).collect();
        Resource {
            id: id.into(),
            family: "EBR".into(),
            fields: vec![
                field(
                    json!({"kind":"enum","name":"EBR0.MODE","fragments":[{"tile":"T","source":"src:1","default":"NONE","options":{"DP16KD":[[0,enable_bit,true]],"NONE":[[0,enable_bit,false]]}}]}),
                ),
                field(
                    json!({"kind":"word","name":"EBR0.WID","fragments":[{"tile":"T","source":"src:2","default":"000000000","word_bits":word}]}),
                ),
            ],
        }
    }
    fn catalog(resources: Vec<Resource>) -> Catalog {
        Catalog {
            schema_version: 1,
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 16,
            resources,
        }
    }

    #[test]
    fn complete_mode_uses_all_fragments_and_does_not_substitute_defaults() {
        let f = mode();
        for (bits, value) in [(0, "NONE"), (2, "DP16KD"), (3, "PDPW16KD")] {
            let r = f.decode(&cram(bits));
            assert_eq!(r.status, "resolved");
            assert_eq!(r.value.as_deref(), Some(value));
        }
        let tail = Field {
            kind: Kind::Enum,
            name: f.name.clone(),
            fragments: f.fragments.into_iter().skip(1).collect(),
        };
        let r = tail.decode(&cram(2));
        assert_eq!(r.status, "ambiguous");
        assert_eq!(r.candidates, vec!["DP16KD", "PDPW16KD"]);
        assert_eq!(mode().decode(&cram(1)).status, "unmatched");
    }

    #[test]
    fn empty_patterns_and_incomplete_domains_remain_explicit() {
        let f = field(
            json!({"kind":"enum","name":"X","fragments":[{"tile":"T","source":"s","default":"B","options":{"A":[],"B":[]}}]}),
        );
        assert_eq!(f.decode(&cram(0)).status, "ambiguous");
        let f = field(
            json!({"kind":"enum","name":"X","fragments":[{"tile":"T","source":"s","options":{"A":[[0,0,true]],"B":[[0,0,false]]}},{"tile":"U","source":"s","options":{"A":[[0,1,true]]}}]}),
        );
        let r = f.decode(&cram(0));
        assert_eq!(r.status, "partial");
        assert!(r.value.is_none());
    }

    #[test]
    fn incomparable_enum_patterns_remain_ambiguous_despite_different_sizes() {
        let mut f = field(json!({"kind":"enum","name":"X","fragments":[
        {"tile":"T","source":"s","options":{
            "A":[[0,0,true]],"B":[[0,1,true],[0,2,true]],"EMPTY":[]
        }}]}));
        let r = f.decode(&cram(7));
        assert_eq!(r.status, "ambiguous");
        assert_eq!(r.candidates, vec!["A", "B"]);
        assert_eq!(r.matching_values, vec!["A", "B", "EMPTY"]);
        f.fragments[0]
            .options
            .insert("C".into(), vec![(0, 0, true), (0, 1, true), (0, 2, true)]);
        assert_eq!(f.decode(&cram(7)).value.as_deref(), Some("C"));
    }

    #[test]
    fn word_fragments_ignore_empty_groups_and_report_replica_conflicts() {
        let f = field(json!({"kind":"word","name":"W","fragments":[
            {"tile":"T","source":"s","word_bits":[[[0,0,true]],[]]},
            {"tile":"U","source":"s","word_bits":[[],[[0,1,false]]]}]}));
        assert_eq!(f.decode(&cram(1)).value.as_deref(), Some("11"));
        let partial = field(
            json!({"kind":"word","name":"W","fragments":[{"tile":"T","source":"s","word_bits":[[],[[0,1,true]]]}]}),
        );
        assert_eq!(
            partial.decode(&cram(0)).word_bits_lsb0,
            vec![None, Some(false)]
        );
        let replica = field(
            json!({"kind":"word","name":"W","fragments":[{"tile":"T","source":"s","word_bits":[[[0,0,true],[0,1,true]]]}]}),
        );
        assert_eq!(replica.decode(&cram(1)).status, "conflict");
    }

    #[test]
    fn all_nine_bit_wids_reverse_and_bind_without_losing_data() {
        let cat = catalog(vec![ebr("ebr", 15)]);
        for wid in 0_u16..512 {
            let mut c = cram((wid.reverse_bits() >> 7) | 0x8000);
            c.bram.insert(wid, vec![wid; 2048]);
            let r = cat.decode(&c, &[]).unwrap();
            let m = r.resources[0].memory.as_ref().unwrap();
            assert_eq!(m.wid, Some(wid));
            assert_eq!(m.image_id, Some(wid));
            assert_eq!(m.status, "bound");
            assert_eq!(r.images[&wid].words_9bit, vec![wid; 2048]);
        }
    }

    #[test]
    fn hidden_ebr_owners_still_prevent_duplicate_wid_binding() {
        let cat = catalog(vec![ebr("a", 15), ebr("b", 15)]);
        let mut c = cram(0x8000);
        c.bram.insert(0, vec![0; 2048]);
        let r = cat.decode(&c, &["a".into()]).unwrap();
        assert_eq!(
            r.resources[0].memory.as_ref().unwrap().status,
            "duplicate_wid"
        );
        assert_eq!(r.images[&0].potential_owners, vec!["a", "b"]);
    }

    #[test]
    fn missing_partial_and_inactive_images_do_not_become_zero_memory() {
        let cat = catalog(vec![ebr("ebr", 15)]);
        let mut c = cram(0x8000);
        assert_eq!(
            cat.decode(&c, &[]).unwrap().resources[0]
                .memory
                .as_ref()
                .unwrap()
                .status,
            "missing_init"
        );
        c.bram.insert(0, vec![7; 2047]);
        let r = cat.decode(&c, &[]).unwrap();
        assert_eq!(
            r.resources[0].memory.as_ref().unwrap().status,
            "invalid_image_length"
        );
        assert_eq!(r.images[&0].words_9bit.len(), 2047);
        let r = cat.decode(&cram(0), &[]).unwrap();
        assert_eq!(r.resources[0].memory.as_ref().unwrap().status, "inactive");
    }

    #[test]
    fn invalid_snapshot_and_catalogue_bounds_are_rejected() {
        let snapshot = json!({"schema_version":1,"device":"LFE5U-85F","frames":1,"bits_per_frame":16,
            "encoding":"frame_byte_lsb0","cram_hex":"0080","bram_data":{"0":[512]},"origin":"text_config"});
        assert!(Cram::from_json(snapshot.to_string().as_bytes()).is_err());
        let mut cat = catalog(vec![ebr("ebr", 16)]);
        assert!(cat.validate().is_err());
        cat.bits_per_frame = 32;
        assert!(cat.decode(&cram(0), &[]).is_err());
    }
}
