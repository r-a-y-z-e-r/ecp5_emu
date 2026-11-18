# Input and intermediate registers of MULT18

The October 23, 2026 milestone extends the [first DSP execution](DSP_EXECUTION.md): the parallel MULT18X18D accepts the optional INPUTA, INPUTB and PIPELINE. All eight bypass combinations of these stages are executed by one kernel. The output stays in CIBOUT_BYP=ON mode; the shared OR/FR is not assigned and stays X. This is a software model of a limited profile with explicit D05 assumptions, not board-confirmed semantics of all DSP registers.

The next milestone is the [first registered MULT18 output in the shared bank](DSP_OUTPUT.md). Below are kept the boundaries of the input/pipeline stages and the results of their own test bench.

## Storage and capture order

In [DeviceState](../src/state.rs), each of the 156 MULT18 has a logical state: 18 bits of A and its sign, 18 bits of B and its sign, 36 bits of the product, and SIGNEDP. Values start as X except the active input/pipeline data stages under configured startup with GSR enabled and CLK_DIV=DISABLED: HD09ps showed zero. Unused stages stay X. `DeviceState::multiplier` exposes these values for observation; `dsp_slice` keeps the earlier single shared 72-bit bank of the slice.

These 75 bits per MULT18 are **logical views of the selected profile**, not a full list of physical FFs and not evidence of a separate physical owner for each control bit. C, the pre-adders and matching/framing are not modeled by this description. The physical binding and possible sharing with other modes remain D01/D05.

In the [DSP kernel](../src/kernel/dsp.rs) and the [shared capture phase](../src/kernel/mod.rs), the input fabric settles first. All internal DSP stages, ordinary FFs, EBR and LUTRAM then read the previous state before any result is published. Coinciding edges of INPUTA/B and PIPELINE do not pass data through both stages in one cycle. The next ordinary FF on the same edge receives the previous DSP result. When the kernel is built, the current clock levels are set without an invented initial edge.

A/B and the signs normalize Z to X. Unknown CE, reset and possible edges merge the alternatives bit by bit, keeping only provable levels. Changes of internal states are counted by separate counters; visible outputs go through the ordinary routing and the transition log.

## Configuration and the limit of the information

[The planner](../src/compile/dsp.rs) binds the selected REG_INPUTA/B/PIPELINE_CLK to CLK0–3, and the corresponding CE/RST to CE0–3 and RST0–3. CLK/CE/RSTMUX are taken into account, including INV, RESETMODE=SYNC/ASYNC and GSR=ENABLED/DISABLED. The shared controls of the block are taken from the composite fields and the real native ports. In the report, `multipliers[].stages` keeps the selected control sources, and `resources[].fields` keeps the raw statuses and candidates.

Some standard codes are empty: CLKxMUX=CLKx, CLKx_DIV=ENABLED, GSR=ENABLED, RESETMODE=SYNC. Their `unobserved` status is resolved only when there is a single expected candidate and a non-empty set of bits read. This is a limited qualification of the profile; a missing field, an incomplete result and an ambiguous active clock are rejected. Standalone INPUTC, PRADD, shift/highspeed and small modes remain unsupported. ALU C/input registers and the output bank are described in DSP_ALU.md and DSP_OUTPUT.md. HD09 adds the disabled divider profile below.

The [sysDSP](https://www.latticesemi.com/view_document?document_id=50469) guide, pp. 7–8 and 30–31, confirms the optional input/intermediate registers and gives an example of a pipelined multiplier with zeroing. [Diamond Libraries](https://www.latticesemi.com/view_document?document_id=53950), pp. 588–592, lists separate CLK/CE/RST and SYNC/ASYNC for MULT18X18D. The Yosys blackbox declarations and the nextpnr writes confirm the interfaces and the selected fields, but do not replace a behavioral reference.

Therefore `multipliers[].behavioural_assumptions` explicitly keeps three D05 statements:

- The input sign uses the control of its own operand; SIGNEDP passes through PIPELINE together with the product.
- Reset zeroes the data and the signs and has priority over CE; the effective GSR asynchronously clears the enabled stages.
- With CLK_DIV=ENABLED, INPUTA/B, PIPELINE and OUTPUT capture on the selected rising edge.
- With CLK_DIV=DISABLED, those stages capture on both rising and falling edges (HD09). ALU/MULT C storage instead captures every rising edge; with ENABLED it retains the HD08 every-other-rising-edge divider. CE and local reset keep their existing priority and reset kind. An enabled GSR clears at once; if active at a rising edge it holds through the following falling edge, until the next rising edge.
- Mixed ENABLED/DISABLED active register clocks or cells, and ALU/MULT divider disagreements in one slice, are refused as unmeasured. The event kernel and fast mode both run this profile. Fast captures declare both edges for non-C stages and keep C rising-only without a divider.

The earlier undecodable MULT_BYPASS=DISABLED, CAS_MATCH_REG=FALSE and HIGHSPEED_CLK=NONE also remain explicit assumptions of the standard nextpnr flow. An arbitrary Diamond bitstream is not certified by them. D05 remains open.

## Checks

The fast/kernel random trace tests in `tests/dsp.rs` cover ENABLED and DISABLED,
all 15 nonempty INPUTA/B/PIPELINE/OUTPUT combinations, both reset kinds,
independent clocks, CE/RST/GSR traffic, and kernel-to-fast handover. DISABLED also
checks a rising-edge slice FF sharing a DSP clock. The forced run uses
`FAST_THREADS=4 FAST_THREAD_MIN=1` and asserts that a DSP clock-group settle is
split across workers. The HD09 routed pin and free UART checks are in
`tests/hd09_hs02_native.rs`.

Fast clock groups OR the requested edge polarities across all members. Adding a
DISABLED DSP capture therefore also settles slice FFs sharing its clock after
falling edges. This extra settle is idempotent and is paid only by DISABLED
designs. ENABLED slot allocation and emitted C remain unchanged; 37 existing DSP
test programs (7,384,356 bytes) were compared byte-for-byte before and after.


The [portable tests](../tests/dsp.rs) compare all eight stage combinations for SYNC and ASYNC at 257 samples each, with independent clocks, CE/RST and a dynamic sign. The reference uses separate numeric states and i128 arithmetic. Also checked are coinciding DSP and FF edges, the absence of an initial edge, X/Z on the controls and data, reset with CE=0, GSR, and the inversions of all three types of control inputs. The planner is checked for each stage combination and for unsupported modes.

[The native test bench](../tests/dsp_register_native.rs) is built through Yosys → nextpnr → ecppack. Four MULT18 in one physical DSP block have the profiles INPUTA, INPUTB, PIPELINE and INPUTA+INPUTB+PIPELINE. **196 GPIOs** carry A/B, the four 36-bit products, three independent sets of CLK/CE/RST, the dynamic signs, a user GSR and four ordinary FFs. In the first slice GSR is enabled, in the second it is disabled. SIGNEDP is observed on an internal native port.

For SYNC and ASYNC separately, `.config` observed, `.bit` observed and `.bit` all-plc were run: **six runs of 513 samples each**. After the unknown stages fill, each product has 508–510 fully known samples with a bit-by-bit comparison; the early unknown states are preserved and compared between the loading methods. SIGNEDP and the known FF captures were checked. In all-plc, all 83 640 FFs, 83 636 LUTs and 716 254 routes are present. All 78 shared output banks remain X.

The CLI separately repeats both modes: 514 observations each with the initial state, 12 probes, including the high and low bits of the products and the four FFs. All working observations match the native run. These checks establish the consistency of the decoder and our model, not hardware equivalence.

```sh
cargo test --locked
cargo test --release --locked --test dsp_register_native -- --ignored
cargo test --release --locked -- --ignored --skip real_bitstream_matches_text_config --test-threads=1
```

In the stored native results, three identical traces are represented by one base trace with four-valued product strings and the checksums of all full source samples. Unknown bits are preserved.

Stored are the [results and checksums](inventory/LFE5U-85F/dsp_register_validation.json), as well as the HDL, LPF, events, CLI and native results for [SYNC](inventory/LFE5U-85F/dsp_register_sync_native.json) and [ASYNC](inventory/LFE5U-85F/dsp_register_async_native.json).

Test bench sources: [SYNC HDL](inventory/LFE5U-85F/dsp_register_sync_fixture.v), [ASYNC HDL](inventory/LFE5U-85F/dsp_register_async_fixture.v), [LPF](inventory/LFE5U-85F/dsp_register_fixture.lpf). The shared [events](inventory/LFE5U-85F/dsp_register_events.json) and the CLI results for [SYNC](inventory/LFE5U-85F/dsp_register_sync_cli.json) / [ASYNC](inventory/LFE5U-85F/dsp_register_async_cli.json) are also stored. The GPIO layout is intended for software checking of routes; a board will need its own connection scheme.

In total, **111 software checks** passed: 98 ordinary and 13 explicitly run native integration checks. The earlier DSP checks without registers, and the EBR, LUTRAM, global controls, carry counters, CRAM/INIT and full architecture graph checks, passed again. Clippy passed with no warnings, and formatting passed. The external blink with ECP5_TEST_BIT was not run separately.

## Board check and the next step

D05 needs separate observations with stable inputs before the edge: change the sign while the negative operand is unchanged and CE is disabled; toggle CE separately for the inputs and PIPELINE; hold CE=0 during a local SYNC/ASYNC reset; apply GSR separately with stopped clocks in slices with GSR enabled and disabled. Shared clocking should show one delay stage each; the standard CLK_DIV should show operation relative to the external edges. The exact coincidences of data/reset switching with the clock are checked in a separate series and are not considered established by these digital tests.

There are no hardware results or Diamond models yet. The next stage is the correspondence of the shared output bank to the MULT/ALU modes (D01) and the registered output; then the ALU, the pre-adders and the cascades. INPUTC and the commonality of the physical internal registers remain in the [DSP queue](DSP_INTERNALS.md).
