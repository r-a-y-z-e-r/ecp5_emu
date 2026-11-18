module top(input clk_a, clk_b, reset, output reg [7:0] a, output reg [7:0] b);
always @(posedge clk_a or posedge reset) if (reset) a <= 0; else a <= a + 1'b1;
always @(negedge clk_b or posedge reset) if (reset) b <= 0; else b <= b + 8'd3;
endmodule
