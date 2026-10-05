# Changelog

All notable changes to this project will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Environment variable snapshot captured in session logs for every command
- Stderr redirection (`2>`, `2>>`, `2>&1`)
- `~` expansion at the start of a word (`cd ~`, `ls ~/src`)
- `exit [n]` exits with `n`, or with the last command's status

### Removed
- Stray `.DS_Store`, `out.txt` and `nospace.txt` files

### Fixed
- Session log path is now canonicalized at startup so logging keeps working after `cd`
- Quoted `|`, `<`, `>` are now treated as plain text instead of operators
- Ctrl+C / Ctrl+\ while a command runs no longer kills patina
- Prompt turns red when the last command exits non-zero; `command not found` exits with 127
- Unset unquoted variables no longer become empty arguments
- History is saved after every command instead of only on exit, and lines starting with a space are kept out of it
- Prompt no longer shows paths like `/home/user2` as `~2`

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
