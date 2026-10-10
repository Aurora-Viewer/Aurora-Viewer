//! Command-line arguments. Most test switches are environment variables
//! (`AURORA_DEMO`, `AURORA_CAPTURE`...: see the README); the command line
//! carries what identifies one running instance.

use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct Cli {
    /// `--title <text>`: shown after the viewer's name and version
    /// and used to name the log, so parallel test windows can be told apart.
    pub title: Option<String>,
    /// Process-only language override for demos and captures.
    pub language: Option<String>,
    /// Additional cap for this process only, including captures and online tests.
    pub fps_limit: Option<u32>,
    help: bool,
    version: bool,
}

static CLI: OnceLock<Cli> = OnceLock::new();

/// The parsed arguments (parsed on first use).
pub fn get() -> &'static Cli {
    CLI.get_or_init(|| {
        let cli = parse(std::env::args().skip(1));
        aurora_i18n::select("default", cli.language.as_deref());
        if cli.help {
            let languages = aurora_i18n::available_languages()
                .map(|(code, _)| code)
                .collect::<Vec<_>>()
                .join(", ");
            println!("{}", aurora_i18n::tr!("cli-help", languages = languages));
            std::process::exit(0);
        }
        if cli.version {
            println!("{}", aurora_i18n::tr!("common-aurora-viewer", value0 = aurora_version::version()));
            std::process::exit(0);
        }
        cli
    })
}

fn parse(args: impl Iterator<Item = String>) -> Cli {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        if a == "-h" || a == "--help" {
            cli.help = true;
        } else if a == "-V" || a == "--version" {
            cli.version = true;
        } else if let Some(value) = a.strip_prefix("--language=") {
            cli.language = Some(value.to_owned());
        } else if a == "--language" {
            if args.peek().is_some_and(|value| !value.starts_with('-')) {
                cli.language = args.next();
            }
        } else if let Some(v) = a.strip_prefix("--title=") {
            cli.title = Some(v.to_owned());
        } else if a == "--title" {
            cli.title = args.next();
        } else if let Some(v) = a.strip_prefix("--fps-limit=") {
            cli.fps_limit = parse_fps_limit(v);
        } else if a == "--fps-limit" {
            cli.fps_limit = args.next().as_deref().and_then(parse_fps_limit);
        }
    }
    cli.title = cli.title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    cli.language = cli.language.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    cli
}

fn parse_fps_limit(value: &str) -> Option<u32> {
    let limit = value.trim().parse::<u32>().ok().filter(|fps| (1..=500).contains(fps));
    if limit.is_none() {
        eprintln!(
            "{}",
            aurora_i18n::tr!("cli-fps-limit-requires-a-value-between-1-and-500").to_owned()
        );
    }
    limit
}

/// The window title, ending with the build profile ("(Dev)", "(Release)",
/// "(Debug)"): test windows of different builds can be told apart.
pub fn window_title() -> String {
    title_with(get().title.as_deref(), env!("AURORA_BUILD_PROFILE"), aurora_version::version())
}

fn title_with(title: Option<&str>, profile_dir: &str, version: &str) -> String {
    let base = match title {
        Some(t) => aurora_i18n::tr!("cli-aurora-viewer-2", version = version.to_string(), t = t.to_string()),
        None => aurora_i18n::tr!("cli-aurora-viewer", version = version.to_string()),
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
        "debugging" => Some(aurora_i18n::tr!("common-debug")),
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
    fn language_forms_and_help_preserve_the_process_override() {
        for arguments in [
            vec!["--language", "en"],
            vec!["--language=en"],
            vec!["--help", "--language", " en "],
        ] {
            let cli = parse(arguments.into_iter().map(String::from));
            assert_eq!(cli.language.as_deref(), Some("en"));
        }
        let cli = parse(["--language", "--title", "Translation test"].into_iter().map(String::from));
        assert_eq!(cli.language, None);
        assert_eq!(cli.title.as_deref(), Some("Translation test"));
        assert_eq!(parse(["--language="].into_iter().map(String::from)).language, None);
    }

    #[test]
    fn title_ends_with_the_build_profile() {
        assert_eq!(
            title_with(Some("Test ombres"), "debug", "0.2.0"),
            aurora_i18n::tr!("cli-aurora-viewer", version = "0.2.0 - Test ombres (Dev)").as_str()
        );
        assert_eq!(
            title_with(None, "release", "0.2.0"),
            aurora_i18n::tr!("cli-aurora-viewer", version = "0.2.0 (Release)").as_str()
        );
        assert_eq!(
            title_with(None, "debugging", "0.2.0"),
            aurora_i18n::tr!("cli-aurora-viewer", version = "0.2.0 (Debug)").as_str()
        );
        assert_eq!(
            title_with(Some("X"), "unknown", "0.2.0"),
            aurora_i18n::tr!("cli-aurora-viewer", version = "0.2.0 - X").as_str()
        );
        assert_eq!(
            title_with(None, "debug", "0.2.0+7.gabcdef123"),
            aurora_i18n::tr!("cli-aurora-viewer", version = "0.2.0+7.gabcdef123 (Dev)").as_str()
        );
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

    #[test]
    fn process_fps_limit_forms_and_invalid_values() {
        assert_eq!(parse_fps_limit("500"), Some(500));
        assert_eq!(parse_fps_limit("30"), Some(30));
        for args in [vec!["--fps-limit", "60"], vec!["--fps-limit=60"]] {
            assert_eq!(parse(args.into_iter().map(String::from)).fps_limit, Some(60));
        }
        for value in ["0", "501", "-1", "nan"] {
            assert_eq!(parse_fps_limit(value), None);
        }
    }
}
