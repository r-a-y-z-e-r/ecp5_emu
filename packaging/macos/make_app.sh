#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || { echo 'Requires macOS arm64' >&2; exit 1; }
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
APP="$ROOT/target/dist/ECP5 Emu.app"
ZIP="$ROOT/target/dist/ecp5_emu-$VERSION-macos-arm64.zip"
# Use the libtrellis built by packaging/build_trellis.sh only if that build finished.
if [[ -f "$ROOT/target/packaging/trellis-prefix/lib/trellis/libtrellis.dylib" ]]; then
    PREFIX=${ECP5_TRELLIS_PREFIX:-$ROOT/target/packaging/trellis-prefix}
    SOURCE=${ECP5_TRELLIS_SOURCE:-$ROOT/target/packaging/prjtrellis}
else
    PREFIX=${ECP5_TRELLIS_PREFIX:-$(brew --prefix)}
    SOURCE=${ECP5_TRELLIS_SOURCE:-$HOME/FPGA/prjtrellis}
fi
DB=${ECP5_DB:-$PREFIX/share/trellis/database}
if [[ -z ${ECP5_DB+x} && ! -f "$DB/ECP5/LFE5U-85F/tilegrid.json" ]]; then
    DB="$(brew --prefix)/share/trellis/database"
fi
cargo build -p ecp5_emu -p ecp5_gui --release --locked
# Boost headers come from Homebrew whichever libtrellis is used (CPATH is read by clang).
CPATH="$(brew --prefix)/include${CPATH:+:$CPATH}" \
    python3 tools/build_image_reader.py --trellis "$SOURCE" --prefix "$PREFIX"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin"
cp target/release/ecp5_emu target/release/ecp5_gui target/trellis_image_reader "$APP/Contents/MacOS/"
python3 packaging/toolchain/stage.py "$APP/Contents/Resources" --platform macos
python3 packaging/examples.py --destination "$APP/Contents/Resources/examples" --manifest "$APP/Contents/Resources/examples-manifest.json"
python3 packaging/resources.py --database "$DB" --trellis "$SOURCE" --destination "$APP/Contents/Resources" --target aarch64-apple-darwin
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE"
cp "$ROOT/assets/icon/ecp5_emu.icns" "$APP/Contents/Resources/ecp5_emu.icns"
TOUR="$APP/Contents/Resources/tour/blinky_cycle"
mkdir -p "$TOUR/prebuilt"
cp "$TOUR_SOURCE/ecp5.toml" "$TOUR_SOURCE/top.v" "$TOUR_SOURCE/top.lpf" "$TOUR/"
cp "$TOUR_PREBUILT/top.config" "$TOUR_PREBUILT/top_routed.json" "$TOUR/prebuilt/"
cat > "$APP/Contents/MacOS/ECP5 Emu" <<'SH'
#!/bin/sh
set -eu
CONTENTS=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
: "${ECP5_DB=$CONTENTS/Resources/database}"
: "${ECP5_IMAGE_READER=$CONTENTS/MacOS/trellis_image_reader}"
: "${ECP5_BOARD_MODEL=$CONTENTS/Resources/ulx3s_v3.0.8.glb}"
: "${ECP5_ARCHITECTURE=$CONTENTS/Resources/architecture}"
export ECP5_DB ECP5_IMAGE_READER ECP5_BOARD_MODEL ECP5_ARCHITECTURE
. "$CONTENTS/Resources/toolchain/environment.sh"
exec "$CONTENTS/MacOS/ecp5_gui" "$@"
SH
cat > "$APP/Contents/Resources/bin/ecp5_emu" <<'SH'
#!/bin/sh
set -eu
CONTENTS=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
: "${ECP5_DB=$CONTENTS/Resources/database}"
: "${ECP5_IMAGE_READER=$CONTENTS/MacOS/trellis_image_reader}"
: "${ECP5_BOARD_MODEL=$CONTENTS/Resources/ulx3s_v3.0.8.glb}"
: "${ECP5_ARCHITECTURE=$CONTENTS/Resources/architecture}"
export ECP5_DB ECP5_IMAGE_READER ECP5_BOARD_MODEL ECP5_ARCHITECTURE
. "$CONTENTS/Resources/toolchain/environment.sh"
case ${1-} in
    device) shift; exec "$CONTENTS/MacOS/ecp5_emu" device --architecture "$CONTENTS/Resources/architecture" "$@" ;;
    load|emulate) command=$1; shift; if [ "$#" -gt 0 ]; then
        design=$1; shift
        exec "$CONTENTS/MacOS/ecp5_emu" "$command" "$design" --architecture "$CONTENTS/Resources/architecture" "$@"
    fi ;;
esac
exec "$CONTENTS/MacOS/ecp5_emu" "$@"
SH
chmod +x "$APP/Contents/MacOS/"* "$APP/Contents/Resources/bin/ecp5_emu"
python3 - "$APP" "$VERSION" <<'PY'
import plistlib,sys
from pathlib import Path
app,version=sys.argv[1:]
with (Path(app)/'Contents/Info.plist').open('wb') as f:
    plistlib.dump({'CFBundleIdentifier':'dev.ecp5emu.gui', 'CFBundleName':'ECP5 Emu',
                  'CFBundleDisplayName':'ECP5 Emu', 'CFBundleExecutable':'ECP5 Emu',
                  'CFBundleIconFile':'ecp5_emu',
                  'CFBundlePackageType':'APPL', 'CFBundleVersion':version,
                  'CFBundleShortVersionString':version, 'NSHighResolutionCapable':True,
                  'LSMinimumSystemVersion':'15.0'},f)
PY
python3 packaging/macos/bundle_dylibs.py "$APP" --check-build-paths "$ROOT"
python3 packaging/toolchain/stage.py "$APP/Contents/Resources" --platform macos --finalize
# Check again after final manifests/signatures exist, before signing the outer bundle.
python3 packaging/check_build_paths.py "$APP" --root "$ROOT"
codesign --force -s - "$APP"
codesign --verify --deep --strict "$APP"
rm -f "$ZIP"
ditto -c -k --norsrc --noextattr --noacl --noqtn --keepParent "$APP" "$ZIP"
"$ROOT/packaging/macos/check_zip.sh" "$ZIP"
du -sh "$APP" "$ZIP"
