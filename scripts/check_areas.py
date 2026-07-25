#!/usr/bin/env python3
"""Diagnostic harness for `service::areas` footprints, run against a live server.

This started life as a throwaway script and is committed because it earned it:
it found, in seconds, a footprint vertex at y = -1,052,070 ft that inflated one
department's area 200-fold -- something visual inspection on the level people
happened to be looking at had missed entirely. The lesson it encodes is the
reason it takes no `--level` argument: **it checks every level, always.** The
spike was on LEVEL 00 while all the visual work had been on LEVEL 01.

It is a *diagnostic*, not a test. It reports; it does not assume a threshold is
a specification. The two invariants cheap enough to state absolutely -- no
spikes, and simple rings -- are also inline Rust tests in `areas.rs`; the other
checks need a whole real model to mean anything, which is exactly what a unit
test cannot supply.

    python scripts/check_areas.py --project "House A"
    python scripts/check_areas.py --project "House A" --base http://localhost:3000

Exit status is 1 if any check fails, so it can gate a run.

Stdlib only, on purpose: this has to work on whatever machine has the model on
it, and a diagnostic that needs an environment set up first does not get run.
The one place that costs something is the sibling-overlap check -- see
`overlap_area` for what it can and cannot tell you.
"""

import argparse
import json
import math
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections import defaultdict

# ---------------------------------------------------------------------------
# thresholds -- all advisory, all reported with the measured value so a reader
# judges the number rather than the verdict
# ---------------------------------------------------------------------------

# A footprint vertex further outside its own rooms' bounding box than this is a
# spike. Generous: a legitimate footprint extends past its rooms by at most the
# wall it filled, and the fills are clipped to the wall zone, so anything beyond
# a wall plus slack is geometry that came from nowhere.
SPIKE_SLACK_FT = 1.2

# footprint / summed net room area. Below 1.0 means area went missing; a
# finish-face model legitimately runs above 1.0 because the footprint includes
# the wall bands the rooms exclude, and how far above depends on how small the
# rooms are. 1.6 is loose enough for a floor of small rooms with thick walls and
# still catches a footprint that has swallowed a neighbour.
AREA_RATIO_LO = 0.95
AREA_RATIO_HI = 1.6

# Angular tolerance for "the same direction", matching `areas::DIR_TOL_RAD`.
DIR_TOL_RAD = 0.02

# Ignore an invented edge shorter than this. Boolean ops leave sub-millimetre
# stubs at junctions; they are not the 45-degree chamfers this check is for.
MIN_INVENTED_EDGE_FT = 0.05

# Sibling overlap below this is reported as a note rather than a failure -- see
# `overlap_area`, which is a sampler and cannot resolve smaller than its grid.
OVERLAP_TOLERANCE_SQFT = 0.5


def fetch(base, path, params):
    url = base.rstrip("/") + path
    query = {k: v for k, v in params.items() if v is not None}
    if query:
        url += "?" + urllib.parse.urlencode(query)
    try:
        with urllib.request.urlopen(url, timeout=600) as response:
            if response.status == 204:
                return None
            return json.load(response)
    except urllib.error.URLError as exc:
        sys.exit(f"could not reach {url}: {exc}")


# ---------------------------------------------------------------------------
# small geometry helpers -- rings are lists of [x, y], closing point dropped
# ---------------------------------------------------------------------------


def ring_area(ring):
    """Signed shoelace area. Sign is orientation; callers take abs()."""
    total = 0.0
    for i, (x0, y0) in enumerate(ring):
        x1, y1 = ring[(i + 1) % len(ring)]
        total += x0 * y1 - x1 * y0
    return total / 2.0


def polygon_area(polygon):
    """Exterior minus holes -- the same net figure the server reports."""
    area = abs(ring_area(polygon["exterior"]))
    for hole in polygon.get("holes", []):
        area -= abs(ring_area(hole))
    return area


def footprint_area(polygons):
    return sum(polygon_area(p) for p in polygons)


def rings_of(polygons):
    for polygon in polygons:
        yield polygon["exterior"]
        for hole in polygon.get("holes", []):
            yield hole


def bbox(points):
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return min(xs), min(ys), max(xs), max(ys)


def segments(ring):
    for i in range(len(ring)):
        yield ring[i], ring[(i + 1) % len(ring)]


def direction(p, q):
    """Line orientation in [0, pi), or None for a zero-length edge."""
    dx, dy = q[0] - p[0], q[1] - p[1]
    if math.hypot(dx, dy) < 1e-9:
        return None
    return math.atan2(dy, dx) % math.pi


def angular_diff(a, b):
    d = abs(a - b) % math.pi
    return min(d, math.pi - d)


def point_in_polygon(x, y, polygon):
    """Even-odd ray cast, holes honoured, with a **half-open** rule.

    The half-open convention (`y0 <= y < y1`) is the load-bearing detail. The
    original throwaway version of this script reported sibling overlaps that a
    Rust-side intersection said were empty, and the most likely explanation was
    exactly this: a sampling rule that counts a point lying on a *shared*
    boundary as interior to both neighbours turns every shared wall into a
    phantom overlap the width of one grid step. With the half-open rule a
    boundary point falls to exactly one side, so a common edge contributes
    nothing.
    """
    inside = False
    for ring in [polygon["exterior"]] + list(polygon.get("holes", [])):
        for (x0, y0), (x1, y1) in segments(ring):
            if (y0 <= y) != (y1 <= y):
                t = (y - y0) / (y1 - y0)
                if x < x0 + t * (x1 - x0):
                    inside = not inside
    return inside


def in_footprint(x, y, polygons):
    return any(point_in_polygon(x, y, p) for p in polygons)


def overlap_area(a_polys, b_polys, step=0.25):
    """Estimated area of the true polygon intersection of two footprints.

    A grid sampler, not an exact boolean, because doing this properly in the
    standard library means writing a robust polygon clipper and a diagnostic is
    not the place for one. Two consequences a reader must hold on to:

    * it resolves nothing finer than `step**2`, so a genuinely tiny overlap and
      zero are indistinguishable -- hence `OVERLAP_TOLERANCE_SQFT`;
    * it is an estimate of a real quantity, so a *large* reading is trustworthy
      and a small one is not.

    A bounding-box reject comes first, and it is not merely an optimisation:
    concave footprints' bounding boxes overlap legitimately all the time, which
    is exactly why the bbox alone was never an acceptable answer to this check.
    """
    if not a_polys or not b_polys:
        return 0.0
    ax0, ay0, ax1, ay1 = bbox([pt for p in a_polys for pt in p["exterior"]])
    bx0, by0, bx1, by1 = bbox([pt for p in b_polys for pt in p["exterior"]])
    x0, y0 = max(ax0, bx0), max(ay0, by0)
    x1, y1 = min(ax1, bx1), min(ay1, by1)
    if x1 <= x0 or y1 <= y0:
        return 0.0

    # Offset by an irrational fraction of a step so samples avoid landing on the
    # axis-aligned edges that dominate a building, where any tie-break rule is
    # doing more work than it should have to.
    offset = step * 0.31830988618  # 1/pi
    hits = 0
    y = y0 + offset
    while y < y1:
        x = x0 + offset
        while x < x1:
            if in_footprint(x, y, a_polys) and in_footprint(x, y, b_polys):
                hits += 1
            x += step
        y += step
    return hits * step * step


# ---------------------------------------------------------------------------
# the checks
# ---------------------------------------------------------------------------


class Report:
    def __init__(self):
        self.failures = []
        self.notes = []

    def fail(self, check, detail):
        self.failures.append((check, detail))

    def note(self, detail):
        self.notes.append(detail)


def path_key(path):
    return tuple((t.get("code"), t.get("name"), t.get("undefined")) for t in path)


def path_label(path):
    parts = []
    for tier in path:
        parts.append(tier.get("name") or tier.get("code") or "<undefined>")
    return " / ".join(parts)


def check_spikes(report, group, room_points, wall_ft):
    """1. No footprint vertex escapes its own rooms' bounding box by > a wall."""
    if not room_points:
        return
    rx0, ry0, rx1, ry1 = bbox(room_points)
    slack = max(wall_ft, SPIKE_SLACK_FT)
    worst = 0.0
    worst_point = None
    for ring in rings_of(group["polygons"]):
        for x, y in ring:
            escape = max(rx0 - x, x - rx1, ry0 - y, y - ry1, 0.0)
            if escape > worst:
                worst, worst_point = escape, (x, y)
    if worst > slack:
        report.fail(
            "spike",
            f"{path_label(group['path'])}: vertex {worst_point} is {worst:.1f} ft "
            f"outside its own rooms' bbox (allowed {slack:.2f})",
        )


def check_area_ratio(report, group, net_room_area):
    """2. Footprint over summed net room area sits in a plausible band."""
    if net_room_area <= 0:
        return
    ratio = group["area"] / net_room_area
    if not (AREA_RATIO_LO <= ratio <= AREA_RATIO_HI):
        report.fail(
            "area",
            f"{path_label(group['path'])}: footprint {group['area']:.1f} ft^2 is "
            f"{ratio:.2f}x its rooms' net {net_room_area:.1f} ft^2 "
            f"(expected {AREA_RATIO_LO}-{AREA_RATIO_HI})",
        )


def check_sibling_overlap(report, groups):
    """3. Two groups at the same tier must not both claim the same area."""
    for i in range(len(groups)):
        for j in range(i + 1, len(groups)):
            a, b = groups[i], groups[j]
            if len(a["path"]) != len(b["path"]):
                continue
            area = overlap_area(a["polygons"], b["polygons"])
            if area > OVERLAP_TOLERANCE_SQFT:
                report.fail(
                    "overlap",
                    f"{path_label(a['path'])} and {path_label(b['path'])} both claim "
                    f"~{area:.2f} ft^2",
                )
            elif area > 0:
                report.note(
                    f"  (sub-tolerance overlap {area:.2f} ft^2 between "
                    f"{path_label(a['path'])} and {path_label(b['path'])} -- at or below "
                    f"what the sampler can resolve)"
                )


def check_invented_directions(report, group, input_dirs):
    """4. No output edge runs at an orientation the input rooms never had.

    This is the de-bevel's own rule, checked from outside: the close cuts
    corners with a chord at a direction (usually 45 degrees) no wall had, so an
    edge direction absent from the rooms is an artifact by definition. It is
    stated against the *input* rather than against "axis-aligned" so a building
    drawn on the diagonal passes for the right reason.
    """
    worst = None
    for ring in rings_of(group["polygons"]):
        for p, q in segments(ring):
            if math.hypot(q[0] - p[0], q[1] - p[1]) < MIN_INVENTED_EDGE_FT:
                continue
            d = direction(p, q)
            if d is None:
                continue
            if not any(angular_diff(d, known) <= DIR_TOL_RAD for known in input_dirs):
                length = math.hypot(q[0] - p[0], q[1] - p[1])
                if worst is None or length > worst[0]:
                    worst = (length, p, q, math.degrees(d))
    if worst:
        length, p, q, degrees = worst
        report.fail(
            "direction",
            f"{path_label(group['path'])}: edge {p}->{q} runs at {degrees:.1f} deg, "
            f"a direction no room has ({length:.2f} ft long)",
        )


def check_rings(report, group):
    """5. Every ring is simple and has at least three distinct points."""
    for ring in rings_of(group["polygons"]):
        if len(ring) < 3:
            report.fail("ring", f"{path_label(group['path'])}: ring has {len(ring)} points")
            continue
        n = len(ring)
        for i in range(n):
            for j in range(i + 2, n):
                if i == 0 and j == n - 1:
                    continue  # first and last edge share a vertex
                if proper_crossing(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]):
                    report.fail(
                        "ring",
                        f"{path_label(group['path'])}: ring self-intersects near {ring[i]}",
                    )
                    return


def proper_crossing(a, b, c, d):
    def cross(o, p, q):
        return (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])

    d1, d2 = cross(a, b, c), cross(a, b, d)
    d3, d4 = cross(c, d, a), cross(c, d, b)
    return (
        d1 != 0.0 and d2 != 0.0 and d3 != 0.0 and d4 != 0.0
        and (d1 > 0) != (d2 > 0) and (d3 > 0) != (d4 > 0)
    )


def check_additivity(report, groups):
    """6. A parent's area is at least the sum of its children's.

    Not in the original script, and free: it is arithmetic on numbers the server
    already reports, no geometry. It is the property the wall-zone partition
    exists to guarantee -- a parent equals its children plus the bands it is the
    first tier to enclose -- so the one-sided form is the real check. A parent
    *below* its children means area is being double-counted somewhere below it.
    Groups a Case-A exclusion withholds are skipped: they are deliberately not
    part of their parent, which is the whole point of the exclusion.
    """
    by_path = {path_key(g["path"]): g for g in groups}
    children = defaultdict(list)
    for group in groups:
        if len(group["path"]) < 2 or not group.get("counted_upward", True):
            continue
        children[path_key(group["path"][:-1])].append(group)

    for parent_key, kids in children.items():
        parent = by_path.get(parent_key)
        if parent is None:
            continue
        total = sum(k["area"] for k in kids)
        # 0.5% slack: the close's bevel at the level's outer boundary is a known,
        # bounded residual (see areas.rs), and it is proportional to perimeter.
        if parent["area"] < total * 0.995:
            report.fail(
                "additivity",
                f"{path_label(parent['path'])} is {parent['area']:.1f} ft^2 but its "
                f"{len(kids)} children sum to {total:.1f} ft^2",
            )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="http://localhost:3000", help="server base URL")
    parser.add_argument("--project", required=True, help="project id")
    parser.add_argument("--building", default=None)
    parser.add_argument("--milestone", default=None)
    parser.add_argument(
        "--skip-overlap",
        action="store_true",
        help="skip check 3 (the sampler is the slow one on a large floor)",
    )
    args = parser.parse_args()

    scope = {"building": args.building, "milestone": args.milestone}
    quoted = urllib.parse.quote(args.project, safe="")
    areas = fetch(args.base, f"/projects/{quoted}/areas", scope)
    rooms = fetch(args.base, "/rooms", dict(scope, project=args.project))
    if areas is None or rooms is None:
        sys.exit(f"no data stored for project {args.project!r}")

    levels = {level["id"]: level["name"] for level in areas["levels"]}
    gaps = areas.get("wall_gap_by_level", {})
    standard = areas.get("measurement_standard")
    print(f"project {args.project!r}: {len(levels)} level(s), {len(areas['groups'])} group(s)")
    print(f"  measurement standard: {standard or '(undeclared)'}")

    # Rooms per level, and the classification path each one resolved to, so a
    # group can be compared against the rooms it was actually built from.
    rooms_by_level = defaultdict(list)
    for room in rooms["rooms"]:
        rooms_by_level[room["level_id"]].append(room)

    report = Report()
    for level_id, level_name in sorted(levels.items(), key=lambda kv: kv[1]):
        level_groups = [g for g in areas["groups"] if g["level_id"] == level_id]
        level_rooms = rooms_by_level.get(level_id, [])
        gap = gaps.get(level_id, 0.0)
        regime = "centreline" if gap == 0 else "finish face"
        print(
            f"\n{level_name}: {len(level_rooms)} room(s), {len(level_groups)} group(s), "
            f"wall gap {gap:g} ft ({regime})"
        )
        if not level_groups:
            continue

        # Every input edge direction on this level -- the reference for check 4.
        input_dirs = []
        room_points = []
        for room in level_rooms:
            loops = room.get("loops") or []
            if not loops:
                continue
            outer = [(p["x"], p["y"]) for p in loops[0]["points"]]
            room_points.extend(outer)
            for p, q in segments(outer):
                d = direction(p, q)
                if d is not None and not any(angular_diff(d, k) <= DIR_TOL_RAD for k in input_dirs):
                    input_dirs.append(d)

        # Net room area per classification prefix, for check 2.
        net_by_prefix = defaultdict(float)
        for room in level_rooms:
            loops = room.get("loops") or []
            if not loops:
                continue
            outer = [(p["x"], p["y"]) for p in loops[0]["points"]]
            area = abs(ring_area(outer))
            path = room.get("classification") or []
            for depth in range(1, len(path) + 1):
                net_by_prefix[path_key(path[:depth])] += area

        for group in level_groups:
            own_rooms = [
                pt
                for room in level_rooms
                if path_key((room.get("classification") or [])[: len(group["path"])]) == path_key(group["path"])
                for loop in (room.get("loops") or [])[:1]
                for pt in [(p["x"], p["y"]) for p in loop["points"]]
            ]
            check_spikes(report, group, own_rooms, gap)
            check_area_ratio(report, group, net_by_prefix.get(path_key(group["path"]), 0.0))
            check_invented_directions(report, group, input_dirs)
            check_rings(report, group)

        check_additivity(report, level_groups)
        if not args.skip_overlap:
            check_sibling_overlap(report, level_groups)

        for group in sorted(level_groups, key=lambda g: (len(g["path"]), path_label(g["path"]))):
            net = net_by_prefix.get(path_key(group["path"]), 0.0)
            ratio = f"{group['area'] / net:.2f}x" if net else "   -  "
            flag = "" if group.get("counted_upward", True) else "  [not counted upward]"
            print(
                f"  {'  ' * (len(group['path']) - 1)}{path_label(group['path']):<44} "
                f"{group['area']:>10.1f} ft^2  {ratio}{flag}"
            )

    print()
    for note in report.notes:
        print(note)
    if report.failures:
        print(f"\n{len(report.failures)} failure(s):")
        for check, detail in report.failures:
            print(f"  [{check}] {detail}")
        return 1
    print("all checks clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
