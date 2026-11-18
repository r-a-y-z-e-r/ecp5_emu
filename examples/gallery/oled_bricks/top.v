// Orchard: a streaming brick-breaker, one 25 MHz domain, no CPU or framebuffer.
module top(input clk_25mhz, input ftdi_txd, output ftdi_rxd,
 input [6:0] btn, input [3:0] sw, output [7:0] led,
 output reg oled_clk=0, output reg oled_mosi=0, output reg oled_dc=0,
 output reg oled_csn=1, output reg oled_resn=0);
  function [7:0] init_byte;
    input [5:0] index;
    begin
      case (index)
        0: init_byte = 8'hae;
        1: init_byte = 8'ha0;
        2: init_byte = 8'h72;
        3: init_byte = 8'ha1;
        4: init_byte = 8'h00;
        5: init_byte = 8'ha2;
        6: init_byte = 8'h00;
        7: init_byte = 8'ha4;
        8: init_byte = 8'ha8;
        9: init_byte = 8'h3f;
        10: init_byte = 8'had;
        11: init_byte = 8'h8e;
        12: init_byte = 8'hb0;
        13: init_byte = 8'h0b;
        14: init_byte = 8'hb1;
        15: init_byte = 8'h31;
        16: init_byte = 8'hb3;
        17: init_byte = 8'hf0;
        18: init_byte = 8'h8a;
        19: init_byte = 8'h64;
        20: init_byte = 8'h8b;
        21: init_byte = 8'h78;
        22: init_byte = 8'h8c;
        23: init_byte = 8'h64;
        24: init_byte = 8'hbb;
        25: init_byte = 8'h3a;
        26: init_byte = 8'hbe;
        27: init_byte = 8'h3e;
        28: init_byte = 8'h87;
        29: init_byte = 8'h06;
        30: init_byte = 8'h81;
        31: init_byte = 8'h91;
        32: init_byte = 8'h82;
        33: init_byte = 8'h50;
        34: init_byte = 8'h83;
        35: init_byte = 8'h7d;
        36: init_byte = 8'haf;
        default: init_byte = 8'haf;
      endcase
    end
  endfunction
 // This implementation advances the reflected CRC one bit per clock, in
 // parallel with SPI's MSB-first shift (CRC consumes the byte LSB first).
 reg [31:0] crc=32'hffffffff, frame_crc=0;
 reg [7:0] crc_bits=0;
 reg [3:0] crc_left=0;
 reg spi_active=0, spi_done=0, spi_go=0;
 reg [7:0] spi_byte=0, shift=0;
 reg spi_data=0;
 reg [1:0] phase=0;
 reg [2:0] bit_index=0;
 wire restart;
 always @(posedge clk_25mhz) begin
   spi_done<=0;
   if(restart) begin
     spi_active<=0; oled_clk<=0; oled_csn<=1; oled_mosi<=0; oled_dc<=0;
   end else if(!spi_active) begin
     if(spi_go) begin
       spi_active<=1; shift<=spi_byte; phase<=0; bit_index<=7;
       oled_csn<=0; oled_dc<=spi_data; oled_mosi<=spi_byte[7];
     end
   end else begin
     phase<=phase+1;
     if(phase==1) oled_clk<=1;
     if(phase==3) begin
       oled_clk<=0;
       if(bit_index==0) begin spi_active<=0; spi_done<=1; oled_csn<=1; end
       else begin bit_index<=bit_index-1; shift<={shift[6:0],1'b0}; oled_mosi<=shift[6]; end
     end
   end
 end
 wire [6:0] pressed, held;
 wire [7:0] key;
 wire available, tx_busy;
 wire send;
 wire [7:0] tx_data;
 gallery_io io(.clk(clk_25mhz),.reset(1'b0),.rx_pin(ftdi_txd),.tx_pin(ftdi_rxd),
   .btn(btn),.pressed(pressed),.held(held),.send(send && !restart),.data(tx_data),
   .busy(tx_busy),.key(key),.available(available),.take(available));
 assign restart=(available && key=="r") || pressed[2];
 reg fire_pending=0;
 reg [1:0] move_seen=0, move_step=0;
 // Switch debounce and an event FIFO. Initial settling is silent. In attract
 // mode switches do not enter any game, renderer, timing or UART decision.
 reg [3:0] sw_sync0=0, sw_sync1=0, candidate=0, stable=0;
 reg [17:0] sw_count=0;
 reg sw_ready=0;
 reg [3:0] sw_fifo[0:15];
 reg [4:0] sw_wr=0, sw_rd=0;
 reg [3:0] options=0, sw_report=0;
 wire human;
 reg pop_sw=0;
 integer s;
 initial for(s=0;s<16;s=s+1) sw_fifo[s]=0;
 always @(posedge clk_25mhz) begin
   sw_sync0<=sw; sw_sync1<=sw_sync0;
   if(sw_sync1!=candidate) begin candidate<=sw_sync1; sw_count<=0; end
   else if(sw_count<249999) sw_count<=sw_count+1;
   else if(!sw_ready || stable!=candidate) begin
     stable<=candidate; sw_ready<=1;
     if(sw_ready && human && !restart) begin sw_fifo[sw_wr[3:0]]<=candidate; sw_wr<=sw_wr+1; end
   end
   if(!human || restart) begin sw_wr<=0; sw_rd<=0; end
   else if(pop_sw) sw_rd<=sw_rd+1;
 end
 reg game_reset=1, game_step=0, player_start=0, launch=0;
 wire game_done, wide, theme, waiting;
 wire [71:0] bricks;
 wire [6:0] remaining, paddle;
 wire [11:0] score, bx, by;
 wire [1:0] lives, level;
 wire [3:0] events, hit_col;
 wire [2:0] hit_row;
 bricks_game game(.clk(clk_25mhz),.reset(game_reset),.step(game_step),
   .player_start(player_start),.launch(launch),.left(move_step[0]),.right(move_step[1]),
   .options(options),.done(game_done),.human(human),.bricks(bricks),.remaining(remaining),
   .score(score),.lives(lives),.level(level),.paddle(paddle),.bx(bx),.by(by),
   .events(events),.hit_col(hit_col),.hit_row(hit_row),.waiting(waiting),.wide(wide),.theme(theme));
 assign led={human,waiting,level,1'b0,lives!=0,lives>=2,lives>=3};
 reg [6:0] x=0;
 reg [5:0] y=0;
 reg low_byte=0;
 wire [15:0] pixel;
 bricks_render render(.x(x),.y(y),.bricks(bricks),.score(score),.lives(lives),
   .level(level),.paddle(paddle),.bx(bx),.by(by),.wide(wide),.theme(theme),
   .human(human),.waiting(waiting),.pixel(pixel));
 reg [15:0] frame=0;
 reg print_start=0;
 reg [3:0] kind=0;
 wire print_done, print_busy;
 bricks_text text(.clk(clk_25mhz),.abort_line(restart),.start(print_start),.kind(kind),
   .frame(frame),.crc(frame_crc),.score(score),.lives(lives),.level(level),
   .remaining(remaining),.col(hit_col),.row(hit_row),.switches(sw_report),
   .send(send),.data(tx_data),.tx_busy(tx_busy),.busy(print_busy),.done(print_done));
 localparam RESET=0, INIT=1, INIT_WAIT=2, WINDOW=3, WINDOW_WAIT=4,
   PIXEL=5, PIXEL_WAIT=6, PRINT_WAIT=7, BOUNDARY=8, GAME_WAIT=9,
   EVENT=10, RESTART_WAIT=11;
 reg [3:0] state=RESET, after_print=BOUNDARY;
 reg [14:0] reset_count=0;
 reg [5:0] init_index=0;
 reg [2:0] window_index=0;
 reg [3:0] pending_events=0;
 reg [7:0] window_byte;
 always @* case(window_index)
   0: window_byte=8'h15;
   1: window_byte=0;
   2: window_byte=95;
   3: window_byte=8'h75;
   4: window_byte=0;
   default: window_byte=63;
 endcase
 always @(posedge clk_25mhz) begin
   spi_go<=0; print_start<=0; game_reset<=0; game_step<=0;
   player_start<=0; launch<=0; pop_sw<=0;
   if(pressed[1]) fire_pending<=1;
   // Remember a filtered click even when it ends during an SPI fill. The
   // snapshot stays valid until game_step is consumed on the following clock.
   if(human && lives!=0) move_seen<=move_seen | held[6:5];
   else move_seen<=0;
   // CRC work finishes in eight clocks, comfortably before the byte ends.
   if(crc_left!=0) begin
     crc<=(crc>>1) ^ ((crc[0]^crc_bits[0]) ? 32'hedb88320 : 32'b0);
     crc_bits<=crc_bits>>1; crc_left<=crc_left-1;
   end
   if(restart) begin
     state<=RESTART_WAIT; game_reset<=1; oled_resn<=0;
     reset_count<=0; init_index<=0; window_index<=0; frame<=0;
     x<=0; y<=0; low_byte<=0; crc<=32'hffffffff; crc_left<=0; crc_bits<=0;
     frame_crc<=0; fire_pending<=0; options<=0; sw_report<=0; pending_events<=0;
     move_seen<=0; move_step<=0;
   end else case(state)
     RESTART_WAIT: if(!tx_busy && !print_busy) begin
       kind<=7; print_start<=1; after_print<=RESET; state<=PRINT_WAIT;
     end
     RESET: begin
       oled_resn<=0;
       if(reset_count==24999) begin oled_resn<=1; state<=INIT; end
       else reset_count<=reset_count+1;
     end
     INIT: begin spi_byte<=init_byte(init_index); spi_data<=0; spi_go<=1; state<=INIT_WAIT; end
     INIT_WAIT: if(spi_done) begin
       if(init_index==36) state<=WINDOW;
       else begin init_index<=init_index+1; state<=INIT; end
     end
     WINDOW: begin spi_byte<=window_byte; spi_data<=0; spi_go<=1; state<=WINDOW_WAIT; end
     WINDOW_WAIT: if(spi_done) begin
       if(window_index==5) begin
         x<=0; y<=0; low_byte<=0; crc<=32'hffffffff; state<=PIXEL;
       end else begin window_index<=window_index+1; state<=WINDOW; end
     end
     PIXEL: begin
       spi_byte<=low_byte ? pixel[7:0] : pixel[15:8]; spi_data<=1; spi_go<=1;
       crc_bits<=low_byte ? pixel[7:0] : pixel[15:8]; crc_left<=8;
       state<=PIXEL_WAIT;
     end
     PIXEL_WAIT: if(spi_done) begin
       low_byte<=!low_byte; state<=PIXEL;
       if(low_byte) begin
         if(x==95) begin
           x<=0;
           if(y==63) begin
             frame_crc<=~crc; kind<=0; print_start<=1;
             after_print<=BOUNDARY; state<=PRINT_WAIT;
           end else y<=y+1;
         end else x<=x+1;
       end
     end
     PRINT_WAIT: if(print_done) state<=after_print;
     BOUNDARY: begin
       // All state changes, inputs and event lines occur between full fills.
       if(human && sw_rd!=sw_wr) begin
         options<=sw_fifo[sw_rd[3:0]]; sw_report<=sw_fifo[sw_rd[3:0]];
         pop_sw<=1; kind<=6; print_start<=1; after_print<=BOUNDARY; state<=PRINT_WAIT;
       end else begin
         frame<=frame+1;
         if(fire_pending && (!human || lives==0)) begin
           options<=stable; player_start<=1; fire_pending<=0;
           move_seen<=0; move_step<=0;
           kind<=5; print_start<=1; after_print<=EVENT; state<=PRINT_WAIT;
         end else begin
           move_step<=move_seen | held[6:5]; move_seen<=0;
           game_step<=1; launch<=fire_pending; fire_pending<=0; state<=GAME_WAIT;
         end
       end
     end
     GAME_WAIT: if(game_done) begin pending_events<=events; state<=EVENT; end
     EVENT: begin
       after_print<=EVENT;
       if(pending_events[0]) begin kind<=1; pending_events[0]<=0; print_start<=1; state<=PRINT_WAIT; end
       else if(pending_events[1]) begin kind<=2; pending_events[1]<=0; print_start<=1; state<=PRINT_WAIT; end
       else if(pending_events[2]) begin kind<=3; pending_events[2]<=0; print_start<=1; state<=PRINT_WAIT; end
       else if(pending_events[3]) begin kind<=4; pending_events[3]<=0; print_start<=1; state<=PRINT_WAIT; end
       else begin window_index<=0; state<=WINDOW; end
     end
     default: state<=RESET;
   endcase
 end
endmodule
