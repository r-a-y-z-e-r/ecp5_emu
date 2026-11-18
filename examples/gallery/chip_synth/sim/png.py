#!/usr/bin/env python3
"""Two reproducible RGB PNGs, drawn with Python struct/zlib; no PIL or downloads."""
from pathlib import Path
import struct
import sys
import zlib

def png(path, rows):
    height, width = len(rows), len(rows[0])//3
    def chunk(kind, body):
        return struct.pack('>I',len(body))+kind+body+struct.pack('>I',zlib.crc32(kind+body))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,2,0,0,0))+
                     chunk(b'IDAT',zlib.compress(b''.join(b'\0'+bytes(r) for r in rows),9))+chunk(b'IEND',b''))

def main():
    out, folder = map(Path,sys.argv[1:])
    raw=(out/'base.frame2.rgb565').read_bytes()
    assert len(raw)==12288
    rows=[]
    for y in range(64):
        row=bytearray()
        for x in range(96):
            p=int.from_bytes(raw[(y*96+x)*2:(y*96+x+1)*2],'big')
            r,g,b=p>>11,(p>>5)&63,p&31
            row.extend(bytes(((r<<3)|(r>>2),(g<<2)|(g>>4),(b<<3)|(b>>2)))*4)
        rows.extend([row]*4)
    png(folder/'scope.png',rows)
    # First 20ms = 500 sample pairs. Two vertically separated step plots.
    samples=[]
    for line in (out/'base.samples').read_text().splitlines():
        if line.startswith('RESTART'): samples=[]
        else: samples.append(tuple(map(int,line.split()))[1:])
    samples=samples[:500]
    w,h=800,360
    canvas=[bytearray((9,16,27)*w) for _ in range(h)]
    def put(x,y,c):
        if 0<=x<w and 0<=y<h: canvas[y][x*3:x*3+3]=bytes(c)
    def line(x1,y1,x2,y2,c):
        n=max(abs(x2-x1),abs(y2-y1),1)
        for i in range(n+1):put(x1+(x2-x1)*i//n,y1+(y2-y1)*i//n,c)
    # Original tiny numeric labels, milliseconds and DAC codes.
    font={'0':['111','101','101','101','111'],'1':['010','110','010','010','111'],
          '2':['110','001','111','100','111'],'4':['101','101','111','001','001'],
          '5':['111','100','110','001','110'],'8':['111','101','111','101','111'],
          'L':['100','100','100','100','111'],'R':['110','101','110','101','101'],
          'm':['000','111','111','101','101'],'s':['011','100','010','001','110']}
    def label(x,y,text,c=(215,220,230)):
        for char in text:
            for j,row in enumerate(font[char]):
                for i,b in enumerate(row):
                    if b=='1':
                        for yy in range(2):
                            for xx in range(2):put(x+i*2+xx,y+j*2+yy,c)
            x+=8
    for channel,color in [(0,(182,245,121)),(1,(250,164,137))]:
        base=145+channel*165
        label(12,base-73,'L' if channel==0 else 'R',color)
        for code in [4,8,12]:
            yy=base-(code-4)*14
            label(24,yy-5,str(code))
            line(50,yy,770,yy,(40,54,67))
        for ms in [0,5,10,15,20]:
            xx=50+ms*36
            line(xx,base-112,xx,base,(31,44,57))
            if channel==1:label(xx-4,base+15,str(ms))
        previous=None
        for i,pair in enumerate(samples):
            xx=50+i*720//499; yy=base-(pair[channel]-4)*14
            if previous:
                px,py=previous;line(px,py,xx,py,color);line(xx,py,xx,yy,color)
            previous=xx,yy
    label(745,342,'ms')
    png(folder/'wave.png',canvas)
    print('scope.png (384x256), wave.png (800x360), stdlib PNG: PASS')

if __name__=='__main__':main()
