//! A web page inside a window, like LLMediaCtrl: a CEF plugin of its own
//! (`aurora-media`), its pixels in an egui texture, mouse, wheel and (once
//! clicked) keyboard sent to the page. Used by the "Feed" tab of the
//! profiles (LLPanelProfileWeb) and by the web tab of the search window
//! (FSPanelSearchWeb), which also follows the page's address and goes back
//! and forward in its history. The search tab's browser is a private one
//! (`WebView::private`): its cache, history and log go to a profile of its
//! own that is deleted with the browser (`media/private_profile.rs`).

use crate::media::pack::PackState;
use crate::media::private_profile::PrivateProfile;
use aurora_media::{BrowserSettings, Cookie, MediaEvent, MediaPlugin, Modifiers, MouseEvent, PluginPaths};
use egui::{Color32, Vec2};
use std::path::PathBuf;
use std::time::Instant;

const PLUGINS_STATUS: &str = "media_plugins_pack_state";

/// State of Aurora's media plugins pack, published once per frame by the app
/// for the pages opened before the pack is installed.
pub fn set_plugins_status(ctx: &egui::Context, state: PackState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(PLUGINS_STATUS), state));
}

/// What a page without plugins shows while the pack is on its way; `None`
/// when no pack is coming.
fn waiting_for_plugins(ctx: &egui::Context) -> Option<String> {
    match ctx.data(|d| d.get_temp::<PackState>(egui::Id::unique(PLUGINS_STATUS)))? {
        PackState::Downloading(p) if p.preparing => Some(aurora_i18n::tr!("media-plugins-installing").to_owned()),
        PackState::Downloading(p) => Some(aurora_i18n::tr!(
            "media-plugins-page-waiting",
            percent = (p.received * 100 / p.total.max(1)).to_string()
        )),
        PackState::Failed(aurora_updater::Failure::Unsupported) | PackState::Idle | PackState::Ready => None,
        PackState::Failed(_) => Some(aurora_i18n::tr!("media-plugins-page-failed").to_owned()),
    }
}

#[derive(Default)]
pub struct WebView {
    // dropped first: the browser stops before its profile is removed
    plugin: Option<MediaPlugin>,
    /// Where this browser's private profiles are made; None for a browser
    /// on the shared cache.
    private_root: Option<PathBuf>,
    /// The profile of the running private browser.
    profile: Option<PrivateProfile>,
    texture: Option<egui::TextureHandle>,
    /// Page to show (loaded once the plugin runs).
    url: String,
    nav_pending: bool,
    size: (i32, i32),
    failed: Option<String>,
    /// The page has the keyboard (clicked; a click elsewhere releases it).
    pub focused: bool,
    nav_started: Option<Instant>,
    /// "Load time: N seconds" (LLPanelProfileWeb::handleMediaEvent).
    pub load_secs: Option<f32>,
    cursor: String,
    scratch: Vec<Color32>,
    /// Address of the page shown, as the browser reports it (it follows the
    /// links clicked in the page).
    pub location: String,
    /// A page is loading (between NavigateBegin and NavigateComplete).
    pub loading: bool,
}

impl WebView {
    /// The system id of this page's browser process, while one runs.
    pub fn process_id(&self) -> Option<u32> {
        self.plugin.as_ref().and_then(|p| p.pid())
    }

    /// A browser that leaves nothing behind: each time it starts it gets a
    /// new profile under `root`, deleted when it stops.
    pub fn private(root: PathBuf) -> WebView {
        WebView {
            private_root: Some(root),
            ..Default::default()
        }
    }

    pub fn navigate(&mut self, url: &str) {
        if self.url != url {
            self.url = url.to_owned();
            self.nav_pending = true;
        }
    }

    /// Stop the browser (window closed).
    pub fn close(&mut self) {
        self.plugin = None;
        // after the browser: the profile is removed once it lets go
        self.profile = None;
        self.texture = None;
        self.focused = false;
    }

    pub fn plugin_mut(&mut self) -> Option<&mut MediaPlugin> {
        self.plugin.as_mut()
    }

    /// The browser runs (a page was asked and the plugin started).
    pub fn running(&self) -> bool {
        self.plugin.is_some()
    }

    /// LLMediaCtrl::navigateBack / navigateForward / reload.
    pub fn back(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_back();
        }
    }

    pub fn forward(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_forward();
        }
    }

    pub fn reload(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_reload(false);
        }
    }

    /// Load `url` even when it is the page asked last ("home" after links
    /// were followed inside the page).
    pub fn navigate_again(&mut self, url: &str) {
        self.url = url.to_owned();
        self.nav_pending = true;
    }

    /// Draw the page in `size` points (starting the browser if needed).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        pal: &crate::theme::Palette,
        paths: Option<&PluginPaths>,
        browser: &BrowserSettings,
        cookie: Option<&Cookie>,
        size: Vec2,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let px = ((size.x * ppp).round().max(16.0) as i32, (size.y * ppp).round().max(16.0) as i32);
        ui.painter().rect_filled(rect, 0.0, pal.field);
        if self.plugin.is_none()
            && self.failed.is_none()
            && paths.is_none()
            && let Some(text) = waiting_for_plugins(ui.ctx())
        {
            // the page starts by itself once the pack is installed
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.0),
                pal.muted,
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            return resp;
        }
        if self.plugin.is_none() && self.failed.is_none() && !self.url.is_empty() {
            // a private browser never falls back on the shared cache
            let private = match &self.private_root {
                Some(root) => match PrivateProfile::create(root) {
                    Ok(profile) => {
                        let settings = profile.settings(browser);
                        self.profile = Some(profile);
                        Ok(Some(settings))
                    }
                    Err(e) => Err(e),
                },
                None => Ok(None),
            };
            let launched = match (&private, paths) {
                (Err(e), _) => Some(Err(std::io::Error::new(e.kind(), e.to_string()))),
                (Ok(settings), Some(paths)) => Some(MediaPlugin::launch(
                    paths,
                    "media_plugin_cef",
                    px.0,
                    px.1,
                    settings.as_ref().unwrap_or(browser),
                    "",
                )),
                (Ok(_), None) => None,
            };
            match launched {
                Some(launched) => match launched {
                    Ok(mut p) => {
                        // LLViewerMedia::getOpenIDCookie before the first page
                        if let Some(c) = cookie {
                            p.set_cookie(c);
                        }
                        self.plugin = Some(p);
                        self.size = px;
                        self.nav_pending = true;
                    }
                    Err(e) => {
                        log::warn!("web view: cannot start the browser: {e}");
                        self.failed = Some(e.to_string());
                    }
                },
                None => self.failed = Some(aurora_i18n::tr!("extra-browser-unavailable-slplugin-cef-plugins").into()),
            }
            // no browser, no profile to keep
            if self.plugin.is_none() {
                self.profile = None;
            }
        }
        let Some(p) = self.plugin.as_mut() else {
            let text = match &self.failed {
                Some(e) => aurora_i18n::tr!("ui-web-view-web-page-unavailable", e = e.to_string()),
                None => aurora_i18n::tr!("common-loading-2").to_owned(),
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.0),
                pal.muted,
            );
            return resp;
        };
        if self.size != px {
            p.set_size(px.0, px.1);
            self.size = px;
        }
        if std::mem::take(&mut self.nav_pending) {
            p.load_uri(&self.url);
            self.nav_started = Some(Instant::now());
            self.load_secs = None;
            self.location = self.url.clone();
            self.loading = true;
        }
        p.idle();
        for ev in p.take_events() {
            match ev {
                MediaEvent::NavigateComplete { uri, .. } => {
                    // the first page of a new browser may complete twice
                    if let Some(t) = self.nav_started.take() {
                        self.load_secs = Some(t.elapsed().as_secs_f32());
                    }
                    if !uri.is_empty() {
                        self.location = uri;
                    }
                    self.loading = false;
                }
                MediaEvent::NavigateBegin(uri) => {
                    if !uri.is_empty() {
                        self.location = uri;
                    }
                    self.loading = true;
                }
                MediaEvent::LocationChanged(uri) if !uri.is_empty() => self.location = uri,
                MediaEvent::CursorChanged(c) => self.cursor = c,
                MediaEvent::PluginFailed(e) => {
                    log::warn!("web view: browser stopped: {e}");
                    self.failed = Some(e);
                }
                _ => {}
            }
        }
        if self.failed.is_some() {
            self.plugin = None;
            self.profile = None;
            return resp;
        }
        // pixels: whole page when it changed (small windows)
        if let Some(frame) = p.frame()
            && (p.dirty().is_some() || self.texture.is_none())
        {
            let (mw, mh) = (frame.media_width as usize, frame.media_height as usize);
            let stride = frame.texture_width as usize;
            self.scratch.clear();
            self.scratch.reserve(mw * mh);
            let mut ok = true;
            for y in 0..mh {
                let row = if frame.bottom_up {
                    frame.texture_height as usize - 1 - y
                } else {
                    y
                };
                let Some(src) = frame.data.get(row * stride * 4..(row * stride + mw) * 4) else {
                    ok = false;
                    break;
                };
                self.scratch
                    .extend(src.as_chunks::<4>().0.iter().map(|s| Color32::from_rgb(s[2], s[1], s[0])));
            }
            if ok && mw > 0 && mh > 0 {
                let img = egui::ColorImage::new([mw, mh], std::mem::take(&mut self.scratch));
                match &mut self.texture {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.texture = Some(ui.ctx().load_texture("web-view", img, egui::TextureOptions::LINEAR)),
                }
            }
            p.reset_dirty();
        }
        if let Some(t) = &self.texture {
            ui.painter().image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                super::widgets::PICTURE_TINT,
            );
        }
        // input, in media pixels from the top left
        let mods = ui.input(|i| Modifiers {
            control: i.modifiers.ctrl,
            alt: i.modifiers.alt,
            shift: i.modifiers.shift,
        });
        let to_px = |pos: egui::Pos2| (((pos.x - rect.left()) * ppp) as i32, ((pos.y - rect.top()) * ppp) as i32);
        if let Some(pos) = resp.hover_pos() {
            let (x, y) = to_px(pos);
            p.mouse_event(MouseEvent::Move, 0, x, y, mods);
            // wheel clicks (LL: down = positive)
            let clicks: f32 = ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit, delta, .. } => match unit {
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Page => delta.y * 3.0,
                            egui::MouseWheelUnit::Point => delta.y / 40.0,
                        },
                        _ => 0.0,
                    })
                    .sum()
            });
            if clicks != 0.0 {
                p.scroll_event(x, y, 0, (-clicks).round() as i32, mods);
            }
            if self.cursor.contains("hand") {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if self.cursor.contains("ibeam") {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        }
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let (pressed, released) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released()));
            let inside = rect.contains(pos);
            if pressed {
                self.focused = inside;
                p.focus(inside);
            }
            let (x, y) = to_px(pos);
            if pressed && inside {
                p.mouse_event(MouseEvent::Down, 0, x, y, mods);
            }
            if released && (inside || resp.dragged()) {
                p.mouse_event(MouseEvent::Up, 0, x, y, mods);
            }
            if resp.double_clicked() {
                p.mouse_event(MouseEvent::DoubleClick, 0, x, y, mods);
            }
        }
        if self.focused {
            ui.painter()
                .rect_stroke(rect, 0.0, egui::Stroke::new(1.0, pal.violet), egui::StrokeKind::Inside);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        resp
    }
}
