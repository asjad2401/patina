# Changelog

All notable changes to this project will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `patina replay` to re-run a recorded session, optionally into another directory (`--cwd`), verifying binary hashes, input files, exit codes and output files; `--dry-run` checks without running
- `patina sessions` to list recorded sessions
- Session log format v2: a session header (host, patina version, start directory, environment), one record per command line with the raw line and full pipeline structure (redirections included), fingerprints of files read and written, and records for `cd`
- Stderr redirection (`2>`, `2>>`, `2>&1`)
- `~` expansion at the start of a word (`cd ~`, `ls ~/src`)
- `exit [n]` exits with `n`, or with the last command's status

### Changed
- Session logs now live in `~/.patina/sessions` (or `$PATINA_LOG_DIR`) instead of `.patina/sessions` in the launch directory
- The environment is recorded once per session with later changes as diffs
- Binary hashes are streamed and cached by path, size and mtime
- A command killed by a signal sets status 128 + N (130 for Ctrl+C), like bash

### Removed
- Stray `.DS_Store`, `out.txt` and `nospace.txt` files

### Fixed
- Ctrl+C / Ctrl+\ while a command runs no longer kills patina
- Prompt turns red when the last command exits non-zero; `command not found` exits with 127
- Running a missing path (`./missing`) exits with 127, and a non-executable file or a directory with 126, each with bash's message
- `cmd 2>&1 > file` keeps stderr on the terminal, since redirections now apply left to right
- Quoted `|`, `<`, `>` are now treated as plain text instead of operators
- Unset unquoted variables no longer become empty arguments
- Built-ins (`cd`, `exit`) are only handled when they aren't part of a pipeline
- History is saved after every command instead of only on exit, and lines starting with a space are kept out of it
- Prompt no longer shows paths like `/home/user2` as `~2`
- Session log path is now canonicalized at startup so logging keeps working after `cd`

### Security
- Secret-looking environment variables, and passwords inside URLs, are logged as `<redacted>`
- Session logs are created with `0600` permissions

## [0.1.0] - 2026-08-01

### Added
- REPL loop with readline editing (Emacs keybindings, history, Ctrl+R search)
- Tokenizer with single/double quote handling, `$VAR` / `${VAR}` expansion, and backslash escaping
- External command execution via `fork` + `execv` + `waitpid`
- Pipeline support (`cmd1 | cmd2 | cmd3`)
- I/O redirection (`>`, `>>`, `<`)
- Built-in commands: `cd`, `exit`
- Binary resolution against `$PATH` with SHA-256 hashing for provenance
- Per-command JSON session logging to `.patina/sessions/*.jsonl`
- Colored prompt showing cwd, git branch, and last-command timing
- GitHub Actions CI (fmt, clippy, build, test)
- Branch protection on `main`
