"""Route and pack HC08 in external scratch; validate the actual written profiles."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
from gen import ANALOG, BASE_PROFILES, CLKOS3_DIV, PROFILES, generate

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def command(argv, cwd, log):
    with (cwd/log).open('w') as output:
        subprocess.run(argv, cwd=cwd, stdout=output, stderr=subprocess.STDOUT, check=True)


def check(directory, profile):
    feedback, analog, internal_return = PROFILES[profile]
    model = json.loads((directory/'routed.json').read_text())['modules']['top']
    plls = [c for c in model['cells'].values() if c['type'] == 'EHXPLLL']
    assert len(plls) == 1
    pll = plls[0]
    assert pll['parameters']['FEEDBK_PATH'].strip() == feedback
    ports = pll['connections']
    assert ports['CLKFB'] == ports['CLKINTFB' if internal_return else 'CLKOP']
    assert internal_return or not ports.get('CLKINTFB')
    text = (directory/'top.config').read_text()
    settings = re.findall(r'^(?:enum: FEEDBK_PATH|word: (?:'+'|'.join(ANALOG)+r')) \S+$', text, re.M)
    assert len(settings) == 5, settings
    assert re.findall(r'^enum: FEEDBK_PATH (\S+)$', text, re.M) == [feedback]
    words = {}
    for key, value in ANALOG.items():
        found = re.findall(rf'^word: {key} ([01]+)$', text, re.M)
        assert len(found) == 1
        words[key] = int(found[0], 2)
        assert words[key] == (value if analog else 0), (key, found)
    dividers = {}
    for key in ('CLKI_DIV', 'CLKFB_DIV', 'CLKOP_DIV', 'CLKOS_DIV', 'CLKOS2_DIV', 'CLKOS3_DIV'):
        found = re.findall(rf'^word: {key} ([01]+)$', text, re.M)
        assert len(found) == 1
        dividers[key] = dict(written=found[0], divider=int(found[0], 2)+1)
    assert dividers['CLKOS3_DIV']['divider'] == CLKOS3_DIV.get(profile, 8)
    # GSR cannot make the reporter depend on PLL lock or either clock starting.
    for cell in model['cells'].values():
        if cell['type'] == 'TRELLIS_FF':
            assert cell['parameters']['GSR'].strip() == 'DISABLED'
    return dict(profile=profile, feedback=feedback, words=words,
                return_net='CLKINTFB' if internal_return else 'CLKOP (fabric)',
                pll_bel=pll['attributes']['NEXTPNR_BEL'],
                config_lines=settings,
                feedback_arcs=re.findall(r'^arc: \S*CLKFB \S+$', text, re.M),
                dividers=dividers,
                bit_sha256=hashlib.sha256((directory/'top.bit').read_bytes()).hexdigest(),
                route_pack='PASS')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    p.add_argument('--profiles', nargs='+', choices=PROFILES, default=BASE_PROFILES)
    args = p.parse_args()
    scratch = args.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        p.error('scratch must be outside the repository')
    scratch.mkdir(parents=True, exist_ok=True)
    reports = []
    for profile in args.profiles:
        d = scratch/profile
        d.mkdir(exist_ok=False)
        (d/'top.v').write_text(generate(profile))
        shutil.copyfile(HERE/'reporter.v', d/'reporter.v')
        shutil.copyfile(HERE/'hc08.lpf', d/'top.lpf')
        command(['yosys', '-Q', '-q', '-p',
                 'read_verilog top.v reporter.v; synth_ecp5 -top top; '
                 'setparam -set GSR "DISABLED" t:TRELLIS_FF; write_json top.json'], d, 'yosys.log')
        command(['nextpnr-ecp5', '--85k', '--package', 'CABGA381', '--seed', '1',
                 '--json', 'top.json', '--lpf', 'top.lpf', '--textcfg', 'top.config',
                 '--write', 'routed.json'], d, 'nextpnr.log')
        command(['ecppack', 'top.config', 'top.bit'], d, 'ecppack.log')
        report = check(d, profile)
        reports.append(report)
        print(f'{profile}: '+', '.join(report['config_lines'])+'; route/pack PASS', flush=True)
    (scratch/'build_checks.json').write_text(json.dumps(reports, indent=2)+'\n')
    for tool, args in [('yosys', ['-V']), ('nextpnr-ecp5', ['--version']), ('ecppack', ['--version'])]:
        r = subprocess.run([tool]+args, capture_output=True, text=True)
        with (scratch/'tools.txt').open('a') as out:
            out.write(r.stdout+r.stderr)


if __name__ == '__main__':
    main()
