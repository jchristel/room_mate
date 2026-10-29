"""Room layout sheet settings: the hand-off between a Revit model and the settings page.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The Revit calls
are made inside the functions that need them, so the pure half (`page_url`, `missing`) is tested
on CPython (`extractor/tests/test_rls_settings.py`).

**The model is the source of truth; the page is only the form.** The settings live in the model's
extensible storage under the key `rls` (`room_m.project_settings`). A grid of our own in Revit
(WPF from IronPython) crashed Revit, so the plug-in posts the current settings and the choices only
Revit knows (`collect_options`) to the server as a draft (`POST /projects/{id}/modelsettings`),
opens `/model-settings/` in the default browser, and when the person says they saved reads the
draft back (`GET`) and writes it into the model. The page's names are the contract in
`src-js/model-settings/model.ts`; change one and the other in the same commit.

**A list that cannot be read is left out**, not sent empty: the page then says Revit offered no
list, which is different from a model that has none.

**Not yet run in Revit.**
"""

import json

try:
    from urllib import quote
except ImportError:  # CPython 3
    from urllib.parse import quote

from room_m import server_config

PROJECT_KEY = "rls"
PAGE_PATH = "/model-settings/"
DRAFT_PATH = "/projects/{}/modelsettings"
SIZES = ("a0", "a1", "a2", "a3")


def draft_path(project_id):
    return DRAFT_PATH.format(quote(project_id, safe=""))


def page_url(project_id, model_id, environ=None, path=None):
    """The address of the settings page for one model, on the server in force."""
    return "{}?project={}&model={}".format(
        server_config.url_for(PAGE_PATH, environ, path), quote(project_id, safe=""), quote(model_id, safe=""))


def draft_get_path(project_id, model_id):
    return "{}?model={}".format(draft_path(project_id), quote(model_id, safe=""))


def missing(settings):
    """What stops the settings being used, in words; empty when complete. The same question the page
    asks (`missing` in model.ts): the tags are left out because without them a task only warns."""
    s = settings or {}
    sizes = s.get("sheet_sizes") if isinstance(s.get("sheet_sizes"), dict) else {}
    out = []
    for size in SIZES:
        if not sizes.get(size):
            out.append("title block for {}".format(size.upper()))
    for key, label in (
        ("plan_view_type", "plan view type"),
        ("elevation_view_type", "elevation view type"),
        ("schedule_base", "schedule base"),
        ("sheet_number", "sheet number logic"),
        ("sheet_name", "sheet name logic"),
        ("view_name", "view name logic"),
    ):
        value = s.get(key)
        if not (isinstance(value, (type(u""), str)) and value.strip()):
            out.append(label)
    return out


def _names(elements, label):
    seen = set()
    for element in elements:
        try:
            name = label(element)
        except Exception:
            continue
        if name:
            seen.add(name)
    return sorted(seen)


def collect_options(doc):
    """The choices this model offers, as the page's `options`. Each list is read on its own, and
    one that fails is left out."""
    from Autodesk.Revit.DB import (
        BuiltInCategory, ElementId, FilteredElementCollector, ViewFamily, ViewFamilyType, ViewSchedule)
    from duHast.Revit.Family.family_symbol_lookup import get_symbol_full_names_by_category

    options = {}

    def view_types(family):
        return [v for v in FilteredElementCollector(doc).OfClass(ViewFamilyType) if v.ViewFamily == family]

    def room_schedules():
        rooms = ElementId(BuiltInCategory.OST_Rooms)
        return [
            v for v in FilteredElementCollector(doc).OfClass(ViewSchedule)
            if not v.IsTemplate and not v.IsInternalKeynoteSchedule and not v.IsTitleblockRevisionSchedule
            and v.Definition.CategoryId == rooms
        ]

    def room_parameters():
        room = FilteredElementCollector(doc).OfCategory(BuiltInCategory.OST_Rooms).WhereElementIsNotElementType().FirstElement()
        return [p.Definition.Name for p in room.Parameters] if room is not None else []

    for key, read in (
        ("title_blocks", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_TitleBlocks)),
        ("plan_view_types", lambda: _names(view_types(ViewFamily.FloorPlan), lambda v: v.Name)),
        ("elevation_view_types", lambda: _names(view_types(ViewFamily.Elevation), lambda v: v.Name)),
        ("schedules", lambda: _names(room_schedules(), lambda v: v.Name)),
        ("room_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_RoomTags)),
        ("independent_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_MultiCategoryTags)),
        ("room_parameters", lambda: sorted(set(room_parameters()))),
    ):
        try:
            options[key] = read()
        except Exception:
            pass
    return options


def open_in_browser(url):
    """Open `url` in the default browser. Raises if the shell cannot."""
    from System.Diagnostics import Process, ProcessStartInfo

    info = ProcessStartInfo(url)
    info.UseShellExecute = True
    Process.Start(info)


def draft_body(model_id, settings, options):
    return {"model_id": model_id, "settings": settings or {}, "options": options or {}}


def start_draft(project_id, model_id, settings, options):
    """POST a fresh, unsaved draft for the page to edit. `(ok, status, text)`."""
    from System.Net.Http import StringContent
    from System.Text import Encoding
    from room_m.post_common import _post_content

    body = json.dumps(draft_body(model_id, settings, options), ensure_ascii=False)
    return _post_content(draft_path(project_id), StringContent(body, Encoding.UTF8, "application/json"))


def read_draft(project_id, model_id):
    """GET the draft back. `(ok, status, draft or text)`."""
    from room_m.post_common import fetch_text

    ok, status, text = fetch_text(draft_get_path(project_id, model_id))
    if not ok:
        return ok, status, text
    draft = parse_draft(text)
    if draft is None:
        return False, status, "the server's answer was not a settings draft"
    return True, status, draft


def parse_draft(text):
    """The draft in a server answer, or None."""
    try:
        draft = json.loads(text)
    except (TypeError, ValueError):
        return None
    return draft if isinstance(draft, dict) else None
