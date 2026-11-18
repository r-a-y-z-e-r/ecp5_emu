// Negative tests of the independent model: a bad controller must fail with
// the timing rule named, rather than silently producing a golden transcript.
#include "sdram_model.h"
#include <functional>
#include <iostream>
Sdram initialised() {
    Sdram r;
    for(int i=0;i<2501;i++) r.edge(7,0,0,0,false,0);
    r.edge(2,0,1024,0,false,0);r.edge(7,0,0,0,false,0);
    r.edge(1,0,0,0,false,0);r.edge(7,0,0,0,false,0);
    r.edge(1,0,0,0,false,0);r.edge(7,0,0,0,false,0);
    r.edge(0,0,0x27,0,false,0);r.edge(7,0,0,0,false,0);
    return r;
}
int main() {
    auto rejects=[](const char* rule,const std::function<void()>& fn) {
        try {fn();} catch(const std::runtime_error& e) {
            if(std::string(e.what()).find(rule)!=std::string::npos) return;
            throw;
        }
        throw std::runtime_error(std::string("model failed to reject ")+rule);
    };
    rejects("init",[]{Sdram r;r.edge(2,0,1024,0,false,0);});
    rejects("tRP",[]{auto r=initialised();r.edge(2,0,1024,0,false,0);r.edge(3,0,0,0,false,0);});
    rejects("tRCD",[]{auto r=initialised();r.edge(3,0,0,0,false,0);r.edge(5,0,0,0,false,0);});
    rejects("tRAS",[]{auto r=initialised();r.edge(3,0,0,0,false,0);r.edge(2,0,0,0,false,0);});
    rejects("tRFC",[]{auto r=initialised();r.edge(1,0,0,0,false,0);r.edge(3,0,0,0,false,0);});
    rejects("tMRD",[]{auto r=initialised();r.edge(0,0,0x27,0,false,0);r.edge(3,0,0,0,false,0);});
    rejects("tRC",[]{auto r=initialised();r.banks[0].active=r.tick;r.edge(3,0,0,0,false,0);});
    rejects("tWR",[]{
        auto r=initialised();r.edge(3,0,0,0,false,0);r.edge(7,0,0,0,false,0);
        for(unsigned i=0;i<128;i++) r.edge(i==0 ? 4:7,0,0,0,true,Sdram::pattern(i,0xace1));
        r.edge(2,0,0,0,false,0);
    });
    rejects("refresh interval",[]{auto r=initialised();for(int i=0;i<196;i++) r.edge(7,0,0,0,false,0);});
    rejects("open bank",[]{auto r=initialised();r.edge(3,0,0,0,false,0);r.edge(7,0,0,0,false,0);r.edge(3,0,0,0,false,0);});
    rejects("reserved",[]{auto r=initialised();r.edge(0,0,0x28,0,false,0);});
    std::cout<<"Model negative tests: init, tRP/tRCD/tRC/tRAS/tWR/tRFC/tMRD, refresh, bank, mode rejected\n";
}
