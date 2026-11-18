// 115200 8N1 at 25 MHz (217 board clocks per bit).
module uart_tx(input clk, input send, input [7:0] data,
               output busy, output tx);
  reg [9:0] frame = 10'h3ff;
  reg [3:0] bits = 0;
  reg [7:0] baud = 0;
  assign busy = bits != 0;
  assign tx = frame[0];
  always @(posedge clk)
    if (busy) begin
      if (baud == 216) begin
        baud <= 0; frame <= {1'b1,frame[9:1]}; bits <= bits - 1;
      end else baud <= baud + 1;
    end else if (send) begin
      frame <= {1'b1,data,1'b0}; bits <= 10; baud <= 0;
    end
endmodule
