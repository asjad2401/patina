# Patina

A shell written in rust aimed to make reproducible, replayable terminal sessions like a docker for a single terminal session.  

## What it is  

Patina is a real standalone shell: its own REPL, its own tokenizer, its own process management via raw `fork`/`execv`/`waitpid`. not a wrapper around bash or another shell.  

Every command it runs is resolved against `$PATH` by hand, hashed (SHA-256) for provenance, and logged as a structured JSON record. The long-term goal is to export and replay full sessions faithfully across machines.  

## Status  

Early and under active development. Currently working:  
 
- REPL loop  
- Real tokenizer: single/double quotes, `$VAR` / `${VAR}` expansion, escaping  
- External command execution via `fork` + `execv` + `waitpid`  
- Built-ins: `cd`, `exit`  
- Output/input redirection: `>`, `>>`, `<`  
- Binary resolution + SHA-256 hashing for every command run  
- Per-command JSON session logging to `.patina/sessions/*.jsonl`  
  
Not yet built: pipes, stderr redirection, signal handling / job control,  
session export & replay.  
  
## Build & run  

```bash  
cargo build --release  
./target/debug/patina        # or target/release/patina after --release  
```  

To install system-wide so you can run it as `patina` from anywhere:  

```bash  
sudo cp target/release/patina /usr/local/bin/  
```  

## Example  
  
```
patina> echo hello world  
hello world  
patina> echo "quoted   spacing" > out.txt  
patina> cat out.txt  
quoted   spacing  
patina> echo $HOME  
/home/you  
```
  
## Why  

Built as a learning project to actually understand what a shell doesat the syscall level (`fork`, `exec`, `dup2`, `pipe`)with a real end goal layered on top: making terminal sessions reproducible, the way containers made environments reproducible.  
