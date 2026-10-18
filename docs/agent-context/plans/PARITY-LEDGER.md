# Parity ledger

Pinned upstream `8c55e8c`. Generated 2026-10-19 from `clean_*`/`check_*` entry functions in upstream `lib/clean/*.sh`, plus commands and test files. Disposition column is a **proposal** from name + module (auto-tagged: macOS-only names → Drop); each row is confirmed when its milestone unit spec is written. Status: `todo` / `spec` / `wip` / `done` / `dropped`.

Edit rules: one row per upstream entry point; never delete a row — set `dropped` with a reason.

## Test environment

| Unit | Milestone | Status | Evidence |
|---|---|---|---|
| Honeypot HOME, canaries, tool shims, Arch smoke | M0.1 | done | `tests/honeypot/README.md`; `tests/honeypot.rs` |

## Commands

| Upstream | Milestone | Disposition | Status | Evidence |
|---|---|---|---|---|
| `mo analyze` | M1, M2 | Port | wip (M1, M2.1 done; M0.2 clap routing) | plans/M1-analyzer.md; issue #3 CLI; issue #4 progressive events, parallel hardlink dedupe, cancellation |
| `mo status` | M9.1 | Adapt | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo clean` | M4–M6 | Adapt | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo purge` | M7.1 | Port | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo installer` | M7.2 | Adapt | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo uninstall` | M8 | Replace | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo optimize` | M9.2 | Replace | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo history` | M3.3 | Port | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| `mo completion` | M9.4 | Port | wip (M0.2 CLI surface; behavior pending) | issue #3 |
| menu | M9.3 | Port | todo | |
| `mo update`/`remove` | M10 | Drop/decide | todo | packaged installs owned by pacman |
| `mo touchid` | — | Drop | dropped | auth/PAM change outside scope (research/upstream-lifecycle-maintenance.md) |

## `lib/clean/user.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `check_large_file_candidates` | M4 | Adapt/Port — triage | todo |
| `clean_app_caches` | M4 | Adapt/Port — triage | todo |
| `clean_apple_silicon_caches` | M4 | Drop (macOS-specific) | todo |
| `clean_application_support_logs` | M4 | Drop (macOS-specific) | todo |
| `clean_brave_old_versions` | M4 | Adapt/Port — triage | todo |
| `clean_browsers` | M4 | Adapt/Port — triage | todo |
| `clean_cached_device_firmware` | M4 | Drop (macOS-specific) | todo |
| `clean_chrome_old_versions` | M4 | Adapt/Port — triage | todo |
| `clean_cloud_storage` | M4 | Adapt/Port — triage | todo |
| `clean_edge_old_versions` | M4 | Adapt/Port — triage | todo |
| `clean_edge_updater_old_versions` | M4 | Drop (macOS-specific) | todo |
| `clean_external_volume_target` | M4 | Adapt/Port — triage | todo |
| `clean_finder_metadata` | M4 | Drop (macOS-specific) | todo |
| `clean_group_container_caches` | M4 | Drop (macOS-specific) | todo |
| `clean_handoff_pasteboard_cache` | M4 | Drop (macOS-specific) | todo |
| `clean_office_applications` | M4 | Adapt/Port — triage | todo |
| `clean_support_app_data` | M4 | Drop (macOS-specific) | todo |
| `clean_tart_caches` | M4 | Drop (macOS-specific) | todo |
| `clean_trash` | M4 | Adapt/Port — triage | todo |
| `clean_user_essentials` | M4 | Adapt/Port — triage | todo |
| `clean_utm_caches` | M4 | Drop (macOS-specific) | todo |
| `clean_virtualization_tools` | M4 | Adapt/Port — triage | todo |

## `lib/clean/caches.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `check_tcc_permissions` | M4.4 | Drop (macOS-specific) | todo |
| `clean_project_caches` | M4.4 | Adapt/Port — triage | todo |
| `clean_project_cache_target` | M4.4 | Adapt/Port — triage | todo |
| `clean_python_bytecode_cache_group` | M4.4 | Adapt/Port — triage | todo |
| `clean_service_worker_cache` | M4.4 | Adapt/Port — triage | todo |

## `lib/clean/dev.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `check_android_ndk` | M5 | Drop (macOS-specific) | todo |
| `check_multiple_versions` | M5 | Adapt/Port — triage | todo |
| `check_orphaned_simulator_runtimes` | M5 | Drop (macOS-specific) | todo |
| `check_rust_toolchains` | M5 | Adapt/Port — triage | todo |
| `clean_agent_browser_old_chrome` | M5 | Adapt/Port — triage | todo |
| `clean_antigravity_caches` | M5 | Adapt/Port — triage | todo |
| `clean_chrome_devtools_mcp_caches` | M5 | Adapt/Port — triage | todo |
| `clean_chromium_default_caches` | M5 | Adapt/Port — triage | todo |
| `clean_clang_module_cache` | M5 | Adapt/Port — triage | todo |
| `clean_claude_desktop_bundled_versions` | M5 | Drop (macOS-specific) | todo |
| `clean_codex_cli` | M5 | Adapt/Port — triage | todo |
| `clean_codex_crashpad_pending` | M5 | Adapt/Port — triage | todo |
| `clean_codex_daemon_releases` | M5 | Adapt/Port — triage | todo |
| `clean_codex_desktop_caches` | M5 | Drop (macOS-specific) | todo |
| `clean_codex_desktop_staging` | M5 | Drop (macOS-specific) | todo |
| `clean_codex_marketplace_staging` | M5 | Adapt/Port — triage | todo |
| `clean_codex_runtimes` | M5 | Adapt/Port — triage | todo |
| `clean_conda_metadata_caches` | M5 | Adapt/Port — triage | todo |
| `clean_corepack_cache` | M5 | Adapt/Port — triage | todo |
| `clean_dev_ai_agents` | M5 | Adapt/Port — triage | todo |
| `clean_dev_api_tools` | M5 | Adapt/Port — triage | todo |
| `clean_dev_automation_browsers` | M5 | Adapt/Port — triage | todo |
| `clean_dev_cicd` | M5 | Adapt/Port — triage | todo |
| `clean_dev_cloud` | M5 | Adapt/Port — triage | todo |
| `clean_dev_database` | M5 | Adapt/Port — triage | todo |
| `clean_dev_docker` | M5 | Adapt/Port — triage | todo |
| `clean_dev_elixir` | M5 | Adapt/Port — triage | todo |
| `clean_developer_tools` | M5 | Adapt/Port — triage | todo |
| `clean_dev_frontend` | M5 | Adapt/Port — triage | todo |
| `clean_dev_go` | M5 | Adapt/Port — triage | todo |
| `clean_dev_jetbrains_logs` | M5 | Adapt/Port — triage | todo |
| `clean_dev_jetbrains_toolbox` | M5 | Adapt/Port — triage | todo |
| `clean_dev_jvm` | M5 | Adapt/Port — triage | todo |
| `clean_dev_misc` | M5 | Adapt/Port — triage | todo |
| `clean_dev_mise` | M5 | Adapt/Port — triage | todo |
| `clean_dev_mobile` | M5 | Adapt/Port — triage | todo |
| `clean_dev_network` | M5 | Adapt/Port — triage | todo |
| `clean_dev_nix` | M5 | Adapt/Port — triage | todo |
| `clean_dev_npm` | M5 | Adapt/Port — triage | todo |
| `clean_dev_ocaml` | M5 | Adapt/Port — triage | todo |
| `clean_dev_other_langs` | M5 | Adapt/Port — triage | todo |
| `clean_dev_perl` | M5 | Adapt/Port — triage | todo |
| `clean_dev_python` | M5 | Adapt/Port — triage | todo |
| `clean_dev_ruby` | M5 | Adapt/Port — triage | todo |
| `clean_dev_rust` | M5 | Adapt/Port — triage | todo |
| `clean_dev_shell` | M5 | Adapt/Port — triage | todo |
| `clean_github_cli_cache` | M5 | Adapt/Port — triage | todo |
| `clean_go_cache_root` | M5 | Adapt/Port — triage | todo |
| `clean_guarded_dev_cache_root` | M5 | Adapt/Port — triage | todo |
| `clean_pnpm_stores` | M5 | Adapt/Port — triage | todo |
| `clean_pyinstaller_bincache` | M5 | Adapt/Port — triage | todo |
| `clean_rust_dependency_cache_root` | M5 | Adapt/Port — triage | todo |
| `clean_tool_cache` | M5 | Adapt/Port — triage | todo |
| `clean_uv_cache` | M5 | Adapt/Port — triage | todo |
| `clean_versioned_agent_root` | M5 | Adapt/Port — triage | todo |
| `clean_xcode_device_support` | M5 | Drop (macOS-specific) | todo |
| `clean_xcode_documentation_cache` | M5 | Drop (macOS-specific) | todo |
| `clean_xcode_system_coresimulator_caches` | M5 | Drop (macOS-specific) | todo |
| `clean_xcode_xctest_devices` | M5 | Drop (macOS-specific) | todo |

## `lib/clean/app_caches.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `clean_3d_tools` | M4.4 | Adapt/Port — triage | todo |
| `clean_ai_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_autodesk_fusion_old_bundles` | M4.4 | Drop (macOS-specific) | todo |
| `clean_code_editors` | M4.4 | Adapt/Port — triage | todo |
| `clean_communication_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_design_tools` | M4.4 | Adapt/Port — triage | todo |
| `clean_dingtalk` | M4.4 | Drop (macOS-specific) | todo |
| `clean_download_managers` | M4.4 | Adapt/Port — triage | todo |
| `clean_editor_obsolete_extensions` | M4.4 | Adapt/Port — triage | todo |
| `clean_email_clients` | M4.4 | Adapt/Port — triage | todo |
| `clean_feishu_service_worker_caches` | M4.4 | Drop (macOS-specific) | todo |
| `clean_gaming_platforms` | M4.4 | Adapt/Port — triage | todo |
| `clean_jianying_pro_generated_caches` | M4.4 | Drop (macOS-specific) | todo |
| `clean_launcher_apps` | M4.4 | Drop (macOS-specific) | todo |
| `clean_media_players` | M4.4 | Adapt/Port — triage | todo |
| `clean_neatdm_stale_segments` | M4.4 | Adapt/Port — triage | todo |
| `clean_note_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_notion_service_worker_caches` | M4.4 | Adapt/Port — triage | todo |
| `clean_productivity_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_remote_desktop` | M4.4 | Adapt/Port — triage | todo |
| `clean_screenshot_tools` | M4.4 | Adapt/Port — triage | todo |
| `clean_shell_utils` | M4.4 | Adapt/Port — triage | todo |
| `clean_system_utils` | M4.4 | Adapt/Port — triage | todo |
| `clean_task_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_translation_apps` | M4.4 | Adapt/Port — triage | todo |
| `clean_user_gui_applications` | M4.4 | Adapt/Port — triage | todo |
| `clean_video_players` | M4.4 | Adapt/Port — triage | todo |
| `clean_video_tools` | M4.4 | Adapt/Port — triage | todo |
| `clean_wechat_container_caches` | M4.4 | Drop (macOS-specific) | todo |
| `clean_xcode_derived_data` | M4.4 | Drop (macOS-specific) | todo |
| `clean_xcode_tools` | M4.4 | Drop (macOS-specific) | todo |

## `lib/clean/system.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `clean_deep_system` | M6 | Drop (macOS-specific) | todo |
| `clean_local_snapshots` | M6 | Drop (macOS-specific) | todo |
| `clean_time_machine_failed_backups` | M6 | Drop (macOS-specific) | todo |

## `lib/clean/apps.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `clean_ds_store_tree` | M4 | Adapt/Port — triage | todo |
| `clean_orphaned_app_data` | M4 | Adapt/Port — triage | todo |
| `clean_orphaned_container_stubs` | M4 | Adapt/Port — triage | todo |
| `clean_orphaned_system_services` | M4 | Adapt/Port — triage | todo |

## `lib/clean/brew.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `clean_homebrew` | M4 | Drop (macOS-specific) | todo |
| `clean_homebrew_service_logs` | M4 | Drop (macOS-specific) | todo |

## `lib/clean/hints.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|

## `lib/clean/project.sh`

| Function | Milestone | Disposition (proposed) | Status |
|---|---|---|---|
| `clean_project_artifacts` | M7.1 | Adapt/Port — triage | todo |

## Upstream test contracts to port

| Test file | Milestone | Status |
|---|---|---|
| `brew_uninstall.bats` | M8 | todo |
| `browser_clones.bats` | M4 | todo |
| `bundle_resolver.bats` | triage | todo |
| `clean_ai_cli_caches.bats` | M5 | todo |
| `clean_app_caches.bats` | M4 | todo |
| `clean_apps.bats` | M4 | todo |
| `clean_automation_browsers.bats` | M5 | todo |
| `clean_browser_versions.bats` | M4 | todo |
| `clean_cached_device_firmware.bats` | M4 | todo |
| `clean_core.bats` | M4 | todo |
| `clean_corepack.bats` | M5 | todo |
| `clean_dev_caches.bats` | M5 | todo |
| `clean_developer_state.bats` | M5 | todo |
| `clean_external.bats` | M4 | todo |
| `clean_hints.bats` | M4 | todo |
| `clean_mail_downloads.bats` | M4 | todo |
| `clean_misc.bats` | M4 | todo |
| `clean_orphaned_runtimes.bats` | M6 | todo |
| `clean_summary_cancel.bats` | M4 | todo |
| `clean_system_caches.bats` | M6 | todo |
| `clean_system_maintenance.bats` | M6 | todo |
| `clean_user_core.bats` | M4 | todo |
| `clean_xcode_derived_data.bats` | M4 | todo |
| `cli.bats` | triage | todo |
| `color_tty.exp` | triage | todo |
| `completion.bats` | M9.4 | todo |
| `core_common.bats` | triage | todo |
| `core_performance.bats` | triage | todo |
| `core_safe_functions.bats` | M3 | todo |
| `core_timeout.bats` | triage | todo |
| `dev_extended.bats` | M5 | todo |
| `e2e_harness_test.py` | triage | todo |
| `file_ops_mole_delete.bats` | M3 | todo |
| `file_ops_safe_remove_symlink.bats` | M3 | todo |
| `file_ops_size.bats` | M3 | todo |
| `format_on_edit_hook.bats` | triage | todo |
| `health_json.bats` | M9.1 | todo |
| `history.bats` | M3 | todo |
| `install_checksum.bats` | M10/Drop | todo |
| `installer.bats` | M7.2 | todo |
| `installer_fd.bats` | M7.2 | todo |
| `installer_zip.bats` | M7.2 | todo |
| `main_menu_pty.py` | M9.3 | todo |
| `manage_sudo.bats` | M3 | todo |
| `manage_whitelist.bats` | M3 | todo |
| `menu_search_pty.py` | M9.3 | todo |
| `menu_trap_restore.bats` | M9.3 | todo |
| `no_color.bats` | triage | todo |
| `optimize.bats` | M9.2 | todo |
| `optimize_catalog.bats` | M9.2 | todo |
| `optimize_db.bats` | M9.2 | todo |
| `optimize_outcomes.bats` | M9.2 | todo |
| `optimize_perf_diagnosis.bats` | M9.2 | todo |
| `optimize_probe_outcomes.bats` | M9.2 | todo |
| `optimize_removal_outcomes.bats` | M9.2 | todo |
| `optimize_summary.bats` | M9.2 | todo |
| `optimize_unavailable_outcomes.bats` | M9.2 | todo |
| `path_validation_fuzz.bats` | M3 | todo |
| `purge.bats` | M7.1 | todo |
| `purge_config_paths.bats` | M7.1 | todo |
| `purge_menu_pty.py` | M7.1 | todo |
| `regression.bats` | triage | todo |
| `scripts.bats` | triage | todo |
| `sudo_adoption_pty.py` | triage | todo |
| `sudo_tty_pty.py` | triage | todo |
| `timeout_tty_background.exp` | triage | todo |
| `timeout_tty_concurrent.exp` | triage | todo |
| `timeout_tty_foreground.bats` | triage | todo |
| `timeout_tty_read.exp` | triage | todo |
| `timeout_tty_restore.exp` | triage | todo |
| `uninstall.bats` | M8 | todo |
| `uninstall_menu.bats` | M8 | todo |
| `uninstall_menu_pty.py` | M8 | todo |
| `uninstall_naming_variants.bats` | M8 | todo |
| `uninstall_process_pattern.bats` | M8 | todo |
| `uninstall_remove_file_list.bats` | M8 | todo |
| `uninstall_review.bats` | M8 | todo |
| `uninstall_review_pty.py` | M8 | todo |
| `uninstall_safety.bats` | M8 | todo |
| `uninstall_scan_bash32.bats` | M8 | todo |
| `uninstall_steam_launcher.bats` | M8 | todo |
| `uninstall_tty_foreground.bats` | M8 | todo |
| `uninstall_tty_foreground.exp` | M8 | todo |
| `update.bats` | M10/Drop | todo |
| `user_file_ops.bats` | M3 | todo |
