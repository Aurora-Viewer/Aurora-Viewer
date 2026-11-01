//! Command-line arguments. Most test switches are environment variables
//! (`AURORA_DEMO`, `AURORA_CAPTURE`...: see the README); the command line
//! carries what identifies one running instance, and where and as whom it
//! starts (the Legacy Viewer's `--grid`, `--loginuri`, `--login` and the
//! startup SLURL of app_settings/cmd_line.xml).

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Default)]
pub struct Cli {
    /// `--title <text>`: shown after the viewer's name and version
    /// and used to name the log, so parallel test windows can be told apart.
    pub title: Option<String>,
    /// Process-only language override for demos and captures.
    pub language: Option<String>,
    /// Additional cap for this process only, including captures and online tests.
    pub fps_limit: Option<u32>,
    /// `--vr`: enter VR once the world shows (the restart that the VR
    /// button offers when the headset came after startup passes it).
    pub vr: bool,
    /// `--grid <name>` or `--loginuri <uri>` (CmdLineGridChoice,
    /// CmdLineLoginURI): the grid the login screen starts on.
    pub grid: Option<Grid>,
    /// The startup SLURL (CmdLineLoginLocation): `--url <slurl>`, `-url`
    /// (the form the system's `secondlife:` handler passes) or a bare
    /// argument. Like the Legacy Viewer's `last_option`, nothing after it is
    /// read: a link opened from a web page cannot slip other options in.
    pub url: Option<String>,
    /// `--login <first> <last> <password>` (UserLoginInfoCmdLine), taken
    /// once by the login screen (`take_login`) and not kept after.
    login: Mutex<Option<Login>>,
    help: bool,
    version: bool,
}

/// The grid named by `--grid` or `--loginuri`.
#[derive(Debug, Clone, PartialEq)]
pub enum Grid {
    SecondLife,
    SecondLifeBeta,
    /// Another grid's login URI.
    LoginUri(String),
    /// A name or nickname from the saved grid catalogue.
    Named(String),
}

/// `--login`: the account to log in with as soon as the login screen opens
/// (LLLoginHandler::loadSavedUserLoginInfo, AutoLogin).
#[derive(Clone, PartialEq)]
pub struct Login {
    pub first: String,
    pub last: String,
    pub password: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("first", &self.first)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

/// The `--login` account, once: the password is not kept after.
pub fn take_login() -> Option<Login> {
    get().login.lock().ok()?.take()
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
        } else if a == "--vr" {
            cli.vr = true;
        } else if let Some(v) = a.strip_prefix("--fps-limit=") {
            cli.fps_limit = parse_fps_limit(v);
        } else if a == "--fps-limit" {
            cli.fps_limit = args.next().as_deref().and_then(parse_fps_limit);
        } else if let Some(v) = a.strip_prefix("--grid=") {
            cli.grid = parse_grid(v);
        } else if a == "--grid" {
            cli.grid = args.next().as_deref().and_then(parse_grid);
        } else if let Some(v) = a.strip_prefix("--loginuri=") {
            cli.grid = parse_login_uri(v);
        } else if a == "--loginuri" {
            cli.grid = args.next().as_deref().and_then(parse_login_uri);
        } else if a == "--login" {
            let mut tokens = (&mut args).take(3).collect::<Vec<_>>();
            if tokens.len() == 3 && tokens.iter().all(|t| !t.trim().is_empty()) {
                let password = tokens.pop().unwrap_or_default();
                let last = tokens.pop().unwrap_or_default();
                let first = tokens.pop().unwrap_or_default();
                cli.login = Mutex::new(Some(Login {
                    first: first.trim().to_owned(),
                    last: last.trim().to_owned(),
                    password,
                }));
            } else {
                eprintln!("{}", aurora_i18n::tr!("cli-login-requires-three-values"));
            }
        } else if let Some(v) = a.strip_prefix("--url=") {
            cli.url = Some(v.to_owned());
            break;
        } else if a == "--url" || a == "-url" || a == "--slurl" {
            cli.url = args.next();
            break;
        } else if !a.starts_with('-') {
            // the positional SLURL of cmd_line.xml (`slurl`)
            cli.url = Some(a);
            break;
        }
    }
    cli.title = cli.title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    cli.language = cli.language.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    cli.url = cli.url.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    cli
}

/// `--grid`: the names the Legacy Viewer's grid manager knows for the two
/// Second Life grids (agni / aditi, their login hosts), or a login URI.
fn parse_grid(value: &str) -> Option<Grid> {
    let name = value.trim().to_ascii_lowercase();
    match name.as_str() {
        "agni" | "secondlife" | "second life" | "sl" | "main" | "util.agni.lindenlab.com" => Some(Grid::SecondLife),
        "aditi" | "beta" | "secondlife_beta" | "util.aditi.lindenlab.com" => Some(Grid::SecondLifeBeta),
        _ if name.starts_with("http://") || name.starts_with("https://") => parse_login_uri(value),
        _ if !name.is_empty() => Some(Grid::Named(value.trim().to_owned())),
        _ => None,
    }
}

/// `--loginuri`: the two Second Life grids keep their own choice, any
/// other URI is a custom grid.
fn parse_login_uri(value: &str) -> Option<Grid> {
    let uri = value.trim();
    if uri.is_empty() {
        return None;
    }
    Some(if uri.eq_ignore_ascii_case(aurora_net::login::SL_MAIN_GRID) {
        Grid::SecondLife
    } else if uri.eq_ignore_ascii_case(aurora_net::login::SL_BETA_GRID) {
        Grid::SecondLifeBeta
    } else {
        Grid::LoginUri(uri.to_owned())
    })
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

/// The viewer's identity on the Linux desktop: Wayland app_id, X11
/// WM_CLASS, and the name of `packaging/linux/aurora-viewer.desktop`, whose
/// `StartupWMClass` repeats it. The panel and the dock match the window to
/// that file (name, icon, pinning) only when the two agree. The executable's
/// name, which X11 used by default: X11 desktops see no change. The Legacy
/// Viewer's `com.secondlife.indra.viewer` is shared by every viewer of its
/// family, so Aurora does not take it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const DESKTOP_ID: &str = "aurora-viewer";

/// Window attributes with the [`DESKTOP_ID`] on Linux; elsewhere unchanged.
pub fn with_desktop_id(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    #[cfg(target_os = "linux")]
    {
        // one field behind both traits: the Wayland app_id is `general`,
        // the X11 WM_CLASS is (`instance`, `general`)
        use winit::platform::wayland::WindowAttributesExtWayland;
        attrs.with_name(DESKTOP_ID, DESKTOP_ID)
    }
    #[cfg(not(target_os = "linux"))]
    attrs
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
pub fn profile_label(dir: &str) -> Option<&'static str> {
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

    fn args(list: &[&str]) -> Cli {
        parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn startup_slurl_ends_the_arguments() {
        let url = "secondlife://Aurora%20D%C3%A9mo/12/34/56";
        for list in [vec!["-url", url], vec!["--url", url], vec![url]] {
            assert_eq!(args(&list).url.as_deref(), Some(url));
        }
        assert_eq!(args(&[&format!("--url={url}")]).url.as_deref(), Some(url));
        // what the system's handler passes, with options slipped into a link
        let cli = args(&["--title", "T", "-url", url, "--login", "a", "b", "c", "--grid", "aditi"]);
        assert_eq!(cli.url.as_deref(), Some(url));
        assert_eq!(cli.title.as_deref(), Some("T"));
        assert!(cli.login.lock().expect("login").is_none());
        assert_eq!(cli.grid, None);
        assert_eq!(args(&["-url", "  "]).url, None);
    }

    #[test]
    fn grid_names_and_login_uris() {
        assert_eq!(args(&["--grid", "agni"]).grid, Some(Grid::SecondLife));
        assert_eq!(args(&["--grid=Aditi"]).grid, Some(Grid::SecondLifeBeta));
        assert_eq!(args(&["--grid", "My Grid"]).grid, Some(Grid::Named("My Grid".into())));
        assert_eq!(args(&["--grid", "util.aditi.lindenlab.com"]).grid, Some(Grid::SecondLifeBeta));
        assert_eq!(
            args(&["--grid", "http://grid.example:8002/"]).grid,
            Some(Grid::LoginUri("http://grid.example:8002/".into()))
        );
        assert_eq!(args(&["--grid", "nowhere"]).grid, Some(Grid::Named("nowhere".into())));
        assert_eq!(
            args(&["--loginuri", aurora_net::login::SL_BETA_GRID]).grid,
            Some(Grid::SecondLifeBeta)
        );
        assert_eq!(
            args(&["--loginuri=https://login.example/cgi"]).grid,
            Some(Grid::LoginUri("https://login.example/cgi".into()))
        );
    }

    #[test]
    fn login_takes_three_values_and_hides_the_password() {
        let cli = args(&["--login", "Synthetic", "Resident", "not-a-password", "--title", "T"]);
        let login = cli.login.lock().expect("login").take().expect("three values");
        assert_eq!((login.first.as_str(), login.last.as_str()), ("Synthetic", "Resident"));
        assert_eq!(login.password, "not-a-password");
        assert!(!format!("{login:?}").contains("not-a-password"));
        assert_eq!(cli.title.as_deref(), Some("T"));
        assert!(args(&["--login", "Synthetic", "Resident"]).login.lock().expect("login").is_none());
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
