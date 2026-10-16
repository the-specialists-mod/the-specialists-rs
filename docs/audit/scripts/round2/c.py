import os
HL = os.environ.get("HL", os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5)))  # holds this repo, ts30-decomp, ts_original, ts21
import struct,sys
# usage: c.py mp21|mp|so addr [f|d|i]...
paths={'mp21':os.path.join(HL, 'ts21/dlls/mp.dll'),'mp':os.path.join(HL, 'ts_original/dlls/mp.dll')}
w=sys.argv[1]
if w=='so':
    import subprocess
    B=os.path.join(HL, 'ts_original/dlls/ts_i386.so')
    for a in sys.argv[2:]:
        va=int(a,16)-0x10000
        out=subprocess.run(['objdump','-s','--start-address=%d'%va,'--stop-address=%d'%(va+8),B],capture_output=True,text=True).stdout.split('\n')
        hx=''.join(l.split()[1]+l.split()[2] if len(l.split())>2 else '' for l in out if l.startswith(' '))
        b=bytes.fromhex(hx[:16].ljust(16,'0'))
        print(a,struct.unpack('<f',b[:4])[0],struct.unpack('<d',b)[0])
    sys.exit()
d=open(paths[w],'rb').read()
pe=struct.unpack_from('<I',d,0x3c)[0]; n=struct.unpack_from('<H',d,pe+6)[0]; opt=struct.unpack_from('<H',d,pe+20)[0]
base=struct.unpack_from('<I',d,pe+24+28)[0]; secs=pe+24+opt
S=[struct.unpack_from('<8sIIII',d,secs+40*i) for i in range(n)]
def rd(va,fmt):
    for name,vs,va0,rs,ro in S:
        if base+va0<=va<base+va0+rs: return struct.unpack_from(fmt,d,ro+va-base-va0)[0]
for a in sys.argv[2:]:
    va=int(a,16); print(a,'f',rd(va,'<f'),'d',rd(va,'<d'),'i',rd(va,'<i'))
