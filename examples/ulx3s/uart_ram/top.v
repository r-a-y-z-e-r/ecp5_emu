// UART transmitter that keeps its text in block RAM. The message is a ROM
// (message.hex); at start it is copied into a 512x16 RAM, entry i holding
// {i, character i}, and the characters are then sent from the RAM: the ROM
// and the RAM each take one EBR. Sends "Hello from block RAM!\r\n" at
// 115200 baud, 8N1, then about ten times per second; the LEDs show the index
// read back with each character.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  localparam BIT_CYCLES = 217;       // 25 MHz / 115200
  localparam PAUSE_CYCLES = 2500000; // 0.1 s between messages
  localparam LENGTH = 23;

  (* ram_style = "block" *) reg [7:0] rom [0:511];
  initial $readmemh("message.hex", rom);
  reg [8:0] copy = 0;
  reg [7:0] rom_q;
  always @(posedge clk_25mhz) rom_q <= rom[copy];

  // The ROM answers a clock later, so entry copy - 1 is written.
  (* ram_style = "block" *) reg [15:0] ram [0:511];
  reg copying = 1, ready = 0;
  always @(posedge clk_25mhz)
    if (copying) begin
      if (copy != 0) ram[copy - 1] <= {copy[7:0] - 8'd1, rom_q};
      copy <= copy + 1;
      if (copy == LENGTH) begin
        copying <= 0;
        ready <= 1;
      end
    end

  // Reads only once the copy is done, so no read meets a write.
  reg [8:0] index = 0;
  reg [15:0] ram_q;
  always @(posedge clk_25mhz) if (ready) ram_q <= ram[index];
  reg go = 0;
  always @(posedge clk_25mhz) go <= ready;

  reg [9:0] frame = 10'h3ff;  // stop bit, 8 data bits LSB first, start bit
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  reg [21:0] pause = 0;
  reg [7:0] shown = 0;
  always @(posedge clk_25mhz)
    if (!go)
      ;
    else if (bits_left != 0) begin
      if (baud == BIT_CYCLES - 1) begin
        baud <= 0;
        frame <= {1'b1, frame[9:1]};
        bits_left <= bits_left - 1;
      end else
        baud <= baud + 1;
    end else if (pause != 0)
      pause <= pause - 1;
    else begin
      frame <= {1'b1, ram_q[7:0], 1'b0};
      bits_left <= 10;
      shown <= ram_q[15:8];
      if (index == LENGTH - 1) begin
        index <= 0;
        pause <= PAUSE_CYCLES;
      end else
        index <= index + 1;
    end
  assign ftdi_rxd = frame[0];
  assign led = shown;
endmodule
