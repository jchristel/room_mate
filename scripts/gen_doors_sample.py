#!/usr/bin/env python3
"""Translate the raw duHast door export (`scripts/fixtures/doors-raw.json`) into the server's
v2 doors contract, for pushing at House A.

This is the *server-side* twin of `extractor/pyRevit/post_doors.py`: same
translation, run over a captured export instead of a live Revit document. It
exists so the doors pipeline can be exercised end to end without Revit — the
House A rooms snapshot and this door set are the same model, so every
`from_room`/`to_room` here resolves against rooms already in the store.

    python scripts/gen_doors_sample.py    # writes data/house-a-doors.json
    curl -X POST -H "Content-Type: application/json" \
         --data-binary @data/house-a-doors.json http://127.0.0.1:5151/doors

Two things it does that a naive field-copy would not, both explained at their
call sites: it drops the degenerate `1e30` footprint Revit emits for a door
family with no 3D geometry, and it collapses the per-phase `from_room`/`to_room`
arrays to the single reference the contract carries.
"""
import io
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

RAW = os.path.join(ROOT, "scripts", "fixtures", "doors-raw.json")
OUT = os.path.join(ROOT, "data", "house-a-doors.json")

SCHEMA_VERSION = 2
PROJECT = {"id": "House A", "name": "House A"}
MODEL = {"id": "Building_BF_Framing_jan.r.christel", "name": "Building_BF_Framing_jan.r.christel", "source": "revit"}
# The export's own header timestamp ("2026_07_29_20_03_41"), as the RFC3339 UTC
# id the contract requires. When the model was READ, not when it was pushed.
TAKEN_AT = "2026-07-29T20:03:41Z"
# Every door in this export carries `phasing.created = "New Construction"`.
PHASE = "New Construction"

# Revit's *uninitialized* BoundingBoxXYZ: min +1e30, max -1e30. duHast returns
# one for a door family with no 3D geometry, and its own "did we get a box" and
# "is the loop non-empty" guards both pass -- so the bad value arrives looking
# like a real footprint rather than an absent one. Anything at this magnitude is
# the sentinel, never a coordinate: House A's real geometry spans ~0..107 feet.
SENTINEL_MAGNITUDE = 1e20


def properties_to_map(container):
    """duHast's [{name, value, storage_type}, ...] -> {name: {value,
    storage_type}}. One generic transform, no per-field logic: which names are
    'builtin' is a server-side settings concern, not this script's."""
    out = {}
    for prop in container.get("properties", []) or []:
        name = prop.get("name")
        if not name:
            continue
        value = prop.get("value")
        out[name] = {
            "value": "" if value is None else str(value),
            "storage_type": prop.get("storage_type"),
        }
    return out


def is_degenerate(loop):
    """Whether a loop is Revit's empty-bounding-box sentinel rather than real
    geometry. Checked by magnitude rather than by equality with 1e30, because
    the value arrives through a float round-trip and the point is to catch the
    class, not one bit pattern."""
    return any(abs(coord) >= SENTINEL_MAGNITUDE for point in loop for coord in point)


def loops_from_polygon(polygons):
    """The room `loops` convention verbatim: [0] outer, [1..] holes, decimal
    feet, model space, Y up.

    Returns [] for a door with no usable footprint. That is a real state the
    contract carries deliberately (`Door.loops`): the door still has properties
    and room references, so QA must see it -- dropping it would silently lose a
    door->room link, and inventing a footprint from the sentinel would poison
    every consumer with a polygon 1e30 feet across."""
    if not polygons:
        return []
    poly = polygons[0]
    outer = poly.get("outer_loop") or []
    if not outer or is_degenerate(outer):
        return []
    loops = [{"points": [{"x": float(p[0]), "y": float(p[1])} for p in outer]}]
    for inner in poly.get("inner_loops") or []:
        if inner and not is_degenerate(inner):
            loops.append({"points": [{"x": float(p[0]), "y": float(p[1])} for p in inner]})
    return loops


def room_reference(door, key):
    """One side's room id, or None.

    The raw export carries an ARRAY because it holds one entry per phase. This
    push is scoped to a single phase, so at most one entry survives and the
    array collapses to the contract's `Option<String>`. The live extractor does
    better than this -- it reads `FamilyInstance.FromRoom[phase]`, which takes
    the phase and answers exactly one room -- but a captured export has no
    document to ask, and every door in this file has at most one entry per side.

    None is an external door: normal, not missing. 6 of these 26 are one-sided."""
    refs = door.get(key) or []
    if not refs:
        return None
    if len(refs) > 1:
        raise ValueError(
            "door {} has {} {} entries; this export was expected to be single-phase".format(
                door["instance_properties"]["id"], len(refs), key
            )
        )
    return str(refs[0]["room_id"])


def translate(raw):
    doors, no_footprint = [], []
    for door in raw.get("door", []):
        instance = door["instance_properties"]
        type_props = door["type_properties"]
        loops = loops_from_polygon(door.get("polygon"))
        door_id = str(instance["id"])
        if not loops:
            no_footprint.append(door_id)
        doors.append({
            "id": door_id,
            "level_id": str(door["level"]["id"]),
            "loops": loops,
            "from_room": room_reference(door, "from_room"),
            "to_room": room_reference(door, "to_room"),
            "type_id": str(type_props["id"]),
            "type_name": type_props["name"],
            "properties": properties_to_map(instance),
            "type_properties": properties_to_map(type_props),
        })

    # v2 carries a `models` list rather than one `model` block: a push is a run,
    # and a run may hold several documents. This fixture is one document, so the
    # list has one entry -- the shape is the same either way, which is the point
    # of not special-casing the single-model case on the wire.
    model = dict(MODEL)
    model["doors"] = doors
    payload = {
        "schema_version": SCHEMA_VERSION,
        "project": PROJECT,
        "snapshot": {"taken_at": TAKEN_AT},
        "phase": PHASE,
        "models": [model],
    }
    return payload, no_footprint


def main():
    with io.open(RAW, encoding="utf-8") as f:
        raw = json.load(f)

    payload, no_footprint = translate(raw)

    # newline="" so the file is written with LF endings on Windows too --
    # .gitattributes enforces LF, and Python's default text mode would silently
    # write CRLF here (see CLAUDE.md "Traps").
    with io.open(OUT, "w", encoding="utf-8", newline="") as f:
        json.dump(payload, f, indent=2)
        f.write("\n")

    doors = payload["models"][0]["doors"]
    print("wrote {} ({} doors)".format(OUT, len(doors)))
    if no_footprint:
        print(
            "  {} door(s) have no usable footprint and were sent with empty loops: {}".format(
                len(no_footprint), ", ".join(no_footprint)
            )
        )
    external = [d["id"] for d in doors if not (d["from_room"] and d["to_room"])]
    print("  {} door(s) reference a room on one side only (external -- normal)".format(len(external)))


if __name__ == "__main__":
    main()
