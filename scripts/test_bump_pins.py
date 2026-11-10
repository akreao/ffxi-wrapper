"""Offline tests for bump-pins.py. Run with: python3 -m unittest discover scripts

They run the script against copies of .env.example, compose.yaml and
stack.rs, so the real files are never touched and GitHub isn't needed.
"""

import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FILES = [".env.example", "compose.yaml", "desktop/src-tauri/src/stack.rs"]
NEW_LSB = "0123456789abcdef0123456789abcdef01234567"
NEW_CLIENT = "89abcdef0123456789abcdef0123456789abcdef"


def env_value(text, key):
    return re.findall(rf"^{key}=(.*)$", text, re.M)[-1].strip()


def app_pins(stack_rs):
    m = re.search(r"const APP_LSB_PINS: \[&str; (\d+)\] = \[\n(.*?)^\];", stack_rs, re.S | re.M)
    return int(m.group(1)), re.findall(r'"([0-9a-f]{40})"', m.group(2))


class BumpPins(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="bump-pins-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        for rel in FILES + ["scripts/bump-pins.py"]:
            (self.tmp / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / rel, self.tmp / rel)

    def run_script(self, *args):
        return subprocess.run(
            [sys.executable, str(self.tmp / "scripts" / "bump-pins.py"), "--offline", *args],
            capture_output=True, text=True)

    def read(self, rel):
        return (self.tmp / rel).read_text(encoding="utf-8")

    def test_current_pins_are_in_step(self):
        """Re-pinning to today's commits changes nothing, so every copy agrees."""
        env = self.read(".env.example")
        r = self.run_script("--lsb", env_value(env, "LSB_COMMIT"),
                            "--client", env_value(env, "CLIENT_COMMIT"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("Nothing to change.", r.stdout,
                      "compose.yaml or APP_LSB_PINS is out of step with .env.example:\n" + r.stdout)

    def test_ahbot_pin_in_step(self):
        commit = env_value(self.read(".env.example"), "AHBOT_COMMIT")
        compose = self.read("compose.yaml")
        defaults = set(re.findall(r"AHBOT_COMMIT:-([0-9a-fA-F]+)", compose))
        self.assertEqual(defaults, {commit}, "compose.yaml's AHBOT_COMMIT defaults differ from .env.example")

    def test_bump_lsb(self):
        old = env_value(self.read(".env.example"), "LSB_COMMIT")
        count, pins = app_pins(self.read("desktop/src-tauri/src/stack.rs"))
        r = self.run_script("--lsb", NEW_LSB)
        self.assertEqual(r.returncode, 0, r.stderr)

        self.assertEqual(env_value(self.read(".env.example"), "LSB_COMMIT"), NEW_LSB)
        compose = self.read("compose.yaml")
        self.assertNotIn(old, compose)
        self.assertEqual(set(re.findall(r"LSB_COMMIT:-([0-9a-fA-F]+)", compose)), {NEW_LSB})
        new_count, new_pins = app_pins(self.read("desktop/src-tauri/src/stack.rs"))
        self.assertEqual(new_pins, pins + [p for p in (old, NEW_LSB) if p not in pins])
        self.assertEqual(new_count, len(new_pins))

    def test_bump_client(self):
        r = self.run_script("--client", NEW_CLIENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(env_value(self.read(".env.example"), "CLIENT_COMMIT"), NEW_CLIENT)

    def test_dry_run_writes_nothing(self):
        before = {rel: self.read(rel) for rel in FILES}
        r = self.run_script("--lsb", NEW_LSB, "--client", NEW_CLIENT, "--dry-run")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("Dry run", r.stdout)
        self.assertEqual({rel: self.read(rel) for rel in FILES}, before)

    def test_offline_needs_full_sha(self):
        r = self.run_script("--lsb", "e42afb4")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("full 40-character", r.stderr)


if __name__ == "__main__":
    unittest.main()
