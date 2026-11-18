# First DSP execution: parallel MULT18

The October 23, 2026 milestone: multiplication by MULT18X18D without internal registers was added to the event kernel and to the plan built from CRAM. `.config` and `.bit` take the same path. This is the first executable DSP profile; it does not close all modes of the family or the D01–D10 questions from the [collected material](DSP_INTERNALS.md).

The next milestone is the [input and intermediate registers of MULT18](DSP_REGISTERS.md). Below are kept the original profile without registers and its results; the current extended model and the D05 assumptions are described separately.

Later, the [registered MULT18 output in the shared bank](DSP_OUTPUT.md) was added, with separate control constraints.

## Fast mode

Since 2026-11-11 [fast mode](FAST_MODE.md) runs this profile: MULT18X18D with SOURCEA/B at 0.
- **Without registers.** This is what yosys writes for every `*`: all REG_*_CLK=NONE,
  CIBOUT_BYP=ON, GSR=ENABLED unused.
- **With register stages.** Any of the [INPUTA/INPUTB/PIPELINE](DSP_REGISTERS.md) and
  [OUTPUT](DSP_OUTPUT.md) stages, each register bit a capture with the kernel's D05 rules:
  - the reset comes before CE;
  - GSR clears the INPUT/PIPELINE stages;
  - SIGNEDP passes through PIPELINE.
- **OUTPUT stage.** It needs CE0..3 tied to 1 and RST0..3 tied to 0. The kernel checks driven
  ones as the design runs; fast mode refuses them.
- **Product.** Each product bit is a line of the generated C over the operand slots
  ([kernel/fast_dsp.rs](../src/kernel/fast_dsp.rs)). It uses the arithmetic of
  `multiply18`: each operand is two's complement when its sign control is 1, and the result is
  limited to 36 bits.
- **Other outputs.** SIGNEDP is SIGNEDA | SIGNEDB. ROA/ROB are the A/B inputs.
- **GSR and resets at edges.** A user GSR from logic resets the stages that take GSR. A reset
  at a clock edge acts as in the kernel: the other stages capture the value from before it.
- **Unknowns.** An unknown operand that the logic reads stops fast mode, as for any gate; the
  event kernel keeps the conservative X arithmetic.

Checks are listed in FAST_MODE.md under *Checks*. The [mac](../examples/ulx3s/mac/top.v)
reference design (seven multipliers, a signed 16×16 dot product and an unsigned 32×32 product)
prints in both executors the line the arithmetic of its Verilog gives.

The ULX3S printed the same line in 3 of 3 loads
([HR02](inventory/LFE5U-85F/hr02_ram32_mac/README.md), 2026-11-11). This is the first board
result for MULT18X18D. It covers this profile only: no registers, static per-multiplier sign
controls. D01–D10 stay open.

## Storage and profile boundaries

Each of the **78 DSP slices** has one reserved 72-bit OR/FR output storage. Both MULT18, the ALU54 and the structural DSP_SLICE access one owner; there are no independent copies for the alternative primitives. All 5616 bits remain X: the correspondence of these bits to the outputs, flags and framing of the different modes has not been established yet, D01. The input and intermediate DSP registers are not described by this reservation; it is not a full count of all FFs inside the DSP.

`DeviceState::dsp_slice` returns the owner and the shared bank. The combinational product is available on P[35:0], is not written to OR/FR, and is not passed off as the state of an output register. The next step can extend the same bank without creating a separate ALU storage. Implementation: [state.rs](../src/state.rs).

A specific profile is supported:

| Condition | Implemented / checked |
|---|---|
| MODE | Full composite MULT18X18D |
| Sources | Parallel A/B, SOURCEA=SOURCEB=0, SOURCEB_MODE=B_SHIFT |
| Input and pipeline stages | REG_INPUTA/B/C_CLK=NONE, REG_PIPELINE_CLK=NONE |
| Output | CIBOUT_BYP=ON on the MULT and CIBOUT=ON on the slice; the ALU54 of this slice is not active |
| Arithmetic | All four signed/unsigned combinations; full 36-bit result |
| SIGNEDP | The documented OR of the controls SIGNEDA/SIGNEDB, separate from the high bit of P |
| Unknowns | Z on data is treated as X; known results are kept when this is provable |

SOURCEA/B=1 or X/Z leads to a diagnostic at execution. A selected JPO→JMULTA route is rejected as an unsupported pre-adder. Internal registers, ALU/joint mode, C/dynamic/highspeed and the other ROA/ROB/ROC/SROA/SROB outputs are rejected for now. Support for 9×9, 36×36 and cascades is not claimed.

MULT_BYPASS, CAS_MATCH_REG and HIGHSPEED_CLK have no standalone full encoding in the current catalog. This profile **explicitly assumes the standard nextpnr settings**: DISABLED, FALSE and NONE respectively. This is recorded in `multipliers[].implicit_unmodelled_parameters` and in the constraints of the plan. It does not certify an arbitrary Diamond bitstream or an unknown small/joint mode from incomplete fields. Reconstructing these settings remains D03/D06.

Input signed values are interpreted as 18-bit two's complement. Known operands are computed with integers, without floating point; the result is limited to 36 bits. Unknown signed controls with known data merge the possible interpretations. For unknown data, a conservative four-valued shift/add computation is used: for example, X×0 gives 0, and an unknown high bit does not have to destroy all the lower bits. Accuracy with X is not equivalent to the measured metastability behavior on a board.

## Event order and route selection

All ordinary input routes settle, then the DSP computes the product, after which its output paths settle. An FF captures the fully computed result in the shared capture phase. If the inputs and the clock change in one batch of events, the result is the digital result after the inputs settle; setup/hold are not modeled. The initial computation does not create a fictitious clock edge.

In Trellis, one CIB output has alternative fixed views of MULT18, MULT9, JP/JR of ALU54/ALU24, and of the ALU flags. For the verified profile of an active MULT18 with an output bypass and an inactive ALU, the MULT18 path is selected. The excluded alternatives are kept in `excluded_routes` with a reason. Ownership of the generic JP/JR is established from the actual JR→native ALU54 output link, not by proximity to the nearest DSP. An unknown overlap still causes an error.

The output clock has identical empty encodings NONE/CLK3, or a single empty NONE. They are allowed only when ON of CIBOUT_BYP is confirmed: the combinational path is used, and no clock or value is assigned to the hidden output register. The original candidates stay in the report. This is not an implementation of the registered output.

In observed, the observation cone is executed. In all-plc, the configured MULT18 are additionally enabled even without an observed P, and since 2026-11-12 so is an active ALU54B of the measured profile ([DSP_ALU.md](DSP_ALU.md)); an ALU54 outside that profile or an unsupported stage causes an error. The name all-plc does not mean that all families of the chip are already executed.

Code: [arithmetic and the MULT18 event group](../src/kernel/dsp.rs), [qualification from CRAM](../src/compile/dsp.rs), [kernel](../src/kernel/mod.rs). The plan report has `multipliers`, the counter `counts.multiplier18`, and the reasons for route exclusion; the statistics count the groups of computations in `multiplier_evaluations`.

## Checks and run

[Five portable tests](../tests/dsp.rs) check the four sign modes at the boundaries and on 5000 pseudo-random vectors, X/Z and soundness of the known bits when 32 concrete instantiations of the unknowns are enumerated, shared bank ownership, and isolation of the two states. Also checked are the FF capture after computation, a sign change without a clock, the runtime refusal of SOURCEA, and the refusal of the planner for registered, C and ALU modes and for a disabled bypass/CIBOUT. Observed and all-plc without probes are distinguished separately.

[The native test bench](../tests/dsp_native.rs) places two MULT18 in one slice: the first with dynamic SIGNEDA/B, the second unsigned, plus an ordinary FF after P0. The inputs and the outputs of the products are routed through 113 package GPIOs. SIGNEDP is not brought out to the package: nextpnr did not find such a route; it is observed on a native internal port and checked against the documented OR.

| Image and scope | Samples | MULT18 / FF / routes |
|---|---:|---:|
| `.config`, observed | 513 | 2 / 1 / 2348 |
| `.bit`, observed | 513 | 2 / 1 / 2348 |
| `.bit`, all-plc | 513 | 2 / 83 640 / 713 269 |

In all runs, both 36-bit products, both SIGNEDP and the FF capture matched bit by bit; the corresponding `.config`/`.bit` and observed/all-plc matched. 98 alternative DSP routes were excluded with explicit reasons. OR/FR remains X in all 78 slices. The CLI separately repeats the 513 working samples and the initial state on seven probes.

```sh
cargo test --locked
cargo test --release --locked --test dsp_native -- --ignored

# After the native test: p0[0], p1[0] and captured in this test bench.
cargo run --release --locked -- emulate target/dsp-mult18/top.bit \
  --startup ready-unknown --scope observed \
  --stimulus target/dsp-mult18/events.json \
  --probe pin:C1 --probe pin:D8 --probe pin:G5 \
  --output target/dsp-mult18/user-run.json
```

Stored are the [native results](inventory/LFE5U-85F/dsp_execution_native.json), the [CLI](inventory/LFE5U-85F/dsp_execution_cli.json), the [HDL](inventory/LFE5U-85F/dsp_execution_fixture.v), the [LPF](inventory/LFE5U-85F/dsp_execution_fixture.lpf), the [stimuli](inventory/LFE5U-85F/dsp_execution_events.json) and the [milestone validation](inventory/LFE5U-85F/dsp_execution_validation.json).

**107 checks** passed: 95 ordinary and 12 explicitly run native integration checks. EBR, LUTRAM, global controls, carry counters, CRAM/INIT and the full architecture graph passed again. Clippy passed with no warnings, and formatting passed. The external blink with ECP5_TEST_BIT was not run separately.

Later, the input and pipeline stages of MULT18, with propagation of the signed controls and CE/reset, were added as a [logical profile with explicit D05 assumptions](DSP_REGISTERS.md). A registered shared output requires D01 to be resolved; the ALU/pre-adders, small modes and cascades keep their place in the queue of the [plan](IMPLEMENTATION_PLAN.md). There are no hardware results for this milestone.
