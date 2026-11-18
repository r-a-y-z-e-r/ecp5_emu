// UART is decoded from its pin, video is sampled at the TMDS input.
// Independent zlib CRC + raster checks catch pixel/tag alignment mistakes.
#include "Vtop.h"
#include "verilated.h"
#include "svdpi.h"
#include <zlib.h>
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

double sc_time_stamp() { return 0; }
static uint64_t now = 0;
static std::vector<uint32_t> video_crcs;
static unsigned pixels = 0, rows = 0;
static uLong picture_crc = 0;
static std::array<unsigned char, 640*3> row{};
static bool old_de = false, old_hs = true, old_vs = true;
static uint64_t de_start = 0, previous_line = 0, hs_start = 0, vs_start = 0;
static unsigned hs_edges = 0, vs_edges = 0;

static void require(bool condition, const char* why) {
    if (!condition) throw std::runtime_error(std::string(why) +
                                            " at cycle " + std::to_string(now));
}
extern "C" void video_sample(unsigned int color, svBit de, svBit hs, svBit vs) {
    if (!hs && old_hs) { hs_start=now; ++hs_edges; }
    if (hs && !old_hs) require(now-hs_start==96, "HS width");
    if (!vs && old_vs) { vs_start=now; ++vs_edges; }
    if (vs && !old_vs) require(now-vs_start==1600, "VS width");
    if (de && !old_de) {
        if (rows) require(now-previous_line==800, "active line pitch");
        de_start=now; previous_line=now; pixels=0;
    }
    if (de) {
        require(pixels<640, "too many pixels in a line");
        row[3*pixels]=color>>16; row[3*pixels+1]=color>>8; row[3*pixels+2]=color;
        ++pixels;
    }
    if (!de && old_de) {
        require(pixels==640 && now-de_start==640, "active line width");
        picture_crc=crc32(picture_crc,row.data(),row.size());
        if (++rows==480) {
            video_crcs.push_back(uint32_t(picture_crc));
            picture_crc=0; rows=0;
        }
    }
    old_de=de; old_hs=hs; old_vs=vs;
}

struct HostByte { uint64_t cycle; unsigned char value; };
int main(int argc, char** argv) {
    try {
        Verilated::commandArgs(argc,argv);
        if (argc!=5) return 2;
        const uint64_t cycles=std::strtoull(argv[1],nullptr,10);
        const std::string mode=argv[2];
        std::vector<HostByte> host;
        std::ifstream input(argv[3],std::ios::binary);
        std::string keys((std::istreambuf_iterator<char>(input)),{});
        for (size_t i=0;i<keys.size();++i) host.push_back({2170*i,(unsigned char)keys[i]});
        if (mode=="controls") {
            // A burst queues commands; one is consumed per frame. Four
            // speed-ups include saturation (which must print no event).
            std::string burst="]]]++++pp";
            for (size_t i=0;i<burst.size();++i)
                host.push_back({4200000+2170*i,(unsigned char)burst[i]});
            host.push_back({11000000,'e'}); // back to plasma, fast speed
            host.push_back({17500000,'p'}); // pause across several frames
            host.push_back({20000000,'p'});
            host.push_back({38000000,'e'}); // negative sine AND cosine in weave
            host.push_back({46000000,'e'}); // negative frame warp in plasma
            host.push_back({50800000,'r'}); // restart while a report transmits
        }
        FILE* output=std::fopen(argv[4],"wb");
        require(output!=nullptr,"open output");
        Vtop top;
        top.btn=1;
        bool prev=true;
        int wait=0, bit=0, byte=0;
        size_t key_index=0, frame_reports=0;
        std::string line;
        unsigned markers=0, events=0;
        for (now=0;now<cycles;++now) {
            while (key_index<host.size() && now>=host[key_index].cycle+2170) ++key_index;
            top.ftdi_txd=1;
            if (key_index<host.size() && now>=host[key_index].cycle) {
                unsigned b=(now-host[key_index].cycle)/217;
                top.ftdi_txd=b==9 ? 1 : b==0 ? 0 : (host[key_index].value>>(b-1))&1;
            }
            top.btn=1;
            if (mode=="controls") {
                // All these 30,000-clock clicks finish between reads. Each
                // must survive the 25,000-clock filter and pending latch.
                if (now>=2500000 && now<2530000) top.btn|=1<<1; // FIRE1
                if (now>=3400000 && now<3430000) top.btn|=1<<5; // LEFT
                if (now>=7800000 && now<7830000) top.btn|=1<<6; // RIGHT
                if (now>=9000000 && now<9030000) top.btn|=1<<4; // DOWN
                if (now>=9800000 && now<9830000) top.btn|=1<<3; // UP
                if (now>=15000000 && now<15030000) top.btn|=1<<2; // FIRE2
                if (now>=16500000 && now<16530000) top.btn|=1<<2;
            }
            top.clk_25mhz=0; top.eval();
            top.clk_25mhz=1; top.eval();
            bool tx=top.ftdi_rxd;
            if (!wait && prev && !tx) { wait=108; bit=-1; byte=0; }
            else if (wait && --wait==0) {
                if (bit==-1) { require(!tx,"UART start"); bit=0; wait=217; }
                else if (bit<8) { byte|=int(tx)<<bit; ++bit; wait=217; }
                else {
                    require(tx,"UART stop"); std::fputc(byte,output);
                    line+=char(byte);
                    if (byte==10) {
                        unsigned n,e,p,c;
                        if (std::sscanf(line.c_str(),"F%4x E%u P%u C%8x",&n,&e,&p,&c)==4) {
                            require(frame_reports<video_crcs.size(),"report precedes picture");
                            require(c==video_crcs[frame_reports],"UART/pixel-stream CRC mismatch");
                            ++frame_reports;
                        } else if (line=="RESTART\r\n") ++markers;
                        else if (line.rfind("SET ",0)==0) ++events;
                        else require(false,"unrecognized UART line");
                        line.clear();
                    }
                }
            }
            prev=tx;
        }
        top.final(); std::fclose(output);
        require(line.empty(),"run ends in a partial UART line");
        require(frame_reports==video_crcs.size(),"completed picture missing UART report");
        require(hs_edges>500 && vs_edges>1,"missing video sync");
        require(markers==(mode=="controls" ? 2u : 1u),"unexpected RESTART count");
        if (mode=="controls") require(events==19,"lost/extra control events");
        std::printf("RTL %s: %llu cycles, %zu complete 640x480 frames; UART/pixel CRCs equal; "
                    "%u markers, %u events; sync/framing PASS\n", mode.c_str(),
                    (unsigned long long)cycles,frame_reports,markers,events);
    } catch (const std::exception& e) {
        std::fprintf(stderr,"FAIL: %s\n",e.what()); return 1;
    }
}
