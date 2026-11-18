# Read-only EDID and a DVI source

`edid_reader.v` can send only A0 (write address), 00 (word pointer), repeated
START, A1 (read address), then read 128 bytes, ACKing all but the last, followed
by NACK and STOP. It has no data-write state, payload port or E-DDC segment write.
Both DDC pads are open-drain with PULLMODE=UP. The plain audit in tests/rtc.rs
seals the FSM, every Verilog driver and the pin mapping, and rejects added writes.

`dvi_source.v` is hdmi_bars/top.v renamed, and pll.v, tmds.v and video_out.v are
copies of that design's video path. The 25 MHz pixel clock gives 640x480 at
59.52 Hz; DDC/UART use the board's 25 MHz clock. EDID does not negotiate this mode.

Every second, UART L4 (115200 8N1) prints `EDID nack`, or `EDID <256 hex chars>
sum ok` (or `sum bad`), all on one line. The byte sum is checked without changing
any EEPROM data. With the emulator's default synthetic monitor it reads XEM,
"ECP5 EMU"; ECP5_EDID=off gives a NACK even though DVI still runs.

[Model, PCB evidence and board method](../../../docs/PERIPHERALS_EDID.md).
A real monitor's EDID contains identifying serial data: captures and replay images
stay in out/edid/, outside git. A future board record stores only hashes and decoded
non-identifying facts. No board loader or serial port is run by these tests.
