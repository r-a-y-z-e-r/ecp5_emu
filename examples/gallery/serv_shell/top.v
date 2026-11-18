// One 25 MHz clock domain: SERV bit-serial RV32I, sixteen independently
// swappable EBRs, a buffered UART, a 64-bit cycle counter, and debounced buttons.
`default_nettype none
module top(
  input wire clk_25mhz,
  input wire ftdi_txd,
  input wire [6:0] btn,
  output wire ftdi_rxd,
  output wire [7:0] led
);
  localparam BIT_CYCLES = 217;
  wire clk = clk_25mhz;
  reg [5:0] reset_count = 6'd0;
  wire resetn = &reset_count;
  wire soft_reset;
  always @(posedge clk)
    if (soft_reset) reset_count <= 6'd0;
    else if (!resetn) reset_count <= reset_count + 1'b1;

  wire [31:0] ibus_adr, dbus_adr, dbus_dat;
  wire [3:0]  dbus_sel;
  wire        ibus_cyc, dbus_we, dbus_cyc;
  reg         wb_ack = 1'b0, wb_grant_d = 1'b0;
  wire [31:0] mem_rdata;

  serv_rf_top #(
    .RESET_PC(32'd0),
    .COMPRESSED(0),
    .MDU(0),
    .WITH_CSR(1)
  ) cpu (
    .clk(clk),
    .i_rst(!resetn),
    .i_timer_irq(1'b0),
    .o_ibus_adr(ibus_adr),
    .o_ibus_cyc(ibus_cyc),
    .i_ibus_rdt(mem_rdata),
    .i_ibus_ack(wb_ack && !wb_grant_d),
    .o_dbus_adr(dbus_adr),
    .o_dbus_dat(dbus_dat),
    .o_dbus_sel(dbus_sel),
    .o_dbus_we(dbus_we),
    .o_dbus_cyc(dbus_cyc),
    .i_dbus_rdt(mem_rdata),
    .i_dbus_ack(wb_ack && wb_grant_d),
    .o_ext_rs1(),
    .o_ext_rs2(),
    .o_ext_funct3(),
    .i_ext_rd(32'd0),
    .i_ext_ready(1'b0),
    .o_mdu_valid()
  );

  // SERV never asserts ibus_cyc and dbus_cyc together, and dbus wins if it
  // ever does. One registered cycle serves RAM and MMIO identically to riscv_shell.
  wire request = resetn && !wb_ack && (dbus_cyc || ibus_cyc);
  wire [31:0] mem_addr  = dbus_cyc ? dbus_adr : ibus_adr;
  wire [31:0] mem_wdata = dbus_dat;
  wire [3:0]  mem_wstrb = (dbus_cyc && dbus_we) ? dbus_sel : 4'b0000;
  wire writing = request && |mem_wstrb;
  always @(posedge clk) begin
    if (!resetn) begin
      wb_ack <= 1'b0;
      wb_grant_d <= 1'b0;
    end else begin
      wb_ack <= request;
      if (request) wb_grant_d <= dbus_cyc;
    end
  end

  wire ram_selected     = mem_addr[31:15] == 17'd0;
  wire uart_selected    = mem_addr[31:3] == (32'h02000000 >> 3);
  wire led_selected     = mem_addr == 32'h03000000;
  wire button_selected  = mem_addr[31:3] == (32'h04000000 >> 3);
  wire counter_selected = mem_addr[31:3] == (32'h06000000 >> 3);

  // SERV implements no cycle CSR, so software reads this free-running counter.
  reg [63:0] cycle_counter = 64'd0;
  always @(posedge clk)
    if (!resetn) cycle_counter <= 64'd0;
    else cycle_counter <= cycle_counter + 64'd1;

  // An EBR is 2048 bytes in this configuration. Keeping the byte write
  // enables separate lets ecpbram identify and replace all sixteen images.
  wire [31:0] bank_q [0:3];
  // Every bank is read during reset, so each EBR output latch holds a known
  // value before software first visits that bank.
  wire [10:0] ram_word = resetn ? mem_addr[12:2] : 11'd0;
  reg [1:0] read_bank = 2'd0;
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
      reg [7:0] q = 8'd0;
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
  reg [3:0] tx_bits = 4'd0;
  reg [7:0] tx_baud = 8'd0;
  wire tx_busy = tx_bits != 4'd0;
  always @(posedge clk)
    if (!resetn) begin tx_frame <= 10'h3ff; tx_bits <= 4'd0; tx_baud <= 8'd0; end
    else if (tx_busy) begin
      if (tx_baud == BIT_CYCLES-1) begin
        tx_baud <= 8'd0;
        tx_frame <= {1'b1, tx_frame[9:1]};
        tx_bits <= tx_bits - 1'b1;
      end else tx_baud <= tx_baud + 1'b1;
    end else if (writing && uart_selected && !mem_addr[2]) begin
      tx_frame <= {1'b1, mem_wdata[7:0], 1'b0};
      tx_bits <= 4'd10;
      tx_baud <= 8'd0;
    end
  assign ftdi_rxd = tx_frame[0];

  reg reset_pending = 1'b0;
  always @(posedge clk)
    if (!resetn) reset_pending <= 1'b0;
    else if (writing && mem_addr == 32'h05000000 &&
             mem_wstrb == 4'b1111 && mem_wdata == 32'h005eb007)
      reset_pending <= 1'b1;
  // Never truncate the UART's last stop bit, including a raw MMIO reset.
  assign soft_reset = resetn && reset_pending && !tx_busy;

  reg [1:0] rx_sync = 2'b11;
  reg [3:0] rx_bits = 4'd0;
  reg [7:0] rx_baud = 8'd0, rx_shift = 8'd0;
  wire rx = rx_sync[1];
  wire received = resetn && rx_bits == 4'd10 && rx_baud == 8'd0 && rx;
  always @(posedge clk) begin
    if (!resetn) begin
      rx_sync <= 2'b11; rx_bits <= 4'd0; rx_baud <= 8'd0; rx_shift <= 8'd0;
    end else begin
      rx_sync <= {rx_sync[0], ftdi_txd};
      if (rx_bits == 4'd0) begin
        if (!rx) begin rx_bits <= 4'd1; rx_baud <= BIT_CYCLES/2; end
      end else if (rx_baud != 8'd0) rx_baud <= rx_baud - 1'b1;
      else begin
        rx_baud <= BIT_CYCLES-1;
        if (rx_bits == 4'd1) rx_bits <= rx ? 4'd0 : 4'd2;
        else if (rx_bits == 4'd10) rx_bits <= 4'd0;
        else begin rx_shift <= {rx, rx_shift[7:1]}; rx_bits <= rx_bits + 1'b1; end
      end
    end
  end

  // The receive FIFO is one EBR (512 x 8). It holds 511 bytes, so the write
  // address (head) equals the read address (tail) only while it is empty:
  // an EBR row read in the clock its other port writes it is undefined on
  // the chip, and the read below skips exactly that case.
  (* ram_style = "block" *) reg [7:0] fifo [0:511];
  integer f;
  initial for (f = 0; f < 512; f = f + 1) fifo[f] = 8'd0;
  reg [8:0] rx_head = 9'd0, rx_tail = 9'd0;
  reg [9:0] rx_count = 10'd0;
  reg rx_overflow = 1'b0;
  wire taking = request && uart_selected && !mem_addr[2] &&
                mem_wstrb == 4'd0 && rx_count != 10'd0;
  wire storing = received && rx_count != 10'd511;
  reg [7:0] fifo_q = 8'd0;
  always @(posedge clk)
    if (!(storing && rx_head == rx_tail)) fifo_q <= fifo[rx_tail];
  always @(posedge clk) begin
    if (storing) fifo[rx_head] <= rx_shift;
    if (!resetn) begin
      rx_head <= 9'd0; rx_tail <= 9'd0; rx_count <= 10'd0; rx_overflow <= 1'b0;
    end else begin
      if (storing) rx_head <= rx_head + 1'b1;
      if (taking) rx_tail <= rx_tail + 1'b1;
      case ({storing, taking})
        2'b10: rx_count <= rx_count + 1'b1;
        2'b01: rx_count <= rx_count - 1'b1;
        default: ;
      endcase
      if (received && !storing) rx_overflow <= 1'b1;
    end
  end

  reg [7:0] leds = 8'd0;
  assign led = leds;
  always @(posedge clk)
    if (!resetn) leds <= 8'd0;
    else if (writing && led_selected && mem_wstrb[0]) leds <= mem_wdata[7:0];

  reg [6:0] button_meta = 7'd0, button_sync = 7'd0, levels = 7'd0, events = 7'd0;
  wire [6:0] pressed = {btn[6:1], ~btn[0]};
  wire [6:0] new_press;
  wire clear_events = request && button_selected && mem_addr[2] && mem_wstrb == 4'd0;
  always @(posedge clk) begin
    if (!resetn) begin button_meta <= 7'd0; button_sync <= 7'd0; events <= 7'd0; end
    else begin
      button_meta <= pressed; button_sync <= button_meta;
      // A press on the read clock remains pending for the next read.
      events <= (clear_events ? 7'd0 : events) | new_press;
    end
  end
  genvar b;
  generate for (b = 0; b < 7; b = b + 1) begin: debounce
    reg [17:0] count = 18'd0;
    assign new_press[b] = button_sync[b] && !levels[b] && count == 18'd249999;
    always @(posedge clk)
      if (!resetn) begin count <= 18'd0; levels[b] <= 1'b0; end
      else if (button_sync[b] == levels[b]) count <= 18'd0;
      else if (count == 18'd249999) begin levels[b] <= button_sync[b]; count <= 18'd0; end
      else count <= count + 1'b1;
  end endgenerate

  reg from_ram = 1'b0, from_fifo = 1'b0;
  reg [31:0] io_q = 32'd0;
  always @(posedge clk) if (request) begin
    from_ram <= ram_selected;
    from_fifo <= uart_selected && !mem_addr[2];
    io_q <= 32'd0;
    if (uart_selected && mem_addr[2])
      io_q <= {28'd0, (rx_bits != 4'd0), rx_overflow, (rx_count != 10'd0), tx_busy};
    if (led_selected) io_q <= {24'd0, leds};
    if (button_selected) io_q <= {25'd0, mem_addr[2] ? events : levels};
    if (counter_selected) io_q <= mem_addr[2] ? cycle_counter[63:32] : cycle_counter[31:0];
  end
  assign mem_rdata = from_ram ? bank_q[read_bank] :
                     from_fifo ? {24'd0, fifo_q} : io_q;
endmodule
`default_nettype wire
