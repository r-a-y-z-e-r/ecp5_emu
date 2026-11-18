// Black-box pad decoders only: sample timing is LED0's documented sample toggle.
#include "Vtop.h"
#include "verilated.h"
#include <zlib.h>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <deque>
#include <fstream>
#include <regex>
#include <stdexcept>
#include <string>
#include <vector>
double sc_time_stamp(){return 0;}
void require(bool ok,const std::string& why){if(!ok)throw std::runtime_error(why);}
struct Picture{uint32_t crc; std::vector<uint8_t> bytes;};
struct Host{uint64_t cycle; char key;};
int main(int argc,char**argv){try{
 require(argc==4,"usage: Vtop cycles output-prefix scenario");
 uint64_t cycles=std::strtoull(argv[1],nullptr,10); std::string prefix=argv[2],scenario=argv[3];
 Vtop top; top.btn=1; top.ftdi_txd=1;
 std::vector<Host> host;
 if(scenario!="power")host.push_back({0,'r'});
 if(scenario=="late")host={{470000,'r'}}; // OLED/UART already active; retain every prior line.
 if(scenario=="controls"){
   // Spaced UART notes, pause/resume, then all six notes as a queued burst.
   for(int i=0;i<6;i++)host.push_back({uint64_t(700000+i*160000),"asdfgh"[i]});
   host.push_back({1800000,'p'}); host.push_back({3000000,'p'});
   for(int i=0;i<6;i++)host.push_back({uint64_t(3500000+i*2170),"asdfgh"[i]});
 }
 if(scenario=="restart"){
   // Two queued restarts and a note following them; neither restart may eat it.
   host.push_back({700000,'r'});host.push_back({702170,'r'});host.push_back({704340,'a'});
 }
 std::ofstream events(prefix+".events"),samples(prefix+".samples"),frames(prefix+".frames");
 for(const auto& h:host)events<<"UART "<<h.cycle<<" "<<h.key<<"\n";
 std::string text,line; std::vector<uint8_t> codes; std::deque<Picture> pictures;
 uint8_t init[]={0xae,0xa0,0x72,0xa1,0,0xa2,0,0xa4,0xa8,0x3f,0xad,0x8e,0xb0,0x0b,
  0xb1,0x31,0xb3,0xf0,0x8a,0x64,0x8b,0x78,0x8c,0x64,0xbb,0x3a,0xbe,0x3e,0x87,6,
  0x81,0x91,0x82,0x50,0x83,0x7d,0xaf};
 uint8_t window[]={0x15,0,95,0x75,0,63};
 unsigned ii=0,wi=0,spi_bits=0,spi_byte=0,checked=0,total_frames=0,audios=0,markers=0,notes=0;
 unsigned uart_wait=0;int uart_bit=0,uart_byte=0;
 bool prev_tx=1,prev_clk=0,prev_cs=1,prev_res=0,prev_sample=0,filling=0;
 bool prev_pause=false;
 bool armed=scenario=="power";
 uint64_t last_rise=0,last_sample=0,snapshot_cycle=0;unsigned block_pos=0;
 uint32_t pad_crc=::crc32(0,nullptr,0);
 std::deque<uint32_t> audio_blocks;
 std::vector<uint8_t> data;
 std::vector<unsigned> played;
 std::regex fline("F([0-9A-F]{4}) C([0-9A-F]{8})\\r");
 std::regex aline("AUDIO 3125 ([0-9A-F]{8})\\r");
 for(uint64_t cycle=0;cycle<cycles;cycle++){
   top.ftdi_txd=1;
   if(scenario=="late" && cycle==470000)require(uart_wait || !line.empty(),"late restart must interrupt an active UART line");
   for(auto h:host)if(cycle>=h.cycle && cycle<h.cycle+2170){
     unsigned b=(cycle-h.cycle)/217;top.ftdi_txd=b==0?0:b==9?1:(h.key>>(b-1))&1;
   }
   top.btn=1;
   if(scenario=="controls")for(int b=1;b<=6;b++){
     uint64_t start=4500000+(b-1)*150000;
     if(cycle==start)events<<"BUTTON "<<start<<" "<<b<<"\n";
     if(cycle>=start && cycle<start+30000)top.btn|=1<<b;
   }
   top.clk_25mhz=0;top.eval();top.clk_25mhz=1;top.eval();
   if(!top.oled_resn){
     ii=wi=spi_bits=spi_byte=0;filling=false;data.clear();last_rise=0;
     // A soft reset is legal only after completed frames have been reported.
     if(prev_res)require(pictures.empty(),"restart dropped a completed OLED report");
   }else if(!top.oled_csn && !prev_clk && top.oled_clk){
     if(last_rise)require(cycle-last_rise>=4,"SPI faster than 6.25 MHz");
     if(spi_bits)require(cycle-last_rise==4,"SPI bit period");
     last_rise=cycle;spi_byte=(spi_byte<<1)|top.oled_mosi;
     if(spi_bits==0 && !top.oled_dc && ii==sizeof(init) && wi==0) snapshot_cycle=cycle-4;
     if(++spi_bits==8){
       uint8_t b=spi_byte;spi_bits=spi_byte=0;
       if(top.oled_dc){
         require(filling,"data without full window");data.push_back(b);
         if(data.size()==12288){
           auto c=::crc32(0,data.data(),data.size());pictures.push_back({uint32_t(c),data});
           std::ofstream raw(prefix+".frame"+std::to_string(total_frames)+".rgb565",std::ios::binary);
           raw.write((const char*)data.data(),data.size());
           frames<<total_frames<<" "<<cycle<<" "<<snapshot_cycle<<" "<<std::hex<<c<<std::dec<<"\n";
           total_frames++;data.clear();filling=false;wi=0;
         }
       }else if(ii<sizeof(init))require(b==init[ii++],"OLED init differs");
       else{require(!filling && data.empty(),"short fill");require(b==window[wi++],"OLED window differs");
         if(wi==sizeof(window))filling=true;}
     }
   }
   if(!prev_cs && top.oled_csn)require(spi_bits==0,"partial SPI transaction");
   prev_cs=top.oled_csn;prev_clk=top.oled_clk;prev_res=top.oled_resn;
   bool pause=(top.led>>6)&1;
   if(pause!=prev_pause)events<<"PAUSE "<<cycle<<" "<<pause<<"\n";
   prev_pause=pause;
   bool sample=top.led&1;
   if(armed && sample!=prev_sample){
     // RES# low can also reset the toggle; ignore that one reset transition.
     if(last_sample && cycle-last_sample!=1000){
       require(!top.oled_resn,"sample spacing changed while running");
       armed=false;
     }else{
       require(top.audio_l>=4 && top.audio_l<=12 && top.audio_r>=4 && top.audio_r<=12,"unsafe DAC code");
       require(top.audio_v==8,"V is not midscale");
       samples<<cycle<<" "<<unsigned(top.audio_l)<<" "<<unsigned(top.audio_r)<<"\n";
       uint8_t pair[]={top.audio_l,top.audio_r};codes.insert(codes.end(),pair,pair+2);
       pad_crc=::crc32(pad_crc,pair,2);last_sample=cycle;
       if(++block_pos==3125){audio_blocks.push_back(pad_crc);block_pos=0;pad_crc=::crc32(0,nullptr,0);}
     }
   }
   prev_sample=sample;
   bool tx=top.ftdi_rxd;
   if(!uart_wait && prev_tx && !tx){uart_wait=108;uart_bit=-1;uart_byte=0;}
   else if(uart_wait && --uart_wait==0){
     if(uart_bit==-1){require(!tx,"UART start");uart_bit=0;uart_wait=217;}
     else if(uart_bit<8){uart_byte|=int(tx)<<uart_bit++;uart_wait=217;}
     else{
       require(tx,"UART stop");text+=char(uart_byte);
       if(uart_byte=='\n'){
         std::smatch m;
         if(line=="RESTART\r"){
           require(pictures.empty(),"restart lost frame line");
           require(audio_blocks.empty(),"restart lost audio line");
           markers++;checked=0;armed=true;last_sample=0;block_pos=0;pad_crc=::crc32(0,nullptr,0);
           samples<<"RESTART "<<cycle<<"\n";codes.clear();
         }else if(std::regex_match(line,m,fline)){
           require(!pictures.empty(),"UART frame without SPI");
           require(std::stoul(m[1],nullptr,16)==(checked&65535),"frame sequence");
           require(std::stoul(m[2],nullptr,16)==pictures.front().crc,"OLED UART CRC != pad CRC");
           pictures.pop_front();checked++;
         }else if(std::regex_match(line,m,aline)){
           if(armed){require(!audio_blocks.empty(),"audio report without samples");
             require(std::stoul(m[1],nullptr,16)==audio_blocks.front(),"AUDIO CRC != DAC pad CRC");audio_blocks.pop_front();}
           audios++;
         }else if(line.rfind("NOTE ",0)==0){notes++;played.push_back(std::stoul(line.substr(5,3)));}
         else require(false,"unknown line: "+line);
         line.clear();
       }else line+=char(uart_byte);
     }
   }
   prev_tx=tx;
 }
 top.final();
 require(pictures.empty(),"end before OLED report: adjust cycle budget");
 require(audio_blocks.empty(),"end before AUDIO report");
 require(line.empty() && !uart_wait,"end inside UART line");
 require(total_frames>0 && audios>0,"insufficient coverage");
 if(scenario=="power")require(markers==0,"power-on printed RESTART");
 if(scenario=="controls"){require(notes==18 && markers==1,"lost live note");
   for(unsigned i=0;i<played.size();i++)require(played[i]==std::vector<unsigned>{72,74,76,77,79,81}[i%6],"wrong live note order");}
 if(scenario=="restart")require(markers==3 && notes==1,"queued restart lost a command");
 std::ofstream out(prefix+".uart",std::ios::binary);out<<text;
 std::printf("%s: %llu clocks, %u SPI/UART CRCs, %u AUDIO/pad CRCs, %u NOTE, %u RESTART: PASS\n",
   scenario.c_str(),(unsigned long long)cycles,total_frames,audios,notes,markers);
 return 0;
}catch(const std::exception& e){std::fprintf(stderr,"FAIL: %s\n",e.what());return 1;}}
