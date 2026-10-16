import os
HL = os.environ.get("HL", os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5)))  # holds this repo, ts30-decomp, ts_original, ts21
import struct,subprocess,sys
B=sys.argv[1]
d=open(B,'rb').read()
out=subprocess.check_output(['objdump','-h',B]).decode()
secs=[]
for l in out.splitlines():
    p=l.split()
    if len(p)>=6 and p[0].isdigit(): secs.append((int(p[3],16),int(p[2],16),int(p[5],16)))
def rd(va,n):
    for vma,size,off in secs:
        if vma<=va<vma+size: return d[off+va-vma:off+va-vma+n]
for a in sys.argv[2:]:
    t,a=a.split(':'); a=int(a,16)
    if t=='f': print(hex(a),struct.unpack('<f',rd(a,4))[0], rd(a,4)[::-1].hex())
    else: print(hex(a),struct.unpack('<d',rd(a,8))[0])
