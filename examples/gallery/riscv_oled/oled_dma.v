// A DMA-like RGB565 sender. CPU byte enables become PDP EBR write masks;
// its independent synchronous read port prefetches the next SPI word.
module oled_dma(input clk, input resetn, input request,
  input [31:0] addr, wdata, input [3:0] wstrb, output reg [31:0] rdata,
  output reg oled_clk=0, output reg oled_mosi=0, output reg oled_dc=0,
  output reg oled_csn=1, output reg oled_resn=0);
  reg busy=0, frame_mode=0, send_buffer=0, rejected=0;
  reg [13:0] index=0;
  reg [4:0] phase=0;
  reg [7:0] shift=0;
  reg [31:0] frames=0, last_crc=0, crc=32'hffffffff;
  reg [7:0] crc_bits=0;
  reg [3:0] crc_left=0;
  wire writing = request && |wstrb;
  wire registers = addr[31:5] == (32'h06008000 >> 5);
  wire fb_selected = addr[31:15] == (32'h06000000 >> 15) && addr[13:0] < 12288;
  wire protected_buffer = busy && frame_mode && addr[14] == send_buffer;
  // index is the byte currently on SPI, including six window bytes. During
  // its 32 clocks read the word containing the NEXT byte, well before use.
  wire [13:0] next_data = index - 14'd5;
  wire [11:0] read_word = index < 6 ? 12'd0 : next_data[13:2];
  wire [31:0] buffer_q [0:1];
  genvar b, lane;
  generate for (b=0; b<2; b=b+1) begin: buffers
    (* ram_style = "block" *) reg [31:0] words [0:3071];
    for (lane=0; lane<4; lane=lane+1) begin: lanes
      always @(posedge clk)
        if (writing && fb_selected && addr[14] == b && !protected_buffer && wstrb[lane])
          words[addr[13:2]][lane*8 +: 8] <= wdata[lane*8 +: 8];
    end
    reg [31:0] q=0;
    always @(posedge clk)
      if (busy && frame_mode && send_buffer == b && index < 12293)
        q <= words[read_word];
    assign buffer_q[b] = q;
  end endgenerate
  wire [31:0] word_q = buffer_q[send_buffer];
  reg [7:0] next_byte;
  always @* begin
    case (index)
      0: next_byte=0;
      1: next_byte=95;
      2: next_byte=8'h75;
      3: next_byte=0;
      4: next_byte=63;
      default: case (next_data[1:0])
        // Native little-endian halfword stores; SSD1331 wants MSB first.
        0: next_byte=word_q[15:8];
        1: next_byte=word_q[7:0];
        2: next_byte=word_q[31:24];
        3: next_byte=word_q[23:16];
      endcase
    endcase
  end
  always @* begin
    rdata=0;
    case (addr[4:2])
      1: rdata={31'd0,oled_resn};
      2: rdata={28'd0,rejected,frame_mode,send_buffer,busy};
      3: rdata=last_crc;
      4: rdata=frames;
    endcase
  end
  always @(posedge clk) begin
    if (!resetn) begin
      busy<=0; frame_mode<=0; send_buffer<=0; rejected<=0;
      index<=0; phase<=0; shift<=0; frames<=0; last_crc<=0;
      crc<=32'hffffffff; crc_bits<=0; crc_left<=0;
      oled_clk<=0; oled_mosi<=0; oled_dc<=0; oled_csn<=1; oled_resn<=0;
    end else begin
      if (writing && fb_selected && protected_buffer) rejected<=1;
      if (writing && registers && addr[4:2]==1 && wstrb[0]) begin
        oled_resn<=wdata[0];
        if (wdata[3]) rejected<=0;
      end
      // Reflected ISO-HDLC CRC: byte bits LSB first while SPI goes MSB first.
      if (crc_left!=0) begin
        crc <= (crc>>1) ^ ((crc[0]^crc_bits[0]) ? 32'hedb88320 : 32'd0);
        crc_bits<=crc_bits>>1; crc_left<=crc_left-1'b1;
      end
      if (!busy) begin
        if (writing && registers && wstrb[0] &&
            (addr[4:2]==0 || (addr[4:2]==1 && wdata[1]))) begin
          busy<=1; frame_mode<=addr[4:2]==1; send_buffer<=wdata[2];
          index<=0; phase<=0; oled_csn<=0; oled_dc<=0;
          shift<=addr[4:2]==0 ? wdata[7:0] : 8'h15;
          oled_mosi<=addr[4:2]==0 ? wdata[7] : 1'b0;
          crc<=32'hffffffff; crc_left<=0;
        end
      end else begin
        phase<=phase+1'b1;
        if (phase[1:0]==1) oled_clk<=1;
        if (phase[1:0]==3) begin
          oled_clk<=0;
          if (phase!=31) begin shift<={shift[6:0],1'b0}; oled_mosi<=shift[6]; end
          else if (!frame_mode || index==12293) begin
            busy<=0; oled_csn<=1;
            if (frame_mode) begin last_crc<=~crc; frames<=frames+1'b1; end
          end else begin
            index<=index+1'b1; shift<=next_byte; oled_mosi<=next_byte[7];
            oled_dc<=index>=5;
            if (index>=5) begin crc_bits<=next_byte; crc_left<=8; end
          end
        end
      end
    end
  end
endmodule
