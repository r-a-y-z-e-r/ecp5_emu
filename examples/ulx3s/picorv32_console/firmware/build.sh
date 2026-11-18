#!/bin/sh
# Builds the firmware and writes its image as four byte lanes, fw0.hex ..
# fw3.hex, next to top.v; then ../../swap.sh puts it into the built design
# without place and route.
# Needs a clang with the RISC-V target (Homebrew LLVM has one, Apple's clang
# does not) and ld.lld; without ld.lld on PATH, the rust-lld that ships with
# Rust is used.
set -eu
cd "$(dirname "$0")"
CC=${CC:-/opt/homebrew/opt/llvm/bin/clang}
OBJCOPY=${OBJCOPY:-/opt/homebrew/opt/llvm/bin/llvm-objcopy}
if [ -z "${LD:-}" ]; then
  if command -v ld.lld >/dev/null; then
    LD=ld.lld
  else
    LD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld -flavor gnu"
  fi
fi
FLAGS="--target=riscv32 -march=rv32i -mabi=ilp32 -Os -ffreestanding -fno-builtin -nostdlib -ffunction-sections -fdata-sections -Wall -Wextra"
mkdir -p build
for source in start.S main.c lib.c; do
  $CC $FLAGS -c "$source" -o "build/${source%.*}.o"
done
$LD -T link.ld --gc-sections -o build/firmware.elf build/start.o build/main.o build/lib.o
$OBJCOPY -O binary build/firmware.elf build/firmware.bin
size=$(wc -c < build/firmware.bin)
echo "firmware: $size bytes"
# Byte k of the image goes to lane k % 4, word k / 4; the rest is zero.
od -An -v -tx1 build/firmware.bin | awk '
  { for (i = 1; i <= NF; i++) byte[n++] = $i }
  END {
    for (lane = 0; lane < 4; lane++) {
      file = "../fw" lane ".hex"
      printf "" > file
      for (word = 0; word < 2048; word++) {
        k = 4 * word + lane
        print (k < n ? byte[k] : "00") > file
      }
      close(file)
    }
  }'
