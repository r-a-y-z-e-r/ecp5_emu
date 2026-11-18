"""Clock configuration experiments; these never program a board or run a chip model."""
import json
from pathlib import Path
import subprocess


def run(command,cwd):
    p=subprocess.run([str(x) for x in command],cwd=cwd,text=True,capture_output=True)
    if p.returncode:raise RuntimeError(f"{Path(command[0]).name}: {p.stderr[-2500:]}")
    return p


def probes(scratch,yosys,nextpnr,ecppll):
    scratch=scratch.resolve();scratch.mkdir(parents=True,exist_ok=True)
    rows,sources=[],[]
    cases=[('dcs_parameter_NEG','DCSC #(.DCSMODE("NEG"))', 'POS'),
           ('dcs_attribute_NEG','(* DCSMODE="NEG" *) DCSC','NEG'),
           ('dcs_parameter_CLK0_LOW','DCSC #(.DCSMODE("CLK0_LOW"))','POS')]
    for name,cell,expected in cases:
        source=f'''module top(input c0,c1,s0,s1,mode,output q);
wire clk;{cell} d(.CLK0(c0),.CLK1(c1),.SEL0(s0),.SEL1(s1),.MODESEL(mode),.DCSOUT(clk));
reg state=0;always @(posedge clk) state<=~state;assign q=state;endmodule\n'''
        rows.append(build(scratch,name,source,yosys,nextpnr,sources,'DCSMODE',expected))
    for div in (2,3,127,128):
        name=f'osc_{div}'
        source=f'''module top(output q);wire clk;OSCG #(.DIV({div})) o(.OSC(clk));
reg state=0;always @(posedge clk) state<=~state;assign q=state;endmodule\n'''
        rows.append(build(scratch,name,source,yosys,nextpnr,sources,'OSC.DIV',str(min(div,127))))
    for name,extra in [('pll_external',[]),('pll_internal',['--internal_feedback'])]:
        p=run([ecppll,'-i','25','-o','100','--clkout1','50','--phase1','90','-n','pll','-f',f'{name}-pll.v',*extra],scratch)
        source=(scratch/f'{name}-pll.v').read_text()
        # Keep the generated clock outputs observable and consumed as clocks.
        source+='''\nmodule top(input clkin,output [1:0] q);wire clkout0,clkout1;
pll p(.clkin(clkin),.clkout0(clkout0),.clkout1(clkout1));
reg a=0,b=0;always @(posedge clkout0)a<=~a;always @(posedge clkout1)b<=~b;assign q={b,a};endmodule\n'''
        rows.append(build(scratch,name,source,yosys,nextpnr,sources,'PLL_divider_fields','from_synthesized_parameters'))
    return rows,sources


def build(scratch,name,source,yosys,nextpnr,sources,field,expected):
    path=scratch/f'{name}.v';path.write_text(source)
    run([yosys,'-Q','-T','-q','-p',f'read_verilog {name}.v; synth_ecp5 -top top -json {name}.json','-l',f'{name}-yosys.log'],scratch)
    p=run([nextpnr,'--85k','--package','CABGA381','--speed','6','--json',f'{name}.json','--textcfg',f'{name}.config','--no-route',
           '--lpf-allow-unconstrained','--seed','1','--timing-allow-fail','--log',f'{name}-nextpnr.log'],scratch)
    config=(scratch/f'{name}.config').read_text()
    observed={}
    for line in config.splitlines():
        tokens=line.split()
        if len(tokens)==3 and tokens[0] in ('word:','enum:'):
            if field=='DCSMODE' and tokens[1].endswith('.DCSMODE') or tokens[1]==field:
                observed[tokens[1]]=tokens[2]
    netlist=json.loads((scratch/f'{name}.json').read_text())['modules']['top']['cells']
    if field=='PLL_divider_fields':
        pll=next(c for c in netlist.values() if c['type']=='EHXPLLL')
        parameters=pll['parameters'];cfg=dict(line.split()[1:] for line in config.splitlines() if line.startswith('word: '))
        for key in ['CLKI_DIV','CLKFB_DIV','CLKOP_DIV','CLKOS_DIV','CLKOS2_DIV','CLKOS3_DIV']:
            default=1 if key in ('CLKI_DIV','CLKFB_DIV') else 8
            requested=int(parameters[key],2) if key in parameters else default
            if int(cfg[key],2)!=requested-1:raise ValueError(f'PLL encoding differs: {name}:{key}')
            observed[key]=int(cfg[key],2)
        for key in ['CLKOP_CPHASE','CLKOS_CPHASE','CLKOP_FPHASE','CLKOS_FPHASE']:
            requested=int(parameters[key],2) if key in parameters else 0
            if int(cfg[key],2)!=requested:raise ValueError('PLL phase encoding differs')
            observed[key]=int(cfg[key],2)
    elif not observed or set(observed.values())!={expected}:
        raise ValueError(f'configuration behavior changed: {name}:{observed}')
    sources.extend([(f'probe:{name}:{suffix}',scratch/f'{name}.{suffix}') for suffix in ('v','json','config')])
    return [name,field,expected,json.dumps(observed,sort_keys=True,separators=(',',':')),
            'actual_Yosys_and_nextpnr_post_placement_config;not_routed;not_hardware_behavior']
