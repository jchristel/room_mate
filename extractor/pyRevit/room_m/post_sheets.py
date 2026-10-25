"""Push chosen sheets to the server's sheet library.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**Its own endpoint, not a snapshot push.** Rooms, doors and the rest replace a model's state
as a snapshot; the sheet library is authored data the server keeps by sheet identity (Revit
sheet id plus document title). So this does not go through `post_entity`, has no run
envelope and no snapshot id: it posts `{"sheets": [...]}` to
`/projects/{id}/sheets` and reads back what the server did with each sheet -- created,
updated, or held pending because the sheet number changed.
"""

import json

try:
    from urllib import quote
except ImportError:  # CPython 3
    from urllib.parse import quote

from room_m.post_common import _post_content

SERVER_URL = "/projects/{}/sheets"


def sheets_url(project_id):
    return SERVER_URL.format(quote(project_id, safe=""))


# The extractor's own keys on a read sheet: not part of what the library stores.
_EXTRACTOR_KEYS = ("tags", "tag_problems")


def _library_sheet(pushed):
    return dict((k, v) for k, v in pushed.items() if k not in _EXTRACTOR_KEYS)


def post_sheets(project_id, pushed):
    """POST `pushed` (a list from `room_m.exporters.sheets.read_sheets`) and return
    `(ok, status, text, outcomes)`; `outcomes` is the server's per-sheet answer, or `[]`
    when it could not be read."""
    from System.Net.Http import StringContent
    from System.Text import Encoding

    # ensure_ascii=False, as every other push here does: escaping a non-ASCII character under
    # IronPython 2.7 decodes with the system code page and aborts the push.
    body = json.dumps({"sheets": [_library_sheet(p) for p in pushed]}, ensure_ascii=False)
    ok, status, text = _post_content(
        sheets_url(project_id), StringContent(body, Encoding.UTF8, "application/json")
    )
    outcomes = []
    if ok:
        try:
            outcomes = json.loads(text)
        except Exception:
            outcomes = []
    return ok, status, text, outcomes


def describe_outcomes(outcomes):
    """One line per sheet for the run's output. A held sheet says what to do next."""
    lines = []
    for o in outcomes:
        what = o.get("outcome")
        number = o.get("sheet_number", "?")
        if what == "created":
            lines.append("{}: added to the sheet library".format(number))
        elif what == "updated":
            lines.append("{}: updated".format(number))
        elif what == "pending":
            lines.append(
                "{}: its sheet number CHANGED, so this push is held until you accept "
                "or reject it in RoomMate".format(number)
            )
        else:
            lines.append("{}: {}".format(number, what))
    return lines
