//! Sample a drop at mouse-up even when winit batches later motion or modifier changes.
use egui::{Context, Event, InputState, LayerId, Modifiers, Order, PointerButton, Pos2, Rect, Ui};

pub(crate) struct Sample {
    pub position: Pos2,
    pub modifiers: Modifiers,
    pub released: bool,
}

pub(crate) fn read(input: &InputState) -> Option<Sample> {
    let released = input.pointer.primary_released();
    if released
        && let Some((position, modifiers)) = input.raw.events.iter().rev().find_map(|event| match event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            } => Some((*pos, *modifiers)),
            _ => None,
        })
    {
        return Some(Sample {
            position,
            modifiers,
            released,
        });
    }
    Some(Sample {
        position: input.pointer.interact_pos()?,
        modifiers: input.modifiers,
        released,
    })
}

/// The caller supplies the clipped rectangle in its layer's local coordinates.
pub(crate) fn over(ctx: &Context, layer: LayerId, rect: Rect) -> bool {
    let Some(pointer) = ctx.input(read) else { return false };
    let rect = ctx.layer_transform_to_global(layer).map_or(rect, |transform| transform * rect);
    rect.is_positive() && rect.contains(pointer.position) && ctx.layer_id_at(pointer.position) == Some(layer)
}

/// The root's remaining rectangle excludes panels but includes unobstructed world.
pub(crate) fn world(root: &Ui) -> Option<Sample> {
    let pointer = root.input(read)?;
    if !root.available_rect_before_wrap().contains(pointer.position)
        || root
            .ctx()
            .layer_id_at(pointer.position)
            .is_some_and(|layer| layer.order != Order::Background)
    {
        return None;
    }
    Some(pointer)
}
