"""Which server the extractor talks to (`room_m.server_config`). Stdlib unittest, CPython."""

import json
import os
import re
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import server_config as sc  # noqa: E402

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")


class Normalise(unittest.TestCase):
    def test_an_address_as_typed_becomes_a_base_url_with_the_default_port(self):
        self.assertEqual("http://192.168.0.97:5151", sc.normalise("192.168.0.97"))
        self.assertEqual("http://mac.local:5151", sc.normalise("  mac.local  "))

    def test_a_port_and_a_scheme_are_kept_and_a_trailing_path_is_dropped(self):
        self.assertEqual("http://192.168.0.97:6000", sc.normalise("192.168.0.97:6000"))
        self.assertEqual("http://192.168.0.97:6000", sc.normalise("http://192.168.0.97:6000/"))
        self.assertEqual("https://plans.internal:443", sc.normalise("https://plans.internal:443/rooms"))
        self.assertEqual("http://192.168.0.97:5151", sc.normalise("HTTP://192.168.0.97"))

    def test_ipv6_goes_in_brackets(self):
        self.assertEqual("http://[fe80::1]:5151", sc.normalise("[fe80::1]"))
        self.assertEqual("http://[fe80::1]:6000", sc.normalise("[fe80::1]:6000"))
        with self.assertRaises(ValueError):
            sc.normalise("fe80::1")

    def test_what_is_not_an_address_says_why(self):
        for bad in ["", "   ", "ftp://x", "a b", "host:abc", "host:0", "host:70000", "[::1", "http://"]:
            with self.assertRaises(ValueError, msg=repr(bad)):
                sc.normalise(bad)


class Resolve(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.path = os.path.join(self.dir, "RoomMate", "server.json")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.addCleanup(sc.set_project_reader, None)
        sc.set_project_reader(None)

    def write(self, text):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        with open(self.path, "w") as f:
            f.write(text)

    def test_with_nothing_kept_there_is_no_server_and_no_silent_default(self):
        self.assertEqual((None, "none"), sc.resolve({}, self.path))
        with self.assertRaises(sc.ServerNotSet) as raised:
            sc.base_url({}, self.path)
        self.assertIn("Server button", str(raised.exception))
        with self.assertRaises(sc.ServerNotSet):
            sc.url_for("/rooms", {}, self.path)

    def test_the_model_beats_the_file_and_the_environment_beats_both(self):
        self.write(json.dumps({"url": "192.168.0.97"}))
        sc.set_project_reader(lambda: "10.0.0.5")
        self.assertEqual(("http://10.0.0.5:5151", "project"), sc.resolve({}, self.path))
        env = {sc.ENV_VAR: "mac.local:6000"}
        self.assertEqual(("http://mac.local:6000", "env"), sc.resolve(env, self.path))

    def test_a_model_with_no_address_falls_back_to_the_file_and_then_to_nothing(self):
        sc.set_project_reader(lambda: None)
        self.assertEqual((None, "none"), sc.resolve({}, self.path))
        self.write(json.dumps({"url": "192.168.0.97"}))
        self.assertEqual(("http://192.168.0.97:5151", "file"), sc.resolve({}, self.path))

    def test_a_reader_that_fails_or_a_value_that_does_not_parse_falls_through(self):
        self.write(json.dumps({"url": "192.168.0.97"}))

        def broken():
            raise RuntimeError("no document")

        sc.set_project_reader(broken)
        self.assertEqual("file", sc.resolve({}, self.path)[1])
        sc.set_project_reader(lambda: "ftp://nope")
        self.assertEqual("file", sc.resolve({}, self.path)[1])
        self.assertEqual("file", sc.resolve({sc.ENV_VAR: "ftp://nope"}, self.path)[1])
        sc.set_project_reader(None)
        self.write("{not json")
        self.assertEqual("none", sc.resolve({}, self.path)[1])
        self.write(json.dumps({"url": 5}))
        self.assertEqual("none", sc.resolve({}, self.path)[1])
        self.write(json.dumps(["not", "a", "dict"]))
        self.assertEqual("none", sc.resolve({}, self.path)[1])

    def test_it_is_read_every_time_not_once(self):
        with self.assertRaises(sc.ServerNotSet):
            sc.base_url({}, self.path)
        sc.save("192.168.0.97", self.path)
        self.assertEqual("http://192.168.0.97:5151", sc.base_url({}, self.path), "a long-lived session must see the change")
        kept = ["10.0.0.5"]
        sc.set_project_reader(lambda: kept[0])
        self.assertEqual("http://10.0.0.5:5151", sc.base_url({}, self.path))
        kept[0] = "10.0.0.6"
        self.assertEqual("http://10.0.0.6:5151", sc.base_url({}, self.path), "the model in front of the person is followed")

    def test_save_keeps_the_normalised_address_local_words_keep_this_computer_and_reset_forgets(self):
        self.assertEqual("http://192.168.0.97:5151", sc.save("192.168.0.97", self.path))
        self.assertTrue(os.path.isfile(self.path))
        self.assertEqual(sc.LOCAL_URL, sc.save("Local", self.path), "this computer is chosen on purpose")
        self.assertEqual((sc.LOCAL_URL, "file"), sc.resolve({}, self.path))
        self.assertIsNone(sc.save("reset", self.path))
        self.assertFalse(os.path.isfile(self.path))
        self.assertIsNone(sc.save("forget", self.path), "forgetting twice is fine")
        with self.assertRaises(ValueError):
            sc.save("ftp://x", self.path)
        self.assertFalse(os.path.isfile(self.path), "a bad address keeps nothing")

    def test_the_config_file_lives_under_the_application_data_folder(self):
        self.assertEqual(os.path.join("C:\\Users\\x\\AppData\\Roaming", "RoomMate", "server.json"), sc.config_path({"APPDATA": "C:\\Users\\x\\AppData\\Roaming"}))

    def test_a_path_is_joined_to_the_server_in_force_and_an_absolute_url_is_left_alone(self):
        sc.save("192.168.0.97", self.path)
        self.assertEqual("http://192.168.0.97:5151/rooms", sc.url_for("/rooms", {}, self.path))
        self.assertEqual("http://192.168.0.97:5151/rooms", sc.url_for("rooms", {}, self.path))
        self.assertEqual("http://other:1/x", sc.url_for("http://other:1/x", {}, self.path))

    def test_the_run_says_which_server_and_why(self):
        self.assertIn("Server button", sc.describe({}, self.path))
        sc.save("192.168.0.97", self.path)
        self.assertIn("server.json", sc.describe({}, self.path))
        sc.set_project_reader(lambda: "10.0.0.5")
        self.assertIn("this model", sc.describe({}, self.path))
        self.assertIn(sc.ENV_VAR, sc.describe({sc.ENV_VAR: "x"}, self.path))


class Hints(unittest.TestCase):
    def test_loopback_is_recognised_in_every_spelling(self):
        for url in ["http://127.0.0.1:5151", "http://localhost:5151", "http://[::1]:5151", "http://LOCALHOST"]:
            self.assertTrue(sc.is_local(url), url)
        for url in ["http://192.168.0.97:5151", "http://mac.local:5151"]:
            self.assertFalse(sc.is_local(url), url)

    def test_the_hint_names_the_likely_fix_for_each_case(self):
        self.assertIn("Server button", sc.unreachable_hint("http://127.0.0.1:5151"))
        remote = sc.unreachable_hint("http://192.168.0.97:5151")
        self.assertIn("--bind", remote)
        self.assertIn("firewall", remote)
        self.assertIn("192.168.0.97:5151", remote)


class Wiring(unittest.TestCase):
    """The modules cannot be imported here (they need .NET), so what makes the setting work is read
    off their source: every pusher holds a PATH, and only `post_common` joins it to a server."""

    def sources(self):
        for name in sorted(os.listdir(ROOM_M)):
            if name.startswith("post_") and name.endswith(".py"):
                with open(os.path.join(ROOM_M, name)) as f:
                    yield name, f.read()

    def test_no_pusher_names_a_server_and_every_url_it_holds_is_a_path(self):
        for name, text in self.sources():
            self.assertNotIn("http://", text.replace("http://-", ""), name)
            for m in re.finditer(r'(?m)^\s*(?:SERVER_URL\w*|BASE_URL|url|url_stream)\s*[=]\s*"([^"]*)"', text):
                self.assertTrue(m.group(1).startswith("/"), "%s: %s" % (name, m.group(1)))

    def test_the_two_network_calls_resolve_the_address_when_they_are_made(self):
        with open(os.path.join(ROOM_M, "post_common.py")) as f:
            text = f.read()
        self.assertEqual(2, text.count("url = server_config.url_for(url)"), "fetch_projects and _post_content")

    def test_a_run_with_no_server_reports_it_instead_of_raising(self):
        with open(os.path.join(ROOM_M, "post_common.py")) as f:
            text = f.read()
        self.assertEqual(2, text.count("except server_config.ServerNotSet as e:"), "fetch_projects and _post_content")

    def test_the_modules_ironpython_loads_are_ascii(self):
        for name in ("server_config.py", "post_common.py", "room_mate.py", "project_settings.py"):
            with open(os.path.join(ROOM_M, name), "rb") as f:
                f.read().decode("ascii")


if __name__ == "__main__":
    unittest.main()
