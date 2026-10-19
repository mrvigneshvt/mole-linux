//! Orphan packages (`pacman -Qdtq`) as a report-only system rule. It yields no candidates and no
//! owner command, so nothing is removed here; removing a package stays a manual decision.
//! Linux-native: no upstream equivalent.

use std::collections::BTreeMap;

use crate::core::privilege::{OwnerCommand, OwnerProgram, OwnerRun, OwnerRunner};
use crate::core::units::{bytes_si, parse_iec_size};
use crate::rules::clean::{Category, CleanRule, Discovery, DiscoveryStatus, Report, RuleContext};

pub struct OrphanPackagesRule;

impl CleanRule for OrphanPackagesRule {
    fn id(&self) -> &str {
        "orphan-packages"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn discover(&self, _ctx: &RuleContext<'_>) -> Discovery {
        Discovery {
            candidates: Vec::new(),
            status: DiscoveryStatus::Complete,
        }
    }

    fn report_with_owner(&self, _ctx: &RuleContext<'_>, owner: &OwnerRunner) -> Option<Report> {
        Some(orphan_report(owner))
    }
}

fn orphan_report(owner: &OwnerRunner) -> Report {
    // Exit 1 with no output and no stderr is "nothing orphaned"; any other failure is an error.
    let list = OwnerCommand::new(OwnerProgram::Pacman, ["-Qdtq"], false).no_match_on_exit(1);
    let names = match owner.query(&list) {
        Ok(stdout) => match names_from(&stdout) {
            Ok(names) => names,
            Err(reason) => return Report::Unavailable(reason),
        },
        Err(run) => return Report::Unavailable(format!("pacman -Qdtq: {}", reason_of(&run))),
    };
    if names.is_empty() {
        return Report::Clear;
    }
    let info = OwnerCommand::new(
        OwnerProgram::Pacman,
        std::iter::once("-Qi".to_owned()).chain(names.iter().cloned()),
        false,
    );
    let sizes = match owner.query(&info) {
        Ok(output) => parse_installed_sizes(&output),
        Err(run) => return Report::Unavailable(format!("pacman -Qi: {}", reason_of(&run))),
    };

    let mut known_total = 0_u64;
    let mut unknown = 0_usize;
    let mut lines = Vec::with_capacity(names.len() + 2);
    for name in &names {
        match sizes.get(name).copied().flatten() {
            Some(bytes) => {
                known_total = known_total.saturating_add(bytes);
                lines.push(format!("  {name}: {}", bytes_si(bytes)));
            }
            None => {
                unknown += 1;
                lines.push(format!("  {name}: size unknown"));
            }
        }
    }
    let total = match (unknown, names.len()) {
        (0, _) => format!("{} installed", bytes_si(known_total)),
        (count, all) if count == all => "installed size unknown".to_owned(),
        _ => format!(
            "at least {} installed; {unknown} size unknown",
            bytes_si(known_total)
        ),
    };
    let mut findings = vec![format!(
        "{} orphan package(s), {total}; report only, nothing removed",
        names.len()
    )];
    findings.extend(lines);
    findings.push(
        "  review, then remove by hand if unneeded (pacman -Rns <name>); sizes are installed size, not space freed"
            .to_owned(),
    );
    Report::Findings(findings)
}

/// Why the owner runner refused a query, for the operator. Every refusal carries a reason.
fn reason_of(run: &OwnerRun) -> String {
    run.reason
        .clone()
        .unwrap_or_else(|| "query failed".to_owned())
}

fn names_from(stdout: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !valid_package_name(line) {
            return Err(format!("unexpected package name from pacman: {line:?}"));
        }
        names.push(line.to_owned());
    }
    Ok(names)
}

/// Names reach argv, so anything that could parse as an option is refused.
fn valid_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '+' | '-'))
}

/// Reads `pacman -Qi` blocks: each `Name` opens a package, and its `Installed Size` follows.
/// A package with no parsable size maps to `None`, which is unknown rather than zero.
fn parse_installed_sizes(output: &str) -> BTreeMap<String, Option<u64>> {
    let mut sizes = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Name" => {
                let name = value.trim().to_owned();
                sizes.insert(name.clone(), None);
                current = Some(name);
            }
            "Installed Size" => {
                if let Some(name) = &current {
                    sizes.insert(name.clone(), parse_iec_size(value.trim()));
                }
            }
            _ => {}
        }
    }
    sizes
}
