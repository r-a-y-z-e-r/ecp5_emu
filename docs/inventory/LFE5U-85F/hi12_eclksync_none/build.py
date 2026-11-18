"""Build and validate all HI12 variants in external scratch. No board access."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
from hi12_gen import PLL, VARIANTS, generate

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def run(command, cwd):
    tool = Path(command[0]).name
    with (cwd / f'{tool}.log').open('w') as log:
        subprocess.run(command, cwd=cwd, stdout=log, stderr=subprocess.STDOUT, check=True)


def enums(text):
    result, tile = {}, None
    for line in text.splitlines():
        if line.startswith('.tile '):
            tile = line.split()[1]
            result[tile] = {}
        elif line.startswith('.'):
            tile = None
        elif tile and line.startswith('enum: '):
            _, key, value = line.split()
            result[tile][key] = value
    return result


def check_pll(model, text):
    cells = [c for c in model['cells'].values() if c['type'] == 'EHXPLLL']
    for cell in cells:
        params, attrs = cell['parameters'], cell['attributes']
        assert params['FEEDBK_PATH'].strip() == 'CLKOP'
        assert cell['connections']['CLKFB'] == cell['connections']['CLKOP']
        assert cell['connections'].get('CLKINTFB', []) == []
        for key, value in {'ICP_CURRENT': 12, 'LPF_RESISTOR': 8,
                           'MFG_ENABLE_FILTEROPAMP': 1, 'MFG_GMCREF_SEL': 2}.items():
            assert int(attrs[key].strip()) == value, (key, attrs)
            written = re.findall(rf'^word: {key} ([01]+)$', text, re.M)
            assert len(written) == 1 and int(written[0], 2) == value, (key, written)
        assert re.findall(r'^enum: FEEDBK_PATH (\S+)$', text, re.M) == ['CLKOP']
        assert re.search(r'^arc: \S*CLKFB \S*JCLKFB[123]$', text, re.M)
    return bool(cells)


def check(directory, ratio, control=False, board=False):
    model = json.loads((directory/'routed.json').read_text())['modules']['top']
    text = (directory/'top.config').read_text()
    settings = enums(text)
    has_pll = check_pll(model, text)
    dividers = [c for c in model['cells'].values() if c['type'] == 'CLKDIVF']
    assert len(dividers) == 1
    bel = dividers[0]['attributes']['NEXTPNR_BEL']
    assert bel == 'X0/Y46/CLKDIV1', bel
    e = settings['CIB_R46C2:ECLK_L']
    assert e['CLKDIV_L1.DIV'] == ratio and e['CLKDIV_L1.GSR'] == 'DISABLED'
    routing = '\n'.join(n.get('attributes', {}).get('ROUTING', '') for n in model['netnames'].values())
    sync_name = 'ECLKSYNC1_BK7'
    if not control:
        internal = re.findall(r'X2/Y46/(-2_[01])_ECLKI_ECLKSYNC1->\1_JECLKO_ECLKSYNC1', routing)
        assert len(internal) == 1, internal
        sync_name = 'ECLKSYNC1_BK7' if internal[0] == '-2_0' else 'ECLKSYNC1_BK6'
        if board:
            assert internal == ['-2_0'], 'board must keep the X13 bank 7 path'
        assert 'JSTOP_ECLKSYNC1' not in routing
    else:
        assert 'JSTOP_ECLKSYNC1' in routing
    # Missing enum is NONE, not an enabled synchronizer.
    mode = e.get(sync_name+'.MODE', 'NONE')
    assert mode == ('ECLKSYNCB' if control else 'NONE'), mode
    assert not any(c['type'] in ('GSR', 'SGSR') for c in model['cells'].values())
    if board:
        assert has_pll
        assert 'arc: W2_JECLKI1 G_JPCLKT61' in text, 'board divider must keep X13 direct pin entry'
        counts = {}
        for name, cell in model['cells'].items():
            if cell['type'] != 'TRELLIS_FF':
                continue
            assert cell['parameters']['GSR'].strip() == 'DISABLED', name
            x, y, sl, reg = re.fullmatch(r'X(\d+)/Y(\d+)/(SLICE[A-D])\.FF([01])', cell['attributes']['NEXTPNR_BEL']).groups()
            tile = settings[f'R{y}C{x}:PLC2']
            assert tile[f'{sl}.GSR'] == 'DISABLED', name
            if any(name.startswith(n+'_TRELLIS_FF_Q') for n in ('input_count', 'rising_count', 'falling_count')):
                bank = name.split('_TRELLIS_FF_Q')[0]
                counts[bank] = counts.get(bank, 0) + 1
                assert tile[f'{sl}.REG{reg}.REGSET'] == 'RESET', name
                assert cell['parameters']['CLKMUX'].strip() == ('INV' if bank == 'falling_count' else 'CLK'), name
        assert counts == dict.fromkeys(('input_count', 'rising_count', 'falling_count'), 32), counts
    return f'{directory.name}: CIB_R46C2:ECLK_L {sync_name}.MODE={mode}; CLKDIV_L1 DIV={ratio} GSR=DISABLED; STOP={"constant 0" if control else "unrouted"}; PLL={"HY01 CLKOP feedback/analog checked" if has_pll else "none"}; route/pack PASS'


def build(scratch):
    scratch = scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        raise ValueError('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=True)
    report = []
    designs = [(v, generate(v), '3.5' if v.endswith('35') else '2.0', v.startswith('sync'), True)
               for v in VARIANTS]
    for source in ('pin', 'pll'):
        for ratio in ('default', '2.0', '3.5'):
            params = '' if ratio == 'default' else f'#(.DIV("{ratio}"))'
            source_code = PLL.replace('observer', 'source') if source == 'pll' else 'wire source=clk;'
            v = f'module top(input clk, output q); {source_code} CLKDIVF {params} u(.CLKI(source),.RST(1\'b0),.ALIGNWD(1\'b0),.CDIVX(q)); endmodule\n'
            designs.append((f'probe_{source}_{ratio}', v, '2.0' if ratio == 'default' else ratio, False, False))
    for name, verilog, ratio, control, board in designs:
        d = scratch/name
        d.mkdir(exist_ok=True)
        (d/'top.v').write_text(verilog)
        lpf = (HERE/'hi12.lpf').read_text()
        if not board:
            lpf = lpf.replace('ftdi_rxd', 'q').replace('L4', 'B2')
        (d/'top.lpf').write_text(lpf)
        if board:
            shutil.copyfile(HERE/'route_x13.py', d/'route_x13.py')
        script = 'read_verilog top.v; synth_ecp5 -top top; '
        if board:
            script += 'setparam -set GSR "DISABLED" t:TRELLIS_FF; '
        script += 'write_json top.json'
        run([os.environ.get('YOSYS', 'yosys'), '-Q', '-q', '-p', script], d)
        run([os.environ.get('NEXTPNR_ECP5', 'nextpnr-ecp5'), '--85k', '--package', 'CABGA381', '--seed', '1', '--json', 'top.json', '--lpf', 'top.lpf', '--textcfg', 'top.config', '--write', 'routed.json', '--quiet'] + (['--pre-route', 'route_x13.py'] if board else []), d)
        run([os.environ.get('ECPPACK', 'ecppack'), 'top.config', 'top.bit'], d)
        line = check(d, ratio, control, board)
        print(line, flush=True)
        report.append(line)
    (scratch/'validation.txt').write_text('\n'.join(report)+'\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    build(p.parse_args().scratch)
