//! "Create an account" on the login screen, like the LL viewer 26.5's Join
//! floater (LLFloaterJoin and LLFloaterJoinHandler, indra/newview/
//! llfloaterjoin.cpp, LLFloaterWebModal, llfloaterwebmodal.cpp,
//! skins/default/xui/en/floater_join.xml and floater_web_modal.xml,
//! originally LGPL 2.1; LLPanelLogin::onClickSignUp, llpanellogin.cpp).
//!
//! On Second Life the button opens Linden Lab's sign-up page in a window of
//! the viewer, over the login screen, instead of the system browser. The
//! page talks to the window through `secondlife:///app/floaterjoin/…`
//! (`slapp::join_command`): `close` closes it, `tos`, `tc` and `privacy`
//! open the Terms of Service, the Terms and Conditions or the Privacy
//! Policy in a window of their own over it, and `join/<name>/<password>`,
//! sent once the account is created, puts the new account's name and
//! password in the login form (LLPanelLogin::setCredentialFields, which also
//! turns "Remember me" and "Remember password" on) and closes the window.
//! The viewer keeps nothing else of the account: it is the form's, as if
//! typed. On another grid the button opens the grid's own registration page
//! (`register` of its grid info) in the same window, as the Legacy Viewer
//! opens it in its built-in browser (FSPanelLogin::onClickNewAccount,
//! loadURLInternal), without the sign-up commands.
//!
//! The commands are UNTRUSTED_THROTTLE, as in the Legacy Viewer: those the
//! page's script sends without a click wait 5 seconds between them. Stricter
//! than the Legacy Viewer, which runs them from any browser of the viewer:
//! only this window's page is heard, and `join` only from a Second Life
//! page (https, secondlife.com), so that no other site the window is led to
//! fills the login form. Deliberate differences: the window has a title bar
//! with a close button (the Legacy floater has neither, so a page that does
//! not load leaves no way out), uses the login screen's glass rather than
//! its 80 % dark overlay, and shrinks its page to a small window instead of
//! scrolling it; the Terms windows are not resizable.

use super::web_view::{JoinRequest, WebView};
use super::widgets;
use crate::slapp::JoinCommand;
use crate::theme::Palette;
use egui::{Id, RichText, Vec2};
use std::time::{Duration, Instant};

/// The `JoinURL` default (app_settings/settings.xml).
pub const JOIN_PAGE: &str = "https://join.secondlife.com/";
/// `TermsOfServiceURL`, `TermsAndConditionsURL` and `PrivacyPolicyURL`.
pub const TERMS_OF_SERVICE: &str = "https://lindenlab.com/tos";
pub const TERMS_AND_CONDITIONS: &str = "https://lindenlab.com/legal/second-life-terms-and-conditions";
pub const PRIVACY_POLICY: &str = "https://lindenlab.com/privacy";

/// floater_join.xml is 1024 x 768, floater_web_modal.xml 800 x 600.
const JOIN_VIEW: Vec2 = Vec2::new(1024.0, 768.0);
const LEGAL_VIEW: Vec2 = Vec2::new(800.0, 600.0);
/// The smallest page either window shows.
const MIN_VIEW: Vec2 = Vec2::new(320.0, 240.0);
/// What a modal adds around its page: its margins and the title row.
const CHROME: Vec2 = Vec2::new(36.0, 36.0 + TITLE_ROW + 8.0);
const TITLE_ROW: f32 = 26.0;
/// Room left between a window and the edges of the screen.
const SCREEN_MARGIN: f32 = 16.0;
/// THROTTLE_PERIOD of llcommandhandler.cpp: seconds between two commands
/// that were not clicked.
const THROTTLE: Duration = Duration::from_secs(5);

/// Where "Create an account" leads for the login form's grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignupPage {
    /// Linden Lab's sign-up page and its commands.
    SecondLife,
    /// Another grid's registration page.
    Grid(String),
}

/// The page of the login form's grid: Second Life's on Linden Lab's grids,
/// else the saved grid's registration page when it gives one (a web page);
/// None when there is nowhere to create an account (the button is hidden).
pub fn signup_page(settings: &crate::settings::Settings) -> Option<SignupPage> {
    if settings.grid != crate::settings::GridChoice::Custom || aurora_net::login::is_linden_login(&settings.login_uri()) {
        return Some(SignupPage::SecondLife);
    }
    let register = crate::grids::active(settings)?.register.trim();
    is_web_page(register).then(|| SignupPage::Grid(register.to_owned()))
}

fn is_web_page(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some_and(|h| !h.is_empty())
            && u.username().is_empty()
            && u.password().is_none()
    })
}

/// The origin of a Second Life page: https on secondlife.com or one of its
/// sub-domains (`web_view::origin`'s form).
fn second_life_origin(document: &str) -> bool {
    let Some(host) = document.strip_prefix("https://") else {
        return false;
    };
    let host = host.split(':').next().unwrap_or("");
    host == "secondlife.com" || host.ends_with(".secondlife.com")
}

/// Whether the window acts on `request`; `last` is when the last command
/// sent without a click ran (LLCommandHandlerRegistry::dispatch,
/// UNTRUSTED_THROTTLE: a click is never throttled). `join` is heard from a
/// Second Life page only, before the throttle, so that a refused one does
/// not hold back the next.
fn accepted(request: &JoinRequest, last: &mut Option<Instant>, now: Instant) -> bool {
    if matches!(request.command, JoinCommand::Join { .. }) && !request.document.as_deref().is_some_and(second_life_origin) {
        log::warn!("sign-up: account details refused from a page that is not Second Life's");
        return false;
    }
    if !request.clicked {
        if last.is_some_and(|at| now.saturating_duration_since(at) < THROTTLE) {
            log::warn!("sign-up: command throttled ({:?})", request.command);
            return false;
        }
        *last = Some(now);
    }
    true
}

/// A page of `native` size inside a modal on `screen`: as large as it is,
/// less what does not fit.
fn fit(native: Vec2, screen: Vec2) -> Vec2 {
    native.min(screen - CHROME - Vec2::splat(2.0 * SCREEN_MARGIN)).max(MIN_VIEW).round()
}

/// Where the window's browsers look and with what (no OpenID cookie: no
/// one is logged in).
pub struct SignupWeb<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
}

/// The new account's name and password, for the login form.
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Default)]
pub struct SignupUi {
    /// The page shown, while the window is open.
    page: Option<SignupPage>,
    web: WebView,
    /// The Terms, Conditions or Privacy page over it.
    legal: Option<&'static str>,
    legal_web: WebView,
    /// When the last command sent without a click ran.
    last_unclicked: Option<Instant>,
    /// Browsers closed this frame: their page was drawn, so its texture
    /// lives until the next frame (`drop_retired`).
    retired: Vec<WebView>,
}

impl SignupUi {
    /// Open the window on `page` (LLFloaterJoin::onOpen navigates again).
    pub fn open(&mut self, page: SignupPage) {
        log::info!(
            "sign-up: opening {}",
            match &page {
                SignupPage::SecondLife => "the Second Life sign-up page",
                SignupPage::Grid(_) => "the grid's registration page",
            }
        );
        self.close();
        self.web.navigate(match &page {
            SignupPage::SecondLife => JOIN_PAGE,
            SignupPage::Grid(url) => url,
        });
        if page == SignupPage::SecondLife {
            self.web.accept_join_commands();
        }
        self.page = Some(page);
    }

    /// Open the window on another address (`AURORA_DEMO_SIGNUP`): the
    /// sign-up commands are heard, `join` only from a Second Life page.
    pub fn open_demo(&mut self, url: &str) {
        self.close();
        self.web.navigate(url);
        self.web.accept_join_commands();
        self.page = Some(SignupPage::SecondLife);
    }

    /// Close the window; its browsers stop at the next frame.
    pub fn close(&mut self) {
        self.page = None;
        self.legal = None;
        self.last_unclicked = None;
        let (web, legal) = (std::mem::take(&mut self.web), std::mem::take(&mut self.legal_web));
        self.retired.extend([web, legal]);
    }

    /// Stop the browsers closed by the frame before.
    pub fn drop_retired(&mut self) {
        self.retired.clear();
    }

    pub fn is_open(&self) -> bool {
        self.page.is_some()
    }

    /// The clicked page owns native keys, like LLMediaCtrl.
    pub fn web_has_focus(&self) -> bool {
        self.is_open() && [&self.web, &self.legal_web].iter().any(|w| w.focused && w.running())
    }

    pub fn focused_web(&mut self) -> Option<&mut aurora_media::MediaPlugin> {
        if !self.is_open() {
            return None;
        }
        [&mut self.legal_web, &mut self.web]
            .into_iter()
            .find(|w| w.focused && w.running())
            .and_then(WebView::plugin_mut)
    }

    /// The system ids of the window's browser processes.
    pub fn web_process_ids(&self) -> [Option<u32>; 2] {
        [self.web.process_id(), self.legal_web.process_id()]
    }

    /// Draw the window (and the Terms window over it); returns the new
    /// account's name and password once the page sends them.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, env: &SignupWeb) -> Option<Credentials> {
        self.drop_retired();
        let page = self.page.clone()?;
        let screen = ctx.content_rect().size();
        let view = fit(JOIN_VIEW, screen);
        let title = match page {
            SignupPage::SecondLife => aurora_i18n::tr!("signup-title"),
            SignupPage::Grid(_) => aurora_i18n::tr!("ui-login-create-account"),
        };
        let covered = self.legal.is_some();
        let mut close = false;
        widgets::Modal::new(Id::unique("signup_window")).login_surface(p).show(ctx, |ui| {
            close = title_row(ui, p, title, view.x);
            // under the Terms window, the page waits: no click reaches it
            ui.add_enabled_ui(!covered, |ui| {
                self.web.show(ui, p, env.paths, env.browser, None, view);
            });
        });
        let mut credentials = None;
        let now = Instant::now();
        for request in std::mem::take(&mut self.web.join_requests) {
            if !accepted(&request, &mut self.last_unclicked, now) {
                continue;
            }
            log::info!("sign-up: the page asks {:?}", request.command);
            match request.command {
                JoinCommand::Close => close = true,
                JoinCommand::TermsOfService => self.open_legal(TERMS_OF_SERVICE),
                JoinCommand::TermsAndConditions => self.open_legal(TERMS_AND_CONDITIONS),
                JoinCommand::Privacy => self.open_legal(PRIVACY_POLICY),
                JoinCommand::Join { username, password } => {
                    credentials = Some(Credentials { username, password });
                    close = true;
                }
            }
        }
        if let Some(url) = self.legal.filter(|_| !close) {
            let view = fit(LEGAL_VIEW, screen);
            // LLFloaterWebModal::handleMediaEvent: the page's title, else its address
            let title = if self.legal_web.title.trim().is_empty() {
                if self.legal_web.location.is_empty() {
                    url
                } else {
                    self.legal_web.location.as_str()
                }
            } else {
                self.legal_web.title.as_str()
            }
            .to_owned();
            let mut close_legal = false;
            widgets::Modal::new(Id::unique("signup_legal_window"))
                .login_surface(p)
                .show(ctx, |ui| {
                    close_legal = title_row(ui, p, &title, view.x);
                    self.legal_web.show(ui, p, env.paths, env.browser, None, view);
                });
            if close_legal {
                self.legal = None;
                let legal = std::mem::take(&mut self.legal_web);
                self.retired.push(legal);
            }
        }
        if close {
            log::info!("sign-up: window closed");
            self.close();
        }
        credentials
    }

    /// A Terms window over the page, which lets go of the keyboard.
    fn open_legal(&mut self, url: &'static str) {
        if self.web.focused {
            self.web.focused = false;
            if let Some(plugin) = self.web.plugin_mut() {
                plugin.focus(false);
            }
        }
        let previous = std::mem::take(&mut self.legal_web);
        self.retired.push(previous);
        self.legal_web.navigate(url);
        self.legal = Some(url);
    }
}

/// The window's title and its close button; true when it is clicked.
fn title_row(ui: &mut egui::Ui, p: &Palette, title: &str, width: f32) -> bool {
    ui.set_width(width);
    let mut close = false;
    ui.allocate_ui_with_layout(
        Vec2::new(width, TITLE_ROW),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            close = widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true);
            ui.add(widgets::elided_label(
                ui,
                RichText::new(title).size(15.0).strong().color(p.ink),
                ui.available_width(),
            ));
        },
    );
    ui.add_space(8.0);
    close
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{GridChoice, Settings};

    fn request(command: JoinCommand, clicked: bool, document: Option<&str>) -> JoinRequest {
        JoinRequest {
            command,
            clicked,
            document: document.map(str::to_owned),
        }
    }

    fn join() -> JoinCommand {
        JoinCommand::Join {
            username: "synthetic.resident".into(),
            password: "synthetic-input".into(),
        }
    }

    #[test]
    fn account_details_come_from_a_second_life_page_only() {
        let now = Instant::now();
        for document in [
            "https://join.secondlife.com",
            "https://secondlife.com",
            "https://join.secondlife.com:443",
        ] {
            assert!(accepted(&request(join(), true, Some(document)), &mut None, now), "{document}");
        }
        for document in [
            None,
            Some("http://join.secondlife.com"),
            Some("https://join.secondlife.com.example.org"),
            Some("https://notsecondlife.com"),
            Some("https://example.org"),
        ] {
            let mut last = None;
            assert!(!accepted(&request(join(), false, document), &mut last, now), "{document:?}");
            // a refused one does not hold back the next command
            assert_eq!(last, None);
        }
        // the other commands come from any page of the window
        assert!(accepted(
            &request(JoinCommand::Close, true, Some("https://example.org")),
            &mut None,
            now
        ));
        assert!(accepted(&request(JoinCommand::Privacy, true, None), &mut None, now));
    }

    #[test]
    fn commands_without_a_click_wait_five_seconds() {
        let start = Instant::now();
        let mut last = None;
        let script = |command| request(command, false, Some("https://join.secondlife.com"));
        assert!(accepted(&script(JoinCommand::TermsOfService), &mut last, start));
        assert!(!accepted(&script(join()), &mut last, start + Duration::from_secs(4)));
        // a click is never throttled, and does not restart the wait
        assert!(accepted(
            &request(JoinCommand::Privacy, true, None),
            &mut last,
            start + Duration::from_secs(4)
        ));
        assert_eq!(last, Some(start));
        assert!(accepted(&script(join()), &mut last, start + THROTTLE));
        assert_eq!(last, Some(start + THROTTLE));
    }

    #[test]
    fn the_button_leads_to_second_life_or_the_grid_s_own_page() {
        let mut settings = Settings::default();
        for grid in [GridChoice::SecondLife, GridChoice::SecondLifeBeta] {
            let name = format!("{grid:?}");
            settings.grid = grid;
            assert_eq!(signup_page(&settings), Some(SignupPage::SecondLife), "{name}");
        }
        settings.grid = GridChoice::Custom;
        settings.custom_login_uri = "https://login.agni.lindenlab.com/cgi-bin/login.cgi".into();
        assert_eq!(signup_page(&settings), Some(SignupPage::SecondLife));
        settings.custom_login_uri = "http://grid.example:8002/".into();
        // a grid that is not saved, or names no page: no button
        assert_eq!(signup_page(&settings), None);
        settings.grids.push(crate::grids::Grid {
            name: "Example".into(),
            login: "http://grid.example:8002/".into(),
            register: "javascript:alert(1)".into(),
            ..Default::default()
        });
        assert_eq!(signup_page(&settings), None);
        settings.grids[0].register = " https://grid.example/register ".into();
        assert_eq!(
            signup_page(&settings),
            Some(SignupPage::Grid("https://grid.example/register".into()))
        );
    }

    #[test]
    fn pages_keep_their_size_and_shrink_on_a_small_screen() {
        assert_eq!(fit(JOIN_VIEW, Vec2::new(1920.0, 1080.0)), JOIN_VIEW);
        let small = fit(JOIN_VIEW, Vec2::new(1024.0, 720.0));
        assert!(small.x <= 1024.0 - CHROME.x - 2.0 * SCREEN_MARGIN, "{small:?}");
        assert!(small.y <= 720.0 - CHROME.y - 2.0 * SCREEN_MARGIN, "{small:?}");
        assert_eq!(fit(LEGAL_VIEW, Vec2::new(200.0, 100.0)), MIN_VIEW);
    }

    #[test]
    fn the_page_s_commands_reach_the_window() {
        let mut signup = SignupUi::default();
        signup.open(SignupPage::SecondLife);
        assert!(signup.is_open());
        signup.web.join_requests.push(request(JoinCommand::TermsOfService, true, None));
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let browser = aurora_media::BrowserSettings::default();
        let env = SignupWeb {
            paths: None,
            browser: &browser,
        };
        let mut shown = None;
        let _ = crate::ui::login::test_output(ctx.run_ui(egui::RawInput::default(), |ui| {
            shown = signup.show(ui.ctx(), &theme.palette(), &env);
        }));
        assert!(shown.is_none());
        assert_eq!(signup.legal, Some(TERMS_OF_SERVICE));
        signup
            .web
            .join_requests
            .push(request(join(), true, Some("https://join.secondlife.com")));
        let _ = crate::ui::login::test_output(ctx.run_ui(egui::RawInput::default(), |ui| {
            shown = signup.show(ui.ctx(), &theme.palette(), &env);
        }));
        let credentials = shown.expect("the account's details");
        assert_eq!(
            (credentials.username.as_str(), credentials.password.as_str()),
            ("synthetic.resident", "synthetic-input")
        );
        // the window closes with them
        assert!(!signup.is_open());
        assert_eq!(signup.legal, None);
    }
}
