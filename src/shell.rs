use crate::record::{append_record, CommandRecord};
use crate::resolve::{hash_file, resolve_binary};
use crate::tokenizer::tokenize;
use anyhow::Result;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execv, fork, ForkResult};
use std::ffi::CString;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

pub fn repl(log_path: &Path) -> Result<()> {
    let mut input = String::new();
    loop {
        print!("patina> ");
        io::stdout().flush()?;
        input.clear();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            // EOF (Ctrl-D)
            println!();
            break;
        }

        let line = input.trim();
        if line.is_empty() {
            continue;
        }

        let tokens = match tokenize(line) {
            Ok(t) if t.is_empty() => continue,
            Ok(t) => t,
            Err(e) => {
                eprintln!("patina: parse error: {}", e);
                continue;
            }
        };
        let (cmd, args) = tokens.split_first().unwrap();

        // Built-ins must run in THIS process, not a fork — a forked `cd`
        // would change the child's cwd and vanish when it exits.
        match cmd.as_str() {
            "exit" => break,
            "cd" => {
                let target = args.first().cloned().unwrap_or_else(|| {
                    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
                });
                if let Err(e) = std::env::set_current_dir(&target) {
                    eprintln!("cd: {}: {}", target, e);
                }
                continue;
            }
            _ => {}
        }

        if let Err(e) = run_external(cmd, args, log_path) {
            eprintln!("patina: {}", e);
        }
    }
    Ok(())
}

fn run_external(cmd: &str, args: &[String], log_path: &Path) -> Result<()> {
    let resolved = resolve_binary(cmd)?;
    let binary_hash = hash_file(&resolved)?;
    let cwd = std::env::current_dir()?;
    let timestamp = chrono::Utc::now().to_rfc3339();

    let start = Instant::now();

    let path_c = CString::new(resolved.to_string_lossy().as_bytes())?;
    let mut argv_c: Vec<CString> = Vec::with_capacity(args.len() + 1);
    argv_c.push(CString::new(cmd)?);
    for a in args {
        argv_c.push(CString::new(a.as_str())?);
    }

    let exit_code = match unsafe { fork() }? {
        ForkResult::Child => {
            let _ = execv(&path_c, &argv_c);
            std::process::exit(127);
        }
        ForkResult::Parent { child } => match waitpid(child, None)? {
            WaitStatus::Exited(_, code) => Some(code),
            WaitStatus::Signaled(_, sig, _) => {
                eprintln!("patina: process killed by signal {:?}", sig);
                None
            }
            _ => None,
        },
    };

    let duration_ms = start.elapsed().as_millis();

    let record = CommandRecord {
        command: cmd.to_string(),
        args: args.to_vec(),
        resolved_path: resolved.to_string_lossy().to_string(),
        binary_sha256: binary_hash,
        cwd: cwd.to_string_lossy().to_string(),
        timestamp,
        exit_code,
        duration_ms,
    };
    append_record(log_path, &record)?;

    Ok(())
}