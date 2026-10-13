//! "People" floater (nearby / friends / groups / blocked).

use super::Panels;
use super::context::{self, AvatarList};
use super::menu;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::mutes::{MuteType, flag};
use crate::world::social::Friend;
use egui::{RichText, Vec2};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

mod radar;
pub use radar::{RadarData, RadarSettings, age_days, radar_name};

pub enum PeopleAction {
    OpenIm(Uuid),
    /// "View profile" (menu_people_nearby.xml / friends, Avatar.Profile).
    Profile(Uuid),
    OfferTeleport(Uuid),
    /// Open the chat of one of our groups.
    GroupChat(Uuid),
    /// Remove a block list entry (by id, or by name for legacy entries).
    Unblock(Uuid, String),
    /// Block an object by its name (LLFloaterGetBlockedObjectName).
    BlockByName(String),
    /// Mute (true) / unmute one property of an entry (mutes::flag).
    BlockFlag {
        id: Uuid,
        name: String,
        kind: MuteType,
        flag: u32,
        on: bool,
    },
    /// Double click on an avatar of the radar that is not drawn
    /// (ZoomToAvatarNotPossible).
    TooFarToZoom,
    /// "+" of the friends tab: choose a resident to offer friendship to
    /// (People.AddFriendWizard).
    PickFriend,
    /// The bell of a group: receive its notices or not.
    GroupNotices(Uuid, bool),
}

/// Options of the friends tab (menu_people_friends_view.xml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FriendsSettings {
    /// FriendsSortOrder: `SORT_NAME`, `SORT_USERNAME` or `SORT_STATUS`.
    pub sort: u8,
    /// FriendsListShowIcons: the profile pictures.
    pub pictures: bool,
    /// FriendsListShowPermissions: the rights each friend gave us.
    pub permissions: bool,
}

impl Default for FriendsSettings {
    fn default() -> Self {
        FriendsSettings {
            sort: SORT_NAME,
            pictures: true,
            permissions: true,
        }
    }
}

/// LLAvatarList sort orders of the friends (E_SORT_BY_NAME, _USERNAME,
/// _STATUS).
const SORT_NAME: u8 = 0;
const SORT_USERNAME: u8 = 1;
const SORT_STATUS: u8 = 2;

/// The friends in the order of the tab: by name as it is shown, by username,
/// or online first then by name (LLAvatarItemNameComparator,
/// LLAvatarItemUserNameComparator, LLAvatarItemStatusComparator).
fn sort_friends(friends: &mut [Friend], sort: u8, name: impl Fn(&Uuid) -> String, username: impl Fn(&Uuid) -> String) {
    friends.sort_by_cached_key(|f| {
        let key = if sort == SORT_USERNAME { username(&f.id) } else { name(&f.id) };
        (sort == SORT_STATUS && !f.online, key.to_lowercase())
    });
}

/// Where the window opens before the user places it: the top left corner of
/// the work area (`layout::work_area`), level with the mini-map window,
/// which a first launch opens in the opposite corner
/// (`minimap::default_rect`).
pub(crate) fn default_pos(work: egui::Rect) -> egui::Pos2 {
    work.left_top() + Vec2::splat(super::layout::GAP)
}

/// Its size then. Firestorm gives 400 x 570 to floater_people.xml and
/// 400 x 440 to floater_fs_radar.xml, with the embedded map and a smaller
/// font; here the map is hidden by default (`RadarSettings::show_minimap`):
/// the width holds the ten columns of the radar and, after the profile
/// picture, a name of some eighteen characters, the height a dozen avatars.
pub(crate) const DEFAULT_SIZE: Vec2 = Vec2::new(462.0, 420.0);

#[derive(Default)]
pub struct PeopleUi {
    pub filter: String,
    pub friends_tab: usize,
    /// Object name typed in "Block by name".
    pub block_name: String,
    /// Avatars selected in the radar.
    pub radar_selected: Vec<Uuid>,
    /// Residents whose profile picture a row on screen waits for (drained by
    /// the viewer, which asks for the profile and the texture).
    pub wanted_pics: HashSet<Uuid>,
    /// The ones already asked for, and the pace of the new ones.
    pics_asked: HashSet<Uuid>,
    pic_budget: super::nameplate::PictureBudget,
}

impl PeopleUi {
    /// A row on screen has no picture yet: ask for it, a few new residents
    /// a second (a crowded region or a long friend list would send as many
    /// profile requests at once), then again every frame until it is there.
    fn want_pic(&mut self, id: Uuid, now: f64) {
        if self.pics_asked.contains(&id) || self.pic_budget.take(now) {
            self.pics_asked.insert(id);
            self.wanted_pics.insert(id);
        }
    }
}

/// Nearby avatars, nearest first: the loaded ones at their exact place, the
/// others from the coarse locations (the radar's sightings).
pub fn nearby(world: &World) -> Vec<(Uuid, String, Vec3, f32)> {
    let me = world.agent.position;
    let mut out: Vec<(Uuid, String, Vec3, f32)> = crate::world::radar::sightings(world)
        .into_iter()
        .map(|s| {
            let name = world.person_name(&s.id).unwrap_or_else(|| "…".into());
            // without an altitude, the distance on the ground
            let distance = if s.altitude_known {
                s.position.distance(me)
            } else {
                s.position.truncate().distance(me.truncate())
            };
            (s.id, name, s.position, distance)
        })
        .collect();
    out.sort_by(|a, b| a.3.total_cmp(&b.3));
    out
}

pub fn people_window(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    panels: &mut Panels,
    st: &mut PeopleUi,
    radar: &mut RadarData,
    friends: &mut FriendsSettings,
    // the mini-map renderer (LLNetMap in the Nearby tab)
    map: &mut dyn FnMut(&mut egui::Ui, Vec2),
) -> Vec<PeopleAction> {
    let mut actions = Vec::new();
    let mut open = panels.people;
    let help = aurora_i18n::tr!("ui-people-nearby-avatars-and-friends-list");
    let pos = default_pos(super::layout::work_area(ctx));
    let floater = Floater::new("people", aurora_i18n::tr!("common-people"), pos, DEFAULT_SIZE).help(help);
    floater.show(ctx, p, &mut open, |ui| {
        let mut tab = panels.people_tab as usize;
        widgets::tabs(
            ui,
            p,
            &mut tab,
            &[
                (aurora_i18n::tr!("ui-people-near-you"), true),
                (aurora_i18n::tr!("common-friends"), true),
                (aurora_i18n::tr!("common-groups"), true),
                (aurora_i18n::tr!("common-recent"), false),
                (aurora_i18n::tr!("common-blocked"), true),
            ],
        );
        panels.people_tab = tab as u8;
        ui.add_space(4.0);
        match panels.people_tab {
            0 => {
                // the radar lays out its own filter, beside its buttons
                let mut blocked = false;
                radar::tab(ui, p, world, radar, st, map, &mut actions, &mut blocked);
                if blocked {
                    panels.people_tab = 4;
                }
            }
            1 => friends_tab(ui, p, world, st, radar.pics, friends, &mut actions),
            _ => {
                widgets::search_field(ui, &mut st.filter, aurora_i18n::tr!("ui-people-filter"), ui.available_width());
                ui.add_space(4.0);
                let filter = st.filter.trim().to_lowercase();
                if panels.people_tab == 2 {
                    groups_tab(ui, p, world, &filter, &mut actions);
                } else {
                    blocked_tab(ui, p, world, st, &filter, &mut actions);
                }
            }
        }
    });
    panels.people = open;
    actions
}

/// View and sort options of the friends (menu_people_friends_view.xml,
/// without the conversation log, which has its own window).
fn friends_view_menu(ui: &mut egui::Ui, p: &Palette, s: &mut FriendsSettings) {
    for (sort, label) in [
        (SORT_NAME, aurora_i18n::tr!("people-friends-sort-name")),
        (SORT_USERNAME, aurora_i18n::tr!("people-friends-sort-username")),
        (SORT_STATUS, aurora_i18n::tr!("people-friends-sort-status")),
    ] {
        if menu::check(ui, p, "", label, s.sort == sort, true) {
            s.sort = sort;
        }
    }
    menu::separator(ui, p);
    menu::toggle(ui, p, "", aurora_i18n::tr!("people-friends-view-pictures"), &mut s.pictures);
    menu::toggle(ui, p, "", aurora_i18n::tr!("people-friends-view-permissions"), &mut s.permissions);
}

/// Friends (the friends tab of LLPanelPeople, panel_people.xml): the filter
/// with the view / sort options and the friendship offer, then the friends
/// online and all of them. Firestorm's button hiding our online status from
/// every friend is the "Invisible" status of the top bar; removing a
/// friend is in the menu of its row.
fn friends_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut PeopleUi,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    settings: &mut FriendsSettings,
    actions: &mut Vec<PeopleAction>,
) {
    // laid out from the right, the filter takes what the buttons leave
    let line = Vec2::new(ui.available_width(), radar::TOOL);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let add = ui
            .add(radar::tool_button(p, "user-plus", true))
            .on_hover_text(aurora_i18n::tr!("people-friends-add"));
        if add.clicked() {
            actions.push(PeopleAction::PickFriend);
        }
        radar::tool_menu(ui, p, "arrows-sort", aurora_i18n::tr!("people-view-sort-options"), true, |ui| {
            friends_view_menu(ui, p, settings);
        });
        widgets::search_field(ui, &mut st.filter, aurora_i18n::tr!("ui-people-filter"), ui.available_width());
    });
    ui.add_space(4.0);
    let filter = st.filter.trim().to_lowercase();
    let mut friends = world.social.friends.clone();
    sort_friends(
        &mut friends,
        settings.sort,
        |id| world.social.name_of(id),
        |id| radar_name(world, id, 1),
    );
    let online = friends.iter().filter(|f| f.online).count();
    let l1 = aurora_i18n::tr!("ui-people-online", online = format!("{}", online));
    let l2 = aurora_i18n::tr!("ui-people-all", value0 = format!("{}", friends.len()));
    widgets::tabs(ui, p, &mut st.friends_tab, &[(&l1, true), (&l2, true)]);
    ui.add_space(2.0);
    let now = ui.input(|i| i.time);
    let all = st.friends_tab == 1;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut shown = 0;
        for f in friends.iter().filter(|f| all || f.online) {
            let name = world.social.name_of(&f.id);
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                continue;
            }
            shown += 1;
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                if settings.pictures {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radar::PICTURE), egui::Sense::hover());
                    let pic = pics.get(&f.id);
                    if pic.is_none() && ui.is_rect_visible(rect) {
                        st.want_pic(f.id, now);
                    }
                    widgets::avatar_picture(ui, p, rect, pic, &name, !f.online);
                }
                widgets::status_dot(ui, widgets::online_color(p, f.online));
                let col = if f.online { p.indigo_light } else { p.muted };
                // complete names can be long: cut before the rights
                // and buttons (full name on hover)
                let buttons = if settings.permissions { 140.0 } else { 80.0 };
                let w = (ui.available_width() - buttons).max(60.0);
                let r = ui
                    .allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(name).size(13.0).color(col))
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    })
                    .inner;
                if r.double_clicked() {
                    actions.push(PeopleAction::OpenIm(f.id));
                }
                // menu_fs_contacts_friends.xml
                menu::context_menu(&r, p, |ui| context::avatar_list_menu(ui, p, world, f.id, AvatarList::Friend));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // the icons these actions have in the avatar menus
                    if widgets::icon_button(ui, p, "send-filled", aurora_i18n::tr!("common-offer-teleport"), f.online) {
                        actions.push(PeopleAction::OfferTeleport(f.id));
                    }
                    if widgets::icon_button(ui, p, "message-filled", aurora_i18n::tr!("ui-people-instant-message-im"), true) {
                        actions.push(PeopleAction::OpenIm(f.id));
                    }
                    if widgets::icon_button(ui, p, "user-filled", aurora_i18n::tr!("common-view-profile"), true) {
                        actions.push(PeopleAction::Profile(f.id));
                    }
                    if !settings.permissions {
                        return;
                    }
                    ui.add_space(2.0);
                    // Rights granted to me: 1 online status, 2 map location, 4 modify
                    // objects, with the icons of the contacts window
                    for (bit, icon, tip) in [
                        (4, "pencil-filled", aurora_i18n::tr!("ui-people-i-can-modify-their-objects")),
                        (2, "map-pin-filled", aurora_i18n::tr!("extra-i-can-see-them-on-the-map")),
                        (1, "eye-filled", aurora_i18n::tr!("ui-people-i-can-see-their-online-status")),
                    ] {
                        let on = f.rights_has & bit != 0;
                        let (cell, hover) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                        if let Some(texture) = super::icons::global(icon) {
                            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                            let tint = if on { p.violet_light } else { p.muted_dim.gamma_multiply(0.6) };
                            ui.painter().image(texture.id(), cell.shrink(0.5), uv, tint);
                        }
                        hover.on_hover_text(tip);
                    }
                });
            });
        }
        if shown == 0 {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-people-nobody-here-yet"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
    });
}

/// Our groups (the groups tab of LLPanelPeople): a row has the bell of its
/// notices and the group's profile, nothing else (asked by the human;
/// Firestorm's bell only opens the notices of the profile, here it turns
/// them on and off as the profile's "Receive group notices" does). The
/// chat is a double click away, the rest in the menu of the row. Under the
/// list, how many groups we are in and how many we can still join
/// (FSCommon::populateGroupCount).
fn groups_tab(ui: &mut egui::Ui, p: &Palette, world: &World, filter: &str, actions: &mut Vec<PeopleAction>) {
    let groups = world.groups.sorted();
    let count = super::contacts::group_count_text(groups.len(), super::contacts::group_limit(world));
    // the list over its count: both inside the room the window has, which
    // their own sizes must not push
    let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
    ui.allocate_rect(body, egui::Sense::hover());
    let (list, footer) = body.split_top_bottom_at_y((body.bottom() - COUNT_LINE).max(body.top()));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("people-groups")
            .max_rect(list)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut ui;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if groups.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("ui-people-no-groups-yet")).size(12.0).color(p.muted));
        }
        for g in groups
            .iter()
            .filter(|g| filter.is_empty() || g.name.to_lowercase().contains(filter))
        {
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                let blocked = world.mutes.group_chat_muted(&g.id);
                let col = if blocked { p.muted } else { p.ink };
                let name_w = (ui.available_width() - 56.0).max(60.0);
                let r = ui
                    .allocate_ui_with_layout(Vec2::new(name_w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&g.name).size(13.0).color(col))
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    })
                    .inner;
                let r = if blocked {
                    r.on_hover_text(aurora_i18n::tr!("ui-people-group-chat-blocked"))
                } else {
                    r
                };
                // menu_people_groups.xml
                menu::context_menu(&r, p, |ui| context::group_menu(ui, p, world, g.id));
                if r.double_clicked() {
                    actions.push(PeopleAction::GroupChat(g.id));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // the icon "View info" has in the menu of the row
                    if widgets::icon_button(ui, p, "info-circle-filled", aurora_i18n::tr!("people-group-profile"), true) {
                        super::group_profile::request_open(ui.ctx(), g.id);
                    }
                    let (icon, tip) = if g.accept_notices {
                        ("bell-filled", aurora_i18n::tr!("people-group-notices-on"))
                    } else {
                        ("bell-off", aurora_i18n::tr!("people-group-notices-off"))
                    };
                    if widgets::icon_button(ui, p, icon, tip, true) {
                        actions.push(PeopleAction::GroupNotices(g.id, !g.accept_notices));
                    }
                });
            });
        }
    });
    let at = egui::pos2(footer.left() + 2.0, footer.bottom() - 8.0);
    let font = egui::FontId::proportional(12.0);
    widgets::paint_elided(ui, &count, at, footer.width() - 4.0, font, p.muted);
}

/// Room kept under the list of groups for their count.
const COUNT_LINE: f32 = 22.0;

/// Block list (LLPanelBlockedList): entries, their blocked properties,
/// unblock, block an object by name.
fn blocked_tab(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut PeopleUi, filter: &str, actions: &mut Vec<PeopleAction>) {
    use crate::world::mutes::LoadState;
    let status = match world.mutes.state {
        LoadState::Initial | LoadState::Requested(_) => Some(aurora_i18n::tr!("ui-people-loading-block-list")),
        LoadState::Degraded => Some(aurora_i18n::tr!("ui-people-the-server-did-not-respond-using-the-local-list")),
        LoadState::Loaded => None,
    };
    if let Some(s) = status {
        ui.label(RichText::new(s).size(11.5).color(p.muted));
    }
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 130.0).max(60.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut st.block_name)
                .hint_text(aurora_i18n::tr!("ui-people-exact-object-name"))
                .desired_width(w),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ok = !st.block_name.trim().is_empty();
        if (widgets::flat_button(ui, p, aurora_i18n::tr!("ui-people-block-by-name")).clicked() || enter) && ok {
            actions.push(PeopleAction::BlockByName(st.block_name.trim().to_owned()));
            st.block_name.clear();
        }
    });
    ui.add_space(4.0);
    let entries = world.mutes.entries();
    let w = ui.available_width();
    let title = aurora_i18n::tr!("ui-people-name", value0 = format!("{}", entries.len()));
    widgets::header_row(ui, p, &[(&title, w - 160.0), (aurora_i18n::tr!("common-type"), 70.0), ("", 90.0)]);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if entries.is_empty() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-people-nobody-is-blocked"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
        for m in entries
            .iter()
            .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(filter))
        {
            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                let name = if m.name.is_empty() {
                    aurora_i18n::tr!("common-unnamed")
                } else {
                    m.name.as_str()
                };
                let partial = m.kind != MuteType::ByName && m.flags != 0;
                let name_w = (ui.available_width() - 170.0).max(60.0);
                let r = ui
                    .allocate_ui_with_layout(Vec2::new(name_w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(name).size(13.0).color(if partial { p.muted } else { p.ink }))
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    })
                    .inner;
                let r = if partial {
                    r.on_hover_text(aurora_i18n::tr!("ui-people-partial-block-right-click-for-details"))
                } else {
                    r
                };
                // menu_fs_block_list.xml
                menu::context_menu(&r, p, |ui| {
                    if menu::item(ui, p, "circle-check-filled", aurora_i18n::tr!("common-stop-ignoring")) {
                        actions.push(PeopleAction::Unblock(m.id, m.name.clone()));
                    }
                    if m.kind != MuteType::ByName {
                        for (bit, icon, label) in [
                            (flag::VOICE_CHAT, "microphone-off", aurora_i18n::tr!("ui-people-block-voice")),
                            (flag::TEXT_CHAT, "message-off", aurora_i18n::tr!("ui-people-block-text")),
                            (flag::PARTICLES, "sparkles-filled", aurora_i18n::tr!("common-block-particles")),
                            (flag::OBJECT_SOUNDS, "volume-off", aurora_i18n::tr!("ui-people-block-object-sounds")),
                        ] {
                            // a set flag means "not blocked" (LLMute flags)
                            let mut on = m.flags & bit == 0;
                            if menu::toggle(ui, p, icon, label, &mut on) {
                                actions.push(PeopleAction::BlockFlag {
                                    id: m.id,
                                    name: m.name.clone(),
                                    kind: m.kind,
                                    flag: bit,
                                    on,
                                });
                            }
                        }
                    }
                    menu::separator(ui, p);
                    if menu::item_if(ui, p, "user-filled", aurora_i18n::tr!("common-profile"), m.kind == MuteType::Agent) {
                        actions.push(PeopleAction::Profile(m.id));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("common-unblock")).clicked() {
                        actions.push(PeopleAction::Unblock(m.id, m.name.clone()));
                    }
                    ui.add_sized(
                        [66.0, 18.0],
                        egui::Label::new(RichText::new(m.kind.label()).size(12.0).color(p.muted)),
                    );
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friends_sort_by_name_username_or_status() {
        let friend = |n: u128, online: bool| Friend {
            id: Uuid::from_u128(n),
            online,
            rights_given: 1,
            rights_has: 1,
        };
        // (display name, username)
        let names = [("", ""), ("Zoe", "anna"), ("bob", "zed"), ("Anna", "mike")];
        let name = |id: &Uuid| names[id.as_u128() as usize].0.to_owned();
        let username = |id: &Uuid| names[id.as_u128() as usize].1.to_owned();
        let order = |sort: u8| -> Vec<u128> {
            let mut friends = vec![friend(1, false), friend(2, true), friend(3, false)];
            sort_friends(&mut friends, sort, name, username);
            friends.iter().map(|f| f.id.as_u128()).collect()
        };
        // whatever the case
        assert_eq!(order(SORT_NAME), vec![3, 2, 1]);
        assert_eq!(order(SORT_USERNAME), vec![1, 3, 2]);
        // online first, then by name
        assert_eq!(order(SORT_STATUS), vec![2, 3, 1]);
        // Firestorm's defaults: by name, pictures and rights shown
        let defaults = FriendsSettings::default();
        assert_eq!(defaults.sort, SORT_NAME);
        assert!(defaults.pictures && defaults.permissions);
    }
}
