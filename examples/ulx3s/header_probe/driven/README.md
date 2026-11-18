# Driven header check: empty J1/J2 only

**This design drives GP0..GP3 only, at DRIVE=4:** J1 pads 6/8/10/12, balls
B11/A10/A9/B9. The PCB shows only J1 and U1 on those nets. Every other header
signal is an input with a pull; ESP32- and ADC-shared nets are never driven.

Loading this variant on a board is allowed **only while J1/J2 are empty**.
J1/J2 were confirmed empty on **2026-11-17**; only the power cable was plugged
in. If connections change, recheck the empty-header condition.
**SRAM loads only.** The inputs-only parent remains the default.

Four bidirectional BB buffers drive the PWM signals and read the physical
pads through BB.O. Each second the design reports internal period counts,
then P and the four pad `rising_edges/high_cycles` pairs, levels and GN0 echo.
The first report measures a full second. The parent README defines the UART
fields and expected emulator line; HR13 will cite the statement and date.

Shared logic and UART are relative symlinks to the parent. No board loads were
performed for the emulator checks. Pad readback checks the digital path, not analog levels.
