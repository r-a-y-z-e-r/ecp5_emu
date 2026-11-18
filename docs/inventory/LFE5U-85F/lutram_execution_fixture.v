module top(input clk_a, clk_b, we, reset, input [3:0] ra, wa, di,
    output [3:0] qa, qb, output reg [3:0] captured);
(* ram_style="distributed" *) reg [3:0] mem_a [0:15];
(* ram_style="distributed" *) reg [3:0] mem_b [0:15];
integer i;
initial for (i=0;i<16;i=i+1) begin mem_a[i]=(3*i+5)&15; mem_b[i]=(5*i+9)&15; end
always @(posedge clk_a) if (we) mem_a[wa] <= di;
always @(negedge clk_b) if (we) mem_b[wa] <= ~di;
assign qa=mem_a[ra];
assign qb=mem_b[ra];
always @(posedge clk_a or posedge reset) if (reset) captured <= 0; else captured <= mem_a[ra];
endmodule
