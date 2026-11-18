#!/usr/bin/env python3
"""Validate HY04 UART records and report timing, retrigger evidence and code drift.

Usage: hy04_analyse.py CAPTURE.txt [--json] [--settle-gap 4] [--selftest]
No temperature scale is assumed. Missing output changes are not completion events.
"""
import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass, field
import json
from pathlib import Path
import statistics
import sys

HZ = 25_000_000
RISE = 16
# kind: (width, spacing, pulse count, last sample index), all in observer ticks.
SCHEDULE = {
    0: (0, 0, 0, 7500), 1: (1, 0, 1, 7516),
    2: (10, 0, 1, 7516), 3: (2500, 0, 1, 10016),
    4: (15000, 0, 1, 22516), 5: (1, 250, 2, 7766),
    6: (1, 750, 2, 8266), 7: (1, 1500, 2, 9016),
    8: (1, 500, 8, 23516), 9: (1, 0, 1, 7516),
}
LENGTHS = {"I": (2, 12, 8), "B": (8, 1, 12, 4, 4, 4, 2, 2),
           "C": (8, 4, 2), "E": (8, 4, 8, 2), "Z": (8, 12)}


@dataclass
class Window:
    seq: int
    kind: int
    tick: int
    end: int
    width: int
    gap: int
    count: int
    base: int
    changes: list = field(default_factory=list)
    final: int = 0

    def sample_sum(self):
        total, previous, start = 0, self.base, 0
        for offset, value in self.changes:
            total += (offset - start) * previous
            start, previous = offset, value
        return (total + (self.end + 1 - start) * previous) & 0xFFFFFFFF


@dataclass
class Run:
    initial: int
    windows: list = field(default_factory=list)
    done: bool = False
    warnings: list = field(default_factory=list)
    last_i_tick: int = -1
    last_i_seq: int = 0
    next_seq: int = 1
    initial_progress: int = 0
    startup_replays: int = 0


def record(tag, *values):
    return "Y4" + tag + "".join(f"{v:0{n}X}" for v, n in zip(values, LENGTHS[tag]))


def parse(text):
    """Synchronize on I, deduplicate immutable startup replays, validate windows."""
    runs, run, active = [], None, None
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line.startswith("Y4"):
            continue
        # A late reader can attach inside a line/window. Only I establishes
        # framing; records preceding the first complete I have no context.
        if run is None and (not line.startswith("Y4I") or len(line) != 25):
            continue
        try:
            tag = line[2:3]
            if tag not in LENGTHS or len(line) != 3 + sum(LENGTHS[tag]):
                raise ValueError("unknown tag or wrong line length")
            pos, fields = 3, []
            for n in LENGTHS[tag]:
                token = line[pos:pos+n]
                if any(c not in "0123456789ABCDEF" for c in token):
                    raise ValueError("payload must be uppercase hexadecimal")
                fields.append(int(token, 16))
                pos += n
            if tag == "I":
                initial, sent_tick, progress = fields
                # A reload resets time/progress. Repeated I within a boot is
                # monotonic and must retain the original acquisition.
                new_run = (run is None or sent_tick <= run.last_i_tick
                           or progress < run.last_i_seq)
                if new_run:
                    if run is not None and not run.done:
                        run.warnings.append("Run ended before Z; reload or truncated capture.")
                    if active is not None:
                        run.warnings.append(f"Discarded unfinished window {active.seq}.")
                    run = Run(initial, next_seq=max(1, progress), initial_progress=progress)
                    if progress > 1:
                        run.warnings.append(f"Late attachment: missed measurement sequences 1..{progress-1}.")
                    runs.append(run)
                    active = None
                else:
                    if initial != run.initial:
                        raise ValueError("initial acquisition changed during replay")
                    if active is not None:
                        raise ValueError("I interrupted an unfinished window")
                    if progress != run.next_seq and not (progress == 0 and run.next_seq == 1):
                        raise ValueError("I progress disagrees with measurement sequence")
                run.last_i_tick, run.last_i_seq = sent_tick, progress
                continue
            if tag == "B":
                if active is not None:
                    raise ValueError("B before preceding E")
                active = Window(*fields)
                if active.kind not in SCHEDULE or (
                    active.width, active.gap, active.count, active.end
                ) != SCHEDULE[active.kind]:
                    raise ValueError("schedule metadata does not match this protocol")
                if active.kind == 0:
                    if active.seq != 0 or active.tick != 2 or active.base != run.initial:
                        raise ValueError("startup window must retain the first-clock sample")
                else:
                    if run.done or not run.windows:
                        raise ValueError("measurement without startup or after Z")
                    if active.seq != run.next_seq:
                        raise ValueError("missing, duplicated or reordered window sequence")
                    if active.tick <= run.windows[-1].tick + run.windows[-1].end:
                        raise ValueError("nonmonotonic or overlapping windows")
            elif tag == "C":
                seq, offset, value = fields
                if active is None or active.seq != seq:
                    raise ValueError("C without matching B")
                prev_offset, prev_value = active.changes[-1] if active.changes else (0, active.base)
                if not prev_offset < offset <= active.end or value == prev_value:
                    raise ValueError("invalid change offset or unchanged value")
                active.changes.append((offset, value))
            elif tag == "E":
                seq, count, checksum, final = fields
                if active is None or active.seq != seq:
                    raise ValueError("E without matching B")
                expected = active.changes[-1][1] if active.changes else active.base
                if count != len(active.changes) or checksum != active.sample_sum() or final != expected:
                    raise ValueError("change count, full-sample checksum or final value mismatch")
                active.final = final
                if active.kind == 0:
                    if run.windows:
                        if active != run.windows[0]:
                            raise ValueError("startup replay differs from retained window")
                    else:
                        run.windows.append(active)
                    run.startup_replays += 1
                else:
                    run.windows.append(active)
                    run.next_seq += 1
                active = None
            elif tag == "Z":
                if active is not None or not run.windows or fields[0] != run.next_seq:
                    raise ValueError("Z with unfinished/missing windows")
                if fields[1] <= run.windows[-1].tick + run.windows[-1].end:
                    raise ValueError("invalid end timestamp")
                run.done = True
        except ValueError as exc:
            raise ValueError(f"line {lineno}: {exc}") from exc
    if not runs:
        raise ValueError("no HY04 I record found")
    if active is not None:
        run.warnings.append(f"Discarded unfinished window {active.seq}.")
    if run is not None and not run.done:
        run.warnings.append("No Z record: capture is incomplete.")
    return runs


def summary(values):
    return {"n": len(values), "min": min(values), "max": max(values),
            "median": statistics.median(values)} if values else {"n": 0}


def clusters(changes, settle_gap=4):
    result = []
    for item in changes:
        if not result or item[0] - result[-1][-1][0] > settle_gap:
            result.append([])
        result[-1].append(item)
    return result


def predicted(model, rises, service):
    """Physical completion offsets; equal output codes can hide any completion."""
    finishes = []
    if model == "queue":
        for rise in rises:
            finishes.append(max(rise, finishes[-1] if finishes else rise) + service)
    elif model == "ignore":
        for rise in rises:
            if not finishes or rise >= finishes[-1]:
                finishes.append(rise + service)
    elif model == "restart":
        for rise in rises:
            if finishes and rise < finishes[-1]:
                finishes[-1] = rise + service
            else:
                finishes.append(rise + service)
    else:
        raise ValueError(model)
    return finishes


def compatible(observed, expected, tolerance):
    """Order-preserving subset match; never require a code change per conversion."""
    j = 0
    for value in observed:
        while j < len(expected) and expected[j] < value - tolerance:
            j += 1
        if j == len(expected) or abs(value - expected[j]) > tolerance:
            return False
        j += 1
    return True


def distribution(windows):
    if not windows:
        return {"n": 0}
    codes = [w.final for w in windows]
    times = [(w.tick + w.end) / HZ for w in windows]
    mean_t, mean_c = statistics.mean(times), statistics.mean(codes)
    variance_t = sum((t - mean_t)**2 for t in times)
    slope = (sum((t-mean_t)*(c-mean_c) for t, c in zip(times, codes)) / variance_t
             if variance_t else 0)
    edge = max(1, len(codes)//10)
    return dict(summary(codes), mean=mean_c, stdev=statistics.pstdev(codes),
                histogram={f"{k:02X}": v for k, v in sorted(Counter(codes).items())},
                span_seconds=times[-1]-times[0], slope_codes_per_minute=slope*60,
                first_decile_mean=statistics.mean(codes[:edge]),
                last_decile_mean=statistics.mean(codes[-edge:]),
                interval_seconds=summary([b-a for a, b in zip(times, times[1:])]))


def analyse(run, settle_gap=4):
    single = [w for w in run.windows if w.kind in (1, 2, 3, 4, 9)]
    by_width = defaultdict(list)
    for w in single:
        by_width[w.width].append(w)
    timing = {}
    for width, windows in sorted(by_width.items()):
        first, last, shapes = [], [], Counter()
        for w in windows:
            changes = [(t, v) for t, v in w.changes if t >= RISE]
            groups = clusters(changes, settle_gap)
            if not changes:
                shapes["no_visible_change"] += 1
                continue
            first.append(changes[0][0] - RISE)
            last.append(changes[-1][0] - RISE)
            shapes["one_jump" if len(changes) == 1 else "multiple_changes"] += 1
            shapes["one_cluster" if len(groups) == 1 else "multiple_clusters"] += 1
            if any(len(g) > 1 for g in groups):
                shapes["possible_bus_skew_or_intermediate_codes"] += 1
                shapes["asynchronous_bus_steps"] += 1
        timing[str(width)] = {
            "windows": len(windows), "first_change_cycles": summary(first),
            "last_change_cycles": summary(last),
            "first_change_us": summary([v/HZ*1e6 for v in first]),
            "estimated_physical_first_us": summary([(v-2.5)/HZ*1e6 for v in first]),
            "shapes": dict(shapes),
        }

    # Calibrate the one-cycle trigger from isolated, visible one-cluster trials.
    calibration = []
    for w in single:
        groups = clusters([(t, v) for t, v in w.changes if t >= RISE], settle_gap)
        if (w.width == 1 and len(groups) == 1
                and groups[0][0][0] - RISE > 2.5
                and not any(t < RISE for t, _ in w.changes)):
            calibration.append(groups[0][0][0] - RISE)
    retrigger = {"calibration_observed_cycles": summary(calibration), "windows": []}
    if calibration:
        service = statistics.median(calibration) - 2.5
        tolerance = max(8, max(calibration)-min(calibration)+4)
        retrigger.update(service_cycles=service, tolerance_cycles=tolerance)
        for w in run.windows:
            if w.kind not in (5, 6, 7, 8):
                continue
            observed = [g[0][0] for g in clusters(
                [(t, v) for t, v in w.changes if t >= RISE], settle_gap)]
            rises = [RISE + j*w.gap for j in range(w.count)]
            expectations = {m: [t+2.5 for t in predicted(m, rises, service)
                                if t+2.5 <= w.end]
                            for m in ("restart", "ignore", "queue")}
            candidates = [m for m, expected in expectations.items()
                          if compatible(observed, expected, tolerance)]
            retrigger["windows"].append({
                "seq": w.seq, "kind": w.kind, "gap_cycles": w.gap,
                "observed_clusters": observed, "predicted_observer_cycles": expectations,
                "candidates": candidates, "informative": bool(observed),
            })
        retrigger["candidate_counts"] = dict(Counter(
            m for item in retrigger["windows"] if item["informative"] for m in item["candidates"]))
    else:
        retrigger["reason"] = "No isolated one-cycle, one-cluster latency calibration."

    details = []
    for w in run.windows:
        # Every change is retained, including pre-trigger changes and bus steps.
        pulses = [RISE + j*w.gap for j in range(w.count)]
        previous, masks = w.base, []
        for offset, value in w.changes:
            masks.append((offset, previous ^ value))
            previous = value
        bus_steps = [g for g in clusters(w.changes, settle_gap) if len(g) > 1]
        details.append({"seq": w.seq, "kind": w.kind, "tick": w.tick,
                        "base": w.base, "final": w.final,
                        "changes": w.changes, "pulse_rises": pulses,
                        "changed_bit_masks": masks,
                        "asynchronous_to_observer_evidence": bool(bus_steps),
                        "multi_tick_bus_step_clusters": bus_steps,
                        "changes_per_pulse": [
                            {"rise": rise, "width": w.width,
                             "events": [(t-rise, v) for t, v in w.changes
                                        if rise <= t <= rise+7500]}
                            for rise in pulses]})
    warnings = list(run.warnings)
    pretrigger = [w.seq for w in run.windows if w.count and any(t < RISE for t, _ in w.changes)]
    if pretrigger:
        warnings.append(f"Pre-trigger activity in windows {pretrigger}; prior activity may contaminate timing.")
    drift = distribution([w for w in run.windows if w.kind == 9])
    if drift.get("span_seconds", 0) < 60:
        warnings.append("Drift readings span less than 60 seconds.")
    return {
        "initial": f"{run.initial:02X}",
        "complete": run.done and run.initial_progress <= 1,
        "device_done": run.done, "startup_replays": run.startup_replays,
        "windows": len(run.windows), "warnings": warnings,
        "baseline_changes": [w.changes for w in run.windows if w.kind == 0],
        "timing_by_width_cycles": timing, "retrigger": retrigger,
        "isolated_final_code_distribution": distribution(single),
        "drift": drift, "records": details,
        "limits": [
            "Latencies measure visible code changes, not a conversion-done signal; identical codes hide conversions.",
            "Observer timestamps include two synchronizer cycles; subtract 2.5 ticks for an estimate with nominal +/-0.5 tick sampling uncertainty, plus routing/metastability uncertainty.",
            "Multi-cycle bit changes are asynchronous-bus/intermediate-code evidence; simultaneous sampled changes do not prove synchrony.",
            "Retrigger candidates are timing compatibility evidence, not proof; hidden same-code conversions preserve ambiguity.",
            "Final-window codes are distribution proxies, not confirmed fresh conversions. There is no calibrated Celsius scale.",
        ],
    }


def print_report(report, index):
    print(f"Run {index}: initial=0x{report['initial']} (first observer clock), "
          f"windows={report['windows']}, startup_replays={report['startup_replays']}, "
          f"complete={report['complete']}")
    print(f"  Baseline changes: {report['baseline_changes']}")
    for width, data in report["timing_by_width_cycles"].items():
        print(f"  Width {width} cycles: first-change cycles {data['first_change_cycles']}; "
              f"last-change cycles {data['last_change_cycles']}; shapes {data['shapes']}")
    retrigger = report["retrigger"]
    print(f"  Retrigger calibration: {retrigger['calibration_observed_cycles']}")
    grouped = defaultdict(Counter)
    for item in retrigger["windows"]:
        label = "/".join(item["candidates"]) or "no simple model fits"
        if not item["informative"]:
            label = "unobservable: no code change"
        grouped[item["gap_cycles"]][label] += 1
    for gap, counts in sorted(grouped.items()):
        print(f"  Retrigger spacing {gap} cycles: {dict(counts)}")
    if "reason" in retrigger:
        print(f"  {retrigger['reason']}")
    for name in ("isolated_final_code_distribution", "drift"):
        print(f"  {name}: {json.dumps(report[name], sort_keys=True)}")
    for warning in report["warnings"]:
        print(f"  Warning: {warning}")
    for limit in report["limits"]:
        print(f"  Limit: {limit}")


def synthetic_window(seq, kind, tick, base, changes):
    width, gap, count, end = SCHEDULE[kind]
    w = Window(seq, kind, tick, end, width, gap, count, base, changes)
    final = changes[-1][1] if changes else base
    return [record("B", seq, kind, tick, end, width, gap, count, base),
            *[record("C", seq, t, v) for t, v in changes],
            record("E", seq, len(changes), w.sample_sum(), final)]


def board_reading(runs):
    """Reproducible HY04 comparison; keep raw steps distinct from filter hypotheses."""
    print("HY04 DTR board reading (25 MHz, 40 ns per observer tick)")
    print("Source: observations.log; no temperature conversion is inferred.")
    for n, run in enumerate(runs, 1):
        drift = distribution([w for w in run.windows if w.kind == 9])
        print(f"Load {n}: initial=0x{run.initial:02X}, complete={run.done}, "
              f"unique windows={len(run.windows)}, startup replays={run.startup_replays}; drift={drift}")
    isolated = [(n, w) for n, run in enumerate(runs, 1) for w in run.windows
                if w.kind in (1, 2, 3, 4, 9)]
    canonical = tuple(range(31)) + (0x9E,)
    clean_count, fleeting = 0, []
    pooled = []
    print("\nRaw isolated value sequences and offsets are NOT identical:")
    for kind in (1, 2, 3, 4, 9):
        windows = [(n, w) for n, w in isolated if w.kind == kind]
        if not windows:
            continue
        raw_values = Counter(tuple(v for t, v in w.changes) for _, w in windows)
        raw_times = Counter(tuple((t-RISE, v) for t, v in w.changes) for _, w in windows)
        lasts = [w.changes[-1][0]-RISE-w.width for _, w in windows if w.changes]
        pooled.extend(lasts)
        print(f"Kind {kind}: n={len(windows)}, raw value sequences={len(raw_values)}, "
              f"raw timed sequences={len(raw_times)}, final-from-fall={summary(lasts)} ticks; "
              f"estimated physical median={(statistics.median(lasts)-2.5)*0.04:.3f} us")
        for values, count in raw_values.most_common():
            print(f"  {count} times: " + " ".join(f"{v:02X}" for v in values))
        for load, w in windows:
            # Filter only one-tick dwell values, as a stated skew hypothesis.
            # Original observations and the ordinary JSON report remain untouched.
            keep = []
            for j, (t, value) in enumerate(w.changes):
                if j+1 < len(w.changes) and w.changes[j+1][0]-t == 1:
                    fleeting.append((load, w.seq, t-RISE, value))
                else:
                    keep.append(value)
            clean_count += tuple(keep) == canonical
        print("  Persistent code (one-tick dwell omitted): rise-relative min/median/max; fall-relative min/median/max (observer ticks)")
        for value in canonical:
            stable_windows = [(w, [(t, v) for j, (t, v) in enumerate(w.changes)
                                  if j+1 == len(w.changes) or w.changes[j+1][0]-t > 1])
                              for _, w in windows]
            offsets = [(next(t for t, v in stable if v == value)-RISE, w.width)
                       for w, stable in stable_windows if any(v == value for _, v in stable)]
            rise = summary([t for t, _ in offsets])
            fall = summary([t-width for t, width in offsets])
            print(f"    {value:02X}: {rise.get('min')}/{rise.get('median')}/{rise.get('max')}; "
                  f"{fall.get('min')}/{fall.get('median')}/{fall.get('max')}")
    print(f"\nRemoving only one-tick dwell values yields 00..1E,9E in {clean_count}/{len(isolated)} isolated windows.")
    print(f"One-tick dwell events: {len(fleeting)}; (load, sequence, rise-relative offset, code):")
    print(" ".join(f"({n},{seq},{t},{v:02X})" for n, seq, t, v in fleeting))
    print(f"Pooled isolated final-from-fall: {summary(pooled)} ticks; "
          f"physical median={(statistics.median(pooled)-2.5)*0.04:.3f} us.")
    steps = []
    for _, w in isolated:
        stable = [(t, v) for j, (t, v) in enumerate(w.changes)
                  if j+1 == len(w.changes) or w.changes[j+1][0]-t > 1]
        steps.extend(b[0]-a[0] for a, b in zip(stable, stable[1:])
                     if 1 <= a[1] < 30 and b[1] == a[1]+1)
    print(f"Stable counter increments 01..1E: spacing {dict(sorted(Counter(steps).items()))} ticks.")
    print("\nPooled persistent step offsets from fall: min/median/max; spread (observer ticks)")
    stable_windows = [(w, [(t, v) for j, (t, v) in enumerate(w.changes)
                          if j+1 == len(w.changes) or w.changes[j+1][0]-t > 1])
                      for _, w in isolated]
    for value in range(1, 31):
        offsets = [next(t for t, v in stable if v == value)-RISE-w.width
                   for w, stable in stable_windows if any(v == value for _, v in stable)]
        stat = summary(offsets)
        if offsets:
            print(f"  {value:02X}: {stat['min']}/{stat['median']}/{stat['max']}; "
                  f"spread={stat['max']-stat['min']}")
    skew = []
    for _, w in isolated:
        for j in range(1, len(w.changes)-1):
            t, value = w.changes[j]
            nt, following = w.changes[j+1]
            if nt-t == 1:
                previous = w.changes[j-1][1]
                skew.append(following == previous+1
                            and value & ~(previous | following) == 0
                            and value & (previous & following) == previous & following)
    print(f"One-tick codes immediately before adjacent persistent counts, "
          f"with only old/new bit mixtures: {sum(skew)}/{len(skew)}.")
    print("24/25-tick increments are regular only AFTER a width/phase-dependent startup delay.")
    firsts = [next(t for t, v in stable if v == 1)-RISE-w.width
              for w, stable in stable_windows if any(v == 1 for _, v in stable)]
    print(f"From-fall first-count offsets: {summary(firsts)}; "
          f"subsequent increment spacing: {summary(steps)} ticks.")
    if firsts and steps:
        print(f"First-count median minus increment median: "
              f"{statistics.median(firsts)-statistics.median(steps)} ticks; "
              "a fall-anchored uniform count requires this within 1-2 ticks.")
    print("Selected rule: 00 from assertion through conversion; final profile byte at latest fall + 32.740 us.")
    print("This is an ascending binary count, not the visible MSB-to-LSB trial sequence of successive approximation.")
    print("A hidden internal successive-approximation converter is neither proved nor ruled out by these output wires.")
    print("Long high pulses clear to 00 and hold it; counting becomes visible only after falling. No periodic completion while held high.")
    print("\nRetrigger evidence (observer timestamps, not one-cluster latency fitting):")
    for kind in (5, 6, 7, 8):
        windows = [w for run in runs for w in run.windows if w.kind == kind]
        if not windows:
            continue
        resets = Counter(sum(any(t == RISE+j*w.gap+3 and v == 0 for t, v in w.changes)
                             for j in range(w.count)) for w in windows)
        finals = Counter(sum(v == 0x9E for _, v in w.changes) for w in windows)
        tails = [w.changes[-1][0]-RISE-(w.count-1)*w.gap-w.width for w in windows]
        print(f"Kind {kind}: n={len(windows)}, spacing={windows[0].gap} ticks, "
              f"rise+3 zero resets/window={dict(resets)}, 9E completions/window={dict(finals)}, "
              f"last-fall-to-final={summary(tails)} ticks")
    print("Kinds 5/6/8 restart the in-progress count at every rise; no old completion or queued tail survives.")
    print("Kind 7 (60 us) completes before the second rise; it demonstrates two independent conversions, not a busy retrigger.")
    print("\nImplementation policy: initial/final full byte from compile::Board::dtr_code, default 0x9E.")
    print("Two-valued fallback: 00 from rise, throughout high, until latest fall + 32.740 us; bit 7 stays clear until done.")
    print("32.740 us is the pooled isolated median minus the nominal 2.5-tick synchronizer delay; it is a deterministic policy, not a width/phase-accurate fit.")
    print("Open: no Celsius scale; one chip at one uncontrolled temperature; no observed drift is not proof of constant die temperature.")
    print("Open: sub-40 ns skew/latency, synchronizer/metastability, internal conversion clock/algorithm, other codes/temperatures and analog noise.")


def selftest():
    lines = [record("I", 0x40, HZ, 0)]
    cases = [(0, [], 0x40), (1, [(1766, 0x41)], 0x40),
             (2, [(1767, 0x40), (1768, 0x42)], 0x41),
             (3, [], 0x42), (4, [(1766, 0x43), (3516, 0x44)], 0x42),
             (5, [(1766, 0x45)], 0x44),
             (6, [(2516, 0x46)], 0x45),
             (7, [(1766, 0x47), (3514, 0x48)], 0x46),
             (8, [], 0x48), (9, [(1766, 0x49)], 0x48),
             (9, [(1766, 0x4A)], 0x49)]
    for seq, (kind, changes, base) in enumerate(cases):
        lines.extend(synthetic_window(seq, kind, 2 if seq == 0 else seq*HZ*6, base, changes))
    lines.append(record("Z", len(cases), len(cases)*HZ*6))
    source = "\r\n".join(lines)
    run = parse(source)[0]
    r = analyse(run)
    assert r["initial"] == "40" and r["complete"] and len(run.windows) == len(cases)
    assert r["timing_by_width_cycles"]["1"]["first_change_cycles"]["median"] == 1750
    assert r["timing_by_width_cycles"]["10"]["first_change_cycles"] == {
        "n": 1, "min": 1751, "max": 1751, "median": 1751}
    assert r["timing_by_width_cycles"]["10"]["last_change_cycles"]["max"] == 1752
    assert r["timing_by_width_cycles"]["2500"]["shapes"]["no_visible_change"] == 1
    assert r["timing_by_width_cycles"]["10"]["shapes"]["possible_bus_skew_or_intermediate_codes"] == 1
    evidence = {w["kind"]: w for w in r["retrigger"]["windows"]}
    assert set(evidence[5]["candidates"]) == {"ignore", "queue"}
    assert evidence[6]["candidates"] == ["restart"]
    assert evidence[7]["candidates"] == ["queue"]
    assert not evidence[8]["informative"] and len(evidence[8]["candidates"]) == 3
    assert r["drift"]["n"] == 2 and r["drift"]["slope_codes_per_minute"] > 0
    assert len(r["records"][7]["changes_per_pulse"]) == 2
    assert r["records"][2]["asynchronous_to_observer_evidence"]
    assert r["records"][2]["changed_bit_masks"] == [(1767, 1), (1768, 2)]
    assert not parse(source.rsplit("\r\n", 1)[0])[0].done
    assert len(parse(source + "\n" + source)) == 2
    startup = synthetic_window(0, 0, 2, 0x40, [])
    # Attach at two seconds: only a later replay is needed; no early bytes.
    replay_prefix = ["Y4C00000000000341", "partial line",
                     record("I", 0x40, 2*HZ, 0), *startup,
                     record("I", 0x40, 5*HZ//2, 0), *startup]
    replayed = "\n".join(replay_prefix + lines[1:] + [
        record("I", 0x40, 70*HZ, len(cases)), *startup,
        record("Z", len(cases), 70*HZ+1)])
    replay_run, = parse(replayed)
    assert replay_run.startup_replays == 4 and len(replay_run.windows) == len(cases)
    assert analyse(replay_run)["drift"] == r["drift"]
    assert analyse(replay_run)["complete"]
    # An even later reader recovers startup but must flag missed measurements.
    late = [record("I", 0x40, 60*HZ, 9), *startup]
    for seq in (9, 10):
        kind, changes, base = cases[seq]
        late.extend(synthetic_window(seq, kind, seq*HZ*6, base, changes))
    late.append(record("Z", len(cases), 70*HZ))
    late_run, = parse("\n".join(late))
    assert late_run.done and not analyse(late_run)["complete"]
    assert late_run.windows[0].seq == 0 and late_run.windows[1].seq == 9
    assert any("Late attachment" in w for w in late_run.warnings)
    bad_sources = [source.replace("Y4C0000000106E641", "Y4C0000000106E642"),
                   source.replace("Y4C0000000106E641\r\n", ""),
                   source.replace("Y4I40", "Y4I4G"),
                   source.replace("Y4B00000001", "Y4B00000002", 1),
                   source + "\n" + record("I", 0x41, 70*HZ, len(cases)),
                   source + "\n" + "\n".join([
                       record("I", 0x40, 70*HZ, len(cases)),
                       *synthetic_window(0, 0, 2, 0x40, [(5, 0x41)])])]
    for bad in bad_sources:
        assert bad != source
        try:
            parse(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("corrupt capture accepted")
    assert predicted("ignore", [16, 2516], 1750) == [1766, 4266]
    assert predicted("restart", [16, 266], 1750) == [2016]
    assert not compatible([100, 101], [100], 4)
    import contextlib
    import io
    counter = [(19, 0)] + [(100+24*(v-1), v) for v in range(1, 31)] + [(838, 0x9E)]
    counter.insert(4, (171, 7))  # A one-tick skew value before the persistent 04.
    counter_log = [record("I", 0x9E, HZ, 0),
                   *synthetic_window(0, 0, 2, 0x9E, []),
                   *synthetic_window(1, 1, 4*HZ, 0x9E, counter),
                   record("Z", 2, 5*HZ)]
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        board_reading(parse("\n".join(counter_log)))
    assert "in 1/1 isolated windows" in output.getvalue()
    assert "physical median=32.740 us" in output.getvalue()
    assert "One-tick dwell events: 1" in output.getvalue()
    assert "with only old/new bit mixtures: 1/1" in output.getvalue()
    assert "01: 83/83/83; spread=0" in output.getvalue()
    assert "Two-valued fallback: 00" in output.getvalue()
    print("HY04 selftest: PASS (initial, timing, bus steps, hidden conversions, all three retrigger models, drift, delayed attachment, startup deduplication, repeated Z, reloads, truncation and corrupt replays)")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("capture", nargs="?", help="UART text file; default stdin")
    p.add_argument("--json", action="store_true", help="include every timed event and model prediction")
    p.add_argument("--settle-gap", type=int, default=4, help="maximum adjacent-change gap within a cluster (ticks)")
    p.add_argument("--selftest", action="store_true")
    p.add_argument("--board-reading", action="store_true", help="compare exact sequences, rise/fall offsets and restart evidence")
    a = p.parse_args()
    if a.selftest:
        selftest()
        return
    if a.settle_gap < 0:
        p.error("--settle-gap must be nonnegative")
    try:
        content = Path(a.capture).read_text() if a.capture else sys.stdin.read()
        runs = parse(content)
        reports = [analyse(run, a.settle_gap) for run in runs]
    except (OSError, ValueError) as exc:
        p.exit(2, f"HY04: {exc}\n")
    if a.board_reading:
        board_reading(runs)
    elif a.json:
        print(json.dumps(reports, indent=2))
    else:
        for index, report in enumerate(reports, 1):
            print_report(report, index)


if __name__ == "__main__":
    main()
