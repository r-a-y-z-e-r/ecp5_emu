"""Build hy01b/c in external scratch, without accessing board or serial.
Usage: python3 -B build_boot.py /private/tmp/hy01-boot-build
"""
import argparse
import json
from pathlib import Path
import shutil
import sys
sys.dont_write_bytecode = True
from build import HERE, REPO, run
from check_boot_routed import check
from hy01_boot_gen import counter_names, generate


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    a = p.parse_args()
    scratch = a.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        p.error('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(HERE/'hy01.lpf', scratch/'hy01.lpf')
    for variant in ('b', 'c'):
        name = f'hy01{variant}'
        (scratch/f'{name}.v').write_text(generate(variant))
        script = (f'read_verilog {name}.v; synth_ecp5 -top top; '
                  'setparam -set GSR "DISABLED" t:TRELLIS_FF; '
                  'setparam -set GSR "ENABLED" c:count_a_*; '
                  f'write_json {name}.json')
        run(['yosys', '-Q', '-q', '-l', f'{name}.yosys.log', '-p', script], scratch)
        cells = json.loads((scratch/f'{name}.json').read_text())['modules']['top']['cells']
        enabled = {n for n,c in cells.items() if c['type'] == 'TRELLIS_FF'
                   and c['parameters']['GSR'] == 'ENABLED'}
        assert enabled == counter_names(True), enabled
        run(['nextpnr-ecp5', '--85k', '--package', 'CABGA381', '--seed', '1',
             '--json', f'{name}.json', '--lpf', 'hy01.lpf', '--freq', '25',
             '--textcfg', f'{name}.config', '--write', f'{name}.routed.json',
             '-l', f'{name}.nextpnr.log', '--quiet'], scratch)
        check(variant, scratch/f'{name}.routed.json', scratch/f'{name}.config')
        run(['ecppack', f'{name}.config', f'{name}.bit'], scratch)


if __name__ == '__main__':
    main()
