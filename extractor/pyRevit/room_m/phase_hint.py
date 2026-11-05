"""Which phase the server already holds for the models about to be pushed.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Pure, so CPython tests it
(`extractor/tests/test_phase_hint.py`).

A model's phase is fixed by its first phased push and a later push that disagrees is quarantined, so the useful
default for the export windows is that phase, not the one the person last picked. The server sends it with the
project list itself (`model_phases` on each `/api/settings/projects` entry: model id -> phase, see
`post_common.fetch_projects`), so choosing a project in a window costs no request. A model with nothing phased
stored is simply absent: "no phase yet". The window is a convenience here and must never stop an export.
"""


def phases_held(projects, project_id, model_ids):
    """The phases the server holds for `model_ids` in `project_id`, in model order, without blanks or repeats.
    `projects` is the list `fetch_projects` returned."""
    held = {}
    for project in projects or []:
        if project.get("id") == project_id:
            held = project.get("model_phases") or {}
    found = []
    for model_id in model_ids:
        phase = held.get(model_id)
        if phase and phase not in found:
            found.append(phase)
    return found
