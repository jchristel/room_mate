"""Which RoomMate server the extractor talks to.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so
that CPython runs it too (`extractor/tests/test_server_config.py`): it imports nothing from
Revit or .NET.

**Resolved when a request is made, never when a module is imported.** A pyRevit session stays open
for hours and every module is imported once, so a URL frozen into a module constant would keep
pointing at the old server after the person changed it. Every other module holds only the PATH of
what it posts (`/rooms`, `/projects/{}/sheets`) and `room_m.post_common` joins it to `base_url()`
at the moment of the POST or GET. That is also why this file is the only one that may name an
address: `scripts/check_extension.py` fails on a literal loopback address anywhere else.

**The order, first match wins:**

1. the environment variable `ROOMMATE_SERVER` (a machine-wide or per-launch override),
2. the model's own settings, in extensible storage in the Revit project file
   (`room_m.project_settings`), read through the reader registered with `set_project_reader`,
3. `server.json` in the RoomMate folder of the user's application data (`{"url": "..."}`),
4. nothing: there is **no silent default**. A first run on a computer that has never been told
   where the server is gets `ServerNotSet`, whose message points at the Server button, rather than
   a request to a loopback address that fails with a network error nobody connects to the setting.
   Typing `local` in the Server button chooses the server on this computer, on purpose.

The Server button writes 2 and 3 together. A source that does not parse is skipped, not fatal.

A person types an address as they know it (`192.168.0.97`, `mac.local:5151`,
`http://192.168.0.97:5151/`); `normalise` makes it a base URL and **adds the default port 5151
when none is given**, because that is the port the server listens on unless it was told otherwise.
"""

import json
import os

DEFAULT_PORT = 5151
LOCAL_URL = "http://127.0.0.1:{}".format(DEFAULT_PORT)
ENV_VAR = "ROOMMATE_SERVER"

# Words that choose the server on this computer, and the word that forgets every kept address.
LOCAL_WORDS = ("default", "local", "this computer")
RESET_WORDS = ("reset", "forget")

NOT_SET_MESSAGE = (
    "No RoomMate server has been set for this model or this computer. Press the Server button "
    "and type the address of the server (for example 192.168.0.97, or mac.local:5151)."
)


class ServerNotSet(ValueError):
    """No source names a server. The message says what to do about it."""

    def __init__(self):
        ValueError.__init__(self, NOT_SET_MESSAGE)


_project_reader = None


def set_project_reader(reader):
    """Register a function returning the address kept in the open model (or None). It is called at
    the moment a request is made, so a session that stays open follows the model in front of it."""
    global _project_reader
    _project_reader = reader


def config_path(environ=None):
    """Where `server.json` lives: `RoomMate\\server.json` under the user's application data
    (`APPDATA` on Windows), else under the home folder."""
    environ = os.environ if environ is None else environ
    root = environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(root, "RoomMate", "server.json")


def normalise(text):
    """A typed address as a base URL with no trailing slash, or raises `ValueError` saying what is
    wrong. `192.168.0.97` becomes `http://192.168.0.97:5151`; a scheme and a port are kept."""
    value = (text or "").strip()
    if not value:
        raise ValueError("no address was given")
    scheme = "http"
    lowered = value.lower()
    if lowered.startswith("http://"):
        value = value[7:]
    elif lowered.startswith("https://"):
        scheme = "https"
        value = value[8:]
    elif "://" in value:
        raise ValueError("only http:// and https:// addresses are supported")
    value = value.split("/", 1)[0]
    if not value or any(c in value for c in " \t?#@"):
        raise ValueError("'{}' is not an address like 192.168.0.97 or mac.local:5151".format(text.strip()))
    host, port = _split_host_port(value)
    if not host:
        raise ValueError("'{}' has no host name or address".format(text.strip()))
    if port is None:
        port = DEFAULT_PORT
    elif not (0 < port < 65536):
        raise ValueError("the port {} is not between 1 and 65535".format(port))
    shown = "[{}]".format(host) if ":" in host else host
    return "{}://{}:{}".format(scheme, shown, port)


def _split_host_port(value):
    """`(host, port or None)` for `host`, `host:port`, `[v6]` and `[v6]:port`."""
    if value.startswith("["):
        end = value.find("]")
        if end < 0:
            raise ValueError("'{}' has an unclosed [".format(value))
        host = value[1:end]
        rest = value[end + 1:]
        if not rest:
            return host, None
        if not rest.startswith(":"):
            raise ValueError("'{}' is not an address".format(value))
        return host, _port(rest[1:])
    if value.count(":") == 1:
        host, port = value.split(":")
        return host, _port(port)
    if value.count(":") > 1:
        raise ValueError("an IPv6 address goes in brackets, like [fe80::1]:5151")
    return value, None


def _port(text):
    try:
        return int(text)
    except ValueError:
        raise ValueError("'{}' is not a port number".format(text))


def _read_file(path):
    """The address in `server.json`, or None when the file is absent, unreadable or empty. A broken
    file is not fatal: the extractor falls through to the next source and `describe` says so."""
    try:
        with open(path, "r") as f:
            data = json.load(f)
    except (IOError, OSError, ValueError):
        return None
    url = data.get("url") if isinstance(data, dict) else None
    return url if isinstance(url, type(u"")) or isinstance(url, str) else None


def _from_project():
    if _project_reader is None:
        return None
    try:
        return _project_reader()
    except Exception:
        return None


def resolve(environ=None, path=None):
    """`(url, source)`: the base URL in force and where it came from (`env`, `project`, `file`), or
    `(None, "none")` when nothing names a server. A value that does not parse is skipped, and the
    next source is used."""
    environ = os.environ if environ is None else environ
    from_env = (environ.get(ENV_VAR) or "").strip()
    if from_env:
        try:
            return normalise(from_env), "env"
        except ValueError:
            pass
    kept = _from_project()
    if kept:
        try:
            return normalise(kept), "project"
        except ValueError:
            pass
    stored = _read_file(path or config_path(environ))
    if stored:
        try:
            return normalise(stored), "file"
        except ValueError:
            pass
    return None, "none"


def base_url(environ=None, path=None):
    """The base URL in force, or raises `ServerNotSet`."""
    url = resolve(environ, path)[0]
    if url is None:
        raise ServerNotSet()
    return url


def url_for(url_or_path, environ=None, path=None):
    """A full URL for what a module posts to. An absolute URL is returned as it is; a path
    (`/rooms`) is joined to the base URL in force right now."""
    text = url_or_path or ""
    if text.startswith("http://") or text.startswith("https://"):
        return text
    if not text.startswith("/"):
        text = "/" + text
    return base_url(environ, path) + text


def describe(environ=None, path=None):
    """One line for the run output: which server, and why that one."""
    url, source = resolve(environ, path)
    if source == "env":
        return "{} (from the {} environment variable)".format(url, ENV_VAR)
    if source == "project":
        return "{} (kept in this model)".format(url)
    if source == "file":
        return "{} (from {})".format(url, path or config_path(environ))
    return "not set. " + NOT_SET_MESSAGE


def save(text, path=None, environ=None):
    """Keep `text` as the server address in `server.json` and return the base URL that was kept.
    The local words (`default`, `local`) keep the server on this computer; the reset words
    (`reset`, `forget`) remove the stored address and return None. Raises `ValueError` for an
    address that does not parse. The model's own copy is written by the caller
    (`room_m.project_settings.write_server`), because it needs a Revit document."""
    target = path or config_path(environ)
    word = (text or "").strip().lower()
    if word in RESET_WORDS:
        try:
            os.remove(target)
        except OSError:
            pass
        return None
    url = LOCAL_URL if word in LOCAL_WORDS else normalise(text)
    folder = os.path.dirname(target)
    if folder and not os.path.isdir(folder):
        os.makedirs(folder)
    with open(target, "w") as f:
        json.dump({"url": url}, f)
    return url


def is_local(url):
    """Whether `url` names this computer, which is the case the person who set up a network
    server most likely forgot to change."""
    host = url.split("://", 1)[-1].split("/", 1)[0]
    if host.startswith("["):
        host = host[1:host.find("]")] if "]" in host else host[1:]
    elif host.count(":") == 1:
        host = host.split(":")[0]
    return host.lower() in ("127.0.0.1", "localhost", "::1")


def unreachable_hint(url):
    """What to check when `url` could not be reached, for the end of an error message."""
    if is_local(url):
        return (
            "RoomMate is looking for the server on THIS computer. If it runs on another one, "
            "set its address with the Server button (or the {} environment variable).".format(ENV_VAR)
        )
    return (
        "Check that the server was started with --bind 0.0.0.0 (or this address), that the "
        "firewall on the server's computer allows incoming connections, and that {} is the "
        "address it printed at startup.".format(url)
    )
