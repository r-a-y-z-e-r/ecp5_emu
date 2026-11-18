//! These tests use the user's installed Trellis database, without copying it.
use ecp5_emu::{chip::Chip, config::Config, database::Database, logic::Logic};

fn counter_wires() -> Vec<String> {
    let mut wires = vec!["R37C2:Q0".to_owned()];
    wires.extend((3..8).map(|i| format!("R38C2:Q{i}")));
    for col in 3..=5 {
        wires.extend((0..8).map(|i| format!("R38C{col}:Q{i}")));
    }
    wires
}

#[test]
#[ignore = "requires a local Trellis database; run cargo test --release --test whole_chip -- --ignored"]
fn routed_counter_counts_through_tile_boundaries() {
    let root = Database::discover(None).unwrap();
    let config = Config::parse(include_str!("../test.config")).unwrap();
    let db = Database::load(&root, &config.device).unwrap();
    let mut chip = Chip::compile(&db, &config, "CABGA381").unwrap();
    assert_eq!(chip.simulation.register_count(), 83640);
    let probes: Vec<_> = counter_wires()
        .iter()
        .map(|p| chip.probe_net(p).unwrap())
        .collect();
    chip.drive_pins(&[("G2".into(), Logic::Zero)]).unwrap();
    for expected in 0..=65537_u32 {
        let actual = probes.iter().enumerate().fold(0_u32, |acc, (i, &n)| {
            let bit = chip
                .simulation
                .value(n)
                .bit()
                .unwrap_or_else(|| panic!("unknown counter bit {i} at cycle {expected}"));
            acc | ((bit as u32) << i)
        });
        assert_eq!(actual, expected, "counter at cycle {expected}");
        chip.drive_pins(&[("G2".into(), Logic::One)]).unwrap();
        chip.drive_pins(&[("G2".into(), Logic::Zero)]).unwrap();
    }
}

#[test]
#[ignore = "requires ECP5_TEST_BIT pointing to the existing blink/top.bit and a local Trellis database"]
fn real_bitstream_matches_text_config() {
    let root = Database::discover(None).unwrap();
    let file = std::env::var_os("ECP5_TEST_BIT").expect("set ECP5_TEST_BIT to blink/top.bit");
    let config = Config::load(std::path::Path::new(&file), &root).unwrap();
    let db = Database::load(&root, &config.device).unwrap();
    let mut chip = Chip::compile(&db, &config, "CABGA381").unwrap();
    let probes: Vec<_> = counter_wires()
        .iter()
        .map(|p| chip.probe_net(p).unwrap())
        .collect();
    chip.drive_pins(&[("G2".into(), Logic::Zero)]).unwrap();
    for expected in 0..256_u32 {
        for (i, &net) in probes.iter().enumerate() {
            assert_eq!(
                chip.simulation.value(net),
                Logic::from_bool(expected & (1 << i) != 0),
                "bit {i} at cycle {expected}"
            );
        }
        chip.drive_pins(&[("G2".into(), Logic::One)]).unwrap();
        chip.drive_pins(&[("G2".into(), Logic::Zero)]).unwrap();
    }
}
