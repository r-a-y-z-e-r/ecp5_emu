//! Wire identity and directed route descriptions, without configured net merging.
use crate::database::{ArcDef, ArcKind, BitConstraint, Database};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Location {
    pub row: i16,
    pub col: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Wire {
    pub location: Location,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Wire(Wire),
    OtherDensity,
    OutsideDevice,
}

/// Unplaced native wire name. Kept separate so incoming routes can be located
/// without materializing every one of the chip's placed route descriptions.
#[derive(Clone)]
pub(crate) struct WirePattern {
    pub name: String,
    pub row_offset: i32,
    pub col_offset: i32,
    pub global: bool,
}
impl WirePattern {
    pub fn parse(raw: &str) -> Result<Option<Self>> {
        let mut name = raw;
        for prefix in ["25K_", "45K_", "85K_"] {
            if let Some(rest) = raw.strip_prefix(prefix) {
                if prefix != "85K_" {
                    return Ok(None);
                }
                name = rest;
                break;
            }
        }
        let mut name = name.to_string();
        let (mut row_offset, mut col_offset) = (0, 0);
        let global =
            name.starts_with("G_") && !["VPTX", "HPBX", "HPRX"].iter().any(|s| name.contains(s));
        if let Some((prefix, local)) = name.split_once('_').filter(|_| {
            !name.starts_with("G_") && !name.starts_with("L_") && !name.starts_with("R_")
        }) {
            let mut rest = prefix;
            let mut offset = (0_i32, 0_i32);
            let mut matched = false;
            for directions in ["NS", "EW"] {
                if let Some(dir) = rest.chars().next().filter(|c| directions.contains(*c)) {
                    let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
                    if digits == 0 {
                        break;
                    }
                    let n: i32 = rest[1..=digits]
                        .parse()
                        .context("relative wire offset overflow")?;
                    match dir {
                        'N' => offset.0 = -n,
                        'S' => offset.0 = n,
                        'W' => offset.1 = -n,
                        'E' => offset.1 = n,
                        _ => unreachable!(),
                    }
                    rest = &rest[digits + 1..];
                    matched = true;
                }
            }
            if matched && rest.is_empty() {
                row_offset = offset.0;
                col_offset = offset.1;
                name = local.to_string();
            }
        }
        Ok(Some(Self {
            name,
            row_offset,
            col_offset,
            global,
        }))
    }
    pub fn place(&self, db: &Database, location: Location) -> Result<Endpoint> {
        let (row, col) = if self.global {
            (0, 0)
        } else {
            (
                i32::from(location.row)
                    .checked_add(self.row_offset)
                    .context("relative wire row overflow")?,
                i32::from(location.col)
                    .checked_add(self.col_offset)
                    .context("relative wire column overflow")?,
            )
        };
        let name = if location.col >= 69 {
            self.name.replacen("PCSA", "PCSB", 1)
        } else {
            self.name.clone()
        };
        if row < 0 || col < 0 || row > i32::from(db.max_row) || col > i32::from(db.max_col) {
            return Ok(Endpoint::OutsideDevice);
        }
        Ok(Endpoint::Wire(Wire {
            location: Location {
                row: row as i16,
                col: col as i16,
            },
            name,
        }))
    }
}

/// Follows native Trellis globalization. Clock segment relationships are separate.
pub fn resolve_wire(db: &Database, location: Location, raw: &str) -> Result<Endpoint> {
    match WirePattern::parse(raw)? {
        Some(pattern) => pattern.place(db, location),
        None => Ok(Endpoint::OtherDensity),
    }
}

#[derive(Debug, Deserialize)]
struct Region {
    x0: i16,
    x1: i16,
    y0: i16,
    y1: i16,
}
#[derive(Debug, Deserialize)]
struct Tap {
    lx0: i16,
    lx1: i16,
    rx0: i16,
    rx1: i16,
}
#[derive(Debug, Deserialize)]
struct Spine {
    x: i16,
    y: i16,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClockTopology {
    quadrants: BTreeMap<String, Region>,
    taps: BTreeMap<String, Tap>,
    spines: BTreeMap<String, Spine>,
}

impl ClockTopology {
    pub fn load(db: &Database) -> Result<Self> {
        let result: Self = serde_json::from_value(db.globals.clone())?;
        ensure!(
            result
                .spines
                .values()
                .all(|s| (0..=db.max_row).contains(&s.y) && (0..=db.max_col).contains(&s.x)),
            "clock spine outside device"
        );
        // A malformed region map must fail here, before it can alias a clock.
        for row in 0..=db.max_row {
            for col in 0..=db.max_col {
                result.segment_location(Location { row, col })?;
            }
        }
        Ok(result)
    }

    fn segment_location(&self, loc: Location) -> Result<(&str, i16, char, &Spine)> {
        let regions: Vec<_> = self
            .quadrants
            .iter()
            .filter(|(_, r)| (r.x0..=r.x1).contains(&loc.col) && (r.y0..=r.y1).contains(&loc.row))
            .collect();
        ensure!(
            regions.len() == 1,
            "clock quadrants must cover each coordinate exactly once"
        );
        let taps: Vec<_> = self
            .taps
            .iter()
            .flat_map(|(name, t)| {
                [('L', t.lx0, t.lx1), ('R', t.rx0, t.rx1)]
                    .into_iter()
                    .filter(move |(_, a, b)| (*a..=*b).contains(&loc.col))
                    .map(move |(side, _, _)| (name, side))
            })
            .collect();
        ensure!(
            taps.len() == 1,
            "clock taps must cover each coordinate exactly once"
        );
        let column: i16 = taps[0]
            .0
            .strip_prefix('C')
            .context("invalid tap column")?
            .parse()?;
        let quadrant = regions[0].0.as_str();
        let spine = self
            .spines
            .get(&format!("{quadrant}{column}"))
            .context("missing clock spine")?;
        Ok((quadrant, column, taps[0].1, spine))
    }

    /// A named segment relationship; this does not enable a buffer or union nets.
    pub fn segment(&self, wire: &Wire) -> Result<Option<Wire>> {
        let prefix = ["G_HPBX", "G_VPTX", "G_HPRX"]
            .into_iter()
            .find(|p| wire.name.starts_with(p));
        let Some(prefix) = prefix else {
            return Ok(None);
        };
        let suffix = &wire.name[prefix.len()..];
        ensure!(
            suffix.len() == 4
                && suffix.ends_with("00")
                && suffix[..2].bytes().all(|b| b.is_ascii_digit()),
            "invalid primary-clock channel {}",
            wire.name
        );
        let channel: u8 = suffix[..2].parse()?;
        ensure!(channel < 16, "primary-clock channel out of range");
        let (quadrant, column, side, spine) = self.segment_location(wire.location)?;
        let (location, name) = match prefix {
            "G_HPBX" => (
                Location {
                    row: wire.location.row,
                    col: column,
                },
                format!("{side}_HPBX{suffix}"),
            ),
            "G_VPTX" => (
                Location {
                    row: spine.y,
                    col: spine.x,
                },
                wire.name.clone(),
            ),
            _ => (
                Location { row: 0, col: 0 },
                format!("G_{quadrant}PCLK{channel}"),
            ),
        };
        Ok(Some(Wire { location, name }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId {
    pub(super) tile: usize,
    pub(super) local: usize,
}

pub struct Connection<'a> {
    pub id: ConnectionId,
    pub tile: &'a crate::database::Tile,
    pub definition: &'a ArcDef,
    pub source: Endpoint,
    pub sink: Endpoint,
}

impl Connection<'_> {
    pub fn stable_key(&self, device: &str) -> String {
        let kind = match self.definition.kind {
            ArcKind::Fixed => "fixed",
            ArcKind::Mux => "mux",
        };
        format!(
            "{device}/{}/route/{kind}/{}->{}",
            self.tile.name, self.definition.source, self.definition.sink
        )
    }

    pub fn cram_constraints(&self) -> impl Iterator<Item = BitConstraint> + '_ {
        self.definition.bits.iter().map(|b| BitConstraint {
            frame: self.tile.cram.start_frame + b.frame,
            bit: self.tile.cram.start_bit + b.bit,
            value: b.value,
        })
    }
}
