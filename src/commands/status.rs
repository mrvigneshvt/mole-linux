use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use clap::Args;

use crate::{status, tui};

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit JSON
    #[arg(long)]
    json: bool,
    /// Watch status continuously as newline-delimited JSON
    #[arg(long)]
    watch: bool,
    /// Watch and dashboard refresh interval
    #[arg(long, default_value = "1s", value_parser = parse_interval)]
    interval: Duration,
}

impl StatusArgs {
    pub fn interactive() -> Self {
        Self {
            json: false,
            watch: false,
            interval: Duration::from_secs(1),
        }
    }
}

pub fn run(args: StatusArgs) -> io::Result<()> {
    // Ported from tw93/Mole cmd/status/main.go and watch.go:runWatchStdout (GPL-3.0), with the Linux collector.
    let mut collector = status::Collector::system();
    if args.watch {
        loop {
            let snapshot = collector.collect();
            if let Err(error) = print_snapshot(&snapshot, true) {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(error);
            }
            std::thread::sleep(args.interval);
        }
    }
    if !args.json && io::stdout().is_terminal() {
        return tui::run_status(collector, args.interval);
    }
    let snapshot = collector.collect();
    print_snapshot(&snapshot, args.json || !io::stdout().is_terminal())
}

fn parse_interval(raw: &str) -> Result<Duration, String> {
    let mut remaining = raw;
    let mut seconds = 0.0;
    while !remaining.is_empty() {
        let unit_start = remaining
            .find(|character: char| !character.is_ascii_digit() && character != '.')
            .ok_or_else(|| format!("invalid interval {raw:?}: include a unit such as s or ms"))?;
        let (amount, suffix) = remaining.split_at(unit_start);
        let amount = amount
            .parse::<f64>()
            .map_err(|error| format!("invalid interval {raw:?}: {error}"))?;
        let (seconds_per_unit, rest) = [
            ("ns", 1e-9),
            ("us", 1e-6),
            ("µs", 1e-6),
            ("ms", 1e-3),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 3600.0),
        ]
        .iter()
        .find_map(|(unit, multiplier)| suffix.strip_prefix(unit).map(|rest| (*multiplier, rest)))
        .ok_or_else(|| format!("invalid interval unit in {raw:?}"))?;
        seconds += amount * seconds_per_unit;
        if !seconds.is_finite() {
            return Err(format!("interval is too large: {raw:?}"));
        }
        remaining = rest;
    }
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("interval must be greater than zero: {raw:?}"));
    }
    let interval = Duration::try_from_secs_f64(seconds)
        .map_err(|error| format!("invalid interval {raw:?}: {error}"))?;
    if interval.is_zero() {
        return Err(format!("interval is below timer resolution: {raw:?}"));
    }
    Ok(interval)
}

fn print_snapshot(snapshot: &status::Snapshot, json: bool) -> io::Result<()> {
    if json {
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, snapshot).map_err(|error| {
            let kind = error.io_error_kind().unwrap_or(io::ErrorKind::Other);
            io::Error::new(kind, error)
        })?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    } else {
        io::stdout()
            .lock()
            .write_all(status::text_report(snapshot).as_bytes())?;
    }
    Ok(())
}
