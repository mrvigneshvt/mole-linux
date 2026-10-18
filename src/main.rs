use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use mole_linux::{scan, tui};

fn main() {
    if let Err(error) = run() {
        eprintln!("mole-linux: {error}");
        std::process::exit(2);
    }
}

fn run() -> io::Result<()> {
    let mut arguments = env::args_os().skip(1);
    let first = arguments.next();
    match first.as_deref() {
        Some(argument) if argument == "--version" => {
            println!("mole-linux {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(argument) if argument == "--help" => {
            println!("Usage: mole-linux [PATH]\n       mole-linux --version");
            return Ok(());
        }
        Some(argument) if argument.to_string_lossy().starts_with('-') => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown option (use --help)",
            ));
        }
        _ => {}
    }

    let path = match first {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("."),
    };
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected at most one PATH argument",
        ));
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_path = path.clone();
    let (report_tx, report_rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("mole-scan".to_owned())
        .spawn(move || {
            let report = scan::scan(&worker_path, &worker_cancel);
            if report_tx.send(report).is_err() && !worker_cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("TUI closed before scan completed"));
            }
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
