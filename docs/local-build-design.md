# geectl local-build vertical — design

Issue: https://github.com/scarabeusiv/geectl/issues/43
Depends on: #17 (working-copy layer — the build takes a checkout directory),
the `_buildinfo` read, #23 (git vs OBS abstraction).

## The decision, up front

Geectl has **exactly one building vertical: podman, for quick debug**.
The engine is the openSUSE `build` script. Geectl prepares the inputs and
reads the result; container mechanics, dependency installation, and RPM
creation stay with the engine.

One operation, `build`, taking a local source directory plus an explicit
repo and arch — plus three small root-management operations
(`build-shell`, `build-wipe`, `build-log`) on the same build root.
Caching is part of the vertical: a shared pkg cache (hdrmd5-verified)
and sccache (per-root storage), both under one geectl cache dir, both
locked.

## Local scope

The local vertical owns service runs, tarball fetching, and
spec-cleaner runs. It is the agent's quick-debug loop: fetch the tarballs
with the service binaries, run spec-cleaner, build — all inside the one
podman container, before anything goes to the server. The server stays the
source of truth for service results; local is for iterating fast.

Local-mode service execution (`manualrun`, `run NAME`, `precommit`-style
runs, #19) lives here, via `build-shell` + direct service binary. The
server has three service commands (trigger, wait, merge). An agent
debugging a service runs it in the build container with `build-shell` and
inspects the result before committing. Tarball fetching is the same shape:
the source-fetching services (`download_url`, `tar_scm`, `obs_scm`, …)
run locally so the agent can inspect the fetched sources. Spec-cleaner
runs in the container the same way.

Out of scope for the vertical:

Server-side service runs (trigger/wait/merge) — #19's server operations.
`buildtime` services — they run on the OBS worker during the build.
`runall` — mode-blind, it fires `buildtime` services locally.

The container needs the service binaries present (`obs-service-*`
packages); a plain Tumbleweed image lacks them. How they get
into the build container is out of scope here.

## The operation, mapped

| Operation | What it does | New server calls |
|---|---|---|
| `build` | Build the package in `<dir>` for `<repo>` / `<arch>` in a podman container, native arch only. Returns a typed result (verdict, exit code, log path, RPMs, rpmlint summary). | `buildinfo` (GET `/_buildinfo`, the resolved dep list — a prerequisite pure-API op, outside this vertical) |
| `build-shell` | Run a scripted command array as shell fragments inside the preserved build root (`chroot` equivalent). Headless, bounded timeout, stdout/stderr captured as data. The `local-build chroot` leaf is the CLI surface: it reads the fragments from stdin, one per line. | None |
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
v1 options: only the options listed above.

## Engine: the `build` script

The engine invocation:

`build --root=<root> --rpmlist=<file> --dist=<buildconfig> --arch=<buildarch> --vm-type=podman --vm-disk=<root>/img --vm-swap=<root>/swap --logfile=<root>/.build.log <spec>` plus buildargs (`--statistics`, `--norootforbuild`, `--clean`, `--ccache` / `--sccache-uri=...`).
Podman builds run as the user (rootless).
Preparation: fetch `_buildinfo` / `_buildconfig` from the OBS API and cache them in the store, download the resolved deps into a package cache, verify every cached package by hdrmd5, write the dep list to an `rpmlist` file, then run the engine. The library transport caps response bodies at 10 MiB; a dep RPM over the cap streams to disk.
Dep downloads are mirror-first: the buildinfo's `<path url>` values are the mirror base URLs for their (project, repository); a dep whose repo has one is fetched from `<url>/<arch>/<filename>`, credential-free, before the API is tried. Any mirror failure — network error, 404, hdrmd5 mismatch — falls back to the API download, which keeps its own retry and name-warming behaviour. A repo lacking a `<path url>` (unpublished, or a local instance) is API-only. The mirror rules:
**Scheme and host.** Only public `https` mirror URLs are used (the scheme is lowercased first, so `HTTPS://` counts). Plain `http`, userinfo, query, and fragment are excluded, as are loopback, link-local, and private hosts. The check runs twice — at buildinfo parse (a rejected URL is dropped and counted in `skipped_paths`, logged on the debug channel) and at use.
**Digest required.** The mirror is tried only when the buildinfo carries a usable hdrmd5 for the dep. A missing digest, or the download-on-demand placeholder, uses the API, which warms the binary and verifies against the server-computed digest.
**Redirects.** The mirror download follows up to 5 redirects, staying on the same scheme (`https` to `https` in production), with credentials only on the first hop. `download.opensuse.org` answers 302 to its backing store.
**Arch segment.** The URL uses the dep's own package arch, falling back to the repo arch when the dep leaves the arch open: a noarch dep reads from `noarch/`. Arch and file name are percent-encoded like the API path.
**Single-shot, dead-mirror skip.** A mirror attempt is one request, bounded by the transport's timeouts. A health failure (transport, 5xx, timeout, redirect, over-cap) marks the (project, repo) mirror dead for the rest of the build; a per-file failure (404, hdrmd5 mismatch) leaves it live. The attempt runs inside the per-package cache lock, so a dead mirror costs at most one attempt per build.
**Streamed, capped, verified once.** The mirror download streams straight to a temp file, capped at 1 GiB (over the cap falls back to the API, which streams uncapped). The temp file is hdrmd5-verified once, in the mirror download, and lands in the cache after a single hash.
**Integrity limit.** hdrmd5 (MD5 over the header and payload) is the only integrity check on a mirror download: repomd and package-signature checks are absent, and MD5 itself is collision-feasible. The digest comes from the buildinfo the trusted OBS server sent, the transport is `https` to a public host, and a body that fails the digest stays out of the cache. A mirror serving a different file with a colliding digest goes undetected.
**Malformed `<path>`.** A `<path>` lacking a project or repository is skipped and counted: mirror data is only an optimisation, and the dep fetch falls back to the API for that repo.
Dep file names resolve from the buildinfo's `<bdep>` attributes: the scheduler sends no `binary` attribute, so the file name is computed locally as `name-version-release.arch.rpm` from `version`/`release`/`arch` (the epoch is outside file names); when any of the three is missing, the binary list of the dep's source package (`package` attribute) resolves it. List and download URLs key on the source package: a subpackage binary (`libz1`) lives under its source package (`zlib`), and the binary name as the package segment 404s both the list and the download. With the source package omitted, the whole repository is listed (`_repository`), which the server serves for both list and download.
Built files are collected from `<root>/.build.packages` (RPMS/SRPMS). On failure geectl returns the exit code, the root path, and the `--clean` / wipe suggestion as data.
The preinstall image, when the buildinfo carries one and `--preinstall-image` is set, seeds a fresh root: the image downloads into the pkg cache (streamed, magic-checked — the buildinfo digest is a synthetic image id, outside content digests), and its `.info` file downloads alongside it. The covered dependencies stay out of downloads and out of `<name> <file>` lines, but keep their place in the install lists, and the rpmlist carries the image lines for the engine. Any image failure drops the image and the build proceeds from plain deps. The image is fetched with a single GET, so a download-on-demand repository serves no image. A root whose init already ran skips image selection. With the flag unset, the offer is parsed and unused: the engine extracts it with bsdtar `--chroot` plus device nodes, which fail as a non-root user. Image building is out of scope for geectl.
The engine is an external dependency: geectl checks for the `build` binary at startup of the operation and reports `NoBuildEngine` (naming how to install it) when the binary is missing. The engine version is recorded in every build record, so a skew is diagnosable.

## Image

**Default: `registry.opensuse.org/opensuse/tumbleweed:latest`.** The default target is Factory/Tumbleweed; the image is the Tumbleweed base plus the `build` package and its helpers, as the engine requires.
The image digest actually used is resolved at the start of every build and recorded in the build record, so a build is reproducible from the record. The image is pulled when missing from the local store; `--refresh-image` forces a pull; otherwise a local image older than 7 days is refreshed.
One image, one target. A Leap pool build stays OBS-ONLY.

## Cache layout

One geectl directory, default `$XDG_CACHE_HOME/geectl/` (`~/.cache/geectl/`,
overridable by config `build.cache_dir`):

`pkg/<project>/<repo>/<arch>/` — the shared package cache. One dir per project+repo+arch, so a Tumbleweed dep stays out of a Leap root (and vice versa), and the same repo name plus the same file name from two projects stays in separate dirs. The layout matches `cachedir/project/repository/repoarch`. Files are named by RPM filename, written to a temp file and atomically renamed (R16), and verified against the hdrmd5 in the buildinfo on every use: a mismatch is re-downloaded once, then a structured `CacheCorrupt` error. On every use geectl touches the file (mtime = last use); verification is by content (hdrmd5), so touching is safe — the eviction policy orders pkg files by mtime.
`sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the compiler cache, one store per build root. Per-package storage prevents cross-cache pollution; geectl tightens it to per-root: two concurrent builds of the same package in different roots hold different root locks, so a shared per-package tar would be written by both at once and corrupt; a per-root tar is guarded by its own root's flock. Repeat builds of the *same* root share compiled objects; different roots hold separate stores. The store directory's mtime is updated on every build that uses it (eviction orders stores by last use). One vertical, one compiler cache: sccache. The `--compiler-cache` argument is out of scope.
`build-roots/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the build root, named project/package/repo/arch as in the `build-root` template, so `build-summary`-style tooling and humans can find it. A `:` in a project, package, repository, arch, or flavor name is percent-encoded in the directory (`:` becomes `%3A`, `%` becomes `%25`); the encoding is injective, so `home:foo` and `home_foo` are different projects with different roots, and the marker keeps the real name. The same encoded form is used for the `sccache/` and `pkg/` layouts above. Directories written before the encoding keep raw `:` in their names; new builds use the encoded form, but eviction still walks and can delete the legacy dirs. A pkg lock held by a concurrently running pre-upgrade binary is unhonored for those legacy paths, because the lock lookup uses the encoded form. A legacy sccache store is still guarded at delete time — its root lock is held via the verbatim store-to-root path, so a locked root stays undeleted under a running build (only the plan-time protection misses it). The root carries a `.geectl-build-root` marker file (image digest, engine version, repo/arch/flavor, created time, last build time — the eviction policy orders roots by it); `build-wipe` refuses a directory lacking the marker.

Locking (R15 — the failure mode is two builds sharing one root):

The pkg cache is shared across builds and serializes only the per-file download/write path, briefly, on `<project>/<repo>/<arch>.lock` (`flock`); readers use the atomically renamed files, lock-free.
The build root and the root's sccache store are exclusive: one build per (package, repo, arch, flavor) at a time, guarded by `flock` on the root's lock file — and the store is per-root, so the same lock guards the tar too. A second invocation refuses with `BuildInProgress`, naming the holder — fail fast, the agent retries. A lock that frees within ~100 ms is retried: a `Command::spawn` in another thread forks the process, and the forked child holds the lock file's open file description until exec closes it, so a just-released lock can read as held for a few milliseconds — a real build holds it far longer, so it still refuses promptly. A destructive op (`--clean`, `build-wipe`) refuses while the lock is held.
Missing cache is a normal state: an absent pkg file is downloaded, an absent sccache store starts empty (cold first build, warm after). A stale cache is detected by hdrmd5 (pkg) or by the sccache store's own versioning; eviction follows the Eviction policy below and every deletion is recorded in the build record.

## Eviction

Build roots, the pkg cache, sccache stores and images grow
unbounded. The vertical evicts automatically — only regenerable items,
only under the safeguards below, and every deletion is recorded in
the build record. Everything in the cache dir is regenerable: the
source checkout lives outside it and stays untouched; a pkg file is
a re-download, a sccache store is a recompile, a root is a dep
reinstall, an image is a re-pull. Eviction costs time; evicted roots rebuild.

### Trigger: a size cap, enforced at build start

One cap over the whole cache dir (`pkg` + `sccache` + `build-roots` + images pulled for the vertical), default **20 GiB**, tunable by config `build.cache_cap_gib`. Low-water is 80% of the cap (16 GiB at the default), sharing the one knob.
Enforcement point: at the start of `build`, **after** the buildinfo fetch (so the current build's resolved dep set is known) and **before** any download. If usage is above the cap, evict until usage is at or below the low-water mark.
A size cap is the trigger that bounds usage: age alone leaves disk usage unbounded, and an on-demand prune op depends on someone running it. Enforcing it at build start means the build evicts only before the build, from a full inventory.
A build runs regardless of size. If everything above the low-water mark is protected (locked, recent, or the current build's deps), eviction frees what it can and reports the shortfall (`shortfall_bytes` in the build record); the build still runs. The cap bounds what geectl retains; a disk full for other reasons is outside this vertical.

### What gets evicted, in what order

LRU within each class, cheapest to regenerate first. Last use is the
file's mtime: geectl touches a pkg file on every use, a sccache store on
every build that uses it, and the root's marker file records the last
build time (see Cache layout).

1. **Pkg cache files** (`pkg/<project>/<repo>/<arch>/`). Cheapest: re-download
   from the repos, hdrmd5-verified on use, and a missing file is
   downloaded anyway — evicting one costs at most a re-download.
   Oldest mtime first.
2. **sccache stores** (`sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/`). Whole stores,
   oldest last-use first. A missing store starts empty (cold build).
   A store whose package has a locked build root stays.
3. **Build roots** (`build-roots/.../`). Only roots that are unlocked,
   other than the most recent for their (project, package),
   and older than the grace period. Oldest last-build first. Evicting
   a root deletes the built RPMs inside it — the most-recent-root rule
   protects the newest root of the package; an older root's outputs
   are rebuilt by reinstalling its deps (from the pkg cache when
   still cached, else re-downloaded from OBS). Grouping is by
   project+package: an older root of the same package in another repo,
   arch or flavor can evict once a newer root of that package exists;
   grouping by the full 5-tuple protects every root, so roots
   effectively stay unevicted.
4. **Images.** Last, and only superseded digests: an older digest of
   the image may be removed once a newer digest has been pulled **and**
   at least one build has succeeded with it. The digest the most
   recent build recorded stays, as does the digest `latest`
   currently resolves to.

### What stays

A build root whose lock is held (a build in progress). Eviction attempts a non-blocking `flock` on each root's lock file; held means skip. Deleting a root a running build is inside corrupts that build.
The most recent root per (project, package) — older roots of the same package in other repos, arches or flavors are regenerable (their deps reinstall — from the pkg cache when still cached, else re-downloaded from OBS), so only the newest is protected. Root reuse itself stays per 5-tuple: that repo's next build recreates its root rather than reusing the newest one.
The current build's resolved dep set (from the buildinfo fetched before eviction). A build's own eviction run leaves untouched every file that same build is about to install.
Anything used within the grace period: **24 hours** since last use. A build about to use a file has used it recently, or it is the current build (listed above).
The current image digest (see above).
Anything outside the cache dir, and any directory lacking the `.geectl-build-root` marker (for roots). Eviction touches only what geectl created.

### Safeguards

Eviction runs at build start and in `build-prune` only. One eviction at a time: a global `eviction.lock` in the cache dir, taken non-blocking — if another eviction holds it, this run skips eviction and the build proceeds (the other run covers it).
Eviction takes each project-repo-arch lock while deleting that dir's pkg files, so a delete stays clear of a download's atomic rename.
At apply time a root's lock is held across its delete, and a sccache store's root lock across the store's delete: the plan-time probe is stale by then, and a build starting in between keeps its root (or its store). The root's lock file itself goes last in the delete, so a build attempting entry still finds the lock file in place.
Every eviction is recorded in the build record (see below). The manual path is inventory-first: `build-prune` prints the eviction plan by default and deletes only with `--apply`.
Everything evicted is regenerable, so a recovery window buys nothing, and a trash dir would make the cap dishonest (trashed bytes still occupy the disk while the cap counts them as freed). The build record is the audit trail.

### What the build record reports

Usage is reported by `cache_usage` (`total_bytes` with per-area
`pkg_bytes`, `sccache_bytes`, `roots_bytes`, `image_bytes`), and the
build record carries `cache` (pkg hits/misses/preferred, sccache hits); eviction
adds an `eviction` object to every build record:

`ran` (bool), `cap_bytes`, `low_water_bytes`, `bytes_before`, `bytes_after`, `freed_bytes`
`evicted`: per item, `kind` (pkg / sccache / root / image), `path`, `bytes`, `reason` (`lru`, `superseded-image`)
`protected_bytes` (bytes above low-water that stayed), `shortfall_bytes` (cap unmet)
`build-prune` reports the same object for a manual run (plan only, in plan mode (no `--apply`).

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
   build-start path deletes on its own, only under
   the safeguards above.

## Periodic cleanup

Eviction runs at build start: a cache that stops being built
against keeps its stale roots until the next build. The periodic
cleanup runs the eviction pass on a daily schedule, under the same
policy and safeguards as automatic eviction.

**The schedule runs `prune --apply` daily at 03:00 local time.** The time is the same on every platform: the systemd timer carries `OnCalendar=*-*-* 03:00:00`, the launchd agent a `StartCalendarInterval` of hour 3, minute 0, and the scheduled task a daily trigger at 03:00. `systemd_units_carry_the_binary_and_0300`, `launchd_plist_carries_the_binary_log_and_0300` and `scheduled_task_xml_carries_the_binary_and_daily_0300` pin this.
**Linux: a systemd user timer.** `geectl-prune.service` + `geectl-prune.timer` in `~/.config/systemd/user/` (or `$XDG_CONFIG_HOME/systemd/user/`), enabled with `systemctl --user enable --now`. The unit files live under `packaging/systemd/` and are the single source of truth: the install command renders them with the running binary's path, and the RPM package ships the same files (separate packaging track; the spec substitutes `@GEECTL_BIN@` with the packaged binary path). `linux_plan_writes_both_units_and_enables_the_timer` pins this.
**macOS: a launchd agent.** `io.github.scarabeusiv.geectl.prune.plist` in `~/Library/LaunchAgents/`, loaded with `launchctl load`, logging to `~/Library/Logs/geectl-prune.log`. A re-install unloads the prior load first, so it picks up the new plist. Install and uninstall ask `launchctl list` first and skip the unload when the query answers that nothing is loaded; where the query itself fails, the unload is still attempted. The dry run lists the full plan either way. `macos_plan_writes_the_plist_and_loads_it` pins this.
**Windows: a Task Scheduler task.** `geectl-prune`, created with `schtasks /create /xml`, running as the creating user (`InteractiveToken`, least privilege). `windows_plan_writes_the_xml_to_temp_and_creates_the_task` pins this.
**Install, uninstall and status are leaves.** `geectl local-build periodic-install --live` writes the entry and enables it; in plan mode it prints the plan only. `geectl local-build periodic-uninstall --live` disables the entry, then removes its files — a disable failure leaves the files in place, so a timer that may still fire keeps its unit file, and uninstalling a schedule that is already gone succeeds as a no-op. `geectl local-build periodic-status` reports installed, enabled and active; each reads `unknown` where the scheduler is unreachable or its answer names no known state — `installed` only on Windows, where the query is the answer; elsewhere the files answer it. `periodic_leaves_parse` pins the leaves.
**The first live build installs the schedule.** A live `local-build build` installs the schedule before any decision is recorded, and prints a one-line notice when it does; a dry run installs nothing, and a missing scheduler leaves the build running. The decision is recorded in `$XDG_CONFIG_HOME/geectl/periodic-auto-install` (else `~/.config/geectl/periodic-auto-install`): once written, the build installs once only — including after `periodic-uninstall`, which records the opt-out, so the next build leaves the schedule out. A manual disable before the decision is recorded is undone by the next build — the files and the disabled state read the same as a failed enable; `periodic-uninstall` is the opt-out. A manual `periodic-install` is the way back in. A dry run records the would-install through `DryRun::allow` instead of installing, and a failed install writes the failed-attempt marker (`periodic-auto-install-failed`, unix time), so later builds skip the probes until the day-long retry window passes. `already_installed_needs_the_scheduler_not_to_report_disabled`, `maybe_auto_install_skips_when_marker_present`, `maybe_auto_install_does_not_decide_when_the_scheduler_cannot_be_asked`, `maybe_auto_install_skips_quietly_when_the_install_fails`, `failed_auto_install_is_negatively_cached`, `stale_failed_marker_does_not_suppress_the_retry`, `failed_auto_install_writes_the_failed_marker` (macOS-only) and `uninstall_records_the_opt_out`.
**The timer's command is exactly `prune --apply`.** `prune` deletes only with its own `--apply` flag: the global `--live` leaves it ungated, because the scheduled task runs `geectl local-build prune --apply` and the leaf's confirmation is the flag itself. The deletion itself goes through `DryRun::allow`, so a plan run records the would-delete; `--live` with `--apply` unset still only records the plan. `prune_parses_defaults_and_apply` pins this.
**The schedule ships on the three platforms only.** Install, uninstall and status report `UnsupportedOs` anywhere else.

## What "quick debug" covers, exactly

**Single native arch only.** `arch` equals the host arch (`uname -m`); anything else is `UnsupportedArch` with the OBS-ONLY route named. Qemu-user emulation is out of scope (slow and flaky: hdrmd5 mismatch loops on i586); foreign arches route to the server. Native arch only means x86_64-only packages build on the server only from arm64 hosts.
**Repo is explicit.** Repo names differ per devel project; the caller resolves one first (the `repos` read) and passes it. Default recommendation in docs: `--alternative-project` Factory, `standard` (`Factory:ARM` on aarch64). The operation itself takes whatever repo it is given.
**Multibuild: every flavor, explicitly.** If the source dir carries a `_multibuild` file and `--flavor` is absent, the op refuses with `MultibuildFlavorRequired` naming the flavors: a flavorless build reports a fake "Architecture is not included" error. One invocation builds one flavor; the caller loops over flavors.
**Debug flags.** `--alternative-project` fetches the buildinfo, the buildconfig, and the deps from another project; the root stays keyed by the package's own project. `--root` builds in a given directory instead of the derived cache-dir root; the sccache store lives inside the root, so two roots hold separate tars. `--noinit` starts the build immediately in the existing root and refuses on a fresh one; it conflicts with `--clean`. `--keep-rpms <dir>` saves the built RPMs there on a green build. `--prefer-packages <dir>` uses a dep file from the dir instead of downloading it, hdrmd5-verified, failing closed on a mismatch; an explicit prefer file wins over the pkg cache too, so a rebuilt RPM is picked up on the next run. `--noinit` is incompatible with `--prefer-packages` (nothing is installed); with `--alternative-project` it changes the trust check only, leaving the installed dep set and buildconfig untouched. A `--root` build sits outside the cache-dir cap, eviction, and prune, and `build-log`/`build-wipe`/`build-shell` leave it unseen. `--clean` on a custom root refuses unless the root carries the geectl marker or is empty.
**Verdict contract.** Success is exit code 0 **and** the log's `<host> finished "build <spec>"` line **and** the RPMs present — a build result is stated only after it has been read. The result is typed JSON: `verdict` (green/failed), `exit_code`, `log_path`, `rpms` (paths), `kept_rpms` (paths copied to the keep dir), `rpmlint` (errors/warnings/badness, read from the end of the log — badness over the project threshold is reported, because the server build aborts on it), `checks` (%check/ctest pass counts when parseable), `image_digest`, `engine_version`, `root_path`, `root_reused` (bool), cache stats (pkg cache hits/misses/preferred, sccache hits). Stdout carries only this payload (R11); the full log stays on disk at `log_path` — `build-log` re-reads it.
**Trust: prompt-free (R1).** The op enumerates the `<bdep project="…">` set from the buildinfo and refuses with `UntrustedProject`, naming the foreign `home:` projects, unless the set contains only non-`home:` projects and the caller's own `home:`. The build project itself is excluded from the set (`check_trusted_projects(... if not i == prj)`, exact match only): a dep resolving from the project being built is the build's own output. `--trust-all-projects` is accepted only after that check passes — the flag leaves a foreign-`home:` refusal in place; that decision stays with the user.
**`--release N`** is supported (passes through to the engine; sets `BUILD_RELEASE` → `SOURCE_DATE_EPOCH`, which removes the local-only `python-bytecode-inconsistent-mtime` artifact for new packages).
**`--clean` is explicit, default off.** The first build in a session may reuse the root; after a failure the caller passes `--clean`: a failed root is dirty and reusing it masks the fix. The result reports `root_reused` so the caller can enforce the rule. Auto-clean is out of scope for the op: a silent rebuild from scratch would hide whether the fix or the clean fixed it.
**Timeout and abort.** The library op is synchronous with a configurable timeout (default 3600 s), one budget for the whole op: the image pull and the engine share it — the engine gets what is left after the pull — so a `--timeout 3600` build runs at most ~3600 s for pull+engine. The capped probes sit outside it, at most a minute each — and the probe count varies: a build runs the engine-version, image-digest, image-size and image-listing probes, plus up to two size probes per stored image digest (the listing is taken once for the evict pre-check and again for the apply re-read) and one delete per evicted image during eviction, so the time outside the budget grows with the stored images, outside the timeout's bound. On timeout or abort it kills the engine's whole process group (the engine spawns in its own process group, `process_group(0)`, and the kill sends SIGTERM to the group, `kill -TERM -- -$pgid`, then SIGKILL to the group after a grace period — killing the wrapper alone leaves the engine running) and returns `BuildTimeout` with the log path; the partial log is preserved. The group kill leaves the container the engine started running: it lives in its own pid namespace, and conmon keeps it running after podman dies — stopping it needs the container id, which the engine leaves unreported. The drain of the engine's output is bounded too: whatever descendants do, the captured output is best-effort past a short join timeout.
**DryRun:** prints the plan (image digest or `latest`, root path, repo/arch/flavor, the engine invocation as data) and touches nothing — same convention as the rest of the library, guard before the credentials check.

## One API per issue #23 (git vs OBS)

The build takes a **directory**. How the directory got
there — a #17 checkout of an OBS-native package, a git clone of a
git-backed package, an OBS `cat` materialization — is the caller's
business and invisible to the build. The engine runs the same command
on the same directory either way; the result shape is identical. A
git-backed package builds from its checkout exactly like an
OBS-native one, and the write-side refusal for scmsync packages lives
in the working-copy layer (#17).

## What stays out

Every other build backend OBS supports: KVM, qemu/cross-arch, lxc, nspawn, xen. Podman is the vertical; the rest is a different decision.
The full option matrix: `--icecream`/distcc, `--jobs`/`--threads` tuning, `--with`/`--without`, `--define`, `--linksources`, `--baselibs`, `--debuginfo`, `--oldpackages`, `--target`, `--preload`. If an agent need for one is proven, it is a new decision with its own justification.
Build-root management beyond the container: root garbage collection is the Eviction policy (automatic at build start, plus `build-prune`); the registry and cross-machine roots stay out of scope.
Server-side build control (`rebuild`/`restartbuild`/`abortbuild`, remote logs) — pure API, outside this vertical.
Image building: geectl pulls the one image only.

## Errors (central store, `src/error.rs`)

`BuildFailed` (exit code + log path + tail), `BuildTimeout`,
`BuildInProgress`, `UntrustedProject` (names the foreign projects),
`MultibuildFlavorRequired` (names the flavors), `UnsupportedArch`
(non-native arch, OBS-ONLY named), `UnsupportedOs` (periodic
schedule off Linux, macOS and Windows), `NoBuildEngine` (engine missing,
install named), `ImageUnavailable` (pull failed), `CacheCorrupt`
(hdrmd5 mismatch after re-download), `BuildRootInUse` (destructive op
while locked), `NotAGeectlBuildRoot` (wipe of a directory lacking
  the marker). Each
with the remediation in the message; exit codes per kind, one exit path.

## Test story

**PR-level (gating):** fixtures, with a stub engine instead of a real build (a script that records its argv, emits a canned log with a `finished` line and fake RPMs). It proves the invocation shape, the verdict parsing, the trust-set computation, the flavor refusal, the timeout kill, and the cache layout (hdrmd5 verify, atomic rename, flock behaviour). The stub is what makes the vertical testable in CI.
**Scheduled live (non-gating):** one trivial package, native arch, weekly, on the Mac — proves the real engine + image + cache end to end. It is non-gating for PRs: image pulls and dep downloads are the flaky part, and a red live run is a report only.
**Live builds stay out of PR CI:** a real build needs podman, network, and minutes. The stub proves the logic; the schedule proves the integration.

## Risks

**Cache poisoning across projects:** a wrong-repo RPM in the cache would install a wrong dep into a root. Mitigated by per-project-repo-arch cache dirs, hdrmd5 verification on every use, and atomic rename (a half-written file stays invisible). Residual: a buildinfo that itself is stale — mitigated by re-fetching buildinfo per build.
**Image drift:** `latest` moves; a build that passed can fail on a newer toolchain with the local tree unchanged. Mitigated by recording the resolved digest in every build record, so any build is reproducible by re-pinning that digest — and by the 7-day refresh, which bounds staleness while pulling only weekly.
**Podman on the Mac vs Linux:** the Mac runs podman rootless inside a VM (podman machine); device nodes, filesystem performance, and available memory differ from native Linux. The engine bind-mounts host devices once it detects the container via `/run/.containerenv`. Mitigated by testing the live build on the Mac itself (the scheduled run); the engine owns container mechanics, so Linux-only paths stay unassumed.
**Disk usage:** pkg cache + sccache stores + build roots + images grow unbounded. Mitigated by the Eviction policy: a size cap enforced at build start, LRU within class, locked/recent/current-deps items untouched, and every deletion recorded in the build record.
**Engine skew:** the `build` script on the host can differ in version from the one this design targets, and its podman mode can change flags. Mitigated by recording the engine version per build and by failing closed on an unrecognized engine error.
**Stale-root masking:** reusing a root after a failure can mask the real fix. Mitigated by the explicit `--clean` rule and the `root_reused` flag in the result — enforcement sits with the caller.
**Timeouts killing the wrong thing:** a build that legitimately takes 40 minutes stays unkilled by a 10-minute tool-call timeout. Mitigated by the op's own 3600 s default (the caller raises it for big packages) and by bounded waits only.
