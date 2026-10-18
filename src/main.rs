use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use clap::CommandFactory;
use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, shells};
use mole_linux::commands::status::StatusArgs;
use mole_linux::core::fsutil::{atomic_replace, resolve_symlink_chain};
use mole_linux::{scan, tui};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "A Linux-first system cleanup and inspection tool",
    arg_required_else_help = false
)]
struct Cli {
    /// Enable debug output
    #[arg(long, global = true)]
    debug: bool,

    /// Path to analyze when no subcommand is provided
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze disk usage at PATH
    Analyze(AnalyzeArgs),
    /// Show system status
    Status(StatusArgs),
    /// Clean supported caches
    Clean(CleanArgs),
    /// Purge development artifacts
    Purge(PurgeArgs),
    /// Find installer files
    Installer(InstallerArgs),
    /// Uninstall applications
    Uninstall(UninstallArgs),
    /// Optimize the system
    Optimize(OptimizeArgs),
    /// Show operation history
    History(HistoryArgs),
    /// Generate shell completion
    Completion(CompletionArgs),
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Directory to analyze
    path: Option<PathBuf>,
    /// Emit JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct CleanArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    categories: Option<String>,
    #[arg(long)]
    select: Option<String>,
    #[arg(long)]
    exclude: Option<String>,
    #[arg(long)]
    external: bool,
    #[arg(long)]
    whitelist: bool,
}

#[derive(Debug, Args)]
struct PurgeArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    paths: Option<String>,
    #[arg(long)]
    include_empty: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct InstallerArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    max_depth: Option<u32>,
    #[arg(long)]
    hidden: bool,
    #[arg(long)]
    r#type: Option<String>,
}

#[derive(Debug, Args)]
struct UninstallArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    list: bool,
    #[arg(long)]
    permanent: bool,
    #[arg(long)]
    whitelist: bool,
}

#[derive(Debug, Args)]
struct OptimizeArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    whitelist: bool,
}

#[derive(Debug, Args)]
struct HistoryArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct CompletionArgs {
    /// Shell to generate completions for (prints to stdout)
    shell: Option<Shell>,
    /// Remove the managed completion block from the detected shell config
    #[arg(long, conflicts_with = "shell")]
    remove: bool,
    /// Preview an rc-file update or removal without writing
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("mole-linux: {error}");
        std::process::exit(2);
    }
}

fn run() -> io::Result<()> {
    let cli = Cli::parse();
    let _debug = cli.debug;
    let command = cli.command.unwrap_or(Command::Analyze(AnalyzeArgs {
        path: cli.path,
        json: false,
    }));

    match command {
        Command::Analyze(args) => {
            if args.json {
                let cancel = AtomicBool::new(false);
                let json = if let Some(path) = args.path {
                    let report = scan::scan(&path, &cancel);
                    scan::report_json(&report, &path)
                } else {
                    let roots = scan::OverviewRoots::from_env().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid")
                    })?;
                    let report = scan::scan_overview(&roots, &cancel);
                    scan::overview_json(&report)
                }
                .map_err(|error| io::Error::other(format!("failed to encode JSON: {error}")))?;
                let mut stdout = io::stdout().lock();
                writeln!(stdout, "{json}")?;
                return Ok(());
            }
            match args.path {
                Some(path) => run_analyzer(path),
                None => run_overview(),
            }
        }
        Command::Status(args) => mole_linux::commands::status::run(args),
        Command::Clean(_) => not_implemented("clean", 16),
        Command::Purge(_) => not_implemented("purge", 31),
        Command::Installer(_) => not_implemented("installer", 33),
        Command::Uninstall(_) => not_implemented("uninstall", 34),
        Command::Optimize(_) => not_implemented("optimize", 39),
        Command::History(_) => not_implemented("history", 12),
        Command::Completion(args) => run_completion(args),
    }
}

const COMPLETION_START: &str = "# >>> mole-linux completion >>>";
const COMPLETION_END: &str = "# <<< mole-linux completion <<<";

fn run_completion(args: CompletionArgs) -> io::Result<()> {
    if let Some(shell) = args.shell {
        let mut command = Cli::command();
        let mut completion = Vec::new();
        match shell {
            Shell::Bash => generate(shells::Bash, &mut command, "mole-linux", &mut completion),
            Shell::Zsh => generate(shells::Zsh, &mut command, "mole-linux", &mut completion),
            Shell::Fish => generate(shells::Fish, &mut command, "mole-linux", &mut completion),
        }
        if let Err(error) = io::stdout().lock().write_all(&completion) {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(io::Error::new(
                error.kind(),
                format!("failed to write completion to stdout: {error}"),
            ));
        }
        return Ok(());
    }

    let shell = detected_shell()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let rc_path = match shell {
        Shell::Bash => {
            let profile = home.join(".bash_profile");
            match std::fs::symlink_metadata(&profile) {
                Ok(_) => profile,
                Err(error) if error.kind() == io::ErrorKind::NotFound => home.join(".bashrc"),
                Err(error) => return Err(error),
            }
        }
        Shell::Zsh => home.join(".zshrc"),
        Shell::Fish => home.join(".config/fish/config.fish"),
    };
    let rc_path = resolve_symlink_chain(&rc_path)?;
    let existing = match std::fs::read_to_string(&rc_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let updated = replace_completion_block(&existing, shell, args.remove)?;

    if args.dry_run {
        print!("{}", completion_diff(&rc_path, &existing, &updated));
        return Ok(());
    }
    if updated != existing {
        if let Some(parent) = rc_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_replace(&rc_path, updated.as_bytes())?;
    }
    println!("Updated {rc_path:?}");
    Ok(())
}

fn completion_diff(path: &std::path::Path, before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.split_inclusive('\n').collect();
    let after_lines: Vec<&str> = after.split_inclusive('\n').collect();
    let mut prefix = 0;
    while prefix < before_lines.len()
        && prefix < after_lines.len()
        && before_lines[prefix] == after_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before_lines.len() - prefix
        && suffix < after_lines.len() - prefix
        && before_lines[before_lines.len() - suffix - 1]
            == after_lines[after_lines.len() - suffix - 1]
    {
        suffix += 1;
    }

    let mut diff = format!("Would update {path:?}\n");
    for line in &before_lines[prefix..before_lines.len() - suffix] {
        diff.push('-');
        diff.push_str(line);
        if !line.ends_with('\n') {
            diff.push('\n');
        }
    }
    for line in &after_lines[prefix..after_lines.len() - suffix] {
        diff.push('+');
        diff.push_str(line);
        if !line.ends_with('\n') {
            diff.push('\n');
        }
    }
    if prefix == before_lines.len() && prefix == after_lines.len() {
        diff.push_str("(no changes)\n");
    }
    diff
}

fn detected_shell() -> io::Result<Shell> {
    let name = std::env::var("SHELL")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "SHELL is not set"))?;
    match std::path::Path::new(&name)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some("bash") => Ok(Shell::Bash),
        Some("zsh") => Ok(Shell::Zsh),
        Some("fish") => Ok(Shell::Fish),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported shell in SHELL: {name}"),
        )),
    }
}

// Ported from tw93/Mole bin/completion.sh auto-install path (GPL-3.0): preserve unrelated rc lines and manage only our marked block.
fn replace_completion_block(existing: &str, shell: Shell, remove: bool) -> io::Result<String> {
    let mut output = String::with_capacity(existing.len());
    let mut in_block = false;
    for line in existing.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == COMPLETION_START {
            if in_block {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "nested completion markers",
                ));
            }
            in_block = true;
            continue;
        }
        if trimmed == COMPLETION_END {
            if !in_block {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "completion end marker without start",
                ));
            }
            in_block = false;
            continue;
        }
        if !in_block {
            output.push_str(line);
        }
    }
    if in_block {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated completion markers",
        ));
    }
    if remove {
        return Ok(output);
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(COMPLETION_START);
    output.push('\n');
    match shell {
        Shell::Bash | Shell::Zsh => {
            output.push_str("eval \"$(mole-linux completion ");
            output.push_str(if matches!(shell, Shell::Bash) {
                "bash"
            } else {
                "zsh"
            });
            output.push_str(" 2>/dev/null)\"\n");
        }
        Shell::Fish => output.push_str("mole-linux completion fish | source\n"),
    }
    output.push_str(COMPLETION_END);
    output.push('\n');
    Ok(output)
}

fn not_implemented(command: &str, issue: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{command} is not implemented yet (#{issue})"),
    ))
}

fn run_analyzer(path: PathBuf) -> io::Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_path = path.clone();
    let (report_tx, report_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("mole-scan".to_owned())
        .spawn(move || {
            scan::scan_progressive(&worker_path, &worker_cancel, report_tx);
            Ok(())
        })?;

    let tui_result = tui::run(&path, report_rx, Arc::clone(&cancel));
    cancel.store(true, Ordering::Relaxed);
    let worker_result = worker
        .join()
        .map_err(|_| io::Error::other("scanner worker panicked"))?;
    tui_result?;
    worker_result
}

fn run_overview() -> io::Result<()> {
    let roots = scan::OverviewRoots::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set or invalid"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (report_tx, report_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("mole-overview".to_owned())
        .spawn(move || {
            let report = scan::scan_overview(&roots, &worker_cancel);
            report_tx
                .send(scan::ScanEvent::Initial(report.root.clone()))
                .map_err(io::Error::other)?;
            report_tx
                .send(scan::ScanEvent::Complete(report))
                .map_err(io::Error::other)?;
            Ok(())
        })?;
    let root_path = PathBuf::from("/");
    let tui_result = tui::run(&root_path, report_rx, Arc::clone(&cancel));
    cancel.store(true, Ordering::Relaxed);
    let worker_result = worker
        .join()
        .map_err(|_| io::Error::other("overview worker panicked"))?;
    tui_result?;
    worker_result
}
