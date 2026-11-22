//! LLFolderView::startDrag / LLFolderBridge::{dragOrDrop,dragItemIntoFolder}
//! (Firestorm indra/{llui,newview}, originally LGPL 2.1). Snapshot the selected
//! inventory IDs; use the existing move mutation and validate the whole batch.

use super::*;
use std::sync::Arc;

#[path = "drag_pointer.rs"]
pub(crate) mod pointer;

/// A batch is deliberately distinct from single-item world/attachment cargo:
/// a destination which accepts one item must never silently take its first ID.
#[derive(Clone)]
pub(super) struct SelectionDrag(Vec<Uuid>);

#[derive(Clone)]
pub(super) enum Cargo {
    Item(Arc<InvDrag>),
    Folder(Arc<FolderDrag>),
    Selection(Arc<SelectionDrag>),
}
impl Cargo {
    fn hover(ui: &egui::Ui, r: &egui::Response) -> Option<Self> {
        if !pointer::over(ui.ctx(), ui.layer_id(), ui.clip_rect().intersect(r.rect)) {
            return None;
        }
        Self::carried(ui.ctx())
    }
    /// The inventory a drag in progress carries, wherever the pointer is.
    pub(super) fn carried(ctx: &egui::Context) -> Option<Self> {
        egui::DragAndDrop::payload::<SelectionDrag>(ctx)
            .map(Self::Selection)
            .or_else(|| egui::DragAndDrop::payload::<InvDrag>(ctx).map(Self::Item))
            .or_else(|| egui::DragAndDrop::payload::<FolderDrag>(ctx).map(Self::Folder))
    }
    pub(super) fn ids(&self) -> &[Uuid] {
        match self {
            Self::Item(d) => std::slice::from_ref(&d.0),
            Self::Folder(d) => std::slice::from_ref(&d.0),
            Self::Selection(d) => &d.0,
        }
    }
}

/// Both inventory layouts publish exactly one payload, in the format understood
/// by world placement, Share, object contents and single-asset input controls.
pub(super) fn carry(ctx: &egui::Context, ids: Vec<Uuid>, folder: bool) {
    match ids.as_slice() {
        [] => {}
        [id] if folder => egui::DragAndDrop::set_payload(ctx, FolderDrag(*id)),
        [id] => egui::DragAndDrop::set_payload(ctx, InvDrag(*id)),
        _ => egui::DragAndDrop::set_payload(ctx, SelectionDrag(ids)),
    }
}

pub(super) fn source(ui: &egui::Ui, r: &egui::Response, id: Uuid, folder: bool, st: &mut InventoryUi) {
    record_row(ui.ctx(), st.ui_id, id, r.rect);
    if !r.drag_started() {
        return;
    }
    // Pressing a selected row must keep a Ctrl/Shift selection until the drag
    // threshold. Starting from another row selects that row alone.
    if !st.selection.contains(&id) {
        st.selection.clear();
        st.selection.insert(id);
        st.anchor = Some(id);
    }
    st.selected = Some(id);
    super::clipboard::request_focus(ui, st);
    carry(ui.ctx(), st.selected_ids(id), folder);
}

#[derive(Clone)]
struct Cached {
    ids: Vec<Uuid>,
    folder: Uuid,
    generation: u64,
    protected: HashSet<Uuid>,
    worn: HashSet<Uuid>,
    result: Arc<Result<Mutation, String>>,
}
#[derive(Clone, Default)]
struct Cache(Option<Cached>);

fn plan(
    ctx: &egui::Context,
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    prefs: &InventoryPreferences,
    facts: &Facts,
) -> Arc<Result<Mutation, String>> {
    ctx.data_mut(|data| {
        let cache = &mut data.get_temp_mut_or_default::<Cache>(egui::Id::unique("inventory_drag_plan")).0;
        if let Some(c) = cache
            && c.ids == ids
            && c.folder == to
            && c.generation == inv.generation
            && c.protected == prefs.protected
            && c.worn == facts.worn
        {
            return c.result.clone();
        }
        // Avoid reconstructing a large move mutation on every hover frame.
        let result = Arc::new(rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn));
        *cache = Some(Cached {
            ids: ids.to_vec(),
            folder: to,
            generation: inv.generation,
            protected: prefs.protected.clone(),
            worn: facts.worn.clone(),
            result: result.clone(),
        });
        result
    })
}

#[derive(Clone)]
struct Feedback {
    pass: u64,
    cursor: egui::CursorIcon,
    draws_label: bool,
}
fn feedback_id() -> egui::Id {
    egui::Id::unique("inventory_move_drag_feedback")
}

/// How many entries a drag of inventory carries while one is in progress (a
/// folder counts as one). Used by drop targets that take inventory cargo.
pub(crate) fn carried_count(ctx: &egui::Context) -> Option<usize> {
    Cargo::carried(ctx).map(|cargo| cargo.ids().len())
}

/// What a drop that gives inventory away would hand over: the inventory
/// cargo, or the landmark a Places row, a favorite or a search result carries
/// (LLFolderView::startDrag drags the landmark's inventory item from
/// Places, indra/llui/llfolderview.cpp). Only the give drops read it: a
/// place list reordering keeps its own cargo.
pub(crate) fn give_ids(ctx: &egui::Context) -> Option<Vec<Uuid>> {
    carried_ids(ctx).or_else(|| carried_landmark(ctx).map(|item| vec![item]))
}

pub(crate) fn give_count(ctx: &egui::Context) -> Option<usize> {
    carried_count(ctx).or_else(|| carried_landmark(ctx).map(|_| 1))
}

fn carried_landmark(ctx: &egui::Context) -> Option<Uuid> {
    egui::DragAndDrop::payload::<crate::ui::places::drag::PlaceDrag>(ctx)?.target.item()
}

/// The entries a drag of inventory carries (an item, a folder or a
/// selection), wherever the pointer is.
pub(crate) fn carried_ids(ctx: &egui::Context) -> Option<Vec<Uuid>> {
    Cargo::carried(ctx).map(|c| c.ids().to_vec())
}

/// The cursor a destination outside the inventory windows gives the cargo
/// held over it this pass (an object's Contents tab).
pub(crate) fn feedback(ctx: &egui::Context, cursor: egui::CursorIcon) {
    set_feedback(ctx, cursor, false);
}

pub(super) fn set_feedback(ctx: &egui::Context, cursor: egui::CursorIcon, draws_label: bool) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(feedback_id(), Feedback { pass, cursor, draws_label }));
}

/// The cargo of `AURORA_DEMO_CONV_GIVE`: one item, or a selection of several.
pub(crate) fn demo_carry(ctx: &egui::Context, ids: Vec<Uuid>) {
    carry(ctx, ids, false);
}

/// Inventory dragged over a conversation with a resident, which gives it
/// (LLFloaterIMSession::handleDragAndDrop → LLGiveInventory, Firestorm
/// indra/newview/llfloaterimsession.cpp): the items carried, and whether
/// they were released over `zone` this frame.
pub(crate) fn give_target(ui: &egui::Ui, zone: egui::Rect) -> Option<(Vec<Uuid>, bool)> {
    give_target_on(ui.ctx(), ui.layer_id(), ui.clip_rect().intersect(zone))
}

/// `give_target` for a zone of the layer `layer`, from outside its `Ui`: a
/// whole window, frame included (the profiles, #991).
pub(crate) fn give_target_on(ctx: &egui::Context, layer: egui::LayerId, zone: egui::Rect) -> Option<(Vec<Uuid>, bool)> {
    if !pointer::over(ctx, layer, zone) {
        return None;
    }
    let ids = give_ids(ctx)?;
    feedback(ctx, egui::CursorIcon::Copy);
    let released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    if released {
        egui::DragAndDrop::clear_payload(ctx);
    }
    Some((ids, released))
}

/// Rows precede their enclosing list. A rejected row consumes the release,
/// rather than accidentally moving the selection to the list's root folder.
pub(super) fn target(
    ui: &mut egui::Ui,
    p: &Palette,
    r: &egui::Response,
    inv: &Inventory,
    to: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(cargo) = Cargo::hover(ui, r) else { return };
    let pass = ui.ctx().cumulative_pass_nr();
    if ui
        .ctx()
        .data(|d| d.get_temp::<Feedback>(feedback_id()).is_some_and(|f| f.pass == pass))
    {
        return;
    }
    let plan = plan(ui.ctx(), inv, cargo.ids(), to, prefs, facts);
    let allowed = st.pending.is_none() && plan.as_ref().as_ref().is_ok_and(|p| !p.operations.is_empty());
    feedback(
        ui.ctx(),
        if allowed {
            egui::CursorIcon::Move
        } else {
            egui::CursorIcon::NotAllowed
        },
    );
    tree::drop_outline(ui, p, r.rect, allowed);
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        // Revalidate on release: permissions, pending requests and inventory
        // may have changed while the cursor was held still over a folder.
        if st.pending.is_some() {
            st.message = aurora_i18n::tr!("ui-inventory-wait-for-the-current-change-to-finish").into();
        } else {
            match rules::move_selection(inv, cargo.ids(), to, &prefs.protected, &facts.worn) {
                Ok(change) if !change.operations.is_empty() => {
                    st.message.clear();
                    actions.push(InvAction::Drop(change));
                }
                Ok(_) => {}
                Err(reason) => st.message = reason,
            }
        }
    }
}

/// Resolve feedback after all windows, like the object-inventory drag cursor.
pub(crate) fn cursor(ctx: &egui::Context) -> Option<egui::CursorIcon> {
    if !ctx.input(|i| i.pointer.primary_down()) {
        return None;
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Feedback>(feedback_id()))
        .filter(|f| f.pass == pass)
        .map(|f| f.cursor)
        .or_else(|| egui::DragAndDrop::has_payload_of_type::<SelectionDrag>(ctx).then_some(egui::CursorIcon::NotAllowed))
}

/// Destination-specific hints and cursor images share screen space while a
/// drag is held. Their rectangles are meaningful only in the current pass.
pub(crate) fn note_hint_rect(ctx: &egui::Context, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("inventory-drag-hint-rect"), (pass, rect)));
}

pub(super) fn hint_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(u64, egui::Rect)>(egui::Id::unique("inventory-drag-hint-rect")))
        .filter(|(at, _)| *at == pass)
        .map(|(_, rect)| rect)
}

pub(crate) fn badge(ctx: &egui::Context, p: &Palette, inv: &Inventory) {
    if super::organized::drag_preview::show(ctx, p, inv) {
        return;
    }
    let Some(pointer) = ctx.input(|i| i.pointer.interact_pos().filter(|_| i.pointer.primary_down())) else {
        return;
    };
    let pass = ctx.cumulative_pass_nr();
    if ctx.data(|d| {
        d.get_temp::<Feedback>(feedback_id())
            .is_some_and(|f| f.pass == pass && f.draws_label)
    }) {
        return;
    }
    let label = if let Some(cargo) = egui::DragAndDrop::payload::<SelectionDrag>(ctx) {
        aurora_i18n::tr!("inventory-move-selection-count", count = cargo.0.len())
    } else if let Some(count) = super::organized::drag_count(ctx) {
        aurora_i18n::tr!("explorer-input-moving", count = count)
    } else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("inventory_drag_count")));
    let text = painter.layout_no_wrap(label, tree::font(), p.ink);
    let size = text.size() + Vec2::new(16.0, 10.0);
    let area = ctx.content_rect();
    let pos = (pointer + Vec2::new(16.0, 20.0)).min(area.max - size).max(area.min);
    let rect = egui::Rect::from_min_size(pos, size);
    painter.rect_filled(rect, ctx.global_style().visuals.window_corner_radius, p.panel);
    painter.rect_stroke(
        rect,
        ctx.global_style().visuals.window_corner_radius,
        egui::Stroke::new(1.0, p.violet_light),
        egui::StrokeKind::Inside,
    );
    painter.galley(pos + Vec2::new(8.0, 5.0), text, p.ink);
}

pub(super) fn record_row(ctx: &egui::Context, editor: Uuid, id: Uuid, rect: egui::Rect) {
    if cfg!(test) || demo::enabled() {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("inventory_drag_row", editor, id)), rect));
    }
}

#[path = "drag_demo.rs"]
pub(crate) mod demo;
#[path = "drag_navigation.rs"]
pub(crate) mod navigation;
