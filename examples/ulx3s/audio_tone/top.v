// 25 kHz, 4-bit tones at 440/660 Hz. Codes are limited to 4..12.
// The jack carries DC; the emulator assumes an unplugged, open circuit.
module top(input clk_25mhz, output reg [3:0] audio_l = 8,
           output reg [3:0] audio_r = 8, output [3:0] audio_v,
           output [7:0] led, output ftdi_rxd);
  assign audio_v = 4'd8;
  assign led = {audio_r, audio_l};
  reg [9:0] divider = 0;
  reg [31:0] phase_l = 0, phase_r = 0;
  reg [14:0] sample_count = 0;
  reg [19:0] count_bcd = 0, report_count = 0;
  reg [31:0] crc = 32'hffffffff, report_crc = 0;
  wire [3:0] next_l = sine(phase_l[31:27]);
  wire [3:0] next_r = sine(phase_r[31:27]);
  wire [31:0] next_crc = crc_byte(crc_byte(crc, {4'b0,next_l}), {4'b0,next_r});
  function [3:0] sine;
    input [4:0] phase;
    begin case (phase)
      0: sine = 8;
      1: sine = 9;
      2: sine = 10;
      3: sine = 10;
      4: sine = 11;
      5: sine = 11;
      6: sine = 12;
      7: sine = 12;
      8: sine = 12;
      9: sine = 12;
      10: sine = 12;
      11: sine = 11;
      12: sine = 11;
      13: sine = 10;
      14: sine = 10;
      15: sine = 9;
      16: sine = 8;
      17: sine = 7;
      18: sine = 6;
      19: sine = 6;
      20: sine = 5;
      21: sine = 5;
      22: sine = 4;
      23: sine = 4;
      24: sine = 4;
      25: sine = 4;
      26: sine = 4;
      27: sine = 5;
      28: sine = 5;
      29: sine = 6;
      30: sine = 6;
      31: sine = 7;
    endcase end
  endfunction
  // CRC-32/ISO-HDLC: reflected EDB88320, initial/final XOR FFFFFFFF.
  // Two bytes per sample, L then R, each zero-extended from four bits.
  function [31:0] crc_byte;
    input [31:0] c;
    input [7:0] b;
    reg [31:0] v;
    integer bit_index;
    begin
      v = c ^ b;
      for (bit_index=0; bit_index<8; bit_index=bit_index+1)
        v = (v >> 1) ^ (v[0] ? 32'hedb88320 : 0);
      crc_byte = v;
    end
  endfunction
  function [19:0] bcd_next;
    input [19:0] n;
    reg carry;
    integer digit;
    begin
      bcd_next = n; carry = 1;
      for (digit=0; digit<5; digit=digit+1) begin
        if (carry) begin
          if (n[digit*4 +: 4] == 9) bcd_next[digit*4 +: 4] = 0;
          else begin bcd_next[digit*4 +: 4] = n[digit*4 +: 4] + 1; carry = 0; end
        end
      end
    end
  endfunction
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? "0"+n : "A"+n-10; end
  endfunction
  reg [1:0] tx_state = 0;
  reg [4:0] char_index = 0;
  reg send = 0;
  reg [7:0] char_data = 0;
  wire busy;
  uart_tx uart(clk_25mhz, send, char_data, busy, ftdi_rxd);
  always @(posedge clk_25mhz) begin
    send <= 0;
    if (divider == 999) begin
      divider <= 0;
      audio_l <= next_l; audio_r <= next_r;
      // Rounded 2^32 * frequency / 25000, phase continuous across seconds.
      phase_l <= phase_l + 32'd75591424;
      phase_r <= phase_r + 32'd113387137;
      crc <= next_crc;
      count_bcd <= bcd_next(count_bcd);
      if (sample_count == 24999) begin
        sample_count <= 0; count_bcd <= 0; crc <= 32'hffffffff;
        report_count <= bcd_next(count_bcd); report_crc <= ~next_crc;
        char_index <= 0; tx_state <= 1;
      end else sample_count <= sample_count + 1;
    end else divider <= divider + 1;
    case (tx_state)
      1: begin
        // AUDIO <decimal sample pairs in the last second> <8 hex CRC>, CRLF.
        case (char_index)
          0: char_data <= "A"; 1: char_data <= "U"; 2: char_data <= "D";
          3: char_data <= "I"; 4: char_data <= "O";
          5,11: char_data <= " ";
          20: char_data <= 13; 21: char_data <= 10;
          default: if (char_index < 11)
            char_data <= "0" + ((report_count >> ((10-char_index)*4)) & 15);
          else char_data <= hex(report_crc >> ((19-char_index)*4));
        endcase
        tx_state <= 2;
      end
      2: if (!busy && !send) begin send <= 1; tx_state <= 3; end
      3: if (busy) begin
        if (char_index == 21) tx_state <= 0;
        else begin char_index <= char_index + 1; tx_state <= 1; end
      end
    endcase
  end
endmodule

// 115200 8N1, 217 clocks per bit at 25 MHz.
module uart_tx(input clk, input send, input [7:0] data, output busy, output tx);
  reg [9:0] frame = 10'h3ff;
  reg [3:0] bits = 0;
  reg [7:0] baud = 0;
  assign busy = bits != 0;
  assign tx = frame[0];
  always @(posedge clk)
    if (busy) begin
      if (baud == 216) begin
        baud <= 0; frame <= {1'b1,frame[9:1]}; bits <= bits - 1;
      end else baud <= baud + 1;
    end else if (send) begin
      frame <= {1'b1,data,1'b0}; bits <= 10; baud <= 0;
    end
endmodule
