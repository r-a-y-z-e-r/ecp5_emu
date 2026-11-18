# EBR addressing and modes

After the [toolchain update](TOOLCHAIN.md), the address tables and the six synthesis probes were checked again with the new Trellis and Yosys 0.70. All 12 288 INIT words and the substantive tables matched. The report in the main directory was updated with the versions and hashes; the previous snapshot is kept in the archive. Hardware questions B01–B08 are not closed by this.

For all 208 EBRs of the LFE5U-85F, the following were collected: the memory views by port width, the address pin assignment, the write modes, and the conversion of INITVAL and of the public PDPW16KD. Six synthesis designs confirm the address and INIT layout in the local Yosys version. This is a research contract. An [executable EBR](EBR_EXECUTION.md) with its own boundaries was added later; no hardware check was made.

The continuation is [EBR control](EBR_CONTROL.md): the CE/OCE/CS/reset rules, seven additional synthesis designs, and 12 series of future hardware experiments covering all of B01–B08. Under the agreed [board check order](HARDWARE_VALIDATION.md), these questions are kept until measurements and do not delay collecting the other families.

Each block has one shared storage of 18 432 bits. To stay compatible with the existing decoder, it is convenient to keep it as 2 048 nine-bit INIT words. This is an architectural representation of the data; the SRAM transistor geometry is not recovered from these sources. Ports A/B and the width modes access this shared state.

## Widths and address pins

The [datasheet FPGA-DS-02012-3.4](https://www.latticesemi.com/view_document?document_id=50461), p. 27, defines five true dual port widths and a sixth width, 36, for single/pseudo dual port. The internal DP16KD interface has 14 address pins each for A and B. The useful word address starts at a different pin depending on the width.

| Word width | Depth | Word address pins | Address shift | Available array bits |
|---:|---:|---|---:|---:|
| 1 | 16 384 | AD[13:0] | 0 | 16 384 |
| 2 | 8 192 | AD[13:1] | 1 | 16 384 |
| 4 | 4 096 | AD[13:2] | 2 | 16 384 |
| 9 | 2 048 | AD[13:3] | 3 | 18 432 |
| 18 | 1 024 | AD[13:4] | 4 | 18 432 |
| 36 | 512 | AD[13:5] | 5 | 18 432 |

In the table, AD means ADA or ADB depending on the mode. For DP16KD, the A/B widths are chosen independently from 1, 2, 4, 9, 18. For the public PDPW16KD, the write width is 36 and the read width is one of six. A generated module may provide a narrower logical write port by spreading it over the groups and write enables of the primitive.

The low pins cannot be included in the word address unconditionally:

- In DP of width 18, pins AD0/AD1 carry the write enables of the groups DI[8:0] and DI[17:9]. In the Yosys probes, AD2/AD3 are zero.
- In PDP, pins ADA0…ADA3 carry four write enables of nine bits each; ADA4 is zero in the direct Yosys wrapper, and ADW[8:0] goes to ADA[13:5].
- On the PDP read side, the low pins below the selected shift are not part of the word address. Such a pin must not be used to create an extra address bit or a separate memory state.

These mappings come from `brams_map_16kd.v` and `nextpnr::pack_ebr`; for the five DP configurations and PDP36, the connections of the synthesized design were also checked. They do not define the behavior of undocumented combinations of the low pins. The full role table has 238 rows in [address_pin_roles.csv](inventory/LFE5U-85F/ebr_research/address_pin_roles.csv).

Before interpreting the control levels, the mux settings must be taken into account. Trellis has ADA0MUX…ADA3MUX, ADB0MUX/ADB1MUX, and separate WE/CE/OCE/CLK/RST muxes with direct and inverted input variants. In write-enable mode, the low address pin first gets its effective level through its own mux. The settings of all 30 fields of each EBR are stored in [field_fragments.csv](inventory/LFE5U-85F/ebr_research/field_fragments.csv); the completeness of MODE/width recovery and the context of application remain the decoder's responsibility.

Clarity Designer also offers logical byte enable groups of eight bits ([Memory User Guide](https://www.latticesemi.com/view_document?document_id=50466), p. 11). This is an interface of the generated module: the primitive groups confirmed here contain nine bits. Their exact packing for each generator setting must be read from the generated design.

## Shared storage and mixed widths

Let `a` be the word address of the selected port, `b` the data bit index, and `w` the width. The coordinates in the stored INIT image are computed as follows:

```text
w = 1, 2, 4:
    n = 8 / w
    word9 = a / n                  # integer division
    bit9  = (a % n) * w + b

w = 9, 18, 36:
    word9 = a * (w / 9) + b / 9    # integer division
    bit9  = b % 9
```

At widths 1/2/4, the low eight bits of each INIT word are used; the ninth bit is not reachable through this view. It is kept in the shared image, because the width of the other port may be 9 or 18. The ninth bits do not turn into automatic ECC logic.

For example, a word of width 18 at address 5 occupies INIT words 10 and 11. Another port of width 9 sees them at addresses 10 and 11. A port of width 1 sees the low eight bits of these words at addresses 80…87 and 88…95. So comparing the numeric addresses of A and B does not determine whether the accesses overlap.

[lane_templates.csv](inventory/LFE5U-85F/ebr_research/lane_templates.csv) stores 204 data positions for the repeating block of four INIT words. [mixed_width_overlaps.csv](inventory/LFE5U-85F/ebr_research/mixed_width_overlaps.csv) holds 454 view overlaps for all 25 DP width pairs in this block. This is an overlap of data cells, not an exhaustive rule for hardware collisions: the granularity of conflicts with masks and narrow accesses is not yet established.

## Read, write, and output registers

All modes register the memory inputs, including address, data, WE, and byte enable. `NOREG` disables the additional output register; the read stays synchronous. `OUTREG` adds an output stage, which on the timing diagrams is one more read cycle. Sources: datasheet, p. 27, and [Memory User Guide](https://www.latticesemi.com/view_document?document_id=50466), pp. 12, 21–26, 29. The NORMAL, WRITETHROUGH, READBEFOREWRITE, and PDP diagrams were compared visually.

The write mode is set separately for each true dual port. The table describes the result on the read path of the same port during a full write, before the additional OUTREG stage.

| WRITEMODE | Result when the same port writes | Documented DP widths |
|---|---|---|
| NORMAL | The previous output value is held | 1, 2, 4, 9, 18 |
| WRITETHROUGH | The write input data is passed to the output | 1, 2, 4, 9, 18 |
| READBEFOREWRITE | The old contents of the word being written are passed to the output | 9, 18 |

The rules follow from Appendix A.4 of the guide, p. 59. They do not define the result of the other port during a collision and do not settle the behavior of the unmasked output bits during a partial write.

In Trellis, the variants of the three WRITEMODEs exist independently of the width fields. The Yosys memory library `brams_16kd.txt` also declares `rdwr old` for all five widths. The documentation limits READBEFOREWRITE to widths 9/18 in DP. Therefore the three combinations with widths 1/2/4 in [write_modes.csv](inventory/LFE5U-85F/ebr_research/write_modes.csv) are marked as outside the documented support. The presence of an encoding does not prove the required hardware behavior.

**Measured on 2026-11-16 by [HB11](inventory/LFE5U-85F/hb11_ebr_narrow_rbw/README.md)** on the ULX3S: DP16KD `READBEFOREWRITE` at widths 1, 2 and 4 behaves as at widths 9 and 18 on both NOREG and OUTREG ports (0 mismatches across 8 680–8 774 known writes per port). The emulator executes narrow `READBEFOREWRITE`.

The signals CE, OCE, CS, the resets, and the clock inversion must remain separate inputs of the model. CS is a three-bit comparison with a configured value, not an ordinary one-bit enable. OCE belongs to the output register. The exact order of action of CE/OCE/CS and reset for the output latches and the additional register still needs to be checked; the WRITEMODE table does not replace these rules.

`RESETMODE` allows SYNC/ASYNC; `ASYNC_RESET_RELEASE` is set separately. According to p. 12 of the guide, reset clears the input/output registers; p. 28 of the datasheet describes the A/B output latches and GSRN. Both sources let us keep the main rule: **reset does not clear the memory array**. The details of resetting the input state, of reset release, and of write suppression are not yet collected into a verified transition table.

## PDPW16KD: one block, two clock domains

The public PDPW16KD has 111 separate ports. nextpnr translates it into 112 connections of the native DP16KD: the common RST is duplicated onto RSTA/RSTB. Writes belong to CLKW/CLKA, reads to CLKR/CLKB. In PDP mode, both output sets DOA/DOB belong to the read side; the letter A in an output name does not assign the write clock to it.

The main conversions of `nextpnr::pack_ebr`:

| Public port | Native interface |
|---|---|
| ADW[8:0] | ADA[13:5] |
| BE[3:0] | ADA[3:0] |
| ADR[13:0] | ADB[13:0] |
| DI[17:0] / DI[35:18] | DIA[17:0] / DIB[17:0] |
| CLKW / CLKR | CLKA / CLKB |
| CEW / CER / OCER | CEA / CEB / OCEB |
| CSW / CSR | CSA / CSB |
| RST | RSTA and RSTB |

WEA/WEB are not separate write enables of the public PDP. The BE masks and CEW must be interpreted in the context of MODE. [PDP_lowering.csv](inventory/LFE5U-85F/ebr_research/PDP_lowering.csv) lists all 112 connections; the collector checks that the set of public ports is covered.

A discrepancy was found between two conversion paths of the 36-bit output:

| Path | Low 18 bits of the logical output | High 18 bits |
|---|---|---|
| Direct Yosys wrapper `$__PDPW16KD_` → DP16KD | DOA[17:0] | DOB[17:0] |
| Public PDPW16KD → `nextpnr::pack_ebr` | DOB[17:0] | DOA[17:0] |

For reads of width 1/2/4/9/18, both paths use the low part of DOB and agree. For width 36, all 36 assignments differ. The synthesis probe confirms the first path for the installed Yosys specifically; it does not establish the physical output order of the chip. [PDP_output_comparison.csv](inventory/LFE5U-85F/ebr_research/PDP_output_comparison.csv) keeps both variants. Until question B01 is closed, the model must not silently pick one order as the confirmed hardware order.

**Closed on 2026-11-11 by [HB01](inventory/LFE5U-85F/hb01_pdp36/README.md)**, measured on the
ULX3S:
- the first path is the chip's: bits 17:0 of a 36-bit read (written through DIA, INIT chunk 2w)
  come out on DOA, and bits 35:18 on DOB;
- reading nextpnr's source rather than measuring it, a public PDPW16KD placed by nextpnr gets
  the two halves of `DO` swapped.

## INITVAL and CSDECODE encoding

`nextpnr::write_bram` parses 64 INITVAL parameters of 320 bits each. Each parameter consists of 16 chunks of 20 bits: 18 useful bits and two skipped bits. For parameter `k`, chunk `j`, and useful bit `d`:

```text
INITVAL_bit = 20*j + d
word9       = 32*k + 2*j + d/9
bit9        = d%9
```

This gives 18 432 useful positions and 2 048 skipped parameter positions. These skips are not additional memory. [INITVAL_layout.csv](inventory/LFE5U-85F/ebr_research/INITVAL_layout.csv) lists every useful position. The binding of the resulting image to the instance through WID is already implemented in the [decoder](CONFIG_DECODER.md).

For CSDECODE, the nextpnr writer takes the inversion of each CS input into account, then reverses the order of the three bits:

```text
raw_pin_match = public_decode XOR CS_input_inversion_mask
Trellis_word  = reverse3(raw_pin_match)
```

For example, decode `101` with inversion mask `011` requires raw CS levels `110`; the Trellis word stores `011`. All 64 combinations are listed in [CSDECODE_encoding.csv](inventory/LFE5U-85F/ebr_research/CSDECODE_encoding.csv). When reading a finished bitstream, the word order must be reversed and compared with the raw inputs. Recovering the unavailable original HDL mask is not needed for such a comparator to work; applying it again would mean a double inversion.

## Port collisions and uncertainty

Appendix A.4, p. 59, warns of a possibly unknown read result when the other port writes to the same address at the same time. If both ports write opposite values at the same time, the contents are undefined. The source does not guarantee a specific winning port, safe writing of equal values, or independence of all non-overlapping narrow subwords.

For the future digital kernel, the following requirements follow:

- Match accesses in the shared storage taking width and masks into account; numeric equality of the port addresses is not enough.
- First collect the accesses of both domains at the coinciding virtual time, then resolve the result. The A/B traversal order in the program must not pick the winner.
- Keep the possibility of an unknown result separately for the output and for the array cells. `READBEFOREWRITE` of one port does not guarantee old data for the other port.
- Express the granularity of unknown bits and the collision window as an explicit model policy after verification. Giving up full timing-awareness does not turn the hardware ambiguity into a fixed A/B priority.

The event kernel now resolves the measured subset from
[HB09](inventory/LFE5U-85F/hb09_ebr_collision/README.md): equal-width, equal-WRITEMODE,
full-word DP accesses on one clock edge with CE/OCE active. A read against a write gets
old data for RBW and new data for NORMAL/WRITETHROUGH; each writer keeps its own mode's
output. At widths 1/2/4/9 a same-word double write leaves B's data in the array. Width 18
double writes sometimes mix bits, so their array result remains X.

Different equal-width narrow words within one INIT9 row are independent in this scope:
the clean reads and writes in HB09 include these pairs and have zero mismatches at
widths 1/2/4. [HB09P](inventory/LFE5U-85F/hb09p_pdp_collision/README.md) adds raw PDP36/NOREG/NORMAL
same-word reads against a full four-lane write on one shared definite edge:
**NEW** data, with active CE/CS, OCE=0/1, inactive reset/GWDIS and GSR disabled,
ASYNC reset/release. The physical write/read widths are 36/36; inferred logical
32/36 at depths 64/128/256/512 use this profile. All 1,299 known collisions per
case return new data, and later clean reads agree. The bench instantiates
primitives and observes raw DOA/DOB, not inferred-RAM forwarding outputs.
Mixed DP widths/modes, partial masks, other PDP profiles and different
clocks/phases keep the conservative row-wide policy. The physical collision window remains unmeasured; no
window is added between separate event times. See [EBR_EXECUTION.md](EBR_EXECUTION.md).

## ROM, ECC, and FIFO within a design

The datasheet, p. 27, describes ROM as a preloaded EBR with writes inhibited. No separate array or new physical resource is needed for this use.

The guide, p. 11, explicitly places ECC in PFU logic outside the EBR. On p. 41, the FIFO_DC flags are likewise implemented by logic around the memory. When a bitstream is executed, the loaded fabric must reproduce these parts. An automatic ECC decoder or FIFO flags inside every EBR would change the behavior of the design. The full structures of the generated FIFOs and the packing of eight-bit byte enables are not yet recovered at this stage.

## Source discrepancies and open questions

Besides the two PDP36 orders and the READBEFOREWRITE restriction, the following discrepancies are recorded:

- On p. 19 of the Memory User Guide, the depth `2049×9` is printed. 2048 is used: this is the value of the datasheet, of the block capacity, and of all INIT probes; the typo was checked against the page image.
- On p. 27 of the same guide, the PDP diagram labels DI/DO as [17:0]. The [Diamond 3.13 library guide](https://www.latticesemi.com/view_document?document_id=53950), p. 735, and the public declaration give DI/DO[35:0]; the 111 ports were checked against the declaration. The figure label is not taken as the primitive's interface.
- The scope of reset is described in different detail in the guide and the datasheet. Array clearing is not added; the missing transitions remain open.

| ID | What needs to be clarified |
|---|---|
| B01 | Physical order of the two halves of the 36-bit output and its match to the conversion paths. Closed by HB01: DOA carries 17:0, as the yosys path |
| B02 | Priority of CE/OCE/CS/reset for the output latch and the additional register |
| B03 | Sequence of ASYNC_RESET_RELEASE, reset of the input state, and write inhibit |
| B04 | Output of WRITETHROUGH/READBEFOREWRITE with a partial nine-bit mask |
| B05 | HB09 settles same-edge equal-width/mode full-word DP accesses: RBW reads old, NORMAL/WRITETHROUGH reads new; disjoint narrow words in one INIT9 row are independent (zero clean-case mismatches). HB09P adds raw PDP36/NOREG/NORMAL same-clock full-word reads returning NEW with its measured controls (1,299/1,299 known collisions per case). Other PDP profiles, mixed DP widths/modes, partial masks and different clocks/phases remain open ([HB09](inventory/LFE5U-85F/hb09_ebr_collision/README.md), [HB09P](inventory/LFE5U-85F/hb09p_pdp_collision/README.md)) |
| B06 | HB09 settles same-word double writes at widths 1/2/4/9: B wins, including equal data; each writer outputs per its own mode. Width 18 showed bit mixes and remains X. Equal-width disjoint narrow words are independent. HB09P confirms later clean reads after its single PDP writer, not a double-write priority; mixed-width subwords and masks remain open ([HB09](inventory/LFE5U-85F/hb09_ebr_collision/README.md), [HB09P](inventory/LFE5U-85F/hb09p_pdp_collision/README.md)) |
| B07 | Real behavior of undocumented combinations of width and WRITEMODE. Closed for DP widths 1/2/4 READBEFOREWRITE by [HB11](inventory/LFE5U-85F/hb11_ebr_narrow_rbw/README.md): behaves as widths 9/18 |
| B08 | Effect of INIT/configuration on the output latches and registers |

The DP16KD declaration in `cells_sim_ecp5.v` is a blackbox, not an executable reference memory. The Yosys mapping library sets synthesis intentions and does not replace a behavioral reference. No Diamond installation is available; the listed questions allow later measurements on the available chip. This stage involves no programming of the board and no measurements.

## Data and reproduction

[ebr_research](inventory/LFE5U-85F/ebr_research/evidence_report.json) holds 15 files. Besides the address tables, modes, and conversions, they contain the list of 208 instances, all 6 240 composite fields from 7 384 fragments, 15 contract rules, all probe connections, and the checksums of the sources.

```sh
python3 tools/collect_ebr_evidence.py --verify-yosys
```

The collector verifies the checksums of the source database and of the Trellis files. The first snapshot used Trellis `73bd411731808d80ead650bbc9840d9e02911e9e`, database `015e0330630d7c238c0e4f2cdd9c8157eb78c54a`, nextpnr `58e3b85866de409eac8b61393746140e7b30c808`, and Yosys 0.66 (`7f8fdfd8d7bc08c749a2a969388d3425d4f369d5`). The current main report was updated after a rerun with the versions from [TOOLCHAIN.md](TOOLCHAIN.md); the previous one is kept in the archive. The hashes of the PDFs read, of the sources, of the Yosys binary, and of the probes are stored in the corresponding report; the PDF cache is in `target/research`.

The six designs cover the five full DP configurations and PDP36. Each ended up with one DP16KD; the address domains, shifts, low address pins, and 2 048 INIT words with distinguishable data were checked. In total, 12 288 words were matched; for PDP36, the assignment of all 36 outputs of the direct wrapper was also checked. Yosys may swap the symmetric ports A/B, which the check accounts for.

Regeneration with the new probe files gave byte-identical 15 research files. Independently checked: the capacity and the absence of overlaps within each of the six views, all 25 DP overlap pairs, the 18 432 INIT positions, the 64 CSDECODE conversions, and the native PDP assignments for all 208 EBRs. In the conversion of the public PDP, WEA/WEB/OCEA/ADA4 stay unconnected; this was checked against the full native interface of 116 ports.

These probes confirm the synthesis conversions and the INIT image. They do not execute WRITEMODE, resets, output registers, or collisions on a reference model or chip. Checking the behavior remains the next separate result.
