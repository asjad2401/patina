mod resolve;
mod record;
mod shell;
mod tokenizer;

fn main() -> anyhow::Result<()> {
    let log_path = record::new_session_log_path();
    println!("Patina v0 - session log: {}", log_path.display());
    shell::repl(&log_path)
}
