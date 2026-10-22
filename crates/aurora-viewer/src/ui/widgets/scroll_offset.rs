//! A scroll area's offset decided after it is shown (rows above that changed
//! height, an automatic scroll). egui 0.37 keeps the offset of
//! `scroll_area::State` private: the offset is handed back to the area at its
//! next show through `ScrollArea::scroll_offset`, which is when a state stored
//! after the show used to take effect.

use egui::{AsIdSalt, Context, Id, IdSalt, ScrollArea, Ui, Vec2};

fn pending_id(area: Id) -> Id {
    area.with("aurora_pending_scroll_offset")
}

/// Moves the scroll area `area` (its `ScrollAreaOutput::id`) to `offset` at
/// its next show through `show`.
pub fn set(ctx: &Context, area: Id, offset: Vec2) {
    ctx.data_mut(|d| d.insert_temp(pending_id(area), offset));
}

/// Where the scroll area `area` is shown next: the offset `set` gave it, else
/// the one of its state.
#[cfg(test)]
pub fn next(ctx: &Context, area: Id) -> Option<Vec2> {
    let pending = ctx.data(|d| d.get_temp::<Vec2>(pending_id(area)));
    pending.or_else(|| egui::scroll_area::State::load(ctx, area).map(|s| s.unclamped_offset()))
}

/// `area` with the id salt `salt`, at the offset `set` gave it since its last
/// show in `ui`, if any.
pub fn show(ui: &Ui, area: ScrollArea, salt: impl AsIdSalt) -> ScrollArea {
    let salt = IdSalt::new(salt);
    let area = area.id_salt(salt);
    // the id egui gives the area: `id_salt` hashes the salt again, then
    // `ScrollArea::show` salts the ui's id with it
    let id = ui.make_persistent_id(IdSalt::new(salt));
    match ui.ctx().data_mut(|d| d.remove_temp::<Vec2>(pending_id(id))) {
        Some(offset) => area.scroll_offset(offset),
        None => area,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &Context, salt: &str, top: &mut f32) -> Id {
        let mut id = Id::NULL;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let area = show(ui, ScrollArea::vertical().max_height(100.0), salt);
            let out = area.show(ui, |ui| {
                for i in 0..100 {
                    ui.label(format!("row {i}"));
                }
            });
            *top = out.state.unclamped_offset().y;
            id = out.id;
        });
        output.textures_delta.clear();
        id
    }

    /// The offset set after a show is the area's at the next one, and only
    /// then; the id the area is found by is egui's.
    #[test]
    fn the_offset_set_after_a_show_is_the_next_one() {
        let ctx = Context::default();
        let mut top = 0.0;
        let id = frame(&ctx, "list", &mut top);
        assert_eq!(top, 0.0);
        set(&ctx, id, Vec2::new(0.0, 250.0));
        assert_eq!(frame(&ctx, "list", &mut top), id);
        assert_eq!(top, 250.0);
        // the other areas keep theirs
        set(&ctx, id, Vec2::new(0.0, 40.0));
        frame(&ctx, "other", &mut top);
        assert_eq!(top, 0.0);
        frame(&ctx, "list", &mut top);
        assert_eq!(top, 40.0);
        frame(&ctx, "list", &mut top);
        assert_eq!(top, 40.0, "the offset given once stays the area's own");
    }
}
