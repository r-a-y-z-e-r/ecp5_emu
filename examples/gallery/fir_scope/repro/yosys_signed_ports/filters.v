module filters(input clk, input [17:0] a, b, output [53:0] r, output [17:0] q);
  dsp_mac mac(clk, 1'b1, 1'b0, a, b, r);
  assign q = a[3:0] * b[3:0];
endmodule
