# LFE5U 85F passport

The initial goal of the project is a structural and functional model of the whole `LFE5U-85F`, including the internal nodes of resources, all connections, and unused areas. This passport records the available architecture data and the open questions before the model is designed.

The inventory date is October 22, 2026. The full plan is in [PLAN.md](../PLAN.md); the state of the families is in the [coverage matrix](RESOURCE_COVERAGE.md).

The first collection of materials for all families is summarized in [RESEARCH_STATUS.md](RESEARCH_STATUS.md). The latest additions are [global clocks/PLL/OSC](CLOCK_NETWORK.md), [startup and system blocks](SYSTEM_STARTUP.md), [PIO/banks](PIO_BANKS.md), and [full pin lists of the four packages/BSDL](PACKAGE_JTAG.md). They do not mean that the internal structure, the decoder, and the behavior of all resources are complete. 78 open topics and 37 descriptions of future measurement series are kept; there are no hardware results yet.

## Device and source data

| Parameter | Value |
|---|---|
| Device from the available configuration | LFE5U-85F |
| User's board | ULX3S, reported on October 23, 2026; revision not specified |
| Database IDCODE | 0x41113043 |
| Tile coordinates | Rows 0–95, columns 0–126 |
| Configuration memory | 13 294 frames of 1 136 bits |
| Tiles | 14 231 instances, 148 types |
| Package of the existing example | CABGA381, 205 user PIOs in iodb |
| Other packages in the database | CABGA554 — 259 PIOs, CABGA756 — 365 PIOs, CSFBGA285 — 118 PIOs |
| SERDES DCU | Absent in the U variant; UM and UM5G need a separate passport |

The PIO counts in the table refer to user signals from `iodb.json`, not to all package pins. The available board is an ULX3S; the board revision and the marking of the physical unit have not been checked yet.

Source database: `~/FPGA/prjtrellis/database`. After the [toolchain update](TOOLCHAIN.md), the database revision is `9754ff473bfaa1e6edfaf57ce13bd447c76a88f6` and the Trellis revision is `a31ff7042e712c0e2ad217054116ce2c4995f5c7`. The repeated inventory gave the same data for all LFE5U-85F types; only the revisions in the metadata changed. The rules for creating functional elements are in `libtrellis/src/Chip.cpp` and `libtrellis/src/Bels.cpp`; they did not change. The checksums of the files read are kept in the inventory. The previous snapshot 015e033/73bd411 and the scope of the repeated checks are described in the update report.

## Data catalog

| Catalog | Contents |
|---|---|
| [device_inventory.json](inventory/LFE5U-85F/device_inventory.json) | All 148 tile types with sources, bit groups, settings, defaults, mux variants, and fixed connections; information about the device, packages, and global networks |
| [tile_instances.csv](inventory/LFE5U-85F/tile_instances.csv) | All 14 231 tiles with coordinates and configuration memory windows |
| [tile_types.csv](inventory/LFE5U-85F/tile_types.csv) | Summary of types, families, and instance counts |
| [site_references.csv](inventory/LFE5U-85F/site_references.csv) | All 54 721 source site records, with the tile ownership and the vendor coordinates preserved |
| [settings.csv](inventory/LFE5U-85F/settings.csv) | All 3 031 enum settings with variants and 243 configuration words |
| [package_pins.csv](inventory/LFE5U-85F/package_pins.csv) | Pins of all four packages, links to PIOs, banks, DQS, and special assignments |
| [resource_instances.csv](inventory/LFE5U-85F/resource_instances.csv) | Architecture instances of the main families and DSP groups; an incomplete catalog of internal nodes |
| [site_bindings.csv](inventory/LFE5U-85F/site_bindings.csv) | All source references with the status of their binding to an instance or a group |
| [binding_report.json](inventory/LFE5U-85F/binding_report.json) | Binding results, integrity checks, and the remaining gaps |
| [resource_ports.csv](inventory/LFE5U-85F/resource_ports.csv) | 96 474 ports of EBR, MULT18, ALU54, and the described clock and system resources; other families are still to come |
| [config_field_bindings.csv](inventory/LFE5U-85F/config_field_bindings.csv) | Owners of 21 987 configuration field fragments, by instance |
| [config_field_bits.csv](inventory/LFE5U-85F/config_field_bits.csv) | Local coordinates and CRAM for the bits of these fields; shared bits are preserved |
| [decode_catalog.json](inventory/LFE5U-85F/decode_catalog.json) | Catalog of CRAM constraints for the working decoder of 14 417 composite fields |
| [config_word_layout.csv](inventory/LFE5U-85F/config_word_layout.csv) | Positions of the logical bits of words, CRAM constraints, and the empty parts of fragments |
| [unmodelled_resource_evidence.json](inventory/LFE5U-85F/unmodelled_resource_evidence.json) | Available information on service and additional DSP sites, with sources |

The catalogs are reproduced by this command:

```sh
python3 tools/inventory_trellis.py --verify-pytrellis
python3 tools/bind_trellis_sites.py
```

For another installation, use `--database PATH` and the `ECP5_TRELLIS_LIB` variable. The inventory does not modify Trellis or the computational core of the project.

## Size of the configuration description

| Entity | Count |
|---|---:|
| Mux descriptions in unique tile types | 3 669 |
| Source variants of these muxes | 58 595 |
| Enum settings | 3 031 |
| Enum setting variants | 11 297 |
| Configuration words | 243 |
| Fixed connection descriptions | 10 692 |
| Site records in tiles | 54 721 |

The mux and connection descriptions in this table are counted per tile type. After the types are placed across the die, they repeat. Some descriptions contain device-density prefixes and relative boundary coordinates; their applicability is decided when the full graph is reconstructed.

The catalog of all 148 types was checked against the API of the installed `pytrellis`: mux names and sources, ends of fixed connections, word names and widths, defaults, enum names and variants. This check confirms the extraction of the catalog. Checking block behavior and the semantics of overlapping bit groups belongs to later stages.

## Tile families

| Family | Types | Instances | Role in the passport |
|---|---:|---:|---|
| Logic fabric | 1 | 10 455 | PLC2 |
| Interconnect | 29 | 1 323 | CIB, MIB_CIB and VCIB variants |
| DSP | 18 | 702 | MIB_DSP and MIB2_DSP fragments |
| EBR | 9 | 452 | MIB_EBR fragments; additional EBRs are in mixed tiles |
| Clock and mixed tiles | 26 | 26 | MID, CMUX, and SPINE, including those combined with EBR |
| Global network distribution | 2 | 564 | TAP_DRIVE and TAP_DRIVE_CIB |
| IO | 21 | 395 | PIC, PIO, and variants by die side |
| IO and system configuration | 4 | 4 | EFB0–EFB3 with the bottom PIOs |
| IO banks | 8 | 8 | BANKREF |
| Edge clocks and DDR DLL | 6 | 6 | ECLK and DDRDLL |
| PLL configuration | 6 | 6 | Four PLLs with split configuration tiles |
| Reserved tiles | 13 | 285 | DUMMY_TILE |
| System, test, and special tiles | 5 | 5 | DTR, POR, OSC, PVT_COUNT2, VIQ_BUF |

This is a classification of configuration tiles. It preserves every type and does not determine the number of independently used functional blocks.

## Main functional resources

| Resource | Count | Basis |
|---|---:|---|
| PLC2 | 10 455 | tilegrid |
| Slices | 41 820 | Four per PLC2 |
| LUT4 | 83 640 | Two per slice |
| Logic fabric FFs | 83 640 | Two per slice; DSP, IO, and memory registers are counted separately |
| 18 Kbit EBRs | 208 | The EBR creation rules in Chip.cpp, including the mixed CMUX tiles |
| 18×18 multipliers | 156 | The MULT18 creation rules in Chip.cpp |
| ALU54 | 78 | The ALU54 creation rules in Chip.cpp |
| Structural DSP slices | 78 | A grouping of two MULT18s and an ALU54; adds no arithmetic capacity |
| PLL | 4 | Four PLL0 anchors, one per quadrant |
| DDR DLL | 4 | Four corner DDRDLLs |
| Physical PIOs by IO metadata | 365 | Unique pio_metadata records |

The table lists the main resources and does not exhaust the internal nodes, the additional DSP modes, the clock elements, and the service sites. Their relation to the shared hardware resources must be described in the structural model.

## Differences between a site and a physical instance

The source map has repeated references to sites from several tiles. Even merging by the triple `site_name`, `pos_row`, `pos_col` does not give the correct count of all resources: it yields 813 PIO records instead of the 365 PIOs in `pio_metadata`, 212 EBR records instead of 208 blocks by the element creation rules, and six PLL records instead of four PLLs.

Therefore the source site records are stored separately from instances. The vendor site coordinates and the tile coordinates are stored separately. The main discrepancies are now explained in the [binding map](RESOURCE_BINDINGS.md): repeated PIO fragments, different source positions of the central EBRs, and split PLL tiles. The set of 365 PIOs matched iodb completely; 112 references from empty peripheral fragments remain candidates. The other families and the full structure of internal nodes need further work.

The DSP map additionally contains the labels `PRADD18`, `PRADD9`, `MULT9`, and `ALU24`. They are needed to reconstruct the full internal structure and the alternative modes. They cannot simply be added to the count of independent MULT18s and ALU54s. The pre-adders are real internal nodes; the width variants use a common structure. In the current Trellis BEL constructor these families are not created as separate primitives, although tilegrid has references to them. The references are so far mapped to the 39 groups that contain them; the sources and limits are given in the [DSP section of the binding map](RESOURCE_BINDINGS.md#dsp-groups-and-internal-state).

## Logic fabric modes

The current database distinguishes the slices:

| Slice | MODE variants in PLC2 |
|---|---|
| A | LOGIC, CCU2, DPRAM |
| B | LOGIC, CCU2, DPRAM |
| C | LOGIC, CCU2, RAMW |
| D | LOGIC, CCU2 |

The RAMW mode of C serves distributed memory. Therefore the general statement "all slices support RAM" is not sufficient for the structural model. Inside each slice the model also needs input muxes, output muxes, FFs with data selection and control, carries, and LUT extension modes.

The [PLC/LUTRAM analysis](PLC_LUTRAM.md) adds the interfaces of whole and split slices, checked on all 10 455 PLCs, 99 patterns of named fields, and 128 INIT positions. Bit F50B11 jointly sets DPRAM A/B and RAMW C; the RAMW address/data paths, the shared CLK1/LSR1, and 100 overlaps of configuration positions were collected. A permutation of address bits 0↔3 in the memory packing was established. The nine open questions L01–L09 and the disagreement about the number of PDPR slices are kept for further research and for the board. Errors of open software models were checked separately and are not treated as chip properties.

The `CCU2.INJECT1` bits overlap the bits that select the output muxes. In CCU2 mode the interpretation of the connections changes. Trellis's `tools/ecp_vlog.py` already accounts for this exception, and it must become part of the model's decoding.

## Memory and DSP

EBR has the modes `DP16KD`, `PDPW16KD`, and disabled. The configuration of one block is spread over several tiles; initialization data are passed as a separate part of the configuration. The [EBR addressing analysis](EBR_ADDRESSING.md) links six widths to the shared 2 048×9 array, the address pins, and the nine-bit write enables. Read/write modes, CSDECODE, INITVAL, and all conversions of the public PDP were collected. Six Yosys probes checked 12 288 INIT words; no hardware check was done. The difference in the order of the PDP36 halves, control priorities, partial write, and collisions are kept in B01–B08.

For all 208 EBRs, 7 384 field fragments and 24 128 ports are bound. Their settings occupy two or three adjacent tiles; for the 52 EBR3s the WID word is split between two tiles. A per-bit map with the empty positions of fragments was added. The link between WID and a separate INIT image is implemented and checked, including the 5+4 split and nine-bit data. The `decode` command and its limits are given in the [decoder description](CONFIG_DECODER.md).

[EBR control](EBR_CONTROL.md) extends the collection with 12 rules with their limits of applicability and seven synthesis schemes. For 896 control combinations the circuits up to the memory inputs were checked; the EBR blackbox was not executed in the process. For B01–B08, 12 series of future measurements are prepared. Disagreements about EBR and other families are checked on the available board; the general order and the queue are in [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md).

DSP has settings for input, intermediate, output, and control registers, clock selection, CE/RST, operand sources, arithmetic operations, and comparison patterns. MIB_DSP and MIB2_DSP together describe these nodes. The default of a single fragment does not determine the effective mode of the whole DSP. The owners of shared bits and the compatibility constraints need explicit reconstruction.

Separate descriptions are needed for pre-adders, the alternative small arithmetic, accumulators, cascades, and all internal states. The nextpnr list of primitives is useful for matching ports, but it does not replace this structure.

The settings of 156 MULT18s, 78 ALU54s, and the shared DSP slices are now linked to instances. The summary keeps the different defaults of the fragments and the shared control bits. All new interfaces were checked against libtrellis; together with the clock and system resources, the check covers 96 282 ports of 502 objects. Details and limits are in the [EBR/DSP description](MEMORY_DSP_RESOURCES.md). The additional [DSP analysis](DSP_INTERNALS.md) contains the confirmed register boundaries of ALU54B and the shared control encodings. The [DSP wire map](DSP_ROUTING.md) adds a mapping of 515 port names, candidate namespaces, the pre-adder paths, and 156 control bits; mode overlaps and chain boundaries are kept as open questions.

## Clocking and IO

The global network contains the MID mux, DCC, the central muxes and DCS, SPINE, TAP_DRIVE, and local branches. The edge network includes CLKDIVF, ECLKSYNCB, ECLKBUF, ECLKBRIDGECS, and bridge synchronization. DDR needs DLLDELD, DDRDLL, and DQSBUFM with pointers and control states.

PIO, IOLOGIC, and SIOLOGIC differ by die side and available modes. PICL0 has ordinary input and output registers, IDDR/ODDR, the 1:4 and 7:1 modes, and the MIDDRX, MODDRX, and MTDDRX groups. The PIO and IOLOGIC settings are in different and partly shared tiles. The package selects which of the 365 PIOs are brought out.

The [IOLOGIC/SIOLOGIC analysis](IOLOGIC_RESOURCES.md) adds a check of all 11 444 ports of these blocks and 1 825 PIO ports, the binding of settings, and a matrix of modes by instance. The side A/C blocks have 42 ports, B/D have 35, and SIOLOGIC has 20. 7:1 needs the registers of both pins of the pair; 896 CRAM positions have shared owners. One special SIOLOGIC at WRITEN has no named settings in the database. The internal network, the DDR/FIFO phases, and the contradiction between the ALIGNWD descriptions remain open.

The [IO path map](IO_ROUTING.md) adds 365 delay switches, 1 460 output boundary paths, and 238 edge clock selections. All 603 control bits lie outside the named enum/word fields. The shared lines of both sides of the 7:1 pair are traced. For the dedicated DQS/ECLK, the local wires and the membership relations to a group/bank are preserved; the physical distribution among the local segments has not been reconstructed yet.

[DQS/DDRDLL control](DQS_CONTROL.md) extends the collection with a signal contract, FIFO requirements, 52 DDRDEL paths, and 34 new select bits. The disagreements about delay ranges and defaults, the additional DIVOSC outputs, the partial MODE masks, and the differences between the soft IP MEM_SYNC and the hardware resources are kept. Questions Q01–Q08 need clarification of dynamics and calibration.

For DCC, all 60 instances are explained: 56 MID and four central. The latter are absent from the BEL creation calls in Chip.cpp but are represented by sites, wires, and the CMUX research. The two DCS and their ports are also mapped; details are in the [binding map](RESOURCE_BINDINGS.md#dcc-and-dcs).

Instances of ECLK, 12 DLLDELDs, four DDRDLLs, and 14 DQSBUFMs were added with their interfaces. The four upper DLLDELDs are also absent from the chip constructor calls. All 784 ports of the new clock and system objects were checked against the constructors of the installed library. The eight source ECLKSYNC site indexes are so far mapped to banks presumably; their settings use explicit instance names. Eight fixed ECLKSYNC bypasses were found that need a separate interpretation for the dynamic model. Details are in the [description of edge clocks and system resources](CLOCK_SYSTEM_RESOURCES.md).

## System and unknown resources

The map includes GSR, OSC, JTAG, SED, CCLK/USRMCLK, DTR, and START. References to CIBTEST, CLKTEST, BREFTEST, PVTTEST, BCINRD, and BCLVDSO are kept. Their purpose, interfaces, and completeness of description have separate statuses in the matrix.

In this revision the `bits.db` files of types OSC, POR, and PVT_COUNT2 contain no known descriptions of settings or connections. Some related settings are in other tiles: for example, `OSC.MODE`, `OSC.DIV`, and `SYSCONFIG.*` are in EFB tiles. An empty database for a specific type does not mean the physical resource is absent. POR and PVT need additional sources and research.

GSR, JTAGG, OSCG, SEDGA, DTR, and USRMCLK are now represented by instances with ports and known configuration fields. In total, 251 fields are bound for the clock and system resources. One CRAM bit, F508B1135, belongs to both OSC.DIV and CCLK.MODE; the link between the fields is kept, and the effective interpretation has not been established yet. START and the factory test sites stay in the source catalog without an invented interface.

For the service families, a search by exact names was run in the database types in use. CIBTEST shows 64 wire names and fixed connections; its executable function is unknown. For CLKTEST/BREFTEST/PVTTEST/START/BCINRD/BCLVDSO, no separate interfaces under these names were found. For PRADD9/18, MULT9, and ALU24, hundreds of wire names and connections are kept, although no separate configuration prefixes were found. The full report contains the sources and does not treat the lack of a name match as evidence that the hardware is absent.

## Limits of the existing prototype

The prototype contains LUT/CCU2/FF, part of the PIO, and running simple circuits. It merges active connections into electrical nets and does not store the full directed graph of all switches as independent observable objects. The behavior of most hard blocks is absent, and the common virtual-time event queue has not been designed yet.

As of October 22, 2026, 18 Rust tests, 15 portable Python checks, and three separately run local integration tests pass. `tests/whole_chip.rs` checks a counter and the existing bitstream; `tests/resource_decode.rs` checks identical decoding of CRAM, EBR/DSP settings, and INIT from `.config` and the matching `.bit`. These checks do not close the structural or functional coverage of the whole chip.

## Open tasks of the first stage

| ID | Task | Result |
|---|---|---|
| R01 | Check the marking of the physical chip and package | An unambiguous device and board profile |
| R02 | Finish binding the service sites and check the IO/ECLKSYNC candidates | 120 candidates and 1 496 unbound records remain; the main clock and system instances have been added |
| R03 | Reconstruct the internal DSP structure and the shared use of sites | 39 groups and 78 slices were identified; the ALU54B register boundaries and the shared encodings were collected. D01–D10 in DSP_INTERNALS.md and DSP_ROUTING.md record the remaining questions about lanes, state owners, route conditions, and chain boundaries |
| R04 | Reconstruct the effective settings from distributed fragments | 14 417 composite fields from 21 987 fragments are decoded; ambiguities, shared bits, and provenance are kept; INIT binding works. Full coverage of the remaining families and the internal-state context are still ahead |
| R05 | Check the IOLOGIC interfaces and variants by side and bank | All ports are checked; fields, boundary paths, pairs, and dedicated connections are collected. I01–I10 in IOLOGIC_RESOURCES.md and IO_ROUTING.md record the internal stages, DDR/FIFO, ALIGNWD, SPICB0, and the DQS/ECLK distribution |
| R06 | Clarify START, POR, the configuration controller, and the factory functions using additional sources | Six user system interfaces are described; for the rest, a report on the available signals, settings, and gaps has been collected |
| R07 | Compare the Lattice manuals with the available reference models | The source of behavior and the way to check each mode |
| R08 | Refine the structural model design and check parts of the prototype | Initial decisions are recorded in STRUCTURAL_MODEL.md; the structural and behavioral checks are still to come |

The full catalog of local data has been extracted and checked. The main instances have been bound, and the first design of the structural model is written. The behavior passport and the binding of the remaining resources and internal nodes are still in progress; stage 1 is not closed yet.

## Main sources

- [Trellis libtrellis](https://prjtrellis.readthedocs.io/en/latest/libtrellis/overview.html) — configuration memory, settings, and the resource graph.
- [Trellis Tiles](https://prjtrellis.readthedocs.io/en/latest/architecture/tiles.html) — the split of functions and settings among tiles; specific modes are checked against the current database.
- [Trellis Global Routing](https://prjtrellis.readthedocs.io/en/latest/architecture/global_routing.html) — the structure of the global clock networks.
- [Trellis Text Config](https://prjtrellis.readthedocs.io/en/latest/libtrellis/textconfig.html) — the format and split settings.
- [Lattice ECP5](https://www.latticesemi.com/ecp5) — the family and the manuals on memory, DSP, IO, PLL/DLL, and configuration.
- [Yosys ECP5 cells](https://github.com/YosysHQ/yosys/tree/main/techlibs/lattice) — interfaces and the available primitive models; blackbox declarations are not behavioral models.
