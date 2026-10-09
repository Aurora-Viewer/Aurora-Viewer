//! A web page inside a window, like LLMediaCtrl: a CEF plugin of its own
//! (`aurora-media`), its pixels in an egui texture, mouse, wheel and (once
//! clicked) keyboard sent to the page. Used by the "Flux" tab of the
//! profiles (LLPanelProfileWeb).

use aurora_media::{BrowserSettings, Cookie, MediaEvent, MediaPlugin, Modifiers, MouseEvent, PluginPaths};
use egui::{Color32, Vec2};
use std::time::Instant;

#[derive(Default)]
pub struct WebView {
    plugin: Option<MediaPlugin>,
    texture: Option<egui::TextureHandle>,
    /// Page to show (loaded once the plugin runs).
    url: String,
    nav_pending: bool,
    size: (i32, i32),
    failed: Option<String>,
    /// The page has the keyboard (clicked; a click elsewhere releases it).
    pub focused: bool,
    nav_started: Option<Instant>,
    /// "Heure de chargement : N secondes" (LLPanelProfileWeb::handleMediaEvent).
    pub load_secs: Option<f32>,
    cursor: String,
    scratch: Vec<Color32>,
}

impl WebView {
    pub fn navigate(&mut self, url: &str) {
        if self.url != url {
            self.url = url.to_owned();
            self.nav_pending = true;
        }
    }

    /// Stop the browser (window closed).
    pub fn close(&mut self) {
        self.plugin = None;
        self.texture = None;
        self.focused = false;
    }

    pub fn plugin_mut(&mut self) -> Option<&mut MediaPlugin> {
        self.plugin.as_mut()
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
        if self.plugin.is_none() && self.failed.is_none() && !self.url.is_empty() {
            match paths {
                Some(paths) => match MediaPlugin::launch(paths, "media_plugin_cef", px.0, px.1, browser, "") {
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
                None => self.failed = Some("navigateur introuvable (plugins SLPlugin / CEF)".into()),
            }
        }
        let Some(p) = self.plugin.as_mut() else {
            let text = match &self.failed {
                Some(e) => format!("Page web indisponible : {e}"),
                None => "Chargement…".to_owned(),
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
        }
        p.idle();
        for ev in p.take_events() {
            match ev {
                MediaEvent::NavigateComplete { .. } => {
                    // the first page of a new browser may complete twice
                    if let Some(t) = self.nav_started.take() {
                        self.load_secs = Some(t.elapsed().as_secs_f32());
                    }
                }
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
                Color32::WHITE,
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
