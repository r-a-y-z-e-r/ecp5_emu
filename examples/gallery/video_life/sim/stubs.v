// Testbench replacements for the PLL and the TMDS output path, so Verilator
// (which has no EHXPLLL or ODDRX1F models) can simulate the same top.v.
// The UART text depends only on logic in the 25 MHz pixel domain, and on the
// chip CLKOS is that 25 MHz in phase with the board clock, so the model PLL
// makes the pixel clock the input clock itself. The shift clock, the TMDS
// encoders and ODDRX1F never change a UART byte — the CRC is computed in
// top.v from the pixel-domain RGB, not from the pads — so video_out is a sink.
module pll(input clkin, output clkout0, output clkout1, output locked);
  assign clkout0 = clkin;
  assign clkout1 = clkin;
  assign locked = 1'b1;
endmodule
module video_out(input shift_clk, input pixel_clk, input [23:0] rgb,
                 input de, input hs, input vs, output [3:0] gpdi_dp);
  assign gpdi_dp = 4'b0000;
endmodule
