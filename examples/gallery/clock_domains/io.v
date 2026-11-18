// Shared serial plumbing, written for these demos. Application restart does
// not reset this FIFO: keys already received must retain their order.
module gallery_io(input clk, input reset, input rx_pin, output tx_pin,
 input [6:0] btn, output reg [6:0] pressed = 0,
 input send, input [7:0] data, output busy,
 output [7:0] key, output available, input take);
 reg [1:0] sync_rx = 3;
 reg [6:0] sync1 = 1, sync2 = 1, stable = 1;
 reg [17:0] debounce[0:6];
 integer i;
 initial for (i=0;i<7;i=i+1) debounce[i]=0;
 always @(posedge clk) begin
   sync1 <= btn; sync2 <= sync1; pressed <= 0;
   for (i=0;i<7;i=i+1) begin
     if (sync2[i] == stable[i]) debounce[i] <= 0;
     else if (debounce[i] == 249999) begin
       stable[i] <= sync2[i]; debounce[i] <= 0;
       if (sync2[i]) pressed[i] <= 1;
     end else debounce[i] <= debounce[i]+1;
   end
 end
 reg [9:0] frame=10'h3ff;
 reg [3:0] bits=0;
 reg [7:0] baud=0;
 assign tx_pin=frame[0]; assign busy=bits!=0;
 always @(posedge clk) begin
   if (reset) begin frame<=10'h3ff; bits<=0; baud<=0; end
   else if (busy) begin
     if (baud==216) begin baud<=0; frame<={1'b1,frame[9:1]}; bits<=bits-1; end
     else baud<=baud+1;
   end else if (send) begin frame<={1'b1,data,1'b0}; bits<=10; baud<=0; end
 end
 reg [3:0] rx_bits=0;
 reg [7:0] rx_baud=0, shift=0;
 reg push=0;
 reg [7:0] received=0;
 always @(posedge clk) begin
   sync_rx<={sync_rx[0],rx_pin}; push<=0;
   if (reset) begin rx_bits<=0; rx_baud<=0; end
   else if (rx_bits==0) begin
     if (!sync_rx[1]) begin rx_bits<=1; rx_baud<=108; end
   end else if (rx_baud!=0) rx_baud<=rx_baud-1;
   else begin
     rx_baud<=216;
     if (rx_bits==1) rx_bits<=sync_rx[1] ? 0 : 2;
     else if (rx_bits==10) begin
       if (sync_rx[1]) begin received<=shift; push<=1; end
       rx_bits<=0;
     end else begin shift<={sync_rx[1],shift[7:1]}; rx_bits<=rx_bits+1; end
   end
 end
 // Circular FIFO: 256 pending bytes, with independent read/write pointers.
 // A full UART burst of the documented commands fits while a frame prints.
 (* ram_style="distributed" *) reg [7:0] fifo[0:255];
 integer j;
 initial for (j=0;j<256;j=j+1) fifo[j]=0;
 reg [8:0] wr=0, rd=0;
 assign available=wr!=rd;
 assign key=fifo[rd[7:0]];
 always @(posedge clk) begin
   if (reset) begin wr<=0; rd<=0; end
   else begin
     if (push) begin fifo[wr[7:0]]<=received; wr<=wr+1; end
     if (take && available) rd<=rd+1;
   end
 end
endmodule
