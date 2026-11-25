# Gitea operations

Tracking: https://github.com/scarabeusiv/geectl/issues/20

The Gitea surface is the git-backed half of the uniform flow: one set of
verbs across both backends, with the backend kind recorded at checkout
selecting the implementation underneath. The `GiteaClient` in `src/gitea.rs`
is the transport client for the git-backed implementation. The
`pull-request` group is the uniform surface over both backends, dispatching
to `ObsClient` or `GiteaClient` on the recorded backend kind; the `gitea`
group exposes the `GiteaClient` operations directly, including `repo-fork`.
Local git work runs through the git CLI.

## The uniform flow

| Verb | OBS-native backend | Git-backed backend |
|---|---|---|
| `update` | Reads the latest revision through the OBS source API | Fetches the upstream git branch and rebases the working copy onto it, preserving local commits; a rebase that conflicts reports the conflicts |
| `status` / `diff` | The working copy read against its base | The working copy read against its base |
| `commit` | `commit_filelist` with staged PUTs, exactly one revision | Push with the git binary, then a bounded poll of the OBS side until the synced commit matches the pushed head |
| `submit` | An OBS submit request, filed through `SrFiling` | A Gitea pull request, opened through `pr_create` |
| `maintenance maintained-instances` / `maintenance maintenance-branch` | The OBS maintenance flow, addressed by project and package | The OBS maintenance flow, addressed by project and package |

The working copy records the backend kind (`obs` or `git`) in git config
`geectl.*` at checkout. The commit poll waits on the `_scmsync.obsinfo`
`commit:` field and runs bounded at 60 seconds.

## The CLI surface

### The `pull-request` group

One group, one subcommand per uniform operation, the handle as the
positional. Kebab-case names, long flags only, one name per command, one
global `--format json|text`. Every mutation dry-runs by default; `--live`
performs it. `--comment` carries a message attached to an act; `--body`
carries a comment's own text.

| Subcommand | Agent use |
|---|---|
| `pull-request show <REF>` | Read one pull request: state, source and target, reviews, labels |
| `pull-request search [--backend obs\|gitea] [--state …] [--creator …] [--repo …]` | Triage: what is open, by state or author, on either backend or both |
| `pull-request approve <REF> [--comment …] [--user\|--group\|--project\|--package …]` | Approve the review |
| `pull-request request-changes <REF> [--comment …] [--user\|--group\|--project\|--package …]` | Ask for changes on the review |
| `pull-request comment <REF> --body … [--parent …]` | Comment on the pull request; `--parent` threads the reply on OBS |
| `pull-request comments <REF>` | List the comments on a pull request, in id order |
| `pull-request merge <REF> [--comment …] [--method …] [--force]` | Land the pull request |
| `pull-request decline <REF> [--comment …]` | Refuse the pull request |
| `pull-request withdraw <REF> [--comment …]` | Withdraw the agent's own pull request |
| `pull-request reopen <REF>` | Reopen a closed pull request |
| `pull-request cancel-review-request <REF> --reviewer … \| --all [--exclude …]` | Cancel review requests: single, all, all-but-excluded |
| `pull-request submit <ARGS>` | File a pull request on either backend and return the handle |

`pull-request search` runs both arms by default, OBS first then
Gitea, and returns both sets under the one list shape. Each arm applies
its own default state: new, review, and declined on OBS; open
on Gitea. `--state` takes `PullRequestStatus` names. `--creator` filters by
author; on the Gitea arm the filter runs client-side on the returned pull
requests. `--repo` scopes the Gitea arm to one repo; otherwise the Gitea
arm uses the global search.

The `request` group carries the OBS backend spelling of read and act; the
`gitea` group carries the Gitea backend spelling of repo and branch. The
`pull-request` group is the uniform surface over both.

### The `person` group

| Subcommand | Agent use |
|---|---|
| `person search <QUERY> [--backend obs\|gitea]` | Find a person: the OBS projects naming the login, and the Gitea users matching the keyword, on either backend or both |

`person search` runs both arms by default, OBS first then Gitea,
and returns both lists under the one payload. The OBS arm matches the login
exactly; the Gitea arm matches the keyword against login, full name, and
email. A search on an arm whose credentials are missing raises the missing-arm
error from the search itself, naming the missing token.

### The `gitea` group

Thin over `GiteaClient`: the flags mirror the library parameters, the JSON
payload carries the typed values, and the text renderer prints the same
values in human-readable form. Every text rendering opens with one header
line echoing the query coordinates, except `branch-head`, whose text is the
bare SHA.

| Subcommand | Agent use |
|---|---|
| `gitea repo-get <OWNER/REPO>` | Read one repo: default branch, parent, clone URLs |
| `gitea repo-list [--owner …] [--org …]` | List repos: a user's, an org's, or the token owner's |
| `gitea repo-search <QUERY>` | Search repos by keyword: the keyword matches the name and the description |
| `gitea repo-create <NAME> [--private] [--description …] [--default-branch …]` | Create a repo |
| `gitea repo-delete <OWNER/REPO>` | Delete a repo; a dry run reports the plan, a live delete needs `--force` to confirm and then reads the repo to confirm owner and name |
| `gitea repo-fork <OWNER/REPO> [--organization …] [--new-name …]` | Fork a repo |
| `gitea branch-list <OWNER/REPO>` | List the branches a repo carries |
| `gitea branch-get <OWNER/REPO> <BRANCH>` | Read one branch |
| `gitea branch-head <OWNER/REPO> <BRANCH>` | The head commit SHA of a branch |
| `gitea maintainers <OWNER/REPO>` | The maintainers recorded in the repo's `_maintainership.json` |

`repo-list` text carries identifiers; the clone URLs and the fork parent
are in the JSON payload. `branch-list` text carries the full head SHA; the
commit message is in the JSON payload. `repo-create` text carries
the created name; the description is in the JSON payload.

### The `branch` group

`branch branch-package` branches an OBS-native package: a linked copy
under a branch project, with `--target-project`, `--target-package`,
`--no-devel-project`, `--new-package`, `--force`, `--revision`, and
`--linkrev`. The group also carries the link operations
`repair-link`, `set-link-rev`, `set-link-rev-project`,
`link-to-branch`, and `detach-branch`. The git-backed fork composite is a
library operation, exposed as `branch create`.

## The pull-request handle

A pull request is named by one typed handle. The backend kind is the
variant; the id is the payload. An OBS request id is globally unique; a
Gitea pull request number is unique per repo, so the Gitea variant carries
the repo with it. The handle is host-free: the configured instance
supplies the host.

```rust
pub enum PullRequestRef {
    /// An OBS submit request, by its number.
    Obs(u64),
    /// A Gitea pull request, by repo and number on the configured instance.
    Gitea { owner: String, repo: String, number: u64 },
}
```

Text form, one form only: `obs:<id>` and `gitea:<owner>/<repo>#<number>`.
`Display` prints that form; `FromStr` parses it and returns a structured
error for anything else. The text renderer echoes the handle as the
coordinate header of every `pull-request` command. The handle lives in
`src/pull_request.rs`, with the uniform operations on a
`PullRequestClient` holding `&ObsClient` and `&GiteaClient`.

## The pull-request read shape

`show` returns one shape on both backends. Fields a backend omits default
to empty, false, or `None`, so the JSON keys are the same on every read.

`reference: PullRequestRef` — the handle.
`title: String`, `description: String` — OBS description or Gitea body;
empty by default.
`author: String` — OBS creator or Gitea author login.
`status: PullRequestStatus` — the unified state, mapped below.
`draft: bool` — Gitea draft flag; false on OBS.
`merged: bool` — Gitea merged flag; an accepted OBS request reads as
merged.
`mergeable: Option<bool>` — Gitea mergeable flag; `None` on OBS.
`source`, `target: PullRequestEndpoint` — `repository` (OBS
`project/package`; Gitea `owner/repo`), `branch: Option<String>` (Gitea
branch; `None` on OBS), `commit: Option<String>` (Gitea head and base SHA;
OBS source revision when the action carries one).
`reviews: Vec<ReviewOpinion>` — one entry per review asked or given, in
order.
`reviews_error: Option<String>` — the reviews-fetch error on the Gitea
arm, when the pull request read but its reviews did not; `None` otherwise,
always `None` on OBS.
`labels: Vec<String>` — Gitea label names; empty on OBS.

`search` returns `PullRequestSummary`: `reference`, `title`, `author`,
`status`, `source`, `target`. The list shape is the triage shape; detail is
a `show` call away.

`PullRequestStatus` is the union of both backends' states:

| `PullRequestStatus` | OBS `RequestState` | Gitea pull request state |
|---|---|---|
| `Open` | `New`, `Review` | `open` |
| `Merged` | `Accepted` | closed and `merged` |
| `Declined` | `Declined` | — |
| `Withdrawn` | `Revoked` | — |
| `Closed` | — | closed, merged flag false |
| `Superseded` | `Superseded` | — |
| `Deleted` | `Deleted` | — |

A Gitea close reads as `Closed`. Decline and withdrawal are separate
operations with separate OBS mappings; the Gitea side closes the pull
request for both.

`ReviewOpinion` is one review on the pull request: `reviewer: String` (the
asked-of target on OBS — user, group, project, or package; the reviewer
login on Gitea, or `@<team name>` for a team review),
`state: OpinionState`, `comment: String`, `when: Option<String>`.
`OpinionState` is the union of the per-review states:

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
`created: String`, `body: String`, `parent_id: Option<u64>` (OBS
threading; `None` on Gitea).

## The pull-request act surface

| Uniform operation | OBS | Gitea |
|---|---|---|
| `show(ref)` | `show_request`, with reviews | `pr_get` with `pr_reviews` |
| `search(filters)` | `search_requests` | `pr_list` in repo scope, `pr_search` globally |
| `approve(ref, comment, target?)` | `change_review_state` accepted, with `by_*` | `POST .../pulls/{n}/reviews` with `APPROVE` |
| `request_changes(ref, comment, target?)` | `change_review_state` declined, with `by_*` | `POST .../pulls/{n}/reviews` with `REQUEST_CHANGES` |
| `comment(ref, body, parent?)` | `add_comment` | `POST .../issues/{n}/comments` |
| `comments(ref)` | `list_comments` | `GET .../issues/{n}/comments` |
| `merge(ref, comment, method?, force?)` | `accept_request` | `POST .../pulls/{n}/merge` with `Do` |
| `decline(ref, comment)` | `decline_request` | `PATCH .../issues/{n}` closing the pull request |
| `withdraw(ref, comment)` | `revoke_request` | `PATCH .../issues/{n}` closing the pull request |
| `reopen(ref)` | `change_state` to `new` | `PATCH .../issues/{n}` reopening the pull request |
| `cancel_review_request(ref, reviewer \| --all [--exclude])` | The review-state operation takes `new`, `accepted`, and `declined`; cancel is a Gitea-side operation | `DELETE .../pulls/{n}/requested_reviewers` |
| `labels_add` / `labels_remove` | The label operations are library operations | `POST .../issues/{n}/labels`, `DELETE .../issues/{n}/labels/{id}` |

`change_review_state` is the typed review-state operation (`newstate` with
`by_user`, `by_group`, `by_project`, or `by_package`, plus the comment),
over `POST /request/{id}?cmd=changereviewstate` with the comment as the
POST body. `approve` and `request_changes` act on the review asked of the
acting user by default — the username from the OBS Basic credentials — and
take the same `--user`, `--group`, `--project`, and `--package` target
flags as `request add-review` for a review asked of a group, project, or
package. The target flags address OBS reviews; a Gitea review is by the
token owner.

`merge` takes `--method` (`merge`, `rebase`, `rebase-merge`, `squash`, or
`fast-forward-only`, sent as the Gitea `Do` field; unset sends `merge`, the
server default style) and `--force` (OBS: accept overriding open reviews,
as on `request accept`). `--force` is an OBS-side option. `decline` and `withdraw` are separate operations
because their OBS sides differ: decline refuses the pull request, revoke
withdraws the agent's own; both close the pull request on Gitea.
`reopen` moves an OBS request back to `new`.

`cancel_review_request` checks the pending set before the DELETE: the
server answers 204 regardless of the pending set, so repeat
cancel is a client-side error against the pending set. `--dry-run` reports
the plan. A team review request reads with a null user and a team object
in `GET .../pulls/{n}/reviews`.

`labels_remove` takes a name and resolves its id from the pull request's
current labels, returning a structured error naming the missing label.

## Submit

The library `submit` files a pull request and returns the handle:

```rust
pub enum SubmitTarget {
    Obs(SrPayload),
    Gitea {
        owner: String,
        repo: String,
        options: PrCreateOptions,
    },
}
```

The OBS arm files through `SrFiling::file` and returns
`PullRequestRef::Obs(id)`; the Gitea arm opens the pull request through
`GiteaClient::pr_create` (`POST /repos/{owner}/{repo}/pulls` with `head`,
`base`, `title`, and `body`) and returns `PullRequestRef::Gitea { owner,
repo, number }`. The caller picks the variant for the backend it files to
— the backend kind is known at the call site from the checkout — and reads
the result backend-agnostically.

The Gitea-side routing: a pull request goes to the immediate parent repo —
for a fork of the devel repo, the devel repo — on the branch the checkout
tracks (its scmsync branch), or the target's default branch when the
checkout tracks none, unless `--base` names another branch. The target
repo defaults to the fork's parent, resolved through `repo_get`; the target
branch defaults to the target repo's default branch, resolved the same way.
The Factory leg files as an OBS submit request; the pull-request target is
the devel branch. The CLI `submit` verb is `pull-request submit`.

## Repositories and branches

| Operation | Endpoint | Agent use |
|---|---|---|
| `repo_get(owner, repo)` | `GET /api/v1/repos/{owner}/{repo}` | Resolve the default branch; verify a fork's parent by re-reading; the read every later operation builds on |
| `repo_list(owner?, org?)` | `GET /api/v1/orgs/{org}/repos`, `/api/v1/users/{user}/repos`, `/api/v1/user/repos` | Find the agent's forks or an org's repos before acting on one |
| `repo_search(query)` | `GET /api/v1/repos/search?q=...&includeDesc=true` | Find a repo by keyword (name and description) before acting on one — the Gitea counterpart of the OBS keyword search |
| `repo_create(name, options)` | `POST /api/v1/user/repos` | Create the repo a new git-backed package's sources live in |
| `repo_delete(owner, repo)` | `DELETE /api/v1/repos/{owner}/{repo}` | Delete a fork after the OBS branch is gone, or for cleanup |
| `branch_get(owner, repo, branch)` / `branch_list(owner, repo)` | `GET /api/v1/repos/{owner}/{repo}/branches[/{branch}]` | Verify a branch exists before naming it in a scmsync URL; list what a fork carries |
| `branch_head(owner, repo, branch)` | Branch read, `commit.id` | Read the head commit SHA of a branch |
| `maintainership(owner, repo)` | `GET /api/v1/repos/{owner}/{repo}/raw/_maintainership.json` | The project maintainers and the per-package maintainers, the git-side counterpart of the OBS `maintainers` read |

List operations page internally at 50 entries per page and fail past a
bound of 500 entries with a structured error naming the filter to narrow.
Reads that take a branch take it as a typed parameter. The maintainership
read takes the file from the repo's default branch; a missing file answers
`NotFound`.

### The fork composite

`GiteaClient::branch` forks a git-backed package or project in one call:
it reads the source meta, answering `InvalidInput` for a source carrying
no `<scmsync>`, resolves the devel project unless `--no-devel-project` is
given, forks the repo, and wires the OBS fork to the fork's scmsync URL.
Then it copies the source project's repositories into the fork's test
project, the server-resolved target: all of them, verbatim, skipping
any the target already carries — the server adds no repositories on
`cmd=fork`, and a source project with no repositories copies nothing.
A copy failure fails the branch loudly.
The scmsync URL carries the fork branch (`url#branch`); when the branch is
unnamed, the composite resolves it with `repo_get` on the default branch, and
`--git-branch` names it explicitly. The composite returns, typed, the fork
repo (`owner`, `name`, `branch`), the scmsync URL, the OBS target
project and package, and the `clone_url` and `ssh_url` for the agent's own
`git clone`. The fork result returns the URLs; the clone runs through the
git CLI.

The composite waits for the first build of the forked package in the
fork's test project and reports it on the result: settled once the
build matrix settles, still running with the latest matrix when the
deadline passes, not waited at deadline zero, no repositories when
the source project carries none, and project fork for a project fork.
The wait is bounded by `--first-build-deadline-secs` (no wait by
default; pass a positive value to wait); a slow build is not a branch failure — the branch stands
and the caller sees the latest state. Tracking is the scmsync link:
a push to the fork branch syncs into the test project and OBS
rebuilds, with no re-run of the leaf. A second branch reuses the
existing fork, target project, and repositories, and still reports
the first build.

An existing fork under the requested name is reused. A same-name repo
whose parent differs from the source answers `AlreadyExists`.
Forking someone else's fork answers 202; the composite verifies the created
fork by re-reading it and checking its `parent` before reporting success,
and a mismatched parent answers `ServerError` naming the re-read. A second
fork under a different name answers 409 (one fork per user per source);
finding the existing fork under its name is a `repo_list` call. The dry-run
guard runs before the meta read: a dry run reports the plan and creates
nothing.

`repo_delete` is dry-run guarded like `delete_package`: a dry run reports
the plan; a live delete needs `--force` to confirm, then GETs to confirm
owner and name; server-side permission is the guard, and the API enforces
owner-or-admin-only delete.

## Library operations

Team operations live in `GiteaClient` as library operations: team list,
get, create, delete, member list, get, add, and remove, repo list, get,
add, and remove. An agent uses them through the pull-request read and
act surfaces: a team review reads as `@<team name>`, and a team review
request is cancelled by `@team`, so the team operations resolve the team
behind a review — its members and its repos. Collaborator operations live
in `GiteaClient` as library operations: permission read, membership check,
add, and remove. An agent uses them to read the access a user holds on a
repo and whether the user is a collaborator, and to grant or revoke repo
access. Label operations live in `GiteaClient` as library operations:
`pr_labels_add` and `pr_labels_remove`. An agent uses them to mark a pull
request with triage labels; the labels read back on the pull-request read
shape.

## Configuration and auth

`gitea_url` in the config file, default `https://src.opensuse.org`, with
the `GEECTL_GITEA_URL` environment override. One configured Gitea instance
per client; a repo URL on another host answers `ConfigError` naming the
configured host. Named instances live in the config `[instances]` table;
the identity is scheme, host, and effective port.

`EnvGiteaAuth` carries the Bearer token from `GEECTL_GITEA_TOKEN` or
`GEECTL_GITEA_TOKEN_FILE`, exactly one, or from the matching alias's
`GEECTL_GITEA_TOKEN_<ALIAS>` and `GEECTL_GITEA_TOKEN_FILE_<ALIAS>` when
the URL is named in `[instances]`, resolved per URL by
`resolve_gitea_bearer_for_url`. `GiteaSshKey` carries the private-key path
from `GEECTL_GITEA_SSH_KEY` for the agent's `git clone` over SSH.
`GiteaClient` construction requires a token, including in dry-run.

Typed package and project meta carry `scmsync: Option<String>`
(`url#branch`); `validate_scmsync` requires the `#fragment`;
`create_package` takes an `scmsync` option. Creating a package inside a
scmsync-managed project is refused upfront; a link starts with the
source's scmsync dropped. Linking *from* a scmsync-managed source is
refused upfront in `link_package` instead: the target's own scmsync
still survives the edit-in-place path.

## Transport and errors

`GiteaClient` is built from the same `Config` and `AuthProvider` as
everything else. JSON travels through `serde` `Deserialize`, unknown
fields ignored — the structs tolerate a newer Gitea than the pinned test
image, keeping the harness working. Only the fields each operation needs
are modelled, and public signatures carry typed values only: owners and
repos as validated names, pull request numbers as `u64`. The `HttpClient`
trait is reused as-is (`get_with`, `post_json_with`, `patch_json_with`,
`delete`); transport is the `HttpClient` trait only. The 10 MiB read cap
applies the same way; a Gitea response over it is the same structured
error as on the OBS side.

The status map: 404 answers `NotFound`, 401 and 403 answer `AuthFailed`,
5xx answers `ServerError`, 422 answers `InvalidInput` carrying the
server's message. `AlreadyExists` (`already_exists`) is a distinct,
actionable state: reuse the fork, or pick another name. A 3xx answer is a
structured redirect error naming the location, and the caller retries or
reports it.

The debug channel reports HTTP attempts, retries, and endpoint selection
through the one `--debug` channel (`--debug` flag with `GEECTL_DEBUG=1`,
human lines on stderr). The Gitea client uses the same `HttpClient` and
the same `Debug` sink as the OBS client. `--debug-body` prints bodies
size-capped and credential-redacted.

## Sync verification

Filing a submit request from a git-backed package verifies the target and
the checkout state: the target repo is the parent of the checkout's origin
and the base exists on the target, then the remote-ahead, conflict, and
uncommitted checks run. The `commit` dispatch on a git-backed checkout
polls the OBS side (`_scmsync.obsinfo` `commit:`) until the synced commit
matches the pushed head, bounded at 60 seconds. The `_scmsync.obsinfo`
parse rules cover the `commit:` field the poll compares against.

## Testing

The live suite is `tests/gitea.rs`: every scenario asserts on server state
through the typed operations, against a disposable Gitea 1.24.7 instance
(`tests/gitea/provision.sh`). The provisioned token carries the scopes the
live tests need; issue comments and labels need `write:issue`, so the
comment-path tests mint a second token carrying it. The instance stays
pinned at Gitea 1.24.7; a version bump is a deliberate change, with the
live suite proving the new version still accepts the client.
