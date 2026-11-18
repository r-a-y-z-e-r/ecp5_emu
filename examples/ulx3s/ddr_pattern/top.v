// A DDR output pattern in the style of an HDMI (DVI) transmitter. Each GPDI
// lane is a pseudo-differential LVCMOS33D pair driven by an ODDRX1F, which
// sends two bits per clock: D0 while the clock is high, then D1. Lanes 0-2
// repeat the TMDS control symbol for c1 c0 = 00 (10'b1101010100, bit 0
// first); lane 3, the TMDS clock lane, repeats five ones and five zeros.
// Real DVI runs the serializer from a PLL at five times the pixel clock; here
// the 25 MHz board clock drives it directly, 50 Mbit/s per lane.
module top(input clk_25mhz, output [3:0] gpdi_dp);
  localparam [9:0] CTRL = 10'b1101010100;
  localparam [9:0] CLOCK = 10'b0000011111;
  reg [2:0] phase = 0;  // bit pair 0..4 of the 10-bit symbol
  always @(posedge clk_25mhz) phase <= phase == 4 ? 0 : phase + 1;
  wire [9:0] ctrl_shifted = CTRL >> (2 * phase);
  wire [9:0] clock_shifted = CLOCK >> (2 * phase);
  genvar i;
  generate
    for (i = 0; i < 4; i = i + 1) begin : lane
      wire [1:0] pair = i == 3 ? clock_shifted[1:0] : ctrl_shifted[1:0];
      ODDRX1F oddr(.SCLK(clk_25mhz), .RST(1'b0), .D0(pair[0]), .D1(pair[1]), .Q(gpdi_dp[i]));
    end
  endgenerate
endmodule
