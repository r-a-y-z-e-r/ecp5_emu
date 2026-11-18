// One 25 MHz clock domain: RV32IMC, sixteen independently swappable EBRs,
// a buffered UART, and synchronised/debounced board inputs.
module top(input clk_25mhz, input ftdi_txd, input [6:0] btn,
           output ftdi_rxd, output [7:0] led);
  localparam BIT_CYCLES = 217;
  wire clk = clk_25mhz;
  reg [5:0] reset_count = 0;
  wire resetn = &reset_count;
  wire soft_reset;
  always @(posedge clk)
    if (soft_reset) reset_count <= 0;
    else if (!resetn) reset_count <= reset_count + 1'b1;

  wire mem_valid;
  wire [31:0] mem_addr, mem_wdata, mem_rdata;
  wire [3:0] mem_wstrb;
  reg mem_ready = 0;
  picorv32 #(
    .COMPRESSED_ISA(1), .ENABLE_MUL(0), .ENABLE_FAST_MUL(1),
    .ENABLE_DIV(1), .BARREL_SHIFTER(1), .ENABLE_REGS_DUALPORT(1),
    .ENABLE_COUNTERS(1), .ENABLE_COUNTERS64(1), .ENABLE_IRQ(0)
  ) cpu (
    .clk(clk), .resetn(resetn), .mem_valid(mem_valid),
    .mem_ready(mem_ready), .mem_addr(mem_addr), .mem_wdata(mem_wdata),
    .mem_wstrb(mem_wstrb), .mem_rdata(mem_rdata),
    .pcpi_wr(1'b0), .pcpi_rd(32'd0), .pcpi_wait(1'b0),
    .pcpi_ready(1'b0), .irq(32'd0)
  );
  wire request = resetn && mem_valid && !mem_ready;
  wire writing = request && |mem_wstrb;
  wire ram_selected = mem_addr[31:15] == 0;
  wire uart_selected = mem_addr[31:3] == (32'h02000000 >> 3);
  wire led_selected = mem_addr == 32'h03000000;
  wire button_selected = mem_addr[31:3] == (32'h04000000 >> 3);
  always @(posedge clk) mem_ready <= request;

  // An EBR is 2048 bytes in this configuration. Keeping the byte write
  // enables separate lets ecpbram identify and replace all sixteen images.
  wire [31:0] bank_q [0:3];
  // Every bank is read during reset, so each EBR output latch holds a known
  // value before software first visits that bank. The chip starts them at 0
  // (HB02), and so does the emulator's configured startup; the read keeps fast
  // mode starting under the ready-unknown startup too.
  wire [10:0] ram_word = resetn ? mem_addr[12:2] : 11'd0;
  reg [1:0] read_bank = 0;
  genvar bank, lane;
  generate for (bank = 0; bank < 4; bank = bank + 1) begin: banks
    for (lane = 0; lane < 4; lane = lane + 1) begin: lanes
      (* ram_style = "block" *) reg [7:0] bytes [0:2047];
      // Literal names avoid tool-dependent string formatting in synthesis.
      if (bank == 0 && lane == 0) initial $readmemh("fw_0_0.hex", bytes);
      if (bank == 0 && lane == 1) initial $readmemh("fw_0_1.hex", bytes);
      if (bank == 0 && lane == 2) initial $readmemh("fw_0_2.hex", bytes);
      if (bank == 0 && lane == 3) initial $readmemh("fw_0_3.hex", bytes);
      if (bank == 1 && lane == 0) initial $readmemh("fw_1_0.hex", bytes);
      if (bank == 1 && lane == 1) initial $readmemh("fw_1_1.hex", bytes);
      if (bank == 1 && lane == 2) initial $readmemh("fw_1_2.hex", bytes);
      if (bank == 1 && lane == 3) initial $readmemh("fw_1_3.hex", bytes);
      if (bank == 2 && lane == 0) initial $readmemh("fw_2_0.hex", bytes);
      if (bank == 2 && lane == 1) initial $readmemh("fw_2_1.hex", bytes);
      if (bank == 2 && lane == 2) initial $readmemh("fw_2_2.hex", bytes);
      if (bank == 2 && lane == 3) initial $readmemh("fw_2_3.hex", bytes);
      if (bank == 3 && lane == 0) initial $readmemh("fw_3_0.hex", bytes);
      if (bank == 3 && lane == 1) initial $readmemh("fw_3_1.hex", bytes);
      if (bank == 3 && lane == 2) initial $readmemh("fw_3_2.hex", bytes);
      if (bank == 3 && lane == 3) initial $readmemh("fw_3_3.hex", bytes);
      reg [7:0] q = 0;
      always @(posedge clk) begin
        if (request && ram_selected && mem_addr[14:13] == bank && mem_wstrb[lane])
          bytes[ram_word] <= mem_wdata[8*lane +: 8];
        if (!resetn || (request && ram_selected && mem_addr[14:13] == bank))
          q <= bytes[ram_word];
      end
      assign bank_q[bank][8*lane +: 8] = q;
    end
  end endgenerate
  always @(posedge clk) if (request) read_bank <= mem_addr[14:13];

  reg [9:0] tx_frame = 10'h3ff;
  reg [3:0] tx_bits = 0;
  reg [7:0] tx_baud = 0;
  wire tx_busy = tx_bits != 0;
  always @(posedge clk)
    if (!resetn) begin tx_frame <= 10'h3ff; tx_bits <= 0; tx_baud <= 0; end
    else if (tx_busy) begin
      if (tx_baud == BIT_CYCLES-1) begin
        tx_baud <= 0;
        tx_frame <= {1'b1, tx_frame[9:1]};
        tx_bits <= tx_bits - 1'b1;
      end else tx_baud <= tx_baud + 1'b1;
    end else if (writing && uart_selected && !mem_addr[2]) begin
      tx_frame <= {1'b1, mem_wdata[7:0], 1'b0};
      tx_bits <= 10;
      tx_baud <= 0;
    end
  assign ftdi_rxd = tx_frame[0];

  reg reset_pending = 0;
  always @(posedge clk)
    if (!resetn) reset_pending <= 0;
    else if (writing && mem_addr == 32'h05000000 &&
             mem_wstrb == 4'b1111 && mem_wdata == 32'h005eb007)
      reset_pending <= 1;
  // Never truncate the UART's last stop bit, including a raw MMIO reset.
  assign soft_reset = resetn && reset_pending && !tx_busy;

  reg [1:0] rx_sync = 2'b11;
  reg [3:0] rx_bits = 0;
  reg [7:0] rx_baud = 0, rx_shift = 0;
  wire rx = rx_sync[1];
  wire received = resetn && rx_bits == 10 && rx_baud == 0 && rx;
  always @(posedge clk) begin
    if (!resetn) begin
      rx_sync <= 2'b11; rx_bits <= 0; rx_baud <= 0; rx_shift <= 0;
    end else begin
      rx_sync <= {rx_sync[0], ftdi_txd};
      if (rx_bits == 0) begin
        if (!rx) begin rx_bits <= 1; rx_baud <= BIT_CYCLES/2; end
      end else if (rx_baud != 0) rx_baud <= rx_baud - 1'b1;
      else begin
        rx_baud <= BIT_CYCLES-1;
        if (rx_bits == 1) rx_bits <= rx ? 4'd0 : 4'd2;
        else if (rx_bits == 10) rx_bits <= 0;
        else begin rx_shift <= {rx, rx_shift[7:1]}; rx_bits <= rx_bits + 1'b1; end
      end
    end
  end

  // The receive FIFO is one EBR (512 x 8). It holds 511 bytes, so the write
  // address (head) equals the read address (tail) only while it is empty:
  // an EBR row read in the clock its other port writes it is undefined on
  // the chip, and the read below skips exactly that case. A byte that comes
  // while it is full is dropped and sets the sticky overflow bit (status
  // bit 2), so exceeding the buffer is visible to software.
  (* ram_style = "block" *) reg [7:0] fifo [0:511];
  integer f;
  initial for (f = 0; f < 512; f = f + 1) fifo[f] = 0;
  reg [8:0] rx_head = 0, rx_tail = 0;
  reg [9:0] rx_count = 0;
  reg rx_overflow = 0;
  wire taking = request && uart_selected && !mem_addr[2] &&
                mem_wstrb == 0 && rx_count != 0;
  wire storing = received && rx_count != 511;
  // The byte at the tail, read every clock; in the clock after a read
  // request it is the byte that request took.
  reg [7:0] fifo_q = 0;
  always @(posedge clk)
    if (!(storing && rx_head == rx_tail)) fifo_q <= fifo[rx_tail];
  always @(posedge clk) begin
    if (storing) fifo[rx_head] <= rx_shift;
    if (!resetn) begin
      rx_head <= 0; rx_tail <= 0; rx_count <= 0; rx_overflow <= 0;
    end else begin
      if (storing) rx_head <= rx_head + 1'b1;
      if (taking) rx_tail <= rx_tail + 1'b1;
      case ({storing, taking})
        2'b10: rx_count <= rx_count + 1'b1;
        2'b01: rx_count <= rx_count - 1'b1;
      endcase
      if (received && !storing) rx_overflow <= 1;
    end
  end

  reg [7:0] leds = 0;
  assign led = leds;
  always @(posedge clk)
    if (!resetn) leds <= 0;
    else if (writing && led_selected && mem_wstrb[0]) leds <= mem_wdata[7:0];

  reg [6:0] button_meta = 0, button_sync = 0, levels = 0, events = 0;
  wire [6:0] pressed = {btn[6:1], ~btn[0]};
  wire [6:0] new_press;
  wire clear_events = request && button_selected && mem_addr[2] && mem_wstrb == 0;
  always @(posedge clk) begin
    if (!resetn) begin button_meta <= 0; button_sync <= 0; events <= 0; end
    else begin
      button_meta <= pressed; button_sync <= button_meta;
      // A press on the read clock remains pending for the next read.
      events <= (clear_events ? 7'd0 : events) | new_press;
    end
  end
  genvar b;
  generate for (b = 0; b < 7; b = b + 1) begin: debounce
    reg [17:0] count = 0;
    assign new_press[b] = button_sync[b] && !levels[b] && count == 249999;
    always @(posedge clk)
      if (!resetn) begin count <= 0; levels[b] <= 0; end
      else if (button_sync[b] == levels[b]) count <= 0;
      else if (count == 249999) begin levels[b] <= button_sync[b]; count <= 0; end
      else count <= count + 1'b1;
  end endgenerate

  reg from_ram = 0, from_fifo = 0;
  reg [31:0] io_q = 0;
  always @(posedge clk) if (request) begin
    from_ram <= ram_selected;
    from_fifo <= uart_selected && !mem_addr[2];
    io_q <= 0;
    if (uart_selected && mem_addr[2])
      io_q <= {28'd0, (rx_bits != 0), rx_overflow, (rx_count != 0), tx_busy};
    if (led_selected) io_q <= {24'd0, leds};
    if (button_selected) io_q <= {25'd0, mem_addr[2] ? events : levels};
  end
  assign mem_rdata = from_ram ? bank_q[read_bank] :
                     from_fifo ? {24'd0, fifo_q} : io_q;
endmodule
