// Only the second stage may feed application logic. Preserve both stages.
module cdc_sync #(parameter WIDTH=1)(input clk, input reset,
 input [WIDTH-1:0] d,
 (* ASYNC_REG="TRUE", syn_preserve=1 *) output reg [WIDTH-1:0] q=0);
 (* ASYNC_REG="TRUE", syn_preserve=1 *) reg [WIDTH-1:0] first=0;
 always @(posedge clk or posedge reset)
   if (reset) begin first<=0; q<=0; end
   else begin first<=d; q<=first; end
endmodule

module cdc_reset(input clk, input request, output reset);
 (* ASYNC_REG="TRUE", syn_preserve=1 *) reg [1:0] release_pipe=3;
 always @(posedge clk or posedge request)
   if (request) release_pipe<=3;
   else release_pipe<={release_pipe[0],1'b0};
 assign reset=release_pipe[1];
endmodule

// A registered Gray counter changes one output bit per source edge.
module cdc_meter(input clk, input reset, output reg [27:0] gray=0,
 output heartbeat);
 reg [27:0] binary=0;
 wire [27:0] next_binary=binary+1'b1;
 assign heartbeat=binary[22];
 always @(posedge clk or posedge reset)
   if (reset) begin binary<=0; gray<=0; end
   else begin binary<=next_binary; gray<=next_binary^(next_binary>>1); end
endmodule
