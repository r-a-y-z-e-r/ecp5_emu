// Full-window SSD1331 streamer; common init follows oled_bricks.
module synth_oled(input clk, input reset, input enable,
 input [1535:0] history, input [4:0] step, input [6:0] note, input paused,
 output reg oled_clk=0, output reg oled_mosi=0, output reg oled_dc=0,
 output reg oled_csn=1, output reg oled_resn=0,
 output idle, output reg valid=0, input take, output reg [15:0] frame=0,
 output reg [31:0] frame_crc=0);
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
 reg [4:0] snap_step=0;
 reg [6:0] snap_note=0;
 reg snap_paused=0;
 reg [6:0] x=0;
 reg [5:0] y=0;
 wire [15:0] pixel;
 reg [5:0] trace_here=38, trace_low=38, trace_high=38;
 reg [5:0] ahead_row=38, ahead_low=38, ahead_high=38;
 wire [5:0] row_zero, row_next;
 wire trace_ready;
 wire capture=(state==9) && enable && (frame==0 || period==624999);
 scope_trace_cache trace_cache(clk,reset,capture,history,
   x==95 ? 7'd0 : x+7'd1,row_zero,row_next,trace_ready);
 scope_render render(clk,x,y,trace_here,trace_low,trace_high,
   snap_step,snap_note,snap_paused,pixel);
 // Look ahead by one column throughout the current pixel's two SPI bytes.
 // The RF read and min/max comparison each have their own register boundary.
 always @(posedge clk) begin
   ahead_row<=row_next;
   ahead_low<=trace_here<ahead_row ? trace_here : ahead_row;
   ahead_high<=trace_here>ahead_row ? trace_here : ahead_row;
 end
 reg [3:0] state=0;
 reg [14:0] reset_count=0;
 reg [19:0] period=0;
 reg [5:0] init_index=0;
 reg [2:0] window_index=0;
 reg low_byte=0;
 reg active=0, done=0, go=0, data_mode=0;
 reg [7:0] send_byte=0, shift=0;
 reg [1:0] phase=0;
 reg [2:0] bit_index=0;
 reg [31:0] crc=32'hffffffff;
 reg [7:0] crc_bits=0;
 reg [3:0] crc_left=0;
 // Exposed to the top only through this combinational status port below.
 assign idle=(state==0 || state==9) && !valid;
 always @(posedge clk) begin
   done<=0;
   if(reset) begin active<=0; oled_clk<=0; oled_csn<=1; oled_mosi<=0; oled_dc<=0; end
   else if(!active) begin
     if(go) begin active<=1; shift<=send_byte; phase<=0; bit_index<=7;
       oled_csn<=0; oled_dc<=data_mode; oled_mosi<=send_byte[7]; end
   end else begin
     phase<=phase+1;
     if(phase==1) oled_clk<=1;
     if(phase==3) begin
       oled_clk<=0;
       if(bit_index==0) begin active<=0; done<=1; oled_csn<=1; end
       else begin bit_index<=bit_index-1; shift<={shift[6:0],1'b0}; oled_mosi<=shift[6]; end
     end
   end
 end
 function [7:0] window_byte;
   input [2:0] n;
   begin case(n) 0:window_byte=8'h15; 1:window_byte=0; 2:window_byte=95;
     3:window_byte=8'h75; 4:window_byte=0; default:window_byte=63; endcase end
 endfunction
 always @(posedge clk) begin
   go<=0;
   if(period!=624999) period<=period+1;
   if(crc_left!=0) begin
     crc<=(crc>>1)^((crc[0]^crc_bits[0])?32'hedb88320:0);
     crc_bits<=crc_bits>>1; crc_left<=crc_left-1;
   end
   if(reset) begin
     state<=0; oled_resn<=0; reset_count<=0; period<=0; init_index<=0;
     window_index<=0; low_byte<=0; x<=0; y<=0; frame<=0; frame_crc<=0;
     valid<=0; crc<=32'hffffffff; crc_bits<=0; crc_left<=0;
     trace_here<=38; trace_low<=38; trace_high<=38; snap_step<=0; snap_note<=0; snap_paused<=0;
   end else case(state)
     0: if(enable) begin
       if(reset_count==24999) begin oled_resn<=1; state<=1; end
       else reset_count<=reset_count+1;
     end
     1: begin send_byte<=init_byte(init_index); data_mode<=0; go<=1; state<=2; end
     2: if(done) begin
       if(init_index==36) state<=9;
       else begin init_index<=init_index+1; state<=1; end
     end
     3: begin send_byte<=window_byte(window_index); data_mode<=0; go<=1; state<=4; end
     4: if(done) begin
       if(window_index==5) begin
         x<=0; y<=0; low_byte<=0; crc<=32'hffffffff; state<=5;
         trace_here<=row_zero; trace_low<=row_zero; trace_high<=row_zero;
       end
       else begin window_index<=window_index+1; state<=3; end
     end
     5: begin send_byte<=low_byte?pixel[7:0]:pixel[15:8]; data_mode<=1; go<=1;
       crc_bits<=low_byte?pixel[7:0]:pixel[15:8]; crc_left<=8; state<=6; end
     6: if(done) begin
       low_byte<=!low_byte; state<=5;
       if(low_byte) begin
         if(x==95) begin x<=0;
           trace_here<=row_zero; trace_low<=row_zero; trace_high<=row_zero;
           if(y==63) begin frame_crc<=~crc; valid<=1; state<=7; end
           else y<=y+1;
         end else begin
           x<=x+1; trace_here<=ahead_row; trace_low<=ahead_low; trace_high<=ahead_high;
         end
       end
     end
     7: if(take) begin valid<=0; frame<=frame+1; state<=9; end
     9: if(enable && (frame==0 || period==624999)) begin
       snap_step<=step; snap_note<=note; snap_paused<=paused;
       period<=0; window_index<=0; state<=3;
     end
     default: state<=0;
   endcase
 end
`ifdef VERILATOR
 // Precomputation must fit in the existing window-command schedule: no
 // wait states may silently move snapshots or UART ordering.
 always @(posedge clk)
   if(!reset && state==5 && !trace_ready) $fatal(1,"trace cache missed first pixel");
`endif
endmodule
