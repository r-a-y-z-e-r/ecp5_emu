// Independent pad decoders: UART 8N1 and mode-0 SSD1331 SPI.
#include "Vtop.h"
#include "verilated.h"
#include <zlib.h>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <deque>
#include <fstream>
#include <iterator>
#include <regex>
#include <stdexcept>
#include <string>
#include <vector>
#include "playtest.h"
#include "wall_playtest.h"
#include "serve_playtest.h"
double sc_time_stamp() { return 0; }
struct Picture { uint32_t crc; uint64_t cycle, start; std::vector<uint8_t> bytes; };
int main(int argc,char**argv) {
 try {
    Verilated::commandArgs(argc,argv);
    if(argc<5) return 2;
    uint64_t cycles=std::strtoull(argv[1],nullptr,10);
    std::ifstream input(argv[2],std::ios::binary);
    std::string keys((std::istreambuf_iterator<char>(input)),{}), text, line;
    Vtop top; top.btn=1; top.sw=argc>5 ? std::strtoul(argv[5],nullptr,0) : 0;
    // Optional human exercise / late restarts are separate from the golden run.
    std::string scenario=argc>6 ? argv[6] : "";
    std::ofstream timing(std::string(argv[4])+".csv");
    timing<<"frame,first_data_edge,last_data_edge,uart_end\n";
    WindowControls playtest;
    WallControls wall_playtest;
    ServeControls serve_playtest;
    uint64_t fill_start=0, ran_cycles=0;
    bool prev_tx=true, prev_clk=false, prev_cs=true;
    int wait=0, uart_bit=0, uart_byte=0, spi_bits=0, spi_byte=0;
    size_t init_index=0, window_index=0;
    bool filling=false;
    const uint8_t init[]={0xae,0xa0,0x72,0xa1,0,0xa2,0,0xa4,0xa8,0x3f,0xad,0x8e,
      0xb0,0x0b,0xb1,0x31,0xb3,0xf0,0x8a,0x64,0x8b,0x78,0x8c,0x64,0xbb,0x3a,
      0xbe,0x3e,0x87,6,0x81,0x91,0x82,0x50,0x83,0x7d,0xaf};
    const uint8_t window[]={0x15,0,95,0x75,0,63};
    std::deque<Picture> completed;
    std::vector<uint8_t> data;
    uint64_t first_fill=0, last_fill=0, first_report=0,last_report=0, last_rise=0;
    unsigned checked=0, hits=0, misses=0, players=0, switches=0, overs=0;
    std::regex report("F([0-9A-F]{4}) C([0-9A-F]{8}) S([0-9]{3}) L([0-3]) B([0-9]{2})\\r");
    for(uint64_t cycle=0;cycle<cycles;cycle++) {
        uint64_t host_cycle=cycle;
        if(scenario=="late" && cycle>=500000) host_cycle=cycle-500000;
        uint64_t host_bit=host_cycle/217, k=host_bit/10; int b=host_bit%10;
        top.ftdi_txd=k>=keys.size() || b==9 ? 1 : b==0 ? 0 :
            (static_cast<unsigned char>(keys[k])>>(b-1))&1;
        if(scenario=="human") {
          top.btn=1 | ((cycle>=2000000 && cycle<2500000) ? 2 : 0)
                    | ((cycle>=3500000 && cycle<4000000) ? 2 : 0)
                    | ((cycle>=4500000 && cycle<5500000) ? 32 : 0)
                    | ((cycle>=6000000 && cycle<7000000) ? 64 : 0);
          top.sw=cycle>=8000000 ? 15 : cycle>=7000000 ? 2 : 0;
        }
        if(scenario=="p18") top.btn=playtest.buttons(cycle);
        if(scenario=="p28") top.btn=serve_playtest.buttons(cycle);
        if(scenario=="p27") { top.btn=wall_playtest.buttons(cycle); top.sw=wall_playtest.switches(); }
        ran_cycles=cycle+1;
        top.clk_25mhz=0;top.eval(); top.clk_25mhz=1;top.eval();
        if(!top.oled_resn) {
          init_index=window_index=0; filling=false; data.clear(); completed.clear(); spi_bits=spi_byte=0;
          last_rise=0;
        } else {
          if(!top.oled_csn && !prev_clk && top.oled_clk) {
            if(last_rise && cycle-last_rise<4) throw std::runtime_error("SPI exceeds 6.25 MHz");
            last_rise=cycle;
            if(top.oled_dc && spi_bits==0 && data.empty()) fill_start=cycle;
            spi_byte=(spi_byte<<1)|top.oled_mosi;
            if(++spi_bits==8) {
              uint8_t byte=spi_byte; spi_bits=spi_byte=0;
              if(top.oled_dc) {
                if(!filling) throw std::runtime_error("data outside full address window");
                data.push_back(byte);
                if(data.size()==12288) {
                  uint32_t crc=::crc32(0,data.data(),data.size());
                  completed.push_back({crc,cycle,fill_start,data});
                  if(scenario=="p18") playtest.picture(cycle,data,crc,top.led);
                  if(scenario=="p27") wall_playtest.picture(data,top.led);
                  if(!first_fill) first_fill=cycle; last_fill=cycle;
                  filling=false; window_index=0; data.clear();
                }
              } else if(init_index<sizeof(init)) {
                if(byte!=init[init_index++]) throw std::runtime_error("wrong common init");
              } else {
                if(filling || !data.empty()) throw std::runtime_error("wrong frame byte count");
                if(byte!=window[window_index++]) throw std::runtime_error("wrong address window");
                if(window_index==sizeof(window)) filling=true;
              }
            }
          }
          if(!prev_cs && top.oled_csn && spi_bits) throw std::runtime_error("partial SPI byte");
        }
        prev_cs=top.oled_csn; prev_clk=top.oled_clk;
        bool tx=top.ftdi_rxd;
        if(!wait && prev_tx && !tx) {wait=108;uart_bit=-1;uart_byte=0;}
        else if(wait && --wait==0) {
            if(uart_bit==-1) {if(tx) throw std::runtime_error("UART start"); uart_bit=0;wait=217;}
            else if(uart_bit<8) {uart_byte|=int(tx)<<uart_bit;uart_bit++;wait=217;}
            else {
              if(!tx) throw std::runtime_error("UART stop");
              text+=char(uart_byte);
              if(uart_byte=='\n') {
                std::smatch m;
                if(line.find("RESTART\r")!=std::string::npos) {
                  checked=hits=misses=0; first_fill=last_fill=first_report=last_report=0;
                } else if(std::regex_match(line,m,report)) {
                  unsigned f=std::stoul(m[1],nullptr,16);
                  uint32_t c=std::stoul(m[2],nullptr,16);
                  if(completed.empty() || c!=completed.front().crc || f!=(checked&65535))
                    throw std::runtime_error("SPI CRC/frame number disagrees with UART: "+line);
                  if(f==75) {
                    std::ofstream raw(argv[4],std::ios::binary);
                    raw.write(reinterpret_cast<const char*>(completed.front().bytes.data()),12288);
                  }
                  if(scenario=="p28") serve_playtest.picture(cycle,completed.front().bytes,c,top.led,
                      std::stoul(m[3]),std::stoul(m[4]),std::stoul(m[5]));
                  timing<<f<<","<<completed.front().start<<","<<completed.front().cycle<<","<<cycle<<"\n";
                  completed.pop_front(); checked++; last_report=cycle;
                  if(!first_report) first_report=cycle;
                } else if(line.rfind("HIT ",0)==0) hits++;
                else if(line=="MISS\r") misses++;
                else if(line=="PLAYER\r") players++;
                else if(line.rfind("OVER ",0)==0) overs++;
                else if(line.rfind("SW ",0)==0) switches++;
                line.clear();
              } else line+=char(uart_byte);
            }
        }
        prev_tx=tx;
        if(((scenario=="p18" && playtest.finished()) || (scenario=="p27" && wall_playtest.finished()) ||
            (scenario=="p28" && serve_playtest.finished())) &&
           completed.empty() && line.empty() && !wait) break;
    }
    top.final();
    std::ofstream output(argv[3],std::ios::binary); output.write(text.data(),text.size());
    std::printf("RTL: %llu cycles; %u SPI/UART frames equal, %u HIT, %u MISS; PLAYER %u, SW %u\n",
      (unsigned long long)ran_cycles,checked,hits,misses,players,switches);
    std::printf("fills %llu..%llu; complete reports %llu..%llu; last boundary margin %llu cycles\n",
      (unsigned long long)first_fill,(unsigned long long)last_fill,
      (unsigned long long)first_report,(unsigned long long)last_report,(unsigned long long)(ran_cycles-last_report));
    if(scenario.empty() && (checked<145 || hits<3 || misses<1)) throw std::runtime_error("attract coverage");
    if(scenario.empty() && (!line.empty() || !completed.empty())) throw std::runtime_error("end inside UART report (adjust check cycles)");
    if(scenario=="human" && (players!=1 || switches!=2)) throw std::runtime_error("human controls coverage");
    if(scenario=="p18") playtest.check(players,misses,overs);
    if(scenario=="p27") wall_playtest.check(players,switches);
    if(scenario=="p28") serve_playtest.check(players,misses,overs);
    return 0;
 } catch(const std::exception& e) {std::fprintf(stderr,"FAIL: %s\n",e.what()); return 1;}
}
