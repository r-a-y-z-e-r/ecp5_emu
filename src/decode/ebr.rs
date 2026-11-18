//! ECP5 EBR.MODE is a base pattern; nextpnr writes WEAMUX afterwards in DP.
//! Qualification is limited to its verified one-bit overlap, never arbitrary
//! masking of unmatched MODE constraints or inference from one enable fragment.
use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, serde::Deserialize)]
pub struct ModeOverride {
    pub selector: String,
    pub frame: usize,
    pub bit: usize,
    pub base_value: bool,
    pub observed_value: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, serde::Deserialize)]
pub struct EbrModeResult {
    pub status: String,
    pub value: Option<String>,
    pub candidates: Vec<String>,
    pub overrides: Vec<ModeOverride>,
    pub rule: String,
}

fn coordinates(field: &Field) -> BTreeSet<(usize, usize)> {
    field
        .constraints()
        .into_iter()
        .map(|(frame, bit, _)| (frame, bit))
        .collect()
}

pub(super) fn mode(
    resource: &Resource,
    decoded: &[FieldResult],
    cram: &Cram,
) -> Option<EbrModeResult> {
    let raw = decoded
        .iter()
        .find(|f| f.name.split('.').count() == 2 && f.name.ends_with(".MODE"))?;
    let mut result = EbrModeResult {
        status: raw.status.clone(),
        value: raw.value.clone(),
        candidates: raw.candidates.clone(),
        overrides: Vec::new(),
        rule: "complete raw MODE pattern; aliases, incomplete fragments and unmatched constraints remain unresolved".into(),
    };
    // A strict match is already complete. Partial and ambiguous catalogues do
    // not acquire an interpretation merely because a WEAMUX field exists.
    if raw.status != "unmatched" {
        return Some(result);
    }
    let Some(base) = resource
        .fields
        .iter()
        .find(|f| f.name == raw.name && f.kind == Kind::Enum)
    else {
        return Some(result);
    };
    let name = format!("{}.WEAMUX", raw.name.strip_suffix(".MODE")?);
    let Some(selector) = resource
        .fields
        .iter()
        .find(|f| f.name == name && f.kind == Kind::Enum)
    else {
        return Some(result);
    };
    if selector.fragments.len() != 1 {
        return Some(result);
    }
    let fragment = &selector.fragments[0];
    let Some(wea) = fragment.options.get("WEA") else {
        return Some(result);
    };
    let Some(inv) = fragment.options.get("INV") else {
        return Some(result);
    };
    if fragment.options.len() != 2 || wea.len() != 1 || inv.len() != 1 {
        return Some(result);
    }
    let (frame, bit, value) = wea[0];
    if value || inv[0] != (frame, bit, true) || !cram.bit(frame, bit) {
        return Some(result);
    }
    let observed_selector = decoded.iter().find(|f| f.name == name)?;
    if observed_selector.status != "resolved" || observed_selector.value.as_deref() != Some("INV") {
        return Some(result);
    }
    let all = coordinates(base);
    if all
        .intersection(&coordinates(selector))
        .copied()
        .collect::<Vec<_>>()
        != [(frame, bit)]
    {
        return Some(result);
    }
    if resource
        .fields
        .iter()
        .filter(|f| f.name != raw.name && f.name != name)
        .any(|f| !all.is_disjoint(&coordinates(f)))
    {
        return Some(result);
    }
    let modes = BTreeSet::from(["DP16KD", "NONE", "PDPW16KD"]);
    let mut shared_fragments = 0;
    for fragment in &base.fragments {
        if fragment
            .options
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != modes
        {
            return Some(result);
        }
        let mut same_coordinates = None;
        let mut shared = false;
        for (mode, bits) in &fragment.options {
            if bits.is_empty() {
                return Some(result);
            }
            let coords = bits
                .iter()
                .map(|&(f, b, _)| (f, b))
                .collect::<BTreeSet<_>>();
            // Contradictory/repeated constraints and differently covered modes
            // are not the known base pattern we are allowed to reinterpret.
            if coords.len() != bits.len()
                || same_coordinates.as_ref().is_some_and(|old| old != &coords)
            {
                return Some(result);
            }
            same_coordinates = Some(coords);
            if let Some(&(_, _, value)) = bits.iter().find(|&&(f, b, _)| (f, b) == (frame, bit)) {
                if value != (mode == "PDPW16KD") || fragment.tile != selector.fragments[0].tile {
                    return Some(result);
                }
                shared = true;
            }
        }
        shared_fragments += usize::from(shared);
    }
    if shared_fragments != 1 {
        return Some(result);
    }
    // PDP does not write WEAMUX in nextpnr. Its base 1 must match as-is; only
    // DP/NONE may replace the base 0 with the selector's observed 1.
    let candidates = ["DP16KD", "NONE"]
        .into_iter()
        .filter(|mode| {
            base.fragments.iter().all(|fragment| {
                fragment.options[*mode]
                    .iter()
                    .all(|&(f, b, v)| (f, b) == (frame, bit) || cram.bit(f, b) == v)
            })
        })
        .map(String::from)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Some(result);
    }
    result.status = if candidates.len() == 1 {
        "qualified"
    } else {
        "ambiguous"
    }
    .into();
    result.value = if candidates.len() == 1 {
        Some(candidates[0].clone())
    } else {
        None
    };
    result.candidates = candidates;
    result.overrides.push(ModeOverride {
        selector: name,
        frame,
        bit,
        base_value: false,
        observed_value: true,
    });
    result.rule="complete MODE across all fragments with exactly one verified WEAMUX overlay; DP/NONE base bit=0, INV bit=1; PDP requires an unmodified strict MODE match".into();
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resource(id: &str) -> Resource {
        let word = (4..13).map(|b| vec![(0, b, true)]).collect::<Vec<_>>();
        serde_json::from_value(json!({"id":id,"family":"EBR","fields":[
            {"kind":"enum","name":"EBR0.MODE","fragments":[
                {"tile":"A","source":"mode:a","options":{"NONE":[[0,0,false],[0,1,false]],"DP16KD":[[0,0,false],[0,1,false]],"PDPW16KD":[[0,0,true],[0,1,true]]}},
                {"tile":"B","source":"mode:b","options":{"NONE":[[0,2,false],[0,3,false]],"DP16KD":[[0,2,true],[0,3,false]],"PDPW16KD":[[0,2,true],[0,3,true]]}}]},
            {"kind":"enum","name":"EBR0.WEAMUX","fragments":[{"tile":"A","source":"wea","options":{"WEA":[[0,0,false]],"INV":[[0,0,true]]}}]},
            {"kind":"word","name":"EBR0.WID","fragments":[{"tile":"B","source":"wid","word_bits":word}]}
        ]})).unwrap()
    }
    fn catalog(resources: Vec<Resource>) -> Catalog {
        Catalog {
            schema_version: 1,
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 16,
            resources,
        }
    }
    fn cram(bits: u16) -> Cram {
        Cram {
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 16,
            data: bits.to_le_bytes().to_vec(),
            bram: BTreeMap::from([(0, vec![0x155; 2048])]),
            origin: "text_config".into(),
        }
    }

    #[test]
    fn qualified_mode_preserves_raw_evidence_and_binds_only_active_init() {
        let catalog = catalog(vec![resource("ebr")]);
        for (bits, mode, status, raw, enabled) in [
            (0, "NONE", "resolved", "resolved", false),
            (1, "NONE", "qualified", "unmatched", false),
            (4, "DP16KD", "resolved", "resolved", true),
            (5, "DP16KD", "qualified", "unmatched", true),
            (15, "PDPW16KD", "resolved", "resolved", true),
        ] {
            let report = catalog.decode(&cram(bits), &[]).unwrap();
            let r = &report.resources[0];
            let effective = r.ebr_mode.as_ref().unwrap();
            assert_eq!(effective.value.as_deref(), Some(mode));
            assert_eq!(effective.status, status);
            assert_eq!(r.enabled, Some(enabled));
            assert_eq!(r.fields[0].status, raw);
            assert_eq!(
                r.fields[1].value.as_deref(),
                Some(if bits & 1 != 0 { "INV" } else { "WEA" })
            );
            assert_eq!(
                effective.overrides.len(),
                usize::from(status == "qualified")
            );
            assert_eq!(
                r.memory.as_ref().unwrap().status,
                if enabled { "bound" } else { "inactive" }
            );
        }
        for bits in [7, 13, 14] {
            let r = catalog.decode(&cram(bits), &[]).unwrap();
            assert!(r.resources[0].ebr_mode.as_ref().unwrap().value.is_none());
            assert_eq!(
                r.resources[0].memory.as_ref().unwrap().status,
                "indeterminate"
            );
        }
    }
    #[test]
    fn overlay_does_not_hide_unknown_other_bits_or_relax_incomplete_catalogues() {
        let base = resource("ebr");
        let mut variants = Vec::new();
        let mut r = base.clone();
        r.fields[0].fragments.pop();
        variants.push(r); // DP/NONE alias
        let mut r = base.clone();
        r.fields[0].fragments[1].options.remove("PDPW16KD");
        variants.push(r);
        let mut r = base.clone();
        r.fields[0].fragments[0]
            .options
            .get_mut("NONE")
            .unwrap()
            .clear();
        variants.push(r);
        let mut r = base.clone();
        r.fields[1].fragments[0]
            .options
            .insert("EXTRA".into(), vec![(0, 0, true)]);
        variants.push(r);
        let mut r = base.clone();
        r.fields[1].fragments[0]
            .options
            .get_mut("INV")
            .unwrap()
            .push((0, 13, false));
        variants.push(r);
        let mut r = base.clone();
        let mut extra = r.fields[1].clone();
        extra.name = "EBR0.UNKNOWN_OVERLAY".into();
        r.fields.push(extra);
        variants.push(r);
        let mut r = base.clone();
        r.fields[1].fragments[0].tile = "other".into();
        variants.push(r);
        for r in variants {
            let report = catalog(vec![r]).decode(&cram(5), &[]).unwrap();
            assert!(report.resources[0]
                .ebr_mode
                .as_ref()
                .unwrap()
                .value
                .is_none());
            assert_eq!(
                report.resources[0].memory.as_ref().unwrap().status,
                "indeterminate"
            );
        }
        let report = catalog(vec![base]).decode(&cram(7), &[]).unwrap();
        assert_eq!(
            report.resources[0].fields[0].observed_bits,
            vec![(0, 0, true), (0, 1, true), (0, 2, true), (0, 3, false)]
        );
    }
    #[test]
    fn hidden_qualified_owners_still_prevent_duplicate_wid_binding() {
        let report = catalog(vec![resource("shown"), resource("hidden")])
            .decode(&cram(5), &["shown".into()])
            .unwrap();
        assert_eq!(report.resources.len(), 1);
        assert_eq!(
            report.resources[0].memory.as_ref().unwrap().status,
            "duplicate_wid"
        );
        assert_eq!(report.images[&0].potential_owners, ["shown", "hidden"]);
    }

    #[test]
    fn all_208_pinned_resources_keep_every_non_overlay_mode_constraint() {
        let catalog: Catalog = serde_json::from_str(include_str!(
            "../../docs/inventory/LFE5U-85F/decode_catalog.json"
        ))
        .unwrap();
        let mut cram = Cram {
            device: catalog.device.clone(),
            frames: catalog.frames,
            bits_per_frame: catalog.bits_per_frame,
            data: vec![0; catalog.frames * (catalog.bits_per_frame / 8)],
            bram: BTreeMap::from([(0, vec![0x155; 2048])]),
            origin: "text_config".into(),
        };
        let mut count = 0;
        for resource in catalog.resources.iter().filter(|r| r.family == "EBR") {
            let base = resource
                .fields
                .iter()
                .find(|f| f.name.ends_with(".MODE"))
                .unwrap();
            let selector = resource
                .fields
                .iter()
                .find(|f| f.name.ends_with(".WEAMUX"))
                .unwrap();
            let shared = coordinates(base)
                .intersection(&coordinates(selector))
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(shared.len(), 1);
            let write = |cram: &mut Cram, (f, b, v): Bit| {
                let byte = &mut cram.data[f * (cram.bits_per_frame / 8) + b / 8];
                if v {
                    *byte |= 1 << (b % 8);
                } else {
                    *byte &= !(1 << (b % 8));
                }
            };
            let cat = Catalog {
                schema_version: 1,
                device: catalog.device.clone(),
                frames: catalog.frames,
                bits_per_frame: catalog.bits_per_frame,
                resources: vec![resource.clone()],
            };
            for (name, inverted) in [
                ("NONE", false),
                ("NONE", true),
                ("DP16KD", false),
                ("DP16KD", true),
                ("PDPW16KD", false),
            ] {
                for fragment in &base.fragments {
                    for &bit in &fragment.options[name] {
                        write(&mut cram, bit);
                    }
                }
                if inverted {
                    write(&mut cram, (shared[0].0, shared[0].1, true));
                }
                let report = cat.decode(&cram, &[]).unwrap();
                let r = &report.resources[0];
                assert_eq!(
                    r.ebr_mode.as_ref().unwrap().value.as_deref(),
                    Some(name),
                    "{}",
                    r.id
                );
                assert_eq!(
                    r.fields
                        .iter()
                        .find(|f| f.name == base.name)
                        .unwrap()
                        .status,
                    if inverted { "unmatched" } else { "resolved" }
                );
            }
            for fragment in &base.fragments {
                for &bit in &fragment.options["DP16KD"] {
                    write(&mut cram, bit);
                }
            }
            write(&mut cram, (shared[0].0, shared[0].1, true));
            for (f, b) in coordinates(base)
                .into_iter()
                .filter(|p| !shared.contains(p))
            {
                let old = cram.bit(f, b);
                write(&mut cram, (f, b, !old));
                assert!(
                    cat.decode(&cram, &[]).unwrap().resources[0]
                        .ebr_mode
                        .as_ref()
                        .unwrap()
                        .value
                        .is_none(),
                    "{} accepted corrupted bit {f}:{b}",
                    resource.id
                );
                write(&mut cram, (f, b, old));
            }
            count += 1;
        }
        assert_eq!(count, 208);
    }
}
