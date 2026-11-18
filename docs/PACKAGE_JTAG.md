# Packages, special pins and JTAG

For the LFE5U-85F, the official [pinout CSV](https://www.latticesemi.com/view_document?document_id=50487) and the [BSDL archive](https://www.latticesemi.com/view_document?document_id=54703) were collected. They were checked against the Trellis iodb. The collection date is October 22, 2026; the physical package and the marking of the available board have not been checked yet.

| Package | All balls from the BSDL | Trellis GPIO matching the pinout/BSDL | Boundary cells |
|---|---:|---:|---:|
| CABGA381 | 381 | 205 | 750 |
| CABGA554 | 554 | 259 | 750 |
| CABGA756 | 756 | 365 | 750 |
| CSFBGA285 | 285 | 118 | 750 |
| Total records over the packages | 1 976 | 947 | 3 000 |

Every GPIO binding matched by ball, by the name PL/PR/PT/PB + number + letter, and by bank. All 1 976 balls were found in the official table, including power, ground, NC and dedicated pins. Non-GPIO pins are not declared absent because the iodb lists only PIOs. All 1 114 PAD rows and the additional functions, DQS and differential metadata are preserved in the source CSV. The number of PAD rows is not counted as the number of independent user inputs/outputs.

The die, its pads, the bonding of the package and the board connection are different levels. A PIO that is not bonded out stays in the die model, but does not get an invented external contact. A button/LED is connected to a ball of the selected package; the number of the board contact is set by the board schematic.

## Limit of confidence in the BSDL

The archive has version 1.2 dated 2025; inside it, the 85F files are version 1.13 of December 2016. The header warns: the description is preliminary, not verified on silicon, and assumes generic bidirectional I/O; for LVDS, VREF and output-only modes an application-specific BSDL is required. Therefore the 750 elements are the confirmed structure of **this description**, not a hardware check of any modes.

The parser checked the continuous indexes 0…749, the uniqueness of balls and the references from output cells to control cells. For each package, the chain order, the type/role of the cell, the safe value and the disable-result are preserved. Before boundary-scan execution, these values are tied to the effective mode of the pad and to the ownership by the configuration controller.

The IR has 8 bits; the BSDL sets IDCODE `0x41113043`, consistent with the target Trellis entry. The possible version bits of the physical instance are checked by reading the board. The BSDL USERCODE reset `0xffffffff` differs from the defaults of the software Trellis Chip; these are different levels of data. There are 23 named opcode entries and 76 PRIVATE per package; SAMPLE/PRELOAD share 0x1c. PRIVATE entries are kept with their semantics absent and do not become permission to experiment with arbitrary commands.

## TAP and the user JTAGG

According to [sysCONFIG FPGA-TN-02039-2.5](https://www.latticesemi.com/view_document?document_id=50462), pp. 20–21, the TAP is available after POR; TMS and input data are captured on the rising edge of TCK, and output data are shifted on the falling edge. TCK may stop at any level. TDO is Z outside Shift-IR/Shift-DR; when the driver is enabled, the instruction/selected register also applies. TDI/TMS/TDO have weak pulls, TCK does not. The configuration observation of DONE through JTAG may be subject to a boundary cell, so it cannot always be read as the direct internal DONE.

The model requires a standard TAP with an IR, BYPASS/IDCODE/USERCODE/boundary scan, an instruction decoder and separate user chains of the JTAGG. The JTAGG itself does not store an arbitrary user data chain: its FFs and logic are in the fabric. The ports JTCK/JTDI/JSHIFT/JUPDATE/JRSTN, JCE/JRTI and JTDO must refer to the same TAP.

The library guide for the JTAGG gives a short description of access to the internal signals. The detailed table of the earlier JTAGF cannot automatically be taken as independent evidence for the ECP5. An additional primary source is the [experimental ECP5 User JTAG project](https://github.com/tomverbeure/ecp5_jtag): the author describes ER1=0x32/ER2=0x38, a registered JTDI on posedge and JTDO on negedge, the selection of JCE in Capture/Shift and of JRTI in Idle. This is a useful candidate for checking; the exact event boundaries, the reassignment of ER1/ER2 by database fields and the interaction with configuration remain Y06. No hardware observations have been recorded for this project yet.

## Saved data

[package_system_research](inventory/LFE5U-85F/package_system_research/evidence_report.json) contains nine files: the package summary, the full table of balls, the source PAD rows, the GPIO check, the boundary cells, the instructions, the register-access/capture data and the gap report. No gaps in the ball matching were found; this does not mean the JTAG model is complete.

```sh
python3 tools/collect_package_system.py
```

The preservation of USERCODE/SED/control registers when reading a bitstream and the necessary additions to the loader are described in [SYSTEM_STARTUP.md](SYSTEM_STARTUP.md). The sequences of real programming, readback/security and reconfiguration belong to Y08.
