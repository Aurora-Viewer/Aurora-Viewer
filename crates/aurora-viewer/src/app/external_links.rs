//! Links from outside the viewer (`external_url.rs`): the startup SLURL,
//! the ones later launches hand over (`slurl_handoff.rs`), and the
//! `--login` / `--grid` of the command line (LLAppViewer::initConfiguration,
//! LLURLDispatcherImpl::dispatchRegion, LLLoginHandler; indra/newview/
//! llappviewer.cpp, llurldispatcher.cpp, llloginhandler.cpp, originally
//! LGPL 2.1).
//!
//! Before login a place becomes the login screen's start location (for
//! this login only, as LLStartUp::setStartSLURL: the saved choice stays);
//! once in the world it opens the place's details. A command, or a place
//! received while logging in, waits for the world. Deliberate difference:
//! the Legacy Viewer drops a command given at startup (LLStartUp::dispatchURL
//! is no longer called); Aurora runs it once in the world.

use super::*;
use crate::external_url::{Request, Throttle};
use crate::ui::context::CtxAction;
use crate::world::notifications::{Data, Kind};

#[derive(Default)]
pub(super) struct ExternalLinks {
    /// Links of later launches (`slurl_handoff::listen`).
    rx: Option<crossbeam_channel::Receiver<String>>,
    /// The command line's SLURL, read at the first loop.
    startup: Option<String>,
    /// `--login`, until the login screen takes it.
    login: Option<crate::cli::Login>,
    /// Requests waiting for the world.
    waiting: Vec<Request>,
    throttle: Throttle,
    /// A place link naming another grid than the login screen's, until the
    /// resident answers the question (`grid_link_question`).
    grid_ask: Option<GridAsk>,
}

/// The question a place link from outside raises before it may change the
/// login grid.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GridAsk {
    location: crate::slurl::Location,
    /// The login address the link leads to.
    login: String,
    /// The grid manager's name for it, when the grid is a saved one.
    name: Option<String>,
}

/// What a place link from outside does to the login screen's grid.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum GridSwitch {
    /// It names no grid, or the one already chosen: only the start location
    /// changes.
    Same,
    /// Another grid: the resident is asked first, since the password goes
    /// to the grid one logs in to.
    Ask { login: String, name: Option<String> },
}

/// Where `authority` (the `host:port` of a `hop://` or
/// `x-grid-location-info://` link) would take the login screen. A saved
/// grid keeps its own login address; an unknown one is reached over
/// https only, never plain http. Deliberate difference: the Legacy Viewer
/// switches the login grid of a hypergrid link without asking
/// (LLSLURL / LLGridManager); here a link from outside never changes it on
/// its own.
pub(super) fn grid_switch(settings: &Settings, authority: Option<&str>) -> GridSwitch {
    let Some(authority) = authority.map(str::trim).filter(|a| !a.is_empty()) else {
        return GridSwitch::Same;
    };
    let same = |uri: &str| crate::grids::authority(uri).is_some_and(|a| a.eq_ignore_ascii_case(authority));
    let saved = settings
        .grids
        .iter()
        .find(|grid| same(&grid.service) || same(&grid.login) || same(&grid.gatekeeper));
    let login = saved.map_or_else(|| format!("https://{authority}/"), |grid| grid.login.clone());
    let current = settings.login_uri();
    if same(&current) || current.trim_end_matches('/').eq_ignore_ascii_case(login.trim_end_matches('/')) {
        return GridSwitch::Same;
    }
    GridSwitch::Ask {
        login,
        name: saved.map(|grid| grid.name.clone()),
    }
}

impl ExternalLinks {
    pub(super) fn start() -> Self {
        crate::url_scheme::repair_at_start();
        ExternalLinks {
            rx: crate::slurl_handoff::listen(),
            startup: crate::cli::get().url.clone(),
            login: crate::cli::take_login(),
            ..Default::default()
        }
    }
}

/// `--grid` / `--loginuri` on the settings (the login screen's grid).
pub(super) fn apply_grid(settings: &mut Settings) {
    match &crate::cli::get().grid {
        Some(crate::cli::Grid::SecondLife) => settings.grid = crate::settings::GridChoice::SecondLife,
        Some(crate::cli::Grid::SecondLifeBeta) => settings.grid = crate::settings::GridChoice::SecondLifeBeta,
        Some(crate::cli::Grid::LoginUri(uri)) => {
            settings.grid = crate::settings::GridChoice::Custom;
            settings.custom_login_uri = uri.clone();
        }
        Some(crate::cli::Grid::Named(name)) => {
            if let Some(grid) = settings
                .grids
                .iter()
                .find(|grid| grid.name.eq_ignore_ascii_case(name) || grid.nickname.eq_ignore_ascii_case(name))
            {
                settings.custom_login_uri = grid.login.clone();
                settings.grid = crate::settings::GridChoice::Custom;
            } else {
                eprintln!("{}", aurora_i18n::tr!("cli-unknown-grid", grid = name.clone()));
            }
        }
        None => {}
    }
}

impl App {
    /// Each loop: the startup link, the links handed over since, and the
    /// requests that waited for the world.
    pub(super) fn external_links(&mut self) {
        if let Some(url) = self.external.startup.take() {
            self.open_external_url(&url, false);
        }
        let received: Vec<String> = self.external.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for url in received {
            self.open_external_url(&url, true);
        }
        if matches!(self.screen, Screen::World) && !self.external.waiting.is_empty() {
            for request in std::mem::take(&mut self.external.waiting) {
                self.run_external(request);
            }
        }
    }

    /// A link from outside; `raise` when another launch handed it over: the
    /// window comes to the front (LLViewerWindow::handleDataCopy).
    fn open_external_url(&mut self, url: &str, raise: bool) {
        let request = crate::external_url::classify(url);
        if request == Request::Ignored {
            log::info!("startup links: not a SLURL, ignored");
            return;
        }
        if raise && let Some(g) = &self.gfx {
            g.window.set_minimized(false);
            g.window.focus_window();
        }
        match request {
            Request::Place(location) if matches!(self.screen, Screen::Login) && !self.login_form.busy => {
                match grid_switch(&self.settings, location.grid.as_deref()) {
                    GridSwitch::Same => {
                        log::info!("startup links: login start location set");
                        self.login_form.start_slurl = Some(location);
                    }
                    GridSwitch::Ask { login, name } => {
                        log::info!("startup links: the link names another grid, asking before switching");
                        self.external.grid_ask = Some(GridAsk { location, login, name });
                    }
                }
            }
            request if matches!(self.screen, Screen::World) => self.run_external(request),
            request => {
                log::info!("startup links: waiting for the world");
                self.external.waiting.push(request);
            }
        }
    }

    /// LLURLDispatcher::dispatch with NAV_TYPE_EXTERNAL, once in the world.
    fn run_external(&mut self, request: Request) {
        match request {
            Request::Place(l) => {
                log::info!("startup links: place details");
                self.on_ctx_action(CtxAction::ShowPlaceInfo(l.target(), l.pos));
            }
            Request::Profile(_) | Request::App(_) => {
                if self.external.throttle.allow(Instant::now()) {
                    log::info!("startup links: command run");
                    match request {
                        Request::Profile(id) => crate::ui::profile::request_open(&self.egui_ctx, id),
                        Request::App(link) => self.run_app_link(link),
                        _ => {}
                    }
                } else {
                    log::info!("startup links: command throttled");
                    self.world
                        .notifications
                        .push(Kind::Info, aurora_i18n::tr!("external-link-throttled"), "", Data::None);
                }
            }
            Request::Blocked => {
                log::info!("startup links: command blocked");
                self.world
                    .notifications
                    .push(Kind::Info, aurora_i18n::tr!("external-link-blocked"), "", Data::None);
            }
            Request::Ignored => {}
        }
    }

    /// The login screen's question when a place link from outside names
    /// another grid: the grid and the start location change only on
    /// "Switch grid". Changing the grid empties the typed password, and a
    /// remembered one belongs to its own grid (`sync_remembered_password`).
    pub(super) fn grid_link_question(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(ask) = &self.external.grid_ask else {
            return;
        };
        if self.login_form.busy {
            // a login started meanwhile: the link no longer applies
            self.external.grid_ask = None;
            return;
        }
        let Some(yes) = ui::context::grid_link_confirm(ctx, p, &ask.login, ask.name.as_deref()) else {
            return;
        };
        let Some(ask) = self.external.grid_ask.take() else {
            return;
        };
        if yes {
            log::info!("startup links: grid switched at the resident's request");
            self.settings.custom_login_uri = ask.login;
            self.settings.grid = crate::settings::GridChoice::Custom;
            self.login_form.start_slurl = Some(ask.location);
        } else {
            log::info!("startup links: grid switch declined");
        }
    }

    /// `--login <first> <last> <password>`: logs in as soon as the login
    /// screen allows it, on the `--grid` (AutoLogin with the command line's
    /// credential; the password follows the usual remember choice).
    pub(super) fn command_line_login(&mut self) {
        if self.login_form.busy || self.external.login.is_none() {
            return;
        }
        let Some(login) = self.external.login.take() else {
            return;
        };
        if let Some(reason) = self.login_form.block_reason {
            log::warn!("command line login: not possible ({reason})");
            return;
        }
        log::info!("command line login");
        // a single user name is "Name Resident" on the command line
        self.settings.username = if login.last.eq_ignore_ascii_case("resident") {
            login.first
        } else {
            format!("{} {}", login.first, login.last)
        };
        apply_grid(&mut self.settings);
        self.login_form.editing = false;
        self.login_form.password = login.password;
        self.sync_remembered_password();
        self.start_login();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_on(grid: crate::settings::GridChoice, custom: &str, saved: &[&str]) -> Settings {
        Settings {
            grid,
            custom_login_uri: custom.to_owned(),
            grids: saved
                .iter()
                .map(|address| crate::grids::Grid::manual(address).expect("valid test grid"))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_link_without_a_grid_changes_only_the_start_location() {
        let settings = settings_on(crate::settings::GridChoice::SecondLife, "", &[]);
        assert_eq!(grid_switch(&settings, None), GridSwitch::Same);
        assert_eq!(grid_switch(&settings, Some("  ")), GridSwitch::Same);
    }

    #[test]
    fn an_unknown_grid_is_asked_for_and_reached_over_https_only() {
        let settings = settings_on(crate::settings::GridChoice::SecondLife, "", &[]);
        assert_eq!(
            grid_switch(&settings, Some("grid.example:8002")),
            GridSwitch::Ask {
                login: "https://grid.example:8002/".into(),
                name: None
            }
        );
    }

    #[test]
    fn a_saved_grid_keeps_its_login_address_and_is_still_asked_for() {
        let settings = settings_on(crate::settings::GridChoice::SecondLife, "", &["http://grid.example:8002"]);
        let GridSwitch::Ask { login, name } = grid_switch(&settings, Some("GRID.example:8002")) else {
            panic!("switching from Second Life to a saved grid must ask");
        };
        assert_eq!(login, settings.grids[0].login);
        assert_eq!(name.as_deref(), Some(settings.grids[0].name.as_str()));
    }

    #[test]
    fn the_grid_already_chosen_is_not_asked_for_again() {
        let settings = settings_on(
            crate::settings::GridChoice::Custom,
            "http://grid.example:8002/",
            &["http://grid.example:8002"],
        );
        assert_eq!(grid_switch(&settings, Some("grid.example:8002")), GridSwitch::Same);
        // the same grid typed by hand, not saved
        let settings = settings_on(crate::settings::GridChoice::Custom, "https://other.example/", &[]);
        assert_eq!(grid_switch(&settings, Some("other.example")), GridSwitch::Same);
    }
}
