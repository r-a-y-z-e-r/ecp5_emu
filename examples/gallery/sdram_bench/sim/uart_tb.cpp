#include "Vsim_top.h"
#include "verilated.h"
#include "sdram_model.h"
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <iostream>
#include <string>
#include <sstream>
double sc_time_stamp() {return 0;}
int main(int argc,char**argv) {
 try {
    Verilated::commandArgs(argc,argv);
    if(argc<4) return 2;
    uint64_t cycles=std::strtoull(argv[1],nullptr,10);
    std::ifstream input(argv[2],std::ios::binary);
    std::string keys((std::istreambuf_iterator<char>(input)),{}),text;
    Vsim_top top;Sdram ram;
    std::string scenario=argc>4 ? argv[4]:"normal";
    bool fault=scenario=="fault";
    uint64_t delay=argc>5 ? std::strtoull(argv[5],nullptr,10):0;
    top.btn=1;top.model_drive=0;top.model_data=0;top.ftdi_txd=1;
    top.clk_25mhz=0;top.eval();
    bool prev=true;int wait=0,bit=0,byte=0;
    for(uint64_t cycle=0;cycle<cycles;cycle++) {
        uint64_t host_bit=(cycle>=delay ? cycle-delay:0)/217,k=host_bit/10;int b=host_bit%10;
        top.btn=1;
        if(scenario=="down" && cycle>=300000 && cycle<600000) top.btn|=1<<4;
        if(scenario=="fire" && cycle>=3000000 && cycle<3300000) top.btn|=1<<1;
        top.ftdi_txd=cycle<delay || k>=keys.size() || b==9 ? 1:b==0 ? 0:
           (static_cast<unsigned char>(keys[k])>>(b-1))&1;
        // Posedge board: consume the DQ published on the preceding SDRAM edge.
        top.clk_25mhz=1;top.eval();
        if(top.sdram_clk) throw std::runtime_error("SDRAM clock must invert board clock");
        bool tx=top.ftdi_rxd;
        if(!wait && prev && !tx) {wait=108;bit=-1;byte=0;}
        else if(wait && --wait==0) {
            if(bit==-1) {if(tx) throw std::runtime_error("UART start");bit=0;wait=217;}
            else if(bit<8) {byte|=int(tx)<<bit;bit++;wait=217;}
            else {if(!tx) throw std::runtime_error("UART stop");text+=char(byte);
                if(text.size()>=8 && text.compare(text.size()-8,8,"RESTART\n")==0) {ram.restart_test();ram.inject=fault;}}
        }
        prev=tx;
        // Falling board = rising SDRAM; inspect actual pins, then settle DQ.
        top.clk_25mhz=0;top.eval();
        if(!top.sdram_clk || !top.sdram_cke) throw std::runtime_error("SDRAM clock/CKE");
        unsigned cmd=top.sdram_csn ? 8 :
          (top.sdram_rasn<<2)|(top.sdram_casn<<1)|top.sdram_wen;
        ram.edge(cmd,top.sdram_ba,top.sdram_a,top.sdram_dqm,top.fpga_drive,top.bus_data);
        top.model_drive=ram.drive;top.model_data=ram.data;top.eval();
    }
    top.final();
    // Only the deterministic response to check.json's restart belongs to
    // expected/. The initial power-on pass can precede the host's marker.
    auto marker=text.find("RESTART\n");
    if(marker==std::string::npos) throw std::runtime_error("restart marker missing");
    std::ofstream output(argv[3],std::ios::binary);output<<text.substr(marker);output.close();
    if(text.substr(marker).find("PASS 1 SIZE 2 ")==std::string::npos)
        throw std::runtime_error("post-restart 2 MiB pass missing");
    if(fault && text.substr(marker).find("FIRST ERROR 123 ")==std::string::npos)
        throw std::runtime_error("first-error report missing");
    std::istringstream lines(text.substr(marker));std::string line;
    unsigned reports=0;
    while(std::getline(lines,line)) if(line.rfind("PASS ",0)==0) {
        ++reports;
        std::string expected=(fault && reports==1) ? " ERRORS 1":" ERRORS 0";
        if(line.size()<expected.size() || line.compare(line.size()-expected.size(),expected.size(),expected)!=0)
            throw std::runtime_error("unexpected error count: "+line);
    }
    if(!reports) throw std::runtime_error("no pass reports");
    if(fault && !(top.led&128)) throw std::runtime_error("sticky error LED missing");
    if(text.back()!='\n') throw std::runtime_error("cycle budget ended in a UART line");
    std::cout<<text.substr(marker);
    std::cout<<"RTL: "<<cycles<<" cycles, "<<ram.written<<" writes, "<<ram.checked
       <<" reads, "<<ram.refresh_count<<" refreshes; no timing violations\n";
    return 0;
 } catch(const std::exception& e) {std::cerr<<e.what()<<'\n';return 1;}
}
