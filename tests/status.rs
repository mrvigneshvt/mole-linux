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
            "1 0 8:1 / {} rw - ext4 /dev/sda1 rw\n2 0 0:1 / /proc rw - proc proc rw\n",
            temp.path().display()
        ),
    );

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
    assert_eq!(snapshot.disks.mounts.len(), 1);
    assert_eq!(snapshot.disks.mounts[0].filesystem, "ext4");
    assert_eq!(snapshot.cpu_pressure.some_avg10, Some(1.0));
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
