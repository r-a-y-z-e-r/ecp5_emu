"""Analyse HB11 ULX3S UART logs: DP16KD READBEFOREWRITE at narrow widths 1, 2, 4.
Usage:
  python3 hb11_analyse.py observations.log [s]
  python3 hb11_analyse.py --selftest

Each full line is "R" + 16 hex (win_lfsr) + 128 * 14 hex (128 samples, 56 bits
each, sample 0 in the MSBs). Lines of any other length (cut by the serial
reading window) are ignored.

For each EBR and port, each window is simulated under three write-cycle models:
  - RBW:          on a write, the output takes the old word at the written address
  - NORMAL:       on a write, the output holds its previous value
  - WRITETHROUGH: on a write, the output takes the written data
On a read cycle (we = 0) all three models output the addressed word. For OUTREG
(e3, e5, with OCE = 1), one output register stage follows that latch, so
sample 1 reveals latch(0) and sample k + 2 reflects traffic(L_k) for k = 0..125.
"""
import ast
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hb11_gen import (  # noqa: E402
    EBRS,
    INIT_MEMS,
    PAD,
    RECORD,
    SAMPLE_BITS,
    SAMPLES,
    SEED,
    cell_coords,
    lfsr_step,
    traffic_fields,
)

MODELS = ("RBW", "NORMAL", "WRITETHROUGH")
HEX_SAMPLES = SAMPLES * (SAMPLE_BITS // 4)
LINE_LEN = 1 + 16 + HEX_SAMPLES


def parse_log(path):
    """Parse run.py log lines, returning (loads, distinct_sorted_lines)."""
    loads = []
    all_lines = set()
    with open(path, encoding="utf-8") as f:
        for raw in f:
            if "distinct=" not in raw:
                continue
            items = ast.literal_eval(raw.partition("distinct=")[2].strip())
            valid = {
                x for x in items
                if isinstance(x, str)
                and len(x) == LINE_LEN
                and x.startswith("R")
                and all(c in "0123456789abcdefABCDEF" for c in x[1:])
            }
            loads.append(valid)
            all_lines |= valid
    return loads, sorted(all_lines)


def decode_line(line):
    """Decode one valid 'R...' line into (lfsr0, list_of_sample_dicts)."""
    lfsr0 = int(line[1:17], 16)
    body = line[17:]
    nibbles = SAMPLE_BITS // 4
    samples = []
    for i in range(SAMPLES):
        val = int(body[nibbles * i:nibbles * (i + 1)], 16)
        pos = SAMPLE_BITS
        row = {}
        for name, port, w in RECORD:
            pos -= w
            row[(name, port)] = (val >> pos) & ((1 << w) - 1)
        assert pos == PAD
        samples.append(row)
    return lfsr0, samples


def encode_line(lfsr0, samples):
    """Encode (lfsr0, list_of_sample_dicts) into an 'R...' line."""
    nibbles = SAMPLE_BITS // 4
    chunks = [f"R{lfsr0:016X}"]
    for row in samples:
        val = 0
        pos = SAMPLE_BITS
        for name, port, w in RECORD:
            pos -= w
            val |= (row[(name, port)] & ((1 << w) - 1)) << pos
        chunks.append(f"{val:0{nibbles}X}")
    return "".join(chunks)


def init_bit_array(ebr_name, known):
    """Create the 18432-bit EBR array from INIT_MEMS (if known) or all None."""
    if not known:
        return [None] * (2048 * 9)
    return [(word >> b) & 1 for word in INIT_MEMS[ebr_name] for b in range(9)]


def read_word(mem, width, addr4, port):
    """Read width bits via EBR_ADDRESSING.md coordinates, or None if any bit unknown."""
    val = 0
    for b in range(width):
        w9, b9 = cell_coords(width, addr4, b, port)
        bit = mem[w9 * 9 + b9]
        if bit is None:
            return None
        val |= bit << b
    return val


def write_word(mem, width, addr4, port, val):
    """Write width bits via EBR_ADDRESSING.md coordinates."""
    for b in range(width):
        w9, b9 = cell_coords(width, addr4, b, port)
        mem[w9 * 9 + b9] = (val >> b) & 1


def score_windows(windows, once=False):
    """Score every (ebr, port) across decoded windows under RBW, NORMAL, WRITETHROUGH."""
    stats = {}
    for name, wa, wb, wma, wmb, regmode in EBRS:
        for port, width, wm in (("a", wa, wma), ("b", wb, wmb)):
            stats[(name, port)] = {
                "width": width,
                "regmode": regmode,
                "configured": wm,
                "known_writes": 0,
                "known_reads": 0,
                "write_miss": {m: 0 for m in MODELS},
                "read_miss": {m: 0 for m in MODELS},
            }

    offset_for = {name: (1 if regmode == "OUTREG" else 0) for name, *_, regmode in EBRS}

    for lfsr0, samples in windows:
        lfsr_seq = []
        cur = lfsr0
        for _ in range(SAMPLES):
            lfsr_seq.append(cur)
            cur = lfsr_step(cur)

        for name, wa, wb, *_ in EBRS:
            offset = offset_for[name]
            n_edges = SAMPLES - 1 - offset
            for port, width in (("a", wa), ("b", wb)):
                st = stats[(name, port)]
                mem = init_bit_array(name, known=once)
                pidx = 0 if port == "a" else 1
                init_obs = samples[offset][(name, port)]
                latches = {m: init_obs for m in MODELS}
                mask = (1 << width) - 1

                for k in range(n_edges):
                    addr, data_full, we = traffic_fields(lfsr_seq[k])[pidx]
                    data = data_full & mask
                    obs = samples[k + 1 + offset][(name, port)]
                    old_word = read_word(mem, width, addr, port)

                    if we == 0:
                        if old_word is not None:
                            st["known_reads"] += 1
                            for m in MODELS:
                                st["read_miss"][m] += (old_word ^ obs).bit_count()
                                latches[m] = old_word
                        else:
                            write_word(mem, width, addr, port, obs)
                            for m in MODELS:
                                latches[m] = obs
                    else:
                        write_word(mem, width, addr, port, data)
                        if old_word is not None:
                            st["known_writes"] += 1
                            expected = {
                                "RBW": old_word,
                                "NORMAL": latches["NORMAL"],
                                "WRITETHROUGH": data,
                            }
                            for m in MODELS:
                                exp = expected[m]
                                if exp is not None:
                                    st["write_miss"][m] += (exp ^ obs).bit_count()
                                latches[m] = exp
                        else:
                            latches["RBW"] = None
                            latches["WRITETHROUGH"] = data
    return stats


def format_report(stats):
    lines = [
        "EBR.P  W  REGMODE  CONF             K_WR  K_RD |  RBW(wr/rd)    NORM(wr/rd)      WT(wr/rd)"
    ]
    for name, *_ in EBRS:
        for port in ("a", "b"):
            st = stats[(name, port)]
            wm, rm = st["write_miss"], st["read_miss"]
            lines.append(
                f"{name}.{port.upper()}  {st['width']:>2}  {st['regmode']:<6}   {st['configured']:<15} "
                f"{st['known_writes']:>4}  {st['known_reads']:>4} | "
                f"{wm['RBW']:>5}/{rm['RBW']:<5}   "
                f"{wm['NORMAL']:>5}/{rm['NORMAL']:<5}   "
                f"{wm['WRITETHROUGH']:>5}/{rm['WRITETHROUGH']:<5}"
            )
    return "\n".join(lines)


def _simulate_synthetic_lines(port_modes, n_windows=4, once=False):
    """Simulate all 8 EBRs from configuration and emit synthetic UART lines."""
    mems = {name: init_bit_array(name, known=True) for name, *_ in EBRS}
    latches = {(name, p): 0x15 & ((1 << w) - 1) for name, p, w in RECORD}
    out_regs = {(name, p): 0x0A & ((1 << w) - 1) for name, p, w in RECORD}
    regmodes = {name: regmode for name, *_, regmode in EBRS}
    widths = {(name, p): w for name, p, w in RECORD}

    lfsr = SEED
    lines = []
    # In free-running mode, advance a few hundred clocks between windows like UART send time.
    gaps = [0] if once else [0, 311, 277, 353][:n_windows]
    for gap in gaps:
        for _ in range(gap):
            lfsr = _sim_step(lfsr, mems, latches, out_regs, regmodes, widths, port_modes)
        win_lfsr = lfsr
        win_samples = []
        for _ in range(SAMPLES):
            win_samples.append({
                (name, p): out_regs[(name, p)] if regmodes[name] == "OUTREG" else latches[(name, p)]
                for name, p, _ in RECORD
            })
            lfsr = _sim_step(lfsr, mems, latches, out_regs, regmodes, widths, port_modes)
        lines.append(encode_line(win_lfsr, win_samples))
    return lines


def _sim_step(lfsr, mems, latches, out_regs, regmodes, widths, port_modes):
    fields = traffic_fields(lfsr)
    for name, *_ in EBRS:
        mem = mems[name]
        for pidx, port in enumerate(("a", "b")):
            key = (name, port)
            w = widths[key]
            addr, data_full, we = fields[pidx]
            data = data_full & ((1 << w) - 1)
            if regmodes[name] == "OUTREG":
                out_regs[key] = latches[key]
            old_word = read_word(mem, w, addr, port)
            if we == 0:
                latches[key] = old_word
            else:
                write_word(mem, w, addr, port, data)
                mode = port_modes[key]
                if mode == "READBEFOREWRITE" or mode == "RBW":
                    latches[key] = old_word
                elif mode == "WRITETHROUGH":
                    latches[key] = data
                elif mode == "NORMAL":
                    pass
                else:
                    raise ValueError(mode)
    return lfsr_step(lfsr)


def selftest():
    """Verify RBW synthetic lines score 0 RBW mismatches and many NORMAL/WT mismatches."""
    all_rbw = {(name, p): "RBW" for name, p, _ in RECORD}
    for once_flag, n_win in ((True, 1), (False, 4)):
        syn_lines = _simulate_synthetic_lines(all_rbw, n_windows=n_win, once=once_flag)
        # Exercise parse_log with a truncated line that must be ignored.
        log_rows = [
            f"load 1 at 2026-01-01T00:00:00Z: loader_done=True lines={len(syn_lines) + 1} "
            f"distinct={syn_lines + [syn_lines[0][:100]]!r}\n"
        ]
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as tf:
            tf.writelines(log_rows)
            log_path = tf.name
        try:
            _, parsed_lines = parse_log(log_path)
        finally:
            os.remove(log_path)
        assert parsed_lines == sorted(syn_lines)
        windows = [decode_line(x) for x in parsed_lines]
        stats = score_windows(windows, once=once_flag)
        for key, st in stats.items():
            assert st["known_writes"] > 10, f"{key} too few known writes: {st['known_writes']}"
            assert st["known_reads"] > 10, f"{key} too few known reads: {st['known_reads']}"
            assert st["write_miss"]["RBW"] == 0, f"{key} RBW write miss: {st['write_miss']}"
            assert st["read_miss"]["RBW"] == 0, f"{key} RBW read miss: {st['read_miss']}"
            assert st["write_miss"]["NORMAL"] > 10, f"{key} NORMAL too few miss: {st['write_miss']}"
            assert st["write_miss"]["WRITETHROUGH"] > 10, f"{key} WT too few miss: {st['write_miss']}"

    # Also check mixed control modes (e6.A=NORMAL, e6.B=WRITETHROUGH, others RBW).
    mixed = {(name, "a"): wma for name, _, _, wma, _, _ in EBRS}
    mixed.update({(name, "b"): wmb for name, _, _, _, wmb, _ in EBRS})
    syn_mixed = _simulate_synthetic_lines(mixed, n_windows=3, once=False)
    stats_mixed = score_windows([decode_line(x) for x in syn_mixed], once=False)
    assert stats_mixed[("e6", "a")]["write_miss"]["NORMAL"] == 0
    assert stats_mixed[("e6", "a")]["write_miss"]["RBW"] > 10
    assert stats_mixed[("e6", "b")]["write_miss"]["WRITETHROUGH"] == 0
    assert stats_mixed[("e6", "b")]["write_miss"]["RBW"] > 10
    for name in ("e0", "e1", "e2", "e3", "e4", "e5", "e7"):
        for port in ("a", "b"):
            assert stats_mixed[(name, port)]["write_miss"]["RBW"] == 0
    print("selftest passed (s-mode all-RBW, free-running all-RBW, mixed control)")


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--selftest":
        selftest()
        return
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "s"):
        raise SystemExit("Usage: python3 hb11_analyse.py LOG [s] | --selftest")

    log_path = sys.argv[1]
    once = len(sys.argv) == 3 and sys.argv[2] == "s"
    loads, lines = parse_log(log_path)
    print(f"loads: {len(loads)} full windows per load: {[len(l) for l in loads]} distinct: {len(lines)}")
    if not lines:
        raise SystemExit("no valid R lines found")
    windows = [decode_line(x) for x in lines]
    stats = score_windows(windows, once=once)
    print(format_report(stats))


if __name__ == "__main__":
    main()
