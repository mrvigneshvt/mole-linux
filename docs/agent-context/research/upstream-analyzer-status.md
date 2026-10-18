# Upstream disk analyzer and status research

Source freeze: `tw93/Mole` commit `8c55e8c07f3b34eeeb0352753b660f823468cd43`, inspected 2026-10-19 from `/tmp/mole-upstream-review-20261019`. Research only; no upstream cleanup, Trash operation, authorization prompt, or live macOS validation was executed.

## Scope and evidence boundary

Read upstream `AGENTS.md` and `.claude/rules/go-tuis.md`; no `.codegraph/` exists in the upstream checkout. Enumerated every file in `cmd/analyze/`, `cmd/status/`, and `internal/units/` (28,068 lines including tests). Inspected architecture, production entrypoints, destructive paths, scanner and cache internals, output schemas, platform probes, and representative actual test bodies. Some rendering/helpers and large test files received symbol/targeted-section inspection rather than a line-by-line audit. This is subsystem context research, not a claim that every line or runtime behavior has been verified.

## Existing analyze flow

`main.go` parses `--json` and an optional path. `MO_ANALYZE_PATH` takes precedence over the positional path. No path means an overview anchored at `/`; an explicit `/` means directory traversal, not the overview. `newModel` initializes Bubble Tea state and free-space statistics. The overview separates Home, User Library, Applications, System Library, and selected hidden-space insights. `update.go` owns the event loop, scan request identities, navigation, refresh, filtering, selection and confirmation. `live_scan.go` paints immediate children, publishes progressive per-child sizes, then produces a final sorted result. `scanner.go` recursively measures ordinary directories and folds dense developer/cache directories into bounded `du` measurements. `cache.go` provides history snapshots, disk caches and overview totals.

Sources: [main.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/main.go), [update.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/update.go), [live_scan.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/live_scan.go), [scanner.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/scanner.go).

- TUI top list is capped at 30 directory entries and 20 large files; the directory JSON scan explicitly requests all immediate entries. Nested large-file discovery still uses bounded top-N heaps. Sorting breaks size ties by path.
- Navigation supports back/history, refresh, top-files view, text filtering against names/paths, multi-selection, opening/revealing, preview, and live-sort modes. Selection/deletion is disabled while scanning; unknown-size entries cannot be selected/deleted, and overview rows are not directly deleted.
- Scan states are `complete`, `partial`, and `unavailable`. A complete state describes the chosen scan surface, not an atomic snapshot of a mutating filesystem. Partial sizes remain visibly partial; percentages requiring complete totals are suppressed.
- JSON includes root and entry `scan_status`, paths, names, size, directory/insight/cleanable flags, optional last access and large-file results. Existing JSON and progressive scanning are parity features, not novel Linux proposals.

Sources: [model.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/model.go), [json.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/json.go), [constants.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/constants.go), [view.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/view.go).

## Sizing and scan limits

- Traversal avoids following symlink targets for size: link entries contribute link size. It uses device+inode identities to count hardlinks once across its shared scan limiter.
- APFS clone handling is already present: `getattrlist` reads volume-local clone IDs and `EF_SHARES_ALL_BLOCKS`; clones known to share all blocks count once. Edited clones count in full because the OS interface does not report exactly how many blocks remain shared. A clone/hardlink-deduplicated subtree is not reused as an independent authoritative cached total.
- `getActualFileSize` uses `min(stat.Blocks * 512, logical length)` when stat metadata exists, preserving sparse/cloud-file behavior. This is an attributed size, not proof that deleting that path immediately releases those bytes. Dense directory `du` paths have their own accounting boundaries.
- Worker limits are separate: entry traversal at most 12; `du` processes at most `min(4, CPUs)`; queue admission twice that; fallback walker its own bounded pool. Overview concurrency is 8. `du` gets 30 seconds; Spotlight gets 5 seconds; fast fallback walking has a five-minute timeout.
- BSD `/usr/bin/du -skPx` is intentional, with `-I` ignore basenames. Home excludes its immediate `Library` separately; User Library uses one serial `du` traversal so cross-child hardlinks count once. Timeout remains an unavailable/partial measurement, not an invented total. `Mobile Documents` is excluded from overview via ignore rules but can be explicitly explored.
- Spotlight `mdfind` supplements large-file discovery; code extensions and folded tool directories are excluded from that list. Cancellation and permission failures remain distinct: a useful partial permission-only measurement may be stored; transient failures must not become authoritative cached results.

Sources: [scanner.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/scanner.go), [clone_darwin.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/clone_darwin.go), [clone_test.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/clone_test.go), [scanner_test.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/scanner_test.go).

## Caching and snapshots

Cache schema version 9 records sizing changes. Analyzer data lives under `~/.cache/mole/analyzer`, separate from shell-owned state. Paths are hashed with xxhash into gob entries. Entries expire after seven days; stale first-paint reuse has a three-day window and requires a background refresh. A directory mtime newer than the stored mtime immediately rejects an authoritative entry. This does not prove arbitrary descendant file-content writes are detected; root-directory mtime is not a recursive filesystem change journal.

Subdirectory cache admission requires 100 files or 10 MiB. Pruning caps analyzer cache at 5,000 entries/50 MiB and removes expired entries and abandoned temp files. Overview measurements use a separate JSON store capped at 1,000 entries with eviction back to 900. Saves stage through temp files and rename. Cache publication is guarded by request cancellation and identity; delete/refresh invalidation cancels relevant publications and invalidates related ancestor/overview totals instead of permitting a canceled scan to resurrect them.

`snapshots.go` is a read-only Time Machine count probe (`tmutil listlocalsnapshotdates /`, 3-second timeout). It parses only valid timestamp rows and explicitly does not invent retained snapshot bytes. UI tells the user snapshot-only space is absent from the listed tree. These are not user-facing scan-history comparison snapshots or a rollback implementation.

Sources: [cache.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/cache.go), [snapshots.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/snapshots.go).

## Analyze deletion safety

Confirmation collects paths, not mutable list indexes. `delete.go` validates raw absolute paths before normalization, rejects NUL and `..` components, checks protected paths and resolved symlink targets, then moves a selected path as one unit to Trash. No recursive precount delays the move. Batch deletion sorts descendants before parents and aggregates failures.

Trash route: absolute `/usr/bin/trash` with timeout → filesystem move into the correct home/per-volume Trash → Finder AppleScript compatibility fallback. The filesystem route validates directory type, rejects symlink Trash directories, checks ownership/writability, and uses `renameatx_np(RENAME_EXCL)` to prevent overwriting a colliding Trash name. The target has critical-root, protected-system-tree, home-root, Docker Desktop, OrbStack, EDR/MDM-cache and symlink-alias protection. These are macOS rules, not a Linux protection policy.

`oplog.go` adds per-session operation rows and deletion TSV rows with escaped controls, fixed failure reasons, known selected sizes, and a visible warning on logging failure. The selected sizes are prior measurements, not a before/after free-space receipt. The path is revalidated before the move, but this reviewed flow does not bind the approved item to an expected inode identity; do not confuse path validation with race-proof execution.

Sources: [delete.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/delete.go), [oplog.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/oplog.go), [delete_test.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/delete_test.go).

## Cleanable hints and insights

`cleanable.go` already recognizes the standard `CACHEDIR.TAG` signature (regular tag file, no symlink tag), and known project dependency/build directory basenames. It excludes paths delegated to `mo clean`. These are manual explorer hints, not proof that a directory with a familiar name contains no authored data. A Linux design should require stronger project/provider evidence for destructive candidate classification.

Insights include iOS backups, old downloads, logs, Homebrew/Xcode/Spotify/JetBrains/pip/uv/Gradle/CocoaPods and container data. Their existence is not permission to delete all of them: Docker and OrbStack live data is explicitly protected. Old Downloads uses 90-day modification age, not reliable last-use evidence; old directory mtime is not proof every descendant is old. Some insight rows are drill-down pointers into bytes also covered by broader overview categories; do not interpret their summed total as exclusive whole-filesystem usage.

Sources: [cleanable.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/cleanable.go), [cleanable_test.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/cleanable_test.go), [insights.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/insights.go).

## Status flow and contracts

`cmd/status/main.go` chooses interactive Bubble Tea or one-shot JSON; piped stdout automatically selects JSON. `--watch --interval DURATION` streams NDJSON from one warm collector. This preserves previous counters for network/disk rates instead of restarting each sample. Startup emits a fast snapshot, then immediate enrichment; subsequent ticks wait the requested interval after collection finishes. Full enrichment attempts occur every 30 seconds and process collection every second. Failed attempts consume their interval, avoiding a tight full-probe retry loop. TUI animation runs separately from collection and narrow layouts reduce displayed CPU cores to fit.

`MetricsSnapshot` includes CPU/load/per-core data, memory/swap/pressure, disks/SMART/purgeable bytes, disk I/O, network/history/proxy, hardware, battery/thermal/GPU/Bluetooth, health score, top processes, persistent high-CPU alerts, zombie counts and parent attribution. Process output carries its own collection timestamp and stale flag; cached process data does not pretend to be fresh. Process watch identity combines PID, parent PID and command; alert threshold/window are configurable (defaults 100% for five minutes). It surfaces alerts inside output; it is not a daemon and does not kill processes.

One-shot JSON writes collection warnings to stderr but preserves a successful exit if core metrics remain; no usable CPU/memory/disk/process sample fails. Optional/unavailable metrics do not all have a uniformly typed unavailable state: several numeric values remain zero when a probe cannot report them. A future Linux schema should model unknown explicitly from the outset.

Sources: [main.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/main.go), [collection.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/collection.go), [watch.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/watch.go), [metrics.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics.go), [process_watch.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/process_watch.go).

### Platform dependencies

Analyze production files are Darwin-build constrained; its non-Darwin main is a stub. Status has portable gopsutil foundations plus partial Linux-adjacent fallbacks (`nvidia-smi`, `bluetoothctl`), but these do not constitute a supported Linux cleanup tool. Process collection explicitly returns no processes outside Darwin. Hardware/CPU/memory/power/GPU/disk enrichments rely on combinations of `system_profiler`, `sysctl`, `vm_stat`, `memory_pressure`, `pmset`, `ioreg`, `powermetrics`, `diskutil`, Finder/AppleScript and Darwin `ps` formats. Disk display filters pseudo/network mounts, drops volumes under 1 GiB, uses device and filesystem+capacity deduplication and shows at most three disks. Linux storage topology needs explicit mount/device identity instead of copying these presentation assumptions.

Sources: [main_stub.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/analyze/main_stub.go), [metrics_process.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics_process.go), [metrics_disk.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics_disk.go), [metrics_memory.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics_memory.go), [metrics_gpu.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics_gpu.go), [metrics_bluetooth.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/cmd/status/metrics_bluetooth.go).

## Shared units

`internal/units/bytes.go` deliberately separates decimal SI disk figures from binary memory/live counters. Binary functions have different exact-boundary promotion conventions: spaced `BytesBin` uses strict `>` while compact variants use `>=`. Linux should pick and label B/kB/MB versus KiB/MiB consistently; reusing a formatter without preserving its consumer convention introduces boundary discrepancies.

Source: [bytes.go](https://github.com/tw93/Mole/blob/8c55e8c07f3b34eeeb0352753b660f823468cd43/internal/units/bytes.go).

## Evidence from representative tests inspected

Read actual test bodies for APFS clone/shared-block and mixed-hardlink fixtures, one-pass Library measurement and exclusion scope, Trash traversal rejection and container aliases, valid/invalid/symlink cache tags, fast collection avoiding external commands, stale versus fresh process enrichment, and real watch-loop subprocess cadence/recovery. The tests include actual platform-dependent filesystem operations and some Darwin-only skips; source existence is not a current green test result. No test suite was executed by this research unit.

## Proposed Linux advantages to validate before implementation

1. **Explainable storage ownership and real reclaimability:** distinguish ordinary files, package-managed data, application caches, containers, snapshots, deleted-open files and filesystem reservations; show why a candidate is rebuildable, which provider owns it, estimated release and uncertainty. Start with `/proc`, mount information and pacman read-only evidence on Arch; validate Linux API/provider contracts before committing to implementation. Inspect inode exhaustion as well as byte pressure. This complements upstream clone awareness rather than pretending it does not exist.
2. **Reviewable plan and execution receipt:** immutable candidate plan with rule/reason, ownership, expected file identity and scan age; revalidate before execution and skip changed items. Record moved-to-Trash separately from released space, and measure free space before/after without attributing unrelated concurrent writes. A receipt is not an all-or-nothing transaction or promise of complete rollback.
3. **Filesystem-native accounting:** mount-aware scans that exclude pseudo-filesystems by default and handle bind mounts, Btrfs subvolumes/reflinks/snapshots, sparse files and hardlinks honestly. Report unknown shared/retained bytes rather than an inflated exact total. Deliver the baseline on ext4 before making Btrfs-exclusive-byte guarantees.
4. **Context-sensitive developer cleanup:** require manifest/provider evidence and preserve Git worktrees, shared stores, active environments, locks and container volumes. Familiar directory names alone should never authorize removal. Use native package/container cache operations when appropriate, with independently previewed scopes.
5. **Growth comparison later:** explicit user-triggered scan baselines and largest-growth deltas, with freshness/error metadata. Upstream's cache and Time Machine count are not that user-facing capability. Avoid turning a cleanup CLI into a permanent monitoring daemon by default.

These are proposals from reviewed source boundaries, not tested claims that mole-linux is already superior or commitments to ship every feature.
