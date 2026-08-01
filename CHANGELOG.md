# Changelog

All notable changes to this project will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Environment variable snapshot captured in session logs for every command

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
