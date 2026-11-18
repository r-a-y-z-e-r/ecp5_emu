"""Check every routed startup-counter/support FF and the b/c GSR resource.
Usage: check_boot_routed.py b DESIGN.routed.json DESIGN.config
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys
sys.dont_write_bytecode = True
from hy01_boot_gen import counter_names


def check(variant, routed, config):
    cells = json.loads(Path(routed).read_text())['modules']['top']['cells']
    text = Path(config).read_text()
    tiles, current = {}, None
    for line in text.splitlines():
        if line.startswith(('.tile ', '.tile_group ')):
            current = line.partition(' ')[2]
            tiles[current] = {}
        elif line.startswith('.'):
            current = None
        elif current and line.startswith('enum: '):
            _, key, value = line.split()
            tiles[current][key] = value
    gsr = [(n, c) for n, c in cells.items() if c['type'] in ('SGSR', 'GSR')]
    sync = re.findall(r'^enum: GSR\.SYNCMODE (\S+)$', text, re.M)
    mode = re.findall(r'^enum: GSR\.GSRMODE (\S+)$', text, re.M)
    if variant == 'b':
        assert len(gsr) == 1 and gsr[0][1]['type'] == 'GSR', gsr
        assert sync == ['SYNC'] and mode == ['ACTIVE_LOW'], (sync, mode)
        assert gsr[0][1]['connections']['CLK'] == cells['count_a_r_00']['connections']['CLK']
        # nextpnr can express a constant through its VCC cell rather than a literal bit.
        signal = gsr[0][1]['connections']['GSR']
        assert signal == ['1'] or any(
            (c['type'] == 'VCC' and c['connections'].get('Z') == signal) or
            (c['type'] == 'TRELLIS_COMB' and c['connections'].get('F') == signal and
             int(c['parameters'].get('INITVAL', '0'), 2) == 65535)
            for c in cells.values()), signal
    else:
        assert not gsr and 'SYNC' not in sync, (gsr, sync)
        assert not mode or mode == ['NONE'], mode
    print(f'hy01{variant}: GSR resources={len(gsr)}; SYNCMODE={sync or "not emitted"}; GSRMODE={mode or "not emitted"}')
    expected = counter_names(True) | counter_names(False)
    seen, totals = set(), Counter()
    for name, cell in cells.items():
        if cell['type'] != 'TRELLIS_FF':
            continue
        x, y, sl, reg = re.fullmatch(r'X(\d+)/Y(\d+)/(SLICE[A-D])\.FF([01])',
                                    cell['attributes']['NEXTPNR_BEL']).groups()
        e = tiles[f'R{y}C{x}:PLC2']
        enabled = name in counter_names(True)
        setting = 'ENABLED' if enabled else 'DISABLED'
        assert cell['parameters']['GSR'].rstrip() == setting, name
        assert e[f'{sl}.GSR'] == setting, (name, sl, e)
        if name in expected:
            seen.add(name)
            assert cell['parameters']['REGSET'].rstrip() == 'RESET', name
            assert e[f'{sl}.REG{reg}.REGSET'] == 'RESET', name
            clock = 'INV' if '_f_' in name else 'CLK'
            assert cell['parameters']['CLKMUX'].rstrip() == clock, name
            bank = '_'.join(name.split('_')[1:3])
            totals[bank] += 1
            print(f'  {name}: R{y}C{x}:PLC2 {sl}.REG{reg} GSR={setting} REGSET=RESET CLKMUX={clock}')
        else:
            totals['support'] += 1
    assert seen == expected, seen ^ expected
    assert all(totals[bank] == 16 for bank in ('a_r', 'b_r', 'a_f', 'b_f')), totals
    assert not any(c['type'] == 'EHXPLLL' for c in cells.values())
    print(f'  PASS: {dict(sorted(totals.items()))}; all support FFs GSR-disabled; no PLL')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('variant', choices=('b', 'c'))
    p.add_argument('routed_json')
    p.add_argument('config')
    a = p.parse_args()
    check(a.variant, a.routed_json, a.config)
