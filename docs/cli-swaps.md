# geectl CLI — osc → geectl swaps

Every deliberate deviation from osc's command/flag shape in the CLI
rollout, with the consistency reason. Where geectl matches osc, there
is no row. The scheme-level rows apply across all
nine groups and are listed once; the per-group sections list only
what is specific to that group.

## Scheme-level (all groups)

| osc | geectl | Why |
|---|---|---|
| Positional `PROJECT`/`PACKAGE` args, often defaulting to the working copy | Explicit `--project`/`--package` (etc.) flags, always required; required coordinates that stay positional are the query itself | No positionals for options; the grouped commands name their coordinates explicitly. The working-copy leaves are the exception: they operate on the checkout in the current directory and take no coordinates, apart from `checkout`'s project/package positionals |
| Short flags (`-f`, `-r`, `-m`, `-e`, `-k`, `-d`, `-N`, `-M`, `-c`, `-R`, `-u`, `-t`, `-a`, `-l`, `-w`, `-s`, `-b`, `-V`, `-F`, `-o`) | Long forms only, except `commit -m` | Agents type full names; one flag name per concept, no aliasing. `commit -m` is the one short flag, carried over from the working-copy command shape |
| Command aliases (`r`, `bl`, `blt`, `buildhist`, `jobhist`, `rpmlint`, `lint`, `bco`, `branchco`, `getpac`, …) | No aliases | Full names only; no aliases |
| Working-copy inference (project/package from the `.osc` checkout) | Explicit coordinates always, except the working-copy leaves, which read project/package from the checkout in the current directory | The grouped commands name their coordinates; the working-copy leaves are the ones that use the checkout |
| Per-command output formats (`--xml`, `--csv`, `--format` with named-field interpolation) and human text only | One global `--format json\|text` | One output switch for the whole CLI, per the #51/#52 design; every command returns `(payload, text)` |
| `osc` mutates on invocation | Every mutation is dry-run by default; the global `--live` performs it | Geectl-wide rule (library `DryRun` default): an agent sees the plan before anything changes. The dry run prints the plan on stderr and returns `dry_run: true` |
| `osc -m/--message` for several meanings | `--comment` for state-change, review, and branch messages; `--description` for a filing's description | One flag name per concept everywhere, instead of one flag carrying two meanings |

## `remote` — inspection and repository edits (issue #72)

| osc | geectl | Why |
|---|---|---|
| Overloaded commands (`ls`, `cat`, `log`, `meta`, `search` with modes) | One command per library operation (`list-files`, `cat`, `package-log`, `project-meta`, `package-meta`, `search-packages`, `search-projects`, …) | One subcommand per operation, grouped under the `remote` noun; no mode flags |
| `osc search` exact-name default | Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches | The library has no exact search server-side; the filter runs in the CLI, and osc's default stays reachable via `--exact` |
| `osc rdiff` (server-side diff) | `remote revision-patch` and `remote project-diff`, computed client-side from per-file reads | The library lists and cats the files itself; a large `project-diff` issues many requests and a differing binary file fails the call |
| `osc rdiff` two-package form | `remote diff-packages` | Client-side diff of two packages from per-file reads (issue #74); a differing binary file fails the call |
| `osc develproject` (prints devel project + package) | `remote devel-project <package>` | Factory-only; the library op returns the (project, package) pair (issue #77). `remote package-meta` shows the same pair as `devel: project/package`. |
| `osc repo add` / `osc repo remove` (project admin writes) | `remote repo-add` / `remote repo-remove` | Grouped under `remote` with the other project reads; mutations take the global `--live` |
| (no osc command) | `remote upload-file` | New surface: a raw file upload is API-only in osc (`osc api -X PUT -T FILE <path>`); the local file is read from `--from`, `--binary` uploads bytes |
| `osc get` single file / `osc cat` (streamed read) | `remote download` | Streams the file to `--output` instead of printing it; the rev passes verbatim (a number or a srcmd5), like `cat` — osc resolves the expanded srcmd5 by default. osc writes to a temp file and renames; geectl streams to the destination with exclusive create, so a failure mid-stream leaves a partial file for the caller to delete |
| (no osc command) | `remote create-project` | New surface: osc has no project-create command; projects are created through `meta -e` — the library creates the server meta directly |
| `osc rmkpac [--scmsync SCMSYNC] [--force] PROJECT PACKAGE` | `remote create-package` | One command per library operation. `--scmsync` / `--force` select the options form of the creation |
| `osc rdelete [-m TEXT] PROJECT PACKAGE` | `remote delete-package` | One command per library operation; osc's `-m` is `--message`, sent as the server's `comment` |
| (no osc command) | `remote delete-link PROJECT PACKAGE` | No single server-side command: a working copy can delete the `_link` file and commit it, or `osc api -X DELETE /source/PROJECT/PACKAGE/_link` deletes it directly; the library guards the delete behind dry-run |
| `osc meta prj PROJECT -e` | `remote set-project-meta PROJECT [--title T] [--description D] [--url U] [--scmsync S] [--devel-project P] [--add-person LOGIN:ROLE] [--remove-person LOGIN]` | Field-scoped write, no editor: only the given fields change, the rest of the meta is kept; osc edits the whole document in an editor |
| `osc meta pkg PROJECT PACKAGE -e` | `remote set-package-meta PROJECT PACKAGE [--title T] [--description D] [--url U] [--scmsync S] [--releasename R] [--devel-project P] [--devel-package K] [--add-person LOGIN:ROLE] [--remove-person LOGIN]` | Same; osc edits the whole document in an editor, geectl writes fields |
| `osc updatepacmetafromspec [WORKING-COPY] [--specfile FILE]` | `remote update-package-meta-from-spec PROJECT PACKAGE [--spec FILE]` | Same update (spec Summary → title, Url → URL, %description → description); geectl takes project/package coordinates, osc takes a working copy |
| `osc rdelete [-r] [-f] [-m TEXT] PROJECT` | `remote delete-project` | One command per library operation; `-r` is `--recursive`, `-f` is `--force`, `-m` is `--comment` |
| `osc undelete [-m TEXT] PROJECT PACKAGE` | `remote undelete-package` | One command per library operation, one package per invocation; osc's `-m` is dropped — the library sends no comment |
| `osc undelete [-m TEXT] PROJECT` | `remote undelete-project` | One command per library operation; osc's `-m` is dropped — the library sends no comment |
| `osc aggregatepac SOURCEPRJ SOURCEPAC DESTPRJ [DESTPAC]` | `remote aggregate-package SOURCE-PROJECT SOURCE-PACKAGE TARGET-PROJECT TARGET-PACKAGE` | Grouped + spelled-out; the target package is required (osc defaults it to the source name). `-m SRC=TARGET` is repeatable `--repository-map SOURCE=TARGET`; `--nosources` is `--no-sources`, `-d` is `--disable-publish` |
| `osc release [SOURCEPROJECT SOURCEPACKAGE]` | `remote release-package` | One command per library operation. `-r` is `--repository`, `-a` is `--arch`; `--target-project` / `--target-repository` / `--set-release` / `--no-delay` keep their names |
| `osc release [SOURCEPROJECT]` | `remote release-project` | One command per library operation; same flag mapping as `release-package` |
| `osc ls /` | `remote list-projects` | Same listing, named as a leaf under `remote` |
| (no osc command) | `remote list-staging-projects PROJECT` | New surface: osc has no staging read (`osc staging` is not a command in this osc); the library reads `/staging/<workflow>/staging_projects` |
| (no osc command) | `remote staging-project PROJECT NAME` | Same; the one-project read |
| `osc search -i LOGIN --project` | `remote search-projects-by-person LOGIN` | Same search: the projects a person is involved in, by login |
| `osc whois [USER ...]` (aliases `who`, `user`) | `remote whois [LOGIN]` | Same read; the bare form defaults to the authenticated user, like osc's configured-user default |
| (no osc command) | `remote count-scmsync-projects URL-PREFIX` | New surface: the scmsync-prefix project count is API-only in osc |

## `request` — submit requests (issue #73)

| osc | geectl | Why |
|---|---|---|
| `osc sr` / `submitrequest` (top-level command) | `request file` | One noun per operation family: filing is a leaf under `request`, not a top-level command. The library op is `SrFiling::file` |
| `osc sr -s ID` / `--supersede` | `request refile --supersede` | The file-first-then-supersede order is explicit in the name; a bare supersede is `request supersede` |
| `osc mr` / `maintenancerequest` (top-level command) | `request file-maintenance-incident`, `request refile-maintenance-incident --supersede` | Incident filing is an `SrFiling` op, so it lives with the other filings under `request`, not with the maintenance group's read/branch ops (`maintained-instances`, `maintenance-branch`, issue #69) |
| `osc changedevelrequest` (`cr`, `changedevelreq`) | `request file-change-devel` | Same grouping: a filing leaf under `request` |
| `osc deletereq` (top-level command, positional `PROJECT [PACKAGE]`) | `request file-delete --target-project … [--package … \| --repository …]` | Same grouping; the target is selected by flags (package, repository, or neither for the whole project) instead of positionals |
| `osc deletereq --accept-in-hours` | `request file-delete --accept-in-hours` | Name kept; the flag names the accept timer on the delete request |
| `osc sr` / `osc creq` with no `--cleanup` / `--no-cleanup` / `--no-update` (and no config) | `request file` (and every filing) always sends `cleanup` unless `--without-cleanup` | Geectl's default is cleanup on every filing; osc sends no `<sourceupdate>` unless told to (or configured) — which matches osc's documented default for branch sources only |
| `osc request show -d` | `request diff` (+ opt-in `--xml`) | The diff is its own leaf, not a flag on `show`; the XML view is an explicit opt-in |
| `osc request review add` | `request add-review --user/--group/--project/--package` | Verb-first leaf; the review target is named by flags |
| `osc comment list/create request ID` (top-level `comment` command with a `request` target word) | `request list-comments`, `request add-comment [--parent …]` | Comments live under the request they belong to, not under a top-level `comment` command. (osc itself has no `request comment` subcommand — comments are API-only there.) |
| `osc request list` | `request search` | The library op is a typed search (`RequestSearch`), and the name says what it does; filters are named flags (`--creator`, `--state`, `--project`, …). Default states new/review/declined when no `--state` is given |
| `osc review list -U USER` | `request search --reviewer USER` | The reviewer filter is a search flag, not a separate list command: it matches open reviews addressed to the user. With no `--state`, only new and review requests are listed, since a declined or accepted request keeps its open reviews. Group and project reviews have no flag. |
| `request supersede` | `request supersede` acts immediately, no dry-run | The library's raw supersede has no dry-run; `request refile` is the dry-runnable supersede flow. Documented on the subcommand |

Not swaps (kept as-is): `request show`, `request log`, `request
accept`, `request decline`, `request revoke` keep osc's names.
`request change-state` / `request try-change-state` follow the
kebab-case rule on the library operation names.

## `branch` — branch/copy/link (issue #76)

Only deliberate deviations; where geectl matches osc (e.g.
`--force`, `--revision`, `--expand`, `--keep-link`, `--maintenance`,
`--disable-build`/`--disable-publish` on link-package,
`--add-repositories*`, `--noaccess`, `--extend-package-names`),
no row.

| osc | geectl | Why (consistency reason) |
|---|---|---|
| Flat top-level commands (`branch`, `copypac`, `linkpac`, …) | `branch <subcommand>` group | Uniform grouping: one top-level noun per library module across all seven CLI groups; osc's flat list doesn't scale and collides (`branch` vs `mbranch`) |
| `osc branch` | `branch branch-package` | Grouped + spelled-out operation name (kebab-case library name) |
| `osc branch --nodevelproject` | `--no-devel-project` | Kebab-case flag names everywhere (osc concatenates) |
| `osc branch -c/--checkout` | `branch branch-package` then `checkout PROJECT PACKAGE` (aliases `bco`/`branchco`/`getpac` have no equivalent) | Branches AND checks out a working copy; geectl does the two steps as two commands — the branch, then a checkout of the new package |
| `osc branch --disable-build` | Dropped | No corresponding option on `BranchOptions`; branch meta build flags are out of the branch op's scope |
| `osc copypac` | `branch copy-package` | Grouped + spelled-out (osc abbreviates to `copypac`) |
| `osc copypac -c/--client-side-copy` | Dropped (same-instance form) | Client-side download/upload copy of a single package; the cross-instance form exists as `--to-apiurl`, but there is no same-instance client-side mode — the same-instance copy stays server-side |
| `osc copypac -k/--keep-maintainers` | Dropped | No library option; the server builds the target meta from the source meta |
| `osc copypac -d/--keep-develproject` | Dropped | No library option |
| `osc copypac -t/--to-apiurl` | `branch copy-package --to-apiurl <URL>` | Cross-instance copy, client-side download+upload; the target instance's credentials resolve from the instance alias matching the target URL (`GEECTL_OBS_USERNAME_<ALIAS>` / `GEECTL_OBS_PASSWORD[_FILE]_<ALIAS>`) and the copy fails clearly when none are configured — the source credentials are never forwarded |
| `osc linkpac` | `branch link-package` | Grouped + spelled-out |
| `osc linkpac -c/--current` | Dropped | Pins the link at the source's current expanded revision, which needs a directory read first; the library op takes an explicit revision only |
| `osc linkpac -N/--new-package` | `--missing-target` | `--new-package` already means "branch a package that does not exist yet" in `branch-package`; the library field is `missing_target` |
| `osc repairlink` | `branch repair-link` | Same name, different operation: osc does a working-copy 3-way merge (checkout, merge, `resolved`, `ci`); geectl strips the failing patches from `_link` server-side and pins it at a revision where it expands, with no local merge. (osc's cross-project form `repairlink PRJ PKG INTO_PRJ [INTO_PKG]` is dropped; geectl repairs the link in place.) |
| `osc setlinkrev` | `branch set-link-rev` | Grouped + kebab-case |
| `osc setlinkrev -r REV` (default: expanded) | `--revision <rev>` (bare = expanded) | Bare value keeps osc's default (expand); `pinned:<rev>` selects the plain form |
| `osc setlinkrev -R/--use-plain-revision` | `--revision pinned:<rev>` | The plain/expanded choice becomes part of the revision value instead of a second flag; one flag per concept |
| `osc setlinkrev -u/--unset` | `--revision unset` | Unset is a revision value, not a separate flag |
| `osc setlinkrev --vrev` | Dropped | The library derives vrev from the directory fetch for md5 revisions; no explicit vrev |
| `osc setlinkrev` with no `-r` (current revision, expanded) | `--revision expanded` | Same default, spelled out as a value |
| `osc setlinkrev -R` with no `-r` (current revision, plain) | `--revision latest` | Same default, spelled out as a value |
| `osc setdevelproject` | `branch set-devel-project` | Grouped + kebab-case |
| `osc linktobranch` | `branch link-to-branch` | Grouped + kebab-case |
| `osc detachbranch` | `branch detach-branch` | Grouped + kebab-case |
| `osc detachbranch -m/--message` | Dropped | Passed as the copy comment in osc; the library detach carries no comment |

Additions with no osc equivalent (not swap rows): `branch
copy-project` (no `osc copyprj` exists in this osc version);
`branch set-link-rev-project` (no osc project-wide command);
`copy-package` also exposes `--noservice`, `--dont-update-source`,
`--with-vrev` and `link-package` exposes `--vrev` — library options
with no osc counterpart.

## `build` — build status (issue #71)

| osc | geectl | Why |
|---|---|---|
| `results -w/--watch` | `build wait-for-results --deadline-secs N` | Waiting is a separate deadline-bounded operation, not a flag; osc's watch polls forever with no bound |
| `results -r/--repo` | `build results --repository` | Full word, one name per concept (`repo` vs `repository` unified to `repository`) |
| `results -s/--status-filter` | `build results --code` | Flag named after the library query field |
| `results -f/--failed` | Dropped (`--code failed`) | No single-value shortcut flags |
| `results -l/--last-build` | `build results --lastbuild` | Library field name |
| `results -M/--multibuild-package FLAVOR` | Dropped (pass `pkg:flavor` in `--package`) | The flavor travels in the package value, per the library |
| `results --no-multibuild` | Dropped | Multibuild is opt-in in the library (`--multibuild` enables it); there is nothing to disable |
| `results -b/--brief, -V/--vertical, -F/--fail-on-error, --show-excluded` | Dropped | Presentation variants with no library equivalent; text rendering is fixed by #52 |
| `results [PROJECT [PACKAGE]]` | `build results PROJECT` + `--package` flag | Positionals are required coordinates only; the optional package is a flag |
| `prjresults [PROJECT]` | Dropped (`build results PROJECT`) | The same matrix without a package filter; one matrix command |
| `buildlog` | `build build-log` | Kebab-case library name under the `build` group; the mapping stays mechanical (group + kebab(library op)) |
| `buildlog [REPOSITORY ARCH \| BUILDLOGURL]` | `build build-log PROJECT PACKAGE REPOSITORY ARCH`, or `build build-log BUILDLOGURL` | Explicit coordinates, or a single build-log URL (API or web-UI form); no hostarch default |
| `buildlog` tail via `blt` alias | Dropped | No follow/tail mode; the library reads whole logs only |
| `buildlog --lastsucceeded` / `--last-succeeded` (two spellings) | `--lastsucceeded` | One spelling, the library field name |
| `buildlog -M/--multibuild-package FLAVOR` | `build build-log --multibuild FLAVOR` | The flavor appends `:FLAVOR` to the package |
| `buildlog -o/--offset` | `build build-log --start` | Flag named after the library query field |
| `buildlog -s/--strip-time` | `build build-log --strip-time` | Client-side strip of the leading `[timestamp] ` from each log line |
| `buildinfo [REPOSITORY ARCH [BUILD_DESCR]]` | `build buildinfo PROJECT PACKAGE REPOSITORY ARCH` | Explicit coordinates; no local-spec (`BUILD_DESCR`) form — the library reads the server-side spec only |
| `buildconfig REPOSITORY (in pkg or prj dir) \| buildconfig PROJECT REPOSITORY` | `build buildconfig PROJECT REPOSITORY` | Explicit coordinates; no in-directory form — project is always required; the config text renders verbatim, coordinates in the payload |
| `buildhistory` | `build build-history` | Kebab-case library name, same mechanical mapping as `build-log` |
| `jobhistory [PROJECT [PACKAGE] REPOSITORY ARCH]` | `build job-history PROJECT REPOSITORY ARCH` + `--package` flag | Positionals are required coordinates only |
| `repos [PROJECT] [PACKAGE]` | `build list-repositories PROJECT` | Verb-noun `list-*` scheme for listings; package argument dropped — the library reads project meta only, no per-package disabled-repo filtering |
| `getbinaries …` | `build list-binaries` + `build download-binary` | One osc command split into list vs download, mirroring the library split |
| `getbinaries` implicit cwd/subdir writes | `download-binary --output PATH` (required) | Explicit destination; no implicit writes into the working directory |
| `getbinaries PROJECT PACKAGE REPOSITORY ARCH` (whole package) | `build download-binary --all --output DIR PROJECT PACKAGE REPOSITORY ARCH` | One list call plus one streaming download per file, into the named directory; downloads every listed file, including the `.src.rpm` and `-debuginfo` files osc skips unless `--sources`/`--debuginfo`; the all-arch form (one invocation per architecture) has no equivalent |
| `rpmlintlog` | No dedicated command (use `build download-binary … rpmlint.log --output …`) | One generic download instead of per-artifact shortcuts |
| `whatdependson REPOSITORY ARCH` | `build whatdependson PROJECT PACKAGE REPOSITORY ARCH` | Explicit coordinates; package is a required positional per the library signature |
| `osc rebuild [PROJECT [PACKAGE]]` | `build rebuild-package PROJECT PACKAGE REPOSITORY ARCH` | One command per library operation, explicit coordinates; osc's project-wide and `--all` forms have no library op (per-package only) |
| `osc restartbuild [PROJECT [PACKAGE]]` | `build restart-build PROJECT PACKAGE REPOSITORY ARCH` | Same mapping; per-package only, no project-wide form |
| `osc abortbuild [PROJECT [PACKAGE]]` | `build abort-build PROJECT PACKAGE REPOSITORY ARCH` | Same mapping; per-package only, no project-wide form |

No geectl equivalent (coverage gaps, not deviations): `chroot` /
`lbl` — local build; out of scope (issue #43, podman vertical).
`build build-status`, `build job-status`, `build
fileinfo`, `build dependson`, `build check-constraints`, `build
list-binaries` — new surface with no osc command (reachable in osc
only via `osc api`).

## `local-build` — podman build vertical (issue #43)

| osc | geectl | Why |
|---|---|---|
| `build` | `local-build build DIRECTORY --project PROJECT --repo REPOSITORY` | One operation per the design: the run-mode service run, the precommit gate, then the engine build in one invocation; project and repo are explicit coordinates, arch defaults to the host arch; `--arch`, `--clean`, `--flavor`, `--release`, `--trust-all-projects`, `--refresh-image`, `--timeout-secs` are the only options in v1 |
| `osc wipe [REPOSITORY ARCH]` (alias of `build`, sets `--wipe`) | `local-build wipe REPOSITORY ARCH SPEC --project PROJECT` | Wipes the root the build's own coordinates name; the package comes from the spec file in the source directory, like the build; `--root` names the root directly instead of the coordinates; no force flag — the marker check and the in-use lock are safety invariants |

`--trust-all-projects` lifts only the foreign-`home:` refusal for the
dep set the buildinfo enumerates: non-`home:` projects and the caller's
own `home:` were always trusted. That is a deliberate difference from
osc's `trusted_prj` globs, which name the trusted projects — geectl
takes no per-project selection and never prompts. The accepted foreign
`home:` projects are recorded in the build record
(`trusted_foreign_projects`); a dry run records the flag in the plan
without evaluating trust.

## `maintenance` — maintained instances + branching (issue #69)

| osc | geectl | Why |
|---|---|---|
| `osc mbranch [SOURCEPACKAGE [TARGETPROJECT]]` (flat top-level command, positionals) | `maintenance maintenance-branch --package … [--target-project …]` | Grouped under the maintenance noun; package and target are named flags, no positionals anywhere in geectl |
| `osc maintained` / `osc sm` (aliases of `mbranch`, list-only) | `maintenance maintained-instances --package …` | The list form is its own leaf, not an alias or a mode flag on the branch command. Semantics match for `sm`: both are the dry-run branch call that reports without branching. `osc maintained` additionally prints `[git] org/package:branch` lines from the Gitea cache, which geectl does not reproduce (Gitea scope, issue #20) |
| `osc mbranch -c/--checkout` | `maintenance maintenance-branch` then `checkout PROJECT PACKAGE` | Branches AND checks out a working copy; geectl does the two steps as two commands — the branch, then a checkout of the new package. (The `--checkout` help calls `osc bco` a shorthand for it, but `bco` is a separate branch/getpac command, not an mbranch alias.) |
| `osc mbranch --noaccess`, `--nodevelproject` | Dropped | No corresponding option on `MaintenanceBranchOptions` |
| `osc mbranch --version` (print version of the maintained package) | Dropped | No geectl equivalent; the top-level `version` command reports geectl itself |

Not swaps (kept as-is): `--attribute` and
`--update-project-attribute` keep osc's names and defaults
(`OBS:Maintained` / `OBS:UpdateProject`). The default branch project
`home:<user>:branches:OBS_Maintained:<package>` matches osc's mbranch
layout. `osc mr` (`maintenancerequest`) is NOT here: incident filing
lives with the other filings under `request` (`request
file-maintenance-incident`, issue #73).

## `service` — service reads

| osc | geectl | Why |
|---|---|---|
| (no osc command) | `service list` / `service state` | New surface: read a package's `_service` file and its last run state; osc has no service reads |

## `gitea` — Gitea operations

| osc | geectl | Why |
|---|---|---|
| (no osc command) | `gitea repo-get` / `gitea repo-list` / `gitea branch-list` / `gitea branch-get` / `gitea branch-head` | New surface: Gitea reads on the configured instance; osc has no Gitea reads |
| (no osc command) | `gitea repo-create` | New surface: create a repository on the configured instance; mutations take the global `--live` |
| `osc api -X DELETE /repos/...` | `gitea repo-delete` | The delete op under the `gitea` noun; mutations take the global `--live`, and the library refuses without `--force` |
| `osc fork` | `gitea repo-fork` (Gitea half only) | `osc fork` is a composite: Gitea fork + OBS branch. `gitea repo-fork` is the Gitea half; the `branch` composite (Gitea fork + OBS `cmd=fork`) has no CLI command. `osc fork` is also scmsync-only, and it clones the fork locally and inits `.osc` — neither of which the Gitea half does |

Scope note: the sibling `git-obs` entry point has `repo list` and `repo fork` (`osc/commands_git/repo_list.py`, `repo_fork.py`); they are `git-obs` commands, not `osc` commands, so they sit outside this table's stated scope — `git-obs repo list` requires `--org`/`--user` (no own-repos form).

## Working copy — git working copy

| osc | geectl | Why |
|---|---|---|
| `osc checkout` / `co` / `getpac` / `bco` `PROJECT PACKAGE` | `checkout PROJECT PACKAGE [--rev REV]` | Same checkout, different metadata: osc records the checkout in `.osc`; geectl checks the package out as a git working copy under `<dir>/<project>/<package>/` and records the base in git config (`geectl.apiurl`, `geectl.project`, `geectl.package`, `geectl.backend`, `geectl.base-rev`, `geectl.base-srcmd5`). A git-backed package is a real clone; an OBS-native package gets a synthesized repo with the base as its root commit. `checkout` streams every listed file to disk, with no size refusal |
| `osc update` / `up` | `update [--rev REV]` | Same update, different merge: osc merges the new revision into the working copy; geectl rebases the local work onto the new base, and a conflict stays in the tree for the agent to resolve |
| `osc status` / `st` | `status` | Same state read; the one-letter codes are `A` (added), `M` (modified), `D` (deleted), `?` (untracked), `!` (missing), `C` (conflict), `R` (renamed) |
| `osc diff` | `diff` | The diff of the worktree and the index against the base, rendered verbatim; a linked package's link diff is out of scope |
| `osc add FILE...` | `add FILE...` | Same stage |
| `osc delete` / `rm FILE...` | `rm FILE...` | Same remove |
| `osc addremove` | `addremove` | Same stage-everything |
| `osc commit` / `ci` / `checkin` | `commit --message MSG [--noservice]` | Same commit, plus a hard gate: the source services and `pre_checkin.sh` run before the send, unless `--noservice` is passed. The send is the staged-plus-committed state against the base; unstaged worktree changes are not sent, except an unstaged edit to a file the service gate changed, which is staged whole with the gate's output |
| `osc pull` | `pull` | Linked packages only: pulls the link target's latest revision into the checkout; a clean working copy (no uncommitted changes and no untracked files) and a recorded baserev are required |
| `osc resolved` | `resolve` | Stages the resolved files; when a rebase is in progress and every conflict is resolved, continues it |
| `osc mkpac PROJECT PACKAGE` | `remote create-package` then `checkout PROJECT PACKAGE` | `mkpac` creates the package AND checks it out; geectl has no such leaf — the create and the checkout are two commands |
| `osc vc` | `changelog` (separate layer) | The changelog validation/generation layer is its own surface; the working copy does not validate the changelog before commit |
