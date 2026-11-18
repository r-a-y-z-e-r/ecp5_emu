// EBR at zero contains the boot ROM and firmware image. Boot copies the
// payload to 0x40000000 and jumps there. Code, data and stack then use SDRAM.
// UART: 0x02000000 data, 0x02000004 TX busy; LEDs: 0x03000000.
module top(
  input clk_25mhz, output ftdi_rxd, output [7:0] led,
  output sdram_clk, output sdram_cke, output sdram_csn,
  output sdram_rasn, output sdram_casn, output sdram_wen,
  output [1:0] sdram_ba, output [12:0] sdram_a,
  output [1:0] sdram_dqm, inout [15:0] sdram_d
);
  wire clk = clk_25mhz;
  reg [5:0] reset_count = 0;
  wire resetn = &reset_count;
  always @(posedge clk) if (!resetn) reset_count <= reset_count + 1;
  wire mem_valid, mem_instr;
  wire [31:0] mem_addr, mem_wdata;
  wire [3:0] mem_wstrb;
  wire [31:0] mem_rdata;
  reg mem_ready = 0;
  picorv32 cpu(.clk(clk),.resetn(resetn),.mem_valid(mem_valid),.mem_instr(mem_instr),
    .mem_ready(mem_ready),.mem_addr(mem_addr),.mem_wdata(mem_wdata),
    .mem_wstrb(mem_wstrb),.mem_rdata(mem_rdata));
  wire request = mem_valid && !mem_ready;
  wire rom_selected = mem_addr[31:24] == 0;
  wire sdram_selected = mem_addr[31:25] == 7'h20;
  wire uart_selected = mem_addr[31:24] == 2;
  wire led_selected = mem_addr[31:24] == 3;
  wire writing = request && mem_wstrb != 0;

  // Four byte lanes, as in picorv32_console, for ecpbram firmware swapping.
  // Boot only reads this region; byte writes remain available for debugging.
  (* ram_style = "block" *) reg [7:0] lane0 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane1 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane2 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane3 [0:2047];
  initial $readmemh("fw0.hex",lane0);
  initial $readmemh("fw1.hex",lane1);
  initial $readmemh("fw2.hex",lane2);
  initial $readmemh("fw3.hex",lane3);
  wire [10:0] word = mem_addr[12:2];
  reg [31:0] rom_q;
  always @(posedge clk) if (request && rom_selected) begin
    if (mem_wstrb[0]) lane0[word] <= mem_wdata[7:0];
    if (mem_wstrb[1]) lane1[word] <= mem_wdata[15:8];
    if (mem_wstrb[2]) lane2[word] <= mem_wdata[23:16];
    if (mem_wstrb[3]) lane3[word] <= mem_wdata[31:24];
    rom_q <= {lane3[word],lane2[word],lane1[word],lane0[word]};
  end

  reg ram_req = 0, ram_wr = 0;
  reg [23:0] ram_addr = 0;
  reg [31:0] saved_data = 0;
  reg [3:0] saved_strobes = 0;
  reg half = 0;
  wire ram_ready;
  wire [15:0] ram_data;
  reg [31:0] sdram_q = 0;
  sdram_controller ram(.clk(clk),.req(ram_req),.wr(ram_wr),.addr(ram_addr),
    .wdata(half ? saved_data[31:16] : saved_data[15:0]),
    .mask(half ? ~saved_strobes[3:2] : ~saved_strobes[1:0]),
    .ready(ram_ready),.rdata(ram_data),
    .sdram_clk(sdram_clk),.sdram_cke(sdram_cke),.sdram_csn(sdram_csn),
    .sdram_rasn(sdram_rasn),.sdram_casn(sdram_casn),.sdram_wen(sdram_wen),
    .sdram_ba(sdram_ba),.sdram_a(sdram_a),.sdram_dqm(sdram_dqm),.sdram_d(sdram_d));
  reg [2:0] bus_state = 0;
  always @(posedge clk) begin
    mem_ready <= 0;
    case (bus_state)
      0: if (request) begin
           if (sdram_selected) begin
             saved_data <= mem_wdata; saved_strobes <= mem_wstrb;
             ram_wr <= |mem_wstrb; ram_addr <= {mem_addr[24:2],1'b0};
             half <= 0; ram_req <= 1; bus_state <= 1;
           end else mem_ready <= 1;
         end
      1: if (ram_ready) begin
           sdram_q[15:0] <= ram_data; ram_req <= 0; bus_state <= 2;
         end
      2: begin half <= 1; ram_addr[0] <= 1; ram_req <= 1; bus_state <= 3; end
      3: if (ram_ready) begin
           sdram_q[31:16] <= ram_data; ram_req <= 0;
           mem_ready <= 1; bus_state <= 4;
         end
      4: bus_state <= 0;
    endcase
  end

  reg [9:0] tx_frame = 10'h3ff;
  reg [3:0] tx_bits = 0;
  reg [7:0] tx_baud = 0;
  wire tx_busy = tx_bits != 0;
  always @(posedge clk)
    if (tx_busy) begin
      if (tx_baud == 216) begin
        tx_baud <= 0; tx_frame <= {1'b1,tx_frame[9:1]}; tx_bits <= tx_bits - 1;
      end else tx_baud <= tx_baud + 1;
    end else if (writing && uart_selected && !mem_addr[2]) begin
      tx_frame <= {1'b1,mem_wdata[7:0],1'b0}; tx_bits <= 10;
    end
  assign ftdi_rxd = tx_frame[0];
  reg [7:0] leds = 0;
  always @(posedge clk) if (writing && led_selected && mem_wstrb[0]) leds <= mem_wdata[7:0];
  assign led = leds;
  reg [31:0] io_q = 0;
  always @(posedge clk) if (request) io_q <= mem_addr[2] ? {31'd0,tx_busy} : 0;
  assign mem_rdata = rom_selected ? rom_q : sdram_selected ? sdram_q : io_q;
endmodule
