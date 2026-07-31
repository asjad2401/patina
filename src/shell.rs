use crate::parser::{self, Command, StdoutRedirect};
use crate::record::{append_record, CommandRecord};
use crate::resolve::{hash_file, resolve_binary};
use crate::tokenizer::tokenize;
use crate::ui;
use anyhow::Result;
use nix::sys::signal::{sigaction, SigAction, SaFlags, SigHandler, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{dup2, execv, fork, pipe as nix_pipe, ForkResult, Pid};
use rustyline::error::ReadlineError;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::ffi::CString;
use std::fs::{File, OpenOptions};

use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Instant;


fn reset_sigpipe_to_default() {
    unsafe {
        let _ = sigaction(
            Signal::SIGPIPE,
            &SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty()),
        );
    }
}


pub fn repl(log_path: &Path) -> Result<()> {

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .build();

    let mut rl: Editor<(), rustyline::history::FileHistory> =
        Editor::with_config(config)?;

    let history_path = dirs_home().map(|h| h.join(".patina_history"));
    if let Some(ref p) = history_path {
        let _ = rl.load_history(p); 
    }


    let mut last_ok = true;
    let mut last_duration_ms: Option<u128> = None;

    loop {
        let prompt = ui::build_prompt(last_ok, last_duration_ms);

        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) => {
                
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!();
                break;
            }
            Err(e) => {
                ui::print_error(&format!("readline: {}", e));
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let _ = rl.add_history_entry(line);

        let tokens = match tokenize(line) {
            Ok(t) if t.is_empty() => continue,
            Ok(t) => t,
            Err(e) => {
                ui::print_error(&format!("parse error: {}", e));
                last_ok = false;
                last_duration_ms = None;
                continue;
            }
        };

        if tokens.len() == 1 || tokens.first().map(String::as_str) != Some("|") {
            if let Some(first) = tokens.first() {
                match first.as_str() {
                    "exit" => break,
                    "cd" => {
                        let target = tokens.get(1).cloned().unwrap_or_else(|| {
                            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
                        });
                        match std::env::set_current_dir(&target) {
                            Ok(()) => last_ok = true,
                            Err(e) => {
                                ui::print_error(&format!("cd: {}: {}", target, e));
                                last_ok = false;
                            }
                        }
                        last_duration_ms = None;
                        continue;
                    }
                    _ => {}
                }
            }
        }

        let commands = match parser::parse_pipeline(tokens) {
            Ok(c) => c,
            Err(e) => {
                ui::print_error(&e.to_string());
                last_ok = false;
                last_duration_ms = None;
                continue;
            }
        };

        let t0 = Instant::now();
        let result = if commands.len() == 1 {
            run_single(commands.into_iter().next().unwrap(), log_path)
        } else {
            run_pipeline(commands, log_path)
        };
        last_duration_ms = Some(t0.elapsed().as_millis());

        match result {
            Ok(()) => last_ok = true,
            Err(e) => {
                ui::print_error(&e.to_string());
                last_ok = false;
            }
        }
    }

    if let Some(ref p) = history_path {
        let _ = rl.save_history(p);
    }

    Ok(())
}


fn open_redirects(command: &Command) -> Result<(Option<File>, Option<File>)> {
    let stdin_file = match &command.stdin {
        Some(path) => Some(
            File::open(path)
                .map_err(|e| anyhow::anyhow!("cannot open '{}' for reading: {}", path, e))?,
        ),
        None => None,
    };
    let stdout_file = match &command.stdout {
        Some(StdoutRedirect::Truncate(path)) => Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .map_err(|e| anyhow::anyhow!("cannot open '{}' for writing: {}", path, e))?,
        ),
        Some(StdoutRedirect::Append(path)) => Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| anyhow::anyhow!("cannot open '{}' for appending: {}", path, e))?,
        ),
        None => None,
    };
    Ok((stdin_file, stdout_file))
}



fn run_single(command: Command, log_path: &Path) -> Result<()> {
    let resolved = resolve_binary(&command.cmd)?;
    let binary_hash = hash_file(&resolved)?;
    let cwd = std::env::current_dir()?;
    let timestamp = chrono::Utc::now().to_rfc3339();
    let (stdin_file, stdout_file) = open_redirects(&command)?;

    let start = Instant::now();
    let argv_c = build_argv(&command)?;
    let stdin_fd = stdin_file.as_ref().map(|f| f.as_raw_fd());
    let stdout_fd = stdout_file.as_ref().map(|f| f.as_raw_fd());
    let path_c = CString::new(resolved.to_string_lossy().as_bytes())?;

    let exit_code = match unsafe { fork() }? {
        ForkResult::Child => {
            reset_sigpipe_to_default();
            if let Some(fd) = stdin_fd {
                if dup2(fd, 0).is_err() {
                    std::process::exit(126);
                }
            }
            if let Some(fd) = stdout_fd {
                if dup2(fd, 1).is_err() {
                    std::process::exit(126);
                }
            }
            let _ = execv(&path_c, &argv_c);
            std::process::exit(127);
        }
        ForkResult::Parent { child } => wait_for(child),
    };

    drop(stdin_file);
    drop(stdout_file);

    let duration_ms = start.elapsed().as_millis();
    append_record(
        log_path,
        &CommandRecord {
            command: command.cmd,
            args: command.args,
            resolved_path: resolved.to_string_lossy().to_string(),
            binary_sha256: binary_hash,
            cwd: cwd.to_string_lossy().to_string(),
            timestamp,
            exit_code,
            duration_ms,
        },
    )
}



fn run_pipeline(commands: Vec<Command>, log_path: &Path) -> Result<()> {
    let n = commands.len();
    let cwd = std::env::current_dir()?;
    let timestamp = chrono::Utc::now().to_rfc3339();

    let mut resolved = Vec::with_capacity(n);
    let mut hashes = Vec::with_capacity(n);
    for c in &commands {
        resolved.push(resolve_binary(&c.cmd)?);
        hashes.push(hash_file(&resolved[resolved.len() - 1])?);
    }

    let mut redirects = Vec::with_capacity(n);
    for c in &commands {
        redirects.push(open_redirects(c)?);
    }

    let mut pipes: Vec<(OwnedFd, OwnedFd)> = Vec::with_capacity(n.saturating_sub(1));
    for _ in 0..n.saturating_sub(1) {
        pipes.push(nix_pipe()?);
    }
    let all_pipe_fds: Vec<RawFd> = pipes
        .iter()
        .flat_map(|(r, w)| [r.as_raw_fd(), w.as_raw_fd()])
        .collect();

    let start = Instant::now();
    let mut child_pids: Vec<Pid> = Vec::with_capacity(n);

    for (i, command) in commands.iter().enumerate() {
        let argv_c = build_argv(command)?;
        let path_c = CString::new(resolved[i].to_string_lossy().as_bytes())?;
        let (stdin_file, stdout_file) = &redirects[i];

        let read_end_from_prev = if i > 0 {
            Some(pipes[i - 1].0.as_raw_fd())
        } else {
            None
        };
        let write_end_to_next = if i < n - 1 {
            Some(pipes[i].1.as_raw_fd())
        } else {
            None
        };
        let explicit_stdin = stdin_file.as_ref().map(|f| f.as_raw_fd());
        let explicit_stdout = stdout_file.as_ref().map(|f| f.as_raw_fd());

        match unsafe { fork() }? {
            ForkResult::Child => {
                reset_sigpipe_to_default();
                if let Some(fd) = read_end_from_prev {
                    let _ = dup2(fd, 0);
                }
                if let Some(fd) = explicit_stdin {
                    let _ = dup2(fd, 0);
                }
                if let Some(fd) = write_end_to_next {
                    let _ = dup2(fd, 1);
                }
                if let Some(fd) = explicit_stdout {
                    let _ = dup2(fd, 1);
                }
                for fd in &all_pipe_fds {
                    let _ = nix::unistd::close(*fd);
                }
                let _ = execv(&path_c, &argv_c);
                std::process::exit(127);
            }
            ForkResult::Parent { child } => child_pids.push(child),
        }
    }

    drop(pipes);
    drop(redirects);

    let mut exit_codes = Vec::with_capacity(n);
    for pid in child_pids {
        exit_codes.push(wait_for(pid));
    }
    let duration_ms = start.elapsed().as_millis();

    for (i, command) in commands.into_iter().enumerate() {
        append_record(
            log_path,
            &CommandRecord {
                command: command.cmd,
                args: command.args,
                resolved_path: resolved[i].to_string_lossy().to_string(),
                binary_sha256: hashes[i].clone(),
                cwd: cwd.to_string_lossy().to_string(),
                timestamp: timestamp.clone(),
                exit_code: exit_codes[i],
                duration_ms,
            },
        )?;
    }

    Ok(())
}


fn build_argv(command: &Command) -> Result<Vec<CString>> {
    let mut argv_c = Vec::with_capacity(command.args.len() + 1);
    argv_c.push(CString::new(command.cmd.as_str())?);
    for a in &command.args {
        argv_c.push(CString::new(a.as_str())?);
    }
    Ok(argv_c)
}

fn wait_for(pid: Pid) -> Option<i32> {
    match waitpid(pid, None) {
        Ok(WaitStatus::Exited(_, code)) => Some(code),
        Ok(WaitStatus::Signaled(_, Signal::SIGPIPE, _)) => None,
        Ok(WaitStatus::Signaled(_, sig, _)) => {
            ui::print_signal(&format!("{:?}", sig));
            None
        }
        Ok(_) => None,
        Err(e) => {
            ui::print_error(&format!("waitpid failed: {}", e));
            None
        }
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}