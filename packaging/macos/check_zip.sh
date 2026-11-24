#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "Usage: $(basename "$0") ZIP [BUNDLE_NAME]" >&2
    exit 2
fi

ZIP=$1
BUNDLE_NAME=${2:-ECP5 Emu.app}

if [[ ! -f "$ZIP" ]]; then
    echo "Zip archive not found: $ZIP" >&2
    exit 1
fi

TMP_DIR=$(mktemp -d)
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ENTRIES_FILE="$TMP_DIR/entries.txt"
zipinfo -1 "$ZIP" > "$ENTRIES_FILE"

BAD_ENTRIES=$(awk '
    {
        path = $0
        sub(/\/$/, "", path)
        n = split(path, parts, "/")
        last = (n > 0 ? parts[n] : "")
        if (path ~ /(^|\/)__MACOSX(\/|$)/ || last ~ /^\._/) {
            print $0
        }
    }
' "$ENTRIES_FILE")

if [[ -n "$BAD_ENTRIES" ]]; then
    BAD_COUNT=$(printf '%s\n' "$BAD_ENTRIES" | wc -l | tr -d ' ')
    echo "FAIL: $ZIP contains $BAD_COUNT forbidden AppleDouble (._*) or __MACOSX/ entries:" >&2
    printf '%s\n' "$BAD_ENTRIES" | sed -n '1,10p' >&2
    exit 1
fi

UNPACK_DIR="$TMP_DIR/unpack"
mkdir -p "$UNPACK_DIR"
unzip -q "$ZIP" -d "$UNPACK_DIR"

APP_PATH="$UNPACK_DIR/$BUNDLE_NAME"
if [[ ! -d "$APP_PATH" ]]; then
    echo "FAIL: unpacked archive does not contain bundle: $BUNDLE_NAME" >&2
    exit 1
fi

if ! codesign --verify --deep --strict "$APP_PATH"; then
    echo "FAIL: codesign --verify --deep --strict failed on unpacked $BUNDLE_NAME" >&2
    exit 1
fi

ENTRY_COUNT=$(wc -l < "$ENTRIES_FILE" | tr -d ' ')
echo "PASS: $ZIP ($ENTRY_COUNT entries checked, no ._* or __MACOSX/ entries, codesign --verify --deep --strict passed on $BUNDLE_NAME)"
