# geectl ← osc test borrow map

osc source: openSUSE/osc @ `89f6bdc`.

**Scope: adopt the pinning cases**. Adopt the osc test cases that pin behaviour geectl is building, as unit tests or as live tests; write the rest from scratch. The point is to cover every case osc covers.

Surveyed: osc's Behave suite (77 feature files, 272 scenarios) and unit suite (58 files, 707 test methods) — 979 items. Adopted as written: 227 (120 behave scenarios, 107 unit tests), plus protocol specs. The rest is SKIP (osc-internal), stdout-only, covered by geectl's existing tests, or a gap with no implementation yet.

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
| 11 list-scmsync-projects | 0 | 0 | 0 |
| Gap batch 2, wave 3 (#175, #176) | 0 | 0 | 0 |
| Gap batch 3 — buildconfig --alternative-project (#252) | 0 (+spec ref) | 0 | 0 (+spec) |
| Gap batch 3 — local-build log (#178) | 0 | 0 | 0 |
| Gap batch 3 — Repository shape in the meta write (#157 OCR follow-ups) | 0 (+source pins) | 0 | 0 (+pins) |
| Gap batch 3 — project-meta full-shape write test (#266) | 0 | 0 | 0 |
| Gap batch — releasetarget repository required (#265) | 0 | 0 | 0 |
| 11 prjconf | 0 | 0 | 0 |
| 12 buildinfo bdep projects (#194) | 0 (+spec ref) | 0 | 0 (+spec) |
| 13 mirror-first dep fetch | 1 | 0 | 1 |
| Gap batch — periodic cache cleanup (#191) | 0 | 0 | 0 |
| Unified person search (#300) | 1 | 0 | 1 |
| Gap batch — --from-file on set-meta (#180) | 0 (+source pins) | 0 | 0 (+pins) |
| **Total adopted** | **107** | **120** | **227** |

## Operations written from scratch

- `rq show`, `rq log`, accept, decline, revoke, `review add`, comment
  list/create — only supersede is pinned anywhere.
- `osc cat`, `meta pkg`/`meta prj` read, `osc search`, owner search
  (`remote owners`, `owners-for-binary`, `owners-for-package`),
  `osc maintainer`, `osc whois`, `updatepacmetafromspec`, `_builddepinfo`,
  binary download via `osc api /build/:prj/:repo/:arch/:pkg/:file.rpm`.
- `/build/…/_status`, `/build/…/_jobhistory`, `whatdependson`,
  `view=fileinfo_ext`.
- `osc mr`, `osc maintained`, `osc mbranch`, `creq`/`deletereq` filing
  flows (action XML exists).
- Valid service run, `source_validator`, `precommit`,
  `run NAME`, `format_spec_file`, `localrun`, `disabledrun`.
- Upload revision-response body (`<revision rev="N"/>` as PUT success).
- `remote delete-file`: a working copy deletes the file and commits it; `test_deletefiles.py` pins working-copy delete semantics only (see "Adoption exclusions").
- `remote list-scmsync-projects`: the scmsync-prefix project list. No osc test pins it; the count's own endpoint and validation are the spec reference.

## Adoption exclusions (and why)

- `.osc` store + WC bookkeeping: `test_store.py` (23), `test_repairwc.py`
  (16), `test_difffiles.py` (22), `test_package_status.py` (8),
  `test_project_status.py` (17), `test_addfiles.py` (6),
  `test_deletefiles.py` (16), `test_revertfiles.py` (9),
  `test_init_package.py` (6), `test_init_project.py` (5), `test_vc.py` (4);
  Behave checkout/add/rm/mv/repairwc scenarios — they pin the `.osc` layout and osc's CLI text.
- CLI text/arity/formatting: `test_request.py` display tests, `test_commandline.py` arg-parsing class (18 of the
  slice — `TestPopProjectPackageRepositoryArchFromArgs` or
  `TestPopProjectPackageTargetProjectTargetPackageFromArgs`), `test_output.py` (28), results `--format`/`--csv` variants,
  list/log stdout text, `addchannels`/`addcontainers`/`enablechannels`
  Behave scenarios (24 total — assert stdout strings only).
- Interactive loop: `test_request_interactive_review.py` (14).
- osc-internal frameworks: `test_models.py` (29), `test_models_xmlmodel.py`
  (10), `test_core_package.py` (14), `test_core_request.py` ordering (4).
- Local builds: `test_build.py` (2), `behave/features/build.feature` (3) —
  excluded (issue #43).
- Git working-copy mode: `behave/features/osc-commands-unsupported-git.feature`
  (32 scenarios) — the osc-git hybrid is excluded from geectl's scope.
- Adoption refused (coverage exists):
  `osc token --create` — `behave/features/token.feature` covers
  create → list → trigger end-to-end, but the secret stays with the user. Secret printers (`config --dump-full`, `config pass`) are excluded: the secret stays with the user.
