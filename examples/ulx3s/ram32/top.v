// A 32-bit wide RAM and ROM in block RAM, the way yosys infers them: each
// is one EBR read 36 bits at a time (PDPW16KD, DATA_WIDTH_R=36).
// The ROM holds 512 words from rom.hex; the RAM is filled with a 32-bit LFSR
// sequence and read back against the same sequence. About ten times a second
// the UART (115200 8N1) prints
//   "ROM <sum of the ROM words> RAM <words read back wrong>\r\n"
// in hex, and the LEDs show the number of passes; led[7] lights on an error.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  reg [31:0] rom [0:511];
  initial $readmemh("rom.hex", rom);
  reg [31:0] ram [0:511];

  // One pass: write 512 words, read them back with the ROM alongside, then
  // print the line and wait 0.1 s.
  localparam WRITE = 0, READ = 1, PRINT = 2, WAIT = 3;
  reg [1:0] phase = WRITE;
  reg [9:0] addr = 0;           // one extra cycle for the read latency
  reg [31:0] lfsr = 32'h1, check = 32'h1;
  reg [31:0] rom_q = 0, ram_q = 0;
  reg [31:0] sum = 0;
  reg [15:0] errors = 0;
  reg [7:0] passes = 0;
  reg reading = 0;
  function [31:0] step(input [31:0] x);
    step = {x[30:0], x[31] ^ x[21] ^ x[1] ^ x[0]};
  endfunction

  // UART, 115200 8N1 from 25 MHz.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [4:0] ci = 0;
  reg [7:0] ch;
  always @* case (ci)
    0: ch = "R"; 1: ch = "O"; 2: ch = "M"; 3: ch = " ";
    4: ch = hex(sum[31:28]); 5: ch = hex(sum[27:24]); 6: ch = hex(sum[23:20]);
    7: ch = hex(sum[19:16]); 8: ch = hex(sum[15:12]); 9: ch = hex(sum[11:8]);
    10: ch = hex(sum[7:4]); 11: ch = hex(sum[3:0]);
    12: ch = " "; 13: ch = "R"; 14: ch = "A"; 15: ch = "M"; 16: ch = " ";
    17: ch = hex(errors[15:12]); 18: ch = hex(errors[11:8]);
    19: ch = hex(errors[7:4]); 20: ch = hex(errors[3:0]);
    21: ch = 8'h0d; default: ch = 8'h0a;
  endcase
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [21:0] pause = 0;
  reg failed = 0;

  always @(posedge clk_25mhz) begin
    rom_q <= rom[addr[8:0]];
    ram_q <= ram[addr[8:0]];
    reading <= phase == READ && addr < 512;
    if (reading) begin
      sum <= sum + rom_q;
      if (ram_q != check) errors <= errors + 1;
      check <= step(check);
    end
    case (phase)
      WRITE: begin
        ram[addr[8:0]] <= lfsr;
        lfsr <= step(lfsr);
        addr <= addr == 511 ? 0 : addr + 1;
        if (addr == 511) phase <= READ;
      end
      READ: begin
        addr <= addr + 1;
        if (addr == 513) begin phase <= PRINT; ci <= 0; failed <= failed | (errors != 0); end
      end
      PRINT:
        if (!busy) begin
          shift <= {1'b1, ch, 1'b0};
          busy <= 1; baud <= 0; bitn <= 0;
        end else if (baud == 216) begin
          baud <= 0;
          tx <= shift[0];
          shift <= {1'b1, shift[9:1]};
          bitn <= bitn + 1;
          if (bitn == 9) begin
            busy <= 0;
            if (ci == 22) begin phase <= WAIT; pause <= 22'd2500000; end
            else ci <= ci + 1;
          end
        end else baud <= baud + 1;
      WAIT:
        if (pause != 0) pause <= pause - 1;
        else begin
          phase <= WRITE; addr <= 0; lfsr <= 32'h1; check <= 32'h1;
          sum <= 0; errors <= 0; passes <= passes + 1;
        end
    endcase
  end
  assign ftdi_rxd = tx;
  assign led = {failed, passes[6:0]};
endmodule
