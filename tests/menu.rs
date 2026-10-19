use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn no_args_opens_menu_and_q_restores_terminal_in_tmux() {
    if Command::new("tmux").arg("-V").output().is_err() {
        return;
    }
    let session = format!("mole-menu-{}", std::process::id());
    let binary = env!("CARGO_BIN_EXE_mole-linux");
    let started = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-x",
            "100",
            "-y",
            "24",
            "-s",
            &session,
            binary,
        ])
        .status()
        .expect("start tmux session");
    assert!(started.success(), "tmux session failed to start");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pane = String::new();
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture menu pane");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("Q Quit") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(pane.contains("Q Quit"), "menu did not render:\n{pane}");
    assert!(pane.contains("Clean"));
    assert!(pane.contains("Analyze"));
    let sent = Command::new("tmux")
        .args(["send-keys", "-l", "-t", &session, "/sta"])
        .status()
        .expect("type menu search");
    assert!(sent.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture filtered menu");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("Commands — search: sta") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        pane.contains("Commands — search: sta"),
        "query not visible:\n{pane}"
    );
    assert!(pane.contains("Status  Monitor system health"));
    assert!(!pane.contains("Clean  Free up disk space"));
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &session, "Escape"])
        .status()
        .expect("clear menu search");
    assert!(sent.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture cleared menu");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("Clean  Free up disk space") && !pane.contains("search: sta") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(pane.contains("Clean  Free up disk space"));
    assert!(!pane.contains("search: sta"));
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &session, "q"])
        .status()
        .expect("send quit key");
    assert!(sent.success());

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let exists = Command::new("tmux")
            .args(["has-session", "-t", &session])
            .output()
            .is_ok_and(|output| output.status.success());
        if !exists {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .status();
    panic!("menu did not exit after q");
}

#[test]
fn sigterm_restores_terminal_before_menu_process_exits() {
    if Command::new("tmux").arg("-V").output().is_err() {
        return;
    }
    let session = format!("mole-menu-term-{}", std::process::id());
    let binary = env!("CARGO_BIN_EXE_mole-linux");
    let wrapper = format!(
        "before=$(stty -g); '{}' ; after=$(stty -g); [ \"$before\" = \"$after\" ] && echo TERMINAL_RESTORED; sleep 2",
        binary.replace('\'', "'\\''")
    );
    let started = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-x",
            "100",
            "-y",
            "24",
            "-s",
            &session,
            "sh",
            "-c",
            &wrapper,
        ])
        .status()
        .expect("start tmux signal session");
    assert!(started.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pane = String::new();
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture signal menu pane");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("Q Quit") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(pane.contains("Q Quit"), "menu did not start:\n{pane}");
    let shell_pid = Command::new("tmux")
        .args(["display-message", "-p", "-t", &session, "#{pane_pid}"])
        .output()
        .expect("read pane pid");
    let shell_pid = String::from_utf8_lossy(&shell_pid.stdout).trim().to_owned();
    let child = Command::new("pgrep")
        .args(["-P", &shell_pid])
        .output()
        .expect("find menu process");
    let menu_pid = String::from_utf8_lossy(&child.stdout)
        .lines()
        .next()
        .expect("menu child pid")
        .to_owned();
    let signaled = Command::new("kill")
        .args(["-TERM", &menu_pid])
        .status()
        .expect("send TERM to menu");
    assert!(signaled.success());

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture restored terminal marker");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("TERMINAL_RESTORED") {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &session])
                .output();
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();
    panic!("SIGTERM did not restore terminal settings:\n{pane}");
}

#[test]
fn ctrl_c_restores_terminal_before_menu_process_exits() {
    if Command::new("tmux").arg("-V").output().is_err() {
        return;
    }
    let session = format!("mole-menu-int-{}", std::process::id());
    let binary = env!("CARGO_BIN_EXE_mole-linux");
    let wrapper = format!(
        "before=$(stty -g); '{}' ; after=$(stty -g); [ \"$before\" = \"$after\" ] && echo TERMINAL_RESTORED; sleep 2",
        binary.replace('\'', "'\\''")
    );
    let started = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-x",
            "100",
            "-y",
            "24",
            "-s",
            &session,
            "sh",
            "-c",
            &wrapper,
        ])
        .status()
        .expect("start tmux interrupt session");
    assert!(started.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pane = String::new();
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture interrupt menu pane");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("Q Quit") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(pane.contains("Q Quit"), "menu did not start:\n{pane}");
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &session, "C-c"])
        .status()
        .expect("send Ctrl-C to menu");
    assert!(sent.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &session])
            .output()
            .expect("capture restored terminal marker");
        pane = String::from_utf8_lossy(&output.stdout).into_owned();
        if pane.contains("TERMINAL_RESTORED") {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &session])
                .output();
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();
    panic!("Ctrl-C did not restore terminal settings:\n{pane}");
}
