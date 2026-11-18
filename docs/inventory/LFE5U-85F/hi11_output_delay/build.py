"""Build all HI11 variants without accessing hardware.
Usage: python3 build.py SCRATCH_DIR [hi11s1 hi11s63 hi11s64 hi11s127]
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
from hi11s_gen import VALUES, THREE_LANE_IDS

source = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('out', type=Path)
parser.add_argument('designs', nargs='*')
args = parser.parse_args()
designs = args.designs or ['hi11', 'hi11g', 'hi11b']
startup_designs = {f'hi11s{value}' for value in VALUES.values()}
three_lane_designs = {f'hi11s{VALUES[ident]}' for ident in THREE_LANE_IDS}
if any(name not in {'hi11', 'hi11g', 'hi11b'} | startup_designs for name in designs):
    parser.error('unknown HI11 design')
out = args.out.resolve()
out.mkdir(parents=True, exist_ok=True)
for name in designs:
    work = out / name
    work.mkdir(exist_ok=True)
    shutil.copyfile(source / f'{name}.v', work / 'top.v')
    lpf = 'hi11s3.lpf' if name in three_lane_designs else (
        'hi11s.lpf' if name in startup_designs else 'hi11.lpf')
    shutil.copyfile(source / lpf, work / 'top.lpf')
    with (work / 'build.log').open('w') as log:
        for command in (
            ['yosys', '-Q', '-p', 'read_verilog top.v; synth_ecp5 -top top -json top.json'],
            ['nextpnr-ecp5', '--85k', '--package', 'CABGA381', '--seed', '1',
             '--json', 'top.json', '--lpf', 'top.lpf', '--textcfg', 'top.config',
             '--write', 'top_routed.json'],
            ['ecppack', 'top.config', 'top.bit'],
        ):
            subprocess.run(command, cwd=work, stdout=log, stderr=log, check=True)
    print(f'{name}: {work}/top.bit')
    # Equivalent to grep on the routed config; retain the tile qualification.
    for tile in (work / 'top.config').read_text().split('.tile '):
        if 'DELAY.OUTDEL ' in tile:
            print('  .tile ' + tile.splitlines()[0])
            for line in tile.splitlines():
                if any(s in line for s in ('DELAY.', '.MODE ', 'DATAMUX_', 'BASE_TYPE ', 'PULLMODE ')):
                    print('  ' + line)
    if name.startswith('hi11s'):
        three_lanes = name in three_lane_designs
        cells = json.loads((work / 'top_routed.json').read_text())['modules']['top']['cells']
        delays = [cell for cell in cells.values() if cell['type'] in ('IOLOGIC', 'SIOLOGIC')]
        assert len(delays) == (3 if three_lanes else 2), 'every DELAYF must survive packing'
        assert sorted(cell['type'] for cell in delays) == (
            ['IOLOGIC', 'IOLOGIC', 'SIOLOGIC'] if three_lanes else ['IOLOGIC', 'IOLOGIC'])
        expected_value = f'{int(name[5:]):032b}'
        for cell in delays:
            params = cell['parameters']
            assert params['DELAY.DEL_VALUE'] == expected_value
            assert cell['connections']['CFLAG'], 'every flag must be recorded'
            print('  routed BEL=' + cell['attributes']['NEXTPNR_BEL'] +
                  ' MODE=' + params['MODE'] + ' OUTDEL=' + params.get('DELAY.OUTDEL', 'DISABLED'))
        configs = [cell['parameters'].get('DELAY.OUTDEL', 'DISABLED') for cell in delays]
        assert sorted(configs) == (['DISABLED', 'DISABLED', 'ENABLED'] if three_lanes
                                   else ['DISABLED', 'ENABLED'])
        for port in ('LOADN', 'MOVE', 'DIRECTION'):
            assert all(cell['connections'][port] == delays[0]['connections'][port] for cell in delays)
        header = cells['side_input$tr_io']
        assert header['parameters']['DIR'] == 'INPUT'
        assert header['connections']['I'] == [] and header['connections']['T'] == []
        assert header['attributes']['NEXTPNR_BEL'] == 'X0/Y35/PIOA'
        if three_lanes:
            bottom = cells['bottom_input$tr_io']
            assert bottom['parameters']['DIR'] == 'INPUT'
            assert bottom['connections']['I'] == [] and bottom['connections']['T'] == []
            assert bottom['attributes']['NEXTPNR_BEL'] == 'X4/Y95/PIOA'
            assert cells['bottom_input$tr_io$IOL']['attributes']['NEXTPNR_BEL'] == 'X4/Y95/SIOLOGICA'
    print('  bit SHA256=' + hashlib.sha256((work / 'top.bit').read_bytes()).hexdigest())
