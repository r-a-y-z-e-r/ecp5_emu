use crate::{
    config::{Config, TileConfig},
    database::Database,
};
use anyhow::{ensure, Context, Result};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Wire {
    row: i16,
    col: i16,
    name: u32,
}

/// Routing wires are interned; enabled switches and fixed connections alias nets.
/// Inactive configurable switches never connect two nets.
pub struct Routing {
    symbols: HashMap<String, u32>,
    wires: HashMap<Wire, usize>,
    parent: Vec<usize>,
    size: Vec<usize>,
    pub fixed_connections: usize,
    pub configured_switches: usize,
    max_row: i16,
    max_col: i16,
    globals: serde_json::Value,
}

impl Routing {
    pub fn build(db: &Database, config: &Config) -> Result<Self> {
        let mut result = Self {
            symbols: HashMap::new(),
            wires: HashMap::new(),
            parent: Vec::new(),
            size: Vec::new(),
            fixed_connections: 0,
            configured_switches: 0,
            max_row: db.max_row,
            max_col: db.max_col,
            globals: db.globals.clone(),
        };
        let empty = TileConfig::default();
        for tile in db.tiles.values() {
            let tt = &db.types[&tile.kind];
            let tc = config.tiles.get(&tile.name).unwrap_or(&empty);
            for (sink, source) in &tt.fixed {
                if let (Some(a), Some(b)) = (
                    result.wire(tile.row, tile.col, sink),
                    result.wire(tile.row, tile.col, source),
                ) {
                    result.join(a, b);
                    result.fixed_connections += 1;
                }
            }
            for (sink, source) in tt
                .default_arcs
                .iter()
                .filter(|(sink, _)| !tc.arcs.contains_key(*sink))
                .chain(tc.arcs.iter())
            {
                // These mux bits are shared with CCU2.INJECT1. In arithmetic
                // mode the fabric output is always the corresponding sum pin.
                // This is also special-cased in Trellis tools/ecp_vlog.py.
                let corrected_source;
                let source = if tile.kind == "PLC2"
                    && sink.len() == 2
                    && sink.starts_with('F')
                    && sink.as_bytes()[1].is_ascii_digit()
                    && sink.as_bytes()[1] <= b'7'
                    && tt.enum_value(
                        tc,
                        &format!(
                            "SLICE{}.MODE",
                            (b'A' + (sink.as_bytes()[1] - b'0') / 2) as char
                        ),
                    ) == Some("CCU2")
                {
                    corrected_source = format!("{sink}_SLICE");
                    &corrected_source
                } else {
                    source
                };
                if let (Some(a), Some(b)) = (
                    result.wire(tile.row, tile.col, sink),
                    result.wire(tile.row, tile.col, source),
                ) {
                    result.join(a, b);
                    result.configured_switches += usize::from(tc.arcs.contains_key(sink));
                } else {
                    ensure!(
                        !tc.arcs.contains_key(sink),
                        "configured arc is outside the device: {} {source} -> {sink}",
                        tile.name
                    );
                }
            }
        }
        Ok(result)
    }

    fn canonical(&self, mut row: i16, mut col: i16, name: &str) -> Option<(i16, i16, String)> {
        let mut name = name.to_string();
        if ["25K_", "45K_", "85K_"].iter().any(|p| name.starts_with(p)) {
            if !name.starts_with("85K_") {
                return None;
            }
            name = name[4..].into();
        }
        if col >= 69 {
            name = name.replace("PCSA", "PCSB");
        }
        if !(name.starts_with("G_") || name.starts_with("L_") || name.starts_with("R_")) {
            if let Some((prefix, local)) = name.split_once('_') {
                let bytes = prefix.as_bytes();
                let mut i = 0;
                while i < bytes.len() && matches!(bytes[i], b'N' | b'S' | b'E' | b'W') {
                    let dir = bytes[i];
                    i += 1;
                    let start = i;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if start == i {
                        break;
                    }
                    let count: i16 = prefix[start..i].parse().ok()?;
                    match dir {
                        b'N' => row -= count,
                        b'S' => row += count,
                        b'E' => col += count,
                        b'W' => col -= count,
                        _ => unreachable!(),
                    }
                }
                if i == bytes.len() && i > 0 {
                    name = local.into();
                }
            }
        }
        if row < 0 || col < 0 || row > self.max_row || col > self.max_col {
            return None;
        }
        // ECP5 clock backbones have a physical segment per quadrant, spine and
        // row. Trellis deliberately leaves these aliases to place-and-route.
        if name.starts_with("G_HPBX") {
            let taps = self.globals["taps"].as_object()?;
            let (tap, side) = taps.iter().find_map(|(key, v)| {
                if col as i64 >= v["lx0"].as_i64()? && col as i64 <= v["lx1"].as_i64()? {
                    Some((key, 'L'))
                } else if col as i64 >= v["rx0"].as_i64()? && col as i64 <= v["rx1"].as_i64()? {
                    Some((key, 'R'))
                } else {
                    None
                }
            })?;
            col = tap.strip_prefix('C')?.parse().ok()?;
            name.replace_range(..1, &side.to_string());
        } else if name.starts_with("G_VPTX") || name.starts_with("G_HPRX") {
            let quadrants = self.globals["quadrants"].as_object()?;
            let (quadrant, _) = quadrants.iter().find(|(_, q)| {
                let inside = || {
                    Some(
                        col as i64 >= q["x0"].as_i64()?
                            && col as i64 <= q["x1"].as_i64()?
                            && row as i64 >= q["y0"].as_i64()?
                            && row as i64 <= q["y1"].as_i64()?,
                    )
                };
                inside() == Some(true)
            })?;
            if name.starts_with("G_HPRX") {
                let index: u8 = name[6..8].parse().ok()?;
                name = format!("G_{quadrant}PCLK{index}");
                row = 0;
                col = 0;
            } else {
                let (tap, _) = self.globals["taps"].as_object()?.iter().find(|(_, v)| {
                    let inside = || {
                        Some(col as i64 >= v["lx0"].as_i64()? && col as i64 <= v["rx1"].as_i64()?)
                    };
                    inside() == Some(true)
                })?;
                let spine = &self.globals["spines"][format!("{quadrant}{}", &tap[1..])];
                row = spine["y"].as_i64()? as i16;
                col = spine["x"].as_i64()? as i16;
            }
        } else if name.starts_with("G_") {
            row = 0;
            col = 0;
        }
        Some((row, col, name))
    }

    pub fn wire(&mut self, row: i16, col: i16, name: &str) -> Option<usize> {
        let (row, col, name) = self.canonical(row, col, name)?;
        let next = self.symbols.len() as u32;
        let symbol = *self.symbols.entry(name).or_insert(next);
        let key = Wire {
            row,
            col,
            name: symbol,
        };
        let next = self.parent.len();
        if let Some(&id) = self.wires.get(&key) {
            return Some(id);
        }
        self.wires.insert(key, next);
        self.parent.push(next);
        self.size.push(1);
        Some(next)
    }

    pub fn find(&self, row: i16, col: i16, name: &str) -> Option<usize> {
        let (row, col, name) = self.canonical(row, col, name)?;
        let symbol = *self.symbols.get(&name)?;
        Some(self.root(*self.wires.get(&Wire {
            row,
            col,
            name: symbol,
        })?))
    }

    pub fn root(&self, mut id: usize) -> usize {
        while self.parent[id] != id {
            id = self.parent[id];
        }
        id
    }

    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
    }

    pub fn net(&mut self, row: i16, col: i16, name: &str) -> Result<usize> {
        let id = self
            .wire(row, col, name)
            .with_context(|| format!("invalid wire R{row}C{col}:{name}"))?;
        Ok(self.root(id))
    }

    pub fn wire_count(&self) -> usize {
        self.parent.len()
    }
}
