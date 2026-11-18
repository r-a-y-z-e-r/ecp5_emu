// Parallel SHA-256 chains, with all final values folded by the same hardware.
module top #(parameter CORES=10, parameter STEP_EVERY=1024, parameter STEPS=3072)(
 input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output [7:0] led);
 localparam [255:0] IV=256'h6a09e667bb67ae853c6ef372a54ff53a510e527f9b05688c1f83d9ab5be0cd19;
 localparam [255:0] ABC_HASH=256'hba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad;
 localparam [255:0] LONG_HASH=256'h248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1;
 localparam [511:0] ABC_BLOCK={32'h61626380,448'b0,32'd24};
 localparam [511:0] LONG_BLOCK=512'h6162636462636465636465666465666765666768666768696768696a68696a6b696a6b6c6a6b6c6d6b6c6d6e6c6d6e6f6d6e6f706e6f70718000000000000000;
 // The 56-byte vector needs two padded blocks; the first output is the IV
 // for the second block. Individual chain links always begin with the IV.
 localparam [511:0] LONG_TAIL={480'b0,32'd448};
 localparam [63:0] FINAL_BITS=CORES*256;
 localparam [4:0] FULL_BLOCKS=CORES/2;
 localparam LANE_BITS=CORES<2 ? 1:$clog2(CORES);
 initial begin
   if(CORES<1 || CORES>16) $error("CORES must be 1..16");
   if(STEP_EVERY<1 || STEPS<1 || STEPS%STEP_EVERY!=0) $error("STEPS must be a positive multiple of STEP_EVERY");
 end
 wire clk=clk_25mhz;
 reg [5:0] reset_count=0;
 wire reset=!(&reset_count);
 always @(posedge clk) if(reset) reset_count<=reset_count+1;
 reg send=0,take=0;
 reg [7:0] data=0;
 wire busy,available;
 wire [7:0] key;
 wire [6:0] pressed;
 gallery_io io(clk,reset,ftdi_txd,ftdi_rxd,btn,pressed,send,data,busy,key,available,take);
 localparam PRINT=0,DRAIN=1,LAUNCH=2,WAIT_HASH=3,NEXT_LANE=4,NEXT_PASS=5,HALT=6,BOOT=7;
 localparam RESTART_LINE=0,ABC_LINE=1,LONG_LINE=2,STEP_LINE=3,CHAIN_LINE=4;
 localparam KAT_ABC=0,KAT_FIRST=1,KAT_TAIL=2,SEED=3,LINK=4,FINAL=5;
 reg [2:0] state=BOOT,after_print=LAUNCH,line_kind=RESTART_LINE,mode=KAT_ABC;
 reg restart_pending=0,kat_failed=0;
 reg [31:0] pass_number=0,step_index=0;
 reg [4:0] aggregate_block=0;
 reg [LANE_BITS-1:0] report_core=0;
 reg [63:0] cycles=0,report_cycles=0;
 reg [255:0] report_hash=0;
 reg report_ok=0;
 reg [7:0] pos=0;
 wire [CORES-1:0] done,kat_good;
 wire [255:0] digest[0:CORES-1];
 wire restart_request=(available && !take && key=="r") || pressed[1];
 wire core_reset=reset || restart_pending || restart_request || state==BOOT;
 wire start=state==LAUNCH && !core_reset;
 // All final digests stay in their core output registers. Core 0 consumes
 // its own value in block 0 before overwriting it; later blocks read only
 // the other, paused cores. Thus no duplicate array or EBR is required.
 reg [511:0] final_block;
 always @* begin
   if(aggregate_block<FULL_BLOCKS)
     final_block={digest[2*aggregate_block],digest[2*aggregate_block+1]};
   else if(CORES%2)
     final_block={digest[CORES-1],8'h80,184'b0,FINAL_BITS};
   else final_block={8'h80,440'b0,FINAL_BITS};
 end
 genvar k;
 generate for(k=0;k<CORES;k=k+1) begin: cores
   localparam [31:0] INDEX=k;
   // 22 ASCII bytes, then 32-bit core index and pass, both big endian:
   // 30 bytes, 0x80, 25 zero bytes, then the 64-bit bit length 240.
   wire [511:0] seed_block={176'h506172616c6c656c20636861696e2073656564202f20,
                            INDEX,pass_number,8'h80,200'b0,64'd240};
   // h(i+1) hashes the raw 32 bytes of h(i), never its hex representation.
   wire [511:0] link_block={digest[k],8'h80,184'b0,64'd256};
   wire [511:0] message_block=mode==KAT_ABC ? ABC_BLOCK : mode==KAT_FIRST ? LONG_BLOCK :
     mode==KAT_TAIL ? LONG_TAIL : mode==SEED ? seed_block : mode==LINK ? link_block:final_block;
   wire [255:0] chaining=(mode==KAT_TAIL || (mode==FINAL && aggregate_block!=0)) ? digest[k]:IV;
   wire enable_core=start && (mode!=FINAL || k==0);
   sha256_core core(clk,core_reset,enable_core,message_block,chaining,digest[k],done[k]);
   assign kat_good[k]=digest[k]==(mode==KAT_ABC ? ABC_HASH:LONG_HASH);
 end endgenerate
 function [7:0] hex;
   input [3:0] n; begin hex=n<10 ? "0"+{4'b0,n} : "A"+{4'b0,n}-8'd10; end
 endfunction
 function [7:0] line_char;
   input [7:0] p;
   reg [7:0] core_byte;
   reg [31:0] step_total;
   begin
     core_byte=report_core;step_total=STEPS;line_char=10;
     case(line_kind)
       RESTART_LINE: case(p)
         0:line_char="R";1:line_char="E";2:line_char="S";3:line_char="T";
         4:line_char="A";5:line_char="R";6:line_char="T";7:line_char=13;
       endcase
       ABC_LINE,LONG_LINE: begin
         if(p>=8 && p<72) line_char=hex(report_hash[255-4*(p-8)-:4]);
         else case(p)
           0:line_char="K";1:line_char="A";2:line_char="T";3:line_char=" ";
           4:line_char=line_kind==ABC_LINE ? "a":"4";
           5:line_char=line_kind==ABC_LINE ? "b":"4";
           6:line_char=line_kind==ABC_LINE ? "c":"8";
           7,72:line_char=" ";73:line_char=report_ok ? "O":"F";
           74:line_char=report_ok ? "K":"A";
           75:line_char=report_ok ? 13:"I";76:line_char=report_ok ? 10:"L";
           77:line_char=13;
         endcase
       end
       STEP_LINE: begin
         if(p>=5 && p<13) line_char=hex(step_index[31-4*(p-5)-:4]);
         else if(p>=17 && p<81) line_char=hex(report_hash[255-4*(p-17)-:4]);
         else case(p)
           0:line_char="S";1:line_char="T";2:line_char="E";3:line_char="P";
           4,13,16:line_char=" ";14:line_char=hex(core_byte[7:4]);
           15:line_char=hex(core_byte[3:0]);81:line_char=13;
         endcase
       end
       CHAIN_LINE: begin
         if(p>=6 && p<14) line_char=hex(pass_number[31-4*(p-6)-:4]);
         else if(p>=21 && p<29) line_char=hex(step_total[31-4*(p-21)-:4]);
         else if(p>=37 && p<101) line_char=hex(report_hash[255-4*(p-37)-:4]);
         else if(p>=109 && p<125) line_char=hex(report_cycles[63-4*(p-109)-:4]);
         else case(p)
           0:line_char="C";1:line_char="H";2:line_char="A";3:line_char="I";4:line_char="N";
           5,14,20,29,36,101,108:line_char=" ";15:line_char="S";16:line_char="T";
           17:line_char="E";18:line_char="P";19:line_char="S";30:line_char="D";
           31:line_char="I";32:line_char="G";33:line_char="E";34:line_char="S";35:line_char="T";
           102:line_char="C";103:line_char="Y";104:line_char="C";105:line_char="L";
           106:line_char="E";107:line_char="S";125:line_char=13;
         endcase
       end
     endcase
   end
 endfunction
 wire [7:0] last_pos=line_kind==RESTART_LINE ? 8 : line_kind==STEP_LINE ? 82 :
                         line_kind==CHAIN_LINE ? 126 : report_ok ? 76 : 78;
 // Low seven LEDs show the current pass modulo 128; LED7 is step bit 10.
 assign led={step_index[10],pass_number[6:0]};
 always @(posedge clk) begin
   send<=0;take<=0;cycles<=cycles+1;
   if(available && !take) take<=1;
   if(restart_request) restart_pending<=1;
   // Finish the current UART byte before the marker. Keep the RX FIFO.
   if(reset) begin state<=BOOT;restart_pending<=0;cycles<=0;end
   else if(state==BOOT || restart_pending || restart_request) begin
     if(!busy && !send) begin
       mode<=KAT_ABC;kat_failed<=0;pass_number<=0;step_index<=0;aggregate_block<=0;
       cycles<=0;report_cycles<=0;report_hash<=0;report_core<=0;report_ok<=0;
       // Configuration enters the KATs silently. Only a requested restart
       // emits the marker, after the current UART byte has finished.
       pos<=0;line_kind<=RESTART_LINE;after_print<=LAUNCH;
       state<=(restart_pending || restart_request) ? PRINT:LAUNCH;
       restart_pending<=0;
     end
   end else case(state)
     PRINT: if(!busy && !send) begin
       data<=line_char(pos);send<=1;
       if(pos==last_pos) begin pos<=0;state<=DRAIN;end else pos<=pos+1;
     end
     DRAIN: if(!busy && !send) state<=after_print;
     LAUNCH: state<=WAIT_HASH;
     WAIT_HASH: if(mode==FINAL ? done[0]:(&done)) begin
       if(mode==KAT_FIRST) begin mode<=KAT_TAIL;state<=LAUNCH;end
       else if(mode==KAT_ABC || mode==KAT_TAIL) begin
         report_hash<=digest[0];report_ok<=&kat_good;
         kat_failed<=kat_failed || !(&kat_good);
         line_kind<=mode==KAT_ABC ? ABC_LINE:LONG_LINE;pos<=0;state<=PRINT;
         after_print<=(mode==KAT_TAIL && (kat_failed || !(&kat_good))) ? HALT:LAUNCH;
         mode<=mode==KAT_ABC ? KAT_FIRST:SEED;
       end else if(mode==SEED) begin step_index<=1;mode<=LINK;state<=LAUNCH;end
       else if(mode==LINK) begin
         if(step_index%STEP_EVERY==0) begin
           report_core<=0;report_hash<=digest[0];line_kind<=STEP_LINE;
           pos<=0;state<=PRINT;after_print<=NEXT_LANE;
         end else begin step_index<=step_index+1;state<=LAUNCH;end
       end else begin
         if(aggregate_block==FULL_BLOCKS) begin
           report_hash<=digest[0];report_cycles<=cycles;
           line_kind<=CHAIN_LINE;pos<=0;state<=PRINT;after_print<=NEXT_PASS;
         end else begin aggregate_block<=aggregate_block+1;state<=LAUNCH;end
       end
     end
     NEXT_LANE: begin
       if(report_core==CORES-1) begin
         if(step_index==STEPS) begin mode<=FINAL;aggregate_block<=0;state<=LAUNCH;end
         else begin step_index<=step_index+1;state<=LAUNCH;end
       end else begin
         report_core<=report_core+1;report_hash<=digest[report_core+1'b1];
         pos<=0;state<=PRINT;after_print<=NEXT_LANE;
       end
     end
     NEXT_PASS: begin pass_number<=pass_number+1;step_index<=0;mode<=SEED;state<=LAUNCH;end
     HALT: state<=HALT;
     default: state<=BOOT;
   endcase
 end
endmodule
