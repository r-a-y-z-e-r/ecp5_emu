//! PIO BASE_TYPE shares CRAM bits with DRIVE, PULLMODE, OPENDRAIN,
//! HYSTERESIS and CLAMP, and with the other PIO of its pair. Independent
//! matching can then read LVCMOS33 with DRIVE=4 as SSTL, or an output with the
//! default PULLMODE as NONE.
//!
//! The writer contract is that of nextpnr and libtrellis. nextpnr writes, per
//! IO cell, BASE_TYPE first and the other settings in a fixed order; libtrellis
//! applies the writes in that order and then the defaults of every unspecified
//! enum, in enum-name order, starting from zeroed CRAM. Inverting that writer
//! (both cell orders of a pair, and only the writes nextpnr can make) gives
//! every assignment that reproduces the observed bits. Images from other
//! writers may have no consistent assignment; those stay unmatched.
//!
//! A differential BASE_TYPE on the true PIO of a pair (A or C) also claims its
//! complement (B or D). nextpnr leaves the complement's BASE_TYPE unwritten and
//! writes only its PULLMODE=NONE, DRIVE and OPENDRAIN; the sysIO guide
//! (FPGA-TN-02032, §4.11.5) drives that pad with the complement of the true
//! pad's data and the true pad's tristate.
use super::*;

/// nextpnr's per-cell write order (ecp5/bitstream.cc, write_io).
const WRITE_ORDER: [&str; 11] = [
    "BASE_TYPE",
    "HYSTERESIS",
    "SLEWRATE",
    "PULLMODE",
    "DIFFRESISTOR",
    "CLAMP",
    "DRIVE",
    "TERMINATION_1V8",
    "TERMINATION_1V5",
    "TERMINATION_1V35",
    "OPENDRAIN",
];
/// Upper bound on simulated assignments per group before giving up.
const SEARCH_LIMIT: u64 = 20_000_000;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, serde::Deserialize)]
pub struct PioMode {
    /// resolved: every consistent assignment gives this PIO one direction.
    pub status: String,
    /// INPUT, OUTPUT, BIDIR, or NONE for an unwritten/NONE BASE_TYPE.
    pub direction: Option<String>,
    /// BASE_TYPE values of the consistent assignments. The single-ended IO
    /// standard is usually not distinguishable from these bits.
    pub base_types: Vec<String>,
    /// Effective settings written over BASE_TYPE: an unspecified enum takes its
    /// default, or "unwritten" when it has none.
    pub overlays: BTreeMap<String, Vec<String>>,
    /// PIOs decoded together because their settings share bits.
    pub group: Vec<String>,
    pub assignments: u64,
    pub rule: String,
    /// The true PIO of the pair, when this PIO is the complement of a
    /// differential BASE_TYPE in every consistent assignment.
    pub complement_of: Option<String>,
}

#[derive(Clone, Copy)]
struct Write {
    mask: u128,
    value: u128,
}

struct Choice {
    /// None: unspecified, so the fragment defaults land in the defaults pass.
    value: Option<String>,
    write: Write,
}

struct Slot<'a> {
    resource: usize,
    field: &'a Field,
    choices: Vec<Choice>,
}

fn suffix(field: &Field) -> &str {
    field.name.split_once('.').map_or("", |(_, s)| s)
}

/// Bits of a PIO's settings that the writer model covers.
pub(super) fn writer_positions(resource: &Resource) -> BTreeSet<(usize, usize)> {
    resource
        .fields
        .iter()
        .filter(|f| f.kind == Kind::Enum && WRITE_ORDER.contains(&suffix(f)))
        .flat_map(positions)
        .collect()
}

fn positions(field: &Field) -> BTreeSet<(usize, usize)> {
    field
        .fragments
        .iter()
        .flat_map(|fr| fr.options.values().flatten())
        .map(|&(frame, bit, _)| (frame, bit))
        .collect()
}

/// Modes of every PIO in `resources`, which must be closed under shared bits.
pub(super) fn group_modes(resources: &[&Resource], cram: &Cram) -> BTreeMap<String, PioMode> {
    let group: Vec<String> = resources.iter().map(|r| r.id.clone()).collect();
    let blank = |status: &str| {
        PioMode {
        status: status.into(),
        direction: None,
        base_types: Vec::new(),
        overlays: BTreeMap::new(),
        group: group.clone(),
        assignments: 0,
        rule: "BASE_TYPE decoded jointly with the settings written over it and with the PIO pair, by inverting the nextpnr/libtrellis writer order and defaults pass".into(),
        complement_of: None,
    }
    };
    let fail = |status: &str| {
        resources
            .iter()
            .map(|r| (r.id.clone(), blank(status)))
            .collect()
    };
    // Fields joined by shared bits, starting from every BASE_TYPE.
    let enums: Vec<(usize, &Field)> = resources
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.fields.iter().map(move |f| (i, f)))
        .filter(|(_, f)| f.kind == Kind::Enum)
        .collect();
    let mut chosen: Vec<(usize, &Field)> = enums
        .iter()
        .filter(|(_, f)| suffix(f) == "BASE_TYPE")
        .copied()
        .collect();
    if chosen.len() != resources.len() {
        return fail("no_base_type");
    }
    let mut covered: BTreeSet<(usize, usize)> =
        chosen.iter().flat_map(|(_, f)| positions(f)).collect();
    while let Some(&(i, f)) = enums.iter().find(|(i, f)| {
        !chosen.iter().any(|(j, g)| j == i && g.name == f.name)
            && !positions(f).is_disjoint(&covered)
    }) {
        if !WRITE_ORDER.contains(&suffix(f)) {
            return fail("unsupported_overlay");
        }
        covered.extend(positions(f));
        chosen.push((i, f));
    }
    if covered.len() > 128 {
        return fail("unsupported_overlay");
    }
    let index: BTreeMap<(usize, usize), usize> =
        covered.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let write = |bits: Vec<Bit>| {
        bits.iter()
            .fold(Write { mask: 0, value: 0 }, |w, &(f, b, v)| {
                let bit = 1_u128 << index[&(f, b)];
                Write {
                    mask: w.mask | bit,
                    value: if v { w.value | bit } else { w.value },
                }
            })
    };
    let observed = covered
        .iter()
        .enumerate()
        .filter(|&(_, &(f, b))| cram.bit(f, b))
        .fold(0_u128, |v, (i, _)| v | 1_u128 << i);
    let mut slots: Vec<Slot> = chosen
        .iter()
        .map(|&(resource, field)| {
            let values: BTreeSet<&String> = field
                .fragments
                .iter()
                .flat_map(|fr| fr.options.keys())
                .collect();
            let mut choices: Vec<Choice> = values
                .into_iter()
                .map(|v| Choice {
                    value: Some(v.clone()),
                    write: write(
                        field
                            .fragments
                            .iter()
                            .filter_map(|fr| fr.options.get(v))
                            .flatten()
                            .copied()
                            .collect(),
                    ),
                })
                .collect();
            choices.push(Choice {
                value: None,
                write: write(
                    field
                        .fragments
                        .iter()
                        .filter_map(|fr| fr.default.as_ref().and_then(|d| fr.options.get(d)))
                        .flatten()
                        .copied()
                        .collect(),
                ),
            });
            Slot {
                resource,
                field,
                choices,
            }
        })
        .collect();
    // A bit only one field touches ends as that choice's value, or 0.
    let touched: Vec<u128> = slots
        .iter()
        .map(|s| s.choices.iter().fold(0, |m, c| m | c.write.mask))
        .collect();
    for (n, slot) in slots.iter_mut().enumerate() {
        let others = touched
            .iter()
            .enumerate()
            .filter(|&(m, _)| m != n)
            .fold(0, |m, (_, t)| m | t);
        let exclusive = touched[n] & !others;
        slot.choices.retain(|c| {
            let value = c.write.value & c.write.mask & exclusive;
            value == observed & exclusive
        });
    }
    if slots.iter().any(|s| s.choices.is_empty()) {
        return fail("unmatched");
    }
    let apply = |s: u128, w: Write| (s & !w.mask) | (w.value & w.mask);
    // Writes of `order` (explicit ones), then defaults in enum-name order.
    let simulate = |order: &[usize], picks: &[usize]| {
        let mut defaults: Vec<usize> = order.to_vec();
        defaults.sort_by(|&a, &b| slots[a].field.name.cmp(&slots[b].field.name));
        let explicit = order.iter().map(|&n| &slots[n].choices[picks[n]]);
        let state = explicit
            .filter(|c| c.value.is_some())
            .fold(0, |s, c| apply(s, c.write));
        defaults
            .iter()
            .map(|&n| &slots[n].choices[picks[n]])
            .filter(|c| c.value.is_none())
            .fold(state, |s, c| apply(s, c.write))
    };
    // Each cell's slots in nextpnr's per-cell order.
    let rank = |n: usize| {
        WRITE_ORDER
            .iter()
            .position(|w| *w == suffix(slots[n].field))
    };
    let cells: Vec<Vec<usize>> = (0..resources.len())
        .map(|i| {
            let mut cell: Vec<usize> = (0..slots.len())
                .filter(|&n| slots[n].resource == i)
                .collect();
            cell.sort_by_key(|&n| rank(n));
            cell
        })
        .collect();
    // Bits only one cell touches depend on that cell alone, so each cell's
    // combinations are filtered on them before pairing.
    let mut survivors: Vec<Vec<Vec<usize>>> = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let mine = cell.iter().fold(0, |m, &n| m | touched[n]);
        let others = cells
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .flat_map(|(_, c)| c.iter())
            .fold(0, |m, &n| m | touched[n]);
        let private = mine & !others;
        let size = cell
            .iter()
            .try_fold(1_u64, |n, &s| n.checked_mul(slots[s].choices.len() as u64));
        if size.is_none_or(|n| n > SEARCH_LIMIT) {
            return fail("search_limit");
        }
        let mut picks = vec![0; slots.len()];
        let mut kept = Vec::new();
        loop {
            if nextpnr_writes(cell, &slots, &picks)
                && simulate(cell, &picks) & private == observed & private
            {
                kept.push(cell.iter().map(|&n| picks[n]).collect());
            }
            if !advance(cell, &slots, &mut picks) {
                break;
            }
        }
        survivors.push(kept);
    }
    let size = survivors
        .iter()
        .try_fold(1_u64, |n, s| n.checked_mul(s.len() as u64));
    if size.is_none_or(|n| n > SEARCH_LIMIT) {
        return fail("search_limit");
    }
    // The order of the cells within the tile is unknown: try every one.
    let mut orders: Vec<Vec<usize>> = Vec::new();
    let mut order: Vec<usize> = (0..cells.len()).collect();
    permutations(&mut order, 0, &mut |o| {
        orders.push(o.iter().flat_map(|&i| cells[i].iter().copied()).collect());
    });
    // The true PIO (A/C) of each complement PIO (B/D) in the group.
    let partner: Vec<Option<usize>> = resources
        .iter()
        .map(|r| {
            let (site, name) = r.id.rsplit_once('/')?;
            let true_name = match name {
                "PIOB" => "PIOA",
                "PIOD" => "PIOC",
                _ => return None,
            };
            resources
                .iter()
                .position(|o| o.id == format!("{site}/{true_name}"))
        })
        .collect();
    let complement_side: Vec<bool> = resources
        .iter()
        .map(|r| r.id.ends_with("PIOB") || r.id.ends_with("PIOD"))
        .collect();
    let base = |cell: &[usize], picks: &[usize]| {
        cell.iter()
            .find(|&&n| suffix(slots[n].field) == "BASE_TYPE")
            .and_then(|&n| slots[n].choices[picks[n]].value.clone())
    };
    let is_differential = |b: &Option<String>| {
        b.as_deref()
            .and_then(|b| b.split_once('_'))
            .is_some_and(|(_, standard)| differential(standard))
    };
    // Some(true): this cell is the complement of a differential true PIO.
    let roles = |picks: &[usize]| -> Option<Vec<bool>> {
        let bases: Vec<Option<String>> = cells.iter().map(|c| base(c, picks)).collect();
        (0..cells.len())
            .map(|i| {
                if complement_side[i] && is_differential(&bases[i]) {
                    return None; // nextpnr places differential IO on A/C only
                }
                let claimed = partner[i].is_some_and(|p| is_differential(&bases[p]));
                let unwritten_base = bases[i].is_none();
                let only_complement_writes = cells[i].iter().all(|&n| {
                    let value = slots[n].choices[picks[n]].value.as_deref();
                    match suffix(slots[n].field) {
                        "BASE_TYPE" => value.is_none(),
                        "PULLMODE" => value.is_none_or(|v| v == "NONE"),
                        "DRIVE" | "OPENDRAIN" => true,
                        _ => value.is_none(),
                    }
                });
                let all_unwritten = cells[i]
                    .iter()
                    .all(|&n| slots[n].choices[picks[n]].value.is_none());
                if claimed {
                    (unwritten_base && only_complement_writes).then_some(true)
                } else {
                    (!unwritten_base || all_unwritten).then_some(false)
                }
            })
            .collect()
    };
    let mut seen: Vec<BTreeSet<String>> = vec![BTreeSet::new(); slots.len()];
    let mut seen_roles: Vec<BTreeSet<bool>> = vec![BTreeSet::new(); resources.len()];
    let mut assignments = 0_u64;
    let mut combo = vec![0; cells.len()];
    let mut picks = vec![0; slots.len()];
    'search: loop {
        if survivors.iter().all(|s| !s.is_empty()) {
            for (i, cell) in cells.iter().enumerate() {
                for (k, &n) in cell.iter().enumerate() {
                    picks[n] = survivors[i][combo[i]][k];
                }
            }
            if let Some(role) = roles(&picks) {
                if orders.iter().any(|o| simulate(o, &picks) == observed) {
                    assignments += 1;
                    for (n, slot) in slots.iter().enumerate() {
                        seen[n].extend(effective(slot, &slot.choices[picks[n]], observed));
                    }
                    for (i, r) in role.into_iter().enumerate() {
                        seen_roles[i].insert(r);
                    }
                }
            }
        }
        let mut i = 0;
        loop {
            if i == combo.len() || survivors.iter().any(|s| s.is_empty()) {
                break 'search;
            }
            combo[i] += 1;
            if combo[i] < survivors[i].len() {
                break;
            }
            combo[i] = 0;
            i += 1;
        }
    }
    resources
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut mode = blank("");
            mode.assignments = assignments;
            for (n, slot) in slots.iter().enumerate().filter(|(_, s)| s.resource == i) {
                let values: Vec<String> = seen[n].iter().cloned().collect();
                if suffix(slot.field) == "BASE_TYPE" {
                    mode.base_types = values;
                } else {
                    mode.overlays.insert(suffix(slot.field).into(), values);
                }
            }
            let directions: BTreeSet<&str> = mode
                .base_types
                .iter()
                .map(|t| match t.split_once('_') {
                    Some((d, _)) => d,
                    None => "NONE",
                })
                .collect();
            mode.status = match directions.len() {
                0 => "unmatched",
                1 => {
                    mode.direction = directions.first().map(|d| d.to_string());
                    "resolved"
                }
                _ => "ambiguous",
            }
            .into();
            if seen_roles[i].contains(&true) {
                if seen_roles[i].len() == 1 {
                    mode.complement_of = partner[i].map(|p| resources[p].id.clone());
                } else {
                    mode.status = "ambiguous_complement".into();
                }
            }
            (r.id.clone(), mode)
        })
        .collect()
}

/// An unspecified enum with a default takes it; an unspecified BASE_TYPE with
/// defaults means no IO cell was placed. Without defaults, the bits are what
/// other writes left, read back as every option whose pattern they match.
fn effective(slot: &Slot, choice: &Choice, observed: u128) -> Vec<String> {
    if let Some(v) = &choice.value {
        return vec![v.clone()];
    }
    let defaults: BTreeSet<String> = slot
        .field
        .fragments
        .iter()
        .filter_map(|fr| fr.default.clone())
        .collect();
    if !defaults.is_empty() {
        return if suffix(slot.field) == "BASE_TYPE" {
            vec!["unwritten".into()]
        } else {
            defaults.into_iter().collect()
        };
    }
    let matching: Vec<String> = slot
        .choices
        .iter()
        .filter(|c| c.value.is_some() && observed & c.write.mask == c.write.value & c.write.mask)
        .filter_map(|c| c.value.clone())
        .collect();
    if matching.is_empty() {
        vec!["unwritten".into()]
    } else {
        matching
    }
}

/// Odometer over the choices of `cell`; false once every combination is done.
fn advance(cell: &[usize], slots: &[Slot], picks: &mut [usize]) -> bool {
    for &n in cell {
        picks[n] += 1;
        if picks[n] < slots[n].choices.len() {
            return true;
        }
        picks[n] = 0;
    }
    false
}

pub(crate) fn differential(standard: &str) -> bool {
    ["LVDS", "LVPECL", "SLVS"]
        .iter()
        .any(|d| standard.contains(d))
        || standard.ends_with('D')
        || standard.contains("D_")
}

/// What nextpnr's write_io can produce for one cell: BASE_TYPE is always
/// written, never as NONE, and nothing else is written without it, except the
/// PULLMODE=NONE, DRIVE and OPENDRAIN of a differential pair's complement
/// (checked with the pair in `roles`); HYSTERESIS is written exactly for
/// single-ended, non-referenced inputs and bidirs; DRIVE only for
/// LVCMOS33/LVCMOS33D.
fn nextpnr_writes(cell: &[usize], slots: &[Slot], picks: &[usize]) -> bool {
    let value = |name: &str| {
        cell.iter()
            .find(|&&n| suffix(slots[n].field) == name)
            .map(|&n| slots[n].choices[picks[n]].value.as_deref())
    };
    let Some(Some(base)) = value("BASE_TYPE") else {
        return cell.iter().all(|&n| {
            let value = slots[n].choices[picks[n]].value.as_deref();
            match suffix(slots[n].field) {
                "PULLMODE" => value.is_none_or(|v| v == "NONE"),
                "DRIVE" | "OPENDRAIN" => true,
                _ => value.is_none(),
            }
        });
    };
    let Some((direction, standard)) = base.split_once('_') else {
        return false;
    };
    // nextpnr classifies the emulated E standards as single-ended for its
    // HYSTERESIS write, even though BASE_TYPE still describes a digital pair.
    // Keep them decodable so unsupported BIDIR_*E gets the planner's reason.
    let writer_differential = differential(standard) && !standard.ends_with('E');
    let referenced =
        !writer_differential && (standard.starts_with("SSTL") || standard.starts_with("HSUL"));
    let hysteresis = ["INPUT", "BIDIR"].contains(&direction) && !writer_differential && !referenced;
    if value("HYSTERESIS").is_some_and(|h| h.is_some() != hysteresis) {
        return false;
    }
    !value("DRIVE").is_some_and(|d| d.is_some() && !["LVCMOS33", "LVCMOS33D"].contains(&standard))
}

fn permutations(items: &mut Vec<usize>, k: usize, visit: &mut impl FnMut(&[usize])) {
    if k == items.len() {
        visit(items);
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permutations(items, k + 1, visit);
        items.swap(k, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One PIO of a synthetic pair, shaped like the real PICL1 overlaps:
    /// DRIVE clears bits of the OUTPUT pattern, the PULLMODE default clears a
    /// BASE_TYPE bit, OPENDRAIN shares a bit with DRIVE, and PIOB's BASE_TYPE
    /// reaches into PIOA's pull bit.
    fn pio(name: &str, o: usize) -> Resource {
        let b = |bits: &[(usize, bool)]| -> serde_json::Value {
            bits.iter().map(|&(i, v)| json!([0, o + i, v])).collect()
        };
        let mut output = b(&[(0, true), (3, true), (4, true), (5, true), (6, true)]);
        if name == "PIOB" {
            output.as_array_mut().unwrap().push(json!([0, 6, true])); // PIOA's pull bit
        }
        let base = json!({"kind":"enum","name":format!("{name}.BASE_TYPE"),"fragments":[{"tile":"T","source":"t","default":"NONE","options":{
            "NONE":b(&[(0,true)]),
            "INPUT_LVCMOS33":b(&[(0,true),(1,true),(2,true)]),
            "OUTPUT_LVCMOS33":output,
            "OUTPUT_SSTL135_II":b(&[(0,true),(3,true),(6,true)]),
            "OUTPUT_LVCMOS33D":b(&[(0,true),(3,true),(7,true),(6,true)]),
            "BIDIR_LVCMOS33":b(&[(0,true),(1,true),(2,true),(3,true),(4,true),(5,true),(6,true)])}}]});
        let enums = [
            (
                "DRIVE",
                json!(null),
                json!({"4":b(&[(4,false),(5,false),(8,true)]),"8":b(&[(4,true),(5,true),(8,false)])}),
            ),
            (
                "PULLMODE",
                json!("DOWN"),
                json!({"DOWN":b(&[(6,false),(9,false)]),"NONE":b(&[(6,true),(9,false)]),"UP":b(&[(6,true),(9,true)])}),
            ),
            (
                "OPENDRAIN",
                json!(null),
                json!({"OFF":b(&[(4,true),(10,false)]),"ON":b(&[(4,false),(10,true)])}),
            ),
        ];
        let mut fields = vec![serde_json::from_value(base).unwrap()];
        for (suffix, default, options) in enums {
            fields.push(
                serde_json::from_value(json!({"kind":"enum","name":format!("{name}.{suffix}"),
                "fragments":[{"tile":"T","source":"t","default":default,"options":options}]}))
                .unwrap(),
            );
        }
        Resource {
            id: format!("X/{name}"),
            family: "PIO".into(),
            fields,
        }
    }
    fn catalog() -> Catalog {
        Catalog {
            schema_version: 1,
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 64,
            resources: vec![pio("PIOA", 0), pio("PIOB", 16)],
        }
    }
    /// The libtrellis writer, independent of the inversion: explicit writes in
    /// order, then the defaults of unspecified enums in name order.
    fn write(catalog: &Catalog, writes: &[(&str, &str)]) -> Cram {
        let mut bits = 0_u64;
        let fields: Vec<&Field> = catalog.resources.iter().flat_map(|r| &r.fields).collect();
        let mut apply = |field: &Field, value: &str| {
            for &(_, bit, v) in field.fragments.iter().flat_map(|f| &f.options[value]) {
                bits = if v {
                    bits | 1 << bit
                } else {
                    bits & !(1 << bit)
                };
            }
        };
        for &(name, value) in writes {
            apply(fields.iter().find(|f| f.name == name).unwrap(), value);
        }
        let mut rest: Vec<&&Field> = fields
            .iter()
            .filter(|f| !writes.iter().any(|(n, _)| *n == f.name))
            .collect();
        rest.sort_by(|a, b| a.name.cmp(&b.name));
        for field in rest {
            if let Some(default) = &field.fragments[0].default {
                apply(field, default);
            }
        }
        Cram {
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 64,
            data: bits.to_le_bytes().to_vec(),
            bram: BTreeMap::new(),
            origin: "text_config".into(),
        }
    }

    #[test]
    fn drive_and_default_pullmode_do_not_turn_an_output_into_another_standard() {
        let c = catalog();
        let cram = write(
            &c,
            &[("PIOA.BASE_TYPE", "OUTPUT_LVCMOS33"), ("PIOA.DRIVE", "4")],
        );
        // Independent matching reads the leftovers as an unused PIO, the
        // failure first seen with the HS01 LEDs.
        let raw = c.resources[0].fields[0].decode(&cram);
        assert_eq!(raw.candidates, ["NONE"]);
        let modes = c.pio_modes(&cram, "X/PIOA").unwrap();
        assert_eq!(modes.len(), 2);
        let a = &modes["X/PIOA"];
        assert_eq!(a.status, "resolved");
        assert_eq!(a.direction.as_deref(), Some("OUTPUT"));
        assert!(a.base_types.contains(&"OUTPUT_LVCMOS33".to_string()));
        assert!(!a.base_types.iter().any(|t| t.ends_with('D')));
        assert_eq!(a.overlays["DRIVE"], ["4"]);
        assert_eq!(a.overlays["PULLMODE"], ["DOWN"]);
        assert_eq!(a.overlays["OPENDRAIN"], ["unwritten"]);
        assert_eq!(modes["X/PIOB"].direction.as_deref(), Some("NONE"));
    }

    #[test]
    fn pair_cells_decode_in_either_write_order() {
        let c = catalog();
        let b_first = [
            ("PIOB.BASE_TYPE", "OUTPUT_LVCMOS33"),
            ("PIOB.PULLMODE", "NONE"),
            ("PIOA.BASE_TYPE", "INPUT_LVCMOS33"),
            ("PIOA.PULLMODE", "UP"),
        ];
        let modes = c.pio_modes(&write(&c, &b_first), "X/PIOB").unwrap();
        let (a, b) = (&modes["X/PIOA"], &modes["X/PIOB"]);
        assert_eq!(a.direction.as_deref(), Some("INPUT"));
        assert_eq!(a.overlays["PULLMODE"], ["UP"]);
        assert_eq!(b.direction.as_deref(), Some("OUTPUT"));
        assert_eq!(b.overlays["PULLMODE"], ["NONE"]);
    }

    #[test]
    fn a_differential_true_pio_claims_its_complement() {
        let c = catalog();
        // nextpnr's top-bank LVCMOS33D: the complement gets only DRIVE.
        let cram = write(
            &c,
            &[
                ("PIOA.BASE_TYPE", "OUTPUT_LVCMOS33D"),
                ("PIOA.DRIVE", "4"),
                ("PIOB.DRIVE", "4"),
            ],
        );
        let modes = c.pio_modes(&cram, "X/PIOB").unwrap();
        let (a, b) = (&modes["X/PIOA"], &modes["X/PIOB"]);
        assert_eq!(a.base_types, ["OUTPUT_LVCMOS33D"]);
        assert_eq!(a.complement_of, None);
        assert_eq!(b.base_types, ["unwritten"]);
        assert_eq!(b.complement_of.as_deref(), Some("X/PIOA"));
        // A single-ended true PIO leaves an unused PIOB unclaimed.
        let cram = write(&c, &[("PIOA.BASE_TYPE", "OUTPUT_LVCMOS33")]);
        let b = &c.pio_modes(&cram, "X/PIOB").unwrap()["X/PIOB"];
        assert_eq!(b.complement_of, None);
    }

    #[test]
    fn differential_input_and_output_aliases_keep_the_same_pair_owner() {
        let mut c = catalog();
        let options = [
            ("OUTPUT_LVDS", "OUTPUT_LVCMOS33D"),
            ("OUTPUT_LVCMOS18D", "OUTPUT_LVCMOS33D"),
            ("OUTPUT_LVDS25E", "OUTPUT_LVCMOS33D"),
            ("OUTPUT_LVPECL33E", "OUTPUT_LVCMOS33D"),
            ("OUTPUT_BLVDS25E", "OUTPUT_LVCMOS33D"),
            ("OUTPUT_MLVDS25E", "OUTPUT_LVCMOS33D"),
            ("INPUT_LVDS", "INPUT_LVCMOS33"),
            ("INPUT_BLVDS25", "INPUT_LVCMOS33"),
            ("INPUT_MLVDS25", "INPUT_LVCMOS33"),
            ("INPUT_LVPECL33", "INPUT_LVCMOS33"),
            ("INPUT_SLVS", "INPUT_LVCMOS33"),
            ("INPUT_SUBLVDS", "INPUT_LVCMOS33"),
        ];
        for (index, resource) in c.resources.iter_mut().enumerate() {
            let fragment = &mut resource.fields[0].fragments[0];
            for (name, source) in options {
                let mut bits = fragment.options[source].clone();
                if name.starts_with("INPUT_") {
                    bits.push((0, index * 16 + 7, true));
                }
                fragment.options.insert(name.into(), bits);
            }
        }
        for (base, _) in options {
            let cram = write(
                &c,
                &[
                    ("PIOA.BASE_TYPE", base),
                    ("PIOA.PULLMODE", "NONE"),
                    ("PIOB.PULLMODE", "NONE"),
                ],
            );
            let modes = c.pio_modes(&cram, "X/PIOB").unwrap();
            let (a, b) = (&modes["X/PIOA"], &modes["X/PIOB"]);
            assert_eq!(
                a.direction.as_deref(),
                base.split_once('_').map(|(d, _)| d),
                "{base}: {a:?}"
            );
            assert!(a.base_types.contains(&base.to_string()), "{base}: {a:?}");
            assert!(
                a.base_types
                    .iter()
                    .all(|b| differential(b.split_once('_').unwrap().1)),
                "{base}: {a:?}"
            );
            assert_eq!(b.complement_of.as_deref(), Some("X/PIOA"), "{base}: {b:?}");
        }
    }

    #[test]
    fn differential_classifier_covers_every_catalogue_pair_type() {
        let mut reader = csv::Reader::from_reader(
            include_bytes!("../../docs/inventory/LFE5U-85F/settings_coverage.csv").as_slice(),
        );
        let row = reader
            .records()
            .map(Result::unwrap)
            .find(|r| &r[0] == "PIO" && &r[1] == "BASE_TYPE")
            .unwrap();
        let pairs: Vec<_> = row[2]
            .split(" | ")
            .filter(|b| b.split_once('_').is_some_and(|(_, s)| differential(s)))
            .collect();
        assert_eq!(pairs.len(), 46);
        for standard in [
            "LVDS",
            "BLVDS25",
            "MLVDS25",
            "SLVS",
            "SUBLVDS",
            "LVPECL33",
            "LVDS25E",
            "BLVDS25E",
            "MLVDS25E",
            "LVPECL33E",
            "HSUL12D",
            "SSTL135D_II",
            "LVCMOS18D",
        ] {
            assert!(differential(standard), "{standard}");
        }
        for standard in ["LVCMOS33", "LVTTL33", "HSUL12", "SSTL135_II"] {
            assert!(!differential(standard), "{standard}");
        }
    }

    #[test]
    fn emulated_bidir_keeps_nextpnrs_single_ended_hysteresis_write() {
        let mut c = catalog();
        let base = &mut c.resources[0].fields[0].fragments[0];
        let mut bits = base.options["BIDIR_LVCMOS33"].clone();
        bits.push((0, 7, true));
        base.options.insert("BIDIR_MLVDS25E".into(), bits);
        c.resources[0].fields.push(
            serde_json::from_value(json!({
                "kind":"enum", "name":"PIOA.HYSTERESIS", "fragments":[{
                    "tile":"T", "source":"t", "default":"OFF",
                "options":{"OFF":[[0,2,false],[0,11,false]],"ON":[[0,2,true],[0,11,true]]}
                }]
            }))
            .unwrap(),
        );
        let cram = write(
            &c,
            &[
                ("PIOA.BASE_TYPE", "BIDIR_MLVDS25E"),
                ("PIOA.HYSTERESIS", "ON"),
                ("PIOA.PULLMODE", "NONE"),
                ("PIOB.PULLMODE", "NONE"),
            ],
        );
        let modes = c.pio_modes(&cram, "X/PIOA").unwrap();
        let a = &modes["X/PIOA"];
        assert_eq!(a.direction.as_deref(), Some("BIDIR"));
        assert_eq!(a.base_types, ["BIDIR_MLVDS25E"]);
        assert_eq!(a.overlays["HYSTERESIS"], ["ON"]);
        assert_eq!(modes["X/PIOB"].complement_of.as_deref(), Some("X/PIOA"));
    }

    #[test]
    fn open_drain_is_recognised_and_a_foreign_image_stays_unmatched() {
        let c = catalog();
        let cram = write(
            &c,
            &[
                ("PIOA.BASE_TYPE", "OUTPUT_LVCMOS33"),
                ("PIOA.PULLMODE", "NONE"),
                ("PIOA.OPENDRAIN", "ON"),
            ],
        );
        assert_eq!(
            c.pio_modes(&cram, "X/PIOA").unwrap()["X/PIOA"].overlays["OPENDRAIN"],
            ["ON"]
        );
        // BASE_TYPE bit 3 without its bit 0 is no writer's output.
        let mut foreign = write(&c, &[]);
        foreign.data[0] = 0b1000;
        let a = &c.pio_modes(&foreign, "X/PIOA").unwrap()["X/PIOA"];
        assert_eq!(a.status, "unmatched");
        assert_eq!(a.direction, None);
    }
}
