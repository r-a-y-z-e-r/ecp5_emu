// A debounced push button. fire1 (FIRE1, the ULX3S's btn[1]) bounces when it is
// pressed or released. The input is synchronised to the clock, and the
// debounced level follows it only after it has stayed at the new level for
// 65536 cycles (2.6 ms). The LEDs count the presses.
// The scalar fire1 port exposes only FIRE1 and can be addressed as net:fire1.
module top(input clk_25mhz, input fire1, output [7:0] led);
  reg [1:0] sync = 0;
  always @(posedge clk_25mhz) sync <= {sync[0], fire1};

  reg level = 0;
  reg [15:0] stable = 0;
  always @(posedge clk_25mhz)
    if (sync[1] == level)
      stable <= 0;
    else if (stable == 16'hffff) begin
      level <= sync[1];
      stable <= 0;
    end else
      stable <= stable + 1;

  reg last = 0;
  reg [7:0] presses = 0;
  always @(posedge clk_25mhz) begin
    last <= level;
    if (level && !last) presses <= presses + 1;
  end
  assign led = presses;
endmodule
