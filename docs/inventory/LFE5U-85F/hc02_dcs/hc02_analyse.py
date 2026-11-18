"""Analyse HC02 UART captures without third-party modules.

Usage: python3 hc02_analyse.py capture.log [--timelines] [--summary-only]
       python3 hc02_analyse.py capture.log --compare-rules [--rule documented]
       python3 hc02_analyse.py --selftest
Accepts raw D2 lines or existing runners' `load ... distinct=[...]` records.
Every complete record is analysed (no deduplication). Truncated D2 records,
bad hex/checksums/controls and inconsistent level metadata are errors.
Unrelated log lines are ignored. Exit 0: no departures; 1: departures or
unqualified/unresolved observations; 2: bad input/no records. MODESEL=1 runts are counted
but are not themselves departures from its non-glitchless specification.

Times are in 5 ns ticks, relative to the physical control write, T-1.
Counter observation k estimates physical time k-4.5 (see hc02_gen.py).
--tolerance defaults to one tick for relative CDC/route uncertainty. First
eligible old/new edges within that tolerance of the boundary are alternatives,
not silently forced into a particular order. A fit requires ALL expected edges
and NO extra edges, including the pre-change region and the settled tail.
Default measured rule: follow old through its first rising edge (POS), hold
high through the SECOND subsequent new rising edge, then follow new. NEG:
the same using falling edges/hold low. --rule documented uses the first new
edge. --compare-rules compares 48 fixed rules without tuning them per event:
old/new active or opposite polarity, old index -1/0/+1, new skip 0/1/2/3.
The new handshake edge is inferred; the first opposite edge is observable.
Static tables are checked separately after 128 settling samples, including
00/11 and MODESEL=1. Transients involving 00/11, static modes or MODESEL changes
are reported but not assigned an undocumented handshake rule.

Runt: two adjacent UNAMBIGUOUS opposite-polarity edge bins shorter than the shortest
observed source phase of that level minus twice the matching tolerance.
Counts >1 and same-bin opposite edges are unordered packets, not zero-width
pulses. Raw legacy runt/repeated-polarity counts are retained for comparison;
resolved repeated polarities are counted only between two single-edge bins.
This is a sampled digital experiment: absence of detected runts does not prove
an analog minimum pulse width, and the modulo-16 counters can alias bursts.
"""

import argparse
import ast
from collections import Counter, defaultdict
from dataclasses import dataclass
import json
import math
from pathlib import Path
import re
import sys

from hc02_gen import LINE_BYTES, MODES, PAYLOAD_END, PAYLOAD_START, SAMPLES

TICK_NS = 5.0
SYNC_CENTRE = 4.5


@dataclass
class Sample:
    control: int
    levels: int
    valid: int
    lock: int
    delta: tuple


@dataclass
class Record:
    variant: str
    sequence: int
    trigger: int
    seed: int
    samples: list

    @property
    def change(self):
        return self.trigger - 1


@dataclass(frozen=True)
class SwitchRule:
    old_flip: int = 0
    new_flip: int = 0
    old_shift: int = 0
    new_skip: int = 1

    @property
    def label(self):
        return (f"old={'O' if self.old_flip else 'A'}{self.old_shift:+d},"
                f"new={'O' if self.new_flip else 'A'}+{self.new_skip}")


MEASURED = SwitchRule()
DOCUMENTED = SwitchRule(new_skip=0)


def candidate_rules():
    return [SwitchRule(old_flip, new_flip, old_shift, new_skip)
            for old_flip in (0, 1) for new_flip in (0, 1)
            for old_shift in (-1, 0, 1) for new_skip in range(4)]


def decode_word(word):
    if word >> 44:
        raise ValueError("nonzero reserved sample bits")
    return Sample((word >> 40) & 7, (word >> 32) & 15,
                  (word >> 36) & 15, (word >> 43) & 1,
                  tuple((word >> (4 * i)) & 15 for i in range(8)))


def encode_word(s):
    return ((s.lock << 43) | (s.control << 40) | (s.valid << 36) |
            (s.levels << 32) | sum(d << (4 * i) for i, d in enumerate(s.delta)))


def parse(line):
    line = line.rstrip("\r\n")
    if len(line) != LINE_BYTES - 2:
        raise ValueError(f"line length {len(line)}; expected {LINE_BYTES-2} before CR/LF")
    if not re.fullmatch(r"D2[01][0-9A-Fa-f]+", line):
        raise ValueError("bad protocol/version/variant/hex")
    variant = "static" if line[2] == "1" else "dynamic"
    sequence, trigger, seed = int(line[3:11], 16), int(line[11:14], 16), int(line[14:22], 16)
    if not 128 <= trigger <= 255 or seed == 0:
        raise ValueError("trigger outside 128..255 or zero LFSR seed")
    words = [int(line[i:i+12], 16) for i in range(PAYLOAD_START, PAYLOAD_END, 12)]
    checksum = 0
    for word in words:
        checksum ^= word
    if checksum != int(line[PAYLOAD_END:], 16):
        raise ValueError("sample XOR checksum mismatch")
    samples = [decode_word(w) for w in words]
    changes = [i for i in range(1, SAMPLES) if samples[i].control != samples[i-1].control]
    if changes != [trigger]:
        raise ValueError(f"control changes {changes}; header says [{trigger}]")
    for k, s in enumerate(samples):
        for channel in range(4):
            rise, fall = s.delta[2*channel:2*channel+2]
            valid, level = (s.valid >> channel) & 1, (s.levels >> channel) & 1
            if rise and fall:
                good = not valid
            elif rise or fall:
                good = valid and level == int(bool(rise))
            elif k:
                previous = samples[k-1]
                good = (valid == ((previous.valid >> channel) & 1) and
                        level == ((previous.levels >> channel) & 1))
            else:
                good = True
            if not good:
                raise ValueError(f"inconsistent observer metadata at sample {k}, channel {channel}")
    return Record(variant, sequence, trigger, seed, samples)


def format_record(rec):
    words = [encode_word(s) for s in rec.samples]
    checksum = 0
    for word in words:
        checksum ^= word
    return (f"D2{int(rec.variant == 'static')}{rec.sequence:08X}{rec.trigger:03X}{rec.seed:08X}" +
            "".join(f"{w:012X}" for w in words) + f"{checksum:012X}\r\n")


def input_lines(path):
    with (sys.stdin if str(path) == "-" else open(path, encoding="utf-8")) as stream:
        for number, row in enumerate(stream, 1):
            if row.startswith("D2"):
                yield number, row
            elif row.startswith("load ") and "distinct=" in row:
                try:
                    lines = ast.literal_eval(row.split("distinct=", 1)[1].strip())
                    if not isinstance(lines, (list, tuple)) or not all(isinstance(s, str) for s in lines):
                        raise ValueError("distinct must be a list of strings")
                except (SyntaxError, ValueError) as exc:
                    raise ValueError(f"{path}:{number}: invalid distinct list: {exc}") from exc
                for line in lines:
                    if line.startswith("D2"):
                        yield number, line


def timeline(rec, channel):
    return [(k-SYNC_CENTRE, polarity)
            for k, s in enumerate(rec.samples)
            for polarity in (0, 1)  # 0=rise, 1=fall; same-bin order is unknown
            for _ in range(s.delta[2*channel+polarity])]


def source_or_level(mode, control):
    sel, bypass = control & 3, control >> 2
    if bypass or mode in ("POS", "NEG"):
        return ("CLK0" if sel == 1 else "CLK1" if sel == 2 else
                int(mode == "POS" and not bypass))
    if mode in ("CLK0", "CLK1"):
        return mode
    if mode in ("LOW", "HIGH"):
        return int(mode == "HIGH")
    source, level = mode.split("_")
    return source if sel == (1 if source == "CLK0" else 2) else int(level == "HIGH")


def match(expected, actual, tolerance):
    """Match edges one-to-one, in order within each polarity, within tolerance."""
    used = set()
    missing = []
    pairs = []
    for edge in expected:
        options = [i for i, other in enumerate(actual)
                   if i not in used and edge[1] == other[1] and abs(edge[0]-other[0]) <= tolerance]
        if options:
            # Earliest eligible pairing preserves maximum ordered matches.
            i = options[0]
            used.add(i)
            pairs.append((edge, actual[i]))
        else:
            missing.append(edge)
    return missing, [e for i, e in enumerate(actual) if i not in used], pairs


def bounded_match(expected, actual, tolerance):
    # Match before cropping: different CDC bins at a window boundary must
    # not turn the same physical edge into a spurious missing/extra pair.
    missing, extra, pairs = match(expected, actual, tolerance)
    lo, hi = -SYNC_CENTRE+2*tolerance, SAMPLES-1-SYNC_CENTRE-2*tolerance
    return ([e for e in missing if lo <= e[0] <= hi],
            [e for e in extra if lo <= e[0] <= hi], pairs)


def first_candidates(edges, after, tolerance):
    candidates = [e for e in edges if e[0] >= after-tolerance]
    definite = next((e[0] for e in candidates if e[0] > after+tolerance), math.inf)
    return [e for e in candidates if e[0] <= definite]


def fit_switch(rec, channel, mode, tolerance, rule=MEASURED):
    old, new = rec.samples[0].control & 3, rec.samples[-1].control & 3
    a, b = timeline(rec, old-1), timeline(rec, new-1)
    output = timeline(rec, channel)
    polarity = int(mode == "NEG")
    old_polarity, new_polarity = polarity ^ rule.old_flip, polarity ^ rule.new_flip
    old_edges = [e for e in a if e[1] == old_polarity]
    new_edges = [e for e in b if e[1] == new_polarity]
    candidates = []
    for first_old in first_candidates(old_edges, rec.change, tolerance):
        old_index = old_edges.index(first_old) + rule.old_shift
        if not 0 <= old_index < len(old_edges):
            continue
        stop = old_edges[old_index]
        for first_new in first_candidates(new_edges, stop[0], tolerance):
            new_index = new_edges.index(first_new) + rule.new_skip
            if not 0 <= new_index < len(new_edges):
                continue
            start = new_edges[new_index]
            expected = ([e for e in a if e[0] <= stop[0]] +
                        [e for e in b if e[0] > start[0] or
                         (e == start and old_polarity != new_polarity)])
            expected.sort()
            missing, extra, pairs = bounded_match(expected, output, tolerance)
            candidates.append((len(missing)+len(extra), stop, start, first_new, missing, extra, pairs))
    if not candidates:
        return {"fit": False, "reason": "no old/new handshake edges within window", "missing": [], "extra": []}
    candidates.sort(key=lambda c: (c[0], c[1][0], c[2][0]))
    cost, stop, start, first_new, missing, extra, pairs = candidates[0]
    resume = (start if old_polarity != new_polarity else
              next((e for e in b if e[0] > start[0]), None))
    measured_stop = next((o for e, o in pairs if e == stop), None)
    measured_resume = next((o for e, o in pairs if e == resume), None)
    before_stop = [e for e in output if measured_stop and e[0] < measured_stop[0]]
    after_resume = [e for e in output if measured_resume and e[0] > measured_resume[0]]
    return {"fit": cost == 0, "rule": rule.label,
            "old_source": f"CLK{old-1}", "new_source": f"CLK{new-1}",
            "stop": stop, "start": start, "resume": resume,
            "first_new": first_new,
            "measured_stop": measured_stop, "measured_resume": measured_resume,
            "hold_level": 1-old_polarity,
            "hold_ticks": (measured_resume[0]-measured_stop[0]
                           if measured_stop and measured_resume else None),
            "handshake_ticks": start[0]-stop[0],
            "old_wait_ticks": stop[0]-rec.change,
            "resume_latency_ticks": measured_resume[0]-rec.change if measured_resume else None,
            "terminating_phase_ticks": (measured_stop[0]-before_stop[-1][0]
                                        if measured_stop and before_stop else None),
            "resumed_phase_ticks": (after_resume[0][0]-measured_resume[0]
                                    if measured_resume and after_resume else None),
            "near_new_active_edge": any(abs(e[0]-stop[0]) <= tolerance for e in new_edges),
            "near_old_active_edge": any(abs(e[0]-rec.change) <= tolerance for e in old_edges),
            "positive_near_old_edge": any(0 < e[0]-rec.change <= tolerance for e in old_edges),
            "boundary_alternatives": sum(c[0] == 0 for c in candidates),
            "missing": missing, "extra": extra}


def steady(rec, channel, mode, control, begin, end, tolerance):
    expected = source_or_level(mode, control)
    samples = rec.samples[begin:end]
    if not all(s.valid & (1 << channel) for s in samples):
        return {"expected": expected, "fit": False, "reason": "unanchored/ambiguous level"}
    edges = [e for e in timeline(rec, channel) if begin <= e[0]+SYNC_CENTRE < end]
    if isinstance(expected, int):
        bad_levels = sum(((s.levels >> channel) & 1) != expected for s in samples)
        return {"expected": expected, "fit": not edges and not bad_levels,
                "edges": len(edges), "bad_levels": bad_levels}
    source = int(expected[-1])
    # Remove a boundary margin from both sets so CDC bins at the ends cannot
    # manufacture missing edges. Keep reference candidates one tolerance wider.
    lo, hi = begin-SYNC_CENTRE+tolerance, end-1-SYNC_CENTRE-tolerance
    actual = [e for e in timeline(rec, channel) if lo <= e[0] <= hi]
    reference = [e for e in timeline(rec, source) if lo-tolerance <= e[0] <= hi+tolerance]
    missing, extra, pairs = match(reference, actual, tolerance)
    missing = [e for e in missing if lo+tolerance <= e[0] <= hi-tolerance]
    # Check levels where there are no source edges nearby; this detects a
    # permanently inverted or flat output even in short edge-free regions.
    bad_levels = 0
    for k in range(begin, end):
        if any(abs((k-SYNC_CENTRE)-e[0]) <= tolerance for e in reference):
            continue
        s = rec.samples[k]
        if s.valid & (1 << source):
            bad_levels += ((s.levels >> source) ^ (s.levels >> channel)) & 1
    return {"expected": expected, "fit": not missing and not extra and not bad_levels,
            "missing": len(missing), "extra": len(extra), "bad_levels": bad_levels,
            "matched": len(pairs)}


def value_at(rec, target, time):
    if isinstance(target, int):
        return target
    channel = int(target[-1])
    edges = timeline(rec, channel)
    previous = [e for e in edges if e[0] <= time]
    if previous:
        return 1-previous[-1][1]
    return (rec.samples[0].levels >> channel) & 1


def observed_output(rec, channel, begin, end, tolerance):
    samples = rec.samples[begin:end]
    if not all(s.valid & (1 << channel) for s in samples):
        return "ambiguous"
    edges = sum(sum(s.delta[2*channel:2*channel+2]) for s in samples)
    levels = {(s.levels >> channel) & 1 for s in samples}
    if not edges and len(levels) == 1:
        return str(next(iter(levels)))
    matches = [f"CLK{source}" for source in (0, 1)
               if steady(rec, channel, f"CLK{source}", 0, begin, end, tolerance)["fit"]]
    return "/".join(matches) if matches else "other edges"


def fit_bypass(rec, channel, mode, tolerance):
    old = source_or_level(mode, rec.samples[0].control)
    new = source_or_level(mode, rec.samples[-1].control)
    expected = []
    for source, before in ((old, True), (new, False)):
        if isinstance(source, str):
            expected.extend(e for e in timeline(rec, int(source[-1]))
                            if (e[0] < rec.change) == before)
    old_level, new_level = value_at(rec, old, rec.change), value_at(rec, new, rec.change)
    if old_level != new_level:
        expected.append((rec.change, 1-new_level))
    # Edges within one tolerance of the control write have unresolved physical
    # order. Exclude that small region from strict bypass fitting and report it.
    expected.sort()
    uncertain = lambda e: abs(e[0]-rec.change) <= tolerance
    missing, extra, _ = bounded_match([e for e in expected if not uncertain(e)],
                                     [e for e in timeline(rec, channel) if not uncertain(e)], tolerance)
    return {"fit": not missing and not extra, "missing": missing, "extra": extra,
            "boundary_edges": sum(uncertain(e) for e in timeline(rec, channel))}


def pulse_findings(rec, channel, tolerance):
    minimum = {}
    for level in (0, 1):
        widths = [b[0]-a[0] for source in (0, 1)
                  for a, b in zip(timeline(rec, source), timeline(rec, source)[1:])
                  if a[1] == 1-level and b[1] == level and b[0] > a[0]]
        minimum[level] = min(widths) if widths else None
    events = timeline(rec, channel)
    raw_runts = [(a[0], 1-a[1], b[0]-a[0]) for a, b in zip(events, events[1:])
             if a[1] != b[1] and minimum[1-a[1]] is not None and
             b[0]-a[0] < minimum[1-a[1]]-2*tolerance]
    bins = [(k-SYNC_CENTRE, s.delta[2*channel], s.delta[2*channel+1])
            for k, s in enumerate(rec.samples) if sum(s.delta[2*channel:2*channel+2])]
    packets = [{"time": t, "rise": rise, "fall": fall} for t, rise, fall in bins if rise+fall != 1]
    runts = [(a[0], int(bool(a[1])), b[0]-a[0]) for a, b in zip(bins, bins[1:])
             if a[1]+a[2] == b[1]+b[2] == 1 and a[1] != b[1] and
             minimum[int(bool(a[1]))] is not None and
             b[0]-a[0] < minimum[int(bool(a[1]))]-2*tolerance]
    repeated = sum(a[1]+a[2] == b[1]+b[2] == 1 and a[1] == b[1]
                   for a, b in zip(bins, bins[1:]))
    # Regardless of unknown within-bin ordering, a binary clock's cumulative
    # rises-minus-falls can only change its anchored level between 0 and 1.
    # A multi-edge packet cannot explain a sustained net level of -1 or 2.
    balance = None
    if rec.samples[0].valid & (1 << channel):
        count_level = (rec.samples[0].levels >> channel) & 1
        violations = []
        for k, s in enumerate(rec.samples[1:], 1):
            count_level += s.delta[2*channel]-s.delta[2*channel+1]
            if count_level not in (0, 1) or (s.valid & (1 << channel) and
                                           count_level != ((s.levels >> channel) & 1)):
                violations.append((k-SYNC_CENTRE, count_level))
        balance = {"invalid_samples": len(violations),
                   "first_time": violations[0][0] if violations else None,
                   "last_time": violations[-1][0] if violations else None,
                   "invalid_level_min": min(v for _, v in violations) if violations else None,
                   "invalid_level_max": max(v for _, v in violations) if violations else None}
    return {"runts": runts, "raw_runts": raw_runts,
            "ambiguous_packets": packets, "count_balance": balance, "source_min_phase_ticks": minimum,
            "same_bin": sum(bool(s.delta[2*channel] and s.delta[2*channel+1]) for s in rec.samples),
            "multiple": sum(s.delta[2*channel] > 1 or s.delta[2*channel+1] > 1 for s in rec.samples),
            "repeated_polarity": repeated,
            "raw_repeated_polarity": sum(a[1] == b[1] for a, b in zip(events, events[1:]))}


def analyse(rec, tolerance=1.0, rule=MEASURED):
    quality = []
    if not all(s.lock for s in rec.samples):
        quality.append("PLL LOCK low")
    for source in (0, 1):
        events = timeline(rec, source)
        if len(events) < 10 or any(a[1] == b[1] for a, b in zip(events, events[1:])):
            quality.append(f"CLK{source} missing/nonalternating edges")
        if any(s.delta[2*source] > 1 or s.delta[2*source+1] > 1 or
               (s.delta[2*source] and s.delta[2*source+1]) for s in rec.samples):
            quality.append(f"CLK{source} observer overrun")
        # Nominal source periods: 32 and 128/3 fast ticks. Do not fit a
        # switching sequence against stopped or substantially wrong sources.
        period = 32 if source == 0 else 128/3
        for polarity in (0, 1):
            times = [t for t, p in events if p == polarity]
            if any(abs(b-a-period) > 2*tolerance+1 for a, b in zip(times, times[1:])):
                quality.append(f"CLK{source} unexpected period")
                break
    old, new = rec.samples[0].control, rec.samples[-1].control
    results = []
    for site, mode in enumerate(MODES[rec.variant]):
        channel = site+2
        pre = steady(rec, channel, mode, old, 16, rec.trigger-16, tolerance)
        post = steady(rec, channel, mode, new, rec.trigger+128, SAMPLES, tolerance)
        post["observed"] = observed_output(rec, channel, rec.trigger+128, SAMPLES, tolerance)
        post["rise_count"] = sum(s.delta[2*channel] for s in rec.samples[rec.trigger+128:])
        post["fall_count"] = sum(s.delta[2*channel+1] for s in rec.samples[rec.trigger+128:])
        findings = pulse_findings(rec, channel, tolerance)
        if mode in ("POS", "NEG") and old >> 2 == new >> 2 == 0 and {old & 3, new & 3} == {1, 2}:
            kind, fit = "source-switch", fit_switch(rec, channel, mode, tolerance, rule)
        elif old >> 2 == new >> 2 == 1:
            kind, fit = "bypass", fit_bypass(rec, channel, mode, tolerance)
        else:
            kind, fit = "steady-only", {"fit": True, "missing": [], "extra": []}
        # Do not claim glitchlessness for unmodelled transitions. Always list
        # their pulse findings; only documented source switches promise it here.
        departure = (not fit["fit"] or not pre["fit"] or not post["fit"] or
                     (kind == "source-switch" and any(findings[k] for k in
                      ("runts", "same_bin", "multiple", "repeated_polarity"))))
        results.append({"site": site, "mode": mode, "kind": kind, "pre": pre, "post": post,
                        "sequence_fit": fit, **findings, "departure": bool(departure),
                        "qualified": not quality})
    return {"variant": rec.variant, "sequence": rec.sequence, "seed": rec.seed,
            "tolerance_ticks": tolerance,
            "trigger": rec.trigger, "old_control": old, "new_control": new,
            "quality": quality, "results": results}


def edge_text(edge, rec):
    if edge is None:
        return "unresolved"
    return f"{'rise' if edge[1] == 0 else 'fall'}@{edge[0]-rec.change:+g}t"


def print_event(rec, result, timelines=False):
    origin = f"{result['input']} load-row={result['load_row']} " if "input" in result else ""
    print(f"{origin}{rec.variant} Q={rec.sequence:08X} seed={rec.seed:08X} "
          f"SEL {result['old_control']&3:02b}->{result['new_control']&3:02b} "
          f"MODESEL {result['old_control']>>2}->{result['new_control']>>2} "
          f"T={rec.trigger} (times relative to control write, 1t=5ns)")
    if result["quality"]:
        print("  UNQUALIFIED: " + "; ".join(result["quality"]))
    for r in result["results"]:
        fit = r["sequence_fit"]
        unresolved = r["ambiguous_packets"] or (r["count_balance"] and r["count_balance"]["invalid_samples"])
        status = ("UNQUALIFIED" if not r["qualified"] else "DEPARTURE" if r["departure"] else
                  "UNRESOLVED observer" if unresolved else "OK")
        print(f"  DCS{r['site']} {r['mode']} {r['kind']}: {status}; "
              f"steady {r['pre']['expected']}->{r['post']['expected']} "
              f"pre={'fit' if r['pre']['fit'] else 'FAIL'} post={'fit' if r['post']['fit'] else 'FAIL'}; "
              f"observed tail={r['post']['observed']} "
              f"rises/falls={r['post']['rise_count']}/{r['post']['fall_count']}")
        if "stop" in fit:
            hold = fit["hold_ticks"]
            duration = "unresolved" if hold is None else f"{hold:g}t/{hold*TICK_NS:g}ns"
            print(f"    old ends: {fit['old_source']} {edge_text(fit['stop'], rec)}; "
                  f"DCSOUT {edge_text(fit['measured_stop'], rec)}; holds {fit['hold_level']} for {duration}")
            print(f"    rule {fit['rule']}; first qualifying new edge "
                  f"{edge_text(fit['first_new'], rec)}; release handshake: "
                  f"{fit['new_source']} {edge_text(fit['start'], rec)} "
                  f"(inferred, wait={fit['handshake_ticks']:g}t); "
                  f"first visible new edge {edge_text(fit['resume'], rec)}, "
                  f"DCSOUT {edge_text(fit['measured_resume'], rec)}; "
                  f"fitting edge-order alternatives={fit['boundary_alternatives']}")
            print(f"    phase before hold={fit['terminating_phase_ticks']}t; "
                  f"first resumed phase={fit['resumed_phase_ticks']}t; "
                  f"old/new active edges within tolerance={fit['near_new_active_edge']}")
        elif r["kind"] == "steady-only":
            events = timeline(rec, r["site"]+2)
            before = [e for e in events if e[0] < rec.change]
            after = [e for e in events if e[0] >= rec.change]
            print(f"    transition rule unspecified; last pre-write edge "
                  f"{edge_text(before[-1] if before else None, rec)}, "
                  f"first post-write edge {edge_text(after[0] if after else None, rec)}")
        print(f"    missing={len(fit['missing'])} extra={len(fit['extra'])} "
              f"runts={len(r['runts'])} same-bin={r['same_bin']} "
              f"multi-count={r['multiple']} repeated-polarity={r['repeated_polarity']} "
              f"unordered-packets={len(r['ambiguous_packets'])} "
              f"legacy-runt-count={len(r['raw_runts'])} "
              f"legacy-repeated-count={r['raw_repeated_polarity']}")
        for label, edges in (("missing", fit["missing"]), ("extra", fit["extra"])):
            if edges:
                print(f"    {label}: " + ", ".join(edge_text(e, rec) for e in edges))
        for time, level, width in r["runts"]:
            print(f"    short phase level={level} start={time-rec.change:+g}t "
                  f"width={width:g}t/{width*TICK_NS:g}ns "
                  f"(separate single-edge bins; nominal width uncertainty +/-5ns, "
                  f"conservative +/-{2*result['tolerance_ticks']*TICK_NS:g}ns)")
        for packet in r["ambiguous_packets"]:
            print(f"    UNRESOLVED packet @{packet['time']-rec.change:+g}t: "
                  f"rise={packet['rise']} fall={packet['fall']}; no within-bin order or pulse width inferred")
        balance = r["count_balance"]
        if balance and balance["invalid_samples"]:
            print(f"    OBSERVER INCONSISTENT: cumulative rises-minus-falls implies level "
                  f"{balance['invalid_level_min']}..{balance['invalid_level_max']} "
                  f"over {balance['invalid_samples']} samples, "
                  f"{balance['first_time']-rec.change:+g}..{balance['last_time']-rec.change:+g}t; "
                  "this cannot be a faithful binary-clock edge timeline")
    if timelines:
        for channel, name in enumerate(("CLK0", "CLK1", "DCS0", "DCS1")):
            values = []
            for k, s in enumerate(rec.samples):
                rise, fall = s.delta[2*channel:2*channel+2]
                if not rise+fall:
                    continue
                if rise+fall == 1:
                    values.append(edge_text((k-SYNC_CENTRE, int(bool(fall))), rec))
                else:
                    values.append(f"{{rise={rise},fall={fall},unordered}}@{k-SYNC_CENTRE-rec.change:+g}t")
            print(f"  {name}:" + (" " + " ".join(values) if values else " (no edges)"))


def synthetic(mode="dynamic", old=1, new=2, trigger=180, phase=0, fault=None, new_skip=1):
    """Synthetic physical timelines, then the four-to-five-tick observer delay.

    This fixture is independent of fit_switch: it evolves output levels at
    physical source edges, advancing wait-old/wait-new/follow-new states.
    """
    change = trigger-1
    source_edges = []
    source_levels = [0, 0]
    all_events = defaultdict(list)
    for source, half in enumerate((16, 64/3)):
        for n in range(-4, 40):
            time = phase + (3 if source == 0 else 11) + n*half
            source_edges.append((time, source, n & 1))
    source_edges.append((change, -1, -1))
    source_edges.sort()
    levels = [0, 0, 0, 0]
    state = ["old", "old"]
    new_count = [0, 0]
    physical = []
    for time, source, polarity in source_edges:
        if time < -50:
            continue
        if source >= 0:
            source_levels[source] = 1-polarity
            physical.append((time, source, polarity))
        for site, dmode in enumerate(MODES[mode]):
            direct = (dmode in ("POS", "NEG") and old >> 2 == new >> 2 == 0 and
                      {old & 3, new & 3} == {1, 2})
            if direct:
                edge_polarity = int(dmode == "NEG")
                if time >= change and state[site] == "old":
                    state[site] = "wait-old"
                if state[site] == "wait-old" and source == (old & 3)-1 and polarity == edge_polarity:
                    state[site] = "wait-new"
                elif state[site] == "wait-new" and source == (new & 3)-1 and polarity == edge_polarity:
                    if new_count[site] == new_skip:
                        state[site] = "new"
                    else:
                        new_count[site] += 1
                if state[site] in ("old", "wait-old"):
                    value = source_levels[(old & 3)-1]
                elif state[site] == "wait-new":
                    value = int(dmode == "POS")
                else:
                    value = source_levels[(new & 3)-1]
            else:
                target = source_or_level(dmode, old if time < change else new)
                value = source_levels[int(target[-1])] if isinstance(target, str) else target
            if value != levels[site+2]:
                physical.append((time, site+2, 1-value))
                levels[site+2] = value
    for time, channel, polarity in physical:
        k = math.ceil(time)+4
        if 0 <= k < SAMPLES:
            all_events[k].append((channel, polarity))
    if fault:
        fault(all_events, change)
    samples = []
    # Infer initial levels from the first observed polarity; every fixture
    # starts in a running source state unless it is testing a constant output.
    levels = 0
    for channel in range(4):
        first = next((p for k in sorted(all_events) for ch, p in all_events[k] if ch == channel), None)
        if first == 1:
            levels |= 1 << channel
        elif first is None and source_or_level(MODES[mode][channel-2], old) == 1:
            levels |= 1 << channel
    valid = 15
    for k in range(SAMPLES):
        delta = [0]*8
        for channel, polarity in all_events[k]:
            delta[2*channel+polarity] += 1
        for channel in range(4):
            rise, fall = delta[2*channel:2*channel+2]
            if rise and fall:
                valid &= ~(1 << channel)
            elif rise or fall:
                valid |= 1 << channel
                levels = (levels | (1 << channel)) if rise else (levels & ~(1 << channel))
        samples.append(Sample(old if k < trigger else new, levels, valid, 1, tuple(delta)))
    return Record(mode, 7, trigger, 0x48430201, samples)


def selftest():
    cases = 0
    for old, new in ((1, 2), (2, 1)):
        for phase in (0, 0.25, 2, 5.5, 11, 17.75, 29):
            rec = parse(format_record(synthetic(old=old, new=new, phase=phase)))
            result = analyse(rec)
            assert not result["quality"], result
            assert all(not r["departure"] for r in result["results"]), result
            assert all(r["sequence_fit"]["hold_ticks"] > 0 for r in result["results"])
            cases += 1
    for old, new in ((1, 2), (2, 1)):
        for skip in range(4):
            rec = parse(format_record(synthetic(old=old, new=new, new_skip=skip)))
            rule = SwitchRule(new_skip=skip)
            result = analyse(rec, rule=rule)
            assert all(r["sequence_fit"]["fit"] for r in result["results"]), result
            measured_rec = synthetic(old=old, new=new)
            documented_fit = analyse(measured_rec, rule=DOCUMENTED)
            assert all(len(r["sequence_fit"]["missing"]) == 2 and
                       not r["sequence_fit"]["extra"] for r in documented_fit["results"])
            cases += 1
    assert len(set(candidate_rules())) == 48
    # A complete candidate sweep must select the right fixed rule, rather
    # than choosing a different delay after looking at each individual edge.
    for ch, mode in ((2, "POS"), (3, "NEG")):
        rec = synthetic()
        winners = [rule for rule in candidate_rules() if fit_switch(rec, ch, mode, 1, rule)["fit"]]
        assert MEASURED in winners, winners
        assert DOCUMENTED not in winners, winners
        cases += 1
    # All 80 entries, checked against a separately written truth table.
    documented = {
        "POS": (1, "CLK0", "CLK1", 1), "NEG": (0, "CLK0", "CLK1", 0),
        "CLK0_LOW": (0, "CLK0", 0, 0), "CLK0_HIGH": (1, "CLK0", 1, 1),
        "CLK1_LOW": (0, 0, "CLK1", 0), "CLK1_HIGH": (1, 1, "CLK1", 1),
        "CLK0": ("CLK0",)*4, "CLK1": ("CLK1",)*4, "LOW": (0,)*4, "HIGH": (1,)*4,
    }
    for dmode, table in documented.items():
        for sel in range(4):
            assert source_or_level(dmode, sel) == table[sel]
            assert source_or_level(dmode, sel | 4) == ("CLK0" if sel == 1 else "CLK1" if sel == 2 else 0)
    cases += 1
    for variant in MODES:
        for new in range(8):
            if new == 1:
                continue
            result = analyse(parse(format_record(synthetic(mode=variant, new=new))))
            assert all(r["post"]["fit"] for r in result["results"]), result
            cases += 1
    for old, new in ((5, 6), (6, 5), (5, 4), (5, 7), (4, 6)):
        for phase in (0, 0.25, 11):
            result = analyse(parse(format_record(synthetic(old=old, new=new, phase=phase))))
            assert all(not r["departure"] and r["kind"] == "bypass" for r in result["results"]), result
            cases += 1
    def jitter(events, change):
        moved = [(k, ch, p) for k in list(events) for ch, p in events[k] if ch >= 2]
        for k in list(events):
            events[k] = [(ch, p) for ch, p in events[k] if ch < 2]
        for k, ch, p in moved:
            events[k+((k+ch) & 1)].append((ch, p))
    for old, new in ((1, 2), (2, 1)):
        result = analyse(parse(format_record(synthetic(old=old, new=new, fault=jitter))))
        assert all(not r["departure"] for r in result["results"]), result
        cases += 1
    rec = synthetic(mode="static", new=0)
    for s in rec.samples:
        s.levels ^= 8  # A stable LOW where CLK1_HIGH requires HIGH.
    bad = analyse(parse(format_record(rec)))
    assert bad["results"][1]["departure"] and not bad["results"][1]["post"]["fit"], bad
    cases += 1
    def extra(events, change):
        # Add a full short pulse in the NEG low hold interval.
        for k in (change+30,):
            events[k].append((3, 0))
            events[k+1].append((3, 1))
    bad = analyse(parse(format_record(synthetic(fault=extra))))
    assert bad["results"][1]["departure"] and bad["results"][1]["runts"], bad
    cases += 1
    def missing(events, change):
        key = next(k for k in sorted(events) if k > change+70 and any(ch == 2 for ch, _ in events[k]))
        events[key] = [(ch, p) for ch, p in events[key] if ch != 2]
    bad = analyse(parse(format_record(synthetic(fault=missing))))
    assert bad["results"][0]["departure"] and bad["results"][0]["sequence_fit"]["missing"], bad
    cases += 1
    def same_bin(events, change):
        events[change+30].extend(((3, 0), (3, 1), (3, 0), (3, 1)))
    bad = analyse(parse(format_record(synthetic(fault=same_bin))))
    assert bad["results"][1]["same_bin"] and bad["results"][1]["multiple"], bad
    assert bad["results"][1]["ambiguous_packets"] and all(width > 0 for _, _, width in bad["results"][1]["runts"])
    cases += 1
    def inconsistent_counter(events, change):
        # One rise is reported a full period late: endpoint totals recover,
        # but unknown same-bin order cannot repair the impossible middle.
        keys = [k for k in sorted(events) if k > change+100 and (2, 0) in events[k]]
        first, second = keys[:2]
        events[first].remove((2, 0))
        events[second].append((2, 0))
    bad = analyse(parse(format_record(synthetic(fault=inconsistent_counter))))
    assert bad["results"][0]["count_balance"]["invalid_samples"] > 1, bad
    cases += 1
    rec = synthetic()
    rec.samples[400].lock = 0
    assert analyse(rec)["quality"] == ["PLL LOCK low"]
    cases += 1
    good_line = format_record(synthetic())
    malformed = [good_line[:-20], good_line[:25]+"Z"+good_line[26:],
                 good_line[:-4]+("0" if good_line[-4] != "0" else "1")+good_line[-3:]]
    rec = synthetic()
    rec.samples[200].control ^= 1
    malformed.append(format_record(rec))
    rec = synthetic()
    rec.samples[200].levels ^= 4
    malformed.append(format_record(rec))
    for line in malformed:
        try:
            parse(line)
        except ValueError:
            cases += 1
        else:
            raise AssertionError("malformed record accepted")
    print(f"selftest: {cases} cases passed (POS/NEG both directions, phase/boundary cases, "
          "48-rule sweep, new-clock delays 0..3, documented-vs-measured distinction, "
          "steady tables, runt/extra/missing/unordered packets, LOCK, corrupt/truncated records)")


def numeric_summary(values):
    values = sorted(v for v in values if v is not None)
    return ({"min": values[0], "median": values[len(values)//2], "max": values[-1]}
            if values else None)


def print_summary(summary):
    print("Summary: " + json.dumps(summary["counts"], sort_keys=True))
    print(f"Switch rule={summary['switch_rule']}, relative edge tolerance={summary['tolerance_ticks']:g}t")
    print("Records by input/load row: " + json.dumps(summary["records_by_input_row"], sort_keys=True))
    for key, value in sorted(summary["by_mode"].items()):
        print(f"  {key}: " + json.dumps(value, sort_keys=True))
    print("Settled outputs by MODESEL/SEL (only observed combinations):")
    for key, value in sorted(summary["steady_by_sel"].items()):
        print(f"  {key}: " + json.dumps(value, sort_keys=True))
    if summary["switch_by_direction"]:
        print("Switch measurements by direction (all durations in 5ns ticks; min/median/max):")
        for key, value in sorted(summary["switch_by_direction"].items()):
            print(f"  {key}: " + json.dumps(value, sort_keys=True))
    if summary["candidate_rules"]:
        print("Candidate rules: A=active (POS rising / NEG falling), O=opposite.")
        print("  old=X+j: j same-polarity periods relative to first eligible X after SEL.")
        print("  new=Y+k: skip k Y edges after first eligible Y following old stop.")
        print("  Follow old through stop, hold its resulting level, then follow new at release.")
        print("  Opposite-polarity release produces an immediate visible edge.")
        print("  Tolerance allows alternate order only within one specified tolerance;")
        print("  counts are best boundary-order fit of each FIXED rule, not a per-event choice of rule.")
        for key, value in sorted(summary["candidate_rules"].items(),
                                 key=lambda kv: (kv[0].split('/')[0], kv[1]["mismatch_events"],
                                                 kv[1]["missing"]+kv[1]["extra"], kv[0])):
            print(f"  {key}: " + json.dumps(value, sort_keys=True))
    print("Pulse counts use separate single-edge bins. Unordered packets have no inferred width/order;")
    print("legacy counts flatten rises before falls and can manufacture zero-width runts/repeated polarities.")
    print("MODESEL/static transients outside the source-switch rule remain steady-only checks.")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--tolerance", type=float, default=1.0, help="edge match tolerance in 5 ns ticks")
    parser.add_argument("--timelines", action="store_true", help="print every reconstructed edge")
    parser.add_argument("--summary-only", action="store_true")
    parser.add_argument("--json", action="store_true", help="emit event objects and summary as JSON")
    parser.add_argument("--rule", choices=("measured", "documented"), default="measured")
    parser.add_argument("--compare-rules", action="store_true", help="compare 48 fixed switch rules")
    args = parser.parse_args()
    if args.selftest:
        selftest()
        return 0
    if not args.paths or not math.isfinite(args.tolerance) or not 0 <= args.tolerance <= 4:
        parser.error("provide capture paths (or -), with a finite tolerance in 0..4")
    counts = Counter()
    by = defaultdict(Counter)
    tables = defaultdict(Counter)
    trials = defaultdict(Counter)
    direction_data = {}
    input_rows = Counter()
    errors = []
    rule = MEASURED if args.rule == "measured" else DOCUMENTED
    output = []
    for path in args.paths:
        try:
            for number, line in input_lines(path):
                try:
                    rec = parse(line)
                except ValueError as exc:
                    counts["bad_records"] += 1
                    errors.append({"input": str(path), "load_row": number, "error": str(exc)})
                    print(f"{path}:{number}: {exc}", file=sys.stderr)
                    continue
                result = analyse(rec, args.tolerance, rule)
                result.update(input=str(path), load_row=number)
                counts["records"] += 1
                input_rows[f"{path}:row{number}"] += 1
                counts["unqualified"] += bool(result["quality"])
                counts["unresolved_records"] += any(r["ambiguous_packets"] or
                    (r["count_balance"] and r["count_balance"]["invalid_samples"]) for r in result["results"])
                for r in result["results"]:
                    key = f"DCS{r['site']}/{r['mode']}/{r['kind']}"
                    by[key]["events"] += 1
                    by[key]["departures"] += r["departure"]
                    by[key]["unqualified"] += not r["qualified"]
                    for field in ("same_bin", "multiple", "repeated_polarity", "raw_repeated_polarity"):
                        by[key][field] += r[field]
                    by[key]["runts"] += len(r["runts"])
                    by[key]["raw_runts"] += len(r["raw_runts"])
                    by[key]["unordered_packets"] += len(r["ambiguous_packets"])
                    by[key]["counter_inconsistent_events"] += bool(
                        r["count_balance"] and r["count_balance"]["invalid_samples"])
                    by[key]["extra"] += len(r["sequence_fit"]["extra"])
                    by[key]["missing"] += len(r["sequence_fit"]["missing"])
                    counts["departures"] += r["departure"]
                    table_key = f"DCS{r['site']}/{r['mode']}/MODESEL={result['new_control']>>2}/SEL={result['new_control']&3:02b}"
                    tables[table_key]["events"] += 1
                    tables[table_key]["fit"] += r["post"]["fit"] and r["qualified"]
                    tables[table_key][f"expected={r['post']['expected']}"] += 1
                    tables[table_key][f"observed={r['post']['observed']}"] += 1
                    if r["kind"] == "source-switch" and r["qualified"]:
                        fit = r["sequence_fit"]
                        direction = f"{r['mode']}/{result['old_control']}->{result['new_control']}"
                        entry = direction_data.setdefault(direction, {
                            "events": 0, "fits": 0, "near_new_active_edge": 0,
                            "near_old_active_edge": 0, "positive_near_old_edge": 0,
                            "values": defaultdict(list)})
                        entry["events"] += 1
                        entry["fits"] += fit["fit"]
                        entry["near_new_active_edge"] += fit.get("near_new_active_edge", False)
                        entry["near_old_active_edge"] += fit.get("near_old_active_edge", False)
                        entry["positive_near_old_edge"] += fit.get("positive_near_old_edge", False)
                        for field in ("hold_ticks", "handshake_ticks", "old_wait_ticks", "resume_latency_ticks",
                                      "terminating_phase_ticks", "resumed_phase_ticks"):
                            entry["values"][field].append(fit.get(field))
                        if args.compare_rules:
                            for candidate in candidate_rules():
                                trial = fit_switch(rec, r["site"]+2, r["mode"], args.tolerance, candidate)
                                totals = trials[f"{r['mode']}/{candidate.label}"]
                                totals["events"] += 1
                                totals["mismatch_events"] += not trial["fit"]
                                totals["missing"] += len(trial["missing"])
                                totals["extra"] += len(trial["extra"])
                                totals["no_release_candidate"] += "stop" not in trial
                if args.json:
                    if not args.summary_only:
                        output.append(result)
                elif not args.summary_only:
                    print_event(rec, result, args.timelines)
        except (OSError, ValueError) as exc:
            counts["bad_records"] += 1
            print(str(exc), file=sys.stderr)
    directions = {key: {**{k: v for k, v in entry.items() if k != "values"},
                        **{field: numeric_summary(values) for field, values in entry["values"].items()}}
                  for key, entry in direction_data.items()}
    summary = {"counts": dict(counts), "by_mode": dict(by), "steady_by_sel": dict(tables),
               "switch_rule": args.rule, "tolerance_ticks": args.tolerance,
               "candidate_rules": dict(trials), "switch_by_direction": directions,
               "records_by_input_row": dict(input_rows), "input_errors": errors}
    if args.json:
        print(json.dumps({"events": output, "summary": summary}, indent=2, allow_nan=False))
    else:
        print_summary(summary)
    if counts["bad_records"] or not counts["records"]:
        return 2
    return int(bool(counts["departures"] or counts["unqualified"] or counts["unresolved_records"]))


if __name__ == "__main__":
    raise SystemExit(main())
