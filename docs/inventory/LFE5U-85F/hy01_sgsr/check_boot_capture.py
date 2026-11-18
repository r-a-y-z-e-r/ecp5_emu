"""Check b/c snapshots, modulo differences, frozen replay and UART 8N1 in scratch.

Usage: python3 -B check_boot_capture.py /private/tmp/hy01-boot-build
Icarus fixture only: b's enabled counters are artificially held for two edges,
c's are not. This checks transport; hy01b/c board records establish startup state.
The delay and baud divider are accelerated. No board or serial port is opened.
"""
import argparse
from functools import reduce
import operator
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
from build import REPO
from check_capture import FIXTURE
from hy01_boot_gen import generate

TB = r'''
module tb;
  reg clk=0, hold=@HOLD@;
  wire tx;
  top dut(.clk_25mhz(clk), .ftdi_rxd(tx));
  always #20 clk=~clk;
  initial begin repeat(2) @(negedge clk); #1; hold=0; end
  integer i, lines=0;
  reg [7:0] ch;
  initial begin
    force dut.baud=0;
    #205; force dut.delay=20'hfffff;
    forever begin
      @(negedge tx); #20;
      if (tx!==0) $fatal(1,"bad start");
      for(i=0;i<8;i=i+1) begin #40; ch[i]=tx; end
      #40;
      if(tx!==1) $fatal(1,"bad stop");
      $write("%c",ch);
      if(ch==10) begin
        lines=lines+1;
        if(lines==3) $finish;
      end
    end
  end
  initial begin #10000000; $fatal(1,"timeout"); end
endmodule
'''


def decode(line):
    if len(line) != 31 or line[:3] not in ('Y1B', 'Y1C'):
        raise ValueError('bad boot frame prefix/length')
    values = [int(line[t:t+4], 16) for t in range(3, 27, 4)]
    if reduce(operator.xor, values, 0) != int(line[27:31], 16):
        raise ValueError('bad boot checksum')
    ar, br, dr, af, bf, df = values
    if dr != (br-ar) & 65535 or df != (bf-af) & 65535:
        raise ValueError('inconsistent modulo differences')
    return values


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('scratch', type=Path)
    a = p.parse_args()
    scratch = a.scratch.resolve()
    if scratch == REPO or REPO in scratch.parents:
        p.error('scratch must be outside repository')
    scratch.mkdir(parents=True, exist_ok=True)
    library = FIXTURE.split('module tb;')[0].replace(
        'always @(posedge clock_value) Q<=DI;',
        'always @(posedge clock_value) if (GSR=="ENABLED" && tb.hold) Q<=0; else Q<=DI;')
    for variant in ('b', 'c'):
        name = f'hy01{variant}'
        (scratch/f'{name}.v').write_text(generate(variant))
        (scratch/'boot_tb.v').write_text(library+TB.replace('@HOLD@', '1' if variant == 'b' else '0'))
        subprocess.run(['iverilog', '-g2012', '-s', 'tb', '-o', f'{name}.sim',
                        f'{name}.v', 'boot_tb.v'], cwd=scratch, check=True)
        result = subprocess.run(['vvp', f'{name}.sim'], cwd=scratch, capture_output=True,
                                text=True, check=True, timeout=60)
        (scratch/f'{name}.simulation.log').write_text(result.stdout)
        lines = [s for s in result.stdout.splitlines() if s.startswith('Y1')]
        assert len(lines) == 3 and len(set(lines)) == 1, lines
        values = decode(lines[0])
        expected = 2 if variant == 'b' else 0
        assert values[2] == values[5] == expected, values
        print(f'{name}: PASS three frozen wire-decoded frames; '
              f'rise/fall B-A={values[2]}/{values[5]} in the artificial fixture')
    bad = 'Y1B'+'0000'*7
    assert decode(bad) == [0]*6
    try:
        decode(bad[:-1]+'1')
    except ValueError:
        pass
    else:
        raise AssertionError('damaged checksum accepted')


if __name__ == '__main__':
    main()
