#!/usr/bin/env python3
"""Compare constants of same-named Class::Method in TS 3.0 (.so decomp) vs 2.1 (mp.dll decomp).
Strings normalised (alnum lowercase; 2.1 s_name_addr refs too), numbers: fractional floats and
hex-float immediates. usage: diff30_21.py <so_decomp> <mp21_decomp>"""
import re, sys, glob, os, struct

HDR = re.compile(r'^/\* ([0-9a-f]{8})  (\w+)::(~?\w+)  \(', re.M)
NXT = re.compile(r'^/\* [0-9a-f]{8}  ', re.M)

def funcs(d):
    out = {}
    for f in glob.glob(os.path.join(d, '*.c')):
        src = open(f, errors='replace').read()
        for m in HDR.finditer(src):
            nx = NXT.search(src, m.end())
            body = src[m.end(): nx.start() if nx else len(src)]
            body = body[body.find('*/') + 2:]
            out.setdefault((m.group(2), m.group(3)), []).append((m.group(1), body))
    return out

def norm(s):
    return re.sub(r'[^a-z0-9]', '', s.lower())

def consts(body):
    body = re.sub(r'/\*.*?\*/', '', body, flags=re.S)
    strs = set(norm(s) for s in re.findall(r'"((?:[^"\\]|\\.)*)"', body))
    strs |= set(norm(s) for s in re.findall(r'\bs_(\w+?)_[0-9a-f]{8}\b', body))
    strs.discard('')
    b = re.sub(r'"(?:[^"\\]|\\.)*"', '', body)
    nums = set()
    for t in re.findall(r'(?<![\w.])(0x[0-9a-f]{8}|\d+\.\d+(?:e[-+]?\d+)?)(?![\w])', b):
        if t.startswith('0x'):
            v = int(t, 16)
            if not (0x3a000000 <= v <= 0x4f000000 or 0xba000000 <= v <= 0xcf000000):
                continue
            f = struct.unpack('<f', struct.pack('<I', v))[0]
        else:
            f = float(t)
        f = abs(round(f, 4))
        if f not in (0.0, 1.0):
            nums.add(f)
    return strs, nums

a, b = funcs(sys.argv[1]), funcs(sys.argv[2])
for k in sorted(set(a) & set(b)):
    if len(a[k]) != 1 or len(b[k]) != 1:
        continue
    (aa, ab), (ba, bb) = a[k][0], b[k][0]
    s1, n1 = consts(ab)
    s2, n2 = consts(bb)
    if s1 != s2 or n1 != n2:
        print(f"{aa} {ba} {k[0]}::{k[1]}  30only s{sorted(s1-s2)} n{sorted(n1-n2)}  21only s{sorted(s2-s1)} n{sorted(n2-n1)}")
