"""The sheet export window's rules: what is valid, and how a room choice reads. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that
CPython runs it too (`extractor/tests/test_sheet_export_form.py`); the window (`sheet_export_window`) only
draws it.

**A sheet needs no room.** The rooms ticked beside a sheet only supply the figures it is ranked on
(size, proportions, area, item count: `room_m.room_figures`); none is linked to it.
"""


def rooms_caption(count):
    """What the room button says for `count` rooms ticked."""
    if count <= 0:
        return "(none)"
    return "1 room" if count == 1 else "{} rooms".format(count)


def matches(label, text):
    """Whether a room whose label is `label` is listed under the filter `text` (blank lists all)."""
    needle = (text or "").strip().lower()
    return not needle or needle in (label or "").lower()


def problems(project, sheets_ticked):
    """What stops the export, as lines for the status; empty when it can run."""
    out = []
    if project is None:
        out.append("Choose the project to export to.")
    if sheets_ticked <= 0:
        out.append("Tick at least one sheet.")
    return out
