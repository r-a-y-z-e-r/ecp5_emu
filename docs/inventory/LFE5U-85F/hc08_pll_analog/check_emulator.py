"""Run each packed bitstream in the real CLI; retain plan/refusal and CLKOP trace."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
sys.dont_write_bytecode = True
from analyse import analyse, parse
from gen import BASE_PROFILES, PROFILES


def transitions(vcd, signal):
    ids, values, now = set(), [], 0
    for line in vcd.splitlines():
        fields = line.split()
        if len(fields) >= 5 and fields[0] == '$var' and fields[4].lstrip('\\') == signal:
            ids.add(fields[3])
        if line.startswith('#'):
            now = int(line[1:])
        elif len(line) >= 2 and line[0] in '01xz' and line[1:] in ids:
            values.append((now, line[0]))
    return values


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('build', type=Path)
    p.add_argument('--emulator', type=Path, required=True)
    p.add_argument('--profiles', nargs='+', choices=PROFILES, default=BASE_PROFILES)
    p.add_argument('--uart-cycles', type=int, default=800000,
                   help='32 ms by default; 101000000 also exercises startup replay')
    args = p.parse_args()
    emulator = str(args.emulator.resolve())
    summaries = []
    for profile in args.profiles:
        d = (args.build/profile).resolve()
        # The event CLI samples at input/stimulus times. Repeat G2's value
        # every 1 ns so 100 MHz cannot alias against the 25 MHz input edges.
        (d/'sample_1ns.json').write_text(json.dumps([
            dict(time=t, set={'pin:G2': str((t//20) % 2)}) for t in range(1, 2561)])+'\n')
        base = [emulator, 'emulate', str(d/'top.bit'), '--names', str(d/'routed.json'),
                '--startup', 'configured', '--scope', 'observed',
                '--period-ns', '40',
                '--probe', 'net:locked', '--probe', 'net:clkop']
        r = subprocess.run(base+['--set', 'pin:G2=0', '--stimulus', str(d/'sample_1ns.json'),
                                '--vcd', str(d/'emulator.vcd'),
                                '--output', str(d/'emulator.plan.json')],
                           capture_output=True, stdin=subprocess.DEVNULL)
        (d/'emulator.trace.log').write_bytes(r.stdout+r.stderr)
        entry = dict(profile=profile, plans=r.returncode == 0,
                     startup='configured', scope='observed')
        if r.returncode:
            entry['refusal'] = (r.stdout+r.stderr).decode(errors='replace')
            summaries.append(entry)
            print(f'{profile}: REFUSED: '+entry['refusal'], flush=True)
            continue
        plan = json.loads((d/'emulator.plan.json').read_text())
        pll = [resource for resource in plan['plan']['resources'] if resource['family'] == 'PLL']
        entry['pll_resources'] = pll
        trace = (d/'emulator.vcd').read_text()
        assert '$timescale 1ns $end' in trace, 'update VCD time conversion'
        lock = transitions(trace, 'locked')
        clock = transitions(trace, 'clkop')
        rising = [t for t, v in clock if v == '1']
        periods = sorted(set(b-a for a, b in zip(rising, rising[1:])))
        assert lock and rising, 'missing requested probes'
        entry.update(lock_transitions_ns=lock, clkop_periods_ns=periods,
                     first_clkop_rise_ns=rising[0])
        r = subprocess.run(base+['--clock', 'pin:G2', '--cycles', str(args.uart_cycles), '--fast',
                                '--console-out', 'pin:L4'],
                           capture_output=True, stdin=subprocess.DEVNULL)
        (d/'emulator.uart.bin').write_bytes(r.stdout)
        (d/'emulator.uart.log').write_bytes(r.stderr)
        lines = []
        good = analyse(r.stdout, f'{profile} emulator', lines.append, require_startup=False)
        (d/'emulator.analysis.txt').write_text('\n'.join(lines)+'\n')
        entry.update(uart_returncode=r.returncode, uart_records=len(parse(r.stdout)),
                     uart_cycles=args.uart_cycles, uart_parse_ok=good)
        if args.uart_cycles >= 800000:
            assert r.returncode == 0 and good, r.stderr.decode(errors='replace')
        print(f'{profile}: plans; LOCK={lock}; CLKOP periods={periods} ns; '
              f'UART records={entry["uart_records"]}', flush=True)
        summaries.append(entry)
    (args.build/'emulator_checks.json').write_text(json.dumps(summaries, indent=2)+'\n')


if __name__ == '__main__':
    main()
