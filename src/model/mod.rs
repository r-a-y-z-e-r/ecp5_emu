//! Persistent, configuration-independent architecture. No signal execution here.
pub(crate) mod catalog;
pub mod topology;

use crate::database::{Database, Tile};
use anyhow::{ensure, Context, Result};
use catalog::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::Path,
};
use topology::{ClockTopology, Connection, ConnectionId, Endpoint, Location, Wire};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileId(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    plc: ResourceId,
    template: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    Instance,
    GroupMember,
    InstanceCandidate,
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Direction {
    #[serde(rename = "in")]
    Input,
    #[serde(rename = "out")]
    Output,
    #[serde(rename = "inout")]
    InOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub direction: Direction,
    pub wire: Wire,
}

#[derive(Debug)]
pub struct Resource {
    pub key: String,
    pub family: String,
    pub name: String,
    pub location: Location,
    pub anchor: TileId,
    pub parent: Option<ResourceId>,
    pub native_bel: bool,
    pub native_name: String,
    pub evidence_rule: String,
    ports: Vec<Port>,
}

#[derive(Debug)]
pub struct SiteReference {
    pub tile: TileId,
    pub index: usize,
    pub name: String,
    pub family: String,
    /// Vendor coordinates deliberately do not become fabric coordinates.
    pub vendor_location: Location,
    pub resource: Option<ResourceId>,
    pub binding: Binding,
    pub evidence_rule: String,
}

#[derive(Debug)]
pub struct PackagePin {
    pub package: String,
    pub ball: String,
    pub bsdl_function: String,
    pub pad: String,
    pub function: String,
    pub dual_function: String,
    pub bank: String,
    pub kind: String,
    pub evidence: String,
    pub pio: Option<ResourceId>,
}

pub struct Node<'a> {
    pub id: NodeId,
    pub owner: ResourceId,
    pub key: String,
    pub kind: &'a str,
    pub native_name: &'a str,
    pub native_z: i16,
    pub interpretation: &'a str,
}

#[derive(Serialize)]
pub struct Summary {
    pub device: String,
    pub manifest_sha256: String,
    pub database_revision: String,
    pub trellis_revision: String,
    pub tiles: usize,
    pub tile_types: usize,
    pub resources: usize,
    pub resource_families: BTreeMap<String, usize>,
    pub site_references: usize,
    pub site_bindings: BTreeMap<String, usize>,
    pub package_balls: BTreeMap<String, usize>,
    pub bonded_pio_references: usize,
    pub resource_ports: usize,
    pub plc_internal_node_views: usize,
    pub plc_internal_ports: usize,
    pub route_templates: usize,
    pub placed_route_descriptions: usize,
    pub behavioral_models: &'static str,
}

/// Dense handles are process-local. Readable keys plus the pinned data revision
/// are external identities. All access is immutable after validated construction.
pub struct DeviceModel {
    db: Database,
    tiles: Vec<String>,
    tile_ids: HashMap<String, TileId>,
    resources: Vec<Resource>,
    resource_ids: HashMap<String, ResourceId>,
    children: Vec<Vec<ResourceId>>,
    sites: Vec<SiteReference>,
    pins: BTreeMap<(String, String), PackagePin>,
    nodes: Vec<NodeTemplate>,
    plc_ports: Vec<PortTemplate>,
    clocks: ClockTopology,
    revision: Manifest,
    manifest_sha256: String,
    cram_dimensions: (usize, usize),
}

impl DeviceModel {
    pub fn load(database: &Path, evidence: &Path) -> Result<Self> {
        let bytes = std::fs::read(evidence.join("model_manifest.json"))
            .context("reading model_manifest.json; run tools/build_model_manifest.py")?;
        let revision: Manifest = serde_json::from_slice(&bytes)?;
        ensure!(
            revision.schema_version == 1 && revision.identity_schema_version == 1,
            "unsupported architecture schema"
        );
        ensure!(
            revision.device == "LFE5U-85F",
            "architecture supports LFE5U-85F"
        );
        let files = verify(evidence, &revision.files)?;
        ensure!(
            files.len() == INPUTS.len() && INPUTS.iter().all(|p| files.contains_key(*p)),
            "architecture input set differs"
        );
        let inventory: Inventory = serde_json::from_slice(&files["device_inventory.json"])?;
        ensure!(
            inventory.device == revision.device
                && inventory.database_revision == revision.database_revision
                && inventory.trellis_revision == revision.trellis_revision,
            "inconsistent architecture revisions"
        );
        let database_sources: HashSet<_> = inventory
            .source_files
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        for name in ["tilegrid.json", "iodb.json", "globals.json"] {
            ensure!(
                database_sources.contains(format!("ECP5/{}/{name}", revision.device).as_str()),
                "database input is not pinned: {name}"
            );
        }
        verify(database, &inventory.source_files)?;
        let db = Database::load(database, &revision.device)?;
        let g = &inventory.device_info;
        ensure!(
            g.max_row == db.max_row && g.max_col == db.max_col,
            "architecture grid dimensions differ"
        );
        let mut windows: HashMap<&str, (usize, usize)> = HashMap::new();
        for t in db.tiles.values() {
            ensure!(
                t.cram
                    .start_frame
                    .checked_add(t.cram.frames)
                    .is_some_and(|n| n <= g.frames)
                    && t.cram
                        .start_bit
                        .checked_add(t.cram.bits)
                        .is_some_and(|n| n <= g.bits_per_frame),
                "tile {} exceeds CRAM",
                t.name
            );
            let w = windows.entry(&t.kind).or_insert((usize::MAX, usize::MAX));
            w.0 = w.0.min(t.cram.frames);
            w.1 = w.1.min(t.cram.bits);
        }
        for (kind, tt) in &db.types {
            ensure!(
                database_sources.contains(format!("ECP5/tiledata/{kind}/bits.db").as_str()),
                "tile type is not pinned: {kind}"
            );
            let (frames, bits) = windows[kind.as_str()];
            for (name, def) in &tt.enums {
                ensure!(
                    def.patterns
                        .values()
                        .flatten()
                        .all(|b| b.frame < frames && b.bit < bits),
                    "enum bit outside {kind}:{name}"
                );
            }
            for (name, groups) in &tt.word_bits {
                ensure!(
                    groups.len() == tt.words[name].len(),
                    "incomplete word layout: {kind}:{name}"
                );
                ensure!(
                    groups
                        .iter()
                        .flatten()
                        .all(|b| b.frame < frames && b.bit < bits),
                    "word bit outside {kind} tile"
                );
            }
            let mut routes = HashSet::new();
            for arc in &tt.routing {
                ensure!(
                    routes.insert((arc.kind, &arc.source, &arc.sink)),
                    "duplicate route identity in {kind}"
                );
                ensure!(
                    arc.bits.iter().all(|b| b.frame < frames && b.bit < bits),
                    "routing bit outside {kind} tile"
                );
            }
        }
        let tiles: Vec<_> = db.tiles.keys().cloned().collect();
        let tile_ids: HashMap<_, _> = tiles
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), TileId(i)))
            .collect();
        let rows: Vec<ResourceRow> = csv(&files, "resource_instances.csv")?;
        let mut resource_ids = HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            ensure!(
                resource_ids
                    .insert(r.resource_id.clone(), ResourceId(i))
                    .is_none(),
                "duplicate resource {}",
                r.resource_id
            );
        }
        let lookup = |key: &str| {
            resource_ids
                .get(key)
                .copied()
                .with_context(|| format!("unknown resource {key}"))
        };
        let mut resources = Vec::new();
        for r in rows {
            ensure!(
                r.resource_id
                    == format!(
                        "{}/R{}C{}/{}",
                        db.device, r.fabric_row, r.fabric_col, r.name
                    ),
                "resource identity differs from its location"
            );
            ensure!(
                (0..=db.max_row).contains(&r.fabric_row)
                    && (0..=db.max_col).contains(&r.fabric_col),
                "resource outside device"
            );
            let native_bel = match r.native_bel.as_str() {
                "True" => true,
                "False" => false,
                _ => anyhow::bail!("invalid native_bel value"),
            };
            resources.push(Resource {
                key: r.resource_id,
                family: r.family,
                name: r.name,
                location: Location {
                    row: r.fabric_row,
                    col: r.fabric_col,
                },
                anchor: *tile_ids
                    .get(&r.anchor_tile)
                    .context("unknown resource anchor")?,
                parent: if r.parent_id.is_empty() {
                    None
                } else {
                    Some(lookup(&r.parent_id)?)
                },
                native_bel,
                native_name: r.native_name,
                evidence_rule: r.rule,
                ports: Vec::new(),
            });
        }
        let mut children = vec![Vec::new(); resources.len()];
        let mut pending = vec![0_u8; resources.len()];
        for (i, r) in resources.iter().enumerate() {
            if let Some(parent) = r.parent {
                children[parent.0].push(ResourceId(i));
                pending[i] = 1;
            }
        }
        let mut ready: VecDeque<_> = pending
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == 0)
            .map(|(i, _)| i)
            .collect();
        let mut visited = 0;
        while let Some(i) = ready.pop_front() {
            visited += 1;
            for child in &children[i] {
                pending[child.0] -= 1;
                if pending[child.0] == 0 {
                    ready.push_back(child.0);
                }
            }
        }
        ensure!(visited == resources.len(), "resource parent cycle");
        let site_rows: Vec<SiteRow> = csv(&files, "site_bindings.csv")?;
        let mut sites = Vec::new();
        let mut seen = HashSet::new();
        for r in site_rows {
            let tile = *tile_ids
                .get(&r.tile)
                .context("site references unknown tile")?;
            let original = db.tiles[&r.tile]
                .sites
                .get(r.site_index)
                .context("site index out of bounds")?;
            ensure!(
                seen.insert((tile, r.site_index)),
                "duplicate site reference"
            );
            ensure!(
                original.name == r.site_name
                    && original.pos_row.parse::<i16>()? == r.vendor_pos_row
                    && original.pos_col.parse::<i16>()? == r.vendor_pos_col,
                "site reference differs from database"
            );
            let resource = if r.resource_id.is_empty() {
                None
            } else {
                Some(lookup(&r.resource_id)?)
            };
            ensure!(
                (r.relation == Binding::Unresolved) == resource.is_none(),
                "binding status and target disagree"
            );
            sites.push(SiteReference {
                tile,
                index: r.site_index,
                name: r.site_name,
                family: r.label_family,
                vendor_location: Location {
                    row: r.vendor_pos_row,
                    col: r.vendor_pos_col,
                },
                resource,
                binding: r.relation,
                evidence_rule: r.rule,
            });
        }
        ensure!(
            sites.len() == db.tiles.values().map(|t| t.sites.len()).sum::<usize>(),
            "not all database sites are retained"
        );
        for name in ["resource_ports.csv", "iologic_research/resource_ports.csv"] {
            let rows: Vec<PortRow> = csv(&files, name)?;
            for r in rows {
                let id = lookup(&r.resource_id)?;
                ensure!(
                    (0..=db.max_row).contains(&r.wire_row)
                        && (0..=db.max_col).contains(&r.wire_col),
                    "port wire outside device"
                );
                resources[id.0].ports.push(Port {
                    name: r.port,
                    direction: r.direction,
                    wire: Wire {
                        location: Location {
                            row: r.wire_row,
                            col: r.wire_col,
                        },
                        name: r.wire,
                    },
                });
            }
        }
        for r in &mut resources {
            r.ports.sort_by(|a, b| a.name.cmp(&b.name));
            ensure!(
                r.ports.windows(2).all(|p| p[0].name != p[1].name),
                "duplicate port in {}",
                r.key
            );
        }
        let nodes: Vec<NodeTemplate> = csv(&files, "plc_research/node_templates.csv")?;
        let plc_ports: Vec<PortTemplate> = csv(&files, "plc_research/native_ports.csv")?;
        let node_keys: HashSet<_> = nodes.iter().map(|n| (&n.view, &n.native_name)).collect();
        let port_keys: HashSet<_> = plc_ports
            .iter()
            .map(|p| (&p.view, &p.bel, &p.port))
            .collect();
        ensure!(
            node_keys.len() == nodes.len() && port_keys.len() == plc_ports.len(),
            "duplicate PLC interface identity"
        );
        ensure!(
            nodes
                .iter()
                .all(|n| n.view == "split" || n.view == "coarse"),
            "unknown PLC view"
        );
        ensure!(
            plc_ports.iter().all(|p| nodes
                .iter()
                .any(|n| n.view == p.view && n.native_name == p.bel)),
            "port has no PLC node view"
        );
        for (index, r) in resources
            .iter()
            .enumerate()
            .filter(|(_, r)| r.family == "PLC2")
        {
            for port in &plc_ports {
                let row = i32::from(r.location.row) + i32::from(port.wire_row_offset);
                let col = i32::from(r.location.col) + i32::from(port.wire_col_offset);
                ensure!(
                    (0..=i32::from(db.max_row)).contains(&row)
                        && (0..=i32::from(db.max_col)).contains(&col),
                    "PLC port template outside device"
                );
            }
            for node in nodes.iter().filter(|n| n.view == "split") {
                let (bel, _) = node
                    .native_name
                    .split_once('.')
                    .context("split PLC node has no slice")?;
                ensure!(
                    children[index]
                        .iter()
                        .filter(|id| resources[id.0].native_name == bel
                            && resources[id.0].family == "SLICE")
                        .count()
                        == 1,
                    "PLC {} has no unique owner for {}",
                    r.key,
                    node.native_name
                );
            }
        }
        let pin_rows: Vec<PinRow> = csv(&files, "package_system_research/physical_pins.csv")?;
        let mut pins = BTreeMap::new();
        let mut pio_matches = 0;
        for r in pin_rows {
            let package = db
                .packages
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&r.package))
                .context("unknown pin package")?;
            let native = package.1.get(&r.ball);
            ensure!(
                (r.kind == "GPIO") == native.is_some(),
                "GPIO package binding differs"
            );
            let pio = native
                .map(|p| lookup(&format!("{}/R{}C{}/PIO{}", db.device, p.row, p.col, p.pio)))
                .transpose()?;
            pio_matches += usize::from(pio.is_some());
            let key = (r.package.to_ascii_uppercase(), r.ball.clone());
            let pin = PackagePin {
                package: r.package,
                ball: r.ball,
                bsdl_function: r.bsdl_function,
                pad: r.pad,
                function: r.pinout_function,
                dual_function: r.dual_function,
                bank: r.bank,
                kind: r.kind,
                evidence: r.evidence,
                pio,
            };
            ensure!(pins.insert(key, pin).is_none(), "duplicate package ball");
        }
        ensure!(
            pio_matches == db.packages.values().map(|p| p.len()).sum::<usize>(),
            "missing package GPIOs"
        );
        let clocks = ClockTopology::load(&db)?;
        let cram_dimensions = (g.frames, g.bits_per_frame);
        Ok(Self {
            db,
            tiles,
            tile_ids,
            resources,
            resource_ids,
            children,
            sites,
            pins,
            nodes,
            plc_ports,
            clocks,
            revision,
            manifest_sha256: digest(&bytes),
            cram_dimensions,
        })
    }

    pub fn database(&self) -> &Database {
        &self.db
    }
    pub fn cram_dimensions(&self) -> (usize, usize) {
        self.cram_dimensions
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    pub fn tiles(&self) -> impl ExactSizeIterator<Item = (TileId, &Tile)> {
        self.tiles
            .iter()
            .enumerate()
            .map(|(i, name)| (TileId(i), &self.db.tiles[name]))
    }
    pub fn tile_id(&self, name: &str) -> Option<TileId> {
        self.tile_ids.get(name).copied()
    }
    pub fn tile(&self, id: TileId) -> &Tile {
        &self.db.tiles[&self.tiles[id.0]]
    }
    pub fn resources(&self) -> impl ExactSizeIterator<Item = (ResourceId, &Resource)> {
        self.resources
            .iter()
            .enumerate()
            .map(|(i, r)| (ResourceId(i), r))
    }
    pub fn resource_id(&self, key: &str) -> Option<ResourceId> {
        self.resource_ids.get(key).copied()
    }
    pub fn resource(&self, id: ResourceId) -> &Resource {
        &self.resources[id.0]
    }
    pub fn children(&self, id: ResourceId) -> &[ResourceId] {
        &self.children[id.0]
    }
    pub fn site_references(&self) -> &[SiteReference] {
        &self.sites
    }
    pub fn package_pins(&self) -> impl Iterator<Item = &PackagePin> {
        self.pins.values()
    }
    pub fn package_pin(&self, package: &str, ball: &str) -> Option<&PackagePin> {
        self.pins
            .get(&(package.to_ascii_uppercase(), ball.to_ascii_uppercase()))
    }
    pub fn nodes(&self, plc: ResourceId) -> impl Iterator<Item = Node<'_>> {
        let resource = self.resource(plc);
        self.nodes
            .iter()
            .enumerate()
            .filter(move |(_, n)| resource.family == "PLC2" && n.view == "split")
            .map(move |(template, n)| {
                let (bel, local) = n
                    .native_name
                    .split_once('.')
                    .expect("validated split PLC node");
                let owner = *self
                    .children(plc)
                    .iter()
                    .find(|id| self.resource(**id).native_name == bel)
                    .expect("validated node owner");
                Node {
                    id: NodeId { plc, template },
                    owner,
                    key: format!("{}/{}", self.resource(owner).key, local),
                    kind: &n.native_type,
                    native_name: &n.native_name,
                    native_z: n.z,
                    interpretation: &n.interpretation,
                }
            })
    }
    fn template_ports(&self, location: Location, view: &str, bel: &str) -> Vec<Port> {
        self.plc_ports
            .iter()
            .filter(|p| p.view == view && p.bel == bel)
            .map(|p| Port {
                name: p.port.clone(),
                direction: p.direction,
                wire: Wire {
                    location: Location {
                        row: location.row + p.wire_row_offset,
                        col: location.col + p.wire_col_offset,
                    },
                    name: p.wire.clone(),
                },
            })
            .collect()
    }
    pub fn resource_ports(&self, id: ResourceId) -> Vec<Port> {
        let r = self.resource(id);
        if r.family == "SLICE" {
            self.template_ports(r.location, "coarse", &r.native_name)
        } else {
            r.ports.clone()
        }
    }
    pub fn node_ports(&self, id: NodeId) -> Vec<Port> {
        self.template_ports(
            self.resource(id.plc).location,
            "split",
            &self.nodes[id.template].native_name,
        )
    }
    pub fn clock_segment(&self, wire: &Wire) -> Result<Option<Wire>> {
        self.clocks.segment(wire)
    }
    pub fn wire(&self, location: Location, name: &str) -> Result<Endpoint> {
        topology::resolve_wire(&self.db, location, name)
    }
    pub fn connections(&self, tile: TileId) -> impl Iterator<Item = Result<Connection<'_>>> {
        let t = self.tile(tile);
        self.db.types[&t.kind]
            .routing
            .iter()
            .enumerate()
            .map(move |(local, definition)| {
                Ok(Connection {
                    id: ConnectionId {
                        tile: tile.0,
                        local,
                    },
                    tile: t,
                    definition,
                    source: self.wire(
                        Location {
                            row: t.row,
                            col: t.col,
                        },
                        &definition.source,
                    )?,
                    sink: self.wire(
                        Location {
                            row: t.row,
                            col: t.col,
                        },
                        &definition.sink,
                    )?,
                })
            })
    }
    pub fn connection(&self, id: ConnectionId) -> Result<Connection<'_>> {
        let tile_name = self.tiles.get(id.tile).context("unknown connection tile")?;
        let tile = &self.db.tiles[tile_name];
        let definition = self.db.types[&tile.kind]
            .routing
            .get(id.local)
            .context("unknown connection")?;
        let location = Location {
            row: tile.row,
            col: tile.col,
        };
        Ok(Connection {
            id,
            tile,
            definition,
            source: self.wire(location, &definition.source)?,
            sink: self.wire(location, &definition.sink)?,
        })
    }
    pub fn connections_to<'a>(
        &'a self,
        tile: TileId,
        sink: &str,
    ) -> impl Iterator<Item = Result<Connection<'a>>> + 'a {
        self.db.types[&self.tile(tile).kind]
            .incoming
            .get(sink)
            .into_iter()
            .flatten()
            .map(move |&local| {
                self.connection(ConnectionId {
                    tile: tile.0,
                    local,
                })
            })
    }
    pub fn summary(&self) -> Summary {
        let mut families = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        let mut balls = BTreeMap::new();
        for r in &self.resources {
            *families.entry(r.family.clone()).or_insert(0) += 1;
        }
        for s in &self.sites {
            *bindings.entry(format!("{:?}", s.binding)).or_insert(0) += 1;
        }
        for p in self.pins.values() {
            *balls.entry(p.package.clone()).or_insert(0) += 1;
        }
        let plcs = families.get("PLC2").copied().unwrap_or(0);
        Summary {
            device: self.db.device.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            database_revision: self.revision.database_revision.clone(),
            trellis_revision: self.revision.trellis_revision.clone(),
            tiles: self.tiles.len(),
            tile_types: self.db.types.len(),
            resources: self.resources.len(),
            resource_families: families,
            site_references: self.sites.len(),
            site_bindings: bindings,
            package_balls: balls,
            bonded_pio_references: self.pins.values().filter(|p| p.pio.is_some()).count(),
            resource_ports: self.resources.iter().map(|r| r.ports.len()).sum::<usize>()
                + plcs * self.plc_ports.iter().filter(|p| p.view == "coarse").count(),
            plc_internal_node_views: plcs * self.nodes.iter().filter(|n| n.view == "split").count(),
            plc_internal_ports: plcs * self.plc_ports.iter().filter(|p| p.view == "split").count(),
            route_templates: self.db.types.values().map(|t| t.routing.len()).sum(),
            placed_route_descriptions: self
                .db
                .tiles
                .values()
                .map(|t| self.db.types[&t.kind].routing.len())
                .sum(),
            behavioral_models: "not_executed_by_architecture_layer",
        }
    }
}
