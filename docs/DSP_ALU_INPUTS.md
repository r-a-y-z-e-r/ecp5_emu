# Observation of the input stages in a slice with an active ALU54

**Since 2026-11-12** the ALU54B itself is executed, for the profile measured on the board
([DSP_ALU.md](DSP_ALU.md)). The operand-only view described here remains for an active ALU
outside that profile (its C or opcode registers on, output registers on two clocks, and so on).
The statements below that the ALU is not executed describe the October milestone.

The October 23, 2026 milestone continues [operand forwarding through ROA/ROB](DSP_FORWARDING.md). These outputs can now be executed and observed from a real `.bit/.config` that contains an active ALU54B. The ALU arithmetic, its flags and its output registers are not implemented by this milestone.

The earlier planner rejected any MULT in a slice with an active ALU. Now, when ROA/ROB is requested, it creates a limited `operands_only` view: it applies INPUTA/B and their CLK/CE/RST/GSR, and keeps the other stages as source configuration information. This makes it possible to check operand delivery before the ALU operations are connected.

## Execution boundary

| Part | Behavior |
|---|---|
| Mode selection | Exact MULT18X18D and an active ALU54B of the same slice |
| Sources | Parallel A/B, SOURCEA=SOURCEB=0, SOURCEB_MODE=B_SHIFT; the existing pre-adder route check is kept |
| INPUTA/B | Bypass or an unambiguous CLK0–3; the selected CE/RST and MUX/INV, SYNC/ASYNC, the earlier D05 contract |
| ROA/ROB | Two 18-bit outputs from the already existing input stages |
| INPUTC/PIPELINE/OUTPUT | Not applied in this view; their source fields are kept in the report |
| P/SIGNEDP | No drivers are created; requests are rejected even after an earlier requested ROA/ROB |
| ALU R/CO/flags | Not computed; a request for an internal output or for a GPIO connected to it is rejected |
| Shared bank OR/FR | One bank per slice is kept; this view does not write it |
| Run scope | `observed`; `all-plc` with an active ALU is still rejected |

The report of each MULT now contains `execution_view`: `operands_only` or the earlier `product_and_operands`. For the first, the candidates of only the two applied register settings are kept, and `cibout_bypass` has the value `not_used_operand_view`. The original decode of CIBOUT_BYP and of all other fields remains available. This value does not declare the real hardware bypass to be on or off.

The structural ROA/ROB→ALU connections are confirmed by the [previous collection](inventory/LFE5U-85F/dsp_forwarding_evidence.json). The behavior of the input registers still uses the explicitly described [D05 assumptions](DSP_REGISTERS.md). The parameters CAS_MATCH_REG/HIGHSPEED and other unconfirmed modes get no new implementation.

## Kernel changes

[Builder::multiplier18_operands](../src/kernel/dsp.rs) returns only `MultiplierOperandOutputs`. An active product/output stage cannot be passed to it. The internal executor has separate operand drivers and optional product drivers; multiplication is not run in the operand view. A repeated binding of the same MULT, including an attempt to replace it with the full view, is rejected: there is only one owner of the input state.

[The planner](../src/compile/dsp.rs) selects this view only for a known active ALU54B and a ROA/ROB request. The mode check runs before an already bound resource is reused. The raw values of the stages that belong to the ALU/product do not become settings of the input registers and are not replaced by defaults.

## Checks on the configuration

[The native test bench](../tests/dsp_alu_operands_native.rs) places four MULT and two ALU in one DSP block. INPUTA/B have four bypass combinations: 0/0, 1/0, 0/1 and 1/1. In the left ALU, OUTPUT0 and FLAG use CLK2, OUTPUT1 uses CLK3; the right one bypasses the output stages. 117 GPIOs carry A/B, C[53:0], OP[10:0], the dynamic signs, four clocks, two CE/reset and six real R/flag outputs. The execution plan contains 111 input PIOs, and the ALU output PIOs are not observed.

The unused dedicated CIN/CFB/SIGNEDCIN are not connected in this test bench. An attempt to tie them to an ordinary constant net did not pass nextpnr routing; the constraints of the dedicated chains are preserved. This test bench checks the observation of the input stages, not the correctness of any ALU operation. The R/flag outputs keep the ALU in the synthesized design, but are not used as an arithmetic reference.

SYNC and ASYNC were checked separately. For each, `.config` and `.bit` observed were run at 257 samples each: **148 032 bit-level operand checks** in total. The comparison is made against an independent numeric accounting of the input stages; the initial X are checked too. The traces matched between the loading methods. The dynamic C, OP and the clocks of the ALU output stages did not affect ROA/ROB. The intermediate products and all shared output banks stayed X.

In the two CLI runs, 16 internal probes and 258 observations each were compared. Refusals for P, SIGNEDP, an internal R, a GPIO with R, and `all-plc` were checked separately. The [kernel tests](../tests/dsp.rs) check the absence of product drivers, the prohibition of product/output stages, a single owner, and INPUTA operation without a write to the shared bank. The planner test accepts ROA/ROB with deliberately unqualified fields that are not applied, and keeps the refusals for unsupported outputs.

```sh
cargo test --locked
cargo test --release --locked --test dsp_alu_operands_native -- --ignored --test-threads=1
cargo test --release --locked -- --ignored --skip real_bitstream_matches_text_config --test-threads=1
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

**122 unique tests** passed: 107 ordinary and 15 native integration. Clippy and formatting passed. The external ECP5_TEST_BIT was not run. The earlier MULT, output bank, EBR, LUTRAM, global/wake and full directed graph were checked again.

The [compressed traces](inventory/LFE5U-85F/dsp_alu_inputs_native.json), the [HDL SYNC](inventory/LFE5U-85F/dsp_alu_inputs_sync_fixture.v), the [HDL ASYNC](inventory/LFE5U-85F/dsp_alu_inputs_async_fixture.v), the [LPF](inventory/LFE5U-85F/dsp_alu_inputs_fixture.lpf) and the [validation with checksums](inventory/LFE5U-85F/dsp_alu_inputs_validation.json) are stored in the inventory. The [results collector](../tools/summarize_dsp_alu_inputs.py) reproduces the compression and the CLI comparison. The full source files remain in `target/dsp-alu-operands-{sync,async}/`.

## What arithmetic needs

The full OP[10:0] table remains D04. The public sysDSP guide, the library reference guide and the open-source code were re-checked. The [example of a third-party model](https://github.com/CharlesCNorton/nosis/blob/master/nosis/postsynth.py) has a different interface, a four-bit OP, and does not use it in the computation; therefore the behavior of our chip is not derived from it. The vendor declaration found in another open project also turned out to be blackbox. The constant example from sysDSP with ALU54A does not establish all ALU54B operations.

The next distinguishing evidence is the exact vendor model, or measurements of the codes and of the source selection on the available **ULX3S** board, identified in the [hardware check plan](HARDWARE_VALIDATION.md). The revision and the actual connection scheme still need to be specified; CABGA381 in the software examples does not by itself establish the package of the actual unit on hand. Until such evidence exists, the ALU gets no guessed addition operation. The research counters keep **78 open topics, 37 prepared series and zero hardware results**; D01–D10 are not closed.
