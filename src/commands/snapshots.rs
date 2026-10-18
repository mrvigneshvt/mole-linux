use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_TOOL_OUTPUT: u64 = 1024 * 1024;
const MAX_STDERR_OUTPUT: u64 = 4096;
const MAX_STDERR_LINE_CHARS: usize = 240;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run() -> io::Result<()> {
    io::stdout().lock().write_all(report().as_bytes())
}

fn report() -> String {
    let mut report = String::new();
    let btrfs = run_listing("btrfs", &["subvolume", "list", "/"]);
    let snapper = run_listing("snapper", &["list"]);

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
        .stderr(Stdio::piped())
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
    let stdout_reader = read_bounded(stdout, MAX_TOOL_OUTPUT);
    let Some(stderr) = child.stderr.take() else {
        let kill_result = child.kill();
        let wait_result = child.wait();
        let reason = match (kill_result, wait_result) {
            (Ok(()), Ok(_)) => "listing command stderr was unavailable".to_owned(),
            (Err(kill_error), Ok(_)) => {
                format!("stderr unavailable; could not terminate command: {kill_error}")
            }
            (_, Err(wait_error)) => {
                format!("stderr unavailable; could not reap command: {wait_error}")
            }
        };
        return Listing {
            installed: true,
            result: Err(reason),
        };
    };
    let stderr_reader = read_bounded(stderr, MAX_STDERR_OUTPUT);
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
    let (read_result, bytes) = match stdout_reader.join() {
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
    let (stderr_read_result, stderr_bytes) = match stderr_reader.join() {
        Ok(result) => result,
        Err(_) => {
            return Listing {
                installed: true,
                result: Err("stderr reader failed".to_owned()),
            };
        }
    };
    if let Err(error) = stderr_read_result {
        return Listing {
            installed: true,
            result: Err(format!("could not read stderr: {error}")),
        };
    }
    if status.success() {
        let output = String::from_utf8_lossy(&bytes);
        Listing {
            installed: true,
            result: Ok(output.lines().map(str::to_owned).collect()),
        }
    } else {
        let stderr_line = first_stderr_line(&stderr_bytes);
        let detail = if stderr_line.is_empty() {
            format!("command exited with {status}")
        } else {
            stderr_line
        };
        let lower_detail = detail.to_ascii_lowercase();
        let permission_problem = lower_detail.contains("permission")
            || lower_detail.contains("not permitted")
            || lower_detail.contains("operation not allowed");
        let hint = if permission_problem {
            format!(
                "; requires root; run `sudo {} {}` yourself — mole-linux does not escalate for read-only listings",
                program,
                args.join(" ")
            )
        } else {
            String::new()
        };
        Listing {
            installed: true,
            result: Err(format!("{detail}{hint}")),
        }
    }
}

fn read_bounded<R: Read + Send + 'static>(
    reader: R,
    limit: u64,
) -> thread::JoinHandle<(io::Result<usize>, Vec<u8>)> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.take(limit + 1).read_to_end(&mut bytes);
        (result, bytes)
    })
}

fn first_stderr_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().next().unwrap_or_default();
    let escaped = escape_controls(line);
    let mut capped: String = escaped.chars().take(MAX_STDERR_LINE_CHARS).collect();
    if escaped.chars().count() > MAX_STDERR_LINE_CHARS {
        capped.push('…');
    }
    capped
}

fn append_listing(report: &mut String, title: &str, program: &str, listing: Listing) {
    if !listing.installed {
        report.push_str(&format!("{title}: Unavailable (not installed)\n"));
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
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().collect::<String>()
            } else {
                character.to_string()
            }
        })
        .collect()
}
