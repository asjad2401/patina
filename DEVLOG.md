# Patina devlog

This file has two parts:

- **[Roadmap](#roadmap)** — features that aren't built yet, why each one matters, and where it would go in the code. Pick one up if you'd like to contribute.
- **[Log](#log)** — what has been built so far and the reasons behind it, newest first.

For exact changes per release, see [CHANGELOG.md](CHANGELOG.md).

## The goal

Patina is a shell that makes terminal sessions **reproducible**, like Docker but for a single terminal session. It records exactly what ran: binaries by SHA-256, inputs, outputs, environment and working directories. It can then replay a session and say precisely what changed. The end state is recording a session on one machine and replaying it faithfully on another.

## Where things stand (2026-10-05)

- **Built:** a standalone shell (its own tokenizer, parser, `fork`/`execv`/`dup2`/`pipe`), session recording (log format v2), and `patina replay` with drift detection.
- **Not built yet:** moving a session to another machine. Replay can't run there because the starting files aren't included.
- **Size:** about 2,200 lines of Rust, 21 tests, and CI on every PR (fmt, clippy, build, test).

### Code map

| File | What it does |
|---|---|
| `src/main.rs` | CLI entry: `patina`, `patina replay`, `patina sessions` |
| `src/shell.rs` | REPL loop and built-ins (`cd`, `exit`); records each line |
| `src/tokenizer.rs` | Line → typed tokens (quotes, `$VAR`, `~`, operators) |
| `src/parser.rs` | Tokens → pipeline of commands with redirections |
| `src/exec.rs` | `fork`/`execv`/`pipe`/`dup2`, signal handling, exit statuses |
| `src/resolve.rs` | `$PATH` lookup, 126/127 errors, cached SHA-256 of binaries |
| `src/record.rs` | Session log format v2, env redaction and diffs, file fingerprints |
| `src/replay.rs` | `patina replay`: checks, re-execution, comparison report |
| `src/ui.rs` | Prompt, banner, colored output |
| `tests/` | End-to-end tests that pipe scripts into the real binary |

---

## Roadmap

Each item has a size (**S** is an afternoon, **M** is a few evenings, **L** is a project) and a pointer into the code. Items marked ⭐ are good first contributions.

Before starting on an item, please open an issue (or comment on an existing one) so two people don't build the same thing.

### Milestone 1: Portable sessions (next up)

The step that turns "replay on my machine" into "replay anywhere".

- **`patina export` — bundle a session with its starting state.** **L**
  Every file in the log with a `before` fingerprint that the session didn't create itself is part of the starting state. Export packs those files, the log and a manifest into one archive (`session.patina`). Replay then accepts that archive, unpacks the starting state into `--cwd`, and runs.
  *Where:* new `src/export.rs`, `Entry`/`FileRecord` in `src/record.rs`, `find_session` in `src/replay.rs`.
  *Open questions:* the archive format (tar + zstd?), a size limit for bundled files, and whether files outside the start directory should be bundled.

- **Binary identity across machines.** **M**
  On another OS the SHA-256 never matches, so every binary shows as "changed". Recording the binary's version (for example `sort --version`, or the package manager's view) would let replay say "same tool, different build" or "different major version".
  *Where:* `StageRecord` in `src/record.rs`, the binary check in `replay_one`.

- **Replay into a fresh temporary directory by default.** **S** ⭐
  `patina replay --sandbox` creates a temp directory, unpacks or copies the starting state there, and prints where it is. This makes it harder to accidentally replay over your real project.
  *Where:* `src/replay.rs`.

### Milestone 2: Replay fidelity

- **Compare terminal output.** **L**
  Right now only output redirected to files is fingerprinted. Capturing what a command prints to the terminal needs a pseudo-terminal between patina and the child, like `script(1)` uses. A plain pipe would make `ls`, `git`, `vim` and `less` behave differently, because they check whether they're writing to a terminal. Record a hash of the output, and optionally the output itself.
  *Where:* `src/exec.rs` (pty setup, forwarding input and window size), `StageRecord`.

- **Full left-to-right redirection model.** **M**
  `> a 2>&1 > b` should send stderr to `a`, but patina sends it to `b`. Fixing it properly means keeping an ordered list of file-descriptor operations instead of one slot per stream. That also opens the door to `3>`, `>&2` and `&>`.
  *Where:* `Command` in `src/parser.rs`, the child setup in `src/exec.rs`. The log format needs a version bump (see "Log format migrations" below).

- **Record and restore the umask and locale.** **S** ⭐
  Both silently change command behavior: file permissions, `sort` order, and date formats. Capture them in the session header, then restore them on replay and warn if they differ.
  *Where:* `SessionHeader` in `src/record.rs`, `apply_recorded_env` in `src/replay.rs`.

### Milestone 3: Everyday shell features

These make patina pleasant enough to use as your daily shell, which is how sessions worth replaying get recorded.

- **Globbing** (`*.rs`, `src/**`, `?`, `[abc]`). **M**
  `ls *.rs` currently passes the literal `*.rs`. Globs must expand after tokenizing, only for unquoted words, and the *expanded* argv is what gets recorded.
  *Where:* `src/tokenizer.rs` (mark which words are unquoted), a new expansion step before `parser::parse_pipeline`.

- **`export`, `unset` and `VAR=value cmd`.** **M**
  This is also the first thing that makes the log's `env_diff` field do real work.
  *Where:* built-ins in `src/shell.rs`, `Recorder::take_env_diff` already computes diffs.

- **`$?`** (last exit status). **S** ⭐
  `last_status` already exists in the REPL loop and just needs to reach variable expansion.
  *Where:* `expand_variable` in `src/tokenizer.rs`, `repl` in `src/shell.rs`.

- **`;`, `&&` and `||`.** **M**
  Each command line becomes a list of pipelines with conditions. Decide how this looks in the log: one record per pipeline, or one per line.
  *Where:* `src/tokenizer.rs`, `src/parser.rs`, `run_and_record` in `src/shell.rs`.

- **`cd -` and `$OLDPWD`.** **S** ⭐
  *Where:* `builtin_cd` in `src/shell.rs`.

- **`#` comments.** **S** ⭐
  Ignore the rest of the line after an unquoted `#` at the start of a word.
  *Where:* `src/tokenizer.rs`.

- **Tab completion** for commands on `$PATH` and file paths. **M**
  rustyline supports this through a `Completer`; the editor is currently `Editor<(), …>`.
  *Where:* `repl` in `src/shell.rs`.

### Milestone 4: Job control

- **Process groups, `Ctrl+Z`, `fg`, `bg` and `jobs`.** **L**
  Each pipeline gets its own process group (`setpgid`), and patina hands the terminal to it (`tcsetpgrp`). This replaces the current approach of ignoring `SIGINT`/`SIGQUIT` in the shell.
  *Where:* `src/exec.rs`; built-ins in `src/shell.rs`.

### Housekeeping

- **CI on macOS too.** **S** ⭐
  Patina is developed on macOS, but CI only runs Ubuntu. Add `macos-latest` to a matrix in `.github/workflows/ci.yml`.
- **Log format migrations.** **M**
  Logs from before v2 can't be replayed at all right now. Add a small upgrade path, or at least a `patina sessions --prune-old`, before the next format change.
- **`patina sessions` filters** (`--since`, `--in DIR`) and a way to delete old sessions. **S** ⭐

---

## Log

### 2026-10-05 — Session replay lands (#23)

The project's main goal took its first real step. Recording was rebuilt around what replay needs:

- **Session log v2.** Each log starts with a header (host, patina version, start directory, environment captured once). It then has one record per input line, holding the raw text and the full pipeline with redirections, plus records for `cd`.
- **File fingerprints.** Files a line reads (`<`, file arguments, `>>` targets) are hashed before it runs. Files it writes are hashed after.
- **`patina replay`** checks binaries and inputs against the recording, re-runs each line, and compares exit codes and output files. `--cwd` replays into another directory and maps recorded paths into it.
- **Logs moved to `~/.patina/sessions`** so the latest session can be found from any directory.

The same PR fixed the last open bugs:

- #18: `2>&1 > file` ordering.
- #19: 126 vs 127 for paths.
- #20: binaries are now streamed and cached instead of read whole on every command.
- #21: the environment is no longer repeated on every log line.

**Decision:** terminal output is not captured yet. Doing it through a pipe would change how programs like `ls`, `vim` and `less` behave, and doing it properly needs a pseudo-terminal (see Milestone 2).

### 2026-10-05 — Bug bash by @ranaumarnadeem (#4–#17, PR #22)

An outside contributor went through patina carefully and filed 18 issues, each with repro steps and the cause. Their PR fixed 14 of them:

- **Ctrl+C:** Ctrl+C no longer kills the shell.
- **Tokenizer and parser:**
  - A typed tokenizer, so quoted `|`, `<` and `>` stay plain text.
  - `2>>`.
  - `~` expansion.
  - Empty unquoted variables are dropped.
- **Exit statuses:**
  - Real exit statuses, with `exit [n]` and 127 for command not found.
  - A red prompt on failure.
- **History:**
  - It's saved after every command.
  - Lines starting with a space stay out of it.
- **Session log security:**
  - Secret-looking environment values and passwords in URLs are redacted.
  - Log files are created `0600`.
- **First unit tests:** the project's first 9.

### 2026-08-09 — Stderr redirection

- `2>` and `2>&1` work.
- Session log paths are canonicalized so logging survives `cd`.

### 2026-08-01 — v0.1.0

- GitHub Actions CI: fmt, clippy, build, test.
- Branch protection on `main`.
- An environment snapshot in every log record (#3). This was later reworked into the v2 header with secrets redacted.

### 2026-07-29 to 2026-07-31 — First commits

- A REPL with its own tokenizer and `$PATH` resolver.
- Commands run through raw `fork`/`execv`/`waitpid` instead of wrapping another shell.
- Pipelines and `>`, `>>`, `<`.
- Every command is hashed (SHA-256) and logged as JSON.
- A colored prompt with git branch and timing.

---

## Contributing

```bash
git clone https://github.com/asjad2401/patina && cd patina
cargo run                        # start the shell from source
cargo test                       # unit + end-to-end tests
cargo fmt && cargo clippy --all-targets -- -D warnings
```

1. Open or comment on an issue for the roadmap item you're taking.
2. Work on a branch in your fork, not your fork's `main`.
3. Add tests. The files in `tests/` show how to drive the real binary by piping a script into it.
4. CI must pass: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
5. In the same PR, update `CHANGELOG.md` under *Unreleased*. If the PR finishes a roadmap item, move it into the log here with a short "what and why".
6. If you change the session log format, bump `FORMAT_VERSION` in `src/record.rs` and say so in the PR.
