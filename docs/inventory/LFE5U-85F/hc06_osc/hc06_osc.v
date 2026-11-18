// HC06 slice: internal oscillator OSCG, frequency and start-up, against the
// 25 MHz reference, reported over the ULX3S FTDI UART.
// Build with -DOSC_DIV=<n>. All counts are hex:
//   E: reference cycles until the first OSC toggle is seen (2-flop sync);
//   S: OSC rising edges while the gate is high in the first window, which
//      starts at reference cycle 0;
//   W: window number; F: OSC rising edges in that window.
// A window is 65536 reference cycles with the gate high, then 65536 low.
// Lines are separated by 5000 idle reference cycles (more than one UART
// frame), so a receiver that opens the port mid-stream can find the frames.
module top(input clk_25mhz, output ftdi_rxd);
  wire osc;
  OSCG #(.DIV(`OSC_DIV)) g(.OSC(osc));

  // OSC domain: a toggle for start-up, an edge counter for the gate.
  reg ot = 0;
  always @(posedge osc) ot <= ~ot;
  reg [1:0] gs = 0;
  reg gd = 0;
  reg [23:0] cnt = 0;
  always @(posedge osc) begin
    gs <= {gs[0], gate};
    gd <= gs[1];
    if (gs[1] && !gd) cnt <= 1;
    else if (gs[1]) cnt <= cnt + 1;
  end

  // Reference domain.
  reg [1:0] os = 0;
  reg od = 0, seen = 0;
  reg [23:0] e = 0;
  always @(posedge clk_25mhz) begin
    os <= {os[0], ot};
    od <= os[1];
    if (!seen) begin
      if (os[1] != od) seen <= 1;
      else if (e != 24'hFFFFFF) e <= e + 1;
    end
  end
  reg [16:0] phase = 0;   // bit 16: gate low half
  reg gate = 1;
  reg [15:0] w = 0;
  reg [23:0] f = 0, s = 0;
  reg first = 1, ready = 0;
  always @(posedge clk_25mhz) begin
    phase <= phase + 1;
    gate <= phase == 17'h1FFFF ? 1 : (phase == 17'h0FFFF ? 0 : gate);
    // Sample in the middle of the low half, long after the count froze.
    if (phase == 17'h18000) begin
      f <= cnt;
      if (first) s <= cnt;
      first <= 0;
      w <= w + 1;
      ready <= 1;
    end
  end

  // UART reporter, 115200 8N1 from 25 MHz.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [23:0] e_l, s_l, f_l;
  reg [15:0] w_l;
  reg [5:0] ci = 0;
  reg [7:0] ch;
  always @* case (ci)
    0: ch = "E"; 1: ch = "=";
    2: ch = hex(e_l[23:20]); 3: ch = hex(e_l[19:16]); 4: ch = hex(e_l[15:12]);
    5: ch = hex(e_l[11:8]); 6: ch = hex(e_l[7:4]); 7: ch = hex(e_l[3:0]);
    8: ch = " "; 9: ch = "S"; 10: ch = "=";
    11: ch = hex(s_l[23:20]); 12: ch = hex(s_l[19:16]); 13: ch = hex(s_l[15:12]);
    14: ch = hex(s_l[11:8]); 15: ch = hex(s_l[7:4]); 16: ch = hex(s_l[3:0]);
    17: ch = " "; 18: ch = "W"; 19: ch = "=";
    20: ch = hex(w_l[15:12]); 21: ch = hex(w_l[11:8]); 22: ch = hex(w_l[7:4]); 23: ch = hex(w_l[3:0]);
    24: ch = " "; 25: ch = "F"; 26: ch = "=";
    27: ch = hex(f_l[23:20]); 28: ch = hex(f_l[19:16]); 29: ch = hex(f_l[15:12]);
    30: ch = hex(f_l[11:8]); 31: ch = hex(f_l[7:4]); 32: ch = hex(f_l[3:0]);
    33: ch = 8'h0d; default: ch = 8'h0a;
  endcase
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [12:0] pause = 0;
  always @(posedge clk_25mhz)
    if (!busy) begin
      if (pause != 0) pause <= pause - 1;
      else if (ready) begin
        if (ci == 0) begin e_l <= e; s_l <= s; w_l <= w; f_l <= f; end
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
        if (ci == 34) begin ci <= 0; pause <= 13'd5000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
