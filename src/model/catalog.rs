use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

pub(super) const INPUTS: [&str; 8] = [
    "device_inventory.json",
    "resource_instances.csv",
    "site_bindings.csv",
    "resource_ports.csv",
    "iologic_research/resource_ports.csv",
    "plc_research/node_templates.csv",
    "plc_research/native_ports.csv",
    "package_system_research/physical_pins.csv",
];

#[derive(Deserialize)]
pub(crate) struct Source {
    pub path: String,
    pub sha256: String,
}
#[derive(Deserialize)]
pub(super) struct Manifest {
    pub schema_version: u32,
    pub identity_schema_version: u32,
    pub device: String,
    pub database_revision: String,
    pub trellis_revision: String,
    pub files: Vec<Source>,
}
#[derive(Deserialize)]
pub(super) struct Geometry {
    pub frames: usize,
    pub bits_per_frame: usize,
    pub max_row: i16,
    pub max_col: i16,
}
#[derive(Deserialize)]
pub(super) struct Inventory {
    pub device: String,
    pub database_revision: String,
    pub trellis_revision: String,
    pub device_info: Geometry,
    pub source_files: Vec<Source>,
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn verify(root: &Path, sources: &[Source]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for source in sources {
        ensure!(
            !source.path.is_empty()
                && Path::new(&source.path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "source path must be relative: {}",
            source.path
        );
        let path = root.join(&source.path);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            digest(&bytes) == source.sha256,
            "architecture source changed: {}",
            path.display()
        );
        ensure!(
            files.insert(source.path.clone(), bytes).is_none(),
            "duplicate source {}",
            source.path
        );
    }
    Ok(files)
}

pub(super) fn csv<T: DeserializeOwned>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> Result<Vec<T>> {
    let data = files
        .get(name)
        .with_context(|| format!("missing architecture input {name}"))?;
    csv::Reader::from_reader(data.as_slice())
        .deserialize()
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("reading architecture table {name}"))
}

#[derive(Deserialize)]
pub(super) struct ResourceRow {
    pub resource_id: String,
    pub family: String,
    pub fabric_row: i16,
    pub fabric_col: i16,
    pub name: String,
    pub anchor_tile: String,
    pub parent_id: String,
    pub native_bel: String,
    pub native_name: String,
    pub rule: String,
}
#[derive(Deserialize)]
pub(super) struct SiteRow {
    pub tile: String,
    pub site_index: usize,
    pub site_name: String,
    pub label_family: String,
    pub vendor_pos_row: i16,
    pub vendor_pos_col: i16,
    pub resource_id: String,
    pub relation: super::Binding,
    pub rule: String,
}
#[derive(Deserialize)]
pub(super) struct PortRow {
    pub resource_id: String,
    pub port: String,
    pub direction: super::Direction,
    pub wire_row: i16,
    pub wire_col: i16,
    pub wire: String,
}
#[derive(Clone, Deserialize)]
pub(super) struct PortTemplate {
    pub view: String,
    pub bel: String,
    pub port: String,
    pub direction: super::Direction,
    pub wire_row_offset: i16,
    pub wire_col_offset: i16,
    pub wire: String,
}
#[derive(Clone, Deserialize)]
pub(super) struct NodeTemplate {
    pub view: String,
    pub native_name: String,
    pub native_type: String,
    pub z: i16,
    pub interpretation: String,
}
#[derive(Deserialize)]
pub(super) struct PinRow {
    pub package: String,
    pub ball: String,
    #[serde(rename = "BSDL_function")]
    pub bsdl_function: String,
    #[serde(rename = "PAD")]
    pub pad: String,
    pub pinout_function: String,
    pub dual_function: String,
    pub bank: String,
    pub kind: String,
    pub evidence: String,
}
