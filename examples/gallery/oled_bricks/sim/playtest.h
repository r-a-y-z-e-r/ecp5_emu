// P18: drive only physical button pins, inspect pictures decoded from SPI.
// No force/deposit, shortened frame period, or access to internal RTL state.
#pragma once
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

class WindowControls {
    enum Phase { START, SHORT, HELD, RELEASE, EDGE, LOSE, OVER, AGAIN, DONE } phase=START;
    uint64_t fire_start=2000000, fire_end=2060000;
    uint64_t left_start=UINT64_MAX, left_end=UINT64_MAX;
    uint64_t right_start=UINT64_MAX, right_end=UINT64_MAX;
    unsigned phase_frames=0;
    uint32_t fresh_crc=0, over_crc=0;
    int over_paddle=0;
    bool launch_scheduled=false;

    void expect(bool ok, const std::string& message) const {
        if(!ok) throw std::runtime_error("P18 phase "+std::to_string(phase)+": "+message);
    }
    static bool down(uint64_t c,uint64_t start,uint64_t end) { return c>=start && c<end; }
    void next(Phase p) { phase=p; phase_frames=0; }
public:
    unsigned buttons(uint64_t cycle) const {
        return 1 | (down(cycle,fire_start,fire_end) ? 2 : 0)
                 | (down(cycle,left_start,left_end) ? 32 : 0)
                 | (down(cycle,right_start,right_end) ? 64 : 0);
    }
    bool finished() const { return phase==DONE; }
    void picture(uint64_t cycle,const std::vector<uint8_t>& bytes,uint32_t crc,unsigned led) {
        int first=-1, last=-1;
        for(int x=0;x<96;x++) {
            size_t i=(58*96+x)*2;
            if(((unsigned(bytes[i])<<8)|bytes[i+1])==0x9e34) {
                if(first<0) first=x;
                last=x;
            }
        }
        expect(first>=0 && last>first,"paddle missing in decoded SPI picture");
        int paddle=(first+last+1)/2;
        bool human=led&128, waiting=led&64, alive=led&7;
        if(phase==START) {
            if(!human) return;
            expect(waiting && paddle==48,"fresh human game must wait at centre");
            fresh_crc=crc;
            // The 30,000-cycle pulse and its entire 25,000-cycle release
            // filter fit inside the next SPI fill, away from both updates.
            left_start=cycle+150000; left_end=left_start+30000;
            next(SHORT);
        } else {
            expect(human,"human game changed to attract without a restart");
            ++phase_frames;
            switch(phase) {
              case SHORT:
                expect(waiting && paddle==(phase_frames==1 ? 48 : 46),
                       "30,000-cycle LEFT must move exactly one two-pixel step");
                if(phase_frames==4) {
                    right_start=cycle+150000;
                    next(HELD);
                }
                break;
              case HELD:
                expect(waiting && paddle==46+2*int(phase_frames>1 ? phase_frames-1 : 0),
                       "held RIGHT must move two pixels on every subsequent picture");
                if(phase_frames==4) {
                    right_end=cycle+1;
                    next(RELEASE);
                }
                break;
              case RELEASE:
                // The already-observed part of the current frame earns one
                // final step. After that, released input must not repeat.
                expect(waiting && paddle==54,"RIGHT repeated after its latched frame was consumed");
                if(phase_frames==3) {
                    left_start=cycle+150000; left_end=UINT64_MAX;
                    next(EDGE);
                }
                break;
              case EDGE:
                expect(phase_frames<26,"held LEFT did not reach the edge");
                if(paddle==8) {
                    left_end=cycle+1;
                    next(LOSE);
                }
                break;
              case LOSE:
                if(!alive) {
                    expect(!waiting,"game-over must not offer a normal serve");
                    over_crc=crc; over_paddle=paddle;
                    right_start=cycle+150000; right_end=UINT64_MAX;
                    next(OVER);
                } else {
                    // Play a real game, deliberately missing from the left
                    // edge. Launch all three lives; never alter game internals.
                    if(waiting && !launch_scheduled) {
                        fire_start=cycle+150000; fire_end=fire_start+60000;
                        launch_scheduled=true;
                    }
                    if(!waiting) launch_scheduled=false;
                }
                break;
              case OVER:
                expect(!alive && !waiting && paddle==over_paddle && crc==over_crc,
                       "game-over picture moved, including with RIGHT held");
                if(phase_frames==120) {
                    right_end=cycle+1;
                    fire_start=cycle+150000; fire_end=fire_start+60000;
                    next(AGAIN);
                }
                break;
              case AGAIN:
                if(!alive) {
                    expect(paddle==over_paddle && crc==over_crc,"game-over changed before FIRE1");
                    phase_frames=0;
                } else {
                    expect(waiting && (led&7)==7 && paddle==48 && crc==fresh_crc,
                           "FIRE1 must start a fresh human game, with no stale movement");
                    if(phase_frames==3) next(DONE);
                }
                break;
              default: break;
            }
        }
    }
    void check(unsigned players,unsigned misses,unsigned overs) const {
        expect(finished() && players==2 && misses>=3 && overs==1,
               "missing short-click/held/game-over/FIRE1 coverage");
        std::puts("P18 PASS: 30,000-cycle LEFT = one step; held RIGHT = a step per frame; "
                  "120 game-over pictures frozen with RIGHT held; FIRE1 = fresh PLAYER game");
    }
};
