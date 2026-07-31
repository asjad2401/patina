use anyhow::{anyhow, Result};

#[derive(Debug)]
pub enum StdoutRedirect {
    Truncate(String),
    Append(String),
}

#[derive(Debug)]
pub struct Command {
    pub cmd: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub stdout: Option<StdoutRedirect>,
}

pub fn parse_pipeline(tokens: Vec<String>) -> Result<Vec<Command>> {
    let mut stages: Vec<Vec<String>> = vec![Vec::new()];
    for tok in tokens {
        if tok == "|" {
            stages.push(Vec::new());
        } else {
            stages.last_mut().unwrap().push(tok);
        }
    }

    if stages.iter().any(|s| s.is_empty()) {
        return Err(anyhow!("empty command in pipeline (check for '||' or a stray '|')"));
    }

    stages.into_iter().map(parse).collect()
}

pub fn parse(tokens: Vec<String>) -> Result<Command> {
    let mut argv: Vec<String> = Vec::new();
    let mut stdin = None;
    let mut stdout = None;

    let mut iter = tokens.into_iter();
    while let Some(tok) = iter.next() {
        match tok.as_str() {
            ">" => {
                let file = iter
                    .next()
                    .ok_or_else(|| anyhow!("expected filename after '>'"))?;
                stdout = Some(StdoutRedirect::Truncate(file));
            }
            ">>" => {
                let file = iter
                    .next()
                    .ok_or_else(|| anyhow!("expected filename after '>>'"))?;
                stdout = Some(StdoutRedirect::Append(file));
            }
            "<" => {
                let file = iter
                    .next()
                    .ok_or_else(|| anyhow!("expected filename after '<'"))?;
                stdin = Some(file);
            }
            _ => argv.push(tok),
        }
    }

    if argv.is_empty() {
        return Err(anyhow!("empty command after removing redirections"));
    }

    let cmd = argv.remove(0);
    Ok(Command {
        cmd,
        args: argv,
        stdin,
        stdout,
    })
}