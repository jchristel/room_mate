"""The sheet export window's rules: what is valid, and how a room choice reads. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that
CPython runs it too (`extractor/tests/test_sheet_export_form.py`); the window (`sheet_export_window`) only
draws it.

**A sheet needs no room.** The rooms ticked beside a sheet only supply the figures it is ranked on
(size, proportions, area, item count: `room_m.room_figures`); none is linked to it.
"""


from room_m import match_rule

# Shared with the blueprint window: see `room_m.match_rule`.
rooms_caption = match_rule.numbers_caption
CAPTION_LIMIT = match_rule.CAPTION_LIMIT
terms = match_rule.terms
filter_matches = match_rule.filter_matches
RELATIONS = match_rule.RELATIONS
make_rule = match_rule.make_rule
describe_rule = match_rule.describe_rule
rooms_for_sheet = match_rule.rooms_for_sheet


def matches(label, text):
    """Whether a room whose label is `label` is listed under the filter `text` (blank lists all)."""
    needle = (text or "").strip().lower()
    return not needle or needle in (label or "").lower()


def to_save(project, text, rule):
    """The section `prefs` keeps after an export: the project id, the sheet filter and the room rule."""
    return {"project": (project or {}).get("id"), "filter": text or "", "rule": rule}


def remembered(saved, projects):
    """`(project index or None, filter text, rule or None)` from the saved section; a project that no longer exists
    and a rule that no longer parses are dropped."""
    saved = saved or {}
    index = None
    for i, project in enumerate(projects):
        if project.get("id") == saved.get("project"):
            index = i
    return index, str(saved.get("filter") or ""), match_rule.rule_from_saved(saved.get("rule"))


def problems(project, sheets_ticked):
    """What stops the export, as lines for the status; empty when it can run."""
    out = []
    if project is None:
        out.append("Choose the project to export to.")
    if sheets_ticked <= 0:
        out.append("Tick at least one sheet.")
    return out
