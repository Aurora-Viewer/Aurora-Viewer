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
                log::info!("startup links: login start location set");
                if let Some(authority) = &location.grid {
                    let grid = self.settings.grids.iter().find(|grid| {
                        crate::grids::authority(&grid.service).as_ref() == Some(authority)
                            || crate::grids::authority(&grid.login).as_ref() == Some(authority)
                            || crate::grids::authority(&grid.gatekeeper).as_ref() == Some(authority)
                    });
                    self.settings.custom_login_uri = grid
                        .map(|grid| grid.login.clone())
                        .unwrap_or_else(|| format!("http://{authority}/"));
                    self.settings.grid = crate::settings::GridChoice::Custom;
                }
                self.login_form.start_slurl = Some(location);
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
