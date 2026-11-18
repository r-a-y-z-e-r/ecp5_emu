#!/usr/bin/env python3
"""Refresh verified notice texts in target/packaging/notices/ and Chrome evidence PNGs."""
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import notices

ROOT = notices.ROOT
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


def capture(record):
    notices.fetch_verified(record)
    screenshot = record.get("screenshot")
    if not screenshot:
        return None
    output = ROOT / "licenses/evidence" / screenshot
    with tempfile.TemporaryDirectory(prefix="chrome-license-", dir=ROOT / "target/packaging") as profile:
        result = subprocess.run(
            [
                CHROME,
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                f"--user-data-dir={profile}",
                "--hide-scrollbars",
                "--window-size=1280,1600",
                f"--screenshot={output}",
                record["url"],
            ],
            capture_output=True,
            text=True,
            timeout=90,
        )
        if result.returncode or not output.is_file():
            print(f"CAPTURE FAILED {screenshot}: exit {result.returncode}; {result.stderr}", flush=True)
            return screenshot, record["url"], ""
    captured = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    print(f"Captured {screenshot} {captured}", flush=True)
    return screenshot, record["url"], captured


def main():
    (ROOT / "target/packaging").mkdir(parents=True, exist_ok=True)
    records = notices.load_sources()
    evidence_dir = ROOT / "licenses/evidence"
    path = evidence_dir / "urls.tsv"
    existing = {}
    if path.is_file():
        for line in path.read_text().splitlines()[1:]:
            if not line.strip():
                continue
            parts = line.split("\t")
            if len(parts) == 3 and parts[2] and (evidence_dir / parts[0]).is_file():
                existing[parts[0]] = (parts[0], parts[1], parts[2])
    pending = [r for r in records if not (r.get("screenshot") and r["screenshot"] in existing)]
    for r in pending:
        notices.fetch_verified(r)
    rows = []
    # Run sequentially so headless Chrome instances do not contend on Crashpad locks.
    for record in pending:
        row = capture(record)
        if row is not None:
            rows.append(row)
    merged = dict(existing)
    for row in rows:
        if row[2] and (evidence_dir / row[0]).is_file():
            merged[row[0]] = row
    ordered = []
    seen = set()
    for name in ["ulx3s_license.png", "inter_license.png"] + [
        r["screenshot"] for r in records if r.get("screenshot")
    ]:
        if name in merged and name not in seen:
            ordered.append("\t".join(merged[name]))
            seen.add(name)
    path.write_text("file\turl\tcaptured_utc\n" + "\n".join(ordered) + "\n")
    if any(not row[2] for row in rows):
        raise SystemExit("Headless Chrome evidence incomplete; blank captured_utc means no screenshot was captured.")


if __name__ == "__main__":
    main()
