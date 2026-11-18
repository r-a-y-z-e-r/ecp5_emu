#!/usr/bin/env python3
"""Additional system checks, after run.sh. All artifacts stay in target/."""
import pathlib
import re
import subprocess

folder = pathlib.Path(__file__).resolve().parents[1]
root = folder.parents[2]
out = root / 'target/gallery/sdram_bench/sim'
exe = out / 'obj/Vsim_top'
canonical = (folder / 'expected/uart.txt').read_bytes()


def run(name, keys, cycles, scenario='normal', delay=0):
    keyfile, transcript = out / (name + '.bin'), out / (name + '.txt')
    keyfile.write_bytes(keys)
    subprocess.run([str(exe), str(cycles), str(keyfile), str(transcript),
                    scenario, str(delay)], check=True)
    text = transcript.read_text()
    lines = [line for line in text.splitlines() if line.startswith('PASS ')]
    for line in lines:
        m = re.fullmatch(r'PASS (\d+) SIZE (2|32) WRITE (\d+) (\d+\.\d) '
                         r'READ (\d+) (\d+\.\d) ERRORS (\d+)', line)
        assert m, line
        _, size, wc, wb, rc, rb, errors = m.groups()
        for clocks, bw in [(wc, wb), (rc, rb)]:
            tenths = int(size) * 250_000_000 // int(clocks)
            assert bw == f'{tenths // 10}.{tenths % 10}', line
        assert errors == ('1' if scenario == 'fault' else '0'), line
    return text, lines


text, _ = run('delayed_restart', b'r', 6_000_000, delay=1_000_000)
assert text.encode() == canonical, 'restart depends on arrival time'
text, lines = run('small_passes', b'rs', 10_000_000)
assert [int(line.split()[1]) for line in lines] == [1, 2, 3]
assert all('SIZE 2 ' in line for line in lines)
text, _ = run('fifo_restart', b'rrfs', 6_000_000)
assert text.count('RESTART\n') == 2, 'restart cleared or reordered the FIFO'
text, _ = run('fault', b'r', 6_000_000, 'fault')
assert 'FIRST ERROR 123 55194 55195\n' in text
text, lines = run('down_button', b'r', 10_000_000, 'down')
assert len(lines) == 3 and all('SIZE 2 ' in line for line in lines)
text, lines = run('fire_button', b'r', 8_000_000, 'fire')
assert text.count('RESTART\n') == 2
assert [int(line.split()[1]) for line in lines] == [1, 1]
print('System checks passed: delayed r, ordered FIFO, repeated seeds, error data/LED, DOWN, FIRE1')
