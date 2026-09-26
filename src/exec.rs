use crate::parser::{Command, StderrRedirect, StdoutRedirect};
use crate::ui;
use anyhow::{anyhow, Result};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{dup2, execv, fork, pipe as nix_pipe, ForkResult, Pid};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub enum Status {
    Exited(i32),
    Signaled(Signal),
    Unknown,
}

impl Status {
    /// Shell-style status: the exit code, or 128 + N when killed by signal N.
    pub fn code(&self) -> i32 {
        match self {
            Status::Exited(c) => *c,
            Status::Signaled(s) => 128 + *s as i32,
            Status::Unknown => 1,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Status::Exited(c) => Some(*c),
            _ => None,
        }
    }

    pub fn signal_name(&self) -> Option<String> {
        match self {
            Status::Signaled(s) => Some(s.as_str().to_string()),
            _ => None,
        }
    }
}

fn set_disposition(signal: Signal, handler: SigHandler) {
    unsafe {
        let _ = sigaction(
            signal,
            &SigAction::new(handler, SaFlags::empty(), SigSet::empty()),
        );
    }
}

/// The shell itself must survive Ctrl+C / Ctrl+\ while a child is running;
/// the terminal delivers those to the whole foreground process group.
pub fn ignore_interactive_signals() {
    set_disposition(Signal::SIGINT, SigHandler::SigIgn);
    set_disposition(Signal::SIGQUIT, SigHandler::SigIgn);
}

/// Ignored dispositions survive exec, so children must restore the defaults.
fn reset_child_signals() {
    for sig in [Signal::SIGPIPE, Signal::SIGINT, Signal::SIGQUIT] {
        set_disposition(sig, SigHandler::SigDfl);
    }
}

enum StderrTarget {
    File(File),
    ToStdout,
}

struct Redirects {
    stdin: Option<File>,
    stdout: Option<File>,
    stderr: Option<StderrTarget>,
}

fn open_write(path: &str, append: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true);
    if append {
        opts.append(true);
    } else {
        opts.write(true).truncate(true);
    }
    opts.open(path)
        .map_err(|e| anyhow!("cannot open '{}' for writing: {}", path, e))
}

fn open_redirects(command: &Command) -> Result<Redirects> {
    let stdin = match &command.stdin {
        Some(path) => Some(
            File::open(path).map_err(|e| anyhow!("cannot open '{}' for reading: {}", path, e))?,
        ),
        None => None,
    };
    let stdout = match &command.stdout {
        Some(StdoutRedirect::Truncate(path)) => Some(open_write(path, false)?),
        Some(StdoutRedirect::Append(path)) => Some(open_write(path, true)?),
        None => None,
    };
    let stderr = match &command.stderr {
        Some(StderrRedirect::Truncate(path)) => Some(StderrTarget::File(open_write(path, false)?)),
        Some(StderrRedirect::Append(path)) => Some(StderrTarget::File(open_write(path, true)?)),
        Some(StderrRedirect::ToStdout) => Some(StderrTarget::ToStdout),
        None => None,
    };
    Ok(Redirects {
        stdin,
        stdout,
        stderr,
    })
}

fn build_argv(command: &Command) -> Result<Vec<CString>> {
    let mut argv_c = Vec::with_capacity(command.args.len() + 1);
    argv_c.push(CString::new(command.cmd.as_str())?);
    for a in &command.args {
        argv_c.push(CString::new(a.as_str())?);
    }
    Ok(argv_c)
}

/// Runs `stages` as a pipeline (a single stage is a pipeline of one) and
/// waits for every process. Each stage is paired with its resolved binary.
/// Relative redirect paths are opened against the current directory.
pub fn run(stages: &[(&Command, &Path)]) -> Result<Vec<Status>> {
    let n = stages.len();

    let mut redirects = Vec::with_capacity(n);
    for (c, _) in stages {
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

    let mut child_pids: Vec<Pid> = Vec::with_capacity(n);

    for (i, (command, resolved)) in stages.iter().enumerate() {
        let argv_c = build_argv(command)?;
        let path_c = CString::new(resolved.to_string_lossy().as_bytes())?;
        let r = &redirects[i];

        // Explicit redirections win over pipe ends, so apply them second.
        let mut stdin_fds = Vec::new();
        let mut stdout_fds = Vec::new();
        if i > 0 {
            stdin_fds.push(pipes[i - 1].0.as_raw_fd());
        }
        if let Some(f) = &r.stdin {
            stdin_fds.push(f.as_raw_fd());
        }
        if i < n - 1 {
            stdout_fds.push(pipes[i].1.as_raw_fd());
        }
        if let Some(f) = &r.stdout {
            stdout_fds.push(f.as_raw_fd());
        }
        let stderr_fd = match &r.stderr {
            Some(StderrTarget::File(f)) => Some(f.as_raw_fd()),
            _ => None,
        };
        let dup_stderr_to_stdout = matches!(r.stderr, Some(StderrTarget::ToStdout));

        match unsafe { fork() }? {
            ForkResult::Child => {
                reset_child_signals();
                for fd in stdin_fds {
                    if dup2(fd, 0).is_err() {
                        std::process::exit(126);
                    }
                }
                for fd in stdout_fds {
                    if dup2(fd, 1).is_err() {
                        std::process::exit(126);
                    }
                }
                if let Some(fd) = stderr_fd {
                    if dup2(fd, 2).is_err() {
                        std::process::exit(126);
                    }
                }
                if dup_stderr_to_stdout && dup2(1, 2).is_err() {
                    std::process::exit(126);
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

    Ok(child_pids.into_iter().map(wait_for).collect())
}

fn wait_for(pid: Pid) -> Status {
    match waitpid(pid, None) {
        Ok(WaitStatus::Exited(_, code)) => Status::Exited(code),
        Ok(WaitStatus::Signaled(_, sig, _)) => {
            match sig {
                Signal::SIGPIPE => {}
                // The terminal already echoed ^C; just end the line cleanly.
                Signal::SIGINT => eprintln!(),
                _ => ui::print_signal(&format!("{:?}", sig)),
            }
            Status::Signaled(sig)
        }
        Ok(_) => Status::Unknown,
        Err(e) => {
            ui::print_error(&format!("waitpid failed: {}", e));
            Status::Unknown
        }
    }
}
