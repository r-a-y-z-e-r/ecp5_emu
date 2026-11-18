# Monitor EDID on the shared DDC bus

Added on 2026-11-17. The pure Rust pad-fed [edid.rs](../src/peripherals/edid.rs)
models a connected monitor's write-protected, 24C02-style EEPROM at seven-bit I2C
address **0x50** (`A0` write, `A1` read). It shares SCL/SDA with the
[MCP7940N RTC](PERIPHERALS_RTC.md), which still answers only **0x6F**.

## PCB evidence

The source is [assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb),
the same file used by the 3D board and the [audio model](PERIPHERALS_AUDIO.md).
Its filename and silkscreen say **v3.0.8**; its title block says **rev v3.0.7**.

- GPDI1 pad **15** is `GPDI_SCL`; pad **16** is `GPDI_SDA`.
- U11, **PCA9306D**, pins **3/4** are `FPDI_SCL` / `FPDI_SDA`, reaching U1
  balls **E12/B19**. VREF1, pin **2**, is `+2V5`.
- U11 pins **6/5** are the connector's `GPDI_SCL` / `GPDI_SDA`.
  VREF2, pin **7**, is `/gpdi/VREF2`.
- U7, **MCP7940N**, is on the same `FPDI_SCL` / `FPDI_SDA` nets; R22/R23
  provide 3.3 kOhm pull-ups.

The model treats this as one ideal open-drain bus. It does **not** model the
level shifter's electrical behavior, reference supplies, rise times or cable.

## Presence, contents and protocol

`Attached` adds EDID whenever it attaches the HDMI receiver: the design must
drive all four positive GPDI pairs. **ECP5_EDID=off** disconnects the EEPROM,
so 0x50 NACKs, while the video receiver continues to decode. Other values leave
it present. A video-only design with no configured DDC bus still has EDID
status, but adds no unconfigured pad probes or answering inputs.

**ECP5_EDID_IMAGE** names a read-only source of exactly **128 or 256 bytes**.
Bytes are used verbatim: no checksum repair or identity changes. With 128 bytes,
the upper half of the 256-byte EEPROM is FF. Missing sources use the synthetic
base block; unreadable or wrong-sized sources warn and use that same default.
The source file is never modified.

The default is a checksum-correct **EDID 1.3** base block, assembled in Rust,
with no extensions or serial number. Manufacturer **XEM** means **eXperimental
EMulator** and is deliberately fictional: it was absent from the
[UEFI PNP registry](https://uefi.org/PNP_ID_List) checked on 2026-11-17. It is
not a claim of a registered identity; `EMU` is a registered ID and is not used.
Product 1, manufacture year 2026 and monitor name **ECP5 EMU** identify only this
synthetic fixture.

It advertises established **640×480@60**, preferred detailed **640×480@60**
(800×525 total, negative syncs), a second detailed **1280×720@60** (1650×750,
positive syncs, 74.25 MHz), sRGB/gamma 2.2, and range limits **59–61 Hz vertical,
31–46 kHz horizontal, 80 MHz pixel clock**. The nominal VGA clock is 25.175 MHz;
EDID detailed timings have **10 kHz units**, so the encoded clock is rounded to
**25.180 MHz** (59.952 Hz). This quantization follows the
[VESA E-EDID Release A, Revision 1 / EDID 1.3 format](https://glenwing.github.io/docs/VESA-EEDID-A1.pdf).
The unit test independently decodes both timings, the identity, descriptors,
range limits and checksum from the resulting bytes.

START, repeated START, STOP and ninth-bit ACK/NACK follow resolved pad levels.
A write address accepts one word-address byte to set the pointer. A repeated
START with A1 reads from that pointer; current-address reads work too. Each
completed byte advances it, including the last byte before the master's NACK.
Sequential reads wrap **FF → 00**, regardless of source length. Other addresses,
including E-DDC segment pointer **0x30**, NACK. X aborts until another START;
released Z reads high through the bus pull-up. There is no clock stretching.

**Data writes are NACKed and ignored**, counted by `ignored_writes`; the pointer
and contents stay unchanged. The model then waits for another START or STOP.
This is a conservative virtual write-protect policy, not a claim that every
monitor NACKs writes: real EEPROM protection/ACK behavior varies. Pointer writes
remain accepted, and the reference below cannot send a data write.

## Shared answers, status and replay

Both RTC and EDID see every row of the same resolved E12/B19 trace and decline
addresses belonging to the other device. `Attached` combines their answers in
**one external SDA driver**: `Some(false)` if either pulls low, otherwise `None`.
Neither drives high. The FPGA's open-drain pad and PULLMODE=UP resolve it, as in
[FAST_MODE.md](FAST_MODE.md). There are no duplicate B19 drivers or trace columns.

A usable DDC bus needs driven SCL and bidirectional SDA, the RTC's existing
attachment rule. The answering horizon stays one edge from START through STOP
and 16 idle edges. At 25 MHz the latter is 0.32 us, shorter than the 0.6 us START
hold time for a 400 kHz I2C master, so START is observed before the first bit.
Answers are applied 1 ps after the instant; the level shifter adds no delay.
RTC behavior and its source are unchanged.

`View.edid` supplies presence, accepted read transactions, completed bytes read,
last pointer, NACKs and rejected data-write count. `last_pointer` is the last
word address written or byte address fully read, before increment; after the
base-block read it is **7F**. `nacks` counts addresses this device declines,
rejected data bytes and the normal final **master NACK**. On the shared bus it
is a device counter, not a count of resolved bus errors: the RTC can ACK an
address EDID declines, and vice versa.

The CLI's final stderr line has this fixed field order:

```text
peripheral: edid present=yes|no reads=N bytes_read=N last_pointer=HH nacks=N
```

The HDMI monitor panel shows **EDID: present, N bytes read** or **EDID:
disconnected**. Changed status adds `# edid …` record notes; these contain only
counters and presence, never EDID contents. Snapshots retain the image, pointer,
protocol, counters and the combined SDA answer. Going back restores them and
forward execution regenerates the answers without logging them as user input.
No live-runner or app.rs changes are needed.

## Read-only reference and board method

[edid_read/top.v](../examples/ulx3s/edid_read/top.v) reads 128 bytes from zero
once per second with an approximately **100 kHz** master, and prints on L4 at
**115200 8N1**:

```text
EDID <256 uppercase hex characters> sum ok
EDID nack
```

A bad checksum prints `sum bad` without altering the bytes. The master can only
send **A0, 00, A1**: START, write address, fixed pointer zero, repeated START,
read address, 127 ACKs and a final NACK, STOP. There is **no data-write state**.
SCL/SDA are `low ? 0 : z` with PULLMODE=UP. The plain audit in
[tests/rtc.rs](../tests/rtc.rs) checks those transmit assignments and seals the
FSM, wiring, constraints and accompanying Verilog; mutation tests reject extra
bytes, states and drivers without running an FPGA toolchain.

The source also drives a copy of **hdmi_bars**' DVI path: 640×480 at 25 MHz,
800×525 total (59.52 Hz). It gives a real monitor a video source and makes the
emulator attach HDMI/EDID. It does not select a mode from EDID. The reference
uses GPDI video, DDC and UART only; it never writes the RTC or EEPROM.

[HR12](inventory/LFE5U-85F/hr12_edid/README.md) records three read-only SRAM
loads at 18:41Z on 2026-11-17, all nine lines **EDID nack**, matching ECP5_EDID=off:
no EDID responded despite the board-provided connector power, with no conclusion
about HR09's earlier run or any real EDID contents.

**A real monitor's EDID contains identifying serial numbers.** Keep its UART log,
raw image and any identifying decoded fields in **out/edid/** only, outside git.
A public record gets only hashes and decoded non-identifying facts, such as EDID
version and preferred mode. Do not copy the hex or the capture into a record.
The synthetic built-in block has no private identity.

[tests/edid.rs](../tests/edid.rs), ignored like the other reference tests, builds
through `common::reference::build`, checks default `sum ok`, disconnected NACK
and video decoding through the CLI's fast answering loop. It passes environment
overrides to child processes, without changing the test process's environment.
The private replay reads the first complete, two-space-indented report in
`out/edid/board_edid.log`, including a NACK capture, and skips cleanly if absent.
Replay images are temporary mode-0600 files in out/edid, removed afterward;
comparison failures do not dump identifying bytes. Live and CLI tests also check
status, and the live test goes back during the read and replays it.

## Limits

This is a 256-byte write-protected EEPROM, not an E-DDC segment implementation,
a hot-plug model, HDMI mode negotiation or a physical monitor. Extension bytes
in a 256-byte source are readable, but not interpreted by this model. The UART
reference reads only the base block. Voltage translation, monitor power, cable
quality and the real monitor's write-protect policy need separate evidence.
