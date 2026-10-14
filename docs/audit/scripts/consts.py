#!/usr/bin/env python3
"""consts.py <asm> <dll> <strings.tsv> <start-hex> [end-hex]
Lists, in order, the constant operands of one function: pushed immediates, `mov [mem],imm`,
and DWORD/QWORD memory operands in the .rdata range resolved to float/double; string refs and
RandomLong/RandomFloat style calls (by target) are printed too."""
import re, struct, sys

asm, dll, strings, start = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4], 16)
end = int(sys.argv[5], 16) if len(sys.argv) > 5 else None
f = open(dll, 'rb').read()
pe = struct.unpack_from('<I', f, 0x3c)[0]
n = struct.unpack_from('<H', f, pe + 6)[0]
opt = struct.unpack_from('<H', f, pe + 20)[0]
base = struct.unpack_from('<I', f, pe + 24 + 28)[0]
secs = [struct.unpack_from('<8sIIII', f, pe + 24 + opt + i * 40) for i in range(n)]

def rd(va, fmt):
    a = va - base
    for nm, vs, v, rs, ro in secs:
        if v <= a < v + rs and nm.startswith(b'.rdata'):
            return struct.unpack_from(fmt, f, ro + a - v)[0]
    return None

strs = {}
for line in open(strings, encoding='utf-8', errors='replace'):
    p = line.rstrip('\n').split('\t')
    try:
        strs[int(p[0], 16)] = p[1]
    except (ValueError, IndexError):
        pass

def imm(v):
    v &= 0xffffffff
    if v in strs:
        return repr(strs[v])
    if 0x30000000 <= v < 0x50000000 or 0xb0000000 <= v < 0xd0000000:
        return f'{struct.unpack("<f", struct.pack("<I", v))[0]:g}f'
    return str(v - (1 << 32)) if v >= 0x80000000 else str(v)

out = []
started = False
for line in open(asm):
    m = re.match(r'([0-9a-f]{8}):\t(\S+)\s*(.*)', line)
    if not m:
        continue
    a = int(m.group(1), 16)
    if a < start:
        continue
    if end is not None and a >= end:
        break
    if end is None and started and m.group(2) == 'ret':
        break
    started = True
    op, args = m.group(2), m.group(3)
    for h in re.findall(r'0x([0-9a-f]{8})', args):
        v = int(h, 16)
        if v in strs:
            out.append(f'{a:08x} str {strs[v]!r}')
    mm = re.search(r'(DWORD|QWORD) PTR ds:0x([0-9a-f]{8})', args)
    if mm and op.startswith('f'):
        va = int(mm.group(2), 16)
        val = rd(va, '<f' if mm.group(1) == 'DWORD' else '<d')
        if val is not None:
            out.append(f'{a:08x} {op} {val:g}')
    if op == 'push' and re.fullmatch(r'0x[0-9a-f]+', args):
        out.append(f'{a:08x} push {imm(int(args, 16))}')
    elif op == 'mov' and re.search(r'(PTR \[[^\]]*\]|PTR ds:0x[0-9a-f]+|ds:0x[0-9a-f]+),0x[0-9a-f]+$', args):
        out.append(f'{a:08x} mov {args.split(",")[0].split("PTR ")[-1]} = {imm(int(args.split(",")[-1], 16))}')
    elif op == 'call':
        out.append(f'{a:08x} call {args}')
print('\n'.join(out))
