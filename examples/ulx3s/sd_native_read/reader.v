// Read-only native SD host. The command-index ROM below is the only source
// of transmitted indices. It has no write/erase/lock command or data driver.
module reader(input clk, output reg sd_clk=0, inout sd_cmd, inout [3:0] sd_d,
  output reg ready=0, output reg error=0, output reg [31:0] ocr=0,
  output reg [15:0] rca=0, output reg [127:0] cid=0, output reg [127:0] csd=0,
  output wire [31:0] block_crc, output reg crc_ok=0,
  input [8:0] dump_address, output [7:0] dump_data);
  reg cmd_oe=0,cmd_out=1;
  wire cmd_in;
  BB command_pad(.I(cmd_out),.T(!cmd_oe),.O(cmd_in),.B(sd_cmd));
  wire [3:0] dat;
  BB d0(.I(1'b0),.T(1'b1),.O(dat[0]),.B(sd_d[0]));
  BB d1(.I(1'b0),.T(1'b1),.O(dat[1]),.B(sd_d[1]));
  BB d2(.I(1'b0),.T(1'b1),.O(dat[2]),.B(sd_d[2]));
  BB d3(.I(1'b0),.T(1'b1),.O(dat[3]),.B(sd_d[3]));
  reg [7:0] block [0:511];
  assign dump_data=block[dump_address];
  reg [31:0] crc32=32'hffffffff;
  assign block_crc=~crc32;
  reg slow=1;
  reg [5:0] divider=0;
  reg [3:0] stage=0;
  reg [3:0] state=0;
  localparam WARM=0,PREP=1,TX=2,TURN=3,WAIT_RESPONSE=4,RX=5,FINISH=6,
    GAP=7,WAIT_DATA=8,DATA=9,DATA_CRC=10,DATA_END=11,HOLD=12;
  reg [6:0] warm=0;
  reg [5:0] bit_index=47;
  reg [7:0] response_bits=0;
  reg [135:0] response=0;
  reg [47:0] packet=0;
  reg [13:0] timeout=0;
  reg [12:0] polls=0;
  reg [4:0] gap=0;
  reg [9:0] nibble_index=0;
  reg [3:0] high_nibble=0;
  reg [4:0] crc_index=0;
  reg [15:0] c0=0,c1=0,c2=0,c3=0;
  reg [15:0] r0=0,r1=0,r2=0,r3=0;

  function [5:0] command_index;
    input [3:0] step;
    begin case(step)
      0:command_index=6'd0;1:command_index=6'd8;
      2:command_index=6'd55;3:command_index=6'd41;
      4:command_index=6'd2;5:command_index=6'd3;
      6:command_index=6'd9;7:command_index=6'd7;
      8:command_index=6'd55;9:command_index=6'd6;
      10:command_index=6'd16;11:command_index=6'd17;
      default:command_index=6'd0;
    endcase end
  endfunction
  function [47:0] command_frame;
    input [3:0] step;input [15:0] address;
    reg [31:0] argument;
    reg [47:0] frame;
    reg [6:0] crc;
    integer b;
    begin
      case(step)
        1:argument=32'h000001aa;
        3:argument=32'h40ff8000;
        6,7,8:argument={address,16'b0};
        9:argument=2;
        10:argument=512;
        default:argument=0;
      endcase
      frame={2'b01,command_index(step),argument,7'b0,1'b1};crc=0;
      for(b=47;b>=8;b=b-1)
        crc={crc[5:0],1'b0}^((crc[6]^frame[b])?7'h09:7'h00);
      frame[7:1]=crc;command_frame=frame;
    end
  endfunction
  function [15:0] crc_bit;
    input [15:0] crc;input b;
    begin crc_bit={crc[14:0],1'b0}^((crc[15]^b)?16'h1021:16'h0000);end
  endfunction
  function [31:0] crc_byte;
    input [31:0] crc;input [7:0] b;
    reg [31:0] c;integer n;
    begin c=crc^b;for(n=0;n<8;n=n+1)c=(c>>1)^(c[0]?32'hedb88320:32'h0);crc_byte=c;end
  endfunction

  always @(posedge clk) begin
    if(divider==(slow?31:1)) begin
      divider<=0;sd_clk<=!sd_clk;
      if(sd_clk) begin // Falling SD edge: launch, release and process a full response.
        case(state)
          PREP:begin packet<=command_frame(stage,rca);cmd_oe<=1;cmd_out<=0;bit_index<=47;state<=TX;end
          TX:cmd_out<=packet[bit_index];
          TURN:begin cmd_oe<=0;timeout<=0;
            if(stage==0)begin gap<=0;stage<=1;state<=GAP;end else state<=WAIT_RESPONSE;
          end
          FINISH:begin
            case(stage)
              1:if(response[39:8]!=32'h1aa)error<=1;
              3:ocr<=response[39:8];
              4:cid<=response[127:0];
              5:rca<=response[39:24];
              6:csd<=response[127:0];
            endcase
            if(stage==11)begin state<=WAIT_DATA;timeout<=0;end
            else begin
              if(stage==3 && !response[39])begin stage<=2;polls<=polls+1;if(polls==4095)error<=1;end
              else stage<=stage+1;
              if(stage==10)slow<=0;
              gap<=0;state<=GAP;
            end
          end
        endcase
      end else begin // Rising SD edge: sample responses and all four DAT lines.
        case(state)
          WARM:if(warm==79)state<=PREP;else warm<=warm+1;
          TX:if(bit_index==0)state<=TURN;else bit_index<=bit_index-1;
          WAIT_RESPONSE:begin
            timeout<=timeout+1;
            if(!cmd_in)begin response<=0;response_bits<=1;state<=RX;end
            else if(timeout==16383)error<=1;
          end
          RX:begin response<={response[134:0],cmd_in};response_bits<=response_bits+1;
            if(response_bits==((stage==4||stage==6)?135:47))state<=FINISH;
          end
          GAP:begin gap<=gap+1;
            if(gap>=8 && (stage!=8||dat[0]))state<=PREP;
          end
          WAIT_DATA:begin
            timeout<=timeout+1;
            if(dat==0)begin nibble_index<=0;c0<=0;c1<=0;c2<=0;c3<=0;crc32<=32'hffffffff;state<=DATA;end
            else if(timeout==16383)error<=1;
          end
          DATA:begin
            c0<=crc_bit(c0,dat[0]);c1<=crc_bit(c1,dat[1]);c2<=crc_bit(c2,dat[2]);c3<=crc_bit(c3,dat[3]);
            if(!nibble_index[0])high_nibble<=dat;
            else begin block[nibble_index[9:1]]<={high_nibble,dat};crc32<=crc_byte(crc32,{high_nibble,dat});end
            nibble_index<=nibble_index+1;
            if(nibble_index==1023)begin crc_index<=0;r0<=0;r1<=0;r2<=0;r3<=0;state<=DATA_CRC;end
          end
          DATA_CRC:begin
            r0<={r0[14:0],dat[0]};r1<={r1[14:0],dat[1]};r2<={r2[14:0],dat[2]};r3<={r3[14:0],dat[3]};
            crc_index<=crc_index+1;if(crc_index==15)state<=DATA_END;
          end
          DATA_END:begin crc_ok<=dat==15&&c0==r0&&c1==r1&&c2==r2&&c3==r3;ready<=1;state<=HOLD;end
        endcase
      end
    end else divider<=divider+1;
  end
endmodule
