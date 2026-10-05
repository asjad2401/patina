use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct CommandRecord {
    pub command: String,
    pub args: Vec<String>,
    pub resolved_path: String,
    pub binary_sha256: String,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub timestamp: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
}

const SECRET_MARKERS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "KEY",
    "PASS",
    "CREDENTIAL",
    "AUTH",
    "PRIVATE",
];

pub fn snapshot_env() -> HashMap<String, String> {
    std::env::vars()
        .map(|(name, value)| {
            if is_secret(&name, &value) {
                (name, "<redacted>".to_string())
            } else {
                (name, value)
            }
        })
        .collect()
}

fn is_secret(name: &str, value: &str) -> bool {
    let name = name.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|m| name.contains(m)) || has_url_password(value)
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

pub fn new_session_log_path() -> PathBuf {
    let dir = PathBuf::from(".patina/sessions");
    let _ = std::fs::create_dir_all(&dir);
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    dir.join(format!("{}.jsonl", ts))
}

pub fn append_record(log_path: &Path, record: &CommandRecord) -> Result<()> {
    let line = serde_json::to_string(record)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log_path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_secret;

    #[test]
    fn detects_secrets() {
        assert!(is_secret("GITHUB_TOKEN", "x"));
        assert!(is_secret("openai_api_key", "x"));
        assert!(is_secret("DB_PASSWORD", "x"));
        assert!(is_secret("DATABASE_URL", "postgres://user:pw@localhost/db"));
        assert!(!is_secret("PATH", "/usr/bin:/bin"));
        assert!(!is_secret("HOME", "/home/user"));
        assert!(!is_secret("DATABASE_URL", "postgres://localhost:5432/db"));
    }
}
