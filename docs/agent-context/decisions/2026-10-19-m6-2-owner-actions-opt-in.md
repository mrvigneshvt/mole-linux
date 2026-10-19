# M6.2 owner actions are opt-in and apply only after one consent

Status: accepted for M6.2 (issue #26), implemented on branch `issue-26`.

## Decision

1. Owner-native candidates (pacman cache via `paccache`) are **not selected by default**. They run only with an explicit `--select <rule-id>` or `--select <category>`. Sink-backed candidates (AUR build dirs) keep the existing default-selected behavior.
2. Owner-native candidates are not checked against the HOME deletion policy, because the owner tool chooses targets outside HOME (`/var/cache/pacman/pkg`). The whitelist and the owner-state recheck still apply.
3. Consent is asked once per run, after the general clean confirmation. The prompt lists the exact argv of every planned owner command. `--yes` grants it. Without a terminal and without `--yes`, nothing runs.

## Why

- Bare `mole-linux clean` on the host selected `paccache -duk0` by default, so a plain run demanded sudo consent. The integration test `global_debug_is_accepted_before_and_after_subcommand` caught it (exit 2 from the non-tty confirm). Privilege should require a named selection.
- The sink's HOME policy cannot describe `/var/cache/pacman/pkg`. Forcing owner actions through it would either reject them or weaken the policy for everything.

## Evidence

- `tests/clean_system.rs` (`owner_actions_are_opt_in_and_never_prompt_without_an_explicit_select`, `category_select_opts_into_owner_actions`, `apply_runs_keep_three_through_sudo_after_one_consent_with_exact_argv`, `declined_consent_runs_no_sudo_and_reports_skipped`).
- `tests/cli.rs` (`global_debug_is_accepted_before_and_after_subcommand`) passes again.
- Honeypot: `tests/honeypot/scenarios/clean-pacman-apply.sh` (run through `tests/honeypot/docker/run.sh clean-pacman-apply`).

## Consequences

- `--categories system` alone discovers and previews but does not apply; add `--select system` to opt in.
- The removed files are chosen by `paccache` at apply time (see KNOWN-GAPS, M6.2 paccache entry).
