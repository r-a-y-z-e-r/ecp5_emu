//! CIBOUT_BYP OFF was fuzzed with output CLK0 and contains that clock's
//! complete pattern. Qualify only the verified two-bit selector shape plus
//! a complete decoded output clock; retain the raw field and changed bits.
use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, serde::Deserialize)]
pub struct DspOutputPath {
    pub status: String,
    pub value: Option<String>,
    pub raw_status: String,
    pub output_clock_candidates: Vec<String>,
    pub selector_bits: Vec<Bit>,
    pub clock_overrides: Vec<ModeOverride>,
    pub rule: String,
}
pub(super) fn output_path(
    resource: &Resource,
    decoded: &[FieldResult],
    cram: &Cram,
) -> Option<DspOutputPath> {
    let raw = decoded.iter().find(|f| f.name.ends_with(".CIBOUT_BYP"))?;
    let mut result=DspOutputPath {status:raw.status.clone(),value:raw.value.clone(),raw_status:raw.status.clone(),output_clock_candidates:Vec::new(),selector_bits:Vec::new(),clock_overrides:Vec::new(),rule:"raw CIBOUT_BYP; unverified shapes and incomplete/ambiguous clocks acquire no overlay interpretation".into()};
    let field = resource
        .fields
        .iter()
        .find(|f| f.name == raw.name && f.kind == Kind::Enum)?;
    let clock_name = raw.name.replace(".CIBOUT_BYP", ".REG_OUTPUT_CLK");
    let Some(clock) = resource
        .fields
        .iter()
        .find(|f| f.name == clock_name && f.kind == Kind::Enum)
    else {
        return Some(result);
    };
    let Some(observed_clock) = decoded.iter().find(|f| f.name == clock_name) else {
        return Some(result);
    };
    result.output_clock_candidates = observed_clock.candidates.clone();
    if decoded
        .iter()
        .find(|f| f.name == raw.name.replace(".CIBOUT_BYP", ".MODE"))
        .and_then(|f| f.value.as_deref())
        != Some("MULT18X18D")
    {
        return Some(result);
    }
    let bypass_domain = BTreeSet::from(["OFF", "ON"]);
    let clock_domain = BTreeSet::from(["NONE", "CLK0", "CLK1", "CLK2", "CLK3"]);
    if ![3, 4].contains(&field.fragments.len())
        || !field.fragments.iter().all(|f| {
            f.options
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == bypass_domain
        })
        || ![2, 3].contains(&clock.fragments.len())
        || !clock.fragments.iter().all(|f| {
            f.options
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == clock_domain
        })
    {
        return Some(result);
    }
    let option = |f: &Field, value: &str| {
        f.fragments
            .iter()
            .flat_map(|f| f.options[value].iter().copied())
            .collect::<BTreeSet<_>>()
    };
    let off = option(field, "OFF");
    let on = option(field, "ON");
    let clk0 = option(clock, "CLK0");
    let clock_coordinates = clock
        .constraints()
        .into_iter()
        .map(|(f, b, _)| (f, b))
        .collect::<BTreeSet<_>>();
    if off != clk0
        || ![8, 12].contains(&off.len())
        || off.iter().any(|&(_, _, v)| !v)
        || on.len() != 2
        || on
            .iter()
            .any(|&(f, b, v)| !v || clock_coordinates.contains(&(f, b)))
    {
        return Some(result);
    }
    // Only the observed constant-clock overlay is removed. Other controls are
    // neither masked nor replaced with defaults. Both independent selector
    // bits must agree; a half-written pair is a conflict even if raw OFF fits.
    let levels = on
        .iter()
        .map(|&(f, b, _)| (f, b, cram.bit(f, b)))
        .collect::<Vec<_>>();
    result.selector_bits = levels.clone();
    if levels.iter().all(|&(_, _, v)| v) {
        result.status = "qualified_bypass".into();
        result.value = Some("ON".into());
        result.rule="complete two-bit CIB bypass selector; raw OFF clock constraints are a CLK0 experiment overlay".into();
        return Some(result);
    }
    if levels.iter().any(|&(_, _, v)| v) {
        result.status = "selector_conflict".into();
        result.value = None;
        result.rule = "inconsistent two-bit CIB output bypass selector".into();
        return Some(result);
    }
    if observed_clock.status != "resolved"
        || observed_clock.candidates.len() != 1
        || !["CLK0", "CLK1", "CLK2", "CLK3"].contains(&observed_clock.candidates[0].as_str())
    {
        result.status = "unresolved_output_clock".into();
        result.value = None;
        return Some(result);
    }
    result.clock_overrides = off
        .into_iter()
        .filter(|&(f, b, v)| cram.bit(f, b) != v)
        .map(|(frame, bit, base_value)| ModeOverride {
            selector: clock_name.clone(),
            frame,
            bit,
            base_value,
            observed_value: cram.bit(frame, bit),
        })
        .collect();
    result.status = "qualified_registered".into();
    result.value = Some("OFF".into());
    result.rule="both bypass bits clear; complete output-clock pattern overrides only verified OFF=CLK0 constraints; raw field retained".into();
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn resource() -> Resource {
        let mut clocks = Vec::new();
        let mut bypass = Vec::new();
        for (tile, offset) in [("A", 0), ("B", 4)] {
            let patterns = |a, b| {
                json!([
                    [0, offset, a],
                    [0, offset + 1, b],
                    [0, offset + 2, true],
                    [0, offset + 3, true]
                ])
            };
            clocks.push(json!({"tile":tile,"source":"clock","options":{"NONE":[],"CLK0":patterns(true,true),"CLK1":patterns(true,false),"CLK2":patterns(false,true),"CLK3":patterns(false,false)}}));
            bypass.push(json!({"tile":tile,"source":"bypass_CLK0_experiment","options":{"OFF":patterns(true,true),"ON":[]}}));
        }
        bypass.push(json!({"tile":"C","source":"bypass_selector","options":{"OFF":[],"ON":[[0,8,true],[0,9,true]]}}));
        serde_json::from_value(json!({"id":"mult","family":"MULT18","fields":[{"kind":"enum","name":"MULT18_0.MODE","fragments":[{"tile":"M","source":"mode","options":{"NONE":[],"MULT18X18D":[[0,10,true]]}}]},{"kind":"enum","name":"MULT18_0.REG_OUTPUT_CLK","fragments":clocks},{"kind":"enum","name":"MULT18_0.CIBOUT_BYP","fragments":bypass}]})).unwrap()
    }
    fn cram(word: u16) -> Cram {
        Cram {
            device: "LFE5U-85F".into(),
            frames: 1,
            bits_per_frame: 16,
            data: word.to_le_bytes().to_vec(),
            bram: BTreeMap::new(),
            origin: "test".into(),
        }
    }
    fn decode(r: &Resource, word: u16) -> DspOutputPath {
        let c = cram(word);
        let fields = r.fields.iter().map(|f| f.decode(&c)).collect::<Vec<_>>();
        output_path(r, &fields, &c).unwrap()
    }
    #[test]
    fn registered_output_overlay_accepts_every_complete_clock_and_keeps_raw_evidence() {
        for (index, word, changed) in [(0, 0xff, 0), (1, 0xdd, 2), (2, 0xee, 2), (3, 0xcc, 4)] {
            let r = decode(&resource(), word | (1 << 10));
            assert_eq!(r.value.as_deref(), Some("OFF"));
            assert_eq!(r.status, "qualified_registered");
            assert_eq!(r.output_clock_candidates, [format!("CLK{index}")]);
            assert_eq!(
                r.raw_status,
                if index == 0 { "resolved" } else { "unmatched" }
            );
            assert_eq!(r.clock_overrides.len(), changed);
            assert!(r
                .clock_overrides
                .iter()
                .all(|o| o.base_value && !o.observed_value));
            assert_eq!(r.selector_bits, [(0, 8, false), (0, 9, false)]);
        }
        let r = decode(&resource(), (1 << 8) | (1 << 9) | (1 << 10));
        assert_eq!(r.value.as_deref(), Some("ON"));
        assert_eq!(r.status, "qualified_bypass");
        assert!(r.clock_overrides.is_empty());
        // The right-hand MULT18_5 splits the selector pair over two tiles.
        let mut split = resource();
        let high = split.fields[2].fragments[2]
            .options
            .get_mut("ON")
            .unwrap()
            .pop()
            .unwrap();
        let mut fragment = split.fields[2].fragments[2].clone();
        fragment.tile = "D".into();
        fragment.options.insert("ON".into(), vec![high]);
        split.fields[2].fragments.push(fragment);
        let r = decode(&split, 0xdd | (1 << 10));
        assert_eq!(r.status, "qualified_registered");
        assert_eq!(r.value.as_deref(), Some("OFF"));
        let r = decode(&split, (1 << 8) | (1 << 9) | (1 << 10));
        assert_eq!(r.status, "qualified_bypass");
        assert_eq!(r.value.as_deref(), Some("ON"));
    }
    #[test]
    fn mixed_bypass_bits_inactive_mode_and_unresolved_clock_do_not_acquire_a_profile() {
        for word in [0xff | (1 << 8) | (1 << 10), 0xff | (1 << 9) | (1 << 10)] {
            let r = decode(&resource(), word);
            assert_eq!(r.raw_status, "resolved");
            assert_eq!(r.status, "selector_conflict");
            assert!(r.value.is_none());
        }
        let r = decode(&resource(), 0xdd);
        assert!(r.value.is_none());
        assert_eq!(r.status, "unmatched");
        let r = decode(&resource(), 1 << 10);
        assert!(r.value.is_none());
        assert_eq!(r.status, "unresolved_output_clock");
        let r = decode(&resource(), 0xcd | (1 << 10));
        assert!(r.value.is_none());
        assert_eq!(r.status, "unresolved_output_clock");
    }
    #[test]
    fn incomplete_domains_and_unknown_nonclock_constraints_remain_unmatched() {
        let mut variants = Vec::new();
        let mut r = resource();
        r.fields[1].fragments.pop();
        variants.push(r);
        let mut r = resource();
        r.fields[1].fragments[0].options.remove("CLK3");
        variants.push(r);
        let mut r = resource();
        r.fields[2].fragments[0]
            .options
            .get_mut("OFF")
            .unwrap()
            .push((0, 11, true));
        variants.push(r);
        let mut r = resource();
        r.fields[2].fragments[0].options.remove("ON");
        variants.push(r);
        let mut r = resource();
        r.fields[2].fragments[2]
            .options
            .get_mut("ON")
            .unwrap()
            .push((0, 11, true));
        variants.push(r);
        for resource in variants {
            let r = decode(&resource, 0xdd | (1 << 10));
            assert!(r.value.is_none());
            assert!(r.clock_overrides.is_empty());
            assert!(!r.status.starts_with("qualified"));
        }
    }
}
