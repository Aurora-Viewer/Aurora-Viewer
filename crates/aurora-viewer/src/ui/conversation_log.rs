//! The Conversation log window (toolbar `conversation_log`, Communicate ›
//! Conversation log…): the conversations of the account, newest first,
//! with a filter, the actions of the selected one and its view options.
//! Port of LLFloaterConversationLog, LLConversationLogList and
//! LLConversationLogListItem (indra/newview/llfloaterconversationlog.cpp,
//! llconversationloglist.cpp, llconversationloglistitem.cpp;
//! floater_conversation_log.xml, menu_conversation_log_gear.xml,
//! menu_conversation_log_view.xml, panel_conversation_log_list_item.xml;
//! originally LGPL 2.1). The log itself is `world/chat_log/conversations.rs`.
//!
//! A row shows the unread offline mark, the picture, the name and the time,
//! and its cross removes it; the cross shows on the row under the pointer
//! or selected instead of on every row (the theme's quick actions), the
//! rest as in the Legacy Viewer. A double click opens the conversation; the
//! right click and the gear button give the actions of the selection.

use super::context::{self, CtxAction};
use super::menu;
use super::widgets::{self, Floater};
use crate::settings::ConversationLogView;
use crate::theme::Palette;
use crate::world::World;
use crate::world::chat_log::conversations::{self, Conversation, Kind, Sort, View};
use egui::{Rect, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The window's id (Floater.Toggle "conversation").
pub const WINDOW: &str = "conversation_log";
const ROW_H: f32 = 26.0;
const PICTURE: f32 = 20.0;
const TOOL: f32 = 26.0;

/// What the window asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// "Open chat transcript…": the History window on its file.
    Transcript(Conversation),
    /// "Open chat transcript externally…": the file in the system's viewer.
    TranscriptExternally(Conversation),
    /// The row's cross (onRemoveBtnClicked).
    Remove { session: Uuid, time: i64 },
}

/// What the order below was made from: it is made again only when one of
/// these changes, not every frame.
#[derive(PartialEq)]
struct Signature {
    revision: u64,
    by_date: bool,
    friends_first: bool,
    show_blocked: bool,
    filter: String,
    /// Friends, blocked residents and the names RLVa hides.
    social: u64,
}

/// The rows shown, in order, with the names they show.
struct Cache {
    signature: Signature,
    order: Vec<usize>,
    names: Vec<String>,
}

#[derive(Default)]
pub struct ConversationLogUi {
    cache: Option<Cache>,
    filter: String,
    /// The selected entry: its session and time.
    selected: Option<(Uuid, i64)>,
    pub wanted_pics: HashSet<Uuid>,
    pub wanted_images: HashSet<Uuid>,
}

/// What the window draws from.
pub struct Inputs<'a> {
    pub world: &'a World,
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
}

impl ConversationLogUi {
    /// The demo's filter, as typed.
    pub fn demo_filter(&mut self, text: &str) {
        self.filter = text.to_owned();
    }

    /// Draw the window while `open`; `view` is its view menu's settings
    /// (true in the second value when one changed).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        input: &Inputs,
        view: &mut ConversationLogView,
        open: &mut bool,
    ) -> (Option<Action>, bool) {
        if !*open {
            return (None, false);
        }
        let mut action = None;
        let mut changed = false;
        let size = Vec2::new(320.0, 260.0);
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-conversation-log"),
            egui::pos2(380.0, 140.0),
            size,
        );
        floater.min_size = Vec2::new(230.0, 120.0);
        floater.show(ctx, p, open, |ui| {
            let world = input.world;
            let log = &world.chat_log.conversations;
            let signature = Signature {
                revision: log.revision,
                by_date: view.by_date,
                friends_first: view.friends_first,
                show_blocked: view.show_blocked,
                filter: self.filter.clone(),
                social: social_signature(world, &log.list),
            };
            if self.cache.as_ref().is_none_or(|c| c.signature != signature) {
                self.cache = Some(Self::sorted(world, signature));
            }
            let Some(cache) = self.cache.take() else { return };
            let (order, names) = (&cache.order, &cache.names);
            // the selection stays on its entry while it is shown
            if let Some((session, time)) = self.selected
                && !order.iter().any(|&i| log.list[i].session == session && log.list[i].time == time)
            {
                self.selected = None;
            }
            let selected = self
                .selected
                .and_then(|(session, time)| log.list.iter().find(|c| c.session == session && c.time == time).cloned());
            // laid out from the right: the filter takes what the buttons
            // leave, so the row never asks for more than the window has
            let line = Vec2::new(ui.available_width(), TOOL);
            ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let sort = ui
                    .add(tool_button(p, "arrows-sort", true))
                    .on_hover_text(aurora_i18n::tr!("people-view-sort-options"));
                menu::dropdown(&sort, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    changed |= view_menu(ui, p, view);
                });
                // the gear: the selection's actions, enabled with one
                let tip = aurora_i18n::tr!("conversation-log-actions");
                match &selected {
                    Some(c) => {
                        let gear = ui.add(tool_button(p, "settings-filled", true)).on_hover_text(tip);
                        menu::dropdown(&gear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                            actions_menu(ui, p, world, c, &mut action);
                        });
                    }
                    None => {
                        ui.add_enabled(false, tool_button(p, "settings-filled", false))
                            .on_disabled_hover_text(tip);
                    }
                }
                let width = ui.available_width().max(80.0);
                let search = widgets::clearable_search_box(ui, p, &mut self.filter, aurora_i18n::tr!("conversation-log-filter"), width);
                // Ctrl+F in the window (FSCommon::isFilterEditorKeyCombo)
                if super::find_shortcut::take(ui, &[search.id]) {
                    search.request_focus();
                }
            });
            ui.add_space(4.0);
            if order.is_empty() {
                let empty = (log.logging, log.list.is_empty());
                self.cache = Some(cache);
                let text = match empty {
                    (false, true) => Some(aurora_i18n::tr!("conversation-log-disabled-empty")),
                    (false, false) => Some(aurora_i18n::tr!("conversation-log-disabled-not-empty")),
                    (true, true) => Some(aurora_i18n::tr!("conversation-log-enabled-empty")),
                    (true, false) => None,
                };
                if let Some(text) = text {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(p.muted)).wrap());
                }
                return;
            }
            let filter = self.filter.trim_start().to_string();
            egui::ScrollArea::vertical()
                .id_salt("conversation_log_rows")
                .auto_shrink([false, false])
                .show_rows(ui, ROW_H, order.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for &i in &order[range] {
                        let c = &log.list[i];
                        self.row(ui, p, input, c, &names[i], &filter, &mut action);
                    }
                });
            self.cache = Some(cache);
        });
        (action, changed)
    }

    /// The order of the rows and the names they show, for the log as it is.
    fn sorted(world: &World, signature: Signature) -> Cache {
        let list = &world.chat_log.conversations.list;
        let names: Vec<String> = list.iter().map(|c| world.conversation_display_name(c)).collect();
        let friends: HashSet<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
        let blocked: HashSet<Uuid> = list
            .iter()
            .map(|c| c.participant)
            .filter(|id| !id.is_nil() && world.is_avatar_blocked(id))
            .collect();
        let order = conversations::shown(
            list,
            &names,
            View {
                sort: if signature.by_date { Sort::Date } else { Sort::Name },
                friends_first: signature.friends_first,
                show_blocked: signature.show_blocked,
                filter: &signature.filter,
            },
            &friends,
            &blocked,
        );
        Cache { signature, order, names }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        input: &Inputs,
        c: &Conversation,
        name: &str,
        filter: &str,
        action: &mut Option<Action>,
    ) {
        let world = input.world;
        let key = (c.session, c.time);
        // the ids follow the entry, not its place in the list: a sort while
        // a menu is open keeps the menu on its resident
        // (people/rows.rs::allocate)
        let id = ui.scope_id().with(("conversation-log-row", key));
        let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), ROW_H));
        let resp = ui.interact(rect, id, Sense::click());
        let selected = self.selected == Some(key);
        let hot = ui.rect_contains_pointer(rect);
        if selected || hot {
            let fill = if selected {
                p.violet.gamma_multiply(0.18)
            } else {
                p.raised.gamma_multiply(0.45)
            };
            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
        }
        let mut x = rect.left() + 4.0;
        // unread_ims_icon: messages that came while we were logged out
        let mark = Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), Vec2::splat(15.0));
        if c.offline {
            if let Some(t) = super::icons::global("inbox") {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), mark, uv, p.violet);
            }
            if ui.rect_contains_pointer(mark) {
                resp.clone().on_hover_text(aurora_i18n::tr!("conversation-log-offline"));
            }
        }
        x += 20.0;
        let pic = Rect::from_min_size(egui::pos2(x, rect.center().y - PICTURE * 0.5), Vec2::splat(PICTURE));
        match c.kind {
            Kind::Group => {
                let insignia = super::chat::sidebar::group_insignia(world, &c.session);
                if let Some(i) = insignia {
                    self.wanted_images.insert(i);
                }
                super::chat::sidebar::paint_group_picture(ui, p, pic, insignia.and_then(|i| input.images.get(&i)), 12.0);
            }
            _ => {
                if !c.participant.is_nil() && ui.is_rect_visible(rect) {
                    self.wanted_pics.insert(c.participant);
                }
                widgets::avatar_disc(ui, p, pic, input.pics.get(&c.participant), name, false);
            }
        }
        x = pic.right() + 6.0;
        // the cross of the row under the pointer or selected
        let cross = Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), Vec2::splat(18.0));
        let mut over_cross = false;
        if hot || selected {
            let hit = ui.interact(cross, id.with("remove"), Sense::click());
            over_cross = hit.hovered();
            if let Some(t) = super::icons::global("x") {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let glyph = Rect::from_center_size(cross.center(), Vec2::splat(12.0));
                ui.painter().image(t.id(), glyph, uv, if over_cross { p.ink } else { p.muted });
            }
            if hit.on_hover_text(aurora_i18n::tr!("conversation-log-remove")).clicked() {
                *action = Some(Action::Remove {
                    session: c.session,
                    time: c.time,
                });
            }
        }
        let font = egui::FontId::proportional(12.0);
        let stamp = conversations::timestamp(c.time);
        let stamp_job = highlighted(&stamp, filter, font.clone(), p.muted, p.violet, f32::INFINITY);
        let stamp_galley = ui.fonts_mut(|f| f.layout_job(stamp_job));
        let stamp_left = cross.left() - 4.0 - stamp_galley.size().x;
        let name_width = (stamp_left - 8.0 - x).max(0.0);
        if stamp_left > x + 40.0 {
            ui.painter().galley(
                egui::pos2(stamp_left, rect.center().y - stamp_galley.size().y * 0.5),
                stamp_galley,
                p.muted,
            );
        }
        let name_color = crate::ui::colors::resident_name(world, &c.participant, p.ink);
        let name_job = highlighted(name, filter, egui::FontId::proportional(12.5), name_color, p.violet, name_width);
        let name_galley = ui.fonts_mut(|f| f.layout_job(name_job));
        let cut = name_galley.elided;
        ui.painter()
            .galley(egui::pos2(x, rect.center().y - name_galley.size().y * 0.5), name_galley, name_color);
        if cut && !over_cross {
            resp.clone().on_hover_text(name);
        }
        if (resp.clicked() || resp.secondary_clicked()) && !over_cross {
            self.selected = Some(key);
        }
        // onDoubleClick: the conversation opens
        if resp.double_clicked() && !over_cross {
            let ctx = ui.ctx().clone();
            match c.kind {
                Kind::Private => context::request(&ctx, CtxAction::Im(c.participant)),
                Kind::Group => context::request(&ctx, CtxAction::GroupChat(c.session)),
                _ => {}
            }
        }
        menu::context_menu(&resp, p, |ui| actions_menu(ui, p, world, c, action));
    }
}

/// What the cached order depends on besides the log: who is a friend, who
/// is blocked and whose name RLVa hides.
fn social_signature(world: &World, list: &[Conversation]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    world.social.friends.iter().for_each(|f| f.id.hash(&mut hash));
    for c in list {
        if !c.participant.is_nil() {
            (world.is_avatar_blocked(&c.participant), world.rlva_can_name(c.participant)).hash(&mut hash);
        }
    }
    hash.finish()
}

/// A toolbar button of the window (People's, with its menu).
fn tool_button<'a>(p: &Palette, icon: &str, enabled: bool) -> egui::Button<'a> {
    let tint = if enabled { p.ink } else { p.muted_dim };
    let button = match super::icons::global(icon) {
        Some(texture) => {
            egui::Button::image(egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), Vec2::splat(14.0))).tint(tint))
        }
        None => egui::Button::new(""),
    };
    button.min_size(Vec2::splat(TOOL))
}

/// One line of `text` in `color`, cut with an ellipsis at `width`, the
/// first match of `filter` (any case) in `accent`
/// (LLConversationLogListItem::highlightNameDate).
fn highlighted(
    text: &str,
    filter: &str,
    font: egui::FontId,
    color: egui::Color32,
    accent: egui::Color32,
    width: f32,
) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let plain = TextFormat::simple(font.clone(), color);
    let lower = text.to_lowercase();
    let needle = filter.to_lowercase();
    // only where the lower case keeps the byte positions of the text
    let found = (!needle.is_empty() && lower.len() == text.len())
        .then(|| lower.find(&needle))
        .flatten()
        .filter(|&at| text.is_char_boundary(at) && text.is_char_boundary(at + needle.len()));
    match found {
        Some(at) => {
            job.append(&text[..at], 0.0, plain.clone());
            job.append(&text[at..at + needle.len()], 0.0, TextFormat::simple(font, accent));
            job.append(&text[at + needle.len()..], 0.0, plain);
        }
        None => job.append(text, 0.0, plain),
    }
    job
}

/// The actions of an entry (menu_conversation_log_gear.xml, enabled as
/// LLConversationLogList::isActionEnabled / isActionChecked).
fn actions_menu(ui: &mut egui::Ui, p: &Palette, world: &World, c: &Conversation, action: &mut Option<Action>) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| context::request(&ctx, a);
    let private = c.kind == Kind::Private;
    let group = c.kind == Kind::Group;
    let member = group && world.groups.is_member(&c.session);
    let id = c.participant;
    let target = if private { id } else { c.session };
    let can_im = (private || member) && world.rlva_can_start_im(target);
    let friend = world.social.friends.iter().find(|f| f.id == id);
    // a friend offline cannot be reached; anyone else may be
    let online = friend.is_none_or(|f| f.online);
    if menu::item_if(ui, p, "message-circle-filled", aurora_i18n::tr!("conversation-log-im"), can_im) {
        act(if private {
            CtxAction::Im(id)
        } else {
            CtxAction::GroupChat(c.session)
        });
    }
    if menu::item_if(
        ui,
        p,
        "phone-filled",
        aurora_i18n::tr!("conversation-log-call"),
        can_im && world.calls.voice_enabled,
    ) {
        act(CtxAction::Call(target));
    }
    if menu::item(ui, p, "history", aurora_i18n::tr!("conversation-log-transcript")) {
        *action = Some(Action::Transcript(c.clone()));
    }
    if menu::item(
        ui,
        p,
        "external-link-filled",
        aurora_i18n::tr!("conversation-log-transcript-externally"),
    ) {
        *action = Some(Action::TranscriptExternally(c.clone()));
    }
    if menu::item_if(ui, p, "user-filled", aurora_i18n::tr!("common-view-profile"), private || group) {
        if private {
            act(CtxAction::Profile(id));
        } else {
            super::group_profile::request_open(ui.ctx(), c.session);
        }
    }
    if menu::item_if(
        ui,
        p,
        "send-filled",
        aurora_i18n::tr!("common-offer-teleport"),
        private && online && world.rlva_can_offer_teleport(id),
    ) {
        act(CtxAction::OfferTeleport(id));
    }
    if menu::item_if(
        ui,
        p,
        "plane-arrival-filled",
        aurora_i18n::tr!("common-request-teleport"),
        private && online,
    ) {
        act(CtxAction::RequestTeleport(id));
    }
    menu::separator(ui, p);
    // add_friend / remove_friend: shown for a resident only
    if private && friend.is_none() && menu::item(ui, p, "user-plus", aurora_i18n::tr!("radar-add-friend")) {
        act(CtxAction::AddFriend(id));
    }
    if private && friend.is_some() && menu::item(ui, p, "user-minus", aurora_i18n::tr!("common-remove-this-friend")) {
        act(CtxAction::RemoveFriend(id));
    }
    if menu::item_if(ui, p, "users-plus", aurora_i18n::tr!("ui-context-invite-to-group"), private) {
        act(CtxAction::InviteToGroup(id));
    }
    menu::separator(ui, p);
    // can_show_on_map: a friend online whose map rights we have
    let mappable = private && friend.is_some_and(|f| f.online) && world.can_locate_avatar(id);
    if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2"), mappable) {
        act(CtxAction::ShowOnMap(id));
    }
    menu::todo(ui, p, "share", aurora_i18n::tr!("common-share"));
    if menu::item_if(ui, p, "coin-filled", aurora_i18n::tr!("common-pay"), private) {
        act(CtxAction::PayResident(id));
    }
    let blocked = private && world.is_avatar_blocked(&id);
    if menu::check(ui, p, "ban", aurora_i18n::tr!("conversation-log-block"), blocked, private) {
        act(CtxAction::ToggleBlock(id));
    }
}

/// The view menu (menu_conversation_log_view.xml, LLFloaterConversationLog::
/// onCustomAction / isActionChecked). True when a setting changed.
fn view_menu(ui: &mut egui::Ui, p: &Palette, view: &mut ConversationLogView) -> bool {
    let mut changed = false;
    if menu::check(
        ui,
        p,
        "sort-ascending-letters",
        aurora_i18n::tr!("conversation-log-sort-name"),
        !view.by_date,
        true,
    ) {
        view.by_date = false;
        changed = true;
    }
    if menu::check(
        ui,
        p,
        "calendar-filled",
        aurora_i18n::tr!("conversation-log-sort-date"),
        view.by_date,
        true,
    ) {
        view.by_date = true;
        changed = true;
    }
    menu::separator(ui, p);
    if menu::check(
        ui,
        p,
        "heart-filled",
        aurora_i18n::tr!("conversation-log-friends-first"),
        view.friends_first,
        true,
    ) {
        view.friends_first = !view.friends_first;
        changed = true;
    }
    if menu::check(
        ui,
        p,
        "ban",
        aurora_i18n::tr!("conversation-log-show-blocked"),
        view.show_blocked,
        true,
    ) {
        view.show_blocked = !view.show_blocked;
        changed = true;
    }
    menu::separator(ui, p);
    // view_nearby_chat_history: the nearby chat's transcript
    if menu::item(ui, p, "message-filled", aurora_i18n::tr!("conversation-log-nearby-transcript")) {
        context::request(ui.ctx(), CtxAction::ShowHistory(None));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(job: &egui::text::LayoutJob) -> Vec<String> {
        job.sections
            .iter()
            .map(|s| job.text[s.byte_range.start.0..s.byte_range.end.0].to_owned())
            .collect()
    }

    #[test]
    fn the_filter_is_highlighted_in_any_case() {
        let font = egui::FontId::proportional(12.0);
        let (ink, accent) = (egui::Color32::GRAY, egui::Color32::BLUE);
        let job = highlighted("Jane Doe", "doe", font.clone(), ink, accent, 100.0);
        assert_eq!(texts(&job), ["Jane ", "Doe", ""]);
        assert_eq!(job.sections[1].format.color, accent);
        let job = highlighted("10/12/2026 14:05", "", font.clone(), ink, accent, 100.0);
        assert_eq!(texts(&job), ["10/12/2026 14:05"]);
        // a text whose lower case moves the bytes is shown plain
        let job = highlighted("İstanbul", "stan", font, ink, accent, 100.0);
        assert_eq!(texts(&job), ["İstanbul"]);
    }
}
