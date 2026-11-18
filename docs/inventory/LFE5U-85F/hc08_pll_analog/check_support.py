"""No hardware: fault-injected RTL, replay, UART parser and load-before-open tests."""
import argparse
from pathlib import Path
import subprocess
import sys
from unittest.mock import patch
sys.dont_write_bytecode = True
from analyse import analyse, parse, STEP
from run import collect

HERE = Path(__file__).resolve().parent


def runner_checks(scratch):
    raw_data = b'\xffpartial\r\nHC08L00000000000000000\r\ntruncated'
    assert len(parse(raw_data)) == 1
    messages = []
    assert not analyse(b'', 'empty', messages.append)
    assert not analyse(raw_data, 'partial', messages.append)
    events = []

    class Port:
        def __init__(self, *args, **kwargs):
            events.append('open')
        def __enter__(self):
            return self
        def __exit__(self, *args):
            pass
        def read(self, length):
            events.append('read')
            return raw_data

    def loader(argv, **kwargs):
        assert argv == ['openFPGALoader', '-b', 'ulx3s', 'design.bit']
        events.append('load')

    with patch('run.subprocess.run', side_effect=loader), \
         patch('run.time.monotonic', side_effect=[0, 0, 2]):
        assert collect('design.bit', 'unused', scratch/'raw.bin', scratch/'load.log', 1, Port) is None
    assert events == ['load', 'open', 'read'], events
    assert (scratch/'raw.bin').read_bytes() == raw_data
    events.clear()
    with patch('run.subprocess.run', side_effect=subprocess.CalledProcessError(1, 'loader')):
        assert collect('design.bit', 'unused', scratch/'failed.bin', scratch/'failed.log', 1, Port)
    assert not events and (scratch/'failed.bin').read_bytes() == b''

    class BrokenPort(Port):
        def read(self, length):
            if 'read' in events:
                raise OSError('injected read failure')
            return super().read(length)

    with patch('run.subprocess.run', side_effect=loader), \
         patch('run.time.monotonic', side_effect=[0, 0, 0]):
        assert collect('design.bit', 'unused', scratch/'broken.bin', scratch/'broken.log', 1, BrokenPort)
    assert (scratch/'broken.bin').read_bytes() == raw_data
    print('PASS: strict parser; empty/partial captures; SRAM-load-before-open; '
          'binary bytes retained; loader/read failures preserve output')


def rtl_checks(scratch):
    executable = scratch/'fault.vvp'
    subprocess.run(['iverilog', '-g2012', '-s', 'fault_tb', '-o', str(executable),
                    'reporter.v', 'fault_tb.v'], cwd=HERE, check=True)
    for mode, label, ratio in [(0, 'no CLKOP, LOCK=0', 0),
                               (1, '100 MHz, LOCK=0', 4),
                               (2, '100 MHz, LOCK rises at 15 ms', 4),
                               (3, '62.5 MHz, LOCK=0', 2.5)]:
        raw = scratch/f'fault-{mode}.bin'
        subprocess.run(['vvp', str(executable), f'+MODE={mode}', f'+OUT={raw}'], check=True)
        records = parse(raw.read_bytes())
        live = [r for r in records if r['kind'] == 'L']
        replay = [r for r in records if r['kind'] == 'S']
        assert len(live) == 8 and len(replay) == 4, (label, records)
        assert [r['pin'] for r in live] == [i*STEP for i in range(8)]
        assert replay == [dict(r, kind='S') for r in live[:4]], label
        for a, b in zip(live[1:], live[2:]):
            assert abs((b['op']-a['op'])/STEP-ratio) < 0.0001, (label, a, b)
        assert [r['lock'] for r in live] == ([0, 0]+[1]*6 if mode == 2 else [0]*8)
        print(f'PASS: RTL {label}; 10 ms heartbeat, edge ratio={ratio:g}, exact startup replay', flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    args = p.parse_args()
    scratch = args.scratch.resolve()
    repo = HERE.parents[3]
    if scratch == repo or repo in scratch.parents:
        p.error('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=False)
    runner_checks(scratch)
    rtl_checks(scratch)
