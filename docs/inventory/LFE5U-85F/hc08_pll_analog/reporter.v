// HC08: 115200 8N1, 25 MHz pin domain. No LOCK/CLKOP gating of reporting.
// HC08{L|S}{LOCK:1 hex}{pin ticks:8 hex}{CLKOP edges:8 hex}\r\n
// L = live, S = repeatedly replayed startup, timestamps from configuration.
module hc08_reporter #(parameter STARTUP_LAST=200)
  (input clk, input clkop, input locked, output ftdi_rxd);
  reg [31:0] op_binary=0, op_gray=0;
  wire [31:0] op_next=op_binary+1'b1;
  always @(posedge clkop) begin
    op_binary<=op_next;
    op_gray<=op_next^(op_next>>1);
  end
  (* async_reg="true", keep=1 *) reg [31:0] gray_meta=0, gray_sync=0;
  (* async_reg="true", keep=1 *) reg [1:0] lock_sync=0;
  always @(posedge clk) begin
    gray_meta<=op_gray; gray_sync<=gray_meta;
    lock_sync<={lock_sync[0],locked};
  end
  function [31:0] ungray(input [31:0] g);
    integer i;
    begin
      ungray[31]=g[31];
      for (i=30; i>=0; i=i-1) ungray[i]=ungray[i+1]^g[i];
    end
  endfunction
  reg [31:0] pin_count=0;
  reg [17:0] interval=0;
  wire sample=interval==0;
  wire [67:0] observation={3'b0,lock_sync[1],pin_count,ungray(gray_sync)};
  // Retain t=0,10,...,2000 ms. Read/write ports are ONLY on the pin clock.
  reg [67:0] startup [0:255];
  reg [7:0] saved=0, replay_index=0;
  reg full=0;
  reg [67:0] replay_data;
  always @(posedge clk) begin
    pin_count<=pin_count+1'b1;
    interval<=sample ? 18'd249999 : interval-1'b1;
    if (sample && !full) begin
      startup[saved]<=observation;
      if (saved==STARTUP_LAST) full<=1;
      else saved<=saved+1'b1;
    end
    replay_data<=startup[replay_index];
  end

  reg [67:0] payload=0;
  reg sending=0, replay_after=0, kind=0;
  reg [4:0] ci=0;
  function [7:0] hex(input [3:0] n);
    hex=n<10 ? 8'h30+n : 8'h41+n-10;
  endfunction
  function [7:0] character(input [4:0] index, input [67:0] data, input startup_kind);
    case(index)
      0: character="H"; 1: character="C"; 2: character="0"; 3: character="8";
      4: character=startup_kind ? "S" : "L";
      22: character=8'h0d; 23: character=8'h0a;
      default: character=hex(data[67-4*(index-5) -: 4]);
    endcase
  endfunction
  wire [7:0] ch=character(ci,payload,kind);
  reg tx=1, busy=0;
  reg [7:0] baud=0;
  reg [3:0] bitn=0;
  reg [9:0] shift=10'h3ff;
  always @(posedge clk) begin
    if (sample) begin
      payload<=observation; ci<=0; kind<=0;
      sending<=1; replay_after<=full;
    end else if (sending && !busy) begin
      shift<={1'b1,ch,1'b0}; tx<=0;
      busy<=1; baud<=0; bitn<=0;
    end else if (busy) begin
      if (baud==216) begin
        baud<=0; bitn<=bitn+1'b1;
        shift<={1'b1,shift[9:1]}; tx<=shift[1];
        if (bitn==9) begin
          busy<=0; tx<=1;
          if (ci!=23) ci<=ci+1'b1;
          else if (replay_after) begin
            payload<=replay_data; ci<=0; kind<=1; replay_after<=0;
            replay_index<=replay_index==STARTUP_LAST ? 0 : replay_index+1'b1;
          end else sending<=0;
        end
      end else baud<=baud+1'b1;
    end
  end
  assign ftdi_rxd=tx;
endmodule
