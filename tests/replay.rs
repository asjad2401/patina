//! End-to-end: record a session by piping commands into patina, then replay it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("patina-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).unwrap();
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::create_dir_all(dir.join("work")).unwrap();
    dir
}

fn patina(root: &Path, cwd: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_patina"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", root.join("home"))
        .env("PATINA_LOG_DIR", root.join("logs"))
        .env("GITHUB_TOKEN", "ghp_should_not_be_logged")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn record_then_replay_into_another_directory() {
    let root = scratch("replay");
    let work = root.join("work");
    std::fs::write(work.join("data.txt"), "b\na\nc\n").unwrap();

    let rec = patina(
        &root,
        &work,
        &[],
        "sort < data.txt > sorted.txt\ncat data.txt | wc -l > count.txt\nexit\n",
    );
    assert!(rec.status.success(), "{:?}", rec);

    let log = std::fs::read_dir(root.join("logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("\"type\":\"session\""));
    assert!(!text.contains("ghp_should_not_be_logged"));
    assert!(text.contains("\"line\":\"cat data.txt | wc -l > count.txt\""));

    // Faithful replay into a copy of the starting state.
    let copy = root.join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join("data.txt"), "b\na\nc\n").unwrap();
    let out = patina(
        &root,
        &root,
        &["replay", "--yes", "--cwd", copy.to_str().unwrap()],
        "",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", stdout);
    assert!(stdout.contains("2 matched"), "{}", stdout);
    assert_eq!(
        std::fs::read_to_string(copy.join("sorted.txt")).unwrap(),
        "a\nb\nc\n"
    );

    // Changed input: replay must flag the drift and fail.
    let drift = root.join("drift");
    std::fs::create_dir_all(&drift).unwrap();
    std::fs::write(drift.join("data.txt"), "z\n").unwrap();
    let out = patina(
        &root,
        &root,
        &["replay", "--yes", "--cwd", drift.to_str().unwrap()],
        "",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{}", stdout);
    assert!(stdout.contains("data.txt differs"), "{}", stdout);
    assert!(stdout.contains("0 matched"), "{}", stdout);

    let _ = std::fs::remove_dir_all(&root);
}
