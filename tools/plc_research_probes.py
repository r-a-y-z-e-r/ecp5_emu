"""External tool/model experiments used by collect_plc_evidence; no emulator runtime."""
from collections import Counter
import json
from pathlib import Path
import subprocess


def run(command, cwd):
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True)
    if result.returncode: raise RuntimeError(f"{Path(command[0]).name}: {result.stderr[-5000:]}")
    return result


def probes(scratch, library, yosys, iverilog, vvp):
    scratch = scratch.resolve(); scratch.mkdir(parents=True, exist_ok=True)
    synthesis, models, initial, sources = [], [], [], []
    for width, depth, falling in ((1,16,False),(2,16,False),(4,16,False),(8,16,False),(4,32,False),(4,16,True)):
        name = f"ram{depth}x{width}{'_falling' if falling else ''}"
        abits = (depth-1).bit_length()
        source = scratch / f"{name}.v"
        source.write_text(f"""module top(input clk,we,input [{abits-1}:0] wa,ra,input [{width-1}:0] d,output [{width-1}:0] q);
(* ram_style="distributed" *) reg [{width-1}:0] mem[0:{depth-1}];
integer i; initial for(i=0;i<{depth};i=i+1) mem[i]=(i*5)^8'ha3;
always @({'negedge' if falling else 'posedge'} clk) if(we) mem[wa]<=d;
assign q=mem[ra];
endmodule
""")
        run([str(yosys), "-Q", "-T", "-q", "-p", f"read_verilog {name}.v; synth_ecp5 -top top -json {name}.json", "-l", f"{name}.log"], scratch)
        netlist = scratch / f"{name}.json"
        module = json.loads(netlist.read_text())["modules"]["top"]
        cells = Counter(c["type"] for c in module["cells"].values())
        count = cells["TRELLIS_DPR16X4"]
        if count != ((width+3)//4)*((depth+15)//16) or cells["DP16KD"]:
            raise ValueError(f"unexpected LUTRAM mapping in {name}: {cells}")
        for cell in module["cells"].values():
            if cell["type"] == "TRELLIS_DPR16X4":
                if cell["parameters"]["WCKMUX"] != ("INV" if falling else "WCK"):
                    raise ValueError("LUTRAM write clock polarity changed")
        synthesis.append([name,width,depth,"falling" if falling else "rising",count,json.dumps(dict(sorted(cells.items())),separators=(",",":")),"Yosys_structure_only;nextpnr_packing_not_executed"])
        sources += [(f"synthesis_source:{name}",source),(f"synthesis_netlist:{name}",netlist)]

    # Exercise the published software models, including their time-zero initialization.
    init_words = [(i*5+3)&15 for i in range(16)]
    init_value = sum(word << (4*i) for i,word in enumerate(init_words))
    combinations = [(polarity,enable) for polarity in ("WCK","INV") for enable in ("WRE","INV","0","1")]
    instances = [f'TRELLIS_DPR16X4 #(.INITVAL(64\'h{init_value:016x}),.WCKMUX("{pol}"),.WREMUX("{enable}")) m{i}(.DI(d),.WAD(wa),.RAD(ra),.WCK(clk),.WRE(we),.DO(q{i}));'
                 for i,(pol,enable) in enumerate(combinations)]
    instances.append(f'DPR16X4C #(.INITVAL("0x{init_value:016x}")) legacy(.DI(d),.WAD(wa),.RAD(ra),.WCK(clk),.WRE(we),.DO(q8));')
    tb = scratch / "lutram-models-tb.v"
    tb.write_text("""module tb;
reg clk=1'bx,we=1'bx;
reg [3:0] wa=0,ra=0,d=3;
""" + "\n".join(f"wire [3:0] q{i};" for i in range(9)) + "\n" + "\n".join(instances) + """
task scan(input integer step,input integer phase);
integer j; begin
for(j=0;j<16;j=j+1)begin ra=j;#1;
""" + '$display("%0d %0d %0d ' + " ".join(["%h"]*9) + '",step,phase,j,' + ",".join(f"q{i}" for i in range(9)) + ');\n' + """end end endtask
integer cycle;
initial begin
#1;we=0;#1;scan(-1,2);
// The X->0 bootstrap edge writes the existing value at address zero in constant-enable negative-clock models.
clk=0;#1;
for(cycle=0;cycle<512;cycle=cycle+1)begin
wa=cycle/32;d=(cycle/2)%16;we=cycle%2;#1;
clk=1;#1;scan(cycle,1);
clk=0;#1;scan(cycle,0);
end
$finish;end
endmodule
""")
    run([str(iverilog), "-g2012", "-DNO_INCLUDES", "-s", "tb", "-I", str(library), "-o", "lutram-models-sim", str(library/"common_sim.vh"), tb.name], scratch)
    result = run([str(vvp), "lutram-models-sim"],scratch)
    trace = scratch / "lutram-models.log"; trace.write_text(result.stdout)
    memory = [init_words[:] for _ in combinations]
    legacy = init_words[:]; legacy[15] = None
    seen = Counter(); previous = None
    for line in result.stdout.splitlines():
        values = line.split()
        if len(values) != 12: continue
        step,phase,address = map(int,values[:3])
        if (step,phase) != previous:
            if step >= 0:
                wa,di,we = step//32,(step//2)%16,step%2
                for i,(polarity,enable) in enumerate(combinations):
                    enabled = we if enable == "WRE" else 1-we if enable == "INV" else int(enable)
                    if phase == int(polarity=="WCK") and enabled: memory[i][wa] = di
                if phase==1 and we: legacy[wa] = di
            previous = step,phase
        for i,text in enumerate(values[3:]):
            expected = memory[i][address] if i < 8 else legacy[address]
            if text.lower() != ("x" if expected is None else f"{expected:x}"):
                raise ValueError(f"published model differs at step {step}, phase {phase}, address {address}, model {i}")
            seen[i] += 1
            if step == -1:
                initial.append(["TRELLIS_DPR16X4" if i < 8 else "DPR16X4C",i,address,init_words[address],text,
                                "legacy_software_initialization_hole" if expected is None else "matched_software_model"])
    if set(seen.values()) != {16400} or set(seen) != set(range(9)):
        raise ValueError("incomplete software model trace")
    for i,(polarity,enable) in enumerate(combinations):
        models.append(["TRELLIS_DPR16X4",i,polarity,enable,seen[i],"matched_published_model;not_hardware"])
    models.append(["DPR16X4C",8,"WCK","WRE",seen[8],"confirmed_software_hole_at_INIT_address_15;live_write_fills_it"])
    sources += [("model_testbench",tb),("model_trace",trace)]
    audit = scratch / "comb-model-audit-tb.v"
    audit.write_text('''module tb;
reg [3:0] ra=0;
reg clk=0,wre=0,wd=0;
wire f_init,f_mask,carry_f,carry_out;
TRELLIS_COMB #(.MODE("DPRAM"),.INITVAL(16'h0100)) init_probe
 (.A(ra[3]),.B(ra[1]),.C(ra[2]),.D(ra[0]),.WCK(clk),.WRE(1'b0),.WD(1'b0),
  .WAD0(1'b0),.WAD1(1'b0),.WAD2(1'b0),.WAD3(1'b0),.F(f_init));
TRELLIS_COMB #(.MODE("DPRAM"),.INITVAL(16'h0000),.WREMUX("0")) mask_probe
 (.A(1'b0),.B(1'b0),.C(1'b0),.D(1'b0),.WCK(clk),.WRE(wre),.WD(wd),
  .WAD0(1'b0),.WAD1(1'b0),.WAD2(1'b0),.WAD3(1'b0),.F(f_mask));
TRELLIS_COMB #(.MODE("CCU2"),.INITVAL(16'hffff),.CCU2_INJECT1("NO")) carry_probe
 (.A(1'b0),.B(1'b0),.C(1'b0),.D(1'b0),.FCI(1'b1),.F(carry_f),.FCO(carry_out));
initial begin
#1;ra=1;#1;$display("INIT_LOGICAL_1 %b",f_init);
ra=8;#1;$display("INIT_LOGICAL_8 %b",f_init);
wd=1;wre=1;#1;clk=1;#1;$display("WREMUX_ZERO_WRITE %b",f_mask);
$display("CARRY_OUTPUT %b",carry_out);
$finish;end
endmodule
''')
    run([str(iverilog),"-g2012","-DNO_INCLUDES","-s","tb","-I",str(library),"-o","comb-model-audit-sim",str(library/"common_sim.vh"),audit.name],scratch)
    audit_trace=scratch/"comb-model-audit.log"
    audit_result=run([str(vvp),"comb-model-audit-sim"],scratch).stdout;audit_trace.write_text(audit_result)
    observed={parts[0]:parts[1] for line in audit_result.splitlines() if len(parts:=line.split())==2 and parts[0] in {"INIT_LOGICAL_1","INIT_LOGICAL_8","WREMUX_ZERO_WRITE","CARRY_OUTPUT"}}
    expected={"INIT_LOGICAL_1":"0","INIT_LOGICAL_8":"1","WREMUX_ZERO_WRITE":"1","CARRY_OUTPUT":"z"}
    if observed != expected:raise ValueError(f"COMB software model changed: {observed}")
    limitations=[
        ["COMB_DPRAM_INIT_order","packed INIT=0x0100; logical RAD=1",observed['INIT_LOGICAL_1'],"model RAM read order differs from physical-LUT INIT order used by packer"],
        ["COMB_DPRAM_INIT_order","packed INIT=0x0100; logical RAD=8",observed['INIT_LOGICAL_8'],"do not use this model unchanged as a post-packing INIT oracle"],
        ["COMB_WREMUX_unused","MODE=DPRAM; WREMUX=0; raw WRE=1; write one",observed['WREMUX_ZERO_WRITE'],"apply effective enable externally or correct model; declared WREMUX is not used in this branch"],
        ["COMB_CCU2_FCO_shadow","MODE=CCU2; INIT=0xffff; INJECT1=NO; FCI=1",observed['CARRY_OUTPUT'],"generate-local FCO shadows the output; software-model issue, not a floating silicon carry"]]
    sources += [("comb_model_audit",audit),("comb_model_trace",audit_trace)]
    return synthesis,models,initial,limitations,sources
