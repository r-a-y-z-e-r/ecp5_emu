// Read-only IS25LP128F controller. There is deliberately no command input.
// op: 0 = READ four bytes, 1 = JEDEC ID, 2/3 = status. SCK = 25 MHz / 2.
// Never add a write, erase or status-write operation: this is configuration flash.
module flash_reader(
  input clk, input start, input [1:0] op, input [23:0] addr,
  output reg ready = 0, output reg [31:0] data = 0,
  output reg flash_csn = 1, output reg flash_mosi = 1,
  input flash_miso, output flash_wpn, output flash_holdn,
  output reg sck = 0
);
  localparam [7:0] CMD_READ = 8'h03, CMD_RDID = 8'h9f, CMD_RDSR = 8'h05;
  assign flash_wpn = 1'b1;
  assign flash_holdn = 1'b1;
  USRMCLK clock_pin(.USRMCLKI(sck), .USRMCLKTS(1'b0));
  reg [31:0] tx = 0;
  reg [6:0] left = 0, receive_bits = 0;
  reg receiving = 0;
  reg busy = 0;
  // Leave a generous CS setup interval, also safe for hosts sampling an
  // idle peripheral in batches of 64 board-clock edges.
  reg [5:0] setup = 0;
  always @(posedge clk) begin
    ready <= 0;
    if (!busy) begin
      if (start) begin
        busy <= 1; setup <= 32; flash_csn <= 0; sck <= 0; receiving <= 0; data <= 0;
        if (op == 0) begin
          tx <= {CMD_READ,addr}; flash_mosi <= CMD_READ[7];
          left <= 32; receive_bits <= 32;
        end else if (op == 1) begin
          tx <= {CMD_RDID,24'd0}; flash_mosi <= CMD_RDID[7];
          left <= 8; receive_bits <= 24;
        end else begin
          tx <= {CMD_RDSR,24'd0}; flash_mosi <= CMD_RDSR[7];
          left <= 8; receive_bits <= 8;
        end
      end
    end else if (setup != 0) setup <= setup - 1;
    else if (!sck) begin
      sck <= 1;
      if (receiving) data <= {data[30:0],flash_miso};
    end else begin
      sck <= 0;
      if (left == 1) begin
        if (!receiving) begin
          receiving <= 1; left <= receive_bits; flash_mosi <= 1;
        end else begin busy <= 0; ready <= 1; flash_csn <= 1; end
      end else begin
        left <= left - 1;
        if (!receiving) begin tx <= {tx[30:0],1'b1}; flash_mosi <= tx[30]; end
      end
    end
  end
endmodule
