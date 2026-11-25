# geectl CLI — osc → geectl swaps

Deliberate deviations from osc's command/flag shape, with the
consistency reason. Rows cover deviations only. The
scheme-level rows apply across all twelve groups and are listed once;
the per-group sections list only what is specific to that group.

## Scheme-level (all groups)

| osc | geectl | Why |
|---|---|---|
| Positional `PROJECT`/`PACKAGE` args, often defaulting to the working copy | Explicit `--project`/`--package` (etc.) flags, always required; required coordinates that stay positional are the query itself | Options use flags; the grouped commands name their coordinates explicitly. The working-copy leaves are the exception: they operate on the checkout in the current directory |
| Short flags (`-f`, `-r`, `-m`, `-e`, `-k`, `-d`, `-N`, `-M`, `-c`, `-R`, `-u`, `-t`, `-a`, `-l`, `-w`, `-s`, `-b`, `-V`, `-F`, `-o`) | Long forms only, apart from clap's built-in `-h`/`-V`, `geectl commit -m`, and `geectl local-build build -k`/`-p` | Agents type full names; one flag name per concept. `geectl commit -m` and `local-build build -k`/`-p` are the short flags |
| Command aliases (`r`, `bl`, `blt`, `buildhist`, `jobhist`, `rpmlint`, `lint`, `bco`, `branchco`, `getpac`, …) | Full names only | Commands have a single full name |
| Working-copy inference (project/package from the `.osc` checkout) | Explicit coordinates always, except the working-copy leaves, which read project/package from the checkout in the current directory | The grouped commands name their coordinates; the working-copy leaves are the ones that use the checkout |
| Per-command output formats (`--xml`, `--csv`, `--format` with named-field interpolation) and human text only | One global `--format json\|text` | One output switch for the whole CLI; every command returns `(payload, text)` |
| `osc` mutates on invocation | Every mutation is dry-run by default; the global `--live` performs it | An agent sees the plan before anything changes. The dry run prints the plan on stderr and returns `dry_run: true` |
| `osc -m/--message` for several meanings | `--comment` for state-change, review, and branch messages; `--description` for a filing's description | One flag name per concept everywhere |

## `remote` — inspection and repository edits (issue #72)

| osc | geectl | Why |
|---|---|---|
| Overloaded commands (`osc ls`, `osc cat`, `osc log`, `osc meta`, `osc search` with modes) | One command per library operation (`geectl remote list-files`, `geectl remote cat`, `geectl remote package-log`, `geectl remote project-meta`, `geectl remote package-meta`, `geectl remote search-packages`, `geectl remote search-projects`, …) | One subcommand per operation, grouped under the `geectl remote` noun |
| `osc search` exact-name default | Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches | Exact search is a CLI-side filter; osc's default stays reachable via `--exact` |
| `osc rdiff` (server-side diff) | `geectl remote revision-patch` and `geectl remote project-diff`, computed client-side from per-file reads | The library lists and cats the files itself; a large `project-diff` issues many requests and a differing binary file fails the call |
| `osc rdiff` two-package form | `geectl remote diff-packages` | Client-side diff of two packages from per-file reads; a differing binary file fails the call |
| `osc develproject` (prints devel project + package) | `geectl remote devel-project <package> [--target-project <PRJ>]` | The target defaults to Factory; the library op returns the (project, package) pair. `geectl remote package-meta` shows the same pair as `devel: project/package`. |
| `osc repo add` / `osc repo remove` (project admin writes) | `geectl remote repo-add` / `geectl remote repo-remove` | Grouped under `geectl remote` with the other project reads; mutations take the global `--live` |
| — | `geectl remote upload-file` | New surface: a raw file upload is API-only in osc (`osc api -X PUT -T FILE <path>`); the local file is read from `--from`, `--binary` uploads bytes |
| `osc get` single file / `osc cat` (streamed read) | `geectl remote download` | Streams the file to `--output` instead of printing it; the rev passes verbatim (a number or a srcmd5), like `cat` — osc resolves the expanded srcmd5 by default. osc writes to a temp file and renames; geectl streams to the destination with exclusive create, so a failure mid-stream leaves a partial file for the caller to delete |
| — | `geectl remote create-project` | New surface: projects are created through `meta -e` in osc; the library creates the server meta directly |
| `osc rmkpac [--scmsync SCMSYNC] [--force] PROJECT PACKAGE` | `geectl remote create-package` | One command per library operation. `--scmsync` / `--force` select the options form of the creation |
| `osc token --create --operation OPERATION [PROJECT PACKAGE]` | `geectl remote create-token --operation OPERATION [--project PROJECT] [--package PACKAGE] [--description TEXT]` | One command per library operation; the operation is a required flag; project and package go together: both scope the token to a package; `--description` names the token, at most 64 characters. The secret prints once, at creation |
| `osc rdelete [-m TEXT] PROJECT PACKAGE` | `geectl remote delete-package` | One command per library operation; osc's `-m` is `--message`, sent as the server's `comment` |
| — | `geectl remote delete-link PROJECT PACKAGE` | Working-copy or API delete: a working copy deletes the `_link` file and commits it, or `osc api -X DELETE /source/PROJECT/PACKAGE/_link` deletes it directly; the library guards the delete behind dry-run |
| — | `geectl remote delete-file PROJECT PACKAGE FILE` | Working-copy or API delete: a working copy deletes the file and commits it, or `osc api -X DELETE /source/PROJECT/PACKAGE/FILE` deletes it directly; the library guards the delete behind dry-run |
| `osc meta prj PROJECT -e` | `geectl remote set-project-meta PROJECT [--from-file FILE] [--title T] [--description D] [--url U] [--scmsync S] [--devel-project P] [--add-person LOGIN:ROLE] [--remove-person LOGIN]` | Field-scoped write: only the given fields change, the rest of the meta is kept; osc edits the whole document in an editor. `--from-file` is the whole-file write (osc `meta -F`): the file's typed fields replace what the server holds |
| `osc meta pkg PROJECT PACKAGE -e` | `geectl remote set-package-meta PROJECT PACKAGE [--from-file FILE] [--title T] [--description D] [--url U] [--scmsync S] [--releasename R] [--devel-project P] [--devel-package K] [--add-person LOGIN:ROLE] [--remove-person LOGIN]` | Same; osc edits the whole document in an editor, geectl writes fields. `--from-file` is the whole-file write (osc `meta -F`): the file's typed fields replace what the server holds |
| `osc updatepacmetafromspec [WORKING-COPY] [--specfile FILE]` | `geectl remote update-package-meta-from-spec PROJECT PACKAGE [--spec FILE]` | Same update (spec Summary → title, Url → URL, %description → description); geectl takes project/package coordinates, osc takes a working copy |
| `osc rdelete [-r] [-f] [-m TEXT] PROJECT` | `geectl remote delete-project` | One command per library operation; `-r` is `--recursive`, `-f` is `--force`, `-m` is `--comment` |
| `osc undelete [-m TEXT] PROJECT PACKAGE` | `geectl remote undelete-package` | One command per library operation, one package per invocation; the restore files comment-free |
| `osc undelete [-m TEXT] PROJECT` | `geectl remote undelete-project` | One command per library operation; the restore files comment-free |
| `osc aggregatepac SOURCEPRJ SOURCEPAC DESTPRJ [DESTPAC]` | `geectl remote aggregate-package SOURCE-PROJECT SOURCE-PACKAGE TARGET-PROJECT TARGET-PACKAGE` | Grouped + spelled-out; the target package is required (osc defaults it to the source name). `-m SRC=TARGET` is repeatable `--repository-map SOURCE=TARGET`; `--nosources` is `--no-sources`, `-d` is `--disable-publish` |
| `osc release [SOURCEPROJECT SOURCEPACKAGE]` | `geectl remote release-package` | One command per library operation. `-r` is `--repository`, `-a` is `--arch`; `--target-project` / `--target-repository` / `--set-release` / `--no-delay` keep their names |
| `osc release [SOURCEPROJECT]` | `geectl remote release-project` | One command per library operation; same flag mapping as `release-package` |
| `osc lock PROJECT [PACKAGE]` | `geectl remote lock PROJECT [PACKAGE] [--message TEXT]` | One command per library operation; osc's `-m` stays `--message`, sent as the server's `comment`; a project lock records it (`cmd=lock`), a package lock sends it but the server does not record it (`set_flag` is the only package path) |
| `osc unlock PROJECT [PACKAGE]` | `geectl remote unlock PROJECT [PACKAGE] --message TEXT` | One command per library operation; the message is a required flag — osc prompts for it in an editor when `-m` is absent, geectl has no editor |
| `osc addchannels [PROJECT [PACKAGE]]` | `geectl remote add-channels PROJECT [PACKAGE] [--enable-all] [--skip-disabled]` | One command per library operation; `--enable-all` / `--skip-disabled` keep their names and exclude each other |
| `osc addcontainers PROJECT PACKAGE` | `geectl remote add-containers PROJECT PACKAGE [--extend-package-names]` | One command per library operation; the package is required, like osc |
| `osc ls /` | `geectl remote list-projects` | Same listing, named as a leaf under `geectl remote` |
| — | `geectl remote list-staging-projects PROJECT` | New surface: the library reads `/staging/<workflow>/staging_projects` |
| — | `geectl remote staging-project PROJECT NAME` | Same; the one-project read |
| `osc search -i LOGIN --project` | `geectl remote search-projects-by-person LOGIN` | Same search: the projects a person is involved in, by login |
| `osc whois [USER ...]` (aliases `who`, `user`) | `geectl remote whois [LOGIN]` | Same read; the bare form defaults to the authenticated user, like osc's configured-user default |
| `osc maintainer` (owner search, `osc/core.py` `owner()`) | `geectl remote owners PROJECT PACKAGE`, `geectl remote owners-for-binary BINARY`, `geectl remote owners-for-package PACKAGE` | One leaf per owner-search form: project/package, binary, and package-only; the package-only form searches the owner root projects, so a package in none of them returns no owners |
| — | `geectl remote count-scmsync-projects URL-PREFIX` | New surface: the scmsync-prefix project count is API-only in osc |
| — | `geectl remote list-scmsync-projects URL-PREFIX` | New surface: the scmsync-prefix project list is API-only in osc, like the count |

## `request` — submit requests (issue #73)

| osc | geectl | Why |
|---|---|---|
| `osc sr` / `submitrequest` (top-level command) | `geectl request file` | One noun per operation family: filing is a leaf under `geectl request`. The library op is `SrFiling::file` |
| `osc sr -s ID` / `--supersede` | `geectl request refile --supersede` | The file-first-then-supersede order is explicit in the name; a bare supersede is `geectl request supersede` |
| `osc mr` / `maintenancerequest` (top-level command) | `geectl request file-maintenance-incident`, `geectl request refile-maintenance-incident --supersede` | Incident filing is an `SrFiling` op, so it lives with the other filings under `geectl request` |
| `osc changedevelrequest` (`cr`, `changedevelreq`) | `geectl request file-change-devel` | Same grouping: a filing leaf under `geectl request` |
| `osc creq -a add_role USER ROLE PROJECT [PACKAGE]` (also `-a add_me` / `-a add_group`; `osc requestmaintainership` files the same action for the maintainer role) | `geectl request file-add-role` | Same grouping: a filing leaf under `geectl request`; the grantee is named by flags (`--person` / `--person-role`, `--group` / `--group-role`) |
| `osc deletereq` (top-level command, positional `PROJECT [PACKAGE]`) | `geectl request file-delete --target-project … [--package … \| --repository …]` | Same grouping; the target is selected by flags (package, repository, or neither for the whole project) |
| `osc deletereq --accept-in-hours` | `geectl request file-delete --accept-in-hours` | Name kept; the flag names the accept timer on the delete request |
| `osc sr` / `osc creq` with no `--cleanup` / `--no-cleanup` / `--no-update` (and no config) | `geectl request file` (and every filing) always sends `cleanup` unless `--without-cleanup` | Geectl's default is cleanup on every filing; osc sends `<sourceupdate>` only when told to or configured, matching osc's documented default for branch sources |
| `osc request show -d` | `geectl request diff` (+ opt-in `--xml`) | The diff is its own leaf; the XML view is an explicit opt-in |
| `osc review add` | `geectl request add-review --user/--group/--project/--package` | Verb-first leaf; the review target is named by flags |
| `osc review accept / decline / reopen [-U USER] [-G GROUP] [-P PROJECT [-p PACKAGE]] ID` | `geectl request change-review-state ID STATE --user/--group/--project/--package` | One leaf with the state as a positional; osc has one subcommand per state |
| `osc comment list/create request ID` (top-level `comment` command with a `request` target word) | `geectl request list-comments`, `geectl request add-comment [--parent …]` | Comments live under the request they belong to. Comments are API-only in osc. |
| `osc request list` | `geectl request search` | The library op is a typed search (`RequestSearch`); filters are named flags (`--creator`, `--state`, `--project`, …). Default states new/review/declined apply unless `--state` is given |
| `osc review list -U USER` | `geectl request search --reviewer USER` | The reviewer filter is a search flag: it matches open reviews addressed to the user. Unless `--state` is given, only new and review requests are listed, since a declined or accepted request keeps its open reviews. Reviewer filtering applies to user reviews. |
| `osc request supersede` | `geectl request supersede` acts immediately | The dry-runnable supersede flow is `geectl request refile` |

Unchanged: `geectl request show`, `geectl request log`, `geectl request accept`, `geectl request decline`,
`geectl request revoke` keep osc's names.
`geectl request change-state` / `geectl request try-change-state` follow the
kebab-case rule on the library operation names.

## `branch` — branch/copy/link (issue #76)

Only deliberate deviations; the table lists deviations only.

| osc | geectl | Why (consistency reason) |
|---|---|---|
| Flat top-level commands (`branch`, `copypac`, `linkpac`, …) | `geectl branch <subcommand>` group | Uniform grouping: one top-level noun per library module across all CLI groups |
| `osc branch` | `geectl branch branch-package` | Grouped + spelled-out operation name (kebab-case library name) |
| `osc branch --nodevelproject` | `--no-devel-project` | Kebab-case flag names everywhere (osc concatenates) |
| `osc branch -c/--checkout` | `geectl branch branch-package` then `geectl checkout PROJECT PACKAGE` | Branches AND checks out a working copy; geectl does the two steps as two commands — the branch, then a checkout of the new package |
| `osc copypac` | `geectl branch copy-package` | Grouped + spelled-out (osc abbreviates to `copypac`) |
| `osc copypac -c/--client-side-copy` | — (same-instance form) | Client-side download/upload copy of a single package; the cross-instance form exists as `--to-apiurl`; the same-instance copy stays server-side |
| `osc copypac -k/--keep-maintainers` | — | The server builds the target meta from the source meta |
| `osc copypac -t/--to-apiurl` | `geectl branch copy-package --to-apiurl <URL>` | Cross-instance copy, client-side download+upload; the target instance's credentials resolve from the instance alias matching the target URL (`GEECTL_OBS_USERNAME_<ALIAS>` / `GEECTL_OBS_PASSWORD[_FILE]_<ALIAS>`) and the copy fails clearly for an unconfigured target; the source credentials stay with the source instance |
| `osc linkpac` | `geectl branch link-package` | Grouped + spelled-out |
| `osc linkpac -c/--current` | — | Pins the link at the source's current expanded revision, which needs a directory read first; the library op takes an explicit revision |
| `osc linkpac -N/--new-package` | `--missing-target` | `--new-package` already means "branch a package missing from the target" in `geectl branch branch-package`; the library field is `missing_target` |
| `osc repairlink` | `geectl branch repair-link` | Same name, different operation: osc does a working-copy 3-way merge (checkout, merge, `resolved`, `ci`); geectl strips the failing patches from `_link` server-side and pins it at a revision where it expands. The cross-project form repairs the link in place. |
| `osc setlinkrev` | `geectl branch set-link-rev` | Grouped + kebab-case |
| `osc setlinkrev -r REV` (default: expanded) | `--revision <rev>` (bare = expanded) | Bare value keeps osc's default (expand); `pinned:<rev>` selects the plain form |
| `osc setlinkrev -R/--use-plain-revision` | `--revision pinned:<rev>` | The plain/expanded choice is part of the revision value; one flag per concept |
| `osc setlinkrev -u/--unset` | `--revision unset` | Unset is a revision value |
| `osc setlinkrev --vrev` | — | The library derives vrev from the directory fetch for md5 revisions |
| `osc setlinkrev` with `-r` omitted (current revision, expanded) | `--revision expanded` | Same default, spelled out as a value |
| `osc setlinkrev -R` with `-r` omitted (current revision, plain) | `--revision latest` | Same default, spelled out as a value |
| `osc setdevelproject` | `geectl branch set-devel-project` | Grouped + kebab-case |
| `osc linktobranch` | `geectl branch link-to-branch` | Grouped + kebab-case |
| `osc detachbranch` | `geectl branch detach-branch` | Grouped + kebab-case |

Additions beyond osc: `geectl branch copy-project`;
`geectl branch set-link-rev-project`;
`geectl branch copy-package` also exposes `--noservice`, `--dont-update-source`,
`--with-vrev` and `geectl branch link-package` exposes `--vrev` — library options
beyond osc.

## `build` — build status (issue #71)

| osc | geectl | Why |
|---|---|---|
| `results -w/--watch` | `geectl build wait-for-results --deadline-secs N` | Waiting is a separate deadline-bounded operation; osc's watch polls indefinitely |
| `results -r/--repo` | `geectl build results --repository` | Full word, one name per concept (`repo` vs `repository` unified to `repository`) |
| `results -s/--status-filter` | `geectl build results --code` | Flag named after the library query field |
| `results -f/--failed` | Use `--code failed` | The failed filter is a `--code` value |
| `results -l/--last-build` | `geectl build results --lastbuild` | Library field name |
| `results -M/--multibuild-package FLAVOR` | Pass `pkg:flavor` in `--package` | The flavor travels in the package value, per the library |
| `results --no-multibuild` | — | Multibuild is opt-in in the library (`--multibuild` enables it) |
| `results -b/--brief, -V/--vertical, -F/--fail-on-error, --show-excluded` | — | Presentation variants; text rendering follows the library |
| `results [PROJECT [PACKAGE]]` | `geectl build results PROJECT` + `--package` flag | Positionals are required coordinates only; the optional package is a flag |
| `prjresults [PROJECT]` | Use `geectl build results PROJECT` | The same matrix for the whole project; one matrix command |
| `buildlog` | `geectl build build-log` | Kebab-case library name under the `geectl build` group; the mapping stays mechanical (group + kebab(library op)) |
| `buildlog [REPOSITORY ARCH \| BUILDLOGURL]` | `geectl build build-log PROJECT PACKAGE REPOSITORY ARCH`, or `geectl build build-log BUILDLOGURL` | Explicit coordinates, or a single build-log URL (API or web-UI form) |
| `buildlog` tail via `blt` alias | — | The library reads whole logs |
| `buildlog --lastsucceeded` / `--last-succeeded` (two spellings) | `--lastsucceeded` | One spelling, the library field name |
| `buildlog -M/--multibuild-package FLAVOR` | `geectl build build-log --multibuild FLAVOR` | The flavor appends `:FLAVOR` to the package |
| `buildlog -o/--offset` | `geectl build build-log --start` | Flag named after the library query field |
| `buildlog -s/--strip-time` | `geectl build build-log --strip-time` | Client-side strip of the leading `[timestamp] ` from each log line |
| `buildinfo [REPOSITORY ARCH [BUILD_DESCR]]` | `geectl build buildinfo PROJECT PACKAGE REPOSITORY ARCH` | Explicit coordinates; the library reads the server-side spec |
| `buildconfig REPOSITORY (in pkg or prj dir) \| buildconfig PROJECT REPOSITORY` | `geectl build buildconfig PROJECT REPOSITORY` | Explicit coordinates; project is always required; the config text renders verbatim, coordinates in the payload |
| `buildhistory` | `geectl build build-history` | Kebab-case library name, same mechanical mapping as `geectl build build-log` |
| `jobhistory [PROJECT [PACKAGE] REPOSITORY ARCH]` | `geectl build job-history PROJECT REPOSITORY ARCH` + `--package` flag | Positionals are required coordinates only |
| `repos [PROJECT] [PACKAGE]` | `geectl build list-repositories PROJECT` | Verb-noun `list-*` scheme for listings; the library reads project meta |
| `getbinaries …` | `geectl build list-binaries` + `geectl build download-binary` | One osc command split into list vs download, mirroring the library split |
| `getbinaries` implicit cwd/subdir writes | `geectl build download-binary --output PATH` (required) | Explicit destination |
| `getbinaries PROJECT PACKAGE REPOSITORY ARCH` (whole package) | `geectl build download-binary --all --output DIR PROJECT PACKAGE REPOSITORY ARCH` | One list call plus one streaming download per file, into the named directory; downloads every listed file, including the `.src.rpm` and `-debuginfo` files; one invocation covers one architecture |
| `rpmlintlog` | Use `geectl build download-binary … rpmlint.log --output …` | One generic download instead of per-artifact shortcuts |
| `whatdependson REPOSITORY ARCH` | `geectl build whatdependson PROJECT PACKAGE REPOSITORY ARCH` | Explicit coordinates; package is a required positional per the library signature |
| `osc rebuild [PROJECT [PACKAGE]]` | `geectl build rebuild-package PROJECT PACKAGE REPOSITORY ARCH` | One command per library operation, explicit coordinates; per-package only |
| `osc restartbuild [PROJECT [PACKAGE]]` | `geectl build restart-build PROJECT PACKAGE REPOSITORY ARCH` | Same mapping; per-package only |
| `osc abortbuild [PROJECT [PACKAGE]]` | `geectl build abort-build PROJECT PACKAGE REPOSITORY ARCH` | Same mapping; per-package only |
| `osc wipebinaries PROJECT [PACKAGE]` | `geectl build wipe-binaries PROJECT [PACKAGE] [--arch ARCH] [--repository REPO] [--all] [--build-disabled] [--build-failed] [--broken] [--unresolvable]` | One command per library operation; `-r` is `--repository`, `-a` is `--arch`. `--all` takes no status filter (osc silently drops it when a filter is given); an unscoped wipe is refused |

`geectl build build-status`, `geectl build job-status`, `geectl build fileinfo`,
`geectl build dependson`, `geectl build check-constraints`, `geectl build list-binaries` — new surface, reachable in osc
via `osc api`.

## `local-build` — podman build vertical (issue #43)

| osc | geectl | Why |
|---|---|---|
| `build` | `geectl local-build build DIRECTORY --project PROJECT --repo REPOSITORY` | One operation: the run-mode service run, the precommit gate, then the engine build in one invocation; project and repo are explicit coordinates, arch defaults to the host arch; the options are `--arch`, `--clean`, `--flavor`, `--release`, `--trust-all-projects`, `--refresh-image`, `--timeout-secs`, `--alternative-project`, `--root`, `--noinit`, `--preinstall-image`, `--keep-rpms`, `--prefer-packages` |
| `osc wipe [REPOSITORY ARCH]` (alias of `build`, sets `--wipe`) | `geectl local-build wipe REPOSITORY ARCH SPEC --project PROJECT` | Wipes the root the build's own coordinates name; the package comes from the spec file in the source directory, like the build; `--root` names the root directly; the marker check and the in-use lock are safety invariants |
| `chroot [REPOSITORY [ARCH]]` (alias of `build`, in-directory form) | `geectl local-build chroot DIRECTORY --project PROJECT --repo REPOSITORY` | Enters the preserved root of the last build for the package; the package comes from the spec file in the directory, like the build; arch defaults to the host arch; the directory, project, and repo are explicit coordinates. Repro commands come from stdin, one per line, joined with `&&` into one shell command; a failing repro exits 0, with the exit code in `exit:` / `record.result.exit_code`; the shell is non-interactive |
| `osc lbl [REPOSITORY [ARCH]]` | `geectl local-build log REPOSITORY ARCH SPEC --project PROJECT` | Reads the `<root>/.build.log` the build's coordinates name; the package comes from the spec file in the source directory, like the wipe; the text is the log verbatim, the verdict and truncation flag ride in the JSON payload; `--tail N` prints only the last N lines, `--full` reads the whole log; the default read caps at 4 MiB |

`--trust-all-projects` trusts the dep set the buildinfo enumerates as a whole: foreign `home:` projects in the set are accepted for the run; non-`home:` projects and the caller's own `home:` are trusted already. Trust in geectl is flag-wide; the flag alone decides trust for the run. The accepted foreign
`home:` projects are recorded in the build record
(`trusted_foreign_projects`); a dry run records the flag in the plan.

## `maintenance` — maintained instances + branching (issue #69)

| osc | geectl | Why |
|---|---|---|
| `osc mbranch [SOURCEPACKAGE [TARGETPROJECT]]` (flat top-level command, positionals) | `geectl maintenance maintenance-branch --package … [--target-project …]` | Grouped under the maintenance noun; package and target are named flags |
| `osc maintained` / `osc sm` (aliases of `mbranch`, list-only) | `geectl maintenance maintained-instances --package …` | The list form is its own leaf. Semantics match for `sm`: both are the dry-run branch call. The `[git] org/package:branch` lines from the Gitea cache are Gitea scope. |
| `osc mbranch -c/--checkout` | `geectl maintenance maintenance-branch` then `geectl checkout PROJECT PACKAGE` | Branches AND checks out a working copy; geectl does the two steps as two commands — the branch, then a checkout of the new package |
| `osc mbranch --version` (print version of the maintained package) | — | The top-level `geectl version` command reports geectl itself |

Unchanged: `--attribute` and
`--update-project-attribute` keep osc's names and defaults
(`OBS:Maintained` / `OBS:UpdateProject`). The default branch project
`home:<user>:branches:OBS_Maintained:<package>` matches osc's mbranch
layout. Incident filing lives with the other filings under `geectl request`
(`geectl request file-maintenance-incident`).

## `service` — service reads and trigger

| osc | geectl | Why |
|---|---|---|
| — | `geectl service list` / `geectl service state` | New surface: read a package's `_service` file and its last run state |
| `osc service remoterun [PROJECT PACKAGE]` | `geectl service trigger PROJECT PACKAGE [--service NAME]` | Same server-side trigger (`POST /source/<project>/<package>?cmd=runservice`): the server runs the package's whole service set; `--service` names the service the trigger is for; a service name outside the stored `_service` fails closed. The trigger queues the run; the outcome is read with `geectl service state`. Dry-run is the default (the plan is printed); `--live` performs the trigger |

## `gitea` — Gitea operations

| osc | geectl | Why |
|---|---|---|
| — | `geectl gitea repo-get` / `geectl gitea repo-list` / `geectl gitea branch-list` / `geectl gitea branch-get` / `geectl gitea branch-head` | New surface: Gitea reads on the configured instance |
| — | `geectl gitea repo-create` | New surface: create a repository on the configured instance; mutations take the global `--live` |
| `osc api -X DELETE /repos/...` | `geectl gitea repo-delete` | The delete op under the `geectl gitea` noun; mutations take the global `--live`; the library requires `--force` |
| `osc fork` | `geectl branch branch` | `osc fork` is a composite: Gitea fork + OBS branch. `geectl branch branch` is the composite: repo fork, OBS fork into a test project with the source project's repositories, and the first-build report. `geectl gitea repo-fork` is the Gitea half only. `osc fork` is scmsync-only; it clones the fork locally and inits `.osc` |

Scope note: the sibling `git-obs` entry point carries its own `repo list` and `repo fork` commands; `git-obs repo list` requires `--org`/`--user`.

## Working copy — git working copy

| osc | geectl | Why |
|---|---|---|
| `osc checkout` / `co` / `getpac` / `bco` `PROJECT PACKAGE` | `geectl checkout PROJECT PACKAGE [--rev REV]` | Same checkout, different metadata: osc records the checkout in `.osc`; geectl checks the package out as a git working copy under `<dir>/<project>/<package>/` and records the base in git config (`geectl.apiurl`, `geectl.project`, `geectl.package`, `geectl.backend`, `geectl.base-rev`, `geectl.base-srcmd5`). A git-backed package is a real clone; an OBS-native package gets a synthesized repo with the base as its root commit. `geectl checkout` streams every listed file to disk |
| `osc update` / `up` | `geectl update [--rev REV]` | Same update, different merge: osc merges the new revision into the working copy; geectl rebases the local work onto the new base, and a conflict stays in the tree for the agent to resolve |
| `osc status` / `st` | `geectl status` | Same state read; the one-letter codes are `A` (added), `M` (modified), `D` (deleted), `?` (untracked), `!` (missing), `C` (conflict), `R` (renamed) |
| `osc diff` | `geectl diff` | The diff covers the worktree and the index against the base, rendered verbatim |
| `osc add FILE...` | `geectl add FILE...` | Same stage |
| `osc delete` / `rm FILE...` | `geectl rm FILE...` | Same remove |
| `osc addremove` | `geectl addremove` | Same stage-new-and-deleted: stages new files and worktree deletions; modifications to tracked files still need `geectl add` |
| `osc commit` / `ci` / `checkin` | `geectl commit --message MSG [--noservice]` | Same commit, plus a hard gate: the source services and `pre_checkin.sh` run before the send, unless `--noservice` is passed. The send is the staged-plus-committed state against the base; an unstaged edit to a file the service gate changed is staged whole with the gate's output |
| `osc pull` | `geectl pull` | Linked packages only: pulls the link target's latest revision into the checkout; a clean working copy (all changes committed, all files tracked) and a recorded baserev are required |
| `osc resolved` | `geectl resolve` | Stages the resolved files; when a rebase is in progress and every conflict is resolved, continues it |
| `osc mkpac PROJECT PACKAGE` | `geectl remote create-package` then `geectl checkout PROJECT PACKAGE` | `mkpac` creates the package AND checks it out; the create and the checkout are two commands |
| `osc vc` | `geectl changelog` (separate layer) | The changelog validation/generation layer is its own surface |
