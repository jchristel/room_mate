import random
from rhh import *
rooms, sk = load_rhh()
random.seed(1)
GROUPS = ["1 BED B", "1 BED A", "ENS-IN", "ENS-OUT", "WCST", "RESUS", "BMEQ", "CONSULT", "PT-HOLD", "PROP-ST", "WCAC", "MFD", "2BR-IN"]
NAMES = list(POLS) + ["centroid"]

def nonthin(r):
    r2 = dict(r); r2["items"] = [i for i in r["items"] if not i["thin"]]; return r2

def cen(poly): return (sum(p[0] for p in poly)/len(poly), sum(p[1] for p in poly)/len(poly))

def match_err(placed_pts, srcs, B):
    """greedy global matching by type; returns distances for matched items"""
    cand = []
    for si, (c, t) in enumerate(zip(placed_pts, srcs)):
        for bj, b in enumerate(B["items"]):
            if b["type"] == t["type"]: cand.append((dist(c, b["c"]), si, bj))
    cand.sort(); us, ub, out = set(), set(), []
    for d, si, bj in cand:
        if si in us or bj in ub: continue
        us.add(si); ub.add(bj); out.append(d)
    return out

def fit(lens_a, poly_b, s):
    L = [e["L"] for e in edges(poly_b)]; k = len(L)
    return sum(abs(math.log(L[(j + s) % k] / lens_a[j])) for j in range(k))

def run_pair(A, B):
    """returns {variant: {policy: (median err, n matched, outside, new overlaps)}} using oracle shift among near-best fits"""
    res = {}
    k = len(B["poly"])
    for variant, src in (("plain", A), ("mirror", reflect(A))):
        if len(src["poly"]) != k: continue
        bp = capture(src)
        costs = [fit(bp["lens"], B["poly"], s) for s in range(k)]
        cands = [s for s in range(k) if costs[s] <= min(costs) + 0.25]
        best = {}
        for s in cands:
            for pn, pol in POLS.items():
                pl = resolve(bp, B["poly"], s, pol)
                errs = match_err([p["c"] for p in pl], [p["src"] for p in pl], B)
                if not errs: continue
                out, ov, _ = findings(pl, B["poly"])
                _, ov0, _ = findings(resolve(bp, src["poly"], 0, pol), src["poly"])
                row = (statistics.median(errs), len(errs), len(out), len(ov - ov0))
                if pn not in best or row[0] < best[pn][0]: best[pn] = row
            # no-recipe baseline: keep offset from centroid under the same rotation
            EA, EB = edges(src["poly"]), edges(B["poly"])
            ta, tb = EA[0]["t"], EB[s]["t"]
            ang = math.atan2(cross(ta, tb), dot(ta, tb)); co, si = math.cos(ang), math.sin(ang)
            ca, cb = cen(src["poly"]), cen(B["poly"])
            pts = [(cb[0] + (i["c"][0]-ca[0])*co - (i["c"][1]-ca[1])*si, cb[1] + (i["c"][0]-ca[0])*si + (i["c"][1]-ca[1])*co) for i in src["items"]]
            errs = match_err(pts, src["items"], B)
            if errs:
                row = (statistics.median(errs), len(errs), 0, 0)
                if "centroid" not in best or row[0] < best["centroid"][0]: best["centroid"] = row
        res[variant] = best
    return res

g = collections.defaultdict(list)
for r in rooms.values():
    r2 = nonthin(r)
    if len(r2["items"]) >= 3: g[r["base"]].append(r2)

print(f"{'group':10s} rooms  modal shape       pairs  | median-of-medians error (mm), plain: " + " ".join(f"{n[:8]:>8s}" for n in NAMES) + " | mirror better | outside/item  new ov/item (hybrid)")
tot = collections.defaultdict(list)
for name in GROUPS:
    L = g.get(name, [])
    if len(L) < 5: continue
    ks = collections.Counter((len(r["poly"]), orthogonal(r["poly"])) for r in L); (k, o), kn = ks.most_common(1)[0]
    M = [r for r in L if len(r["poly"]) == k and orthogonal(r["poly"]) == o]
    allpairs = [(a, b) for a in M for b in M if a is not b]
    random.shuffle(allpairs); pairs = allpairs[:40]
    per = collections.defaultdict(list); mir_better = 0; n = 0; out = ov = items = 0; ratio = []
    for A, B in pairs:
        r = run_pair(A, B)
        if "plain" not in r or "hybrid" not in r["plain"]: continue
        n += 1
        for pn, row in r["plain"].items(): per[pn].append(row[0])
        mp = r["plain"]["hybrid"][0]; mm = r.get("mirror", {}).get("hybrid", (1e9,))[0]
        mir_better += (mm < mp - 50)
        out += r["plain"]["hybrid"][2]; ov += r["plain"]["hybrid"][3]; items += len(A["items"])
        ra = (edges(B["poly"])[0]["L"]) / edges(A["poly"])[0]["L"]; ratio.append(ra)
    if n == 0: continue
    med = {pn: statistics.median(v) for pn, v in per.items()}
    for pn, v in per.items(): tot[pn] += v
    print(f"{name:10s} {len(L):4d}  {k:2d}-edge {'ortho' if o else 'angled':6s} {kn:3d}/{len(L):3d} {n:4d}  | {'':36s}" + " ".join(f"{med.get(pn, float('nan')):8.0f}" for pn in NAMES) + f" | {mir_better:3d}/{n:<3d}        | {out/max(items,1):6.3f}        {ov/max(items,1):6.3f}")
print("\npooled median-of-pair-medians:", {pn: round(statistics.median(v)) for pn, v in tot.items()})
