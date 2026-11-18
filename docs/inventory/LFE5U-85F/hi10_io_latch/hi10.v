// HI10 slice: the IO latches (INREGMODE/OUTREGMODE=LATCH), which yosys cannot
// map (its FL1*/IFS1S1* cells are commented out). The design builds IO
// registers; hi10_latch.py then sets FF.INREGMODE=LATCH on I0's IOLOGIC and
// OUTREG.OUTREGMODE=LATCH on O0's in the nextpnr text configuration. O1 and
// I1 stay flip-flops as the control.
//   sclk = 25 MHz / 4 from fabric (div[1]): high two 25 MHz cycles, low two.
//   O0 (B2, latch) and O1 (C2, FF): OFS1P3DX, D = dout, read back through
//     their BB.
//   I0 (D1, latch) and I1 (E2, FF): IFS1P3DX, pads driven with din.
//   All four: SP = ce, CD = rst, GSR disabled.
//   din, dout, ce and rst are fabric registers stepped by a 16-bit LFSR at
//   the 25 MHz edges between sclk edges, so a latch's transparent phase sees
//   one change of its D, CE and LSR.
// Sample = {sclk, div0, ce, rst, din, dout, ob1, ob0, qi1, qi0, 0, 0} at every
// 25 MHz edge, taken before the edge. Line "E<2 hex> <768 hex>": 256 samples,
// the first in the MSBs.
//   E00: samples t = 0..255 after the start; the controls are idle (ce 1,
//   rst 0) for t < 16.
//   E01..E07: windows of 256 samples, one after another line.
module top(input clk_25mhz, output ftdi_rxd, inout [7:0] led);
  reg [15:0] r = 16'hACE1;
  reg [8:0] t = 0;
  reg [1:0] div = 0;
  always @(posedge clk_25mhz) begin
    r <= {r[14:0], r[15] ^ r[13] ^ r[12] ^ r[10]};
    if (t != 9'h1ff) t <= t + 1;
    div <= div + 1;
  end
  wire sclk = div[1];
  wire active = t >= 16;
  reg din = 0, dout = 0, ce = 1, rst = 0;
  always @(posedge clk_25mhz) if (!div[0]) begin
    din <= active & r[0];
    dout <= active & r[1];
    ce <= !active | (r[4:3] != 0);
    rst <= active & (r[8:6] == 0);
  end

  wire [1:0] oq, ob;
  OFS1P3DX #(.GSR("DISABLED")) o0(.SCLK(sclk), .SP(ce), .CD(rst), .D(dout), .Q(oq[0]));
  OFS1P3DX #(.GSR("DISABLED")) o1(.SCLK(sclk), .SP(ce), .CD(rst), .D(dout), .Q(oq[1]));
  BB b0(.I(oq[0]), .T(1'b0), .O(ob[0]), .B(led[0]));
  BB b1(.I(oq[1]), .T(1'b0), .O(ob[1]), .B(led[1]));
  wire [1:0] pad, qi;
  BB b4(.I(din), .T(1'b0), .O(pad[0]), .B(led[4]));
  BB b5(.I(din), .T(1'b0), .O(pad[1]), .B(led[5]));
  IFS1P3DX #(.GSR("DISABLED")) i0(.SCLK(sclk), .SP(ce), .CD(rst), .D(pad[0]), .Q(qi[0]));
  IFS1P3DX #(.GSR("DISABLED")) i1(.SCLK(sclk), .SP(ce), .CD(rst), .D(pad[1]), .Q(qi[1]));

  wire [11:0] s = {sclk, div[0], ce, rst, din, dout, ob, qi, 2'b00};
  reg [3071:0] rec0 = 0, rec = 0;
  always @(posedge clk_25mhz) if (t < 256) rec0 <= {rec0[3059:0], s};

  // UART: "E" 2 hex " " 768 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [2:0] e = 0;
  wire [3071:0] src = e == 0 ? rec0 : rec;
  reg [9:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "E";
    else if (ci == 1) ch = "0";
    else if (ci == 2) ch = hex({1'b0, e});
    else if (ci == 3) ch = " ";
    else if (ci < 772) ch = hex(src[3071 - 4 * (ci - 4) -: 4]);
    else if (ci == 772) ch = 8'h0d;
    else ch = 8'h0a;
  end
  localparam BOOT = 0, REC = 1, SEND = 2;
  reg [1:0] state = BOOT;
  reg [8:0] u = 0;
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz) begin
    case (state)
      BOOT: if (t == 9'h1ff) state <= SEND;
      REC: begin
        rec <= {rec[3059:0], s};
        u <= u + 1;
        if (u == 255) state <= SEND;
      end
      SEND:
        if (!busy) begin
          shift <= {1'b1, ch, 1'b0};
          busy <= 1; baud <= 0; bitn <= 0;
        end else if (baud == 216) begin
          baud <= 0;
          tx <= shift[0];
          shift <= {1'b1, shift[9:1]};
          bitn <= bitn + 1;
          if (bitn == 9) begin
            busy <= 0;
            if (ci == 773) begin
              ci <= 0;
              e <= e + 1;
              if (e == 7) state <= SEND;
              else begin state <= REC; u <= 0; end
            end else ci <= ci + 1;
          end
        end else baud <= baud + 1;
      default: state <= BOOT;
    endcase
  end
  assign ftdi_rxd = tx;
endmodule
