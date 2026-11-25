# Gitea flow: branch, build, merge, forward

Issue: https://github.com/scarabeusiv/geectl/issues/23 — the git vs OBS workflow as one flow.

## Factory

| geectl command | OBS flow | Gitea flow |
|---|---|---|
| `geectl --live branch branch` | Home test project created under `home:<user>:branches:<project>`; source project repositories copied in, Factory repo included; only the Factory build blocks | Repo forked; home package tracks the fork branch through scmsync |
| `git push` (local-build pre-push step planned, on by default) | scmsync syncs the pushed branch into the home package; OBS rebuilds | Branch head advances; PR diff updates |
| `geectl --live pull-request comment <reference>` (live build-info comment planned) | Build results read from the home test project | PR comment carries the build result, updated on each rebuild |
| `geectl --live pull-request merge <reference>` (forwards after merge; `--no-forward` skips it) | scmsync syncs the merged branch into the devel project package; forward SR filed from the devel package to its link parent; devel check (recorded project and package) runs first | PR merges; accepting the PR files the Factory submit automatically; a refused forward reports, the merge stands |
| `geectl --live request accept <id>` | Factory SR accepted; package lands in Factory | Home scmsync version deleted (open: automatic when the fork is pruned, or geectl deletes it on SR acceptance) |

## Maintenance

| geectl command | OBS flow | Gitea flow |
|---|---|---|
| `geectl --live maintenance maintenance-branch --package <package>`; `branch` subcommand for the git-backed setup (planned) | Package branched into its maintenance branch project; repositories from the maintenance branch codestream | Repo forked; home package tracks the maintenance branch through scmsync |
| `git push` (local-build pre-push step planned, on by default) | scmsync syncs the pushed branch into the home package; OBS rebuilds | Branch head advances; PR diff updates |
| `geectl --live pull-request comment <reference>` (live build-info comment planned) | Build results read from the home test project | PR comment carries the build result, updated on each rebuild |
| `geectl --live pull-request merge <reference>` (forwards to the maintenance incident; `--no-forward` skips it) | scmsync syncs the merged branch into the maintenance project package; maintained target files a maintenance incident | PR merges; accepting the PR files the incident automatically; a refused forward reports, the merge stands |
| `geectl --live request accept <id>` | Incident accepted; update released to `:Update` | Home scmsync version deleted (open: automatic when the fork is pruned, or geectl deletes it on SR acceptance) |

## Forward

Forwarding files the same submit request to the next project in the chain: from the devel package to the link parent of each submit action's target package. The hop resolves from the target package's `<linkinfo>`; an action whose target has no link is skipped and reported, and a broken link refuses the forward. A maintained hop target files a maintenance incident instead of a submit request. The forwarded description is `Forwarded request #{id} from {creator}` plus the original description, and the `.changes` entry is the original description. A re-forward supersedes an earlier still-open forward for the same hop.

Forwarding runs coupled to the step that lands the request: `request accept` files it after accepting, and `pull-request merge` files it after merging. `--no-forward` skips it on both.

The source must be the package's devel project and package as OBS records them in the link parent's package meta: a different recorded devel, or none, refuses the forward. A link-parent package that does not exist yet has no record to verify the source against, so the forward is refused fail-closed.

A refused forward is reported and the work stays queued in the devel project; the accepted request merges and the merge stands.

On a Gitea pull request, the forward is validated before merging, then the merge lands and the forward files afterwards. A preflight failure — a missing OBS credential, an ambiguous or unforwardable package, a refused hop — fails with the pull request still open. A refusal after the merge reports the refusal and changes nothing: the merge already landed, and there is no request to decline on that path.

Leap:Update packages carry no `<devel>` record, so the devel check refuses the forward before the maintenance lookup (open).
