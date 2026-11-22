#!/usr/bin/env python3
"""Self-tests for scripts/check-docs-conformance.py.

Runnable with python3 directly, no cargo, no network:

    python3 scripts/test-check-docs-conformance.py

Every case builds a miniature doc + repo in a temp dir, so the tests
are hermetic: they do not read the real design doc or the real tree.
The wrong-word and missing-code regression cases use the real strings (a doc saying
``re-issuing accept`` where the test pins ``accepted``, and a doc
saying ``transient status`` where the test pins ``transient status
500``) against synthetic test files.
"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).resolve().parent / "check-docs-conformance.py"
spec = importlib.util.spec_from_file_location("check_docs_conformance", CHECKER)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

DOC_HEAD = "# geectl CLI output — design\n\n## 1. The debug channel (#51)\n\n"
DOC_TAIL = "\n## 2. Human-readable output (#52)\n\n- `request show <id>` (single object): `id: 1398977`, `state: new`.\n"


def write_case(root, doc, files):
    """Write doc + repo files into root; return the doc path."""
    doc_path = root / "docs" / "cli-output-design.md"
    doc_path.parent.mkdir(parents=True, exist_ok=True)
    doc_path.write_text(doc, encoding="utf-8")
    for rel, content in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return doc_path


class CheckTests(unittest.TestCase):
    def run_check(self, doc, files):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            doc_path = write_case(root, doc, files)
            failures, summary = checker.check(doc_path, root)
            return failures, summary

    def test_cited_example_passes(self):
        """Example + citation match, with an instance id differing."""
        doc = (
            DOC_HEAD
            + "- Accept check (`decision: request 1398977 still new, re-issuing\n"
            + "  accepted`; test `src/obs/request.rs:10`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/request.rs": "\n".join(
                ["", "", "", "", "", "", "", "", "",
                 '    "decision: request 42 still new, re-issuing accepted".to_string(),']
            )
            + "\n"
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("1 verified" in line for line in summary))

    def test_cited_id_with_trailing_comma_passes(self):
        """A cited id carrying sentence punctuation is an instance value."""
        doc = (
            DOC_HEAD
            + "- Accept check (`decision: request 1398977, still new, re-issuing\n"
            + "  accepted`; test `src/obs/request.rs:10`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/request.rs": "\n".join(
                ["", "", "", "", "", "", "", "", "",
                 '    "decision: request 42 still new, re-issuing accepted".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(failures, [])

    def test_trailing_comma_still_fails_equality(self):
        """Punctuation stripping is class-test only: `new,` must match `new,`."""
        doc = (
            DOC_HEAD
            + "- Accept check (`decision: request 1398977, still new, re-issuing\n"
            + "  accepted`; test `src/obs/request.rs:10`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/request.rs": "\n".join(
                ["", "", "", "", "", "", "", "", "",
                 '    "decision: request 42 still new re-issuing accepted".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("'new,'", failures[0])


    def test_wrong_word_fails(self):
        """Doc says 'accept', test pins 'accepted': the token is missing."""
        doc = (
            DOC_HEAD
            + "- Accept check (`decision: request 1398977 still new, re-issuing\n"
            + "  accept`; test `src/obs/request.rs:10`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/request.rs": "\n".join(
                ["", "", "", "", "", "", "", "", "",
                 '    "decision: request 42 still new, re-issuing accepted".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("'accept'", failures[0])
        self.assertIn("src/obs/request.rs:10", failures[0])

    def test_missing_code_fails(self):
        """Doc says 'transient status', test pins 'transient status 500'."""
        doc = (
            DOC_HEAD
            + "- Retry (`http: GET /source/... -> 500 in 210ms (attempt 2/4, retry in 2s: transient status)`;\n"
            + "  test `src/http.rs:20`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                [""] * 19
                + ['    "http: GET /public/source/foo -> 500 (attempt 2/4, retry in 2s: transient status 500)".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("'status)'", failures[0])
        self.assertIn("src/http.rs:20", failures[0])

    def test_exhaustion_tail_missing_citation(self):
        """A '...' line whose literal tail is pinned still needs its citation."""
        doc = (
            DOC_HEAD
            + "- Exhaustion (`http: GET /source/... failed after 4 attempts (last: 500)`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                [""] * 9
                + ['    "http: GET /public/source/foo failed after 4 attempts (last: 500)".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/http.rs:10", failures[0])

    def test_longest_fragment_names_the_pin(self):
        """A generic prefix fragment must not outrank the specific tail."""
        doc = (
            DOC_HEAD
            + "- Exhaustion (`http: GET /source/... failed after 4 attempts (last: 500)`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                [""] * 9
                + ['    "http: GET /public/source/foo failed after 4 attempts (last: 500)".to_string(),']
                + [""] * 9
                + ['    "http: GET /source/foo response body (4 bytes)".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("src/http.rs:10", failures[0])

    def test_dry_run_fragment_pinned(self):
        """A '<...>' line whose literal prefix is pinned needs its citation."""
        doc = (
            DOC_HEAD
            + "- Dry run (`decision: dry-run: mutation refused, plan only: <action>`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/types.rs": (
                '    "decision: dry-run: mutation refused, plan only: {action}"\n'
            )
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/obs/types.rs:1", failures[0])

    def test_corrected_line_passes(self):
        """The corrected line (with the code, public path) verifies."""
        doc = (
            DOC_HEAD
            + "- Retry (`http: GET /public/source/... -> 500 in 210ms (attempt 2/4, retry in 2s: transient status 500)`;\n"
            + "  test `src/http.rs:20`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                [""] * 19
                + ['    "http: GET /public/source/foo -> 500 (attempt 2/4, retry in 2s: transient status 500)".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(failures, [])

    def test_duration_stripped(self):
        """'in 210ms' in the doc matches a cited line without durations."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /public/source/foo -> 200 in 123ms (attempt 1/4)`;\n"
            + "  test `src/http.rs:5`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                [""] * 4
                + ['    "http: GET /public/source/foo -> 200 (attempt 1/4)".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(failures, [])

    def test_missing_citation(self):
        """A literal line pinned in the repo but carrying no citation fails."""
        doc = (
            DOC_HEAD
            + "- Dedup (`decision: open submit request(s) 1398492 already target foo; refusing\n"
            + "  to file`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/payload.rs": (
                '    "decision: open submit request(s) 1398492 already target foo; refusing to file"\n'
            )
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/obs/payload.rs:1", failures[0])

    def test_illustrative_skipped(self):
        """A literal-looking line pinned nowhere is illustrative, not a failure."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /source/openSUSE:Factory/geectl -> 200 in 123ms (attempt 1/4)`).\n"
            + DOC_TAIL
        )
        failures, summary = self.run_check(doc, {})
        self.assertEqual(failures, [])
        self.assertTrue(any("illustrative" in line for line in summary))

    def test_schematic_skipped(self):
        """'...' with no checkable fragment is schematic: skipped, never verified."""
        doc = (
            DOC_HEAD
            + "- Shapes (`http: ...`, `retry: ...`, `decision: ...`, `config: ...`).\n"
            + DOC_TAIL
        )
        failures, summary = self.run_check(doc, {})
        self.assertEqual(failures, [])
        self.assertTrue(any("4 schematic skipped" in line for line in summary))

    def test_bad_citation(self):
        """A citation pointing at a missing file or line fails."""
        doc = (
            DOC_HEAD
            + "- Accept check (`decision: request 1398977 still new, re-issuing accepted`;\n"
            + "  test `src/obs/nope.rs:10`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("does not exist", failures[0])

    def test_orphan_citation(self):
        """A citation with no preceding example in its bullet fails."""
        doc = DOC_HEAD + "- Note (test `src/http.rs:5`).\n" + DOC_TAIL
        failures, _ = self.run_check(doc, {"src/http.rs": "x\n" * 5})
        self.assertEqual(len(failures), 1)
        self.assertIn("no preceding example", failures[0])

    def test_emit_sites_unpinned(self):
        """Emit-sites citations need resolving lines, not a test pin."""
        doc = (
            DOC_HEAD
            + "- Service lines (`service_state <project>/<package>: <code>` and\n"
            + "  `list_services <project>/<package>: <n> service(s)` — emit sites `src/obs/services.rs:2,4`).\n"
            + DOC_TAIL
        )
        files = {"src/obs/services.rs": "a\nb\nc\nd\n"}
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("unpinned by design" in line for line in summary))

    def test_emit_sites_must_resolve(self):
        doc = (
            DOC_HEAD
            + "- Service lines (`service_state <project>/<package>: <code>` — emit sites `src/obs/services.rs:99`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {"src/obs/services.rs": "a\n"})
        self.assertEqual(len(failures), 1)
        self.assertIn("out of range", failures[0])

    def test_section2_excluded(self):
        """Worked examples in section 2 are shape-only: not checked.

        The section-2 span below is a literal line pinned in the
        repo: if the checker ever started reading section 2, this
        case would fail — so a vacuous pass is impossible.
        """
        doc = (
            DOC_HEAD
            + "## 2. Human-readable output (#52)\n\n"
            + "- `http: GET /source/foo -> 200`.\n"
        )
        files = {
            "src/http.rs": '    "http: GET /source/foo -> 200".to_string(),\n'
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("out of scope" in line for line in summary))

    def test_comma_list_citation_passes(self):
        """A citation may name several lines: each must exist."""
        doc = (
            DOC_HEAD
            + "- Service (`service_state foo/bar: ok`; test `src/x.rs:2,4`).\n"
            + DOC_TAIL
        )
        files = {
            "src/x.rs": "\n".join(
                ["// head", "// filler", "// more",
                 '    "service_state foo/bar: ok".to_string(),']
            )
            + "\n"
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("1 verified" in line for line in summary))

    def test_unparsable_citation_fails(self):
        """`test `...` ` text that does not parse is a failure, not ignored."""
        doc = (
            DOC_HEAD
            + "- Service (`service_state foo/bar: ok`; test `src/x.rs:abc`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_range_citation_split_tokens_fail(self):
        """A fragment split across two cited lines does not verify."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo charlie`; test `src/x.rs:2-3`).\n"
            + DOC_TAIL
        )
        files = {
            "src/x.rs": "\n".join(
                ["// head",
                 '    "decision: alpha bravo".to_string(),',
                 '    "charlie delta".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("'charlie'", failures[0])
        self.assertIn("src/x.rs:2", failures[0])

    def test_range_citation_single_line_passes(self):
        """A range citation verifies when one cited line holds all tokens."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo charlie`; test `src/x.rs:2-3`).\n"
            + DOC_TAIL
        )
        files = {
            "src/x.rs": "\n".join(
                ["// head",
                 '    "decision: alpha".to_string(),',
                 '    "decision: alpha bravo charlie".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(failures, [])

    def test_duplicate_citation_orphan(self):
        """A second citation for one example is an orphan failure."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; test `src/x.rs:2`; test `src/x.rs:3`).\n"
            + DOC_TAIL
        )
        files = {
            "src/x.rs": "\n".join(
                ["// head",
                 '    "decision: alpha bravo".to_string(),',
                 '    "decision: alpha bravo".to_string(),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("second citation for one example", failures[0])
        # A duplicate is labelled a duplicate, not an orphan.
        self.assertNotIn("has no preceding example", failures[0])

    def test_out_of_range_test_citation(self):
        """A test citation past the end of its file fails."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; test `src/x.rs:99`).\n"
            + DOC_TAIL
        )
        files = {"src/x.rs": "a\nb\nc\nd\ne\n"}
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("out of range", failures[0])

    def test_emit_spec_malformed_fails_not_crash(self):
        """A malformed emit-sites spec fails, never raises ValueError."""
        for spec in ("2,,4", "2,"):
            with self.subTest(spec=spec):
                doc = (
                    DOC_HEAD
                    + "- Service (`service_state foo/bar: ok` — emit sites `src/x.rs:"
                    + spec + "`).\n"
                    + DOC_TAIL
                )
                files = {"src/x.rs": "a\nb\nc\nd\ne\n"}
                failures, _ = self.run_check(doc, files)
                self.assertEqual(len(failures), 1)
                self.assertIn("unparsable emit-sites", failures[0])

    def test_emit_spec_unparsable_fails(self):
        """Emit-sites text that does not parse is a failure, not ignored."""
        for spec in ("", "2-3"):
            with self.subTest(spec=spec):
                doc = (
                    DOC_HEAD
                    + "- Service (`service_state foo/bar: ok` — emit sites `src/x.rs:"
                    + spec + "`).\n"
                    + DOC_TAIL
                )
                files = {"src/x.rs": "a\nb\nc\nd\ne\n"}
                failures, _ = self.run_check(doc, files)
                self.assertEqual(len(failures), 1)
                self.assertIn("unparsable emit-sites", failures[0])

    def test_normalized_unique_shorter_fragment_pins(self):
        """An ambiguous longest fragment must not hide a unique shorter one."""
        doc = (
            DOC_HEAD
            + "- Probe (`retry: probe alpha 3/30s ... bravo charlie 2/4`).\n"
            + DOC_TAIL
        )
        files = {
            "src/x.rs": "\n".join(
                [
                    '    "retry: probe alpha 1/30s".to_string(),',
                    '    "retry: probe alpha 2/30s".to_string(),',
                    '    "decision: bravo charlie 1/4".to_string(),',
                ]
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/x.rs:3", failures[0])
        self.assertIn("normalized", failures[0])

    def test_prose_test_backticks_not_a_citation(self):
        """Ordinary prose `test `...`` is not an unparsable citation."""
        doc = (
            DOC_HEAD
            + "- Note (run the test `debug_on_records` and see).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(failures, [])

    def test_word_ending_in_test_is_not_a_citation(self):
        """Prose ending in `test` (`latest`) does not steal a citation."""
        doc = (
            DOC_HEAD
            + "- Note (see the latest `src/x.rs:1`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {"src/x.rs": "x\n"})
        self.assertEqual(failures, [])


    def test_cite_range_cap(self):
        """A range wider than the cap does not parse."""
        self.assertIsNone(checker.parse_cite_lines("1-2000000"))
        self.assertIsNone(checker.parse_cite_lines("1-999999999"))
        self.assertIsNone(checker.parse_cite_lines("1-1001"))
        self.assertEqual(checker.parse_cite_lines("1-1000"), list(range(1, 1001)))
        self.assertEqual(checker.parse_cite_lines("10-12"), [10, 11, 12])

    def test_huge_range_citation_fails(self):
        """A citation with a huge range fails instead of expanding.

        Two failures, both true: the citation does not parse, and
        the example is pinned in the repo without a valid citation.
        """
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; test `src/x.rs:1-2000000`).\n"
            + DOC_TAIL
        )
        files = {"src/x.rs": '    "decision: alpha bravo".to_string(),\n'}
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 2)
        self.assertTrue(any("unparsable" in f for f in failures))
        self.assertTrue(any("no citation" in f and "src/x.rs:1" in f for f in failures))

    def test_regex_matched_unparsable_citation_fails(self):
        """A citation the regex matches but the parser rejects fails."""
        doc = (
            DOC_HEAD
            + "- Service (`service_state foo/bar: ok`; test `src/x.rs:10-`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_two_token_fragment_never_pins(self):
        """A fragment under three tokens is illustrative, never a pin."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET ...`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": '    "http: GET /source/foo -> 200".to_string(),\n'
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("illustrative" in line for line in summary))

    def test_three_token_fragment_pins(self):
        """The threshold is exactly three tokens — a 3-token fragment pins."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /foo ...`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": '    "http: GET /foo -> 200".to_string(),\n'
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/http.rs:1", failures[0])

    def test_normalized_two_token_fragment_never_pins(self):
        """A fragment with fewer than three normalized tokens never pins.

        Four raw tokens, but only two survive normalization (the path
        and the id are instance values), so the normalized search must
        not pin it.
        """
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /source/foo 1398977`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": '    "http: GET /zzz 42".to_string(),\n'
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("illustrative" in line for line in summary))


    def test_counter_normalized_pin(self):
        """A probe counter differing (3/30s vs 1/30s) is still a pin."""
        doc = (
            DOC_HEAD
            + "- Probe (`retry: create_package registration probe 3/30s: still not found`).\n"
            + DOC_TAIL
        )
        files = {
            "src/obs/remote.rs": "\n".join(
                ["", "", "", "",
                 '            ["retry: create_package registration probe 1/30s: still not found".to_string()]']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/obs/remote.rs:5", failures[0])
        self.assertIn("normalized", failures[0])

    def test_cited_counter_mismatch_fails(self):
        """A cited line pins its exact counters — attempt 3/4 is not attempt 2/4."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /source/foo -> 200 (attempt 3/4)`;\n"
            + "  test `src/http.rs:1`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": '    "http: GET /public/source/foo -> 200 (attempt 2/4)".to_string(),\n'
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("'3/4)'", failures[0])
        self.assertIn("src/http.rs:1", failures[0])

    def test_route_normalized_pin(self):
        """A route differing (/source vs /test) is still a pin."""
        doc = (
            DOC_HEAD
            + "- Endpoint (`decision: read via /source (authenticated)`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                ["", "",
                 '                .any(|l| l == "decision: read via /test (authenticated)"),']
            )
            + "\n"
        }
        failures, _ = self.run_check(doc, files)
        self.assertEqual(len(failures), 1)
        self.assertIn("no citation", failures[0])
        self.assertIn("src/http.rs:3", failures[0])
        self.assertIn("normalized", failures[0])

    def test_ambiguous_shape_stays_illustrative(self):
        """A generic 200 shape matching two tests is illustrative, not pinned."""
        doc = (
            DOC_HEAD
            + "- Attempt (`http: GET /source/foo -> 200 in 123ms (attempt 1/4)`).\n"
            + DOC_TAIL
        )
        files = {
            "src/http.rs": "\n".join(
                ["",
                 '                "http: GET /public/source/foo -> 200 (attempt 2/4)".to_string(),',
                 "",
                 '                "http: GET /test -> 200 (attempt 1/4)".to_string(),']
            )
            + "\n"
        }
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("ambiguous pin" in line for line in summary))

    def test_long_line_number_fails_not_crash(self):
        """A >4300-digit line number fails, never raises ValueError."""
        self.assertIsNone(checker.parse_cite_lines("9" * 5000))
        doc = (
            DOC_HEAD
            + "- Service (`service_state foo/bar: ok`; test `src/x.rs:"
            + "9" * 5000 + "`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_non_ascii_digit_citation_fails(self):
        """A non-ASCII decimal digit is not a line number."""
        self.assertIsNone(checker.parse_cite_lines("\u0663"))
        doc = (
            DOC_HEAD
            + "- Service (`service_state foo/bar: ok`; test `src/x.rs:\u0663`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_bare_path_prose_not_a_citation(self):
        """Prose `test `path`` with no line spec is not a citation."""
        doc = (
            DOC_HEAD
            + "- Note (run the test `src/http.rs` to see the output).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(failures, [])

    def test_plural_citation_fails(self):
        """`tests `path:LINE`` is a misspelled citation, not ignored."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; tests `src/x.rs:1`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_no_space_citation_fails(self):
        """`test`path:LINE`` (no space) is a misspelled citation."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; test`src/x.rs:1`).\n"
            + DOC_TAIL
        )
        failures, _ = self.run_check(doc, {})
        self.assertEqual(len(failures), 1)
        self.assertIn("unparsable", failures[0])

    def test_capital_test_citation_verifies(self):
        """A capital `Test` is a citation, not prose."""
        doc = (
            DOC_HEAD
            + "- Check (`decision: alpha bravo`; Test `src/x.rs:1`).\n"
            + DOC_TAIL
        )
        files = {"src/x.rs": '    "decision: alpha bravo".to_string(),\n'}
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("1 verified" in line for line in summary))


    def test_instance_only_fragment_is_illustrative(self):
        """A fragment of only instance values pins nothing."""
        doc = (
            DOC_HEAD
            + "- Route (`decision: ... /source/foo 1398977`; test `src/x.rs:1`).\n"
            + DOC_TAIL
        )
        files = {"src/x.rs": '    "decision: x".to_string(),\n'}
        failures, summary = self.run_check(doc, files)
        self.assertEqual(failures, [])
        self.assertTrue(any("no checkable tokens" in line for line in summary))

    def test_token_check_vacuous_returns_empty(self):
        """token_check reports a fragment with no checkable tokens."""
        self.assertEqual(
            checker.token_check("/foo 1398977", [(1, '"http: GET /zzz 42"')]), ""
        )
        self.assertIsNone(
            checker.token_check("http: GET", [(1, '"http: GET /zzz 42"')])
        )

    def test_missing_doc(self):
        with tempfile.TemporaryDirectory() as tmp:
            failures, _ = checker.check(Path(tmp) / "nope.md", Path(tmp))
            self.assertEqual(len(failures), 1)
            self.assertIn("not found", failures[0])


if __name__ == "__main__":
    unittest.main(verbosity=2)
