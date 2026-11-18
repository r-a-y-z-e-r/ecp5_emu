// Independent SDR SDRAM behavioural model. No emulator code or libraries.
// Commands sampled only at rising SDRAM_CLK. DQ is published a half-board
// clock before capture: a CL2 READ at n supplies word 0 after n+1.
#pragma once
#include <algorithm>
#include <array>
#include <cstdint>
#include <deque>
#include <stdexcept>
#include <string>
#include <vector>
struct Sdram {
    static constexpr int64_t never=-10000000;
    struct Bank { int row=-1; int64_t active=never, pre=never, written=never; };
    struct Burst { bool on=false; unsigned bank=0,row=0,col=0,beat=0;
        int64_t first=0,stop=INT64_MAX; } read,write;
    std::array<Bank,4> banks;
    std::vector<uint16_t> memory=std::vector<uint16_t>(1u<<24);
    std::deque<int64_t> refreshes;
    int64_t tick=0,last_ref=never,last_mode=never,mode_at=never;
    unsigned stage=0,cl=2,length=512;
    uint64_t written=0,checked=0,refresh_count=0;
    bool drive=false,inject=false,injected=false;
    bool writing_phase=true,seen_write=false,reseed=false;
    uint16_t pattern_seed=0xace1;
    unsigned write_index=0,read_index=0,completed_small=0,completed_full=0;
    void restart_test() {reseed=true;}
    static uint16_t pattern(unsigned addr,uint32_t seed) {
        return uint16_t(addr ^ ((addr&255)<<8) ^ (addr>>16) ^ seed);
    }
    uint16_t data=0;
    void need(bool ok,const std::string& rule) const {
        if(!ok) throw std::runtime_error("SDRAM edge "+std::to_string(tick)+": "+rule);
    }
    void gap(int64_t since,int clocks,const char* name) const {need(tick-since>=clocks,name);}
    unsigned index(const Burst& b) const {
        unsigned col=length==512 ? ((b.col+b.beat)&511) :
          ((b.col&~(length-1))|((b.col+b.beat)&(length-1)));
        return (b.bank<<22)|(b.row<<9)|col;
    }
    void edge(unsigned cmd,unsigned bank,unsigned a,unsigned dqm,bool oe,uint16_t dq) {
        ++tick;
        need(dqm==0,"DQM must stay zero");
        if(stage==4) {
            need(tick-last_ref<=195,"refresh interval exceeds 7.8 us");
            while(!refreshes.empty() && refreshes.front()<=tick-1600000) refreshes.pop_front();
            if(tick-mode_at>=1600000) need(refreshes.size()>=8192,"8192 refreshes / rolling 64 ms");
        }
        if(cmd!=7 && cmd!=8) {gap(last_ref,2,"tRFC (<2 clocks)");gap(last_mode,2,"tMRD (<2 clocks)");}
        if(stage<4 && cmd!=7 && cmd!=8) {
            if(stage==0) need(tick>=2500 && cmd==2 && (a&1024),"init: 100 us, PRECHARGE ALL");
            else if(stage<3) need(cmd==1,"init: two AUTO REFRESH commands");
            else need(cmd==0,"init: LOAD MODE");
            ++stage;
        }
        auto& b=banks[bank];
        if(cmd==3) { // ACTIVE
            need(b.row<0,"ACTIVE on open bank");gap(b.pre,2,"tRP");gap(b.active,2,"tRC");
            b.row=a&8191;b.active=tick;b.written=never;
        } else if(cmd==4 || cmd==5) { // WRITE / READ
            need(stage==4 && b.row>=0,"column command on closed/uninitialised bank");
            gap(b.active,2,"tRCD");need(!(a&1024),"auto precharge unused by this controller");
            Burst burst;burst.on=true;burst.bank=bank;burst.row=b.row;burst.col=a&511;
            if(cmd==4) {
                need(!read.on,"WRITE overlaps read pipeline");
                if(!writing_phase || !seen_write) {
                    if(seen_write) {
                        need(read_index==write_index,"read phase skipped halfwords");
                        if(write_index==(1u<<20)) ++completed_small;else ++completed_full;
                    }
                    if(reseed) {pattern_seed=0xace1;reseed=false;}
                    else if(seen_write) pattern_seed=(pattern_seed<<1)|
                      (((pattern_seed>>15)^(pattern_seed>>13)^(pattern_seed>>12)^(pattern_seed>>10))&1);
                    write_index=0;read_index=0;writing_phase=true;seen_write=true;
                }
                write=burst;
            }
            else {
                need(!write.on,"READ overlaps write burst");
                if(writing_phase) {
                    need(write_index==(1u<<20) || write_index==(1u<<24),"wrong region size");
                    writing_phase=false;read_index=0;
                }
                read=burst;read.first=tick+cl-1;
            }
        } else if(cmd==6 || cmd==2) { // BURST TERMINATE / PRECHARGE
            if(write.on) {need(write.beat==128,"write burst must contain 128 beats");write.on=false;}
            if(read.on) read.stop=std::min(read.stop,tick+cl-1);
            if(cmd==2) for(unsigned i=0;i<4;i++) if((a&1024)||i==bank) {
                auto& k=banks[i];
                if(k.row>=0) {gap(k.active,2,"tRAS");gap(k.written,2,"tWR");}
                k.row=-1;k.pre=tick;
            }
        } else if(cmd==1 || cmd==0) {
            for(auto& k:banks) {need(k.row<0,"REFRESH/MODE with open bank");gap(k.pre,2,"tRP before REFRESH/MODE");}
            if(cmd==1) {last_ref=tick;refreshes.push_back(tick);++refresh_count;}
            else {
                need((a&~0x37u)==0 && !(a&8),"reserved / interleaved mode");
                cl=(a>>4)&3;need(cl==2 || cl==3,"CAS latency");
                need((a&7)==7 || (a&7)==3,"model supports full page or BL8");
                length=(a&7)==7 ? 512:8;last_mode=tick;mode_at=tick;
            }
        }
        if(write.on) {
            need(oe,"FPGA released DQ during write burst");
            need(index(write)==write_index++,"write skipped/repeated/address order");
            need(dq==pattern(index(write),pattern_seed),"wrong address/pass pattern on write");
            memory[index(write)]=dq;banks[write.bank].written=tick;++write.beat;++written;
            if(length!=512 && write.beat==length) write.on=false;
        } else need(!oe,"FPGA drives DQ outside a write burst");
        drive=false;
        if(read.on && tick>=read.stop) {
            need(read.beat==128,"read burst must contain 128 beats");read.on=false;
        }
        if(read.on && tick>=read.first) {
            need(!oe,"DQ contention");
            need(index(read)==read_index++,"read skipped/repeated/address order");
            data=memory[index(read)];drive=true;
            if(inject && !injected && index(read)==123) {data^=1;injected=true;}
            ++read.beat;++checked;
            if(length!=512 && read.beat==length) read.on=false;
        }
    }
};
