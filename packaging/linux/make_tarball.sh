#!/usr/bin/env bash
# Linux x86_64 only. Syntax/lint checked on macOS; Linux execution is CI-only.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Requires Linux x86_64' >&2; exit 1; }
: "${OSS_CAD_SUITE_ROOT:?Set OSS_CAD_SUITE_ROOT to extracted OSS CAD Suite 2026-10-22}"
TOUR_SOURCE="$ROOT/examples/ulx3s/blinky_cycle"
TOUR_PREBUILT="$ROOT/target/reference/blinky_cycle"
for FILE in top.config top_routed.json; do
    if [[ ! -f "$TOUR_PREBUILT/$FILE" ]]; then
        printf 'Missing tour input: %s\nBuild it with:\n  examples/ulx3s/build.sh examples/ulx3s/blinky_cycle target/reference/blinky_cycle\n' "$TOUR_PREBUILT/$FILE" >&2
        exit 1
    fi
done
VERSION=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
PREFIX=${ECP5_TRELLIS_PREFIX:-$ROOT/target/packaging/trellis-prefix}
SOURCE=${ECP5_TRELLIS_SOURCE:-$ROOT/target/packaging/prjtrellis}
: "${ECP5_DB:?Set ECP5_DB to the pinned Trellis database}"
NAME="ecp5_emu-$VERSION-linux-x86_64"
DEST="$ROOT/target/dist/$NAME"
cargo build -p ecp5_emu -p ecp5_gui --release --locked
mkdir -p target/packaging
# tools/build_image_reader.py writes a macOS-specific dylib build report;
# compile its identical C++ source/flags directly for ELF instead.
clang++ -std=c++14 -Wall -Wextra -Werror \
    -I "$SOURCE/libtrellis/include" -I "$PREFIX/include" tools/trellis_image_reader.cpp \
    -L "$PREFIX/lib/trellis" -ltrellis -Wl,-rpath,"$PREFIX/lib/trellis" \
    -o target/packaging/trellis_image_reader
rm -rf "$DEST"
mkdir -p "$DEST/bin" "$DEST/libexec"
cp target/release/ecp5_emu target/release/ecp5_gui target/packaging/trellis_image_reader "$DEST/libexec/"
python3 packaging/toolchain/stage.py "$DEST/share" --platform linux
python3 packaging/examples.py --destination "$DEST/share/examples" --manifest "$DEST/share/examples-manifest.json"
python3 packaging/resources.py --database "$ECP5_DB" --trellis "$SOURCE" --destination "$DEST/share" --target x86_64-unknown-linux-gnu
mkdir -p "$DEST/share/icons" "$DEST/share/applications"
cp -R "$ROOT/assets/icon/hicolor" "$DEST/share/icons/"
cat > "$DEST/share/applications/ecp5_emu.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=ECP5 Emu
Comment=An emulator of the Lattice ECP5 LFE5U-85F FPGA that runs the real bitstream from the open toolchain (Yosys → nextpnr-ecp5 → ecppack), with a live view of the die.
Exec=ecp5_gui
Icon=ecp5_emu
Terminal=false
Categories=Development;Electronics;
DESKTOP
TOUR="$DEST/share/tour/blinky_cycle"
mkdir -p "$TOUR/prebuilt"
cp "$TOUR_SOURCE/ecp5.toml" "$TOUR_SOURCE/top.v" "$TOUR_SOURCE/top.lpf" "$TOUR/"
cp "$TOUR_PREBUILT/top.config" "$TOUR_PREBUILT/top_routed.json" "$TOUR/prebuilt/"
for PROGRAM in ecp5_emu ecp5_gui; do
    cat > "$DEST/bin/$PROGRAM" <<'SH'
#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PROGRAM=${0##*/}
: "${ECP5_DB=$ROOT/share/database}"
: "${ECP5_IMAGE_READER=$ROOT/libexec/trellis_image_reader}"
: "${ECP5_BOARD_MODEL=$ROOT/share/ulx3s_v3.0.8.glb}"
: "${ECP5_ARCHITECTURE=$ROOT/share/architecture}"
export ECP5_DB ECP5_IMAGE_READER ECP5_BOARD_MODEL ECP5_ARCHITECTURE
. "$ROOT/share/toolchain/environment.sh"
if [ "$PROGRAM" = ecp5_emu ]; then
    case ${1-} in
        device) shift; exec "$ROOT/libexec/ecp5_emu" device --architecture "$ROOT/share/architecture" "$@" ;;
        load|emulate) command=$1; shift; if [ "$#" -gt 0 ]; then
            design=$1; shift
            exec "$ROOT/libexec/ecp5_emu" "$command" "$design" --architecture "$ROOT/share/architecture" "$@"
        fi ;;
    esac
fi
exec "$ROOT/libexec/$PROGRAM" "$@"
SH
    chmod +x "$DEST/bin/$PROGRAM"
done
python3 packaging/linux/bundle_libraries.py "$DEST"
python3 packaging/toolchain/stage.py "$DEST/share" --platform linux --finalize
cp "$DEST/share/README.md" "$DEST/README.md"
cp "$DEST/share/THIRD_PARTY_NOTICES.txt" "$DEST/THIRD_PARTY_NOTICES.txt"
cp "$ROOT/LICENSE" "$DEST/LICENSE"
python3 packaging/check_build_paths.py "$DEST" --root "$ROOT"
tar -czf "$ROOT/target/dist/$NAME.tar.gz" -C "$ROOT/target/dist" "$NAME"
du -sh "$DEST" "$ROOT/target/dist/$NAME.tar.gz"
