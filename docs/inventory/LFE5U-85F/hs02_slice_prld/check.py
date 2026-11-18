"""Regenerate in memory, synthesize both HS02 designs, and verify every probe FF.
Try the requested default flow; on the known ABC9 PRLD init failure use the blackbox-interface workaround.
Writes temporary JSON only in this directory, then removes it. Does not route.
Usage: PYTHONDONTWRITEBYTECODE=1 python3 check.py
"""
import collections
import json
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
from hs02_gen import SETTINGS, generate
from synth import script

ROOT = Path(__file__).resolve().parent


def main():
    print(subprocess.check_output(["yosys", "-V"], text=True).strip())
    out = ROOT / ".check.json"
    try:
        for once in (False, True):
            v = "hs02" + ("s" if once else "") + ".v"
            assert (ROOT/v).read_text() == generate(once), f"regenerate {v}"
            result = subprocess.run(["yosys", "-q", "-p", f"read_verilog {v}; synth_ecp5 -top top -json .check.json"], cwd=ROOT, text=True, capture_output=True)
            if result.returncode:
                print(v, "default synth_ecp5: FAILED", result.stderr.strip(), flush=True)
                if "Failed to get a constant init value for \\Q: \\M" not in result.stderr:
                    raise RuntimeError(result.stderr)
                print(v, "retrying with FF library blackbox interface; hardware parameters unchanged", flush=True)
                subprocess.run(["yosys", "-q", "-p", script(v, ".check.json")], cwd=ROOT, check=True)
            cells = json.loads(out.read_text())["modules"]["top"]["cells"]
            assert not any(c["type"].startswith("$") for c in cells.values()), "unmapped internal cells"
            for i, setting in enumerate(SETTINGS):
                cell = cells[f"ff{i:02}"]
                assert cell["type"] == "TRELLIS_FF"
                for key, value in zip(("LSRMODE", "SRMODE", "REGSET", "CEMUX"), setting):
                    assert cell["parameters"][key].rstrip() == value, (i, key, cell["parameters"])
                lut = cells[f"di_lut{i:02}"]
                assert lut["type"] == "TRELLIS_COMB"
                assert not lut["connections"].get("M"), "LUT M would conflict with the paired preload FF"
                assert cell["connections"]["DI"] == lut["connections"]["F"]
                assert cell["connections"]["DI"] != cell["connections"]["M"]
                assert int(lut["parameters"]["INITVAL"], 2) == 0xAAAA
                for pin in ("DI", "M", "LSR", "CE"):
                    assert len(cell["connections"][pin]) == 1
                    assert isinstance(cell["connections"][pin][0], int), (i, pin)
            counts = dict(sorted(collections.Counter(c["type"] for c in cells.values()).items()))
            print(v, counts, "24 PRLD + 8 LSR; all FF parameters and traffic pins: PASS", flush=True)
            out.unlink()
    finally:
        out.unlink(missing_ok=True)
    subprocess.run([sys.executable, "-B", str(ROOT/"hs02_analyse.py"), "--selftest"], check=True)


if __name__ == "__main__":
    main()
