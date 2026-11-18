"""Score HB09P UART windows without choosing a PDP collision rule.

Accepts HB09 run.py's distinct=[...] logs or raw R... lines. --once is ONLY for
hb09ps (the first window from configuration); free windows start with unknown
memory and become scoreable after writes. Sample 0 is never scored.
"""
import argparse
import ast
from collections import Counter
import json

from hb09p_gen import EBRS, LINE_LEN, SAMPLE_BITS, SAMPLES, SEED, init_words, lfsr_step, traffic_fields


def parse_log(path):
    loads, lines = [], set()
    with open(path, encoding="utf-8") as stream:
        for raw in stream:
            items = ast.literal_eval(raw.partition("distinct=")[2]) if "distinct=" in raw else [raw.strip()]
            valid = {x for x in items if isinstance(x, str) and len(x) == LINE_LEN
                     and x.startswith("R") and all(c in "0123456789abcdefABCDEF" for c in x[1:])}
            if "distinct=" in raw:
                loads.append(valid)
            lines.update(valid)
    return loads, sorted(lines)


def decode_line(line):
    rows = []
    size = SAMPLE_BITS // 4
    for k in range(SAMPLES):
        value, pos, row = int(line[17 + k*size:17 + (k+1)*size], 16), SAMPLE_BITS, {}
        for name, width, _ in EBRS:
            pos -= width
            row[name] = (value >> pos) & ((1 << width) - 1)
        rows.append(row)
    return int(line[1:17], 16), rows


def encode_line(seed, samples):
    chunks = [f"R{seed:016X}"]
    for row in samples:
        value = 0
        for name, width, _ in EBRS:
            value = (value << width) | row[name]
        chunks.append(f"{value:0{SAMPLE_BITS//4}X}")
    return "".join(chunks)


def classify(value, old, new, mask):
    if old == new and value == old:
        return "same"
    if value == old:
        return "old"
    if value == new:
        return "new"
    # Every bit agrees with at least one candidate, but neither full word wins.
    return "mix" if ((value ^ old) & (value ^ new) & mask) == 0 else "other"


def score(windows, once=False):
    result = {name: {"width": width, "depth": depth, "cases": {case: Counter()
                   for case in ("idle_read", "disjoint_write", "collision", "clean_after_collision")},
                    "lanes": [Counter() for _ in range(4)], "examples": []}
              for name, width, depth in EBRS}
    for seed, samples in windows:
        if once and seed != SEED:
            raise ValueError("--once requires the configuration window's SEED; wrong image/log")
        states, state = [], seed
        for _ in range(SAMPLES):
            states.append(state)
            state = lfsr_step(state)
        for name, width, depth in EBRS:
            mask = (1 << width) - 1
            memory = init_words(name, width) if once else [None] * 512
            written_in_collision = set()
            stats = result[name]
            for k, state in enumerate(states[:-1]):
                wa, ra, we, data = traffic_fields(state, depth)
                data &= mask
                old, value, held = memory[ra], samples[k+1][name], samples[k][name]
                collision = bool(we and wa == ra)
                case = "collision" if collision else "disjoint_write" if we else "idle_read"
                counts = stats["cases"][case]
                counts["edges"] += 1
                if old is None:
                    counts["unknown_old"] += 1
                else:
                    counts["known"] += 1
                    if collision:
                        category = classify(value, old, data, mask)
                        counts[category] += 1
                        counts["held_match"] += value == held
                        counts["bits_outside_old_new"] += ((value ^ old) & (value ^ data) & mask).bit_count()
                        for lane in range(4):
                            lane_mask = ((1 << min(9, width - 9*lane)) - 1)
                            stats["lanes"][lane][classify((value >> (9*lane)) & lane_mask,
                                (old >> (9*lane)) & lane_mask, (data >> (9*lane)) & lane_mask, lane_mask)] += 1
                        if category in ("mix", "other") and len(stats["examples"]) < 8:
                            stats["examples"].append({"seed": f"{seed:016X}", "edge": k,
                                "address": ra, "old": old, "new": data, "held": held, "output": value})
                    else:
                        counts["match" if value == old else "mismatch"] += 1
                        if ra in written_in_collision:
                            post = stats["cases"]["clean_after_collision"]
                            post["known"] += 1
                            post["match" if value == old else "mismatch"] += 1
                # One writer: model its requested data, then VERIFY it using
                # later clean reads. Never infer array contents from a colliding DO.
                if we:
                    memory[wa] = data
                    written_in_collision.discard(wa)
                    if collision:
                        written_in_collision.add(wa)
    return result


def synthetic(policy, seed=SEED):
    memory = {name: init_words(name, width) for name, width, _ in EBRS}
    rows = [{name: 0 for name, _, _ in EBRS}]
    state = seed
    for _ in range(SAMPLES-1):
        row = {}
        for name, width, depth in EBRS:
            wa, ra, we, data = traffic_fields(state, depth)
            data &= (1 << width) - 1
            row[name] = data if policy == "new" and we and wa == ra else memory[name][ra]
            if we:
                memory[name][wa] = data
        rows.append(row)
        state = lfsr_step(state)
    return seed, rows


def selftest():
    assert classify(0b0101, 0b0000, 0b1111, 15) == "mix"
    assert classify(0b0100, 0b0000, 0b0011, 15) == "other"
    assert classify(3, 3, 3, 15) == "same"
    for policy in ("old", "new"):
        window = synthetic(policy)
        line = encode_line(*window)
        assert len(line) == LINE_LEN and decode_line(line) == window
        for once in (True, False):
            scores = score([decode_line(line)], once)
            for stats in scores.values():
                collision = stats["cases"]["collision"]
                assert collision["known"] > 10 and collision[policy] > 10
                assert collision["known"] == collision[policy] + collision["same"]
                for case in ("idle_read", "disjoint_write", "clean_after_collision"):
                    assert stats["cases"][case]["mismatch"] == 0
                assert stats["cases"]["clean_after_collision"]["known"] > 0
    # A one-bit corruption of an idle read must be reported, including in the
    # high half. This prevents a parser/order error from looking like a rule.
    seed, samples = synthetic("old")
    state = seed
    for k in range(SAMPLES-1):
        if not traffic_fields(state, 128)[2]:
            samples[k+1][EBRS[-1][0]] ^= 1 << 35
            break
        state = lfsr_step(state)
    assert score([(seed, samples)], True)[EBRS[-1][0]]["cases"]["idle_read"]["mismatch"] == 1
    try:
        score([synthetic("old", lfsr_step(SEED))], True)
    except ValueError:
        pass
    else:
        raise AssertionError("accepted free window as configuration window")
    print("HB09P analysis selftest passed (old/new, both halves, clean/post-collision, framing, INIT scope)")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        selftest()
    else:
        if not args.log:
            parser.error("log required")
        loads, lines = parse_log(args.log)
        if not lines:
            parser.error("no complete HB09P windows")
        report = {"distinct_windows": len(lines), "windows_per_load": [len(x) for x in loads],
                  "once": args.once, "results": score([decode_line(x) for x in lines], args.once)}
        # Per-load results expose instability; deduplicated totals are not
        # independent repeats when hb09ps resends the same frozen window.
        report["per_load"] = [score([decode_line(x) for x in sorted(load)], args.once) for load in loads]
        print(json.dumps(report, indent=2))
