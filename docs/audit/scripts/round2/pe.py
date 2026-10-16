import struct, sys, os
HL = os.environ.get("HL", os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5)))  # holds this repo, ts30-decomp, ts_original, ts21
class PE:
    def __init__(s, path):
        f = s.f = open(os.path.expanduser(path), 'rb').read()
        pe = struct.unpack_from('<I', f, 0x3c)[0]
        n = struct.unpack_from('<H', f, pe + 6)[0]
        opt = struct.unpack_from('<H', f, pe + 20)[0]
        s.base = struct.unpack_from('<I', f, pe + 24 + 28)[0]
        s.secs = [struct.unpack_from('<8sIIII', f, pe + 24 + opt + i * 40) for i in range(n)]
    def off(s, va):
        a = va - s.base
        for nm, vs, v, rs, ro in s.secs:
            if v <= a < v + max(rs, vs):
                return ro + a - v if a - v < rs else None
        return None
    def rd(s, va, fmt):
        o = s.off(va)
        return None if o is None else struct.unpack_from(fmt, s.f, o)[0]
    def cstr(s, va):
        o = s.off(va)
        if o is None: return None
        e = s.f.index(b'\0', o)
        return s.f[o:e].decode('latin1')
mp21 = PE(os.path.join(HL, 'ts21/dlls/mp.dll'))
if __name__ == '__main__':
    # scan cvar_t (name,string,flags,value,next) 0x14 in .data
    p = mp21
    for va in range(0x10093000, 0x10094800, 4):
        n = p.rd(va, '<I'); st = p.rd(va + 4, '<I')
        if not n or not st: continue
        a = p.cstr(n) if 0x10080000 <= n < 0x100a0000 else None
        b = p.cstr(st) if 0x10080000 <= st < 0x100a0000 else None
        if a and b is not None and a.replace('_','').isalnum() and len(a) > 2:
            print(hex(va), a, repr(b), 'flags', p.rd(va+8,'<I'), 'value@', hex(va+12))
