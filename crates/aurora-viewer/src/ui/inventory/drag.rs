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

/// DRAG_N_DROP_DISTANCE_THRESHOLD (indra/llui/llui.h): how far, in points,
/// the pointer travels from where it was pressed before a drag starts.
const DRAG_THRESHOLD: f32 = 3.0;

/// Port of LLFolderView::isOverDragThreshold (indra/llui/llfolderview.cpp):
/// strictly farther than the threshold from the press point.
pub(super) fn over_threshold(press: egui::Pos2, pointer: egui::Pos2) -> bool {
    press.distance_sq(pointer) > DRAG_THRESHOLD * DRAG_THRESHOLD
}

/// Whether the current press already started a drag. The Legacy Viewer
/// starts one drag per press (LLFolderViewItem::handleMouseDown captures the
/// mouse, handleHover starts the drag once), and a press that dragged is no
/// click. Every new press clears it, at the start of its frame.
#[derive(Default)]
struct PressLatch {
    dragged: bool,
    inventory_press: Option<egui::Pos2>,
    deferred_release: Option<Vec<egui::Event>>,
    later_input: Vec<egui::Event>,
}

impl egui::Plugin for PressLatch {
    fn debug_name(&self) -> &'static str {
        "InventoryDragPress"
    }

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if input.viewport_id != egui::ViewportId::ROOT {
            return;
        }
        if let Some(release) = self.deferred_release.take() {
            self.later_input.append(&mut input.events);
            input.events = release;
            if !self.later_input.is_empty() {
                ctx.request_repaint();
            }
            return;
        }
        if !self.later_input.is_empty() {
            self.later_input.append(&mut input.events);
            input.events = std::mem::take(&mut self.later_input);
        }
        let Some(origin) = self.inventory_press else {
            return;
        };
        if !input.focused
            || input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerGone
                        | egui::Event::WindowFocused(false)
                        | egui::Event::Key {
                            key: egui::Key::Escape,
                            pressed: true,
                            ..
                        }
                )
            })
        {
            self.inventory_press = None;
            return;
        }
        let Some((release, pressed, modifiers)) = input.events.iter().enumerate().find_map(|(index, event)| match event {
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
                ..
            } => Some((index, *pressed, *modifiers)),
            _ => None,
        }) else {
            return;
        };
        self.inventory_press = None;
        if let Some(motion) = (!pressed).then(|| &input.events[..release]).and_then(|events| {
            events
                .iter()
                .position(|event| matches!(event, egui::Event::PointerMoved(pos) if over_threshold(origin, *pos)))
        }) {
            // Give the source one pass with the button still held. Deliver
            // the actual release position and modifiers before newer input
            // can change the target or turn a copy into a move.
            self.later_input = input.events.split_off(release + 1);
            let mut events = input.events.split_off(motion + 1);
            events.insert(events.len() - 1, egui::Event::ModifiersChanged(modifiers));
            self.deferred_release = Some(events);
            log::debug!("inventory drag: process batched motion before primary release");
            ctx.request_repaint();
        }
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        // a right press too, as inventory rows start right-button drags, but
        // not one made while the left press that dragged is still held
        if ui.input(|i| i.pointer.primary_pressed() || (i.pointer.secondary_pressed() && !i.pointer.primary_down())) {
            self.dragged = false;
        }
    }
}

fn latch(ctx: &egui::Context) -> egui::plugin::TypedPluginHandle<PressLatch> {
    ctx.plugin_or_default::<PressLatch>()
}

/// Whether a drag from `r` starts this frame: the left button was pressed on
/// it and the pointer is now past the threshold from the press point
/// (LLFolderViewItem::handleMouseDown keeps the press, handleHover starts
/// the drag). egui's own `drag_started` also fires after the button is held
/// 0.8 s without moving, or as soon as the pointer slips onto the next row,
/// so a long or sloppy click moved a folder into its neighbour (#4057).
/// Only once per press: a drag cancelled with Escape does not start again
/// while the button stays down. Every inventory drag source goes through it.
pub(super) fn begins(ui: &egui::Ui, r: &egui::Response) -> bool {
    begins_by(ui, r, egui::PointerButton::Primary)
}

/// [`begins`] for a drag held with `button`: the inventory's own rows also
/// start one with the right button, whose drop asks what to do
/// ([`DropMenu`]), under the same threshold.
fn begins_by(ui: &egui::Ui, r: &egui::Response, button: egui::PointerButton) -> bool {
    let latch = latch(ui.ctx());
    let dragged = latch.lock().dragged;
    let begins = !dragged
        && r.is_pointer_button_down_on()
        && !egui::DragAndDrop::has_any_payload(ui.ctx())
        && ui.input(|i| {
            i.pointer.button_down(button)
                && !i.key_pressed(egui::Key::Escape)
                && i.pointer
                    .press_origin()
                    .zip(i.pointer.latest_pos())
                    .is_some_and(|(press, pointer)| over_threshold(press, pointer))
        });
    if begins {
        let mut latch = latch.lock();
        latch.dragged = true;
        latch.inventory_press = None;
    }
    begins
}

/// Whether the press now held or just released started a drag: its release
/// is a drop, not a click, even when it stayed within egui's click distance.
/// The Legacy Viewer keeps the selection after a drag and opens a notecard's
/// embedded item only on a press that did not move.
pub(super) fn dragged_this_press(ctx: &egui::Context) -> bool {
    latch(ctx).lock().dragged
}

pub(super) fn source(ui: &egui::Ui, r: &egui::Response, id: Uuid, folder: bool, st: &mut InventoryUi) {
    record_row(ui.ctx(), st.ui_id, id, r.rect);
    if ui.ctx().viewport_id() == egui::ViewportId::ROOT
        && r.is_pointer_button_down_on()
        && let Some(origin) = ui.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten())
    {
        latch(ui.ctx()).lock().inventory_press = Some(origin);
    }
    let right = if begins(ui, r) {
        false
    } else if begins_by(ui, r, egui::PointerButton::Secondary) {
        true
    } else {
        return;
    };
    // Pressing a selected row must keep a Ctrl/Shift selection until the drag
    // threshold. Starting from another row selects that row alone.
    if !st.selection.contains(&id) {
        st.selection.clear();
        st.selection.insert(id);
        st.anchor = Some(id);
    }
    st.selected = Some(id);
    super::clipboard::request_focus(ui, st);
    ui.ctx().data_mut(|d| d.insert_temp(right_drag_id(), right));
    let ids = st.selected_ids(id);
    if ids.len() > 1 {
        egui::DragAndDrop::set_payload(ui.ctx(), SelectionDrag(ids));
    } else if folder {
        egui::DragAndDrop::set_payload(ui.ctx(), FolderDrag(id));
    } else {
        egui::DragAndDrop::set_payload(ui.ctx(), InvDrag(id));
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

/// Whether a drop of `ids` into `to` would change anything (the menus'
/// Copy / Move to the other pane, greyed otherwise).
pub(super) fn would_drop(inv: &Inventory, ids: &[Uuid], to: Uuid, mode: DropMode, prefs: &InventoryPreferences, facts: &Facts) -> bool {
    mutation(inv, ids, to, mode, prefs, facts).is_ok_and(|plan| changes(&plan))
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
        // as Paste does it (`clipboard::paste_ids`): an outfit folder by its
        // rules; never the wearing a paste into the Current Outfit does
        DropMode::Copy => match crate::world::outfit_paste::plan(inv, ids, to, false, &prefs.protected)? {
            crate::world::outfit_paste::Paste::Inventory => rules::paste(inv, ids, to, false, false, &prefs.protected, &facts.worn),
            crate::world::outfit_paste::Paste::Edit(change) => Ok(change),
            crate::world::outfit_paste::Paste::Wear(_) => Err(aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").to_string()),
        },
        DropMode::Link => rules::paste(inv, ids, to, false, true, &prefs.protected, &facts.worn),
    }
}

fn right_drag_id() -> egui::Id {
    egui::Id::unique("inventory_right_drag")
}

/// The drag in progress was started with the right button: its drop asks
/// what to do (Windows Explorer and Dolphin). Only `source` sets the flag,
/// while many other places start inventory drags (the Texture Library,
/// the pickers, the profile…): it holds only while the right button is
/// down or is being released, and is forgotten otherwise, so a later left
/// drag from anywhere is never taken for a right one.
pub(super) fn right_drag(ctx: &egui::Context) -> bool {
    if !ctx.data(|d| d.get_temp::<bool>(right_drag_id())).unwrap_or(false) {
        return false;
    }
    // read apart from `data`: egui's context locks once at a time
    let held = ctx.input(|i| i.pointer.secondary_down() || i.pointer.button_released(egui::PointerButton::Secondary));
    if !held {
        ctx.data_mut(|d| d.remove::<bool>(right_drag_id()));
    }
    held
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

/// The pointer is over the world: inside the viewer's window, and over no
/// window of the interface. A held drag still tells where the pointer is
/// beyond the edges of the window it began in; nothing of the world is
/// there, and what is let go of there is carried to the viewer's other
/// windows (`outside::carry`), not dropped.
pub(crate) fn over_world(ctx: &egui::Context) -> bool {
    in_world(
        ctx.is_pointer_over_egui(),
        ctx.input(|i| i.pointer.interact_pos()),
        ctx.content_rect(),
    )
}

/// `over_world`, from what the interface says: a window of the interface
/// is under the pointer, where the pointer is, and the viewer's window.
pub(crate) fn in_world(over_interface: bool, at: Option<egui::Pos2>, window: egui::Rect) -> bool {
    !over_interface && at.is_none_or(|at| window.contains(at))
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
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
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
    let ids = give_ids(ctx)?;
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
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
    let pass = ui.ctx().cumulative_pass_nr_for(egui::ViewportId::ROOT);
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
            plan.as_ref().as_ref().is_ok_and(changes)
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
        let m = ui.input(|i| i.modifiers);
        log::info!(
            "inventory drop: {mode:?} of {} entries (allowed {allowed}; Ctrl {}, Shift {}, Alt {}, Command {})",
            cargo.ids().len(),
            m.ctrl,
            m.shift,
            m.alt,
            m.command
        );
        // Revalidate on release: permissions, pending requests and inventory
        // may have changed while the cursor was held still over a folder.
        if st.pending.is_some() {
            st.message = aurora_i18n::tr!("ui-inventory-wait-for-the-current-change-to-finish").into();
        } else {
            // what the cursor showed is what happens: a copy or links the
            // place refuses (the Current Outfit) do nothing
            if allowed || mode == DropMode::Move {
                drop_cargo(inv, cargo.ids(), to, mode, st, prefs, facts, actions);
            }
        }
    }
}

/// What a drop does: a move as before, a copy or links by Paste's routes
/// (the Marketplace's paste, the outfit folders' rules).
#[allow(clippy::too_many_arguments)]
pub(super) fn drop_cargo(
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
            // A folder let go on its own row is a click that slipped past
            // the threshold: refused without a word, as the Legacy Viewer
            // only shows TooltipDragOntoSelf while hovering.
            Err(_) if ids == [to] => {}
            Err(reason) => st.message = reason,
        },
        DropMode::Copy | DropMode::Link => {
            super::clipboard::paste_ids(inv, ids, to, false, mode == DropMode::Link, st, prefs, facts, actions);
        }
    }
}

/// Whether a drop's plan changes anything: an item's copy travels in
/// `copies`, not in `operations`, so a Ctrl-drag of items was refused.
pub(super) fn changes(plan: &Mutation) -> bool {
    !plan.operations.is_empty() || !plan.copies.is_empty()
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
    plan(ctx, inv, ids, to, mode, prefs, facts).as_ref().as_ref().is_ok_and(changes)
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
    // the flag is read before `input`: egui's context is locked inside it,
    // and reading its data there would wait on itself forever
    let right = right_drag(ctx);
    if !(crate::outside::carry::dragging(ctx) || (right && ctx.input(|i| i.pointer.secondary_down()))) {
        return None;
    }
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
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
    // (read outside `input`, which holds egui's context lock)
    let right = right_drag(ctx);
    let held = crate::outside::carry::dragging(ctx) || (right && ctx.input(|i| i.pointer.secondary_down()));
    let Some(pointer) = ctx.input(|i| i.pointer.interact_pos().filter(|_| held)) else {
        return;
    };
    let count = cargo.ids().len();
    // what the drop will do, as Explorer's "+ Copy" / "Create link" badge;
    // a single entry moved says nothing, as before
    // over an inventory folder that takes it, what that folder would do;
    // elsewhere (the world, a conversation) the keys mean what that place says
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
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

#[cfg(test)]
mod fast_drag_tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    #[test]
    fn inventory_drag_delivers_batched_motion_and_release_to_an_earlier_target() {
        let ctx = egui::Context::default();
        let item = Uuid::from_u128(1);
        let mut st = InventoryUi::default();
        let row = egui::Rect::from_min_max(egui::pos2(20.0, 20.0), egui::pos2(200.0, 80.0));
        let target = egui::Rect::from_min_max(egui::pos2(250.0, 20.0), egui::pos2(440.0, 80.0));
        let start = row.center();
        let end = target.center();
        let button = |pos, pressed, modifiers| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        let mut dropped = Vec::new();
        let mut clicks = 0;
        for events in [
            vec![],
            vec![Event::PointerMoved(start), button(start, true, egui::Modifiers::NONE)],
            vec![
                Event::PointerMoved(end),
                button(end, false, egui::Modifiers::CTRL),
                Event::PointerMoved(start),
            ],
            vec![Event::PointerMoved(end + egui::vec2(0.0, 150.0))],
            vec![],
            vec![Event::PointerMoved(start), button(start, true, egui::Modifiers::NONE)],
            vec![
                Event::PointerMoved(start + egui::vec2(1.0, 0.0)),
                button(start, false, egui::Modifiers::NONE),
            ],
            vec![],
        ] {
            ctx.run_ui(
                RawInput {
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    // Conversations can process the release before Inventory
                    // draws its source row during the same UI pass.
                    if ui.rect_contains_pointer(target)
                        && ui.input(|i| i.pointer.primary_released())
                        && let Some(cargo) = Cargo::carried(ui.ctx())
                    {
                        dropped.extend_from_slice(cargo.ids());
                        assert!(ui.input(|i| i.modifiers.ctrl), "the drop keeps its release modifiers");
                        egui::DragAndDrop::clear_payload(ui.ctx());
                    }
                    let response = ui.interact(row, egui::Id::unique("inventory test row"), egui::Sense::click_and_drag());
                    source(ui, &response, item, false, &mut st);
                    clicks += usize::from(response.clicked());
                },
            )
            .textures_delta
            .clear();
        }
        assert_eq!(dropped, [item]);
        assert_eq!(clicks, 1, "a below-threshold release still clicks normally");
    }
}

#[cfg(test)]
mod right_button_tests {
    use super::*;

    /// A right-button drag over the world: the cursor and the badge read
    /// the drag's flag without locking egui's context inside itself (the
    /// viewer froze at the first right-click when they did).
    #[test]
    fn a_right_button_drag_never_locks_egui_twice() {
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_temp(right_drag_id(), true));
        egui::DragAndDrop::set_payload(&ctx, InvDrag(Uuid::from_u128(1)));
        let p = crate::theme::Theme::default().palette();
        let pos = egui::pos2(100.0, 100.0);
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Default::default(),
            }],
        ] {
            // the font atlas a first pass uploads is not drawn here
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    let _ = cursor(&ctx);
                    badge(&ctx, &p);
                },
            )
            .textures_delta
            .clear();
        }
        assert!(ctx.input(|i| i.pointer.secondary_down()));
    }

    /// A right-button drag ended long ago; a later left drag, started
    /// anywhere (not through `source`), is not a right one.
    #[test]
    fn a_left_drag_after_a_right_one_is_a_left_drag() {
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_temp(right_drag_id(), true));
        egui::DragAndDrop::set_payload(&ctx, InvDrag(Uuid::from_u128(1)));
        let pos = egui::pos2(100.0, 100.0);
        let mut right = true;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ui| {
                right = right_drag(ui.ctx());
                // the keys held decide again, not the old right button
                let held = ui.input(|i| i.modifiers);
                assert_eq!(current_mode(ui.ctx()), mode_for(held, cfg!(target_os = "macos")));
            },
        );
        output.textures_delta.clear();
        assert!(!right);
        assert!(ctx.data(|d| d.get_temp::<bool>(right_drag_id())).is_none(), "the flag is forgotten");
    }
}
