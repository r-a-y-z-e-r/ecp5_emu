// HI08c slice (the HI08b circuit, a third program): the dynamic input delay DELAYF (I06): LOADN, MOVE, DIRECTION
// and CFLAG. The delay itself (taps of tens of ps) is not visible at 25 MHz;
// the control is.
// Two DELAYF on the button inputs (FIRE1 R1, FIRE2 T1), DEL_VALUE 0 and
// 127, share LOADN, MOVE and DIRECTION from fabric registers. Their Z go to
// fabric registers (kept), their CFLAG are sampled at 25 MHz.
// Program (one step a 25 MHz cycle):
//   E: no LOADN first: DIRECTION 1, 140 MOVE pulses (1 high, 3 low) from the
//      start values after configuration;
//   F: LOADN low 4, high; DIRECTION 0; MOVE high for 12 cycles; DIRECTION 1
//      from its 3rd to its 6th cycle; then 8 low;
//   G: LOADN low 4, high; DIRECTION 1; one pulse (1 high, 3 low); DIRECTION
//      0; MOVE high for 12 cycles, then low;
//   H: LOADN low 4, high; DIRECTION 0; MOVE high for 12 cycles at the limit
//      (DEL_VALUE 127), then low.
// Sample = {loadn, move, direction, cflag1, cflag0} each cycle of the
// program (2048 cycles), the first in the MSBs.
// Line "D=<2560 hex>": 2048 samples of 5 bits as 10240 bits.
module top(input clk_25mhz, input [1:0] btn, output ftdi_rxd, output [1:0] led);
  reg loadn = 1, move = 0, direction = 0;
  wire z0, z1, c0, c1;
  DELAYF #(.DEL_MODE("USER_DEFINED"), .DEL_VALUE(0))
    d0(.A(btn[0]), .LOADN(loadn), .MOVE(move), .DIRECTION(direction), .Z(z0), .CFLAG(c0));
  DELAYF #(.DEL_MODE("USER_DEFINED"), .DEL_VALUE(127))
    d1(.A(btn[1]), .LOADN(loadn), .MOVE(move), .DIRECTION(direction), .Z(z1), .CFLAG(c1));
  reg [1:0] zr = 0;
  always @(posedge clk_25mhz) zr <= {z1, z0};
  assign led = zr;

  reg [11:0] u = 0;           // program step
  reg run = 0;
  reg [15:0] wait0 = 0;
  always @(posedge clk_25mhz) begin
    if (!run) begin
      wait0 <= wait0 + 1;
      if (wait0 == 16'hffff) run <= 1;
    end else if (u != 12'hfff) u <= u + 1;
  end
  // Experiment starts (in steps).
  localparam F = 600, G = 640, H = 680;
  always @(posedge clk_25mhz) if (run) begin
    if (u < 8) direction <= 1;
    else if (u < 8 + 560) move <= (u - 8) % 4 == 0;
    else if (u < F) move <= 0;
    else if (u < F + 4) loadn <= 0;
    else if (u < F + 8) begin loadn <= 1; direction <= 0; end
    else if (u < F + 20) begin move <= 1; direction <= (u >= F + 10 && u < F + 14); end
    else if (u < G) begin move <= 0; direction <= 0; end
    else if (u < G + 4) loadn <= 0;
    else if (u < G + 8) begin loadn <= 1; direction <= 1; end
    else if (u < G + 9) move <= 1;
    else if (u < G + 12) move <= 0;
    else if (u < G + 13) direction <= 0;
    else if (u < G + 25) move <= 1;
    else if (u < H) move <= 0;
    else if (u < H + 4) loadn <= 0;
    else if (u < H + 8) begin loadn <= 1; direction <= 0; end
    else if (u < H + 20) move <= 1;
    else move <= 0;
  end
  reg [10239:0] rec = 0;
  always @(posedge clk_25mhz) if (run && u < 2048) rec <= {rec[10234:0], loadn, move, direction, c1, c0};
  wire done = u == 12'hfff;

  // UART: "D=" 2560 hex CR LF, 115200 8N1, repeated.
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [11:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "D";
    else if (ci == 1) ch = "=";
    else if (ci < 2562) ch = hex(rec[10239 - 4 * (ci - 2) -: 4]);
    else if (ci == 2562) ch = 8'h0d;
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
        if (ci == 2563) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
endmodule
