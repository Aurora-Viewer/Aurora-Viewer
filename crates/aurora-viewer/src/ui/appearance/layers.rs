//! Worn clothing layers in the outfit editor: shown furthest from the body
//! first, moved one step by their arrows or dragged within their wearable
//! type. Port of LLCOFWearables::populateClothingList, buildClothingListItem
//! and its drag to LLAppearanceMgr::reorderWearableGroup
//! (indra/newview/llcofwearables.cpp, llappearancemgr.cpp, originally
//! LGPL 2.1).

use crate::{theme::Palette, world::inventory::Inventory};
use aurora_net::outfits::OutfitLink;
use egui::{Pos2, Rect, Response};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The wearable type of a clothing link (`flags & 0xff`), None for anything
/// else: body parts and attachments are not reordered.
pub(super) fn clothing_kind(inv: &Inventory, link: &OutfitLink) -> Option<u32> {
    if link.folder {
        return None;
    }
    inv.items
        .get(&link.target)
        .filter(|it| it.asset_type == 5)
        .map(|it| it.flags & 0xff)
}

/// A link's place in the outfit, `@<type * 100 + index>` in its
/// description (LLAppearanceMgr::updateClothingOrderingInfo), compared as
/// a number: as text, "@1400" (a tattoo) would come before "@500" (pants).
pub(super) fn order(link: &OutfitLink) -> Option<u32> {
    link.desc.strip_prefix('@').and_then(|n| n.parse().ok())
}

/// `links`, sorted closest to the body first, with each clothing type's
/// layers reversed in their own places: the furthest from the body on top,
/// as the Legacy Viewer lists them.
pub(super) fn furthest_first(inv: &Inventory, links: &mut [OutfitLink]) {
    let kinds: Vec<_> = links.iter().map(|l| clothing_kind(inv, l)).collect();
    for kind in kinds.iter().flatten().copied().collect::<BTreeSet<_>>() {
        let at: Vec<usize> = (0..kinds.len()).filter(|i| kinds[*i] == Some(kind)).collect();
        for n in 0..at.len() / 2 {
            links.swap(at[n], at[at.len() - 1 - n]);
        }
    }
}

struct Row {
    id: Uuid,
    kind: u32,
    rect: Rect,
}

/// Where a dragged layer would land.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// Its body index in its type (0: closest to the body), and the height
    /// of the insertion marker.
    Move { body: usize, y: f32 },
    /// Where it already is.
    Stay,
}

/// The drag of a clothing row.
#[derive(Default)]
pub(super) struct Drag {
    from: Option<Uuid>,
    rows: Vec<Row>,
    /// The pointer's last position: it may leave the window before the
    /// button is let go.
    last_pointer: Option<Pos2>,
}

impl Drag {
    pub fn begin(&mut self, ctx: &egui::Context) {
        self.rows.clear();
        // the synthetic drag of the demo runs in a window that may not have
        // the focus
        let unfocused = ctx.input(|i| !i.focused) && demo_mode().is_none();
        if self.from.is_some()
            && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                || unfocused
                || ctx.input(|i| !i.pointer.primary_down() && !i.pointer.primary_released()))
        {
            self.cancel();
        }
    }

    fn cancel(&mut self) {
        self.from = None;
        self.last_pointer = None;
    }

    /// A draggable layer of the wearable type `kind`, in display order.
    pub fn row(&mut self, id: Uuid, kind: u32, response: &Response) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.from = Some(id);
        }
        self.rows.push(Row {
            id,
            kind,
            rect: response.rect,
        });
    }

    fn landing(&self, at: Pos2, clip: Rect) -> Option<Landing> {
        let id = self.from?;
        let source = self.rows.iter().find(|row| row.id == id)?;
        if !clip.contains(at) {
            return None;
        }
        // only among the layers of its own type, as reorderWearableGroup
        let group: Vec<&Row> = self.rows.iter().filter(|row| row.kind == source.kind).collect();
        let (first, last) = (group.first()?, group.last()?);
        let half = source.rect.height() * 0.5;
        if at.y < first.rect.top() - half || at.y > last.rect.bottom() + half {
            return None;
        }
        let others: Vec<&Row> = group.iter().copied().filter(|row| row.id != id).collect();
        let index = others.iter().filter(|row| row.rect.center().y < at.y).count();
        if Some(index) == group.iter().position(|row| row.id == id) {
            return Some(Landing::Stay);
        }
        let y = others
            .get(index)
            .map_or_else(|| others.last().map(|r| r.rect.bottom() + 1.0), |r| Some(r.rect.top() - 1.0))?;
        // shown furthest first: the top of the list is the last body index
        Some(Landing::Move {
            body: group.len() - 1 - index,
            y,
        })
    }

    /// After the rows: the drag's marker, cursor and scrolling. A drop
    /// gives the layer and its new body index.
    pub fn finish(&mut self, ui: &mut egui::Ui, p: &Palette) -> Option<(Uuid, usize)> {
        if demo_mode().is_some() && self.from.is_none() {
            self.keep_demo_rects(ui.ctx());
        }
        let id = self.from?;
        let Some(source) = self.rows.iter().find(|row| row.id == id).map(|row| row.rect) else {
            self.cancel();
            return None;
        };
        let clip = ui.clip_rect();
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if pointer.is_some() {
            self.last_pointer = pointer;
        }
        let landing = pointer.or(self.last_pointer).and_then(|at| self.landing(at, clip));
        ui.painter()
            .rect_filled(source, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.12));
        if let Some(Landing::Move { y, .. }) = landing {
            // The insertion marker shows the destination, not a row separator.
            ui.painter().with_clip_rect(clip).line_segment(
                [egui::pos2(source.left() + 3.0, y), egui::pos2(source.right() - 3.0, y)],
                egui::Stroke::new(2.0, p.violet_light),
            );
        }
        if ui.input(|i| i.pointer.primary_released()) {
            self.cancel();
            return match landing {
                Some(Landing::Move { body, .. }) => Some((id, body)),
                _ => None,
            };
        }
        ui.ctx().set_cursor_icon(if landing.is_some() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if let Some(at) = pointer.filter(|at| clip.contains(*at)) {
            let edge = 24.0;
            let speed = if at.y < clip.top() + edge {
                (clip.top() + edge - at.y) / edge
            } else if at.y > clip.bottom() - edge {
                -(at.y - clip.bottom() + edge) / edge
            } else {
                0.0
            };
            let dt = ui.input(|i| i.stable_dt.min(0.1));
            ui.scroll_with_delta(egui::vec2(0.0, speed * 420.0 * dt));
        }
        ui.ctx().request_repaint();
        None
    }

    /// The first and last rows of the first type with three layers or more,
    /// for the synthetic drag.
    fn keep_demo_rects(&self, ctx: &egui::Context) {
        let Some(kind) = self
            .rows
            .iter()
            .map(|row| row.kind)
            .find(|kind| self.rows.iter().filter(|row| row.kind == *kind).count() >= 3)
        else {
            return;
        };
        let mut group = self.rows.iter().filter(|row| row.kind == kind);
        if let (Some(first), Some(last)) = (group.next(), group.next_back()) {
            ctx.data_mut(|d| d.insert_temp(demo_id(), (first.rect, last.rect)));
        }
    }
}

fn demo_id() -> egui::Id {
    egui::Id::unique("appearance_layer_drag_demo")
}

/// `AURORA_DEMO_APPEARANCE=layers-drag|layers-hold`: whether the synthetic
/// drag holds on over its landing place.
fn demo_mode() -> Option<bool> {
    match std::env::var("AURORA_DEMO_APPEARANCE").ok()?.as_str() {
        "layers-drag" => Some(false),
        "layers-hold" => Some(true),
        _ => None,
    }
}

/// Synthetic pointer events of the layer demo: the top layer of the
/// tattoos taken at frame 701, moved for 19 frames under the last one, and
/// let go at frame 760 (held on with `layers-hold`).
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=760).contains(&frame) {
        return Vec::new();
    }
    let Some(hold) = demo_mode() else {
        return Vec::new();
    };
    let Some((first, last)) = ctx.data(|d| d.get_temp::<(Rect, Rect)>(demo_id())) else {
        return Vec::new();
    };
    let start = egui::pos2(first.left() + first.width() * 0.4, first.center().y);
    let end = egui::pos2(start.x, last.bottom() - 4.0);
    let pos = start.lerp(end, (frame.saturating_sub(701) as f32 / 19.0).min(1.0));
    let mut events = vec![egui::Event::PointerMoved(pos)];
    if frame == 701 || (frame == 760 && !hold) {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: frame == 701,
            modifiers: egui::Modifiers::NONE,
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const C: Uuid = Uuid::from_u128(3);
    const D: Uuid = Uuid::from_u128(4);

    const CLIP: Rect = Rect {
        min: egui::pos2(0.0, 0.0),
        max: egui::pos2(400.0, 600.0),
    };

    /// Three tattoos A, B, C shown from y 100, 24 high, 1 apart, then a
    /// jacket D.
    fn drag(from: Uuid) -> Drag {
        let mut drag = Drag {
            from: Some(from),
            ..Drag::default()
        };
        for (n, (id, kind)) in [(A, 14), (B, 14), (C, 14), (D, 8)].into_iter().enumerate() {
            drag.rows.push(Row {
                id,
                kind,
                rect: Rect::from_min_size(egui::pos2(10.0, 100.0 + 25.0 * n as f32), egui::vec2(200.0, 24.0)),
            });
        }
        drag
    }

    #[test]
    fn a_layer_lands_at_a_body_index_of_its_own_type() {
        let at = |y| egui::pos2(50.0, y);
        let st = drag(A);
        // over itself, then under B, then under C: A on top is body index 2
        assert_eq!(st.landing(at(110.0), CLIP), Some(Landing::Stay));
        assert_eq!(st.landing(at(140.0), CLIP), Some(Landing::Move { body: 1, y: 149.0 }));
        assert_eq!(st.landing(at(168.0), CLIP), Some(Landing::Move { body: 0, y: 175.0 }));
        // over the jacket, or far above: none
        assert_eq!(st.landing(at(200.0), CLIP), None);
        assert_eq!(st.landing(at(60.0), CLIP), None);
        // C, closest to the body, taken to the top
        let st = drag(C);
        assert_eq!(st.landing(at(102.0), CLIP), Some(Landing::Move { body: 2, y: 99.0 }));
        // outside the list
        assert_eq!(st.landing(egui::pos2(500.0, 102.0), CLIP), None);
        // the jacket alone has nowhere to go
        assert_eq!(drag(D).landing(at(110.0), CLIP), None);
    }

    #[test]
    fn layers_are_listed_furthest_from_the_body_first() {
        let mut inv = Inventory::default();
        let item = |n: u128, asset_type: i32, flags: u32| aurora_net::inventory::InvItem {
            id: Uuid::from_u128(n),
            asset_type,
            flags,
            ..Default::default()
        };
        inv.add_items(vec![item(1, 13, 1), item(2, 5, 14), item(3, 5, 14), item(4, 5, 8), item(5, 5, 14)]);
        let link = |n: u128| OutfitLink {
            target: Uuid::from_u128(n),
            name: String::new(),
            desc: String::new(),
            folder: false,
            inv_type: 18,
        };
        // the skin, tattoos 2 and 3, the jacket and tattoo 5, closest first
        let mut links: Vec<_> = [1, 2, 3, 4, 5].into_iter().map(link).collect();
        furthest_first(&inv, &mut links);
        let order: Vec<_> = links.iter().map(|l| l.target.as_u128()).collect();
        assert_eq!(order, [1, 5, 3, 4, 2]);
        // a tattoo's place comes after the pants', as a number
        let placed = |desc: &str| OutfitLink {
            desc: desc.into(),
            ..link(1)
        };
        assert!(super::order(&placed("@1400")) > super::order(&placed("@500")));
        assert_eq!(super::order(&placed("")), None);
    }
}
