// HI02b slice: IOLOGIC start-up state, asynchronous reset, and IO registers.
// sclk = 25 MHz / 16 from fabric (global). A 6-bit sample is taken on every
// 25 MHz edge in two windows, t = 0..127 and t = 1008..1263:
//   {oddr pad, iddr Q0, iddr Q1, oreg pad, tsreg pad, ireg Q}
//   ODDRX1F on led0 (B2), readback through its BB; RST = rst.
//   IDDRX1F on led4 (D1), pad driven by fabric LFSR lw; RST = rst.
//   OREG (syn_useioff) on led3 (D2): init 1, async set by rst, data LFSR lc.
//   TSREG (syn_useioff) on led7 (H3): init 1, T = tq, I = 1, PULLMODE=DOWN.
//   IREG (syn_useioff) on led5 (E2): init 1, pad driven by fabric LFSR lv.
// rst is high for samples t = 1028..1099 (set mid low phase, cleared mid high).
// Line: "S=" + 576 hex (first sample in the MSBs) + CR LF, 115200 8N1.
module top(input clk_25mhz, output ftdi_rxd,
           inout led0, inout led3, inout led4, inout led5, inout led7);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire sclk = div[3];
  reg [11:0] t = 0;
  always @(posedge clk_25mhz) if (t != 12'hfff) t <= t + 1;
  reg rst = 0;
  always @(posedge clk_25mhz) rst <= t >= 1027 && t < 1099;

  reg [6:0] la = 7'h01, lb = 7'h55, lc = 7'h2b, ld = 7'h4c;
  always @(posedge sclk) begin
    la <= {la[5:0], la[6] ^ la[5]};
    lb <= {lb[5:0], lb[6] ^ lb[4]};
    lc <= {lc[5:0], lc[6] ^ lc[5]};
    ld <= {ld[5:0], ld[6] ^ ld[4]};
  end
  reg [6:0] lw = 7'h3a, lv = 7'h66;
  always @(posedge clk_25mhz) if (!div[0]) begin
    lw <= {lw[5:0], lw[6] ^ lw[5]};
    lv <= {lv[5:0], lv[6] ^ lv[4]};
  end

  wire oddr_q, oddr_in;
  ODDRX1F oddr(.SCLK(sclk), .RST(rst), .D0(la[0]), .D1(lb[0]), .Q(oddr_q));
  BB p0(.I(oddr_q), .T(1'b0), .O(oddr_in), .B(led0));

  wire iddr_in, q0, q1;
  BB p4(.I(lw[0]), .T(1'b0), .O(iddr_in), .B(led4));
  IDDRX1F iddr(.SCLK(sclk), .RST(rst), .D(iddr_in), .Q0(q0), .Q1(q1));

  (* syn_useioff *) reg oq = 1;
  always @(posedge sclk or posedge rst) if (rst) oq <= 1; else oq <= lc[0];
  wire oreg_in;
  BB p3(.I(oq), .T(1'b0), .O(oreg_in), .B(led3));

  (* syn_useioff *) reg tq = 1;
  always @(posedge sclk) tq <= ld[0];
  wire ts_in;
  BB p7(.I(1'b1), .T(tq), .O(ts_in), .B(led7));

  wire ireg_pad;
  BB p5(.I(lv[0]), .T(1'b0), .O(ireg_pad), .B(led5));
  (* syn_useioff *) reg iq = 1;
  always @(posedge sclk) iq <= ireg_pad;

  wire [5:0] sample = {oddr_in, q0, q1, oreg_in, ts_in, iq};
  wire done = t == 12'hfff;
  wire recording = t < 128 || (t >= 1008 && t < 1264);

  // UART
  reg [9:0] ci = 0;
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [22:0] pause = 0;
  reg [2303:0] rec = 0;
  wire [3:0] top4 = rec[2303:2300];
  wire [7:0] hexch = top4 < 10 ? 8'h30 + top4 : 8'h37 + top4;
  wire [7:0] ch = ci == 0 ? "S" : ci == 1 ? "=" : ci < 578 ? hexch : ci == 578 ? 8'h0d : 8'h0a;
  wire start = !busy && pause == 0 && done;
  always @(posedge clk_25mhz)
    if (recording) rec <= {rec[2297:0], sample};
    else if (start && ci >= 2 && ci < 578) rec <= {rec[2299:0], top4};
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
        if (ci == 579) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
