const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

const FG_WHITE: &str = "\x1b[97m";
const FG_CYAN: &str = "\x1b[96m";
const FG_RED: &str = "\x1b[91m";
const FG_YELLOW: &str = "\x1b[93m";

const BG_MAGENTA: &str = "\x1b[45m";
const BG_DARK: &str = "\x1b[48;5;236m";

pub fn print_banner(log_path: &std::path::Path) {
    let cyan_bold = format!("{}{}", BOLD, FG_CYAN);
    let banner = format!(
        r#"
{c} ██████╗  █████╗ ████████╗██╗███╗   ██╗ █████╗ {r}
{c} ██╔══██╗██╔══██╗╚══██╔══╝██║████╗  ██║██╔══██╗{r}
{c} ██████╔╝███████║   ██║   ██║██╔██╗ ██║███████║{r}
{c} ██╔═══╝ ██╔══██║   ██║   ██║██║╚██╗██║██╔══██║{r}
{c} ██║     ██║  ██║   ██║   ██║██║ ╚████║██║  ██║{r}
{c} ╚═╝     ╚═╝  ╚═╝   ╚═╝   ╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝{r}
{dim}  A smarter shell  ·  v{ver}  ·  log → {log}{r}
{dim}  ↑↓ history  ·  Ctrl+R search  ·  type 'exit' to quit{r}
"#,
        c = cyan_bold,
        r = RESET,
        dim = DIM,
        ver = env!("CARGO_PKG_VERSION"),
        log = log_path.display(),
    );
    println!("{}", banner);
}

pub fn build_prompt(last_ok: bool, last_duration_ms: Option<u128>) -> String {
    let cwd = cwd_segment();
    let git = git_segment();
    let timing = timing_segment(last_duration_ms);
    let glyph_color = if last_ok { FG_CYAN } else { FG_RED };

    let badge = format!("{}{} ⬡ patina {}{}", BG_MAGENTA, FG_WHITE, RESET, RESET);

    let cwd_part = format!("{}{} {} {}", BG_DARK, FG_CYAN, cwd, RESET);

    let git_part = if git.is_empty() {
        String::new()
    } else {
        format!("{}{} {} {} {}", BG_DARK, FG_YELLOW, git, RESET, RESET)
    };

    let timing_part = if timing.is_empty() {
        String::new()
    } else {
        format!("{}{}{}{} ", DIM, FG_YELLOW, timing, RESET)
    };

    let glyph = format!("{}{}{}{}{} ", BOLD, glyph_color, "❯", RESET, RESET);

    format!("{}{}{} {}{}", badge, cwd_part, git_part, timing_part, glyph)
}

pub fn print_error(msg: &str) {
    eprintln!("{}{}✗  patina: {}{}", BOLD, FG_RED, msg, RESET);
}
#[allow(dead_code)]
pub fn print_warn(msg: &str) {
    eprintln!("{}⚠  patina: {}{}", FG_YELLOW, msg, RESET);
}
pub fn print_signal(sig: &str) {
    eprintln!("{}{}  Killed by signal: {}{}", DIM, FG_RED, sig, RESET);
}

fn cwd_segment() -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());

    let home = std::env::var("HOME").unwrap_or_default();
    if let Some(rest) = under_home(&cwd, &home) {
        format!("~{}", rest)
    } else {
        let parts: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() > 3 {
            format!("…/{}", parts[parts.len() - 3..].join("/"))
        } else {
            cwd
        }
    }
}

// Whole path components only, so /home/user2 isn't treated as inside /home/user.
fn under_home<'a>(cwd: &'a str, home: &str) -> Option<&'a str> {
    let home = home.trim_end_matches('/');
    let rest = cwd.strip_prefix(home)?;
    (!home.is_empty() && (rest.is_empty() || rest.starts_with('/'))).then_some(rest)
}

fn git_segment() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .stderr(std::process::Stdio::null())
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if branch.is_empty() || branch == "HEAD" {
                let sha = std::process::Command::new("git")
                    .args(["rev-parse", "--short", "HEAD"])
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|| "HEAD".to_string());
                format!(" {}", sha)
            } else {
                format!(" {}", branch)
            }
        }
        _ => String::new(),
    }
}

fn timing_segment(duration_ms: Option<u128>) -> String {
    match duration_ms {
        Some(ms) if ms >= 1_000 => {
            let secs = ms as f64 / 1_000.0;
            format!("took {:.1}s", secs)
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::under_home;

    #[test]
    fn home_prefix_matches_whole_components() {
        assert_eq!(under_home("/home/user", "/home/user"), Some(""));
        assert_eq!(under_home("/home/user/src", "/home/user/"), Some("/src"));
        assert_eq!(under_home("/home/user2", "/home/user"), None);
        assert_eq!(under_home("/home/user", ""), None);
    }
}
