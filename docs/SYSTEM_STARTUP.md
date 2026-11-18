# Startup, configuration controller and system blocks

The continuation in the main code is [global controls and explicit wake-up phases](SYSTEM_EXECUTION.md): ASYNC and HY01 synchronized user GSR, a separate startup GSR, GOE/GWDIS and a software profile of sequences 4/21. Physical POR/START/transport and PUR remain unmodelled. HY01 closes the user-GSR latency and configuration-state questions below; the remaining controller questions in the research snapshot stay open.

## Synchronous user GSR: HY01

[HY01](inventory/LFE5U-85F/hy01_sgsr/README.md#accepted-functional-rule-and-timing-hazards)
measured same-clock, inverted-clock and other-clock slice FFs with REGSET RESET
and SET. The executed rule is `user_gsr = active(GSR pin) OR s2`, with plain
rising-SGSR-CLK stages `(s1,s2) <- (active,old s1)` and initial state `11` after
configuration. Neither stage has an asynchronous preset, local reset or global
reset. The effective user GSR acts asynchronously through the existing reset
path for every supported GSR-enabled resource; startup GSR remains separate.

With GSR_N high, hy01b's ten loads hold enabled FFs through rising edges 1 and
2; first capture is edge 3. Falling B-A is 1 in loads 1,2,4,10 and 2 in
3,5,6,7,8,9, depending on wake phase. The no-SGSR hy01c control has rising and
falling B-A=0 in every load. Both counters retain configuration REGSET regardless
of whether user GSR is enabled (HS01).

All 120 exact R-log residual bins (71 at s2 assertion, 15 at raw release, 34
at s2 release) lie within two 100 MHz observer ticks (20 ns) after a modelled
reset transition. These are reset-network propagation-delay hazards. The emulator
gives propagation delays zero time; captures in that window are not additional
data rules. S has zero residuals; B/C have zero startup-offset residuals. A
short raw pulse can release the OR before s2 asserts; same-edge captures at s2
assertion are immediately reset again in the settled emulator state.

SYNC requires measured ACTIVE_LOW polarity and a routed CLK. ASYNC/NONE retain
their existing polarity/disable behavior. Event/fast equal traces and X20 in
text/bitstream form are covered by `tests/sgsr_native.rs`. This does not model
the physical configuration transport, PUR or START clock.

Code update: the new [`load` path](CONFIGURATION_STATE.md) reads `.bit` directly and keeps the CTRL0/CTRL1/SED/USERCODE values, marking where parser defaults may apply. The loss in the earlier text conversion, which remains in the old `decode`/`run`, is described below. The full POR/programming procedure is not executed yet.

Materials collected for the digital model of the LFE5U-85F, October 22, 2026. The prototype does not yet execute the controller described here. The goal is to keep all of its states and user interfaces, even if some transitions are still unknown.

## From bitstream to state

Loading a ready configuration snapshot must be distinguished from executing the programming transport over time. Both modes use the common chip structure; executing SPI/JTAG, reset/reconfigure, CRC/error/status and DONE requires additional controller state.

Trellis already reads CRAM, EBR INIT and some of the service commands. However, equality of CRAM/INIT after unpacking does not prove equality of the whole configuration procedure. A study of `Bitstream.cpp`, `ChipConfig.cpp`, the Python binding and our bridge showed:

| Data | Where it is kept when `.bit` is read | What happens on conversion to `.config` |
|---|---|---|
| CRAM, EBR INIT | Chip.cram, Chip.bram_data | Kept |
| CTRL0, CTRL1 | Chip.ctrl0, Chip.ctrl1 | Lost |
| 32-bit SED reference | Chip.sed, command LSC_PROG_SED_CRC | Lost |
| USERCODE | Chip.usercode | Lost |
| SECURITY, command/DONE order | Some commands are consumed without controller state | Not restored |
| JUMP | Address/speed parameters are skipped with a TODO | Not restored |

A native research probe adds synthetic commands to an in-memory copy of the stream and checks four distinguishable values. All four were read into Chip and were replaced by defaults after `ChipConfig.from_chip → text → to_chip`. This probe checks the parser; the resulting stream is not intended for programming a board. In the current Python Chip, USERCODE and metadata are available, but not ctrl0/ctrl1/sed; our runtime bridge exports none of the four values.

`ChipConfig.sysconfig` can read/print text directives, but `to_chip()` does not apply them as a programming controller would. Some nextpnr settings are written to CRAM separately. For each `.sysconfig`, both the directive and its actual effect must be kept; the absence of the original command in `.config` is marked as unknown or by an explicit startup profile. The loader must not claim byte-for-byte equivalence of complete streams.

[system_metadata_research](inventory/LFE5U-85F/system_metadata_research/evidence_report.json) contains an audit of commands, a preservation table, four native probe results and eight startup rows. Among the declared Trellis enum commands, `LSC_WRITE_ADDRESS` has no handler in the deserializer; a name in the header does not mean support. The 22 rows also include commands of other families and not the whole ECP5 protocol. The SSPI/JTAG transport commands given in §6 and Appendix B of the sysCONFIG document need a separate controller decoder on top of this image reading.

## Configuration process and statuses

According to sysCONFIG §5, pp. 22–24, the state sequence is power/POR → initialization → configuration → four wake-up phases → user mode. POR monitors VCC/VCCIO8/VCCAUX. During initialization the controller holds INITN/DONE low and clears the SRAM; the transition needs the end of tINITL, a deasserted PROGRAMN and a released external INITN. INITN is open-drain; it both shows the state and lets an external device delay the start. Its rise opens configuration; a stream error drives INITN low again.

At power-up the GPIOs are described as tristate with weak pull-downs, but there are exceptions for D[7:0], dedicated and dual-purpose pins. This pull cannot be assigned to all package pins. After wake-up the effective user settings apply. PROGRAMN, REFRESH through a supporting port, or a new power/POR return the process to loading; SRAM updates in transparent/background and offline/foreground modes have different effects on the running fabric.

The physical ports include master SPI with an external flash, slave SPI, slave parallel/serial and JTAG. In master mode the FPGA generates the clock and the flash commands; in slave mode the clock/commands come from outside. Boot-mode pins, port enable/ownership, HOLDN/BUSY/INITN/DONE and the daisy chain belong to the controller state and the board connection. The external flash is not an embedded ECP5 memory and is modeled as a peripheral. Dual/multiboot uses its images and controller transitions.

READ_STATUS returns a 32-bit word, least significant bit first (§4.2). The status groups collected:

| Bits | Meaning |
|---|---|
| 0, 3:1, 4 | Transparent, selected SRAM/eFuse target, JTAG activity |
| 5,7,14–16,20 | Password protection, decrypt enable/only, OTP/feature, detected encrypted preamble |
| 8–13 | DONE, ISC enable, write/read permission, busy, fail |
| 21–22 | Standard preamble, primary SPI image error |
| 25:23 | BSE: no error, ID, command, CRC, preamble, abort, overflow, SRAM size exceeded |
| 26–31 | Execution/ID/invalid-command/SED error, bypass, flow-through |

Reserved/internal-use bits are not given invented functions. Security/eFuse/password/AES must be separate persisted states of the device profile; CRC does not replace a permission check. The current CRAM loader does not support encrypted streams and unavailable keys. Their presence in the specification does not declare support by the emulator; execution would need a supplied key/profile and explicit command handling, with no attempt to guess the secret of the physical chip. These boundaries belong to Y08.

## POR, configuration and wake-up

According to [sysCONFIG FPGA-TN-02039-2.5](https://www.latticesemi.com/view_document?document_id=50462), January 2026, §8, pp. 49–50, four phases follow the internal DONE. The external DONE, GOE, GSR and GWDIS have different functions:

| Sequence | T0 | T1 | T2 | T3 |
|---|---|---|---|---|
| 4, default with DONE_EX=ON | DONE | GOE | GWDIS, GSR | — |
| 21, default with DONE_EX=OFF | — | GOE | GWDIS, GSR | DONE |

Before GOE the output drivers are blocked; inputs may work. GWDIS blocks memory/LUT writes. GSR holds the corresponding core state. In the needed mode an external low DONE can hold the completion. The document does not define the exact correspondence of the phases to the wake-clock boundaries; it was not extracted from the scale of the figure.

START selects the external clock of the startup procedure; the document requires a source that runs before the fabric starts and sets an upper frequency of 100 MHz. The four START references found belong to a real documented function but are not yet bound to controls/bits. They cannot be removed from the full map. PUR, the internal startup reset and the user GSR are kept separate; the effects on FFs with GSR disabled, on RAM and on other blocks are checked per family. GSR.GSRMODE and GSR.SYNCMODE are separate fields. Questions Y01/Y02 complement S01/L04/B08.

The JTAG port works after POR has fired. Bank power, the configuration mode and the physical marking belong to the device/board profile; CRAM does not report the actual voltages or the die revision.

## SED

[SED Guide FPGA-TN-02207-2.0](https://www.latticesemi.com/view_document?document_id=32323), May 2022, describes sequential reading of the configuration memory and comparison with a 32-bit reference. It needs the scan state, clock/divider, enable/start and a latched error; it is not a check of the file hash at load time.

SEDENABLE=0 stops the check and resets the statuses. SEDSTART allows passes; deasserting it while running lets the current one finish. SEDINPROG reflects the process; SEDDONE is set at the end and cleared on a new start/disable. SEDFRCERR=1 forces the error according to the table; SEDERR is kept until disable. The prose on p. 11 about a low SEDFRCERR contradicts this table; this is noted separately. The figure on p. 12 is labeled for ECP3 and does not become an exact timing model of ECP5.

For ECP5, SED uses the dividers of the internal oscillator 128/64/32/16/8/5, independently of MCLK. The external clock configuration must also be kept. The polynomial, the seed, the scan order/exclusions and the exact progress are not yet established: the CRC16 of the Trellis transport stream with polynomial 0x8005 cannot be substituted for the 32-bit SED CRC.

[PB1384 Rev1.0](https://www.latticesemi.com/view_document?document_id=52087), April 2017, reports false SEDERRORs when distributed RAM is used at the same time: its contents are not excluded from the calculation. The manufacturer's stated measure is to replace such memory with EBR. The 2022 guide continues to state that distributed RAM is excluded; the specific erratum is kept as a separate profile tied to PLC/LUTRAM. The bulletin promised a future revision in Q3 2017, but that does not confirm a fix in our instance. The public materials found are not enough to declare the problem fixed for the available board. HY03 and identification of the revision are required.

## DTR and USRMCLK

According to [Power Consumption and Management FPGA-TN-02210-1.4](https://www.latticesemi.com/view_document?document_id=50468), pp. 9–10, DTR outputs the code [5:0], a reserved zero bit 6, and valid in bit 7. STARTPULSE must be wider than 5 ns and is stretched internally to two cycles. The diagram shows 70 µs from the release of the pulse to valid; this is a documented reference point, not the HY04 timing measured on this board.

[HY04](inventory/LFE5U-85F/hy04_dtr/README.md#reading) now provides three complete room-temperature loads. Initial and final bytes are 0x9E. Assertion clears the output in three observer ticks; long high pulses hold 00, then release permits an ascending 00..1E count and valid 9E. Busy starts restart that count, with no queued completion. The 24/25-tick increments follow a variable startup delay (01 at fall + 27..101 ticks), so the event kernel and fast mode use the two-valued fallback 00 during high/conversion and restore the board-profile byte 32.740 us after the latest definite fall. This is the 1995-isolated-window pooled median after nominal synchronizer correction; width/phase dependence is not replayed. Fast mode host-schedules completion at the exact latest fall + 32 740 000 ps (1c79418); a coincident clock edge and completion are published together before captures. Retrigger cancels/restarts the deadline, and fast snapshots preserve it. Y05 still includes the Celsius scale, other temperatures and sub-40 ns conversion details.

DTR_TEMP=25 is a value of the six-bit simulation code for which the document names 25 °C. A linear scale of all 64 codes cannot be derived from a single match. The temperature and the conversion profile are environment inputs, not CRAM contents.

USRMCLK has data and tristate for CCLK. The owner of the pin may be the configuration controller or the user logic; the MASTER_SPI_PORT mode and the reconfigure stage must take part in resolving ownership. The shared bit CCLK.MODE/OSC.DIV is discussed in [CLOCK_NETWORK.md](CLOCK_NETWORK.md). Y07/Y08 cover switching of owners, CRC/security/readback and partial/transparent reconfiguration; these functions are not implemented yet.

## JTAG and service functions

The package pins, IDCODE, IR and boundary scan are analyzed in [PACKAGE_JTAG.md](PACKAGE_JTAG.md). The standard TAP, the user JTAGG and the programming interface must be separate parts of one controller, with common TCK/TMS/TDI/TDO and an observable state.

BCINRD/BCLVDSO are now identified from the power guide as dynamic controllers of the side banks 2/3/6/7, see [PIO_BANKS.md](PIO_BANKS.md). Their function is known, but the source references do not yet have a confirmed structural binding.

No public executable description was found for the 1 463 CIBTEST, 12 CLKTEST, eight BREFTEST and one PVTTEST. The references, the observed wires and the POR/VIQ tiles remain in the map with their level of detail. Full architectural coverage does not allow assigning an invented function to the factory nodes.

Questions Y01–Y09 and the series HY01–HY05 are kept in [clock_system_contract.json](inventory/LFE5U-85F/clock_system_research/clock_system_contract.json). HY04 now supplies the DTR observations above; it does not close the other system questions.
