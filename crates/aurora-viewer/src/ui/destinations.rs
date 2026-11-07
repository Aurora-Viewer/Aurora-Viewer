//! The Destination Guide window, like LLFloaterDestinations (indra/newview/
//! llfloaterdestinations.cpp and skins/default/xui/en/floater_destinations.xml,
//! originally LGPL 2.1) opened by the toolbar's `destinations` command
//! (commands.xml: Floater.Toggle destinations) and World › Destinations
//! (menu_viewer.xml, after Places): the grid's guide page in a browser
//! without chrome, signed in with the session's OpenID cookie on Second
//! Life (LLViewerMedia::getOpenIDCookie). Its links are those of the shared
//! `WebView`: a teleport link (`secondlife://`) acts in the viewer.
//!
//! The page is the grid's (LFSimFeatureHandler::destinationGuideURL): on
//! Second Life the `DestinationGuideURL` setting, on another grid the
//! region's "destination-guide-url", else the login response's
//! "destination_guide_url". A grid that names none has no guide: the
//! button is dimmed and the menu entry hidden (GridFeatureCheck
//! destination_guide).

use super::web_view::WebView;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use egui::{Rect, RichText, Vec2};

pub const WINDOW: &str = "destinations";

/// floater_destinations.xml: 550 x 225, at least 350 x 225.
const DEFAULT_SIZE: Vec2 = Vec2::new(550.0, 225.0);
const MIN_SIZE: Vec2 = Vec2::new(350.0, 225.0);

/// The `DestinationGuideURL` default (app_settings/settings.xml).
const SECOND_LIFE_GUIDE: &str = "https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/[GRID_LOWERCASE]/guide.html";

/// The grid the guide is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideGrid<'a> {
    /// Second Life's main grid (grid id "Agni").
    SecondLife,
    /// Second Life's beta grid (grid id "Aditi").
    SecondLifeBeta,
    /// Another grid: the guide it names, the region's first. `region` is
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

/// The guide page of `grid`, None on a grid without one (or with an address
/// that is not a web page). [GRID_LOWERCASE], [GRID] and [LANGUAGE] are
/// filled as LLWeb::expandURLSubstitutions does ([GRID] is "secondlife.com"
/// on every grid but Linden Lab's staging one); [SESSION_ID] is left empty:
/// the session never goes to the guide's site. Spaces and backslashes are
/// escaped (LLWeb::escapeURL).
pub fn guide_url(grid: GuideGrid, language: &str) -> Option<String> {
    let (template, id) = match grid {
        GuideGrid::SecondLife => (SECOND_LIFE_GUIDE, "agni"),
        GuideGrid::SecondLifeBeta => (SECOND_LIFE_GUIDE, "aditi"),
        GuideGrid::OpenSim { region, login, id } => {
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
pub struct GuideWeb<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    pub cookie: Option<&'a aurora_media::Cookie>,
    /// The grid's guide page, None on a grid without one.
    pub url: Option<&'a str>,
}

#[derive(Default)]
pub struct DestinationsUi {
    web: WebView,
    /// The page the running browser was sent to (the grid's guide can
    /// change after a Hypergrid teleport: handleUrlChanged).
    loaded: String,
}

impl DestinationsUi {
    /// The clicked guide owns native keys, like LLMediaCtrl.
    pub fn web_has_focus(&self, open: bool) -> bool {
        open && self.web.focused && self.web.running()
    }

    pub fn focused_web(&mut self, open: bool) -> Option<&mut aurora_media::MediaPlugin> {
        self.web_has_focus(open).then(|| self.web.plugin_mut()).flatten()
    }

    /// The system id of the guide's browser process, while one runs.
    pub fn web_process_id(&self) -> Option<u32> {
        self.web.process_id()
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, env: &GuideWeb) {
        if !*open {
            // the browser does not run behind a closed window
            if self.web.running() || !self.loaded.is_empty() {
                self.web = WebView::default();
                self.loaded.clear();
            }
            return;
        }
        let work = super::layout::work_area(ctx);
        let size = DEFAULT_SIZE.min(work.size() - Vec2::splat(2.0 * super::layout::GAP)).max(MIN_SIZE);
        let default = Rect::from_min_size((work.center() - size * 0.5).round(), size);
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-destinations"),
            default.min,
            default.size(),
        );
        floater.min_size = MIN_SIZE;
        floater.show(ctx, p, open, |ui| self.contents(ui, p, env));
        if (!*open || widgets::floater_minimized(ctx, WINDOW)) && self.web.focused {
            self.web.focused = false;
            if let Some(plugin) = self.web.plugin_mut() {
                plugin.focus(false);
            }
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, p: &Palette, env: &GuideWeb) {
        let Some(url) = env.url else {
            // the guide is gone (a Hypergrid teleport): stop the browser so
            // that no focus or key stays with a page that is not drawn
            if self.web.running() || self.web.focused || !self.loaded.is_empty() {
                self.web.close();
                self.loaded.clear();
            }
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("destinations-unavailable"))
                        .size(12.5)
                        .color(p.muted),
                );
            });
            return;
        };
        if self.loaded != url {
            log::info!("destination guide: {url}");
            self.loaded = url.to_owned();
            self.web.navigate_again(url);
        }
        let size = Vec2::new(ui.available_width(), (ui.available_height() - 2.0).max(120.0));
        self.web.show(ui, p, env.paths, env.browser, env.cookie, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_life_guides_follow_the_grid_id() {
        assert_eq!(
            guide_url(GuideGrid::SecondLife, "fr").as_deref(),
            Some("https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/agni/guide.html")
        );
        assert_eq!(
            guide_url(GuideGrid::SecondLifeBeta, "en").as_deref(),
            Some("https://lecs-viewer-web-components.s3.amazonaws.com/v3.0/aditi/guide.html")
        );
    }

    #[test]
    fn another_grid_names_its_own_guide_or_has_none() {
        let grid = |region: &'static str, login| GuideGrid::OpenSim {
            region: Some(region).filter(|r| *r != "<absent>"),
            login,
            id: "MyGrid",
        };
        // the region's first, then the login response's
        assert_eq!(
            guide_url(grid(" https://r.example/[GRID_LOWERCASE]/[LANGUAGE] ", "https://l.example/"), "ja").as_deref(),
            Some("https://r.example/mygrid/ja")
        );
        assert_eq!(
            guide_url(grid("<absent>", "https://l.example/g?x=a b"), "en").as_deref(),
            Some("https://l.example/g?x=a%20b")
        );
        // the region's key given empty: no guide, whatever the login says
        assert_eq!(guide_url(grid("", "https://l.example/"), "en"), None);
        assert_eq!(
            guide_url(grid("<absent>", "https://l.example/g?x=a b"), "en").as_deref(),
            Some("https://l.example/g?x=a%20b")
        );
        // none named, or not a web page: no guide
        assert_eq!(guide_url(grid("<absent>", ""), "en"), None);
        assert_eq!(guide_url(grid("javascript:alert(1)", ""), "en"), None);
        assert_eq!(guide_url(grid("file:///c:/guide.html", ""), "en"), None);
        assert_eq!(guide_url(grid("https://", ""), "en"), None);
    }

    #[test]
    fn the_session_never_goes_into_the_address() {
        let grid = GuideGrid::OpenSim {
            region: Some("https://g.example/?sid=[SESSION_ID]&grid=[GRID]"),
            login: "",
            id: "",
        };
        assert_eq!(
            guide_url(grid, "en").as_deref(),
            Some("https://g.example/?sid=&grid=secondlife.com")
        );
    }

    #[test]
    fn the_window_keeps_its_size() {
        let browser = aurora_media::BrowserSettings::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[DEFAULT_SIZE, MIN_SIZE],
            aurora_i18n::text("destinations-unavailable"),
            DestinationsUi::default,
            |ui, ctx, p| {
                let env = GuideWeb {
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
