// EBR boot and stack at zero; read-only XIP flash at 0x10000000.
// UART 0x02000000/+4, fixed RDID/RDSR at 0x04000000/+4.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led,
  output flash_csn, output flash_mosi, input flash_miso,
  output flash_wpn, output flash_holdn);
  wire clk = clk_25mhz;
  reg [5:0] reset_count = 0;
  wire resetn = &reset_count;
  always @(posedge clk) if (!resetn) reset_count <= reset_count + 1;
  wire valid;
  wire [31:0] addr, wdata;
  wire [3:0] wstrb;
  reg ready = 0;
  wire [31:0] rdata;
  picorv32 cpu(.clk(clk),.resetn(resetn),.mem_valid(valid),.mem_ready(ready),
    .mem_addr(addr),.mem_wdata(wdata),.mem_wstrb(wstrb),.mem_rdata(rdata));
  wire request = valid && !ready;
  wire ram = addr[31:13] == 0;
  wire flash = addr[31:24] == 8'h10;
  wire id = addr[31:24] == 4;
  wire uart = addr[31:24] == 2;
  wire writing = request && |wstrb;
  (* ram_style = "block" *) reg [7:0] lane0 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane1 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane2 [0:2047];
  (* ram_style = "block" *) reg [7:0] lane3 [0:2047];
  initial $readmemh("fw0.hex",lane0);
  initial $readmemh("fw1.hex",lane1);
  initial $readmemh("fw2.hex",lane2);
  initial $readmemh("fw3.hex",lane3);
  wire [10:0] word = addr[12:2];
  reg [31:0] ram_q;
  always @(posedge clk) if (request && ram) begin
    if (wstrb[0]) lane0[word] <= wdata[7:0];
    if (wstrb[1]) lane1[word] <= wdata[15:8];
    if (wstrb[2]) lane2[word] <= wdata[23:16];
    if (wstrb[3]) lane3[word] <= wdata[31:24];
    ram_q <= {lane3[word],lane2[word],lane1[word],lane0[word]};
  end
  reg start = 0;
  reg [1:0] op = 0;
  reg [23:0] flash_addr = 0;
  wire flash_ready;
  wire [31:0] flash_data;
  flash_reader reader(.clk(clk),.start(start),.op(op),.addr(flash_addr),
    .ready(flash_ready),.data(flash_data),.flash_csn(flash_csn),
    .flash_mosi(flash_mosi),.flash_miso(flash_miso),
    .flash_wpn(flash_wpn),.flash_holdn(flash_holdn),.sck());
  reg [31:0] flash_q = 0;
  reg [21:0] cached_addr = 0;
  reg [31:0] cached_data = 0;
  reg cached = 0;
  reg [1:0] state = 0;
  reg is_id = 0;
  always @(posedge clk) begin
    ready <= 0; start <= 0;
    case (state)
      0: if (request) begin
        if ((flash || id) && !writing) begin
          if (flash && cached && cached_addr == addr[23:2]) begin
            flash_q <= cached_data; ready <= 1; state <= 2;
          end else begin
            is_id <= id; op <= id ? (addr[2] ? 2 : 1) : 0;
            flash_addr <= {addr[23:2],2'b00}; start <= 1; state <= 1;
          end
        end else begin ready <= 1; state <= 2; end
      end
      1: if (flash_ready) begin
        // SPI is big-endian on the wire; the CPU reads little-endian words.
        flash_q <= is_id ? flash_data : {flash_data[7:0],flash_data[15:8],flash_data[23:16],flash_data[31:24]};
        if (!is_id) begin
          cached <= 1; cached_addr <= flash_addr[23:2];
          cached_data <= {flash_data[7:0],flash_data[15:8],flash_data[23:16],flash_data[31:24]};
        end
        ready <= 1; state <= 2;
      end
      2: state <= 0;
    endcase
  end
  wire tx_busy;
  uart_tx transmitter(.clk(clk),.send(writing && uart && !addr[2]),
    .data(wdata[7:0]),.busy(tx_busy),.tx(ftdi_rxd));
  reg [31:0] io_q = 0;
  always @(posedge clk) if (request) io_q <= addr[2] ? {31'd0,tx_busy} : 0;
  assign rdata = ram ? ram_q : (flash || id) ? flash_q : io_q;
  assign led = {6'd0,flash_csn,resetn};
endmodule
