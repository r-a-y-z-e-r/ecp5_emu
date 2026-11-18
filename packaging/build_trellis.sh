#!/usr/bin/env bash
# Build the native runtime dependency at the revision documented in TOOLCHAIN.md.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
REV=a31ff7042e712c0e2ad217054116ce2c4995f5c7
SOURCE="$ROOT/target/packaging/prjtrellis"
PREFIX="$ROOT/target/packaging/trellis-prefix"
mkdir -p "$SOURCE"
# rust-cache can leave a partial .git; Git would then walk up to the outer repository.
# Only use SOURCE when it is the top level of its own repository.
if [[ "$(git -C "$SOURCE" rev-parse --show-toplevel 2>/dev/null || true)" != "$(cd "$SOURCE" && pwd -P)" ]]; then
    echo "Resetting Trellis source, build and prefix: source is not the top level of its own Git repository."
    rm -rf "$SOURCE" "$ROOT/target/packaging/trellis-build" "$PREFIX"
    mkdir -p "$SOURCE"
    git init -q "$SOURCE"
fi
# Three tries: a fetch from GitHub can fail on a transient network error.
for TRY in 1 2 3; do
    git -C "$SOURCE" fetch --depth 1 https://github.com/YosysHQ/prjtrellis "$REV" && break
    [[ $TRY == 3 ]] && exit 1
    sleep $((TRY * 5))
done
git -C "$SOURCE" checkout --detach FETCH_HEAD
if [[ "$(git -C "$SOURCE" rev-parse HEAD)" != "$REV" ]]; then
    echo "Error: Trellis checkout does not match expected revision $REV." >&2
    exit 1
fi
# Ninja when it is installed (CI installs it), else CMake's default generator.
GENERATOR=()
if command -v ninja >/dev/null; then GENERATOR=(-G Ninja); fi
cmake -S "$SOURCE/libtrellis" -B "$ROOT/target/packaging/trellis-build" "${GENERATOR[@]}" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" -DCMAKE_INSTALL_LIBDIR=lib \
    -DBUILD_PYTHON=OFF -DBUILD_SHARED=ON -DBUILD_ECPBRAM=OFF \
    -DBUILD_ECPPACK=OFF -DBUILD_ECPUNPACK=OFF -DBUILD_ECPPLL=OFF -DBUILD_ECPMULTI=OFF
cmake --build "$ROOT/target/packaging/trellis-build" --parallel "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
cmake --install "$ROOT/target/packaging/trellis-build"
