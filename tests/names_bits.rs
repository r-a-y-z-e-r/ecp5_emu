//! Synthetic routed JSON exercises HDL indices without FPGA tools.
mod common;
use ecp5_emu::{
    compile::Reference,
    names::Names,
    sources::{Collected, Files, NameLookup},
};
use serde_json::{json, Value};

// Expected HDL indices in JSON array order, independently of the implementation.
const CASES: [(&str, usize, u32, bool, &[u32]); 5] = [
    ("scalar", 1, 0, false, &[]),
    ("single", 1, 1, false, &[1]),
    ("down", 4, 4, false, &[4, 5, 6, 7]),
    ("up", 4, 0, true, &[3, 2, 1, 0]),
    ("plain", 4, 0, false, &[0, 1, 2, 3]),
];

fn expected(name: &str, indices: &[u32], i: usize) -> String {
    if indices.is_empty() {
        name.into()
    } else {
        format!("{name}[{}]", indices[i])
    }
}

fn design(name: &str, len: usize, offset: u32, upto: bool, indices: &[u32]) -> Value {
    let bits: Vec<_> = (10..10 + len).collect();
    let mut netnames = serde_json::Map::new();
    netnames.insert(
        name.into(),
        json!({"bits": bits, "offset": offset, "upto": u8::from(upto),
            "attributes": {"src": "top.v:1"}}),
    );
    // nextpnr supplies per-bit routing, possibly under another alias.
    for (i, bit) in bits.iter().enumerate() {
        netnames.insert(
            format!("route_{i}"),
            json!({"bits": [bit], "attributes": {"ROUTING": format!("X1/Y1/W{i};;1")}}),
        );
        // Actual nextpnr output has canonical per-bit names too. Give them
        // no routing to ensure JSON key ordering cannot erase a good alias.
        if !indices.is_empty() {
            netnames.insert(
                expected(name, indices, i),
                json!({"bits": [bit], "attributes": {"ROUTING": " ", "src": "top.v:1"}}),
            );
        }
    }
    json!({"modules": {"top": {"netnames": netnames, "ports": {}, "cells": {}}}})
}

#[test]
fn netnames_expand_scalars_and_ranges_in_hdl_index_order() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    for (name, len, offset, upto, indices) in CASES {
        let json = design(name, len, offset, upto, indices);
        for flattened in [false, true] {
            let mut json = json.clone();
            if flattened && !indices.is_empty() {
                json["modules"]["top"]["netnames"]
                    .as_object_mut()
                    .unwrap()
                    .remove(name);
            }
            let names = Names::from_json(&json, &model, "CABGA381").unwrap();
            let expanded = names.expand(name).unwrap();
            let mut wanted: Vec<_> = (0..len).map(|i| expected(name, indices, i)).collect();
            wanted.sort();
            assert_eq!(
                expanded.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                wanted.iter().collect::<Vec<_>>()
            );
            for i in 0..len {
                let selected = names.expand(&expected(name, indices, i)).unwrap();
                let Reference::Wire(wire) = &selected[0].1 else {
                    panic!("wire")
                };
                assert_eq!(wire.name, format!("W{i}"), "{name} bit {i}");
            }
            assert_eq!(expanded.len(), len);
        }
    }
}

#[test]
fn ports_override_pad_netnames_using_hdl_indices() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    for (name, len, offset, upto, indices) in CASES {
        // One placed pad at a time makes every array position observable
        // with the synthetic model's single package ball.
        for placed in 0..len {
            let mut json = design(name, len, offset, upto, indices);
            let top = &mut json["modules"]["top"];
            top["ports"][name] = json!({"bits": (10..10+len).collect::<Vec<_>>(),
                "offset": offset, "upto": u8::from(upto), "direction": "input"});
            top["cells"]["pad"] = json!({"type": "TRELLIS_IO",
                "attributes": {"NEXTPNR_BEL": "X1/Y1/PIOA"},
                "connections": {"B": [10+placed]}});
            let names = Names::from_json(&json, &model, "CABGA381").unwrap();
            assert_eq!(names.expand(name).unwrap().len(), len);
            for i in 0..len {
                let selected = names.expand(&expected(name, indices, i)).unwrap();
                if i == placed {
                    assert_eq!(selected[0].1, Reference::Pin("G1".into()));
                    // Alias netnames for the same pad bit agree with ports.
                    assert_eq!(
                        names.expand(&format!("route_{i}")).unwrap()[0].1,
                        selected[0].1
                    );
                } else {
                    assert!(matches!(selected[0].1, Reference::Wire(_)));
                }
            }
            // Independently exercise the port loop: no netname can supply
            // the canonical name or hide an incorrectly indexed port.
            json["modules"]["top"]["netnames"] = json!({});
            let ports = Names::from_json(&json, &model, "CABGA381").unwrap();
            assert_eq!(
                ports.expand(name).unwrap(),
                vec![(expected(name, indices, placed), Reference::Pin("G1".into()))]
            );
            assert_eq!(ports.len(), 1);
        }
    }
}

#[test]
fn cross_probing_and_names_identify_the_same_bit() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    let files = Files::project(".".into(), ["top.v".into()]);
    for (name, len, offset, upto, indices) in CASES {
        let mut json = design(name, len, offset, upto, indices);
        // Exercise aggregate metadata alone, including a one-bit vector.
        json["modules"]["top"]["netnames"]
            .as_object_mut()
            .unwrap()
            .retain(|n, _| !n.ends_with(']'));
        let names = Names::from_json(&json, &model, "CABGA381").unwrap();
        let index = Collected::from_top(&json["modules"]["top"], &files).build(|n| {
            // The live window resolves exact names through overlay_nets,
            // rather than expanding a whole bus before cross-probing.
            names
                .resolved()
                .filter(|(name, _)| *name == n)
                .filter_map(|(_, r)| match r {
                    Reference::Wire(wire) => wire.name.strip_prefix('W')?.parse::<u32>().ok(),
                    _ => None,
                })
                .collect()
        });
        let NameLookup::Resolved(rows) = index.name_only("top.v", name) else {
            panic!("{name}")
        };
        assert_eq!(rows[0].bits.len(), len);
        for i in 0..len {
            let canonical = expected(name, indices, i);
            let bit = rows[0].bits.iter().find(|b| b.name == canonical).unwrap();
            assert_eq!(bit.targets, [i as u32], "{canonical}");
            assert_eq!(bit.index, indices.get(i).copied());
            let NameLookup::Resolved(selected) = index.name_only("top.v", &canonical) else {
                panic!("{canonical}")
            };
            assert_eq!(selected[0].bits.len(), 1);
        }
    }
}

#[test]
fn constants_and_ambiguous_bare_names_keep_their_meaning() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    let json = json!({"modules": {"top": {"netnames": {
        "btn": {"bits": [2], "attributes": {"ROUTING": "X1/Y1/SCALAR;;1"}},
        "btn[1]": {"bits": [3], "attributes": {"ROUTING": "X1/Y1/VECTOR;;1"}},
        "constant": {"bits": ["0"], "offset": 1}
    }}}});
    let names = Names::from_json(&json, &model, "CABGA381").unwrap();
    assert_eq!(names.expand("btn").unwrap()[0].0, "btn");
    assert_ne!(
        names.expand("btn").unwrap()[0].1,
        names.expand("btn[1]").unwrap()[0].1
    );
    assert!(names
        .expand("constant[1]")
        .unwrap_err()
        .to_string()
        .contains("no routed source"));
}

#[test]
fn ports_observe_data_direction_and_keep_pin_stimulus_references() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    for (direction, data) in [("output", "I"), ("input", "O"), ("output", "IOLDO")] {
        let mut json = json!({"modules": {"top": {
            "ports": {"port": {"bits": [10], "direction": direction}},
            "netnames": {
                "port": {"bits": [10], "attributes": {"src": "top.v:1"}},
                "pad_alias": {"bits": [10]},
                "fabric_i": {"bits": [20], "attributes": {"ROUTING": "X1/Y1/DATA_I;;1"}},
                "fabric_o": {"bits": [30], "attributes": {"ROUTING": "X1/Y1/DATA_O;;1"}},
                "control_t": {"bits": [40], "attributes": {"ROUTING": "X1/Y1/CONTROL_T;;1"}},
                "iologic": {"bits": [50], "attributes": {"ROUTING": "X1/Y1/DATA_IOLDO;;1"}}
            },
            "cells": {"pad": {"type": "TRELLIS_IO",
                "attributes": {"NEXTPNR_BEL": "X1/Y1/PIOA"},
                "connections": {"B": [10], "I": [20], "O": [30], "T": [40]}}}
        }}});
        if data == "IOLDO" {
            json["modules"]["top"]["cells"]["pad"]["parameters"]["DATAMUX_ODDR"] = json!("IOLDO");
            json["modules"]["top"]["cells"]["pad"]["connections"]["IOLDO"] = json!([50]);
        }
        let names = Names::from_json(&json, &model, "CABGA381").unwrap();
        for alias in ["port", "pad_alias"] {
            assert_eq!(
                names.expand(alias).unwrap()[0].1,
                Reference::Pin("G1".into()),
                "stimuli and explicit pin probes retain their semantics"
            );
            let (_, physical) = names.resolved().find(|(name, _)| *name == alias).unwrap();
            assert_eq!(*physical, Reference::Pin("G1".into()));
            let (_, reference) = names
                .observations()
                .find(|(name, _)| *name == alias)
                .unwrap();
            let Reference::Wire(wire) = reference else {
                panic!("fabric data reference")
            };
            assert_eq!(wire.name, format!("DATA_{data}"));
        }
    }
}

#[test]
fn unavailable_observations_keep_physical_pad_references_and_aliases() {
    let fixture = common::Fixture::new();
    let model = fixture.load();
    let json = json!({"modules": {"top": {
        "ports": {"port": {"bits": [10], "direction": "output"}},
        "netnames": {
            "port": {"bits": [10]},
            "pad_alias": {"bits": [10]},
            "control_t": {"bits": [40], "attributes": {"ROUTING": "X1/Y1/CONTROL_T;;1"}}
        },
        "cells": {"pad": {"type": "TRELLIS_IO",
            "attributes": {"NEXTPNR_BEL": "X1/Y1/PIOA"},
            "connections": {"B": [10], "I": [], "T": [40]}}}
    }}});
    let names = Names::from_json(&json, &model, "CABGA381").unwrap();
    for alias in ["port", "pad_alias"] {
        let (_, physical) = names.resolved().find(|(name, _)| *name == alias).unwrap();
        assert_eq!(*physical, Reference::Pin("G1".into()));
        assert!(names.observations().all(|(name, _)| name != alias));
    }
}

#[test]
fn a_port_with_no_data_route_never_uses_a_routed_control_as_its_value() {
    let fixture = common::Fixture::with_compiler();
    let model = fixture.load();
    let json = json!({"modules": {"top": {
        "ports": {"port": {"bits": [10], "direction": "output"}},
        "netnames": {
            "port": {"bits": [10]},
            "control_t": {"bits": [40], "attributes": {"ROUTING": "X1/Y1/CONTROL_T;;1"}}
        },
        "cells": {"pad": {"type": "TRELLIS_IO",
            "attributes": {"NEXTPNR_BEL": "X1/Y1/PIOA"},
            "connections": {"B": [10], "I": ["1"], "T": [40]}}}
    }}});
    let names = Names::from_json(&json, &model, "CABGA381").unwrap();
    let (_, observation) = names
        .observations()
        .find(|(name, _)| *name == "port")
        .unwrap();
    let Reference::Wire(wire) = observation else {
        panic!("exact PIO data endpoint")
    };
    let pio = model.package_pin("CABGA381", "G1").unwrap().pio.unwrap();
    let data = model
        .resource_ports(pio)
        .into_iter()
        .find(|p| p.name == "I")
        .unwrap();
    assert_eq!(*wire, data.wire);
    assert_ne!(wire.name, "CONTROL_T");
}

#[test]
fn iologic_port_observations_use_the_pio_data_endpoint_before_the_routed_source() {
    let fixture = common::Fixture::with_compiler();
    let ports = fixture.evidence.join("iologic_research/resource_ports.csv");
    let mut text = std::fs::read_to_string(&ports).unwrap();
    text.push_str("LFE5U-85F/R1C1/PIOA,IOLDO,in,1,1,IOLDOA_PIO\n");
    std::fs::write(ports, text).unwrap();
    fixture.refresh_database();
    let model = fixture.load();
    let json = json!({"modules": {"top": {
        "ports": {"q": {"bits": [10], "direction": "output"}},
        "netnames": {
            "q": {"bits": [10]},
            "q_alias": {"bits": [10]},
            "r_Q": {"bits": [20], "attributes": {"ROUTING": "X1/Y1/REGISTER_Q;;1"}}
        },
        "cells": {"q$tr_io": {"type": "TRELLIS_IO",
            "parameters": {"DATAMUX_OREG": "IOLDO"},
            "attributes": {"NEXTPNR_BEL": "X1/Y1/PIOA"},
            "connections": {"B": [10], "I": [], "IOLDO": [20]}}}
    }}});
    let names = Names::from_json(&json, &model, "CABGA381").unwrap();
    for alias in ["q", "q_alias"] {
        let (_, reference) = names
            .observations()
            .find(|(name, _)| *name == alias)
            .unwrap();
        let Reference::Wire(wire) = reference else {
            panic!("selected data endpoint")
        };
        assert_eq!(wire.name, "IOLDOA_PIO");
        assert_eq!(
            names.expand(alias).unwrap()[0].1,
            Reference::Pin("G1".into())
        );
    }
}
