# PLC structure and distributed memory

After this research pass, the [first executable LUTRAM/RAMW](LUTRAM_EXECUTION.md) is implemented: shared LUT/INIT cells, writing on both edges, WREMUX, all addresses and an end-to-end `.bit`/`.config` check. The DI function of FF C in RAMW remains explicitly unknown. The rest of the text keeps the results of the collection; L01–L09 and the hardware series are not closed by software tests.

Addition from the system collection: [SYSTEM_STARTUP.md](SYSTEM_STARTUP.md) keeps the errata PB1384 on the effect of distributed RAM on SED CRC and the unknown applicability of the fix to the physical revision. This is a separate link between the LUTRAM state and the configuration scanner, question Y04/HY03.

For all 10 455 PLC2 of the LFE5U-85F, the slice interfaces, LUT/FF and RAMW, the configuration templates and the shared control bits are mapped. Both representations of each PLC were cross-checked with the installed libtrellis: whole SLICEs and separate COMB/FF/RAMW. LUTRAM addressing and INIT were collected, and synthesis probes and open software models were run. The executable kernel of the emulator was not extended by this collection; there are no hardware measurements yet.

## One PLC and several representations

| Structure | Per PLC | Whole device |
|---|---:|---:|
| Slices A/B/C/D | 4 | 41 820 |
| LUT4 | 8 | 83 640 |
| Fabric FFs | 8 | 83 640 |
| Native COMB | 8 | They represent the same LUTs, carries and extension muxes |
| Native RAMW | 1 | An alternative function of the C combinational nodes |

In `split_slice_mode`, Trellis creates eight TRELLIS_COMB, eight TRELLIS_FF and one TRELLIS_RAMW per PLC. The whole SLICEA…D are another interface to the same hardware. The number of native descriptions does not add up to the physical capacity of LUT/FF, and RAMW does not add separate memory to the LUTs.

| Native interface | Ports per object |
|---|---:|
| A/B: K0 and K1 | 17 and 18 |
| C/D: K0 and K1 | 10 and 11 |
| Each FF | 6 |
| C.RAMW | 16 |
| Whole slices A/B/C/D | 33 / 33 / 33 / 25 |

For a PLC, 176 ports of the separate representation and 124 ports of the whole representation were obtained. Across all positions, 3 136 500 interface records were cross-checked; they matched a single template. This is a cross-check of BEL constructors, names, directions and local wires. It does not confirm executable behavior or all permitted combinations of modes.

Port records of internal nodes are counted here, including shared wires and alternative representations. This is not the number of external PFU inputs/outputs from the block diagram in the datasheet.

The data keeps 300 template ports, 21 descriptions of alternative nodes, all PLC instances and the existing slice IDs. The configuration templates obtain global coordinates through the `start_frame/start_bit` of each instance. Thanks to this, identical tables are not duplicated for each of the ten thousand PLCs.

## Modes A/B/C are linked

| Slice | Database MODE | Function when memory is used |
|---|---|---|
| A | LOGIC, CCU2, DPRAM | Two 16-bit SRAM columns in the LUTs |
| B | LOGIC, CCU2, DPRAM | Another two 16-bit SRAM columns in the LUTs |
| C | LOGIC, CCU2, RAMW | Address and four write-data lines |
| D | LOGIC, CCU2 | Stays outside the RAMW structure; the carry conditions are studied separately |

The DPRAM of slices A/B and the RAMW of slice C have the same local bit, **F50B11**. Therefore these MODEs cannot be treated as three independent switches. For each PLC, the global position of this bit is stored in `instances.csv`.

In total, four MODEs involve nine distinct bits. All 512 combinations of their states are listed, with the matching and the most specific templates. This is an encoding table, not a list of 512 permitted hardware modes: LOGIC has an empty template, and the set shared RAM bit can be combined at the same time with the local bits of CCU2. Such configurations are stored with ambiguity and are not given invented semantics.

According to the `nextpnr::pack_dram` sources, one TRELLIS_DPR16X4 turns into four COMB columns with MODE=DPRAM in A/B, a RAMW in C, and two RAMW_BLOCK markers that occupy the combinational positions of C. The markers help placement; the configuration writer skips them. For C it writes MODE=RAMW and zero INITs of both LUTs.

One structure of this representation stores 64 bits in four LUTs. The data state uses the same LUT/INIT cells. Creating an additional array alongside the independent LUT tables would break the link between the modes and double the state. The configuration bits of C are also kept in this case; the meaning of non-standard nonzero INITs of C in RAMW is not established.

The nextpnr placement check forbids combining RAMW with the ordinary combinational functions of C. The FFs are checked separately: the code does not declare all eight FFs occupied merely because a RAMW is present. The physical availability of their data paths, and of the extension muxes in mixed modes, remains L02.

## Address, data and control of RAMW

`dram_to_ramw_split` uses the LUT input pins of slice C as follows:

| Logical signal | RAMW pin |
|---|---|
| WAD0 / WAD1 / WAD2 / WAD3 | D0 / B0 / C0 / A0 |
| DI0 / DI1 / DI2 / DI3 | C1 / A1 / D1 / B1 |

Four WADO go through fixed connections to all four columns A/B. WDO0/1 go to K0/K1 of slice A, and WDO2/3 go to K0/K1 of slice B. The two LUTs of one slice therefore do not have independent addresses and write enables.

Both WCKs come from **CLK1**, and both WREs come from **LSR1**. `SLICEA.WREMUX` shares bit F55B11 with `LSR1.LSRMUX`; the WREMUX constants additionally use two bits of the LSR1 route. The inversion and the constant selection must be recovered jointly, preserving all the owners of the bits.

A PLC also has CLK0 and LSR0, and a separate CLK/LSR selector for each slice. Besides this structure, nextpnr imposes common constraints on the clocks/resets of the FFs of the whole PLC, and on the FF polarity when LUTRAM is present. A packer constraint does not by itself prove the physical impossibility of another configuration. The differences are checked in L05.

## LUT address and memory INIT

In an ordinary LUT4 the table index equals `A + 2*B + 4*C + 8*D`. This is confirmed by the INIT fuzzer and the LUT4 model. In `dram_to_comb` the logical RAM read pins are connected in a different order:

| Logical address | LUT pin |
|---|---|
| RAD0 | D |
| RAD1 | B |
| RAD2 | C |
| RAD3 | A |

Therefore the logical RAM address `a` corresponds to the physical LUT INIT index:

```text
p = ((a & 1) << 3) | (a & 6) | ((a & 8) >> 3)
```

Bits 0 and 3 swap places. For example, logical address 1 goes to LUT index 8, and address 8 goes to index 1. The packer permutes the INIT accordingly. Bit `4*a + lane` of the TRELLIS_DPR16X4 parameter becomes bit `p` of the INIT word of one of A.K0, A.K1, B.K0, B.K1. All 64 correspondences are given in `RAM_INIT_mapping.csv` with the specific local CRAM constraints.

All 128 positions of the eight INIT words of PLC2 use inverted physical bits. For example, SLICEA.K0.INIT[0] is given by `!F25B10`: a logical one corresponds to a CRAM zero. The raw representation from the bitstream, the logical LUT table and the RAM address representation differ; the conversion is done once, at the corresponding boundary. The exact correspondence of a live write to these cells remains to be confirmed by experiment HL02.

Separately from RAM addressing, Trellis can add 96 pseudo-connections for permuting the LUT inputs per PLC. This is a router capability: `nextpnr::permute_lut` changes the INIT table for the selected pins. In a loaded bitstream the result is already written into the physical LUTs. Additional permanently connected wires must not be created from these pseudo-connections, and INIT must not be permuted again from absent router metadata. In LUTRAM packing, nextpnr forbids such arbitrary permutation.

## LUT extension, carries and overlaps

Each even COMB K0 has an extension output: `OFX = M ? F1 : F`. The odd K1 uses `OFX = M ? FXB : FXA`. Local fixed paths and the selection of the F outputs allow slices to be combined into LUT5/6/7. For D, one of the extension inputs receives `E1_HL7W0001`; the local HL7W0001 is connected to F3. This is a connection to the neighboring PLC for LUT8. The conditions of the output switches and the boundary continuations must be taken into account explicitly.

The native COMBs also describe eight sequential carry stages. Within the pair K0→K1 the shared name FCI…1_SLICE is used; the inter-slice connections A→B→C→D and the continuation into the neighboring column are in the database. In CCU2 mode the interpretation of the outputs differs from the ordinary extension muxes.

All 128 muxes of PLC2 with 2 242 source variants and 124 fixed connections are stored. **100 local positions** described by several fields or routes were found. Among them are the shared MODE A/B/C, WREMUX/LSR1 and eight intersections of CCU2.INJECT1 with the switches F0…F7. Therefore the graph cannot be executed as a union of all formally matching empty templates and fixed representations.

When writing non-CCU2 modes, nextpnr uses the value `_NONE_` for INJECT1 so as not to interfere with the routes of the extension muxes. In libtrellis this means no change to the bit group, not a separate hardware mode or forced zeroing.

## What the software probes confirmed

Six Yosys designs check the memories 16×1, 16×2, 16×4, 16×8, 32×4 and a write on the falling edge. The first three occupy one TRELLIS_DPR16X4 each; those extended in width and depth occupy two each. For 32×4, six LUT4s were additionally obtained. This is a synthesis result; placement and routing of these designs with nextpnr were not performed here.

The published TRELLIS_DPR16X4 has asynchronous read, write on a selected edge and WREMUX WRE/INV/0/1. Eight parameter combinations, all 16 addresses and all four-bit data values with both WRE levels were checked. For these models, 131 200 word samples were cross-checked. Another 16 400 samples checked the legacy DPR16X4C, with the skip of initial address 15 accounted for separately. In total, 147 600 observations of the software models, without a hardware cross-check.

Additional probes revealed limitations of the installed `common_sim.vh`:

- The legacy DPR16X4C executes an initial loop `i < 15`, so address 15 stays X until written; TRELLIS_DPR16X4 initializes all 16 addresses.
- The DPRAM branch of TRELLIS_COMB reads `ram[{A,C,B,D}]`. Without adaptation this does not match the physical INIT order prepared by `dram_to_comb`: with INIT=0x0100 and the stated RAD connection, a one is observed at logical address 8 instead of 1.
- In this branch the declared WREMUX is not used: with WREMUX=0, a raw WRE=1 still writes the data. Preliminary processing of the effective signal or a fix to the model is needed.
- In the CCU2 branch, a local declaration of FCO hides the module output. A probe with an internal expected carry=1 got the value Z on the external FCO.

These are properties of the specific software models, confirmed by Icarus and by the sources. They do not mean a lost address, an ignored WRE or a floating carry in the physical chip. The models must not be used as a reference after packing without checking; the digital contract and the hardware observations remain separate evidence.

The [Memory User Guide](https://www.latticesemi.com/view_document?document_id=50466), pp. 52–53, separately states that the read clock/enable and the output registers of a generated module are implemented by the external fabric. The figure and the table there have inconsistent labels of the primitive and of the widths; they do not replace the actual interfaces. The [datasheet](https://www.latticesemi.com/view_document?document_id=50461), p. 19, specifies six slices for PDPR16×4, whereas the nextpnr path examined places the primitive with separate RAD/WAD in one A/B/C structure. This disagreement is kept as L01, without choosing the hardware truth based on the popularity of a tool.

## Open questions and the next collection

| ID | What remains |
|---|---|
| L01 | Physical structure and independence of the PDPR addresses, disagreement in slice costs |
| L02 | FFs and muxes under RAMW/DPRAM, non-standard mixed modes and INIT of C |
| L03 | Read during write and operation with undefined addresses/enables |
| L04 | Initial state, reset/GSR and retention of the LUTRAM array |
| L05 | Shared CLK1/LSR1 and the boundary between hardware constraints and packer rules |
| L06 | Activity conditions of the extension muxes/carry, ambiguous MODEs and chain boundaries |
| L07 | All 16 initial addresses on the chip; the legacy software error is stored separately |
| L08 | INIT, physical pins and live write without double permutation |
| L09 | Configuration of the level-sensitive register mode mentioned in the datasheet |

The datasheet, p. 16, mentions both edge-triggered and level-sensitive registers. The current PLC2 fields and the examined TRELLIS_FF do not yet establish a binding of the latter mode. The absence of a found field is not considered evidence that the function is absent.

The contract includes six series of future experiments, HL01–HL06, tied to these questions; for L09 a configuration binding is needed first. They are included in the shared [hardware check queue](HARDWARE_VALIDATION.md). The continuation of the collection is [FF reset/CE/PRLD/init, CCU2 and output muxes](FF_CCU2.md): the encodings and the open models were studied separately; the physical activity conditions and startup remain open. These families are not closed yet.

## Data and reproduction

[plc_research](inventory/LFE5U-85F/plc_research/evidence_report.json) stores 18 files: instances and slices, node/port templates, enum and INIT, routes, shared bits/wires, MODE combinations, RAMW/INIT conversions, synthesis and model results, separate observations of model limitations, the contract and the report.

These are research templates. The existing working `decode` catalog of 14 417 fields was not extended by this collection.

```sh
clang++ -std=c++14 -Wall -Wextra -Werror \
  -I ~/FPGA/prjtrellis/libtrellis/include \
  -I /opt/homebrew/include tools/trellis_plc_probe.cpp \
  -L /opt/homebrew/lib/trellis -ltrellis \
  -Wl,-rpath,/opt/homebrew/lib/trellis -o target/trellis_plc_probe
python3 tools/collect_plc_evidence.py
```

Besides the interface cross-check, the names/defaults of the PLC2 catalog and all 2 565 bit-level groups were checked: 128 INIT positions, 195 enum variants and 2 242 mux variants. The templates were checked to fall within the configuration window of each PLC. The report stores the checksums of the sources, models, programs, the loaded libtrellis/pytrellis, the input catalogs and the probes. The first source HDL and traces are in `target/research/plc/probes`. During the [tools update](TOOLCHAIN.md) the collection was repeated with the new Trellis and Yosys 0.70 in `target/toolchain-update/plc-probes`: all substantive tables matched, and the main report was replaced with a new one with current hashes. The previous Yosys 0.66 report is kept in the archive.

A repeated collection with a new probe directory gave 18 byte-identical files. Checked separately: all IDs and parents of the slices, the uniqueness of the shared mode bit across instances, 128 inverted INIT positions, 64 RAM address/data correspondences, all 512 MODE templates, observations of model errors and document links. L01–L09 remain open; a successful software check does not change the status of hardware measurements.
