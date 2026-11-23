# geectl working-copy layer — design

Issue: https://github.com/scarabeusiv/geectl/issues/17
Status: design. The layer is blocked until the Gitea ops surface exists (see "Git-backed commit").
Depends on: https://github.com/scarabeusiv/geectl/issues/20 (Gitea ops surface), https://github.com/scarabeusiv/geectl/issues/112 (unified pull-request surface), https://github.com/scarabeusiv/geectl/issues/19 (services layer), https://github.com/scarabeusiv/geectl/issues/153 (changelog layer).

## The layer, up front

The 9 local-state operations (`checkout`, `update`, `status`, `diff`,
`add`/`rm`/`addremove`, `commit`, `changelog`, `mkpac`, `pull`) are one design
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
| `checkout` | OBS-native: `list_packages` (project form) or `list_files` + `cat` per file + `package_meta`. Git-backed: `package_meta` (to read the scmsync URL) + clone. | Writes files to `<dir>/<project>/<package>/`. Git-backed package: real clone of the Gitea repository. OBS-native package: synthesized repository whose root commit holds the base revision content; a brand-new package synthesizes a root commit over the empty tree. Records `apiurl`, `project`, `package`, `backend` (`obs` \| `git`), `base_rev`, `base_srcmd5` in the checkout's git config (`geectl.*`). No credentials, ever. |
| `update` | OBS-native: `list_files` at latest rev (or `rev=`), `cat` for changed files. Git-backed: fetch from the Gitea remote. | Rebase. OBS-native: the new base becomes the new root commit and local commits rebase onto it. Git-backed: local commits rebase onto the synced HEAD. Git merges text changes; real conflicts stay in the tree as merge conflicts. No local commit is lost. Downloads stream to disk; no size refusal. |
| `status` | `list_files` (remote listing, for `?` vs `A` disambiguation) | Compares the working tree and the index against the base commit. Same single-letter vocabulary as the existing tool (`A`/`M`/`D`/`?`/`!`/`C`/`R`, blank for clean). The mapping from git states to letters is the layer's own, pinned by the layer's tests. There is no `S` status: files are never skipped for size. |
| `diff` | None for local diff; `project_diff` for remote-vs-remote | Local git diff of the working tree against the base commit. Binary files report "binary, no diff" as a structured result, not a failure. Remote-vs-remote diff reuses `project_diff`. |
| `add` / `rm` / `addremove` | None (local intent only) | Stage into the index: `add` stages, `rm` removes from index and working tree. `addremove` is `status` + add of `?` + rm of `!`. `commit` sends the full file list derived from the base plus the staged diff. |
| `commit` | `commit_filelist` (validate, then final) + staged file PUTs (`rev=repository`) + `open_requests_for_package` (report after) | Requires base rev == upstream rev, else `WorkingCopyOutdated`. One revision per commit, exactly (see protocol). Services run before commit, hard gate: a failing service run blocks the commit. Dispatches on the recorded backend kind (see "Git-backed commit"). Refreshes the base at the new rev: OBS-native, the new server revision becomes the new root commit and the committed local commits' content is in it, their separate identities gone; git-backed, the pushed HEAD is the new base. |
| `changelog` | None pre-commit; validate after filing reads `request diff` | `changelog validate` + `changelog generate` (see "`changelog`"). Runs pre-commit in the hard gate, pre-file, and after filing. |
| `mkpac` | `create_package` (exists) + `checkout` | No new operation. |
| `pull` | Expanded directory read (`list_files` with expand) + `update` | Only for linked packages with a baserev and a clean WC (same gates as the existing tool: refuse on local changes, refuse on non-expanded links, refuse without baserev). Rebases link-target changes into the WC, like `update`. |

## The base record

The authoritative base for content is the base commit: the root commit
for OBS-native checkouts, the synced HEAD for git-backed checkouts. The
git config records the OBS-side coordinates (`base_rev`, `base_srcmd5`)
for server reads. The OBS revision and the git SHA are two different base
identities: the git SHA is the checkout's base, the OBS srcmd5 names the
same base on the server side. If the recorded coordinates disagree with
the base commit, the checkout is `WorkingCopyInconsistent`.

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
  server-side services unless `noservice`.

So the layer needs two library additions (small, typed, no raw XML out):

- `put_file_staged(...)` — `upload_file` with `rev=repository`; returns no
  revision (the server answers `rev=repository`).
- `commit_filelist(...)` — POST `cmd=commitfilelist` with the file list
  (name + md5 + optional `sha256:` hash), `comment`, `user`, and the link
  flags (`keeplink`, `repairlink`, `linkrev`, `noservice`,
  `withvalidate` for the first pass). Returns the new revision + file list,
  or the `missing` list. The SHA256 hash challenge is part of this method,
  not caller logic.

Commit sequence (mirrors the existing tool's `Package.commit`, with the
local service gate added and the parts geectl does not do left out):
refuse if base rev != upstream rev; refuse on conflicts; refuse with
nothing to do; run the service gate (see "Services before commit");
validate pass (`withvalidate=1`) → stage missing files
(`rev=repository`) → final pass → one revision → poll until the
server-side services settle → single base refresh → report open requests
for the package. The file list sent to `commitfilelist` is the full list
derived from the base commit plus the staged diff; deletions are expressed
by omission from the list. If the response carries
`<serviceinfo code="running">`, poll the directory until it settles
(bounded) before the base refresh — the existing tool does the same wait
after commit.

What geectl does not do at commit time: no link-to-different-project
auto-branch (the existing tool branches; geectl refuses with a structured
error naming `branch_package` — the agent decides).

## Git-backed commit

The layer does not ship until the Gitea ops surface exists. When it lands,
`commit` dispatches on the recorded backend kind (OBS `commitfilelist` vs
git flow via the Gitea ops surface) behind the same operation; the agent's
call does not change. No refusal exists for git-backed checkouts: the
layer ships only after the Gitea ops surface lands.

Open question: sync freshness on git-backed commit — whether `commit`
polls until the Gitea mirror is synced (bounded 60 s) or refuses on a
stale sync. That question belongs to the Gitea ops design
(https://github.com/scarabeusiv/geectl/issues/20;
`docs/gitea-ops-design.md`, landing with the Gitea ops surface); it is
not settled here.

## Services before commit

The service run before `commit` is a hard gate: a failing service run
blocks the commit. The run is local, before the commit reaches the server,
and covers the services the services layer (issue #19) defines for a
pre-commit gate. The gate applies to both commit paths (OBS
`commitfilelist` and the git flow). Server-side services still run after
`commitfilelist` (polled until settled, above); the local run does not
replace them.

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
  on the checkout through git; the binding (git binary vs library) is an
  implementation choice, not settled here.

## Conflicts, DryRun, errors

- **Conflicts:** on `update`, local commits rebase onto the new base and
  git merges text changes. A real conflict stays in the tree as a merge
  conflict, `status` shows `C`, and the error names the files. The agent
  edits the file, then runs `resolve`, which stages the resolved files
  (the git equivalent of marking resolved); the agent owns the content —
  the layer does not verify the merge. `commit` refuses while any file is
  conflicted.
- **DryRun:** same convention as the rest of the library — the DryRun guard
  runs before the credentials check. `checkout` in DryRun lists what would
  be downloaded, writes nothing. `update`/`commit` print the plan (files
  to PUT, files to delete, the would-be revision is not created) and touch
  neither server nor local state. `add`/`rm` in DryRun are no-ops that
  report the intent change.
- **New error kinds** (central store, `src/error.rs`): `WorkingCopyNotFound`
  (not a geectl checkout), `WorkingCopyOutdated` (base rev != upstream rev
  at commit), `WorkingCopyInconsistent` (checkout not in a usable state,
  or the recorded base coordinates disagree with the base commit),
  `FileConflict` (merge conflict left in the tree), `NothingToCommit`.
  Each with the remediation in the message; exit codes per kind, one exit
  path.
- **Size limit / large files:** none. Files over the transport cap stream
  to disk; the layer never refuses a file for size.

## Test story (from the 36-test spec)

Proposed — the layer is unimplemented; the cases below are the transfers
to adopt, with the git remapping stated. The counts are osc's, not
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

1. **Git-backed commit sync freshness:** poll until synced (bounded 60 s)
   vs refuse — Gitea ops design decision 3, open.
2. **Git binding:** git binary vs library — implementation choice, open.
3. **Filing** waits on the working copy (unified pull-request surface,
   issue #112, step 6: labels); the working copy waits on the Gitea ops
   surface.
