"""Unit tests for the scmsync harness's state-file validation.

The harness itself runs weekly against local instances; these tests
need no instance — they pin the cases where a stale or corrupt state
file must be refused before teardown deletes anything. Run with:
python3 -m unittest discover -s tests/live -p "test_*.py" -v
"""

import importlib.util
import json
import os
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_HARNESS = os.path.join(_HERE, "scmsync-harness.py")


def _load_harness():
    spec = importlib.util.spec_from_file_location(
        "scmsync_harness", _HARNESS)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


harness = _load_harness()


def _state(**overrides):
    tag = "test1"
    state = {
        "tag": tag,
        "obs_owner": "Admin",
        "gitea_user": harness.GITEA_USER,
        "repo": f"scmsync-harness-{tag}",
        "branch": "main",
        "project": f"home:Admin:scmsync-harness-{tag}",
        "package": f"scmsync-harness-{tag}",
        "clone_url": "http://127.0.0.1:3001/geectl/scmsync-harness-test1.git",
        "rundir": f"{harness.RUNDIR_PREFIX}{tag}",
        "results": {},
        "leftovers": [],
    }
    state.update(overrides)
    return state


class FromStateTest(unittest.TestCase):
    def _load(self, state):
        with tempfile.NamedTemporaryFile(
                "w", suffix=".json", delete=False) as f:
            json.dump(state, f)
            path = f.name
        try:
            return harness.Harness.from_state(path)
        finally:
            os.unlink(path)

    def test_valid_state_loads(self):
        h = self._load(_state())
        self.assertEqual(h.tag, "test1")
        self.assertEqual(h.repo, "scmsync-harness-test1")
        self.assertEqual(h.project, "home:Admin:scmsync-harness-test1")
        self.assertEqual(h.gitea_user, harness.GITEA_USER)
        self.assertEqual(
            os.path.realpath(h.rundir),
            os.path.realpath("/tmp/scmsync-harness-test1"))

    def test_rundir_dotdot_escape_rejected(self):
        # The exact case from the review: a string-prefix test passes
        # this, and teardown would rm -rf outside the harness prefix.
        with self.assertRaises(RuntimeError):
            self._load(_state(
                rundir="/tmp/scmsync-harness-../../home/user"))

    def test_rundir_dotdot_inside_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(
                rundir="/tmp/scmsync-harness-test1/../../etc"))

    def test_rundir_symlink_escape_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            target = os.path.join(d, "target")
            os.makedirs(target)
            link = os.path.join(d, "scmsync-harness-esc")
            os.symlink(target, link)
            with self.assertRaises(RuntimeError):
                self._load(_state(rundir=link))

    def test_rundir_wrong_suffix_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(rundir="/tmp/scmsync-harness-evil!"))

    def test_rundir_other_tag_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(rundir="/tmp/scmsync-harness-test2"))

    def test_project_mismatch_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(
                project="home:victim:scmsync-harness-test1"))

    def test_project_missing_rejected(self):
        state = _state()
        del state["project"]
        with self.assertRaises(RuntimeError):
            self._load(state)

    def test_repo_mismatch_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(repo="someone-elses-repo"))

    def test_package_mismatch_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(package="someone-elses-package"))

    def test_gitea_user_mismatch_rejected(self):
        with self.assertRaises(RuntimeError):
            self._load(_state(gitea_user="mallory"))


if __name__ == "__main__":
    unittest.main()
