#!/usr/bin/env python3
"""For port fns whose doc comment cites a TS .so address, list port numeric literals absent from
that TS function. usage: portvts.py <ts_decomp> <rust files...>"""
import re, sys, glob, os, struct
ts_dir = sys.argv[1]
HDR = re.compile(r'^/\* ([0-9a-f]{8})  (\S+)  \(', re.M)
NXT = re.compile(r'^/\* [0-9a-f]{8}  ', re.M)
ts = {}
for f in glob.glob(os.path.join(ts_dir, '*.c')):
    src = open(f, errors='replace').read()
    for m in HDR.finditer(src):
        nx = NXT.search(src, m.end())
        ts[int(m.group(1), 16)] = (m.group(2), src[m.end(): nx.start() if nx else len(src)])
def tsnums(b):
    out = set()
    for t in re.findall(r'(?<![\w.])(0x[0-9a-fA-F]+|\d+\.\d*(?:e[-+]?\d+)?|\d+)(?![\w])', b):
        if t.lower().startswith('0x'):
            v = int(t, 16)
            out.add(float(v))
            if 0x3a000000 <= v <= 0x4f000000 or 0xba000000 <= v <= 0xcf000000:
                out.add(abs(round(struct.unpack('<f', struct.pack('<I', v))[0], 4)))
            if v >= 0x80000000: out.add(float(0x100000000 - v))
        else:
            out.add(abs(round(float(t), 4)))
    # derived: reciprocals and halves the compiler may fold
    der = set()
    for x in out:
        if x: der |= {round(1/x, 4), round(x*2, 4), round(x/2, 4), round(x*10, 4), round(x/10, 4)}
    return out | der
IGN = {0, 1, 2, 3, 4, 8, 16, 32, 64, 100, 255, 256, 0.5, 10}
for path in sys.argv[2:]:
    src = open(path).read()
    for m in re.finditer(r'((?:^[ \t]*///[^\n]*\n)+)[ \t]*(?:pub(?:\(\w+\))? )?fn (\w+)[^{]*\{', src, re.M):
        addrs = re.findall(r'\b0x([0-9a-f]{5,6})\b', m.group(1))
        if not addrs: continue
        a = int(addrs[0], 16)
        if a not in ts: continue
        i = m.end() - 1; d = 0; s = i
        while i < len(src):
            if src[i] == '{': d += 1
            elif src[i] == '}':
                d -= 1
                if d == 0: break
            i += 1
        body = re.sub(r'//[^\n]*', '', src[s:i])
        body = re.sub(r'"[^"]*"', '', body)
        pn = set()
        for t in re.findall(r'(?<![\w.])(\d+\.\d+|\d+)(?:_?[fiu](?:32|64|8|16)|usize)?(?![\w.])', body):
            v = abs(round(float(t), 4))
            if v not in IGN: pn.add(v)
        miss = sorted(x for x in pn if x not in tsnums(ts[a][1]))
        if miss:
            print(f"{os.path.basename(path)}:{src[:m.start()].count(chr(10))+1} {m.group(2)} -> {hex(a)} {ts[a][0]} port-only {miss}")
