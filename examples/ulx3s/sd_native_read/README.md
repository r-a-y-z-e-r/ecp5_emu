# Read-only native SD reader

Plain Verilog on the 25 MHz ULX3S clock. The reader sends 80 wake clocks at
390.625 kHz, initializes an SDHC card, selects four-bit data, and reads block 0
at 6.25 MHz. It then repeats the saved result once per second on `ftdi_rxd`
(L4), 115200 8N1:

```text
SDN OCR C0FF8000 RCA 0001 CSD <32 hex characters>
SDN CID <32 hex characters>
SDN B0 <8 hex CRC-32/ISO-HDLC characters> CRC16 ok
SDN B0RAW <1024 hex characters>
```

The CRC16 result checks all four DAT lines and the end bit. LEDs show error,
ready and CRC validity in bits 7..5. A timeout or unsupported card leaves the
error LED on; this small reader does not handle SDSC byte addressing.

The only command indices in `reader.v` are CMD0, CMD8, CMD55+ACMD41,
CMD2, CMD3, CMD9, CMD7, CMD55+ACMD6, CMD16 and CMD17 (argument zero).
No media-write, erase or lock command exists. CMD is bidirectional and driven
only for those 48-bit command frames; every DAT pad's output enable is tied
off. All five pads have `PULLMODE=UP`. The plain test
`reference_can_only_send_read_only_initialization_and_cmd17` audits these paths
and seals every HDL and LPF file. Updating a seal requires reviewing the full
reference again.

Build without loading any board:

```sh
examples/ulx3s/build.sh examples/ulx3s/sd_native_read target/reference/sd_native_read
```

The board comparison is pending. Use an SRAM load under the shared board lock
and HR04's `run.py` with prefix `SDN`, capturing to `out/sd/board_native.log`.
**A real card's CID and raw block 0 are private: keep the whole capture in
`out/sd/`, outside git.** Committed records use host-computed SHA-256 hashes,
never raw CID, raw block, or CRC-32 as a privacy hash. The replay test uses the
private block to reproduce the `B0 ... CRC16 ok` line; it does not compare
CID/CSD/OCR/RCA with the fictional emulator card. See
[the SD model and board method](../../../docs/PERIPHERALS_SD.md).
