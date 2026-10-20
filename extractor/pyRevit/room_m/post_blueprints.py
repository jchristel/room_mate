"""Push a blueprint from Revit: a room and the room layout sheets chosen for it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**Its own endpoint, not a snapshot push**, and the room's contents are NOT sent. The server
captures the room from the rooms and FF&E it already holds (one capture rule, in Rust), so
this posts only which room, the name, and the sheets: `{"name", "room_id", "model_id",
"origin", "sheets": [...]}` to `/projects/{id}/blueprints/from-revit`. A blueprint comes only
from here, always with its sheets (docs/PLAN-blueprints.md, decision 18): the server refuses a
push with none, and so does the button before it asks.

A name the project already has becomes a PENDING revision of that blueprint, for a person to
accept or reject in the blueprint viewer; a new name creates the blueprint.
"""

import json

try:
    from urllib import quote
except ImportError:  # CPython 3
    from urllib.parse import quote

from room_m.post_common import _post_content

SERVER_URL = "http://127.0.0.1:5151/projects/{}/blueprints/from-revit"

# What the server keeps of a sheet in a blueprint: its identity (to recognise it on a re-push)
# and duHast's layout. Not the room and not the phase, which belong to the library's records.
_SHEET_KEYS = ("revit_sheet_id", "document_title", "sheet_number", "sheet_name", "sheet")


def blueprint_url(project_id):
    return SERVER_URL.format(quote(project_id, safe=""))


def strip_tags(sheet):
    """Drop tag data from a sheet before it is pushed (plan decision 24): tags belong to the
    items of one room and do not travel with a layout. duHast does not export tags today, so
    this removes nothing yet; it keeps the rule true when it does."""
    if isinstance(sheet, dict):
        sheet.pop("tags", None)
        for port in sheet.get("view_ports", []) or []:
            if isinstance(port, dict):
                port.pop("tags", None)
    return sheet


def blueprint_sheets(pushed):
    """The sheets of a blueprint push, from `room_m.exporters.sheets.read_sheets` output."""
    out = []
    for p in pushed:
        entry = dict((k, p[k]) for k in _SHEET_KEYS if k in p)
        entry["sheet"] = strip_tags(entry.get("sheet"))
        out.append(entry)
    return out


def post_blueprint(project_id, name, room_id, model_id, origin, pushed, note=None):
    """POST one room's blueprint and return `(ok, status, text, outcome)`; `outcome` is the
    server's answer (`created` or `pending`), or `{}` when it could not be read."""
    from System.Net.Http import StringContent
    from System.Text import Encoding

    body = {
        "name": name,
        "room_id": room_id,
        "model_id": model_id,
        "origin": origin,
        "sheets": blueprint_sheets(pushed),
    }
    if note:
        body["note"] = note
    # ensure_ascii=False, as every other push here does: escaping a non-ASCII character under
    # IronPython 2.7 decodes with the system code page and aborts the push.
    ok, status, text = _post_content(
        blueprint_url(project_id),
        StringContent(json.dumps(body, ensure_ascii=False), Encoding.UTF8, "application/json"),
    )
    outcome = {}
    if ok:
        try:
            outcome = json.loads(text)
        except Exception:
            outcome = {}
    return ok, status, text, outcome


def describe_outcome(name, outcome):
    """One line for the run's output. A pending revision says what to do next."""
    what = outcome.get("outcome")
    if what == "created":
        return "{}: blueprint created".format(name)
    if what == "pending":
        return (
            "{}: a blueprint of this name exists, so this is a PENDING revision; accept or "
            "reject it in the blueprint viewer".format(name)
        )
    return "{}: {}".format(name, what)


def explain_refusal(status, text):
    """What to tell the person when the server refused a blueprint push."""
    if status == 404:
        return (
            "the server has no such room: push the Rooms and FFE first, and check the model "
            "name matches ({})".format(text)
        )
    return "blueprint push failed ({}): {}".format(status if status is not None else "no response", text)
