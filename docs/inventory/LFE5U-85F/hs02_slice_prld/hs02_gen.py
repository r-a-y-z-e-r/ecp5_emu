"""What does a slice FF preload from M when LSRMODE=PRLD?

Design table: 32 directly instantiated TRELLIS_FF, CLK=25 MHz, GSR=ENABLED,
CLKMUX=CLK, LSRMUX=LSR. Each set of eight enumerates the Cartesian product:
  SRMODE       REGSET     CEMUX
  LSR_OVER_CE  RESET/SET  CE/1
  ASYNC        RESET/SET  CE/1
FFs 00..23 are three copies of those eight settings with LSRMODE=PRLD;
FFs 24..31 use the same eight settings with LSRMODE=LSR as controls.
Every FF receives independent registered DI, M, CE and LSR traffic fields.
A kept identity TRELLIS_COMB drives each DI so nextpnr packs the FF with its
LUT and preserves M as the separate preload input (SD=1). The identity LUT
leaves its unused M port disconnected: LUT M and FF M share a physical wire,
so tying the LUT port to zero would conflict with the FF preload traffic.

Traffic: one 64-bit LFSR, x^64+x^63+x^61+x^60+1, seed 1D2CACE15EED0001.
For FF i, take the low seven bits of step^(7*i)(L): DI=bit0, M=bit1,
CE=(bits3:2 != 0), LSR=(bits6:4 == 0). CE is high about 3/4, LSR about 1/8.
Advance 224 single-bit steps per clock; traffic is registered, initialized
from the seed and updated from the next LFSR state, so traffic(k)=fields(L_k).
CE and LSR are decoded before their fabric registers to avoid decode glitches
on the asynchronous control pins.

Line: P + 16 hex win_lfsr + 1024 hex Q + CR LF, 115200 8N1.
Each of 128 samples is Q[31:0] (eight hex); sample zero is in the MSBs.
Alignment: sample k is BEFORE rising edge k and uses traffic(step224^k(L_0)).
Synchronous updates at edge k use traffic k; asynchronous responses can also
use traffic k+1 before sample k+1. No startup Q value is assumed in fitting.
The LFSR and FFs run through UART. Free-running records a new window after
sending; variant s repeats the first window from configuration forever.
Usage: python3 hs02_gen.py [s] > hs02[s].v
"""
import itertools
import sys

SEED = 0x1D2CACE15EED0001
MASK = (1 << 64) - 1
COMBOS = list(itertools.product(("LSR_OVER_CE", "ASYNC"), ("RESET", "SET"), ("CE", "1")))
SETTINGS = [("PRLD" if i < 24 else "LSR", *COMBOS[i % 8]) for i in range(32)]


def shift(x, n):
    for _ in range(n):
        x = ((x << 1) & MASK) | (((x >> 63) ^ (x >> 62) ^ (x >> 60) ^ (x >> 59)) & 1)
    return x


def step(x):
    return shift(x, 224)


def traffic(x):
    fields = []
    for _ in SETTINGS:
        fields.append((x & 1, (x >> 1) & 1, int((x >> 2) & 3 != 0), int((x >> 4) & 7 == 0)))
        x = shift(x, 7)
    return fields


def generate(once=False):
    rows, cells = [], []
    for i, (lsrmode, srmode, regset, cemux) in enumerate(SETTINGS):
        rows.append(f"// ff{i:02} {lsrmode:4} {srmode:11} {regset:5} {cemux}")
        cells.append(f'''  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut{i:02} (
    .A(t[{4*i}]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[{i}]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("{lsrmode}"), .SRMODE("{srmode}"),
    .REGSET("{regset}"), .CEMUX("{cemux}")) ff{i:02} (
    .CLK(clk_25mhz), .DI(di[{i}]), .M(t[{4*i+1}]),
    .CE(t[{4*i+2}]), .LSR(t[{4*i+3}]), .Q(q[{i}]));''')
    header = "\n".join(("// " + line).rstrip() for line in __doc__.splitlines())
    return header + "\n// name LSRMODE SRMODE REGSET CEMUX\n" + "\n".join(rows) + f'''
module top(input clk_25mhz, output ftdi_rxd);
  function [63:0] shift7(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 7; j = j + 1)
        a = {{a[62:0], a[63] ^ a[62] ^ a[60] ^ a[59]}};
      shift7 = a;
    end
  endfunction
  function [127:0] expand(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 32; j = j + 1) begin
        expand[4*j] = a[0];
        expand[4*j+1] = a[1];
        expand[4*j+2] = |a[3:2];
        expand[4*j+3] = ~|a[6:4];
        a = shift7(a);
      end
    end
  endfunction
  function [63:0] step224(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 32; j = j + 1) a = shift7(a);
      step224 = a;
    end
  endfunction
  reg [63:0] lfsr = 64'h{SEED:016X};
  wire [63:0] next_lfsr = step224(lfsr);
  reg [127:0] t = expand(64'h{SEED:016X});
  always @(posedge clk_25mhz) begin
    lfsr <= next_lfsr;
    t <= expand(next_lfsr);
  end
  wire [31:0] q, di;
{chr(10).join(cells)}
  reg [63:0] win_lfsr = 64'h{SEED:016X};
  reg [6:0] u = 0;
  reg [4095:0] rec = 0;
  localparam REC = 0, SEND = 1;
  reg state = REC;
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [10:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "P";
    else if (ci < 17) ch = hex(win_lfsr[63 - 4 * (ci - 1) -: 4]);
    else if (ci < 1041) ch = hex(rec[4095 - 4 * (ci - 17) -: 4]);
    else if (ci == 1041) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz)
    case (state)
      REC: begin
        if (u == 0) win_lfsr <= lfsr;
        rec <= {{rec[4063:0], q}};
        u <= u + 1;
        if (u == 127) begin state <= SEND; u <= 0; end
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
            if (ci == 1042) begin ci <= 0; state <= {"SEND" if once else "REC"}; end
            else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  assign ftdi_rxd = tx;
endmodule
'''


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["s"]):
        raise SystemExit("Usage: hs02_gen.py [s]")
    print(generate(bool(sys.argv[1:])), end="")
