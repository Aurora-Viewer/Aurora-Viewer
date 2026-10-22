//! "People" of the Region / Estate window: the estate's managers, allowed
//! residents, allowed groups and banned residents of the Access tab
//! (LLPanelEstateAccess in newview/llfloaterregioninfo.cpp, originally
//! LGPL 2.1), as rows with a picture, a search and a box to select several.
//! The rules are the tab's (updateControls): the owner changes every list, a
//! manager all but the managers', nobody bans on a Linden estate. A change
//! asks first "this estate or all of them" (EstateAllowedAgentAdd, …) in one
//! dialog, the ChangeLindenAccess warning before it on a Linden estate, then
//! goes out as `estateaccessdelta` (accessCoreConfirm); the simulator's
//! `setaccess` brings the lists back. Copy, CSV export and import are FS:PP's.

use super::{RegionUi, View};
use crate::theme::Palette;
use crate::ui::avatar_picker::AvatarPicker;
use crate::ui::land::files;
use crate::ui::sheet::{self, Person, ROW_H, list_title, matches, names, person_head, person_row, semibold};
use crate::ui::widgets::{self, ActionKind, DialogButton};
use crate::world::World;
use crate::world::estate::Role;
use aurora_net::estate::{
    BannedAgent, ESTATE_LAST_LINDEN, ESTATE_MAX_ACCESS_IDS, ESTATE_MAX_BANNED_IDS, ESTATE_MAX_GROUP_IDS, ESTATE_MAX_MANAGERS, EstateAccess,
    estate_access as ea,
};
use egui::{RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The four lists, in the mockup's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum List {
    Managers,
    Allowed,
    Groups,
    Banned,
}

impl List {
    fn index(self) -> usize {
        self as usize
    }

    fn of_index(i: u32) -> Option<List> {
        [List::Managers, List::Allowed, List::Groups, List::Banned].get(i as usize).copied()
    }

    fn title(self) -> &'static str {
        match self {
            List::Managers => aurora_i18n::tr!("region-people-managers"),
            List::Allowed => aurora_i18n::tr!("region-people-allowed"),
            List::Groups => aurora_i18n::tr!("region-people-groups"),
            List::Banned => aurora_i18n::tr!("region-people-banned"),
        }
    }

    /// The list in a sentence ("… is already in the allowed residents").
    fn noun(self) -> &'static str {
        match self {
            List::Managers => aurora_i18n::tr!("region-people-list-managers"),
            List::Allowed => aurora_i18n::tr!("region-people-list-allowed"),
            List::Groups => aurora_i18n::tr!("region-people-list-groups"),
            List::Banned => aurora_i18n::tr!("region-people-list-banned"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            List::Managers => "crown-filled",
            List::Allowed => "user-check",
            List::Groups => "users-group",
            List::Banned => "ban",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            List::Managers => aurora_i18n::tr!("region-people-managers-hint"),
            List::Allowed => aurora_i18n::tr!("region-people-allowed-hint"),
            List::Groups => aurora_i18n::tr!("region-people-groups-hint"),
            List::Banned => aurora_i18n::tr!("region-people-banned-hint"),
        }
    }

    /// ESTATE_MAX_MANAGERS, _ACCESS_IDS, _GROUP_IDS (the count the panel
    /// shows for the groups), _BANNED_IDS.
    pub(crate) fn limit(self) -> usize {
        match self {
            List::Managers => ESTATE_MAX_MANAGERS,
            List::Allowed => ESTATE_MAX_ACCESS_IDS,
            List::Groups => ESTATE_MAX_GROUP_IDS,
            List::Banned => ESTATE_MAX_BANNED_IDS,
        }
    }

    pub(crate) fn add_flag(self) -> u32 {
        match self {
            List::Managers => ea::MANAGER_ADD,
            List::Allowed => ea::ALLOWED_AGENT_ADD,
            List::Groups => ea::ALLOWED_GROUP_ADD,
            List::Banned => ea::BANNED_AGENT_ADD,
        }
    }

    pub(crate) fn remove_flag(self) -> u32 {
        match self {
            List::Managers => ea::MANAGER_REMOVE,
            List::Allowed => ea::ALLOWED_AGENT_REMOVE,
            List::Groups => ea::ALLOWED_GROUP_REMOVE,
            List::Banned => ea::BANNED_AGENT_REMOVE,
        }
    }

    /// The export's file name (onClickExport…List).
    fn file(self) -> &'static str {
        match self {
            List::Managers => "estate_manager_list.csv",
            List::Allowed => "estate_allowed_residents.csv",
            List::Groups => "estate_allowed_groups.csv",
            List::Banned => "estate_banned_residents.csv",
        }
    }

    fn ids(self, a: &EstateAccess) -> Vec<Uuid> {
        match self {
            List::Managers => a.managers.clone(),
            List::Allowed => a.allowed_agents.clone(),
            List::Groups => a.allowed_groups.clone(),
            List::Banned => a.banned.iter().map(|b| b.id).collect(),
        }
    }
}

/// Who changes a list (updateControls): the owner every one, a manager all
/// but the managers' ("estate managers can't add estate managers"); nobody
/// bans nor lifts a ban on a Linden estate. Copy, import and export follow.
pub(crate) fn can_change(list: List, shown: Role, linden: bool) -> bool {
    let role_ok = match shown {
        Role::Resident => false,
        Role::Manager => list != List::Managers,
        Role::Owner => true,
    };
    role_ok && !(linden && list == List::Banned)
}

/// The scope flag of "All estates" (accessCoreConfirm): the estates the
/// owner owns, or those a manager manages for this owner.
pub(crate) fn scope_flags(role: Role, all: bool) -> u32 {
    match (all, role) {
        (true, Role::Owner) => ea::APPLY_TO_ALL_ESTATES,
        (true, Role::Manager) => ea::APPLY_TO_MANAGED_ESTATES,
        _ => 0,
    }
}

/// What of the residents picked can be added (accessAddCore3).
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Picked {
    /// Those to send.
    pub ids: Vec<Uuid>,
    /// Already in the list (AgentIsAlreadyInList).
    pub already: Vec<Uuid>,
    /// Managers, who cannot be banned (ProblemBanningEstateManager).
    pub managers: Vec<Uuid>,
    /// The estate owner, who cannot be banned (OwnerCanNotBeDenied).
    pub owner: bool,
}

/// accessAddCore3 for `list` holding `current`: Err(excess) when the batch
/// does not fit (MaxAgentOnRegionBatch); else who is already there, managers
/// and the owner kept out of the bans, and the rest. The owner check is
/// accessCoreConfirm's, made here before anything is sent.
pub(crate) fn pick(list: List, picked: &[Uuid], current: &[Uuid], managers: &[Uuid], owner: Uuid) -> Result<Picked, usize> {
    let total = picked.len() + current.len();
    if matches!(list, List::Allowed | List::Banned) && total > list.limit() {
        return Err(total - list.limit());
    }
    let mut out = Picked::default();
    for id in picked {
        if out.ids.contains(id) {
            continue;
        }
        let ban = list == List::Banned;
        if ban && *id == owner {
            out.owner = true;
        } else if ban && managers.contains(id) {
            out.managers.push(*id);
        } else if current.contains(id) {
            out.already.push(*id);
        } else {
            out.ids.push(*id);
        }
    }
    Ok(out)
}

/// The FS:PP export: "Name,UUID", and for the banned residents their last
/// login, ban date and who banned them, as the list shows them.
pub(crate) fn export_csv(rows: &[(Uuid, String, [String; 3])], banned: bool) -> String {
    let mut out = String::from("Name,UUID");
    if banned {
        out.push_str(",Last Login,Ban Date,Banned By");
    }
    out.push('\n');
    for (id, name, extra) in rows {
        out.push_str(&format!("{name},{id}"));
        if banned {
            out.push_str(&format!(",{},{},{}", extra[0], extra[1], extra[2]));
        }
        out.push('\n');
    }
    out
}

/// At most this many names in a dialog, then "and N more".
const LISTED: usize = 5;

/// "Nova, Tess and 3 more".
fn listed(names: &[String]) -> String {
    let shown = names.iter().take(LISTED).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > LISTED {
        aurora_i18n::tr!("region-people-and-more", names = shown, count = (names.len() - LISTED) as i64)
    } else {
        shown
    }
}

/// What the page asks or tells.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PeopleAsk {
    /// ChangeLindenAccess before changing a Linden estate's lists: the
    /// residents to remove, or None for an addition (the picker comes next).
    Linden { list: List, remove: Option<Vec<Uuid>> },
    /// "This estate or all of them" (EstateAllowedAgentAdd, …), with the
    /// residents or groups concerned.
    Scope { list: List, add: bool, ids: Vec<Uuid>, all: bool },
    /// A notice, then what comes after it.
    Notice { text: String, then: Option<Box<PeopleAsk>> },
}

fn notice(text: impl Into<String>) -> PeopleAsk {
    PeopleAsk::Notice {
        text: text.into(),
        then: None,
    }
}

/// What AURORA_DEMO_REGION opens on People (`demo_region.rs`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeopleDemo {
    /// A row ticked, in the allowed residents.
    pub select: Option<Uuid>,
    pub ask: Option<PeopleAsk>,
    /// The resident picker opened on the allowed residents.
    pub pick: bool,
}

/// What the page keeps between frames.
#[derive(Default)]
pub(super) struct PeopleState {
    filter: [String; 4],
    selected: [HashSet<Uuid>; 4],
    picker: AvatarPicker,
    /// The list the resident picker adds to.
    picking: Option<List>,
    /// The group picker is open (LLFloaterGroupPicker).
    groups_open: bool,
    ask: Option<PeopleAsk>,
    file: Option<files::Job>,
    pub demo: Option<PeopleDemo>,
}

impl PeopleState {
    /// Another region, or the window closed: nothing of the lists is kept
    /// but a demo waiting for its lists.
    pub(super) fn reset(&mut self) {
        let demo = self.demo.take();
        *self = PeopleState {
            demo,
            ..Default::default()
        };
    }
}

/// Is the estate a Linden one (isLindenEstate).
fn linden(world: &World) -> bool {
    world.estate.estate.as_ref().is_some_and(|e| e.id <= ESTATE_LAST_LINDEN)
}

/// AURORA_DEMO_REGION on People, once the demo simulator sent the lists.
fn demo(s: &mut RegionUi, world: &World) {
    if !matches!(world.estate.access, Some(Some(_))) {
        return;
    }
    let Some(d) = s.people.demo.take() else {
        return;
    };
    if let Some(id) = d.select {
        s.people.selected[List::Allowed.index()].insert(id);
    }
    if d.pick {
        open_picker(&mut s.people, List::Allowed);
    }
    s.people.ask = d.ask;
}

/// The page under the header.
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    avatars: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    compact: bool,
) {
    demo(s, world);
    poll_files(s, v, world);
    if !v.manages() {
        // "View as" a resident: what the simulator would not send
        let mut c = sheet::card_begin(ui);
        let owner = names(world, &v.owner).0;
        empty(
            &mut c.child,
            p,
            aurora_i18n::tr!("region-people-not-sent-title"),
            &format!(
                "{}\n{}",
                aurora_i18n::tr!("region-people-not-sent"),
                aurora_i18n::tr!("region-people-owner-is", name = owner)
            ),
        );
        sheet::card_end(ui, p, c);
        return;
    }
    let access = match world.estate.access.clone() {
        None => {
            ui.label(RichText::new(aurora_i18n::tr!("region-people-loading")).size(12.0).color(p.muted));
            return;
        }
        Some(None) => {
            let mut c = sheet::card_begin(ui);
            empty(&mut c.child, p, "", aurora_i18n::tr!("region-people-refused"));
            sheet::card_end(ui, p, c);
            return;
        }
        Some(Some(a)) => a,
    };
    let pics = Pics { avatars, images };
    if compact {
        for list in [List::Managers, List::Allowed, List::Groups, List::Banned] {
            let mut c = sheet::card_begin(ui);
            c.child.spacing_mut().item_spacing.y = 6.0;
            list_card(&mut c.child, p, s, v, world, &pics, &access, list);
            sheet::card_end(ui, p, c);
        }
        return;
    }
    for (left, right) in [(List::Managers, List::Allowed), (List::Groups, List::Banned)] {
        sheet::cards(
            ui,
            p,
            world,
            s,
            1.0,
            |ui, world, s| list_card(ui, p, s, v, world, &pics, &access, left),
            |ui, world, s| list_card(ui, p, s, v, world, &pics, &access, right),
        );
        ui.add_space(12.0);
    }
}

/// The pictures the rows draw: residents' profile pictures, groups'
/// insignia.
struct Pics<'a> {
    avatars: &'a HashMap<Uuid, egui::TextureHandle>,
    images: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// A sentence in the middle of a card, under a large faint icon and its
/// title.
fn empty(ui: &mut egui::Ui, p: &Palette, title: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(14.0);
        widgets::glyph(ui, "users", p.muted_dim, 30.0);
        ui.add_space(6.0);
        if !title.is_empty() {
            ui.label(semibold(ui, title, 12.5, p.ink));
            ui.add_space(2.0);
        }
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
        ui.add_space(14.0);
    });
}

/// One row as the list shows it: the id, its name and user name, and for a
/// banned resident (ban date and who, last login, raw dates for the export).
struct Row {
    id: Uuid,
    name: String,
    user: Option<String>,
    banned: Option<([String; 2], [String; 3])>,
}

/// "9 Oct 2026" of a capability's "2026-10-09 10:00"; the raw text when it
/// does not read.
fn day_of(date: &str) -> String {
    let mut parts = date.split(['-', ' ']);
    let (y, m, d) = (parts.next(), parts.next(), parts.next());
    match (
        y.and_then(|v| v.parse::<i64>().ok()),
        m.and_then(|v| v.parse::<usize>().ok()),
        d.and_then(|v| v.parse::<i64>().ok()),
    ) {
        (Some(y), Some(m @ 1..=12), Some(d)) => aurora_i18n::tr!(
            "ui-land-overview-date",
            day = d.to_string(),
            month = crate::ui::land::months()[m - 1].to_owned(),
            year = y.to_string()
        ),
        _ => date.to_owned(),
    }
}

fn rows(world: &mut World, list: List, access: &EstateAccess) -> Vec<Row> {
    let na = aurora_i18n::tr!("region-people-not-known");
    let mut out: Vec<Row> = match list {
        List::Groups => access
            .allowed_groups
            .iter()
            .map(|id| {
                let groups = world.groups.groups.clone();
                let name = world
                    .land
                    .group_name(id, &groups)
                    .unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
                Row {
                    id: *id,
                    name,
                    user: None,
                    banned: None,
                }
            })
            .collect(),
        List::Banned => access
            .banned
            .iter()
            .map(|b: &BannedAgent| {
                let (name, user) = names(world, &b.id);
                let by = match b.banned_by {
                    Some(by) if by == world.agent_id => aurora_i18n::tr!("region-people-you").to_owned(),
                    Some(by) => names(world, &by).0,
                    None => na.to_owned(),
                };
                let on = b.banned_on.as_deref().map_or_else(|| na.to_owned(), day_of);
                let login = b.last_login.as_deref().map_or_else(|| na.to_owned(), day_of);
                let raw = [
                    b.last_login.clone().unwrap_or_else(|| na.to_owned()),
                    b.banned_on.clone().unwrap_or_else(|| na.to_owned()),
                    by.clone(),
                ];
                Row {
                    id: b.id,
                    name,
                    user,
                    banned: Some(([aurora_i18n::tr!("region-people-banned-on", date = on, by = by), login], raw)),
                }
            })
            .collect(),
        _ => list
            .ids(access)
            .into_iter()
            .map(|id| {
                let (name, user) = names(world, &id);
                Row {
                    id,
                    name,
                    user,
                    banned: None,
                }
            })
            .collect(),
    };
    // sortByName
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

#[allow(clippy::too_many_arguments)]
fn list_card(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    pics: &Pics<'_>,
    access: &EstateAccess,
    list: List,
) {
    let linden = linden(world);
    let can = can_change(list, v.shown, linden);
    let all = rows(world, list, access);
    let full = all.len() >= list.limit();
    let count = format!("{} / {}", all.len(), list.limit());
    let i = list.index();
    list_title(ui, p, list.icon(), list.title(), &count, |ui| {
        let label = if list == List::Banned {
            aurora_i18n::tr!("region-people-ban")
        } else {
            aurora_i18n::tr!("common-add")
        };
        if widgets::action_button_if(ui, p, "plus", label, ActionKind::Flat, Vec2::new(0.0, 26.0), can).clicked() {
            add(s, list, all.len(), linden);
        }
    });
    // why the list is greyed
    if list == List::Managers && v.shown == Role::Manager {
        sheet::lock_line(ui, p, aurora_i18n::tr!("region-people-owner-only"));
    } else if list == List::Banned && linden && v.manages() {
        sheet::lock_line(ui, p, aurora_i18n::tr!("region-people-linden-ban"));
    }
    // search, copy, import, export
    let copy = list != List::Managers;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let buttons = if copy { 3.0 } else { 2.0 };
        let width = ui.available_width() - buttons * 32.0;
        widgets::search_box(ui, p, &mut s.people.filter[i], aurora_i18n::tr!("common-search"), width);
        if copy && widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("region-people-copy"), can && !all.is_empty()) {
            // copyListToClipboard: the names, one a line
            let text = all.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join("\n");
            ui.ctx().copy_text(text);
        }
        if widgets::icon_button(ui, p, "file-import", aurora_i18n::tr!("region-people-import"), can && !full) {
            s.people.file = Some(files::open(i as u32));
        }
        if widgets::icon_button(ui, p, "file-export", aurora_i18n::tr!("region-people-export"), can) {
            if all.is_empty() {
                s.people.ask = Some(notice(aurora_i18n::tr!("region-people-list-empty")));
            } else {
                let csv_rows: Vec<(Uuid, String, [String; 3])> = all
                    .iter()
                    .map(|r| (r.id, r.name.clone(), r.banned.as_ref().map(|b| b.1.clone()).unwrap_or_default()))
                    .collect();
                s.people.file = Some(files::save(list.file(), export_csv(&csv_rows, list == List::Banned)));
            }
        }
    });
    ui.add_space(2.0);
    let width = widgets::scrolled_width(ui);
    if list == List::Banned {
        person_head(
            ui,
            p,
            width,
            aurora_i18n::tr!("region-people-head-resident"),
            aurora_i18n::tr!("region-people-head-banned"),
            aurora_i18n::tr!("region-people-head-login"),
        );
    }
    let filter = s.people.filter[i].clone();
    let shown: Vec<&Row> = all.iter().filter(|r| matches(&filter, &r.name, r.user.as_deref())).collect();
    s.people.selected[i].retain(|id| all.iter().any(|r| r.id == *id));
    egui::ScrollArea::vertical()
        .id_salt(("region_people", i))
        .max_height(ROW_H * 5.5)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if shown.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(aurora_i18n::tr!("status-empty")).size(12.0).color(p.muted_dim));
            }
            for r in &shown {
                let selected = s.people.selected[i].contains(&r.id);
                let insignia = (list == List::Groups)
                    .then(|| world.groups.groups.iter().find(|g| g.id == r.id).map(|g| g.insignia))
                    .flatten()
                    .filter(|id| !id.is_nil());
                let pic = match insignia {
                    Some(t) => pics.images.get(&t),
                    None if list == List::Groups => None,
                    None => pics.avatars.get(&r.id),
                };
                let row = Person {
                    id: r.id,
                    pic,
                    name: &r.name,
                    user: r.user.as_deref(),
                    exp: r.banned.as_ref().map_or("", |b| b.0[1].as_str()),
                    timed: false,
                    mid: r.banned.as_ref().map(|b| b.0[0].as_str()),
                    group: list == List::Groups,
                    selected,
                    can,
                };
                let answer = person_row(ui, p, world, &row, width);
                if answer.toggled {
                    if selected {
                        s.people.selected[i].remove(&r.id);
                    } else {
                        s.people.selected[i].insert(r.id);
                    }
                }
                if answer.wants_pic {
                    match (list, insignia) {
                        (List::Groups, Some(t)) => {
                            s.wanted_images.insert(t);
                        }
                        (List::Groups, None) => {}
                        _ => {
                            s.wanted_pics.insert(r.id);
                        }
                    }
                }
            }
        });
    // foot: what the list is for, or the selection and its removal
    let selected: Vec<Uuid> = all.iter().filter(|r| s.people.selected[i].contains(&r.id)).map(|r| r.id).collect();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        if selected.is_empty() {
            ui.add(egui::Label::new(RichText::new(list.hint()).size(11.5).color(p.muted)).wrap());
            return;
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-people-selected", count = selected.len() as i64))
                .size(11.5)
                .color(p.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("region-people-remove"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                can,
            )
            .clicked()
            {
                remove(s, list, selected.clone(), linden);
            }
        });
    });
}

/// "Add" or "Ban" (onClickAdd…): the list's limit first, the Linden
/// warning on a Linden estate, then the picker.
fn add(s: &mut RegionUi, list: List, count: usize, linden: bool) {
    if count >= list.limit() {
        let key = match list {
            List::Managers => aurora_i18n::tr!("region-people-max-managers", max = list.limit() as i64),
            List::Allowed => aurora_i18n::tr!("region-people-max-allowed", max = list.limit() as i64),
            List::Groups => aurora_i18n::tr!("region-people-max-groups", max = list.limit() as i64),
            List::Banned => aurora_i18n::tr!("region-people-max-banned", max = list.limit() as i64),
        };
        s.people.ask = Some(notice(key));
    } else if linden {
        s.people.ask = Some(PeopleAsk::Linden { list, remove: None });
    } else {
        open_picker(&mut s.people, list);
    }
}

fn open_picker(s: &mut PeopleState, list: List) {
    if list == List::Groups {
        s.groups_open = true;
    } else {
        s.picking = Some(list);
        // multi-select (accessAddCore2)
        s.picker.open("region_avatar_picker", true);
    }
}

/// "Remove…" (accessRemoveCore): the Linden warning, else the scope.
fn remove(s: &mut RegionUi, list: List, ids: Vec<Uuid>, linden: bool) {
    s.people.ask = Some(if linden {
        PeopleAsk::Linden { list, remove: Some(ids) }
    } else {
        PeopleAsk::Scope {
            list,
            add: false,
            ids,
            all: false,
        }
    });
}

/// The residents or the group picked for `list` (accessAddCore3,
/// addAllowedGroup2): what cannot be added is told, the rest asks its scope
/// (a Linden estate's is this one, sent at once).
fn picked(s: &mut RegionUi, v: &View, world: &mut World, list: List, ids: &[Uuid]) {
    let Some(Some(access)) = world.estate.access.clone() else {
        return;
    };
    let current = list.ids(&access);
    let p = match pick(list, ids, &current, &access.managers, v.owner) {
        Err(excess) => {
            s.people.ask = Some(notice(aurora_i18n::tr!(
                "region-people-batch-too-many",
                count = ids.len() as i64,
                max = list.limit() as i64,
                excess = excess as i64
            )));
            return;
        }
        Ok(p) => p,
    };
    let name = |world: &mut World, id: &Uuid| {
        if list == List::Groups {
            let groups = world.groups.groups.clone();
            world.land.group_name(id, &groups).unwrap_or_default()
        } else {
            names(world, id).0
        }
    };
    let mut told: Vec<String> = Vec::new();
    if p.owner {
        told.push(aurora_i18n::tr!("region-people-owner-ban").to_owned());
    }
    if !p.managers.is_empty() {
        let n: Vec<String> = p.managers.iter().map(|id| name(world, id)).collect();
        told.push(aurora_i18n::tr!("region-people-manager-ban", names = listed(&n)));
    }
    if !p.already.is_empty() {
        let n: Vec<String> = p.already.iter().map(|id| name(world, id)).collect();
        told.push(aurora_i18n::tr!(
            "region-people-already",
            names = listed(&n),
            count = n.len() as i64,
            list = list.noun()
        ));
    }
    let next = if p.ids.is_empty() {
        None
    } else if linden(world) {
        // just apply to this estate
        send(world, v, list, true, p.ids.clone(), false);
        None
    } else {
        Some(PeopleAsk::Scope {
            list,
            add: true,
            ids: p.ids,
            all: false,
        })
    };
    s.people.ask = if told.is_empty() {
        next
    } else {
        Some(PeopleAsk::Notice {
            text: told.join("\n"),
            then: next.map(Box::new),
        })
    };
}

/// accessCoreConfirm: one `estateaccessdelta` each, the scope's flag added.
fn send(world: &mut World, v: &View, list: List, add: bool, ids: Vec<Uuid>, all: bool) {
    let op = if add { list.add_flag() } else { list.remove_flag() };
    world.estate.change_access(v.handle, op | scope_flags(v.role, all), ids, true);
}

/// The save and open dialogs' answers (exportListCallback,
/// importListCallback).
fn poll_files(s: &mut RegionUi, v: &View, world: &mut World) {
    let Some(done) = s.people.file.as_ref().and_then(|j| j.try_take()) else {
        return;
    };
    s.people.file = None;
    match done {
        files::Done::Saved(path) => s.people.ask = Some(notice(aurora_i18n::tr!("region-people-exported", path = path))),
        files::Done::Failed(e) => s.people.ask = Some(notice(e)),
        files::Done::Cancelled => {}
        files::Done::Opened(kind, text) => {
            let (Some(list), Some(Some(access))) = (List::of_index(kind), world.estate.access.clone()) else {
                return;
            };
            let ids = crate::ui::land::import_csv(&text);
            let free = list.limit().saturating_sub(list.ids(&access).len());
            s.people.ask = Some(notice(if ids.is_empty() {
                aurora_i18n::tr!("ui-land-access-no-valid-uuid-found-in-the-file").to_owned()
            } else if ids.len() > free {
                aurora_i18n::tr!("region-people-import-too-large", count = ids.len() as i64, max = free as i64)
            } else {
                let count = ids.len() as i64;
                // each alone, for this estate
                world.estate.change_access(v.handle, list.add_flag(), ids, false);
                aurora_i18n::tr!("region-people-imported", count = count)
            }));
        }
    }
}

/// The page's pickers and dialogs, over the window.
pub(super) fn dialogs(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    if let Some(ids) = s.people.picker.show(ctx, p, world)
        && let Some(list) = s.people.picking.take()
    {
        picked(s, v, world, list, &ids);
    }
    if !s.people.picker.open {
        s.people.picking = None;
    }
    group_picker(ctx, p, s, v, world);
    ask(ctx, p, s, v, world);
}

/// LLFloaterGroupPicker without "none": the agent's groups.
fn group_picker(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    if !s.people.groups_open {
        return;
    }
    let groups = world.groups.sorted();
    let mut chosen = None;
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("region_estate_group_picker")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::glyph(ui, "users-group", p.violet_light, 16.0);
            ui.label(semibold(ui, aurora_i18n::tr!("region-people-pick-group"), 13.5, p.ink));
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            if groups.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("region-people-no-groups")).size(12.0).color(p.muted));
            }
            for g in &groups {
                let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                if r.hovered() {
                    ui.painter()
                        .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.35));
                }
                let mut job = semibold(ui, &g.name, 12.5, p.ink);
                job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 16.0);
                let galley = ui.painter().layout_job(job);
                let at = widgets::row_text_origin(ui.painter(), &galley, rect.left() + 8.0, rect.center().y);
                ui.painter().galley(at, galley, p.ink);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    chosen = Some(g.id);
                }
            }
        });
        ui.add_space(12.0);
        let cancel = [DialogButton {
            icon: "",
            label: aurora_i18n::tr!("common-cancel"),
            kind: ActionKind::Flat,
            enabled: true,
        }];
        if widgets::dialog_buttons(ui, p, &cancel, 28.0).is_some() {
            close = true;
        }
    });
    if modal.should_close() {
        close = true;
    }
    if let Some(id) = chosen {
        s.people.groups_open = false;
        picked(s, v, world, List::Groups, &[id]);
    } else if close {
        s.people.groups_open = false;
    }
}

/// The scope question, the Linden warning or a notice.
fn ask(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let Some(mut ask) = s.people.ask.take() else {
        return;
    };
    let names_of = |world: &mut World, list: List, ids: &[Uuid]| -> String {
        let n: Vec<String> = ids
            .iter()
            .map(|id| {
                if list == List::Groups {
                    let groups = world.groups.groups.clone();
                    world.land.group_name(id, &groups).unwrap_or_default()
                } else {
                    names(world, id).0
                }
            })
            .collect();
        listed(&n)
    };
    // (icon, danger, title, text, main button, cancel shown)
    let (icon, danger, title, text, main): (&str, bool, String, String, Option<(&str, ActionKind)>) = match &ask {
        PeopleAsk::Notice { text, .. } => (
            "info-circle-filled",
            false,
            aurora_i18n::tr!("region-people-notice-title").to_owned(),
            text.clone(),
            None,
        ),
        PeopleAsk::Linden { .. } => (
            "alert-triangle-filled",
            true,
            aurora_i18n::tr!("region-people-ask-linden-title").to_owned(),
            aurora_i18n::tr!("region-people-ask-linden").to_owned(),
            Some((aurora_i18n::tr!("region-people-ask-linden-ok"), ActionKind::Danger)),
        ),
        PeopleAsk::Scope { list, add, ids, .. } => {
            let who = names_of(world, *list, ids);
            let (title, text, ok, kind) = match (list, add) {
                (List::Managers, true) => (
                    aurora_i18n::tr!("region-people-ask-add-managers-title"),
                    aurora_i18n::tr!("region-people-ask-add-managers", names = who),
                    aurora_i18n::tr!("common-add"),
                    ActionKind::Primary,
                ),
                (List::Allowed, true) => (
                    aurora_i18n::tr!("region-people-ask-add-allowed-title"),
                    aurora_i18n::tr!("region-people-ask-add-allowed", names = who),
                    aurora_i18n::tr!("region-people-ok-allow"),
                    ActionKind::Primary,
                ),
                (List::Groups, true) => (
                    aurora_i18n::tr!("region-people-ask-add-groups-title"),
                    aurora_i18n::tr!("region-people-ask-add-groups", names = who),
                    aurora_i18n::tr!("region-people-ok-allow"),
                    ActionKind::Primary,
                ),
                (List::Banned, true) => (
                    aurora_i18n::tr!("region-people-ask-add-banned-title"),
                    aurora_i18n::tr!("region-people-ask-add-banned", names = who),
                    aurora_i18n::tr!("region-people-ban"),
                    ActionKind::Danger,
                ),
                (_, false) => (
                    match list {
                        List::Managers => aurora_i18n::tr!("region-people-ask-remove-managers-title"),
                        List::Allowed => aurora_i18n::tr!("region-people-ask-remove-allowed-title"),
                        List::Groups => aurora_i18n::tr!("region-people-ask-remove-groups-title"),
                        List::Banned => aurora_i18n::tr!("region-people-ask-remove-banned-title"),
                    },
                    match list {
                        List::Managers => aurora_i18n::tr!("region-people-ask-remove-managers", names = who),
                        List::Allowed => aurora_i18n::tr!("region-people-ask-remove-allowed", names = who),
                        List::Groups => aurora_i18n::tr!("region-people-ask-remove-groups", names = who),
                        List::Banned => aurora_i18n::tr!("region-people-ask-remove-banned", names = who),
                    },
                    if *list == List::Banned {
                        aurora_i18n::tr!("region-people-ok-unban")
                    } else {
                        aurora_i18n::tr!("ui-land-people-remove")
                    },
                    ActionKind::Primary,
                ),
            };
            let icon = if *list == List::Banned && *add { "ban" } else { list.icon() };
            (icon, *list == List::Banned && *add, title.to_owned(), text, Some((ok, kind)))
        }
    };
    // the scope's choices (all_estates_text)
    let all_label = match v.role {
        Role::Owner => aurora_i18n::tr!("region-people-all-owned").to_owned(),
        _ => aurora_i18n::tr!("region-people-all-managed", owner = names(world, &v.owner).0),
    };
    let mut answer: Option<bool> = None;
    let modal = widgets::Modal::new(egui::Id::unique("region_estate_people_ask")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let tint = if danger { p.danger } else { p.violet_light };
            widgets::glyph(ui, icon, tint, 16.0);
            ui.label(semibold(ui, &title, 13.5, p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, r) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                sheet::icon_at(ui, "x", rect.center(), 15.0, if r.hovered() { p.ink } else { p.muted });
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    answer = Some(false);
                }
            });
        });
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(&text).size(12.5).color(p.ink)).wrap());
        if let PeopleAsk::Scope { all, .. } = &mut ask {
            ui.add_space(10.0);
            ui.label(semibold(ui, aurora_i18n::tr!("region-people-applies-to"), 11.5, p.muted));
            ui.add_space(2.0);
            widgets::radio_value(ui, p, all, false, aurora_i18n::tr!("region-people-this-estate"));
            widgets::radio_value(ui, p, all, true, all_label.as_str());
        }
        ui.add_space(14.0);
        let mut buttons = Vec::new();
        if main.is_some() {
            buttons.push(DialogButton {
                icon: "",
                label: aurora_i18n::tr!("common-cancel"),
                kind: ActionKind::Flat,
                enabled: true,
            });
        }
        let (ok, kind) = main.unwrap_or((aurora_i18n::tr!("common-ok"), ActionKind::Primary));
        buttons.push(DialogButton {
            icon: "",
            label: ok,
            kind,
            enabled: true,
        });
        if let Some(i) = widgets::dialog_buttons(ui, p, &buttons, 28.0) {
            answer = Some(i + 1 == buttons.len());
        }
    });
    let notice = matches!(ask, PeopleAsk::Notice { .. });
    if modal.should_close() && answer.is_none() {
        answer = Some(notice);
    }
    let Some(ok) = answer else {
        s.people.ask = Some(ask);
        return;
    };
    match ask {
        PeopleAsk::Notice { then, .. } => s.people.ask = then.map(|b| *b),
        PeopleAsk::Linden { list, remove } if ok => match remove {
            // a Linden estate's change applies to it alone
            Some(ids) => {
                send(world, v, list, false, ids, false);
                s.people.selected[list.index()].clear();
            }
            None => open_picker(&mut s.people, list),
        },
        PeopleAsk::Scope { list, add, ids, all } if ok => {
            send(world, v, list, add, ids, all);
            if !add {
                s.people.selected[list.index()].clear();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// updateControls: the owner every list, a manager all but the
    /// managers', no ban on a Linden estate, a resident nothing.
    #[test]
    fn who_changes_which_list() {
        for list in [List::Managers, List::Allowed, List::Groups, List::Banned] {
            assert!(can_change(list, Role::Owner, false));
            assert!(!can_change(list, Role::Resident, false));
        }
        assert!(!can_change(List::Managers, Role::Manager, false));
        assert!(can_change(List::Banned, Role::Manager, false));
        assert!(!can_change(List::Banned, Role::Owner, true));
        assert!(can_change(List::Allowed, Role::Owner, true));
    }

    #[test]
    fn all_estates_is_the_owner_s_or_the_managed_ones() {
        assert_eq!(scope_flags(Role::Owner, true), ea::APPLY_TO_ALL_ESTATES);
        assert_eq!(scope_flags(Role::Manager, true), ea::APPLY_TO_MANAGED_ESTATES);
        assert_eq!(scope_flags(Role::Owner, false), 0);
        assert_eq!(scope_flags(Role::Resident, true), 0);
    }

    /// accessAddCore3: the batch's limit, who is already there, managers
    /// and the owner kept out of the bans.
    #[test]
    fn what_of_a_pick_is_added() {
        let current = [id(1), id(2)];
        let managers = [id(3)];
        let p = pick(List::Allowed, &[id(2), id(4), id(4)], &current, &managers, id(9)).expect("fits");
        assert_eq!(p.ids, [id(4)]);
        assert_eq!(p.already, [id(2)]);
        let p = pick(List::Banned, &[id(3), id(9), id(5)], &current, &managers, id(9)).expect("fits");
        assert_eq!(p.ids, [id(5)]);
        assert_eq!(p.managers, [id(3)]);
        assert!(p.owner);
        // a manager may be allowed: only the bans keep them out
        assert_eq!(
            pick(List::Allowed, &[id(3)], &current, &managers, id(9)).map(|p| p.ids),
            Ok(vec![id(3)])
        );
        let many: Vec<Uuid> = (10..760).map(id).collect();
        assert_eq!(pick(List::Banned, &many, &current, &managers, id(9)), Err(2));
        assert!(pick(List::Allowed, &many[..498], &current, &managers, id(9)).is_ok());
    }

    #[test]
    fn the_export_has_the_bans_columns() {
        let rows = [(
            id(7),
            "pixelboutique".to_owned(),
            ["2026-10-13 18:02".into(), "2026-10-09 10:00".into(), "Tess".into()],
        )];
        assert_eq!(
            export_csv(&rows, true),
            "Name,UUID,Last Login,Ban Date,Banned By\npixelboutique,00000000-0000-0000-0000-000000000007,2026-10-13 18:02,2026-10-09 10:00,Tess\n"
        );
        assert_eq!(
            export_csv(&rows, false),
            "Name,UUID\npixelboutique,00000000-0000-0000-0000-000000000007\n"
        );
    }

    #[test]
    fn a_ban_date_reads_as_a_day() {
        assert_eq!(
            day_of("2026-10-09 10:00"),
            aurora_i18n::tr!(
                "ui-land-overview-date",
                day = "9".to_string(),
                month = crate::ui::land::months()[9].to_owned(),
                year = "2026".to_string()
            )
        );
        assert_eq!(day_of("n/a"), "n/a");
    }

    #[test]
    fn many_names_end_in_a_count() {
        let names: Vec<String> = (1..=7).map(|i| format!("R{i}")).collect();
        assert_eq!(listed(&names[..2]), "R1, R2");
        assert_eq!(
            listed(&names),
            aurora_i18n::tr!("region-people-and-more", names = "R1, R2, R3, R4, R5".to_string(), count = 2_i64)
        );
    }
}
