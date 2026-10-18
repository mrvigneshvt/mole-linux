# mole-linux agent context

This project is a Rust scaffold for a Linux-first system cleanup and inspection tool. The current machine is Omarchy 4.0.4 (Arch-based), x86_64, verified 2026-10-19. Upstream Mole describes itself as a macOS tool requiring macOS 12 or newer; this project is an independent Linux tool, not an affiliated port.

## Resume

1. Read [STATUS.md](STATUS.md) for verified repository and toolchain state.
2. Read [HANDOFF.md](HANDOFF.md) for project intent, safety direction, and unresolved decisions.
3. Check current Git state and reverify the target machine before making platform-specific claims.
4. M1 (read-only analyzer TUI) is done; next milestone proposal is in [HANDOFF.md](HANDOFF.md). Decisions: [decisions/](decisions/), plans: [plans/](plans/).

## Upstream research — 2026-10-19

Read the [Mole source review and proposed roadmap](research/mole-upstream-review.md)
for the pinned upstream architecture, existing safety baseline, Linux feature
opportunities and review limits. Supporting reports and the complete tracked-file
inventory are linked there. This is research, not an approved implementation plan.
See [KNOWN-GAPS.md](KNOWN-GAPS.md) for outstanding verification and release decisions.

## Source of truth

- Product overview: [../../README.md](../../README.md)
- Current state: [STATUS.md](STATUS.md)
- Next-agent handoff: [HANDOFF.md](HANDOFF.md)
