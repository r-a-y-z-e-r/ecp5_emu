#!/usr/bin/env python3
"""Collect official pinout/BSDL facts; never infer undocumented scan operations."""
import argparse
from collections import Counter
import csv,io,json,re
from pathlib import Path

from collect_plc_evidence import read_csv
from inventory_trellis import sha256,write_csv
from resource_audit import packed

ROOT=Path(__file__).resolve().parents[1]
PACKAGES={'CABGA381':'caBGA381','CABGA554':'caBGA554','CABGA756':'caBGA756','CSFBGA285':'csfBGA285'}


def strings(block):return ''.join(re.findall(r'"([^"\n]*)"',block))


def attribute(text,name):
    m=re.search(r'attribute\s+'+re.escape(name)+r'\s+of\s+\w+\s*:\s*entity\s+is\s+(.*?);',text,re.I|re.S)
    if not m:raise ValueError('missing BSDL attribute '+name)
    return m[1]


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--source-directory',type=Path,default=ROOT/'target/research/clock_system')
    ap.add_argument('--inventory',type=Path,default=ROOT/'docs/inventory/LFE5U-85F')
    ap.add_argument('--output',type=Path)
    args=ap.parse_args();src=args.source_directory
    raw=(src/'pinout-85.csv').read_text(encoding='utf-8-sig');start=raw.index('PAD,Pin/Ball Function')
    pad_rows=[r for r in csv.DictReader(io.StringIO(raw[start:])) if r['PAD'] and r['PAD'].isdigit()]
    official={};pad_table=[]
    for row in pad_rows:
        pad_table.append([row['PAD'],row['Pin/Ball Function'],row['Bank'],row['Dual Function'],row['Differential'],row['High Speed'],row['DQS']])
        for package in PACKAGES:
            value=(row.get(package) or '').strip()
            for ball in re.findall(r'\b[A-Z]+\d+\b',value):
                key=(package,ball)
                if key in official:raise ValueError('duplicate official package ball '+str(key))
                official[key]=row
    native={(r['package'],r['pin']):r for r in read_csv(args.inventory/'package_pins.csv')}
    native_matches=[]
    for key,row in native.items():
        public=official.get(key)
        if public is None:raise ValueError('native IO absent from public pinout '+str(key))
        fr,fc=int(row['fabric_row']),int(row['fabric_col'])
        label=('PL'+str(fr) if fc==0 else 'PR'+str(fr) if fc==126 else 'PT'+str(fc) if fr==0 else 'PB'+str(fc) if fr==95 else 'UNKNOWN')+row['pio']
        if public['Pin/Ball Function']!=label:raise ValueError(f'GPIO mismatch {key}: {label} != {public["Pin/Ball Function"]}')
        if public['Bank']!=row['bank']:raise ValueError('IO bank mismatch')
        native_matches.append([*key,label,row['fabric_row'],row['fabric_col'],row['pio'],row['bank'],public['Dual Function'],public['DQS'],'exact_GPIO_name_bank_and_ball_match'])

    pin_maps={};summary=[];cells=[];instructions=[];registers=[];sources=[('collector',Path(__file__)),('official_pinout',src/'pinout-85.csv'),('native_pins',args.inventory/'package_pins.csv')]
    for package,suffix in PACKAGES.items():
        path=src/f'LFE5U_85F_{suffix}.bsm';text=path.read_text();sources.append((package+':BSDL',path))
        clean=re.sub(r'--[^\n]*','',text)
        mapping=re.search(r'constant\s+\w+\s*:\s*PIN_MAP_STRING\s*:=\s*(.*?);',clean,re.I|re.S)
        if not mapping:raise ValueError('missing PIN_MAP')
        pintext=strings(mapping[1]);pinmap={}
        for m in re.finditer(r'(\w+)\s*:\s*(\([^)]*\)|[A-Z]+\d+)',pintext):
            for ball in re.findall(r'\b[A-Z]+\d+\b',m[2]):
                if ball in pinmap:raise ValueError('duplicate BSDL pin')
                pinmap[ball]=m[1]
        if len(pinmap)!=int(re.search(r'\d+',package)[0]):raise ValueError('BSDL physical pin count differs')
        pin_maps[package]=pinmap
        ir=int(attribute(clean,'INSTRUCTION_LENGTH'));length=int(attribute(clean,'BOUNDARY_LENGTH'))
        idbits=strings(attribute(clean,'IDCODE_REGISTER'))
        if len(idbits)!=32 or int(idbits,2)!=0x41113043:raise ValueError('BSDL IDCODE differs from device')
        boundary=strings(attribute(clean,'BOUNDARY_REGISTER'));seen=set();control_refs=[]
        package_cells={}
        for m in re.finditer(r'(\d+)\s*\(([^)]+)\)',boundary):
            index=int(m[1]);data=[x.strip() for x in m[2].split(',')]
            if index in seen or len(data) not in (4,7):raise ValueError('bad boundary cell')
            seen.add(index);package_cells[index]=data
            control=int(data[4]) if len(data)==7 else None
            if control is not None:control_refs.append(control)
            cells.append([package,index,*data[:4],control if control is not None else '',data[5] if len(data)==7 else '',data[6] if len(data)==7 else '',
                          'preliminary_generic_BSDL;application_specific_modes_may_change_scan_behavior'])
        if seen!=set(range(length)):raise ValueError('incomplete boundary chain')
        if any(c not in package_cells or package_cells[c][2].lower()!='control' for c in control_refs):raise ValueError('invalid BSDL control reference')
        opcode=strings(attribute(clean,'INSTRUCTION_OPCODE'));opcodes=[]
        for m in re.finditer(r'(\w+)\s*\(([^)]+)\)',opcode):
            for value in [x.strip() for x in m[2].split(',')]:
                if len(value)!=ir or not re.fullmatch('[01]+',value):raise ValueError('bad instruction width')
                instructions.append([package,m[1],value,f'0x{int(value,2):02x}','listed_private_without_semantics' if m[1]=='PRIVATE' else 'BSDL_documented_name'])
                opcodes.append((m[1],value))
        registers.append({'package':package,'register_access':strings(attribute(clean,'REGISTER_ACCESS')),
                          'instruction_capture':strings(attribute(clean,'INSTRUCTION_CAPTURE')),
                          'usercode_reset_bits':strings(attribute(clean,'USERCODE_REGISTER')),
                          'source_warning':'preliminary;not_verified_on_silicon;generic_bidirectional_IO_assumption;LVDS_VREF_output_only_require_application_specific_BSDL'})
        summary.append([package,len(pinmap),ir,length,f'0x{int(idbits,2):08x}',sum(n!='PRIVATE' for n,_ in opcodes),sum(n=='PRIVATE' for n,_ in opcodes),
                        'official_archive_1.2;contained_file_1.13_2016;not_verified_on_silicon'])
    pin_rows=[];differences=[]
    for package,pinmap in pin_maps.items():
        for ball,bsdl_function in sorted(pinmap.items()):
            p=official.get((package,ball));n=native.get((package,ball))
            if p is None:
                differences.append([package,ball,bsdl_function,'absent_from_PAD_csv','BSDL_kept;do_not_invent_die_pad'])
            elif n is not None and bsdl_function!=p['Pin/Ball Function']:
                raise ValueError('BSDL GPIO name differs from pinout')
            pin_rows.append([package,ball,bsdl_function,p['PAD'] if p else '',p['Pin/Ball Function'] if p else '',p['Dual Function'] if p else '',
                             p['Bank'] if p else '', 'GPIO' if n else 'dedicated_power_ground_or_NC', 'native_IO_verified' if n else 'public_package_metadata_only'])
    out=args.output or args.inventory/'package_system_research';out.mkdir(parents=True,exist_ok=True)
    write_csv(out/'package_summary.csv',['package','physical_balls','instruction_bits','boundary_cells','IDCODE','named_opcode_entries','private_opcode_entries','status'],summary)
    write_csv(out/'physical_pins.csv',['package','ball','BSDL_function','PAD','pinout_function','dual_function','bank','kind','evidence'],pin_rows)
    write_csv(out/'die_pads.csv',['PAD','function','bank','dual_function','differential','high_speed','DQS'],pad_table)
    write_csv(out/'GPIO_crosscheck.csv',['package','ball','function','row','col','pio','bank','dual_function','DQS','evidence'],native_matches)
    write_csv(out/'boundary_cells.csv',['package','index','cell_type','port','function','safe','control_cell','disable_value','disable_result','status'],cells)
    write_csv(out/'instructions.csv',['package','instruction','opcode_MSB_first','hex','status'],instructions)
    write_csv(out/'pinout_gaps.csv',['package','ball','BSDL_function','pinout_status','interpretation'],differences)
    report={'schema_version':1,'device':'LFE5U-85F','packages':4,'physical_balls':len(pin_rows),'official_PAD_rows':len(pad_rows),'GPIO_matches':len(native_matches),
            'boundary_cells':len(cells),'boundary_cells_per_package':sorted({r[3] for r in summary}),'BSDL_IDCODE':'0x41113043',
            'private_opcodes_are_not_implemented':True,'pinout_gaps':len(differences),'hardware':'not_run','runtime_changed':False,
            'sources':{'pinout_url':'https://www.latticesemi.com/view_document?document_id=50487','BSDL_archive_url':'https://www.latticesemi.com/view_document?document_id=54703'},
            'source_files':[{'source':n,'sha256':sha256(p)} for n,p in sources]}
    for name,data in [('registers.json',registers),('evidence_report.json',report)]:
        (out/name).write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n')
    print(packed({k:v for k,v in report.items() if k!='source_files'}))


if __name__=='__main__':main()
