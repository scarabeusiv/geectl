# geectl CLI usage

The command reference for the geectl binary: one subcommand per
operation, grouped under a top-level noun (`remote`, `request`,
`branch`, `build`, `maintenance`, `service`, `gitea`), plus `version`
and `config`.
Every `--help` block below is the real output of the built binary
(`geectl 0.2.1`); run `geectl --help` or `geectl <group> --help` for
the same text live.

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
```

The plain `GEECTL_OBS_USERNAME` / `GEECTL_OBS_PASSWORD` (or
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
credentials this way. (The API-key credential, when it lands, uses
the same pattern: `GEECTL_OBS_APIKEY_<ALIAS>`.)

A half-configured credential pair (a username with no password source,
or the reverse) is handled per group: `request` swallows it in dry-run
(a preview never needs credentials; the dropped error goes to the
debug channel), while `branch`, `remote`, `build`, and `maintenance`
fail fast on it before any work, in dry-run too — a dry run that
cannot be performed live is not a useful preview. A live run resolves
credentials up front in all groups, and so does every read except
`build list-repositories`, a public read that never touches them.

## Top level

```
Geeko Controller — a machine-first CLI for agents (OBS + src.opensuse.org)

Usage: geectl [OPTIONS] [COMMAND]

Commands:
  version      Print version information as the result payload
  config       Inspect or initialise the config file
  remote       Remote reads and repository edits: inspect what the distribution holds
  request      Read and act on OBS submit requests, and file new ones
  branch       Branch, copy, and link packages
  build        Build-status commands: results, logs, histories, binaries
  maintenance  Maintenance operations: maintained-instances listing and maintenance branching
  service      Service reads: list a package's services and their run state
  gitea        Gitea operations: repository and branch reads and mutations on the configured instance
  help         Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
  -V, --version          Print version
```

## `remote` — inspection, plus repository edits

```
Remote reads and repository edits: inspect what the distribution holds

Usage: geectl remote [OPTIONS] <COMMAND>

Commands:
  cat                Print a source file to standard output. Text only: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes
  list-files         List the files of a package
  list-packages      List the packages of a project
  list-projects      List the top-level projects on the instance
  package-log        Show the commit history of a package
  revision-patch     Show the per-file diff of one revision against the previous one. Computed client-side from per-file reads (the library lists both revisions and cats each changed file), not a server-side diff; a differing binary file fails the call
  project-meta       Show a project's meta (title, description, persons, repositories)
  package-meta       Show a package's meta (title, description, persons, devel project)
  search-packages    Search packages by name. Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches (exact matching is not provided server-side, so the filter runs client-side)
  search-projects    Search projects by name. Substring matches on name, title, and description by default; `--exact` keeps only exact-name matches (exact matching is not provided server-side, so the filter runs client-side)
  owners             Show the owners of a package (project/package scope)
  owners-for-binary  Show the owners of a binary package
  maintainers        Show the maintainers of a package
  whois              Show a user's full name, email, and account state
  link-target        Show the link target of a package, when it is a link
  project-diff       Diff two projects, package by package. Computed client-side: the library lists both projects and reads every file of every differing package, so a large diff issues many requests and a differing binary file fails the call
  diff-packages      Diff two packages, file by file. Computed client-side: the library diffs the two file lists and cats each changed file, so a differing binary file fails the call
  devel-project      Show the devel project and devel package of a package on Factory, when it has one
  repo-add           Add a repository to a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the addition
  repo-remove        Remove a repository from a project. A mutation: dry-run is the default (only the plan is printed); `--live` performs the removal. A repository that is not there is not an error
  help               Print this message or the help of the given subcommand(s)

Options:
      --format <FORMAT>  Output format for the result payload. Applies to every command. Default: the `GEECTL_FORMAT` environment variable, else the `format` key in the config file, else `json` [possible values: json, text]
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
      --debug-body       Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies `--debug`
      --live             Actually perform mutations. Without it, only the plan is printed: dry-run is the default for mutations
  -h, --help             Print help
```

One leaf, to show the flag style (required coordinates as
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
      --rev <REV>        Print the file as of this revision (default: latest)
      --debug            Enable the debug channel: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; `GEECTL_DEBUG=1` enables it without the flag
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
  diff                         Show the diff of a submit request: the unified diff, or the request XML with the diff injected with `--xml`
  list-comments                List the comments on a submit request, in id order
  accept                       Accept a submit request, with a comment as the message left with the accept
  decline                      Decline a submit request, with a comment as the reason
  revoke                       Revoke (withdraw) a submit request, with a comment as the reason
  change-state                 Change a submit request's state to one of new/review/accepted/declined/revoked
  try-change-state             Best-effort state change: try it, and report whether it happened instead of failing when the request cannot move
  add-review                   Ask a user, group, project, or package to review a submit request
  add-comment                  Comment on a submit request, optionally as a reply to another comment
  file                         File a submit request: source package to a devel project
  refile                       File a submit request and supersede an old one with it: the new request is filed first, and only then is the old one superseded pointing at it
  supersede                    Supersede one request with another existing one. A mutation that needs `--live`: the library's raw supersede has no dry-run, so without `--live` this command refuses — use `request refile --supersede` for the dry-runnable flow
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
Build-status commands: results, logs, histories, binaries

Usage: geectl build [OPTIONS] <COMMAND>

Commands:
  results            Read the build-results matrix for a project
  wait-for-results   Poll the build results until they settle, or the deadline passes
  build-log          Read the build log for a package
  build-status       Read the build status code for a package
  job-status         Read the running build job for a package, if any
  buildinfo          Read the buildinfo for a package: the blocking error, if any, and the dependencies the build pulls in
  build-history      List the completed builds for a package, newest first
  job-history        List the build jobs that ran in a repository, newest first
  list-binaries      List the built files for a package (or the whole repository)
  download-binary    Download one built file by name
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
  -h, --help                     Print help
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

## `service` — service reads

```
Service reads: list a package's services and their run state

Usage: geectl service [OPTIONS] <COMMAND>

Commands:
  list   List the services defined in a package's `_service` file
  state  Show the last service run state of a package
  help   Print this message or the help of the given subcommand(s)

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

## Common tasks

```sh
# Read-only: what the distribution holds
geectl remote list-packages openSUSE:Factory
geectl remote list-files openSUSE:Factory vim
geectl remote cat openSUSE:Factory vim vim.spec
geectl --format text remote list-packages openSUSE:Factory

# Build status
geectl build results openSUSE:Factory --code failed
geectl build build-log openSUSE:Factory vim standard x86_64
geectl build list-binaries openSUSE:Factory standard x86_64 --package vim

# Submit requests
geectl request show 1398977
geectl request search --creator alice --state new
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

# Gitea (needs GEECTL_GITEA_TOKEN, even in dry-run)
geectl gitea repo-get pool systemd
geectl gitea branch-head pool systemd factory
geectl gitea repo-create vim --private
```

The `request file` flags above are illustrative — run
`geectl request file --help` for the exact flag set. The same holds
for every leaf: the binary's `--help` is the authority, and this doc
reproduces it verbatim where it quotes it.

## How it differs from osc

geectl is not osc with a new name: one noun per operation family,
explicit flags instead of positionals, no working-copy inference,
JSON by default with opt-in text, and dry-run by default for
mutations. The full list of deliberate deviations, with reasons, is
[docs/cli-swaps.md](cli-swaps.md).
