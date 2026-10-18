use mole_linux::status::Collector;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

fn write(root: &Path, path: &str, data: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("parent")).expect("create fixture dirs");
    fs::write(target, data).expect("write fixture");
}

fn sample(proc_root: &Path, second: bool) {
    let (cpu, rx, tx, read_sectors, write_sectors, first_ticks, second_ticks) = if second {
        (
            "cpu 140 0 70 1290 0 0 0 0 0 0\ncpu0 70 0 35 645 0 0 0 0 0 0\ncpu1 70 0 35 645 0 0 0 0 0 0\n",
            3072,
            6144,
            24,
            44,
            20,
            25,
        )
    } else {
        (
            "cpu 100 0 50 850 0 0 0 0 0 0\ncpu0 50 0 25 425 0 0 0 0 0 0\ncpu1 50 0 25 425 0 0 0 0 0 0\n",
            1024,
            2048,
            20,
            40,
            10,
            5,
        )
    };
    write(proc_root, "stat", cpu);
    write(
        proc_root,
        "net/dev",
        &format!(
            "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n eth0: {rx} 0 0 0 0 0 0 0 {tx} 0 0 0 0 0 0 0\n"
        ),
    );
    write(
        proc_root,
        "diskstats",
        &format!("8 0 sda 10 0 {read_sectors} 0 30 0 {write_sectors} 0 0 0 0\n"),
    );
    write(
        proc_root,
        "123/stat",
        &format!("123 (steady-worker) S 1 0 0 0 0 0 0 0 0 0 {first_ticks} 0 0 0 0 0 0 0 0 0\n"),
    );
    write(
        proc_root,
        "124/stat",
        &format!("124 (busy-worker) S 1 0 0 0 0 0 0 0 0 0 {second_ticks} 0 0 0 0 0 0 0 0 0\n"),
    );
}

#[test]
fn warm_collector_reports_unknown_first_sample_then_counter_rates_and_cpu_percent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    let sys_root = temp.path().join("sys");
    sample(&proc_root, false);
    let mut collector = Collector::new(&proc_root, &sys_root);
    let start = Instant::now();

    let first = collector.collect_at(start);
    assert_eq!(first.cpu.usage_percent, None);
    assert_eq!(first.cpu.per_core_ticks[0].usage_percent, None);
    assert_eq!(first.network.interfaces[0].rx_bytes_per_sec, None);
    assert_eq!(first.disk_io.devices[0].read_bytes_per_sec, None);
    assert_eq!(first.processes.top[0].cpu_percent, None);

    sample(&proc_root, true);
    let second = collector.collect_at(start + Duration::from_secs(2));
    assert_eq!(second.cpu.usage_percent, Some(12.0));
    assert_eq!(second.cpu.per_core_ticks[0].usage_percent, Some(12.0));
    assert_eq!(second.network.interfaces[0].rx_bytes_per_sec, Some(1024));
    assert_eq!(second.network.interfaces[0].tx_bytes_per_sec, Some(2048));
    assert_eq!(second.disk_io.devices[0].read_bytes_per_sec, Some(1024));
    assert_eq!(second.disk_io.devices[0].write_bytes_per_sec, Some(1024));
    assert_eq!(second.processes.top[0].pid, 124);
    assert_eq!(second.processes.top[0].cpu_percent, Some(8.0));
    assert_eq!(second.processes.top[1].cpu_percent, Some(4.0));
}

#[test]
fn counter_resets_make_rates_unknown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    let sys_root = temp.path().join("sys");
    sample(&proc_root, false);
    let mut collector = Collector::new(&proc_root, &sys_root);
    let start = Instant::now();
    collector.collect_at(start);
    sample(&proc_root, true);
    collector.collect_at(start + Duration::from_secs(2));
    sample(&proc_root, false);

    let reset = collector.collect_at(start + Duration::from_secs(4));
    assert_eq!(reset.cpu.usage_percent, None);
    assert_eq!(reset.cpu.per_core_ticks[0].usage_percent, None);
    assert_eq!(reset.network.interfaces[0].rx_bytes_per_sec, None);
    assert_eq!(reset.disk_io.devices[0].read_bytes_per_sec, None);
    assert!(
        reset
            .processes
            .top
            .iter()
            .all(|process| process.cpu_percent.is_none())
    );
}

#[test]
fn malformed_cpu_counters_remain_unknown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    write(
        &proc_root,
        "stat",
        "cpu 100 bad 50 850\ncpu0 100 0 50 850\n",
    );
    let mut collector = Collector::new(&proc_root, &temp.path().join("sys"));

    let snapshot = collector.collect();
    assert_eq!(snapshot.cpu.total_ticks, None);
    assert_eq!(snapshot.cpu.usage_percent, None);
    assert_eq!(
        snapshot.cpu.availability,
        mole_linux::status::Availability::Unavailable
    );
}

#[test]
fn device_interface_and_battery_names_escape_controls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    let sys_root = temp.path().join("sys");
    write(&proc_root, "stat", "cpu 1 0 1 8\ncpu0 1 0 1 8\n");
    write(
        &proc_root,
        "net/dev",
        "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed|\n eth0\u{1b}[31m: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n",
    );
    write(
        &proc_root,
        "diskstats",
        "8 0 sda\u{1b}[31m 10 0 20 0 30 0 40 0 0 0 0\n",
    );
    write(
        &sys_root,
        "class/power_supply/BAT0\u{1b}[31m/type",
        "Battery\n",
    );
    write(
        &sys_root,
        "class/power_supply/BAT0\u{1b}[31m/status",
        "Charging\n",
    );

    let snapshot = Collector::new(&proc_root, &sys_root).collect();
    assert_eq!(snapshot.network.interfaces[0].name, "eth0\\u{1b}[31m");
    assert_eq!(snapshot.disk_io.devices[0].name, "sda\\u{1b}[31m");
    assert_eq!(snapshot.batteries[0].name, "BAT0\\u{1b}[31m");
}
