"""Matching sheets to rooms by a rule, and the small rules the sheet and blueprint windows share. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that CPython runs it
too (`extractor/tests/test_match_rule.py`).

**A rule is two templates and a relation**: text for the sheet side and text for the room side, each with
`{Parameter}` placeholders filled from that element's own parameters (the same notation as the RLS view, sheet
number and name templates: `room_m.rls_template.TOKEN`), and the relation between the two results:

    {Sheet Number}   is part of   {Number}          sheet "101" matches room "101-A"
    {Sheet Name}     equals       {Name} {Number}   any sheet parameter against any room parameter

An entry (a pick-list item from `room_m.utils.sheets`) carries its parameters as `params`, name -> text; the
window fills that in (`utils.sheets.ensure_params`), so this module never touches Revit. Names are compared
without regard to case. **A template that cannot be filled never matches**: if any placeholder names a parameter the
element lacks or leaves blank, the side is empty and pairs with nothing (a blank room number would otherwise be
"part of" every sheet).
"""

from room_m.rls_template import TOKEN

CAPTION_LIMIT = 30

# "the sheet's text contains the room's", "the sheet's text is part of the room's", or equals.
RELATIONS = [("contains", "contains"), ("inside", "is part of"), ("equals", "equals")]

DEFAULT_SHEET = "{Sheet Number}"
DEFAULT_ROOM = "{Number}"


def numbers_caption(entries):
    """What a button says for the `entries` ticked: their NUMBERS, so a row shows which ones it holds and not just
    how many. A long list is cut with how many more there are; an entry with no number shows its label."""
    if not entries:
        return "(none)"
    shown = []
    used = 0
    for entry in entries:
        number = (entry.get("number") or entry.get("label") or "?").strip()
        if shown and used + len(number) + 2 > CAPTION_LIMIT:
            break
        shown.append(number)
        used += len(number) + 2
    text = ", ".join(shown)
    return text if len(shown) == len(entries) else "{} +{} more".format(text, len(entries) - len(shown))


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
    """Whether something labelled `label` is listed under the filter `text`: every word must be in it and no `-word`
    may be. Case is ignored and a blank filter lists everything. `101 -roof` lists labels with 101 and no roof."""
    wanted, unwanted = terms(text)
    low = (label or "").lower()
    return all(w in low for w in wanted) and not any(w in low for w in unwanted)


def make_rule(sheet_template, relation, room_template):
    """A rule as the dict the functions below take (and `prefs` keeps), or None when the relation is unknown or
    either template is blank."""
    sheet_template = (sheet_template or "").strip()
    room_template = (room_template or "").strip()
    if not sheet_template or not room_template or relation not in dict(RELATIONS):
        return None
    return {"sheet": sheet_template, "relation": relation, "room": room_template}


def rule_from_saved(saved):
    """The rule `prefs` kept, or None when it does not parse. A rule saved before templates existed named a field
    (`number`, `name`); those two are read as the matching placeholder, `either` is dropped."""
    if not isinstance(saved, dict):
        return None
    old = {"sheet": {"number": "{Sheet Number}", "name": "{Sheet Name}"},
           "room": {"number": "{Number}", "name": "{Name}"}}

    def side(key):
        value = saved.get(key)
        if value == "either":
            return None
        return old[key].get(value, value) if hasattr(value, "strip") else None

    return make_rule(side("sheet"), saved.get("relation"), side("room"))


def describe_rule(rule):
    """The rule as a sentence: `Sheet {Sheet Number} is part of room {Number}.`"""
    if not rule:
        return "No rule: choose by hand."
    return "Sheet {} {} room {}.".format(rule["sheet"], dict(RELATIONS)[rule["relation"]], rule["room"])


def fill(template, params):
    """`template` with each `{Name}` replaced from `params` (names compared without case), lower case and trimmed;
    None when a placeholder names a parameter that is missing or blank, or the result is empty."""
    lowered = dict((k.lower(), v) for k, v in (params or {}).items())
    missing = []

    def value(match):
        found = lowered.get(match.group(1).strip().lower())
        found = "" if found is None else found
        if not found.strip():
            missing.append(match.group(1))
        return found.strip()

    text = TOKEN.sub(value, template or "")
    if missing:
        return None
    text = text.strip().lower()
    return text or None


def pair_matches(rule, sheet, room):
    """Whether `rule` pairs `sheet` with `room` (pick-list entries carrying `params`)."""
    if not rule:
        return False
    mine = fill(rule["sheet"], sheet.get("params"))
    theirs = fill(rule["room"], room.get("params"))
    if mine is None or theirs is None:
        return False
    if rule["relation"] == "contains":
        return theirs in mine
    if rule["relation"] == "inside":
        return mine in theirs
    return mine == theirs


def rooms_for_sheet(rule, sheet, rooms):
    """The rooms `rule` pairs with `sheet`, in list order."""
    return [room for room in rooms if pair_matches(rule, sheet, room)]


def sheets_for_room(rule, room, sheets, taken=()):
    """The sheets `rule` pairs with `room`, in list order, leaving out ids in `taken` (a sheet belongs to one room)."""
    return [s for s in sheets if s["id"] not in taken and pair_matches(rule, s, room)]


def unknown_parameters(rule, sheets, rooms):
    """Lines naming a placeholder no sheet (or no room) has a parameter for, so a typo is said and not just a rule
    that quietly matches nothing. Empty when the rule is fine or there is nothing to check against."""
    out = []
    if not rule:
        return out
    for key, entries, what in (("sheet", sheets, "sheet"), ("room", rooms, "room")):
        if not entries:
            continue
        known = set()
        for entry in entries:
            known.update(k.lower() for k in (entry.get("params") or {}))
        for name in TOKEN.findall(rule[key]):
            if name.strip().lower() not in known:
                out.append("No {} has a parameter called '{}'.".format(what, name.strip()))
    return out


def duplicates(names):
    """The positions in `names` whose text another position shares (case and edges ignored); blanks are not
    duplicates of each other. For marking blueprint names that are not unique."""
    seen = {}
    for i, name in enumerate(names):
        key = (name or "").strip().lower()
        if key:
            seen.setdefault(key, []).append(i)
    out = set()
    for positions in seen.values():
        if len(positions) > 1:
            out.update(positions)
    return out
