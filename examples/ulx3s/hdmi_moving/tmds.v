// DVI 1.0 Figure 3-5, one byte per pixel clock. The first stage minimises
// transitions; the second balances the running disparity. Bit 0 leaves first.
module tmds(input clk, input [7:0] data, input de, input [1:0] control,
            output reg [9:0] word = 10'b1101010100);
  reg [3:0] ones;
  reg [8:0] q;
  reg signed [4:0] balance;
  reg signed [5:0] disparity = 0;
  integer i;
  always @* begin
    ones = 0;
    for (i=0; i<8; i=i+1) ones = ones + data[i];
    q[8] = !(ones > 4 || (ones == 4 && !data[0]));
    q[0] = data[0];
    for (i=1; i<8; i=i+1) q[i] = q[i-1] ^ data[i] ^ !q[8];
    balance = -8;
    for (i=0; i<8; i=i+1) balance = balance + (q[i] ? 2 : 0);
  end
  always @(posedge clk) begin
    if (!de) begin
      disparity <= 0;
      case (control)
        0: word <= 10'b1101010100;
        1: word <= 10'b0010101011;
        2: word <= 10'b0101010100;
        3: word <= 10'b1010101011;
      endcase
    end else if (disparity == 0 || balance == 0) begin
      word <= {!q[8], q[8], q[8] ? q[7:0] : ~q[7:0]};
      disparity <= disparity + (q[8] ? balance : -balance);
    end else if ((disparity > 0 && balance > 0) || (disparity < 0 && balance < 0)) begin
      word <= {1'b1, q[8], ~q[7:0]};
      disparity <= disparity + (q[8] ? 2 : 0) - balance;
    end else begin
      word <= {1'b0, q[8], q[7:0]};
      disparity <= disparity - (q[8] ? 0 : 2) + balance;
    end
  end
endmodule
