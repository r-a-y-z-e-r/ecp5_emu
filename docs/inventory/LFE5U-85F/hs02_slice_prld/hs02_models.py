"""Extended HS02 sample models. NumPy is used for transition mismatch scoring.

An async-level preload follows M while LSR is active. On simultaneous M change
and LSR release, the trace can retain old M, new M, old&new or old|new. Those
release rules describe samples, not a guaranteed hardware Boolean function.
Enable overrides diagnose the CEMUX=1 / routed-CE configuration collision.
"""
import itertools
import numpy as np
from hs02_gen import SETTINGS, SEED

ENABLES = ('pin', 'one', 'zero', 'inverse pin', 'pin OR ff27.DI')
MODELS=[]
for source,timing,priority,enable in itertools.product(('M','REGSET'),('sync','async-level','async-assert','assert-latched','async-clock-M'),('over','under'),ENABLES):
    releases=('old','new','and','or') if source=='M' and timing=='async-level' else ('old',)
    for release in releases:
        MODELS.append((source,timing,priority,enable,release))


def models_for(i):
    # The ff27.DI proxy is supported by ff03's record and routing evidence only.
    return [m for m in MODELS if m[3] != 'pin OR ff27.DI' or i == 3]


class Data:
    def __init__(self, windows):
        self.inputs=np.array([[s[0] for s in w] for w in windows],dtype=np.int8)
        self.q=np.array([[s[1] for s in w] for w in windows],dtype=np.uint32)
        self.nw,self.ns=self.q.shape
        self.latched={}

    def subset(self,indices):
        result=object.__new__(Data)
        result.inputs=self.inputs[indices];result.q=self.q[indices]
        result.nw,result.ns=result.q.shape;result.latched={}
        return result

    def assertion_latch(self,i):
        if i not in self.latched:
            inputs=self.inputs[:,:,i]
            held=np.full((self.nw,self.ns),-1,dtype=np.int8)
            value=np.full(self.nw,-1,dtype=np.int8)
            for k in range(self.ns):
                active=inputs[:,k,3].astype(bool)
                if k:
                    rising=active & (inputs[:,k-1,3]==0)
                    value=np.where(rising,inputs[:,k,1],value)
                value=np.where(active,value,-1)
                held[:,k]=value
            self.latched[i]=held
        return self.latched[i]

    def predict(self,i,model):
        source,timing,priority,enable,release=model
        a,b=self.inputs[:,:-1,i],self.inputs[:,1:,i]
        da,ma,ca,la=[a[:,:,j] for j in range(4)]
        mb,cb,lb=[b[:,:,j] for j in (1,2,3)]
        def effective(c,half):
            if enable=='one':return np.ones_like(c,dtype=bool)
            if enable=='zero':return np.zeros_like(c,dtype=bool)
            if enable=='inverse pin':return c==0
            if enable=='pin OR ff27.DI':return (c!=0) | (self.inputs[:,half,27,0]!=0)
            return c!=0
        ea,eb=effective(ca,slice(None,-1)),effective(cb,slice(1,None))
        if source=='REGSET':
            va=np.full_like(ma,int(SETTINGS[i][2]=='SET'));vb=va
        elif timing=='assert-latched':
            latch=self.assertion_latch(i);va,vb=latch[:,:-1],latch[:,1:]
        else:va,vb=ma,mb
        qa=((self.q[:,:-1]>>i)&1).astype(np.int8)
        want=np.where((la!=0) & ((priority=='over') | ea),va,
                      np.where((la==0) & ea,da,qa))
        if timing!='sync':
            allow=(lb!=0) & ((priority=='over') | eb)
            if timing=='async-assert':allow &= la==0
            value=ma if timing=='async-clock-M' and source=='M' else vb
            want=np.where(allow,value,want)
            if timing=='async-level' and source=='M' and release!='old':
                released=(la!=0) & (lb==0) & ((priority=='over') | ea)
                value={'new':mb,'and':ma&mb,'or':ma|mb}[release]
                want=np.where(released,value,want)
        return want

    def score(self,i,model):
        want=self.predict(i,model)
        actual=(self.q[:,1:]>>i)&1
        known=want>=0
        return int(((want!=actual)&known).sum()),int(known.sum())


def describe(model):
    source,timing,priority,enable,release=model
    text=f'{source}, {timing}, LSR {priority} CE, effective CE={enable}'
    if source=='M' and timing=='async-level':text+=f', release M={release}'
    return text


def best(scores):
    minimum=min(miss/total for miss,total in scores.values() if total)
    found=[m for m,(miss,total) in scores.items() if total and miss/total==minimum]
    # Prefer coverage, then ordinary pin semantics and a level model.
    order={'sync':0,'async-level':1,'async-assert':2,'assert-latched':3,'async-clock-M':4}
    found.sort(key=lambda m:(-scores[m][1],ENABLES.index(m[3]),order[m[1]],m[2]!='over',m[4],m[0]))
    return found


def analyse(lines, parser):
    windows=[parser(line) for line in lines]
    data=Data(windows)
    print(f'Unique windows={data.nw}; samples/FF={data.nw*data.ns}; transitions/FF={data.nw*(data.ns-1)}')
    print('Repeated loads are deduplicated. Group counts sum replicas, not reloads.')
    print('Search: M/REGSET; sync, async-level, async-assert, M latched at LSR rise, clocked-M async load;')
    print('        over/under CE; pin/1/0/inverse pin; release old/new/AND/OR M.')
    print('        ff03 additionally tests the record-specific CE|ff27.DI routing proxy; this is not applied to other FFs.')
    print('Nonzero best counts remain unexplained; an enable override is not a claim about valid CEMUX=1 hardware.')
    scores=[]
    for i,setting in enumerate(SETTINGS):
        sc={m:data.score(i,m) for m in models_for(i)}
        scores.append(sc)
        found=best(sc);winner=found[0];miss,total=sc[winner]
        nominal='one' if setting[3]=='1' else 'pin'
        original=[m for m in MODELS if m[3]==nominal and m[4]=='old' and m[1] in ('sync','async-level','async-assert')]
        old=min((sc[m][0],sc[m][1]) for m in original)
        print(f'ff{i:02} {"/".join(setting)}: nominal old-model best={old[0]}/{old[1]}; extended best={miss}/{total}')
        print('  '+describe(winner)+f'; tied best models={len(found)}'+(' ; NO EXACT MODEL' if miss else ''))
        source='M' if setting[0]=='PRLD' else 'REGSET'
        family=[]
        for timing in ('sync','async-level','async-assert','assert-latched','async-clock-M'):
            options={m:v for m,v in sc.items() if m[0]==source and m[1]==timing and m[3]==winner[3]}
            chosen=best(options)[0];mm,nn=options[chosen];family.append(f'{timing}={mm}/{nn}')
        print('  timing hypotheses with this effective CE: '+', '.join(family))
        if setting[:2]==('PRLD','ASYNC'):
            rel=[]
            for release in ('old','new','and','or'):
                model=('M','async-level','over',winner[3],release)
                m,n=sc[model];rel.append(f'{release}={m}/{n}')
            print('  release alternatives with this effective CE: '+', '.join(rel))
    print('\nSetting groups: common model versus sum of individually fitted replicas')
    for setting in dict.fromkeys(SETTINGS):
        ids=[i for i,s in enumerate(SETTINGS) if s==setting]
        common=[m for m in MODELS if all(m in scores[i] for i in ids)]
        grouped={m:(sum(scores[i][m][0] for i in ids),sum(scores[i][m][1] for i in ids)) for m in common}
        model=best(grouped)[0];miss,total=grouped[model]
        per=[scores[i][best(scores[i])[0]] for i in ids]
        print(f'group {"/".join(setting)} FFs={ids}: common={miss}/{total}; individual={sum(v[0] for v in per)}/{sum(v[1] for v in per)}')
        print('  '+describe(model)+(' ; NO COMMON EXACT MODEL' if miss else ''))
    print('\nEnable diagnosis on ordinary DI transitions (LSR(a)=LSR(b)=0):')
    for i in range(1,32,2):
        a,b=data.inputs[:,:-1,i],data.inputs[:,1:,i]
        qa=(data.q[:,:-1]>>i)&1;qb=(data.q[:,1:]>>i)&1
        mask=(a[:,:,3]==0)&(b[:,:,3]==0)
        want=np.where(a[:,:,2]!=0,a[:,:,0],qa)
        bad=(want!=qb)&mask
        print(f'  ff{i:02}: pin-CE DI rule {int(bad.sum())}/{int(mask.sum())}; one-CE DI rule {int(((a[:,:,0]!=qb)&mask).sum())}/{int(mask.sum())}')
    print('\nASYNC PRLD with nominal CEMUX=CE: original level-model errors by LSR transition:')
    for i,s in enumerate(SETTINGS):
        if s[0]=='PRLD' and s[1]=='ASYNC' and s[3]=='CE':
            pred=data.predict(i,('M','async-level','over','pin','old'))
            error=pred!=((data.q[:,1:]>>i)&1)
            la=data.inputs[:,:-1,i,3];lb=data.inputs[:,1:,i,3]
            print(f'  ff{i:02}: '+', '.join(f'{x}->{y}: {int((error&(la==x)&(lb==y)).sum())}' for x,y in ((0,0),(0,1),(1,0),(1,1))))
            changed=(la!=0)&(lb!=0)&(data.inputs[:,:-1,i,1]!=data.inputs[:,1:,i,1])
            bad=((data.q[:,1:]>>i)&1)!=data.inputs[:,1:,i,1]
            print(f'    M changes while LSR stays high: {int((bad&changed).sum())}/{int(changed.sum())} fail to follow new M')
    print('\nRelease rules are empirical timing outcomes for concurrent M/LSR changes, not universal asynchronous logic semantics.')
    if data.nw==1:
        print(f'First observed Q[31:0]={int(data.q[0,0]):08X}')
        if int(lines[0][1:17],16)==SEED:
            expected=sum((int(data.inputs[0,0,i,1]) if s[0]=='PRLD' else int(s[2]=='SET'))<<i for i,s in enumerate(SETTINGS))
            print(f'Configuration M-init/REGSET hypothesis={expected:08X}; bit mismatches={(expected^int(data.q[0,0])).bit_count()}/32')


def selftest(parser):
    """Synthetic serial windows from an independent scalar release-latch model."""
    from hs02_gen import SEED,traffic,step
    for release in ('old','new','and','or'):
        x=SEED; q=0; words=[]
        for _ in range(128):
            words.append(f'{q:08X}')
            aa,bb=traffic(x),traffic(step(x));nxt=0
            for i,setting in enumerate(SETTINGS):
                d,m,c,l=aa[i];_,m2,c2,l2=bb[i]
                value=(q>>i)&1
                if l:value=m
                elif c:value=d
                if l2:value=m2
                elif l:value={'old':m,'new':m2,'and':m&m2,'or':m|m2}[release]
                nxt|=value<<i
            q=nxt;x=step(x)
        line='P'+f'{SEED:016X}'+''.join(words)
        data=Data([parser(line)])
        for i in range(32):
            model=('M','async-level','over','pin',release)
            assert data.score(i,model)==(0,127),(i,model,data.score(i,model))
            candidates={m:data.score(i,m) for m in models_for(i)}
            assert model in best(candidates),(i,model,best(candidates))
    print('HS02 extended selftest: independent serialized old/new/AND/OR release traces, 32 FFs: PASS')
