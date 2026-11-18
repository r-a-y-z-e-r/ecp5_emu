"""Replay the packed HI13 bitstream through the actual emulator CLI; no hardware."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
from analyze import inspect
from run import parse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build', type=Path)
    parser.add_argument('--emulator', type=Path, default=Path('target/release/ecp5_emu'))
    args = parser.parse_args()
    build, emulator = args.build.resolve(), args.emulator.resolve()
    command = [str(emulator), 'emulate', str(build/'top.bit'), '--names', str(build/'routed.json'),
               '--startup', 'configured', '--scope', 'observed', '--clock', 'pin:G2',
               '--period-ns', '40', '--cycles', '1600000', '--fast', '--console-out', 'pin:L4']
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, check=True)
    (build/'emulator.uart.bin').write_bytes(result.stdout)
    (build/'emulator.status.log').write_bytes(result.stderr)
    records, heartbeats = parse(result.stdout)
    assert records and heartbeats and len(set(records)) == 1, (records, heartbeats)
    reading = inspect(records[0])
    assert not reading['errors'], reading
    # Explicit expected values, not an emulation of the RTL serializer.
    for snapshot, i in zip(reading['snapshots'], (524288, 1048576)):
        expected = dict(I=i, A=i-64, CE_at=64, G_in=i-64,
                        P_r=i//2, P_f=i//2,
                        G_r=(i-68)//2, G_f=(i-68)//2, P_deficit=0, G_deficit=4)
        assert snapshot == expected, (snapshot, expected)
    summary = dict(startup='configured', scope='observed', simulated_pin_cycles=1600000,
                   complete_records=len(records), heartbeats=len(heartbeats),
                   bitstream_sha256=hashlib.sha256((build/'top.bit').read_bytes()).hexdigest(),
                   emulator_sha256=hashlib.sha256(emulator.read_bytes()).hexdigest(), **reading)
    (build/'emulator_checks.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
