#!/usr/bin/env python3
"""pv_portvts.py <rust files...>
Round 1's portvts.py, extended: for each port fn whose doc comment cites TS addresses
(.so ghidra 0x5-6 hex digits, or mp.dll 0x100xxxxx: 2.1 for crates/ts21 paths or when the
doc line says 2.1, else 3.0 mp.dll with 2.1 fallback), print port numeric literals that appear
in none of the cited TS functions (with the round-1 folded variants)."""
import re, sys, glob, os, struct
HL = os.environ.get("HL", os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5)))  # holds this repo, ts30-decomp, ts_original, ts21
R = os.path.join(HL, 'ts30-decomp/')
HDR = re.compile(r'^/\* ([0-9a-f]{8})  (\S+)  \(', re.M)
NXT = re.compile(r'^/\* [0-9a-f]{8}  ', re.M)
def load(d):
    out = {}
    for f in glob.glob(os.path.join(R, d, 'decomp', '*.c')):
        src = open(f, errors='replace').read()
        for m in HDR.finditer(src):
            nx = NXT.search(src, m.end())
            out[int(m.group(1), 16)] = (m.group(2), src[m.end(): nx.start() if nx else len(src)])
    return out
SO, WIN, T21 = load('server'), load('server-win'), load('ts21/server-win')
def tsnums(b):
    out = set()
    for t in re.findall(r'(?<![\w.])(0x[0-9a-fA-F]+|\d+\.\d*(?:e[-+]?\d+)?|\d+)(?![\w])', b):
        if t.lower().startswith('0x'):
            v = int(t, 16)
            out.add(float(v))
            if 0x3a000000 <= v <= 0x4f000000 or 0xba000000 <= v <= 0xcf000000:
                out.add(abs(round(struct.unpack('<f', struct.pack('<I', v))[0], 4)))
            if 0x3f000000 <= v <= 0x41000000:
                pass
            if v >= 0x80000000 and v < 0x100000000: out.add(float(0x100000000 - v))
        else:
            out.add(abs(round(float(t), 4)))
    der = set()
    for x in out:
        if x: der |= {round(1/x, 4), round(x*2, 4), round(x/2, 4), round(x*10, 4), round(x/10, 4)}
    return out | der
IGN = {0, 1, 2, 3, 4, 8, 16, 32, 64, 100, 255, 256, 0.5, 10}
for path in sys.argv[1:]:
    src = open(path).read()
    is21 = '/ts21/' in path
    for m in re.finditer(r'((?:^[ \t]*///[^\n]*\n)+)[ \t]*(?:pub(?:\(\w+\))? )?(?:const )?fn (\w+)[^{;]*\{', src, re.M):
        cites = []
        for line in m.group(1).splitlines():
            l21 = is21 or '2.1' in line
            for a in re.findall(r'\b0x([0-9a-f]{5,8})\b', line):
                a = int(a, 16)
                if a >= 0x10000000:
                    for tab in ((T21, WIN) if l21 else (WIN, T21)):
                        if a in tab: cites.append(tab[a]); break
                elif a in SO:
                    cites.append(SO[a])
        if not cites: continue
        i = m.end() - 1; d = 0; s = i
        while i < len(src):
            if src[i] == '{': d += 1
            elif src[i] == '}':
                d -= 1
                if d == 0: break
            i += 1
        body = re.sub(r'//[^\n]*', '', src[s:i])
        body = re.sub(r'"[^"]*"', '', body)
        if '#[cfg(test)]' in body or m.group(2).startswith('test'): continue
        pn = set()
        for t in re.findall(r'(?<![\w.])(\d+\.\d+|\d+)(?:_?[fiu](?:32|64|8|16)|usize)?(?![\w.])', body):
            v = abs(round(float(t), 4))
            if v not in IGN: pn.add(v)
        have = set()
        for _, b in cites: have |= tsnums(b)
        miss = sorted(x for x in pn if x not in have)
        if miss:
            print(f"{path.split('crates/')[-1]}:{src[:m.start()].count(chr(10))+1} {m.group(2)} -> {'/'.join(c[0] for c in cites)} port-only {miss}")
