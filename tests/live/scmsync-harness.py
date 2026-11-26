#!/usr/bin/env python3
"""Live test harness for the Gitea <-> OBS scmsync bridge.

Spins up a throwaway Gitea repo + a scmsync home test project/package,
pushes git changes, and asserts the OBS side follows (or, for the
`_link` case, refuses to). Tears everything down afterwards — never
leaves a test project behind.

Local instances only. The harness talks to the loopback OBS instance
provisioned by `tests/live/provision.sh` and the disposable Gitea
container provisioned by `tests/gitea/provision.sh` — never to a
production instance. Every endpoint comes from the environment (see
below); the defaults are the loopback instances those scripts start,
and the harness refuses a non-loopback OBS or Gitea URL outright. The
OBS backend fetches the repo over the shared compose network, so the
scmsync clone URL is the Gitea container's name on that network
(`SCMSYNC_GITEA_URL`), not the loopback URL the host uses.

Environment (set by `.github/workflows/scmsync-live.yml`, or by hand
for a local run after provisioning both instances):
- GEECTL_BIN: geectl binary to drive (default: `geectl` on PATH).
- GEECTL_OBS_URL: OBS API base URL (default http://127.0.0.1:3000).
- GEECTL_OBS_USERNAME / GEECTL_OBS_PASSWORD: OBS credentials for
  that URL — the local instance's bootstrap admin (Admin / opensuse).
  Read by geectl itself; the harness only passes the environment
  through. Never oscrc, never a file in this repo.
- GEECTL_GITEA_URL: Gitea base URL (default http://127.0.0.1:3001).
- GEECTL_GITEA_TOKEN_FILE: file holding the Gitea API token
  (written by tests/gitea/provision.sh, 0600). Required: the
  harness fails fast when it is unset or missing.
- GITEA_USER: the Gitea user the token belongs to (default `geectl`).
  The OBS login (from `remote whois`) names the home project; the
  Gitea user names the repo — they differ on a local instance
  (OBS Admin vs Gitea geectl), so the two are kept separate.
- SCMSYNC_GITEA_URL: Gitea base URL the OBS backend uses to fetch
  the repo (default: GEECTL_GITEA_URL). On the shared compose
  network this is the Gitea container name, e.g.
  http://geectl-gitea-live:3000.

Setup shape (from the #23 sequenced plan):
- Gitea repo: public, default branch main, created with
  `gitea repo-create`; the first commit is pushed over HTTPS.
- OBS project: home:<login>:scmsync-harness-<tag>, created with
  `remote create-project`. No repositories: sync observation needs no
  build, so the step-(a) repos question does not apply here. The
  description names the Gitea repo and run tag, so a stray project
  is identifiable.
- OBS package: created with `remote create-package --scmsync
  <clone-url>#main` — the direct scmsync wiring (the same thing the
  branch composite's `cmd=fork&scmsync=` does server-side).

Sync observation reads `remote cat <project> <package>
_scmsync.obsinfo` and compares its `commit:` line with the pushed
git HEAD — the same primitive working_copy::commit polls.

Case matrix (each asserts and reports its timing):
- push:    a commit syncs (bridge normally seconds to ~2 min).
- link:    a HEAD containing `_link` does NOT sync within 5 minutes;
            removing the `_link` restores the sync.
- trigger: `service trigger` on a `_link`-free package forces a sync
            (the one live probe still owed from the scmsync work).
            The bridge may also have fired — this case proves the
            trigger path end to end (queued + sync lands), not
            trigger-vs-bridge exclusivity; all three timestamps are
            recorded so the lag is visible.
- delete:  a deleted file disappears from the OBS package.
- rename:  a renamed file appears under its new name on OBS.

Usage (after provisioning both local instances):
  python3 tests/live/scmsync-harness.py [--cases push,link,...] [--keep]
  python3 tests/live/scmsync-harness.py --cleanup <state.json>
  python3 tests/live/scmsync-harness.py --tag <tag>   # reuse a tag

Exit 0 only when every selected case passes and teardown is verified
(or --keep, when teardown is skipped by request and the kept names
are reported). A case that cannot be expressed with the leaves geectl
has fails with the exact missing piece named — never a guessed
workaround.
"""
import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from urllib.parse import urlparse

# Local instances only: every endpoint from the environment, defaults
# are the loopback instances the provisioning scripts start. No
# production endpoint appears anywhere in this file.
GEECTL_BIN = os.environ.get("GEECTL_BIN", "geectl")
OBS_URL = os.environ.get("GEECTL_OBS_URL", "http://127.0.0.1:3000")
GITEA_URL = os.environ.get("GEECTL_GITEA_URL", "http://127.0.0.1:3001")
GITEA_TOKEN_FILE = os.environ.get("GEECTL_GITEA_TOKEN_FILE", "")
GITEA_USER = os.environ.get("GITEA_USER", "geectl")
# URL the OBS backend uses to fetch the repo: reachable from inside
# the OBS containers (the Gitea container's name on the shared compose
# network), not the loopback URL the host uses.
SCMSYNC_GITEA_URL = os.environ.get("SCMSYNC_GITEA_URL") or GITEA_URL
GIT_USER_NAME = os.environ.get("GIT_USER_NAME", "geectl scmsync harness")
GIT_USER_EMAIL = os.environ.get("GIT_USER_EMAIL", "geectl@test.invalid")

SYNC_TIMEOUT = 180        # seconds to wait for a push to land on OBS
LINK_NO_SYNC_WINDOW = 300  # seconds a _link HEAD must stay unsynced
POLL_INTERVAL = 10
CALL_TIMEOUT = 90          # per geectl/git call

ALL_CASES = ["push", "link", "trigger", "delete", "rename"]

# Run tags and rundirs: the tag names the repo, the project and
# the rundir, and teardown `rm -rf`s paths derived from the
# rundir — so a tag may only be the safe charset, and a rundir
# read back from a state file must sit under the harness prefix.
TAG_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]*")
RUNDIR_PREFIX = "/tmp/scmsync-harness-"
# The Gitea container on the shared compose network (see
# tests/gitea/provision.sh): the only non-loopback host the
# backend-facing URL may name.
SCMSYNC_CONTAINER_NAME = "geectl-gitea-live"


def log(msg):
    print(f"[{datetime.now(timezone.utc).strftime('%H:%M:%S')}Z] {msg}",
          flush=True)


def require_local_url(name, url):
    """Refuse anything but a loopback URL for a host-facing endpoint.

    The harness is local-instances-only: an OBS or Gitea URL that is
    not loopback is a wiring mistake, and failing fast here keeps a
    misconfigured run from ever touching a real instance.
    """
    host = urlparse(url).hostname or ""
    if host not in ("127.0.0.1", "::1", "localhost"):
        raise RuntimeError(
            f"{name} must be a loopback URL for the local instances; "
            f"got {url!r}")


def require_scmsync_url(url):
    """The backend-facing Gitea URL: loopback, or the Gitea container.

    On the shared compose network the backend reaches Gitea by the
    container name — the single label in SCMSYNC_CONTAINER_NAME. Any
    other dotless host would be an unrelated machine, which this
    harness never targets.
    """
    parsed = urlparse(url)
    host = parsed.hostname or ""
    if parsed.scheme not in ("http", "https"):
        raise RuntimeError(
            f"SCMSYNC_GITEA_URL must be http(s); got {url!r}")
    if host in ("127.0.0.1", "::1", "localhost") \
            or host == SCMSYNC_CONTAINER_NAME:
        return
    raise RuntimeError(
        f"SCMSYNC_GITEA_URL must be loopback or "
        f"{SCMSYNC_CONTAINER_NAME}; got {url!r}")


def run(cmd, cwd=None, env=None, timeout=CALL_TIMEOUT):
    """Run a command; return (rc, stdout, stderr). Never raises."""
    try:
        proc = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True,
                              text=True, timeout=timeout)
        return proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        return -1, "", f"timed out after {timeout}s: {' '.join(cmd)}"
    except OSError as e:
        return -1, "", f"failed to start {' '.join(cmd)}: {e}"


def geectl_env():
    env = dict(os.environ)
    env["GEECTL_GITEA_TOKEN_FILE"] = GITEA_TOKEN_FILE
    env["GEECTL_FORMAT"] = "json"
    return env


def geectl(*args, timeout=CALL_TIMEOUT):
    return run([GEECTL_BIN, *args], env=geectl_env(), timeout=timeout)


def git(*args, cwd=None, env=None):
    return run(["git", *args], cwd=cwd, env=env or dict(os.environ))


def parse_json(text):
    try:
        return json.loads(text)
    except (ValueError, TypeError):
        return None


def obs_sync_commit(project, package):
    """The commit OBS last synced for the package, or None if unknown.

    Reads _scmsync.obsinfo; the file does not exist until the first
    sync lands, so any failure here means 'not synced yet', not an
    error. The cat output is parsed defensively (raw text or a JSON
    wrapper) and only the `commit:` line is used.
    """
    rc, out, _err = geectl("remote", "cat", project, package,
                           "_scmsync.obsinfo")
    if rc != 0:
        return None
    data = parse_json(out)
    if isinstance(data, dict):
        text = (data.get("content") or data.get("file") or
                data.get("text") or "")
        if not text:
            # Unknown wrapper shape: fall back to the raw dump.
            text = json.dumps(data)
    else:
        text = out
    m = re.search(r"^commit:\s*(\S+)\s*$", text, re.MULTILINE)
    return m.group(1) if m else None


def obs_file_names(project, package):
    """The file names OBS lists for the package, or None on failure."""
    rc, out, _err = geectl("remote", "list-files", project, package)
    if rc != 0:
        return None
    data = parse_json(out)
    if isinstance(data, dict):
        files = data.get("files") or []
    elif isinstance(data, list):
        files = data
    else:
        return None
    names = []
    for f in files:
        if isinstance(f, dict) and "name" in f:
            names.append(f["name"])
        elif isinstance(f, str):
            names.append(f)
    return names


def wait_for_commit(project, package, head, timeout):
    """Poll until OBS shows `head` as the synced commit.

    Returns (observed, lag_seconds) — observed False on timeout.
    """
    deadline = time.monotonic() + timeout
    start = time.monotonic()
    while True:
        commit = obs_sync_commit(project, package)
        if commit == head:
            return True, time.monotonic() - start
        if time.monotonic() >= deadline:
            return False, time.monotonic() - start
        time.sleep(POLL_INTERVAL)


def wait_for_no_sync(project, package, head, window):
    """Assert OBS does NOT sync `head` for `window` seconds.

    Returns (held, last_seen_commit) — held True when the synced
    commit stayed different from `head` for the whole window.
    """
    deadline = time.monotonic() + window
    last = None
    while True:
        last = obs_sync_commit(project, package)
        if last == head:
            return False, last
        if time.monotonic() >= deadline:
            return True, last
        time.sleep(POLL_INTERVAL)

class Harness:
    def __init__(self, tag, cases, keep):
        if not TAG_RE.fullmatch(tag):
            raise RuntimeError(
                f"run tag must be {TAG_RE.pattern!r}; got {tag!r}")
        self.tag = tag
        self.cases = cases
        self.keep = keep
        self.obs_owner = None
        self.gitea_user = GITEA_USER
        self.repo = f"scmsync-harness-{tag}"
        self.project = None
        self.package = self.repo
        self.branch = "main"
        self.clone_url = None
        self.rundir = f"{RUNDIR_PREFIX}{tag}"
        self.clone_dir = os.path.join(self.rundir, "repo")
        self.askpass = os.path.join(self.rundir, "askpass.sh")
        self.state_path = os.path.join(self.rundir, "state.json")
        self.results = {}
        self.leftovers = []

    # -- plumbing ---------------------------------------------------

    def save_state(self):
        state = {
            "tag": self.tag,
            "obs_owner": self.obs_owner,
            "gitea_user": self.gitea_user,
            "repo": self.repo,
            "branch": self.branch,
            "project": self.project,
            "package": self.package,
            "clone_url": self.clone_url,
            "rundir": self.rundir,
            "results": self.results,
            "leftovers": self.leftovers,
        }
        os.makedirs(self.rundir, exist_ok=True)
        with open(self.state_path, "w") as f:
            json.dump(state, f, indent=2)
        return self.state_path

    @classmethod
    def from_state(cls, path):
        with open(path) as f:
            state = json.load(f)
        h = cls(state["tag"], [], keep=False)
        h.obs_owner = state.get("obs_owner") or state.get("owner")
        h.gitea_user = state.get("gitea_user", GITEA_USER)
        h.repo = state["repo"]
        h.branch = state.get("branch", "main")
        h.project = state.get("project")
        h.package = state.get("package")
        h.clone_url = state.get("clone_url")
        h.rundir = state.get("rundir") or f"{RUNDIR_PREFIX}{h.tag}"
        # A rundir read back from a state file must be this run's own
        # rundir. A string-prefix test is not containment:
        # "/tmp/scmsync-harness-../../home/user" passes it, and
        # teardown would `rm -rf` paths outside the harness prefix.
        # Resolve symlinks and `..` first (realpath resolves the /tmp
        # symlink on macOS too, so realpath the prefix as well), then
        # check containment under the parent directory — the prefix
        # ends mid-component, so it cannot go into commonpath itself
        # — and require the suffix to be this run's tag.
        real = os.path.realpath(h.rundir)
        parent = os.path.realpath(os.path.dirname(RUNDIR_PREFIX))
        if os.path.commonpath([real, parent]) != parent:
            raise RuntimeError(
                f"state rundir escapes {parent!r}; got {h.rundir!r}")
        prefix = os.path.realpath(RUNDIR_PREFIX)
        if not real.startswith(prefix):
            raise RuntimeError(
                f"state rundir must be under {RUNDIR_PREFIX!r}; "
                f"got {h.rundir!r}")
        suffix = real[len(prefix):]
        if not TAG_RE.fullmatch(suffix) or suffix != h.tag:
            raise RuntimeError(
                f"state rundir must be {RUNDIR_PREFIX!r} plus the run "
                f"tag {h.tag!r}; got {h.rundir!r}")
        h.rundir = real
        h.clone_dir = os.path.join(h.rundir, "repo")
        h.askpass = os.path.join(h.rundir, "askpass.sh")
        h.state_path = os.path.join(h.rundir, "state.json")
        # Teardown deletes the project, the package and the repo named
        # here, so a stale or corrupt state file must not name
        # anything but this run's own harness things: the project
        # under the stated OBS owner, the tag-named repo and package,
        # and the Gitea user the token belongs to. Anything else is
        # refused before any live delete runs.
        expected_repo = f"scmsync-harness-{h.tag}"
        if h.repo != expected_repo:
            raise RuntimeError(
                f"state repo must be {expected_repo!r}; got {h.repo!r}")
        if h.package != expected_repo:
            raise RuntimeError(
                f"state package must be {expected_repo!r}; "
                f"got {h.package!r}")
        if h.gitea_user != GITEA_USER:
            raise RuntimeError(
                f"state gitea_user must be {GITEA_USER!r}; "
                f"got {h.gitea_user!r}")
        expected_project = f"home:{h.obs_owner}:scmsync-harness-{h.tag}"
        if h.project != expected_project:
            raise RuntimeError(
                f"state project must be {expected_project!r}; "
                f"got {h.project!r}")
        return h

    def git_env(self):
        env = dict(os.environ)
        env["GIT_ASKPASS"] = self.askpass
        env["GIT_TERMINAL_PROMPT"] = "0"
        return env

    def git_push(self):
        rc, out, err = git("push", "origin", self.branch,
                           cwd=self.clone_dir, env=self.git_env())
        if rc != 0:
            raise RuntimeError(f"git push failed rc={rc}: {out}{err}")
        return rc

    def git_commit(self, message):
        # Stage the whole tree (no pathspec): a `git rm` already stages
        # the deletion, and `git add -A <deleted-path>` would fail with
        # "pathspec did not match any files".
        rc, out, err = git("add", "-A", cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError(f"git add failed rc={rc}: {out}{err}")
        rc, out, err = git("commit", "-m", message, cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError(f"git commit failed rc={rc}: {out}{err}")
        rc, out, _err = git("rev-parse", "HEAD", cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError("git rev-parse HEAD failed")
        return out.strip()

    def write_file(self, name, content):
        path = os.path.join(self.clone_dir, name)
        with open(path, "w") as f:
            f.write(content)

    # -- setup ------------------------------------------------------

    def setup(self):
        log("setup: reading the authenticated OBS user")
        rc, out, err = geectl("remote", "whois")
        if rc != 0:
            raise RuntimeError(f"remote whois failed rc={rc}: {out}{err}")
        data = parse_json(out) or {}
        self.obs_owner = data.get("login")
        if not self.obs_owner:
            raise RuntimeError(f"remote whois gave no login: {out}")
        log(f"setup: OBS user {self.obs_owner}, "
            f"Gitea user {self.gitea_user}")

        self.project = f"home:{self.obs_owner}:scmsync-harness-{self.tag}"
        # The OBS backend fetches the repo over the shared compose
        # network, so the scmsync URL names the Gitea container, not
        # the loopback host. The host-side git operations below use
        # the loopback URL instead.
        scmsync_base = SCMSYNC_GITEA_URL.rstrip("/")
        self.clone_url = (f"{scmsync_base}/{self.gitea_user}/"
                          f"{self.repo}.git")
        os.makedirs(self.rundir, exist_ok=True)
        with open(self.askpass, "w") as f:
            f.write("#!/bin/sh\n")
            f.write(f"cat {shlex.quote(GITEA_TOKEN_FILE)}\n")
        os.chmod(self.askpass, 0o700)
        self.save_state()

        log(f"setup: creating Gitea repo "
            f"{self.gitea_user}/{self.repo}")
        rc, out, err = geectl(
            "gitea", "repo-create", self.repo,
            "--default-branch", self.branch,
            "--description",
            f"Throwaway scmsync harness rig {self.tag} (deleted after "
            f"the run).",
            "--live")
        if rc != 0:
            raise RuntimeError(
                f"gitea repo-create failed rc={rc}: {out}{err}")

        log("setup: cloning the empty repo and pushing the first commit")
        gitea_base = GITEA_URL.rstrip("/")
        push_url = (f"{urlparse(gitea_base).scheme}://"
                    f"{self.gitea_user}@"
                    f"{urlparse(gitea_base).netloc}/"
                    f"{self.gitea_user}/{self.repo}.git")
        rc, out, err = git("clone", push_url, self.clone_dir,
                           env=self.git_env())
        if rc != 0:
            raise RuntimeError(f"git clone failed rc={rc}: {out}{err}")
        git("config", "user.name", GIT_USER_NAME, cwd=self.clone_dir)
        git("config", "user.email", GIT_USER_EMAIL, cwd=self.clone_dir)
        self.write_file("README.md",
                        "# scmsync harness rig\n\nThrowaway repo for "
                        "the live Gitea<->OBS scmsync harness. Deleted "
                        "after the run.\n")
        self.write_file("marker.txt", "harness marker 1\n")
        head = self.git_commit("harness: initial commit")
        self.git_push()
        rc, out, _err = geectl("gitea", "branch-head", self.gitea_user,
                               self.repo, self.branch)
        # Content command in text form, but JSON-wrapped under
        # GEECTL_FORMAT=json: parse both shapes.
        data = parse_json(out)
        server_head = (data.get("head") if isinstance(data, dict)
                       else out.strip())
        if rc != 0 or server_head != head:
            raise RuntimeError(
                f"server head mismatch after push: branch-head "
                f"rc={rc} out={out.strip()!r} want {head}")
        log(f"setup: initial commit {head[:12]} on the server")

        log(f"setup: creating OBS project {self.project}")
        rc, out, err = geectl(
            "remote", "create-project", self.project,
            "--title", f"scmsync harness rig {self.tag}",
            "--description",
            f"Throwaway scmsync harness rig {self.tag} (deleted after "
            f"the run). scmsync-harness repo={self.gitea_user}/"
            f"{self.repo} branch={self.branch} run={self.tag}",
            "--live")
        if rc != 0:
            raise RuntimeError(
                f"remote create-project failed rc={rc}: {out}{err}")

        log(f"setup: creating scmsync package {self.package}")
        rc, out, err = geectl(
            "remote", "create-package",
            "--title", f"scmsync harness rig {self.tag}",
            "--description",
            "Throwaway scmsync harness package (deleted after the run).",
            "--scmsync", f"{self.clone_url}#{self.branch}",
            self.project, self.package, "--live")
        if rc != 0:
            raise RuntimeError(
                f"remote create-package --scmsync failed rc={rc}: "
                f"{out}{err}")
        self.save_state()

        log("setup: waiting for the initial sync")
        ok, lag = wait_for_commit(self.project, self.package, head,
                                  SYNC_TIMEOUT)
        if not ok:
            raise RuntimeError(
                f"initial sync did not land within {SYNC_TIMEOUT}s "
                f"(head {head})")
        log(f"setup: initial sync landed in {lag:.0f}s")
        return head

    # -- cases ------------------------------------------------------

    def record(self, name, passed, **detail):
        detail["passed"] = passed
        self.results[name] = detail
        status = "PASS" if passed else "FAIL"
        log(f"[{status}] {name}: " + ", ".join(
            f"{k}={v}" for k, v in detail.items() if k != "passed"))
        self.save_state()

    def case_push(self):
        log("case push: appending to marker.txt and pushing")
        with open(os.path.join(self.clone_dir, "marker.txt"), "a") as f:
            f.write("harness marker 2\n")
        head = self.git_commit("harness: marker 2")
        t_push = time.time()
        self.git_push()
        ok, lag = wait_for_commit(self.project, self.package, head,
                                  SYNC_TIMEOUT)
        self.record("push", ok, head=head[:12], lag_s=round(lag, 1),
                    timeout_s=SYNC_TIMEOUT,
                    note="" if ok else "bridge did not sync in time")

    def case_link(self):
        log("case link: pushing a HEAD containing _link")
        self.write_file(
            "_link",
            '<link project="openSUSE:Factory" package="hello"/>\n')
        head = self.git_commit("harness: add _link")
        self.git_push()
        held, last = wait_for_no_sync(self.project, self.package, head,
                                      LINK_NO_SYNC_WINDOW)
        if not held:
            self.record("link", False, head=head[:12],
                        synced_commit=(last or "none")[:12],
                        note="HEAD with _link synced — the block did "
                        "not hold")
            return
        log("case link: no sync for 5 minutes as expected; removing "
            "_link to confirm the sync restores")
        rc, out, err = git("rm", "_link", cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError(f"git rm _link failed rc={rc}: {out}{err}")
        head2 = self.git_commit("harness: remove _link")
        self.git_push()
        ok, lag = wait_for_commit(self.project, self.package, head2,
                                  SYNC_TIMEOUT)
        self.record("link", ok, blocked_head=head[:12],
                    restored_head=head2[:12], restore_lag_s=round(lag, 1),
                    no_sync_window_s=LINK_NO_SYNC_WINDOW,
                    note="" if ok else
                    "sync did not restore after removing _link")

    def case_trigger(self):
        log("case trigger: pushing, then service trigger immediately")
        with open(os.path.join(self.clone_dir, "marker.txt"), "a") as f:
            f.write("harness marker 3\n")
        head = self.git_commit("harness: marker 3")
        t_push = time.time()
        self.git_push()
        rc, out, err = geectl("service", "trigger", self.project,
                              self.package, "--live")
        t_trigger = time.time()
        if rc != 0:
            self.record("trigger", False, head=head[:12],
                        trigger_rc=rc, trigger_output=(out + err)[:300],
                        note="service trigger refused — see output")
            return
        ok, lag = wait_for_commit(self.project, self.package, head,
                                  SYNC_TIMEOUT)
        self.record(
            "trigger", ok, head=head[:12],
            push_to_trigger_s=round(t_trigger - t_push, 1),
            # `lag` is measured by wait_for_commit from just
            # after the trigger call, so it already is the
            # trigger-to-sync time.
            trigger_to_sync_s=round(lag, 1) if ok else None,
            total_lag_s=round(lag, 1),
            note="bridge may also have fired; trigger queued rc=0 "
            "and the sync landed" if ok else
            "trigger queued but the sync did not land in time")

    def case_delete(self):
        log("case delete: removing marker.txt from git")
        rc, out, err = git("rm", "marker.txt", cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError(
                f"git rm marker.txt failed rc={rc}: {out}{err}")
        head = self.git_commit("harness: delete marker.txt")
        self.git_push()
        ok, lag = wait_for_commit(self.project, self.package, head,
                                  SYNC_TIMEOUT)
        names = obs_file_names(self.project, self.package) if ok else None
        gone = names is not None and "marker.txt" not in names
        self.record("delete", ok and gone, head=head[:12],
                    lag_s=round(lag, 1),
                    obs_files=names,
                    note="" if (ok and gone) else
                    "marker.txt still listed on OBS" if ok else
                    "sync did not land in time")

    def case_rename(self):
        log("case rename: README.md -> README-renamed.md")
        rc, out, err = git("mv", "README.md", "README-renamed.md",
                           cwd=self.clone_dir)
        if rc != 0:
            raise RuntimeError(f"git mv failed rc={rc}: {out}{err}")
        head = self.git_commit("harness: rename README")
        self.git_push()
        ok, lag = wait_for_commit(self.project, self.package, head,
                                  SYNC_TIMEOUT)
        names = obs_file_names(self.project, self.package) if ok else None
        renamed = (names is not None
                   and "README-renamed.md" in names
                   and "README.md" not in names)
        self.record("rename", ok and renamed, head=head[:12],
                    lag_s=round(lag, 1),
                    obs_files=names,
                    note="" if (ok and renamed) else
                    "rename not reflected in the OBS file list" if ok
                    else "sync did not land in time")

    # -- teardown ---------------------------------------------------

    def teardown(self):
        """Delete package, then project, then repo. Never give up early.

        Returns True when every created thing is verified gone.
        Leftovers are recorded in self.leftovers and reported.
        """
        # A 404/not_found on a delete means the thing never existed (or
        # is already gone): not a leftover. Verification below is the
        # authority on what is actually gone.
        def delete_or_gone(kind, label, args):
            rc, out, err = geectl(*args)
            if rc == 0:
                log(f"teardown: {label} deleted")
                return
            if "not_found" in (out + err) or "404" in (out + err):
                log(f"teardown: {label} already gone (404)")
                return
            self.leftovers.append(
                f"{kind} {label} (delete rc={rc}: {(out + err)[:200]})")
            log(f"teardown: {label} delete failed rc={rc}: {out}{err}")

        log("teardown: deleting the OBS package")
        delete_or_gone(
            "package", f"{self.project}/{self.package}",
            ("remote", "delete-package", self.project, self.package,
             "--live"))

        log("teardown: deleting the OBS project")
        delete_or_gone(
            "project", self.project,
            ("remote", "delete-project", self.project, "--live"))

        log("teardown: deleting the Gitea repo")
        delete_or_gone(
            "repo", f"{self.gitea_user}/{self.repo}",
            ("gitea", "repo-delete", self.gitea_user, self.repo, "--force",
             "--live"))

        # Verify: a deleted project answers 404; a deleted repo too.
        # Only a 404/not_found counts as gone — any other failure
        # (500, auth, network) leaves the state unverified, not gone.
        def verified_gone(rc, out, err):
            return rc != 0 and ("not_found" in (out + err)
                                or "404" in (out + err))

        log("teardown: verifying everything is gone")
        rc, out, err = geectl("remote", "project-meta", self.project)
        project_gone = verified_gone(rc, out, err)
        if rc == 0:
            self.leftovers.append(f"project {self.project} still exists")
        elif not project_gone:
            self.leftovers.append(
                f"project {self.project} delete unverified "
                f"(rc={rc}: {(out + err)[:200]})")
        rc, out, err = geectl("gitea", "repo-get", self.gitea_user,
                              self.repo)
        repo_gone = verified_gone(rc, out, err)
        if rc == 0:
            self.leftovers.append(f"repo {self.gitea_user}/{self.repo} "
                                  "still exists")
        elif not repo_gone:
            self.leftovers.append(
                f"repo {self.gitea_user}/{self.repo} delete unverified "
                f"(rc={rc}: {(out + err)[:200]})")
        log(f"teardown: project gone={project_gone}, "
            f"repo gone={repo_gone}")

        # Local scratch: the clone and the askpass script.
        rc, _out, _err = run(["rm", "-rf", self.clone_dir, self.askpass])
        if rc != 0:
            log(f"teardown: local scratch removal rc={rc} (ignored)")
        self.save_state()
        return not self.leftovers

    # -- driver -----------------------------------------------------

    def run(self):
        case_fns = {
            "push": self.case_push,
            "link": self.case_link,
            "trigger": self.case_trigger,
            "delete": self.case_delete,
            "rename": self.case_rename,
        }
        setup_ok = True
        try:
            self.setup()
        except Exception as e:
            setup_ok = False
            log(f"setup FAILED: {e}")
            self.results["setup"] = {"passed": False, "error": str(e)}
            self.save_state()

        if setup_ok:
            for name in self.cases:
                try:
                    case_fns[name]()
                except Exception as e:
                    self.record(name, False, error=str(e))
                    log(f"case {name} raised: {e}")

        teardown_ok = None
        if self.keep:
            log(f"--keep: leaving behind project {self.project}, "
                f"package {self.package}, repo {self.gitea_user}/"
                f"{self.repo}")
        else:
            try:
                teardown_ok = self.teardown()
            except Exception as e:
                log(f"teardown raised: {e}")
                teardown_ok = False

        all_passed = (setup_ok
                      and all(r.get("passed") for r in self.results.values()
                              if isinstance(r, dict))
                      and (self.keep or teardown_ok is True))
        summary = {"tag": self.tag, "setup_ok": setup_ok,
                   "results": self.results, "teardown_ok": teardown_ok,
                   "leftovers": self.leftovers, "all_passed": all_passed,
                   "state": self.state_path}
        print("RESULT: " + json.dumps(summary, indent=2))
        return 0 if all_passed else 1


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Live Gitea<->OBS scmsync harness "
        "(local instances only).")
    parser.add_argument("--cases", default=",".join(ALL_CASES),
                        help="comma-separated subset of "
                        + ",".join(ALL_CASES))
    parser.add_argument("--keep", action="store_true",
                        help="skip teardown, leave the rig for "
                        "inspection (names are printed)")
    parser.add_argument("--cleanup",
                        help="teardown only, from a state.json written "
                        "by a previous run")
    parser.add_argument("--tag",
                        help="run tag (default: UTC timestamp)")
    args = parser.parse_args(argv)

    # Local instances only: refuse a non-loopback OBS or Gitea URL
    # before anything runs, and require the token file the Gitea
    # provision step writes.
    require_local_url("GEECTL_OBS_URL", OBS_URL)
    require_local_url("GEECTL_GITEA_URL", GITEA_URL)
    require_scmsync_url(SCMSYNC_GITEA_URL)
    if not GITEA_TOKEN_FILE or not os.path.isfile(GITEA_TOKEN_FILE):
        print("GEECTL_GITEA_TOKEN_FILE must name the token file written "
              "by tests/gitea/provision.sh; got "
              f"{GITEA_TOKEN_FILE!r}", file=sys.stderr)
        return 2

    if args.cleanup:
        h = Harness.from_state(args.cleanup)
        log(f"cleanup: tearing down rig {h.tag} from {args.cleanup}")
        ok = h.teardown()
        summary = {"tag": h.tag, "teardown_ok": ok,
                   "leftovers": h.leftovers}
        print("RESULT: " + json.dumps(summary, indent=2))
        return 0 if ok else 1

    tag = args.tag or datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    cases = [c.strip() for c in args.cases.split(",") if c.strip()]
    unknown = [c for c in cases if c not in ALL_CASES]
    if unknown:
        print(f"unknown cases: {', '.join(unknown)} "
              f"(known: {', '.join(ALL_CASES)})", file=sys.stderr)
        return 2
    if not cases:
        print("no cases selected", file=sys.stderr)
        return 2
    h = Harness(tag, cases, args.keep)
    return h.run()


if __name__ == "__main__":
    sys.exit(main())
