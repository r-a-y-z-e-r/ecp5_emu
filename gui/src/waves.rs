//! The waveform view: nets of the design picked by name, from the map or
//! the inspector, and their values after every clock edge over the last
//! cycles, as traces under the map (src/live.rs `Command::Watch`, which the
//! run answers with a row of the watched wires after each edge). A click on
//! the traces goes to that cycle of the run, so the map, the inspector and
//! the board show that moment.
//!
//! The run samples a net after every edge only where its fast-mode program
//! keeps the net up to date (`FastProgram::fresh`: registers, inputs and the
//! lines of the settles); a net folded into the table of the gate that reads
//! it is drawn as not sampled, never with a wrong value.
use crate::{
    fabric_view::{source_menu, source_unavailable},
    theme,
};
use ecp5_emu::{
    fast::NOT_SAMPLED,
    live::{Command, Static, WaveRun},
    sources::{Index, SourceKind, SourceLocation, Span},
};
use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Vec2};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// No row for that edge: the run has not sent one.
const NONE: u8 = NOT_SAMPLED - 1;
/// How many cycles of rows are kept, and how many nets at most.
pub const KEEP_CYCLES: u64 = 4096;
pub const MOST_NETS: usize = 64;
/// Heights and widths, in points, fixed whatever is shown: the panel never
/// resizes itself under the pointer.
const ROW: f32 = 18.0;
const NAMES: f32 = 200.0;
const AXIS: f32 = 18.0;
const EMPTY: &str = "No nets watched yet: type an HDL name above, right-click a wire on the map → Watch this net, or press W over a wire.";
const KEYBOARD: &str =
    "W: click the map first (a text field has the keyboard), or right-click a wire";
const SEARCH_LIMIT: usize = 50;

/// A search result keeps the resolver's name separate from the bus label.
#[derive(Debug, PartialEq)]
struct Suggestion {
    name: String,
    label: String,
    bus: bool,
}

fn suggestions(names: &BTreeMap<String, u32>, query: &str) -> Vec<Suggestion> {
    let query = query.trim().to_lowercase();
    let rank = |name: &str| {
        let name = name.to_lowercase();
        if name.starts_with(&query) {
            Some(0)
        } else {
            let mut chars = name.chars();
            query.chars().all(|c| chars.any(|n| n == c)).then_some(1)
        }
    };
    let mut matches = Vec::new();
    let mut buses: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for name in names.keys() {
        if let Some((base, bit)) = bus_of(name) {
            buses.entry(base).or_default().push(bit);
        }
        if let Some(rank) = rank(name) {
            matches.push((
                rank,
                Suggestion {
                    name: name.clone(),
                    label: name.clone(),
                    bus: false,
                },
            ));
        }
    }
    for (base, bits) in buses {
        if let Some(rank) = rank(base) {
            let lo = bits.iter().min().expect("bus has a bit");
            let hi = bits.iter().max().expect("bus has a bit");
            matches.push((
                rank,
                Suggestion {
                    name: base.into(),
                    label: format!("{base}[{hi}:{lo}]  {} bits", bits.len()),
                    bus: true,
                },
            ));
        }
    }
    matches.sort_by(|(a, x), (b, y)| (a, &x.name, !x.bus).cmp(&(b, &y.name, !y.bus)));
    matches.truncate(SEARCH_LIMIT);
    matches
        .into_iter()
        .map(|(_, suggestion)| suggestion)
        .collect()
}

/// A net watched: its HDL name (else "net N"), and the wire of the overlay
/// the run samples for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Watched {
    pub name: String,
    pub net: u32,
    pub wire: u32,
}

/// What is shown in one row: a net, or the nets of a bus (`name[i]`, most
/// significant first) as one value.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Bit(usize),
    Bus { name: String, bits: Vec<usize> },
}

/// A row's cross-probe, shared by the menu and window scripts.
#[derive(Debug, PartialEq)]
pub enum RowAction {
    Chip {
        name: String,
        nets: Vec<u32>,
    },
    Source {
        name: String,
        locations: Vec<SourceLocation>,
        chosen: Span,
    },
}

/// A missing row is distinct from a row with an unavailable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowError {
    NotRow,
    Unavailable(&'static str),
}

/// Counts since the last note, in edges (one row per edge).
#[derive(Default)]
struct Taken {
    lists: BTreeMap<u64, (u64, u64)>,
    earlier: u64,
    width: u64,
    trimmed: u64,
    branched: u64,
    missing: BTreeMap<u32, u64>,
}

struct Drawn {
    names: Vec<String>,
    #[cfg(test)]
    rect: Rect,
    first: u64,
    last: u64,
    present: u64,
    end: u64,
    following: bool,
}

pub struct Waves {
    pub nets: Vec<Watched>,
    /// The list's number, sent with it; rows of an earlier list are not
    /// taken.
    id: u64,
    /// The list changed since it was last sent to the run.
    changed: bool,
    pub keep_cycles: u64,
    /// Rows from edge `lo` on, a byte per net each (`NONE` where none came).
    lo: u64,
    rows: Vec<u8>,
    /// The current publication is independent of sampled history and time zoom.
    published: Option<(u64, HashMap<u32, u8>)>,
    /// The cursor sample chosen this frame, for the fixed column heading.
    column_cursor: Option<u64>,
    /// The end of the recorded future last seen after going back.
    recorded_to: Option<u64>,
    /// Points per edge, and the last edge shown (none: the newest row or
    /// the published edge, whichever is later).
    scale: f32,
    end: Option<u64>,
    pub open: bool,
    query: String,
    search_open: bool,
    selected: usize,
    /// A picked signal, including one already watched, to bring into view.
    scroll_to: Option<u32>,
    /// Buses shown as their bits.
    open_buses: HashSet<String>,
    /// The design's HDL names and their nets, to add by name, found once.
    names: Option<(usize, BTreeMap<String, u32>)>,
    /// What the last add could not do.
    note: Option<String>,
    /// Lines for the window's record, without depending on its writer.
    pub recording: bool,
    notes: Vec<String>,
    taken: Taken,
    drawn: Option<Drawn>,
    sampled: HashMap<u32, bool>,
    taken_at: f64,
    drawn_at: f64,
    watch_status: Option<String>,
    pub sources: Option<Arc<Index>>,
    pub project_sources: bool,
    pub row_action: Option<RowAction>,
}

impl Default for Waves {
    fn default() -> Self {
        Self {
            nets: Vec::new(),
            id: 0,
            changed: false,
            keep_cycles: KEEP_CYCLES,
            lo: 0,
            rows: Vec::new(),
            published: None,
            column_cursor: None,
            recorded_to: None,
            scale: 2.0,
            end: None,
            open: false,
            query: String::new(),
            search_open: false,
            selected: 0,
            scroll_to: None,
            open_buses: HashSet::new(),
            names: None,
            note: None,
            recording: false,
            notes: Vec::new(),
            taken: Taken::default(),
            drawn: None,
            sampled: HashMap::new(),
            taken_at: f64::NEG_INFINITY,
            drawn_at: f64::NEG_INFINITY,
            watch_status: None,
            sources: None,
            project_sources: false,
            row_action: None,
        }
    }
}

impl Waves {
    fn note_record(&mut self, line: String) {
        if self.recording {
            self.notes.push(line);
        }
    }

    fn add_note(&mut self, how: &str, target: &str, result: &str) {
        self.note_record(format!("add\t{how}\t{target}\t{result}"));
    }

    pub fn set_open(&mut self, open: bool) {
        if self.open != open {
            self.open = open;
            self.note_record(if open { "opened" } else { "closed" }.into());
        }
    }

    /// The button and script use the same kept net.
    pub fn add_shown(&mut self, s: &Static, net: Option<u32>, how: &str) {
        if let Some(net) = net {
            if how == "shown" {
                self.add(s, net, None);
            } else {
                self.add_from(s, net, None, how);
            }
        } else {
            let note = "no net kept on the map (click a wire first)";
            self.note = Some(note.to_string());
            self.add_note(how, "shown", note);
        }
    }

    /// A map menu names its net when opened, and needs no keyboard.
    pub fn add_menu(&mut self, s: &Static, net: u32) {
        self.add_from(s, net, None, "menu");
    }

    /// W and the script both respect a text field's keyboard ownership.
    pub fn add_pointed(&mut self, s: &Static, net: Option<u32>, keyboard: bool, how: &str) {
        if keyboard {
            self.add_note(how, "pointed", "W ignored: a text field has the keyboard");
        } else if let Some(net) = net {
            self.add_from(s, net, None, how);
        } else {
            self.add_note(how, "pointed", "no net under the pointer");
        }
    }

    /// The window writes these lines with the seconds since ready first.
    pub fn notes(&mut self, seconds: f64, status: Option<&str>, edge: u64) -> Vec<String> {
        if self.watch_status.as_deref() != status {
            self.watch_status = status.map(str::to_string);
            self.note_record(format!("status\tedge {edge}\t{}", status.unwrap_or("none")));
        }
        if seconds - self.taken_at >= 1.0 && (!self.nets.is_empty() || !self.taken.lists.is_empty())
        {
            self.taken_at = seconds;
            let taken = std::mem::take(&mut self.taken);
            self.note_record(format!(
                "taken\tid {}\tlists {:?} (id: runs, edges)\tkept {}..{}\tdropped earlier list {}\tdifferent width {}\ttrimmed {}\ttrimmed new input {}\tNOT_SAMPLED {:?} (net: rows)",
                self.id, taken.lists, self.lo, self.hi(), taken.earlier,
                taken.width, taken.trimmed, taken.branched, taken.missing
            ));
        }
        if self.open && seconds - self.drawn_at >= 1.0 {
            if let Some(d) = self.drawn.take() {
                self.drawn_at = seconds;
                self.note_record(format!(
                    "drawn\tlines {} {:?}\tedges {}..={}\tpresent {}\tend {}\tfollowing {}",
                    d.names.len(),
                    d.names,
                    d.first,
                    d.last,
                    d.present,
                    d.end,
                    d.following
                ));
            }
        }
        std::mem::take(&mut self.notes)
    }

    /// The design closed or another opened: nothing watched, the panel as
    /// it was.
    pub fn reset(&mut self) {
        let open = self.open;
        let id = self.id;
        let recording = self.recording;
        *self = Self {
            open,
            id: id + 1,
            recording,
            ..Self::default()
        };
    }

    /// Watches `net` of the design, as `name`: whether it was added (not
    /// already watched, room left, a wire of the design).
    pub fn add(&mut self, s: &Static, net: u32, name: Option<String>) -> bool {
        self.add_from(s, net, name, "shown")
    }

    /// A source probe already resolved the overlay net. Preserve its alias
    /// when named, and allow generated nets with no readable alias too.
    pub fn add_net_from(&mut self, s: &Static, net: u32, name: Option<String>, how: &str) -> bool {
        self.add_from(
            s,
            net,
            Some(name.unwrap_or_else(|| format!("net {net}"))),
            how,
        )
    }

    fn add_from(&mut self, s: &Static, net: u32, name: Option<String>, how: &str) -> bool {
        let target = format!("net {net}\tname {}", name.as_deref().unwrap_or(""));
        if let Some(watched) = self.nets.iter().find(|w| w.net == net) {
            if how == "source" {
                if let Some((base, _)) = bus_of(&watched.name) {
                    self.open_buses.insert(base.to_owned());
                }
            }
            self.add_note(how, &target, "already watched");
            return false;
        }
        if self.nets.len() >= MOST_NETS {
            self.add_note(how, &target, "full");
            self.note = Some(format!("At most {MOST_NETS} nets are watched"));
            return false;
        }
        let Some(&piece) = s.overlay.pieces_of(net).first() else {
            self.add_note(how, &target, "no wire on the map");
            self.note = Some(format!("Net {net} has no wire on the map"));
            return false;
        };
        let wire = s.overlay.pieces[piece as usize].value;
        let name = name
            .or_else(|| s.net_names.get(&net).and_then(|n| n.first().cloned()))
            .unwrap_or_else(|| format!("net {net}"));
        self.add_watched(Watched { name, net, wire }, how);
        self.note = None;
        self.add_note(how, &target, "added; freshness pending first row");
        self.set_open(true);
        true
    }

    fn add_watched(&mut self, watched: Watched, how: &str) {
        // Watching a source register promises its bits, including in the tour.
        // Repeating the action opens a bus the visitor may have collapsed.
        if how == "source" {
            if let Some((base, _)) = bus_of(&watched.name) {
                self.open_buses.insert(base.to_owned());
            }
        }
        let mut nets = self.nets.clone();
        nets.push(watched);
        self.set(nets);
    }

    /// Watches the nets of an HDL name, or of every bit of a bus `name`
    /// (`name[i]`): how many were added.
    pub fn add_name(&mut self, s: &Static, name: &str) -> usize {
        self.add_name_from(s, name, "name")
    }

    pub fn add_name_from(&mut self, s: &Static, name: &str, how: &str) -> usize {
        let names = self.names_of(s);
        let mut found: Vec<(String, u32)> = match names.get(name) {
            Some(&net) => vec![(name.to_string(), net)],
            None => names
                .iter()
                .filter(|(n, _)| bus_of(n).is_some_and(|(base, _)| base == name))
                .map(|(n, &net)| (n.clone(), net))
                .collect(),
        };
        found.sort_by_key(|(n, _)| std::cmp::Reverse(bus_of(n).map(|b| b.1)));
        let mut added = 0;
        if found.is_empty() {
            self.add_note(how, name, "no net named");
        }
        for (n, net) in found {
            added += usize::from(self.add_from(s, net, Some(n), how));
        }
        if added == 0 && self.note.is_none() {
            self.note = Some(format!("No net named {name}"));
        }
        added
    }

    /// The design's HDL observations. Names resolves a port to its fabric
    /// data connection before these labels reach the window, excluding T.
    /// Remaining aliases use their largest drawn net.
    fn names_of(&mut self, s: &Static) -> &BTreeMap<String, u32> {
        let key = s as *const Static as usize;
        if self.names.as_ref().is_none_or(|n| n.0 != key) {
            let mut names: BTreeMap<String, (u32, usize)> = BTreeMap::new();
            for (&net, list) in &s.net_names {
                let size = s.overlay.pieces_of(net).len();
                for n in list {
                    let n = n.split(" (").next().unwrap_or(n).to_string();
                    let e = names.entry(n).or_insert((net, size));
                    if size > e.1 {
                        *e = (net, size);
                    }
                }
            }
            self.names = Some((
                key,
                names.into_iter().map(|(n, (net, _))| (n, net)).collect(),
            ));
        }
        &self.names.as_ref().expect("found above").1
    }

    /// Watches nothing.
    pub fn clear(&mut self) {
        self.set(Vec::new());
    }

    /// Watches `nets` in place of the list: the rows of nets watched before
    /// are kept, the new ones have none yet.
    fn set(&mut self, nets: Vec<Watched>) {
        let (old, new) = (self.nets.len(), nets.len());
        let from: Vec<Option<usize>> = nets
            .iter()
            .map(|n| self.nets.iter().position(|o| o.net == n.net))
            .collect();
        let edges = if old == 0 { 0 } else { self.rows.len() / old };
        let mut rows = vec![NONE; edges * new];
        for e in 0..edges {
            for (k, f) in from.iter().enumerate() {
                if let Some(f) = f {
                    rows[e * new + k] = self.rows[e * old + f];
                }
            }
        }
        self.rows = if new == 0 { Vec::new() } else { rows };
        self.nets = nets;
        self.sampled
            .retain(|net, _| self.nets.iter().any(|n| n.net == *net));
        self.id += 1;
        self.changed = true;
    }

    /// The command that tells the run what to watch, once after each change.
    pub fn command(&mut self) -> Option<Command> {
        std::mem::take(&mut self.changed).then(|| {
            self.note_record(format!("watch\tid {}\twires {}", self.id, self.nets.len()));
            Command::Watch {
                id: self.id,
                wires: self.nets.iter().map(|n| n.wire).collect(),
                keep: self.keep_cycles * 2,
            }
        })
    }

    /// Takes the rows the run sent (`Shared::waves`). `present` is the edge
    /// the run is at; `ahead`, the edges recorded ahead of it after going
    /// back (none once a new input has changed the run from there).
    pub fn take(&mut self, runs: Vec<WaveRun>, present: u64, ahead: u64) {
        // Discard the old branch before inserting any newly delivered rows.
        self.trim(present, ahead);
        let width = self.nets.len();
        for run in runs {
            if self.recording {
                let counts = self.taken.lists.entry(run.id).or_default();
                counts.0 += 1;
                counts.1 += run.edges();
                if run.id != self.id {
                    self.taken.earlier += run.edges();
                } else if run.width != width || width == 0 {
                    self.taken.width += run.edges();
                }
            }
            if run.id == self.id && run.width == width && width > 0 {
                if self.recording {
                    for (k, net) in self.nets.iter().enumerate() {
                        let missing = run
                            .rows
                            .chunks_exact(width)
                            .filter(|row| row[k] == NOT_SAMPLED)
                            .count() as u64;
                        *self.taken.missing.entry(net.net).or_default() += missing;
                        if let Some(row) = run.rows.get(..width) {
                            let fresh = row[k] != NOT_SAMPLED;
                            if self.sampled.insert(net.net, fresh) != Some(fresh) {
                                self.notes.push(format!(
                                    "freshness\tid {}\tnet {}\tname {}\tedge {}\t{}",
                                    self.id,
                                    net.net,
                                    net.name,
                                    run.first,
                                    if fresh {
                                        "sampled; running program keeps it"
                                    } else {
                                        "NOT_SAMPLED; needs a program switch or has no signal"
                                    }
                                ));
                            }
                        }
                    }
                }
                self.insert(run.first, &run.rows);
            }
        }
        self.trim_old(present);
    }

    /// Rows from edge `first` on, over those there.
    fn insert(&mut self, first: u64, rows: &[u8]) {
        let width = self.nets.len();
        let n = (rows.len() / width) as u64;
        let keep = self.keep_cycles * 2;
        let hi = self.hi();
        // Far from what is kept (going back a long way): start again there.
        if self.rows.is_empty() || first + n + keep < self.lo || first > hi + keep {
            self.taken.trimmed += (self.rows.len() / width) as u64;
            self.lo = first;
            self.rows = rows.to_vec();
            return;
        }
        if first < self.lo {
            let mut grown = vec![NONE; (self.lo - first) as usize * width];
            grown.extend_from_slice(&self.rows);
            self.rows = grown;
            self.lo = first;
        }
        if first + n > self.hi() {
            self.rows
                .resize((first + n - self.lo) as usize * width, NONE);
        }
        let at = (first - self.lo) as usize * width;
        self.rows[at..at + rows.len()].copy_from_slice(rows);
    }

    /// The edge after the last row.
    fn hi(&self) -> u64 {
        self.lo + self.rows.len().checked_div(self.nets.len()).unwrap_or(0) as u64
    }

    /// Only a new input after going back invalidates rows ahead. Delivery
    /// precedes publishing, so other rows past `present` are legitimate.
    fn trim(&mut self, present: u64, ahead: u64) {
        let width = self.nets.len();
        if width > 0 && ahead == 0 && self.recorded_to.is_some_and(|end| present < end) {
            let keep = (present + 1).saturating_sub(self.lo) as usize * width;
            if keep < self.rows.len() {
                self.taken.branched += ((self.rows.len() - keep) / width) as u64;
                self.rows.truncate(keep);
            }
        }
        self.recorded_to = (ahead > 0).then_some(present + ahead);
        self.trim_old(present);
    }

    /// Keep the last cycles before the published edge, and all newer rows.
    fn trim_old(&mut self, present: u64) {
        let width = self.nets.len();
        if width == 0 {
            return;
        }
        let from = present.saturating_sub(self.keep_cycles * 2);
        if self.lo < from {
            let drop = ((from - self.lo) as usize * width).min(self.rows.len());
            self.rows.drain(..drop);
            self.taken.trimmed += (drop / width) as u64;
            self.lo = from;
        }
    }

    /// Net `k`'s value after edge `edge`: 0, 1, 2 (X), 3 (Z),
    /// `NOT_SAMPLED`, or `NONE`.
    fn value(&self, edge: u64, k: usize) -> u8 {
        let width = self.nets.len();
        if edge < self.lo || edge >= self.hi() {
            return NONE;
        }
        self.rows[(edge - self.lo) as usize * width + k]
    }

    /// How many nets the last row has not sampled: folded by the running
    /// program.
    fn not_sampled(&self) -> usize {
        let last = self.hi().saturating_sub(1);
        (0..self.nets.len())
            .filter(|&k| self.value(last, k) == NOT_SAMPLED)
            .count()
    }

    /// Watches the first `n` HDL names of the design (by name), for a
    /// script's `watch N`: how many were added.
    pub fn add_first(&mut self, s: &Static, n: usize) -> usize {
        let names: Vec<(String, u32)> = self
            .names_of(s)
            .iter()
            .map(|(name, &net)| (name.clone(), net))
            .collect();
        let mut added = 0;
        for (name, net) in names {
            if added == n {
                break;
            }
            added += usize::from(self.add_from(s, net, Some(name), "script"));
        }
        added
    }

    /// The rows shown: a bus's bits together unless it is opened, most
    /// significant first in either case. Only the display order changes.
    fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut k = 0;
        while k < self.nets.len() {
            let base = bus_of(&self.nets[k].name).map(|b| b.0.to_string());
            let mut bits = vec![k];
            while let (Some(b), Some(next)) = (&base, self.nets.get(k + bits.len())) {
                if bus_of(&next.name).is_some_and(|n| n.0 == b) {
                    bits.push(k + bits.len());
                } else {
                    break;
                }
            }
            match base {
                Some(name) if bits.len() > 1 => {
                    k += bits.len();
                    bits.sort_by_key(|&i| {
                        std::cmp::Reverse(bus_of(&self.nets[i].name).map(|b| b.1))
                    });
                    if self.open_buses.contains(&name) {
                        lines.extend(bits.into_iter().map(Line::Bit));
                    } else {
                        lines.push(Line::Bus { name, bits });
                    }
                }
                _ => {
                    lines.push(Line::Bit(k));
                    k += 1;
                }
            }
        }
        lines
    }

    /// Resolve watched rows, including a bus whose bits are currently open.
    pub fn row_nets(&self, name: &str) -> Vec<u32> {
        let mut nets: Vec<_> = if let Some(bit) = self.nets.iter().find(|n| n.name == name) {
            vec![bit.net]
        } else {
            self.nets
                .iter()
                .filter(|n| bus_of(&n.name).is_some_and(|(base, _)| base == name))
                .map(|n| n.net)
                .collect()
        };
        nets.sort_unstable();
        nets.dedup();
        nets
    }

    fn row_sources(&self, name: &str) -> Result<Vec<SourceLocation>, &'static str> {
        let index = self
            .sources
            .as_deref()
            .ok_or_else(|| source_unavailable(None, self.project_sources))?;
        if index.locations == 0 {
            return Err(source_unavailable(Some(index), self.project_sources));
        }
        let mut locations: Vec<_> = self
            .row_nets(name)
            .into_iter()
            .flat_map(|net| index.net_source_choices(net))
            .collect();
        // Match the map's default: assignments before declarations, merging
        // expressions on the same line within each kind across bus bits.
        locations.sort_by_key(|location| match location.kind {
            SourceKind::Driven => 0,
            SourceKind::Declared => 1,
            SourceKind::Cell => 2,
        });
        let mut seen = HashSet::new();
        locations.retain(|location| {
            seen.insert((
                location.kind,
                location.span.file.clone(),
                location.span.line,
            ))
        });
        if locations.is_empty() {
            Err(source_unavailable(Some(index), self.project_sources))
        } else {
            Ok(locations)
        }
    }

    pub fn chip_action(&self, name: &str) -> Option<RowAction> {
        let nets = self.row_nets(name);
        (!nets.is_empty()).then(|| RowAction::Chip {
            name: name.into(),
            nets,
        })
    }

    pub fn source_action(&self, name: &str, chosen: Option<Span>) -> Option<RowAction> {
        self.source_action_result(name, chosen).ok()
    }

    fn source_action_result(
        &self,
        name: &str,
        chosen: Option<Span>,
    ) -> Result<RowAction, &'static str> {
        let locations = self.row_sources(name)?;
        let chosen = chosen.unwrap_or_else(|| locations[0].span.clone());
        Ok(RowAction::Source {
            name: name.into(),
            locations,
            chosen,
        })
    }

    /// Scripts must distinguish unknown row names before testing availability.
    pub fn action_for_row(&self, name: &str, source: bool) -> Result<RowAction, RowError> {
        if self.row_nets(name).is_empty() {
            return Err(RowError::NotRow);
        }
        if source {
            self.source_action_result(name, None)
                .map_err(RowError::Unavailable)
        } else {
            self.chip_action(name).ok_or(RowError::NotRow)
        }
    }

    fn row_menu(&mut self, ui: &mut egui::Ui, name: &str) {
        if ui.button("Show on the chip").clicked() {
            self.row_action = self.chip_action(name);
            ui.close();
        }
        match self.row_sources(name) {
            Ok(locations) => {
                if let Some(chosen) = source_menu(ui, &locations, None) {
                    self.row_action = self.source_action(name, Some(chosen));
                    ui.close();
                }
            }
            Err(reason) => {
                source_menu(ui, &[], Some(reason));
            }
        }
    }

    /// A line's value after `edge`: a bit as it is; a bus as a number when
    /// every bit is 0 or 1 (`Ok`), else the worst of its bits (`Err`).
    fn line_value(&self, line: &Line, edge: u64) -> Result<u64, u8> {
        Self::bits_value(line, |k| self.value(edge, k))
    }

    fn bits_value(line: &Line, value: impl Fn(usize) -> u8) -> Result<u64, u8> {
        match line {
            Line::Bit(k) => match value(*k) {
                v @ (0 | 1) => Ok(u64::from(v)),
                v => Err(v),
            },
            Line::Bus { bits, .. } => {
                let mut n = 0u64;
                let mut worst = None;
                for &k in bits {
                    match value(k) {
                        v @ (0 | 1) => n = n << 1 | u64::from(v),
                        v => worst = Some(worst.map_or(v, |w: u8| w.max(v))),
                    }
                }
                worst.map_or(Ok(n), Err)
            }
        }
    }

    /// Current values from the same publication as the frame's edge. A cursor
    /// still reads sampled history, including its unavailable/folded values.
    pub fn publish(&mut self, edge: u64, values: &[u8]) {
        self.published = Some((
            edge,
            self.nets
                .iter()
                .map(|n| (n.net, values.get(n.wire as usize).copied().unwrap_or(NONE)))
                .collect(),
        ));
    }

    fn column_value(&self, line: &Line, cursor: Option<u64>, present: u64) -> Result<u64, u8> {
        if let Some(edge) = cursor {
            return self.line_value(line, edge);
        }
        if let Some((edge, values)) = &self.published {
            if *edge == present {
                return Self::bits_value(line, |k| {
                    values.get(&self.nets[k].net).copied().unwrap_or(NONE)
                });
            }
        }
        self.line_value(line, present)
    }

    fn add_suggestion(&mut self, s: &Static, suggestion: &Suggestion) {
        let names = self.names_of(s);
        let mut found: Vec<_> = names
            .iter()
            .filter(|(name, _)| {
                if suggestion.bus {
                    bus_of(name).is_some_and(|(base, _)| base == suggestion.name)
                } else {
                    **name == suggestion.name
                }
            })
            .map(|(name, &net)| (name.clone(), net))
            .collect();
        found.sort_by_key(|(name, _)| std::cmp::Reverse(bus_of(name).map(|b| b.1)));
        self.note = None;
        let mut target = None;
        for (name, net) in found {
            if !suggestion.bus && !self.nets.iter().any(|watched| watched.net == net) {
                self.add_name(s, &name);
            } else {
                self.add_from(s, net, Some(name), "name");
            }
            if target.is_none() && self.nets.iter().any(|watched| watched.net == net) {
                target = Some(net);
            }
        }
        self.scroll_to = target;
        self.query.clear();
        self.search_open = false;
    }

    fn search(&mut self, ui: &mut egui::Ui, s: Option<&Static>) {
        let id = ui.make_persistent_id("waves_signal_search");
        // Consume navigation before TextEdit can move the caret or give up focus.
        let (down, up, enter, escape) = if ui.memory(|m| m.has_focus(id)) {
            ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                )
            })
        } else {
            (false, false, false, false)
        };
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(id)
                .hint_text(if s.is_some() {
                    "Search HDL signals…"
                } else {
                    "Load a design to add signals"
                })
                .desired_width(220.0),
        );
        if field.changed() || field.gained_focus() || field.clicked() {
            self.search_open = true;
            self.selected = 0;
            self.note = None;
        }
        if down || up {
            self.search_open = true;
        }
        if escape {
            self.search_open = false;
        }
        if !self.search_open {
            return;
        }
        let query = self.query.clone();
        let matches = s.map_or_else(Vec::new, |s| suggestions(self.names_of(s), &query));
        self.selected = self.selected.min(matches.len().saturating_sub(1));
        if down {
            self.selected = (self.selected + 1).min(matches.len().saturating_sub(1));
        } else if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if enter {
            if let (Some(s), Some(suggestion)) = (s, matches.get(self.selected)) {
                self.add_suggestion(s, suggestion);
            }
            return;
        }
        let mut open = self.search_open;
        let mut picked = None;
        egui::Popup::from_response(&field)
            .open_bool(&mut open)
            .show(|ui| {
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        if s.is_none() || self.names.as_ref().is_some_and(|(_, n)| n.is_empty()) {
                            ui.label("Load a design to add signals");
                        } else if matches.is_empty() {
                            ui.label(format!("No signal matches “{}”", self.query.trim()));
                        }
                        for (i, suggestion) in matches.iter().enumerate() {
                            let row = ui.selectable_label(i == self.selected, &suggestion.label);
                            if (down || up || field.changed()) && i == self.selected {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                picked = Some(i);
                            }
                        }
                    });
            });
        self.search_open = open;
        if let (Some(s), Some(i)) = (s, picked) {
            self.add_suggestion(s, &matches[i]);
        }
    }

    /// The panel: a line of controls, the cycles' axis and a row per net or
    /// bus. `present` is the edge the run is at. Returns a cycle to go to
    /// (a click on the traces).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        s: Option<&Static>,
        present: u64,
        shown_net: Option<u32>,
        watch_status: Option<&str>,
    ) -> Option<u64> {
        let palette = theme::colours(ui.visuals().dark_mode);
        let mut go = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Waves").strong());
            ui.label(
                egui::RichText::new(format!(
                    "{} of {MOST_NETS} nets, last {} cycles",
                    self.nets.len(),
                    self.keep_cycles
                ))
                .small()
                .weak(),
            );
            self.search(ui, s);
            let Some(s) = s else {
                return;
            };
            if ui
                .button("Add the net shown")
                .on_hover_text("The net kept on the map (a click on a wire, or a net in the inspector). W adds the net under the pointer.")
                .clicked()
            {
                self.add_shown(s, shown_net, "shown");
            }
            if ui.add_enabled(!self.nets.is_empty(), egui::Button::new("Clear")).clicked() {
                self.clear();
            }
            if ui
                .add_enabled(self.end.is_some(), egui::Button::new("Follow"))
                .on_hover_text("Follow the newest rows and the present (double-click the traces)")
                .clicked()
            {
                self.end = None;
            }
            if let Some(note) = &self.note {
                ui.colored_label(palette.error, egui::RichText::new(note).small());
            }
            let folded = self.not_sampled();
            if folded > 0 {
                ui.label(egui::RichText::new(format!("{folded} not sampled (hatched)")).small().weak())
                    .on_hover_text("The running fast-mode program folds these nets into the tables of the gates that read them, so they are up to date only after a full settle, not after every edge. Watching them in a program that keeps them is the next step of this view.");
            }
            if let Some(status) = watch_status {
                ui.label(egui::RichText::new(status).small().weak())
                    .on_hover_text(status);
            }
        });
        // Reserve the same toolbar height whether or not a field owns W.
        let hint = if ui.ctx().egui_wants_keyboard_input() {
            KEYBOARD
        } else {
            "W over a wire, or right-click a wire on the map → Watch this net"
        };
        let (hint_rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
        ui.put(
            Rect::from_min_max(
                Pos2::new(hint_rect.left() + NAMES, hint_rect.top()),
                hint_rect.max,
            ),
            egui::Label::new(egui::RichText::new(hint).small().weak()).truncate(),
        );
        self.column_cursor = None;
        let lines = self.lines();
        let height = AXIS + ROW * lines.len().max(1) as f32;
        // Content height must never become the panel's minimum height. Keep a
        // bounded viewport even while the resize edge crosses the last row.
        let viewport = ui.available_rect_before_wrap();
        let names = Rect::from_min_max(
            viewport.min,
            Pos2::new(
                (viewport.left() + NAMES).min(viewport.right()),
                viewport.bottom(),
            ),
        );
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("waves_list")
                .max_rect(viewport),
        );
        list_ui.set_clip_rect(ui.clip_rect().intersect(viewport));
        egui::ScrollArea::vertical()
            .id_salt("waves_rows")
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .max_height(viewport.height().max(0.0))
            .scroll_source(egui::scroll_area::ScrollSource {
                mouse_wheel: ui.rect_contains_pointer(names),
                drag: egui::scroll_area::DragScroll::Never,
                ..Default::default()
            })
            .show(&mut list_ui, |ui| {
                let width = ui.available_width();
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
                if let Some(net) = self.scroll_to.take() {
                    let row = lines.iter().position(|line| match line {
                        Line::Bit(k) => self.nets[*k].net == net,
                        Line::Bus { bits, .. } => bits.iter().any(|&k| self.nets[k].net == net),
                    });
                    if let Some(row) = row {
                        let top = rect.top() + AXIS + row as f32 * ROW;
                        ui.scroll_to_rect(
                            Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(width, ROW)),
                            Some(egui::Align::Center),
                        );
                    }
                }
                go = self.draw(ui, rect, &response, &lines, present);
            });
        ui.advance_cursor_after_rect(viewport);
        let at = self.column_cursor.unwrap_or(present);
        ui.painter().text(
            hint_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            format!(
                "{} @ {}{}",
                if self.column_cursor.is_some() {
                    "Cursor"
                } else {
                    "Current"
                },
                at / 2,
                if at % 2 == 1 { "½" } else { "" }
            ),
            egui::FontId::monospace(11.0),
            ui.visuals().weak_text_color(),
        );
        go
    }

    /// The axis and the rows in `rect`, the pointer's cursor, and what the
    /// pointer does there.
    fn draw(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        response: &egui::Response,
        lines: &[Line],
        present: u64,
    ) -> Option<u64> {
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let palette = theme::colours(visuals.dark_mode);
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        let traces = Rect::from_min_max(Pos2::new(rect.left() + NAMES, rect.top()), rect.max);
        // A pinned range can expire while hidden or while the run advances.
        // Resume following instead of drawing an empty, permanently stale span.
        if self.end.is_some_and(|end| end < self.lo) {
            self.end = None;
        }
        // The last edge shown, and the edge at a point.
        let following = if self.rows.is_empty() {
            present
        } else {
            present.max(self.hi().saturating_sub(1))
        };
        let end = self.end.unwrap_or(following).max(1);
        let scale = self.scale;
        let x_of = |edge: f64| traces.right() - (end as f64 - edge) as f32 * scale;
        let edge_at = |x: f32| {
            (end as f64 - ((traces.right() - x) / scale) as f64)
                .round()
                .max(0.0) as u64
        };
        // Wheel: zoom about the pointer; drag: move along; double-click: follow.
        let mut go = None;
        if let Some(p) = response.hover_pos().filter(|p| traces.contains(*p)) {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zoom * (scroll / 200.0).exp();
            if factor != 1.0 {
                let at = edge_at(p.x) as f64;
                self.scale = (self.scale * factor).clamp(0.02, 40.0);
                let right = at + ((traces.right() - p.x) / self.scale) as f64;
                self.end = Some(right.max(1.0) as u64);
            }
            if response.clicked() {
                go = Some(edge_at(p.x) / 2);
            }
        }
        if response.dragged() {
            let moved = (response.drag_delta().x / self.scale) as f64;
            self.end = Some((end as f64 - moved).max(1.0) as u64);
        }
        if response.double_clicked() {
            self.end = None;
        }
        let first = edge_at(traces.left()).max(self.lo);
        let last = end.min(self.hi().saturating_sub(1));
        if self.recording {
            let clip = ui.clip_rect().intersect(rect);
            let names = lines
                .iter()
                .enumerate()
                .filter(|(row, _)| {
                    let top = rect.top() + AXIS + *row as f32 * ROW;
                    top < clip.bottom() && top + ROW > clip.top()
                })
                .map(|(_, line)| match line {
                    Line::Bit(k) => self.nets[*k].name.clone(),
                    Line::Bus { name, bits } => format!("{name}[{} bits]", bits.len()),
                })
                .collect();
            self.drawn = Some(Drawn {
                names,
                #[cfg(test)]
                rect,
                first,
                last,
                present,
                end,
                following: self.end.is_none(),
            });
        }
        if lines.is_empty() {
            painter.text(
                rect.left_top() + Vec2::new(0.0, AXIS),
                egui::Align2::LEFT_TOP,
                EMPTY,
                egui::FontId::proportional(12.0),
                weak,
            );
            return None;
        }
        // The axis: cycles at steps of 1, 2 or 5 times a power of ten, at
        // least 70 points apart.
        let per_cycle = 2.0 * scale;
        let mut step = 1u64;
        while (step as f32) * per_cycle < 70.0 {
            step = match step.to_string().as_bytes()[0] {
                b'1' => step * 2,
                b'2' => step / 2 * 5,
                _ => step * 2,
            };
        }
        let axis = Rect::from_min_max(traces.min, Pos2::new(traces.right(), traces.top() + AXIS));
        let mut c = (edge_at(traces.left()) / 2).div_ceil(step) * step;
        while (c * 2) as f64 <= end as f64 {
            let x = x_of((c * 2) as f64);
            painter.line_segment(
                [
                    Pos2::new(x, axis.bottom() - 4.0),
                    Pos2::new(x, axis.bottom()),
                ],
                Stroke::new(1.0, weak),
            );
            painter.text(
                Pos2::new(x + 2.0, axis.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{c}"),
                egui::FontId::monospace(11.0),
                weak,
            );
            c += step;
        }
        // The present, and what lies ahead of it (recorded, after going back).
        let now = x_of(present as f64);
        if now >= traces.left() && now <= traces.right() {
            painter.line_segment(
                [
                    Pos2::new(now, traces.top()),
                    Pos2::new(now, traces.bottom()),
                ],
                Stroke::new(1.5, palette.net),
            );
        }
        let cursor = response
            .hover_pos()
            .filter(|p| traces.contains(*p))
            .map(|p| edge_at(p.x));
        self.column_cursor = cursor;
        for (row, line) in lines.iter().enumerate() {
            let top = rect.top() + AXIS + row as f32 * ROW;
            let band = Rect::from_min_max(
                Pos2::new(traces.left(), top + 3.0),
                Pos2::new(traces.right(), top + ROW - 3.0),
            );
            if row % 2 == 1 {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(rect.left(), top),
                        Pos2::new(rect.right(), top + ROW),
                    ),
                    0.0,
                    visuals.faint_bg_color,
                );
            }
            // The name, and the cursor sample or the latest publication.
            let name = match line {
                Line::Bit(k) => self.nets[*k].name.clone(),
                Line::Bus { name, bits } => format!("{name}[{}]", bits.len()),
            };
            let value = match self.column_value(line, cursor, present) {
                Ok(n) => match line {
                    Line::Bit(_) => format!("{n}"),
                    Line::Bus { bits, .. } => format!("{n:0w$X}", w = bits.len().div_ceil(4)),
                },
                Err(2) => "X".into(),
                Err(3) => "Z".into(),
                Err(NOT_SAMPLED) => "?".into(),
                Err(_) => String::new(),
            };
            let names = Rect::from_min_max(
                Pos2::new(rect.left(), top),
                Pos2::new(rect.left() + NAMES - 8.0, top + ROW),
            );
            let galley = painter.layout_no_wrap(name, egui::FontId::proportional(12.0), text);
            let clip = painter.with_clip_rect(names.intersect(rect));
            clip.galley(
                Pos2::new(
                    names.left() + 18.0,
                    names.center().y - galley.size().y / 2.0,
                ),
                galley,
                text,
            );
            painter.text(
                Pos2::new(names.right(), names.center().y),
                egui::Align2::RIGHT_CENTER,
                value,
                egui::FontId::monospace(11.0),
                palette.net,
            );
            // A small × to stop watching (a bus: all its bits), a ▸ to open a bus.
            let close = Rect::from_center_size(
                Pos2::new(names.left() + 8.0, names.center().y),
                Vec2::splat(12.0),
            );
            let hovered = response.hover_pos().is_some_and(|p| close.contains(p));
            painter.text(
                close.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(12.0),
                if hovered { palette.error } else { weak },
            );
            if hovered && response.clicked() {
                let remove: Vec<usize> = match line {
                    Line::Bit(k) => vec![*k],
                    Line::Bus { bits, .. } => bits.clone(),
                };
                let nets = self
                    .nets
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !remove.contains(i))
                    .map(|(_, n)| n.clone())
                    .collect();
                self.set(nets);
                return None;
            }
            let label = Rect::from_min_max(Pos2::new(names.left() + 16.0, top), names.max);
            let row_name = match line {
                Line::Bit(k) => self.nets[*k].name.clone(),
                Line::Bus { name, .. } => name.clone(),
            };
            let label_response = ui.interact(
                label,
                response.id.with(("name", row)),
                Sense::click_and_drag(),
            );
            label_response.context_menu(|ui| self.row_menu(ui, &row_name));
            if label_response.dragged() && !response.dragged() {
                let moved = (label_response.drag_delta().x / self.scale) as f64;
                self.end = Some((end as f64 - moved).max(1.0) as u64);
            }
            if label_response.double_clicked() {
                self.end = None;
            }
            if label_response.clicked()
                || (response.hover_pos().is_some_and(|p| label.contains(p)) && response.clicked())
            {
                let base = match line {
                    Line::Bus { name, .. } => Some(name.clone()),
                    Line::Bit(k) => bus_of(&self.nets[*k].name).map(|b| b.0.to_string()),
                };
                if let Some(base) = base {
                    if !self.open_buses.remove(&base) {
                        self.open_buses.insert(base);
                    }
                }
                return None;
            }
            self.trace(
                &painter,
                line,
                band,
                (first, last),
                (&x_of, present),
                visuals,
            );
        }
        if let Some(c) = cursor {
            let x = x_of(c as f64);
            painter.line_segment(
                [Pos2::new(x, traces.top()), Pos2::new(x, traces.bottom())],
                Stroke::new(1.0, weak),
            );
            painter.text(
                Pos2::new(x + 3.0, traces.top() + 1.0),
                egui::Align2::LEFT_TOP,
                format!("{}{}", c / 2, if c % 2 == 1 { "½" } else { "" }),
                egui::FontId::monospace(11.0),
                text,
            );
        }
        go
    }

    /// New deliveries can precede a publish. Only the future recorded
    /// after going back is dimmed, as far as that recording reaches.
    fn recorded_ahead(&self, edge: u64, present: u64) -> bool {
        edge > present && self.recorded_to.is_some_and(|end| edge <= end)
    }

    /// One line's trace over edges `first..=last` in `band`: 0 and 1 as low
    /// and high, X red, Z in the middle, not sampled hatched; a bus as its
    /// value between transitions; many changes within a point as a block.
    fn trace(
        &self,
        painter: &egui::Painter,
        line: &Line,
        band: Rect,
        (first, last): (u64, u64),
        (x_of, present): (&dyn Fn(f64) -> f32, u64),
        visuals: &egui::Visuals,
    ) {
        if first > last {
            return;
        }
        let palette = theme::colours(visuals.dark_mode);
        let colour = visuals.text_color();
        let (high, low, middle) = (band.top(), band.bottom(), band.center().y);
        let bus = matches!(line, Line::Bus { .. });
        // Runs of one value, each drawn from where it starts to where the
        // next does (a value holds after its edge until the next).
        let mut run_start = first;
        let mut run_value = self.line_value(line, first);
        let mut busy: Option<(f32, egui::Color32)> = None;
        let flush_busy = |busy: &mut Option<(f32, egui::Color32)>, x: f32| {
            if let Some((from, colour)) = busy.take() {
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(from, high), Pos2::new(x.max(from + 1.0), low)),
                    0.0,
                    colour.gamma_multiply(0.5),
                );
            }
        };
        let mut previous: Option<Result<u64, u8>> = None;
        let mut edge = first;
        while edge <= last + 1 {
            let value = if edge <= last {
                self.line_value(line, edge)
            } else {
                Err(NONE)
            };
            if edge <= last
                && value == run_value
                && self.recorded_ahead(edge, present) == self.recorded_ahead(run_start, present)
            {
                edge += 1;
                continue;
            }
            let (x0, x1) = (x_of(run_start as f64), x_of(edge as f64));
            let dim = if self.recorded_ahead(run_start, present) {
                0.45
            } else {
                1.0
            };
            let stroke_colour = colour.gamma_multiply(dim);
            if busy.is_some_and(|(_, colour)| colour != stroke_colour) {
                flush_busy(&mut busy, x0);
            }
            if x1 - x0 < 1.5 && run_value != Err(NONE) {
                busy.get_or_insert((x0, stroke_colour));
            } else {
                flush_busy(&mut busy, x0);
                let stroke = Stroke::new(1.5, stroke_colour);
                match run_value {
                    Ok(n) if !bus => {
                        let y = if n == 1 { high } else { low };
                        painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], stroke);
                        if let Some(Ok(p)) = previous {
                            if p != n {
                                painter.line_segment(
                                    [Pos2::new(x0, high), Pos2::new(x0, low)],
                                    stroke,
                                );
                            }
                        }
                    }
                    Ok(n) => {
                        let s = 3.0f32.min((x1 - x0) / 2.0);
                        let shape = vec![
                            Pos2::new(x0, middle),
                            Pos2::new(x0 + s, high),
                            Pos2::new(x1 - s, high),
                            Pos2::new(x1, middle),
                            Pos2::new(x1 - s, low),
                            Pos2::new(x0 + s, low),
                        ];
                        painter.add(egui::Shape::closed_line(shape, stroke));
                        let digits = match line {
                            Line::Bus { bits, .. } => bits.len().div_ceil(4),
                            Line::Bit(_) => 1,
                        };
                        let label = format!("{n:0digits$X}");
                        if (x1 - x0) > 8.0 * label.len() as f32 + 6.0 {
                            painter.text(
                                Pos2::new((x0 + x1) / 2.0, middle),
                                egui::Align2::CENTER_CENTER,
                                label,
                                egui::FontId::monospace(10.5),
                                stroke_colour,
                            );
                        }
                    }
                    Err(2) => {
                        painter.rect_filled(
                            Rect::from_min_max(Pos2::new(x0, high), Pos2::new(x1, low)),
                            0.0,
                            palette.error.gamma_multiply(0.35 * dim),
                        );
                    }
                    Err(3) => {
                        painter.line_segment(
                            [Pos2::new(x0, middle), Pos2::new(x1, middle)],
                            Stroke::new(1.5, palette.unknown.gamma_multiply(dim)),
                        );
                    }
                    Err(NOT_SAMPLED) => {
                        let hatch = Stroke::new(1.0, palette.muted.gamma_multiply(0.6 * dim));
                        let mut x = x0 - (x0 % 6.0);
                        while x < x1 {
                            let a = Pos2::new(x.max(x0), low);
                            let b = Pos2::new((x + (low - high)).min(x1), high);
                            painter.line_segment([a, b], hatch);
                            x += 6.0;
                        }
                    }
                    Err(_) => {}
                }
            }
            previous = Some(run_value);
            run_start = edge;
            run_value = value;
            edge += 1;
        }
        flush_busy(&mut busy, x_of(last as f64 + 1.0));
    }
}

/// A bus bit's name split: `name[i]` as (`name`, i).
fn bus_of(name: &str) -> Option<(&str, u32)> {
    let open = name.rfind('[')?;
    let index = name[open + 1..].strip_suffix(']')?.parse().ok()?;
    Some((&name[..open], index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watched(names: &[&str]) -> Vec<Watched> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Watched {
                name: n.to_string(),
                net: i as u32,
                wire: i as u32,
            })
            .collect()
    }

    fn waves(names: &[&str]) -> Waves {
        let mut w = Waves::default();
        w.set(watched(names));
        w
    }

    fn run(w: &Waves, first: u64, rows: &[u8]) -> WaveRun {
        WaveRun {
            id: w.id,
            first,
            width: w.nets.len(),
            rows: rows.to_vec(),
        }
    }

    fn panel_frame(
        w: &mut Waves,
        ctx: &egui::Context,
        present: u64,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Rect>) {
        let mut rect = None;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                events,
                ..Default::default()
            },
            |ui| {
                if w.open {
                    rect = Some(
                        egui::Panel::bottom("waves")
                            .resizable(true)
                            .default_size(240.0)
                            .min_size(96.0)
                            .show(ui, |ui| w.show(ui, None, present, None, None))
                            .response
                            .rect,
                    );
                }
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label("Map");
                });
            },
        );
        (out, rect)
    }

    fn many_waves() -> Waves {
        let names: Vec<_> = (0..MOST_NETS).map(|i| format!("signal_{i}")).collect();
        let mut w = waves(&names.iter().map(String::as_str).collect::<Vec<_>>());
        w.open = true;
        w.recording = true;
        w
    }

    #[test]
    fn wheel_scrolls_rows_only_over_names_and_zoom_only_over_traces() {
        for column in ["names", "traces", "toolbar", "map"] {
            let ctx = egui::Context::default();
            let mut w = many_waves();
            for _ in 0..3 {
                panel_frame(&mut w, &ctx, 100, vec![]);
            }
            let d = w.drawn.as_ref().unwrap().rect;
            let pointer = match column {
                "names" => Pos2::new(d.left() + 70.0, d.top() + AXIS + ROW),
                "traces" => Pos2::new(d.left() + NAMES + 100.0, d.top() + AXIS + ROW),
                "toolbar" => Pos2::new(100.0, d.top() - 15.0),
                _ => Pos2::new(100.0, 100.0),
            };
            let top = d.top();
            let scale = w.scale;
            panel_frame(
                &mut w,
                &ctx,
                100,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -100.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            );
            for _ in 0..8 {
                panel_frame(&mut w, &ctx, 100, vec![]);
            }
            let moved = w.drawn.as_ref().unwrap().rect.top() < top - 1.0;
            assert_eq!(moved, column == "names", "{column}");
            assert_eq!(w.scale != scale, column == "traces", "{column}");
        }
    }

    #[test]
    fn a_pinned_view_reopens_with_published_rows_after_history_expires() {
        let ctx = egui::Context::default();
        let mut w = many_waves();
        w.keep_cycles = 4;
        w.take(vec![run(&w, 1, &vec![0; MOST_NETS * 4])], 4, 0);
        w.end = Some(4);
        panel_frame(&mut w, &ctx, 4, vec![]);
        w.set_open(false);
        for edge in 5..=40 {
            w.take(vec![run(&w, edge, &[(edge % 2) as u8; MOST_NETS])], edge, 0);
            panel_frame(&mut w, &ctx, edge, vec![]);
        }
        w.set_open(true);
        for edge in 41..=45 {
            w.take(vec![run(&w, edge, &[(edge % 2) as u8; MOST_NETS])], edge, 0);
            panel_frame(&mut w, &ctx, edge, vec![]);
            let d = w.drawn.as_ref().unwrap();
            assert!(
                d.first <= d.last,
                "empty reopened span {}..={}",
                d.first,
                d.last
            );
            assert_eq!(d.end, edge);
            assert!(!d.names.is_empty());
            assert_eq!(w.value(edge, 0), (edge % 2) as u8);
        }
    }

    #[test]
    fn value_column_uses_published_edge_even_when_the_view_is_pinned_or_ahead() {
        let ctx = egui::Context::default();
        let mut w = waves(&["led[1]", "led[0]"]);
        w.recording = true;
        w.open = true;
        w.take(vec![run(&w, 1, &[0, 0, 0, 1, 1, 0])], 2, 0);
        w.end = Some(1);
        let (out, _) = panel_frame(&mut w, &ctx, 2, vec![]);
        text_at(&out, "1");
        text_at(&out, "Current @ 1");
        w.end = None;
        let (out, _) = panel_frame(&mut w, &ctx, 2, vec![]);
        text_at(&out, "1");
        let d = w.drawn.as_ref().unwrap().rect;
        let cursor = Pos2::new(d.right(), d.top() + AXIS + ROW / 2.0);
        let (out, _) = panel_frame(&mut w, &ctx, 2, vec![egui::Event::PointerMoved(cursor)]);
        text_at(&out, "Cursor @ 1½");
        text_at(&out, "2");
    }

    #[test]
    fn current_publication_updates_without_rewriting_cursor_samples() {
        let mut w = waves(&["led[1]", "led[0]"]);
        w.take(vec![run(&w, 1, &[NOT_SAMPLED, NOT_SAMPLED])], 1, 0);
        let line = w.lines().remove(0);
        w.publish(1, &[1, 0]);
        assert_eq!(w.column_value(&line, None, 1), Ok(2));
        assert_eq!(w.column_value(&line, Some(1), 1), Err(NOT_SAMPLED));
        w.publish(1, &[0, 1]);
        assert_eq!(w.column_value(&line, None, 1), Ok(1));
        assert_eq!(w.column_value(&line, Some(1), 1), Err(NOT_SAMPLED));
        w.publish(1, &[]);
        assert_eq!(w.column_value(&line, None, 1), Err(NONE));
    }

    #[test]
    fn many_rows_do_not_force_panel_height_and_the_edge_can_shrink_it() {
        for count in [0, 1, MOST_NETS] {
            let ctx = egui::Context::default();
            let mut w = many_waves();
            w.nets.truncate(count);
            let mut panel = None;
            for _ in 0..3 {
                panel = panel_frame(&mut w, &ctx, 100, vec![]).1;
            }
            let before = panel.unwrap();
            assert!((before.height() - 240.0).abs() < 2.0, "{count}: {before:?}");
            let edge = Pos2::new(before.center().x, before.top());
            panel_frame(&mut w, &ctx, 100, vec![egui::Event::PointerMoved(edge)]);
            panel_frame(
                &mut w,
                &ctx,
                100,
                pointer_button(edge, egui::PointerButton::Primary, true),
            );
            let target = edge + Vec2::new(0.0, 200.0);
            for _ in 0..3 {
                panel_frame(&mut w, &ctx, 100, vec![egui::Event::PointerMoved(target)]);
            }
            panel_frame(
                &mut w,
                &ctx,
                100,
                pointer_button(target, egui::PointerButton::Primary, false),
            );
            let after = panel_frame(&mut w, &ctx, 100, vec![]).1.unwrap();
            assert!(
                (after.height() - 96.0).abs() < 2.0,
                "{count}: {before:?} -> {after:?}"
            );
            w.set_open(false);
            panel_frame(&mut w, &ctx, 101, vec![]);
            w.set_open(true);
            let reopened = panel_frame(&mut w, &ctx, 102, vec![]).1.unwrap();
            assert!((reopened.height() - after.height()).abs() < 2.0);
        }
    }

    #[test]
    #[ignore = "headless blinky reference design, native reader and device database"]
    fn blinky_current_column_and_reopening_receive_running_publications() {
        use ecp5_emu::live::{self, Design, Shared};
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/blinky_cycle");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let s = loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if let Some(s) = &sh.ready {
                break s.clone();
            }
            assert!(Instant::now() < deadline, "{}", sh.stage);
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut w = Waves {
            recording: true,
            keep_cycles: 4,
            ..Default::default()
        };
        assert_eq!(w.add_name(&s, "led"), 8);
        commands.send(w.command().unwrap()).unwrap();
        commands.send(Command::Rate(Some(20.0))).unwrap();
        commands.send(Command::Run).unwrap();
        let ctx = egui::Context::default();
        let mut values = HashSet::new();
        let mut closed_at = None;
        let mut reopened = 0;
        let mut received_after_reopen = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (runs, edge, lit, leds, running) = {
                let mut sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{:?}", sh.error);
                (
                    std::mem::take(&mut sh.waves),
                    sh.edges,
                    sh.lit.clone(),
                    sh.leds,
                    sh.running,
                )
            };
            let received = runs.iter().any(|r| r.id == w.id && !r.rows.is_empty());
            w.take(runs, edge, 0);
            if received && closed_at.is_none() {
                w.end = Some(edge);
                closed_at = Some(edge);
                w.set_open(false);
            } else if !w.open && closed_at.is_some_and(|closed| edge > closed + 20) {
                w.set_open(true);
            }
            if w.open && leds.iter().all(Option::is_some) {
                w.publish(edge, &lit);
                let line = w.lines().remove(0);
                let expected = leds.iter().enumerate().fold(0u64, |value, (bit, high)| {
                    value | (u64::from(high.expect("blinky LED published")) << bit)
                });
                assert_eq!(w.column_value(&line, None, edge), Ok(expected));
                let (out, _) = panel_frame(&mut w, &ctx, edge, vec![]);
                text_at(&out, &format!("{expected:02X}"));
                values.insert(expected);
                if closed_at.is_some() {
                    let d = w.drawn.as_ref().unwrap();
                    assert!(d.first <= d.last);
                    assert!(!d.names.is_empty());
                    reopened += 1;
                    received_after_reopen |= received;
                }
            } else {
                panel_frame(&mut w, &ctx, edge, vec![]);
            }
            if reopened >= 5 && received_after_reopen && values.len() >= 5 && running {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "values {values:?}, reopened {reopened}, receiving {received_after_reopen}"
            );
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    #[test]
    fn signal_search_ranks_prefixes_then_subsequences_and_limits_rows() {
        let mut names: BTreeMap<_, _> = (0..8).map(|i| (format!("led[{i}]"), i)).collect();
        names.insert("a.deep.logic.enable_d".into(), 20);
        names.insert("top.worker.ready".into(), 21);
        let found = suggestions(&names, "LED");
        assert_eq!(found[0].label, "led[7:0]  8 bits");
        assert!(found[0].bus);
        assert_eq!(found[1].name, "led[0]");
        assert!(!found[1].bus);
        assert_eq!(found.last().unwrap().name, "a.deep.logic.enable_d");
        assert_eq!(suggestions(&names, "twrkdy")[0].name, "top.worker.ready");
        assert!(suggestions(&names, "zzz").is_empty());
        for i in 0..100 {
            names.insert(format!("led_extra_{i}"), 100 + i);
        }
        assert_eq!(suggestions(&names, "l").len(), SEARCH_LIMIT);
    }

    fn search_frame(
        w: &mut Waves,
        ctx: &egui::Context,
        s: Option<&Static>,
        events: Vec<egui::Event>,
        focus: bool,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))),
                events,
                ..Default::default()
            },
            |ui| {
                if focus {
                    let id = ui.make_persistent_id("waves_signal_search");
                    ui.memory_mut(|m| m.request_focus(id));
                }
                w.search(ui, s);
            },
        )
    }

    fn search_key(key: egui::Key) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn signal_search_without_a_design_explains_loading_and_escape_closes() {
        let ctx = egui::Context::default();
        let mut w = Waves::default();
        search_frame(&mut w, &ctx, None, vec![], true);
        let out = search_frame(&mut w, &ctx, None, vec![], false);
        text_at(&out, "Load a design to add signals");
        assert!(w.search_open);
        search_frame(&mut w, &ctx, None, search_key(egui::Key::Escape), false);
        assert!(!w.search_open);
        search_frame(&mut w, &ctx, None, vec![], false);
        assert!(!w.search_open);
    }

    #[test]
    fn picking_a_watched_signal_scrolls_its_row_into_view() {
        let ctx = egui::Context::default();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |style| {
                style.scroll_animation = egui::style::ScrollAnimation::none();
            });
        }
        let names: Vec<_> = (0..50).map(|i| format!("signal_{i}")).collect();
        let mut w = waves(&names.iter().map(String::as_str).collect::<Vec<_>>());
        w.recording = true;
        let size = Vec2::new(1000.0, 500.0);
        for _ in 0..2 {
            probe_frame(&mut w, &ctx, size, vec![]);
        }
        assert!(!w.drawn.as_ref().unwrap().names.contains(&names[49]));
        w.scroll_to = Some(w.nets[49].net);
        for _ in 0..3 {
            probe_frame(&mut w, &ctx, size, vec![]);
        }
        assert!(w.scroll_to.is_none());
        assert!(w.drawn.as_ref().unwrap().names.contains(&names[49]));
        assert_eq!(w.nets.len(), 50);
    }

    /// The real routed aliases and overlay, driven by egui input without a window.
    #[test]
    #[ignore = "headless reference design, native reader and device database"]
    fn signal_search_blinky_enter_bus_bits_fuzzy_and_duplicates() {
        use ecp5_emu::live::{self, Design, Shared};
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/blinky_cycle");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let runner = live::spawn(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            shared.clone(),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let s = loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if let Some(s) = &sh.ready {
                break s.clone();
            }
            assert!(Instant::now() < deadline, "{}", sh.stage);
            drop(sh);
            std::thread::sleep(Duration::from_millis(10));
        };
        let ctx = egui::Context::default();
        let mut w = Waves::default();
        search_frame(&mut w, &ctx, Some(&s), vec![], true);
        search_frame(
            &mut w,
            &ctx,
            Some(&s),
            vec![egui::Event::Text("led".into())],
            false,
        );
        let found = suggestions(w.names_of(&s), "led");
        assert_eq!(found[0].label, "led[7:0]  8 bits");
        let out = search_frame(&mut w, &ctx, Some(&s), vec![], false);
        text_at(&out, "led[7:0]  8 bits");
        search_frame(&mut w, &ctx, Some(&s), search_key(egui::Key::Enter), false);
        assert_eq!(
            w.nets.iter().map(|n| n.name.clone()).collect::<Vec<_>>(),
            (0..8)
                .rev()
                .map(|i| format!("led[{i}]"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            w.lines(),
            [Line::Bus {
                name: "led".into(),
                bits: (0..8).collect()
            }]
        );
        assert!(w.query.is_empty());
        assert!(!w.search_open);
        w.command();
        let id = w.id;
        search_frame(
            &mut w,
            &ctx,
            Some(&s),
            vec![egui::Event::Text("led".into())],
            false,
        );
        search_frame(
            &mut w,
            &ctx,
            Some(&s),
            search_key(egui::Key::ArrowDown),
            false,
        );
        assert_eq!(w.selected, 1);
        search_frame(
            &mut w,
            &ctx,
            Some(&s),
            search_key(egui::Key::ArrowUp),
            false,
        );
        assert_eq!(w.selected, 0);
        search_frame(&mut w, &ctx, Some(&s), search_key(egui::Key::Enter), false);
        assert_eq!(w.nets.len(), 8);
        assert_eq!(w.id, id);
        assert!(w.command().is_none());
        assert_eq!(w.scroll_to, Some(w.nets[0].net));
        // Script lookup remains exact, with the same failure note and record.
        w.recording = true;
        assert_eq!(w.add_name_from(&s, "ld", "script"), 0);
        assert_eq!(w.note.as_deref(), Some("No net named ld"));
        assert!(w.notes.contains(&"add\tscript\tld\tno net named".into()));
        search_frame(
            &mut w,
            &ctx,
            Some(&s),
            vec![egui::Event::Text("zzz".into())],
            false,
        );
        let out = search_frame(&mut w, &ctx, Some(&s), vec![], false);
        text_at(&out, "No signal matches “zzz”");
        // A bit result also uses Enter, independently of its whole bus.
        w.query = "led[0]".into();
        search_frame(&mut w, &ctx, Some(&s), vec![], false);
        search_frame(&mut w, &ctx, Some(&s), search_key(egui::Key::Enter), false);
        assert_eq!(w.nets.len(), 8);
        assert_eq!(w.scroll_to, Some(w.nets[7].net));
        let mut bit = Waves::default();
        bit.add_suggestion(&s, &found[1]);
        assert_eq!(bit.nets.len(), 1);
        assert_eq!(bit.nets[0].name, "led[0]");
        drop(runner);
    }

    #[test]
    fn source_watch_in_the_tour_draws_each_counter_bit() {
        let mut w = Waves::default();
        let names: Vec<_> = [2, 1, 4, 3, 0, 6, 5, 7]
            .map(|i| format!("count[{i}]"))
            .into();
        for watched in watched(&names.iter().map(String::as_str).collect::<Vec<_>>()) {
            // This is the source-watch action used by App::watch_probed at step 6.
            w.add_watched(watched, "source");
        }
        let lines = w.lines();
        assert_eq!(lines.len(), 8);
        let drawn: Vec<_> = lines
            .iter()
            .map(|line| match line {
                Line::Bit(i) => w.nets[*i].name.as_str(),
                Line::Bus { .. } => panic!("the tour must show the binary counter's bits"),
            })
            .collect();
        assert_eq!(
            drawn,
            (0..8)
                .rev()
                .map(|i| format!("count[{i}]"))
                .collect::<Vec<_>>()
        );
        assert!(
            w.rows.is_empty(),
            "opening the bus does not supply Waves rows"
        );
        w.take(vec![run(&w, 1, &[0; 8])], 1, 0);
        assert!(!w.rows.is_empty());
    }

    /// Rows are kept by edge: sent again (going back) over themselves, the
    /// oldest let go past the kept cycles, those ahead let go when the run
    /// has none recorded there any more, rows of an earlier list ignored.
    #[test]
    fn rows_are_kept_by_edge() {
        let mut w = waves(&["a", "b"]);
        w.keep_cycles = 4;
        let id = w.id;
        w.take(vec![run(&w, 1, &[0, 1, 1, 1, 0, 0])], 3, 0);
        assert_eq!((w.value(1, 0), w.value(2, 0), w.value(3, 1)), (0, 1, 0));
        assert_eq!(w.value(4, 0), NONE);
        // Back to edge 2 and on: the same again, and on to edge 5.
        w.take(vec![run(&w, 2, &[1, 1, 0, 0, 1, 0, 0, 1])], 5, 0);
        assert_eq!((w.value(2, 0), w.value(4, 0), w.value(5, 1)), (1, 1, 1));
        // Past four cycles (eight edges) the oldest go.
        w.take(vec![run(&w, 6, &[0; 2 * 6])], 11, 0);
        assert_eq!(w.value(1, 0), NONE);
        assert_eq!(w.value(3, 0), 0);
        assert_eq!(w.value(11, 0), 0);
        // Gone back to edge 6, the run recorded to 11: kept; a new input
        // there (none ahead): the rows after 6 go.
        w.take(Vec::new(), 6, 5);
        assert_eq!(w.value(11, 0), 0);
        w.take(Vec::new(), 6, 0);
        assert_eq!((w.value(6, 0), w.value(7, 0)), (0, NONE));
        // Rows of an earlier list are not taken.
        let mut stale = run(&w, 7, &[1, 1]);
        stale.id = id - 1;
        w.take(vec![stale], 7, 0);
        assert_eq!(w.value(7, 0), NONE);
    }

    /// Delivery precedes publishing, so the frame may still see edge 10.
    #[test]
    fn rows_ahead_of_the_publish_are_kept() {
        let mut w = waves(&["a"]);
        w.take(vec![run(&w, 11, &[0, 1, 0, 1])], 10, 0);
        assert_eq!((w.value(11, 0), w.value(14, 0)), (0, 1));
        w.take(Vec::new(), 14, 0);
        assert_eq!((w.value(11, 0), w.value(14, 0)), (0, 1));
    }

    /// Only the old future is invalidated: new rows can arrive before the
    /// publish of the input's branch too.
    #[test]
    fn a_new_input_after_going_back_drops_only_the_old_future() {
        let mut w = waves(&["a"]);
        w.take(vec![run(&w, 1, &[0; 12])], 12, 0);
        w.take(Vec::new(), 6, 6);
        assert_eq!(w.value(12, 0), 0);
        w.take(vec![run(&w, 7, &[1, 1])], 6, 0);
        assert_eq!((w.value(6, 0), w.value(7, 0), w.value(8, 0)), (0, 1, 1));
        assert_eq!(w.value(9, 0), NONE);
        w.take(Vec::new(), 8, 0);
        assert_eq!(w.value(8, 0), 1);
    }

    /// A new list keeps the rows of the nets still watched, in their new
    /// places, and is sent to the run once.
    #[test]
    fn a_new_list_keeps_the_rows_of_nets_still_watched() {
        let mut w = waves(&["a", "b", "c"]);
        w.take(vec![run(&w, 1, &[0, 1, 2, 1, 0, 3])], 2, 0);
        let mut nets = w.nets.clone();
        nets.remove(0);
        nets.push(Watched {
            name: "d".into(),
            net: 9,
            wire: 9,
        });
        assert!(w.command().is_some());
        w.set(nets);
        assert_eq!((w.value(1, 0), w.value(1, 1), w.value(1, 2)), (1, 2, NONE));
        assert_eq!((w.value(2, 0), w.value(2, 1)), (0, 3));
        match w.command() {
            Some(Command::Watch { wires, .. }) => assert_eq!(wires, [1, 2, 9]),
            other => panic!("{other:?}"),
        }
        assert!(w.command().is_none());
    }

    /// A bus's bits are one line (most significant first) unless opened; its
    /// value a number when every bit is 0 or 1, else the worst bit.
    #[test]
    fn buses_are_one_line_with_a_value() {
        let mut w = waves(&["cnt[0]", "cnt[2]", "cnt[1]", "led", "x[3]"]);
        let lines = w.lines();
        assert_eq!(
            lines,
            [
                Line::Bus {
                    name: "cnt".into(),
                    bits: vec![1, 2, 0]
                },
                Line::Bit(3),
                Line::Bit(4),
            ]
        );
        w.take(vec![run(&w, 1, &[1, 1, 0, 1, 0, 1, 2, 0, 1, 0])], 2, 0);
        assert_eq!(w.line_value(&lines[0], 1), Ok(0b101));
        assert_eq!(w.line_value(&lines[0], 2), Err(2));
        assert_eq!(w.line_value(&lines[1], 1), Ok(1));
        w.open_buses.insert("cnt".into());
        assert_eq!(w.lines().len(), 5);
        assert_eq!(bus_of("a.b[12]"), Some(("a.b", 12)));
        assert_eq!(bus_of("led"), None);
        assert_eq!(bus_of("m[x]"), None);
    }

    #[test]
    fn opening_a_bus_only_reorders_its_display_lines() {
        let mut w = waves(&[
            "before", "cnt[1]", "cnt[3]", "cnt[0]", "cnt[2]", "after", "x[0]", "x[1]", "last",
        ]);
        w.take(vec![run(&w, 1, &[0, 1, 1, 0, 0, 1, 1, 0, 1])], 1, 0);
        let nets = w.nets.clone();
        let rows = w.rows.clone();
        let id = w.id;
        let collapsed = vec![
            Line::Bit(0),
            Line::Bus {
                name: "cnt".into(),
                bits: vec![2, 4, 1, 3],
            },
            Line::Bit(5),
            Line::Bus {
                name: "x".into(),
                bits: vec![7, 6],
            },
            Line::Bit(8),
        ];
        assert_eq!(w.lines(), collapsed);
        assert_eq!(w.line_value(&collapsed[1], 1), Ok(0b1010));
        w.open_buses.insert("cnt".into());
        let lines = w.lines();
        assert_eq!(
            lines,
            [
                Line::Bit(0),
                Line::Bit(2),
                Line::Bit(4),
                Line::Bit(1),
                Line::Bit(3),
                Line::Bit(5),
                collapsed[3].clone(),
                Line::Bit(8),
            ]
        );
        assert_eq!(
            lines[1..5]
                .iter()
                .map(|line| w.line_value(line, 1).unwrap())
                .collect::<Vec<_>>(),
            [1, 0, 1, 0]
        );
        assert_eq!(w.nets, nets);
        assert_eq!(w.rows, rows);
        assert_eq!(w.id, id);
        match w.command() {
            Some(Command::Watch { wires, .. }) => assert_eq!(wires, (0..9).collect::<Vec<_>>()),
            other => panic!("{other:?}"),
        }
        w.open_buses.clear();
        assert_eq!(w.lines(), collapsed);
        assert!(w.command().is_none());
    }

    fn cross_probe_waves() -> Waves {
        use ecp5_emu::sources::NetSources;
        let span = |line, column| Span {
            file: "rtl/top.v".into(),
            line,
            column: Some(column),
            end_line: line,
            end_column: Some(column + 2),
        };
        let mut index = Index::default();
        index.locations = 6;
        for net in 0..3 {
            index.nets.insert(
                net,
                NetSources {
                    names: vec![format!("count[{net}]")],
                    driven: vec![span(if net == 2 { 30 } else { 20 }, net + 1)],
                    declared: vec![span(4, net + 1)],
                },
            );
        }
        let mut w = waves(&["count[0]", "count[1]", "count[2]", "led"]);
        w.sources = Some(Arc::new(index));
        w.recording = true;
        w
    }

    #[test]
    fn row_cross_probes_resolve_watched_bits_and_merge_source_choices() {
        let mut w = cross_probe_waves();
        assert_eq!(w.row_nets("count"), [0, 1, 2]);
        assert_eq!(w.row_nets("count[1]"), [1]);
        assert_eq!(w.row_nets("led"), [3]);
        assert!(w.row_nets("missing").is_empty());
        assert!(w.chip_action("missing").is_none());
        let choices = w.row_sources("count").unwrap();
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.kind, c.span.line))
                .collect::<Vec<_>>(),
            [
                (SourceKind::Driven, 20),
                (SourceKind::Driven, 30),
                (SourceKind::Declared, 4)
            ]
        );
        for net in 0..3 {
            assert_eq!(
                w.row_sources(&format!("count[{net}]")).unwrap(),
                w.sources.as_ref().unwrap().net_source_choices(net)
            );
        }
        let chip = w.chip_action("count");
        let source = w.source_action("count", None);
        w.open_buses.insert("count".into());
        assert_eq!(w.row_nets("count"), [0, 1, 2]);
        assert_eq!(w.chip_action("count"), chip);
        assert_eq!(w.source_action("count", None), source);
        assert_eq!(
            w.row_sources("led"),
            Err("no source line for this net (synthesis left it only a library location)")
        );
        assert!(w.source_action("led", None).is_none());
        w.sources = None;
        assert_eq!(
            w.row_sources("count"),
            Err("design has no source index yet")
        );
        assert!(w.source_action("count", None).is_none());
        assert!(w.chip_action("count").is_some());
        w.reset();
        assert!(w.sources.is_none());
        assert!(w.row_action.is_none());
    }

    fn probe_frame(
        w: &mut Waves,
        ctx: &egui::Context,
        size: Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::bottom("waves")
                    .exact_size(240.0)
                    .show(ui, |ui| {
                        w.show(ui, None, 0, None, None);
                    });
            },
        )
    }

    fn pointer_button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn text_at(out: &egui::FullOutput, text: &str) -> Pos2 {
        out.shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing text: {text}"))
    }

    #[test]
    fn row_menus_cross_probe_bits_collapsed_and_opened_buses_in_both_themes() {
        for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
            for size in [Vec2::new(1000.0, 500.0), Vec2::new(1500.0, 900.0)] {
                // A scalar, a collapsed bus, and a bit in an opened bus.
                for (opened, row, name, source_label) in [
                    (false, 1, "led", "Show in source"),
                    (false, 0, "count", "Show in source (rtl/top.v:20)"),
                    (true, 0, "count[2]", "Show in source (rtl/top.v:30)"),
                ] {
                    for source in [false, true] {
                        let ctx = egui::Context::default();
                        theme::install(&ctx, theme);
                        let mut w = cross_probe_waves();
                        if opened {
                            w.open_buses.insert("count".into());
                        }
                        for _ in 0..2 {
                            probe_frame(&mut w, &ctx, size, vec![]);
                        }
                        let rect = w.drawn.as_ref().unwrap().rect;
                        let point = Pos2::new(
                            rect.left() + 70.0,
                            rect.top() + AXIS + (row as f32 + 0.5) * ROW,
                        );
                        for pressed in [true, false] {
                            probe_frame(
                                &mut w,
                                &ctx,
                                size,
                                pointer_button(point, egui::PointerButton::Secondary, pressed),
                            );
                        }
                        probe_frame(&mut w, &ctx, size, vec![]);
                        let out = probe_frame(&mut w, &ctx, size, vec![]);
                        text_at(&out, "Show on the chip");
                        text_at(&out, source_label);
                        assert_eq!(w.drawn.as_ref().unwrap().rect, rect, "menu moved the panel");
                        let target = text_at(
                            &out,
                            if source {
                                source_label
                            } else {
                                "Show on the chip"
                            },
                        );
                        for pressed in [true, false] {
                            probe_frame(
                                &mut w,
                                &ctx,
                                size,
                                pointer_button(target, egui::PointerButton::Primary, pressed),
                            );
                        }
                        let expected = if source {
                            w.source_action(name, None)
                        } else {
                            w.chip_action(name)
                        };
                        assert_eq!(w.row_action, expected, "{name}, source={source}");
                        if name != "led" {
                            text_at(&out, "Source locations");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn source_menu_disabled_tooltips_and_declared_submenu_choice() {
        for project in [false, true] {
            for state in 0..3 {
                let ctx = egui::Context::default();
                let mut w = cross_probe_waves();
                w.project_sources = project;
                match state {
                    0 => w.sources = None,
                    1 => w.sources = Some(Arc::new(Index::default())),
                    _ => {}
                }
                let reason = match state {
                    0 => "design has no source index yet",
                    1 if project => "no project source matched the build's src attributes",
                    1 => "no source files found for this design (looked beside its top.config)",
                    _ => "no source line for this net (synthesis left it only a library location)",
                };
                assert_eq!(w.row_sources("led"), Err(reason));
                let size = Vec2::new(1000.0, 500.0);
                let time = std::cell::Cell::new(0.0);
                let frame = |w: &mut Waves, events| {
                    time.set(time.get() + 1.0);
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            time: Some(time.get()),
                            ..Default::default()
                        },
                        |ui| {
                            w.row_menu(ui, "led");
                        },
                    )
                };
                let out = frame(&mut w, vec![]);
                let point = text_at(&out, "Show in source");
                // The explanation is visible without waiting for a tooltip.
                text_at(&out, reason);
                for pressed in [true, false] {
                    frame(
                        &mut w,
                        pointer_button(point, egui::PointerButton::Primary, pressed),
                    );
                }
                assert!(
                    w.row_action.is_none(),
                    "disabled source item cannot dispatch"
                );
                frame(&mut w, vec![egui::Event::PointerMoved(point)]);
                frame(&mut w, vec![]);
                let out = frame(&mut w, vec![]);
                text_at(&out, reason);
            }
        }
        let ctx = egui::Context::default();
        let mut w = cross_probe_waves();
        let size = Vec2::new(1000.0, 500.0);
        for _ in 0..2 {
            probe_frame(&mut w, &ctx, size, vec![]);
        }
        let rect = w.drawn.as_ref().unwrap().rect;
        let point = Pos2::new(rect.left() + 70.0, rect.top() + AXIS + ROW / 2.0);
        for pressed in [true, false] {
            probe_frame(
                &mut w,
                &ctx,
                size,
                pointer_button(point, egui::PointerButton::Secondary, pressed),
            );
        }
        probe_frame(&mut w, &ctx, size, vec![]);
        let out = probe_frame(&mut w, &ctx, size, vec![]);
        let submenu = text_at(&out, "Source locations");
        for pressed in [true, false] {
            probe_frame(
                &mut w,
                &ctx,
                size,
                pointer_button(submenu, egui::PointerButton::Primary, pressed),
            );
        }
        probe_frame(&mut w, &ctx, size, vec![egui::Event::PointerMoved(submenu)]);
        let out = probe_frame(&mut w, &ctx, size, vec![]);
        text_at(&out, "assigned in rtl/top.v:20");
        text_at(&out, "assigned in rtl/top.v:30");
        let declared = text_at(&out, "declared in rtl/top.v:4");
        for pressed in [true, false] {
            probe_frame(
                &mut w,
                &ctx,
                size,
                pointer_button(declared, egui::PointerButton::Primary, pressed),
            );
        }
        assert_eq!(
            w.row_action,
            w.source_action(
                "count",
                Some(w.row_sources("count").unwrap()[2].span.clone())
            )
        );
    }

    /// Names, values at the pointer, the record and removal use the net's
    /// column, even when its displayed row has moved.
    #[test]
    fn opened_bus_rows_keep_their_names_hover_values_and_clicks() {
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let draw = |w: &mut Waves, events| {
            let lines = w.lines();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(800.0, AXIS + ROW * lines.len() as f32),
                        Sense::click_and_drag(),
                    );
                    w.draw(ui, rect, &response, &lines, 2);
                },
            )
        };
        let names = ["before", "cnt[3]", "cnt[2]", "cnt[1]", "cnt[0]", "after"];
        let values = ["0", "X", "?", "1", "Z", "1"];
        for (row, net) in [0, 2, 4, 1, 3, 5].into_iter().enumerate() {
            let mut w = waves(&["before", "cnt[1]", "cnt[3]", "cnt[0]", "cnt[2]", "after"]);
            w.recording = true;
            w.set_open(true);
            w.open_buses.insert("cnt".into());
            w.scale = 40.0;
            w.end = Some(2);
            w.take(
                vec![run(&w, 1, &[0, 1, 2, 3, NOT_SAMPLED, 1, 1, 0, 0, 0, 1, 0])],
                2,
                0,
            );
            let nets = w.nets.clone();
            let _ = draw(&mut w, Vec::new());
            let rect = w.drawn.as_ref().unwrap().rect;
            let y = rect.top() + AXIS + (row as f32 + 0.5) * ROW;
            let pointer = Pos2::new(rect.right() - 40.0, y);
            let out = draw(&mut w, vec![egui::Event::PointerMoved(pointer)]);
            assert_eq!(w.drawn.as_ref().unwrap().names, names);
            for (r, (&name, &value)) in names.iter().zip(&values).enumerate() {
                let top = rect.top() + AXIS + r as f32 * ROW;
                let text: Vec<_> = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) if (top..top + ROW).contains(&t.pos.y) => {
                            Some(t.galley.text())
                        }
                        _ => None,
                    })
                    .collect();
                assert!(text.contains(&name), "row {r}: {text:?}");
                assert!(text.contains(&value), "row {r}: {text:?}");
            }
            let notes = w.notes(0.0, None, 2).join("\n");
            assert!(notes.contains(&format!("drawn\tlines 6 {names:?}")));
            let close = Pos2::new(rect.left() + 8.0, y);
            for pressed in [true, false] {
                let _ = draw(
                    &mut w,
                    vec![
                        egui::Event::PointerMoved(close),
                        egui::Event::PointerButton {
                            pos: close,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                w.nets,
                nets.into_iter()
                    .filter(|n| n.net != net)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// The record distinguishes rejected rows, freshness and a program
    /// switch, and emits totals only once a second.
    #[test]
    fn notes_explain_rows_and_watch_changes() {
        let mut w = waves(&["a", "b"]);
        w.recording = true;
        w.set_open(true);
        assert!(w.command().is_some());
        let mut earlier = run(&w, 1, &[0, 0]);
        earlier.id -= 1;
        let mut wrong_width = run(&w, 1, &[0]);
        wrong_width.width = 1;
        w.take(
            vec![earlier, wrong_width, run(&w, 1, &[NOT_SAMPLED, 1])],
            0,
            0,
        );
        let notes = w.notes(0.0, Some("Building"), 0).join("\n");
        assert!(notes.contains("opened"));
        assert!(notes.contains("watch\tid 1\twires 2"));
        assert!(notes.contains("NOT_SAMPLED; needs a program switch"));
        assert!(notes.contains("sampled; running program keeps it"));
        assert!(notes.contains("kept 1..2"));
        assert!(notes.contains("dropped earlier list 1\tdifferent width 1"));
        assert!(notes.contains("NOT_SAMPLED {0: 1, 1: 0}"));
        assert!(notes.contains("status\tedge 0\tBuilding"));
        assert!(w.notes(0.5, Some("Building"), 0).is_empty());
        w.take(vec![run(&w, 2, &[0, 1])], 2, 0);
        let notes = w.notes(1.0, Some("Running"), 2).join("\n");
        assert!(notes.contains("freshness\tid 1\tnet 0\tname a\tedge 2\tsampled"));
        assert!(notes.contains("status\tedge 2\tRunning"));
        w.set_open(false);
        let notes = w.notes(1.1, Some("failed"), 2).join("\n");
        assert!(notes.contains("closed"));
        assert!(notes.contains("status\tedge 2\tfailed"));
    }

    #[test]
    fn following_draws_rows_newer_than_the_publish() {
        let mut w = waves(&["a"]);
        w.recording = true;
        w.set_open(true);
        w.take(vec![run(&w, 1000, &[1; 20])], 100, 0);
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 300.0))),
            ..Default::default()
        };
        let _ = ctx.run_ui(input(), |ui| {
            egui::Panel::bottom("waves")
                .exact_size(240.0)
                .show(ui, |ui| {
                    w.show(ui, None, 100, None, None);
                });
        });
        let d = w.drawn.as_ref().unwrap();
        assert!(
            d.first <= d.last,
            "empty drawn span {}..={}",
            d.first,
            d.last
        );
        assert_eq!((d.first, d.last, d.end, d.present), (1000, 1019, 1019, 100));
        let pointer = Pos2::new(d.rect.right() - 20.0, d.rect.top() + AXIS + ROW / 2.0);
        let note = w.notes(0.0, None, 100).join("\n");
        assert!(note.contains("edges 1000..=1019\tpresent 100\tend 1019\tfollowing true"));
        let mut goto = None;
        for pressed in [true, false] {
            let mut raw = input();
            raw.events = vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
            let _ = ctx.run_ui(raw, |ui| {
                egui::Panel::bottom("waves")
                    .exact_size(240.0)
                    .show(ui, |ui| {
                        goto = goto.or(w.show(ui, None, 100, None, None));
                    });
            });
        }
        assert_eq!(goto, Some(1009 / 2));
    }

    #[test]
    fn only_the_recorded_future_is_dimmed() {
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let mut w = waves(&["a"]);
        w.take(vec![run(&w, 1, &[1; 6])], 2, 0);
        let draw = |w: &Waves| {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                w.trace(
                    ui.painter(),
                    &Line::Bit(0),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(60.0, 12.0)),
                    (1, 6),
                    (&|edge| (edge as f32 - 1.0) * 10.0, 2),
                    ui.visuals(),
                );
            });
            out.shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::LineSegment { points, stroke } => Some((points, stroke.color)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let full = ctx.style_of(egui::Theme::Dark).visuals.text_color();
        assert_eq!(
            draw(&w),
            vec![([Pos2::new(0.0, 0.0), Pos2::new(60.0, 0.0)], full)]
        );
        w.take(Vec::new(), 2, 3);
        let dimmed = draw(&w);
        assert_eq!(
            dimmed,
            vec![
                ([Pos2::new(0.0, 0.0), Pos2::new(20.0, 0.0)], full),
                (
                    [Pos2::new(20.0, 0.0), Pos2::new(50.0, 0.0)],
                    full.gamma_multiply(0.45)
                ),
                ([Pos2::new(50.0, 0.0), Pos2::new(60.0, 0.0)], full),
            ]
        );
    }

    #[test]
    fn the_empty_panel_explains_adding_at_a_fixed_height() {
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let mut w = Waves {
            recording: true,
            ..Waves::default()
        };
        let mut text = String::new();
        let mut height = None;
        for (focus, pointer) in [
            (false, Pos2::new(800.0, 180.0)),
            (true, Pos2::new(40.0, 120.0)),
            (false, Pos2::new(1490.0, 290.0)),
        ] {
            let mut frame_height = 0.0;
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 300.0))),
                    events: vec![egui::Event::PointerMoved(pointer)],
                    ..Default::default()
                },
                |ui| {
                    let field = ui.text_edit_singleline(&mut text);
                    if focus {
                        field.request_focus();
                    } else {
                        field.surrender_focus();
                    }
                    egui::Panel::bottom("waves")
                        .exact_size(240.0)
                        .show(ui, |ui| {
                            w.show(ui, None, 0, None, None);
                            frame_height = ui.min_rect().height();
                        });
                },
            );
            let strings: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text()),
                    _ => None,
                })
                .collect();
            assert!(strings.contains(&EMPTY));
            assert_eq!(strings.contains(&KEYBOARD), focus);
            assert_eq!(w.drawn.as_ref().unwrap().rect.height(), AXIS + ROW);
            assert_eq!(*height.get_or_insert(frame_height), frame_height);
        }
    }

    /// The panel keeps its height wherever the pointer is (a height that
    /// followed what is hovered once made the bottom panel resize itself in
    /// a loop), and draws any values at any zoom.
    #[test]
    fn the_panel_keeps_its_height_and_draws_anything() {
        let mut w = waves(&["cnt[0]", "cnt[1]", "led", "x", "y"]);
        let values = [0, 1, 2, 3, NOT_SAMPLED, NONE];
        let rows: Vec<u8> = (0..600u32)
            .flat_map(|e| (0..5u32).map(move |k| values[((e / (k + 1)) % 6) as usize]))
            .collect();
        w.take(vec![run(&w, 1, &rows)], 600, 0);
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let mut heights = Vec::new();
        for (scale, pointer) in [
            (2.0, None),
            (0.02, Some(Pos2::new(800.0, 120.0))),
            (40.0, Some(Pos2::new(60.0, 100.0))),
            (2.0, Some(Pos2::new(1400.0, 150.0))),
        ] {
            w.scale = scale;
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 300.0))),
                ..Default::default()
            };
            if let Some(p) = pointer {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let _ = ctx.run_ui(input, |ui| {
                egui::Panel::bottom("waves")
                    .exact_size(240.0)
                    .show(ui, |ui| {
                        w.show(ui, None, 600, None, None);
                        heights.push(ui.min_rect().height());
                    });
            });
        }
        assert!(heights.windows(2).all(|h| h[0] == h[1]), "{heights:?}");
    }
    /// The console demo's live rows, taken as frames take them, without a
    /// native window or GPU. Run explicitly with the reference design built.
    #[test]
    #[ignore]
    fn picorv32_waves_at_each_rate() {
        use ecp5_emu::live::{self, Design, Shared};
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/reference/picorv32_console");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let commands = live::spawn(
            Design {
                config: root.join("top.config"),
                names: Some(root.join("top_routed.json")),
                run: false,
            },
            Arc::clone(&shared),
            || {},
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let s = loop {
            let sh = shared.lock().unwrap();
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if let Some(s) = &sh.ready {
                break Arc::clone(s);
            }
            assert!(Instant::now() < deadline, "loading: {}", sh.stage);
            drop(sh);
            std::thread::sleep(Duration::from_millis(16));
        };
        // Keep memory bounded even when full speed outruns a headless frame.
        // A full handover queue can evict older edges (src/live.rs, deliver).
        let mut w = Waves {
            recording: true,
            keep_cycles: 1 << 20,
            ..Waves::default()
        };
        w.set_open(true);
        w.add_shown(&s, None, "script");
        assert_eq!(
            w.note.as_deref(),
            Some("no net kept on the map (click a wire first)")
        );
        assert!(w
            .notes
            .iter()
            .any(|n| n.contains("no net kept on the map (click a wire first)")));
        let names = w.names_of(&s).clone();
        // Use names present on this routed design. Synthesis can remove
        // source-level nets such as `request` by folding them into a LUT.
        let mut seen = HashSet::new();
        let by_name: Vec<_> = names
            .iter()
            .filter(|(name, _)| name.contains("_DI") && bus_of(name).is_none())
            .filter(|(_, net)| seen.insert(**net))
            .take(6)
            .map(|(name, _)| name.clone())
            .collect();
        assert_eq!(by_name.len(), 6);
        for name in by_name {
            assert_eq!(w.add_name(&s, &name), 1, "{name}");
        }
        let shown = *names.get("clk_25mhz").expect("clock HDL name");
        assert!(w.add(&s, shown, None));
        w.add_menu(&s, shown);
        assert!(w
            .notes
            .iter()
            .any(|n| n.contains("add\tmenu\tnet") && n.ends_with("already watched")));
        let mut buses: BTreeMap<String, usize> = BTreeMap::new();
        for name in names.keys() {
            if let Some((base, _)) = bus_of(name) {
                *buses.entry(base.to_string()).or_default() += 1;
            }
        }
        let bus = buses
            .iter()
            .filter(|(_, n)| (2..=8).contains(*n))
            .min_by_key(|(name, _)| (name.len(), *name))
            .expect("an HDL bus")
            .0;
        assert!(w.add_name(&s, bus) > 1, "bus {bus}");
        assert!(w
            .lines()
            .iter()
            .any(|line| matches!(line, Line::Bus { .. })));
        let watch = w.command().unwrap();
        let Command::Watch { keep, .. } = watch else {
            panic!("expected a watch command");
        };
        commands.send(watch).unwrap();
        let ctx = egui::Context::default();
        theme::install(&ctx, egui::ThemePreference::Dark);
        let start = Instant::now();
        let mut stale = HashSet::new();
        let mut recovered = HashSet::new();
        let mut statuses = Vec::new();
        let mut delivered_before_publish = 0;
        let mut evictions = 0;
        let mut drawings = 0;
        // Each frame checks every received edge is still in the panel. The
        // ordinary retention limit can discard older frames at full speed.
        let mut frame = |w: &mut Waves, span: &mut Option<(u64, u64)>| {
            let (runs, edge, ahead, running, status, lit) = {
                let mut sh = shared.lock().unwrap();
                assert!(sh.error.is_none(), "{:?}", sh.error);
                (
                    std::mem::take(&mut sh.waves),
                    sh.edges,
                    sh.ahead,
                    sh.running,
                    sh.watch_status.clone(),
                    sh.lit.clone(),
                )
            };
            if statuses.last() != Some(&status) {
                println!("watch status at {edge}: {status:?}");
                statuses.push(status.clone());
            }
            assert!(
                !status.as_deref().is_some_and(|s| s.contains("failed")),
                "{status:?}"
            );
            let mut received = Vec::new();
            let taken_edges: u64 = runs.iter().map(live::WaveRun::edges).sum();
            for r in &runs {
                assert_eq!((r.id, r.width), (w.id, w.nets.len()));
                if r.edges() == 0 {
                    continue;
                }
                let last = r.first + r.edges() - 1;
                if last > edge {
                    delivered_before_publish += 1;
                }
                if let Some((_, hi)) = span {
                    if r.first > *hi + 1 {
                        assert_eq!(
                            taken_edges,
                            2 * keep,
                            "delivery gap: {hi} to {}; frame took {taken_edges} edges",
                            r.first
                        );
                        evictions += 1;
                    }
                    *hi = (*hi).max(last);
                } else {
                    *span = Some((r.first, last));
                }
                for row in r.rows.chunks_exact(r.width) {
                    for (k, &v) in row.iter().enumerate() {
                        if v == NOT_SAMPLED {
                            stale.insert(k);
                        } else if stale.contains(&k) {
                            recovered.insert(k);
                        }
                    }
                }
                received.push((r.first, last));
            }
            w.take(runs, edge, if ahead > 0 { ahead * 2 + 1 } else { 0 });
            for (first, last) in received {
                for e in first.max(w.lo)..=last {
                    for k in 0..w.nets.len() {
                        assert_ne!(
                            w.value(e, k),
                            NONE,
                            "frame lost edge {e}, published {edge}, kept {}..{}",
                            w.lo,
                            w.hi()
                        );
                    }
                }
            }
            if drawings % 8 == 0 {
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(1500.0, 500.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::bottom("waves")
                            .exact_size(320.0)
                            .show(ui, |ui| {
                                w.show(ui, Some(&s), edge, Some(shown), status.as_deref());
                            });
                    },
                );
                let d = w.drawn.as_ref().expect("panel drew lines");
                assert_eq!(d.names.len(), w.lines().len());
                if running && !w.rows.is_empty() {
                    assert!(
                        d.first <= d.last,
                        "empty span at speed: {}..={} present {edge}",
                        d.first,
                        d.last
                    );
                    assert_eq!(d.end, edge.max(w.hi() - 1));
                }
                assert!(out.shapes.len() > d.names.len());
                if !w.rows.is_empty() && d.first <= d.last {
                    for row in 0..d.names.len() {
                        let top = d.rect.top() + AXIS + row as f32 * ROW;
                        let band = Rect::from_min_max(
                            Pos2::new(d.rect.left() + NAMES, top + 3.0),
                            Pos2::new(d.rect.right(), top + ROW - 3.0),
                        )
                        .expand(2.0);
                        assert!(
                            out.shapes.iter().any(|shape| {
                                matches!(
                                    shape.shape,
                                    egui::Shape::LineSegment { .. }
                                        | egui::Shape::Path(_)
                                        | egui::Shape::Rect(_)
                                ) && band.contains_rect(
                                    shape
                                        .shape
                                        .visual_bounding_rect()
                                        .intersect(shape.clip_rect),
                                )
                            }),
                            "no waveform drawn for {}",
                            d.names[row]
                        );
                    }
                }
            }
            drawings += 1;
            for line in w.notes(start.elapsed().as_secs_f64(), status.as_deref(), edge) {
                println!("# waves\t{:.3}\t{line}", start.elapsed().as_secs_f64());
            }
            (edge, running, lit)
        };
        for rate in [Some(live::REAL_TIME), Some(4.0), None] {
            commands.send(Command::Rate(rate)).unwrap();
            commands.send(Command::Run).unwrap();
            let until =
                Instant::now() + Duration::from_millis(if rate == Some(4.0) { 2200 } else { 1200 });
            let mut span = None;
            let mut saw_running = false;
            while Instant::now() < until || !saw_running || span.is_none() {
                assert!(
                    start.elapsed() < Duration::from_secs(180),
                    "run did not advance"
                );
                saw_running |= frame(&mut w, &mut span).1;
                std::thread::sleep(Duration::from_millis(16));
            }
            commands.send(Command::Pause).unwrap();
            let (edge, lit) = loop {
                let (edge, running, lit) = frame(&mut w, &mut span);
                if !running {
                    break (edge, lit);
                }
                assert!(
                    start.elapsed() < Duration::from_secs(180),
                    "pause timed out"
                );
                std::thread::sleep(Duration::from_millis(16));
            };
            let (first, last) = span.expect("rows at this rate");
            assert!(last > first, "rate {rate:?} has only one edge");
            for e in first.max(w.lo)..=last {
                for k in 0..w.nets.len() {
                    assert_ne!(w.value(e, k), NONE, "gap at {e}");
                }
            }
            let mut sampled = 0;
            for (k, net) in w.nets.iter().enumerate() {
                let value = w.value(edge, k);
                assert_ne!(value, NONE, "no pause row for {} at {edge}", net.name);
                if value != NOT_SAMPLED {
                    assert_eq!(value, lit[net.wire as usize], "{} at pause", net.name);
                    sampled += 1;
                }
            }
            assert!(sampled > 0);
            println!(
                "rate {rate:?}: contiguous {first}..={last}; {sampled} pause values match lit"
            );
        }
        // Keep taking frames while the background compiler finishes.
        commands.send(Command::Run).unwrap();
        let mut span = None;
        loop {
            frame(&mut w, &mut span);
            if statuses_running(&shared) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(180),
                "watch switch timed out"
            );
            std::thread::sleep(Duration::from_millis(16));
        }
        commands.send(Command::Step(2)).unwrap();
        loop {
            let (_, running, _) = frame(&mut w, &mut span);
            if !running {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(180));
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(statuses.iter().flatten().any(|s| s.starts_with("Building")));
        assert!(statuses.iter().flatten().any(|s| s.starts_with("Running")));
        assert!(
            !stale.is_empty(),
            "exercise a net the default program folds"
        );
        assert!(
            stale.is_subset(&recovered),
            "not sampled after switch: {:?}",
            stale.difference(&recovered).collect::<Vec<_>>()
        );
        assert!(
            delivered_before_publish > 0,
            "exercise delivery before publish"
        );
        println!("{drawings} headless frames; {delivered_before_publish} deliveries before publish; {evictions} queue evictions; {} folded nets became sampled", stale.len());

        fn statuses_running(shared: &Mutex<Shared>) -> bool {
            shared
                .lock()
                .unwrap()
                .watch_status
                .as_deref()
                .is_some_and(|s| s.starts_with("Running"))
        }
    }
}
