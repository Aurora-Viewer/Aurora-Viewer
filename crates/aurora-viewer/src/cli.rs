//! Command-line arguments. Most test switches are environment variables
//! (`AURORA_DEMO`, `AURORA_CAPTURE`...: see the README); the command line
//! carries what identifies one running instance.

use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct Cli {
    /// `--title <text>`: shown in the window title ("Aurora Viewer - <text>")
    /// and used to name the log, so parallel test windows can be told apart.
    pub title: Option<String>,
}

const HELP: &str = "Aurora Viewer

Usage: aurora-viewer [--title <text>]

  --title <text>   Name this window (\"Aurora Viewer - <text>\") and its log
                   file (aurora-<text>.log, aurora-demo-<text>.log in demo mode)
  -h, --help       Show this help

Test switches are environment variables (AURORA_DEMO=1, AURORA_CAPTURE=...):
see README.md.";

static CLI: OnceLock<Cli> = OnceLock::new();

/// The parsed arguments (parsed on first use).
pub fn get() -> &'static Cli {
    CLI.get_or_init(|| parse(std::env::args().skip(1)))
}

fn parse(args: impl Iterator<Item = String>) -> Cli {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        if a == "-h" || a == "--help" {
            println!("{HELP}");
            std::process::exit(0);
        } else if let Some(v) = a.strip_prefix("--title=") {
            cli.title = Some(v.to_owned());
        } else if a == "--title" {
            cli.title = args.next();
        }
    }
    cli.title = cli.title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    cli
}

/// The window title, ending with the build profile ("(Dev)", "(Release)",
/// "(Debug)"): test windows of different builds can be told apart.
pub fn window_title() -> String {
    title_with(get().title.as_deref(), env!("AURORA_BUILD_PROFILE"))
}

fn title_with(title: Option<&str>, profile_dir: &str) -> String {
    let base = match title {
        Some(t) => format!("Aurora Viewer - {t}"),
        None => "Aurora Viewer".to_owned(),
    };
    match profile_label(profile_dir) {
        Some(p) => format!("{base} ({p})"),
        None => base,
    }
}

/// Label of a cargo profile from its target directory (`dev` builds into
/// `debug`; `debugging` is aurora-tools' debug viewer).
fn profile_label(dir: &str) -> Option<&'static str> {
    match dir {
        "release" => Some("Release"),
        "debug" => Some("Dev"),
        "debugging" => Some("Debug"),
        "ci" => Some("CI"),
        _ => None,
    }
}

/// The title as a file-name part: letters, digits and dashes.
pub fn title_slug() -> Option<String> {
    let t = get().title.as_ref()?;
    let slug: String = t
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_ends_with_the_build_profile() {
        assert_eq!(title_with(Some("Test ombres"), "debug"), "Aurora Viewer - Test ombres (Dev)");
        assert_eq!(title_with(None, "release"), "Aurora Viewer (Release)");
        assert_eq!(title_with(None, "debugging"), "Aurora Viewer (Debug)");
        assert_eq!(title_with(Some("X"), "unknown"), "Aurora Viewer - X");
    }

    #[test]
    fn title_forms() {
        let cli = parse(["--title", "Test LookAt"].map(String::from).into_iter());
        assert_eq!(cli.title.as_deref(), Some("Test LookAt"));
        let cli = parse(["--title=  Sons  "].map(String::from).into_iter());
        assert_eq!(cli.title.as_deref(), Some("Sons"));
        let cli = parse(["--title", "   "].map(String::from).into_iter());
        assert_eq!(cli.title, None);
    }
}
