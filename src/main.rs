use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use clap::{Args, Parser, Subcommand, ValueEnum};
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
struct StatusArgs {
    /// Emit JSON
    #[arg(long)]
    json: bool,
    /// Watch status continuously
    #[arg(long)]
    watch: bool,
    /// Watch interval in seconds
    #[arg(long, default_value_t = 1)]
    interval: u64,
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
    /// Shell to generate completions for
    shell: Shell,
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
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "JSON output is not implemented yet (#7)",
                ));
            }
            let path = args.path.unwrap_or_else(|| PathBuf::from("."));
            run_analyzer(path)
        }
        Command::Status(_) => not_implemented("status", 37),
        Command::Clean(_) => not_implemented("clean", 16),
        Command::Purge(_) => not_implemented("purge", 31),
        Command::Installer(_) => not_implemented("installer", 33),
        Command::Uninstall(_) => not_implemented("uninstall", 34),
        Command::Optimize(_) => not_implemented("optimize", 39),
        Command::History(_) => not_implemented("history", 12),
        Command::Completion(_) => not_implemented("completion", 41),
    }
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
