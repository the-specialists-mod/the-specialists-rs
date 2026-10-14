#!/usr/bin/env python3
"""calls.py <asm> <strings.tsv> <start-hex> [end-hex]
Per call in [start,end): the immediates pushed since the previous call (floats decoded), and
any string / known data referenced. Used for event sound/effect comparison."""
import re, struct, sys

asm, strings, start = sys.argv[1], sys.argv[2], int(sys.argv[3], 16)
end = int(sys.argv[4], 16) if len(sys.argv) > 4 else None
strs = {}
for line in open(strings, encoding='utf-8', errors='replace'):
    p = line.rstrip('\n').split('\t')
    if len(p) >= 2:
        try:
            strs[int(p[0], 16)] = p[1]
        except ValueError:
            pass

def dec(v):
    v &= 0xffffffff
    if v in strs:
        return repr(strs[v])
    if v >= 0x30000000 and v < 0x50000000 or v >= 0xb0000000 and v < 0xd0000000:
        f = struct.unpack('<f', struct.pack('<I', v))[0]
        return f'{f:g}f'
    if v >= 0x80000000:
        return str(v - (1 << 32))
    return hex(v) if v > 255 else str(v)

pend = []
started = False
for line in open(asm):
    m = re.match(r'([0-9a-f]{8}):\t(\S+)\s*(.*)', line)
    if not m:
        continue
    a = int(m.group(1), 16)
    if a < start:
        continue
    if end is None and started and m.group(2) == 'ret':
        print(f'{a:08x} ret')
        break
    if end is not None and a >= end:
        break
    started = True
    op, args = m.group(2), m.group(3)
    for h in re.findall(r'0x([0-9a-f]{6,8})', args):
        v = int(h, 16)
        if v in strs:
            pend.append(f'[{strs[v]!r}]')
    if op == 'push' and re.fullmatch(r'0x[0-9a-f]+', args):
        pend.append(dec(int(args, 16)))
    elif op == 'mov' and re.search(r'PTR \[(esp|ebp)[^\]]*\],0x[0-9a-f]+$', args):
        pend.append('m:' + dec(int(args.split(',')[-1], 16)))
    elif op in ('fld', 'fmul', 'fadd', 'fsub', 'fdiv', 'fcomp', 'fsubr', 'fdivr') and 'ds:0x' in args:
        pend.append(f'{op}@{args.split("ds:")[-1]}')
    elif op == 'call':
        print(f'{a:08x} call {args:<24} {" ".join(pend)}')
        pend = []
