"""Write hd08.v: the C registers and the opcode registers of the ALU54B (HD08).
Usage: python3 hd08_gen.py > hd08.v; python3 hd08_gen.py s > hd08s.v;
python3 hd08_gen.py b > hd08b.v; python3 hd08_gen.py c > hd08c.v;
python3 hd08_gen.py bs > hd08bs.v

hd08s.v records only the first window, from the first clock after
configuration, and sends it again and again. hd08b.v has no IN_1 register:
o4 has the three first stages, o7 IN_0 and both stages of OP0 and OP1.
hd08bs.v is hd08b.v recording only its first window, as hd08s.v does.
hd08c.v is hd08b.v with the DSP data constants on nets: each is xored with
z = &r, which the control LFSR never reaches, so nextpnr routes them
instead of tying CIB inputs (in hd08 and hd08b it left one of them
floating and tied others twice), and the ALU's C[17:0] and C[44:27] take
the nets of the MULT18 C inputs, the same wires.

The port lists of MULT18X18D and ALU54B are bit-per-port, so the file is
generated."""

import sys

ROW = 58
M54 = (1 << 54) - 1
VARIANT = sys.argv[1] if len(sys.argv) > 1 else ""
ONCE = VARIANT in ("s", "bs")

# Constant operands of the opcode ALUs: the two products, A, B, C and RNDPAT
# differ enough that every OP bit moves R[15:0] (checked against the HD02
# model before the build).
A0, B0, A1, B1 = 0x2A5C3, 0x1B6E9, 0x31D47, 0x0F5A3
CM0, CM1 = 0x2C6B1, 0x13A5E          # the MULT18 C inputs
CALU = 0x2D5B3C1E4F6A97 & M54        # the ALU's C pins

# Opcode ALUs: name, block X, second slice, RESETMODE (ALU; its MULT18 SYNC),
# opcode register parameters. The CE and RST choices differ from CE0/RST0,
# which the output register takes, so the fit can tell them apart.
OPCODE = [
    ("o1", 13, False, "SYNC", dict(REG_OPCODEIN_0_CLK="CLK0", REG_OPCODEIN_0_CE="CE1", REG_OPCODEIN_0_RST="RST2")),
    ("o2", 22, False, "ASYNC", dict(REG_OPCODEOP0_0_CLK="CLK0", REG_OPCODEOP0_0_CE="CE2", REG_OPCODEOP0_0_RST="RST1")),
    # OP1_0 has no CE or RST field: OP0_0's are set (OP0_0 itself off) to see
    # whether OP1_0 takes them.
    ("o3", 31, False, "SYNC", dict(REG_OPCODEOP1_0_CLK="CLK0", REG_OPCODEOP0_0_CE="CE3", REG_OPCODEOP0_0_RST="RST3")),
    ("o4", 40, False, "ASYNC", dict(REG_OPCODEIN_1_CLK="CLK0", REG_OPCODEIN_1_CE="CE3", REG_OPCODEIN_1_RST="RST1")),
    ("o5", 49, False, "SYNC", dict(REG_OPCODEOP0_1_CLK="CLK0", REG_OPCODEOP0_1_CE="CE1", REG_OPCODEOP0_1_RST="RST3")),
    ("o6", 58, False, "ASYNC", dict(REG_OPCODEOP1_1_CLK="CLK0", REG_OPCODEOP0_1_CE="CE2", REG_OPCODEOP0_1_RST="RST2")),
    ("o7", 58, True, "SYNC", dict(
        REG_OPCODEIN_0_CLK="CLK0", REG_OPCODEIN_0_CE="CE1", REG_OPCODEIN_0_RST="RST2",
        REG_OPCODEIN_1_CLK="CLK0", REG_OPCODEIN_1_CE="CE3", REG_OPCODEIN_1_RST="RST1",
        REG_OPCODEOP0_0_CLK="CLK0", REG_OPCODEOP0_0_CE="CE2", REG_OPCODEOP0_0_RST="RST1",
        REG_OPCODEOP1_0_CLK="CLK0",
        REG_OPCODEOP0_1_CLK="CLK0", REG_OPCODEOP0_1_CE="CE1", REG_OPCODEOP0_1_RST="RST3",
        REG_OPCODEOP1_1_CLK="CLK0")),
]
NETS = VARIANT == "c"
if VARIANT in ("b", "bs", "c"):
    OPCODE[3] = ("o4", 40, False, "ASYNC", dict(
        REG_OPCODEIN_0_CLK="CLK0", REG_OPCODEIN_0_CE="CE3", REG_OPCODEIN_0_RST="RST1",
        REG_OPCODEOP0_0_CLK="CLK0", REG_OPCODEOP0_0_CE="CE0", REG_OPCODEOP0_0_RST="RST2",
        REG_OPCODEOP1_0_CLK="CLK0"))
    OPCODE[6] = ("o7", 58, True, "SYNC", dict(
        REG_OPCODEIN_0_CLK="CLK0", REG_OPCODEIN_0_CE="CE1", REG_OPCODEIN_0_RST="RST2",
        REG_OPCODEOP0_0_CLK="CLK0", REG_OPCODEOP0_0_CE="CE2", REG_OPCODEOP0_0_RST="RST1",
        REG_OPCODEOP1_0_CLK="CLK0",
        REG_OPCODEOP0_1_CLK="CLK0", REG_OPCODEOP0_1_CE="CE1", REG_OPCODEOP0_1_RST="RST3",
        REG_OPCODEOP1_1_CLK="CLK0"))
READ_C = "11'b01000111111"   # R = C: Z = 3, X and Y zero, a sum
READ_A = "11'b01000001110"   # R = A: X = 2, Y and Z zero, a sum
# C ALUs: name, block X (second slice), OP, RESETMODE (ALU and both MULT18),
# REG_INPUTC0/1_CLK of the ALU, REG_INPUTC_CLK of the first and the second
# MULT18 (the same bits), recorded bits of R.
C_ALUS = [
    ("c1", 13, READ_C, "SYNC", ("CLK0", "CLK0"), ("CLK0", "CLK0"), (50, 30, 20, 0)),
    ("c2", 22, READ_A, "ASYNC", ("CLK0", "CLK0"), ("CLK0", "CLK0"), (50, 44, 40, 36)),
    ("c3", 31, READ_C, "SYNC", ("CLK0", "NONE"), ("CLK0", "NONE"), (50, 30, 20, 0)),
    # As in HD03 a1: the ALU asks for the registers, its MULT18 do not.
    ("c4", 40, READ_C, "SYNC", ("CLK0", "CLK0"), ("NONE", "NONE"), (50, 30, 20, 0)),
]


def bus(port, net, n, lo=0, plo=0):
    return ", ".join(f".{port}{k + plo}({net}[{k + lo}])" for k in range(n))


def const(port, value, n, plo=0):
    if NETS:
        # A net that is the constant for as long as z is low.
        return ", ".join(f".{port}{k + plo}({'~z' if value >> k & 1 else 'z'})" for k in range(n))
    return ", ".join(f".{port}{k + plo}(1'b{value >> k & 1})" for k in range(n))


def alu_c_pins(value):
    """The ALU's C pins of an opcode ALU: in hd08c C[17:0] and C[44:27] take
    the MULT18 C constants, which are the same wires."""
    if not NETS:
        return const("C", value, 54)
    value = value & ~(0x3FFFF | 0x3FFFF << 27) | CM0 | CM1 << 27
    return const("C", value, 54)


CONTROLS = ("CLK0(clk_25mhz), .CLK1(1'b0), .CLK2(1'b0), .CLK3(1'b0), "
            + ", ".join(f".CE{k}(ce[{k}])" for k in range(4)) + ", "
            + ", ".join(f".RST{k}(rst[{k}])" for k in range(4)))


def params(d):
    return "".join(f'.{k}("{v}"), ' for k, v in d.items())


def mult(bel, name, a, b, c, cclk, mode):
    return f"""  (* BEL="{bel}" *)
  MULT18X18D #(.REG_INPUTA_CLK("NONE"), .REG_INPUTB_CLK("NONE"), .REG_INPUTC_CLK("{cclk}"),
      .REG_PIPELINE_CLK("NONE"), .REG_OUTPUT_CLK("NONE"), .GSR("ENABLED"), .RESETMODE("{mode}"))
    {name}({a}, {b}, {c},
      .SIGNEDA(1'b0), .SIGNEDB(1'b0), .SOURCEA(1'b0), .SOURCEB(1'b0), .{CONTROLS},
      {bus("P", name + "_p", 36)}, {bus("ROA", name + "_roa", 18)}, {bus("ROB", name + "_rob", 18)},
      .SIGNEDP({name}_sp));
"""


def alu(name, bx, second, op, mode, extra, c_pins, mults):
    x = bx + 4 * second
    m = [f"{name}_m0", f"{name}_m1"]
    out = f"""  wire [35:0] {m[0]}_p, {m[1]}_p;
  wire [17:0] {m[0]}_roa, {m[0]}_rob, {m[1]}_roa, {m[1]}_rob;
  wire {m[0]}_sp, {m[1]}_sp;
  wire [53:0] {name}_r;
"""
    out += mult(f"X{x}/Y{ROW}/MULT18_{4 * second}", m[0], *mults[0])
    out += mult(f"X{x + 1}/Y{ROW}/MULT18_{4 * second + 1}", m[1], *mults[1])
    out += f"""  (* BEL="X{x + 3}/Y{ROW}/ALU54_{3 + 4 * second}" *)
  ALU54B #(.REG_OUTPUT0_CLK("CLK0"), .REG_OUTPUT1_CLK("CLK0"), .REG_FLAG_CLK("CLK0"),
      {params(extra)}.GSR("ENABLED"), .RESETMODE("{mode}"))
    {name}({bus("A", m[0] + "_roa", 18)}, {bus("A", m[0] + "_rob", 18, 0, 18)},
      {bus("B", m[1] + "_roa", 18)}, {bus("B", m[1] + "_rob", 18, 0, 18)},
      {bus("MA", m[0] + "_p", 36)}, {bus("MB", m[1] + "_p", 36)},
      {c_pins}, {", ".join(f".OP{k}({op}[{k}])" for k in range(11))},
      .SIGNEDIA({m[0]}_sp), .SIGNEDIB({m[1]}_sp), .{CONTROLS},
      {bus("R", name + "_r", 54)});
"""
    return out


opcode_rows = "".join(
    f"//   {n}    {bx}     {'second' if s else 'first '}  {mode:<5}      "
    + ", ".join(f"{k.removeprefix('REG_OPCODE')}={v}" for k, v in d.items()) + "\n"
    for n, bx, s, mode, d in OPCODE)
c_rows = "".join(
    f"//   {n}    {bx}     second  {'R = C' if op == READ_C else 'R = A'}  {mode:<5}      "
    f"{ac[0]:<4} {ac[1]:<4}    {mc[0]:<4} {mc[1]:<4}   R[{', '.join(map(str, bits))}]\n"
    for n, bx, op, mode, ac, mc, bits in C_ALUS)
print(f"""// HD08{VARIANT} slice: the C registers and the opcode registers of the ALU54B (D01,
// D05). Generated by hd08_gen.py{" " + VARIANT if VARIANT else ""}.{chr(10) + "// The DSP data constants are nets: each is xored with z = &r, which the" + chr(10) + "// control LFSR never reaches." if NETS else ""}
// Eleven ALU54B in six DSP blocks of row {ROW} (X 13, 22, 31, 40, 49, 58), each
// with its two MULT18X18D (no registers but the C registers listed). Output
// and flag registers on CLK0, the 25 MHz clock; GSR=ENABLED on every cell.
// Opcode ALUs: constant operands, OP new at every clock.
//   name  block  slice   RESETMODE  opcode registers
{opcode_rows}// C ALUs: C new at every clock, on the ALU's pins and on the MULT18 C ports
// (first c[17:0], second c[44:27]); A and B of the MULT18 zero.
//   name  block  slice   OP     RESETMODE  ALU C0/C1    MULT18 C   recorded
{c_rows}// Controls, shared by every block: ce[3:0] (each bit high 3/4 of the time),
// rst[3:0] (each bit high 1/8) and gsr_n (low 1/32; it drives a GSR
// primitive), fabric registers new at every clock from disjoint fields of a
// 64-bit LFSR. C is another LFSR, OP a third.
// OP: OP[3:2] and OP[10:7] random, and by window, so that no mix of old and
// new bits can select R or CIN:
//   mode 0: OP0, OP4 random; OP1 = OP5 = OP6 = 1 (X = A or 0, Z = RNDPAT or 0)
//   mode 1: OP1, OP5 random; OP0 = OP6 = 1, OP4 = 0 (X = MA or 0, Z = A or RNDPAT)
//   mode 2: OP0, OP6 random; OP1 = OP4 = OP5 = 1 (X = A or 0, Z = C or 0)
// The mode moves on when a window has been recorded.
// Sample (160 bits) = {{ce[3:0], rst[3:0], 000, gsr_n, 0, op[10:0],
// c[50], c[30], c[20], c[0], c[14], c[8], c[4], 0, the recorded bits of c1..c4,
// R[15:0] of o1..o7}} at every 25 MHz edge, taken before the edge.
// Line "M<5120 hex>": 128 samples, the first in the MSBs""" + (""";
// only the first window, from the first clock, sent again and again.""" if ONCE else ".") + f"""
module top(input clk_25mhz, output ftdi_rxd);
  reg [63:0] r = 64'h1D2CACE15EED0001;{chr(10) + "  wire z = &r;  // low unless the LFSR is all ones, which it never is" if NETS else ""}
  reg [63:0] d = 64'h0123456789ABCDEF;
  reg [63:0] q = 64'h0F1E2D3C4B5A6978;
  always @(posedge clk_25mhz) begin
    r <= {{r[62:0], r[63] ^ r[62] ^ r[60] ^ r[59]}};
    d <= {{d[62:0], d[63] ^ d[62] ^ d[60] ^ d[59]}};
    q <= {{q[62:0], q[63] ^ q[62] ^ q[60] ^ q[59]}};
  end
  wire [53:0] c = d[53:0];
  reg [3:0] ce = 4'hf, rst = 0;
  reg gsr_n = 1;
  reg [10:0] op = 11'b01001111011;
  reg [1:0] mode = 0;
  always @(posedge clk_25mhz) begin
    ce <= {{r[1:0] != 0, r[3:2] != 0, r[5:4] != 0, r[7:6] != 0}};
    rst <= {{r[10:8] == 0, r[13:11] == 0, r[16:14] == 0, r[19:17] == 0}};
    gsr_n <= r[24:20] != 0;
    // The random bits are five LFSR places apart: q shifts by one a clock,
    // so a bit delayed by one or two clocks is never another one of them.
    case (mode)
      0: op <= {{q[0], q[5], q[10], q[15], 1'b1, 1'b1, q[20], q[25], q[30], 1'b1, q[35]}};
      1: op <= {{q[0], q[5], q[10], q[15], 1'b1, q[20], 1'b0, q[25], q[30], q[35], 1'b1}};
      default: op <= {{q[0], q[5], q[10], q[15], q[20], 1'b1, 1'b1, q[25], q[30], 1'b1, q[35]}};
    endcase
  end
  GSR gsr_inst(.GSR(gsr_n));
""")
for name, bx, second, mode, extra in OPCODE:
    zero = const("A", 0, 18)
    mults = [
        (const("A", A0, 18), const("B", B0, 18), const("C", CM0, 18), "NONE", "SYNC"),
        (const("A", A1, 18), const("B", B1, 18), const("C", CM1, 18), "NONE", "SYNC"),
    ]
    print(alu(name, bx, second, "op", mode, extra, alu_c_pins(CALU), mults))
for name, bx, op, mode, alu_c, mult_c, _ in C_ALUS:
    zeros = (const("A", 0, 18), const("B", 0, 18))
    mults = [
        (*zeros, bus("C", "c", 18, 0), mult_c[0], mode),
        (*zeros, bus("C", "c", 18, 27), mult_c[1], mode),
    ]
    extra = dict(REG_INPUTC0_CLK=alu_c[0], REG_INPUTC1_CLK=alu_c[1])
    print(f"  wire [10:0] {name}_op = {op};")
    print(alu(name, bx, True, f"{name}_op", mode, extra, bus("C", "c", 54), mults))
picks = lambda n, bits: ", ".join(f"{n}_r[{b}]" for b in bits)
print(f"""  wire [159:0] s = {{ce, rst, 3'b0, gsr_n, 1'b0, op, c[50], c[30], c[20], c[0], c[14], c[8], c[4], 1'b0,
                    {", ".join(picks(n, bits) for n, *_, bits in C_ALUS)},
                    {", ".join(f"{n}_r[15:0]" for n, *_ in OPCODE)}}};

  reg [6:0] u = 0;
  reg [20479:0] rec = 0;
  localparam REC = 0, SEND = 1;
  reg state = REC;
  // UART: "M" 5120 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [12:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "M";
    else if (ci < 5121) ch = hex(rec[20479 - 4 * (ci - 1) -: 4]);
    else if (ci == 5121) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz)
    case (state)
      REC: begin
        rec <= {{rec[20319:0], s}};
        u <= u + 1;
        if (u == 127) begin
          state <= SEND; u <= 0;
          mode <= mode == 2 ? 2'd0 : mode + 2'd1;
        end
      end
      SEND:
        if (!busy) begin
          shift <= {{1'b1, ch, 1'b0}};
          busy <= 1; baud <= 0; bitn <= 0;
        end else if (baud == 216) begin
          baud <= 0;
          tx <= shift[0];
          shift <= {{1'b1, shift[9:1]}};
          bitn <= bitn + 1;
          if (bitn == 9) begin
            busy <= 0;
            if (ci == 5122) begin ci <= 0; state <= {"SEND" if ONCE else "REC"}; end
            else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  assign ftdi_rxd = tx;
endmodule""")
