// Both clocks come from the same PLL. phase=0 loads the registered pixel
// word after the coincident pixel edge; four more shifts send its other pairs.
// ODDRX1F adds the same two-rise delay to every lane, including the clock.
module video_out(input shift_clk, input pixel_clk, input [23:0] rgb,
                 input de, input hs, input vs, output [3:0] gpdi_dp);
  wire [9:0] blue, green, red;
  tmds b(.clk(pixel_clk), .data(rgb[7:0]), .de(de), .control({vs,hs}), .word(blue));
  tmds g(.clk(pixel_clk), .data(rgb[15:8]), .de(de), .control(2'b00), .word(green));
  tmds r(.clk(pixel_clk), .data(rgb[23:16]), .de(de), .control(2'b00), .word(red));
  reg [2:0] phase = 0;
  always @(posedge shift_clk) phase <= phase == 4 ? 0 : phase + 1;
  genvar lane;
  generate for (lane=0; lane<4; lane=lane+1) begin: ser
    wire [9:0] word = lane == 0 ? blue : lane == 1 ? green : lane == 2 ? red : 10'b0000011111;
    reg [9:0] bits = 0;
    always @(posedge shift_clk)
      if (phase == 0) bits <= word;
      else bits <= {2'b00, bits[9:2]};
    ODDRX1F oddr(.SCLK(shift_clk), .RST(1'b0), .D0(bits[0]), .D1(bits[1]), .Q(gpdi_dp[lane]));
  end endgenerate
endmodule
