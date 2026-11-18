"""Build both HY01 variants in an external scratch directory; never load a board.
Usage: python3 -B build.py /private/tmp/hy01-build
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
from hy01_gen import generate
from check_routed import check

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def run(command, directory):
    print('+', ' '.join(command), flush=True)
    subprocess.run(command, cwd=directory, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scratch', type=Path)
    args = parser.parse_args()
    scratch = args.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        parser.error('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(HERE / 'hy01.lpf', scratch / 'hy01.lpf')
    for name in ('hy01', 'hy01s'):
        (scratch / f'{name}.v').write_text(generate(name.endswith('s')))
        script = (f'read_verilog {name}.v; synth_ecp5 -top top; '
                  'setparam -set GSR "DISABLED" t:TRELLIS_FF; '
                  'setparam -set GSR "ENABLED" c:probe_*; '
                  'setparam -set GSR "DISABLED" t:DP16KD; '
                  f'write_json {name}.json')
        run(['yosys', '-Q', '-q', '-l', f'{name}.yosys.log', '-p', script], scratch)
        cells = json.loads((scratch / f'{name}.json').read_text())['modules']['top']['cells']
        enabled = {n for n, c in cells.items() if c['type'] == 'TRELLIS_FF'
                   and c['parameters'].get('GSR') == 'ENABLED'}
        expected = {f'probe_{g}_{s}' for g in range(3) for s in ('reset', 'set')}
        assert enabled == expected, enabled
        run(['nextpnr-ecp5', '--85k', '--package', 'CABGA381', '--seed', '1',
             '--json', f'{name}.json', '--lpf', 'hy01.lpf', '--freq', '25',
             '--textcfg', f'{name}.config', '--write', f'{name}.routed.json',
             '-l', f'{name}.nextpnr.log', '--quiet'], scratch)
        check(scratch / f'{name}.routed.json', scratch / f'{name}.config')
        run(['ecppack', f'{name}.config', f'{name}.bit'], scratch)


if __name__ == '__main__':
    main()
