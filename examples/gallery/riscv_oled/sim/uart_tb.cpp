// Independent pad decoders. No CPU bus, internal CRC, RAM or drawing shortcuts.
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
struct Picture { uint32_t crc; uint64_t first, last; std::vector<uint8_t> bytes; };
static void require(bool ok, const std::string& message) {
    if (!ok) throw std::runtime_error(message);
}
int main(int argc, char **argv) {
  try {
    if (argc<5) return 2;
    Verilated::commandArgs(argc,argv);
    const uint64_t cycles=std::strtoull(argv[1],nullptr,10);
    const std::string scenario=argc>5 ? argv[5] : "late";
    // The golden run receives r only AFTER several power-on fills have started.
    const uint64_t host_start=scenario=="early" ? 0 : 1200000;
    std::ifstream input(argv[2],std::ios::binary);
    const std::string keys((std::istreambuf_iterator<char>(input)),{});
    require(input.good(),"cannot read host input");
    Vtop top; top.btn=1; top.ftdi_txd=1; top.clk_25mhz=0; top.eval();
    const uint8_t init[]={0xae,0xa0,0x72,0xa1,0,0xa2,0,0xa4,0xa8,0x3f,0xad,0x8e,
      0xb0,0x0b,0xb1,0x31,0xb3,0xf0,0x8a,0x64,0x8b,0x78,0x8c,0x64,0xbb,0x3a,
      0xbe,0x3e,0x87,6,0x81,0x91,0x82,0x50,0x83,0x7d,0xaf};
    const uint8_t window[]={0x15,0,95,0x75,0,63};
    std::regex report("F([0-9A-F]{4}) C([0-9A-F]{8}) T([0-9]+)\\r");
    std::string text, line;
    std::ofstream timing(std::string(argv[4])+".csv");
    timing<<"epoch,frame,first_data_edge,last_data_edge,uart_end,draw_cycles\n";
    bool prev_tx=true, prev_clk=false, prev_cs=true, prev_res=false;
    uint64_t uart_start=0, last_rise=0, reset_start=0, fill_start=0, transaction_start=0;
    bool uart_active=false, filling=false;
    unsigned uart_byte=0, spi_bits=0, spi_byte=0, init_index=0, window_index=0, transaction_bytes=0;
    unsigned checked=0, power_frames=0, markers=0, init_sequences=0, aborted=0;
    unsigned total_reports=0;
    uint64_t last_fill=0, last_report=0, first_report=0, min_draw=UINT64_MAX, max_draw=0;
    std::deque<Picture> completed;
    std::vector<uint8_t> data;
    for (uint64_t cycle=0;cycle<cycles;cycle++) {
        uint64_t host_cycle=cycle>=host_start ? cycle-host_start : UINT64_MAX;
        uint64_t frame=host_cycle/2170, bit=host_cycle%2170/217;
        top.ftdi_txd=frame>=keys.size() || bit==9 ? 1 : bit==0 ? 0 :
          (uint8_t(keys[frame])>>(bit-1))&1;
        // Extra scenario verifies FIRE1 uses the same reset register path.
        top.btn=1 | ((scenario=="fire" && cycle>=host_start-250000 && cycle<host_start+150000) ? 2 : 0);
        if (scenario=="fire") top.ftdi_txd=1;
        top.clk_25mhz=0; top.eval(); top.clk_25mhz=1; top.eval();

        if (!top.oled_resn) {
            if (prev_res) {
                reset_start=cycle;
                require(completed.empty(),"reset lost a completed unreported fill");
                if (!data.empty()) aborted++;
            }
            init_index=window_index=0; filling=false; data.clear();
            spi_bits=spi_byte=0; last_rise=0;
        } else {
            if (!prev_res) require(cycle-reset_start>=25000,"OLED reset shorter than 1 ms");
            if (prev_cs && !top.oled_csn) { transaction_start=cycle; transaction_bytes=0; last_rise=0; }
            if (!top.oled_csn && !prev_clk && top.oled_clk) {
                require(!last_rise || cycle-last_rise>=4,"SPI exceeds 6.25 MHz");
                // Continuous CS# during a full frame: every bit is exactly four clocks.
                if (last_rise) require(cycle-last_rise==4,"SPI bit period is not four clocks");
                last_rise=cycle;
                if (top.oled_dc && spi_bits==0 && data.empty()) fill_start=cycle;
                spi_byte=(spi_byte<<1)|top.oled_mosi;
                if (++spi_bits==8) {
                    uint8_t byte=spi_byte; spi_bits=spi_byte=0;
                    transaction_bytes++;
                    if (top.oled_dc) {
                        require(init_index==sizeof(init) && filling,"data outside a full address window");
                        data.push_back(byte);
                        if (data.size()==12288) {
                            require(cycle-fill_start==393212,"data fill is not 393216 clocks");
                            completed.push_back({uint32_t(::crc32(0,data.data(),data.size())),fill_start,cycle,data});
                            last_fill=cycle; filling=false; window_index=0; data.clear();
                        }
                    } else if (init_index<sizeof(init)) {
                        require(byte==init[init_index],"wrong common init byte "+std::to_string(init_index));
                        if (++init_index==sizeof(init)) init_sequences++;
                    } else {
                        require(!filling && data.empty(),"window interrupted a partial fill");
                        require(byte==window[window_index],"wrong address-window byte "+std::to_string(window_index));
                        if (++window_index==sizeof(window)) filling=true;
                    }
                }
            }
            if (prev_clk && !top.oled_clk)
                require(cycle-last_rise==2,"SPI high pulse is not two clocks");
            if (!prev_cs && top.oled_csn) {
                require(spi_bits==0,"partial SPI byte");
                require(transaction_bytes==1 || transaction_bytes==12294,"wrong SPI transaction length");
                require(cycle-transaction_start==transaction_bytes*32,"SPI transaction is not 32 clocks per byte");
            }
        }
        prev_clk=top.oled_clk; prev_cs=top.oled_csn; prev_res=top.oled_resn;

        bool tx=top.ftdi_rxd;
        if (!uart_active && prev_tx && !tx) { uart_active=true; uart_start=cycle; uart_byte=0; }
        else if (uart_active) {
            uint64_t age=cycle-uart_start;
            for (unsigned b=0;b<8;b++) if (age==325+b*217) uart_byte|=unsigned(tx)<<b;
            if (age==2061) {
                require(tx,"bad UART stop bit"); uart_active=false;
                text+=char(uart_byte);
                if (uart_byte=='\n') {
                    std::smatch match;
                    if (line=="RESTART\r") {
                        require(++markers==1,"more than one RESTART");
                        require(power_frames>0,"r arrived before the power-on frames");
                        checked=0; first_report=0; min_draw=UINT64_MAX; max_draw=0;
                    } else if (std::regex_match(line,match,report)) {
                        unsigned f=std::stoul(match[1],nullptr,16);
                        uint32_t crc=std::stoul(match[2],nullptr,16);
                        uint64_t drawn=std::stoull(match[3]);
                        unsigned expected=markers ? checked : power_frames;
                        require(f==(expected&65535),"wrong frame number: "+line);
                        require(!completed.empty(),"UART reports a frame not received on SPI");
                        require(crc==completed.front().crc,"SPI zlib CRC differs from UART: "+line);
                        timing<<markers<<","<<f<<","<<completed.front().first<<","<<completed.front().last
                              <<","<<cycle<<","<<drawn<<"\n";
                        if (markers && f==32) {
                            std::ofstream raw(argv[4],std::ios::binary);
                            raw.write(reinterpret_cast<const char*>(completed.front().bytes.data()),12288);
                        }
                        completed.pop_front(); total_reports++; last_report=cycle;
                        if (markers) {
                            checked++; if (!first_report) first_report=cycle;
                            if (drawn<min_draw) min_draw=drawn;
                            if (drawn>max_draw) max_draw=drawn;
                        } else power_frames++;
                    } else require(false,"unexpected UART line: "+line);
                    line.clear();
                } else line+=char(uart_byte);
            }
        }
        prev_tx=tx;
    }
    top.final();
    std::ofstream output(argv[3],std::ios::binary); output.write(text.data(),text.size());
    std::printf("RTL %s: %llu clocks; %u checked + %u power-on frames; %u marker; %u init sequences; %u reset-aborted fills\n",
      scenario.c_str(),(unsigned long long)cycles,checked,power_frames,markers,init_sequences,aborted);
    std::printf("last fill %llu, last report %llu, boundary margin %llu; draws %llu..%llu clocks\n",
      (unsigned long long)last_fill,(unsigned long long)last_report,(unsigned long long)(cycles-last_report),
      (unsigned long long)min_draw,(unsigned long long)max_draw);
    require(markers==1 && init_sequences==2 && total_reports>checked,"reset coverage");
    require(checked>=55 && checked<=70,"expected about 60 checked frames");
    require(completed.empty() && line.empty() && !uart_active,"end inside an unreported fill/UART line; adjust check cycles");
    std::puts("PASS: init, windows, SPI timing, fill lengths, frame numbers and every zlib CRC");
    return 0;
  } catch (const std::exception& e) { std::fprintf(stderr,"FAIL: %s\n",e.what()); return 1; }
}
