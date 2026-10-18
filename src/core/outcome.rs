// Ported from tw93/Mole lib/optimize/outcomes.sh (GPL-3.0).

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Applied,
    Unchanged,
    Skipped,
    Unavailable,
    Attention,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeError {
    TaskAlreadyActive,
    TaskNotActive,
    OutcomeAlreadySet,
    MissingOutcome,
    DuplicateAction(String),
    InvalidAction(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutcomeLedger {
    task_active: bool,
    task_outcome: Option<Outcome>,
    results: Vec<(String, Outcome)>,
}

impl OutcomeLedger {
    pub fn start(&mut self) -> Result<(), OutcomeError> {
        if self.task_active {
            return Err(OutcomeError::TaskAlreadyActive);
        }
        self.task_active = true;
        self.task_outcome = None;
        Ok(())
    }

    pub fn report(&mut self, outcome: Outcome) -> Result<(), OutcomeError> {
        if !self.task_active {
            return Err(OutcomeError::TaskNotActive);
        }
        if self.task_outcome.is_some() {
            return Err(OutcomeError::OutcomeAlreadySet);
        }
        self.task_outcome = Some(outcome);
        Ok(())
    }

    pub fn report_from_counts(
        &mut self,
        applied: u64,
        failed: u64,
        skipped: u64,
    ) -> Result<(), OutcomeError> {
        let outcome = if failed > 0 {
            Outcome::Failed
        } else if applied > 0 {
            Outcome::Applied
        } else if skipped > 0 {
            Outcome::Skipped
        } else {
            Outcome::Unchanged
        };
        self.report(outcome)
    }

    pub fn finish(&mut self, action: &str) -> Result<(), OutcomeError> {
        if !self.task_active {
            return Err(OutcomeError::TaskNotActive);
        }
        if !valid_action(action) {
            return Err(OutcomeError::InvalidAction(action.to_owned()));
        }
        let Some(outcome) = self.task_outcome else {
            return Err(OutcomeError::MissingOutcome);
        };
        if self.results.iter().any(|(existing, _)| existing == action) {
            return Err(OutcomeError::DuplicateAction(action.to_owned()));
        }
        self.results.push((action.to_owned(), outcome));
        self.task_active = false;
        self.task_outcome = None;
        Ok(())
    }

    pub fn count(&self, requested: Outcome) -> usize {
        self.results
            .iter()
            .filter(|(_, outcome)| *outcome == requested)
            .count()
    }

    pub fn total(&self) -> usize {
        self.results.len()
    }

    pub fn actions_with_outcome(&self, requested: Outcome) -> Vec<&str> {
        self.results
            .iter()
            .filter(|(_, outcome)| *outcome == requested)
            .map(|(action, _)| action.as_str())
            .collect()
    }

    pub fn failed_actions(&self) -> Vec<&str> {
        self.actions_with_outcome(Outcome::Failed)
    }

    pub fn succeeded(&self) -> bool {
        self.count(Outcome::Failed) == 0
    }
}

fn valid_action(action: &str) -> bool {
    !action.is_empty()
        && action
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
