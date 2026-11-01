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

from room_m.post_common import _post_content, fetch_text

BASE_URL = "/projects/{}/roomtasks"
LIST_URL = "/projects/{}/tasks"


def claim_url(project_id):
    return BASE_URL.format(quote(project_id, safe=""))


def report_url(project_id):
    return claim_url(project_id) + "/report"


def _post_json(url, body):
    from System.Net.Http import StringContent
    from System.Text import Encoding

    # ensure_ascii=False, as every other push here does (IronPython 2.7 and the code page).
    return _post_content(url, StringContent(json.dumps(body, ensure_ascii=False), Encoding.UTF8, "application/json"))


def list_queued(project_id, model_id):
    """`(ok, status, text, summaries)`: the model's queued tasks as the server lists them (a summary
    each, no sheets or items), WITHOUT claiming them: this is what a person chooses from."""
    ok, status, text = fetch_text("{}?model_id={}&state=queued".format(
        LIST_URL.format(quote(project_id, safe="")), quote(model_id, safe="")))
    summaries = []
    if ok:
        try:
            summaries = json.loads(text)
        except Exception:
            ok = False
    return ok, status, text, summaries


def claim_tasks(project_id, model_id, kinds=None, task_ids=None):
    """`(ok, status, text, tasks)`: the model's queued tasks, now in progress. `kinds` narrows them
    (`["create_rls"]` for the RLS script, which must not take a populate task); absent, every kind.
    `task_ids` narrows them to the tasks a person ticked; absent, all of them."""
    body = {"model_id": model_id}
    if kinds:
        body["kinds"] = list(kinds)
    if task_ids:
        body["task_ids"] = list(task_ids)
    ok, status, text = _post_json(claim_url(project_id), body)
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
