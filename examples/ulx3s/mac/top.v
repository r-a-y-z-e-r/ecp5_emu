// Multiply-accumulate in the DSP blocks, as a student writes it with `*`:
// yosys maps each product to MULT18X18D.
//   - mac: the signed 16x16 products of two LFSR sequences, summed in 40 bits
//     over 4096 pairs (a dot product);
//   - mix: the low 32 bits of an unsigned 32x32 product (yosys splits it
//     into four MULT18X18D and adds the parts in the fabric), xored together
//     over the same 4096 steps.
// After each pass the UART (115200 8N1) prints
//   "MAC <mac, 10 hex digits> MIX <mix, 8 hex digits>\r\n"
// and the next pass starts from the same seeds, so every line is the same.
// The LEDs show the top byte of mac.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  reg [15:0] x = 16'hACE1, y = 16'h1D2B;
  reg [31:0] u = 32'h9E3779B9, w = 32'h7F4A7C15;
  reg signed [31:0] product = 0;
  reg [31:0] wide = 0;
  reg signed [39:0] mac = 0;
  reg [31:0] mix = 0;
  reg [12:0] n = 0;            // 4096 steps, then 2 to drain the pipeline
  reg printing = 0;

  // UART, 115200 8N1 from 25 MHz.
  function [7:0] hex(input [3:0] d);
    hex = d < 10 ? 8'h30 + d : 8'h37 + d;
  endfunction
  reg [4:0] ci = 0;
  reg [7:0] ch;
  always @* case (ci)
    0: ch = "M"; 1: ch = "A"; 2: ch = "C"; 3: ch = " ";
    4: ch = hex(mac[39:36]); 5: ch = hex(mac[35:32]); 6: ch = hex(mac[31:28]);
    7: ch = hex(mac[27:24]); 8: ch = hex(mac[23:20]); 9: ch = hex(mac[19:16]);
    10: ch = hex(mac[15:12]); 11: ch = hex(mac[11:8]); 12: ch = hex(mac[7:4]);
    13: ch = hex(mac[3:0]);
    14: ch = " "; 15: ch = "M"; 16: ch = "I"; 17: ch = "X"; 18: ch = " ";
    19: ch = hex(mix[31:28]); 20: ch = hex(mix[27:24]); 21: ch = hex(mix[23:20]);
    22: ch = hex(mix[19:16]); 23: ch = hex(mix[15:12]); 24: ch = hex(mix[11:8]);
    25: ch = hex(mix[7:4]); 26: ch = hex(mix[3:0]);
    27: ch = 8'h0d; default: ch = 8'h0a;
  endcase
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;

  always @(posedge clk_25mhz)
    if (!printing) begin
      // Stage 1: the products of the current operands; stage 2: accumulate.
      if (n < 4096) begin
        x <= {x[14:0], x[15] ^ x[13] ^ x[12] ^ x[10]};
        y <= {y[14:0], y[15] ^ y[14] ^ y[12] ^ y[3]};
        u <= u * 32'd1664525 + 32'd1013904223;
        w <= {w[30:0], w[31] ^ w[21] ^ w[1] ^ w[0]};
        product <= $signed(x) * $signed(y);
        wide <= u * w;
      end
      if (n >= 1 && n <= 4096) begin
        mac <= mac + product;
        mix <= mix ^ wide;
      end
      n <= n + 1;
      if (n == 4097) begin printing <= 1; ci <= 0; end
    end else if (!busy) begin
      shift <= {1'b1, ch, 1'b0};
      busy <= 1; baud <= 0; bitn <= 0;
    end else if (baud == 216) begin
      baud <= 0;
      tx <= shift[0];
      shift <= {1'b1, shift[9:1]};
      bitn <= bitn + 1;
      if (bitn == 9) begin
        busy <= 0;
        if (ci == 28) begin
          printing <= 0; n <= 0; mac <= 0; mix <= 0;
          x <= 16'hACE1; y <= 16'h1D2B; u <= 32'h9E3779B9; w <= 32'h7F4A7C15;
        end else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
  assign led = mac[39:32];
endmodule
