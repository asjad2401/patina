mod exec;
mod parser;
mod record;
mod replay;
mod resolve;
mod shell;
mod tokenizer;
mod ui;

const USAGE: &str = "usage: patina [COMMAND]

  (no command)   start an interactive, recorded shell session
  replay ...     re-run a recorded session and compare the results
                 (see 'patina replay --help')
  sessions       list recorded sessions
  --version      print the version

Sessions are logged to $PATINA_LOG_DIR, or ~/.patina/sessions by default.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run_shell(),
        Some("replay") => replay::main(&args[1..]),
        Some("sessions") => list_sessions(),
        Some("--version" | "-V") => {
            println!("patina {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("--help" | "-h" | "help") => {
            println!("{}", USAGE);
            Ok(0)
        }
        Some(other) => Err(anyhow::anyhow!("unknown command '{}'\n\n{}", other, USAGE)),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            ui::print_error(&format!("{:#}", e));
            std::process::exit(2);
        }
    }
}

fn run_shell() -> anyhow::Result<i32> {
    let mut recorder = record::Recorder::start()?;
    ui::print_banner(recorder.path());
    shell::repl(&mut recorder)
}

fn list_sessions() -> anyhow::Result<i32> {
    let sessions = record::list_sessions()?;
    if sessions.is_empty() {
        println!("No sessions in {}", record::log_dir().display());
    }
    for path in sessions {
        let summary = match record::read_log(&path) {
            Ok((h, entries)) => {
                let n = entries
                    .iter()
                    .filter(|e| matches!(e, record::Entry::Command(_)))
                    .count();
                format!("{} command line(s) · started in {}", n, h.cwd)
            }
            Err(_) => "old format, not replayable".to_string(),
        };
        println!("{}  {}", path.display(), ui::dim(&summary));
    }
    Ok(0)
}
