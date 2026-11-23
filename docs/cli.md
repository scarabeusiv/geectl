# geectl CLI usage

The command reference for the geectl binary: one subcommand per
operation, grouped under a top-level noun (`remote`, `request`,
`branch`, `build`, `maintenance`, `service`, `gitea`, `pull-request`), plus `version`
and `config`, plus the ten working-copy leaves (`checkout`, `update`,
`status`, `diff`, `add`, `rm`, `addremove`, `commit`, `pull`,
`resolve`) that operate on the checkout in the current directory.
Every `--help` block below is the real output of the built binary;
run `geectl --help` or `geectl <group> --help` for the same text live.

## Invocation shape

```sh
geectl [--format json|text] [--debug] [--debug-body] [--live] <group> <command> [args] [flags]
```

- Required coordinates are positionals where the leaf takes them;
  the build group orders them project, package, repository, arch.
  Branch and maintenance take coordinates as long flags. The exact
  order per leaf is in its `--help` block below. Everything else is
  a named long flag. No short flags on group/leaf options apart from
  clap's built-in `-h`/`-V`, and no command aliases.
- `--format` selects the stdout payload: `json` (default — the only
  format an agent should parse) or `text` (a human rendering of the
  same data). The same invocation produces the same bytes in every
  context; there is no terminal sniffing. A persistent default comes
  from `GEECTL_FORMAT`, else the `format` key in the config file;
  `--format` wins over both. Agent invocations pass
  `--format json` explicitly.
- `--debug` prints human-readable diagnostic lines on stderr (HTTP
  attempts, retries, library decisions); `GEECTL_DEBUG=1` enables it
  without the flag. `--debug-body` additionally prints request and
  response bodies (capped, redacted, never headers). Debug never
  changes stdout.
- Mutations are dry-run by default: without `--live` only the plan is
  printed (on stderr as it is recorded, and in the payload as
  `dry_run: true` with a `plan`). Pass the global `--live` to perform
  the mutation. The one exception is `request supersede`, which has
  no dry-run at all: without `--live` it refuses — the library's raw
  supersede cannot be previewed, so use `request refile --supersede`
  for the dry-runnable supersede flow.
- `remote set-project-meta` in dry-run validates every repository
  the write carries, since it performs no fetch and cannot tell a
  changed repository from a pre-existing one: a dry run can refuse
  a write whose repositories it did not change when a carried
  repository is invalid, while `--live` only validates the
  repositories when they change.
- `request file-delete` deletes a whole project only with
  `--whole-project`: omitting `--package` and `--repository` without
  it refuses.
- Errors go to stderr in the selected format (JSON payload, or
  `error:` + `remediation:` lines in text mode) with the error kind's
  exit code. A parse error has no parsed CLI yet, so it stays JSON.
- Text renderings open with one header line echoing the query
  coordinates (`project: … package: … …`); tables follow as designed.
  Content commands (`remote cat`, `remote revision-patch`,
  `remote project-diff`, `remote diff-packages`, `build build-log`,
  `request diff`, `gitea branch-head`)
  render verbatim, with no header — the coordinates live in the JSON
  payload. Two renderings keep no header either: `remote
  list-projects` (the query names nothing to echo) and the request
  action texts (one-line outcome summaries; the plan is on stderr
  and in the payload).

### Credentials in dry-run

Credentials are stored per instance, named by an alias: the config
file names each instance once in an `[instances]` table mapping
alias to URL, and the credential variables use the alias uppercased:

```toml
[instances]
myloveinstance = "https://api.opensuse.org"
local = "http://127.0.0.1:3001"
```

```sh
GEECTL_OBS_USERNAME_MYLOVEINSTANCE=... GEECTL_OBS_PASSWORD_MYLOVEINSTANCE=...
GEECTL_OBS_USERNAME_LOCAL=... GEECTL_OBS_PASSWORD_FILE_LOCAL=/run/secrets/local-password
GEECTL_OBS_APIKEY_MYLOVEINSTANCE=...   # a token instead of the pair
```

The plain `GEECTL_OBS_APIKEY` (or `..._APIKEY_FILE`) or
`GEECTL_OBS_USERNAME` / `GEECTL_OBS_PASSWORD` (or
`..._PASSWORD_FILE`) apply to the configured instance; a call to
another instance resolves the alias whose configured URL matches
it (same scheme, host, and effective port — `https://host` and
`https://host:443` are the same instance, `http://host:3000` and
`https://host:3000` are not) and uses that alias's variables. A
call to another instance with no credentials configured for it
fails clearly naming what to set — the alias variables when an
alias matches, the `[instances]` entry to add when none does — and
the configured instance's credentials are never forwarded.
`branch copy-package --to-apiurl` resolves the target's
credentials this way.

A half-configured credential pair (a username with no password source,
or the reverse) is handled per group. `branch`, `maintenance`, and
`service` fail fast on it before any work — `branch` and `maintenance`
in dry-run too, since a dry run that cannot be performed live is not a
useful preview (`service`'s trigger is the mutation there; its reads
need no credentials but resolve them up front).
`request` swallows it in dry-run (a preview never needs credentials;
the dropped error goes to the debug channel). `remote` and `build`
resolve credentials per command instead: the public reads (see
Credentials in `docs/architecture.md`) never touch them, so a half-set
env must not break them, and neither does a dry-run `remote` mutation
preview; every other command in those groups fails fast on a half-set
pair up front. `remote list-files --include-deleted` resolves
credentials too: the deleted listing reads the authenticated route.
Setting both a token and a Basic pair for one instance
is a configuration error at the same resolution points.

### `.env` file

A `.env` file in the working directory supplies `GEECTL_*` variables
without exporting them, when the run opts in with `GEECTL_DOTENV=1` in
the real process environment: geectl loads it at startup and merges it into
the environment snapshot, with a real environment variable winning
over the same key in the file. Only `GEECTL_*` keys are honoured. A
key starting with `GEEC` that is not a valid `GEECTL_*` key, or a
`GEECTL_*` key with anything but ASCII letters, digits, or `_` after
the prefix, is a typo for a geectl variable and fails as a config
error rather than being silently ignored. The syntax is minimal —
`KEY=value`, `#` comments, blank lines, an optional `export` prefix
followed by whitespace (a key glued to `export` is malformed), and optional single or double quotes — with no
variable interpolation and no multi-line values. A duplicate key keeps
its last value, and quoted values are literal inside the quotes, with
no escapes. The file must be a regular file, at most 64 KiB, and
readable only by its owner (`chmod 600`) on Unix — on non-Unix the
mode cannot be checked, and the opt-in is the guard there. A missing
file is fine, and so is a file that carries no `GEECTL_*` keys,
whatever its mode or syntax — but an unreadable, malformed, or
group/other-readable file that carries `GEECTL_*` keys fails at
startup as a config error naming the file (and the line, for a
malformed line), never the value. `--version` and `--help` print and
exit before the file is loaded, so they never fail on it. `--debug`
reports that a `.env`
file was loaded, never its values. Values from the file report their
source as `environment` in `config show`. No parent directory is
searched, and `.env.*` files are not loaded.

## Top level

```
Geeko Controller — a CLI for openSUSE packaging workflows (OBS + src.opensuse.org)

Usage: geectl [OPTIONS] [COMMAND]

Commands:
  version       Print version information as the result payload
  config        Inspect or initialise the config file
  remote        Remote reads and mutations: inspect what the distribution holds, and change it
  request       Read and act on OBS submit requests, and file new ones
  branch        Branch, copy, and link packages
  build         Build-status commands: results, logs, histories, binaries, buildconfig
  local-build   Local builds in the podman build vertical: build a package, run its `_service` services, the precommit gate, `spec-cleaner`, wipe a build root, prune the cache, manage the daily periodic cleanup, enter the preserved build root, and read the log of the last local build
  maintenance   Maintenance operations: maintained-instances listing and maintenance branching
  service       Service reads and trigger: list a package's services, their run state, and trigger a server-side run
  gitea         Gitea operations: repository and branch reads and mutations on the configured instance
  pull-request  Read and act on pull requests: OBS submit requests and Gitea pull requests as one surface
  changelog     Validate and generate `.changes` entries in a package directory, pre-file
  checkout      Check out a package into a git working copy, under the current directory
  update        Update the working copy in the current directory to the latest base
  status        Show the working copy's state against the base
  diff          Show the diff of the worktree and the index against the base, plus the link diff on a linked checkout
  add           Stage files for commit
  rm            Remove files from the working copy
  addremove     Stage new files and worktree deletions
  commit        Commit the staged changes
  pull          Pull a linked package to the link target's latest revision
  resolve       Stage the resolved files and continue a stopped rebase
  prjconf       Read, validate and clean a project's prjconf
  help          Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
  -V, --version          Print version
```

## `remote` — inspection and mutations

```
Remote reads and mutations: inspect what the distribution holds, and change it

Usage: geectl remote [OPTIONS] <COMMAND>

Commands:
  cat                            Print a source file to standard output. Text only: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes
  download                       Download a source file to a local path. The binary counterpart of `cat`: the library streams the file to disk instead of reading it as a UTF-8 string, so a binary file (a source tarball) downloads instead of failing the UTF-8 check, and a file larger than the transport's 10 MiB body cap downloads instead of failing. A read: `--live` is ignored, like `cat`
  list-files                     List the files of a package
  list-packages                  List the packages of a project
  list-projects                  List the top-level projects on the instance
  list-staging-projects          List the staging projects of a workflow project
  staging-project                Show one staging project of a workflow project
  package-log                    Show the commit history of a package
  revision-patch                 Show the per-file diff of one revision against the previous one. Computed client-side from per-file reads (the library lists both revisions and cats each changed file), not a server-side diff; a differing binary file fails the call
  project-meta                   Show a project's meta (title, description, persons, repositories)
  set-project-meta               Set fields of a project's meta. A mutation: dry-run is the default (only the plan is printed); `--live` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field
  package-meta                   Show a package's meta (title, description, persons, devel project)
  set-package-meta               Set fields of a package's meta. A mutation: dry-run is the default (only the plan is printed); `--live` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field
  update-package-meta-from-spec  Update a package's meta from its spec file. A mutation: dry-run is the default (only the plan is printed); `--live` performs the update. The spec file is found by listing the package files when exactly one `*.spec` exists; otherwise name it with `--spec`
  search-packages                Search packages by name. Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches (exact matching is not provided server-side, so the filter runs client-side). With `--devel-project`, only packages developed in that project; without a term, every package the devel project develops
  search-projects                Search projects by name. Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches (exact matching is not provided server-side, so the filter runs client-side)
  search-projects-by-person      Search projects by a person's login: every project whose meta names the login in a role
  search-packages-by-person      Search packages by a person's login: every package whose meta names the login in a role
  count-scmsync-projects         Count the projects syncing from a git URL prefix
  owners                         Show the owners of a package (project/package scope)
  owners-for-binary              Show the owners of a binary package
  owners-for-package             Show the owners of a package by name alone, in the owner root projects
  maintainers                    Show the maintainers of a package
  whois                          Show a user's full name, email, and account state. Without a login, the authenticated user
  link-target                    Show the link target of a package, when it is a link
  project-diff                   Diff two projects, package by package. Computed client-side: the library lists both projects and reads every file of every differing package, so a large diff issues many requests and a differing binary file fails the call
  diff-packages                  Diff two packages, file by file. Computed client-side: the library diffs the two file lists and cats each changed file, so a differing binary file fails the call
  devel-project                  Show the devel project and devel package of a package, when it has one
  repo-add                       Add a repository to a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the addition
  repo-remove                    Remove a repository from a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the removal. A repository that is not there is not an error
  upload-file                    Upload a file into a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the upload. The local file is read from `--from`; `--binary` uploads bytes instead of text
  create-project                 Create a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the creation
  create-package                 Create a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the creation. Without `--scmsync` and `--force` the package is created directly; with either, the creation goes through the options form, which refuses an existing package unless `--force` is set and refuses a project managed in git
  delete-package                 Delete a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the deletion. Without `--force` a package with open submit requests is refused
  delete-link                    Delete a `_link` file from a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the deletion. The package keeps its files and meta, but stops tracking the source
  delete-file                    Delete a file from a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the deletion. The general form of `delete-link`: deleting `_link` here issues the same delete as `delete-link`. The delete is not idempotent: deleting a file that is already gone is refused by the server (NotFound). The server records the delete with the fixed comment `removed <file>`; there is no `--message` option, unlike `delete-package` — the asymmetry is intentional
  delete-project                 Delete a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the deletion. Without `--recursive` a project holding packages is refused
  undelete-package               Restore a deleted package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the restore
  undelete-project               Restore a deleted project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the restore
  aggregate-package              Aggregate a package's binaries into another package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the aggregation. The target package is created from the source package's meta when it does not exist yet; an `_aggregate` file that already exists is refused
  release-package                Release a package's sources and binaries. A mutation: dry-run is the default (only the plan is printed); `--live` performs the release. A release to a target project needs a repository and a target repository
  release-project                Release a whole project's sources and binaries. A mutation: dry-run is the default (only the plan is printed); `--live` performs the release. A release to a target project needs a repository and a target repository
  help                           Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

Two leaves, to show the flag style (required coordinates as
positionals, everything else a long flag):

```
Print a source file to standard output. Text only: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes

Usage: geectl remote cat [OPTIONS] <PROJECT> <PACKAGE> <FILE>

Arguments:
  <PROJECT>  The project the package lives in
  <PACKAGE>  The package the file lives in
  <FILE>     The file to print

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --rev <REV>        Print the file as of this revision (default: latest): a revision number or a srcmd5
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Download a source file to a local path. The binary counterpart of `cat`: the library streams the file to disk instead of reading it as a UTF-8 string, so a binary file (a source tarball) downloads instead of failing the UTF-8 check, and a file larger than the transport's 10 MiB body cap downloads instead of failing. A read: `--live` is ignored, like `cat`

Usage: geectl remote download [OPTIONS] --output <OUTPUT> <PROJECT> <PACKAGE> <FILE>

Arguments:
  <PROJECT>  The project the package lives in
  <PACKAGE>  The package the file lives in
  <FILE>     The file to download

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --rev <REV>        Download the file as of this revision: a revision number or a srcmd5 (default: latest)
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --output <OUTPUT>  Where to write the downloaded bytes
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

## `request` — submit requests

```
Read and act on OBS submit requests, and file new ones

Usage: geectl request [OPTIONS] <COMMAND>

Commands:
  show                         Show one submit request: source/target/message, reviews, history
  log                          Show one submit request's history: creation, then each state change
  search                       List the submit requests matching the filters: a typed search, not a plain listing
  list                         List submit requests: the same search, with the states still awaiting action (new, review) as the default. A thin wrapper over the search operation: the flags select the same server-side search
  diff                         Show the diff of a submit request: the unified diff, or the request XML with the diff injected with `--xml`
  list-comments                List the comments on a submit request, in id order
  accept                       Accept a submit request, with a comment as the message left with the accept
  decline                      Decline a submit request, with a comment as the reason
  revoke                       Revoke (withdraw) a submit request, with a comment as the reason
  change-state                 Change a submit request's state to one of new/review/accepted/declined/revoked
  try-change-state             Best-effort state change: try it, and report whether it happened instead of failing when the request cannot move
  add-review                   Ask a user, group, project, or package to review a submit request
  change-review-state          Change the state of one review on a submit request: accept it, decline it, or reopen it to new
  add-comment                  Comment on a submit request, optionally as a reply to another comment
  file                         File a submit request: source package to a devel project, superseding every open submit request for the package from the same user
  refile                       File a submit request and supersede an old one with it, plus every other open submit request for the package from the same user: the new request is filed first, and only then is each open one superseded pointing at it
  supersede                    Supersede one request with another existing one. A mutation that needs `--live`: the library's raw supersede has no dry-run, so without `--live` this command refuses — use `request refile --supersede` for the dry-runnable flow
  file-add-role                File an add-role request: grant a person or a group a role on a project or a package
  file-change-devel            File a change-devel request: change a package's devel project
  file-delete                  File a delete request: delete a package, a repository, or a whole project
  file-maintenance-incident    File a maintenance incident request: ask the maintenance team to start a maintenance incident from a maintenance update
  refile-maintenance-incident  File a maintenance incident request and supersede an old one with it: the new request is filed first, and only then is the old one superseded pointing at it
  help                         Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show one submit request: source/target/message, reviews, history

Usage: geectl request show [OPTIONS] <ID>

Arguments:
  <ID>  The request id

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

## `branch` — branch, copy, link

```
Branch, copy, and link packages

Usage: geectl branch [OPTIONS] <COMMAND>

Commands:
  branch-package        Branch a package: create a linked copy under a branch project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the branch
  copy-package          Copy a package to another project (or another OBS instance with `--to-apiurl`). A mutation: dry-run is the default
  copy-project          Copy a whole project. A mutation: dry-run is the default
  set-devel-project     Set (or, without `--devel-project`, remove) the devel project of a package. A mutation: dry-run is the default
  link-package          Link a package to another package. A mutation: dry-run is the default
  repair-link           Repair a broken link: strip the failing patches from `_link` server-side and pin it at a revision where it expands. A mutation: dry-run is the default
  set-link-rev          Set the revision a link expands at. A mutation: dry-run is the default
  set-link-rev-project  Set the revision every linked package in a project expands at. A mutation: dry-run is the default
  link-to-branch        Turn a link into a branch: replace the link with a copy of the linked files. A mutation: dry-run is the default
  detach-branch         Detach a branch: replace the branch link with a copy of the branched files. A mutation: dry-run is the default
  help                  Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Branch a package: create a linked copy under a branch project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the branch

Usage: geectl branch branch-package [OPTIONS] --project <PROJECT> --package <PACKAGE>

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --project <PROJECT>
          The project to branch from
      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --package <PACKAGE>
          The package to branch
      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --target-project <TARGET_PROJECT>
          The branch project. Without it, the server picks the default (usually `home:<user>:branches:<source project>`)
      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
      --target-package <TARGET_PACKAGE>
          The branch package name. Without it, the source package name
      --no-devel-project
          Ignore the devel project when resolving the link source
      --new-package
          Branch a package that does not exist yet
      --force
          Overwrite an existing target package
      --revision <REVISION>
          Branch from this source revision instead of the latest
      --linkrev <LINKREV>
          The link revision the branch records
      --comment <COMMENT>
          A comment recorded on the branch
      --noaccess
          Do not give the branch read access to the source package's project
      --add-repositories
          Add the source project's repositories to the branch project
      --add-repositories-block <ADD_REPOSITORIES_BLOCK>
          Block the added repositories. Only sent with `--add-repositories` (clap refuses it without that flag: a block the live run would silently drop must fail at parse time, not vanish)
      --add-repositories-rebuild <ADD_REPOSITORIES_REBUILD>
          Rebuild the added repositories. Only sent with `--add-repositories` (clap refuses it without that flag, as above)
      --extend-package-names
          Extend the branch package names with the source project name
      --maintenance
          Branch as a maintenance incident
  -h, --help
          Print help
```

## `build` — build status

```
Build-status commands: results, logs, histories, binaries, buildconfig

Usage: geectl build [OPTIONS] <COMMAND>

Commands:
  results            Read the build-results matrix for a project
  wait-for-results   Poll the build results until they settle, or the deadline passes
  build-log          Read the build log for a package
  build-status       Read the build status code for a package
  job-status         Read the running build job for a package, if any
  rebuild-package    Rebuild a package: rerun a build without touching the sources. A mutation: dry-run is the default (only the plan is printed); `--live` performs the rebuild
  restart-build      Restart the running build of a package, without touching the sources. A mutation: dry-run is the default (only the plan is printed); `--live` performs the restart
  abort-build        Abort the running build of a package. A mutation: dry-run is the default (only the plan is printed); `--live` performs the abort
  buildinfo          Read the buildinfo for a package: the blocking error, if any, and the dependencies the build pulls in
  buildconfig        Read the build configuration for a project/repository: the expanded `_buildconfig` text the engine takes as `--dist`
  build-history      List the completed builds for a package, newest first
  job-history        List the build jobs that ran in a repository, newest first
  list-binaries      List the built files for a package (or the whole repository)
  download-binary    Download one built file by name. With `--all`, every built file
  fileinfo           Read the metadata of a built file: RPM header fields plus dependency lists
  dependson          List which packages each package depends on for its build
  whatdependson      List the packages that depend on a package for their build
  check-constraints  List the worker ids satisfying a `_constraints` document
  list-repositories  List the repositories of a project
  help               Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Read the build-results matrix for a project

Usage: geectl build results [OPTIONS] <PROJECT>

Arguments:
  <PROJECT>  The project to read

Options:
      --format <FORMAT>          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --package <PACKAGE>        Only this package's statuses (the repository/architecture cells still come back whole)
      --debug                    Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --repository <REPOSITORY>  Only these repositories (repeatable)
      --arch <ARCH>              Only these architectures (repeatable)
      --debug-body               Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --code <CODE>              Only packages with this build code (e.g. `failed`)
      --live                     Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
      --multibuild               Expand multibuild flavors into their own statuses
      --locallink                Follow local links when expanding multibuild flavors
      --lastbuild                The last build's results instead of the current state
      --oldstate <OLDSTATE>      Poll for changes since this state hash (from a previous run). Only `results` takes it: `wait-for-results` polls plainly, and the library strips a caller-set `oldstate` there (a held reply would outlast the read timeout), so the flag is not offered on the wait
      --all-instances            Merge the results from every configured instance into one view, instead of only the instance currently pointed at. Cannot combine with `--oldstate`: a state hash is per instance
  -h, --help                     Print help
```

`build buildinfo` as JSON carries the dependency names in `bdeps` (strings)
and the project each resolves from in the parallel `bdep_projects` array,
aligned by index (`null` when the server did not say — a virtual provide
or a not-installed flag). Text output renders each dependency as
`name (project)` when the server said one.

## `local-build` — podman build vertical

```
Local builds in the podman build vertical: build a package, run its `_service` services, the precommit gate, `spec-cleaner`, wipe a build root, prune the cache, manage the daily periodic cleanup, enter the preserved build root, and read the log of the last local build

Usage: geectl local-build [OPTIONS] <COMMAND>

Commands:
  service-run         Run the `_service` file's services in the container, filtered by the run mode
  precommit           The precommit gate: the `precommit`-mode service run, then `pre_checkin.sh` when the source directory carries one
  spec-cleaner        Run `spec-cleaner` on a spec in the container
  build               Build the package in the container: the run-mode service run, the precommit gate, then the engine build
  wipe                Wipe a build root: delete its contents (the lock file stays). A mutation: dry-run is the default (only the plan is printed); `--live` performs the wipe. There is no force flag: the marker check and the in-use lock are safety invariants, never bypassed
  prune               Print the eviction plan (what would be evicted, in order, with bytes); delete only with `--apply`. The plan is the default: the global `--live` does not gate this leaf, the leaf's own `--apply` is the confirmation, so the scheduled task runs exactly `prune --apply`
  periodic-install    Install the daily periodic cleanup for this platform: write the scheduler entry and enable it. A mutation: dry-run is the default (only the plan is printed); `--live` performs the install
  periodic-uninstall  Remove the daily periodic cleanup for this platform: disable the scheduler entry, then remove its files. A mutation: dry-run is the default (only the plan is printed); `--live` performs the uninstall
  periodic-status     Show whether the daily periodic cleanup is installed and enabled on this platform. Read-only
  chroot              Enter the preserved build root of the last build for the package and run repro commands there, read from stdin, one per line, joined with `&&` into one shell command. A mutation: dry-run is the default (only the plan is printed); `--live` runs the commands in the root. There is no interactive shell: a terminal on stdin is refused instead of blocking on a read that never arrives. A failing repro still exits 0: the exit code is data, in `exit:` and in `record.result.exit_code`
  log                 Read the log of the last local build: the `<root>/.build.log` the build's coordinates name. A read, not a mutation: no `--live` needed. The text is the log verbatim (design rule 4); the verdict, exit code, log path, and truncation flag ride in the JSON payload
  help                Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

One leaf, the mode-filtered service run:

```
Run the `_service` file's services in the container, filtered by the run mode

Usage: geectl local-build service-run [OPTIONS] <DIRECTORY>

Arguments:
  <DIRECTORY>  The source directory carrying the `_service` file

Options:
      --format <FORMAT>              Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --mode <MODE>                  The run mode: run, manualrun, precommit, localrun, disabledrun [default: run]
      --debug                        Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --service <SERVICE>            Run only this service, with the run mode forced as its mode
      --debug-body                   Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --timeout-secs <TIMEOUT_SECS>  Timeout in seconds for the whole run [default: 3600]
      --live                         Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help                         Print help
```

`local-build build --trust-all-projects` allows build dependencies from
other users' `home:` projects: their packages run code in the build root,
so without the flag such builds are refused. The flag lifts only the
foreign-`home:` refusal for the dep set the buildinfo enumerates —
non-`home:` projects and the caller's own `home:` were always trusted —
and takes no per-project selection, unlike osc's `trusted_prj` globs.
The accepted foreign `home:` projects are recorded in the build record
(`trusted_foreign_projects`). A dry run records the flag in the plan
without evaluating trust.
The eviction plan, and the delete switch:

```
Print the eviction plan (what would be evicted, in order, with bytes); delete only with `--apply`. The plan is the default: the global `--live` does not gate this leaf, the leaf's own `--apply` is the confirmation, so the scheduled task runs exactly `prune --apply`

Usage: geectl local-build prune [OPTIONS]

Options:
      --apply            Delete what the plan lists
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

The daily periodic cleanup: install, uninstall, status.

```
Install the daily periodic cleanup for this platform: write the scheduler entry and enable it. A mutation: dry-run is the default (only the plan is printed); `--live` performs the install

Usage: geectl local-build periodic-install [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Remove the daily periodic cleanup for this platform: disable the scheduler entry, then remove its files. A mutation: dry-run is the default (only the plan is printed); `--live` performs the uninstall

Usage: geectl local-build periodic-uninstall [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show whether the daily periodic cleanup is installed and enabled on this platform. Read-only

Usage: geectl local-build periodic-status [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

## `maintenance` — maintained instances + branching

```
Maintenance operations: maintained-instances listing and maintenance branching

Usage: geectl maintenance [OPTIONS] <COMMAND>

Commands:
  maintained-instances  List the maintained instances of a package: the projects carrying the OBS:Maintained attribute that the package exists in, with where a branch would land. Read-only; requires OBS credentials
  maintenance-branch    Branch a maintained package into its maintenance branch project and report where it landed. A mutation: dry-run is the default (only the plan is printed); the global `--live` performs the branch
  help                  Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
List the maintained instances of a package: the projects carrying the OBS:Maintained attribute that the package exists in, with where a branch would land. Read-only; requires OBS credentials

Usage: geectl maintenance maintained-instances [OPTIONS] --package <PACKAGE>

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --package <PACKAGE>
          The package whose maintained instances to list
      --attribute <ATTRIBUTE>
          The attribute marking a project as maintaining the package (`NAMESPACE:NAME`, default `OBS:Maintained`)
      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --update-project-attribute <UPDATE_PROJECT_ATTRIBUTE>
          The attribute pointing a project at its update project (`NAMESPACE:NAME`, default `OBS:UpdateProject`)
      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help
          Print help
```

## `service` — service reads and trigger

```
Service reads and trigger: list a package's services, their run state, and trigger a server-side run

Usage: geectl service [OPTIONS] <COMMAND>

Commands:
  list     List the services defined in a package's `_service` file
  state    Show the last service run state of a package
  trigger  Trigger a server-side run of a package's services. A mutation: dry-run is the default (only the plan is printed); `--live` performs the trigger. The trigger only queues the run: the server runs the services asynchronously, so a successful trigger says the run was queued, not that it finished. Check the outcome with `service state`
  help     Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
List the services defined in a package's `_service` file

Usage: geectl service list [OPTIONS] <PROJECT> <PACKAGE>

Arguments:
  <PROJECT>  The project the package lives in
  <PACKAGE>  The package to list the services of

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Trigger a server-side run of a package's services. A mutation: dry-run is the default (only the plan is printed); `--live` performs the trigger. The trigger only queues the run: the server runs the services asynchronously, so a successful trigger says the run was queued, not that it finished. Check the outcome with `service state`

Usage: geectl service trigger [OPTIONS] <PROJECT> <PACKAGE>

Arguments:
  <PROJECT>  The project the package lives in
  <PACKAGE>  The package to trigger the services of

Options:
      --format <FORMAT>    Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --service <SERVICE>  The service the trigger is for: it must be defined in the package's `_service`, and the trigger fails closed when it is not. The server runs the whole service set either way; without this flag, every service runs
      --debug              Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body         Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live               Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help               Print help
```

## `gitea` — Gitea operations

```
Gitea operations: repository and branch reads and mutations on the configured instance

Usage: geectl gitea [OPTIONS] <COMMAND>

Commands:
  repo-get     Show a repository: owner, default branch, privacy, clone URLs
  repo-list    List repositories: the token owner's own repos, one user's repos with `--owner`, one organization's with `--org`, or both (the user's first, then the organization's)
  repo-create  Create a repository under the token owner's account. A mutation: dry-run is the default (only the plan is printed); `--live` performs the creation
  repo-delete  Delete a repository. A mutation: dry-run is the default (only the plan is printed); `--live` performs the deletion. The library refuses without `--force`, even live
  repo-fork    Fork a repository into the token owner's account (or an organization with `--organization`). A mutation: dry-run is the default (only the plan is printed); `--live` performs the fork
  branch-list  List the branches of a repository
  branch-get   Show one branch: name, head commit, protection
  branch-head  Print the head commit SHA of a branch. Content command: the text IS the SHA, with no header
  maintainers  Show the maintainers recorded in a git project's `_maintainership.json`: the project maintainers and the per-package maintainers. The git-side counterpart of `remote maintainers`, read from the repo's default branch
  help         Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Create a repository under the token owner's account. A mutation: dry-run is the default (only the plan is printed); `--live` performs the creation

Usage: geectl gitea repo-create [OPTIONS] <NAME>

Arguments:
  <NAME>  The repository name

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --private
          Create a private repository; public by default
      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --description <DESCRIPTION>
          A short description
      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --default-branch <DEFAULT_BRANCH>
          The default branch; the server default when unset
      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help
          Print help
```

## `pull-request` — unified pull-request read and act

```
Read and act on pull requests: OBS submit requests and Gitea pull requests as one surface

Usage: geectl pull-request [OPTIONS] <COMMAND>

Commands:
  show                   Show one pull request: `obs:<id>` or `gitea:<owner>/<repo>#<number>`
  search                 Search pull requests: OBS submit requests and Gitea pull requests as one list, OBS first then Gitea. With no `--backend` both arms run; `--backend` scopes to one
  approve                Approve a pull request as the review asked of the target: with no target flag, as the review asked of the acting user (OBS) or the token owner (Gitea). Dry-run by default; mutates only with `--live`
  request-changes        Ask for changes on a pull request, as `approve` but with the review declined instead of accepted. Dry-run by default; mutates only with `--live`
  comment                Comment on a pull request. Dry-run by default; mutates only with `--live`
  comments               List the comments on a pull request, oldest first. A read, always live
  cancel-review-request  Cancel pending review requests on a pull request. Dry-run by default; mutates only with `--live`. On OBS the cancel fails closed: OBS has no request call that removes a reviewer
  merge                  Merge a pull request. On OBS this accepts the submit request; on Gitea it merges the pull request. Dry-run by default; mutates only with `--live`
  decline                Decline a pull request. On OBS this changes the request state to declined; on Gitea it closes the pull request. Dry-run by default; mutates only with `--live`
  withdraw               Withdraw a pull request. On OBS this revokes the request; on Gitea it closes the pull request. Dry-run by default; mutates only with `--live`
  reopen                 Reopen a pull request. On OBS this changes a declined request back to `new`; on Gitea it reopens the closed pull request. Dry-run by default; mutates only with `--live`
  submit                 File a submission from the working copy in the current directory: an OBS submit request from an OBS-native checkout, a Gitea pull request from a git-backed one. The checkout's backend decides; the other arm's flags are refused. Dry-run by default; mutates only with `--live`
  help                   Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show one pull request: `obs:<id>` or `gitea:<owner>/<repo>#<number>`

Usage: geectl pull-request show [OPTIONS] <REFERENCE>

Arguments:
  <REFERENCE>  The pull-request handle: `obs:<id>` or `gitea:<owner>/<repo>#<number>`

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

## `changelog` — `.changes` validation and generation

```
Validate and generate `.changes` entries in a package directory, pre-file

Usage: geectl changelog [OPTIONS] <COMMAND>

Commands:
  validate  Validate the `.changes` entries in a package directory against the V1–V12 rules. Local-only by default; with `--target-project`/`--target-package` the target-relative rules (V2, V4, V5 newest-vs-target, V11 older entries) run against the target's current files. Exits non-zero naming the failing rule
  generate  Generate a skeleton `.changes` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball. The output is a draft: what the diff cannot supply arrives as explicit `TODO` slots for a human to fill. `--write` prepends the draft into the package's `.changes` file, and refuses an incomplete draft (V9 needs-detail)
  help      Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Validate the `.changes` entries in a package directory against the V1–V12 rules. Local-only by default; with `--target-project`/`--target-package` the target-relative rules (V2, V4, V5 newest-vs-target, V11 older entries) run against the target's current files. Exits non-zero naming the failing rule

Usage: geectl changelog validate [OPTIONS] [DIR]

Arguments:
  [DIR]  The package directory (default: the current directory) [default: .]

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --target-project <TARGET_PROJECT>
          The target project to compare against
      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --target-package <TARGET_PACKAGE>
          The target package to compare against
      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --entries <ENTRIES>
          Newest N entries to lint for format, and the number of new entries the submission may carry vs the target (default 1: one entry per SR) [default: 1]
      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help
          Print help
```

```
Generate a skeleton `.changes` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball. The output is a draft: what the diff cannot supply arrives as explicit `TODO` slots for a human to fill. `--write` prepends the draft into the package's `.changes` file, and refuses an incomplete draft (V9 needs-detail)

Usage: geectl changelog generate [OPTIONS] [DIR]

Arguments:
  [DIR]  The package directory (default: the current directory) [default: .]

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --target-project <TARGET_PROJECT>
          The target project to diff against
      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --target-package <TARGET_PACKAGE>
          The target package to diff against
      --author <AUTHOR>
          The author line for the entry header, in `Name <email>` form — exactly what a bare `changelog` invocation writes
      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
      --write
          Prepend the draft into the package's `.changes` file instead of printing it. The whole file is normalized on write: trailing ASCII whitespace stripped, CRLF→LF, legacy encodings transcoded to UTF-8, a BOM stripped. The write is atomic and refuses a symlink destination, an unreadable file, or bytes no known encoding decodes. A file decoded from a guessed single-byte encoding is transcoded on the guess instead — a wrong guess cannot be detected — and the write warns about it, naming the encoding
  -h, --help
          Print help
```

## Working copy — git working copy

The working-copy leaves operate on the checkout in the current
directory: `checkout` writes `<dir>/<project>/<package>/` under the
current directory (a `:` in a project or package name becomes `_` in
the directory name), the rest open the checkout found there. `commit`
runs the source services and `pre_checkin.sh` before sending, unless
`--noservice` is passed. The base record lives in the checkout's git
config (`geectl.apiurl`, `geectl.project`, `geectl.package`,
`geectl.backend`, `geectl.base-rev`, `geectl.base-srcmd5`).

```
Check out a package into a git working copy, under the current directory

Usage: geectl checkout [OPTIONS] <PROJECT> <PACKAGE>

Arguments:
  <PROJECT>  The OBS project
  <PACKAGE>  The OBS package

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --rev <REV>        The revision to check out (a number or a source md5). The latest when omitted
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Update the working copy in the current directory to the latest base

Usage: geectl update [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --rev <REV>        The revision to update to (a number or a source md5). The latest when omitted
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show the working copy's state against the base

Usage: geectl status [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show the diff of the worktree and the index against the base, plus the link diff on a linked checkout

Usage: geectl diff [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Stage files for commit

Usage: geectl add [OPTIONS] <FILES>...

Arguments:
  <FILES>...  The files to stage, relative to the checkout root

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Remove files from the working copy

Usage: geectl rm [OPTIONS] <FILES>...

Arguments:
  <FILES>...  The files to remove, relative to the checkout root

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Stage new files and worktree deletions

Usage: geectl addremove [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Commit the staged changes

Usage: geectl commit [OPTIONS] --message <MESSAGE>

Options:
      --format <FORMAT>    Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
  -m, --message <MESSAGE>  The commit message
      --debug              Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --noservice          Skip the service gate (no source services, no `pre_checkin.sh`)
      --debug-body         Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live               Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help               Print help
```

```
Pull a linked package to the link target's latest revision

Usage: geectl pull [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Stage the resolved files and continue a stopped rebase

Usage: geectl resolve [OPTIONS]

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

## `prjconf` — project-config reads and cleanup

```
Read, validate and clean a project's prjconf

Usage: geectl prjconf [OPTIONS] <COMMAND>

Commands:
  show      Show a project's raw prjconf
  validate  Validate a project's prjconf across its repositories. Exits 24 when error findings exist; warnings alone do not fail the run
  clean     Clean a project's prjconf. A mutation: dry-run is the default. Shares validate's credential gate and 32-repository bound
  help      Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Show a project's raw prjconf

Usage: geectl prjconf show [OPTIONS] <PROJECT>

Arguments:
  <PROJECT>  The project to read the prjconf of

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

```
Validate a project's prjconf across its repositories. Exits 24 when error findings exist; warnings alone do not fail the run.

The config is evaluated once per repository and architecture, with one buildconfig fetch per repository; package and owner lookups are cached across evaluations. At most 32 repositories are evaluated; a project with more is refused. Requires OBS credentials; without them the command fails closed before any request. A `%if` branch is reported dead only when it is dead in every evaluation.

Usage: geectl prjconf validate [OPTIONS] <PROJECT>

Arguments:
  <PROJECT>
          The project to validate the prjconf of

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json`
          
          [possible values: json, text]

      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag

      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`

      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations

  -h, --help
          Print help (see a summary with '-h')
```

```
Clean a project's prjconf. A mutation: dry-run is the default. Shares validate's credential gate and 32-repository bound.

A prjconf is inherited: downstream projects concatenate this config with different repository paths, architectures and `_repository` / `_project` values. By default clean removes only lines proven dead independent of the repository path and architecture (same-line duplicate tokens and unconditionally overridden `Substitute` lines); `--include-path-dependent` also removes lines proven dead in every repository and architecture evaluation of this project.

Usage: geectl prjconf clean [OPTIONS] <PROJECT>

Arguments:
  <PROJECT>
          The project to clean the prjconf of

Options:
      --format <FORMAT>
          Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json`
          
          [possible values: json, text]

      --include-path-dependent
          Also remove lines proven dead only in this project's own repository and architecture evaluations

      --debug
          Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag

      --debug-body
          Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`

      --live
          Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations

  -h, --help
          Print help (see a summary with '-h')
```

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
geectl request show 1398977
geectl request search --creator alice --state new
geectl request search --reviewer alice
geectl request diff 1398977

# Branch, then file (dry-run first, `--live` to perform)
geectl branch branch-package --project openSUSE:Factory --package vim
geectl --live branch branch-package --project openSUSE:Factory --package vim
geectl request file --source-project home:alice:branches:openSUSE:Factory --source-package vim --target-project devel:languages:python --target-package vim --description "Update vim to 9.1" --changes-entry "- Update to 9.1"

# Maintenance
geectl maintenance maintained-instances --package vim
geectl maintenance maintenance-branch --package vim

# Services
geectl service list openSUSE:Factory vim
geectl service state openSUSE:Factory vim
geectl --live service trigger openSUSE:Factory vim

# Gitea (needs GEECTL_GITEA_TOKEN, even in dry-run)
geectl gitea repo-get pool systemd
geectl gitea branch-head pool systemd factory
geectl gitea maintainers pool systemd
geectl gitea repo-create vim --private

# Pull requests (OBS + Gitea as one surface)
geectl pull-request show obs:1398977
geectl pull-request show gitea:openSUSE/geectl#122
geectl pull-request search --backend gitea --state open

# .changes validation and generation (local, pre-file)
geectl changelog validate ~/work/foo
geectl changelog validate ~/work/foo --target-project openSUSE:Factory --target-package foo
geectl changelog generate ~/work/foo --author 'Jane Packager <jane@example.com>'
```

Validate a package directory before filing: `changelog validate` exits
24 (`validation_failed`) naming the failing rule, and lists the
skipped target-relative rules when no target is given. `changelog
generate` prints a draft entry — a bare version line plus explicit
`TODO` slots — never a submission; `--write` prepends it into the
package's `.changes` file and refuses a draft with unfilled
`TODO` slots or a needs-detail flag. Writing normalizes the whole
file: trailing ASCII whitespace is stripped from every line,
CRLF becomes LF, legacy-encoded bytes are transcoded to UTF-8
(best-effort; valid UTF-8 is never re-decoded), a BOM is stripped,
and a UTF-16 file is decoded whole first. The write is atomic (a
temp file in the same directory, then a rename, keeping the
destination's permissions) and refuses a symlink destination, an
unreadable file, or bytes no known encoding decodes; the guessed
encoding is named on the debug channel. One limit remains: a file
decoded from a guessed single-byte encoding is transcoded on the
guess, never refused — a single-byte guess cannot report a decode
error, so a wrong guess (a corrupt byte disabling the multi-byte
probers) still decodes cleanly and the valid UTF-8 half can be
rewritten as mojibake. The write warns about the guess on stderr,
naming the encoding, instead of staying silent.

The `request file` flags above are illustrative — run
`geectl request file --help` for the exact flag set. The same holds
for every leaf: the binary's `--help` is the authority, and this doc
reproduces it verbatim where it quotes it.

The `request file` JSON's `cve` field carries the `--cve` values
parsed and normalized to uppercase `CVE-<year>-<number>` (year 1999
or later, number at least 4 digits), never the raw strings.

## How it differs from osc

geectl is not osc with a new name: one noun per operation family,
explicit flags instead of positionals, no working-copy inference,
JSON by default with opt-in text, and dry-run by default for
mutations. The full list of deliberate deviations, with reasons, is
[docs/cli-swaps.md](cli-swaps.md).
