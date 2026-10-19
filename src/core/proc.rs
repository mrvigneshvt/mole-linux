use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Running,
    NotRunning,
    Unknown,
}

/// Checks a process through procfs. An unreadable or malformed proc entry stays unknown.
pub fn is_process_running(proc_root: &Path, pid: u32) -> ProcessState {
    let proc_entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(_) => return ProcessState::Unknown,
    };
    drop(proc_entries);

    let process_dir = proc_root.join(pid.to_string());
    match fs::metadata(&process_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return ProcessState::Unknown,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return ProcessState::NotRunning,
        Err(_) => return ProcessState::Unknown,
    }

    let mut stat = String::new();
    let stat_file = match fs::File::open(process_dir.join("stat")) {
        Ok(file) => file,
        Err(_) => return ProcessState::Unknown,
    };
    if stat_file.take(4097).read_to_string(&mut stat).is_err() || stat.len() > 4096 {
        return ProcessState::Unknown;
    }
    let Some(close_comm) = stat.rfind(')') else {
        return ProcessState::Unknown;
    };
    let Some((stat_pid, _)) = stat.split_once(' ') else {
        return ProcessState::Unknown;
    };
    if stat_pid.parse::<u32>().ok() != Some(pid) {
        return ProcessState::Unknown;
    }
    let Some(state) = stat[close_comm + 1..].split_whitespace().next() else {
        return ProcessState::Unknown;
    };
    if state.len() != 1 {
        return ProcessState::Unknown;
    }
    if matches!(state, "Z" | "X") {
        ProcessState::NotRunning
    } else {
        ProcessState::Running
    }
}
