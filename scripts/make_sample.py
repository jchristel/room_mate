#!/usr/bin/env python3
"""Build `samples/house-a/`: the sample project people run the server against.

**Why a script.** A sample that is copied by hand goes stale the day a stored format changes, and nobody
notices until someone opens it. Here the snapshots are the ones Revit pushed (copied as they are: they are
what the server itself wrote), and everything AUTHORED -- the blueprint, its assignment, and later its sheets
-- is made by calling the running server's own HTTP API, so it is always written in the current format.
`cargo test` (module `sample_data`) boots the sample and reads all of it, so a format change that the sample
does not survive fails CI; re-run this script to rebuild it.

    cargo build --bin roommate
    python scripts/make_sample.py                     # snapshots + blueprint + assignment
    python scripts/make_sample.py --sheets-dir <dir>  # ... and its sheets (see below)

**Sheets.** A blueprint's sheets come from Revit and cannot be invented. Push them from Revit to a server you
run, then point `--sheets-dir` at that server's `sheets/<project>` folder (the library's record files) and name
the ones to keep with `--sheet-numbers`. With sheets the blueprint is made the way the Revit button makes it
(`POST .../blueprints/from-revit`), so the sheets arrive bound to it; `--generic-sheet-numbers` adds sheets
bound to nothing. Without `--sheets-dir` the blueprint is made from the capture alone.

Stdlib only. Nothing outside `samples/house-a/` and a temp folder is written.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROJECT = "House A"
MODEL = "Building_BF_Framing_jan.r.christel"

# The snapshots the sample keeps, by entity folder ("" is rooms). Two points in time, so the milestone
# comparison has something to compare: an earlier set (what was pushed in August and September) and the
# current one (all seven entities, pushed 2026-10-30). Names are the files' own.
EARLIER = {
    "": "2026-08-23T00-44-17.490000Z.json",
    "doors": "2026-08-23T00-44-17.490000Z.json",
    "windows": "2026-09-04T12-41-04.924000Z.json",
    "ffe": "2026-09-05T12-57-49.149000Z.json",
}
CURRENT = {
    entity: "2026-10-30T08-52-45.876000Z.json"
    for entity in ("", "doors", "windows", "ffe", "spaces", "ceilings", "floors")
}

# The blueprint is captured from this room and put in the other, a bedroom of the same kind.
BLUEPRINT_ROOM = "BED 01"
ASSIGN_ROOM = "BED 02"
BLUEPRINT_NAME = "Bedroom layout"

TAG_TYPE_CODE = "Assembly Code"


def stamp(name):
    """`2026-08-23T00-44-17.490000Z.json` -> the id a milestone names, `2026-08-23T00:44:17.490000Z`."""
    base = name[: -len(".json")]
    date, _, time_part = base.partition("T")
    return f"{date}T{time_part.replace('-', ':')}"


def milestones_block():
    def attachments(chosen):
        lines = []
        for table, entity in (
            ("attachments", ""),
            ("door_attachments", "doors"),
            ("window_attachments", "windows"),
            ("ffe_attachments", "ffe"),
            ("space_attachments", "spaces"),
            ("ceiling_attachments", "ceilings"),
            ("floor_attachments", "floors"),
        ):
            lines.append(f"[milestones.{table}]")
            if entity in chosen:
                lines.append(f'"{MODEL}" = "{stamp(chosen[entity])}"')
            lines.append("")
        return "\n".join(lines)

    return (
        "# Two milestones over the snapshots in this sample: an earlier push, and the current one. The earlier set\n"
        "# has no spaces, ceilings or floors (they were not exported yet), which the comparison reports as added.\n"
        '[[milestones]]\nname = "Earlier push"\ndate = "2026-09-05"\n\n[milestones.reference_snapshots]\n\n'
        + attachments(EARLIER)
        + '[[milestones]]\nname = "Current push"\ndate = "2026-10-30"\n\n[milestones.reference_snapshots]\n\n'
        + attachments(CURRENT)
    )


def project_settings(source_toml):
    """The dev project settings with this sample's milestones and the tag type code."""
    text = open(source_toml, encoding="utf-8").read()
    start = text.index("# Two milestones over the data actually in the store.")
    end = text.index("[appearance.rooms]")
    text = text[:start] + milestones_block() + text[end:]
    note = (
        "# The FF&E TYPE parameter whose value identifies a kind of item. Tag rules are keyed by it (CLAUDE.md,\n"
        "# 'Tag rules'); set it to a parameter your models fill in.\n"
        f'tag_type_code = "{TAG_TYPE_CODE}"\n\n'
    )
    # The dev settings compare doors only; a sample that compares milestones should compare rooms too.
    rooms = (
        "# Rooms are matched across pushes by their Revit id; Area is compared.\n"
        'comparison_key = "$id"\n'
        'comparison_properties = ["Area"]\n'
    )
    text = text.replace("comparison_properties = []\n", rooms, 1)
    return text.replace("[sources]", note + "[sources]", 1)


# The manifest's list for each entity, and the folder those snapshots are in ("" is the model folder itself).
MANIFEST_LISTS = (
    ("snapshots", ""), ("doors", "doors"), ("windows", "windows"), ("ffe", "ffe"),
    ("spaces", "spaces"), ("ceilings", "ceilings"), ("floors", "floors"),
)


def manifest(source_manifest, model_dir):
    """The store's index (`project.toml`) for what this sample actually holds.

    The source store's manifest lists every snapshot that store ever took, so copying it names dozens of files
    that are not here and the server logs a warning for each ("filesystem wins"). The model's name, phase and
    placement are kept; the lists are rebuilt from the files present.
    """
    text = open(source_manifest, encoding="utf-8").read()
    start = text.index("snapshots = [")
    end = text.index("\n[", start)  # the next table: [reference_snapshots]
    lines = []
    for key, folder in MANIFEST_LISTS:
        folder_path = os.path.join(model_dir, folder)
        names = sorted(n for n in os.listdir(folder_path) if n.endswith(".json")) if os.path.isdir(folder_path) else []
        ids = ", ".join('"%s"' % stamp(n) for n in names)
        lines.append(f"{key} = [{ids}]" if names else f"{key} = []")
    return text[:start] + "\n".join(lines) + "\n" + text[end:]


SERVER_TOML = """# The sample server settings: snapshots under ./data/snapshots, projects under ./projects.
# Relative paths resolve against THIS file's directory, so it runs from anywhere:
#   cargo run --bin roommate -- --server-settings samples/house-a/server.toml --project-settings samples/house-a/projects
# Regenerate everything here with scripts/make_sample.py.

[storage]
root = "./data/snapshots"
"""


class Server:
    def __init__(self, binary, root, port):
        self.binary, self.root, self.port = binary, root, port
        self.base = f"http://127.0.0.1:{port}"
        self.proc = None

    def __enter__(self):
        log = open(os.path.join(self.root, "server.log"), "w")
        self.proc = subprocess.Popen(
            [self.binary, "--server-settings", os.path.join(self.root, "server.toml"),
             "--project-settings", os.path.join(self.root, "projects"), "--port", str(self.port)],
            cwd=REPO, stdout=log, stderr=subprocess.STDOUT,
        )
        for _ in range(100):
            try:
                self.get("/projects")
                return self
            except Exception:
                if self.proc.poll() is not None:
                    raise SystemExit("the server exited at start; see " + os.path.join(self.root, "server.log"))
                time.sleep(0.2)
        raise SystemExit("the server did not answer")

    def __exit__(self, *exc):
        if self.proc:
            self.proc.terminate()
            self.proc.wait(timeout=10)

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.base + path, data=data, method=method, headers={"content-type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                text = response.read().decode()
        except urllib.error.HTTPError as e:
            raise SystemExit(f"{method} {path}: {e.code} {e.read().decode()}")
        return json.loads(text) if text else None

    def get(self, path):
        return self.call("GET", path)

    def post(self, path, body):
        return self.call("POST", path, body)


def room_named(server, prefix):
    rooms = server.get("/rooms?project=" + urllib.parse.quote(PROJECT))["rooms"]
    found = [r for r in rooms if r["name"].startswith(prefix)]
    if not found:
        raise SystemExit(f"no room named {prefix!r} in the sample")
    return found[0]


def read_sheets(sheets_dir, numbers):
    """The library's record files for `numbers`, as the blueprint push carries them."""
    records = {}
    for name in sorted(os.listdir(sheets_dir)):
        if name.endswith(".json"):
            record = json.load(open(os.path.join(sheets_dir, name), encoding="utf-8"))
            records[record["sheet_number"]] = record
    missing = [n for n in numbers if n not in records]
    if missing:
        raise SystemExit(f"no sheet numbered {missing} in {sheets_dir}; it has {sorted(records)}")
    keep = ("revit_sheet_id", "document_title", "sheet_number", "sheet_name", "size_mm", "sheet")
    return [{k: records[n][k] for k in keep if k in records[n]} for n in numbers]


def build(args, work):
    source = os.path.join(args.source, MODEL)
    model_dir = os.path.join(work, "data", "snapshots", PROJECT, MODEL)
    for chosen in (EARLIER, CURRENT):
        for entity, name in chosen.items():
            src = os.path.join(source, entity, name)
            if not os.path.exists(src):
                raise SystemExit(f"missing {src}; --source must be the House A store with these snapshots")
            dst = os.path.join(model_dir, entity, name)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copyfile(src, dst)
    with open(os.path.join(work, "data", "snapshots", PROJECT, "project.toml"), "w", encoding="utf-8", newline="\n") as f:
        f.write(manifest(os.path.join(args.source, "project.toml"), model_dir))
    os.makedirs(os.path.join(work, "projects"), exist_ok=True)
    with open(os.path.join(work, "server.toml"), "w", encoding="utf-8", newline="\n") as f:
        f.write(SERVER_TOML)
    with open(os.path.join(work, "projects", PROJECT + ".toml"), "w", encoding="utf-8", newline="\n") as f:
        f.write(project_settings(os.path.join(REPO, "settings", "projects", PROJECT + ".toml")))

    with Server(args.server_bin, work, args.port) as server:
        project = urllib.parse.quote(PROJECT)
        source_room = room_named(server, BLUEPRINT_ROOM)
        target = room_named(server, ASSIGN_ROOM)
        if args.sheets_dir:
            sheets = read_sheets(args.sheets_dir, args.sheet_numbers)
            made = server.post(f"/projects/{project}/blueprints/from-revit", {
                "name": BLUEPRINT_NAME, "room_id": source_room["id"], "model_id": MODEL,
                "note": "Sample: captured from " + source_room["name"], "origin": "make_sample.py", "sheets": sheets,
            })
            blueprint_id = made["blueprint_id"]
            if args.generic_sheet_numbers:
                server.post(f"/projects/{project}/sheets", {"sheets": read_sheets(args.sheets_dir, args.generic_sheet_numbers)})
        else:
            captured = server.get(f"/projects/{project}/blueprints/capture?room_id={source_room['id']}")
            made = server.post(f"/projects/{project}/blueprints", {
                "name": BLUEPRINT_NAME,
                "source": {"model_id": MODEL, "room_id": source_room["id"], "room_name": source_room["name"]},
                "note": "Sample: captured from " + source_room["name"],
                "outline": captured["outline"], "items": captured["items"], "doors": captured["doors"],
            })
            blueprint_id = made["id"]
        server.post(f"/projects/{project}/assignments", {
            "blueprint_id": blueprint_id, "room_id": target["id"], "model_id": MODEL,
            "note": "Sample: the blueprint of " + source_room["name"] + " placed in " + target["name"],
        })
        print(f"blueprint {blueprint_id!r} from {source_room['name']}, assigned to {target['name']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", default=os.path.join(REPO, "data", "snapshots", PROJECT),
                        help="the House A store holding the snapshots named in this script (default: data/snapshots/House A)")
    parser.add_argument("--server-bin", default=os.path.join(REPO, "target", "debug", "roommate.exe" if os.name == "nt" else "roommate"))
    parser.add_argument("--out", default=os.path.join(REPO, "samples", "house-a"))
    parser.add_argument("--port", type=int, default=5198)
    parser.add_argument("--sheets-dir", help="a library's sheets/<project> folder to take the blueprint's sheets from")
    parser.add_argument("--sheet-numbers", type=lambda s: [x for x in s.split(",") if x], default=[],
                        help="comma-separated sheet numbers bound to the blueprint")
    parser.add_argument("--generic-sheet-numbers", type=lambda s: [x for x in s.split(",") if x], default=[],
                        help="comma-separated sheet numbers added bound to no blueprint")
    args = parser.parse_args()
    if args.sheets_dir and not args.sheet_numbers:
        parser.error("--sheets-dir needs --sheet-numbers")
    if not os.path.exists(args.server_bin):
        parser.error(f"{args.server_bin} does not exist; run `cargo build --bin roommate`")

    work = tempfile.mkdtemp(prefix="roommate-sample-")
    try:
        build(args, work)
        for transient in ("server.log",):
            path = os.path.join(work, transient)
            if os.path.exists(path):
                os.remove(path)
        shutil.rmtree(os.path.join(work, "projects", ".backups"), ignore_errors=True)
        # The one hand-written file in the sample survives a rebuild.
        readme = os.path.join(args.out, "README.md")
        if os.path.exists(readme):
            shutil.copyfile(readme, os.path.join(work, "README.md"))
        if os.path.exists(args.out):
            shutil.rmtree(args.out)
        shutil.copytree(work, args.out)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    size = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(args.out) for f in fs)
    print(f"wrote {args.out} ({size / 1e6:.1f} MB)")


if __name__ == "__main__":
    sys.exit(main())
