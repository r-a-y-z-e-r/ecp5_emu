//! The fabric view (src/fabric.rs): every wire of the die is drawn
//! connected and square. In every tile no segment end and no link end
//! touches nothing, no piece is diagonal, no wire runs through a site (a LUT,
//! a flip-flop, an EBR, a multiplier, a pad …), every pin ends on the edge of
//! its site, every dot sits on the wires it joins, no two wires run over
//! each other, every site has wires, and no group of wires reaches nothing.
//! No two parallel pieces side by side, links and the DSP blocks' wires
//! included, come closer than they do in the plain logic tiles. Arcs that
//! would join a wire to nothing else are left out and counted.
use ecp5_emu::{
    database::Database,
    fabric::{self, Check, Fabric},
    model::DeviceModel,
};
use std::path::PathBuf;

fn report(name: &str, f: &Fabric, c: &Check) {
    println!(
        "{name}: {} positions, {} ends; lonely {:?}, diagonal {}, through sites {}, pins off site {}, unplaced pins {}, loose link ends {}, dots off their wires {}, overlaps {}, bare sites {}, islands {}, pairs too close {} (closest {:.5})",
        c.positions,
        c.ends,
        c.lonely,
        c.diagonal,
        c.through_sites,
        c.pins_off_site,
        c.unplaced,
        c.links,
        c.dots_off_wires,
        c.overlaps,
        c.bare_sites,
        c.islands,
        c.tight,
        c.closest
    );
    let mut by_types: Vec<_> = c.by_types.iter().collect();
    by_types.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (kinds, n) in by_types.iter().take(25) {
        println!("  {n:8}  {kinds}");
    }
    println!("  {} examples", c.examples.len());
    for e in c.examples.iter().filter(|e| !e.contains("DSP")).take(40) {
        println!("  e.g. {e}");
    }
    let _ = f;
}

#[test]
fn every_wire_of_the_fabric_is_connected_and_square() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let model = DeviceModel::load(
        &Database::discover(None).unwrap(),
        &root.join("docs/inventory/LFE5U-85F"),
    )
    .unwrap();
    let f = fabric::build(&model);
    let (h, v) = f.lanes.counts();
    let left_out: usize = (0..f.rows)
        .flat_map(|r| (0..f.cols).map(move |c| (r, c)))
        .map(|(r, c)| f.template(r, c).left_out)
        .sum();
    println!(
        "{} arcs, {} templates, {} links, {} arcs to a wire the neighbour lacks, {left_out} arcs left out that join a wire to nothing else; lanes {h} H {v} V",
        f.arcs,
        f.templates.len(),
        f.links.len(),
        f.broken_links,
    );
    // Arcs left out per position, by tile types.
    let mut left: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
    for r in 0..f.rows {
        for c in 0..f.cols {
            let t = f.template(r, c);
            let e = left.entry(f.types_at(r, c).join("+")).or_default();
            e.0 += 1;
            e.1 += t.left_out;
            e.2 = e.2.max(t.left_out);
        }
    }
    let mut left: Vec<_> = left.into_iter().collect();
    left.sort_by_key(|(_, (_, n, _))| std::cmp::Reverse(*n));
    for (kinds, (positions, n, most)) in left.iter().take(8) {
        println!("  left out {n:7} at {positions:5} positions (at most {most}): {kinds}");
    }
    let logic = |k: &[String]| k.iter().any(|k| k == "PLC2");
    // The spacing of the plain logic tiles is the one every tile keeps.
    let closest = f.closest();
    let mut min_gap = f32::MAX;
    for r in 0..f.rows {
        for c in 0..f.cols {
            if f.types_at(r, c) == ["PLC2"] {
                min_gap = min_gap.min(
                    closest[f.template_at[r as usize * f.cols as usize + c as usize] as usize],
                );
            }
        }
    }
    println!("closest parallel pieces in plain logic tiles: {min_gap:.5}");
    // The rows of the DSP blocks, drawn whole under their interconnect.
    let dsp_rows: std::collections::BTreeSet<u16> = (0..f.rows)
        .filter(|&r| (0..f.cols).any(|c| f.types_at(r, c).iter().any(|k| k.starts_with("MIB_DSP"))))
        .collect();
    let in_logic = f.check(|_, _, k| logic(k), min_gap);
    report("logic tiles", &f, &in_logic);
    let rest = f.check(|r, _, k| !logic(k) && !dsp_rows.contains(&r), min_gap);
    report("other tiles", &f, &rest);
    let dsp = f.check(|r, _, k| !logic(k) && dsp_rows.contains(&r), min_gap);
    report(&format!("DSP rows {dsp_rows:?}"), &f, &dsp);
    for m in &f.macros {
        println!(
            "  DSP row {}: {:.3} high, {} blocks, {} parts, {} wires from the CIB, {} pieces between ports",
            m.row,
            f.row_height(m.row),
            m.frames.len(),
            m.parts.len(),
            m.wires.len(),
            m.inner.len()
        );
    }
    println!(
        "  DSP fixed connections within a part {}, leading out of a row's blocks {}",
        f.dsp_inside, f.dsp_lost
    );
    // Arcs to a neighbour's wire that no tile at that position has, by the
    // wire's family (its name without digits).
    let mut families: std::collections::BTreeMap<String, usize> = Default::default();
    for ((_, wire, there), n) in &f.broken {
        let name = wire.split_once(' ').map_or(wire.as_str(), |w| w.1);
        let family: String = name.chars().filter(|c| !c.is_ascii_digit()).collect();
        *families.entry(format!("{family} at {there}")).or_default() += n;
    }
    let mut families: Vec<_> = families.into_iter().collect();
    families.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (family, n) in families.iter().take(12) {
        println!("  missing {n:5}  {family}");
    }
    assert!(in_logic.positions > 3000);
    assert_eq!(in_logic.total(), 0);
    assert!(rest.positions > 1000);
    assert_eq!(rest.total(), 0);
    assert_eq!(dsp.total(), 0);
}

#[test]
fn fabric_cache_round_trips_and_rejects_corruption() {
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let arch = root.join("docs/inventory/LFE5U-85F");
    let model = DeviceModel::load(&Database::discover(None).unwrap(), &arch).unwrap();

    let dir =
        TempDir(std::env::temp_dir().join(format!("ecp5_emu_fabric_test_{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = fabric::cache_path(&arch, &dir.0).unwrap();

    let t0 = std::time::Instant::now();
    let original = fabric::build(&model);
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;

    let t1 = std::time::Instant::now();
    let encoded = fabric::encode_cache(&original).unwrap();
    std::fs::write(&path, &encoded).unwrap();
    let write_ms = t1.elapsed().as_secs_f64() * 1e3;

    let t2 = std::time::Instant::now();
    let bytes = std::fs::read(&path).unwrap();
    let decoded = fabric::decode_cache(&bytes).expect("valid cache file should decode");
    let read_ms = t2.elapsed().as_secs_f64() * 1e3;

    let t3 = std::time::Instant::now();
    let via_cached = fabric::cached_in(&model, &arch, &dir.0);
    let cached_hit_ms = t3.elapsed().as_secs_f64() * 1e3;

    println!(
        "fabric cache: size={} bytes ({:.2} MiB), build={build_ms:.1} ms, write={write_ms:.1} ms, read={read_ms:.1} ms, cached_hit={cached_hit_ms:.1} ms",
        encoded.len(),
        encoded.len() as f64 / (1024.0 * 1024.0),
    );

    assert!(decoded == original, "decoded fabric differs from original");
    assert!(
        via_cached == original,
        "cached_in fabric differs from original"
    );
    assert_eq!(fabric::encode_cache(&decoded).unwrap(), encoded);

    // Corrupted payload must be rejected by decode_cache and rebuilt by cached_in.
    let mut corrupted = encoded.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xff;
    assert!(fabric::decode_cache(&corrupted).is_none());
    std::fs::write(&path, &corrupted).unwrap();
    let rebuilt = fabric::cached_in(&model, &arch, &dir.0);
    assert!(
        rebuilt == original,
        "rebuilt fabric after corruption differs"
    );
    let disk = std::fs::read(&path).unwrap();
    assert!(fabric::decode_cache(&disk).is_some_and(|f| f == original));

    // Truncated file must be rejected by decode_cache and rebuilt by cached_in.
    let truncated = &encoded[..encoded.len() / 2];
    assert!(fabric::decode_cache(truncated).is_none());
    std::fs::write(&path, truncated).unwrap();
    let rebuilt = fabric::cached_in(&model, &arch, &dir.0);
    assert!(
        rebuilt == original,
        "rebuilt fabric after truncation differs"
    );
    let disk = std::fs::read(&path).unwrap();
    assert!(fabric::decode_cache(&disk).is_some_and(|f| f == original));
}
