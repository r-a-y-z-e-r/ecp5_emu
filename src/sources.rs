//! Source locations kept by yosys and nextpnr, independent of the window.
//! `Collected` is taken while Names owns the JSON. The overlay index is built
//! later, after ready; it owns no JSON and does no source-file IO.
use crate::names::BitRange;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

/// Only relative project inputs are eligible, including symlinked inputs.
#[derive(Clone, Debug)]
pub struct Files {
    root: PathBuf,
    paths: Option<HashSet<PathBuf>>,
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && !path.to_string_lossy().contains(['\\', ':'])
}

impl Files {
    pub fn project(root: PathBuf, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            root,
            paths: Some(paths.into_iter().filter(|p| relative(p)).collect()),
        }
    }

    /// Config-only designs keep their source copies beside the routed JSON.
    pub fn beside(json: &Path) -> Self {
        Self {
            root: json.parent().unwrap_or(Path::new(".")).into(),
            paths: None,
        }
    }

    fn accepts(&self, path: &Path) -> bool {
        relative(path)
            && self.paths.as_ref().map_or_else(
                || self.root.join(path).is_file(),
                |paths| paths.contains(path),
            )
    }
}

/// Yosys columns and lines are one-based. A missing column covers the line.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub file: Arc<str>,
    pub line: u32,
    pub column: Option<u32>,
    pub end_line: u32,
    pub end_column: Option<u32>,
}

impl Span {
    pub fn label(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }

    pub fn contains(&self, line: u32, column: u32) -> bool {
        (self.line..=self.end_line).contains(&line)
            && (line != self.line || self.column.is_none_or(|c| column >= c))
            && (line != self.end_line || self.end_column.is_none_or(|c| column <= c))
    }

    /// For equal line counts, a later start or earlier end is narrower.
    /// Missing columns cover the whole line and rank after bounded spans.
    fn width(&self) -> (u32, i64) {
        (
            self.end_line - self.line,
            i64::from(self.end_column.unwrap_or(u32::MAX)) - i64::from(self.column.unwrap_or(0)),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Driven,
    Declared,
    Cell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub span: Span,
    pub kind: SourceKind,
}

impl SourceLocation {
    pub fn label(&self) -> String {
        match self.kind {
            SourceKind::Driven => format!("assigned in {}", self.span.label()),
            SourceKind::Declared => format!("declared in {}", self.span.label()),
            SourceKind::Cell => self.span.label(),
        }
    }
}

pub struct Probe {
    pub nets: Vec<u32>,
    pub message: String,
}

impl Probe {
    /// The record uses the same message the editor and status line show.
    pub fn note(&self, file: &str, line: u32, column: u32) -> String {
        format!("probe source {file}:{line}:{column} -> {}", self.message)
    }
}

fn position(text: &str) -> Option<(u32, Option<u32>)> {
    let (line, column) = text
        .split_once('.')
        .map_or((text, None), |(l, c)| (l, Some(c)));
    let line = line.parse::<u32>().ok().filter(|n| *n > 0)?;
    let column = column
        .map(str::parse::<u32>)
        .transpose()
        .ok()?
        .filter(|n| *n > 0);
    Some((line, column))
}

/// Absolute library paths and unknown files are discarded before allocation.
pub fn parse(text: &str, files: &Files) -> Vec<Span> {
    let mut spans = Vec::new();
    for origin in text.split('|') {
        let Some((file, range)) = origin.trim().rsplit_once(':') else {
            continue;
        };
        if !files.accepts(Path::new(file)) {
            continue;
        }
        let (start, end) = range.split_once('-').unwrap_or((range, range));
        let (Some((line, column)), Some((end_line, end_column))) = (position(start), position(end))
        else {
            continue;
        };
        if (end_line, end_column) < (line, column) {
            continue;
        }
        spans.push(Span {
            file: Arc::from(file),
            line,
            column,
            end_line,
            end_column,
        });
    }
    spans.sort_unstable();
    spans.dedup();
    spans
}

struct Net {
    name: String,
    bits: Vec<Option<u64>>,
    range: BitRange,
    hidden: bool,
    declared: Vec<Span>,
}

struct Cell {
    locations: Vec<Span>,
    outputs: Vec<u64>,
    bel: Option<String>,
    pad: Option<u64>,
}

/// Compact inputs from the same Value Names parses; no library locations.
#[derive(Default)]
pub struct Collected {
    nets: Vec<Net>,
    cells: Vec<Cell>,
    pub built_at: Option<SystemTime>,
    pub collect_time: Duration,
}

impl Collected {
    pub fn from_top<'a>(top: &'a Value, files: &Files) -> Self {
        let started = std::time::Instant::now();
        // Generated designs repeat src strings many thousands of times.
        let mut cache: HashMap<&str, Vec<Span>> = HashMap::new();
        let mut locations = |value: &'a Value| {
            let src = value["attributes"]["src"].as_str().unwrap_or_default();
            cache
                .entry(src)
                .or_insert_with(|| parse(src, files))
                .clone()
        };
        let mut nets: Vec<Net> = top["netnames"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, net)| Net {
                name: name.clone(),
                bits: net["bits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(Value::as_u64)
                    .collect(),
                range: BitRange::from_json(net),
                hidden: net["hide_name"].as_u64().unwrap_or(0) != 0 || name.contains('$'),
                declared: locations(net),
            })
            .collect();
        let cells: Vec<Cell> = top["cells"]
            .as_object()
            .into_iter()
            .flat_map(|c| c.values())
            .filter_map(|cell| {
                let locations = locations(cell);
                if locations.is_empty() {
                    return None;
                }
                let outputs = cell["port_directions"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, direction)| direction.as_str() == Some("output"))
                    .flat_map(|(port, _)| {
                        cell["connections"][port].as_array().into_iter().flatten()
                    })
                    .filter_map(Value::as_u64)
                    .collect();
                Some(Cell {
                    locations,
                    outputs,
                    bel: cell["attributes"]["NEXTPNR_BEL"]
                        .as_str()
                        .map(str::to_string),
                    pad: (cell["type"] == "TRELLIS_IO")
                        .then(|| cell["connections"]["B"][0].as_u64())
                        .flatten(),
                })
            })
            .collect();
        // nextpnr moves a top port's src onto its IO cell and leaves its
        // pad net without attributes. Keep that user origin for port clicks.
        let pads: HashMap<_, _> = cells
            .iter()
            .filter_map(|c| Some((c.pad?, &c.locations)))
            .collect();
        for net in &mut nets {
            if net.declared.is_empty() {
                for bit in net.bits.iter().flatten() {
                    if let Some(locations) = pads.get(bit) {
                        net.declared.extend(locations.iter().cloned());
                    }
                }
            }
        }
        Self {
            nets,
            cells,
            built_at: None,
            collect_time: started.elapsed(),
        }
    }

    /// `resolve` uses Names' source wire and overlay.value_of, exactly as
    /// Static::net_names does. Constants and nets inside cells have no entry.
    pub fn build(self, mut resolve: impl FnMut(&str) -> Vec<u32>) -> Index {
        let mut index = Index {
            built_at: self.built_at,
            collect_time: self.collect_time,
            ..Index::default()
        };
        let mut drivers: HashMap<u64, Vec<Span>> = HashMap::new();
        for cell in self.cells {
            for bit in cell.outputs {
                drivers
                    .entry(bit)
                    .or_default()
                    .extend(cell.locations.iter().cloned());
            }
            if let Some(bel) = cell.bel {
                index.sites.entry(bel).or_default().extend(cell.locations);
            }
        }
        for net in self.nets {
            let mut targets = if net.bits.len() == 1 {
                resolve(&net.range.name(&net.name, 1, 0))
            } else {
                resolve(&net.name)
            };
            targets.sort_unstable();
            targets.dedup();
            let mut driven: Vec<Span> = net
                .bits
                .iter()
                .filter_map(|b| b.as_ref().and_then(|b| drivers.get(b)))
                .flatten()
                .cloned()
                .collect();
            driven.sort_unstable();
            driven.dedup();
            if !net.hidden {
                let last = net.name.rsplit('.').next().unwrap_or(&net.name);
                let mut files: Vec<_> = net
                    .declared
                    .iter()
                    .chain(&driven)
                    .map(|s| s.file.clone())
                    .collect();
                files.sort_unstable();
                files.dedup();
                let layout = name_layout(&net, &targets, &mut resolve);
                let mut keys: Vec<&str> = [
                    Some(last),
                    bus_base(last),
                    Some(&net.name),
                    bus_base(&net.name),
                ]
                .into_iter()
                .flatten()
                .collect();
                keys.sort_unstable();
                keys.dedup();
                for file in files {
                    if net.range.is_vector(net.bits.len()) {
                        for bit in &layout.bits {
                            let selected = NameBits {
                                name: layout.name.clone(),
                                bits: vec![bit.clone()],
                            };
                            let local = bit.name.rsplit('.').next().unwrap_or(&bit.name);
                            for key in [local, &bit.name] {
                                let rows = index
                                    .layouts
                                    .entry(file.clone())
                                    .or_default()
                                    .entry(key.into())
                                    .or_default();
                                if !rows.contains(&selected) {
                                    rows.push(selected.clone());
                                }
                            }
                        }
                    }
                    for &key in &keys {
                        let rows = index
                            .layouts
                            .entry(file.clone())
                            .or_default()
                            .entry(key.into())
                            .or_default();
                        if let Some(row) = rows.iter_mut().find(|row| row.name == layout.name) {
                            row.bits.extend(layout.bits.iter().cloned());
                        } else {
                            rows.push(layout.clone());
                        }
                        index
                            .names
                            .entry(file.clone())
                            .or_default()
                            .entry(key.into())
                            .or_default()
                            .extend(&targets);
                    }
                }
            }
            // A known name with no routed source still wins over the line.
            // Its empty entry reports a constant or a net inside a cell,
            // rather than selecting a neighbouring expression by location.
            if targets.is_empty() {
                continue;
            }
            for target in targets {
                let entry = index.nets.entry(target).or_default();
                entry.names.push(net.name.clone());
                entry.declared.extend(net.declared.iter().cloned());
                entry.driven.extend(driven.iter().cloned());
            }
        }
        for (&net, entry) in &mut index.nets {
            entry
                .names
                .sort_by_key(|name| (name.contains('$'), name.clone()));
            entry.names.dedup();
            for spans in [&mut entry.declared, &mut entry.driven] {
                spans.sort_unstable();
                spans.dedup();
            }
            for span in entry.declared.iter().chain(&entry.driven) {
                index.locations += 1;
                for line in span.line..=span.end_line {
                    index
                        .lines
                        .entry(span.file.clone())
                        .or_default()
                        .entry(line)
                        .or_default()
                        .push(net);
                }
            }
        }
        for lines in index.lines.values_mut() {
            for nets in lines.values_mut() {
                nets.sort_unstable();
                nets.dedup();
            }
        }
        for names in index.names.values_mut() {
            for nets in names.values_mut() {
                nets.sort_unstable();
                nets.dedup();
            }
        }
        for names in index.layouts.values_mut() {
            for rows in names.values_mut() {
                rows.sort_by(|a, b| a.name.cmp(&b.name));
                for row in rows {
                    let mut bits: BTreeMap<Option<u32>, NameBit> = BTreeMap::new();
                    for bit in row.bits.drain(..) {
                        bits.entry(bit.index)
                            .or_insert_with(|| NameBit {
                                name: bit.name.clone(),
                                index: bit.index,
                                targets: Vec::new(),
                            })
                            .targets
                            .extend(bit.targets);
                    }
                    if let (Some(Some(low)), Some(Some(high))) = (
                        bits.keys().next().copied(),
                        bits.keys().next_back().copied(),
                    ) {
                        for index in low..=high {
                            bits.entry(Some(index)).or_insert_with(|| NameBit {
                                name: format!("{}[{index}]", row.name),
                                index: Some(index),
                                targets: Vec::new(),
                            });
                        }
                    }
                    row.bits = bits
                        .into_values()
                        .rev()
                        .map(|mut bit| {
                            bit.targets.sort_unstable();
                            bit.targets.dedup();
                            bit
                        })
                        .collect();
                }
            }
        }
        for spans in index.sites.values_mut() {
            spans.sort_unstable();
            spans.dedup();
        }
        index
    }
}

/// A name's layout before route resolution, in descending numeric bit order.
/// Rows keep qualified instance names; a bit may have several physical targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameBits {
    pub name: String,
    pub bits: Vec<NameBit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameBit {
    pub name: String,
    pub index: Option<u32>,
    pub targets: Vec<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameLookup<'a> {
    Absent,
    Unrouted(&'a [NameBits]),
    Resolved(&'a [NameBits]),
}

fn name_layout(net: &Net, targets: &[u32], resolve: &mut impl FnMut(&str) -> Vec<u32>) -> NameBits {
    let indexed = net
        .name
        .strip_suffix(']')
        .and_then(|n| n.rsplit_once('['))
        .and_then(|(base, index)| Some((base, index.parse::<u32>().ok()?)));
    let bus = indexed.is_some() || net.range.is_vector(net.bits.len());
    let name = indexed
        .map_or(net.name.as_str(), |(base, _)| base)
        .to_string();
    let bits = (0..net.bits.len().max(1))
        .map(|i| {
            let index = bus.then(|| {
                indexed.map_or_else(|| net.range.index(net.bits.len(), i), |(_, index)| index)
            });
            let bit_name = index.map_or_else(|| name.clone(), |i| format!("{name}[{i}]"));
            let targets = if net.bits.len() <= 1 {
                targets.to_vec()
            } else {
                resolve(&bit_name)
            };
            NameBit {
                name: bit_name,
                index,
                targets,
            }
        })
        .collect();
    NameBits { name, bits }
}

fn bus_base(name: &str) -> Option<&str> {
    let (base, bit) = name.strip_suffix(']')?.rsplit_once('[')?;
    bit.parse::<u32>().ok()?;
    Some(base)
}

#[derive(Default, Debug)]
pub struct NetSources {
    pub names: Vec<String>,
    pub declared: Vec<Span>,
    pub driven: Vec<Span>,
}

#[derive(Default)]
pub struct Index {
    pub nets: HashMap<u32, NetSources>,
    names: HashMap<Arc<str>, HashMap<String, Vec<u32>>>,
    layouts: HashMap<Arc<str>, HashMap<String, Vec<NameBits>>>,
    lines: HashMap<Arc<str>, BTreeMap<u32, Vec<u32>>>,
    sites: HashMap<String, Vec<Span>>,
    pub locations: usize,
    pub built_at: Option<SystemTime>,
    pub collect_time: Duration,
}

impl Index {
    /// Name only: never falls back to a nearby source location. Empty routed
    /// targets remain distinct from names absent from the built design.
    pub fn name_only(&self, file: &str, name: &str) -> NameLookup<'_> {
        let Some(rows) = self.layouts.get(file).and_then(|names| names.get(name)) else {
            return NameLookup::Absent;
        };
        if rows
            .iter()
            .flat_map(|row| &row.bits)
            .any(|bit| !bit.targets.is_empty())
        {
            NameLookup::Resolved(rows)
        } else {
            NameLookup::Unrouted(rows)
        }
    }

    /// The token wins even if its declaration is on a different line.
    /// An unmatched token only considers spans starting on the clicked line:
    /// the narrowest covering the column, else the narrowest on that line.
    pub fn resolve(&self, file: &str, line: u32, column: u32, identifier: &str) -> Vec<u32> {
        if let Some(nets) = self.names.get(file).and_then(|n| n.get(identifier)) {
            return nets.clone();
        }
        let candidates = self
            .lines
            .get(file)
            .and_then(|lines| lines.get(&line))
            .into_iter()
            .flatten()
            .flat_map(|net| {
                let entry = &self.nets[net];
                entry
                    .declared
                    .iter()
                    .chain(&entry.driven)
                    .filter(move |s| s.file.as_ref() == file && s.line == line)
                    .map(move |span| (*net, span))
            });
        let rank = |span: &Span| (!span.contains(line, column), span.width());
        let narrowest = candidates.clone().map(|(_, span)| rank(span)).min();
        let mut nets: Vec<_> = candidates
            .filter(|(_, span)| Some(rank(span)) == narrowest)
            .map(|(net, _)| net)
            .collect();
        nets.sort_unstable();
        nets.dedup();
        nets
    }

    pub fn probe(&self, file: &str, line: u32, column: u32, identifier: &str) -> Probe {
        let nets = self.resolve(file, line, column, identifier);
        let message = if !nets.is_empty() {
            self.summary(&nets)
        } else if self
            .names
            .get(file)
            .is_some_and(|names| names.contains_key(identifier))
        {
            format!("`{identifier}` has no net on the chip (a constant, or inside a cell)")
        } else {
            format!("no net at {file}:{line}:{column}")
        };
        Probe { nets, message }
    }

    pub fn net_label(&self, net: u32) -> String {
        let names = self.nets.get(&net).map_or(&[][..], |n| n.names.as_slice());
        match names {
            [] => format!("net {net}"),
            [name] => format!("net {net} ({name})"),
            [name, rest @ ..] => format!("net {net} ({name} +{})", rest.len()),
        }
    }

    /// Preserve the default driven location, followed by declarations.
    /// Origins on the same line share a choice within each kind.
    pub fn net_source_choices(&self, net: u32) -> Vec<SourceLocation> {
        let Some(entry) = self.nets.get(&net) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        [
            (SourceKind::Driven, &entry.driven),
            (SourceKind::Declared, &entry.declared),
        ]
        .into_iter()
        .flat_map(|(kind, spans)| spans.iter().map(move |span| (kind, span)))
        .filter(|(kind, span)| seen.insert((*kind, span.file.clone(), span.line)))
        .map(|(kind, span)| SourceLocation {
            kind,
            span: span.clone(),
        })
        .collect()
    }

    pub fn net_locations(&self, net: u32) -> &[Span] {
        self.nets.get(&net).map_or(&[], |n| {
            if n.driven.is_empty() {
                &n.declared
            } else {
                &n.driven
            }
        })
    }

    /// Placement names are Xcol/Yrow/SLICEA.K0, .K1 or .FF0. Slice menus
    /// include all cells in that slice; LUT/FF menus choose the part.
    pub fn site_locations(&self, row: u16, col: u16, label: &str) -> Vec<Span> {
        let prefix = format!("X{col}/Y{row}/");
        let part = label
            .strip_prefix("LUT")
            .and_then(|n| n.parse::<u32>().ok())
            .map(|n| format!("SLICE{}.K{}", (b'A' + (n / 2) as u8) as char, n % 2))
            .or_else(|| {
                label
                    .strip_prefix("FF")
                    .and_then(|n| n.parse::<u32>().ok())
                    .map(|n| format!("SLICE{}.FF{}", (b'A' + (n / 2) as u8) as char, n % 2))
            })
            .unwrap_or_else(|| label.to_string());
        let mut found: Vec<_> = self
            .sites
            .iter()
            .filter(|(bel, _)| {
                bel.strip_prefix(&prefix).is_some_and(|site| {
                    site == part || site.strip_prefix(&part).is_some_and(|s| s.starts_with('.'))
                })
            })
            .flat_map(|(_, spans)| spans.iter().cloned())
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    pub fn summary(&self, nets: &[u32]) -> String {
        let names: Vec<_> = nets
            .iter()
            .flat_map(|net| self.nets.get(net).into_iter().flat_map(|n| &n.names))
            .map(String::as_str)
            .collect();
        format!("{} nets: {}", nets.len(), names.join(", "))
    }
}

/// The editor and scripts use the same one-based character coordinates.
/// Constant bit selects belong to the identifier; numeric literals do not.
pub fn identifier_at(text: &str, line: u32, column: u32) -> String {
    let Some(line) = text.lines().nth(line.saturating_sub(1) as usize) else {
        return String::new();
    };
    let chars: Vec<char> = line.chars().collect();
    let at = column.saturating_sub(1) as usize;
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        if chars[i] == '\\' {
            i += 1;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
        } else if chars[i].is_ascii_alphabetic() || matches!(chars[i], '_' | '$') {
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '$'))
            {
                i += 1;
            }
            if chars.get(i) == Some(&'[') {
                let mut end = i + 1;
                while chars.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                if end > i + 1 && chars.get(end) == Some(&']') {
                    i = end + 1;
                }
            }
        } else {
            i += 1;
            continue;
        }
        if (start..i).contains(&at) {
            return chars[start..i]
                .iter()
                .collect::<String>()
                .trim_start_matches('\\')
                .into();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn files() -> Files {
        Files::project(
            PathBuf::from("project"),
            ["top.v", "picorv32.v", "rtl/a file.v"].map(PathBuf::from),
        )
    }

    fn fixture(design: &str) -> Option<Value> {
        let root = if design == "life_full" {
            crate::test_designs::reference_design(design)
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/reference")
                .join(design)
        };
        let path = root.join("top_routed.json");
        if !path.is_file() {
            eprintln!("reference JSON unavailable: {design}");
            return None;
        }
        let json: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        Some(
            json["modules"]
                .as_object()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .clone(),
        )
    }

    /// Identity bits stand in for overlay nets here; actual wire resolution
    /// is exercised by the headless live test in live.rs.
    fn index(top: &Value) -> Index {
        Collected::from_top(top, &files()).build(|name| {
            top["netnames"][name]["bits"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|b| b.as_u64().map(|b| b as u32))
                .collect()
        })
    }

    #[test]
    fn name_only_preserves_gaps_empty_names_aliases_and_never_uses_locations() {
        let top = json!({"netnames": {
            "data[8]": {"bits":[1],"attributes":{"src":"top.v:2.1-2.20"}},
            "data[6]": {"bits":["0"],"attributes":{"src":"top.v:2.1-2.20"}},
            "data[2]": {"bits":[2],"attributes":{"src":"top.v:2.1-2.20"}},
            "alias": {"bits":[1],"attributes":{"src":"top.v:2.1-2.20"}}
        }});
        let i = Collected::from_top(&top, &files()).build(|name| match name {
            "data[8]" | "alias" => vec![900],
            "data[2]" => vec![5, 4, 5],
            _ => vec![],
        });
        let NameLookup::Resolved(rows) = i.name_only("top.v", "data") else {
            panic!("resolved bus")
        };
        assert_eq!(rows.len(), 1);
        let bits = &rows[0].bits;
        assert_eq!(
            bits.iter().map(|b| b.index.unwrap()).collect::<Vec<_>>(),
            [8, 7, 6, 5, 4, 3, 2]
        );
        assert_eq!(bits[0].targets, [900]);
        assert_eq!(bits[6].targets, [4, 5]);
        assert_eq!(
            bits.iter()
                .filter(|b| b.targets.is_empty())
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["data[7]", "data[6]", "data[5]", "data[4]", "data[3]"]
        );
        assert!(matches!(
            i.name_only("top.v", "data[6]"),
            NameLookup::Unrouted(_)
        ));
        assert!(matches!(
            i.name_only("top.v", "alias"),
            NameLookup::Resolved(_)
        ));
        assert_eq!(i.name_only("top.v", "removed"), NameLookup::Absent);
        assert_eq!(i.name_only("picorv32.v", "data"), NameLookup::Absent);
        assert!(
            !i.resolve("top.v", 2, 5, "removed").is_empty(),
            "clicks retain their location fallback"
        );
    }

    #[test]
    fn name_only_bus_split_across_instances_keeps_every_prefix_and_local_alias() {
        let top = json!({"netnames": {
            "a.data[3]": {"bits":[40],"attributes":{"src":"top.v:2"}},
            "a.data[1]": {"bits":[30],"attributes":{"src":"top.v:2"}},
            "b.data[3]": {"bits":[20],"attributes":{"src":"top.v:2"}},
            "b.data[1]": {"bits":[10],"attributes":{"src":"top.v:2"}},
            "a.alias[3]": {"bits":[40],"attributes":{"src":"top.v:2"}}
        }});
        let i = index(&top);
        let NameLookup::Resolved(rows) = i.name_only("top.v", "data") else {
            panic!("bus")
        };
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a.data", "b.data"]
        );
        for (row, expected) in rows.iter().zip([[40, 30], [20, 10]]) {
            assert_eq!(row.bits.len(), 3);
            assert_eq!(row.bits[0].targets, [expected[0]]);
            assert!(row.bits[1].targets.is_empty());
            assert_eq!(row.bits[1].name, format!("{}[2]", row.name));
            assert_eq!(row.bits[2].targets, [expected[1]]);
        }
        let NameLookup::Resolved(bits) = i.name_only("top.v", "data[1]") else {
            panic!("select")
        };
        assert_eq!(bits.len(), 2);
        assert!(bits
            .iter()
            .all(|row| row.bits.len() == 1 && row.bits[0].index == Some(1)));
        let NameLookup::Resolved(qualified) = i.name_only("top.v", "a.data") else {
            panic!("qualified")
        };
        assert_eq!(qualified, &rows[..1]);
        assert!(matches!(
            i.name_only("top.v", "alias"),
            NameLookup::Resolved(_)
        ));
    }

    #[test]
    fn raw_nextpnr_bit_arrays_keep_constants_offsets_and_direction() {
        let top = json!({"netnames": {
            "array": {"bits":[7,"0",8,"x"],"offset":5,"attributes":{"src":"top.v:1"}},
            "ascending": {"bits":[9,"1",10],"offset":2,"upto":1,"attributes":{"src":"top.v:1"}}
        }});
        let i = Collected::from_top(&top, &files()).build(|name| match name {
            "array[5]" => vec![70],
            "array[7]" => vec![80],
            "ascending[4]" => vec![90],
            "ascending[2]" => vec![100],
            _ => vec![],
        });
        let NameLookup::Resolved(rows) = i.name_only("top.v", "array") else {
            panic!("array")
        };
        assert_eq!(
            rows[0]
                .bits
                .iter()
                .map(|b| b.index.unwrap())
                .collect::<Vec<_>>(),
            [8, 7, 6, 5]
        );
        assert!(rows[0].bits[0].targets.is_empty());
        assert!(rows[0].bits[2].targets.is_empty());
        let NameLookup::Resolved(rows) = i.name_only("top.v", "ascending") else {
            panic!("ascending")
        };
        assert_eq!(rows[0].bits[0].targets, [90]);
        assert!(rows[0].bits[1].targets.is_empty());
        assert_eq!(rows[0].bits[2].targets, [100]);
    }

    #[test]
    fn only_project_paths_survive_including_spaces_and_line_only_forms() {
        let spans = parse("top.v:5.42-5.45|/share/yosys/map.v:1.1-9.2|unknown.v:3|../top.v:1|C:\\map.v:2|rtl/a file.v:8-9|picorv32.v:12", &files());
        assert_eq!(spans.len(), 3);
        assert!(spans
            .iter()
            .all(|s| !Path::new(s.file.as_ref()).is_absolute()));
        let span = spans.iter().find(|s| s.file.as_ref() == "top.v").unwrap();
        assert!(span.contains(5, 42));
        assert!(!span.contains(5, 41));
        assert!(!span.contains(6, 42));
        assert!(parse("top.v:0|top.v:bad|top.v:3.9-2.1", &files()).is_empty());
        assert_eq!(parse("top.v:3|top.v:3", &files()).len(), 1);
        assert!(spans.iter().find(|s| s.line == 8).unwrap().contains(9, 100));
    }

    #[test]
    #[cfg(unix)]
    fn config_only_sources_keep_relative_symlink_names() {
        let root = std::env::temp_dir().join(format!("ecp5_source_files_{}", std::process::id()));
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(root.join("library.v"), "module cpu; endmodule").unwrap();
        std::os::unix::fs::symlink("../library.v", project.join("picorv32.v")).unwrap();
        let spans = parse(
            "picorv32.v:159.13-159.19|missing.v:1|/share/yosys/map.v:2",
            &Files::beside(&project.join("top.config")),
        );
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].label(), "picorv32.v:159");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn token_wins_over_the_assignment_span_and_unknown_tokens_fall_back() {
        let top = json!({"netnames": {
            "a": {"bits":[1], "attributes":{"src":"top.v:1.6-1.7"}},
            "b": {"bits":[2], "attributes":{"src":"top.v:2.6-2.7"}},
            "c": {"bits":[3], "attributes":{"src":"top.v:2.9-2.10"}},
            "$hidden": {"bits":[4], "hide_name":1, "attributes":{"src":"top.v:5.1-5.20"}},
            "inside": {"bits":[5], "attributes":{"src":"top.v:6"}},
            "constant": {"bits":[6], "attributes":{"src":"top.v:4.1-4.18"}}
        }, "cells": {"lut": {"attributes":{"src":"top.v:4.1-4.18|/share/yosys/map.v:1"}, "port_directions":{"Y":"output","A":"input"}, "connections":{"Y":[1],"A":[2,3]}}}});
        let i = Collected::from_top(&top, &files()).build(|name| {
            if matches!(name, "inside" | "constant") {
                vec![]
            } else {
                vec![top["netnames"][name]["bits"][0].as_u64().unwrap() as u32]
            }
        });
        let text = "wire a;\nwire b, c;\n\nassign a = b & c;";
        assert_eq!(identifier_at(text, 4, 12), "b");
        assert_eq!(i.resolve("top.v", 4, 12, "b"), [2]);
        assert_eq!(i.resolve("top.v", 4, 1, "assign"), [1]);
        assert!(i.resolve("top.v", 4, 12, "constant").is_empty());
        assert_eq!(i.resolve("top.v", 4, 20, "unmatched"), [1]);
        assert_eq!(i.resolve("top.v", 5, 3, "hidden"), [4]);
        assert!(i.resolve("top.v", 9, 1, "$hidden").is_empty());
        assert!(i.resolve("top.v", 6, 1, "inside").is_empty());
        assert_eq!(
            i.probe("top.v", 9, 1, "gone").message,
            "no net at top.v:9:1"
        );
        assert_eq!(i.net_locations(1)[0].line, 4);
        assert!(i.nets[&2].driven.is_empty(), "input ports are not drivers");
    }

    #[test]
    fn buses_aliases_instances_and_files_are_disambiguated() {
        let top = json!({"netnames": {
            "one.data[0]": {"bits":[1],"attributes":{"src":"top.v:2"}},
            "one.data[1]": {"bits":[2],"attributes":{"src":"top.v:2"}},
            "two.data[0]": {"bits":[3],"attributes":{"src":"top.v:2"}},
            "cpu.data[0]": {"bits":[4],"attributes":{"src":"picorv32.v:2"}},
            "alias": {"bits":[1],"attributes":{"src":"top.v:3"}},
            "row[26].col[3].n": {"bits":[5],"attributes":{"src":"top.v:5"}}
        }});
        let i = index(&top);
        assert_eq!(i.resolve("top.v", 20, 1, "data"), [1, 2, 3]);
        assert_eq!(i.resolve("top.v", 20, 1, "data[0]"), [1, 3]);
        assert_eq!(i.resolve("picorv32.v", 20, 1, "data"), [4]);
        assert_eq!(i.nets[&1].names, ["alias", "one.data[0]"]);
        assert_eq!(i.resolve("top.v", 20, 1, "n"), [5]);
        assert_eq!(identifier_at("assign x = data[12];", 1, 18), "data[12]");
        assert_eq!(identifier_at("assign x = 42;", 1, 12), "");
    }

    #[test]
    fn blinky_count_has_eight_declared_nets() {
        let Some(top) = fixture("blinky_cycle") else {
            return;
        };
        let i = index(&top);
        let nets = i.resolve("top.v", 1, 1, "count");
        assert_eq!(nets.len(), 8);
        assert!(nets.iter().all(|n| i.nets[n]
            .declared
            .iter()
            .any(|s| s.line == 5 && s.file.as_ref() == "top.v")));
        assert_eq!(i.resolve("top.v", 1, 1, "count[2]").len(), 1);
        assert!(i.resolve("top.v", 1000, 1, "optimized_away").is_empty());
    }

    #[test]
    fn picorv32_file_only_matches_cpu_and_renamed_ff_is_found_by_location() {
        let Some(top) = fixture("picorv32_console") else {
            return;
        };
        let i = index(&top);
        let nets = i.resolve("picorv32.v", 1, 1, "reg_pc");
        assert!(!nets.is_empty());
        assert!(nets
            .iter()
            .all(|n| i.nets[n].names.iter().any(|n| n.starts_with("cpu."))));
        assert!(i.resolve("top.v", 1, 1, "reg_pc").is_empty());
        let (&net, entry) = i
            .nets
            .iter()
            .find(|(net, n)| {
                n.names.iter().any(|n| n.contains("_TRELLIS_FF_Q"))
                    && n.declared.is_empty()
                    && n.driven.iter().any(|s| {
                        i.resolve(&s.file, s.line, s.column.unwrap_or(1), "unmatched")
                            .contains(net)
                    })
            })
            .unwrap();
        assert!(entry.driven.iter().any(|s| i
            .resolve(&s.file, s.line, s.column.unwrap_or(1), "unmatched")
            .contains(&net)));
        assert!(i
            .nets
            .values()
            .flat_map(|n| n.declared.iter().chain(&n.driven))
            .all(|s| !s.file.starts_with('/')));
    }

    #[test]
    fn switches_sw_and_led_keep_the_ports_io_origins() {
        let Some(top) = fixture("switches") else {
            return;
        };
        let i = index(&top);
        assert_eq!(i.resolve("top.v", 1000, 1, "sw").len(), 4);
        // led[7:4] are constants; nextpnr gives their cells no user origin.
        assert_eq!(i.resolve("top.v", 1000, 1, "led").len(), 4);
    }

    #[test]
    fn site_locations_use_the_placed_cell_and_slice_parts() {
        let top = json!({"cells": {
            "lut":{"attributes":{"src":"top.v:12.3-12.8","NEXTPNR_BEL":"X2/Y3/SLICEB.K1"}},
            "ff":{"attributes":{"src":"top.v:14","NEXTPNR_BEL":"X2/Y3/SLICEB.FF1"}}
        }});
        let i = Collected::from_top(&top, &files()).build(|_| vec![]);
        assert_eq!(i.site_locations(3, 2, "SLICEB").len(), 2);
        assert_eq!(i.site_locations(3, 2, "LUT3")[0].line, 12);
        assert_eq!(i.site_locations(3, 2, "FF3")[0].line, 14);
        assert!(i.site_locations(3, 2, "SLICEA").is_empty());
    }

    #[test]
    fn location_fallback_keeps_only_the_narrowest_covering_spans() {
        let top = json!({"netnames": {
            "process": {"bits":[1],"attributes":{"src":"picorv32.v:1374.2-1946.5"}},
            "statement": {"bits":[2],"attributes":{"src":"picorv32.v:1504.5-1504.30"}},
            "expression": {"bits":[3],"attributes":{"src":"picorv32.v:1504.9-1504.14"}},
            "alias": {"bits":[4],"attributes":{"src":"picorv32.v:1504.9-1504.14"}},
            "elsewhere": {"bits":[5],"attributes":{"src":"picorv32.v:1504.18-1504.20"}},
            "short_process": {"bits":[6],"attributes":{"src":"picorv32.v:1503.5-1505.10"}},
            "other_file": {"bits":[7],"attributes":{"src":"top.v:1504.11-1504.13"}}
        }});
        let i = index(&top);
        assert_eq!(i.resolve("picorv32.v", 1504, 12, ""), [3, 4]);
        assert_eq!(i.resolve("picorv32.v", 1504, 25, ""), [2]);
        assert_eq!(i.resolve("picorv32.v", 1504, 40, ""), [5]);
        assert!(i.resolve("picorv32.v", 1490, 12, "").is_empty());
        assert_eq!(i.resolve("picorv32.v", 1504, 12, "elsewhere"), [5]);
        assert_eq!(i.resolve("top.v", 1504, 12, ""), [7]);
    }

    #[test]
    fn blinky_declaration_and_always_keywords_probe_count() {
        // These are the source coordinates of the current editor buffer;
        // the archived reference JSON predates its added comment line.
        let mut top = json!({"netnames":{},"cells":{
            "ff":{"attributes":{"src":"top.v:7.3-7.50"},"port_directions":{"Q":"output"},"connections":{"Q":[1,2,3,4,5,6,7,8]}},
            "lut":{"attributes":{"src":"top.v:7.40-7.49"},"port_directions":{"F":"output"},"connections":{"F":[9]}}
        }});
        for bit in 0..8 {
            top["netnames"][format!("count[{bit}]")] =
                json!({"bits":[bit+1],"attributes":{"src":"top.v:6.13-6.18"}});
        }
        top["netnames"]["$mapped"] = json!({"bits":[9],"hide_name":1});
        let i = index(&top);
        let count = i.resolve("top.v", 100, 1, "count");
        assert_eq!(count.len(), 8);
        assert_eq!(i.resolve("top.v", 6, 3, "reg"), count);
        assert_eq!(i.resolve("top.v", 7, 3, "always"), count);
        assert_eq!(i.resolve("top.v", 7, 43, ""), [9]);
    }

    #[test]
    fn multiline_process_fallback_matches_its_header_and_names_still_match_in_its_body() {
        let top = json!({"netnames":{
            "state":{"bits":[1],"attributes":{"src":"top.v:2.7-2.12"}}
        },"cells":{
            "ff":{"attributes":{"src":"top.v:10.3-20.6"},"port_directions":{"Q":"output"},"connections":{"Q":[1]}}
        }});
        let i = index(&top);
        assert_eq!(i.resolve("top.v", 10, 3, "always"), [1]);
        assert!(i.resolve("top.v", 15, 12, "").is_empty());
        assert_eq!(i.resolve("top.v", 15, 12, "state"), [1]);
    }

    #[test]
    fn picorv32_process_body_has_no_location_fallback_on_the_reference_json() {
        let Some(top) = fixture("picorv32_console") else {
            return;
        };
        let i = index(&top);
        assert!(
            i.nets.values().flat_map(|n| &n.driven).any(|s| {
                s.file.as_ref() == "picorv32.v" && s.line < 1504 && s.end_line >= 1504
            }),
            "the reference contains process spans covering the clicked body line"
        );
        let probe = i.probe("picorv32.v", 1504, 12, "");
        assert!(probe.nets.is_empty());
        assert_eq!(probe.message, "no net at picorv32.v:1504:12");
        assert!(!i.resolve("picorv32.v", 1504, 12, "reg_pc").is_empty());
    }

    #[test]
    fn probe_messages_distinguish_known_unrouted_names_and_empty_locations() {
        let top = json!({"netnames":{
            "inside":{"bits":[1],"attributes":{"src":"top.v:2.10-2.20"}}
        }});
        let i = Collected::from_top(&top, &files()).build(|_| vec![]);
        let known = i.probe("top.v", 6, 3, "inside");
        assert_eq!(
            known.message,
            "`inside` has no net on the chip (a constant, or inside a cell)"
        );
        assert!(known.note("top.v", 6, 3).ends_with(&known.message));
        for identifier in ["reg", "comment_word", ""] {
            let unknown = i.probe("top.v", 6, 3, identifier);
            assert_eq!(unknown.message, "no net at top.v:6:3");
            assert_eq!(
                unknown.note("top.v", 6, 3),
                "probe source top.v:6:3 -> no net at top.v:6:3"
            );
        }
        assert_eq!(
            i.probe("picorv32.v", 6, 3, "inside").message,
            "no net at picorv32.v:6:3"
        );
    }

    #[test]
    fn net_labels_and_source_choices_keep_aliases_and_both_location_kinds() {
        let top = json!({"netnames":{
            "cpu.reg_pc[8]":{"bits":[8],"attributes":{"src":"picorv32.v:159.13-159.19"}},
            "z_alias":{"bits":[8],"attributes":{"src":"picorv32.v:159.20-159.28"}}
        },"cells":{
            "ff":{"attributes":{"src":"picorv32.v:1374.2-1946.5"},"port_directions":{"Q":"output"},"connections":{"Q":[8]}}
        }});
        let i = index(&top);
        assert_eq!(i.net_label(8), "net 8 (cpu.reg_pc[8] +1)");
        assert_eq!(i.net_label(9), "net 9");
        let choices = i.net_source_choices(8);
        assert_eq!(
            choices
                .iter()
                .map(SourceLocation::label)
                .collect::<Vec<_>>(),
            ["assigned in picorv32.v:1374", "declared in picorv32.v:159"]
        );
        assert_eq!(choices[0].span, i.net_locations(8)[0]);
    }

    /// Isolates the added load pass from JSON parsing, using the same Value.
    #[test]
    #[ignore = "release timing on the local life_full routed JSON"]
    fn life_full_collection_timing() {
        let top = fixture("life_full").unwrap();
        let files = files();
        for _ in 0..3 {
            let collected = Collected::from_top(&top, &files);
            println!(
                "# probe load-pass {:.6}",
                collected.collect_time.as_secs_f64()
            );
            let started = std::time::Instant::now();
            let i = collected.build(|name| {
                top["netnames"][name]["bits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|b| b.as_u64().map(|b| b as u32))
                    .collect()
            });
            println!(
                "# probe index-pure {:.6} {} {}",
                started.elapsed().as_secs_f64(),
                i.nets.len(),
                i.locations
            );
        }
    }
}
