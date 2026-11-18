// HI04b slice: the edge-clock controls (questions C04/C05/I03): ECLKSYNCB STOP,
// CLKDIVF RST and ALIGNWD, CLKDIVF /3.5, and the dividers' start after
// configuration.
// eclk_in = 25 MHz / 8 from the fabric (nextpnr --allow-fabric-eclk), high in
// phases 4..7 of div. A CLKDIVF takes only one of the two edge clocks of a
// bank (CLKDIV0 ECLK0, CLKDIV1 ECLK1), so eclk_in goes through two ECLKSYNCB
// with the same STOP: es to eclk for cd2 (/2), es35 to eclk35 for cd35
// (/3.5). Flip-flops toggle on the rising (p2, p35) and on the
// falling (n2, n35) edges of each divider output. An ODDRX2F on SCLK = cd2,
// with data 1,0,1,0, puts ECLKO itself on led E2 (read back through the BB).
// Sample = {eclk_in, pad, p2, n2, p35, n35}, taken at 25 MHz.
// Line "E<2 hex> <384 hex>": 256 samples, the first in the MSBs.
//   E00: samples t = 0..255 after the start (t counts 25 MHz cycles).
//   E01..E26: a window of 256 samples that starts with div = 0 (sample u
//     has div = u mod 8); the experiment table below says which control is
//     high from which sample, never changing with an edge of eclk_in.
// HI04b: the second design of the series, with the table of experiments.
module top(input clk_25mhz, output ftdi_rxd, inout led_e2);
  reg [2:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire eclk_in = div[2];
  reg stop = 0, rst2 = 0, rst35 = 0, al2 = 0, al35 = 0;
  wire eclk, eclk35, sclk2, sclk35;
  ECLKSYNCB es(.ECLKI(eclk_in), .STOP(stop), .ECLKO(eclk));
  ECLKSYNCB es35(.ECLKI(eclk_in), .STOP(stop), .ECLKO(eclk35));
  CLKDIVF #(.DIV("2.0")) cd2(.CLKI(eclk), .RST(rst2), .ALIGNWD(al2), .CDIVX(sclk2));
  CLKDIVF #(.DIV("3.5")) cd35(.CLKI(eclk35), .RST(rst35), .ALIGNWD(al35), .CDIVX(sclk35));
  reg p2 = 0, n2 = 0, p35 = 0, n35 = 0;
  always @(posedge sclk2) p2 <= ~p2;
  always @(negedge sclk2) n2 <= ~n2;
  always @(posedge sclk35) p35 <= ~p35;
  always @(negedge sclk35) n35 <= ~n35;
  wire oddr_q, pad;
  ODDRX2F oddr(.SCLK(sclk2), .ECLK(eclk), .RST(1'b0),
               .D0(1'b1), .D1(1'b0), .D2(1'b1), .D3(1'b0), .Q(oddr_q));
  BB pad0(.I(oddr_q), .T(1'b0), .O(pad), .B(led_e2));
  wire [5:0] s = {eclk_in, pad, p2, n2, p35, n35};

  // Start-up record.
  reg [8:0] t = 0;
  always @(posedge clk_25mhz) if (t != 9'h1ff) t <= t + 1;
  reg [1535:0] rec0 = 0;
  always @(posedge clk_25mhz) if (t < 256) rec0 <= {rec0[1529:0], s};

  // Experiments: kind 1 STOP, 2 cd2 RST, 3 cd35 RST, 4 cd2 ALIGNWD,
  // 5 cd35 ALIGNWD; pre = 1 pulses that divider's RST at sample 10 first, so
  // its phase is known; the control is high from sample a for dur samples.
  reg [5:0] e = 0;
  reg [2:0] kind;
  reg pre;
  reg [7:0] a;
  reg [6:0] dur;
  always @* begin
    kind = 0; pre = 0; a = 0; dur = 1;
    case (e)
      // STOP pulses shorter than the synchronizer.
      1: begin kind = 1; a = 34; dur = 8; end
      2: begin kind = 1; a = 34; dur = 16; end
      3: begin kind = 1; a = 38; dur = 8; end
      // RST at every phase that does not meet an ECLK edge.
      4: begin kind = 2; a = 33; end
      5: begin kind = 2; a = 34; end
      6: begin kind = 2; a = 35; end
      7: begin kind = 2; a = 37; end
      8: begin kind = 2; a = 38; end
      9: begin kind = 2; a = 39; end
      10: begin kind = 3; a = 33; end
      11: begin kind = 3; a = 34; end
      12: begin kind = 3; a = 35; end
      13: begin kind = 3; a = 37; end
      14: begin kind = 3; a = 38; end
      15: begin kind = 3; a = 39; end
      // cd2 ALIGNWD after a reset: one ECLK period, every position over
      // four ECLK periods; then two ECLK periods.
      16: begin kind = 4; pre = 1; a = 66; dur = 8; end
      17: begin kind = 4; pre = 1; a = 70; dur = 8; end
      18: begin kind = 4; pre = 1; a = 74; dur = 8; end
      19: begin kind = 4; pre = 1; a = 78; dur = 8; end
      20: begin kind = 4; pre = 1; a = 82; dur = 8; end
      21: begin kind = 4; pre = 1; a = 86; dur = 8; end
      22: begin kind = 4; pre = 1; a = 90; dur = 8; end
      23: begin kind = 4; pre = 1; a = 94; dur = 8; end
      24: begin kind = 4; pre = 1; a = 66; dur = 16; end
      25: begin kind = 4; pre = 1; a = 74; dur = 16; end
      26: begin kind = 4; pre = 1; a = 82; dur = 16; end
      27: begin kind = 4; pre = 1; a = 90; dur = 16; end
      // cd35 ALIGNWD after a reset: one ECLK period at every ECLK half of
      // one CDIVX cycle; then two ECLK periods.
      28: begin kind = 5; pre = 1; a = 66; dur = 8; end
      29: begin kind = 5; pre = 1; a = 70; dur = 8; end
      30: begin kind = 5; pre = 1; a = 74; dur = 8; end
      31: begin kind = 5; pre = 1; a = 78; dur = 8; end
      32: begin kind = 5; pre = 1; a = 82; dur = 8; end
      33: begin kind = 5; pre = 1; a = 86; dur = 8; end
      34: begin kind = 5; pre = 1; a = 90; dur = 8; end
      35: begin kind = 5; pre = 1; a = 66; dur = 16; end
      36: begin kind = 5; pre = 1; a = 70; dur = 16; end
      37: begin kind = 5; pre = 1; a = 74; dur = 16; end
      38: begin kind = 5; pre = 1; a = 78; dur = 16; end
    endcase
  end
  localparam LAST = 38;
  localparam BOOT = 0, ALIGN = 1, REC = 2, SEND = 3, PAUSE = 4;
  reg [2:0] state = BOOT;
  reg [8:0] u = 0;
  reg [1535:0] rec = 0;
  // Sample u is taken before edge u + 1, so a control set at edge u = a - 1
  // is high from sample a.
  wire on = u + 1 == a;
  wire off = u + 1 == a + dur;
  wire pre_on = pre && u + 1 == 10;
  wire pre_off = pre && u + 1 == 11;
  // UART: "E" 2 hex " " 384 hex CR LF, 115200 8N1.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  wire [1535:0] src = e == 0 ? rec0 : rec;
  reg [8:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "E";
    else if (ci == 1) ch = hex({2'b0, e[5:4]});
    else if (ci == 2) ch = hex(e[3:0]);
    else if (ci == 3) ch = " ";
    else if (ci < 388) ch = hex(src[1535 - 4 * (ci - 4) -: 4]);
    else if (ci == 388) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [21:0] pause = 0;
  always @(posedge clk_25mhz) begin
    case (state)
      BOOT: if (t == 9'h1ff) state <= SEND;
      ALIGN: if (div == 7) begin state <= REC; u <= 0; end
      REC: begin
        rec <= {rec[1529:0], s};
        u <= u + 1;
        if (on || off) begin
          stop <= on && kind == 1;
          al2 <= on && kind == 4;
          al35 <= on && kind == 5;
        end
        if (on || off || pre_on || pre_off) begin
          rst2 <= (on && kind == 2) || (pre_on && kind == 4);
          rst35 <= (on && kind == 3) || (pre_on && kind == 5);
        end
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
            if (ci == 389) begin
              ci <= 0;
              if (e == LAST) begin e <= 0; state <= PAUSE; pause <= 22'd2500000; end
              else begin e <= e + 1; state <= ALIGN; end
            end else ci <= ci + 1;
          end
        end else baud <= baud + 1;
      PAUSE: if (pause != 0) pause <= pause - 1; else state <= SEND;
    endcase
  end
  assign ftdi_rxd = tx;
endmodule
