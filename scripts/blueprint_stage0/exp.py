from bp0 import *
rooms, skipped = load()
print("skipped:", dict(skipped))
thin = sum(i["thin"] for r in rooms.values() for i in r["items"]); tot = sum(len(r["items"]) for r in rooms.values())
print(f"items kept {tot}; thin (<100 mm, wall devices) {thin}\n")

def nonthin(r): 
    r2 = dict(r); r2["items"] = [i for i in r["items"] if not i["thin"]]; return r2

# ---- E1 round trip
print("E1 round trip (capture, resolve back onto the same room, shift 0), max centre error mm per policy")
for name, pol in POLS.items():
    worst = 0
    for r in rooms.values():
        if not r["items"]: continue
        bp = capture(r); pl = resolve(bp, r["poly"], 0, pol)
        worst = max(worst, max(dist(p["c"], p["src"]["c"]) for p in pl))
    print(f"  {name:16s} {worst:8.3f}")

# ---- E2 synthetic size/proportion change
SC = [(1.0,1.0),(0.85,0.85),(1.15,1.15),(1.3,1.3),(1.3,0.8),(0.8,1.3),(1.6,1.0),(1.0,0.6)]
print("\nE2 synthetic: scale the room in its own axes; items (non-thin) resolved by each policy")
print("   metrics summed over rooms with >=3 non-thin items: items outside | NEW overlapping pairs | items total")
srcs = [nonthin(r) for r in rooms.values() if sum(not i["thin"] for i in r["items"]) >= 3]
print(f"   rooms used: {len(srcs)}")
print(f"   {'scale':>12s} " + " ".join(f"{n:>26s}" for n in POLS))
for sx, sy in SC:
    row = []
    for name, pol in POLS.items():
        o = n = t = 0
        for r in srcs:
            bp = capture(r); tgt = scaled(r["poly"], sx, sy)
            base_out, base_ov, _ = findings(resolve(bp, r["poly"], 0, pol), r["poly"])
            out, ov, _ = findings(resolve(bp, tgt, 0, pol), tgt)
            o += len(set(out) - set(base_out)); n += len(ov - base_ov); t += len(r["items"])
        row.append(f"{o:6d} | {n:6d} | {t:6d}")
    print(f"   {sx:5.2f}x{sy:4.2f}  " + " ".join(f"{x:>26s}" for x in row))

# ---- E3 real siblings, ground truth by type name
print("\nE3 real siblings: same simplified edge count and orthogonal. For each source item whose type exists in the target,")
print("   distance (mm) from the resolved centre to the nearest unused real item of that type in the target. Median over matched items.")
print("   baseline 'offset' = same offset from room centroid with the same rotation (no recipe).")
def cen(poly): return (sum(p[0] for p in poly)/len(poly), sum(p[1] for p in poly)/len(poly))
pairs = []
for A in rooms.values():
    if sum(not i["thin"] for i in A["items"]) < 3: continue
    for B in rooms.values():
        if A is B or len(A["poly"]) != len(B["poly"]) or not (orthogonal(A["poly"]) and orthogonal(B["poly"])): continue
        shared = {i["type"] for i in A["items"]} & {i["type"] for i in B["items"]}
        if len(shared) < 2: continue
        pairs.append((A, B, shared))
print(f"   pairs with >=2 shared types: {len(pairs)}")
agg = collections.defaultdict(list)
for A, B, shared in pairs:
    bp = capture(A); s = best_shift(bp, B["poly"])
    la, lb = [e["L"] for e in edges(A["poly"])], [e["L"] for e in edges(B["poly"])]
    res = {}
    for name, pol in POLS.items():
        pl = resolve(bp, B["poly"], s, pol); used = set(); errs = []
        for p in pl:
            if p["src"]["type"] not in shared: continue
            cands = [(dist(p["c"], j["c"]), idx) for idx, j in enumerate(B["items"]) if j["type"] == p["src"]["type"] and idx not in used]
            if cands: d, idx = min(cands); used.add(idx); errs.append(d)
        res[name] = errs
    # baseline: rotate onto target frame by edge mapping, keep offset from centroid
    EA, EB = edges(A["poly"]), edges(B["poly"]); k = len(EA)
    ca, cb = cen(A["poly"]), cen(B["poly"])
    ta, tb = EA[0]["t"], EB[s % k]["t"]
    ang = math.atan2(cross(ta, tb), dot(ta, tb)); co, si = math.cos(ang), math.sin(ang)
    used = set(); errs = []
    for it in A["items"]:
        if it["type"] not in shared: continue
        r = sub(it["c"], ca); pc = (cb[0] + r[0]*co - r[1]*si, cb[1] + r[0]*si + r[1]*co)
        cands = [(dist(pc, j["c"]), idx) for idx, j in enumerate(B["items"]) if j["type"] == it["type"] and idx not in used]
        if cands: d, idx = min(cands); used.add(idx); errs.append(d)
    res["offset from centroid"] = errs
    ar = (EB[0]["L"]*EB[1]["L"])/(EA[0]["L"]*EA[1]["L"])
    line = f"   {A['name'][:22]:22s} -> {B['name'][:22]:22s} area x{ar:4.2f} n={len(res['hybrid']):3d} "
    line += " ".join(f"{n[:6]}={statistics.median(e):6.0f}" if e else f"{n[:6]}=   -  " for n, e in res.items())
    print(line)
    for n, e in res.items(): agg[n] += e
print("\n   pooled over all matched items (mm): median / mean / n")
for n, e in agg.items():
    if e: print(f"   {n:22s} {statistics.median(e):7.0f} {statistics.mean(e):7.0f} {len(e):5d}")
