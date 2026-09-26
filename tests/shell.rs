//! Shell behaviour, driven by piping a script into patina's stdin.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("patina-shell-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(name: &str, script: &str) -> (Output, PathBuf) {
    let dir = scratch(name);
    let mut child = Command::new(env!("CARGO_BIN_EXE_patina"))
        .current_dir(&dir)
        .env("HOME", &dir)
        .env("PATINA_LOG_DIR", dir.join("logs"))
        .env_remove("PATINA_UNSET_VAR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    (child.wait_with_output().unwrap(), dir)
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn exit_status_follows_exit_and_last_command() {
    assert_eq!(run("exit3", "exit 3\n").0.status.code(), Some(3));
    assert_eq!(run("false", "false\n").0.status.code(), Some(1));
    assert_eq!(
        run("notfound", "nosuchcmd_xyz\n").0.status.code(),
        Some(127)
    );
    assert_eq!(run("true", "false\ntrue\n").0.status.code(), Some(0));
}

#[test]
fn quoted_operators_and_empty_expansions() {
    let (o, _) = run("quoting", "echo '|' x '>' y\necho a $PATINA_UNSET_VAR b\n");
    let out = stdout(&o);
    assert!(out.contains("| x > y\n"), "{}", out);
    assert!(out.contains("a b\n"), "{}", out);
}

#[test]
fn builtins_in_a_pipeline_do_not_change_the_shell() {
    let (o, dir) = run("cdpipe", "cd / | cat\npwd\n");
    let out = stdout(&o);
    let here = std::fs::canonicalize(&dir).unwrap();
    assert!(out.contains(&format!("{}\n", here.display())), "{}", out);
}

#[test]
fn stderr_append_is_recorded_and_replayable() {
    let (o, dir) = run(
        "stderrappend",
        "ls /patina_nope 2>> err.log\nls /patina_nope 2>> err.log\n",
    );
    assert_eq!(o.status.code().map(|c| c != 0), Some(true));
    let err = std::fs::read_to_string(dir.join("err.log")).unwrap();
    assert_eq!(err.lines().count(), 2, "{}", err);

    // Replaying into an empty copy: both appends happen again, identically.
    let copy = dir.join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_patina"))
        .args(["replay", "--yes", "--cwd", copy.to_str().unwrap()])
        .env("HOME", &dir)
        .env("PATINA_LOG_DIR", dir.join("logs"))
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(out.status.success(), "{}", text);
    assert!(text.contains("2 matched"), "{}", text);
}
