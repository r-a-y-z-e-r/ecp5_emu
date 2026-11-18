# From the collected materials to the emulator

The plan of October 22, 2026 for the full LFE5U-85F. Implementation followed the initial collection. The first milestones are implemented for the architecture, the configuration/storages, the event kernel, [automatic execution of LUT/FF/CCU2/PIO](AUTOMATIC_EXECUTION.md) and [LUTRAM/RAMW](LUTRAM_EXECUTION.md), then [asynchronous/synchronized GSR and global controls with explicit wake phases](SYSTEM_EXECUTION.md) and [EBR ports](EBR_EXECUTION.md). The physical POR/START/transport and PUR are not implemented yet. The exact earlier limits are in [CORE_STATUS.md](CORE_STATUS.md), [CONFIGURATION_STATE.md](CONFIGURATION_STATE.md) and [EVENT_KERNEL.md](EVENT_KERNEL.md). The overlay of [EBR MODE/WEAMUX and CLK/LSR constants](EBR_MODE_OVERLAY.md) is qualified by the next milestone. The [first MULT18 profile](DSP_EXECUTION.md) adds the shared OR/FR reserve and combinational multiplication. The [input/pipeline stages of MULT](DSP_REGISTERS.md) and the transfer of the signed controls are executed with explicit D05 assumptions. The [shared output storage](DSP_OUTPUT.md) has the first MULT profile with CE=1/RST=0/GSR disabled; CIBOUT_BYP/OUTPUT_CLK is qualified, while the ALU/flags and the output SIGNEDP are not matched yet. The [operand forwarding ROA/ROB](DSP_FORWARDING.md) is now executed up to PIPELINE/OUTPUT; 5 616 connections to the ALU are checked structurally. The [limited representation of the input stages in an active ALU slice](DSP_ALU_INPUTS.md) now makes it possible to observe ROA/ROB without executing the ALU and the product. Next: establish the operation table and the ALU sources, from an exact model or from the board, then its execution with the shared storage. The output controls need separate evidence. PIO overlays, PRLD and mixed modes remain open. The items below keep the general target sequence; they do not claim that all families are ready.

The entry point of the materials is [RESEARCH_STATUS.md](RESEARCH_STATUS.md). Unknowns have IDs and stay open until the corresponding evidence appears. Support for a family requires structure, configuration, transitions and a check; the presence of its BEL on the map is not enough.

## 1. Unified architecture description

Introduce an immutable `DeviceModel`: all 14 231 tiles, the resources and internal architectural nodes, raw wires/segments, ports, mux/buffers and dedicated relations. Permanent IDs are kept regardless of whether the configuration uses them. Die pad, package ball and board connection are stored separately. Unused parts remain available to the observer.

The existing union-find of the prototype does not become the general routing executor. Merging is allowed only for a proven representation of one conductor; a directed buffer, a mux, a mode bypass and a clock-gate remain separate objects. For G_HPBX/VPTX/HPRX, special relations from CLOCK_NETWORK are used; for DQS/ECLK, membership in a bank/group plus the known local segments. Missing physical detail is marked, without an invented trace.

Shared RAM, FF and DSP lanes get a single state owner and different mode representations. The number of BELs/primitives is not taken as the number of independent physical storages. Service references remain objects with a level of information; an unused unknown object can be stored without computation, while an active unknown path requires a diagnostic.

Acceptance: all source references and 148 types are accounted for; the counts match the passport; endpoints are within bounds; alternative sites do not duplicate state; SPINE/TAP are directed and controlled by their own bits. This is a check of the architecture, with no promise of full metallization.

## 2. Full configuration and its origin

Extend the researched family catalogs into a single decoder: PLC/FF/carry/LUTRAM, PIO/banks, IOLOGIC/DQS/DLL, PLL/DCC/DCS and system settings. Check joint fragments, empty groups, identical codes, mode restrictions and shared bits. In particular, keep the 328 PIO BASE_TYPE default discrepancies, the 63 raw OSC codes and the contexts of the shared DSP/FF bits. The name of the source HDL parameter is not recovered when the encoding is indistinguishable.

The loader stores the original `.bit`/`.config`, the CRAM, the EBR INIT and a separate `ConfigurationImage` of service data. For `.bit`, CTRL0/CTRL1/SED/USERCODE and the commands lost by the current bridge must be extracted. For `.config`, mark the missing transport/control data and the startup profile that is set. Do not rewrite someone else's text configuration format without an explicit extension/sidecar.

For each decoded value, the source of the bits, the alternatives, the mode applicability and the limits of the evidence are available. Unknown bits are preserved. A conflict of interpretation gets an explanation and an ID, not a silent priority of defaults.

Acceptance: matching `.bit`/`.config` give identical known CRAM/INIT and fields; the service data and its absence are checked separately. The native metadata probe serves as a negative example of the loss of four fields. All split PLL, EBR WID and IO fragments are checked together.

## 3. Event kernel and observation

The first milestone is implemented: exact rational picoseconds, a queue with timer generations, batch resolution of drivers, stabilization/capture/publication and a bounded trace. [EVENT_KERNEL.md](EVENT_KERNEL.md) sets the verified policy for coincidences and X/Z. The future OSC/PLL, startup, memory and full save/resume below remain tasks.

Define `DeviceState` separately from the architecture/configuration. Store time with an exact representation of the periods of generated clocks, without accumulating rounding errors; choose the final type by load/range. An event needs a time, a delta phase, a cause, a source and a timer generation, to cancel stale events after reset/reconfigure.

At one time instant, first collect the input stimuli, resolve the drivers and stabilize the combinational dependencies; then capture the inputs of the sequential blocks uniformly and publish the new states. The clock/asynchronous events that arise pass through the next explicitly defined phases. Fix the concrete semantics of coincidences by a specification and checks, before memory and DDR are integrated. The container iteration order does not set a hardware priority.

Support 0/1/X/Z, strong/weak, driver conflict, unknown address/enable, asynchronous resets and both edges. An allowed X is a tooling uncertainty; the physical board does not output Verilog X. Non-settling loops are detected and diagnosed with a chain of causes. For a documented collision, do not invent a constant port priority.

The log and the state snapshots are addressed by the same IDs; they are needed for checks, the GUI and repeating experiments. Disabled trace recording does not change the result. The optimization computes only what is affected, preserving the identity of all objects.

Acceptance: independent clocks, both edges, asynchronous reset, equal timestamps, conflict/Z/pulls, feedback and timer cancellation are reproducible when the storage order of the objects changes. These are meaningful checks of the kernel, without mirroring implementation details.

## 4. First executable part

Software end-to-end examples were reached for two eight-bit counters and two independent LUTRAMs with different edges and clocks. `.config`/compressed `.bit`, observed and all 83 640 FFs together with the active RAMW were checked. An explicit ready-unknown profile was used, without a startup procedure and with an unambiguous PIO/PULLMODE=NONE; [AUTOMATIC_EXECUTION.md](AUTOMATIC_EXECUTION.md) and [LUTRAM_EXECUTION.md](LUTRAM_EXECUTION.md) preserve the limits. The standard group A/B DPRAM + C RAMW is executed; PRLD, the physical startup and other PIO modes still need continuation.

Start with LUT/mux, the directed fabric, TRELLIS_FF/CCU2 and the basic PIO, then LUTRAM/RAMW. Use the formulas already collected, the INIT/address permutations and the tables of shared controls. Do not carry over the errors found in the open Verilog models. GSR/PUR/GOE/GWDIS are part of the system context from the first integration example.

The first end-to-end result: a real bitstream with two external clocks, a counter/carry, GPIO and a separate reset, with observation of paths/FFs/pads. It runs on the full structural map of the 85F, although at this milestone the other active families will still report an unsupported mode. This is a checkable intermediate result, not the completion of the full-chip task.

LUTRAM is now checked on all sixteen addresses and four columns, both edges and WREMUX; all-plc executes the write even without an observable read. SD=1 on FF C in RAMW has explicitly marked X data. ASYNC GSR, GOE/GWDIS and the explicit wake phases were added at the next milestone. Complex coincidences from L01–L09/S01–S06 remain questions/policies until HL/HS. The EBR ports were added at the next milestone; disputed modes stay explicitly limited. The GSR implementation relies on separate configuration and documentation rules: the open TRELLIS_FF has no full global behavior.

## 5. Clocks and startup

The first part of the system context is implemented in [SYSTEM_EXECUTION.md](SYSTEM_EXECUTION.md): ASYNC and HY01 synchronized user GSR, a separate startup GSR, GOE/GWDIS and the standard sequences 4/21. sys:step sets explicit logical boundaries; DONE in sequence 4 holds completion at T3. The exact stall of the early phases, the physical clocking of START and PUR remain open. [HY01](inventory/LFE5U-85F/hy01_sgsr/README.md) executes ACTIVE_LOW SYNC user GSR with a routed CLK, plain stages initially 11 and zero propagation delay; reset-boundary captures within 20 ns are timing hazards. The EBR ports use the same context; a configuration snapshot is not counted as execution of the transport.

Connect all primary/ECLK paths, 60 DCCs, two DCSs, ECLKSYNC/bridge/CLKDIVF. Implement the switching and stopping states, including the documented four edges of ECLKSYNC. A static DCS table does not replace the handshake.

Add the OSC with an explicit clock profile of the environment, four PLLs and four DDRDLLs/12 DLLDELs. Separate the periodic digital function, reset/standby/enable and the validity of lock/calibration. For unresolved analog transitions, use a named profile with a stated origin, without an instant LOCK by default. Phases, dynamic steps and the 3.5 factor need events on both edges.

The startup controller performs GOE/GSR/GWDIS/DONE and interacts with START/external DONE; the profile of a ready configuration is separated from the loading procedure. HC/HY check the disputed rules on the board. The whole chip does not become timing-aware for the sake of the digital order of these events.

## 6. Memory, DSP and high-speed IO

| Family | Order of implementation | Main acceptance conditions |
|---|---|---|
| 208 EBR | Shared array and INIT → widths/CS/masks → CE/OCE/reset/write modes → independent clocks | All six representations of one array; PDP36 B01 separately; 12 HB series without assigning an invented priority to collisions |
| 39 DSP groups | Shared lanes/output storage → arithmetic and sign/width → pipeline/control → cascades | All MULT/ALU/PRADD modes from the materials, unique register owners; D01–D10 stay visible |
| 365 IOLOGIC/SIOLOGIC | FF/latch/tristate/delay → DDR X1 → X2/7:1 → memory gearboxes | Shared A/B and C/D storages, edge order, separate SPICB0 and mode bypasses |
| 14 DQS | Pointers/FIFO → DATAVALID/BURSTDET → PAUSE/calibration | One pointer state per group, 224 consumers, no fictitious DQSG distribution graph |
| PIO/banks | Bidir/pulls/open-drain → differential/reference → bank controllers | Package binding, power as environment, unified driver resolution and HP01–HP04 |

Check each family first with distinguishing circuits, then the interactions: PLL→DCC→EBR, LUTRAM+GSR+SED, DSP cascade with different CE/RST, DQS→several IOLOGICs. Support for a common combination does not close the other modes.

## 7. System functions and programming

Add TAP/boundary/JTAGG/USERCODE and SED; USRMCLK/CCLK ownership and the HY04 DTR profile now execute in the event kernel and fast mode. The loading CRC16, the SED CRC32 and an ordinary file hash are different mechanisms. SED needs a reference and a profile of how the errata apply to the revision; DTR uses an environment byte (ULX3S default 0x9E), 00 during high/conversion because fall-relative count steps are nonuniform, busy restart and latest fall + measured pooled median 32.740 µs. The documented 70 µs remains separate evidence. Temperature-to-code calibration, other temperatures, width/phase variation and sub-40 ns details remain open. Fast mode uses host-scheduled completion at latest fall + 32 740 000 ps, publishing the profile byte with any coincident clock edge before captures (1c79418).

Extend the executing transport command by command: JTAG/SPI/parallel loading, error/security/readback, JUMP/multiboot, partial/transparent reconfigure. Keep an unknown command and report the support boundary explicitly. Partial reconfiguration changes only the affected configuration/states according to the established contract, and does not recreate the chip from scratch.

The factory CIBTEST/CLKTEST/BREFTEST/PVTTEST remain visible nodes with an unknown function until information appears. They do not get a "successful execution" from an empty handler. The completeness of the user architecture and a literal copy of the closed factory circuitry have different evidence bases.

## 8. Board, peripherals, then GUI

Before the experiments, record the board model/marking/package and the connection schematic. Develop distinguishing benches with an independent observer and saved `.bit/.config`/routes/versions. The prepared series are collected in [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md); none has been run on the board yet.

Buttons, LEDs and seven-segment displays first work without the GUI, through balls and virtual time: polarity, PWM, multiplexing and a configurable bounce. Then the interface shows the whole chip, the modes, the internal states, the configuration sources and the real signal history. Animation does not add delay to the kernel; the visual map is not presented as a photograph of the metallization.

Readiness criterion: all available user resources and modes have checkable structure/decoding/behavior, the interactions are checked, and each remaining gap is explicitly visible with its source and limit of applicability. A readiness percentage by the number of tile types is not used for this.
