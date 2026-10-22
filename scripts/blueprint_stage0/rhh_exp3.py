import sys
exec(open('rhh_exp.py').read().split("g = collections.defaultdict(list)")[0])

def turns(poly):
    E = edges(poly)
    return [math.degrees(math.atan2(cross(e["t"], E[(i+1) % len(E)]["t"]), dot(e["t"], E[(i+1) % len(E)]["t"]))) for i, e in enumerate(E)]

def shape_cost(pa, pb, s):
    La = [e["L"] for e in edges(pa)]; Lb = [e["L"] for e in edges(pb)]; ta, tb = turns(pa), turns(pb); k = len(La)
    c = sum(abs(math.log(Lb[(j+s) % k] / La[j])) for j in range(k))
    c += sum(10.0 for j in range(k) if abs(ta[j] - tb[(j+s) % k]) > 25)
    return c

def best_s(pa, pb):
    k = len(pa); costs = [shape_cost(pa, pb, s) for s in range(k)]; s = min(range(k), key=costs.__getitem__); return s, costs[s]

def layout(src, B, s):
    bp = capture(src); out = {}
    for pn, pol in POLS.items():
        pl = resolve(bp, B["poly"], s, pol)
        errs = match_err([p["c"] for p in pl], [p["src"] for p in pl], B)
        if not errs: continue
        o, ov, _ = findings(pl, B["poly"]); _, ov0, _ = findings(resolve(bp, src["poly"], 0, pol), src["poly"])
        out[pn] = (statistics.median(errs), len(errs), len(o), len(ov - ov0), len(pl))
    return out

g = collections.defaultdict(list)
for r in rooms.values():
    r2 = nonthin(r)
    if len(r2["items"]) >= 3: g[r["base"]].append(r2)
CT, T = 1.0, 150.0
print(f"{'group':9s} rooms pairs | shape: plain  mirror-only  none | layout err<{T:.0f}mm of shape-compatible pairs (plain / needs-mirror) | median err mm (compat): hybrid percent fixed(near) | size-differing compat (cost>0.15): hybrid percent fixed(near) n | out/ov per item (hybrid)")
tot = collections.Counter()
for name in GROUPS:
    L = g.get(name, [])
    if len(L) < 5: continue
    allpairs = [(a, b) for a in L for b in L if a is not b and len(a["poly"]) >= 4]
    random.shuffle(allpairs); pairs = allpairs[:120]
    cnt = collections.Counter(); okp = []; okm = []; err = collections.defaultdict(list); dif = collections.defaultdict(list); o = ov = it = 0
    for A, B in pairs:
        if len(A["poly"]) != len(B["poly"]): cnt["edge count differs"] += 1; continue
        sp, cp = best_s(A["poly"], B["poly"]); Am = reflect(A); sm, cm = best_s(Am["poly"], B["poly"])
        if cp <= CT and cp <= cm:
            cnt["plain"] += 1; r = layout(A, B, sp); variant = "plain"
        elif cm <= CT:
            cnt["mirror-only"] += 1; r = layout(Am, B, sm); variant = "mirror"
        else: cnt["none"] += 1; continue
        if "hybrid" not in r: continue
        (okp if variant == "plain" else okm).append(r["hybrid"][0] < T)
        for pn in ("hybrid", "percent", "fixed(near end)"):
            if pn in r:
                err[pn].append(r[pn][0])
                if min(cp, cm) > 0.15: dif[pn].append(r[pn][0])
        o += r["hybrid"][2]; ov += r["hybrid"][3]; it += r["hybrid"][4]
    n = len(pairs); sc = sum(cnt[k] for k in ("plain", "mirror-only", "none"))
    if sc == 0: print(f"{name:9s} {len(L):4d} {n:4d} | edge counts differ on all pairs"); continue
    f = lambda d, pn: f"{statistics.median(d[pn]):5.0f}" if d[pn] else "    -"
    pp = f"{sum(okp)/len(okp):4.2f}({len(okp)})" if okp else "  -  "; pm = f"{sum(okm)/len(okm):4.2f}({len(okm)})" if okm else "  -  "
    print(f"{name:9s} {len(L):4d} {n:4d} | {cnt['plain']/n:5.2f} {cnt['mirror-only']/n:7.2f} {cnt['none']/n:7.2f} (edge count differs {cnt['edge count differs']/n:4.2f}) | {pp:>9s} / {pm:>9s} | {f(err,'hybrid')} {f(err,'percent')} {f(err,'fixed(near end)')} | {f(dif,'hybrid')} {f(dif,'percent')} {f(dif,'fixed(near end)')} {len(dif['hybrid']):3d} | {o/max(it,1):.3f} {ov/max(it,1):.3f}")
    for k, v in cnt.items(): tot[k] += v
print("\npooled shape outcome:", dict(tot))
