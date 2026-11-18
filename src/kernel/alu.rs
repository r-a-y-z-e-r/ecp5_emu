//! ALU54B as measured on the board (HD02, HD03, HD04;
//! docs/inventory/LFE5U-85F/hd02_alu54, hd03_alu54_inputs,
//! hd04_alu54_cascade).
//!
//! Three 54-bit sources and an operation, all chosen by OP:
//! - X = OP[1:0]: 0 R, 1 MA, 2 A, 3 zero;
//! - Y = OP[3:2]: 0 MB << 18, 1 MB, 2 B, 3 zero;
//! - Z = OP[6:4]: 0 zero, 1 CIN >> 18, 2 CIN, 3 C, 4 A, 5 R, 6 RNDPAT, 7 zero;
//! - OP[10:9] = 1: R = (OP8 ? -Z : Z) + X + (OP7 ? -Y : Y) modulo 2^54;
//!   0, 2, 3: bit by bit and without X, with Y' = OP7 ? ~Y : Y and
//!   Z' = OP8 ? ~Z : Z: ~(Y' & Z'), Y' & Z', ~(Y' ^ Z').
//!
//! MA and MB are the 36-bit products, sign-extended when the multiplier's
//! SIGNEDP is 1. A, B and C are 54 bits as the planner assembles them (the
//! multipliers' C inputs are part of all three). CIN is R of the ALU before
//! this one. The top 18 bits of CIN >> 18 are CIN[53] & SIGNEDIB, the ALU's
//! own second sign input (HD04).
//!
//! R is the output register: 0 after configuration, it captures at a rising
//! edge of its clock while the enable is high. Two masks say which bits are
//! not such a register:
//! - `unregistered`: the bit leaves the ALU before the register (a
//!   CIBOUT_BYP half). The fabric, the cascade and the ALU's own R operand
//!   see the result itself there. An operation that reads R through such a
//!   bit is a loop on the chip, and the board gave values that differ between
//!   loads (HD03 b0, b2; HD04 ta, tb, ub): those bits read as unknown.
//! - `stuck`: a register without a clock (REG_OUTPUTn_CLK=NONE and no
//!   bypass); it stays 0 (HD04 ta, ua, ub).
//!
//! The reset wins over the enable (HD06, HD07). R has two halves for it:
//! R[17:0] and R[44:27] (`ALU_FIRST_HALF`) take `resets[0]`, the line the
//! slice's REG_OUTPUT0/1_RST selects; R[26:18] and R[53:45] take `resets[1]`,
//! the block's RST0 line. Where a reset is synchronous it clears at a clock
//! edge; where asynchronous (`async_reset`), at once and for as long as it is
//! high. A global reset (`global_reset`, the slice's GSR enabled) clears
//! every bit at once and holds it while active (HD07). With the slice's GSR
//! disabled it does nothing (HD06). The flag register takes neither reset:
//! that is not measured.
//!
//! The flags never followed R on the board. They are what a zero word would
//! give against the patterns, as constants; a flag register starts at 0 and
//! takes them at its first enabled clock edge (HD04).
//!
//! Z = 7 reads ~RNDPAT & (RNDPAT - 1), the mask of RNDPAT's trailing zeros:
//! 0 for an odd RNDPAT (HD02..HD05), all ones for 0 (HD08z).
//!
//! The input registers (HD08), each 0 after configuration, cleared by its
//! reset (over the enable) and by an enabled GSR:
//! - opcode registers, by group: IN holds OP[6:0], OP0 OP7, OP1 OP[10:8]; a
//!   group has a first stage and a second one after it, each optional, each
//!   capturing at every rising edge of its clock;
//! - C registers: C0 holds C[26:0] and A[53:36], C1 C[53:27] and B[53:36].
//!   They capture at every other rising edge of their clock: a divider
//!   toggles at each edge, and an edge counts when it is 0 before it. An
//!   enabled GSR holds the divider at 0; it is 0 after configuration (not
//!   measured: GSR came before any edge showed it).
//!
//! HD09 with CLK_DIV disabled: R/opcode capture both edges, C every rise;
//! GSR active at a rise holds the non-C stages through the following fall.
use super::*;

/// The bits of a 54-bit word.
const WIDTH: usize = 54;
const MASK: u64 = (1 << WIDTH) - 1;
/// R[17:0] and R[44:27]: the half of R behind the slice's first MULT18, which
/// takes `Alu54::resets[0]`.
pub const ALU_FIRST_HALF: u64 = 0x3FFFF | 0x3FFFF << 27;
/// The OP bits of the opcode register groups IN, OP0 and OP1.
pub const ALU_OPCODE_GROUPS: [std::ops::Range<usize>; 3] = [0..7, 7..8, 8..11];
/// The C bits of the C registers C0 and C1; each also holds the top 18 bits
/// of A (C0) or B (C1), the C input of its MULT18.
pub const ALU_C_GROUPS: [std::ops::Range<usize>; 2] = [0..27, 27..54];

/// An input register of an ALU54B: its clock, enable and reset.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct AluRegister {
    pub clock: Input,
    pub enable: Input,
    pub reset: Input,
    /// The reset clears at once and for as long as it is high (else at an
    /// edge).
    pub asynchronous: bool,
}

/// The mask of RNDPAT's trailing zeros, which Z = 7 reads (HD08z).
pub fn trailing_zeros_mask(rndpat: u64) -> u64 {
    !rndpat & rndpat.wrapping_sub(1) & MASK
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Alu54 {
    /// HD09: non-C registers capture both edges; C captures every rise.
    pub both_edges: bool,
    pub resource: ResourceId,
    pub op: [Input; 11],
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub ma: [Input; 36],
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub mb: [Input; 36],
    /// SIGNEDIA and SIGNEDIB: the multipliers' SIGNEDP.
    pub signed_ma: Input,
    pub signed_mb: Input,
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub a: [Input; WIDTH],
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub b: [Input; WIDTH],
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub c: [Input; WIDTH],
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub cin: [Input; WIDTH],
    pub rndpat: u64,
    /// The clock of the registered bits of R, and the enable of every
    /// register: the DSP block's CE0 line.
    pub clock: Input,
    pub enable: Input,
    /// The resets of the output register: of R[17:0] and R[44:27] (the line
    /// REG_OUTPUT0/1_RST selects), and of R[26:18] and R[53:45] (RST0).
    pub resets: [Input; 2],
    /// GSR where the slice has it enabled: clears every bit of R at once,
    /// and every input register.
    pub global_reset: Option<Input>,
    /// The opcode registers of IN, OP0, OP1 ([`ALU_OPCODE_GROUPS`]): first
    /// and second stage.
    pub opcode: [[Option<AluRegister>; 2]; 3],
    /// The C registers C0 and C1 ([`ALU_C_GROUPS`]), on every other edge.
    pub c_registers: [Option<AluRegister>; 2],
    /// Bits of R whose reset is asynchronous; the others' is synchronous.
    pub async_reset: u64,
    /// Bits of R taken before the register.
    pub unregistered: u64,
    /// Bits of R whose register has no clock: always 0.
    pub stuck: u64,
    /// EQZ, EQZM, EQOM, EQPAT, EQPATB, OVER, UNDER, OVERUNDER: constants on
    /// the board (HD02, HD03, HD04).
    pub flags: [Logic; 8],
    /// The flag register's clock; `None` when the flags are unregistered.
    pub flag_clock: Option<Input>,
}

pub struct Alu54Outputs {
    pub r: [SignalId; WIDTH],
    pub flags: [SignalId; 8],
    pub signed_r: SignalId,
}

pub const ALU_FLAGS: [&str; 8] = [
    "EQZ",
    "EQZM",
    "EQOM",
    "EQPAT",
    "EQPATB",
    "OVER",
    "UNDER",
    "OVERUNDER",
];

/// The input registers' state.
#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct AluInputs {
    /// The opcode stages, by group and stage: the group's bits of OP.
    pub opcode: [[[Logic; 11]; 2]; 3],
    /// The C registers: C, and the top 18 bits of A (C0) and B (C1).
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    pub c: [Logic; WIDTH],
    pub top: [[Logic; 18]; 2],
    /// The divider of each C register (0: its next edge counts).
    pub divider: [Logic; 2],
}

#[derive(Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Alu {
    config: Alu54,
    /// The output register; 0 after configuration.
    #[cfg_attr(test, serde(serialize_with = "crate::kernel::cache::snapshot::array"))]
    register: [Logic; WIDTH],
    /// The flag register; 0 after configuration.
    flag_register: [Logic; 8],
    /// The input registers; 0 after configuration.
    inputs: AluInputs,
    previous_clock: Logic,
    pub(super) global_at_rise: Logic,
    pub(super) opcode_global_at_rise: [[Logic; 2]; 3],
    previous_flag_clock: Logic,
    previous_opcode_clocks: [[Logic; 2]; 3],
    previous_c_clocks: [Logic; 2],
    /// Drivers of R0..R53, the eight flags and SIGNEDR.
    drivers: Vec<usize>,
}

impl Alu {
    pub(super) fn config(&self) -> &Alu54 {
        &self.config
    }
    pub(super) fn register(&self) -> &[Logic; WIDTH] {
        &self.register
    }
    pub(super) fn flag_register(&self) -> &[Logic; 8] {
        &self.flag_register
    }
    pub(super) fn input_registers(&self) -> &AluInputs {
        &self.inputs
    }
    /// The drivers of R0..R53.
    pub(super) fn r_drivers(&self) -> &[usize] {
        &self.drivers[..WIDTH]
    }
    /// The drivers of the eight flags.
    pub(super) fn flag_drivers(&self) -> &[usize] {
        &self.drivers[WIDTH..WIDTH + 8]
    }
    /// The sum (or the bitwise result) from the signals as they are and the
    /// register as it is.
    fn result(&self, signals: &[Signal]) -> [Logic; WIDTH] {
        let c = &self.config;
        let read = |i: Input| match i.read(signals) {
            Logic::Z => Logic::X,
            v => v,
        };
        // OP, A, B and C through the input registers where they are on.
        let mut op = c.op.map(read);
        for (g, bits) in ALU_OPCODE_GROUPS.iter().enumerate() {
            if let Some(s) = (0..2).rev().find(|&s| c.opcode[g][s].is_some()) {
                op[bits.clone()].copy_from_slice(&self.inputs.opcode[g][s][bits.clone()]);
            }
        }
        let mut a = c.a.map(read);
        let mut b = c.b.map(read);
        let mut cw = c.c.map(read);
        for (n, bits) in ALU_C_GROUPS.iter().enumerate() {
            if c.c_registers[n].is_some() {
                cw[bits.clone()].copy_from_slice(&self.inputs.c[bits.clone()]);
                let top = if n == 0 { &mut a } else { &mut b };
                top[36..].copy_from_slice(&self.inputs.top[n]);
            }
        }
        // What an operation reads as R: the register, 0 where it has no
        // clock, or nothing known where it is bypassed.
        let feedback: [Logic; WIDTH] = std::array::from_fn(|k| {
            if c.unregistered >> k & 1 == 1 {
                Logic::X
            } else if c.stuck >> k & 1 == 1 {
                Logic::Zero
            } else {
                self.register[k]
            }
        });
        let extend = |product: &[Input; 36], signed: Input| -> [Logic; WIDTH] {
            let sign = read(product[35]).and(read(signed));
            std::array::from_fn(|k| if k < 36 { read(product[k]) } else { sign })
        };
        let zero = [Logic::Zero; WIDTH];
        let ma = extend(&c.ma, c.signed_ma);
        let mb = extend(&c.mb, c.signed_mb);
        let cin = c.cin.map(read);
        let x = select(&op[0..2], &[feedback, ma, a, zero]);
        let shifted: [Logic; WIDTH] =
            std::array::from_fn(|k| if k < 18 { Logic::Zero } else { mb[k - 18] });
        let y = select(&op[2..4], &[shifted, mb, b, zero]);
        let fill = cin[53].and(read(c.signed_mb));
        let cin_high: [Logic; WIDTH] =
            std::array::from_fn(|k| if k < 36 { cin[k + 18] } else { fill });
        let z = select(
            &op[4..7],
            &[
                zero,
                cin_high,
                cin,
                cw,
                a,
                feedback,
                word_bits(c.rndpat),
                word_bits(trailing_zeros_mask(c.rndpat)),
            ],
        );
        operate([op[7], op[8], op[9], op[10]], &x, &y, &z)
    }
    /// R as the fabric sees it: the register, or the result itself where the
    /// register is bypassed.
    fn outputs(&self, signals: &[Signal]) -> [Logic; WIDTH] {
        let c = &self.config;
        let result = (c.unregistered != 0).then(|| self.result(signals));
        std::array::from_fn(|k| {
            if c.unregistered >> k & 1 == 1 {
                result.unwrap()[k]
            } else if c.stuck >> k & 1 == 1 {
                Logic::Zero
            } else {
                self.register[k]
            }
        })
    }
    /// The flags as the fabric sees them.
    fn flag_outputs(&self) -> [Logic; 8] {
        match self.config.flag_clock {
            Some(_) => self.flag_register,
            None => self.config.flags,
        }
    }
}

fn word_bits(word: u64) -> [Logic; WIDTH] {
    std::array::from_fn(|k| Logic::from_bool(word >> k & 1 == 1))
}

fn word(bits: &[Logic; WIDTH]) -> Option<u64> {
    bits.iter()
        .enumerate()
        .try_fold(0, |w, (k, b)| Some(w | (b.bit()? as u64) << k))
}

/// One of 2^n sources by n select bits, LSB first; an unknown select bit
/// keeps the bits on which its candidates agree.
fn select<const N: usize>(select: &[Logic], sources: &[[Logic; WIDTH]; N]) -> [Logic; WIDTH] {
    let mut merged: Option<[Logic; WIDTH]> = None;
    for (n, source) in sources.iter().enumerate() {
        let possible = select
            .iter()
            .enumerate()
            .all(|(k, s)| s.bit().is_none_or(|b| b == (n >> k & 1 == 1)));
        if possible {
            merged = Some(match merged {
                None => *source,
                Some(old) => std::array::from_fn(|k| Logic::mux(Logic::X, old[k], source[k])),
            });
        }
    }
    merged.unwrap()
}

/// a + b + carry over 54 bits; an unknown bit makes unknown what it can reach.
fn add(a: &[Logic; WIDTH], b: &[Logic; WIDTH], mut carry: Logic) -> [Logic; WIDTH] {
    if let (Some(a), Some(b), Some(c)) = (word(a), word(b), carry.bit()) {
        return word_bits((a + b + c as u64) & MASK);
    }
    std::array::from_fn(|k| {
        let (x, y) = (a[k], b[k]);
        let sum = x.xor(y).xor(carry);
        carry = x.and(y).or(x.and(carry)).or(y.and(carry));
        sum
    })
}

/// The operation `op` = [OP7, OP8, OP9, OP10] on three sources.
pub fn operate(
    op: [Logic; 4],
    x: &[Logic; WIDTH],
    y: &[Logic; WIDTH],
    z: &[Logic; WIDTH],
) -> [Logic; WIDTH] {
    let [invert_y, invert_z, low, high] = op;
    let y: [Logic; WIDTH] = std::array::from_fn(|k| y[k].xor(invert_y));
    let z: [Logic; WIDTH] = std::array::from_fn(|k| z[k].xor(invert_z));
    match (high.bit(), low.bit()) {
        // -v is ~v + 1: the inversions above and a carry each.
        (Some(false), Some(true)) => add(&add(&z, x, invert_z), &y, invert_y),
        (Some(false), Some(false)) => std::array::from_fn(|k| y[k].and(z[k]).invert()),
        (Some(true), Some(false)) => std::array::from_fn(|k| y[k].and(z[k])),
        (Some(true), Some(true)) => std::array::from_fn(|k| y[k].xor(z[k]).invert()),
        _ => [Logic::X; WIDTH],
    }
}

pub(super) struct AluCapture {
    index: usize,
    register: [Logic; WIDTH],
    flag_register: [Logic; 8],
    inputs: AluInputs,
}

/// A register bit at an edge: `old`, or `data` where enabled, cleared by a
/// synchronous reset, an asynchronous one and the global reset.
fn capture_bit(
    old: Logic,
    data: Logic,
    edge: Activity,
    enable: Logic,
    reset: Logic,
    asynchronous: bool,
    global: Logic,
) -> Logic {
    let mut next = old;
    if edge != Activity::None {
        let mut captured = Logic::mux(enable, old, data);
        if !asynchronous {
            captured = Logic::mux(reset, captured, Logic::Zero);
        }
        next = if edge == Activity::Definite {
            captured
        } else {
            Logic::mux(Logic::X, old, captured)
        };
    }
    if asynchronous {
        next = Logic::mux(reset, next, Logic::Zero);
    }
    Logic::mux(global, next, Logic::Zero)
}

impl Builder {
    /// Binds an ALU54B; its outputs are R0..R53, the flags and SIGNEDR.
    /// SIGNEDR is driven unknown: its rule is not measured.
    pub fn alu54(&mut self, config: Alu54) -> Result<Alu54Outputs> {
        let resource = self.device.model().resource(config.resource);
        ensure!(resource.family == "ALU54", "resource is not an ALU54");
        ensure!(
            !self.bound_alus.contains(&config.resource),
            "ALU54 already has an execution owner"
        );
        ensure!(
            config.rndpat <= MASK
                && config.unregistered <= MASK
                && config.stuck <= MASK
                && config.async_reset <= MASK,
            "ALU54 word wider than 54 bits"
        );
        ensure!(
            config.unregistered & config.stuck == 0,
            "an ALU54 bit is both unregistered and without a clock"
        );
        let inputs: Vec<Input> = config
            .op
            .into_iter()
            .chain(config.ma)
            .chain(config.mb)
            .chain(config.a)
            .chain(config.b)
            .chain(config.c)
            .chain(config.cin)
            .chain([
                config.signed_ma,
                config.signed_mb,
                config.clock,
                config.enable,
            ])
            .chain(config.flag_clock)
            .chain(config.resets)
            .chain(config.global_reset)
            .chain(
                config
                    .opcode
                    .iter()
                    .flatten()
                    .chain(&config.c_registers)
                    .flatten()
                    .flat_map(|r| [r.clock, r.enable, r.reset]),
            )
            .collect();
        for &input in &inputs {
            self.validate_input(input)?;
        }
        let key = resource.key.clone();
        let ports = self.device.model().resource_ports(config.resource);
        let names = (0..WIDTH)
            .map(|k| format!("R{k}"))
            .chain(ALU_FLAGS.iter().map(|f| f.to_string()))
            .chain(["SIGNEDR".to_string()]);
        let mut signals = Vec::new();
        let mut drivers = Vec::new();
        for (n, name) in names.enumerate() {
            let port = ports
                .iter()
                .find(|p| p.name == name && p.direction == Direction::Output)
                .with_context(|| format!("missing ALU54 output {name}"))?;
            let signal = self.signal(port.wire.clone())?;
            let label = format!("alu54:{key}:{name}");
            self.label(&label)?;
            let initial = match n {
                0..WIDTH => Logic::Zero,
                n if n < WIDTH + 8 && config.flag_clock.is_none() => config.flags[n - WIDTH],
                n if n < WIDTH + 8 => Logic::Zero,
                _ => Logic::X,
            };
            drivers.push(self.driver(signal, label, Strength::Strong, initial, false));
            signals.push(signal);
        }
        let index = self.alus.len();
        for input in inputs {
            if let Some(signal) = input.signal() {
                self.alu_readers.push((signal.index, index));
            }
        }
        self.bound_alus.insert(config.resource);
        self.alus.push(Alu {
            config,
            register: [Logic::Zero; WIDTH],
            flag_register: [Logic::Zero; 8],
            inputs: AluInputs {
                opcode: [[[Logic::Zero; 11]; 2]; 3],
                c: [Logic::Zero; WIDTH],
                top: [[Logic::Zero; 18]; 2],
                divider: [Logic::Zero; 2],
            },
            previous_clock: Logic::X,
            global_at_rise: Logic::Zero,
            opcode_global_at_rise: [[Logic::Zero; 2]; 3],
            previous_flag_clock: Logic::X,
            previous_opcode_clocks: [[Logic::X; 2]; 3],
            previous_c_clocks: [Logic::X; 2],
            drivers,
        });
        Ok(Alu54Outputs {
            r: signals[..WIDTH].try_into().unwrap(),
            flags: signals[WIDTH..WIDTH + 8].try_into().unwrap(),
            signed_r: signals[WIDTH + 8],
        })
    }
}

impl Kernel {
    pub(super) fn initialize_alu_clocks(&mut self) {
        for alu in &mut self.alus {
            alu.previous_clock = alu.config.clock.read(&self.signals);
            alu.previous_flag_clock = alu
                .config
                .flag_clock
                .map_or(Logic::Zero, |c| c.read(&self.signals));
            let clock =
                |r: &Option<AluRegister>| r.map_or(Logic::Zero, |r| r.clock.read(&self.signals));
            alu.previous_opcode_clocks = alu.config.opcode.map(|g| g.map(|s| clock(&s)));
            alu.previous_c_clocks = alu.config.c_registers.map(|r| clock(&r));
        }
    }
    /// The register captures of the dirty ALUs (`indices` count from the
    /// first ALU), from the state before this wave.
    pub(super) fn capture_alus(&mut self, indices: &[usize]) -> Result<Vec<AluCapture>> {
        let mut captures = Vec::new();
        for &index in indices {
            let alu = &self.alus[index];
            let level = |input: Input| match input.read(&self.signals) {
                Logic::Z => Logic::X,
                v => v,
            };
            let resets = alu.config.resets.map(level);
            let reset = |k: usize| resets[usize::from(ALU_FIRST_HALF >> k & 1 == 0)];
            let live_global = alu.config.global_reset.map_or(Logic::Zero, level);
            let mut global_at_rise = alu.global_at_rise;
            let global = super::dsp::dsp_global(
                alu.config.both_edges,
                alu.previous_clock,
                level(alu.config.clock),
                live_global,
                &mut global_at_rise,
            );
            let mut opcode_global_at_rise = alu.opcode_global_at_rise;
            let asynchronous = |k: usize| alu.config.async_reset >> k & 1 == 1;
            let clock = alu.config.clock.read(&self.signals);
            let edge = super::dsp::dsp_edge(alu.config.both_edges, alu.previous_clock, clock);
            if edge == Activity::Possible {
                self.stats.possible_clock_edges += 1;
            }
            let mut next = if edge == Activity::None {
                alu.register
            } else {
                let result = alu.result(&self.signals);
                let enable = match alu.config.enable.read(&self.signals) {
                    Logic::Z => Logic::X,
                    v => v,
                };
                std::array::from_fn(|k| {
                    let mut captured = Logic::mux(enable, alu.register[k], result[k]);
                    // A synchronous reset wins over the enable.
                    if !asynchronous(k) {
                        captured = Logic::mux(reset(k), captured, Logic::Zero);
                    }
                    if edge == Activity::Definite {
                        captured
                    } else {
                        Logic::mux(Logic::X, alu.register[k], captured)
                    }
                })
            };
            // An asynchronous reset and GSR clear at any time, the edge
            // included.
            for (k, bit) in next.iter_mut().enumerate() {
                if asynchronous(k) {
                    *bit = Logic::mux(reset(k), *bit, Logic::Zero);
                }
                *bit = Logic::mux(global, *bit, Logic::Zero);
            }
            // The flag register takes the constant flags the same way.
            let flag_clock = alu
                .config
                .flag_clock
                .map_or(Logic::Zero, |c| c.read(&self.signals));
            let flag_edge = clock_edge(Edge::Rising, alu.previous_flag_clock, flag_clock);
            let next_flags = if flag_edge == Activity::None {
                alu.flag_register
            } else {
                let enable = match alu.config.enable.read(&self.signals) {
                    Logic::Z => Logic::X,
                    v => v,
                };
                std::array::from_fn(|k| {
                    let captured = Logic::mux(enable, alu.flag_register[k], alu.config.flags[k]);
                    if flag_edge == Activity::Definite {
                        captured
                    } else {
                        Logic::mux(Logic::X, alu.flag_register[k], captured)
                    }
                })
            };
            // The input registers (HD08), from the state before this wave.
            let config = &alu.config;
            let old = &alu.inputs;
            let mut inputs = *old;
            let mut opcode_clocks = alu.previous_opcode_clocks;
            for (g, bits) in ALU_OPCODE_GROUPS.iter().enumerate() {
                for (s, register) in config.opcode[g].iter().enumerate() {
                    let Some(r) = register else {
                        continue;
                    };
                    let clock = level(r.clock);
                    let global = super::dsp::dsp_global(
                        config.both_edges,
                        alu.previous_opcode_clocks[g][s],
                        clock,
                        live_global,
                        &mut opcode_global_at_rise[g][s],
                    );
                    let edge = super::dsp::dsp_edge(
                        config.both_edges,
                        alu.previous_opcode_clocks[g][s],
                        clock,
                    );
                    opcode_clocks[g][s] = clock;
                    let (enable, reset) = (level(r.enable), level(r.reset));
                    for k in bits.clone() {
                        // The second stage takes the first's output where the
                        // first is on.
                        let data = if s == 1 && config.opcode[g][0].is_some() {
                            old.opcode[g][0][k]
                        } else {
                            level(config.op[k])
                        };
                        inputs.opcode[g][s][k] = capture_bit(
                            old.opcode[g][s][k],
                            data,
                            edge,
                            enable,
                            reset,
                            r.asynchronous,
                            global,
                        );
                    }
                }
            }
            let mut c_clocks = alu.previous_c_clocks;
            for (n, bits) in ALU_C_GROUPS.iter().enumerate() {
                let Some(r) = config.c_registers[n] else {
                    continue;
                };
                let clock = level(r.clock);
                let edge = clock_edge(Edge::Rising, alu.previous_c_clocks[n], clock);
                c_clocks[n] = clock;
                let global = live_global;
                // Every other edge: the one before which the divider is 0.
                let divider = old.divider[n];
                let counted = if config.both_edges {
                    edge
                } else {
                    match (edge, divider) {
                        (Activity::None, _) | (_, Logic::One) => Activity::None,
                        (edge, Logic::Zero) => edge,
                        _ => Activity::Possible,
                    }
                };
                let (enable, reset) = (level(r.enable), level(r.reset));
                let bit = |old: Logic, data: Input| {
                    capture_bit(
                        old,
                        level(data),
                        counted,
                        enable,
                        reset,
                        r.asynchronous,
                        global,
                    )
                };
                for k in bits.clone() {
                    inputs.c[k] = bit(old.c[k], config.c[k]);
                }
                let top = if n == 0 { &config.a } else { &config.b };
                for k in 0..18 {
                    inputs.top[n][k] = bit(old.top[n][k], top[36 + k]);
                }
                let toggled = match edge {
                    Activity::None => divider,
                    Activity::Definite => divider.invert(),
                    Activity::Possible => Logic::mux(Logic::X, divider, divider.invert()),
                };
                inputs.divider[n] = Logic::mux(global, toggled, Logic::Zero);
            }
            let changed =
                next != alu.register || next_flags != alu.flag_register || inputs != alu.inputs;
            let alu = &mut self.alus[index];
            alu.previous_clock = clock;
            alu.global_at_rise = global_at_rise;
            alu.opcode_global_at_rise = opcode_global_at_rise;
            alu.previous_flag_clock = flag_clock;
            alu.previous_opcode_clocks = opcode_clocks;
            alu.previous_c_clocks = c_clocks;
            if changed {
                captures.push(AluCapture {
                    index,
                    register: next,
                    flag_register: next_flags,
                    inputs,
                });
            }
        }
        Ok(captures)
    }
    /// Stores the captures; returns the ALUs to evaluate again.
    pub(super) fn commit_alus(&mut self, captures: Vec<AluCapture>) -> Vec<usize> {
        captures
            .into_iter()
            .map(|c| {
                self.alus[c.index].register = c.register;
                self.alus[c.index].flag_register = c.flag_register;
                self.alus[c.index].inputs = c.inputs;
                c.index
            })
            .collect()
    }
    /// The output updates of one ALU.
    pub(super) fn evaluate_alu(&self, index: usize, updates: &mut Vec<Update>) {
        let alu = &self.alus[index];
        let r = alu.outputs(&self.signals);
        let values = r.into_iter().chain(alu.flag_outputs()).chain([Logic::X]);
        for (&driver, value) in alu.drivers.iter().zip(values) {
            updates.push(Update {
                driver,
                value,
                reason: Reason::Static(
                    "ALU54B: R is the output register (the result where it is bypassed, 0 where it has no clock); flags constant as on the board; SIGNEDR unknown",
                ),
            });
        }
    }
}
