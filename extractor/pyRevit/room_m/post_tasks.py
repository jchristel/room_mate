"""Ask the server for a model's room tasks and report what became of them.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

`POST /projects/{id}/roomtasks {"model_id"}` hands over every queued task of the model, in
full, and marks them in progress: asking changes them, so it is a POST and a session that
asks and then dies leaves tasks a person must resend. `POST .../roomtasks/report
{"reports": [{"task_id", "status", "messages"}]}` records a status each; the server refuses
one for a task that is not in progress. The checking and the writing are not here: see
`room_m.task_plan`.
"""

import json

try:
    from urllib import quote
except ImportError:  # CPython 3
    from urllib.parse import quote

from room_m.post_common import _post_content

BASE_URL = "http://127.0.0.1:5151/projects/{}/roomtasks"


def claim_url(project_id):
    return BASE_URL.format(quote(project_id, safe=""))


def report_url(project_id):
    return claim_url(project_id) + "/report"


def _post_json(url, body):
    from System.Net.Http import StringContent
    from System.Text import Encoding

    # ensure_ascii=False, as every other push here does (IronPython 2.7 and the code page).
    return _post_content(url, StringContent(json.dumps(body, ensure_ascii=False), Encoding.UTF8, "application/json"))


def claim_tasks(project_id, model_id):
    """`(ok, status, text, tasks)`: the model's queued tasks, now in progress."""
    ok, status, text = _post_json(claim_url(project_id), {"model_id": model_id})
    tasks = []
    if ok:
        try:
            tasks = json.loads(text)
        except Exception:
            ok = False
    return ok, status, text, tasks


def report_tasks(project_id, reports):
    """`(ok, status, text, outcomes)`; an outcome that is not `recorded` names its `error`."""
    if not reports:
        return True, None, "", []
    ok, status, text = _post_json(report_url(project_id), {"reports": reports})
    outcomes = []
    if ok:
        try:
            outcomes = json.loads(text)
        except Exception:
            outcomes = []
    return ok, status, text, outcomes
