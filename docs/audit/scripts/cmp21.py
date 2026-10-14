import sys, struct, tomllib
sys.path.insert(0,'/tmp/audit/scripts')
from dump21 import out, f32
t=tomllib.load(open('../rust-hlmod-the-specialists/crates/ts21/data/weapons21.toml','rb'))
MODES=['auto','semi','burst','pump','ak_semi','ak_auto']
KIND={1:'shotgun',2:'grenade',3:'knife',4:'akimbo',5:'akimbo_only',6:'rifle',7:'sniper',8:'pistol',9:'smg',10:'katana'}
F=[('clip',4,'i'),('ammo',8,'i'),('reload',0x14,'f'),('rate',0x18,'f'),('mult',0x1c,'f'),('bullet',0x20,'i'),('pellets',0x24,'i'),('accuracy',0x28,'f'),('scale_spread',0x2c,'bool'),('view_kick',0x30,'f'),('acc_kick',0x34,'f'),('kick',0x38,'bool'),('stock',0x10,'bool'),('akimbo',0xc6,'s'),('single',0xc8,'s'),('slots',0xca,'i'),('cost',0xd0,'i'),('stack',0xd4,'bool'),('att_allowed',0xd8,'i'),('att_default',0xdc,'i')]
def val(v,off):
    if off not in v: return 0
    return int(v[off][1],0)
seen=set()
for w in t['weapon']:
    wid=w['id']; seen.add(wid)
    if wid==0: continue
    if wid not in out: print('port has',wid,'not in 2.1'); continue
    name,v=out[wid]
    for f,off,ty in F:
        r=val(v,off)
        if ty=='f': tv=f32(r); ok=struct.pack('<f',tv)==struct.pack('<f',float(w[f]))
        elif ty=='bool': tv=bool(r); ok=tv==w[f]
        elif ty=='s': tv=r-0x10000 if r>=0x8000 else r; ok=tv==w[f]
        else: tv=r; ok=tv==w[f]
        if not ok: print(wid,name,f,'ts=',tv,'port=',w[f])
    m=val(v,0xc); pm=sum(1<<MODES.index(x) for x in w['modes'])
    if m!=pm: print(wid,name,'modes',bin(m),bin(pm))
    z=[val(v,0x3c),val(v,0x40)]
    if z!=w['zoom']: print(wid,name,'zoom',z,w['zoom'])
    h=[val(v,0x84),val(v,0x85)]
    if h!=w['hud']: print(wid,name,'hud',h,w['hud'])
    k=KIND.get(val(v,0x360))
    if k!=w['kind']: print(wid,name,'kind',k,w['kind'])
    print(wid, 'extra 0xcc=%g'%f32(val(v,0xcc)), [val(v,o) for o in (0x364,0x368,0x36c,0x370)])
print('missing in port:', sorted(set(out)-seen))
