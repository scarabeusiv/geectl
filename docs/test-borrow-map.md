# geectl ← osc test borrow map

osc source: openSUSE/osc @ `89f6bdc`.

**Scope: adopt the pinning cases**. Adopt the osc test cases that pin behaviour geectl is building, as unit tests or as live tests; write the rest from scratch. The point is to cover every in-scope case osc covers.

Surveyed: osc's Behave suite (77 feature files, 272 scenarios) and unit suite (58 files, 707 test methods) — 979 items. Adopted as written: 227 (120 behave scenarios, 107 unit tests), plus protocol specs. The rest is SKIP (osc-internal), stdout-only, covered by geectl's existing tests, or a gap with no implementation yet.

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
- `remote list-scmsync-projects`: the scmsync-prefix project list. No osc test pins it; the command's own endpoint and validation are the spec reference.
- `meta set` (unified meta write): OBS project/package meta and Gitea repo settings. No osc test pins writing project or package meta, and osc's Gitea `Repo` has no edit method; the OBS arm follows the `set-project-meta` / `set-package-meta` behaviour, and the Gitea endpoint is the spec for the client method.

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
