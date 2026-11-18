// HS01 slice, repeat series: runs A and B of hs01_init_slice in one bitstream.
// All eight flip-flops are inferred `reg = init` (nextpnr: SLICE GSR disabled).
// led[7:4]: free-running 25 MHz clock, clock enable FIRE1 (as run A), init 4'b1001.
// led[3:0]: clocked by FIRE2, clock enable FIRE1 (as run B), init 4'b1010.
module top(input clk_25mhz, input [6:0] btn, output [7:0] led);
  wire en = btn[1];
  reg [3:0] a = 4'b1001;
  reg [3:0] b = 4'b1010;
  always @(posedge clk_25mhz) if (en) a <= ~a;
  always @(posedge btn[2]) if (en) b <= ~b;
  assign led = {a, b};
endmodule
