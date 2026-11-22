# geectl ← osc test borrow map

osc source: openSUSE/osc @ `89f6bdc`.
Geectl side: gap map (86 operations), implementation order 1 file upload →
2 request read/act → 3 remote read → 4 branch/copy/link → 5 build status →
6 working-copy layer → 7 maintenance requests → 8 services → 9 Gitea ops.

**Not parity**. Adopt the osc test cases that pin behaviour geectl is
building, as unit tests or as live tests; write the rest from
scratch. The point is not to miss cases osc covers.

## 0. What "their smoketest" is

There is **no file named "smoketest" anywhere in the osc repo**.
The smoketest is osc's **Behave suite**:

- `behave/features/*.feature` — **77 feature files, 272 scenarios**
  (scenarios start at column 0, `Scenario:`).
- Run by `.github/workflows/tests.yaml`, job `behave`: pulls
  `ghcr.io/suse-autobuild/obs-server:latest`, runs the real `osc` CLI
  against a live OBS server in a podman container. This is the direct
  analogue of geectl's live tests (`tests/live_obs.rs`).
- Fixture projects recur throughout: `test:factory/test-pkgA` (devel project
  `test:devel`), `test:factory/test-pkgB` (no devel project),
  `test:leap:15.6:update/test-pkgA`, `home:Admin`, repo `standard`,
  archs `x86_64`/`i586`, multibuild `multibuild-pkg` (`:flavor1`, `:flavor2`).
- Step definitions: `behave/features/steps/{common,osc,podman}.py`
  (865 lines). Suite entry: `behave/README.md`
  (build/pull/run a container, then `behave -Dosc=../osc-wrapper.py …`).

The skill's `contrib/harness/README.md` "Smoke test" section is unrelated
(it tests the pr-guard hook, not osc).

## 0b. The unit suite (for the unit-test half of the borrowing)

- `tests/test_*.py` — **58 files, 707 test methods** (`python3 -m unittest`).
- Harness: `tests/common.py` — `OscTestCase` plus `@GET/@POST/@PUT/@DELETE`
  decorators serving canned fixture files over a mocked HTTP layer
  (`http://localhost`). **No unit test in the surveyed set hits a live
  server** — they translate to geectl unit tests, never to live tests
  (except as behaviour specs to re-verify live).
- Fixture dirs sit next to the tests (`request_fixtures/`,
  `commit_fixtures/`, `results_fixtures/`, …).

## 0c. Survey boundary

- Surveyed in detail: **62 behave files / 216 scenarios** and
  **369 unit tests**.
- NOT surveyed in detail: `behave/features/git-*.feature` — 15 files /
  56 scenarios (Gitea/git-obs side; batch 9) — and
  338 unit tests in 32 files: config/credentials (`test_conf.py` 102,
  `test_config_parser.py`, `test_credentials.py`), Gitea/git-SCM
  (`test_gitea_api_*.py` 10 files, `test_maintainership.py` 3,
  `test_maintainership_converter.py` 7, `test_git_scm_store.py` 20,
  `test_gitattributes.py`, `test_gitignore.py`, `test_gitea_exceptions.py` 22,
  `test_git_editor.py`, `test_scmsync_obsinfo.py`), utilities
  (`test_helpers.py`, `test_grabber.py`, `test_keyinfo.py`,
  `test_util_ar.py`, `test_util_cpio.py`, `test_util_yaml.py` 8),
  `test_output.py` 28 (CLI formatting), `test__private_api.py`,
  `test__private_package.py`, `test_doc_plugins.py`, and the remaining
  47 of `test_commandline.py` (67 total, 20 in the slice below).
  None of these map to gap-map operations except as noted under batch 9.
- 216 + 56 = 272 scenarios ✓; 369 + 338 = 707 unit tests ✓.

---

## Batch 1 — file upload

**Direct adoptions: 0. Protocol spec: 1 (5 behaviours).**

osc has no dedicated test for the raw upload (`PUT /source/:prj/:pkg/:file`);
upload only happens inside commit flows. The adoptable material is the
commit protocol sequence in `tests/test_commit.py` (mocked HTTP + fixture
working copies) — WC-state assertions SKIP, the server exchange is the spec:

- `test_commit.py:37` `test_simple` — one modified file: GET file list →
  POST `commitfilelist` (validate, then final) → PUT the file → GET open
  requests; "Committed revision 2".
- `test_commit.py:190` `test_interrupt` — IOError mid-PUT aborts the commit;
  file stays Modified (atomicity edge).
- `test_commit.py:264` `test_expand` — commit of an expanded (linked)
  package passes `keeplink=1`.
- `test_commit.py:316` `test_commitfilelist_error` — server 500 on the final
  `commitfilelist` raises; file stays Modified.
- `test_commit.py:338` `test_simple_sha256` — same exchange with SHA256
  digests in the filelist.

**Gap (new from scratch):** no osc test pins the upload success body.
geectl's `parse_revision` (PUT returns `<revision rev="N" vrev="N"
srcmd5="…"/>`, not `<status code="ok"/>`) is verified only by geectl's own
live test `live_upload_file_reads_back_on_server`.
The remaining 11 `test_commit.py` tests are WC-state bookkeeping — SKIP.

## Batch 2 — request read/act

**Direct adoptions: 59 (54 unit + 5 behave). Live new-from-scratch: 8 ops.**

### Unit — adopt (mocked/in-memory, all directly portable)

`tests/test_request.py` (26 tests; adopt 22 — XML build/parse/round-trip):

- `:26` `test_createsr` — submit action serialises to exact expected XML;
  unset options are None.
- `:50` `test_createsr_with_option` — `opt_sourceupdate=cleanup`,
  `opt_updatelink=1` serialise inside `<options>`.
- `:79` `test_createsr_missing_tgt_package` — submit without target package
  is allowed; target element has project only.
- `:100` `test_createsr_invalid_argument`, `:105`
  `test_create_request_invalid_type` — unknown action kwarg/type raises
  `WrongArgs`.
- `:110` `test_create_add_role_person`, `:129`
  `test_create_add_role_group`, `:150` `test_create_add_role_person_group`
  — add_role serialises `<person>`/`<group>`/both.
- `:173` `test_create_set_bugowner_project`, `:191`
  `test_create_set_bugowner_package` — set_bugowner at project/package level.
- `:209` `test_create_delete_project`, `:225` `test_create_delete_package`
  — delete action, project vs package.
- `:241` `test_create_change_devel` — change_devel action, source + target.
- `:260` `test_action_from_xml1`, `:277` `test_action_from_xml2`, `:298`
  `test_action_from_xml3` — actions parse from XML and round-trip
  byte-identical (incl. `<acceptinfo rev/srcmd5/xsrcmd5>`; absent
  `osrcmd5/oxsrcmd5` are None).
- `:321` `test_action_from_xml_unknown_type` — unknown action type in XML
  raises `WrongArgs`.
- `:326` `test_read_request1` — full request XML (2 actions, state, history,
  title, description) parses and round-trips.
- `:357` `test_read_request2` — request in `review` state with a review
  entry; approver/who/when/comment fields.
- `:395` `test_read_request3` — empty `<comment>`/`<description>` parse as
  empty strings; serialised form drops the empty elements.
- `:531` `test_legacy_request` — old-style
  `<request type="submit"><submit>…` parses and re-serialises to modern
  `<action>` form (back-compat edge).
- `:566` `test_get_actions` — `get_actions()` filters by type(s).
- SKIP (osc display format, geectl has no equivalent output):
  `:431` `test_request_list_view1`, `:449` `test_request_list_view2`, `:464` `test_request_str1`, `:500`
  `test_request_str2`.

`tests/test_obs_api_request.py` (15 tests; adopt all — search `match=`
construction rules geectl needs for typed list/search ops; mocked HTTP):

- `:18` `test_search_default` — default matches states new/review/declined.
- `:22` `test_search_request_ids` — `request_ids` → `@id='1' or @id='2'`.
- `:26` `test_search_projects` / `:30` `test_search_packages` — match source
  OR target project/package.
- `:34` `test_search_source_projects` / `:38` `test_search_source_packages`
  — source only (note: builds `source[@packages=…]`, plural attribute —
  the server's form).
- `:42` `test_search_target_projects` / `:46` `test_search_target_packages`
  — target only (same plural form).
- `:50` `test_search_creators` — `@creator='foo' or @creator='bar'`.
- `:54` `test_search_who` — `state[@who]` OR `history[@who]`.
- `:58` `test_search_states` — `state[@name=…]`.
- `:62` `test_search_types` — `action[@type=…]`.
- `:66` `test_search_exclude_target_projects` —
  `target[not(@project=…)] and …`.
- `:70` `test_search_with_history` — adds `withhistory=1`.
- `:74` `test_search_with_full_history` — adds `withfullhistory=1`.

`tests/test_xpath.py` (15 tests; adopt all — the `match=` string forms):

- `:7` `test_noop`, `:11` `test_not`, `:15` `test_eq`, `:19` `test_not_eq`,
  `:23` `test_contains`, `:27` `test_and`, `:37` `test_or`, `:47`
  `test_and_or`, `:61` `test_or_and`, `:75` `test_and_or_and`, `:83`
  `test_or_and_or` (parenthesisation rules), `:91`
  `test_multiple_kwargs`, `:95` `test_eq_list`, `:99` `test_not_eq_list`,
  `:103` `test_review_state` (`state[@name="new"]`, nested predicate).

`tests/test_commandline.py` (2 tests; adopt — `change_request_state`
regression, osc#2130):

- `:838` `test_can_fail_suppresses_any_http_error` — `can_fail=True`
  swallows any HTTPError (403 tested), returns None, prints one warning.
- `:853` `test_without_can_fail_raises` — without `can_fail`, the HTTPError
  propagates.

### Behave — adopt as live precedent (partly covered)

- `behave/features/submitrequest.feature:14` — file an SR, exit 0
  (covered by geectl `live_filed_request_shows_up_on_server`).
- `behave/features/submitrequest.feature:25` — supersede flow: request 1
  ends `<state name="superseded" superseded_by="2">` (covered by
  geectl `live_refile_supersedes_and_deletes_on_server`).
- `behave/features/createrequest.feature:14` and `:25` — same two flows via
  the generic `createrequest -a submit` creator (the generic-create shape
  batch 7 reuses for other action types).
- `behave/features/request-list.feature:4` — `request list -P test:factory`
  exits 0 — no stdout/state check.

### Gap — NO osc coverage, live tests written from scratch

No scenarios/tests for `rq show`, `rq log`, accept, decline, revoke,
`review add`, comment list/create. Geectl's live tests for these
(`rq show` read + parse state/history/reviews; `rq log`; accept/decline/
revoke state transitions verified by re-read; review add; comment
list/create) have no osc precedent to borrow. The only state transition osc
pins anywhere is supersede (above).

SKIP in this batch: `tests/test_request_interactive_review.py` (14 tests)
— osc's interactive terminal review loop (keystrokes `a`/`d`/`bl`/`li`,
buildlog/rpmlint display, message-template editing); geectl has no
interactive loop. `tests/test_core_request.py` (4 tests) — Request-object
ordering by reqid; SKIP unless geectl sorts request lists by id.

## Batch 3 — remote read

**Direct adoptions: 53 (36 unit + 17 behave).**

### Unit — adopt

`tests/test_core.py` (23 of 26; adopt the parsing/URL rules):

- `TestSlashSplit` `:15`–`:40` — `project/package` splitting: no slash,
  leading/trailing slash, middle slash, `//` and `/////////` collapse
  (empty segments dropped).
- `TestParseRevisionOption` `:47`–`:84` — `""`/`":"` → `(None, None)`;
  `"1"` → `("1", None)`; `"1:2"` → `("1","2")`; 32-hex md5 → `(hash, None)`;
  `hash1:hash2`; `":::::"` and `"1:2:3:4:5"` and three hashes raise
  `OscInvalidRevision`.
- `TestMakeurl` `:90`–`:147` — path join + repeated query keys, `UrlQueryArray`
  (`k2[]=v2&k2[]=v3`), None/empty-list drops the query, int → `?int=1`,
  bool True→`1`/False→`0`, path percent-encoding table (`/` and `:` left
  unencoded), query percent-encoding table (space→`+`, `/`→`%2F`,
  `:`→`%3A`).
- SKIP: `TestBinaryFile` `:185`–`:203` (local file-type detection for
  diff/commit).

`tests/test_prdiff.py` (13 tests; adopt the substance — project-diff
classification + rdiff API params; exact CLI stdout text is osc-specific):

- `:77` `testPrdiffTooManyArgs` — 3 args → `WrongArgs`.
- `:85` `testPrdiffZeroArgs` — 0 args outside any WC → `WrongArgs`; inside
  branch dir → diffs branch vs upstream.
- `:108` `testPrdiffOneArg` — one arg (branch project) → full
  classification, `only-in-new` counts as identical.
- `:120` `testPrdiffTwoArgs` — `identical: common-one`, `differs:
  common-two`, `identical: common-three`.
- `:131` `testPrdiffOldOnly` / `:143` `testPrdiffNewOnly` — `--show-not-in-new`
  / `--show-not-in-old` flags.
- `:155` `testPrdiffDiffstat` / `:170` `testPrdiffUnified` — `--diffstat`
  and `--unified` output forms.
- `:194` `testPrdiffInclude` / `:206` `testPrdiffExclude` / `:216`
  `testPrdiffIncludeExclude` — include/exclude filtering.
- `:231` `testPrdiffRequestsMatching` — `--requests` annotates a differing
  package with the matching open submit request (id, state, by, description).
- `:255` `testPrdiffRequestsSwitched` — reversed direction finds no requests
  and flips the classification.

### Behave — adopt (live-server)

- `behave/features/list.feature:4` — list all projects (incl. `home:*`,
  `openSUSE.org`, `test:*`); `:21` — list packages of a project (multibuild
  `multibuild-pkg:flavor1/2` as separate entries); `:34` — list files of a
  package (`.changes`, `.spec`). API adoptable; exact stdout text is
  osc-specific.
- `behave/features/log.feature:4` — full history r1–r3, newest first,
  author/date/revision-hash/comment; `:25` — `--revision=2` returns only r2;
  `:38` — `--revision=1:2` returns r2 then r1; `:55` — `log --patch` adds
  per-file unified diffs, incl. "new file" rendering for r1.
- `behave/features/rdiff.feature:9` / `:14` / `:19` / `:24` — rdiff
  project/package vs project/package, vs project-only (same name implied),
  vs no target (defaults to devel/origin), `--change=1` single-revision
  diff. All assert exit 0 only; the semantics live in
  `test_prdiff.py` above.
- `behave/features/develproject-project-package.feature:9` — `develproject
  test:factory test-pkgA` prints `test:devel/test-pkgA`; `:17` / `:26` —
  `test-pkgB` (no devel project) exits 1, stderr "has no devel project"
  (two-arg and slash forms).
- `behave/features/showlinked-project-package.feature:10` / `:18` —
  `showlinked test:factory test-pkgA` prints `home:Admin/test-pkgA`
  (two-arg and slash forms).
- `behave/features/checkout.feature:69` — concept only: `--revision=2`
  files byte-identical to rev-2 fixtures ("fetch source at revision").

### Gap — NO osc coverage, written from scratch

- `osc cat` — no `cat.feature`, no unit test mentions it.
  The operation is used implicitly in fixture setups only.
- `meta pkg` / `meta prj` read — no `meta.feature`; only WC-local
  `get_meta_value` in `tests/test_store.py:214` (SKIP).
- `osc search` (package/project), owner search (`/search/owner`,
  `/search/package?match=person`, `/search/project?match=person`),
  `osc maintainer` list/add, `osc whois`, `updatepacmetafromspec`,
  `_builddepinfo`, binary download via
  `osc api /build/:prj/:repo/:arch/:pkg/:file.rpm` — no tests found in the
  surveyed files, except `osc maintainer` and `updatepacmetafromspec`,
  which appear only in the unsupported-git refusal scenario.
- SKIP in this batch: `tests/test_core_package.py` (14 tests, in-memory
  Package identity/ordering + `.osc` path resolution), `tests/test_models.py`
  (29 tests, osc's internal `BaseModel` framework — no OBS behaviour),
  develproject/showlinked/setdevelproject `-pkgcheckout`/`-prjcheckout`
  variants (working-copy inference; explicit forms above cover the server
  behaviour), `develproject-project.feature:9` (CLI arity error only).

## Batch 4 — branch/copy/link

**Direct adoptions: 51 (42 behave + 9 unit).**

### Behave — adopt as live tests

`behave/features/branch.feature` (8 — the richest set in the survey; all
verify the resulting `_link` via `api /source/.../_link`):

- `:5` — branch an inherited package with no devel project: link points at
  the inherited source (`test:leap:15.6`), not the `:update` project.
- `:13` — `--nodevelproject` on the same case — NOTE: the command in the
  file contains no `--nodevelproject` flag; it is a byte-duplicate of `:5`.
  The flag needs a real test.
- `:21` — `--new-package` keeps the `:update` project as link source.
- `:29` — branch a package with a devel project: link points at
  `test:devel`, not factory.
- `:37` — `--nodevelproject` overrides the devel project: link points at
  `test:factory`.
- `:45` — branch an inherited package that HAS a devel project: still
  resolves through the devel project.
- `:53` — `--nodevelproject` on the inherited package: link points at
  `test:factory`.
- `:61` — `--new-package` on a package that doesn't exist: succeeds, link
  to `test:factory` for the not-yet-existing package.

`behave/features/copypac.feature` (4):

- `:10` — copy to a target project (same package name); `:16` — copy with
  rename; `:22` — self-copy succeeds with `--keep-link --revision=1`;
  `:27` — same self-copy without flags exits 1 (error case).

`behave/features/linkpac.feature` (5 — all verify `_link`/`_meta` by re-read):

- `:10` — link created: `_link` `<link project="test:factory"
  package="test-pkgA">`; `:18` — `--disable-build` adds `<build><disable/>`
  to target `_meta`; `:28` — `--disable-publish` adds
  `<publish><disable/></publish>`; `:38` — link a locked package: succeeds,
  target `_meta` does NOT inherit `<lock><enable/></lock>`; `:47` — target
  project with a project-level link still takes the explicit package `_link`.

`behave/features/linktobranch.feature:11` and
`behave/features/detachbranch.feature:11` — convert/detach a link; both
assert exit 0 only, no state verification. Adopt the flows; geectl's
live tests assert the resulting link/branch state (stronger than osc).

`behave/features/setlinkrev-project-package.feature` (4) and
`behave/features/setlinkrev-project.feature` (3) — stdout-verified:

- pp `:10` — set link revision to latest (3); `:18` — slash form, same;
  `:26` — `--revision=2` pins rev 2; `:34` — `--unset` removes the revision.
- p `:10` — project-only sets its sole linked package's revision;
  `:18` — project-only `--revision=2` exits 2 (CLI rejects `--revision`
  without a package — adopt as a validation rule);
  `:23` — project-only `--unset`.

`behave/features/showlinked-project-package.feature:10` / `:18` — (counted
under batch 3 above; same scenarios serve the link-read side here.)

`behave/features/setdevelproject-project-package.feature` (4):

- `:10` — set devel project of `test:factory/test-pkgA` to `test:devel`;
  `:20` — cross-package mapping (`test-pkgB` → `test:devel/test-pkgA`);
  `:30` — slash forms both sides; `:40` — `--unset`.

`behave/features/develproject-project-package.feature:9` / `:17` / `:26`
— (counted under batch 3; same scenarios serve the devel-read side here.)

`behave/features/rdelete.feature:4` — `rdelete -r -f test:factory -m
'cleanup'` deletes a whole project; exit 0, no verification the project
is gone. Geectl's live tests verify by re-read.
`behave/features/undelete.feature:10` / `:17` — undelete package, then
undelete project (after `rdelete … -m 'why:delete'`).
`behave/features/aggregatepac.feature:10` / `:16` / `:21` — aggregate
same-name, with rename, same-source exits 1 (mirrors copypac `:27`).
`behave/features/release.feature:5` / `:13` — `release` on a project / on
a package (`--repo=standard --target-project=… --target-repository=…`);
assertions are stdout text only ("Releasing … options: delayed") — releases
are queued asynchronously by the server, so a live test can only assert
acceptance, not completion.
`behave/features/rmkpac.feature` (4):

- `:9` — `rmkpac . <package>` from a project checkout (creates package,
  verified via `api …/_meta`); `--scmsync https://mygitea1.com#main`
  writes `<scmsync>` into `_meta`; re-running with a different URL exits 1
  and leaves the first URL; `--force` overwrites it.
- `:35` — same via explicit `rmkpac <project> <package>`; empty package
  name exits 1.
- `:59` — non-existing project exits 1 even with `--force`.
- `:69` — `--scmsync` without a fragment exits 2, stderr "--scmsync must
  include a fragment with the branch name".

### Unit — adopt

`tests/test_setlinkrev.py` (9 tests; adopt all — mocked HTTP, `@GET _link`
+ source file list, `@PUT _link` with expected body):

- `:28` `test_simple1` — no revision: fetch source latest, PUT `_link`
  with `rev="42"`.
- `:35` `test_simple2` — explicit revision 42: PUT without fetching.
- `:43` `test_expandedsrc` — `expand=True` resolves the expanded source
  revision (md5), sets `rev` + `vrev="1"`.
- `:51` `test_existingrev` / `:61` `test_expandexistingrev` — link already
  has a rev: re-resolve latest and PUT the updated rev.
- `:67` `test_linkerror` — expanding a broken link: server 400, HTTPError
  propagates.
- `:76` `test_deleterev` / `:83` `test_deleterev_md5` — `revision=None`
  strips `rev` (and `vrev` when the rev was an md5).
- `:90` `test_deleterevnonexistent` — unsetting a rev that isn't there still
  PUTs the unchanged link (no-op PUT, not an error).

SKIP in this batch: `linktobranch`/`detachbranch` no-package and
working-copy variants (`:17`/`:23` in both files — CLI arity / WC
inference), `release` WC variants (`:20`/`:31`/`:41`), `showlinked` /
`setdevelproject` / `develproject` `-pkgcheckout`/`-prjcheckout` variants.

## Batch 5 — build status

**Direct adoptions: 46 (42 behave + 4 unit).**

### Behave — adopt as live tests

`behave/features/results.feature` (adopt 5 of 11 — the data, not the
formatting):

- `:14` — matrix for `multibuild-pkg` = base + flavor1 + flavor2 ×
  x86_64/i586, all `disabled`.
- `:43` — `--no-multibuild` collapses to base package only (2 rows).
- `:52` — `-M flavor1` filters to flavor1 only (2 rows).
- `:111` — `--xml` raw `<resultlist>` passthrough ("results XML returned
  verbatim").
- `:130` — `--xml -M flavor1` filtered XML.
- SKIP: `:4` (no-args outside a WC → exit 1, WC inference), `:27`
  (WC-inferred form of `:14`), `:63`/`:76`/`:89`/`:98` (custom `--format`
  / `--csv` text formatting — same data as `:14`).
- **Caveat: no scenario in this file shows a code other than `disabled`.**
  `failed` vs `unresolvable` vs `succeeded` distinctions live only in
  `test_results.py` fixtures (below).

`behave/features/buildinfo.feature:4` — `buildinfo` on a package whose spec
includes a `.inc` referencing a missing dep: exit 0, stdout contains
`<error>unresolvable: nothing provides DOES-NOT-EXIST</error>`. Adoptable in
substance (buildinfo surfaces unresolvable deps, incl. via `.inc`).

`behave/features/buildhist.feature:9` / `:14` — `buildhist
<project>/<package> <repo>/<arch>`, `--limit=1`; exit 0 only, no content
asserted. (Titles misspell it `buildist` in the source.) SKIP `:19`/`:26`/
`:31` (WC-inferred forms / WC errors).

`behave/features/checkconstraints.feature:9` — explicit args, exit 0 only.
SKIP `:14` (WC-inferred form).

`behave/features/getbinaries-*.feature` (30 scenarios; adopt the file-set
semantics, not the local `binaries/` layout) — the 30 collapse to 8 distinct
rules:

- Single named binary RPM (`…-file.feature:9`).
- Whole build output for one package: binary RPM + `_buildenv`,
  `_statistics`, `rpmlint.log` (`…-project-package-repo-arch.feature:9`).
- Multibuild flavor via `--multibuild-package=<flavor>` and via
  `pkg:flavor` syntax (both forms pinned in several files).
- `--source` adds the `.src.rpm` (`:42`, `:49`, `:53` variants; note the
  multibuild `.src.rpm` listed 3×, once per flavor — fixture quirk).
- `--debuginfo` adds `-debuginfo` (and `-debugsource` in the
  whole-project/tree forms, not in the single-package form).
- Non-RPM log file fetchable by name (`rpmlint.log`,
  `…-file.feature:65`).
- Whole-project form: 5 RPMs at top + per-package
  `_buildenv/_statistics/rpmlint.log` subdirs (tree form,
  `…-project-repo-arch.feature:9`).
- Repo-only form fetches both archs (`…-repo-pkgcheckout.feature:11`,
  `…-repo-prjcheckout.feature:11`).
- WC-inferred variants (`-pkgcheckout`/`-prjcheckout` files) pin the same
  file sets with project/package inferred — adopt the sets, not the inference.

`behave/features/repo.feature` (adopt 3 of 4):

- `:11` — `repo list` on a project: repo `standard`, archs `x86_64, i586`,
  path `openSUSE.org:openSUSE:Tumbleweed/standard`, build flag `disable`.
- `:24` — `repo add` adds `new-repo` (x86_64+aarch64, two paths), verified
  by re-listing.
- `:45` — `repo remove` removes `standard` (nonexistent repo name tolerated,
  not an error), verified by re-listing to empty.
- SKIP `:4` (`repo` with no arguments prints empty output — no WC context).

### Unit — adopt

`tests/test_results.py` (4 tests; adopt all — mocked HTTP, fixture dir
`results_fixtures/`):

- `:25` `testPrjresults` — `prjresults --xml` GETs
  `/build/testproject/_result`, stdout is fixture `result.xml` verbatim.
- `:31` `testPrjresultsWatch` — `--watch` first returns a dirty result,
  then polls with `?oldstate=<hash>` and returns the final result.
- `:36` `testResults` — `results --xml …` GETs `…/_result?package=…
  &multibuild=1&locallink=1`, stdout is fixture verbatim (multibuild
  expansion + local-link handling).
- `:42` `testResultsWatch` — `--watch` variant with `oldstate` polling.
- Adoptable: results XML passthrough + the `--watch`/`oldstate` polling loop.
  This is where result codes beyond `disabled` live — in fixtures.

### Gap — NO osc coverage, written from scratch

- `/build/…/_status` (404 = no log) and `/build/…/_jobhistory` (verify a
  green build is from the current srcmd5, not a stale rebuild): **zero**
  hits in `tests/` and `behave/features/`.
- `whatdependson` (reverse build deps) and `view=fileinfo_ext` (RPM
  requires inspection): no coverage found in the surveyed files.
- SKIP in this batch: `tests/test_build.py` (2 tests — local-build trust
  gate for untrusted projects; geectl does no local builds),
  `behave/features/build.feature` (3 scenarios — local buildroot/chroot;
  out of scope, issue #43).

## Batch 6 — working-copy layer

**Direct adoptions: 0. Spec reference: 36 tests.**

osc's WC tests are the only specification of the commit/update protocol;
whether they transfer depends on the API-only-vs-`.osc`-compatible decision,
which is undecided. Under API-only, only the protocol sequence transfers;
under `.osc`-compatible, the WC tests become directly relevant (large scope).

- `tests/test_commit.py` (16) — commit protocol (see batch 1): validate →
  final `commitfilelist`, per-file PUT, open-request check; refused on
  conflict (`:100`), refused with nothing to do (`:113`), partial commit
  by file list (`:165`), added-file-missing refusal (`:277`, `:296`),
  remote-exists idempotence (`:238`), expanded-package `keeplink=1`
  (`:264`), SHA256 digest variants (`:338`, `:363`).
- `tests/test_update.py` (20) — file-list diff semantics: no changes
  (`:24`), new file (`:33`), new-file/local collision → `PackageFileConflict`
  (`:42`), upstream delete (`:52`), upstream modify (`:65`), both-modified
  conflict (`:77`), already-in-conflict skip (`:92`), local deletions
  (`:107`), restore (`:126`, `:139`), size-limit skipping (`:150`, `:165`,
  `:185`), `_service:*` files downloaded only with `service_files=True`
  (`:213`) and ignored by default (`:229`), meta-mode update (`:244`),
  brand-new package (`:259`), resume after abort (`:275`, `:289`),
  tolerating `_in_update` disappearing (`:306`).
- State alphabet reference only: `tests/test_package_status.py` (8) pins
  the vocabulary (A/?/D/!/R/M/S/` `/C) at file granularity, and
  `tests/test_project_status.py` (17) the project-level subset
  (` `/!/?/A/D).
- SKIP as tests (all `.osc` store / WC bookkeeping, no geectl equivalent):
  `tests/test_store.py` (23), `tests/test_repairwc.py` (16),
  `tests/test_difffiles.py` (22 — unified-diff rendering),
  `tests/test_addfiles.py` (6), `tests/test_deletefiles.py` (16),
  `tests/test_revertfiles.py` (9), `tests/test_init_package.py` (6),
  `tests/test_init_project.py` (5), `tests/test_vc.py` (4 — `vc` env vars
  for hooks), `behave/features/checkout.feature` (10 — WC layout, file
  filters, `--output-dir`, `--current-dir`; only the revision-fetch concept
  transfers, batch 3), `behave/features/add.feature` (1),
  `behave/features/rm.feature` (1), `behave/features/mv.feature` (2),
  `behave/features/repairwc.feature` (4 — out of scope anyway).

## Batch 7 — maintenance requests

**Direct adoptions: 0 new (material already counted under batches 2/4).
Gap: mr / maintained / mbranch filing flows.**

- Counted under batches 2/4: `behave/features/createrequest.feature:14` / `:25`
  (generic creator — `maintenance_incident` is just another action type
  through it; batch 2) and `tests/test_request.py:209` / `:225` / `:241`
  (delete project/package and change_devel action XML; batch 2).
  No new unit adoptions here.
- Behave flows in the maintenance-writes family are counted under batch 4
  (`rdelete`, `undelete`, `aggregatepac`, `release`, `rmkpac`) — they are
  server-side package/project operations, implemented with that batch.

### Gap — NO osc coverage, written from scratch

- `osc mr` / `maintenancerequest` (maintenance_incident action): no test
  beyond the unsupported-git refusal scenario
  (`behave/features/osc-commands-unsupported-git.feature:162`, SKIP).
- `osc maintained` (which maintained products carry a package): only config
  attribute tests (`tests/test_conf.py:336` `test_maintained_attribute`,
  `:342` `test_maintained_update_project_attribute` — config defaults,
  not the operation).
- `osc mbranch`: no test found.
- `creq` / `deletereq` filing flows: action XML is covered
  (`test_request.py:209`/`:225`/`:241`); the file-and-watch flow is not.

## Batch 8 — services

**Direct adoptions: 1. Nearly everything is new-from-scratch.**

- `behave/features/service.feature:4` — the ONLY service test in osc:
  `service manualrun` with an invalid `_service`
  (`_service-set_version-invalid`) → stdout "Aborting: service call
  failed", exit 255. No valid-service case, no `runall` case.
- Adjacent (file handling, not service-run):
  `tests/test_update.py:213` / `:229` — `_service:*` files downloaded only
  with `service_files=True`, ignored by default.

### Gap — NO osc coverage, written from scratch

Valid service run, `source_validator` gate (gate 2), `precommit`,
`remoterun`, `run NAME` (manual-mode single service), `format_spec_file`,
`localrun`, `disabledrun`. No dedicated service-run unit tests exist
(grep for `service` over `tests/test_*.py` hits only `test_commit.py`,
`test_conf.py`, `test_deletefiles.py`, `test_update.py` — `_service` file
handling, not the run operation).

## Batch 9 — Gitea operations (deferred)

Not surveyed in detail; material exists for the batch:

- Behave: `behave/features/git-*.feature` — 15 files / 56 scenarios
  (`git-pr.feature` 15, `git-obs-staging-group.feature` 6,
  `git-pr-review-request.feature` 5, `git-ssh-key.feature` 5,
  `git-pr-new-package-request.feature` 4, `git-repo-list.feature` 4,
  `git-login.feature` 3, `git-repo-clone.feature` 3,
  `git-repo-fork.feature` 3, `git-obs-staging-remove.feature` 2,
  `git-pr-empty.feature` 2, `git-osc-precommit-hook.feature` 1,
  `git-pr-fork-to-fork.feature` 1, `git-repo-init-local.feature` 1,
  `git-repo-init-remote.feature` 1).
- Surveyed: `behave/features/fork.feature:11` / `:19` — Gitea fork
  incl. cross-org re-fork refused (exit 1) and `--new-repo-name` retry;
  `behave/features/user.feature:5` — `ls` under a newly registered user
  whose password contains unicode (🚀🚀🚀) — adoptable as an
  auth-with-unicode-password case.
- Unit (unsurveyed): `test_gitea_api_*.py` (10 files),
  `test_maintainership.py` (3 — `obs-maintainers` document parsing, v1 +
  legacy formats), `test_maintainership_converter.py` (7),
  `test_git_scm_store.py` (20), `test_gitattributes.py`,
  `test_gitignore.py`, `test_gitea_exceptions.py` (22),
  `test_git_editor.py`, `test_scmsync_obsinfo.py`.

---

## Operations with NO osc coverage at all (write from scratch)

Batch in brackets:

- `rq show`, `rq log`, accept, decline, revoke, `review add`, comment
  list/create [2] — only supersede is pinned anywhere.
- `osc cat`, `meta pkg`/`meta prj` read, `osc search`, owner search,
  `osc maintainer`, `osc whois`, `updatepacmetafromspec`, `_builddepinfo`,
  binary download via `osc api /build/:prj/:repo/:arch/:pkg/:file.rpm` [3].
- `/build/…/_status`, `/build/…/_jobhistory`, `whatdependson`,
  `view=fileinfo_ext` [5].
- `osc mr`, `osc maintained`, `osc mbranch`, `creq`/`deletereq` filing
  flows (action XML exists; flows don't) [7].
- Valid service run, `source_validator`, `precommit`, `remoterun`,
  `run NAME`, `format_spec_file`, `localrun`, `disabledrun` [8].
- Upload revision-response body (`<revision rev="N"/>` as PUT success) [1].

## Explicitly NOT adopting (and why)

- `.osc` store + WC bookkeeping: `test_store.py` (23), `test_repairwc.py`
  (16), `test_difffiles.py` (22), `test_package_status.py` (8),
  `test_project_status.py` (17), `test_addfiles.py` (6),
  `test_deletefiles.py` (16), `test_revertfiles.py` (9),
  `test_init_package.py` (6), `test_init_project.py` (5), `test_vc.py` (4);
  Behave checkout/add/rm/mv/repairwc scenarios — geectl has no working copy
  (batch 6 decision undecided; see above).
- CLI text/arity/formatting: `test_request.py` display tests (`:431`,
  `:449`, `:464`, `:500`), `test_commandline.py` arg-parsing class (18 of the
  slice — `TestPopProjectPackageRepositoryArchFromArgs` or
  `TestPopProjectPackageTargetProjectTargetPackageFromArgs`), `test_output.py` (28), results `--format`/`--csv` variants,
  list/log stdout text, `addchannels`/`addcontainers`/`enablechannels`
  Behave scenarios (24 total — assert stdout strings only, no server-side
  effect verified).
- Interactive loop: `test_request_interactive_review.py` (14).
- osc-internal frameworks: `test_models.py` (29), `test_models_xmlmodel.py`
  (10), `test_core_package.py` (14), `test_core_request.py` ordering (4).
- Local builds: `test_build.py` (2), `behave/features/build.feature` (3) —
  out of scope (issue #43).
- Git working-copy mode: `behave/features/osc-commands-unsupported-git.feature`
  (32 scenarios) — geectl has no osc-git hybrid to guard.
- Not adopted (coverage exists, adoption refused):
  `osc token --create` — `behave/features/token.feature:12` covers
  create → list → trigger end-to-end, but agents must never handle the
  secret. Secret printers (`config --dump-full`, `config pass`) — no tests
  found; not adopted regardless.

## Counts (direct adoptions, de-duplicated)

| Batch | Unit | Behave | Total |
|---|---|---|---|
| 1 file upload | 0 (+1 protocol spec) | 0 | 0 (+spec) |
| 2 request read/act | 54 | 5 | 59 |
| 3 remote read | 36 | 17 | 53 |
| 4 branch/copy/link | 9 | 42 | 51 |
| 5 build status | 4 | 42 | 46 |
| 6 working-copy layer | 0 (+36 spec ref) | 0 | 0 (+spec) |
| 7 maintenance requests | 0 (counted in 2/4) | 0 (counted in 2/4) | 0 (+gaps) |
| 8 services | 0 | 1 | 1 |
| 9 Gitea ops | deferred | deferred | deferred |
| **Total adopted** | **103** | **107** | **210** |

Surveyed base: 216 behave scenarios + 369 unit tests = 585 items;
adoptable as written: 210 of them (107 behave scenarios, 103 unit tests),
plus the batch 1/6 protocol specs. The rest is SKIP (osc-internal),
stdout-only, or covered by geectl's existing live tests.

## Quirks in osc's own tests

- `behave/features/branch.feature:13` — title claims `--nodevelproject`
  but the command omits the flag (byte-duplicate of `:5`).
- `behave/features/submitrequest.feature:25` and
  `createrequest.feature:25` — create `another_file` but `add new_file`.
- `behave/features/buildhist.feature` — scenario titles say `buildist`.
- `behave/features/getbinaries-repo-pkgcheckout.feature` — title says
  "project checkout", Background is a package checkout.
- `behave/features/rmkpac.feature:59` — title has a stray trailing `'`.
- `tests/test_obs_api_request.py:38` — `source_packages` builds
  `source[@packages=…]` (plural attribute); a server-side quirk.
- `tests/test_prdiff.py:108` — `only-in-new` counts as identical in the
  one-arg form.
- `behave/features/getbinaries-*-project*` — multibuild `.src.rpm` listed
  3×, once per flavor (fixture quirk).
