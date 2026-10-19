//! Runs only inside the Arch honeypot container with real passwordless sudo:
//! `tests/honeypot/docker/run.sh privilege-sudo`.

use mole_linux::core::outcome::Outcome;
use mole_linux::core::privilege::{Consent, OwnerCommand, OwnerProgram, OwnerRunner, RunMode};

#[test]
#[ignore = "needs real sudo; run through tests/honeypot/docker/run.sh privilege-sudo"]
fn real_sudo_runs_pacman_query_through_owner_runner() {
    let mut runner = OwnerRunner::system(Consent::Granted);
    let cmd = OwnerCommand::new(OwnerProgram::Pacman, ["-Qq"], true);

    let run = runner.run(&cmd, RunMode::Apply, || panic!("--yes must not prompt"));

    assert_eq!(run.outcome, Outcome::Applied, "reason: {:?}", run.reason);
}
