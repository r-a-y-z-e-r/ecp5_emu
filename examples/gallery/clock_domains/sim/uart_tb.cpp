// Independent event scheduler, pad UART decoder, and content oracle.
#include "Vtop.h"
#include "Vtop___024root.h"
#include "verilated.h"
#include "verilated_fst_c.h"
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <iterator>
#include <memory>
#include <regex>
#include <stdexcept>
#include <string>
#include <vector>
double sc_time_stamp() { return 0; }
static uint32_t next_pattern(uint32_t n) {
    return (n<<1)|(((n>>31)^(n>>21)^(n>>1)^n)&1);
}
static uint32_t expected_crc(uint32_t seed, unsigned words, unsigned bits) {
    uint32_t c=~uint32_t(0);
    for(unsigned i=0;i<words;i++,seed=next_pattern(seed))
        for(unsigned b=0;b<bits;b++)
            c=(c>>1)^(((c^(seed>>b))&1) ? 0xedb88320u : 0u);
    return ~c;
}
struct Train {
    double origin,half;
    uint64_t edges=0;
    bool level=false;
    uint64_t next() const { return std::llround(origin+half*edges); }
    void advance() { level=!level; edges++; }
};
int main(int argc,char**argv) {
 try {
    if(argc<5) return 2;
    Verilated::commandArgs(argc,argv);
    Verilated::traceEverOn(true);
    const uint64_t cycles=std::stoull(argv[1]);
    std::ifstream input(argv[2],std::ios::binary);
    std::string keys((std::istreambuf_iterator<char>(input)),{}),text,line;
    const int variant=std::stoi(argv[4]);
    const bool controls=argc>6 && std::string(argv[6])=="controls";
    const double scale=variant==1 ? 1.006 : variant==2 ? .97 : 1;
    // A common PLL phase shift keeps both outputs on one 600 MHz VCO
    // grid relative to each other. The oscillator has an unrelated phase.
    const double phase=variant==1 ? 3750 : variant==2 ? 11250 : 0;
    Train board{20000,20000},c40{60000+phase,12500},c33{60000+phase,15000};
    Train osc{(variant==1 ? 7311.0 : variant==2 ? 19007.0 : 0)+1e12/(77500000*scale),
              1e12/(77500000*scale)};
    Vtop top;
    top.btn=1; top.ftdi_txd=1; top.clk_25mhz=0;
    std::unique_ptr<VerilatedFstC> trace;
    if(argc>5 && argv[5][0]) {
        trace=std::make_unique<VerilatedFstC>(); top.trace(trace.get(),8); trace->open(argv[5]);
    }
    top.eval();
    bool previous=true;
    int wait=0,bit=0,byte=0;
    uint64_t cycle=0,first=0,last=0;
    unsigned checked=0,restarts=0,frequencies=0;
    uint64_t measure_start=0,compute_start=0,compute_cycles=0;
    bool was_measuring=false;
    int was_state=0;
    bool marker=false;
    const std::regex report("PASS ([0-9A-F]{8}) FIFO ([0-9A-F]{8}) ERR ([0-9A-F]{8}) CRC ([0-9A-F]{8}) HS ([0-9A-F]{8}) ERR ([0-9A-F]{8}) CRC ([0-9A-F]{8}) PULSE ([0-9A-F]{8}) ([0-9A-F]{8})");
    while(cycle<cycles) {
        uint64_t t=std::min({board.next(),c40.next(),c33.next(),osc.next()});
        bool sample=false;
        if(t==board.next()) {
            board.advance(); top.clk_25mhz=board.level; sample=board.level;
            if(sample) {
                uint64_t host_cycle=cycle;
                std::string host_keys=keys;
                if(controls) {
                    if(cycle>=1500000) { host_cycle=cycle-1500000;host_keys="f"; }
                    else if(cycle>=500000) { host_cycle=cycle-500000;host_keys="r"; }
                    else if(cycle>=390000) { host_cycle=cycle-390000;host_keys="r"; }
                    top.btn=1 | ((cycle>=1100000 && cycle<1380000) ? 2 : 0);
                }
                uint64_t host_bit=host_cycle/217,k=host_bit/10; int b=host_bit%10;
                top.ftdi_txd=k>=host_keys.size() || b==9 ? 1 : b==0 ? 0 :
                    (static_cast<unsigned char>(host_keys[k])>>(b-1))&1;
            }
        }
        if(t==c40.next()) c40.advance();
        if(t==c33.next()) c33.advance();
        if(t==osc.next()) osc.advance();
        top.rootp->top__DOT__clocks__DOT__pll_i__DOT__drive40=c40.level;
        top.rootp->top__DOT__clocks__DOT__pll_i__DOT__drive33=c33.level;
        top.rootp->top__DOT__oscillator__DOT__drive=osc.level;
        top.eval();
        if(trace && cycle<60000) trace->dump(t);
        if(!sample) continue;
        const bool measuring=top.rootp->top__DOT__measuring;
        if(measuring && !was_measuring) measure_start=cycle;
        if(!measuring && was_measuring && controls && cycle-measure_start!=25000000)
            throw std::runtime_error("frequency window is not exactly 25000000 board cycles");
        was_measuring=measuring;
        const int state=top.rootp->top__DOT__state;
        if(state==3 && was_state!=3) compute_start=cycle;
        if(state==4 && was_state==3) compute_cycles=cycle-compute_start;
        was_state=state;
        bool tx=top.ftdi_rxd;
        if(!wait && previous && !tx) { wait=108;bit=-1;byte=0; }
        else if(wait && --wait==0) {
            if(bit==-1) { if(tx) throw std::runtime_error("UART start bit");bit=0;wait=217; }
            else if(bit<8) { byte|=int(tx)<<bit;bit++;wait=217; }
            else {
                if(!tx) throw std::runtime_error("UART stop bit");
                text+=char(byte);
                if(byte=='\n') {
                    if(line.size()>=7 && line.substr(line.size()-7)=="RESTART") {
                        marker=true;checked=0;restarts++;
                    }
                    else if(marker) {
                        std::smatch m;
                        if(controls && line.rfind("FREQ ",0)==0) {
                            unsigned a,b,c;
                            if(std::sscanf(line.c_str(),"FREQ %x %x %x",&a,&b,&c)!=3 ||
                               a!=40000000 || b<33333332 || b>33333335 ||
                               std::abs(double(c)-38750000*scale)>2)
                                throw std::runtime_error("frequency reading: "+line);
                            frequencies++;line.clear();previous=tx;cycle++;continue;
                        }
                        if(!std::regex_match(line,m,report)) throw std::runtime_error("bad report: "+line);
                        std::vector<uint32_t> v;
                        for(unsigned i=1;i<m.size();i++) v.push_back(std::stoul(m[i],nullptr,16));
                        if(v[0]!=checked+1 || v[1]!=262144 || v[2]!=0 ||
                           v[3]!=expected_crc(0x1aceb00c^v[0],262144,16) ||
                           v[4]!=4096 || v[5]!=0 || v[6]!=expected_crc(0xc001c0de^v[0],4096,32) ||
                           v[7]!=4096 || v[8]!=4096 || (top.led&128))
                            throw std::runtime_error("count/error/independent CRC failure: "+line);
                        if(!checked) first=cycle;
                        last=cycle;checked++;
                    }
                    line.clear();
                } else line+=char(byte);
            }
        }
        previous=tx;cycle++;
    }
    top.final(); if(trace) trace->close();
    std::ofstream output(argv[3],std::ios::binary);output<<text;
    std::printf("RTL variant %d: %llu board cycles, %u checked passes, report ends %llu..%llu; computation %llu cycles\n",
      variant,(unsigned long long)cycles,checked,(unsigned long long)first,(unsigned long long)last,
      (unsigned long long)compute_cycles);
    if(controls) {
        if(restarts!=4 || frequencies!=1 || checked<40 || (top.led&128))
            throw std::runtime_error("UART restart, FIRE1, or FREQ coverage");
        std::puts("Controls: restarted during report and transfer; FIRE1; exact 25000000-cycle FREQ window: PASS");
    } else if(!marker || checked!=3 || !line.empty())
        throw std::runtime_error("expected three complete reports and no partial line");
    return 0;
 } catch(const std::exception& e) { std::fprintf(stderr,"FAIL: %s\n",e.what());return 1; }
}
