use crate::tokenizer::Token;
use anyhow::{anyhow, Result};

#[derive(Debug)]
pub enum StdoutRedirect {
    Truncate(String),
    Append(String),
}

#[derive(Debug)]
pub enum StderrRedirect {
    Truncate(String),
    Append(String),
    ToStdout,
}

#[derive(Debug)]
pub struct Command {
    pub cmd: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub stdout: Option<StdoutRedirect>,
    pub stderr: Option<StderrRedirect>,
}

pub fn parse_pipeline(tokens: Vec<Token>) -> Result<Vec<Command>> {
    let mut stages: Vec<Vec<Token>> = vec![Vec::new()];
    for tok in tokens {
        if tok == Token::Pipe {
            stages.push(Vec::new());
        } else {
            stages.last_mut().unwrap().push(tok);
        }
    }

    if stages.iter().any(|s| s.is_empty()) {
        return Err(anyhow!(
            "empty command in pipeline (check for '||' or a stray '|')"
        ));
    }

    stages.into_iter().map(parse).collect()
}

pub fn parse(tokens: Vec<Token>) -> Result<Command> {
    let mut argv: Vec<String> = Vec::new();
    let mut stdin = None;
    let mut stdout = None;
    let mut stderr = None;

    let mut iter = tokens.into_iter();
    while let Some(tok) = iter.next() {
        match tok {
            Token::Word(word) => argv.push(word),
            Token::Out => stdout = Some(StdoutRedirect::Truncate(filename(&mut iter, ">")?)),
            Token::Append => stdout = Some(StdoutRedirect::Append(filename(&mut iter, ">>")?)),
            Token::In => stdin = Some(filename(&mut iter, "<")?),
            Token::Stderr => stderr = Some(StderrRedirect::Truncate(filename(&mut iter, "2>")?)),
            Token::StderrAppend => {
                stderr = Some(StderrRedirect::Append(filename(&mut iter, "2>>")?))
            }
            Token::StderrToStdout => stderr = Some(StderrRedirect::ToStdout),
            Token::Pipe => return Err(anyhow!("unexpected '|'")),
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
        stderr,
    })
}

fn filename(iter: &mut impl Iterator<Item = Token>, op: &str) -> Result<String> {
    match iter.next() {
        Some(Token::Word(word)) => Ok(word),
        _ => Err(anyhow!("expected filename after '{}'", op)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::tokenize;

    fn parse_line(line: &str) -> Result<Vec<Command>> {
        parse_pipeline(tokenize(line).unwrap())
    }

    #[test]
    fn quoted_pipe_does_not_split() {
        let cmds = parse_line("echo '|' x").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].args, ["|", "x"]);
    }

    #[test]
    fn stderr_append() {
        let cmd = &parse_line("ls missing 2>>err.log").unwrap()[0];
        assert_eq!(cmd.args, ["missing"]);
        assert!(matches!(&cmd.stderr, Some(StderrRedirect::Append(f)) if f == "err.log"));
    }

    #[test]
    fn redirect_needs_filename() {
        assert!(parse_line("echo > > f").is_err());
    }
}
