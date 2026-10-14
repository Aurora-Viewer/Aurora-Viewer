//! Aurora's About Land (the default; Preferences › Aurora gives Firestorm's
//! nine tabs back, `Settings::classic_about_land`): the parcel at a glance
//! in a header shared by three pages, "Overview" for everyone
//! (`overview.rs`), "Manage" for the settings and "People and objects" for
//! the lists. Nothing of Firestorm's floater is left out and the protocol
//! is the same: until the settings are regrouped by intent, "Manage" and
//! "People and objects" hold the panes of the nine tabs (`Tab`), the
//! covenant being a card of the overview.

use super::{LandUi, Tab, View, overview};
use crate::theme::Palette;
use crate::ui::{presence, widgets};
use crate::world::World;
use crate::world::land::{PrimRoom, PrimUsage};
use aurora_net::ParcelInfo;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, RichText, Sense, Shape, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

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

    /// The panes of the nine tabs a page holds for now, in their order.
    pub(super) fn panes(self) -> &'static [Tab] {
        match self {
            Page::Overview => &[],
            Page::Manage => &[
                Tab::General,
                Tab::Options,
                Tab::Media,
                Tab::Sound,
                Tab::Experiences,
                Tab::Environment,
            ],
            Page::People => &[Tab::Access, Tab::Objects],
        }
    }

    /// The page that shows a tab of the classic window (the covenant is
    /// a card of the overview).
    pub(super) fn of(tab: Tab) -> Page {
        Page::ALL
            .into_iter()
            .find(|page| page.panes().contains(&tab))
            .unwrap_or(Page::Overview)
    }
}

/// Default size: the mockup's 1000 px, tall enough for the overview.
pub(super) const SIZE: Vec2 = Vec2::new(1000.0, 720.0);
pub(super) const MIN_SIZE: Vec2 = Vec2::new(760.0, 480.0);

/// Side of the header's picture.
const PICTURE: Vec2 = Vec2::new(220.0, 148.0);
/// Width of the objects gauge.
const GAUGE_W: f32 = 220.0;
/// Width of the navigation of "Manage" and "People and objects".
const NAV_W: f32 = 200.0;

/// The tint of a pill: allowed, for the group only, refused, or a plain fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Allow,
    Group,
    Deny,
    Neutral,
}

/// Text in the theme's semi-bold face.
fn semibold(ui: &egui::Ui, text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let family = crate::ui::fonts::semibold(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, crate::ui::fonts::semibold_format(&family, size, color));
    job
}

/// A small Tabler icon `size` wide.
fn icon_at(ui: &egui::Ui, name: &str, center: Pos2, size: f32, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// A pill: an icon (or none) and a short text on a tinted face.
pub(super) fn pill(ui: &mut egui::Ui, p: &Palette, tone: Tone, icon: &str, label: &str) -> egui::Response {
    const PAD: f32 = 9.0;
    const ICON: f32 = 13.0;
    let (fill, ink) = match tone {
        Tone::Allow => (p.success.gamma_multiply(0.12), p.success),
        Tone::Group => (p.violet_light.gamma_multiply(0.16), p.violet_pale),
        Tone::Deny => (p.danger.gamma_multiply(0.12), p.danger),
        Tone::Neutral => (p.bar, p.muted),
    };
    let galley = ui.painter().layout_job(semibold(ui, label, 12.0, ink));
    let lead = if icon.is_empty() { 0.0 } else { ICON + 5.0 };
    let size = Vec2::new(PAD * 2.0 + lead + galley.size().x, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    if !icon.is_empty() {
        icon_at(ui, icon, egui::pos2(rect.left() + PAD + ICON / 2.0, rect.center().y), ICON, ink);
    }
    let pos = egui::pos2(rect.left() + PAD + lead, rect.center().y - galley.size().y / 2.0).round();
    ui.painter().galley(pos, galley, ink);
    resp
}

/// The title of a card: its icon, its name, and a link at the right end
/// (its response).
pub(super) fn card_title(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, link: Option<&str>) -> Option<egui::Response> {
    let out = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            widgets::glyph(ui, icon, p.violet_light, 15.0);
            ui.label(semibold(ui, title, 13.0, p.ink));
            link.map(|text| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(semibold(ui, text, 12.0, p.violet_light)).sense(Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                })
                .inner
            })
        })
        .inner;
    ui.add_space(2.0);
    out
}

/// Width of the keys of `kv`.
const KEY_W: f32 = 140.0;

/// A line of facts: its key in the left column, then `value`.
pub(super) fn kv(ui: &mut egui::Ui, p: &Palette, key: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.allocate_ui_with_layout(Vec2::new(KEY_W, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(KEY_W);
            ui.add(egui::Label::new(RichText::new(key).size(12.5).color(p.muted)).truncate());
        });
        ui.add_space(6.0);
        value(ui);
    });
}

/// Two cards side by side, 1.3 to 1 like the mockup's grid, as tall as the
/// taller one.
pub(super) fn cards(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    s: &mut LandUi,
    left: impl FnOnce(&mut egui::Ui, &mut World, &mut LandUi),
    right: impl FnOnce(&mut egui::Ui, &mut World, &mut LandUi),
) {
    const GAP: f32 = 12.0;
    const PAD: Vec2 = Vec2::new(12.0, 10.0);
    let width = ui.available_width();
    let left_w = ((width - GAP) * 1.3 / 2.3).floor();
    let right_w = width - GAP - left_w;
    let top = ui.cursor().min;
    let faces = [ui.painter().add(Shape::Noop), ui.painter().add(Shape::Noop)];
    let mut column = |x: f32, w: f32, body: &mut dyn FnMut(&mut egui::Ui)| {
        let inner = Rect::from_min_size(
            egui::pos2(x + PAD.x, top.y + PAD.y),
            Vec2::new((w - PAD.x * 2.0).max(1.0), 10_000.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // the lines of facts as close as the mockup's (12.5 px text, 6 px apart)
        child.spacing_mut().item_spacing.y = 6.0;
        child.spacing_mut().interact_size.y = 16.0;
        body(&mut child);
        child.min_rect().height()
    };
    let mut left = Some(left);
    let mut right = Some(right);
    let h_left = column(top.x, left_w, &mut |ui| {
        if let Some(f) = left.take() {
            f(ui, world, s)
        }
    });
    let h_right = column(top.x + left_w + GAP, right_w, &mut |ui| {
        if let Some(f) = right.take() {
            f(ui, world, s)
        }
    });
    let h = h_left.max(h_right) + PAD.y * 2.0;
    let radius = ui.visuals().window_corner_radius;
    ui.painter().set(
        faces[0],
        Shape::rect_filled(Rect::from_min_size(top, Vec2::new(left_w, h)), radius, p.field),
    );
    ui.painter().set(
        faces[1],
        Shape::rect_filled(
            Rect::from_min_size(egui::pos2(top.x + left_w + GAP, top.y), Vec2::new(right_w, h)),
            radius,
            p.field,
        ),
    );
    ui.allocate_rect(Rect::from_min_size(top, Vec2::new(width, h)), Sense::hover());
}

/// The page tabs: a lit face for the open one.
fn page_tabs(ui: &mut egui::Ui, p: &Palette, page: &mut Page) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for it in Page::ALL {
            let selected = *page == it;
            let ink = if selected { p.ink } else { p.muted };
            let galley = ui.painter().layout_job(semibold(ui, it.label(), 12.5, ink));
            let size = Vec2::new(12.0 * 2.0 + 14.0 + 6.0 + galley.size().x, 28.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            let face = if selected {
                Some(p.raised)
            } else if resp.hovered() {
                Some(p.raised.gamma_multiply(0.5))
            } else {
                None
            };
            if let Some(face) = face {
                ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
            }
            icon_at(ui, it.icon(), egui::pos2(rect.left() + 12.0 + 7.0, rect.center().y), 14.0, ink);
            let pos = egui::pos2(rect.left() + 12.0 + 14.0 + 6.0, rect.center().y - galley.size().y / 2.0).round();
            ui.painter().galley(pos, galley, ink);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, it.label()));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *page = it;
            }
        }
    });
}

/// The part of a `w` x `h` picture that covers a `size` frame, cut in its
/// middle.
pub(super) fn cover_uv(w: usize, h: usize, size: Vec2) -> Rect {
    let picture = w.max(1) as f32 / h.max(1) as f32;
    let frame = size.x / size.y.max(1.0);
    let half = if picture > frame {
        Vec2::new(frame / picture, 1.0) * 0.5
    } else {
        Vec2::new(1.0, picture / frame) * 0.5
    };
    Rect::from_min_max(egui::pos2(0.5, 0.5) - half, egui::pos2(0.5, 0.5) + half)
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

/// A fact of the header: its small key over its value.
fn fact(ui: &mut egui::Ui, p: &Palette, key: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(key).size(11.0).color(p.muted));
        value(ui);
    });
}

/// The parcel at a glance, over the three pages.
fn header(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, pics: &Pictures) {
    const PAD: f32 = 12.0;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let face = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_size(top + Vec2::splat(PAD), Vec2::new(width - PAD * 2.0, 10_000.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.x = 14.0;
    picture(&mut child, p, s, v, parcel, pics);
    child.vertical(|ui| {
        // rows as tall as their text, 8 px apart, like the mockup
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.spacing_mut().interact_size.y = 16.0;
        main_column(ui, p, s, v, world, parcel, pics);
    });
    let h = child.min_rect().height() + PAD * 2.0;
    ui.painter().set(
        face,
        Shape::rect_filled(
            Rect::from_min_size(top, Vec2::new(width, h)),
            ui.visuals().window_corner_radius,
            p.field,
        ),
    );
    ui.allocate_rect(Rect::from_min_size(top, Vec2::new(width, h)), Sense::hover());
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
                ui.add_space(3.0);
                let tint = match usage.room() {
                    PrimRoom::Plenty => None,
                    PrimRoom::Low => Some(p.warn),
                    PrimRoom::Critical => Some(p.danger),
                };
                widgets::progress(ui, p, "land_objects", usage.fraction(), GAUGE_W, "", tint);
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

/// The navigation of a page holding panes of the classic window.
fn pane_nav(ui: &mut egui::Ui, p: &Palette, tabs: &[Tab], selected: &mut Tab) {
    ui.vertical(|ui| {
        ui.set_width(NAV_W);
        ui.spacing_mut().item_spacing.y = 2.0;
        for tab in tabs {
            if widgets::navigation(ui, p, tab.icon(), tab.label(), *selected == *tab).clicked() {
                *selected = *tab;
            }
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
    page_tabs(ui, p, &mut s.page);
    ui.add_space(8.0);
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return false;
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
        page => {
            header(ui, p, s, v, world, &parcel, pics);
            ui.add_space(12.0);
            let tabs: Vec<Tab> = page.panes().iter().copied().filter(|t| *t != Tab::Experiences || has_xp).collect();
            let mut selected = if page == Page::Manage { s.manage_tab } else { s.people_tab };
            if !tabs.contains(&selected) {
                selected = tabs.first().copied().unwrap_or(Tab::General);
            }
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.allocate_ui_with_layout(Vec2::new(NAV_W, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    pane_nav(ui, p, &tabs, &mut selected);
                });
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    streams_changed = super::pane(ui, p, s, v, world, pics.images, streams, selected);
                });
            });
            if page == Page::Manage {
                s.manage_tab = selected;
            } else {
                s.people_tab = selected;
            }
        }
    }
    streams_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header's picture keeps its aspect: a square picture fills the
    /// width and loses its top and bottom.
    #[test]
    fn a_picture_covers_its_frame() {
        let frame = Vec2::new(220.0, 148.0);
        let full = cover_uv(220, 148, frame);
        assert!((full.width() - 1.0).abs() < 1e-5 && (full.height() - 1.0).abs() < 1e-5);
        let square = cover_uv(512, 512, frame);
        assert!((square.width() - 1.0).abs() < 1e-5);
        assert!((square.height() - 148.0 / 220.0).abs() < 1e-5);
        assert!((square.center().y - 0.5).abs() < 1e-5);
        let wide = cover_uv(1024, 256, frame);
        assert!((wide.height() - 1.0).abs() < 1e-5 && wide.width() < 1.0);
    }

    #[test]
    fn every_classic_tab_has_a_place() {
        for tab in Tab::ALL {
            let page = Page::of(tab);
            if tab == Tab::Covenant {
                assert_eq!(page, Page::Overview, "the covenant is a card of the overview");
            } else {
                assert!(page.panes().contains(&tab), "{tab:?} is on {page:?}");
            }
        }
        // the lists of people and of object owners are under "People and objects"
        assert_eq!(Page::of(Tab::Access), Page::People);
        assert_eq!(Page::of(Tab::Objects), Page::People);
        assert_eq!(Page::of(Tab::Options), Page::Manage);
    }
}
