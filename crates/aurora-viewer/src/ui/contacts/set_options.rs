//! The options shared by every contact set, in a window of their own
//! (opened from the gear of People's sets and from a set editor's
//! Behaviour tab): the default colour of a new set (the global swatch of
//! FSFloaterContactSetConfiguration, mGlobalSwatch), the online notices of
//! the sets with "Show notifications" as a toast or in nearby chat
//! (FSContactSetsNotificationToast / ...NearbyChat), and whether a plate
//! shows every set's badge or the highest set's only. The Legacy Viewer
//! keeps the default colour in each set's floater; Aurora gathers what
//! applies to all sets here, so that a set's editor is about that set.

use super::*;

const SIZE: Vec2 = Vec2::new(440.0, 300.0);

pub(super) fn window(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi) {
    if !st.options_open {
        return;
    }
    // asked for again: whole and in front (LLFloater::openFloater)
    if std::mem::take(&mut st.options_raise) {
        widgets::bring_floater_front(ctx, "contact_set_options");
    }
    let mut open = true;
    widgets::Floater::new(
        "contact_set_options",
        aurora_i18n::tr!("set-options-title"),
        ctx.content_rect().center() - SIZE / 2.0,
        SIZE,
    )
    .auto_height()
    .show(ctx, p, &mut open, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if let Some(c) = set_color_button(ui, p, "default_color", world.contact_sets.default_color) {
                world.contact_sets.set_default_color(c);
            }
            ui.label(RichText::new(aurora_i18n::tr!("ui-contacts-default-color")).size(12.5).color(p.ink));
        });
        widgets::divider(ui, p);
        let mut toast = world.contact_sets.notices_toast;
        if widgets::switch_labeled_wrapped(ui, p, &mut toast, aurora_i18n::tr!("ui-contacts-set-notices-toast")).changed() {
            world.contact_sets.set_notices_toast(toast);
        }
        let mut chat = world.contact_sets.notices_chat;
        if widgets::switch_labeled_wrapped(ui, p, &mut chat, aurora_i18n::tr!("ui-contacts-set-notices-chat")).changed() {
            world.contact_sets.set_notices_chat(chat);
        }
        widgets::divider(ui, p);
        let mut stack = world.contact_sets.stack_icons;
        if widgets::switch_labeled_wrapped(ui, p, &mut stack, aurora_i18n::tr!("set-options-stack")).changed() {
            world.contact_sets.set_stack_icons(stack);
        }
    });
    if !open {
        st.options_open = false;
    }
}
