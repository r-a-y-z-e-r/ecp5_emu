# Data paths and dedicated IO networks

For all 365 IOLOGIC/SIOLOGIC of the LFE5U-85F, the input delay selection, the data outputs, the return path, and the tristate control at the boundary with the PIO were traced. 603 route control bits were found that are absent among the named enum/word settings. The dedicated DQS and ECLK links are supplemented with group and bank membership; the detailed physical chain between their local wires is not recovered in this data.

This analysis supplements the [IOLOGIC interfaces and modes](IOLOGIC_RESOURCES.md). It sets the links and constraints of the future structure. The internal register stages, the delay switching, and the FIFO remain subjects of research.

## Scope and cross-check

| Data | Scope |
|---|---:|
| Used tile types with IO, BNK_ECLK, or DQSG | 34 |
| Source connection records for these types | 2 086 |
| Placed connection records | 16 619 |
| Name and coordinate transformations matching libtrellis | 25 471 |
| Mux variants checked bit by bit through pytrellis | 361 |
| IOLOGIC/SIOLOGIC/PIO ports with a connection of the expected direction | All 13 269 |
| Input switches of the direct signal and the delay | 365 |
| Output boundary paths | 1 460, four per IOLOGIC/SIOLOGIC |

All fixed connections, mux sets, enums, and words of these 34 types were additionally cross-checked against the Trellis API. Normalization accounts for density, relative coordinates, and the rules for global names. All 74 rejected requests belong to the 25K/45K prefixes and are stored separately; they do not concern the selected 85F.

The research graph includes all records of every selected type. Paths are not continued through the internal function of a BEL or through the full general routing network. These numbers describe database records and their interpretation, not the number of independent silicon elements.

## Input and delay selection

Two branches converging at `JDI<letter>` were found for each instance:

| Branch | Boundary path |
|---|---|
| Direct input | PIO.O / JPADDI → JDI → IOLOGIC.DI |
| Delay output | IOLOGIC.INDD → JDI → IOLOGIC.DI |

The path from PIO.O is also connected to PADDI. The internal delay function between the inputs and INDD is not replaced by a connection in this table. Through JDI the signal can also go to the fabric, so the delay selection affects not only the input of the internal register.

All 365 delayed branches have their own one-bit selection pattern. In 130 instances the direct variant and the delayed one are in the same mux; in the remaining 235 the direct variant is recorded as a fixed connection and the delayed one as a mux. The latter 235 include 112 side C/D and 123 SIOLOGIC. In the remaining 18 SIOLOGIC the direct variant is in the mux, as in the side A/B.

For example, for R11C0/IOLOGICA the delayed variant `INDDA_IOLOGIC → JDIA` requires CRAM F8B124=1. The direct variant from JPADDIA has an empty pattern of the same mux. For IOLOGICC the direct variant is located in PICL0, and the delayed one in PICL2, two rows below. Therefore processing the tiles separately and then merging the connections unconditionally loses the meaning of the switch. The empty pattern of the direct variant also cannot be taken as an explicit requirement of zero: the selection must account for the other variants of the mux.

For the special SPICB0/SIOLOGICA, which has no named IOLOGIC settings, the INDD selection through CRAM F1867B1134=1 was found. The source is SPICB0/bits.db:3. This refines question I05: part of the control is described by a route. The initial delay value, the availability of the plain registers, and the remaining control bits of the special WRITEN pin are not established by this.

## Output paths and shared pair lines

Four connections were traced for all 365 instances:

1. IOLDO → intermediate IOLDO → PIO.IOLDO.
2. IOLDOD → the same intermediate IOLDO → PIO.IOLDO.
3. IOLDO → IOLDOI, the return input inside the IOLOGIC interface.
4. IOLTO → PIO.IOLTO, the tristate control.

The first two outputs are recorded as fixed sources of one wire. Their actual use depends on DELAY.OUTDEL and the block mode; they must not become two permanently active drivers. In addition, a PIO DATAMUX/TRIMUX selection between the ordinary fabric inputs and the IOLOGIC is needed. `output_paths.csv` keeps the paths and the references to the configuration fields that belong to them, including the cases of missing fields on SPICB0.

The following wire coincidences were traced between the wide A/C interface and the neighboring B/D:

| Wide interface A or C | Neighbor interface B or D | Link |
|---|---|---|
| TXDATA4 | TXDATA0 | Shared source from the fabric |
| TXDATA5 | TXDATA1 | Shared source from the fabric |
| TXDATA6 | TXDATA2 | Shared source from the fabric |
| RXDATA4 | RXDATA1 | Shared output to the fabric |
| RXDATA5 | RXDATA2 | Shared output to the fabric |
| RXDATA6 | RXDATA3 | Shared output to the fabric |

That is 336 coincidences on the input and 336 on the output across all 112 pairs. For example, R11C1:JA3 feeds both TXDATA4A and TXDATA0B at the same time, and R11C1:JQ1 receives the representations RXDATA4A and RXDATA1B. The different offsets of the input and output indices are kept literally. They confirm the shared lines, but by themselves they do not yet define the sequence of the internal FFs and do not allow the remaining port of the neighbor to be used independently in 7:1 mode.

In total, 701 nodes with several fixed sources from IOLOGIC outputs were found: 365 shared direct/delayed representations and 336 output coincidences of the pair. On the inputs, 504 ordinary DQS fan-outs to four IOLOGIC of one local section are additionally kept. They have a separate classification and are not counted as driver conflicts.

## Edge clock and bank membership

For each of the 224 side IOLOGIC, both local branches BNK_ECLK0/1 were traced to the ECLK port. 224 control bits selecting ECLK1 were found. For the 14 DQSBUF, 14 more bits and 28 branches were found in the same way. In total, 476 input link variants were kept for 238 resources.

In the 112 IOLOGIC C/D, the ECLK1 selection overlays the fixed representation of ECLK0. DQSBUF has such an overlay in all 14 instances. The selection of variant 1 may be located in a neighboring tile: for C/D the BNK_ECLK1 source is located on the anchor row +2, for DQSBUF on row +1. The source coordinates were obtained from the connections, not copied from the coordinates of the consumer.

The bank of each IOLOGIC is taken from iodb; for DQSBUF it was additionally checked that all members of its group belong to one bank. The corresponding ECLKBUF with the output `G_BANK<bank>ECLK<0/1>` was found. The next section, up to the local BNK_ECLK, is represented by a separate bank-membership relation. In nextpnr the special ECLK router stops at BNK_ECLK or G_BANK, which confirms the need to treat this architectural link separately from the ordinary records.

In the examined records, 182 used local BNK_ECLK nodes have no incoming edges. All database records that mention BNK_ECLK are included in the scope of the study. The ECLKBUF output and the consumer are linked by bank, but no additional physical segments between them were invented.

## DQS and pointer distribution

For the 224 side IOLOGIC, nine links to the corresponding DQSBUF are kept: DQSR90, DQSW, DQSW270, RDPNTR0…2, and WRPNTR0…2. That is 2 016 links across 14 groups.

The 56 IOLOGIC at the DQSBUF anchors have 504 two-edge paths from the DQSBUF output through the local DQSG. For the remaining 168 IOLOGIC, 1 512 relations with the group source are kept, but the local DQSG have no incoming edges: these are 378 nodes, nine for each of the 42 remaining local sections.

This result agrees with nextpnr: `pack_dqsbuf` marks nine nets as special, which the ordinary router does not process; `process_dqs_port` checks that the port name and the DQS group are the same. For the digital kernel this membership must be explicit. To map the physical network completely, the distribution segments between the local sections still have to be established.

The pointers are represented by separate wires, not by one common value for all banks. Their transitions, the FIFO, the phases, and the order of simultaneous events remain question I07 of the main IOLOGIC analysis.

## Ambiguities and remaining nodes

The 603 selection bits consist of 365 input delays, 224 IOLOGIC ECLK selections, and 14 DQSBUF ECLK selections. Their positions were checked against all enum/word settings of all used database types: no overlaps were found. Therefore the next configuration model must combine the named fields and the route selections.

In 385 nodes, fixed and programmable inputs are present at the same time: 235 input bypasses, 112 ECLK C/D, 14 DQSECLK, and 12 each of DDRDEL and JINCK from the included clock tiles. The last 24 cases are kept for the later analysis of the DLL and the clock inputs; the activity conditions are not yet established.

Another 560 IO/PIO wire names are not ports of the native Trellis BELs: 224 INRDA…D_PIO, 224 LVDSA…D_PIO, and 112 JSLIPB/D_IOLOGIC. They must not be lost when the full structure is built. In particular, the presence of the name JSLIPB does not create a confirmed standalone SLIP port of resource B. INRDA/LVDS require binding to internal nodes and to the bank control in the PIO study.

Questions I01, I05, I07, and I09 have advanced: the boundary paths, both sides of the shared 7:1 lines, the additional SPICB0 control, and the dedicated group links were found. The internal state and the event sequences are not established by these paths. The new question I10 (the physical distribution of the dedicated DQS/ECLK and the membership of the remaining names in internal nodes) remains open.

## Data and reproduction

15 files are stored in [io_routing_research](inventory/LFE5U-85F/io_routing_research/routing_report.json):

- `connection_templates.csv`, `port_adjacency.csv`, `rejected_name_requests.csv` — source connections, connections of all ports, and rejected density prefixes;
- `input_delay_paths.csv`, `output_paths.csv`, `route_controls.csv` — boundary paths and control bits;
- `shared_input_terminals.csv`, `shared_output_sinks.csv`, `fixed_mux_overlaps.csv` — shared lines and representations that need mode conditions;
- `dqs_links.csv`, `edge_clock_links.csv`, `dedicated_wire_roots.csv` — dedicated links, exact local paths, and sections without an incoming edge;
- `unnamed_primitive_wires.csv`, `selector_named_field_overlaps.csv`, `routing_report.json` — the remaining names, the check of field overlaps, and a summary with the SHA-256 of the sources.

```sh
python3 tools/collect_io_routing.py \
  --native-probe target/trellis_wire_probe --verify-pytrellis
```

The collection uses the existing research `trellis_wire_probe`, whose build instructions are given in the [DSP analysis](DSP_ROUTING.md#reproduction). All 15 files were collected again with a byte-for-byte comparison. The main decoder and the executable models are not changed by this study.

The sources are pinned in the report: the original inventory and the IO catalogs, `RoutingGraph.cpp`, `Bels.cpp`, `nextpnr/ecp5/pack.cc`, the research programs, the verification binary, pytrellis, and the loaded libtrellis. The hashes of the database files are inherited from the original inventory and are checked before the collection. The main ECLK rules are in `make_eclk`, the DQS rules are in `pack_dqsbuf` and `process_dqs_port`; the mode restrictions are discussed in [IOLOGIC_RESOURCES.md](IOLOGIC_RESOURCES.md).
