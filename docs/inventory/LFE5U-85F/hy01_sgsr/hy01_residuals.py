"""Locate EVERY exact residual of HY01's joint kr=2 model or raw-OR-sync2 candidate.

Usage: python3 -B hy01_residuals.py observations.log observations_s.log
       python3 -B hy01_residuals.py --candidate observations*.log
       python3 -B hy01_residuals.py --selftest
Window IDs are zero-based, deduplicated in first-seen order, as in the analyser.
Offsets and edge counts are observations in 10 ns bins, not analog delays.
"""
import argparse
from collections import Counter, defaultdict
from pathlib import Path
import re
import sys
sys.dont_write_bytecode = True
from hy01_analyse import GROUPS, Model, Window, changes, parse, predict, reset_trace, stable_start

MODEL = Model('async-assert/sync-release', 2, 0, True, False)


def reset_transitions(w):
    """Raw pin and plain s2 transitions, before reset-distribution delay.

    Keep the paths separate: a raw transition can be masked by s2 at the OR.
    A residual must follow a transition, rather than precede a future one.
    """
    n, reference = w.signal(14), w.signal(11)
    transitions = [(t, 'GSR_N-rise' if n[t] else 'GSR_N-fall') for t in changes(n)]
    s1 = s2 = 1
    for t in changes(reference):
        s1, new_s2 = 1-n[t], s1
        if new_s2 != s2:
            transitions.append((t, 's2-rise' if new_s2 else 's2-fall'))
        s2 = new_s2
    return sorted(transitions)


def nearest_transition(transitions, tick):
    preceding = [(tick-t, kind, t) for t, kind in transitions if t <= tick]
    assert preceding, ('no preceding reset transition', tick)
    # Prefer s2 when raw and synchronized transitions share a bin (phase 0).
    distance, kind, origin = min(preceding, key=lambda row: (row[0], not row[1].startswith('s2')))
    assert distance <= 2, ('residual outside 20 ns hazard window', tick, origin, kind, distance)
    return kind, distance, origin


def raw_or_sync_predict(w, group, regset):
    """Candidate: raw active OR s2, plain rising-edge stages initialized to 1.

    Markers toggle at each physical edge; marker 0 is SGSR CLK rising only.
    Sample the new raw input (the observed new-input bin convention). Captures
    see old s2, then asynchronous reset sees the updated s2. In particular, a
    raw release must not keep a capture reset just because the prior bin was
    active. There is no asynchronous preset of either synchronizer stage.
    """
    n, reference = w.signal(14), w.signal(11)
    local = w.signal(11+group)
    s1 = s2 = 1
    q = w.signal(5+2*group+regset)[0]
    result = []
    for t, pin in enumerate(n):
        raw = 1-pin
        edge = t and local[t] != local[t-1]
        if raw or s2:
            q = regset
        elif edge:
            q = 1-local[t]
        if t and reference[t] != reference[t-1]:
            s1, s2 = raw, s1
        if raw or s2:
            q = regset
        result.append(q)
    return result


def boot_offsets(with_sgsr, falling_first):
    """Run the proposed plain stages and four startup counters, GSR_N high."""
    s1 = s2 = int(with_sgsr)
    enabled, disabled = [0, 0], [0, 0]
    for half_edge in range(16):
        falling = (half_edge % 2 == 0) == falling_first
        group = int(falling)
        disabled[group] += 1
        enabled[group] = 0 if s2 else enabled[group]+1
        if not falling:
            s1, s2 = 0, s1
        if s2:
            enabled = [0, 0]
    return tuple(b-a for a, b in zip(enabled, disabled))


def selftest():
    # Independent, literal timing fixture: sample raw active at t=20, transport
    # it to s2 at t=24 and clear at t=28. Initial 11 clears s2 at t=8.
    # No asynchronous preset: raw release t=21 permits inverted/other captures
    # before t=24; same-clock capture at t=24 is asynchronously reset again.
    phases = [0, 0, 0]
    words = []
    for t in range(48):
        edges = [t > 0 and t % 4 == 0, t % 4 == 2, t % 3 == 1]
        phases = [p ^ int(e) for p, e in zip(phases, edges)]
        word = (1 << 15) | (int(not 17 <= t < 21) << 14)
        word |= sum(p << (11+g) for g, p in enumerate(phases))
        word |= sum(r << (5+2*g+r) for g in range(3) for r in range(2))
        words.append(word)
    w = Window('R', 0, 0, words)
    for g in range(3):
        for r in range(2):
            expected, q = [], r
            for t in range(48):
                old_s2 = t <= 8 or 24 < t <= 28
                new_s2 = t < 8 or 24 <= t < 28
                raw = 17 <= t < 21
                phase = w.signal(11+g)
                if raw or old_s2:
                    q = r
                elif t and phase[t] != phase[t-1]:
                    q = 1-phase[t]
                if raw or new_s2:
                    q = r
                expected.append(q)
            assert raw_or_sync_predict(w, g, r) == expected, (g, r)
            assert expected[24] == r  # no settled same-bin excursion survives
    assert {boot_offsets(True, phase) for phase in (False, True)} == {(2, 1), (2, 2)}
    assert {boot_offsets(False, phase) for phase in (False, True)} == {(0, 0)}
    assert nearest_transition([(8, 's2-fall')], 10) == ('s2-fall', 2, 8)
    try:
        nearest_transition([(8, 's2-fall'), (12, 'GSR_N-rise')], 11)
    except AssertionError:
        pass
    else:
        raise AssertionError('accepted residual 3 ticks after preceding transition')
    print('PASS: raw-OR-sync2 against literal stage-output timing, all groups/REGSETs; '
          'startup counters for both wake phases and no-SGSR control')


def collect(path):
    windows, indices, sources = [], {}, defaultdict(list)
    rejected = Counter()
    load, records = '?', 0
    metadata = []
    for line_no, line in enumerate(path.read_text().splitlines(), 1):
        if line.startswith('#'):
            metadata.append(line)
            match = re.match(r'# load=(\d+) ', line)
            if match:
                load = match[1]
        if not line.startswith('Y1'):
            continue
        try:
            w = parse(line)
        except ValueError as exc:
            rejected[str(exc)] += 1
            continue
        records += 1
        key = (w.variant, tuple(w.words))
        if key not in indices:
            indices[key] = len(windows)
            windows.append(w)
        sources[indices[key]].append(f'L{load}/seq{w.sequence}/line{line_no}')
    return windows, sources, metadata, records, rejected


def runs(ticks):
    result = []
    for tick in ticks:
        if result and tick == result[-1][-1]+1:
            result[-1].append(tick)
        else:
            result.append([tick])
    return result


def report(path, candidate=False):
    windows, sources, metadata, records, rejected = collect(path)
    print(f'\n=== {path.name} ===')
    print(f'valid_frames={records} unique_windows={len(windows)} duplicates={records-len(windows)} rejected={dict(rejected)}')
    print('\n'.join(metadata))
    bins, run_counts, phase_counts, width_counts = Counter(), Counter(), Counter(), Counter()
    affected, class_windows = set(), defaultdict(set)
    class_group_bins, class_group_windows = Counter(), defaultdict(set)
    class_channel_bins = Counter()
    transition_bins = Counter()
    details = []
    examined = sum(max(0, 512-stable_start(w, 4)) for w in windows)
    assertion_latencies = defaultdict(Counter)
    regular_captures = defaultdict(Counter)
    for wi, w in enumerate(windows):
        n = w.signal(14)
        transitions = reset_transitions(w) if candidate else []
        raw_edges = [t for t in changes(n) if t >= stable_start(w, 4)]
        assert len(raw_edges) == 2 and [n[t] for t in raw_edges] == [0, 1], (wi, raw_edges)
        assertion, release = raw_edges
        width = release-assertion
        ref = changes(w.signal(11))
        phase = release-max(t for t in ref if t <= release)
        active = reset_trace(n, w.signal(11), MODEL)
        nominal_release = next(t for t in range(release, 512) if not active[t])
        for g, group in enumerate(GROUPS):
            q0, q1 = w.signal(5+2*g), w.signal(6+2*g)
            held = [a == 0 and b == 1 for a, b in zip(q0, q1)]
            onset = next(t for t in range(assertion, release) if held[t])
            assertion_latencies[group][onset-assertion] += 1
            # Ignore the early excursions when measuring the regular release:
            # first pair equality at/after the nominal two-stage release.
            resume = next(t for t in range(nominal_release, 512) if not held[t])
            regular_captures[group, phase][resume-release] += 1
            local = changes(w.signal(11+g))
            for r, regset in enumerate(('RESET', 'SET')):
                q = w.signal(5+2*g+r)
                pred = raw_or_sync_predict(w, g, r) if candidate else predict(w, g, r, MODEL)
                bad = [t for t in range(stable_start(w, 4), 512) if q[t] != pred[t]]
                for run in runs(bad):
                    t, end = run[0], run[-1]
                    if all(q[v] != r and pred[v] == r and v < nominal_release for v in run):
                        kind = 'early-excursion'
                    elif all(q[v] == r and pred[v] != r and v >= nominal_release for v in run):
                        kind = 'held-through-capture'
                    elif candidate and all(q[v] == r and pred[v] != r and v < nominal_release for v in run):
                        kind = 'unobserved-early-capture'
                    else:
                        kind = 'elsewhere'
                    hazard = []
                    if candidate:
                        for v in run:
                            transition, distance, origin = nearest_transition(transitions, v)
                            transition_bins[kind, transition, distance] += 1
                            hazard.append(f'{v}:{transition}@{origin}+{distance}')
                    bins[group, regset] += len(run)
                    run_counts[kind] += 1
                    phase_counts[kind, phase] += len(run)
                    width_counts[kind, width] += len(run)
                    class_windows[kind].add(wi)
                    class_group_bins[kind, group] += len(run)
                    class_channel_bins[kind, group, regset] += len(run)
                    class_group_windows[kind, group].add(wi)
                    affected.add(wi)
                    clk_slot = sum(release <= v <= t for v in ref)
                    local_slot = sum(release <= v <= t for v in local)
                    suffix = ''
                    if kind == 'early-excursion':
                        expected = next(v for v in range(nominal_release, 512) if pred[v] != r)
                        suffix = (f' regular_pred_Q_departure={expected} early_by={expected-t}ticks/'
                                  f'{sum(t < v <= expected for v in local)}local_edges/'
                                  f'{sum(t < v <= expected for v in ref)}CLK_edges')
                    elif kind == 'held-through-capture':
                        depart = next(v for v in range(end+1, 512) if q[v] != r)
                        suffix = (f' next_Q_departure={depart} departure_shift=+{depart-t}ticks/'
                                  f'{sum(t < v <= depart for v in local)}local_edges/'
                                  f'{sum(t < v <= depart for v in ref)}CLK_edges'
                                  ' (D toggles: not a direct measurement of reset-release delay)')
                    details.append(
                        f'w={wi:03d} sources={",".join(sources[wi])} seed={w.seed:08X} '
                        f'assert={assertion} release={release} phase={phase} width={width}ticks/{width//4}cycles '
                        f'{group}/{regset} class={kind} ticks={",".join(map(str, run))} '
                        f'dt_release={t-release}..{end-release} CLK_slot={clk_slot} local_slot={local_slot} '
                        f'dt_nominal_reset_release={t-nominal_release}..{end-nominal_release} '
                        f'observed={"".join(str(q[v]) for v in run)} predicted={"".join(str(pred[v]) for v in run)}'
                        + suffix + (' nearest_reset_transition=' + ','.join(hazard) if candidate else ''))
    total = sum(bins.values())
    print('model=raw-active-OR-sync2(initial=11,ref=same,new-input,old-s2-at-capture)'
          if candidate else f'model={MODEL.label()}')
    print(f'examined={examined} bins/channel, {6*examined} channel-bins; exact_residuals={total}; affected_windows={len(affected)}')
    for g in GROUPS:
        print(f'{g}: RESET={bins[g,"RESET"]} SET={bins[g,"SET"]}; assertion_pair_latency_ticks={dict(assertion_latencies[g])}')
    for kind in sorted(run_counts):
        print(f'{kind}: runs={run_counts[kind]} windows={len(class_windows[kind])} '
              f'bins={sum(v for (k,p),v in phase_counts.items() if k==kind)}')
        print('  bins_by_release_phase=' + str({p:v for (k,p),v in sorted(phase_counts.items()) if k==kind}))
        print('  bins_by_width_ticks=' + str({p:v for (k,p),v in sorted(width_counts.items()) if k==kind}))
        print('  window_ids=' + ','.join(map(str, sorted(class_windows[kind]))))
        print('  by_group=' + str({group:dict(bins=class_group_bins[kind,group],
                                             windows=len(class_group_windows[kind,group]))
                                  for group in GROUPS if (kind,group) in class_group_bins}))
        print('  by_channel=' + str({f'{group}/{regset}': class_channel_bins[kind,group,regset]
                                    for group in GROUPS for regset in ('RESET', 'SET')}))
    for (group, phase), counts in sorted(regular_captures.items()):
        print(f'regular_pair_capture {group} phase={phase}: latency_ticks={dict(sorted(counts.items()))}')
    print('Interpretation:')
    print('  Early excursions are Q departures before the nominal release, followed by return to REGSET;')
    print('  they are not a whole-trace shift and not evidence for assertion being synchronous.')
    print('  A phase-0 release shares a bin with CLK rising; other phases do not. Compare phase counts above.')
    print('  Held-through-capture is a missed predicted non-REGSET capture; a later departure is censored by alternating D.')
    print('  Bin timing alone cannot identify metastability, analog reset distribution delay, or internal circuit topology.')
    if candidate:
        assert sum(transition_bins.values()) == total
        for (kind, transition, distance), count in sorted(transition_bins.items()):
            print(f'  hazard_class={kind} nearest_transition={transition} distance_ticks={distance} bins={count}')
        print(f'  execution_gate=PASS: within_2_ticks={total} outside_2_ticks=0')
        print('  Accepted functional rule: raw-active OR plain sync2(initial 11).')
        print('  Reset network propagation delay accounts for boundary captures within 2 ticks (20 ns).')
        print('  The emulator assigns zero time to propagation delays; these are timing hazards, not extra data rules.')
    print('Every exact residual, grouped into contiguous runs (all ticks enumerated):')
    print('\n'.join(details) if details else '(none)')
    return total


def report_boot(path):
    # Boot frames contain a frozen count offset, not edge traces. Falling-edge
    # wake phase is unobserved: the candidate predicts the set {1,2} for B.
    from check_boot_capture import decode
    loads, rejected = defaultdict(list), Counter()
    load = None
    for line in path.read_text().splitlines():
        match = re.match(r'# load=(\d+) ', line)
        if match:
            load = int(match[1])
        if not line.startswith('Y1'):
            continue
        try:
            values = decode(line)
        except ValueError as exc:
            rejected[str(exc)] += 1
            continue
        loads[load].append((line[2], tuple(values)))
    print(f'\n=== {path.name} ===')
    print(f'valid_frames={sum(map(len, loads.values()))} loads={len(loads)} rejected={dict(rejected)}')
    residuals = Counter()
    for load, frames in sorted(loads.items()):
        assert len(set(frames)) == 1, (load, 'snapshot changed within load')
        variant, values = frames[0]
        rise, fall = values[2], values[5]
        predictions = {boot_offsets(variant == 'B', phase) for phase in (False, True)}
        expected_rise = tuple(sorted({p[0] for p in predictions}))
        expected_fall = tuple(sorted({p[1] for p in predictions}))
        residuals['startup-rising'] += rise not in expected_rise
        residuals['startup-falling'] += fall not in expected_fall
        print(f'load={load} frames={len(frames)} rising_B-A={rise} falling_B-A={fall} '
              f'predicted_rising={expected_rise} predicted_falling={expected_fall}')
    print(f'exact_offset_residuals_per_class={dict(residuals)}; '
          'falling wake phase is not measured, so this is set membership, not a phase fit')
    assert not rejected and not any(residuals.values()), ('boot residual outside model', path)
    print('execution_gate=PASS: offset_residuals=0; frozen snapshots contain no transition timestamps')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('logs', nargs='*', type=Path)
    p.add_argument('--candidate', action='store_true', help='test raw GSR OR plain sync2, initialized 11')
    p.add_argument('--selftest', action='store_true')
    args = p.parse_args()
    if args.selftest:
        selftest()
    if not args.logs:
        if args.selftest:
            return
        p.error('provide UART logs or --selftest')
    print('HY01 joint-model residual audit. No board or serial access.')
    print('Clock slots include a marker in the input-release bin as slot 1; sub-bin order is unknown.')
    for path in args.logs:
        if any(line.startswith(('Y1B', 'Y1C')) for line in path.read_text().splitlines()):
            report_boot(path)
        else:
            report(path, args.candidate)


if __name__ == '__main__':
    main()
