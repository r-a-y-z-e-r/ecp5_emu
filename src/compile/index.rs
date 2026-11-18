//! Reverse route lookup using shared templates, not 28 million placed edges.
use crate::model::{
    topology::{Endpoint, Location, Wire, WirePattern},
    DeviceModel, TileId,
};
use anyhow::Result;
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet},
};

pub(super) fn canonical_ref<'a>(model: &DeviceModel, wire: &'a Wire) -> Result<Cow<'a, Wire>> {
    Ok(match model.clock_segment(wire)? {
        Some(segment) => Cow::Owned(segment),
        None => Cow::Borrowed(wire),
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct WireId(pub(super) usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NameId(usize);
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct WireKey {
    location: Location,
    name: NameId,
}
/// The model numbers resources and PLC nodes, but has no wire numbering.
/// Intern names separately so repeated locations hash only compact keys.
#[derive(Default)]
struct Wires {
    names: HashMap<String, NameId>,
    name_text: Vec<String>,
    name_keys: Vec<NameId>,
    ids: HashMap<WireKey, WireId>,
    keys: Vec<WireKey>,
    canonical: Vec<Option<WireId>>,
}
impl Wires {
    fn name(&mut self, name: &str) -> NameId {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = NameId(self.name_text.len());
        self.names.insert(name.to_owned(), id);
        self.name_text.push(name.to_owned());
        self.name_keys.push(id);
        if let Cow::Owned(key) = name_key(name) {
            self.name_keys[id.0] = self.name(&key);
        }
        id
    }
    fn intern(&mut self, wire: &Wire) -> WireId {
        let key = WireKey {
            location: wire.location,
            name: self.name(&wire.name),
        };
        *self.ids.entry(key).or_insert_with(|| {
            let id = WireId(self.keys.len());
            self.keys.push(key);
            self.canonical.push(None);
            id
        })
    }
    fn wire(&self, id: WireId) -> Wire {
        let key = self.keys[id.0];
        Wire {
            location: key.location,
            name: self.name_text[key.name.0].clone(),
        }
    }
}
fn name_key(name: &str) -> Cow<'_, str> {
    if name.contains("PCSA") {
        Cow::Owned(name.replacen("PCSA", "PCSB", 1))
    } else {
        Cow::Borrowed(name)
    }
}
struct Template {
    kind: String,
    raw: String,
    pattern: WirePattern,
    constant: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Incoming {
    pub tile: TileId,
    pub raw: String,
    pub constant: Option<String>,
}
pub(super) struct Index {
    // Interior mutability only interns identities; lookup/planning order stays
    // with the caller, and interned wires never create kernel signals.
    wires: RefCell<Wires>,
    templates: Vec<Template>,
    by_name: HashMap<NameId, Vec<usize>>,
    by_kind: HashMap<String, Vec<TileId>>,
    by_location: HashMap<Location, Vec<TileId>>,
    clocks: HashMap<WireId, Vec<Incoming>>,
    tile_order: HashMap<Location, usize>,
}
impl Index {
    pub fn new(model: &DeviceModel) -> Result<Self> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/index::new");
        let mut result = Self {
            wires: RefCell::new(Wires::default()),
            templates: Vec::new(),
            by_name: HashMap::new(),
            by_kind: HashMap::new(),
            by_location: HashMap::new(),
            clocks: HashMap::new(),
            tile_order: HashMap::new(),
        };
        for (order, (id, tile)) in model.tiles().enumerate() {
            result
                .tile_order
                .entry(Location {
                    row: tile.row,
                    col: tile.col,
                })
                .or_insert(order);
            result
                .by_kind
                .entry(tile.kind.clone())
                .or_default()
                .push(id);
            result
                .by_location
                .entry(Location {
                    row: tile.row,
                    col: tile.col,
                })
                .or_default()
                .push(id);
        }
        for (kind, tt) in &model.database().types {
            let sinks =
                tt.incoming
                    .keys()
                    .map(|s| (s.clone(), None))
                    .chain(tt.enums.keys().filter_map(|name| {
                        name.strip_prefix("CIB.")
                            .and_then(|s| s.strip_suffix("MUX"))
                            .map(|s| (s.to_string(), Some(name.clone())))
                    }));
            for (raw, constant) in sinks {
                let Some(pattern) = WirePattern::parse(&raw)? else {
                    continue;
                };
                if ["G_VPTX", "G_HPRX", "G_HPBX"]
                    .iter()
                    .any(|p| pattern.name.starts_with(p))
                {
                    for &tile in &result.by_kind[kind] {
                        let t = model.tile(tile);
                        if let Endpoint::Wire(wire) = pattern.place(
                            model.database(),
                            Location {
                                row: t.row,
                                col: t.col,
                            },
                        )? {
                            let id = result.canonical(model, &wire)?;
                            result.clocks.entry(id).or_default().push(Incoming {
                                tile,
                                raw: raw.clone(),
                                constant: constant.clone(),
                            });
                        }
                    }
                } else {
                    let index = result.templates.len();
                    let name = result
                        .wires
                        .get_mut()
                        .name(name_key(&pattern.name).as_ref());
                    result.by_name.entry(name).or_default().push(index);
                    result.templates.push(Template {
                        kind: kind.clone(),
                        raw,
                        pattern,
                        constant,
                    });
                }
            }
        }
        Ok(result)
    }
    pub fn id(&self, wire: &Wire) -> WireId {
        self.wires.borrow_mut().intern(wire)
    }
    pub fn canonical(&self, model: &DeviceModel, wire: &Wire) -> Result<WireId> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/index::canonical");
        let mut wires = self.wires.borrow_mut();
        let raw = wires.intern(wire);
        if let Some(id) = wires.canonical[raw.0] {
            return Ok(id);
        }
        let id = match model.clock_segment(wire)? {
            Some(segment) => wires.intern(&segment),
            None => raw,
        };
        wires.canonical[raw.0] = Some(id);
        Ok(id)
    }
    pub fn wire(&self, id: WireId) -> Wire {
        self.wires.borrow().wire(id)
    }
    pub fn matches(&self, id: WireId, wire: &Wire) -> bool {
        let wires = self.wires.borrow();
        let key = wires.keys[id.0];
        key.location == wire.location && wires.name_text[key.name.0] == wire.name
    }
    pub fn incoming(&self, model: &DeviceModel, id: WireId) -> Result<Vec<Incoming>> {
        self.with_lookup(|lookup| lookup.incoming(model, id))
    }
    pub fn with_lookup<T>(&self, work: impl FnOnce(Lookup<'_>) -> T) -> T {
        let wires = self.wires.borrow();
        work(Lookup {
            wires: &wires,
            templates: &self.templates,
            by_name: &self.by_name,
            by_kind: &self.by_kind,
            by_location: &self.by_location,
            clocks: &self.clocks,
            tile_order: &self.tile_order,
        })
    }
}

/// A read-only view used while a batch's workers are alive. No worker interns
/// wires or mutates the index; the RefCell borrow ends before the walk resumes.
#[derive(Clone, Copy)]
pub(super) struct Lookup<'a> {
    wires: &'a Wires,
    templates: &'a [Template],
    by_name: &'a HashMap<NameId, Vec<usize>>,
    by_kind: &'a HashMap<String, Vec<TileId>>,
    by_location: &'a HashMap<Location, Vec<TileId>>,
    clocks: &'a HashMap<WireId, Vec<Incoming>>,
    tile_order: &'a HashMap<Location, usize>,
}
impl Lookup<'_> {
    pub fn tile_order(&self, id: WireId) -> usize {
        self.tile_order
            .get(&self.wires.keys[id.0].location)
            .copied()
            .unwrap_or(0)
    }
    pub fn incoming(&self, model: &DeviceModel, id: WireId) -> Result<Vec<Incoming>> {
        #[cfg(all(test, feature = "compile-timing"))]
        let _timer = crate::compile::timing::enter("compile/index::incoming");
        let wires = self.wires;
        let key = wires.keys[id.0];
        let name = &wires.name_text[key.name.0];
        let mut result = self.clocks.get(&id).cloned().unwrap_or_default();
        for &index in self
            .by_name
            .get(&wires.name_keys[key.name.0])
            .into_iter()
            .flatten()
        {
            let spec = &self.templates[index];
            let candidates = if spec.pattern.global {
                if key.location != (Location { row: 0, col: 0 }) {
                    continue;
                }
                self.by_kind.get(&spec.kind)
            } else {
                let row = i32::from(key.location.row) - spec.pattern.row_offset;
                let col = i32::from(key.location.col) - spec.pattern.col_offset;
                let (Ok(row), Ok(col)) = (i16::try_from(row), i16::try_from(col)) else {
                    continue;
                };
                self.by_location.get(&Location { row, col })
            };
            for &tile in candidates.into_iter().flatten() {
                let t = model.tile(tile);
                if t.kind != spec.kind {
                    continue;
                }
                if let Endpoint::Wire(resolved) = spec.pattern.place(
                    model.database(),
                    Location {
                        row: t.row,
                        col: t.col,
                    },
                )? {
                    let resolved = canonical_ref(model, &resolved)?;
                    if resolved.location == key.location && resolved.name == *name {
                        result.push(Incoming {
                            tile,
                            raw: spec.raw.clone(),
                            constant: spec.constant.clone(),
                        });
                    }
                }
            }
        }
        if result.len() <= 1 {
            return Ok(result);
        }
        let mut seen = HashSet::new();
        result.retain(|item| seen.insert(item.clone()));
        result.sort_by(|a, b| {
            (&model.tile(a.tile).name, &a.raw, &a.constant).cmp(&(
                &model.tile(b.tile).name,
                &b.raw,
                &b.constant,
            ))
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_ids_preserve_location_and_names_without_merging_pcs_sides() {
        let mut wires = Wires::default();
        let a = Wire {
            location: Location { row: 1, col: 2 },
            name: "JPCSA0".into(),
        };
        let b = Wire {
            name: "JPCSB0".into(),
            ..a.clone()
        };
        let other = Wire {
            location: Location { row: 2, col: 2 },
            ..a.clone()
        };
        let ids = [wires.intern(&a), wires.intern(&b), wires.intern(&other)];
        assert_eq!(ids, [WireId(0), WireId(1), WireId(2)]);
        for (id, wire) in ids.into_iter().zip([&a, &b, &other]) {
            assert_eq!(wires.intern(wire), id);
            assert_eq!(wires.wire(id), *wire);
        }
        // Incoming templates share a PCSB bucket; placed wire identity does not.
        let a_name = wires.keys[ids[0].0].name;
        let b_name = wires.keys[ids[1].0].name;
        assert_eq!(wires.name_keys[a_name.0], b_name);
        assert_eq!(wires.name_keys[b_name.0], b_name);
    }
}
