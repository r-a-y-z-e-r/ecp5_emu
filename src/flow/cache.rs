//! Content hashes for incremental steps. State is replaced atomically.
use super::pipeline::Step;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn hash(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn file_hash(path: &Path) -> Result<String> {
    Ok(hash(
        &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    ))
}
/// Includes named input hashes and unambiguous, length-delimited argv/parameters.
pub(crate) fn key(parts: &[String], inputs: &[(PathBuf, PathBuf)]) -> Result<String> {
    let inputs = inputs
        .iter()
        .map(|(name, path)| Ok((name, file_hash(path)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(hash(&serde_json::to_vec(&(1, parts, inputs))?))
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Cache {
    entries: BTreeMap<Step, Entry>,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    key: String,
    outputs: Vec<(PathBuf, String)>,
}
impl Cache {
    pub fn read(dir: &Path) -> Self {
        fs::read(dir.join(".flow/state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        let dir = dir.join(".flow");
        fs::create_dir_all(&dir)?;
        let tmp = dir.join("state.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, dir.join("state.json"))?;
        Ok(())
    }
    pub fn matches(&self, step: Step, key: &str, outputs: &[PathBuf]) -> bool {
        self.entries.get(&step).is_some_and(|e| {
            e.key == key
                && e.outputs.len() == outputs.len()
                && e.outputs.iter().zip(outputs).all(|((p, h), expected)| {
                    p == expected && file_hash(p).is_ok_and(|actual| actual == *h)
                })
        })
    }
    pub fn remove(&mut self, step: Step) {
        self.entries.remove(&step);
    }
    pub fn record(&mut self, step: Step, key: String, outputs: &[PathBuf]) -> Result<()> {
        let outputs = outputs
            .iter()
            .map(|p| Ok((p.clone(), file_hash(p)?)))
            .collect::<Result<_>>()?;
        self.entries.insert(step, Entry { key, outputs });
        Ok(())
    }
}
