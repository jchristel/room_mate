#!/usr/bin/env python3
"""Face-of-wall sample levels for the 'showcase' project, drawn from the plan
screenshots that motivated the void-closure work (a lift core, a service core,
and a courtyard). Unlike gen_showcase.py — whose rooms tile edge-to-edge (wall
CENTRELINE) — every room here is inset from its neighbours by a wall-width gap
(wall FINISH FACE). That is what exercises service::areas' morphological close:
the gaps must bridge and the wall bands fill, while a genuine courtyard stays
open and is excluded from the area.

Emitted as a SEPARATE model ("campus-samples") under project "showcase", on its
own levels and under a new Building "Sample Tower", so it sits alongside the
existing North/South Tower demo without disturbing the milestones or comparison.

    python scripts/gen_samples.py        # writes data/showcase-samples.json
    curl -X POST --data-binary @data/showcase-samples.json http://127.0.0.1:5151/rooms
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen_big_plate import rect, prop  # noqa: E402

SAMPLES_TS = "2026-07-24T09:00:00Z"
WALL = 0.5           # internal wall gap between rooms (finish-face inset), feet
CORE_WALL = 0.75     # thicker gap between a core and its surrounding corridor

BUILDING = "Sample Tower"


def room(rn, name, level_id, x0, y0, x1, y1, dept, sub):
    """A face-of-wall room: a solid rectangle (no column holes — these samples
    are about wall bands and voids between rooms, not within them)."""
    return {
        "id": rn,
        "name": name,
        "level_id": level_id,
        "loops": [rect(x0, y0, x1, y1)],
        "properties": {
            "Name": prop(name),
            "RoomNumber": prop(rn),
            "Building": prop(BUILDING),
            "Department": prop(dept),
            "SubDepartment": prop(sub),
            "Area": prop(round((x1 - x0) * (y1 - y0), 1)),
        },
    }


def lift_core(lid):
    """A lift core: two columns of four lifts around a central lobby (Dept
    'Vertical Transport'), wrapped on two sides by a corridor (Dept
    'Circulation'). Internal lift walls fill at the VT department; the wall
    between the core and the corridor fills only at the Building above them."""
    rooms = []
    n = 0

    def add(*a):
        nonlocal n
        n += 1
        rooms.append(room(f"ST-LC-{n:02d}", *a))

    lift_w, lift_h = 9.5, 9.5
    rows = [j * (lift_h + WALL) for j in range(4)]   # 4 rows, wall gaps
    left_x = 0.0
    lobby_x0 = left_x + lift_w + WALL                # lobby starts one wall in
    lobby_w = 12.0
    right_x = lobby_x0 + lobby_w + WALL
    top = rows[-1] + lift_h

    for j, y0 in enumerate(rows):
        add(f"Lift {j + 1}", lid, left_x, y0, left_x + lift_w, y0 + lift_h, "Vertical Transport", "Lift")
        add(f"Lift {j + 5}", lid, right_x, y0, right_x + lift_w, y0 + lift_h, "Vertical Transport", "Lift")
    add("Lift Lobby", lid, lobby_x0, 0.0, lobby_x0 + lobby_w, top, "Vertical Transport", "Lobby")

    # Corridor wrapping the left and bottom, one CORE_WALL off the core.
    core_x1 = right_x + lift_w
    add("Corridor W", lid, left_x - CORE_WALL - 6.0, 0.0, left_x - CORE_WALL, top, "Circulation", "Corridor")
    add("Corridor S", lid, left_x - CORE_WALL - 6.0, -CORE_WALL - 6.0, core_x1, -CORE_WALL, "Circulation", "Corridor")
    return rooms


def service_core(lid):
    """A stair beside a row of mechanical rooms (Dept 'Services') next to a
    plant room of a DIFFERENT department ('Vertical Transport' again, standing
    in for an adjacent core). The Services internal walls fill within Services;
    the Services-to-core wall fills only at the Building."""
    rooms = []
    n = 0

    def add(*a):
        nonlocal n
        n += 1
        rooms.append(room(f"ST-SC-{n:02d}", *a))

    # Stair, full height on the left.
    add("Stair 5", lid, 0.0, 0.0, 14.0, 20.0, "Services", "Stair")
    # Three mech rooms in a row below, face-of-wall.
    mx = 0.0
    for k in range(3):
        add(f"Mech {k + 1}", lid, mx, -8.0 - WALL, mx + 9.0, -WALL, "Services", "Mech")
        mx += 9.0 + WALL
    # An adjacent lift (different department), one core-wall to the right of the stair.
    add("Lift 9", lid, 14.0 + CORE_WALL, 0.0, 14.0 + CORE_WALL + 9.5, 20.0, "Vertical Transport", "Lift")
    return rooms


def courtyard(lid):
    """Eight offices ringing a genuine open courtyard (Dept 'Offices'), all
    face-of-wall. The perimeter walls fill; the courtyard — far wider than a
    wall — stays open and is excluded from the department area."""
    rooms = []
    n = 0

    def add(*a):
        nonlocal n
        n += 1
        rooms.append(room(f"ST-CY-{n:02d}", *a))

    cell = 10.0
    step = cell + WALL
    # 3x3 grid of cells, centre (1,1) left empty as the courtyard (10x10 > wall).
    for i in range(3):
        for j in range(3):
            if i == 1 and j == 1:
                continue
            x0, y0 = i * step, j * step
            add(f"Office {n + 1}", lid, x0, y0, x0 + cell, y0 + cell, "Offices", "Perimeter")
    return rooms


LEVELS = [
    ("sample-lift-core", "Sample — Lift Core", 8000.0, lift_core),
    ("sample-service-core", "Sample — Service Core", 8100.0, service_core),
    ("sample-courtyard", "Sample — Courtyard", 8200.0, courtyard),
]


def main():
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    rooms = []
    for lid, _name, _elev, builder in LEVELS:
        rooms.extend(builder(lid))
    snap = {
        "schema_version": 5,
        "project": {"id": "showcase", "name": "Sample Campus"},
        "model": {"id": "campus-samples", "name": "Campus-SAMPLES", "source": "revit"},
        "snapshot": {"taken_at": SAMPLES_TS},
        "levels": [{"id": lid, "name": name, "elevation": elev} for lid, name, elev, _ in LEVELS],
        "rooms": rooms,
    }
    path = os.path.join(here, "data", "showcase-samples.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(snap, f)
    sys.stderr.write(f"{len(rooms)} rooms across {len(LEVELS)} levels -> {os.path.relpath(path, here)}\n")
    sys.stderr.write("Push:\n  curl -X POST --data-binary @data/showcase-samples.json http://127.0.0.1:5151/rooms\n")


if __name__ == "__main__":
    main()
