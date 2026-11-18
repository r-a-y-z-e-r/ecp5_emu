module top(input clk,we,input [3:0] a,input [3:0] d,output [3:0] q);
  reg [3:0] m[0:15];
  integer i;
  initial for(i=0;i<16;i=i+1)m[i]=0;
  always @(posedge clk) if(we)m[a]<=d;
  assign q=m[a];
endmodule
