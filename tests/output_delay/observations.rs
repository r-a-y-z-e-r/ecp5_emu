//! A real nextpnr OREG route through the window's published-value lookup.
use super::*;
use ecp5_emu::{fabric, fabric_overlay, fast::Reader, layout, live, names::Names};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn registered_output_port_observes_q_at_edges_and_between_edges() {
    let fixture = Fixture::build("names-oreg", X12, "A2", false);
    let json: Value =
        serde_json::from_slice(&std::fs::read(fixture.dir.join("top_routed.json")).unwrap())
            .unwrap();
    let top = &json["modules"]["top"];
    let cells = top["cells"].as_object().unwrap();
    let pad = &cells["q$tr_io"];
    assert_eq!(pad["type"], "TRELLIS_IO");
    assert_eq!(pad["parameters"]["DATAMUX_OREG"], "IOLDO");
    assert_eq!(pad["connections"]["I"], serde_json::json!([]));
    let register = cells
        .values()
        .find(|cell| {
            cell["type"] == "IOLOGIC" && cell["connections"]["IOLDO"] == pad["connections"]["IOLDO"]
        })
        .unwrap();
    assert_eq!(register["parameters"]["MODE"], "IREG_OREG");
    println!(
        "q TRELLIS_IO: DATAMUX_OREG={}, I={}, IOLDO={}; register {}",
        pad["parameters"]["DATAMUX_OREG"],
        pad["connections"]["I"],
        pad["connections"]["IOLDO"],
        register["attributes"]["NEXTPNR_BEL"]
    );

    let mut compiled = fixture.compile(false).unwrap();
    let model = Arc::clone(compiled.kernel.device().model());
    let pio = model
        .package_pin("CABGA381", fixture.output)
        .unwrap()
        .pio
        .unwrap();
    let native_name = register["attributes"]["NEXTPNR_BEL"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let (id, _) = model
        .resources()
        .find(|(_, resource)| {
            resource.location == model.resource(pio).location
                && resource.native_name == native_name
                && resource.family == "IOLOGIC"
        })
        .unwrap();
    let register_wire = model
        .resource_ports(id)
        .into_iter()
        .find(|port| port.name == "IOLDO")
        .unwrap()
        .wire;
    let register_q = compiled.kernel.signal(&register_wire).unwrap();
    let pad_q = compiled.probes[&format!("pin:{}", fixture.output)];
    let names = Names::from_json(&json, &model, "CABGA381").unwrap();
    let fabric = fabric::cached(&model, &live::architecture());
    let report = &compiled.report;
    let overlay = fabric_overlay::build(
        &fabric,
        &model,
        &report.routes,
        &report.constants,
        &layout::logic_usage(&model, report),
        &report.fields,
    );

    // Static::net_names uses these same observation references and overlay nets.
    // No probe of q's observation wire is added to make it readable.
    let mut net_names: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (name, reference) in names.observations() {
        if let Reference::Wire(wire) = reference {
            if let Some(value) = overlay.value_of(&model, wire) {
                net_names
                    .entry(overlay.nets[value as usize])
                    .or_default()
                    .push(name);
            }
        }
    }
    let nets: Vec<_> = net_names
        .iter()
        .filter_map(|(net, names)| names.contains(&"q").then_some(*net))
        .collect();
    assert_eq!(nets.len(), 1, "exactly one data net for q");
    assert_eq!(names.overlay_nets("q", &model, "CABGA381", &overlay), nets);
    let piece = overlay
        .pieces_of(nets[0])
        .first()
        .expect("drawn Q data net");
    let value = overlay.pieces[*piece as usize].value as usize;
    let signal = compiled
        .kernel
        .signal(&overlay.values[value])
        .expect("published Q wire");
    let mut reader = Reader::new(&[Some(signal)]);
    let mut published = Vec::new();
    let mut fast = FastSim::new(&compiled.kernel).unwrap();
    let clock = compiled.drivers["pin:G2"];
    let data = compiled.drivers["pin:B2"];
    fast.clock(clock, Time::from_ps(PERIOD_PS)).unwrap();
    let mut q = true; // OFS1P3BX's configured SET state, as in X12.
    let mut observations = 0;
    for cycle in 0..8 {
        let d = cycle % 3 == 0;
        let base = cycle * PERIOD_PS;
        for (ps, input, edge) in [
            (base + PERIOD_PS / 4, Some(d), None),
            (base + PERIOD_PS / 2, None, Some(true)),
            (base + 3 * PERIOD_PS / 4, Some(!d), None),
            (base + PERIOD_PS, None, Some(false)),
        ] {
            let time = Time::from_ps(ps);
            if let Some(input) = input {
                compiled
                    .kernel
                    .schedule(time, data, Logic::from_bool(input))
                    .unwrap();
                fast.set_at(time, &[(data, input)]).unwrap();
            }
            if let Some(edge) = edge {
                compiled
                    .kernel
                    .schedule(time, clock, Logic::from_bool(edge))
                    .unwrap();
                if edge {
                    q = d;
                }
            }
            compiled.kernel.run_until(time).unwrap();
            fast.run_until(time).unwrap();
            fast.step().unwrap(); // The full settle used by a publication.
            fast.read(&mut reader, &mut published);
            let expected = Logic::from_bool(q);
            assert_eq!(
                compiled.kernel.value(register_q).unwrap(),
                expected,
                "OREG Q at {time}"
            );
            assert_eq!(
                compiled.kernel.value(pad_q).unwrap(),
                expected,
                "pad Q at {time}"
            );
            assert_eq!(
                compiled.kernel.value(signal).unwrap(),
                expected,
                "window wire at {time}"
            );
            assert_eq!(
                fast.value(register_q),
                Some(expected),
                "fast OREG Q at {time}"
            );
            assert_eq!(
                published,
                [u8::from(q)],
                "published window observation at {time}"
            );
            observations += 1;
        }
    }
    assert_eq!(observations, 32);
}
