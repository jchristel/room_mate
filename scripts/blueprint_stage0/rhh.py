"""RHH loader for the stage 0 scripts: four interior models, rooms + FF&E by (model, room id)."""
import json, re, os, glob
from bp0 import *

BASE = "/Users/janchristel/Documents/repos/RoomMateProd/run/data/snapshots/RHH"
MODELS = ["RHH-HDR-AR-MDL-HOS-INT_LG-G", "RHH-HDR-AR-MDL-HOS-INT_L1-L2",
          "RHH-HDR-AR-MDL-HOS-INT_L3-L5", "RHH-HDR-AR-MDL-HOS-INT_L6-RF"]
CODE = re.compile(r"[-\s]*[A-Z]*\d[\w.\-]*$")


def latest(d):
    return sorted(glob.glob(d + "/2*.json"))[-1]


def base_name(n): return CODE.sub("", n).strip() or n


def load_rhh():
    rooms = {}; skipped = collections.Counter()
    for m in MODELS:
        R = json.load(open(latest(f"{BASE}/{m}")))
        lv = {l["id"]: l for l in R["levels"]}
        for r in R["rooms"]:
            if not r.get("loops"): skipped["room no loops"] += 1; continue
            raw = [(p["x"] * FT, p["y"] * FT) for p in r["loops"][0]["points"]]
            if len(raw) < 3: skipped["room degenerate"] += 1; continue
            poly = simplify(raw, float(os.environ.get("TOL", "30")))
            if len(poly) < 3 or abs(signed_area(poly)) < 1e6: skipped["room degenerate"] += 1; continue
            rooms[(m, r["id"])] = dict(id=r["id"], model=m, name=r["name"], base=base_name(r["name"]), raw=raw,
                                       poly=poly, level=lv.get(r["level_id"], {}).get("name"), items=[])
        F = json.load(open(latest(f"{BASE}/{m}/ffe")))
        for it in F["ffe"]:
            key = (m, it.get("room"))
            if key not in rooms: skipped["no room"] += 1; continue
            if it.get("super_component_id"): skipped["nested"] += 1; continue
            if not it.get("loops"): skipped["no footprint"] += 1; continue
            P = [(p["x"] * FT, p["y"] * FT) for p in it["loops"][0]["points"]]
            if len(P) < 3: skipped["no footprint"] += 1; continue
            c = (sum(p[0] for p in P) / len(P), sum(p[1] for p in P) / len(P))
            f = unit((it["facing"]["x"], it["facing"]["y"])) if it.get("facing") else (1.0, 0.0)
            v = (-f[1], f[0])
            hu = max(abs(dot(sub(p, c), f)) for p in P); hv = max(abs(dot(sub(p, c), v)) for p in P)
            rooms[key]["items"].append(dict(id=it["id"], type=it["type_name"], cat=it["category"], c=c, f=f,
                                            hu=hu, hv=hv, thin=2 * min(hu, hv) < 100))
    return rooms, skipped


def reflect(room):
    """Mirror a room and its items across the x axis (a reflection, so winding flips)."""
    fl = lambda p: (p[0], -p[1])
    poly = simplify([fl(p) for p in room["poly"]])
    items = [dict(i, c=fl(i["c"]), f=(i["f"][0], -i["f"][1])) for i in room["items"]]
    return dict(room, poly=poly, items=items)
