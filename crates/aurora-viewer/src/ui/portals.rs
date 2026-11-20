//! Portal creator and depth-occluded destination labels. Portals are this
//! viewer's own: nobody else sees them (`world/portals.rs`).
use super::{
    hud::Projector,
    widgets::{self, Floater},
};
use crate::{
    slurl,
    theme::Palette,
    world::{World, portals},
};
use glam::Vec3;

#[derive(Default)]
pub struct PortalUi {
    pub open: bool,
    pub destination: String,
    pub demo_seeded: bool,
    pub notice: Option<String>,
    #[cfg(test)]
    create_rect: Option<egui::Rect>,
}

#[derive(Default)]
pub struct Actions {
    pub create: bool,
    pub close: bool,
}

impl PortalUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> Actions {
        let mut actions = Actions::default();
        if !self.open {
            return actions;
        }
        Floater::new(
            "portals",
            aurora_i18n::tr!("portal-title"),
            egui::pos2(70.0, 150.0),
            egui::vec2(400.0, 260.0),
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
            ui.label(egui::RichText::new(aurora_i18n::tr!("portal-help")).color(p.muted));
            if let Some(notice) = &self.notice {
                ui.label(egui::RichText::new(notice).color(p.warn));
            }
            let valid = slurl::parse(&self.destination).is_some_and(|location| location.grid.is_none());
            ui.add_space(8.0);
            ui.add_enabled_ui(valid && !world.teleporting, |ui| {
                let create =
                    widgets::primary_button_sized(ui, p, aurora_i18n::tr!("portal-create"), egui::vec2(ui.available_width(), 30.0));
                actions.create = create.clicked();
                #[cfg(test)]
                {
                    self.create_rect = Some(create.rect);
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

fn label_text(world: &World, entry: &portals::Entry, unix: u64) -> String {
    let timer = aurora_i18n::tr!("portal-expires", seconds = entry.portal.expires.saturating_sub(unix).to_string());
    let destination = if world.rlva.has("showloc") {
        aurora_i18n::tr!("portal-destination-hidden").to_string()
    } else {
        let at = entry.portal.position;
        format!("{} ({:.0}, {:.0}, {:.0})", entry.portal.destination, at[0], at[1], at[2])
    };
    let status = if entry.waiting_for_clearance() {
        aurora_i18n::tr!("portal-step-away")
    } else {
        aurora_i18n::tr!("portal-walk-through")
    };
    format!("{destination}\n{timer}\n{status}")
}

pub fn labels(ctx: &egui::Context, p: &Palette, world: &World, projector: &Projector) {
    let painter = super::hud::world_painter(ctx);
    let unix = portals::unix_time();
    for entry in world.portals.visible(world) {
        let center = Vec3::from(entry.portal.center);
        if center.distance(world.agent.position) > 40.0 {
            continue;
        }
        let anchor = center + Vec3::Z * (portals::HEIGHT + 0.25);
        let Some((position, _)) = projector.project(anchor) else { continue };
        let text = label_text(world, entry, unix);
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
