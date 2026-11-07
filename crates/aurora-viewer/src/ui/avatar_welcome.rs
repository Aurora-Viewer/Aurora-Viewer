//! The Avatar Welcome Pack window, like LLFloaterAvatarWelcomePack (indra/
//! newview/llfloateravatarwelcomepack.cpp and skins/default/xui/en/
//! floater_avatar_welcome_pack.xml, originally LGPL 2.1) opened by the
//! toolbar's `avatar` command (commands.xml: Floater.Toggle
//! avatar_welcome_pack) and Me › Avatar Welcome Pack… (menu_viewer.xml,
//! after the attachments): the grid's page of ready-made avatars in a
//! browser without chrome. Its web view is 530 x 438 like the Legacy
//! `<web_browser>` (the window adds the title bar and frame), which is also
//! its minimum; unlike the Legacy Viewer's fixed floater it can be enlarged,
//! a deliberate difference asked for by the human after testing on the grid,
//! and the web view follows the window.
//!
//! Its browser is trusted (`trusted_content="true"`): a body or an outfit
//! chosen on the page sends `secondlife://app/wear_folder/?folder_name=…`
//! (the body, replacing the outfit) then `add_folder` (the clothing), which
//! the app wears from the Library (`app/avatar_welcome.rs`). The page's
//! other links are those of the shared `WebView`.
//!
//! The page is the grid's (LFSimFeatureHandler::avatarPickerURL): on Second
//! Life the `AvatarWelcomePack` setting, on another grid the region's
//! "avatar-picker-url", else the login response's. A grid that names none
//! has no welcome pack: the button is dimmed and the menu entry hidden
//! (GridFeatureCheck avatar_picker).

use super::web_view::WebView;
use super::widgets::{self, Floater};
use crate::slapp::{FolderAction, FolderCommand};
use crate::theme::Palette;
use egui::{Rect, RichText, Vec2};

pub const WINDOW: &str = "avatar_welcome_pack";

/// What the page gets, from floater_avatar_welcome_pack.xml: its
/// `<web_browser>` is 530 x 438 (the floater adds the title bar around it).
/// The page's avatar images are laid out for that view, so it is the web
/// view, not the window, that opens at this size, and it is also the
/// smallest it can be: the window can only grow (the Legacy floater has
/// `can_resize="false"`).
const VIEW: Vec2 = Vec2::new(530.0, 438.0);

/// What the window adds around the web view before it is measured: the
/// title bar and the frame (`measure_chrome` replaces it with the real one).
const FIRST_CHROME: Vec2 = Vec2::new(2.0, super::layout::TITLE_BAR_H + 7.0);

/// The window that holds a `VIEW` web view inside `chrome`.
fn window_size(chrome: Vec2) -> Vec2 {
    (VIEW + chrome).round()
}

/// The `AvatarWelcomePack` default (app_settings/settings.xml).
const SECOND_LIFE_PAGE: &str = "https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/[GRID_LOWERCASE]/vawp/index.html";

/// The grid the page is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackGrid<'a> {
    /// Second Life's main grid (grid id "Agni").
    SecondLife,
    /// Second Life's beta grid (grid id "Aditi").
    SecondLifeBeta,
    /// Another grid: the page it names, the region's first. `region` is
    /// `None` when the region does not give the key at all: then (and only
    /// then) the login response's page counts, as in
    /// LFSimFeatureHandler::setSupportedFeatures.
    OpenSim {
        region: Option<&'a str>,
        login: &'a str,
        /// The grid's id ([GRID_LOWERCASE]): its saved nickname.
        id: &'a str,
    },
}

/// The welcome pack page of `grid`, None on a grid without one (or with an
/// address that is not a web page). [GRID_LOWERCASE], [GRID] and [LANGUAGE]
/// are filled as LLWeb::expandURLSubstitutions does ([GRID] is
/// "secondlife.com" on every grid but Linden Lab's staging one);
/// [SESSION_ID] is left empty: the session never goes to the page's site.
/// Spaces and backslashes are escaped (LLWeb::escapeURL).
pub fn pack_url(grid: PackGrid, language: &str) -> Option<String> {
    let (template, id) = match grid {
        PackGrid::SecondLife => (SECOND_LIFE_PAGE, "agni"),
        PackGrid::SecondLifeBeta => (SECOND_LIFE_PAGE, "aditi"),
        PackGrid::OpenSim { region, login, id } => {
            let template = region.unwrap_or(login).trim();
            if template.is_empty() {
                return None;
            }
            (template, id)
        }
    };
    let url = template
        .replace("[GRID_LOWERCASE]", &id.to_lowercase())
        .replace("[GRID]", "secondlife.com")
        .replace("[LANGUAGE]", language)
        .replace("[SESSION_ID]", "")
        .replace(' ', "%20")
        .replace('\\', "%5C");
    is_web_page(&url).then_some(url)
}

/// An http or https address with a host.
fn is_web_page(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty()))
}

/// Where the window's browser looks and with what.
pub struct PackWeb<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    pub cookie: Option<&'a aurora_media::Cookie>,
    /// The grid's welcome pack page, None on a grid without one.
    pub url: Option<&'a str>,
}

#[derive(Default)]
pub struct AvatarWelcomeUi {
    web: WebView,
    /// The page the running browser was sent to (the grid's page can change
    /// after a Hypergrid teleport: handleUrlChanged).
    loaded: String,
    /// What the window adds around the web view, as last measured.
    chrome: Option<Vec2>,
}

impl AvatarWelcomeUi {
    /// The clicked page owns native keys, like LLMediaCtrl.
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

    /// Draw the window; returns the folder commands its page sent.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, env: &PackWeb) -> Vec<FolderCommand> {
        if !*open {
            // the browser does not run behind a closed window
            if self.web.running() || !self.loaded.is_empty() {
                self.web = WebView::default();
                self.loaded.clear();
            }
            return Vec::new();
        }
        let work = super::layout::work_area(ctx);
        let chrome = self.chrome.unwrap_or(FIRST_CHROME);
        let size = window_size(chrome);
        let default = Rect::from_min_size((work.center() - size * 0.5).round(), size);
        let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("toolbar-command-avatar"), default.min, default.size());
        floater.min_size = size;
        floater.resizable = true;
        floater.flush_body = true;
        let view = floater.show(ctx, p, open, |ui| self.contents(ui, p, env));
        // the title bar and frame as laid out: the web view never gets less
        // than VIEW, so the page's images are not cut
        let outer = ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW)));
        if let (Some(view), Some(outer)) = (view.flatten(), outer)
            && !widgets::floater_minimized(ctx, WINDOW)
        {
            let measured = (outer.size() - view).max(Vec2::ZERO);
            self.chrome = Some(measured);
            if (view.x < VIEW.x - 0.5 || view.y < VIEW.y - 0.5) && !ctx.input(|i| i.pointer.any_down()) {
                widgets::size_floater(ctx, WINDOW, outer.size().max(window_size(measured)));
            }
        }
        let commands = std::mem::take(&mut self.web.folder_commands);
        // LLWearFolderHandler: the page lets go of the keyboard once a body
        // is worn; and no window keeps it once closed or minimized
        let release = commands.iter().any(|c| c.action == FolderAction::Wear) || !*open || widgets::floater_minimized(ctx, WINDOW);
        if release && self.web.focused {
            self.web.focused = false;
            if let Some(plugin) = self.web.plugin_mut() {
                plugin.focus(false);
            }
        }
        commands
    }

    /// Draws the page; returns the web view's size (None: no page).
    fn contents(&mut self, ui: &mut egui::Ui, p: &Palette, env: &PackWeb) -> Option<Vec2> {
        let Some(url) = env.url else {
            // the page is gone (a Hypergrid region): stop the browser so that
            // no focus or key stays with a page that is not drawn
            if self.web.running() || self.web.focused || !self.loaded.is_empty() {
                self.web.close();
                self.loaded.clear();
            }
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("avatar-welcome-unavailable"))
                        .size(12.5)
                        .color(p.muted),
                );
            });
            return None;
        };
        if self.loaded != url {
            log::info!("avatar welcome pack: {url}");
            self.loaded = url.to_owned();
            self.web.trust(url);
            self.web.navigate_again(url);
        }
        let size = ui.available_size().max(Vec2::splat(120.0));
        self.web.show(ui, p, env.paths, env.browser, env.cookie, size);
        Some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_life_pages_follow_the_grid_id() {
        assert_eq!(
            pack_url(PackGrid::SecondLife, "fr").as_deref(),
            Some("https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/agni/vawp/index.html")
        );
        assert_eq!(
            pack_url(PackGrid::SecondLifeBeta, "en").as_deref(),
            Some("https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/aditi/vawp/index.html")
        );
    }

    #[test]
    fn another_grid_names_its_own_page_or_has_none() {
        let grid = |region: &'static str, login| PackGrid::OpenSim {
            region: Some(region).filter(|r| *r != "<absent>"),
            login,
            id: "MyGrid",
        };
        // the region's first, then the login response's
        assert_eq!(
            pack_url(grid(" https://r.example/[GRID_LOWERCASE]/[LANGUAGE] ", "https://l.example/"), "ja").as_deref(),
            Some("https://r.example/mygrid/ja")
        );
        assert_eq!(
            pack_url(grid("<absent>", "https://l.example/p?x=a b"), "en").as_deref(),
            Some("https://l.example/p?x=a%20b")
        );
        // the region's key given empty: no welcome pack, whatever the login says
        assert_eq!(pack_url(grid("", "https://l.example/"), "en"), None);
        // none named, or not a web page: no welcome pack
        assert_eq!(pack_url(grid("<absent>", ""), "en"), None);
        assert_eq!(pack_url(grid("javascript:alert(1)", ""), "en"), None);
        assert_eq!(pack_url(grid("file:///c:/pack.html", ""), "en"), None);
        assert_eq!(pack_url(grid("https://", ""), "en"), None);
    }

    #[test]
    fn the_session_never_goes_into_the_address() {
        let grid = PackGrid::OpenSim {
            region: Some("https://g.example/?sid=[SESSION_ID]&grid=[GRID]"),
            login: "",
            id: "",
        };
        assert_eq!(pack_url(grid, "en").as_deref(), Some("https://g.example/?sid=&grid=secondlife.com"));
    }

    #[test]
    fn the_web_view_opens_at_the_legacy_size_and_never_shrinks_below_it() {
        assert_eq!(VIEW, Vec2::new(530.0, 438.0));
        // the window is the view plus what it adds around it, whatever that is
        for chrome in [FIRST_CHROME, Vec2::new(0.0, 43.0), Vec2::new(4.0, 50.0)] {
            assert_eq!(window_size(chrome) - chrome, VIEW);
        }
    }

    #[test]
    fn the_window_keeps_its_size() {
        let browser = aurora_media::BrowserSettings::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[window_size(FIRST_CHROME)],
            aurora_i18n::text("avatar-welcome-unavailable"),
            AvatarWelcomeUi::default,
            |ui, ctx, p| {
                let env = PackWeb {
                    paths: None,
                    browser: &browser,
                    cookie: None,
                    url: None,
                };
                ui.show(ctx, p, &mut true, &env);
            },
        );
    }
}
