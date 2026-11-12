//! The Conversation Log window (LLFloaterConversationLog,
//! floater_conversation_log.xml, originally LGPL 2.1): every past
//! conversation of the account, newest first, with its kind and when it
//! was last written to; a filter by name; a click opens its whole
//! transcript in the History window (`history.rs`). The list comes from the
//! transcripts' folder (`world/chat_log/conversations.rs`), read again each
//! time the window opens.

use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::chat_log::conversations::{self, Conversation, Kind};
use egui::{RichText, Sense, Vec2};
use std::path::{Path, PathBuf};

/// The window's id (the toolbar's `conversation_log` command).
pub const WINDOW: &str = "conversation_log";

#[derive(Default)]
pub struct ConversationLogUi {
    /// The list read, or None while it is read.
    list: Option<Vec<Conversation>>,
    reading: Option<crossbeam_channel::Receiver<Vec<Conversation>>>,
    /// The folder the list was read from (read again when it changes).
    dir: Option<PathBuf>,
    filter: String,
    /// The window was shown last frame (the list is read again on opening).
    shown: bool,
}

/// A conversation to open in the History window: its title and file.
pub struct OpenTranscript {
    pub title: String,
    pub file: PathBuf,
}

impl ConversationLogUi {
    /// The window while `open`; `dir` is the account's transcripts' folder
    /// (None logged out or without transcripts).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, dir: Option<&Path>, open: &mut bool) -> Option<OpenTranscript> {
        if !*open {
            self.shown = false;
            self.reading = None;
            return None;
        }
        if !self.shown || self.dir.as_deref() != dir {
            self.shown = true;
            self.dir = dir.map(Path::to_path_buf);
            self.list = None;
            self.reading = dir.map(|d| conversations::list(d.to_path_buf()));
        }
        if let Some(rx) = &self.reading
            && let Ok(list) = rx.try_recv()
        {
            self.list = Some(list);
            self.reading = None;
        }
        if self.reading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut picked = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("conversation-log-title"),
            egui::pos2(300.0, 140.0),
            Vec2::new(420.0, 460.0),
        );
        floater.min_size = Vec2::new(300.0, 220.0);
        floater.show(ctx, p, open, |ui| {
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                widgets::search_box(ui, p, &mut self.filter, aurora_i18n::tr!("conversation-log-filter"), w);
            });
            ui.add_space(6.0);
            let message = |ui: &mut egui::Ui, text: &str| {
                ui.add_space(8.0);
                ui.label(RichText::new(text).size(12.0).color(p.muted));
            };
            if dir.is_none() {
                message(ui, aurora_i18n::tr!("conversation-log-no-transcripts"));
                return;
            }
            let Some(list) = &self.list else {
                message(ui, aurora_i18n::tr!("conversation-log-reading"));
                return;
            };
            let shown = conversations::filtered(list, &self.filter);
            if shown.is_empty() {
                message(ui, aurora_i18n::tr!("conversation-log-empty"));
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let radius = ui.visuals().window_corner_radius;
                for c in shown {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                    }
                    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(15.0));
                    if let Some(t) = crate::ui::icons::global(kind_icon(c.kind)) {
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter().image(t.id(), icon, uv, p.muted);
                    }
                    let date = date_text(c.last);
                    let date_font = egui::FontId::proportional(11.5);
                    let date_w = ui.painter().layout_no_wrap(date.clone(), date_font.clone(), p.muted).size().x;
                    let name = title(c);
                    let left = egui::pos2(rect.left() + 30.0, rect.center().y);
                    let room = (rect.width() - 30.0 - date_w - 16.0).max(20.0);
                    widgets::paint_elided(ui, &name, left, room, egui::FontId::proportional(12.5), p.ink);
                    let right = egui::pos2(rect.right() - 8.0 - date_w, rect.center().y);
                    widgets::paint_elided(ui, &date, right, date_w + 1.0, date_font, p.muted);
                    let tip = aurora_i18n::tr!("conversation-log-open-tip", name = name.clone());
                    if resp.on_hover_text(tip).clicked() {
                        picked = Some(OpenTranscript {
                            title: name,
                            file: c.file.clone(),
                        });
                    }
                }
            });
        });
        picked
    }
}

/// The name a conversation is listed and its history titled with.
fn title(c: &Conversation) -> String {
    match c.kind {
        Kind::Nearby => aurora_i18n::tr!("common-nearby-chat").to_owned(),
        Kind::Group | Kind::Other => c.name.clone(),
    }
}

/// The glyph of a conversation's kind (the Conversations list's).
fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Nearby => "message-filled",
        Kind::Group => "users-group",
        Kind::Other => "user-filled",
    }
}

/// "2026-10-12 14:05", Second Life time, as the other lists write dates.
fn date_text(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if secs <= 0 {
        return String::new();
    }
    let (y, m, d, _, h, mi, _) = crate::ui::land::slt_parts(secs);
    aurora_i18n::tr!(
        "inspect-date",
        year = format!("{y:04}"),
        month = format!("{m:02}"),
        day = format!("{d:02}"),
        hour = format!("{h:02}"),
        minute = format!("{mi:02}")
    )
}
