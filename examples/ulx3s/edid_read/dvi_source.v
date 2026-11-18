// 640x480 DVI, eight 80-pixel colour bars, 800x525 at exactly 25 MHz.
// Syncs are negative. The conventional "60 Hz" mode is 59.5238 Hz here.
module dvi_source(input clk_25mhz, output [3:0] gpdi_dp);
  wire shift_clk, pixel_clk, locked;
  pll clocks(.clkin(clk_25mhz), .clkout0(shift_clk), .clkout1(pixel_clk), .locked(locked));
  reg [9:0] x = 0;
  reg [9:0] y = 0;
  always @(posedge pixel_clk)
    if (x == 799) begin
      x <= 0;
      y <= y == 524 ? 0 : y + 1;
    end else x <= x + 1;
  reg [23:0] rgb;
  always @*
    if      (x < 80)  rgb = 24'hffffff;
    else if (x < 160) rgb = 24'hffff00;
    else if (x < 240) rgb = 24'h00ffff;
    else if (x < 320) rgb = 24'h00ff00;
    else if (x < 400) rgb = 24'hff00ff;
    else if (x < 480) rgb = 24'hff0000;
    else if (x < 560) rgb = 24'h0000ff;
    else             rgb = 24'h000000;
  video_out video(.shift_clk(shift_clk), .pixel_clk(pixel_clk), .rgb(rgb),
    .de(x < 640 && y < 480), .hs(!(x >= 656 && x < 752)),
    .vs(!(y >= 490 && y < 492)), .gpdi_dp(gpdi_dp));
endmodule
