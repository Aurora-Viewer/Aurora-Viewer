//! The Aurora profile window (mock-up A of the profiles, asked for by the
//! human on 2026-10-14, its head redrawn for #1089): a banner drawn from the
//! profile picture, as tall as what it holds (`widgets::hero`), with the
//! picture and its status dot, who it is (names, birth date and age,
//! badges) and what to do (conversation, call, teleport, pay, block, more)
//! centred on one line, the tabs along its bottom; then the Profile tab in
//! cards, a friend's rights on the Info card. Under `COMPACT_W` the header
//! shrinks, the actions keep their icons and the tabs go into a drop-down
//! list.
//!
//! The data, requests and actions are those of the classic window (`super`,
//! Firestorm's LLFloaterProfile), which Preferences › Aurora › Profiles
//! brings back: only the drawing differs. The other tabs are in `tabs`.

mod tabs;

use super::{
    Edit, PictureSlot, ProfileAction, TAB_CLASSIFIEDS, TAB_FEED, TAB_NOTES, TAB_PICKS, TAB_RL, TAB_SL, WebContext, Window, has_unsaved,
    loading, rich, save_all,
};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::presence::{self, Status};
use crate::ui::widgets::{self, ActionKind, Floater, hero};
use crate::ui::{emoji::Emoji, fonts, group_profile, icons, menu};
use crate::world::World;
use crate::world::profiles::{self, Online};
use crate::world::social::{Friend, rights};
use aurora_llsd::llsd_map;
use aurora_net::{AvatarProfile, NetCommand};
use egui::{Color32, FontId, Pos2, Rect, Response, RichText, Sense, Vec2, pos2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Narrower than this, the window takes its compact layout (the mock-up's
/// "~620 px").
const COMPACT_W: f32 = 620.0;
/// The mock-up's window, and the least room the compact one needs.
const DEFAULT_SIZE: Vec2 = Vec2::new(960.0, 760.0);
const MIN_SIZE: Vec2 = Vec2::new(360.0, 440.0);
/// The right column of the Profile tab.
const SIDE_W: f32 = 290.0;
/// Between two cards.
const GAP: f32 = 10.0;
/// What the body adds to the floater's own margin (14 in the mock-up).
const INSET: f32 = 4.0;
/// Height of the header's buttons.
const BUTTON_H: f32 = 28.0;
/// "About" holds at most this many characters (sl_about_text).
const ABOUT_MAX: usize = 65_000;

const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// What the window reads and fills, from `ProfileUi::show`.
pub(super) struct Env<'a, 'w> {
    pub p: &'a Palette,
    pub emoji: &'a mut Emoji,
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub banners: &'a HashMap<Uuid, egui::TextureHandle>,
    pub world: &'a World,
    pub want_names: &'a mut HashSet<Uuid>,
    pub web: &'a WebContext<'w>,
    pub wanted_pics: &'a mut HashSet<Uuid>,
    pub wanted_images: &'a mut HashSet<Uuid>,
    pub actions: &'a mut Vec<ProfileAction>,
    pub pick_picture: &'a mut Option<(PictureSlot, Uuid)>,
    /// `Settings::profile_rights`.
    pub friend_rights: bool,
}

pub(super) fn window_id(id: Uuid) -> String {
    // not the classic window's: each keeps its own size
    format!("profile-card-{id}")
}

/// The banner of a profile picture: the picture shrunk to a few pixels, a
/// little more saturated and slightly see-through, that the linear filter
/// spreads into a soft blur across the header (the mock-up's blurred and
/// enlarged photo, without a blur pass at each frame).
pub fn banner_image(w: usize, h: usize, rgba: &[u8]) -> egui::ColorImage {
    const SIDE: usize = 12;
    let (w, h) = (w.max(1), h.max(1));
    let bw = SIDE;
    let bh = ((SIDE * h) as f32 / w as f32).round().clamp(1.0, SIDE as f32 * 4.0) as usize;
    let mut out = Vec::with_capacity(bw * bh * 4);
    for by in 0..bh {
        for bx in 0..bw {
            let (x0, x1) = (bx * w / bw, ((bx + 1) * w / bw).max(bx * w / bw + 1).min(w));
            let (y0, y1) = (by * h / bh, ((by + 1) * h / bh).max(by * h / bh + 1).min(h));
            let mut sum = [0u64; 3];
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    if let Some(px) = rgba.get(i..i + 3) {
                        for c in 0..3 {
                            sum[c] += px[c] as u64;
                        }
                        n += 1;
                    }
                }
            }
            let avg = sum.map(|s| s as f32 / n.max(1) as f32);
            // saturate(1.2) of the mock-up
            let luma = 0.299 * avg[0] + 0.587 * avg[1] + 0.114 * avg[2];
            for c in avg {
                out.push((luma + (c - luma) * 1.2).round().clamp(0.0, 255.0) as u8);
            }
            out.push(230);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([bw, bh], &out)
}

/// Draw the window of `w`; false once its close button was pressed.
pub(super) fn window(ctx: &egui::Context, env: &mut Env, w: &mut Window, index: usize) -> bool {
    let world = env.world;
    let own = w.id == world.agent_id;
    let title = if own {
        aurora_i18n::tr!("ui-profile-aurora-my-profile").to_owned()
    } else {
        display_name(world, &w.id)
    };
    let screen = ctx.content_rect();
    let pos = screen.center() - DEFAULT_SIZE * 0.5 + Vec2::splat(24.0 * (index % 6) as f32);
    let win_id = window_id(w.id);
    let mut floater = Floater::new(&win_id, title.clone(), pos, DEFAULT_SIZE).help(aurora_i18n::tr!("ui-profile-aurora-help"));
    floater.min_size = MIN_SIZE;
    let mut open = true;
    if floater.show(ctx, env.p, &mut open, |ui| body(ui, env, w)).is_some() && !own {
        super::give_drop(ctx, env.p, &win_id, &title, w.id, env.actions);
    }
    open
}

/// The name in large: the display name, else the name the viewer has.
pub(super) fn display_name(world: &World, id: &Uuid) -> String {
    world
        .social
        .avatar_names
        .get(id)
        .map(|n| n.display_name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| world.social.name_of(id))
}

/// The line under the name: the legacy name and the username, each when it
/// differs from the name above.
fn names_line(world: &World, id: &Uuid, shown: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let from_names = world.social.avatar_names.get(id).and_then(|n| {
        let (first, last) = (n.legacy_first.trim(), n.legacy_last.trim());
        match (first.is_empty(), last.is_empty() || last.eq_ignore_ascii_case("resident")) {
            (true, _) => None,
            (false, true) => Some(first.to_owned()),
            (false, false) => Some(format!("{first} {last}")),
        }
    });
    if let Some(legacy) = from_names.or_else(|| world.legacy_name(id)).filter(|n| n != shown) {
        parts.push(legacy);
    }
    if let Some(user) = world
        .social
        .avatar_names
        .get(id)
        .map(|n| n.username.clone())
        .filter(|u| !u.is_empty() && u != shown && !parts.contains(u))
    {
        parts.push(user);
    }
    parts
}

/// What the header shows of the resident's status: the scene and the friend
/// list first (the dots of the plates and the conversations), then what the
/// profile reply said.
fn status_of(world: &World, id: &Uuid, own: bool, profile: Option<&AvatarProfile>, friend: Option<&Friend>) -> Status {
    if own {
        return Status::Online;
    }
    let seen = presence::of(world, id);
    if seen != Status::Unknown {
        return seen;
    }
    match profiles::online_status(
        profile,
        false,
        friend.map(|f| (f.online, f.rights_has & rights::ONLINE_STATUS != 0)),
    ) {
        Online::Yes => Status::Online,
        Online::No => Status::Offline,
        Online::Unknown => Status::Unknown,
    }
}

/// The region of a resident the viewer sees (its coarse location), or ours.
fn region_of(world: &World, id: &Uuid, own: bool) -> Option<String> {
    if world.rlva.has("showloc") {
        return None;
    }
    if own {
        return Some(world.region_name()).filter(|n| !n.is_empty());
    }
    world
        .coarse
        .iter()
        .find(|(_, list)| list.iter().any(|(a, _)| a == id))
        .and_then(|(h, _)| world.regions.get(h))
        .map(|r| r.name.clone())
        .filter(|n| !n.is_empty())
}

/// A badge of the header (`.bdg` of the mock-up), in its order.
#[derive(Debug, Clone, PartialEq)]
enum Badge {
    Friend,
    /// Groups we are in too.
    Common(usize),
    /// The account: its name, and whether it is a Linden's.
    Account(&'static str, bool),
    /// A contact set (Firestorm's), with its color.
    Set(String, Color32),
    Blocked,
    /// The link to the People window.
    People,
}

/// What the badges are made of.
#[derive(Default)]
struct Facts {
    own: bool,
    friend: bool,
    common: usize,
    account: Option<(&'static str, bool)>,
    sets: Vec<(String, Color32)>,
    blocked: bool,
}

/// The badges come from what the server sends (the account) and from what
/// the viewer already knows (friendship, groups in common, contact sets,
/// block list).
fn badges(f: &Facts, compact: bool) -> Vec<Badge> {
    let mut out = Vec::new();
    if f.friend {
        out.push(Badge::Friend);
    }
    if f.common > 0 && !f.own {
        out.push(Badge::Common(f.common));
    }
    if let Some((name, linden)) = f.account {
        out.push(Badge::Account(name, linden));
    }
    out.extend(f.sets.iter().map(|(n, c)| Badge::Set(n.clone(), *c)));
    if f.blocked {
        out.push(Badge::Blocked);
    }
    // the compact header has no room for the link: it is in "…"
    if !f.own && !compact {
        out.push(Badge::People);
    }
    out
}

fn facts(world: &World, id: &Uuid, own: bool, profile: Option<&AvatarProfile>) -> Facts {
    let friend = world.social.friends.iter().any(|f| f.id == *id);
    let sets = world
        .contact_sets
        .friend_sets(id)
        .into_iter()
        .filter_map(|name| {
            let c = world.contact_sets.set(&name)?.color;
            // the color the resident gave the contact set
            let color = Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255);
            Some((name, color))
        })
        .collect();
    Facts {
        own,
        friend,
        common: profile.map_or(0, |pr| pr.groups.iter().filter(|g| world.groups.is_member(&g.id)).count()),
        account: profile.and_then(profiles::header_badge),
        sets,
        blocked: world.is_avatar_blocked(id),
    }
}

/// A button of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Conversation,
    Call,
    AddFriend,
    OfferTeleport,
    Pay,
    Block,
    More,
    Edit,
    ViewAsOthers,
    Done,
}

/// The header's buttons. A stranger's "Add as friend" takes the place of
/// "Call" in the compact row (the mock-up); it is in Info otherwise.
fn header_acts(own: bool, editing: bool, friend: bool, compact: bool) -> Vec<Act> {
    use Act::*;
    match (own, editing) {
        (true, true) => vec![ViewAsOthers, Done],
        (true, false) => vec![Edit],
        _ if compact && !friend => vec![Conversation, AddFriend, OfferTeleport, Pay, Block, More],
        _ => vec![Conversation, Call, OfferTeleport, Pay, Block, More],
    }
}

/// A click on one of the rights we grant: sent at once, or asked first for
/// the right to modify our objects (GrantModifyRights / RevokeModifyRights,
/// LLFloaterProfilePermissions::confirmModifyRights).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightChange {
    Send(i32),
    Ask { rights: i32, grant: bool },
}

fn right_change(given: i32, bit: i32) -> RightChange {
    let on = given & bit == 0;
    let rights = crate::ui::contacts::rights_after(given, bit, on);
    if bit == rights::MODIFY_OBJECTS {
        RightChange::Ask { rights, grant: on }
    } else {
        RightChange::Send(rights)
    }
}

/// The tabs of the Aurora window, as the classic window's tab numbers.
fn tabs_of(own: bool) -> Vec<usize> {
    // no private notes on oneself (the mock-up of our own profile)
    let mut t = vec![TAB_SL, TAB_PICKS, TAB_CLASSIFIEDS, TAB_RL, TAB_FEED];
    if !own {
        t.push(TAB_NOTES);
    }
    t
}

/// What is edited and not saved, for the bar ("bio, 1 place").
fn unsaved_parts(w: &Window) -> Vec<String> {
    let mut parts = Vec::new();
    if w.about.dirty() {
        parts.push(aurora_i18n::tr!("ui-profile-aurora-part-bio").to_owned());
    }
    if w.fl_about.dirty() {
        parts.push(aurora_i18n::tr!("ui-profile-aurora-part-first-life").to_owned());
    }
    if w.notes.dirty() {
        parts.push(aurora_i18n::tr!("ui-profile-aurora-part-notes").to_owned());
    }
    if w.pick_edit.as_ref().is_some_and(|(e, base)| e != base) {
        parts.push(aurora_i18n::tr!("ui-profile-aurora-part-places", count = 1));
    }
    if w.classified_edit.as_ref().is_some_and(|(e, base)| e != base) {
        parts.push(aurora_i18n::tr!("ui-profile-aurora-part-classified").to_owned());
    }
    parts
}

/// The edits put back as the server has them; a new pick or classified is
/// dropped.
fn revert(w: &mut Window) {
    for e in [&mut w.about, &mut w.fl_about, &mut w.notes] {
        e.text = e.base.clone();
    }
    if w.pick_edit.as_ref().is_some_and(|(e, _)| e.id.is_nil()) {
        w.pick_edit = None;
    } else if let Some((e, base)) = w.pick_edit.as_mut() {
        *e = base.clone();
    }
    if w.classified_edit.as_ref().is_some_and(|(e, _)| e.id.is_nil()) {
        w.classified_edit = None;
    } else if let Some((e, base)) = w.classified_edit.as_mut() {
        *e = base.clone();
    }
    w.publish = None;
}

fn semibold(ui: &egui::Ui, size: f32) -> FontId {
    FontId::new(size, fonts::semibold(ui.ctx()))
}

fn paint_icon(ui: &egui::Ui, name: &str, center: Pos2, size: f32, tint: Color32) {
    if let Some(t) = icons::global(name) {
        ui.painter()
            .image(t.id(), Rect::from_center_size(center, Vec2::splat(size)), UV, tint);
    }
}

fn body(ui: &mut egui::Ui, env: &mut Env, w: &mut Window) {
    let world = env.world;
    let own = w.id == world.agent_id;
    if !own {
        w.editing = false;
    }
    let profile = world.profiles.get(&w.id);
    if profile.is_some_and(|pr| !pr.sl_image.is_nil()) {
        env.wanted_pics.insert(w.id);
    }
    let compact = ui.available_width() < COMPACT_W;
    // the gaps between the header and the tabs are the mock-up's
    ui.spacing_mut().item_spacing.y = 0.0;
    let f = facts(world, &w.id, own, profile);
    let before = w.tab;
    if !tabs_of(own).contains(&w.tab) {
        w.tab = TAB_SL;
    }
    if compact {
        header_compact(ui, env, w, &f);
        ui.add_space(10.0);
        tab_dropdown(ui, env, w, own);
        ui.add_space(8.0);
    } else {
        // the tabs along the banner's bottom
        header(ui, env, w, &f);
        ui.add_space(10.0);
    }
    if before == TAB_FEED && w.tab != TAB_FEED {
        w.web.focused = false;
    }
    // the bar of the edits not saved stays at the bottom
    let closing = w.confirm_close;
    let bar = closing || (own && has_unsaved(w));
    let bar_h = if bar { 46.0 } else { 0.0 };
    let full = ui.max_rect();
    let rect = Rect::from_min_max(ui.cursor().min, pos2(full.right(), full.bottom() - bar_h));
    ui.allocate_rect(Rect::from_min_max(rect.min, full.max), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    if w.tab == TAB_FEED {
        // the browser takes the room left: no scrolling around it
        let mut inner = child.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(INSET, 0.0))));
        tabs::web(&mut inner, env, w);
    } else {
        egui::ScrollArea::vertical()
            .id_salt(("profile-card-body", w.id, w.tab))
            .auto_shrink([false, false])
            .show(&mut child, |ui| match w.tab {
                TAB_PICKS | TAB_CLASSIFIEDS | TAB_RL | TAB_NOTES => {
                    ui.horizontal_top(|ui| {
                        ui.add_space(INSET);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - INSET);
                            ui.spacing_mut().item_spacing.y = GAP;
                            match w.tab {
                                TAB_PICKS => tabs::picks(ui, env, w, compact),
                                TAB_CLASSIFIEDS => tabs::classifieds(ui, env, w, compact),
                                TAB_RL => tabs::first_life(ui, env, w, compact),
                                _ => tabs::notes(ui, env, w),
                            }
                        });
                    });
                    ui.add_space(INSET + 6.0);
                }
                _ => profile_tab(ui, env, w, compact),
            });
    }
    if bar {
        let r = Rect::from_min_max(pos2(full.left(), full.bottom() - bar_h + 8.0), full.max);
        let mut bottom = ui.new_child(egui::UiBuilder::new().max_rect(r));
        unsaved_bar(&mut bottom, env, w, closing);
    }
}

/// The banner as tall as what it holds (`widgets::hero`): the picture on
/// the left, who it is beside it and the buttons on the right, centred on
/// one line, the tabs along its bottom.
fn header(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, f: &Facts) {
    const PHOTO: f32 = 116.0;
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let own = f.own;
    let profile = world.profiles.get(&id);
    let full = ui.max_rect();
    let top = ui.cursor().top();
    let key = ui.scope_id().with(("profile-hero", id));
    let who_h = hero::measured(ui, key, PHOTO);
    let (banner, line, tabs) = hero::rows(full, top, who_h.max(PHOTO));
    let picture = profile.map(|pr| pr.sl_image).filter(|i| !i.is_nil());
    paint_banner(ui, p, banner, picture.and_then(|i| env.banners.get(&i)));
    hero::veil(ui, ui.painter(), p, banner);
    let mark = crate::world::staff::mark(world, &id);
    crate::ui::staff::banner_light(ui, p, banner, mark);
    // in edit mode, where the banner comes from; the picture in large is a
    // click on the round picture, with no second button for it
    if w.editing {
        banner_chip(
            ui,
            p,
            banner,
            "info-circle-filled",
            aurora_i18n::tr!("ui-profile-aurora-banner-follows"),
        );
    }
    let photo_rect = Rect::from_center_size(pos2(full.left() + 22.0 + PHOTO / 2.0, line.center().y), Vec2::splat(PHOTO));
    let status = photo(ui, env, w, photo_rect, 4.0);
    // the buttons, centred on the line
    let pad = 150.0;
    let row = Rect::from_min_max(
        pos2(full.left() + pad, line.center().y - BUTTON_H / 2.0),
        pos2(full.right() - 14.0, line.center().y + BUTTON_H / 2.0),
    );
    let acts = header_acts(own, w.editing, f.friend, false);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    // with their names while who it is keeps room for its badges on a line
    // or two, else the main action alone named (a long language)
    let mut probe = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(layout).sizing_pass().invisible());
    probe.spacing_mut().item_spacing.x = 5.0;
    for act in acts.iter().rev() {
        act_button(&mut probe, env, w, *act, f, Look::Labelled);
    }
    let named = row.width() - probe.min_rect().width() - 18.0 >= hero::WHO_MIN_W;
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(layout));
    right.spacing_mut().item_spacing.x = 5.0;
    for act in acts.iter().rev() {
        let main = matches!(act, Act::Conversation | Act::Edit | Act::Done);
        let look = if named || main { Look::Labelled } else { Look::Icon };
        act_button(&mut right, env, w, *act, f, look);
    }
    let acts_w = right.min_rect().width() + 18.0;
    // who it is, centred on the line too, measured for the next pass
    let col = Rect::from_min_max(
        pos2(full.left() + pad, line.center().y - who_h / 2.0),
        pos2(full.right() - 14.0 - acts_w, line.center().y + who_h / 2.0 + 200.0),
    );
    let mut who = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(col)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    who_it_is(&mut who, env, w, f, false, status, mark);
    hero::measure(ui, key, who.min_rect().height());
    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(tabs.shrink2(Vec2::new(10.0, 0.0))));
    tab_strip(&mut strip, env, w, own);
    ui.advance_cursor_after_rect(banner);
}

/// The compact header: a short banner, the picture over its edge, who it is
/// beside it, then the actions in a row of icons as wide as the window.
fn header_compact(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, f: &Facts) {
    const PHOTO: f32 = 64.0;
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let own = f.own;
    let profile = world.profiles.get(&id);
    let full = ui.max_rect();
    let top = ui.cursor().top();
    let banner = Rect::from_min_size(pos2(full.left(), top), Vec2::new(full.width(), 70.0));
    let picture = profile.map(|pr| pr.sl_image).filter(|i| !i.is_nil());
    paint_banner(ui, p, banner, picture.and_then(|i| env.banners.get(&i)));
    let mark = crate::world::staff::mark(world, &id);
    crate::ui::staff::banner_light(ui, p, banner, mark);
    let photo_rect = Rect::from_min_size(banner.min + Vec2::new(14.0, 34.0), Vec2::splat(PHOTO));
    let status = photo(ui, env, w, photo_rect, 3.0);
    let pad = 82.0;
    let y0 = banner.bottom() + 10.0;
    let acts = header_acts(own, w.editing, f.friend, true);
    // our own buttons at the right of the name
    let mut acts_w = 0.0;
    if own {
        let row = Rect::from_min_max(pos2(full.left() + pad, y0), pos2(full.right() - INSET, y0 + BUTTON_H));
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::right_to_left(egui::Align::Min)),
        );
        right.spacing_mut().item_spacing.x = 5.0;
        for act in acts.iter().rev() {
            act_button(&mut right, env, w, *act, f, Look::Labelled);
        }
        acts_w = right.min_rect().width() + 14.0;
    }
    let col = Rect::from_min_max(pos2(full.left() + pad, y0), pos2(full.right() - INSET - acts_w, y0 + 200.0));
    let mut who = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(col)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    who_it_is(&mut who, env, w, f, true, status, mark);
    let mut bottom = who.min_rect().bottom().max(photo_rect.bottom()).max(y0 + BUTTON_H);
    if !own {
        // the actions in a row of icons, as wide as the window
        let row = Rect::from_min_size(pos2(full.left(), bottom + 10.0), Vec2::new(full.width(), BUTTON_H));
        let mut icons_row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        icons_row.spacing_mut().item_spacing.x = 4.0;
        let n = acts.len() as f32;
        let each = ((row.width() - 4.0 * (n - 1.0)) / n).floor();
        for act in &acts {
            act_button(&mut icons_row, env, w, *act, f, Look::Row(each));
        }
        bottom = row.bottom();
    }
    ui.advance_cursor_after_rect(Rect::from_min_max(pos2(full.left(), top), pos2(full.right(), bottom)));
}

/// The round picture with its status, a click opens it in large; in edit
/// mode, the camera that changes it.
fn photo(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, rect: Rect, ring: f32) -> Status {
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let own = id == world.agent_id;
    let profile = world.profiles.get(&id);
    let friend = world.social.friends.iter().find(|fr| fr.id == id);
    let pic = env.pics.get(&id);
    let status = status_of(world, &id, own, profile, friend);
    paint_photo(ui, p, rect, ring, pic, &display_name(world, &id), status, w.editing);
    let resp = ui.interact(rect, ui.scope_id().with(("profile-photo", id)), Sense::click());
    if pic.is_some() && !w.editing && resp.on_hover_text(aurora_i18n::tr!("ui-profile-aurora-big-photo")).clicked() {
        w.open_photo(PictureSlot::SecondLife, profile.map_or(Uuid::nil(), |pr| pr.sl_image));
    }
    if w.editing {
        camera_button(ui, env, rect, profile);
    }
    status
}

/// The name (with a staff member's tag), the names under it, the birth
/// line and the badges; compact: the username and the age, no birth line.
fn who_it_is(
    ui: &mut egui::Ui,
    env: &mut Env,
    w: &Window,
    f: &Facts,
    compact: bool,
    status: Status,
    mark: Option<crate::world::staff::Mark>,
) {
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let profile = world.profiles.get(&id);
    let name = display_name(world, &id);
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let size = if compact { 16.0 } else { 22.0 };
        let tag = if compact { 10.0 } else { 12.0 };
        // a staff member's tag after the name, cut before it
        let staff = mark.map_or(0.0, |mark| crate::ui::staff::Tag::new(ui, p, mark, tag).size().x + 8.0);
        let max = (ui.available_width() - if w.editing { 30.0 } else { 0.0 } - staff).max(40.0);
        ui.add(widgets::elided_label(
            ui,
            RichText::new(&name)
                .font(semibold(ui, size))
                .color(crate::ui::colors::resident_name(world, &id, p.ink)),
            max,
        ));
        crate::ui::staff::after_name(ui, p, mark, tag);
        if w.editing && widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("common-set-display-name"), true) {
            env.actions.push(ProfileAction::DisplayName);
        }
    });
    let mut sub = names_line(world, &id, &name);
    if compact {
        // the username alone, then the age (the mock-up's "orion.exemple · 19 ans")
        sub.retain(|s| !s.contains(' '));
        if let Some(age) = profile.and_then(|pr| age_only(pr, f.own)) {
            sub.push(age);
        }
    }
    if !sub.is_empty() {
        let max = ui.available_width();
        ui.add(widgets::elided_label(
            ui,
            RichText::new(sub.join(" · ")).size(12.5).color(p.muted),
            max,
        ));
    }
    ui.add_space(if compact { 4.0 } else { 6.0 });
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().interact_size.y = 16.0;
    if !compact {
        meta_line(ui, env, id, f.own, profile, status);
    }
    let list = badges(f, compact);
    if !list.is_empty() {
        let is_friend = world.social.friends.iter().any(|fr| fr.id == id);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
            for b in &list {
                badge_widget(ui, env, id, b, is_friend, compact);
            }
        });
    }
}

/// The age alone, for the compact line under the name.
fn age_only(pr: &AvatarProfile, own: bool) -> Option<String> {
    let line = profiles::birth_line(pr, own, now())?;
    line.split_once(" · ").map(|(_, age)| age.to_owned())
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The picture's middle band over the banner, faded into the window toward
/// its bottom; a band of the brand's colors without a picture.
fn paint_banner(ui: &egui::Ui, p: &Palette, rect: Rect, tex: Option<&egui::TextureHandle>) {
    let radius = ui.visuals().window_corner_radius;
    match tex {
        Some(t) => {
            let [tw, th] = t.size();
            let aspect = tw.max(1) as f32 / th.max(1) as f32;
            let want = rect.width() / rect.height();
            // cover: the banner shows the middle of the picture
            let uv = if aspect > want {
                let u = want / aspect;
                Rect::from_min_max(pos2(0.5 - u / 2.0, 0.0), pos2(0.5 + u / 2.0, 1.0))
            } else {
                let v = aspect / want;
                Rect::from_min_max(pos2(0.0, 0.5 - v / 2.0), pos2(1.0, 0.5 + v / 2.0))
            };
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                .uv(uv)
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.navy);
            let mut m = egui::Mesh::default();
            let (a, b) = (p.violet.gamma_multiply(0.55), p.indigo.gamma_multiply(0.45));
            for (pos, c) in [
                (rect.left_top(), a),
                (rect.right_top(), b),
                (rect.right_bottom(), b),
                (rect.left_bottom(), a),
            ] {
                m.colored_vertex(pos, c);
            }
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            ui.painter().add(egui::Shape::mesh(m));
        }
    }
    // toward the window's panel from a third of its height
    let fade_top = rect.top() + rect.height() * 0.3;
    let base = p.panel.gamma_multiply(0.85);
    let mut m = egui::Mesh::default();
    for (pos, c) in [
        (pos2(rect.left(), fade_top), Color32::TRANSPARENT),
        (pos2(rect.right(), fade_top), Color32::TRANSPARENT),
        (rect.right_bottom(), base),
        (rect.left_bottom(), base),
    ] {
        m.colored_vertex(pos, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(m));
}

/// The note at the top right of the banner (a label, not a button).
fn banner_chip(ui: &egui::Ui, p: &Palette, banner: Rect, icon: &str, text: &str) {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let size = Vec2::new(8.0 + 12.0 + 4.0 + galley.size().x + 8.0, 20.0);
    let rect = Rect::from_min_size(pos2(banner.right() - 10.0 - size.x, banner.top() + 10.0), size);
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.navy.gamma_multiply(0.45));
    let ink = p.ink.gamma_multiply(0.85);
    paint_icon(ui, icon, pos2(rect.left() + 14.0, rect.center().y), 12.0, ink);
    widgets::paint_ink_left(ui.painter(), galley, pos2(rect.left() + 24.0, rect.center().y), ink);
}

/// The round picture in its ring, with the status dot at its bottom right
/// (round like the plates' and the conversations', docs/BRANDING.md).
#[allow(clippy::too_many_arguments)]
fn paint_photo(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    ring: f32,
    pic: Option<&egui::TextureHandle>,
    name: &str,
    status: Status,
    editing: bool,
) {
    let painter = ui.painter();
    let r = rect.width() / 2.0;
    painter.circle_filled(rect.center(), r, p.panel);
    widgets::avatar_disc(ui, p, rect.shrink(ring), pic, name, false);
    if editing {
        painter.circle_stroke(rect.center(), r + 1.0, egui::Stroke::new(2.0, p.violet_light));
    }
    // the dot: 24 px with its ring in the large header, 16 in the compact one
    let dot = if rect.width() > 80.0 { 12.0 } else { 8.0 };
    let inset = if rect.width() > 80.0 { 4.0 } else { 0.0 };
    let c = rect.right_bottom() - Vec2::splat(inset + dot);
    painter.circle_filled(c, dot, p.panel);
    presence::paint_mark(painter, c, dot - ring, status.shape(), presence::color(p, status), p.panel);
}

/// The camera button of our picture in edit mode and its menu (the classic
/// window's picture menu, menu_fs_profile_image_actions.xml).
fn camera_button(ui: &mut egui::Ui, env: &mut Env, photo: Rect, profile: Option<&AvatarProfile>) {
    let p = env.p;
    let c = photo.min + Vec2::new(99.0, 101.0) * (photo.width() / 116.0);
    let rect = Rect::from_center_size(c, Vec2::splat(34.0));
    let resp = ui.interact(rect, ui.scope_id().with("profile-camera"), Sense::click());
    let fill = if resp.hovered() { p.violet_light } else { p.violet };
    ui.painter().circle_filled(c, 17.0, fill);
    paint_icon(ui, "camera-filled", c, 17.0, p.on_accent);
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-profile-change-picture"));
    let image = profile.map(|pr| pr.sl_image).unwrap_or_default();
    let mut choose = false;
    let mut remove = false;
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        menu::item_tip(
            ui,
            p,
            "file-upload-filled",
            aurora_i18n::tr!("ui-profile-aurora-upload-image"),
            false,
            aurora_i18n::tr!("ui-profile-coming-soon"),
        );
        choose = menu::item(ui, p, "photo-filled", aurora_i18n::tr!("ui-profile-aurora-choose-from-inventory"));
        remove = menu::item_if(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("ui-profile-aurora-remove-photo"),
            !image.is_nil(),
        );
    });
    if choose && profile.is_some() {
        *env.pick_picture = Some((PictureSlot::SecondLife, image));
    }
    if remove {
        env.actions.push(ProfileAction::Save {
            target: env.world.agent_id,
            data: llsd_map! { "sl_image_id" => Uuid::nil() },
        });
    }
}

/// One of the header's buttons; `icon_only` in the compact row, `width`
/// wide.
/// How a button of the header shows.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Look {
    /// Its icon and its name.
    Labelled,
    /// Its icon alone, its name in its tooltip (a narrow large header).
    Icon,
    /// Its icon alone, this wide, in the compact header's row.
    Row(f32),
}

fn act_button(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, act: Act, f: &Facts, look: Look) {
    let p = env.p;
    let id = w.id;
    let (icon, label, kind, enabled, tip): (&str, &str, ActionKind, bool, Option<&str>) = match act {
        Act::Conversation => (
            "message-circle-filled",
            aurora_i18n::tr!("ui-profile-aurora-conversation"),
            ActionKind::Primary,
            true,
            None,
        ),
        // LLAvatarActions::startCall, enabled with voice (canCall)
        Act::Call => {
            let voice = env.world.calls.voice_enabled;
            (
                "phone-filled",
                aurora_i18n::tr!("ui-profile-aurora-call"),
                ActionKind::Flat,
                voice,
                (!voice).then(|| aurora_i18n::tr!("ui-call-voice-off")),
            )
        }
        Act::AddFriend => (
            "user-plus",
            aurora_i18n::tr!("ui-profile-aurora-add-friend"),
            ActionKind::Flat,
            true,
            None,
        ),
        Act::OfferTeleport => (
            "send-filled",
            aurora_i18n::tr!("ui-profile-aurora-offer-tp"),
            ActionKind::Flat,
            true,
            None,
        ),
        Act::Pay => ("coin-filled", aurora_i18n::tr!("common-pay"), ActionKind::Flat, true, None),
        Act::Block => {
            let label = if f.blocked {
                aurora_i18n::tr!("common-unblock")
            } else {
                aurora_i18n::tr!("common-block")
            };
            ("ban", label, ActionKind::Flat, true, None)
        }
        Act::More => ("dots-filled", aurora_i18n::tr!("ui-profile-more"), ActionKind::Flat, true, None),
        Act::Edit => (
            "pencil-filled",
            aurora_i18n::tr!("ui-profile-aurora-edit"),
            ActionKind::Primary,
            true,
            None,
        ),
        Act::ViewAsOthers => (
            "eye-filled",
            aurora_i18n::tr!("ui-profile-aurora-view-as-others"),
            ActionKind::Flat,
            true,
            None,
        ),
        Act::Done => ("check", aurora_i18n::tr!("ui-profile-aurora-done"), ActionKind::Primary, true, None),
    };
    // the block button and "…" are icons in the large header too
    let alone = look != Look::Labelled || matches!(act, Act::Block | Act::More);
    let size = Vec2::new(
        match look {
            Look::Row(width) => width,
            _ if alone => 30.0,
            _ => 0.0,
        },
        BUTTON_H,
    );
    let shown = if alone { "" } else { label };
    let resp = widgets::action_button_if(ui, p, icon, shown, kind, size, enabled);
    if alone {
        // its name for a screen reader too
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    }
    let resp = match (tip, alone) {
        (Some(t), _) => resp.on_hover_text(format!("{label} · {t}")),
        (None, true) => resp.on_hover_text(label),
        _ => resp,
    };
    if act == Act::More {
        more_menu(&resp, env, w, f, matches!(look, Look::Row(_)));
        return;
    }
    if !resp.clicked() || !enabled {
        return;
    }
    match act {
        Act::Conversation => env.actions.push(ProfileAction::Im(id)),
        Act::AddFriend => context::request(ui.ctx(), CtxAction::AddFriend(id)),
        Act::OfferTeleport => env.actions.push(ProfileAction::OfferTeleport(id)),
        // LLAvatarActions::pay
        Act::Pay => context::request(ui.ctx(), CtxAction::PayResident(id)),
        Act::Block => env.actions.push(ProfileAction::ToggleBlock(id)),
        Act::Call => context::request(ui.ctx(), CtxAction::Call(id)),
        Act::Edit => w.editing = true,
        Act::ViewAsOthers => w.editing = false,
        Act::Done => {
            save_all(w, env.actions);
            w.editing = false;
        }
        Act::More => {}
    }
}

/// "…": the actions that have no button (menu_fs_profile_overflow.xml and
/// the name field's copy menu).
fn more_menu(resp: &Response, env: &Env, w: &Window, f: &Facts, compact: bool) {
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let names = &world.social.avatar_names;
    let display = names.get(&id).map(|n| n.display(&names.options));
    let legacy = world.legacy_name(&id);
    let mappable = world.can_locate_avatar(id);
    menu::dropdown(resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        let ctx = ui.ctx().clone();
        let act = |a: CtxAction| context::request(&ctx, a);
        if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-map"), mappable) {
            act(CtxAction::ShowOnMap(id));
        }
        // no Share: it only brought the inventory forward, and an item
        // dropped anywhere on the window is given (`super::give_drop`)
        if menu::item(ui, p, "plane-arrival-filled", aurora_i18n::tr!("ui-profile-aurora-request-tp")) {
            act(CtxAction::RequestTeleport(id));
        }
        if compact && menu::item(ui, p, "users", aurora_i18n::tr!("ui-profile-aurora-view-in-people")) {
            act(CtxAction::OpenPeople(people_tab(f)));
        }
        if f.friend && menu::item(ui, p, "user-minus", aurora_i18n::tr!("ui-profile-aurora-remove-friend")) {
            act(CtxAction::RemoveFriend(id));
        }
        menu::separator(ui, p);
        if menu::item_if(
            ui,
            p,
            "id-filled",
            aurora_i18n::tr!("ui-profile-copy-display-name"),
            display.is_some(),
        ) {
            ui.ctx().copy_text(display.clone().unwrap_or_default());
        }
        if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-name"), legacy.is_some()) {
            ui.ctx().copy_text(legacy.clone().unwrap_or_default());
        }
        if menu::item(ui, p, "key-filled", aurora_i18n::tr!("ui-profile-aurora-copy-key")) {
            ui.ctx().copy_text(id.to_string());
        }
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("ui-profile-copy-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
        }
        menu::separator(ui, p);
        // LLFloaterReporter::showFromAvatar (llpanelprofile.cpp)
        if menu::item(ui, p, "flag-filled", aurora_i18n::tr!("common-report")) {
            act(CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Avatar(id)));
        }
        if menu::item_if(
            ui,
            p,
            "history",
            aurora_i18n::tr!("ui-profile-aurora-history"),
            world.chat_log.dir().is_some(),
        ) {
            act(CtxAction::ShowHistory(Some(id)));
        }
    });
}

/// The People tab that has the resident: Friends, Blocked, else Nearby
/// (the People window does not select them yet).
fn people_tab(f: &Facts) -> u8 {
    if f.friend {
        1
    } else if f.blocked {
        4
    } else {
        0
    }
}

/// The birth date and age, then the status with the region.
fn meta_line(ui: &mut egui::Ui, env: &Env, id: Uuid, own: bool, profile: Option<&AvatarProfile>, status: Status) {
    let p = env.p;
    let world = env.world;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if let Some(pr) = profile
            && let Some(line) = profiles::birth_line(pr, own, now())
        {
            let r = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                    paint_icon(ui, "cake", rect.center(), 13.0, p.muted_dim);
                    ui.label(RichText::new(line).size(12.0).color(p.muted));
                })
                .response;
            if let Some(full) = profiles::birth_text(pr, own, now()) {
                r.on_hover_text(format!("{}\n{full}", aurora_i18n::tr!("ui-profile-aurora-birth-date")));
            }
        }
        // our own status is the top bar's (the mock-up of our profile has none)
        if status != Status::Unknown && !own {
            let color = match status {
                Status::Offline => p.muted,
                s => presence::color(p, s),
            };
            let text = match region_of(world, &id, own).filter(|_| status != Status::Offline) {
                Some(region) => format!("{} · {region}", status.label()),
                None => status.label().to_owned(),
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 14.0), Sense::hover());
                presence::paint_mark(
                    ui.painter(),
                    rect.center(),
                    4.0,
                    status.shape(),
                    presence::color(p, status),
                    p.panel,
                );
                ui.label(RichText::new(text).size(12.0).color(color));
            });
        }
    });
}

/// What leads a badge's text.
enum Lead<'a> {
    Icon(&'a str),
    Dot(Color32),
}

/// A pill of the header: its lead, then the text, in `fg` over `bg`; a
/// trailing icon for a link.
fn pill(ui: &mut egui::Ui, lead: Option<Lead>, text: &str, trail: Option<&str>, fg: Color32, bg: Color32, sense: Sense) -> Response {
    let font = semibold(ui, 11.5);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
    let pad = if bg == Color32::TRANSPARENT { 2.0 } else { 8.0 };
    let lead_w = if lead.is_some() { 16.0 } else { 0.0 };
    let trail_w = if trail.is_some() { 16.0 } else { 0.0 };
    let size = Vec2::new(pad + lead_w + galley.size().x + trail_w + 8.0, 19.0);
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, bg);
    }
    let mut x = rect.left() + pad;
    match lead {
        Some(Lead::Icon(icon)) => paint_icon(ui, icon, pos2(x + 6.0, rect.center().y), 12.0, fg),
        Some(Lead::Dot(c)) => {
            ui.painter().circle_filled(pos2(x + 4.0, rect.center().y), 4.0, c);
        }
        None => {}
    }
    x += lead_w;
    let text_w = galley.size().x;
    widgets::paint_ink_left(ui.painter(), galley, pos2(x, rect.center().y), fg);
    if let Some(icon) = trail {
        paint_icon(ui, icon, pos2(x + text_w + 8.0, rect.center().y), 12.0, fg);
    }
    if sense.senses_click() && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn badge_widget(ui: &mut egui::Ui, env: &Env, id: Uuid, b: &Badge, is_friend: bool, compact: bool) {
    let p = env.p;
    let soft = |c: Color32| c.gamma_multiply(0.14);
    match b {
        Badge::Friend => {
            if env.world.contact_sets.appearance(&id).replace_friend {
                return;
            }
            pill(
                ui,
                Some(Lead::Icon("heart-filled")),
                aurora_i18n::tr!("ui-profile-aurora-friend"),
                None,
                p.success,
                soft(p.success),
                Sense::hover(),
            );
        }
        Badge::Common(n) => {
            let text = if compact {
                aurora_i18n::tr!("ui-profile-aurora-in-common-short", count = *n as i64)
            } else {
                aurora_i18n::tr!("ui-profile-aurora-groups-in-common", count = *n as i64)
            };
            pill(ui, Some(Lead::Icon("users-group")), &text, None, p.muted, p.bar, Sense::hover());
        }
        Badge::Account(name, linden) => {
            let (fg, icon) = if *linden { (p.teal, "id-filled") } else { (p.amber, "star-filled") };
            pill(ui, Some(Lead::Icon(icon)), name, None, fg, soft(fg), Sense::hover());
        }
        Badge::Set(name, color) => {
            let appearance = env.world.contact_sets.appearance(&id);
            let icon = appearance.icons.iter().find(|icon| &icon.set == name);
            let lead = icon
                .map(|icon| Lead::Icon(crate::world::contact_sets::appearance::ICONS[icon.icon]))
                .unwrap_or(Lead::Dot(*color));
            let ink = icon.map(|icon| crate::ui::colors::set_color(icon.color)).unwrap_or(p.muted);
            pill(ui, Some(lead), name, None, ink, p.bar, Sense::hover()).on_hover_text(aurora_i18n::tr!("ui-profile-aurora-contact-set"));
        }
        Badge::Blocked => {
            pill(
                ui,
                Some(Lead::Icon("ban")),
                aurora_i18n::tr!("common-blocked"),
                None,
                p.danger,
                soft(p.danger),
                Sense::hover(),
            );
        }
        Badge::People => {
            let resp = pill(
                ui,
                None,
                aurora_i18n::tr!("ui-profile-aurora-view-in-people"),
                Some("chevron-right"),
                p.violet_light,
                Color32::TRANSPARENT,
                Sense::click(),
            );
            if resp.clicked() {
                let facts = Facts {
                    friend: is_friend,
                    blocked: env.world.is_avatar_blocked(&id),
                    ..Facts::default()
                };
                context::request(ui.ctx(), CtxAction::OpenPeople(people_tab(&facts)));
            }
        }
    }
}

/// The rights of a friendship, on a line of Info: the ones we grant (a
/// click changes one, in violet), then a step apart the ones we receive (in
/// turquoise). Their tooltips say which is which; ending the friendship is
/// in the "…" menu.
fn rights_line(ui: &mut egui::Ui, env: &mut Env, id: Uuid, f: &Friend) {
    let p = env.p;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (bit, icon, tip) in GRANTS {
            let on = f.rights_given & bit != 0;
            let state = if on {
                aurora_i18n::tr!("ui-profile-allowed")
            } else {
                aurora_i18n::tr!("ui-profile-not-allowed")
            };
            let tip = aurora_i18n::tr!("ui-profile-click-change-permissions", tip = aurora_i18n::text(tip), state = state);
            if right_icon(ui, p, icon, on, true).on_hover_text(tip).clicked() {
                match right_change(f.rights_given, bit) {
                    RightChange::Send(rights) => env
                        .actions
                        .push(ProfileAction::Net(NetCommand::GrantUserRights { friend: id, rights })),
                    RightChange::Ask { rights, grant } => env.actions.push(ProfileAction::ModifyRights { friend: id, rights, grant }),
                }
            }
        }
        // a gap, not a line, between ours and theirs (docs/BRANDING.md)
        ui.add_space(14.0);
        for (bit, icon, tip) in RECEIVED {
            let on = f.rights_has & bit != 0;
            let state = if on {
                aurora_i18n::tr!("ui-profile-allowed")
            } else {
                aurora_i18n::tr!("ui-profile-not-allowed")
            };
            right_icon(ui, p, icon, on, false).on_hover_text(aurora_i18n::tr!(
                "ui-profile-aurora-received-tip",
                right = aurora_i18n::text(tip),
                state = state
            ));
        }
    });
}

/// The rights we grant: online status, place on the map, our objects.
const GRANTS: [(i32, &str, &str); 3] = [
    (rights::ONLINE_STATUS, "eye-filled", "ui-profile-see-when-i-am-online"),
    (rights::MAP_LOCATION, "map-pin-filled", "ui-profile-find-me-on-map"),
    (
        rights::MODIFY_OBJECTS,
        "pencil-filled",
        "ui-profile-modify-delete-or-take-my-objects",
    ),
];

/// The same rights, received.
const RECEIVED: [(i32, &str, &str); 3] = [
    (rights::ONLINE_STATUS, "eye-filled", "ui-profile-aurora-their-online"),
    (rights::MAP_LOCATION, "map-pin-filled", "ui-profile-aurora-their-map"),
    (rights::MODIFY_OBJECTS, "pencil-filled", "ui-profile-aurora-their-objects"),
];

/// A right of the bar: lit in violet when granted by us, in turquoise when
/// received.
fn right_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, on: bool, ours: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(19.0), if ours { Sense::click() } else { Sense::hover() });
    let radius = widgets::inner_radius(ui.visuals());
    if ours && on {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.15));
    }
    if ours && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tint = match (on, ours) {
        (true, true) => p.violet_light,
        (true, false) => p.teal,
        _ => p.muted_dim,
    };
    paint_icon(ui, icon, rect.center(), 13.0, tint);
    resp
}

/// One tab's look: its icon, name, and what follows the name.
struct TabLook {
    tab: usize,
    icon: &'static str,
    label: &'static str,
    count: Option<String>,
    dot: bool,
    enabled: bool,
}

fn tab_looks(env: &Env, w: &Window, own: bool) -> Vec<TabLook> {
    let world = env.world;
    let profile = world.profiles.get(&w.id);
    tabs_of(own)
        .into_iter()
        .map(|tab| {
            let (icon, label) = match tab {
                TAB_PICKS => ("map-pin-filled", aurora_i18n::tr!("ui-profile-aurora-tab-picks")),
                TAB_CLASSIFIEDS => ("speakerphone", aurora_i18n::tr!("ui-profile-aurora-tab-classifieds")),
                TAB_RL => ("user-heart", aurora_i18n::tr!("ui-profile-aurora-tab-first-life")),
                TAB_FEED => ("world-filled", aurora_i18n::tr!("ui-profile-aurora-tab-web")),
                TAB_NOTES => ("lock-filled", aurora_i18n::tr!("common-notes")),
                _ => ("id-filled", aurora_i18n::tr!("ui-profile-aurora-tab-profile")),
            };
            let count = match tab {
                TAB_PICKS => profile.map(|pr| {
                    let n = pr.picks.len().to_string();
                    if own { format!("{n} / {}", super::picks_limit(world)) } else { n }
                }),
                // known once the tab asked for them, like Firestorm's
                TAB_CLASSIFIEDS => world.profiles.classified_lists.get(&w.id).map(|l| l.len().to_string()),
                _ => None,
            }
            .filter(|n| n != "0");
            TabLook {
                tab,
                icon,
                label,
                count,
                dot: tab == TAB_NOTES && profile.and_then(|pr| pr.notes.as_deref()).is_some_and(|n| !n.trim().is_empty()),
                enabled: tab != TAB_FEED || env.web.profile_base.is_some(),
            }
        })
        .collect()
}

/// The tabs as pills, the current one raised.
fn tab_strip(ui: &mut egui::Ui, env: &Env, w: &mut Window, own: bool) {
    let p = env.p;
    let looks = tab_looks(env, w, own);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        ui.spacing_mut().item_spacing.x = 2.0;
        for t in &looks {
            let font = semibold(ui, 12.5);
            let galley = ui.painter().layout_no_wrap(t.label.to_owned(), font, p.ink);
            let count = t
                .count
                .as_ref()
                .map(|c| ui.painter().layout_no_wrap(c.clone(), semibold(ui, 11.0), p.muted_dim));
            let extra = count.as_ref().map_or(0.0, |g| 5.0 + g.size().x) + if t.dot { 11.0 } else { 0.0 };
            let size = Vec2::new(12.0 + 14.0 + 5.0 + galley.size().x + extra + 12.0, 28.0);
            let (rect, resp) = ui.allocate_exact_size(size, if t.enabled { Sense::click() } else { Sense::hover() });
            let on = w.tab == t.tab;
            let radius = ui.visuals().widgets.inactive.corner_radius;
            if on {
                ui.painter().rect_filled(rect, radius, p.raised);
            } else if resp.hovered() && t.enabled {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
            }
            let ink = match (on, t.enabled) {
                (true, _) => p.ink,
                (false, true) => p.muted,
                (false, false) => p.muted_dim.gamma_multiply(0.6),
            };
            let mut x = rect.left() + 12.0;
            paint_icon(ui, t.icon, pos2(x + 7.0, rect.center().y), 14.0, ink);
            x += 19.0;
            let label_w = galley.size().x;
            widgets::paint_ink_left(ui.painter(), galley, pos2(x, rect.center().y), ink);
            x += label_w;
            if let Some(g) = count {
                let gw = g.size().x;
                widgets::paint_ink_left(ui.painter(), g, pos2(x + 5.0, rect.center().y), p.muted_dim);
                x += 5.0 + gw;
            }
            if t.dot {
                ui.painter().circle_filled(pos2(x + 8.0, rect.center().y), 3.0, p.teal);
            }
            if resp.clicked() && t.enabled {
                w.tab = t.tab;
            }
        }
    });
}

/// The compact window's tabs: the current one in a drop-down list.
fn tab_dropdown(ui: &mut egui::Ui, env: &Env, w: &mut Window, own: bool) {
    let p = env.p;
    let looks = tab_looks(env, w, own);
    let width = ui.available_width() - 2.0 * INSET;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    let rect = Rect::from_min_size(pos2(row.left() + INSET, row.top()), Vec2::new(width, 32.0));
    let resp = ui.interact(rect, ui.scope_id().with(("profile-tabs", w.id)), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if resp.hovered() {
        p.raised.lerp_to_gamma(p.ink, 0.08)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, radius, fill);
    let current = looks.iter().find(|t| t.tab == w.tab).unwrap_or(&looks[0]);
    paint_icon(ui, current.icon, pos2(rect.left() + 17.0, rect.center().y), 14.0, p.ink);
    let galley = ui.painter().layout_no_wrap(current.label.to_owned(), semibold(ui, 12.5), p.ink);
    widgets::paint_ink_left(ui.painter(), galley, pos2(rect.left() + 30.0, rect.center().y), p.ink);
    let sections = aurora_i18n::tr!("ui-profile-aurora-sections", count = looks.len() as i64);
    let right = ui.painter().layout_no_wrap(sections, semibold(ui, 12.5), p.muted);
    let rw = right.size().x;
    paint_icon(ui, "caret-down-filled", pos2(rect.right() - 16.0, rect.center().y), 12.0, p.muted);
    widgets::paint_ink_left(ui.painter(), right, pos2(rect.right() - 26.0 - rw, rect.center().y), p.muted);
    let mut chosen = None;
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(width - 8.0);
        for t in &looks {
            let label = match &t.count {
                Some(c) => format!("{} · {c}", t.label),
                None => t.label.to_owned(),
            };
            if menu::check(ui, p, t.icon, &label, w.tab == t.tab, t.enabled) {
                chosen = Some(t.tab);
            }
        }
    });
    if let Some(tab) = chosen {
        w.tab = tab;
    }
}

/// The link at the right of a card's head; `waiting` greys it out with
/// that tooltip (a feature not ported yet).
struct Link<'a> {
    text: &'a str,
    icon: Option<&'a str>,
    waiting: Option<&'a str>,
}

impl<'a> Link<'a> {
    fn new(text: &'a str) -> Link<'a> {
        Link {
            text,
            icon: None,
            waiting: None,
        }
    }
}

/// A card of the Profile tab: its head (icon, title in capitals, a link on
/// the right), then its content. `note`: the turquoise edge of the private
/// notes. Returns whether the head's link was clicked.
fn card(
    ui: &mut egui::Ui,
    p: &Palette,
    head: Option<(&str, &str)>,
    link: Option<Link>,
    note: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut clicked = false;
    let resp = egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: if note { 15 } else { 12 },
            right: 12,
            top: 10,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // the head 8 px over the content (.card > .h)
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some((icon, title)) = head {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
                    paint_icon(ui, icon, rect.center(), 11.0, p.muted_dim);
                    ui.label(RichText::new(title.to_uppercase()).font(semibold(ui, 10.5)).color(p.muted_dim));
                    if let Some(link) = link {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            let color = if link.waiting.is_some() { p.muted_dim } else { p.violet_light };
                            let sense = if link.waiting.is_some() { Sense::hover() } else { Sense::click() };
                            // right to left: a chevron after the text, a
                            // leading icon before it
                            let trailing = link.icon == Some("chevron-right");
                            let icon = |ui: &mut egui::Ui| {
                                if let Some(icon) = link.icon {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                                    paint_icon(ui, icon, rect.center(), 12.0, color);
                                }
                            };
                            if trailing {
                                icon(ui);
                            }
                            let mut r =
                                ui.add(egui::Label::new(RichText::new(link.text).font(semibold(ui, 11.5)).color(color)).sense(sense));
                            if !trailing {
                                icon(ui);
                            }
                            r = match link.waiting {
                                Some(tip) => r.on_hover_text(tip),
                                None => r.on_hover_cursor(egui::CursorIcon::PointingHand),
                            };
                            clicked = r.clicked() && link.waiting.is_none();
                        });
                    }
                });
            }
            body(ui);
        })
        .response;
    if note {
        let r = resp.rect;
        ui.painter().rect_filled(
            Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom())),
            egui::CornerRadius::ZERO,
            p.teal,
        );
    }
    clicked
}

/// The Profile tab: About and the favorite places on the left, Info,
/// Groups and the private notes on the right; one column when compact. An
/// item is given by dropping it anywhere on the window (`super::give_drop`).
fn profile_tab(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let width = ui.available_width() - 2.0 * INSET;
    ui.horizontal_top(|ui| {
        ui.add_space(INSET);
        if compact {
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                about_card(ui, env, w, true);
                info_card(ui, env, w, true);
                picks_card(ui, env, w);
                groups_card(ui, env, w);
                notes_card(ui, env, w);
            });
        } else {
            let main_w = width - SIDE_W - 12.0;
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.vertical(|ui| {
                ui.set_width(main_w);
                ui.spacing_mut().item_spacing.y = GAP;
                about_card(ui, env, w, false);
                picks_card(ui, env, w);
            });
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                ui.spacing_mut().item_spacing.y = GAP;
                if w.editing {
                    visibility_card(ui, env, w);
                }
                info_card(ui, env, w, false);
                groups_card(ui, env, w);
                notes_card(ui, env, w);
            });
        }
    });
    ui.add_space(INSET + 6.0);
}

/// About (sl_about_text): read with its links, or written in edit mode.
fn about_card(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let profile = world.profiles.get(&w.id);
    let head = (!compact).then_some(("quote-filled", aurora_i18n::tr!("ui-profile-aurora-about")));
    card(ui, p, head, None, false, |ui| {
        if profile.is_none() {
            loading(ui, p);
            return;
        }
        if w.editing {
            about_editor(ui, env, w);
            return;
        }
        if w.about.text.trim().is_empty() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-profile-aurora-no-about"))
                    .size(12.5)
                    .color(p.muted_dim),
            );
        } else {
            ui.spacing_mut().item_spacing.y = 4.0;
            rich(ui, p, env.emoji, world, env.want_names, &w.about.text);
        }
    });
}

/// The editor of About: Write / Preview and the count of characters.
fn about_editor(ui: &mut egui::Ui, env: &mut Env, w: &mut Window) {
    let p = env.p;
    ui.horizontal(|ui| {
        let options = [
            (aurora_i18n::tr!("ui-profile-aurora-write"), true),
            (aurora_i18n::tr!("ui-profile-aurora-preview"), true),
        ];
        let mut selected = usize::from(w.about_preview);
        if widgets::segmented(ui, p, &mut selected, &options) {
            w.about_preview = selected == 1;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let count = aurora_i18n::tr!(
                "ui-profile-aurora-char-count",
                count = aurora_i18n::integer(w.about.text.chars().count() as i64),
                max = aurora_i18n::integer(ABOUT_MAX as i64)
            );
            ui.label(RichText::new(count).size(11.0).color(p.muted_dim));
        });
    });
    ui.add_space(4.0);
    if w.about_preview {
        rich(ui, p, env.emoji, env.world, env.want_names, &w.about.text);
    } else {
        ui.add(
            egui::TextEdit::multiline(&mut w.about.text)
                .desired_width(ui.available_width())
                .desired_rows(4)
                .char_limit(ABOUT_MAX)
                .font(FontId::proportional(12.5)),
        );
    }
}

/// A row of a key / value card.
fn kv(ui: &mut egui::Ui, p: &Palette, key: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.allocate_ui_with_layout(Vec2::new(84.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(84.0);
            ui.label(RichText::new(key).size(12.5).color(p.muted));
        });
        ui.vertical(|ui| {
            ui.set_min_height(20.0);
            value(ui);
        });
    });
}

/// Info: the account, the partner, a friend's rights (a stranger's "Add as
/// friend"), the key (Account in edit mode).
fn info_card(ui: &mut egui::Ui, env: &mut Env, w: &Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let id = w.id;
    let profile = world.profiles.get(&id);
    let head = (!compact).then(|| {
        if w.editing {
            ("info-circle-filled", aurora_i18n::tr!("ui-profile-aurora-account"))
        } else {
            ("info-circle-filled", aurora_i18n::tr!("ui-profile-aurora-infos"))
        }
    });
    card(ui, p, head, None, false, |ui| {
        let Some(pr) = profile else {
            loading(ui, p);
            return;
        };
        ui.spacing_mut().item_spacing.y = 7.0;
        kv(ui, p, aurora_i18n::tr!("ui-profile-aurora-account"), |ui| {
            ui.label(RichText::new(profiles::account_line(pr)).size(12.5).color(p.ink));
        });
        kv(ui, p, aurora_i18n::tr!("ui-profile-aurora-partner"), |ui| {
            if pr.partner.is_nil() {
                ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.5).color(p.muted));
            } else {
                partner_chip(ui, env, pr.partner);
            }
        });
        let friend = world.social.friends.iter().find(|fr| fr.id == id);
        match friend {
            // can be hidden (Preferences › Aurora › Profiles): the rights
            // stay in Contacts
            Some(fr) if env.friend_rights => kv(ui, p, aurora_i18n::tr!("ui-profile-aurora-permissions"), |ui| {
                rights_line(ui, env, id, fr)
            }),
            // a stranger's way to it; the compact header has its own
            None if !compact && id != world.agent_id => kv(ui, p, aurora_i18n::tr!("ui-profile-aurora-friendship"), |ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "user-plus",
                    aurora_i18n::tr!("ui-profile-aurora-add-friend"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 24.0),
                )
                .clicked()
                {
                    context::request(ui.ctx(), CtxAction::AddFriend(id));
                }
            }),
            _ => {}
        }
        if !compact && !w.editing {
            kv(ui, p, aurora_i18n::tr!("ui-profile-aurora-key"), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let s = id.to_string();
                    let short = format!("{}…{}", &s[..8], &s[s.len() - 4..]);
                    ui.label(RichText::new(short).monospace().size(11.0).color(p.muted))
                        .on_hover_text(&s);
                    if widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("ui-profile-aurora-copy-key"), true) {
                        ui.ctx().copy_text(s);
                    }
                });
            });
        }
    });
}

/// The partner: their picture and name, a click opens their profile.
fn partner_chip(ui: &mut egui::Ui, env: &mut Env, partner: Uuid) {
    let (p, world) = (env.p, env.world);
    world.social.avatar_names.want(&partner);
    env.wanted_pics.insert(partner);
    let name = display_name(world, &partner);
    let font = semibold(ui, 12.5);
    let galley = ui.painter().layout_no_wrap(name.clone(), font, p.ink);
    let size = Vec2::new(2.0 + 20.0 + 6.0 + galley.size().x + 9.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { p.raised } else { p.bar };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    let disc = Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), Vec2::splat(20.0));
    widgets::avatar_disc(ui, p, disc, env.pics.get(&partner), &name, false);
    widgets::paint_ink_left(ui.painter(), galley, pos2(rect.left() + 28.0, rect.center().y), p.ink);
    if resp
        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        super::request_open(ui.ctx(), partner);
    }
}

/// The first three favorite places as cards, and the way to all of them.
fn picks_card(ui: &mut egui::Ui, env: &mut Env, w: &mut Window) {
    let (p, world) = (env.p, env.world);
    let Some(pr) = world.profiles.get(&w.id) else {
        return;
    };
    if pr.picks.is_empty() {
        return;
    }
    let all = aurora_i18n::tr!("ui-profile-aurora-see-all", count = pr.picks.len() as i64);
    let mut open: Option<Option<Uuid>> = None;
    let clicked = card(
        ui,
        p,
        Some(("map-pin-filled", aurora_i18n::tr!("ui-profile-aurora-tab-picks"))),
        Some(Link {
            icon: Some("chevron-right"),
            ..Link::new(&all)
        }),
        false,
        |ui| {
            let shown: Vec<Uuid> = pr.picks.iter().take(3).map(|(id, _)| *id).collect();
            let columns = if ui.available_width() < 360.0 { 2 } else { 3 };
            let gap = 8.0;
            let col_w = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                for pick in shown {
                    if pick_card(ui, env, w, pick, col_w) {
                        open = Some(Some(pick));
                    }
                }
            });
        },
    );
    if clicked {
        open = Some(None);
    }
    if let Some(pick) = open {
        w.tab = TAB_PICKS;
        if pick.is_some() {
            w.pick_sel = pick;
            w.pick_edit = None;
        }
    }
}

/// One favorite place: its snapshot with the rating of its region, its
/// name, place and description; "TP" over the snapshot under the pointer.
/// True when it is clicked (to show it in its tab).
fn pick_card(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, pick: Uuid, width: f32) -> bool {
    let (p, world) = (env.p, env.world);
    let info = world.profiles.picks.get(&pick);
    if info.is_none() {
        w.ask(pick, NetCommand::PickInfoRequest { creator: w.id, pick }, env.actions);
    }
    let parcel = info.and_then(|i| {
        if i.parcel.is_nil() {
            return None;
        }
        let pa = world.profiles.parcels.get(&i.parcel);
        if pa.is_none() {
            w.ask(i.parcel, NetCommand::ParcelInfoRequest(i.parcel), env.actions);
        }
        pa
    });
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 180.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, p.bar);
    let img = Rect::from_min_size(rect.min, Vec2::new(width, 92.0));
    let snapshot = info.map(|i| i.snapshot).filter(|s| !s.is_nil());
    if let Some(s) = snapshot {
        env.wanted_images.insert(s);
    }
    let top = egui::CornerRadius {
        nw: radius.nw,
        ne: radius.ne,
        sw: 0,
        se: 0,
    };
    match snapshot.and_then(|s| env.images.get(&s)) {
        Some(t) => {
            let [tw, th] = t.size();
            let aspect = tw.max(1) as f32 / th.max(1) as f32;
            let want = img.width() / img.height();
            let uv = if aspect > want {
                let u = want / aspect;
                Rect::from_min_max(pos2(0.5 - u / 2.0, 0.0), pos2(0.5 + u / 2.0, 1.0))
            } else {
                let v = aspect / want;
                Rect::from_min_max(pos2(0.0, 0.5 - v / 2.0), pos2(1.0, 0.5 + v / 2.0))
            };
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), img.size()))
                .uv(uv)
                .corner_radius(top)
                .paint_at(ui, img);
        }
        None => {
            ui.painter().rect_filled(img, top, p.raised.gamma_multiply(0.6));
            paint_icon(ui, "map-pin-filled", img.center(), 26.0, p.muted_dim);
        }
    }
    if let Some(pa) = parcel {
        let maturity = match pa.flags {
            f if f & 0x2 != 0 => aurora_i18n::text("common-adult"),
            f if f & 0x1 != 0 => aurora_i18n::text("common-moderate"),
            _ => aurora_i18n::text("common-general"),
        };
        let badge = Rect::from_min_size(img.min + Vec2::splat(7.0), Vec2::splat(15.0));
        widgets::paint_maturity_badge(ui.painter(), p, maturity, badge, widgets::inner_radius(ui.visuals()));
    }
    let x = rect.left() + 9.0;
    let text_w = width - 18.0;
    let name = info.map(|i| i.name.clone()).unwrap_or_else(|| {
        world
            .profiles
            .get(&w.id)
            .and_then(|pr| pr.picks.iter().find(|(id, _)| *id == pick))
            .map(|(_, n)| n.clone())
            .unwrap_or_default()
    });
    let mut y = img.bottom() + 7.0;
    widgets::paint_elided(ui, &name, pos2(x, y + 8.0), text_w, semibold(ui, 12.5), p.ink);
    y += 17.0;
    let place = match (parcel.map(|pa| pa.name.as_str()), info.map(|i| i.sim_name.as_str())) {
        (Some(pa), Some(sim)) if !pa.is_empty() && !sim.is_empty() => format!("{pa} · {sim}"),
        (_, Some(sim)) => sim.to_owned(),
        (Some(pa), None) => pa.to_owned(),
        _ => String::new(),
    };
    widgets::paint_elided(ui, &place, pos2(x, y + 7.0), text_w, FontId::proportional(11.5), p.muted);
    y += 18.0;
    if let Some(i) = info {
        let mut job = egui::text::LayoutJob::simple(i.desc.clone(), FontId::proportional(11.5), p.muted_dim, text_w);
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 3,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, y), galley, p.muted_dim);
    }
    // "TP" over the snapshot, while the pointer is on the card
    let mut tp = false;
    if let Some(i) = info.filter(|i| i.pos_global != glam::DVec3::ZERO)
        && ui.rect_contains_pointer(rect)
    {
        let b = Rect::from_min_size(pos2(img.right() - 7.0 - 52.0, img.bottom() - 7.0 - 24.0), Vec2::new(52.0, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(b));
        if widgets::action_button(&mut child, p, "send-filled", "TP", ActionKind::Primary, b.size())
            .on_hover_text(aurora_i18n::tr!("common-teleport-2"))
            .clicked()
        {
            env.actions.push(ProfileAction::Teleport(i.pos_global));
            tp = true;
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked() && !tp
}

/// Groups: insignia, name, whether we are in it too; a click opens the
/// group's profile, a double click its chat when we are a member.
fn groups_card(ui: &mut egui::Ui, env: &mut Env, w: &Window) {
    let (p, world) = (env.p, env.world);
    let Some(pr) = world.profiles.get(&w.id) else {
        return;
    };
    let own = w.id == world.agent_id;
    // in edit mode the groups shown are a setting of Visibility (the mock-up)
    if w.editing {
        return;
    }
    let title = aurora_i18n::tr!("ui-profile-aurora-groups", count = pr.groups.len() as i64);
    // inviting to a group is not ported yet: the link waits greyed
    let invite = (!own).then(|| Link {
        text: aurora_i18n::tr!("ui-profile-aurora-invite"),
        icon: Some("user-plus"),
        waiting: Some(aurora_i18n::tr!("ui-profile-invite-to-group-coming-soon")),
    });
    card(ui, p, Some(("users-group", &title)), invite, false, |ui| {
        if pr.groups.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.5).color(p.muted));
            return;
        }
        let mut groups: Vec<_> = pr.groups.iter().collect();
        groups.sort_by_key(|g| g.name.to_lowercase());
        ui.spacing_mut().item_spacing.y = 0.0;
        for g in groups {
            let member = world.groups.is_member(&g.id);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, ui.visuals().window_corner_radius, p.raised.gamma_multiply(0.4));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if !g.insignia.is_nil() {
                env.wanted_images.insert(g.insignia);
            }
            let tile = Rect::from_min_size(pos2(rect.left(), rect.center().y - 13.0), Vec2::splat(26.0));
            widgets::avatar_picture(ui, p, tile, env.images.get(&g.insignia), &g.name, false);
            let mut name_w = rect.width() - 34.0;
            if member && !own {
                let tag = aurora_i18n::tr!("ui-profile-aurora-in-common");
                let galley = ui.painter().layout_no_wrap(tag.to_owned(), semibold(ui, 10.5), p.teal);
                let tw = galley.size().x + 12.0;
                let t = Rect::from_min_size(pos2(rect.right() - tw, rect.center().y - 8.0), Vec2::new(tw, 16.0));
                ui.painter()
                    .rect_filled(t, ui.visuals().window_corner_radius, p.teal.gamma_multiply(0.14));
                widgets::paint_ink_left(ui.painter(), galley, pos2(t.left() + 6.0, t.center().y), p.teal);
                name_w -= tw + 8.0;
            }
            widgets::paint_elided(
                ui,
                &g.name,
                pos2(tile.right() + 8.0, rect.center().y),
                name_w,
                semibold(ui, 12.5),
                p.ink,
            );
            let tip = if member {
                aurora_i18n::tr!("ui-profile-aurora-group-tip-member")
            } else {
                aurora_i18n::tr!("ui-profile-aurora-group-tip")
            };
            let resp = resp.on_hover_text(tip);
            if resp.double_clicked() && member {
                env.actions.push(ProfileAction::GroupChat(g.id));
            } else if resp.clicked() {
                group_profile::request_open(ui.ctx(), g.id);
            }
        }
    });
}

/// Our private notes on someone, a click away from the Notes tab.
fn notes_card(ui: &mut egui::Ui, env: &Env, w: &mut Window) {
    if w.id == env.world.agent_id {
        return;
    }
    let p = env.p;
    let text = w.notes.text.trim().to_owned();
    let clicked = card(
        ui,
        p,
        Some(("lock-filled", aurora_i18n::tr!("ui-profile-aurora-private-notes"))),
        Some(Link::new(aurora_i18n::tr!("ui-profile-aurora-edit-note"))),
        true,
        |ui| {
            if text.is_empty() {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-aurora-no-note"))
                        .size(12.5)
                        .color(p.muted_dim),
                );
            } else {
                ui.label(RichText::new(&text).size(12.5).color(p.ink));
            }
        },
    );
    if clicked {
        w.tab = TAB_NOTES;
    }
}

/// Our profile in edit mode: who finds it, how much of the birth date shows.
fn visibility_card(ui: &mut egui::Ui, env: &mut Env, w: &Window) {
    let (p, world) = (env.p, env.world);
    let Some(pr) = world.profiles.get(&w.id) else {
        return;
    };
    card(
        ui,
        p,
        Some(("eye-filled", aurora_i18n::tr!("ui-profile-aurora-visibility"))),
        None,
        false,
        |ui| {
            let mut publish = pr.allow_publish();
            if switch_row(ui, p, "search-filled", aurora_i18n::tr!("common-show-in-search"), &mut publish) {
                env.actions.push(ProfileAction::Save {
                    target: w.id,
                    data: llsd_map! { "allow_publish" => publish },
                });
            }
            // only offered by servers that support it, to accounts older than a year
            if let (Some(hide), Some(born)) = (pr.hide_age, pr.born)
                && now() - born > 365.0 * 86_400.0
            {
                let mut full = !hide;
                if switch_row(ui, p, "cake", aurora_i18n::tr!("ui-profile-show-full-birth-date"), &mut full) {
                    env.actions.push(ProfileAction::Save {
                        target: w.id,
                        data: llsd_map! { "hide_age" => !full },
                    });
                }
            }
        },
    );
}

/// An icon, a label and a switch at the right edge.
fn switch_row(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 20.0), Sense::hover());
        paint_icon(ui, icon, rect.center(), 15.0, p.muted);
        let label_w = (ui.available_width() - widgets::SWITCH_SIZE.x - 8.0).max(40.0);
        ui.allocate_ui_with_layout(
            Vec2::new(label_w, 20.0),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                ui.set_width(label_w);
                ui.add(egui::Label::new(RichText::new(label).size(12.5).color(p.ink)).wrap());
            },
        );
        changed = widgets::switch(ui, p, on).changed();
    });
    changed
}

/// "Unsaved changes (bio, 1 place)": while editing, Cancel puts the texts
/// back and Save sends them; when closing, Cancel keeps the window, Ignore
/// closes it without saving and Save closes it after saving
/// (LLFloaterProfile's ProfileUnsavedChanges).
fn unsaved_bar(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, closing: bool) {
    let p = env.p;
    let parts = unsaved_parts(w);
    let text = if parts.is_empty() {
        aurora_i18n::tr!("ui-profile-there-are-unsaved-changes").to_owned()
    } else {
        aurora_i18n::tr!("ui-profile-aurora-unsaved", parts = parts.join(", "))
    };
    let resp = egui::Frame::new()
        .fill(p.amber.gamma_multiply(0.10))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 13,
            right: 10,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 26.0), Sense::hover());
                paint_icon(ui, "alert-triangle-filled", rect.center(), 14.0, p.amber);
                ui.label(RichText::new(text).font(semibold(ui, 12.5)).color(p.amber));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let size = Vec2::new(0.0, 26.0);
                    if widgets::action_button(
                        ui,
                        p,
                        "device-floppy-filled",
                        aurora_i18n::tr!("common-save"),
                        ActionKind::Primary,
                        size,
                    )
                    .clicked()
                    {
                        save_all(w, env.actions);
                        if closing {
                            w.open = false;
                        }
                        w.confirm_close = false;
                    }
                    if closing && widgets::action_button(ui, p, "", aurora_i18n::tr!("common-ignore"), ActionKind::Flat, size).clicked() {
                        revert(w);
                        w.open = false;
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        if closing {
                            w.confirm_close = false;
                        } else {
                            revert(w);
                        }
                    }
                });
            });
        })
        .response;
    let r = resp.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom())),
        egui::CornerRadius::ZERO,
        p.amber,
    );
}

impl Edit {
    #[cfg(test)]
    fn with(text: &str) -> Edit {
        Edit {
            text: text.into(),
            base: text.into(),
        }
    }
}

#[cfg(test)]
mod tests;
