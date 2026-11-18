"""Keep bitstream decoding in libtrellis; the digital simulator lives in Rust."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import sys

for directory in (os.environ.get("ECP5_TRELLIS_LIB"), "/opt/homebrew/lib/trellis", "/usr/local/lib/trellis"):
    if directory:
        sys.path.append(directory)

try:
    import pytrellis
except ImportError as error:
    raise SystemExit("pytrellis is unavailable; set ECP5_TRELLIS_LIB and use a compatible Python via ECP5_PYTHON") from error

raw_cram = len(sys.argv) > 3 and sys.argv[3] == "cram"
if raw_cram:
    catalog = json.loads(Path(sys.argv[4]).read_text())
    for source in catalog["source_files"]:
        path = Path(sys.argv[1]) / source["path"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != source["sha256"]:
            raise SystemExit(f"decode catalogue is stale: {source['path']}")

with contextlib.redirect_stdout(sys.stderr):
    pytrellis.load_database(sys.argv[1])
    if not raw_cram or Path(sys.argv[2]).suffix.lower() == ".bit":
        chip = pytrellis.Bitstream.read_bit(sys.argv[2]).deserialise_chip()
        native_config = pytrellis.ChipConfig.from_chip(chip)
    else:
        native_config = pytrellis.ChipConfig.from_string(Path(sys.argv[2]).read_text())
        chip = native_config.to_chip()
    if not raw_cram:
        config = native_config.to_string()
if not raw_cram:
    sys.stdout.write(config)
else:
    if chip.info.name != catalog["device"] or chip.cram.frames() != catalog["frames"] or chip.cram.bits() != catalog["bits_per_frame"]:
        raise SystemExit("chip and decode catalogue dimensions/device differ")
    if chip.cram.bits() % 8:
        raise SystemExit("CRAM frame width is not byte aligned")
    get_bit = chip.cram.bit
    data = bytearray()
    for frame in range(chip.cram.frames()):
        for offset in range(0, chip.cram.bits(), 8):
            data.append(sum(int(get_bit(frame, offset + bit)) << bit for bit in range(8)))
    json.dump(dict(schema_version=1, device=chip.info.name, frames=chip.cram.frames(),
                   bits_per_frame=chip.cram.bits(), encoding="frame_byte_lsb0", cram_hex=data.hex(),
                   bram_data={str(k): list(v) for k, v in native_config.bram_data.items()},
                   origin="bitstream" if Path(sys.argv[2]).suffix.lower() == ".bit" else "text_config"), sys.stdout)
