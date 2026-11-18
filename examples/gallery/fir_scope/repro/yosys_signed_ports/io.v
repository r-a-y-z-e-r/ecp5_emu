// UART 115200 8N1 and a 256-byte command FIFO. The receiver runs even
// while a frame is being drawn; restarting the demo never flushes its keys.
module gallery_io #(
  parameter K1="+", K2="-", K3="w", K4="s", K5="a", K6="d"
)(input clk, input reset, input rx_pin, output tx_pin, input [6:0] btn,
  input [7:0] tx_data, input tx_valid, output tx_ready,
  output [7:0] key, output key_valid, input key_pop, output reg overflow=0);
  reg [9:0] tx_shift=10'h3ff;
  reg [3:0] tx_bits=0;
  reg [7:0] tx_baud=0;
  assign tx_ready = tx_bits==0;
  assign tx_pin = tx_shift[0];
  always @(posedge clk) begin
    if (reset) begin tx_bits<=0; tx_shift<=10'h3ff; tx_baud<=0; end
    else if (tx_bits!=0) begin
      if (tx_baud==216) begin
        tx_baud<=0; tx_shift<={1'b1,tx_shift[9:1]}; tx_bits<=tx_bits-1;
      end else tx_baud<=tx_baud+1;
    end else if (tx_valid) begin
      tx_shift<={1'b1,tx_data,1'b0}; tx_bits<=10; tx_baud<=0;
    end
  end
  reg [1:0] sync_rx=3;
  always @(posedge clk) sync_rx<={sync_rx[0],rx_pin};
  reg [3:0] rx_bits=0;
  reg [7:0] rx_baud=0, rx_shift=0, received=0;
  reg received_valid=0;
  always @(posedge clk) begin
    received_valid<=0;
    // Do not reset the receiver during POR: the host starts at cycle zero.
    if (rx_bits==0) begin
      if (!sync_rx[1]) begin rx_bits<=1; rx_baud<=108; end
    end else if (rx_baud!=0) rx_baud<=rx_baud-1;
    else begin
      rx_baud<=216;
      if (rx_bits==1) rx_bits<=sync_rx[1] ? 0:2;
      else if (rx_bits==10) begin
        if (sync_rx[1]) begin received<=rx_shift; received_valid<=1; end
        rx_bits<=0;
      end else begin rx_shift<={sync_rx[1],rx_shift[7:1]}; rx_bits<=rx_bits+1; end
    end
  end
  reg [6:1] bs1=0,bs2=0,stable=0,press=0,pending=0;
  reg [17:0] debounce[1:6];
  integer j;
  initial for(j=1;j<=6;j=j+1) debounce[j]=0;
  always @(posedge clk) begin
    bs1<=btn[6:1]; bs2<=bs1; press<=0;
    for(j=1;j<=6;j=j+1) begin
      if (bs2[j]==stable[j]) debounce[j]<=0;
      else if (&debounce[j]) begin
        stable[j]<=bs2[j]; debounce[j]<=0; press[j]<=bs2[j];
      end else debounce[j]<=debounce[j]+1;
    end
  end
  reg [7:0] fifo[0:255];
  reg [8:0] wr=0,rd=0;
  integer m;
  initial for(m=0;m<256;m=m+1) fifo[m]=0;
  wire full=(wr-rd)==256;
  assign key_valid=wr!=rd;
  assign key=fifo[rd[7:0]];
  reg [7:0] button_key;
  reg [6:1] taken;
  always @* begin
    taken=0; button_key=0;
    if(pending[1]) begin taken[1]=1; button_key=K1; end
    else if(pending[2]) begin taken[2]=1; button_key=K2; end
    else if(pending[3]) begin taken[3]=1; button_key=K3; end
    else if(pending[4]) begin taken[4]=1; button_key=K4; end
    else if(pending[5]) begin taken[5]=1; button_key=K5; end
    else if(pending[6]) begin taken[6]=1; button_key=K6; end
  end
  always @(posedge clk) begin
    pending<=pending|press;
    if(key_pop && key_valid) rd<=rd+1;
    if(received_valid) begin
      if(!full) begin fifo[wr[7:0]]<=received; wr<=wr+1; end
      else overflow<=1;
    end else if((|pending) && !full) begin
      fifo[wr[7:0]]<=button_key; wr<=wr+1; pending<=(pending & ~taken)|press;
    end
  end
endmodule
