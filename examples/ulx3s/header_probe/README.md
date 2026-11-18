# Header probe: inputs-only board default

**The default design never drives a J1/J2 signal.** All 56 GP/GN ports are
inputs, alternating GN=UP and GP=DOWN in connector order. It remains the
default even when empty headers allow the separate driven check. Only the
LEDs and UART L4 are outputs. `led[0]` echoes GN0.

The four counters run at 25 MHz with periods for 1/2/5/10 kHz PWM and
25/50/75/10% high times. Their wrap counts and a snapshot of all 56 input
levels print at 115200 8N1, first after 1 ms and then every second:

```text
HEADER 00000001 00000002 00000005 0000000A 55555555555555 E1
```

This is the **emulator's pull-only prediction**, not a board observation.
The four eight-digit hex values count completed periods since startup; the
14-digit hex word has GN0 in bit 0, GP0 in bit 1, through GN27/GP27 in bits
54/55. E is the captured GN0 echo. External attachments can change the inputs.

The plain tests in tests/header.rs audit both wrappers, pulls, driver strength,
pad-buffer wiring and the PCB endpoints of the four driven nets. They seal the
reviewed Verilog file lists, source and constraints without FPGA tools.

## Driven variant: empty headers required

**driven/ drives GP0..GP3 only, at DRIVE=4.** These are J1 pads 6/8/10/12,
balls B11/A10/A9/B9. Every other header signal remains an input with a pull.
Never drive the ESP32-shared GP/GN11..13 or ADC-shared GP/GN14..17.

Loading driven/ on a board is allowed **only while J1/J2 are empty**.
J1/J2 were confirmed empty on **2026-11-17**; only the power cable was plugged
into the board. Recheck that condition if the connections change.
Use **SRAM loads only**. No board loads were performed for the emulator checks.

The four inout pads use explicit ECP5 BB buffers. BB.O, not the internal PWM
wire, feeds the per-pad rising-edge and high-cycle counters. Every complete
25,000,000-clock window resets those counters and latches their results. The
first driven report follows a **full second**. Its emulator line is:

```text
HEADER 000003E8 000007D0 00001388 00002710 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1
```

After the four cumulative internal period counters, P gives four GP0..GP3
pairs `rising_edges/high_cycles`, all eight-digit hex. Expected per-second
counts are 1000/6250000, 2000/12500000, 5000/18750000 and 10000/2500000.
Divide high cycles by 25,000,000 for duty. The pad measurement samples the
input buffer at 25 MHz; it is not an analog voltage or edge-quality measurement.
GN0 changes also print after the first report, retaining the last completed
pad window while updating the internal counts, levels and echo.

The driven wrapper enables DRIVE_PWM=1 on the shared header_logic.v;
header_logic.v and uart_tx.v are relative symlinks to the parent files. Both
directories build directly with examples/ulx3s/build.sh.

The board check will SRAM-load **both** variants under the shared lock, prefix
HEADER, allowing at least a full second plus UART completion for driven/ (a
4 s capture allows three complete periodic lines). Compare internal counters
and pad-count pairs with the emulator; record the other input levels as
observations. HR13 will cite the empty-header confirmation and its date.

See [the mapping and input API](../../../docs/PERIPHERALS_HEADER.md).
