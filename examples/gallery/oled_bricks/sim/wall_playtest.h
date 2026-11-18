// P27: physical buttons and SW2, checked against independently decoded SPI.
#pragma once
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

class WallControls {
    enum Phase { START, NLEFT, RELEASE_NLEFT, WLEFT_CLAMP, WRIGHT, RELEASE_WRIGHT,
                 NARROW, NRIGHT, RELEASE_NRIGHT, WRIGHT_CLAMP, WLEFT, DONE } phase=START;
    unsigned movement=0, sw=0, phase_frames=0, stable_frames=0;
    void expect(bool ok,const std::string& message) const {
        if(!ok) throw std::runtime_error("P27 phase "+std::to_string(phase)+": "+message);
    }
    void next(Phase p) { phase=p; phase_frames=stable_frames=0; }
    bool settled(bool at) { stable_frames=at ? stable_frames+1 : 0; return stable_frames==3; }
public:
    unsigned buttons(uint64_t cycle) const {
        return 1 | movement | (cycle>=2000000 && cycle<2060000 ? 2 : 0);
    }
    unsigned switches() const { return sw; }
    bool finished() const { return phase==DONE; }
    void picture(const std::vector<uint8_t>& bytes,unsigned led) {
        if(phase==START && !(led&128)) return;
        expect((led&0xc7)==0xc7,"must remain in a human game with the ball waiting");
        int first=-1,last=-1,count=0;
        for(int x=0;x<96;x++) {
            size_t i=(58*96+x)*2;
            if(((unsigned(bytes[i])<<8)|bytes[i+1])==0x9e34) {
                if(first<0) first=x;
                last=x; count++;
            }
        }
        expect(count==16 || count==24,"paddle must be completely visible at its current width");
        expect(last-first+1==count,"paddle pixels must be contiguous");
        int centre=(first+last+1)/2;
        expect(centre%2==0,"two-pixel steps or width changes lost endpoint parity");
        expect(++phase_frames<48,"button hold or width change did not reach its limit");
        switch(phase) {
          case START:
            expect(centre==48 && count==16,"fresh normal paddle");
            movement=32; next(NLEFT); break;
          case NLEFT:
            expect(count==16 && centre>=8,"normal LEFT crossed the wall");
            if(settled(centre==8)) {
                expect(first==0 && last==15,"normal LEFT must touch column 0");
                movement=0; next(RELEASE_NLEFT);
            }
            break;
          case RELEASE_NLEFT:
            expect(centre==8 && count==16,"normal paddle drifted after release");
            if(phase_frames==2) { sw=2; next(WLEFT_CLAMP); }
            break;
          case WLEFT_CLAMP:
            if(count==24) {
                expect(centre==12 && first==0 && last==23,"SW2 must re-clamp 8 to 12 without movement");
                if(settled(true)) { movement=64; next(WRIGHT); }
            } else expect(centre==8,"normal LEFT changed before SW2 settled");
            break;
          case WRIGHT:
            expect(count==24 && centre<=84,"wide RIGHT crossed the wall");
            if(settled(centre==84)) {
                expect(first==72 && last==95,"wide RIGHT must touch column 95");
                movement=0; next(RELEASE_WRIGHT);
            }
            break;
          case RELEASE_WRIGHT:
            expect(centre==84 && count==24,"wide paddle drifted after release");
            if(phase_frames==2) { sw=0; next(NARROW); }
            break;
          case NARROW:
            expect(centre==84,"narrowing a legal centre must not move it");
            if(count==16 && settled(true)) { movement=64; next(NRIGHT); }
            break;
          case NRIGHT:
            expect(count==16 && centre<=88,"normal RIGHT crossed the wall");
            if(settled(centre==88)) {
                expect(first==80 && last==95,"normal RIGHT must touch column 95");
                movement=0; next(RELEASE_NRIGHT);
            }
            break;
          case RELEASE_NRIGHT:
            expect(centre==88 && count==16,"normal paddle drifted after release");
            if(phase_frames==2) { sw=2; next(WRIGHT_CLAMP); }
            break;
          case WRIGHT_CLAMP:
            if(count==24) {
                expect(centre==84 && first==72 && last==95,"SW2 must re-clamp 88 to 84 without movement");
                if(settled(true)) { movement=32; next(WLEFT); }
            } else expect(centre==88,"normal RIGHT changed before SW2 settled");
            break;
          case WLEFT:
            expect(count==24 && centre>=12,"wide LEFT crossed the wall");
            if(settled(centre==12)) {
                expect(first==0 && last==23,"wide LEFT must touch column 0");
                movement=0; next(DONE);
            }
            break;
          default: break;
        }
    }
    void check(unsigned players,unsigned switches) const {
        expect(finished() && players==1 && switches==3,"missing wall/width-change coverage");
        std::puts("P27 pins/SPI PASS: normal centres 8/88, wide 12/84; both widths touch columns 0/95; "
                  "SW2 re-clamps both ends with movement released");
    }
};
