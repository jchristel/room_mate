"""Stage 0 throwaway: recipes (capture) and a resolver, measured on House A.

Not product code. Units: the snapshots are feet; everything here is mm.
No mirroring (decision 7), so edge mapping is a cyclic shift only.
"""
import json, math, statistics, collections, itertools, sys

FT = 304.8
ROOT = "/Users/janchristel/Documents/repos/room_mate/data/snapshots/House A/Building_BF_Framing_jan.r.christel"


# ---------- geometry ----------
def dist(a, b): return math.hypot(a[0] - b[0], a[1] - b[1])
def sub(a, b): return (a[0] - b[0], a[1] - b[1])
def dot(a, b): return a[0] * b[0] + a[1] * b[1]
def cross(a, b): return a[0] * b[1] - a[1] * b[0]
def unit(a):
    l = math.hypot(*a)
    return (a[0] / l, a[1] / l) if l else (1.0, 0.0)


def signed_area(p):
    return sum(cross(p[i], p[(i + 1) % len(p)]) for i in range(len(p))) / 2


def seg_dist(pt, a, b):
    ab = sub(b, a); l2 = dot(ab, ab)
    t = 0 if l2 == 0 else max(0, min(1, dot(sub(pt, a), ab) / l2))
    return dist(pt, (a[0] + ab[0] * t, a[1] + ab[1] * t))


def line_dist(pt, a, b):
    ab = sub(b, a); l = math.hypot(*ab)
    return dist(pt, a) if l == 0 else abs(cross(ab, sub(pt, a))) / l


def simplify(p, tol=30.0):
    p = list(p)
    changed = True
    while changed and len(p) > 3:
        changed = False
        q = []
        for a in p:
            if not q or dist(a, q[-1]) > 1.0: q.append(a)
        if len(q) > 1 and dist(q[0], q[-1]) <= 1.0: q.pop()
        p = q
        n = len(p)
        for i in range(n):
            if line_dist(p[i], p[i - 1], p[(i + 1) % n]) < tol:
                p.pop(i); changed = True; break
    if signed_area(p) < 0: p.reverse()
    return p


def inside(pt, poly, tol=20.0):
    if any(seg_dist(pt, poly[i], poly[(i + 1) % len(poly)]) < tol for i in range(len(poly))):
        return True
    c = False
    n = len(poly)
    for i in range(n):
        a, b = poly[i], poly[(i + 1) % n]
        if (a[1] > pt[1]) != (b[1] > pt[1]) and pt[0] < (b[0] - a[0]) * (pt[1] - a[1]) / (b[1] - a[1]) + a[0]:
            c = not c
    return c


def overlap(P, Q, pen=5.0):
    """SAT on convex polygons; True when they interpenetrate by more than pen mm."""
    best = 1e18
    for poly in (P, Q):
        for i in range(len(poly)):
            e = sub(poly[(i + 1) % len(poly)], poly[i]); ax = unit((-e[1], e[0]))
            pa = [dot(x, ax) for x in P]; pb = [dot(x, ax) for x in Q]
            o = min(max(pa), max(pb)) - max(min(pa), min(pb))
            if o <= pen: return False
            best = min(best, o)
    return True


def edges(poly):
    k = len(poly); out = []
    for i in range(k):
        a, b = poly[i], poly[(i + 1) % k]
        t = unit(sub(b, a)); n = (-t[1], t[0])  # CCW => n points into the room
        out.append(dict(a=a, b=b, t=t, n=n, L=dist(a, b)))
    return out


def depth_at(poly, i, pt):
    """Distance from pt on edge i, along its inward normal, to the next wall."""
    E = edges(poly); e = E[i]
    o = (pt[0] + e["n"][0] * 0.5, pt[1] + e["n"][1] * 0.5)
    best = 1e18
    for j, f in enumerate(E):
        if j == i: continue
        d = (f["b"][0] - f["a"][0], f["b"][1] - f["a"][1])
        den = cross(e["n"], d)
        if abs(den) < 1e-9: continue
        w = sub(f["a"], o)
        s = cross(w, d) / den; u = cross(w, e["n"]) / den
        if s > 0 and -1e-6 <= u <= 1 + 1e-6: best = min(best, s)
    return best if best < 1e17 else e["L"]


def orthogonal(poly, tol=1.5):
    E = edges(poly); base = math.degrees(math.atan2(E[0]["t"][1], E[0]["t"][0])) % 90
    for e in E:
        a = math.degrees(math.atan2(e["t"][1], e["t"][0])) % 90
        d = min(abs(a - base), 90 - abs(a - base))
        if d > tol: return False
    return True


# ---------- data ----------
def load():
    R = json.load(open(ROOT + "/2026-08-23T00-44-17.490000Z.json"))
    F = json.load(open(ROOT + "/ffe/2026-09-05T12-57-49.149000Z.json"))
    rooms = {}
    for r in R["rooms"]:
        raw = [(p["x"] * FT, p["y"] * FT) for p in r["loops"][0]["points"]]
        rooms[r["id"]] = dict(id=r["id"], name=r["name"], raw=raw, poly=simplify(raw), level=r["level_id"], items=[])
    skipped = collections.Counter()
    for it in F["ffe"]:
        rid = it.get("room")
        if rid not in rooms: skipped["no room"] += 1; continue
        if it.get("super_component_id"): skipped["nested component"] += 1; continue
        if not it.get("loops"): skipped["no footprint"] += 1; continue
        P = [(p["x"] * FT, p["y"] * FT) for p in it["loops"][0]["points"]]
        c = (sum(p[0] for p in P) / len(P), sum(p[1] for p in P) / len(P))
        f = unit((it["facing"]["x"], it["facing"]["y"])) if it.get("facing") else (1.0, 0.0)
        v = (-f[1], f[0])
        hu = max(abs(dot(sub(p, c), f)) for p in P); hv = max(abs(dot(sub(p, c), v)) for p in P)
        rooms[rid]["items"].append(dict(id=it["id"], type=it["type_name"], cat=it["category"], c=c, f=f, hu=hu, hv=hv,
                                        thin=2 * min(hu, hv) < 100))
    return rooms, skipped


def box(c, f, hu, hv):
    v = (-f[1], f[0])
    return [(c[0] + sx * hu * f[0] + sy * hv * v[0], c[1] + sx * hu * f[1] + sy * hv * v[1])
            for sx, sy in ((1, 1), (-1, 1), (-1, -1), (1, -1))]


# ---------- capture ----------
def capture(room):
    poly = room["poly"]; E = edges(poly); items = []
    for it in room["items"]:
        # an edge the item lies "beside": its projection falls within the edge, nearest first
        valid = [j for j in range(len(E)) if 0 <= dot(sub(it["c"], E[j]["a"]), E[j]["t"]) <= E[j]["L"]
                 and dot(sub(it["c"], E[j]["a"]), E[j]["n"]) >= -50]
        corner = not valid
        i = (min(valid, key=lambda j: abs(dot(sub(it["c"], E[j]["a"]), E[j]["n"]))) if valid
             else min(range(len(E)), key=lambda j: seg_dist(it["c"], E[j]["a"], E[j]["b"])))
        e = E[i]; r = sub(it["c"], e["a"])
        along = dot(r, e["t"]); off = dot(r, e["n"])
        ang = math.atan2(cross(e["n"], it["f"]), dot(e["n"], it["f"]))
        foot = (e["a"][0] + e["t"][0] * along, e["a"][1] + e["t"][1] * along)
        depth = depth_at(poly, i, foot)
        # half extent of the box along the edge tangent and normal
        B = box(it["c"], it["f"], it["hu"], it["hv"])
        ht = max(abs(dot(sub(p, it["c"]), e["t"])) for p in B); hn = max(abs(dot(sub(p, it["c"]), e["n"])) for p in B)
        items.append(dict(it, corner=corner, edge=i, along=along, off=off, ang=ang, depth=depth, ht=ht, hn=hn, L=e["L"]))
    return dict(k=len(poly), lens=[e["L"] for e in E], items=items)


# ---------- resolve ----------
END_GAP, WALL_GAP = 300.0, 150.0  # "near a corner", "flush": the hybrid rule's thresholds


def resolve(bp, poly, shift, pol):
    E = edges(poly); out = []
    for it in bp["items"]:
        j = (it["edge"] + shift) % len(E); e = E[j]
        gs, ge = it["along"] - it["ht"], it["L"] - it["along"] - it["ht"]
        if pol["along"] == "fixed_start": al = it["along"]
        elif pol["along"] == "percent": al = it["along"] / it["L"] * e["L"]
        elif pol["along"] == "fixed_near":
            al = it["along"] if it["along"] <= it["L"] / 2 else e["L"] - (it["L"] - it["along"])
        else:  # hybrid
            if gs <= END_GAP and gs <= ge: al = it["along"]
            elif ge <= END_GAP: al = e["L"] - (it["L"] - it["along"])
            else: al = it["along"] / it["L"] * e["L"]
        foot = (e["a"][0] + e["t"][0] * al, e["a"][1] + e["t"][1] * al)
        gap = it["off"] - it["hn"]
        if pol["off"] == "fixed" or gap <= WALL_GAP: of = it["off"]
        else: of = it["off"] / it["depth"] * depth_at(poly, j, foot)
        c = (foot[0] + e["n"][0] * of, foot[1] + e["n"][1] * of)
        ca, sa = math.cos(it["ang"]), math.sin(it["ang"])
        f = (e["n"][0] * ca - e["n"][1] * sa, e["n"][0] * sa + e["n"][1] * ca)
        out.append(dict(src=it, c=c, f=f))
    return out


def findings(placed, poly):
    boxes = [box(p["c"], p["f"], p["src"]["hu"], p["src"]["hv"]) for p in placed]
    out = [i for i, b in enumerate(boxes) if not all(inside(pt, poly) for pt in b)]
    ov = {(i, j) for i in range(len(boxes)) for j in range(i + 1, len(boxes)) if overlap(boxes[i], boxes[j])}
    return out, ov, boxes


def best_shift(bp, poly):
    L = [e["L"] for e in edges(poly)]; k = len(L)
    return min(range(k), key=lambda s: sum(abs(math.log(L[(j + s) % k] / bp["lens"][j])) for j in range(k)))


POLS = {
    "fixed(start)": dict(along="fixed_start", off="fixed"),
    "fixed(near end)": dict(along="fixed_near", off="fixed"),
    "percent": dict(along="percent", off="percent"),
    "hybrid": dict(along="hybrid", off="hybrid"),
}


def scaled(poly, sx, sy):
    E = edges(poly); ct = (sum(p[0] for p in poly) / len(poly), sum(p[1] for p in poly) / len(poly))
    t, n = E[0]["t"], E[0]["n"]
    out = []
    for p in poly:
        r = sub(p, ct); u, v = dot(r, t) * sx, dot(r, n) * sy
        out.append((ct[0] + u * t[0] + v * n[0], ct[1] + u * t[1] + v * n[1]))
    return out
