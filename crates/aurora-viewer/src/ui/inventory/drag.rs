//! LLFolderView::startDrag / LLFolderBridge::{dragOrDrop,dragItemIntoFolder}
//! (Firestorm indra/{llui,newview}, originally LGPL 2.1). Snapshot the selected
//! inventory IDs; use the existing move mutation and validate the whole batch.

use super::*;
use std::sync::Arc;

/// A batch is deliberately distinct from single-item world/attachment cargo:
/// a destination which accepts one item must never silently take its first ID.
#[derive(Clone)]
struct SelectionDrag(Vec<Uuid>);

enum Cargo {
    Item(Arc<InvDrag>),
    Folder(Arc<FolderDrag>),
    Selection(Arc<SelectionDrag>),
}
impl Cargo {
    fn hover(ui: &egui::Ui, r: &egui::Response) -> Option<Self> {
        if !ui.rect_contains_pointer(r.rect) {
            return None;
        }
        Self::carried(ui.ctx())
    }
    /// The inventory a drag in progress carries, wherever the pointer is.
    fn carried(ctx: &egui::Context) -> Option<Self> {
        egui::DragAndDrop::payload::<SelectionDrag>(ctx)
            .map(Self::Selection)
            .or_else(|| egui::DragAndDrop::payload::<InvDrag>(ctx).map(Self::Item))
            .or_else(|| egui::DragAndDrop::payload::<FolderDrag>(ctx).map(Self::Folder))
    }
    fn ids(&self) -> &[Uuid] {
        match self {
            Self::Item(d) => std::slice::from_ref(&d.0),
            Self::Folder(d) => std::slice::from_ref(&d.0),
            Self::Selection(d) => &d.0,
        }
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
    let right = r.drag_started_by(egui::PointerButton::Secondary);
    ui.ctx().data_mut(|d| d.insert_temp(right_drag_id(), right));
    let ids = st.selected_ids(id);
    if ids.len() > 1 {
        r.dnd_set_drag_payload(SelectionDrag(ids));
    } else if folder {
        r.dnd_set_drag_payload(FolderDrag(id));
    } else {
        r.dnd_set_drag_payload(InvDrag(id));
    }
}

#[derive(Clone)]
struct Cached {
    ids: Vec<Uuid>,
    folder: Uuid,
    mode: DropMode,
    generation: u64,
    protected: HashSet<Uuid>,
    worn: HashSet<Uuid>,
    result: Arc<Result<Mutation, String>>,
}
#[derive(Clone, Default)]
struct Cache(Option<Cached>);

/// What a drop does. A plain drag moves, as in the Legacy Viewer
/// (LLFolderBridge::dragOrDrop); held keys copy or link as Windows
/// Explorer and Dolphin do: Ctrl copies, Alt or Ctrl+Shift makes links
/// (on a Mac, Finder's Option copies and Cmd+Option makes links). Aurora
/// difference. The Second Life rules still decide: a no-copy item is not
/// copied, the Trash and the Library are not linked from, as Paste and
/// Paste as link refuse them (`rules::paste`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DropMode {
    Move,
    Copy,
    Link,
}

pub(crate) fn mode_for(m: egui::Modifiers, mac: bool) -> DropMode {
    if mac {
        match (m.alt, m.mac_cmd || m.command) {
            (true, true) => DropMode::Link,
            (true, false) => DropMode::Copy,
            _ => DropMode::Move,
        }
    } else if m.alt || (m.ctrl && m.shift) {
        DropMode::Link
    } else if m.ctrl {
        DropMode::Copy
    } else {
        DropMode::Move
    }
}

/// The change a drop of `ids` into `to` makes.
fn mutation(
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    mode: DropMode,
    prefs: &InventoryPreferences,
    facts: &Facts,
) -> Result<Mutation, String> {
    match mode {
        DropMode::Move => rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn),
        DropMode::Copy => rules::paste(inv, ids, to, false, false, &prefs.protected, &facts.worn),
        DropMode::Link => rules::paste(inv, ids, to, false, true, &prefs.protected, &facts.worn),
    }
}

fn right_drag_id() -> egui::Id {
    egui::Id::unique("inventory_right_drag")
}

/// The drag in progress was started with the right button: its drop asks
/// what to do (Windows Explorer and Dolphin).
fn right_drag(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(right_drag_id())).unwrap_or(false)
}

/// The mode the drag in progress would drop with.
fn current_mode(ctx: &egui::Context) -> DropMode {
    if right_drag(ctx) {
        DropMode::Move
    } else {
        mode_for(ctx.input(|i| i.modifiers), cfg!(target_os = "macos"))
    }
}

/// A right-button drag released over a folder: Move here, Copy here,
/// Create links here, Cancel.
#[derive(Clone, Debug)]
pub(super) struct DropMenu {
    ids: Vec<Uuid>,
    to: Uuid,
    at: egui::Pos2,
    /// Whether each of Move, Copy and Link would go through, read when the
    /// cargo was dropped.
    allowed: [bool; 3],
}

/// Draws the menu of a right-button drop, if one waits, and does what is
/// chosen in it.
#[allow(clippy::too_many_arguments)]
pub(super) fn drop_menu(
    ui: &egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(menu) = st.drop_menu.clone() else { return };
    let mut open = true;
    let mut chosen = None;
    let entries = [
        (
            DropMode::Move,
            "hand-move",
            aurora_i18n::tr!("inventory-drop-move-here"),
            menu.allowed[0],
        ),
        (
            DropMode::Copy,
            "copy-filled",
            aurora_i18n::tr!("inventory-drop-copy-here"),
            menu.allowed[1],
        ),
        (
            DropMode::Link,
            "link-filled",
            aurora_i18n::tr!("inventory-drop-link-here"),
            menu.allowed[2],
        ),
    ];
    crate::ui::menu::popup_at(
        ui.ctx(),
        egui::Id::unique(("inventory_drop_menu", st.ui_id)),
        menu.at,
        &mut open,
        p,
        |ui| {
            for (mode, icon, label, enabled) in entries {
                if crate::ui::menu::item_tip(ui, p, icon, label, enabled, "") {
                    chosen = Some(mode);
                }
            }
            crate::ui::menu::separator(ui, p);
            if crate::ui::menu::item(ui, p, "x", aurora_i18n::tr!("common-cancel")) {
                chosen = None;
                ui.close();
            }
        },
    );
    if let Some(mode) = chosen {
        st.drop_menu = None;
        if st.pending.is_none() {
            drop_cargo(inv, &menu.ids, menu.to, mode, st, prefs, facts, actions);
        }
    } else if !open {
        st.drop_menu = None;
    }
}

fn plan(
    ctx: &egui::Context,
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    mode: DropMode,
    prefs: &InventoryPreferences,
    facts: &Facts,
) -> Arc<Result<Mutation, String>> {
    ctx.data_mut(|data| {
        let cache = &mut data.get_temp_mut_or_default::<Cache>(egui::Id::unique("inventory_drag_plan")).0;
        if let Some(c) = cache
            && c.ids == ids
            && c.folder == to
            && c.mode == mode
            && c.generation == inv.generation
            && c.protected == prefs.protected
            && c.worn == facts.worn
        {
            return c.result.clone();
        }
        // Avoid reconstructing a large move mutation on every hover frame.
        let result = Arc::new(mutation(inv, ids, to, mode, prefs, facts));
        *cache = Some(Cached {
            ids: ids.to_vec(),
            folder: to,
            mode,
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
    /// What an inventory folder would do with the drop (None: a
    /// conversation, an object or no place that takes it).
    folder: Option<DropMode>,
}
fn feedback_id() -> egui::Id {
    egui::Id::unique("inventory_move_drag_feedback")
}

/// How many entries a drag of inventory carries while one is in progress (a
/// folder counts as one): a conversation says what a drop would do before
/// the pointer reaches it.
pub(crate) fn carried_count(ctx: &egui::Context) -> Option<usize> {
    Cargo::carried(ctx).map(|c| c.ids().len())
}

/// The entries a drag of inventory carries (an item, a folder or a
/// selection), wherever the pointer is.
pub(crate) fn carried_ids(ctx: &egui::Context) -> Option<Vec<Uuid>> {
    Cargo::carried(ctx).map(|c| c.ids().to_vec())
}

/// The cursor a destination outside the inventory windows gives the cargo
/// held over it this pass (an object's Contents tab).
pub(crate) fn feedback(ctx: &egui::Context, cursor: egui::CursorIcon) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            feedback_id(),
            Feedback {
                pass,
                cursor,
                folder: None,
            },
        )
    });
}

/// The cargo of `AURORA_DEMO_CONV_GIVE`: one item, or a selection of several.
pub(crate) fn demo_carry(ctx: &egui::Context, ids: Vec<Uuid>) {
    match ids[..] {
        [] => {}
        [id] => egui::DragAndDrop::set_payload(ctx, InvDrag(id)),
        _ => egui::DragAndDrop::set_payload(ctx, SelectionDrag(ids)),
    }
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
    if !ctx.rect_contains_pointer(layer, zone) {
        return None;
    }
    let cargo = Cargo::carried(ctx)?;
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            feedback_id(),
            Feedback {
                pass,
                cursor: egui::CursorIcon::Copy,
                folder: None,
            },
        )
    });
    let released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    if released {
        egui::DragAndDrop::clear_payload(ctx);
    }
    Some((cargo.ids().to_vec(), released))
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
    // Aurora's window only: the Legacy-style window drops as the Legacy
    // Viewer does, a move with the left button
    let aurora = st.aurora.is_some();
    let right = aurora && right_drag(ui.ctx());
    let mode = if aurora { current_mode(ui.ctx()) } else { DropMode::Move };
    let plan = plan(ui.ctx(), inv, cargo.ids(), to, mode, prefs, facts);
    // a right-button drop asks first: a folder that takes any of the three
    let allowed = st.pending.is_none()
        && if right {
            [DropMode::Move, DropMode::Copy, DropMode::Link]
                .into_iter()
                .any(|mode| plan_allows(ui.ctx(), inv, cargo.ids(), to, mode, prefs, facts))
        } else {
            plan.as_ref().as_ref().is_ok_and(|p| !p.operations.is_empty())
        };
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            feedback_id(),
            Feedback {
                pass,
                cursor: match (allowed, mode) {
                    (false, _) => egui::CursorIcon::NotAllowed,
                    (true, DropMode::Move) => egui::CursorIcon::Move,
                    (true, DropMode::Copy) => egui::CursorIcon::Copy,
                    (true, DropMode::Link) => egui::CursorIcon::Alias,
                },
                folder: allowed.then_some(mode),
            },
        );
    });
    tree::drop_outline(ui, p, r.rect, allowed);
    if right && ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary)) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        if allowed && let Some(at) = ui.input(|i| i.pointer.interact_pos()) {
            let allowed = [DropMode::Move, DropMode::Copy, DropMode::Link]
                .map(|mode| plan_allows(ui.ctx(), inv, cargo.ids(), to, mode, prefs, facts));
            st.drop_menu = Some(DropMenu {
                ids: cargo.ids().to_vec(),
                to,
                at,
                allowed,
            });
        }
        return;
    }
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        // Revalidate on release: permissions, pending requests and inventory
        // may have changed while the cursor was held still over a folder.
        if st.pending.is_some() {
            st.message = aurora_i18n::tr!("ui-inventory-wait-for-the-current-change-to-finish").into();
        } else {
            drop_cargo(inv, cargo.ids(), to, mode, st, prefs, facts, actions);
        }
    }
}

/// What a drop does: a move as before, a copy or links by Paste's routes
/// (the Marketplace's paste, the outfit folders' rules).
#[allow(clippy::too_many_arguments)]
fn drop_cargo(
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    mode: DropMode,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    match mode {
        DropMode::Move => match rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn) {
            Ok(change) if !change.operations.is_empty() => {
                st.message.clear();
                actions.push(InvAction::Drop(change));
            }
            Ok(_) => {}
            Err(reason) => st.message = reason,
        },
        DropMode::Copy | DropMode::Link => {
            super::clipboard::paste_ids(inv, ids, to, false, mode == DropMode::Link, st, prefs, facts, actions);
        }
    }
}

fn plan_allows(
    ctx: &egui::Context,
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    mode: DropMode,
    prefs: &InventoryPreferences,
    facts: &Facts,
) -> bool {
    plan(ctx, inv, ids, to, mode, prefs, facts)
        .as_ref()
        .as_ref()
        .is_ok_and(|p| !p.operations.is_empty())
}

/// The folders a drag that may copy or link carries (Ctrl, Alt, or the
/// right button, whose menu offers both): their whole tree is loaded for
/// the copy, as Copy / Paste loads it (`InventoryUi::clipboard_loaded`).
pub(super) fn copied_folders(ctx: &egui::Context, inv: &Inventory) -> Vec<Uuid> {
    let copying = right_drag(ctx) || current_mode(ctx) != DropMode::Move;
    match Cargo::carried(ctx) {
        Some(cargo) if copying => cargo.ids().iter().copied().filter(|id| inv.folders.contains_key(id)).collect(),
        _ => Vec::new(),
    }
}

/// Resolve feedback after all windows, like the object-inventory drag cursor.
pub(crate) fn cursor(ctx: &egui::Context) -> Option<egui::CursorIcon> {
    if !ctx.input(|i| i.pointer.primary_down() || (i.pointer.secondary_down() && right_drag(ctx))) {
        return None;
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Feedback>(feedback_id()))
        .filter(|f| f.pass == pass)
        .map(|f| f.cursor)
        .or_else(|| egui::DragAndDrop::has_payload_of_type::<SelectionDrag>(ctx).then_some(egui::CursorIcon::NotAllowed))
}

pub(crate) fn badge(ctx: &egui::Context, p: &Palette) {
    let Some(cargo) = Cargo::carried(ctx) else {
        return;
    };
    // (Esc ends a drag without dropping: egui's DragAndDrop does it)
    let held = ctx.input(|i| i.pointer.primary_down() || (i.pointer.secondary_down() && right_drag(ctx)));
    let Some(pointer) = ctx.input(|i| i.pointer.interact_pos().filter(|_| held)) else {
        return;
    };
    let count = cargo.ids().len();
    // what the drop will do, as Explorer's "+ Copy" / "Create link" badge;
    // a single entry moved says nothing, as before
    // over an inventory folder that takes it, what that folder would do;
    // elsewhere (the world, a conversation) the keys mean what that place says
    let pass = ctx.cumulative_pass_nr();
    let mode = ctx
        .data(|d| d.get_temp::<Feedback>(feedback_id()))
        .filter(|f| f.pass == pass)
        .and_then(|f| f.folder)
        .unwrap_or(DropMode::Move);
    let label = match mode {
        DropMode::Move if count > 1 => aurora_i18n::tr!("inventory-move-selection-count", count = count),
        DropMode::Move => return,
        DropMode::Copy => aurora_i18n::tr!("inventory-copy-selection-count", count = count),
        DropMode::Link => aurora_i18n::tr!("inventory-link-selection-count", count = count),
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
