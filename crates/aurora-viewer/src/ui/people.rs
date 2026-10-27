//! "People" floater. In Aurora's look (the People mock-up v2) it holds every
//! person: Near you (`nearby.rs`), Friends, Groups, Contact sets, Recent and
//! Blocked (`friends.rs`, `groups.rs`, `sets.rs`, `recent.rs`,
//! `blocked.rs`: the tabs of Conversations › Contacts with all their
//! rights and actions, `ui/contacts.rs` reused), each with the sheet of the
//! person held or group clicked (`sheet.rs`); `rows.rs` holds what their rows
//! share. The friends tab is also the Contacts tab of the Conversations
//! workspace (`friends_pane`). In the Legacy look (Preferences › Aurora,
//! `Settings::classic_people`) Near you is the radar table (`radar.rs`) and
//! the other tabs keep their simple lists below.

use super::Panels;
use super::contacts::ContactsUi;
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

mod blocked;
mod friends;
mod groups;
mod nearby;
mod radar;
mod recent;
mod rows;
mod sets;
mod sheet;
pub use radar::{RadarData, RadarSettings, age_days, radar_name};

/// The tabs (`Panels::people_tab`), in Aurora's order: Near you, Friends,
/// Groups, Contact sets, Recent, Blocked. The numbers of the first five are
/// the ones the rest of the viewer opens People on.
pub(crate) const TAB_NEARBY: u8 = 0;
pub(crate) const TAB_FRIENDS: u8 = 1;
pub(crate) const TAB_GROUPS: u8 = 2;
pub(crate) const TAB_RECENT: u8 = 3;
pub(crate) const TAB_BLOCKED: u8 = 4;
pub(crate) const TAB_SETS: u8 = 5;
const AURORA_TABS: [u8; 6] = [TAB_NEARBY, TAB_FRIENDS, TAB_GROUPS, TAB_SETS, TAB_RECENT, TAB_BLOCKED];

/// Search every name with a trimmed, lowercase filter, independently of the
/// row's display preferences. Unlike
/// LLAvatarList::getNameForDisplay (indra/newview/llavatarlist.cpp), People
/// deliberately also finds hidden username/legacy forms (#1003). `get` keeps
/// contact aliases and RLVa anonymity; never search the unfiltered cache.
pub(super) fn matches_name(world: &World, id: &Uuid, shown: &str, filter: &str) -> bool {
    filter.is_empty()
        || shown.to_lowercase().contains(filter)
        || world.social.avatar_names.get(id).is_some_and(|name| {
            name.display_name.to_lowercase().contains(filter)
                || name.username.to_lowercase().contains(filter)
                || name
                    .user_name_for_display(&crate::world::names::NameOptions {
                        trim_resident: false,
                        ..world.social.avatar_names.options
                    })
                    .to_lowercase()
                    .contains(filter)
        })
}

/// Id of the window.
pub(crate) const WINDOW: &str = "people";

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
    /// The switch of the right to modify our objects in a sheet: asked
    /// first (GrantModifyRights / RevokeModifyRights).
    ModifyRights {
        friend: Uuid,
        rights: i32,
        grant: bool,
    },
    /// "Join..." of the groups tab: the group search.
    SearchGroups,
    /// A group's sheet opened: its titles, members and notices are asked
    /// for (LLGroupMgr, as the group profile does).
    GroupData(Uuid),
    /// The title we wear in a group (sendGroupTitleUpdate).
    GroupTitle {
        group: Uuid,
        role: Uuid,
    },
    /// "Show in my profile" of a group's sheet (SetGroupAcceptNotices).
    GroupFlags {
        group: Uuid,
        notices: bool,
        profile: bool,
    },
    /// "Invite" of a group's sheet (LLGroupActions::invite).
    InviteToGroup(Uuid),
}

/// What a sheet asks of the contact list's dialogs (they are shown by the
/// app, whichever window asked).
pub(super) enum Ask {
    RemoveFromSet(String, Vec<Uuid>),
    NewSet,
}

/// What the tabs of Aurora's look use, gathered.
pub(super) struct Ctx<'a> {
    pub p: &'a Palette,
    pub world: &'a World,
    pub contacts: &'a mut ContactsUi,
    pub friends: &'a mut FriendsSettings,
    /// Group insignias.
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub actions: &'a mut Vec<PeopleAction>,
    /// Conversations keeps its sheet inside the embedded Contacts pane.
    pub embedded_contacts: bool,
    /// A row click can open the sheet when the inline pane fits both sides.
    pub contact_sheet_on_click: bool,
}

/// A tab's list side: (ui, context, radar, state, beside a sheet).
type ListFn = fn(&mut egui::Ui, &mut Ctx<'_>, &RadarData<'_>, &mut PeopleUi, bool);
/// A tab's sheet of `id`; true when it was closed.
type SheetFn = fn(&mut egui::Ui, &mut Ctx<'_>, &RadarData<'_>, &mut PeopleUi, Uuid) -> bool;

/// A tab's list, and beside it the sheet of the person held or group clicked
/// (`nearby::areas`: the window widens for it before its frame).
fn split(ui: &mut egui::Ui, c: &mut Ctx, data: &RadarData, st: &mut PeopleUi, salt: &str, list: ListFn, sheet: SheetFn) {
    let persistent_sheet = c.embedded_contacts && c.contact_sheet_on_click;
    let (list_rect, sheet_rect) = nearby::areas(ui, st, persistent_sheet);
    if let Some(rect) = list_rect {
        let beside = sheet_rect.is_some();
        list(&mut nearby::list_ui(ui, rect, salt, beside), c, data, st, beside);
    }
    if let Some(rect) = sheet_rect {
        let mut child = nearby::list_ui(ui, rect, "people-sheet", false);
        if let Some(id) = st.sheet {
            if sheet(&mut child, c, data, st, id) {
                nearby::close_sheet(st);
            }
        } else if persistent_sheet {
            sheet::empty(&mut child, c.p);
        }
    }
}

/// The sheet of a person, from any tab but Near you.
fn person_sheet(ui: &mut egui::Ui, c: &mut Ctx, data: &RadarData, st: &mut PeopleUi, id: Uuid) -> bool {
    sheet::show(ui, c.p, c.world, data, st, id, false, !c.contact_sheet_on_click, c.actions)
}

/// The row whose quick actions' menu is open keeps them shown.
fn keep_row_menu(st: &mut PeopleUi, id: Uuid, open: bool) {
    if open {
        st.row_menu = Some(id);
    } else if st.row_menu == Some(id) {
        st.row_menu = None;
    }
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
            // online first, as the contact list always sorts them
            // (FSFloaterContacts::sortFriendList) and as the mock-up's
            // friends tab opens
            sort: SORT_STATUS,
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

/// Its size in Aurora's look, the mock-up's: a dozen rich rows.
pub(crate) const AURORA_SIZE: Vec2 = Vec2::new(520.0, 560.0);

#[derive(Default)]
pub struct PeopleUi {
    pub filter: String,
    pub friends_tab: usize,
    /// Object name typed in "Block by name".
    pub block_name: String,
    /// Avatars selected in the radar.
    pub radar_selected: Vec<Uuid>,
    /// The row Shift selects from (the last one clicked).
    pub radar_anchor: Option<Uuid>,
    /// Residents whose profile picture a row on screen waits for (drained by
    /// the viewer, which asks for the profile and the texture).
    pub wanted_pics: HashSet<Uuid>,
    /// The ones already asked for, and the pace of the new ones.
    pics_asked: HashSet<Uuid>,
    pic_budget: super::nameplate::PictureBudget,
    /// The person whose sheet is open (Aurora's look).
    pub sheet: Option<Uuid>,
    /// Leaving the permanent inline detail panel returns to the friend list.
    inline_friends_wide: bool,
    /// A sheet just opened: the window grows for it if it is too narrow.
    sheet_grow: bool,
    /// The width the window took for the sheet, given back when it closes.
    sheet_grown: f32,
    /// The row whose "…" menu is open: its quick actions stay shown.
    row_menu: Option<Uuid>,
    /// AURORA_DEMO_PEOPLE=rows: the row shown as under the pointer, its
    /// quick actions out (a capture has no pointer).
    pub demo_hover: Option<Uuid>,
    /// The open sheet is a group's (the groups tab).
    sheet_group: bool,
    /// The rows chosen with Ctrl / Shift in the friends, contact sets,
    /// recent and blocked tabs, and the row Shift selects from.
    pub sel: Vec<Uuid>,
    anchor: Option<Uuid>,
    /// The contact set the friends tab shows alone.
    friend_set: Option<String>,
    /// The nickname being typed in the sheet, and whose.
    nick: Option<(Uuid, String)>,
    /// What the sheets ask of the contact list's dialogs.
    asks: Vec<Ask>,
    /// Group insignias a row on screen waits for (drained by the viewer).
    pub wanted_images: HashSet<Uuid>,
    /// The tab shown last frame: another one starts afresh.
    shown_tab: Option<u8>,
}

impl PeopleUi {
    /// Open the sheet of `id` in Near you (Aurora's look).
    pub fn open_sheet(&mut self, id: Uuid) {
        nearby::open_sheet(self, id);
    }

    /// Return from a person's sheet to the list, as its cross does.
    pub fn close_sheet(&mut self) {
        nearby::close_sheet(self);
    }

    /// Open the sheet of one of our groups (the groups tab).
    pub fn open_group_sheet(&mut self, id: Uuid) {
        nearby::open_sheet(self, id);
        self.sheet_group = true;
    }

    /// Choose rows of a list as with Ctrl (AURORA_DEMO_PEOPLE).
    pub fn select(&mut self, ids: Vec<Uuid>) {
        self.anchor = ids.last().copied();
        self.sel = ids;
    }

    /// A tab is shown: another than last frame's starts afresh (no filter,
    /// no selection) and a sheet of the other kind closes (a group's in the
    /// groups tab only, none in the blocked tab).
    fn tab_shown(&mut self, tab: u8) {
        if self.shown_tab == Some(tab) {
            return;
        }
        if self.shown_tab.is_some() {
            self.filter.clear();
            self.sel.clear();
            self.anchor = None;
        }
        self.shown_tab = Some(tab);
        let keep = match tab {
            TAB_GROUPS => self.sheet_group,
            TAB_BLOCKED => false,
            _ => !self.sheet_group,
        };
        if self.sheet.is_some() && !keep {
            nearby::close_sheet(self);
        }
    }

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
    if world.rlva.has("shownearby") {
        return Vec::new();
    }
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

/// The contact list's shared dialogs and the group insignias. Aurora's
/// rows do not depend on the Legacy table's presentation settings.
pub struct ContactsParts<'a> {
    pub ui: &'a mut ContactsUi,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
}

#[allow(clippy::too_many_arguments)]
pub fn people_window(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    panels: &mut Panels,
    st: &mut PeopleUi,
    radar: &mut RadarData,
    friends: &mut FriendsSettings,
    contacts: ContactsParts,
    // the mini-map renderer (LLNetMap in the Nearby tab)
    map: &mut dyn FnMut(&mut egui::Ui, Vec2),
    // Aurora's look (`Settings::classic_people` off)
    aurora: bool,
) -> Vec<PeopleAction> {
    let mut actions = Vec::new();
    if world.rlva.has("shownearby") && panels.people_tab == TAB_NEARBY {
        nearby::close_sheet(st);
    }
    let mut open = panels.people;
    let help = aurora_i18n::tr!("ui-people-nearby-avatars-and-friends-list");
    let pos = default_pos(super::layout::work_area(ctx));
    let size = if aurora { AURORA_SIZE } else { DEFAULT_SIZE };
    // the table has no sheet: back to Firestorm's look, the window gives
    // back the width it took for one
    if !aurora && st.sheet.is_some() {
        nearby::close_sheet(st);
    }
    if !aurora && panels.people_tab == TAB_SETS {
        panels.people_tab = TAB_FRIENDS;
    }
    if aurora {
        st.tab_shown(panels.people_tab);
    }
    nearby::fit_window(ctx, st);
    let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("common-people"), pos, size).help(help);
    if aurora {
        // the mock-up's tabs right under the title
        floater = floater.compact_header();
    }
    floater.show(ctx, p, &mut open, |ui| {
        if aurora {
            panels.people_tab = aurora_tabs(ui, p, world, radar, panels.people_tab);
            let mut c = Ctx {
                p,
                world,
                contacts: &mut *contacts.ui,
                friends: &mut *friends,
                images: contacts.images,
                actions: &mut actions,
                embedded_contacts: false,
                contact_sheet_on_click: false,
            };
            match panels.people_tab {
                TAB_NEARBY => {
                    let mut blocked = false;
                    nearby::tab(ui, p, world, radar, st, map, c.actions, &mut blocked);
                    if blocked {
                        panels.people_tab = TAB_BLOCKED;
                    }
                }
                TAB_FRIENDS => friends::tab(ui, &mut c, radar, st),
                TAB_GROUPS => groups::tab(ui, &mut c, radar, st),
                TAB_SETS => sets::tab(ui, &mut c, radar, st),
                TAB_RECENT => recent::tab(ui, &mut c, radar, st),
                _ => blocked::tab(ui, &mut c, st),
            }
            return;
        }
        let mut tab = panels.people_tab as usize;
        let labels = [
            (aurora_i18n::tr!("ui-people-near-you"), true),
            (aurora_i18n::tr!("common-friends"), true),
            (aurora_i18n::tr!("common-groups"), true),
            (aurora_i18n::tr!("common-recent"), false),
            (aurora_i18n::tr!("common-blocked"), true),
        ];
        widgets::tabs(ui, p, &mut tab, &labels);
        panels.people_tab = tab as u8;
        ui.add_space(4.0);
        match panels.people_tab {
            0 => {
                // the radar lays out its own filter, beside its buttons
                let mut blocked = false;
                radar::tab(ui, p, world, radar, st, map, &mut actions, &mut blocked);
                if blocked {
                    panels.people_tab = TAB_BLOCKED;
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
    // what the sheets asked of the contact list's dialogs
    for ask in std::mem::take(&mut st.asks) {
        match ask {
            Ask::RemoveFromSet(set, ids) => contacts.ui.ask_remove_from_set(&set, ids),
            Ask::NewSet => contacts.ui.ask_new_set(),
        }
    }
    panels.people = open;
    actions
}

/// What Conversations › Contacts shows in Aurora's look: the friends tab of
/// this window (its list, and the sheet of the friend held), with a
/// state of its own so that each window keeps its filter, selection and
/// sheet.
pub struct FriendsPane<'a, 'r> {
    pub st: &'a mut PeopleUi,
    pub data: &'a RadarData<'r>,
    pub settings: &'a mut FriendsSettings,
    /// Group insignias.
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// The friends tab drawn in `ui` (Conversations › Contacts). No window
/// widens for a sheet there: it shows beside the list when the pane has
/// room, alone otherwise.
pub fn friends_pane(ui: &mut egui::Ui, p: &Palette, world: &World, pane: FriendsPane, contacts: &mut ContactsUi) -> Vec<PeopleAction> {
    let FriendsPane {
        st,
        data,
        settings,
        images,
    } = pane;
    st.tab_shown(TAB_FRIENDS);
    st.sheet_grow = false;
    let wide = nearby::sheet_fits(ui.available_width());
    if st.inline_friends_wide && !wide {
        nearby::close_sheet(st);
    }
    st.inline_friends_wide = wide;
    let mut actions = Vec::new();
    let mut c = Ctx {
        p,
        world,
        contacts: &mut *contacts,
        friends: settings,
        images,
        actions: &mut actions,
        embedded_contacts: true,
        contact_sheet_on_click: wide,
    };
    friends::tab(ui, &mut c, data, st);
    for ask in std::mem::take(&mut st.asks) {
        match ask {
            Ask::RemoveFromSet(set, ids) => contacts.ask_remove_from_set(&set, ids),
            Ask::NewSet => contacts.ask_new_set(),
        }
    }
    actions
}

/// The six tabs in pills, the open one's count in the accent; a window too
/// narrow for them all shows those around the open one, the others behind
/// "…". Returns the tab chosen.
fn aurora_tabs(ui: &mut egui::Ui, p: &Palette, world: &World, radar: &RadarData, mut tab: u8) -> u8 {
    let label = |t: u8| match t {
        TAB_NEARBY => aurora_i18n::tr!("ui-people-near-you"),
        TAB_FRIENDS => aurora_i18n::tr!("common-friends"),
        TAB_GROUPS => aurora_i18n::tr!("common-groups"),
        TAB_SETS => aurora_i18n::tr!("people-tab-sets"),
        TAB_RECENT => aurora_i18n::tr!("common-recent"),
        _ => aurora_i18n::tr!("common-blocked"),
    };
    // the icons of the People entries of the bars' menus
    let icon = |t: u8| match t {
        TAB_NEARBY => "radar-filled",
        TAB_FRIENDS => "users",
        TAB_GROUPS => "users-group",
        TAB_SETS => "tags-filled",
        TAB_RECENT => "history",
        _ => "ban",
    };
    let count = |t: u8| -> Option<usize> {
        match t {
            TAB_NEARBY => Some(radar.radar.total),
            TAB_FRIENDS => Some(world.social.friends.len()),
            TAB_GROUPS => Some(world.groups.groups.len()),
            TAB_RECENT => Some(world.recent.len()),
            TAB_BLOCKED => Some(world.mutes.entries().len()),
            _ => None,
        }
    };
    let open = tab;
    let counted = |t: u8| (t == open).then(|| count(t).map(|n| aurora_i18n::integer(n as i64))).flatten();
    let widths: Vec<f32> = AURORA_TABS
        .iter()
        .map(|t| pill_tab_width(ui, label(*t), counted(*t).as_deref()))
        .collect();
    let shown = tabs_shown(
        &widths,
        AURORA_TABS.iter().position(|t| *t == tab).unwrap_or(0),
        ui.available_width(),
        pill_tab_width(ui, "…", None),
    );
    let hidden: Vec<u8> = AURORA_TABS
        .iter()
        .enumerate()
        .filter(|(i, _)| !shown.contains(i))
        .map(|(_, t)| *t)
        .collect();
    let more = |ui: &mut egui::Ui, tab: &mut u8| {
        let resp = widgets::pill_tab(ui, p, "", "…", None, false, false, true);
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for t in &hidden {
                if menu::item(ui, p, icon(*t), label(*t)) {
                    *tab = *t;
                }
            }
        });
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let before = shown.start > 0;
        if before {
            more(ui, &mut tab);
        }
        for i in shown.clone() {
            let t = AURORA_TABS[i];
            if widgets::pill_tab(ui, p, "", label(t), counted(t), true, t == open, true).clicked() {
                tab = t;
            }
        }
        if !before && shown.end < AURORA_TABS.len() {
            more(ui, &mut tab);
        }
    });
    tab
}

/// What `pill_tab` takes.
fn pill_tab_width(ui: &egui::Ui, label: &str, count: Option<&str>) -> f32 {
    let w = |t: &str, size: f32| {
        ui.painter()
            .layout_no_wrap(
                t.to_owned(),
                egui::FontId::new(size, super::fonts::semibold(ui.ctx())),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
    };
    12.0 + w(label, 12.5) + count.map_or(0.0, |c| 5.0 + w(c, 11.0) + 10.0) + 12.0
}

/// The tabs a row of `room` shows: all of them, or a run around the open
/// one (`open`) beside the "…" of the others, as many as fit.
fn tabs_shown(widths: &[f32], open: usize, room: f32, more: f32) -> std::ops::Range<usize> {
    const GAP: f32 = 2.0;
    let total = |r: &std::ops::Range<usize>| widths[r.clone()].iter().sum::<f32>() + GAP * r.len().saturating_sub(1) as f32;
    if total(&(0..widths.len())) <= room {
        return 0..widths.len();
    }
    let room = room - more - GAP;
    let mut run = open..open + 1;
    // the tabs before the open one first (the mock-up's "… Recent Blocked"),
    // then the ones after
    while run.start > 0 && total(&(run.start - 1..run.end)) <= room {
        run.start -= 1;
    }
    while run.end < widths.len() && total(&(run.start..run.end + 1)) <= room {
        run.end += 1;
    }
    run
}

/// View and sort options of the friends (menu_people_friends_view.xml,
/// without the conversation log, which has its own window).
fn friends_view_menu(ui: &mut egui::Ui, p: &Palette, s: &mut FriendsSettings) {
    for (sort, label) in [
        (SORT_NAME, aurora_i18n::tr!("people-friends-sort-name")),
        (SORT_USERNAME, aurora_i18n::tr!("people-friends-sort-username")),
        (SORT_STATUS, aurora_i18n::tr!("people-friends-sort-status")),
    ] {
        if menu::check(ui, p, friends::sort_icon(sort), label, s.sort == sort, true) {
            s.sort = sort;
        }
    }
    menu::separator(ui, p);
    menu::toggle(
        ui,
        p,
        "photo-filled",
        aurora_i18n::tr!("people-friends-view-pictures"),
        &mut s.pictures,
    );
    menu::toggle(
        ui,
        p,
        "key-filled",
        aurora_i18n::tr!("people-friends-view-permissions"),
        &mut s.permissions,
    );
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
            if !matches_name(world, &f.id, &name, &filter) {
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
                    if widgets::icon_button(
                        ui,
                        p,
                        "message-circle-filled",
                        aurora_i18n::tr!("ui-people-instant-message-im"),
                        true,
                    ) {
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
    let list = ui.scope_id().with("blocked_list");
    let w = widgets::header_width(ui, list);
    let title = aurora_i18n::tr!("ui-people-name", value0 = format!("{}", entries.len()));
    widgets::header_row(ui, p, &[(&title, w - 160.0), (aurora_i18n::tr!("common-type"), 70.0), ("", 90.0)]);
    let out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
    widgets::remember_scroll_bar(ui, list, &out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// However narrow the window, the tabs shown and the "…" of the others
    /// fit in it, the open tab is among them, and a wide window shows them
    /// all.
    #[test]
    fn the_tabs_fit_beside_their_ellipsis() {
        let widths = [99.0, 76.0, 74.0, 69.0, 66.0, 88.0];
        let more = 30.0;
        for room in (200..=600).step_by(5) {
            for open in 0..widths.len() {
                let run = tabs_shown(&widths, open, room as f32, more);
                assert!(run.contains(&open), "{room}: the open tab is shown");
                let used: f32 = widths[run.clone()].iter().sum::<f32>() + 2.0 * (run.len() - 1) as f32;
                if run.len() < widths.len() {
                    assert!(
                        used + 2.0 + more <= room as f32 || run.len() == 1,
                        "{room} {open}: {run:?} overflows"
                    );
                } else {
                    assert!(used <= room as f32);
                }
            }
        }
        assert_eq!(tabs_shown(&widths, 5, 600.0, more), 0..6);
        // the mock-up's narrow window: the blocked tab and the ones before it
        let run = tabs_shown(&widths, 5, 402.0, more);
        assert_eq!(run.end, 6);
        assert!(run.start > 0);
    }

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
        // online first (the contact list's order), pictures and rights shown
        let defaults = FriendsSettings::default();
        assert_eq!(defaults.sort, SORT_STATUS);
        assert!(defaults.pictures && defaults.permissions);
    }
}
