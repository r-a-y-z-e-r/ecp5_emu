# Binding of sites to LFE5U 85F resources

The main instances of logic, IO, memory, DSP, global and edge clock management, and user system blocks are bound. This is a structural catalog built from local Trellis sources, not a check of behavior on the die. It complements the [passport](DEVICE_PASSPORT.md); the full list of source sites is kept regardless of the binding result. Details are in the descriptions of [clock and system resources](CLOCK_SYSTEM_RESOURCES.md) and [EBR/DSP](MEMORY_DSP_RESOURCES.md).

## Reproduction and results

```sh
python3 tools/inventory_trellis.py --verify-pytrellis
python3 tools/bind_trellis_sites.py
```

The second command checks the checksums of the database and of the previously pinned sources. A mismatch stops generation. The rules are currently intended only for `LFE5U-85F`.

| File | Contents |
|---|---|
| [resource_instances.csv](inventory/LFE5U-85F/resource_instances.csv) | Instances of the supported families, architecture anchors, parent objects, and the corresponding BEL names |
| [site_bindings.csv](inventory/LFE5U-85F/site_bindings.csv) | All 54 721 source records, each one's position in the tile list, source coordinates, target object, relation type, and rule |
| [clock_ports.csv](inventory/LFE5U-85F/clock_ports.csv) | 938 ports of the described clock resources, with direction and Trellis wire name |
| [resource_ports.csv](inventory/LFE5U-85F/resource_ports.csv) | 96 474 ports of the described clock and system resources, EBR, MULT18, and ALU54; a flag for whether the wire name exists in the database |
| [config_field_bindings.csv](inventory/LFE5U-85F/config_field_bindings.csv) | 21 987 field fragments by instance, with owners and sources |
| [config_field_bits.csv](inventory/LFE5U-85F/config_field_bits.csv) | 65 634 field-to-bit references: local coordinates and CRAM; shared bits are kept for all owners |
| [decode_catalog.json](inventory/LFE5U-85F/decode_catalog.json) | Executable catalog of CRAM requirements for composite fields; the reading rules are in the [decoder description](CONFIG_DECODER.md) |
| [config_field_groups.csv](inventory/LFE5U-85F/config_field_groups.csv) | 14 417 composite fields, their fragments, defaults, and pattern matches |
| [config_word_layout.csv](inventory/LFE5U-85F/config_word_layout.csv) | Logical bit positions of words, CRAM constraints, and the explicit empty places of fragments |
| [unmodelled_resource_evidence.json](inventory/LFE5U-85F/unmodelled_resource_evidence.json) | Information and sources for service sites and additional DSP modes |
| [routing_interpretation_issues.csv](inventory/LFE5U-85F/routing_interpretation_issues.csv) | Eight fixed ECLKSYNC bypasses that need a mode-aware interpretation |
| [binding_report.json](inventory/LFE5U-85F/binding_report.json) | Counts, check results, remaining gaps, and the checksums of additional sources |

The catalog does not yet contain all internal nodes and all resource families. `native_bel=True` means that a matching constructor call exists in Trellis, not that the full `RoutingGraph` was exported or independently checked. The `native_name` field keeps the Trellis spelling, for example `SLICEA` for our identifier `…/SLICE_A`.

Checked on October 22, 2026: all source records are kept with unchanged content, IDs are unique, targets and parents exist, and all package pins are bound. With the installed Trellis library, 96 282 ports of 502 objects match exactly; the previously exported DCC/DCS ports are not part of this check. All 96 474 wire names occur in the applicable database descriptions. Fifteen portable catalog checks pass. The repeatability of generation and the refusal on a stale checksum are checked separately.

## Record statuses

| Relation | Records | Meaning |
|---|---:|---|
| `instance` | 47 372 | Binding to a specific architecture instance by the family rules |
| `instance_candidate` | 120 | 112 references of empty peripheral IO fragments and eight source ECLKSYNC indexes; need confirmation |
| `group_member` | 5 733 | Membership in a DSP group is established; the mapping to internal lanes and registers is not defined yet |
| `unresolved` | 1 496 | The source record is kept; a binding rule has not been added yet |

The numbers describe records, not independent blocks and not the percentage of emulator readiness. The statuses reflect the matching of architecture sources; there are no hardware measurements at this step.

## PIO and IOLOGIC

The `Chip.cpp` rules create 365 PIOs. Their full set `(row, col, A/B/C/D)` matched the 365 unique `pio_metadata` records, and all 947 pin records of the four packages resolve to these instances. For the side PIOs, 224 IOLOGICs are created; for the top and bottom ones, 141 SIOLOGICs. The sites of the top and bottom IO are called `IOLOGIC` in tilegrid, so the type is taken from the constructor, not only from the site label.

The additional [IOLOGIC analysis](IOLOGIC_RESOURCES.md) checks all 13 269 ports of these 730 objects against libtrellis and binds 9 660 IOLOGIC/SIOLOGIC fields and 1 316 PIO mux fields. No named fields were found for SPICB0/SIOLOGICA; 56 IOLOGIC references in empty fragments remain candidates. The [IO path map](IO_ROUTING.md) adds 603 route bits, including the SPICB0 delay selection, and keeps the group/bank membership of the dedicated DQS/ECLK networks. The data are in the research catalogs, without extending the working decoder.

Repeated references are in the configuration fragments of one group. For example, `PIOA` from `MIB_R11C0:PICL0`, `MIB_R12C0:PICL1`, and `MIB_R13C0:PICL2` belongs to `LFE5U-85F/R11C0/PIOA`. The source positions of these records have rows 27, 8, and −11. They are kept as vendor data and do not set the position of three separate PIOs.

For ordinary side groups, the anchor is PICL0/PICR0, and the next fragments are on rows +1 and +2. For top and bottom groups, the anchor is the fragment with index 0; the fragment with index 1 is in the adjacent column. This split of the PIO and IOLOGIC settings is confirmed by `nextpnr/ecp5/bitstream.cc`, the functions `get_pio_tile` and `get_pic_tile`.

112 references from the empty `MIB_CIB_LRC`, `MIB_CIB_LRC_A`, `MIB_CIB_LX`, `MIB_CIB_RX` are mapped to the adjacent group presumably. The match of the PIO set with iodb does not prove the membership of each such reference, so they have the separate status `instance_candidate`.

## EBR

All 572 EBR references are reduced to 208 instances. The anchor is determined by the tile row, the reference column, and the EBR index; each target must exist among the instances created by `Chip.cpp`.

The difference between the 212 source triples and the 208 blocks arises in two central groups on rows 22 and 70: the EBR3 references from `EBR_CMUX_UL/LL` and the EBR0 references from `EBR_CMUX_UR/LR` have different source rows than the references of the adjacent fragments. For example, `EBR0` from `MIB_R22C69:EBR_CMUX_UR` has source row −416, and from `MIB_R22C70:MIB_EBR1` it has 40. Both records are bound to `LFE5U-85F/R22C69/EBR0`. The coordinates are neither corrected nor lost.

The known EBR configuration fields and the positions of the WID bits are now linked to instances. The first whole-block decoder and the link to the separate INIT images are implemented and checked on a composite example; details are in the [memory description](MEMORY_DSP_RESOURCES.md).

## PLL

Eight PLL references, forming six different source triples, belong to four instances:

| PLL | BEL anchor | Referencing tile types |
|---|---|---|
| UL | R4C1 | PLL0_UL, PLL1_UL |
| UR | R4C125 | PLL0_UR, PLL1_UR |
| LL | R94C2 | PLL0_LL, BANKREF8 |
| LR | R94C124 | PLL0_LR, BANKREF4 |

The anchor position is taken from the offsets in `Chip.cpp`. For the upper PLLs, the source positions of PLL0 and PLL1 differ, although they are parts of one resource. BANKREF remains a separate configuration tile and can contain settings of other resources.

## DCC and DCS

The map has 60 DCCs: 56 at MID and four at the central muxes. All 88 source DCC references are bound to them. The two DCSs also have independent instances.

| Central site | Name by wires | Quadrant | Input | Output |
|---|---|---|---|---|
| DCC0 in CMUX_LL_0 | DCCBL | LL | G_JCLKI_DCCBL | G_CLKO_DCCBL |
| DCC1 in CMUX_LR_0 | DCCBR | LR | G_JCLKI_DCCBR | G_CLKO_DCCBR |
| DCC2 in CMUX_UL_0 | DCCTL | UL | G_JCLKI_DCCTL | G_CLKO_DCCTL |
| DCC3 in CMUX_UR_0 | DCCTR | UR | G_JCLKI_DCCTR | G_CLKO_DCCTR |

The central DCCs are bound to the architecture position R46C67 from the `023-cmux` job for 85K. Their CE and CLKI come from the fabric through the fixed connections in `EBR_CMUX_UL/bits.db`; the outputs are tied to `G_LLCPCLKCIB0`, `G_LRCPCLKCIB0`, `G_ULCPCLKCIB0`, `G_URCPCLKCIB0`. The input is called `G_JCLKI_…`, whereas the ordinary MID constructor uses `G_CLKI_…`. Its interface cannot be copied without this correction.

The current `get_routing_graph_ecp5` creates only the 56 MID DCCs. The four central instances were added to the catalog based on the sites, the wires, the CMUX research, and the description of the global network. The full model cannot be limited to the list of BELs that Trellis creates. The decoding and the dynamic behavior of the central DCCs have not been checked yet.

## DSP groups and internal state

39 groups of nine columns each were reconstructed, with 78 structural DSP slices inside them. Each group contains 18 configuration tiles: MIB_DSP0–8 and MIB2_DSP0–8. The constructor creates four MULT18s at offsets 0, 1, 4, 5 and two ALU54s at offsets 3, 7. All 1 638 MULT18 references and 819 ALU54 references are resolved into 156 and 78 instances, respectively. The settings of these primitives and of the shared slices are linked to specific objects; shared control bits are preserved.

The Lattice manual describes two DSP slices per block; each contains two pre-adders, two multipliers, and an ALU. The pre-adders and multipliers support 9/18-bit modes, the ALU 24/54-bit. The output and framing registers use a shared 72-bit storage whose assignment depends on the mode. Therefore independent states of the "MULT registers" and the "ALU registers" may wrongly double one physical memory. Source: [FPGA-TN-02205-1.3, pp. 6–9](https://www.latticesemi.com/view_document?document_id=50469).

The documented interfaces, modes, and questions about the shared state are collected in [DSP_INTERNALS.md](DSP_INTERNALS.md). The comparison revealed 1 326 groups of identical enum encodings and 702 overlaps of register field bits. This information establishes the commonality of configuration control; the physical owners of the state still need to be clarified.

`PRADD9`, `PRADD18`, `MULT9`, and `ALU24` are so far linked only to the group that contains them. A pre-adder cannot be considered simply a second name for a multiplier: it is a separate node inside the common structure. The number of independently working 9-bit lanes also cannot be counted from the source labels. In the [DSP wire map](DSP_ROUTING.md), all 5 733 references of these families are matched to 546 namespaces with candidate status. The source vendor coordinates, 515 matched port names, incomplete ROC outputs, and two OP mismatches are preserved. The physical binding of both small lanes and of the state owners is still to come.

## What remains

The main discrepancies in PIO, EBR, PLL, and DCC are explained. Edge clocks, 12 DLLDELDs, four DDRDLLs, 14 DQSBUFMs, and six system instances were added. Of the 1 496 records not yet bound, 1 463 are CIBTEST; the rest belong to service and test functions. `unresolved` marks the remaining matching work and does not mean that the resource or data about it is absent.

The first decoder of the effective EBR/DSP configuration and the binding of memory initialization work. Next are needed the owners of internal state and the extension of the structure and decoding to the other families. For the service families, the available evidence is collected; their gaps and the IO/ECLKSYNC candidates remain open. Implementing the behavior needs a separate check; the current computational prototype was not changed here.
