// Original burst controller for this demo. All logic uses the board posedge;
// commands settle for half a clock before the SDRAM's opposite edge.
// Address = {bank[1:0], row[12:0], column[8:0]}, in HALFWORDS.
module burst_controller(
 input clk, input reset, input start, input writing, input full_size,
 input [15:0] wdata, output [23:0] address,
 output read_valid, output [15:0] rdata,
 output reg ready=0, output reg done=0, output reg [31:0] cycles=0,
 output sdram_clk, output sdram_cke, output sdram_csn,
 output reg sdram_rasn, output reg sdram_casn, output reg sdram_wen,
 output [1:0] sdram_ba, output reg [12:0] sdram_a,
 output [1:0] sdram_dqm, inout [15:0] sdram_d);
 localparam INIT=0, IPRE=1, IPWAIT=2, IR1=3, IRW1=4, IR2=5,
  IRW2=6, MODE=7, MWAIT=8, IDLE=9, REF=10, RFC=11,
  ACT=12, RCD=13, WRITE=14, WSTOP=15, WREC=16,
  READ=17, RDATA=18, PRE=19, RP=20, NEXT=21, IDLEREF=22, IDLERFC=23;
 reg [4:0] state=INIT;
 reg [11:0] power_wait=2501;
 reg [7:0] refresh_age=0;
 reg [23:0] base=0;
 reg [6:0] beat=0;
 reg wr=0, full=0, running=0;
 assign sdram_clk=~clk;
 assign sdram_cke=1;
 assign sdram_csn=0;
 assign sdram_dqm=0; // Both bytes always enabled, including read latency.
 assign sdram_ba=base[23:22];
 assign address=base+{17'd0,beat};
 wire driving=state==WRITE;
 assign sdram_d=driving ? wdata : 16'bz;
 assign rdata=sdram_d;
 assign read_valid=state==RDATA;
 // Full page, sequential, CL2, programmed write burst: mode 0x027.
 // Terminate after 128 words (never cross a row). A whole 512-word page
 // would starve refresh: 512*40 ns exceeds 7.8 us. Each 128-word chunk
 // instead starts with REFRESH, so even the longest interval is < 5.6 us.
 always @* begin
   {sdram_rasn,sdram_casn,sdram_wen}=3'b111;
   sdram_a=0;
   case(state)
     IPRE: begin {sdram_rasn,sdram_casn,sdram_wen}=3'b010; sdram_a=13'h400; end
     IR1,IR2,REF,IDLEREF: {sdram_rasn,sdram_casn,sdram_wen}=3'b001;
     MODE: begin {sdram_rasn,sdram_casn,sdram_wen}=3'b000; sdram_a=13'h027; end
     ACT: begin {sdram_rasn,sdram_casn,sdram_wen}=3'b011; sdram_a=base[21:9]; end
     WRITE: if (beat==0) begin
       {sdram_rasn,sdram_casn,sdram_wen}=3'b100; sdram_a={4'b0,base[8:0]};
     end
     READ: begin {sdram_rasn,sdram_casn,sdram_wen}=3'b101; sdram_a={4'b0,base[8:0]}; end
     WSTOP: {sdram_rasn,sdram_casn,sdram_wen}=3'b110;
     // READ at n: first DQ appears after n+1 (CL2), captured on the next
     // board posedge. BT at n+128 preserves the final pipelined read beat.
     RDATA: if (beat==127) {sdram_rasn,sdram_casn,sdram_wen}=3'b110;
     PRE: {sdram_rasn,sdram_casn,sdram_wen}=3'b010;
     default: begin end
   endcase
 end
 always @(posedge clk) begin
   done<=0;
   if (reset) begin
     state<=INIT; power_wait<=2501; ready<=0; running<=0;
     cycles<=0; base<=0; beat<=0; refresh_age<=0;
   end else begin
     if (state>=IDLE) refresh_age<=refresh_age+1;
     if (running) cycles<=cycles+1;
     case(state)
       INIT: if (power_wait!=0) power_wait<=power_wait-1; else state<=IPRE;
       IPRE: state<=IPWAIT;
       IPWAIT: state<=IR1; // tRP=2
       IR1: state<=IRW1;
       IRW1: state<=IR2; // tRFC=2
       IR2: state<=IRW2;
       IRW2: state<=MODE;
       MODE: state<=MWAIT;
       MWAIT: begin state<=IDLE; ready<=1; refresh_age<=0; end // tMRD=2
       IDLE: if (start) begin
         wr<=writing; full<=full_size; base<=0; beat<=0;
         cycles<=0; running<=1; ready<=0; state<=REF;
       end else if (refresh_age>=150) begin ready<=0; state<=IDLEREF; end
       IDLEREF: begin refresh_age<=0; state<=IDLERFC; end
       IDLERFC: begin state<=IDLE; ready<=1; end
       REF: begin refresh_age<=0; state<=RFC; end
       RFC: state<=ACT; // REF to ACTIVE=2, tRFC=2
       ACT: state<=RCD;
       RCD: begin beat<=0; state<=wr ? WRITE:READ; end // tRCD=2
       WRITE: if (beat==127) state<=WSTOP; else beat<=beat+1;
       WSTOP: state<=WREC;
       WREC: state<=PRE; // last write to PRE=3, tWR=2
       READ: begin beat<=0; state<=RDATA; end
       RDATA: if (beat==127) state<=PRE; else beat<=beat+1;
       PRE: state<=RP; // ACTIVE to PRE > 130 clocks: tRAS=2, tRC=2
       RP: state<=NEXT; // PRE to next REF >= 3, tRP=2
       NEXT: if (base==(full ? 24'hffff80:24'h0fff80)) begin
         state<=IDLE; ready<=1; running<=0; done<=1;
       end else begin base<=base+128; beat<=0; state<=REF; end
       default: state<=INIT;
     endcase
   end
 end
endmodule
