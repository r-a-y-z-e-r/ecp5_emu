// HI03 slice: the X2 gearbox (questions I03/C04/C05): ECLKSYNCB, CLKDIVF /2,
// ODDRX2F and IDDRX2F, cycle by cycle.
// eclk_in = 25 MHz / 16 from the fabric (nextpnr --allow-fabric-eclk), through
// ECLKSYNCB (STOP=0) to eclk; CLKDIVF /2 makes sclk from eclk. A flip-flop
// toggles on every sclk rising edge (st). All recording is in the 25 MHz
// domain or in sclk's own; the result goes out over the FTDI UART.
//   ODDRX2F on led E2: D0..D3 are four 7-bit LFSRs stepped on sclk rising
//     edges; the pad is read back through the same BB and sampled at 25 MHz
//     with {eclk_in, st}.
//   IDDRX2F on led D2: the pad is driven by the fabric with a 7-bit LFSR that
//     changes at 25 MHz edges in the middle of each eclk half period;
//     Q0..Q3 are recorded on sclk rising edges.
// Line: "O=<144 hex> I=<32 hex> N=<2 hex>": O = 192 samples of
// {eclk_in, st, pad}, the first sample in the MSBs; I = 32 sclk cycles of
// {Q0, Q1, Q2, Q3}; N = sclk rising edges before sample 0 (t = 2048).
module top(input clk_25mhz, output ftdi_rxd, inout led_e2, inout led_d2);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire eclk_in = div[3];
  wire eclk, sclk;
  ECLKSYNCB es(.ECLKI(eclk_in), .STOP(1'b0), .ECLKO(eclk));
  CLKDIVF #(.DIV("2.0")) cd(.CLKI(eclk), .RST(1'b0), .ALIGNWD(1'b0), .CDIVX(sclk));
  reg st = 0;
  always @(posedge sclk) st <= ~st;

  // sclk domain: data for ODDR, recording of IDDR.
  reg [6:0] l0 = 7'h01, l1 = 7'h55, l2 = 7'h3a, l3 = 7'h6c;
  always @(posedge sclk) begin
    l0 <= {l0[5:0], l0[6] ^ l0[5]};
    l1 <= {l1[5:0], l1[6] ^ l1[4]};
    l2 <= {l2[5:0], l2[6] ^ l2[5]};
    l3 <= {l3[5:0], l3[6] ^ l3[4]};
  end
  wire oddr_q, oddr_in;
  ODDRX2F oddr(.SCLK(sclk), .ECLK(eclk), .RST(1'b0),
               .D0(l0[0]), .D1(l1[0]), .D2(l2[0]), .D3(l3[0]), .Q(oddr_q));
  BB pad0(.I(oddr_q), .T(1'b0), .O(oddr_in), .B(led_e2));

  reg [6:0] lw = 7'h2b;
  always @(posedge clk_25mhz) if (div[2:0] == 3) lw <= {lw[5:0], lw[6] ^ lw[5]};
  wire iddr_in, q0, q1, q2, q3;
  BB pad1(.I(lw[0]), .T(1'b0), .O(iddr_in), .B(led_d2));
  IDDRX2F iddr(.SCLK(sclk), .ECLK(eclk), .RST(1'b0), .ALIGNWD(1'b0), .D(iddr_in),
               .Q0(q0), .Q1(q1), .Q2(q2), .Q3(q3));

  // Recording windows.
  reg [12:0] t = 0;          // 25 MHz cycles since start, saturating
  always @(posedge clk_25mhz) if (t != 13'h1fff) t <= t + 1;
  reg [575:0] rec_o = 0;     // from t = 2048 for 192 cycles
  always @(posedge clk_25mhz)
    if (t >= 2048 && t < 2240) rec_o <= {rec_o[572:0], eclk_in, st, oddr_in};
  reg [7:0] k = 0;           // sclk cycles
  reg [127:0] rec_i = 0;     // sclk cycles 64..95
  always @(posedge sclk) begin
    if (k != 8'hff) k <= k + 1;
    if (k >= 64 && k < 96) rec_i <= {rec_i[123:0], q0, q1, q2, q3};
  end
  // k is stable at t = 2048: sclk edges come with eclk rising edges at
  // t = 24 mod 32 only.
  reg [7:0] k_snap = 0;
  always @(posedge clk_25mhz) if (t == 2048) k_snap <= k;
  wire done = t == 13'h1fff;

  // UART: "O=" 144 hex " I=" 32 hex " N=" 2 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [7:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "O";
    else if (ci == 1) ch = "=";
    else if (ci < 146) ch = hex(rec_o[575 - 4 * (ci - 2) -: 4]);
    else if (ci == 146) ch = " ";
    else if (ci == 147) ch = "I";
    else if (ci == 148) ch = "=";
    else if (ci < 181) ch = hex(rec_i[127 - 4 * (ci - 149) -: 4]);
    else if (ci == 181) ch = " ";
    else if (ci == 182) ch = "N";
    else if (ci == 183) ch = "=";
    else if (ci == 184) ch = hex(k_snap[7:4]);
    else if (ci == 185) ch = hex(k_snap[3:0]);
    else if (ci == 186) ch = 8'h0d;
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
        if (ci == 187) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
