// HS01 slice: do flip-flops take their REGSET value at configuration?
// led[7]=1 and led[6]=0 are constants (polarity and reading direction).
// led[5:3]: Lattice primitives with SLICE GSR enabled, init 3'b101.
// led[2:0]: inferred registers (nextpnr leaves SLICE GSR disabled), init 3'b110.
// All six flip-flops hold their value unless FIRE1 (btn[1]) is pressed.
module top(input clk_25mhz, input [6:0] btn, output [7:0] led);
  wire en = btn[1];
  wire [2:0] g;
  FD1P3AY #(.GSR("ENABLED")) g2 (.D(~g[2]), .SP(en), .CK(clk_25mhz), .Q(g[2]));
  FD1P3AX #(.GSR("ENABLED")) g1 (.D(~g[1]), .SP(en), .CK(clk_25mhz), .Q(g[1]));
  FD1P3AY #(.GSR("ENABLED")) g0 (.D(~g[0]), .SP(en), .CK(clk_25mhz), .Q(g[0]));
  reg [2:0] r = 3'b110;
  always @(posedge clk_25mhz) if (en) r <= ~r;
  assign led = {1'b1, 1'b0, g, r};
endmodule
