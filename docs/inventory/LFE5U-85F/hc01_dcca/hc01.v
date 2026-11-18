// HC01 slice (C02): when does a DCCA's CE stop and restart its output?
// CLKI is a slow clock made in the fabric: a period of 8 cycles of the 25 MHz
// board clock, high in phases 0..3. For each experiment the bench sets CE to
// its start value, waits 8 periods, then changes CE at phase K (1, 2, 3, 5, 6
// or 7: never with an edge of CLKI). Two flip-flops toggle on the rising and
// on the falling edges of CLKO; the 25 MHz clock samples them, so each bit
// of the masks below is one 25 MHz cycle, from the cycle CE changed in.
// Line per experiment (UART 115200 8N1 on ftdi_rxd), all hex:
//   "E<n> K<phase> D<0 stop, 1 start> R<48-bit rising mask> F<48-bit falling mask>"
// The program repeats forever.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  reg [2:0] ph = 0;
  always @(posedge clk_25mhz) ph <= ph + 1;
  reg ck = 0;
  always @(posedge clk_25mhz)
    if (ph == 7) ck <= 1;
    else if (ph == 3) ck <= 0;

  wire gclk;
  reg ce = 1;
  DCCA dcc(.CLKI(ck), .CE(ce), .CLKO(gclk));
  reg tp = 0, tn = 0;
  always @(posedge gclk) tp <= ~tp;
  always @(negedge gclk) tn <= ~tn;

  // Experiments: 6 phases x stop/start.
  reg [3:0] e = 0;
  reg [2:0] k;
  always @* case (e % 6)
    0: k = 1; 1: k = 2; 2: k = 3; 3: k = 5; 4: k = 6; default: k = 7;
  endcase
  wire start = e >= 6;

  // States: 0 settle, 1 wait for phase, 2 capture, 3 print.
  reg [1:0] state = 0;
  reg [7:0] n = 0;
  reg tp_d = 0, tn_d = 0;
  reg [47:0] rmask = 0, fmask = 0;
  reg [5:0] ci = 0;

  // UART.
  function [7:0] hex(input [3:0] d);
    hex = d < 10 ? 8'h30 + d : 8'h37 + d;
  endfunction
  reg [7:0] ch;
  always @* begin
    ch = " ";
    case (ci)
      0: ch = "E"; 1: ch = hex(e); 2: ch = " "; 3: ch = "K"; 4: ch = hex({1'b0, k});
      5: ch = " "; 6: ch = "D"; 7: ch = hex({3'b0, start}); 8: ch = " "; 9: ch = "R";
      22: ch = " "; 23: ch = "F"; 36: ch = 8'h0d; 37: ch = 8'h0a;
      default:
        if (ci >= 10 && ci <= 21) ch = hex(rmask[(21 - ci) * 4 +: 4]);
        else if (ci >= 24 && ci <= 35) ch = hex(fmask[(35 - ci) * 4 +: 4]);
    endcase
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;

  always @(posedge clk_25mhz) begin
    tp_d <= tp;
    tn_d <= tn;
    case (state)
      0: begin
        ce <= ~start;
        n <= n + 1;
        if (n == 64) begin n <= 0; state <= 1; end
      end
      1: if (ph == k - 1) begin   // CE changes at the edge that starts phase k
        ce <= start;
        state <= 2;
        n <= 0;
      end
      2: begin
        rmask <= {rmask[46:0], tp ^ tp_d};
        fmask <= {fmask[46:0], tn ^ tn_d};
        n <= n + 1;
        if (n == 47) begin state <= 3; ci <= 0; end
      end
      3:
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
            if (ci == 37) begin
              state <= 0; n <= 0;
              e <= e == 11 ? 0 : e + 1;
            end else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  end
  assign ftdi_rxd = tx;
  assign led = {e, 4'b0};
endmodule
