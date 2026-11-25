# `changelog` — `.changes` validation and generation

`changelog validate` and `changelog generate` work on a package directory on disk,
before filing, and file nothing. `commit` runs the changelog gate on the
staged `.changes` entries, before anything is filed.

## Commands

```sh
geectl changelog validate <dir> [--target-project P --target-package PKG] [--entries N]
geectl changelog generate <dir> [--target-project P --target-package PKG] [--author 'Name <email>'] [--write]
```

`dir` defaults to `.`. The target flags pass as a pair.
`--author` is required for `generate` (geectl reads the identity from the flag only). `changelog` accepts `--live` and runs locally regardless; `--write` is the explicit local-write flag.

## Rules (V1–V12)

Validation runs over a package snapshot (file name → text) plus an
optional target snapshot. A `.changes` snapshot is the normalized
form of the file (see below); target reads are public
(`list_files`, plus `cat` for `.spec` and a byte fetch for
`.changes`, normalized at load like the local files). A target that
does not exist is a new package: the target-relative rules are
skipped. Any other fetch failure is a hard error.

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
- V5 — `.changes` headers parse and run strictly descending, old
  entries included (the source validator checks every header, so a
  foreign-style old header is repaired, not tolerated); a
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
  license operation counts as non-substantive. No news found → the rule is
  skipped; when no source tarball was on disk at all,
  the skip says so — the check skipped the line. A leftover `TODO`
  slot is also a V9 finding.
- V10 — the license tokens in the newest entry match the spec
  `License:` (normalised: `GPLv3` → `GPL-3.0`, `-only`/`-or-later`
  stripped; a token must carry a letter and a digit, or be a known
  short name). A license change claimed in the entry while the spec
  `License:` is unchanged vs the target is also a finding (a line
  stating there is no change counts as no claim).
- V11 — the newest N entries (default 1) pass the format lint
  (bullets `- ` at indent 0, `* ` nested at indent 2), and the
  submission carries at most N new entries vs the target; older
  entries match the target's up to the whole-file normalization
  `generate --write` applies (sanctioned exceptions are named in
  the message for the human to judge).
- V12 — bug references: a `bnc#` reference (use `boo#`/`bsc#`),
  incomplete CVE ids, a bare `CVE(s)` or
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
explicit `TODO` reason slot for a gap the diff leaves. The
upstream NEWS section for the new version, extracted from the source
tarball on disk, arrives as a `TODO` highlights slot (up to 15 lines,
wrapped at 67 columns) — the human distills it; the slot counts as
non-substantive.

`needs_detail` is set when news exists, the version changed, and the
draft has no substantive line beyond the version line (mechanical
file/dependency/license lines count as non-substantive). `--write`
prepends the draft into the package's `.changes` file
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
naming the encoding.

## NEWS extraction

`find_tarball` prefers `Source0`, then a tarball carrying the
version, then a sole tarball. Extraction uses the `tar` CLI
(`tar -tf` / `tar -xOf`, 30 s timeout, 1 MiB output cap per stream). Archive
paths and member names starting with `-` are passed as operands
(`--` before the member; `./`-prefix on the archive path).
The section is the text under
the heading naming the version, up to the next version heading or a
heading of equal or higher level; deeper subheadings (`### Added`
under `## 2.0`) stay inside the section. A file with no version
headings yields an empty section. A lookup failure is
an error value.

## Scope

This layer validates and generates `.changes` entries; filing, OBS mutation, and commit wiring live in their own layers. V1–V8 and V10–V12 are exact; V9 is a deterministic heuristic gated on the NEWS section existing. Old entries change through the whole-file normalization: `--write` normalizes the whole file (trailing whitespace, CRLF→LF, legacy encodings to UTF-8, BOM stripped) and V11 compares older entries after that normalization, so a difference normalization alone explains counts as no finding.
