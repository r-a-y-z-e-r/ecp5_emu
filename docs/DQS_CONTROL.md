# DQSBUFM and DDRDLL control

For the 14 DQSBUFM and four DDRDLL of the LFE5U-85F, the public interfaces, the CRAM settings, and the dependencies on delay sources are mapped. The control rules and the requirements on the FIFO state are collected. The full sequence of internal events, the pointer values at reset, and the DLL lock duration remain open; this stage adds no executable models.

The [IO path map](IO_ROUTING.md) is extended with 52 DDRDEL paths and 34 separate source-selection bits, which are not among the earlier 603 IO bits.

## Interfaces and configuration

| Resources | Native interface | Composite configuration |
|---|---|---|
| 14 DQSBUFM | 38 ports each | 11 fields each from 13 fragments; MODE is spread over three tiles |
| Four DDRDLL | 15 ports each | Three fields each: MODE, GSR, FORCE_MAX_DELAY |
| Total | 592 ports | 166 fields from 194 fragments |

All public ports of the Yosys declarations match the Trellis catalog in names and directions. Each DDRDLL additionally has DIVOSC, which is absent from the public DDRDLLA. These four outputs are kept with an unknown function.

For MODE, all 464 combinations of its known bits are listed per instance; separately, 16 combinations of FORCE_MAX_DELAY are listed. This is a study of encodings. The empty NONE pattern can formally match partially set MODE bits too, and two independent FORCE_MAX_DELAY patterns allow both no match and a simultaneous NO/YES match. Such states need context and verification; the catalog does not declare them valid hardware modes.

## Delay and reference clock sources

For each of the 14 DQSBUF, the paths from both DDRDLL of the same side were traced: 28 in all. For 12 DLLDEL, 24 more paths were traced. The top DLLDEL get variants from the top left/right DDRDLL, the side ones from the top/bottom DDRDLL of their side. All 52 paths consist of three records: the DDRDLL output, the global name DDRDEL, the local selection, and the consumer input.

For each of the 26 consumers, one selection bit for a second source was found. All of them also have a fixed representation of the default source. Selection conditions are needed before a path is executed; DQS.DDRDEL additionally sets the use of an external DDRDEL or of zero.

For each DDRDLL, three reference clock variants were found: BNK_ECLK0, BNK_ECLK1, and JCIBCLK0. They are encoded by two bits per instance. In total, the new 34 CRAM positions consist of 26 DDRDEL selections and eight clock mux bits. They were checked against all named settings of the database: no overlaps were found. The previously found local ECLK links with banks remain in the separate [IO catalog](IO_ROUTING.md#edge-clock-and-bank-membership).

DDRDEL has one port in the public and native interfaces, and the guides call it a delay-code channel. DCNTL has eight separate observable bits. The future kernel must preserve the semantics of the code transfer; the representation of DDRDEL and its exact relation to DCNTL are not yet established. Automatic reduction of this channel to an ordinary logic level has no sufficient basis.

## Control rules and state requirements

The FREEZE polarity was refined from the [datasheet FPGA-DS-02012-3.4](https://www.latticesemi.com/view_document?document_id=50461), p. 25: one stops the internal clock and the ring oscillator of the DLL, zero resumes operation. UDDCNTLN updates the code at zero; LOCK marks the code as usable. The duration of the lock and the relock is not yet specified.

The [library guide](https://www.latticesemi.com/view_document?document_id=53950), p. 296, defines the choice of the delay change direction on the falling edge of MOVE: one decrements the counter, zero increments it. LOADN returns the nominal delay; PAUSE stops DQSW/DQSW270. The held level of the clocks, the phase of the stop, and the influence of PAUSE on reads need clarification.

The [High-Speed I/O guide](https://www.latticesemi.com/view_document?document_id=50467), pp. 35–38 and 74, establishes the following requirements:

- The FIFO of the input IDDR has eight levels and three-bit pointers; the group controller is located in the DQSBUF. Writing belongs to DQS, reading belongs to ECLK.
- READCLKSEL sets a step of a quarter of the memory period. For the listed control changes, PAUSE is required 4T before the change and another 4T after it.
- DATAVALID marks the window of valid data and can remain high between consecutive reads. BURSTDET holds the result until the next read.

224 requirements for the FIFO in memory mode are kept in the data, with the IOLOGIC owner and the corresponding DQSBUF. The pointer encoding, the initial values, the width of the internal word, and the order of coinciding events are left unknown. No separate physical FFs were created from these requirements.

## Start-up and MEM_SYNC

MEM_SYNC in the generated interface is soft IP in the logic fabric. According to the description on pp. 39 and 80 of High-Speed I/O, its independent start clock controls STOP, resets, FREEZE, UDDCNTLN, and PAUSE. A snapshot of the outputs at RST is kept in `control_contract.json`; the exact order of releasing the signals does not follow from the table.

Bitstream emulation must execute the loaded control circuit, including a user replacement of MEM_SYNC. Adding an automatic start-up sequence on behalf of the chip would change the behavior of the circuit. For observation, this logic can later be recognized as a group of ordinary resources.

`LOCK_CYC` is mentioned in the library description of DDRDLLA, p. 251, as a simulation parameter. No standalone bitstream field is expected for it. The rule of the digital LOCK model has yet to be defined and verified separately.

## Delay encoding and defaults

Eight CRAM positions are kept for all 28 LI/LO words. The nextpnr writer uses the direct byte for PLUS and `(256 − value) & 255` for MINUS. For example, MINUS 1 gives 255, and MINUS 256 gives zero; the sign is set by a separate ADJ field. The table lists 512 transformation variants shared by LI and LO.

The ranges in the sources diverge: High-Speed I/O, p. 73, gives PLUS 0…255 and MINUS 1…256; the library guide, p. 295, gives 0…63 and 1…64. The eight bits and the transformation are confirmed by the database, the fuzzer, and the writer; the physical range and the delay scale remain a separate question.

Three levels of defaults are kept: the HDL declaration, the writer's fallback when the parameter is absent, and the default of the Trellis field.

| Parameter | Yosys declaration | nextpnr fallback | Trellis field |
|---|---|---|---|
| DQS_LI_DEL_VAL | 4 | 0 | A word of eight zeros |
| DQS_LI/LO_DEL_ADJ | FACTORYONLY | PLUS | Only PLUS/MINUS; default PLUS |
| GSR on DQSBUFM/DDRDLL | ENABLED | DISABLED | ENABLED |

These differences do not prove a tool error: the values belong to different stages of preparing the configuration. For FACTORYONLY, the transformation and the factory calibration have to be established. The library WRITE_LEVELING is also kept as a parameter with no established correspondence to a separate field.

## Open questions

| ID | What is still required |
|---|---|
| Q01 | The representation of DDRDEL, the relation to DCNTL, and the function of DIVOSC |
| Q02 | DLL lock/relock, reset, FORCE_MAX_DELAY, and the conditions for a change of LOCK |
| Q03 | The limits of the delay counters, saturation/boundary crossing, and CFLAG |
| Q04 | The encoding, the initial values, and the transitions of the FIFO pointers; the word width and the internal stages |
| Q05 | The entry/exit phases of PAUSE, the levels of the stopped clocks, and the effect on reads |
| Q06 | The exact delays READ → DATAVALID/BURSTDET and of data publication |
| Q07 | FACTORYONLY, the factory values, WRITE_LEVELING, and the disputed LI/LO range |
| Q08 | The start-up/update order of the specific MEM_SYNC circuit in the fabric |

This refines I07 of the [IOLOGIC analysis](IOLOGIC_RESOURCES.md). The current sources are enough to check the interfaces and the encodings; to close the listed transitions, additional models or hardware experiments are needed. No Diamond installation is available.

## Data and checks

13 files are stored in [dqs_research](inventory/LFE5U-85F/dqs_research/evidence_report.json): the interfaces, the parameters, the MODE/FORCE_MAX_DELAY masks, the placement of the word bits, the delay transformation, the DDRDEL paths, the clock sources, the selection bits, the FIFO requirements, the source connections, the configuration catalog, the control contract, and the report.

```sh
python3 tools/collect_dqs_evidence.py \
  --native-probe target/trellis_wire_probe --verify-pytrellis
```

5 843 name/coordinate transformations matched libtrellis. Through pytrellis, the catalogs of 14 tile types and 251 bitwise mux patterns were additionally checked. Regenerating all files gave a byte-for-byte match. These checks confirm the database data and the reproducibility, but do not replace verification of the dynamics of the blocks.

The hashes of the database source files and of the pinned Trellis sources are checked against the main inventory. The report pins the input catalogs, the nextpnr sources, the fuzzers, Yosys, the research programs, the library, and three PDFs. The new datasheet is FPGA-DS-02012-3.4, September 2025; High-Speed I/O is FPGA-TN-02035-1.3, October 2020; the library guide is Diamond 3.13 dated August 24, 2023. The diagrams and tables of High-Speed I/O on pp. 35, 37, 73, 80, of the datasheet on p. 25, and of the library guide on p. 251 were inspected visually.
