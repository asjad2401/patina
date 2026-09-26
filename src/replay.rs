use crate::exec;
use crate::parser::{Command, StderrRedirect, StdoutRedirect};
use crate::record::{
    self, CommandRecord, Entry, FileRole, FileState, HostInfo, SessionHeader, REDACTED,
};
use crate::resolve::{hash_file, is_executable, resolve_binary};
use crate::ui;
use anyhow::{anyhow, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: patina replay [SESSION] [--dry-run] [--yes] [--cwd DIR]

  SESSION     path to a session log, or 'latest' (default)
  --dry-run   check binaries and input files without running anything
  --yes, -y   don't ask for confirmation before running commands
  --cwd DIR   replay into DIR instead of the directory the session started in;
              recorded paths under the original directory are mapped into DIR";

/// Environment variables that describe the recording shell itself rather than
/// the session, so they're not restored on replay.
const SHELL_LOCAL_VARS: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

struct Options {
    session: Option<String>,
    dry_run: bool,
    yes: bool,
    cwd: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Options> {
    let mut opts = Options {
        session: None,
        dry_run: false,
        yes: false,
        cwd: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" | "-n" => opts.dry_run = true,
            "--yes" | "-y" => opts.yes = true,
            "--cwd" => {
                let dir = it
                    .next()
                    .ok_or_else(|| anyhow!("--cwd needs a directory"))?;
                opts.cwd = Some(PathBuf::from(dir));
            }
            "--help" | "-h" => {
                println!("{}", USAGE);
                std::process::exit(0);
            }
            s if s.starts_with('-') => return Err(anyhow!("unknown option '{}'\n\n{}", s, USAGE)),
            s if opts.session.is_none() => opts.session = Some(s.to_string()),
            s => return Err(anyhow!("unexpected argument '{}'\n\n{}", s, USAGE)),
        }
    }
    Ok(opts)
}

fn find_session(arg: Option<&str>) -> Result<PathBuf> {
    match arg {
        None | Some("latest") => record::list_sessions()?
            .pop()
            .ok_or_else(|| anyhow!("no recorded sessions in {}", record::log_dir().display())),
        Some(p) => Ok(PathBuf::from(p)),
    }
}

/// Maps paths under the recorded start directory into the replay directory.
struct Rebase {
    from: PathBuf,
    to: PathBuf,
}

impl Rebase {
    fn path(&self, p: &str) -> String {
        match Path::new(p).strip_prefix(&self.from) {
            Ok(rest) if Path::new(p).is_absolute() => {
                if rest.as_os_str().is_empty() {
                    self.to.to_string_lossy().into()
                } else {
                    self.to.join(rest).to_string_lossy().into()
                }
            }
            _ => p.to_string(),
        }
    }

    fn command(&self, c: &Command) -> Command {
        Command {
            cmd: c.cmd.clone(),
            args: c.args.iter().map(|a| self.path(a)).collect(),
            stdin: c.stdin.as_deref().map(|p| self.path(p)),
            stdout: c.stdout.as_ref().map(|r| match r {
                StdoutRedirect::Truncate(p) => StdoutRedirect::Truncate(self.path(p)),
                StdoutRedirect::Append(p) => StdoutRedirect::Append(self.path(p)),
            }),
            stderr: c.stderr.as_ref().map(|r| match r {
                StderrRedirect::Truncate(p) => StderrRedirect::Truncate(self.path(p)),
                StderrRedirect::Append(p) => StderrRedirect::Append(self.path(p)),
                StderrRedirect::ToStdout => StderrRedirect::ToStdout,
            }),
        }
    }
}

#[derive(Default)]
struct Tally {
    matched: usize,
    differed: usize,
    failed: usize,
    checked: usize,
    skipped: usize,
    warnings: usize,
}

/// Entry point for `patina replay ...`. Returns the process exit code.
pub fn main(args: &[String]) -> Result<i32> {
    let opts = parse_args(args)?;
    let log_path = find_session(opts.session.as_deref())?;
    let (header, entries) = record::read_log(&log_path)?;

    let target_root = match &opts.cwd {
        Some(d) => std::fs::canonicalize(d).map_err(|e| anyhow!("--cwd {}: {}", d.display(), e))?,
        None => PathBuf::from(&header.cwd),
    };
    let rebase = Rebase {
        from: PathBuf::from(&header.cwd),
        to: target_root.clone(),
    };

    let commands = entries
        .iter()
        .filter(|e| matches!(e, Entry::Command(_)))
        .count();

    print_intro(&log_path, &header, commands, &rebase, opts.dry_run);

    if commands == 0 {
        println!("\nNothing to replay.");
        return Ok(0);
    }

    if !opts.dry_run && !opts.yes && !confirm(commands)? {
        println!("Aborted.");
        return Ok(1);
    }

    exec::ignore_interactive_signals();
    let restored = apply_recorded_env(&header);
    if !restored.is_empty() {
        ui::print_note(&format!(
            "restored recorded values for: {}",
            restored.join(", ")
        ));
    }

    let mut tally = Tally::default();
    let mut n = 0;
    for entry in &entries {
        match entry {
            Entry::Session(_) => {}
            Entry::Builtin(b) => {
                // Each command carries its own cwd, so `cd` needs no re-execution.
                println!(
                    "\n{}",
                    ui::dim(&format!("   {} {}", b.name, b.args.join(" ")))
                );
            }
            Entry::Command(rec) => {
                n += 1;
                replay_one(rec, n, commands, &rebase, opts.dry_run, &mut tally);
            }
        }
    }

    print_summary(&tally, opts.dry_run);
    Ok(if tally.differed + tally.failed > 0 {
        1
    } else {
        0
    })
}

fn print_intro(
    log_path: &Path,
    header: &SessionHeader,
    commands: usize,
    rebase: &Rebase,
    dry_run: bool,
) {
    let mode = if dry_run { "Checking" } else { "Replaying" };
    println!("{} {}", ui::bold(mode), log_path.display());
    println!(
        "   recorded {} on {} ({}/{}), patina {}",
        header.started_at,
        header.host.hostname.as_deref().unwrap_or("unknown host"),
        header.host.os,
        header.host.arch,
        header.patina_version
    );
    println!("   {} command line(s)", commands);
    if rebase.from == rebase.to {
        println!("   in {}", rebase.to.display());
    } else {
        println!(
            "   in {}  (recorded in {})",
            rebase.to.display(),
            rebase.from.display()
        );
    }

    let here = HostInfo::current();
    if here.os != header.host.os || here.arch != header.host.arch {
        ui::print_warn(&format!(
            "recorded on {}/{}, replaying on {}/{}",
            header.host.os, header.host.arch, here.os, here.arch
        ));
    }
}

fn confirm(commands: usize) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Err(anyhow!(
            "replay runs {} command(s); pass --yes to run without a terminal, or --dry-run to only check",
            commands
        ));
    }
    print!(
        "\nThis will run {} command line(s) for real. Continue? [y/N] ",
        commands
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// Overlays the recorded environment onto ours. Redacted values keep whatever
/// the current shell has. Returns the names of variables that changed.
fn apply_recorded_env(header: &SessionHeader) -> Vec<String> {
    let mut changed = Vec::new();
    for (k, v) in &header.env {
        if v == REDACTED || SHELL_LOCAL_VARS.contains(&k.as_str()) {
            continue;
        }
        if std::env::var(k).ok().as_deref() != Some(v.as_str()) {
            std::env::set_var(k, v);
            changed.push(k.clone());
        }
    }
    changed
}

fn is_self_replay(rec: &CommandRecord) -> bool {
    rec.stages.iter().any(|s| {
        Path::new(&s.resolved_path)
            .file_name()
            .is_some_and(|f| f == "patina")
            && s.command.args.first().map(String::as_str) == Some("replay")
    })
}

fn replay_one(
    rec: &CommandRecord,
    n: usize,
    total: usize,
    rebase: &Rebase,
    dry_run: bool,
    tally: &mut Tally,
) {
    println!("\n{} {}", ui::bold(&format!("[{}/{}]", n, total)), rec.line);

    if is_self_replay(rec) {
        ui::print_note("skipped: nested 'patina replay'");
        tally.skipped += 1;
        return;
    }

    for (k, v) in &rec.env_diff.set {
        if v != REDACTED {
            std::env::set_var(k, v);
        }
    }
    for k in &rec.env_diff.unset {
        std::env::remove_var(k);
    }

    let cwd = PathBuf::from(rebase.path(&rec.cwd));
    if let Err(e) = std::env::set_current_dir(&cwd) {
        ui::print_fail(&format!("working directory {}: {}", cwd.display(), e));
        tally.failed += 1;
        return;
    }
    std::env::set_var("PWD", &cwd);

    // Binaries: prefer the recorded path, fall back to $PATH, and say when
    // what we'll run isn't byte-identical to what was recorded.
    let mut binaries = Vec::with_capacity(rec.stages.len());
    for st in &rec.stages {
        let recorded = PathBuf::from(rebase.path(&st.resolved_path));
        let bin = if is_executable(&recorded) {
            recorded
        } else {
            match resolve_binary(&st.command.cmd) {
                Ok(p) => {
                    ui::print_warn(&format!(
                        "{} is gone; using {}",
                        recorded.display(),
                        p.display()
                    ));
                    tally.warnings += 1;
                    p
                }
                Err(_) => {
                    ui::print_fail(&format!(
                        "{}: not found (recorded at {})",
                        st.command.cmd,
                        recorded.display()
                    ));
                    tally.failed += 1;
                    return;
                }
            }
        };
        match hash_file(&bin) {
            Ok(h) if h == st.binary_sha256 => {}
            Ok(h) => {
                ui::print_warn(&format!(
                    "{} changed since recording (sha256 {} → {})",
                    bin.display(),
                    &st.binary_sha256[..12.min(st.binary_sha256.len())],
                    &h[..12]
                ));
                tally.warnings += 1;
            }
            Err(e) => {
                ui::print_fail(&format!("cannot hash {}: {}", bin.display(), e));
                tally.failed += 1;
                return;
            }
        }
        binaries.push(bin);
    }

    // Preconditions: files the line read should look like they did then.
    for f in &rec.files {
        if let Some(before) = &f.before {
            let now = FileState::of(&cwd.join(rebase.path(&f.path)));
            if &now != before {
                ui::print_warn(&format!(
                    "{} differs from when it was recorded ({} → {})",
                    f.path,
                    before.short(),
                    now.short()
                ));
                tally.warnings += 1;
            }
        }
    }

    if dry_run {
        tally.checked += 1;
        return;
    }

    let commands: Vec<Command> = rec
        .stages
        .iter()
        .map(|s| rebase.command(&s.command))
        .collect();
    let stages: Vec<_> = commands
        .iter()
        .zip(&binaries)
        .map(|(c, b)| (c, b.as_path()))
        .collect();
    let statuses = match exec::run(&stages) {
        Ok(s) => s,
        Err(e) => {
            ui::print_fail(&e.to_string());
            tally.failed += 1;
            return;
        }
    };

    let mut differences = Vec::new();
    for (st, status) in rec.stages.iter().zip(&statuses) {
        if st.exit_code != status.exit_code() || st.signal != status.signal_name() {
            differences.push(format!(
                "{} exited {} (recorded {})",
                st.command.cmd,
                describe_exit(status.exit_code(), status.signal_name().as_deref()),
                describe_exit(st.exit_code, st.signal.as_deref())
            ));
        }
    }
    for f in &rec.files {
        // An argument the command only read was already checked beforehand.
        if f.role == FileRole::Arg && f.after == f.before {
            continue;
        }
        if let Some(after) = &f.after {
            let now = FileState::of(&cwd.join(rebase.path(&f.path)));
            if &now != after {
                let what = match f.role {
                    FileRole::Stdout | FileRole::Stderr => "output",
                    _ => "file",
                };
                differences.push(format!(
                    "{} {} ended up different ({} recorded, {} now)",
                    what,
                    f.path,
                    after.short(),
                    now.short()
                ));
            }
        }
    }

    if differences.is_empty() {
        let outputs = rec.files.iter().filter(|f| f.after.is_some()).count();
        let exit = statuses
            .last()
            .map(|s| describe_exit(s.exit_code(), s.signal_name().as_deref()))
            .unwrap_or_default();
        let detail = if outputs > 0 {
            format!("exit {}, {} file(s) identical", exit, outputs)
        } else {
            format!("exit {}", exit)
        };
        ui::print_ok(&format!("matches recording ({})", detail));
        tally.matched += 1;
    } else {
        for d in &differences {
            ui::print_fail(d);
        }
        tally.differed += 1;
    }
}

fn describe_exit(code: Option<i32>, signal: Option<&str>) -> String {
    match (code, signal) {
        (Some(c), _) => c.to_string(),
        (None, Some(s)) => format!("by {}", s),
        (None, None) => "?".to_string(),
    }
}

fn print_summary(t: &Tally, dry_run: bool) {
    println!();
    let mut parts = Vec::new();
    if dry_run {
        parts.push(format!("{} checked", t.checked));
    } else {
        parts.push(format!("{} matched", t.matched));
        parts.push(format!("{} differed", t.differed));
    }
    if t.failed > 0 {
        parts.push(format!("{} could not run", t.failed));
    }
    if t.skipped > 0 {
        parts.push(format!("{} skipped", t.skipped));
    }
    parts.push(format!("{} warning(s)", t.warnings));
    let line = parts.join(" · ");
    if t.differed + t.failed == 0 {
        ui::print_ok(&line);
    } else {
        ui::print_fail(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_maps_paths_under_the_start_dir_only() {
        let r = Rebase {
            from: PathBuf::from("/home/a/proj"),
            to: PathBuf::from("/tmp/copy"),
        };
        assert_eq!(r.path("/home/a/proj"), "/tmp/copy");
        assert_eq!(r.path("/home/a/proj/src/x.rs"), "/tmp/copy/src/x.rs");
        assert_eq!(r.path("/home/a/project2"), "/home/a/project2");
        assert_eq!(r.path("relative.txt"), "relative.txt");
        assert_eq!(r.path("/usr/bin/ls"), "/usr/bin/ls");
    }
}
