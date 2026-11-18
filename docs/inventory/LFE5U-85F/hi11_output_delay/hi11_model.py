"""Validate HI11 and HI11s frames, reporting rather than assuming startup.
Usage: python3 hi11_model.py RECORD.txt [RECORD.txt ...]

F/G/B: original 2048 nibble samples. S0/S1: two CFLAG lanes (DEL_VALUE 10/100).
S2..S5: three CFLAG lanes (DEL_VALUE 1/63/64/127), adding bottom-bank FIRE1.
Startup records have 2048 byte samples. Startup candidates are inferred
from the pre-load flags under HI08's stepping model, separately for each path.
Only the samples from the first load onward are checked against hi08_model.py.
The adapter replaces the pre-load prefix with copies of the first load sample
so that HI08's fixed-length checker starts in a known loaded state; all real
post-load samples retain their original indexes and are checked, including
that first load. Startup is never silently forced to zero or DEL_VALUE.
"""
from functools import lru_cache
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from hi11_gen import controls
from hi11s_gen import controls as startup_controls, VALUES, THREE_LANE_IDS

SCHEMA = re.compile(r'H11([FGB])([0-9A-F]{8})([0-9A-F]{2048})([0-9A-F])')
STARTUP_SCHEMA = re.compile(r'H11S([0-5])([0-9A-F]{8})([0-9A-F]{4096})([0-9A-F]{2})')
HI08 = Path(__file__).resolve().parent.parent / 'hi08_delay' / 'hi08_model.py'


def first_load(rows):
    return next(i for i, row in enumerate(rows) if not row[0])


def startup_candidates(rows, lane):
    """Initial 0..127 counts consistent with every pre-load flag, not a guess."""
    prefix = rows[:first_load(rows)]
    candidates = []
    for initial in range(128):
        count, prev_move = initial, 0
        for loadn, move, direction, *flags in prefix:
            if prev_move and not move:
                count = min(127, max(0, count + (-1 if direction else 1)))
            prev_move = move
            if flags[lane] != int(count == (0 if direction else 127)):
                break
        else:
            candidates.append(initial)
    return candidates


def startup_report(rows, lane, label):
    prefix = rows[:first_load(rows)]
    falls = [i for i in range(1, len(prefix)) if prefix[i - 1][1] and not prefix[i][1]]
    endpoint = next((i for i, row in enumerate(prefix) if row[2] and row[3 + lane]), None)
    candidates = startup_candidates(rows, lane)
    if not candidates:
        bound = 'no initial 0..127 count fits HI08 stepping during startup'
    elif len(candidates) == 1:
        bound = f'initial counter={candidates[0]} (HI08-compatible)'
    else:
        assert candidates == list(range(candidates[0], candidates[-1] + 1))
        bound = f'initial counter={candidates[0]}..{candidates[-1]} (HI08-compatible)'
    reached = ('not reached before load' if endpoint is None else
               f'sample {endpoint}, after {sum(i <= endpoint for i in falls)} down steps')
    return (f'startup {label}: CFLAG[0]={prefix[0][3 + lane]}; '
            f'{len(falls)} down steps before load; first down endpoint {reached}; {bound}')


def check_loaded(rows, values):
    start = first_load(rows)
    assert rows[start][1] == 0, 'the first load must have MOVE low'
    # Preserve the original 2048-sample format and index alignment. Synthetic
    # prefix copies do not constrain startup and cannot affect state after load.
    aligned = [rows[start]] * start + rows[start:]
    assert len(values) == len(rows[0]) - 3
    # HI08 checks two flags. Project each pair, duplicating the last lane
    # when needed, so a third lane receives the same post-load validation.
    for first in range(0, len(values), 2):
        indices = [first, min(first + 1, len(values) - 1)]
        bits = ''.join(''.join(str(bit) for bit in
                       (*row[:3], *(row[3 + lane] for lane in indices))) for row in aligned)
        with tempfile.TemporaryDirectory(prefix='hi11-model-') as temp:
            converted = Path(temp) / 'hi08-compatible.txt'
            converted.write_text(f'D={int(bits, 2):02560X}\n')
            result = subprocess.run([sys.executable, str(HI08), str(converted),
                                     str(values[indices[1]]), str(values[indices[0]])],
                                    capture_output=True, text=True, check=True)
        assert result.stdout.rstrip().endswith('model fits'), result.stdout
        assert '0 samples skipped' in result.stdout, result.stdout
    lanes = f' ({len(values)} lanes)' if len(values) > 2 else ''
    return f'HI08 check from first load sample {start}: {len(rows) - start} real samples/path, 0 differ, 0 skipped{lanes}'


@lru_cache(maxsize=64)
def analyse(ident, payload):
    startup = ident.startswith('S')
    samples = ([int(payload[i:i + 2], 16) for i in range(0, len(payload), 2)] if startup
               else [int(c, 16) for c in payload])
    checksum = 0
    for sample in samples:
        checksum ^= sample
    if startup:
        three_lanes = ident[1] in THREE_LANE_IDS
        if three_lanes:
            assert all(s & 0x80 == 0 for s in samples), 'bottom input must read low (FIRE1 released, PULLMODE=DOWN)'
        else:
            assert all(s & 0xc0 == 0 for s in samples), 'reserved sample bits'
        assert all(s & 0x20 for s in samples), 'side input must read high (PULLMODE=UP)'
        rows = [tuple((s >> b) & 1 for b in (4, 3, 2, 1, 0)) for s in samples]
        if three_lanes:
            rows = [row + ((sample >> 6) & 1,) for row, sample in zip(rows, samples)]
        expected = startup_controls()
    else:
        rows = [tuple((s >> b) & 1 for b in (3, 2, 1)) + (s & 1, s & 1) for s in samples]
        expected = controls()
    assert [row[:3] for row in rows] == expected, 'controls'
    messages = []
    if ident == 'F' or startup:
        messages.append(startup_report(rows, 1 if startup else 0, 'output'))
        if startup:
            messages.append(startup_report(rows, 0, 'side-bank input'))
            if three_lanes:
                messages.append(startup_report(rows, 2, 'bottom-bank input (FIRE1/R1)'))
        value = VALUES[ident[1]] if startup else 10
        messages.append(check_loaded(rows, (value,) * (len(rows[0]) - 3)))
    else:
        assert all(s & 1 == 0 for s in samples), 'DELAYG reserved CFLAG bit'
    return checksum, tuple(messages)


def check(path):
    frames = []
    summaries = set()
    # Captures may begin/end inside a frame; reject malformed complete frames.
    chunks = Path(path).read_bytes().split(b'\r\n')[:-1]
    for i, raw in enumerate(chunks):
        if i == 0 and not raw.startswith(b'H11'):
            continue
        line = raw.decode('ascii')
        match = STARTUP_SCHEMA.fullmatch(line)
        if match is not None:
            ident, seq, payload, xor = match.groups()
            ident = 'S' + ident
        else:
            match = SCHEMA.fullmatch(line)
            if match is None:
                raise ValueError(f'{path}: malformed complete frame {i}')
            ident, seq, payload, xor = match.groups()
        if frames:
            assert ident == frames[-1][0], 'variant changed'
            assert int(seq, 16) == (frames[-1][1] + 1) % (1 << 32), 'missing/repeated sequence'
        checksum, messages = analyse(ident, payload)
        assert checksum == int(xor, 16), f'{path}: checksum'
        summaries.add(messages)
        frames.append((ident, int(seq, 16)))
    assert frames, f'{path}: no complete HI11 frames'
    print(f'{path}: {len(frames)} frames OK, variant={frames[0][0]}, '
          f'sequences={frames[0][1]}..{frames[-1][1]}; framing, controls and XOR OK')
    for messages in sorted(summaries):
        for message in messages:
            print('  ' + message)
    return len(frames)


if __name__ == '__main__':
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    for arg in sys.argv[1:]:
        check(arg)
