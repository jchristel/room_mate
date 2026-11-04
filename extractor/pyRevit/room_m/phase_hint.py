"""Which phase the server already holds for the models about to be pushed.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The request is passed in
(`fetch_text`), so CPython tests it without a server (`extractor/tests/test_phase_hint.py`).

A model's phase is fixed by its first phased push and a later push that disagrees is quarantined, so the useful
default for the export window is that phase, not the one the person last picked. The server answers it for one
model at `/projects/{id}/models/{model}/snapshots/latest` (`phase`: the lineage's); a model with nothing stored
answers 404, which here is simply "no phase yet". Any other failure is also "no hint": the window is a
convenience here and must never stop an export.
"""

import json

try:
    from urllib import quote
except ImportError:  # Python 3
    from urllib.parse import quote


def latest_url(project_id, model_id):
    return "/projects/{}/models/{}/snapshots/latest".format(quote(project_id, safe=""), quote(model_id, safe=""))


def phases_held(fetch_text, project_id, model_ids):
    """The phases the server holds for `model_ids` in `project_id`, in model order, without blanks or repeats."""
    found = []
    for model_id in model_ids:
        try:
            ok, _status, text = fetch_text(latest_url(project_id, model_id))
            phase = json.loads(text).get("phase") if ok else None
        except Exception:
            phase = None
        if phase and phase not in found:
            found.append(phase)
    return found
