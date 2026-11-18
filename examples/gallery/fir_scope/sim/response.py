#!/usr/bin/env python3
"""Measures the frequency response of the demo's filter, from the RTL:
for each tone of the board's 'u'/'j' list it feeds the filters module
(filter.v + dsp_mac.v, with sim/dsp_models.v for the primitives) a pure
full-scale DDS tone at the demo's 2 kHz sample rate, and prints the
settled output amplitude over the input amplitude for both filters.
Writes nothing into the source folders; its files live in target/."""
import pathlib
import subprocess

folder = pathlib.Path(__file__).resolve().parents[1]
root = folder.parents[2]
out = root / "target/gallery" / folder.name / "response"
out.mkdir(parents=True, exist_ok=True)

TB = r"""
`timescale 1ns/1ps
module tb;
  reg clk=0; always #20 clk=~clk;
  reg reset=1, tick=0;
  reg signed [17:0] sample=0;
  wire valid;
  wire signed [17:0] pout, sout;
  wire [31:0] mism;
  filters f(clk,reset,tick,sample,valid,pout,sout,mism);
  reg signed [17:0] sine[0:255];
  initial $readmemh("sine.hex",sine);
  reg [31:0] phase=0, increment=0;
  integer tone=0, nsample=0, vin=0, vp=0, vs=0, a;
  always @(posedge clk) if(valid) begin
    // Settled amplitudes, after the 31-tap delay line holds only the tone.
    if(nsample>40) begin
      a=sample<0 ? -sample:sample; if(a>vin) vin=a;
      a=pout<0 ? -pout:pout; if(a>vp) vp=a;
      a=sout<0 ? -sout:sout; if(a>vs) vs=a;
    end
    nsample=nsample+1;
  end
  integer c;
  initial begin
    if(!$value$plusargs("tone=%d",tone)) tone=0;
    increment=32'd2147484<<tone;
    repeat(4) @(posedge clk); reset<=0;
    // Settle (the 31-tap delay line fills), then measure over two periods.
    for(c=0;c<40+(4000>>tone);c=c+1) begin
      // One 2 kHz tick, then 63 quiet clocks (the serial filter needs 31).
      @(negedge clk); tick=1; phase=phase+increment;
      sample=sine[phase[31:24]];
      @(negedge clk); tick=0;
      repeat(62) @(negedge clk);
    end
    $display("TONE %0d VIN %0d VP %0d VS %0d MISM %0d",tone,vin,vp,vs,mism);
    $finish;
  end
endmodule
"""

(out / "tb.v").write_text(TB)
sources = [folder / "filter.v", folder / "dsp_mac.v", folder / "sim" / "dsp_models.v", out / "tb.v"]
subprocess.run(["iverilog", "-g2012", "-o", str(out / "response.vvp"), *[str(s) for s in sources]],
               check=True, cwd=folder)
print(" tone    Hz    out/in (parallel)  out/in (serial, ALU54B)  mismatches")
for tone in range(8):
    run = subprocess.run(["vvp", str(out / "response.vvp"), f"+tone={tone}"],
                         check=True, capture_output=True, text=True, cwd=folder)
    fields = dict(zip(run.stdout.split()[::2], run.stdout.split()[1::2]))
    hz = (2147484 << tone) * 2000 / 2**32
    vin, vp, vs, mism = (int(fields[k]) for k in ("VIN", "VP", "VS", "MISM"))
    print(f"  {tone}   {hz:4.0f}       {vp / vin:6.3f}                {vs / vin:6.3f}               {mism}")
