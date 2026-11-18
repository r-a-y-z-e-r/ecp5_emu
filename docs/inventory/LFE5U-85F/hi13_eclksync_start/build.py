"""Build HI13 in external scratch; validate the actual route and configuration."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def run(command, directory):
    with (directory / (Path(command[0]).name + '.log')).open('w') as log:
        subprocess.run(command, cwd=directory, stdout=log, stderr=subprocess.STDOUT, check=True)


def check(directory):
    model = json.loads((directory/'routed.json').read_text())['modules']['top']
    cells = model['cells']
    text = (directory/'top.config').read_text()
    tiles, tile = {}, None
    for line in text.splitlines():
        if line.startswith(('.tile ', '.tile_group ')):
            tile = line.split()[1]
            tiles.setdefault(tile, {})
        elif line.startswith('.'):
            tile = None
        elif tile and line.startswith('enum: '):
            _, key, value = line.split()
            tiles[tile][key] = value
    e = tiles['CIB_R46C2:ECLK_L']
    assert sum(c['type'] == 'ECLKSYNCB' for c in cells.values()) == 2
    for lane, bank, index, y in [('p', 7, 1, 46), ('g', 6, 0, 47)]:
        sync, div = cells['sync_'+lane], cells['div_'+lane]
        assert sync['attributes']['NEXTPNR_BEL'] == f'X0/Y{y}/ECLKSYNC{index}_BK{bank}'
        assert e[f'ECLKSYNC{index}_BK{bank}.MODE'] == 'ECLKSYNCB'
        assert sync['connections']['ECLKO'] == div['connections']['CLKI']
        assert div['attributes']['NEXTPNR_BEL'] == f'X0/Y46/CLKDIV{index}'
        assert e[f'CLKDIV_L{index}.DIV'] == '2.0'
        assert e[f'CLKDIV_L{index}.GSR'] == 'DISABLED'
        stop = sync['connections']['STOP']
        grounds = [c['connections']['F'] for c in cells.values()
                   if c['type'] == 'TRELLIS_COMB' and int(c['parameters']['INITVAL'], 2) == 0]
        assert stop in grounds or stop == ['0'], ('STOP must be constant zero', stop, grounds)
    assert cells['sync_p']['connections']['ECLKI'] == cells['clk$tr_io']['connections']['O']
    assert cells['sync_g']['connections']['ECLKI'] == cells['delayed_buffer']['connections']['CLKO']
    assert 'arc: W2_JECLKI1 G_JPCLKT61' in text
    for name in ['support_buffer', 'delayed_buffer']:
        cell = cells[name]
        assert cell['type'] == 'DCCA'
        bel = cell['attributes']['NEXTPNR_BEL'].split('/')[-1]
        field = f'DCC_{bel[0]}{bel[4:]}.MODE'
        assert any(settings.get(field) == 'DCCA' for settings in tiles.values()), (bel, field)
    assert cells['delayed_buffer']['connections']['CE'] != cells['support_buffer']['connections']['CE']
    assert cells['delayed_buffer']['connections']['CE'] == cells['ce_TRELLIS_FF_Q']['connections']['Q']
    counters = dict.fromkeys(['i', 'a', 'gr', 'pr', 'pf', 'qr', 'qf'], 0)
    for name, cell in cells.items():
        if cell['type'] != 'TRELLIS_FF':
            continue
        assert cell['parameters']['GSR'].strip() == 'DISABLED', name
        x, y, sl, reg = re.fullmatch(r'X(\d+)/Y(\d+)/(SLICE[A-D])\.FF([01])', cell['attributes']['NEXTPNR_BEL']).groups()
        settings = tiles[f'R{y}C{x}:PLC2']
        assert settings[f'{sl}.GSR'] == 'DISABLED', name
        base = name.split('_TRELLIS_FF_Q')[0]
        if base in counters or base in ('ce', 'ce_at', 'freeze'):
            if base in counters:
                counters[base] += 1
            assert settings[f'{sl}.REG{reg}.REGSET'] == 'RESET', name
            assert cell['parameters']['CLKMUX'].strip() == ('INV' if base in ('pf', 'qf', 'ce', 'ce_at', 'freeze') else 'CLK'), name
    assert set(counters.values()) == {32}, counters
    assert not any(c['type'] in ('GSR', 'SGSR') for c in cells.values())
    lines = []
    tile = None
    for line in text.splitlines():
        if line.startswith(('.tile ', '.tile_group ')):
            tile = line.split()[1]
        if re.search(r'^enum: (ECLKSYNC|CLKDIV|DCC)', line):
            lines.append(f'{tile}: {line}')
    for name in ['sync_p', 'div_p', 'sync_g', 'div_g', 'delayed_buffer', 'support_buffer']:
        lines.append(f'{name}: {cells[name]["attributes"]["NEXTPNR_BEL"]}')
    lines.append('PASS: raw G2 P entry; G DCCA connectivity; both STOP=0; /2 GSR DISABLED; all FF GSR DISABLED; 224 counter bits RESET/polarity checked; route and pack')
    return '\n'.join(lines)+'\n'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    args = p.parse_args()
    scratch = args.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        p.error('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=True)
    for name in ['top.v', 'top.lpf', 'route_p.py']:
        shutil.copyfile(HERE/name, scratch/name)
    run([os.environ.get('YOSYS', 'yosys'), '-Q', '-q', '-p',
         'read_verilog top.v; synth_ecp5 -top top; setparam -set GSR "DISABLED" t:TRELLIS_FF; write_json top.json'], scratch)
    run([os.environ.get('NEXTPNR_ECP5', 'nextpnr-ecp5'), '--85k', '--package', 'CABGA381',
         '--seed', '1', '--json', 'top.json', '--lpf', 'top.lpf', '--textcfg', 'top.config',
         '--write', 'routed.json', '--allow-fabric-eclk', '--pre-route', 'route_p.py'], scratch)
    run([os.environ.get('ECPPACK', 'ecppack'), 'top.config', 'top.bit'], scratch)
    report = check(scratch)
    (scratch/'validation.txt').write_text(report)
    (scratch/'SHA256SUMS').write_text(''.join(
        f'{hashlib.sha256((scratch/name).read_bytes()).hexdigest()}  {name}\n'
        for name in ['top.v', 'top.lpf', 'top.json', 'routed.json', 'top.config', 'top.bit']))
    print(report, end='')


if __name__ == '__main__':
    main()
