#!/usr/bin/env python3
"""Audit the pinned EBR base/WEAMUX overlap; no hardware behavior is inferred."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def coordinates(field):
    return {(f, b) for part in field['fragments']
            for group in part.get('options', {}).values() for f, b, _ in group}


def mode_pattern(field, value):
    result = {}
    for part in field['fragments']:
        assert set(part['options']) == {'NONE', 'DP16KD', 'PDPW16KD'}
        for f, b, v in part['options'][value]:
            assert (f, b) not in result or result[f, b] == v
            result[f, b] = v
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--trellis', type=Path, default=Path.home() / 'FPGA/prjtrellis')
    parser.add_argument('--nextpnr', type=Path, default=Path.home() / 'FPGA/nextpnr')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/ebr-mode-overlay-audit.json')
    args = parser.parse_args()
    path = ROOT / 'docs/inventory/LFE5U-85F/decode_catalog.json'
    catalog = json.loads(path.read_text())
    resources = []
    invalid_count = 0
    fragment_counts = Counter()
    for resource in catalog['resources']:
        if resource['family'] != 'EBR':
            continue
        fields = {f['name'].split('.', 1)[1]: f for f in resource['fields']}
        mode = fields['MODE']
        all_bits = coordinates(mode)
        owners = {name: sorted(all_bits & coordinates(field))
                  for name, field in fields.items() if name != 'MODE'}
        # Words need independent inspection: a new WID overlap must fail too.
        for name, field in fields.items():
            if field['kind'] == 'word':
                owners[name] = sorted(all_bits & {(f, b) for part in field['fragments']
                                                for group in part['word_bits'] for f, b, _ in group})
        owners = {k: v for k, v in owners.items() if v}
        assert set(owners) == {'WEAMUX'} and len(owners['WEAMUX']) == 1
        shared = tuple(owners['WEAMUX'][0])
        selector = fields['WEAMUX']['fragments']
        assert len(selector) == 1
        assert selector[0]['options'] == {'WEA': [[*shared, False]], 'INV': [[*shared, True]]}
        patterns = {name: mode_pattern(mode, name) for name in ['NONE', 'DP16KD', 'PDPW16KD']}
        assert all(set(pattern) == all_bits for pattern in patterns.values())
        assert [patterns[m][shared] for m in patterns] == [False, False, True]
        legal = []
        for name, inverted in [('NONE', False), ('NONE', True), ('DP16KD', False),
                               ('DP16KD', True), ('PDPW16KD', False)]:
            observed = dict(patterns[name])
            if inverted:
                observed[shared] = True
            strict = [n for n, p in patterns.items() if p == observed]
            assert strict == ([] if inverted else [name])
            qualified = [n for n, p in patterns.items()
                         if n != 'PDPW16KD' and all(p[k] == observed[k] for k in p if k != shared)]
            assert not inverted or qualified == [name]
            legal.append(observed)
        inverted_dp = dict(patterns['DP16KD'])
        inverted_dp[shared] = True
        rejected = 0
        for bit in all_bits - {shared}:
            corrupt = dict(inverted_dp)
            corrupt[bit] = not corrupt[bit]
            assert corrupt not in legal
            rejected += 1
        invalid_count += rejected
        fragment_counts[len(mode['fragments'])] += 1
        resources.append({'resource': resource['id'], 'mode_field': mode['name'],
                          'mode_fragments': len(mode['fragments']), 'mode_bits': len(all_bits),
                          'overlap_field': fields['WEAMUX']['name'], 'overlap': list(shared),
                          'valid_cases': 5, 'rejected_non_overlay_bit_flips': rejected})
    assert len(resources) == 208
    sources = [path, Path(__file__), args.trellis / 'libtrellis/src/BitDatabase.cpp',
               args.trellis / 'fuzzers/ECP5/043-cib_constmux/fuzzer.py',
               args.trellis / 'fuzzers/ECP5/043-cib_constmux/cibconst.ncl',
               args.nextpnr / 'ecp5/bitstream.cc']
    report = {'schema_version': 1, 'date': '2026-10-22', 'hardware_results': 0,
              'scope': 'pinned configuration encodings and writer/fuzzer semantics; no silicon measurements',
              'resource_count': len(resources), 'mode_fragment_counts': dict(fragment_counts),
              'valid_encoding_cases': len(resources) * 5, 'rejected_bit_flips': invalid_count,
              'rule': 'retain all MODE fragments; DP/NONE may override exactly the WEAMUX bit; PDP must match strictly',
              'cib_constant_rule': 'CLK/LSR label 0 comes from the EBR pin-level fuzzer; nextpnr tie_cib_signal drives raw high, then inverts the consuming input',
              'resources': resources,
              'source_files': [{'path': str(p.relative_to(ROOT) if p.is_relative_to(ROOT) else p),
                                'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in sources]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ['resource_count', 'mode_fragment_counts',
                                          'valid_encoding_cases', 'rejected_bit_flips']}))


if __name__ == '__main__':
    main()
