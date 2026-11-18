// Blinky through a PLL. ecppll makes 100 MHz from the 25 MHz board clock
// (pll.v: ecppll -i 25 -o 100 -n pll -f pll.v), and a 32-bit counter shows
// its top byte on the LEDs: led[0] toggles about three times per second.
module top(input clk_25mhz, output [7:0] led);
  wire clk, locked;
  pll pll_i(.clkin(clk_25mhz), .clkout0(clk), .locked(locked));
  reg [31:0] count = 0;
  always @(posedge clk)
    if (locked) count <= count + 1;
  assign led = count[31:24];
endmodule
