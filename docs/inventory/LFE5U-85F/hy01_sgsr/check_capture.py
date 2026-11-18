"""Check generator capture/handshake/UART mux with Icarus behavioral fixtures.

Usage: python3 -B check_capture.py /private/tmp/hy01-build
The fixture IGNORES SGSR: this checks recorder plumbing, never SGSR hardware
semantics. UART divider is forced to zero to shorten simulation; 8N1 wire
decoding still checks start/data/stop and every byte including CR/LF.
All generated files and simulation outputs stay in external scratch.
"""
import argparse
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
from build import REPO
from hy01_gen import generate, LINE_BYTES
from hy01_analyse import parse, changes

FIXTURE = r'''
`timescale 1ns/1ps
module EHXPLLL #(
  parameter PLLRST_ENA="", INTFB_WAKE="", STDBY_ENABLE="", DPHASE_SOURCE="",
  OUTDIVIDER_MUXA="", OUTDIVIDER_MUXB="", OUTDIVIDER_MUXC="", OUTDIVIDER_MUXD="",
  CLKI_DIV=1, CLKFB_DIV=4, FEEDBK_PATH="", CLKOP_ENABLE="", CLKOP_DIV=6,
  CLKOP_CPHASE=2, CLKOP_FPHASE=0, CLKOS_ENABLE="", CLKOS_DIV=17,
  CLKOS_CPHASE=8, CLKOS_FPHASE=0
)(input CLKI, CLKFB, RST, STDBY, PHASESEL0, PHASESEL1, PHASEDIR, PHASESTEP,
  PHASELOADREG, PLLWAKESYNC, ENCLKOP,
  output reg CLKOP=0, CLKOS=0, LOCK=0);
  always #5 CLKOP=~CLKOP;
  always #14.166667 CLKOS=~CLKOS;
  initial #100 LOCK=1;
endmodule
module SGSR(input GSR, CLK);
  // Intentionally no reset semantics. Hardware is the measurement target.
endmodule
module TRELLIS_FF #(parameter GSR="ENABLED", REGSET="RESET", CLKMUX="CLK",
  CEMUX="1", LSRMUX="LSR", SRMODE="ASYNC")
  (input CLK, CE, LSR, DI, output reg Q);
  wire clock_value=(CLKMUX=="INV") ? ~CLK : CLK;
  initial Q=(REGSET=="SET");
  always @(posedge clock_value) Q<=DI;
endmodule
module tb;
  reg clk=0;
  wire tx;
  top dut(.clk_25mhz(clk), .ftdi_rxd(tx));
  always #20 clk=~clk;
  integer lines=0, i;
  reg [7:0] ch;
  initial begin
    force dut.baud=0;
    forever begin
      @(negedge tx);
      #20;
      if (tx!==0) $fatal(1,"bad start");
      for (i=0;i<8;i=i+1) begin #40; ch[i]=tx; end
      #40;
      if (tx!==1) $fatal(1,"bad stop");
      $write("%c",ch);
      if (ch==10) begin
        lines=lines+1;
        if (lines==@LINES@) $finish;
      end
    end
  end
  initial begin #50000000; $fatal(1,"timeout"); end
endmodule
'''


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    args = p.parse_args()
    scratch = args.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        p.error('scratch must be outside repository')
    scratch.mkdir(parents=True, exist_ok=True)
    for startup, count in ((False, 16), (True, 3)):
        name = 'hy01s' if startup else 'hy01'
        (scratch / f'{name}.v').write_text(generate(startup))
        (scratch / 'capture_tb.v').write_text(FIXTURE.replace('@LINES@', str(count)))
        subprocess.run(['iverilog', '-g2012', '-s', 'tb', '-o', f'{name}.sim',
                        f'{name}.v', 'capture_tb.v'], cwd=scratch, check=True)
        result = subprocess.run(['vvp', f'{name}.sim'], cwd=scratch,
                                check=True, capture_output=True, text=True, timeout=60)
        (scratch / f'{name}.simulation.log').write_text(result.stdout)
        raw = [x for x in result.stdout.splitlines() if x.startswith('Y1')]
        assert len(raw) == count, (name, len(raw))
        assert all(len(x) == LINE_BYTES-2 for x in raw)
        windows = [parse(x) for x in raw]
        assert [w.sequence for w in windows] == list(range(count))
        payloads = {tuple(w.words) for w in windows}
        assert len(payloads) == (1 if startup else count), (name, len(payloads))
        widths, phases = set(), set()
        for w in windows:
            n = w.signal(14)
            events = [t for t in changes(n) if t > 32]
            assert len(events) == 2 and [n[t] for t in events] == [0, 1], events
            widths.add(events[1]-events[0])
            # Marker 0 changes on each rising 25 MHz edge, phase relative to it.
            rises = changes(w.signal(11))
            phases.add(events[0]-max(t for t in rises if t <= events[0]))
            assert widths <= {4, 8, 12, 20, 80}, widths
        if not startup:
            assert widths == {4, 8, 12, 20, 80}, widths
            assert phases == {0, 1, 2, 3}, phases
        print(f'{name}: PASS {count} wire-decoded frames, checksum/length/sequence; '
              f'{len(payloads)} payloads; widths={sorted(widths)}, phases={sorted(phases)}')


if __name__ == '__main__':
    main()
