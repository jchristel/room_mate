# House A: the sample project

A small house (32 rooms on three levels, its doors, windows, FF&E, ceilings, floors) as Revit pushed it, with a
blueprint and its assignment on top. Run the server on it and open the viewer:

```bash
cargo run --bin roommate -- --server-settings samples/house-a/server.toml --project-settings samples/house-a/projects
```

Then `http://127.0.0.1:5151/?project=House%20A`. Copy the folder first if you want to push into it or author on it:
the server writes beside what it reads.

## What is in it

| | |
|---|---|
| `data/snapshots/House A/` | Two pushes of one model: an earlier one (rooms, doors, windows, FF&E) and the current one (all seven entities). The project settings' two milestones, **Earlier push** and **Current push**, compare them. |
| `projects/House A.toml` | The project settings, including `tag_type_code` (the FF&E type parameter tag rules are keyed by). |
| `projects/blueprints/`, `projects/assignments/` | A blueprint captured from **BED 01** and placed in **BED 02**. |

It has **no room layout sheets and no tag rules yet.** Those come from Revit and are added by the script below once
there are real ones.

## Keeping it current

Nothing here is edited by hand. The snapshots are what Revit pushed; the blueprint and assignment were made by
calling the running server, so they are written in the format the server writes today. A cargo test
(`src/sample_data.rs`) boots the server on a copy of this folder and reads all of it, so a format change that the
sample does not survive fails CI. When it does:

```bash
cargo build --bin roommate
python scripts/make_sample.py
```

and commit what changed.

## Adding sheets

Push the sheets from Revit to a server you run, then point the script at that server's library folder
(`<project settings>/sheets/<project>`) and name the sheets to keep:

```bash
python scripts/make_sample.py --sheets-dir "<...>/sheets/House A" --sheet-numbers A101,A102 --generic-sheet-numbers A103
```

The first list is bound to the blueprint (made the way the Blueprint button makes it); the second is bound to
nothing. Only sheets from your own model should be published: they are Revit's export, unchanged.
