import sys, re, difflib
sys.path.insert(0,'/tmp/audit')
from norm import func
from pairs import P
OPS = ('fld','fild','fmul','fdiv','fdivr','fadd','fsub','fsubr','fsqrt','fchs','fabs','fst','fstp','fcom','fcomp','fucom','fxch','faddp','fmulp','fsubp','fsubrp','fdivp','fdivrp')
def fp(k,s,e):
    out=[]
    for i in func(k,s,e):
        op=i.split()[0]
        if op not in OPS: continue
        arg=i[len(op):].strip()
        arg=re.sub(r'(DWORD|QWORD) PTR \[e(sp|bp)[^\]]*\]', r'\1 STK', arg)
        arg=re.sub(r'(DWORD|QWORD) PTR \[e\w\w(\+e\w\w\*\d)?\+0x([0-9a-f]+)\]', lambda m: '%s pm+%s'%(m.group(1),m.group(3)), arg)
        arg=re.sub(r'(DWORD|QWORD) PTR \[e\w\w\]', r'\1 pm+0', arg)
        if op=='fxch': arg=''
        out.append(op+' '+arg)
    return out
names = sys.argv[1:]
for n,a,sa,b,sb in P:
    if names and n not in names: continue
    x=fp('mp',a,a+sa); y=fp('mp21',b,b+sb)
    r=difflib.SequenceMatcher(None,x,y,autojunk=False).ratio()
    print('== %s %.3f'%(n,r))
    if names:
        for l in difflib.unified_diff(x,y,lineterm='',n=2): print(l)
