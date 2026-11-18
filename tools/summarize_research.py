#!/usr/bin/env python3
"""Index existing research and unresolved question IDs; do not mark them solved."""
import json
from pathlib import Path
import re

from inventory_trellis import sha256

ROOT=Path(__file__).resolve().parents[1]
INV=ROOT/'docs/inventory/LFE5U-85F'


def main():
    questions=[];sources=[Path(__file__),ROOT/'PLAN.md']
    sources += [ROOT/'docs'/n for n in ['RESEARCH_STATUS.md','IMPLEMENTATION_PLAN.md','CLOCK_NETWORK.md',
                                      'SYSTEM_STARTUP.md','PIO_BANKS.md','PACKAGE_JTAG.md','HARDWARE_VALIDATION.md','RESOURCE_COVERAGE.md']]
    for prefix,name,count in [('D','DSP_INTERNALS.md',10),('I','IOLOGIC_RESOURCES.md',10),('Q','DQS_CONTROL.md',8),
                              ('B','EBR_ADDRESSING.md',8),('L','PLC_LUTRAM.md',9),('S','FF_CCU2.md',6)]:
        path=ROOT/'docs'/name;text=path.read_text();found=[]
        for line in text.splitlines():
            m=re.match(r'^\|\s*('+prefix+r'\d{2})\s*\|\s*([^|]+)',line)
            if m:found.append({'id':m[1],'question':m[2].strip(),'source':'docs/'+name,'status':'open'})
        if [q['id'] for q in found]!=[f'{prefix}{i:02}' for i in range(1,count+1)]:
            raise ValueError('missing/duplicate question IDs in '+name)
        questions+=found;sources.append(path)
    for name,doc in [('clock_system_contract.json','CLOCK_NETWORK.md;SYSTEM_STARTUP.md'),('pio_bank_contract.json','PIO_BANKS.md')]:
        path=ROOT/'tools/data'/name;contract=json.loads(path.read_text());sources.append(path)
        for q in contract['open_questions']:
            questions.append({**q,'source':'tools/data/'+name,'documents':doc.split(';'),'status':'open'})
    if len(questions)!=78 or len({q['id'] for q in questions})!=78:
        raise ValueError('unexpected question census')
    cases=[]
    for name in ['ebr_control_contract.json','plc_contract.json','logic_state_contract.json','clock_system_contract.json','pio_bank_contract.json']:
        path=ROOT/'tools/data'/name;d=json.loads(path.read_text());sources.append(path)
        for case in d['hardware_cases']:
            if not set(case['questions'])<={q['id'] for q in questions}:raise ValueError('unknown case question')
            cases.append({'id':case['id'],'questions':case['questions'],'source':'tools/data/'+name,'status':'not_run'})
    if len(cases)!=37 or len({c['id'] for c in cases})!=37:
        raise ValueError('unexpected hardware series census')
    reports=[]
    for folder in sorted(INV.iterdir()):
        if folder.is_dir():
            for p in sorted(folder.glob('*report.json')):
                reports.append({'path':str(p.relative_to(ROOT)),'sha256':sha256(p),
                                'scope':'family_report;see_its_sources_and_toolchain_update_scope'})
    # Rows of the Results table in HARDWARE_VALIDATION.md: one per board observation.
    validation=(ROOT/'docs/HARDWARE_VALIDATION.md').read_text().split('\n## Results',1)
    results=0
    if len(validation)==2:
        results=sum(1 for line in validation[1].split('\n## ',1)[0].splitlines()
                    if re.match(r'^\|\s*\d{4}-\d{2}-\d{2}\s*\|',line))
    result={'schema_version':1,'device':'LFE5U-85F','date':'2026-10-22','scope':'primary_material_collection;not_complete_physical_reverse_engineering',
            'main_runtime_changed_in_this_pass':False,'unresolved_questions':len(questions),'questions':questions,
            'prepared_hardware_series':len(cases),'hardware_cases':cases,'hardware_results':results,
            'candidate_site_references':120,'unbound_service_site_references':1496,
            'latest_repeated_artifact_files':43,'latest_repeated_folders':['clock_system_research','package_system_research','pio_bank_research','system_metadata_research'],
            'family_reports':reports,'source_files':[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(set(sources))]}
    (INV/'research_manifest.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ('unresolved_questions','prepared_hardware_series','hardware_results','latest_repeated_artifact_files')}))


if __name__=='__main__':main()
