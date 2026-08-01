mod parser;
mod record;
mod resolve;
mod shell;
mod tokenizer;
mod ui;

fn main() -> anyhow::Result<()> {
    let log_path = record::new_session_log_path();
    ui::print_banner(&log_path);
    shell::repl(&log_path)
}
