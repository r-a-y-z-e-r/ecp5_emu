# Edge clocks, DLL/DQS and system resources

Continuation of the October 22 collection: [global segments, PLL/DCC/DCS and OSC](CLOCK_NETWORK.md), [startup and system functions](SYSTEM_STARTUP.md), [PIO/banks](PIO_BANKS.md), [packages/JTAG](PACKAGE_JTAG.md). They complement the original structural catalog below. This research did not extend the main runtime; the current overall milestone is [RESEARCH_STATUS.md](RESEARCH_STATUS.md).

Clock and system resources are part of the [instance map](RESOURCE_BINDINGS.md). The interfaces were checked against the installed Trellis library. There are no executable models of these blocks yet. The catalog was later extended to EBR/DSP; its current overall totals are given in the [memory and DSP description](MEMORY_DSP_RESOURCES.md).

## Added instances

| Family | Instances | Ports in the catalog | Placement |
|---|---:|---:|---|
| CLKDIVF | 4 | 16 | Two each at R46C0 and R46C126 |
| ECLKSYNCB | 8 | 24 | Two each on rows 46 and 47 at the left and right edges |
| TRELLIS_ECLKBUF | 8 | 16 | The same four positions, two ECLK outputs per bank |
| ECLKBRIDGECS | 2 | 8 | R46C0 and R46C126 |
| BRGECLKSYNC | 2 | 6 | Synchronization of the outputs of two bridges |
| DLLDELD | 12 | 84 | Four each at the side ECLKs and four at the top TMID |
| DDRDLL | 4 | 60 | R0C0, R0C126, R95C0, R95C126 |
| DQSBUFM | 14 | 532 | Seven each on the left and right edges |
| GSR | 1 | 2 | R94C4 |
| JTAGG | 1 | 15 | R94C4 |
| OSCG | 1 | 2 | R94C4; the OSC output uses a global wire |
| SEDGA | 1 | 7 | R94C4 |
| DTR | 1 | 9 | R94C22 |
| USRMCLK | 1 | 3 | R95C0 |

This part of the catalog contains 60 objects with 784 ports. Together with DCC/DCS, the clock and system parts have 976 ports. All the wire names of these ports appear in the applicable database descriptions after the relative prefix is stripped. Such a name match does not by itself prove that the position or the complete route is correct.

## Banks and source names of ECLKSYNC

The Trellis constructor defines banks 7 and 6 on the left and 2 and 3 on the right. The blocks of the upper half are on row 46, those of the lower half on row 47. Each bank receives ECLK0 and ECLK1 through separate ECLKBUFs; their outputs are connected to the global `G_BANK{bank}ECLK{index}`.

In the tilegrid the synchronizers are named ECLKSYNC0–3, and in the configuration and the BEL they are named ECLKSYNC0/1_BK2/3/6/7. The BEL instances and the owners of their settings are determined by explicit names. The mapping of the source numbers 0–3 to these instances is so far assumed by order: 0/1 is the upper bank, 2/3 the lower one. These eight references have the status `instance_candidate`; a separate confirmation of the site dictionary is still needed. It is not required for the exact matching of the field `ECLKSYNC0_BK6.MODE` to the resource of the same name.

The bridge ECLKBRIDGECS0 is mentioned again in RMID_0. Its source record fully coincides with the ECLK_R record, so this is an additional reference to an already existing instance.

## Four DLLDELs missed by the chip constructor

`Chip.cpp` creates eight side DLLDELDs. The `132-dlldel` study also describes four top blocks. In the 85F they correspond to:

| Site | Anchor | Configuration tile |
|---|---|---|
| DLLDEL00 | R0C66 | TMID_0 |
| DLLDEL01 | R0C67 | TMID_0 |
| DLLDEL10 | R0C68 | TMID_1 |
| DLLDEL11 | R0C69 | TMID_1 |

The columns agree with the source sites and with the relative ends of the TMID connections. These instances have `native_bel=False`: calling the general DLLDELD constructor lets us check their interface, but does not prove that the full chip constructor creates them.

In the current database each DLLDEL has the fields DEL_VAL and DEL_ADJ. The additional settings MODE, DDRDEL and LOADN_USED are mentioned in the study, but they are absent from the tile descriptions in use. The export does not invent bits or values for them. Their effect and the completeness of the database must be clarified before the behavior model.

## DQS and its four configuration fragments

For each DQSBUFM the anchor is located in PICL0_DQS2 or PICR0_DQS2. The settings are spread over four tiles on rows −2, −1, 0 and +1 relative to the anchor: PICx1_DQS0, PICx2_DQS1, PICx0_DQS2, PICx1_DQS3. This is confirmed by `130-dqsbuf` and `nextpnr/ecp5/bitstream.cc::write_dqsbuf`.

56 source references are consolidated into 14 instances. Each one has 13 configuration field records, including three DQS.MODE fragments. The three MODE records must take part together in recovering one mode; they are not independent blocks.

The [DQS/DDRDLL control analysis](DQS_CONTROL.md) maps all 166 composite fields of 18 resources, the public interfaces and the defaults. 52 DDRDEL paths to DQSBUF and DLLDEL were traced, along with three reference-clock variants of each DDRDLL and 34 more selection bits. The control contract and the FIFO requirements are collected; the pointer encoding, the DDRDEL representation and the capture time remain open questions Q01–Q08.

## System blocks and setting owners

| Resource | Known fields and their tiles |
|---|---|
| GSR | GSR.GSRMODE in EFB0; GSR.SYNCMODE in VIQ_BUF |
| JTAGG | JTAG.ER1 and JTAG.ER2 in EFB0 |
| OSCG | OSC.DIV is split between EFB0 and EFB1; OSC.MODE in EFB1 |
| SEDGA | SED.CHECKALWAYS, SED.CLK_FREQ, SED.SEDEXCLK_USED in EFB2 |
| USRMCLK | CCLK.MODE in EFB1 and EFB3 |
| DTR | DTR.MODE in the DTR tile |

Repeated OSC/SED references from the OSC tile refer to the same system instances. The empty bits.db file of the OSC itself does not mean that the oscillator has no configuration.

For the clock and system resources listed here, 251 fields and 582 references to CRAM bits were exported, 581 unique bits in all. For each field the source tile, kind, name, default and database line are kept; for each bit, the local coordinates and the CRAM coordinates. Variants, polarities and bit groups remain in the original device_inventory.json. This is a map of the origin of the configuration, not a ready decoder of effective modes.

CRAM bit F508B1135, local F40B1 in EFB1_PICB1, belongs to both OSC.DIV and CCLK.MODE. Both owners are kept. The reason and the rules for joint interpretation still need to be checked; applying the defaults of these fields independently is not sufficient.

## Obstacle to literal execution of the Trellis graph

All eight ECLKSYNC have in bits.db a fixed connection from ECLKI to ECLKO, and next to it the ECLKSYNCB mode and the STOP port are described. Executing this connection unconditionally together with the dynamic model may leave the block bypassed while STOP is active. Such a description is suitable for routing tasks, but its semantics for the emulator need a separate check.

The eight cases are listed in [routing_interpretation_issues.csv](inventory/LFE5U-85F/routing_interpretation_issues.csv) with references to the mode and the source line. At this step the connection is not removed and is not declared a verified bypass model. Before the behavior is integrated, the dependence of the path on the mode must be determined, and STOP must be compared with a reference or a hardware experiment.

## Checks and reproduction

Added [nine portable catalog checks](../tests/test_resource_catalogue.py): preservation of the source records, identity of the objects, all PIOs and pins, the four DQS fragments, the constructor gaps, the statuses of uncertainties, ports, the origin of bits and the open questions of the ECLKSYNC bypass.

```sh
python3 tools/bind_trellis_sites.py
python3 -m unittest discover -s tests -p 'test_*.py' -v
```

A small research helper was used for an additional check against the local library. It calls only the constructors of the needed BELs and dumps their ports, without building the whole routing graph:

```sh
clang++ -std=c++14 \
  -I ~/FPGA/prjtrellis/libtrellis/include \
  -I /opt/homebrew/include tools/trellis_resource_probe.cpp \
  -L /opt/homebrew/lib/trellis -ltrellis \
  -Wl,-rpath,/opt/homebrew/lib/trellis -o target/trellis_resource_probe
python3 tools/bind_trellis_sites.py --native-probe target/trellis_resource_probe
```

An exact match was obtained for all 784 ports of the 60 calls: BEL name, port name, direction, coordinates and wire name. This is a check of the interfaces of the installed library, not independent proof of the placement of the blocks, of the whole routing, or of the digital behavior. The checksums of the helper, of its source and of the libtrellis actually loaded are kept in the report; without `--native-probe` the check field is null.

The original nine checks of the clock and system part passed and are now part of the common set of fifteen catalog checks. The reproducibility of generation and the agreement of the CSV with and without the library check are verified on the whole current set of files. Separately checked: the failure on a stale checksum before files are written, on malformed requests to the helper, and on a deliberate port mismatch.

## Remaining gaps

The [IO path analysis](IO_ROUTING.md) adds ECLK selections for all 224 side IOLOGICs and 14 DQSBUFs, and 2 016 DQS connections to consumers. Separate control bits of these selections were found. The local BNK_ECLK and some of the DQSG have no ordinary incoming edges; membership in the bank/group is confirmed by iodb and by special handling in nextpnr. The full model must keep both the local wires and these dedicated relations; the detailed physical distribution network has not been recovered yet.

Of the 1 496 unbound records, 1 463 belong to CIBTEST. The rest: CLKTEST — 12, BREFTEST — 8, PVTTEST — 1, START — 4, BCINRD — 4, BCLVDSO — 4. Their records and source tiles are kept. POR is so far represented by a tile with unknown internal structure. The interface of the configuration controller and the special package pins need a separate description.

Candidate bindings: 112 IO references and the eight source ECLKSYNC. For the service nodes the available evidence is now collected, and the EBR/DSP bit map has been added. Next, an effective decoder and the internal state of the resources are needed. The dynamics of PLL/DLL, STOP, RESET, locking and startup sequences remain part of the future behavior models.

## DTR execution (HY04)

[HY04](inventory/LFE5U-85F/hy04_dtr/README.md#reading) measured three complete
ULX3S loads: initial/final byte 0x9E, a variable binary-count intermediate
sequence, assertion/retrigger reset and completion after deassertion.
`DTR.MODE=DTR` dispatches the DTR model in the event kernel and fast mode; `NONE` is inactive X.
The full byte is an environment input (`compile::Board::dtr_code`, default
0x9E), not CRAM or a Celsius conversion. The bus is 00 from assertion until
latest fall + pooled median 32.740 us. Since 1c79418, fast mode pauses on
STARTPULSE changes and host-schedules completion at the exact fall + 32 740 000 ps;
completion and a coincident clock edge are published together before captures.
Retrigger cancels the pending deadline, and fast snapshots retain DTR state.
Width/phase-dependent latency, one-temperature calibration and details below
the 40 ns observer resolution remain open.
