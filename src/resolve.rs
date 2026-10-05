use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct CommandNotFound(String);

impl std::fmt::Display for CommandNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}: command not found", self.0)
    }
}

impl std::error::Error for CommandNotFound {}

pub fn resolve_binary(cmd: &str) -> Result<PathBuf> {
    if cmd.contains('/') {
        let p = PathBuf::from(cmd);
        return if is_executable(&p) {
            Ok(p)
        } else {
            Err(anyhow!("{}: not an executable file", cmd))
        };
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(cmd);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(CommandNotFound(cmd.to_string()).into())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}
