// HI08 slice: the dynamic input delay DELAYF (I06): LOADN, MOVE, DIRECTION
// and CFLAG. The delay itself (taps of tens of ps) is not visible at 25 MHz;
// the control is.
// Two DELAYF on the button inputs (FIRE1 R1, FIRE2 T1), DEL_VALUE 10 and
// 120, share LOADN, MOVE and DIRECTION from fabric registers. Their Z go to
// fabric registers (kept), their CFLAG are sampled at 25 MHz.
// Program (one step a 25 MHz cycle, all controls change between steps):
//   phase 0: LOADN low for 4 cycles, then high;
//   phase 1: DIRECTION 0, 140 MOVE pulses (1 cycle high, 3 low);
//   phase 2: LOADN low 4 cycles, high; DIRECTION 1, 140 pulses;
//   phase 3: LOADN low 4, high; DIRECTION 0, MOVE held high 32 cycles, low 8,
//            then DIRECTION 1, MOVE held high 32, low 8;
//   phase 4: LOADN low 4, high; DIRECTION 0, 20 pulses of 2 cycles high, 2 low;
//            then LOADN low while 8 more pulses come (LOADN over MOVE).
// Sample = {loadn, move, direction, cflag1, cflag0} each cycle of the
// program (2048 cycles), the first in the MSBs.
// Line "D=<2560 hex>": 2048 samples of 5 bits as 10240 bits.
module top(input clk_25mhz, input [1:0] btn, output ftdi_rxd, output [1:0] led);
  reg loadn = 1, move = 0, direction = 0;
  wire z0, z1, c0, c1;
  DELAYF #(.DEL_MODE("USER_DEFINED"), .DEL_VALUE(10))
    d0(.A(btn[0]), .LOADN(loadn), .MOVE(move), .DIRECTION(direction), .Z(z0), .CFLAG(c0));
  DELAYF #(.DEL_MODE("USER_DEFINED"), .DEL_VALUE(120))
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
  // Phase boundaries (in steps).
  localparam P1 = 8, P2 = P1 + 560, P3 = P2 + 8 + 560, P4 = P3 + 8 + 80, P5 = P4 + 8 + 80 + 40;
  always @(posedge clk_25mhz) if (run) begin
    if (u < 4) loadn <= 0;
    else if (u < P1) loadn <= 1;
    else if (u < P2) begin direction <= 0; move <= (u - P1) % 4 == 0; end
    else if (u < P2 + 4) begin loadn <= 0; move <= 0; end
    else if (u < P2 + 8) loadn <= 1;
    else if (u < P3) begin direction <= 1; move <= (u - P2 - 8) % 4 == 0; end
    else if (u < P3 + 4) begin loadn <= 0; move <= 0; end
    else if (u < P3 + 8) begin loadn <= 1; direction <= 0; end
    else if (u < P3 + 8 + 32) move <= 1;
    else if (u < P3 + 8 + 40) move <= 0;
    else if (u < P3 + 8 + 41) direction <= 1;
    else if (u < P3 + 8 + 73) move <= 1;
    else if (u < P4) move <= 0;
    else if (u < P4 + 4) loadn <= 0;
    else if (u < P4 + 8) begin loadn <= 1; direction <= 0; end
    else if (u < P4 + 88) move <= (u - P4 - 8) % 4 < 2;
    else if (u < P4 + 120) begin loadn <= 0; move <= (u - P4 - 88) % 4 < 2; end
    else begin loadn <= 1; move <= 0; end
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
