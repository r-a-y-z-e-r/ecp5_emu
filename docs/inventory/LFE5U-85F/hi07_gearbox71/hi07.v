// HI07 slice: the 7:1 gearbox (I01, I03): ECLKSYNCB, CLKDIVF /3.5, ODDR71B and
// IDDR71B, cycle by cycle.
// eclk_in = 25 MHz / 16 from the fabric (nextpnr --allow-fabric-eclk), high in
// phases 8..15 of div, through ECLKSYNCB (STOP 0) to eclk; CLKDIVF /3.5 makes
// sclk from eclk. Flip-flops toggle on the rising (p) and falling (n) edges of
// sclk. All recording is in the 25 MHz domain or in sclk's own; the result
// goes out over the FTDI UART.
//   ODDR71B on led C1 (PIO A): D0..D6 are seven 7-bit LFSRs stepped on sclk
//     rising edges; the pad is read back through the same BB and sampled at
//     25 MHz with {eclk_in, p, n}.
//   IDDR71B on led D2 (PIO C): the fabric drives the pad with a 7-bit LFSR
//     that steps at 25 MHz edges in the middle of each eclk half period;
//     Q0..Q6 are recorded on sclk rising edges.
// Line: "O=<384 hex> I=<28 hex> N=<2 hex>": O = 384 samples of
// {eclk_in, p, n, pad} from t = 2048, the first sample in the MSBs; I = 16
// sclk cycles of {Q0..Q6} (sclk rising edges 64..79, counted by the design);
// N = sclk rising edges before sample 0.
module top(input clk_25mhz, output ftdi_rxd, inout led_c1, inout led_d2);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire eclk_in = div[3];
  wire eclk, sclk;
  ECLKSYNCB es(.ECLKI(eclk_in), .STOP(1'b0), .ECLKO(eclk));
  CLKDIVF #(.DIV("3.5")) cd(.CLKI(eclk), .RST(1'b0), .ALIGNWD(1'b0), .CDIVX(sclk));
  reg p = 0, n = 0;
  always @(posedge sclk) p <= ~p;
  always @(negedge sclk) n <= ~n;

  // sclk domain: data for ODDR71B.
  reg [6:0] l0 = 7'h01, l1 = 7'h55, l2 = 7'h3a, l3 = 7'h6c, l4 = 7'h17, l5 = 7'h62, l6 = 7'h4d;
  always @(posedge sclk) begin
    l0 <= {l0[5:0], l0[6] ^ l0[5]};
    l1 <= {l1[5:0], l1[6] ^ l1[4]};
    l2 <= {l2[5:0], l2[6] ^ l2[5]};
    l3 <= {l3[5:0], l3[6] ^ l3[4]};
    l4 <= {l4[5:0], l4[6] ^ l4[5]};
    l5 <= {l5[5:0], l5[6] ^ l5[4]};
    l6 <= {l6[5:0], l6[6] ^ l6[5]};
  end
  wire oddr_q, oddr_in;
  ODDR71B oddr(.SCLK(sclk), .ECLK(eclk), .RST(1'b0),
               .D0(l0[0]), .D1(l1[0]), .D2(l2[0]), .D3(l3[0]), .D4(l4[0]), .D5(l5[0]), .D6(l6[0]),
               .Q(oddr_q));
  BB pad0(.I(oddr_q), .T(1'b0), .O(oddr_in), .B(led_c1));

  reg [6:0] lw = 7'h2b;
  always @(posedge clk_25mhz) if (div[2:0] == 3) lw <= {lw[5:0], lw[6] ^ lw[5]};
  wire iddr_in;
  wire [6:0] q;
  BB pad1(.I(lw[0]), .T(1'b0), .O(iddr_in), .B(led_d2));
  IDDR71B iddr(.SCLK(sclk), .ECLK(eclk), .RST(1'b0), .ALIGNWD(1'b0), .D(iddr_in),
               .Q0(q[0]), .Q1(q[1]), .Q2(q[2]), .Q3(q[3]), .Q4(q[4]), .Q5(q[5]), .Q6(q[6]));

  // Recording windows.
  reg [12:0] t = 0;
  always @(posedge clk_25mhz) if (t != 13'h1fff) t <= t + 1;
  reg [1535:0] rec_o = 0;
  always @(posedge clk_25mhz)
    if (t >= 2048 && t < 2432) rec_o <= {rec_o[1531:0], eclk_in, p, n, oddr_in};
  reg [7:0] k = 0;
  reg [111:0] rec_i = 0;
  always @(posedge sclk) begin
    if (k != 8'hff) k <= k + 1;
    if (k >= 64 && k < 80) rec_i <= {rec_i[104:0], q[0], q[1], q[2], q[3], q[4], q[5], q[6]};
  end
  reg [7:0] k_snap = 0;
  always @(posedge clk_25mhz) if (t == 2048) k_snap <= k;
  wire done = t == 13'h1fff;

  // UART: "O=" 384 hex " I=" 28 hex " N=" 2 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [8:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "O";
    else if (ci == 1) ch = "=";
    else if (ci < 386) ch = hex(rec_o[1535 - 4 * (ci - 2) -: 4]);
    else if (ci == 386) ch = " ";
    else if (ci == 387) ch = "I";
    else if (ci == 388) ch = "=";
    else if (ci < 417) ch = hex(rec_i[111 - 4 * (ci - 389) -: 4]);
    else if (ci == 417) ch = " ";
    else if (ci == 418) ch = "N";
    else if (ci == 419) ch = "=";
    else if (ci == 420) ch = hex(k_snap[7:4]);
    else if (ci == 421) ch = hex(k_snap[3:0]);
    else if (ci == 422) ch = 8'h0d;
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
        if (ci == 423) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
