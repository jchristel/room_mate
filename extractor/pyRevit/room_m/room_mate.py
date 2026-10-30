# License:
#
#
# Revit Batch Processor Sample Code
#
# BSD License
# Copyright 2026, Jan Christel
# All rights reserved.

# Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

# - Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
# - Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
# - Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.
#
# This software is provided by the copyright holder "as is" and any express or implied warranties, including, but not limited to, the implied warranties of merchantability and fitness for a particular purpose are disclaimed.
# In no event shall the copyright holder be liable for any direct, indirect, incidental, special, exemplary, or consequential damages (including, but not limited to, procurement of substitute goods or services; loss of use, data, or profits;
# or business interruption) however caused and on any theory of liability, whether in contract, strict liability, or tort (including negligence or otherwise) arising in any way out of the use of this software, even if advised of the possibility of such damage.
#
#
#

import copy

from collections import namedtuple

from duHast.Utilities.Objects.result import Result
from duHast.pyRevit.UI.doc_selector import pick_document

from room_m.utils.ui import (
    choose_project,
    choose_phase,
)

from room_m.utils.generic import (
    entities_label,
)

from room_m.utils.provenance import (
    report_build,
)

from room_m.utils.sheets import (
    placed_rooms,
    sheets_of,
    pick as pick_items,
)
from room_m.exporters.sheets import read_sheets
from room_m.post_blueprints import describe_outcome, explain_refusal, post_blueprint
from room_m.post_sheets import describe_outcomes, post_sheets
from room_m.post_tags import all_observations, post_observations
from room_m.post_tasks import claim_tasks, report_tasks
from room_m import project_settings, rls_settings, server_config
from room_m.post_common import SERVER_URL_PROJECTS, fetch_projects
from room_m.sheet_tags import describe_outcome as describe_tag_outcome, describe_problems

from room_m.utils.post_envelope import (
    build_run_envelope,
    build_model_block,
)

from room_m.exporters import rooms as rooms_exporter
from room_m.exporters import doors as doors_exporter
from room_m.exporters import windows as windows_exporter
from room_m.exporters import ffe as ffe_exporter
from room_m.exporters import spaces as spaces_exporter
from room_m.exporters import ceilings as ceilings_exporter
from room_m.exporters import floors as floors_exporter


# The address kept in the open model is looked for when a request is made (room_m.server_config).
server_config.set_project_reader(project_settings.server_of_active_document)


ROOMS = "rooms"
DOORS = "doors"
WINDOWS = "windows"
FFE = "ffe"
FFE_BY_SPACE = "ffe_by_space"
SPACES = "spaces"
CEILINGS = "ceilings"
FLOORS = "floors"


EntityExporter = namedtuple(
    "EntityExporter", ["export_model", "post_bucket", "stamp_envelope"])

# What each entity contributes to a run, and the ONLY place an entity is named
# in this module's machinery. `export_entry` dispatches over this table rather
# than branching per entity, so adding windows or FF&E is a new module in
# `room_m.exporters`, one row here, and one entry point -- not another `if` in
# the run driver.
#
# **Windows cost exactly one row**, which is what this table was written to make
# true and is now measured rather than claimed: no new branch in the run driver,
# no ordering, no new envelope field. The one thing that did NOT come for free
# was the pyRevit button, which used to live outside this repository -- the
# bundles are in `extractor/pyRevit/RoomMate.extension` now, so a new entity
# costs a row here and a button in the same commit.
#
# **FF&E cost one row too, and that is the stronger result.** Windows are a
# second OPENING and reuse the door record entirely, so a row was always going
# to be enough. An item is a different record with a different translation, a
# different extraction pass and no nested filter -- and the table still did not
# notice, because what it dispatches over is "how does this entity contribute to
# a run", which was never about what the entity is. What each row points at
# absorbed the difference: `room_m.post_entity` grew a `translate` field, and
# `room_m.exporters.ffe` supplies its own.
#
# **FF&E's second row costs nothing new either, and that is the table proving
# the same point from a third direction.** `FFE_BY_SPACE` pushes the identical
# record to the identical server bucket -- only `export_model` differs, by one
# keyword argument -- so it is a row whose `post_bucket` and `stamp_envelope`
# are literally the ones already in the table above it. See
# `room_m.exporters.ffe` for why this is two rows and two buttons rather than
# one row auto-detecting which reference a document has.
#
# **Spaces cost one row as well, and they are the case that tested the table
# from the other side.** Doors, windows and FF&E all push elements that reference
# rooms. A space references nothing -- it is matched to a room later, by value,
# on the server -- and it is the first entity whose push rules differ from every
# sibling's: an empty bucket is sent rather than refused, and a disagreeing phase
# is quarantined rather than refused. Neither difference reached this table,
# because both live in `room_m.exporters.spaces` and `room_m.post_spaces`, which
# is exactly the split the table exists to keep.
#
# **`blocking` is gone, and its absence is the change.** It used to say that a
# model's rooms had to land before its doors were attempted, because the server
# refused a doors push to a model with no rooms. The server no longer asks: it
# resolves a door's rooms itself, and "the rooms have not arrived yet" is a state
# it reports rather than refuses. So the buckets are independent, and there is no
# ordering left for the table to encode. Windows and FF&E inherit that.
ENTITY_EXPORTERS = {
    ROOMS: EntityExporter(
        export_model=rooms_exporter.export_model,
        post_bucket=rooms_exporter.post_bucket,
        stamp_envelope=rooms_exporter.stamp_envelope,
    ),
    DOORS: EntityExporter(
        export_model=doors_exporter.export_model,
        post_bucket=doors_exporter.post_bucket,
        stamp_envelope=doors_exporter.stamp_envelope,
    ),
    WINDOWS: EntityExporter(
        export_model=windows_exporter.export_model,
        post_bucket=windows_exporter.post_bucket,
        stamp_envelope=windows_exporter.stamp_envelope,
    ),
    SPACES: EntityExporter(
        export_model=spaces_exporter.export_model,
        post_bucket=spaces_exporter.post_bucket,
        stamp_envelope=spaces_exporter.stamp_envelope,
    ),
    FFE: EntityExporter(
        export_model=ffe_exporter.export_model,
        post_bucket=ffe_exporter.post_bucket,
        stamp_envelope=ffe_exporter.stamp_envelope,
    ),
    # Same server bucket as FFE -- `ffe_exporter.post_bucket` posts under the
    # one "ffe" entity regardless of which row called it. Only `export_model`
    # differs, and only by which spatial reference it reads; see
    # `room_m.exporters.ffe.export_model_by_space`.
    FFE_BY_SPACE: EntityExporter(
        export_model=ffe_exporter.export_model_by_space,
        post_bucket=ffe_exporter.post_bucket,
        stamp_envelope=ffe_exporter.stamp_envelope,
    ),
    CEILINGS: EntityExporter(
        export_model=ceilings_exporter.export_model,
        post_bucket=ceilings_exporter.post_bucket,
        stamp_envelope=ceilings_exporter.stamp_envelope,
    ),
    FLOORS: EntityExporter(
        export_model=floors_exporter.export_model,
        post_bucket=floors_exporter.post_bucket,
        stamp_envelope=floors_exporter.stamp_envelope,
    ),
}


def rooms_export_entry(doc, uiapp, output, forms):
    """Push ROOMS AND THEN DOORS -- the full push, and the original entry point.

    Kept combined under its original name deliberately. Narrowing it to rooms
    would not fail -- it would keep succeeding while quietly no longer pushing
    doors, which is the worst shape a behaviour change can take. The split
    lives in the two siblings below instead, where wiring a new button is what
    opts into it.

    The BUTTON is called "Rooms + Doors" (`RoomsAndDoors.pushbutton`), which is
    the half of this that a modeller can see. It used to be called Export, over
    a function named for rooms, in a repository that held neither.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (ROOMS, DOORS))


def rooms_only_export_entry(doc, uiapp, output, forms):
    """Push ROOMS alone, leaving whatever doors the model already has on the
    server untouched.

    For re-pushing rooms after a plan change without paying for the door export
    and its per-door Revit room-reference reads, which are the slow half of a
    combined run.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (ROOMS,))


def doors_export_entry(doc, uiapp, output, forms):
    """Push DOORS alone.

    A doors push carries no room data, only room *ids*, so it never needed the
    rooms re-sent. It used to need them to already be *there* -- the server
    refused otherwise -- and that is no longer true either: the server resolves a
    door's rooms itself and reports "not yet" rather than refusing. So this entry
    is now genuinely independent, and doors may be pushed before their rooms.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (DOORS,))


def windows_export_entry(doc, uiapp, output, forms):
    """Push WINDOWS alone.

    A windows push carries no room data, only room *ids* -- and in a facade
    model, usually not even those, because Revit cannot resolve a room across a
    link. It needs nothing on the server first: an unresolvable or absent
    reference is reported rather than refused, so windows may be pushed before
    their rooms or without them entirely.

    **Its button is wired in this repository**, under
    `extractor/pyRevit/RoomMate.extension`, which is what stops an entity being
    added without one. Widening an existing entry point instead would be worse:
    `rooms_export_entry` still pushes rooms AND doors despite its name, and
    quietly adding windows to it would keep succeeding while changing what every
    existing button does.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (WINDOWS,))


def ffe_export_entry(doc, uiapp, output, forms):
    """Push FF&E alone.

    An FF&E push carries no room data, only room *ids* -- and unlike a windows
    push in a facade model, it usually does carry them: FF&E lives in the same
    document as the rooms it serves, which is the premise of the entity. Revit
    cannot schedule FF&E against those rooms, and this is the push that lets
    RoomMate do it instead.

    It needs nothing on the server first: an unresolvable or absent reference is
    reported rather than refused, so FF&E may be pushed before its rooms.

    **Its button is wired in this repository**, under
    `extractor/pyRevit/RoomMate.extension`. Widening an existing entry instead
    would be worse: `rooms_export_entry` still pushes rooms AND doors despite
    its name, and quietly adding a third entity to it would keep succeeding
    while changing what every existing button does.

    **A combined rooms-and-FF&E entry would be genuinely useful** and is
    deliberately not added here. The two live in one document, so one run could
    read it once and push both -- but that is a new entry point and a new button,
    one line of `export_entry(..., (ROOMS, FFE))`, and adding it before anyone
    has asked would be a third unwired button rather than a saving.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (FFE,))


def ffe_by_space_export_entry(doc, uiapp, output, forms):
    """Push FF&E alone, identified against SPACES rather than rooms.

    The entry a services model uses: `ffe_export_entry` reads
    `FamilyInstance.get_Room(phase)`, which resolves nothing for every
    instance in a document that holds no Room elements at all -- the ordinary
    shape of a fit-out services model, which holds Spaces instead. This entry
    reads `get_Space(phase)` in the same way and writes what it finds to
    `owner_spaces` rather than `room`.

    **A separate entry point rather than a flag that probes the document.**
    Which spatial element a model actually carries is a project fact this run
    does not detect; `ffe_export_entry` and this one are two explicit choices,
    on the same terms `rooms_export_entry` and `rooms_only_export_entry` are.
    Reading both references per instance was rejected for the same reason
    `rooms_only_export_entry` exists: it is the slow half of a push, doubled,
    for a value this run was not asked for.

    Still pushes to the same server bucket as `ffe_export_entry` -- FF&E is one
    entity on the wire regardless of which reference identified it, and
    `owner_spaces` rides every item alongside `room`, whichever one a given
    run populated. It needs nothing on the server first, for the same reason
    `ffe_export_entry` does not: nothing server-side joins `owner_spaces` to
    anything, so there is nothing for it to arrive too early for.

    **Its button is wired in this repository**, under
    `extractor/pyRevit/RoomMate.extension`.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (FFE_BY_SPACE,))


def spaces_export_entry(doc, uiapp, output, forms):
    """Push SPACES alone.

    The entry a services model uses. A spaces push carries no room references at
    all -- not even ids -- because a space is matched to a room by a value
    somebody chose, on the server, across models. So it needs nothing on the
    server first and cannot be pushed too early.

    **Two things about a spaces run differ from every other entity's, and both
    are worth knowing before pressing the button.**

    A run pushes ONE phase, and the disciplines do not agree on what to call it.
    Measured on RHH: the mechanical model keeps 1,532 of its 1,533 spaces in a
    phase called `Future` while the hydraulic, fire and electrical models use
    `New Construction`. `choose_phase` offers only names common to every selected
    document, so a run over all four under the common name pushes three models in
    full and one space from the fourth. That is a correct push of a phase that
    model barely uses, and nothing about it is an error -- which is why
    `exporters.spaces.export_model` reports "N of M spaces are in phase X" for
    every model, every run. **Read that line.** Two runs, one per phase, is the
    answer when it looks wrong.

    An empty bucket is sent rather than refused, unlike every other entity's.
    "This services model was audited and holds no spaces" is the finding, and a
    different fact from "it was never pushed".

    **Its button is wired in this repository**, under
    `extractor/pyRevit/RoomMate.extension`. Widening an existing entry instead
    would be worse: `rooms_export_entry` still pushes rooms AND doors despite
    its name, and quietly adding a fifth entity to it would keep succeeding
    while changing what every existing button does.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, (SPACES,))



def ceilings_export_entry(doc, uiapp, output, forms):
    """Push CEILINGS alone.

    **A ceilings push carries no room reference at all, so it can never be too
    early.** Doors and windows name their rooms by id and FFE names one; a
    Revit ceiling has no room parameter and a room has no ceiling parameter, so
    the association is derived from polygon overlap on the server, on every
    read. Push ceilings before their rooms and nothing dangles -- the ceilings
    simply attribute to nothing until the rooms land, and then they do.

    **The phase filter here is the DOORS range test, not the rooms equality
    test.** A ceiling is built in one phase and may be demolished in a later
    one. Running it through the rooms predicate would return nothing, silently.
    House A cannot catch that mistake -- no ceiling there is demolished, so the
    two tests agree on that document by accident -- which is why it is written
    down in `exporters.ceilings` rather than left to a passing run.

    **A ceiling whose geometry duHast cannot measure is still pushed**, with an
    empty polygon, so "no such ceiling" and "a ceiling nobody could measure"
    stay distinguishable. The exporter fails the model if the export comes back
    SHORT of the collector's count, which is what an older duHast does: check
    which duHast the extension is running before trusting such a push.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, entities=(CEILINGS,))


def floors_export_entry(doc, uiapp, output, forms):
    """Push FLOORS alone.

    Everything `ceilings_export_entry` says holds here, and for the same
    reasons: a floor carries no room reference, so it cannot be pushed too
    early; its phase filter is the range test; an unmeasurable floor is still
    pushed with an empty polygon; and a short export fails the model.

    **Two things are worth knowing before pressing the button on a real
    project.** The category is `OST_Floors`, so structural slabs are pushed
    alongside finish floors -- the `Structural` parameter tells them apart on
    the server. And the room association is MODEL-SCOPED: a floor is only ever
    attributed to rooms in its own document, so if a project keeps its slabs
    in a base-build model and its rooms in fit-out models, every slab reports
    no room. That is a reported state, not a failure, and it is the first
    thing a floors probe on such a project should look at.

    **Its button is wired in this repository**, like every entry point's since
    the bundles moved here.

    :return: Result object with status and message.
    :rtype: Result
    """
    return export_entry(doc, uiapp, output, forms, entities=(FLOORS,))

def _single_document(doc, forms, button_name):
    """The ONE document a sheet run works in, or None. `pick_document` is multiselect-capable;
    a sheet run names rooms and sheets of one document, so more than one is refused rather
    than guessed at."""
    picked = pick_document(doc, forms, button_name=button_name, multiselect=True)
    if not picked:
        return None
    if not isinstance(picked, (list, tuple)):
        picked = [picked]
    if len(picked) != 1:
        forms.alert(
            "Pick exactly one model: the rooms and the sheets you choose belong to one document.",
            title="Roommate - export aborted", warn_icon=True)
        return None
    return picked[0]


def _report_tags(project_id, pushed, return_value):
    """Say which tags could not be read, then send the ones that were to the tag rules store.
    A failure here is reported and never stops the sheet push: the layout does not need its tags."""
    for p in pushed:
        for line in describe_problems(p.get("sheet_number", "?"), p.get("tag_problems")):
            return_value.append_message(line)
    observations = all_observations(pushed)
    if not observations:
        return
    ok, status, text, outcome = post_observations(project_id, observations)
    if not ok:
        return_value.append_message(
            "Tag rules not updated ({}): {}".format(status if status is not None else "no response", text))
        return
    for line in describe_tag_outcome(outcome):
        return_value.append_message(line)


def _push_sheets(project, assignments, selected_doc, phase_name, return_value):
    """Read the chosen sheets with duHast and push them to the sheet library, reporting what
    the server did with each. `assignments` is a list of `(sheet entry, room or None)`."""
    pushed, failures = read_sheets(selected_doc, assignments, phase_name)
    for label, reason in failures:
        return_value.update_sep(False, "{}: could not be read: {}".format(label, reason))
    if not pushed:
        return_value.update_sep(False, "No sheet could be read, so nothing was pushed.")
        return
    ok, status, text, outcomes = post_sheets(project["id"], pushed)
    if not ok:
        return_value.update_sep(
            False, "Sheet push failed ({}): {}".format(status if status is not None else "no response", text))
        return
    for line in describe_outcomes(outcomes):
        return_value.append_message(line)
    return_value.append_message("{} sheet(s) pushed".format(len(pushed)))
    # after the sheets: tags are only worth keeping from sheets the server accepted
    _report_tags(project["id"], pushed, return_value)


def sheets_export_entry(doc, uiapp, output, forms):
    """Push SHEETS you choose to the sheet library, with no room attached.

    The sheets go in as they are; the room each was made for is set afterwards, in the
    viewer. Nothing is inferred from sheet names or parameters: the export has no marker
    for a room layout sheet, so the person picks. Use the Blueprint button to name rooms
    at the same time.

    Not a snapshot push and not an entry in `ENTITY_EXPORTERS`: the library is authored
    data the server keeps by sheet identity (Revit sheet id plus document title).

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_doc = _single_document(doc, forms, "Select the model whose sheets to export")
        if selected_doc is None:
            return_value.append_message("No document selected")
            return return_value
        project = choose_project(forms)
        if project is None:
            return_value.update_sep(False, "Export aborted: no project selected.")
            return return_value

        sheets = sheets_of(selected_doc)
        if not sheets:
            forms.alert("This model has no sheets.", title="Roommate - nothing to export", warn_icon=True)
            return_value.update_sep(False, "Export aborted: the model has no sheets.")
            return return_value
        chosen = pick_items(forms, sheets, "Select the sheets to export", "Export these sheets")
        if not chosen:
            return_value.update_sep(False, "Export aborted: no sheets selected.")
            return return_value

        _push_sheets(project, [(entry, None) for entry in chosen], selected_doc, None, return_value)
    except Exception as e:
        return_value.update_sep(False, "Sheet export failed with exception: {}".format(e))
    return return_value


def blueprint_export_entry(doc, uiapp, output, forms):
    """The BLUEPRINT push: a phase, then the rooms, then the sheets that belong to each.

    1. **Phase.** The rooms offered are the placed rooms of the phase you pick.
    2. **Rooms.** The rooms you want a blueprint from.
    3. **Sheets, room by room.** For each chosen room, its room layout sheets. **A room with
       no sheet is not pushed**: a blueprint is a room and its sheets together, and the server
       refuses one without. A sheet is offered once: one given to an earlier room is not
       offered again.
    4. **Name.** One per room, defaulting to the room's name. A name the project already has
       becomes a pending revision of that blueprint, to accept or reject in the viewer.

    The room's contents are NOT sent: the server captures them from the rooms and FF&E it
    already holds, so push those first with the Rooms and FFE buttons. A sheets-only push to the
    library is the Sheets button (docs/PLAN-blueprints.md, decisions 18 and 19).

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_doc = _single_document(doc, forms, "Select the model holding the rooms and sheets")
        if selected_doc is None:
            return_value.append_message("No document selected")
            return return_value
        project = choose_project(forms)
        if project is None:
            return_value.update_sep(False, "Export aborted: no project selected.")
            return return_value

        # 1. phase
        phase_name = choose_phase([selected_doc], forms)
        if phase_name is None:
            return_value.update_sep(False, "Export aborted: no phase selected.")
            return return_value
        return_value.append_message("Phase '{}'".format(phase_name))

        # 2. rooms
        rooms = placed_rooms(selected_doc, phase_name)
        if not rooms:
            forms.alert("No placed rooms in phase '{}'.".format(phase_name),
                        title="Roommate - nothing to export", warn_icon=True)
            return_value.update_sep(False, "Export aborted: no placed rooms in that phase.")
            return return_value
        chosen_rooms = pick_items(forms, rooms, "Select the rooms to make blueprints from", "Use these rooms")
        if not chosen_rooms:
            return_value.update_sep(False, "Export aborted: no rooms selected.")
            return return_value

        # 3. sheets, room by room
        available = sheets_of(selected_doc)
        if not available:
            forms.alert("This model has no sheets, and a blueprint needs its room layout sheets.",
                        title="Roommate - nothing to export", warn_icon=True)
            return_value.update_sep(False, "Export aborted: the model has no sheets.")
            return return_value
        plan = []
        taken = set()
        for room in chosen_rooms:
            remaining = [s for s in available if s["id"] not in taken]
            chosen = pick_items(
                forms, remaining,
                "Sheets for room {} (at least one)".format(room["label"]),
                "Use for this room") if remaining else []
            if chosen is None:
                return_value.update_sep(False, "Export aborted at room {}.".format(room["label"]))
                return return_value
            if not chosen:
                return_value.append_message(
                    "{}: skipped, a blueprint needs at least one room layout sheet".format(room["label"]))
                continue
            for entry in chosen:
                taken.add(entry["id"])
            # 4. name
            name = forms.ask_for_string(
                default=room["name"] or room["label"],
                prompt="Blueprint name for room {}".format(room["label"]),
                title="Roommate - blueprint name")
            if name is None:
                return_value.update_sep(False, "Export aborted at room {}.".format(room["label"]))
                return return_value
            name = name.strip()
            if not name:
                return_value.append_message("{}: skipped, no blueprint name".format(room["label"]))
                continue
            plan.append((room, name, chosen))

        if not plan:
            return_value.update_sep(False, "Nothing to push: no room had both a sheet and a name.")
            return return_value

        for room, name, chosen in plan:
            pushed, failures = read_sheets(selected_doc, [(entry, None) for entry in chosen], None)
            for label, reason in failures:
                return_value.update_sep(False, "{}: could not be read: {}".format(label, reason))
            if failures or not pushed:
                # A blueprint with some of its sheets would read as the whole layout.
                return_value.update_sep(
                    False, "{}: not pushed, because not every chosen sheet could be read.".format(name))
                continue
            ok, status, text, outcome = post_blueprint(
                project["id"], name, room["id"], selected_doc.Title, selected_doc.Title, pushed)
            if not ok:
                return_value.update_sep(False, "{}: {}".format(name, explain_refusal(status, text)))
                continue
            return_value.append_message(describe_outcome(name, outcome))
            _report_tags(project["id"], pushed, return_value)
    except Exception as e:
        return_value.update_sep(False, "Blueprint export failed with exception: {}".format(e))
    return return_value


def server_entry(doc, uiapp, output, forms):
    """Choose which RoomMate server this model and this computer's extractor talk to.

    The server runs on this computer unless it was started to serve a network (`--bind 0.0.0.0`);
    to push from Revit to a server on ANOTHER computer, type the address it printed at startup
    (`192.168.0.97`, or `192.168.0.97:5151`, or its `.local` name). The address is checked by asking
    that server for its projects BEFORE it is kept. It is kept in **two places**: in the open
    model's extensible storage (`room_m.project_settings`), so anyone opening the model finds it,
    and in `server.json` in the RoomMate folder of the user's application data, for a computer that
    opens a model that does not have it yet. Later runs read the model first, then the file, and a
    computer with neither is told to come here (`room_m.server_config`). Typing `local` chooses the
    server on this computer; `reset` forgets the kept address in both places. The `ROOMMATE_SERVER`
    environment variable, when set, wins over both and the run says so.

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        current, source = server_config.resolve()
        typed = forms.ask_for_string(
            default=current or "",
            prompt="Address of the RoomMate server (for example 192.168.0.97 or mac.local:5151).\n"
                   "Type 'local' for the server on this computer, 'reset' to forget the kept address.",
            title="Roommate - server")
        if typed is None:
            return_value.append_message("Server unchanged: {}".format(server_config.describe()))
            return return_value
        typed = typed.strip()
        word = typed.lower()
        resetting = word in server_config.RESET_WORDS
        answer = None
        ok = False
        candidate = None
        if not resetting:
            candidate = server_config.LOCAL_URL
            if word not in server_config.LOCAL_WORDS:
                try:
                    candidate = server_config.normalise(typed)
                except ValueError as e:
                    forms.alert(str(e), title="Roommate - server address", warn_icon=True)
                    return_value.update_sep(False, "Server unchanged: {}".format(e))
                    return return_value
            ok, status, answer = fetch_projects(candidate + SERVER_URL_PROJECTS)
            if not ok:
                keep = forms.alert(
                    "{}\n\nKeep this address anyway?".format(answer),
                    title="Roommate - server not reachable", yes=True, no=True, warn_icon=True)
                if not keep:
                    return_value.update_sep(False, "Server unchanged: {}".format(answer))
                    return return_value
        kept = server_config.save(typed)
        # the same address in the model, so whoever opens it next needs no set-up
        stored, why = project_settings.write_server(doc, kept)
        if resetting:
            return_value.append_message("Server forgotten: this computer's address is removed")
        else:
            return_value.append_message("Server set to {} (this computer: {})".format(kept, server_config.config_path()))
            return_value.append_message(
                "{} project(s) registered there".format(len(answer)) if ok else "Not reachable now: {}".format(answer))
        return_value.append_message(
            "Model: {}".format(why) if stored else "Model: not stored ({}); the address is kept for this computer only".format(why))
        in_force = server_config.resolve()[0]
        if kept and in_force != kept:
            return_value.append_message(
                "NOTE: the {} environment variable is set, so {} is what is used until it is removed".format(
                    server_config.ENV_VAR, in_force))
    except Exception as e:
        return_value.update_sep(False, "Server setting failed with exception: {}".format(e))
    return return_value


def _edit_rls_settings(selected_doc, forms, return_value):
    """Show the RLS settings window for a model and store what the person saves in the model.

    Reads the model's settings (extensible storage, key `rls`) and the choices the model offers,
    shows `room_m.rls_settings_window`, and writes the result back. Returns the saved settings, or
    None when the window was cancelled or the model could not take them (and says why on
    `return_value`). Shared by the RLS settings button and the RLS create button, which sends a
    person here when the settings are not complete."""
    from room_m import rls_settings_window

    current = project_settings.get_section(project_settings.read_text(selected_doc), rls_settings.PROJECT_KEY) or {}
    options = rls_settings.collect_options(selected_doc)
    saved = rls_settings_window.show(forms, selected_doc.Title, current, options)
    if saved is None:
        return_value.append_message("RLS settings unchanged: the window was cancelled")
        return None
    stored, why = project_settings.write_section(selected_doc, rls_settings.PROJECT_KEY, saved)
    if not stored:
        return_value.update_sep(False, "The settings were not stored in the model: {}".format(why))
        return None
    return_value.append_message("RLS settings stored in the model '{}'".format(selected_doc.Title))
    return saved


def rls_settings_entry(doc, uiapp, output, forms):
    """Set how room layout sheets are made in a model, in a window.

    Pick the model, then choose the title block per sheet size, the plan and elevation view types, the
    base schedule, the tags and the number and name logic (`{Room Parameter}` templates) in the window
    (`room_m.rls_settings_window`). What is saved goes into the model's extensible storage, where the
    RLS create button reads it. Nothing is sent to the server.

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_doc = _single_document(doc, forms, "Select the model the settings are for")
        if selected_doc is None:
            return_value.append_message("No document selected")
            return return_value
        saved = _edit_rls_settings(selected_doc, forms, return_value)
        if saved is not None:
            todo = rls_settings.missing(saved)
            if todo:
                return_value.append_message("Still to choose before an RLS can be made: {}".format(", ".join(todo)))
    except Exception as e:
        return_value.update_sep(False, "RLS settings failed with exception: {}".format(e))
    return return_value


def rls_create_entry(doc, uiapp, output, forms):
    """Make room layout sheets (RLS) from the server's `create_rls` tasks for this model.

    1. **Settings first.** The model's RLS settings are read (extensible storage). Missing or incomplete
       ones open the settings window; if they are still incomplete afterwards the script stops.
    2. **Ask the server** for this model's queued `create_rls` tasks (and only those: a populate task
       is the Apply button's). No answer, or no tasks, is said and the script stops.
    3. **Check the whole list** before anything is made (`room_m.rls_task`): an existing sheet or view, no
       title block for the size, a view type or the base schedule missing are aborts reported first;
       a missing room or independent tag is a warning, put to the person once.
    4. **Make each sheet** in one transaction group, so a failed one leaves no trace, and report it as
       soon as it is done (`room_m.rls_write`, which calls duHast for the Revit work).

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_doc = _single_document(doc, forms, "Select the model to make room layout sheets in")
        if selected_doc is None:
            return_value.append_message("No document selected")
            return return_value
        settings = project_settings.get_section(project_settings.read_text(selected_doc), rls_settings.PROJECT_KEY) or {}
        todo = rls_settings.missing(settings)
        if todo:
            forms.alert(
                "The RLS settings of this model are not complete yet:\n\n{}\n\nThe settings window opens next.".format(", ".join(todo)),
                title="Roommate - RLS settings")
            settings = _edit_rls_settings(selected_doc, forms, return_value) or {}
            todo = rls_settings.missing(settings)
            if todo:
                return_value.update_sep(False, "RLS stopped: the settings are still incomplete ({}).".format(", ".join(todo)))
                return return_value

        project = choose_project(forms)
        if project is None:
            return_value.update_sep(False, "RLS aborted: no project selected.")
            return return_value

        ok, status, text, tasks = claim_tasks(project["id"], selected_doc.Title, kinds=["create_rls"])
        if not ok:
            return_value.update_sep(
                False, "The server did not answer ({}): {}".format(status if status is not None else "no response", text))
            return return_value
        if not tasks:
            forms.alert("There are no queued room layout sheet tasks for model '{}'.".format(selected_doc.Title),
                        title="Roommate - nothing to make")
            return_value.append_message("No queued create_rls tasks for '{}'".format(selected_doc.Title))
            return return_value
        return_value.append_message("{} task(s) claimed for '{}'".format(len(tasks), selected_doc.Title))

        from room_m import rls_task
        from room_m.apply_run import run
        from room_m.rls_write import RlsFacts, RlsWriter

        facts = RlsFacts(selected_doc)
        writer = RlsWriter(selected_doc, facts)

        def ask_proceed(lines):
            message = "Some sheets have warnings:\n\n" + "\n".join(lines) + "\n\nProceed with them, or abort those tasks?"
            return forms.alert(message, title="Roommate - warnings", options=["Proceed", "Abort those tasks"]) == "Proceed"

        def send(reports):
            ok, status, text, outcomes = report_tasks(project["id"], reports)
            if not ok:
                return False, "{}: {}".format(status if status is not None else "no response", text)
            bad = [o.get("error") or "not recorded" for o in outcomes if not o.get("recorded")]
            return (not bad), "; ".join(bad)

        def say(line):
            try:
                output.print_md(line)
            except Exception:
                pass

        sent, lines = run(
            tasks, facts, writer, ask_proceed, send, say,
            planner=lambda claimed: rls_task.plan_tasks(claimed, settings, facts))
        failed = [r for r in sent if r["status"] in ("failed", "aborted")]
        return_value.append_message("{} report(s) sent, {} failed or aborted".format(len(sent), len(failed)))
    except Exception as e:
        return_value.update_sep(False, "RLS creation failed with exception: {}".format(e))
    return return_value


def apply_tasks_entry(doc, uiapp, output, forms):
    """APPLY the room tasks the server holds for this model.

    1. **Claim.** Every queued task for this model is handed over and marked in progress.
    2. **Check the whole list** before anything is written (`room_m.task_plan`). A task that
       cannot run is reported aborted straight away.
    3. **Ask once** if some tasks only have warnings: proceed with them, or abort those.
    4. **Write each task** in one Revit transaction group, so a task that fails leaves no trace,
       and report it as soon as it is done.

    The plug-in only validates and writes; it never resolves a layout (the task already holds
    the placements). See `room_m.apply_run`, `room_m.apply_write` and docs/PLAN-blueprints.md.

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_doc = _single_document(doc, forms, "Select the model the tasks are for")
        if selected_doc is None:
            return_value.append_message("No document selected")
            return return_value
        project = choose_project(forms)
        if project is None:
            return_value.update_sep(False, "Apply aborted: no project selected.")
            return return_value

        ok, status, text, tasks = claim_tasks(project["id"], selected_doc.Title, kinds=["populate_room", "update_room"])
        if not ok:
            return_value.update_sep(
                False, "Could not ask the server for tasks ({}): {}".format(status if status is not None else "no response", text))
            return return_value
        if not tasks:
            forms.alert("There are no queued populate or update tasks for model '{}'.".format(selected_doc.Title),
                        title="Roommate - nothing to apply")
            return_value.append_message("No queued tasks for '{}'".format(selected_doc.Title))
            return return_value
        return_value.append_message("{} task(s) claimed for '{}'".format(len(tasks), selected_doc.Title))

        from room_m.apply_facts import RevitFacts
        from room_m.apply_write import TaskWriter
        from room_m.apply_run import run

        facts = RevitFacts(selected_doc)
        writer = TaskWriter(selected_doc, facts)

        def ask_proceed(lines):
            message = "Some tasks have warnings:\n\n" + "\n".join(lines) + "\n\nProceed with them, or abort those tasks?"
            choice = forms.alert(message, title="Roommate - warnings", options=["Proceed", "Abort those tasks"])
            return choice == "Proceed"

        def send(reports):
            ok, status, text, outcomes = report_tasks(project["id"], reports)
            if not ok:
                return False, "{}: {}".format(status if status is not None else "no response", text)
            bad = [o.get("error") or "not recorded" for o in outcomes if not o.get("recorded")]
            return (not bad), "; ".join(bad)

        def say(line):
            try:
                output.print_md(line)
            except Exception:
                pass

        sent, lines = run(tasks, facts, writer, ask_proceed, send, say)
        failed = [r for r in sent if r["status"] in ("failed", "aborted")]
        return_value.append_message("{} report(s) sent, {} failed or aborted".format(len(sent), len(failed)))
    except Exception as e:
        return_value.update_sep(False, "Apply failed with exception: {}".format(e))
    return return_value


def export_all_entry(doc, uiapp, output, forms):
    """Every export in one window: what to push, to which project, under which phase, and how much.

    1. Pick the models (as the other export buttons do).
    2. Read what the window needs: the server's projects, the phases the models share, and their levels.
    3. Show the window (`room_m.export_window`): check boxes for the entities, the project and phase, and
       the whole model or chosen levels. A level choice is by name across the models and becomes a
       level-scoped push per model (`scope` on the model block; the server keeps the other levels).
    4. Run the export as the single-entity buttons do (`_run_export`).

    The Revit export itself still reads whole models (duHast has no level filter yet); a level choice
    narrows what is SENT and replaced, not what Revit reads.

    **Not yet run in Revit.**

    :return: Result object with status and message.
    :rtype: Result
    """
    return_value = Result()
    try:
        report_build(return_value, output)
        selected_docs = pick_document(doc, forms, button_name="Select model(s) to export", multiselect=True)
        if not selected_docs:
            return_value.append_message("No document(s) selected")
            return return_value

        from room_m import export_form, export_window
        from room_m.post_common import fetch_projects
        from room_m.utils.generic import common_phase_names, document_levels

        ok, status, payload = fetch_projects()
        if not ok:
            forms.alert("Could not load projects from the server.\n\n{}".format(payload),
                        title="Roommate - export aborted", warn_icon=True)
            return_value.update_sep(False, "Export aborted: {}".format(payload))
            return return_value
        if not payload:
            return_value.update_sep(False, "Export aborted: the server has no registered projects.")
            return return_value
        phases = common_phase_names(selected_docs)
        if not phases:
            forms.alert("The selected models share no phase name, so there is no single phase to export. "
                        "Export them separately.", title="Roommate - export aborted", warn_icon=True)
            return_value.update_sep(False, "Export aborted: no common phase.")
            return return_value

        choice = export_window.show(forms, selected_docs, payload, phases, [document_levels(d) for d in selected_docs])
        if choice is None:
            return_value.append_message("Export cancelled")
            return return_value

        return_value.append_message(export_form.summary(
            choice["entities"], choice["project"]["name"], choice["phase"], selected_docs, choice["levels"]))
        _run_export(
            selected_docs, choice["project"], choice["phase"], choice["entities"],
            return_value, forms, levels=choice["levels"])
    except Exception as e:
        return_value.update_sep(False, "Export failed with exception: {}".format(e))
        print("Export failed with exception: {}".format(e))
    return return_value


def export_entry(doc, uiapp, output, forms, entities):

    """
    Exports `entities` from the selected Revit document(s) and pushes them.

    The single run driver behind all three entry points: document selection,
    the one project, the one phase, and the per-model loop are identical whether
    a run pushes rooms, doors or both, so they are written once. `entities` is
    the only thing that varies, and it varies in one place -- what
    `export_model` reads per document.

    **Every selected model is exported first, then each entity is pushed once.**
    A run used to be N pushes per entity, one per model, which meant a doors push
    depended on whether its siblings had already landed and gave the run's models
    N snapshot ids minutes apart. Now the run reads everything, then sends one
    bucket per entity under one snapshot id.

    :param doc: Current Revit model document.
    :type doc: Autodesk.Revit.DB.Document
    :param output: pyRevit output.
    :type output: pyRevit.output
    :param forms: pyRevit forms.
    :type forms: pyRevit.forms
    :param entities: which `ENTITY_EXPORTERS` keys this run pushes. Order is
        no longer meaningful -- the buckets are independent, see
        `ENTITY_EXPORTERS`.
    :type entities: tuple

    :return: Result object with status and message.
    :rtype: Result
    """

    # set up a status tracker
    return_value = Result()

    try:

        # Which RoomMate and which duHast are about to run, recorded BEFORE a
        # document is picked. Three separate investigations have started with
        # "check which duHast the extension is running", and until this line
        # existed the answer had to be reconstructed from files on disk -- by
        # which time the session that produced the export was long gone. It
        # never fails a run: an unexpected duHast is usually a dev setup.
        report_build(return_value, output)

        # ask user to select active or linked document
        selected_docs = pick_document(
            doc, forms,
            button_name="Select model to collect {} data from".format(entities_label(entities)),
            multiselect=True)
        if not selected_docs or len(selected_docs) == 0:
            return_value.append_message("No document(s) selected")
            return return_value

        # pick ONE server-registered project for this run: every selected
        # model posts under it (matching "several models under one project").
        # None means abort -- server unreachable, nothing registered, or the
        # user cancelled -- so nothing is exported or pushed.
        project = choose_project(forms)
        if project is None:
            return_value.update_sep(False, "Push aborted: no project selected.")
            return return_value

        # Pick ONE phase for the whole run (see choose_phase). Every selected
        # model is filtered to it and declares it on the envelope; the server
        # refuses a push that declares none, and a model whose declared phase
        # disagrees with what it was first pushed under is quarantined rather
        # than made live. Asked once, after the project, because it is a
        # property of what is being pushed rather than of where it goes.
        phase_name = choose_phase(selected_docs, forms)
        if phase_name is None:
            return_value.update_sep(False, "Push aborted: no phase selected.")
            return return_value
        return_value.append_message("Pushing phase '{}'".format(phase_name))

        _run_export(selected_docs, project, phase_name, entities, return_value, forms)

    except Exception as e:
        message = "Failed to export {} data with exception: {}".format(
            entities_label(entities), e)
        return_value.update_sep(False, message)
        print(message)

    print("Finished")

    return return_value


def _level_ids(levels, selected_doc):
    """The ids of `selected_doc`'s levels named in `levels` (None for the whole model), or None for no
    scope. An empty list is a model with none of the chosen levels: the caller leaves it out."""
    if levels is None:
        return None
    from room_m import export_form
    from room_m.utils.generic import document_levels

    return export_form.scope_for_doc(document_levels(selected_doc), levels)


def _run_export(selected_docs, project, phase_name, entities, return_value, forms, levels=None):
    """Export `entities` from `selected_docs` and push them to `project` under `phase_name`.

    The part of a run that is the same whichever way the choices were made (the buttons that ask in
    turn, or the export window that asks once). `levels` is None to push whole models, or a list of level
    NAMES: each model then pushes a level-scoped push of its own ids of those levels
    (`utils.phase_filter.in_scope`), and a model that has none of them is left out and said so."""
    # One envelope for the whole run: one project, one phase, one moment
    # of reading. Built before the loop so every model in the push shares a
    # snapshot id -- which is what makes "these documents were read
    # together" a thing the store can express.
    run_envelope = build_run_envelope(project, phase_name)

    # get going
    model_counter = 0

    # One bucket per entity, filled model by model and posted once at the
    # end. The export is the slow half and happens per document; the push is
    # one request per entity for the whole run.
    buckets = dict((entity, []) for entity in entities)

    # Read inside the `with` and acted on outside it, rather than reaching
    # for `pb.cancelled` after the progress bar has exited.
    cancelled = False

    # set up a progress bar
    with forms.ProgressBar(
        title="Exporting model: {value} of {max_value}", cancellable=True
    ) as pb:

        # get data for each selected document. Each model's export runs in
        # its own try: one model failing (an export exception, a broken
        # envelope) must not abandon the remaining models. It costs that
        # model's contribution to every bucket and nothing else -- the run
        # still pushes what it did read, and still ends red.
        for selected_doc in selected_docs:

            # update progress bar
            model_counter += 1
            pb.update_progress(model_counter, max_value=len(selected_docs))

            try:
                level_ids = _level_ids(levels, selected_doc)
                if level_ids is not None and not level_ids:
                    # none of the chosen levels exist here: pushing it unscoped would replace the model's
                    # whole snapshot with nothing, so it is left out
                    return_value.append_message(
                        "{}: has none of the chosen level(s), so it was left out".format(selected_doc.Title))
                else:
                    export_model(
                        selected_doc, phase_name, return_value, entities, buckets,
                        level_ids=level_ids)
            except Exception as e:
                return_value.update_sep(
                    False, "{}: failed with exception: {}".format(selected_doc.Title, e)
                )

            # check for cancel
            if pb.cancelled:
                cancelled = True
                return_value.update_sep(False, "User cancelled.")
                break

    # A cancelled run pushes nothing. The alternative -- sending the models
    # read so far -- would store a partial run under one snapshot id, which
    # reads downstream as "these are the documents that were exported
    # together" and would be a lie. Someone who cancels wants nothing sent.
    if cancelled:
        return return_value

    for entity in entities:
        entries = buckets[entity]
        if not entries:
            # Nothing was read for this entity at all -- every model failed,
            # and each failure is already on `return_value`. Posting an empty
            # bucket would only add a second, vaguer complaint.
            continue
        ENTITY_EXPORTERS[entity].post_bucket(run_envelope, entries, return_value)



def export_model(selected_doc, phase_name, return_value, entities, buckets, level_ids=None):
    """Export one model's `entities` into the run's `buckets`, scoped to
    `phase_name`, recording any failure on `return_value`.

    Raises on envelope failures -- the caller catches per model so one bad model
    doesn't abandon the rest. An *entity's* export failure is not a raise: the
    exporter records it and answers None, so an unreadable door set does not also
    cost this model's rooms.

    **There is no ordering here, and there used to be.** Rooms had to be pushed
    before doors for the same model, because the server refused a doors push to a
    model with no rooms. It no longer does, so the entities are independent and
    the loop below is genuinely a loop rather than a sequence with a rule in it.

    `level_ids` (the ids of this model's levels to replace, or None for the whole model) is stamped on
    the block as its `scope`.

    Each entity gets its own deep copy of the model's envelope block: a rooms
    push stamps `room_boundary` and `levels` onto its copy and a doors push has
    no key for either, and two entities sharing one nested dict is the exact
    cross-contamination the old per-push deep copy existed to prevent."""
    block = build_model_block(selected_doc, return_value)
    if level_ids:
        # a level-scoped push: the server replaces this model's elements on these levels and keeps the
        # rest. Every exporter's post step drops an element on any other level (`phase_filter.in_scope`).
        block["scope"] = {"levels": list(level_ids)}

    for entity in entities:
        exporter = ENTITY_EXPORTERS[entity]
        entity_block = copy.deepcopy(block)

        # Only the entity actually being pushed stamps its own field. A
        # doors-only run therefore never reads the boundary regime -- which
        # would spend a document read to produce a warning about a value nothing
        # on the wire would carry.
        if exporter.stamp_envelope is not None:
            exporter.stamp_envelope(selected_doc, entity_block, return_value)

        contribution = exporter.export_model(selected_doc, phase_name, return_value, level_ids=level_ids)
        if contribution is None:
            continue
        buckets[entity].append((entity_block, contribution))
