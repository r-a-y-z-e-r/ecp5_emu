// MAX11125 at 390.625 kHz, mode 3, manual channel selection, CHAN_ID on.
// Header pins are INPUTS ONLY. The LPF supplies alternating weak pulls.
module top(input clk_25mhz, input adc_miso,
           (* keep *) input [7:0] adc_ain,
           output reg adc_csn = 1, output reg adc_sclk = 1,
           output reg adc_mosi = 0, output [7:0] led, output ftdi_rxd);
  // Retain the input buffers and their pulls even though only U8 reads them.
  (* keep *) wire [7:0] analog_pads = adc_ain;
  reg [11:0] codes [0:7];
  integer i;
  initial for (i=0; i<8; i=i+1) codes[i] = 0;
  assign led = codes[0][11:4];
  reg [22:0] period = 0;
  reg [15:0] startup = 0;
  reg ready = 0;
  reg [3:0] state = 0;
  reg [4:0] frame = 0;
  reg [15:0] tx_word = 0, rx_word = 0;
  reg [4:0] bit_index = 0;
  reg [5:0] divider = 0;
  reg [5:0] char_index = 0;
  reg send = 0;
  reg [7:0] char_data = 0;
  wire busy;
  uart_tx uart(clk_25mhz, send, char_data, busy, ftdi_rxd);
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? "0"+n : "A"+n-10; end
  endfunction
  always @(posedge clk_25mhz) begin
    send <= 0;
    if (!ready) begin
      startup <= startup + 1;
      if (startup == 65535) ready <= 1; // >2 ms for power-up and weak pulls
    end else begin
      if (period == 4999999) period <= 0; else period <= period + 1;
      case (state)
        0: begin frame <= 0; state <= 1; end
        1: begin
          // Configure normal operation, reset single-ended registers, then
          // prime channel 0 twice before selecting channels 1..7 and flushing.
          case (frame)
            0: tx_word <= 16'h8000; // ADC configuration: normal, REF- grounded
            1: tx_word <= 16'h8800; // Unipolar: all single-ended
            2: tx_word <= 16'h9000; // Bipolar: all disabled
            3: tx_word <= 16'h9800; // RANGE: default
            4,5,14: tx_word <= 16'h0804; // Manual, AIN0, CHAN_ID; flush AIN7 at 14
            default: tx_word <= 16'h0804 | ((frame-6) << 7);
          endcase
          adc_csn <= 0; divider <= 0; bit_index <= 0; rx_word <= 0; state <= 2;
        end
        2: begin
          divider <= divider + 1;
          if (divider == 31) begin
            adc_sclk <= 0; adc_mosi <= tx_word[15]; tx_word <= {tx_word[14:0],1'b0};
          end
          if (divider == 63) begin
            adc_sclk <= 1; rx_word <= {rx_word[14:0],adc_miso};
            if (bit_index == 15) state <= 3;
            else bit_index <= bit_index + 1;
          end
        end
        3: begin
          // Keep SCLK high for >one period before the next CS falling edge.
          divider <= divider + 1;
          if (divider == 31) adc_csn <= 1;
          if (divider == 63) begin
            if (frame >= 7) codes[frame-7] <= rx_word[11:0];
            if (frame == 14) begin char_index <= 0; state <= 4; end
            else begin frame <= frame + 1; state <= 1; end
          end
        end
        4: begin
          // ADC c0 c1 ... c7, four hex digits per 12-bit code, CRLF.
          case (char_index)
            0: char_data <= "A"; 1: char_data <= "D"; 2: char_data <= "C";
            43: char_data <= 13; 44: char_data <= 10;
            default: begin
              if ((char_index-3)%5 == 0) char_data <= " ";
              else char_data <= hex({4'b0,codes[(char_index-4)/5]} >>
                                   ((3-(char_index-4)%5)*4));
            end
          endcase
          state <= 5;
        end
        5: if (!busy && !send) begin send <= 1; state <= 6; end
        6: if (busy) begin
          if (char_index == 44) state <= 7;
          else begin char_index <= char_index + 1; state <= 4; end
        end
        7: if (period == 4999999) state <= 0;
      endcase
    end
  end
endmodule
// 115200 8N1, 217 board clocks per bit.
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
