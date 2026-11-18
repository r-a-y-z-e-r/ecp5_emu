// HI06 slice: the edge-clock bridge ECLKBRIDGECS and its synchronizer
// BRGECLKSYNC (an ECLKSYNCB placed after the bridge).
// Two edge clocks from the fabric (nextpnr --allow-fabric-eclk): c0 = div[2]
// (period 8, high in phases 4..7 of div) and c1 = div[3] (period 16, high in
// 8..15). The bridge selects c0 (SEL 0) or c1 (SEL 1); its output goes
// through ECLKSYNCB (STOP) to eclk, and CLKDIVF /2 makes sclk. Flip-flops
// toggle on the rising (p2) and falling (n2) edges of sclk; an ODDRX2F on
// sclk with data 1,0,1,0 puts eclk itself on LED E2 (read back through the
// BB). Sample = {c0, c1, sel, pad, p2, n2} at 25 MHz.
// Line "E<2 hex> <384 hex>": 256 samples, the first in the MSBs.
//   E00: samples t = 0..255 after the start.
//   E01..E07: a window of 256 samples that starts with div = 0 (sample u has
//     div = u mod 16); a control is high from sample a for dur samples:
//     E01-E04: SEL from a = 34, 38, 42, 46 for 68 samples, so it switches
//       in each of the four (c0, c1) states and switches back in another;
//     E05-E07: STOP from (34, 8), (34, 64), (38, 16).
module top(input clk_25mhz, output ftdi_rxd, inout led_e2);
  reg [3:0] div = 0;
  always @(posedge clk_25mhz) div <= div + 1;
  wire c0 = div[2], c1 = div[3];
  reg stop = 0, sel = 0;
  wire br, eclk, sclk2;
  ECLKBRIDGECS bridge(.CLK0(c0), .CLK1(c1), .SEL(sel), .ECSOUT(br));
  ECLKSYNCB bs(.ECLKI(br), .STOP(stop), .ECLKO(eclk));
  CLKDIVF #(.DIV("2.0")) cd2(.CLKI(eclk), .RST(1'b0), .ALIGNWD(1'b0), .CDIVX(sclk2));
  reg p2 = 0, n2 = 0;
  always @(posedge sclk2) p2 <= ~p2;
  always @(negedge sclk2) n2 <= ~n2;
  wire oddr_q, pad;
  ODDRX2F oddr(.SCLK(sclk2), .ECLK(eclk), .RST(1'b0),
               .D0(1'b1), .D1(1'b0), .D2(1'b1), .D3(1'b0), .Q(oddr_q));
  BB pad0(.I(oddr_q), .T(1'b0), .O(pad), .B(led_e2));
  wire [5:0] s = {c0, c1, sel, pad, p2, n2};

  // Start-up record.
  reg [8:0] t = 0;
  always @(posedge clk_25mhz) if (t != 9'h1ff) t <= t + 1;
  reg [1535:0] rec0 = 0;
  always @(posedge clk_25mhz) if (t < 256) rec0 <= {rec0[1529:0], s};

  // Experiments: kind 1 STOP, 2 SEL; the control is high from sample a
  // for dur samples.
  reg [5:0] e = 0;
  reg [2:0] kind;
  reg pre;
  reg [7:0] a;
  reg [6:0] dur;
  always @* begin
    kind = 0; pre = 0; a = 0; dur = 1;
    case (e)
      1: begin kind = 2; a = 34; dur = 68; end
      2: begin kind = 2; a = 38; dur = 68; end
      3: begin kind = 2; a = 42; dur = 68; end
      4: begin kind = 2; a = 46; dur = 68; end
      5: begin kind = 1; a = 34; dur = 8; end
      6: begin kind = 1; a = 34; dur = 64; end
      7: begin kind = 1; a = 38; dur = 16; end
    endcase
  end
  localparam LAST = 7;
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
      ALIGN: if (div == 15) begin state <= REC; u <= 0; end
      REC: begin
        rec <= {rec[1529:0], s};
        u <= u + 1;
        if (on || off) begin
          stop <= on && kind == 1;
          sel <= on && kind == 2;
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
