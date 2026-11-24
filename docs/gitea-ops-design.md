# geectl Gitea operations — design proposal

Issue: https://github.com/scarabeusiv/geectl/issues/20 — the Gitea
operations (fork, repo, branch, user, SSH keys) and the unified
pull-request surface of https://github.com/scarabeusiv/geectl/issues/112.
Depends on: #13 (request read/act), #17 (working-copy design; its git-backed commit dispatch point), #23
(git-vs-OBS abstraction — this design follows it), #51 (debug
channel — Gitea HTTP attempts are reported the same way as OBS ones;
see "Debug channel").

## Uniform flow

The agent-facing flow is one flow with five steps, identical on both
backends (governing target: issue #23). The backend kind (OBS-native vs
git-backed, recorded in git config `geectl.*` at checkout, #17) selects the
implementation underneath; it leaves the agent's call unchanged,
and the user-facing surface uses backend-neutral names. The separate
`GiteaClient` (§ "Typed API shape") is internal plumbing — the
transport client for the git-backed implementation.

| Flow step | OBS-native backend | Git-backed (Gitea) backend | Status |
|---|---|---|---|
| `update` | Working-copy `update` (#17): `list_files` at latest rev + `cat` for changed files, three-way compare, base manifest swap | The same `update` call: the OBS source API serves the synced tree, so the same code path runs | Settled as the same call; whether `update` itself verifies sync freshness is open (decision 8) |
| `changes` (`status` / `diff`) | Local disk vs base manifest vs remote listing (#17) | Identical — local comparison only; the read path is backend-independent | Settled |
| `commit` | `commit_filelist` + staged PUTs (`rev=repository`) → exactly one revision (#17) | The #17 dispatch point: the same `commit` call dispatches on the backend kind recorded in git config `geectl.*` at checkout. The agent pushes with the git binary through the working-copy `Git` wrapper (argv-only); `commit` takes the pushed head SHA and polls the OBS side until the synced commit matches (bounded 60 s), then reports | Settled |
| `submit` | OBS submit request: `file`/`refile` via `SrFiling` → request id | Gitea PR: `pr_create` (phase 3) — the same `submit` operation, different backend. The user sees `submit` and a pull-request reference; the backend decides whether that is an SR or a PR | Settled in principle; the unified return type is settled (decision 7) |
| `maintenance revisions` | OBS maintenance incident: `file_maintenance_incident` → incident + request | Open question (decision 9): the incident machinery is OBS-side, so the likely shape is the same OBS incident filed from the synced sources — but the Gitea-side route (a PR against the maintenance branch) is the alternative if that proves unworkable | Open |

### Uniform surface naming

The operation names in the phase tables below are the internal
`GiteaClient` method names, except where marked as a user-facing verb.
The user-facing verbs are the five flow steps above, plus `branch` for
the setup step. Names:

`fork_repo` → user-facing verb `branch`: on OBS-native it is `branch_package`; on git-backed it is the fork composite (Gitea fork + OBS `cmd=fork`). `fork_repo` stays as the internal Gitea-side step of that composite.
`pr_create` → user-facing verb `submit`: on OBS-native it files an SR; on git-backed it opens a PR. `pr_create` stays as the internal Gitea-side step.
The remaining `pr_*` operations (get/list/merge/close/reviews/ comments/labels) are the internal Gitea-side implementation of the uniform review read/act operations (OBS side: request read/act). Their exact uniform names and signatures are decided at implementation (decision 7); the user-facing surface uses the uniform names only.
`repo_*` / `branch_*` (read) / `user_*` / `ssh_key_*` are internal primitives supporting the flow steps (verification, sync checks, account setup). They stay internal primitives.

## Measured state

**What exists.** The OBS client is split (`src/obs/`) and the Gitea
client is a separate module (`src/gitea.rs`, `GiteaClient`) with the
phase 1–2 operations (fork, repo read/list/create/delete, branch
read/head, user read/search, SSH keys, teams, collaborators) and the phase-3
read operations (`pr_get`, `pr_list`, `pr_search`, `pr_reviews`)
plus the review-level act operations (`pr_review`,
`pr_comments`, `pr_comment_add`, `pr_review_cancel_request`)
and the pull-request-level act operations (`pr_merge`,
`pr_close`, `pr_reopen`).
The unified pull-request surface (`src/pull_request.rs`,
`PullRequestClient`, `pull-request` CLI group) carries the read
surface, the review-level act operations and the
pull-request-level act operations; label operations are library-only (see Phase 3). The unified person search (`src/person.rs`, `PersonClient`, `person search`) carries the OBS person-match and the Gitea user search under one operation. The OBS side has the typed `change_review_state`
(`src/obs/request.rs`).

Config (`src/config.rs`): `gitea_url` (default `https://src.opensuse.org`), `GEECTL_GITEA_URL` env override.
Auth (`src/auth.rs`): `EnvGiteaAuth` (Bearer from `GEECTL_GITEA_TOKEN` or `GEECTL_GITEA_TOKEN_FILE`, exactly one — or the matching alias's `GEECTL_GITEA_TOKEN_<ALIAS>` / `GEECTL_GITEA_TOKEN_FILE_<ALIAS>` when the URL is named in `[instances]`, resolved per URL by `resolve_gitea_bearer_for_url`), `GiteaSshKey` (private-key path from `GEECTL_GITEA_SSH_KEY`), `resolve_gitea_bearer`, `resolve_gitea_ssh_key`. The `AuthProvider` trait is the seam.
Transport (`src/http.rs`): `HttpClient::post_json` and `get` with the production status map (404 → `NotFound`, 401/403 → `AuthFailed`, 5xx → `ServerError`), loopback guard, idempotent-only retries. `serde_json` is a dependency.
OBS side, typed scmsync (`src/obs/`): `ProjectMeta` / `PackageMeta` carry `scmsync: Option<String>` (`url#branch`); `validate_scmsync` requires the `#fragment`; `create_package` takes an `scmsync` option; creating a package inside a scmsync-managed project is refused upfront; a link starts with the source's scmsync dropped.
Live harness (`tests/gitea.rs`): a disposable Gitea 1.24.7 instance (provision.sh, per-PR gating) with the typed operations exercised against the server and raw `UreqClient` calls where no typed operation exists yet.

**What osc has.** `osc/gitea_api/`:
`fork.py` (list/create), `repo.py` (get, clone, clone_or_update,
list_org/my/user_repos, label ids, parent repos, fork-tree root),
`pr.py` (create/get/list/search/set/get_patch/add_comment/reviews/
approve/decline/merge/cancel-scheduled-merge/close/reopen/labels,
cancel-review-requests), `branch.py` (get/list/create),
`git.py` (URL parse/join, local git detection), `user.py`,
`ssh_key.py`, `issue.py`, `staging.py` (group/remove),
`pr_review.py`, `git_diff_generator.py`, `tardiff.py`,
`maintainership.py`, `conf.py` (multi-login entries), `connection.py`,
`exceptions.py` (per-operation 301 mapping). The `osc fork` command
(`osc/commands/fork.py`) composes: read OBS meta → require
`<scmsync>` → resolve devel project (unless `--no-devel-project`)
→ Gitea `Fork.create` → clone the fork locally over SSH, fetch the
parent branch, `checkout -B` the fork branch → OBS
`POST /source/{project}/{package}?cmd=fork&scmsync={fork-url}`
(`obs_api/package.py`) → print the scmsync URL and "push
manually". The local clone exists so a human can review and push;
geectl leaves the local clone to the agent (decision 1).

**Test material (borrow map).** Behave `git-*.feature` and unit `test_gitea_api_*.py`, `test_git_scm_store.py`, `test_scmsync_obsinfo.py`, `test_gitea_exceptions.py`, `test_maintainership.py` + converter. Classified under
"Test borrowing" below.

## Scope gate: what an agent uses this for

Every operation below names its agent use case. The agent's git-backed
flow is: fork the package's repo → work on the fork (git CLI) → push → the OBS side syncs from scmsync → file the SR from
the synced sources. Geectl owns the API half of that flow: the fork,
the reads that verify it, the PR operations that carry the review,
and the primitives the working-copy commit dispatch needs. Local git work belongs to the git CLI (#17).

## The operations, in phases

Phases are the implementation order; each phase is its own PR. Phase 1 is the issue #20 scope (fork, repo delete) plus the one read that verifies it.

### Phase 1 — `branch` (fork), repo read (verify), repo delete

| Operation | Endpoint | Agent use case |
|---|---|---|
| `repo_get(owner, repo)` | GET `/api/v1/repos/{owner}/{repo}` | Resolve the default branch (fork needs it when no branch is given); verify a fork's parent by re-reading; the read every later phase builds on |
| `branch(source, options)` — user-facing verb; internally Gitea `fork_repo` | POST `/api/v1/repos/{owner}/{repo}/forks` + POST `/source/{project}/{package}?cmd=fork&scmsync=…` | Give the agent a place to work on a git-backed package (or project): Gitea fork + OBS fork wired to the fork's scmsync URL, in one call. On OBS-native the same verb is `branch_package`. |
| `repo_delete(owner, repo)` | DELETE `/api/v1/repos/{owner}/{repo}` | Delete a fork after the OBS branch is gone / cleanup; the `osc api -X DELETE /repos/...` case from the gap map |

`branch` on a git-backed package is a composite (internal Gitea step
`fork_repo`); its steps (all typed):

1. Read the OBS package (or project) meta; refuse with
   `InvalidInput` when it carries no `<scmsync>` ("Forking is
   possible only with packages managed in Git"). Resolve the devel project unless `no_devel_project` — fork from the devel source when a devel exists.
2. Parse the scmsync URL (`url#branch`, `trackingbranch` query
   honoured); when no branch is named, resolve it with `repo_get`
   (default branch). `fork_branch` is the explicit `--git-branch`
   or the source
   branch.
3. `Fork.create`: POST `/forks` with `{name: new_repo_name,
   organization: target_org}`. `ForkExists` returns the existing
   fork (owner + repo) instead of failing — the agent reuses it.
   A non-fork repo of the requested name is an error naming the
   alternative (new name, or delete and retry). Forking someone
   else's fork answers 202; verify the created fork by re-reading the created repo and checking its
   `parent` before reporting success.
The reuse holds only for the same name. A second fork under a different name 409s (one fork per user per source); the 409 body names the existing fork, but the transport discards it, and the fork re-reads the target by name only — so the original refusal stands. `repo_list` is the primitive for finding the fork under a different name.
4. Ensure the target project exists: the explicit target, or the
   server default `home:<user>:branches:<project>` when the acting
   OBS username is known — a missing one is generated with
   `create_project` (titled as a branch project for the package,
   and described as the test project for the git branch it is wired
   to) and the creation is reported on the debug channel. With
   Bearer credentials the default cannot be named client-side, so
   the server's branch path generates it.
5. OBS fork: `POST /source/{project}/{package}?cmd=fork` with
   `scmsync={fork-owner}/{fork-repo}#{fork_branch}` (package
   `"_project"` for a project fork), `target_project` defaulting to
   `home:<user>:branches`, `target_package` defaulting to the
   package name. The server answers the target project/package.
6. Return, typed: the fork repo (`owner`, `name`, `branch`),
   the scmsync URL, the OBS target project/package, and the
   `clone_url` / `ssh_url` for the agent's own `git clone`.
   The agent clones from the returned URLs (decision 1).

Errors: source not scmsync-managed → `InvalidInput`; Gitea 404 →
`NotFound`; 401/403 → `AuthFailed`; 422 (e.g. bad key content on
the ssh-key path, invalid repo name) → `InvalidInput` carrying the
server's message; fork-of-fork 202 with a mismatched parent →
`ServerError` naming the re-read. `repo_delete` is DryRun-guarded
like `delete_package`, GETs first to confirm owner/name, and
returns the deleted name; server-side permission is the guard
(the API enforces owner/admin-only delete).

### Phase 2 — repo list, branch read, branch head

| Operation | Endpoint | Agent use case |
|---|---|---|
| `repo_list(owner?, org?)` | GET `/api/v1/orgs/{org}/repos`, `/api/v1/users/{user}/repos`, `/api/v1/user/repos` | Find the agent's forks / an org's repos before acting on one |
| `repo_create(name, options)` | POST `/api/v1/user/repos` | Create the repo a new git-backed package's sources will live in (the git-backed new-package flow; also the fixture primitive the live tests use) |
| `branch_get(owner, repo, branch)` / `branch_list(owner, repo)` | GET `/api/v1/repos/{owner}/{repo}/branches[/{branch}]` | Verify a branch exists before naming it in a scmsync URL; list what a fork carries |
| `branch_head(owner, repo, branch)` | branch read, `commit.id` | The sync-verification primitive: the SHA the filer compares against the OBS side before filing (osc #2251, see "scmsync lag") |

List operations paginate internally (`limit=50` per page, like
osc's fork list), bounded — past the bound the call fails with a
structured error naming the filter to narrow. `meta list` honours the branch filter:
reads that take a branch take it as a typed parameter.

### Phase 3 — `submit` + review read/act (pull requests)

| Operation | Endpoint | Agent use case |
|---|---|---|
| `submit(source, target, title, body, options)` — user-facing verb; internally Gitea `pr_create` | POST `/api/v1/repos/{owner}/{repo}/pulls` | Open the review for a change: on OBS-native this files an SR; on git-backed it opens a PR. `--source owner/repo:branch`, `--target owner/repo:branch` (the `:branch` form resolves the repo from the checkout's remote — see decision 5) |
| `pr_get(owner, repo, number)` | GET `/api/v1/repos/{owner}/{repo}/pulls/{n}` | Read one PR: state, draft, merged, mergeable, allow-maintainer-edit, author, source/target (repo, branch, commit), labels |
| `pr_list(owner, repo)` / `pr_search(query)` | GET `.../pulls`, GET `/api/v1/repos/issues/search?type=pulls` | Triage: what is open on a repo; find a PR by state/author |
| `pr_merge(owner, repo, number, options)` / `pr_close` / `pr_reopen` | POST `.../pulls/{n}/merge`, PATCH `.../issues/{n}` | Land or close the review |
| `pr_reviews(owner, repo, number)` / `pr_review(owner, repo, number, event, body)` | GET/POST `.../pulls/{n}/reviews` | The review half of the loop |
| `pr_comments(owner, repo, number)` / `pr_comment_add` | GET/POST `.../issues/{n}/comments` | Comment on the review |
| `pr_review_cancel_request(owner, repo, number, reviewer \| --all [--exclude …])` | DELETE `.../pulls/{n}/requested_reviewers` | Cancel review requests (single, all, all-but-excluded); repeat cancel is a client-side error, `--dry-run` reports the plan |
| `pr_labels_add/remove` | POST/DELETE `.../issues/{n}/labels` | Label the review (incl. the `staging/*` labels the staging flow uses) |

Defaults: `pr_create` defaults the target
branch to the target repo's default branch (resolved via
`repo_get`) and the target repo to the fork's parent; an empty PR
(identical source/target) is refused unless `--allow-empty`. One target-branch form only.

### Phase 3 — unified pull requests: the `pull-request` surface

One interface for pull requests on either backend: an OBS
submit request and a Gitea pull request are the same object to the
agent — a pull request with a source, a target, a state, reviews, and
comments. The agent names a pull request by a backend-neutral handle
and reads or acts on it backend-agnostically. The `request`
group stays the OBS filing and backend surface; the `gitea` group
stays the Gitea repo/branch surface; `pull-request` is the uniform surface
over both.

#### The handle: `PullRequestRef` (decision 7 — settled)

A pull request is named by one typed handle. The backend kind is the
variant; the id is the payload. An OBS request id is globally unique;
a Gitea PR number is unique per repo, so the Gitea variant carries
the repo with it. The handle is host-free: the configured
instance supplies the host (decision 5 — a single configured instance per
client).

```rust
pub enum PullRequestRef {
    /// An OBS submit request, by its number.
    Obs(u64),
    /// A Gitea pull request, by repo and number on the configured instance.
    Gitea { owner: String, repo: String, number: u64 },
}
```

Text form, one form only: `obs:<id>` and
`gitea:<owner>/<repo>#<number>`. `Display` prints that form;
`FromStr` parses it and refuses anything else with a structured
error. The text renderer echoes the handle as the coordinate header
of every `pull-request` command. The handle lives in a new module
`src/pull_request.rs`, with the uniform operations on a `PullRequestClient`
holding `&ObsClient` and `&GiteaClient`.

#### The read shape: `PullRequest`

`show` returns one shape on both backends. Fields a backend omits default to empty/false/`None`, so the JSON keys are the
same on every read.

`reference: PullRequestRef` — the handle.
`title: String`, `description: String` — OBS description / Gitea body; empty by default.
`author: String` — OBS creator / Gitea author login.
`status: PullRequestStatus` — the unified state (mapping below).
`draft: bool` — Gitea draft flag; false on OBS.
`merged: bool` — Gitea merged flag; OBS accepted reads as merged.
`mergeable: Option<bool>` — Gitea mergeable flag; `None` on OBS.
`source`, `target: PullRequestEndpoint` — `repository` (OBS `project/package`; Gitea `owner/repo`), `branch: Option<String>` (Gitea branch; `None` on OBS), `commit: Option<String>` (Gitea head/base SHA; OBS source revision when the action carries one).
`reviews: Vec<ReviewOpinion>` — one entry per review asked or given, in order.
`labels: Vec<String>` — Gitea label names; empty on OBS.

`search` returns `PullRequestSummary`: `reference`, `title`, `author`,
`status`, `source`, `target`. The list shape is the triage shape;
detail is a `show` call away.

`PullRequestStatus` is the union of both backends' states:

| `PullRequestStatus` | OBS `RequestState` | Gitea PR state |
|---|---|---|
| `Open` | `New`, `Review` | `open` |
| `Merged` | `Accepted` | closed and `merged` |
| `Declined` | `Declined` | — |
| `Withdrawn` | `Revoked` | — |
| `Closed` | — | closed, not merged |
| `Superseded` | `Superseded` | — |
| `Deleted` | `Deleted` | — |

A Gitea close reads as `Closed`. Decline and withdrawal are separate operations with separate OBS mappings; the Gitea side records neither outcome.

`ReviewOpinion` is one review on the pull request: `reviewer: String`
(the asked-of target on OBS — user, group, project, or package;
the reviewer login on Gitea), `state: OpinionState`,
`comment: String`, `when: Option<String>`. `OpinionState` is the
union of the per-review states:

| `OpinionState` | OBS `ReviewState` | Gitea review state |
|---|---|---|
| `Asked` | `New` | `PENDING` |
| `Approved` | `Accepted` | `APPROVED` |
| `ChangesRequested` | `Declined` | `REQUEST_CHANGES` |
| `Commented` | — | `COMMENT` |
| `Dismissed` | — | `DISMISSED` |
| `Superseded` | `Superseded` | — |
| `Obsolete` | `Obsoleted` | — |

`ReviewComment` is one comment: `id: u64`, `author: String`,
`created: String`, `body: String`, `parent_id: Option<u64>` (OBS threading; `None` on Gitea).

#### The act surface

| Uniform operation | OBS | Gitea |
|---|---|---|
| `show(ref)` | `show_request` (+ reviews) | `pr_get` + `pr_reviews` |
| `search(filters)` | `search_requests` | `pr_list` (repo scope) / `pr_search` (global) |
| `approve(ref, comment, target?)` | `change_review_state(accepted, by_*)` | `POST .../pulls/{n}/reviews` `{event: APPROVE, body}` |
| `request_changes(ref, comment, target?)` | `change_review_state(declined, by_*)` | `POST .../pulls/{n}/reviews` `{event: REQUEST_CHANGES, body}` |
| `comment(ref, body, parent?)` | `add_comment` | `POST .../issues/{n}/comments` `{body}` |
| `comments(ref)` | `list_comments` | `GET .../issues/{n}/comments` |
| `merge(ref, comment, method?, force?)` | `accept_request` | `POST .../pulls/{n}/merge` `{Do}` |
| `decline(ref, comment)` | `decline_request` | `PATCH .../issues/{n}` `{state: closed}` |
| `withdraw(ref, comment)` | `revoke_request` | `PATCH .../issues/{n}` `{state: closed}` |
| `reopen(ref)` | `change_state(new)` | `PATCH .../issues/{n}` `{state: open}` |
| `cancel_review_request(ref, reviewer \| --all [--exclude])` | `change_review_state` — verify against the server (see below) | `DELETE .../pulls/{n}/requested_reviewers` `{reviewers: [...]}` |
| `labels_add/remove` (library only, no uniform CLI) | — (no OBS counterpart) | `POST .../issues/{n}/labels`, `DELETE .../issues/{n}/labels/{id}` |

`change_review_state` is the typed review-state operation (`newstate` + `by_user`/`by_group`/`by_project`/`by_package` + comment), over `POST /request/{id}?cmd=changereviewstate&newstate=…&by_user=…` with the comment as the POST body.
`approve` and `request_changes` act on the review asked of the
acting user by default — the credentials' username — and take the
same `--user`/`--group`/`--project`/`--package` target flags as
`request add-review` for a review asked of a group, project, or
package. The target flags are refused on a Gitea ref: a Gitea
review is by the token owner.

`merge` takes `--method` (`merge` | `rebase` | `rebase-merge` |
`squash` | `fast-forward-only`, sent as the Gitea `Do` field; unset
sends `merge`, the server default style — Gitea requires `Do`, an
empty body answers 422) and `--force`
(OBS: accept overriding open reviews, as on `request accept`). A
backend-only option on the other backend's ref fails closed with a
structured error naming the option.
`decline` and `withdraw` are separate operations because their OBS
sides differ (decline refuses the pull request, revoke withdraws the
agent's own); both close the PR on Gitea. `reopen` moves an OBS
request back to `new`.

`cancel_review_request` on OBS fails closed with a structured error naming the gap, and stays Gitea-only in practice: `changereviewstate` takes `new`, `accepted`, and `declined` only (the server raises InvalidStateError, rendered as 404), and a review reaches `obsoleted` only when its reviewer is removed from the project or package.
Repeat cancel is a client-side error on Gitea: the server answers
204 even for a reviewer with no pending request, so the check runs
against the pending set before the DELETE.

#### Live probe findings (review-level act)

Probed against a disposable Gitea 1.24.7 instance:

A team review request reads as `"user": null` with a `team` object (`id`, `name`, ...) in `GET .../pulls/{n}/reviews`: the review's user is optional, and a team review reads as `@<team name>` in the unified opinion. Team review requests are org-repo-only: only teams with repo access produce a pending entry, and adding a team to a user-owned repo answers 405.
A repeat DELETE of `requested_reviewers` answers 204: the repeat-cancel error is client-side, against the pending set.
Minting a token for another user through the admin token answers 401 on `POST /api/v1/users/{name}/tokens`: a second user's token is minted through the user's own Basic auth.
Posting an issue comment needs `write:issue`: a token without it gets 403 naming the missing scope.

#### Live probe findings (pull-request-level act)

Probed against a disposable Gitea 1.24.7 instance, and the OBS
transition against the server source:

Merging with an empty body answers 422 `[Do]: Required`: `Do` is required, so an unset `--method` sends `merge`, the server default style.
The provisioned Gitea token carries `read:user`, `read:repository`, `read:issue`, `write:repository`, `write:user`, `write:organization`, `write:admin`: closing or reopening as it answers 403, so the close/reopen live tests act as a collaborator with a minted token carrying `write:issue`.
A declined OBS request reopens to `new`: the creator and the decliner can reopen it (server source, `BsRequestPermissionCheck`), and the live test pins the creator path.

Labels are Gitea-only and library-only: the operations land in `GiteaClient` as primitives the staging flow uses later. The read shape carries
label names; `labels_remove` takes a name and resolves its id from
the PR's current labels, failing closed when the PR carries no such
label.

#### `submit`

The library `submit` files a pull request and returns the handle:

```rust
pub enum SubmitTarget {
    Obs(SrPayload),
    Gitea(PrCreateOptions),
}
```

The OBS arm files through `SrFiling::file` and returns
`PullRequestRef::Obs(id)`; the Gitea arm opens the PR through
`GiteaClient::pr_create` (`POST /repos/{owner}/{repo}/pulls`
`{head, base, title, body}`) and returns
`PullRequestRef::Gitea { owner, repo, number }`. The caller picks the
variant for the backend it files to — the backend kind is known at
the call site from the checkout — and reads the result backend-agnostically. `pr_create` defaults the target branch to the
target repo's default branch (resolved via `repo_get`) and the
target repo to the fork's parent; an empty PR is refused unless
`--allow-empty`. The CLI `submit` verb and the `branch` composite land with the working-copy layer: they read the backend kind from the checkout.

The Gitea-side devel rule: a pull request goes to the immediate
parent repo — for a fork of the devel repo, the devel repo —
never into the fork, by default on the branch the checkout tracks
(its scmsync branch), or the target's default branch when the
checkout tracks none, unless `--base` names another branch. The
`factory` branch takes no pull requests; the Factory leg is an OBS
submit request.

#### The CLI surface: the `pull-request` group

One group, one subcommand per uniform operation, the handle as the
positional. The group follows the CLI scheme: kebab-case names,
long flags only, one name per command, one global `--format json|text`, every
mutation dry-run by default with the global `--live`, `--comment`
for messages attached to an act, `--body` for a comment's own text.

| Subcommand | Agent use |
|---|---|
| `pull-request show <REF>` | Read one pull request: state, source/target, reviews, labels |
| `pull-request search [--backend obs\|gitea] [--state …] [--creator …] [--repo …]` | Triage: what is open, by state or author, on either backend or both |
| `pull-request approve <REF> [--comment …] [--user\|--group\|--project\|--package …]` | Approve the review: the pull request is good to land |
| `pull-request request-changes <REF> [--comment …] [--user\|--group\|--project\|--package …]` | Ask for changes on the review |
| `pull-request comment <REF> --body … [--parent …]` | Comment on the pull request; `--parent` threads the reply on OBS and is refused on a Gitea ref |
| `pull-request comments <REF>` | List the comments on a pull request, in id order |
| `pull-request merge <REF> [--comment …] [--method …] [--force]` | Land the pull request |
| `pull-request decline <REF> [--comment …]` | Refuse the pull request |
| `pull-request withdraw <REF> [--comment …]` | Withdraw the agent's own pull request |
| `pull-request reopen <REF>` | Reopen a closed pull request |
| `pull-request cancel-review-request <REF> --reviewer … \| --all [--exclude …]` | Cancel review requests (single, all, all-but-excluded); repeat cancel is a client-side error, `--dry-run` reports the plan |

`pull-request search` with no `--backend` runs both arms — OBS first,
then Gitea — and returns both sets under the one list shape. With
no `--state`, each arm applies its own default (`request search`
defaults new/review/declined on OBS; open on Gitea). `--state`
takes `PullRequestStatus` names: OBS maps `Open` to new/review,
`Merged` to accepted, `Declined` to declined, `Withdrawn` to
revoked, `Superseded` to superseded, `Deleted` to deleted, `Closed`
to declined; Gitea maps `Open` to open and every other state to
closed. `--creator` filters by author; on the Gitea arm the filter
runs client-side on the returned PRs. `--repo` scopes the Gitea arm
to one repo (`pr_list`); otherwise the Gitea arm uses the global
search (`GET /api/v1/repos/issues/search?type=pulls`).

The `request` group keeps its read/act leaves as the OBS backend
spelling, and the `gitea` group keeps repo/branch as the Gitea
backend spelling. The uniform group is the CLI surface for PR read/act. The Gitea operations live in `GiteaClient` as the
library layer the uniform operations dispatch to (`pr_get`,
`pr_list`, `pr_search`, `pr_create`, `pr_merge`, `pr_close`,
`pr_reopen`, `pr_reviews`, `pr_review` (event POST), `pr_comments`,
`pr_comment_add`, `pr_review_cancel_request`, `pr_labels_add`,
`pr_labels_remove`), with the Gitea PR/review/comment shapes
(`GiteaPullRequest`, `GiteaReview`, `GiteaComment`,
`GiteaReviewEvent`, `GiteaMergeMethod`, `GiteaLabel`) modelled the
same way as `GiteaRepository` and `GiteaBranch` — only the fields
each operation needs, unknown fields ignored.

#### The CLI surface: the `person` group

One group, one subcommand for the uniform operation. The group
follows the CLI scheme: kebab-case names, long flags only, no
aliases, one global `--format json|text`.

| Subcommand | Agent use |
|---|---|
| `person search <QUERY> [--backend obs\|gitea]` | Find a person: the OBS projects naming the login, and the Gitea users matching the keyword, on either backend or both |

`person search` with no `--backend` runs both arms — OBS first,
then Gitea — and returns both lists under the one payload. The
OBS arm matches the login exactly (`search_projects_by_person`);
the Gitea arm matches the keyword against login, full name, and
email (`user_search`). A scoped search on an arm whose
credentials are not configured fails closed naming them; a
both-arms search fails on the first missing credential — at
client construction for the Gitea token, at the OBS arm for
OBS credentials — before any HTTP in either case.

#### Implementation order

Each step is its own PR.

1. `change_review_state` in `src/obs/request.rs`: the typed
   review-state operation, adopting the raw harness call; the live
   harness switches to it. The review-history readback test pins the parse.
2. `PullRequestRef` and the unified types in `src/pull_request.rs`:
   `PullRequestRef`, `PullRequestStatus`, `PullRequest`, `PullRequestSummary`,
   `ReviewOpinion`, `OpinionState`, `ReviewComment`,
   `PullRequestEndpoint`; `Display`/`FromStr` unit
   tests. (`SubmitTarget` lands with the working-copy layer, with `submit`.)
3. Read surface: `pr_get`/`pr_list`/`pr_search` in `src/gitea.rs`,
   `PullRequestClient::show`/`search`, `pull-request show`/`pull-request search`
   with JSON/text renderers and agreement tests. Test borrowing:
   `git-pr.feature` get/list/search, `git-pr-fork-to-fork`.
4. Review-level act: `pr_reviews`, the review-event POST,
   `pr_comments`/`pr_comment_add`, `PullRequestClient::approve`/
   `request_changes`/`comment`/`comments`,
   `pull-request approve`/`request-changes`/`comment`/`comments`, plus
   `pr_review_cancel_request` and `pull-request cancel-review-request`
   with the OBS probe. Test borrowing:
   `git-pr-review-request.feature`.
5. Pull-request-level act: `pr_merge`/`pr_close`/`pr_reopen`,
   `PullRequestClient::merge`/`decline`/`withdraw`/`reopen`,
   `pull-request merge`/`decline`/`withdraw`/`reopen`.
   Unset `--method` sends `Do: merge`, the server default style:
   Gitea 1.24.7 requires `Do` — an empty body answers 422
   `[Do]: Required`. Gitea PR states are open and closed; merged is a flag:
   decline and withdraw both close, posting the reason as an
   issue comment first. Reopening a declined OBS request: the creator and the decliner can reopen it. Test
   borrowing: none — osc carries zero unit tests for these acts, and
   the behave close/reopen/merge steps sit in a commented-out
   timeline scenario broken by a Gitea issue.
6. Labels: `pr_labels_add`/`pr_labels_remove` (library only).
   Test borrowing: none — osc lacks a label operation.
   Filing lands with the working-copy layer:
   `pr_create` and the library `submit` returning
   `PullRequestRef`, with the CLI `submit` verb. Test borrowing
   for filing: `git-pr.feature` create, `git-pr-empty.feature`.

The provisioned live token (`provision.sh`) carries `write:repository` for the write operations (review events, cancel); issue comments and labels need `write:issue`, so the review-act live tests mint a second token with `write:issue` for the comment path. The scopes are recorded per step.

### Phase 4 — branches (write)

| Operation | Endpoint | Agent use case |
|---|---|---|
| `branch_create(owner, repo, new_branch, old_branch)` | POST `/api/v1/repos/{owner}/{repo}/branches` | Create the branch a change will land on (fork `--git-branch` needs it for a new branch) |

### Phase 5 — user, SSH keys

| Operation | Endpoint | Agent use case |
|---|---|---|
| `user_get(name?)` | GET `/api/v1/user`, GET `/api/v1/users/{name}` | Who the token belongs to (verify before acting; the login auto-select resolves from this) |
| `ssh_key_list` / `ssh_key_add(key \| key_path)` / `ssh_key_remove(id)` | GET/POST/DELETE `/api/v1/user/keys[/{id}]` | The key the agent's `git clone` over SSH needs must exist on the account; invalid keys are refused client-side before the call, a key already used by any account is the server's 422 |

## Typed API shape

A new module `src/gitea.rs` with a `GiteaClient` struct, parallel to `ObsClient`. The Gitea surface is JSON over a different base URL with different auth (Bearer vs OBS Basic) and different wire rules (JSON where OBS uses XML). The client is internal plumbing (§ "Uniform flow"). The public surface is the uniform operations,
which dispatch to `ObsClient` or `GiteaClient` internally on the
recorded backend kind. `GiteaClient` is public in `pub mod gitea`
(exported from `pub mod gitea` only) because the live harness
(`tests/gitea.rs`) can only reach the typed operations through the
public API; uniform-flow signatures carry plain fields only
(`branch` takes `&ObsClient` / `&ForkOptions` and returns
plain-field `ForkResult`). The client is built from the same
`Config` (gitea_url) and `AuthProvider` (Bearer) as everything
else; construction fails closed when no token is configured.

JSON via `serde` `Deserialize`, unknown fields ignored — the structs tolerate a newer Gitea than the pinned test image (1.24.7, `tests/gitea/provision.sh`), keeping the harness working. Only the fields each operation needs are modelled.
Public signatures carry typed values only. IDs are typed (`owner`/`repo` as validated names, PR numbers as `u64`).
The `HttpClient` trait is reused as-is (`get`, `post_json`, `delete`); transport is the `HttpClient` trait only. The 10 MiB read cap applies the same way; a Gitea response over it is the same structured error as on the OBS side.
DryRun follows the library convention: the guard runs before the credentials check; a dry run reports the plan (calls that would be made, names that would be created) and leaves server and local state unchanged. The guard runs before any server read, so a dry run resolves neither the devel project nor the branch and reports placeholders instead.
New error kind (central store, `src/error.rs`): `AlreadyExists` (`already_exists`) — ForkExists / a repo of the requested name already existing. It is a distinct, actionable state (reuse the fork, pick another name). `commit` on a git-backed checkout (push + poll) belongs to the working-copy layer (#17).
Redirects: Gitea answers 301s on several operations (osc's `test_gitea_exceptions.py` maps one per operation). The client follows redirects on the same host only, so the token stays on the configured host. Redirect handling reuses the main-side redirect re-validation.

## scmsync lag

Filing an SR from a git-backed package right after pushing: the git→OBS sync is still propagating. The handling lives in the filing path (#42): before filing from a git-backed package, verify the source revision is current — compare the pushed head SHA (`branch_head`, phase 2) against the OBS side (`_scmsync.obsinfo` commit / directory rev). The Gitea half of that comparison is `branch_head` + the `_scmsync.obsinfo` parse rules, adopted from `test_scmsync_obsinfo.py`. Freshness enforcement is decision 3. Fork creates the fork only; filing is a separate step.

## The #17 dispatch point (git-backed commit)

The working-copy design records the backend kind (`obs` | `git`)
in git config `geectl.*` at checkout. On a git-backed checkout,
`commit` pushes with the git binary through the working-copy `Git`
wrapper (argv-only), takes the pushed head SHA, and polls the OBS
side (`_scmsync.obsinfo` `commit:`) until the synced commit matches,
bounded 60 s, then reports. What the dispatch needs:

`branch_head` (phase 2): the SHA to compare against.
`repo_get` (phase 1): the fork's parent and default branch, for the refusal message ("push to the fork, then sync").
The `_scmsync.obsinfo` parse rules (adopted from `test_scmsync_obsinfo.py`): the `commit:` field the poll compares against.

The push itself is the agent's `git push` through the `Git` wrapper,
and the sync wait is the 60 s bounded poll. This dispatch is the
`commit` step of the uniform flow (§ "Uniform flow").

## Debug channel (#51)

The debug channel reports HTTP attempts, retries, and endpoint
selection through the one `--debug` channel (`--debug` flag +
`GEECTL_DEBUG=1`, human lines on stderr, secrets and bodies excluded).
The Gitea client uses the same `HttpClient` and the same `Debug` sink as the OBS client, so Gitea attempts are reported the same way. One debug format covers both clients.

## Test borrowing

Adopt the osc cases that pin the behaviour built here; write the rest from scratch.

### Behave — adopt as live Gitea tests (disposable 1.24.7 instance)

The live suite is `tests/gitea.rs`: every adopted scenario asserts on server state through the typed operations, with unique repo names per test.

| osc scenarios | Geectl adoption |
|---|---|
| `git-repo-fork.feature`: fork; fork twice under different names (ForkExists → reuse + warning); fork from pool + fork someone else's fork (202 quirk) | Adopt all. The 202 case asserts the re-read parent check beyond the 202. |
| `fork.feature`: `osc fork` incl. cross-org re-fork refused (exit 1) and `--new-repo-name` retry | Adopt both as the `fork_repo` composite (Gitea fork + OBS `cmd=fork`); the cross-org refusal is the RepoExists error case. |
| `git-repo-list.feature`: `--org`, `--user`, both, `--export` JSON | Adopt the list forms; `--export` is CLI rendering (the #52 renderer). |
| `git-ssh-key.feature`: add (inline + `--key-path`), invalid key refused client-side, invalid base64 → server 422, remove by id | Adopt all. |
| `git-pr.feature`: create (default, `--source`/`--target`, `:branch` form, deprecated-options warning, mixed-options error), list (text + `--export`), search (text + `--export`), get (+ labels, + nonexistent → exit 1), checkout, checkout-as-other-user + push, rebase ff / non-ff | Adopt create/list/search/get/labels/empty (see below); checkout + rebase: local git work, git CLI; deprecated-options warning and mixed-options error: one form only; commented-out timeline scenario: broken upstream (gitea#35152). |
| `git-pr-empty.feature`: identical commits refused; `--allow-empty` after revert | Adopt both. |
| `git-pr-fork-to-fork.feature`: fork-to-fork PR | Adopt. |
| `git-pr-review-request.feature`: setup check, cancel single (+ `--dry-run`, repeat → exit 1), `--all`, `--all --exclude` (lowercase + mixed case + unknown) | Adopt all. |
| `git-pr-new-package-request.feature` | OUT of v1 (see below). |
| `git-obs-staging-group.feature` / `git-obs-staging-remove.feature` | OUT of v1 (see below). |
| `git-repo-clone.feature` | Clone: git CLI. HTTP-clone failure case: adopted as a test pinning auth behaviour; the credential-helper setup is git's. |
| `git-login.feature` | Out of scope — geectl auth is env/mounted secrets only. |
| `git-osc-precommit-hook.feature` | Out of scope — the hook runs local checks; the checks belong to the services (#19) and local-build (#43) work. |
| `git-repo-init-local.feature` / `git-repo-init-remote.feature` | Out of scope as operations — the template (`.gitignore` / `.gitattributes` / `bs_mergechanges` driver) lives in the packaging skill / local git config. The remote-template read is `repo_get`, phase 1. |

### Unit — adopt

`test_gitea_api_pr.py`: PR/issue object shapes → serde round-trip tests; URL parsing (`split_id`, `get_host_owner_repo_number`, https ± port, issues endpoint, invalid endpoint/format) → unit tests on the ID parsing; PR-reference parse/add/remove → adopted with PR-description reference editing (staging flow); the pure helpers stay unported otherwise.
`test_gitea_api_branch.py`, `test_gitea_api_repo.py`, `test_gitea_api_user.py`, `test_gitea_api_ssh_key.py`: object shapes + `split_id` → serde + ID-parsing unit tests.
`test_gitea_api_git.py`: `urlparse`/`urljoin` → adopt (the scmsync-URL parsing in `fork_repo` uses them); git detection (normal/subdir/inside `.git`/bare) → out of scope (geectl works through the API only).
`test_gitea_api_common.py`: `dt_sanitize` → adopt if timestamps are parsed into typed values; `TemporaryDirectory` → out of scope.
`test_gitea_api_conf.py`: XDG config-home resolution → out of scope; geectl's config resolution covers it.
`test_gitea_api_git_diff_generator.py` + `test_gitea_api_tardiff.py`: out of scope — diff rendering is JSON data through the one renderer (same ruling as `test_difffiles` in #17).
`test_git_scm_store.py`: out of scope under API-only #17 — the `.osc` git store is outside the API-only format. The `scmurl` parsing informs the `wc.json` backend detection.
`test_scmsync_obsinfo.py`: adopt the parse rules (empty/mandatory/all fields) — the sync-verification primitive needs them.
`test_gitea_exceptions.py`: adopt the mapping table as unit tests — 301 per operation, 404 → `NotFound`, 401/403 → `AuthFailed`, 422 → `InvalidInput` with the server message.
`test_maintainership.py`: adopted by #207 — the git maintainership read (`gitea maintainers`) parses the `obs-maintainers` document (v1.0 + legacy) via the Gitea raw endpoint. The converter stays out of scope: the migrate/write command has no agent use case.
Config/credentials (`test_conf.py`, `test_config_parser.py`, `test_credentials.py`): Out of scope — geectl's config/auth cover the single-instance env story; the multi-login model is decision 5.

### Gap — no osc coverage, written from scratch

The `fork_repo` composite itself (Gitea fork + OBS `cmd=fork` + scmsync URL construction + parent re-read after a 202): no osc test pins the composition, only its halves.
`repo_delete` and `repo_create` as typed operations (the harness does them raw today).
The `branch_head` sync-verification comparison and the `_scmsync.obsinfo` read-back on the OBS side.
Pagination bounds on list operations.

### Live-test setup notes

`tests/gitea/provision.sh` creates the token with the scopes the live tests need — the write operations (fork, repo create/delete, PR create/merge, ssh-key add) take token scopes extended per phase as those phases land; the scopes are recorded per phase.
The instance stays pinned at Gitea 1.24.7; a version bump is a deliberate change, with the live suite proving the new version still accepts the client.

## Sequencing and dependencies

`src/gitea.rs` sits beside `src/obs/`. No dependency on #33 (build status is OBS-side) or #51 beyond the debug-channel note above.
After #17's decisions: the commit dispatch needs decision 3 settled; the dispatch itself is a follow-up after phase 2.
Services (#19) is designed in parallel; the two are independent. The gap-map order (services 8, Gitea 9) is landing order.
The working-copy layer (#17) covers both backends: OBS-native commit through `commitfilelist`, git-backed commit through push + poll (see the #17 dispatch point).

## Out of scope (v1)

**Local git operations** (clone, fetch, checkout, push, rebase, `pr checkout`): the git CLI does them (#17). The fork result returns the URLs; the agent clones.
**Staging group / staging remove**: orchestration over PRs + labels + local git (submodules, merge). A project workflow.
**`pr new-package-request`**: creates a Gitea issue with a label for a new package. A project-process operation.
**PR timeline**: display-only, broken upstream (gitea#35152).
**Login entries / OAuth2** (osc #2042): geectl auth is env/mounted secrets. Multi-login entries are decision 5.
**SSH key PIN prompting** (osc #1935): geectl takes input from flags and env only — missing input fails fast. The PIN belongs to git when it uses the key.
**`repo init` templates, diff generators**: local config / repo-file conventions / rendering (see test borrowing). Maintainership read is a library operation (`gitea maintainers`, #207); the migrate/write side stays out.

## Open decisions

1. **Fork scope: Gitea fork + OBS `cmd=fork` only, or also the
   local clone?** API-only: no local clone. The clone
   is a git-CLI step for the agent (the fork result returns the
   URLs), and #17 forbids shelling out to git. The osc clone
   exists so a human can review before pushing; an agent reviews
   through the working-copy layer.
2. **Client shape: separate `GiteaClient` in `src/gitea.rs`, or
   methods on `ObsClient`?** A separate module and client. Different base URL, auth, and wire format (JSON vs
   XML); and the OBS client is being split precisely because it
   is too big to grow a second service.
3. **scmsync lag on git-backed commit: refuse, or poll until
   synced?** Poll-with-bound: the #17 commit dispatch
   takes the pushed head SHA and polls the OBS side until the
   synced commit matches (bounded, 60 s like the accept retry),
   then reports.
4. **PR scope in v1: read/act only, or staging + new-package-
   request too?** Read/act only (phase 3 as listed). Staging group/remove and `new-package-request` are project workflows.
5. **Auth: single configured Gitea instance, or osc-style
   multi-login entries?** A single instance per client (config + env token); a repo URL on another host fails with `ConfigError` naming the configured host. Multi-login (osc #2051 auto-select included) is a later decision.
6. **First PR: fork + repo delete (the #20 scope) alone, or with
   the repo read it needs?** One first PR carrying `repo_get` + `fork_repo` + `repo_delete`: `repo_get` is its verification primitive (default branch, parent re-read). `repo_list` follows in phase 2.
7. **Uniform `submit`: what does it return?** Settled: a
   backend-neutral pull-request reference, `PullRequestRef` — `Obs(u64)` for
   an OBS request id, `Gitea { owner, repo, number }` for a Gitea
   PR number. Text form `obs:<id>` /
   `gitea:<owner>/<repo>#<number>`. See the Phase 3 extension
   ("The handle: `PullRequestRef`").
8. **Sync freshness on `update` for git-backed checkouts: check in
   `update`, or enforce at `commit`/`submit`?** Enforce at `commit`/`submit` only: `update` stays a pure read on the identical code path (#17), and staleness is caught by the commit dispatch's sync poll (decision 3) and the filing-path freshness check (scmsync lag, #42).
9. **Maintenance revisions on git-backed packages: OBS incident
   from synced sources, or Gitea PR to the maintenance branch?**
   The OBS incident: the incident machinery (maintenance project, incident branching, release) is entirely OBS-side; the git repo only supplies sources via push + sync, so the uniform verb "maintenance revision" maps to the same incident flow on both backends.
