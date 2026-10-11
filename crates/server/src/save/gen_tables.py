"""Generates crates/server/src/save/tables.rs from TS 3.0's server library and the decomp.

    python3 -I gen_tables.py <ts_i386.so> <ts30-decomp/server> > tables.rs && rustfmt --edition 2024 tables.rs

Reads every TYPEDESCRIPTION array the decomp's data_symbols.tsv names (`::m_SaveData`,
`m_playerSaveData`, `gEntvarsDescription`, `gGlobalEntitySaveData`) straight from the binary
(vaddr == file offset; Ghidra addresses are vaddr + 0x10000), the `::Save` parents and table names
from the decomp, each class's vtable `Save` slot from vtables.tsv, and the classnames from the
`LINK_ENTITY_TO_CLASS` functions.
"""
import struct, re, glob, sys

SO = sys.argv[1]
DEC = sys.argv[2]
so = open(SO, 'rb').read()
TYPES = ['Float', 'String', 'Entity', 'ClassPtr', 'EHandle', 'EVars', 'Edict', 'Vector', 'PositionVector', 'Pointer',
         'Integer', 'Function', 'Boolean', 'Short', 'Character', 'Time', 'ModelName', 'SoundName']


def cstr(a):
    return so[a:so.index(b'\0', a)].decode()


# table symbol -> (ghidra addr, count)
tables = {}
for l in open(DEC + '/data_symbols.tsv'):
    p = l.rstrip('\n').split('\t')
    if len(p) < 3:
        continue
    name = p[1]
    if '::m_SaveData' in name or '::m_playerSaveData' in name or name in ('gEntvarsDescription', 'gGlobalEntitySaveData'):
        m = re.search(r'\[(\d+)\]', p[2])
        n = int(m.group(1)) // 16 if m else 86
        tables[name] = (int(p[0], 16), n)


def fields(sym):
    addr, n = tables[sym]
    out = []
    for i in range(n):
        t, np_, off, sz, fl = struct.unpack_from('<iIihh', so, addr - 0x10000 + i * 16)
        out.append((TYPES[t], cstr(np_), off, sz, fl))
    return out


# Save chains from the decomp: class -> (parent or None, WriteFields name, table symbol, count)
save = {}
for f in glob.glob(DEC + '/decomp/*.c'):
    src = open(f, errors='replace').read()
    for m in re.finditer(r'\n\S+ __thiscall ([A-Za-z_0-9]+)::Save\((?:[A-Za-z_0-9]+) \*this,CSave \*param_1\)\n\n\{(.*?)\n\}', src, re.S):
        cls, body = m.group(1), m.group(2)
        par = re.search(r'= ([A-Za-z_0-9]+)::Save\(', body)
        wf = re.search(r'WriteFields\(param_1,"([^"]*)",\(int\)this,\(int \*\)(m_SaveData|m_playerSaveData),(0x[0-9a-f]+|\d+)\)', body)
        if wf:
            save[cls] = (par.group(1) if par else None, wf.group(1), cls + '::' + wf.group(2), int(wf.group(3), 0))

# vtable Save slot per class
slot = {}
for l in open(DEC + '/vtables.tsv'):
    p = l.rstrip('\n').split('\t')
    if len(p) > 4 and p[2] == '10' and p[4].endswith('::Save'):
        slot[p[0]] = p[4][:-len('::Save')]

# classname -> class
links = {}
for f in glob.glob(DEC + '/decomp/*.c'):
    src = open(f, errors='replace').read()
    for m in re.finditer(r'\nvoid ([A-Za-z_0-9]+)\(uint param_1\)\n\n\{(.*?)\n\}', src, re.S):
        g = re.search(r'GetClassPtr<([A-Za-z_0-9]+)>', m.group(2))
        if g:
            links[m.group(1)] = g.group(1)


def ident(cls):
    return cls


def chain(cls):
    """The table identifiers TS's Save writes for `cls`'s own Save function."""
    if cls == 'CBaseEntity':
        return ['ENTVARS', 'BASE']
    par, _, _, _ = save[cls]
    return chain(par) + [ident(cls)]


w = sys.stdout.write
w('//! TS 3.0\'s save tables, generated from `ts_i386.so` (the `TYPEDESCRIPTION` arrays in `.data`,\n')
w('//! with their field types, names, counts and flags) and the decomp (`::Save` parents,\n')
w('//! `vtables.tsv`, the `LINK_ENTITY_TO_CLASS` functions). Don\'t edit by hand: regenerate with `save/gen_tables.py`.\n')
w('//! Each field\'s comment is its offset in TS\'s object (`+0x..`), for mapping to the port\'s fields.\n\n')
w('#![allow(non_upper_case_globals)] // the statics are named after TS\'s classes\n\n')
w('use goldsrc::ext::saverestore::{FieldDesc, FieldType::*};\n')
w('use super::Table;\n\n')
w('const fn f(ty: goldsrc::ext::saverestore::FieldType, name: &\'static std::ffi::CStr, count: i16, flags: i16) -> FieldDesc {\n')
w('    FieldDesc { ty, name, count, flags }\n}\n\n')


def emit(ident_, wname, sym, cnt_check=None):
    fl = fields(sym)
    if cnt_check is not None:
        assert len(fl) == cnt_check, (sym, len(fl), cnt_check)
    addr = tables[sym][0]
    w('/// `%s` (0x%x): written as `"%s"`.\n' % (sym, addr, wname))
    w('pub static %s: Table = Table {\n    name: c"%s",\n    fields: &[\n' % (ident_, wname))
    for (t, n, off, sz, flg) in fl:
        w('        f(%s, c"%s", %d, %d), // +0x%x\n' % (t, n, sz, flg, off))
    w('    ],\n};\n\n')


emit('ENTVARS', 'ENTVARS', 'gEntvarsDescription', 86)
emit('BASE', 'BASE', 'CBaseEntity::m_SaveData', 5)
emit('GLOBAL', 'GLOBAL', 'CGlobalState::m_SaveData', 1)
emit('GENT', 'GENT', 'gGlobalEntitySaveData', 3)
for cls in sorted(save):
    if cls in ('CBaseEntity', 'CGlobalState'):
        continue
    par, wname, sym, cnt = save[cls]
    emit(ident(cls), wname, sym, cnt)

# chains per class with a Save in its vtable
w('/// Every TS class\'s `Save` chain (its vtable `Save`, then the parents that one calls first).\n')
w('pub static CHAINS: &[(&str, &[&Table])] = &[\n')
for cls in sorted(slot):
    s_ = slot[cls]
    if s_ not in save and s_ != 'CBaseEntity':
        continue
    ch = chain(s_)
    w('    ("%s", &[%s]),\n' % (cls, ', '.join('&' + c for c in ch)))
w('];\n\n')
w('/// The chain of a TS class ([`CHAINS`]).\n')
w('pub fn chain(ts_class: &str) -> Option<&\'static [&\'static Table]> {\n')
w('    CHAINS.iter().find(|(c, _)| *c == ts_class).map(|(_, t)| *t)\n}\n\n')
w('/// `LINK_ENTITY_TO_CLASS`: the TS class each classname builds.\n')
w('pub fn class_of(classname: &[u8]) -> Option<&\'static str> {\n    Some(match classname {\n')
for k in sorted(links):
    w('        b"%s" => "%s",\n' % (k, links[k]))
w('        _ => return None,\n    })\n}\n')
