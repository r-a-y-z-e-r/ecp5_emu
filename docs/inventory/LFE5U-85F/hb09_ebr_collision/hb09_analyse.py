"""Analyse HB09 ULX3S UART logs: DP16KD same-clock port collision behaviour.
Usage:
  python3 hb09_analyse.py observations.log [s]
  python3 hb09_analyse.py --selftest

Each full line is "R" + 16 hex (win_lfsr) + 128 * 31 hex (128 samples, 124 bits
each, sample 0 in the MSBs). Lines of any other length (cut by the serial
reading window) are ignored.

For each EBR and port, each clock edge is classified into one of five cases:
  - clean_rd:  port reads while the other port reads or addresses another word
  - clean_wr:  port writes while the other port addresses another word
  - rd_wr_col: port reads while the other port writes the same word
  - wr_wr_col: port writes while the other port writes the same word
  - wr_rd_col: port writes while the other port reads the same word

For OUTREG (e7, with OCE = 1), sample k + 2 reflects edge k (k = 0..125); for
NOREG (e0..e6), sample k + 1 reflects edge k (k = 0..126).
"""
import ast
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hb09_gen import (  # noqa: E402
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

CASES = ("clean_rd", "clean_wr", "rd_wr_col", "wr_wr_col", "wr_rd_col")
BW_CATS = ("A", "B", "same", "mix", "neither")
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


def init_word_array(ebr_name, width, known):
    """Create the 4-word array from INIT_MEMS (if known) or all None."""
    if not known:
        return [None] * 4
    words9 = INIT_MEMS[ebr_name]
    out = []
    for addr in range(4):
        val = 0
        for b in range(width):
            w9, b9 = cell_coords(width, addr, b)
            val |= ((words9[w9] >> b9) & 1) << b
        out.append(val)
    return out


def _classify_case(we_p, w_p, we_q, w_q):
    if we_p == 0:
        return "rd_wr_col" if (w_p == w_q and we_q == 1) else "clean_rd"
    if w_p != w_q:
        return "clean_wr"
    return "wr_wr_col" if we_q == 1 else "wr_rd_col"


def _classify_bw_read(obs, d_a, d_b, mask):
    if d_a == d_b:
        return "same" if obs == d_a else "neither"
    if obs == d_a:
        return "A"
    if obs == d_b:
        return "B"
    if ((obs ^ d_a) & (obs ^ d_b) & mask) == 0:
        return "mix"
    return "neither"


def _new_expected(case, wmode_p, d_p, d_q):
    if case == "rd_wr_col":
        return d_q
    if case != "clean_rd" and wmode_p == "WRITETHROUGH":
        return d_p
    return None


def _own_expected(we_p, wmode_p, old, prev_out, d_p):
    if we_p == 0 or wmode_p == "READBEFOREWRITE":
        return old
    if wmode_p == "WRITETHROUGH":
        return d_p
    if wmode_p == "NORMAL":
        return prev_out
    raise ValueError(wmode_p)


def score_windows(windows, once=False):
    """Score every EBR/port across decoded windows; return (stats, bw_stats, none_details, total_none)."""
    stats = {}
    bw_stats = {}
    for name, wa, wb, wma, wmb, regmode in EBRS:
        bw_stats[name] = {"total": 0, **{c: 0 for c in BW_CATS}}
        for port, width, wm in (("a", wa, wma), ("b", wb, wmb)):
            stats[(name, port)] = {
                "width": width,
                "regmode": regmode,
                "configured": wm,
                "cases": {
                    c: {
                        "known": 0,
                        "old": 0,
                        "new": 0,
                        "own": 0,
                        "none_words": 0,
                        "none_bits": 0,
                        "new_applicable": (c == "rd_wr_col")
                        or (c != "clean_rd" and wm == "WRITETHROUGH"),
                    }
                    for c in CASES
                },
            }

    none_details = []
    total_none = 0

    for win_idx, (lfsr0, samples) in enumerate(windows):
        lfsr_seq = []
        cur = lfsr0
        for _ in range(SAMPLES):
            lfsr_seq.append(cur)
            cur = lfsr_step(cur)

        for name, wa, wb, wma, wmb, regmode in EBRS:
            width = wa
            assert wa == wb
            mask = (1 << width) - 1
            offset = 1 if regmode == "OUTREG" else 0
            n_edges = SAMPLES - 1 - offset
            mem = init_word_array(name, width, known=once)
            bw_pending = [None] * 4

            for k in range(n_edges):
                (w_a, da_full, we_a), (w_b, db_full, we_b) = traffic_fields(lfsr_seq[k])
                d_a = da_full & mask
                d_b = db_full & mask
                obs_a = samples[k + 1 + offset][(name, "a")]
                obs_b = samples[k + 1 + offset][(name, "b")]
                prev_a = samples[k + offset][(name, "a")]
                prev_b = samples[k + offset][(name, "b")]
                pre_mem = list(mem)

                port_rows = (
                    ("a", w_a, d_a, we_a, wma, obs_a, prev_a, w_b, d_b, we_b),
                    ("b", w_b, d_b, we_b, wmb, obs_b, prev_b, w_a, d_a, we_a),
                )
                for port, w_p, d_p, we_p, wm_p, obs_p, prev_p, w_q, d_q, we_q in port_rows:
                    case = _classify_case(we_p, w_p, we_q, w_q)
                    old = pre_mem[w_p]
                    if old is None:
                        continue

                    cs = stats[(name, port)]["cases"][case]
                    cs["known"] += 1

                    new_val = _new_expected(case, wm_p, d_p, d_q)
                    own_val = _own_expected(we_p, wm_p, old, prev_p, d_p)

                    if obs_p == old:
                        cs["old"] += 1
                    if new_val is not None and obs_p == new_val:
                        cs["new"] += 1
                    if obs_p == own_val:
                        cs["own"] += 1

                    candidates = {old, own_val}
                    if new_val is not None:
                        candidates.add(new_val)
                    if obs_p not in candidates:
                        diff_all = mask
                        for cand in candidates:
                            diff_all &= obs_p ^ cand
                        nbits = diff_all.bit_count()
                        cs["none_words"] += 1
                        cs["none_bits"] += nbits
                        total_none += 1
                        if len(none_details) < 10:
                            none_details.append({
                                "ebr": name,
                                "port": port,
                                "case": case,
                                "win_idx": win_idx,
                                "win_lfsr": lfsr0,
                                "clk": k,
                                "addr": w_p,
                                "old": old,
                                "new_val": new_val,
                                "d_a": d_a,
                                "d_b": d_b,
                                "own": own_val,
                                "out": obs_p,
                                "width": width,
                                "none_bits": nbits,
                            })

                # Update simulated memory array after evaluating edge k.
                if w_a == w_b and we_a == 1 and we_b == 1:
                    mem[w_a] = None
                    bw_pending[w_a] = (d_a, d_b)
                elif w_a == w_b and we_a == 0 and we_b == 0:
                    w = w_a
                    if bw_pending[w] is not None:
                        col_da, col_db = bw_pending[w]
                        for obs_p in (obs_a, obs_b):
                            bw_stats[name]["total"] += 1
                            bw_stats[name][_classify_bw_read(obs_p, col_da, col_db, mask)] += 1
                        bw_pending[w] = None
                    if mem[w] is None:
                        mem[w] = obs_a
                elif w_a == w_b:
                    w = w_a
                    d_w = d_a if we_a == 1 else d_b
                    bw_pending[w] = None
                    if mem[w] is not None:
                        mem[w] = d_w
                else:
                    for w_p, d_p, we_p, obs_p in (
                        (w_a, d_a, we_a, obs_a),
                        (w_b, d_b, we_b, obs_b),
                    ):
                        if we_p == 1:
                            mem[w_p] = d_p
                            bw_pending[w_p] = None
                        else:
                            if bw_pending[w_p] is not None:
                                col_da, col_db = bw_pending[w_p]
                                bw_stats[name]["total"] += 1
                                bw_stats[name][_classify_bw_read(obs_p, col_da, col_db, mask)] += 1
                                bw_pending[w_p] = None
                            if mem[w_p] is None:
                                mem[w_p] = obs_p

    return stats, bw_stats, none_details, total_none


def _fmt_hex(val, width):
    if val is None:
        return "-"
    nibbles = (width + 3) // 4
    return f"0x{val:0{nibbles}X}"


def format_report(stats, bw_stats, none_details, total_none):
    sections = []
    for name, wa, wb, wma, wmb, regmode in EBRS:
        lines = [
            f"=== {name} ({wa}/{wb}, {wma}/{wmb}, {regmode}) ===",
            "PORT  CASE       KNOWN |   OLD    NEW    OWN  NONE(BITS)",
        ]
        for port in ("a", "b"):
            st = stats[(name, port)]
            for case in CASES:
                cs = st["cases"][case]
                new_str = f"{cs['new']:>5}" if cs["new_applicable"] else "    -"
                none_str = f"{cs['none_words']}({cs['none_bits']})"
                lines.append(
                    f"  {port.upper()}   {case:<9}  {cs['known']:>5} | "
                    f"{cs['old']:>5}  {new_str}  {cs['own']:>5}  {none_str:>10}"
                )
        bw = bw_stats[name]
        lines.append(
            f"post-both-write clean reads: total={bw['total']} "
            f"A={bw['A']} B={bw['B']} same(A==B)={bw['same']} "
            f"mix={bw['mix']} neither={bw['neither']}"
        )
        sections.append("\n".join(lines))

    detail_lines = [f"none-of-these cases (showing {len(none_details)} of {total_none}):"]
    for item in none_details:
        w = item["width"]
        if item["case"] == "wr_wr_col":
            new_disp = f"a:{_fmt_hex(item['d_a'], w)}/b:{_fmt_hex(item['d_b'], w)}"
        elif item["new_val"] is not None:
            new_disp = _fmt_hex(item["new_val"], w)
        else:
            d_self = item["d_a"] if item["port"] == "a" else item["d_b"]
            new_disp = f"self:{_fmt_hex(d_self, w)}"
        detail_lines.append(
            f"  {item['ebr']}.{item['port'].upper()} {item['case']} "
            f"win={item['win_lfsr']:016X} clk={item['clk']} addr={item['addr']} "
            f"old={_fmt_hex(item['old'], w)} new={new_disp} "
            f"out={_fmt_hex(item['out'], w)} none_bits={item['none_bits']}"
        )
    sections.append("\n".join(detail_lines))
    return "\n\n".join(sections)


def _simulate_synthetic_lines(
    collision_read="old",
    both_write_winner="A",
    n_windows=4,
    once=False,
    corrupt_ebr_port=None,
):
    """Simulate all 8 EBRs from configuration and emit synthetic UART lines."""
    mems = {
        name: init_word_array(name, wa, known=True)
        for name, wa, *_ in EBRS
    }
    latches = {(name, p): 0x15 & ((1 << w) - 1) for name, p, w in RECORD}
    out_regs = {(name, p): 0x0A & ((1 << w) - 1) for name, p, w in RECORD}
    regmodes = {name: regmode for name, *_, regmode in EBRS}
    widths = {name: wa for name, wa, *_ in EBRS}
    modes = {(name, "a"): wma for name, _, _, wma, _, _ in EBRS}
    modes.update({(name, "b"): wmb for name, _, _, _, wmb, _ in EBRS})

    lfsr = SEED
    lines = []
    gaps = [0] if once else [0, 311, 277, 353][:n_windows]
    for gap in gaps:
        for _ in range(gap):
            lfsr = _sim_step(
                lfsr, mems, latches, out_regs, regmodes, widths, modes,
                collision_read, both_write_winner, corrupt_ebr_port,
            )
        win_lfsr = lfsr
        win_samples = []
        for _ in range(SAMPLES):
            win_samples.append({
                (name, p): out_regs[(name, p)] if regmodes[name] == "OUTREG" else latches[(name, p)]
                for name, p, _ in RECORD
            })
            lfsr = _sim_step(
                lfsr, mems, latches, out_regs, regmodes, widths, modes,
                collision_read, both_write_winner, corrupt_ebr_port,
            )
        lines.append(encode_line(win_lfsr, win_samples))
    return lines


def _sim_step(
    lfsr, mems, latches, out_regs, regmodes, widths, modes,
    collision_read, both_write_winner, corrupt_ebr_port,
):
    (w_a, da_full, we_a), (w_b, db_full, we_b) = traffic_fields(lfsr)
    for name, *_ in EBRS:
        w = widths[name]
        mask = (1 << w) - 1
        d_a = da_full & mask
        d_b = db_full & mask
        mem = mems[name]
        old_a = mem[w_a]
        old_b = mem[w_b]

        if regmodes[name] == "OUTREG":
            out_regs[(name, "a")] = latches[(name, "a")]
            out_regs[(name, "b")] = latches[(name, "b")]

        for port, w_p, d_p, we_p, old_p, w_q, d_q, we_q in (
            ("a", w_a, d_a, we_a, old_a, w_b, d_b, we_b),
            ("b", w_b, d_b, we_b, old_b, w_a, d_a, we_a),
        ):
            key = (name, port)
            if we_p == 0:
                if w_p == w_q and we_q == 1:
                    if collision_read == "old":
                        latches[key] = old_p
                    elif collision_read == "new":
                        latches[key] = d_q
                    elif collision_read == "mix":
                        # Low bit from old, remaining bits from new.
                        latches[key] = (old_p & 1) | (d_q & ~1 & mask)
                    else:
                        raise ValueError(collision_read)
                else:
                    latches[key] = old_p
            else:
                mode = modes[key]
                if mode == "READBEFOREWRITE":
                    latches[key] = old_p
                elif mode == "WRITETHROUGH":
                    latches[key] = d_p
                elif mode == "NORMAL":
                    pass
                else:
                    raise ValueError(mode)

            if corrupt_ebr_port == key and we_p == 0 and w_p == w_q and we_q == 1:
                latches[key] ^= mask

        if w_a == w_b and we_a == 1 and we_b == 1:
            if both_write_winner == "A":
                mem[w_a] = d_a
            elif both_write_winner == "B":
                mem[w_a] = d_b
            else:
                raise ValueError(both_write_winner)
        else:
            if we_a == 1:
                mem[w_a] = d_a
            if we_b == 1:
                mem[w_b] = d_b

    return lfsr_step(lfsr)


def selftest():
    """Verify collision_read='old' vs 'new' and bit-mix/error detection."""
    for once_flag, n_win in ((True, 1), (False, 4)):
        # 1. Collision read returns the old word, both-write winner A.
        old_lines = _simulate_synthetic_lines(
            collision_read="old", both_write_winner="A", n_windows=n_win, once=once_flag
        )
        log_rows = [
            f"load 1 at 2026-01-01T00:00:00Z: loader_done=True lines={len(old_lines) + 1} "
            f"distinct={old_lines + [old_lines[0][:100]]!r}\n"
        ]
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as tf:
            tf.writelines(log_rows)
            log_path = tf.name
        try:
            _, parsed_lines = parse_log(log_path)
        finally:
            os.remove(log_path)
        assert parsed_lines == sorted(old_lines)

        old_stats, old_bw, old_none, old_tot_none = score_windows(
            [decode_line(x) for x in parsed_lines], once=once_flag
        )
        assert old_tot_none == 0 and not old_none, f"unexpected none cases: {old_none}"
        for key, st in old_stats.items():
            for case in CASES:
                cs = st["cases"][case]
                assert cs["known"] >= 4, f"{key} {case} too few known: {cs['known']}"
                assert cs["own"] == cs["known"], f"{key} {case} own mismatch: {cs}"
                assert cs["none_words"] == 0 and cs["none_bits"] == 0
            rdwr = st["cases"]["rd_wr_col"]
            assert rdwr["old"] == rdwr["known"], f"{key} rd_wr_col old mismatch: {rdwr}"
            assert rdwr["new"] < rdwr["known"], f"{key} rd_wr_col failed to separate new: {rdwr}"
        for name, *_ in EBRS:
            bw = old_bw[name]
            assert bw["total"] > 0 and bw["A"] == bw["total"] - bw["same"], f"{name} bw: {bw}"

        # 2. Collision read returns the new word, both-write winner B.
        new_lines = _simulate_synthetic_lines(
            collision_read="new", both_write_winner="B", n_windows=n_win, once=once_flag
        )
        new_stats, new_bw, new_none, new_tot_none = score_windows(
            [decode_line(x) for x in new_lines], once=once_flag
        )
        assert new_tot_none == 0 and not new_none, f"unexpected none cases: {new_none}"
        for key, st in new_stats.items():
            for case in ("clean_rd", "clean_wr", "wr_wr_col", "wr_rd_col"):
                cs = st["cases"][case]
                assert cs["own"] == cs["known"], f"{key} {case} own mismatch: {cs}"
                assert cs["none_words"] == 0 and cs["none_bits"] == 0
            rdwr = st["cases"]["rd_wr_col"]
            assert rdwr["new"] == rdwr["known"], f"{key} rd_wr_col new mismatch: {rdwr}"
            assert rdwr["old"] < rdwr["known"], f"{key} rd_wr_col failed to separate old: {rdwr}"
            assert rdwr["own"] < rdwr["known"], f"{key} rd_wr_col own should miss: {rdwr}"
            assert rdwr["none_words"] == 0 and rdwr["none_bits"] == 0
        for name, *_ in EBRS:
            bw = new_bw[name]
            assert bw["total"] > 0 and bw["B"] == bw["total"] - bw["same"], f"{name} bw: {bw}"

    # 3. Bit-mix on collision read gives none_words > 0 with none_bits == 0 on multi-bit EBRs.
    mix_lines = _simulate_synthetic_lines(collision_read="mix", n_windows=2, once=False)
    mix_stats, _, mix_none, mix_tot_none = score_windows(
        [decode_line(x) for x in mix_lines], once=False
    )
    assert mix_tot_none > 0 and len(mix_none) == 10
    assert mix_stats[("e4", "a")]["cases"]["rd_wr_col"]["none_words"] > 0
    assert mix_stats[("e4", "a")]["cases"]["rd_wr_col"]["none_bits"] == 0

    # 4. Corrupted output triggers both none_words > 0 and none_bits > 0.
    bad_lines = _simulate_synthetic_lines(
        collision_read="old", n_windows=2, once=False, corrupt_ebr_port=("e3", "a")
    )
    bad_stats, _, bad_none, bad_tot_none = score_windows(
        [decode_line(x) for x in bad_lines], once=False
    )
    assert bad_tot_none > 0 and len(bad_none) > 0
    assert bad_stats[("e3", "a")]["cases"]["rd_wr_col"]["none_words"] > 0
    assert bad_stats[("e3", "a")]["cases"]["rd_wr_col"]["none_bits"] > 0

    print("selftest passed (old vs new collision reads, s-mode & free-running, bit-mix & corrupt)")


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--selftest":
        selftest()
        return
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "s"):
        raise SystemExit("Usage: python3 hb09_analyse.py LOG [s] | --selftest")

    log_path = sys.argv[1]
    once = len(sys.argv) == 3 and sys.argv[2] == "s"
    loads, lines = parse_log(log_path)
    print(f"loads: {len(loads)} full windows per load: {[len(l) for l in loads]} distinct: {len(lines)}")
    if not lines:
        raise SystemExit("no valid R lines found")
    windows = [decode_line(x) for x in lines]
    stats, bw_stats, none_details, total_none = score_windows(windows, once=once)
    print(format_report(stats, bw_stats, none_details, total_none))


if __name__ == "__main__":
    main()
