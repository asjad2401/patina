use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

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

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

type CacheKey = (PathBuf, u64, Option<SystemTime>);

static HASH_CACHE: Mutex<Option<HashMap<CacheKey, String>>> = Mutex::new(None);

/// SHA-256 of a file, streamed. Binaries are hashed on every command, so
/// results are cached by (path, size, mtime) — a rebuilt binary gets a new key.
pub fn hash_file(path: &Path) -> Result<String> {
    let meta = fs::metadata(path)?;
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(hit) = HASH_CACHE
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|c| c.get(&key))
    {
        return Ok(hit.clone());
    }

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = format!("{:x}", hasher.finalize());

    HASH_CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, digest.clone());
    Ok(digest)
}
