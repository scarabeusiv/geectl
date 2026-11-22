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
| `build-shell` | Run a scripted command array inside the preserved build root (`chroot` equivalent). No TTY, bounded timeout, stdout/stderr captured as data. | None |
| `build-wipe` | Delete a build root. Refuses when a build holds the root's lock, and refuses when the directory is not a geectl root (marker file). | None |
| `build-log` | Read the last build's log (`<root>/.build.log`) — tail, or full, plus the recorded verdict. | None |
| `build-prune` | Print the eviction plan (what would be evicted, in order, with bytes); delete only with `--apply`. Same policy and safeguards as automatic eviction. | None |

`build` inputs: source directory (spec + sources — a checkout from #17,
a git clone, or any dir with a spec), `repo` (explicit name, e.g.
`standard`), `arch` (explicit; must equal the host arch — see scope).
Options: `--clean`, `--flavor` (multibuild), `--release N`,
`--trust-all-projects` (see trust), `--refresh-image`, `--timeout SECS`.
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
  library transport caps response bodies at 10 MiB: a dep RPM over the cap
  fails closed with a structured error; streaming downloads are not
  supported yet.
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
- The preinstall image, when the buildinfo carries one, seeds the root;
  an unprivileged user cannot extract it (chroot/device nodes), so the
  wrapper drops it when building as a user unless the container
  preinstallimage flag is given. Geectl records the buildinfo's
  preinstall image filename in the build record but does not pass it
  to the engine: a rootless podman build drops the image, exactly as
  the existing tool does. Geectl does not select or build images
  itself.
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
  and humans can find it. The root carries a `.geectl-build-root` marker
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
build record carries `cache` (pkg hits/misses, sccache hits); eviction
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
- **Verdict contract.** Success is exit code 0 **and** the log's
  `<host> finished "build <spec>"` line **and** the RPMs present — a
  build result is stated only after it has been read.
  The result is typed JSON: `verdict` (green/failed), `exit_code`,
  `log_path`, `rpms` (paths), `rpmlint` (errors/warnings/badness, read
  from the end of the log — badness over the project threshold is
  reported, not hidden, because the server build aborts on it), `checks`
  (%check/ctest pass counts when parseable), `image_digest`,
  `engine_version`, `root_path`, `root_reused` (bool), cache stats
  (pkg cache hits/misses, sccache hits). Stdout carries only this payload
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
  `--baselibs`, `--debuginfo`, `--oldpackages`, `--prefer` (beyond the
  pkg cache), `--target`, `--preload`, `--noinit` (except as the engine
  needs it internally). If an agent need for one is proven, it is a new
  decision with its own justification, not a quiet addition.
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
(non-native arch, OBS-ONLY named), `NoBuildEngine` (engine missing,
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
