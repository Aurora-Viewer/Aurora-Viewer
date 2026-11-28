//! The History window: the whole transcript of a conversation, by pages of
//! a hundred messages like Firestorm's conversation preview
//! (LLFloaterConversationPreview, indra/newview/llfloaterconversationpreview.cpp),
//! opened on the most recent page. The file is read by its own thread
//! (`chat_log::transcript`) and never held in memory; the search goes
//! through the whole file, not only the page shown.

use super::view::{Source, View};
use super::{LineEnv, Mark, find};
use crate::theme::Palette;
use crate::world::chat_log::transcript::{HistorySource, Index, PAGE, Reply, Spot, Transcript, contains_ci};
use crate::world::transcripts::LineClock;
use crate::world::{ChatLine, World};
use egui::RichText;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// The names a transcript read again shows under RLVa's `@shownames`: those
/// it was written with. Neither LLLogChat::loadChatHistory
/// (indra/newview/lllogchat.cpp) nor
/// LLFloaterConversationPreview::showHistory
/// (indra/newview/llfloaterconversationpreview.cpp) has an RLVa test. What
/// the restriction hid when a line was said is hidden in the file already:
/// the nearby chat's transcript is written as the chat showed it
/// (`World::transcript_text`).
const NAMES: crate::world::names::NameScope = crate::world::names::NameScope::Roster;

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
    /// A direct conversation under the Chat Service: its stitched history,
    /// read once and paged in memory (LLFloaterConversationPreview with
    /// loadStitchedHistory, LL viewer 26.5).
    stitched: Option<Stitched>,
    /// The other resident of a private conversation (`World::private_peer`):
    /// its lines resolve their names when drawn, as in the conversation.
    /// None for the nearby chat, a group or a conference, whose lines keep
    /// the names the file gives.
    peer: Option<uuid::Uuid>,
}

struct Stitched {
    resident: uuid::Uuid,
    /// None while the read runs.
    entries: Option<Vec<crate::world::chat_log::Entry>>,
    /// The transcripts' folder, for its button.
    folder: Option<PathBuf>,
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
    /// Show the transcript of a conversation; `peer`: the other resident of
    /// a private one (`World::private_peer`); `source` resolves and opens
    /// lazily on the `"aurora-transcript"` thread without main-thread I/O.
    pub fn show_source(&mut self, title: String, peer: Option<uuid::Uuid>, source: Option<HistorySource>) {
        *self = HistoryUi {
            open: true,
            title,
            index: source.is_none().then_some(None),
            file: source.and_then(Transcript::open_source),
            searches: self.searches,
            peer,
            ..Default::default()
        };
        if self.file.is_none() {
            self.index = Some(None);
        }
    }

    /// Show the stitched history of a direct conversation, read by the
    /// World (`World::open_direct_preview`); `source`: its transcript source.
    pub fn show_stitched(&mut self, title: String, resident: uuid::Uuid, source: Option<HistorySource>) {
        *self = HistoryUi {
            open: true,
            title,
            searches: self.searches,
            stitched: Some(Stitched {
                resident,
                entries: None,
                folder: source.as_ref().and_then(HistorySource::folder).map(PathBuf::from),
            }),
            peer: Some(resident),
            ..Default::default()
        };
    }

    /// A delete invalidated the stitched history: the window closes.
    pub fn close_stitched(&mut self) {
        if self.stitched.is_some() {
            self.open = false;
        }
    }

    /// The resident whose stitched read the window waits for.
    pub fn stitched_waiting(&self) -> Option<uuid::Uuid> {
        self.stitched
            .as_ref()
            .filter(|s| self.open && s.entries.is_none())
            .map(|s| s.resident)
    }

    /// The stitched read came, oldest first: the most recent page shows.
    pub fn set_stitched(&mut self, entries: Vec<crate::world::chat_log::Entry>, folder: Option<PathBuf>) {
        let Some(s) = self.stitched.as_mut() else {
            return;
        };
        s.folder = s.folder.take().or(folder);
        let messages = entries.len();
        s.entries = Some(entries);
        if messages == 0 {
            self.index = Some(None);
            return;
        }
        let pages = messages.div_ceil(PAGE).max(1);
        self.index = Some(Some(Index {
            pages,
            messages: messages as u64,
            bytes: 0,
        }));
        self.loaded = None;
        self.go_to(pages - 1);
        if self.search.is_some() {
            self.start_search();
        }
    }

    /// The match after (or before) `from` in the stitched history; `None`
    /// starts from the end.
    fn stitched_step(&self, needle: &str, from: Option<Spot>, forward: bool) -> Option<Spot> {
        let entries = self.stitched.as_ref()?.entries.as_ref()?;
        let hit = |e: &crate::world::chat_log::Entry| contains_ci(&e.text, needle) || contains_ci(&e.from, needle);
        let at = from.map(|s| s.page * PAGE + s.message);
        let found = if forward {
            let start = at.map_or(0, |i| i + 1);
            (start..entries.len()).find(|&i| hit(&entries[i]))
        } else {
            let end = at.unwrap_or(entries.len());
            (0..end).rev().find(|&i| hit(&entries[i]))
        }?;
        Some(Spot {
            page: found / PAGE,
            message: found % PAGE,
        })
    }

    /// An import merged these transcripts: a page's place in the file is no
    /// longer the same, so the one shown is read again from the start.
    pub fn imported(&mut self, changed: &std::collections::BTreeSet<String>) {
        if self.open
            && let Some(source) = self
                .file
                .as_ref()
                .filter(|f| f.matches_changed(changed))
                .map(|f| f.source().clone())
        {
            let title = std::mem::take(&mut self.title);
            self.show_source(title, self.peer, Some(source));
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
        if let Some(entries) = self.stitched.as_ref().and_then(|st| st.entries.as_ref()) {
            let needle = s.text.to_lowercase();
            s.total = entries
                .iter()
                .filter(|e| contains_ci(&e.text, &needle) || contains_ci(&e.from, &needle))
                .count() as u64;
            s.counted = true;
            let found = self.stitched_step(&needle, None, false);
            self.stitched_found(found);
            return;
        }
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

    /// A match of the stitched history was found (or none): shown like a
    /// file's `Reply::Found`.
    fn stitched_found(&mut self, spot: Option<Spot>) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.walking = false;
        s.at_end = spot.is_none();
        let Some(spot) = spot else {
            return;
        };
        s.current = Some(spot);
        s.rank = None;
        if self.loaded == Some(spot.page) && self.page == spot.page {
            self.view.scroll_to(spot.message);
        } else {
            self.go_to(spot.page);
        }
    }

    fn step(&mut self, forward: bool) {
        if self.stitched.is_some() {
            let Some(s) = self.search.as_ref().filter(|s| !s.text.is_empty()) else {
                return;
            };
            let found = self.stitched_step(&s.text.to_lowercase(), s.current, forward);
            self.stitched_found(found);
            return;
        }
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
        if let Some(entries) = self.stitched.as_ref().and_then(|s| s.entries.as_ref())
            && self.loaded != Some(self.page)
        {
            let page: Vec<_> = entries.iter().skip(self.page * PAGE).take(PAGE).cloned().collect();
            self.lines = world.history_lines(page, false, true, self.peer).into();
            self.clock = LineClock::default();
            self.loaded = Some(self.page);
            self.selection.clear();
            if let Some(spot) = self.search.as_ref().and_then(|s| s.current).filter(|s| s.page == self.page) {
                self.view.scroll_to(spot.message);
            }
        }
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
                    self.lines = world.history_lines(entries, false, true, self.peer).into();
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
            if self.file.is_some() || self.stitched.is_some() {
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
                if let Some(dir) = self
                    .file
                    .as_ref()
                    .and_then(Transcript::folder)
                    .or_else(|| self.stitched.as_ref().and_then(|s| s.folder.as_deref()))
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
            images: None,
            wanted_images: None,
            share_texture_previews: false,
            world,
            names: NAMES,
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
            moderators: None,
        };
        let wash = find::wash(p);
        let (lines, clock, view, selection) = (&self.lines, self.clock, &mut self.view, &mut self.selection);
        // a page opens at its end, the most recent of its messages
        let area = crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::vertical(), ("chat_history", self.page))
            .stick_to_bottom(true)
            // the view moves the offset before this frame's wheel (`View::show`)
            .animated(false)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height);
        area.show_viewport(ui, |ui, viewport| {
            let width = (ui.available_width() * 2.0).round() / 2.0;
            view.sync(
                &Source {
                    lines,
                    clock,
                    filter: "",
                    find: &needle,
                    layout: super::view::Layout::Classic,
                    first_unread: None,
                    blocked: Default::default(),
                },
                width,
            );
            let mark = (!needle.is_empty()).then_some(Mark { needle: &needle, wash });
            let content = view.show(ui, viewport, clock.first, &mut |ui, row, painted| {
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
                names: NAMES,
                own_name: &own_name,
            };
            // (no field here: what is typed goes nowhere)
            super::select_lines(ui, &content, &drawn, selection, &copied);
        });
        super::line_menu::show(ui, menu, p, |ui, target| {
            if let Some(sender) = target.sender {
                super::sender_menu(ui, p, world, sender, NAMES, &target.line);
            }
        });
        debug_assert!(self.lines.len() <= PAGE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::names::NameScope;

    /// LLFloaterConversationPreview has no RLVa test: a line read back from
    /// a transcript keeps the name and the text it was written with under
    /// `@shownames`, in this window and at the top of the nearby chat
    /// (FSFloaterNearbyChat::loadHistory), where a line said now hides them.
    #[test]
    fn a_transcript_keeps_the_names_it_was_written_with_under_shownames() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let near = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .map(|(_, o)| o.full_id)
            .unwrap();
        world.social.avatar_names.on_legacy(near, "Near", "Tester");
        world.rlva.configure(true);
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Transcript fixture".into(),
            source_id: uuid::Uuid::from_u128(1),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        world.refresh_rlva_privacy();
        assert!(!world.rlva_shows_name(NameScope::World, near));
        // as the file gives a line: a name, and no resident looked up
        let line = ChatLine {
            time: std::time::SystemTime::UNIX_EPOCH,
            from: "Near Tester".into(),
            text: "hello".into(),
            kind: crate::world::ChatKind::Local(aurora_net::ChatType::Normal),
            source: uuid::Uuid::nil(),
            origin: crate::world::Origin::Log,
            blocked: false,
            beyond_range: false,
            speaker: Default::default(),
            private_history: false,
        };
        assert_eq!(NAMES, NameScope::Roster);
        assert_eq!(super::super::sender_name(&world, "Aurora Demo", &line), "Near Tester");
        // the same lines as a private conversation and a group load them
        // (`World::history_lines`), the group member's name cached too
        let me = world.agent_id;
        let name = |id, username: &str, display: &str, first: &str, last: &str| aurora_net::AvatarNameData {
            id,
            username: username.into(),
            display_name: display.into(),
            legacy_first: first.into(),
            legacy_last: last.into(),
            ..Default::default()
        };
        world.social.avatar_names.on_display_names(
            &[
                name(near, "near.tester", "Nova", "Near", "Tester"),
                name(me, "aurora.demo", "Aurie", "Aurora", "Demo"),
            ],
            &[],
            Some(3600),
        );
        let entry = |from: &str| crate::world::chat_log::Entry {
            stamp: String::new(),
            time: None,
            from: from.into(),
            text: "hello".into(),
        };
        let entries = || vec![entry("Near Tester"), entry("Aurora Demo")];
        let private = world.history_lines(entries(), false, false, Some(near));
        let group = world.history_lines(entries(), false, false, None);
        let senders = |world: &World, lines: &[ChatLine]| -> Vec<String> {
            lines.iter().map(|l| super::super::sender_name(world, "Me", l)).collect()
        };
        for lines in [&private, &group] {
            let kinds: Vec<_> = lines.iter().map(|l| (l.kind, l.source)).collect();
            assert_eq!(
                kinds,
                [(crate::world::ChatKind::Im, near), (crate::world::ChatKind::OWN, uuid::Uuid::nil())]
            );
        }
        // a private conversation's history: the cached names, theirs and
        // ours, as its live lines show them (`draw_line`, the rows, follows
        // the same rule)
        assert_eq!(senders(&world, &private), ["Nova", "Aurie"]);
        assert!(private.iter().all(super::super::log_names_resolve));
        // a group member's line keeps the name it was written with, and so
        // does ours there
        assert_eq!(senders(&world, &group), ["Near Tester", "Aurora Demo"]);
        assert!(!group.iter().any(super::super::log_names_resolve));
        assert!(!super::super::log_names_resolve(&line));
        // display names turned off while the conversation is open: both
        // sides follow, and back
        world.social.avatar_names.options.use_display_names = false;
        assert_eq!(senders(&world, &private), ["Near Tester", "Aurora Demo"]);
        world.social.avatar_names.options.use_display_names = true;
        assert_eq!(senders(&world, &private), ["Nova", "Aurie"]);
        // a name not cached yet: as the file gives it, theirs (the lookup is
        // queued) and ours (not the live lines' "Me")
        let mut unknown = private[0].clone();
        unknown.source = uuid::Uuid::from_u128(77);
        assert_eq!(super::super::sender_name(&world, "Me", &unknown), "Near Tester");
        world.agent_id = uuid::Uuid::from_u128(78);
        assert_eq!(super::super::sender_name(&world, "Me", &private[1]), "Aurora Demo");
        world.agent_id = me;
        // said now: the stand-in the line takes when it arrives
        let mut said = line.clone();
        said.origin = crate::world::Origin::Live;
        said.source = near;
        world.rlva_freeze_line(NameScope::World, &mut said);
        let around = super::super::sender_name(&world, "Aurora Demo", &said);
        assert!(!around.contains("Tester"), "{around}");
        // the text: a notice of the viewer is rewritten when it comes, and
        // read back as the file has it
        let mut notice = line.clone();
        notice.kind = crate::world::ChatKind::System;
        notice.text = "Near Tester is online".into();
        for scope in [NAMES, NameScope::World] {
            let wording = notice.wording(scope);
            assert_eq!(world.rlva_filter_text_in(wording, &notice.text), notice.text, "{scope:?}");
            assert_eq!(
                super::super::resolved_text_in(&world, wording, &notice.text),
                notice.text,
                "{scope:?}"
            );
        }
        notice.origin = crate::world::Origin::Live;
        world.rlva_freeze_line(NameScope::World, &mut notice);
        assert!(!notice.text.contains("Tester"), "{}", notice.text);
        // and nothing rewrites it again when it is drawn
        assert!(!notice.wording(NameScope::World).rewritten);
    }

    /// The window on a private conversation resolves its lines' names when
    /// they are drawn, as the conversation does; on a group, a conference
    /// or the nearby chat it keeps the names the file gives
    /// (`World::private_peer`).
    #[test]
    fn the_window_resolves_names_only_for_a_private_conversation() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let group = uuid::Uuid::from_u128(0x4268);
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: uuid::Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        let resident = uuid::Uuid::from_u128(0x4267);
        assert_eq!(world.private_peer(resident), Some(resident));
        assert_eq!(world.private_peer(group), None);
        assert_eq!(world.private_peer(uuid::Uuid::nil()), None);
        // a transcript file: the window keeps what the caller decided
        let mut ui = HistoryUi::default();
        ui.show_source("Group".into(), world.private_peer(group), None);
        assert_eq!(ui.peer, None);
        ui.show_source("Near".into(), world.private_peer(resident), None);
        assert_eq!(ui.peer, Some(resident));
        // a stitched history is always a direct conversation's
        let entry = crate::world::chat_log::Entry {
            stamp: String::new(),
            time: None,
            from: "Near Tester".into(),
            text: "hello".into(),
        };
        let mut ui = HistoryUi::default();
        ui.show_stitched("Near".into(), resident, None);
        ui.set_stitched(vec![entry], None);
        ui.poll(&world);
        assert_eq!(ui.lines.len(), 1);
        assert!(ui.lines.iter().all(|l| l.private_history));
    }
}
