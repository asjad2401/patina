use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Why a command can't be run, with the status bash would report.
#[derive(Debug)]
pub enum ResolveError {
    /// A bare name that isn't on `$PATH`.
    NotFound(String),
    /// A path (contains `/`) that doesn't exist.
    NoSuchFile(String),
    /// A path that exists but is a directory.
    IsDirectory(String),
    /// A path that exists but isn't executable.
    PermissionDenied(String),
}

impl ResolveError {
    pub fn status(&self) -> i32 {
        match self {
            ResolveError::NotFound(_) | ResolveError::NoSuchFile(_) => 127,
            ResolveError::IsDirectory(_) | ResolveError::PermissionDenied(_) => 126,
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ResolveError::NotFound(c) => write!(f, "{}: command not found", c),
            ResolveError::NoSuchFile(c) => write!(f, "{}: No such file or directory", c),
            ResolveError::IsDirectory(c) => write!(f, "{}: is a directory", c),
            ResolveError::PermissionDenied(c) => write!(f, "{}: Permission denied", c),
        }
    }
}

impl std::error::Error for ResolveError {}

pub fn resolve_binary(cmd: &str) -> Result<PathBuf> {
    if cmd.contains('/') {
        let p = PathBuf::from(cmd);
        let err = match fs::metadata(&p) {
            Err(_) => ResolveError::NoSuchFile(cmd.to_string()),
            Ok(m) if m.is_dir() => ResolveError::IsDirectory(cmd.to_string()),
            Ok(_) if is_executable(&p) => return Ok(p),
            Ok(_) => ResolveError::PermissionDenied(cmd.to_string()),
        };
        return Err(err.into());
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(cmd);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ResolveError::NotFound(cmd.to_string()).into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn status_of(cmd: &str) -> i32 {
        resolve_binary(cmd)
            .unwrap_err()
            .downcast_ref::<ResolveError>()
            .unwrap()
            .status()
    }

    #[test]
    fn statuses_match_bash() {
        let dir = std::env::temp_dir().join(format!("patina-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let noexec = dir.join("noexec.sh");
        std::fs::write(&noexec, "echo hi\n").unwrap();
        std::fs::set_permissions(&noexec, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(status_of("patina_no_such_command_xyz"), 127);
        assert_eq!(status_of(dir.join("missing").to_str().unwrap()), 127);
        assert_eq!(status_of(noexec.to_str().unwrap()), 126);
        assert_eq!(status_of(dir.to_str().unwrap()), 126);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
