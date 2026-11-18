// HI02 slice: cycle-level behaviour of ODDRX1F and IDDRX1F (question I02).
// sclk = 25 MHz / 16 from fabric. All recording happens in the 25 MHz domain
// or in sclk's own domain; the result goes out over the FTDI UART.
//   ODDRX1F on led0 (B2): D0/D1 are two 7-bit LFSRs stepped on sclk rising
//     edges; the pad is read back through the same BB and sampled at 25 MHz.
//   IDDRX1F on led4 (D1): the pad is driven by fabric with a 7-bit LFSR that
//     only changes at 25 MHz edges between sclk edges; Q0/Q1 are recorded on
//     sclk rising edges.
// Line: "O=<64 hex> I=<16 hex>": O = 128 samples of {sclk, pad}, first sample
// in the MSBs; I = 32 sclk cycles of {Q0, Q1}, first cycle in the MSBs.
module top(input clk_25mhz, output ftdi_rxd, inout led0, inout led4);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire sclk = div[3];

  // sclk domain: data for ODDR, recording of IDDR.
  reg [6:0] la = 7'h01, lb = 7'h55;
  always @(posedge sclk) begin
    la <= {la[5:0], la[6] ^ la[5]};
    lb <= {lb[5:0], lb[6] ^ lb[4]};
  end
  wire oddr_q, oddr_in;
  ODDRX1F oddr(.SCLK(sclk), .RST(1'b0), .D0(la[0]), .D1(lb[0]), .Q(oddr_q));
  BB pad0(.I(oddr_q), .T(1'b0), .O(oddr_in), .B(led0));

  reg [6:0] lw = 7'h3a;
  always @(posedge clk_25mhz) if (!div[0]) lw <= {lw[5:0], lw[6] ^ lw[5]};
  wire iddr_in, q0, q1;
  BB pad4(.I(lw[0]), .T(1'b0), .O(iddr_in), .B(led4));
  IDDRX1F iddr(.SCLK(sclk), .RST(1'b0), .D(iddr_in), .Q0(q0), .Q1(q1));

  // Recording windows.
  reg [11:0] t = 0;          // 25 MHz cycles since start, saturating
  always @(posedge clk_25mhz) if (t != 12'hfff) t <= t + 1;
  reg [255:0] rec_o = 0;     // from t = 1024 for 128 cycles
  always @(posedge clk_25mhz) if (t >= 1024 && t < 1152) rec_o <= {rec_o[253:0], sclk, oddr_in};
  reg [7:0] k = 0;           // sclk cycles
  reg [63:0] rec_i = 0;      // sclk cycles 64..95
  always @(posedge sclk) begin
    if (k != 8'hff) k <= k + 1;
    if (k >= 64 && k < 96) rec_i <= {rec_i[61:0], q0, q1};
  end
  wire done = t == 12'hfff;

  // UART: "O=" 64 hex " I=" 16 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [6:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "O";
    else if (ci == 1) ch = "=";
    else if (ci < 66) ch = hex(rec_o[255 - 4 * (ci - 2) -: 4]);
    else if (ci == 66) ch = " ";
    else if (ci == 67) ch = "I";
    else if (ci == 68) ch = "=";
    else if (ci < 85) ch = hex(rec_i[63 - 4 * (ci - 69) -: 4]);
    else if (ci == 85) ch = 8'h0d;
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
        if (ci == 86) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
