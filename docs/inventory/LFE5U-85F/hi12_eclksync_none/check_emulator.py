"""Run routed HI12 bitstreams in the actual emulator, without board access.
Usage: check_emulator.py BUILD_DIR --emulator target/release/ecp5_emu
Use --legacy --variants none_default sync_2 to reproduce the failed profile.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
sys.dont_write_bytecode = True
from hi12_gen import VARIANTS
from hi12_model import decode, predict
from run import parse


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('build', type=Path)
    p.add_argument('--emulator', type=Path, default=Path('target/release/ecp5_emu'))
    p.add_argument('--variants', nargs='+', choices=VARIANTS, default=list(VARIANTS))
    p.add_argument('--legacy', action='store_true', help='old finished-gated UART has no heartbeat')
    args = p.parse_args()
    emulator, build = args.emulator.resolve(), args.build.resolve()
    report = []
    for variant in args.variants:
        directory = build/variant
        command = [str(emulator), 'emulate', str(directory/'top.bit'),
                   '--names', str(directory/'routed.json'), '--startup', 'configured',
                   '--scope', 'observed', '--clock', 'pin:G2', '--period-ns', '40',
                   '--probe', 'net:locked', '--probe', 'net:finished']
        result = subprocess.run(command+['--cycles', '1600000', '--fast', '--console-out', 'pin:L4'],
                                stdin=subprocess.DEVNULL, capture_output=True, check=True)
        (directory/'emulator.uart.bin').write_bytes(result.stdout)
        (directory/'emulator.status.log').write_bytes(result.stderr)
        records, heartbeats = parse(result.stdout)
        assert records, (variant, 'no complete record')
        assert args.legacy or heartbeats, (variant, 'no early heartbeat')
        wanted = 4 if variant.startswith('sync') else 0
        seven = variant.endswith('35')
        for record in records:
            identifier, snapshots, _ = decode(record)
            assert identifier == VARIANTS.index(variant)
            assert all(predict(i, wanted, seven) == (up, down) for i, up, down in snapshots), (variant, snapshots)
        status = result.stderr.decode()
        assert re.search(r'^0 ns: .*locked=1', status, re.M), status
        assert re.search(r'^64000000 ns: .*finished=1.*locked=1', status, re.M), status
        entry = dict(variant=variant, legacy=args.legacy, startup='configured', scope='observed',
                     simulated_cycles=1600000, lock_at_time_zero=1, finished_at_64ms=1,
                     complete_records=len(records), heartbeats=len(heartbeats), snapshots=snapshots)
        report.append(entry)
        print(f'{variant}: emulator LOCK=1 at time zero; finished=1 at 64 ms; '
              f'HC={len(records)} HB={len(heartbeats)}; snapshot counts PASS', flush=True)
    directory = build/args.variants[0]
    plan = directory/'emulator.plan.json'
    subprocess.run([str(emulator), 'emulate', str(directory/'top.bit'),
                    '--names', str(directory/'routed.json'), '--startup', 'configured',
                    '--scope', 'observed', '--clock', 'pin:G2', '--period-ns', '40',
                    '--cycles', '8', '--probe', 'net:locked', '--probe', 'net:observer',
                    '--probe', 'net:finished', '--output', str(plan)],
                   stdin=subprocess.DEVNULL, capture_output=True, check=True)
    pll = next(r for r in json.loads(plan.read_text())['plan']['resources'] if r['family'] == 'PLL')
    summary = dict(runs=report, pll_resource=pll['id'], pll_rule=pll['rule'],
                   feedback_fields=[f for f in pll['fields'] if f['name'] == 'FEEDBK_PATH'])
    (build/'emulator_checks.json').write_text(json.dumps(summary, indent=2)+'\n')


if __name__ == '__main__':
    main()
