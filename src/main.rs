mod resolve;
mod record;
mod shell;
mod tokenizer;
mod parser;
mod ui;

fn main() -> anyhow::Result<()> {
    let log_path = record::new_session_log_path();
    ui::print_banner(&log_path);
    shell::repl(&log_path)
}
