#!/usr/bin/env python3
"""Add the pinned mdBook theme notices to the build output, never site sources."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import notices

output = Path(sys.argv[1])
if not (output / "index.html").is_file():
    raise SystemExit(f"No built site at {output}")
with (output / "THIRD_PARTY_NOTICES.txt").open("w") as stream:
    stream.write(
        "mdBook v0.5.4 and bundled theme assets\n"
        "Source: https://github.com/rust-lang/mdBook/tree/v0.5.4\n"
        "Original MPL theme source: https://github.com/rust-lang/mdBook/tree/v0.5.4/crates/mdbook-html/front-end\n"
        "See licenses/evidence/urls.tsv in the ECP5 Emu source for provenance.\n\n"
    )
    for path in notices.ensure_notices("mdbook-"):
        stream.write(f"--- {path.name} ---\n{path.read_text()}\n\n")
