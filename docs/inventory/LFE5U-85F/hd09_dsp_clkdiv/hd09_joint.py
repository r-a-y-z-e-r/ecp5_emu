"""Joint C/opcode and output-stage timing hypotheses for HD09 board records.
The output stage's rise+fall model is measured independently by HD09m. Internal
states start unknown per window; unpredicted samples and their counts are shown.
All simultaneous stages use old stage outputs. Async resets settle before reads.
"""
from hd09_timing import CLOCKS, Clock, settle, capture


def score_c(windows, a, name, bit, data, clock, bypass=False, initial=None):
    sums = [0, 0, 0]
    phases = []
    for w in windows:
        choices = []
        for phase in range(clock.period):
            q, state = initial, phase
            miss = checked = 0
            for x, y in zip(w, w[1:]):
                r = x[name][bit]
                gsr = bool(x['gsr'] or y['gsr'])
                for half, s in enumerate((x,y)):
                    q = settle(q, s, 0, 0, a.C_ASYNC[name], gsr)
                    r = settle(r, s, 0, 0, a.C_ASYNC[name], gsr)
                    d = s['c'][data] if bypass else q
                    r = capture(r, d, s, 0, 0, True, gsr)
                    q = capture(q, s['c'][data], s, 0, 0, clock.active(state,half), gsr)
                state = clock.advance(state, x)
                if r is not None:
                    miss += r != y[name][bit]
                    checked += 1
            choices.append((miss, checked, phase))
        m,n,p = min(choices,key=lambda x:(x[0]/max(1,x[1]),-x[1],x[2]))
        sums[0]+=m; sums[1]+=n; sums[2]+=len(w)-1-n; phases.append(p)
    return (*sums, phases)


def opcode_stages(a, name):
    if name in a.COMPOSITE:
        return a.composite(name, [(1,None)]*len(a.COMPOSITE[name]))
    mask,ce,rst = {'o1':(a.IN,1,2), 'o2':(a.OP0,2,1), 'o3':(a.OP1,3,3), 'o5':(a.OP0,1,3), 'o6':(a.OP1,2,2)}[name]
    return [(mask,[(1,None,ce,rst,'async' if a.O_ASYNC[name] else 'sync',True)])]


def score_opcode(windows, a, name, clock, bypass=False, initial=None):
    stages = [] if bypass else opcode_stages(a,name)
    sums=[0,0,0]; phases=[]
    for w in windows:
        choices=[]
        for phase in range(clock.period):
            qs=[[initial]*len(chain) for _,chain in stages]
            state=phase; miss=checked=0
            for x,y in zip(w,w[1:]):
                r=x[name]
                gsr=bool(x['gsr'] or y['gsr'])
                for half,s in enumerate((x,y)):
                    for (_,chain),q in zip(stages,qs):
                        for j,model in enumerate(chain):
                            q[j]=settle(q[j],s,model[2],model[3],model[4]=='async',gsr)
                    r=settle(r,s,0,0,a.O_ASYNC[name],gsr)
                    op=s['op']; known=True
                    for (mask,_),q in zip(stages,qs):
                        if q[-1] is None:known=False
                        else:op=op&~mask|q[-1]&mask
                    d=a.result(op,r) if known else None
                    r=capture(r,d,s,0,0,True,gsr)
                    for (mask,chain),q in zip(stages,qs):
                        inputs=[s['op']&mask]+q[:-1]
                        for j,model in enumerate(chain):
                            q[j]=capture(q[j],inputs[j],s,model[2],model[3],clock.active(state,half),gsr)
                state=clock.advance(state,x)
                if r is not None:
                    miss+=r!=y[name];checked+=1
            choices.append((miss,checked,phase))
        m,n,p=min(choices,key=lambda x:(x[0]/max(1,x[1]),-x[1],x[2]))
        sums[0]+=m;sums[1]+=n;sums[2]+=len(w)-1-n;phases.append(p)
    return (*sums,phases)


def analyse(lines, a):
    windows=[a.samples(line) for line in lines]
    print(f'Unique windows={len(windows)}; samples={len(windows)*128}; available transitions={len(windows)*127}')
    print('Output R: measured rise+fall from HD09m, CE0/RST0/GSR, RESETMODE respected.')
    print('Internal stages: HD08c masks/controls; rising/falling/rise+fall, /2,/4 all phases; never; bypass.')
    print('Hidden state starts unknown each window. Unknown predictions are excluded and counted, never treated as matches.')
    print('Multi-stage opcode tests apply the same candidate clock to all active opcode stages.')
    if len(windows)==1:
        print('First observed outputs:', {n: f'{windows[0][0][n]:04X}' for n in a.OPCODE})
    summary={'C':[0,0,0], 'bypass':[0,0,0], 'opcode':[0,0,0]}
    for name,bits in a.C_ALUS:
        for bit,data in zip(bits,a.C_DATA[name]):
            bypass=name=='c4' or (name=='c3' and bit>=27)
            print(f'\n{name} R[{bit}] from c[{data}]: '+('configured bypass' if bypass else 'C register'))
            for clock in (CLOCKS if not bypass else [Clock('rise')]):
                miss,n,unknown,phases=score_c(windows,a,name,bit,data,clock,bypass)
                if clock==Clock('rise'):
                    key='bypass' if bypass else 'C'
                    for j,v in enumerate((miss,n,unknown)):summary[key][j]+=v
                print(f'  {"bypass" if bypass else clock.label()}: {miss}/{n} mismatches; unknown={unknown}'
                      +(f'; phase per sorted window={phases}' if clock.period>1 else ''))
            if not bypass:
                miss,n,unknown,_=score_c(windows,a,name,bit,data,Clock('rise'),True)
                print(f'  combinational bypass ahead of R: {miss}/{n}; unknown={unknown}')
    for name in a.OPCODE:
        print(f'\n{name} opcode register(s):')
        for clock in CLOCKS:
            miss,n,unknown,phases=score_opcode(windows,a,name,clock)
            if clock==Clock('both'):
                for j,v in enumerate((miss,n,unknown)):summary['opcode'][j]+=v
            print(f'  {clock.label()}: {miss}/{n} word mismatches; unknown={unknown}'
                  +(f'; phase per sorted window={phases}' if clock.period>1 else ''))
        miss,n,unknown,_=score_opcode(windows,a,name,Clock('rise'),True)
        print(f'  opcode bypass ahead of R: {miss}/{n}; unknown={unknown}')
    print('\nPreferred timing summaries:')
    for kind,(miss,n,unknown) in summary.items():
        print(f'  {kind}: {miss}/{n}; unknown={unknown}; '+('EXACT ON PREDICTED SAMPLES' if miss==0 and n else 'NO EXACT MODEL'))
    if len(windows)==1:
        cm=cn=om=on=0
        for name,bits in a.C_ALUS:
            for bit,data in zip(bits,a.C_DATA[name]):
                bypass=name=='c4' or (name=='c3' and bit>=27)
                m,n,_,_=score_c(windows,a,name,bit,data,Clock('rise'),bypass,initial=0)
                cm+=m;cn+=n
        for name in a.OPCODE:
            m,n,_,_=score_opcode(windows,a,name,Clock('both'),initial=0)
            om+=m;on+=n
        print(f'  Single-window zero-hidden-state hypothesis: C/bypass {cm}/{cn}; opcode {om}/{on}; no unknown predictions')
    print('Physical edge sensitivity versus phase transparency is not uniquely identified. Flags and IN_1 remain unmeasured.')


def selftest(a):
    """Independent two-stage trace exercises output half-cycle ordering and async reset."""
    import random
    rng=random.Random(90923)
    w=[]
    for _ in range(128):
        s=dict(ce=rng.randrange(16),rst=rng.randrange(16) if rng.randrange(4)==0 else 0,
               gsr=int(rng.randrange(32)==0),op=0x43e,c={0:rng.randrange(2)})
        s['c1']={0:0};w.append(s)
    q=r=0
    for k,s in enumerate(w):
        s['c1'][0]=r
        if k==127:break
        b=w[k+1]
        g=s['gsr'] or b['gsr']
        if g or s['rst']&1:r=0
        elif s['ce']&1:r=q
        if g or s['rst']&1:q=0
        elif s['ce']&1:q=s['c'][0]
        if g or b['rst']&1:r=0
        elif b['ce']&1:r=q
    chunks=[]
    for item in w:
        bits=f'{item["ce"]:04b}{item["rst"]:04b}000{1-item["gsr"]}0{item["op"]:011b}'
        bits+=''.join(str(item['c'].get(b,0)) for b in (50,30,20,0,14,8,4))+'0'
        bits+=''.join(str(item.get(name,{}).get(b,0)) for name,bs in a.C_ALUS for b in bs)
        bits+='0'*112
        chunks.append(f'{int(bits,2):040X}')
    w=a.samples('M'+''.join(chunks))
    result=score_c([w],a,'c1',0,0,Clock('rise'))
    assert result[0]==0,result
    assert score_c([w],a,'c1',0,0,Clock('rise'),initial=0)[:2]==(0,127)
    assert score_c([w],a,'c1',0,0,Clock('both'))[0]>0
    print('HD09 joint selftest: independent rising-C/rise+fall-output trace and wrong-stage rejection: PASS')
