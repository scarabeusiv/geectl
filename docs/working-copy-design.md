# geectl working-copy layer — design

Issue: https://github.com/scarabeusiv/geectl/issues/17
Status: implemented.
Depends on: https://github.com/scarabeusiv/geectl/issues/20 (Gitea ops surface), https://github.com/scarabeusiv/geectl/issues/112 (unified pull-request surface), https://github.com/scarabeusiv/geectl/issues/19 (services layer), https://github.com/scarabeusiv/geectl/issues/153 (changelog layer).

## The layer, up front

The 10 local-state operations (`checkout`, `update`, `status`, `diff`,
`add`/`rm`/`addremove`, `commit`, `changelog`, `mkpac`, `pull`, `resolve`) are one design
decision, not nine API calls: geectl keeps a working copy, and the working
copy is a git repository.

**The checkout is a git repository.** A git-backed package is a real clone
of its Gitea repository. An OBS-native package is a synthesized repository
whose root commit is the base revision. `update` is a rebase. `commit`
derives the full file list from the base plus the staged diff and sends it
through the commit protocol. The agent's call never changes with the
backend kind: one operation per verb, and the backend kind recorded at
checkout routes writes only.

## The 9 operations, mapped

All reads use the typed API; the only new server calls the layer needs are
the two commit-protocol calls (see "Commit protocol").

| Operation | API calls | Local state |
|---|---|---|
| `checkout` | OBS-native: `list_packages` (project form) or `list_files` + `cat` per file + `package_meta`. Downloads are pinned to the listing's revision and md5-verified (see `update`). Git-backed: `package_meta` (to read the scmsync URL) + clone (the `#branch` fragment names the branch to check out and is stripped before the clone, since git does not understand it). The scmsync URL must be `http(s)`: any other scheme is refused, the clone runs with `--` before the URL and `GIT_ALLOW_PROTOCOL=http:https`. | Writes files to `<dir>/<project>/<package>/`. A `:` in a project or package name becomes `_` in the directory name (a Windows path cannot hold `:`); the recorded project and package keep the real names. A `:` in a file name is not sanitized — the file keeps its server name — so on Windows such a file cannot be created and the checkout fails there. Git-backed package: real clone of the Gitea repository. OBS-native package: synthesized repository whose root commit holds the base revision content; a brand-new package synthesizes a root commit over the empty tree. Records `apiurl`, `project`, `package`, `backend` (`obs` \| `git`), `base-rev`, `base-srcmd5` in the checkout's git config (`geectl.*`). No OBS credentials; a git-backed checkout clones with the configured Gitea token when one is set. The token is scoped to the configured Gitea instance (`http.<gitea-url>.extraHeader`): git sends it only to that instance, never to whatever host a scmsync URL names. The root commit is built with `git add -f -A`, so a downloaded file matching the user's global excludes is still committed. |
| `update` | OBS-native: `list_files` at latest rev (or `rev=`), `cat` for the files the listing adds or changes — a download is pinned to the listing's revision (`rev=<n>`) and its md5 is verified against the downloaded bytes (hashed in chunks from disk); a file whose listing md5 matches the base blob is reused from the object store instead of re-downloaded. Git-backed: fetch from the Gitea remote. | Rebase. OBS-native: the new base becomes the new root commit and local commits rebase onto it. Git-backed: local commits rebase onto the synced HEAD. Git merges text changes; real conflicts stay in the tree as merge conflicts. No local commit is lost. Dirty worktree changes are stashed before the rebase (a manual stash, popped with `--index` so staged intent survives) instead of `--autostash`; a pop that conflicts leaves the entry in the stash and the conflict in the tree, and the report names both (`conflicts`, `stash_remaining`). Downloads stream to disk; no size refusal. |
| `status` | None | Compares the working tree and the index against the base commit. Same single-letter vocabulary as the existing tool (`A`/`M`/`D`/`?`/`!`/`C`/`R`, blank for clean). The mapping from git states to letters is the layer's own, pinned by the layer's tests. There is no `S` status: files are never skipped for size. |
| `diff` | None for local diff; `project_diff` for remote-vs-remote | Local git diff of the staged index against the base commit plus the unstaged worktree against the index — the two halves together, so a staged-only change shows. Binary files appear as git's `Binary files … differ` line in the diff text, not a failure. Remote-vs-remote diff reuses `project_diff`. |
| `add` / `rm` / `addremove` | None (local intent only) | Stage into the index: `add` stages, `rm` removes from index and working tree. `addremove` is `status` + add of `?` + rm of `!`. `commit` sends the full file list derived from the base plus the staged diff. |
| `commit` | `commit_filelist` (validate, then final) + staged file PUTs (`rev=repository`) + `open_requests_for_package` (report after) | Requires base rev == upstream rev, else `WorkingCopyOutdated`. One revision per commit, exactly (see protocol). Services run before commit, hard gate: a failing service run blocks the commit. Dispatches on the recorded backend kind (see "Git-backed commit"). Refreshes the base at the new rev: OBS-native, the new server revision becomes the new root commit and the committed local commits' content is in it, their separate identities gone; git-backed, the pushed HEAD is the new base. |
| `changelog` | None pre-commit; validate after filing reads `request diff` | `changelog validate` + `changelog generate` (see "`changelog`"). Runs pre-commit in the hard gate, pre-file, and after filing. |
| `mkpac` | `create_package` (exists) + `checkout` | No new operation. |
| `pull` | Expanded directory read (`list_files` with expand) + expanded file downloads + `update` | Only for linked packages with a baserev and a clean WC: no uncommitted changes and no untracked files (local commits are rebased, not refused), refuse without baserev. The clean-tree check does not honour user excludes: `status` hides ignored untracked files, so the untracked half is read with plain `ls-files --others` (no `--exclude-standard`) — an ignored file blocks the pull like any other untracked file. Rebases link-target changes into the WC, like `update`. The expanded listing never carries `_link` (the server's expanded tree excludes it): pull fetches `_link` through the unexpanded route and rewrites its baserev to the target's latest srcmd5. Expanded downloads are pinned to the expanded listing's revision and md5-verified. A link package whose revision the server has not assigned yet is waited for (bounded retry of the revision read only); after the budget the pull errors instead of pulling unpinned. |
| `resolve` | None (local git only) | Stage the resolved files and continue a stopped rebase. A file that still holds conflict markers (`<<<<<<<` / `=======` / `>>>>>>>`) is refused, not staged. A successful continue adopts the new base — OBS-native from the new root commit's message, git-backed from the rebase target — on disk and in the checkout. Unstaged edits are stashed around the continue and restored after; a failed restore strands them in the stash (named in the error) with the base already adopted. An autostash entry left by an earlier stopped update is restored at the end of `resolve` (or immediately when there is nothing to resolve), so it is not left stranded. |

## The base record

The authoritative base for content is the base commit: the root commit
for OBS-native checkouts, the synced HEAD for git-backed checkouts. The
git config records the OBS-side coordinates (`geectl.base-rev`, `geectl.base-srcmd5`)
for server reads. The OBS revision and the git SHA are two different base
identities: the git SHA is the checkout's base, the OBS srcmd5 names the
same base on the server side. If the recorded coordinates disagree with
the base commit, the record is healed from the base commit: the commit is
the source of truth, and a live open re-reads the record from it, so
an update stopped mid-rebase still resolves against the new base. A
read-only open (`open_read`, used by read paths and dry-run plans) returns
the root commit's coordinates without rewriting the record. That
healing is OBS-native only: a git-backed checkout records the base in
config, not in a root commit. `resolve` adopts the new base after a
successful continue on both backends — OBS-native from the new root
commit's message, git-backed from the rebase target — so the record
never keeps pointing at the old base after the rebase finishes. A root
commit that is not a `geectl base …` commit is `WorkingCopyInconsistent`.

## Commit protocol

A plain file PUT is its own revision; the commit is finalized by
`commitfilelist`, not by the PUTs.

- `PUT /source/:prj/:pkg/:file` with no `rev` param creates a real revision
  immediately. Geectl's current `upload_file` (plain PUT, returns the
  revision) is not a commit building block: N files uploaded that way
  would be N revisions.
- `PUT ...?rev=repository` stages files in the file store with no revision
  created. This is the staging upload the existing tool uses.
- `POST /source/:prj/:pkg?cmd=commitfilelist` is the finalizer: the client
  sends the full file list (name + md5); the server answers `error=missing`
  with the files it does not have yet; the client stages those with
  `rev=repository`; the final call creates exactly one revision, then runs
  server-side services unless `noservice`. A validate pass with a complete
  file list also creates a revision: the server commits whenever nothing
  is missing, and `withvalidate` only adds the SHA validation — so the
  sequence runs exactly one validate pass, and the final pass reports
  anything still missing itself (a stage that raced is retried, bounded).

So the layer needs two library additions (small, typed, no raw XML out):

- `put_file_staged(...)` — `upload_file` with `rev=repository`; returns no
  revision (the server answers `rev=repository`).
- `commit_filelist(...)` — POST `cmd=commitfilelist` with the file list
  (name + md5 + optional `sha256:` hash), `comment`, `user`, and the link
  flags (`keeplink`, `repairlink`, `linkrev`, `noservice`,
  `withvalidate` for the first pass). Returns the new revision + file list,
  or the `missing` list. The SHA256 hash challenge is parsed by this
  method and answered by the caller, which holds the file content: the
  final pass carries the digest.

Commit sequence (mirrors the existing tool's `Package.commit`, with the
local service gate added and the parts geectl does not do left out):
refuse if base rev != upstream rev; refuse on conflicts; refuse with
nothing to do; run the service gate (see "Services before commit");
validate pass (`withvalidate=1`) → stage missing files
(`rev=repository`) → final pass → one revision → poll until the
server-side services settle → single base refresh → report open requests
for the package. A validate pass that answers with a revision already
committed: no final pass runs, and the settle poll and base refresh run on
that revision. The file list sent to `commitfilelist` is the full list
derived from the base commit plus the staged diff; deletions are expressed
by omission from the list. If the response carries
`<serviceinfo code="running">`, poll the directory until it settles
(bounded) before the base refresh — the existing tool does the same wait
after commit.

What geectl does not do at commit time: no link-to-different-project
auto-branch (the existing tool branches; geectl refuses with a structured
error naming `branch_package` — the agent decides). The refusal runs in
`commit` and in the dry-run plan alike, and names the `branch_package`
that branches the link target into this project and package. The staged
diff is derived with `--no-renames`, so a rename is a delete-plus-add:
the old path is omitted from the file list and the server deletes it.

## Git-backed commit

`commit` dispatches on the recorded backend kind (OBS `commitfilelist`
vs git flow via the Gitea ops surface) behind the same operation; the
agent's call does not change. On a git-backed checkout, `commit` pushes
with the git binary through the working-copy `Git` wrapper, takes the
pushed head SHA, and polls the OBS side (`_scmsync.obsinfo` `commit:`)
until the synced commit matches, bounded 60 s, then reports. The
synthesized commit is authored with the checkout's repo-local identity
(`geectl <geectl@localhost>`), not the user's configured git identity,
and is pushed as such. The push authenticates with the configured Gitea
token, scoped to the Gitea instance the same way as the clone.

## Services before commit

The service run before `commit` is a hard gate: a failing service run
blocks the commit. The run is local, before the commit reaches the server,
and covers the services the services layer (issue #19) defines for a
pre-commit gate. The gate applies to both commit paths (OBS
`commitfilelist` and the git flow). Server-side services still run after
`commitfilelist` (polled until settled, above); the local run does not
replace them. What the gate changes in a tracked file is staged into the
index after the gate, so the commit sends it; new files are never staged
by this, and unstaged changes to other files are not staged. A file the
gate changed is staged whole, including any pre-existing unstaged edit
to the same file, so that edit is sent with the commit.

## `changelog`

`changelog` is a validation/generation layer for `.changes` entries. The rules
and the evidence live in issue #153; the layer shape is:

- `changelog validate`: runs the deterministic rules (V1–V8, V10–V12) on the
  working copy against its target (local diff before commit, `request diff`
  after filing), with the V9 too-terse rule as a `needs-detail` heuristic.
  Exit non-zero naming the failing rule. No LLM.
- `changelog generate`: emits the skeleton entry from the diff into the `.changes`
  file, with explicit slots for what it cannot supply (reason, upstream
  highlights). It never emits a bare version-only entry: a version-only
  entry is always left flagged incomplete, whether or not upstream news
  exists. Flagged incomplete means the entry carries an explicit marker
  naming what is missing, and `changelog validate` fails it until a human fills
  the slots.
- Human step: fill reasons and news bullets; the layer re-validates.
- Placement: `validate` runs pre-commit (the working-copy hard gate),
  pre-file, and after filing against the filed diff.

## Why git, why not `.osc`, why not a state file

- Git is the store. History, merge, and conflict markers already exist;
  an API-only state file would re-implement them, and the version-control
  behaviour of the existing tool is the part agents trip over. The
  checkout is a plain git repository: local edits, patch, and build work
  directly in it, with git.
- `.osc` is rejected: the existing tool's store is versioned, and geectl
  would own that format without controlling it — every store-version bump
  and every bookkeeping quirk becomes geectl's surface, plus the git-store
  split. Interoperability (humans reading the same directory with the
  existing tool) is not an agent use case: agents check out, edit, and
  commit through geectl; nothing else needs to read the directory.
- The base revision is the root commit (OBS-native) or the synced HEAD
  (git-backed), so the base content is in the repository itself; there is
  no separate pristine-copy cache. The git config holds coordinates, not
  file bodies.

## One API per issue #23 (git vs OBS)

The layer exposes one operation per verb, and the backend kind never
changes the call the agent makes:

- The 9 operations are the same verbs with the same arguments for both
  backends. `checkout` and `update` populate the base differently per
  backend (clone vs synthesized repository; fetch vs download), and
  `commit` dispatches on the backend kind. `status` and `diff` run the
  same local git operations against the base in both cases: the OBS source
  API serves the synced tree of a git-backed package exactly like an
  OBS-native one, so the same code path runs. The backend kind is recorded
  in the checkout at checkout time, for routing writes — never for
  choosing a different read path.
- No git-mode refusals copied from the existing tool. The layer operates
  on the checkout through git, via a small `Git` wrapper around the git
  binary (argv-only, no shell). The wrapper passes
  `-c commit.gpgsign=false -c core.autocrlf=false -c core.hooksPath=/dev/null -c protocol.ext.allow=never`:
  no signing, no line-ending conversion — the working copy holds the
  server bytes exactly on every platform — no user hooks, and no `ext::`
  transport. It scrubs `GIT_*` environment variables (directory, config,
  identity, and transport variables) before every spawn, so the user's
  environment cannot redirect a call; the only `GIT_*` a call carries are
  the ones the layer sets itself (the token header, a temporary index).
  Mutating git calls add `--no-verify`; diff output adds
  `--no-ext-diff --no-color --no-textconv`; pathspecs are literal
  (`--literal-pathspecs`), so a file named like a glob is a path, never
  a pattern. Clone, fetch, and push use a configurable network timeout
  (`GEectl_GIT_NETWORK_TIMEOUT_SECS`, seconds, default 300); local git
  operations use the fixed 300 s timeout.

## Conflicts, DryRun, errors

- **Conflicts:** on `update`, local commits rebase onto the new base and
  git merges text changes. A real conflict stays in the tree as a merge
  conflict, `status` shows `C`, and the error names the files. The agent
  edits the file, then runs `resolve`, which stages the resolved files
  (the git equivalent of marking resolved); the agent owns the content —
  the layer does not verify the merge. A file that still holds conflict
  markers is refused at `resolve`, not staged. When the resolutions are already
  staged, `resolve` continues the rebase directly. Unrelated unstaged edits are
  stashed around the rebase continue (git refuses to continue with a
  dirty worktree) and restored afterwards, so they stay unstaged; when
  the continue fails and the stashed edits cannot be restored, the error
  names the stash they stay in.
  `commit` refuses while any file is conflicted.
- **DryRun:** same convention as the rest of the library — the DryRun guard
  runs before the credentials check. A dry-run resolves no Gitea
  credentials and opens the checkout read-only (`open_read`): the base
  record is read, never rewritten, so a read path cannot heal or move
  it. `pull` is the exception on the OBS side: its dry-run plan reads
  the authed expanded routes, so it resolves OBS credentials even in
  dry-run; the plan is read-only, so no mutation can happen.
  `checkout` in DryRun lists what would
  be downloaded, writes nothing. `update`/`commit` print the plan (files
  to PUT, files to delete, the would-be revision is not created) and touch
  neither server nor local state. `add`/`rm` in DryRun are no-ops that
  report the intent change.
- **New error kinds** (central store, `src/error.rs`): `WorkingCopyNotFound`
  (not a geectl checkout), `WorkingCopyOutdated` (base rev != upstream rev
  at commit), `WorkingCopyInconsistent` (checkout not in a usable state,
  or the recorded base coordinates disagree with the base commit),
  `FileConflict` (merge conflict left in the tree), `NothingToCommit`,
  `NoGit` (the git binary is not installed).
  Each with the remediation in the message; exit codes per kind, one exit
  path.
- **Size limit / large files:** none. Files over the transport cap stream
  to disk; the layer never refuses a file for size.

## Test story (from the 36-test spec)

The cases below are the transfers adopted, with the git remapping stated. The counts are osc's, not
geectl's.

Transfers to the layer's tests: `test_commit.py` (16) — the commit
protocol sequence (validate → final `commitfilelist`, per-file staged PUT,
open-request check, refuse on conflict, refuse with nothing to do, partial
commit by file list, added-file-missing refusal, remote-exists idempotence,
`keeplink=1`, SHA256 digest variants); `test_update.py` (20) — file-list
diff semantics (no changes, new file, new-file/local collision, upstream
delete/modify, both-modified conflict, local deletions, restore,
`_service:*` handling, brand-new package with a root commit over the empty
tree). The both-modified case becomes a git merge conflict left in the
tree; an in-progress rebase conflict blocks `update`, it is not skipped.
The resume cases become "interrupted update leaves a valid old base"
cases. Vocabulary reference only: `test_package_status.py` /
`test_project_status.py`.

Does not transfer: every `.osc` store/bookkeeping test, `test_difffiles.py`
(unified-diff rendering — the layer's diff output is JSON data),
`test_vc.py` (the changelog layer has its own tests under issue #153). Temporal
live tests assert on server state per test: checkout lands the files,
commit creates exactly one revision, update moves the base rev, a refused
`commit` (conflicted, outdated, or nothing to do) leaves the server
untouched.

## Open points

1. **Filing** waits on the working copy (unified pull-request surface,
   issue #112, step 6: labels); the working copy is implemented.
2. **Pre-commit changelog validation** is a follow-up after the
   `changelog` layer lands; the service gate is the commit gate until
   then.
