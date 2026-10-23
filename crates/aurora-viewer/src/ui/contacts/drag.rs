//! Local contact-set assignment from the shared resident drag payload.

use super::*;
use crate::ui::call_invite::PersonDrag;

pub(crate) fn person_source(response: &egui::Response, id: Uuid) {
    if !id.is_nil() {
        response.interact(Sense::click_and_drag()).dnd_set_drag_payload(PersonDrag(id));
    }
}

pub(crate) fn set_target(response: &egui::Response, set: &str) {
    // Asking egui to release the wrong payload type would consume an
    // inventory item dragged through the same UI. Check before taking it.
    if ContactSets::is_internal(set) || !egui::DragAndDrop::has_payload_of_type::<PersonDrag>(&response.ctx) {
        return;
    }
    if response.dnd_hover_payload::<PersonDrag>().is_some() {
        response.ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
    if let Some(person) = response.dnd_release_payload::<PersonDrag>()
        && !person.0.is_nil()
    {
        context::request(
            &response.ctx,
            CtxAction::ContactSet(SetEdit::Add {
                set: set.to_owned(),
                ids: vec![person.0],
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn target(ctx: &egui::Context, events: Vec<egui::Event>, set: &str) -> (egui::Rect, bool) {
        let mut result = (egui::Rect::NOTHING, false);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(300.0, 200.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let response = ui.allocate_response(Vec2::new(120.0, 40.0), Sense::hover());
                set_target(&response, set);
                result = (
                    response.rect,
                    egui::DragAndDrop::has_payload_of_type::<crate::ui::inventory::InvDrag>(ctx),
                );
            },
        );
        // This headless interaction test does not render font-atlas uploads.
        output.textures_delta.clear();
        result
    }

    #[test]
    fn contact_set_drop_accepts_residents_but_leaves_inventory_and_internal_sets_alone() {
        let ctx = egui::Context::default();
        let id = Uuid::from_u128(13);
        let rect = target(&ctx, Vec::new(), "Friends").0;
        let at = rect.center();
        target(&ctx, vec![egui::Event::PointerMoved(at), button(at, true)], "Friends");
        egui::DragAndDrop::set_payload(&ctx, PersonDrag(id));
        target(&ctx, vec![button(at, false)], "Friends");
        let requests = context::take_requests(&ctx);
        assert!(matches!(&requests[..], [CtxAction::ContactSet(SetEdit::Add {set,ids})] if set == "Friends" && ids == &[id]));
        target(&ctx, vec![button(at, true)], "Friends");
        egui::DragAndDrop::set_payload(&ctx, crate::ui::inventory::InvDrag(id));
        assert!(target(&ctx, vec![button(at, false)], "Friends").1);
        assert!(context::take_requests(&ctx).is_empty());
        target(&ctx, vec![button(at, true)], "All Sets");
        egui::DragAndDrop::set_payload(&ctx, PersonDrag(id));
        target(&ctx, vec![button(at, false)], "All Sets");
        assert!(context::take_requests(&ctx).is_empty());
    }
}
