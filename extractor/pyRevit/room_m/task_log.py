"""A log file for the buttons: every step of a run, written as it happens.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

A run that dies inside Revit leaves nothing on the screen, so each line is written to disk the moment it is
made and the last one says where the run was. The file is `RoomMate\\RoomMate.log` under the user's application
data (`APPDATA`, else the home folder), beside `server.json`, and rolls over to `RoomMate.log.1` at 5 MB.

**No `logging` module, no duHast logger.** Each line is one open, append and close of the file. Both of the
others were used and a run's log came up empty on a machine in Revit (duHast's `LoggerObject` reconfigures the
root logger; the standard handler's behaviour inside the embedded interpreter was not something to bet on), so
this does the one thing a log must do with nothing between the line and the disk. **Nothing here ever raises**:
a log that could fail a run would be worse than no log. When the log cannot be started, `last_error()` says why,
so the button can print the reason and not just a missing file. Until `start` is called every function does
nothing, which keeps the tests and any module that logs quiet when it is run on its own.
"""

import io
import os
import time
import traceback

NAME = "RoomMate"
MAX_BYTES = 5 * 1024 * 1024

_path = None
_error = None


def log_dir(environ=None):
    """The folder the log lives in: `RoomMate` under the application data, as `server.json` is."""
    environ = os.environ if environ is None else environ
    root = environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(root, NAME)


def _text(message):
    try:
        return message if isinstance(message, type(u"")) else message.decode("utf-8", "replace")
    except Exception:
        return repr(message)


def _append(path, line):
    with io.open(path, "a", encoding="utf-8") as handle:
        handle.write(line + u"\n")


def _roll(path):
    """Keep one older file: past MAX_BYTES the log becomes `.log.1`."""
    try:
        if os.path.exists(path) and os.path.getsize(path) > MAX_BYTES:
            older = path + ".1"
            if os.path.exists(older):
                os.remove(older)
            os.rename(path, older)
    except Exception:
        pass


def start(title, directory=None):
    """Begin (or carry on) the log and write `title` as its first line. Returns the file's path, or None when no
    log could be made (`last_error()` says why)."""
    global _path, _error
    try:
        directory = directory or log_dir()
        if not os.path.isdir(directory):
            os.makedirs(directory)
        path = os.path.join(directory, NAME + ".log")
        _roll(path)
        _append(path, u"INFO | {} | ---- {} ----".format(time.strftime("%Y-%m-%d %H:%M:%S"), _text(title)))
        _path, _error = path, None
        return _path
    except Exception:
        _path = None
        _error = traceback.format_exc()
        return None


def last_error():
    """Why the log could not be started, or None."""
    return _error


def stop():
    """Forget the log: later calls do nothing again. For the tests."""
    global _path, _error
    _path = None
    _error = None


def path():
    """The log file in use, or None."""
    return _path


def _write(level, message):
    if _path is None:
        return
    try:
        _append(_path, u"{} | {} | {}".format(level, time.strftime("%Y-%m-%d %H:%M:%S"), _text(message)))
    except Exception:
        pass


def info(message):
    _write("INFO", message)


def step(message):
    """A step of the work, about to be done: the last one in the file is where a run stopped."""
    _write("INFO", u"STEP | {}".format(_text(message)))


def warning(message):
    _write("WARNING", message)


def error(message):
    _write("ERROR", message)


def exception(message):
    """`message` and the traceback of the exception being handled."""
    try:
        _write("ERROR", u"{}\n{}".format(_text(message), traceback.format_exc()))
    except Exception:
        pass
