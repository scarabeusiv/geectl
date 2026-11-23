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

- Surveyed in detail: **all 77 behave files / 272 scenarios** and
  **all 707 unit tests** (the 15 `git-*.feature` files and the 338 unsurveyed unit tests).
- Nothing unsurveyed remains: 272 scenarios and 707 unit tests,
  all surveyed in detail.

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
  exits 0 — no stdout/state check (covered by geectl `request list`, the
  thin wrapper over the search operation; `request_list_*` binary tests
  and `live_request_list_defaults_cover_the_filed_request`).

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

### Remote download — `get_source_file` protocol (GAP 3)

**Direct adoptions: 0 tests. Protocol spec: 1.**

osc has no dedicated test for a single-file source download; the
protocol comes from the implementation, mined at openSUSE/osc @
`a83810c` (`osc/core.py:2820` `get_source_file`,
`osc/core.py:2791` `download`, `osc/commandline.py:10379`
`do_cat`, `osc/commandline.py:5689` checkout single-file path):

- URL shape: `GET /source/<prj>/<pkg>/<filename>?rev=<revision>`
  (`get_source_file`).
- `rev` passes verbatim — a revision number or a srcmd5 (`do_cat`
  passes `opts.revision` verbatim as `rev`; `get_source_file`
  passes `revision` verbatim).
- Streamed to disk in chunks, not buffered (`download` streams via
  `streamfile` in `BUFSIZE` chunks).

Deliberate divergences, not misses:

- osc writes to a temp file and renames (atomic); geectl streams to
  the destination with exclusive create — a failure mid-stream
  leaves a partial file for the caller to delete (the #111
  transport rule, shared with `download_binary_to_file`).
- osc resolves the rev through the expanded upstream srcmd5 by
  default (`show_upstream_srcmd5` + `expand=1`); geectl passes the
  rev verbatim, like `cat` (no link expansion).
- osc sets the downloaded file's mtime; geectl does not (the
  transport leaves the file as created).
- Not adopted: the `getbinaries-*.feature` scenarios (built-binary
  downloads, a different endpoint) — already counted under batch 5.

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

## Batch 5b — build-log selectors

**Direct adoptions: 0. Source pins: 3.**

No osc test pins the `rbl`/`buildlog` behaviour being built: `behave/features/`
has no `buildlog`/`rbl` scenario, and `tests/` has no `parse_buildlogurl`
or strip-time test — only `tests/test_conf.py:211`
(`test_buildlog_strip_time`, the `buildlog_strip_time = 0` config
default), which is SKIP: geectl has no such config; `--strip-time`
is a per-invocation flag. Adopted as source pins instead (behaviour
specs, not test cases):

- `osc/core.py:105` `BUILDLOGURL_RE` — the API-form URL shape
  (`https?://<host>/build/<project>/<repository>/<arch>/<package>/_log`),
  adopted verbatim for geectl's URL form.
- `osc/core.py:4722` `buildlog_strip_time` — the `^\[[^\]]*\] ` strip
  rule, adopted as a manual line-prefix strip (no regex dependency).
- `osc/commandline.py:6807-6808` (`do_buildlog`) — `-M FLAVOR`
  appends `:FLAVOR` to the package, adopted for `--multibuild`.

The web-UI URL form (`build.opensuse.org/package/live_build_log/…`)
has no osc equivalent (`parse_buildlogurl` only takes the API form);
it is geectl's own addition, written from scratch and pinned by
geectl unit tests.

## Batch 5c — single-review state change

**Direct adoptions: 0. Source pins: 1.**

No osc test pins the `changereviewstate` behaviour: `tests/` has no
review state-change test — only `tests/test_obs_api_request.py`
(search `match=` construction, already adopted under batch 2) and
`tests/test_request.py` (request read/act, batch 2). Adopted as a
source pin instead (behaviour spec, not a test case):

- `osc/core.py:2289` `change_review_state` — the query construction
  (`cmd=changereviewstate`, `newstate`, `by_user`/`by_group`/
  `by_project`/`by_package`, comment as the POST body), already
  adopted verbatim in the library (`ObsClient::change_review_state`);
  the CLI leaf mirrors it with the state as a positional and the
  target as flags. osc's per-state subcommands (`review accept` /
  `decline` / `reopen`) collapse into the one leaf.

## Batch 6 — working-copy layer

**Direct adoptions: 6 (5 commit-protocol cases + 1 status-vocabulary
reference). Live new-from-scratch: 2 tests.**

Adopted in this batch, each pinned by a geectl test:

- `tests/test_commit.py:37` `test_simple` — validate → final
  `commitfilelist`, per-file staged PUT, open-request check, revision 2:
  `working_copy::tests::commit_obs_sends_the_index_state` + the live
  `live_working_copy_checkout_edit_commit_update`.
- `tests/test_commit.py:113` — refuse with nothing to do:
  `working_copy::tests::commit_obs_refuses_nothing_to_commit`.
- `tests/test_commit.py:264` `test_expand` — expanded-package `keeplink=1`:
  `obs::remote::tests::commit_filelist_keeplink_passes_the_flag`.
- `tests/test_commit.py:338` / `:363` — SHA256 digest variants:
  `obs::remote::tests::commit_filelist_sha256_challenge_names_the_files`.
- `tests/test_package_status.py` (8) — the status vocabulary (A/?/D/!/R/M/S/` `/C):
  `working_copy::tests::status_maps_porcelain_codes`.

Not adopted as tests in this batch (implemented, no pinning test):
refuse on conflict (`test_commit.py:100`), partial commit by file list
(`:165`), added-file-missing refusal (`:277`, `:296`), remote-exists
idempotence (`:238`), IOError mid-PUT abort (`:190`), server 500 on the
final `commitfilelist` (`:316`). `tests/test_update.py` (20) transfers
as file-list diff semantics through the rebase mechanics, pinned by
`working_copy::tests::update_obs_rebases_local_commits_onto_the_new_base`
and the live update test — no individual case adopted one-to-one;
size-limit skipping does not transfer (no size refusal), and the resume
cases become "an interrupted update leaves a valid old base".

osc's WC tests are the only specification of the commit/update protocol;
whether they transfer depends on the API-only-vs-`.osc`-compatible decision.
Under API-only, only the protocol sequence transfers;
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
- `service trigger` (issue #182): no osc test pins `remoterun` — the
  only service test is the manualrun case above — so the trigger is
  written from scratch. The endpoint is mined from osc's code, not
  its tests (`osc/commandline.py` `do_service` and `osc/core.py`
  `runservice`: `POST /source/<project>/<package>?cmd=runservice`,
  `<status code="ok"/>` on success), and the server behaviour from
  the OBS backend (`triggerservicerun` re-runs the package's whole
  `_service`; no per-service parameter exists anywhere in the
  trigger path, so `--service` validates the name against the
  stored file and fails closed instead of narrowing the run).
  No count change: 0 new adoptions.

### Gap — NO osc coverage, written from scratch

Valid service run, `source_validator` gate (gate 2), `precommit`,
`run NAME` (manual-mode single service), `format_spec_file`,
`localrun`, `disabledrun`. No dedicated service-run unit tests exist
(grep for `service` over `tests/test_*.py` hits only `test_commit.py`,
`test_conf.py`, `test_deletefiles.py`, `test_update.py` — `_service` file
handling, not the run operation).

Comment handling in `_service` files is also new-from-scratch
(#218): no osc test or fixture carries a comment in a `_service`
file (no `<!--` in any behave feature or unit test, no
`changesgenerate` anywhere in the suite), and osc reads `_service`
XML through ElementTree (`xml_fromstring`), which skips comments
natively — there is no adoptable comment case. Counts unchanged.

## Batch 9 — Gitea operations

**Direct adoptions: 15 (2 unit + 13 behave). Surveyed in full
(sweep 2): all 15 `git-*.feature` files (56 scenarios) and all 338
unsurveyed unit tests.**

### Behave — adopt

- `git-pr.feature` (15; surveyed in the #121–#141 sweep): 6 read
  scenarios adopted in #140, 1 read scenario adopted in sweep 2
  (`:170` "Get a pull request that doesn't exist" →
  `pr_get_missing_pr_is_not_found`, a 404 mapping to `NotFound`),
  8 checkout/rebase/create scenarios out of scope for the read
  surface (working-copy ops / `pr create`, not implemented).
- `git-pr-review-request.feature` (5): 5 adopted in #154 (step 4).
- `git-repo-fork.feature` (3): 1 adopted in sweep 2 (`:72`
  fork-of-fork → `fork_repo_fork_of_a_fork_verifies_the_immediate_parent`,
  the re-read parent is the fork it was made from, not the root);
  `:9` basic fork covered by `fork_repo_creates_and_verifies_by_re_read`;
  `:31` fork-twice-under-a-different-name NOT adopted — geectl refuses
  `AlreadyExists` where osc reuses the existing fork with a warning
  (deliberate divergence; the fork-discovery-by-source the reuse needs
  is a `repo_list` use case, recorded under gaps).
- `git-repo-list.feature` (4): covered in substance — org/user/both
  forms, user-first-then-org order, same-account dedupe, and the
  JSON/text agreement on repo objects (`repo_list_*`,
  `repository_renders_parent_when_a_fork`,
  `repositories_render_a_table_without_clone_urls`).
- `behave/features/fork.feature:11` / `:19` — Gitea fork incl.
  cross-org re-fork refused (exit 1) and `--new-repo-name` retry
  (covered by `fork_repo_refuses_a_fork_of_another_source` and the
  reuse tests).
- `behave/features/user.feature:5` — `ls` under a newly registered user
  whose password contains unicode (🚀🚀🚀) — adoptable as an
  auth-with-unicode-password case.

### Unit — adopt

- `test_gitea_api_pr.py::test_object_pull_request` — head/base commit
  SHAs → `pr_get_pins_head_and_base_commit_shas` (sweep 2). The rest
  of the case is not modelled: `url`, `allow_maintainer_edit`, and the
  head/base `ssh_url` are deliberately unparsed (a field the operation
  does not need is not parsed); head/base repo owner+name are not
  modelled (full name only).
- `tests/test__private_api.py::TestXmlEscape` (5 cases) →
  `xml_escape_escapes_the_five_xml_specials` (sweep 2).
- `test_maintainership.py` (3) →
  `parse_maintainership_v1_reads_project_and_packages`,
  `parse_maintainership_legacy_splits_at_prefixed_groups`,
  `parse_maintainership_empty_document_has_no_maintainers`
  (#207, the git maintainership read). An unknown package/user
  is `NotFound` where osc raises `ValueError`. The `null`-list
  case comes from
  `test_maintainership_converter.py::test_v1_format_passthrough`;
  the converter's other 6 cases pin the migrate/write command,
  which #207 does not build.
- Covered in substance, no new test: `test_gitea_api_repo.py`
  (`repo_get_parses_a_repo_and_ignores_unknown_fields`,
  `repo_get_parses_a_fork_parent`); `test_gitea_api_branch.py`
  (`branch_get_parses_a_branch`,
  `branch_without_protected_fails_to_parse`); `test_gitea_api_conf.py`
  (config-path rules — the absolute/default/empty cases match the
  `resolve_config_path` tests; relative and tilde diverge
  deliberately, geectl follows the XDG spec; the explicit-path case
  (`test_explicit_path_takes_precedence`, an explicit `Config(path)`
  wins over XDG) has no `resolve_config_path` equivalent — geectl
  loads an explicit path through `load_config_at`, a different
  entry point);
  `test_gitea_exceptions.py` (22 × 301 → `redirect_is_not_followed`
  at the transport layer; geectl maps 3xx to `ServerError` naming the
  Location, not a distinct `MovedPermanently` type).
- Modelled differently, not adopted: `test_object_issue`
  (issue-shaped PR object — geectl models search items as `GiteaIssue`,
  not as a PR with empty endpoints); `test_gitea_api_user.py`
  (`full_name`/`email` — `GiteaOwner` models the login only).

### Gap — no geectl operation yet, cases recorded for the batch that builds it

- Staging group/remove (`git-obs-staging-group.feature` 6,
  `git-obs-staging-remove.feature` 2): needs `pr create` + label
  add/remove + close + PR-reference parse/add/remove
  (`test_gitea_api_pr.py` reference cases are the spec for the
  helpers: parse, add-after-last-ref, remove).
- SSH keys (`git-ssh-key.feature` 5, `test_gitea_api_ssh_key.py` 1):
  no `ssh-key` operations.
- PR-URL parsing (`test_gitea_api_pr.py`
  `test_get_host_owner_repo_number_*`, 5): geectl handles are
  `owner/repo#n` and there is no PR-URL parser; the only URL parser
  is `parse_scmsync_url` (`src/gitea.rs:3221`). The five cases
  (https with/without port, issues endpoint, invalid endpoint,
  invalid format) are the spec for one.
- Fork discovery by source: finding an existing fork of a source repo
  needs a `repo_list`-by-source use case; no such operation (this is
  what the fork-twice reuse in `git-repo-fork.feature:31` needs).
- New-package request (`git-pr-new-package-request.feature` 4): no
  issue-create/label operations.
- Login management (`git-login.feature` 3): no config-entry
  add/remove/update operations (credentials come from env/file).
- Repo clone (`git-repo-clone.feature` 3), repo init
  (`git-repo-init-local.feature` 1, `git-repo-init-remote.feature` 1),
  precommit hook (`git-osc-precommit-hook.feature` 1), pr
  checkout/rebase (`git-pr.feature` lines 190, 217, 231, 259):
  working-copy layer (#17, blocked on Gitea ops).
- PR create (`git-pr.feature` create scenarios, `git-pr-empty.feature`
  2, `git-pr-fork-to-fork.feature` 1): not implemented; the empty-PR
  rule ("Source and target are identical" refused) and the
  fork-to-fork target form are validation rules for the create op.
  `Branch.split_id` (`test_gitea_api_branch.py`) is the target-spec
  parser for it.
- Git helpers for the working-copy layer: `Git.urlparse`/`urljoin`/
  `detect_git` (`test_gitea_api_git.py`), `GitDiffGenerator`
  (`test_gitea_api_git_diff_generator.py`, incl. the `.gitmodules`
  rendering), `TarDiff` (`test_gitea_api_tardiff.py`),
  `test_git_scm_store.py` (20), `.gitattributes`/`.gitignore`
  parsing and merging (`test_gitattributes.py` 7,
  `test_gitignore.py` 7), `ScmsyncObsinfo`
  (`test_scmsync_obsinfo.py` 3).
- No equivalent, surveyed and closed: `test_conf.py` (102, oscrc
  attributes geectl does not have), `test_config_parser.py` (6, INI
  format preservation — geectl config is TOML and never written),
  `test_credentials.py` (1, obfuscated passwords — geectl never
  stores passwords), `test_helpers.py` (2, bytes→str coercion —
  strings are UTF-8 by construction), `test_grabber.py` (1,
  mirror-group download — the transport covers downloads),
  `test_gitea_api_common.py` (3, `dt_sanitize` — no datetime
  normalisation in geectl — and `TemporaryDirectory` delete/no-delete
  — a test helper only),
  `test_keyinfo.py` (2, no keyinfo operation), `test_util_ar.py` (6),
  `test_util_cpio.py` (4), `test_util_yaml.py` (8, no YAML anywhere),
  `test__private_package.py` (7, osc-internal Package framework),
  `test_doc_plugins.py` (5, no plugins), `test_git_editor.py` (6,
  interactive editor), `test_output.py` (28, CLI formatting — SKIP),
  `test_commandline.py` remaining 47 (CLI arity / WC inference —
  SKIP; the `can_fail` pair is counted under batch 2).

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
- Valid service run, `source_validator`, `precommit`,
  `run NAME`, `format_spec_file`, `localrun`, `disabledrun` [8].
- Upload revision-response body (`<revision rev="N"/>` as PUT success) [1].
- `remote delete-file`: no osc command deletes one server file (a
  working copy can delete the file and commit it);
  `test_deletefiles.py` pins working-copy delete semantics only
  (see "Explicitly NOT adopting").

## Explicitly NOT adopting (and why)

- `.osc` store + WC bookkeeping: `test_store.py` (23), `test_repairwc.py`
  (16), `test_difffiles.py` (22), `test_package_status.py` (8),
  `test_project_status.py` (17), `test_addfiles.py` (6),
  `test_deletefiles.py` (16), `test_revertfiles.py` (9),
  `test_init_package.py` (6), `test_init_project.py` (5), `test_vc.py` (4);
  Behave checkout/add/rm/mv/repairwc scenarios — they pin the `.osc`
  layout and osc's CLI text, not the git working copy.
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

## Batch 10 — local-build flag parity

**Direct adoptions: 0. Spec reference: `osc/build.py` (the flag
semantics, not its tests).**

- `tests/test_build.py` (2 tests — filename-glob only, nothing about
  `--alternative-project`, `--root`, `--noinit`, `-k`/`-p`): nothing
  to adopt.
- `behave/features/build.feature` (3 scenarios — local buildroot/chroot,
  out of scope for v1 by design): nothing to adopt.

## Gap batch 2, wave 3 — package search by person / by devel project

**Direct adoptions: 0. No osc test covers either form.**

- `#175` (`remote search-packages-by-person`): the person-match
  form (`person/@userid='…'`) is the same rule osc's `do_search`
  builds for project and package search alike
  (`osc/commandline.py:9288`, already adopted for
  `search-projects-by-person` under batch 3); no osc test pins it
  for packages — the map already records
  `/search/package?match=person` as a gap — so the new tests are
  written from scratch against the mocked HTTP shape.
- `#176` (`remote search-packages --devel-project`): osc's search
  has no devel-project option at all. The predicate
  (`devel/@project='…'`) follows the package-meta shape — the
  devel element lives in the meta (`osc/core.py:1402`
  `show_devel_project`) — and the OBS search API evaluates the
  match over complete package meta files (`/package[<match>]`).
  With a term, the substring match is combined with the devel
  predicate (parenthesised: XPath `and` binds tighter than `or`).

## Gap batch 3 — buildconfig --alternative-project (#252)

**Direct adoptions: 0. Spec reference: `osc/build.py:987-988`,
`osc/core.py:4813-4818`.**

- No osc test pins the buildconfig operation at all: no behave
  scenario (`behave/features/` carries no buildconfig feature) and no
  unit test (`get_buildconfig` is only exercised inside build flows;
  `tests/test_build.py` covers the trust gate only). `osc buildconfig`
  itself (`osc/commandline.py:7272`) takes no `--alternative-project`
  — the flag exists only on `osc build` (`osc/commandline.py:7144`,
  `:7756`).
- The wire shape is the spec: the alternative project replaces the
  project in the path (`osc/build.py:987-988`: `prj =
  opts.alternative_project`, then `get_buildconfig(apiurl, prj,
  repo)`), and the endpoint takes no alternative-project query
  parameter (`osc/core.py:4813-4818`: only the kiwi `path` query).
  That is the same substitution geectl already does for buildinfo
  (#213) and for local builds (`fetch_project()`).
- The new tests are written from scratch against the mocked HTTP
  shape: the CLI stub-server test pins the request target, the unit
  tests pin the payload and the header.
## Gap batch 3 — Repository shape in the meta write (#157 OCR follow-ups)

**Direct adoptions: 0. No reference test pins any of the nine
behaviours; the fixes are pinned by the source model instead.**

- The reference `repo add` flow (`osc/commands/repo_add.py`) PUTs
  the whole project meta back without validating the repositories
  already in it: it loads the meta, refuses only a duplicate name for
  the repository being added, appends it, and calls `to_api`. That is
  why a meta write validates only the repositories it changes.
- The reference download model
  (`osc/obs_api/repository_download.py`) holds `archfilter`/`master`/
  `pubkey` as singular optionals, and its releasetarget model
  (`osc/obs_api/repository_releasetarget.py`) types `repository` as
  required while `trigger` is optional — the shape the two parsers
  and the validator follow.
- The server `DownloadRepository` model (open-build-service
  `src/api/app/models/download_repository.rb`) validates download
  arch presence + case-insensitive uniqueness, url scheme, and
  repotype inclusion, with no format validation on
  sslfingerprint/archfilter/pubkey.
- No full-shape write scenario exists in the reference suite: the
  only behave scenario that PUTs project meta
  (`behave/features/linkpac.feature:48`) carries no `<repository>`
  at all, and no scenario mentions `download` or `releasetarget`;
  the `repo add` scenario (`behave/features/repo.feature:24`)
  covers arch + path only.

Counts unchanged: 0 new adoptions. Live issue #40 counts 125 behave /
116 unit / 241 total (verified live 2026-11-23); the map's Counts
table in this tree still records #40 as 231 (125 + 106) — that gap
predates this PR (the #217/#232/#207 adoptions are recorded in #40
but not in this tree's map) and is not changed by it.

## Batch 11 — prjconf

**Direct adoptions: 0. osc has no prjconf tests.**

- A code search for `prjconf` over osc hits only `osc/core.py` and
  `contrib/osc.complete` (no test file, no test case). The prjconf
  grammar, the effective-config evaluation and the safe cleanup are
  written from scratch (51 unit tests, 6 CLI tests, 2 live tests).

## Batch 12 — buildinfo bdep projects

**Direct adoptions: 0. Spec reference: `osc/build.py` (the flag
semantics and the project-set enumeration, not its tests).**

- `behave/features/buildinfo.feature:4` — `osc buildinfo` exits 0 and
  prints the raw `<error>unresolvable: nothing provides
  DOES-NOT-EXIST</error>` XML: stdout-only raw XML, and the error case
  is already pinned by geectl's `buildinfo_error_and_clean` in its
  typed form — nothing to adopt.
- `tests/test_build.py` (2 tests — `test_name`, `test_glob`) — they
  pin the `trusted_prj` config globs; geectl has no trusted-project
  config, and batch 10 already surveyed them as nothing to adopt.
- `osc/build.py:1287-1289` (`if not opts.trust_all_projects:` around
  the `check_trusted_projects` call) is the spec reference for the
  flag skipping the trust check; `osc/build.py:351-370` (the buildinfo
  parse collecting `self.projects[p.project]`) is the spec reference
  for enumerating the dep set's source projects. The flag help
  (`osc/commandline.py:7788`, "trust packages from all projects") is
  the wording geectl's flag help follows.
## Gap batch — periodic cache cleanup (#191)

**Direct adoptions: 0. No osc test covers the prune pass or the
scheduler entries.**

- `tests/test_build.py` (2 tests — `test_name`, `test_glob`,
  filename-glob only): nothing to adopt; osc has no prune,
  eviction, or scheduling concept anywhere in its suite (no
  `prune`/`evict`/`systemd`/`launchd`/`schtasks`/`periodic` in
  `osc/*.py`, `tests/*.py`, or `behave/features/build.feature`).
- The prune library (`prune_plan`/`prune_apply`) already exists with
  its own tests, written from the design doc; this batch adds the
  `prune` CLI leaf and the periodic install/uninstall/status leaves,
  all tested from scratch against the design doc. The unit files
  under `packaging/` are the single source of truth for the RPM
  and the install command alike.

## Batch 13 — mirror-first dep fetch

**Direct adoptions: 1 unit test.**

- `tests/test_grabber.py::test_invalid_scheme` — a `container://`
  mirror URL must not fail the fetch →
  `download_dep_ignores_a_non_http_mirror_url` (mirror URL skipped,
  API only). The grabber's other behaviour (mirrors tried in order,
  any failure falling through) is the mirror-first order itself,
  pinned by geectl's own fallback tests, not adopted case by case.
- `osc/build.py:375-382` (path-url to download-URL construction) and
  `osc/grabber.py:36-51` (mirror-group fallback) are the spec reference
  for the URL shape and the any-failure-falls-back rule, not adopted
  tests.
- The "1 unit test" above counts direct adoptions only, per the
  table's wording: geectl's own mirror tests (21 in
  `local_build.rs`, plus the buildinfo-parse and transport tests
  that pin the mirror rules) are not adopted osc tests and are not
  counted here.

## Gap batch 3 — local-build log (#178)

**Direct adoptions: 0.**

- `osc localbuildlog` (`osc/commandline.py:6897`, alias `lbl`):
  no osc test covers it — no matches in `tests/` or `behave/` —
  so there is nothing to adopt. The operation reads
  `<buildroot>/.build.log`, failing when the file is missing; that
  read is already pinned by geectl's own library tests
  (`build_log_full_read_is_capped`,
  `build_log_uncapped_read_returns_the_whole_log`, and the
  `build_log_for_source_*` tests in this PR). Its `--offset`
  (byte offset) and `--strip-time` options are surveyed, not
  adopted: the library reads by lines — `--tail N` for the last
  lines, `--full` for the uncapped whole log — under the 4 MiB cap
  the design doc pins, and no osc test pins either option on the
  local log.
- `tests/test_build.py` (2 tests) and `behave/features/build.feature`
  (3 scenarios) were already surveyed under batch 10: nothing to
  adopt.

## Counts (direct adoptions, de-duplicated)

| Batch | Unit | Behave | Total |
|---|---|---|---|
| 1 file upload | 0 (+1 protocol spec) | 0 | 0 (+spec) |
| 2 request read/act | 54 | 5 | 59 |
| 3 remote read | 36 | 17 | 53 (+1 protocol spec) |
| 4 branch/copy/link | 9 | 42 | 51 |
| 5 build status | 4 | 42 | 46 |
| 5b build-log selectors (#206) | 0 (+3 source pins) | 0 | 0 (+pins) |
| 5c single-review state change (#209) | 0 (+1 source pin) | 0 | 0 (+pin) |
| 6 working-copy layer | 0 (+36 spec ref) | 0 | 0 (+spec) |
| 7 maintenance requests | 0 (counted in 2/4) | 0 (counted in 2/4) | 0 (+gaps) |
| 8 services | 0 | 1 | 1 |
| 9 Gitea ops | 2 | 13 | 15 |
| 10 local-build flag parity | 0 (+spec ref) | 0 | 0 (+spec) |
| Gap batch 2, wave 3 (#175, #176) | 0 | 0 | 0 |
| Gap batch 3 — buildconfig --alternative-project (#252) | 0 (+spec ref) | 0 | 0 (+spec) |
| Gap batch 3 — local-build log (#178) | 0 | 0 | 0 |
| Gap batch 3 — Repository shape in the meta write (#157 OCR follow-ups) | 0 (+source pins) | 0 | 0 (+pins) |
| 11 prjconf | 0 | 0 | 0 |
| 12 buildinfo bdep projects (#194) | 0 (+spec ref) | 0 | 0 (+spec) |
| 13 mirror-first dep fetch | 1 | 0 | 1 |
| Gap batch — periodic cache cleanup (#191) | 0 | 0 | 0 |
| **Total adopted** | **106** | **120** | **226** |

Surveyed base: 272 behave scenarios + 707 unit tests = 979 items;
adoptable as written: 226 of them (120 behave scenarios, 106 unit tests),
plus the batch 1/6 protocol specs. The rest is SKIP (osc-internal),
stdout-only, covered by geectl's existing tests, or a gap for a batch
that has not landed. The table sums to 226; issue #40 counts 241
(125 behave + 116 unit) — the 15-count gap (5 behave + 10 unit)
predates this batch and is unresolved: 6 of the unit gap is #217's
working-copy adoptions, recorded in the issue but not in this table.

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
