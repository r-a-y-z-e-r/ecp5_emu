// Directed game tests cover edge serves a pin-level player cannot prescribe.
#include "Vgame_tb.h"
#include "verilated.h"
#include <algorithm>
#include <cstdio>
#include <stdexcept>
#include <string>
double sc_time_stamp() { return 0; }
static void tick(Vgame_tb& t) { t.clk=0;t.eval();t.clk=1;t.eval(); }
static void require(bool ok,const std::string& message) {
    if(!ok) throw std::runtime_error(message);
}
static void deposit(Vgame_tb& t,int x,int centre,bool wide,bool waiting=false,int vx=0) {
    t.step=0; t.options=wide ? 2 : 0; t.inject=1;
    t.test_x=x; t.test_y=56*16; t.test_vx=vx; t.test_vy=18;
    t.test_paddle=centre; t.test_waiting=waiting;
    tick(t); t.inject=0;
}
static void advance(Vgame_tb& t) {
    t.step=1;tick(t);t.step=0;
    for(int i=0;!t.done && i<100;i++) tick(t);
    require(t.done,"game update did not finish");
}
static void serve(Vgame_tb& t,int x,int centre,bool wide,bool caught=true,int vx=0) {
    deposit(t,x,centre,wide,false,vx);advance(t);
    require(t.human && t.lives==3 && !t.waiting && !(t.events&2),"unexpected life loss or serve reset");
    require(bool(t.velocity_y&128)==caught,"catch interval mismatch at Q8.4 x="+std::to_string(x));
    require(t.bx>=16 && t.bx<=1504,"wall reflection changed the legal ball-centre field");
    if(caught) {
        require(t.by==57*16,"catch must place the ball on the paddle crossing");
        advance(t);
        require(t.by<57*16,"caught ball did not travel upward on the next update");
    }
}
int main(int argc,char** argv) {
 try {
    Verilated::commandArgs(argc,argv);
    Vgame_tb t;
    t.reset=1;tick(t);t.reset=0;t.player_start=1;tick(t);t.player_start=0;
    unsigned serves=0;
    for(bool wide : {false,true}) {
        int half=wide ? 12 : 8, limit=96-half;
        // The leftmost and rightmost wall serves, at matching paddle limits.
        serve(t,1*16,half,wide);serve(t,94*16,limit,wide);serves+=2;
        // Every legal integer column has a reachable, even-centred paddle catch.
        for(int x=1;x<=94;x++) {
            int centre=std::clamp(x&~1,half,limit);
            serve(t,x*16,centre,wide);serves++;
        }
        // Catch edges agree with renderer [centre-half, centre+half), down
        // to one sixteenth of a pixel: no invisible extra column on the right.
        int l=(48-half)*16, r=(48+half)*16;
        serve(t,l,48,wide);serve(t,l-1,48,wide,false);
        serve(t,r-1,48,wide);serve(t,r,48,wide,false);serves+=4;
        // Crossing a side wall at the paddle row still reflects and catches.
        serve(t,16,half,wide,true,-8);serve(t,1504,limit,wide,true,8);serves+=2;
        // Saturation reaches the exact limits even from an odd injected centre.
        t.left=1;deposit(t,48*16,half+1,wide,true);advance(t);
        require(t.paddle==half,"left step skipped its limit");t.left=0;
        t.right=1;deposit(t,48*16,limit-1,wide,true);advance(t);
        require(t.paddle==limit,"right step skipped its limit");t.right=0;
    }
    t.final();
    std::printf("P27 game unit PASS: %u downward serves (all 1..94 columns, both widths, "
                "wall reflections and fractional catch edges); odd-centre step saturation\n",serves);
    return 0;
 } catch(const std::exception& e) { std::fprintf(stderr,"P27 game FAIL: %s\n",e.what()); return 1; }
}
