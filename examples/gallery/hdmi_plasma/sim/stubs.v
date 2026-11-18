// Only the device PLL and the TMDS serializer are replaced. The real
// pixel-domain design runs unchanged. This does NOT test pad decoding.
module pll(input clkin, output clkout0, output clkout1, output locked);
  assign clkout0 = clkin;
  assign clkout1 = clkin;
  assign locked = 1;
endmodule
module video_out(input shift_clk, input pixel_clk, input [23:0] rgb,
                 input de, input hs, input vs, output [3:0] gpdi_dp);
  import "DPI-C" function void video_sample(input int unsigned color,
                                            input bit de, hs, vs);
  always @(posedge pixel_clk) video_sample({8'd0,rgb}, de, hs, vs);
  assign gpdi_dp = 0;
endmodule
