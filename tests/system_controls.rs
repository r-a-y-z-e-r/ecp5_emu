mod common;
use common::Fixture;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    decode::Catalog,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        Builder, Edge, FlipFlop, GlobalReset, Input, LutRam, Options, Strength, SystemConfig,
        SystemPort, Time,
    },
    logic::Logic::{self, *},
    model::{
        topology::{Location, Wire},
        NodeId,
    },
    prepared::ConfiguredDevice,
    state::DeviceState,
};
use serde_json::json;
use std::sync::Arc;
fn w(name: &str) -> Reference {
    Reference::Wire(Wire {
        location: Location { row: 1, col: 1 },
        name: name.into(),
    })
}
fn sys(name: &str) -> Reference {
    Reference::System(SystemPort::parse(name).unwrap())
}
fn bit(data: &mut [u8; 48], frame: usize, bit: usize, value: bool) {
    let offset = bit + 3;
    let byte = &mut data[(frame + 2) * 4 + offset / 8];
    if value {
        *byte |= 1 << (offset % 8);
    } else {
        *byte &= !(1 << (offset % 8));
    }
}
fn image_data(sequence4: bool) -> [u8; 48] {
    let mut data = [0; 48];
    for frame in 0..8 {
        for b in 0..16 {
            bit(&mut data, frame, b, true);
        }
    }
    bit(&mut data, 8, 0, true);
    data[2] = if sequence4 { 0x20 } else { 0x18 };
    if sequence4 {
        data[3] = 1;
    }
    data
}
fn configured(f: &Fixture, data: [u8; 48]) -> Arc<ConfiguredDevice> {
    let fields = vec![
        json!({"kind":"enum","name":"GSR.GSRMODE","fragments":[{"tile":"SYS0_R1C1:EFB0_PICB0","source":"fixture","default":"NONE","options":{"NONE":[],"ACTIVE_HIGH":[[0,16,true]],"ACTIVE_LOW":[[0,16,true],[0,17,true]]}}]}),
        json!({"kind":"enum","name":"GSR.SYNCMODE","fragments":[{"tile":"SYS0_R1C1:EFB0_PICB0","source":"fixture","default":"ASYNC","options":{"ASYNC":[],"NONE":[],"SYNC":[[0,18,true]]}}]}),
    ];
    let mut pio = Vec::new();
    for (name, options) in [
        ("BASE_TYPE", json!({"BIDIR_LVCMOS33":[[0,4,false]]})),
        ("PULLMODE", json!({"UP":[[0,5,false]]})),
        ("OPENDRAIN", json!({"OFF":[[0,6,false]]})),
        ("DATAMUX_OREG", json!({"PADDO":[],"IOLDO":[[1,4,true]]})),
        ("TRIMUX_TSREG", json!({"PADDT":[],"IOLTO":[[1,5,true]]})),
    ] {
        pio.push(json!({"kind":"enum","name":format!("PIOA.{name}"),"fragments":[{"tile":"CIB_R1C1:CIB","source":"fixture","default":null,"options":options}]}));
    }
    let catalogue=Catalog::merge_documents(&[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[{"id":"LFE5U-85F/R1C1/GSR","family":"GSR","fields":fields},{"id":"LFE5U-85F/R1C1/PIOA","family":"PIO","fields":pio}]})).unwrap()],"LFE5U-85F",12,32).unwrap();
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F","frames":12,"bits_per_frame":32,
        "encoding":"frame_byte_lsb0","cram_hex":data.iter().map(|b|format!("{b:02x}")).collect::<String>(),"bram_data":{},"origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,"native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap())
}
fn request(startup: Startup) -> Request {
    let mut drives = vec![
        (w("CLK0"), Zero),
        (w("LSR0"), Zero),
        (w("CLK1"), Zero),
        (w("LSR1"), One),
        (w("JGSR_GSR"), Zero),
        (w("M0_SLICE"), One),
        (w("M2_SLICE"), One),
    ];
    for n in 0..6 {
        for p in ["A", "B", "C", "D"] {
            drives.push((w(&format!("{p}{n}_SLICE")), if n == 5 { One } else { Zero }));
        }
    }
    Request {
        package: "CABGA381".into(),
        scope: Scope::Observed,
        startup,
        drives,
        probes: vec![
            w("Q0_SLICE"),
            w("Q2_SLICE"),
            w("F0_SLICE"),
            sys("goe"),
            sys("gwdis"),
            sys("gsr"),
            sys("ready"),
            sys("done"),
        ],
        options: Options::default(),
    }
}
fn apply(c: &mut Compiled, time: u64, values: Vec<(Reference, Logic)>) {
    let updates = values
        .into_iter()
        .map(|(key, v)| (c.drivers[&key.label()], v))
        .collect::<Vec<_>>();
    c.kernel
        .schedule_batch(Time::from_ps(time), &updates, None)
        .unwrap();
    c.kernel.run_until(Time::from_ps(time)).unwrap();
}
fn get(c: &Compiled, r: Reference) -> Logic {
    c.value(&r).unwrap()
}
fn node(device: &ConfiguredDevice, name: &str) -> NodeId {
    device
        .model()
        .nodes(device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap())
        .find(|n| n.native_name == name)
        .unwrap()
        .id
}

#[test]
fn explicit_wake_phases_reset_only_enabled_ff_and_release_memory_before_capture() {
    let f = Fixture::with_system();
    for seq4 in [false, true] {
        let mut data = image_data(seq4);
        bit(&mut data, 9, 6, true); // A disabled, B enabled
        let mut req = request(Startup::WakePhases);
        req.drives.push((sys("step"), Zero));
        if seq4 {
            req.drives.push((sys("done"), Zero));
        }
        let mut c = compile::compile(configured(&f, data), req).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(get(&c, w("Q0_SLICE")), X);
        assert_eq!(get(&c, w("Q2_SLICE")), Zero);
        assert_eq!(get(&c, sys("goe")), Zero);
        assert_eq!(get(&c, sys("gwdis")), One);
        apply(&mut c, 1, vec![(sys("step"), One)]); // T0
        apply(&mut c, 2, vec![(sys("step"), Zero), (w("CLK1"), One)]);
        assert_eq!(get(&c, w("F0_SLICE")), Zero); // inhibited write
        apply(&mut c, 3, vec![(sys("step"), One)]); // T1
        assert_eq!(get(&c, sys("goe")), One);
        assert_eq!(get(&c, sys("gsr")), One);
        apply(
            &mut c,
            4,
            vec![(sys("step"), Zero), (w("CLK0"), One), (w("CLK1"), Zero)],
        );
        assert_eq!(get(&c, w("Q0_SLICE")), One);
        assert_eq!(get(&c, w("Q2_SLICE")), Zero);
        apply(&mut c, 5, vec![(sys("step"), One), (w("CLK1"), One)]); // T2 plus write edge
        assert_eq!(get(&c, sys("gsr")), Zero);
        assert_eq!(get(&c, sys("gwdis")), Zero);
        assert_eq!(get(&c, w("F0_SLICE")), One);
        apply(&mut c, 6, vec![(sys("step"), Zero), (w("CLK0"), Zero)]);
        apply(&mut c, 7, vec![(sys("step"), One), (w("CLK0"), One)]); // T3
        assert_eq!(get(&c, w("Q2_SLICE")), One);
        assert_eq!(get(&c, sys("ready")), if seq4 { Zero } else { One });
        if seq4 {
            apply(&mut c, 8, vec![(sys("done"), Z)]);
            assert_eq!(get(&c, sys("done")), One);
            assert_eq!(get(&c, sys("ready")), One);
            apply(&mut c, 9, vec![(sys("done"), Zero)]);
            assert_eq!(get(&c, sys("ready")), One); // completion is latched
        }
        assert_eq!(c.kernel.state().system().unwrap().phase, Some(3));
    }
}

#[test]
fn configured_profile_starts_flip_flops_at_regset_whatever_slice_gsr_says() {
    // Hardware series HS01 (docs/inventory/LFE5U-85F/hs01_init_slice): on the
    // board, flip-flops took REGSET at configuration with SLICE GSR enabled and
    // disabled alike.
    let f = Fixture::with_system();
    let mut data = image_data(false);
    bit(&mut data, 9, 6, true); // A disabled, B enabled
    bit(&mut data, 9, 7, true); // A REG0 REGSET=SET; B keeps RESET
    let request = |startup| {
        let mut req = request(startup);
        // B's local reset stays inactive, so only the profile decides.
        for (key, value) in &mut req.drives {
            if *key == w("LSR1") {
                *value = Zero;
            }
        }
        req
    };
    let c = compile::compile(configured(&f, data), request(Startup::Configured))
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(c.report.system.gsr_disabled_ff, 1);
    assert_eq!(c.report.system.gsr_enabled_ff, 1);
    assert!(c.report.system.flip_flop_initial.starts_with("REGSET"));
    assert_eq!(get(&c, w("Q0_SLICE")), One);
    assert_eq!(get(&c, w("Q2_SLICE")), Zero);
    assert_eq!(get(&c, sys("goe")), One);
    assert_eq!(get(&c, sys("gwdis")), Zero);
    assert_eq!(get(&c, sys("gsr")), Zero);
    let c = compile::compile(configured(&f, data), request(Startup::ReadyUnknown))
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(get(&c, w("Q0_SLICE")), X);
    assert_eq!(get(&c, w("Q2_SLICE")), X);
}

#[test]
fn asynchronous_user_gsr_is_a_root_respects_polarity_and_never_clears_ram() {
    let f = Fixture::with_system();
    for active_low in [false, true] {
        let mut data = image_data(false);
        data[2] |= 1;
        if active_low {
            data[2] |= 2;
        }
        bit(&mut data, 9, 6, true);
        let mut req = request(Startup::ReadyUnknown);
        req.drives[4].1 = Logic::from_bool(active_low);
        let mut c = compile::compile(configured(&f, data), req).unwrap_or_else(|e| panic!("{e:#}"));
        assert!(c.report.roots.iter().any(|r| r.ends_with("/GSR:GSR")));
        assert_eq!(get(&c, w("Q2_SLICE")), X);
        apply(&mut c, 1, vec![(w("CLK0"), One), (w("CLK1"), One)]);
        assert_eq!(get(&c, w("Q2_SLICE")), One);
        assert_eq!(get(&c, w("F0_SLICE")), One);
        apply(
            &mut c,
            2,
            vec![(w("JGSR_GSR"), Logic::from_bool(!active_low))],
        );
        assert_eq!(get(&c, w("Q2_SLICE")), Zero); // clock is stopped high
        assert_eq!(get(&c, w("Q0_SLICE")), One); // GSR disabled
        assert_eq!(get(&c, w("F0_SLICE")), One); // SRAM preserved
        assert_eq!(get(&c, sys("gwdis")), Zero); // user GSR doesn't inhibit writes
        apply(&mut c, 3, vec![(w("CLK1"), Zero), (w("C5_SLICE"), Zero)]);
        apply(&mut c, 4, vec![(w("CLK1"), One)]);
        assert_eq!(get(&c, w("F0_SLICE")), Zero); // writes continue during user reset
        apply(&mut c, 5, vec![(w("JGSR_GSR"), Z)]);
        assert_eq!(get(&c, w("Q2_SLICE")), Zero); // both reset/hold agree
        apply(&mut c, 6, vec![(w("CLK0"), Zero)]);
        apply(&mut c, 7, vec![(w("CLK0"), One)]);
        assert_eq!(get(&c, w("Q2_SLICE")), X); // reset versus new data disagree
        apply(
            &mut c,
            8,
            vec![
                (w("JGSR_GSR"), Logic::from_bool(active_low)),
                (w("CLK0"), Zero),
            ],
        );
        apply(&mut c, 9, vec![(w("CLK0"), One)]);
        assert_eq!(get(&c, w("Q2_SLICE")), One);
    }
}

#[test]
fn goe_blocks_own_pad_driver_while_external_input_and_pull_remain_active() {
    let f = Fixture::with_system();
    let path = f.evidence.join("iologic_research/resource_ports.csv");
    let mut ports = std::fs::read_to_string(&path).unwrap();
    ports.push_str(
        "LFE5U-85F/R1C1/PIOA,I,in,1,1,PADDOA_PIO\nLFE5U-85F/R1C1/PIOA,T,in,1,1,PADDTA_PIO\n",
    );
    common::write(&path, ports);
    f.seal();
    let mut req = request(Startup::WakePhases);
    let pin = Reference::Pin("G1".into());
    req.probes.extend([pin.clone(), w("JPADDIA_PIO")]);
    req.drives.extend([
        (sys("step"), Zero),
        (pin.clone(), Z),
        (w("PADDOA_PIO"), One),
        (w("PADDTA_PIO"), Zero),
    ]);
    let mut c = compile::compile(configured(&f, image_data(false)), req)
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(get(&c, pin.clone()), One); // weak pull, strong output blocked
    apply(&mut c, 1, vec![(sys("step"), One), (pin.clone(), Zero)]);
    assert_eq!(get(&c, pin.clone()), Zero);
    assert_eq!(get(&c, w("JPADDIA_PIO")), Zero);
    apply(&mut c, 2, vec![(sys("step"), Zero)]);
    apply(&mut c, 3, vec![(sys("step"), One)]); // GOE high at T1
    assert_eq!(get(&c, pin.clone()), X);
    assert_eq!(get(&c, w("JPADDIA_PIO")), X);
    apply(&mut c, 4, vec![(pin.clone(), Z)]);
    assert_eq!(get(&c, pin), One);
}

#[test]
fn unknown_done_defers_certain_completion_and_initial_high_step_creates_no_phase() {
    let f = Fixture::with_system();
    let mut req = request(Startup::WakePhases);
    req.drives.extend([(sys("step"), One), (sys("done"), X)]);
    let mut c = compile::compile(configured(&f, image_data(true)), req).unwrap();
    assert_eq!(c.kernel.state().system().unwrap().phase, None);
    for t in 1..=8 {
        apply(
            &mut c,
            t,
            vec![(sys("step"), if t % 2 == 0 { One } else { Zero })],
        );
    }
    assert_eq!(get(&c, sys("ready")), X);
    apply(&mut c, 9, vec![(sys("done"), Zero)]);
    assert_eq!(get(&c, sys("ready")), X); // completion might have happened earlier
    apply(&mut c, 10, vec![(sys("done"), Z)]);
    assert_eq!(get(&c, sys("ready")), One);
    apply(&mut c, 11, vec![(sys("done"), Zero)]);
    assert_eq!(get(&c, sys("ready")), One);
}

#[test]
fn unmeasured_sync_gsr_profiles_custom_phases_and_readonly_drives_are_rejected() {
    let f = Fixture::with_system();
    for case in [
        "sync-active-high",
        "sync-mode-none",
        "phase",
        "fragment",
        "readonly",
        "ready-step",
    ] {
        let mut data = image_data(false);
        let mut req = request(Startup::WakePhases);
        match case {
            "sync-active-high" => data[2] |= 5,
            "sync-mode-none" => data[2] |= 4,
            "phase" => data[2] |= 64,
            "fragment" => data[3] |= 1,
            "readonly" => req.drives.push((sys("gsr"), One)),
            "ready-step" => {
                req.startup = Startup::ReadyUnknown;
                req.drives.push((sys("step"), Zero));
            }
            _ => unreachable!(),
        }
        let error = compile::compile(configured(&f, data), req)
            .err()
            .expect("unsupported system mode accepted");
        let text = format!("{error:#}");
        assert!(
            text.contains(match case {
                "sync-active-high" | "sync-mode-none" => "HY01-measured ACTIVE_LOW polarity",
                "phase" => "custom wake phase",
                "fragment" => "DONE_EX fragments",
                _ => "read-only or unavailable",
            }),
            "{case}: {text}"
        );
    }
}

#[test]
fn global_reset_priority_and_unknown_write_inhibit_have_conservative_kernel_semantics() {
    let f = Fixture::with_system();
    let device = configured(&f, image_data(false));
    let lut = node(&device, "SLICEA.K0");
    let ff = node(&device, "SLICEA.FF0");
    let mut state = DeviceState::new(&device).unwrap();
    state.write_register(ff, One).unwrap();
    let mut b = Builder::with_state(Arc::clone(&device), state).unwrap();
    let make = |b: &mut Builder, name: &str, value: Logic| {
        let Reference::Wire(wire) = w(name) else {
            unreachable!()
        };
        let s = b.signal(wire).unwrap();
        (
            s,
            b.external_driver(s, name, Strength::Strong, value).unwrap(),
        )
    };
    let (clock, ck) = make(&mut b, "CK", Zero);
    let (inhibit, wd) = make(&mut b, "INHIBIT", X);
    let (reset, rst) = make(&mut b, "GLOBAL", One);
    let output = b
        .lut_ram_with_inhibit(
            LutRam {
                node: lut,
                read_address: [Input::Constant(Zero); 4],
                write_address: [Input::Constant(Zero); 4],
                data: Input::Constant(One),
                clock: clock.into(),
                enable: Input::Constant(One),
                edge: Edge::Rising,
            },
            inhibit.into(),
        )
        .unwrap();
    let q = b
        .flip_flop_with_global_reset(
            FlipFlop {
                preload: None,
                node: ff,
                data: Input::Constant(One),
                clock: clock.into(),
                enable: Input::Constant(Zero),
                edge: Edge::Rising,
                reset: Some(ecp5_emu::kernel::Reset {
                    input: Input::Constant(One),
                    kind: ecp5_emu::kernel::ResetKind::Asynchronous,
                    value: One,
                }),
            },
            Some(GlobalReset {
                input: reset.into(),
                value: Zero,
            }),
        )
        .unwrap();
    let mut k = b.build().unwrap();
    assert_eq!(k.value(q).unwrap(), Zero); // global precedes local and CE
    k.schedule(Time::from_ps(1), ck, One).unwrap();
    k.run_until(Time::from_ps(1)).unwrap();
    assert_eq!(k.value(output).unwrap(), X); // could hold 0 or write 1
    k.schedule_batch(
        Time::from_ps(2),
        &[(ck, Zero), (wd, Zero), (rst, Zero)],
        None,
    )
    .unwrap();
    k.schedule(Time::from_ps(3), ck, One).unwrap();
    k.run_until(Time::from_ps(3)).unwrap();
    assert_eq!(k.value(output).unwrap(), One);
    assert_eq!(k.value(q).unwrap(), One); // local reset still active
}

#[test]
fn uncertain_phase_clock_faults_instead_of_inventing_progress() {
    let f = Fixture::with_system();
    let mut req = request(Startup::WakePhases);
    req.drives.push((sys("step"), Zero));
    let mut c = compile::compile(configured(&f, image_data(false)), req).unwrap();
    c.kernel
        .schedule(Time::from_ps(1), c.drivers["sys:step"], X)
        .unwrap();
    assert!(c.kernel.run_until(Time::from_ps(1)).is_err());
    assert!(c
        .kernel
        .fault()
        .unwrap()
        .contains("sys:step must remain binary"));
    // No second controller may take ownership of the same system state.
    let mut b = Builder::new(configured(&f, image_data(false))).unwrap();
    b.system_controls(SystemConfig {
        wake: None,
        user_gsr: Input::Constant(Zero),
    })
    .unwrap();
    assert!(b
        .system_controls(SystemConfig {
            wake: None,
            user_gsr: Input::Constant(Zero)
        })
        .is_err());
}
