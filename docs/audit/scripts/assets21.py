import re,sys,glob,tomllib,csv
sys.path.insert(0,'/tmp/audit/scripts')
import dump21
D='../ts30-decomp/ts21/server-win/'
S={}
for r in csv.reader(open(D+'strings.tsv'),delimiter='\t'):
    try: S[int(r[0],16)]=r[1]
    except: pass
src=dump21.src
def strs(name):
    m=re.search(r'void (?:__cdecl )?'+name+r'\(undefined1 \*param_1\)\s*\{(.*?)\n\}',src,re.S)
    body=m.group(1); res={}
    cur=None
    for l in body.splitlines():
        a=re.search(r'pcVar\d+ = (?:&DAT_|s_\w*_)([0-9a-f]{8});',l)
        if a: cur=int(a.group(1),16)
        b=re.search(r'\(param_1 \+ -0x([0-9a-f]+)\)\[\(int\)pcVar',l)
        if b and cur is not None:
            off=cur-int(b.group(1),16); res[off]=S.get(cur,'?%x'%cur); cur=None
    return res
t=tomllib.load(open('../rust-hlmod-the-specialists/crates/ts21/data/assets21.toml','rb'))
for a in t['asset']:
    name,v=dump21.out[a['id']]
    s=strs(name)
    body=[]
    for o in (0x120,0x122,0x124,0x126,0x128):
        x=int(v[o][1],0) if o in v else 0
        body.append(x-0x10000 if x>=0x8000 else x)
    if body!=a['body']: print(a['id'],'body',body,a['body'])
    print(a['id'],{hex(k):x for k,x in sorted(s.items())})
