#!/usr/bin/env python3
"""Generates the FindPath reference data for `bots::nodes`' tests from the decomp's replica.

Usage: python3 -I gen.py <ndm.py> <ts_original/maps> <ts_custom/maps> <out dir>

The replica (`scripts/ndm.py` in the-specialists-decomp) is imported by path. Its `find_path`
rounds the heuristic distance to f32 before truncating it; mp.dll keeps it on the x87 stack (the
stored link distances of all 43 shipped files match the unrounded value, and 8 of them don't
match the f32-rounded one), so the reference is generated with that rounding removed. The
number of pairs whose result the rounding changes is printed.

Output, one `<map>.txt` per map:
  map <name> <n>
  row <start> <digest>                         (all-pairs maps: one digest per start node)
  pairs <seed> <count> <chunk>                 (sampled maps: the LCG pairs below)
  chunk <k> <digest>                           (digest of pairs k*chunk .. (k+1)*chunk)
  path <a> <b> <ok> <iterations> <i,j,...>     (full results, start first)
Digest: FNV-1a 64 over, per pair in order, ok (u8), path length (u16 le), indices (u16 le).
LCG: x = x * 6364136223846793005 + 1442695040888963407 (mod 2^64); a = (x >> 33) % n, then the
same for b.
"""
import importlib.util
import os
import sys
from multiprocessing import Pool

M64 = (1 << 64) - 1
ndm = None
graph = None


def load_ndm(path):
    spec = importlib.util.spec_from_file_location('ndm_replica', path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def fnv(h, data):
    for b in data:
        h ^= b
        h = (h * 0x100000001b3) & M64
    return h


def encode(ok, path):
    out = bytes([1 if ok else 0]) + len(path).to_bytes(2, 'little')
    for i in path:
        out += i.to_bytes(2, 'little')
    return out


def init(ndm_path, file):
    global ndm, graph
    ndm = load_ndm(ndm_path)
    ndm.f32_orig = ndm.f32
    graph = ndm.Graph(file)


def run(pair, exact=True):
    a, b = pair
    ndm.f32 = (lambda x: x) if exact else ndm.f32_orig
    return ndm.find_path(graph, a, b)


def run_pair(pair):
    ok, p, it = run(pair, True)
    ok2, p2, _ = run(pair, False)
    return pair, ok, p, it, (ok, p) != (ok2, p2)


def lcg_pairs(seed, count, n):
    x = seed
    out = []
    for _ in range(count):
        x = (x * 6364136223846793005 + 1442695040888963407) & M64
        a = (x >> 33) % n
        x = (x * 6364136223846793005 + 1442695040888963407) & M64
        b = (x >> 33) % n
        out.append((a, b))
    return out


def classify(g, a, b, ok, p, it):
    if not ok:
        return 'fail'
    if p[0] != a:
        return 'limit'
    if len(p) != it + 2:
        return 'backtrack'
    if any(g.nodes[i][3] & 1 for i in p):
        return 'ladder'
    return 'plain'


def compute(ndm_path, file, pairs):
    with Pool(int(os.environ.get('JOBS', '6')), initializer=init, initargs=(ndm_path, file)) as pool:
        return pool.map(run_pair, pairs, chunksize=16)


def main(argv):
    ndm_path, orig, custom, out = argv
    # (map, dir, sampled pairs or None for all pairs with row digests, scan): `scan` adds the
    # backtrack / limit / failure paths of an all-pairs run (no digests).
    maps = [
        ('ts_dojo', orig, None, False),
        ('ts_japa', custom, None, False),
        ('ts_fall', orig, (0x5eed0001, 3000, 250), True),
        ('ts_central', orig, (0x5eed0002, 2000, 250), False),
        ('ts_cave_of_fear', custom, (0x5eed0003, 2000, 250), False),
        ('ts_urbannights', orig, 'scan', True),
    ]
    local = load_ndm(ndm_path)
    for name, d, sampled, scan in maps:
        file = os.path.join(d, name + '.ndm')
        g = local.Graph(file)
        n = g.n
        all_pairs = [(a, b) for a in range(n) for b in range(n)]
        lines = ['map %s %d' % (name, n)]
        results = []
        if sampled == 'scan':
            pass
        elif sampled:
            seed, count, chunk = sampled
            results = compute(ndm_path, file, lcg_pairs(seed, count, n))
            lines.append('pairs %d %d %d' % (seed, count, chunk))
            for k in range(0, count, chunk):
                h = 0xcbf29ce484222325
                for (_, ok, p, _, _) in results[k:k + chunk]:
                    h = fnv(h, encode(ok, p))
                lines.append('chunk %d %016x' % (k // chunk, h))
        else:
            results = compute(ndm_path, file, all_pairs)
            for a in range(n):
                h = 0xcbf29ce484222325
                for (_, ok, p, _, _) in results[a * n:(a + 1) * n]:
                    h = fnv(h, encode(ok, p))
                lines.append('row %d %016x' % (a, h))
        # Full paths: every category, interesting ones first.
        quota = {'backtrack': 25, 'limit': 10, 'fail': 20, 'ladder': 30, 'plain': 40}
        stats = {}
        f32_changed = 0
        picked = []
        seen = set()

        def pick(results, quota):
            nonlocal f32_changed
            for ((a, b), ok, p, it, changed) in results:
                f32_changed += changed
                if a == b or (a, b) in seen:
                    continue
                c = classify(g, a, b, ok, p, it)
                stats[c] = stats.get(c, 0) + 1
                if changed:
                    # Where the replica's f32 rounding of h gives a different result: always kept.
                    print('  f32-rounded h changes %s %d->%d' % (name, a, b))
                    seen.add((a, b))
                    picked.append('path %d %d %d %d %s' % (a, b, 1 if ok else 0, it, ','.join(map(str, p)) or '-'))
                elif quota.get(c, 0) > 0 and (c != 'plain' or (a * 7 + b) % 11 == 0):
                    quota[c] -= 1
                    seen.add((a, b))
                    picked.append('path %d %d %d %d %s' % (a, b, 1 if ok else 0, it, ','.join(map(str, p)) or '-'))

        pick(results, quota)
        scanned = 0
        if scan:
            stats = {}
            scanned = len(all_pairs)
            pick(compute(ndm_path, file, all_pairs), {'backtrack': 30, 'limit': 10, 'fail': 5})
        lines += picked
        with open(os.path.join(out, name + '.txt'), 'w') as f:
            f.write('\n'.join(lines) + '\n')
        print('%s: n=%d pairs=%d scanned=%d %s; f32-rounded h changes %d results' % (name, n, len(results), scanned, stats, f32_changed))


if __name__ == '__main__':
    main(sys.argv[1:])
