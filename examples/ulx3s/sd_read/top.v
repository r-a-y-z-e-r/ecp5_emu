// SDHC card SPI-mode reader in the 25 MHz domain, plain fabric logic only.
// Initialises at ~391 kHz (80 wake clocks with CS high, CMD0, CMD8, ACMD41,
// CMD58), switches to 6.25 MHz, reads block 0, and prints OCR and block 0
// header/tail on the 115200 8N1 UART every second.
module top(input clk_25mhz,
  output reg sd_clk = 0, output reg sd_cmd = 1,
  input \sd_d[0] , output \sd_d[3] ,
  output ftdi_rxd, output [7:0] led);

  reg sd_cs = 1;
  assign \sd_d[3] = sd_cs;
  wire sd_miso = \sd_d[0] ;

  function [7:0] hex;
    input [3:0] nibble;
    begin
      if (nibble < 10) hex = "0" + nibble;
      else hex = "A" + nibble - 10;
    end
  endfunction

  function [7:0] cmd_byte;
    input [7:0] step;
    input [2:0] idx;
    begin
      case ({step[2:0], idx})
        // CMD0: 40 00 00 00 00 95
        {3'd1, 3'd0}: cmd_byte = 8'h40;
        {3'd1, 3'd5}: cmd_byte = 8'h95;
        // CMD8: 48 00 00 01 AA 87
        {3'd2, 3'd0}: cmd_byte = 8'h48;
        {3'd2, 3'd3}: cmd_byte = 8'h01;
        {3'd2, 3'd4}: cmd_byte = 8'haa;
        {3'd2, 3'd5}: cmd_byte = 8'h87;
        // CMD55: 77 00 00 00 00 65
        {3'd3, 3'd0}: cmd_byte = 8'h77;
        {3'd3, 3'd5}: cmd_byte = 8'h65;
        // ACMD41 (HCS=1): 69 40 00 00 00 77
        {3'd4, 3'd0}: cmd_byte = 8'h69;
        {3'd4, 3'd1}: cmd_byte = 8'h40;
        {3'd4, 3'd5}: cmd_byte = 8'h77;
        // CMD58: 7A 00 00 00 00 FD
        {3'd5, 3'd0}: cmd_byte = 8'h7a;
        {3'd5, 3'd5}: cmd_byte = 8'hfd;
        // CMD17 block 0: 51 00 00 00 00 55
        {3'd6, 3'd0}: cmd_byte = 8'h51;
        {3'd6, 3'd5}: cmd_byte = 8'h55;
        default: cmd_byte = 8'h00;
      endcase
    end
  endfunction

  // Mode-0 SPI byte engine: slow_mode=1 -> 25 MHz / 64 (~391 kHz);
  // slow_mode=0 -> 25 MHz / 4 (6.25 MHz).
  reg slow_mode = 1;
  reg spi_start = 0, spi_busy = 0, spi_done = 0;
  reg [7:0] spi_tx = 8'hff, tx_shift = 8'hff, spi_rx = 0, rx_shift = 0;
  reg [3:0] sub_cnt = 0;
  reg [1:0] spi_phase = 0;
  reg [2:0] spi_bit = 0;

  always @(posedge clk_25mhz) begin
    spi_done <= 0;
    if (spi_start && !spi_busy) begin
      spi_busy <= 1;
      sd_clk <= 0;
      sd_cmd <= spi_tx[7];
      tx_shift <= spi_tx;
      spi_bit <= 7;
      spi_phase <= 0;
      sub_cnt <= 0;
    end else if (spi_busy) begin
      if (slow_mode && sub_cnt != 4'd15) begin
        sub_cnt <= sub_cnt + 1;
      end else begin
        sub_cnt <= 0;
        spi_phase <= spi_phase + 1;
        if (spi_phase == 2'd1) begin
          sd_clk <= 1;
        end else if (spi_phase == 2'd2) begin
          rx_shift <= {rx_shift[6:0], sd_miso};
        end else if (spi_phase == 2'd3) begin
          sd_clk <= 0;
          if (spi_bit == 0) begin
            spi_busy <= 0;
            spi_done <= 1;
            spi_rx <= rx_shift;
            sd_cmd <= 1;
          end else begin
            spi_bit <= spi_bit - 1;
            tx_shift <= {tx_shift[6:0], 1'b0};
            sd_cmd <= tx_shift[6];
          end
        end
      end
    end
  end

  // 115200 8N1 UART transmitter (25 MHz / 217 = 115207 baud).
  reg uart_start = 0;
  reg [7:0] uart_data = 0;
  reg [9:0] uart_shift = 10'h3ff;
  reg [3:0] uart_bits = 0;
  reg [7:0] baud = 0;
  wire uart_busy = uart_bits != 0;
  assign ftdi_rxd = uart_shift[0];

  always @(posedge clk_25mhz)
    if (uart_busy) begin
      if (baud == 8'd216) begin
        baud <= 0;
        uart_shift <= {1'b1, uart_shift[9:1]};
        uart_bits <= uart_bits - 1;
      end else baud <= baud + 1;
    end else if (uart_start) begin
      uart_shift <= {1'b1, uart_data, 1'b0};
      uart_bits <= 10;
      baud <= 0;
    end

  // Main SD initialization, block 0 read, and periodic UART report FSM.
  localparam
    ST_WAKE            = 4'd0,
    ST_WAKE_WAIT       = 4'd1,
    ST_CMD_START       = 4'd2,
    ST_CMD_SEND        = 4'd3,
    ST_CMD_WAIT        = 4'd4,
    ST_POLL_R1         = 4'd5,
    ST_POLL_R1_WAIT    = 4'd6,
    ST_AFTER_R1        = 4'd7,
    ST_READ_32         = 4'd8,
    ST_READ_32_WAIT    = 4'd9,
    ST_WAIT_TOKEN      = 4'd10,
    ST_WAIT_TOKEN_WAIT = 4'd11,
    ST_READ_DATA       = 4'd12,
    ST_READ_DATA_WAIT  = 4'd13,
    ST_GAP_CS_HIGH     = 4'd14,
    ST_GAP_WAIT        = 4'd15;

  // UART / idle sub-states are entered once ST_DONE is reached.
  reg [3:0] state = ST_WAKE;
  reg [2:0] uart_state = 0;
  localparam
    U_IDLE       = 3'd0,
    U_SEND       = 3'd1,
    U_WAIT_START = 3'd2,
    U_WAIT_END   = 3'd3,
    U_PAUSE      = 3'd4;

  reg [7:0] step_reached = 0;
  reg [7:0] next_step = 0;
  reg [3:0] byte_cnt = 0;
  reg [15:0] poll_cnt = 0;
  reg [9:0] data_cnt = 0;
  // A real card may stay idle for up to a second of ACMD41 polls (each
  // CMD55 + ACMD41 pair takes about 0.4 ms at 391 kHz): up to 4095, 1.6 s.
  reg [11:0] acmd_tries = 0;
  reg [7:0] r1 = 0;
  reg [31:0] r32 = 0;
  reg [31:0] ocr = 0;
  reg [127:0] b0_head = 0;
  reg [15:0] b0_tail = 0;

  reg err_flag = 0;
  reg [7:0] err_step = 0;
  reg [7:0] err_r1 = 0;

  reg [5:0] text_idx = 0;
  reg [24:0] pause_cnt = 0;

  assign led = step_reached;

  reg [7:0] text_byte;
  always @* begin
    if (err_flag) begin
      case (text_idx)
        6'd0:  text_byte = "S";
        6'd1:  text_byte = "D";
        6'd2:  text_byte = " ";
        6'd3:  text_byte = "E";
        6'd4:  text_byte = "R";
        6'd5:  text_byte = "R";
        6'd6:  text_byte = " ";
        6'd7:  text_byte = hex(err_step[7:4]);
        6'd8:  text_byte = hex(err_step[3:0]);
        6'd9:  text_byte = " ";
        6'd10: text_byte = hex(err_r1[7:4]);
        6'd11: text_byte = hex(err_r1[3:0]);
        6'd12: text_byte = 8'h0d;
        default: text_byte = 8'h0a;
      endcase
    end else begin
      if (text_idx <= 6'd6) begin
        case (text_idx)
          6'd0: text_byte = "S";
          6'd1: text_byte = "D";
          6'd2: text_byte = " ";
          6'd3: text_byte = "O";
          6'd4: text_byte = "C";
          6'd5: text_byte = "R";
          default: text_byte = " ";
        endcase
      end else if (text_idx <= 6'd14) begin
        text_byte = hex(ocr[(6'd14 - text_idx) * 4 +: 4]);
      end else if (text_idx <= 6'd18) begin
        case (text_idx)
          6'd15: text_byte = " ";
          6'd16: text_byte = "B";
          6'd17: text_byte = "0";
          default: text_byte = " ";
        endcase
      end else if (text_idx <= 6'd50) begin
        text_byte = hex(b0_head[(6'd50 - text_idx) * 4 +: 4]);
      end else if (text_idx == 6'd51) begin
        text_byte = " ";
      end else if (text_idx <= 6'd55) begin
        text_byte = hex(b0_tail[(6'd55 - text_idx) * 4 +: 4]);
      end else if (text_idx == 6'd56) begin
        text_byte = 8'h0d;
      end else begin
        text_byte = 8'h0a;
      end
    end
  end

  wire [5:0] last_text_idx = err_flag ? 6'd13 : 6'd57;

  always @(posedge clk_25mhz) begin
    spi_start <= 0;
    uart_start <= 0;

    if (uart_state != U_IDLE) begin
      case (uart_state)
        U_SEND: begin
          uart_data <= text_byte;
          uart_start <= 1;
          uart_state <= U_WAIT_START;
        end
        U_WAIT_START: if (uart_busy) uart_state <= U_WAIT_END;
        U_WAIT_END: if (!uart_busy) begin
          if (text_idx == last_text_idx) begin
            pause_cnt <= 25'd24999999;
            uart_state <= U_PAUSE;
          end else begin
            text_idx <= text_idx + 1;
            uart_state <= U_SEND;
          end
        end
        U_PAUSE: begin
          if (pause_cnt == 0) begin
            text_idx <= 0;
            uart_state <= U_SEND;
          end else begin
            pause_cnt <= pause_cnt - 1;
          end
        end
        default: uart_state <= U_IDLE;
      endcase
    end else begin
      case (state)
        ST_WAKE: begin
          sd_cs <= 1;
          slow_mode <= 1;
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_WAKE_WAIT;
        end
        ST_WAKE_WAIT: if (spi_done) begin
          if (byte_cnt == 4'd9) begin
            byte_cnt <= 0;
            step_reached <= 8'd1;
            state <= ST_CMD_START;
          end else begin
            byte_cnt <= byte_cnt + 1;
            state <= ST_WAKE;
          end
        end
        ST_CMD_START: begin
          sd_cs <= 0;
          byte_cnt <= 0;
          poll_cnt <= 0;
          state <= ST_CMD_SEND;
        end
        ST_CMD_SEND: begin
          spi_tx <= cmd_byte(step_reached, byte_cnt[2:0]);
          spi_start <= 1;
          state <= ST_CMD_WAIT;
        end
        ST_CMD_WAIT: if (spi_done) begin
          if (byte_cnt == 4'd5) begin
            byte_cnt <= 0;
            state <= ST_POLL_R1;
          end else begin
            byte_cnt <= byte_cnt + 1;
            state <= ST_CMD_SEND;
          end
        end
        ST_POLL_R1: begin
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_POLL_R1_WAIT;
        end
        ST_POLL_R1_WAIT: if (spi_done) begin
          if (!spi_rx[7]) begin
            r1 <= spi_rx;
            state <= ST_AFTER_R1;
          end else if (poll_cnt == 16'd31) begin
            sd_cs <= 1;
            err_flag <= 1;
            err_step <= step_reached;
            err_r1 <= spi_rx;
            text_idx <= 0;
            uart_state <= U_SEND;
          end else begin
            poll_cnt <= poll_cnt + 1;
            state <= ST_POLL_R1;
          end
        end
        ST_AFTER_R1: begin
          case (step_reached)
            8'd1: begin
              if (r1 != 8'h01) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                next_step <= 8'd2;
                state <= ST_GAP_CS_HIGH;
              end
            end
            8'd2: begin
              if (r1 != 8'h01) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                byte_cnt <= 0;
                state <= ST_READ_32;
              end
            end
            8'd3: begin
              if (r1 > 8'h01) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                next_step <= 8'd4;
                state <= ST_GAP_CS_HIGH;
              end
            end
            8'd4: begin
              if (r1 == 8'h00) begin
                next_step <= 8'd5;
                state <= ST_GAP_CS_HIGH;
              end else if (r1 == 8'h01 && acmd_tries != 12'hfff) begin
                acmd_tries <= acmd_tries + 1;
                next_step <= 8'd3;
                state <= ST_GAP_CS_HIGH;
              end else begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end
            end
            8'd5: begin
              if (r1 != 8'h00) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                byte_cnt <= 0;
                state <= ST_READ_32;
              end
            end
            8'd6: begin
              if (r1 != 8'h00) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= r1;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                poll_cnt <= 0;
                state <= ST_WAIT_TOKEN;
              end
            end
            default: begin
              sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= 8'hff;
              text_idx <= 0; uart_state <= U_SEND;
            end
          endcase
        end
        ST_READ_32: begin
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_READ_32_WAIT;
        end
        ST_READ_32_WAIT: if (spi_done) begin
          r32 <= {r32[23:0], spi_rx};
          if (byte_cnt == 4'd3) begin
            if (step_reached == 8'd2) begin
              if ({r32[3:0], spi_rx} != 12'h1aa) begin
                sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= 8'hee;
                text_idx <= 0; uart_state <= U_SEND;
              end else begin
                next_step <= 8'd3;
                state <= ST_GAP_CS_HIGH;
              end
            end else begin
              ocr <= {r32[23:0], spi_rx};
              slow_mode <= 0;
              next_step <= 8'd6;
              state <= ST_GAP_CS_HIGH;
            end
          end else begin
            byte_cnt <= byte_cnt + 1;
            state <= ST_READ_32;
          end
        end
        ST_WAIT_TOKEN: begin
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_WAIT_TOKEN_WAIT;
        end
        ST_WAIT_TOKEN_WAIT: if (spi_done) begin
          if (spi_rx == 8'hfe) begin
            data_cnt <= 0;
            state <= ST_READ_DATA;
          end else if (spi_rx != 8'hff || poll_cnt == 16'hffff) begin
            sd_cs <= 1; err_flag <= 1; err_step <= step_reached; err_r1 <= spi_rx;
            text_idx <= 0; uart_state <= U_SEND;
          end else begin
            poll_cnt <= poll_cnt + 1;
            state <= ST_WAIT_TOKEN;
          end
        end
        ST_READ_DATA: begin
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_READ_DATA_WAIT;
        end
        ST_READ_DATA_WAIT: if (spi_done) begin
          if (data_cnt < 10'd16) b0_head <= {b0_head[119:0], spi_rx};
          if (data_cnt == 10'd510 || data_cnt == 10'd511) b0_tail <= {b0_tail[7:0], spi_rx};
          if (data_cnt == 10'd513) begin
            next_step <= 8'd7;
            state <= ST_GAP_CS_HIGH;
          end else begin
            data_cnt <= data_cnt + 1;
            state <= ST_READ_DATA;
          end
        end
        ST_GAP_CS_HIGH: begin
          sd_cs <= 1;
          spi_tx <= 8'hff;
          spi_start <= 1;
          state <= ST_GAP_WAIT;
        end
        ST_GAP_WAIT: if (spi_done) begin
          step_reached <= next_step;
          if (next_step == 8'd7) begin
            text_idx <= 0;
            uart_state <= U_SEND;
          end else begin
            state <= ST_CMD_START;
          end
        end
        default: state <= ST_WAKE;
      endcase
    end
  end
endmodule
