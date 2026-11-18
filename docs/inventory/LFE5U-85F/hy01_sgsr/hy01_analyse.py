"""Analyse HY01 UART captures; standard library only.

Fit complete Q traces to async, sync(ka,kr), async-assert/sync-release(kr), and
ignored hypotheses. Try CLK (the SGSR pin), inverted CLK, and other as the
synchronizer reference, k=1..--max-k, and both same-bin release orderings.
Use --selftest for synthetic, independently generated timing/protocol checks.
"""
import argparse
from collections import Counter
import csv
from dataclasses import dataclass
from functools import reduce
import operator
from pathlib import Path
import re
import sys
sys.dont_write_bytecode = True
from hy01_gen import SAMPLES, PAYLOAD_START, PAYLOAD_END, LINE_BYTES

GROUPS = ('same', 'inverted', 'other')


@dataclass
class Window:
    variant: str
    sequence: int
    seed: int
    words: list

    def signal(self, bit):
        return [(w >> bit) & 1 for w in self.words]


def parse(line):
    line = line.rstrip('\r\n')
    if len(line) != LINE_BYTES - 2 or not line.startswith(('Y1R', 'Y1S')):
        raise ValueError('bad prefix or length')
    if not re.fullmatch(r'[0-9A-F]+', line[3:]):
        raise ValueError('non-hexadecimal payload')
    try:
        seq = int(line[3:11], 16)
        seed = int(line[11:19], 16)
        words = [int(line[i:i+4], 16) for i in range(PAYLOAD_START, PAYLOAD_END, 4)]
        checksum = int(line[PAYLOAD_END:], 16)
    except ValueError as exc:
        raise ValueError('non-hexadecimal payload') from exc
    if reduce(operator.xor, words, 0) != checksum:
        raise ValueError('checksum mismatch')
    if any(w & 31 for w in words):
        raise ValueError('reserved bits nonzero')
    return Window(line[2], seq, seed, words)


def read(paths):
    windows, seen, rejected = [], set(), Counter()
    repeats = 0
    for path in paths:
        for line in Path(path).read_text(errors='replace').splitlines():
            if not line.startswith('Y1'):
                continue  # Runner's # load metadata, or unrelated UART noise.
            try:
                w = parse(line)
            except ValueError as exc:
                rejected[str(exc)] += 1
                continue
            key = (w.variant, tuple(w.words))
            if key in seen:
                repeats += 1
            else:
                seen.add(key)
                windows.append(w)
    print(f'windows={len(windows)} duplicate_payloads={repeats} rejected={dict(rejected)}')
    return windows


def changes(values):
    return [i for i in range(1, len(values)) if values[i] != values[i-1]]


def stable_start(w, max_k):
    """Exclude startup/unlocked bins and enough subsequent edges to flush history."""
    lock = w.signal(15)
    last_bad = max((i for i, v in enumerate(lock) if not v), default=-1)
    return max(16, last_bad + 4 * max_k + 8)


@dataclass(frozen=True)
class Model:
    kind: str
    k: int = 0
    ref: int = 0
    release_old: bool = True
    sample_old: bool = False
    assert_k: int = 0

    def label(self):
        if self.kind == 'ignored':
            return self.kind
        order = 'old-reset' if self.release_old else 'new-reset'
        if self.kind == 'async':
            return f'async({order})'
        sample = 'old-input' if self.sample_old else 'new-input'
        latency = f'ka={self.assert_k},kr={self.k}' if self.kind == 'sync' else f'kr={self.k}'
        return f'{self.kind}({latency},ref={GROUPS[self.ref]},{order},{sample})'


def models(max_k):
    candidates = [Model('ignored')] + [Model('async', release_old=old) for old in (True, False)]
    for kind in ('sync', 'async-assert/sync-release'):
        for kr in range(1, max_k+1):
            for ka in (range(1, max_k+1) if kind == 'sync' else (0,)):
                for ref in range(3):
                    for old in (True, False):
                        for sample_old in (False, True):
                            candidates.append(Model(kind, kr, ref, old, sample_old, ka))
    return candidates


def reset_trace(n, reference, model):
    """Global active reset AFTER each bin; equal ka/kr is a shift pipeline.

    With k=1, the first reference edge sampling a new input changes reset.
    A short pulse sampled once propagates through sync(k), even if the input
    has since released. Mixed mode asynchronously fills all stages with 1;
    release shifts in zeros. Ignored is an explicit negative control.
    """
    if model.kind == 'ignored':
        return [0] * len(n)
    if model.kind == 'async':
        return [1-v for v in n]
    ka = model.assert_k or model.k
    pipe = [1-n[0]] * max(ka, model.k)
    active = pipe[-1]
    out = []
    for t in range(len(n)):
        if model.kind == 'async-assert/sync-release' and not n[t]:
            pipe = [1] * len(pipe)
            active = 1
        elif t and reference[t] != reference[t-1]:
            sampled = n[t-1] if model.sample_old else n[t]
            old_a, old_r = pipe[ka-1], pipe[model.k-1]
            pipe = [1-sampled] + pipe[:-1]
            if model.kind == 'sync':
                # Separate transition-transport delays. With ka==kr this is
                # exactly a k-stage pipeline, including propagated short pulses.
                if old_r and not pipe[model.k-1]:
                    active = 0
                if not old_a and pipe[ka-1]:
                    active = 1
            else:
                active = pipe[model.k-1]
        out.append(active)
    return out


def predict(w, group, regset, model):
    n = w.signal(14)
    phase = w.signal(11+group)
    active = reset_trace(n, w.signal(11+model.ref), model)
    result = [w.signal(5+2*group+regset)[0]]
    for t in range(1, len(n)):
        value = result[-1]
        if active[t]:
            value = regset
        elif phase[t] != phase[t-1]:
            # Probe captures the old marker; the observer sees its new value.
            if not (active[t-1] and model.release_old):
                value = 1-phase[t]
        result.append(value)
    return result


def score(actual, predicted, start, tolerance):
    """Exact errors and symmetric errors beyond the specified bin uncertainty.

    Symmetry penalizes both an extra observed pulse and a missing predicted
    pulse. This is a bin tolerance, not a metastability probability model.
    """
    exact = sum(a != p for a, p in zip(actual[start:], predicted[start:]))
    robust = 0
    for t in range(start+tolerance, len(actual)-tolerance):
        lo, hi = t-tolerance, t+tolerance+1
        robust += actual[t] not in predicted[lo:hi]
        robust += predicted[t] not in actual[lo:hi]
    return exact, robust


def fit(windows, candidates, tolerance, max_k):
    if not any(stable_start(w, max_k) < SAMPLES for w in windows):
        print('No settled samples: no timing model can be assessed.')
        return {}
    fitted = {}
    for g, group in enumerate(GROUPS):
        for r, regset in enumerate(('RESET', 'SET')):
            totals = []
            count = sum(max(0, SAMPLES-stable_start(w, max_k)) for w in windows)
            for m in candidates:
                exact = robust = 0
                for w in windows:
                    start = stable_start(w, max_k)
                    if start >= SAMPLES:
                        continue
                    a, b = score(w.signal(5+2*g+r), predict(w, g, r, m), start, tolerance)
                    exact += a
                    robust += b
                totals.append((m, exact, robust))
            fitted[g, r] = totals
            if not count:
                print(f'{group}/{regset}: no settled samples; no model fit')
                continue
            best_exact = min(e for _, e, _ in totals)
            exact_models = [m.label() for m, e, _ in totals if e == best_exact]
            best_robust = min(e for _, _, e in totals)
            robust_models = [m.label() for m, _, e in totals if e == best_robust]
            print(f'{group}/{regset}: exact={best_exact}/{count}: {", ".join(exact_models)}')
            print(f'  tolerance=+/-{tolerance} tick symmetric_errors={best_robust}: '
                  f'{", ".join(robust_models)}')
            for kind in ('async', 'sync', 'async-assert/sync-release', 'ignored'):
                rows = [(e, b) for m, e, b in totals if m.kind == kind]
                print(f'  {kind}: min_exact={min(e for e, _ in rows)} '
                      f'min_tolerant={min(b for _, b in rows)}')
    # A common model must explain all six FFs, not just one REGSET or domain.
    joint = [(m, sum(fitted[g, r][i][1] for g in range(3) for r in range(2)),
              sum(fitted[g, r][i][2] for g in range(3) for r in range(2)))
             for i, m in enumerate(candidates)]
    best = min(e for _, e, _ in joint)
    print(f'joint exact={best}: ' + ', '.join(m.label() for m, e, _ in joint if e == best))
    return fitted


def edge_range(reference, edge, reaction, tolerance):
    """Number of reference edges after input through reaction, inclusive at end.
    Same-bin input/reference ordering is unknown, so also report an interval.
    """
    events = changes(reference)
    nominal = sum(edge < t <= reaction for t in events)
    low = sum(edge+tolerance < t <= reaction-tolerance for t in events)
    high = sum(edge-tolerance <= t <= reaction+tolerance for t in events)
    return nominal, low, high


def event_rows(w, number, tolerance, max_k):
    n = w.signal(14)
    edges = changes(n)
    start = stable_start(w, max_k)
    rows = []
    pairs = {}
    # Match each assertion to a stable pair-hold epoch. Its end belongs to
    # the following release, including when a pipelined short pulse reaches
    # the FFs only AFTER the raw input has already released.
    for g in range(3):
        a, b = w.signal(5+2*g), w.signal(6+2*g)
        held = [x == 0 and y == 1 for x, y in zip(a, b)]
        state = held[0]
        transitions = []
        for t in range(1, SAMPLES-1):
            if held[t] == held[t+1] and held[t] != state:
                state = held[t]
                transitions.append((t, state))
        for ei, edge in enumerate(edges):
            if n[edge] or edge < start:
                continue
            next_assert = next((t for t in edges[ei+1:] if not n[t]), SAMPLES)
            limit = min(next_assert-tolerance, edge+4*(max_k+3)+tolerance, SAMPLES)
            onset = next((t for t, active in transitions
                          if active and edge-tolerance <= t < limit), None)
            pairs[g, edge] = onset
            raw_release = next((t for t in edges[ei+1:] if n[t]), None)
            if onset is not None and raw_release is not None:
                resume = next((t for t, active in transitions
                               if not active and onset < t < next_assert
                               and t >= raw_release-tolerance), None)
                pairs[g, raw_release] = resume
    for ei, edge in enumerate(edges):
        kind = 'assert' if not n[edge] else 'release'
        for g, group in enumerate(GROUPS):
            a, b = w.signal(5+2*g), w.signal(6+2*g)
            target = kind == 'assert'
            reaction = pairs.get((g, edge))
            for r, regset in enumerate(('RESET', 'SET')):
                q = (a, b)[r]
                row = dict(window=number, variant=w.variant, sequence=w.sequence,
                           seed=f'{w.seed:08X}', event=kind, input_tick=edge,
                           group=group, regset=regset, pair_tick='', q_tick='',
                           latency_ticks='', clk_edges='', clk_edges_min='', clk_edges_max='',
                           q_latency_ticks='', q_clk_edges='', q_clk_edges_min='', q_clk_edges_max='',
                           q_status='unresolved/missed/overlapped',
                           status='no associated stable pair reaction')
                if edge < start:
                    row['status'] = 'startup/unlocked; event excluded from timing fit'
                elif reaction is not None:
                    nominal, low, high = edge_range(w.signal(11), edge, reaction, tolerance)
                    row.update(pair_tick=reaction, latency_ticks=reaction-edge,
                               clk_edges=nominal, clk_edges_min=low, clk_edges_max=high,
                               status='pair hold begins' if target else 'pair capture resumes')
                    if target:
                        if q[reaction-1] != r and q[reaction] == r:
                            row.update(q_tick=reaction, q_status='transition to REGSET')
                        else:
                            row['q_status'] = 'already REGSET; individual assertion censored'
                    else:
                        next_assert = next((t for t in edges[ei+1:] if not n[t]), SAMPLES)
                        qt = next((t for t in range(reaction, next_assert) if q[t] != r), None)
                        if qt is not None:
                            row.update(q_tick=qt, q_status='first departure from REGSET')
                    if row['q_tick'] != '':
                        qt = row['q_tick']
                        nominal, low, high = edge_range(w.signal(11), edge, qt, tolerance)
                        row.update(q_latency_ticks=qt-edge, q_clk_edges=nominal,
                                   q_clk_edges_min=low, q_clk_edges_max=high)
                rows.append(row)
    return rows


def coverage(windows, max_k):
    widths, phases = Counter(), Counter()
    for w in windows:
        n = w.signal(14)
        edges = changes(n)
        rises = changes(w.signal(11))
        for i, t in enumerate(edges):
            if t < stable_start(w, max_k):
                continue
            preceding = [e for e in rises if e <= t]
            phases[(('assert' if not n[t] else 'release'),
                    t-preceding[-1] if preceding else -1)] += 1
            if not n[t] and i+1 < len(edges) and n[edges[i+1]]:
                widths[edges[i+1]-t] += 1
    print(f'measured pulse widths (100 MHz bins): {dict(sorted(widths.items()))}')
    print(f'edge phase (bins since CLK rise marker): {dict(sorted(phases.items()))}')
    missing = [cycles for cycles in (1, 2, 3, 5, 20)
               if not any(abs(t-4*cycles) <= 1 for t in widths)]
    if missing:
        print(f'INCOMPLETE width coverage: missing nominal cycles {missing}')
    if len({p for (_, p) in phases}) < 3:
        print('INCOMPLETE phase coverage: collect more windows (rise/fall/between).')


def encode(w):
    checksum = reduce(operator.xor, w.words, 0)
    return (f'Y1{w.variant}{w.sequence:08X}{w.seed:08X}' +
            ''.join(f'{x:04X}' for x in w.words) + f'{checksum:04X}\r\n')


def synthetic(kind, k=2, offset=1, assert_k=None):
    """Independent event-driven fixture: queue sampled reset changes by edge ID.
    This deliberately does not call reset_trace() or predict().
    """
    phase = [0, 0, 0]
    q = [0, 1, 0, 1, 0, 1]
    n, active, sampled, edge_id = 1, False, False, 0
    queue = {}
    words = []
    pulses = [(48+offset, 4), (100+offset, 8), (160+offset, 12),
              (228+offset, 20), (320+offset, 80)]
    for t in range(SAMPLES):
        n = int(not any(a <= t < a+width for a, width in pulses))
        edges = [t % 4 == 0, t % 4 == 2, t % 7 in (1, 4)]
        previous_active = active
        if kind == 'async':
            active = not n
        elif kind == 'mixed' and not n:
            active, sampled = True, True
            queue.clear()
        if edges[0]:
            edge_id += 1
            value = not n
            if value != sampled:
                delay = (assert_k or k) if value else k
                queue[edge_id+delay-1] = value
                sampled = value
            if edge_id in queue and kind in ('sync', 'mixed'):
                active = queue.pop(edge_id)
        for g in range(3):
            if edges[g]:
                phase[g] ^= 1
            for r in range(2):
                if active:
                    q[2*g+r] = r
                elif edges[g] and not previous_active:
                    q[2*g+r] = 1-phase[g]
        word = (1 << 15) | (n << 14)
        word |= sum(v << (11+g) for g, v in enumerate(phase))
        word |= sum(v << (5+i) for i, v in enumerate(q))
        words.append(word)
    return Window('R', offset, 0x1234+offset, words)


def selftest():
    candidates = models(4)
    for kind, expected in [('async', 'async'), ('sync', 'sync'),
                           ('mixed', 'async-assert/sync-release'), ('ignored', 'ignored')]:
        ws = [synthetic(kind, offset=p) for p in (0, 1, 2, 3)]
        for w in ws:
            assert parse(encode(w)).words == w.words
        joint = []
        for m in candidates:
            error = sum(score(w.signal(5+2*g+r), predict(w, g, r, m), 32, 0)[0]
                        for w in ws for g in range(3) for r in range(2))
            joint.append((m, error))
        best = min(e for _, e in joint)
        winners = [m for m, e in joint if e == best]
        assert best == 0 and {m.kind for m in winners} == {expected}, (kind, best, winners)
        if kind in ('sync', 'mixed'):
            assert {(m.k, m.ref) for m in winners} == {(2, 0)}, winners
            if kind == 'sync':
                assert {m.assert_k for m in winners} == {2}, winners
        rows = event_rows(ws[0], 0, 1, 4)
        assert len(rows) == 60, len(rows)
        if kind != 'ignored':
            assert any('censored' in x['q_status'] for x in rows)
            assert any(x['event'] == 'release' and x['q_tick'] != '' for x in rows)
        if kind == 'sync':
            # One-cycle pulse at phase 1 releases before sync(2) asserts.
            delayed = event_rows(ws[1], 0, 0, 4)
            assert any(x['event'] == 'assert' and x['input_tick'] == 49 and x['pair_tick'] != '' and
                       x['pair_tick'] > 53 for x in delayed), delayed
    raw = encode(synthetic('sync'))
    asymmetric = [synthetic('sync', k=3, assert_k=1, offset=p) for p in (0, 1, 2, 3)]
    errors = [(m, sum(score(w.signal(5+2*g+r), predict(w, g, r, m), 32, 0)[0]
                      for w in asymmetric for g in range(3) for r in range(2)))
              for m in candidates]
    assert any(e == 0 for _, e in errors)
    assert all(m.kind == 'sync' and (m.assert_k, m.k, m.ref) == (1, 3, 0)
               for m, e in errors if e == 0)
    for bad in (raw[:-8]+'\r\n', raw[:19]+'Z'+raw[20:], raw[:-6]+'FFFF\r\n'):
        try:
            parse(bad)
        except ValueError:
            pass
        else:
            raise AssertionError('malformed record accepted')
    a = [0]*20 + [1]*20
    b = [0]*21 + [1]*19
    assert score(a, b, 0, 0)[0] == 1
    assert score(a, b, 0, 1)[1] == 0
    assert score(a, [0]*40, 0, 1)[1] > 0
    print('PASS: independent async/sync(2)/sync(1,3)/mixed(2)/ignored, all groups/REGSETs, alternate clocks, '
          'phase offsets, pulse widths, event censoring, protocol rejection, bin tolerance')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('logs', nargs='*')
    p.add_argument('--selftest', action='store_true')
    p.add_argument('--max-k', type=int, default=4)
    p.add_argument('--tolerance', type=int, choices=(0, 1), default=1)
    p.add_argument('--events-csv', type=Path)
    args = p.parse_args()
    if args.max_k < 1 or args.max_k > 16:
        p.error('--max-k must be 1..16')
    if args.selftest:
        selftest()
    if not args.logs:
        if args.selftest:
            return
        p.error('provide UART log(s) or --selftest')
    windows = read(args.logs)
    if not windows:
        raise SystemExit('no complete checksum-valid HY01 records')
    print('10 ns nominal tick; common observation latency cancels in differences; '
          'CDC/skew adds bin uncertainty. k counts the first sampling edge as 1.')
    for w in windows:
        if w.variant == 'S':
            print(f'startup seq={w.sequence}: sample0={w.words[0]:04X}; '
                  f'stable-fit starts at {stable_start(w, args.max_k)}; '
                  'no physical time origin before PLL produces fast edges')
    coverage(windows, args.max_k)
    fit(windows, models(args.max_k), args.tolerance, args.max_k)
    rows = [row for i, w in enumerate(windows)
            for row in event_rows(w, i, args.tolerance, args.max_k)]
    if args.events_csv and rows:
        with args.events_csv.open('w', newline='') as out:
            writer = csv.DictWriter(out, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
        print(f'event details: {args.events_csv} ({len(rows)} rows)')
    else:
        for row in rows:
            print('event ' + ' '.join(f'{k}={v}' for k, v in row.items()))
    print('Fits are hypotheses: retain tied models, inspect nonzero residuals and missing '
          'width/phase coverage; pair release is first capture, not the invisible reset release.')


if __name__ == '__main__':
    main()
