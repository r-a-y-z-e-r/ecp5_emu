package pkg;
  localparam int W = 8;
endpackage
module top(input logic clk_25mhz, output logic [7:0] led);
  import pkg::*;
  logic [W-1:0] count = '0;
  always_ff @(posedge clk_25mhz) count <= count + 1'b1;
  assign led = count;
endmodule
