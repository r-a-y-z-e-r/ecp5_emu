"""Verify every routed FF's GSR enum and probe REGSET/clock selection.
Usage: python3 -B check_routed.py DESIGN.routed.json DESIGN.config
"""
import argparse
import json
from pathlib import Path
import re


def check(routed, config):
    cells = json.loads(Path(routed).read_text())['modules']['top']['cells']
    tiles = {}
    current = None
    for line in Path(config).read_text().splitlines():
        if line.startswith(('.tile ', '.tile_group ')):
            current = line.partition(' ')[2]
            tiles[current] = {}
        elif line.startswith('.'):
            current = None
        elif current and line.startswith('enum: '):
            _, key, value = line.split()
            tiles[current][key] = value
    sync = [(t, e) for t, e in tiles.items() if 'GSR.SYNCMODE' in e]
    assert len(sync) == 1, sync
    tile, enums = sync[0]
    assert enums['GSR.SYNCMODE'] == 'SYNC', sync
    mode = [(t, e['GSR.GSRMODE']) for t, e in tiles.items() if 'GSR.GSRMODE' in e]
    assert len(mode) == 1 and mode[0][1] == 'ACTIVE_LOW', mode
    print(f'{Path(config).name}: {tile} GSR.SYNCMODE SYNC; {mode[0][0]} GSR.GSRMODE ACTIVE_LOW')
    assert cells['u']['type'] == 'GSR', cells['u']['type']
    for group in range(3):
        clock = cells[f'probe_{group}_reset']['connections']['CLK']
        assert clock == cells[f'probe_{group}_set']['connections']['CLK'], group
        assert (clock == cells['u']['connections']['CLK']) == (group != 2), group
    expected = {f'probe_{g}_{s}' for g in range(3) for s in ('reset', 'set')}
    seen = set()
    count = 0
    for name, cell in cells.items():
        if cell['type'] == 'DP16KD':
            assert cell['parameters']['GSR'].rstrip() == 'DISABLED', name
        if cell['type'] != 'TRELLIS_FF':
            continue
        x, y, sl, reg = re.fullmatch(
            r'X(\d+)/Y(\d+)/(SLICE[A-D])\.FF([01])',
            cell['attributes']['NEXTPNR_BEL']).groups()
        loc = f'R{y}C{x}:PLC2'
        e = tiles[loc]
        gsr = 'ENABLED' if name in expected else 'DISABLED'
        assert cell['parameters']['GSR'].rstrip() == gsr, (name, 'parameter')
        assert e[f'{sl}.GSR'] == gsr, (name, loc, sl, e.get(f'{sl}.GSR'))
        if name in expected:
            seen.add(name)
            setting = name.split('_')[2].upper()
            mux = 'INV' if name.startswith('probe_1_') else 'CLK'
            assert cell['parameters']['REGSET'].rstrip() == setting, name
            assert e[f'{sl}.REG{reg}.REGSET'] == setting, name
            assert cell['parameters']['CLKMUX'].rstrip() == mux, name
            print(f'  {name}: {loc} {sl}.REG{reg} REGSET={setting} GSR={gsr} CLKMUX={mux}')
        count += 1
    assert seen == expected, seen
    ebr_gsr = [v for e in tiles.values() for k, v in e.items()
               if re.fullmatch(r'EBR\d+\.GSR', k)]
    assert ebr_gsr and set(ebr_gsr) == {'DISABLED'}, ebr_gsr
    print(f'  PASS: {len(seen)} enabled probes; {count-len(seen)} disabled support FFs; EBR GSR disabled')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('routed_json')
    p.add_argument('config')
    a = p.parse_args()
    check(a.routed_json, a.config)
