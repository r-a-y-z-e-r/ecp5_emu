# Wires and routes of internal DSP nodes

The wire namespaces of PRADD9/18, MULT9 and ALU24 were reconstructed in all 39 DSP blocks of the LFE5U-85F. For the pre-adders, the routes to the A input of the multiplier were traced, and 156 control bits of these routes were found. This information complements the [analysis of the internal state](DSP_INTERNALS.md); the purpose of all physical lanes and the behavior of the blocks still need verification.

## What was collected and cross-checked

| Set | Size |
|---|---:|
| DSP configuration tiles | 702 instances, 18 types |
| Types that contain connection entries | 12; the other six contain configuration fields |
| Source connection entries by type | 6 873 |
| Entries after placement on the chip | 268 047 |
| Wire-name resolutions cross-checked against libtrellis | 340 899 |
| Source references of small and internal sites | All 5 733 kept |
| Namespaces mapped to these references | 546 |

These are the sizes of the description and of the mapping, not the number of independent hardware connections. Several descriptions may belong to alternative modes of a shared structure.

Density prefixes, relative N/S/E/W and coordinates are given according to `RoutingGraph::globalise_net_ecp5`. All 340 899 queries matched the installed libtrellis. The call checks the conversion of the name and the coordinates; the full RoutingGraph and the existence of every hardware path are not certified by this comparison.

[routing_report.json](inventory/LFE5U-85F/dsp_routing_research/routing_report.json) records the checksums of the inputs, the normalization source, the research programs, the verification binary and the loaded libtrellis. Before collection, the SHA-256 sums of the source database files from the inventory are verified.

## Interfaces of the pre-adders and the small modes

The expected names and directions are taken from the tables for PRADD9A/18A, MULT9X9D and ALU24B in the Lattice guides listed in [DSP_INTERNALS.md](DSP_INTERNALS.md#sources-and-limits-of-the-information). The wire mapping does not add new independently working resources.

| Family | Documented port names | Match with database names | Observation per namespace instance |
|---|---:|---|---|
| PRADD18 | 158 | Full | All 158 found in each of the 156 namespaces |
| PRADD9 | 86 | Full | All 86 found in each of the 156 namespaces |
| MULT9 | 125 | Full in aggregate | All 125 found on MULT9_1/5; the separate ROC[8:0] are not found on the 78 MULT9_0/4 |
| ALU24 | 148 | 146 matches | OP5/OP7 are observed instead of the documented OPADDNSUB/OPCINSEL; the functional correspondence is not assigned yet |

[port_catalogue.csv](inventory/LFE5U-85F/dsp_routing_research/port_catalogue.csv) stores 515 matched names, two documented names without a match and two observed names without an assignment. Directions for OP5/OP7 are not filled in from similarity to the documented signals.

PRADD shows PA, PB, C, SRIA, SRIB, PO, SROA, SROB, SOURCEA, OPPRE, and four each of CLK/CE/RST. MULT9 additionally has the product P[17:0], the signed signals and the outputs ROA/ROB/ROC. These names make it possible to continue tracing the internal paths. The presence of a name and the direction of a library port do not yet define its mode, value or switching time.

There is additional evidence for CFB: the ALU24B diagram and the database use CFB. The spelling CBF in the text list of the same reference guide remains a discrepancy in the source; it does not create a second set of inputs.

## Binding of site labels to wires

[endpoint_namespaces.csv](inventory/LFE5U-85F/dsp_routing_research/endpoint_namespaces.csv) maps each site label to a wire-namespace coordinate and a presumed parent slice. The original vendor coordinates and the number of repeated references are preserved.

| Family | Labels inside the block | Namespaces found on the chip |
|---|---|---:|
| PRADD18 | _0, _1, _4, _5 | 156 |
| PRADD9 | _0, _1, _4, _5 | 156 |
| MULT9 | _0, _1, _4, _5 | 156 |
| ALU24 | _2, _6 | 78 |

The mapping has the status `wire_namespace_candidate`. The confirmed bindings of the original common catalog are not renamed and not promoted to physical instances.

The hardware capacity of the 9×9 mode is eight multipliers per block, whereas the labels and the described MULT9 namespaces show four positions. Likewise, ALU24 has two sets of names per block against four small arithmetic lanes. From these data it is not yet possible to assign each of the two lanes inside the wide structure its own full interface. Creating 312 independent MULT9 by copying one wire set would require unconfirmed assumptions.

## Path from the pre-adder to the multiplier

The following path is traced in the database. The nodes in the diagram stand for wire names and source selection; this is a data-connection diagram for research.

```mermaid
flowchart LR
    P["PO of the pre-adder"] --> J["JPO"]
    J --> M["Input selection JMULTA"]
    I["Parallel JMUIA"] --> M
    M --> A["A of the multiplier"]
```

[preadder_paths.csv](inventory/LFE5U-85F/dsp_routing_research/preadder_paths.csv) contains 7 020 bit-level paths, 180 per block. Each path consists of three entries: the pre-adder output into JPO, the programmable selection of JPO for JMULTA, and the link from JMULTA to the A input. The PRADD9/18 and MULT9/18 entries are kept separately as description variants; their combinations still need a mode-validity check.

Example from the first block:

```text
R10C4:JPO0_PRADD18 → R10C4:JPO0 → R10C4:JMULTA0 → R10C4:JA0_MULT18
```

The JPO selection on this path requires CRAM F425B120=1. The exact sources are MIB_DSP0/bits.db, lines 1170, 4 and 710. Other positions have different bits and source tiles; the table keeps every actual binding.

[preadder_controls.csv](inventory/LFE5U-85F/dsp_routing_research/preadder_controls.csv) groups the paths by the 156 A inputs of the wide multipliers. One control bit was found for each position, shared by the 18 bit positions described. All 156 bits appear as `.mux` and are not among the existing 14 417 named resource fields. Their absence among the enum fields does not mean that the setting is absent from Trellis. These bits are now tied to the purpose of the route; the other operating conditions of the pre-adder remain open.

## Unconditional entries and mode selection

In 306 nodes of each block, fixed sources and programmable inputs occur at the same time, 11 934 such nodes in total. The full entries are in [fixed_mux_overlaps.csv](inventory/LFE5U-85F/dsp_routing_research/fixed_mux_overlaps.csv).

| Node name | Per block | Per chip | What overlaps |
|---|---:|---:|---|
| JMULTA | 54 | 2 106 | The parallel JMUIA and the selectable pre-adder output |
| JCFB | 54 | 2 106 | JNEXTR and the selectable JR |
| JDSPC | 54 | 2 106 | JMUIC and the selectable JROC |
| JP | 108 | 4 212 | JR and the selectable JCO |
| JQ | 36 | 1 404 | Outputs of different modes and the programmable output of the shift register |

For example, R10C5:JMULTA0 has a fixed entry from JMUIA0 and a programmable entry from JPO0, set by CRAM F819B120. The first is held by MIB_DSP1, the second by the neighboring MIB_DSP4. For the input of the first multiplier, the analogous selection is described by a single mux with an empty bit pattern for the parallel variant.

This requires an interpretation of the mode and of the bypasses. If all fixed and selected programmable entries are merged directly, several sources that belong to different DSP views could be connected by mistake. The source entries are kept; the conditions for applying them must be reconstructed separately. For 5 265 of the pre-adder paths found, such an overlap of a fixed source is marked.

## Chain boundaries

The connection descriptions contain 24 external PRADD/MULT9/ALU24 namespaces that do not match the candidate coordinates from the site labels. They belong to the start of the DSP rows and to the crossings of the central area; the exact coordinates are stored in the report.

For example, row 10 has PRADD/MULT9 references in columns 0 and 65 and ALU24 references in columns 1 and 66. The resolution of these relative names matches libtrellis. Binding them to real chain terminations or continuations requires additional evidence. Automatically creating additional physical resources there, or closing the chain onto the nearest DSP, is not allowed.

The first limited execution of the CIB paths for the parallel MULT18 is now described in [DSP_EXECUTION.md](DSP_EXECUTION.md); the exclusions of alternative views are kept in the runtime report. The full mode interpretation is not closed by this.

## Status of open questions

- D02 advanced: the PO→A paths and the control bits are known. The remaining internal muxes, OPPRE, symmetry and the bypass conditions are still needed.
- D03 remains open: the positions of the small lanes are described incompletely.
- D07 advanced: the interface names, the binding candidates and the directed paths are collected; the full graph of internal nodes is still to come.
- D08 was refined for CFB; the OP5/OP7 correspondence and the range discrepancies in the guides remain separately.
- D09 added: reconstruct the conditions for applying the fixed and the programmable views of one node.
- D10 added: establish the continuation and termination of the chains at the edges and in the central area.

No Diamond installation is available. The Yosys declarations for the DSP that were read are blackbox. The exact library models needed for D01/D04/D05 and for behavioral comparison have not been obtained yet. Structural collection for the other families can continue from local data and the guides.

## Reproduction

```sh
clang++ -std=c++14 -Wall -Wextra -Werror \
  -I ~/FPGA/prjtrellis/libtrellis/include -I /opt/homebrew/include \
  tools/trellis_wire_probe.cpp -L /opt/homebrew/lib/trellis -ltrellis \
  -Wl,-rpath,/opt/homebrew/lib/trellis -o target/trellis_wire_probe
python3 tools/collect_dsp_routing.py --native-probe target/trellis_wire_probe
```

The paths to the database, Trellis, the inventory and the output folder are given as arguments. A collection run without the native check stores the same set of research tables and states explicitly in the report that this check is absent. The path search is limited to three links and stops at the input of the next primitive; the internal arithmetic is not replaced by wires.

The [MULT18 output profile](DSP_OUTPUT.md) also qualifies the overlap of CIBOUT_BYP OFF with CLK0 and the registered product with an inactive ALU. This is a limited application of the paths to products; the full layout of the ALU/flags and the remaining D09 cases are not closed.
