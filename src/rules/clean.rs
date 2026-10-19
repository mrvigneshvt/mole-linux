use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryMode {
    Regenerate,
    Trash,
    OwnerNative,
    Irreversible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerCheck {
    None,
    ProcessName(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    pub id: &'static str,
    pub category: &'static str,
    pub evidence: &'static str,
    pub non_targets: &'static [&'static str],
    pub owner_check: OwnerCheck,
    pub recovery: RecoveryMode,
    pub targets: Vec<PathBuf>,
}

/// M4.1 defines the catalog contract; concrete Linux targets arrive in M4.2+.
pub fn catalog() -> Vec<Rule> {
    Vec::new()
}
