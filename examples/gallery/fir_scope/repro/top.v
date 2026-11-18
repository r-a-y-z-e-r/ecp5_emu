// Minimal ALU54B MAC reproducer: drives dsp_mac with a fixed script and
// prints the 54-bit R of the ALU and of a fabric reference (`*`) after each
// phase, one line per phase, so the emulator can be compared against the
// RTL sim bit for bit.
//   P1: rst, then one tap 4096*8192            -> R = 2^25
//   P2: rst, then (-4096)*8192, 4096*(-8192)   -> R = -2^26 (two negatives in a row)
//   P3: NO rst, then one tap 4096*8192         -> R = -2^25
//   P4: rst, then one tap 1*1                  -> R = 1
//   P5: rst, then 4096*8192 twice              -> R = 2^26 (two positives in a row)
//   P6: rst, then (-4096)*8192 twice, one idle between -> R = -2^26
//   P7: rst, then one tap (-4096)*8192         -> R = -2^25 (single negative)
module top(input clk_25mhz, output ftdi_rxd);
  reg [7:0] reset_count = 0;
  wire reset = reset_count != 8'hff;
  always @(posedge clk_25mhz) if (reset) reset_count <= reset_count + 1;

  reg [3:0] phase = 0, step = 0;
  reg mac_en = 0, mac_rst = 0;
  reg [17:0] a = 0, b = 0;
  wire [53:0] r_alu;
  dsp_mac mac(clk_25mhz, mac_en, mac_rst, a, b, r_alu);

  // Fabric reference on the same schedule.
  reg signed [53:0] r_ref = 0;
  wire signed [35:0] p_ref = $signed(a) * $signed(b);
  always @(posedge clk_25mhz)
    if (mac_rst) r_ref <= 0;
    else if (mac_en) r_ref <= r_ref + p_ref;

  // 31-byte line: 14 hex of R_ALU, '-', 14 hex of R_ref, CR, LF.
  reg [7:0] msg[0:30];
  integer i;
  reg [5:0] byte_idx = 0;
  reg [3:0] bit_idx = 0;   // 0 idle, 1 start, 2..9 data, 10 stop
  reg [8:0] bitcnt = 0;
  reg tx = 1;
  assign ftdi_rxd = tx;
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h41 + v - 10;
  endfunction

  localparam S_RUN = 0, S_SEND = 1, S_DONE = 2;
  reg [1:0] state = S_RUN;
  reg sending = 0;

  always @(posedge clk_25mhz) begin
    if (reset) begin
      state <= S_RUN; phase <= 0; step <= 0;
      mac_en <= 0; mac_rst <= 0; a <= 0; b <= 0;
      byte_idx <= 0; bit_idx <= 0; bitcnt <= 0; tx <= 1; sending <= 0;
    end else case (state)
      S_RUN: begin
        mac_rst <= 0; mac_en <= 0;
        case (step)
          0: mac_rst <= (phase != 2);       // P3 skips the reset
          1, 2, 3: begin                    // tap slots 1..3 (3 is used by P6 only)
            case (phase)
              0: mac_en <= (step == 1);
              1: mac_en <= (step != 3);
              2: mac_en <= (step == 1);
              3: mac_en <= (step == 1);
              4: mac_en <= (step != 3);
              5: mac_en <= (step == 1 || step == 3);
              6: mac_en <= (step == 1);
            endcase
            case (phase)
              0, 2, 4: begin a <= 18'h01000; b <= 18'h02000; end
              1: begin
                if (step == 1) begin a <= 18'h3F000; b <= 18'h02000; end  // -4096*8192
                else         begin a <= 18'h01000; b <= 18'h3E000; end  // 4096*-8192
              end
              3: begin a <= 18'h00001; b <= 18'h00001; end
              5, 6: begin a <= 18'h3F000; b <= 18'h02000; end  // -4096*8192
            endcase
          end
          4: begin end  // settle: the last tap accumulates at this cycle's end
          default: begin
            // 54 bits padded to 56 so the hex digits align to bit 0.
            for (i = 0; i < 14; i = i + 1) begin
              msg[i]    <= hex({2'b0, r_alu} >> (52 - 4*i));
              msg[i+15] <= hex({2'b0, r_ref} >> (52 - 4*i));
            end
            msg[14] <= 8'h2D;  // '-'
            msg[29] <= 8'h0D;
            msg[30] <= 8'h0A;
            byte_idx <= 0; bit_idx <= 1; bitcnt <= 0; state <= S_SEND;
          end
        endcase
        step <= step + 1;
      end
      S_SEND: begin
        if (bitcnt == 216) begin
          bitcnt <= 0;
          if (bit_idx == 10) begin
            bit_idx <= 1;
            if (byte_idx == 30) begin
              bit_idx <= 0;
              if (phase == 6) state <= S_DONE;
              else begin phase <= phase + 1; step <= 0; state <= S_RUN; end
            end else byte_idx <= byte_idx + 1;
          end else bit_idx <= bit_idx + 1;
        end else bitcnt <= bitcnt + 1;
        case (bit_idx)
          0: tx <= 1;
          1: tx <= 0;                       // start
          10: tx <= 1;                      // stop
          default: tx <= msg[byte_idx][bit_idx - 2];
        endcase
      end
      S_DONE: tx <= 1;
    endcase
  end
endmodule
