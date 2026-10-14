#!/usr/bin/env python3
"""Integer + float constant diff of TS 3.0 decomp vs SDK source, offsets stripped.
usage: sdkint.py <sdk_dlls> <ts_decomp> [class-regex]"""
import re, sys, glob, os, struct
sdk_dir, ts_dir = sys.argv[1:3]
cre = re.compile(sys.argv[3]) if len(sys.argv) > 3 else None
def strip_ts(b):
    b = re.sub(r'/\*.*?\*/', '', b, flags=re.S)
    b = re.sub(r'"(?:[^"\\]|\\.)*"', '""', b)
    b = re.sub(r'field_0x[0-9a-f]+', 'F', b)
    b = re.sub(r'(\+|-)\s*0x[0-9a-f]+\s*\)', ')', b)       # (p + 0x1c)
    b = re.sub(r'\[0x[0-9a-f]+\]', '[]', b)
    b = re.sub(r'\[\d+\]', '[]', b)
    b = re.sub(r'vtable\[[^]]*\]', 'V', b)
    b = re.sub(r'(?:DAT|PTR|LAB|FUN|s)_[0-9a-f]+', 'X', b)
    b = re.sub(r'\b(?:local|uStack|iStack|fStack|auStack|in_stack|param|uVar|iVar|fVar|pcVar|piVar|puVar|bVar|lVar|pvVar|peVar|pCVar)_?\w*', 'v', b)
    return b
def strip_sdk(b):
    b = re.sub(r'//[^\n]*|/\*.*?\*/', '', b, flags=re.S)
    b = re.sub(r'"(?:[^"\\]|\\.)*"', '""', b)
    b = re.sub(r'\[\s*\d+\s*\]', '[]', b)
    return b
NUM = re.compile(r'(?<![\w.])(0x[0-9a-fA-F]+|\d+\.\d*(?:e[-+]?\d+)?|\d+)[fFlLuU]*(?![\w.])')
def nums(b, ts):
    out = set()
    for t in NUM.findall(b):
        if t.lower().startswith('0x'):
            v = int(t, 16)
            if ts and (0x3a000000 <= v <= 0x4f000000 or 0xba000000 <= v <= 0xcf000000):
                f = abs(round(struct.unpack('<f', struct.pack('<I', v))[0], 4))
            else:
                f = float(v if v < 0x80000000 else v - 0x100000000)
        else:
            f = float(t)
        f = abs(round(f, 4))
        if f in (0, 1): continue
        out.add(f)
    return out
sdk = {}
for f in glob.glob(os.path.join(sdk_dir, '*.cpp')):
    src = open(f, errors='replace').read()
    for m in re.finditer(r'^[\w\s\*&:<>]*?\b(\w+)\s*::\s*(~?\w+)\s*\([^;{]*?\)\s*(?:const\s*)?\{', src, re.M):
        i = m.end() - 1; d = 0; s = i
        while i < len(src):
            if src[i] == '{': d += 1
            elif src[i] == '}':
                d -= 1
                if d == 0: break
            i += 1
        sdk[(m.group(1), m.group(2))] = sdk.get((m.group(1), m.group(2)), '') + src[s:i+1]
HDR = re.compile(r'^/\* ([0-9a-f]{8})  (\w+)::(~?\w+)  \(', re.M)
NXT = re.compile(r'^/\* [0-9a-f]{8}  ', re.M)
for f in sorted(glob.glob(os.path.join(ts_dir, '*.c'))):
    src = open(f, errors='replace').read()
    for m in HDR.finditer(src):
        c, me = m.group(2), m.group(3)
        if cre and not cre.search(c): continue
        if (c, me) not in sdk: continue
        nx = NXT.search(src, m.end())
        body = src[m.end(): nx.start() if nx else len(src)]
        body = body[body.find('*/') + 2:]
        a = nums(strip_ts(body), True); b = nums(strip_sdk(sdk[(c, me)]), False)
        extra = sorted(a - b)
        if extra:
            print(f"{m.group(1)} {c}::{me} +{extra} -{sorted(b-a)}")
