#!/usr/bin/env python3
"""Compare constants (strings, numbers) of TS 3.0 decomp functions vs HL SDK source functions.
usage: sdkdiff.py <sdk_dlls_dir> <ts_decomp_dir> <functions.tsv> [class-regex]"""
import re, sys, os, struct, glob

sdk_dir, ts_dir, ftsv = sys.argv[1:4]
cls_re = re.compile(sys.argv[4]) if len(sys.argv) > 4 else None

def norm_num(tok):
    t = tok.rstrip('fFlLuU')
    try:
        if t.lower().startswith('0x'):
            v = int(t, 16)
            if v >= 0x3c000000 and v <= 0x4f000000 or (v >= 0xbc000000 and v <= 0xcf000000):
                f = struct.unpack('<f', struct.pack('<I', v))[0]
                return round(f, 4)
            return float(v)
        return round(float(t), 4)
    except ValueError:
        return None

NUM = re.compile(r'(?<![\w.])(0x[0-9a-fA-F]+|\d+\.\d*(?:e[-+]?\d+)?|\d+(?:e[-+]?\d+)?|\.\d+)[fFlLuU]*(?![\w])')
STR = re.compile(r'"((?:[^"\\]|\\.)*)"')

def consts(body):
    body = re.sub(r'//[^\n]*|/\*.*?\*/', '', body, flags=re.S)
    strs = set(s for s in STR.findall(body) if s)
    b2 = STR.sub('', body)
    nums = set()
    for m in NUM.finditer(b2):
        v = norm_num(m.group(1))
        if v is not None and abs(v) not in (0.0, 1.0, 2.0, 3.0, 4.0):
            nums.add(abs(v))
    return strs, nums

# SDK functions
sdk = {}
for f in glob.glob(os.path.join(sdk_dir, '*.cpp')):
    src = open(f, errors='replace').read()
    for m in re.finditer(r'^[\w\s\*&:<>]*?\b(\w+)\s*::\s*(~?\w+)\s*\([^;{]*?\)\s*(?:const\s*)?\{', src, re.M):
        start = m.end() - 1
        depth = 0
        i = start
        while i < len(src):
            c = src[i]
            if c == '{': depth += 1
            elif c == '}':
                depth -= 1
                if depth == 0: break
            i += 1
        key = (m.group(1), m.group(2))
        sdk.setdefault(key, '')
        sdk[key] += src[start:i + 1]

# TS functions
ts = {}
for f in glob.glob(os.path.join(ts_dir, '*.c')):
    src = open(f, errors='replace').read()
    for m in re.finditer(r'^/\* ([0-9a-f]{8})  (\w+)::(~?\w+)  \(', src, re.M):
        nx = re.compile(r'^/\* [0-9a-f]{8}  ', re.M).search(src, m.end())
        end = nx.start() if nx else -1
        body = src[m.end(): end if end > 0 else len(src)]
        body = body[body.find('*/')+2:]
        ts[(m.group(2), m.group(3), m.group(1))] = body

for (c, meth, addr), body in sorted(ts.items(), key=lambda x: x[0][2]):
    if cls_re and not cls_re.search(c):
        continue
    if (c, meth) not in sdk:
        continue
    s1, n1 = consts(body)
    s2, n2 = consts(sdk[(c, meth)])
    s_extra = s1 - s2
    n_extra = n1 - n2
    n_missing = n2 - n1
    s_missing = s2 - s1
    if s_extra or n_extra or s_missing:
        print(f"{addr} {c}::{meth}  +str{sorted(s_extra)} -str{sorted(s_missing)} +num{sorted(n_extra)} -num{sorted(n_missing)}")
