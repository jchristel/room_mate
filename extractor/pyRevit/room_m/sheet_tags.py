"""Take the tags out of an exported sheet, and keep them as observations.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Imports
nothing from Revit, so `extractor/tests/test_sheet_tags.py` runs it on CPython.

duHast puts a view's tags inside the viewport: `sheet["view_ports"][i]["view"]["tags"]`, beside
the view's `id` and `view_scale`. A sheet pushed to the library or inside a blueprint carries
NO tags (plan decision 24: tags belong to the items of one room and do not travel with a
layout), so `take_tags` removes them and hands them back as flat observations, one per tag,
each saying which kind of viewport and which view scale it was read in. Those are what a tag
rule is made from; the server's rules store is keyed by names and never by the ids in them.
"""


def _views(sheet):
    """`(viewport, view)` for each viewport of the sheet that has a view dictionary."""
    out = []
    if not isinstance(sheet, dict):
        return out
    for port in sheet.get("view_ports") or []:
        if isinstance(port, dict) and isinstance(port.get("view"), dict):
            out.append((port, port["view"]))
    return out


def take_tags(sheet, identity=None):
    """Remove every tag from `sheet` (in place) and return them as observations.

    `identity` is where they were read (`document_title`, `revit_sheet_id`, `sheet_number`),
    kept as provenance. Each observation is
    `{"viewport_type", "view_id", "view_scale", "sheet", "tag"}`; `tag` is the duHast tag as it
    was exported. A sheet-level or viewport-level `tags` key, from an older export, is dropped."""
    observations = []
    if not isinstance(sheet, dict):
        return observations
    sheet.pop("tags", None)
    for port, view in _views(sheet):
        port.pop("tags", None)
        for tag in view.pop("tags", None) or []:
            observations.append(
                {
                    "viewport_type": port.get("vp_type"),
                    "view_id": view.get("id"),
                    "view_scale": view.get("view_scale"),
                    "sheet": dict(identity or {}),
                    "tag": tag,
                }
            )
    return observations


def describe_problems(label, problems):
    """Lines for the run's output: tags that could not be read, as a warning and never a stop."""
    lines = []
    if problems:
        lines.append("{}: {} tag(s) could not be read and are left out".format(label, len(problems)))
        for p in problems:
            lines.append("    {}".format(p))
    return lines
