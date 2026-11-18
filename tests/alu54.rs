//! ALU54B on a synthetic plan: the operation table against an independent
//! reference, and the event kernel and fast mode against that reference on
//! random traffic. The model is the board's (HD02..HD04,
//! docs/inventory/LFE5U-85F/hd02_alu54 and after).
mod common;
use common::{write, Fixture};
use ecp5_emu::{
    decode::Catalog,
    fast::FastSim,
    image::{ConfigurationImage, SourceKind},
    kernel::{
        alu54_operate, trailing_zeros_mask, Alu54, AluRegister, Builder, DriverId, Input, Kernel,
        SignalId, Strength, Time, ALU_FIRST_HALF, ALU_FLAGS,
    },
    logic::Logic::{self, *},
    model::topology::{Location, Wire},
    prepared::ConfiguredDevice,
};
use serde_json::json;
use std::{fs, sync::Arc};

const MASK: u64 = (1 << 54) - 1;
const RNDPAT: u64 = 0x01_2345_6789_ABC5;
/// R[26:18] and R[53:45], R[44:27]: a bypassed half and a group without a
/// clock, as in HD03 b0 and HD04 ua.
const HALF: u64 = 0x3F_E000_07FC_0000;
const GROUP: u64 = 0x1FFF_F800_0000;

/// The operation on three sources, by OP[10:7].
fn operate(op: u64, x: u64, y: u64, z: u64) -> u64 {
    let (ny, nz, kind) = (op & 1 == 1, op >> 1 & 1 == 1, op >> 2 & 3);
    if kind == 1 {
        let y = if ny { y.wrapping_neg() } else { y };
        let z = if nz { z.wrapping_neg() } else { z };
        return z.wrapping_add(x).wrapping_add(y) & MASK;
    }
    let y = if ny { !y } else { y };
    let z = if nz { !z } else { z };
    (match kind {
        0 => !(y & z),
        2 => y & z,
        _ => !(y ^ z),
    }) & MASK
}

/// One step's inputs.
#[derive(Clone, Copy, Default)]
struct Inputs {
    op: u64,
    ma: u64,
    mb: u64,
    sma: bool,
    smb: bool,
    a: u64,
    b: u64,
    c: u64,
    cin: u64,
    /// The resets of the first and the second half of R, and GSR.
    rst: bool,
    rst2: bool,
    gsr: bool,
    /// The enables and resets of the input registers, when the bench has them.
    cei: u64,
    rsti: u64,
}

impl Inputs {
    fn reads_r(&self) -> bool {
        self.op & 3 == 0 || self.op >> 4 & 7 == 5
    }
    /// The result with `r` as the R operand.
    fn result(&self, r: u64) -> u64 {
        let extend = |p: u64, signed: bool| {
            if signed && p >> 35 & 1 == 1 {
                p | 0x3FFFF << 36
            } else {
                p
            }
        };
        let (ma, mb) = (extend(self.ma, self.sma), extend(self.mb, self.smb));
        let x = [r, ma, self.a, 0][(self.op & 3) as usize];
        let y = [mb << 18 & MASK, mb, self.b, 0][(self.op >> 2 & 3) as usize];
        let fill = if self.cin >> 53 & 1 == 1 && self.smb {
            0x3FFFF << 36
        } else {
            0
        };
        let z = [
            0,
            self.cin >> 18 | fill,
            self.cin,
            self.c,
            self.a,
            r,
            RNDPAT,
            trailing_zeros_mask(RNDPAT),
        ][(self.op >> 4 & 7) as usize];
        operate(self.op >> 7, x, y, z)
    }
}

fn configured(f: &Fixture) -> Arc<ConfiguredDevice> {
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
    // The slice's two multipliers, which the device state expects; the ALU
    // here takes its products from drivers.
    for name in ["MULT18_0", "MULT18_1"] {
        for port in (0..36).map(|bit| format!("P{bit}")) {
            rows.push_str(&format!(
                "LFE5U-85F/R1C1/{name},{port},out,1,1,{name}_{port}\n"
            ));
        }
    }
    for port in (0..54)
        .map(|k| format!("R{k}"))
        .chain(ALU_FLAGS.iter().map(|f| f.to_string()))
        .chain(["SIGNEDR".to_string()])
    {
        rows.push_str(&format!(
            "LFE5U-85F/R1C1/ALU54_3,{port},out,1,1,ALU54_3_{port}\n"
        ));
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
    Arc::new(ConfiguredDevice::bind(Arc::new(f.load()), Arc::new(image), catalog).unwrap())
}

struct Bench {
    kernel: Kernel,
    /// Drivers by name: buses bit by bit, "CLK", "CE".
    drivers: Vec<(String, DriverId)>,
    r: [SignalId; 54],
    flags: [SignalId; 8],
}

/// An ALU with every input on a driver of its own; `op` fixes OP when given,
/// `gsr` gives it a global reset.
#[allow(clippy::too_many_arguments)]
fn bench(
    f: &Fixture,
    unregistered: u64,
    stuck: u64,
    async_reset: u64,
    flag_register: bool,
    op: Option<u64>,
    gsr: bool,
) -> Bench {
    bench_with(
        f,
        unregistered,
        stuck,
        async_reset,
        flag_register,
        op,
        gsr,
        false,
        RNDPAT,
        false,
    )
}

/// `bench`, with input registers when `registers` (each on its own enable
/// CEIn and reset RSTIn): IN's first stage, both stages of OP0, OP1's second
/// stage, and both C registers.
#[allow(clippy::too_many_arguments)]
fn bench_with(
    f: &Fixture,
    unregistered: u64,
    stuck: u64,
    async_reset: u64,
    flag_register: bool,
    op: Option<u64>,
    gsr: bool,
    registers: bool,
    rndpat: u64,
    both_edges: bool,
) -> Bench {
    let device = configured(f);
    let id = device
        .model()
        .resource_id("LFE5U-85F/R1C1/ALU54_3")
        .unwrap();
    let mut b = Builder::new(device).unwrap();
    let mut drivers = Vec::new();
    let mut input = |b: &mut Builder, name: String| -> Input {
        let signal = b
            .signal(Wire {
                location: Location { row: 1, col: 1 },
                name: name.clone(),
            })
            .unwrap();
        let driver = b
            .external_driver(signal, &name, Strength::Strong, Zero)
            .unwrap();
        drivers.push((name, driver));
        signal.into()
    };
    let mut bus = |b: &mut Builder, name: &str, n: usize| -> Vec<Input> {
        (0..n).map(|k| input(b, format!("{name}{k}"))).collect()
    };
    let clock = bus(&mut b, "CLK", 1)[0];
    let mut opcode: [[Option<AluRegister>; 2]; 3] = [[None; 2]; 3];
    let mut c_registers = [None; 2];
    if registers {
        let ce = bus(&mut b, "CEI", 4);
        let rst = bus(&mut b, "RSTI", 4);
        let reg = |n: usize, asynchronous: bool| AluRegister {
            clock,
            enable: ce[n],
            reset: rst[n],
            asynchronous,
        };
        opcode[0][0] = Some(reg(1, false));
        opcode[1][0] = Some(reg(2, true));
        opcode[1][1] = Some(reg(3, false));
        opcode[2][1] = Some(reg(0, true));
        if both_edges {
            opcode[2][0] = Some(reg(1, false));
        }
        c_registers = [Some(reg(0, false)), Some(reg(2, true))];
    }
    let config = Alu54 {
        both_edges,
        resource: id,
        op: match op {
            Some(op) => {
                std::array::from_fn(|k| Input::Constant(Logic::from_bool(op >> k & 1 == 1)))
            }
            None => bus(&mut b, "OP", 11).try_into().unwrap(),
        },
        ma: bus(&mut b, "MA", 36).try_into().unwrap(),
        mb: bus(&mut b, "MB", 36).try_into().unwrap(),
        signed_ma: bus(&mut b, "SMA", 1)[0],
        signed_mb: bus(&mut b, "SMB", 1)[0],
        a: bus(&mut b, "A", 54).try_into().unwrap(),
        b: bus(&mut b, "B", 54).try_into().unwrap(),
        c: bus(&mut b, "C", 54).try_into().unwrap(),
        cin: bus(&mut b, "CIN", 54).try_into().unwrap(),
        rndpat,
        clock,
        enable: bus(&mut b, "CE", 1)[0],
        resets: [bus(&mut b, "RST", 1)[0], bus(&mut b, "RSTB", 1)[0]],
        global_reset: gsr.then(|| bus(&mut b, "GSR", 1)[0]),
        opcode,
        c_registers,
        async_reset,
        unregistered,
        stuck,
        flags: [One, One, Zero, One, Zero, Zero, Zero, Zero],
        flag_clock: flag_register.then_some(clock),
    };
    let outputs = b.alu54(config).unwrap();
    for &s in outputs.r.iter().chain(&outputs.flags) {
        b.watch(s).unwrap();
    }
    Bench {
        kernel: b.build().unwrap(),
        drivers,
        r: outputs.r,
        flags: outputs.flags,
    }
}

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// The driver levels of a step: the inputs, the clock and the enable.
fn levels(bench: &Bench, i: &Inputs, clock: bool, enable: bool) -> Vec<(DriverId, bool)> {
    bench
        .drivers
        .iter()
        .map(|(name, driver)| {
            let split = name.find(|c: char| c.is_ascii_digit()).unwrap();
            let (bus, k) = (&name[..split], name[split..].parse::<u32>().unwrap());
            let word = match bus {
                "OP" => i.op,
                "MA" => i.ma,
                "MB" => i.mb,
                "SMA" => u64::from(i.sma),
                "SMB" => u64::from(i.smb),
                "A" => i.a,
                "B" => i.b,
                "C" => i.c,
                "CIN" => i.cin,
                "RST" => u64::from(i.rst),
                "RSTB" => u64::from(i.rst2),
                "GSR" => u64::from(i.gsr),
                "CEI" => i.cei,
                "RSTI" => i.rsti,
                "CLK" => u64::from(clock),
                "CE" => u64::from(enable),
                other => panic!("{other}"),
            };
            (*driver, word >> k & 1 == 1)
        })
        .collect()
}

fn word(kernel: &Kernel, signals: &[SignalId]) -> Option<u64> {
    signals.iter().enumerate().try_fold(0, |w, (k, &s)| {
        Some(w | (kernel.value(s).unwrap().bit()? as u64) << k)
    })
}

/// Random steps: new inputs while the clock is low, then a rising edge.
/// Checks the kernel against the reference and fast mode, when `fast`,
/// against the kernel. Returns how many steps captured.
#[allow(clippy::too_many_arguments)]
fn run(
    unregistered: u64,
    stuck: u64,
    async_reset: u64,
    flag_register: bool,
    op: Option<u64>,
    gsr: bool,
    fast: bool,
    steps: u64,
) -> u64 {
    let f = Fixture::with_kernel();
    let mut b = bench(&f, unregistered, stuck, async_reset, flag_register, op, gsr);
    if fast {
        common::snapshot_dsp_source(
            &format!("alu-{unregistered}-{stuck}-{async_reset}-{flag_register}-{op:?}-{gsr}"),
            &b.kernel,
        );
    }
    let mut sim = fast.then(|| FastSim::new(&b.kernel).unwrap_or_else(|e| panic!("{e:#}")));
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64 ^ unregistered ^ stuck << 1 ^ op.unwrap_or(0);
    // The register (None: unknown, after a loop through an unregistered bit)
    // and the flag register.
    let (mut register, mut flags, mut captures) = (Some(0_u64), 0_u64, 0);
    let constant_flags = 0b0000_1011; // EQZ, EQZM, EQPAT as bits 0, 1, 3
    for step in 1..=steps {
        let mut next = |bits: u32| xorshift(&mut seed) & ((1 << bits) - 1);
        let i = Inputs {
            op: op.unwrap_or_else(|| next(11)),
            ma: next(36),
            mb: next(36),
            sma: next(1) == 1,
            smb: next(1) == 1,
            a: next(54),
            b: next(54),
            c: next(54),
            cin: next(54),
            rst: next(3) == 0,
            rst2: next(3) == 0,
            gsr: gsr && next(4) == 0,
            ..Inputs::default()
        };
        // The bits each reset clears, and GSR all of them.
        let resets = if i.rst { ALU_FIRST_HALF } else { 0 }
            | if i.rst2 { MASK & !ALU_FIRST_HALF } else { 0 };
        let kept = if i.gsr { 0 } else { MASK };
        let enable = next(2) != 0;
        for (phase, clock) in [false, true].into_iter().enumerate() {
            let time = Time::from_ps((2 * step + phase as u64) * 1000);
            let set = levels(&b, &i, clock, enable);
            let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
            b.kernel.schedule_batch(time, &batch, None).unwrap();
            b.kernel.run_until(time).unwrap();
            if let Some(sim) = &mut sim {
                sim.set_at(time, &set).unwrap();
            }
            // The reference: the register captures at the rising edge.
            let visible = !unregistered & !stuck & MASK;
            let operand = match register {
                Some(r) if !i.reads_r() || unregistered == 0 => Some(r & visible),
                _ if !i.reads_r() => Some(0),
                _ => None,
            };
            if clock {
                // The reset wins over the enable, for both kinds of bits,
                // and GSR over both.
                if enable {
                    register = operand.map(|r| i.result(r));
                }
                if resets | !kept & MASK == MASK {
                    register = Some(0);
                }
                register = register.map(|r| r & !resets & kept);
                if enable {
                    flags = constant_flags;
                    captures += 1;
                }
            } else {
                // While the clock is low only the asynchronous bits clear,
                // and GSR clears all.
                register = register.map(|r| r & !(resets & async_reset) & kept);
            }
            // R as seen: the register, the result where unregistered, 0
            // where there is no clock.
            let seen = match (register, operand) {
                (Some(r), Some(o)) => Some((r & visible) | (i.result(o) & unregistered)),
                (Some(r), None) if unregistered == 0 => Some(r & visible),
                _ => None,
            };
            let got = word(&b.kernel, &b.r);
            if let Some(seen) = seen {
                assert_eq!(
                    got,
                    Some(seen),
                    "R at step {step}, clock {clock}, op {:011b}",
                    i.op
                );
            }
            let want_flags = if flag_register { flags } else { constant_flags };
            assert_eq!(
                word(&b.kernel, &b.flags),
                Some(want_flags),
                "flags at step {step}"
            );
            if let Some(sim) = &sim {
                for (k, &s) in b.r.iter().chain(&b.flags).enumerate() {
                    assert_eq!(
                        sim.value(s),
                        Some(b.kernel.value(s).unwrap()),
                        "output {k} at step {step}"
                    );
                }
            }
        }
    }
    captures
}

#[test]
fn hd09_disabled_gsr_hold_survives_fast_handover() {
    for asynchronous in [0, HALF, MASK] {
        let f = Fixture::with_kernel();
        let mut b = bench_with(
            &f,
            0,
            0,
            asynchronous,
            false,
            Some(0b010_0011_1111),
            true,
            false,
            RNDPAT,
            true,
        );
        let mut fast = None;
        for (step, clock, gsr, data, want) in [
            (1, true, true, 1, 0),
            (2, true, false, 2, 0),
            (3, false, false, 3, 0),
            (4, true, false, 4, 4),
        ] {
            let i = Inputs {
                c: data,
                gsr,
                ..Inputs::default()
            };
            let set = levels(&b, &i, clock, true);
            let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
            let at = Time::from_ps(step);
            b.kernel.schedule_batch(at, &batch, None).unwrap();
            b.kernel.run_until(at).unwrap();
            if let Some(sim) = &mut fast {
                let sim: &mut FastSim = sim;
                sim.set_at(at, &set).unwrap();
                for &s in &b.r {
                    assert_eq!(
                        sim.value(s),
                        Some(b.kernel.value(s).unwrap()),
                        "step {step}"
                    );
                }
            }
            assert_eq!(word(&b.kernel, &b.r), Some(want), "step {step}");
            if step == 2 {
                fast = Some(FastSim::new(&b.kernel).unwrap());
            }
        }
    }
}

#[test]
fn operation_table_is_the_reference_for_every_code() {
    let mut seed = 0x1234_5678_9ABC_DEF1_u64;
    let bits =
        |w: u64| -> [Logic; 54] { std::array::from_fn(|k| Logic::from_bool(w >> k & 1 == 1)) };
    for op in 0..16_u64 {
        for _ in 0..200 {
            let [x, y, z] = [0; 3].map(|_| xorshift(&mut seed) & MASK);
            let got = alu54_operate(
                std::array::from_fn(|k| Logic::from_bool(op >> k & 1 == 1)),
                &bits(x),
                &bits(y),
                &bits(z),
            );
            assert_eq!(got, bits(operate(op, x, y, z)), "op {op:04b}");
        }
    }
    // An unknown bit reaches only what it can: a sum's bits below it, and a
    // bitwise result's other bits, stay known.
    let mut y = bits(0b0110);
    y[2] = X;
    let sum = alu54_operate([Zero, Zero, One, Zero], &bits(1), &y, &bits(0));
    assert_eq!(&sum[..2], &[One, One]);
    assert_eq!(sum[2], X);
    let and = alu54_operate([Zero, Zero, Zero, One], &bits(0), &y, &bits(0b1111));
    assert_eq!(&and[..4], &[Zero, One, X, Zero]);
}

#[test]
fn registered_alu_follows_the_reference_in_the_kernel_and_in_fast_mode() {
    // A synchronous reset, an asynchronous one, and one of each kind per half.
    assert!(run(0, 0, 0, true, None, false, true, 6_000) > 3_000);
    assert!(run(0, GROUP, MASK, false, None, false, true, 3_000) > 1_500);
    assert!(run(0, 0, HALF, true, None, false, true, 3_000) > 1_500);
    // GSR with each kind of reset.
    assert!(run(0, 0, HALF, true, None, true, true, 3_000) > 1_500);
    assert!(run(0, GROUP, MASK, false, None, true, true, 3_000) > 1_500);
}

#[test]
fn unregistered_bits_show_the_result_and_make_r_unknown_as_an_operand() {
    // The kernel alone: a random OP reads R through the unregistered bits in
    // some steps, which the reference skips.
    run(HALF, 0, MASK & !HALF, true, None, false, false, 6_000);
    run(MASK, 0, 0, false, None, false, false, 3_000);
    // Fast mode refuses a program that could read R through such bits, and
    // runs one whose OP is fixed and does not: R = C + MA - MB.
    let f = Fixture::with_kernel();
    let error = FastSim::new(&bench(&f, HALF, 0, 0, true, None, false).kernel)
        .err()
        .expect("fast mode must refuse a loop through unregistered bits");
    assert!(format!("{error:#}").contains("unregistered"), "{error:#}");
    // OP[10:7] = 0101, Z = C (3), Y = MB (1), X = MA (1).
    let sum = 0b010_1011_0101;
    run(HALF, 0, 0, true, Some(sum), true, true, 3_000);
    run(MASK, 0, 0, true, Some(sum), false, true, 3_000);
    // No register at all: the program has no clock group.
    run(MASK, 0, 0, false, Some(sum), false, true, 3_000);
}

#[test]
fn z7_reads_the_trailing_zeros_of_rndpat() {
    // HD08z: Z = 7 is ~RNDPAT & (RNDPAT - 1).
    assert_eq!(trailing_zeros_mask(0x10), 0xF);
    assert_eq!(trailing_zeros_mask(0), MASK);
    assert_eq!(trailing_zeros_mask(RNDPAT), 0);
    let f = Fixture::with_kernel();
    // R = Z: X and Y zero, a sum, Z = 7.
    let mut b = bench_with(
        &f,
        0,
        0,
        0,
        false,
        Some(0b010_0111_1111),
        false,
        false,
        0x100,
        false,
    );
    let ce = b.drivers.iter().find(|(n, _)| n == "CE0").unwrap().1;
    let clk = b.drivers.iter().find(|(n, _)| n == "CLK0").unwrap().1;
    b.kernel.schedule(Time::from_ps(1), ce, One).unwrap();
    b.kernel.schedule(Time::from_ps(2), clk, One).unwrap();
    b.kernel.run_until(Time::from_ps(2)).unwrap();
    assert_eq!(word(&b.kernel, &b.r), Some(0xFF));
}

#[test]
fn input_registers_in_fast_mode_follow_the_kernel() {
    input_register_profiles(false);
}

#[test]
fn hd09_disabled_input_registers_in_fast_mode_follow_the_kernel() {
    input_register_profiles(true);
}

fn input_register_profiles(both_edges: bool) {
    // The opcode and C registers (HD08/09) on random traffic, with every kind
    // of reset and GSR: fast mode prints what the kernel prints, R and flags,
    // at every half clock.
    for rndpat in [RNDPAT, 0x10] {
        let f = Fixture::with_kernel();
        let mut b = bench_with(&f, 0, 0, HALF, true, None, true, true, rndpat, both_edges);
        if !both_edges {
            common::snapshot_dsp_source(&format!("alu-inputs-{rndpat}"), &b.kernel);
        }
        let mut sim = FastSim::new(&b.kernel).unwrap_or_else(|e| panic!("{e:#}"));
        let mut seed = 0x5DEE_CE66_D1CE_4E5B_u64 ^ rndpat;
        let mut changes = 0;
        let mut last = None;
        for step in 1..=4_000_u64 {
            let mut next = |bits: u32| xorshift(&mut seed) & ((1 << bits) - 1);
            let mut ones = |every: u64| (0..4).fold(0, |w, k| w | u64::from(next(3) < every) << k);
            let cei = ones(6);
            let rsti = ones(1);
            let i = Inputs {
                op: next(11),
                ma: next(36),
                mb: next(36),
                sma: next(1) == 1,
                smb: next(1) == 1,
                a: next(54),
                b: next(54),
                c: next(54),
                cin: next(54),
                rst: next(3) == 0,
                rst2: next(3) == 0,
                gsr: next(5) == 0,
                cei,
                rsti,
            };
            let enable = next(2) != 0;
            for (phase, clock) in [false, true].into_iter().enumerate() {
                let time = Time::from_ps((2 * step + phase as u64) * 1000);
                let set = levels(&b, &i, clock, enable);
                let batch: Vec<_> = set.iter().map(|&(d, v)| (d, Logic::from_bool(v))).collect();
                b.kernel.schedule_batch(time, &batch, None).unwrap();
                b.kernel.run_until(time).unwrap();
                sim.set_at(time, &set).unwrap();
                for (k, &s) in b.r.iter().chain(&b.flags).enumerate() {
                    assert_eq!(
                        sim.value(s),
                        Some(b.kernel.value(s).unwrap()),
                        "output {k} at step {step}, RNDPAT {rndpat:#x}, both_edges {both_edges}"
                    );
                }
            }
            if both_edges && i.gsr {
                // Carry the saved GSR hold across a kernel/fast handover.
                sim = FastSim::new(&b.kernel).unwrap();
            }
            let r = word(&b.kernel, &b.r);
            changes += usize::from(r != last);
            last = r;
        }
        assert!(changes > 1_000, "{changes}");
    }
}

#[test]
fn hd09_disabled_alu_captures_random_traffic_on_both_edges() {
    let f = Fixture::with_kernel();
    let mut b = bench_with(&f, 0, 0, 0, false, None, false, false, RNDPAT, true);
    let mut seed = 0x909_u64;
    let mut stored = 0;
    let mut fast = FastSim::new(&b.kernel).unwrap();
    for step in 1..=1000 {
        let mut next = |bits: u32| xorshift(&mut seed) & ((1 << bits) - 1);
        let i = Inputs {
            op: next(11),
            ma: next(36),
            mb: next(36),
            sma: next(1) != 0,
            smb: next(1) != 0,
            a: next(54),
            b: next(54),
            c: next(54),
            cin: next(54),
            rst: next(3) == 0,
            rst2: next(3) == 0,
            ..Inputs::default()
        };
        let enable = next(2) != 0;
        if enable {
            stored = i.result(stored);
        }
        if i.rst {
            stored &= !ALU_FIRST_HALF;
        }
        if i.rst2 {
            stored &= ALU_FIRST_HALF;
        }
        let set = levels(&b, &i, step % 2 == 1, enable);
        let batch: Vec<_> = set
            .iter()
            .copied()
            .map(|(d, v)| (d, Logic::from_bool(v)))
            .collect();
        let time = Time::from_ps(step);
        b.kernel.schedule_batch(time, &batch, None).unwrap();
        b.kernel.run_until(time).unwrap();
        assert_eq!(word(&b.kernel, &b.r), Some(stored), "edge {step}");
        fast.set_at(time, &set).unwrap();
        for &s in &b.r {
            assert_eq!(
                fast.value(s),
                Some(b.kernel.value(s).unwrap()),
                "edge {step}"
            );
        }
    }
}
