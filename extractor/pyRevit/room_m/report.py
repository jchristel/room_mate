"""What a run says to the person, in lines they can read.

**Why this module exists.** A run's `Result` is a record for the caller; nothing shows it, so what a person saw
was whatever was `print`ed on the way: a server's raw JSON reply after every push, and a bare "Finished" from one
of the six buttons. This module is the one place a run speaks: an item line (`exported sheet 101 Plan ...... [ok]`),
a push line made from the server's reply, and a closing line that is the same for every button and says whether
anything went wrong. It prints with plain `print`, which pyRevit shows in the output window, so there is no
markup for a name with an underscore in it to trip over.

Pure Python (no Revit, no .NET), so the formatting is tested on CPython in `extractor/tests`.

**A line is said once.** Everything said is remembered, and `finish` prints whatever the run put on its `Result`
that was not said, so a failure recorded deep in an exporter (`update_sep(False, ...)`) is never lost to the
console just because that code path predates this module.
"""

import json

OK = "[ok]"
FAILED = "[FAILED]"
HELD = "[HELD]"
CANCELLED = "[cancelled]"

_said = set()
_baseline = [0]


def _print(text):
    """`print` as a function: IronPython 2.7's is a statement, which cannot be named or stored."""
    print(text)


_sink = [_print]


def begin(result=None, sink=None, title=None):
    """Start a run's report: forget what was said, and note how many lines `result` already holds (the build
    provenance, which `report_build` has already shown) so `finish` does not repeat them.

    With a `title`, the run's log file is started too (`room_m.task_log`: `RoomMate\\RoomMate.log`), which is where
    `server_failure` writes what a server said in full."""
    _said.clear()
    _baseline[0] = len(_lines_of(result))
    _sink[0] = sink or _print
    if title:
        try:
            from room_m import task_log

            if task_log.start(title) is None:
                say("No log file could be written: {}".format(
                    (task_log.last_error() or "unknown reason").strip().splitlines()[-1]), FAILED)
            else:
                say("Log file: {}".format(task_log.path()))
        except Exception:
            pass  # no log is not a reason to lose a run


def _lines_of(result):
    if result is None:
        return []
    try:
        return list(result.message_as_list())
    except Exception:
        return []


def say(line, state=None):
    """Print `line`, with `state` (OK, FAILED, HELD) after it when given. Returns the line as printed."""
    text = line if state is None else "{} {}".format(line, state)
    _said.add(line)
    _said.add(text)
    try:
        _sink[0](text)
    except Exception:
        pass  # an output window that cannot print is not a reason to lose a run
    return text


def item(verb, kind, label, ok=True, detail=None):
    """`exported room 101 Kitchen ... [ok]`: one thing done. A `detail` follows a failure or a hold."""
    line = "{} {} {}".format(verb, kind, label).rstrip()
    if detail:
        line += ": {}".format(detail)
    return say(line, OK if ok is True else FAILED if ok is False else ok)


def noun(count, singular, plural=None):
    """`1 room`, `26 rooms`."""
    return "{} {}".format(count, singular if count == 1 else (plural or singular + "s"))


def ingest_lines(entity, reply):
    """The lines for one entity's push, from the server's reply text: `[(state, line), ...]`.

    The reply is JSON with an `accepted` flag, a `<singular>_count` total and a `models` list whose members name
    the model, count what it stored and say why it was held (`quarantined`). A reply that is not that shape is
    shown short rather than dropped: the person should see what the server said, just not all of it."""
    try:
        data = json.loads(reply)
    except (TypeError, ValueError):
        return [(OK, "{}: the server accepted the push".format(entity))] if not reply else [
            (None, "{}: the server answered: {}".format(entity, _short(reply)))]
    if not isinstance(data, dict):
        return [(None, "{}: the server answered: {}".format(entity, _short(reply)))]

    lines = []
    models = data.get("models") or []
    for m in models:
        if not isinstance(m, dict):
            continue
        name = m.get("model_id") or m.get("id") or "model"
        count = _count_of(m)
        held = m.get("quarantined")
        if held:
            lines.append((HELD, "{}: {} - {} stored but NOT live ({})".format(
                entity, name, _counted(entity, count), held)))
        else:
            lines.append((OK, "{}: {} - {} stored".format(entity, name, _counted(entity, count))))
    if not lines:
        total = _count_of(data)
        lines.append((OK if data.get("accepted", True) else HELD,
                      "{}: {} stored".format(entity, _counted(entity, total))))
    return lines


def _count_of(d):
    for key, value in d.items():
        if key.endswith("_count") and isinstance(value, int):
            return value
    return None


def _counted(entity, count):
    return "some {}".format(entity) if count is None else "{} {}".format(count, entity if count != 1 else entity.rstrip("s"))


def _short(text, limit=160):
    text = " ".join(str(text).split())
    return text if len(text) <= limit else text[:limit - 1] + "..."


def log_path():
    """The log file in use, or None."""
    try:
        from room_m import task_log

        return task_log.path()
    except Exception:
        return None


def server_failure(what, status, reply, result=None, where=None, hint=None):
    """The server refused, or could not be reached. The full reply goes to the LOG; the person gets one line with
    the error number and where the log is, because a reply is a block of text nobody can act on from the output
    window. `status` is the HTTP code, or None when there was no answer at all. Records the failure on `result`
    (which turns the run red) and returns the line said."""
    code = "error {}".format(status) if status is not None else "no response"
    try:
        from room_m import task_log

        task_log.error("{} failed ({}){}: {}".format(
            what, code, " [{}]".format(where) if where else "", reply))
    except Exception:
        pass
    path = log_path()
    line = "{}: the server reported {}.".format(what, code) if status is not None else \
        "{}: the server could not be reached.".format(what)
    if hint:
        line += " " + hint
    line += " Details are in the log file: {}".format(path) if path else " (No log file could be written.)"
    said = say(line, FAILED)
    if result is not None:
        result.update_sep(False, said)
    return said


def say_ingest(entity, reply, result=None):
    """Say each line of a push's reply, record it on `result` too, and return whether nothing was held."""
    clean = True
    for state, line in ingest_lines(entity, reply):
        said = say(line, state)
        if result is not None:
            result.append_message(said)
        if state == HELD:
            clean = False
    return clean


def finish(result, what):
    """The closing line, the same for every button.

    Anything the run recorded on `result` that was not already said is printed first, so no failure stays only in
    the record. Then `Finished: <what>` with whether it went cleanly. `result.status` is duHast's flag; it is
    False once anything called `update_sep(False, ...)`."""
    for line in _lines_of(result)[_baseline[0]:]:
        if line not in _said:
            say(line)
    ok = True
    try:
        ok = bool(result.status)
    except Exception:
        pass
    _sink[0]("")
    path = log_path()
    if path:
        _sink[0]("Log file: {}".format(path))
    if ok:
        _sink[0]("Finished: {} - completed with no problems.".format(what))
    else:
        _sink[0]("Finished: {} - completed WITH PROBLEMS. Read the lines marked {} or {} above.".format(
            what, FAILED, HELD))
    return ok
