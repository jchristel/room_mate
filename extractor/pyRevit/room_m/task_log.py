"""A log file for the Tasks button: every step of a run, written as it happens.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

A run that dies inside Revit leaves nothing on the screen, so each line is written to disk the moment
it is made and the last one says where the run was. The file is `RoomMate\\RoomMate.log` under the
user's application data (`APPDATA`, else the home folder), beside `server.json`, and rotates at 5 MB.

**It uses duHast's `LoggerObject` when the copy of duHast in use has one** (file handler, line format
and rotation are duHast's own) and the standard `logging` module otherwise, so a log never depends on
which duHast is loaded. **Nothing here ever raises**: a log that could fail a run would be worse than
no log. Until `start` is called every function does nothing, which keeps the tests and any module
that logs quiet when it is run on its own.
"""

import logging
import os
import traceback

NAME = "RoomMate"
FILE_LEVEL = 10  # DEBUG
CONSOLE_LEVEL = 100  # above CRITICAL: the output window has its own lines

_logger = None
_path = None


def log_dir(environ=None):
    """The folder the log lives in: `RoomMate` under the application data, as `server.json` is."""
    environ = os.environ if environ is None else environ
    root = environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(root, NAME)


def _duhast_logger(directory):
    """duHast's logger writing to `directory`, or None when this duHast has none."""
    try:
        from duHast.Utilities.Objects.logger_object import LoggerObject

        made = LoggerObject(
            log_name=NAME,
            output_path=os.path.dirname(directory),
            log_level=(FILE_LEVEL, CONSOLE_LEVEL),
            file_format=".log",
        )
        return made.get_logger_obj(), made.log_file_path
    except Exception:
        return None


def _plain_logger(directory):
    """The same file through the standard library: a rotating handler and duHast's line format."""
    from logging.handlers import RotatingFileHandler

    if not os.path.isdir(directory):
        os.makedirs(directory)
    path = os.path.join(directory, NAME + ".log")
    made = logging.getLogger(NAME + ".tasks")
    made.setLevel(FILE_LEVEL)
    made.propagate = False
    for old in list(made.handlers):
        made.removeHandler(old)
        old.close()
    handler = RotatingFileHandler(path, maxBytes=5 * 1024 * 1024, backupCount=1)
    handler.setFormatter(
        logging.Formatter("%(levelname)s | %(asctime)s.%(msecs)03d | %(message)s", datefmt="%Y-%m-%d %H:%M:%S")
    )
    made.addHandler(handler)
    return made, path


def start(title, directory=None):
    """Begin (or carry on) the log and write `title` as its first line. Returns the file's path, or
    None when no log could be made."""
    global _logger, _path
    try:
        directory = directory or log_dir()
        made = _duhast_logger(directory) if _logger is None else None
        if made is None and _logger is None:
            made = _plain_logger(directory)
        if made is not None:
            _logger, _path = made
        info("---- {} ----".format(title))
        return _path
    except Exception:
        _logger = None
        _path = None
        return None


def stop():
    """Forget the log: later calls do nothing again. For the tests."""
    global _logger, _path
    if _logger is not None:
        for handler in list(_logger.handlers):
            try:
                handler.close()
                _logger.removeHandler(handler)
            except Exception:
                pass
    _logger = None
    _path = None


def path():
    """The log file in use, or None."""
    return _path


def _write(level, message):
    if _logger is None:
        return
    try:
        _logger.log(level, message)
        for handler in _logger.handlers:
            handler.flush()
    except Exception:
        pass


def info(message):
    _write(logging.INFO, message)


def step(message):
    """A step of the work, about to be done: the last one in the file is where a run stopped."""
    _write(logging.INFO, "STEP | {}".format(message))


def warning(message):
    _write(logging.WARNING, message)


def error(message):
    _write(logging.ERROR, message)


def exception(message):
    """`message` and the traceback of the exception being handled."""
    try:
        _write(logging.ERROR, "{}\n{}".format(message, traceback.format_exc()))
    except Exception:
        pass
