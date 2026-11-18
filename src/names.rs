//! HDL names from the routed design that nextpnr writes with `--write`.
//!
//! A net resolves to the wire that drives it: the entry of its `ROUTING`
//! attribute without a pip. A top-level port resolves to the package pin of its
//! `TRELLIS_IO` cell for stimuli and explicit pin probes. Published values
//! and waveforms observe its connected fabric data path instead of its controls.
//! Probes, stimuli and waveforms can then use the design's
//! own names. Names are per bit, as nextpnr writes them (`count[3]`); a bus
//! name without an index expands to its bits.
use crate::{
    compile::Reference,
    fabric_overlay::Overlay,
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
    sources::{Collected, Files},
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub struct Names {
    /// The reference of each net, or why it has none.
    nets: BTreeMap<String, std::result::Result<Reference, String>>,
    /// Direction-selected data references for pad names and their aliases.
    observations: BTreeMap<String, std::result::Result<Reference, String>>,
    /// Collected during the JSON pass, consumed by the runner after ready.
    pub sources: Collected,
    /// Kept from the same routed JSON for after-ready timing mapping.
    pub creator: String,
    pub placements: BTreeMap<String, String>,
    clock_names: BTreeMap<String, String>,
}

/// Yosys stores bits least-significant first, independently of HDL indices.
/// nextpnr may instead write individual, already-indexed net names.
#[derive(Clone, Copy)]
pub(crate) struct BitRange {
    offset: u32,
    upto: bool,
}

impl BitRange {
    pub(crate) fn from_json(net: &Value) -> Self {
        Self {
            offset: net["offset"].as_u64().unwrap_or(0) as u32,
            upto: net["upto"].as_u64() == Some(1),
        }
    }

    pub(crate) fn is_vector(self, len: usize) -> bool {
        len > 1 || self.offset != 0 || self.upto
    }

    pub(crate) fn index(self, len: usize, i: usize) -> u32 {
        self.offset + if self.upto { len - 1 - i } else { i } as u32
    }

    pub(crate) fn name(self, name: &str, len: usize, i: usize) -> String {
        if self.is_vector(len) {
            format!("{name}[{}]", self.index(len, i))
        } else {
            name.into()
        }
    }
}

/// `X<col>/Y<row>/<wire>` as nextpnr writes routing wires.
fn wire(text: &str) -> Result<Wire> {
    let mut parts = text.splitn(3, '/');
    let (Some(x), Some(y), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        bail!("unexpected routing wire {text:?}");
    };
    let col = x
        .strip_prefix('X')
        .context("routing wire column")?
        .parse()?;
    let row = y.strip_prefix('Y').context("routing wire row")?.parse()?;
    Ok(Wire {
        location: Location { row, col },
        name: name.into(),
    })
}

/// The source wire of a net: the routing entry without a pip. A global clock
/// net also lists its branch taps (`G_HPBX…`) without pips; its source is the
/// DCC output.
fn source(routing: &str) -> std::result::Result<Wire, String> {
    let parts: Vec<&str> = routing.split(';').collect();
    let mut roots: Vec<&str> = parts
        .chunks(3)
        .filter(|c| c.len() == 3 && c[1].is_empty() && !c[0].trim().is_empty())
        .map(|c| c[0])
        .collect();
    if roots.len() > 1 {
        roots.retain(|r| !r.split('/').nth(2).is_some_and(|n| n.starts_with("G_HPBX")));
    }
    match roots.as_slice() {
        [] => Err("no routed source (a constant or a net inside a cell)".into()),
        [one] => wire(one).map_err(|e| format!("{e:#}")),
        many => Err(format!("several routed sources {many:?}")),
    }
}

impl Names {
    pub fn load(path: &Path, model: &DeviceModel, package: &str) -> Result<Self> {
        Self::load_with_sources(path, model, package, &Files::beside(path))
    }
    pub fn load_with_sources(
        path: &Path,
        model: &DeviceModel,
        package: &str,
        files: &Files,
    ) -> Result<Self> {
        let json: Value = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        let mut names = Self::from_json(&json, model, package)?;
        let top = json["modules"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        names.sources = Collected::from_top(top, files);
        names.sources.built_at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(names)
    }
    pub fn from_json(json: &Value, model: &DeviceModel, package: &str) -> Result<Self> {
        let modules = json["modules"]
            .as_object()
            .context("routed JSON has no modules")?;
        ensure!(
            modules.len() == 1,
            "expected the single top module that nextpnr writes, found {}",
            modules.len()
        );
        let top = modules.values().next().unwrap();
        let netnames = top["netnames"]
            .as_object()
            .context("routed JSON has no netnames")?;
        // An aggregate/alias can have no ROUTING of its own. Only a
        // single-bit net's routing identifies the source of that bit.
        let mut references = BTreeMap::new();
        for net in netnames.values() {
            if net["bits"].as_array().is_some_and(|bits| bits.len() == 1) {
                if let (Some(bit), Ok(wire)) = (
                    net["bits"][0].as_u64(),
                    source(net["attributes"]["ROUTING"].as_str().unwrap_or_default()),
                ) {
                    references.insert(bit, Reference::Wire(wire));
                }
            }
        }
        // Ports: the pad bit of each TRELLIS_IO cell names its package pin.
        let device = &model.database().device;
        let mut pads = BTreeMap::new();
        let mut pad_ports = BTreeMap::new();
        for cell in top["cells"]
            .as_object()
            .into_iter()
            .flat_map(|c| c.values())
        {
            if cell["type"] != "TRELLIS_IO" {
                continue;
            }
            let (Some(bel), Some(bit)) = (
                cell["attributes"]["NEXTPNR_BEL"].as_str(),
                cell["connections"]["B"][0].as_u64(),
            ) else {
                continue;
            };
            let site = wire(bel).context("IO cell placement")?;
            let key = format!(
                "{device}/R{}C{}/{}",
                site.location.row, site.location.col, site.name
            );
            let id = model
                .resource_id(&key)
                .with_context(|| format!("IO cell at unknown site {key}"))?;
            let pin = model
                .package_pins()
                .find(|p| p.package == package && p.pio == Some(id))
                .with_context(|| format!("{key} has no ball in {package}"))?;
            pad_ports.insert(bel.to_string(), model.resource_ports(id));
            pads.insert(bit, Reference::Pin(pin.ball.clone()));
        }
        let port_values = port_observations(top, |cell, data| {
            let endpoint = || {
                let ports = pad_ports.get(cell["attributes"]["NEXTPNR_BEL"].as_str()?)?;
                ports
                    .iter()
                    .find(|p| p.name == data)
                    .map(|p| Reference::Wire(p.wire.clone()))
            };
            // An IOLOGIC output can pass through an output delay before the
            // PIO. Its routed source need not be drawn; the selected PIO data
            // endpoint is the wire both the pad driver and overlay consume.
            if data == "IOLDO" {
                if let Some(reference) = endpoint() {
                    return Some(reference);
                }
            }
            // Constants need not have ROUTING. Only the selected data
            // endpoint is eligible as a fallback, never another PIO port.
            cell["connections"][data][0]
                .as_u64()
                .and_then(|bit| references.get(&bit))
                .cloned()
                .or_else(endpoint)
        });
        references.extend(pads.clone());
        let mut nets = BTreeMap::new();
        let mut observations = BTreeMap::new();
        for (name, net) in netnames {
            let bits = net["bits"].as_array().cloned().unwrap_or_default();
            let range = BitRange::from_json(net);
            for (i, bit) in bits.iter().enumerate() {
                let reference = bit
                    .as_u64()
                    .and_then(|bit| references.get(&bit))
                    .cloned()
                    .ok_or_else(|| "no routed source (a constant or a net inside a cell)".into());
                // Keep diagnostics for malformed/ambiguous single-bit routing.
                let reference = if reference.is_err() && bits.len() == 1 {
                    source(net["attributes"]["ROUTING"].as_str().unwrap_or_default())
                        .map(Reference::Wire)
                } else {
                    reference
                };
                let name = range.name(name, bits.len(), i);
                if let Some(observation) = bit.as_u64().and_then(|b| port_values.get(&b)) {
                    observations.insert(name.clone(), observation.clone());
                }
                nets.insert(name, reference);
            }
        }
        for (name, port) in top["ports"].as_object().into_iter().flatten() {
            let bits = port["bits"].as_array().cloned().unwrap_or_default();
            let range = BitRange::from_json(port);
            for (i, bit) in bits.iter().enumerate() {
                let Some(pin) = bit.as_u64().and_then(|b| pads.get(&b)) else {
                    continue;
                };
                let name = range.name(name, bits.len(), i);
                if let Some(observation) = bit.as_u64().and_then(|b| port_values.get(&b)) {
                    observations.insert(name.clone(), observation.clone());
                }
                nets.insert(name, Ok(pin.clone()));
            }
        }
        Ok(Self {
            nets,
            observations,
            sources: Collected::default(),
            creator: json["creator"].as_str().unwrap_or_default().into(),
            placements: top["cells"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, cell)| {
                    Some((
                        name.clone(),
                        cell["attributes"]["NEXTPNR_BEL"].as_str()?.into(),
                    ))
                })
                .collect(),
            clock_names: clock_names(top),
        })
    }
    pub fn clock_name(&self, name: &str) -> Option<&str> {
        self.clock_names.get(name).map(String::as_str)
    }
    /// The same data observation used for Static::net_names. A port's data
    /// reference wins even when unavailable; control nets are never substitutes.
    pub fn overlay_nets(
        &self,
        name: &str,
        model: &DeviceModel,
        package: &str,
        overlay: &Overlay,
    ) -> Vec<u32> {
        let Some(reference) = self.observation(name).and_then(|r| r.as_ref().ok()) else {
            return Vec::new();
        };
        let wires = match reference {
            Reference::Wire(wire) => vec![wire.clone()],
            Reference::Pin(ball) => model
                .package_pin(package, ball)
                .and_then(|p| p.pio)
                .map(|id| {
                    model
                        .resource_ports(id)
                        .into_iter()
                        .map(|p| p.wire)
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        wires
            .iter()
            .filter_map(|w| overlay.value_of(model, w).map(|v| overlay.nets[v as usize]))
            .collect()
    }
    /// One net, or every bit of a bus in ascending HDL index order.
    pub fn expand(&self, name: &str) -> Result<Vec<(String, Reference)>> {
        let resolve = |name: &str, reference: &std::result::Result<Reference, String>| {
            reference
                .clone()
                .map_err(|why| anyhow::anyhow!("net {name}: {why}"))
        };
        if let Some(reference) = self.nets.get(name) {
            return Ok(vec![(name.into(), resolve(name, reference)?)]);
        }
        let prefix = format!("{name}[");
        let mut bits: Vec<(u32, &String, &std::result::Result<Reference, String>)> = self
            .nets
            .iter()
            .filter_map(|(n, r)| {
                let index = n.strip_prefix(&prefix)?.strip_suffix(']')?.parse().ok()?;
                Some((index, n, r))
            })
            .collect();
        if bits.is_empty() {
            let similar: Vec<&String> = self
                .nets
                .keys()
                .filter(|n| n.contains(name) && !n.contains('$'))
                .take(8)
                .collect();
            bail!("unknown net {name}; similar names: {similar:?}");
        }
        bits.sort_by_key(|&(i, _, _)| i);
        bits.into_iter()
            .map(|(_, n, r)| Ok((n.clone(), resolve(n, r)?)))
            .collect()
    }
    fn observation(&self, name: &str) -> Option<&std::result::Result<Reference, String>> {
        self.observations.get(name).or_else(|| self.nets.get(name))
    }

    /// Every physical reference, in name order, as used by `expand`.
    /// Pad references remain package pins, independently of value observation.
    pub fn resolved(&self) -> impl Iterator<Item = (&str, &Reference)> {
        self.nets
            .iter()
            .filter_map(|(name, reference)| Some((name.as_str(), reference.as_ref().ok()?)))
    }

    /// Every value observation, in name order. Only value consumers use this
    /// direction-selected data path; it does not select wires for drawing.
    pub fn observations(&self) -> impl Iterator<Item = (&str, &Reference)> {
        self.nets
            .iter()
            .filter_map(|(name, _)| Some((name.as_str(), self.observation(name)?.as_ref().ok()?)))
    }
    pub fn len(&self) -> usize {
        self.nets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }
}

/// Follow the routed JSON's bit identities through each pad's data connection.
/// I/T/O are different nets: T is never an output value or an input sample.
fn port_observations(
    top: &Value,
    mut resolve: impl FnMut(&Value, &str) -> Option<Reference>,
) -> BTreeMap<u64, std::result::Result<Reference, String>> {
    let mut cells: BTreeMap<u64, Vec<&Value>> = BTreeMap::new();
    for cell in top["cells"]
        .as_object()
        .into_iter()
        .flat_map(|c| c.values())
    {
        if cell["type"] == "TRELLIS_IO" {
            if let Some(bit) = cell["connections"]["B"][0].as_u64() {
                cells.entry(bit).or_default().push(cell);
            }
        }
    }
    let mut observations = BTreeMap::new();
    for port in top["ports"]
        .as_object()
        .into_iter()
        .flat_map(|p| p.values())
    {
        let direction = port["direction"].as_str().unwrap_or_default();
        if !matches!(direction, "input" | "output") {
            continue;
        }
        for bit in port["bits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
        {
            let mut references = Vec::new();
            let mut missing = false;
            for cell in cells.get(&bit).into_iter().flatten() {
                let data = if direction == "input" {
                    "O"
                } else if cell["parameters"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .any(|(name, value)| name.starts_with("DATAMUX_") && value == "IOLDO")
                {
                    "IOLDO"
                } else {
                    "I"
                };
                let reference = resolve(cell, data);
                if let Some(reference) = reference {
                    references.push(reference);
                } else {
                    missing = true;
                }
            }
            references.sort();
            references.dedup();
            let reference = match references.as_slice() {
                [one] if !missing => Ok(one.clone()),
                _ => Err("no unique routed fabric data for port".into()),
            };
            observations.insert(bit, reference);
        }
    }
    observations
}

/// Follow only clock buffers and input pads, using the JSON's bit identities.
/// Generated spelling alone is never evidence of an HDL alias.
fn clock_names(top: &Value) -> BTreeMap<String, String> {
    let mut inputs = BTreeMap::new();
    for cell in top["cells"]
        .as_object()
        .into_iter()
        .flat_map(|c| c.values())
    {
        let (output, input) = match cell["type"].as_str() {
            Some("DCCA") => ("CLKO", "CLKI"),
            Some("TRELLIS_IO") => ("O", "B"),
            _ => continue,
        };
        if let (Some(output), Some(input)) = (
            cell["connections"][output][0].as_u64(),
            cell["connections"][input][0].as_u64(),
        ) {
            inputs.insert(output, input);
        }
    }
    let netnames = top["netnames"].as_object();
    let bits = |net: &Value| net["bits"][0].as_u64();
    let wanted: std::collections::BTreeSet<_> =
        inputs.keys().chain(inputs.values()).copied().collect();
    let visible: BTreeMap<_, _> = netnames
        .into_iter()
        .flatten()
        .filter(|(name, _)| !name.contains('$'))
        .filter_map(|(name, net)| Some((bits(net).filter(|b| wanted.contains(b))?, name)))
        .collect();
    netnames
        .into_iter()
        .flatten()
        .filter_map(|(name, net)| {
            let mut bit = bits(net).filter(|b| inputs.contains_key(b))?;
            let mut seen = std::collections::BTreeSet::new();
            loop {
                if let Some(hdl) = visible.get(&bit) {
                    return Some((name.clone(), (*hdl).clone()));
                }
                if !seen.insert(bit) {
                    return None;
                }
                bit = *inputs.get(&bit)?;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_aliases_follow_buffers_and_pads_without_guessing_names() {
        let top = serde_json::json!({
            "netnames": {
                "$glbnet$unrelated_spelling": {"bits": [3]},
                "intermediate": {"bits": [2]},
                "hdl_clock": {"bits": [1]},
                "$glbnet$hdl_clock": {"bits": [9]}
            },
            "cells": {
                "buffer": {"type": "DCCA", "connections": {"CLKO": [3], "CLKI": [2]}},
                "pad": {"type": "TRELLIS_IO", "connections": {"O": [2], "B": [1]}}
            }
        });
        let aliases = clock_names(&top);
        assert_eq!(aliases["$glbnet$unrelated_spelling"], "intermediate");
        assert_eq!(aliases["intermediate"], "intermediate");
        assert!(!aliases.contains_key("$glbnet$hdl_clock"));
    }

    #[test]
    fn the_source_is_the_routing_wire_without_a_pip() {
        let w = source("X3/Y24/H02E0201;X2/Y24/0_0_Q0->1_0_H02E0201;1;X2/Y24/Q0_SLICE;;1").unwrap();
        assert_eq!(w.location, Location { row: 24, col: 2 });
        assert_eq!(w.name, "Q0_SLICE");
        // A global net lists its branch taps without pips too.
        let w = source("X2/Y21/G_HPBX0100;;1;X0/Y0/G_CLKO_LDCC1;;1;X3/Y20/G_HPBX0100;;1").unwrap();
        assert_eq!(w.name, "G_CLKO_LDCC1");
        assert!(source(" ").unwrap_err().contains("no routed source"));
        assert!(source("X1/Y1/A0;;1;X2/Y2/B0;;1")
            .unwrap_err()
            .contains("several"));
    }
}
