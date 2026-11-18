//! Configuration bound to an immutable architecture, independent of live state.
use crate::{
    database::{ArcKind, BitConstraint},
    decode::{Catalog, Report},
    image::ConfigurationImage,
    logic::Logic,
    model::{
        topology::{ConnectionId, Endpoint},
        DeviceModel, ResourceId, TileId,
    },
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

/// Opt-in load measurements used by the ignored window-sequence benchmark.
#[cfg(feature = "compile-timing")]
pub mod load_timing {
    use std::{cell::RefCell, time::Instant};

    #[derive(Debug, serde::Serialize)]
    pub struct Sample {
        pub step: &'static str,
        pub seconds: f64,
        pub bytes: u64,
    }
    thread_local! {
        static SAMPLES: RefCell<Option<Vec<Sample>>> = const { RefCell::new(None) };
    }
    pub struct Timer(Option<(&'static str, u64, Instant)>);
    pub fn enter(step: &'static str, bytes: u64) -> Timer {
        Timer(SAMPLES.with(|s| s.borrow().is_some().then(|| (step, bytes, Instant::now()))))
    }
    impl Drop for Timer {
        fn drop(&mut self) {
            if let Some((step, bytes, since)) = self.0 {
                SAMPLES.with(|s| {
                    if let Some(samples) = s.borrow_mut().as_mut() {
                        samples.push(Sample {
                            step,
                            bytes,
                            seconds: since.elapsed().as_secs_f64(),
                        });
                    }
                });
            }
        }
    }
    pub fn begin() {
        SAMPLES.with(|s| *s.borrow_mut() = Some(Vec::new()));
    }
    pub fn finish() -> Vec<Sample> {
        SAMPLES.with(|s| s.borrow_mut().take().unwrap_or_default())
    }

    /// Complete construction snapshot, excluding process-local handle scopes.
    pub fn compiled_identity(compiled: &crate::compile::Compiled) -> Option<Vec<u8>> {
        crate::kernel::cache::encode(compiled)
    }
}

pub const CATALOGUES: [&str; 5] = [
    "decode_catalog.json",
    "iologic_research/config_catalogue.json",
    "pio_bank_research/config_catalogue.json",
    "clock_system_research/config_catalogue.json",
    "dqs_research/config_catalogue.json",
];

#[derive(Deserialize)]
struct ConfigurationManifest {
    schema_version: u32,
    device: String,
    architecture_manifest_sha256: String,
    files: Vec<crate::model::catalog::Source>,
}

pub fn load_catalogue(model: &DeviceModel, folder: &Path) -> Result<Catalog> {
    #[cfg(feature = "compile-timing")]
    let _timer = load_timing::enter("catalogue.load", 0);
    let manifest: ConfigurationManifest = serde_json::from_slice(
        &std::fs::read(folder.join("configuration_manifest.json")).context(
            "reading configuration_manifest.json; run tools/build_configuration_manifest.py",
        )?,
    )?;
    ensure!(
        manifest.schema_version == 1
            && manifest.device == model.database().device
            && manifest.architecture_manifest_sha256 == model.manifest_sha256(),
        "configuration manifest and architecture disagree"
    );
    let files = crate::model::catalog::verify(folder, &manifest.files)?;
    ensure!(
        files.len() == CATALOGUES.len() && CATALOGUES.iter().all(|p| files.contains_key(*p)),
        "configuration catalogue set differs"
    );
    let (frames, bits) = model.cram_dimensions();
    Catalog::merge_documents(
        &CATALOGUES
            .iter()
            .map(|p| files[*p].clone())
            .collect::<Vec<_>>(),
        &manifest.device,
        frames,
        bits,
    )
}

#[derive(Serialize)]
pub struct ConfigurationSummary {
    pub resources_with_composed_fields: usize,
    pub composed_fields: usize,
    pub field_statuses: BTreeMap<String, usize>,
    pub ebr_binding_statuses: BTreeMap<String, usize>,
    pub ebr_mode_statuses: BTreeMap<String, usize>,
    pub init_image_count: usize,
    pub plc_words_from_tile_bit_patterns: usize,
    pub sysconfig_application: &'static str,
}

pub struct ConfiguredDevice {
    model: Arc<DeviceModel>,
    image: Arc<ConfigurationImage>,
    decoded: Report,
    fingerprint: String,
    /// PIO settings for the lazy joint decoder (decode/pio.rs).
    pio_catalogue: Catalog,
}

#[derive(Debug)]
pub struct RawMuxSelection {
    pub matching_sources: Vec<String>,
    pub candidates: Vec<ConnectionId>,
    pub candidate_sources: Vec<String>,
    pub outside_device_sources: Vec<String>,
    /// A bit-pattern match is not physical route activation (MODE/shared bits matter).
    pub requires_mode_interpretation: bool,
}

pub(crate) struct MatchingConnection<'a> {
    pub tile: &'a crate::database::Tile,
    pub definition: &'a crate::database::ArcDef,
    pub source: Endpoint,
    pub sink: Endpoint,
}
impl MatchingConnection<'_> {
    pub fn cram_constraints(&self) -> impl Iterator<Item = BitConstraint> + '_ {
        self.definition.bits.iter().map(|b| BitConstraint {
            frame: self.tile.cram.start_frame + b.frame,
            bit: self.tile.cram.start_bit + b.bit,
            value: b.value,
        })
    }
}

impl ConfiguredDevice {
    /// Test CRAM before placing endpoints: most arcs of a routed sink are off.
    /// Retain native arc order and resolve every selected endpoint as before.
    pub(crate) fn matching_connections_to<'a>(
        &'a self,
        tile: TileId,
        sink: &str,
        source_override: Option<&'a str>,
    ) -> impl Iterator<Item = Result<MatchingConnection<'a>>> + 'a {
        let t = self.model.tile(tile);
        let tt = &self.model.database().types[&t.kind];
        tt.incoming
            .get(sink)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |&local| {
                let arc = &tt.routing[local];
                if let Some(source) = source_override {
                    arc.source == source
                } else {
                    arc.bits.iter().all(|b| {
                        self.image
                            .cram()
                            .bit(t.cram.start_frame + b.frame, t.cram.start_bit + b.bit)
                            == b.value
                    })
                }
            })
            .map(move |local| {
                let definition = &tt.routing[local];
                let location = crate::model::topology::Location {
                    row: t.row,
                    col: t.col,
                };
                Ok(MatchingConnection {
                    tile: t,
                    definition,
                    source: self.model.wire(location, &definition.source)?,
                    sink: self.model.wire(location, &definition.sink)?,
                })
            })
    }
    pub fn bind(
        model: Arc<DeviceModel>,
        image: Arc<ConfigurationImage>,
        catalogue: Catalog,
    ) -> Result<Self> {
        #[cfg(feature = "compile-timing")]
        let _timer = load_timing::enter("prepared.bind", 0);
        let cram = image.cram();
        let (frames, bits) = model.cram_dimensions();
        ensure!(
            cram.device == model.database().device
                && (cram.frames, cram.bits_per_frame) == (frames, bits),
            "configuration image and architecture dimensions/device disagree"
        );
        ensure!(
            catalogue.device == cram.device
                && (catalogue.frames, catalogue.bits_per_frame) == (frames, bits),
            "configuration catalogue and image disagree"
        );
        catalogue.validate_tiles(model.database())?;
        for r in &catalogue.resources {
            if let Some(id) = model.resource_id(&r.id) {
                ensure!(
                    model.resource(id).family == r.family,
                    "configuration resource family differs: {}",
                    r.id
                );
            } else {
                let bank = r.id.strip_prefix(&format!("{}/BANK", cram.device));
                ensure!(
                    r.family == "BANK"
                        && bank.is_some_and(|b| b.parse::<u8>().is_ok()
                            && model.database().types.contains_key(&format!("BANKREF{b}"))),
                    "configuration has no architecture owner: {}",
                    r.id
                );
            }
        }
        let catalogue_digest = crate::model::catalog::digest(&serde_json::to_vec(&catalogue)?);
        let fingerprint = crate::model::catalog::digest(
            format!(
                "ecp5-bound-configuration-v1\0{}:{}:{}:{}",
                model.manifest_sha256(),
                image.source_sha256(),
                image.decoded_sha256(),
                catalogue_digest
            )
            .as_bytes(),
        );
        #[cfg(feature = "compile-timing")]
        let decode_timer = load_timing::enter("prepared.catalogue_decode", 0);
        let decoded = catalogue.decode(cram, &[])?;
        #[cfg(feature = "compile-timing")]
        drop(decode_timer);
        Ok(Self {
            model,
            image,
            decoded,
            fingerprint,
            pio_catalogue: catalogue.pio_subset(),
        })
    }
    pub fn model(&self) -> &Arc<DeviceModel> {
        &self.model
    }
    pub fn image(&self) -> &Arc<ConfigurationImage> {
        &self.image
    }
    pub fn decoded(&self) -> &Report {
        &self.decoded
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    /// Joint mode of a PIO and the PIOs that share its configuration bits.
    pub fn pio_modes(
        &self,
        id: ResourceId,
    ) -> Result<std::collections::BTreeMap<String, crate::decode::PioMode>> {
        self.pio_catalogue
            .pio_modes(self.image.cram(), &self.model.resource(id).key)
    }
    pub fn resource_settings(&self, id: ResourceId) -> Option<&crate::decode::ResourceResult> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("prepared::resource_settings");
        let key = &self.model.resource(id).key;
        self.decoded.resources.iter().find(|r| &r.id == key)
    }
    /// Logical bits in LSB order. Missing or contradictory replicas remain X.
    pub fn word(&self, tile: TileId, name: &str) -> Result<Vec<Logic>> {
        let t = self.model.tile(tile);
        let groups = self.model.database().types[&t.kind]
            .word_bits
            .get(name)
            .with_context(|| format!("unknown word {name}"))?;
        Ok(groups
            .iter()
            .map(|g| {
                if g.is_empty() {
                    return Logic::X;
                }
                let ones = g
                    .iter()
                    .filter(|b| {
                        self.image
                            .cram()
                            .bit(t.cram.start_frame + b.frame, t.cram.start_bit + b.bit)
                            == b.value
                    })
                    .count();
                if ones == g.len() {
                    Logic::One
                } else if ones == 0 {
                    Logic::Zero
                } else {
                    Logic::X
                }
            })
            .collect())
    }
    /// Raw enum candidates, with strict-subset fallback patterns removed.
    /// Aliases and incomparable matches stay explicit; defaults are not evidence.
    pub fn enum_candidates(&self, tile: TileId, name: &str) -> Result<Vec<String>> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("prepared::enum_candidates");
        let t = self.model.tile(tile);
        let def = self.model.database().types[&t.kind]
            .enums
            .get(name)
            .with_context(|| format!("unknown enum {}:{name}", t.name))?;
        let matches: Vec<_> = def
            .patterns
            .iter()
            .filter(|(_, bits)| {
                bits.iter().all(|b| {
                    self.image
                        .cram()
                        .bit(t.cram.start_frame + b.frame, t.cram.start_bit + b.bit)
                        == b.value
                })
            })
            .collect();
        Ok(matches
            .iter()
            .filter(|(_, bits)| {
                !matches.iter().any(|(_, other)| {
                    bits.iter().all(|b| other.contains(b))
                        && !other.iter().all(|b| bits.contains(b))
                })
            })
            .map(|(value, _)| (*value).clone())
            .collect())
    }
    pub fn raw_mux(&self, tile: TileId, sink: &str) -> Result<RawMuxSelection> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("prepared::raw_mux");
        let mut matches = Vec::new();
        let mut seen = false;
        for connection in self.model.connections_to(tile, sink) {
            let c = connection?;
            if c.definition.kind != ArcKind::Mux || c.definition.sink != sink {
                continue;
            }
            seen = true;
            if c.source == Endpoint::OtherDensity || c.sink == Endpoint::OtherDensity {
                continue;
            }
            let bits: BTreeSet<_> = c
                .cram_constraints()
                .map(|b: BitConstraint| (b.frame, b.bit, b.value))
                .collect();
            if bits
                .iter()
                .all(|&(f, b, v)| self.image.cram().bit(f, b) == v)
            {
                matches.push((
                    c.id,
                    c.definition.source.clone(),
                    bits,
                    c.source == Endpoint::OutsideDevice || c.sink == Endpoint::OutsideDevice,
                ));
            }
        }
        ensure!(seen, "unknown mux sink {sink}");
        let best: Vec<_> = matches
            .iter()
            .filter(|m| !matches.iter().any(|n| m.2 != n.2 && m.2.is_subset(&n.2)))
            .collect();
        Ok(RawMuxSelection {
            matching_sources: matches.iter().map(|m| m.1.clone()).collect(),
            candidates: best.iter().map(|m| m.0).collect(),
            candidate_sources: best.iter().map(|m| m.1.clone()).collect(),
            outside_device_sources: best.iter().filter(|m| m.3).map(|m| m.1.clone()).collect(),
            requires_mode_interpretation: true,
        })
    }
    pub fn summary(&self) -> ConfigurationSummary {
        let mut bindings = BTreeMap::new();
        let mut modes = BTreeMap::new();
        for r in &self.decoded.resources {
            if let Some(m) = &r.memory {
                *bindings.entry(m.status.clone()).or_insert(0) += 1;
            }
            if let Some(mode) = &r.ebr_mode {
                *modes.entry(mode.status.clone()).or_insert(0) += 1;
            }
        }
        ConfigurationSummary {
            resources_with_composed_fields: self.decoded.resources.len(),
            composed_fields: self.decoded.resources.iter().map(|r| r.fields.len()).sum(),
            field_statuses: self.decoded.field_status_counts.clone(),
            ebr_binding_statuses: bindings,
            ebr_mode_statuses: modes,
            init_image_count: self.decoded.images.len(),
            plc_words_from_tile_bit_patterns: self
                .model
                .database()
                .tiles
                .values()
                .filter(|t| t.kind == "PLC2")
                .map(|t| self.model.database().types[&t.kind].word_bits.len())
                .sum(),
            sysconfig_application: "directives_preserved;startup_controller_not_executed",
        }
    }
}
