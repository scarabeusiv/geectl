# geectl CLI usage

The command reference for the geectl binary: one subcommand per
operation, grouped under a top-level noun (`remote`, `request`,
`branch`, `build`, `local-build`, `maintenance`, `service`, `gitea`, `pull-request`, `person`, `meta`, `changelog`, `prjconf`), plus `version`
and `config`, plus the ten working-copy leaves (`checkout`, `update`,
`status`, `diff`, `add`, `rm`, `addremove`, `commit`, `pull`,
`resolve`) that operate on the checkout in the current directory.
The reference below lists every command with what it does
and its key flags. The binary's `--help` is the source of
truth for the full text: run `geectl --help` or
`geectl <group> --help` for it.

## Invocation shape

```sh
geectl [--format json|text] [--debug] [--debug-body] [--live] <group> <command> [args] [flags]
```

Required coordinates are positionals where the leaf takes them; the build group orders them project, package, repository, arch. Branch and maintenance take coordinates as long flags. The exact order per leaf is in its `--help` output. Everything else is a named long flag. Group/leaf options take long flags only, apart from clap's built-in `-h`/`-V`, `commit -m`, and `local-build build -k`/`-p`. Each command has a single name.

`--format` selects the stdout payload: `json` (default — the format agents parse) or `text` (a human rendering of the same data). The same invocation produces the same bytes in every context. A persistent default comes from `GEECTL_FORMAT`, else the `format` key in the config file; `--format` wins over both. Agent invocations pass `--format json` explicitly.

`--debug` prints human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions); `GEECTL_DEBUG=1` enables it without the flag. `--debug-body` additionally prints request and response bodies (capped, redacted, header-free). Debug leaves stdout alone.

Mutations are dry-run by default: without `--live` only the plan is printed (on stderr as it is recorded, and in the payload as `dry_run: true` with a `plan`). Pass the global `--live` to perform the mutation. `request supersede` needs `--live`: it performs the supersede directly. The dry-runnable supersede flow is `request refile --supersede`.

`remote set-project-meta` in dry-run validates no repositories: it performs no fetch and reads every carried repository as pre-existing; `--live` validates only the repositories it changes.

`request file-delete` deletes a whole project only with `--whole-project`: omitting `--package` and `--repository` without it refuses.

Errors go to stderr in the selected format (JSON payload, or `error:` + `remediation:` lines in text mode) with the error kind's exit code. A parse error stays JSON.

Text renderings open with one header line echoing the query coordinates (`project: … package: … …`); tables follow as designed. Content commands (`remote cat`, `remote revision-patch`, `remote project-diff`, `remote diff-packages`, `build build-log`, `request diff`, `gitea branch-head`) render verbatim, opening with the content — the coordinates live in the JSON payload. Two renderings open with the content directly too: `remote list-projects` (the query names the instance only) and the request action texts (one-line outcome summaries; the plan is on stderr and in the payload).

### Credentials in dry-run

Credentials are stored per instance, named by an alias: the config
file names each instance once in an `[instances]` table mapping
alias to URL, and the credential variables use the alias uppercased:

```toml
[instances]
example = "https://api.opensuse.org"
local = "http://127.0.0.1:3001"
```

```sh
GEECTL_OBS_USERNAME_EXAMPLE=... GEECTL_OBS_PASSWORD_EXAMPLE=...
GEECTL_OBS_USERNAME_LOCAL=... GEECTL_OBS_PASSWORD_FILE_LOCAL=/run/secrets/local-password
GEECTL_OBS_APIKEY_EXAMPLE=...   # a token instead of the pair
```

The plain `GEECTL_OBS_APIKEY` (or `..._APIKEY_FILE`) or
`GEECTL_OBS_USERNAME` / `GEECTL_OBS_PASSWORD` (or
`..._PASSWORD_FILE`) apply to the configured instance; a call to
another instance resolves the alias whose configured URL matches
it (same scheme, host, and effective port — `https://host` and
`https://host:443` resolve to one instance; `http://host:3000` and
`https://host:3000` resolve to two) and uses that alias's variables. A
call to another instance with zero configured credentials
fails naming what to set — the alias variables for a matching
alias, the `[instances]` entry to add otherwise — and
the configured instance's credentials stay with it.
`branch copy-package --to-apiurl` resolves the target's
credentials this way.

The same rule applies to Gitea: `GEECTL_GITEA_TOKEN_<ALIAS>` (or
`GEECTL_GITEA_TOKEN_FILE_<ALIAS>`) for the instance named in
`[instances]`, e.g. `GEECTL_GITEA_TOKEN_LIVE` for an instance named
`live`; the plain `GEECTL_GITEA_TOKEN` (or
`GEECTL_GITEA_TOKEN_FILE`) applies to the configured Gitea URL only.

A half-configured credential pair (username only, or password
only) is handled per group. `branch`, `maintenance`, and
`service` fail fast on it before any work — `branch` and `maintenance`
in dry-run too, since a dry run previews a live run
(`service`'s trigger is the mutation there; its reads are public
but resolve credentials up front).
`request` swallows it in dry-run (a preview performs no write;
the dropped error goes to the debug channel). `meta` takes the same leniency on its OBS arm: a dry-run `meta set --project` prints the plan while any credential-resolution error (a half-configured pair or a conflicting set) is reported on the debug channel and as a stderr warning. The Gitea arm resolves its token up front: a dry-run `meta set --repo` reports a missing token as a configuration error. `remote` and `build`
resolve credentials per command instead: the public reads (see
Credentials in `docs/architecture.md`) skip credentials, so a half-set
env leaves them working; a dry-run `remote` mutation
preview leaves them working too; every other command in those groups
fails fast on a half-set pair up front. `remote list-files --include-deleted` resolves
credentials too: the deleted listing reads the authenticated route.
Setting both a token and a Basic pair for one instance
is a configuration error at the same resolution points.

### `.env` file

A `.env` file in the working directory supplies `GEECTL_*` variables
without exporting them, when the run opts in with `GEECTL_DOTENV=1` in
the real process environment: geectl loads it at startup and merges it into
the environment snapshot, with a real environment variable winning
over the same key in the file. Only `GEECTL_*` keys are honoured. A
key starting with `GEEC` that fails the `GEECTL_*` shape, or a
`GEECTL_*` key carrying anything outside ASCII letters, digits, or `_` after
the prefix, is a typo for a geectl variable and fails as a config
error. The syntax is minimal —
`KEY=value`, `#` comments, blank lines, an optional `export` prefix
followed by whitespace (a key glued to `export` is malformed), and optional single or double quotes — values stay literal, one line each. A duplicate key keeps
its last value, and quoted values are literal inside the quotes. The file must be a regular file, at most 64 KiB, and
readable only by its owner (`chmod 600`) on Unix — on non-Unix the
mode goes unchecked, and the opt-in is the guard there. A missing
file is fine, and so is a file with zero `GEECTL_*` keys,
whatever its mode or syntax — but an unreadable, malformed, or
group/other-readable file that carries `GEECTL_*` keys fails at
startup as a config error naming the file (and the line, for a
malformed line) only. `--version` and `--help` print and
exit before the file is loaded, so they ignore it. `--debug`
reports a `.env`
load; the values stay in the file. Values from the file report their
source as `environment` in `config show`. The load covers the working
directory's `.env` only.

## Top level

- `geectl remote` — Remote reads and mutations: inspect what the distribution holds, and change it.
- `geectl request` — Read and act on OBS submit requests, and file new ones.
- `geectl branch` — Branch, copy, and link packages.
- `geectl build` — Build-status commands: results, logs, histories, binaries, buildconfig.
- `geectl local-build` — Local builds in the podman build vertical: build a package, run its `_service` services, the precommit gate, `spec-cleaner`, wipe a build root, prune the cache, manage the daily periodic cleanup, enter the preserved build root, and read the log of the last local build.
- `geectl maintenance` — Maintenance operations: maintained-instances listing and maintenance branching.
- `geectl service` — Service reads and trigger: list a package's services, their run state, and trigger a server-side run.
- `geectl gitea` — Gitea operations: repository and branch reads and mutations on the configured instance.
- `geectl pull-request` — Read and act on pull requests: OBS submit requests and Gitea pull requests as one surface.
- `geectl person` — Search for a person: the OBS projects naming the login and the Gitea users matching the keyword, as one operation.
- `geectl meta` — Set the meta of an OBS project, an OBS package, or a Gitea repository as one operation: the target selects the backend.
- `geectl changelog` — Validate and generate `.changes` entries in a package directory, pre-file.
- `geectl prjconf` — Read, validate and clean a project's prjconf.
- `geectl version` — Print version information as the result payload.
- `geectl config` — Inspect or initialise the config file.
- `geectl config show` — Show the effective config: endpoint URLs with their source, and credential status (never a secret value).
- `geectl config init` — Write the config template (endpoints only); refuses if the file already exists.

## `remote` — inspection and mutations

- `geectl remote cat <PROJECT> <PACKAGE> <FILE>` — Print a source file to standard output; key flags: --rev <REV>
- `geectl remote download --output <OUTPUT> <PROJECT> <PACKAGE> <FILE>` — Download a source file to a local path; key flags: --rev <REV>
- `geectl remote list-files <PROJECT> <PACKAGE>` — List the files of a package; key flags: --rev <REV>, --include-deleted
- `geectl remote list-packages <PROJECT>` — List the packages of a project.
- `geectl remote list-projects` — List the top-level projects on the instance.
- `geectl remote list-staging-projects <PROJECT>` — List the staging projects of a workflow project; key flags: --no-requests, --no-status
- `geectl remote staging-project <PROJECT> <NAME>` — Show one staging project of a workflow project; key flags: --no-requests, --no-status
- `geectl remote package-log <PROJECT> <PACKAGE>` — Show the commit history of a package; key flags: --range <RANGE>, --limit <LIMIT>
- `geectl remote revision-patch <PROJECT> <PACKAGE> <REV>` — Show the per-file diff of one revision against the previous one.
- `geectl remote project-meta <PROJECT>` — Show a project's meta (title, description, persons, repositories).
- `geectl remote set-project-meta <PROJECT>` — Set a project's meta, field by field or whole from a file; key flags: --from-file <FROM_FILE>, --title <TITLE>, --description <DESCRIPTION>, --devel-project <DEVEL_PROJECT>, --add-person <ADD_PERSON>, --remove-person <REMOVE_PERSON>
- `geectl remote package-meta <PROJECT> <PACKAGE>` — Show a package's meta (title, description, persons, devel project).
- `geectl remote set-package-meta <PROJECT> <PACKAGE>` — Set a package's meta, field by field or whole from a file; key flags: --from-file <FROM_FILE>, --title <TITLE>, --description <DESCRIPTION>, --devel-project <DEVEL_PROJECT>, --devel-package <DEVEL_PACKAGE>, --add-person <ADD_PERSON>, --remove-person <REMOVE_PERSON>
- `geectl remote update-package-meta-from-spec <PROJECT> <PACKAGE>` — Update a package's meta from its spec file; key flags: --spec <SPEC>
- `geectl remote search-packages [TERM]` — Search packages by name; key flags: --exact, --devel-project <DEVEL_PROJECT>
- `geectl remote search-projects <TERM>` — Search projects by name; key flags: --exact
- `geectl remote search-projects-by-person <USERID>` — Search projects by a person's login: every project whose meta names the login in a role.
- `geectl remote search-packages-by-person <USERID>` — Search packages by a person's login: every package whose meta names the login in a role.
- `geectl remote count-scmsync-projects <URL_PREFIX>` — Count the projects syncing from a git URL prefix.
- `geectl remote list-scmsync-projects <URL_PREFIX>` — List the projects syncing from a git URL prefix; key flags: --limit <LIMIT>, --offset <OFFSET>
- `geectl remote owners <PROJECT> <PACKAGE>` — Show the owners of a package (project/package scope).
- `geectl remote owners-for-binary <BINARY>` — Show the owners of a binary package.
- `geectl remote owners-for-package <PACKAGE>` — Show the owners of a package by name alone, in the owner root projects.
- `geectl remote maintainers <PROJECT> <PACKAGE>` — Show the maintainers of a package.
- `geectl remote whois [LOGIN]` — Show a user's full name, email, and account state.
- `geectl remote link-target <PROJECT> <PACKAGE>` — Show the link target of a package, when it is a link.
- `geectl remote project-diff <OLD> <NEW>` — Diff two projects, package by package; key flags: --include <INCLUDE>, --exclude <EXCLUDE>, --annotate-requests
- `geectl remote diff-packages <OLD_PROJECT> <OLD_PACKAGE> <NEW_PROJECT> <NEW_PACKAGE>` — Diff two packages, file by file.
- `geectl remote devel-project <PACKAGE>` — Show the devel project and devel package of a package, when it has one; key flags: --target-project <TARGET_PROJECT>
- `geectl remote repo-add --arch <ARCH> --path <PATH> <PROJECT> <NAME>` — Add a repository to a project; key flags: --disable-publish
- `geectl remote repo-remove <PROJECT> <NAME>` — Remove a repository from a project.
- `geectl remote upload-file --from <FROM> <PROJECT> <PACKAGE> <FILE>` — Upload a file into a package; key flags: --binary
- `geectl remote create-project --title <TITLE> --description <DESCRIPTION> <PROJECT>` — Create a project.
- `geectl remote create-package --title <TITLE> --description <DESCRIPTION> <PROJECT> <PACKAGE>` — Create a package; key flags: --scmsync <SCMSYNC>, --force
- `geectl remote create-token --operation <OPERATION>` — Create an operation-scoped trigger token for the authenticated user; key flags: --project <PROJECT>, --package <PACKAGE>, --description <DESCRIPTION>
- `geectl remote delete-package <PROJECT> <PACKAGE>` — Delete a package; key flags: --force, --message <MESSAGE>
- `geectl remote delete-link <PROJECT> <PACKAGE>` — Delete a `_link` file from a package.
- `geectl remote delete-file <PROJECT> <PACKAGE> <FILE>` — Delete a file from a package.
- `geectl remote delete-project <PROJECT>` — Delete a project; key flags: --recursive, --force, --comment <COMMENT>
- `geectl remote undelete-package <PROJECT> <PACKAGE>` — Restore a deleted package.
- `geectl remote undelete-project <PROJECT>` — Restore a deleted project.
- `geectl remote aggregate-package <SOURCE_PROJECT> <SOURCE_PACKAGE> <TARGET_PROJECT> <TARGET_PACKAGE>` — Aggregate a package's binaries into another package; key flags: --repository-map <REPOSITORY_MAP>, --no-sources, --disable-publish
- `geectl remote release-package <PROJECT> <PACKAGE>` — Release a package's sources and binaries; key flags: --repository <REPOSITORY>, --arch <ARCH>, --target-project <TARGET_PROJECT>, --target-repository <TARGET_REPOSITORY>, --set-release <SET_RELEASE>, --no-delay
- `geectl remote release-project <PROJECT>` — Release a whole project's sources and binaries; key flags: --repository <REPOSITORY>, --arch <ARCH>, --target-project <TARGET_PROJECT>, --target-repository <TARGET_REPOSITORY>, --set-release <SET_RELEASE>, --no-delay
- `geectl remote lock <PROJECT> [PACKAGE]` — Lock a project or a package; key flags: --message <MESSAGE>
- `geectl remote unlock <PROJECT> [PACKAGE] --message <MESSAGE>` — Unlock a project or a package
- `geectl remote add-channels <PROJECT> [PACKAGE]` — Add the channels defined for a project or a package; key flags: --enable-all, --skip-disabled
- `geectl remote add-containers <PROJECT> <PACKAGE>` — Add the maintained containers for a package; key flags: --extend-package-names

## `request` — submit requests

- `geectl request show <ID>` — Show one submit request: source/target/message, reviews, history.
- `geectl request log <ID>` — Show one submit request's history: creation, then each state change.
- `geectl request search` — List the submit requests matching the filters: a typed search; key flags: --creator <CREATOR>, --reviewer <REVIEWER>, --state <STATE>, --project <PROJECT>, --package <PACKAGE>, --type <TYPES>, --since <SINCE>, --limit <LIMIT>
- `geectl request list` — List submit requests: the same search, with the states still awaiting action (new, review) as the default; key flags: --project <PROJECT>, --package <PACKAGE>, --state <STATE>, --since <SINCE>, --limit <LIMIT>
- `geectl request diff <ID>` — Show the diff of a submit request: the unified diff, or the request XML with the diff injected with `--xml`; key flags: --xml
- `geectl request list-comments <ID>` — List the comments on a submit request, in id order.
- `geectl request accept <ID>` — Accept a submit request, with a comment as the message left with the accept. An accepted request is forwarded to its link parent unless `--no-forward` is given; a refused forward is reported and the accepted request stands, with the work staying queued in the devel project; key flags: --comment <COMMENT>, --force, --no-forward
- `geectl request decline <ID>` — Decline a submit request, with a comment as the reason; key flags: --comment <COMMENT>
- `geectl request revoke <ID>` — Revoke (withdraw) a submit request, with a comment as the reason; key flags: --comment <COMMENT>
- `geectl request change-state <ID> <STATE>` — Change a submit request's state to one of new/review/accepted/declined/revoked; key flags: --comment <COMMENT>, --force
- `geectl request try-change-state <ID> <STATE>` — Best-effort state change: try it, and report whether it happened; key flags: --comment <COMMENT>, --force
- `geectl request add-review --comment <COMMENT> <ID>` — Ask a user, group, project, or package to review a submit request; key flags: --user <USER>, --group <GROUP>, --project <PROJECT>, --package <PACKAGE>
- `geectl request change-review-state <ID> <STATE>` — Change the state of one review on a submit request: accept it, decline it, or reopen it to new; key flags: --user <USER>, --group <GROUP>, --project <PROJECT>, --package <PACKAGE>, --comment <COMMENT>
- `geectl request add-comment --body <BODY> <ID>` — Comment on a submit request, optionally as a reply to another comment; key flags: --parent <PARENT>
- `geectl request file --source-project <SOURCE_PROJECT> --source-package <SOURCE_PACKAGE> --target-project <TARGET_PROJECT> --target-package <TARGET_PACKAGE> --description <DESCRIPTION> --changes-entry <CHANGES_ENTRY>` — File a submit request: source package to a devel project, superseding every open submit request for the package from the same user; key flags: --cve <CVE>, --without-cleanup
- `geectl request refile --source-project <SOURCE_PROJECT> --source-package <SOURCE_PACKAGE> --target-project <TARGET_PROJECT> --target-package <TARGET_PACKAGE> --description <DESCRIPTION> --changes-entry <CHANGES_ENTRY> --supersede <SUPERSEDE>` — File a submit request and supersede an old one with it, plus every other open submit request for the package from the same user: the new request is filed first, and only then is each open one superseded pointing at it; key flags: --cve <CVE>, --without-cleanup
- `geectl request supersede <OLD_ID> <NEW_ID>` — Supersede one request with another existing one. Needs `--live`.
- `geectl request file-add-role --target-project <TARGET_PROJECT> --description <DESCRIPTION>` — File an add-role request: grant a person or a group a role on a project or a package; key flags: --target-package <TARGET_PACKAGE>, --person <PERSON>, --person-role <PERSON_ROLE>, --group <GROUP>, --group-role <GROUP_ROLE>
- `geectl request file-change-devel --source-project <SOURCE_PROJECT> --source-package <SOURCE_PACKAGE> --target-project <TARGET_PROJECT> --target-package <TARGET_PACKAGE> --description <DESCRIPTION>` — File a change-devel request: change a package's devel project.
- `geectl request file-delete --target-project <TARGET_PROJECT> --description <DESCRIPTION>` — File a delete request: delete a package, a repository, or a whole project; key flags: --package <PACKAGE>, --repository <REPOSITORY>, --whole-project, --accept-in-hours <ACCEPT_IN_HOURS>
- `geectl request file-maintenance-incident --source-project <SOURCE_PROJECT> --target-project <TARGET_PROJECT> --description <DESCRIPTION>` — File a maintenance incident request: ask the maintenance team to start a maintenance incident from a maintenance update; key flags: --source-package <SOURCE_PACKAGE>, --release-project <RELEASE_PROJECT>, --with-cleanup
- `geectl request refile-maintenance-incident --source-project <SOURCE_PROJECT> --target-project <TARGET_PROJECT> --description <DESCRIPTION> --supersede <SUPERSEDE>` — File a maintenance incident request and supersede an old one with it: the new request is filed first, and only then is the old one superseded pointing at it; key flags: --source-package <SOURCE_PACKAGE>, --release-project <RELEASE_PROJECT>, --with-cleanup

## `branch` — branch, copy, link

- `geectl branch branch-package --project <PROJECT> --package <PACKAGE>` — Branch a package: create a linked copy under a branch project; key flags: --target-project <TARGET_PROJECT>, --target-package <TARGET_PACKAGE>, --add-repositories, --maintenance, --revision <REVISION>, --force, --new-package
- `geectl branch copy-package --source-project <SOURCE_PROJECT> --source-package <SOURCE_PACKAGE> --target-project <TARGET_PROJECT> --target-package <TARGET_PACKAGE>` — Copy a package to another project (or another OBS instance with `--to-apiurl`); key flags: --revision <REVISION>, --keep-link, --expand, --comment <COMMENT>, --noservice, --to-apiurl <TO_APIURL>
- `geectl branch copy-project --source-project <SOURCE_PROJECT> --target-project <TARGET_PROJECT>` — Copy a whole project; key flags: --with-binaries, --with-history, --noservice, --comment <COMMENT>
- `geectl branch set-devel-project --project <PROJECT> --package <PACKAGE>` — Set (or, without `--devel-project`, remove) the devel project of a package; key flags: --devel-project <DEVEL_PROJECT>, --devel-package <DEVEL_PACKAGE>
- `geectl branch link-package --source-project <SOURCE_PROJECT> --source-package <SOURCE_PACKAGE> --target-project <TARGET_PROJECT> --target-package <TARGET_PACKAGE>` — Link a package to another package; key flags: --force, --revision <REVISION>, --vrev <VREV>, --missing-target, --disable-build, --disable-publish
- `geectl branch repair-link --project <PROJECT> --package <PACKAGE>` — Repair a broken link: strip the failing patches from `_link` server-side and pin it at a revision where it expands.
- `geectl branch set-link-rev --project <PROJECT> --package <PACKAGE> --revision <REVISION>` — Set the revision a link expands at.
- `geectl branch set-link-rev-project --project <PROJECT> --revision <REVISION>` — Set the revision every linked package in a project expands at.
- `geectl branch link-to-branch --project <PROJECT> --package <PACKAGE>` — Turn a link into a branch: replace the link with a copy of the linked files.
- `geectl branch detach-branch --project <PROJECT> --package <PACKAGE>` — Detach a branch: replace the branch link with a copy of the branched files.

## `build` — build status

- `geectl build results <PROJECT>` — Read the build-results matrix for a project; key flags: --package <PACKAGE>, --repository <REPOSITORY>, --arch <ARCH>, --code <CODE>, --multibuild, --lastbuild, --oldstate <OLDSTATE>, --all-instances
- `geectl build wait-for-results --deadline-secs <DEADLINE_SECS> <PROJECT>` — Poll the build results until they settle, or the deadline passes; key flags: --package <PACKAGE>, --repository <REPOSITORY>, --arch <ARCH>, --code <CODE>, --multibuild, --lastbuild
- `geectl build build-log <PROJECT> [PACKAGE] [REPOSITORY] [ARCH]` — Read the build log for a package; key flags: --start <START>, --last, --lastsucceeded, --multibuild <FLAVOR>, --strip-time
- `geectl build build-status <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Read the build status code for a package.
- `geectl build job-status <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Read the running build job for a package, if any.
- `geectl build rebuild-package <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Rebuild a package: rerun a build without touching the sources.
- `geectl build restart-build <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Restart the running build of a package, without touching the sources.
- `geectl build abort-build <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Abort the running build of a package.
- `geectl build wipe-binaries <PROJECT> [PACKAGE]` — Wipe the binary packages of a project or a package; key flags: --all, --arch <ARCH>, --repository <REPOSITORY>, --build-disabled, --build-failed, --broken, --unresolvable
- `geectl build buildinfo <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — Read the buildinfo for a package: the blocking error, if any, and the dependencies the build pulls in; key flags: --alternative-project <ALTERNATIVE_PROJECT>
- `geectl build buildconfig <PROJECT> <REPOSITORY>` — Read the build configuration for a project/repository: the expanded `_buildconfig` text the engine takes as `--dist`; key flags: --alternative-project <ALTERNATIVE_PROJECT>
- `geectl build build-history <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — List the completed builds for a package, newest first; key flags: --limit <LIMIT>
- `geectl build job-history <PROJECT> <REPOSITORY> <ARCH>` — List the build jobs that ran in a repository, newest first; key flags: --package <PACKAGE>, --limit <LIMIT>, --code <CODE>
- `geectl build list-binaries <PROJECT> <REPOSITORY> <ARCH>` — List the built files for a package (or the whole repository); key flags: --package <PACKAGE>
- `geectl build download-binary --output <OUTPUT> <PROJECT> <PACKAGE> <REPOSITORY> <ARCH> [FILENAME]` — Download one built file by name; key flags: --all
- `geectl build fileinfo <PROJECT> <PACKAGE> <REPOSITORY> <ARCH> <FILENAME>` — Read the metadata of a built file: RPM header fields plus dependency lists; key flags: --ext
- `geectl build dependson <PROJECT> <REPOSITORY> <ARCH>` — List which packages each package depends on for its build; key flags: --package <PACKAGE>
- `geectl build whatdependson <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — List the packages that depend on a package for their build.
- `geectl build check-constraints <PROJECT> <PACKAGE> <REPOSITORY> <ARCH>` — List the worker ids satisfying a `_constraints` document; key flags: --constraints <CONSTRAINTS>
- `geectl build list-repositories <PROJECT>` — List the repositories of a project.

`build buildinfo` as JSON carries the dependency names in `bdeps` (strings)
and the project each resolves from in the parallel `bdep_projects` array,
aligned by index (`null` for a silent server — a virtual provide
or a not-installed flag). Text output renders each dependency as
`name (project)` when the server said one.

## `local-build` — podman build vertical

- `geectl local-build service-run <DIRECTORY>` — Run the `_service` file's services in the container, filtered by the run mode; key flags: --mode <MODE>, --service <SERVICE>, --timeout-secs <TIMEOUT_SECS>
- `geectl local-build precommit <DIRECTORY>` — The precommit gate: the `precommit`-mode service run, then `pre_checkin.sh` when the source directory carries one; key flags: --timeout-secs <TIMEOUT_SECS>
- `geectl local-build spec-cleaner <DIRECTORY> <SPEC> [-- <ARGS>...]` — Run `spec-cleaner` on a spec in the container; key flags: --timeout-secs <TIMEOUT_SECS>
- `geectl local-build build --project <PROJECT> --repo <REPO> <DIRECTORY>` — Build the package in the container: the run-mode service run, the precommit gate, then the engine build; key flags: --arch <ARCH>, --flavor <FLAVOR>, --clean, --trust-all-projects, --alternative-project <ALTERNATIVE_PROJECT>, --root <ROOT>, --timeout-secs <TIMEOUT_SECS>
- `geectl local-build wipe [REPO] [ARCH] [SPEC]` — Wipe a build root: delete its contents (the lock file stays); key flags: --project <PROJECT>, --flavor <FLAVOR>, --root <ROOT>
- `geectl local-build prune` — Print the eviction plan (what would be evicted, in order, with bytes); delete only with `--apply`; key flags: --apply
- `geectl local-build periodic-install` — Install the daily periodic cleanup for this platform: write the scheduler entry and enable it.
- `geectl local-build periodic-uninstall` — Remove the daily periodic cleanup for this platform: disable the scheduler entry, then remove its files.
- `geectl local-build periodic-status` — Show whether the daily periodic cleanup is installed and enabled on this platform.
- `geectl local-build chroot --project <PROJECT> --repo <REPO> <DIRECTORY>` — Enter the preserved build root of the last build for the package and run repro commands there, read from stdin, one per line, joined with `&&` into one shell command; key flags: --arch <ARCH>, --flavor <FLAVOR>, --timeout-secs <TIMEOUT_SECS>
- `geectl local-build log --project <PROJECT> <REPO> <ARCH> <SPEC>` — Read the log of the last local build: the `<root>/.build.log` the build's coordinates name; key flags: --flavor <FLAVOR>, --tail <TAIL>, --full

`local-build build --trust-all-projects` allows build dependencies from
other users' `home:` projects: their packages run code in the build root,
so the flag gates such builds. The flag accepts the whole dep set the
buildinfo enumerates at once: foreign `home:` projects in the set are
trusted for the run; non-`home:` projects and the caller's own `home:` are
trusted already.
The accepted foreign `home:` projects are recorded in the build record
(`trusted_foreign_projects`). A dry run records the flag in the plan
without evaluating trust.

## `maintenance` — maintained instances + branching

- `geectl maintenance maintained-instances --package <PACKAGE>` — List the maintained instances of a package: the projects carrying the OBS:Maintained attribute that the package exists in, with where a branch would land; key flags: --attribute <ATTRIBUTE>, --update-project-attribute <UPDATE_PROJECT_ATTRIBUTE>
- `geectl maintenance maintenance-branch --package <PACKAGE>` — Branch a maintained package into its maintenance branch project and report where it landed; key flags: --attribute <ATTRIBUTE>, --update-project-attribute <UPDATE_PROJECT_ATTRIBUTE>, --target-project <TARGET_PROJECT>

## `service` — service reads and trigger

- `geectl service list <PROJECT> <PACKAGE>` — List the services defined in a package's `_service` file.
- `geectl service state <PROJECT> <PACKAGE>` — Show the last service run state of a package.
- `geectl service trigger <PROJECT> <PACKAGE>` — Trigger a server-side run of a package's services; key flags: --service <SERVICE>

## `gitea` — Gitea operations

- `geectl gitea repo-get <OWNER> <REPO>` — Show a repository: owner, default branch, privacy, clone URLs.
- `geectl gitea repo-list` — List repositories: the token owner's own repos, one user's repos with `--owner`, one organization's with `--org`, or both (the user's first, then the organization's); key flags: --owner <OWNER>, --org <ORG>
- `geectl gitea repo-search <QUERY>` — Search repositories across the instance by keyword: the keyword matches the name and the description. The Gitea counterpart of `remote search-packages` / `remote search-projects`.
- `geectl gitea repo-create <NAME>` — Create a repository under the token owner's account; key flags: --private, --description <DESCRIPTION>, --default-branch <DEFAULT_BRANCH>
- `geectl gitea repo-delete <OWNER> <REPO>` — Delete a repository; key flags: --force
- `geectl gitea repo-fork <OWNER> <REPO>` — Fork a repository into the token owner's account (or an organization with `--organization`); key flags: --new-name <NEW_NAME>, --organization <ORGANIZATION>
- `geectl gitea branch-list <OWNER> <REPO>` — List the branches of a repository.
- `geectl gitea branch-get <OWNER> <REPO> <BRANCH>` — Show one branch: name, head commit, protection.
- `geectl gitea branch-head <OWNER> <REPO> <BRANCH>` — Print the head commit SHA of a branch.
- `geectl gitea maintainers <OWNER> <REPO>` — Show the maintainers recorded in a git project's `_maintainership.json`: the project maintainers and the per-package maintainers.

## `pull-request` — unified pull-request read and act

- `geectl pull-request show <REFERENCE>` — Show one pull request: `obs:<id>` or `gitea:<owner>/<repo>#<number>`.
- `geectl pull-request search` — Search pull requests: OBS submit requests and Gitea pull requests as one list, OBS first then Gitea; key flags: --backend <BACKEND>, --state <STATE>, --creator <CREATOR>, --repo <REPO>
- `geectl pull-request approve <REFERENCE>` — Approve a pull request as the review asked of the target: with no target flag, as the review asked of the acting user (OBS) or the token owner (Gitea); key flags: --message <MESSAGE>, --user <USER>, --group <GROUP>, --project <PROJECT>, --package <PACKAGE>
- `geectl pull-request request-changes <REFERENCE>` — Ask for changes on a pull request, as `approve` but with the review declined instead of accepted; key flags: --message <MESSAGE>, --user <USER>, --group <GROUP>, --project <PROJECT>, --package <PACKAGE>
- `geectl pull-request comment --body <BODY> <REFERENCE>` — Comment on a pull request; key flags: --parent <PARENT>
- `geectl pull-request comments <REFERENCE>` — List the comments on a pull request, oldest first.
- `geectl pull-request cancel-review-request <REFERENCE>` — Cancel pending review requests on a pull request; key flags: --reviewer <REVIEWER>, --all, --exclude <EXCLUDE>
- `geectl pull-request merge <REFERENCE>` — Merge a pull request. On OBS this accepts the submit request and forwards it to its link parent; a refused forward is reported and the accepted request stands. On Gitea the forward is resolved and validated before the merge lands, and the merged pull request is forwarded to its link parent too, unless `--no-forward` is given. The OBS arm checks the hop after accepting while the Gitea arm preflights it, because the Gitea merge is irreversible while the accept flow's forward is retryable; key flags: --message <MESSAGE>, --method <METHOD>, --force, --no-forward
- `geectl pull-request decline <REFERENCE>` — Decline a pull request; key flags: --message <MESSAGE>
- `geectl pull-request withdraw <REFERENCE>` — Withdraw a pull request; key flags: --message <MESSAGE>
- `geectl pull-request reopen <REFERENCE>` — Reopen a pull request.
- `geectl pull-request submit` — File a submission from the working copy in the current directory: an OBS submit request from an OBS-native checkout, a Gitea pull request from a git-backed one; key flags: --target-project <TARGET_PROJECT>, --target-package <TARGET_PACKAGE>, --description <DESCRIPTION>, --changes-entry <CHANGES_ENTRY>, --title <TITLE>, --body <BODY>, --draft

## `person` — unified person search

- `geectl person search <QUERY>` — Search for a person: the OBS projects naming the login, and the Gitea users matching the keyword; key flags: --backend <BACKEND>

## `meta` — unified meta write

- `geectl meta set` — Set the meta of an OBS project, an OBS package, or a Gitea repository: the description and/or URL. The target selects the backend: `--project` (with `--package` for a package) writes OBS meta, `--repo` writes Gitea repo settings. A mutation: dry-run is the default (only the plan is printed); `--live` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field; key flags: --project <PROJECT>, --package <PACKAGE>, --repo <REPO>, --description <DESCRIPTION>, --url <URL>

## `changelog` — `.changes` validation and generation

- `geectl changelog validate [DIR]` — Validate the `.changes` entries in a package directory against the V1–V12 rules; key flags: --target-project <TARGET_PROJECT>, --target-package <TARGET_PACKAGE>, --entries <ENTRIES>
- `geectl changelog generate [DIR]` — Generate a skeleton `.changes` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball; key flags: --target-project <TARGET_PROJECT>, --target-package <TARGET_PACKAGE>, --author <AUTHOR>, --write

## Working copy — git working copy

The working-copy leaves operate on the checkout in the current
directory: `checkout` writes `<dir>/<project>/<package>/` under the
current directory (a `:` in a project or package name becomes `_` in
the directory name), the rest open the checkout found there. `commit`
runs the source services and `pre_checkin.sh` before sending, with
`--noservice` skipping that gate. The base record lives in the checkout's git
config (`geectl.apiurl`, `geectl.project`, `geectl.package`,
`geectl.backend`, `geectl.base-rev`, `geectl.base-srcmd5`).

- `geectl checkout <PROJECT> <PACKAGE>` — Check out a package into a git working copy, under the current directory; key flags: --rev <REV>
- `geectl update` — Update the working copy in the current directory to the latest base; key flags: --rev <REV>
- `geectl status` — Show the working copy's state against the base.
- `geectl diff` — Show the diff of the worktree and the index against the base, plus the link diff on a linked checkout.
- `geectl add <FILES>...` — Stage files for commit.
- `geectl rm <FILES>...` — Remove files from the working copy.
- `geectl addremove` — Stage new files and worktree deletions.
- `geectl commit --message <MESSAGE>` — Commit the staged changes; key flags: --noservice
- `geectl pull` — Pull a linked package to the link target's latest revision.
- `geectl resolve` — Stage the resolved files and continue a stopped rebase.

## `prjconf` — project-config reads and cleanup

- `geectl prjconf show <PROJECT>` — Show a project's raw prjconf.
- `geectl prjconf validate <PROJECT>` — Validate a project's prjconf across its repositories.
- `geectl prjconf clean <PROJECT>` — Clean a project's prjconf; key flags: --include-path-dependent

## Common tasks

```sh
# Read-only: what the distribution holds
geectl remote list-packages openSUSE:Factory
geectl remote list-files openSUSE:Factory vim
geectl remote cat openSUSE:Factory vim vim.spec
geectl remote download openSUSE:Factory vim vim.spec --output vim.spec
geectl --format text remote list-packages openSUSE:Factory

# Build status
geectl build results openSUSE:Factory --code failed
geectl build build-log openSUSE:Factory vim standard x86_64
geectl build build-log openSUSE:Factory multibuild-pkg standard x86_64 --multibuild flavor1 --strip-time
geectl build build-log https://build.opensuse.org/package/live_build_log/openSUSE:Factory/vim/standard/x86_64
geectl build list-binaries openSUSE:Factory standard x86_64 --package vim

# Submit requests
geectl request show 1234567
geectl request search --creator alice --state new
geectl request search --reviewer alice
geectl request diff 1234567

# Branch, then file (dry-run first, `--live` to perform)
geectl branch branch-package --project openSUSE:Factory --package vim
geectl --live branch branch-package --project openSUSE:Factory --package vim
geectl request file --source-project home:alice:branches:openSUSE:Factory --source-package vim --target-project devel:example --target-package vim --description "Update vim to 9.1" --changes-entry "- Update to 9.1"

# Maintenance
geectl maintenance maintained-instances --package vim
geectl maintenance maintenance-branch --package vim

# Services
geectl service list openSUSE:Factory vim
geectl service state openSUSE:Factory vim
geectl --live service trigger openSUSE:Factory vim

# Gitea (needs GEECTL_GITEA_TOKEN, or GEECTL_GITEA_TOKEN_<ALIAS> for an aliased instance, even in dry-run)
geectl gitea repo-get pool systemd
geectl gitea repo-search geectl
geectl gitea branch-head pool systemd factory
geectl gitea maintainers pool systemd
geectl gitea repo-create vim --private

# Pull requests (OBS + Gitea as one surface)
geectl pull-request show obs:1234567
geectl pull-request show gitea:openSUSE/geectl#122
geectl pull-request search --backend gitea --state open

# Person search (OBS + Gitea as one operation)
geectl person search alice
geectl person search alice --backend gitea

# Meta (OBS + Gitea as one surface)
geectl meta set --project home:scarabeus_iv --description "New description"
geectl meta set --repo pool/systemd --description "New description" --url https://example.com/systemd

# .changes validation and generation (local, pre-file)
geectl changelog validate ~/work/foo
geectl changelog validate ~/work/foo --target-project openSUSE:Factory --target-package foo
geectl changelog generate ~/work/foo --author 'Jane Packager <jane@example.com>'
```

Validate a package directory before filing: `changelog validate` exits
24 (`validation_failed`) naming the failing rule, and lists the
skipped target-relative rules when no target is given. `changelog
generate` prints a draft entry — a bare version line plus explicit
`TODO` slots — a draft only; `--write` prepends it into the
package's `.changes` file and refuses a draft with unfilled
`TODO` slots or a needs-detail flag. Writing normalizes the whole
file: trailing ASCII whitespace is stripped from every line,
CRLF becomes LF, legacy-encoded bytes are transcoded to UTF-8
(best-effort; valid UTF-8 passes through untouched), a BOM is stripped,
and a UTF-16 file is decoded whole first. The write is atomic (a
temp file in the same directory, then a rename, keeping the
destination's permissions) and refuses a symlink destination, an
unreadable file, or bytes outside every known encoding; the guessed
encoding is named on the debug channel. One limit remains: a file
decoded from a guessed single-byte encoding is transcoded on the
guess — a single-byte guess always decodes, so a wrong guess (a corrupt byte disabling the multi-byte
probers) still decodes cleanly and the valid UTF-8 half can be
rewritten as mojibake. The write warns about the guess on stderr,
naming the encoding.

The `request file` flags above are illustrative — run
`geectl request file --help` for the exact flag set. The same holds
for every command: the reference above is a summary, and the binary's
`--help` is the authority.

The `request file` JSON's `cve` field carries the `--cve` values
parsed and normalized to uppercase `CVE-<year>-<number>` (year 1999
or later, number at least 4 digits); raw strings become normalized values.

## How it differs from osc

geectl differs from osc: one noun per operation family,
explicit flags instead of positionals, explicit working-copy leaves,
JSON by default with opt-in text, and dry-run by default for
mutations. The full list of deliberate deviations, with reasons, is
[docs/cli-swaps.md](cli-swaps.md).
