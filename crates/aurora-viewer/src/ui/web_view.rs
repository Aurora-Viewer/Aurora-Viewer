//! A web page inside a window, like LLMediaCtrl: a CEF plugin of its own
//! (`aurora-media`), its pixels in an egui texture, mouse, wheel and (once
//! clicked) keyboard sent to the page. Used by the "Feed" tab of the
//! profiles (LLPanelProfileWeb) and by the web tab of the search window
//! (FSPanelSearchWeb), which also follows the page's address and goes back
//! and forward in its history. The search tab's browser is a private one
//! (`WebView::private`): its cache, history and log go to a profile of its
//! own that is deleted with the browser (`media/private_profile.rs`).
//! A trusted browser (`WebView::trust`, LLMediaCtrl's `trusted_content`,
//! the Avatar Welcome Pack's) may also wear folders: the folder commands its
//! own page sends are handed to its window (`folder_commands`).
//! The sign-up window's browser (`WebView::accept_join_commands`) hands its
//! window the sign-up page's commands instead (`join_requests`).

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

const PLUGINS_DOWNLOAD: &str = "media_plugins_pack_download";

/// What a place without plugins shows instead of the web or the media.
enum Waiting {
    /// The pack is on its way, or its download failed.
    Text(String),
    /// Not downloaded (never asked, or "Not now"): a Download button.
    Offer,
}

/// `None` when no pack is coming nor can be downloaded.
fn waiting_for_plugins(ctx: &egui::Context) -> Option<Waiting> {
    let text = match ctx.data(|d| d.get_temp::<PackState>(egui::Id::unique(PLUGINS_STATUS)))? {
        PackState::Idle => return Some(Waiting::Offer),
        PackState::Downloading(p) if p.preparing => aurora_i18n::tr!("media-plugins-installing").to_owned(),
        PackState::Downloading(p) => aurora_i18n::tr!(
            "media-plugins-page-waiting",
            percent = (p.received * 100 / p.total.max(1)).to_string()
        ),
        PackState::Failed(aurora_updater::Failure::Unsupported) | PackState::Ready => return None,
        PackState::Failed(_) => aurora_i18n::tr!("media-plugins-page-failed").to_owned(),
    };
    Some(Waiting::Text(text))
}

/// A Download button was pressed this frame (taken by the app, which
/// records the answer and starts the download).
pub fn take_plugins_download(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(PLUGINS_DOWNLOAD)))
        .unwrap_or(false)
}

/// The pack's size in whole megabytes, as the consent card says it.
pub fn pack_megabytes() -> String {
    aurora_i18n::integer(aurora_updater::media_pack::PACK.size.div_ceil(1024 * 1024) as i64)
}

/// "Web pages and media need …" and its Download button, where the web or a
/// media would show without plugins.
fn plugins_offer(ui: &mut egui::Ui, pal: &crate::theme::Palette) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(aurora_i18n::tr!("media-plugins-page-offer", size = pack_megabytes()))
                .size(12.0)
                .color(pal.muted),
        )
        .wrap(),
    );
    let download = super::widgets::action_button(
        ui,
        pal,
        "download-filled",
        aurora_i18n::tr!("media-plugins-download"),
        super::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    );
    if download.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(PLUGINS_DOWNLOAD), true));
    }
}

/// The line of the media controls (`ui/media.rs`) when no plugins are
/// found: where the pack stands, or its Download button.
pub fn plugins_notice(ui: &mut egui::Ui, pal: &crate::theme::Palette) {
    match waiting_for_plugins(ui.ctx()) {
        Some(Waiting::Offer) => {
            ui.horizontal(|ui| plugins_offer(ui, pal));
        }
        Some(Waiting::Text(text)) => {
            ui.label(egui::RichText::new(text).size(11.0).color(pal.muted));
        }
        None => {}
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
    /// The window of the viewer that had the system's focus at the last
    /// frame (`outside::focused_window`).
    focus_seen: Option<egui::ViewportId>,
    /// A press on the page holds the pointer until its release: the page
    /// gets the moves and the release wherever the pointer goes
    /// (`read_web_pointer`).
    captured: bool,
    /// The last pointer position sent to the page: where a release the
    /// viewer never saw (focus lost mid-drag) is sent.
    last_pos: Option<egui::Pos2>,
    /// This view among the window's pages, for the one capture they share
    /// (`CaptureOwner`); given at its first `show`.
    token: u64,
    nav_started: Option<Instant>,
    /// "Load time: N seconds" (LLPanelProfileWeb::handleMediaEvent).
    pub load_secs: Option<f32>,
    cursor: String,
    scratch: Vec<Color32>,
    /// The sign-in cookie was handed to the running browser.
    cookie_set: bool,
    /// Address of the page shown, as the browser reports it (it follows the
    /// links clicked in the page).
    pub location: String,
    /// A page is loading (between NavigateBegin and NavigateComplete).
    pub loading: bool,
    /// The page's title (LLViewerMediaObserver MEDIA_EVENT_NAME_CHANGED).
    pub title: String,
    /// The browser's status line (MEDIA_EVENT_STATUS_TEXT_CHANGED: the link
    /// under the pointer).
    pub status: String,
    /// Load progress, 0 to 100 (MEDIA_EVENT_PROGRESS_UPDATED).
    pub progress: i32,
    /// The origin (scheme, host, port) of the trusted page, if any.
    trusted: Option<String>,
    /// The origin of the top document the browser has committed
    /// (NavigateComplete or LocationChanged), cleared while a navigation to
    /// another origin is under way: a command is trusted by the page that
    /// sends it, not by the address that was asked. A frame inside that
    /// document is not told apart from it.
    document: Option<String>,
    /// The folder commands the trusted page sent, for its window to take.
    pub folder_commands: Vec<crate::slapp::FolderCommand>,
    /// The sign-up commands are handed to the window (`join_requests`).
    accepts_join: bool,
    /// The sign-up commands the page sent, for its window to take.
    pub join_requests: Vec<JoinRequest>,
}

/// A sign-up command (`secondlife:///app/floaterjoin/…`) a page sent: the
/// window decides, from whether it was clicked and which document sent it
/// (`ui/signup.rs`).
#[derive(Debug)]
pub struct JoinRequest {
    pub command: crate::slapp::JoinCommand,
    /// Sent by a click (NAV_TYPE_CLICKED), not by the page's script alone.
    pub clicked: bool,
    /// The origin of the top document when it was sent (`origin`), None
    /// while a navigation to another origin is under way.
    pub document: Option<String>,
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

    /// LLMediaCtrl `trusted_content`: the pages of `page`'s site may send
    /// the folder commands an untrusted page cannot (UNTRUSTED_BLOCK), clicked
    /// or not (LLViewerMediaImpl::handleMediaEvent dispatches them with the
    /// browser's trust and any nav type). Their other links follow the
    /// usual rules.
    pub fn trust(&mut self, page: &str) {
        let trusted = origin(page);
        if trusted != self.trusted {
            // commands of the previous trust must not outlive it
            self.folder_commands.clear();
            self.trusted = trusted;
        }
    }

    /// The sign-up window's browser: the page's `floaterjoin` commands,
    /// clicked or not, go to `join_requests` instead of the link rules
    /// (LLFloaterJoinHandler is UNTRUSTED_THROTTLE: the window throttles
    /// those that were not clicked).
    pub fn accept_join_commands(&mut self) {
        self.accepts_join = true;
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
        self.captured = false;
        self.last_pos = None;
        self.cookie_set = false;
        self.document = None;
        self.folder_commands.clear();
        self.join_requests.clear();
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

    /// LLMediaCtrl::navigateStop.
    pub fn stop(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_stop();
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
            && let Some(waiting) = waiting_for_plugins(ui.ctx())
        {
            // the page starts by itself once the pack is installed
            match waiting {
                Waiting::Text(text) => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(12.0),
                        pal.muted,
                    );
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                }
                Waiting::Offer => {
                    // the text and its button, in the middle of the page
                    let room = rect.shrink(16.0);
                    let width = room.width().min(360.0);
                    let block = egui::Rect::from_center_size(room.center(), Vec2::new(width, room.height().min(72.0)));
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(block)
                            .layout(egui::Layout::top_down(egui::Align::Center)),
                        |ui| plugins_offer(ui, pal),
                    );
                }
            }
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
                            self.cookie_set = true;
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
        // a cookie that arrived after the browser started (the sign-in takes a
        // moment): install it, and reload the page that went out without it
        let late_cookie = !self.cookie_set && cookie.is_some();
        if let Some(c) = cookie.filter(|_| late_cookie) {
            p.set_cookie(c);
            self.cookie_set = true;
            if !self.nav_pending && !self.url.is_empty() {
                p.browse_reload(false);
            }
        }
        if std::mem::take(&mut self.nav_pending) {
            p.load_uri(&self.url);
            self.nav_started = Some(Instant::now());
            self.load_secs = None;
            self.location = self.url.clone();
            self.loading = true;
            // the old document is going away: nothing is trusted until the new one commits
            self.document = None;
        }
        p.idle();
        for ev in p.take_events() {
            if self.accepts_join
                && let MediaEvent::ClickLinkNoFollow { url, nav_type } = &ev
                && let Some(command) = crate::slapp::join_command(url)
            {
                self.join_requests.push(JoinRequest {
                    command,
                    clicked: nav_type == "clicked",
                    document: self.document.clone(),
                });
                continue;
            }
            if let Some(command) = trusted_folder_command(&ev, self.trusted.as_deref(), self.document.as_deref()) {
                self.folder_commands.push(command);
                continue;
            }
            if let Some(url) = ev.clicked_url() {
                super::context::request(ui.ctx(), clicked_link_action(url));
                continue;
            }
            match ev {
                MediaEvent::NavigateComplete { uri, .. } => {
                    // the first page of a new browser may complete twice
                    if let Some(t) = self.nav_started.take() {
                        self.load_secs = Some(t.elapsed().as_secs_f32());
                    }
                    if !uri.is_empty() {
                        self.document = origin(&uri);
                        self.location = uri;
                    }
                    self.loading = false;
                }
                MediaEvent::NavigateBegin(uri) => {
                    if !uri.is_empty() {
                        // a navigation to another origin: the page that is
                        // leaving no longer speaks for the browser
                        if origin(&uri) != self.document {
                            self.document = None;
                        }
                        self.location = uri;
                    }
                    self.loading = true;
                }
                MediaEvent::LocationChanged(uri) if !uri.is_empty() => {
                    // reported when the navigation commits
                    self.document = origin(&uri);
                    self.location = uri;
                }
                MediaEvent::CursorChanged(c) => self.cursor = c,
                MediaEvent::NameChanged(name) => self.title = name,
                MediaEvent::StatusText(text) => self.status = text,
                MediaEvent::Progress(percent) => self.progress = percent,
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
            // (egui sets it on macOS only)
            command: i.modifiers.mac_cmd,
        });
        let to_px = |pos: egui::Pos2| (((pos.x - rect.left()) * ppp) as i32, ((pos.y - rect.top()) * ppp) as i32);
        // a click in another window of the viewer is a click elsewhere too,
        // and this window's pointer does not show it: the keys typed there
        // would still go to the page
        let window = crate::outside::focused_window(ui.ctx());
        if crate::outside::focus_went_elsewhere(ui.ctx().viewport_id(), self.focus_seen, window) && self.focused {
            self.focused = false;
            p.focus(false);
        }
        self.focus_seen = window;
        // a disabled page (under another window, `ui/signup.rs`) is drawn
        // but gets no pointer; otherwise only the view under the pointer
        // gets it, or the one a press captured until its release
        if self.token == 0 {
            self.token = next_token();
        }
        let pointer = read_web_pointer(ui, &resp, rect, ui.is_enabled(), self.captured, self.token);
        self.captured = pointer.captured;
        if let Some(pos) = pointer.moved {
            self.last_pos = Some(pos);
            let (x, y) = to_px(pos);
            p.mouse_event(MouseEvent::Move, 0, x, y, mods);
            if let Some(clicks) = pointer.wheel {
                p.scroll_event(x, y, 0, clicks, mods);
            }
            if pointer.hover {
                if self.cursor.contains("hand") {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                } else if self.cursor.contains("ibeam") {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
            }
        }
        if let Some(focused) = pointer.focus {
            self.focused = focused;
            p.focus(focused);
        }
        // (a release the viewer never saw goes where the page last saw the
        // pointer)
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()).or(self.last_pos) {
            let (x, y) = to_px(pos);
            if pointer.press {
                self.last_pos = Some(pos);
                p.mouse_event(MouseEvent::Down, 0, x, y, mods);
            }
            if pointer.release {
                p.mouse_event(MouseEvent::Up, 0, x, y, mods);
            }
            if pointer.double_click {
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

/// What this frame's pointer does to the page. `WebView::show` sends it to
/// the plugin; the tests drive the same decision.
#[derive(Debug, Default, PartialEq)]
struct WebPointer {
    /// Where the pointer moved for the page: over this view, or anywhere
    /// while a press on it holds the capture (a text selection, a scrollbar
    /// or a seek bar dragged out of the page keeps following).
    moved: Option<egui::Pos2>,
    /// The pointer is over this view (its cursor shows).
    hover: bool,
    /// Primary press on this view (`MouseEvent::Down`).
    press: bool,
    /// Primary release on this view, or anywhere ending a press on it.
    release: bool,
    /// Double click on this view.
    double_click: bool,
    /// Keyboard focus to take or give up, on a primary press.
    focus: Option<bool>,
    /// Wheel clicks while the pointer is over this view (down positive).
    wheel: Option<i32>,
    /// The page holds the pointer after this frame: from a press on it to
    /// its release.
    captured: bool,
}

/// The page that holds the pointer in an egui context, and the frame it
/// last said so: one capture for all the pages of the window, as the Legacy
/// Viewer's `gFocusMgr.getMouseCapture()` is one for the whole viewer. A
/// holder not drawn since the frame before (closed, hidden) holds nothing.
#[derive(Clone, Copy, Debug)]
struct CaptureOwner {
    token: u64,
    frame: u64,
    /// Still held; false for the frame its release came in, so that a page
    /// drawn after the holder in that frame does not take the release for
    /// its own.
    held: bool,
}

/// A token for a web view's capture, never 0.
fn next_token() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Pointer events for one frame of a web view. `enabled`: the view takes
/// the pointer (a disabled page under another window does not);
/// `captured`: a press on it is still held from an earlier frame; `token`:
/// this view's (`CaptureOwner`).
///
/// Legacy Viewer: the topmost view under the pointer gets the press, and
/// `LLMediaCtrl::handleMouseDown` takes the mouse capture
/// (`gFocusMgr.setMouseCapture`, indra/newview/llmediactrl.cpp): the page
/// gets the moves, the wheel and the release wherever the pointer goes, and
/// no other view gets them, until `handleMouseUp` gives it back. A capture
/// whose release never comes (the focus lost mid-drag, the button let go
/// outside the window or while the view was not drawn) ends at the first
/// frame that sees the button up, with a release for the page, as
/// `LLViewerWindow::handleFocusLost` drops the capture.
fn read_web_pointer(ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, enabled: bool, captured: bool, token: u64) -> WebPointer {
    let owner_id = egui::Id::unique("web-view-capture-owner");
    // (the main viewport's count: a page in a window drawn outside the
    // viewer counts the same frames, outside.rs)
    let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
    let owner = ui.data(|d| d.get_temp::<CaptureOwner>(owner_id));
    let held_elsewhere = owner.is_some_and(|o| o.token != token && if o.held { o.frame + 1 >= frame } else { o.frame == frame });
    let (latest, interact, pressed, released, down, focused) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.interact_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.focused,
        )
    });
    // another page holds the pointer: this one gets none of it (a capture
    // of this one left from before ends with it)
    if held_elsewhere {
        return WebPointer {
            release: captured,
            ..WebPointer::default()
        };
    }
    let hit = |pos: egui::Pos2| enabled && pointer_hits_web_view(ui, resp, rect, pos);
    let hover = latest.filter(|&pos| hit(pos));
    let on_view = interact.is_some_and(hit);
    let press = pressed && on_view;
    // (a press and its release in the same frame: a click)
    let held = captured || press;
    // a capture whose release never came: the button is up, or the window
    // lost the focus (the release of a button held then never arrives)
    let lost = captured && !released && !press && (!down || !focused);
    let release = (released && (held || on_view)) || lost;
    let still = held && !released && !lost;
    if held {
        ui.data_mut(|d| d.insert_temp(owner_id, CaptureOwner { token, frame, held: still }));
    }
    WebPointer {
        moved: if still || (held && released) { latest.or(interact) } else { hover },
        hover: hover.is_some(),
        press,
        release,
        double_click: resp.double_clicked() && on_view,
        focus: (enabled && pressed).then_some(on_view),
        // the holder takes the wheel for the whole hold
        wheel: if still {
            wheel_clicks(ui)
        } else {
            hover.and_then(|_| wheel_clicks(ui))
        },
        captured: still,
    }
}

/// Whether `pos` is on this web view, and this view is the one that gets it.
///
/// Legacy Viewer: only the topmost view under the pointer gets the click.
/// `LLMediaCtrl::handleMouseDown` (indra/newview/llmediactrl.cpp) is reached
/// through `LLView::childrenHandleMouseEvent` (indra/llui/llview.cpp), which
/// walks children front-first and stops at the first one that handles the
/// event or blocks it (a floater does, over its whole rect). The drawn rect and its clip are not enough: the pointer must also
/// be on this response, and this layer must be the topmost one there. A
/// window that only partly covers the page can leave `contains_pointer` set
/// on the page, so the layer check is what refuses that click.
fn pointer_hits_web_view(ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, pos: egui::Pos2) -> bool {
    rect.intersect(ui.clip_rect()).contains(pos) && resp.contains_pointer() && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
}

/// Wheel clicks for the plugin (down positive, as `LLMediaCtrl::handleScrollWheel`
/// passes them). `None` when this frame did not scroll.
fn wheel_clicks(ui: &egui::Ui) -> Option<i32> {
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
    (clicks != 0.0).then_some((-clicks).round() as i32)
}

/// `scheme://host:port` of a web address.
pub(crate) fn origin(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
}

/// A folder command sent while the top document's origin (`document`) is the
/// trusted one, with or without a click: the welcome pack's script wears the
/// body, then adds the clothing a moment later, without a gesture. A frame
/// inside the trusted document is not told apart from it (still stricter than
/// the Legacy Viewer, which trusts the whole browser).
fn trusted_folder_command(event: &MediaEvent, trusted: Option<&str>, document: Option<&str>) -> Option<crate::slapp::FolderCommand> {
    let MediaEvent::ClickLinkNoFollow { url, .. } = event else {
        return None;
    };
    let trusted = trusted?;
    let command = crate::slapp::folder_command(url)?;
    (document == Some(trusted)).then_some(command)
}

/// Follow the same SLURL and app-link rules as a clicked link in chat
/// (also used for a link clicked in a page shown on a prim, `media/update.rs`).
pub(crate) fn clicked_link_action(url: &str) -> super::context::CtxAction {
    use super::context::CtxAction;
    if let Some(place) = crate::slurl::place_link(url) {
        let location = place.location;
        if place.teleport {
            CtxAction::TeleportToPlace(location.target(), location.pos)
        } else {
            CtxAction::ShowPlaceInfo(location.target(), location.pos)
        }
    } else if let Some(link) = crate::slapp::parse(url) {
        CtxAction::AppLink(link)
    } else {
        CtxAction::OpenUrl(url.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_scheme_links_require_a_click_gesture() {
        assert_eq!(
            MediaEvent::clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: "secondlife:///app/teleport/Aurora/12/34/56".into(),
                nav_type: "clicked".into(),
            }),
            Some("secondlife:///app/teleport/Aurora/12/34/56")
        );
        assert_eq!(
            MediaEvent::clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: "secondlife:///app/teleport/Aurora/12/34/56".into(),
                nav_type: "navigated".into(),
            }),
            None
        );
        assert_eq!(
            MediaEvent::clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: String::new(),
                nav_type: "clicked".into(),
            }),
            None
        );
    }

    #[test]
    fn only_the_trusted_page_sends_folder_commands() {
        let wear = |nav: &str| MediaEvent::ClickLinkNoFollow {
            url: "secondlife://app/wear_folder/?folder_name=Female%20body%20-%20A".into(),
            nav_type: nav.into(),
        };
        let page = "https://pages.example/v3.0/agni/vawp/index.html";
        let trusted = origin(page);
        let trusted = trusted.as_deref();
        let doc = |url: &str| origin(url);
        // clicked or run by the page's script, from the trusted site
        for nav in ["clicked", "navigated"] {
            let command = trusted_folder_command(&wear(nav), trusted, doc(page).as_deref()).expect("trusted page");
            assert_eq!(command.name.as_deref(), Some("Female body - A"));
        }
        // another site, or an untrusted browser: nothing worn
        assert_eq!(
            trusted_folder_command(&wear("clicked"), trusted, doc("https://other.example/").as_deref()),
            None
        );
        assert_eq!(
            trusted_folder_command(&wear("clicked"), trusted, doc("http://pages.example/").as_deref()),
            None
        );
        assert_eq!(trusted_folder_command(&wear("clicked"), None, doc(page).as_deref()), None);
        // no committed document (navigation under way): nothing is trusted
        assert_eq!(trusted_folder_command(&wear("navigated"), trusted, None), None);
        // any other link takes the usual path
        let teleport = MediaEvent::ClickLinkNoFollow {
            url: "secondlife:///app/teleport/Aurora/12/34/56".into(),
            nav_type: "clicked".into(),
        };
        assert_eq!(trusted_folder_command(&teleport, trusted, doc(page).as_deref()), None);
        assert_eq!(origin("file:///c:/page.html"), None);
    }

    #[test]
    fn clicked_teleport_slurl_uses_the_viewers_teleport_flow() {
        let action = clicked_link_action("secondlife:///app/teleport/Aurora%20Plaza/12/34/56");
        assert!(matches!(action, super::super::context::CtxAction::TeleportToPlace(region, pos)
            if region == "Aurora Plaza" && pos == glam::Vec3::new(12.0, 34.0, 56.0)));
    }

    #[test]
    fn clicked_map_slurl_opens_place_details() {
        let action = clicked_link_action("https://maps.secondlife.com/secondlife/Aurora%20Plaza/12/34/56");
        assert!(matches!(action, super::super::context::CtxAction::ShowPlaceInfo(region, pos)
            if region == "Aurora Plaza" && pos == glam::Vec3::new(12.0, 34.0, 56.0)));
    }

    #[test]
    fn clicked_http_links_keep_the_external_link_policy() {
        let action = clicked_link_action("https://example.org/");
        assert!(matches!(action, super::super::context::CtxAction::OpenUrl(url) if url == "https://example.org/"));
    }

    /// One frame of a page with a window over part of it. The page uses the
    /// same `read_web_pointer` that `WebView::show` sends to the plugin.
    struct PageFrame {
        page: egui::Rect,
        cover: egui::Rect,
        press: bool,
        release: bool,
        focus: Option<bool>,
        wheel: Option<i32>,
        /// Where the page saw the pointer move this frame.
        moved: Option<egui::Pos2>,
        contains_pointer: bool,
        layer_ok: bool,
    }

    fn show_page_and_cover(ui: &mut egui::Ui, cover_pos: egui::Pos2) -> PageFrame {
        let mut frame = PageFrame {
            page: egui::Rect::NOTHING,
            cover: egui::Rect::NOTHING,
            press: false,
            release: false,
            focus: None,
            wheel: None,
            moved: None,
            contains_pointer: false,
            layer_ok: false,
        };
        let page = egui::Window::new("Chat")
            .id(egui::Id::unique("covered-web-view-page"))
            .fixed_pos(egui::pos2(24.0, 24.0))
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(360.0, 220.0), egui::Sense::click_and_drag());
                // the page's capture from one frame to the next, as `WebView` keeps it
                let capture = egui::Id::unique("covered-web-view-capture");
                let captured = ui.data(|d| d.get_temp::<bool>(capture).unwrap_or(false));
                let pointer = read_web_pointer(ui, &resp, rect, true, captured, 1);
                ui.data_mut(|d| d.insert_temp(capture, pointer.captured));
                let (contains_pointer, layer_ok) = ui.input(|i| i.pointer.interact_pos()).map_or((false, false), |pos| {
                    (resp.contains_pointer(), ui.ctx().layer_id_at(pos) == Some(ui.layer_id()))
                });
                (rect, pointer, contains_pointer, layer_ok)
            });
        if let Some((rect, pointer, contains_pointer, layer_ok)) = page.and_then(|page| page.inner) {
            frame.page = rect;
            frame.press = pointer.press;
            frame.release = pointer.release;
            frame.focus = pointer.focus;
            frame.wheel = pointer.wheel;
            frame.moved = pointer.moved;
            frame.contains_pointer = contains_pointer;
            frame.layer_ok = layer_ok;
        }
        if let Some(cover) = egui::Window::new("Inventory")
            .id(egui::Id::unique("covered-web-view-window"))
            .fixed_pos(cover_pos)
            .fixed_size(egui::vec2(180.0, 120.0))
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label("Inventory");
            })
        {
            frame.cover = cover.response.rect;
        }
        frame
    }

    fn page_frame(frames: &mut crate::ui::widgets::elide_tests::Frames, events: Vec<egui::Event>, cover_pos: egui::Pos2) -> PageFrame {
        let mut shown = None;
        frames.run(events, |ui| {
            shown = Some(show_page_and_cover(ui, cover_pos));
        });
        shown.expect("the frame ran")
    }

    fn wheel_event() -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// A point of `page` that `cover` does not contain.
    fn uncovered_point(page: egui::Rect, cover: egui::Rect) -> Option<egui::Pos2> {
        let xs = [page.left() + 8.0, page.center().x, page.right() - 8.0];
        let ys = [page.top() + 8.0, page.center().y, page.bottom() - 8.0];
        ys.into_iter()
            .flat_map(|y| xs.map(|x| egui::pos2(x, y)))
            .find(|pos| page.contains(*pos) && !cover.contains(*pos))
    }

    #[test]
    fn covered_web_view_ignores_a_click_on_the_window_above() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        let mut cover_pos = egui::pos2(900.0, 640.0);
        let mut laid = page_frame(&mut frames, Vec::new(), cover_pos);
        assert!(laid.page.is_positive(), "the web view is drawn, got {:?}", laid.page);
        // Lay Inventory over the middle of the page and let that rect settle.
        cover_pos = laid.page.center() - egui::vec2(30.0, 24.0);
        for _ in 0..3 {
            laid = page_frame(&mut frames, Vec::new(), cover_pos);
        }
        let covered = laid.page.center();
        assert!(
            laid.page.contains(covered) && laid.cover.contains(covered),
            "Inventory must cover the page: page {:?} cover {:?} at {covered:?}",
            laid.page,
            laid.cover
        );
        let Some(open) = uncovered_point(laid.page, laid.cover) else {
            panic!(
                "the page must keep a point Inventory does not cover: page {:?} cover {:?}",
                laid.page, laid.cover
            );
        };

        page_frame(&mut frames, vec![egui::Event::PointerMoved(covered)], cover_pos);
        let press = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(covered), pointer_button(covered, true)],
            cover_pos,
        );
        let release = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(covered), pointer_button(covered, false)],
            cover_pos,
        );
        assert!(
            !press.press && !release.release && press.focus != Some(true),
            "a click on the window over the web view sent Down={down} Up={up} focus={focus:?} (contains_pointer={contains_pointer} layer_ok={layer_ok} page {page:?} cover {cover:?})",
            down = press.press,
            up = release.release,
            focus = press.focus,
            contains_pointer = press.contains_pointer,
            layer_ok = press.layer_ok,
            page = press.page,
            cover = press.cover,
        );
        let wheeled = page_frame(&mut frames, vec![egui::Event::PointerMoved(covered), wheel_event()], cover_pos);
        assert!(
            wheeled.wheel.is_none(),
            "a wheel over the covering window scrolled the page by {:?}",
            wheeled.wheel
        );

        page_frame(&mut frames, vec![egui::Event::PointerMoved(open)], cover_pos);
        let press = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(open), pointer_button(open, true)],
            cover_pos,
        );
        let release = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(open), pointer_button(open, false)],
            cover_pos,
        );
        assert!(
            press.press && release.release && press.focus == Some(true),
            "a click on the uncovered web view must send Down and Up and take focus, got Down={down} Up={up} focus={focus:?} (contains_pointer={contains_pointer} layer_ok={layer_ok})",
            down = press.press,
            up = release.release,
            focus = press.focus,
            contains_pointer = press.contains_pointer,
            layer_ok = press.layer_ok,
        );
        page_frame(&mut frames, vec![egui::Event::PointerMoved(open)], cover_pos);
        let wheeled = page_frame(&mut frames, vec![egui::Event::PointerMoved(open), wheel_event()], cover_pos);
        assert_eq!(
            wheeled.wheel,
            Some(-1),
            "a wheel on the uncovered web view must reach the page (contains_pointer={contains_pointer} layer_ok={layer_ok})",
            contains_pointer = wheeled.contains_pointer,
            layer_ok = wheeled.layer_ok,
        );
    }

    /// One frame of a page whose lower half is clipped away when `clip`
    /// (scrolled out of view): the point in that half, and whether the page
    /// gets it.
    fn clipped_page_hit(frames: &mut crate::ui::widgets::elide_tests::Frames, events: Vec<egui::Event>, clip: bool) -> (egui::Pos2, bool) {
        let mut out = (egui::Pos2::ZERO, false);
        frames.run(events, |ui| {
            egui::Window::new("Chat")
                .id(egui::Id::unique("clipped-web-view"))
                .fixed_pos(egui::pos2(24.0, 24.0))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(360.0, 220.0), egui::Sense::click_and_drag());
                    let below = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.25);
                    if clip {
                        ui.set_clip_rect(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.center().y)));
                    }
                    out = (below, pointer_hits_web_view(ui, &resp, rect, below));
                });
        });
        out
    }

    #[test]
    fn covered_web_view_ignores_a_click_outside_its_clip() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        let (below, _) = clipped_page_hit(&mut frames, Vec::new(), false);
        clipped_page_hit(&mut frames, vec![egui::Event::PointerMoved(below)], false);
        // the pointer on that point: the page gets it while it is in view…
        let (_, open) = clipped_page_hit(&mut frames, vec![egui::Event::PointerMoved(below)], false);
        assert!(open, "the point at {below:?} has to be on the page while nothing clips it");
        // …and not once the clip hides it
        let (_, clipped) = clipped_page_hit(&mut frames, vec![egui::Event::PointerMoved(below)], true);
        assert!(!clipped, "a click in the clipped-away part of a web view must not reach the page");
    }

    /// One frame of two pages side by side, `a` and `b`, each a `WebView`'s
    /// pointer decision with its own token and capture, and optionally a
    /// window kept in front of everything (`Order::Foreground`) over `a`.
    struct TwoPages {
        a: egui::Rect,
        b: egui::Rect,
        a_pointer: WebPointer,
        b_pointer: WebPointer,
    }

    fn two_pages(frames: &mut crate::ui::widgets::elide_tests::Frames, events: Vec<egui::Event>, cover: Option<egui::Rect>) -> TwoPages {
        let mut out = TwoPages {
            a: egui::Rect::NOTHING,
            b: egui::Rect::NOTHING,
            a_pointer: WebPointer::default(),
            b_pointer: WebPointer::default(),
        };
        frames.run(events, |ui| {
            for (name, pos, token) in [
                ("Page A", egui::pos2(24.0, 24.0), 11u64),
                ("Page B", egui::pos2(460.0, 24.0), 12u64),
            ] {
                let shown = egui::Window::new(name)
                    .id(egui::Id::unique(name))
                    .fixed_pos(pos)
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(360.0, 220.0), egui::Sense::click_and_drag());
                        let key = egui::Id::unique(("two-pages-capture", token));
                        let captured = ui.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
                        let pointer = read_web_pointer(ui, &resp, rect, true, captured, token);
                        ui.data_mut(|d| d.insert_temp(key, pointer.captured));
                        (rect, pointer)
                    });
                if let Some((rect, pointer)) = shown.and_then(|w| w.inner) {
                    if token == 11 {
                        (out.a, out.a_pointer) = (rect, pointer);
                    } else {
                        (out.b, out.b_pointer) = (rect, pointer);
                    }
                }
            }
            if let Some(cover) = cover {
                egui::Area::new(egui::Id::unique("two-pages-cover"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(cover.min)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(cover.size(), egui::Sense::click());
                    });
            }
        });
        out
    }

    /// The drag crosses a window that stays in front of the page (the press
    /// raises the page's own window, so a floater over it would no longer
    /// cover it): the page still follows, as with the Legacy Viewer's capture.
    #[test]
    fn a_drag_over_a_window_in_front_keeps_reaching_the_page() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        let laid = two_pages(&mut frames, Vec::new(), None);
        let cover = egui::Rect::from_center_size(laid.a.center(), egui::vec2(120.0, 80.0));
        two_pages(&mut frames, Vec::new(), Some(cover));
        let laid = two_pages(&mut frames, Vec::new(), Some(cover));
        let open = egui::pos2(laid.a.left() + 12.0, laid.a.top() + 12.0);
        let covered = laid.a.center();
        assert!(!cover.contains(open) && cover.contains(covered));
        // over the cover, before any press: not the page's
        let before = two_pages(&mut frames, vec![egui::Event::PointerMoved(covered)], Some(cover));
        assert_eq!(before.a_pointer.moved, None, "the window in front keeps the pointer");
        two_pages(&mut frames, vec![egui::Event::PointerMoved(open)], Some(cover));
        let press = two_pages(
            &mut frames,
            vec![egui::Event::PointerMoved(open), pointer_button(open, true)],
            Some(cover),
        );
        assert!(press.a_pointer.press);
        let over = two_pages(&mut frames, vec![egui::Event::PointerMoved(covered)], Some(cover));
        assert_eq!(
            over.a_pointer.moved,
            Some(covered),
            "the drag keeps moving the page under the window in front"
        );
        assert!(!over.a_pointer.hover, "(it is the capture, not a hover)");
        let release = two_pages(
            &mut frames,
            vec![egui::Event::PointerMoved(covered), pointer_button(covered, false)],
            Some(cover),
        );
        assert!(release.a_pointer.release && !release.a_pointer.captured);
    }

    /// A capture whose release never comes (the window lost the focus
    /// mid-drag: the button's release then never arrives) ends at once, with
    /// a release for the page, and the pointer is no longer the page's.
    #[test]
    fn a_capture_whose_release_never_comes_ends() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        two_pages(&mut frames, Vec::new(), None);
        let laid = two_pages(&mut frames, Vec::new(), None);
        let on_a = laid.a.center();
        let away = egui::pos2(laid.a.right() + 40.0, laid.a.bottom() + 120.0);
        two_pages(&mut frames, vec![egui::Event::PointerMoved(on_a)], None);
        let press = two_pages(&mut frames, vec![egui::Event::PointerMoved(on_a), pointer_button(on_a, true)], None);
        assert!(press.a_pointer.press && press.a_pointer.captured);
        frames.focused = false;
        let lost = two_pages(&mut frames, vec![egui::Event::WindowFocused(false)], None);
        assert!(lost.a_pointer.release, "the page gets the release it would never get");
        assert!(!lost.a_pointer.captured);
        frames.focused = true;
        let after = two_pages(
            &mut frames,
            vec![egui::Event::WindowFocused(true), egui::Event::PointerMoved(away)],
            None,
        );
        assert_eq!(after.a_pointer.moved, None, "the pointer is no longer the page's");
    }

    /// While one page holds the pointer, another page gets none of it: no
    /// move, no wheel, no release it never had the press of.
    #[test]
    fn only_the_page_holding_the_pointer_gets_it() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        two_pages(&mut frames, Vec::new(), None);
        let laid = two_pages(&mut frames, Vec::new(), None);
        let (on_a, on_b) = (laid.a.center(), laid.b.center());
        assert!(!laid.a.intersects(laid.b), "the two pages side by side: {:?} {:?}", laid.a, laid.b);
        two_pages(&mut frames, vec![egui::Event::PointerMoved(on_a)], None);
        two_pages(&mut frames, vec![egui::Event::PointerMoved(on_a), pointer_button(on_a, true)], None);
        let over_b = two_pages(&mut frames, vec![egui::Event::PointerMoved(on_b), wheel_event()], None);
        assert_eq!(over_b.a_pointer.moved, Some(on_b), "the holder follows the pointer");
        assert!(over_b.a_pointer.wheel.is_some(), "and takes the wheel");
        assert_eq!(over_b.b_pointer, WebPointer::default(), "the other page gets nothing");
        let release = two_pages(
            &mut frames,
            vec![egui::Event::PointerMoved(on_b), pointer_button(on_b, false)],
            None,
        );
        assert!(release.a_pointer.release, "the release is the holder's");
        assert!(!release.b_pointer.release, "the other page had no press");
        // the hold over: the other page gets its pointer again
        let after = two_pages(&mut frames, vec![egui::Event::PointerMoved(on_b + egui::vec2(4.0, 0.0))], None);
        assert!(after.b_pointer.hover && after.a_pointer.moved.is_none());
    }

    /// A press on the page holds the pointer until its release, as the
    /// Legacy Viewer's mouse capture does: the moves of a drag that crosses
    /// a window over the page or leaves it still reach the page, and so does
    /// the release; after it, a move outside the page is not the page's.
    #[test]
    fn a_drag_from_the_page_keeps_reaching_it_until_released() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        let mut cover_pos = egui::pos2(900.0, 640.0);
        let mut laid = page_frame(&mut frames, Vec::new(), cover_pos);
        cover_pos = laid.page.center() - egui::vec2(30.0, 24.0);
        for _ in 0..3 {
            laid = page_frame(&mut frames, Vec::new(), cover_pos);
        }
        let covered = laid.page.center();
        let open = uncovered_point(laid.page, laid.cover).expect("a point of the page the window leaves");
        let away = egui::pos2(laid.page.right() + 40.0, open.y);
        page_frame(&mut frames, vec![egui::Event::PointerMoved(open)], cover_pos);
        let press = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(open), pointer_button(open, true)],
            cover_pos,
        );
        assert!(press.press, "the press on the page reaches it");
        // the drag crosses the place the other window covered (the press
        // raised the page's window above it, so that is plain hover; a
        // window that stays in front is `a_drag_over_a_window_in_front_…`),
        // then leaves the page
        let over = page_frame(&mut frames, vec![egui::Event::PointerMoved(covered)], cover_pos);
        assert_eq!(over.moved, Some(covered));
        let out = page_frame(&mut frames, vec![egui::Event::PointerMoved(away)], cover_pos);
        assert_eq!(out.moved, Some(away), "a drag out of the page keeps moving it");
        // released outside the page: the page still gets its mouseup
        let release = page_frame(
            &mut frames,
            vec![egui::Event::PointerMoved(away), pointer_button(away, false)],
            cover_pos,
        );
        assert!(release.release, "the release of a drag that left the page reaches it");
        // the capture is over: a move out of the page is no longer the
        // page's (the press raised its window, so the point the other
        // window covered is now on the page)
        let after = page_frame(&mut frames, vec![egui::Event::PointerMoved(away + egui::vec2(8.0, 0.0))], cover_pos);
        assert_eq!(after.moved, None, "after the release, a move out of the page is not the page's");
    }
}
