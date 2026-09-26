use crate::parser::{Command, StderrRedirect, StdoutRedirect};
use crate::resolve::hash_file;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Bump when the on-disk log format changes incompatibly.
pub const FORMAT_VERSION: u32 = 2;

/// Files larger than this are noted but not hashed.
const MAX_HASH_BYTES: u64 = 256 * 1024 * 1024;

pub const REDACTED: &str = "<redacted>";

/// Env var names containing any of these (case-insensitive) have their value
/// replaced with `REDACTED` before being written to a log.
const SECRET_MARKERS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASS",
    "KEY",
    "CREDENTIAL",
    "AUTH",
    "COOKIE",
    "PRIVATE",
    "SESSION",
    "DSN",
];

/// One line of a session log.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    Session(SessionHeader),
    Command(CommandRecord),
    Builtin(BuiltinRecord),
}

/// First line of every session log.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionHeader {
    pub version: u32,
    pub patina_version: String,
    pub started_at: String,
    pub host: HostInfo,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HostInfo {
    pub os: String,
    pub arch: String,
    pub hostname: Option<String>,
}

impl HostInfo {
    pub fn current() -> Self {
        HostInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            hostname: nix::unistd::gethostname()
                .ok()
                .map(|h| h.to_string_lossy().to_string()),
        }
    }
}

/// One input line that ran external programs: a single command or a pipeline.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandRecord {
    /// The line exactly as typed, before variable expansion.
    pub line: String,
    pub cwd: String,
    pub timestamp: String,
    pub duration_ms: u64,
    /// Changes to the environment since the previous record (or the header).
    #[serde(default, skip_serializing_if = "EnvDiff::is_empty")]
    pub env_diff: EnvDiff,
    pub stages: Vec<StageRecord>,
    /// Files the line read or wrote, fingerprinted before and after it ran.
    #[serde(default)]
    pub files: Vec<FileRecord>,
}

/// One program in a pipeline (argv after expansion, plus its redirections).
#[derive(Debug, Serialize, Deserialize)]
pub struct StageRecord {
    #[serde(flatten)]
    pub command: Command,
    pub resolved_path: String,
    pub binary_sha256: String,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// A shell built-in that changes shell state, e.g. `cd`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuiltinRecord {
    pub name: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub new_cwd: Option<String>,
    pub ok: bool,
    pub timestamp: String,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvDiff {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset: Vec<String>,
}

impl EnvDiff {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.unset.is_empty()
    }

    pub fn between(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> Self {
        let set = new
            .iter()
            .filter(|(k, v)| old.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let unset = old
            .keys()
            .filter(|k| !new.contains_key(*k))
            .cloned()
            .collect();
        EnvDiff { set, unset }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileRole {
    Stdin,
    Arg,
    Stdout,
    Stderr,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileRecord {
    /// Path as written on the command line (relative to the record's cwd).
    pub path: String,
    pub role: FileRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<FileState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<FileState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Missing,
    NotRegular,
    TooLarge(u64),
    Sha256(String),
}

impl FileState {
    pub fn of(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Err(_) => FileState::Missing,
            Ok(m) if !m.is_file() => FileState::NotRegular,
            Ok(m) if m.len() > MAX_HASH_BYTES => FileState::TooLarge(m.len()),
            Ok(_) => match hash_file(path) {
                Ok(h) => FileState::Sha256(h),
                Err(_) => FileState::Missing,
            },
        }
    }

    pub fn short(&self) -> String {
        match self {
            FileState::Missing => "missing".into(),
            FileState::NotRegular => "not a regular file".into(),
            FileState::TooLarge(n) => format!("too large to hash ({} bytes)", n),
            FileState::Sha256(h) => format!("sha256 {}", &h[..h.len().min(12)]),
        }
    }
}

/// Files a pipeline will touch, with their state captured before it runs.
/// Call `finish` after the pipeline exits to fill in the `after` states.
pub struct FileWatch {
    cwd: PathBuf,
    records: Vec<FileRecord>,
}

impl FileWatch {
    pub fn before(commands: &[Command], cwd: &Path) -> Self {
        let mut records: Vec<FileRecord> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut add = |path: &str, role: FileRole, capture_before: bool| {
            if !seen.insert((path.to_string(), role as u8)) {
                return;
            }
            let before = capture_before.then(|| FileState::of(&cwd.join(path)));
            records.push(FileRecord {
                path: path.to_string(),
                role,
                before,
                after: None,
            });
        };

        for c in commands {
            if let Some(p) = &c.stdin {
                add(p, FileRole::Stdin, true);
            }
            // Arguments that name existing files are treated as inputs; this
            // catches `cat data.csv`, `python script.py`, `sed -i ... f.txt`.
            for a in &c.args {
                if !a.is_empty() && cwd.join(a).is_file() {
                    add(a, FileRole::Arg, true);
                }
            }
            match &c.stdout {
                // Truncating writes don't depend on the previous contents.
                Some(StdoutRedirect::Truncate(p)) => add(p, FileRole::Stdout, false),
                Some(StdoutRedirect::Append(p)) => add(p, FileRole::Stdout, true),
                None => {}
            }
            match &c.stderr {
                Some(StderrRedirect::Truncate(p)) => add(p, FileRole::Stderr, false),
                Some(StderrRedirect::Append(p)) => add(p, FileRole::Stderr, true),
                _ => {}
            }
        }

        FileWatch {
            cwd: cwd.to_path_buf(),
            records,
        }
    }

    pub fn finish(mut self) -> Vec<FileRecord> {
        for r in &mut self.records {
            if r.role != FileRole::Stdin {
                r.after = Some(FileState::of(&self.cwd.join(&r.path)));
            }
        }
        self.records
    }
}

pub fn is_secret(name: &str, value: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|m| upper.contains(m)) || has_url_password(value)
}

// e.g. DATABASE_URL=postgres://user:pass@host/db
fn has_url_password(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

/// The current environment, with secret-looking values redacted.
pub fn snapshot_env() -> BTreeMap<String, String> {
    std::env::vars()
        .map(|(k, v)| {
            let v = if is_secret(&k, &v) {
                REDACTED.to_string()
            } else {
                v
            };
            (k, v)
        })
        .collect()
}

/// Where session logs live: `$PATINA_LOG_DIR`, else `~/.patina/sessions`.
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PATINA_LOG_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".patina/sessions")
}

/// All session logs, oldest first.
pub fn list_sessions() -> Result<Vec<PathBuf>> {
    let dir = log_dir();
    let mut logs: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect(),
        Err(_) => Vec::new(),
    };
    logs.sort();
    Ok(logs)
}

pub fn read_log(path: &Path) -> Result<(SessionHeader, Vec<Entry>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty()).enumerate();

    let header = match lines.next() {
        None => return Err(anyhow!("{} is empty", path.display())),
        Some((_, first)) => match serde_json::from_str::<Entry>(first) {
            Ok(Entry::Session(h)) => h,
            _ => {
                return Err(anyhow!(
                    "{} was recorded by an older patina (no session header) and can't be replayed",
                    path.display()
                ))
            }
        },
    };
    if header.version != FORMAT_VERSION {
        return Err(anyhow!(
            "{} uses log format v{}, this patina reads v{}",
            path.display(),
            header.version,
            FORMAT_VERSION
        ));
    }

    let mut entries = Vec::new();
    for (i, line) in lines {
        let entry = serde_json::from_str::<Entry>(line)
            .with_context(|| format!("{} line {}", path.display(), i + 1))?;
        entries.push(entry);
    }
    Ok((header, entries))
}

/// Appends entries to one session log, tracking env changes between records.
pub struct Recorder {
    path: PathBuf,
    last_env: BTreeMap<String, String>,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        let dir = log_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating log directory {}", dir.display()))?;
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let path = dir.join(format!("{}-{}.jsonl", ts, std::process::id()));

        let env = snapshot_env();
        let recorder = Recorder {
            path,
            last_env: env.clone(),
        };
        recorder.append(&Entry::Session(SessionHeader {
            version: FORMAT_VERSION,
            patina_version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            host: HostInfo::current(),
            cwd: std::env::current_dir()?.to_string_lossy().to_string(),
            env,
        }))?;
        Ok(recorder)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Env changes since the last call (empty on the first command, since the
    /// header already holds the full environment).
    pub fn take_env_diff(&mut self) -> EnvDiff {
        let now = snapshot_env();
        let diff = EnvDiff::between(&self.last_env, &now);
        self.last_env = now;
        diff
    }

    pub fn append(&self, entry: &Entry) -> Result<()> {
        let line = serde_json::to_string(entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secrets() {
        assert!(is_secret("GITHUB_TOKEN", "x"));
        assert!(is_secret("aws_secret_access_key", "x"));
        assert!(is_secret("openai_api_key", "x"));
        assert!(is_secret("DB_PASSWORD", "x"));
        assert!(is_secret("DATABASE_URL", "postgres://user:pw@localhost/db"));
        assert!(is_secret("SOME_URL", "https://user:pw@example.com/x"));
        assert!(!is_secret("PATH", "/usr/bin:/bin"));
        assert!(!is_secret("HOME", "/home/user"));
        assert!(!is_secret("LANG", "en_US.UTF-8"));
        assert!(!is_secret("REPO", "https://github.com/a/b"));
        assert!(!is_secret("DATABASE_URL", "postgres://localhost:5432/db"));
    }

    #[test]
    fn env_diff_reports_sets_and_unsets() {
        let old: BTreeMap<_, _> = [("A", "1"), ("B", "2")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let new: BTreeMap<_, _> = [("A", "1"), ("C", "3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let d = EnvDiff::between(&old, &new);
        assert_eq!(d.set.get("C").map(String::as_str), Some("3"));
        assert_eq!(d.set.len(), 1);
        assert_eq!(d.unset, vec!["B".to_string()]);
        assert!(EnvDiff::between(&old, &old).is_empty());
    }

    #[test]
    fn command_record_round_trips() {
        let entry = Entry::Command(CommandRecord {
            line: "ls | grep x > out.txt 2>&1".into(),
            cwd: "/tmp".into(),
            timestamp: "t".into(),
            duration_ms: 5,
            env_diff: EnvDiff::default(),
            stages: vec![StageRecord {
                command: Command {
                    cmd: "grep".into(),
                    args: vec!["x".into()],
                    stdin: None,
                    stdout: Some(StdoutRedirect::Truncate("out.txt".into())),
                    stderr: Some(StderrRedirect::ToStdout),
                },
                resolved_path: "/usr/bin/grep".into(),
                binary_sha256: "abc".into(),
                exit_code: Some(1),
                signal: None,
            }],
            files: vec![FileRecord {
                path: "out.txt".into(),
                role: FileRole::Stdout,
                before: None,
                after: Some(FileState::Sha256("def".into())),
            }],
        });
        let json = serde_json::to_string(&entry).unwrap();
        let back: Entry = serde_json::from_str(&json).unwrap();
        match back {
            Entry::Command(c) => {
                assert_eq!(c.stages[0].command.cmd, "grep");
                assert!(matches!(
                    c.stages[0].command.stdout,
                    Some(StdoutRedirect::Truncate(ref p)) if p == "out.txt"
                ));
                assert!(matches!(
                    c.stages[0].command.stderr,
                    Some(StderrRedirect::ToStdout)
                ));
                assert_eq!(c.stages[0].exit_code, Some(1));
                assert_eq!(c.files[0].after, Some(FileState::Sha256("def".into())));
            }
            _ => panic!("wrong variant"),
        }
    }
}
