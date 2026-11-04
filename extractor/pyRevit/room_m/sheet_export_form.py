"""The sheet export window's rules: what is valid, and how a room choice reads. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that
CPython runs it too (`extractor/tests/test_sheet_export_form.py`); the window (`sheet_export_window`) only
draws it.

**A sheet needs no room.** The rooms ticked beside a sheet only supply the figures it is ranked on
(size, proportions, area, item count: `room_m.room_figures`); none is linked to it.
"""


CAPTION_LIMIT = 30


def rooms_caption(rooms):
    """What the room button says for the rooms ticked (their pick-list entries): their NUMBERS, so the row shows
    which rooms a sheet covers and not just how many. A long list is cut with how many more there are."""
    if not rooms:
        return "(none)"
    shown = []
    used = 0
    for room in rooms:
        number = (room.get("number") or room.get("label") or "?").strip()
        if shown and used + len(number) + 2 > CAPTION_LIMIT:
            break
        shown.append(number)
        used += len(number) + 2
    text = ", ".join(shown)
    return text if len(shown) == len(rooms) else "{} +{} more".format(text, len(rooms) - len(shown))


def terms(text):
    """The words of a filter: `(wanted, unwanted)`. A word with a leading `-` is one the label must NOT contain."""
    wanted, unwanted = [], []
    for word in (text or "").lower().split():
        if word.startswith("-") and len(word) > 1:
            unwanted.append(word[1:])
        else:
            wanted.append(word)
    return wanted, unwanted


def filter_matches(label, text):
    """Whether a sheet labelled `label` is listed under the filter `text`: every word must be in it and no `-word`
    may be. Case is ignored and a blank filter lists everything. `101 -roof` lists sheets with 101 and no roof."""
    wanted, unwanted = terms(text)
    low = (label or "").lower()
    return all(w in low for w in wanted) and not any(w in low for w in unwanted)


# What a room rule compares. The sheet's side and the room's side are each its number, its name or either, and the
# relation is "the sheet's text contains the room's" or "the sheet's text is part of the room's" (or equals).
FIELDS = [("number", "number"), ("name", "name"), ("either", "number or name")]
RELATIONS = [("contains", "contains"), ("inside", "is part of"), ("equals", "equals")]


def make_rule(sheet_field, relation, room_field):
    """A room rule as the dict `rooms_for_sheet` takes (and `prefs` keeps), or None when any part is unknown."""
    if sheet_field not in dict(FIELDS) or room_field not in dict(FIELDS) or relation not in dict(RELATIONS):
        return None
    return {"sheet": sheet_field, "relation": relation, "room": room_field}


def describe_rule(rule):
    """The rule as a sentence: `Sheet number is part of room name`."""
    if not rule:
        return "No rule: choose the rooms by hand."
    return "Sheet {} {} room {}.".format(
        dict(FIELDS)[rule["sheet"]], dict(RELATIONS)[rule["relation"]], dict(FIELDS)[rule["room"]])


def _values(entry, field):
    keys = ["number", "name"] if field == "either" else [field]
    return [str(entry.get(k) or "").strip().lower() for k in keys if (entry.get(k) or "").strip()]


def rule_matches(rule, sheet, room):
    """Whether `rule` pairs `sheet` with `room`. A blank value never matches anything (an unnumbered room would
    otherwise be 'part of' every sheet)."""
    if not rule:
        return False
    for mine in _values(sheet, rule["sheet"]):
        for theirs in _values(room, rule["room"]):
            if rule["relation"] == "contains" and theirs in mine:
                return True
            if rule["relation"] == "inside" and mine in theirs:
                return True
            if rule["relation"] == "equals" and mine == theirs:
                return True
    return False


def rooms_for_sheet(rule, sheet, rooms):
    """The rooms `rule` pairs with `sheet`, in list order."""
    return [room for room in rooms if rule_matches(rule, sheet, room)]


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
    rule = saved.get("rule")
    rule = make_rule(rule.get("sheet"), rule.get("relation"), rule.get("room")) if isinstance(rule, dict) else None
    return index, str(saved.get("filter") or ""), rule


def problems(project, sheets_ticked):
    """What stops the export, as lines for the status; empty when it can run."""
    out = []
    if project is None:
        out.append("Choose the project to export to.")
    if sheets_ticked <= 0:
        out.append("Tick at least one sheet.")
    return out
