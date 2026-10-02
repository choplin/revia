#![cfg(unix)]

use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn repository() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the Unix epoch")
        .as_nanos();
    let sequence = REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("revia-pty-{nonce}-{sequence}"));
    fs::create_dir_all(&path).expect("could not create temporary repository");
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&path)
            .status()
            .expect("could not initialize temporary Git repository")
            .success()
    );
    for arguments in [
        &["config", "user.name", "Revia Test"][..],
        &["config", "user.email", "revia@example.invalid"][..],
        &["commit", "--allow-empty", "-qm", "base"][..],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(arguments)
                .status()
                .expect("could not prepare temporary Git repository")
                .success()
        );
    }
    path
}

#[test]
fn starts_and_quits_cleanly_in_a_pseudo_terminal() {
    let repository = repository();
    let binary = env!("CARGO_BIN_EXE_revia");
    let mut command = Command::new("script");
    configure_pseudo_terminal(&mut command, binary, &repository);
    let mut child = command
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("could not start the pseudo-terminal helper");
    child
        .stdin
        .take()
        .expect("pseudo-terminal helper should accept input")
        .write_all(b"q")
        .expect("could not send quit input to pseudo-terminal");
    let output = child
        .wait_with_output()
        .expect("could not wait for pseudo-terminal helper");
    let _ = fs::remove_dir_all(&repository);

    assert!(
        output.status.success(),
        "revia did not exit cleanly from the pseudo-terminal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "macos")]
fn configure_pseudo_terminal(command: &mut Command, binary: &str, repository: &std::path::Path) {
    command
        .args(["-q", "-e", "/dev/null", binary, "--repo"])
        .arg(repository);
}

#[cfg(not(target_os = "macos"))]
fn configure_pseudo_terminal(command: &mut Command, binary: &str, repository: &std::path::Path) {
    let invocation = format!(
        "{} --repo {}",
        shell_escape(binary),
        shell_escape(&repository.display().to_string())
    );
    command.args(["-q", "-e", "-c", &invocation, "/dev/null"]);
}

#[cfg(not(target_os = "macos"))]
fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
