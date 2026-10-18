use mole_linux::status::{Availability, collect_from};
use std::fs;
use std::path::Path;

fn write(root: &Path, path: &str, data: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("parent")).expect("create fixture dirs");
    fs::write(target, data).expect("write fixture");
}

#[test]
fn injected_proc_files_produce_metrics_and_missing_data_stays_unknown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    let sys_root = temp.path().join("sys");
    write(
        &proc_root,
        "stat",
        "cpu 100 0 50 850 0 0 0 0 0 0\ncpu0 60 0 20 420 0 0 0 0 0 0\ncpu1 40 0 30 430 0 0 0 0 0 0\nprocs_running 2\nprocs_blocked 1\n",
    );
    write(&proc_root, "loadavg", "1.25 0.50 0.25 2/100 500\n");
    write(
        &proc_root,
        "meminfo",
        "MemTotal: 1000 kB\nMemAvailable: 250 kB\nSwapTotal: 400 kB\nSwapFree: 300 kB\n",
    );
    write(
        &proc_root,
        "pressure/cpu",
        "some avg10=1.00 avg60=0.50 avg300=0.10 total=123\n",
    );
    write(
        &proc_root,
        "net/dev",
        "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 10 1 0 0 0 0 0 0 20 2 0 0 0 0 0 0\n",
    );
    write(
        &proc_root,
        "diskstats",
        "8 0 sda 10 0 20 0 30 0 40 0 0 0 0\n",
    );
    write(
        &proc_root,
        "123/stat",
        "123 (worker task) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    );
    write(
        &proc_root,
        "124/stat",
        "124 (zombie) Z 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    );
    write(&proc_root, "123/cmdline", "worker\0--flag\0");
    write(
        &proc_root,
        "self/mountinfo",
        &format!(
            "1 0 8:1 / {} rw - ext4 /dev/mapper/root rw\n2 0 8:1 / {} rw - ext4 /dev/mapper/root rw\n3 0 0:1 / /proc rw - proc proc rw\n4 0 0:2 / /fuse rw - fuse.portal portal rw\n5 0 8:2 / {} rw - ext4 /dev/sdb1 rw\n6 0 8:1 / {} rw - btrfs /dev/mapper/root rw\n",
            temp.path().display(),
            temp.path().join("child").display(),
            temp.path().join("other").display(),
            temp.path().join("alternate-fs").display()
        ),
    );
    fs::create_dir_all(temp.path().join("child")).expect("nested mount fixture");
    fs::create_dir_all(temp.path().join("other")).expect("separate device fixture");
    fs::create_dir_all(temp.path().join("alternate-fs")).expect("separate filesystem fixture");
    write(
        &sys_root,
        "class/thermal/thermal_zone0/type",
        "x86_pkg_temp\n",
    );
    write(&sys_root, "class/thermal/thermal_zone0/temp", "42000\n");
    write(
        &sys_root,
        "class/thermal/cooling_device0/type",
        "Processor\n",
    );
    write(&sys_root, "class/thermal/cooling_device0/temp", "0\n");

    let snapshot = collect_from(&proc_root, &sys_root);
    assert_eq!(snapshot.cpu.availability, Availability::Available);
    assert_eq!(snapshot.cpu.logical_cores, Some(2));
    assert_eq!(snapshot.memory.used_kb, Some(750));
    assert_eq!(snapshot.memory.swap_used_kb, Some(100));
    assert_eq!(snapshot.load.one, Some(1.25));
    assert_eq!(snapshot.processes.zombies, Some(1));
    assert_eq!(snapshot.network.interfaces[0].name, "lo");
    assert_eq!(snapshot.network.interfaces[0].received_bytes, Some(10));
    assert_eq!(snapshot.disk_io.devices[0].name, "sda");
    assert_eq!(snapshot.disks.mounts.len(), 3);
    let root_mount = snapshot
        .disks
        .mounts
        .iter()
        .find(|mount| mount.source == "/dev/mapper/root" && mount.filesystem == "ext4")
        .expect("deduplicated ext4 mount");
    assert_eq!(root_mount.mountpoint, temp.path().to_string_lossy());
    assert_eq!(
        root_mount.aliases,
        vec![temp.path().join("child").to_string_lossy()]
    );
    assert!(
        snapshot
            .disks
            .mounts
            .iter()
            .any(|mount| { mount.source == "/dev/mapper/root" && mount.filesystem == "btrfs" })
    );
    assert_eq!(snapshot.thermal.len(), 1);
    assert_eq!(
        snapshot.thermal[0].sensor_type.as_deref(),
        Some("x86_pkg_temp")
    );
    let report = mole_linux::status::text_report(&snapshot);
    assert!(report.contains("Disk ("), "{report}");
    assert!(report.contains(" GB"), "{report}");
    assert_eq!(snapshot.cpu_pressure.some_avg10, Some(1.0));
}

#[test]
fn text_report_uses_snapshot_zombie_count_and_human_units() {
    use mole_linux::status::{format_bytes_gb, format_kib_gib, text_report};
    let temp = tempfile::tempdir().expect("tempdir");
    let proc_root = temp.path().join("proc");
    write(
        &proc_root,
        "stat",
        "cpu 1 0 0 1 0 0 0 0 0 0\ncpu0 1 0 0 1 0 0 0 0 0 0\n",
    );
    write(
        &proc_root,
        "124/stat",
        "124 (zombie) Z 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    );
    let snapshot = collect_from(&proc_root, &temp.path().join("sys"));
    let report = text_report(&snapshot);
    let json = serde_json::to_value(&snapshot).expect("snapshot serializes");
    assert!(report.contains("Processes: 1 zombies"), "{report}");
    assert_eq!(json["processes"]["zombies"], 1);
    assert!(
        report.contains("Memory: unknown GiB / unknown GiB"),
        "{report}"
    );
    assert!(report.contains("Disk: unknown GB / unknown GB"), "{report}");
    assert_eq!(format_kib_gib(Some(1_572_864)), "1.50 GiB");
    assert_eq!(format_bytes_gb(Some(1_500_000_000)), "1.50 GB");
}

#[test]
fn missing_or_malformed_proc_data_is_unknown_not_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot = collect_from(
        &temp.path().join("absent-proc"),
        &temp.path().join("absent-sys"),
    );
    assert_eq!(snapshot.cpu.availability, Availability::Unavailable);
    assert_eq!(snapshot.memory.total_kb, None);
    assert_eq!(snapshot.processes.zombies, None);
}
