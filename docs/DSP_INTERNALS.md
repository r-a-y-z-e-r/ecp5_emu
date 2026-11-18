# DSP internal structure and open questions

For the LFE5U-85F, information is collected on the two slices of each of the 39 DSP blocks, the width variants, the registers, the data-transfer chains and the shared control settings. The bit-level mapping covers all 156 MULT18 and 78 ALU54. It found 1 326 groups of identical enum encodings and 702 overlaps of register-setting bits between resources. Later additions are [the first executable MULT18 profile](DSP_EXECUTION.md) and [the logical input/pipeline stages of MULT18](DSP_REGISTERS.md), with the explicit D05 assumptions. Then [a limited registered MULT18 output in a single bank](DSP_OUTPUT.md) was added, keeping D01/D05/D06. Physical ownership of the ALU/flag bits, the output CE/RST/SIGNEDP, ALU/PRADD and other modes remain separate stages.

An additional [map of wires and routes](DSP_ROUTING.md) maps all 5 733 references of internal and small sites to namespaces. The collection includes the PO→A paths, 156 control bits, overlaps of fixed and programmable views, and gaps at the chain boundaries.

## Sources and limits of the information

| Source | What it establishes |
|---|---|
| [Lattice sysDSP FPGA-TN-02205-1.3](https://www.latticesemi.com/view_document?document_id=50469), September 2024, pp. 6–9 | Hierarchy, alternative widths, shared output registers, directions of the main chains |
| The same document, pp. 13–25 | Interfaces and attributes of the legacy primitives MULT9X9C, MULT18X18C, ALU24A, ALU54A and of the pre-adders PRADD9A/18A |
| [Diamond 3.13 FPGA Libraries Reference Guide](https://www.latticesemi.com/view_document?document_id=53950), August 24, 2023, pp. 197–203, 587–591 | Specifically ALU54B and MULT18X18D, the ones Trellis uses; the register split and the CFB mode |
| The same reference guide, pp. 186–189, 625–629, 745–750 | ALU24B, MULT9X9D and PRADD18A/9A, shift directions and symmetry modes |
| Trellis `libtrellis/src/Bels.cpp:328–413` | The MULT18X18D and ALU54B ports visible in the graph constructor |
| nextpnr `ecp5/arch_place.cc`, functions `is_dsp_location_valid` and `remap_dsp_blocks` | Shared CLK/CE/RST sets for the two slices of one DSP block |
| nextpnr `ecp5/bitstream.cc`, functions `get_dsp_tiles`, `write_mult18`, `write_alu54` | How this version of the tool applies fields to tiles, and the writes it makes |
| Local Yosys `lattice/cells_bb_ecp5.v:215–256,488–559` | Declarations of interfaces and parameters; both DSP modules are marked blackbox |

Source revisions are pinned in the [device passport](DEVICE_PASSPORT.md). The SHA-256 sums of the files read and of the two PDFs are stored in [evidence_report.json](inventory/LFE5U-85F/dsp_research/evidence_report.json). The PDFs with figures were checked visually. The research copies are in `target/research/dsp`; the source URLs allow them to be downloaded again.

The designations C/D on the multipliers and A/B on the ALU belong to different library interfaces. When information is carried over, the specific primitive must be checked. For example, the presence of CFB is confirmed for ALU54B by a separate reference guide. Blackbox declarations alone give no rules for computing the outputs.

## Hierarchy and alternative modes

One DSP block contains two slices. Each slice holds two 18-bit pre-adders, two 18×18 multipliers, an ALU54, input and intermediate registers, output storage, muxes, and shift and cascade chains. A pre-adder is used together with its multiplier. The multipliers and the arithmetic blocks can also be used on their own. Basis: FPGA-TN-02205-1.3, pp. 6–8, figures 2.1–2.3.

| View of one full block | Resources in this mode | Corresponding capacity of the 39 blocks |
|---|---|---:|
| 18×18 multipliers | Four multipliers and two add/accumulate blocks | 156 multipliers, 78 ALU54 |
| 9×9 multipliers | Eight multipliers and four add/accumulate blocks | 312 multipliers, 156 ALU24 |
| 36×36 multiplier | One multiplier occupying the full block | 39 multipliers |

The capacities in the rows are alternative ways to use the shared structure. They are computed from the number of DSP blocks and the modes of the guide, pp. 8–9. The guide describes the 36×36 row as a mode without additional add/accumulate blocks. Summing these capacities separately, or summing raw site references, would give a wrong chip size.

The PRADD9/18 and ALU24/54 variants also use a shared structure. The exact correspondence of each small lane to the source site references and wire names still needs binding. The current DSP_SLICE_LEFT/RIGHT define the parents of the resources; internal nodes were not added to the executable model at this step.

## Registers and state owners

| Node | Confirmed rule | What must be mapped to bits and wires |
|---|---|---|
| MULT18 inputs | A/B/C and ROA/ROB/ROC are 18 bits wide; the input and pipeline registers are selected by the CLK/CE/RST settings | The exact placement of the FFs and how they are shared with the pre-adder and the ALU |
| Multiplier pipeline | In the simplified diagram PR is 36 bits wide; REG_PIPELINE_* are available | The reset value, control of the signed state, and all bypasses |
| Shared slice output | OR and the framing register FR use a shared 72-bit storage; its purpose changes between the MULT and ALU modes | The bit-by-bit allocation among products, R, flags and framing in each mode |
| ALU54B INPUTC0 | The low 27 bits of C, that is C[26:0] | Physical banks and the full set of control signals |
| ALU54B INPUTC1 | The high 27 bits of C, that is C[53:27] | Physical banks and the full set of control signals |
| ALU54B OUTPUT0 | R[17:0] | Link to the shared 72-bit storage |
| ALU54B OUTPUT1 | R[53:18] | Link to the shared 72-bit storage |
| ALU54B CFB | With REG_INPUTCFB_CLK=NONE the C path through C0/C1 is used. When a clock is selected, CFB is registered onto the CO path and C passes without an input register; C0/C1 must be NONE | Where this mode is encoded and which registers switch the purpose |
| Dynamic operations | ALU54B has 11 OP inputs and the register groups OPCODEIN/OPCODEOP; the pre-adder has OPPRE and REG_OPPRE_* | The operation table, the delays of the control bits and their alignment with the data |

The widths of MULT and its outputs are confirmed by the Diamond reference guide, pp. 587–591, and by Bels.cpp. The shared output and PR are confirmed by the sysDSP guide, pp. 7–8. The split of C and R and the CFB rules are stated directly in the notes on ALU54B on p. 200 of the Diamond reference guide.

So for ALU54B the input is split as 27+27 and the output as 18+36. These boundaries must be preserved when the state is reconstructed. When several fields select the same clock, this establishes that their configuration control is common. To assign them specific FFs, the data paths themselves still need to be bound.

## Chains and source selection

Figure 2.3 of the sysDSP guide shows a 54-bit CIN from the DSP on the left and a COUT to the DSP on the right. The main SRIA/SRIB come from the left and SROA/SROB go to the right; the chain contains cascade matching registers. The pre-adder has a separate reverse chain: its SRIB can come from the neighboring pre-adder on the right. So the same name SRIB in different library interfaces requires a direction check.

PRADD9A/18A are documented with the inputs PA, PB, C, SRIA, SRIB, SOURCEA and OPPRE, the outputs PO, SROA, SROB, and four each of CLK/CE/RST. The data bus widths correspond to 9 or 18 bits. Basis: sysDSP, tables 4.9 and 4.14; Diamond, pp. 745–747.

For PRADD, the Diamond reference guide, pp. 748–750, establishes the following rules:

- SOURCEA_MODE allows A_SHIFT, C_SHIFT, A_C_DYNAMIC and HIGHSPEED. SOURCEA takes part in selecting the parallel or the shift source.
- SOURCEB_MODE selects the input from the right neighbor, the parallel PB, or the internal output of Reg. 12.
- FB_MUX selects Reg. 16 or the bypass through Reg. 15; DISABLED has a special purpose for the placement of the left pre-adder.
- PRADD_LOC sets the left or the right position. SYMMETRY_MODE and SOURCEB_MODE determine direct data feed or internal delayed data for symmetric filters.

The numbers Reg. 12/13/15/16 are vendor designations. The mapping of these numbers to our permanent IDs has not been established yet. The full connection diagram and the conditions under which the cascades are available remain open. The local nextpnr support list separately states limited support for ALU54B and no support for the MULT18X18D cascade in its flow.

## Identical encodings and CRAM overlaps

[enum_encoding_aliases.csv](inventory/LFE5U-85F/dsp_research/enum_encoding_aliases.csv) compares the combined requirements of each enum variant against all global CRAM bits. For the included groups, the variant names, the coordinates and the polarities match. All variant domains are complete, fragment by fragment. The default values in this comparison do not replace observed bits.

The same 34 groups were found in all 39 blocks, 1 326 in total. The groups consist of different resources of one block:

| Type of shared encoding | Groups per DSP block |
|---|---:|
| CE0MUX–CE3MUX of the MULT pair of one slice | 8 |
| CLK0MUX–CLK3MUX of the MULT pair of one slice | 8 |
| CLK0_DIV–CLK3_DIV of the two MULTs and the ALU of one slice | 8 |
| GSR of the two MULTs and the ALU of one slice | 2 |
| REG_INPUTC_CLK of MULT and the corresponding REG_INPUTC0/1_CLK of the ALU | 4 |
| RST0MUX–RST3MUX of all four MULTs of the block | 4 |

For example, MULT18_0.REG_INPUTC_CLK matches ALU54_3.REG_INPUTC0_CLK, and MULT18_1.REG_INPUTC_CLK matches ALU54_3.REG_INPUTC1_CLK. The right slice has an analogous pair. This is an equality of the full known encodings, not only an overlap of individual bits.

[register_bit_overlaps.csv](inventory/LFE5U-85F/dsp_research/register_bit_overlaps.csv) additionally stores 702 pairs of register fields that share bits: 18 pairs per block. Some of the MULT output settings overlap with OUTPUT0, OUTPUT1 and FLAG of the ALU, while keeping different full bit patterns. Such overlaps require a joint interpretation of the mode; creating an independent set of registers for each field name on its own may double the state.

The control connections also share the block context. nextpnr limits the two slices to four distinct CLK, four CE and four RST, and renumbers the inputs across all six primitives. This check has not yet established the equality of all internal control wires of the physical graph.

## Library parameters and known fields

[parameter_coverage.csv](inventory/LFE5U-85F/dsp_research/parameter_coverage.csv) matches the 72 parameters of the local Yosys declarations against the presence of fields with the same name on the instances. This is a name-match check; for a missing name, the question of a possible different encoding remains.

| Primitive | Parameters in the declaration | Field with the same name exists | No field with the same name |
|---|---:|---:|---:|
| MULT18X18D | 25 | 19 | 6 |
| ALU54B | 47 | 35 | 12 |

MULT18 has no standalone fields of the same name for REG_INPUTC_CE, REG_INPUTC_RST, REG_OUTPUT_CE, HIGHSPEED_CLK, CAS_MATCH_REG and MULT_BYPASS. REG_OUTPUT_RST is present on only 78 of the 156 instances, with indices 0 and 4.

ALU54 lacks REG_INPUTC0_CE/RST, REG_INPUTC1_CE/RST, REG_OUTPUT0_CE, REG_OUTPUT1_CE, REG_FLAG_CE/RST, REG_INPUTCFB_CLK/CE/RST and MULT9_MODE. The other 35 fields of the same name are present on all 78 instances.

Direct writes of 19 parameters were found in write_mult18 and of 33 in write_alu54. For the two ALU parameters REG_OUTPUT0_RST and REG_OUTPUT1_RST, the fields exist in the database, but this function has no direct writes of these names. This is the limit of checking specific functions; it does not replace a study of the transformations of the whole nextpnr flow.

The Trellis studies 073/075 list, among others, CAS_MATCH_REG, MULT_BYPASS and MULT9_MODE, which are not in the current catalog under these names. The mere existence of a study task does not establish individual observed bits. These tasks use the LFE5U-25F; a hardware check on the 85F that is on hand is still to come.

## Open questions

| ID | What information is needed | Next evidence |
|---|---|---|
| D01 | Every bit of the shared output storage and its owner in the MULT/ALU/9-bit modes | Vendor models, port tracing and configuration experiments |
| D02 | The remaining settings of the pre-adders, OPPRE, SOURCEA_MODE, FB_MUX and SYMMETRY_MODE | The PO→A paths and 156 control bits are collected in DSP_ROUTING.md; next, the conditions of the modes and the remaining muxes |
| D03 | Selection of the small lanes and MULT9_MODE | Comparison of the configurations of the 9/18/36-bit modes and of the lane placement |
| D04 | The full semantics of OP[10:0], flags, rounding, saturation, signed extensions | Executable library models and operation tables |
| D05 | Initialization, reset/CE, bypasses, the order of capturing data and control registers | Vendor models and edge checks |
| D06 | Parameters without a standalone field, including CFB and CAS_MATCH_REG | A search for alternative settings, fixed conditions and shared control bits |
| D07 | Inputs/outputs and directions of each internal lane across the whole chip | 515 port names and 546 namespaces are mapped; next, the second small lanes, ROC and the full directed graph |
| D08 | Ambiguities of the designations in the guides | A check against the exact library declarations and connections |
| D09 | Conditions for applying the fixed and the programmable views of one DSP node | DSP_ROUTING.md stores 306 cases per block, including the pre-adder inputs and the output bypasses |
| D10 | Continuation and termination of the chains at the boundaries and in the central area | DSP_ROUTING.md stores 24 external namespaces without a binding to site labels |

D08 has concrete examples. Table 4.14 of sysDSP gives C[47:27] for PRADD18, which is 21 positions on an 18-bit port. In the ALU24B section of the Diamond reference guide, CBF appears, while the diagram, the attributes and the database use CFB. In addition, OP5/OP7 were found in the database where the library description has OPADDNSUB/OPCINSEL. A functional correspondence of the latter two signals is not assigned from interface similarity alone.

## Reproducing the collection

```sh
python3 tools/collect_dsp_evidence.py \
  --sysdsp-pdf target/research/dsp/FPGA-TN-02205-1.3.pdf \
  --library-pdf target/research/dsp/Diamond-3.13-FPGA-Libraries.pdf
```

The paths to Trellis, nextpnr, the Yosys declaration, the inventory and the output directory are given as arguments. The generator checks the structure of the DSP groups, the CRAM boundaries, and that no opposite requirements on one bit exist inside a variant. Checksums of all sources read are computed before the result is written. The PDF arguments in this command fix the checksums of the sources; the substantive relations from the guides were checked separately and are described above.

No Diamond installation is available, and exact behavioral models of the DSP have not been obtained yet. The DSP priorities are the remaining route conditions and D01–D10. For the full chip, collection also continues for IOLOGIC, clock and system resources; the status of the other families stays in the [coverage matrix](RESOURCE_COVERAGE.md).

Runtime addition: [ROA/ROB](DSP_FORWARDING.md) forward the operands through the existing INPUTA/B. For all 156 MULT, 5 616 fixed connections to the ALU inputs were cross-checked. This advances D07 in structure; the register contract D05, the ALU operations and D01–D10 remain open.
