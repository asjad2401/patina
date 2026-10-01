use anyhow::{anyhow, Result};
use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, PartialEq)]
pub enum Token {
    Word(String),
    Pipe,
    In,
    Out,
    Append,
    Stderr,
    StderrAppend,
    StderrToStdout,
}

pub fn tokenize(line: &str) -> Result<Vec<Token>> {
    let mut chars = line.chars().peekable();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quoted = false;

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '|' | '<' | '>' => {
                chars.next();
                // Only a bare, unquoted `2` right before `>` names stderr.
                let stderr = c == '>' && in_token && !quoted && current == "2";
                if stderr {
                    current.clear();
                } else if in_token {
                    tokens.push(Token::Word(std::mem::take(&mut current)));
                }
                in_token = false;
                quoted = false;
                match c {
                    '|' => tokens.push(Token::Pipe),
                    '<' => tokens.push(Token::In),
                    '>' => tokens.push(redirect_out(&mut chars, stderr)?),
                    _ => {}
                }
            }
            '\'' => {
                chars.next();
                in_token = true;
                quoted = true;
                consume_single_quoted(&mut chars, &mut current)?;
            }
            '"' => {
                chars.next();
                in_token = true;
                quoted = true;
                consume_double_quoted(&mut chars, &mut current)?;
            }
            '\\' => {
                chars.next();
                in_token = true;
                quoted = true;
                match chars.next() {
                    Some(escaped) => current.push(escaped),
                    None => return Err(anyhow!("dangling backslash at end of line")),
                }
            }
            '$' => {
                chars.next();
                in_token = true;
                quoted = true;
                current.push_str(&expand_variable(&mut chars));
            }
            _ => {
                chars.next();
                in_token = true;
                current.push(c);
            }
        }
    }

    if in_token {
        tokens.push(Token::Word(current));
    }

    Ok(tokens)
}

fn redirect_out(chars: &mut Peekable<Chars>, stderr: bool) -> Result<Token> {
    let append = chars.next_if_eq(&'>').is_some();
    if !stderr {
        return Ok(if append { Token::Append } else { Token::Out });
    }
    if append {
        return Ok(Token::StderrAppend);
    }
    if chars.next_if_eq(&'&').is_some() {
        return match chars.next() {
            Some('1') => Ok(Token::StderrToStdout),
            _ => Err(anyhow!("expected '1' after '2>&'")),
        };
    }
    Ok(Token::Stderr)
}

fn consume_single_quoted(chars: &mut Peekable<Chars>, out: &mut String) -> Result<()> {
    loop {
        match chars.next() {
            Some('\'') => return Ok(()),
            Some(c) => out.push(c),
            None => return Err(anyhow!("unterminated single quote")),
        }
    }
}

fn consume_double_quoted(chars: &mut Peekable<Chars>, out: &mut String) -> Result<()> {
    loop {
        match chars.next() {
            Some('"') => return Ok(()),
            Some('\\') => match chars.peek() {
                Some('$') | Some('"') | Some('\\') => {
                    out.push(chars.next().unwrap());
                }
                _ => out.push('\\'),
            },
            Some('$') => {
                out.push_str(&expand_variable(chars));
            }
            Some(c) => out.push(c),
            None => return Err(anyhow!("unterminated double quote")),
        }
    }
}

fn expand_variable(chars: &mut Peekable<Chars>) -> String {
    if chars.peek() == Some(&'{') {
        chars.next(); // consume '{'
        let mut name = String::new();
        for c in chars.by_ref() {
            if c == '}' {
                break;
            }
            name.push(c);
        }
        return std::env::var(&name).unwrap_or_default();
    }

    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }

    if name.is_empty() {
        "$".to_string()
    } else {
        std::env::var(&name).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(s: &str) -> Token {
        Token::Word(s.to_string())
    }

    #[test]
    fn quoted_operators_are_words() {
        assert_eq!(
            tokenize(r#"echo '|' ">" \< '2'>f"#).unwrap(),
            vec![
                word("echo"),
                word("|"),
                word(">"),
                word("<"),
                word("2"),
                Token::Out,
                word("f")
            ]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            tokenize("a|b <i >o >>p 2>e 2>>f 2>&1").unwrap(),
            vec![
                word("a"),
                Token::Pipe,
                word("b"),
                Token::In,
                word("i"),
                Token::Out,
                word("o"),
                Token::Append,
                word("p"),
                Token::Stderr,
                word("e"),
                Token::StderrAppend,
                word("f"),
                Token::StderrToStdout,
            ]
        );
    }
}
