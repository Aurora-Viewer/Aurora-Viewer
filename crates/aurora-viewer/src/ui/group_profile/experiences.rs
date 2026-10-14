//! The Experiences tab (LLPanelGroupExperiences, indra/newview/
//! llpanelgroupexperiences.cpp, originally LGPL 2.1): the experiences the
//! group owns (LLExperienceCache::getGroupExperiences), each with its name,
//! rating and scope from the experience cache, and "Profile" opening the
//! experience profile (allow, forget, block). Members only, as Firestorm
//! shows the accordion (LLPanelGroup::update).

use super::view::View;
use super::{GroupAction, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::place_details::Ctx;
use crate::ui::widgets;
use aurora_net::experience::ExperienceDetails;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{FontId, Pos2, Rect, Shape, Vec2};
use uuid::Uuid;

/// Side of a row's picture.
const ICON: f32 = 38.0;

#[derive(Default)]
pub struct ExperiencesState {
    /// The group the tab was activated for; None when another tab shows
    /// (activate() asks the list again each time).
    pub active: Option<Uuid>,
}

/// The tab shows for a member.
pub fn visible(v: &View) -> bool {
    v.member.is_some()
}

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    if !visible(v) {
        return;
    }
    if st.experiences.active != Some(v.id) {
        st.experiences.active = Some(v.id);
        actions.push(GroupAction::LoadExperiences(v.id));
    }
    let p = c.p;
    egui::ScrollArea::vertical()
        .id_salt("group_profile_experiences")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width() - 8.0;
            ui.horizontal_top(|ui| {
                ui.add_space(4.0);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match c.world.experiences.groups.get(&v.id).and_then(|l| l.as_ref()) {
                        None => muted(ui, p, aurora_i18n::tr!("ui-group-experiences-loading")),
                        Some(list) if list.is_empty() => muted(ui, p, aurora_i18n::tr!("ui-group-experiences-none")),
                        Some(list) => {
                            for id in list {
                                if row(ui, c, *id, c.world.experiences.details.get(id)) {
                                    actions.push(GroupAction::ExperienceProfile(*id));
                                }
                            }
                        }
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(aurora_i18n::tr!("ui-group-experiences-help"))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                });
            });
        });
}

fn muted(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(12.5).color(p.muted));
}

/// The experience's picture: the accent from teal to indigo, a flask.
fn picture(ui: &egui::Ui, p: &Palette, rect: Rect) {
    let painter = ui.painter();
    // a rounded square of the theme's radius, then the gradient inside it
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.teal);
    let inner = rect.shrink(1.0);
    let v = |pos: Pos2, color| Vertex { pos, uv: WHITE_UV, color };
    let mid = p.teal.lerp_to_gamma(p.indigo, 0.5);
    let mut m = Mesh::default();
    m.vertices.extend([
        v(inner.left_top(), p.teal),
        v(inner.right_top(), mid),
        v(inner.right_bottom(), p.indigo),
        v(inner.left_bottom(), mid),
    ]);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(m));
    if let Some(t) = crate::ui::icons::global("flask-filled") {
        painter.image(
            t.id(),
            Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.on_accent,
        );
    }
}

/// One experience (LLExperienceItem): its picture, its name and rating,
/// "Owned by the group · Land", and "Profile". True when Profile was
/// clicked.
fn row(ui: &mut egui::Ui, c: &Ctx, id: Uuid, details: Option<&ExperienceDetails>) -> bool {
    let p = c.p;
    let mut open = false;
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ICON);
            let full = ui.max_rect();
            let icon = Rect::from_min_size(full.min, Vec2::splat(ICON));
            picture(ui, p, icon);
            // "Profile ›" at the right
            let button_area = Rect::from_min_max(Pos2::new(full.right() - 120.0, full.top()), full.max);
            let mut b = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(button_area)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if widgets::flat_button(&mut b, p, aurora_i18n::tr!("ui-group-experiences-profile")).clicked() {
                open = true;
            }
            let text_left = icon.right() + 10.0;
            let text_w = (b.min_rect().left() - 10.0 - text_left).max(40.0);
            let name = details
                .map(|d| d.name.trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| {
                    if details.is_some() {
                        id.to_string()
                    } else {
                        aurora_i18n::tr!("common-loading-2").to_owned()
                    }
                });
            let font = FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
            let name_w = ui
                .painter()
                .layout_no_wrap(name.clone(), font.clone(), p.ink)
                .size()
                .x
                .min(text_w - 22.0);
            widgets::paint_elided(ui, &name, Pos2::new(text_left, icon.top() + 11.0), text_w - 22.0, font, p.ink);
            // the rating after the name (the mockup's "mat" badge)
            if let Some(d) = details.filter(|d| !d.invalid() && d.maturity > 0) {
                let badge = Rect::from_min_size(Pos2::new(text_left + name_w + 6.0, icon.top() + 4.0), Vec2::splat(14.0));
                widgets::paint_maturity_badge(
                    ui.painter(),
                    p,
                    crate::ui::bars::maturity_name(d.maturity),
                    badge,
                    widgets::inner_radius(ui.visuals()),
                );
            }
            let scope = match details {
                Some(d) if d.invalid() => aurora_i18n::tr!("ui-experience-profile-unknown").to_owned(),
                Some(d) => {
                    let scope = if d.grid_wide() {
                        aurora_i18n::tr!("ui-group-experiences-grid")
                    } else {
                        aurora_i18n::tr!("ui-group-experiences-land")
                    };
                    format!("{} · {scope}", aurora_i18n::tr!("ui-group-experiences-owned"))
                }
                None => aurora_i18n::tr!("ui-group-experiences-owned").to_owned(),
            };
            widgets::paint_elided(
                ui,
                &scope,
                Pos2::new(text_left, icon.bottom() - 10.0),
                text_w,
                FontId::proportional(11.5),
                p.muted,
            );
        });
    open
}
