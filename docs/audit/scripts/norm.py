import re, struct, sys, difflib
BIN = {
 'mp':  ('../ts_original/dlls/mp.dll', '/tmp/audit/mp.asm', (0x10089000,0x1009a000), (0x1009a000,0x10400000)),
 'cl':  ('../ts_original/cl_dlls/client.dll', '/tmp/audit/cl.asm', (0x10079000,0x10091000), (0x10091000,0x10400000)),
 'mp21':('../ts21/dlls/mp.dll', '/tmp/audit/mp21.asm', (0x10080000,0x1008f000), (0x1008f000,0x10400000)),
 'cl21':('../ts21/cl_dlls/client.dll', '/tmp/audit/cl21.asm', (0x10267000,0x10288000), (0x10288000,0x10500000)),
}
_cache = {}
def load(k):
    if k in _cache: return _cache[k]
    path, asm, ro, rw = BIN[k]
    data = open(path,'rb').read()
    lines = {}
    order = []
    for l in open(asm):
        m = re.match(r'^([0-9a-f]{8}):\t(.*)$', l.rstrip('\n'))
        if m:
            a = int(m.group(1),16); lines[a] = m.group(2).strip(); order.append(a)
    _cache[k] = (data, lines, order, ro, rw)
    return _cache[k]

def func(k, start, end, names=None):
    data, lines, order, ro, rw = load(k)
    import bisect
    i = bisect.bisect_left(order, start)
    addrs = []
    while i < len(order) and order[i] < end:
        addrs.append(order[i]); i += 1
    idx = {a:n for n,a in enumerate(addrs)}
    out = []
    for a in addrs:
        ins = lines[a]
        if ins in ('nop','int3') or ins.startswith('lea    esi,[esi') or ins.startswith('lea    edi,[edi') : continue
        def mem(m):
            size, va = m.group(1), int(m.group(2),16)
            if ro[0] <= va < ro[1]:
                off = va - 0x10000000
                if size == 'QWORD': return 'C%r' % struct.unpack('<d', data[off:off+8])[0]
                if size == 'DWORD': return 'C%r' % struct.unpack('<f', data[off:off+4])[0]
                return 'C?'
            return 'G'
        ins = re.sub(r'(QWORD|DWORD|WORD|BYTE) PTR ds:0x([0-9a-f]+)', mem, ins)
        m = re.match(r'(j\w+|call)\s+0x([0-9a-f]+)$', ins)
        if m:
            t = int(m.group(2),16)
            if t in idx: ins = '%s L%d' % (m.group(1), idx[t]-idx[a])
            else: ins = '%s %s' % (m.group(1), (names or {}).get(t, 'EXT'))
        ins = re.sub(r'0x1[0-9a-f]{7}', 'ADDR', ins)
        out.append(ins)
    return out

def diff(k1, s1, e1, k2, s2, e2, show=True, n=1):
    a = func(k1,s1,e1); b = func(k2,s2,e2)
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    r = sm.ratio()
    if show:
        for l in difflib.unified_diff(a, b, lineterm='', n=n):
            print(l)
    return r, len(a), len(b)

if __name__ == '__main__':
    k1,s1,e1,k2,s2,e2 = sys.argv[1], int(sys.argv[2],16), int(sys.argv[3],16), sys.argv[4], int(sys.argv[5],16), int(sys.argv[6],16)
    print(diff(k1,s1,e1,k2,s2,e2))
