//! Aurora's About Land (the default; Preferences › Aurora gives Firestorm's
//! nine tabs back, `Settings::classic_about_land`): the parcel at a glance
//! in a header shared by three pages, "Overview" for everyone
//! (`overview.rs`), "Manage" for the settings and "People and objects" for
//! the lists (`manage.rs`, `people.rs`). Nothing of Firestorm's floater is
//! left out and the protocol is the same. Who may manage the parcel can look
//! at it as a visitor would ("View as").

use super::manage::Section;
use super::{LandUi, Tab, View, manage, overview, people};
use crate::theme::Palette;
use crate::ui::sheet::{self, GAUGE_W, PICTURE, PageTab, fact, icon_at};
use crate::ui::{presence, widgets};
use crate::world::World;
use crate::world::land::{PrimRoom, PrimUsage};
use aurora_net::ParcelInfo;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Rect, RichText, Sense, Shape, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// The pieces the pages share with the other redesigned windows.
pub(super) use crate::ui::sheet::{Tone, card_title, cards, cover_uv, kv, pill, semibold};

/// The three pages of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Page {
    #[default]
    Overview,
    Manage,
    People,
}

impl Page {
    pub(super) const ALL: [Page; 3] = [Page::Overview, Page::Manage, Page::People];

    /// Name of the page for AURORA_DEMO_LAND, whatever the language.
    pub(super) fn key(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Manage => "manage",
            Page::People => "people",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Page::Overview => aurora_i18n::tr!("ui-land-overview-tab"),
            Page::Manage => aurora_i18n::tr!("ui-land-overview-manage-tab"),
            Page::People => aurora_i18n::tr!("ui-land-overview-people-tab"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Overview => "eye-filled",
            Page::Manage => "adjustments-horizontal-filled",
            Page::People => "users",
        }
    }

    /// Where a tab of the classic window went: its page, and on "Manage"
    /// its section (the covenant is a card of the overview).
    pub(super) fn of(tab: Tab) -> (Page, Option<Section>) {
        match tab {
            Tab::Covenant => (Page::Overview, None),
            Tab::General => (Page::Manage, Some(Section::Sale)),
            Tab::Objects => (Page::Manage, Some(Section::Objects)),
            Tab::Options => (Page::Manage, Some(Section::Rules)),
            Tab::Media | Tab::Sound => (Page::Manage, Some(Section::Media)),
            Tab::Access => (Page::Manage, Some(Section::Access)),
            Tab::Experiences => (Page::Manage, Some(Section::Experiences)),
            Tab::Environment => (Page::Manage, Some(Section::Environment)),
        }
    }
}

/// Default size: the mockup's 1000 px, tall enough for the overview.
pub(super) const SIZE: Vec2 = Vec2::new(1000.0, 720.0);
pub(super) const MIN_SIZE: Vec2 = Vec2::new(760.0, 480.0);

/// The name of the agent's rights on the parcel for "View as": None when
/// they can change nothing (they already see what a visitor sees).
pub(super) fn manager_role(v: &View) -> Option<&'static str> {
    let parcel = v.parcel.as_ref()?;
    manage::lock_banner(v, false, "").is_none().then(|| {
        if parcel.owner_id == v.agent {
            aurora_i18n::tr!("common-owner")
        } else {
            aurora_i18n::tr!("ui-land-overview-officer")
        }
    })
}

/// The same parcel seen by someone without any right on it.
pub(super) fn as_visitor(v: &View) -> View {
    View {
        agent: Uuid::max(),
        groups: Vec::new(),
        // nor the estate's rights (sale, release of the estate's land)
        region: v.region.clone().map(|r| super::RegionView {
            estate_manager: false,
            ..r
        }),
        ..v.clone()
    }
}

/// What the window draws with, besides the parcel.
pub(super) struct Pictures<'a> {
    /// Textures by asset (the parcel snapshot).
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    /// Profile pictures by resident.
    pub avatars: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// The parcel's snapshot covering its frame, darkened toward the bottom,
/// with the camera of LLPanelLandOptions' snapshot for those who may change
/// it.
fn picture(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, parcel: &ParcelInfo, pics: &Pictures) {
    let (rect, _) = ui.allocate_exact_size(PICTURE, Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    let tex = (!parcel.snapshot_id.is_nil())
        .then(|| {
            s.wanted_images.insert(parcel.snapshot_id);
            pics.images.get(&parcel.snapshot_id)
        })
        .flatten();
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                .uv(cover_uv(w, h, rect.size()))
                .tint(widgets::PICTURE_TINT)
                .corner_radius(radius)
                .paint_at(ui, rect);
            // the lower part darkens, for the camera to read on any picture
            let top = rect.bottom() - rect.height() * 0.38;
            let clear = p.navy.gamma_multiply(0.0);
            let dark = p.navy.gamma_multiply(0.55);
            let vertex = |x: f32, y: f32, color: Color32| Vertex {
                pos: egui::pos2(x, y),
                uv: WHITE_UV,
                color,
            };
            let mut m = Mesh::default();
            m.vertices.extend([
                vertex(rect.left(), top, clear),
                vertex(rect.right(), top, clear),
                vertex(rect.right(), rect.bottom(), dark),
                vertex(rect.left(), rect.bottom(), dark),
            ]);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            ui.painter().add(Shape::mesh(m));
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.bar);
            icon_at(ui, "mountain-filled", rect.center(), 48.0, p.muted_dim);
        }
    }
    if overview::can_change_picture(v) {
        let slot = Rect::from_min_size(rect.right_bottom() - Vec2::splat(8.0 + 28.0), Vec2::splat(28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(slot));
        let r = widgets::action_button(&mut child, p, "camera-filled", "", widgets::ActionKind::Flat, Vec2::splat(28.0));
        if r.on_hover_text(aurora_i18n::tr!("ui-land-overview-change-picture")).clicked() {
            s.texture_snapshot = true;
            s.texture.open(aurora_i18n::tr!("ui-land-options-land-picture"), parcel.snapshot_id);
        }
    }
}

/// The owner: their round picture with their status, their name, and the
/// parcel's group; a group that owns the land shows its name only.
fn owner(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, world: &mut World, parcel: &ParcelInfo, pics: &Pictures) {
    const SIDE: f32 = 30.0;
    let groups = world.groups.groups.clone();
    let group_name = (!parcel.group_id.is_nil())
        .then(|| world.land.group_name(&parcel.group_id, &groups))
        .flatten();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(SIDE), Sense::hover());
        let (name, person) = if parcel.owner_id.is_nil() {
            (aurora_i18n::tr!("ui-land-overview-public-land").to_owned(), None)
        } else if parcel.is_group_owned {
            let group = if parcel.group_id.is_nil() {
                parcel.owner_id
            } else {
                parcel.group_id
            };
            (world.land.group_name(&group, &groups).unwrap_or_else(|| "…".to_owned()), None)
        } else {
            world.social.want_name(parcel.owner_id);
            (world.social.name_of(&parcel.owner_id), Some(parcel.owner_id))
        };
        match person {
            Some(id) => {
                let pic = pics.avatars.get(&id);
                if pic.is_none() {
                    s.wanted_pics.insert(id);
                }
                widgets::avatar_disc(ui, p, rect, pic, &name, false);
                // the agent knows they are online
                let status = if id == world.agent_id {
                    presence::Status::Online
                } else {
                    presence::of(world, &id)
                };
                if status != presence::Status::Unknown {
                    presence::paint_dot(ui.painter(), p, rect, status, p.field);
                }
            }
            None if parcel.owner_id.is_nil() => icon_at(ui, "world-filled", rect.center(), 22.0, p.muted),
            // a group: the square of the group lists, not a disc
            None => widgets::avatar_picture(ui, p, rect, None, &name, false),
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let r = ui
                .add(egui::Label::new(semibold(ui, &name, 13.0, p.ink)).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                if let Some(id) = person {
                    crate::ui::profile::request_open(ui.ctx(), id);
                } else if parcel.is_group_owned {
                    let group = if parcel.group_id.is_nil() {
                        parcel.owner_id
                    } else {
                        parcel.group_id
                    };
                    crate::ui::group_profile::request_open(ui.ctx(), group);
                }
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let role = if parcel.owner_id.is_nil() {
                    ""
                } else if parcel.is_group_owned {
                    aurora_i18n::tr!("ui-land-overview-group-owner")
                } else {
                    aurora_i18n::tr!("common-owner")
                };
                ui.label(RichText::new(role).size(12.0).color(p.muted));
                if let Some(group) = group_name.filter(|_| !parcel.is_group_owned) {
                    ui.label(
                        RichText::new(format!(" · {} ", aurora_i18n::tr!("ui-land-overview-group-word")))
                            .size(12.0)
                            .color(p.muted),
                    );
                    if ui
                        .add(egui::Label::new(semibold(ui, &group, 12.0, p.ink)).sense(Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("ui-group-profile-view"))
                        .clicked()
                    {
                        crate::ui::group_profile::request_open(ui.ctx(), parcel.group_id);
                    }
                }
                if parcel.status == aurora_net::land::OS_LEASE_PENDING {
                    ui.label(
                        RichText::new(format!(" · {}", aurora_i18n::tr!("ui-land-general-sale-pending")))
                            .size(12.0)
                            .color(p.warn),
                    );
                }
            });
        });
    });
}

/// The parcel at a glance, over the three pages.
fn header(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, pics: &Pictures) {
    sheet::hero(ui, p, |ui| {
        picture(ui, p, s, v, parcel, pics);
        sheet::hero_column(ui, |ui| main_column(ui, p, s, v, world, parcel, pics));
    });
}

fn main_column(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, pics: &Pictures) {
    // name, rating and category
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(egui::Label::new(semibold(ui, &parcel.name, 19.0, p.ink)).truncate());
        if let Some(r) = v.region.as_ref() {
            widgets::maturity_badge_of(ui, p, r.sim_access, 15.0);
        }
        if let Some(category) = super::options::category_name(parcel.category) {
            egui::Frame::new()
                .fill(p.teal.gamma_multiply(0.14))
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::symmetric(6, 1))
                .show(ui, |ui| {
                    ui.label(semibold(ui, category, 10.5, p.teal));
                });
        }
    });
    // where it is, its kind and area
    let ground = |x: f32, y: f32| world.regions.get(&v.handle).map_or(0.0, |r| r.heightmap.sample(x, y));
    let point = overview::place_point(parcel, ground);
    let (link, rest) = overview::region_line(v, parcel, point);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        widgets::glyph(ui, "map-pin-filled", p.muted, 13.0);
        let region = v.region.as_ref().map_or("", |r| r.name.as_str());
        let r = ui
            .add(egui::Label::new(RichText::new(&link).size(12.0).color(p.teal)).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(aurora_i18n::tr!("common-copy-slurl"));
        if r.clicked() && !region.is_empty() {
            ui.ctx().copy_text(crate::slurl::make(region, point));
        }
        ui.label(RichText::new(format!("· {rest}")).size(12.0).color(p.muted));
    });
    owner(ui, p, s, world, parcel, pics);
    // objects, access, arrival
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let usage = PrimUsage::of(parcel, v.region.as_ref().map_or(0, |r| r.max_tasks));
        if usage.max > 0 || usage.used > 0 {
            fact(ui, p, aurora_i18n::tr!("common-objects"), |ui| {
                let (main, rest) = overview::objects_fact(usage.used, usage.max);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(semibold(ui, &main, 14.0, p.ink));
                    ui.label(RichText::new(format!(" · {rest}")).size(12.0).color(p.muted));
                });
                let tint = match usage.room() {
                    PrimRoom::Plenty => None,
                    PrimRoom::Low => Some(p.warn),
                    PrimRoom::Critical => Some(p.danger),
                };
                sheet::gauge(ui, p, "land_objects", usage.fraction(), GAUGE_W, tint);
            });
        }
        fact(ui, p, aurora_i18n::tr!("ui-land-overview-access"), |ui| {
            let (tone, label) = overview::access_fact(parcel);
            pill(ui, p, tone, "door-enter", label);
        });
        fact(ui, p, aurora_i18n::tr!("ui-land-overview-arrival"), |ui| {
            ui.label(semibold(ui, &overview::arrival(parcel), 12.5, p.ink));
        });
    });
    // what visitors may do
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let region_flags = v.region.as_ref().map_or(0, |r| r.flags);
        for permit in overview::permits(parcel, region_flags) {
            pill(ui, p, permit.tone, permit.icon, permit.label).on_hover_text(permit.tip);
        }
    });
}

/// The window's body. True when the saved music streams changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    pics: &Pictures,
    streams: &mut Vec<String>,
    has_xp: bool,
) -> bool {
    let mut streams_changed = false;
    let role = manager_role(v);
    if role.is_none() {
        s.view_as_visitor = false;
    }
    let tabs = Page::ALL.map(|page| PageTab {
        page,
        icon: page.icon(),
        label: page.label(),
        disabled: None,
    });
    let role = role.map(|mine| sheet::ViewAs {
        mine,
        other: aurora_i18n::tr!("ui-land-overview-visitor"),
        tip: aurora_i18n::tr!("ui-land-overview-view-as-tip"),
        accent: false,
    });
    sheet::page_tabs(ui, p, &tabs, &mut s.page, role, &mut s.view_as_visitor);
    ui.add_space(8.0);
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return false;
    };
    let preview = s.view_as_visitor;
    let visitor;
    let v = if preview {
        visitor = as_visitor(v);
        &visitor
    } else {
        v
    };
    match s.page {
        Page::Overview => {
            egui::ScrollArea::vertical()
                .id_salt("about_land_overview")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 6.0);
                    header(ui, p, s, v, world, &parcel, pics);
                    overview::show(ui, p, s, v, world);
                    ui.add_space(4.0);
                });
        }
        Page::Manage => {
            header(ui, p, s, v, world, &parcel, pics);
            ui.add_space(12.0);
            streams_changed = manage::show(ui, p, s, v, world, pics.images, streams, preview, has_xp);
        }
        Page::People => {
            header(ui, p, s, v, world, &parcel, pics);
            ui.add_space(12.0);
            people::show(ui, p, s, v, world, pics);
        }
    }
    streams_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_classic_tab_has_a_place() {
        for tab in Tab::ALL {
            let (page, section) = Page::of(tab);
            if tab == Tab::Covenant {
                assert_eq!(page, Page::Overview, "the covenant is a card of the overview");
            } else {
                assert_eq!(page, Page::Manage, "{tab:?}");
                assert!(section.is_some(), "{tab:?} has its section");
            }
        }
        assert_eq!(Page::of(Tab::Options).1, Some(Section::Rules));
        assert_eq!(Page::of(Tab::Sound).1, Some(Section::Media));
    }

    /// "View as" a visitor: the owner and an officer lose every right, and
    /// only they are offered it.
    #[test]
    fn view_as_a_visitor_takes_every_right() {
        use crate::world::land::powers;
        let (me, group) = (Uuid::from_u128(1), Uuid::from_u128(9));
        let parcel = |owner: Uuid| ParcelInfo {
            owner_id: owner,
            group_id: group,
            is_group_owned: owner == group,
            status: aurora_net::land::OS_LEASED,
            ..Default::default()
        };
        let view = |parcel: ParcelInfo, powers: u64| View {
            parcel: Some(std::sync::Arc::new(parcel)),
            handle: 0,
            region: None,
            agent: me,
            groups: vec![aurora_net::GroupMembership {
                id: group,
                name: String::new(),
                insignia: Uuid::nil(),
                powers,
                accept_notices: true,
                list_in_profile: true,
                contribution: 0,
            }],
            same_region: true,
            now: 0,
        };
        let owner = view(parcel(me), 0);
        assert_eq!(manager_role(&owner), Some(aurora_i18n::tr!("common-owner")));
        assert!(!as_visitor(&owner).can(powers::LAND_OPTIONS));
        let officer = view(parcel(group), powers::LAND_OPTIONS);
        assert_eq!(manager_role(&officer), Some(aurora_i18n::tr!("ui-land-overview-officer")));
        assert!(!as_visitor(&officer).can(powers::LAND_OPTIONS));
        assert_eq!(manager_role(&view(parcel(group), 0)), None, "a member without powers");
        assert_eq!(manager_role(&view(parcel(Uuid::from_u128(5)), 0)), None, "a visitor");
    }
}
