use anyhow::Result;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct CommandRecord {
    pub command: String,
    pub args: Vec<String>,
    pub resolved_path: String,
    pub binary_sha256: String,
    pub cwd: String,
    pub timestamp: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
}

pub fn new_session_log_path() -> PathBuf {
    let dir = PathBuf::from(".patina/sessions");
    let _ = std::fs::create_dir_all(&dir);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    dir.join(format!("{}.jsonl", ts))
}

pub fn append_record(log_path: &Path, record: &CommandRecord) -> Result<()> {
    let line = serde_json::to_string(record)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}
