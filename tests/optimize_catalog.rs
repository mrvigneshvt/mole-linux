//! Port of tw93/Mole tests/optimize_catalog.bats (Linux task catalog contract).
//! Upstream: lib/optimize/catalog.sh (GPL-3.0).

use mole_linux::rules::optimize::{
    CatalogError, Exclusions, Handler, OptimizeTask, catalog, lookup, parse_journal_usage, validate,
};

fn task(action: &'static str, safe: bool) -> OptimizeTask {
    OptimizeTask {
        action,
        label: "Label",
        description: "Description",
        safe,
        handler: Handler::FontCache,
    }
}

#[test]
fn catalog_preserves_the_linux_task_contract() {
    let actual: Vec<(&str, &str)> = catalog()
        .iter()
        .map(|task| (task.action, task.label))
        .collect();

    assert_eq!(
        actual,
        vec![
            ("font_cache_refresh", "Font Cache Refresh"),
            ("desktop_mime_refresh", "Desktop & MIME Database Refresh"),
            ("failed_user_units_report", "Failed User Units"),
            ("fstrim_status_report", "Fstrim Timer Status"),
            ("journal_health_report", "Journal Health"),
        ]
    );
    validate(catalog()).expect("shipped catalog is valid");
}

#[test]
fn validate_rejects_an_empty_catalog() {
    assert_eq!(validate(&[]), Err(CatalogError::Empty));
}

#[test]
fn validate_rejects_duplicate_action_identities() {
    let tasks = [
        task("font_cache_refresh", true),
        task("font_cache_refresh", true),
    ];

    assert_eq!(
        validate(&tasks),
        Err(CatalogError::Duplicate("font_cache_refresh".to_owned()))
    );
}

#[test]
fn validate_rejects_tasks_not_safe_for_automatic_execution() {
    let tasks = [task("font_cache_refresh", false)];

    assert_eq!(
        validate(&tasks),
        Err(CatalogError::Unsafe("font_cache_refresh".to_owned()))
    );
}

#[test]
fn validate_rejects_invalid_action_identities() {
    let tasks = [task("Font Cache", true)];

    assert_eq!(
        validate(&tasks),
        Err(CatalogError::InvalidIdentity("Font Cache".to_owned()))
    );
}

#[test]
fn lookup_resolves_only_exact_action_ids() {
    assert_eq!(
        lookup("fstrim_status_report").map(|task| task.handler),
        Some(Handler::FstrimStatus)
    );
    assert!(lookup("fstrim").is_none());
    assert!(lookup("FSTRIM_STATUS_REPORT").is_none());
}

#[test]
fn exclusions_skip_listed_actions_and_warn_about_unknown_entries() {
    let exclusions = Exclusions::parse(
        "# keep these\n\nfont_cache_refresh\nnot_a_task\n  journal_health_report  \n",
    );

    assert!(exclusions.excludes("font_cache_refresh"));
    assert!(exclusions.excludes("journal_health_report"));
    assert!(!exclusions.excludes("fstrim_status_report"));
    assert_eq!(
        exclusions.warnings,
        vec!["unknown optimize task: not_a_task".to_owned()]
    );
}

#[test]
fn journal_usage_parses_systemd_sizes_exactly() {
    let gib = 1024_u64 * 1024 * 1024;

    assert_eq!(
        parse_journal_usage("Archived and active journals take up 1.5G in the file system.\n"),
        Some(gib + gib / 2)
    );
    assert_eq!(
        parse_journal_usage("Journals take up 512.0M in the file system."),
        Some(512 * 1024 * 1024)
    );
    assert_eq!(
        parse_journal_usage("Journals take up 8.0K in the file system."),
        Some(8 * 1024)
    );
    assert_eq!(parse_journal_usage("unexpected output"), None);
    assert_eq!(
        parse_journal_usage("Journals take up 99999999999999999999G in the file system."),
        None,
        "overflowing sizes are refused, not wrapped"
    );
}
