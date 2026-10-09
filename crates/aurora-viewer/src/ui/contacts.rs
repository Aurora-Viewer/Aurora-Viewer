//! Contacts window, port of Firestorm's FSFloaterContacts and
//! FSPanelContactSets (indra/newview/fsfloatercontacts.cpp,
//! fspanelcontactsets.cpp, floater_fs_contacts.xml,
//! panel_fs_contacts_{friends,groups,sets}.xml, originally LGPL 2.1): the
//! first tab of the Conversations window (or its own window once torn off,
//! ContactsTornOff) with three tabs:
//! - « Amis »: the friend list (multi-selection, online first, configurable
//!   name columns, the rights given and received as check boxes) and its
//!   buttons;
//! - « Groupes »: our groups (pinned first, active one in bold, « aucun »
//!   on top) and their buttons;
//! - « Cercles »: contact sets (LGGContactSets) and aliases.

use super::avatar_picker::AvatarPicker;
use super::context::{self, CtxAction};
use super::{menu, widgets};
use crate::theme::Palette;
use crate::world::World;
use crate::world::contact_sets::{self, ContactSets};
use crate::world::names::{AvatarName, NameOptions};
use crate::world::social::rights;
use aurora_net::NetCommand;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Most friends one action applies to (MAX_FRIEND_SELECT, MAX_SELECTIONS).
const MAX_SELECT: usize = 20;
/// Width of the button column (80 px in Firestorm; French labels are longer).
const BUTTONS_W: f32 = 96.0;
const ROW_H: f32 = 18.0;
/// Group powers (llgroupmgr / roles_constants.h).
const GP_SESSION_JOIN: u64 = 1 << 16;
/// IM dialogs.
const IM_FRIENDSHIP_OFFERED: u8 = 38;
const IM_TELEPORT_REQUEST: u8 = 26;

/// Firestorm settings of the contacts window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactsSettings {
    /// ContactsTornOff: in its own window instead of the first tab of
    /// Conversations.
    pub torn_off: bool,
    /// FSContactListShowSearch.
    pub show_search: bool,
    /// FSFriendListColumnShowUserName / DisplayName / FullName / Permissions.
    pub column_username: bool,
    pub column_display_name: bool,
    pub column_full_name: bool,
    pub column_permissions: bool,
    /// FSFriendListSortOrder: by display name (else by username).
    pub sort_by_display_name: bool,
    /// FSFriendListFullNameFormat: « Nom d'affichage (Nom d'utilisateur) »
    /// (else the other way round).
    pub full_name_display_first: bool,
    /// FSContactSetsColorizeFriends: contact set colors in the friend list.
    pub colorize_friends: bool,
    /// FSContactSetsListShowIcons: profile pictures in the contact set list.
    pub set_list_icons: bool,
    /// Tab shown: 0 friends, 1 groups, 2 contact sets.
    pub tab: u8,
}

impl Default for ContactsSettings {
    fn default() -> Self {
        ContactsSettings {
            torn_off: false,
            show_search: true,
            column_username: false,
            column_display_name: false,
            column_full_name: true,
            column_permissions: true,
            sort_by_display_name: false,
            full_name_display_first: true,
            colorize_friends: false,
            set_list_icons: true,
            tab: 0,
        }
    }
}

pub enum ContactsAction {
    Im(Uuid),
    Profile(Uuid),
    OfferTeleport(Uuid),
    GroupChat(Uuid),
    /// Sent as is (rights, friendship end, group activation...).
    Net(NetCommand),
    /// IM_FRIENDSHIP_OFFERED with the calling card folder.
    OfferFriendship {
        to: Uuid,
        message: String,
    },
    /// Detach from / back into the Conversations window.
    ToggleTornOff,
}

/// Confirmations and text prompts (Firestorm notifications).
enum Dialog {
    RemoveFriends(Vec<Uuid>),
    /// Grant (true) / revoke the right to modify our objects.
    ModifyRights {
        friend: Uuid,
        rights: i32,
        grant: bool,
    },
    LeaveGroup(Uuid),
    /// AddFriendWithMessage.
    Friendship {
        to: Uuid,
        message: String,
    },
    /// TeleportRequest prompt.
    TeleportRequest {
        to: Uuid,
        message: String,
    },
    AddSet(String),
    RemoveSet(String),
    RemoveFromSet {
        set: String,
        ids: Vec<Uuid>,
    },
    SetAlias {
        ids: Vec<Uuid>,
        alias: String,
    },
}

#[derive(Default)]
pub struct ContactsUi {
    friend_filter: String,
    friends_sel: Vec<Uuid>,
    friends_anchor: Option<Uuid>,
    group_filter: String,
    /// Selected group (nil = « aucun »).
    group_sel: Option<Uuid>,
    /// Contact set shown (None = the first one, else « Tous »).
    set_choice: Option<String>,
    set_filter: String,
    set_sel: Vec<Uuid>,
    set_anchor: Option<Uuid>,
    dialog: Option<Dialog>,
    /// « Ajouter au cercle » of the friend menu: (set, avatars).
    add_to_set: Option<(String, Vec<Uuid>)>,
    picker: AvatarPicker,
    /// What the picker is open for.
    pick_for: Option<PickFor>,
    /// Contact set being configured (FSFloaterContactSetConfiguration).
    config: Option<String>,
    rename: String,
    /// Height under the group and contact set lists (the count line),
    /// measured each frame.
    groups_footer: f32,
    sets_footer: f32,
    /// Avatars whose profile picture the lists want, and group insignias.
    pub wanted_pics: HashSet<Uuid>,
    pub wanted_images: HashSet<Uuid>,
}

impl ContactsUi {
    /// « Ajouter... » of the friend list: the resident picker.
    pub fn pick_new_friend(&mut self) {
        self.pick_for = Some(PickFor::Friend);
        self.picker.open(PICKER_ID, false);
    }

    /// « Devenir amis » of any menu (AddFriendWithMessage).
    pub fn ask_friendship(&mut self, to: Uuid) {
        self.dialog = Some(Dialog::Friendship {
            to,
            message: String::new(),
        });
    }

    /// « Supprimer cet ami » of any menu (RemoveFromFriends).
    pub fn ask_remove_friend(&mut self, id: Uuid) {
        self.dialog = Some(Dialog::RemoveFriends(vec![id]));
    }

    /// « Demander une téléportation » of any menu (TeleportRequest).
    pub fn ask_teleport_request(&mut self, to: Uuid) {
        self.dialog = Some(Dialog::TeleportRequest {
            to,
            message: String::new(),
        });
    }

    /// « Quitter » of any group menu (GroupLeaveConfirmMember).
    pub fn ask_leave_group(&mut self, id: Uuid) {
        self.dialog = Some(Dialog::LeaveGroup(id));
    }
}

/// The confirmations and prompts, shown whether the contact list is open
/// or not (they are also asked from the other right-click menus).
pub fn dialog_windows(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi) -> Vec<ContactsAction> {
    let mut actions = Vec::new();
    dialogs(ctx, p, world, st, &mut actions);
    actions
}

/// Textures the lists draw.
pub struct Pics<'a> {
    pub avatars: &'a HashMap<Uuid, egui::TextureHandle>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// Selection after a click (LLScrollListCtrl: Ctrl toggles, Shift extends
/// from the last clicked row), at most MAX_SELECT rows.
fn click_select(sel: &mut Vec<Uuid>, anchor: &mut Option<Uuid>, order: &[Uuid], id: Uuid, mods: egui::Modifiers) {
    if mods.command {
        if let Some(i) = sel.iter().position(|s| *s == id) {
            sel.remove(i);
        } else if sel.len() < MAX_SELECT {
            sel.push(id);
        }
        *anchor = Some(id);
    } else if mods.shift
        && let Some(a) = anchor.and_then(|a| order.iter().position(|x| *x == a))
        && let Some(b) = order.iter().position(|x| *x == id)
    {
        let (lo, hi) = (a.min(b), a.max(b));
        *sel = order[lo..=hi].iter().take(MAX_SELECT).copied().collect();
    } else {
        *sel = vec![id];
        *anchor = Some(id);
    }
}

/// The rights we give after ticking a box (applyRightsToFriends): the map
/// needs the online status, so they go on / off together.
fn rights_after(current: i32, bit: i32, on: bool) -> i32 {
    match (bit, on) {
        (rights::ONLINE_STATUS, false) => current & !(rights::ONLINE_STATUS | rights::MAP_LOCATION),
        (rights::MAP_LOCATION, true) => current | rights::MAP_LOCATION | rights::ONLINE_STATUS,
        (b, true) => current | b,
        (b, false) => current & !b,
    }
}

/// FSFloaterContacts::getFullName.
fn full_name(n: &AvatarName, o: &NameOptions, display_first: bool) -> String {
    let user = n.user_name_for_display(o);
    if n.is_default || !o.use_display_names {
        user
    } else if display_first {
        format!("{} ({user})", n.display_name)
    } else {
        format!("{user} ({})", n.display_name)
    }
}

fn color32(c: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
        (c[3] * 255.0) as u8,
    )
}

/// Group limit of the account (LLAgentBenefitsMgr group_membership_limit).
fn group_limit(world: &World) -> usize {
    world
        .login
        .as_ref()
        .map(|l| {
            let b = l.raw["account_level_benefits"]["group_membership_limit"].as_i32();
            if b > 0 { b } else { l.raw["max-agent-groups"].as_i32() }
        })
        .filter(|n| *n > 0)
        .unwrap_or(0) as usize
}

/// FSCommon::populateGroupCount (strings.xml groupcountstring).
fn group_count_text(count: usize, limit: usize) -> String {
    if limit > 0 {
        format!("Vous faites partie de {count} groupes ({} restant).", limit.saturating_sub(count))
    } else {
        format!("Vous faites partie de {count} groupes.")
    }
}

/// Button of the right-hand column; `tip` explains it (or why it is off).
fn side_button(ui: &mut egui::Ui, p: &Palette, label: &str, tip: &str, enabled: bool) -> bool {
    let r = side_button_resp(ui, p, label, enabled);
    let r = if tip.is_empty() {
        r
    } else if enabled {
        r.on_hover_text(tip)
    } else {
        r.on_disabled_hover_text(tip)
    };
    r.clicked()
}

/// The button itself, label centered like LLButton.
fn side_button_resp(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(label).size(12.0).color(if enabled { p.ink } else { p.muted_dim }))
        .fill(p.raised)
        .corner_radius(CornerRadius::same(2))
        .truncate();
    ui.add_enabled_ui(enabled, |ui| ui.add_sized([BUTTONS_W, 22.0], b)).inner
}

/// Small icon button (contact set combo row).
fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if enabled && resp.hovered() {
        p.raised.gamma_multiply(1.4)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, CornerRadius::same(2), fill);
    if let Some(t) = super::icons::global(icon) {
        let tint = if enabled { p.ink } else { p.muted_dim };
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp.on_hover_text(tip)
}

/// Check box cell of a right; returns the click on an editable one.
fn right_cell(ui: &egui::Ui, p: &Palette, rect: egui::Rect, on: bool, editable: bool, id: egui::Id) -> bool {
    let b = egui::Rect::from_center_size(rect.center(), Vec2::splat(12.0));
    let resp = ui.interact(b.expand(2.0), id, if editable { Sense::click() } else { Sense::hover() });
    let painter = ui.painter();
    let border = if editable && resp.hovered() { p.violet_light } else { p.muted_dim };
    if on {
        painter.rect_filled(
            b,
            CornerRadius::same(2),
            if editable { p.violet } else { p.violet.gamma_multiply(0.45) },
        );
        if let Some(t) = super::icons::global("check") {
            painter.image(
                t.id(),
                b.shrink(1.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    } else {
        painter.rect_stroke(b, CornerRadius::same(2), Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
    editable && resp.clicked()
}

/// Height of a tab when a new window is first sized.
const DEFAULT_TAB_H: f32 = 300.0;

/// Height left in the window, or `default` during the sizing pass of a new
/// window: egui sizes it from its content, and a list filling the available
/// height then asks for the whole screen. Lists take this height minus their
/// footer measured the frame before, exactly (a guessed footer too small
/// made the resizable window grow every frame).
pub(crate) fn fill_height(ui: &egui::Ui, default: f32) -> f32 {
    if ui.is_sizing_pass() { default } else { ui.available_height() }
}

/// A footer height not measured yet: about one line of text.
pub(crate) fn footer_or_default(measured: f32) -> f32 {
    if measured > 0.0 { measured } else { 24.0 }
}

/// List text cut to `width`, centered vertically on `left_center`. Bold is
/// drawn like LLFontGL without a bold face: twice, one pixel apart
/// (BOLD_OFFSET); egui's "strong" text only changes the color.
fn row_text(ui: &egui::Ui, text: &str, left_center: egui::Pos2, width: f32, col: Color32, bold: bool) {
    let galley = egui::WidgetText::from(RichText::new(text).size(12.5).color(col)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width - if bold { 1.0 } else { 0.0 },
        egui::TextStyle::Body,
    );
    let pos = egui::pos2(left_center.x, left_center.y - galley.size().y * 0.5);
    if bold {
        let ppp = ui.ctx().pixels_per_point();
        let dx = ppp.round().max(1.0) / ppp;
        ui.painter().galley(pos + Vec2::new(dx, 0.0), galley.clone(), col);
    }
    ui.painter().galley(pos, galley, col);
}

/// Picture (or initials) of an avatar or a group in a list row.
fn row_icon(ui: &egui::Ui, p: &Palette, rect: egui::Rect, tex: Option<&egui::TextureHandle>, fallback: &str) {
    match tex {
        Some(t) => {
            ui.painter().image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(rect, 2.0, p.raised);
            if let Some(t) = super::icons::global(fallback) {
                ui.painter().image(
                    t.id(),
                    rect.shrink(2.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    p.muted,
                );
            }
        }
    }
}

/// The contacts: tab strip and the tab shown.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    settings: &mut ContactsSettings,
    pics: &Pics,
) -> Vec<ContactsAction> {
    let mut actions = Vec::new();
    let mut tab = settings.tab.min(2) as usize;
    widgets::tabs(ui, p, &mut tab, &[("Amis", true), ("Groupes", true), ("Cercles", true)]);
    settings.tab = tab as u8;
    ui.add_space(4.0);
    match tab {
        0 => friends_tab(ui, p, world, st, settings, &mut actions),
        1 => groups_tab(ui, p, world, st, pics, &mut actions),
        _ => sets_tab(ui, p, world, st, settings, pics, &mut actions),
    }
    if let Some((set, ids)) = st.add_to_set.take() {
        picked(world, st, PickFor::Set(set), ids);
    }
    // the resident picker shared with About Land (LLFloaterAvatarPicker)
    if let Some(ids) = st.picker.show(ui.ctx(), p, world)
        && let Some(what) = st.pick_for.take()
    {
        picked(world, st, what, ids);
    }
    set_config_window(ui.ctx(), p, world, st, settings);
    actions
}

/// The torn-off Contacts window (floater_fs_contacts.xml, 396 × 390); its
/// title bar button docks it back into Conversations.
pub fn window(
    ctx: &egui::Context,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    settings: &mut ContactsSettings,
    pics: &Pics,
    open: &mut bool,
) -> Vec<ContactsAction> {
    let mut dock = false;
    let screen = ctx.content_rect();
    let mut actions = widgets::Floater::new(
        "contacts",
        "Contacts",
        egui::pos2(screen.left() + 40.0, screen.bottom() - 460.0),
        Vec2::new(430.0, 390.0),
    )
    .action("arrow-square-in", "Rattacher à Conversations", &mut dock)
    .show(ctx, p, open, |ui| panel(ui, p, world, st, settings, pics))
    .unwrap_or_default();
    if dock {
        actions.push(ContactsAction::ToggleTornOff);
    }
    actions
}

/// The resident picker of the contacts window: what it was opened for.
enum PickFor {
    /// « Ajouter... » of the friend list (LLFloaterAvatarPicker with
    /// isItemsFreeOfFriends).
    Friend,
    /// « Ajouter... » of a contact set.
    Set(String),
}

/// The picker id: one window, whichever list opened it.
const PICKER_ID: &str = "contacts_avatar_picker";

/// Avatars chosen in the picker.
fn picked(world: &mut World, st: &mut ContactsUi, what: PickFor, ids: Vec<Uuid>) {
    match what {
        PickFor::Friend => {
            // onAvatarPicked → requestFriendshipDialog; Firestorm's picker
            // refuses friends (isItemsFreeOfFriends), the shared one does not
            match ids.iter().find(|id| **id != world.agent_id && !world.social.is_friend(id)) {
                Some(to) => {
                    st.dialog = Some(Dialog::Friendship {
                        to: *to,
                        message: "Voulez-vous être mon ami(e) ?".into(),
                    })
                }
                None => {
                    if let Some(id) = ids.first() {
                        let name = world.social.name_of(id);
                        world.system_message(format!("{name} fait déjà partie de vos amis."));
                    }
                }
            }
        }
        PickFor::Set(set) => {
            let social = &world.social;
            world.contact_sets.add_to_set(&ids, &set, |id| social.is_friend(id));
        }
    }
}

// ------------------------------------------------------------------ friends

struct FriendRow {
    id: Uuid,
    online: bool,
    given: i32,
    has: i32,
    user: String,
    display: String,
    full: String,
    color: Option<Color32>,
}

fn friend_rows(world: &World, settings: &ContactsSettings, filter: &str) -> Vec<FriendRow> {
    let names = &world.social.avatar_names;
    let o = names.options;
    let social = &world.social;
    let mut rows: Vec<FriendRow> = world
        .social
        .friends
        .iter()
        .map(|f| {
            let (user, display, full) = match names.get(&f.id) {
                Some(n) => (
                    n.user_name_for_display(&o),
                    n.display(&o),
                    full_name(n, &o, settings.full_name_display_first),
                ),
                None => {
                    let legacy = world.social.names.get(&f.id).cloned().unwrap_or_else(|| "…".into());
                    (legacy.clone(), legacy.clone(), legacy)
                }
            };
            let color = settings
                .colorize_friends
                .then(|| world.contact_sets.color_to_show(&f.id, |id| social.is_friend(id)))
                .flatten()
                .map(color32);
            FriendRow {
                id: f.id,
                online: f.online,
                given: f.rights_given,
                has: f.rights_has,
                user,
                display,
                full,
                color,
            }
        })
        .filter(|r| filter.is_empty() || r.full.to_lowercase().contains(filter))
        .collect();
    // sortFriendList: by username or display name, then online first
    rows.sort_by_cached_key(|r| {
        let key = if settings.sort_by_display_name { &r.display } else { &r.user };
        (!r.online, key.to_lowercase())
    });
    rows
}

fn friends_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    settings: &mut ContactsSettings,
    actions: &mut Vec<ContactsAction>,
) {
    let filter = if settings.show_search {
        st.friend_filter.trim().to_lowercase()
    } else {
        String::new()
    };
    let rows = friend_rows(world, settings, &filter);
    // forget friends that are gone or filtered out
    st.friends_sel.retain(|id| rows.iter().any(|r| r.id == *id));
    let order: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    for r in &rows {
        world.social.want_name(r.id);
    }
    let h = fill_height(ui, DEFAULT_TAB_H).max(120.0);
    ui.horizontal_top(|ui| {
        let list_w = (ui.available_width() - BUTTONS_W - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(list_w);
            if settings.show_search {
                widgets::search_field(ui, &mut st.friend_filter, "Filtrer les amis", list_w);
                ui.add_space(3.0);
            }
            friend_table(ui, p, world, st, settings, &rows, &order, list_w, actions);
        });
        ui.vertical(|ui| {
            ui.set_min_height(h);
            ui.spacing_mut().item_spacing.y = 3.0;
            friend_buttons(ui, p, world, st, actions);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn friend_table(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut ContactsUi,
    settings: &mut ContactsSettings,
    rows: &[FriendRow],
    order: &[Uuid],
    width: f32,
    actions: &mut Vec<ContactsAction>,
) {
    // columns: status, the name columns shown (sharing the width), rights
    let names: Vec<(&str, &str, u8)> = [
        (
            settings.column_username,
            "Nom d'utilisateur",
            "Le nom d'utilisateur du contact.",
            0u8,
        ),
        (settings.column_display_name, "Nom d'affichage", "Le nom d'affichage du contact.", 1),
        (settings.column_full_name, "Nom", "Le nom choisi pour cette personne.", 2),
    ]
    .into_iter()
    .filter(|c| c.0)
    .map(|c| (c.1, c.2, c.3))
    .collect();
    let rights_cols: [(&str, &str); 5] = [
        ("eye", "La personne peut savoir lorsque vous êtes connecté"),
        ("map-pin", "La personne peut vous localiser"),
        ("cube", "La personne peut modifier, supprimer ou s'approprier vos objets"),
        ("map-pin", "Vous pouvez localiser cette personne"),
        (
            "cube",
            "Vous pouvez modifier, supprimer ou vous approprier les objets de cette personne",
        ),
    ];
    let status_w = 20.0;
    let right_w = 18.0;
    let rights_w = if settings.column_permissions { right_w * 5.0 } else { 0.0 };
    let name_w = ((width - status_w - rights_w - 4.0) / names.len().max(1) as f32).max(40.0);
    // header
    let (hrect, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    ui.painter().rect_filled(hrect, 0.0, p.field);
    let mut x = hrect.left();
    let hcell = egui::Rect::from_min_size(egui::pos2(x, hrect.top()), Vec2::new(status_w, 20.0));
    ui.painter().circle_filled(hcell.center(), 3.5, p.success.gamma_multiply(0.7));
    ui.interact(hcell, ui.id().with("h_status"), Sense::hover()).on_hover_text("Statut");
    x += status_w;
    for (i, (label, tip, _)) in names.iter().enumerate() {
        let cell = egui::Rect::from_min_size(egui::pos2(x, hrect.top()), Vec2::new(name_w, 20.0));
        ui.painter().text(
            egui::pos2(cell.left() + 4.0, cell.center().y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(12.0),
            p.muted,
        );
        ui.interact(cell, ui.id().with(("h_name", i)), Sense::hover()).on_hover_text(*tip);
        x += name_w;
    }
    if settings.column_permissions {
        let rx = hrect.right() - rights_w;
        for (i, (icon, tip)) in rights_cols.iter().enumerate() {
            let cell = egui::Rect::from_min_size(egui::pos2(rx + i as f32 * right_w, hrect.top()), Vec2::new(right_w, 20.0));
            if let Some(t) = super::icons::global(icon) {
                // theirs: dimmer, like the "_theirs" images
                let tint = if i < 3 { p.muted } else { p.muted_dim };
                ui.painter().image(
                    t.id(),
                    egui::Rect::from_center_size(cell.center(), Vec2::splat(12.0)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }
            ui.interact(cell, ui.id().with(("h_right", i)), Sense::hover()).on_hover_text(*tip);
        }
    }
    // nothing under the list
    let list_h = fill_height(ui, DEFAULT_TAB_H - 50.0).max(60.0);
    egui::Frame::new().fill(p.field.gamma_multiply(0.6)).show(ui, |ui| {
        ui.set_width(width);
        egui::ScrollArea::vertical()
            .id_salt("friend_list")
            .auto_shrink([false, false])
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if rows.is_empty() {
                    ui.label(RichText::new("Aucun ami").size(12.0).color(p.muted));
                }
                for r in rows {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let selected = st.friends_sel.contains(&r.id);
                    if selected {
                        ui.painter().rect_filled(rect, 0.0, p.violet.gamma_multiply(0.35));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, p.raised.gamma_multiply(0.6));
                    }
                    // status: green square when online (icon_avatar_online)
                    let s = egui::Rect::from_center_size(egui::pos2(rect.left() + status_w * 0.5, rect.center().y), Vec2::splat(9.0));
                    if r.online {
                        ui.painter().rect_filled(s, CornerRadius::same(1), p.success);
                    }
                    // names: bold when online (updateFriendItem), contact set color
                    let col = r.color.unwrap_or(if r.online { p.ink } else { p.muted });
                    let mut x = rect.left() + status_w;
                    for (_, _, which) in &names {
                        let text = match which {
                            0 => &r.user,
                            1 => &r.display,
                            _ => &r.full,
                        };
                        row_text(ui, text, egui::pos2(x + 4.0, rect.center().y), name_w - 6.0, col, r.online);
                        x += name_w;
                    }
                    let mut clicked_right = false;
                    if settings.column_permissions {
                        let rx = rect.right() - rights_w;
                        let cells = [
                            (r.given & rights::ONLINE_STATUS != 0, true, rights::ONLINE_STATUS),
                            (r.given & rights::MAP_LOCATION != 0, true, rights::MAP_LOCATION),
                            (r.given & rights::MODIFY_OBJECTS != 0, true, rights::MODIFY_OBJECTS),
                            (r.has & rights::MAP_LOCATION != 0, false, 0),
                            (r.has & rights::MODIFY_OBJECTS != 0, false, 0),
                        ];
                        for (i, (on, editable, bit)) in cells.into_iter().enumerate() {
                            let cell =
                                egui::Rect::from_min_size(egui::pos2(rx + i as f32 * right_w, rect.top()), Vec2::new(right_w, ROW_H));
                            if right_cell(ui, p, cell, on, editable, ui.id().with(("right", r.id, i))) {
                                clicked_right = true;
                                let new = rights_after(r.given, bit, !on);
                                if bit == rights::MODIFY_OBJECTS {
                                    st.dialog = Some(Dialog::ModifyRights {
                                        friend: r.id,
                                        rights: new,
                                        grant: !on,
                                    });
                                } else {
                                    actions.push(ContactsAction::Net(NetCommand::GrantUserRights { friend: r.id, rights: new }));
                                }
                            }
                        }
                    }
                    if resp.clicked() && !clicked_right {
                        let mods = ui.input(|i| i.modifiers);
                        click_select(&mut st.friends_sel, &mut st.friends_anchor, order, r.id, mods);
                    }
                    if resp.double_clicked() {
                        actions.push(ContactsAction::Im(r.id));
                    }
                    if resp.secondary_clicked() && !selected {
                        st.friends_sel = vec![r.id];
                        st.friends_anchor = Some(r.id);
                    }
                    menu::context_menu(&resp, p, |ui| friend_menu(ui, p, world, st, settings, r.id, actions));
                }
            });
    });
}

/// Right-click menu of the friend list (menu_fs_contacts_friends.xml).
fn friend_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut ContactsUi,
    settings: &mut ContactsSettings,
    id: Uuid,
    actions: &mut Vec<ContactsAction>,
) {
    let friend = world.social.friends.iter().find(|f| f.id == id);
    let online = friend.is_some_and(|f| f.online);
    let here = context::avatar_position(world, &id).is_some();
    // the selection when the clicked row is part of it
    let ids = if st.friends_sel.contains(&id) {
        st.friends_sel.clone()
    } else {
        vec![id]
    };
    if menu::item(ui, p, "user-circle", "Voir le profil") {
        actions.push(ContactsAction::Profile(id));
    }
    if menu::item(ui, p, "chat-text", "Envoyer un IM...") {
        actions.push(ContactsAction::Im(id));
    }
    menu::todo(ui, p, "clock-counter-clockwise", "Voir l'historique de conversations");
    let sets = world.contact_sets.set_names();
    menu::submenu(ui, p, "users-three", "Ajouter au cercle", true, |ui| {
        if sets.is_empty() {
            menu::item_if(ui, p, "users-three", "Aucun cercle", false);
        }
        for set in sets {
            if menu::item(ui, p, "users-three", &set) {
                st.add_to_set = Some((set, ids.clone()));
            }
        }
    });
    if menu::item_if(ui, p, "magnifying-glass-plus", "Zoomer", here) {
        context::request(ui.ctx(), CtxAction::ZoomAvatar(id));
    }
    if menu::item_if(ui, p, "navigation-arrow", "Se téléporter vers", here) {
        context::request(ui.ctx(), CtxAction::TeleportToAvatar(id));
    }
    if menu::item_if(ui, p, "paper-plane-tilt", "Proposer une téléportation", online) {
        actions.push(ContactsAction::OfferTeleport(id));
    }
    if menu::item(ui, p, "airplane-landing", "Demander une téléportation") {
        st.ask_teleport_request(id);
    }
    menu::todo(ui, p, "currency-circle-dollar", "Payer");
    menu::todo(ui, p, "crosshair", "Suivre");
    if menu::item(ui, p, "user-minus", "Supprimer cet ami") {
        st.dialog = Some(Dialog::RemoveFriends(ids.clone()));
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "copy", "Copier le nom") {
        ui.ctx().copy_text(world.social.name_of(&id));
    }
    if menu::item(ui, p, "link", "Copier l'URL") {
        ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
    }
    if menu::item(ui, p, "at", "Copier l'URI de la mention") {
        ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/mention"));
    }
    menu::separator(ui, p);
    menu::submenu(ui, p, "sliders", "Options...", true, |ui| options_menu(ui, p, settings));
    if let Some(f) = friend {
        menu::separator(ui, p);
        // GlobalOnlineStatusToggle: the online status right we give
        let on = f.rights_given & rights::ONLINE_STATUS != 0;
        if menu::check(ui, p, "eye", "Statut connecté visible pour cet ami", on, true) {
            actions.push(ContactsAction::Net(NetCommand::GrantUserRights {
                friend: id,
                rights: rights_after(f.rights_given, rights::ONLINE_STATUS, !on),
            }));
        }
    }
}

/// « Options... » of the friend list menu: columns, sort, name format,
/// search filter (onColumnDisplayModeChanged keeps one name column).
fn options_menu(ui: &mut egui::Ui, p: &Palette, s: &mut ContactsSettings) {
    let before = (s.column_username, s.column_display_name, s.column_full_name);
    menu::toggle(ui, p, "user", "Afficher la colonne Nom d'utilisateur", &mut s.column_username);
    menu::toggle(
        ui,
        p,
        "identification-card",
        "Afficher la colonne Nom d'affichage",
        &mut s.column_display_name,
    );
    menu::toggle(ui, p, "text-aa", "Afficher la colonne Nom complet", &mut s.column_full_name);
    if !s.column_username && !s.column_display_name && !s.column_full_name {
        (s.column_username, s.column_display_name, s.column_full_name) = before;
    }
    menu::toggle(ui, p, "key", "Afficher les colonnes de permissions", &mut s.column_permissions);
    menu::separator(ui, p);
    if menu::check(ui, p, "list", "Trier par nom d'utilisateur", !s.sort_by_display_name, true) {
        s.sort_by_display_name = false;
    }
    if menu::check(ui, p, "list", "Trier par nom d'affichage", s.sort_by_display_name, true) {
        s.sort_by_display_name = true;
    }
    menu::separator(ui, p);
    if menu::check(
        ui,
        p,
        "text-t",
        "Format du nom complet : Nom d'utilisateur (Nom d'affichage)",
        !s.full_name_display_first,
        true,
    ) {
        s.full_name_display_first = false;
    }
    if menu::check(
        ui,
        p,
        "text-t",
        "Format du nom complet : Nom d'affichage (Nom d'utilisateur)",
        s.full_name_display_first,
        true,
    ) {
        s.full_name_display_first = true;
    }
    menu::separator(ui, p);
    menu::toggle(ui, p, "magnifying-glass", "Afficher la recherche", &mut s.show_search);
}

/// Buttons of the friend list (refreshUI / refreshRightsChangeList).
fn friend_buttons(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut ContactsUi, actions: &mut Vec<ContactsAction>) {
    let sel = &st.friends_sel;
    let n = sel.len();
    let all_online = sel.iter().all(|id| world.social.friends.iter().any(|f| f.id == *id && f.online));
    let soon = "À venir dans Aurora";
    let im_tip = if n > 1 {
        "Conférence à plusieurs : à venir dans Aurora"
    } else {
        "Ouvrir une fenêtre de conversation privée"
    };
    if side_button(ui, p, "IM/Appel", im_tip, n == 1) {
        actions.push(ContactsAction::Im(sel[0]));
    }
    if side_button(
        ui,
        p,
        "Profil",
        "Affiche sa photo, ses groupes et d'autres informations utiles",
        n == 1,
    ) {
        actions.push(ContactsAction::Profile(sel[0]));
    }
    if side_button(
        ui,
        p,
        "Téléporter...",
        "Proposer à cet ami de se téléporter à votre emplacement actuel",
        n >= 1 && all_online,
    ) {
        for id in sel {
            actions.push(ContactsAction::OfferTeleport(*id));
        }
    }
    side_button(ui, p, "Carte...", soon, false);
    side_button(ui, p, "Payer...", soon, false);
    if side_button(ui, p, "Supprimer...", "Supprimer cette personne de vos amis", n >= 1) {
        st.dialog = Some(Dialog::RemoveFriends(sel.clone()));
    }
    if side_button(ui, p, "Ajouter...", "Proposer à une personne de devenir ami", true) {
        st.pick_new_friend();
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!("Amis : {}", world.social.friends.len()))
            .size(12.0)
            .color(p.muted),
    );
}

// ------------------------------------------------------------------- groups

fn groups_tab(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, pics: &Pics, actions: &mut Vec<ContactsAction>) {
    let filter = st.group_filter.trim().to_lowercase();
    let groups = world.groups.sorted();
    if st.group_sel.is_some_and(|g| !g.is_nil() && !world.groups.is_member(&g)) {
        st.group_sel = None;
    }
    // LLGroupList::refresh selects the active group
    if st.group_sel.is_none() {
        st.group_sel = Some(world.groups.active);
    }
    let h = fill_height(ui, DEFAULT_TAB_H).max(120.0);
    ui.horizontal_top(|ui| {
        let list_w = (ui.available_width() - BUTTONS_W - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(list_w);
            widgets::search_field(ui, &mut st.group_filter, "Filtrer les groupes", list_w);
            ui.add_space(3.0);
            let list_h = (fill_height(ui, DEFAULT_TAB_H - 30.0) - footer_or_default(st.groups_footer)).max(60.0);
            let list = group_list(ui, p, world, st, pics, &groups, &filter, list_w, list_h, actions);
            ui.add_space(3.0);
            let count = ui.label(
                RichText::new(group_count_text(world.groups.groups.len(), group_limit(world)))
                    .size(12.0)
                    .color(p.muted),
            );
            st.groups_footer = count.rect.bottom() - list.bottom();
        });
        ui.vertical(|ui| {
            ui.set_min_height(h);
            ui.spacing_mut().item_spacing.y = 3.0;
            group_buttons(ui, p, world, st, actions);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn group_list(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    pics: &Pics,
    groups: &[aurora_net::GroupMembership],
    filter: &str,
    list_w: f32,
    list_h: f32,
    actions: &mut Vec<ContactsAction>,
) -> egui::Rect {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.6))
        .show(ui, |ui| {
            ui.set_width(list_w);
            egui::ScrollArea::vertical()
                .id_salt("group_list")
                .auto_shrink([false, false])
                .max_height(list_h)
                .min_scrolled_height(list_h)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let shown: Vec<_> = groups
                        .iter()
                        .filter(|g| filter.is_empty() || g.name.to_lowercase().contains(filter))
                        .collect();
                    if groups.is_empty() {
                        ui.label(
                            RichText::new("Vous cherchez à rejoindre un groupe ? Essayez la recherche.")
                                .size(12.0)
                                .color(p.muted),
                        );
                    } else if shown.is_empty() {
                        ui.label(
                            RichText::new(format!("Aucun groupe contenant « {} » n'a été trouvé.", st.group_filter.trim()))
                                .size(12.0)
                                .color(p.muted),
                        );
                    }
                    // « aucun » on top when not filtering (LLGroupList::refresh)
                    if filter.is_empty() && !groups.is_empty() {
                        group_row(ui, p, world, st, pics, None, list_w, actions);
                    }
                    let pinned = shown.iter().filter(|g| world.groups.favorites.contains(&g.id)).count();
                    for (i, g) in shown.iter().enumerate() {
                        if filter.is_empty() && pinned > 0 && i == pinned && pinned < shown.len() {
                            // LLGroupListSeparator after the pinned groups
                            let (r, _) = ui.allocate_exact_size(Vec2::new(list_w, 7.0), Sense::hover());
                            ui.painter()
                                .hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0, p.raised));
                        }
                        group_row(ui, p, world, st, pics, Some(g), list_w, actions);
                    }
                });
        })
        .response
        .rect
}

#[allow(clippy::too_many_arguments)]
fn group_row(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    pics: &Pics,
    g: Option<&&aurora_net::GroupMembership>,
    width: f32,
    actions: &mut Vec<ContactsAction>,
) {
    let id = g.map(|g| g.id).unwrap_or_default();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let selected = st.group_sel == Some(id);
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.violet.gamma_multiply(0.35));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.raised.gamma_multiply(0.6));
    }
    let icon = egui::Rect::from_min_size(egui::pos2(rect.left() + 3.0, rect.top() + 2.0), Vec2::splat(18.0));
    let insignia = g.map(|g| g.insignia).filter(|i| !i.is_nil());
    if let Some(i) = insignia {
        st.wanted_images.insert(i);
    }
    row_icon(ui, p, icon, insignia.and_then(|i| pics.images.get(&i)), "users-three");
    // active group in bold; hidden from the profile: GroupHiddenInProfile
    let active = world.groups.active == id;
    let name = g.map(|g| g.name.clone()).unwrap_or_else(|| "aucun".into());
    let col = match g {
        Some(g) if g.list_in_profile => p.indigo_light,
        _ => p.ink,
    };
    let pinned = world.groups.favorites.contains(&id);
    let text_w = width - 30.0 - if pinned { 18.0 } else { 0.0 };
    row_text(ui, &name, egui::pos2(icon.right() + 6.0, rect.center().y), text_w, col, active);
    if pinned && let Some(t) = super::icons::global("push-pin") {
        let r = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, rect.center().y), Vec2::splat(12.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            p.muted,
        );
    }
    if resp.clicked() || resp.secondary_clicked() {
        st.group_sel = Some(id);
    }
    if resp.double_clicked() && !id.is_nil() {
        actions.push(ContactsAction::GroupChat(id));
    }
    menu::context_menu(&resp, p, |ui| group_menu(ui, p, world, st, id, actions));
}

/// Right-click menu of a group (menu_people_groups.xml).
fn group_menu(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, id: Uuid, actions: &mut Vec<ContactsAction>) {
    let real = !id.is_nil();
    if menu::item_if(ui, p, "check-circle", "Activer", world.groups.active != id) {
        actions.push(ContactsAction::Net(NetCommand::ActivateGroup(id)));
    }
    menu::todo(ui, p, "info", "Voir les infos");
    if menu::item_if(ui, p, "link", "Copier le SLurl", real) {
        ui.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
    }
    if menu::item_if(ui, p, "chat-text", "Chat", real) {
        actions.push(ContactsAction::GroupChat(id));
    }
    menu::todo(ui, p, "phone", "Appel vocal");
    if real {
        let label = if world.groups.favorites.contains(&id) {
            "Désépingler le groupe"
        } else {
            "Épingler le groupe"
        };
        if menu::item(ui, p, "push-pin", label) {
            world.groups.toggle_favorite(id);
        }
    }
    menu::separator(ui, p);
    if menu::item_if(ui, p, "sign-out", "Quitter", real) {
        st.ask_leave_group(id);
    }
}

/// Buttons of the group list (updateGroupButtons).
fn group_buttons(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, actions: &mut Vec<ContactsAction>) {
    let sel = st.group_sel.unwrap_or_default();
    let real = !sel.is_nil();
    let powers = world.groups.group(&sel).map(|g| g.powers).unwrap_or(0);
    let soon = "À venir dans Aurora";
    if side_button(
        ui,
        p,
        "IM/Appel",
        "Ouvrir une fenêtre de conversation",
        real && powers & GP_SESSION_JOIN != 0,
    ) {
        actions.push(ContactsAction::GroupChat(sel));
    }
    side_button(ui, p, "Informations", soon, false);
    side_button(ui, p, "Titres", soon, false);
    if side_button(ui, p, "Activer", "", sel != world.groups.active) {
        actions.push(ContactsAction::Net(NetCommand::ActivateGroup(sel)));
    }
    let pinned = world.groups.favorites.contains(&sel);
    if side_button(
        ui,
        p,
        if pinned { "Désépingler" } else { "Épingler" },
        "Ajouter ou supprimer un groupe des favoris",
        real,
    ) {
        world.groups.toggle_favorite(sel);
    }
    if side_button(ui, p, "Quitter", "", real) {
        st.dialog = Some(Dialog::LeaveGroup(sel));
    }
    ui.add_space(6.0);
    side_button(ui, p, "Créer...", soon, false);
    side_button(ui, p, "Rechercher...", soon, false);
    side_button(ui, p, "Inviter...", soon, false);
}

// ------------------------------------------------------------- contact sets

/// Label of a combo choice (internal names are translated).
fn set_label(choice: &str) -> String {
    match choice {
        contact_sets::ALL_SETS => "Tous".into(),
        contact_sets::NO_SETS => "Sans cercle".into(),
        contact_sets::PSEUDONYM => "Avec un surnom".into(),
        contact_sets::EXTRA_AVS => "Non-amis".into(),
        name => name.into(),
    }
}

fn sets_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    settings: &ContactsSettings,
    pics: &Pics,
    actions: &mut Vec<ContactsAction>,
) {
    let set_names = world.contact_sets.set_names();
    let choice = match &st.set_choice {
        Some(c) if ContactSets::is_internal(c) || world.contact_sets.is_valid(c) => c.clone(),
        _ => set_names.first().cloned().unwrap_or_else(|| contact_sets::ALL_SETS.to_owned()),
    };
    st.set_choice = Some(choice.clone());
    let mutable = !ContactSets::is_internal(&choice);
    let friend_ids: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let social = &world.social;
    let mut members = world.contact_sets.members(&choice, &friend_ids, |id| social.is_friend(id));
    let o = social.avatar_names.options;
    let name_of = |id: &Uuid| {
        social
            .avatar_names
            .get(id)
            .map(|n| n.complete(&o))
            .or_else(|| social.names.get(id).cloned())
            .unwrap_or_else(|| "…".into())
    };
    let online = |id: &Uuid| social.friends.iter().any(|f| f.id == *id && f.online);
    let count = members.len();
    let filter = st.set_filter.trim().to_lowercase();
    members.retain(|id| filter.is_empty() || name_of(id).to_lowercase().contains(&filter));
    // FSPanelContactSets::updateAvatarListSorting
    if world.contact_sets.sorts_by_online(&choice) {
        members.sort_by_cached_key(|id| (!online(id), name_of(id).to_lowercase()));
    } else {
        members.sort_by_cached_key(|id| name_of(id).to_lowercase());
    }
    let rows: Vec<(Uuid, String, bool, Option<Color32>)> = members
        .iter()
        .map(|id| {
            let color = world.contact_sets.friend_color(id, |x| social.is_friend(x)).map(color32);
            (*id, name_of(id), online(id), color)
        })
        .collect();
    for (id, ..) in &rows {
        world.social.want_name(*id);
    }
    st.set_sel.retain(|id| members.contains(id));
    let order = members.clone();
    let h = fill_height(ui, DEFAULT_TAB_H).max(120.0);
    ui.horizontal_top(|ui| {
        let list_w = (ui.available_width() - BUTTONS_W - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(list_w);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let combo_w = list_w - 3.0 * 24.0;
                egui::ComboBox::from_id_salt("contact_set_combo")
                    .width(combo_w - 8.0)
                    .selected_text(set_label(&choice))
                    .show_ui(ui, |ui| {
                        for s in &set_names {
                            if ui.selectable_label(choice == *s, s).clicked() {
                                st.set_choice = Some(s.clone());
                                st.set_sel.clear();
                            }
                        }
                        if !set_names.is_empty() {
                            ui.separator();
                        }
                        for s in [
                            contact_sets::ALL_SETS,
                            contact_sets::NO_SETS,
                            contact_sets::PSEUDONYM,
                            contact_sets::EXTRA_AVS,
                        ] {
                            if ui.selectable_label(choice == s, set_label(s)).clicked() {
                                st.set_choice = Some(s.to_owned());
                                st.set_sel.clear();
                            }
                        }
                    });
                if icon_button(ui, p, "gear-six", "Configurer le cercle sélectionné", mutable).clicked() && mutable {
                    st.rename = choice.clone();
                    st.config = Some(choice.clone());
                }
                if icon_button(ui, p, "plus", "Ajouter un nouveau cercle", true).clicked() {
                    st.dialog = Some(Dialog::AddSet("Nouveau cercle de contacts".into()));
                }
                if icon_button(ui, p, "trash", "Supprimer le cercle sélectionné", mutable).clicked() && mutable {
                    st.dialog = Some(Dialog::RemoveSet(choice.clone()));
                }
            });
            ui.add_space(3.0);
            widgets::search_field(ui, &mut st.set_filter, "Filtrer les personnes", list_w);
            ui.add_space(3.0);
            let list_h = (fill_height(ui, DEFAULT_TAB_H - 60.0) - footer_or_default(st.sets_footer)).max(60.0);
            let list = egui::Frame::new().fill(p.field.gamma_multiply(0.6)).show(ui, |ui| {
                ui.set_width(list_w);
                egui::ScrollArea::vertical()
                    .id_salt("contact_set_list")
                    .auto_shrink([false, false])
                    .max_height(list_h)
                    .min_scrolled_height(list_h)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if rows.is_empty() {
                            ui.label(RichText::new("Ce cercle est vide.").size(12.0).color(p.muted));
                        }
                        let row_h = if settings.set_list_icons { 22.0 } else { ROW_H };
                        for (id, name, on, color) in &rows {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(list_w, row_h), Sense::click());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let selected = st.set_sel.contains(id);
                            if selected {
                                ui.painter().rect_filled(rect, 0.0, p.violet.gamma_multiply(0.35));
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 0.0, p.raised.gamma_multiply(0.6));
                            }
                            let mut x = rect.left() + 4.0;
                            if settings.set_list_icons {
                                st.wanted_pics.insert(*id);
                                let icon = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), Vec2::splat(18.0));
                                row_icon(ui, p, icon, pics.avatars.get(id), "user");
                                x = icon.right() + 6.0;
                            }
                            let col = color.unwrap_or(if *on { p.ink } else { p.muted });
                            let galley = egui::WidgetText::from(RichText::new(name.as_str()).size(12.5).color(col)).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                rect.right() - x - 4.0,
                                egui::TextStyle::Body,
                            );
                            ui.painter()
                                .galley(egui::pos2(x, rect.center().y - galley.size().y * 0.5), galley, col);
                            if resp.clicked() {
                                let mods = ui.input(|i| i.modifiers);
                                click_select(&mut st.set_sel, &mut st.set_anchor, &order, *id, mods);
                            }
                            if resp.double_clicked() {
                                actions.push(ContactsAction::Im(*id));
                            }
                            if resp.secondary_clicked() && !selected {
                                st.set_sel = vec![*id];
                                st.set_anchor = Some(*id);
                            }
                            // the member part of menu_fs_contacts_friends.xml
                            menu::context_menu(&resp, p, |ui| {
                                if menu::item(ui, p, "user-circle", "Voir le profil") {
                                    actions.push(ContactsAction::Profile(*id));
                                }
                                if menu::item(ui, p, "chat-text", "Envoyer un IM...") {
                                    actions.push(ContactsAction::Im(*id));
                                }
                                if menu::item_if(ui, p, "paper-plane-tilt", "Proposer une téléportation", *on) {
                                    actions.push(ContactsAction::OfferTeleport(*id));
                                }
                            });
                        }
                    });
            });
            ui.add_space(3.0);
            let c = ui.label(RichText::new(format!("Membres : {count}")).size(12.0).color(p.muted));
            st.sets_footer = c.rect.bottom() - list.response.rect.bottom();
        });
        ui.vertical(|ui| {
            ui.set_min_height(h);
            ui.spacing_mut().item_spacing.y = 3.0;
            set_buttons(ui, p, world, st, &choice, &set_names, actions);
        });
    });
}

/// Buttons of the contact set panel (FSPanelContactSets::resetControls).
fn set_buttons(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    choice: &str,
    set_names: &[String],
    actions: &mut Vec<ContactsAction>,
) {
    let mutable = !ContactSets::is_internal(choice);
    let sel = st.set_sel.clone();
    let has_sel = !sel.is_empty() && sel.len() <= MAX_SELECT;
    if side_button(
        ui,
        p,
        "Ajouter...",
        "Ajouter un résident au cercle actuellement sélectionné",
        mutable,
    ) {
        st.pick_for = Some(PickFor::Set(choice.to_owned()));
        st.picker.open(PICKER_ID, true);
    }
    let move_btn = side_button_resp(ui, p, "Déplacer...", mutable && has_sel).on_hover_text("Déplacer ce résident vers un autre cercle");
    egui::Popup::menu(&move_btn).show(|ui| {
        let others: Vec<&String> = set_names.iter().filter(|s| s.as_str() != choice).collect();
        if others.is_empty() {
            ui.add_enabled(false, egui::Button::new("Aucun autre cercle"));
        }
        for to in others {
            if ui.button(to).clicked() {
                let social = &world.social;
                world.contact_sets.move_to_set(&sel, choice, to, |id| social.is_friend(id));
                ui.close();
            }
        }
    });
    if side_button(
        ui,
        p,
        "Supprimer...",
        "Supprimer ce résident du cercle actuellement sélectionné",
        mutable && has_sel,
    ) {
        st.dialog = Some(Dialog::RemoveFromSet {
            set: choice.to_owned(),
            ids: sel.clone(),
        });
    }
    if side_button(ui, p, "Profil...", "Afficher le profil du résident", has_sel) {
        for id in &sel {
            actions.push(ContactsAction::Profile(*id));
        }
    }
    let im_tip = if sel.len() > 1 {
        "Conférence à plusieurs : à venir dans Aurora"
    } else {
        "Ouvrir une fenêtre de conversation privée"
    };
    if side_button(ui, p, "IM...", im_tip, sel.len() == 1) {
        actions.push(ContactsAction::Im(sel[0]));
    }
    if side_button(
        ui,
        p,
        "Téléporter...",
        "Proposer une téléportation à votre emplacement actuel",
        has_sel,
    ) {
        for id in &sel {
            actions.push(ContactsAction::OfferTeleport(*id));
        }
    }
    ui.add_space(6.0);
    if side_button(ui, p, "Surnommer...", "Donner un surnom à ce résident", has_sel) {
        let alias = sel
            .first()
            .and_then(|id| world.contact_sets.pseudonym(id))
            .filter(|a| *a != contact_sets::DN_REMOVED)
            .unwrap_or_default()
            .to_owned();
        st.dialog = Some(Dialog::SetAlias { ids: sel.clone(), alias });
    }
    let any_alias = sel.iter().any(|id| world.contact_sets.pseudonym(id).is_some());
    if side_button(ui, p, "Suppr Surnom", "Supprimer le surnom de ce résident", has_sel && any_alias) {
        let social = &world.social;
        world.contact_sets.clear_pseudonym(&sel, |id| social.is_friend(id));
    }
    let any_dn_removed = sel.iter().any(|id| world.contact_sets.has_display_name_removed(id));
    if side_button(
        ui,
        p,
        "Suppr. DN",
        "Supprime le nom d'affichage de ce résident",
        has_sel && !any_dn_removed,
    ) {
        let social = &world.social;
        world.contact_sets.remove_display_name(&sel, |id| social.is_friend(id));
    }
}

/// FSFloaterContactSetConfiguration: name, color, sorting; global options
/// of the contact sets.
fn set_config_window(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi, settings: &mut ContactsSettings) {
    let Some(set) = st.config.clone() else {
        return;
    };
    let Some(cfg) = world.contact_sets.set(&set).cloned() else {
        st.config = None;
        return;
    };
    let mut open = true;
    widgets::Floater::new(
        "contact_set_config",
        format!("Configurer : {set}"),
        ctx.content_rect().center() - Vec2::new(160.0, 140.0),
        Vec2::new(320.0, 280.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.rename).desired_width(200.0));
            if widgets::flat_button(ui, p, "Renommer").clicked() {
                let new = st.rename.trim().to_owned();
                if new != set {
                    if world.contact_sets.rename_set(&set, &new) {
                        st.config = Some(new.clone());
                        if st.set_choice.as_deref() == Some(set.as_str()) {
                            st.set_choice = Some(new);
                        }
                    } else {
                        world.system_message(format!(
                            "Impossible de renommer le groupe '{set}' en '{new}' car un groupe portant le même nom existe déjà ou le nouveau nom n'est pas valide."
                        ));
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut c = cfg.color;
            if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                world.contact_sets.set_color(&set, c);
            }
            ui.label(RichText::new("Couleur").size(12.5).color(p.ink));
        });
        let mut sort = cfg.sort_by_online;
        if ui.checkbox(&mut sort, "Trier par statut en ligne").changed() {
            world.contact_sets.set_sort_by_online(&set, sort);
        }
        ui.separator();
        ui.label(RichText::new("Tous les cercles").size(12.5).strong().color(p.ink));
        ui.horizontal(|ui| {
            let mut c = world.contact_sets.default_color;
            if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                world.contact_sets.set_default_color(c);
            }
            ui.label(RichText::new("Couleur par défaut").size(12.5).color(p.ink));
        });
        ui.checkbox(&mut settings.set_list_icons, "Afficher les icônes de profil dans les listes des cercles");
        ui.checkbox(&mut settings.colorize_friends, "Couleur de la liste d'amis basée sur le cercle");
    });
    if !open {
        st.config = None;
    }
}

// ------------------------------------------------------------------ dialogs

/// The pending confirmation or prompt, as a modal window.
fn dialogs(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi, actions: &mut Vec<ContactsAction>) {
    let Some(d) = st.dialog.as_mut() else {
        return;
    };
    let name = |id: &Uuid| world.social.name_of(id);
    let (title, text, ok, has_input) = match d {
        Dialog::RemoveFriends(ids) if ids.len() == 1 => (
            "Supprimer un ami",
            format!("Voulez-vous supprimer {} de votre liste d'amis ?", name(&ids[0])),
            "OK",
            false,
        ),
        Dialog::RemoveFriends(_) => (
            "Supprimer des amis",
            "Voulez-vous supprimer plusieurs résidents de votre liste d'amis ?".into(),
            "OK",
            false,
        ),
        Dialog::ModifyRights { friend, grant: true, .. } => (
            "Droits de modification",
            format!(
                "Lorsque vous accordez des droits de modification à un autre résident, vous lui permettez de changer, \
                 supprimer ou prendre n'importe lequel de vos objets dans Second Life. Réfléchissez bien avant d'accorder \
                 ces droits.\nVoulez-vous vraiment accorder des droits de modification à {} ?",
                name(friend)
            ),
            "Oui",
            false,
        ),
        Dialog::ModifyRights { friend, .. } => (
            "Droits de modification",
            format!("Voulez-vous retirer les droits de modification à {} ?", name(friend)),
            "Oui",
            false,
        ),
        Dialog::LeaveGroup(g) => {
            let n = world.groups.group(g).map(|g| g.name.clone()).unwrap_or_default();
            ("Quitter le groupe", format!("Quitter le groupe '{n}' ?"), "Quitter", false)
        }
        Dialog::Friendship { to, .. } => (
            "Devenir amis",
            format!(
                "Vous pouvez suivre les déplacements de vos amis sur la carte et voir lorsqu'ils se connectent.\n\n\
                 Proposer à {} de devenir votre ami(e) ?",
                name(to)
            ),
            "OK",
            true,
        ),
        Dialog::TeleportRequest { to, .. } => (
            "Demander une téléportation",
            format!("Demander à {} de vous téléporter ?", name(to)),
            "Envoyer",
            true,
        ),
        Dialog::AddSet(_) => (
            "Nouveau cercle",
            "Créer un nouveau groupe de contacts avec le nom :".into(),
            "Créer",
            true,
        ),
        Dialog::RemoveSet(s) => (
            "Supprimer un cercle",
            format!("Êtes-vous sûr de vouloir supprimer {s} ? Vous ne pourrez pas le restaurer."),
            "OK",
            false,
        ),
        Dialog::RemoveFromSet { set, ids } => {
            let target = if ids.len() > 1 { ids.len().to_string() } else { name(&ids[0]) };
            let text = if ids.len() > 1 {
                format!("Êtes-vous sûr de vouloir supprimer ces avatars {target} de {set} ?")
            } else {
                format!("Êtes-vous sûr de vouloir supprimer {target} de {set} ?")
            };
            ("Retirer du cercle", text, "OK", false)
        }
        Dialog::SetAlias { ids, .. } => {
            let text = if ids.len() > 1 {
                format!("Entrez un alias pour {} avatars :", ids.len())
            } else {
                format!("Entrez un alias pour {} :", name(&ids[0]))
            };
            ("Surnom", text, "Créer", true)
        }
    };
    let mut done: Option<bool> = None;
    let modal = egui::Modal::new(egui::Id::new("contacts_dialog")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(&text).size(12.5).color(p.muted));
        if has_input {
            ui.add_space(6.0);
            let buf = match d {
                Dialog::Friendship { message, .. } | Dialog::TeleportRequest { message, .. } => message,
                Dialog::AddSet(s) => s,
                Dialog::SetAlias { alias, .. } => alias,
                _ => unreachable!("dialog without input"),
            };
            let r = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let yes = ui.add(
                egui::Button::new(RichText::new(ok).color(Color32::WHITE))
                    .fill(p.violet)
                    .corner_radius(CornerRadius::same(2)),
            );
            if yes.clicked() {
                done = Some(true);
            }
            if widgets::flat_button(ui, p, "Annuler").clicked() {
                done = Some(false);
            }
        });
    });
    if modal.should_close() && done.is_none() {
        done = Some(false);
    }
    let Some(confirmed) = done else {
        return;
    };
    let Some(d) = st.dialog.take() else {
        return;
    };
    if !confirmed {
        return;
    }
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let is_friend = |id: &Uuid| friends.contains(id);
    match d {
        Dialog::RemoveFriends(ids) => {
            for id in ids {
                actions.push(ContactsAction::Net(NetCommand::TerminateFriendship(id)));
            }
        }
        Dialog::ModifyRights { friend, rights, .. } => {
            actions.push(ContactsAction::Net(NetCommand::GrantUserRights { friend, rights }));
        }
        Dialog::LeaveGroup(g) => actions.push(ContactsAction::Net(NetCommand::LeaveGroup(g))),
        Dialog::Friendship { to, message } => actions.push(ContactsAction::OfferFriendship { to, message }),
        Dialog::TeleportRequest { to, message } => actions.push(ContactsAction::Net(NetCommand::SendImDialog {
            to,
            dialog: IM_TELEPORT_REQUEST,
            id: Uuid::nil(),
            message,
            bucket: Vec::new(),
        })),
        Dialog::AddSet(name) => {
            if world.contact_sets.add_set(&name) {
                st.set_choice = Some(name.trim().to_owned());
            }
        }
        Dialog::RemoveSet(set) => {
            world.contact_sets.remove_set(&set, is_friend);
            st.set_choice = None;
        }
        Dialog::RemoveFromSet { set, ids } => {
            world.contact_sets.remove_from_set(&ids, &set, is_friend);
            st.set_sel.clear();
        }
        Dialog::SetAlias { ids, alias } => world.contact_sets.set_pseudonym(&ids, &alias, is_friend),
    }
}

/// IM_FRIENDSHIP_OFFERED as sent by LLAvatarActions::requestFriendship.
pub fn friendship_offer(to: Uuid, message: String, calling_cards: Uuid) -> NetCommand {
    NetCommand::SendImDialog {
        to,
        dialog: IM_FRIENDSHIP_OFFERED,
        id: calling_cards,
        message,
        bucket: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn selection_rules() {
        let order: Vec<Uuid> = (1..=5).map(id).collect();
        let (mut sel, mut anchor) = (Vec::new(), None);
        let none = egui::Modifiers::NONE;
        click_select(&mut sel, &mut anchor, &order, id(2), none);
        assert_eq!(sel, vec![id(2)]);
        click_select(&mut sel, &mut anchor, &order, id(4), egui::Modifiers::SHIFT);
        assert_eq!(sel, vec![id(2), id(3), id(4)]);
        click_select(&mut sel, &mut anchor, &order, id(3), egui::Modifiers::COMMAND);
        assert_eq!(sel, vec![id(2), id(4)]);
        click_select(&mut sel, &mut anchor, &order, id(5), none);
        assert_eq!(sel, vec![id(5)]);
    }

    #[test]
    fn map_right_needs_online_status() {
        let all = rights::ONLINE_STATUS | rights::MAP_LOCATION | rights::MODIFY_OBJECTS;
        assert_eq!(rights_after(all, rights::ONLINE_STATUS, false), rights::MODIFY_OBJECTS);
        assert_eq!(
            rights_after(0, rights::MAP_LOCATION, true),
            rights::ONLINE_STATUS | rights::MAP_LOCATION
        );
        assert_eq!(
            rights_after(rights::ONLINE_STATUS, rights::MODIFY_OBJECTS, true),
            rights::ONLINE_STATUS | rights::MODIFY_OBJECTS
        );
        assert_eq!(
            rights_after(all, rights::MAP_LOCATION, false),
            rights::ONLINE_STATUS | rights::MODIFY_OBJECTS
        );
    }

    #[test]
    fn full_name_formats() {
        let o = NameOptions::default();
        let n = AvatarName {
            username: "janedoe".into(),
            display_name: "Jane".into(),
            legacy_first: "janedoe".into(),
            legacy_last: "Resident".into(),
            ..Default::default()
        };
        assert_eq!(full_name(&n, &o, true), "Jane (janedoe)");
        assert_eq!(full_name(&n, &o, false), "janedoe (Jane)");
        let default = AvatarName { is_default: true, ..n };
        assert_eq!(full_name(&default, &o, true), "janedoe");
    }

    #[test]
    fn group_count() {
        assert_eq!(group_count_text(11, 50), "Vous faites partie de 11 groupes (39 restant).");
        assert_eq!(group_count_text(3, 0), "Vous faites partie de 3 groupes.");
    }
}
