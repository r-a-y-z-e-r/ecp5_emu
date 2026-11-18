// HP02 slice: pseudo-differential LVCMOS33D output on the top bank, as ULX3S
// HDMI designs use it. The pair gp[3]/gn[3] (B9/C10, R0C58 PIOA/PIOB) is an
// ODDRX1F output with IO_TYPE=LVCMOS33D and a fabric tristate. Wire jumpers
// gp[3]->gp[4] (A7) and gn[3]->gn[4] (A8) bring both pads back as inputs with
// PULLMODE=DOWN; without the jumpers both inputs read 0.
// sclk = 25 MHz / 16. Samples {true pad, complement pad} at 25 MHz for
// t = 1024..1279. The tristate t_z is high for samples t = 1156..1211.
// Line: "P=" + 128 hex (first sample in the MSBs) + CR LF, 115200 8N1.
module top(input clk_25mhz, output ftdi_rxd, output gp3, input gp4, input gn4);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire sclk = div[3];
  reg [11:0] t = 0;
  always @(posedge clk_25mhz) if (t != 12'hfff) t <= t + 1;
  reg t_z = 0;
  always @(posedge clk_25mhz) t_z <= t >= 1155 && t < 1211;

  reg [6:0] la = 7'h01, lb = 7'h55;
  always @(posedge sclk) begin
    la <= {la[5:0], la[6] ^ la[5]};
    lb <= {lb[5:0], lb[6] ^ lb[4]};
  end
  wire q;
  ODDRX1F oddr(.SCLK(sclk), .RST(1'b0), .D0(la[0]), .D1(lb[0]), .Q(q));
  OBZ ob(.I(q), .T(t_z), .O(gp3));

  wire done = t == 12'hfff;
  reg [511:0] rec = 0;
  always @(posedge clk_25mhz) if (t >= 1024 && t < 1280) rec <= {rec[509:0], gp4, gn4};

  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [7:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "P";
    else if (ci == 1) ch = "=";
    else if (ci < 130) ch = hex(rec[511 - 4 * (ci - 2) -: 4]);
    else if (ci == 130) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [22:0] pause = 0;
  always @(posedge clk_25mhz)
    if (!busy) begin
      if (pause != 0) pause <= pause - 1;
      else if (done) begin
        shift <= {1'b1, ch, 1'b0};
        busy <= 1; baud <= 0; bitn <= 0;
      end
    end else if (baud == 216) begin
      baud <= 0;
      tx <= shift[0];
      shift <= {1'b1, shift[9:1]};
      bitn <= bitn + 1;
      if (bitn == 9) begin
        busy <= 0;
        if (ci == 131) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
