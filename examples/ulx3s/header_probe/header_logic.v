// Shared logic. Only driven/ enables DRIVE_PWM and four header pad buffers.
// Four internal PWM periods at 25 MHz: 1/2/5/10 kHz, 25/50/75/10% high.
module header_logic #(parameter DRIVE_PWM=0)(input clk,
  input [27:0] gp, input [27:0] gn, output [3:0] pwm,
  output [7:0] led, output tx);
  reg [14:0] c0=0, c1=0, c2=0, c3=0;
  reg [31:0] n0=0, n1=0, n2=0, n3=0;
  wire [3:0] internal_pwm = {c3<250,c2<3750,c1<6250,c0<6250};
  assign pwm = DRIVE_PWM ? internal_pwm : 4'b0000;
  assign led = {7'b0,gn[0]};
  reg [24:0] interval=0;
  reg first=1;
  // The driven reader always measures a full second, including its first.
  wire report = (!DRIVE_PWM && first) ? interval==24999 : interval==24999999;
  reg [55:0] levels=0;
  reg [31:0] r0=0,r1=0,r2=0,r3=0;
  reg [3:0] previous=0;
  reg [31:0] edges [0:3];
  reg [31:0] high_cycles [0:3];
  reg [31:0] read_edges [0:3];
  reg [31:0] read_high [0:3];
  integer init_pin;
  initial for(init_pin=0;init_pin<4;init_pin=init_pin+1) begin
    edges[init_pin]=0;high_cycles[init_pin]=0;
    read_edges[init_pin]=0;read_high[init_pin]=0;
  end
  reg echo=0;
  reg [1:0] state=0;
  reg [7:0] index=0;
  reg send=0;
  reg [7:0] char=0;
  wire busy;
  uart_tx uart(.clk(clk),.send(send),.data(char),.busy(busy),.tx(tx));
  integer i;
  localparam TAIL=DRIVE_PWM?117:43;
  function [7:0] hex;
    input [3:0] n;
    begin hex=n<10?8'd48+n:8'd65+n-10; end
  endfunction
  always @(posedge clk) begin
    if(c0==24999) begin c0<=0;n0<=n0+1;end else c0<=c0+1;
    if(c1==12499) begin c1<=0;n1<=n1+1;end else c1<=c1+1;
    if(c2==4999) begin c2<=0;n2<=n2+1;end else c2<=c2+1;
    if(c3==2499) begin c3<=0;n3<=n3+1;end else c3<=c3+1;
    interval <= report ? 0 : interval+1;
    previous <= gp[3:0];
    // gp[3:0] comes from BB.O in driven/top.v, never from internal_pwm.
    for(i=0;i<4;i=i+1) if(DRIVE_PWM) begin
      if(report) begin
        read_edges[i]<=edges[i]+(gp[i]&&!previous[i]);
        read_high[i]<=high_cycles[i]+gp[i];
        edges[i]<=0;high_cycles[i]<=0;
      end else begin
        edges[i]<=edges[i]+(gp[i]&&!previous[i]);
        high_cycles[i]<=high_cycles[i]+gp[i];
      end
    end
    send<=0;
    case(state)
      0: if(report || (!first && DRIVE_PWM && gn[0]!=echo)) begin
        first<=0;
        r0<=n0+(c0==24999);r1<=n1+(c1==12499);
        r2<=n2+(c2==4999);r3<=n3+(c3==2499);
        for(i=0;i<28;i=i+1) begin levels[2*i]<=gn[i];levels[2*i+1]<=gp[i];end
        echo<=gn[0];index<=0;state<=1;
      end
      1: begin
        if(index<6) case(index)
          0:char<="H";1:char<="E";2:char<="A";3:char<="D";4:char<="E";5:char<="R";
        endcase
        else if(index==6||index==15||index==24||index==33||index==42)char<=" ";
        else if(index<15)char<=hex(r0>>((14-index)*4));
        else if(index<24)char<=hex(r1>>((23-index)*4));
        else if(index<33)char<=hex(r2>>((32-index)*4));
        else if(index<42)char<=hex(r3>>((41-index)*4));
        else if(DRIVE_PWM && index<117) begin
          case(index)
            43:char<="P";
            44,62,80,98,116:char<=" ";
            53,71,89,107:char<="/";
            default:
              if(index<53)char<=hex(read_edges[0]>>((52-index)*4));
              else if(index<62)char<=hex(read_high[0]>>((61-index)*4));
              else if(index<71)char<=hex(read_edges[1]>>((70-index)*4));
              else if(index<80)char<=hex(read_high[1]>>((79-index)*4));
              else if(index<89)char<=hex(read_edges[2]>>((88-index)*4));
              else if(index<98)char<=hex(read_high[2]>>((97-index)*4));
              else if(index<107)char<=hex(read_edges[3]>>((106-index)*4));
              else char<=hex(read_high[3]>>((115-index)*4));
          endcase
        end else if(index<TAIL+14)char<=hex(levels>>((TAIL+13-index)*4));
        else if(index==TAIL+14)char<=" ";
        else if(index==TAIL+15)char<="E";
        else if(index==TAIL+16)char<=echo?"1":"0";
        else if(index==TAIL+17)char<=13;
        else char<=10;
        send<=1;state<=2;
      end
      2:if(busy)state<=3;
      3:if(!busy)begin index<=index+1;state<=index==TAIL+18?0:1;end
    endcase
  end
endmodule
