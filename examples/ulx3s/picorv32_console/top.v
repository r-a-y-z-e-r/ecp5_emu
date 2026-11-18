// A small RISC-V system: the stock PicoRV32 core (RV32I, picorv32.v from
// YosysHQ, unchanged), 8 KB of block RAM holding the firmware, a UART at
// 115200 baud 8N1 and the eight LEDs. The firmware is in firmware/. Its
// image is split into four byte lanes, fw0.hex .. fw3.hex, one EBR
// (2048x9) each, so every byte strobe is the write enable of its own block.
//
// Memory map:
//   0x0000_0000  RAM, 8 KB; the core starts at 0
//   0x0200_0000  UART data: a write sends a byte, a read takes the received one
//   0x0200_0004  UART status: bit 0 the transmitter is busy, bit 1 a byte arrived
//   0x0300_0000  LEDs
module top(input clk_25mhz, input ftdi_txd, output ftdi_rxd, output [7:0] led);
  localparam BIT_CYCLES = 217;  // 25 MHz / 115200

  wire clk = clk_25mhz;
  reg [5:0] reset_count = 0;
  wire resetn = &reset_count;
  always @(posedge clk) if (!resetn) reset_count <= reset_count + 1;

  wire mem_valid, mem_instr;
  wire [31:0] mem_addr, mem_wdata;
  wire [3:0] mem_wstrb;
  wire [31:0] mem_rdata;
  reg mem_ready = 0;

  picorv32 cpu (
    .clk(clk),
    .resetn(resetn),
    .mem_valid(mem_valid),
    .mem_instr(mem_instr),
    .mem_ready(mem_ready),
    .mem_addr(mem_addr),
    .mem_wdata(mem_wdata),
    .mem_wstrb(mem_wstrb),
    .mem_rdata(mem_rdata)
  );

  // Every access takes two cycles: the request, then mem_ready with the data.
  wire request = mem_valid && !mem_ready;
  wire ram_selected = mem_addr[31:24] == 8'h00;
  wire uart_selected = mem_addr[31:24] == 8'h02;
  wire led_selected = mem_addr[31:24] == 8'h03;
  wire writing = request && mem_wstrb != 0;
  always @(posedge clk) mem_ready <= request;

  wire [10:0] word = mem_addr[12:2];
  (* ram_style = "block" *) reg [7:0] lane0 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane1 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane2 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane3 [0:2047];
  initial $readmemh("fw0.hex", lane0);
  initial $readmemh("fw1.hex", lane1);
  initial $readmemh("fw2.hex", lane2);
  initial $readmemh("fw3.hex", lane3);
  reg [31:0] ram_q;
  always @(posedge clk)
    if (request && ram_selected) begin
      if (mem_wstrb[0]) lane0[word] <= mem_wdata[7:0];
      if (mem_wstrb[1]) lane1[word] <= mem_wdata[15:8];
      if (mem_wstrb[2]) lane2[word] <= mem_wdata[23:16];
      if (mem_wstrb[3]) lane3[word] <= mem_wdata[31:24];
      ram_q <= {lane3[word], lane2[word], lane1[word], lane0[word]};
    end

  // Transmitter: a write while it is busy is dropped; the firmware polls.
  reg [9:0] tx_frame = 10'h3ff;  // stop bit, 8 data bits LSB first, start bit
  reg [3:0] tx_bits = 0;
  reg [7:0] tx_baud = 0;
  wire tx_busy = tx_bits != 0;
  always @(posedge clk)
    if (tx_busy) begin
      if (tx_baud == BIT_CYCLES - 1) begin
        tx_baud <= 0;
        tx_frame <= {1'b1, tx_frame[9:1]};
        tx_bits <= tx_bits - 1;
      end else
        tx_baud <= tx_baud + 1;
    end else if (writing && uart_selected && !mem_addr[2]) begin
      tx_frame <= {1'b1, mem_wdata[7:0], 1'b0};
      tx_bits <= 10;
    end
  assign ftdi_rxd = tx_frame[0];

  // Receiver: samples each bit in its middle; a byte waits in rx_data until
  // the firmware reads it.
  reg [1:0] rx_sync = 2'b11;
  always @(posedge clk) rx_sync <= {rx_sync[0], ftdi_txd};
  wire rx = rx_sync[1];
  reg [3:0] rx_bits = 0;  // 0 idle, 1 start bit, 2..9 data bits, 10 stop bit
  reg [7:0] rx_baud = 0;
  reg [7:0] rx_shift = 0;
  reg [7:0] rx_data = 0;
  reg rx_valid = 0;
  wire taking = request && uart_selected && !mem_addr[2] && mem_wstrb == 0;
  always @(posedge clk) begin
    if (taking) rx_valid <= 0;
    if (rx_bits == 0) begin
      if (!rx) begin
        rx_bits <= 1;
        rx_baud <= BIT_CYCLES / 2;
      end
    end else if (rx_baud != 0)
      rx_baud <= rx_baud - 1;
    else begin
      rx_baud <= BIT_CYCLES - 1;
      if (rx_bits == 1)
        rx_bits <= rx ? 4'd0 : 4'd2;  // a glitch is not a start bit
      else if (rx_bits == 10) begin
        if (rx) begin
          rx_data <= rx_shift;
          rx_valid <= 1;
        end
        rx_bits <= 0;
      end else begin
        rx_shift <= {rx, rx_shift[7:1]};
        rx_bits <= rx_bits + 1;
      end
    end
  end

  reg [7:0] leds = 0;
  always @(posedge clk) if (writing && led_selected && mem_wstrb[0]) leds <= mem_wdata[7:0];
  assign led = leds;

  reg [31:0] io_q;
  reg from_ram;
  always @(posedge clk)
    if (request) begin
      from_ram <= ram_selected;
      io_q <= mem_addr[2] ? {30'd0, rx_valid, tx_busy} : {24'd0, rx_data};
    end
  assign mem_rdata = from_ram ? ram_q : io_q;
endmodule
