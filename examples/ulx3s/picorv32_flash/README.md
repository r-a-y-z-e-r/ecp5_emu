# PicoRV32 executing from configuration flash

Build from the repository root:

```sh
examples/ulx3s/build.sh examples/ulx3s/picorv32_flash target/reference/picorv32_flash
```

The committed four EBR hex lanes contain the boot program. `firmware.hex`
contains the XIP payload used by the emulator's Rust demo image. To rebuild
both, run `firmware/build.sh` with RISC-V clang and lld available.

Memory map: 8 KB EBR at zero (boot below 1000, sieve/stack above), UART at
02000000 (data) / 02000004 (busy), fixed flash ID/status reads at 04000000/+4,
and a read-only 16 MB flash aperture at 10000000. Flash stores are ignored.
The controller has no arbitrary command input; it only emits 03, 9F and 05.
SCK is 12.5 MHz through USRMCLK, with nWP/nHOLD high.

Boot prints the ID, then checks the RVFW header at flash 200000: magic,
little-endian payload length and CRC-32, followed by code at 20000C. It
bounds the length to 4..65536 bytes and validates CRC before jumping to
1020000C. Firmware runs from flash and repeatedly reports 303 primes below
2000 and the cycle count. A missing or invalid image prints the first 16
bytes at 200000 once per second.

The emulator supplies a demo; this example does not install anything into a
real board. Never write, erase, or change the status of configuration flash.
See [PERIPHERALS_FLASH.md](../../../docs/PERIPHERALS_FLASH.md).
