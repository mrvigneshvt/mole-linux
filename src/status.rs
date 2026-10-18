use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Partial,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub cpu: Cpu,
    pub memory: Memory,
    pub load: Load,
    pub cpu_pressure: Pressure,
    pub memory_pressure: Pressure,
    pub io_pressure: Pressure,
    pub network: Network,
    pub disks: Disks,
    pub disk_io: DiskIo,
    pub processes: Processes,
    pub high_cpu_alerts: Vec<HighCpuAlert>,
    pub batteries: Vec<Battery>,
    pub thermal: Vec<Thermal>,
}
#[derive(Debug, Serialize)]
pub struct Cpu {
    pub availability: Availability,
    pub logical_cores: Option<usize>,
    pub total_ticks: Option<u64>,
    pub idle_ticks: Option<u64>,
    pub usage_percent: Option<f64>,
    pub per_core_ticks: Vec<CoreTicks>,
}
#[derive(Debug, Serialize)]
pub struct CoreTicks {
    pub name: String,
    pub total: u64,
    pub idle: u64,
    pub usage_percent: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct Memory {
    pub total_kb: Option<u64>,
    pub used_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
    pub swap_used_kb: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct Load {
    pub one: Option<f64>,
    pub five: Option<f64>,
    pub fifteen: Option<f64>,
    pub running: Option<u64>,
    pub total: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct Pressure {
    pub availability: Availability,
    pub some_avg10: Option<f64>,
    pub full_avg10: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct Network {
    pub interfaces: Vec<Interface>,
    pub availability: Availability,
}
#[derive(Debug, Serialize)]
pub struct Interface {
    pub name: String,
    pub received_bytes: Option<u64>,
    pub transmitted_bytes: Option<u64>,
    pub rx_bytes_per_sec: Option<u64>,
    pub tx_bytes_per_sec: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct DiskIo {
    pub devices: Vec<DeviceIo>,
    pub availability: Availability,
}
#[derive(Debug, Serialize)]
pub struct Disks {
    pub mounts: Vec<MountUsage>,
    pub availability: Availability,
}
#[derive(Debug, Serialize)]
pub struct MountUsage {
    pub source: String,
    pub filesystem: String,
    pub mountpoint: String,
    pub aliases: Vec<String>,
    pub total_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct DeviceIo {
    pub name: String,
    pub reads: u64,
    pub writes: u64,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub read_bytes_per_sec: Option<u64>,
    pub write_bytes_per_sec: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct Processes {
    pub zombies: Option<usize>,
    pub top: Vec<Process>,
}
#[derive(Debug, Serialize)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
    pub state: char,
    pub cpu_ticks: u64,
    pub cpu_percent: Option<f64>,
}
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct HighCpuAlert {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
}
#[derive(Debug)]
struct ProcessWindow {
    ppid: u32,
    command: String,
    ticks: u64,
    since: Instant,
}
#[derive(Debug, Default)]
pub struct StatusTracker {
    previous_total: Option<u64>,
    previous: HashMap<u32, ProcessWindow>,
}

#[derive(Debug)]
pub struct Collector {
    proc_root: PathBuf,
    sys_root: PathBuf,
    previous: Option<CounterSample>,
    tracker: StatusTracker,
}

#[derive(Debug)]
struct CounterSample {
    collected_at: Instant,
    cpu_total: Option<u64>,
    cpu_idle: Option<u64>,
    cores: HashMap<String, (u64, u64)>,
    disks: HashMap<String, (Option<u64>, Option<u64>)>,
    network: HashMap<String, (Option<u64>, Option<u64>)>,
    processes: HashMap<u32, (u32, String, u64)>,
}

impl Collector {
    pub fn system() -> Self {
        Self::new(Path::new("/proc"), Path::new("/sys"))
    }

    pub fn new(proc_root: &Path, sys_root: &Path) -> Self {
        Self {
            proc_root: proc_root.to_owned(),
            sys_root: sys_root.to_owned(),
            previous: None,
            tracker: StatusTracker::default(),
        }
    }

    pub fn collect(&mut self) -> Snapshot {
        self.collect_at(Instant::now())
    }

    pub fn collect_at(&mut self, now: Instant) -> Snapshot {
        // Ported from tw93/Mole cmd/status/metrics.go:NewCollector (GPL-3.0): keep one collector's samples warm across frames.
        let mut snapshot = collect_snapshot(&self.proc_root, &self.sys_root, true);
        let current = CounterSample::from_snapshot(&snapshot, now);
        if let Some(previous) = &self.previous {
            apply_rates(previous, &mut snapshot, now);
        }
        self.tracker.update(&mut snapshot, now);
        self.previous = Some(current);
        snapshot
    }
}

impl CounterSample {
    fn from_snapshot(snapshot: &Snapshot, collected_at: Instant) -> Self {
        Self {
            collected_at,
            cpu_total: snapshot.cpu.total_ticks,
            cpu_idle: snapshot.cpu.idle_ticks,
            cores: snapshot
                .cpu
                .per_core_ticks
                .iter()
                .map(|core| (core.name.clone(), (core.total, core.idle)))
                .collect(),
            disks: snapshot
                .disk_io
                .devices
                .iter()
                .map(|device| (device.name.clone(), (device.read_bytes, device.write_bytes)))
                .collect(),
            network: snapshot
                .network
                .interfaces
                .iter()
                .map(|interface| {
                    (
                        interface.name.clone(),
                        (interface.received_bytes, interface.transmitted_bytes),
                    )
                })
                .collect(),
            processes: snapshot
                .processes
                .top
                .iter()
                .map(|process| {
                    (
                        process.pid,
                        (process.ppid, process.command.clone(), process.cpu_ticks),
                    )
                })
                .collect(),
        }
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::system()
    }
}

fn apply_rates(previous: &CounterSample, snapshot: &mut Snapshot, now: Instant) {
    // Linux counter-delta adaptation of tw93/Mole cmd/status/metrics_disk.go:collectDiskIO and metrics_network.go:collectNetwork (GPL-3.0).
    let elapsed = now.saturating_duration_since(previous.collected_at);
    let total_delta = snapshot
        .cpu
        .total_ticks
        .zip(previous.cpu_total)
        .and_then(|(current, old)| current.checked_sub(old));
    let idle_delta = snapshot
        .cpu
        .idle_ticks
        .zip(previous.cpu_idle)
        .and_then(|(current, old)| current.checked_sub(old));
    snapshot.cpu.usage_percent = total_delta
        .zip(idle_delta)
        .and_then(|(total, idle)| total.checked_sub(idle).map(|busy| percent(busy, total, 1)))
        .flatten();
    for core in &mut snapshot.cpu.per_core_ticks {
        core.usage_percent = previous
            .cores
            .get(&core.name)
            .and_then(|(old_total, old_idle)| {
                core.total
                    .checked_sub(*old_total)
                    .zip(core.idle.checked_sub(*old_idle))
            })
            .and_then(|(total, idle)| total.checked_sub(idle).map(|busy| percent(busy, total, 1)))
            .flatten();
    }
    for device in &mut snapshot.disk_io.devices {
        if let Some((old_read, old_write)) = previous.disks.get(&device.name) {
            device.read_bytes_per_sec = byte_rate(*old_read, device.read_bytes, elapsed);
            device.write_bytes_per_sec = byte_rate(*old_write, device.write_bytes, elapsed);
        }
    }
    for interface in &mut snapshot.network.interfaces {
        if let Some((old_rx, old_tx)) = previous.network.get(&interface.name) {
            interface.rx_bytes_per_sec = byte_rate(*old_rx, interface.received_bytes, elapsed);
            interface.tx_bytes_per_sec = byte_rate(*old_tx, interface.transmitted_bytes, elapsed);
        }
    }
    let cores = snapshot
        .cpu
        .logical_cores
        .and_then(|count| u64::try_from(count).ok());
    for process in &mut snapshot.processes.top {
        process.cpu_percent = previous
            .processes
            .get(&process.pid)
            .filter(|(ppid, command, _)| *ppid == process.ppid && *command == process.command)
            .and_then(|(_, _, old_ticks)| process.cpu_ticks.checked_sub(*old_ticks))
            .zip(total_delta)
            .and_then(|(used, total)| percent(used, total, cores?));
    }
    snapshot.processes.top.sort_by(|left, right| {
        right
            .cpu_percent
            .unwrap_or(-1.0)
            .total_cmp(&left.cpu_percent.unwrap_or(-1.0))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    snapshot.processes.top.truncate(10);
}

fn percent(numerator: u64, denominator: u64, multiplier: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    let basis_points = numerator
        .checked_mul(multiplier)?
        .checked_mul(10_000)?
        .checked_div(denominator)?;
    Some(f64::from(u32::try_from(basis_points).ok()?) / 100.0)
}

fn byte_rate(previous: Option<u64>, current: Option<u64>, elapsed: Duration) -> Option<u64> {
    let delta = current?.checked_sub(previous?)?;
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return None;
    }
    u128::from(delta)
        .checked_mul(1_000_000_000)?
        .checked_div(nanos)?
        .try_into()
        .ok()
}
impl StatusTracker {
    pub fn update(&mut self, snapshot: &mut Snapshot, now: Instant) {
        let Some(total) = snapshot.cpu.total_ticks else {
            self.previous.clear();
            self.previous_total = None;
            return;
        };
        let cores = snapshot
            .cpu
            .logical_cores
            .and_then(|count| u64::try_from(count).ok());
        let elapsed_ticks = self.previous_total.and_then(|v| total.checked_sub(v));
        let mut current = HashMap::new();
        let mut alerts = Vec::new();
        for process in &snapshot.processes.top {
            let prior = self.previous.remove(&process.pid);
            let valid = prior
                .as_ref()
                .is_some_and(|p| p.ppid == process.ppid && p.command == process.command);
            let cpu_ticks = prior
                .as_ref()
                .filter(|_| valid)
                .and_then(|p| process.cpu_ticks.checked_sub(p.ticks));
            let over = cpu_ticks.zip(elapsed_ticks).is_some_and(|(used, total)| {
                total > 0
                    && cores
                        .and_then(|count| used.checked_mul(count))
                        .is_some_and(|scaled| scaled >= total)
            });
            let since = if over {
                prior.filter(|_| valid).map_or(now, |p| p.since)
            } else {
                now
            };
            if over && now.saturating_duration_since(since) >= Duration::from_secs(300) {
                alerts.push(HighCpuAlert {
                    pid: process.pid,
                    ppid: process.ppid,
                    command: process.command.clone(),
                });
            }
            current.insert(
                process.pid,
                ProcessWindow {
                    ppid: process.ppid,
                    command: process.command.clone(),
                    ticks: process.cpu_ticks,
                    since,
                },
            );
        }
        self.previous = current;
        self.previous_total = Some(total);
        snapshot.high_cpu_alerts = alerts;
    }
}
#[derive(Debug, Serialize)]
pub struct Battery {
    pub name: String,
    pub status: Option<String>,
    pub capacity_percent: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct Thermal {
    pub name: String,
    pub sensor_type: Option<String>,
    pub millicelsius: Option<i64>,
}

pub fn collect() -> Snapshot {
    Collector::system().collect()
}

// Ported from tw93/Mole lib/check/health_json.sh:get_memory_info (GPL-3.0), using Linux meminfo units.
pub fn collect_from(proc_root: &Path, sys_root: &Path) -> Snapshot {
    Collector::new(proc_root, sys_root).collect()
}

fn collect_snapshot(proc_root: &Path, sys_root: &Path, keep_all_processes: bool) -> Snapshot {
    let stat = read(proc_root, "stat");
    let (cpu, zombies) = parse_stat(&stat);
    let memory = parse_meminfo(&read(proc_root, "meminfo"));
    let load = parse_load(&read(proc_root, "loadavg"));
    let cpu_pressure = parse_pressure(&read(proc_root, "pressure/cpu"));
    let memory_pressure = parse_pressure(&read(proc_root, "pressure/memory"));
    let io_pressure = parse_pressure(&read(proc_root, "pressure/io"));
    let network = parse_network(&read(proc_root, "net/dev"));
    let disk_io = parse_diskstats(&read(proc_root, "diskstats"));
    let disks = collect_disks(proc_root);
    let processes = scan_processes(proc_root, zombies, keep_all_processes);
    Snapshot {
        cpu,
        memory,
        load,
        cpu_pressure,
        memory_pressure,
        io_pressure,
        network,
        disks,
        disk_io,
        processes,
        high_cpu_alerts: Vec::new(),
        batteries: collect_batteries(sys_root),
        thermal: collect_thermal(sys_root),
    }
}

fn read(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join(name)).unwrap_or_default()
}
fn parse_stat(raw: &str) -> (Cpu, Option<usize>) {
    let mut cores = Vec::new();
    let mut total = None;
    let mut idle_total = None;
    let mut zombie_count = None;
    for line in raw.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() == Some(&"cpu") {
            let values = fields
                .iter()
                .skip(1)
                .map(|value| value.parse::<u64>().ok())
                .collect::<Option<Vec<_>>>();
            total = values
                .as_ref()
                .and_then(|values| values.iter().copied().try_fold(0u64, u64::checked_add));
            idle_total = values.as_ref().and_then(|values| {
                values
                    .get(3)
                    .copied()
                    .zip(values.get(4).copied().or(Some(0)))
                    .and_then(|(idle, wait)| idle.checked_add(wait))
            });
        } else if fields
            .first()
            .is_some_and(|v| v.starts_with("cpu") && v[3..].parse::<usize>().is_ok())
        {
            if fields.len() >= 5 {
                let vals = fields[1..]
                    .iter()
                    .map(|value| value.parse::<u64>().ok())
                    .collect::<Option<Vec<_>>>();
                if let Some(vals) = vals.filter(|values| values.len() >= 4) {
                    let sum = vals.iter().copied().try_fold(0u64, u64::checked_add);
                    if let Some(total) = sum {
                        cores.push(CoreTicks {
                            name: fields[0].to_owned(),
                            total,
                            idle: vals[3].saturating_add(*vals.get(4).unwrap_or(&0)),
                            usage_percent: None,
                        });
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("procs_zombie ") {
            zombie_count = rest.parse::<usize>().ok();
        }
    }
    let avail = if total.is_some() {
        Availability::Available
    } else {
        Availability::Unavailable
    };
    let zombies = zombie_count;
    (
        Cpu {
            availability: avail,
            logical_cores: if cores.is_empty() {
                None
            } else {
                Some(cores.len())
            },
            total_ticks: total,
            idle_ticks: idle_total,
            usage_percent: None,
            per_core_ticks: cores,
        },
        zombies,
    )
}
fn parse_meminfo(raw: &str) -> Memory {
    let mut total = None;
    let mut available = None;
    let mut swap_total = None;
    let mut swap_free = None;
    for line in raw.lines() {
        let mut f = line.split_whitespace();
        let key = f.next().unwrap_or("").trim_end_matches(':');
        let val = f.next().and_then(|v| v.parse::<u64>().ok());
        match key {
            "MemTotal" => total = val,
            "MemAvailable" => available = val,
            "SwapTotal" => swap_total = val,
            "SwapFree" => swap_free = val,
            _ => {}
        }
    }
    Memory {
        total_kb: total,
        used_kb: total.zip(available).and_then(|(t, a)| t.checked_sub(a)),
        swap_total_kb: swap_total,
        swap_used_kb: swap_total
            .zip(swap_free)
            .and_then(|(t, f)| t.checked_sub(f)),
    }
}
fn parse_load(raw: &str) -> Load {
    let mut f = raw.split_whitespace();
    let one = f.next().and_then(|v| v.parse().ok());
    let five = f.next().and_then(|v| v.parse().ok());
    let fifteen = f.next().and_then(|v| v.parse().ok());
    let pair = f.next().and_then(|v| v.split_once('/'));
    Load {
        one,
        five,
        fifteen,
        running: pair.and_then(|p| p.0.parse().ok()),
        total: pair.and_then(|p| p.1.parse().ok()),
    }
}
fn parse_pressure(raw: &str) -> Pressure {
    let mut some = None;
    let mut full = None;
    for line in raw.lines() {
        let mut f = line.split_whitespace();
        let kind = f.next();
        let avg = f.find_map(|v| v.strip_prefix("avg10=").and_then(|x| x.parse::<f64>().ok()));
        match kind {
            Some("some") => some = avg,
            Some("full") => full = avg,
            _ => {}
        }
    }
    let availability = if some.is_some() || full.is_some() {
        Availability::Available
    } else {
        Availability::Unavailable
    };
    Pressure {
        availability,
        some_avg10: some,
        full_avg10: full,
    }
}
fn parse_network(raw: &str) -> Network {
    let mut interfaces = Vec::new();
    for line in raw.lines().skip(2) {
        if let Some((name, data)) = line.split_once(':') {
            let f: Vec<_> = data.split_whitespace().collect();
            if f.len() >= 9 {
                interfaces.push(Interface {
                    name: escape_controls(name.trim()),
                    received_bytes: f[0].parse().ok(),
                    transmitted_bytes: f[8].parse().ok(),
                    rx_bytes_per_sec: None,
                    tx_bytes_per_sec: None,
                });
            }
        }
    }
    let availability = if interfaces.is_empty() {
        Availability::Unavailable
    } else {
        Availability::Available
    };
    Network {
        interfaces,
        availability,
    }
}
fn parse_diskstats(raw: &str) -> DiskIo {
    let mut devices = Vec::new();
    for line in raw.lines() {
        let f: Vec<_> = line.split_whitespace().collect();
        if f.len() >= 14
            && let (Ok(reads), Ok(writes), Ok(read_sectors), Ok(write_sectors)) = (
                f[3].parse(),
                f[7].parse(),
                f[5].parse::<u64>(),
                f[9].parse::<u64>(),
            )
        {
            devices.push(DeviceIo {
                name: escape_controls(f[2]),
                reads,
                writes,
                read_bytes: read_sectors.checked_mul(512),
                write_bytes: write_sectors.checked_mul(512),
                read_bytes_per_sec: None,
                write_bytes_per_sec: None,
            });
        }
    }
    let availability = if devices.is_empty() {
        Availability::Unavailable
    } else {
        Availability::Available
    };
    DiskIo {
        devices,
        availability,
    }
}
fn collect_disks(proc_root: &Path) -> Disks {
    let mut mounts = Vec::new();
    let mut partial = false;
    for line in read(proc_root, "self/mountinfo").lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            partial = true;
            continue;
        };
        let left: Vec<_> = before.split_whitespace().collect();
        let right: Vec<_> = after.split_whitespace().collect();
        if left.len() < 5 || right.len() < 2 {
            partial = true;
            continue;
        }
        let filesystem = right[0];
        if is_pseudo_fs(filesystem) {
            continue;
        }
        let mountpoint = decode_mount_field(left[4]);
        let path = Path::new(&mountpoint);
        match rustix::fs::statvfs(path) {
            Ok(info) => {
                let unit = info.f_frsize;
                let total = info.f_blocks.checked_mul(unit);
                let free = info.f_bfree.checked_mul(unit);
                let avail = info.f_bavail.checked_mul(unit);
                let used = total.zip(free).and_then(|(t, f)| t.checked_sub(f));
                mounts.push(MountUsage {
                    source: decode_mount_field(right[1]),
                    filesystem: filesystem.to_owned(),
                    mountpoint: escape_controls(&mountpoint),
                    aliases: Vec::new(),
                    total_bytes: total,
                    used_bytes: used,
                    available_bytes: avail,
                });
            }
            Err(_) => partial = true,
        }
    }
    mounts.sort_by(|left, right| {
        (&left.source, &left.filesystem)
            .cmp(&(&right.source, &right.filesystem))
            .then_with(|| left.mountpoint.len().cmp(&right.mountpoint.len()))
            .then_with(|| left.mountpoint.cmp(&right.mountpoint))
    });
    let mut unique: Vec<MountUsage> = Vec::new();
    for mount in mounts {
        if let Some(primary) = unique.last_mut().filter(|primary| {
            primary.source == mount.source && primary.filesystem == mount.filesystem
        }) {
            primary.aliases.push(mount.mountpoint);
        } else {
            unique.push(mount);
        }
    }
    let mounts = unique;
    let availability = if mounts.is_empty() {
        Availability::Unavailable
    } else if partial {
        Availability::Partial
    } else {
        Availability::Available
    };
    Disks {
        mounts,
        availability,
    }
}
fn is_pseudo_fs(kind: &str) -> bool {
    matches!(
        kind,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "devpts"
            | "tmpfs"
            | "cgroup"
            | "cgroup2"
            | "securityfs"
            | "debugfs"
            | "tracefs"
            | "configfs"
            | "pstore"
            | "bpf"
            | "autofs"
            | "mqueue"
            | "hugetlbfs"
            | "fusectl"
            | "rpc_pipefs"
            | "binfmt_misc"
            | "efivarfs"
    ) || kind.starts_with("fuse.")
}
fn decode_mount_field(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let n = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(n);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn scan_processes(root: &Path, stat_zombies: Option<usize>, keep_all: bool) -> Processes {
    let mut found = Vec::new();
    let mut zombies = 0usize;
    let mut complete = true;
    let entries = match fs::read_dir(root) {
        Ok(v) => v,
        Err(_) => {
            return Processes {
                zombies: None,
                top: found,
            };
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        let raw = match fs::read_to_string(entry.path().join("stat")) {
            Ok(v) => v,
            Err(error) if is_transient_proc_disappearance(&error) => continue,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        if let Some(p) = parse_process(pid, &raw, root) {
            if p.state == 'Z' {
                zombies += 1;
            }
            found.push(p);
        } else {
            complete = false;
        }
    }
    found.sort_by_key(|p| std::cmp::Reverse(p.cpu_ticks));
    if !keep_all {
        found.truncate(10);
    }
    Processes {
        zombies: stat_zombies.or_else(|| complete.then_some(zombies)),
        top: found,
    }
}
fn is_transient_proc_disappearance(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}
fn parse_process(pid: u32, raw: &str, root: &Path) -> Option<Process> {
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    let command = &raw[open + 1..close];
    let rest = raw.get(close + 2..)?.split_whitespace().collect::<Vec<_>>();
    let state = rest.first()?.chars().next()?;
    let ppid = rest.get(1)?.parse().ok()?;
    let utime = rest.get(11)?.parse::<u64>().ok()?;
    let stime = rest.get(12)?.parse::<u64>().ok()?;
    let cpu_ticks = utime.checked_add(stime)?;
    let cmd = read_limited(&root.join(pid.to_string()).join("cmdline"), 4096)
        .filter(|v| !v.is_empty())
        .map(|v| {
            String::from_utf8_lossy(&v)
                .replace('\0', " ")
                .trim()
                .to_owned()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| command.to_owned());
    Some(Process {
        pid,
        ppid,
        command: escape_controls(&cmd),
        state,
        cpu_ticks,
        cpu_percent: None,
    })
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref().take(limit).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}
fn escape_controls(v: &str) -> String {
    v.chars().flat_map(char::escape_default).collect()
}
fn collect_batteries(root: &Path) -> Vec<Battery> {
    let mut out = Vec::new();
    let path = root.join("class/power_supply");
    if let Ok(entries) = fs::read_dir(path) {
        for e in entries.flatten() {
            let d = e.path();
            if read(&d, "type").trim() != "Battery" {
                continue;
            }
            let name = escape_controls(&e.file_name().to_string_lossy());
            let status = fs::read_to_string(d.join("status"))
                .ok()
                .map(|v| escape_controls(v.trim()));
            let cap = fs::read_to_string(d.join("capacity"))
                .ok()
                .and_then(|v| v.trim().parse().ok());
            out.push(Battery {
                name,
                status,
                capacity_percent: cap,
            });
        }
    }
    out
}
fn collect_thermal(root: &Path) -> Vec<Thermal> {
    let mut out = Vec::new();
    let path = root.join("class/thermal");
    if let Ok(entries) = fs::read_dir(path) {
        for e in entries.flatten() {
            let d = e.path();
            let Some(name) = e.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let sensor_type = fs::read_to_string(d.join("type"))
                .ok()
                .map(|v| escape_controls(v.trim()));
            let temp = fs::read_to_string(d.join("temp"))
                .ok()
                .and_then(|v| v.trim().parse().ok());
            out.push(Thermal {
                name: escape_controls(&name),
                sensor_type,
                millicelsius: temp,
            });
        }
    }
    out
}

pub fn text_report(snapshot: &Snapshot) -> String {
    let mut lines = vec![
        "System status".to_owned(),
        format!(
            "CPU: {:?} ({} cores)",
            snapshot.cpu.availability,
            snapshot
                .cpu
                .logical_cores
                .map_or_else(|| "unknown".to_owned(), |n| n.to_string())
        ),
        format!(
            "Load: {} / {} / {}",
            display(snapshot.load.one),
            display(snapshot.load.five),
            display(snapshot.load.fifteen)
        ),
        format!(
            "Memory: {} / {}",
            format_kib_gib(snapshot.memory.used_kb),
            format_kib_gib(snapshot.memory.total_kb)
        ),
        format!(
            "Swap: {} / {}",
            format_kib_gib(snapshot.memory.swap_used_kb),
            format_kib_gib(snapshot.memory.swap_total_kb)
        ),
    ];
    if let Some(disk) = snapshot
        .disks
        .mounts
        .iter()
        .find(|disk| disk.mountpoint == "/")
        .or_else(|| snapshot.disks.mounts.first())
    {
        lines.push(format!(
            "Disk ({}): {} / {}",
            disk.mountpoint,
            format_bytes_gb(disk.used_bytes),
            format_bytes_gb(disk.total_bytes)
        ));
    } else {
        lines.push("Disk: unknown GB / unknown GB".to_owned());
    }
    lines.push(format!(
        "Processes: {} zombies",
        display(snapshot.processes.zombies)
    ));
    lines.join("\n") + "\n"
}

pub fn format_kib_gib(value: Option<u64>) -> String {
    format_hundredths(value.map(u128::from), 1024 * 1024, "GiB")
}

pub fn format_bytes_gb(value: Option<u64>) -> String {
    format_hundredths(value.map(u128::from), 1_000_000_000, "GB")
}

fn format_hundredths(value: Option<u128>, denominator: u128, unit: &str) -> String {
    value.map_or_else(
        || format!("unknown {unit}"),
        |value| {
            let hundredths =
                value.saturating_mul(100).saturating_add(denominator / 2) / denominator;
            format!("{}.{:02} {unit}", hundredths / 100, hundredths % 100)
        },
    )
}

fn display<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_memory_fields_do_not_become_zero() {
        let m = parse_meminfo("MemTotal: broken kB\n");
        assert_eq!(m.total_kb, None);
        assert_eq!(m.used_kb, None);
    }

    #[test]
    fn disappearing_proc_entries_do_not_make_zombie_count_unknown() {
        assert!(is_transient_proc_disappearance(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
        assert!(!is_transient_proc_disappearance(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));
    }

    #[test]
    fn high_cpu_alert_requires_five_minutes_and_process_identity() {
        use std::fs;
        let dir = tempfile::tempdir().expect("fixture");
        let proc_root = dir.path().join("proc");
        let sys_root = dir.path().join("sys");
        let make = |total: u64, ticks: u64| {
            fs::create_dir_all(proc_root.join("123")).expect("proc dir");
            fs::write(
                proc_root.join("stat"),
                format!("cpu {total} 0 0 0 0 0 0 0 0 0\ncpu0 {total} 0 0 0 0 0 0 0 0 0\n"),
            )
            .expect("cpu stat");
            fs::write(
                proc_root.join("123/stat"),
                format!("123 (worker) S 1 0 0 0 0 0 0 0 0 0 0 {ticks} 0 0 0 0 0 0\n"),
            )
            .expect("process stat");
            collect_from(&proc_root, &sys_root)
        };
        let start = Instant::now();
        let mut tracker = StatusTracker::default();
        let mut first = make(1000, 100);
        tracker.update(&mut first, start);
        let mut second = make(1100, 200);
        tracker.update(&mut second, start + Duration::from_secs(1));
        assert!(second.high_cpu_alerts.is_empty());
        let mut third = make(41100, 40200);
        tracker.update(&mut third, start + Duration::from_secs(301));
        assert_eq!(third.high_cpu_alerts.len(), 1);
        assert_eq!(third.high_cpu_alerts[0].ppid, 1);
        fs::write(
            proc_root.join("123/stat"),
            "123 (replacement) S 1 0 0 0 0 0 0 0 0 0 0 40300 0 0 0 0 0 0\n",
        )
        .expect("replace identity");
        let mut replaced = collect_from(&proc_root, &sys_root);
        tracker.update(&mut replaced, start + Duration::from_secs(302));
        assert!(replaced.high_cpu_alerts.is_empty());
    }
}
