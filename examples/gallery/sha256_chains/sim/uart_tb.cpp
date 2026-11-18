// Pad-level UART driver/decoder; records hardware values for Python hashlib.
#include "Vtop.h"
#include "Vtop___024root.h"
#include "verilated.h"
#include <chrono>
#include <cstdio>
#include <cstdint>
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <string>
double sc_time_stamp() { return 0; }
int main(int argc,char**argv) {
 try {
    if(argc<4) throw std::runtime_error("usage: Vtop cycles input output-prefix [controls|late]");
    Verilated::commandArgs(argc,argv);
    const auto start_time=std::chrono::steady_clock::now();
    const uint64_t limit=std::stoull(argv[1]);
    const std::string prefix=argv[3];
    const std::string scenario=argc>4 ? argv[4]:"";
    const bool controls=scenario=="controls",late=scenario=="late";
    const uint64_t late_at=200000;
    std::ifstream input(argv[2],std::ios::binary);
    const std::string keys((std::istreambuf_iterator<char>(input)),{});
    std::ofstream uart(prefix+".txt",std::ios::binary),events(prefix+".events");
    FILE* values=std::fopen((prefix+".values").c_str(),"w");
    if(!uart || !events || !values) throw std::runtime_error("cannot open simulation output");
    Vtop top;top.btn=1;top.ftdi_txd=1;top.clk_25mhz=0;top.eval();
    auto* r=top.rootp;
    const unsigned lanes=r->top__DOT__digest.size();
    bool previous=true,computing=false;
    int wait=0,bit=0,byte=0;
    uint64_t origin=0,launch=0,records=0,link_restart=0,previous_launch=0,min_period=UINT64_MAX;
    uint32_t pass=0,step=0;
    unsigned restarts=0,requests=0;
    std::string text;
    for(uint64_t cycle=0;cycle<limit;cycle++) {
        uint64_t host_cycle=cycle;
        const bool host_active=!late || cycle>=late_at;
        if(late && host_active) host_cycle=cycle-late_at;
        if(late && cycle==late_at) {
            if(text.rfind("KAT abc ",0)!=0 || text.find("\r\nKAT 448 ")==std::string::npos ||
               text.find("RESTART")!=std::string::npos)
                throw std::runtime_error("power-on must begin both KAT lines without a marker before late r");
        }
        if(controls) {
            // Interrupt a KAT byte, then actual chain rounds, then FIRE1.
            if(link_restart && cycle>=link_restart) host_cycle=cycle-link_restart;
            else if(cycle>=70000) host_cycle=cycle-70000;
            top.btn=1 | ((cycle>=750000 && cycle<1050000) ? 2:0);
        }
        uint64_t host_bit=host_cycle/217,k=host_bit/10;int b=host_bit%10;
        top.ftdi_txd=!host_active || k>=keys.size() || b==9 ? 1 : b==0 ? 0 :
            (static_cast<unsigned char>(keys[k])>>(b-1))&1;
        top.clk_25mhz=0;top.eval();
        if(r->top__DOT__restart_request) {
            requests++;
            if(controls && requests==2 && !r->top__DOT__busy)
                throw std::runtime_error("late r did not interrupt a UART byte");
            if(controls && requests==3 && (r->top__DOT__mode!=4 || r->top__DOT__state!=3))
                throw std::runtime_error("r did not interrupt chain compression rounds");
        }
        const bool launched=r->top__DOT__start && (r->top__DOT__mode==3 || r->top__DOT__mode==4);
        if(launched) {
            if(controls && !link_restart && cycle>70000 && r->top__DOT__mode==4) link_restart=cycle+50;
            if(previous_launch) {
                const uint64_t period=cycle-previous_launch;
                if(period<67) throw std::runtime_error("chain interval is shorter than 67 clocks");
                if(period<min_period) min_period=period;
            }
            previous_launch=cycle;computing=true;
            pass=r->top__DOT__pass_number;step=r->top__DOT__mode==3 ? 0:r->top__DOT__step_index;
            launch=cycle;
        }
        const unsigned old_state=r->top__DOT__state;
        top.clk_25mhz=1;top.eval();
        if(r->top__DOT__cycles==0 && cycle>=63) {
            origin=cycle;computing=false;previous_launch=0;restarts++;
            events<<"RESTART "<<restarts<<" "<<cycle<<"\n";
        } else if(cycle>=64 && r->top__DOT__cycles!=cycle-origin) {
            throw std::runtime_error("board counter did not include all UART/computation clocks");
        }
        if(computing && r->top__DOT__done==((1u<<lanes)-1)) {
            if(cycle-launch!=65) throw std::runtime_error("compression latency is not 66 edges");
            for(unsigned lane=0;lane<lanes;lane++) {
                std::fprintf(values,"%u %08x %08x %02x ",restarts,pass,step,lane);
                for(int w=7;w>=0;w--) std::fprintf(values,"%08x",r->top__DOT__digest[lane][w]);
                std::fputc('\n',values);records++;
            }
            computing=false;
        }
        if(old_state==3 && r->top__DOT__state==0 && r->top__DOT__line_kind==4) {
            if(r->top__DOT__report_cycles!=cycle-origin-1)
                throw std::runtime_error("CHAIN cycle snapshot differs from measured edges");
            events<<"CHAIN "<<restarts<<" "<<r->top__DOT__pass_number<<" "<<cycle-origin-1<<"\n";
        }
        if((top.led&127)!=(r->top__DOT__pass_number&127) ||
           bool(top.led&128)!=bool(r->top__DOT__step_index&1024))
            throw std::runtime_error("LEDs differ from pass/activity state");
        bool tx=top.ftdi_rxd;
        if(!wait && previous && !tx) {wait=108;bit=-1;byte=0;}
        else if(wait && --wait==0) {
            if(bit==-1) {if(tx) throw std::runtime_error("bad UART start bit");bit=0;wait=217;}
            else if(bit<8) {byte|=int(tx)<<bit;bit++;wait=217;}
            else {
                if(!tx) throw std::runtime_error("bad UART stop bit");
                text+=char(byte);
            }
        }
        previous=tx;
    }
    top.final();std::fclose(values);uart<<text;
    if(controls && requests!=4) throw std::runtime_error("restart request coverage");
    if(min_period!=67) throw std::runtime_error("no 67-clock chain interval observed");
    events<<"END "<<restarts<<" "<<r->top__DOT__pass_number<<" "<<r->top__DOT__step_index<<"\n";
    if(text.empty() || text.back()!='\n') throw std::runtime_error("window ends in an incomplete UART line");
    double seconds=std::chrono::duration<double>(std::chrono::steady_clock::now()-start_time).count();
    std::printf("Verilator %s: %llu cycles, %zu bytes, %llu values, %u restarts; %.3f s\n",
      controls ? "controls":late ? "late":"nominal",(unsigned long long)limit,text.size(),
      (unsigned long long)records,restarts,seconds);
    return 0;
 } catch(const std::exception& e) {std::fprintf(stderr,"FAIL: %s\n",e.what());return 1;}
}
