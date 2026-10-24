//! The `secondlife:///app/...` commands of a text (`slapp.rs`): the label
//! each one shows, its tooltip and its right-click menu (LLUrlEntryGroup,
//! LLUrlEntryParcel, LLUrlEntryObjectIM, LLUrlEntryInventory,
//! LLUrlEntryExperienceProfile, LLUrlEntryWorldMap, LLUrlEntryChat,
//! FSUrlEntryWear, LLUrlEntrySL; menu_url_group.xml, menu_url_parcel.xml,
//! menu_url_objectim.xml, menu_url_inventory.xml, menu_url_experience.xml,
//! menu_url_map.xml, menu_url_slapp.xml; indra/llui/llurlentry.cpp,
//! originally LGPL 2.1). A click asks the app to run the command
//! (`CtxAction::AppLink`).

use super::super::context::{CtxAction, request};
use super::super::menu;
use crate::slapp::{AgentVerb, AppLink, Blocked, unescape};
use crate::theme::Palette;
use crate::world::World;
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;
use uuid::Uuid;

/// Names a label is waiting for, asked by the app at the end of the frame
/// (the texts only see the world, read-only).
#[derive(Debug, Default)]
pub struct Wanted {
    pub groups: BTreeSet<Uuid>,
    pub experiences: BTreeSet<Uuid>,
    pub parcels: BTreeSet<Uuid>,
    /// Avatars named by a text drawn without the chat's set of wanted names
    /// (a notification's or a script dialog's, #898).
    pub agents: BTreeSet<Uuid>,
}

static WANTED: Mutex<Wanted> = Mutex::new(Wanted {
    groups: BTreeSet::new(),
    experiences: BTreeSet::new(),
    parcels: BTreeSet::new(),
    agents: BTreeSet::new(),
});

fn want(f: impl FnOnce(&mut Wanted)) {
    if let Ok(mut w) = WANTED.lock() {
        f(&mut w);
    }
}

/// Ask the names of these avatars at the end of the frame.
pub(crate) fn want_agents(ids: impl IntoIterator<Item = Uuid>) {
    want(|w| w.agents.extend(ids));
}

/// The lookups asked since the last call.
pub fn take_wanted() -> Wanted {
    WANTED.lock().map(|mut w| std::mem::take(&mut *w)).unwrap_or_default()
}

/// A group's name: ours, then the name cache, else asked (LLUrlEntryGroup:
/// "(none)" for the null group).
pub(crate) fn group_name(world: &World, id: Uuid) -> String {
    if id.is_nil() {
        return aurora_i18n::tr!("status-none").to_owned();
    }
    world
        .groups
        .groups
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.name.clone())
        .or_else(|| world.land.group_names.get(&id).cloned())
        .unwrap_or_else(|| {
            want(|w| {
                w.groups.insert(id);
            });
            "…".to_owned()
        })
}

/// What a command shows in the text (getLabel of its LLUrlEntry; the
/// generic LLUrlEntrySL shows the address, unescaped).
pub(super) fn label(world: &World, want_names: &mut HashSet<Uuid>, url: &str, link: &AppLink) -> String {
    let label = match link {
        AppLink::Agent { id, .. } => {
            want_names.insert(*id);
            world.social.name_of(id)
        }
        AppLink::Group(id) => group_name(world, *id),
        // the address until ParcelInfoReply, then the parcel's name or
        // "Region (x, y, z)" (LLUrlEntryParcel::processParcelInfo)
        AppLink::Parcel(id) => match world.profiles.parcels.get(id) {
            Some(d) if !d.name.is_empty() => d.name.clone(),
            Some(d) if !d.sim_name.is_empty() => format!(
                "{} ({}, {}, {})",
                d.sim_name,
                (d.global.x.round() as i64).rem_euclid(256),
                (d.global.y.round() as i64).rem_euclid(256),
                d.global.z.round() as i64
            ),
            Some(_) => aurora_i18n::tr!("ui-chat-applink-parcel-error").to_owned(),
            None => {
                want(|w| {
                    w.parcels.insert(*id);
                });
                unescape(url)
            }
        },
        AppLink::ObjectIm(o) if !o.name.is_empty() => o.name.clone(),
        AppLink::Inventory { name: Some(n), .. } => n.clone(),
        AppLink::Experience(id) if id.is_nil() => aurora_i18n::tr!("ui-chat-applink-experience-none").to_owned(),
        AppLink::Experience(id) => {
            let name = world
                .land
                .experience_names
                .get(id)
                .cloned()
                .or_else(|| world.experiences.details.get(id).map(|d| d.name.clone()));
            match name {
                Some(n) if n.trim().is_empty() => aurora_i18n::tr!("ui-chat-applink-experience-untitled").to_owned(),
                Some(n) => n,
                None => {
                    want(|w| {
                        w.experiences.insert(*id);
                    });
                    "…".to_owned()
                }
            }
        }
        // "Show Map for Region (x,y,z)"
        AppLink::WorldMap { region, pos } => aurora_i18n::tr!(
            "ui-chat-applink-show-map-for",
            place = format!("{region} ({},{},{})", pos.x as i32, pos.y as i32, pos.z as i32)
        ),
        AppLink::Blocked(Blocked::WearFolder) => aurora_i18n::tr!("ui-chat-applink-wear-label").to_owned(),
        // Human-requested fallback: an unsupported URI has no destination
        // name to resolve. Keep the actual address in the copy-link menu.
        AppLink::Unsupported => aurora_i18n::tr!("ui-chat-applink-unsupported-label").to_owned(),
        _ => unescape(url),
    };
    world.rlva_filter_text(&label).into_owned()
}

/// Describe the actual click action, as LLUrlEntryAgent::getTooltip does
/// for its verbs. Native links do not need a protocol address in the hint.
pub(super) fn tooltip(link: &AppLink) -> &'static str {
    match link {
        AppLink::Agent { verb, .. } => match verb {
            AgentVerb::Im => aurora_i18n::tr!("ui-chat-applink-im-tip"),
            AgentVerb::Pay => aurora_i18n::tr!("ui-chat-applink-pay-tip"),
            AgentVerb::OfferTeleport => aurora_i18n::tr!("ui-chat-applink-offer-teleport-tip"),
            AgentVerb::RequestFriend => aurora_i18n::tr!("ui-chat-applink-request-friend-tip"),
            AgentVerb::RemoveFriend => aurora_i18n::tr!("ui-chat-applink-remove-friend-tip"),
            AgentVerb::Mute => aurora_i18n::tr!("ui-chat-applink-block-resident-tip"),
            AgentVerb::Unmute => aurora_i18n::tr!("ui-chat-applink-unblock-resident-tip"),
            AgentVerb::Block(_) => aurora_i18n::tr!("ui-chat-applink-block-object-tip"),
            AgentVerb::Unblock(_) => aurora_i18n::tr!("ui-chat-applink-unblock-object-tip"),
            AgentVerb::ReportAbuse => aurora_i18n::tr!("ui-chat-applink-report-tip"),
        },
        AppLink::Group(_) => aurora_i18n::tr!("ui-chat-applink-group-tip"),
        AppLink::Parcel(_) => aurora_i18n::tr!("ui-chat-applink-parcel-tip"),
        AppLink::ObjectIm(_) => aurora_i18n::tr!("ui-chat-applink-object-tip"),
        AppLink::Inventory { .. } => aurora_i18n::tr!("ui-chat-applink-inventory-select-tip"),
        AppLink::InventoryShow => aurora_i18n::tr!("ui-chat-applink-inventory-show-tip"),
        AppLink::Experience(_) => aurora_i18n::tr!("ui-chat-applink-experience-tip"),
        AppLink::Classified(_) => aurora_i18n::tr!("ui-chat-applink-classified-tip"),
        AppLink::Event(_) => aurora_i18n::tr!("ui-chat-applink-event-tip"),
        AppLink::WorldMap { .. } => aurora_i18n::tr!("ui-chat-applink-map-tip"),
        AppLink::Chat { .. } => aurora_i18n::tr!("ui-chat-applink-chat-tip"),
        AppLink::Search { .. } => aurora_i18n::tr!("ui-chat-applink-search-tip"),
        AppLink::Blocked(_) => aurora_i18n::tr!("ui-chat-applink-blocked-tip"),
        AppLink::Unsupported => aurora_i18n::tr!("ui-chat-applink-unsupported-tip"),
    }
}

/// "Region/x/y/z" of an object link as a place.
fn object_place(slurl: &str) -> Option<crate::slurl::PlaceLink> {
    (!slurl.is_empty())
        .then(|| crate::slurl::place_link(&format!("secondlife:///app/teleport/{slurl}")))
        .flatten()
}

/// The right-click menu of a command, in the order of its menu_url_*.xml.
pub(super) fn menu(r: &egui::Response, p: &Palette, world: &World, url: &str, link: &AppLink, label: &str) {
    if let AppLink::Agent { id, .. } = link {
        super::name_menu(r, p, world, *id);
        return;
    }
    menu::context_menu(r, p, |ui| {
        let ctx = ui.ctx().clone();
        let run = || request(&ctx, CtxAction::AppLink(link.clone()));
        let copy_label = |ui: &mut egui::Ui, text: &'static str| {
            if menu::item(ui, p, "copy-filled", text) {
                ui.ctx().copy_text(label.to_owned());
            }
        };
        match link {
            AppLink::Group(_) => {
                if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-chat-applink-show-group")) {
                    run();
                }
                copy_label(ui, aurora_i18n::tr!("ui-chat-applink-copy-group"));
            }
            AppLink::Parcel(id) => {
                if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-chat-applink-show-parcel")) {
                    run();
                }
                // Url.ShowParcelOnMap: once the parcel's position is known
                let at = world.profiles.parcels.get(id).filter(|d| !d.sim_name.is_empty());
                if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), at.is_some())
                    && let Some(d) = at
                {
                    let g = d.global;
                    let pos = glam::Vec3::new(g.x.rem_euclid(256.0) as f32, g.y.rem_euclid(256.0) as f32, g.z as f32);
                    request(&ctx, CtxAction::ShowPlace(d.sim_name.clone(), pos));
                }
            }
            AppLink::ObjectIm(o) => {
                if menu::item(ui, p, "zoom-scan-filled", aurora_i18n::tr!("ui-chat-applink-object-profile")) {
                    run();
                }
                let (icon, text) = if world.mutes.is_muted_id(&o.id) {
                    ("circle-check-filled", aurora_i18n::tr!("common-unblock"))
                } else {
                    ("ban", aurora_i18n::tr!("common-block"))
                };
                if menu::item(ui, p, icon, text) {
                    request(
                        &ctx,
                        CtxAction::ToggleBlockObject {
                            id: o.id,
                            name: o.name.clone(),
                        },
                    );
                }
                let place = object_place(&o.slurl);
                if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), place.is_some())
                    && let Some(l) = &place
                {
                    request(&ctx, CtxAction::ShowPlace(l.location.target(), l.location.pos));
                }
                if menu::item_if(
                    ui,
                    p,
                    "location-filled",
                    aurora_i18n::tr!("ui-chat-applink-teleport-object"),
                    place.is_some(),
                ) && let Some(l) = place
                {
                    request(&ctx, CtxAction::TeleportToPlace(l.location.target(), l.location.pos));
                }
                copy_label(ui, aurora_i18n::tr!("ui-chat-applink-copy-object-name"));
            }
            AppLink::Inventory { .. } => {
                if menu::item(ui, p, "briefcase-filled", aurora_i18n::tr!("ui-chat-applink-show-item")) {
                    run();
                }
                copy_label(ui, aurora_i18n::tr!("ui-chat-applink-copy-name"));
            }
            // menu_url_experience.xml: the SLurl only
            AppLink::Experience(_) => {}
            AppLink::WorldMap { region, pos } => {
                if menu::item(ui, p, "map", aurora_i18n::tr!("common-show-on-map")) {
                    run();
                }
                if menu::item(ui, p, "location-filled", aurora_i18n::tr!("ui-chat-teleport-to-this-location")) {
                    request(&ctx, CtxAction::TeleportToPlace(region.clone(), *pos));
                }
            }
            _ => {
                if menu::item(ui, p, "player-play-filled", aurora_i18n::tr!("ui-chat-applink-run")) {
                    run();
                }
            }
        }
        if !matches!(link, AppLink::Experience(_)) {
            menu::separator(ui, p);
        }
        if menu::item_if(
            ui,
            p,
            "link-filled",
            aurora_i18n::tr!("common-copy-slurl"),
            world.rlva_filter_text(url) == url,
        ) {
            ui.ctx().copy_text(url.to_owned());
        }
    });
    super::line_menu::inline(r);
}

/// A command of a wrapping text (conversation, profile…): its label in the
/// link colour, the command run on a click.
pub(super) fn link(ui: &mut egui::Ui, p: &Palette, world: &World, url: &str, link: &AppLink, text: &str, size: f32) {
    let filtered = world.rlva_filter_text(text);
    let text = filtered.as_ref();
    let k = super::super::colors::get();
    let color = if let AppLink::Agent { id, .. } = link {
        super::super::colors::resident_name(world, id, super::super::colors::text(k.chat_slurl))
    } else {
        super::super::colors::text(k.chat_slurl)
    };
    let r = super::action_marker::link(ui, super::action_marker::Action::app(link), text, size, color, None);
    menu(&r, p, world, url, link, text);
    if r.on_hover_text(tooltip(link)).clicked() {
        request(ui.ctx(), CtxAction::AppLink(link.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hovering_the_same_name_distinguishes_profile_im_and_pay_in_text_and_bubbles() {
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let id = crate::demo::DEMO_LOUP;
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        for bubble in [false, true] {
            for (verb, tip_key) in [
                ("about", "common-view-profile"),
                ("im", "ui-chat-applink-im-tip"),
                ("pay", "ui-chat-applink-pay-tip"),
            ] {
                let ctx = egui::Context::default();
                theme.apply(&ctx, 1.0);
                ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
                let url = format!("secondlife:///app/agent/{id}/{verb}");
                let target = if verb == "about" {
                    super::super::Link::Agent(id)
                } else {
                    super::super::Link::App(url.clone(), crate::slapp::parse(&url).expect("agent action"))
                };
                let mut pos = egui::pos2(80.0, 50.0);
                let mut texts = Vec::new();
                fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
                        _ => {}
                    }
                }
                for frame in 0..10 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                            time: Some(frame as f64 * 0.2),
                            events: if frame == 2 {
                                vec![egui::Event::PointerMoved(pos)]
                            } else {
                                Vec::new()
                            },
                            ..Default::default()
                        },
                        |ui| {
                            if bubble {
                                super::super::bubble_link(
                                    ui.ctx(),
                                    &p,
                                    &world,
                                    0,
                                    egui::Rect::from_center_size(pos, egui::vec2(100.0, 20.0)),
                                    &target,
                                    p.violet,
                                );
                            } else {
                                let shown = ui.scope(|ui| match &target {
                                    super::super::Link::Agent(id) => super::super::agent_link(ui, &p, &world, *id, "Resident", 14.0),
                                    super::super::Link::App(url, action) => link(ui, &p, &world, url, action, "Resident", 14.0),
                                    _ => unreachable!(),
                                });
                                pos = shown.response.rect.center();
                            }
                        },
                    );
                    output.textures_delta.clear();
                    texts.clear();
                    output.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
                }
                let expected = aurora_i18n::text(tip_key);
                assert!(
                    texts.iter().any(|text| text == expected),
                    "bubble={bubble}, verb={verb}, texts={texts:?}"
                );
                assert!(
                    !texts.iter().any(|text| text.contains("secondlife:")),
                    "a native action hint has no protocol/UUID: {texts:?}"
                );
            }
        }
    }

    #[test]
    fn labels_of_the_legacy_entries() {
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut names = HashSet::new();
        let label = |url: &str, names: &mut HashSet<Uuid>| {
            let link = crate::slapp::parse(url).expect("app link");
            label(&world, names, url, &link)
        };
        let obj = "secondlife:///app/objectim/0e346d8b-4433-4d66-a6b0-fd37083abc4c?name=Old%20Sign&slurl=A/1/2/3";
        assert_eq!(label(obj, &mut names), "Old Sign");
        let inv = "secondlife:///app/inventory/0e346d8b-4433-4d66-a6b0-fd37083abc4c/select";
        assert_eq!(label(inv, &mut names), inv);
        assert_eq!(
            label("secondlife:///app/event/12/details", &mut names),
            "secondlife:///app/event/12/details"
        );
        assert_eq!(
            label("secondlife:///app/chat/5/hi%20there", &mut names),
            "secondlife:///app/chat/5/hi there"
        );
        let group = "secondlife:///app/group/00000000-0000-0000-0000-000000000000/about";
        assert_eq!(label(group, &mut names), aurora_i18n::tr!("status-none"));
        // an unknown group waits for its name, asked once the frame ends
        let group = "secondlife:///app/group/3de548e1-57be-cfea-2b78-83ae3ad95998/about";
        assert_eq!(label(group, &mut names), "…");
        let id = Uuid::parse_str("3de548e1-57be-cfea-2b78-83ae3ad95998").expect("id");
        assert!(take_wanted().groups.contains(&id));
        // an agent verb shows the resident's name
        let im = format!("secondlife:///app/agent/{id}/im");
        label(&im, &mut names);
        assert!(names.contains(&id));
    }
}
