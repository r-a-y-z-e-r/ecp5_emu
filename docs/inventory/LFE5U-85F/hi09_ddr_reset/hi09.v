// HI09 slice: RST and GSR on the X1 DDR paths, ODDRX1F and IDDRX1F (I02).
// The emulator ran DDR paths only with GSR=DISABLED and RST tied low (HI02);
// it executed RST as an asynchronous clear without a board run and refused
// GSR=ENABLED, which nextpnr writes when the primitive's GSR parameter says
// so. Here:
//   sclk = 25 MHz / 4 from fabric (div[1]): high two 25 MHz cycles, low two.
//   ODDRX1F on led[0] (GSR=ENABLED) and led[1] (GSR=DISABLED): D0/D1 are two
//     7-bit LFSRs stepped at sclk rises; each pad is read back through its BB.
//   IDDRX1F on led[4] (GSR=ENABLED) and led[5] (GSR=DISABLED): both pads are
//     driven by the fabric bit din.
//   rst (RST of all four) and gsr_n (a user GSR primitive) are fabric
//     registers, as is e. All three change only at the 25 MHz edges between
//     sclk edges, so a change that shows at once is asynchronous.
// Sample = {sclk, rst, gsr_n, din, d0, d1, ob0, ob1, q0e, q1e, q0d, q1d} at
// every 25 MHz edge. Line "E<2 hex> <768 hex>": 256 samples, the first in the
// MSBs.
//   E00: samples t = 0..255 after the start; the controls are idle (din 0,
//   rst 0, gsr_n 1) for t < 16.
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
  // Between sclk edges: sclk changes when div goes 01->10 and 11->00, so the
  // controls change when div goes 00->01 and 10->11.
  reg din = 0, rst = 0, gsr_n = 1;
  always @(posedge clk_25mhz) if (!div[0]) begin
    din <= active & r[0];
    rst <= active & (r[7:5] == 0);
    gsr_n <= !(active & (r[15:11] == 0));
  end
  GSR gsr_inst(.GSR(gsr_n));

  // sclk domain: the ODDR data.
  reg [6:0] la = 7'h01, lb = 7'h55;
  always @(posedge sclk) begin
    la <= {la[5:0], la[6] ^ la[5]};
    lb <= {lb[5:0], lb[6] ^ lb[4]};
  end
  wire d0 = la[0], d1 = lb[0];
  wire [1:0] oq, ob;
  ODDRX1F #(.GSR("ENABLED"))  o0(.SCLK(sclk), .RST(rst), .D0(d0), .D1(d1), .Q(oq[0]));
  ODDRX1F #(.GSR("DISABLED")) o1(.SCLK(sclk), .RST(rst), .D0(d0), .D1(d1), .Q(oq[1]));
  BB b0(.I(oq[0]), .T(1'b0), .O(ob[0]), .B(led[0]));
  BB b1(.I(oq[1]), .T(1'b0), .O(ob[1]), .B(led[1]));

  wire [1:0] pad;
  BB b4(.I(din), .T(1'b0), .O(pad[0]), .B(led[4]));
  BB b5(.I(din), .T(1'b0), .O(pad[1]), .B(led[5]));
  wire q0e, q1e, q0d, q1d;
  IDDRX1F #(.GSR("ENABLED"))  i0(.SCLK(sclk), .RST(rst), .D(pad[0]), .Q0(q0e), .Q1(q1e));
  IDDRX1F #(.GSR("DISABLED")) i1(.SCLK(sclk), .RST(rst), .D(pad[1]), .Q0(q0d), .Q1(q1d));

  wire [11:0] s = {sclk, rst, gsr_n, din, d0, d1, ob, q0e, q1e, q0d, q1d};
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
