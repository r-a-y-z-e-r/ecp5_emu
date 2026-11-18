"""Audit PLL selector overlap against Trellis and actual packed CRAM, offline."""
import argparse
import hashlib
import importlib
import json
from pathlib import Path
import re
import sys


def section(text, header):
    lines = text.splitlines()
    start = next(i for i, s in enumerate(lines) if s.startswith(header))
    stop = next((i for i in range(start+1, len(lines))
                 if not lines[i].strip() or lines[i].startswith('.')), len(lines))
    return lines[start:stop]


def entries(lines):
    return {line.split()[0]: set(line.split()[1:]) - {'-'} for line in lines[1:]}


def audit(args):
    sys.path.insert(0, str(args.pytrellis_dir))
    trellis = importlib.import_module('pytrellis')
    trellis.load_database(str(args.db))
    dbfile = args.db/'ECP5/tiledata/PLL0_UL/bits.db'
    text = dbfile.read_text()
    enum_lines = section(text, '.config_enum FEEDBK_PATH ')
    mux_lines = section(text, '.mux E1_CLKFB')
    enums, mux = entries(enum_lines), entries(mux_lines)
    mask = set.union(*enums.values())
    assert mask == {f'F{i}B1' for i in range(1, 7)}
    reports = []
    for profile in args.profiles:
        d = args.build/profile
        config = (d/'top.config').read_text()
        selector, = re.findall(r'^enum: FEEDBK_PATH (\S+)$', config, re.M)
        arc, = re.findall(r'^arc: E1_CLKFB (\S+)$', config, re.M)
        union = enums[selector] | mux[arc]
        chip = trellis.Bitstream.read_bit(str(d/'top.bit')).deserialise_chip()
        tile = chip.get_tile_by_name('MIB_R4C0:PLL0_UL')
        packed = {bit for bit in mask if tile.cram.bit(int(bit[1:bit.index('B')]), 1)}
        assert packed == union, (profile, packed, union)
        config_chip = trellis.ChipConfig.from_string(config).to_chip()
        config_tile = config_chip.get_tile_by_name('MIB_R4C0:PLL0_UL')
        assert packed == {bit for bit in mask
                          if config_tile.cram.bit(int(bit[1:bit.index('B')]), 1)}
        dividers = {}
        for key in ('CLKI_DIV', 'CLKFB_DIV', 'CLKOP_DIV', 'CLKOS_DIV', 'CLKOS2_DIV', 'CLKOS3_DIV'):
            value, = re.findall(rf'^word: {key} ([01]+)$', config, re.M)
            dividers[key] = dict(written=value, divider=int(value, 2)+1)
        reports.append(dict(profile=profile, source_selector=selector, source_arc=arc,
                            selector_ones=sorted(enums[selector]), arc_ones=sorted(mux[arc]),
                            union_ones=sorted(union), union_zeroes=sorted(mask-union),
                            exact_enum_matches=[name for name, bits in enums.items() if bits == union],
                            packed_equals_union=True, dividers=dividers,
                            config_sha256=hashlib.sha256((d/'top.config').read_bytes()).hexdigest(),
                            bit_sha256=hashlib.sha256((d/'top.bit').read_bytes()).hexdigest()))
    return dict(database_file='ECP5/tiledata/PLL0_UL/bits.db',
                database_sha256=hashlib.sha256(dbfile.read_bytes()).hexdigest(),
                enum_lines=enum_lines, mux_lines=mux_lines, mask=sorted(mask),
                all_internal_plus_JCLKFB3=[dict(selector=name, union_ones=sorted(bits|mux['E1_JCLKFB3']),
                    exact_enum_matches=[other for other, pattern in enums.items()
                                        if pattern == bits|mux['E1_JCLKFB3']])
                    for name, bits in enums.items() if name.startswith('INT_')],
                profiles=reports)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('build', type=Path)
    p.add_argument('--db', type=Path, required=True)
    p.add_argument('--pytrellis-dir', type=Path, default=Path('/opt/homebrew/lib/trellis'))
    p.add_argument('--profiles', nargs='+', default=list('CDFGH'))
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    result = audit(args)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    for report in result['profiles']:
        print(f"{report['profile']}: {report['source_selector']} | {report['source_arc']}: "
              f"{report['union_ones']}; exact enum matches={report['exact_enum_matches']}; packed CRAM agrees")
