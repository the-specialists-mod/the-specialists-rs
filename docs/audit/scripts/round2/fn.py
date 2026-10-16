#!/usr/bin/env python3
"""fn.py <so|21|win> <hex addr or name>  -- print decomp function(s) from the export."""
import sys, csv, re, os
HL = os.environ.get("HL", os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5)))  # holds this repo, ts30-decomp, ts_original, ts21
D = {'so': os.path.join(HL, 'ts30-decomp/server/'),
     '21': os.path.join(HL, 'ts30-decomp/ts21/server-win/'),
     'win': os.path.join(HL, 'ts30-decomp/server-win/')}
k, q = sys.argv[1], sys.argv[2]
d = D[k]
rows = list(csv.reader(open(d + 'functions.tsv'), delimiter='\t'))[1:]
hits = []
for r in rows:
    a = int(r[0], 16)
    if re.fullmatch(r'(0x)?[0-9a-fA-F]{5,8}', q):
        if a == int(q, 16): hits.append(r)
    elif q == r[3] or q == (r[2] + '::' + r[3]):
        hits.append(r)
for r in hits:
    src = open(d + r[7], errors='replace').read()
    m = re.search(r'^/\* %s .*?$' % r[0].lower(), src, re.M | re.I)
    if not m:
        m = re.search(r'%s' % r[0].lower(), src)
    if not m:
        print('?? not found in', r[7]); continue
    nx = re.search(r'^/\* [0-9a-f]{8}  ', src[m.end():], re.M)
    print(src[m.start(): m.end() + (nx.start() if nx else len(src))])
if not hits: print('no match')
