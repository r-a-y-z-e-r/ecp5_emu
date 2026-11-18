#!/bin/sh
# Build in target/, and keep the resulting byte-lane images next to top.v.
# Homebrew LLVM supplies the RISC-V target; rust-lld is the fallback linker.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
out="$root/target/reference/picorv32_sdram/firmware"
mkdir -p "$out"
CC=${CC:-/opt/homebrew/opt/llvm/bin/clang}
OBJCOPY=${OBJCOPY:-/opt/homebrew/opt/llvm/bin/llvm-objcopy}
if [ -z "${LD:-}" ]; then
  if command -v ld.lld >/dev/null; then LD=ld.lld
  else LD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld"; fi
fi
link() {
  case "$LD" in *rust-lld) "$LD" -flavor gnu "$@";; *) "$LD" "$@";; esac
}
for source in start.S main.c lib.c boot.S; do
  "$CC" --target=riscv32 -march=rv32i -mabi=ilp32 -Os -ffreestanding \
    -fno-builtin -nostdlib -ffunction-sections -fdata-sections -Wall -Wextra \
    -c "$here/$source" -o "$out/${source%.*}.o"
done
link -T "$here/link.ld" --gc-sections -o "$out/firmware.elf" "$out/start.o" "$out/main.o" "$out/lib.o"
link -T "$here/boot.ld" -o "$out/boot.elf" "$out/boot.o"
"$OBJCOPY" -O binary "$out/firmware.elf" "$out/firmware.bin"
"$OBJCOPY" -O binary "$out/boot.elf" "$out/boot.bin"
python3 - "$out" "$here/.." <<'PY'
import sys
from pathlib import Path
out, design = map(Path, sys.argv[1:])
boot = (out/'boot.bin').read_bytes()
payload = (out/'firmware.bin').read_bytes()
assert len(boot) <= 256 and len(payload) <= 8192-256
image = boot.ljust(256,b'\0') + payload
image = image.ljust(8192,b'\0')
for lane in range(4):
    (design/f'fw{lane}.hex').write_text(''.join(f'{v:02x}\n' for v in image[lane::4]))
print(f'boot: {len(boot)} bytes, SDRAM payload: {len(payload)} bytes, EBR image: {len(image)} bytes')
PY
