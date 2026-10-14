import re, struct, sys, glob
D='../ts30-decomp/ts21/server-win/decomp/'
src=''.join(open(f).read() for f in glob.glob(D+'*.c'))
cw=re.search(r'void FUN_10049890\(void\)\s*\{(.*?)\n\}',src,re.S).group(1)
calls=re.findall(r'(Write_\w+|FUN_10002e50)\(&DAT_([0-9a-f]+)\)',cw)
def f32(v): return struct.unpack('<f',struct.pack('<I',v&0xffffffff))[0]
def parse(name):
    m=re.search(r'void (?:__cdecl )?'+name+r'\(undefined1 \*param_1\)\s*\{(.*?)\n\}',src,re.S)
    body=m.group(1); vals={}
    for ty,off,v in re.findall(r'\*\((undefined4|undefined2|float|int) \*\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) = (0x[0-9a-f]+|-?[\d.]+);',body):
        vals[int(off,0)]=(ty,v)
    for off,v in re.findall(r'param_1\[(0x[0-9a-f]+|\d+)\] = (0x[0-9a-f]+|-?\d+|\'.\');',body):
        vals[int(off,0)]=('b',v)
    m0=re.search(r'\n  \*param_1 = (\w+);',body)
    if m0: vals[0]=('b',m0.group(1))
    return vals
out={}
for name,addr in calls:
    wid=(int(addr,16)-0x100bbea8)//0x3f4
    out[wid]=(name,parse(name))
if __name__=='__main__':
    for wid in sorted(out):
        name,v=out[wid]
        print(wid,name,{hex(k):(t,v2) for k,(t,v2) in sorted(v.items()) if k<0x120 or k>0x300})
