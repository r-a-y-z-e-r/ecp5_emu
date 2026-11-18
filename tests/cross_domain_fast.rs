//! Fast mode against the event kernel where one clock is made from another
//! by an IOLOGIC capture, so its edges come one wave later in the same
//! instant: registers on the two clocks feed each other in both directions.
//! - A DCCA (`ClockGate`) on CLKI, its CE from a CLKI register, with
//!   registers on CLKI and on CLKO passing data back and forth.
//! - The X2 gearbox: CLKDIVF on ECLK makes SCLK, SCLK registers feed ODDRX2F,
//!   its pad feeds IDDRX2F, and SCLK registers take the holding nets that the
//!   GearIn mid rise writes in the same instant. RST of the divider changes
//!   at random, also together with an ECLK rise.
//! - The same for the 7:1 gearbox with CLKDIVF /3.5.
//!
//! Both compare every watched net after every step.
mod common;
use common::Fixture;
use ecp5_emu::{
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{Builder, DriverId, Input, IoLogicConfig, IoPath, Kernel, SignalId, Strength, Time},
    logic::Logic::{self, *},
    model::topology::{Location, Wire},
    prepared::ConfiguredDevice,
};
use serde_json::json;
use std::sync::Arc;

fn configured(f: &Fixture) -> Arc<ConfiguredDevice> {
    let snapshot = json!({"schema_version":1,"image_reader_schema":1,"device":"LFE5U-85F",
        "frames":4,"bits_per_frame":16,"encoding":"frame_byte_lsb0",
        "cram_hex":"0000000000000000","bram_data":{},
        "origin":"text_config","native_registers":null,"parser_defaults_may_apply":false,
        "native_metadata_hex":[]});
    let image = ConfigurationImage::from_snapshot(
        b".device LFE5U-85F\n".to_vec(),
        SourceKind::TextConfig,
        &serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalogue = Catalog::merge_documents(
        &[serde_json::to_vec(&json!({"device":"LFE5U-85F","resources":[]})).unwrap()],
        "LFE5U-85F",
        4,
        16,
    )
    .unwrap();
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalogue).unwrap())
}

struct Plan {
    builder: Builder,
    resource: ecp5_emu::model::ResourceId,
}

impl Plan {
    fn new(f: &Fixture) -> Self {
        let device = configured(f);
        let resource = device.model().resource_id("LFE5U-85F/R1C1/PLC2").unwrap();
        Plan {
            builder: Builder::new(device).unwrap(),
            resource,
        }
    }
    fn net(&mut self, name: &str) -> SignalId {
        self.builder
            .signal(Wire {
                location: Location { row: 1, col: 1 },
                name: name.into(),
            })
            .unwrap()
    }
    fn input(&mut self, name: &str) -> (SignalId, DriverId) {
        let s = self.net(name);
        let d = self
            .builder
            .external_driver(s, name, Strength::Strong, Zero)
            .unwrap();
        (s, d)
    }
    fn path(&mut self, name: &'static str, path: IoPath, clock: SignalId, reset: Option<Input>) {
        self.builder
            .iologic(IoLogicConfig {
                resource: self.resource,
                name,
                path,
                clock: clock.into(),
                reset,
            })
            .unwrap();
    }
    /// A rising-edge register on `clock`.
    fn register(
        &mut self,
        name: &'static str,
        data: Input,
        enable: Input,
        clock: SignalId,
    ) -> SignalId {
        let q = self.net(name);
        self.path(
            name,
            IoPath::Register {
                data,
                output: q,
                value: Zero,
                enable,
                sync_reset: None,
                global_reset: None,
            },
            clock,
            None,
        );
        q
    }
}

fn random(seed: u64) -> impl FnMut() -> u64 {
    let mut seed = seed;
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    }
}

/// Runs `steps`, each setting the drivers `next` returns, and compares the
/// watched nets after every step; returns how often each changed.
fn compare(
    mut kernel: Kernel,
    watched: &[SignalId],
    steps: u64,
    mut next: impl FnMut(u64) -> Vec<(DriverId, bool)>,
) -> Vec<u32> {
    let mut fast = FastSim::new(&kernel).unwrap_or_else(|e| panic!("{e:#}"));
    let mut moved = vec![0_u32; watched.len()];
    let mut last = vec![Zero; watched.len()];
    for step in 1..=steps {
        let set = next(step);
        let time = Time::from_ps(step * 1000);
        let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
        kernel.schedule_batch(time, &batch, None).unwrap();
        kernel.run_until(time).unwrap();
        fast.set_at(time, &set).unwrap();
        for (k, &s) in watched.iter().enumerate() {
            let v = kernel.value(s).unwrap();
            assert_eq!(fast.value(s), Some(v), "step {step}, watched net {k}");
            if v != last[k] {
                moved[k] += 1;
                last[k] = v;
            }
        }
    }
    moved
}

#[test]
fn fast_mode_matches_the_kernel_across_a_clock_gate_in_both_directions() {
    let f = Fixture::with_kernel();
    let mut p = Plan::new(&f);
    let (clki, clk_driver) = p.input("CLKI");
    let (cex, cex_driver) = p.input("CEX");
    let (din, din_driver) = p.input("DIN");
    let clko = p.net("CLKO");
    let gate_enable = p.register("E", din.into(), Input::Constant(One), clki);
    p.path(
        "DCC",
        IoPath::ClockGate {
            enable: gate_enable.into(),
            output: clko,
        },
        clki,
        None,
    );
    // A toggles on CLKI under a random enable; B takes A on CLKO; C takes B
    // back on CLKI; D takes C on CLKO, inverted, under the same enable.
    let a_net = p.net("A");
    p.path(
        "A",
        IoPath::Register {
            data: Input::Inverted(a_net),
            output: a_net,
            value: Zero,
            enable: cex.into(),
            sync_reset: None,
            global_reset: None,
        },
        clki,
        None,
    );
    let b = p.register("B", a_net.into(), cex.into(), clko);
    let c = p.register("C", b.into(), Input::Constant(One), clki);
    let d = p.register("D", Input::Inverted(c), cex.into(), clko);
    let watched = [clko, gate_enable, a_net, b, c, d];
    for &s in &watched {
        p.builder.watch(s).unwrap();
    }
    let kernel = p.builder.build().unwrap();
    let mut r = random(0x6a09_e667_f3bc_c908);
    let mut level = [false; 3];
    let drivers = [clk_driver, cex_driver, din_driver];
    let moved = compare(kernel, &watched, 30_000, |_| {
        let mut set = Vec::new();
        // The clock toggles every other step on average; CE and the gate's
        // data change at random, often in the same step as a clock edge.
        for (k, odds) in [2, 3, 3].into_iter().enumerate() {
            if r().is_multiple_of(odds) {
                level[k] = !level[k];
                set.push((drivers[k], level[k]));
            }
        }
        set
    });
    assert!(moved.iter().all(|&m| m > 500), "{moved:?}");
}

#[test]
fn fast_mode_matches_the_kernel_through_the_x2_gearbox_with_divider_resets() {
    let f = Fixture::with_kernel();
    let mut p = Plan::new(&f);
    let (eclk, eclk_driver) = p.input("ECLK");
    let (rst, rst_driver) = p.input("RST");
    let mut data_drivers = Vec::new();
    let mut word = [Input::Constant(Zero); 4];
    for (k, name) in ["D0", "D1", "D2", "D3"].into_iter().enumerate() {
        let (s, d) = p.input(name);
        word[k] = s.into();
        data_drivers.push(d);
    }
    let sclk = p.net("SCLK");
    p.path(
        "DIV",
        IoPath::Divider {
            align: Input::Constant(Zero),
            output: sclk,
            seven_halves: false,
        },
        eclk,
        Some(rst.into()),
    );
    // ODDRX2F: three ranks on SCLK, then the ECLK side onto the pad.
    const RANKS: [[&str; 4]; 3] = [
        ["A0", "A1", "A2", "A3"],
        ["B0", "B1", "B2", "B3"],
        ["C0", "C1", "C2", "C3"],
    ];
    let mut watched = vec![sclk];
    for names in RANKS {
        let mut next = word;
        for (k, name) in names.into_iter().enumerate() {
            let q = p.register(name, word[k], Input::Constant(One), sclk);
            next[k] = q.into();
            watched.push(q);
        }
        word = next;
    }
    let pad = p.net("PAD");
    p.path(
        "GEAROUT",
        IoPath::GearOut {
            data: word,
            sclk: sclk.into(),
            output: pad,
        },
        eclk,
        None,
    );
    // IDDRX2F: the pad at four ECLK edges, handed to SCLK registers.
    let holding = ["H0", "H1", "H2", "H3"].map(|n| p.net(n));
    p.path(
        "GEARIN",
        IoPath::GearIn {
            data: pad.into(),
            sclk: sclk.into(),
            outputs: holding,
        },
        eclk,
        None,
    );
    let mut outputs = Vec::new();
    for (k, name) in ["Q0", "Q1", "Q2", "Q3"].into_iter().enumerate() {
        outputs.push(p.register(name, holding[k].into(), Input::Constant(One), sclk));
    }
    watched.push(pad);
    watched.extend(holding);
    watched.extend(&outputs);
    for &s in &watched {
        p.builder.watch(s).unwrap();
    }
    let kernel = p.builder.build().unwrap();
    let mut r = random(0xbb67_ae85_84ca_a73b);
    let mut data = [false; 4];
    let mut reset = false;
    let moved = compare(kernel, &watched, 40_000, |step| {
        // ECLK toggles at every step, so every ECLK edge is compared.
        let mut set = vec![(eclk_driver, step % 2 == 1)];
        for (k, d) in data_drivers.iter().enumerate() {
            if r().is_multiple_of(3) {
                data[k] = !data[k];
                set.push((*d, data[k]));
            }
        }
        // RST: rare, short or long, and as often with a rising edge as
        // without.
        if r().is_multiple_of(97) || (reset && r().is_multiple_of(4)) {
            reset = !reset;
            set.push((rst_driver, reset));
        }
        set
    });
    assert!(moved.iter().all(|&m| m > 200), "{moved:?}");
}

#[test]
fn fast_mode_matches_the_kernel_through_the_7_to_1_gearbox_with_divider_resets() {
    let f = Fixture::with_kernel();
    let mut p = Plan::new(&f);
    let (eclk, eclk_driver) = p.input("ECLK");
    let (rst, rst_driver) = p.input("RST");
    const DATA: [&str; 7] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6"];
    let mut data_drivers = Vec::new();
    let mut word = [Input::Constant(Zero); 7];
    for (k, name) in DATA.into_iter().enumerate() {
        let (s, d) = p.input(name);
        word[k] = s.into();
        data_drivers.push(d);
    }
    let sclk = p.net("SCLK");
    p.path(
        "DIV",
        IoPath::Divider {
            align: Input::Constant(Zero),
            output: sclk,
            seven_halves: true,
        },
        eclk,
        Some(rst.into()),
    );
    const RANKS: [[&str; 7]; 2] = [
        ["A0", "A1", "A2", "A3", "A4", "A5", "A6"],
        ["B0", "B1", "B2", "B3", "B4", "B5", "B6"],
    ];
    let mut watched = vec![sclk];
    for names in RANKS {
        let mut next = word;
        for (k, name) in names.into_iter().enumerate() {
            let q = p.register(name, word[k], Input::Constant(One), sclk);
            next[k] = q.into();
            watched.push(q);
        }
        word = next;
    }
    let pad = p.net("PAD");
    p.path(
        "GEAROUT7",
        IoPath::GearOut7 {
            data: word,
            sclk: sclk.into(),
            output: pad,
        },
        eclk,
        Some(rst.into()),
    );
    let holding = ["H0", "H1", "H2", "H3", "H4", "H5", "H6"].map(|n| p.net(n));
    p.path(
        "GEARIN7",
        IoPath::GearIn7 {
            data: pad.into(),
            sclk: sclk.into(),
            outputs: holding,
        },
        eclk,
        None,
    );
    let mut outputs = Vec::new();
    for (k, name) in ["Q0", "Q1", "Q2", "Q3", "Q4", "Q5", "Q6"]
        .into_iter()
        .enumerate()
    {
        outputs.push(p.register(name, holding[k].into(), Input::Constant(One), sclk));
    }
    watched.push(pad);
    watched.extend(holding);
    watched.extend(&outputs);
    for &s in &watched {
        p.builder.watch(s).unwrap();
    }
    let kernel = p.builder.build().unwrap();
    let mut r = random(0x3c6e_f372_fe94_f82b);
    let mut data = [false; 7];
    let mut reset = false;
    let moved = compare(kernel, &watched, 40_000, |step| {
        let mut set = vec![(eclk_driver, step % 2 == 1)];
        for (k, d) in data_drivers.iter().enumerate() {
            if r().is_multiple_of(3) {
                data[k] = !data[k];
                set.push((*d, data[k]));
            }
        }
        if r().is_multiple_of(97) || (reset && r().is_multiple_of(4)) {
            reset = !reset;
            set.push((rst_driver, reset));
        }
        set
    });
    assert!(moved.iter().all(|&m| m > 200), "{moved:?}");
}
