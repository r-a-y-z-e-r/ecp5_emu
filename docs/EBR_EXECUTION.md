# EBR execution from configuration

On 22 October 2026, the EBR was connected to the event model, the shared storages, and the system controls. `.config` and `.bit` use one plan built from composite CRAM fields. This is the next partial milestone of the full chip: the original milestone did not program the board. Subsequent HB01/HB02/HB09/HB09P/HB11 measurements qualify the subsets described below; other questions remain open.

## Supported scope

| Part | Implemented | Limit |
|---|---|---|
| Array | Each of the 208 resources has one 2048×9 storage; INIT/WID and the ninth bits are preserved | A missing/ambiguous INIT stays X |
| True dual port | Independent A/B widths 1, 2, 4, 9, 18; all 25 combinations use the same cells | An unambiguously decoded MODE is required |
| Pseudo dual port | 36-bit write through DIA+DIB, four masks ADA0…3; independent read B of width 1/2/4/9/18/36 | The 36-bit read (below) keeps an order and a control rule that HB01 checks on the board |
| Read | Synchronous main stage in NOREG; the additional OUTREG stage takes the old state of the main one | Initial read states are X without an explicit reset; INIT does not set the output |
| Write | NORMAL holds the output; WRITETHROUGH outputs DI; READBEFOREWRITE outputs the old word across widths 1, 2, 4, 9 and 18 ([HB11](inventory/LFE5U-85F/hb11_ebr_narrow_rbw/README.md)); same-clock DP collisions follow HB09 and the measured PDP36 profile follows HB09P below | Mixed widths/modes, partial masks, different clocks/phases, CE/OCE gating, resets/GSR during RBW and RBW in PDP are not measured |
| Controls | CE, three-bit CS, WE, OCE, reset, polarities of CLK/CE/OCE/WE/RST and of the low addresses | Unconfirmed combinations use the policy given below |
| System | GWDIS blocks writes; GSR resets the read states when GSR=ENABLED, preserving the array | The GSR model does not replace the hardware check B08 |
| Execution scope | observed executes the memory in the observation cone; all-plc additionally runs all configured EBR and RAMW | all-plc does not yet mean the behavior of all families of the chip |

Addressing follows the [collected contract](EBR_ADDRESSING.md): the low address pins are excluded according to the width, and in the 18/36-bit mode they set the masks of the nine-bit groups. Inversion is applied before addressing/masking. The stored CSDECODE word is reversed exactly once for the comparison with raw CS: nextpnr has already included the source CS inversions in it.

The states of the main stages and of the output registers are available through `DeviceState::ebr_pipeline`; the array through `ebr_word`. The ports have no copies of the array. FFs, LUTRAM, and both sides of the EBR first capture the shared inputs and the previous state, then publish the changes at the same time. An FF on the same edge gets the previous EBR output. A change of address between edges does not change the output, even in NOREG.

Implementation: [shared array and stage states](../src/state.rs), [EBR transitions](../src/kernel/ebr.rs), [plan from composite fields](../src/compile/ebr.rs), [common event order](../src/kernel/mod.rs).

## Start

Measured on 2026-11-16 ([HB02](inventory/LFE5U-85F/hb02_ebr_start/README.md)): after
configuration every read latch and output register of an EBR reads 0 until its port reads.
That holds for every width, both ports, NOREG and OUTREG, GSR enabled and disabled, and either
RESETMODE. Under the `configured` startup the planner starts them at 0
([compile/ebr.rs](../src/compile/ebr.rs), `Builder::clear_ebr_pipeline`). The `ready-unknown`
and `wake-phases` startups keep them unknown.

## Explicit uncertainty policy

The rules of the sources and the remaining questions are separated in [EBR_CONTROL.md](EBR_CONTROL.md).

[HB09](inventory/LFE5U-85F/hb09_ebr_collision/README.md), measured on 2026-11-17, resolves
DP16KD accesses when both ports have the same width and WRITEMODE, use the same clock and
polarity on one definite edge, and access full words. CE/CS must select both ports, OCE must
be 1, resets and GWDIS inactive, and addresses and WE known. In this scope:

- A port reading while the other writes the same word gets the old word for
  READBEFOREWRITE, or the new word for NORMAL/WRITETHROUGH. Both ports had the same mode in
  HB09, so whether the reader's or writer's mode decides remains unseparated.
- Each writing port outputs its own write-mode result, including when both write.
- Two writes to the same word store B's data at widths 1/2/4/9. At width 18 both output
  paths still follow their write modes, but the two array rows become X: the board sometimes
  mixed bits in the low nine-bit lane even with both masks enabled.
- Different equal-width words are independent, including narrow words sharing an INIT9
  row. HB09's clean reads and writes at widths 1/2/4 include these accesses and had zero
  mismatches. A write does not poison neighboring narrow words or the preserved ninth bit.
- NOREG captures the result on the edge; OUTREG transfers the previous read latch as usual.

[HB09P](inventory/LFE5U-85F/hb09p_pdp_collision/README.md), measured on 2026-11-17,
adds raw PDP36 same-word read/write collisions: the read returns the **new word**.
The qualifier is write/read width 36/36, NORMAL, NOREG on both paths,
ASYNC reset/release, GSR disabled; both ports must have a definite shared
clock edge and polarity, known addresses, active CE/matching CS, inactive
resets/GWDIS, OCE=0/1 and all four write lanes enabled. The pending A write
supplies each read bit, including all ninth bits. No array poisoning or
conservative collision notice is added for these accesses. Logical widths
32/36 and depths 64/128/256/512 share this physical profile. Narrow reads,
OUTREG and other control/mask/clock combinations remain outside the rule.
The board sampled explicit primitive outputs, without inferred forwarding.
The event replay matches all 17,408 bits of the configuration window.

The remaining decisions are a conservative policy of the emulator, not measured hardware rules:

- Unknown address bits enumerate the compatible addresses. A read merges the possible data; a write merges the previous/new value of each affected cell. Unknown CE/CS/WE/masks and a possible edge also keep the variants. Z on a data input becomes X.
- Outside the HB09/HB09P scopes, if one port writes an INIT9 row, a read of the other port in this row gets X. Two writes into one row make all nine of its bits X, including identical data and different narrow positions. This keeps the previous policy for different widths or modes, partial masks, unqualified PDP profiles, ambiguous controls, and different clocks or polarities. No additional collision window is invented between different event times; separated accesses retain their ordinary ordered behavior.
- In a partial write, WRITETHROUGH guarantees DI in the enabled groups, and in the others merges the held value, the previous memory, and DI. READBEFOREWRITE in the non-enabled groups merges the held value and the previous memory. The array changes strictly according to the masks.
- OUTREG with OCE=0 is held. With OCE=1 and a CE/CS that is not guaranteed, the hold and the transfer of the main stage are merged. For NOREG with OCE≠1, the hold and the main result are merged. The exact hardware action of OCE in these combinations awaits HB03/HB04.
- Synchronous reset zeroes the read states on the edge, including when CE/CS/OCE is off, under the synthesis contract EC02. Asynchronous reset with ASYNC release acts without an edge. Reset never clears the array. A simultaneous potential write/reset merges the hold and the array write; no hardware priority is assigned.
- With GWDIS=1, the array is held. The output in a WE cycle merges the read variant with the result of the selected write mode: the exact switching of the output path when writes are inhibited is not measured.

`RESETMODE=ASYNC, ASYNC_RESET_RELEASE=SYNC` is allowed only with a known inactive local reset. Its activation or X/Z leads to a diagnostic: the number of internal release stages is not established. SYNC reset is not restricted by this parameter. Outputs outside the selected width are rejected at planning.

## 36-bit PDP read

Added on 2026-11-11. the recorded default Yosys 0.70 maps widths 32/36 at depths 64/128/256/512 to a
DP16KD with `DATA_WIDTH_A=DATA_WIDTH_B=36`, which nextpnr writes as `MODE=PDPW16KD`,
`DATA_WIDTH_R=36`. A `reg [31:0] mem[0:511]` is the common case. Until then both executors
refused such a design.

- **Two read paths, one read.** The block has two physical 18-bit read paths, A (pins DOA)
  and B (pins DOB), each with its latch and output register (`EbrPipeline`). A 36-bit read at
  port B fills both on the CLKB edge, from the word that ADB[13:5] selects. ADB[4:0] address
  nothing and are not inputs of the plan.
- **Order.** Bits 17:0 go on path `PDP36_LOW_PATH` and bits 35:18 on the other
  ([kernel/ebr.rs](../src/kernel/ebr.rs)). The path is A, the order of the yosys PDP wrapper.
  It was measured on the board (HB01, B01).
- **Controls.** CEB/CSB, OCEB, RSTB and REGMODE_B act on both paths
  (`PDP36_A_PATH_OWN_CONTROLS = false`), and RSTA, OCEA and CLKA on neither. This was measured
  (HB01). The plan refuses REGMODE_A ≠ REGMODE_B, which was not measured; both writers set them
  equal.
- **Report.** `ebr[].assumptions` states both rules and their source for each such block.
- **Board result.** The [HB01 bench](inventory/LFE5U-85F/hb01_pdp36/README.md) told eight
  hypotheses apart on the ULX3S, and the board matched this model in all 46 steps over 6
  loads. `ram32` ran on the board too ([HR02](inventory/LFE5U-85F/hr02_ram32_mac/README.md)).
- **Not measured:**
  - ASYNC reset and GSR on a 36-bit read;
  - CS on its read port;
  - read/write collisions outside the exact HB09P profile above.

Checks:
- `tests/ebr.rs` `pdp36_wide_read_drives_doa_and_dob_from_the_read_port`: 200 random masked
  writes and reads, NOREG and OUTREG; RSTB clears both paths and OCEB=0 holds both registers.
- `compiler_uses_composed_words_muxes_and_roots_unobserved_memory_writes` plans DOA17/DOB0 of a
  36-bit read and refuses mixed REGMODEs.
- `tests/ebr_wide_read_native.rs` runs the HB01 configuration. The emulator prints the 46 lines
  the board printed (`pass.txt`). `expect.py` gives the same lines from the bench program alone.
- `tests/reference_designs.rs` `ram32_reads_36_bit_block_ram_words_back` builds
  [examples/ulx3s/ram32](../examples/ulx3s/ram32/top.v) (a yosys 512×32 ROM and RAM, both
  36-bit reads). Its UART line `ROM C9083F00 RAM 0000` gives the sum of `rom.hex` and no wrong
  word.

Fast mode runs the 36-bit read the same way (`ebr_program` in
[kernel/fast.rs](../src/kernel/fast.rs)). The read code runs once on CLKB for 36 bits and puts each
bit on its path, and the DOA outputs belong to the CLKB group:
- `tests/ebr.rs` compares it with the kernel on two random [36, 36] configurations;
- the HB01 configuration prints the same 46 lines in fast mode;
- `ram32` prints `ROM C9083F00 RAM 0000` in fast mode at 7.1 MHz of board clock.

In the next milestone, the [MODE/WEAMUX overlap was qualified](EBR_MODE_OVERLAY.md): DP/NONE are determined from all the other bits, with explicit confirmation of the single override. Raw MODE keeps unmatched; the CIB CLK/LSR constants for a read-only port were also fixed. Unknown overlap forms are still rejected. Similar open overlaps of PIO are kept separately.

Collisions and combinations unconfirmed by hardware are recorded in `ebr_notices` with the time, the delta, and the resource. For a collision, the number of INIT9 rows and the first 16 addresses are stored. The ring is limited to `min(trace_capacity,1024)`; overflows are counted separately; with the trace disabled, the counters remain. Ordinary uncertainty of the input data is kept in 0/1/X/Z without a separate message for each bit.
Resolved HB09/HB09P accesses do not produce a conservative collision notice or increment
`ebr_collision_rows`; width-18 double writes still do.

The event-kernel replay in [tests/ebr_collision_native.rs](../tests/ebr_collision_native.rs)
loads `hb09s.config` from configuration and reads the DOA/DOB wires before each of its 128
edges. Every known bit must match the board, and X must occur exactly in e4 outputs reading
words poisoned by a previous width-18 double write. The test also counts same-word
collisions, disjoint narrow words in one row, and output bits the former policy made X.

### Fast-mode follow-up for HB09P

HB09's DP rule is already ported to fast mode. The HB09P PDP rule and selected
mux-input demand checks remain pending: apply
[fast_a_hb09p.patch](inventory/LFE5U-85F/hb09p_pdp_collision/fast_a_hb09p.patch), then
[fast_b_selected_demand.patch](inventory/LFE5U-85F/hb09p_pdp_collision/fast_b_selected_demand.patch).
The first forwards the pending full-word PDP36 write under the shared static
profile and equivalent dynamic guard. The second defers only fully checked
mux/LUT consumers to lazy runtime reads, retaining the capture check for other
consumers and rejecting a selected X even on a selector change without an EBR
edge. It does not pass an X into a Boolean shift/index. `src/fast.rs`'s error-2
handling needs no change.
`tests/ebr_r02.rs` has separate passing kernel tests and two fast tests ignored
with their pending-hunk reasons. They pass in the isolated patched preview.

## Checks and rerun

[Twelve portable tests](../tests/ebr.rs) check the full capacity of each address view, all 25 DP pairs, 16 PDP36 masks for each narrow read width, independent clocks and inversion, the main/additional output, write modes, CE/OCE/CS, reset/GSR/GWDIS, X/Z, unknown addresses/masks, collisions, and rejections. A separate composite-word test checks the CSDECODE reversal and a write into an unobserved EBR under all-plc.

The [native design](../tests/ebr_native.rs) uses DP18×18 with an asymmetric INIT, separate addresses/data/masks, a negative CLKA, inversion of CEA and CSA0, OUTREG on B, and an ordinary FF after DOA0. The outputs and the whole final array are compared with a separate expected model, without reading the intermediate computations of the kernel.

| Image and profile | Samples | FF / EBR / routes |
|---|---:|---:|
| `.config`, ready-unknown, observed | 513 | 1 / 1 / 1998 |
| `.bit`, ready-unknown, observed | 513 | 1 / 1 / 1998 |
| `.config`, wake-phases, observed | 513 | 1 / 1 / 1998 |
| `.bit`, wake-phases, all-plc | 513 | 83 640 / 1 / 712 920 |

Each run has 78/35 reads and 39/13 write attempts on A/B; the array was checked across all 1024×18 bits after the sequence. The active A/B edges are spaced apart, so the native fixture does not check physical collisions. In the wake runs, one write is suppressed separately before GWDIS is released. The corresponding `.config`/`.bit` and observed/all-plc pairs matched. At that milestone PDP and unknown collisions were software checks only. HB09P now supplies the raw PDP36 hardware subset above; other unknown/collision profiles remain unmeasured.

The CLI repeats ready-unknown with seven probes (outermost/ninth bits of A/B and captured), **514 samples** together with the initial state; all 513 working samples are compared with the same expectation. The JSON includes the composite EBR plan, which states are known, the counters, the transitions, and the messages of the conservative policy.

```sh
cargo test --locked
cargo test --release --locked --test ebr_native -- --ignored

# After the native test:
cargo run --release --locked -- emulate target/ebr-execution/top.bit \
  --startup ready-unknown --scope observed \
  --stimulus target/ebr-execution/events.json \
  --probe pin:C10 --probe pin:E8 --probe pin:G5 --output target/ebr-execution/user-run.json
```

In this fixture, C10/E8 are the low bits of qa/qb, G5 is captured; the other pins are given in the [saved fixture map](inventory/LFE5U-85F/ebr_execution_native.json). No GUI is required. Saved: [HDL](inventory/LFE5U-85F/ebr_execution_fixture.v), [LPF](inventory/LFE5U-85F/ebr_execution_fixture.lpf), [native results](inventory/LFE5U-85F/ebr_execution_native.json), [CLI](inventory/LFE5U-85F/ebr_execution_cli.json), [CLI stimuli](inventory/LFE5U-85F/ebr_execution_events.json), and [milestone check](inventory/LFE5U-85F/ebr_execution_validation.json).

At the first EBR milestone, **94 checks** passed: 85 ordinary ones and nine explicitly launched native integration ones. The carry counters, LUTRAM, global controls, the full architecture graph, CRAM/INIT, and the previous counter passed again. Clippy passed with no warnings, and formatting passed. The external blink with ECP5_TEST_BIT was not run separately. The materials check kept 78 open topics, 37 future experiment series, and zero board results.

The MODE/WEAMUX overlap was qualified by the next milestone. Next is adding the shared storages and the DSP arithmetic according to the [plan](IMPLEMENTATION_PLAN.md). HB01–HB12 remain in the [board queue](HARDWARE_VALIDATION.md); this milestone does not cancel the goal of the full chip, the peripherals, and the later GUI.
