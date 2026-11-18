"""Decode HI12 count/phase records, allowing the /3.5 phase at fabric start.
Usage: python3 -B hi12_model.py observations.log [--samples]
An exact startup deficit is inferred at /2; /3.5 has phase/rounding ambiguity.
"""
import argparse
from fractions import Fraction
import itertools
from pathlib import Path
import re

RECORD = re.compile(r'HC[0-4] [0-9A-F]{176}')


def predict(inputs, delay, seven, phase=6):
    """Counts after I input periods, omitting D, from a modulo-seven phase.

    Phase is the divider state just before the first counted half edge;
    CDIVX is high in states 0..3. Phase 6 is the emulator's initial low
    state, which rises on its first edge. A continuously running pin clock
    need not leave that phase when the fabric counters start.
    """
    active = max(0, inputs-delay)
    if not seven:
        return (active+1)//2, active//2
    halves = 2*active
    rising = (phase+halves)//7 - phase//7
    falling = (phase+halves-4)//7 - (phase-4)//7
    return rising, falling


def startup_candidates(snapshots, seven):
    return {delay: phases for delay in range(9)
            if (phases := [phase for phase in (range(7) if seven else (6,))
                           if all(predict(i, delay, seven, phase) == (p, f)
                                  for i, p, f in snapshots)])}


def decode(line):
    if not RECORD.fullmatch(line):
        raise ValueError('not a complete HI12 line')
    variant = int(line[2])
    payload = line[4:]
    counts = [int(payload[n:n+8], 16) for n in range(0, 48, 8)]
    bits = f'{int(payload[48:], 16):0512b}'
    samples = [(int(bits[n]), int(bits[n+1])) for n in range(0, 512, 2)]
    return variant, [tuple(counts[:3]), tuple(counts[3:])], samples


def inspect(line):
    variant, snapshots, samples = decode(line)
    seven = variant in (2, 4)
    candidates = startup_candidates(snapshots, seven)
    clki = [n for n in range(1, 256) if samples[n][0] != samples[n-1][0]]
    qedges = [n for n in range(1, 256) if samples[n][1] != samples[n-1][1]]
    if len(clki) <= 100 or len(qedges) <= 20:
        raise ValueError('missing observer phase activity')
    offsets = sorted({n-min(clki, key=lambda c: abs(n-c)) for n in qedges})
    rising = [n for n in qedges if samples[n][1]]
    intervals = sorted({b-a for a, b in zip(rising, rising[1:])})
    runs = [(level, len(list(group)))
            for level, group in itertools.groupby(q for _, q in samples)]
    widths = {level: sorted({n for q, n in runs[1:-1] if q == level})
              for level in (0, 1)}
    if not candidates:
        raise ValueError('no constant delay and initial phase fit both snapshots')
    return dict(variant=variant, seven=seven, snapshots=snapshots,
                candidates=candidates, offsets=offsets, intervals=intervals,
                widths=widths, samples=samples)


def analyse(line, show_samples=False):
    result = inspect(line)
    v, snapshots = result['variant'], result['snapshots']
    print(f'HC{v}: snapshots (CLKI rises, CDIVX rises, CDIVX falls)={snapshots}')
    if result['seven']:
        print(f'  /3.5 delay -> initial phase candidates: {result["candidates"]}')
        print('  zero-deficit fit: '+str(0 in result['candidates'])+
              '; phase/rounding prevents a unique first-edge startup inference')
        print(f'  ideal cycles 2I/7: {[str(Fraction(2*i, 7)) for i, _, _ in snapshots]}')
        print(f'  rounding residuals (7P-2I, 7F-2I): {[(7*p-2*i, 7*f-2*i) for i,p,f in snapshots]}')
    else:
        print(f'  startup delay candidates in CLKI periods: {list(result["candidates"])}')
        print(f'  I-(P+F)={[i-p-f for i,p,f in snapshots]}; I-2P={[i-2*p for i,p,f in snapshots]}')
    a, b = snapshots
    print(f'  snapshot increments: I={b[0]-a[0]} P={b[1]-a[1]} F={b[2]-a[2]}')
    print(f'  CDIVX transitions relative to nearest CLKI transition, 10 ns samples: {result["offsets"]}')
    print(f'  CDIVX rising intervals, 10 ns samples: {result["intervals"]}; low/high widths: {result["widths"]}')
    if show_samples:
        print('  phase samples (CLKI,CDIVX): '+' '.join(f'{a}{b}' for a,b in result['samples']))
    return result


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('log', type=Path)
    p.add_argument('--samples', action='store_true')
    args = p.parse_args()
    records = sorted({s for s in args.log.read_text().splitlines() if RECORD.fullmatch(s)})
    if not records:
        p.error('no HI12 records')
    for record in records:
        analyse(record, args.samples)
