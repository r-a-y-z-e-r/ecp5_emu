# ECP5 85F emulation plan

> **Note (2026-10-29):** the order of work is now defined in [ROADMAP.md](ROADMAP.md). This file
> keeps the original design plan; its stage statuses are out of date. It was translated to English on
> 2026-10-29; reconciling the statuses is still open.

Current code milestone: [observation of the input stages in a slice with an active ALU54](docs/DSP_ALU_INPUTS.md). For a .bit/.config with two ALUs, a limited MULT representation `operands_only` is executed: INPUTA/B and ROA/ROB, without P/SIGNEDP and without writing the shared output bank. ALU arithmetic and flags are not implemented yet; all-plc with an active ALU is rejected. The earlier [ROA/ROB and 5 616 connections](docs/DSP_FORWARDING.md), [two products of the shared bank](docs/DSP_OUTPUT.md), EBR, LUTRAM and global controls are preserved. Next come the OP table and the ALU sources, from an exact model or from board measurements; the output CE/reset/SIGNEDP and the physical layout of the ALU/flags remain open. The GUI and the physical POR/START/transport are still ahead. The old `run` is a separate prototype.

Implementation started after the initial collection was completed. The first milestone is the Rust `DeviceModel` with the constant objects of the full 85F, directed route descriptions, source sites and package pins. The implemented scope, checks and limits are in [docs/CORE_STATUS.md](docs/CORE_STATUS.md). The summary of the materials is in [docs/RESEARCH_STATUS.md](docs/RESEARCH_STATUS.md), and the order of further implementation is in [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md). The history of earlier milestones below keeps their original scope of checking.

On October 22, Trellis/database, nextpnr and Yosys were updated to the current installed toolchain. The new bitstream build, the main catalog, PLC/LUTRAM, EBR addressing and FF/CCU2 were checked; the changed preload/latch synthesis rules are kept separately. The versions, the scope of the repeated checks and the archive of the previous snapshot are in the [update report](docs/TOOLCHAIN.md).

The agreed goal of October 22, 2026 is a full structural digital twin of the ECP5. Every architectural resource, wire, directed buffer and programmable connection keeps its identity, position, settings and observable state. The model includes the used and unused areas of the die.

The initial target model is `LFE5U-85F`, as named in the existing bitstream. `CABGA381` is used by the existing example; the marking of the physical chip and board must be checked before the hardware checks. Extending to UM and UM5G will require the SERDES and PCS resources that the U variant does not have.

The exact digital behavior of the available modes is required, including the sequence of events, initialization, internal registers, pipelines and the interactions between blocks. PLL, DLL, controlled delays and sensors need explicitly defined digital models. A full electrical model, a transistor schematic and extraction of the physical delays of all conductors are out of scope. The geometry from Trellis defines the architectural placement; the shape of the die metallization would need other data.

## Main decisions

- The main model describes the whole die. Optimization of the computation keeps the link to all physical objects.
- Physical tiles, functional blocks, internal nodes and the site names from the database are distinguished. One block may receive settings from several tiles; several sites may describe alternative modes of one hardware resource.
- Trellis is used to read the configuration and to describe the architecture. Both inputs, `.bit` and `.config`, are reduced to one CRAM representation that accounts for defaults and overlapping settings.
- The event kernel supports several clocks, both edges, asynchronous inputs, the states `0/1/X/Z`, driver conflicts and a consistent order of simultaneous events.
- Every observable state is bound to a resource, a port or a wire. The GUI and the peripherals use the state of the same kernel.
- Unexplored resources, modes and bits are accounted for explicitly. The presence of an object on the map and the presence of a verified behavior model have different statuses.
- As of October 22, 2026, the remaining disagreements between sources and behavior are checked on the available board. They receive an ID and a distinguishing experiment; the general collection continues. The order of recording results and the queue are in [docs/HARDWARE_VALIDATION.md](docs/HARDWARE_VALIDATION.md).
- The GUI starts after the kernel is checked against the agreed resource matrix. For now the emulation develops without a graphical interface.

## Stage 1 Device passport

Status: in progress.

Parse all tile types, sites, resource interfaces, settings and connections of the local database. Record the versions and checksums of the sources, the coordinate ranges, the configuration areas and the package variants. Account separately for service and factory test resources, split settings and alternative DSP sites. The initial collection across all families is now done to the extent of the [summary of materials](docs/RESEARCH_STATUS.md); the full recovery of internal nodes and unknown functions is not finished.

Results: a reproducible inventory, the device passport and a matrix "resource — internal structure — modes — decoding — behavior model — source — check". Every tile type must be assigned to a family; all discovered settings and unknown regions must remain in the catalog.

Done: all 148 tile types, the instances, the source site references, 3 274 settings and the pins of four packages were extracted. The catalogs were checked against the Trellis API, and the checksums and record links were verified. The discrepancies in PIO, EBR, PLL and DCC were explained, and 39 groups and 78 DSP slices were identified. Edge clocks, DLL/DQS and user system instances were added. The 96 282 ports of 502 objects were checked exactly against the Trellis library, including all EBR/MULT18/ALU54. The settings map covers 21 987 field fragments by instance; shared bits, ambiguities and split WID words are preserved. All 54 721 site references are preserved; 120 IO/ECLKSYNC bindings are still tentative, and 1 496 service records are not bound yet. For unknown families, a report with the exact limits of the available information was assembled. The first decoder of the effective configuration and the link of INIT to EBR are implemented. Next are the internal DSP storages and the full structure of the remaining families.

## Stage 2 Structural model

Status: the Rust architecture layer has been started; the full graph of internal nodes and the integration of behavior are still ahead.

Define the permanent identifiers of the device, tile, resource instance, internal node, port, wire and switch. Distinguish tile coordinates, vendor site coordinates and the position of objects for visualization. Preserve the directions of buffers and the structure of the clock network. Define the links of alternative sites to shared hardware nodes.

Result: a specification of the structural model and of the data formats. With it, the whole die can be described and the internals of each block family can be exposed. The existing prototype is checked against this specification before any further extension.

The first draft is recorded in [docs/STRUCTURAL_MODEL.md](docs/STRUCTURAL_MODEL.md). It fixes separate layers of architecture, configuration and state, permanent IDs, directed connections, a shared storage of modes and the decisions on the parts of the prototype. The storage format, the internal structures of all families and the full check are still ahead.

## Stage 3 Configuration recovery

Status: started; the decoder of the first set of resources works.

Recover the effective settings of whole resources from CRAM. Account for default settings, inversions of configuration bits, shared bits, interpretation modes, tile groups, the separate initialization of EBR and the bank configuration. Preserve the original and unknown bits with their owners.

Result: the same model configuration from the corresponding `.bit` and `.config`. For each resource, an explanation is available of which bits set its mode and connections.

Done: the `decode` command recovers CRAM through libtrellis and parses 14 417 composite EBR/DSP fields and bound clock/system resources. Ambiguities and the origin of bits are preserved. The link of EBR to the nine-bit INIT through WID is implemented, including the 5+4 split. On a purpose-built example, the CRAM, the settings and the memory data from `.config` and `.bit` matched. Full coverage of the remaining families, addressing of memory ports and partial reconfiguration are still ahead. Details are in [docs/CONFIG_DECODER.md](docs/CONFIG_DECODER.md).

## Stage 4 Event kernel

Status: the first milestone is implemented, see [EVENT_KERNEL.md](docs/EVENT_KERNEL.md). Checked: independent and generated clocks, both edges, local reset, X/Z/pulls, simultaneous capture, timer cancellation and feedback. Automatic plans, save/resume, startup and the remaining sequential blocks are still ahead.

Define the virtual time, the phases of event processing, the capture of inputs and the publication of the new state. Support several independent clocks, generated clocks, DDR edges, asynchronous resets, delayed events and the detection of circuits that do not settle. Provide for observation, an event log and saving of state.

Result: deterministic checks of the order of events, of synchronous and asynchronous circuits. Graphical replay of events does not change the virtual time or the behavior of the circuit.

## Stage 5 Models of all resource families

Status: ahead.

The order of implementation is set by dependencies:

1. Logic fabric, LUT and extension mux, FF, CCU2, LUTRAM, routing, basic PIO, GSR, POR and OSC.
2. PLL, DCC, DCS, dividers, global and edge clock networks, DLL and controlled delays.
3. EBR with port modes, initialization, output registers and access conflicts; DSP with pre-adders, arithmetic nodes, cascades and internal pipelines.
4. IOLOGIC and SIOLOGIC, DDR, gearboxes, DQS and tri-state control.
5. JTAG, USRMCLK, SED, DTR and the remaining configuration, system and test functions from the passport.

Result for each family: a structural description, decoding of modes, an executable model and a check. Support for one common mode does not close the whole family.

## Stage 6 Checks and peripherals

Status: ahead, in parallel with Stage 5.

For each mode, prepare small circuits, then check the interaction of resources. Use the vendor reference models where they are available, and reproducible experiments on the physical chip. Check each result at the level of the interfaces and of the available internal states. Give particular attention to memory with independent clocks, DSP pipelines and cascades, clock switching and DDR.

Add models of buttons, LEDs and seven-segment displays that can be checked without a GUI. The connection is made through package pins. The models use virtual time, account for polarity, PWM and multiplexing; button bounce is set as an environment parameter.

Result: a coverage matrix with links to tests, references, hardware measurements and the remaining limits. Unexplored user modes remain open tasks.

## Stage 7 Full-chip GUI

Status: ahead, after the kernel is checked.

Show the whole die with scaling down to the internal nodes of the resources. Add a view of settings, memory, registers, routes and signal history. In step-by-step viewing, show the events; in fast running, show the state and the activity statistics. Buttons, LEDs and displays are connected to the peripheral models that are already checked.

Result: a visual representation of the same structural model and its state, with the ability to explore the whole chip.

## Current milestone

The current priority is collecting more information about the device. The first layer of EBR/DSP decoding and the INIT binding work. An [analysis of the DSP internal structure](docs/DSP_INTERNALS.md) was assembled: width modes, the shared output, the split of the ALU54B registers, the directions of the chains, 1 326 groups of identical encodings and 18 parameters without a field of the same name. A [map of the DSP wires](docs/DSP_ROUTING.md) was assembled: 515 matched port names, 546 namespaces, 7 020 PO→A paths and 156 control bits. 340 899 conversions of names and coordinates were checked against libtrellis. 306 overlays of fixed and programmable representations per block and 24 boundary namespaces need interpretation. The questions D01–D10 are kept separately. Diamond is not installed; exact behavior models have not been obtained yet. The DSP gaps stay in the research queue; the lack of Diamond does not stop the collection for the other families. The full structure, the event kernel and the extension of the decoder remain the next implementation stages. The service gaps, the IO/ECLKSYNC candidates and the eight fixed ECLKSYNC bypasses stay in the passport with separate checks. The existing computational kernel remains a prototype and is not considered a full model of the die.

Additionally, an [analysis of IOLOGIC/SIOLOGIC](docs/IOLOGIC_RESOURCES.md) was assembled: 13 269 ports of all 365 IOLOGIC/SIOLOGIC and the related PIO were checked against libtrellis. In a separate research catalog, 9 660 IOLOGIC/SIOLOGIC fields and 1 316 PIO data/tri-state selection fields are bound. 896 shared CRAM positions, 364 groups of identical encodings, the pairs for 7:1, the links to 14 DQS groups and the special SPICB0 without named settings are preserved. The questions I01–I10 remain open; the working decoder was not extended by this collection.

A [map of IO paths](docs/IO_ROUTING.md) was assembled: all 365 input delay switches, 1 460 output edge paths and both sides of the shared 7:1 lines. 603 route bits outside the named fields were found. 25 471 conversions of name/coordinates and 361 bitwise mux patterns were checked. The DQS/ECLK links are preserved by local paths and by membership in a group/bank; the physical distribution segments still need research. The ownership of internal FFs and the conditions for the activity of bypasses remain open.

An [analysis of DQS/DDRDLL control](docs/DQS_CONTROL.md) was assembled: 19 control rules, 224 FIFO requirements, 52 delay code paths and another 34 selection bits outside the named fields. The interfaces and 166 fields out of 194 fragments were matched; the mode masks, the conversion of negative delay and the discrepancies of defaults are preserved. MEM_SYNC is accounted for as a soft IP in the fabric. The exact FIFO transitions, the representation of DDRDEL, DIVOSC, the LOCK time and the calibration remain in Q01–Q08; the available sources do not close them.

An [analysis of EBR addressing and modes](docs/EBR_ADDRESSING.md) was assembled: six representations of the shared array, 238 roles of address pins, nine-bit masks, three write modes, INITVAL and 64 CSDECODE conversions. All 111 public PDP ports and 112 connections after conversion are preserved. Six Yosys probes checked the addresses and 12 288 INIT words. The discrepancy of the PDP36 halves, the control priorities, partial writes, collisions and the initial outputs remain in B01–B08. This is information collection; EBR execution was not added.

An [analysis of EBR control](docs/EBR_CONTROL.md) was added: the synthesis contract is separated from the hardware rules, and seven circuits showed the conditions for moving the reset/pipeline out into the fabric. 812 connections and 896 combinations of control inputs were checked; the level calculation matched Icarus on the synthesized circuits up to the inputs of the EBR blackbox. All of B01–B08 are covered by 12 described series of future experiments HB01–HB12. The bench code and the hardware runs are still ahead; these questions do not delay the further collection.

An [analysis of PLC/LUTRAM](docs/PLC_LUTRAM.md) was assembled: the interfaces of both representations of all 10 455 PLCs were checked against libtrellis, 3 136 500 port records in total. 99 patterns of named fields, 128 INIT positions, the shared MODE A/B/C, the RAMW paths, the address permutation, 100 shared CRAM positions and all local routes were assembled. 2 565 bitwise groups were checked against the API. Six Yosys circuits and probes of the open models are saved separately; the errors found in the legacy/COMB models were not carried into the hardware contract. L01–L09 and the six future series HL01–HL06 are in the board queue. The decoder and the computational kernel were not extended by this collection.

An [analysis of FF/CCU2 and PLC outputs](docs/FF_CCU2.md) was added: 50 fields, 194 route variants and 305 bitwise groups were checked against libtrellis. DI/M/SD, the areas of the shared controls, 64 raw combinations of CE/CLK selection/LSR and 16 contexts of the shared INJECT/F bits are preserved. The open TRELLIS_FF was checked in 256 variants on 14 336 transitions; CCU2C in 164 variants on 83 968 input sets. All 10 455 carry outputs were collected: 10 370 with a neighboring PLC and 85 boundaries without a PLC BEL. Among the 14 synthesis probes, two preload limits, the LUT implementation of the latch and the added CE priority logic are saved separately. The global GSR and SD are absent from the behavior of the open FF model; the questions S01–S06 and the five future series HS01–HS05 keep the hardware status open. This next collection was done: [global networks and PLL/DCC/DCS/OSC](docs/CLOCK_NETWORK.md), [system startup](docs/SYSTEM_STARTUP.md), [PIO/banks](docs/PIO_BANKS.md), [packages/JTAG](docs/PACKAGE_JTAG.md). All results and the remaining questions are collected in [RESEARCH_STATUS.md](docs/RESEARCH_STATUS.md).

Materials: [LFE5U 85F passport](docs/DEVICE_PASSPORT.md), [resource bindings](docs/RESOURCE_BINDINGS.md), [PLC/LUTRAM](docs/PLC_LUTRAM.md), [FF/CCU2](docs/FF_CCU2.md), [edge clocks and system blocks](docs/CLOCK_SYSTEM_RESOURCES.md), [EBR and DSP](docs/MEMORY_DSP_RESOURCES.md), [EBR addressing](docs/EBR_ADDRESSING.md), [EBR control](docs/EBR_CONTROL.md), [hardware checks](docs/HARDWARE_VALIDATION.md), [DSP internal structure](docs/DSP_INTERNALS.md), [DSP wires](docs/DSP_ROUTING.md), [IOLOGIC/SIOLOGIC](docs/IOLOGIC_RESOURCES.md), [IO paths](docs/IO_ROUTING.md), [DQS/DDRDLL control](docs/DQS_CONTROL.md), [configuration decoder](docs/CONFIG_DECODER.md), [structural model draft](docs/STRUCTURAL_MODEL.md).
