// Read-only MCP7940N random-read controller, 100 kHz at 25 MHz.
// There is no data-write state or arbitrary transmit-byte port.
module rtc_reader(
  input clk, input start, input [7:0] pointer, input [6:0] count,
  output reg ready = 0, output reg valid = 0,
  output reg [7:0] data = 0, output reg error = 0,
  inout rtc_scl, inout rtc_sda
);
  localparam IDLE=0, START=1, SEND_LOW=2, SEND_HIGH=3,
    ACK_LOW=4, ACK_HIGH=5, ACK_END=6, RESTART_RELEASE=7,
    RESTART_HIGH=8, RESTART=9, READ_LOW=10, READ_HIGH=11,
    READ_END=12, HOST_ACK_LOW=13, HOST_ACK_HIGH=14,
    HOST_ACK_END=15, STOP_LOW=16, STOP_HIGH=17, STOP=18;
  reg [4:0] state = IDLE;
  reg scl_low = 0, sda_low = 0;
  assign rtc_scl = scl_low ? 1'b0 : 1'bz;
  assign rtc_sda = sda_low ? 1'b0 : 1'bz;
  reg [6:0] divider = 0, remaining = 0;
  reg [7:0] tx = 8'hde, rx = 0;
  reg [2:0] bit_index = 7;
  reg [1:0] stage = 0;
  always @(posedge clk) begin
    ready <= 0; valid <= 0;
    if (divider == 124) begin
      divider <= 0;
      case (state)
        IDLE: if (start) begin
          scl_low <= 0; sda_low <= 0; remaining <= count;
          error <= 0; stage <= 0; tx <= 8'hde; bit_index <= 7; state <= START;
        end
        START: begin sda_low <= 1; state <= SEND_LOW; end
        SEND_LOW: begin scl_low <= 1; sda_low <= !tx[7]; state <= SEND_HIGH; end
        SEND_HIGH: begin
          scl_low <= 0;
          tx <= {tx[6:0],1'b1};
          if (bit_index == 0) state <= ACK_LOW;
          else begin bit_index <= bit_index - 1; state <= SEND_LOW; end
        end
        ACK_LOW: begin scl_low <= 1; sda_low <= 0; state <= ACK_HIGH; end
        ACK_HIGH: begin scl_low <= 0; state <= ACK_END; end
        ACK_END: begin
          scl_low <= 1; bit_index <= 7;
          if (rtc_sda) begin error <= 1; state <= STOP_LOW; end
          else case (stage)
            0: begin tx <= pointer; stage <= 1; state <= SEND_LOW; end
            1: begin stage <= 2; state <= RESTART_RELEASE; end
            2: begin rx <= 0; state <= READ_LOW; end
            default: state <= STOP_LOW;
          endcase
        end
        RESTART_RELEASE: begin sda_low <= 0; state <= RESTART_HIGH; end
        RESTART_HIGH: begin scl_low <= 0; state <= RESTART; end
        RESTART: begin sda_low <= 1; tx <= 8'hdf; state <= SEND_LOW; end
        READ_LOW: begin scl_low <= 1; sda_low <= 0; state <= READ_HIGH; end
        READ_HIGH: begin scl_low <= 0; state <= READ_END; end
        READ_END: begin
          rx <= {rx[6:0],rtc_sda}; scl_low <= 1;
          if (bit_index == 0) state <= HOST_ACK_LOW;
          else begin bit_index <= bit_index - 1; state <= READ_HIGH; end
        end
        HOST_ACK_LOW: begin
          data <= rx; valid <= 1; sda_low <= remaining != 1; state <= HOST_ACK_HIGH;
        end
        HOST_ACK_HIGH: begin scl_low <= 0; state <= HOST_ACK_END; end
        HOST_ACK_END: begin
          scl_low <= 1; sda_low <= 0; remaining <= remaining - 1;
          bit_index <= 7; rx <= 0;
          state <= remaining == 1 ? STOP_LOW : READ_LOW;
        end
        STOP_LOW: begin scl_low <= 1; sda_low <= 1; state <= STOP_HIGH; end
        STOP_HIGH: begin scl_low <= 0; state <= STOP; end
        STOP: begin sda_low <= 0; ready <= 1; state <= IDLE; end
        default: state <= IDLE;
      endcase
    end else divider <= divider + 1;
  end
endmodule
