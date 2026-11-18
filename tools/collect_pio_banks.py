#!/usr/bin/env python3
"""Research all PIO/bank fields; leave the runtime and its decoder unchanged."""
import argparse
from collections import Counter, defaultdict
import csv
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import sys

from bind_trellis_sites import bind_config_fields
from collect_plc_evidence import bits
from inventory_trellis import sha256, write_csv
from resource_audit import audit_config, compile_decode_catalog, packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = 'LFE5U-85F'


def rows(path):
    with path.open(newline='') as stream:
        return list(csv.DictReader(stream))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--inventory', type=Path, default=ROOT/'docs/inventory'/DEVICE)
    ap.add_argument('--output', type=Path)
    ap.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    ap.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    ap.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    args = resolve_toolchain(ap, ap.parse_args())
    invpath = args.inventory/'device_inventory.json'
    inv = json.loads(invpath.read_text())
    db = args.database
    for source in inv['source_files']:
        if sha256(db/source['path']) != source['sha256']:
            raise ValueError('stale inventory: '+source['path'])
    grid = json.loads((db/'ECP5'/DEVICE/'tilegrid.json').read_text())
    resources = {r['resource_id']: r for r in rows(args.inventory/'resource_instances.csv') if r['family']=='PIO'}
    targets = {(r['tile'], r['site_name']): r['resource_id'] for r in rows(args.inventory/'site_bindings.csv')
               if r['resource_id'] in resources and r['relation']=='instance'}
    for tile, data in sorted(grid.items()):
        if data['type'].startswith('BANKREF'):
            bank = int(re.match(r'BANKREF(\d+)',data['type'])[1])
            rid = f'{DEVICE}/BANK{bank}'
            if rid in resources:
                raise ValueError('duplicate physical bank')
            resources[rid] = {'family':'BANK','name':f'BANK{bank}','anchor_tile':tile,'bank':bank}
            targets[tile,'BANK'] = rid
    fields, bitrefs = bind_config_fields(grid, inv, targets)
    unbound = [(tile,n) for tile,d in sorted(grid.items()) for cat in ('enums','words') for n in inv['tile_types'][d['type']][cat]
               if (n.startswith('PIO') or n.startswith('BANK.')) and (tile,n.split('.')[0]) not in targets]
    if unbound:
        raise ValueError('unbound PIO/bank fields: '+str(unbound[:4]))
    groups, aliases, shared, conflicts, words = audit_config(grid,inv,fields,bitrefs)
    catalog = compile_decode_catalog(grid,inv,fields,resources)
    sys.path.insert(0,'/opt/homebrew/lib/trellis')
    import pytrellis
    pytrellis.load_database(str(db))
    checked=set();native_groups=0
    for f in fields:
        typ=grid[f[2]]['type']; name=f[4]; marker=typ,f[3],name
        if marker in checked:
            continue
        checked.add(marker)
        bitdb=pytrellis.get_tile_bitdata(pytrellis.TileLocator('ECP5',DEVICE,typ))
        normalize=lambda g: sorted((b.frame,b.bit,not b.inv) for b in g.bits)
        desc=inv['tile_types'][typ]['enums' if f[3]=='enum' else 'words'][name]
        if f[3]=='enum':
            data=bitdb.get_data_for_enum(name)
            for o in desc['options']:
                if normalize(data.options[o['value']]) != bits(o['bits']):
                    raise ValueError('native enum differs')
                native_groups+=1
        else:
            data=bitdb.get_data_for_setword(name)
            for index,g in enumerate(desc['bit_groups']):
                if normalize(data.bits[index]) != bits(g):
                    raise ValueError('native word differs')
                native_groups+=1
    metadata={(m['row'],m['col'],m['pio']):m for m in inv['pio_metadata']}
    pios=[];counts=Counter()
    for rid,r in sorted(resources.items()):
        if r['family']!='PIO':continue
        row,col=int(r['fabric_row']),int(r['fabric_col']);letter=r['name'][-1]
        m=metadata[row,col,letter];bank=m['bank'];counts[bank]+=1
        pios.append([rid,bank,row,col,letter,m.get('function',''),m.get('dqs',''),
                     sum(f[1]==rid for f in fields),'GPIO_metadata;not_an_electrical_model'])
    owners=defaultdict(set)
    for b in bitrefs:owners[b[5],b[6]].add((b[1],b[0]))
    for rel in ['iologic_research/config_field_bits.csv','config_field_bits.csv']:
        for b in rows(args.inventory/rel):
            k=(int(b.get('global_frame',b.get('cram_frame'))),int(b.get('global_bit',b.get('cram_bit'))))
            if k in owners:owners[k].add((b['resource_id'],b['field_id']))
    external=[[f,b,packed(sorted({r for r,_ in v})),packed(sorted({fid for _,fid in v}))]
              for (f,b),v in sorted(owners.items()) if any(r not in resources for r,_ in v)]
    controls=[]
    for typ,d in sorted(inv['tile_types'].items()):
        for c in d['fixed_connections']:
            if re.search(r'BNK_(INRD|LVDS)|(?:INRD|LVDS)[A-D]_PIO',c['source']+' '+c['sink']):
                controls.append([typ,c['source'],c['sink'],'fixed_metadata;bank_endpoint_binding_open'])
    refs=[r for r in rows(args.inventory/'site_bindings.csv') if r['label_family'] in ('BCINRD','BCLVDSO')]
    banks=[[rid,r['bank'],r['anchor_tile'],counts[r['bank']],sum(f[1]==rid for f in fields),
            'documented_dynamic_bank_controls_binding_open' if r['bank'] in (2,3,6,7) else 'no_documented_dynamic_BCINRD_BCLVDSO']
           for rid,r in sorted(resources.items()) if r['family']=='BANK']
    source_paths=[('collector',Path(__file__)),('inventory',invpath),('pytrellis',Path(pytrellis.__file__)),
                  ('bindings',args.inventory/'site_bindings.csv'),('audit',ROOT/'tools/resource_audit.py'),
                  ('bind_fields',ROOT/'tools/bind_trellis_sites.py'),
                  ('sysIO',ROOT/'target/research/iologic/FPGA-TN-02032-1.4.pdf'),
                  ('power',ROOT/'target/research/clock_system/power-DTR.pdf'),
                  ('writer',args.nextpnr/'ecp5/bitstream.cc'),
                  ('bank_fuzzer',args.trellis/'fuzzers/ECP5/057-bankref/fuzzer.py')]
    contract_path=ROOT/'tools/data/pio_bank_contract.json'
    contract=json.loads(contract_path.read_text());source_paths.append(('contract',contract_path))
    out=args.output or args.inventory/'pio_bank_research';out.mkdir(parents=True,exist_ok=True)
    write_csv(out/'PIO.csv',['resource','bank','row','col','pio','function','dqs','field_fragments','scope'],pios)
    write_csv(out/'banks.csv',['resource','bank','tile','GPIO_count','field_fragments','dynamic_control_status'],banks)
    write_csv(out/'fields.csv',['field_id','resource_id','tile','kind','name','default','bit_count','source'],fields)
    write_csv(out/'bits.csv',['field_id','resource_id','tile','local_frame','local_bit','global_frame','global_bit'],bitrefs)
    write_csv(out/'field_groups.csv',['resource','kind','name','fragments','tiles','defaults','default_discrepancy','encoding_aliases'],groups)
    write_csv(out/'shared_bits.csv',['frame','bit','resources','fields'],shared)
    write_csv(out/'option_conflicts.csv',['resource','field','value','frame','bit'],conflicts)
    write_csv(out/'other_family_shared_bits.csv',['frame','bit','resources','fields'],external)
    write_csv(out/'bank_control_links.csv',['tile_type','source','sink','status'],controls)
    write_csv(out/'bank_control_sites.csv',list(refs[0]),[[r[k] for k in refs[0]] for r in refs])
    report={'schema_version':1,'device':DEVICE,'PIO':len(pios),'banks':len(banks),'PIO_per_bank':dict(sorted(counts.items())),
            'field_fragments':len(fields),'composed_fields':len(groups),'bit_references':len(bitrefs),'unique_cram_bits':len(owners),
            'encoding_alias_groups':sum(bool(json.loads(g[7])) for g in groups),'default_discrepancies':sum(g[6] for g in groups),
            'option_polarity_conflicts':len(conflicts),'native_setting_descriptors':len(checked),
            'native_encoding_groups':native_groups,'shared_bits_in_selection':len(shared),'shared_bits_with_other_families':len(external),
            'bank_control_fixed_templates':len(controls),'bank_control_unbound_references':len(refs),
            'runtime_changed':False,'hardware':'not_run','source_files':[{'source':n,'sha256':sha256(p)} for n,p in source_paths]}
    (out/'config_catalogue.json').write_text(json.dumps(catalog,ensure_ascii=False,separators=(',',':'))+'\n')
    (out/'pio_bank_contract.json').write_text(json.dumps(contract,ensure_ascii=False,indent=2)+'\n')
    (out/'evidence_report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(packed({k:v for k,v in report.items() if k!='source_files'}))


if __name__=='__main__':main()
