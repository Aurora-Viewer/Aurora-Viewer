//! Portal creator and depth-occluded destination labels.
use super::{
    hud::Projector,
    widgets::{self, Floater},
};
use crate::{
    slurl,
    theme::Palette,
    world::{World, portals},
};
use aurora_net::bridge::portal::Access;
use glam::Vec3;
use uuid::Uuid;

#[derive(Default)]
pub struct PortalUi {
    pub open: bool,
    pub destination: String,
    pub access: Access,
    pub demo_seeded: bool,
    pub notice: Option<String>,
    #[cfg(test)]
    rects: [Option<egui::Rect>; 2],
}

#[derive(Default)]
pub struct Actions {
    pub create: bool,
    pub close: bool,
}

impl PortalUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, ready: bool) -> Actions {
        let mut actions = Actions::default();
        if !self.open {
            return actions;
        }
        Floater::new(
            "portals",
            aurora_i18n::tr!("portal-title"),
            egui::pos2(70.0, 150.0),
            egui::vec2(400.0, 350.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(aurora_i18n::tr!("portal-destination"));
            ui.add(
                egui::TextEdit::singleline(&mut self.destination)
                    .desired_width(f32::INFINITY)
                    .hint_text(aurora_i18n::tr!("portal-destination-hint")),
            );
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("portal-access"));
            widgets::radio_value(ui, p, &mut self.access, Access::Everyone, aurora_i18n::tr!("portal-everyone"));
            widgets::radio_value(ui, p, &mut self.access, Access::Friends, aurora_i18n::tr!("portal-friends"));
            let selected_group = match self.access {
                Access::Group(id) => id,
                _ => Uuid::nil(),
            };
            let first_group = world.groups.groups.first().map_or(Uuid::nil(), |group| group.id);
            let mut group_only = matches!(self.access, Access::Group(_));
            let group_radio = widgets::radio(ui, p, group_only, aurora_i18n::tr!("portal-group"));
            #[cfg(test)]
            {
                self.rects[0] = Some(group_radio.rect);
            }
            if group_radio.clicked() {
                group_only = true;
                self.access = Access::Group(if selected_group.is_nil() { first_group } else { selected_group });
            }
            if group_only {
                let mut selected = if let Access::Group(id) = self.access { id } else { Uuid::nil() };
                egui::ComboBox::from_id_salt("portal-group")
                    .selected_text(
                        world
                            .groups
                            .groups
                            .iter()
                            .find(|group| group.id == selected)
                            .map_or(aurora_i18n::tr!("portal-select-group"), |group| group.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for group in &world.groups.groups {
                            ui.selectable_value(&mut selected, group.id, &group.name);
                        }
                    });
                self.access = Access::Group(selected);
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(aurora_i18n::tr!("portal-help")).color(p.muted));
            if !ready {
                ui.label(egui::RichText::new(aurora_i18n::tr!("portal-bridge-required")).color(p.warn));
            }
            if let Some(notice) = &self.notice {
                ui.label(egui::RichText::new(notice).color(p.warn));
            }
            let valid = slurl::parse(&self.destination).is_some_and(|location| location.grid.is_none())
                && !matches!(self.access, Access::Group(id) if id.is_nil());
            ui.add_space(8.0);
            ui.add_enabled_ui(ready && valid && !world.teleporting, |ui| {
                let create =
                    widgets::primary_button_sized(ui, p, aurora_i18n::tr!("portal-create"), egui::vec2(ui.available_width(), 30.0));
                actions.create = create.clicked();
                #[cfg(test)]
                {
                    self.rects[1] = Some(create.rect);
                }
            });
            if let Some(own) = world.portals.own() {
                let seconds = own.portal.expires.saturating_sub(portals::unix_time());
                ui.label(aurora_i18n::tr!("portal-expires", seconds = seconds.to_string()));
                actions.close = ui.button(aurora_i18n::tr!("portal-close")).clicked();
            }
        });
        actions
    }
}

fn label_text(world: &World, entry: &portals::Entry) -> String {
    let authorized = entry.allowed(world);
    if !authorized {
        return format!("{}\n{}", aurora_i18n::tr!("portal-private"), aurora_i18n::tr!("portal-restricted"));
    }
    let details = entry.details.as_ref().filter(|_| authorized);
    let destination = match details {
        None => aurora_i18n::tr!("portal-private").to_string(),
        Some(_) if world.rlva.has("showloc") => aurora_i18n::tr!("portal-destination-hidden").to_string(),
        Some(details) => {
            let destination = &details.destination;
            format!(
                "{} ({:.0}, {:.0}, {:.0})",
                destination.region, destination.position[0], destination.position[1], destination.position[2]
            )
        }
    };
    let access = match details.map(|details| details.access) {
        Some(Access::Everyone) => aurora_i18n::tr!("portal-everyone"),
        Some(Access::Friends) => aurora_i18n::tr!("portal-friends"),
        Some(Access::Group(_)) => aurora_i18n::tr!("portal-group"),
        None => aurora_i18n::tr!("portal-private"),
    };
    let status = if entry.waiting_for_clearance() {
        aurora_i18n::tr!("portal-step-away")
    } else {
        aurora_i18n::tr!("portal-walk-through")
    };
    let name = world
        .person_name(&entry.label_owner())
        .unwrap_or_else(|| aurora_i18n::tr!("poser-resident").into());
    let name = world.rlva_hover_privacy.apply(&name);
    format!("{destination}\n{name} · {access}\n{status}")
}

pub fn labels(ctx: &egui::Context, p: &Palette, world: &World, projector: &Projector) {
    let painter = super::hud::world_painter(ctx);
    for entry in world.portals.visible(world) {
        let center = Vec3::from(entry.portal.center);
        if center.distance(world.agent.position) > 40.0 {
            continue;
        }
        let anchor = center + Vec3::Z * (portals::HEIGHT + 0.25);
        let Some((position, _)) = projector.project(anchor) else { continue };
        let text = label_text(world, entry);
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), p.ink);
        let rect = egui::Rect::from_center_size(position, galley.size() + egui::vec2(18.0, 10.0));
        aurora_render::world_label_at(ctx, Some(anchor));
        widgets::paint_world_label(&painter, p, rect);
        painter.galley(rect.min + egui::vec2(9.0, 5.0), galley, p.ink);
    }
    aurora_render::world_label_at(ctx, None);
}

#[cfg(test)]
mod tests;
