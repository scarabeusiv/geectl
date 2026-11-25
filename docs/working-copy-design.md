# geectl working-copy layer — design

Issue: https://github.com/scarabeusiv/geectl/issues/17
Status: implemented.
Depends on: https://github.com/scarabeusiv/geectl/issues/20 (Gitea ops surface), https://github.com/scarabeusiv/geectl/issues/112 (unified pull-request surface), https://github.com/scarabeusiv/geectl/issues/19 (services layer), https://github.com/scarabeusiv/geectl/issues/153 (changelog layer).

## The layer, up front

The 10 operation groups (`checkout`, `update`, `status`, `diff`,
`add`/`rm`/`addremove`, `commit`, `changelog`, `mkpac`, `pull`, `resolve`) are one design
decision: geectl keeps a working copy, and the working
copy is a git repository.

**The checkout is a git repository.** A git-backed package is a real clone
of its Gitea repository. An OBS-native package is a synthesized repository
whose root commit is the base revision. `update` is a rebase. `commit`
derives the full file list from the base plus the staged diff and sends it
through the commit protocol. The agent's call is the same for either
backend: one operation per verb, and the backend kind recorded at
checkout routes writes only.

## The 10 operations, mapped

All reads use the typed API; the only new server calls the layer needs are
the two commit-protocol calls (see "Commit protocol").

| Operation | API calls | Local state |
|---|---|---|
| `checkout` | OBS-native: `list_packages` (project form) or `list_files` + `cat` per file + `package_meta`. Downloads are pinned to the listing's revision and md5-verified (see `update`). Git-backed: `package_meta` (to read the scmsync URL) + clone (the `#branch` fragment names the branch to check out and is stripped before the clone; the clone takes the bare URL). The scmsync URL must be `http(s)`: any other scheme is refused, the clone runs with `--` before the URL and `GIT_ALLOW_PROTOCOL=http:https`. | Writes files to `<dir>/<project>/<package>/`. A `:` in a project or package name becomes `_` in the directory name (Windows paths exclude `:`); the recorded project and package keep the real names. A `:` in a file name keeps its server name, so on Windows such a file blocks the checkout there. Git-backed package: real clone of the Gitea repository. OBS-native package: synthesized repository whose root commit holds the base revision content; a brand-new package synthesizes a root commit over the empty tree. Records `apiurl`, `project`, `package`, `backend` (`obs` \| `git`), `base-rev`, `base-srcmd5` in the checkout's git config (`geectl.*`). An OBS-native checkout runs credential-free; a git-backed checkout clones with the configured Gitea token when one is set. The token is scoped to the configured Gitea instance (`http.<gitea-url>.extraHeader`): git sends it only to that instance. The root commit is built with `git add -f -A`, so a downloaded file matching the user's global excludes is still committed. |
| `update` | OBS-native: `list_files` at latest rev (or `rev=`), `cat` for the files the listing adds or changes — a download is pinned to the listing's revision (`rev=<n>`) and its md5 is verified against the downloaded bytes (hashed in chunks from disk); a file whose listing md5 matches the base blob is reused from the object store. Git-backed: fetch from the Gitea remote. | Rebase. OBS-native: the new base becomes the new root commit and local commits rebase onto it. Git-backed: local commits rebase onto the synced HEAD. Git merges text changes; real conflicts stay in the tree as merge conflicts. Local commits survive. Dirty worktree changes are stashed before the rebase (a manual stash, popped with `--index` so staged intent survives); a pop that conflicts leaves the entry in the stash and the conflict in the tree, and the report names both (`conflicts`, `stash_remaining`). Downloads stream to disk. |
| `status` | Local only | Compares the working tree and the index against the base commit. Same single-letter vocabulary as the existing tool (`A`/`M`/`D`/`?`/`!`/`C`/`R`, blank for clean). The mapping from git states to letters is the layer's own, pinned by the layer's tests. Every file gets a status, whatever its size. |
| `diff` | Local diff: local only. Link diff: `list_files` at the pinned revision (at latest when the link's baserev is unset) + `cat` per target file pinned to the listing's srcmd5 (public routes, credential-free); `project_diff` for remote-vs-remote | Local git diff of the staged index against the base commit plus the unstaged worktree against the index — the two halves together, so a staged-only change shows. On a linked checkout a link section follows: the staged index overlaid on the link target's content at the `_link` baserev (the target's latest srcmd5 when the link's baserev is unset), diffed against that content, so the link's delta over its target shows in the same staged/worktree split. `_link` itself is excluded from the link section; when `_link` is conflicted the plain halves still show the conflict. The target read is the target's own files at the pinned revision. When the target read fails (a missing srcmd5, a deleted target, a malformed `_link`, the network down) the plain halves still print, and the failure is a stderr note. A conflicted file other than `_link` falls back to the target's version in the link section. A chained link resolves one level only: a file the target itself inherits from its target is absent from the target tree and shows as a pure addition when the link carries it. Binary files appear as git's `Binary files … differ` line in the diff text. Remote-vs-remote diff reuses `project_diff`. |
| `add` / `rm` / `addremove` | Local intent only | Stage into the index: `add` stages, `rm` removes from index and working tree. `addremove` is `status` + add of `?` + rm of `!`. `commit` sends the full file list derived from the base plus the staged diff. |
| `commit` | `commit_filelist` (validate, then final) + staged file PUTs (`rev=repository`) + `open_requests_for_package` (report after) | Requires base rev == upstream rev, else `WorkingCopyOutdated`. One revision per commit, exactly (see protocol). Services run before commit, hard gate: a failing service run blocks the commit. Dispatches on the recorded backend kind (see "Git-backed commit"). Refreshes the base at the new rev: OBS-native, the new server revision becomes the new root commit and the committed local commits' content is in it, their separate identities gone; git-backed, the pushed HEAD is the new base. |
| `changelog` | Local pre-commit; validate after filing reads `request diff` | `changelog validate` + `changelog generate` (see "`changelog`"). Runs pre-commit in the hard gate, pre-file, and after filing. |
| `mkpac` | `create_package` (exists) + `checkout` | The checkout holds the state. |
| `pull` | Expanded directory read (`list_files` with expand) + expanded file downloads + `update` | Only for linked packages with a baserev and a clean WC: zero uncommitted changes, zero untracked files (local commits are rebased). A pull needs a baserev. The clean-tree check reads the untracked half with plain `ls-files --others` (no `--exclude-standard`): `status` hides ignored untracked files, and an ignored file blocks the pull like any other untracked file. Rebases link-target changes into the WC, like `update`. The server's expanded tree excludes `_link`, so pull fetches `_link` through the unexpanded route and rewrites its baserev to the target's latest srcmd5. Expanded downloads are pinned to the expanded listing's revision and md5-verified. A directory read that comes back revision-less is waited for (bounded retry of the revision read only); after the budget the pull errors. The pin is a revision spec (a number, or the expanded content's md5 for a linked package); `_link` downloads pinned to the link package's own revision. |
| `resolve` | Local git only | Stage the resolved files and continue a stopped rebase. A file that still holds conflict markers (`<<<<<<<` / `=======` / `>>>>>>>`) fails resolve. A successful continue adopts the new base — OBS-native from the new root commit's message, git-backed from the rebase target — on disk and in the checkout. Unstaged edits are stashed around the continue and restored after; a failed restore strands them in the stash (named in the error) with the base already adopted. An autostash entry left by an earlier stopped update is restored at the end of `resolve` (or immediately when resolve is a no-op). |

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
the root commit's coordinates, leaving the record unchanged. That
healing applies to OBS-native checkouts: a git-backed checkout records the base in
config. `resolve` adopts the new base after a
successful continue on both backends — OBS-native from the new root
commit's message, git-backed from the rebase target — so the record
points at the new base after the rebase finishes. A root
commit other than a `geectl base …` commit is `WorkingCopyInconsistent`.

## Commit protocol

A plain file PUT is its own revision; `commitfilelist` finalizes the
commit.

`PUT /source/:prj/:pkg/:file` with the `rev` param omitted creates a real revision immediately. Geectl's `upload_file` (plain PUT, returns the revision) is a single-revision upload: N files uploaded that way are N revisions.
`PUT ...?rev=repository` stages files in the file store; the revision is created by the finalizer. This is the staging upload the existing tool uses.
`POST /source/:prj/:pkg?cmd=commitfilelist` is the finalizer: the client sends the full file list (name + md5); the server answers `error=missing` with the files still to stage; the client stages those with `rev=repository`; the final call creates exactly one revision, then runs server-side services unless `noservice`. A validate pass with a complete file list also creates a revision: the server commits whenever the file list is complete, and `withvalidate` adds the SHA validation — so the sequence runs exactly one validate pass, and the final pass reports anything still missing itself (a stage that raced is retried, bounded).

So the layer needs two library additions (small, typed, typed values only):

`put_file_staged(...)` — `upload_file` with `rev=repository`; the server answers `rev=repository`.
`commit_filelist(...)` — POST `cmd=commitfilelist` with the file list (name + md5 + optional `sha256:` hash), `comment`, `user`, and the link flags (`keeplink`, `repairlink`, `linkrev`, `noservice`, `withvalidate` for the first pass). Returns the new revision + file list, or the `missing` list. The SHA256 hash challenge is parsed by this method and answered by the caller, which holds the file content: the final pass carries the digest.

Commit sequence: base rev == upstream rev, a conflict-free tree, something to
commit; run the service gate (see "Services before commit");
validate pass (`withvalidate=1`) → stage missing files
(`rev=repository`) → final pass → one revision → poll until the
server-side services settle → single base refresh → report open requests
for the package. A validate pass that answers with a revision already
committed ends the sequence there: the settle poll and base refresh run
on that revision. The file list sent to `commitfilelist` is the full list
derived from the base commit plus the staged diff; a deletion is a path
absent from the list. If the response carries
`<serviceinfo code="running">`, poll the directory until it settles
(bounded) before the base refresh.

At commit time, a link to a different project fails with a structured
error naming `branch_package` — the agent decides. The failure runs in
`commit` and in the dry-run plan alike, and names the `branch_package`
that branches the link target into this project and package. The staged
diff is derived with `--no-renames`, so a rename is a delete-plus-add:
the old path is absent from the file list and the server deletes it.

## Git-backed commit

`commit` dispatches on the recorded backend kind (OBS `commitfilelist`
vs git flow via the Gitea ops surface) behind the same operation; the
agent's call is the same. On a git-backed checkout, `commit` pushes
with the git binary through the working-copy `Git` wrapper, takes the
pushed head SHA, and polls the OBS side (`_scmsync.obsinfo` `commit:`)
until the synced commit matches, bounded 60 s, then reports. The
synthesized commit is authored with the checkout's repo-local identity
(`geectl <geectl@localhost>`) and is pushed as such. The push authenticates with the configured Gitea
token, scoped to the Gitea instance the same way as the clone.
Before the push, `commit` runs the pre-push sanity checks (see "Pre-push sanity checks"): a failed check blocks the push.

## Services before commit

The service run before `commit` is a hard gate: a failing service run
blocks the commit. The run is local, before the commit reaches the server,
and covers the services the services layer (issue #19) defines for a
pre-commit gate. The gate applies to both commit paths (OBS
`commitfilelist` and the git flow). Server-side services run after
`commitfilelist` as well (polled until settled, above); the local run is
in addition to them. What the gate changes in a tracked file is staged into the
index after the gate, so the commit sends it; the gate stages only files
it changed. A file the
gate changed is staged whole, including any pre-existing unstaged edit
to the same file, so that edit is sent with the commit.

## Pre-push sanity checks

The git sanity checks run as a pre-push gate for git-backed packages,
the same way `pre_checkin` runs before commit: a failed check blocks
the push with an actionable message. The gate runs after the
synthesized commit exists, so it checks every commit the push would
send, and before the push itself. The gate covers git-backed pushes; OBS-native commits take the `commitfilelist` path. The checks are local only and cover the three states the issue names:

- Hooks: an executable `pre-push` hook in the checkout's effective
  hooks directory blocks the push. The directory is `core.hooksPath`
  when set, else the common git directory's `hooks` (in a linked
  worktree the hooks live in the main checkout's git directory, not
  the worktree's). The value is read scoped — `--worktree` first
  when `extensions.worktreeConfig` is on, then `--local`, then
  `--global`, then `--system` — and the first scope that sets it
  wins; a relative path resolves against the
  checkout root, and a leading `~` expands the way git expands it
  (`~/` from `$HOME`, `~user/` from that user's home directory;
  on Windows git cannot expand `~user/` at all, so such a path
  refuses the push). A
  hooks path that is a file, not a directory, refuses the push
  naming the path itself. The push runs with `--no-verify` and
  `core.hooksPath=/dev/null`; geectl runs its own pre-push checks
  instead of repository hooks. The push is refused until the hook is
  removed. The synthesized commit skips `pre-commit` hooks.
- Per-commit state: every commit in the push range carries a
  non-empty message and is not empty (its tree differs from its
  parent's). Merge commits are exempt from the empty check: their
  tree routinely matches one parent's.
- Submodule state: every gitlink a pushed commit changes names a
  URL in that commit's `.gitmodules` and points at a commit object
  present locally, in the checkout or in the submodule's own
  checkout. The gate covers pointers a pushed commit changes; a
  checkout whose submodules were never initialized still pushes
  commits that do not touch them. A pointer that cannot be verified fails
  the gate: the OBS sync would fetch it, and a revision that does
  not exist breaks the package there. A commit that moves a
  pointer needs the submodule fetched first (`git submodule update
  --init <path>`), before the commit.

The gate and the push run under an exclusive lock on the checkout
(a lock file in the common git directory), taken before the
synthesized commit is created and held until the push returns, so
a concurrent commit cannot land between the gate and the push and
be sent ungated. A failure inside the gate itself (a git call that
cannot run, a lock that cannot be taken) is a gate failure too: it
refuses the push as `PrePushCheckFailed`, exit 34.

The checks are geectl's own, written against the git repository
directly.

## `changelog`

`changelog` is a validation/generation layer for `.changes` entries. The layer shape is:

`changelog validate`: runs the deterministic rules (V1–V8, V10–V12) on the working copy against its target (local diff before commit, `request diff` after filing), with the V9 too-terse rule as a `needs-detail` heuristic. Exit non-zero naming the failing rule. The rules are deterministic; V9 is a needs-detail heuristic.
`changelog generate`: emits the skeleton entry from the diff into the `.changes` file, with explicit slots for the missing pieces (reason, upstream highlights). A version-only entry is left flagged incomplete, whether or not upstream news exists. Flagged incomplete means the entry carries an explicit marker naming what is missing, and `changelog validate` fails it until a human fills the slots.
Human step: fill reasons and news bullets; the layer re-validates.
Placement: `validate` runs pre-commit (the working-copy hard gate), pre-file, and after filing against the filed diff.

## Why git

Git is the store. History, merge, and conflict markers already exist; an API-only state file would re-implement them, and the version-control behaviour of the existing tool is the part agents trip over. The checkout is a plain git repository: local edits, patch, and build work directly in it, with git.
`.osc`: the existing tool's store is versioned, and geectl would own that format, controlled elsewhere — every store-version bump and every bookkeeping quirk becomes geectl's surface, plus the git-store split. Agents check out, edit, and
commit through geectl; the directory is geectl's alone to read.
The base revision is the root commit (OBS-native) or the synced HEAD (git-backed), so the base content is in the repository itself. The git config holds
coordinates only.

## One API per issue #23 (git vs OBS)

The layer exposes one operation per verb, and the backend kind leaves
the call the agent makes unchanged:

The 10 operations are the same verbs with the same arguments for both backends. `checkout` and `update` populate the base differently per backend (clone vs synthesized repository; fetch vs download), and `commit` dispatches on the backend kind. `status` and `diff` run the same local git operations against the base in both cases: the OBS source API serves the synced tree of a git-backed package exactly like an OBS-native one, so the same code path runs. The backend kind is recorded in the checkout at checkout time, for routing writes only.
The layer operates on the checkout through git, via a small `Git` wrapper around the git binary (argv-only, no shell). The wrapper passes `-c commit.gpgsign=false -c core.autocrlf=false -c core.hooksPath=/dev/null -c protocol.ext.allow=never`, so the working copy holds the server bytes exactly on every platform. It scrubs `GIT_*` environment variables (directory, config, identity, and transport variables) before every spawn, so the user's environment leaves every call unaffected; a call carries only the `GIT_*` the layer sets itself (the token header, a temporary index). Mutating git calls add `--no-verify`; diff output adds `--no-ext-diff --no-color --no-textconv`; pathspecs are literal (`--literal-pathspecs`), so a file named like a glob is treated as a path. Clone, fetch, and push use a configurable network timeout (`GEectl_GIT_NETWORK_TIMEOUT_SECS`, seconds, default 300); local git operations use the fixed 300 s timeout.

## Conflicts, DryRun, errors

**Conflicts:** on `update`, local commits rebase onto the new base and git merges text changes. A real conflict stays in the tree as a merge conflict, `status` shows `C`, and the error names the files. The agent edits the file, then runs `resolve`, which stages the resolved files (the git equivalent of marking resolved); the agent owns the content. A file that still holds conflict markers fails `resolve`. When the resolutions are already staged, `resolve` continues the rebase directly. Unrelated unstaged edits are stashed around the rebase continue (git requires a clean worktree to continue) and restored afterwards, so they stay unstaged; when the continue fails with the stashed edits still in the stash, the error names it. `commit` requires a conflict-free tree.
**DryRun:** same convention as the rest of the library — the DryRun guard runs before the credentials check. A dry-run opens the checkout read-only (`open_read`) and skips Gitea
credential resolution: the base record is read, leaving it unchanged.
`pull` is the exception on the OBS side: its dry-run plan reads the
authed expanded routes and resolves OBS credentials even in dry-run;
the plan is read-only.
  `checkout` in DryRun lists what would
be downloaded. `update`/`commit` print the plan (files
  to PUT, files to delete) only. `add`/`rm` in DryRun report the intent change only.
**New error kinds** (central store, `src/error.rs`): `WorkingCopyNotFound` (outside a geectl checkout), `WorkingCopyOutdated` (base rev != upstream rev at commit), `WorkingCopyInconsistent` (checkout in an unusable state, or the recorded base coordinates disagree with the base commit), `FileConflict` (merge conflict left in the tree), `NothingToCommit`, `NoGit` (git binary absent), `PrePushCheckFailed` (a pre-push sanity check failed; the context names the failing check). Each with the remediation in the message; exit codes per kind, one exit path.
**Size limit / large files:** unlimited. Files over the transport cap stream to disk.

## Test story

The layer's tests live in `src/working_copy.rs` and `src/obs/remote.rs`: `commit_obs_validate_pass_that_commits_skips_the_final_pass` and `commit_filelist_final_pass_returns_rev_srcmd5_and_service_state` cover the commit protocol sequence (validate → final `commitfilelist`); `commit_obs_sends_the_index_state` covers the per-file staged PUT; `commit_filelist_validate_pass_returns_the_missing_list` covers the missing list; `commit_obs_sends_keeplink_when_the_link_is_staged` covers `keeplink=1`; `commit_obs_answers_a_sha256_challenge` covers the SHA256 digest challenge. `commit_obs_refuses_an_outdated_base`, `commit_obs_refuses_nothing_to_commit`, and `commit_refuses_with_conflicts` cover the refusals. `update_obs_rebases_local_commits_onto_the_new_base` covers the update rebase; `update_git_stops_on_a_conflict` covers the both-modified case, a git merge conflict left in the tree. An in-progress rebase conflict blocks `update`. Vocabulary reference: `status_maps_porcelain_codes`.

Live tests assert on server state per test (`live_working_copy_checkout_edit_commit_update`, `live_working_copy_pull_pulls_a_linked_package`): checkout lands the files, commit creates exactly one revision, update moves the base rev, a refused `commit` (conflicted, outdated, or nothing to do) leaves the server untouched.

