# Read-only configuration flash dump

Build with `examples/ulx3s/build.sh examples/ulx3s/flash_dump target/reference/flash_dump`
from the repository root.

SCK uses USRMCLK at 12.5 MHz. The shared controller has only READ, RDID and
RDSR constants, with no arbitrary command port. nWP/nHOLD stay high. Never
add program, erase or status writes: this is the board's configuration flash.

UART on L4, 115200 8N1, prints ID/status, 32-byte hex lines for
000000..00FFFF and 200000..200FFF, and CRC-32 over their concatenation.
It waits one second and repeats. LED0 mirrors the fabric SCK for the
USRMCLK probe test; LED1 is nCS. The design does not change the board image.

See [PERIPHERALS_FLASH.md](../../../docs/PERIPHERALS_FLASH.md) for the model,
timing, tests and measured UART output.
