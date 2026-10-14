import sys, re, collections
sys.path.insert(0,'/tmp/audit')
from norm import func
from pairs import P
def sig(k,s,e):
    ins = func(k,s,e)
    consts = collections.Counter(re.findall(r'C(-?[0-9.e+-]+|nan|inf)', ' '.join(ins)))
    imm = collections.Counter()
    offs = collections.Counter()
    for i in ins:
        for m in re.findall(r'\[(?:e\w\w)\+0x([0-9a-f]+)\]', i): offs[int(m,16)] += 1
        m = re.match(r'(cmp|mov|test|and|or)\s+.*,0x([0-9a-f]+)$', i)
        if m and int(m.group(2),16) > 1: imm[int(m.group(2),16)] += 1
    calls = [i.split()[1] for i in ins if i.startswith('call')]
    return consts, offs, imm, len(calls)
for n,a,sa,b,sb in P:
    c1,o1,i1,n1 = sig('mp',a,a+sa); c2,o2,i2,n2 = sig('mp21',b,b+sb)
    dc = (c1-c2)+(c2-c1); do = set(o1)^set(o2); di = set(i1)^set(i2)
    if dc or do or di or n1!=n2:
        print(n, '| consts 3.0-only', dict(c1-c2), '2.1-only', dict(c2-c1), '| offs^', sorted(hex(x) for x in do), '| imm^', sorted(hex(x) for x in di), '| calls', n1, n2)
