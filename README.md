# Patina

A shell written in Rust aimed to make reproducible, replayable terminal sessions — like Docker, but for a single terminal session.

## What it is

Patina is a real standalone shell: its own REPL, its own tokenizer, its own process management via raw `fork`/`execv`/`waitpid`. Not a wrapper around bash or another shell.

Every command it runs is resolved against `$PATH` by hand, hashed (SHA-256) for provenance, and logged as a structured JSON record. The long-term goal is to export and replay full sessions faithfully across machines.

## Status

Early and under active development.

**Working:**

- REPL loop with readline editing (Emacs keybindings, `↑↓` history, `Ctrl+R` search)
- Tokenizer: single/double quotes, `$VAR` / `${VAR}` expansion, `~` expansion, backslash escaping
- External command execution via `fork` + `execv` + `waitpid`
- Pipelines: `cmd1 | cmd2 | cmd3`
- I/O redirection: `>`, `>>`, `<`
- Stderr redirection: `2>`, `2>>`, `2>&1`
- Built-ins: `cd`, `exit [n]`
- `Ctrl+C` / `Ctrl+\` interrupt the running command, not the shell
- Binary resolution + SHA-256 hashing for every command run
- Session recording to `~/.patina/sessions/*.jsonl`: the raw line, the full pipeline with redirections, exit codes, `cd`s, the environment (secrets redacted), and fingerprints of files each command read or wrote
- `patina replay`: re-run a session (optionally into another directory) and verify binaries, inputs, exit codes and output files against the recording
- Colored prompt with cwd, git branch, last-command timing, and red `❯` after a failure

**Not yet built:**

- Job control (`bg`, `fg`, `Ctrl+Z`)
- Capturing terminal output (only output redirected to files is fingerprinted)
- Exporting a session together with its input files for use on another machine

## Build & run

```bash
cargo build --release
./target/release/patina
```

To install for your user (puts `patina` in `~/.cargo/bin`):

```bash
cargo install --path .
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

Each session writes one JSONL file to `~/.patina/sessions/` (override with `$PATINA_LOG_DIR`). The first line describes the session; each following line is a command line or a built-in:

```jsonc
{"type":"session","version":2,"patina_version":"0.1.0","started_at":"…","host":{"os":"macos","arch":"aarch64","hostname":"…"},"cwd":"/Users/you/project","env":{"PATH":"…","GITHUB_TOKEN":"<redacted>"}}
{"type":"command","line":"sort < data.txt > sorted.txt","cwd":"/Users/you/project","timestamp":"…","duration_ms":4,
 "stages":[{"cmd":"sort","args":[],"stdin":"data.txt","stdout":{"truncate":"sorted.txt"},"stderr":null,
            "resolved_path":"/usr/bin/sort","binary_sha256":"e595…","exit_code":0}],
 "files":[{"path":"data.txt","role":"stdin","before":{"sha256":"14c5…"}},
          {"path":"sorted.txt","role":"stdout","after":{"sha256":"5366…"}}]}
{"type":"builtin","name":"cd","args":["src"],"cwd":"/Users/you/project","new_cwd":"/Users/you/project/src","ok":true,"timestamp":"…"}
```

- **`stages`** holds every program in a pipeline with its expanded argv, redirections, resolved binary and its SHA-256, and exit code (or `signal`).
- **`files`** fingerprints each file a line touched: `<` inputs and arguments that name existing files (hashed *before*), and `>` / `>>` / `2>` / `2>>` outputs (hashed *after*; `>>` and `2>>` targets are hashed before too).
- **`env`** is captured once in the header; later records carry only an `env_diff`. Variables whose names look secret (`*TOKEN*`, `*KEY*`, `*SECRET*`, `*PASS*`, …), and any value with a password inside a URL, are stored as `<redacted>`. Log files are created readable only by you (`0600`).

## Replay

```bash
patina sessions                          # list recorded sessions
patina replay --dry-run                  # check the latest session without running anything
patina replay                            # re-run it in place (asks first)
patina replay --cwd /tmp/copy SESSION    # re-run it inside another directory
```

For each command line, replay checks that the recorded binary is still byte-identical and that input files match what was recorded, then runs the line and compares exit codes and output-file hashes:

```
[2/3] sort < data.txt > sorted.txt
   ⚠ data.txt differs from when it was recorded (sha256 14c5e74c4b96 → sha256 16fbd7d1f18d)
   ✗ output sorted.txt ended up different (sha256 5366e723d113 recorded, sha256 52a02bc89141 now)
```

With `--cwd`, recorded absolute paths under the session's starting directory (working directories, arguments, redirections) are mapped into the new directory, so you can replay into a scratch copy instead of your real project. Replay exits `1` if anything differed. Values that were redacted at record time come from your current environment.

## Roadmap & contributing

See **[DEVLOG.md](DEVLOG.md)** for planned features (with sizes and pointers into the code, and ⭐ marking good first contributions), the project's history, and how to contribute.

## Why

Built as a learning project to understand what a shell does at the syscall level (`fork`, `exec`, `dup2`, `pipe`), with a real end goal layered on top: making terminal sessions reproducible the way containers made environments reproducible.
