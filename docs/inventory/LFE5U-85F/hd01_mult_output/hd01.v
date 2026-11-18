// HD01 slice: which CE and RST control the MULT18X18D output register (D01,
// D05, D06). Trellis has no REG_OUTPUT_CE field and REG_OUTPUT_RST only on
// half the MULT18s, so the primitive's parameters may not be what the chip
// does. Two MULT18X18D, output register only (no input or pipeline
// registers), on CLK0 = 25 MHz, share CE0..3 and RST0..3, which fabric
// registers drive independently and at random (CE high 3/4 of the time,
// RST 1/8):
//   m0: REG_OUTPUT_CE CE2, REG_OUTPUT_RST RST3;
//   m1: REG_OUTPUT_CE CE1, REG_OUTPUT_RST RST2;
// both RESETMODE SYNC, GSR DISABLED. A and B are 4-bit LFSR values (the
// other bits 0), new every cycle; m1 multiplies A by B+1.
// Sample = {ce[3:0], rst[3:0], a[3:0], b[3:0], p0[3:0], p1[3:0]} each 25 MHz
// cycle. Line "M=<1536 hex>": 256 samples, then repeated windows.
module top(input clk_25mhz, output ftdi_rxd);
  reg [15:0] r = 16'h1D2C;
  always @(posedge clk_25mhz) r <= {r[14:0], r[15] ^ r[13] ^ r[12] ^ r[10]};
  reg [3:0] ce = 4'hf, rst = 0, a = 1, b = 3;
  always @(posedge clk_25mhz) begin
    ce <= {r[1:0] != 0, r[3:2] != 0, r[5:4] != 0, r[7:6] != 0};
    rst <= {r[10:8] == 0, r[13:11] == 0, r[15:14] == 0 && r[0], r[9:7] == 7};
    a <= r[11:8];
    b <= r[15:12];
  end
  wire [3:0] b1 = b + 1;
  wire [35:0] p0, p1;
  MULT18X18D #(.REG_INPUTA_CLK("NONE"), .REG_INPUTB_CLK("NONE"), .REG_PIPELINE_CLK("NONE"),
               .REG_OUTPUT_CLK("CLK0"), .REG_OUTPUT_CE("CE2"), .REG_OUTPUT_RST("RST3"),
               .RESETMODE("SYNC"), .GSR("DISABLED"))
    m0(.A0(a[0]), .A1(a[1]), .A2(a[2]), .A3(a[3]), .B0(b[0]), .B1(b[1]), .B2(b[2]), .B3(b[3]),
       .SIGNEDA(1'b0), .SIGNEDB(1'b0), .SOURCEA(1'b0), .SOURCEB(1'b0),
       .CLK0(clk_25mhz), .CLK1(1'b0), .CLK2(1'b0), .CLK3(1'b0),
       .CE0(ce[0]), .CE1(ce[1]), .CE2(ce[2]), .CE3(ce[3]),
       .RST0(rst[0]), .RST1(rst[1]), .RST2(rst[2]), .RST3(rst[3]),
       .P0(p0[0]), .P1(p0[1]), .P2(p0[2]), .P3(p0[3]));
  MULT18X18D #(.REG_INPUTA_CLK("NONE"), .REG_INPUTB_CLK("NONE"), .REG_PIPELINE_CLK("NONE"),
               .REG_OUTPUT_CLK("CLK0"), .REG_OUTPUT_CE("CE1"), .REG_OUTPUT_RST("RST2"),
               .RESETMODE("SYNC"), .GSR("DISABLED"))
    m1(.A0(a[0]), .A1(a[1]), .A2(a[2]), .A3(a[3]), .B0(b1[0]), .B1(b1[1]), .B2(b1[2]), .B3(b1[3]),
       .SIGNEDA(1'b0), .SIGNEDB(1'b0), .SOURCEA(1'b0), .SOURCEB(1'b0),
       .CLK0(clk_25mhz), .CLK1(1'b0), .CLK2(1'b0), .CLK3(1'b0),
       .CE0(ce[0]), .CE1(ce[1]), .CE2(ce[2]), .CE3(ce[3]),
       .RST0(rst[0]), .RST1(rst[1]), .RST2(rst[2]), .RST3(rst[3]),
       .P0(p1[0]), .P1(p1[1]), .P2(p1[2]), .P3(p1[3]));
  wire [23:0] s = {ce, rst, a, b, p0[3:0], p1[3:0]};

  reg [8:0] u = 0;
  reg [6143:0] rec = 0;
  localparam REC = 0, SEND = 1;
  reg state = REC;
  // UART: "M=" 1536 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [10:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "M";
    else if (ci == 1) ch = "=";
    else if (ci < 1538) ch = hex(rec[6143 - 4 * (ci - 2) -: 4]);
    else if (ci == 1538) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz)
    case (state)
      REC: begin
        rec <= {rec[6119:0], s};
        u <= u + 1;
        if (u == 255) begin state <= SEND; u <= 0; end
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
            if (ci == 1539) begin ci <= 0; state <= REC; end
            else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  assign ftdi_rxd = tx;
endmodule
