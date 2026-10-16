import os, re, struct, sys, tomllib

HL = os.environ.get('HL', os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *['..'] * 4)))  # holds this repo and ts30-decomp
D = os.path.join(HL, 'ts30-decomp/server/decomp/')
src = open(D + 'global_00066308.c').read() + open(D + 'global_0008bd38.c').read()
toml = tomllib.load(open(sys.argv[1], 'rb'))

# address of each Write_ call -> id
cw = re.search(r'void compile_weapons_info\(void\)\s*\{(.*?)\n\}', src, re.S).group(1)
calls = re.findall(r'Write_(\w+)\((?:&DAT_)?(?:0x)?([0-9a-f]+)\)', cw)
funcs = {}
for name, addr in calls:
    a = int(addr, 16)
    funcs[name] = (a - 0x17db40) // 0x3dc


def f32(v):
    return struct.unpack('<f', struct.pack('<I', v & 0xffffffff))[0]


def parse(name):
    m = re.search(r'void Write_' + name + r'\((?:undefined1 \*|int )param_1\)\s*\{(.*?)\n\}', src, re.S)
    body = m.group(1)
    vals = {}
    for off, v in re.findall(r'\*\(undefined4 \*\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) = (0x[0-9a-f]+|-?\d+);', body):
        vals[int(off, 0)] = int(v, 0)
    for off, v in re.findall(r'param_1\[(0x[0-9a-f]+|\d+)\] = (0x[0-9a-f]+|-?\d+|\'.\');', body):
        vals[int(off, 0)] = int(v, 0) if not v.startswith("'") else ord(v[1])
    for off, v in re.findall(r'\*\(undefined2 \*\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) = (0x[0-9a-f]+|-?\d+);', body):
        vals[('s', int(off, 0))] = int(v, 0)
    return vals


ts = {}
for name, wid in funcs.items():
    if name == 'melee':
        continue
    ts[wid] = parse(name)

sp = re.search(r'void Sparta\(void\)\s*\{(.*?)\n\}', src, re.S).group(1)
for a in re.findall(r'SetSpartaWeapon\(([^)]*)\)', sp):
    p = [int(x, 0) for x in a.split(',')]
    v = ts[p[0]]
    v[0x28], v[0x30], v[0x34], v[0x1c] = p[1], p[2], p[3], p[4]

MODES = ['auto', 'semi', 'burst', 'pump', 'ak_semi', 'ak_auto']
KIND = {1: 'shotgun', 2: 'grenade', 3: 'knife', 4: 'akimbo', 5: 'akimbo_only', 6: 'rifle', 7: 'sniper', 8: 'pistol', 9: 'smg', 10: 'katana'}
fields = [
    ('clip', 4, 'i'), ('ammo', 8, 'b'), ('stock', 0x10, 'i'), ('reload', 0x14, 'f'), ('rate', 0x18, 'f'),
    ('damage', 0x1c, 'f'), ('bullet', 0x20, 'i'), ('pellets', 0x24, 'i'), ('accuracy', 0x28, 'f'),
    ('scale_spread', 0x2c, 'bool'), ('view_kick', 0x30, 'f'), ('acc_kick', 0x34, 'f'), ('kick', 0x38, 'bool'),
    ('akimbo', 200, 'i'), ('single', 0xcc, 'i'), ('slots', 0xd0, 'b'), ('cost', 0xd8, 'i'), ('stack', 0xdc, 'bool'),
    ('att_allowed', 0xe0, 'i'), ('att_default', 0xe4, 'i'),
]
for w in toml['weapon']:
    wid = w['id']
    if wid == 0:
        continue
    v = ts[wid]
    for fname, off, t in fields:
        raw = v.get(off, 0)
        if t == 'f':
            tv = f32(raw)
            ok = struct.pack('<f', tv) == struct.pack('<f', float(w[fname]))
        elif t == 'bool':
            tv = bool(raw)
            ok = tv == w[fname]
        else:
            tv = raw if raw < 0x80000000 else raw - 0x100000000
            ok = tv == w[fname]
        if not ok:
            print(wid, w['key'], fname, 'ts=', tv, 'port=', w[fname])
    mask = v.get(0xc, 0)
    pm = sum(1 << MODES.index(m) for m in w['modes'])
    if mask != pm:
        print(wid, w['key'], 'modes ts=', bin(mask), 'port=', bin(pm))
    z = [v.get(0x3c, 0), v.get(0x40, 0)]
    if z != w['zoom']:
        print(wid, w['key'], 'zoom', z, w['zoom'])
    h = [v.get(0x84, 0), v.get(0x85, 0)]
    if h != w['hud']:
        print(wid, w['key'], 'hud', h, w['hud'])
    k = KIND.get(v.get(0x348, 0))
    if k != w['kind']:
        print(wid, w['key'], 'kind', k, w['kind'])
# anim tables
import json
print('ts anim tables')
for wid in sorted(ts):
    v = ts[wid]
    a = [v.get(('s', 0x252 + 2 * i), 0) for i in range(29)]
    b = [v.get(('s', 0x28c + 2 * i), 0) for i in range(29)]
    a = [x - 0x10000 if x >= 0x8000 else x for x in a]
    b = [x - 0x10000 if x >= 0x8000 else x for x in b]
    print(wid, a, b)
