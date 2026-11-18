//! nextpnr 0.11.1's printed timing estimates, independent of the window.
//! A report is accepted whole, including its summary and every routing sink.
use crate::{
    board,
    fabric_overlay::{Kind as PieceKind, Overlay, Piece},
    model::{topology::Location, DeviceModel},
    names::Names,
    sources::{Index, Span},
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, HashMap},
    sync::Arc,
};

pub const NO_PATH: &str = "no critical path found in this log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ClkToQ,
    Routing,
    Logic,
    Setup,
    Source,
}
impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Self::ClkToQ => "clk-to-q",
            Self::Routing => "routing",
            Self::Logic => "logic",
            Self::Setup => "setup",
            Self::Source => "source",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "clk-to-q" => Self::ClkToQ,
            "routing" => Self::Routing,
            "logic" => Self::Logic,
            "setup" => Self::Setup,
            "source" => Self::Source,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Domain {
    Clock { name: String, edges: String },
    Cross { from: String, to: String },
}
impl Domain {
    pub fn label(&self) -> String {
        match self {
            Self::Clock { name, edges } if edges == "posedge -> posedge" => name.clone(),
            Self::Clock { name, edges } => format!("{name} ({edges})"),
            Self::Cross { from, to } => format!("{from} -> {to}"),
        }
    }
    pub fn clock(&self) -> Option<&str> {
        match self {
            Self::Clock { name, .. } => Some(name),
            Self::Cross { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub cell: String,
    pub port: String,
}
impl Endpoint {
    fn parse(s: &str) -> Option<Self> {
        let (cell, port) = s.rsplit_once('.')?;
        (!cell.is_empty() && !port.is_empty() && !s.chars().any(char::is_whitespace)).then(|| {
            Self {
                cell: cell.into(),
                port: port.into(),
            }
        })
    }
    pub fn label(&self) -> String {
        format!("{}.{}", self.cell, self.port)
    }
}

/// nextpnr coordinates are (X column, Y row), not (row, column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub col: u16,
    pub row: u16,
}
fn tile(s: &str) -> Option<Tile> {
    let (col, row) = s.strip_prefix('(')?.strip_suffix(')')?.split_once(',')?;
    Some(Tile {
        col: col.parse().ok()?,
        row: row.parse().ok()?,
    })
}
#[derive(Clone, Debug)]
pub struct Hop {
    pub kind: Kind,
    pub curr: f64,
    pub total: f64,
    pub source: Endpoint,
    pub net: Option<String>,
    pub tiles: Option<(Tile, Tile)>,
    pub sink: Option<Endpoint>,
    /// Driver-to-sink overlay pieces, populated once by Report::map.
    pub route: Vec<RoutePiece>,
    /// A routing hop without a chain is explained in the report, never drawn
    /// using approximate tile positions.
    pub route_error: Option<RouteError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteError {
    NoNet,
    DriverSite,
    SinkSite,
    DriverWire,
    SinkWire,
    Disconnected,
    SiteInterior,
}
impl RouteError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoNet => "net has no wires in the overlay",
            Self::DriverSite => "driver site is missing from the wire overlay",
            Self::SinkSite => "sink site is missing from the wire overlay",
            Self::DriverWire => "no routed wire reaches the driver site",
            Self::SinkWire => "no routed wire reaches the sink site",
            Self::Disconnected => "driver and sink wires are disconnected in the overlay",
            Self::SiteInterior => "route crosses an undrawn mux interior before the flip-flop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutePiece {
    pub piece: u32,
    /// Traverse the overlay piece from b to a.
    pub reverse: bool,
}
impl Hop {
    pub fn name(&self) -> String {
        self.net.clone().unwrap_or_else(|| self.source.label())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Frequency {
    pub mhz: f64,
    pub constraint: Option<f64>,
    pub pass: Option<bool>,
}
#[derive(Clone, Debug)]
pub struct Path {
    pub domain: Domain,
    pub hops: Vec<Hop>,
    pub logic: f64,
    pub routing: f64,
    pub frequency: Option<Frequency>,
}
impl Path {
    pub fn total(&self) -> f64 {
        self.hops.last().map_or(0.0, |h| h.total)
    }
    /// The period represented by Fmax also accounts for clock timing.
    pub fn slack(&self) -> Option<f64> {
        let f = self.frequency?;
        Some(1000.0 / f.constraint? - 1000.0 / f.mhz)
    }
    pub fn note(&self) -> String {
        format!(
            "critical {} {} {} {:.2}",
            self.domain.label(),
            self.frequency
                .map_or_else(|| "-".into(), |f| format!("{:.2}", f.mhz)),
            self.hops.len(),
            self.total()
        )
    }
}
#[derive(Clone, Debug)]
pub struct Report {
    pub paths: Vec<Path>,
}

fn body(line: &str) -> &str {
    line.strip_prefix("Info:")
        .or_else(|| line.strip_prefix("ERROR:"))
        .unwrap_or(line)
        .trim()
}
fn number(s: &str) -> Option<f64> {
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let (whole, fraction) = unsigned.split_once('.')?;
    if whole.is_empty()
        || fraction.len() != 2
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.010_001
}
fn domain(s: &str) -> Option<Domain> {
    let s = s.strip_prefix("Critical path report for ")?;
    if let Some(s) = s.strip_prefix("clock '") {
        let (name, edges) = s.split_once("' (")?;
        let edges = edges.strip_suffix("):")?;
        let (from, to) = edges.split_once(" -> ")?;
        if name.is_empty()
            || !["posedge", "negedge"].contains(&from)
            || !["posedge", "negedge"].contains(&to)
        {
            return None;
        }
        Some(Domain::Clock {
            name: name.into(),
            edges: edges.into(),
        })
    } else {
        let (from, to) = s
            .strip_prefix("cross-domain path '")?
            .strip_suffix("':")?
            .split_once("' -> '")?;
        let valid = |s: &str| {
            s == "<async>"
                || ["posedge ", "negedge "]
                    .iter()
                    .any(|e| s.strip_prefix(e).is_some_and(|n| !n.is_empty()))
        };
        (valid(from) && valid(to)).then(|| Domain::Cross {
            from: from.into(),
            to: to.into(),
        })
    }
}
fn frequency(s: &str) -> Option<(String, Frequency)> {
    let (name, rest) = s
        .strip_prefix("Max frequency for clock '")?
        .split_once("': ")?;
    let (mhz, tail) = rest.split_once(" MHz")?;
    let (constraint, pass) = if tail.is_empty() {
        (None, None)
    } else {
        let (pass, constraint) = tail
            .strip_prefix(" (")?
            .strip_suffix(" MHz)")?
            .split_once(" at ")?;
        (
            Some(number(constraint).filter(|v| *v > 0.0)?),
            Some(match pass {
                "PASS" => true,
                "FAIL" => false,
                _ => return None,
            }),
        )
    };
    Some((
        name.into(),
        Frequency {
            mhz: number(mhz).filter(|v| *v > 0.0)?,
            constraint,
            pass,
        },
    ))
}
fn summary(s: &str) -> Option<(f64, f64)> {
    let (logic, routing) = s.strip_suffix(" ns routing")?.split_once(" ns logic, ")?;
    Some((number(logic)?, number(routing)?))
}
fn parse_path(lines: &[&str], at: &mut usize, domain: Domain) -> Option<Path> {
    let (first_kind, last_kind) = match &domain {
        Domain::Clock { .. } => (Kind::ClkToQ, Kind::Setup),
        Domain::Cross { from, to } => (
            if from == "<async>" {
                Kind::Source
            } else {
                Kind::ClkToQ
            },
            if to == "<async>" {
                Kind::Routing
            } else {
                Kind::Setup
            },
        ),
    };
    *at += 1;
    if body(lines.get(*at)?) != "type curr  total name" {
        return None;
    }
    *at += 1;
    let mut hops: Vec<Hop> = Vec::new();
    loop {
        let line = body(lines.get(*at)?);
        if let Some((logic, routing)) = summary(line) {
            let last = hops.last()?;
            if last.kind != last_kind {
                return None;
            }
            if last.kind == Kind::Routing && last.sink.is_none() {
                return None;
            }
            if !close(last.total, logic + routing) {
                return None;
            }
            let sum_routing: f64 = hops
                .iter()
                .filter(|h| h.kind == Kind::Routing)
                .map(|h| h.curr)
                .sum();
            let sum_logic: f64 = hops
                .iter()
                .filter(|h| h.kind != Kind::Routing)
                .map(|h| h.curr)
                .sum();
            // Each displayed delay has independently been rounded to 0.01 ns.
            let tolerance = hops.len() as f64 * 0.005 + 0.01;
            if (sum_routing - routing).abs() > tolerance || (sum_logic - logic).abs() > tolerance {
                return None;
            }
            return Some(Path {
                domain,
                hops,
                logic,
                routing,
                frequency: None,
            });
        }
        let mut words = line.split_whitespace();
        let kind = Kind::parse(words.next()?)?;
        let curr = number(words.next()?)?;
        let total = number(words.next()?)?;
        let tag = words.next()?;
        let name = words.collect::<Vec<_>>().join(" ");
        let previous = hops.last();
        if !close(total, previous.map_or(0.0, |h| h.total) + curr) {
            return None;
        }
        let (source, net, tiles) = if kind == Kind::Routing {
            if tag != "Net" {
                return None;
            }
            let source = previous
                .filter(|h| h.kind != Kind::Routing && h.kind != Kind::Setup)?
                .source
                .clone();
            let (start, end) = name.rsplit_once(" -> ")?;
            let (net, start) = start.rsplit_once(' ')?;
            if net.is_empty() {
                return None;
            }
            (source, Some(net.into()), Some((tile(start)?, tile(end)?)))
        } else {
            if tag != "Source" {
                return None;
            }
            if hops.is_empty() {
                if kind != first_kind {
                    return None;
                }
            } else if previous?.kind != Kind::Routing || !matches!(kind, Kind::Logic | Kind::Setup)
            {
                return None;
            }
            let source = Endpoint::parse(&name)?;
            if previous.is_some_and(|h| h.sink.as_ref().is_none_or(|s| s.cell != source.cell)) {
                return None;
            }
            (source, None, None)
        };
        *at += 1;
        let sink = if kind == Kind::Routing {
            let sink = Endpoint::parse(body(lines.get(*at)?).strip_prefix("Sink ")?)?;
            *at += 1;
            Some(sink)
        } else {
            None
        };
        // Defined-in text is syntax only. Source jumps always use Index.
        if body(lines.get(*at)?) == "Defined in:" {
            *at += 1;
            let start = *at;
            while let Some(line) = lines.get(*at) {
                let s = body(line);
                if !s.is_empty()
                    && s.rsplit_once(':')
                        .is_some_and(|(_, range)| range.starts_with(|c: char| c.is_ascii_digit()))
                {
                    *at += 1;
                } else {
                    break;
                }
            }
            if *at == start {
                return None;
            }
        }
        hops.push(Hop {
            kind,
            curr,
            total,
            source,
            net,
            tiles,
            sink,
            route: Vec::new(),
            route_error: None,
        });
    }
}

/// The creator comes from the routed JSON written by the same nextpnr run.
/// Unknown versions and any malformed timing block publish no paths.
pub fn parse(creator: &str, log: &str) -> Result<Report, String> {
    check_version(creator)?;
    parse_log(log).ok_or_else(|| NO_PATH.into())
}

pub fn check_version(creator: &str) -> Result<(), String> {
    let version = creator
        .trim()
        .strip_prefix("Next Generation Place and Route (Version ")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("unknown");
    if !version.starts_with("nextpnr-0.11.1") {
        return Err(format!(
            "critical path: nextpnr {} not supported",
            version.strip_prefix("nextpnr-").unwrap_or(version)
        ));
    }
    Ok(())
}

fn parse_log(log: &str) -> Option<Report> {
    let lines: Vec<_> = log.lines().collect();
    let mut paths = BTreeMap::new();
    let mut frequencies = BTreeMap::new();
    let mut at = 0;
    while at < lines.len() {
        let line = body(lines[at]);
        if line.starts_with("Critical path report") {
            let domain = domain(line)?;
            let path = parse_path(&lines, &mut at, domain.clone())?;
            paths.insert(domain, path);
        } else if line.starts_with("Max frequency for clock") {
            let (name, frequency) = frequency(line)?;
            frequencies.insert(name, frequency);
        }
        at += 1;
    }
    if paths.is_empty() {
        return None;
    }
    for path in paths.values_mut() {
        path.frequency = path
            .domain
            .clock()
            .and_then(|n| frequencies.get(n))
            .copied();
        if path.domain.clock().is_some() && path.frequency.is_none() {
            return None;
        }
    }
    Some(Report {
        paths: paths.into_values().collect(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub tile: Tile,
    pub label: String,
}
impl Site {
    fn from_bel(bel: &str) -> Option<Self> {
        let mut parts = bel.splitn(3, '/');
        let col = parts.next()?.strip_prefix('X')?.parse().ok()?;
        let row = parts.next()?.strip_prefix('Y')?.parse().ok()?;
        let part = parts.next()?;
        let label = if let Some((slice, bit)) =
            part.strip_prefix("SLICE").and_then(|s| s.split_once('.'))
        {
            let slice = slice.as_bytes();
            if slice.len() != 1 || !(b'A'..=b'D').contains(&slice[0]) {
                return None;
            }
            let (prefix, bit) = if let Some(k) = bit.strip_prefix('K') {
                ("LUT", k)
            } else if let Some(k) = bit.strip_prefix("FF") {
                ("FF", k)
            } else {
                return Some(Self {
                    tile: Tile { col, row },
                    label: format!("SLICE{}", slice[0] as char),
                });
            };
            format!(
                "{prefix}{}",
                (slice[0] - b'A') * 2 + bit.parse::<u8>().ok().filter(|n| *n < 2)?
            )
        } else {
            part.into()
        };
        Some(Self {
            tile: Tile { col, row },
            label,
        })
    }
}
#[derive(Clone, Debug)]
pub struct MappedHop {
    pub hop: Hop,
    pub name: String,
    pub nets: Vec<u32>,
    pub sites: Vec<Site>,
    pub locations: Vec<Span>,
}
impl MappedHop {
    pub fn route_not_shown(&self) -> Option<&'static str> {
        (self.hop.kind == Kind::Routing && self.hop.route.is_empty()).then(|| {
            self.hop
                .route_error
                .map_or("route has not been resolved", RouteError::reason)
        })
    }

    pub fn note(&self, n: usize) -> String {
        format!(
            "critical hop {n} {} {:.2} {} -> {}",
            self.hop.kind.word(),
            self.hop.curr,
            self.name,
            self.locations
                .first()
                .map_or_else(|| "-".into(), Span::label)
        )
    }
}

const ROUTE_TOUCH: f32 = 2e-4;

struct RouteEnd {
    min: [f32; 2],
    max: [f32; 2],
    /// The overlay's value for the actual port wire, including fixed aliases.
    pin: Option<u32>,
    /// A drawn M pin continues on the other side of this mux box. The
    /// overlay deliberately omits the connection through the box itself.
    interior: Option<([f32; 2], [f32; 2])>,
}
impl RouteEnd {
    fn contains(&self, p: [f32; 2]) -> bool {
        (0..2).all(|i| p[i] >= self.min[i] - ROUTE_TOUCH && p[i] <= self.max[i] + ROUTE_TOUCH)
            && (0..2).any(|i| {
                (p[i] - self.min[i]).abs() <= ROUTE_TOUCH
                    || (p[i] - self.max[i]).abs() <= ROUTE_TOUCH
            })
    }

    fn placed(
        endpoint: &Endpoint,
        placements: &BTreeMap<String, String>,
        model: &DeviceModel,
        overlay: &Overlay,
        shape: (usize, usize),
    ) -> Option<Self> {
        let site = Site::from_bel(placements.get(&endpoint.cell)?)?;
        let (cols, rows) = shape;
        if site.tile.col as usize >= cols || site.tile.row as usize >= rows {
            return None;
        }
        let position = site.tile.row as usize * cols + site.tile.col as usize;
        let logic = site
            .label
            .strip_prefix("LUT")
            .and_then(|n| n.parse::<usize>().ok());
        let ff = site
            .label
            .strip_prefix("FF")
            .and_then(|n| n.parse::<usize>().ok());
        let pin_name = site_pin(&endpoint.port, logic, ff);
        let pin = pin_name.and_then(|name| {
            let wire = model
                .wire(
                    Location {
                        row: site.tile.row as i16,
                        col: site.tile.col as i16,
                    },
                    &name,
                )
                .ok()?;
            let crate::model::topology::Endpoint::Wire(wire) = wire else {
                return None;
            };
            overlay.value_of(model, &wire)
        });
        let mux = || {
            let value = pin?;
            let mut boxes = overlay
                .boxes_in(position)
                .iter()
                .filter(|b| b.label == "F5 FX M" && b.values.contains(&value));
            let b = boxes.next()?;
            // A value shared by several boxes does not identify one port.
            // Leave it unresolved rather than choosing a nearby box.
            boxes.next().is_none().then_some((b.min, b.max))
        };
        let bounds = if logic.is_some()
            && matches!(endpoint.port.as_str(), "F1" | "OFX" | "FXA" | "FXB" | "M")
        {
            mux()?
        } else {
            overlay
                .sites_in(position)
                .iter()
                .find(|b| b.label == site.label)
                .map(|b| (b.min, b.max))
                .or_else(|| {
                    overlay
                        .boxes_in(position)
                        .iter()
                        .find(|b| b.label == site.label)
                        .map(|b| (b.min, b.max))
                })?
        };
        Some(Self {
            min: bounds.0,
            max: bounds.1,
            pin,
            interior: (ff.is_some() && endpoint.port == "M").then(mux).flatten(),
        })
    }

    fn undrawn_interior(&self, pieces: &[(u32, &Piece)]) -> bool {
        let Some((min, max)) = self.interior else {
            return false;
        };
        let b = Self {
            min,
            max,
            pin: None,
            interior: None,
        };
        let contacts = |kind| {
            pieces
                .iter()
                .filter(|(_, p)| p.route && p.kind == kind && Some(p.value) == self.pin)
                .flat_map(|(_, p)| [p.a, p.b])
                .filter(|&p| b.contains(p))
                .collect::<Vec<_>>()
        };
        let pins = contacts(PieceKind::Pin);
        let internal = contacts(PieceKind::Internal);
        pins.iter().any(|a| {
            internal
                .iter()
                .any(|b| (a[0] - b[0]).hypot(a[1] - b[1]) > ROUTE_TOUCH)
        })
    }
}

/// nextpnr's TRELLIS_COMB ports name functions within a slice, not always
/// LUT inputs/outputs. In particular F1/OFX are contacts on the F5/FX box.
fn site_pin(port: &str, lut: Option<usize>, ff: Option<usize>) -> Option<String> {
    if let Some(n) = lut {
        let slice = (b'A' + (n / 2) as u8) as char;
        let carry_in = || {
            if n / 2 == 0 {
                "FCI_SLICE".into()
            } else {
                format!("FCI{slice}_SLICE")
            }
        };
        let carry_out = || {
            if n / 2 == 3 {
                "FCO_SLICE".into()
            } else {
                format!("FCO{slice}_SLICE")
            }
        };
        Some(match port {
            "F1" => format!("F{}_SLICE", n | 1),
            "OFX" => format!("{}{slice}_SLICE", if n % 2 == 0 { "F5" } else { "FX" }),
            "FXA" | "FXB" => format!("{port}{slice}_SLICE"),
            "FCI" if n % 2 == 0 => carry_in(),
            "FCI" => format!("FCI{slice}1_SLICE"),
            "FCO" if n % 2 == 0 => format!("FCI{slice}1_SLICE"),
            "FCO" => carry_out(),
            _ => format!("{port}{n}_SLICE"),
        })
    } else {
        ff.map(|n| {
            format!(
                "{port}{}_SLICE",
                if matches!(port, "CLK" | "LSR" | "CE") {
                    n / 2
                } else {
                    n
                }
            )
        })
    }
}

/// The overlay has already split conductors at their junctions and tile
/// borders. Only shared endpoints join here; crossing interiors do not.
fn connection(
    pieces: &[(u32, &Piece)],
    source: &RouteEnd,
    sink: &RouteEnd,
    pin_matches: impl Fn(u32, u32) -> bool,
) -> Result<Vec<RoutePiece>, RouteError> {
    let key = |p: [f32; 2]| p.map(|v| (v / ROUTE_TOUCH).round() as i32);
    let mut nodes = Vec::new();
    let mut node_of = HashMap::new();
    let mut edges = Vec::new();
    for &(id, piece) in pieces.iter().filter(|(_, p)| p.route) {
        let mut node = |p| {
            *node_of.entry(key(p)).or_insert_with(|| {
                nodes.push(p);
                nodes.len() - 1
            })
        };
        edges.push((node(piece.a), node(piece.b), id));
    }
    let mut adjacent = vec![Vec::new(); nodes.len()];
    let mut site_pin = vec![false; nodes.len()];
    for (&(a, b, _), (_, piece)) in edges.iter().zip(pieces.iter().filter(|(_, p)| p.route)) {
        if matches!(
            piece.kind,
            PieceKind::Pin | PieceKind::Internal | PieceKind::Block
        ) {
            site_pin[a] = true;
            site_pin[b] = true;
        }
    }
    for &(a, b, piece) in &edges {
        let length = (nodes[a][0] - nodes[b][0]).hypot(nodes[a][1] - nodes[b][1]);
        adjacent[a].push((
            b,
            length,
            RoutePiece {
                piece,
                reverse: false,
            },
        ));
        adjacent[b].push((
            a,
            length,
            RoutePiece {
                piece,
                reverse: true,
            },
        ));
    }
    let ends = |end: &RouteEnd| {
        let candidates: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(i, p)| site_pin[*i] && end.contains(**p))
            .map(|(i, _)| i)
            .collect();
        let named: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|&i| {
                end.pin.as_ref().is_some_and(|pin| {
                    adjacent[i]
                        .iter()
                        .any(|(_, _, p)| pin_matches(p.piece, *pin))
                })
            })
            .collect();
        // The overlay can carry a fixed-alias or remapped LUT-input value
        // on the physical pin (e.g. ram_q[31]'s D0 is drawn as C0_SLICE).
        // Its routed pin endpoint on this exact box is authoritative; a
        // missing value match must not discard that contact. Track ends
        // and crossing interiors are never candidates.
        if !named.is_empty() {
            named
        } else {
            candidates
        }
    };
    let starts = ends(source);
    let targets = ends(sink);
    if starts.is_empty() {
        return Err(RouteError::DriverWire);
    }
    if targets.is_empty() {
        return Err(RouteError::SinkWire);
    }
    let mut distance = vec![f32::INFINITY; nodes.len()];
    let mut via = vec![None; nodes.len()];
    let mut heap = BinaryHeap::new();
    for start in starts {
        distance[start] = 0.0;
        heap.push(Reverse((0, start)));
    }
    while let Some(Reverse((d, at))) = heap.pop() {
        let d = f32::from_bits(d);
        if d > distance[at] {
            continue;
        }
        if targets.contains(&at) {
            let mut route = Vec::new();
            let mut at = at;
            while let Some((previous, piece)) = via[at] {
                route.push(piece);
                at = previous;
            }
            route.reverse();
            return if route.is_empty() {
                Err(RouteError::Disconnected)
            } else {
                Ok(route)
            };
        }
        for &(to, length, piece) in &adjacent[at] {
            let next = d + length;
            if next < distance[to] {
                distance[to] = next;
                via[to] = Some((at, piece));
                heap.push(Reverse((next.to_bits(), to)));
            }
        }
    }
    Err(RouteError::Disconnected)
}

fn mapped_route(
    hop: &Hop,
    nets: &[u32],
    placements: &BTreeMap<String, String>,
    model: &DeviceModel,
    overlay: &Overlay,
    shape: (usize, usize),
) -> Result<Vec<RoutePiece>, RouteError> {
    if nets.is_empty() {
        return Err(RouteError::NoNet);
    }
    let source = RouteEnd::placed(&hop.source, placements, model, overlay, shape)
        .ok_or(RouteError::DriverSite)?;
    let sink = hop
        .sink
        .as_ref()
        .and_then(|s| RouteEnd::placed(s, placements, model, overlay, shape))
        .ok_or(RouteError::SinkSite)?;
    let mut failure = RouteError::DriverWire;
    for &net in nets {
        let pieces: Vec<_> = overlay
            .pieces_of(net)
            .iter()
            .map(|&i| (i, &overlay.pieces[i as usize]))
            .collect();
        match connection(&pieces, &source, &sink, |piece, pin| {
            overlay.pieces[piece as usize].value == pin
        }) {
            Ok(route) => return Ok(route),
            Err(RouteError::Disconnected) if sink.undrawn_interior(&pieces) => {
                failure = RouteError::SiteInterior
            }
            Err(error) => failure = failure.max(error),
        }
    }
    Err(failure)
}

#[derive(Clone, Debug)]
pub struct MappedPath {
    pub path: Path,
    pub label: String,
    pub hops: Vec<MappedHop>,
}
#[derive(Debug)]
pub struct Published {
    pub result: Result<Vec<Arc<MappedPath>>, String>,
    pub seconds: f64,
    pub bytes: usize,
}
impl Report {
    pub fn map(
        self,
        names: &Names,
        model: &DeviceModel,
        overlay: &Overlay,
        sources: &Index,
    ) -> Vec<Arc<MappedPath>> {
        let shape = model.tiles().fold((0, 0), |(cols, rows), (_, t)| {
            (cols.max(t.col as usize + 1), rows.max(t.row as usize + 1))
        });
        self.map_with(
            &names.placements,
            sources,
            |n| names.overlay_nets(n, model, board::PACKAGE, overlay),
            |n| names.clock_name(n).map(str::to_string),
            |hop, nets| mapped_route(hop, nets, &names.placements, model, overlay, shape),
        )
    }

    fn map_with(
        self,
        placements: &BTreeMap<String, String>,
        sources: &Index,
        resolve: impl Fn(&str) -> Vec<u32>,
        clock_name: impl Fn(&str) -> Option<String>,
        route: impl Fn(&Hop, &[u32]) -> Result<Vec<RoutePiece>, RouteError>,
    ) -> Vec<Arc<MappedPath>> {
        let hdl = |nets: &[u32]| {
            nets.iter()
                .filter_map(|n| sources.nets.get(n))
                .flat_map(|n| &n.names)
                .filter(|n| !n.contains('$'))
                .min()
                .cloned()
        };
        self.paths
            .into_iter()
            .map(|path| {
                let clock_label = |n: &str| {
                    clock_name(n)
                        .or_else(|| hdl(&resolve(n)))
                        .unwrap_or_else(|| n.into())
                };
                let label = match &path.domain {
                    Domain::Clock { name, edges } if edges == "posedge -> posedge" => {
                        clock_label(name)
                    }
                    Domain::Clock { name, edges } => format!("{} ({edges})", clock_label(name)),
                    Domain::Cross { from, to } => {
                        let endpoint = |s: &str| {
                            s.split_once(' ').map_or_else(
                                || s.into(),
                                |(edge, n)| format!("{edge} {}", clock_label(n)),
                            )
                        };
                        format!("{} -> {}", endpoint(from), endpoint(to))
                    }
                };
                let hops = path
                    .hops
                    .iter()
                    .map(|hop| {
                        let mut nets = hop.net.as_deref().map(&resolve).unwrap_or_default();
                        nets.sort_unstable();
                        nets.dedup();
                        let mut sites = Vec::new();
                        let mut locations: Vec<_> = nets
                            .iter()
                            .flat_map(|n| sources.net_locations(*n))
                            .cloned()
                            .collect();
                        for endpoint in [Some(&hop.source), hop.sink.as_ref()].into_iter().flatten()
                        {
                            if let Some(site) = placements
                                .get(&endpoint.cell)
                                .and_then(|b| Site::from_bel(b))
                            {
                                if locations.is_empty() {
                                    locations.extend(sources.site_locations(
                                        site.tile.row,
                                        site.tile.col,
                                        &site.label,
                                    ));
                                }
                                if !sites.contains(&site) {
                                    sites.push(site);
                                }
                            }
                        }
                        locations.sort_unstable();
                        locations.dedup();
                        let mut mapped = hop.clone();
                        mapped.route.clear();
                        mapped.route_error = None;
                        if hop.kind == Kind::Routing {
                            match route(hop, &nets) {
                                Ok(pieces) if !pieces.is_empty() => mapped.route = pieces,
                                Ok(_) => mapped.route_error = Some(RouteError::Disconnected),
                                Err(error) => mapped.route_error = Some(error),
                            }
                        }
                        MappedHop {
                            hop: mapped,
                            name: hdl(&nets).unwrap_or_else(|| hop.name()),
                            nets,
                            sites,
                            locations,
                        }
                    })
                    .collect();
                Arc::new(MappedPath { path, label, hops })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_connection_excludes_fanout_and_unconnected_crossings() {
        use crate::fabric_overlay::Kind as PieceKind;
        let piece = |a, b, route| Piece {
            a,
            b,
            value: 0,
            d0: 0.0,
            d1: 1.0,
            kind: PieceKind::Pin,
            route,
        };
        // Reversed and shuffled pieces: the route must be ordered from the
        // driver's pin, through the shared trunk, to this sink's pin.
        let pieces = [
            piece([2.0, 0.0], [1.0, 0.0], true),
            piece([2.0, 0.0], [2.0, 2.0], true),
            piece([3.0, 0.0], [2.0, 0.0], true),
            piece([0.0, 0.0], [1.0, 0.0], true),
            piece([1.5, -1.0], [1.5, 1.0], true),
            piece([0.0, 0.0], [3.0, 0.0], false),
            // A second port on the same sink cell, on the other branch.
            piece([2.0, 2.0], [3.0, 2.0], true),
        ];
        let indexed: Vec<_> = pieces
            .iter()
            .enumerate()
            .map(|(i, p)| (i as u32, p))
            .collect();
        let source = RouteEnd {
            min: [-1.0, -0.5],
            max: [0.0, 0.5],
            pin: None,
            interior: None,
        };
        let sink = RouteEnd {
            min: [3.0, -0.5],
            max: [4.0, 2.5],
            pin: Some(2),
            interior: None,
        };
        let route = connection(&indexed, &source, &sink, |p, pin| p == 2 && pin == 2).unwrap();
        assert_eq!(
            route,
            vec![
                RoutePiece {
                    piece: 3,
                    reverse: false
                },
                RoutePiece {
                    piece: 0,
                    reverse: true
                },
                RoutePiece {
                    piece: 2,
                    reverse: true
                },
            ]
        );
        let disconnected = RouteEnd {
            min: [1.0, 1.0],
            max: [2.0, 1.5],
            pin: None,
            interior: None,
        };
        assert_eq!(
            connection(&indexed, &source, &disconnected, |_, _| false),
            Err(RouteError::Disconnected)
        );
        let absent = RouteEnd {
            min: [10.0, 10.0],
            max: [11.0, 11.0],
            pin: None,
            interior: None,
        };
        assert_eq!(
            connection(&indexed, &source, &absent, |_, _| false),
            Err(RouteError::SinkWire)
        );
        assert_eq!(
            connection(&indexed, &absent, &sink, |_, _| false),
            Err(RouteError::DriverWire)
        );
        // A track endpoint on a site's boundary is not a site-pin contact.
        let mut without_driver_pin = pieces;
        without_driver_pin[3].kind = PieceKind::Track;
        let indexed: Vec<_> = without_driver_pin
            .iter()
            .enumerate()
            .map(|(i, p)| (i as u32, p))
            .collect();
        assert_eq!(
            connection(&indexed, &source, &sink, |p, pin| p == pin),
            Err(RouteError::DriverWire)
        );
    }

    #[test]
    #[ignore = "needs local reference designs, Trellis and clang"]
    fn critical_reference_contacts_and_resolution() {
        // Reproduce the old LUT/FF-box contact test on the same overlay,
        // alongside the port-aware mapping. No baseline counts are inferred.
        fn legacy_end(
            endpoint: &Endpoint,
            names: &Names,
            s: &crate::live::Static,
        ) -> Option<RouteEnd> {
            let site = Site::from_bel(names.placements.get(&endpoint.cell)?)?;
            let p = site.tile.row as usize * s.fabric.cols as usize + site.tile.col as usize;
            let (min, max) = s
                .overlay
                .sites_in(p)
                .iter()
                .find(|b| b.label == site.label)
                .map(|b| (b.min, b.max))
                .or_else(|| {
                    s.overlay
                        .boxes_in(p)
                        .iter()
                        .find(|b| b.label == site.label)
                        .map(|b| (b.min, b.max))
                })?;
            Some(RouteEnd {
                min,
                max,
                pin: None,
                interior: None,
            })
        }
        for design in ["blinky_cycle", "picorv32_console"] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/reference")
                .join(design);
            let shared = Arc::new(std::sync::Mutex::new(crate::live::Shared::default()));
            let commands = crate::live::spawn(
                crate::live::Design {
                    config: root.join("top.config"),
                    names: Some(root.join("top_routed.json")),
                    run: false,
                },
                Arc::clone(&shared),
                || {},
            );
            let start = std::time::Instant::now();
            let s = loop {
                let state = shared.lock().unwrap();
                assert!(state.error.is_none(), "{:?}", state.error);
                if let Some(s) = state.ready.as_ref().filter(|s| s.timing.get().is_some()) {
                    break Arc::clone(s);
                }
                assert!(start.elapsed().as_secs() < 180);
                drop(state);
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let names =
                Names::load(&root.join("top_routed.json"), &s.model, board::PACKAGE).unwrap();
            let paths = s.timing.get().unwrap().result.as_ref().unwrap();
            assert!(!paths.is_empty());
            let (mut before, mut after, mut unresolved) = (0, 0, 0);
            let routing_hops = paths
                .iter()
                .flat_map(|path| &path.path.hops)
                .filter(|hop| hop.kind == Kind::Routing)
                .count();
            for path in paths {
                let expected = &path.path;
                assert_eq!(path.hops.len(), expected.hops.len());
                assert_eq!(path.hops.last().unwrap().hop.total, expected.total());
                for (hop, expected) in path.hops.iter().zip(&expected.hops) {
                    assert_eq!(hop.hop.source, expected.source);
                    assert_eq!(hop.hop.sink, expected.sink);
                    assert_eq!(hop.hop.kind, expected.kind);
                    assert_eq!(hop.hop.total, expected.total);
                    if hop.hop.kind != Kind::Routing {
                        continue;
                    }
                    let legacy = legacy_end(&hop.hop.source, &names, &s).zip(legacy_end(
                        hop.hop.sink.as_ref().unwrap(),
                        &names,
                        &s,
                    ));
                    if let Some((source, sink)) = legacy {
                        before += usize::from(hop.nets.iter().any(|&net| {
                            let pieces: Vec<_> = s
                                .overlay
                                .pieces_of(net)
                                .iter()
                                .map(|&id| (id, &s.overlay.pieces[id as usize]))
                                .collect();
                            connection(&pieces, &source, &sink, |_, _| false).is_ok()
                        }));
                    }
                    if let Some(reason) = hop.route_not_shown() {
                        unresolved += 1;
                        println!("# {design} unresolved {}: {reason}", hop.name);
                        assert!(hop.hop.route_error.is_some(), "missing route explanation");
                        continue;
                    }
                    after += 1;
                    let shape = (s.fabric.cols as usize, s.fabric.rows as usize);
                    let source = RouteEnd::placed(
                        &hop.hop.source,
                        &names.placements,
                        &s.model,
                        &s.overlay,
                        shape,
                    )
                    .unwrap();
                    let sink = RouteEnd::placed(
                        hop.hop.sink.as_ref().unwrap(),
                        &names.placements,
                        &s.model,
                        &s.overlay,
                        shape,
                    )
                    .unwrap();
                    let mut last = None;
                    for (i, route) in hop.hop.route.iter().enumerate() {
                        assert!(hop
                            .nets
                            .iter()
                            .any(|&n| s.overlay.pieces_of(n).contains(&route.piece)));
                        let p = &s.overlay.pieces[route.piece as usize];
                        assert!(p.route);
                        let (a, b) = if route.reverse {
                            (p.b, p.a)
                        } else {
                            (p.a, p.b)
                        };
                        if i == 0 {
                            assert!(source.contains(a), "{}: driver", hop.name);
                        }
                        if let Some(previous) = last {
                            let previous: [f32; 2] = previous;
                            assert!(
                                (a[0] - previous[0]).hypot(a[1] - previous[1]) < 0.001,
                                "{}: gap",
                                hop.name
                            );
                        }
                        last = Some(b);
                    }
                    assert!(sink.contains(last.unwrap()), "{}: sink", hop.name);
                }
            }
            println!("# {design} resolved {before} -> {after}, unresolved {unresolved}");
            assert!(after > 0, "{design}: no resolved route exercised");
            assert!(after >= before, "{design}: contact resolution regressed");
            assert_eq!(after + unresolved, routing_hops, "{design}");
            drop(commands);
        }
    }

    const FIXTURES: [(&str, &str, &str, usize, f64, f64); 3] = [
        (
            "blinky_cycle",
            include_str!("../tests/fixtures/timing_path/blinky_cycle/creator.txt"),
            include_str!("../tests/fixtures/timing_path/blinky_cycle/nextpnr.log"),
            19,
            2.82,
            354.99,
        ),
        (
            "picorv32_console",
            include_str!("../tests/fixtures/timing_path/picorv32_console/creator.txt"),
            include_str!("../tests/fixtures/timing_path/picorv32_console/nextpnr.log"),
            19,
            11.43,
            87.46,
        ),
        (
            "life_full",
            include_str!("../tests/fixtures/timing_path/life_full/creator.txt"),
            include_str!("../tests/fixtures/timing_path/life_full/nextpnr.log"),
            49,
            31.17,
            32.09,
        ),
    ];
    #[test]
    fn pinned_logs() {
        for (name, creator, log, count, total, mhz) in FIXTURES {
            let report = parse(creator, log).unwrap();
            let clock = report
                .paths
                .iter()
                .find(|p| p.domain.clock().is_some())
                .unwrap();
            assert_eq!(clock.hops.len(), count, "{name}");
            assert_eq!(clock.total(), total, "{name}");
            assert_eq!(clock.frequency.unwrap().mhz, mhz, "{name}");
            for path in &report.paths {
                let mut prev = 0.0;
                for hop in &path.hops {
                    assert!(close(hop.total, prev + hop.curr), "{name}: {hop:?}");
                    prev = hop.total;
                }
                assert!(close(prev, path.logic + path.routing));
            }
            println!(
                "{name}: {} domains, {} hops, {total:.2} ns, {mhz:.2} MHz",
                report.paths.len(),
                clock.hops.len()
            );
        }
    }
    #[test]
    fn rejects_unknown_versions_and_partial_reports() {
        let (_, creator, log, ..) = FIXTURES[0];
        assert_eq!(
            parse(
                "Next Generation Place and Route (Version nextpnr-0.12)",
                log
            )
            .unwrap_err(),
            "critical path: nextpnr 0.12 not supported"
        );
        for bad in [
            log.replace("routing  1.10", "unknown  1.10"),
            log.replace("Sink led_TRELLIS_FF_Q.DI", "not a sink"),
            log.replace("1.59 ns logic, 1.23 ns routing", "1.59 ns logic"),
            log.replace(
                "Info:      setup  0.00  2.82 Source led_TRELLIS_FF_Q.DI\n",
                "",
            ),
            log.replace("routing  1.10  1.62", "routing  1.10  9.99"),
            log.replace("(2,37) -> (2,38)", "(x,37) -> (2,38)"),
            log.replace("clk-to-q  0.52", "clk-to-q  0.520"),
            format!("{log}\nInfo: Critical path report for clock 'unfinished'"),
        ] {
            assert_eq!(parse(creator, &bad).unwrap_err(), NO_PATH);
        }
        assert_eq!(parse(creator, "").unwrap_err(), NO_PATH);
    }
    #[test]
    fn last_pass_and_error_frequency() {
        let (_, creator, log, ..) = FIXTURES[0];
        let repeated = format!(
            "{log}\n{}",
            log.replace("354.99 MHz (PASS at 25.00", "12.00 MHz (FAIL at 25.00")
                .replace("Info: Max frequency", "ERROR: Max frequency")
                .replace("routing  1.46  1.98", "routing  1.47  1.99")
                .replace(
                    "0.52 ns logic, 1.46 ns routing",
                    "0.52 ns logic, 1.47 ns routing"
                )
        );
        let report = parse(creator, &repeated).unwrap();
        assert_eq!(report.paths.len(), 2);
        let p = &report.paths[0];
        assert_eq!(p.frequency.unwrap().mhz, 12.0);
        assert_eq!(p.frequency.unwrap().pass, Some(false));
        assert!(p.slack().unwrap() < 0.0);
        assert_eq!(report.paths[1].total(), 1.99);
    }

    #[test]
    fn mapping_uses_overlay_identity_placements_and_project_index() {
        use crate::sources::{Collected, Files};
        use serde_json::json;
        let (_, creator, log, ..) = FIXTURES[0];
        let top = json!({
            "netnames": {"count[0]": {"bits": [1], "attributes": {"src": "top.v:99.1-99.8|/library/map.v:4"}}},
            "cells": {"led_TRELLIS_FF_Q_7": {
                "attributes": {"NEXTPNR_BEL": "X2/Y37/SLICEA.FF0", "src": "top.v:101.1-101.8|/library/map.v:6"},
                "port_directions": {"Q": "output"}, "connections": {"Q": [1]}
            }}
        });
        let resolve = |n: &str| if n == "count[0]" { vec![42] } else { vec![] };
        let sources =
            Collected::from_top(&top, &Files::project(".".into(), ["top.v".into()])).build(resolve);
        let placements =
            BTreeMap::from([("led_TRELLIS_FF_Q_7".into(), "X2/Y37/SLICEA.FF0".into())]);
        let paths = parse(creator, log).unwrap().map_with(
            &placements,
            &sources,
            resolve,
            |_| Some("clk_25mhz".into()),
            |_, _| Err(RouteError::SinkSite),
        );
        let path = &paths[0];
        assert_eq!(path.label, "clk_25mhz");
        let first = &path.hops[0];
        assert_eq!(
            first.sites,
            vec![Site {
                tile: Tile { col: 2, row: 37 },
                label: "FF0".into()
            }]
        );
        assert_eq!(first.locations[0].label(), "top.v:101");
        let routed = &path.hops[1];
        assert_eq!(routed.nets, vec![42]);
        assert_eq!(routed.name, "count[0]");
        assert_eq!(routed.locations[0].label(), "top.v:101");
        assert!(routed.note(2).ends_with("-> top.v:101"));
        assert_eq!(
            routed.route_not_shown(),
            Some(RouteError::SinkSite.reason())
        );
        assert_eq!(first.route_not_shown(), None);
        let internal = &path.hops[3];
        assert!(internal.nets.is_empty());
        assert_eq!(
            internal.hop.tiles,
            Some((Tile { col: 2, row: 38 }, Tile { col: 2, row: 38 }))
        );
        assert!(paths
            .iter()
            .flat_map(|p| &p.hops)
            .flat_map(|h| &h.locations)
            .all(|s| s.file.as_ref() == "top.v"));
    }
}
