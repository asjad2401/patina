use crate::exec;
use crate::parser::{self, Command};
use crate::record::{BuiltinRecord, CommandRecord, Entry, FileWatch, Recorder, StageRecord};
use crate::resolve::{hash_file, resolve_binary, CommandNotFound};
use crate::tokenizer::{tokenize, Token};
use crate::ui;
use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::path::PathBuf;
use std::time::Instant;

pub fn repl(recorder: &mut Recorder) -> Result<i32> {
    exec::ignore_interactive_signals();

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .build();

    let mut rl: Editor<(), rustyline::history::FileHistory> = Editor::with_config(config)?;

    let history_path = dirs_home().map(|h| h.join(".patina_history"));
    if let Some(ref p) = history_path {
        let _ = rl.load_history(p);
    }

    let mut last_status = 0;
    let mut last_duration_ms: Option<u128> = None;

    loop {
        let prompt = ui::build_prompt(last_status == 0, last_duration_ms);

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

        if line.trim().is_empty() {
            continue;
        }

        // Keep the leading space: history_ignore_space uses it to skip the entry.
        let _ = rl.add_history_entry(line.trim_end());
        if let Some(ref p) = history_path {
            let _ = rl.append_history(p);
        }
        let line = line.trim();

        let tokens = match tokenize(line) {
            Ok(t) if t.is_empty() => continue,
            Ok(t) => t,
            Err(e) => {
                ui::print_error(&format!("parse error: {}", e));
                last_status = 1;
                last_duration_ms = None;
                continue;
            }
        };

        // Built-ins only apply when they're the whole line, not a pipeline stage.
        if !tokens.contains(&Token::Pipe) {
            let words: Vec<String> = tokens
                .iter()
                .filter_map(|t| match t {
                    Token::Word(w) => Some(w.clone()),
                    _ => None,
                })
                .collect();
            match words.first().map(String::as_str) {
                Some("exit") => {
                    if let Some(arg) = words.get(1) {
                        match arg.parse() {
                            Ok(code) => last_status = code,
                            Err(_) => {
                                ui::print_error(&format!(
                                    "exit: {}: numeric argument required",
                                    arg
                                ));
                                last_status = 1;
                                last_duration_ms = None;
                                continue;
                            }
                        }
                    }
                    break;
                }
                Some("cd") => {
                    last_status = if builtin_cd(&words[1..], recorder) {
                        0
                    } else {
                        1
                    };
                    last_duration_ms = None;
                    continue;
                }
                _ => {}
            }
        }

        let commands = match parser::parse_pipeline(tokens) {
            Ok(c) => c,
            Err(e) => {
                ui::print_error(&e.to_string());
                last_status = 1;
                last_duration_ms = None;
                continue;
            }
        };

        let t0 = Instant::now();
        let result = run_and_record(line, commands, recorder);
        last_duration_ms = Some(t0.elapsed().as_millis());

        match result {
            Ok(status) => last_status = status,
            Err(e) => {
                ui::print_error(&e.to_string());
                last_status = if e.is::<CommandNotFound>() { 127 } else { 1 };
            }
        }
    }

    Ok(last_status)
}

fn builtin_cd(args: &[String], recorder: &Recorder) -> bool {
    let before = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let target = args
        .first()
        .cloned()
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));

    let ok = match std::env::set_current_dir(&target) {
        Ok(()) => true,
        Err(e) => {
            ui::print_error(&format!("cd: {}: {}", target, e));
            false
        }
    };

    let entry = Entry::Builtin(BuiltinRecord {
        name: "cd".into(),
        args: args.to_vec(),
        cwd: before,
        new_cwd: std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
        ok,
        timestamp: chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = recorder.append(&entry) {
        ui::print_error(&format!("could not write session log: {}", e));
    }
    ok
}

/// Runs one parsed line and logs it. Returns the last stage's status, like
/// `$?` in other shells.
fn run_and_record(line: &str, commands: Vec<Command>, recorder: &mut Recorder) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let timestamp = chrono::Utc::now().to_rfc3339();
    let env_diff = recorder.take_env_diff();

    let mut resolved: Vec<PathBuf> = Vec::with_capacity(commands.len());
    let mut hashes = Vec::with_capacity(commands.len());
    for c in &commands {
        let path = resolve_binary(&c.cmd)?;
        hashes.push(hash_file(&path)?);
        resolved.push(path);
    }

    let watch = FileWatch::before(&commands, &cwd);

    let start = Instant::now();
    let stages: Vec<_> = commands
        .iter()
        .zip(&resolved)
        .map(|(c, p)| (c, p.as_path()))
        .collect();
    let statuses = exec::run(&stages)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let files = watch.finish();
    let status = statuses.last().map_or(1, |s| s.code());

    let stages = commands
        .into_iter()
        .zip(resolved)
        .zip(hashes)
        .zip(&statuses)
        .map(|(((command, path), hash), status)| StageRecord {
            command,
            resolved_path: path.to_string_lossy().to_string(),
            binary_sha256: hash,
            exit_code: status.exit_code(),
            signal: status.signal_name(),
        })
        .collect();

    recorder.append(&Entry::Command(CommandRecord {
        line: line.to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        timestamp,
        duration_ms,
        env_diff,
        stages,
        files,
    }))?;

    Ok(status)
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}
