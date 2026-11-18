#!/bin/sh
# Homebrew LLVM supplies the RV32I target; Rust supplies lld if needed.
set -eu
cd "$(dirname "$0")"
CC=${CC:-/opt/homebrew/opt/llvm/bin/clang}
OBJCOPY=${OBJCOPY:-/opt/homebrew/opt/llvm/bin/llvm-objcopy}
if [ -z "${LD:-}" ]; then
  if command -v ld.lld >/dev/null; then LD=ld.lld
  else LD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld -flavor gnu"
  fi
fi
FLAGS="--target=riscv32 -march=rv32i -mabi=ilp32 -O2 -ffreestanding -fno-builtin -fwrapv -msmall-data-limit=0 -nostdlib -ffunction-sections -fdata-sections -Wall -Wextra -Werror"
mkdir -p build
for source in start.S lib.c main.c; do
  $CC $FLAGS -c "$source" -o "build/${source%.*}.o"
done
$LD -T link.ld --no-relax --gc-sections -Map=build/firmware.map -o build/firmware.elf build/start.o build/lib.o build/main.o
$OBJCOPY -O binary build/firmware.elf build/firmware.bin
python3 - <<'PY'
from pathlib import Path
image = Path('build/firmware.bin').read_bytes()
assert len(image) <= 32768, 'firmware exceeds RAM'
ram = image.ljust(32768, b'\0')
for bank in range(4):
    for lane in range(4):
        Path(f'../fw_{bank}_{lane}.hex').write_text(''.join(
            f'{ram[bank*8192 + word*4 + lane]:02x}\n' for word in range(2048)))
print(f'firmware: {len(image)} bytes; 16 images of 2048 bytes')
PY
