//! Optional visual overrides and a preview using the actual nameplate painter.

use super::*;
use crate::world::contact_sets::appearance::{ICON_LABELS, ICONS};
use crate::world::contact_sets::{PlateSize, Visuals};

pub(super) fn editor(ui: &mut egui::Ui, p: &Palette, world: &mut World, name: &str, original: &Visuals) {
    let mut v = original.clone();
    let order = world.contact_sets.ordered_set_names();
    let position = order.iter().position(|s| s == name).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!(
            "contact-visual-priority",
            rank = position + 1,
            total = order.len()
        ));
        if widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("contact-visual-higher"), position > 0) {
            world.contact_sets.move_priority(name, true);
        }
        if widgets::icon_button(
            ui,
            p,
            "arrow-down",
            aurora_i18n::tr!("contact-visual-lower"),
            position + 1 < order.len(),
        ) {
            world.contact_sets.move_priority(name, false);
        }
    });
    ui.label(
        egui::RichText::new(aurora_i18n::tr!("contact-visual-inherit-hint"))
            .small()
            .color(p.muted),
    );
    for (value, key) in [
        (&mut v.border, "contact-visual-border"),
        (&mut v.background, "contact-visual-background"),
        (&mut v.nameplate_name, "contact-visual-plate-name"),
        (&mut v.ui_name, "contact-visual-ui-name"),
        (&mut v.chat_name, "contact-visual-chat-name"),
        (&mut v.chat_text, "contact-visual-chat-text"),
        (&mut v.minimap, "contact-visual-minimap"),
    ] {
        widgets::switch_labeled(ui, p, value, aurora_i18n::text(key));
    }
    ui.label(aurora_i18n::tr!("contact-visual-size"));
    for (size, key) in [
        (PlateSize::Inherit, "contact-visual-size-inherit"),
        (PlateSize::Expanded, "contact-visual-size-expanded"),
        (PlateSize::Reduced, "contact-visual-size-reduced"),
    ] {
        widgets::radio_value(ui, p, &mut v.size, size, aurora_i18n::text(key));
    }
    ui.label(
        egui::RichText::new(aurora_i18n::tr!("contact-visual-size-hint"))
            .small()
            .color(p.muted),
    );
    let mut has_icon = v.icon.is_some();
    if widgets::switch_labeled(ui, p, &mut has_icon, aurora_i18n::tr!("contact-visual-icon")).changed() {
        v.icon = has_icon.then_some(0);
    }
    if has_icon {
        egui::Grid::new("contact-visual-icons")
            .num_columns(8)
            .spacing(Vec2::splat(3.0))
            .show(ui, |ui| {
                for (i, icon) in ICONS.iter().enumerate() {
                    if widgets::icon_toggle(ui, p, icon, aurora_i18n::text(ICON_LABELS[i]), v.icon == Some(i)) {
                        v.icon = Some(i);
                    }
                    if (i + 1) % 8 == 0 {
                        ui.end_row();
                    }
                }
            });
        widgets::switch_labeled(ui, p, &mut v.replace_friend, aurora_i18n::tr!("contact-visual-replace-friend"));
    }
    world.contact_sets.set_visuals(name, v);
    let mut stack = world.contact_sets.stack_icons;
    if widgets::switch_labeled(ui, p, &mut stack, aurora_i18n::tr!("contact-visual-stack")).changed() {
        world.contact_sets.set_stack_icons(stack);
    }
}

pub(super) fn preview(ui: &mut egui::Ui, p: &Palette, world: &World, name: &str) {
    ui.label(aurora_i18n::tr!("contact-visual-preview"));
    if let Some(set) = world.contact_sets.set(name) {
        // Preview this set alone, even when it has no members yet. This uses
        // in-memory data only, and the same cascade/painter as live plates.
        let mut sample = ContactSets::default();
        sample.add_set(name);
        sample.set_color(name, set.color);
        sample.set_visuals(name, set.visuals.clone());
        let id = Uuid::from_u128(1);
        sample.add_to_set(&[id], name, |_| true);
        crate::ui::nameplate::preview(ui, p, sample.appearance(&id).clone());
    }
}
