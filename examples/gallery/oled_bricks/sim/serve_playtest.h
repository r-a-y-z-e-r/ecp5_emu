// P28: use button pins, decoded pictures, and the matching UART frame reports.
#pragma once
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

class ServeControls {
    enum Phase { START, EDGE, FIRST_SERVE, MOVE_WAITING, SETTLE, RESERVE,
                 DRAIN, OVER, NEW_GAME, DONE } phase=START;
    uint64_t fire_start=2000000, fire_end=2060000;
    unsigned movement=0, phase_frames=0, stable=0;
    unsigned saved_score=0, saved_bricks=0, previous_score=0, previous_bricks=0;
    uint32_t fresh_crc=0, over_crc=0;
    std::vector<uint8_t> fresh_layout, saved_layout, previous_layout;
    bool launched=false, resumed=false, launch_scheduled=false, check_upward=false;
    int launch_y=0;

    void expect(bool ok,const std::string& message) const {
        if(!ok) throw std::runtime_error("P28 phase "+std::to_string(phase)+": "+message);
    }
    void next(Phase p) { phase=p; phase_frames=stable=0; }
    void fire(uint64_t cycle) { fire_start=cycle+150000;fire_end=fire_start+60000; }
public:
    unsigned buttons(uint64_t cycle) const {
        return 1 | movement | (cycle>=fire_start && cycle<fire_end ? 2 : 0);
    }
    bool finished() const { return phase==DONE; }
    void picture(uint64_t cycle,const std::vector<uint8_t>& bytes,uint32_t crc,
                 unsigned led,unsigned score,unsigned lives,unsigned bricks) {
        bool human=led&128, waiting=led&64;
        if(phase==START && !human) return;
        expect(human,"human game unexpectedly returned to attract");
        expect(++phase_frames<400,"serve or game-over transition timed out");
        int first=96,last=-1,ball_l=96,ball_r=-1,ball_t=64,ball_b=-1;
        for(int y=0;y<64;y++) for(int x=0;x<96;x++) {
            size_t i=(y*96+x)*2;
            unsigned colour=(unsigned(bytes[i])<<8)|bytes[i+1];
            if(y==58 && colour==0x9e34) { if(x<first) first=x;if(x>last) last=x; }
            if(colour==0xff9c) {
                if(x<ball_l) ball_l=x;if(x>ball_r) ball_r=x;
                if(y<ball_t) ball_t=y;if(y>ball_b) ball_b=y;
            }
        }
        expect(last>=first && ball_r>=ball_l,"paddle or ball missing from SPI picture");
        int paddle=(first+last+1)/2, ball_x=(ball_l+ball_r)/2, ball_y=(ball_t+ball_b)/2;
        if(waiting) expect(ball_x==paddle && ball_y==56,
                           "waiting ball must rest on this frame's paddle, with no lag");
        // Golden, hand-drawn FIRE1 raster: visible only while waiting, never
        // confused with the moving ball's different colour after launch.
        const char* hint[5]={"1110111011001110010", "1000010010101000110",
                             "1100010011001100010", "1000010010101000010",
                             "1000111010101110111"};
        for(int y=0;y<5;y++) for(int x=0;x<19;x++) {
            size_t i=((43+y)*96+38+x)*2;
            unsigned colour=(unsigned(bytes[i])<<8)|bytes[i+1];
            expect((colour==0xeeb8)==(waiting && hint[y][x]=='1'),
                   "FIRE1 prompt missing while waiting or still visible after launch");
        }
        std::vector<uint8_t> layout(bytes.begin()+12*96*2,bytes.begin()+36*96*2);
        auto preserved = [&] {
            expect(lives==2 && score==saved_score && bricks==saved_bricks && layout==saved_layout,
                   "life-loss wait/re-serve reset score, bricks or remaining lives");
        };
        switch(phase) {
          case START:
            expect(waiting && lives==3 && score==0 && bricks==62 && paddle==48,"fresh human game");
            fresh_crc=crc; fresh_layout=layout; movement=32; next(EDGE); break;
          case EDGE:
            expect(waiting && lives==3,"ball launched without FIRE1");
            stable=paddle==8 ? stable+1 : 0;
            if(stable==3) { movement=0;fire(cycle);next(FIRST_SERVE); }
            break;
          case FIRST_SERVE:
            if(!waiting) {
                expect(lives==3,"lost a life before the first serve");
                launched=true; previous_score=score; previous_bricks=bricks; previous_layout=layout;
            } else if(lives==2) {
                expect(launched && score>0 && bricks<62,"need a real brick hit before first life loss");
                expect(score==previous_score && bricks==previous_bricks && layout==previous_layout,
                       "MISS reset score or layout");
                saved_score=score;saved_bricks=bricks;saved_layout=layout;
                movement=64;next(MOVE_WAITING);
            } else expect(lives==3 && phase_frames<6,"FIRE1 did not launch the first ball");
            break;
          case MOVE_WAITING:
            preserved();expect(waiting,"waiting ball launched from movement alone");
            if(paddle==14) { movement=0;next(SETTLE); }
            break;
          case SETTLE:
            preserved();expect(waiting,"waiting ball launched before FIRE1");
            if(phase_frames==3) { fire(cycle);next(RESERVE); }
            break;
          case RESERVE:
            preserved();
            if(!waiting) {
                resumed=true; launch_y=ball_y; check_upward=true; movement=32;next(DRAIN);
                std::printf("P28 pin diagnosis: FIRE1 after MISS launches with S%03u L%u B%02u preserved\n",
                            score,lives,bricks);
            } else expect(phase_frames<6,"FIRE1 did not re-serve the ball");
            break;
          case DRAIN:
            if(check_upward) {
                expect(!waiting && ball_y<launch_y,"re-served ball did not travel upward");
                check_upward=false;
            }
            if(!lives) {
                expect(!waiting,"last MISS must enter game-over");
                over_crc=crc;movement=64;next(OVER);
            } else {
                if(waiting && !launch_scheduled) { fire(cycle);launch_scheduled=true; }
                if(!waiting) launch_scheduled=false;
            }
            break;
          case OVER:
            expect(!lives && !waiting && crc==over_crc,"game-over moved before FIRE1");
            if(phase_frames==5) { movement=0;fire(cycle);next(NEW_GAME); }
            break;
          case NEW_GAME:
            if(!lives) expect(!waiting && crc==over_crc,"game-over changed before FIRE1");
            else {
                expect(waiting && lives==3 && score==0 && bricks==62 && layout==fresh_layout && crc==fresh_crc,
                       "FIRE1 after game-over must restore full lives and a fresh layout");
                next(DONE);
            }
            break;
          default: break;
        }
    }
    void check(unsigned players,unsigned misses,unsigned overs) const {
        expect(finished() && resumed && players==2 && misses>=3 && overs==1,"missing serve/restart coverage");
        std::puts("P28 PASS: lost life keeps score/bricks/lives; ball rests on and follows paddle; "
                  "pin FIRE1 re-serves; last loss + FIRE1 starts fresh; FIRE1 prompt appears/disappears");
    }
};
