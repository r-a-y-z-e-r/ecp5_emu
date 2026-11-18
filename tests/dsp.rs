mod common;
use common::{write, Fixture};
use ecp5_emu::{
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{multiply18, Builder, DriverId, Input, Kernel, Multiplier18, Strength, Time},
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        ResourceId,
    },
    prepared::ConfiguredDevice,
    state::DeviceState,
};
use serde_json::json;
use std::{fs, sync::Arc};
fn bits(value: u64) -> [Logic; 18] {
    std::array::from_fn(|bit| Logic::from_bool(value & (1 << bit) != 0))
}
fn oracle(a: u64, b: u64, sa: bool, sb: bool) -> [Logic; 36] {
    let signed = |v: u64, s: bool| {
        if s && v >= 131072 {
            v as i128 - 262144
        } else {
            v as i128
        }
    };
    let product = (signed(a, sa) * signed(b, sb)).rem_euclid(1i128 << 36) as u64;
    std::array::from_fn(|bit| Logic::from_bool(product & (1 << bit) != 0))
}
fn fixture() -> (Fixture, Arc<ConfiguredDevice>, [ResourceId; 4]) {
    let f = Fixture::with_compiler();
    let path = f.evidence.join("resource_instances.csv");
    let mut rows = fs::read_to_string(&path).unwrap();
    rows.push_str("LFE5U-85F/R1C1/DSP_SLICE_LEFT,DSP_SLICE,1,1,DSP_SLICE_LEFT,R1C1:PLC2,,False,,synthetic_dsp\n");
    for (name, family) in [
        ("MULT18_0", "MULT18"),
        ("MULT18_1", "MULT18"),
        ("ALU54_3", "ALU54"),
    ] {
        rows.push_str(&format!("LFE5U-85F/R1C1/{name},{family},1,1,{name},R1C1:PLC2,LFE5U-85F/R1C1/DSP_SLICE_LEFT,True,{name},synthetic_dsp\n"));
    }
    write(&path, rows);
    let path = f.evidence.join("resource_ports.csv");
    let mut rows = fs::read_to_string(&path).unwrap();
    for name in ["MULT18_0", "MULT18_1"] {
        for port in (0..18)
            .flat_map(|bit| [format!("A{bit}"), format!("B{bit}")])
            .chain(["SIGNEDA", "SIGNEDB", "SOURCEA", "SOURCEB"].map(String::from))
            .chain((0..4).flat_map(|n| [format!("CLK{n}"), format!("CE{n}"), format!("RST{n}")]))
        {
            rows.push_str(&format!(
                "LFE5U-85F/R1C1/{name},{port},in,1,1,{name}_{port}\n"
            ));
        }
        for port in (0..36)
            .map(|bit| format!("P{bit}"))
            .chain(["SIGNEDP".into()])
            .chain((0..18).flat_map(|bit| [format!("ROA{bit}"), format!("ROB{bit}")]))
        {
            rows.push_str(&format!(
                "LFE5U-85F/R1C1/{name},{port},out,1,1,{name}_{port}\n"
            ));
        }
    }
    write(&path, rows);
    f.seal();
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":4,"bits_per_frame":16,"encoding":"frame_byte_lsb0","cram_hex":"0000000000000000","bram_data":{},"origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalog = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[]})).unwrap()],
        "LFE5U-85F",
        4,
        16,
    )
    .unwrap();
    let device =
        Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalog).unwrap());
    let ids = ["DSP_SLICE_LEFT", "MULT18_0", "MULT18_1", "ALU54_3"].map(|n| {
        device
            .model()
            .resource_id(&format!("LFE5U-85F/R1C1/{n}"))
            .unwrap()
    });
    (f, device, ids)
}
fn wire(name: &str) -> Wire {
    Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    }
}
#[test]
fn known_arithmetic_includes_all_sign_modes_boundaries_and_random_vectors() {
    for a in [0, 1, 2, 131071, 131072, 262143] {
        for b in [0, 1, 3, 131071, 131072, 262143] {
            for sa in [false, true] {
                for sb in [false, true] {
                    assert_eq!(
                        multiply18(
                            &bits(a),
                            &bits(b),
                            Logic::from_bool(sa),
                            Logic::from_bool(sb)
                        ),
                        oracle(a, b, sa, sb)
                    );
                }
            }
        }
    }
    let mut random = 0x7b459926u64;
    for _ in 0..5000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = random & 262143;
        let b = (random >> 32) & 262143;
        let sa = random & (1 << 19) != 0;
        let sb = random & (1 << 20) != 0;
        assert_eq!(
            multiply18(
                &bits(a),
                &bits(b),
                Logic::from_bool(sa),
                Logic::from_bool(sb)
            ),
            oracle(a, b, sa, sb)
        );
    }
}
#[test]
fn unknown_data_and_signs_preserve_only_sound_known_bits() {
    assert_eq!(multiply18(&[X; 18], &[Zero; 18], X, One), [Zero; 36]);
    assert_eq!(multiply18(&[Zero; 18], &[Z; 18], One, X), [Zero; 36]);
    for sa in [Zero, One, X, Z] {
        for sb in [Zero, One, X, Z] {
            let a = bits(0x3ffab);
            let b = bits(0x20001);
            let actual = multiply18(&a, &b, sa, sb);
            let mut possible = Vec::new();
            for x in [false, true]
                .into_iter()
                .filter(|v| sa.bit().is_none_or(|s| s == *v))
            {
                for y in [false, true]
                    .into_iter()
                    .filter(|v| sb.bit().is_none_or(|s| s == *v))
                {
                    possible.push(oracle(0x3ffab, 0x20001, x, y));
                }
            }
            for bit in 0..36 {
                let first = possible[0][bit];
                assert_eq!(
                    actual[bit],
                    if possible.iter().all(|p| p[bit] == first) {
                        first
                    } else {
                        X
                    }
                );
            }
        }
    }
    // Exhaustively concretize three unknown data bits and both sign controls.
    let mut a = bits(0x30035);
    let mut b = bits(0x20017);
    a[0] = X;
    a[17] = Z;
    b[3] = X;
    let actual = multiply18(&a, &b, X, Z);
    for n in 0..32 {
        let mut av = 0x30035;
        let mut bv = 0x20017;
        av = (av & !1) | (n & 1);
        av = (av & !(1 << 17)) | (((n >> 1) & 1) << 17);
        bv = (bv & !8) | (((n >> 2) & 1) << 3);
        let expected = oracle(av, bv, n & 8 != 0, n & 16 != 0);
        for bit in 0..36 {
            if actual[bit].bit().is_some() {
                assert_eq!(actual[bit], expected[bit]);
            }
        }
    }
}
#[test]
fn all_views_share_one_reserved_bank_and_initial_state_is_isolated() {
    let (_f, device, ids) = fixture();
    let a = DeviceState::new(&device).unwrap();
    let b = DeviceState::new(&device).unwrap();
    let (owner, bank) = a.dsp_slice(ids[0]).unwrap();
    for id in ids {
        let (peer, view) = a.dsp_slice(id).unwrap();
        assert_eq!(peer, owner);
        assert!(std::ptr::eq(bank, view));
    }
    assert_eq!(bank.shared_output(), &[X; 72]);
    assert!(!std::ptr::eq(bank, b.dsp_slice(owner).unwrap().1));
    assert_eq!(a.summary().dsp_slices, 1);
    assert_eq!(a.summary().dsp_shared_output_bits, 72);
    assert_eq!(a.summary().known_dsp_shared_output_bits, 0);
}
#[test]
fn multiplier_outputs_settle_before_ff_capture_and_reject_runtime_source_modes() {
    let (_f, device, ids) = fixture();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let ff = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_FF")
        .unwrap()
        .id;
    let mut b = Builder::new(device).unwrap();
    let mut signals = Vec::new();
    let mut drivers = Vec::new();
    for (name, value) in [
        ("A", One),
        ("B", One),
        ("SA", Zero),
        ("SB", Zero),
        ("SOURCE", Zero),
    ] {
        let s = b.signal(wire(name)).unwrap();
        drivers.push(b.external_driver(s, name, Strength::Strong, value).unwrap());
        signals.push(s);
    }
    let mut a = [Input::Constant(Zero); 18];
    a[0] = signals[0].into();
    let mut input_b = a;
    input_b[0] = signals[1].into();
    let config = Multiplier18 {
        resource: ids[1],
        a,
        b: input_b,
        signed_a: signals[2].into(),
        signed_b: signals[3].into(),
        source_a: signals[4].into(),
        source_b: Input::Constant(Zero),
    };
    let outputs = b.multiplier18(config.clone()).unwrap();
    assert!(b.multiplier18(config).is_err());
    let clock = b.signal(wire("CLOCK")).unwrap();
    let ck = b
        .external_driver(clock, "CLOCK", Strength::Strong, Zero)
        .unwrap();
    let captured = b
        .flip_flop(ecp5_emu::kernel::FlipFlop {
            preload: None,
            node: ff,
            data: outputs.product[0].into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: ecp5_emu::kernel::Edge::Rising,
            reset: None,
        })
        .unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(outputs.product[0]).unwrap(), One);
    assert_eq!(k.value(outputs.signed).unwrap(), Zero);
    k.schedule_batch(Time::from_ps(1), &[(drivers[0], Zero), (ck, One)], None)
        .unwrap();
    k.run_until(Time::from_ps(1)).unwrap();
    assert!(outputs.product.iter().all(|&p| k.value(p).unwrap() == Zero));
    assert_eq!(k.value(captured).unwrap(), Zero);
    k.schedule(Time::from_ps(2), drivers[2], One).unwrap();
    k.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(k.value(outputs.signed).unwrap(), One);
    assert_eq!(
        k.state().dsp_slice(ids[1]).unwrap().1.shared_output(),
        &[X; 72]
    );
    k.schedule(Time::from_ps(3), drivers[4], One).unwrap();
    assert!(k.run_until(Time::from_ps(3)).is_err());
}

/// Fast mode (src/fast.rs, kernel/fast_dsp.rs) against the event kernel on
/// two parallel MULT18 without registers, as yosys writes them: random
/// operands and sign controls, some operand bits inverted or tied, and a
/// flip-flop capturing a product bit. Every P, SIGNEDP, ROA/ROB and the
/// flip-flop agree after every instant.
#[test]
#[ignore = "requires clang"]
fn fast_mode_matches_the_event_kernel_on_combinational_multipliers() {
    let (_f, device, ids) = fixture();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let ff = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_FF")
        .unwrap()
        .id;
    let mut b = Builder::new(device).unwrap();
    let mut inputs = Vec::new();
    let mut input = |b: &mut Builder, name: String| {
        let s = b.signal(wire(&name)).unwrap();
        inputs.push(b.external_driver(s, &name, Strength::Strong, Zero).unwrap());
        s
    };
    let mut watched = Vec::new();
    for m in 0..2 {
        let a: [_; 18] = std::array::from_fn(|k| input(&mut b, format!("M{m}A{k}")));
        let bb: [_; 18] = std::array::from_fn(|k| input(&mut b, format!("M{m}B{k}")));
        let (sa, sb) = (
            input(&mut b, format!("M{m}SA")),
            input(&mut b, format!("M{m}SB")),
        );
        let config = Multiplier18 {
            resource: ids[1 + m],
            // The second multiplier inverts some bits of A, ties its top bit
            // and treats A as signed.
            a: std::array::from_fn(|k| match (m, k) {
                (1, 17) => Input::Constant(One),
                (1, k) if k % 3 == 1 => Input::Inverted(a[k]),
                _ => a[k].into(),
            }),
            b: bb.map(Input::from),
            signed_a: if m == 1 {
                Input::Constant(One)
            } else {
                sa.into()
            },
            signed_b: sb.into(),
            source_a: Input::Constant(Zero),
            source_b: Input::Constant(Zero),
        };
        let o = b.multiplier18(config).unwrap();
        watched.extend(o.product);
        watched.push(o.signed);
        watched.extend(o.roa);
        watched.extend(o.rob);
    }
    let clock = input(&mut b, "CLOCK".into());
    let captured = b
        .flip_flop(ecp5_emu::kernel::FlipFlop {
            preload: None,
            node: ff,
            data: watched[5].into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: ecp5_emu::kernel::Edge::Rising,
            reset: None,
        })
        .unwrap();
    watched.push(captured);
    for &s in &watched {
        b.watch(s).unwrap();
    }
    let mut k = b.build().unwrap();
    let mut fast = ecp5_emu::fast::FastSim::new(&k).unwrap();
    let mut seed = 0x853c_49e6_748f_ea9b_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut values = vec![false; inputs.len()];
    for step in 1..=3000u64 {
        let mut set = Vec::new();
        for (i, &d) in inputs.iter().enumerate() {
            if next() % 4 == 0 {
                values[i] = next() % 2 == 1;
                set.push((d, values[i]));
            }
        }
        let time = Time::from_ps(step);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        k.schedule_batch(time, &batch, None).unwrap();
        k.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        for (n, &s) in watched.iter().enumerate() {
            assert_eq!(
                fast.value(s),
                Some(k.value(s).unwrap()),
                "step {step} output {n}"
            );
        }
    }
}

/// Fast mode against the event kernel on MULT18 register stages: every
/// combination of INPUTA, INPUTB, PIPELINE and OUTPUT, with SYNC and ASYNC
/// local resets, four independent clocks, random CE/RST and a global reset
/// (where the profile takes one: not with OUTPUT). The kernel runs until
/// every stage is known, then fast mode takes over; after each of 2000
/// random instants every P, SIGNEDP (but after OUTPUT, where both keep it
/// X), ROA and ROB agree. Resets, local and global, also change in instants
/// with clock edges: every stage captures from the state before the instant,
/// and a stage's own reset wins over its own edge.
#[test]
#[ignore = "requires clang"]
fn fast_mode_matches_the_event_kernel_on_multiplier_register_stages() {
    multiplier_register_profiles(false);
}

#[test]
#[ignore = "requires clang"]
fn hd09_disabled_fast_mode_matches_the_event_kernel_on_multiplier_register_stages() {
    multiplier_register_profiles(true);
}

fn multiplier_register_profiles(both_edges: bool) {
    use ecp5_emu::kernel::{
        MultiplierOutputRegister, MultiplierPipeline, MultiplierRegister, ResetKind,
    };
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut threaded = false;
    for mask in 1..16 {
        for kind in [ResetKind::Synchronous, ResetKind::Asynchronous] {
            let label = format!("stages {mask:04b} {kind:?}, both_edges {both_edges}");
            let (_f, device, ids) = fixture();
            let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
            let ff = device
                .model()
                .nodes(plc)
                .find(|n| n.kind == "TRELLIS_FF")
                .unwrap()
                .id;
            let mut b = Builder::new(device).unwrap();
            let mut inputs = Vec::new();
            let mut input = |b: &mut Builder, name: String, value: Logic| {
                let s = b.signal(wire(&name)).unwrap();
                inputs.push(
                    b.external_driver(s, &name, Strength::Strong, value)
                        .unwrap(),
                );
                Input::from(s)
            };
            let a: [Input; 18] = std::array::from_fn(|k| input(&mut b, format!("A{k}"), Zero));
            let bb: [Input; 18] = std::array::from_fn(|k| input(&mut b, format!("B{k}"), Zero));
            let (sa, sb) = (
                input(&mut b, "SA".into(), Zero),
                input(&mut b, "SB".into(), Zero),
            );
            let control: [MultiplierRegister; 4] = std::array::from_fn(|n| MultiplierRegister {
                clock: input(&mut b, format!("CK{n}"), Zero),
                enable: input(&mut b, format!("CE{n}"), One),
                reset: input(&mut b, format!("RST{n}"), Zero),
                reset_kind: kind,
            });
            let output = mask & 8 != 0;
            let gsr = (both_edges || !output).then(|| input(&mut b, "GSR".into(), Zero));
            let o = b
                .multiplier18_with_pipeline(
                    Multiplier18 {
                        resource: ids[1],
                        a,
                        b: bb,
                        signed_a: sa,
                        signed_b: sb,
                        source_a: Input::Constant(Zero),
                        source_b: Input::Constant(Zero),
                    },
                    MultiplierPipeline {
                        both_edges,
                        input_a: (mask & 1 != 0).then_some(control[0]),
                        input_b: (mask & 2 != 0).then_some(control[1]),
                        product: (mask & 4 != 0).then_some(control[2]),
                        output: output.then_some(MultiplierOutputRegister {
                            clock: control[3].clock,
                            enable: control[3].enable,
                            reset: control[3].reset,
                            reset_kind: control[3].reset_kind,
                        }),
                        global_reset: gsr,
                    },
                )
                .unwrap();
            let mut watched: Vec<_> = o.product.to_vec();
            if !output {
                watched.push(o.signed);
            }
            if both_edges {
                watched.push(
                    b.flip_flop(ecp5_emu::kernel::FlipFlop {
                        preload: None,
                        node: ff,
                        data: o.product[0].into(),
                        clock: control[0].clock,
                        enable: Input::Constant(One),
                        edge: ecp5_emu::kernel::Edge::Rising,
                        reset: None,
                    })
                    .unwrap(),
                );
            }
            watched.extend(o.roa);
            watched.extend(o.rob);
            for &s in &watched {
                b.watch(s).unwrap();
            }
            let mut k = b.build().unwrap();
            let mut values = vec![false; inputs.len()];
            let names = 18 + 18 + 2;
            let mut time = 0;
            let mut draw = |values: &mut Vec<bool>| {
                let mut set = Vec::new();
                for n in 0..4 {
                    let i = names + 3 * n;
                    if next() % 3 == 0 {
                        values[i] = !values[i];
                        set.push((inputs[i], values[i]));
                    }
                }
                for (i, &d) in inputs.iter().enumerate() {
                    let v = if i < names {
                        next() % 2 == 1
                    } else {
                        // CE, RST per stage, then GSR.
                        match (i - names) % 3 {
                            _ if i - names >= 12 => next() % 100 < 2,
                            0 => continue,
                            1 => next() % 10 < 8,
                            _ => next() % 100 < 5,
                        }
                    };
                    if v != values[i] || next() % 4 == 0 {
                        values[i] = v;
                        set.push((d, v));
                    }
                }
                set
            };
            let mut apply = |k: &mut ecp5_emu::kernel::Kernel, set: &[(DriverId, bool)]| {
                time += 1;
                let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
                k.schedule_batch(Time::from_ps(time), &batch, None).unwrap();
                k.run_until(Time::from_ps(time)).unwrap();
                Time::from_ps(time)
            };
            let mut warm = 0;
            let mut fast = loop {
                match ecp5_emu::fast::FastSim::new(&k) {
                    Ok(fast) => break fast,
                    Err(e)
                        if e.downcast_ref::<ecp5_emu::kernel::StartsUnknown>()
                            .is_some()
                            && warm < 400 =>
                    {
                        warm += 1;
                        let set = draw(&mut values);
                        apply(&mut k, &set);
                    }
                    Err(e) => panic!("{label}: {e:#}"),
                }
            };
            threaded |= fast.program().settle_phases.iter().any(|&n| n > 0);
            if !both_edges {
                common::snapshot_dsp_source(&format!("mult-{mask}-{kind:?}"), &k);
            }
            for step in 0..2000 {
                let set = draw(&mut values);
                let at = apply(&mut k, &set);
                fast.set_at(at, &set)
                    .unwrap_or_else(|e| panic!("{label} step {step}: {e:#}"));
                for (n, &s) in watched.iter().enumerate() {
                    assert_eq!(
                        fast.value(s),
                        Some(k.value(s).unwrap()),
                        "{label} step {step} output {n} (warm {warm})"
                    );
                }
                if both_edges && step % 137 == 0 {
                    fast = ecp5_emu::fast::FastSim::new(&k).unwrap();
                }
            }
        }
    }
    if both_edges
        && std::env::var("FAST_THREADS").as_deref() == Ok("4")
        && std::env::var("FAST_THREAD_MIN").as_deref() == Ok("1")
    {
        assert!(
            threaded,
            "forced run must exercise split DSP clock-group settles"
        );
    }
}

/// HD01: the OUTPUT stage takes CE0 and its selected reset like any stage;
/// fast mode follows the kernel through a driven enable, a synchronous or
/// asynchronous reset, and GSR (HD07).
#[test]
#[ignore = "requires clang"]
fn fast_mode_runs_an_output_stage_with_a_driven_enable_and_reset() {
    use ecp5_emu::kernel::ResetKind;
    output_stage_in_fast_mode(ResetKind::Synchronous);
    output_stage_in_fast_mode(ResetKind::Asynchronous);
}

fn output_stage_in_fast_mode(reset_kind: ecp5_emu::kernel::ResetKind) {
    use ecp5_emu::kernel::{MultiplierOutputRegister, MultiplierPipeline};
    let (_f, device, ids) = fixture();
    let mut b = Builder::new(device).unwrap();
    let input = |b: &mut Builder, name: &str, value: Logic| {
        let s = b.signal(wire(name)).unwrap();
        let d = b.external_driver(s, name, Strength::Strong, value).unwrap();
        (Input::from(s), d)
    };
    let (clock, ck) = input(&mut b, "CK", Zero);
    let (enable, ce) = input(&mut b, "CE", One);
    let (reset, rst) = input(&mut b, "RST", Zero);
    let (global, gsr) = input(&mut b, "GSR", Zero);
    let (a0, a) = input(&mut b, "A0", One);
    let mut operand_a = [Input::Constant(Zero); 18];
    operand_a[0] = a0;
    operand_a[1] = Input::Constant(One);
    let mut operand_b = [Input::Constant(Zero); 18];
    operand_b[0] = Input::Constant(One);
    let out = b
        .multiplier18_with_pipeline(
            Multiplier18 {
                resource: ids[1],
                a: operand_a,
                b: operand_b,
                signed_a: Input::Constant(Zero),
                signed_b: Input::Constant(Zero),
                source_a: Input::Constant(Zero),
                source_b: Input::Constant(Zero),
            },
            MultiplierPipeline {
                both_edges: false,
                output: Some(MultiplierOutputRegister {
                    clock,
                    enable,
                    reset,
                    reset_kind,
                }),
                global_reset: Some(global),
                ..MultiplierPipeline::default()
            },
        )
        .unwrap();
    for &p in &out.product {
        b.watch(p).unwrap();
    }
    let mut k = b.build().unwrap();
    // One edge makes the output bank known (it starts X).
    k.schedule(Time::from_ps(1), ck, One).unwrap();
    k.run_until(Time::from_ps(1)).unwrap();
    let mut fast = ecp5_emu::fast::FastSim::new(&k).unwrap_or_else(|e| panic!("{e:#}"));
    let mut seed = 0x1234_5678_9abc_def1_u64;
    let mut level = [true, true, false, true, false];
    let drivers = [ck, ce, rst, a, gsr];
    let mut changes = 0;
    let mut last = X;
    for step in 1..=4000u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let mut set = Vec::new();
        for (n, d) in drivers.iter().enumerate() {
            // GSR rises rarely and falls soon; the others change often.
            let period = if n == 4 && !level[4] { 17 } else { 3 };
            if ((seed >> (8 * n)) & 0xff).is_multiple_of(period) {
                level[n] = !level[n];
                set.push((*d, level[n]));
            }
        }
        let time = Time::from_ps(step + 1);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        k.schedule_batch(time, &batch, None).unwrap();
        k.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        for &p in &out.product {
            assert_eq!(
                fast.value(p),
                Some(k.value(p).unwrap()),
                "{reset_kind:?} step {step}"
            );
        }
        let v = k.value(out.product[0]).unwrap();
        if v != last {
            changes += 1;
            last = v;
        }
    }
    assert!(changes > 100, "{changes}");
}

#[test]
fn compiler_accepts_parallel_register_profiles_and_rejects_unsupported_modes() {
    use ecp5_emu::compile::{self, Reference, Request, Scope, Startup};
    let (_f, original, _ids) = fixture();
    let field = |name: &str, value: &str| json!({"name":name,"kind":"enum","fragments":[{"tile":"R1C1:PLC2","source":name,"options":{value:[[2,3,false]]}}]});
    let mut fields = vec![
        field("MULT18_0.MODE", "MULT18X18D"),
        field("MULT18_0.CIBOUT_BYP", "ON"),
        field("MULT18_0.SOURCEB_MODE", "B_SHIFT"),
    ];
    for stage in ["INPUTA", "INPUTB", "INPUTC", "PIPELINE", "OUTPUT"] {
        fields.push(field(&format!("MULT18_0.REG_{stage}_CLK"), "NONE"));
    }
    for stage in ["INPUTA", "INPUTB", "PIPELINE"] {
        fields.push(field(&format!("MULT18_0.REG_{stage}_CE"), "CE1"));
        fields.push(field(&format!("MULT18_0.REG_{stage}_RST"), "RST2"));
    }
    for (name, value) in [
        ("CLK0MUX", "INV"),
        ("CE1MUX", "CE1"),
        ("RST2MUX", "RST2"),
        ("CLK0_DIV", "ENABLED"),
        ("RESETMODE", "ASYNC"),
        ("GSR", "DISABLED"),
    ] {
        fields.push(field(&format!("MULT18_0.{name}"), value));
    }
    let catalog = |fields: &Vec<serde_json::Value>, parent: &str, alu: &str| {
        Catalog::merge_documents(&[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[
        {"id":"LFE5U-85F/R1C1/MULT18_0","family":"MULT18","fields":fields},
        {"id":"LFE5U-85F/R1C1/MULT18_1","family":"MULT18","fields":[field("MULT18_1.MODE","NONE")]},
        {"id":"LFE5U-85F/R1C1/DSP_SLICE_LEFT","family":"DSP_SLICE","fields":[field("DSP_LEFT.CIBOUT",parent)]},
        {"id":"LFE5U-85F/R1C1/ALU54_3","family":"ALU54","fields":[field("ALU54_3.MODE",alu),field("ALU54_3.CLK0_DIV","ENABLED")]}]})).unwrap()],"LFE5U-85F",4,16).unwrap()
    };
    let device = |fields: &Vec<serde_json::Value>, parent, alu| {
        Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(original.model()),
                Arc::clone(original.image()),
                catalog(fields, parent, alu),
            )
            .unwrap(),
        )
    };
    let reference = |port: &str| Reference::Wire(wire(&format!("MULT18_0_{port}")));
    let mut drives = Vec::new();
    for side in ['A', 'B'] {
        for bit in 0..18 {
            drives.push((
                reference(&format!("{side}{bit}")),
                Logic::from_bool(bit == 0),
            ));
        }
    }
    for port in ["SIGNEDA", "SIGNEDB", "SOURCEA", "SOURCEB"] {
        drives.push((reference(port), Zero));
    }
    drives.extend([
        (reference("CLK0"), Zero),
        (reference("CE1"), One),
        (reference("RST2"), One),
    ]);
    let request = |scope, probes| Request {
        package: "CABGA381".into(),
        scope,
        startup: Startup::ReadyUnknown,
        drives: drives.clone(),
        probes,
        options: Default::default(),
    };
    let c = compile::compile(
        device(&fields, "ON", "NONE"),
        request(Scope::Observed, vec![reference("P0")]),
    )
    .unwrap();
    assert_eq!(c.value(&reference("P0")).unwrap(), One);
    for port in ["ROA0", "ROA17", "ROB0", "ROB17"] {
        let c = compile::compile(
            device(&fields, "ON", "NONE"),
            request(Scope::Observed, vec![reference(port)]),
        )
        .unwrap();
        assert_eq!(c.report.counts.multiplier18, 1);
        assert_eq!(
            c.value(&reference(port)).unwrap(),
            Logic::from_bool(port.ends_with('0'))
        );
    }
    let c = compile::compile(
        device(&fields, "ON", "NONE"),
        request(Scope::AllPlcRegisters, vec![]),
    )
    .unwrap();
    assert_eq!(c.report.counts.multiplier18, 1);
    let c = compile::compile(
        device(&fields, "ON", "NONE"),
        request(
            Scope::Observed,
            vec![Reference::System(ecp5_emu::kernel::SystemPort::Ready)],
        ),
    )
    .unwrap();
    assert_eq!(c.report.counts.multiplier18, 0);
    for mask in 1u32..8 {
        let mut registered = fields.clone();
        for (n, index) in [3, 4, 6].into_iter().enumerate() {
            if mask & (1 << n) != 0 {
                registered[index]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
            }
        }
        let mut c = compile::compile(
            device(&registered, "ON", "NONE"),
            request(Scope::Observed, vec![reference("P0")]),
        )
        .unwrap();
        assert_eq!(
            c.report.multipliers[0].stages.len(),
            mask.count_ones() as usize
        );
        assert_eq!(c.report.multipliers[0].behavioural_assumptions.len(), 3);
        assert_eq!(c.value(&reference("P0")).unwrap(), Zero);
        // Decode INV, don't convert it into a normal rising-edge clock.
        let first_capture = if mask & 4 != 0 && mask & 3 != 0 {
            Zero
        } else {
            One
        };
        for (t, clock, reset, expected) in [
            (1, One, Zero, Zero),
            (2, Zero, Zero, first_capture),
            (3, One, Zero, first_capture),
            (4, Zero, Zero, One),
        ] {
            c.kernel
                .schedule_batch(
                    Time::from_ps(t),
                    &[
                        (c.drivers[&reference("CLK0").label()], clock),
                        (c.drivers[&reference("RST2").label()], reset),
                    ],
                    None,
                )
                .unwrap();
            c.kernel.run_until(Time::from_ps(t)).unwrap();
            // With input(s)+pipeline enabled, the first effective edge only
            // advances the input(s); product uses the prior reset operand(s).
            assert_eq!(
                c.value(&reference("P0")).unwrap(),
                expected,
                "mask={mask} t={t}"
            );
        }
    }
    for inverted in 0..4 {
        let mut registered = fields.clone();
        registered[6]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
        for (name, inv) in [
            ("CE1MUX", inverted & 1 != 0),
            ("RST2MUX", inverted & 2 != 0),
        ] {
            let f = registered
                .iter_mut()
                .find(|f| f["name"] == format!("MULT18_0.{name}"))
                .unwrap();
            let value = if inv { "INV" } else { &name[..name.len() - 3] };
            f["fragments"][0]["options"] = json!({value:[[2,3,false]]});
        }
        let mut c = compile::compile(
            device(&registered, "ON", "NONE"),
            request(Scope::Observed, vec![reference("P0")]),
        )
        .unwrap();
        for (t, clk, enable, reset, expected) in [
            (1, One, false, true, Zero),
            (2, Zero, false, false, Zero),
            (3, One, true, false, Zero),
            (4, Zero, true, false, One),
        ] {
            let updates = [
                (c.drivers[&reference("CLK0").label()], clk),
                (
                    c.drivers[&reference("CE1").label()],
                    Logic::from_bool(enable ^ (inverted & 1 != 0)),
                ),
                (
                    c.drivers[&reference("RST2").label()],
                    Logic::from_bool(reset ^ (inverted & 2 != 0)),
                ),
            ];
            c.kernel
                .schedule_batch(Time::from_ps(t), &updates, None)
                .unwrap();
            c.kernel.run_until(Time::from_ps(t)).unwrap();
            assert_eq!(
                c.value(&reference("P0")).unwrap(),
                expected,
                "INV={inverted} t={t}"
            );
        }
    }
    {
        let mut registered = fields.clone();
        registered[1]["fragments"][0]["options"] = json!({"OFF":[[2,3,false]]});
        registered[7]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
        for prefix in ["CE", "RST"] {
            for n in 0..4 {
                let name = format!("MULT18_0.{prefix}{n}MUX");
                if !registered.iter().any(|f| f["name"] == name) {
                    registered.push(field(&name, &format!("{prefix}{n}")));
                }
            }
        }
        let mut output_drives = drives.clone();
        output_drives.retain(|(r, _)| r != &reference("CE1") && r != &reference("RST2"));
        for n in 0..4 {
            output_drives.extend([
                (reference(&format!("CE{n}")), One),
                (reference(&format!("RST{n}")), Zero),
            ]);
        }
        let request_output = |probes| Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::ReadyUnknown,
            drives: output_drives.clone(),
            probes,
            options: Default::default(),
        };
        let mut c = compile::compile(
            device(&registered, "ON", "NONE"),
            request_output(vec![reference("P0")]),
        )
        .unwrap();
        assert_eq!(
            c.report.multipliers[0]
                .output_register
                .as_ref()
                .unwrap()
                .logical_bit_start,
            0
        );
        assert_eq!(c.value(&reference("P0")).unwrap(), X);
        for (t, clock, expected) in [(1, One, X), (2, Zero, One), (3, One, One), (4, Zero, One)] {
            c.kernel
                .schedule(
                    Time::from_ps(t),
                    c.drivers[&reference("CLK0").label()],
                    clock,
                )
                .unwrap();
            c.kernel.run_until(Time::from_ps(t)).unwrap();
            assert_eq!(c.value(&reference("P0")).unwrap(), expected);
        }
        assert_eq!(c.kernel.state().summary().known_dsp_shared_output_bits, 36);
        for preceding in [None, Some("P0"), Some("ROA0"), Some("ROB17")] {
            let probes = preceding
                .into_iter()
                .chain(["SIGNEDP"])
                .map(reference)
                .collect();
            assert!(
                compile::compile(device(&registered, "ON", "NONE"), request_output(probes))
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("SIGNEDP")
            );
        }
        // GSR=ENABLED: the output stage takes GSR as well (HD07).
        let mut gsr = registered.clone();
        gsr.iter_mut()
            .find(|f| f["name"] == "MULT18_0.GSR")
            .unwrap()["fragments"][0]["options"] = json!({"ENABLED":[[2,3,false]]});
        compile::compile(
            device(&gsr, "ON", "NONE"),
            request_output(vec![reference("P0")]),
        )
        .unwrap();
        // The output stage reads CE0 (HD01): without its mux there is no plan.
        let mut bad = registered.clone();
        bad.retain(|f| f["name"] != "MULT18_0.CE0MUX");
        assert!(compile::compile(
            device(&bad, "ON", "NONE"),
            request_output(vec![reference("P0")])
        )
        .is_err());
    }
    // HD09 replaces the earlier refusal of the disabled divider.
    let mut disabled = fields.clone();
    disabled[6]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
    disabled
        .iter_mut()
        .find(|f| f["name"] == "MULT18_0.CLK0_DIV")
        .unwrap()["fragments"][0]["options"] = json!({"DISABLED":[[2,3,false]]});
    compile::compile(
        device(&disabled, "ON", "NONE"),
        request(Scope::Observed, vec![reference("P0")]),
    )
    .unwrap();
    let mut disagreement = disabled.clone();
    disagreement[3]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
    let error = compile::compile(
        device(&disagreement, "ON", "ALU54B"),
        request(Scope::Observed, vec![reference("ROA0")]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("ALU/MULT18 CLK_DIV disagreement"));
    let mut disagreement = disabled.clone();
    disagreement[3]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
    let error = compile::compile(
        device(&disagreement, "ON", "ALU54B"),
        request(Scope::Observed, vec![reference("ROA0")]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("ALU/MULT18 CLK_DIV disagreement"));
    // One cell with active stages on differently configured clocks is refused.
    let mut mixed = disabled.clone();
    mixed[3]["fragments"][0]["options"] = json!({"CLK1":[[2,3,false]]});
    mixed.push(field("MULT18_0.CLK1_DIV", "ENABLED"));
    let error = compile::compile(
        device(&mixed, "ON", "NONE"),
        request(Scope::Observed, vec![reference("P0")]),
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("mixed ENABLED/DISABLED DSP register clocks"));
    for (name, options) in [
        (
            "REG_PIPELINE_CE",
            json!({"CE0":[[2,3,false]],"CE1":[[2,3,false]]}),
        ),
        (
            "REG_PIPELINE_CLK",
            json!({"CLK0":[[2,3,false]],"CLK1":[[2,3,false]]}),
        ),
    ] {
        let mut bad = fields.clone();
        bad[6]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
        let f = bad
            .iter_mut()
            .find(|f| f["name"] == format!("MULT18_0.{name}"))
            .unwrap();
        f["fragments"][0]["options"] = options;
        assert!(compile::compile(
            device(&bad, "ON", "NONE"),
            request(Scope::Observed, vec![reference("P0")])
        )
        .is_err());
    }
    for index in [5, 7] {
        let mut bad = fields.clone();
        bad[index]["fragments"][0]["options"] = json!({"CLK0":[[2,3,false]]});
        assert!(compile::compile(
            device(&bad, "ON", "NONE"),
            request(Scope::Observed, vec![reference("P0")])
        )
        .is_err());
    }
    for (parent, alu) in [("OFF", "NONE"), ("ON", "ALU54B")] {
        assert!(compile::compile(
            device(&fields, parent, alu),
            request(Scope::Observed, vec![reference("P0")])
        )
        .is_err());
    }
    // An active ALU allows only a view of the independently known operands.
    // Its output/PIPELINE settings must neither gate this view nor be executed.
    let mut combined = fields.clone();
    for suffix in [
        "CIBOUT_BYP",
        "REG_INPUTC_CLK",
        "REG_PIPELINE_CLK",
        "REG_OUTPUT_CLK",
    ] {
        combined
            .iter_mut()
            .find(|f| f["name"] == format!("MULT18_0.{suffix}"))
            .unwrap()["fragments"][0]["options"] = json!({"unqualified":[[2,3,false]]});
    }
    for first in ["ROA0", "ROB17"] {
        let c = compile::compile(
            device(&combined, "ON", "ALU54B"),
            request(Scope::Observed, vec![reference(first)]),
        )
        .unwrap();
        assert_eq!(c.report.multipliers[0].execution_view, "operands_only");
        assert_eq!(c.report.multipliers[0].register_candidates.len(), 2);
        assert!(c.report.multipliers[0].output_register.is_none());
        assert!(c.report.multipliers[0].stages.is_empty());
        for unwanted in ["P0", "SIGNEDP"] {
            assert!(compile::compile(
                device(&combined, "ON", "ALU54B"),
                request(Scope::Observed, vec![reference(first), reference(unwanted)])
            )
            .is_err());
        }
    }
    assert!(compile::compile(
        device(&combined, "ON", "ALU54B"),
        request(Scope::AllPlcRegisters, vec![reference("ROA0")])
    )
    .is_err());
    let mut bad = fields.clone();
    bad[1]["fragments"][0]["options"] = json!({"OFF":[[2,3,false]]});
    assert!(compile::compile(
        device(&bad, "ON", "NONE"),
        request(Scope::Observed, vec![reference("P0")])
    )
    .is_err());
    let mut bad = fields.clone();
    bad[2]["fragments"][0]["options"] = json!({"C_SHIFT":[[2,3,false]]});
    assert!(compile::compile(
        device(&bad, "ON", "NONE"),
        request(Scope::Observed, vec![reference("P0")])
    )
    .is_err());
}

#[test]
fn all_input_pipeline_combinations_follow_independent_clocks_enables_and_resets() {
    use ecp5_emu::kernel::{MultiplierPipeline, MultiplierRegister, ResetKind};
    for mask in 0..8 {
        for kind in [ResetKind::Synchronous, ResetKind::Asynchronous] {
            let (_f, device, ids) = fixture();
            let mut builder = Builder::new(device).unwrap();
            let mut ext = |name: &str, initial| {
                let s = builder.signal(wire(name)).unwrap();
                let d = builder
                    .external_driver(s, name, Strength::Strong, initial)
                    .unwrap();
                (Input::from(s), d)
            };
            let aa = (0..18)
                .map(|n| ext(&format!("DA{n}"), Zero))
                .collect::<Vec<_>>();
            let bb = (0..18)
                .map(|n| ext(&format!("DB{n}"), Zero))
                .collect::<Vec<_>>();
            let sa = ext("SA", Zero);
            let sb = ext("SB", Zero);
            let clocks = (0..3)
                .map(|n| ext(&format!("CK{n}"), Zero))
                .collect::<Vec<_>>();
            let ce = (0..3)
                .map(|n| ext(&format!("CE{n}"), One))
                .collect::<Vec<_>>();
            let rst = (0..3)
                .map(|n| ext(&format!("RST{n}"), Zero))
                .collect::<Vec<_>>();
            let gsr = ext("GSR", Zero);
            let controls: [Option<MultiplierRegister>; 3] = std::array::from_fn(|n| {
                (mask & (1 << n) != 0).then_some(MultiplierRegister {
                    clock: clocks[n].0,
                    enable: ce[n].0,
                    reset: rst[n].0,
                    reset_kind: kind,
                })
            });
            let outputs = builder
                .multiplier18_with_pipeline(
                    Multiplier18 {
                        resource: ids[1],
                        a: std::array::from_fn(|n| aa[n].0),
                        b: std::array::from_fn(|n| bb[n].0),
                        signed_a: sa.0,
                        signed_b: sb.0,
                        source_a: Input::Constant(Zero),
                        source_b: Input::Constant(Zero),
                    },
                    MultiplierPipeline {
                        both_edges: false,
                        input_a: controls[0],
                        input_b: controls[1],
                        product: controls[2],
                        global_reset: Some(gsr.0),
                        output: None,
                    },
                )
                .unwrap();
            let mut kernel = builder.build().unwrap();
            // Numeric reference with unknown initial stage values; independent
            // from the kernel's bit-vector capture and arithmetic implementation.
            let mut ar: Option<(u64, bool)> = None;
            let mut br: Option<(u64, bool)> = None;
            let mut pr: Option<(u64, bool)> = None;
            let mut previous = [false; 3];
            let mut random = 0xa2b7493851u64;
            for t in 0..257u64 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let a = random & 262143;
                let b = (random >> 32) & 262143;
                let asigned = t & 2 != 0;
                let bsigned = t & 4 != 0;
                let clk = [t & 1 != 0, (t / 3) & 1 != 0, (t / 5) & 1 != 0];
                let enable = [t % 7 != 1, t % 11 != 2, t % 13 != 3];
                let reset = [t % 23 == 5, t % 29 == 7, t % 31 == 9];
                let global = t % 47 == 11;
                let mut changes = Vec::new();
                for bit in 0..18 {
                    changes.push((aa[bit].1, Logic::from_bool(a & (1 << bit) != 0)));
                    changes.push((bb[bit].1, Logic::from_bool(b & (1 << bit) != 0)));
                }
                changes.extend([
                    (sa.1, Logic::from_bool(asigned)),
                    (sb.1, Logic::from_bool(bsigned)),
                    (gsr.1, Logic::from_bool(global)),
                ]);
                for n in 0..3 {
                    changes.extend([
                        (clocks[n].1, Logic::from_bool(clk[n])),
                        (ce[n].1, Logic::from_bool(enable[n])),
                        (rst[n].1, Logic::from_bool(reset[n])),
                    ]);
                }
                let av = if mask & 1 != 0 {
                    ar
                } else {
                    Some((a, asigned))
                };
                let bv = if mask & 2 != 0 {
                    br
                } else {
                    Some((b, bsigned))
                };
                let expected_product = |a: Option<(u64, bool)>, b: Option<(u64, bool)>| {
                    a.zip(b).map(|((a, sa), (b, sb))| {
                        let signed =
                            |v: u64, s| v as i128 - if s && v >= 131072 { 262144 } else { 0 };
                        (
                            ((signed(a, sa) * signed(b, sb)).rem_euclid(1i128 << 36) as u64),
                            sa || sb,
                        )
                    })
                };
                let edges = std::array::from_fn::<_, 3, _>(|n| !previous[n] && clk[n]);
                let next = |n: usize, old, data| {
                    if global || (reset[n] && (kind == ResetKind::Asynchronous || edges[n])) {
                        Some((0, false))
                    } else if edges[n] && enable[n] {
                        data
                    } else {
                        old
                    }
                };
                if mask & 4 != 0 {
                    pr = next(2, pr, expected_product(av, bv));
                }
                if mask & 1 != 0 {
                    ar = next(0, ar, Some((a, asigned)));
                }
                if mask & 2 != 0 {
                    br = next(1, br, Some((b, bsigned)));
                }
                previous = clk;
                let time = Time::from_ps(t + 1);
                kernel.schedule_batch(time, &changes, None).unwrap();
                kernel.run_until(time).unwrap();
                for (signals, operand, label) in [
                    (
                        outputs.roa,
                        if mask & 1 != 0 {
                            ar
                        } else {
                            Some((a, asigned))
                        },
                        "ROA",
                    ),
                    (
                        outputs.rob,
                        if mask & 2 != 0 {
                            br
                        } else {
                            Some((b, bsigned))
                        },
                        "ROB",
                    ),
                ] {
                    for (bit, signal) in signals.into_iter().enumerate() {
                        let expected =
                            operand.map_or(X, |(v, _)| Logic::from_bool(v & (1 << bit) != 0));
                        assert_eq!(
                            kernel.value(signal).unwrap(),
                            expected,
                            "mask={mask} {kind:?} t={t} {label}{bit}"
                        );
                    }
                }
                let expected = if mask & 4 != 0 {
                    pr
                } else {
                    expected_product(
                        if mask & 1 != 0 {
                            ar
                        } else {
                            Some((a, asigned))
                        },
                        if mask & 2 != 0 {
                            br
                        } else {
                            Some((b, bsigned))
                        },
                    )
                };
                if let Some((word, sign)) = expected {
                    for bit in 0..36 {
                        assert_eq!(
                            kernel.value(outputs.product[bit]).unwrap(),
                            Logic::from_bool(word & (1 << bit) != 0),
                            "mask={mask} {kind:?} t={t} bit={bit}"
                        );
                    }
                    assert_eq!(
                        kernel.value(outputs.signed).unwrap(),
                        Logic::from_bool(sign)
                    );
                }
            }
            assert_eq!(
                kernel.state().dsp_slice(ids[1]).unwrap().1.shared_output(),
                &[X; 72]
            );
        }
    }
}

#[test]
fn coincident_input_pipeline_and_fabric_edges_preserve_each_stage_latency() {
    use ecp5_emu::kernel::{Edge, FlipFlop, MultiplierPipeline, MultiplierRegister, ResetKind};
    let (_f, device, ids) = fixture();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let ff = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_FF")
        .unwrap()
        .id;
    let mut b = Builder::new(device).unwrap();
    let clock = b.signal(wire("CLK")).unwrap();
    let ck = b
        .external_driver(clock, "clock", Strength::Strong, One)
        .unwrap();
    let data = b.signal(wire("DATA")).unwrap();
    let d = b
        .external_driver(data, "data", Strength::Strong, One)
        .unwrap();
    let control = MultiplierRegister {
        clock: clock.into(),
        enable: Input::Constant(One),
        reset: Input::Constant(Zero),
        reset_kind: ResetKind::Synchronous,
    };
    let mut a = [Input::Constant(Zero); 18];
    a[0] = data.into();
    let mut one = [Input::Constant(Zero); 18];
    one[0] = Input::Constant(One);
    let outputs = b
        .multiplier18_with_pipeline(
            Multiplier18 {
                resource: ids[1],
                a,
                b: one,
                signed_a: Input::Constant(Zero),
                signed_b: Input::Constant(Zero),
                source_a: Input::Constant(Zero),
                source_b: Input::Constant(Zero),
            },
            MultiplierPipeline {
                both_edges: false,
                input_a: Some(control),
                product: Some(control),
                ..Default::default()
            },
        )
        .unwrap();
    let q = b
        .flip_flop(FlipFlop {
            preload: None,
            node: ff,
            data: outputs.product[0].into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: Edge::Rising,
            reset: None,
        })
        .unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(outputs.product[0]).unwrap(), X); // no construction edge
    assert_eq!(k.state().multiplier(ids[1]).unwrap().input_a, [X; 18]);
    for (t, clk, input, p, ff) in [
        (1, Zero, One, X, X),
        (2, One, One, X, X),
        (3, Zero, Zero, X, X),
        (4, One, Zero, One, X),
        (5, Zero, One, One, X),
        (6, One, One, Zero, One),
        (7, Zero, One, Zero, One),
        (8, One, One, One, Zero),
    ] {
        k.schedule_batch(Time::from_ps(t), &[(ck, clk), (d, input)], None)
            .unwrap();
        k.run_until(Time::from_ps(t)).unwrap();
        assert_eq!(k.value(outputs.product[0]).unwrap(), p, "t={t}");
        assert_eq!(k.value(q).unwrap(), ff, "t={t}");
    }
}

#[test]
fn unknown_dsp_controls_merge_values_and_reset_dominates_disabled_enable() {
    use ecp5_emu::kernel::{MultiplierPipeline, MultiplierRegister, ResetKind};
    for kind in [ResetKind::Synchronous, ResetKind::Asynchronous] {
        for input_stage in [false, true] {
            let (_f, device, ids) = fixture();
            let mut b = Builder::new(device).unwrap();
            let mut external = |name, level| {
                let s = b.signal(wire(name)).unwrap();
                let d = b.external_driver(s, name, Strength::Strong, level).unwrap();
                (s, d)
            };
            let clock = external("CLK", Zero);
            let reset = external("RESET", One);
            let enable = external("ENABLE", Zero);
            let data = external("DATA", One);
            let global = external("GLOBAL", Zero);
            let mut a = [Input::Constant(Zero); 18];
            a[0] = data.0.into();
            let mut one = [Input::Constant(Zero); 18];
            one[0] = Input::Constant(One);
            let control = MultiplierRegister {
                clock: clock.0.into(),
                enable: enable.0.into(),
                reset: reset.0.into(),
                reset_kind: kind,
            };
            let out = b
                .multiplier18_with_pipeline(
                    Multiplier18 {
                        resource: ids[1],
                        a,
                        b: one,
                        signed_a: Input::Constant(Zero),
                        signed_b: Input::Constant(Zero),
                        source_a: Input::Constant(Zero),
                        source_b: Input::Constant(Zero),
                    },
                    MultiplierPipeline {
                        both_edges: false,
                        input_a: input_stage.then_some(control),
                        product: (!input_stage).then_some(control),
                        global_reset: Some(global.0.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut k = b.build().unwrap();
            assert_eq!(
                k.value(out.product[0]).unwrap(),
                if kind == ResetKind::Asynchronous {
                    Zero
                } else {
                    X
                }
            );
            let sequence = [
                (vec![(clock.1, One)], Zero), // reset wins despite CE=0
                (vec![(clock.1, Zero), (reset.1, Zero), (enable.1, X)], Zero),
                (vec![(clock.1, One)], X), // possible CE, old zero vs new one
                (vec![(global.1, One)], Zero), // async effective GSR, no clock
                (
                    vec![(global.1, Zero), (clock.1, Zero), (enable.1, One)],
                    Zero,
                ),
                (vec![(clock.1, X)], X), // possible edge
                (vec![(global.1, One), (clock.1, One)], Zero),
                (vec![(global.1, Zero), (clock.1, Zero), (data.1, Z)], Zero),
                (vec![(clock.1, One)], X), // Z sampled as unknown data
                (vec![(global.1, X)], X),
                (vec![(global.1, One)], Zero),
                (vec![(global.1, Zero), (clock.1, Zero), (data.1, One)], Zero),
                (vec![(clock.1, One)], One),
                (
                    vec![(reset.1, X)],
                    if kind == ResetKind::Asynchronous {
                        X
                    } else {
                        One
                    },
                ),
                (
                    vec![(clock.1, Zero)],
                    if kind == ResetKind::Asynchronous {
                        X
                    } else {
                        One
                    },
                ),
                (vec![(clock.1, One)], X), // unknown reset: zero or captured one
                (vec![(global.1, One)], Zero),
                (
                    vec![(reset.1, Zero), (global.1, Zero), (clock.1, Zero)],
                    Zero,
                ),
                (vec![(clock.1, One)], One),
                (vec![(global.1, X)], X), // unknown GSR: held one or reset zero
            ];
            for (t, (updates, expected)) in sequence.into_iter().enumerate() {
                let time = Time::from_ps(t as u64 + 1);
                k.schedule_batch(time, &updates, None).unwrap();
                k.run_until(time).unwrap();
                assert_eq!(k.value(out.product[0]).unwrap(), expected, "{kind:?} t={t}");
                if input_stage {
                    assert_eq!(
                        k.value(out.roa[0]).unwrap(),
                        expected,
                        "{kind:?} t={t} ROA0"
                    );
                }
                for signal in &out.product[1..] {
                    assert_eq!(k.value(*signal).unwrap(), Zero);
                }
            }
        }
    }
}

#[test]
fn multiplier_output_halves_share_one_bank_and_capture_without_overwriting_the_peer() {
    use ecp5_emu::kernel::{MultiplierOutputRegister, MultiplierPipeline, ResetKind};
    let (_f, device, ids) = fixture();
    let mut b = Builder::new(device).unwrap();
    let clocks = [
        b.signal(wire("OUTCLK0")).unwrap(),
        b.signal(wire("OUTCLK1")).unwrap(),
    ];
    let ck = std::array::from_fn::<_, 2, _>(|i| {
        b.external_driver(clocks[i], &format!("outclock{i}"), Strength::Strong, Zero)
            .unwrap()
    });
    let input = b.signal(wire("OUTDATA")).unwrap();
    let data = b
        .external_driver(input, "outdata", Strength::Strong, One)
        .unwrap();
    let mut outputs = Vec::new();
    for i in 0..2 {
        let mut a = bits(262143).map(Input::Constant);
        a[0] = input.into();
        outputs.push(
            b.multiplier18_with_pipeline(
                Multiplier18 {
                    resource: ids[i + 1],
                    a,
                    b: bits(if i == 0 { 0x15555 } else { 0x20001 }).map(Input::Constant),
                    signed_a: Input::Constant(Logic::from_bool(i == 1)),
                    signed_b: Input::Constant(Zero),
                    source_a: Input::Constant(Zero),
                    source_b: Input::Constant(Zero),
                },
                MultiplierPipeline {
                    both_edges: false,
                    output: Some(MultiplierOutputRegister {
                        clock: clocks[i].into(),
                        enable: Input::Constant(One),
                        reset: Input::Constant(Zero),
                        reset_kind: ResetKind::Synchronous,
                    }),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
    }
    let mut k = b.build().unwrap();
    assert_eq!(
        k.state().multiplier_output_view(ids[1]).unwrap(),
        (ids[0], 0..36)
    );
    assert_eq!(
        k.state().multiplier_output_view(ids[2]).unwrap(),
        (ids[0], 36..72)
    );
    assert!(k.state().multiplier_output_view(ids[3]).is_err()); // ALU aliases aren't assigned yet
    assert_eq!(
        k.state().dsp_slice(ids[3]).unwrap().1.shared_output(),
        &[X; 72]
    );
    let mut expected = [[X; 36]; 2];
    let sequence = [
        (One, Zero, One),
        (One, Zero, Zero),
        (Zero, Zero, Zero),
        (One, One, Zero),
        (Zero, One, X),
        (One, One, X),
        (Zero, Zero, One),
        (One, One, One),
    ];
    let mut previous = [Zero; 2];
    for (t, (c0, c1, d)) in sequence.into_iter().enumerate() {
        let clocks = [c0, c1];
        for i in 0..2 {
            if previous[i] == Zero && clocks[i] == One {
                let mut a = bits(262143);
                a[0] = d;
                expected[i] = multiply18(
                    &a,
                    &bits(if i == 0 { 0x15555 } else { 0x20001 }),
                    Logic::from_bool(i == 1),
                    Zero,
                );
            }
        }
        previous = clocks;
        let time = Time::from_ps(t as u64 + 1);
        k.schedule_batch(time, &[(ck[0], c0), (ck[1], c1), (data, d)], None)
            .unwrap();
        k.run_until(time).unwrap();
        for i in 0..2 {
            assert_eq!(
                k.state().multiplier_output(ids[i + 1]).unwrap(),
                expected[i],
                "t={t} lane={i}"
            );
            for (bit, &value) in expected[i].iter().enumerate() {
                assert_eq!(k.value(outputs[i].product[bit]).unwrap(), value);
            }
            assert_eq!(k.value(outputs[i].signed).unwrap(), X); // unresolved output SIGNEDP
        }
        let bank = k.state().dsp_slice(ids[3]).unwrap().1.shared_output();
        assert_eq!(&bank[..36], &expected[0]);
        assert_eq!(&bank[36..], &expected[1]);
    }
    assert_eq!(k.state().summary().known_dsp_shared_output_bits, 72);
    assert!(k.statistics().dsp_output_register_bit_changes >= 72);
}

#[test]
fn output_register_preserves_the_latency_of_every_input_pipeline_combination() {
    use ecp5_emu::kernel::{
        MultiplierOutputRegister, MultiplierPipeline, MultiplierRegister, ResetKind,
    };
    for mask in 0..8 {
        let (_f, device, ids) = fixture();
        let mut b = Builder::new(device).unwrap();
        let mut ext = |name: &str| {
            let s = b.signal(wire(name)).unwrap();
            let d = b.external_driver(s, name, Strength::Strong, Zero).unwrap();
            (Input::from(s), d)
        };
        let aa = (0..18).map(|n| ext(&format!("A{n}"))).collect::<Vec<_>>();
        let bb = (0..18).map(|n| ext(&format!("B{n}"))).collect::<Vec<_>>();
        let sa = ext("SA");
        let sb = ext("SB");
        let clocks = (0..4).map(|n| ext(&format!("CLK{n}"))).collect::<Vec<_>>();
        let stages = std::array::from_fn::<_, 3, _>(|n| {
            (mask & (1 << n) != 0).then_some(MultiplierRegister {
                clock: clocks[n].0,
                enable: Input::Constant(One),
                reset: Input::Constant(Zero),
                reset_kind: ResetKind::Synchronous,
            })
        });
        let out = b
            .multiplier18_with_pipeline(
                Multiplier18 {
                    resource: ids[1],
                    a: std::array::from_fn(|n| aa[n].0),
                    b: std::array::from_fn(|n| bb[n].0),
                    signed_a: sa.0,
                    signed_b: sb.0,
                    source_a: Input::Constant(Zero),
                    source_b: Input::Constant(Zero),
                },
                MultiplierPipeline {
                    both_edges: false,
                    input_a: stages[0],
                    input_b: stages[1],
                    product: stages[2],
                    output: Some(MultiplierOutputRegister {
                        clock: clocks[3].0,
                        enable: Input::Constant(One),
                        reset: Input::Constant(Zero),
                        reset_kind: ResetKind::Synchronous,
                    }),
                    global_reset: None,
                },
            )
            .unwrap();
        let mut k = b.build().unwrap();
        let mut ar = None;
        let mut br = None;
        let mut pr = None;
        let mut qr = None;
        let mut previous = [false; 4];
        let mut random = 0x172389ab563u64;
        let numeric = |a: Option<(u64, bool)>, b: Option<(u64, bool)>| {
            a.zip(b).map(|((a, sa), (b, sb))| {
                let operand = |v: u64, s| v as i128 - if s && v >= 131072 { 262144 } else { 0 };
                (operand(a, sa) * operand(b, sb)).rem_euclid(1i128 << 36) as u64
            })
        };
        for t in 0..257u64 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = random & 262143;
            let bbv = (random >> 32) & 262143;
            let asigned = t & 2 != 0;
            let bsigned = t & 4 != 0;
            let clk = [
                t & 1 != 0,
                (t / 3) & 1 != 0,
                (t / 5) & 1 != 0,
                (t / 7) & 1 != 0,
            ];
            let edges = std::array::from_fn::<_, 4, _>(|n| !previous[n] && clk[n]);
            let av = if mask & 1 != 0 {
                ar
            } else {
                Some((a, asigned))
            };
            let bv = if mask & 2 != 0 {
                br
            } else {
                Some((bbv, bsigned))
            };
            if edges[3] {
                qr = if mask & 4 != 0 { pr } else { numeric(av, bv) };
            }
            if mask & 4 != 0 && edges[2] {
                pr = numeric(av, bv);
            }
            if mask & 1 != 0 && edges[0] {
                ar = Some((a, asigned));
            }
            if mask & 2 != 0 && edges[1] {
                br = Some((bbv, bsigned));
            }
            previous = clk;
            let mut updates = Vec::new();
            for n in 0..18 {
                updates.push((aa[n].1, Logic::from_bool(a & (1 << n) != 0)));
                updates.push((bb[n].1, Logic::from_bool(bbv & (1 << n) != 0)));
            }
            updates.extend([
                (sa.1, Logic::from_bool(asigned)),
                (sb.1, Logic::from_bool(bsigned)),
            ]);
            for n in 0..4 {
                updates.push((clocks[n].1, Logic::from_bool(clk[n])));
            }
            let time = Time::from_ps(t + 1);
            k.schedule_batch(time, &updates, None).unwrap();
            k.run_until(time).unwrap();
            for (signals, operand) in [
                (
                    out.roa,
                    if mask & 1 != 0 {
                        ar
                    } else {
                        Some((a, asigned))
                    },
                ),
                (
                    out.rob,
                    if mask & 2 != 0 {
                        br
                    } else {
                        Some((bbv, bsigned))
                    },
                ),
            ] {
                for (bit, signal) in signals.into_iter().enumerate() {
                    assert_eq!(
                        k.value(signal).unwrap(),
                        operand.map_or(X, |(v, _)| Logic::from_bool(v & (1 << bit) != 0)),
                        "mask={mask} t={t} forwarding bit={bit}"
                    );
                }
            }
            if let Some(word) = qr {
                let expected =
                    std::array::from_fn::<_, 36, _>(|n| Logic::from_bool(word & (1 << n) != 0));
                assert_eq!(
                    k.state().multiplier_output(ids[1]).unwrap(),
                    expected,
                    "mask={mask} t={t}"
                );
                for (signal, value) in out.product.into_iter().zip(expected) {
                    assert_eq!(k.value(signal).unwrap(), value);
                }
            }
            assert_eq!(k.state().multiplier_output(ids[2]).unwrap(), [X; 36]);
        }
    }
}

#[test]
fn output_stage_follows_its_enable_its_reset_and_a_global_reset() {
    // HD01: the output stage captures under its enable (CE0 in the plan) and
    // clears at a rising edge under a synchronous reset; unknown controls
    // merge as for the other stages. A global reset on it is not measured.
    use ecp5_emu::kernel::{MultiplierOutputRegister, MultiplierPipeline, ResetKind};
    let (_f, device, ids) = fixture();
    let mut b = Builder::new(device).unwrap();
    let input = |b: &mut Builder, name: &str, value: Logic| {
        let s = b.signal(wire(name)).unwrap();
        let d = b.external_driver(s, name, Strength::Strong, value).unwrap();
        (Input::from(s), d)
    };
    let (clock, ck) = input(&mut b, "CK", Zero);
    let (enable, ce) = input(&mut b, "CE", One);
    let (reset, rst) = input(&mut b, "RST", Zero);
    let (global, gsr) = input(&mut b, "GSR", Zero);
    let config = Multiplier18 {
        resource: ids[1],
        a: bits(3).map(Input::Constant),
        b: bits(5).map(Input::Constant),
        signed_a: Input::Constant(Zero),
        signed_b: Input::Constant(Zero),
        source_a: Input::Constant(Zero),
        source_b: Input::Constant(Zero),
    };
    let output = MultiplierOutputRegister {
        clock,
        enable,
        reset,
        reset_kind: ResetKind::Synchronous,
    };
    let out = b
        .multiplier18_with_pipeline(
            config,
            MultiplierPipeline {
                both_edges: false,
                output: Some(output),
                global_reset: Some(global),
                ..Default::default()
            },
        )
        .unwrap();
    let mut k = b.build().unwrap();
    let p0 = |k: &Kernel| k.value(out.product[0]).unwrap();
    let p4 = |k: &Kernel| k.value(out.product[4]).unwrap();
    assert_eq!(p0(&k), X);
    let mut t = 0;
    let mut step = |k: &mut Kernel, d: DriverId, v: Logic| {
        t += 1;
        k.schedule(Time::from_ps(t), d, v).unwrap();
        k.run_until(Time::from_ps(t)).unwrap();
    };
    // 3 * 5 = 15: bits 0..3 one, bit 4 zero.
    step(&mut k, ck, One);
    assert_eq!((p0(&k), p4(&k)), (One, Zero));
    step(&mut k, ck, Zero);
    step(&mut k, rst, One);
    assert_eq!(p0(&k), One, "a synchronous reset waits for the edge");
    step(&mut k, ck, One);
    assert_eq!(p0(&k), Zero, "cleared at the edge");
    step(&mut k, ck, Zero);
    step(&mut k, rst, Zero);
    step(&mut k, ce, Zero);
    step(&mut k, ck, One);
    assert_eq!(p0(&k), Zero, "CE low holds");
    step(&mut k, ck, Zero);
    step(&mut k, ce, X);
    step(&mut k, ck, One);
    assert_eq!(p0(&k), X, "an unknown CE merges held and captured");
    step(&mut k, ck, Zero);
    step(&mut k, ce, One);
    step(&mut k, ck, One);
    assert_eq!(p0(&k), One);
    // GSR clears the output stage at once and holds it through an edge (HD07).
    step(&mut k, ck, Zero);
    step(&mut k, gsr, One);
    assert_eq!(p0(&k), Zero, "GSR clears at once");
    step(&mut k, ck, One);
    assert_eq!(p0(&k), Zero, "GSR holds through an edge");
    step(&mut k, ck, Zero);
    step(&mut k, gsr, Zero);
    assert_eq!(p0(&k), Zero, "released, it waits for an edge");
    step(&mut k, ck, One);
    assert_eq!(p0(&k), One);
}

#[test]
fn parallel_operand_forwarding_preserves_each_bit_including_unknowns() {
    let (_f, device, ids) = fixture();
    let mut b = Builder::new(device).unwrap();
    let data = b.signal(wire("FORWARD_DATA")).unwrap();
    let driver = b
        .external_driver(data, "forward-data", Strength::Strong, Zero)
        .unwrap();
    let a = std::array::from_fn(|n| Input::Constant([Zero, One, X, Z][n % 4]));
    let mut bv = std::array::from_fn(|n| Input::Constant([Z, X, One, Zero][n % 4]));
    bv[17] = data.into();
    let out = b
        .multiplier18(Multiplier18 {
            resource: ids[1],
            a,
            b: bv,
            signed_a: Input::Constant(X),
            signed_b: Input::Constant(Z),
            source_a: Input::Constant(Zero),
            source_b: Input::Constant(Zero),
        })
        .unwrap();
    let mut k = b.build().unwrap();
    for (t, value) in [Zero, One, X, Z].into_iter().enumerate() {
        let time = Time::from_ps(t as u64 + 1);
        k.schedule(time, driver, value).unwrap();
        k.run_until(time).unwrap();
        for bit in 0..18 {
            assert_eq!(k.value(out.roa[bit]).unwrap(), [Zero, One, X, X][bit % 4]);
            let expected = if bit == 17 {
                if value == Z {
                    X
                } else {
                    value
                }
            } else {
                [X, X, One, Zero][bit % 4]
            };
            assert_eq!(k.value(out.rob[bit]).unwrap(), expected);
        }
        assert_eq!(
            k.state().dsp_slice(ids[1]).unwrap().1.shared_output(),
            &[X; 72]
        );
    }
}

#[test]
fn operand_only_binding_has_no_product_drivers_or_shared_output_capture() {
    use ecp5_emu::kernel::{MultiplierPipeline, MultiplierRegister, ResetKind};
    let (_f, device, ids) = fixture();
    let mut b = Builder::new(device).unwrap();
    let p = b.signal(wire("MULT18_0_P0")).unwrap();
    let signed = b.signal(wire("MULT18_0_SIGNEDP")).unwrap();
    let ck = b.signal(wire("OPERAND_CLOCK")).unwrap();
    let clock = b
        .external_driver(ck, "operand-clock", Strength::Strong, Zero)
        .unwrap();
    let control = MultiplierRegister {
        clock: ck.into(),
        enable: Input::Constant(One),
        reset: Input::Constant(Zero),
        reset_kind: ResetKind::Synchronous,
    };
    let config = Multiplier18 {
        resource: ids[1],
        a: bits(1).map(Input::Constant),
        b: bits(2).map(Input::Constant),
        signed_a: Input::Constant(Zero),
        signed_b: Input::Constant(Zero),
        source_a: Input::Constant(Zero),
        source_b: Input::Constant(Zero),
    };
    assert!(b
        .multiplier18_operands(
            config.clone(),
            MultiplierPipeline {
                both_edges: false,
                product: Some(control),
                ..Default::default()
            }
        )
        .is_err());
    let out = b
        .multiplier18_operands(
            config.clone(),
            MultiplierPipeline {
                both_edges: false,
                input_a: Some(control),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(b.multiplier18(config).is_err()); // one owner even across execution views
    let mut k = b.build().unwrap();
    assert_eq!(k.value(out.roa[0]).unwrap(), X);
    assert_eq!(k.value(out.rob[1]).unwrap(), One);
    k.schedule(Time::from_ps(1), clock, One).unwrap();
    k.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(k.value(out.roa[0]).unwrap(), One);
    assert_eq!(k.value(p).unwrap(), Z);
    assert_eq!(k.value(signed).unwrap(), Z);
    assert_eq!(k.state().multiplier(ids[1]).unwrap().product, [X; 36]);
    assert_eq!(
        k.state().dsp_slice(ids[1]).unwrap().1.shared_output(),
        &[X; 72]
    );
    assert_eq!(k.statistics().dsp_output_register_bit_changes, 0);
}

#[test]
fn forwarded_input_and_product_stages_capture_old_values_on_the_same_edge() {
    use ecp5_emu::kernel::{
        Edge, FlipFlop, MultiplierOutputRegister, MultiplierPipeline, MultiplierRegister, ResetKind,
    };
    let (_f, device, ids) = fixture();
    let plc = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
    let ff = device
        .model()
        .nodes(plc)
        .find(|n| n.kind == "TRELLIS_FF")
        .unwrap()
        .id;
    let mut b = Builder::new(device).unwrap();
    let clock = b.signal(wire("FORWARD_CLOCK")).unwrap();
    let ck = b
        .external_driver(clock, "forward-clock", Strength::Strong, One)
        .unwrap();
    let data = b.signal(wire("FORWARD_DATA")).unwrap();
    let driver = b
        .external_driver(data, "forward-data", Strength::Strong, One)
        .unwrap();
    let control = MultiplierRegister {
        clock: clock.into(),
        enable: Input::Constant(One),
        reset: Input::Constant(Zero),
        reset_kind: ResetKind::Synchronous,
    };
    let mut a = bits(0).map(Input::Constant);
    a[0] = data.into();
    let out = b
        .multiplier18_with_pipeline(
            Multiplier18 {
                resource: ids[1],
                a,
                b: bits(1).map(Input::Constant),
                signed_a: Input::Constant(Zero),
                signed_b: Input::Constant(Zero),
                source_a: Input::Constant(Zero),
                source_b: Input::Constant(Zero),
            },
            MultiplierPipeline {
                both_edges: false,
                input_a: Some(control),
                product: Some(control),
                output: Some(MultiplierOutputRegister {
                    clock: clock.into(),
                    enable: Input::Constant(One),
                    reset: Input::Constant(Zero),
                    reset_kind: ResetKind::Synchronous,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let captured = b
        .flip_flop(FlipFlop {
            preload: None,
            node: ff,
            data: out.roa[0].into(),
            clock: clock.into(),
            enable: Input::Constant(One),
            edge: Edge::Rising,
            reset: None,
        })
        .unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(out.roa[0]).unwrap(), X); // initially high clock is not an edge
    assert_eq!(k.value(out.rob[0]).unwrap(), One);
    assert_eq!(k.value(out.product[0]).unwrap(), X);
    for (edge, (value, ra, q, p)) in [
        (One, One, X, X),
        (Zero, Zero, One, X),
        (Z, X, Zero, One),
        (One, One, X, Zero),
    ]
    .into_iter()
    .enumerate()
    {
        let low = Time::from_ps(edge as u64 * 2 + 1);
        k.schedule(low, ck, Zero).unwrap();
        k.run_until(low).unwrap();
        let high = Time::from_ps(edge as u64 * 2 + 2);
        k.schedule_batch(high, &[(ck, One), (driver, value)], None)
            .unwrap();
        k.run_until(high).unwrap();
        assert_eq!(k.value(out.roa[0]).unwrap(), ra, "edge={edge}");
        assert_eq!(k.value(captured).unwrap(), q, "edge={edge} FF");
        assert_eq!(k.value(out.product[0]).unwrap(), p, "edge={edge} P");
        assert_eq!(k.value(out.rob[0]).unwrap(), One);
    }
}

#[test]
fn hd09_disabled_multiplier_stages_capture_random_traffic_on_both_edges() {
    use ecp5_emu::kernel::{
        MultiplierOutputRegister, MultiplierPipeline, MultiplierRegister, ResetKind,
    };
    let (_f, d, ids) = fixture();
    let mut b = Builder::new(d).unwrap();
    let mut drivers = Vec::new();
    let mut input = |b: &mut Builder, name: String, value: Logic| {
        let s = b.signal(wire(&name)).unwrap();
        drivers.push(
            b.external_driver(s, &name, Strength::Strong, value)
                .unwrap(),
        );
        Input::from(s)
    };
    let a = std::array::from_fn(|i| input(&mut b, format!("A{i}"), Zero));
    let bb = std::array::from_fn(|i| input(&mut b, format!("B{i}"), Zero));
    let clock = input(&mut b, "CLK".into(), Zero);
    let enable = input(&mut b, "CE".into(), One);
    let reset = input(&mut b, "RST".into(), Zero);
    let r = MultiplierRegister {
        clock,
        enable,
        reset,
        reset_kind: ResetKind::Synchronous,
    };
    let out = b
        .multiplier18_with_pipeline(
            Multiplier18 {
                resource: ids[1],
                a,
                b: bb,
                signed_a: Input::Constant(Zero),
                signed_b: Input::Constant(Zero),
                source_a: Input::Constant(Zero),
                source_b: Input::Constant(Zero),
            },
            MultiplierPipeline {
                both_edges: true,
                input_a: Some(r),
                input_b: Some(r),
                product: Some(r),
                output: Some(MultiplierOutputRegister {
                    clock,
                    enable,
                    reset,
                    reset_kind: r.reset_kind,
                }),
                global_reset: None,
            },
        )
        .unwrap();
    let mut kernel = b.build().unwrap();
    let mut state = [0_u64; 4];
    let mut fast = None;
    let mut seed = 909_u64;
    for edge in 1..=1000 {
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let av = next() & 262143;
        let bv = next() & 262143;
        let ce = next() & 3 != 0;
        let rst = next() & 7 == 0 || edge == 1;
        let batch: Vec<_> = drivers
            .iter()
            .enumerate()
            .map(|(i, &d)| {
                (
                    d,
                    Logic::from_bool(match i {
                        0..18 => av >> i & 1 != 0,
                        18..36 => bv >> (i - 18) & 1 != 0,
                        36 => edge % 2 == 1,
                        37 => ce,
                        _ => rst,
                    }),
                )
            })
            .collect();
        let want = if rst {
            [0; 4]
        } else if ce {
            [av, bv, state[0] * state[1], state[2]]
        } else {
            state
        };
        let time = Time::from_ps(edge);
        let set: Vec<_> = batch.iter().map(|&(d, v)| (d, v.bit().unwrap())).collect();
        kernel.schedule_batch(time, &batch, None).unwrap();
        kernel.run_until(time).unwrap();
        state = want;
        let word = |pins: &[ecp5_emu::kernel::SignalId]| {
            pins.iter().enumerate().fold(0, |w, (i, &s)| {
                w | ((kernel.value(s).unwrap().bit().unwrap() as u64) << i)
            })
        };
        assert_eq!(word(&out.roa), state[0], "edge {edge}");
        assert_eq!(word(&out.rob), state[1], "edge {edge}");
        assert_eq!(word(&out.product), state[3], "edge {edge}");
        if let Some(fast) = &mut fast {
            let fast: &mut ecp5_emu::fast::FastSim = fast;
            fast.set_at(time, &set).unwrap();
            for &s in out.roa.iter().chain(&out.rob).chain(&out.product) {
                assert_eq!(fast.value(s), Some(kernel.value(s).unwrap()), "edge {edge}");
            }
        } else {
            fast = Some(ecp5_emu::fast::FastSim::new(&kernel).unwrap());
        }
    }
}
