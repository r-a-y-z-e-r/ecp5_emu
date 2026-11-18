#!/usr/bin/env python3
"""Collect clocks, PLL and system evidence; does not implement the emulator."""
import argparse
from collections import Counter,defaultdict
from copy import deepcopy
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re

from collect_plc_evidence import bits,read_csv
from clock_system_probes import probes
from inventory_trellis import sha256,verify_native,write_csv
from resource_audit import packed

ROOT=Path(__file__).resolve().parents[1]
DEVICE='LFE5U-85F'
FAMILIES={'PLL','DCC_MID','DCC_CENTRE','DCS','CLKDIVF','ECLKSYNCB','TRELLIS_ECLKBUF','ECLKBRIDGECS','BRGECLKSYNC',
          'DLLDELD','GSR','OSCG','JTAGG','SEDGA','DTR','USRMCLK'}
CLOCK_WIRE=re.compile(r'HPBX|HPRX|VPTX|PCLK|DCC|DCS|HPFE|HPFW|VPFN|VPFS|ECLK|CDIV|ECSOUT|CLKDIV|PLL')


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--inventory',type=Path,default=ROOT/'docs/inventory'/DEVICE)
    ap.add_argument('--output',type=Path)
    ap.add_argument('--probe-directory',type=Path,default=ROOT/'target/research/clock_system/probes')
    ap.add_argument('--trellis',type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    ap.add_argument('--nextpnr-source',type=Path, help="Source checkout (NEXTPNR_ROOT)")
    for name in ('yosys','nextpnr-ecp5','ecppll'):ap.add_argument('--'+name,type=Path,default=Path('/opt/homebrew/bin')/name)
    ap.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    args=resolve_toolchain(ap, ap.parse_args());folder=args.inventory;inv_path=folder/'device_inventory.json';inv=json.loads(inv_path.read_text())
    if inv['device']!=DEVICE:raise ValueError('wrong device')
    db=args.database;types=inv['tile_types']
    for source in inv['source_files']:
        if sha256(db/source['path'])!=source['sha256']:raise ValueError('stale inventory')
    verify_native(db,DEVICE,types)
    import pytrellis
    chip=pytrellis.Chip(DEVICE)
    tiles=read_csv(folder/'tile_instances.csv');tile_by_name={t['tile']:t for t in tiles}
    resources=[r for r in read_csv(folder/'resource_instances.csv') if r['family'] in FAMILIES]
    resource_by_id={r['resource_id']:r for r in resources}
    ports=[r for r in read_csv(folder/'resource_ports.csv') if r['resource_id'] in resource_by_id]
    globals_path=db/'ECP5'/DEVICE/'globals.json';g=json.loads(globals_path.read_text())
    regions=[];coords={}
    for row in range(96):
        for col in range(127):
            quad=[n for n,q in g['quadrants'].items() if q['x0']<=col<=q['x1'] and q['y0']<=row<=q['y1']]
            tap=[(int(n[1:]),'L' if t['lx0']<=col<=t['lx1'] else 'R') for n,t in g['taps'].items() if t['lx0']<=col<=t['rx1']]
            if len(quad)!=1 or len(tap)!=1:raise ValueError('overlapping global metadata')
            quad=quad[0];tap_col,side=tap[0];spine=g['spines'][f'{quad}{tap_col}']
            native_tap=chip.global_data.get_tap_driver(row,col);native_spine=chip.global_data.get_spine_driver(quad,tap_col)
            if (chip.global_data.get_quadrant(row,col),native_tap.col,int(native_tap.dir),native_spine.first,native_spine.second)!=(quad,tap_col,0 if side=='L' else 1,spine['y'],spine['x']):
                raise ValueError('native global metadata differs')
            value=[quad,tap_col,side,spine['y'],spine['x']];coords[row,col]=value
            regions.append([row,col,*value,'matched_native_global_data;not_a_passive_wire_union'])

    # Retain implicit aliases separately from configurable, directed buffers.
    aliases=set();routes=[]
    for t in tiles:
        row,col=int(t['fabric_row']),int(t['fabric_col']);desc=types[t['type']];quad,tap,side,sr,sc=coords[row,col]
        names=set(desc['muxes'])
        for m in desc['muxes'].values():names.update(o['source'] for o in m['sources'])
        for f in desc['fixed_connections']:names.update((f['source'],f['sink']))
        for name in names:
            m=re.fullmatch(r'G_(HPBX|VPTX|HPRX)(\d\d)00',name)
            if not m:continue
            kind,index=m[1],int(m[2])
            if kind=='HPBX':target=(row,tap,f'{side}_HPBX{index:02}00')
            elif kind=='VPTX':target=(sr,sc,name)
            else:target=(0,0,f'G_{quad}PCLK{index}')
            aliases.add((row,col,name,*target))
    route_types=set()
    for typ,desc in types.items():
        for sink,mux in desc['muxes'].items():
            for o in mux['sources']:
                if CLOCK_WIRE.search(sink) or CLOCK_WIRE.search(o['source']):
                    routes.append([typ,sink,o['source'],packed(bits(o['bits'])),f"{desc['source']}:{o['line']}"]);route_types.add(typ)
    spine_rows=[];tap_rows=[]
    for t in tiles:
        typ=t['type'];row,col=int(t['fabric_row']),int(t['fabric_col']);desc=types[typ]
        if 'SPINE' in typ:
            for sink,mux in desc['muxes'].items():
                if not re.fullmatch(r'G_VPTX\d\d00',sink):continue
                if len(mux['sources'])!=1:raise ValueError('SPINE selection changed')
                o=mux['sources'][0];spine_rows.append([t['tile'],row,col,o['source'],sink,packed(bits(o['bits'])),'directed_configurable_buffer'])
        if typ.startswith('TAP_DRIVE'):
            for sink,mux in desc['muxes'].items():
                if len(mux['sources'])!=1:raise ValueError('TAP selection changed')
                o=mux['sources'][0];tap_rows.append([t['tile'],row,col,o['source'],sink,packed(bits(o['bits'])),'directed_configurable_buffer'])
    if len(spine_rows)!=192 or len(tap_rows)!=18048:raise ValueError('unexpected primary distribution size')
    eligibility=[]
    for channel in range(16):
        sinks=sorted(sink for sink,m in types['PLC2']['muxes'].items() if any(o['source']==f'G_HPBX{channel:02}00' for o in m['sources']))
        eligibility.append([channel,int(any(s.startswith('CLK') for s in sinks)),int(any(s.startswith('LSR') for s in sinks)),
                            int(any(re.fullmatch('CE[0-3]',s) for s in sinks)),packed([s for s in sinks if not re.match(r'CLK|LSR|CE',s)]),packed(sinks)])

    catalog=json.loads((folder/'decode_catalog.json').read_text())
    selected={r['id']:deepcopy(r) for r in catalog['resources'] if r['id'] in resource_by_id}
    # PLL/DCC/DCS are not yet in the working runtime catalog. Collect their
    # configuration here, without silently changing decoder coverage.
    pll_names={cat:set(types['PLL0_UL'][cat])|set(types['PLL1_UL'][cat]) for cat in ('enums','words')}
    def add_field(resource,tile,name,category):
        desc=types[tile['type']];value=desc[category][name];kind='enum' if category=='enums' else 'word'
        def physical(tokens):return [[int(tile['start_frame'])+f,int(tile['start_bit'])+b,v] for f,b,v in bits(tokens)]
        fragment={'tile':tile['tile'],'raw_name':name,'source':f"{desc['source']}:{value['line']}",'default':value['default']}
        if kind=='enum':fragment['options']={o['value']:physical(o['bits']) for o in value['options']}
        else:fragment['word_bits']=[physical(group) for group in value['bit_groups']]
        target=selected.setdefault(resource['resource_id'],{'id':resource['resource_id'],'family':resource['family'],'fields':[]})
        field=next((f for f in target['fields'] if f['name']==name and f['kind']==kind),None)
        if field is None:field={'name':name,'kind':kind,'fragments':[]};target['fields'].append(field)
        field['fragments'].append(fragment)
    for r in resources:
        if r['family']=='PLL':
            quad=r['name'][-2:];primary=r['anchor_tile'];row,col=int(r['fabric_row']),int(r['fabric_col'])
            if quad in ('UL','UR'):companion=next(t for t in tiles if t['type']=='PLL1_'+quad)
            else:companion=next(t for t in tiles if int(t['fabric_row'])==95 and int(t['fabric_col'])==col+(1 if quad=='LL' else -1) and t['type'] in ('BANKREF8','BANKREF4','PLL1_LR'))
            for t in (tile_by_name[primary],companion):
                for category,names in pll_names.items():
                    for name in sorted(names & set(types[t['type']][category])):add_field(r,t,name,category)
        elif r['family']=='DCC_MID':
            raw='DCC_'+r['name'][0]+r['name'].split('DCC')[1]+'.MODE'
            for t in tiles:
                if raw in types[t['type']]['enums']:add_field(r,t,raw,'enums')
        elif r['family']=='DCS':
            raw=r['name']+'.DCSMODE'
            for t in tiles:
                if raw in types[t['type']]['enums']:add_field(r,t,raw,'enums')
    fields=[(r,f,fragment) for r in selected.values() for f in r['fields'] for fragment in f['fragments']]
    native_groups=0;native_seen=set();enum_rows=[];word_rows=[]
    # A source header line uniquely identifies the original raw field.
    for r,f,fragment in fields:
        t=tile_by_name[fragment['tile']];desc=types[t['type']];category='enums' if f['kind']=='enum' else 'words'
        raw=fragment.get('raw_name') or next(n for n,v in desc[category].items() if str(v['line'])==fragment['source'].rsplit(':',1)[1])
        marker=(t['type'],raw,category)
        if marker not in native_seen:
            native_seen.add(marker);bitdb=pytrellis.get_tile_bitdata(pytrellis.TileLocator('ECP5',DEVICE,t['type']))
            norm=lambda group:sorted((b.frame,b.bit,not b.inv) for b in group.bits)
            if category=='enums':
                native=bitdb.get_data_for_enum(raw)
                for o in desc[category][raw]['options']:
                    if norm(native.options[o['value']])!=bits(o['bits']):raise ValueError('native clock enum mismatch')
                    native_groups+=1
            else:
                native=bitdb.get_data_for_setword(raw)
                for index,group in enumerate(desc[category][raw]['bit_groups']):
                    if norm(native.bits[index])!=bits(group):raise ValueError('native clock word mismatch')
                    native_groups+=1
        if category=='enums':
            enum_rows.extend([r['id'],f['name'],fragment['tile'],v,packed(group),fragment['source']] for v,group in fragment['options'].items())
        else:
            word_rows.extend([r['id'],f['name'],fragment['tile'],i,packed(group),fragment['source']] for i,group in enumerate(fragment['word_bits']))
    # Verify all distribution buffer bit patterns against the native database too.
    checked_routes=set()
    for typ,sink,source,group,_ in routes:
        marker=(typ,sink,source)
        if marker in checked_routes:continue
        checked_routes.add(marker)
        bitdb=pytrellis.get_tile_bitdata(pytrellis.TileLocator('ECP5',DEVICE,typ))
        if sorted((b.frame,b.bit,not b.inv) for b in bitdb.get_mux_data_for_sink(sink).arcs[source].bits.bits)!=[tuple(b) for b in json.loads(group)]:raise ValueError('native clock route mismatch')

    osc=next(r for r in selected.values() if r['family']=='OSCG');div=next(f for f in osc['fields'] if f['name']=='OSC.DIV')
    osc_codes=defaultdict(list)
    for n in range(2,128):
        group=sorted({tuple(bit) for frag in div['fragments'] for bit in frag['options'][str(n)]})
        osc_codes[packed(group)].append(n)
    osc_rows=[[group,packed(values),'indistinguishable_raw_DIV_labels;physical_frequency_not_inferred'] for group,values in sorted(osc_codes.items())]
    dcs=[]
    for mode in ('POS','NEG','CLK0_LOW','CLK0_HIGH','CLK1_LOW','CLK1_HIGH','CLK0','CLK1','LOW','HIGH'):
        for modesel in (0,1):
            for sel in range(4):
                if modesel:result='CLK0' if sel==1 else 'CLK1' if sel==2 else '0'
                elif mode in ('POS','NEG'):result='CLK0' if sel==1 else 'CLK1' if sel==2 else ('1' if mode=='POS' else '0')
                elif mode in ('CLK0','CLK1','LOW','HIGH'):result={'LOW':'0','HIGH':'1'}.get(mode,mode)
                else:
                    active=1 if mode.startswith('CLK0') else 2
                    result=mode[:4] if sel==active else ('1' if mode.endswith('HIGH') else '0')
                dcs.append([mode,modesel,sel,result,'documented_steady_selection;transition_handshake_separate;sysCLOCK_p22'])
    probe_rows,extra=probes(args.probe_directory,args.yosys,getattr(args,'nextpnr_ecp5'),args.ecppll)
    source_paths=[('collector',Path(__file__)),('probe_helper',ROOT/'tools/clock_system_probes.py'),('inventory',inv_path),('base_bindings',folder/'resource_instances.csv'),
                  ('base_decode',folder/'decode_catalog.json'),('pytrellis',Path(pytrellis.__file__)),('globals',globals_path)]
    source_paths += [('trellis:'+n,args.trellis/n) for n in ['libtrellis/src/Chip.cpp','libtrellis/src/RoutingGraph.cpp','libtrellis/src/Bels.cpp','docs/architecture/global_routing.rst',
                    'fuzzers/ECP5/020-tap_drive/fuzzer.py','fuzzers/ECP5/021-spine/fuzzer.py','fuzzers/ECP5/026-dcc/fuzzer.py','fuzzers/ECP5/027-dcs/fuzzer.py','fuzzers/ECP5/091-pll_config/fuzzer.py','fuzzers/ECP5/103-gsr/fuzzer.py','fuzzers/ECP5/102-oscg/fuzzer.py']]
    source_paths += [('nextpnr:'+n,args.nextpnr_source/'ecp5'/n) for n in ('globals.cc','trellis_import.py','bitstream.cc','pack.cc')]
    source_paths += [('binary:'+n,getattr(args,n.replace('-','_')).resolve()) for n in ('yosys','nextpnr-ecp5','ecppll')]+extra
    contract_path=ROOT/'tools/data/clock_system_contract.json';contract=json.loads(contract_path.read_text());source_paths.append(('contract',contract_path))
    for name,entry in contract['sources'].items():source_paths.append((name,ROOT/entry['path']))
    output=args.output or folder/'clock_system_research';output.mkdir(parents=True,exist_ok=True)
    def table(name,columns,rows):write_csv(output/name,columns,rows)
    table('global_regions.csv',['row','col','quadrant','tap_col','tap_direction','spine_row','spine_col','evidence'],regions)
    table('implicit_aliases.csv',['raw_row','raw_col','raw_wire','segment_row','segment_col','segment_wire'],sorted(aliases))
    table('route_templates.csv',['tile_type','sink','source','local_constraints','evidence'],routes)
    table('spine_buffers.csv',['tile','row','col','source','sink','local_constraints','interpretation'],spine_rows)
    table('tap_buffers.csv',['tile','row','col','source','sink','local_constraints','interpretation'],tap_rows)
    table('PLC_global_eligibility.csv',['channel','CLK','LSR','CE','local_sinks','all_sinks'],eligibility)
    table('resources.csv',list(resources[0]),[[r[k] for k in resources[0]] for r in resources])
    table('ports.csv',list(ports[0]),[[r[k] for k in ports[0]] for r in ports])
    table('enum_fragments.csv',['resource','field','tile','value','CRAM_constraints','source'],enum_rows)
    table('word_fragments.csv',['resource','field','tile','logical_bit','CRAM_constraints','source'],word_rows)
    table('OSC_code_aliases.csv',['CRAM_pattern','DIV_labels','interpretation'],osc_rows)
    table('DCS_steady_selection.csv',['DCSMODE','MODESEL','SEL1_SEL0','output_source_or_level','evidence'],dcs)
    table('software_probes.csv',['case','field','expected_tool_result','observed_config','evidence'],probe_rows)
    report={'schema_version':1,'device':DEVICE,'native_global_coordinates':len(regions),'implicit_aliases':len(aliases),'spine_buffers':len(spine_rows),'tap_buffers':len(tap_rows),
            'resources':len(resources),'resource_counts':dict(sorted(Counter(r['family'] for r in resources).items())),'ports':len(ports),
            'config_resources':len(selected),'composed_fields':sum(len(r['fields']) for r in selected.values()),'fragments':len(fields),'native_field_bit_groups':native_groups,
            'native_route_bit_groups':len(checked_routes),'OSC_labels':126,'OSC_distinct_patterns':len(osc_rows),'DCS_steady_rows':len(dcs),'software_configuration_probes':len(probe_rows),
            'hardware':'not_run','runtime_changed':False,'source_files':[{'source':n,'sha256':sha256(p)} for n,p in source_paths]}
    for name,data in [('config_catalogue.json',{'device':DEVICE,'resources':list(selected.values()),'scope':'research_only;PLL_DCC_DCS_not_added_to_runtime_decoder'}),
                      ('clock_system_contract.json',contract),('evidence_report.json',report)]:
        (output/name).write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n')
    print(packed({k:v for k,v in report.items() if k!='source_files'}))


if __name__=='__main__':main()
