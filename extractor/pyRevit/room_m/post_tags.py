"""Send the tags read from sheets to the server's tag rules store.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

`POST /projects/{id}/tagrules/observations {"observations": [...]}`: each observation is one tag
as `room_m.sheet_tags.take_tags` made it. The server keeps one rule per key (names only) and
works out where a tag goes relative to its element; the extractor sends what it read and decides
nothing. Pushing the same sheet again replaces its samples.
"""

import json

try:
    from urllib import quote
except ImportError:  # CPython 3
    from urllib.parse import quote

from room_m.post_common import _post_content

SERVER_URL = "http://127.0.0.1:5151/projects/{}/tagrules/observations"

# A push of observations is at most this many on the server.
BATCH = 2000


def observations_url(project_id):
    return SERVER_URL.format(quote(project_id, safe=""))


def all_observations(pushed):
    """Every observation of a list of sheets read by `room_m.exporters.sheets.read_sheets`."""
    out = []
    for p in pushed:
        out.extend(p.get("tags") or [])
    return out


def post_observations(project_id, observations):
    """POST in batches and return `(ok, status, text, outcome)`. `outcome` adds the batches'
    answers up (`samples`, `rules_changed`, `skipped`); it stops at the first batch that fails."""
    from System.Net.Http import StringContent
    from System.Text import Encoding

    total = {"samples": 0, "rules_changed": 0, "skipped": []}
    for start in range(0, len(observations), BATCH):
        body = json.dumps({"observations": observations[start:start + BATCH]}, ensure_ascii=False)
        # ensure_ascii=False, as every other push here does (IronPython 2.7 and the code page).
        ok, status, text = _post_content(
            observations_url(project_id), StringContent(body, Encoding.UTF8, "application/json")
        )
        if not ok:
            return False, status, text, total
        try:
            answer = json.loads(text)
        except Exception:
            answer = {}
        total["samples"] += answer.get("samples", 0)
        total["rules_changed"] += answer.get("rules_changed", 0)
        total["skipped"].extend(answer.get("skipped") or [])
    return True, None, "", total
