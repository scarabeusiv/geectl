#!/usr/bin/env python3
"""Check the CLI-output design doc's literal examples against the code.

The design doc (docs/cli-output-design.md) states example output lines
that the code must be able to produce literally. Hand-written examples
drift: a wrong word, a missing code. This script makes that drift fail
the gate instead of surfacing in OCR passes.

Usage:
    python3 scripts/check-docs-conformance.py [--doc PATH] [--root PATH]

``--doc`` defaults to ``docs/cli-output-design.md`` under ``--root``
(default: the repository root, derived from this script's location).
Exits 0 when every checkable example verifies, 1 otherwise, with one
``FAIL`` line per failure naming the doc line, the example, and what
was found instead.

Rules (the doc convention this script enforces):

1. Scope: only section 1 (``## 1. The debug channel`` up to
   ``## 2. Human-readable output``) is checked. The worked examples
   in section 2 are labelled by the doc itself as "shape, not an
   exhaustive field list" — they illustrate renderers that do not
   exist yet, so they carry no pin. They gain citations when the
   renderers land; extending this script to section 2 then is a
   follow-up, not a silent gap: the summary names the exclusion.
2. Extraction: an example is an inline-code span whose text starts
   with a debug-line prefix (``http:``, ``retry:``, ``decision:``,
   ``service_state``, ``list_services``, ``config:``, ``error:``,
   ``remediation:``, ``geectl ``). Spans may wrap across lines; the
   wrapped text is joined with single spaces.
3. Fragments: an example is split on its schematic markers
   (``...`` and angle-bracket placeholders ``<...>``) into literal
   fragments. A fragment counts only if it states more than a bare
   prefix: at least two whitespace-separated tokens. A span with no
   such fragment (``http: ...``) is schematic and is reported as
   skipped, never silently. The markers are the doc's own way of
   saying "illustrative here"; the rest of the line is still checked.
4. Citations: a literal example is cited by ``(test `path:LINE`)``
   (or ``test `path:LINE` ``) following it in the same bullet, where
   ``LINE`` may be one line, a comma list (``931,947``), or a range
   (``10-12``). The citation belongs to the nearest preceding
   example span in that bullet. The check verifies (a) every cited
   line exists and (b) every fragment of the example verifies against
   a single cited line (rule 5). A citation with no preceding example
   is a failure, as is a second citation for one example and any
   ``test `...` `` text that looks like a citation — it carries a
   ``:`` (a bare path such as ``src/http.rs`` is prose, not a
   citation) — but does not parse as ``path:LINE[s]``: an unparsable
   citation fails instead of being silently ignored, while ordinary
   prose (``run the test `debug_on_records` ``) does not. The plural
   ``tests`` and the no-space ``test`` form are caught the same way:
   they are misspelled citations, so they fail rather than being
   silently ignored. Line numbers are ASCII digits only, at most
   seven of them; a non-ASCII digit or a longer run does not parse.
   A range in a citation may span at most 1000 lines; a wider
   range does not parse. ``emit sites `path:L1,L2,...` `` covers
   every example span in its bullet and marks them unpinned by
   design: the check verifies each listed line exists — the spec is
   parsed the same way as a test citation, so a malformed emit spec
   fails instead of crashing or being silently ignored — and requires
   no test pin for them.
5. Token check (per fragment, per cited line): durations are
   stripped from both sides exactly the way the test helper
   ``debug_lines_without_durations`` does it (drop from the first
   `` in `` to `` (attempt`` when both are present and ordered).
   The cited line's string-literal content is used when it is one.
   Instance values are skipped: path tokens (starting with ``/`` —
   the code emits whatever path the request has) and pure-number
   tokens of five or more digits (a request id: the doc's
   ``1398977`` and the test's ``42`` are the same shape, different
   instances). Every other token must appear as a whole token in
   ONE cited line: a fragment whose tokens are split across two
   cited lines does not verify (the lines are checked separately,
   never pooled). Whole-token matching is what catches the drift
   class: ``accept`` is not ``accepted``, and a trailing ``status)``
   is not ``status 500)``. A cited line pins its exact counters
   (``attempt 2/4`` is not ``attempt 3/4``): counters are instance
   values only for the uncited search (rule 6), not for a pin.
   A fragment with no checkable (non-instance-value) tokens pins
   nothing: it is illustrative, not verified.
6. Missing citation: an uncited example is searched for in the repo
   (``src/`` and ``tests/``, duration-stripped), fragment by
   fragment, longest first. First verbatim: if any fragment appears
   verbatim in a test assertion or emitting code, the line is
   literal and missing its citation: a failure naming the fragment
   and where it was found. Then normalized: both sides are reduced
   to tokens with instance values normalized (paths and >=5-digit
   ids dropped, counters such as ``3/30s`` or ``1/4`` replaced by
   one placeholder — but bare status codes such as ``500`` stay
   pinned vocabulary), and a fragment counts as pinned only when
   its normalized tokens are a subset of exactly ONE repo line's.
   A normalized pin is a failure naming the line. If the normalized
   tokens match several lines, the example is a generic shape, not
   a pin: it is reported as illustrative (ambiguous pin), never
   failed on an arbitrary one. Only fragments of at least three
   tokens can pin (rule 8); a shorter fragment is illustrative even
   when it matches. Documented case: the doc's
   ``-> 200 in 123ms (attempt 1/4)`` shape matches several 200 tests
   after normalization, so it stays illustrative; the
   ``probe 3/30s`` line matches exactly one test (``probe 1/30s``)
   and is a missing citation. If no fragment appears anywhere, the
   line is illustrative (its values are invented) and is reported
   as skipped, not failed.
7. A missing doc file is a failure: the design doc is a required
   record on main. (Branches predating the doc rebase onto main
   before merging; the check does not silently pass on its absence.)
8. Pin threshold: exactly one threshold governs pinning — a fragment
   with fewer than three whitespace-separated tokens never pins.
   Two tokens (a bare prefix plus one word, e.g. ``http: GET``)
   match unrelated lines by traversal order, so such an example is
   illustrative, not a missing citation. The threshold applies to
   both the verbatim and the normalized search, and only to pinning:
   a cited example's fragments are still verified against their
   cited line whatever their length (rule 5).

When this script reports a failure, the fix direction follows the
which-side-is-right rule: decide first whether the doc or the code
states the intended behaviour. Never tighten the doc to bless
behaviour the design says is wrong.
"""

import argparse
import re
import sys
from pathlib import Path

SECTION_START = "## 1. The debug channel"
SECTION_END = "## 2. Human-readable output"
DEFAULT_DOC = "docs/cli-output-design.md"

# Inline-code spans starting with one of these are example output lines.
PREFIXES = (
    "http:",
    "retry:",
    "decision:",
    "service_state",
    "list_services",
    "config:",
    "error:",
    "remediation:",
    "geectl ",
)

SPAN_RE = re.compile(r"`([^`]+)`", re.DOTALL)
# A test citation: path:LINE, path:L1,L2, path:L1-L2, or mixes thereof.
# Line numbers are ASCII digits only ([0-9], never \d): \d also
# matches non-ASCII decimal digits, so a citation a human reads as
# non-numeric would silently mean another line.
TEST_CITE_RE = re.compile(r"test `([^`]+):([0-9,\-]+)`")
# Only `test `...` ` text that looks like a citation — it carries a
# `:` — is an unparsable citation: a bare path such as
# `src/http.rs` is prose, and ordinary prose such as
# `run the test `debug_on_records` ` is not. The plural `tests` and
# the no-space `test` form are misspelled citations: they fail
# instead of being silently ignored. (A `:` without a digit run,
# e.g. `src/x.rs:abc`, is still a citation attempt and still fails:
# requiring a digit here would silently ignore it.)
ANY_TEST_CITE_RE = re.compile(r"tests?\s*`[^`]*:[^`]*`")
EMIT_CITE_RE = re.compile(r"emit sites `([^`]+):([0-9,]+)`")
# Any `emit sites `...` ` text: matches that fail EMIT_CITE_RE are
# unparsable emit specs and must fail, never be silently ignored.
ANY_EMIT_CITE_RE = re.compile(r"emit sites `[^`]+`")
PLACEHOLDER_RE = re.compile(r"<[^<>]*>")
ID_TOKEN_RE = re.compile(r"^[0-9]{5,}$")
COUNTER_TOKEN_RE = re.compile(r"^[0-9]+/[0-9]+s?$")


# A citation range may span at most this many lines: a wider range
# does not parse. Without the cap a mistyped range such as
# `1-999999999` would build a gigantic list and wedge the CI job.
MAX_CITE_RANGE = 1000


# A line number in a citation is ASCII digits only, and at most seven
# of them: ``str.isdigit`` accepts non-ASCII decimal digits (Arabic-
# Indic 3 reads as 3), and ``int`` raises ValueError past
# Python's 4300-digit string-conversion limit — both must fail
# the parse instead of silently meaning another line or crashing the
# gate. Seven digits still allows any real file (9,999,999 lines).
MAX_CITE_DIGITS = 7


def _ascii_line_number(text):
    """An ASCII-only line number as an int, or None when it is not one."""
    if not text or len(text) > MAX_CITE_DIGITS:
        return None
    if not all("0" <= c <= "9" for c in text):
        return None
    return int(text)


def parse_cite_lines(spec):
    """Expand a citation line spec ('10', '10-12', '931,947') to a
    sorted list of 1-based line numbers, or None when it does not parse."""
    lines = []
    for part in spec.split(","):
        part = part.strip()
        if not part:
            return None
        if "-" in part:
            first, last = part.split("-", 1)
            first_n = _ascii_line_number(first)
            last_n = _ascii_line_number(last)
            if first_n is None or last_n is None:
                return None
            if last_n < first_n:
                return None
            if last_n - first_n + 1 > MAX_CITE_RANGE:
                return None
            lines.extend(range(first_n, last_n + 1))
        else:
            n = _ascii_line_number(part)
            if n is None:
                return None
            lines.append(n)
    return sorted(set(lines))


def strip_durations(line):
    """Mirror debug_lines_without_durations: drop ' in <dur>' before '(attempt'."""
    start = line.find(" in ")
    end = line.find(" (attempt")
    if start != -1 and end != -1 and start < end:
        return line[:start] + line[end:]
    return line


def literal_content(line):
    """Content of the first double-quoted string, or the raw line."""
    start = line.find('"')
    if start == -1:
        return line
    out = []
    i = start + 1
    while i < len(line):
        ch = line[i]
        if ch == "\\" and i + 1 < len(line):
            out.append(line[i + 1])
            i += 2
        elif ch == '"':
            return "".join(out)
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def join_span(text):
    return re.sub(r"\s+", " ", text).strip()


def line_number(text, pos):
    return text.count("\n", 0, pos) + 1


def fragments(text):
    """Literal fragments of an example: split on schematic markers.

    A fragment counts only if it states more than a bare prefix
    (at least two whitespace-separated tokens). A span with no such
    fragment is schematic. Whether a fragment can pin is rule 8's
    threshold, not this function's: a two-token fragment is a
    fragment here, it just never pins.
    """
    parts = re.split(r"\.\.\.|" + PLACEHOLDER_RE.pattern, text)
    out = []
    for part in parts:
        part = join_span(part)
        if len(part.split()) >= 2:
            out.append(part)
    return out


def extract_section(doc_text, doc_path):
    start = doc_text.find(SECTION_START)
    end = doc_text.find(SECTION_END)
    if start == -1 or end == -1 or end <= start:
        raise ValueError(
            f"{doc_path}: expected {SECTION_START!r} followed by "
            f"{SECTION_END!r}; refusing to check a restructured doc"
        )
    return doc_text[start:end], start


def find_spans(section, section_offset, doc_text):
    """Example spans in the section: (doc_line, text, start, end)."""
    spans = []
    for m in SPAN_RE.finditer(section):
        text = join_span(m.group(1))
        if text.startswith(PREFIXES):
            spans.append(
                (
                    line_number(doc_text, section_offset + m.start()),
                    text,
                    section_offset + m.start(),
                    section_offset + m.end(),
                )
            )
    return spans


def same_bullet(doc_text, span_end, cite_pos):
    """True when no blank line and no new bullet lie between the two."""
    between = doc_text[span_end:cite_pos]
    return "\n\n" not in between and "\n- " not in between


def attribute_citations(section, section_offset, doc_text, spans):
    """Attribute citations to spans.

    A test citation belongs to the nearest preceding example span in
    its bullet. An emit-sites citation covers every example span in
    its bullet preceding it. Returns (test_cites, emit_covered,
    emit_cites, orphans, bad_cites, bad_emit) where test_cites maps
    span index -> (path, [lines]), emit_covered is a set of span
    indexes, emit_cites is a list of (doc_line, path, [lines]),
    orphans is a list of (doc_line, message) for citations with no
    preceding example or a second citation for one example (distinct
    messages per kind), bad_cites is a list of (doc_line, text)
    for `test `...` ` text that does not parse as ``path:LINE[s]``,
    and bad_emit is a list of (doc_line, text) for `emit sites `...` `
    text that does not parse as ``path:L1,L2,...``.
    """
    test_cites = {}
    emit_covered = set()
    emit_cites = []
    orphans = []
    bad_cites = []
    bad_emit = []
    parsed_spans = set()
    emit_parsed_spans = set()
    for m in TEST_CITE_RE.finditer(section):
        cite_pos = section_offset + m.start()
        cite_line = line_number(doc_text, cite_pos)
        parsed_spans.add((m.start(), m.end()))
        lines = parse_cite_lines(m.group(2))
        if lines is None:
            bad_cites.append((cite_line, m.group(0)))
            continue
        target = None
        for i, (_line, _text, _s, span_end) in enumerate(spans):
            if span_end > cite_pos:
                break
            if same_bullet(doc_text, span_end, cite_pos):
                target = i
        if target is None:
            orphans.append((cite_line, f"citation {m.group(0)} has no preceding example"))
        elif target in test_cites:
            orphans.append((cite_line, f"second citation for one example: {m.group(0)}"))
        else:
            test_cites[target] = (m.group(1), lines)
    for m in ANY_TEST_CITE_RE.finditer(section):
        if (m.start(), m.end()) in parsed_spans:
            continue
        cite_line = line_number(doc_text, section_offset + m.start())
        bad_cites.append((cite_line, m.group(0)))
    for m in EMIT_CITE_RE.finditer(section):
        cite_pos = section_offset + m.start()
        cite_line = line_number(doc_text, cite_pos)
        emit_parsed_spans.add((m.start(), m.end()))
        lines = parse_cite_lines(m.group(2))
        if lines is None:
            bad_emit.append((cite_line, m.group(0)))
            continue
        emit_cites.append((cite_line, m.group(1), lines))
        for i, (_line, _text, _s, span_end) in enumerate(spans):
            if span_end > cite_pos:
                break
            if i not in test_cites and same_bullet(doc_text, span_end, cite_pos):
                emit_covered.add(i)
    for m in ANY_EMIT_CITE_RE.finditer(section):
        if (m.start(), m.end()) in emit_parsed_spans:
            continue
        cite_line = line_number(doc_text, section_offset + m.start())
        bad_emit.append((cite_line, m.group(0)))
    return test_cites, emit_covered, emit_cites, orphans, bad_cites, bad_emit


def read_cited_lines(root, path, lines):
    """Contents of the cited lines of a repo file, or None with the reason.

    ``lines`` is a list of 1-based line numbers. Returns a list of
    (line_number, text) pairs, in the cited order.
    """
    target = root / path
    if not target.is_file():
        return None, f"cited file {path} does not exist"
    all_lines = target.read_text(encoding="utf-8", errors="replace").splitlines()
    for n in lines:
        if n < 1 or n > len(all_lines):
            return None, f"cited line {n} out of range in {path} ({len(all_lines)} lines)"
    return [(n, all_lines[n - 1]) for n in lines], None


def token_check(fragment, cited_lines):
    """Whole-token check of one fragment against its cited line(s).

    Each cited line is checked separately: a fragment verifies only
    when every (non-instance-value) token is present in ONE cited
    line — the lines are never pooled, so tokens split across two
    lines do not verify. Returns the first fragment token missing
    from the first cited line, None when some line holds them all,
    or "" when the fragment has no checkable (non-instance-value)
    tokens at all: such a fragment pins nothing.
    """
    fragment = strip_durations(fragment)
    tokens = [
        t
        for t in fragment.split()
        if not t.startswith("/") and not ID_TOKEN_RE.match(t)
    ]
    if not tokens:
        # Every token is an instance value, so this fragment pins
        # nothing — it must not count as verified against any cited
        # line.
        return ""
    for _n, line in cited_lines:
        cited_tokens = set(strip_durations(literal_content(line)).split())
        if all(t in cited_tokens for t in tokens):
            return None
    first_tokens = set(strip_durations(literal_content(cited_lines[0][1])).split())
    return next((t for t in tokens if t not in first_tokens), None)


def search_repo(root, fragment):
    """First (path, line) in src/ or tests/ containing fragment, or None."""
    needle = strip_durations(fragment)
    for base in ("src", "tests"):
        top = root / base
        if not top.is_dir():
            continue
        for path in sorted(top.rglob("*.rs")):
            for n, line in enumerate(
                path.read_text(encoding="utf-8", errors="replace").splitlines(), 1
            ):
                if needle in strip_durations(line):
                    return (str(path.relative_to(root)), n)
    return None


def normalize_token(token):
    """Instance-value normal form of one token, or None to drop it.

    Paths and >=5-digit ids are dropped (same instance-value classes
    as the cited-line check). A counter token (``3/30s``, ``1/4``,
    with a trailing ``:``, ``)`` or ``,`` stripped first) becomes the
    single placeholder ``<counter>``. Every other token — including
    bare status codes such as ``500`` — is kept verbatim.
    """
    if token.startswith("/"):
        return None
    if ID_TOKEN_RE.match(token):
        return None
    if COUNTER_TOKEN_RE.match(token.rstrip(":),")):
        return "<counter>"
    return token


def normalize_tokens(text):
    """The set of normalized tokens of a line (string-literal content
    when the line holds one), durations stripped."""
    return {
        t
        for t in (normalize_token(tok) for tok in strip_durations(text).split())
        if t is not None
    }


def normalized_search(root, fragment):
    """All (path, line) in src/ or tests/ whose normalized tokens
    cover the fragment's, sorted by path then line.

    A fragment counts as pinned only when exactly one line matches:
    a generic shape matching several lines is ambiguous, not a pin,
    and the caller treats that as illustrative, never as a failure
    naming an arbitrary one.
    """
    want = normalize_tokens(fragment)
    if not want:
        return []
    matches = []
    for base in ("src", "tests"):
        top = root / base
        if not top.is_dir():
            continue
        for path in sorted(top.rglob("*.rs")):
            for n, line in enumerate(
                path.read_text(encoding="utf-8", errors="replace").splitlines(), 1
            ):
                if want <= normalize_tokens(literal_content(line)):
                    matches.append((str(path.relative_to(root)), n))
    return matches


def check(doc_path, root):
    """Run the check. Returns (failures, summary_lines)."""
    failures = []
    summary = []
    if not doc_path.is_file():
        return [f"FAIL {doc_path}: design doc not found"], summary
    doc_text = doc_path.read_text(encoding="utf-8")
    try:
        section, section_offset = extract_section(doc_text, doc_path)
    except ValueError as exc:
        return [f"FAIL {exc}"], summary
    spans = find_spans(section, section_offset, doc_text)
    test_cites, emit_covered, emit_cites, orphans, bad_cites, bad_emit = attribute_citations(
        section, section_offset, doc_text, spans
    )
    for cite_line, message in orphans:
        failures.append(f"FAIL {doc_path}:{cite_line}: {message}")
    for cite_line, cite in bad_cites:
        failures.append(
            f"FAIL {doc_path}:{cite_line}: unparsable test citation {cite!r}: "
            "expected `path:LINE` (comma list or range allowed)"
        )
    for emit_line, cite in bad_emit:
        failures.append(
            f"FAIL {doc_path}:{emit_line}: unparsable emit-sites citation {cite!r}: "
            "expected `path:L1,L2,...`"
        )
    for emit_line, path, lines in emit_cites:
        for n in lines:
            _found, err = read_cited_lines(root, path, [n])
            if err:
                failures.append(f"FAIL {doc_path}:{emit_line}: emit site {err}")
    counts = {"verified": 0, "schematic": 0, "illustrative": 0, "unpinned": 0}
    for i, (doc_line, text, _s, _e) in enumerate(spans):
        frags = fragments(text)
        if i in test_cites:
            path, lines = test_cites[i]
            cited, err = read_cited_lines(root, path, lines)
            if err:
                failures.append(f"FAIL {doc_path}:{doc_line}: {text!r}: {err}")
                continue
            if not frags:
                counts["schematic"] += 1
                summary.append(f"SKIP {doc_path}:{doc_line}: schematic: {text}")
                continue
            failed = False
            any_checkable = False
            for frag in frags:
                missing = token_check(frag, cited)
                if missing is None:
                    any_checkable = True
                    continue
                if missing == "":
                    # No checkable tokens: the fragment pins nothing
                    # Neither verified nor failed.
                    continue
                failures.append(
                    f"FAIL {doc_path}:{doc_line}: {text!r}: token {missing!r} "
                    f"not in {path}:{lines[0]}"
                )
                failed = True
                break
            if failed:
                continue
            if any_checkable:
                counts["verified"] += 1
            else:
                counts["illustrative"] += 1
                summary.append(
                    f"SKIP {doc_path}:{doc_line}: illustrative, no checkable "
                    f"tokens: {text}"
                )
            continue
        if i in emit_covered:
            counts["unpinned"] += 1
            summary.append(f"SKIP {doc_path}:{doc_line}: unpinned by design: {text}")
            continue
        if not frags:
            counts["schematic"] += 1
            summary.append(f"SKIP {doc_path}:{doc_line}: schematic: {text}")
            continue
        found = None
        # Only fragments of at least three tokens can pin (rule 8):
        # a shorter fragment (``http: GET``) matches unrelated lines
        # by traversal order and is illustrative, not a pin.
        pinnable = [f for f in frags if len(f.split()) >= 3]
        # Longest fragment first: a short prefix fragment (``http: GET
        # /source/``) matches unrelated lines, while the longest
        # fragment names the line that actually pins the example.
        for frag in sorted(pinnable, key=len, reverse=True):
            found = search_repo(root, frag)
            if found:
                break
        if found:
            failures.append(
                f"FAIL {doc_path}:{doc_line}: {text!r}: literal example has no "
                f"citation; pinned at {found[0]}:{found[1]}"
            )
            continue
        # Second pass, normalized: counters/paths/ids are instance
        # values, so a fragment whose only differences are those is
        # still a pin — but only when exactly one repo line matches.
        # Break only on a unique match: an ambiguous longest
        # fragment must not stop a shorter fragment from pinning.
        norm = []
        for frag in sorted(pinnable, key=len, reverse=True):
            found_norm = normalized_search(root, frag)
            if len(found_norm) == 1:
                norm = found_norm
                break
            if found_norm:
                norm = found_norm
        if len(norm) == 1:
            failures.append(
                f"FAIL {doc_path}:{doc_line}: {text!r}: literal example has no "
                f"citation; pinned at {norm[0][0]}:{norm[0][1]} "
                "(normalized: counters/paths/ids)"
            )
        elif len(norm) > 1:
            counts["illustrative"] += 1
            summary.append(
                f"SKIP {doc_path}:{doc_line}: ambiguous pin ({len(norm)} matches), "
                f"treating as illustrative: {text}"
            )
        else:
            counts["illustrative"] += 1
            summary.append(f"SKIP {doc_path}:{doc_line}: illustrative, no pin: {text}")
    summary.append(
        f"{doc_path}: {len(spans)} example(s): {counts['verified']} verified, "
        f"{counts['schematic']} schematic skipped, "
        f"{counts['illustrative']} illustrative (no pin), "
        f"{counts['unpinned']} unpinned by design; "
        f"section 2 worked examples out of scope (shape-only by the doc); "
        f"{len(failures)} failure(s)"
    )
    return failures, summary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--doc", default=DEFAULT_DOC, help="design doc to check (default: %(default)s)"
    )
    parser.add_argument(
        "--root",
        default=None,
        help="repo root (default: parent of this script's directory)",
    )
    args = parser.parse_args(argv)
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    doc_path = Path(args.doc)
    if not doc_path.is_absolute():
        doc_path = root / doc_path
    failures, summary = check(doc_path, root)
    for line in summary:
        print(line)
    for failure in failures:
        print(failure)
    if failures:
        print(f"conformance check FAILED: {len(failures)} failure(s)")
        return 1
    print("conformance check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
