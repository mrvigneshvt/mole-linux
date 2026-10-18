# Linux cleanup opportunities and upstream safety baseline

Research date: 2026-10-19. Upstream snapshot: `tw93/Mole` commit `8c55e8c07f3b34eeeb0352753b660f823468cd43`. Source inspection only; no cleanup commands or upstream tests executed. This report covers selected safety, purge and uninstall paths, not an independent claim that every upstream file was read.

## What upstream already does

Treat these as requirements to preserve, not differentiators to advertise:

- Central deletion funnel: `mole_delete` validates targets, supports permanent/Trash modes, records policy refusals, refuses privileged operations below user-mutable ancestors, distinguishes unknown size from zero, propagates cancellation and checks selected-path identities. See [file_ops.sh lines 2367 onward](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/lib/core/file_ops.sh#L2367).
- Purge does not equate a directory name with disposable content. `purge_artifact_has_authored_content` resolves physical ancestry and checks nested Git metadata, deployment keypairs and tracked files. Probe failure is unknown, not permission to delete. See [project.sh lines 501–527](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/lib/clean/project.sh#L501).
- Worktree removal is explicitly rejected as an inferred safe action; only allowlisted rebuildable artifacts inside worktrees qualify. Discovery and final deletion recheck protection. See [purge rules](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/.claude/rules/purge-installer.md).
- Uninstall validates identifiers before constructing patterns and protects shared roots. `remove_file_list` rechecks whether the application reappeared or a sibling installation still owns its data; an incomplete inventory retains that data. See [app_protection.sh](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/lib/core/app_protection.sh#L1134) and [batch.sh](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/lib/uninstall/batch.sh#L692).
- Safety tests explicitly cover shared XDG roots, Steam libraries, user source/credentials, mutable configured roots and replacement targets. I read representative test bodies including [shared-root/Steam tests](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/tests/uninstall_safety.bats#L29) and [root replacement test](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/tests/purge.bats#L2585). Their presence is source evidence, not proof that the suite passes today.

Upstream's [agent contract](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/AGENTS.md) already requires dry runs, operation history, bounded scans, meaningful refusal reasons, preservation of live databases and protection of developer state. Rust alone does not improve any of these contracts.

## Recommended product advantage

Build a Linux-native cleanup planner that explains ownership, consequence and recoverability before action. Compete on trustworthy decisions and Linux ecosystem integration; do not claim universal superiority over the macOS tool. The proposals below are not claims of missing upstream functionality across every branch/product.

| Priority | Proposed feature | Concrete user benefit | Boundary / evidence |
|---|---|---|---|
| First | Explain every candidate: owner, rule, size confidence, recreate command, privilege, recovery method | User can choose a rebuildable cache without confusing it with application data | XDG distinguishes cache from configuration, data, state and runtime; location alone still does not prove every file disposable. [XDG specification](https://specifications.freedesktop.org/basedir/0.8/) |
| First | Arch package-cache preview with retained-version policy | Reclaim old archives while retaining downgrade options | Delegate selection to installed `paccache`, which has dry-run and keep policies; default retention is three versions. Never directly delete package-manager databases. Missing helper means unsupported, not automatic dependency installation. [paccache manual](https://man.archlinux.org/man/paccache.8) |
| Second | Per-filesystem storage explanation | Explain why removing large paths may free little space | Distinguish logical bytes, allocated bytes, hardlink deduplication, shared extents and observed free-space delta. Btrfs reports total, exclusive and set-shared bytes; shared totals require interpretation. Do not promise exact reclaimable bytes or infer snapshot deletability. [Btrfs filesystem manual](https://btrfs.readthedocs.io/en/latest/btrfs-filesystem.html#subcommand) |
| Second | Flatpak cleanup by explicit installation and ref | Remove unused runtime refs through their owner without deleting application state | Flatpak supports user/system/named installations and unused-ref removal; `--delete-data` separately removes app data and permissions. Never conflate those actions or silently add `--delete-data`. [Flatpak command reference](https://docs.flatpak.org/en/latest/flatpak-command-reference.html#flatpak-uninstall) |
| Second | Journal retention preview and bounded owner-native cleanup | Reduce log history with an explicit diagnostic retention tradeoff | `journalctl` vacuum acts on archived files, whereas disk usage includes active files. Explain the difference; no raw journal deletion. [journalctl manual](https://www.freedesktop.org/software/systemd/man/latest/journalctl.html) |
| Later | Developer storage map across project artifacts and container build caches | See where developer space lives without destroying environments | Container volume data is not a cache. Docker documents separate image/container/build-cache/volume pruning semantics; keep volumes out of default cleanup and avoid broad `system prune` as the normal action. Rootless/remote contexts need explicit scope. [Docker pruning guide](https://docs.docker.com/engine/manage-resources/pruning/) |

## Additional ideas worth validating

1. **Cleanup benefit versus rebuild cost:** show whether an action needs downloads, recompilation or a running owner's cooperation. Begin with qualitative labels and documented recovery commands; do not invent time estimates. This is a proposal, not a measured competitive advantage.
2. **Space discrepancy diagnosis:** read-only explanation for large deleted-but-open files, inode pressure, Trash and snapshot-held extents. Make diagnosis distinct from deleting data or restarting a process. Kernel/proc and filesystem details require a separate design review before implementation.
3. **Reviewable cleanup sessions:** retain a local machine-readable receipt of planned/completed/skipped actions with reason codes and before/after filesystem readings. Upstream already has operation logging; the proposed extension is a consistent plan-to-result model across Linux owners, not logging itself. Never replay an old plan without revalidation.
4. **Capability report:** show supported adapters and why others are unavailable. Probe installed tools and versions; do not equate the Linux distribution name with installed capabilities. Start with Arch/Omarchy rather than claiming all distributions.

## Recovery tradeoff to make visible

Moving files to same-filesystem Trash retains their storage. A reversible action and immediate disk reclamation are different promises. Cache regeneration, file restore and filesystem snapshots are different recovery mechanisms; package-manager/container operations may not support general undo. Do not advertise a universal Undo button. Snapshot creation itself can preserve extents and cost space; snapshot management should remain read-only until separately designed.

## Suggested first milestone

Read-only CLI inventory for explicit user-selected roots plus an Arch package-cache adapter. Emit a human summary and a stable structured plan from the same model. Include retained/unknown entries and explain reasons. No uninstall, root mode, browser session cleanup, live database mutation, snapshot deletion or plugin execution in this milestone.

Acceptance targets for later approved implementation:

- Custom XDG locations are honored; configuration, state and credentials are never classified as caches just because they are large.
- Permission failures and timeouts remain unknown and visible; partial inventory never becomes a complete deletion plan.
- Candidate overlap/hardlinks do not inflate totals; unsupported extent accounting is explicitly unknown.
- Path replacement, symlink swaps, mount crossings, control characters and cancellation have adversarial fixture tests before enabling mutation.
- Any future owner-native action is revalidated and scoped at execution; native command availability is not equivalent to safe preview support.

## Verification boundary

Research uses pinned upstream source and live primary Linux documentation. No comparative performance benchmarks, usability studies, real-device cleanup or restoration checks were run. Superiority remains a product hypothesis until measured on representative Linux fixtures and user machines. No runtime or implementation change was made by this research.
