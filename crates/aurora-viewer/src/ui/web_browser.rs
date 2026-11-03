//! The built-in web browser, like LLFloaterWebContent (indra/newview/
//! llfloaterwebcontent.cpp and skins/default/xui/en/floater_web_content.xml,
//! originally LGPL 2.1) opened by the toolbar's `webbrowser` command
//! (commands.xml: ToggleWebBrowser with HOME_PAGE, llviewermenu.cpp
//! toggleWebBrowser): back, forward, stop, reload, an address field, the
//! padlock of secure pages, "open in the system browser", and a status line
//! with the load progress. The button opens the home page (the
//! `FSBrowserHomePage` setting) and closes the window when it is shown.
//!
//! Links clicked in a page keep Aurora's routing (SLURLs in the viewer,
//! other addresses through the link checks to the system browser): the
//! Legacy Viewer's PreferredBrowserBehavior is not ported.

use super::web_view::WebView;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use egui::{Rect, RichText, Vec2};

pub const WINDOW: &str = "web_browser";

/// The `FSBrowserHomePage` default (app_settings/settings.xml).
pub const DEFAULT_HOME: &str = "https://duckduckgo.com";

/// floater_web_content.xml is 780 x 775 with a minimum of 500 x 400.
const DEFAULT_SIZE: Vec2 = Vec2::new(780.0, 700.0);
const MIN_SIZE: Vec2 = Vec2::new(500.0, 400.0);

/// What the address field sends to the browser. LLFloaterWebContent::
/// onEnterAddress navigates to the trimmed text as it is; the browser engine
/// wants a scheme, so a bare host gets `https://`. Only web pages (and local
/// `file:` pages and `about:blank`) are accepted: a `javascript:` or `data:`
/// address typed or pasted here goes nowhere. None for an empty field.
pub fn normalize_address(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if lower == "about:blank" || lower.starts_with("file://") {
        return Some(text.to_owned());
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(text.to_owned());
    }
    // `name:` followed by something that is not a port is another scheme
    // (javascript:, data:, mailto:…); `localhost:8080` and `host:443/path` are hosts
    if let Some((scheme, rest)) = text.split_once(':')
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !scheme.is_empty()
        && !rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    Some(format!("https://{text}"))
}

/// The page the button opens: the setting, or the Legacy default when it is
/// empty or not an address.
pub fn home_page(setting: &str) -> String {
    normalize_address(setting).unwrap_or_else(|| DEFAULT_HOME.to_owned())
}

/// The padlock of LLFloaterWebContent (media_secure_lock_flag, shown for
/// https pages).
pub fn is_secure(url: &str) -> bool {
    url.trim_start().to_ascii_lowercase().starts_with("https://")
}

/// What the window needs to run its browser.
pub struct BrowserWeb<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    /// The `FSBrowserHomePage` setting.
    pub home: &'a str,
}

#[derive(Default)]
pub struct WebBrowserUi {
    /// The browser keeps the shared profile (cookies, sign-ins), like the
    /// Marketplace's and the Legacy Viewer's.
    web: WebView,
    /// The home page was sent to the running browser.
    started: bool,
    /// The address being typed, until Enter or leaving the field.
    edit: Option<String>,
    /// Ask the field for the keyboard on the next frame (the Legacy address
    /// bar has it when the floater opens).
    raise: bool,
}

impl WebBrowserUi {
    pub fn web_has_focus(&self, open: bool) -> bool {
        open && self.web.focused && self.web.running()
    }

    pub fn focused_web(&mut self, open: bool) -> Option<&mut aurora_media::MediaPlugin> {
        self.web_has_focus(open).then(|| self.web.plugin_mut()).flatten()
    }

    /// The system id of the page's browser process, while one runs.
    pub fn web_process_id(&self) -> Option<u32> {
        self.web.process_id()
    }

    /// Show `address` at the next frame (demo, or a link sent here).
    pub fn go_to(&mut self, address: &str) {
        if let Some(url) = normalize_address(address) {
            self.web.navigate_again(&url);
        }
        self.started = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, env: &BrowserWeb) {
        if !*open {
            // the browser does not run behind a closed window, and the next
            // opening starts on the home page again
            if self.web.running() || self.started {
                self.web = WebView::default();
                self.started = false;
                self.edit = None;
            }
            return;
        }
        if !self.started {
            self.started = true;
            self.raise = true;
            self.web.navigate_again(&home_page(env.home));
        }
        let work = super::layout::work_area(ctx);
        let size = DEFAULT_SIZE.min(work.size() - Vec2::splat(2.0 * super::layout::GAP)).max(MIN_SIZE);
        let default = Rect::from_min_size((work.center() - size * 0.5).round(), size);
        let title = match self.web.title.trim() {
            "" => aurora_i18n::tr!("web-browser-title").to_string(),
            page => page.to_owned(),
        };
        let mut floater = Floater::new(WINDOW, title, default.min, default.size());
        floater.min_size = MIN_SIZE;
        floater.show(ctx, p, open, |ui| self.contents(ui, p, env));
        if (!*open || widgets::floater_minimized(ctx, WINDOW)) && self.web.focused {
            self.web.focused = false;
            if let Some(plugin) = self.web.plugin_mut() {
                plugin.focus(false);
            }
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, p: &Palette, env: &BrowserWeb) {
        let home = home_page(env.home);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let running = self.web.running();
            if widgets::icon_button(ui, p, "arrow-left", aurora_i18n::tr!("search-web-back"), running) {
                self.web.back();
            }
            if widgets::icon_button(ui, p, "arrow-right", aurora_i18n::tr!("search-web-forward"), running) {
                self.web.forward();
            }
            if widgets::icon_button(
                ui,
                p,
                "player-stop-filled",
                aurora_i18n::tr!("web-browser-stop"),
                running && self.web.loading,
            ) {
                self.web.stop();
                self.web.loading = false;
            }
            if widgets::icon_button(ui, p, "reload", aurora_i18n::tr!("search-web-reload"), running) {
                self.web.reload();
            }
            if widgets::icon_button(ui, p, "home-filled", aurora_i18n::tr!("web-browser-home"), true) {
                self.edit = None;
                self.web.navigate_again(&home);
            }
            let external = Vec2::new(0.0, 22.0);
            let label = aurora_i18n::tr!("search-web-open-external");
            let reserve = ui
                .painter()
                .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), p.ink)
                .size()
                .x
                + 62.0;
            let shown = if self.web.location.is_empty() {
                home.clone()
            } else {
                self.web.location.clone()
            };
            // the lock of an https page (media_secure_lock_flag)
            let secure = is_secure(&shown);
            let (lock, lock_resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
            let icon = if secure { "lock-filled" } else { "lock-open" };
            let tip = if secure {
                aurora_i18n::tr!("web-browser-secure")
            } else {
                aurora_i18n::tr!("web-browser-not-secure")
            };
            if let Some(t) = super::icons::global(icon) {
                ui.painter().image(
                    t.id(),
                    lock,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    if secure { p.success } else { p.muted },
                );
            }
            lock_resp.on_hover_text(tip);
            // the address follows the page until it is edited
            let mut address = self.edit.clone().unwrap_or_else(|| shown.clone());
            let field = ui.add(
                egui::TextEdit::singleline(&mut address)
                    .desired_width((ui.available_width() - reserve).max(120.0))
                    .font(egui::FontId::proportional(12.0)),
            );
            if std::mem::take(&mut self.raise) {
                field.request_focus();
            }
            if field.changed() {
                self.edit = Some(address.clone());
            }
            if field.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(url) = normalize_address(&address)
                {
                    self.web.navigate_again(&url);
                }
                self.edit = None;
            }
            if widgets::action_button(ui, p, "external-link-filled", label, ActionKind::Flat, external).clicked() {
                // the page itself, in the system browser (loadURLExternal)
                if let Some(url) = normalize_address(&shown) {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        });
        ui.add_space(4.0);
        // the status line is drawn under the page: its height comes off the page
        let status_h = 18.0;
        let size = Vec2::new(ui.available_width(), (ui.available_height() - status_h - 4.0).max(120.0));
        self.web.show(ui, p, env.paths, env.browser, None, size);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let loading = self.web.loading;
            let text = if self.web.status.is_empty() {
                if loading {
                    aurora_i18n::tr!("web-browser-loading").to_string()
                } else {
                    String::new()
                }
            } else {
                self.web.status.clone()
            };
            let bar = 120.0;
            if loading {
                widgets::progress(ui, p, "web_browser_load", self.web.progress as f32 / 100.0, bar, "", None);
            }
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).truncate());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_page_default_is_the_legacy_one() {
        assert_eq!(DEFAULT_HOME, "https://duckduckgo.com");
        assert_eq!(crate::settings::Settings::default().browser_home_page, DEFAULT_HOME);
        assert_eq!(home_page(""), DEFAULT_HOME);
        assert_eq!(home_page("   "), DEFAULT_HOME);
        assert_eq!(home_page("javascript:alert(1)"), DEFAULT_HOME);
        assert_eq!(home_page("example.org/start"), "https://example.org/start");
    }

    #[test]
    fn the_address_is_navigated_to_as_typed_with_a_scheme_added() {
        assert_eq!(
            normalize_address("  https://example.org/a?b=1  ").as_deref(),
            Some("https://example.org/a?b=1")
        );
        assert_eq!(normalize_address("HTTP://Example.org").as_deref(), Some("HTTP://Example.org"));
        assert_eq!(normalize_address("example.org").as_deref(), Some("https://example.org"));
        assert_eq!(normalize_address("localhost:8080/x").as_deref(), Some("https://localhost:8080/x"));
        assert_eq!(normalize_address("file:///C:/page.html").as_deref(), Some("file:///C:/page.html"));
        assert_eq!(normalize_address("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(normalize_address(""), None);
        assert_eq!(normalize_address("   "), None);
    }

    #[test]
    fn scripts_and_odd_schemes_are_not_navigated_to() {
        for bad in [
            "javascript:alert(1)",
            "data:text/html,hi",
            "mailto:a@b.c",
            "secondlife:///app/x",
            "two words",
        ] {
            assert_eq!(normalize_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_https_pages_get_the_padlock() {
        assert!(is_secure("https://example.org"));
        assert!(is_secure(" HTTPS://example.org"));
        assert!(!is_secure("http://example.org"));
        assert!(!is_secure("file:///x"));
        assert!(!is_secure(""));
    }
}
