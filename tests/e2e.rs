use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("slumber-{name}-{}-{nonce}", std::process::id()))
}

fn isolated(command: &mut Command, root: &Path) {
    command
        .env("SLUMBER_HOME", root.join("state"))
        .env("SLUMBER_SOCKET", root.join("slumber.sock"))
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID");
}

#[test]
fn delegates_records_and_resumes_a_job() {
    let binary = env!("CARGO_BIN_EXE_slumber");
    let root = temp_root("e2e");
    fs::create_dir_all(&root).unwrap();
    let child_pid_path = root.join("child.pid");
    let delegated = format!(
        "sleep 30 & echo $! > '{}'; printf \"$LOG_BODY\"; printf \"$ERR_BODY\" >&2; exit 7",
        child_pid_path.display()
    );

    let mut run = Command::new(binary);
    isolated(&mut run, &root);
    let output = run
        .env("LOG_BODY", "output-only-body")
        .env("ERR_BODY", "error-only-body")
        .args([
            "run",
            "--resume-template",
            "printf resumed > \"$SLUMBER_HOME/resumed\"",
        ])
        .arg(delegated)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let submission = String::from_utf8(output.stdout).unwrap();
    let job_id = submission.split_whitespace().nth(1).unwrap();
    let job_dir = root.join("state/jobs").join(job_id);

    for _ in 0..50 {
        if job_dir.join("payload.md").exists() && root.join("state/resumed").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let meta = fs::read_to_string(job_dir.join("meta.json")).unwrap();
    let stdout = fs::read_to_string(job_dir.join("stdout.log")).unwrap();
    let stderr = fs::read_to_string(job_dir.join("stderr.log")).unwrap();
    let payload = fs::read_to_string(job_dir.join("payload.md")).unwrap();
    let child_pid: i32 = fs::read_to_string(&child_pid_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut child_was_cleaned_up = false;
    for _ in 0..50 {
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None)
            == Err(nix::errno::Errno::ESRCH)
        {
            child_was_cleaned_up = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mut status = Command::new(binary);
    isolated(&mut status, &root);
    let status = status.arg("status").output().unwrap();

    let daemon_pid: i32 = fs::read_to_string(root.join("state/slumberd.pid"))
        .unwrap()
        .parse()
        .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon_pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    thread::sleep(Duration::from_millis(20));
    fs::remove_dir_all(&root).unwrap();

    assert!(meta.contains("\"Exited\": 7"));
    assert_eq!(stdout, "output-only-body");
    assert_eq!(stderr, "error-only-body");
    assert!(payload.contains("Exit Status      : exit 7"));
    assert!(!payload.contains("output-only-body"));
    assert!(!payload.contains("error-only-body"));
    assert!(child_was_cleaned_up, "background process was left running");
    assert!(String::from_utf8(status.stdout).unwrap().contains("exit 7"));
}

#[test]
fn init_is_idempotent() {
    let binary = env!("CARGO_BIN_EXE_slumber");
    let root = temp_root("init");
    fs::create_dir_all(&root).unwrap();
    let instructions = root.join("AGENTS.md");
    fs::write(&instructions, "# Existing instructions\n").unwrap();

    for _ in 0..2 {
        let output = Command::new(binary)
            .args(["init", "--file"])
            .arg(&instructions)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    let content = fs::read_to_string(&instructions).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(content.matches("<!-- SLUMBER PROTOCOL -->").count(), 1);
}

#[test]
fn remote_job_recovers_after_local_daemon_restart() {
    let binary = env!("CARGO_BIN_EXE_slumber");
    let root = temp_root("remote");
    let fake_bin = root.join("bin");
    let fake_remote = root.join("remote");
    let tmux_state = root.join("tmux");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&fake_remote).unwrap();
    fs::create_dir_all(&tmux_state).unwrap();
    let fake_ssh = fake_bin.join("ssh");
    fs::write(
        &fake_ssh,
        r#"#!/bin/sh
case "$*" in
  *"sh -s"*)
    cat > "$FAKE_SSH_HOME/wrapper.sh"
    printf '4242\n'
    ;;
  *"exit_code"*)
    test -f "$FAKE_SSH_HOME/exit_code" || exit 1
    cat "$FAKE_SSH_HOME/exit_code"
    ;;
  *"stdout.log"*) printf 'remote stdout' ;;
  *"stderr.log"*) printf 'remote stderr' ;;
  *) exit 2 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_tmux = fake_bin.join("tmux");
    fs::write(
        &fake_tmux,
        r#"#!/bin/sh
case "$1" in
  split-window)
    printf '%s\n' "$*" > "$TMUX_TEST_DIR/split"
    printf '%%9\n'
    ;;
  kill-pane)
    printf '%s\n' "$*" > "$TMUX_TEST_DIR/killed"
    ;;
  *) exit 2 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());

    let configure = |command: &mut Command| {
        isolated(command, &root);
        command
            .env("PATH", &fake_path)
            .env("FAKE_SSH_HOME", &fake_remote)
            .env("TMUX", "/tmp/fake-remote-tmux,456,0")
            .env("TMUX_PANE", "%2")
            .env("TMUX_TEST_DIR", &tmux_state);
    };

    let mut run = Command::new(binary);
    configure(&mut run);
    run.env("REMOTE_TEST_SECRET", "request-secret")
        .env_remove("DAEMON_ONLY_SECRET");
    let output = run
        .args([
            "run",
            "--ssh",
            "gpu-box",
            "--resume-template",
            "test -f \"$TMUX_TEST_DIR/killed\" && printf '%s|%s' \"$REMOTE_TEST_SECRET\" \"${DAEMON_ONLY_SECRET-unset}\" > \"$SLUMBER_HOME/resumed\"",
            "printf remote-command",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let submission = String::from_utf8(output.stdout).unwrap();
    let job_id = submission.split_whitespace().nth(1).unwrap().to_owned();
    let local_job = root.join("state/jobs").join(&job_id);
    let persisted_request = fs::read_to_string(local_job.join("request.json")).unwrap();
    let request_mode = fs::metadata(local_job.join("request.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let job_dir_mode = fs::metadata(&local_job).unwrap().permissions().mode() & 0o777;
    let first_daemon: i32 = fs::read_to_string(root.join("state/slumberd.pid"))
        .unwrap()
        .parse()
        .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(first_daemon),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    thread::sleep(Duration::from_millis(50));

    fs::write(fake_remote.join("exit_code"), "23\n").unwrap();
    let mut daemon_command = Command::new(binary);
    configure(&mut daemon_command);
    daemon_command
        .env_remove("REMOTE_TEST_SECRET")
        .env("DAEMON_ONLY_SECRET", "daemon-secret");
    let mut daemon = daemon_command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    for _ in 0..100 {
        let request_is_cleared = fs::read_to_string(local_job.join("request.json"))
            .map(|request| !request.contains("request-secret"))
            .unwrap_or(false);
        if root.join("state/resumed").exists()
            && local_job.join("payload.md").exists()
            && request_is_cleared
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let meta = fs::read_to_string(local_job.join("meta.json")).unwrap();
    let request = fs::read_to_string(local_job.join("request.json")).unwrap();
    let payload = fs::read_to_string(local_job.join("payload.md")).unwrap();
    let wrapper = fs::read_to_string(fake_remote.join("wrapper.sh")).unwrap();
    let tmux_split = fs::read_to_string(tmux_state.join("split")).unwrap();
    let tmux_kill = fs::read_to_string(tmux_state.join("killed")).unwrap();
    let resumed = fs::read_to_string(root.join("state/resumed")).unwrap();
    let mut logs = Command::new(binary);
    configure(&mut logs);
    let logs = logs.args(["logs", &job_id]).output().unwrap();

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert!(meta.contains("\"Exited\": 23"));
    assert!(meta.contains("\"ssh_target\": \"gpu-box\""));
    assert!(persisted_request.contains("request-secret"));
    assert_eq!(request_mode, 0o600);
    assert_eq!(job_dir_mode, 0o700);
    assert!(!request.contains("request-secret"));
    assert!(request.contains("\"env_vars\": {}"));
    assert_eq!(resumed, "request-secret|unset");
    assert!(payload.contains("Remote Target    : gpu-box"));
    assert!(payload.contains("gpu-box:~/.slumber/jobs/"));
    assert!(tmux_split.contains("ssh -t"));
    assert!(tmux_split.contains("gpu-box"));
    assert!(tmux_split.contains("stdout.log"));
    assert!(tmux_split.contains("stderr.log"));
    assert!(tmux_kill.contains("kill-pane -t %9"));
    assert!(wrapper.contains("nohup sh -c"));
    assert!(wrapper.contains("exit_code.tmp"));
    assert_eq!(String::from_utf8(logs.stdout).unwrap(), "remote stdout");
}

#[test]
fn tmux_log_pane_opens_and_closes_before_resume() {
    let binary = env!("CARGO_BIN_EXE_slumber");
    let root = temp_root("tmux");
    let fake_bin = root.join("bin");
    let tmux_state = root.join("tmux");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&tmux_state).unwrap();
    let fake_tmux = fake_bin.join("tmux");
    fs::write(
        &fake_tmux,
        r#"#!/bin/sh
case "$1" in
  split-window)
    printf 'split %s\n' "$*" >> "$TMUX_TEST_DIR/events"
    printf '%%7\n'
    ;;
  kill-pane)
    printf 'kill %s\n' "$*" >> "$TMUX_TEST_DIR/events"
    : > "$TMUX_TEST_DIR/killed"
    ;;
  *) exit 2 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let configure = |command: &mut Command| {
        isolated(command, &root);
        command
            .env("PATH", &fake_path)
            .env("TMUX", "/tmp/fake-tmux,123,0")
            .env("TMUX_PANE", "%1")
            .env("TMUX_TEST_DIR", &tmux_state);
    };

    let mut run = Command::new(binary);
    configure(&mut run);
    let output = run
        .args([
            "run",
            "--resume-template",
            "test -f \"$TMUX_TEST_DIR/killed\" && printf resumed > \"$SLUMBER_HOME/resumed\"",
            "sleep 0.05",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let submission = String::from_utf8(output.stdout).unwrap();
    assert!(submission.contains("tail pane: %7"));
    let job_id = submission.split_whitespace().nth(1).unwrap();
    let job_dir = root.join("state/jobs").join(job_id);
    for _ in 0..50 {
        if root.join("state/resumed").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let meta = fs::read_to_string(job_dir.join("meta.json")).unwrap();
    let first_events = fs::read_to_string(tmux_state.join("events")).unwrap();

    let mut no_tail = Command::new(binary);
    configure(&mut no_tail);
    let output = no_tail
        .args(["run", "--no-tail", "--resume-template", "true", "true"])
        .output()
        .unwrap();
    assert!(output.status.success());
    thread::sleep(Duration::from_millis(50));

    fs::write(root.join("state/config.toml"), "auto_tmux_tail = false\n").unwrap();
    let mut config_disabled = Command::new(binary);
    configure(&mut config_disabled);
    let output = config_disabled
        .args(["run", "--resume-template", "true", "true"])
        .output()
        .unwrap();
    assert!(output.status.success());
    thread::sleep(Duration::from_millis(50));
    let final_events = fs::read_to_string(tmux_state.join("events")).unwrap();

    let daemon_pid: i32 = fs::read_to_string(root.join("state/slumberd.pid"))
        .unwrap()
        .parse()
        .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon_pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    thread::sleep(Duration::from_millis(20));
    fs::remove_dir_all(&root).unwrap();

    assert!(meta.contains("\"tmux_pane_id\": \"%7\""));
    assert!(first_events.contains("split split-window -h -d -P -F #{pane_id} -t %1"));
    assert!(first_events.contains("tail -f -n 20"));
    assert!(first_events.contains("stdout.log"));
    assert!(first_events.contains("stderr.log"));
    assert!(first_events.contains("kill kill-pane -t %7"));
    assert_eq!(first_events, final_events);
}
