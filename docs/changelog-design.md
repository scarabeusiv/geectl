# `changelog` — `.changes` validation and generation

`changelog validate` and `changelog generate` work on a package directory on disk,
before filing. They never file anything, and nothing here wires into
`commit` — pre-commit placement waits on the working-copy layer.

## Commands

```sh
geectl changelog validate <dir> [--target-project P --target-package PKG] [--entries N]
geectl changelog generate <dir> [--target-project P --target-package PKG] [--author 'Name <email>'] [--write]
```

`dir` defaults to `.`. The target flags go together or not at all.
`--author` is required for `generate` (no git identity is read, and
geectl config carries none). `--live` is ignored: `changelog` performs no
OBS mutation; `--write` is the explicit local-write flag.

## Rules (V1–V12)

Validation runs over a package snapshot (file name → text) plus an
optional target snapshot. A `.changes` snapshot is the normalized
form of the file (see below); target reads are public
(`list_files`, plus `cat` for `.spec` and a byte fetch for
`.changes`, normalized at load like the local files). A target that
does not exist is a new package: the target-relative rules are
skipped. Any other fetch failure is a hard error — a failed lookup
never reads as good news.

- V1 — every spec has a matching `<stem>.changes` file. (The
  reverse — every `.changes` file matching a subpackage or a
  `Source:` — is V6.)
- V2 — at least one `.changes` differs from the target's (target
  only).
- V3 — every spec carries a `%changelog` section.
- V4 — a patch added or removed vs the target is named literally on
  a `+`/`-` line of the `.changes` diff (LCS diff). Exempt when the
  filename matches a `Source:` value in either spec (glob;
  `%{...}` is a wildcard, over the value's basename).
- V5 — `.changes` headers parse and run strictly descending; a
  duplicated header (a short `* Day Mon DD YYYY Name` line matching
  the entry's own header, same date and author) is a finding; the
  newest local entry is newer than the target's newest (target
  only).
- V6 — every `.changes` file matches a subpackage (`%package` in
  the spec) or a `Source:` (glob; `%{...}` is a wildcard, over the
  value's basename).
- V7 — every file in the directory is referenced in the spec as a
  `Source:` or `Patch:` (glob; `%{...}` is a wildcard);
  `.spec`/`.changes` files and `_`-prefixed files are excluded.
- V8 — `Version:` is present and a single token.
- V9 — a bare `Update to <version>` entry with no other substantive
  line is a finding, but only when the upstream NEWS section for the
  new version exists. A line that only names a file, dependency, or
  license operation is not substantive. No news found → the rule is
  skipped, never flagged; when no source tarball was on disk at all,
  the skip says so — the line was never checked. A leftover `TODO`
  slot is also a V9 finding.
- V10 — the license tokens in the newest entry match the spec
  `License:` (normalised: `GPLv3` → `GPL-3.0`, `-only`/`-or-later`
  stripped; a token must carry a letter and a digit, or be a known
  short name). A license change claimed in the entry while the spec
  `License:` is unchanged vs the target is also a finding (a line
  stating there is no change is not a claim).
- V11 — the newest N entries (default 1) pass the format lint
  (bullets `- ` at indent 0, `* ` nested at indent 2), and the
  submission carries at most N new entries vs the target; older
  entries match the target's up to the whole-file normalization
  `generate --write` applies (sanctioned exceptions are named in
  the message for the human to judge).
- V12 — bug references: a `bnc#` reference (the old abbreviation —
  use `boo#`/`bsc#`), incomplete CVE ids, a bare `CVE(s)` or
  security claim with no full id, and a CVE named as not fixed /
  not affected with no fix statement elsewhere in the entry.

Rules skipped for lack of a target (or of a NEWS section) are listed
in the report with the reason. `validate` exits 0 with verdict
`pass`, or 24 (`validation_failed`) with one
`file:line: V#: message` line per finding in the error context.

## Generation

`generate` emits the diff-derived skeleton entry: the version line,
patch add/drop lines, source add/remove lines, `Requires:` /
`BuildRequires:` add/drop lines, and the license line, each with an
explicit `TODO` reason slot where the diff cannot supply one. The
upstream NEWS section for the new version, extracted from the source
tarball on disk, arrives as a `TODO` highlights slot (up to 15 lines,
wrapped at 67 columns) — the human distills it; the slot never
counts as substantive.

`needs_detail` is set when news exists, the version changed, and the
draft has no substantive line beyond the version line (mechanical
file/dependency/license lines never count). `--write`
prepends the draft into the package's `.changes` file (verified:
separator count +1, old content preserved as its normalized form)
and refuses a draft with unfilled `TODO` slots or a needs-detail
flag, exit 24. The write normalizes the whole file — trailing
whitespace stripped, CRLF→LF, legacy encodings transcoded to
UTF-8, a BOM stripped — atomically (temp file in the same directory,
then rename, keeping the destination's permissions and syncing the
directory after the rename). `--write` also refuses when the
destination `.changes` file is a symlink, when the file cannot be
read (only a missing file reads as empty), or when the file has
bytes no known encoding decodes; the guessed encoding is named on
the debug channel. A file decoded from a guessed single-byte
encoding is transcoded on the guess and the write warns about it,
naming the encoding, instead of refusing.

## NEWS extraction

`find_tarball` prefers `Source0`, then a tarball carrying the
version, then a sole tarball. Extraction uses the `tar` CLI
(`tar -tf` / `tar -xOf`, 30 s timeout, 1 MiB output cap per stream). Archive
paths and member names starting with `-` are never passed as
options (`--` before the member; `./`-prefix on the archive path).
The section is the text under
the heading naming the version, up to the next version heading or a
heading of equal or higher level; deeper subheadings (`### Added`
under `## 2.0`) stay inside the section. A file with no version
headings yields no section. A lookup failure is
an error value, never an empty section.

## What it does not do

- No filing, no OBS mutation, no commit wiring.
- No LLM: V1–V8 and V10–V12 are exact; V9 is a deterministic
  heuristic gated on the NEWS section existing.
- No changelog rewrite of old entries beyond the whole-file
  normalization: `--write` normalizes the whole file (trailing
  whitespace, CRLF→LF, legacy encodings to UTF-8, BOM stripped)
  and V11 compares older entries after that normalization, so a
  difference normalization alone explains is not a finding.

## Rule provenance

V11's format lint and V12's bug-reference checks are ports of the
packaging skill's `changes-lint.sh`; V4's patch-naming rule is a port
of its `changes-patches.sh`; V9's needs-detail condition and the
entry-count rule follow its `changelog-rules.md`. V12 is a
simplification of the skill's full attribution logic, which stays in
the skill.
