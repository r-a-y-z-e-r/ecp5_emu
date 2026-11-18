module top(input clk, gsr, reset, we, input [3:0] ra,wa,d,
    output q_reset,q_set,q_nogsr,output [3:0] qa);
GSR global_reset(.GSR(gsr));
TRELLIS_FF #(.GSR("ENABLED"),.REGSET("RESET"),.SRMODE("ASYNC")) r0
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[0]),.Q(q_reset));
TRELLIS_FF #(.GSR("ENABLED"),.REGSET("SET"),.SRMODE("ASYNC")) r1
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[1]),.Q(q_set));
TRELLIS_FF #(.GSR("DISABLED"),.REGSET("RESET"),.SRMODE("ASYNC")) r2
    (.CLK(clk),.LSR(reset),.CE(1'b1),.DI(d[2]),.Q(q_nogsr));
(* ram_style="distributed" *) reg [3:0] mem [0:15];
integer i; initial for(i=0;i<16;i=i+1) mem[i]=(3*i+5)&15;
always @(posedge clk) if(we) mem[wa]<=d;
assign qa=mem[ra];
endmodule
