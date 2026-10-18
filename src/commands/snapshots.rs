use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_TOOL_OUTPUT: u64 = 1024 * 1024;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run() -> io::Result<()> {
    io::stdout().lock().write_all(report().as_bytes())
}

fn report() -> String {
    let mut report = String::new();
    let btrfs = run_listing("btrfs", &["subvolume", "list", "/"]);
    let snapper = run_listing("snapper", &["list"]);

    if !btrfs.installed && !snapper.installed {
        return "Unavailable: neither btrfs nor snapper is installed\n".to_owned();
    }

    append_listing(&mut report, "Btrfs subvolumes/snapshots", "btrfs", btrfs);
    append_listing(&mut report, "Snapper snapshots", "snapper", snapper);
    report.push_str("Snapshots can retain shared filesystem extents; listed counts or logical sizes do not show reclaimable space.\n");
    report
}

struct Listing {
    installed: bool,
    result: Result<Vec<String>, String>,
}

fn run_listing(program: &str, args: &[&str]) -> Listing {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Listing {
                installed: false,
                result: Err("not installed".to_owned()),
            };
        }
        Err(error) => {
            return Listing {
                installed: true,
                result: Err(format!("could not start: {error}")),
            };
        }
    };

    let Some(stdout) = child.stdout.take() else {
        return Listing {
            installed: true,
            result: Err("listing command stdout was unavailable".to_owned()),
        };
    };
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_TOOL_OUTPUT + 1).read_to_end(&mut bytes);
        (result, bytes)
    });
    let started = Instant::now();
    let status: ExitStatus = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TOOL_TIMEOUT => {
                thread::sleep(Duration::from_millis(25))
            }
            Ok(None) => {
                let kill_result = child.kill();
                let wait_result = child.wait();
                let reason = match (kill_result, wait_result) {
                    (Ok(()), Ok(_)) => "command timed out after 10 seconds".to_owned(),
                    (Err(kill_error), Ok(_)) => {
                        format!("command timed out; could not terminate it: {kill_error}")
                    }
                    (_, Err(wait_error)) => {
                        format!("command timed out; could not reap it: {wait_error}")
                    }
                };
                return Listing {
                    installed: true,
                    result: Err(reason),
                };
            }
            Err(error) => {
                let kill_result = child.kill();
                let wait_result = child.wait();
                let reason = match (kill_result, wait_result) {
                    (Ok(()), Ok(_)) => format!("could not check command status: {error}"),
                    (Err(kill_error), Ok(_)) => format!(
                        "could not check command status: {error}; could not terminate it: {kill_error}"
                    ),
                    (_, Err(wait_error)) => format!(
                        "could not check command status: {error}; could not reap it: {wait_error}"
                    ),
                };
                return Listing {
                    installed: true,
                    result: Err(reason),
                };
            }
        }
    };
    let (read_result, bytes) = match reader.join() {
        Ok(result) => result,
        Err(_) => {
            return Listing {
                installed: true,
                result: Err("output reader failed".to_owned()),
            };
        }
    };
    if let Err(error) = read_result {
        return Listing {
            installed: true,
            result: Err(format!("could not read output: {error}")),
        };
    }
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_TOOL_OUTPUT) {
        return Listing {
            installed: true,
            result: Err("output exceeded 1 MiB limit".to_owned()),
        };
    }
    if status.success() {
        let output = String::from_utf8_lossy(&bytes);
        Listing {
            installed: true,
            result: Ok(output.lines().map(str::to_owned).collect()),
        }
    } else {
        Listing {
            installed: true,
            result: Err(format!("command exited with {status}")),
        }
    }
}

fn append_listing(report: &mut String, title: &str, program: &str, listing: Listing) {
    if !listing.installed {
        return;
    }
    match listing.result {
        Ok(lines) => {
            let entries: Vec<&str> = lines
                .iter()
                .filter(|line| match program {
                    "btrfs" => line.trim_start().starts_with("ID "),
                    "snapper" => line
                        .split_whitespace()
                        .next()
                        .is_some_and(|field| field.parse::<u64>().is_ok()),
                    _ => false,
                })
                .map(String::as_str)
                .collect();
            report.push_str(&format!("{title} ({} entries)\n", entries.len()));
            for entry in entries {
                report.push_str("  ");
                report.push_str(&escape_controls(entry));
                report.push('\n');
            }
        }
        Err(reason) => report.push_str(&format!("{title}: Unavailable ({reason})\n")),
    }
}

fn escape_controls(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
