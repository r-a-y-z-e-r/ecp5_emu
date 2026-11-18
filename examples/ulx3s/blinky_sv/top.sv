// An 8-bit SystemVerilog counter on the board clock, shown on the LEDs.
module top(input logic clk_25mhz, output logic [7:0] led);
  logic [7:0] count = 0;
  always_ff @(posedge clk_25mhz) count <= count + 1;
  assign led = count;
endmodule
