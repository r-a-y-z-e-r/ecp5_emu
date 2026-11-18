# MULT18 operand forwarding through ROA/ROB

The October 23, 2026 milestone adds to the [registered product](DSP_OUTPUT.md) two 18-bit operand outputs of MULT18X18D. ROA reads A after the selected INPUTA, ROB reads B after INPUTB. When the input stage is bypassed, the corresponding output is combinational. PIPELINE and OUTPUT delay the product, but not these operands.

This prepares the ALU input path; the ALU itself is not executed yet. The output CE/RST and SIGNEDP remain constraints of the earlier profile. There is no additional persistent storage for ROA/ROB: the kernel reads the existing input registers or input signals.

The next milestone allowed [limited observation of these input stages with an active ALU54B](DSP_ALU_INPUTS.md). The requirement of an inactive ALU below applies to the earlier full view that includes the product.

## What the correspondence is based on

[sysDSP, figures 2.2–2.3](https://www.latticesemi.com/view_document?document_id=50469) shows the input registers and the separate operand paths to the ALU. [Diamond Libraries, MULT18X18D, pp. 588–592](https://www.latticesemi.com/view_document?document_id=53950) lists ROA17…0 and ROB17…0. Port descriptions by themselves do not replace a behavioral model.

In the local nextpnr, the function `check_alu` requires the following connection. It was independently cross-checked against the native Trellis port catalog and the fixed entries of MIB_DSP3/MIB_DSP7:

| ALU54 input | Source |
|---|---|
| A[17:0] | ROA[17:0] of the first MULT18 in the slice |
| A[35:18] | ROB[17:0] of the first MULT18 |
| B[17:0] | ROA[17:0] of the second MULT18 |
| B[35:18] | ROB[17:0] of the second MULT18 |

[The collector](../tools/collect_dsp_forwarding.py) checks all **78 slices, 156 MULT and 5 616 bit-level connections**. For each, the port direction, the coordinates, the wire name and the exact `.fixed_conn` entry are checked; the [report](inventory/LFE5U-85F/dsp_forwarding_evidence.json) stores the instances, the source lines of the patterns and the checksums. That the number of these wires equals the capacity of OR/FR does not mean shared storage: these are different paths.

```sh
python3 tools/collect_dsp_forwarding.py
python3 tools/collect_dsp_forwarding.py --output target/dsp-forwarding-repeat.json
cmp docs/inventory/LFE5U-85F/dsp_forwarding_evidence.json target/dsp-forwarding-repeat.json
```

The structure of these connections is confirmed by the database. The executable behavior of ROA/ROB after INPUTA/B is a contract of the current parallel profile, based on the architecture diagram. The capture moment, reset/CE/GSR and the divider inherit the explicit D05 assumptions from [DSP_REGISTERS.md](DSP_REGISTERS.md). Their match with silicon is still to be checked. SROA/SROB, CAS_MATCH_REG, C/ROC, the pre-adders and other widths are not derived from this correspondence.

## Kernel and planner changes

[MultiplierOutputs](../src/kernel/dsp.rs) now returns `roa[18]` and `rob[18]` along with `product[36]` and `signed`. Each bit has its own driver and goes through the ordinary signal resolution, the event log and change propagation. Z on a data input becomes X; known neighboring bits are preserved; an unknown sign does not turn the whole operand into X.

The parallel profile still needs SOURCEA=SOURCEB=0, SOURCEB_MODE=B_SHIFT, CIBOUT=ON and an inactive ALU. The OUTPUT requirements CE=1/RST=0/GSR disabled also apply when ROA/ROB is observed on a primitive with an active OUTPUT. Observing a single output does not lift the constraints of the whole executable profile.

[The planner](../src/compile/dsp.rs) accepts the internal native probes ROA0…17 and ROB0…17, including as the only observed outputs. A primitive gets one executor regardless of the number of probes. The validity checks apply to every request: a bypass of the check was fixed in which a registered SIGNEDP was accepted after an already requested P. Now it is also rejected after ROA/ROB.

When the input clock coincides with the clock of an ordinary FF connected to ROA, the FF receives the previous value of the input register. The new ROA appears after the shared capture. Coinciding INPUTA/PIPELINE/OUTPUT do not turn into one combinational pass. An initial high clock does not by itself create an edge.

## Checks

The [kernel tests](../tests/dsp.rs) are extended with a bit-by-bit comparison of ROA/ROB in all eight INPUTA/B/PIPELINE combinations: 257 samples for SYNC and ASYNC with independent clock/CE/reset/GSR; another 257 samples per combination with an active OUTPUT. Undefined initial input registers are also checked, not skipped. The checks of unknown CE/reset/GSR and of possible edges are extended to INPUTA/ROA.

Two separate tests check mixed 0/1/X/Z in all bits and the transfer of the old ROA into an FF on a shared edge with INPUTA, PIPELINE and OUTPUT enabled. The planner checks cover the sole probes of both operands and the refusal for a registered SIGNEDP regardless of the previously requested output.

Both earlier native test benches were extended: the [input stages](../tests/dsp_register_native.rs) and the [output bank](../tests/dsp_output_native.rs). HDL, placement, GPIO and configuration settings are unchanged; internal probes of all 144 bits of ROA/ROB of the four MULTs were added. These are observations of the software model on a configuration built by the tools, not a measurement of these wires on a board.

Nine runs of 513 samples each were performed: for SYNC, ASYNC and OUTPUT separately, `.config` observed, `.bit` observed and `.bit` all-plc. **664 848 bit-level operand checks** were compared with an independent numeric accounting of the input stages. The traces, including X, matched between the loading methods. The earlier checks of the products, the shared bank and the observing FFs also passed. In the two CLI runs of the input stages, 16 probes of the extreme bits of ROA/ROB were added; 28 probes and 514 observations each in total. The CLI of the output bench keeps the earlier 16 probes of the products/FFs.

```sh
cargo test --locked
cargo test --release --locked --test dsp_register_native --test dsp_output_native -- --ignored --test-threads=1
cargo test --release --locked -- --ignored --skip real_bitstream_matches_text_config --test-threads=1
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Current check: **120 tests**, 106 ordinary and 14 native integration. Clippy and formatting passed. The external ECP5_TEST_BIT was not run. The [compressed native traces](inventory/LFE5U-85F/dsp_forwarding_native.json) store four pairs of 18-bit ROA/ROB per sample, including X; the [final report](inventory/LFE5U-85F/dsp_forwarding_validation.json) holds the checksums and the scope of the check. The original extended traces remain in `target/dsp-registers-{sync,async}/` and `target/dsp-output/`. The compression and the CLI comparison are reproduced by the [results collector](../tools/summarize_dsp_forwarding.py).

## Further work

The input path advanced D07. All D01–D10 remain open; hardware results are still zero, and the list of 78 research topics and 37 future series is not reduced.

Next, the OP[10:0] operation table and the selected ALU sources need to be established, then its transitions and the R/flag views in the shared bank. The public materials read in this pass give the ALU54B interface and a constant example of OP, but not an unambiguous full table of the arithmetic. The sysDSP example additionally involves ALU54A. Therefore a specific opcode is not yet declared to be addition by guesswork. The output CE/RST/SIGNEDP also keep their own open questions and future distinguishing experiments on the board.
