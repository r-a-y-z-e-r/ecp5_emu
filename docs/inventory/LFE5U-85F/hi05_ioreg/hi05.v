// HI05 slice: IO registers as yosys maps the Diamond primitives (LiteX's SDR
// IO uses them): IFS1P3{B,D,I,J}X and OFS1P3{B,D,I,J}X. B/D: asynchronous
// preset/clear (SRMODE=ASYNC); I/J: synchronous clear/preset
// (SRMODE=LSR_OVER_CE); SP is the clock enable; GSR enabled on half of them;
// a user GSR primitive driven by logic.
// All eight share one clock (25 MHz), one data bit d, one enable ce and one
// set/reset sr, all fabric registers stepped by a 16-bit LFSR; gsr_n drives
// GSR. The output registers drive led[3:0] and are read back through their
// BB; the pads led[7:4] are driven by the fabric with d, and the input
// registers sample them.
// Sample = {d, ce, sr, gsr_n, ob[3:0], qi[3:0]} at every 25 MHz edge.
// Line "E<2 hex> <768 hex>": 256 samples, the first in the MSBs.
//   E00: samples t = 0..255 after the start; the controls are idle (d 0,
//   ce 0, sr 0, gsr_n 1) for t < 16.
//   E01..E07: windows of 256 samples, one after another line.
module top(input clk_25mhz, output ftdi_rxd, inout [7:0] led);
  reg [15:0] r = 16'hACE1;
  reg [8:0] t = 0;
  always @(posedge clk_25mhz) begin
    r <= {r[14:0], r[15] ^ r[13] ^ r[12] ^ r[10]};
    if (t != 9'h1ff) t <= t + 1;
  end
  wire active = t >= 16;
  reg d = 0, ce = 0, sr = 0, gsr_n = 1;
  always @(posedge clk_25mhz) begin
    d <= active & r[0];
    ce <= active & (r[3:2] != 0);
    sr <= active & (r[7:5] == 0);
    gsr_n <= !(active & (r[15:11] == 0));
  end
  GSR gsr_inst(.GSR(gsr_n));

  // Output registers on led[3:0]: B (GSR on), D (off), I (on), J (off).
  wire [3:0] oq, ob;
  OFS1P3BX #(.GSR("ENABLED"))  o0(.SCLK(clk_25mhz), .SP(ce), .PD(sr), .D(d), .Q(oq[0]));
  OFS1P3DX #(.GSR("DISABLED")) o1(.SCLK(clk_25mhz), .SP(ce), .CD(sr), .D(d), .Q(oq[1]));
  OFS1P3IX #(.GSR("ENABLED"))  o2(.SCLK(clk_25mhz), .SP(ce), .CD(sr), .D(d), .Q(oq[2]));
  OFS1P3JX #(.GSR("DISABLED")) o3(.SCLK(clk_25mhz), .SP(ce), .PD(sr), .D(d), .Q(oq[3]));
  genvar k;
  generate for (k = 0; k < 4; k = k + 1) begin : outs
    BB b(.I(oq[k]), .T(1'b0), .O(ob[k]), .B(led[k]));
  end endgenerate

  // Input registers on led[7:4]: B (GSR off), D (on), I (off), J (on).
  wire [3:0] pad, qi;
  generate for (k = 0; k < 4; k = k + 1) begin : ins
    BB b(.I(d), .T(1'b0), .O(pad[k]), .B(led[4 + k]));
  end endgenerate
  IFS1P3BX #(.GSR("DISABLED")) i0(.SCLK(clk_25mhz), .SP(ce), .PD(sr), .D(pad[0]), .Q(qi[0]));
  IFS1P3DX #(.GSR("ENABLED"))  i1(.SCLK(clk_25mhz), .SP(ce), .CD(sr), .D(pad[1]), .Q(qi[1]));
  IFS1P3IX #(.GSR("DISABLED")) i2(.SCLK(clk_25mhz), .SP(ce), .CD(sr), .D(pad[2]), .Q(qi[2]));
  IFS1P3JX #(.GSR("ENABLED"))  i3(.SCLK(clk_25mhz), .SP(ce), .PD(sr), .D(pad[3]), .Q(qi[3]));

  wire [11:0] s = {d, ce, sr, gsr_n, ob, qi};
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
