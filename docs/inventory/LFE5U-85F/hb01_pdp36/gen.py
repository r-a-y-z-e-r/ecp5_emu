"""Writes hb01.v, the HB01 bench: which pins carry which half of a 36-bit
PDPW16KD read, and which controls act on each half.

Three EBRs share one write port and one read address. All are DP16KD with
DATA_WIDTH_A=36, which nextpnr places as PDPW16KD, so every DOA/DOB pin is
named in the HDL and no tool chooses an order for us:
  E1: 36-bit read, NOREG, SYNC reset with RSTA and RSTB driven separately;
  E2: 36-bit read, OUTREG, OCEA and OCEB driven separately;
  E3: 18-bit read (DATA_WIDTH_B=18), NOREG: the existing narrow view.
CLKA and CLKB are flip-flops of the 25 MHz observer, pulsed by a step
program, so the bench decides each edge of each port. After each step the
observer prints one UART line, from the second pass of the program on (115200 8N1 on ftdi_rxd):
  S<step> A<E1 DOA> B<E1 DOB> C<E2 DOA> D<E2 DOB> E<E3 DOB>
with each 18-bit value as five hex digits. The program repeats forever.

Usage: python3 gen.py  (writes hb01.v and program.json next to it)
"""
import json, os

here = os.path.dirname(os.path.abspath(__file__))


def lcg(seed):
    while True:
        seed = (seed * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        yield seed >> 28


rand = lcg(0x4842_3031)
# 36-bit INIT words 0..3, then five 36-bit write patterns.
init = [next(rand) & ((1 << 36) - 1) for _ in range(4)]
data = [next(rand) & ((1 << 36) - 1) for _ in range(5)]

# Steps: pa/pb pulse CLKA/CLKB; wa/ra word addresses; half is the low bit of
# E3's 18-bit read address; wsel picks the write pattern; be the four 9-bit
# group enables (ADA3..0); cea/ceb, ocea/oceb (E2), rsta/rstb (E1).
DEFAULT = dict(pa=0, pb=0, wa=0, ra=0, half=0, wsel=0, be=0xF,
               cea=0, ceb=0, ocea=1, oceb=1, rsta=0, rstb=0)


def read(ra, half=0, **kw):
    return dict(DEFAULT, pb=1, ceb=1, ra=ra, half=half, **kw)


def write(wa, wsel, be=0xF):
    return dict(DEFAULT, pa=1, cea=1, wa=wa, wsel=wsel, be=be)


def edge(**kw):
    """A clock edge with CEB=0. E2's OCE stays 0 unless asked: an OUTREG
    edge with OCE=1 and CE=0 is HB03's question, not this bench's."""
    return dict(DEFAULT, **dict(dict(ocea=0, oceb=0), **kw))


steps = [
    # INIT: words 0..3, E3 low then high 18-bit word of each.
    read(0), read(1), read(2), read(3),
    read(0, 1), read(1, 1), read(2, 1), read(3, 1),
    # Live writes through DIA (data[17:0]) and DIB (data[35:18]).
    write(8, 0), write(9, 1),
    read(8), read(9), read(8, 1), read(9, 1),
    # Group enables: ADA0..3 select the 9-bit groups of the 36-bit write.
    write(10, 2), write(10, 3, 0b0001), write(10, 4, 0b0100),
    read(10), read(10, 1),
    write(11, 2), write(11, 3, 0b0010), write(11, 4, 0b1000),
    read(11), read(11, 1),
    # E1 SYNC reset: RSTB alone, RSTA alone (CLKB, then CLKA), with CEB=0;
    # then each with CEB=1 reading word 1.
    read(0), edge(pb=1, rstb=1),
    read(0), edge(pb=1, rsta=1),
    read(0), edge(pa=1, rsta=1),
    read(0), read(1, rstb=1),
    read(0), read(1, rsta=1),
    # E2 OUTREG: output W2, latch W3 with OCE=0; then one edge that may move
    # the latch into the output: CLKA alone with both OCE, then a CLKB read
    # of word 3 again with OCEB only, and with OCEA only.
    read(2), read(2), read(3, ocea=0, oceb=0), edge(pa=1, ocea=1, oceb=1),
    read(2), read(2), read(3, ocea=0, oceb=0), read(3, ocea=0),
    read(2), read(2), read(3, ocea=0, oceb=0), read(3, oceb=0),
]
assert len(steps) <= 64


def initval(words):
    """INITVAL_00 of a 36-bit view: word w is 18-bit chunks 2w (bits 17:0)
    and 2w+1 (bits 35:18), 20 bits per chunk (EBR_ADDRESSING.md)."""
    value = 0
    for w, word in enumerate(words):
        for half in range(2):
            value |= ((word >> (18 * half)) & 0x3FFFF) << (20 * (2 * w + half))
    return value


def rom():
    lines = []
    for i, s in enumerate(steps):
        word = (s["pa"] << 39 | s["pb"] << 38 | s["wa"] << 29 | s["ra"] << 20 | s["half"] << 19
                | s["wsel"] << 16 | s["be"] << 12 | s["cea"] << 11 | s["ceb"] << 10
                | s["ocea"] << 9 | s["oceb"] << 8 | s["rsta"] << 7 | s["rstb"] << 6)
        lines.append(f"      6'd{i}: c = 40'h{word:010X};")
    return "\n".join(lines)


def ebr(name, width_b, regmode, ocea, oceb, rsta, rstb):
    ports = []
    ports += [f".DIA{i}(wd[{i}])" for i in range(18)]
    ports += [f".DIB{i}(wd[{18 + i}])" for i in range(18)]
    ports += [f".ADA{i}(be[{i}])" for i in range(4)] + [".ADA4(1'b0)"]
    ports += [f".ADA{5 + i}(wa[{i}])" for i in range(9)]
    if width_b == 36:
        ports += [f".ADB{i}(1'b0)" for i in range(5)]
        ports += [f".ADB{5 + i}(ra[{i}])" for i in range(9)]
    else:
        ports += [f".ADB{i}(1'b0)" for i in range(4)] + [".ADB4(half)"]
        ports += [f".ADB{5 + i}(ra[{i}])" for i in range(9)]
    ports += [f".DOA{i}({name}_a[{i}])" for i in range(18)]
    ports += [f".DOB{i}({name}_b[{i}])" for i in range(18)]
    ports += [".CLKA(clka)", ".CLKB(clkb)", ".CEA(cea)", ".CEB(ceb)",
              f".OCEA({ocea})", f".OCEB({oceb})", ".WEA(1'b1)", ".WEB(1'b0)",
              f".RSTA({rsta})", f".RSTB({rstb})",
              ".CSA0(1'b0)", ".CSA1(1'b0)", ".CSA2(1'b0)",
              ".CSB0(1'b0)", ".CSB1(1'b0)", ".CSB2(1'b0)"]
    params = [
        ".DATA_WIDTH_A(36)", f".DATA_WIDTH_B({width_b})",
        f'.REGMODE_A("{regmode}")', f'.REGMODE_B("{regmode}")',
        '.RESETMODE("SYNC")', '.GSR("DISABLED")',
        f".INITVAL_00(320'h{initval(init):080X})",
    ]
    return (f"  wire [17:0] {name}_a, {name}_b;\n"
            f"  DP16KD #({', '.join(params)}) {name} (\n    "
            + ",\n    ".join(ports) + ");\n")


VERILOG = f"""// HB01: order of the two halves of a 36-bit PDPW16KD read, and the
// controls of each half. Generated by gen.py; see README.md.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  // Step program: {{pa, pb, wa[8:0], ra[8:0], half, wsel[2:0], be[3:0],
  // cea, ceb, ocea, oceb, rsta, rstb, 6'b0}}.
  reg [5:0] step = 0;
  reg [39:0] c;
  always @* case (step)
{rom()}
      default: c = 40'h0;
    endcase
  reg [35:0] pattern;
  always @* case (c[18:16])
{chr(10).join(f"      3'd{i}: pattern = 36'h{d:09X};" for i, d in enumerate(data))}
      default: pattern = 36'h0;
  endcase

  // Controls are set at t=0, the port clocks rise at t=4 and fall at t=8,
  // the outputs are sampled at t=12, then the line is printed.
  reg clka = 0, clkb = 0;
  reg [8:0] wa = 0, ra = 0;
  reg half = 0;
  reg [35:0] wd = 0;
  reg [3:0] be = 0;
  reg cea = 0, ceb = 0, ocea = 1, oceb = 1, rsta = 0, rstb = 0;
  reg [7:0] t = 0;
  reg [17:0] q1a = 0, q1b = 0, q2a = 0, q2b = 0, q3b = 0;
  // The first pass of the program is not printed: it brings every latch
  // and output register to a value the program set.
  reg printing = 0, primed = 0;
{ebr("e1", 36, "NOREG", "1'b1", "1'b1", "rsta", "rstb")}
{ebr("e2", 36, "OUTREG", "ocea", "oceb", "1'b0", "1'b0")}
{ebr("e3", 18, "NOREG", "1'b1", "1'b1", "1'b0", "1'b0")}
  // UART, 115200 8N1 from 25 MHz.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  function [7:0] digit(input [17:0] v, input [2:0] i);
    case (i)
      0: digit = hex({{2'b0, v[17:16]}}); 1: digit = hex(v[15:12]);
      2: digit = hex(v[11:8]); 3: digit = hex(v[7:4]); default: digit = hex(v[3:0]);
    endcase
  endfunction
  reg [5:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    ch = " ";
    if (ci == 0) ch = "S";
    else if (ci == 1) ch = hex({{2'b0, step[5:4]}});
    else if (ci == 2) ch = hex(step[3:0]);
    else if (ci == 39) ch = 8'h0d;
    else if (ci == 40) ch = 8'h0a;
    else if (ci >= 3) case ((ci - 3) / 7)
      0: ch = (ci - 3) % 7 == 1 ? "A" : (ci - 3) % 7 >= 2 ? digit(q1a, (ci - 3) % 7 - 2) : " ";
      1: ch = (ci - 3) % 7 == 1 ? "B" : (ci - 3) % 7 >= 2 ? digit(q1b, (ci - 3) % 7 - 2) : " ";
      2: ch = (ci - 3) % 7 == 1 ? "C" : (ci - 3) % 7 >= 2 ? digit(q2a, (ci - 3) % 7 - 2) : " ";
      3: ch = (ci - 3) % 7 == 1 ? "D" : (ci - 3) % 7 >= 2 ? digit(q2b, (ci - 3) % 7 - 2) : " ";
      default: ch = (ci - 3) % 7 == 1 ? "E" : (ci - 3) % 7 >= 2 ? digit(q3b, (ci - 3) % 7 - 2) : " ";
    endcase
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz)
    if (!printing) begin
      t <= t + 1;
      if (t == 0) begin
        wa <= c[37:29]; ra <= c[28:20]; half <= c[19]; wd <= pattern; be <= c[15:12];
        cea <= c[11]; ceb <= c[10]; ocea <= c[9]; oceb <= c[8]; rsta <= c[7]; rstb <= c[6];
      end
      if (t == 4) begin clka <= c[39]; clkb <= c[38]; end
      if (t == 8) begin clka <= 0; clkb <= 0; end
      if (t == 12) begin
        q1a <= e1_a; q1b <= e1_b; q2a <= e2_a; q2b <= e2_b; q3b <= e3_b;
        t <= 0; ci <= 0;
        if (primed) printing <= 1;
        else begin
          step <= step == {len(steps) - 1} ? 0 : step + 1;
          if (step == {len(steps) - 1}) primed <= 1;
        end
      end
    end else if (!busy) begin
      shift <= {{1'b1, ch, 1'b0}};
      busy <= 1; baud <= 0; bitn <= 0;
    end else if (baud == 216) begin
      baud <= 0;
      tx <= shift[0];
      shift <= {{1'b1, shift[9:1]}};
      bitn <= bitn + 1;
      if (bitn == 9) begin
        busy <= 0;
        if (ci == 40) begin
          printing <= 0;
          step <= step == {len(steps) - 1} ? 0 : step + 1;
        end else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
  assign led = {{2'b0, step}};
endmodule
"""

with open(os.path.join(here, "hb01.v"), "w") as f:
    f.write(VERILOG)
with open(os.path.join(here, "program.json"), "w") as f:
    json.dump({"init": [f"{w:09X}" for w in init],
               "data": [f"{d:09X}" for d in data],
               "steps": steps}, f, indent=1)
    f.write("\n")
