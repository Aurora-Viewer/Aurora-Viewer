//! The History window: the whole transcript of a conversation, by pages of
//! a hundred messages like Firestorm's conversation preview
//! (LLFloaterConversationPreview, indra/newview/llfloaterconversationpreview.cpp),
//! opened on the most recent page. The file is read by its own thread
//! (`chat_log::transcript`) and never held in memory; the search goes
//! through the whole file, not only the page shown.

use super::view::{Source, View};
use super::{LineEnv, Mark, find};
use crate::theme::Palette;
use crate::world::chat_log::transcript::{Index, PAGE, Reply, Spot, Transcript};
use crate::world::transcripts::LineClock;
use crate::world::{ChatLine, World};
use egui::RichText;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

#[derive(Default)]
pub struct HistoryUi {
    pub open: bool,
    /// Name of the conversation.
    title: String,
    file: Option<Transcript>,
    /// None while the file is read through; Some(None): there is no file.
    index: Option<Option<Index>>,
    /// Page shown (asked for), and the one whose lines are loaded.
    page: usize,
    loaded: Option<usize>,
    lines: VecDeque<ChatLine>,
    clock: LineClock,
    view: View,
    /// The text selected in the page's lines (`select.rs`).
    selection: super::select::Selection,
    search: Option<Search>,
    /// Counts the searches: answers to an older one are dropped.
    searches: u64,
}

#[derive(Default)]
struct Search {
    text: String,
    id: u64,
    /// Matches counted so far, and whether the count is complete.
    total: u64,
    counted: bool,
    /// The match shown, its rank among all of them when known.
    current: Option<Spot>,
    rank: Option<u64>,
    /// A step was asked and its answer has not come.
    walking: bool,
    /// The last step found nothing further.
    at_end: bool,
    focus: bool,
}

/// What the window asks the app.
pub enum HistoryAction {
    OpenFolder(PathBuf),
}

fn field_id() -> egui::Id {
    egui::Id::unique("chat_history_find_field")
}

impl HistoryUi {
    /// Show the transcript of a conversation; `path`: its file, when the
    /// account has one for it.
    pub fn show_file(&mut self, title: String, path: Option<PathBuf>) {
        *self = HistoryUi {
            open: true,
            title,
            index: path.is_none().then_some(None),
            file: path.and_then(Transcript::open),
            searches: self.searches,
            ..Default::default()
        };
        if self.file.is_none() {
            self.index = Some(None);
        }
    }

    fn go_to(&mut self, page: usize) {
        self.page = page;
        if self.loaded != Some(page)
            && let Some(f) = &self.file
        {
            f.page(page);
        }
    }

    fn start_search(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        self.searches += 1;
        s.id = self.searches;
        (s.total, s.counted, s.current, s.rank, s.at_end) = (0, false, None, None, false);
        s.walking = false;
        if let Some(f) = self.file.as_ref().filter(|_| self.index.flatten().is_some()) {
            let needle = s.text.to_lowercase();
            f.search(s.id, needle.clone());
            if !needle.is_empty() {
                // the most recent match first
                f.step(s.id, None, false);
                s.walking = true;
            }
        }
    }

    fn step(&mut self, forward: bool) {
        let (Some(s), Some(f)) = (self.search.as_mut(), self.file.as_ref()) else {
            return;
        };
        if s.text.is_empty() || s.walking {
            return;
        }
        f.step(s.id, s.current, forward);
        s.walking = true;
    }

    fn poll(&mut self, world: &World) {
        let Some(file) = &self.file else {
            return;
        };
        let replies: Vec<Reply> = file.replies.try_iter().collect();
        for reply in replies {
            match reply {
                Reply::Indexed(index) => {
                    self.index = Some(index);
                    if let Some(i) = index {
                        // the most recent page
                        self.go_to(i.pages.saturating_sub(1));
                        if self.search.is_some() {
                            self.start_search();
                        }
                    }
                }
                Reply::Page { page, entries } if page == self.page => {
                    self.lines = world.history_lines(entries, false, true, None).into();
                    self.clock = LineClock::default();
                    self.loaded = Some(page);
                    // the lines are another page's: numbered again
                    self.selection.clear();
                    if let Some(spot) = self.search.as_ref().and_then(|s| s.current).filter(|s| s.page == page) {
                        self.view.scroll_to(spot.message);
                    }
                }
                Reply::Page { .. } => {}
                Reply::Counted { id, total, done } => {
                    if let Some(s) = self.search.as_mut().filter(|s| s.id == id) {
                        (s.total, s.counted) = (total, done);
                    }
                }
                Reply::Found { id, spot, rank } => {
                    let Some(s) = self.search.as_mut().filter(|s| s.id == id) else {
                        continue;
                    };
                    s.walking = false;
                    s.at_end = spot.is_none();
                    let Some(spot) = spot else {
                        continue;
                    };
                    (s.current, s.rank) = (Some(spot), rank);
                    if self.loaded == Some(spot.page) && self.page == spot.page {
                        self.view.scroll_to(spot.message);
                    } else {
                        self.go_to(spot.page);
                    }
                }
            }
        }
    }

    /// The window. `env` draws the lines like the conversation window does.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        icons: &super::super::icons::Icons,
        emoji: &mut super::super::emoji::Emoji,
        pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
        world: &World,
        want_names: &mut HashSet<uuid::Uuid>,
    ) -> Option<HistoryAction> {
        if !self.open {
            // the thread and its file handle go with the window
            if self.file.is_some() {
                *self = HistoryUi {
                    searches: self.searches,
                    ..Default::default()
                };
            }
            return None;
        }
        self.poll(world);
        let mut action = None;
        let mut open = true;
        let work = super::super::layout::work_area(ctx);
        let size = egui::vec2(540.0, 460.0);
        let title = aurora_i18n::tr!("ui-history-title", name = self.title.to_string());
        super::super::widgets::Floater::new("chat_history", title, work.center() - size * 0.5, size).show(ctx, p, &mut open, |ui| {
            if find::wants_shortcut(ui, &[field_id()]) {
                match self.search.as_mut() {
                    Some(s) => s.focus = true,
                    None => {
                        self.search = Some(Search {
                            focus: true,
                            ..Default::default()
                        });
                    }
                }
            }
            self.toolbar(ui, p, &mut action);
            if self.search.is_some() {
                ui.add_space(2.0);
                self.find_bar(ui, p);
            }
            ui.add_space(4.0);
            // a little less than what is left: content that overflows makes
            // the resizable window grow
            let h = (ui.available_height() - 6.0).max(80.0);
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(ui.visuals().window_corner_radius)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(h);
                    match self.index {
                        None => {
                            let progress = self.file.as_ref().map_or(0.0, Transcript::progress);
                            ui.add_space(h * 0.4);
                            ui.vertical_centered(|ui| {
                                super::super::widgets::progress(
                                    ui,
                                    p,
                                    "chat_history_reading",
                                    progress,
                                    220.0,
                                    &format!("{} {{}}", aurora_i18n::tr!("ui-history-reading")),
                                    None,
                                );
                            });
                            ui.ctx().request_repaint();
                        }
                        Some(None) => {
                            ui.add_space(h * 0.4);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(aurora_i18n::tr!("ui-history-none")).size(12.5).color(p.muted));
                            });
                        }
                        Some(Some(_)) => self.lines_area(ui, p, icons, emoji, pics, world, want_names, h),
                    }
                });
        });
        self.open = open;
        action
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: &Palette, action: &mut Option<HistoryAction>) {
        use super::super::widgets::icon_button;
        let pages = self.index.flatten().map_or(0, |i| i.pages);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (first, last) = (self.page == 0, self.page + 1 >= pages);
            if icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("ui-history-older"), !first) {
                self.go_to(self.page - 1);
            }
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-history-page",
                    page = (self.page + 1).min(pages.max(1)).to_string(),
                    pages = pages.max(1).to_string()
                ))
                .size(12.0)
                .color(p.ink),
            );
            if icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("ui-history-newer"), !last) {
                self.go_to(self.page + 1);
            }
            if let Some(i) = self.index.flatten() {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-history-messages", count = i.messages))
                        .size(11.5)
                        .color(p.muted),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(dir) = self.file.as_ref().and_then(|f| f.path.parent())
                    && icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("ui-options-chat-logs-open"), true)
                {
                    *action = Some(HistoryAction::OpenFolder(dir.to_path_buf()));
                }
                let searching = self.search.is_some();
                if super::super::widgets::icon_toggle(ui, p, "search-filled", aurora_i18n::tr!("ui-chat-search-conversation"), searching) {
                    self.search = (!searching).then(|| Search {
                        focus: true,
                        ..Default::default()
                    });
                }
            });
        });
    }

    fn find_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        use super::super::widgets::icon_button;
        let mut close = false;
        let mut changed = false;
        let mut step = None;
        let Some(s) = self.search.as_mut() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.set_height(find::BAR_H);
            ui.spacing_mut().item_spacing.x = 4.0;
            let field = ui.add(
                egui::TextEdit::singleline(&mut s.text)
                    .id(field_id())
                    .hint_text(aurora_i18n::tr!("ui-chat-search"))
                    .desired_width((ui.available_width() - 200.0).clamp(80.0, 260.0)),
            );
            if std::mem::take(&mut s.focus) {
                field.request_focus();
            }
            changed = field.changed();
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            let ready = !s.text.is_empty() && !s.walking;
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Enter walks back in time, from the latest match
                step = Some(ui.input(|i| i.modifiers.shift));
                field.request_focus();
            }
            if icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ui-chat-find-previous"), ready) {
                step = Some(false);
            }
            if icon_button(ui, p, "caret-down-filled", aurora_i18n::tr!("ui-chat-find-next"), ready) {
                step = Some(true);
            }
            let (text, color) = if s.text.is_empty() {
                (String::new(), p.muted)
            } else if s.counted && s.total == 0 {
                (aurora_i18n::tr!("ui-chat-find-none").to_owned(), p.warn)
            } else {
                let total = if s.counted {
                    s.total.to_string()
                } else {
                    // still counting
                    format!("{}…", s.total)
                };
                let current = s.rank.map_or_else(|| "–".to_owned(), |r| (r + 1).to_string());
                (aurora_i18n::tr!("ui-chat-find-count", current = current, total = total), p.muted)
            };
            ui.label(RichText::new(text).size(12.0).color(color));
            if s.at_end && !s.text.is_empty() && s.total > 0 {
                ui.label(RichText::new(aurora_i18n::tr!("ui-history-find-end")).size(11.5).color(p.muted_dim));
            }
            if !s.counted && !s.text.is_empty() {
                ui.ctx().request_repaint();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, p, "x", aurora_i18n::tr!("ui-chat-find-close"), true) {
                    close = true;
                }
            });
        });
        if close {
            self.search = None;
        } else if changed {
            self.start_search();
        } else if let Some(forward) = step {
            self.step(forward);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lines_area(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        icons: &super::super::icons::Icons,
        emoji: &mut super::super::emoji::Emoji,
        pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
        world: &World,
        want_names: &mut HashSet<uuid::Uuid>,
        height: f32,
    ) {
        let needle = self.search.as_ref().map(|s| s.text.to_lowercase()).unwrap_or_default();
        let current = self
            .search
            .as_ref()
            .and_then(|s| s.current)
            .filter(|s| self.loaded == Some(s.page))
            .map(|s| s.message);
        // nothing is looked up for the lines of a transcript
        let mut no_pictures = HashSet::new();
        let own_name = world.own_name();
        let menu = ui.layer_id().id.with(("history_line_menu", self.page));
        let mut env = LineEnv {
            p,
            icons,
            emoji,
            pics,
            world,
            want_names,
            wanted: &mut no_pictures,
            own_name: &own_name,
            plain_history: true,
            // the transcripts as they were said
            compact_automated: false,
            compact_notices: false,
            menu: Some(menu),
            bubbles: None,
            previews: None,
        };
        let wash = find::wash(p);
        let (lines, clock, view, selection) = (&self.lines, self.clock, &mut self.view, &mut self.selection);
        // a page opens at its end, the most recent of its messages
        let area = crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::vertical(), ("chat_history", self.page))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height);
        let out = area.show_viewport(ui, |ui, viewport| {
            let width = (ui.available_width() * 2.0).round() / 2.0;
            view.sync(
                &Source {
                    lines,
                    clock,
                    filter: "",
                    find: &needle,
                    layout: super::view::Layout::Classic,
                    first_unread: None,
                },
                width,
            );
            let mark = (!needle.is_empty()).then_some(Mark { needle: &needle, wash });
            let (shift, content) = view.show(ui, viewport, clock.first, &mut |ui, row, painted| {
                let Some(line) = lines.get(row.line) else {
                    return;
                };
                let seq = clock.first + row.line as u64;
                super::line_menu::highlight(ui, menu, seq, line, p, |ui| {
                    super::select::row(ui, seq, selection.of_row(seq), painted, |ui| {
                        super::draw_line(ui, &mut env, line, row.look, mark.filter(|_| row.hit));
                    });
                });
                if painted && current == Some(row.line) {
                    find::current_marker(ui, p);
                }
            });
            let drawn = super::select::take_drawn();
            let copied = super::Copied {
                view,
                lines,
                first: clock.first,
                world,
                own_name: &own_name,
            };
            // (no field here: what is typed goes nowhere)
            super::select_lines(ui, &content, &drawn, selection, &copied);
            shift
        });
        if out.inner != 0.0 {
            let offset = out.state.unclamped_offset() + egui::vec2(0.0, out.inner);
            crate::ui::widgets::scroll_offset::set(ui.ctx(), out.id, offset);
        }
        super::line_menu::show(ui, menu, p, |ui, target| {
            if let Some(avatar) = target.sender {
                super::super::context::chat_sender_menu(ui, p, world, avatar, &target.line);
            }
        });
        debug_assert!(self.lines.len() <= PAGE);
    }
}
