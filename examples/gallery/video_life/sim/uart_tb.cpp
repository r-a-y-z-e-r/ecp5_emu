// Independent reference: simulate the unsynthesised RTL, not the bitstream.
// One iteration is one 25 MHz cycle. Host bytes are contiguous 217-cycle
// UART bits from cycle zero; the receiver samples the middle of each bit.
#include "Vtop.h"
#include "verilated.h"
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
double sc_time_stamp() { return 0; }
int main(int argc,char**argv) {
    Verilated::commandArgs(argc,argv);
    if(argc!=4) return 2;
    uint64_t cycles=std::strtoull(argv[1],nullptr,10);
    std::ifstream input(argv[2],std::ios::binary);
    std::string keys((std::istreambuf_iterator<char>(input)),{});
    FILE* output=std::fopen(argv[3],"wb");
    Vtop top;
    top.btn=1;
    bool prev=true;
    int wait=0,bit=0,byte=0;
    uint64_t decoded=0;
    for(uint64_t cycle=0;cycle<cycles;cycle++) {
        uint64_t host_bit=cycle/217, k=host_bit/10;
        int b=host_bit%10;
        top.ftdi_txd=k>=keys.size() || b==9 ? 1 : b==0 ? 0 :
            (static_cast<unsigned char>(keys[k])>>(b-1))&1;
        top.clk_25mhz=0;top.eval();
        top.clk_25mhz=1;top.eval();
        bool tx=top.ftdi_rxd;
        if(!wait && prev && !tx) {wait=108;bit=-1;byte=0;}
        else if(wait && --wait==0) {
            if(bit==-1) {if(tx) return 3; bit=0;wait=217;}
            else if(bit<8) {byte|=int(tx)<<bit;bit++;wait=217;}
            else {if(!tx) return 4;std::fputc(byte,output);decoded++;}
        }
        prev=tx;
    }
    top.final();std::fclose(output);
    std::printf("RTL UART: %llu cycles, %llu bytes\n",
        (unsigned long long)cycles,(unsigned long long)decoded);
}
