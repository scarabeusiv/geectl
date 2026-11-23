# geectl local-build vertical — design

Issue: https://github.com/scarabeusiv/geectl/issues/43
Depends on: #17 (working-copy layer — the build takes a checkout directory),
the `_buildinfo` read, #23 (git vs OBS abstraction).

## The decision, up front

The skill needs a local build, and OBS supports too many build backends to
mirror. Geectl grows **exactly one building vertical: podman, for quick
debug**. The engine is the openSUSE `build` script, reused exactly as the
existing tool uses it — geectl prepares the inputs and reads the result, it
does not reimplement container mechanics, dependency installation, or RPM
creation.

One operation, `build`, taking a local source directory
plus an explicit repo and arch — plus three small root-management operations
(`build-shell`, `build-wipe`, `build-log`) that come with the vertical
because they are the same build root.** Caching is part of the vertical:
a shared pkg cache (hdrmd5-verified) and sccache (per-root storage),
both under one geectl cache dir, both locked.

## Local scope

The local vertical owns service runs, tarball fetching, and
spec-cleaner runs. It is the agent's quick-debug loop: fetch the tarballs
with the service binaries, run spec-cleaner, build — all inside the one
podman container, before anything goes to the server. The server stays the
source of truth for service results; local is for iterating fast.

How it connects to the services batch (#19): local-mode service execution
(`manualrun`, `run NAME`, `precommit`-style runs) lives here, via
`build-shell` + direct service binary — the server has exactly three
service commands (trigger, wait, merge) and
the local-mode commands are local executions of service binaries. An agent
debugging a service runs it in the build container with `build-shell` and
inspects the result before committing. Tarball fetching is the same shape:
the source-fetching services (`download_url`, `tar_scm`, `obs_scm`, …)
run locally so the agent can inspect the fetched sources. Spec-cleaner
runs in the container the same way (the packaging skill runs
spec-cleaner inside an openSUSE container).

What the vertical does NOT own:

- Server-side service runs (trigger/wait/merge) — those are #19's server
  operations, not local execution.
- `buildtime` services — they run on the OBS worker during the build,
  never in a checkout, never locally.
- `runall` — never; mode-blind, it would fire `buildtime` services
  locally.

The container needs the service binaries present (`obs-service-*`
packages); they do not come with a plain Tumbleweed image. How they get
into the build container is not specified here.

## The operation, mapped

| Operation | What it does | New server calls |
|---|---|---|
| `build` | Build the package in `<dir>` for `<repo>` / `<arch>` in a podman container, native arch only. Returns a typed result (verdict, exit code, log path, RPMs, rpmlint summary). | `buildinfo` (GET `/_buildinfo`, the resolved dep list — a prerequisite pure-API op, not part of this vertical) |
| `build-shell` | Run a scripted command array as shell fragments inside the preserved build root (`chroot` equivalent). No TTY, bounded timeout, stdout/stderr captured as data. The `local-build chroot` leaf is the CLI surface: it reads the fragments from stdin, one per line. | None |
| `build-wipe` | Delete a build root. Refuses when a build holds the root's lock, and refuses when the directory is not a geectl root (marker file). | None |
| `build-log` | Read the last build's log (`<root>/.build.log`) — tail, or the whole log up to the 4 MiB read cap by default, uncapped on request — plus the recorded verdict. | None |
| `build-prune` | Print the eviction plan (what would be evicted, in order, with bytes); delete only with `--apply`. Same policy and safeguards as automatic eviction. | None |
| `periodic-install` | Write the scheduler entry for this platform and enable it (daily 03:00 `prune --apply`); dry-run is the default, `--live` performs the install. | None |
| `periodic-uninstall` | Disable the scheduler entry, then remove its files; dry-run is the default, `--live` performs the uninstall. | None |
| `periodic-status` | Show whether the daily cleanup is installed and enabled. Read-only. | None |

`build` inputs: source directory (spec + sources — a checkout from #17,
a git clone, or any dir with a spec), `repo` (explicit name, e.g.
`standard`), `arch` (explicit; must equal the host arch — see scope).
Options: `--clean`, `--flavor` (multibuild), `--release N`,
`--trust-all-projects` (see trust), `--refresh-image`, `--timeout SECS`,
`--alternative-project` (fetch project), `--root` (build root),
`--noinit`, `--preinstall-image`, `--keep-rpms <dir>` (`-k`), `--prefer-packages <dir>` (`-p`).
No other options in v1: no `--jobs`/`--threads` tuning, no `--with`/
`--without`, no `--define` — the full option matrix stays out.

## Engine: reuse the `build` script (how the existing tool does it)

The existing tool's invocation, from `osc` `build.py`:

- `osc build` is a thin wrapper around the `build` script (`build-cmd`):
  `build --root=<root> --rpmlist=<file> --dist=<buildconfig>
  --arch=<buildarch> --vm-type=podman --vm-disk=<root>/img
  --vm-swap=<root>/swap --logfile=<root>/.build.log <spec>` plus
  buildargs (`--statistics`, `--norootforbuild`, `--clean`, `--ccache` /
  `--sccache-uri=...`). Geectl invokes the same command, same flags.
- Podman builds run as the user (rootless, no `su` wrapper —
  `calculate_build_root_user` returns the user for podman; `build_as_user`
  is true). Geectl never runs the engine as root and never asks for sudo.
- The wrapper fetches `_buildinfo` / `_buildconfig` from the OBS API and
  caches them in the store, downloads the resolved deps into a package
  cache (`fetcher.run(bi)`), verifies every cached package by hdrmd5
  (`verify_pacs`), writes the dep list to an `rpmlist` file, then runs the
  engine. Geectl does the same preparation with its own typed calls. The
  library transport caps response bodies at 10 MiB; a dep RPM over the cap
  streams to disk instead of failing.
- Dep downloads are mirror-first: the buildinfo's `<path url>` values
  are the mirror base URLs for their (project, repository); a dep
  whose repo has one is fetched from `<url>/<arch>/<filename>` with no
  credentials before the API is tried. Any mirror failure — network
  error, 404, hdrmd5 mismatch — falls back to the API download, which
  keeps its own retry and name-warming behaviour. A repo with no
  `<path url>` (unpublished, or a local instance) is API-only. The
  mirror rules:
  - **Scheme and host.** Only public `https` mirror URLs are used
    (the scheme is lowercased first, so `HTTPS://` counts): plain
    `http`, userinfo, query, and fragment are rejected, and so are
    loopback, link-local, and private hosts. The check runs twice —
    at buildinfo parse (a rejected URL is dropped and counted in
    `skipped_paths`, logged on the debug channel) and at use, so a
    directly built buildinfo cannot smuggle one in.
  - **Digest required.** The mirror is tried only when the buildinfo
    carries a usable hdrmd5 for the dep. A missing digest, or the
    download-on-demand placeholder, goes straight to the API, which
    warms the binary and verifies against the server-computed digest.
  - **Redirects.** The mirror download follows up to 5 redirects,
    staying on the same scheme (so `https` to `https` in production,
    never a downgrade to `http`), with no credentials past the first
    hop. `download.opensuse.org` answers 302 to its backing store,
    so without this the mirror path never succeeds.
  - **Arch segment.** The URL uses the dep's own package arch,
    falling back to the repo arch only when the dep names none: a
    noarch dep reads from `noarch/`, not from the repo arch. Arch
    and file name are percent-encoded like the API path.
  - **Single-shot, dead-mirror skip.** A mirror attempt is one
    request with no retries and no backoff, bounded by the
    transport's timeouts. A health failure (transport, 5xx, timeout,
    redirect, over-cap) marks the (project, repo) mirror dead for
    the rest of the build; a per-file failure (404, hdrmd5 mismatch)
    does not. The attempt runs inside the per-package cache lock,
    so a dead mirror costs at most one attempt per build, not one
    per dep.
  - **Streamed, capped, verified once.** The mirror download streams
    straight to a temp file — never buffered in memory, never
    downloaded twice — capped at 1 GiB (over the cap falls back to
    the API, which streams uncapped, so the cap never breaks a
    build). The temp file is hdrmd5-verified once, in the mirror
    download, and lands in the cache without a second hash.
  - **Integrity limit.** hdrmd5 (MD5 over the header and payload)
    is the only integrity check on a mirror download: there is no
    repomd or package-signature check, and MD5 itself is
    collision-feasible. The digest comes from the buildinfo the
    trusted OBS server sent, the transport is `https` to a public
    host, and a body that fails the digest never lands in the cache
    — but a mirror that serves a different file with a colliding
    digest is not detected. A stronger check (SHA from trusted
    metadata, or a signature) is future work, not this change.
  - **Malformed `<path>`.** A `<path>` without a project or
    repository is skipped and counted, never fatal: mirror data is
    only an optimisation, and the dep fetch falls back to the API
    for that repo.
- Dep file names resolve from the buildinfo's `<bdep>` attributes:
  the scheduler sends no `binary` attribute, so the file name is computed
  locally as `name-version-release.arch.rpm` from `version`/`release`/`arch`
  (the epoch is never part of a file name); without all three, the binary
  list of the dep's source package (`package` attribute) resolves it. List
  and download URLs key on the source package, never the binary name — a
  subpackage binary (`libz1`) lives under its source package (`zlib`), and
  the binary name as the package segment 404s both the list and the download. Without a source package the whole repository is listed
  (`_repository`), which the server serves for both list and download.
- Built files are collected from `<root>/.build.packages` (RPMS/SRPMS).
  On failure the wrapper prints the exit code, the root path, and suggests
  `--clean` / wipe. Geectl returns all three as data, not prose.
- The preinstall image, when the buildinfo carries one and
  `--preinstall-image` is set, seeds a fresh root: the image
  downloads into the pkg cache (streamed, magic-checked — the
  buildinfo digest is a synthetic image id, not a content digest),
  and its `.info` file downloads alongside it. The covered
  dependencies get no download and no `<name> <file>` line, but
  keep their place in the install lists, and the rpmlist carries
  the image lines for the engine. Any image failure drops the
  image and the build proceeds from plain deps. The image is
  fetched with a single GET and never retried, so a
  download-on-demand repository serves no image. A root whose init
  already ran never selects an image. Without the flag the offer
  is parsed and never used: the engine extracts it with bsdtar
  `--chroot` plus device nodes, which fail as a non-root user.
  Geectl does not build images itself.
- The engine is an external dependency: geectl checks for the `build`
  binary at startup of the operation and fails closed with
  `NoBuildEngine` (naming how to install it) when absent. The engine
  version is recorded in every build record, so a skew is diagnosable.

## Image

- **Default: `registry.opensuse.org/opensuse/tumbleweed:latest`.** The
  default target is Factory/Tumbleweed; the image is the Tumbleweed
  base plus the `build` package and its helpers, as the engine requires.
- The image digest actually used is resolved at the start of every build
  and recorded in the build record, so a build is reproducible from the
  record. The image is pulled when absent; `--refresh-image` forces a
  pull; otherwise a local image older than 7 days is refreshed.
- Leap/SLE images are out: one image, one target. A Leap pool build stays
  OBS-ONLY.

## Cache layout

One geectl directory, default `$XDG_CACHE_HOME/geectl/` (`~/.cache/geectl/`,
overridable by config `build.cache_dir`):

- `pkg/<project>/<repo>/<arch>/` — the shared package cache. One dir per
  project+repo+arch, so a Tumbleweed dep can never be installed into a Leap root
  (or vice versa), and the same repo name plus the same file name from two
  projects never collide in one dir. The layout matches the existing
  tool's `cachedir/project/repository/repoarch`. Files are named by RPM filename, written to a temp
  file and atomically renamed (R16), and verified against the hdrmd5 in
  the buildinfo on every use: a mismatch is re-downloaded once, then a
  structured `CacheCorrupt` error — never silently reused, never silently
  skipped (the existing tool's positive pattern: download verification by
  default). On every use geectl touches the file (mtime = last use);
  verification is by content (hdrmd5), so touching is safe — the
  eviction policy orders pkg files by mtime.
- `sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the compiler cache, one store per
  build root. The existing tool passes `--sccache-uri=/var/tmp/osbuild-
  sccache-{pkgname}.tar` with an explicit comment: per-package storage
  exists "to prevent cross-cache pollutions". Geectl keeps that rule
  and tightens it to per-root: two concurrent builds of the same package
  in different roots hold different root locks, so a shared per-package
  tar would be written by both at once and corrupt; a per-root tar is
  guarded by its own root's flock. Repeat builds of the *same* root
  share compiled objects; different roots never share a store. The store directory's mtime is updated
  on every build that uses it (eviction orders stores by last use).
  ccache is not offered —
  one vertical, one compiler cache. There is no `--compiler-cache`
  argument.
- `build-roots/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the
  build root, named the same way the existing tool's `build-root` template
  names it (project/package/repo/arch), so `build-summary`-style tooling
  and humans can find it. A `:` in a project, package, repository,
  arch, or flavor name is percent-encoded in the directory (`:`
  becomes `%3A`, `%` becomes `%25`); the encoding is injective, so
  `home:foo` and `home_foo` are different projects with different
  roots, and the marker keeps the real name. The same encoded form
  is used for the `sccache/` and `pkg/` layouts above. Directories
  written before the encoding keep raw `:` in their names; new
  builds never reuse them, but eviction still walks and can delete
  them. A pkg lock held by a concurrently running pre-upgrade
  binary is not honored for those legacy paths, because the lock
  lookup uses the encoded form. A legacy sccache store is still
  guarded at delete time — its root lock is held via the verbatim
  store-to-root path, so a locked root is not deleted under a
  running build (only the plan-time protection misses it). The root carries a `.geectl-build-root` marker
  file (image digest, engine version, repo/arch/flavor, created time,
  last build time — the eviction policy orders roots by it);
  `build-wipe` refuses a directory without the marker.

Locking (R15 — the proven failure mode is two builds sharing one root):

- The pkg cache is shared across builds and never serializes them: only
  the per-file download/write path takes a lock, briefly, on
  `<project>/<repo>/<arch>.lock` (`flock`); readers use the atomically renamed
  files and need no lock.
- The build root and the root's sccache store are exclusive: one
  build per (package, repo, arch, flavor) at a time, guarded by `flock`
  on the root's lock file — and the store is per-root, so the same
  lock guards the tar too. A second invocation refuses with
  `BuildInProgress`, naming the holder — fail fast, the
  agent retries. A lock that frees within ~100 ms is retried, not
  refused: a `Command::spawn` in another thread forks the process,
  and the forked child holds the lock file's open file description
  until exec closes it, so a just-released lock can read as held for
  a few milliseconds — a real build holds it far longer, so it still
  refuses promptly. A destructive op (`--clean`, `build-wipe`) refuses while
  the lock is held.
- Missing cache is never an error: an absent pkg file is downloaded, an
  absent sccache store starts empty (cold first build, warm after). A
  stale cache is detected by hdrmd5 (pkg) or by the sccache store's own
  versioning; geectl never evicts cache data silently — eviction follows
  the Eviction policy below and every deletion is recorded in the build
  record.

## Eviction

Build roots, the pkg cache, sccache stores and
images grow without bound, and nobody prunes until the disk is full.
The vertical evicts automatically — but only regenerable items, only
under the safeguards below, and every deletion is recorded in the build
record. Nothing in the cache dir is irreplaceable: the source checkout
lives outside it and is never touched; a pkg file is a re-download, a
sccache store is a recompile, a root is a dep reinstall, an image is a
re-pull. Eviction costs time, never data.

### Trigger: a size cap, enforced at build start

- One cap over the whole cache dir (`pkg` + `sccache` + `build-roots` +
  images pulled for the vertical), default **20 GiB**, tunable by config
  `build.cache_cap_gib`. Low-water is 80% of the cap (16 GiB at the
  default) and is not separately tunable — one knob.
- Enforcement point: at the start of `build`, **after** the buildinfo
  fetch (so the current build's resolved dep set is known) and **before**
  any download. If usage is above the cap, evict until usage is at or
  below the low-water mark.
- Why a size cap and not the alternatives: age alone cannot bound disk
  usage — a busy day fills the disk in hours, whatever the ages. An
  on-demand prune op alone relies on someone remembering to run it.
  A size cap is the only
  trigger that bounds usage, and enforcing it at build start means the
  build never evicts mid-flight and the decision is made from a full
  inventory.
- The build is never refused for size. If everything above the
  low-water mark is protected (locked, recent, or the current build's
  deps), eviction frees what it can and reports the shortfall
  (`shortfall_bytes` in the build record); the build still runs. The cap
  bounds what geectl retains, not the disk itself — a disk full for other
  reasons is outside this vertical.

### What gets evicted, in what order

LRU within each class, cheapest to regenerate first. Last use is the
file's mtime: geectl touches a pkg file on every use, a sccache store on
every build that uses it, and the root's marker file records the last
build time (see Cache layout).

1. **Pkg cache files** (`pkg/<project>/<repo>/<arch>/`). Cheapest: re-download
   from the repos, hdrmd5-verified on use, and a missing file is
   downloaded anyway — evicting one can only cost a re-download, never
   a failure. Oldest mtime first.
2. **sccache stores** (`sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/`). Whole stores,
   oldest last-use first. A missing store starts empty (cold build).
   Never a store whose package has a locked build root.
3. **Build roots** (`build-roots/.../`). Only roots that are unlocked,
   not the most recent for their (project, package),
   and older than the grace period. Oldest last-build first. Evicting
   a root deletes the built RPMs inside it — the most-recent-root rule
   protects the newest root of the package; an older root's outputs
   are rebuilt by reinstalling its deps (from the pkg cache when
   still cached, else re-downloaded from OBS). Grouping is by
   project+package: an older root of the same package in another repo,
   arch or flavor can evict once a newer root of that package exists;
   grouping by the full 5-tuple protects every root and roots
   effectively never evict.
4. **Images.** Last, and only superseded digests: an older digest of
   the image may be removed once a newer digest has been pulled **and**
   at least one build has succeeded with it. Never the digest the most
   recent build recorded, never the digest `latest` currently resolves
   to.

### What never gets evicted

- A build root whose lock is held (a build in progress). Eviction
  attempts a non-blocking `flock` on each root's lock file; held means
  skip. This is the hard rule — deleting a root a running build is
  inside would corrupt that build.
- The most recent root per (project, package) — older roots of the
  same package in other repos, arches or flavors are regenerable
  (their deps reinstall — from the pkg cache when still cached,
  else re-downloaded from OBS), so only the newest is
  protected. Root reuse itself stays per 5-tuple: that repo's next
  build recreates its root rather than reusing the newest one.
- The current build's resolved dep set (from the buildinfo fetched
  before eviction). A build's own eviction run can never delete a file
  that same build is about to install.
- Anything used within the grace period: **24 hours** since last use. A
  build about to use a file has used it recently, or it is the current
  build (excluded above).
- The current image digest (see above).
- Anything outside the cache dir, and any directory without the
  `.geectl-build-root` marker (for roots). Eviction only touches what
  geectl created.

### Safeguards

- Eviction runs only at build start and in `build-prune`, never during
  a build. One eviction at a time: a global `eviction.lock` in the cache
  dir, taken non-blocking — if another eviction holds it, this run skips
  eviction and the build proceeds (the other run covers it).
- Eviction takes each project-repo-arch lock while deleting that dir's
  pkg files, so a delete can never race a download's atomic rename.
- At apply time a root's lock is held across its delete, and a sccache
  store's root lock across the store's delete: the plan-time probe is
  stale by then, and a build starting in between must not lose its
  root (or its store) under it. The root's lock file itself goes last
  in the delete, so a build cannot enter a root being deleted.
- No silent deletion: every eviction is recorded in the build record
  (see below). The manual path is inventory-first: `build-prune` prints
  the eviction plan by default and deletes only with `--apply`.
- No trash dir. Everything evicted is regenerable, so a recovery window
  buys nothing, and a trash dir would make the cap dishonest (trashed
  bytes still occupy the disk while the cap counts them as freed). The
  build record is the audit trail, not a trash can.

### What the build record reports

Usage is reported by `cache_usage` (`total_bytes` with per-area
`pkg_bytes`, `sccache_bytes`, `roots_bytes`, `image_bytes`), and the
build record carries `cache` (pkg hits/misses/preferred, sccache hits); eviction
adds an `eviction` object to every build record:

- `ran` (bool), `cap_bytes`, `low_water_bytes`, `bytes_before`,
  `bytes_after`, `freed_bytes`
- `evicted`: per item, `kind` (pkg / sccache / root / image), `path`,
  `bytes`, `reason` (`lru`, `superseded-image`)
- `protected_bytes` (bytes above low-water that could not be evicted),
  `shortfall_bytes` (cap not met)
- `build-prune` reports the same object for a manual run (plan only,
  without `--apply`).

### Eviction parameters

1. **Default cap 20 GiB (low-water 16 GiB).** A Tumbleweed root is a
   few GiB, so 20 GiB holds several packages' roots plus the pkg cache
   and sccache with headroom, and it is small enough to be a real bound
   on a dev disk. Tunable via `build.cache_cap_gib`.
2. **Grace period 24 hours.** It covers the daily build rhythm: a file
   a second build the same evening needs was used within the last day.
   Shorter (1 hour) risks evicting a file the next build needs; longer
   (7 days) blunts eviction when the disk is genuinely full.
3. **`build-prune` prints the plan by default, deletes only with
   `--apply`.** Manual cleanup is inventory-first; the automatic
   build-start path is the only one that deletes without a prior
   proposal, and only under the safeguards above.

## Periodic cleanup

Eviction runs at build start only: a cache that stops being built
against keeps its stale roots until the next build. The periodic
cleanup runs the eviction pass on a daily schedule, under the same
policy and safeguards as automatic eviction.

- **The schedule runs `prune --apply` daily at 03:00 local time.**
  The time is the same on every platform: the systemd timer carries
  `OnCalendar=*-*-* 03:00:00`, the launchd agent a
  `StartCalendarInterval` of hour 3, minute 0, and the scheduled task
  a daily trigger at 03:00. Pinned by
  `systemd_units_carry_the_binary_and_0300` (`src/periodic.rs:1420`),
  `launchd_plist_carries_the_binary_log_and_0300`
  (`src/periodic.rs:1448`) and
  `scheduled_task_xml_carries_the_binary_and_daily_0300`
  (`src/periodic.rs:1466`).
- **Linux: a systemd user timer.** `geectl-prune.service` +
  `geectl-prune.timer` in `~/.config/systemd/user/` (or
  `$XDG_CONFIG_HOME/systemd/user/`), enabled with `systemctl --user
  enable --now`. The unit files live under `packaging/systemd/` and
  are the single source of truth: the install command renders them
  with the running binary's path, and the RPM package will ship the
  same files (separate packaging track; the spec substitutes
  `@GEECTL_BIN@` with the packaged binary path). Pinned by
  `linux_plan_writes_both_units_and_enables_the_timer`
  (`src/periodic.rs:1509`).
- **macOS: a launchd agent.** `io.github.scarabeusiv.geectl.prune.plist`
  in `~/Library/LaunchAgents/`, loaded with `launchctl load`, logging
  to `~/Library/Logs/geectl-prune.log`. A re-install unloads the
  prior load first, so it picks up the new plist instead of failing
  on an already-loaded agent; install and uninstall ask
  `launchctl list` first and skip the unload when the query answers
  that nothing is loaded, so they do not fail on the unload step
  where load/unload cannot run — where the query itself fails, the
  unload is still attempted. The dry run lists the full plan either
  way. Pinned by
  `macos_plan_writes_the_plist_and_loads_it` (`src/periodic.rs:1569`).
- **Windows: a Task Scheduler task.** `geectl-prune`, created with
  `schtasks /create /xml`, running as the creating user
  (`InteractiveToken`, least privilege). Pinned by
  `windows_plan_writes_the_xml_to_temp_and_creates_the_task`
  (`src/periodic.rs:1602`).
- **Install, uninstall and status are leaves, not packaging.**
  `geectl local-build periodic-install --live` writes the entry and
  enables it; without `--live` it prints the plan only. `geectl
  local-build periodic-uninstall --live` disables the entry, then
  removes its files — a disable failure leaves the files in place,
  so a timer that may still fire never loses its unit file first,
  and uninstalling a schedule that is already gone succeeds as a
  no-op. `geectl local-build periodic-status` reports installed,
  enabled and active; each reads `unknown` where the scheduler
  cannot be asked or its answer names no known state — `installed`
  only on Windows, where the query is the answer; elsewhere the
  files answer it. The leaves parse as
  `periodic_leaves_parse` (`src/cli_local_build.rs:1273`).
- **The first live build installs the schedule.** A live
  `local-build build` installs the schedule when no decision is
  recorded yet, so nobody has to remember `periodic-install`, and
  prints a one-line notice when it does; a dry run installs
  nothing, and a missing scheduler never fails the build. The decision is recorded in
  `$XDG_CONFIG_HOME/geectl/periodic-auto-install` (else
  `~/.config/geectl/periodic-auto-install`): once written, the
  build never installs again — including after
  `periodic-uninstall`, which records the opt-out, so the next
  build does not put the schedule back. A manual disable before
  the decision is recorded is undone by the next build — the
  files and the disabled state read the same as a failed enable;
  `periodic-uninstall` is the opt-out. A manual
  `periodic-install` is the way back in. A dry run records the
  would-install through `DryRun::allow` instead of installing,
  and a failed install writes the failed-attempt marker
  (`periodic-auto-install-failed`, unix time), so later builds
  skip the probes until the day-long retry window passes. Pinned by
  `already_installed_needs_the_scheduler_not_to_report_disabled`
  (`src/periodic.rs:1855`),
  `maybe_auto_install_skips_when_marker_present`
  (`src/periodic.rs:1878`),
  `maybe_auto_install_does_not_decide_when_the_scheduler_cannot_be_asked`
  (`src/periodic.rs:1898`),
  `maybe_auto_install_skips_quietly_when_the_install_fails`
  (`src/periodic.rs:1928`),
  `failed_auto_install_is_negatively_cached`,
  `stale_failed_marker_does_not_suppress_the_retry`,
  `failed_auto_install_writes_the_failed_marker` (macOS-only) and
  `uninstall_records_the_opt_out`
  (`src/periodic.rs:2058`).
- **The timer's command is exactly `prune --apply`.** `prune`
  deletes only with its own `--apply` flag: the global `--live`
  does not gate it, because the scheduled task runs
  `geectl local-build prune --apply` and the leaf's confirmation is
  the flag itself. The deletion itself goes through
  `DryRun::allow`, so a plan run records the would-delete;
  `--live` without `--apply` still only notes that `--live` never
  deletes here. Pinned by `prune_parses_defaults_and_apply`
  (`src/cli_local_build.rs:1261`).
- **No schedule ships off the three platforms.** Install, uninstall
  and status fail with `UnsupportedOs` anywhere else.

## What "quick debug" covers, exactly

- **Single native arch only.** `arch` must equal the host arch
  (`uname -m`); anything else is `UnsupportedArch` with the OBS-ONLY
  route named. No qemu-user emulation: qemu-user is slow and flaky
  (hdrmd5 mismatch loops on i586); foreign arches route
  to the server. Native arch only means x86_64-only packages have no
  local build on arm64 hosts.
- **Repo is explicit.** Repo names differ per devel project; the caller
  resolves one first (the `repos` read) and
  passes it. Default recommendation in docs: `--alternative-project`
  Factory, `standard` (`Factory:ARM` on aarch64). The operation itself
  takes whatever repo it is given.
- **Multibuild: every flavor, explicitly.** If the source dir carries a
  `_multibuild` file and no `--flavor` is given, the op refuses with
  `MultibuildFlavorRequired` naming the flavors: a flavorless build dies
  with a fake "Architecture is not included" error. One invocation builds
  one flavor; the
  caller loops over flavors.
- **Debug flags.** `--alternative-project` fetches the buildinfo, the
  buildconfig, and the deps from another project; the root stays keyed
  by the package's own project. `--root` builds in a given directory
  instead of the derived cache-dir root; the sccache store lives inside
  the root, so two roots never share a tar. `--noinit` starts the build
  immediately in the existing root and refuses on a fresh one; it
  conflicts with `--clean`. `--keep-rpms <dir>` saves the built RPMs
  there on a green build. `--prefer-packages <dir>` uses a dep file from
  the dir instead of downloading it, hdrmd5-verified, failing closed on
  a mismatch; an explicit prefer file wins over the pkg cache too, so a
  rebuilt RPM is picked up on the next run. `--noinit` takes no
  `--prefer-packages` (nothing is installed); with
  `--alternative-project` it changes only the trust check, not the
  installed dep set or buildconfig. A `--root` build sits outside the
  cache-dir cap, eviction, and prune, and `build-log`/`build-wipe`/
  `build-shell` do not see it. `--clean` on a custom root refuses
  unless the root carries the geectl marker or is empty.
- **Verdict contract.** Success is exit code 0 **and** the log's
  `<host> finished "build <spec>"` line **and** the RPMs present — a
  build result is stated only after it has been read.
  The result is typed JSON: `verdict` (green/failed), `exit_code`,
  `log_path`, `rpms` (paths), `kept_rpms` (paths copied to the keep
  dir), `rpmlint` (errors/warnings/badness, read
  from the end of the log — badness over the project threshold is
  reported, not hidden, because the server build aborts on it), `checks`
  (%check/ctest pass counts when parseable), `image_digest`,
  `engine_version`, `root_path`, `root_reused` (bool), cache stats
  (pkg cache hits/misses/preferred, sccache hits). Stdout carries only this payload
  (R11); the full log stays on disk at `log_path` and is never dumped to
  stdout — `build-log` re-reads it.
- **Trust: no prompts, ever (R1).** The op enumerates the `<bdep
  project="…">` set from the buildinfo and
  refuses with `UntrustedProject`, naming the foreign `home:` projects,
  unless the set contains only non-`home:` projects and the caller's own
  `home:`. The build project itself is excluded from the set (osc parity,
  `check_trusted_projects(... if not i == prj)`, exact match only): a dep
  resolving from the project being built is the build's own output.
  `--trust-all-projects` is accepted only after that check
  passes — the flag cannot override a foreign-`home:` refusal; that
  decision stays with the user.
- **`--release N`** is supported (passes through to the engine; sets
  `BUILD_RELEASE` → `SOURCE_DATE_EPOCH`, which removes the local-only
  `python-bytecode-inconsistent-mtime` artifact for new packages).
- **`--clean` is explicit, default off.** The first build in a session
  may reuse the root; after a failure the caller passes `--clean`: a
  failed root is dirty and reusing it masks the fix. The result reports
  `root_reused` so the caller can enforce the
  rule. The op never auto-cleans: a silent rebuild from scratch would
  hide whether the fix or the clean fixed it.
- **Timeout and abort.** The library op is synchronous with a
  configurable timeout (default 3600 s), one budget for the whole
  op: the image pull and the engine share it — the engine gets what
  is left after the pull — so a `--timeout 3600` build runs at most
  ~3600 s for pull+engine. The capped probes sit outside it, at
  most a minute each — and there is no fixed count: a build runs
  the engine-version, image-digest, image-size and image-listing
  probes, plus up to two size probes per stored image digest (the
  listing is taken once for the evict pre-check and again for the
  apply re-read) and one delete per evicted image during eviction,
  so the time outside the budget grows with the stored images and
  is not bounded by the timeout. On timeout or abort it kills
  the engine's whole process group (the engine spawns in its own
  process group, `process_group(0)`, and the kill sends SIGTERM to
  the group, `kill -TERM -- -$pgid`, then SIGKILL to the group after
  a grace period — killing the wrapper alone leaves the engine
  running) and returns `BuildTimeout` with the log path; the partial
  log is preserved, never deleted. The group kill does NOT stop the
  container the engine started: it lives in its own pid namespace,
  and conmon keeps it running after podman dies — stopping it needs
  the container id, which the engine does not report.
  The drain of the engine's output is bounded too: a descendant
  that inherited the pipe and outlives the engine cannot hang the
  caller on the join — the captured output is best-effort past a
  short join timeout.
- **DryRun:** prints the plan (image digest or `latest`, root path,
  repo/arch/flavor, the engine invocation as data) and touches nothing —
  same convention as the rest of the library, guard before the
  credentials check.

## One API per issue #23 (git vs OBS)

The build takes a **directory**, never a backend. How the directory got
there — a #17 checkout of an OBS-native package, a git clone of a
git-backed package, an OBS `cat` materialization — is the caller's
business and invisible to the build. The engine runs the same command on
the same directory either way; the result shape is identical. No git-mode
refusals are copied: a git-backed package builds from its checkout
exactly like an OBS-native one, and the write-side refusal for scmsync
packages lives in the working-copy layer (#17), not here.

## What stays out

- Every other build backend OBS supports: KVM, qemu/cross-arch, lxc,
  nspawn, xen. Podman is the vertical; the rest is not a later phase of
  this design, it is a different decision.
- The full option matrix: `--icecream`/distcc, `--jobs`/`--threads`
  tuning, `--with`/`--without`, `--define`, `--linksources`,
  `--baselibs`, `--debuginfo`, `--oldpackages`, `--target`, `--preload`.
  If an agent need for one is proven, it is a new decision with its own
  justification, not a quiet addition.
- Build-root management beyond the container: no persistent root
  registry; root garbage collection is the Eviction policy (automatic
  at build start, plus `build-prune`) and nothing beyond that; no
  cross-machine roots.
- Server-side build control (`rebuild`/`restartbuild`/`abortbuild`,
  remote logs) — pure API, a different batch, not this vertical.
- No image building: geectl never builds container images, only pulls
  the one image.

## Errors (central store, `src/error.rs`)

`BuildFailed` (exit code + log path + tail), `BuildTimeout`,
`BuildInProgress`, `UntrustedProject` (names the foreign projects),
`MultibuildFlavorRequired` (names the flavors), `UnsupportedArch`
(non-native arch, OBS-ONLY named), `UnsupportedOs` (periodic
schedule off Linux, macOS and Windows), `NoBuildEngine` (engine missing,
install named), `ImageUnavailable` (pull failed), `CacheCorrupt`
(hdrmd5 mismatch after re-download), `BuildRootInUse` (destructive op
while locked), `NotAGeectlBuildRoot` (wipe without the marker). Each
with the remediation in the message; exit codes per kind, one exit path.

## Test story

- **PR-level (gating):** fixtures, never a real build. A stub engine
  (a script that records its argv, emits a canned log with a `finished`
  line and fake RPMs) proves the invocation shape, the verdict parsing,
  the trust-set computation, the flavor refusal, the timeout kill, and
  the cache layout (hdrmd5 verify, atomic rename, flock behaviour).
  The stub is what makes the vertical testable in CI at all.
- **Scheduled live (non-gating):** one trivial package, native arch,
  weekly, on the Mac — proves the
  real engine + image + cache end to end. It does not gate PRs: image
  pulls and dep downloads are the flaky part, and a red live run is a
  report, not a merge block. Same split as the live OBS tests.
- **No live build in PR CI:** a real build needs podman, network, and
  minutes — it does not belong in the per-PR loop. The stub proves the
  logic; the schedule proves the integration.

## Risks

- **Cache poisoning across projects:** a wrong-repo RPM in the cache
  would install a wrong dep into a root. Mitigated by per-project-repo-arch
  cache dirs, hdrmd5 verification on every use, and atomic rename
  (a half-written file is never visible). Residual: a buildinfo that
  itself is stale — mitigated by re-fetching buildinfo per build, never
  reusing a cached one across builds.
- **Image drift:** `latest` moves; a build that passed can
  fail on a newer toolchain with no local change. Mitigated by recording
  the resolved digest in every build record, so any build is
  reproducible by re-pinning that digest — and by the 7-day refresh,
  which bounds staleness without pulling on every build.
- **Podman on the Mac vs Linux:** the Mac runs podman rootless inside a
  VM (podman machine); device nodes, filesystem performance, and
  available memory differ from native Linux. The engine bind-mounts host
  devices once it detects the container via `/run/.containerenv`.
  Mitigated by testing
  the live build on the Mac itself (the scheduled run) and by never
  assuming Linux-only paths — the engine owns container mechanics.
- **Disk usage:** pkg cache + sccache stores + build roots + images grow
  without bound. Mitigated by the Eviction policy: a size cap enforced
  at build start, LRU within class, locked/recent/current-deps items
  never touched, and every deletion recorded in the build record.
- **Engine skew:** the `build` script on the host can be older/newer
  than the one the design was written against, and its podman mode can
  change flags. Mitigated by recording the engine version per build and
  by failing closed (not degrading) on an unrecognized engine error.
- **Stale-root masking:** reusing a root after a failure can mask the
  real fix. Mitigated by the explicit `--clean` rule and the
  `root_reused` flag in the result — the caller, not the op, enforces it.
- **Timeouts killing the wrong thing:** a build that legitimately takes
  40 minutes must not be killed by a 10-minute tool-call timeout.
  Mitigated by the op's own 3600 s default (the caller raises it for big
  packages) and by bounded waits, never a `sleep` loop.
