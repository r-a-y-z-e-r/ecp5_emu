# Shared DSP output bank: the first MULT18 profile

The October 23, 2026 milestone adds the registered MULT18X18D product to the [input and intermediate stages](DSP_REGISTERS.md). The two MULTs of one slice write different 36-bit views of **one shared 72-bit bank**. This is a partial resolution of D01 for the ordinary MULT18; the ALU, the flags, framing and the physical FF numbering have not yet been mapped.

The next milestone added [operand forwarding through ROA/ROB](DSP_FORWARDING.md) and re-checked this output profile. The original result of the shared-bank milestone is kept below.

## Executable profile

| Setting | Supported boundary |
|---|---|
| MODE / ALU | MULT18X18D; the ALU54 of the same slice is inactive |
| Inputs | Parallel A/B, SOURCEA=SOURCEB=0, SOURCEB_MODE=B_SHIFT |
| INPUTA/B and PIPELINE | Any of the eight bypass combinations; separately selected clocks |
| Output to CIB | CIBOUT=ON, qualified CIBOUT_BYP=OFF |
| Output clock | An unambiguous CLK0–CLK3; CLK_DIV=ENABLED (rising) or DISABLED (both edges, HD09) |
| CE / RST | CE0, and the reset `REG_OUTPUT_RST` selects on MULT18 0/4 (RST0 on 1/5), synchronous or asynchronous by RESETMODE ([HD01](inventory/LFE5U-85F/hd01_mult_output/README.md), board, 2026-11-11; RESETMODE=ASYNC measured in [HD07](inventory/LFE5U-85F/hd07_dsp_gsr/README.md), 2026-11-15) |
| GSR | ENABLED or DISABLED. Enabled, GSR clears the output register at once and holds it at 0 while active; disabled, it does nothing ([HD07](inventory/LFE5U-85F/hd07_dsp_gsr/README.md)) |
| Start | 0 under the `configured` startup where the slice has GSR enabled (HD07s); X until the first capture where it is disabled (not measured) |
| Product | Full 36 bits, signed/unsigned, and conservative X/Z |
| SIGNEDP after OUTPUT | Not modeled; X in the API, a request for this output is rejected at planning |

**Update 2026-11-11 ([HD01](inventory/LFE5U-85F/hd01_mult_output/README.md)):** on the board the output register captures under CE0 whatever `REG_OUTPUT_CE` asks for. Its reset is the `REG_OUTPUT_RST` selection on MULT18 0/4 and RST0 on 1/5. The emulator now executes that; the paragraph below describes the earlier, restricted profile.

**Update 2026-11-15 ([HD07](inventory/LFE5U-85F/hd07_dsp_gsr/README.md)):** with the slice's GSR enabled, GSR clears the output register asynchronously, and `RESETMODE=ASYNC` makes its reset asynchronous. Both are executed in the event kernel and in fast mode. Under the `configured` startup the register starts at 0 where GSR is enabled, as the board showed at the first clock.

The REG_OUTPUT_CE setting is absent on all 156 MULTs, and REG_OUTPUT_RST exists only on 78 instances, with native indices 0/4. The nextpnr writer likewise does not write REG_OUTPUT_CE and writes REG_OUTPUT_RST only on 0/4. These settings do not allow arbitrary control of the output stage to be reconstructed. Therefore the profile requires **all** CE enabled and **all** local resets inactive, does not pick a convenient CE0/RST0, and does not claim that the output is always enabled in hardware. This condition is checked at build time and after the inputs settle; 0/X/Z on CE or 1/X/Z on RST stop execution before the capture. GSR cannot be enabled in this profile yet.

The INPUTA/B/PIPELINE registers use the earlier model and the explicit D05 assumptions. When OUTPUT is active, the common CE/RST constraints apply to the whole primitive. The initial values of the output bank are X; before the first edge there is no invented clearing or capture. The parallel CLK_DIV=ENABLED profile uses the selected rising edge. HD09 adds CLK_DIV=DISABLED: INPUTA/B, PIPELINE, OUTPUT and ALU R/opcode stages update on both edges, with existing CE/reset priority. C instead captures every rise. GSR active at a rise holds through the next fall; mixed divider profiles and ALU/MULT disagreements are refused. Fast mode runs this profile with both-edge captures and saved GSR-at-rise state. Its clock groups OR edge requests: slice FFs sharing the clock also receive an idempotent falling-edge settle, paid only by DISABLED designs. Random kernel/fast traces cover every stage combination with both reset kinds and CE/RST/GSR traffic, under default and forced four-thread settings. Highspeed settings remain unsupported. Coincidences of data/reset/clock on the physical chip still need a separate check.

## Single owner and logical coordinates

[The sysDSP guide](https://www.latticesemi.com/view_document?document_id=50469), p. 8, figure 2.3, shows the shared OR/FR bank and four 18-bit groups for the two 36-bit products in a slice. From this the capacity and the two views of the products can be determined. It does not give the numbering of specific physical FFs or the placement of the ALU/flags inside the bank.

[DeviceState](../src/state.rs) defines logical coordinates in the order of the native MULTs along the column:

| Logical bank bits | View in an ordinary MULT18 |
|---|---|
| 0–17 | P[17:0] of the first MULT |
| 18–35 | P[35:18] of the first MULT |
| 36–53 | P[17:0] of the second MULT |
| 54–71 | P[35:18] of the second MULT |

This is an API convention for the indices of the shared array, **not an established physical layout of the ALU or the CRAM**. The pair 0/1 and the pair 4/5 have separate banks. `multiplier_output_view` returns the owner and the range, `multiplier_output` reads it; there is no separate persistent copy of the output product in `MultiplierState`. The MULT, ALU and DSP_SLICE ports address the same bank owner; the ALU view has not been assigned yet.

[The kernel](../src/kernel/dsp.rs) computes the captures of all input, intermediate and output DSP stages and of the ordinary FFs from the previous state. Then the write touches only the 36 bits of its own product. Simultaneous captures of the two MULTs do not overwrite the neighboring half; when the INPUT/PIPELINE/OUTPUT/FF clocks coincide, the data advances one stage per edge. Changes of the shared bank are counted in `dsp_output_register_bit_changes`; the observable outputs go through the ordinary event log.

## Overlap of CIBOUT_BYP and the output clock

[A check of all 156 instances](inventory/LFE5U-85F/dsp_output_evidence.json) established an exact equality: the OFF bit pattern of the CIBOUT_BYP field contains **the whole bit pattern of REG_OUTPUT_CLK=CLK0**, 8 or 12 bits. ON has two separate positive bits that do not belong to the clock selection. The original fuzzer `076-dsp_cibo` compared the output bypass at NONE with the registered output at CLK0. So when CLK1/2/3 is selected, the raw field can be `unmatched`, even though the output stage is specified unambiguously.

[The new decoder](../src/decode/dsp.rs) keeps the raw field and adds `dsp_output_path`:

- Recovery from `unmatched` is allowed only with complete ON/OFF and NONE/CLK0–3 domains, an exact OFF=CLK0 equality and two separate bypass bits.
- If both bypass bits are set, the path is bypass. If both are cleared and the output clock is fully resolved, the path is registered.
- A mixed pair of bits is rejected, even when the raw OFF happens to match. An ambiguous/incomplete clock and unknown additional constraints are not masked.
- All bits that changed relative to CLK0 stay in `clock_overrides`. This is not a global loosening of decoding or a replacement with defaults.

In the native test bench, CLK0 keeps the raw OFF, and CLK1/2/3 have `unmatched` with 2/2/4 changed bits. All three variants are qualified correctly, with the evidence preserved. The overlapping fixed descriptions of MULT9 and of the inactive ALU are excluded only in the context of the selected MULT18 profile; the raw routes stay in the report. This does not make the ALU executable.

The encodings and the shared fields are reproduced by [the collector](../tools/collect_dsp_output.py):

```sh
python3 tools/collect_dsp_output.py
python3 tools/collect_dsp_output.py --output target/research/dsp-output-repeat.json
cmp docs/inventory/LFE5U-85F/dsp_output_evidence.json target/research/dsp-output-repeat.json
```

## Checks and stored results

Three [decoder tests](../src/decode/dsp.rs) check all output clocks, the split of fields across three/four tiles, mixed bypass bits, an inactive MODE, incomplete domains and extraneous constraints. Three new [kernel tests](../tests/dsp.rs) check shared ownership of the bank, independent/coinciding clocks, all eight INPUTA/B/PIPELINE combinations with OUTPUT at 257 samples each, preservation of the neighboring product, and a refusal for each unsupported CE/RST/GSR. The extended planner test also checks the output clock with INV, the control constraints and the refusal for SIGNEDP.

[The native test bench](../tests/dsp_output_native.rs) places four MULTs in one physical DSP block. Profiles: OUTPUT; INPUTA+B+OUTPUT; PIPELINE+OUTPUT; INPUTA+B+PIPELINE+OUTPUT. The four output clocks use CLK0–3. **198 GPIOs** carry A/B, the dynamic signs, all CE/RST, the four 36-bit results and four ordinary FFs that capture the old product on the same edge. The LSR of the observing FFs is connected to an explicitly driven rst0=0.

Three runs of 513 samples each were performed: `.config` observed, `.bit` observed, `.bit` all-plc. The numeric i128 reference is independent of the kernel and accounts for the old values of each stage. After the pipelines fill, 512/504/498/506 samples of the four products are fully known; their 144 bits and the known FF captures were compared. All states, including the initial X, matched between the loads. In all-plc, all 83 640 FFs and 83 636 LUTs execute through 716 339 routes.

The state holds the same 78 banks/5616 bits; only 144 bits of the two slices used are known, and the rest remain X. A check of all 156 views confirms that they cover the 78 logical banks without overlaps or gaps. This is a check of the structure of the software model, not a measurement of the physical layout.

The CLI repeats the 513 working samples and the initial state on 16 probes. [The events](inventory/LFE5U-85F/dsp_output_events.json) include time=0 with CE=1/RST=0: without explicitly set initial levels the environment would leave them Z and would correctly refuse to run. After the successful run, a separate check lowers CE0 and confirms the runtime refusal.

```sh
cargo test --locked
cargo test --release --locked --test dsp_output_native -- --ignored
cargo test --release --locked -- --ignored --skip real_bitstream_matches_text_config --test-threads=1
```

Stored are the [native results](inventory/LFE5U-85F/dsp_output_native.json), the [CLI](inventory/LFE5U-85F/dsp_output_cli.json), the [HDL](inventory/LFE5U-85F/dsp_output_fixture.v), the [LPF](inventory/LFE5U-85F/dsp_output_fixture.lpf) and the [final validation with checksums](inventory/LFE5U-85F/dsp_output_validation.json). The repeated native traces are stored as one base sample with four-valued product strings and the checksums of the full source samples.

In total, **118 software checks** passed: 104 ordinary and 14 explicitly run native integration checks. The new checks are three decoder checks, three kernel checks and one native test bench. The earlier DSP, EBR, LUTRAM, global/wake, carry counter, CRAM/INIT and full-graph checks passed again. Clippy passed with no warnings, and formatting passed. The external ECP5_TEST_BIT was not run. The [raw and qualified decode of the four outputs](inventory/LFE5U-85F/dsp_output_decode.json) is also stored.

## What remains

D01 has partially advanced for MULT18 products. The physical mapping of the ALU R, the flags and framing to the shared bank, the registered SIGNEDP, the output CE/RST, the small modes and the cascades remain open under D01/D03/D05/D06. The current software checks and the nextpnr writer do not replace a hardware reference. This milestone has no board checks or Diamond models.

The next stage is to separate the remaining questions of OUTPUT control and SIGNEDP propagation, then map the ALU/flags onto the same bank and start the selected ALU operations. Distinguishing board experiments must check CE, reset/GSR and the capture of the sign separately, with stopped/separate clocks. The GUI keeps its place after the kernel in the [overall plan](IMPLEMENTATION_PLAN.md).
