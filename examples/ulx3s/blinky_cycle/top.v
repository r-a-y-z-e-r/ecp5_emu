// Blinky that changes on every clock: an 8-bit counter on the 25 MHz board
// clock, with no PLL, shown on the LEDs. led[0] toggles on every rising
// edge, led[1] on every second one, and so on: at a few cycles per second
// in the emulator each cycle shows on the board and runs along the wires.
module top(input clk_25mhz, output [7:0] led);
  reg [7:0] count = 0;
  always @(posedge clk_25mhz) count <= count + 1;
  assign led = count;
endmodule
