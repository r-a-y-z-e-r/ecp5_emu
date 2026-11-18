mod common;
use common::{write, Fixture};
use ecp5_emu::{
    database::{ArcKind, Database},
    model::{
        topology::{Endpoint, Location, Wire},
        Binding, DeviceModel,
    },
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

#[test]
fn retains_directed_alternatives_unresolved_sites_and_dedicated_pins() {
    let f = Fixture::new();
    let model = f.load();
    let plc = model.tile_id("R1C1:PLC2").unwrap();
    let cib = model.tile_id("CIB_R1C1:CIB").unwrap();
    assert_ne!(plc, cib);
    let arcs = model
        .connections(plc)
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(arcs.len(), 4); // both mux choices plus two directed fixed descriptions
    assert_eq!(arcs[0].definition.source, "A0");
    assert_eq!(arcs[0].definition.sink, "F0");
    assert_eq!(arcs[0].definition.kind, ArcKind::Mux);
    assert_eq!(
        arcs[0]
            .cram_constraints()
            .map(|b| (b.frame, b.bit, b.value))
            .collect::<Vec<_>>(),
        [(3, 4, true), (2, 3, false)]
    );
    assert_eq!(
        model.connection(arcs[1].id).unwrap().definition.source,
        "G_HPBX0000"
    );
    assert_eq!(arcs[3].definition.kind, ArcKind::Fixed); // no automatic ECLKSYNC bypass execution
    let excluded = model
        .connections(cib)
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(excluded[1].source, Endpoint::OtherDensity);
    assert_eq!(excluded[2].source, Endpoint::OutsideDevice);
    let unknown = model
        .site_references()
        .iter()
        .find(|s| s.binding == Binding::Unresolved)
        .unwrap();
    assert_eq!(unknown.name, "CIBTEST");
    assert_eq!(unknown.vendor_location.row, 30);
    assert!(unknown.resource.is_none());
    let pin = model.package_pin("cabga381", "g1").unwrap();
    assert!(pin.pio.is_some());
    assert!(model.package_pin("CABGA381", "T1").unwrap().pio.is_none());
    let root = model.resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let node = model.nodes(root).next().unwrap();
    assert_eq!(node.key, "LFE5U-85F/R1C1/SLICE_A/K0");
    assert_eq!(model.resource(node.owner).key, "LFE5U-85F/R1C1/SLICE_A");
    assert_eq!(model.node_ports(node.id)[0].wire.name, "F0_SLICE");
}

#[test]
fn clock_segment_relationship_does_not_replace_raw_identity() {
    let f = Fixture::new();
    let model = f.load();
    let loc = Location { row: 1, col: 1 };
    let Endpoint::Wire(raw) = model.wire(loc, "G_HPBX0000").unwrap() else {
        panic!()
    };
    assert_eq!(raw.location, loc);
    assert_eq!(raw.name, "G_HPBX0000");
    let segment = model.clock_segment(&raw).unwrap().unwrap();
    assert_eq!(
        segment,
        Wire {
            location: Location { row: 1, col: 1 },
            name: "R_HPBX0000".into()
        }
    );
    assert_ne!(raw, segment);
    assert_eq!(
        model.wire(loc, "G_JDCC").unwrap(),
        Endpoint::Wire(Wire {
            location: Location { row: 0, col: 0 },
            name: "G_JDCC".into()
        })
    );
    assert_eq!(
        model.wire(loc, "N1W1_A").unwrap(),
        Endpoint::Wire(Wire {
            location: Location { row: 0, col: 0 },
            name: "A".into()
        })
    );
    assert!(model.wire(loc, "S2147483647_A").is_err());
}

#[test]
fn rejects_stale_data_before_accepting_the_architecture() {
    let f = Fixture::new();
    write(&f.evidence.join("resource_instances.csv"), "broken");
    assert!(f.error().contains("architecture source changed"));
    let f = Fixture::new();
    write(
        &f.db.join("ECP5/tiledata/PLC2/bits.db"),
        ".fixed_conn OTHER A\n",
    );
    assert!(f.error().contains("architecture source changed"));
}

#[test]
fn rejects_missing_sites_wrong_vendor_coordinates_and_parent_cycles() {
    let f = Fixture::new();
    f.replace(
        "site_bindings.csv",
        "CIB_R1C1:CIB,0,CIBTEST,CIBTEST,30,20,,unresolved,unresolved\n",
        "",
    );
    assert!(f.error().contains("not all database sites"));
    let f = Fixture::new();
    f.replace(
        "site_bindings.csv",
        "SLICEA,SLICE,-2,20",
        "SLICEA,SLICE,1,1",
    );
    assert!(f.error().contains("differs from database"));
    let f = Fixture::new();
    f.replace(
        "resource_instances.csv",
        "PLC2,R1C1:PLC2,,False",
        "PLC2,R1C1:PLC2,LFE5U-85F/R1C1/SLICE_A,False",
    );
    assert!(f.error().contains("parent cycle"));
}

#[test]
fn rejects_source_paths_outside_the_pinned_root() {
    let f = Fixture::new();
    let p = f.evidence.join("model_manifest.json");
    let mut m: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    m["files"][0]["path"] = json!("../outside.json");
    write(&p, m.to_string());
    assert!(f.error().contains("must be relative"));
}

#[test]
#[ignore = "requires the local Trellis database and collected architecture evidence"]
fn full_85f_architecture_matches_inventory_and_native_clock_segments() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inventory = root.join("docs/inventory/LFE5U-85F");
    let model = DeviceModel::load(&Database::discover(None).unwrap(), &inventory).unwrap();
    let s = model.summary();
    assert_eq!(
        (s.tiles, s.tile_types, s.resources, s.site_references),
        (14231, 148, 53690, 54721)
    );
    assert_eq!(s.resource_families["PLC2"], 10455);
    assert_eq!(s.resource_families["SLICE"], 41820);
    assert_eq!(s.site_bindings["Unresolved"], 1496);
    assert_eq!(s.site_bindings["InstanceCandidate"], 120);
    // 3 246 243 before the 4 × 22 EHXPLLL ports from Bels.cpp:add_pll.
    assert_eq!(s.resource_ports + s.plc_internal_ports, 3246331);
    assert_eq!(s.plc_internal_node_views, 177735);
    assert_eq!(s.route_templates, 69287);
    assert_eq!(s.bonded_pio_references, 947);
    assert_eq!(s.package_balls.values().sum::<usize>(), 1976);
    let mut rows =
        csv::Reader::from_path(inventory.join("clock_system_research/implicit_aliases.csv"))
            .unwrap();
    let mut count = 0;
    for row in rows.records() {
        let r = row.unwrap();
        let loc = Location {
            row: r[0].parse().unwrap(),
            col: r[1].parse().unwrap(),
        };
        let Endpoint::Wire(raw) = model.wire(loc, &r[2]).unwrap() else {
            panic!("missing clock wire")
        };
        let expected = Wire {
            location: Location {
                row: r[3].parse().unwrap(),
                col: r[4].parse().unwrap(),
            },
            name: r[5].into(),
        };
        assert_eq!(model.clock_segment(&raw).unwrap(), Some(expected));
        count += 1;
    }
    assert_eq!(count, 197408);
}
