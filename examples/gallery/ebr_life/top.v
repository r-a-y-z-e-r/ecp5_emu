// Space for time: two 2048-cell worlds in EBR, rather than 2048 parallel
// neighbour counters. Fetch three rows, rotate them one bit each clock,
// and write one completed row to the other world. The source never changes
// during a sweep or a UART frame. All coordinates wrap by truncation.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output [7:0] led);
 wire clk=clk_25mhz;
 reg [5:0] reset_count=0;
 wire reset= !(&reset_count);
 always @(posedge clk) if (reset) reset_count<=reset_count+1;
 reg send=0, take=0;
 reg [7:0] data=0;
 wire busy, available;
 wire [7:0] key;
 wire [6:0] pressed;
 gallery_io io(clk,reset,ftdi_txd,ftdi_rxd,btn,pressed,send,data,busy,key,available,take);

 (* ram_style="block" *) reg [63:0] world0[0:31];
 (* ram_style="block" *) reg [63:0] world1[0:31];
 (* rom_style="block" *) reg [63:0] seed[0:31];
 integer i;
 initial begin
   for (i=0;i<32;i=i+1) begin world0[i]=0; world1[i]=0; end
   $readmemh("seed.hex",seed);
 end
 reg [4:0] address=0, row=0;
 reg [5:0] column=0;
 reg bank=0;
 reg [63:0] q0=0,q1=0,qs=0;
 wire [63:0] source=bank ? q1:q0;
 reg [63:0] north=0,middle=0,south=0,result=0;
 reg [11:0] population=0, sum=0;
 assign led=population[11:5]; // Maximum 2048/32=64, within the 8-bit saturation limit.
 reg [19:0] generation=0; // Decimal digits make the teaching printer small.
 reg [2:0] generation_mod8=0;
 reg paused=0, sparse=0, soup=0, single_step=0;
 // Generations in a row whose population equaled the one before. A world
 // that has settled into still lifes and blinkers keeps its population;
 // after 32 such generations a new soup is sown, so the demo never stops.
 reg [5:0] still=0;
 reg [31:0] lfsr=32'h1aceb00c;
 wire random_bit=lfsr[0];
 wire [3:0] neighbours={3'b0,north[63]}+{3'b0,north[0]}+{3'b0,north[1]}+
   {3'b0,middle[63]}+{3'b0,middle[1]}+
   {3'b0,south[63]}+{3'b0,south[0]}+{3'b0,south[1]};
 wire born=(neighbours==3) || ((neighbours==2) && middle[0]);
 wire fill_bit=soup ? random_bit : middle[0];
 localparam LOAD_ADDR=0, LOAD_WAIT=1, LOAD_CAPTURE=2, LOAD_BITS=3, LOAD_WRITE=4,
   BOUNDARY=5, KEY_WAIT=6, FETCH_N=7, WAIT_N=8, CAP_N=9,
   WAIT_M=10, CAP_M=11, WAIT_S=12, CAP_S=13, CELLS=14, WRITE_ROW=15,
   SWAP=16, CONVERT=17, HEADER=18, PRINT_ADDR=19, PRINT_WAIT=20,
   PRINT_CAPTURE=21, PRINT_BITS=22, PRINT_EOL=23, PRINT_BLANK=24,
   MARKER=25, AFTER_MARKER=26;
 reg [4:0] state=LOAD_ADDR;
 wire writing=state==WRITE_ROW || state==LOAD_WRITE;
 // The worlds are not read while a row is written: in LOAD_WRITE the read
 // address equals the row being written, and an EBR read of a row its other
 // port writes in the same clock returns undefined data on the chip (the
 // emulator makes it unknown). No state captures a read made while writing.
 always @(posedge clk) begin
   if (!writing) begin q0<=world0[address]; q1<=world1[address]; end
   qs<=seed[address];
   if (writing) begin
     if (state==LOAD_WRITE || bank) world0[row]<=result;
     else world1[row]<=result;
   end
 end
 // The population is converted to decimal in 12 shift/add-3 cycles.
 reg [15:0] decimal=0;
 reg [11:0] binary=0;
 reg [3:0] convert_count=0;
 reg [15:0] adjusted;
 integer d;
 always @* begin
   adjusted=decimal;
   for (d=0;d<4;d=d+1)
     if (decimal[d*4+:4]>=5) adjusted[d*4+:4]=decimal[d*4+:4]+3;
 end
 reg [4:0] pos=0;
 reg [6:0] pending_buttons=0;
 // Button pulses are latched throughout printing and computation. UART bytes
 // take priority at each boundary; button events then use a fixed order.
 reg [7:0] command=0;
 function [7:0] header_char;
   input [4:0] p;
   begin
     case(p)
       0:header_char="G"; 1:header_char="E"; 2:header_char="N"; 3:header_char=" ";
       4:header_char="0"+generation[19:16]; 5:header_char="0"+generation[15:12];
       6:header_char="0"+generation[11:8]; 7:header_char="0"+generation[7:4];
       8:header_char="0"+generation[3:0]; 9:header_char=" ";
       10:header_char="P"; 11:header_char="O"; 12:header_char="P";13:header_char=" ";
       14:header_char="0"+decimal[15:12];15:header_char="0"+decimal[11:8];
       16:header_char="0"+decimal[7:4];17:header_char="0"+decimal[3:0];
       default:header_char=10;
     endcase
   end
 endfunction
 function [7:0] restart_char;
   input [4:0] p;
   begin
     case(p)
       0:restart_char="R";1:restart_char="E";2:restart_char="S";3:restart_char="T";
       4:restart_char="A";5:restart_char="R";6:restart_char="T";
       default:restart_char=10;
     endcase
   end
 endfunction
 always @(posedge clk) begin
   send<=0; take<=0;
   pending_buttons<=pending_buttons | (pressed & 7'b1111110);
   if (reset) begin state<=LOAD_ADDR; row<=0; address<=0; sum<=0; end
   else case(state)
     LOAD_ADDR: begin address<=row; state<=LOAD_WAIT; end
     LOAD_WAIT: state<=LOAD_CAPTURE;
     LOAD_CAPTURE: begin middle<=qs; column<=0; result<=0; state<=LOAD_BITS; end
     LOAD_BITS: begin
       result<={fill_bit,result[63:1]}; middle<={middle[0],middle[63:1]};
       lfsr<={lfsr[30:0],lfsr[31]^lfsr[21]^lfsr[1]^lfsr[0]};
       sum<=sum+fill_bit;
       if (column==63) state<=LOAD_WRITE; else column<=column+1;
     end
     LOAD_WRITE: begin
       if (row==31) begin
         population<=sum; bank<=0; generation<=0; generation_mod8<=0;
         binary<=sum; decimal<=0; convert_count<=0; state<=CONVERT;
       end else begin row<=row+1; state<=LOAD_ADDR; end
     end
     BOUNDARY: begin
       if (available && !take) begin command<=key; take<=1; state<=KEY_WAIT; end
       else if (pending_buttons[1]) begin command<="s"; pending_buttons[1]<=0; state<=KEY_WAIT; end
       else if (pending_buttons[2]) begin command<="p"; pending_buttons[2]<=0; state<=KEY_WAIT; end
       else if (pending_buttons[3]) begin sparse<=0; pending_buttons[3]<=0; end
       else if (pending_buttons[4]) begin sparse<=1; pending_buttons[4]<=0; end
       else if (pending_buttons[5] || pending_buttons[6]) begin
         command<="n"; pending_buttons[6:5]<=0; state<=KEY_WAIT;
       end else if (!paused && still==32) begin
         command<="s"; still<=0; state<=KEY_WAIT;
       end else if (!paused) begin row<=0; sum<=0; state<=FETCH_N; end
     end
     KEY_WAIT: begin
       state<=BOUNDARY;
       case(command)
         "r": begin
           // Finish the frame (and its final stop bit) before the marker.
           if (busy || send) state<=KEY_WAIT;
           else begin
             paused<=0; sparse<=0; soup<=0; single_step<=0; still<=0;
             lfsr<=32'h1aceb00c; generation<=0; generation_mod8<=0;
             population<=0; bank<=0; sum<=0; row<=0; column<=0;
             north<=0; middle<=0; south<=0; result<=0; address<=0;
             pending_buttons<=0; pos<=0; state<=MARKER;
           end
         end
         "s","g": begin soup<=command=="s"; still<=0; row<=0; sum<=0; state<=LOAD_ADDR; end
         "p": paused<=!paused;
         "f": sparse<=!sparse;
         "n": if (paused) begin single_step<=1; row<=0; sum<=0; state<=FETCH_N; end
       endcase
     end
     MARKER: if (!busy && !send) begin
       data<=restart_char(pos); send<=1;
       if (pos==7) begin pos<=0; state<=AFTER_MARKER; end else pos<=pos+1;
     end
     AFTER_MARKER: begin row<=0; sum<=0; state<=LOAD_ADDR; end
     FETCH_N: begin address<=row-5'd1; state<=WAIT_N; end
     WAIT_N: state<=CAP_N;
     CAP_N: begin north<=source; address<=row; state<=WAIT_M; end
     WAIT_M: state<=CAP_M;
     CAP_M: begin middle<=source; address<=row+5'd1; state<=WAIT_S; end
     WAIT_S: state<=CAP_S;
     CAP_S: begin south<=source; column<=0; result<=0; state<=CELLS; end
     CELLS: begin
       result<={born,result[63:1]}; sum<=sum+born;
       north<={north[0],north[63:1]}; middle<={middle[0],middle[63:1]};
       south<={south[0],south[63:1]};
       if (column==63) state<=WRITE_ROW; else column<=column+1;
     end
     WRITE_ROW: if (row==31) state<=SWAP;
       else begin row<=row+1; state<=FETCH_N; end
     SWAP: begin
       bank<=!bank; population<=sum; generation_mod8<=generation_mod8+1;
       still<=sum!=population ? 6'd0 : still==32 ? still : still+1'b1;
       // Carry through five decimal digits; the printed number wraps at 100000.
       generation[3:0]<=generation[3:0]+1;
       if (generation[3:0]==9) begin
         generation[3:0]<=0; generation[7:4]<=generation[7:4]+1;
         if (generation[7:4]==9) begin
           generation[7:4]<=0; generation[11:8]<=generation[11:8]+1;
           if (generation[11:8]==9) begin
             generation[11:8]<=0; generation[15:12]<=generation[15:12]+1;
             if (generation[15:12]==9) begin
               generation[15:12]<=0;
               generation[19:16]<=generation[19:16]==9 ? 0:generation[19:16]+1;
             end
           end
         end
       end
       single_step<=0;
       if (!sparse || generation_mod8==7 || single_step) begin
         binary<=sum; decimal<=0; convert_count<=0; state<=CONVERT;
       end else state<=BOUNDARY;
     end
     CONVERT: begin
       decimal<={adjusted[14:0],binary[11]}; binary<={binary[10:0],1'b0};
       if (convert_count==11) begin pos<=0; state<=HEADER; end
       else convert_count<=convert_count+1;
     end
     HEADER: if (!busy && !send) begin
       data<=header_char(pos); send<=1;
       if (pos==18) begin row<=0; state<=PRINT_ADDR; end else pos<=pos+1;
     end
     PRINT_ADDR: begin address<=row; state<=PRINT_WAIT; end
     PRINT_WAIT: state<=PRINT_CAPTURE;
     PRINT_CAPTURE: begin middle<=source; column<=0; state<=PRINT_BITS; end
     PRINT_BITS: if (!busy && !send) begin
       data<=middle[0] ? "#":"."; send<=1; middle<={middle[0],middle[63:1]};
       if (column==63) state<=PRINT_EOL; else column<=column+1;
     end
     PRINT_EOL: if (!busy && !send) begin
       data<=10; send<=1;
       if (row==31) state<=PRINT_BLANK; else begin row<=row+1; state<=PRINT_ADDR; end
     end
     PRINT_BLANK: if (!busy && !send) begin data<=10; send<=1; state<=BOUNDARY; end
     default: state<=LOAD_ADDR;
   endcase
 end
endmodule
