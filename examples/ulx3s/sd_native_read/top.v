// Read-only native SD reference. CID and B0RAW from a real card stay in out/sd.
module top(input clk_25mhz, output sd_clk, inout sd_cmd, inout [3:0] sd_d,
  output ftdi_rxd, output [7:0] led);
  wire ready,error,crc_ok;
  wire [31:0] ocr,block_crc;
  wire [15:0] rca;
  wire [127:0] cid,csd;
  reg [1:0] line=0;
  reg [10:0] index=0;
  wire [8:0] address=(index-10)>>1;
  wire [7:0] data;
  reader reader(.clk(clk_25mhz),.sd_clk(sd_clk),.sd_cmd(sd_cmd),.sd_d(sd_d),
    .ready(ready),.error(error),.ocr(ocr),.rca(rca),.cid(cid),.csd(csd),
    .block_crc(block_crc),.crc_ok(crc_ok),.dump_address(address),.dump_data(data));
  assign led={error,ready,crc_ok,5'b0};
  reg [24:0] period=0;
  reg [2:0] state=0;
  reg send=0;
  reg [7:0] char=0;
  wire busy;
  uart_tx uart(.clk(clk_25mhz),.send(send),.data(char),.busy(busy),.tx(ftdi_rxd));
  function [7:0] hex;
    input [3:0] n;begin hex=n<10?8'd48+n:8'd65+n-10;end
  endfunction
  always @(posedge clk_25mhz) begin
    send<=0;
    if(period==24999999)period<=0;else period<=period+1;
    case(state)
      0:if(ready)begin line<=0;index<=0;state<=1;end
      1:begin
        case(line)
          0:case(index)
            0:char<="S";
            1:char<="D";
            2:char<="N";
            3:char<=" ";
            4:char<="O";
            5:char<="C";
            6:char<="R";
            7:char<=" ";
            16:char<=" ";
            17:char<="R";
            18:char<="C";
            19:char<="A";
            20:char<=" ";
            25:char<=" ";
            26:char<="C";
            27:char<="S";
            28:char<="D";
            29:char<=" ";
            62:char<=13;
            63:char<=10;
            default:if(index<16)char<=hex(ocr>>((15-index)*4));
              else if(index<25)char<=hex(rca>>((24-index)*4));else char<=hex(csd>>((61-index)*4));
          endcase
          1:case(index)
            0:char<="S";
            1:char<="D";
            2:char<="N";
            3:char<=" ";
            4:char<="C";
            5:char<="I";
            6:char<="D";
            7:char<=" ";
            40:char<=13;
            41:char<=10;
            default:char<=hex(cid>>((39-index)*4));
          endcase
          2:case(index)
            0:char<="S";
            1:char<="D";
            2:char<="N";
            3:char<=" ";
            4:char<="B";
            5:char<="0";
            6:char<=" ";
            15:char<=" ";
            16:char<="C";
            17:char<="R";
            18:char<="C";
            19:char<="1";
            20:char<="6";
            21:char<=" ";
            26:char<=10;
            22:char<=crc_ok?"o":"b";23:char<=crc_ok?"k":"a";
            24:char<=crc_ok?13:"d";25:char<=crc_ok?10:13;
            default:char<=hex(block_crc>>((14-index)*4));
          endcase
          3:case(index)
            0:char<="S";
            1:char<="D";
            2:char<="N";
            3:char<=" ";
            4:char<="B";
            5:char<="0";
            6:char<="R";
            7:char<="A";
            8:char<="W";
            9:char<=" ";
            1034:char<=13;
            1035:char<=10;
            default:char<=index[0]?hex(data[3:0]):hex(data[7:4]);
          endcase
        endcase
        send<=1;state<=2;
      end
      2:if(busy)state<=3;
      3:if(!busy)begin
        if(index==((line==0)?63:(line==1)?41:(line==2)?(crc_ok?25:26):1035))begin
          index<=0;
          if(line==3)state<=4;
          else begin line<=line+1;state<=1;end
        end else begin index<=index+1;state<=1;end
      end
      4:if(period==24999999)begin line<=0;index<=0;state<=1;end
    endcase
  end
endmodule
