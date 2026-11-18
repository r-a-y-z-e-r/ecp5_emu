// HS01 slice, variant B: same flip-flops and init values as hs01_init.v, but
// clocked by FIRE2 (btn[2]) and enabled by FIRE1 (btn[1]). With no button
// pressed there is no clock edge at all after configuration.
module top(input clk_25mhz, input [6:0] btn, output [7:0] led);
  wire en = btn[1];
  wire ck = btn[2];
  wire [2:0] g;
  FD1P3AY #(.GSR("ENABLED")) g2 (.D(~g[2]), .SP(en), .CK(ck), .Q(g[2]));
  FD1P3AX #(.GSR("ENABLED")) g1 (.D(~g[1]), .SP(en), .CK(ck), .Q(g[1]));
  FD1P3AY #(.GSR("ENABLED")) g0 (.D(~g[0]), .SP(en), .CK(ck), .Q(g[0]));
  reg [2:0] r = 3'b110;
  always @(posedge ck) if (en) r <= ~r;
  assign led = {1'b1, 1'b0, g, r};
endmodule
