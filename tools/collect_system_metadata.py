#!/usr/bin/env python3
"""Collect command retention and native text-config loss evidence, research only."""
import argparse
import csv
import io
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import subprocess

from inventory_trellis import sha256, write_csv
from resource_audit import packed

ROOT=Path(__file__).resolve().parents[1]


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--output',type=Path,default=ROOT/'docs/inventory/LFE5U-85F/system_metadata_research')
    ap.add_argument('--native-probe',type=Path,default=ROOT/'target/trellis_metadata_probe')
    ap.add_argument('--bit',type=Path,default=ROOT/'target/toolchain-update/smoke/top.bit')
    ap.add_argument('--trellis',type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    args=resolve_toolchain(ap, ap.parse_args());base=args.trellis/'libtrellis'
    result=subprocess.run([str(args.native_probe),str(args.trellis/'database'),str(args.bit)],text=True,capture_output=True,check=True)
    native=list(csv.DictReader(io.StringIO(result.stdout)))
    expected={'ctrl0':('0x11223344','0x40000000'),'ctrl1':('0x55667788','0x45000000'),
              'sed_reference':('0x89abcdef','0xffffffff'),'usercode':('0x13579bdf','0x00000000')}
    if {r['field']:(r['native_Chip'],r['after_text_config']) for r in native}!=expected:
        raise ValueError('native metadata retention changed')
    header=(base/'include/Bitstream.hpp').read_text()
    parser=(base/'src/Bitstream.cpp').read_text().split('Bitstream Bitstream::generate_jump',1)[0]
    enum=header.split('enum class BitstreamCommand',1)[1].split('};',1)[0]
    handling={'LSC_PROG_CNTRL0':'Chip.ctrl0','LSC_PROG_CNTRL1':'Chip.ctrl1','LSC_PROG_SED_CRC':'Chip.sed',
              'ISC_PROGRAM_USERCODE':'Chip.usercode','ISC_PROGRAM_USERCODE_2':'other_family_Chip.usercode',
              'LSC_PROG_CNTRL0_2':'other_family_control',
              'ISC_PROGRAM_SECURITY':'consumed_without_security_state',
              'ISC_PROGRAM_DONE':'CRC_checked_if_requested;no_controller_state',
              'ISC_PROGRAM_DONE_2':'other_family_done',
              'JUMP':'payload_skipped_TODO_address_speed',
              'VERIFY_ID':'create_Chip_and_select_device',
              'LSC_EBR_ADDRESS':'transient_EBR_cursor','LSC_EBR_WRITE':'Chip.bram_data',
              'LSC_PROG_INCR_CMP':'decompressed_CRAM;optional_CRC16','LSC_PROG_INCR_RTI':'CRAM;optional_CRC16',
              'LSC_INIT_ADDRESS':'transient_frame_cursor','LSC_WRITE_ADDRESS':'transient_frame_cursor',
              'LSC_RESET_CRC':'parser_CRC16_reset','LSC_WRITE_COMP_DIC':'transient_dictionary_and_SPI_mode',
              'DUMMY':'ignored_padding','SPI_MODE':'transient_SPI_transport',
              'WRITE_INC_FRAME':'other_family_CRAM'}
    commands=[]
    for name,binary in re.findall(r'(\w+)\s*=\s*0b([01]+)',enum):
        if name not in handling:raise ValueError('unreviewed command: '+name)
        state=handling[name] if 'case BitstreamCommand::'+name+':' in parser else 'declared_but_unsupported_deserializer_command'
        commands.append([name,f'0x{int(binary,2):02x}',state,'source_audit;not_full_transport_emulation'])
    wake=[]
    for seq,done in [(4,'ON'),(21,'OFF')]:
        for phase in range(4):
            signals={1:'GOE',2:'GWDIS;GSR'}
            if (seq==4 and phase==0) or (seq==21 and phase==3):signals[phase]='DONE'
            wake.append([seq,done,'T'+str(phase),signals.get(phase,''),'sysCONFIG_2.5_p49;phase_edge_relation_unspecified'])
    retention=[['CRAM','Chip.cram','tile/tilegroup encoding','exported','exact_configuration_bits'],
               ['EBR init','Chip.bram_data','.bram_init','exported','not_runtime_RAM_contents'],
               ['IDCODE','Chip.info','device/variant identification','device_name_only','original_verify_commands_not_retained'],
               ['ctrl0','Chip.ctrl0','not_serialized','not_exported','Python_Chip_field_not_bound'],
               ['ctrl1','Chip.ctrl1','not_serialized','not_exported','Python_Chip_field_not_bound'],
               ['SED reference','Chip.sed','not_serialized','not_exported','Python_Chip_field_not_bound'],
               ['USERCODE','Chip.usercode','not_serialized','not_exported','bound_in_Python_but_bridge_omits'],
               ['security','not_retained','not_serialized','not_exported','command_consumed'],
               ['command sequence/timing','not_retained_as_controller_state','not_serialized','not_exported','preserve_original_bitstream'],
               ['metadata comments','Chip.metadata','.comment','not_in_CRAM_JSON','not_equivalent_to_control_registers'],
               ['sysconfig directives','ChipConfig.sysconfig','parsed/text_serialized','not_applied_by_to_chip','nextpnr_writer_also_sets_relevant_CRAM_fields']]
    sources=[('collector',Path(__file__)),('native_probe_source',ROOT/'tools/trellis_metadata_probe.cpp'),('native_probe_binary',args.native_probe),('input_bit',args.bit),
             ('libtrellis',Path('/opt/homebrew/lib/trellis/libtrellis.dylib')),('bridge',ROOT/'tools/trellis_bridge.py')]
    sources += [(n,base/n) for n in ('src/Bitstream.cpp','src/ChipConfig.cpp','src/PyTrellis.cpp','include/Bitstream.hpp','include/Chip.hpp')]
    sources.append(('sysCONFIG',ROOT/'target/research/clock_system/sysCONFIG.pdf'))
    out=args.output;out.mkdir(parents=True,exist_ok=True)
    write_csv(out/'native_metadata_loss.csv',list(native[0]),[[r[k] for k in native[0]] for r in native])
    write_csv(out/'commands.csv',['command','opcode','deserializer_handling','scope'],commands)
    write_csv(out/'retention.csv',['data','native','text_config','runtime_bridge','scope'],retention)
    write_csv(out/'wake_sequences.csv',['sequence','DONE_EX','phase','signals_released','source'],wake)
    report={'device':'LFE5U-85F','command_descriptors':len(commands),'native_sentinels_checked':4,'text_config_losses':4,
            'wake_phase_rows':len(wake),'synthetic_tail':'parser_only;not_a_hardware_programming_stream',
            'hardware':'not_run','runtime_changed':False,'source_files':[{'source':n,'sha256':sha256(p)} for n,p in sources]}
    (out/'evidence_report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(packed({k:v for k,v in report.items() if k!='source_files'}))


if __name__=='__main__':main()
