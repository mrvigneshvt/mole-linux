use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
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

/// Finds a process by its procfs `comm` name without treating unreadable entries as absence.
pub fn is_process_name_running(proc_root: &Path, expected: &str) -> ProcessState {
    let entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(_) => return ProcessState::Unknown,
    };
    let mut unknown = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                unknown = true;
                continue;
            }
        };
        if entry
            .file_name()
            .to_str()
            .is_none_or(|name| !name.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let comm_path = entry.path().join("comm");
        let mut comm = String::new();
        let file = match fs::File::open(comm_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                unknown = true;
                continue;
            }
        };
        if file.take(4097).read_to_string(&mut comm).is_err() || comm.len() > 4096 {
            unknown = true;
            continue;
        }
        if comm.trim_end_matches(['\r', '\n']) == expected {
            return ProcessState::Running;
        }
    }
    if unknown {
        ProcessState::Unknown
    } else {
        ProcessState::NotRunning
    }
}

const CMDLINE_LIMIT: u64 = 64 * 1024;

/// Reports whether a process runs one of `names`, judged by the basenames of argv[0] and argv[1].
/// argv[1] catches node-wrapped owners (`node .../pnpm.cjs`) without matching a lockfile or an
/// argument that merely mentions the name. Unreadable entries stay unknown unless a match is found.
pub fn is_command_running(proc_root: &Path, names: &[&str]) -> ProcessState {
    let entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(_) => return ProcessState::Unknown,
    };
    let mut unknown = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                unknown = true;
                continue;
            }
        };
        if entry
            .file_name()
            .to_str()
            .is_none_or(|name| !name.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let file = match fs::File::open(entry.path().join("cmdline")) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                unknown = true;
                continue;
            }
        };
        let mut cmdline = Vec::new();
        if file.take(CMDLINE_LIMIT).read_to_end(&mut cmdline).is_err() {
            unknown = true;
            continue;
        }
        if argv_names_command(&cmdline, names) {
            return ProcessState::Running;
        }
        // A truncated argv may hide argv[1]; absence is then unproven.
        if u64::try_from(cmdline.len()).is_ok_and(|len| len >= CMDLINE_LIMIT) {
            unknown = true;
        }
    }
    if unknown {
        ProcessState::Unknown
    } else {
        ProcessState::NotRunning
    }
}

fn argv_names_command(cmdline: &[u8], names: &[&str]) -> bool {
    cmdline.split(|byte| *byte == 0).take(2).any(|arg| {
        Path::new(OsStr::from_bytes(arg))
            .file_name()
            .is_some_and(|base| names.iter().any(|name| base == OsStr::new(name)))
    })
}
