// UART and filtered buttons feed one ordered 512-command queue. No soft reset:
// bytes received behind 'r' retain their order across the restart marker.
module synth_io(input clk, input rx_pin, output tx_pin, input [6:0] btn,
 input send, input [7:0] data, output busy,
 output [7:0] key, output available, input take, output reg overflow=0);
 reg [9:0] tx_frame=10'h3ff;
 reg [3:0] tx_bits=0;
 reg [7:0] tx_baud=0;
 assign tx_pin=tx_frame[0]; assign busy=tx_bits!=0;
 always @(posedge clk) begin
   if(busy) begin
     if(tx_baud==216) begin tx_baud<=0; tx_frame<={1'b1,tx_frame[9:1]}; tx_bits<=tx_bits-1; end
     else tx_baud<=tx_baud+1;
   end else if(send) begin tx_frame<={1'b1,data,1'b0}; tx_bits<=10; tx_baud<=0; end
 end
 reg [1:0] rx_sync=3;
 reg [3:0] rx_bits=0;
 reg [7:0] rx_baud=0, shift=0, received=0;
 reg push=0;
 always @(posedge clk) begin
   rx_sync<={rx_sync[0],rx_pin}; push<=0;
   if(rx_bits==0) begin if(!rx_sync[1]) begin rx_bits<=1; rx_baud<=108; end end
   else if(rx_baud!=0) rx_baud<=rx_baud-1;
   else begin
     rx_baud<=216;
     if(rx_bits==1) rx_bits<=rx_sync[1]?0:2;
     else if(rx_bits==10) begin if(rx_sync[1]) begin received<=shift; push<=1; end rx_bits<=0; end
     else begin shift<={rx_sync[1],shift[7:1]}; rx_bits<=rx_bits+1; end
   end
 end
 reg [5:0] meta=0, sync=0, stable=0, edges=0, pending=0;
 reg [14:0] count[0:5];
 integer i;
 initial for(i=0;i<6;i=i+1) count[i]=0;
 always @(posedge clk) begin
   meta<=btn[6:1]; sync<=meta; edges<=0;
   for(i=0;i<6;i=i+1) begin
     if(sync[i]==stable[i]) count[i]<=0;
     else if(count[i]==24999) begin
       stable[i]<=sync[i]; count[i]<=0; if(sync[i]) edges[i]<=1;
     end else count[i]<=count[i]+1;
   end
 end
 reg [5:0] chosen;
 reg [7:0] button_key;
 always @* begin
   chosen=0; button_key=0;
   if(pending[0]) begin chosen=1; button_key="a"; end
   else if(pending[1]) begin chosen=2; button_key="s"; end
   else if(pending[2]) begin chosen=4; button_key="d"; end
   else if(pending[3]) begin chosen=8; button_key="f"; end
   else if(pending[4]) begin chosen=16; button_key="g"; end
   else if(pending[5]) begin chosen=32; button_key="h"; end
 end
 (* ram_style="distributed" *) reg [7:0] fifo[0:511];
 reg [9:0] wr=0, rd=0;
 integer j;
 initial for(j=0;j<512;j=j+1) fifo[j]=0;
 assign available=wr!=rd; assign key=fifo[rd[8:0]];
 wire full=(wr-rd)==512;
 wire writing=push || |chosen;
 always @(posedge clk) begin
   pending<=pending | edges;
   if(writing && !full) begin
     fifo[wr[8:0]]<=push?received:button_key; wr<=wr+1;
     if(!push) pending<=(pending & ~chosen) | edges;
   end
   if((writing && full) || |(edges & pending)) overflow<=1; // Explicit fault; never silently overwrites data.
   if(take && available) rd<=rd+1;
 end
endmodule
