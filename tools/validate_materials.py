#!/usr/bin/env python3
"""Check the saved material pass against its independent repeated collectors.
See docs/RESEARCH_STATUS.md#why-material-validation-currently-fails for known drift.
"""
import csv
import hashlib
import json
from pathlib import Path
import re

ROOT=Path(__file__).resolve().parents[1]
INV=ROOT/'docs/inventory/LFE5U-85F'


def table(path):
    with path.open(newline='') as stream:return list(csv.DictReader(stream))


def main():
    checks={};total=0
    pairs={'clock_system_research':'repeated-clock','package_system_research':'repeated-package',
           'pio_bank_research':'repeated-PIO','system_metadata_research':'repeated-metadata'}
    for name,repeated in pairs.items():
        a=INV/name;b=ROOT/'target/research/clock_system'/repeated
        files=sorted(p.name for p in a.iterdir() if p.is_file())
        if files!=sorted(p.name for p in b.iterdir() if p.is_file()):raise ValueError('different file set: '+name)
        for n in files:
            p=a/n
            if p.read_bytes()!=(b/n).read_bytes():raise ValueError('not reproducible: '+str(p))
            if n.endswith('.json'):json.loads(p.read_text())
            elif n.endswith('.csv'):
                with p.open(newline='') as stream:rows=list(csv.reader(stream))
                if not rows or any(len(r)!=len(rows[0]) for r in rows):raise ValueError('ragged CSV: '+str(p))
        total+=len(files);checks[name]={'files':len(files),'byte_identical':True,'tables_parse':True}
    constraints=0
    for name in ['clock_system_research','pio_bank_research']:
        c=json.loads((INV/name/'config_catalogue.json').read_text())
        if len({r['id'] for r in c['resources']})!=len(c['resources']):raise ValueError('duplicate resource')
        for r in c['resources']:
            if len({(f['kind'],f['name']) for f in r['fields']})!=len(r['fields']):raise ValueError('duplicate field')
            for f in r['fields']:
                for frag in f['fragments']:
                    groups=frag['options'].values() if 'options' in frag else frag['word_bits']
                    for g in groups:
                        for frame,bit,polarity in g:
                            if not (0<=frame<13294 and 0<=bit<1136 and type(polarity)==bool):raise ValueError('bad CRAM constraint')
                            constraints+=1
    package=INV/'package_system_research'
    pins=table(package/'physical_pins.csv');gpio=table(package/'GPIO_crosscheck.csv');boundary=table(package/'boundary_cells.csv')
    if len({(r['package'],r['ball']) for r in pins})!=1976 or len(pins)!=1976:raise ValueError('invalid physical pins')
    counts={p:sum(r['package']==p for r in gpio) for p in sorted({r['package'] for r in gpio})}
    if counts!={'CABGA381':205,'CABGA554':259,'CABGA756':365,'CSFBGA285':118}:raise ValueError('invalid GPIO counts')
    if len(boundary)!=3000:raise ValueError('invalid boundary counts')
    for p in counts:
        if {int(r['index']) for r in boundary if r['package']==p}!=set(range(750)):raise ValueError('boundary index gap')
    manifest=json.loads((INV/'research_manifest.json').read_text())
    if len(manifest['questions'])!=78 or len(manifest['hardware_cases'])!=37:raise ValueError('invalid question/case census')
    for s in manifest['source_files']+manifest['family_reports']:
        if hashlib.sha256((ROOT/s['path']).read_bytes()).hexdigest()!=s['sha256']:raise ValueError('stale manifest: '+s['path'])
    broken=[];links=0
    for p in [ROOT/'PLAN.md',*sorted((ROOT/'docs').glob('*.md'))]:
        for url in re.findall(r'\[[^\]]*\]\(([^)]+)\)',p.read_text()):
            url=url.strip('<>').split('#')[0]
            if not url or '://' in url:continue
            links+=1
            if not (p.parent/url).exists():broken.append([str(p.relative_to(ROOT)),url])
    if broken:raise ValueError('broken document targets: '+str(broken))
    report={'date':'2026-10-22','scope':'latest_material_collection','checks':checks,'byte_identical_files':total,
            'new_bit_constraints_in_bounds':constraints,'document_file_links_checked':links,'broken_local_links':broken,
            'questions':78,'prepared_hardware_series':37,'hardware_results':0,'package_GPIO_counts':counts,'runtime_changed':False,
            'Rust_tests':'not_rerun_in_research_only_pass;prior_toolchain_results_in_TOOLCHAIN.md',
            'validator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    (INV/'material_validation.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['byte_identical_files','new_bit_constraints_in_bounds','document_file_links_checked','questions','prepared_hardware_series']}))


if __name__=='__main__':main()
