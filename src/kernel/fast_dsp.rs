//! MULT18X18D in fast mode ([`super::fast`]): the multiplier's output nets as
//! nodes of the netlist, next to the gates, and its INPUT/PIPELINE/OUTPUT
//! register stages as captures, like the IOLOGIC ones.
//!
//! As in kernel/dsp.rs:
//! - the operands are the INPUTA/INPUTB registers when those stages are on,
//!   else the A/B inputs; ROA/ROB are the operands;
//! - P is the OUTPUT register, else the PIPELINE register, else the product
//!   of the operands, `v[p] = mul18(A, B, SA, SB) >> k & 1` with `mul18` in
//!   the prelude ([`PRELUDE`]). The 36 lines of one product read the same
//!   slots and stand side by side, but clang merges few of them: in
//!   `examples/ulx3s/mac` the 152 such lines of `converge` became 128
//!   multiplications, each line gathering its 36 operand bits again. One line
//!   writing all 36 bits would need the multi-output support the line model
//!   does not have (layout, `FAST_ACTIVITY`);
//! - SIGNEDP is the PIPELINE sign register, else SIGNEDA | SIGNEDB of the
//!   operands; after an OUTPUT stage the kernel keeps it X, and so does this;
//! - a stage register takes `enable ? data : itself` at a rising edge of its
//!   clock (both edges with CLK_DIV=DISABLED), a synchronous reset to 0 first;
//!   an asynchronous local reset and
//!   the global reset clear it at any time. All stages sample the state from
//!   before the edge. The OUTPUT stage takes CE0 and its selected reset
//!   (HD01), and GSR like the other stages (HD07), as the kernel does.
//!
//! Every line writes one slot and reads only slots, so the netlist passes
//! (ordering, splitting, threads, activity) take it as any other line; it is
//! opaque to folding.
use super::alu::{Alu54, AluRegister};
use super::fast::{FastKernel, Term};
use super::*;

/// One output net of a multiplier.
#[derive(Clone, Copy, Debug)]
pub(super) enum DspOutput {
    Product(usize),
    Signed,
    OperandA(usize),
    OperandB(usize),
    /// Bit k of an ALU54B's R; the node's first number is then the ALU.
    AluR(usize),
    /// A flag of an ALU54B, a constant.
    AluFlag(usize),
}

/// What a node's net is: a copy of a term (a constant, another net or a
/// register slot), or a line computing it (C right-hand side, terms read).
pub(super) enum NodeValue {
    Alias(Term),
    Line(String, Vec<Term>),
}

/// The multiplier output nets of a kernel, each a node of the netlist.
pub(super) struct DspNodes {
    /// (multiplier, output) of each node.
    nodes: Vec<(usize, DspOutput)>,
    /// The kernel driver of each node.
    drivers: Vec<usize>,
    of_driver: HashMap<usize, usize>,
}

/// The first slot of each register stage of one multiplier: INPUTA and
/// INPUTB hold 18 data bits and the sign, PIPELINE 36 product bits and the
/// sign, OUTPUT 36 product bits.
#[derive(Clone, Copy, Default)]
pub(super) struct StageSlots {
    input_a: Option<usize>,
    input_b: Option<usize>,
    product: Option<usize>,
    output: Option<usize>,
    /// GSR sampled at each stage's rise, held through the fall (HD09).
    global_rise: [Option<usize>; 4],
}

/// The register slots of one ALU54B: the first of its output register (54
/// bits), of its flag register (8 bits) when the flags are registered, of
/// each opcode stage (its group's bits) and of each C register (its C bits,
/// the 18 top bits of A or B, and its divider with CLK_DIV=ENABLED).
#[derive(Clone, Copy)]
pub(super) struct AluSlots {
    r: usize,
    flags: Option<usize>,
    opcode: [[Option<usize>; 2]; 3],
    c: [Option<usize>; 2],
    global_rise: Option<usize>,
    opcode_global_rise: [[Option<usize>; 2]; 3],
}

/// The register stage slots of every multiplier and ALU54B.
pub(super) struct DspRegisters {
    multipliers: Vec<StageSlots>,
    alus: Vec<AluSlots>,
}

/// One register bit of a stage, captured at a rising edge of `clock`.
pub(super) struct DspCapture {
    pub slot: usize,
    pub clock: Term,
    /// Also capture on falling edges (DISABLED non-C stages only).
    pub both_edges: bool,
    /// The next value at an edge: enable, data and synchronous reset.
    pub next: String,
    /// Conditions that clear the register at any time, in C.
    pub clears: Vec<String>,
    /// Terms the capture reads.
    pub reads: Vec<Term>,
}

/// The C prelude of a program with multipliers: before `typedef uint8_t u8;`,
/// so every unit of a split program has it.
pub(super) const PRELUDE: &str = "/* MULT18X18D: the 36-bit product of two 18-bit operands, each two's
   complement when its sign control is 1 (kernel/dsp.rs multiply18). */
static inline __attribute__((always_inline, const)) unsigned long long mul18(int a, int b, int sa, int sb) {
  long long x = sa && (a >> 17 & 1) ? a - (1 << 18) : a;
  long long y = sb && (b >> 17 & 1) ? b - (1 << 18) : b;
  return (unsigned long long)(x * y) & 0xFFFFFFFFFull;
}
/* ALU54B (kernel/alu.rs): X, Y and Z by OP[6:0] from R, the products ma and
   mb (36 bits, sign-extended when sma, smb), A, B, C, CIN (its >> 18 filled
   with CIN[53] & smb), RNDPAT and its trailing-zero mask; OP[10:9] = 1 a sum
   with OP7 and OP8 negating Y and Z, else NAND, AND or XNOR of Y and Z,
   inverted likewise. */
static inline __attribute__((always_inline, const)) unsigned long long alu54(
    unsigned op, unsigned long long r, unsigned long long ma, int sma, unsigned long long a,
    unsigned long long mb, int smb, unsigned long long b, unsigned long long cin,
    unsigned long long c, unsigned long long rnd) {
  const unsigned long long m = 0x3FFFFFFFFFFFFFull, top = 0x3FFFFull << 36;
  if (sma && (ma >> 35 & 1)) ma |= top;
  if (smb && (mb >> 35 & 1)) mb |= top;
  unsigned long long x = (op & 3) == 0 ? r : (op & 3) == 1 ? ma : (op & 3) == 2 ? a : 0;
  unsigned long long y = (op >> 2 & 3) == 0 ? mb << 18 : (op >> 2 & 3) == 1 ? mb : (op >> 2 & 3) == 2 ? b : 0;
  unsigned long long z = 0;
  switch (op >> 4 & 7) {
    case 1: z = cin >> 18 | ((cin >> 53 & 1) && smb ? top : 0); break;
    case 2: z = cin; break;
    case 3: z = c; break;
    case 4: z = a; break;
    case 5: z = r; break;
    case 6: z = rnd; break;
    case 7: z = ~rnd & (rnd - 1) & m; break; /* RNDPAT's trailing zeros (HD08z) */
  }
  unsigned kind = op >> 9 & 3;
  if (kind == 1) return (((op >> 8 & 1) ? -z : z) + x + ((op >> 7 & 1) ? -y : y)) & m;
  if (op >> 7 & 1) y = ~y;
  if (op >> 8 & 1) z = ~z;
  return (kind == 0 ? ~(y & z) : kind == 2 ? (y & z) : ~(y ^ z)) & m;
}";

/// The operands of a multiplier: A, B, SIGNEDA, SIGNEDB.
type Operands = (Vec<Term>, Vec<Term>, Term, Term);
/// A register stage to capture: its first slot, its control, whether the
/// global reset clears it, and the data of each bit (C and terms read).
type Stage = (usize, MultiplierRegister, bool, Vec<(String, Vec<Term>)>);

impl DspNodes {
    /// The nodes of every multiplier.
    pub(super) fn new(kernel: &FastKernel<'_>) -> Result<Self> {
        let mut nodes = Vec::new();
        let mut drivers = Vec::new();
        for (m, multiplier) in kernel.multipliers.iter().enumerate() {
            let output_stage = multiplier.pipeline().output.is_some();
            if let Some(product) = multiplier.product_drivers() {
                for (k, &d) in product.iter().enumerate() {
                    // SIGNEDP after an OUTPUT stage is X in the kernel: its
                    // net stays a constant X, which the program cannot read.
                    if k == 36 && output_stage {
                        continue;
                    }
                    nodes.push((
                        m,
                        if k < 36 {
                            DspOutput::Product(k)
                        } else {
                            DspOutput::Signed
                        },
                    ));
                    drivers.push(d);
                }
            }
            for (k, &d) in multiplier.operand_drivers().iter().enumerate() {
                nodes.push((
                    m,
                    if k < 18 {
                        DspOutput::OperandA(k)
                    } else {
                        DspOutput::OperandB(k - 18)
                    },
                ));
                drivers.push(d);
            }
        }
        // The ALUs: R and the flags. SIGNEDR is X in the kernel: its net stays
        // a constant X, which the program cannot read.
        for (i, alu) in kernel.alus.iter().enumerate() {
            for (k, &d) in alu.r_drivers().iter().enumerate() {
                nodes.push((i, DspOutput::AluR(k)));
                drivers.push(d);
            }
            for (k, &d) in alu.flag_drivers().iter().enumerate() {
                nodes.push((i, DspOutput::AluFlag(k)));
                drivers.push(d);
            }
        }
        let of_driver = drivers.iter().enumerate().map(|(n, &d)| (d, n)).collect();
        Ok(Self {
            nodes,
            drivers,
            of_driver,
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The node a kernel driver is, if it drives a multiplier output.
    pub(super) fn of_driver(&self, driver: usize) -> Option<usize> {
        self.of_driver.get(&driver).copied()
    }

    /// The kernel driver of node `n`.
    pub(super) fn driver(&self, n: usize) -> usize {
        self.drivers[n]
    }

    /// Slots for the register stages of every multiplier, from `*slots` on,
    /// with their values from the kernel's state pushed onto `initial`.
    pub(super) fn registers(
        &self,
        kernel: &FastKernel<'_>,
        slots: &mut usize,
        initial: &mut Vec<u8>,
    ) -> Result<DspRegisters> {
        let mut all = Vec::new();
        for multiplier in &kernel.multipliers {
            let id = multiplier.config().resource;
            let key = &kernel.device.model().resource(id).key;
            let pipeline = multiplier.pipeline();
            let state = kernel.state.multiplier(id)?;
            let mut stage = |on: bool, values: Vec<Logic>| -> Result<Option<usize>> {
                if !on {
                    return Ok(None);
                }
                let first = *slots;
                for v in values {
                    initial.push(match v.bit() {
                        Some(b) => u8::from(b),
                        None => {
                            return Err(StartsUnknown(format!(
                                "a register stage of the MULT18 {key}"
                            ))
                            .into())
                        }
                    });
                }
                *slots = initial.len();
                Ok(Some(first))
            };
            let with_sign = |bits: &[Logic], sign: Logic| {
                bits.iter().copied().chain([sign]).collect::<Vec<_>>()
            };
            let input_a = stage(
                pipeline.input_a.is_some(),
                with_sign(&state.input_a, state.signed_a),
            )?;
            let input_b = stage(
                pipeline.input_b.is_some(),
                with_sign(&state.input_b, state.signed_b),
            )?;
            let product = stage(
                pipeline.product.is_some(),
                with_sign(&state.product, state.signed_product),
            )?;
            let output = if pipeline.output.is_some() {
                stage(true, kernel.state.multiplier_output(id)?.to_vec())?
            } else {
                None
            };
            let mut global_rise = [None; 4];
            if pipeline.both_edges && pipeline.global_reset.is_some() {
                for (n, active) in [input_a, input_b, product, output].iter().enumerate() {
                    global_rise[n] = stage(active.is_some(), vec![multiplier.global_at_rise[n]])?;
                }
            }
            all.push(StageSlots {
                input_a,
                input_b,
                product,
                output,
                global_rise,
            });
        }
        let mut alus = Vec::new();
        for alu in &kernel.alus {
            let c = alu.config();
            let key = &kernel.device.model().resource(c.resource).key;
            let mut push = |values: &mut dyn Iterator<Item = Logic>| -> Result<usize> {
                let first = *slots;
                for v in values {
                    initial.push(match v.bit() {
                        Some(b) => u8::from(b),
                        None => {
                            return Err(
                                StartsUnknown(format!("a register of the ALU54 {key}")).into()
                            )
                        }
                    });
                }
                *slots = initial.len();
                Ok(first)
            };
            let r = push(&mut alu.register().iter().copied())?;
            let flags = match c.flag_clock {
                Some(_) => Some(push(&mut alu.flag_register().iter().copied())?),
                None => None,
            };
            let state = alu.input_registers();
            let mut opcode = [[None; 2]; 3];
            for (g, bits) in ALU_OPCODE_GROUPS.iter().enumerate() {
                for (s, slot) in opcode[g].iter_mut().enumerate() {
                    if c.opcode[g][s].is_some() {
                        *slot = Some(push(&mut state.opcode[g][s][bits.clone()].iter().copied())?);
                    }
                }
            }
            let mut cs = [None; 2];
            for (n, bits) in ALU_C_GROUPS.iter().enumerate() {
                if c.c_registers[n].is_some() {
                    let values = state.c[bits.clone()]
                        .iter()
                        .chain(&state.top[n])
                        .chain((!c.both_edges).then_some(&state.divider[n]));
                    cs[n] = Some(push(&mut values.copied())?);
                }
            }
            let mut global_rise = None;
            let mut opcode_global_rise = [[None; 2]; 3];
            if c.both_edges && c.global_reset.is_some() {
                if !(c.unregistered | c.stuck) & ((1 << 54) - 1) != 0 {
                    global_rise = Some(push(&mut [alu.global_at_rise].into_iter())?);
                }
                for (g, slots) in opcode_global_rise.iter_mut().enumerate() {
                    for (s, slot) in slots.iter_mut().enumerate() {
                        if c.opcode[g][s].is_some() {
                            *slot = Some(push(&mut [alu.opcode_global_at_rise[g][s]].into_iter())?);
                        }
                    }
                }
            }
            alus.push(AluSlots {
                r,
                flags,
                opcode,
                c: cs,
                global_rise,
                opcode_global_rise,
            });
        }
        Ok(DspRegisters {
            multipliers: all,
            alus,
        })
    }

    /// The inputs node `n` depends on: those its value reads that are not
    /// register stages.
    pub(super) fn inputs(
        &self,
        kernel: &FastKernel<'_>,
        regs: &DspRegisters,
        n: usize,
    ) -> Vec<Input> {
        let (m, output) = self.nodes[n];
        match output {
            // A registered bit and a flag read no input; a bit before the
            // register reads what the result reads.
            DspOutput::AluR(k) => {
                let c = kernel.alus[m].config();
                return if c.unregistered >> k & 1 == 1 {
                    alu_inputs(c)
                } else {
                    Vec::new()
                };
            }
            DspOutput::AluFlag(_) => return Vec::new(),
            _ => {}
        }
        let c = kernel.multipliers[m].config();
        let s = regs.multipliers[m];
        let a = |v: &mut Vec<Input>| {
            if s.input_a.is_none() {
                v.extend(c.a);
                v.push(c.signed_a);
            }
        };
        let b = |v: &mut Vec<Input>| {
            if s.input_b.is_none() {
                v.extend(c.b);
                v.push(c.signed_b);
            }
        };
        let mut v = Vec::new();
        match output {
            DspOutput::Product(_) | DspOutput::Signed
                if s.output.is_none() && s.product.is_none() =>
            {
                a(&mut v);
                b(&mut v);
            }
            DspOutput::Product(_) | DspOutput::Signed => {}
            DspOutput::OperandA(k) if s.input_a.is_none() => v.push(c.a[k]),
            DspOutput::OperandB(k) if s.input_b.is_none() => v.push(c.b[k]),
            DspOutput::OperandA(_) | DspOutput::OperandB(_) => {}
            DspOutput::AluR(_) | DspOutput::AluFlag(_) => unreachable!(),
        }
        v
    }

    /// The operands of multiplier `m`: its INPUTA/B registers, or its inputs.
    fn operands(
        kernel: &FastKernel<'_>,
        regs: &DspRegisters,
        m: usize,
        term: &dyn Fn(Input) -> Result<Term>,
    ) -> Result<Operands> {
        let c = kernel.multipliers[m].config();
        let s = regs.multipliers[m];
        let register = |slot: usize| Term::Net {
            slot,
            inverted: false,
        };
        let side = |first: Option<usize>, bits: &[Input; 18], sign: Input| -> Result<_> {
            Ok(match first {
                Some(f) => ((0..18).map(|k| register(f + k)).collect(), register(f + 18)),
                None => (
                    bits.iter().map(|&i| term(i)).collect::<Result<Vec<_>>>()?,
                    term(sign)?,
                ),
            })
        };
        let (a, sa) = side(s.input_a, &c.a, c.signed_a)?;
        let (b, sb) = side(s.input_b, &c.b, c.signed_b)?;
        Ok((a, b, sa, sb))
    }

    /// The kernel stops at SOURCEA/B other than 0 (kernel/dsp.rs).
    fn check_sources(
        kernel: &FastKernel<'_>,
        m: usize,
        term: &dyn Fn(Input) -> Result<Term>,
    ) -> Result<()> {
        let c = kernel.multipliers[m].config();
        for source in [c.source_a, c.source_b] {
            ensure!(
                matches!(term(source), Ok(Term::Lit(0))),
                "fast mode runs parallel MULT18 inputs; {} has SOURCEA/B other than 0",
                kernel.device.model().resource(c.resource).key
            );
        }
        Ok(())
    }

    /// What node `n` computes, with `term` giving each input as a term.
    pub(super) fn value(
        &self,
        kernel: &FastKernel<'_>,
        regs: &DspRegisters,
        n: usize,
        term: &dyn Fn(Input) -> Result<Term>,
    ) -> Result<NodeValue> {
        let (m, output) = self.nodes[n];
        match output {
            DspOutput::AluFlag(k) => {
                let flag = kernel.alus[m].config().flags[k];
                return Ok(NodeValue::Alias(match regs.alus[m].flags {
                    Some(first) => Term::Net {
                        slot: first + k,
                        inverted: false,
                    },
                    None => Term::Lit(u8::from(flag == Logic::One)),
                }));
            }
            DspOutput::AluR(k) => {
                let c = kernel.alus[m].config();
                if c.stuck >> k & 1 == 1 {
                    return Ok(NodeValue::Alias(Term::Lit(0)));
                }
                if c.unregistered >> k & 1 == 0 {
                    return Ok(NodeValue::Alias(Term::Net {
                        slot: regs.alus[m].r + k,
                        inverted: false,
                    }));
                }
                let (expr, reads) = alu_result(kernel, regs, m, term)?;
                return Ok(NodeValue::Line(format!("{expr} >> {k} & 1"), reads));
            }
            _ => {}
        }
        Self::check_sources(kernel, m, term)?;
        let s = regs.multipliers[m];
        let register = |slot: usize| {
            NodeValue::Alias(Term::Net {
                slot,
                inverted: false,
            })
        };
        let (a, b, sa, sb) = Self::operands(kernel, regs, m, term)?;
        Ok(match output {
            DspOutput::OperandA(k) => NodeValue::Alias(a[k]),
            DspOutput::OperandB(k) => NodeValue::Alias(b[k]),
            DspOutput::Product(k) => match (s.output, s.product) {
                (Some(o), _) => register(o + k),
                (None, Some(p)) => register(p + k),
                (None, None) => match product(&a, &b, sa, sb, k) {
                    (expr, read) if read.is_empty() => {
                        NodeValue::Alias(Term::Lit(expr.parse().expect("a literal product bit")))
                    }
                    (expr, read) => NodeValue::Line(expr, read),
                },
            },
            DspOutput::Signed => match s.product {
                Some(p) => register(p + 36),
                None => match signs(sa, sb) {
                    Ok(t) => NodeValue::Alias(t),
                    Err((expr, read)) => NodeValue::Line(expr, read),
                },
            },
            DspOutput::AluR(_) | DspOutput::AluFlag(_) => unreachable!(),
        })
    }

    /// Every register bit of every stage, as a capture.
    pub(super) fn captures(
        &self,
        kernel: &FastKernel<'_>,
        regs: &DspRegisters,
        term: &dyn Fn(Input) -> Result<Term>,
    ) -> Result<Vec<DspCapture>> {
        let mut captures = Vec::new();
        for (m, multiplier) in kernel.multipliers.iter().enumerate() {
            let s = regs.multipliers[m];
            if s.input_a.is_none()
                && s.input_b.is_none()
                && s.product.is_none()
                && s.output.is_none()
            {
                continue;
            }
            Self::check_sources(kernel, m, term)?;
            let c = multiplier.config();
            let pipeline = multiplier.pipeline();
            let global = pipeline.global_reset.map(term).transpose()?;
            let (a, b, sa, sb) = Self::operands(kernel, regs, m, term)?;
            // (first slot, control, data of each bit as C and terms read).
            let mut stages: Vec<Stage> = Vec::new();
            let input = |bits: &[Input; 18], sign: Input| -> Result<Vec<(String, Vec<Term>)>> {
                bits.iter()
                    .chain([&sign])
                    .map(|&i| {
                        let t = term(i)?;
                        Ok((t.c(), vec![t]))
                    })
                    .collect()
            };
            if let (Some(f), Some(r)) = (s.input_a, pipeline.input_a) {
                stages.push((f, r, true, input(&c.a, c.signed_a)?));
            }
            if let (Some(f), Some(r)) = (s.input_b, pipeline.input_b) {
                stages.push((f, r, true, input(&c.b, c.signed_b)?));
            }
            if let (Some(f), Some(r)) = (s.product, pipeline.product) {
                let mut data: Vec<_> = (0..36).map(|k| product(&a, &b, sa, sb, k)).collect();
                data.push(match signs(sa, sb) {
                    Ok(t) => (t.c(), vec![t]),
                    Err(line) => line,
                });
                stages.push((f, r, true, data));
            }
            if let (Some(f), Some(o)) = (s.output, pipeline.output) {
                let data = (0..36)
                    .map(|k| match s.product {
                        Some(p) => {
                            let t = Term::Net {
                                slot: p + k,
                                inverted: false,
                            };
                            (t.c(), vec![t])
                        }
                        None => product(&a, &b, sa, sb, k),
                    })
                    .collect();
                let control = MultiplierRegister {
                    clock: o.clock,
                    enable: o.enable,
                    reset: o.reset,
                    reset_kind: o.reset_kind,
                };
                // GSR clears the OUTPUT stage like the others (HD07).
                stages.push((f, control, true, data));
            }
            for (first, control, global_resets, data) in stages {
                let clock = term(control.clock)?;
                let enable = term(control.enable)?;
                let reset = term(control.reset)?;
                let stage = [s.input_a, s.input_b, s.product, s.output]
                    .iter()
                    .position(|&slot| slot == Some(first))
                    .expect("an allocated stage");
                let held = s.global_rise[stage];
                if let (Some(slot), Some(global)) = (held, global) {
                    captures.push(gsr_capture(slot, clock, global));
                }
                for (k, (expr, mut reads)) in data.into_iter().enumerate() {
                    let slot = first + k;
                    let mut next = match enable {
                        Term::Lit(1) => format!("({expr})"),
                        Term::Lit(_) => format!("v[{slot}]"),
                        e => format!("({} ? ({expr}) : v[{slot}])", e.c()),
                    };
                    let mut clears = Vec::new();
                    match (control.reset_kind, reset) {
                        (_, Term::Lit(0)) => {}
                        (ResetKind::Synchronous, r) => next = format!("({} ? 0 : {next})", r.c()),
                        (ResetKind::Asynchronous, r) => clears.push(r.c()),
                    }
                    if let Some(g) = global.filter(|_| global_resets) {
                        if g != Term::Lit(0) {
                            clears.push(gsr_clear(clock, g, held));
                        }
                    }
                    reads.extend([enable, reset]);
                    if global_resets {
                        reads.extend(global);
                        reads.extend(gsr_reads(clock, held));
                    }
                    captures.push(DspCapture {
                        slot,
                        clock,
                        both_edges: pipeline.both_edges,
                        next,
                        clears,
                        reads,
                    });
                }
            }
        }
        // The ALUs' output registers: every registered bit takes its bit of
        // the result at the selected edges while CE0 is high.
        for (i, alu) in kernel.alus.iter().enumerate() {
            let c = alu.config();
            captures.extend(alu_input_captures(kernel, regs, i, term)?);
            let enable = term(c.enable)?;
            // The flag register takes its constants the same way.
            if let (Some(first), Some(flag_clock)) = (regs.alus[i].flags, c.flag_clock) {
                let clock = term(flag_clock)?;
                for (k, flag) in c.flags.iter().enumerate() {
                    let slot = first + k;
                    let flag = u8::from(*flag == Logic::One);
                    captures.push(DspCapture {
                        slot,
                        clock,
                        both_edges: false,
                        next: match enable {
                            Term::Lit(1) => flag.to_string(),
                            Term::Lit(_) => format!("v[{slot}]"),
                            e => format!("({} ? {flag} : v[{slot}])", e.c()),
                        },
                        clears: Vec::new(),
                        reads: vec![enable],
                    });
                }
            }
            let registered = !(c.unregistered | c.stuck) & ((1 << 54) - 1);
            if registered == 0 {
                continue;
            }
            let (expr, reads) = alu_result(kernel, regs, i, term)?;
            let clock = term(c.clock)?;
            let resets = [term(c.resets[0])?, term(c.resets[1])?];
            let global = c.global_reset.map(term).transpose()?;
            let held = regs.alus[i].global_rise;
            if let (Some(slot), Some(global)) = (held, global) {
                captures.push(gsr_capture(slot, clock, global));
            }
            for k in (0..54).filter(|k| registered >> k & 1 == 1) {
                let reset = resets[usize::from(ALU_FIRST_HALF >> k & 1 == 0)];
                let slot = regs.alus[i].r + k;
                let bit = format!("({expr} >> {k} & 1)");
                let mut next = match enable {
                    Term::Lit(1) => bit,
                    Term::Lit(_) => format!("v[{slot}]"),
                    e => format!("({} ? {bit} : v[{slot}])", e.c()),
                };
                // The reset wins over the enable: at the edge when it is
                // synchronous, at any time when asynchronous.
                let mut clears = Vec::new();
                match (c.async_reset >> k & 1 == 1, reset) {
                    (_, Term::Lit(0)) => {}
                    (false, r) => next = format!("({} ? 0 : {next})", r.c()),
                    (true, r) => clears.push(r.c()),
                }
                // GSR clears every bit at any time.
                if let Some(g) = global.filter(|&g| g != Term::Lit(0)) {
                    clears.push(gsr_clear(clock, g, held));
                }
                let mut reads = reads.clone();
                reads.extend([enable, reset]);
                reads.extend(global);
                reads.extend(gsr_reads(clock, held));
                captures.push(DspCapture {
                    slot,
                    clock,
                    both_edges: c.both_edges,
                    next,
                    clears,
                    reads,
                });
            }
        }
        Ok(captures)
    }
}

/// A rising-only sample of GSR. The old held bit must not inhibit the next
/// rise, where live GSR replaces it; at the fall it keeps that rise's hold.
fn gsr_capture(slot: usize, clock: Term, global: Term) -> DspCapture {
    DspCapture {
        slot,
        clock,
        both_edges: false,
        next: global.c(),
        clears: Vec::new(),
        reads: vec![global],
    }
}

fn gsr_clear(clock: Term, global: Term, held: Option<usize>) -> String {
    match held {
        Some(slot) => format!("({} | (!({}) & v[{slot}]))", global.c(), clock.c()),
        None => global.c(),
    }
}

fn gsr_reads(clock: Term, held: Option<usize>) -> Vec<Term> {
    held.map_or_else(Vec::new, |slot| {
        vec![
            clock,
            Term::Net {
                slot,
                inverted: false,
            },
        ]
    })
}

/// OP of ALU `i` as terms: the slots of its last opcode stage where a group
/// has one, else its inputs.
fn alu_op_terms(
    kernel: &FastKernel<'_>,
    regs: &DspRegisters,
    i: usize,
    term: &dyn Fn(Input) -> Result<Term>,
) -> Result<Vec<Term>> {
    let c = kernel.alus[i].config();
    let s = regs.alus[i];
    let mut op: Vec<Term> = c.op.iter().map(|&x| term(x)).collect::<Result<_>>()?;
    for (g, bits) in ALU_OPCODE_GROUPS.iter().enumerate() {
        if let Some(first) = s.opcode[g][1].or(s.opcode[g][0]) {
            for (n, k) in bits.clone().enumerate() {
                op[k] = Term::Net {
                    slot: first + n,
                    inverted: false,
                };
            }
        }
    }
    Ok(op)
}

/// A, B or C (`which` 0, 1, 2) of ALU `i` as terms: the slots of its C
/// registers where they are on, else its inputs. Only a source the result
/// uses is asked for: an unused input may be floating.
fn alu_word_terms(
    kernel: &FastKernel<'_>,
    regs: &DspRegisters,
    i: usize,
    term: &dyn Fn(Input) -> Result<Term>,
    which: usize,
) -> Result<Vec<Term>> {
    let c = kernel.alus[i].config();
    let s = regs.alus[i];
    let inputs = [&c.a, &c.b, &c.c][which];
    let mut word: Vec<Option<Term>> = vec![None; 54];
    for (n, bits) in ALU_C_GROUPS.iter().enumerate() {
        let Some(first) = s.c[n] else {
            continue;
        };
        let net = |slot: usize| {
            Some(Term::Net {
                slot,
                inverted: false,
            })
        };
        match which {
            2 => {
                for (j, k) in bits.clone().enumerate() {
                    word[k] = net(first + j);
                }
            }
            w if w == n => {
                for j in 0..18 {
                    word[36 + j] = net(first + bits.len() + j);
                }
            }
            _ => {}
        }
    }
    word.into_iter()
        .zip(inputs)
        .map(|(slot, &input)| slot.map_or_else(|| term(input), Ok))
        .collect()
}

/// The captures of ALU `i`'s input registers (HD08): each bit takes its
/// data at a rising edge of its clock while enabled, a synchronous reset
/// first; an asynchronous reset and GSR clear it at any time. A C register
/// takes every rise with CLK_DIV=DISABLED; otherwise it takes only rises
/// before which its divider is 0. GSR holds the enabled divider at 0.
fn alu_input_captures(
    kernel: &FastKernel<'_>,
    regs: &DspRegisters,
    i: usize,
    term: &dyn Fn(Input) -> Result<Term>,
) -> Result<Vec<DspCapture>> {
    let c = kernel.alus[i].config();
    let s = regs.alus[i];
    let global = c
        .global_reset
        .map(term)
        .transpose()?
        .filter(|&g| g != Term::Lit(0));
    let net = |slot: usize| Term::Net {
        slot,
        inverted: false,
    };
    let mut captures = Vec::new();
    // One bit of register `r` at `slot` with `data`; `gate` the divider.
    let bit = |r: &AluRegister,
               slot: usize,
               data: Term,
               gate: Option<usize>,
               both_edges: bool,
               held: Option<usize>|
     -> Result<DspCapture> {
        let (clock, enable, reset) = (term(r.clock)?, term(r.enable)?, term(r.reset)?);
        let mut next = match enable {
            Term::Lit(1) => data.c(),
            Term::Lit(_) => format!("v[{slot}]"),
            e => format!("({} ? {} : v[{slot}])", e.c(), data.c()),
        };
        let mut clears = Vec::new();
        match (r.asynchronous, reset) {
            (_, Term::Lit(0)) => {}
            (false, rst) => next = format!("({} ? 0 : {next})", rst.c()),
            (true, rst) => clears.push(rst.c()),
        }
        if let Some(divider) = gate {
            next = format!("(v[{divider}] ? v[{slot}] : {next})");
        }
        clears.extend(global.map(|g| gsr_clear(clock, g, held)));
        let mut reads = vec![data, enable, reset];
        reads.extend(gate.map(net));
        reads.extend(global);
        reads.extend(gsr_reads(clock, held));
        Ok(DspCapture {
            slot,
            clock,
            both_edges,
            next,
            clears,
            reads,
        })
    };
    for (g, bits) in ALU_OPCODE_GROUPS.iter().enumerate() {
        for stage in 0..2 {
            let (Some(r), Some(first)) = (c.opcode[g][stage], s.opcode[g][stage]) else {
                continue;
            };
            let held = s.opcode_global_rise[g][stage];
            if let (Some(slot), Some(global)) = (held, global) {
                captures.push(gsr_capture(slot, term(r.clock)?, global));
            }
            for (n, k) in bits.clone().enumerate() {
                // The second stage takes the first's output where it is on.
                let data = match s.opcode[g][0].filter(|_| stage == 1) {
                    Some(f) => net(f + n),
                    None => term(c.op[k])?,
                };
                captures.push(bit(&r, first + n, data, None, c.both_edges, held)?);
            }
        }
    }
    for (n, bits) in ALU_C_GROUPS.iter().enumerate() {
        let (Some(r), Some(first)) = (c.c_registers[n], s.c[n]) else {
            continue;
        };
        let divider = first + bits.len() + 18;
        let top = if n == 0 { &c.a } else { &c.b };
        let data = bits
            .clone()
            .map(|k| c.c[k])
            .chain(top[36..].iter().copied());
        for (j, input) in data.enumerate() {
            captures.push(bit(
                &r,
                first + j,
                term(input)?,
                (!c.both_edges).then_some(divider),
                false,
                None,
            )?);
        }
        if c.both_edges {
            continue;
        }
        let clock = term(r.clock)?;
        let mut reads = Vec::new();
        reads.extend(global);
        captures.push(DspCapture {
            slot: divider,
            clock,
            both_edges: false,
            next: format!("(v[{divider}] ^ 1)"),
            clears: global.map(|g| g.c()).into_iter().collect(),
            reads,
        });
    }
    Ok(captures)
}

/// Every input an ALU54B's result may read.
fn alu_inputs(c: &Alu54) -> Vec<Input> {
    c.op.into_iter()
        .chain(c.ma)
        .chain(c.mb)
        .chain(c.a)
        .chain(c.b)
        .chain(c.c)
        .chain(c.cin)
        .chain([c.signed_ma, c.signed_mb])
        .collect()
}

/// The result word of ALU `i` as C (`alu54` in the prelude) and the terms it
/// reads. A source that the known bits of OP cannot select is passed as 0,
/// so a constant OP reads only what it uses.
fn alu_result(
    kernel: &FastKernel<'_>,
    regs: &DspRegisters,
    i: usize,
    term: &dyn Fn(Input) -> Result<Term>,
) -> Result<(String, Vec<Term>)> {
    let c = kernel.alus[i].config();
    let key = &kernel.device.model().resource(c.resource).key;
    let op = alu_op_terms(kernel, regs, i, term)?;
    // Whether the field of OP starting at `low`, `bits` wide, can be `value`.
    let can = |low: usize, bits: usize, value: usize| {
        (0..bits).all(|k| match op[low + k] {
            Term::Lit(b) => usize::from(b) == (value >> k & 1),
            Term::Net { .. } => true,
        })
    };
    let mut reads = op.clone();
    let (uses_a, uses_cin) = (can(0, 2, 2) || can(4, 3, 4), can(4, 3, 1) || can(4, 3, 2));
    let uses_mb = can(2, 2, 0) || can(2, 2, 1);
    // SIGNEDIB also fills the top of CIN >> 18.
    let (uses_ma, uses_smb) = (can(0, 2, 1), uses_mb || can(4, 3, 1));
    // A product straight from a multiplier is computed here from its
    // operands: the 36 product nets, a multiplication each, are then read by
    // nothing and the program need not keep them up at every cycle.
    let mut product = |bits: &[Input; 36],
                       sign: Input,
                       used: bool,
                       sign_used: bool|
     -> Result<(String, String)> {
        if let Some((p, sp, read)) = direct_product(kernel, regs, bits, sign, term)? {
            if used || sign_used {
                reads.extend(read);
            }
            return Ok((
                if used { p } else { "0".into() },
                if sign_used { sp } else { "0".into() },
            ));
        }
        let mut word = |inputs: &[Input], used: bool| -> Result<String> {
            if !used {
                return Ok("0".into());
            }
            let terms: Vec<Term> = inputs.iter().map(|&i| term(i)).collect::<Result<_>>()?;
            reads.extend(terms.iter().filter(|t| matches!(t, Term::Net { .. })));
            Ok(gather_word(&terms))
        };
        Ok((word(bits, used)?, word(&[sign], sign_used)?))
    };
    let (ma, sma) = product(&c.ma, c.signed_ma, uses_ma, uses_ma)?;
    let (mb, smb) = product(&c.mb, c.signed_mb, uses_mb, uses_smb)?;
    // Only a source the known bits of OP can select is read: an unused input
    // may be floating.
    let mut word = |terms: Option<Result<Vec<Term>>>| -> Result<String> {
        let Some(terms) = terms else {
            return Ok("0".into());
        };
        let terms = terms?;
        reads.extend(terms.iter().filter(|t| matches!(t, Term::Net { .. })));
        Ok(gather_word(&terms))
    };
    let source =
        |which: usize, used: bool| used.then(|| alu_word_terms(kernel, regs, i, term, which));
    let a = word(source(0, uses_a))?;
    let b = word(source(1, can(2, 2, 2)))?;
    let cin = word(uses_cin.then(|| c.cin.iter().map(|&x| term(x)).collect()))?;
    let cw = word(source(2, can(4, 3, 3)))?;
    // R as an operand: the register. A bit that leaves before the register
    // makes a loop the kernel reads as unknown.
    let uses_r = can(0, 2, 0) || can(4, 3, 5);
    ensure!(
        !uses_r || c.unregistered == 0,
        "fast mode does not run the ALU54B {key}: its OP can read R, and some bits of R are unregistered (a loop on the chip)"
    );
    let r = if uses_r {
        let first = regs.alus[i].r;
        // A bit without a clock is 0.
        let terms: Vec<Term> = (0..54)
            .map(|k| {
                if c.stuck >> k & 1 == 1 {
                    Term::Lit(0)
                } else {
                    Term::Net {
                        slot: first + k,
                        inverted: false,
                    }
                }
            })
            .collect();
        reads.extend(terms.iter().filter(|t| matches!(t, Term::Net { .. })));
        gather_word(&terms)
    } else {
        "0".into()
    };
    let op_word = gather(&op);
    Ok((
        format!(
            "alu54({op_word}, {r}, {ma}, {sma}, {a}, {mb}, {smb}, {b}, {cin}, {cw}, {:#x}ull)",
            c.rndpat
        ),
        reads,
    ))
}

/// When `bits` and `sign` are P0..P35 and SIGNEDP of one multiplier with no
/// PIPELINE or OUTPUT register: the product and its sign as C over the
/// multiplier's operands, and the terms that reads.
fn direct_product(
    kernel: &FastKernel<'_>,
    regs: &DspRegisters,
    bits: &[Input; 36],
    sign: Input,
    term: &dyn Fn(Input) -> Result<Term>,
) -> Result<Option<(String, String, Vec<Term>)>> {
    let Ok(wanted) = bits
        .iter()
        .chain([&sign])
        .map(|&i| term(i))
        .collect::<Result<Vec<Term>>>()
    else {
        return Ok(None);
    };
    for (m, multiplier) in kernel.multipliers.iter().enumerate() {
        let (Some(drivers), stages) = (multiplier.product_drivers(), regs.multipliers[m]) else {
            continue;
        };
        if stages.product.is_some() || stages.output.is_some() {
            continue;
        }
        let same = drivers.iter().zip(&wanted).all(|(&d, &w)| {
            let signal = SignalId {
                scope: kernel.scope,
                index: kernel.drivers[d].signal,
            };
            term(Input::Signal(signal)).is_ok_and(|t| t == w)
        });
        if !same || wanted.iter().any(|t| matches!(t, Term::Lit(_))) {
            continue;
        }
        DspNodes::check_sources(kernel, m, term)?;
        let (a, b, sa, sb) = DspNodes::operands(kernel, regs, m, term)?;
        let mut reads: Vec<Term> = a.iter().chain(&b).copied().collect();
        reads.extend([sa, sb]);
        reads.retain(|t| matches!(t, Term::Net { .. }));
        let signed = match signs(sa, sb) {
            Ok(t) => t.c(),
            Err((expr, _)) => format!("({expr})"),
        };
        return Ok(Some((
            format!(
                "mul18({}, {}, {}, {})",
                gather(&a),
                gather(&b),
                sa.c(),
                sb.c()
            ),
            signed,
            reads,
        )));
    }
    Ok(None)
}

/// A word of up to 54 bits from its bit terms, as an unsigned long long
/// expression.
fn gather_word(bits: &[Term]) -> String {
    let parts: Vec<String> = bits
        .iter()
        .enumerate()
        .filter(|(_, t)| !matches!(t, Term::Lit(0)))
        .map(|(i, t)| match (i, t) {
            (0, t) => format!("(unsigned long long){}", t.c()),
            (i, Term::Lit(_)) => format!("1ull << {i}"),
            (i, t) => format!("(unsigned long long){} << {i}", t.c()),
        })
        .collect();
    if parts.is_empty() {
        "0ull".into()
    } else {
        format!("({})", parts.join(" | "))
    }
}

/// Product bit `k` of the operands: a C expression and the terms it reads,
/// or a literal ("0"/"1") and nothing when every operand is constant.
fn product(a: &[Term], b: &[Term], sa: Term, sb: Term, k: usize) -> (String, Vec<Term>) {
    let mut read = a.to_vec();
    read.extend(b);
    read.extend([sa, sb]);
    if read.iter().all(|t| matches!(t, Term::Lit(_))) {
        let word = |w: &[Term]| -> i64 {
            w.iter().enumerate().fold(0, |x, (i, t)| match t {
                Term::Lit(1) => x | 1 << i,
                _ => x,
            })
        };
        let operand = |w: i64, s: Term| match s {
            Term::Lit(1) if w >> 17 & 1 == 1 => w - (1 << 18),
            _ => w,
        };
        let p = (operand(word(a), sa) * operand(word(b), sb)) as u64;
        return ((p >> k & 1).to_string(), Vec::new());
    }
    (
        format!(
            "mul18({}, {}, {}, {}) >> {k} & 1",
            gather(a),
            gather(b),
            sa.c(),
            sb.c()
        ),
        read,
    )
}

/// SIGNEDA | SIGNEDB: a term when it folds, else a C expression and its reads.
#[allow(clippy::result_large_err)]
fn signs(sa: Term, sb: Term) -> std::result::Result<Term, (String, Vec<Term>)> {
    match (sa, sb) {
        (Term::Lit(1), _) | (_, Term::Lit(1)) => Ok(Term::Lit(1)),
        (Term::Lit(_), t) | (t, Term::Lit(_)) => Ok(t),
        _ => Err((format!("{} | {}", sa.c(), sb.c()), vec![sa, sb])),
    }
}

/// An 18-bit operand from its bit terms, as an int expression.
fn gather(bits: &[Term]) -> String {
    let parts: Vec<String> = bits
        .iter()
        .enumerate()
        .filter(|(_, t)| !matches!(t, Term::Lit(0)))
        .map(|(i, t)| match (i, t) {
            (0, t) => t.c(),
            (i, Term::Lit(_)) => format!("1 << {i}"),
            (i, t) => format!("{} << {i}", t.c()),
        })
        .collect();
    if parts.is_empty() {
        "0".into()
    } else {
        format!("({})", parts.join(" | "))
    }
}
