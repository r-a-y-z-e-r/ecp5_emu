//! The OSCG divider table in the planner must follow from the board sweep
//! (docs/inventory/LFE5U-85F/hc06_osc/sweep.json) and cover every OSC.DIV
//! value of the Trellis database exactly once.
use ecp5_emu::compile::OSCILLATOR_DIVIDERS;
use serde_json::Value;

#[test]
fn divider_table_follows_the_board_sweep() {
    let sweep: Value = serde_json::from_str(include_str!(
        "../docs/inventory/LFE5U-85F/hc06_osc/sweep.json"
    ))
    .unwrap();
    // DIV=2 is exact on the board; its count at the start and the end of the
    // sweep gives the base, which drifts slightly in between.
    let before = sweep["reference_before"]["mean"].as_f64().unwrap() * 2.0;
    let after = sweep["reference_after"]["mean"].as_f64().unwrap() * 2.0;
    let patterns = sweep["patterns"].as_array().unwrap();
    assert_eq!(patterns.len(), OSCILLATOR_DIVIDERS.len());
    let n = patterns.len() as f64;
    let mut covered = Vec::new();
    for (i, (pattern, &(values, divider))) in patterns.iter().zip(OSCILLATOR_DIVIDERS).enumerate() {
        let measured: Vec<u32> = pattern["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        assert_eq!(measured, values);
        let base = before + (after - before) * (i as f64 + 1.0) / (n + 1.0);
        let ratio = base / pattern["mean"].as_f64().unwrap();
        // Neighbouring dividers differ by at least 0.8%; the board wanders by 0.1%.
        assert!(
            (ratio - divider as f64).abs() / (divider as f64) < 2e-3,
            "{values:?}: measured {ratio:.3}, table {divider}"
        );
        covered.extend_from_slice(values);
    }
    covered.sort_unstable();
    assert_eq!(covered, (2..=127).collect::<Vec<u32>>());
}
