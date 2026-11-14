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
        self.cookie_set = false;
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
        }
        p.idle();
        for ev in p.take_events() {
            if let Some(url) = clicked_url(&ev) {
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
        // press, release, focus and wheel only when this view is the one under the pointer
        let pointer = read_web_pointer(ui, &resp, rect);
        if let Some(pos) = pointer.hover {
            let (x, y) = to_px(pos);
            p.mouse_event(MouseEvent::Move, 0, x, y, mods);
            if let Some(clicks) = pointer.wheel {
                p.scroll_event(x, y, 0, clicks, mods);
            }
            if self.cursor.contains("hand") {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if self.cursor.contains("ibeam") {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        }
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let (x, y) = to_px(pos);
            if let Some(focused) = pointer.focus {
                self.focused = focused;
                p.focus(focused);
            }
            if pointer.press {
                p.mouse_event(MouseEvent::Down, 0, x, y, mods);
            }
            if pointer.release {
                p.mouse_event(MouseEvent::Up, 0, x, y, mods);
            }
            if resp.double_clicked() && pointer.on_view {
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
struct WebPointer {
    /// Pointer over this view: move, the cursor and the wheel.
    hover: Option<egui::Pos2>,
    /// Primary press on this view (`MouseEvent::Down`).
    press: bool,
    /// Primary release on this view, or the end of a drag that started on it.
    release: bool,
    /// Keyboard focus to take or give up, on a primary press.
    focus: Option<bool>,
    /// The click position is on this view (a double-click too).
    on_view: bool,
    /// Wheel clicks while the pointer is on this view (down positive).
    /// `None` when there is no wheel event, or the pointer is not on this view.
    wheel: Option<i32>,
}

/// Pointer events for one frame of a web view.
fn read_web_pointer(ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect) -> WebPointer {
    let hit = |pos: egui::Pos2| pointer_hits_web_view(ui, resp, rect, pos);
    let hover = resp.hover_pos().filter(|&pos| hit(pos));
    let (interact, pressed, released) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_released()));
    let on_view = interact.is_some_and(hit);
    WebPointer {
        hover,
        press: pressed && on_view,
        release: released && (on_view || resp.dragged()),
        focus: pressed.then_some(on_view),
        on_view,
        wheel: hover.and_then(|_| wheel_clicks(ui)),
    }
}

/// Whether `pos` is on this web view, and this view is the one that gets it.
///
/// Legacy Viewer: only the topmost view under the pointer gets the click.
/// `LLMediaCtrl::handleMouseDown` (indra/newview/llmediactrl.cpp) is reached
/// through `LLView::childrenHandleMouseEvent` (indra/llui/llview.cpp), which
/// walks children front-first and stops at the first one that contains the
/// point. The drawn rect and its clip are not enough: the pointer must also
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

/// Links from the browser are actionable only after a user click. CEF sends
/// custom schemes separately and records whether navigation had a gesture;
/// a redirect or script navigation must never trigger a SLURL action.
fn clicked_url(event: &MediaEvent) -> Option<&str> {
    match event {
        MediaEvent::ClickLinkNoFollow { url, nav_type } if nav_type == "clicked" && !url.is_empty() => Some(url),
        _ => None,
    }
}

/// Follow the same SLURL and app-link rules as a clicked link in chat.
fn clicked_link_action(url: &str) -> super::context::CtxAction {
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
            clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: "secondlife:///app/teleport/Aurora/12/34/56".into(),
                nav_type: "clicked".into(),
            }),
            Some("secondlife:///app/teleport/Aurora/12/34/56")
        );
        assert_eq!(
            clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: "secondlife:///app/teleport/Aurora/12/34/56".into(),
                nav_type: "navigated".into(),
            }),
            None
        );
        assert_eq!(
            clicked_url(&MediaEvent::ClickLinkNoFollow {
                url: String::new(),
                nav_type: "clicked".into(),
            }),
            None
        );
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
            contains_pointer: false,
            layer_ok: false,
        };
        let page = egui::Window::new("Chat")
            .id(egui::Id::unique("covered-web-view-page"))
            .fixed_pos(egui::pos2(24.0, 24.0))
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(360.0, 220.0), egui::Sense::click_and_drag());
                let pointer = read_web_pointer(ui, &resp, rect);
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

    #[test]
    fn covered_web_view_ignores_a_click_outside_its_clip() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        let mut inside_rect = false;
        let mut hit = true;
        frames.run(Vec::new(), |ui| {
            egui::Window::new("Chat")
                .id(egui::Id::unique("clipped-web-view"))
                .fixed_pos(egui::pos2(24.0, 24.0))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(360.0, 220.0), egui::Sense::click_and_drag());
                    let below = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.25);
                    inside_rect = rect.contains(below);
                    // The lower half of the page is scrolled out of view.
                    ui.set_clip_rect(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.center().y)));
                    hit = pointer_hits_web_view(ui, &resp, rect, below);
                });
        });
        assert!(inside_rect, "the point has to sit in the page rect");
        assert!(!hit, "a click in the clipped-away part of a web view must not reach the page");
    }
}
