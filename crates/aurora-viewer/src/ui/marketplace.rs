//! The Marketplace window, like LLFloaterMarketplace (indra/newview/
//! llfloatermarketplace.cpp, originally LGPL 2.1) opened by Firestorm's
//! World › SL Marketplace (FSLoadSLMarketplace in llviewermenu.cpp): the
//! grid's Marketplace web site in a browser of its own, signed in with the
//! session's OpenID cookie, with back, forward, reload, home and "open in
//! browser". Marketplace links clicked anywhere open here too
//! (LLWeb::loadURLInternal / isMarketplaceURL).

use super::web_view::WebView;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use egui::{Rect, RichText, Vec2};

pub const WINDOW: &str = "marketplace";

/// floater_marketplace.xml opens at 980 x 700 (a web page needs the room).
const DEFAULT_SIZE: Vec2 = Vec2::new(980.0, 680.0);
const MIN_SIZE: Vec2 = Vec2::new(560.0, 400.0);

/// The `MarketplaceURL` setting of Firestorm (app_settings/settings.xml). Its
/// menu entry only shows on Second Life grids (GridCheck "secondlife"), and
/// opens this address on both of them.
const SECOND_LIFE: &str = "https://marketplace.secondlife.com/";

/// The Marketplace page of a grid, None on a grid without one.
pub fn home(grid: &crate::settings::GridChoice) -> Option<&'static str> {
    match grid {
        crate::settings::GridChoice::SecondLife | crate::settings::GridChoice::SecondLifeBeta => Some(SECOND_LIFE),
        crate::settings::GridChoice::Custom => None,
    }
}

/// LLFloaterMarketplace::isMarketplaceURL: the address starts with the
/// Marketplace's, scheme and trailing slashes left out.
pub fn is_marketplace_url(url: &str, home: &str) -> bool {
    fn trim(url: &str) -> &str {
        let url = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
        url.trim_end_matches('/')
    }
    let home = trim(home);
    !home.is_empty() && trim(url).starts_with(home)
}

/// Where the browser of the window looks and with what (the plugins, the
/// shared browser settings and the session's OpenID cookie).
pub struct MarketWeb<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    pub cookie: Option<&'a aurora_media::Cookie>,
    /// The grid's Marketplace page, None on a grid without one.
    pub home: Option<&'a str>,
}

#[derive(Default)]
pub struct MarketplaceUi {
    pub open: bool,
    raise: bool,
    /// The browser keeps the shared profile, like Firestorm's: a sign-in on
    /// the site lasts from one opening to the next.
    web: WebView,
    /// An address to show instead of the home page (a Marketplace link).
    wanted: Option<String>,
    /// The home page the browser was last sent to.
    home_loaded: String,
}

impl MarketplaceUi {
    /// The system id of the marketplace page's browser process, while one
    /// runs.
    pub fn web_process_id(&self) -> Option<u32> {
        self.web.process_id()
    }

    /// LLFloaterMarketplace::onOpen: the home page, or `url` when a link
    /// asked for one (openMarketplaceURL).
    pub fn open(&mut self, url: Option<String>) {
        log::info!("marketplace window opened ({})", if url.is_some() { "link" } else { "home" });
        self.open = true;
        self.raise = true;
        self.wanted = url;
        if self.wanted.is_none() {
            // openMarketplace: back home only when elsewhere
            self.home_loaded.clear();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, env: &MarketWeb) {
        if !self.open {
            // the browser does not run behind a closed window
            if self.web.running() {
                self.web.close();
                self.home_loaded.clear();
            }
            return;
        }
        let work = super::layout::work_area(ctx);
        if std::mem::take(&mut self.raise) {
            widgets::expand_floater(ctx, WINDOW);
        }
        let size = DEFAULT_SIZE.min(work.size() - Vec2::splat(2.0 * super::layout::GAP)).max(MIN_SIZE);
        let default = Rect::from_min_size((work.center() - size * 0.5).round(), size);
        let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("marketplace-title"), default.min, default.size());
        floater.min_size = MIN_SIZE;
        let mut open = self.open;
        floater.show(ctx, p, &mut open, |ui| self.contents(ui, p, env));
        self.open = open;
    }

    fn contents(&mut self, ui: &mut egui::Ui, p: &Palette, env: &MarketWeb) {
        let Some(home) = env.home else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("marketplace-unavailable")).size(12.5).color(p.muted));
            });
            return;
        };
        if let Some(url) = self.wanted.take() {
            self.web.navigate_again(&url);
            self.home_loaded = home.to_owned();
        } else if self.home_loaded != home {
            self.home_loaded = home.to_owned();
            self.web.navigate_again(home);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let running = self.web.running();
            if widgets::icon_button(ui, p, "arrow-left", aurora_i18n::tr!("search-web-back"), running) {
                self.web.back();
            }
            if widgets::icon_button(ui, p, "arrow-right", aurora_i18n::tr!("search-web-forward"), running) {
                self.web.forward();
            }
            if widgets::icon_button(ui, p, "reload", aurora_i18n::tr!("search-web-reload"), running) {
                self.web.reload();
            }
            if widgets::icon_button(ui, p, "home-filled", aurora_i18n::tr!("marketplace-home"), true) {
                self.web.navigate_again(home);
            }
            ui.add_space(6.0);
            let location = if self.web.location.is_empty() {
                home
            } else {
                self.web.location.as_str()
            };
            let shown = location.to_owned();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = aurora_i18n::tr!("search-web-open-external");
                // the page itself, in the system browser (loadURLExternal): not
                // through the links' routing, which would bring it back here
                if widgets::action_button(ui, p, "external-link-filled", label, ActionKind::Flat, Vec2::new(0.0, 22.0)).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(shown.clone()));
                }
                ui.add(egui::Label::new(RichText::new(shown).size(12.0).color(p.muted)).truncate());
            });
        });
        ui.add_space(4.0);
        let size = Vec2::new(ui.available_width(), (ui.available_height() - 2.0).max(120.0));
        self.web.show(ui, p, env.paths, env.browser, env.cookie, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_urls_are_recognised_like_firestorm() {
        let home = SECOND_LIFE;
        for url in [
            "https://marketplace.secondlife.com/",
            "https://marketplace.secondlife.com",
            "http://marketplace.secondlife.com/p/Some-Item/12345",
            "https://marketplace.secondlife.com/stores/42",
        ] {
            assert!(is_marketplace_url(url, home), "{url}");
        }
        for url in [
            "https://secondlife.com/",
            "https://my.secondlife.com/someone",
            "https://example.com/marketplace.secondlife.com",
            "marketplace.secondlife.co",
        ] {
            assert!(!is_marketplace_url(url, home), "{url}");
        }
        // no home, no Marketplace
        assert!(!is_marketplace_url("https://marketplace.secondlife.com/", ""));
    }

    #[test]
    fn the_marketplace_belongs_to_the_second_life_grids() {
        use crate::settings::GridChoice;
        assert_eq!(home(&GridChoice::SecondLife), Some(SECOND_LIFE));
        assert_eq!(home(&GridChoice::SecondLifeBeta), Some(SECOND_LIFE));
        assert_eq!(home(&GridChoice::Custom), None);
    }

    #[test]
    fn a_link_opens_its_page_and_the_menu_goes_home() {
        let mut m = MarketplaceUi {
            home_loaded: SECOND_LIFE.into(),
            ..Default::default()
        };
        m.open(Some("https://marketplace.secondlife.com/p/x".into()));
        assert!(m.open);
        assert_eq!(m.wanted.as_deref(), Some("https://marketplace.secondlife.com/p/x"));
        m.wanted = None;
        // World › SL Marketplace: back to the home page (openMarketplace)
        m.open(None);
        assert!(m.home_loaded.is_empty());
    }
}
