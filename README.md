# Patina

A shell written in Rust aimed to make reproducible, replayable terminal sessions — like Docker, but for a single terminal session.

## What it is

Patina is a real standalone shell: its own REPL, its own tokenizer, its own process management via raw `fork`/`execv`/`waitpid`. Not a wrapper around bash or another shell.

Every command it runs is resolved against `$PATH` by hand, hashed (SHA-256) for provenance, and logged as a structured JSON record. The long-term goal is to export and replay full sessions faithfully across machines.

## Status

Early and under active development.

**Working:**

- REPL loop with readline editing (Emacs keybindings, `↑↓` history, `Ctrl+R` search)
- Tokenizer: single/double quotes, `$VAR` / `${VAR}` expansion, backslash escaping
- External command execution via `fork` + `execv` + `waitpid`
- Pipelines: `cmd1 | cmd2 | cmd3`
- I/O redirection: `>`, `>>`, `<`
- Stderr redirection: `2>`, `2>&1`
- Built-ins: `cd`, `exit`
- Binary resolution + SHA-256 hashing for every command run
- Per-command JSON session logging to `.patina/sessions/*.jsonl`
- Colored prompt with cwd, git branch, and last-command timing

**Not yet built:**

- Job control (`bg`, `fg`, `Ctrl+Z`)
- Environment variable capture in session logs
- Session export & replay

## Build & run

```bash
cargo build --release
./target/release/patina
```

To install system-wide:

```bash
sudo cp target/release/patina /usr/local/bin/
```

## Example

```
 ⬡ patina  ~/project   main  ❯ echo hello world
hello world
 ⬡ patina  ~/project   main  ❯ echo "quoted   spacing" > out.txt
 ⬡ patina  ~/project   main  ❯ cat out.txt
quoted   spacing
 ⬡ patina  ~/project   main  ❯ ls src | grep -v mod
main.rs
```

## Session logs

Each session writes a `.patina/sessions/<timestamp>.jsonl` file. Every command is one JSON line:

```json
{
  "command": "grep",
  "args": ["-r", "fn main", "src/"],
  "resolved_path": "/usr/bin/grep",
  "binary_sha256": "a3f1...",
  "cwd": "/home/you/project",
  "timestamp": "2026-08-01T10:00:00Z",
  "exit_code": 0,
  "duration_ms": 12
}
```

The SHA-256 hash of each binary is the key primitive for future replay: it lets patina verify at replay time that the exact same binary is being used, or warn when it isn't.

## Why

Built as a learning project to understand what a shell does at the syscall level (`fork`, `exec`, `dup2`, `pipe`), with a real end goal layered on top: making terminal sessions reproducible the way containers made environments reproducible.
