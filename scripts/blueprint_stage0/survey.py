from bp0 import *
rooms, skipped = load()
print("skipped items:", dict(skipped))
print("rooms:", len(rooms), " items attributed with footprint:", sum(len(r["items"]) for r in rooms.values()))
print()
print(f"{'room':32s} raw simp  ortho  rect  items  L1(m) L2(m)  baselineOverlap  outside")
shapes = collections.Counter()
for r in sorted(rooms.values(), key=lambda r: r["name"]):
    E = edges(r["poly"]); k = len(E); o = orthogonal(r["poly"])
    shapes[(k, o)] += 1
    bp = capture(r)
    boxes = [box(i["c"], i["f"], i["hu"], i["hv"]) for i in r["items"]]
    ov = sum(1 for a in range(len(boxes)) for b in range(a+1, len(boxes)) if overlap(boxes[a], boxes[b]))
    out = sum(1 for b in boxes if not all(inside(p, r["poly"]) for p in b))
    xs = [e["L"]/1000 for e in E[:2]]
    print(f"{r['name']:32s} {len(r['raw']):3d} {k:4d}  {str(o):5s}  {str(k==4 and o):5s} {len(r['items']):4d}  {xs[0]:5.1f} {xs[1]:5.1f}  {ov:6d}  {out:6d}")
print()
print("(simplified edge count, orthogonal):", dict(sorted(shapes.items())))
