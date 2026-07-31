use anyhow::{anyhow, Result};
use std::iter::Peekable;
use std::str::Chars;

pub fn tokenize(line: &str) -> Result<Vec<String>> {
    let mut chars = line.chars().peekable();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' => {
                chars.next();
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '|' => {
                chars.next();
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
                tokens.push("|".to_string());
            }
            '>' => {
                chars.next();
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(">>".to_string());
                } else {
                    tokens.push(">".to_string());
                }
            }
            '<' => {
                chars.next();
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
                tokens.push("<".to_string());
            }
            '\'' => {
                chars.next();
                in_token = true;
                consume_single_quoted(&mut chars, &mut current)?;
            }
            '"' => {
                chars.next();
                in_token = true;
                consume_double_quoted(&mut chars, &mut current)?;
            }
            '\\' => {
                chars.next();
                in_token = true;
                match chars.next() {
                    Some(escaped) => current.push(escaped),
                    None => return Err(anyhow!("dangling backslash at end of line")),
                }
            }
            '$' => {
                chars.next();
                in_token = true;
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
        tokens.push(current);
    }

    Ok(tokens)
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