// A complete small computer on the ULX3S: riscv_shell's PicoRV32 RV32IMC
// with 32 KB of RAM, its UART with a 511-byte FIFO, buttons, LEDs and reset
// register, plus a text screen on HDMI: 80 x 30 characters of 8 x 16 pixels
// on a 640 x 480 @ 60 DVI picture, with a blinking hardware cursor and a
// CRC-32 of every frame that the firmware can read back. Everything except
// the TMDS shift runs in the PLL's 25 MHz pixel clock domain, as in
// video_life; the CPU side is riscv_shell's, held in reset until the PLL
// locks.
module top(input clk_25mhz, input ftdi_txd, input [6:0] btn,
           output ftdi_rxd, output [7:0] led, output [3:0] gpdi_dp);
  localparam BIT_CYCLES = 217;

  // One PLL: 125 MHz shift clock for the TMDS lanes, 25 MHz pixel clock for
  // everything else. Hold the computer in reset until the PLL locks (in the
  // RTL simulation sim/stubs.v ties locked high, as the emulator does).
  wire shift_clk, pixel_clk, locked;
  pll clocks(.clkin(clk_25mhz), .clkout0(shift_clk), .clkout1(pixel_clk),
             .locked(locked));
  wire clk = pixel_clk;
  reg [5:0] reset_count = 0;
  wire resetn = &reset_count;
  wire soft_reset;
  always @(posedge clk)
    if (soft_reset || !locked) reset_count <= 0;
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
  // The screen: 2 400 character cells at 0x0600_0000, registers at 0x0600_1000.
  wire char_selected = mem_addr[31:12] == (32'h06000000 >> 12) &&
                       mem_addr[11:0] < 2400;
  wire screen_selected = mem_addr[31:8] == (32'h06001000 >> 8);
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

  // ---- The text screen ----

  // The exact video timing of hdmi_moving: 800 x 525 total, syncs active low.
  reg [9:0] x = 0, y = 0;
  always @(posedge clk)
    if (x == 799) begin
      x <= 0;
      if (y == 524) y <= 0; else y <= y + 1;
    end else x <= x + 1;
  wire active = x < 640 && y < 480;

  // The character buffer: 2 400 bytes (80 x 30 cells, row-major) in two EBRs,
  // one write port (the CPU's MMIO stores) and one read port (the display).
  // The CPU writes bytes; the display reads one character cell per 8 pixels. The buffer
  // starts as zeros, which the font maps to blank, as does a space.
  (* ram_style = "block" *) reg [7:0] cells_lo [0:2047];
  (* ram_style = "block" *) reg [7:0] cells_hi [0:2047];
  integer ci;
  initial for (ci = 0; ci < 2048; ci = ci + 1) begin
    cells_lo[ci] = 0; cells_hi[ci] = 0;
  end
  // PicoRV32 aligns mem_addr down to the word boundary even for a byte
  // store, so the byte address is the word address plus the position of the
  // single mem_wstrb lane. Only single-lane strobes write a cell: the
  // firmware stores bytes (volatile), and a word store has no single byte
  // address to file under.
  wire [1:0] char_lane = mem_wstrb[1] ? 2'd1 : mem_wstrb[2] ? 2'd2 :
                         mem_wstrb[3] ? 2'd3 : 2'd0;
  wire one_lane = mem_wstrb == (4'b0001 << char_lane);
  wire [11:0] char_waddr = {mem_addr[11:2], char_lane};
  wire char_we = writing && char_selected && char_waddr < 2400 && one_lane;
  wire [7:0] char_wbyte = mem_wdata[8*char_lane +: 8];

  // The screen registers: the top row (for scrolling without copying), the
  // cursor, and the CRC of the last complete frame with its number.
  reg [4:0] top_row = 0;
  reg cursor_en = 0;
  reg [4:0] cursor_row = 0;
  reg [6:0] cursor_col = 0;
  always @(posedge clk)
    if (!resetn) begin
      top_row <= 0; cursor_en <= 0; cursor_row <= 0; cursor_col <= 0;
    end else if (writing && screen_selected) begin
      if (mem_addr[4:2] == 0 && mem_wstrb[0]) top_row <= mem_wdata[4:0];
      if (mem_addr[4:2] == 1) begin
        if (mem_wstrb[0]) cursor_col <= mem_wdata[6:0];
        if (mem_wstrb[1]) cursor_row <= mem_wdata[12:8];
        if (mem_wstrb[2]) cursor_en <= mem_wdata[16];
      end
    end

  // The display pipeline. A cell is 8 pixels wide, so its data is fetched one
  // cell ahead of the beam: the character code at pixel 0 of the previous
  // cell, the glyph row at pixel 2, loaded into the shift register at the
  // cell boundary. vrow/vline track the buffer row and glyph line of the line
  // being scanned; at the last cell the fetch wraps to the next line's first
  // cell (for line 524, to the first line of the next frame). The fetch runs
  // through the blanks with a safe address; the picture is dark there anyway.
  reg [4:0] vrow = 0;
  reg [3:0] vline = 0;
  wire [6:0] col = x[9:3];
  wire [2:0] pix = x[2:0];
  wire [4:0] nrow = y == 524 ? top_row :
                    y >= 479 ? 5'd0 :
                    vline == 15 ? (vrow == 29 ? 5'd0 : vrow + 5'd1) : vrow;
  wire [3:0] nline = y >= 479 ? 4'd0 :
                     vline == 15 ? 4'd0 : vline + 4'd1;
  always @(posedge clk)
    if (x == 799) begin
      if (y == 524) begin vrow <= top_row; vline <= 0; end
      else if (y >= 479) begin vrow <= 0; vline <= 0; end
      else if (vline == 15) begin
        vline <= 0; vrow <= vrow == 29 ? 5'd0 : vrow + 5'd1;
      end else vline <= vline + 4'd1;
    end
  wire last_cell = col == 79;
  wire [4:0] frow = last_cell ? nrow : vrow;
  wire [3:0] fline = last_cell ? nline : vline;
  wire [6:0] fcol = last_cell ? 7'd0 : col + 7'd1;
  wire fetch = pix == 0 && x < 640;
  // frow * 80 + fcol: (frow << 6) + (frow << 4) + fcol, always below 2400.
  wire [11:0] faddr = {frow, 6'd0} + {frow, 4'd0} + fcol;
  // A cell byte read in the clock the CPU writes the same byte would be
  // undefined on the chip and unknown in the emulator, so that read is
  // skipped: the register keeps the previous cell's code, one cell shows a
  // stale glyph for one frame, and the next line's fetch heals it. During a
  // `screen` measurement the CPU writes nothing, so no frame it checks can
  // contain such a cell.
  wire fetch_lo = fetch && !faddr[11] && !(char_we && char_waddr == faddr);
  wire fetch_hi = fetch &&  faddr[11] && !(char_we && char_waddr == faddr);
  reg [7:0] q_lo = 0, q_hi = 0;
  always @(posedge clk) begin
    if (char_we) begin
      if (char_waddr[11]) cells_hi[char_waddr[10:0]] <= char_wbyte;
      else cells_lo[char_waddr[10:0]] <= char_wbyte;
    end
    if (fetch_lo) q_lo <= cells_lo[faddr[10:0]];
    if (fetch_hi) q_hi <= cells_hi[faddr[10:0]];
  end
  reg fblock = 0;
  reg [3:0] fline_r = 0;
  always @(posedge clk)
    if (fetch) begin fblock <= faddr[11]; fline_r <= fline; end

  // The font ROM: 95 glyphs of 16 rows in one EBR, written by make_font.py.
  // Codes outside the printable range draw blank.
  (* rom_style = "block" *) reg [7:0] font [0:2047];
  initial $readmemh("font.hex", font);
  wire [7:0] char_code = fblock ? q_hi : q_lo;
  wire printable = char_code >= 8'd32 && char_code < 8'd127;
  reg [7:0] glyph_q = 0;
  always @(posedge clk)
    if (pix == 2 && x < 640)
      glyph_q <= printable ? font[{char_code - 8'd32, fline_r}] : 8'd0;

  // The shift register: bit 7 is the leftmost pixel of the cell. At the end
  // of a line it loads the first cell of the next line, fetched during this
  // line's last cell and held in glyph_q through the horizontal blank.
  reg [7:0] shifter = 0;
  always @(posedge clk)
    if (x == 799) shifter <= glyph_q;
    else if (x < 640) shifter <= pix == 7 ? glyph_q : {shifter[6:0], 1'b0};

  // The cursor is a blinking block: it inverts the cell at its position at
  // about 2 Hz (16 frames on, 16 off). Its row is logical; the hardware adds
  // the top row, so the cursor stays on its line when the screen scrolls.
  reg [31:0] frame_no = 0;
  wire [5:0] cursor_sum = cursor_row + top_row;
  wire [4:0] cursor_phys = cursor_sum >= 30 ? cursor_sum - 30 : cursor_sum;
  wire cursor_here = cursor_en && frame_no[4] && active &&
                     col == cursor_col && vrow == cursor_phys;
  // Amber on near-black, like a terminal of the 1980s.
  wire pixel_on = shifter[7] ^ cursor_here;
  wire [23:0] rgb = !active ? 24'h000000 :
                    pixel_on ? 24'hffb000 : 24'h0c0c0c;
  video_out video(.shift_clk(shift_clk), .pixel_clk(clk), .rgb(rgb),
    .de(active), .hs(!(x >= 656 && x < 752)),
    .vs(!(y >= 490 && y < 492)), .gpdi_dp(gpdi_dp));

  // Reflected CRC-32/ISO-HDLC: R, then G, then B; bits LSB first in each
  // byte. Copied from examples/ulx3s/hdmi_moving/top.v, so the design's CRC
  // is computed exactly as the emulator's receiver computes it (zlib crc32).
  function [31:0] crc_pixel;
    input [31:0] state;
    input [23:0] color;
    reg [31:0] c;
    reg [7:0] byte_value;
    integer channel, bit_index;
    begin
      c = state;
      for (channel = 2; channel >= 0; channel = channel - 1) begin
        byte_value = color >> (8 * channel);
        for (bit_index = 0; bit_index < 8; bit_index = bit_index + 1)
          c = (c >> 1) ^ ((c[0] ^ byte_value[bit_index]) ? 32'hedb88320 : 0);
      end
      crc_pixel = c;
    end
  endfunction
  reg [31:0] crc = 32'hffffffff;
  reg [31:0] report_crc = 0, report_number = 0;
  wire [31:0] next_crc = crc_pixel(crc, rgb);
  always @(posedge clk) begin
    if (active) crc <= next_crc;
    if (x == 799 && y == 524) begin
      crc <= 32'hffffffff;
      frame_no <= frame_no + 1;
    end
    if (x == 639 && y == 479) begin
      report_crc <= ~next_crc;
      report_number <= frame_no;
    end
  end

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
    if (screen_selected)
      case (mem_addr[4:2])
        3'd0: io_q <= {27'd0, top_row};
        3'd1: io_q <= {15'd0, cursor_en, 3'd0, cursor_row, 1'b0, cursor_col};
        3'd2: io_q <= report_crc;
        3'd3: io_q <= report_number;
        default: io_q <= 0;
      endcase
  end
  assign mem_rdata = from_ram ? bank_q[read_bank] :
                     from_fifo ? {24'd0, fifo_q} : io_q;
endmodule
