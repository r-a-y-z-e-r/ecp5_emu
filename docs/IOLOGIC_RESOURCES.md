# IOLOGIC and SIOLOGIC: interfaces, modes, and shared state

For the LFE5U-85F, the interfaces of all 224 side IOLOGIC and 141 top/bottom SIOLOGIC are collected, together with their links to PIO, package pins, and DQS groups. The settings are tied to physical CRAM bits in a separate research catalog. This supplements the [device passport](DEVICE_PASSPORT.md); the exact internal register network and the behavior are still under study.

## Verified scope

| Data | Result |
|---|---:|
| Side IOLOGIC A/C | 112 instances, 42 ports each |
| Side IOLOGIC B/D | 112 instances, 35 ports each |
| Top and bottom SIOLOGIC | 141 instances, 20 ports each |
| Linked PIO | 365 instances, five ports each |
| Ports matching libtrellis | 13 269 on 730 objects; of these, 11 444 on IOLOGIC/SIOLOGIC |
| Named IOLOGIC/SIOLOGIC fields | 9 660 on 364 instances |
| Accompanying data-select and tristate fields in PIO | 1 316 |
| Bit references in total / unique CRAM positions | 17 976 / 13 888 |
| CRAM positions with several owners in this set | 896 |
| Groups of identical full enum encodings | 364 |

Each field of this set lies in a single configuration tile. This does not mean that a whole IOLOGIC lies in one tile: its placement, PIO connections, DQS, internal routes, and mode selection are described by different parts of the database. No polarity contradictions within one value, and no differing defaults between fragments, were found here.

The check calls `add_iologic` and `add_pio` of the installed libtrellis for all cataloged positions. It compares the BEL name, type, port, direction, and the destination wire with its coordinates. The full RoutingGraph, the physical validity of all mode combinations, and the behavior are not confirmed by this check.

## Placement and available modes

| Area | Plain registers and DELAY | DDR 2:1 | DDR 4:1 and memory modes | DDR 7:1 |
|---|---|---|---|---|
| Side A/C | Interface and settings present | Present | Interfaces present | Start of an A/B or C/D pair |
| Side B/D | Interface and settings present | Present | Interfaces present | No standalone seven-bit interface |
| Top/bottom SIOLOGIC | Present; the special SPICB0 is described below | Present | No ECLK and no wide ports | None |

The restrictions by side, and the use of the neighboring pin's registers in 7:1 mode, are described directly in §8.7 of the [High-Speed I/O guide](https://www.latticesemi.com/view_document?document_id=50467), p. 68. They match the Trellis ports and the nextpnr checks: the seven bits and SLIP are created only on A/C; the packer forbids placing IDDR71B/ODDR71B on B/D.

However, `IDDRXN.MODE=IDDR71` and `ODDRXN.MODE=ODDR71` are present in the database for B/D too. Fuzzers 063/064 iterate over these values for all four letters. Such an entry describes the encoding, but does not prove that an independent seven-bit block exists on every pin. The structural model needs 112 possible pair starts with shared DDR storage. The joint use of the input and output banks within a pair requires a separate check.

For the 365 instances, a matrix of 15 mode/primitive groups is stored: plain registers, DELAYF/G, IDDR/ODDR X1/X2/71, and six memory primitives. The `available` label means availability by location, not verified behavior or permission for an arbitrary combination. In particular, nextpnr separately warns that the joint use of IDDRXN and ODDRXN on one pin has no official support.

The configuration names of the top and bottom resources remain `IOLOGICA/B`, although the native BEL is named `SIOLOGICA/B`. For the top A, the settings are in PICT0 on row 1; for B, in PICT1 of the next column. The side C/D take their settings from the anchor row +2. These rules were checked separately against nextpnr's `get_pic_tile`; the coordinates of the vendor's site labels were not used as die coordinates.

## Internal paths the model must preserve

The following table describes the interface roles according to the Trellis constructors and the nextpnr transformations. It does not fix the number of physical FFs or the final scheme of the internal muxes.

| Path | Ports and settings | What is already known |
|---|---|---|
| Plain input register | PADDI → INFF; `FF.INREGMODE`, `FF.REGSET` | FF/LATCH are encoded separately; nextpnr packs a plain FF onto this path |
| Output register | TXDATA0 → IOLDO; `OUTREG.OUTREGMODE`, `OUTREG.REGSET` | The output is selected by the PIO DATAMUX fields; the IOLOGIC mode name alone is not sufficient |
| Tristate register | TSDATA0 → IOLTO; `TSREG.*`, `IOLTOMUX` | Controls the PIO through TRIMUX_TSREG; the output-enable state is stored separately from the data value |
| Generic DDR X1 | PADDI → RXDATA0/1; TXDATA0/1 → IOLDO | Input and output use the IDDRX1_ODDRX1 mode and the primary CLK |
| Generic DDR X2 | RXDATA0…3, TXDATA0…3, CLK and ECLK | Separate IDDRXN.MODE/ODDRXN.MODE set the kind of conversion |
| Generic DDR 7:1 | RXDATA0…6, TXDATA0…6, SLIP | A register pair shared by two pins; the interface is available on A/C |
| Memory input | DQSR90, RDPNTR0…2, WRPNTR0…2; RXDATA0…3 | IDDRX2DQA uses MIDDRX; QWL is packed into INFF, which coincides with the output of the plain input register |
| Memory data/strobe output | TXDATA0…3, DQSW270 or DQSW | ODDRX2DQA/B use MODDRX and WRCLKMUX |
| Memory tristate | TSDATA0/1, DQSW/DQSW270 → IOLTO | TSHX2DQA/SA use MTDDRX and a separate inversion setting |
| Memory address/command | TXDATA0 and TXDATA2 → IOLDO | OSHX2A uses MOSHX2; the sequential D0/D1 do not correspond to TXDATA0/1 |
| Shared controllable delay | DI, PADDI, INDD, IOLDOI, IOLDOD; LOADN/MOVE/DIRECTION/CFLAG | All intermediate ports are kept; the exact switching of the bypasses and return paths still has to be recovered |

Each IOLOGIC/SIOLOGIC has one external set of CLK, CE, and LSR. The fields CLKIMUX/CLKOMUX, CEIMUX/CEOMUX, and LSRIMUX/LSROMUX choose how the shared control is applied to the input and the output. nextpnr has checks for conflicting signals and for CE polarity. SRMODE distinguishes ASYNC and LSR_OVER_CE; REGSET and GSR also cannot be reduced to one common reset of all internal stages.

The database has no separate named enable fields `FF`, `OUTREG`, `TSREG`, `IDDRX1`, `ODDRX1`, although the fuzzers explore such switches. Consequently, the presence of a register configuration does not determine whether its output is used: routes, PIO settings, and clock muxes are also needed. This remains a task for the next structural pass.

For memory, the guide specifies an input FIFO of depth eight with three-bit pointers; one DQSBUF distributes the pointers to its group (§6.2.6, pp. 38–39). According to `iodb.json`, all 224 side IOLOGIC are mapped to 14 DQSBUFM. The designations DQ, DQS, and DQSN are kept separate, but resolve to one corresponding group. The width of the stored words, the write/read phases, and the behavior on coinciding events are not yet established.

The [IO path map](IO_ROUTING.md) supplements these interfaces: all 365 input delay switches, 1 460 output boundary paths, the shared 7:1 transmit/receive lines, and 603 route control bits were found. The dedicated DQS/ECLK links are kept together with their group/bank membership; sections without ordinary incoming edges are marked explicitly.

## Configuration ambiguities

All 364 examined `DELAY.DEL_VALUE` words have seven bits. The delay is a resource shared by the input/output path, not two independent lines. The LOADN/MOVE/DIRECTION/CFLAG control requires its own digital state even with a simplified propagation time.

In the 224 side IOLOGIC, `IOLTOMUX=NONE` and `IOLTOMUX=TS` have identical full patterns. In 140 SIOLOGIC, `MODE=NONE` and `MODE=IREG_OREG` coincide. The decoder must keep these alternatives and recover the path usage from additional conditions. A default is not evidence that allows choosing one of the identically encoded names.

The 896 shared CRAM positions affect the IOLOGICA/B pairs, the IOLOGICC/D pairs, and some DATAMUX fields of PIO. Eight such positions were found in each side pair. For example, F0B132 belongs at the same time to ODDRXN.MODE of R11C0/IOLOGICA and to MODDRX.MODE, MTDDRX.MODE, ODDRXN.MODE of R11C0/IOLOGICB. Decoding each field independently, without checking the combinations, can produce a contradictory configuration. The links with the previously collected EBR/DSP/clock/system set do not intersect by bits; the other PIO and bank fields are not part of this conclusion.

## Gaps and disputed information

**Update 2026-10-29:** I02 is answered for the generic DDR X1 and the IO registers on the left-side IOLOGIC and the top-bank SIOLOGIC by the board slices HI02/HI02c ([hi02_iologic](inventory/LFE5U-85F/hi02_iologic/README.md)): publication edges, latency, start-up values and the asynchronous reset. The emulator executes them as described in [IOLOGIC_EXECUTION.md](IOLOGIC_EXECUTION.md). CE, synchronous reset, GSR and simultaneous edges stay open. On the SIOLOGIC, the identical `MODE=NONE`/`IREG_OREG` bits are resolved by the register path the plan uses.

| ID | Question | Next evidence |
|---|---|---|
| I01 | The full network of FFs, latches, muxes, and bypasses; ownership of the pair's registers in 7:1 | Boundary paths and both sides of the shared 7:1 lines are collected in IO_ROUTING.md; next are the internal stages and their owners |
| I02 | The exact publication of RXDATA, the delay in cycles, resets, GSR/CE, and simultaneous edges | Vendor behavioral models or a hardware check |
| I03 | ALIGNWD is described as a one-bit shift in the tables on p. 69 and as a two-bit rotation in the IP description, p. 81 | Comparison of the primitive, the IP, and the hardware sequence; the contradiction is not yet resolved |
| I04 | ALIGNWD is documented for IDDRX2F, but the pinned nextpnr does not carry it into the IOLOGIC; SLIP exists only on A/C | A separate study of the interface and the setting; the absence of handling by the packer does not mean the absence of the function in silicon |
| I05 | R95C18/SIOLOGICA of SPICB0 has ports but not a single named IOLOGIC field | The input delay select bit F1867B1134 was found in IO_ROUTING.md; the remaining WRITEN and bank 8 control is still unknown |
| I06 | WAIT_FOR_EDGE, reaching the delay boundaries, changes of taps, and propagation of control events | Vendor DELAY model or a hardware check; the exact behavior is not inferred from the names |
| I07 | FIFO and QWL: internal stages, pointers, and the relation to DQSBUFM/DDRDLL | IO_ROUTING.md collects the dedicated links; DQS_CONTROL.md adds the control contract and 224 FIFO requirements. Transitions and pointer encoding remain in Q04–Q06 |
| I08 | 56 IOLOGIC references in empty tile fragments remain candidates | Additional links in the database; the current binding is not promoted to confirmed |
| I09 | Shared bits, the treatment of aliases, and input/output/tristate combinations | IO_ROUTING.md keeps 701 shared output nodes and bypass overlays; next are the configuration conditions and verifiable examples |
| I10 | Physical segments of the dedicated DQS/ECLK and internal wires that are absent among the BEL ports | 378 local DQSG and 182 BNK_ECLK without incoming edges; 560 additional names are kept in IO_ROUTING.md |

The special SIOLOGIC of I05 is tied to the physical PIOA marked as WRITEN; it is brought out in all four packages of the database. For CABGA381 this is T3. Therefore the absence of settings cannot be used as grounds to remove this object from the full model.

The DDR tables confirm the order D0…D3 on transmit and the assignment of the input bits to edges. They do not give enough information for all publication phases and internal stages. In the local Yosys these primitives are declared as blackbox; Diamond is absent. Exact reference models were not obtained at this step.

## Files and reproduction

The data are in [iologic_research](inventory/LFE5U-85F/iologic_research/evidence_report.json):

- `instances.csv`, `resource_ports.csv`, `mode_capabilities.csv` — instances, all ports, pairs, DQS, package pins, and availability by location;
- `config_field_bindings.csv`, `config_field_bits.csv`, `config_field_groups.csv`, `config_catalogue.json` — fields, exact bits, and full value patterns;
- `enum_fragment_aliases.csv`, `shared_config_bits.csv`, `prior_catalogue_shared_bits.csv` — identical encodings and owners of the shared bits;
- `candidate_references.csv`, `evidence_report.json` — unverified references, the summary, the boundaries, and the checksums of the sources.

The research `config_catalogue.json` is not connected to the `decode` command: the existing 14 417 fields of the working decoder were not extended by this collection.

```sh
clang++ -std=c++14 -Wall -Wextra -Werror \
  -I ~/FPGA/prjtrellis/libtrellis/include \
  -I /opt/homebrew/include tools/trellis_io_probe.cpp \
  -L /opt/homebrew/lib/trellis -ltrellis \
  -Wl,-rpath,/opt/homebrew/lib/trellis -o target/trellis_io_probe
python3 tools/collect_iologic_evidence.py --native-probe target/trellis_io_probe
```

The report records the SHA-256 of the research programs, the input catalogs, the Trellis/nextpnr sources and fuzzers, the verification binary, the loaded libtrellis, and the available official PDFs. Before the collection, the pinned database files and Trellis sources are checked. The catalog was collected again with a byte-for-byte comparison of the results; the field placement was additionally checked against a separate reproduction of the `get_pic_tile` rules.

## Sources

- Trellis `73bd411731808d80ead650bbc9840d9e02911e9e`, database `015e0330630d7c238c0e4f2cdd9c8157eb78c54a`: `Bels.cpp`, `Chip.cpp`, tilegrid, iodb, bits.db and fuzzers 060/061/063–067.
- nextpnr `58e3b85866de409eac8b61393746140e7b30c808`: `pack_iologic`, `get_pic_tile`, `write_iol`.
- [ECP5 and ECP5-5G High-Speed I/O Interface, FPGA-TN-02035-1.3](https://www.latticesemi.com/view_document?document_id=50467), October 2020: pp. 38–39, 64–71, 81. The diagrams and tables on pp. 17, 39, 68–69 were inspected visually.
- [ECP5 and ECP5-5G sysI/O Usage Guide, FPGA-TN-02032-1.4](https://www.latticesemi.com/view_document?document_id=50464), January 2023: pin pairs and the difference between the sides, pp. 10–12; the diagram on p. 11 was inspected visually. The electrical elements of the diagram are not considered an already implemented digital PIO model.
