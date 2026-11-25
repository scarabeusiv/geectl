# Gitea flow: branch, build, merge, forward

Issue: https://github.com/scarabeusiv/geectl/issues/23 — the git vs OBS workflow as one flow.

## Factory

| geectl command | OBS flow | Gitea flow |
|---|---|---|
| `geectl --live branch` subcommand (planned; primitives: `gitea repo-fork`, `branch branch-package --add-repositories`) | Home test project created under `home:<user>:branches:<project>`; source project repositories copied in, Factory repo included; only the Factory build blocks | Repo forked; home package tracks the fork branch through scmsync |
| `git push` (local-build pre-push step planned, on by default) | scmsync syncs the pushed branch into the home package; OBS rebuilds | Branch head advances; PR diff updates |
| `geectl --live pull-request comment <reference>` (live build-info comment planned) | Build results read from the home test project | PR comment carries the build result, updated on each rebuild |
| `geectl --live pull-request merge <reference>` (forward after merge, PR #313; `--no-forward` skips it) | scmsync syncs the merged branch into the devel project package; forward SR filed from the devel package to its link parent; devel check (recorded project and package) runs first | PR merges; accepting the PR files the Factory submit automatically; a refused forward reports, the merge stands |
| `geectl --live request accept <id>` | Factory SR accepted; package lands in Factory | Home scmsync version deleted (open: automatic when the fork is pruned, or geectl deletes it on SR acceptance) |

## Maintenance

| geectl command | OBS flow | Gitea flow |
|---|---|---|
| `geectl --live maintenance maintenance-branch --package <package>`; `branch` subcommand for the git-backed setup (planned) | Package branched into its maintenance branch project; repositories from the maintenance branch codestream | Repo forked; home package tracks the maintenance branch through scmsync |
| `git push` (local-build pre-push step planned, on by default) | scmsync syncs the pushed branch into the home package; OBS rebuilds | Branch head advances; PR diff updates |
| `geectl --live pull-request comment <reference>` (live build-info comment planned) | Build results read from the home test project | PR comment carries the build result, updated on each rebuild |
| `geectl --live pull-request merge <reference>` (forward to the maintenance incident, PR #313; `--no-forward` skips it) | scmsync syncs the merged branch into the maintenance project package; maintained target files a maintenance incident | PR merges; accepting the PR files the incident automatically; a refused forward reports, the merge stands |
| `geectl --live request accept <id>` | Incident accepted; update released to `:Update` | Home scmsync version deleted (open: automatic when the fork is pruned, or geectl deletes it on SR acceptance) |

Leap:Update packages carry no `<devel>` record, so the devel check refuses the forward before the maintenance lookup (open).
