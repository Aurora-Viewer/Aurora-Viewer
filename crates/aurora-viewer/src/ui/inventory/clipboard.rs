//! LLFolderView::copy/cut/paste and LLInvFVBridge clipboard rules
//! (Firestorm indra/llui/llfolderview.cpp and newview/llinventorybridge.cpp,
//! originally LGPL 2.1). Menus and keyboard share the same inventory clipboard.

use super::*;

/// LLFolderViewItem::isFadeItem / draw (indra/llui/llfolderviewitem.cpp):
/// a cut folder fades its descendants too, without disabling their controls.
#[derive(Default)]
pub(super) struct CutRows {
    ids: Vec<Uuid>,
    roots: HashSet<Uuid>,
}

pub(super) fn prepare(st: &mut InventoryUi) {
    if st.cut && st.cut_rows.ids != st.clipboard {
        st.cut_rows.ids.clone_from(&st.clipboard);
        st.cut_rows.roots = st.clipboard.iter().copied().collect();
    }
}

pub(super) fn opacity(inv: &Inventory, st: &InventoryUi, id: Uuid) -> f32 {
    // Only visible rows walk their ancestors. Never expand or scan a cut
    // folder's entire contents on the frame thread.
    if st.cut && rules::ancestors(inv, id).any(|id| st.cut_rows.roots.contains(&id)) {
        0.5
    } else {
        1.0
    }
}

impl InventoryUi {
    /// Cancel the shared cut in every inventory window. Copy remains reusable.
    pub fn cancel_cut(&mut self) {
        if self.cut {
            self.clipboard.clear();
            self.cut = false;
            self.consume_clipboard = false;
        }
        for window in &mut self.windows {
            window.state.cancel_cut();
        }
    }

    /// Called before fields consume input, even while Inventory is closed.
    pub fn clipboard_input(&mut self, events: &[egui::Event]) {
        for event in events {
            match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.cancel_cut(),
                egui::Event::Paste(text) => self.clipboard_changed(Some(text)),
                _ => {}
            }
        }
    }

    /// The Windows counter also catches asynchronous image copies and other
    /// applications changing the clipboard without taking keyboard focus.
    pub fn native_clipboard_changed(&mut self) -> bool {
        #[cfg(windows)]
        {
            // SAFETY: this reads the calling desktop's change counter, with
            // no pointers, ownership transfer or clipboard lock required.
            let sequence = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
            sequence != 0 && self.clipboard_sequence.replace(sequence) != Some(sequence)
        }
        #[cfg(not(windows))]
        false
    }

    /// Check only on native changes or focus return; never poll clipboard text.
    pub fn clipboard_changed(&mut self, text: Option<&str>) {
        if self.cut && !holds_marker(self, text.unwrap_or_default()) {
            self.cancel_cut();
        }
    }

    /// A copy from any other viewer field replaces the cut's native marker.
    pub fn clipboard_output(&mut self, commands: &[egui::OutputCommand]) {
        for command in commands.iter().rev() {
            match command {
                egui::OutputCommand::CopyText(text) => {
                    self.clipboard_changed(Some(text));
                    break;
                }
                egui::OutputCommand::CopyImage(_) => {
                    self.cancel_cut();
                    break;
                }
                _ => {}
            }
        }
    }
}

pub(super) fn focus_id(st: &InventoryUi) -> egui::Id {
    egui::Id::unique(("inventory_clipboard_focus", st.ui_id))
}

pub(super) fn request_focus(ui: &egui::Ui, st: &mut InventoryUi) {
    request_focus_ctx(ui.ctx(), st);
}

/// Hands the keyboard back to the list when a modal or a preview over it
/// closes. egui keeps the closed window as the top layer for one more pass,
/// so this request is dropped and blocks [`list_focus`]'s restore; the restore
/// window is moved two passes on (a preview's own text field keeps the focus
/// for one more) so the list takes it once nothing is above it.
pub(super) fn return_focus(ctx: &egui::Context, st: &mut InventoryUi) {
    request_focus_ctx(ctx, st);
    st.list_focus_frame = Some(ctx.cumulative_frame_nr() + 2);
    // run those passes now, not at the next input
    ctx.request_repaint();
}

fn pending_focus_id() -> egui::Id {
    egui::Id::unique("inventory_pending_focus")
}

/// The list to focus, the widget that had it when the document closed, and
/// the frame from which the request may be taken.
type PendingFocus = (egui::Id, Option<egui::Id>, u64);

/// Hands the keyboard back to `focus` (a list's [`focus_id`]) once the window
/// above it is gone. A document lives in a window state of its own, so the
/// request waits in the context for the list that opened it to take it.
pub(super) fn return_focus_to(ctx: &egui::Context, focus: egui::Id) {
    let at = ctx.cumulative_frame_nr() + 2;
    let from = ctx.memory(|m| m.focused());
    ctx.data_mut(|d| d.insert_temp(pending_focus_id(), Some((focus, from, at))));
    ctx.request_repaint();
}

/// The document `st` closed: ask the list that opened it for the keyboard.
pub(super) fn return_document_focus(ctx: &egui::Context, st: &mut InventoryUi) {
    match st.document_owner {
        Some(owner) => return_focus_to(ctx, owner),
        None => return_focus(ctx, st),
    }
}

fn request_focus_ctx(ctx: &egui::Context, st: &mut InventoryUi) {
    st.list_focus_frame = Some(ctx.cumulative_frame_nr());
    ctx.memory_mut(|m| {
        m.request_focus(focus_id(st));
        m.set_focus_lock_filter(
            focus_id(st),
            egui::EventFilter {
                vertical_arrows: true,
                horizontal_arrows: true,
                ..Default::default()
            },
        );
    });
}

pub(super) fn list_has_focus(ui: &egui::Ui, st: &InventoryUi) -> bool {
    ui.is_enabled()
        && st.dialog.is_none()
        && ui.memory(|m| m.has_focus(focus_id(st)) && m.allows_interaction(ui.layer_id()))
        && !ui.ctx().text_edit_focused()
        && !egui::Popup::is_any_open(ui.ctx())
}

/// A stable focus target survives scrolling selected rows out of the viewport.
/// Register the background before the rows so their clicks take precedence.
pub(super) fn list_focus(ui: &mut egui::Ui, st: &mut InventoryUi, height: f32) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    let response = ui.interact(rect, focus_id(st), egui::Sense::click());
    let leaving = ui.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_none_or(|pos| !rect.contains(pos))
            || i.key_pressed(egui::Key::Tab)
            || i.key_pressed(egui::Key::Escape)
    });
    if leaving {
        st.list_focus_frame = None;
    }
    // A document opened from this list closed: take the keyboard back once
    // the window above is gone and no other widget holds it. The window can
    // outlive the close by a pass or two, so the request stays a few passes.
    let now = ui.ctx().cumulative_frame_nr();
    let focused = ui.memory(|m| m.focused());
    if let Some((focus, from, at)) = ui.ctx().data(|d| d.get_temp::<Option<PendingFocus>>(pending_focus_id())).flatten() {
        if focus == focus_id(st) {
            if leaving || now > at + 2 {
                ui.ctx().data_mut(|d| d.remove_temp::<Option<PendingFocus>>(pending_focus_id()));
            } else if now >= at
                && ui.is_enabled()
                && st.dialog.is_none()
                && !egui::Popup::is_any_open(ui.ctx())
                && (focused.is_none() || focused == from)
                && ui.memory(|m| m.allows_interaction(ui.layer_id()))
            {
                ui.ctx().data_mut(|d| d.remove_temp::<Option<PendingFocus>>(pending_focus_id()));
                request_focus(ui, st);
            }
        } else if now > at + 2 {
            ui.ctx().data_mut(|d| d.remove_temp::<Option<PendingFocus>>(pending_focus_id()));
        }
    }
    // A discarded sizing pass can omit the list and trigger egui's focus
    // dead-man switch. Restore only its immediately preceding focus request,
    // with no other focused widget, click outside, Tab, Escape or modal.
    let restore = !leaving
        && ui.is_enabled()
        && st.dialog.is_none()
        && !egui::Popup::is_any_open(ui.ctx())
        && st.list_focus_frame.is_some_and(|frame| ui.ctx().cumulative_frame_nr() <= frame + 1)
        && ui.memory(|m| m.focused().is_none() && m.allows_interaction(ui.layer_id()));
    if response.clicked() || response.has_focus() || restore {
        request_focus(ui, st);
    }
}

pub(super) fn copy_allowed(inv: &Inventory, ids: &[Uuid]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| {
            inv.folders.contains_key(id)
                || inv
                    .items
                    .get(id)
                    .is_some_and(|it| rules::held(inv, it) & rules::COPY != 0 || !matches!(it.asset_type, 2 | 49))
        })
}

pub(super) fn store(ctx: &egui::Context, st: &mut InventoryUi, ids: Vec<Uuid>, cut: bool) {
    if let Some(id) = ids.first() {
        // egui-winit translates Ctrl+V into Paste only for nonempty OS text.
        // Keep the UUID marker used by the context menu; never import UUIDs
        // supplied by external text into the inventory clipboard.
        ctx.copy_text(id.to_string());
    }
    st.clipboard = ids;
    st.cut = cut;
    st.consume_clipboard = false;
}

/// Ctrl+C / Ctrl+X: `ids` go on the clipboard when they may be copied, or
/// all moved for a cut; `false` leaves the clipboard as it was.
pub(crate) fn offer(
    ctx: &egui::Context,
    inv: &Inventory,
    st: &mut InventoryUi,
    ids: Vec<Uuid>,
    cut: bool,
    protected: &HashSet<Uuid>,
) -> bool {
    let allowed = if cut {
        !ids.is_empty() && ids.iter().all(|id| rules::relocatable(inv, *id, protected))
    } else {
        copy_allowed(inv, &ids)
    };
    if allowed {
        store(ctx, st, ids, cut);
    }
    allowed
}

/// Ctrl+V pastes only while the system clipboard still holds the marker
/// `store` left: another application or a text field may have replaced it,
/// and must not paste an old inventory selection.
pub(crate) fn holds_marker(st: &InventoryUi, text: &str) -> bool {
    st.clipboard.first().is_some_and(|id| text.trim() == id.to_string())
}

/// Ctrl+C, Ctrl+X or Ctrl+V in Appearance, on this same clipboard. A paste
/// with an outfit selected takes items into that outfit, and folders beside
/// it, in its group (or into the group, when a group is selected).
pub fn appearance_request(
    ctx: &egui::Context,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    request: crate::ui::appearance::ClipboardRequest,
) -> Vec<InvAction> {
    use crate::ui::appearance::{ClipboardRequest, PasteTo};
    use crate::world::outfit_folders;
    let mut actions = Vec::new();
    match request {
        ClipboardRequest::Store { ids, cut } => {
            if !offer(ctx, inv, st, ids, cut, &prefs.protected) {
                st.message = aurora_i18n::tr!("ui-appearance-clipboard-cannot-take").into();
            }
        }
        ClipboardRequest::Paste { to, text } => {
            if !holds_marker(st, &text) {
                return actions;
            }
            let folders = st.clipboard.iter().all(|id| inv.folders.contains_key(id));
            let to = match to {
                PasteTo::Worn => crate::world::appearance::cof(inv),
                PasteTo::Outfit(Some(id)) if outfit_folders::container(inv, id) => Some(id),
                PasteTo::Outfit(Some(id)) if folders => rules::parent(inv, id).filter(|parent| outfit_folders::container(inv, *parent)),
                PasteTo::Outfit(Some(id)) => Some(id),
                PasteTo::Outfit(None) if folders => outfit_folders::my_outfits(inv),
                PasteTo::Outfit(None) => {
                    st.message = aurora_i18n::tr!("ui-appearance-clipboard-select-an-outfit").into();
                    None
                }
            };
            if let Some(to) = to {
                paste(inv, to, false, st, prefs, facts, &mut actions);
            }
        }
    }
    actions
}

/// Whether Paste (not as link) into `to` would go through: the outfit rules
/// for the outfit folders, the inventory's elsewhere.
pub(super) fn paste_allowed(inv: &Inventory, st: &InventoryUi, to: Uuid, prefs: &InventoryPreferences, facts: &Facts) -> bool {
    match crate::world::outfit_paste::plan(inv, &st.clipboard, to, st.cut, &prefs.protected) {
        Ok(crate::world::outfit_paste::Paste::Inventory) => {
            rules::paste_allowed(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn)
        }
        plan => plan.is_ok(),
    }
}

pub(super) fn paste(
    inv: &Inventory,
    to: Uuid,
    links: bool,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let cut = st.cut && !links;
    if !links && crate::world::marketplace::root(inv).is_some_and(|root| rules::under(inv, to, root)) {
        actions.push(InvAction::MarketplacePaste {
            to,
            ids: st.clipboard.clone(),
            copy: !cut,
        });
        return;
    }
    if !links {
        match crate::world::outfit_paste::plan(inv, &st.clipboard, to, cut, &prefs.protected) {
            Ok(crate::world::outfit_paste::Paste::Inventory) => {}
            Ok(crate::world::outfit_paste::Paste::Edit(change)) => {
                st.consume_clipboard = cut;
                st.message.clear();
                actions.push(InvAction::Edit(change));
                return;
            }
            // dropToOutfit into the Current Outfit wears, and leaves the
            // clipboard as it was.
            Ok(crate::world::outfit_paste::Paste::Wear(items)) => {
                st.message.clear();
                actions.push(InvAction::Appearance(crate::world::appearance::Action::WearItems {
                    items,
                    replace: true,
                    point: 0,
                }));
                return;
            }
            Err(reason) => {
                st.consume_clipboard = false;
                st.message = reason;
                return;
            }
        }
    }
    match rules::paste(inv, &st.clipboard, to, cut, links, &prefs.protected, &facts.worn) {
        Ok(change) => {
            st.consume_clipboard = cut;
            st.message.clear();
            actions.push(InvAction::Edit(change));
        }
        Err(reason) => {
            st.consume_clipboard = false;
            st.message = reason;
        }
    }
}

pub(super) fn shortcuts(
    ui: &mut egui::Ui,
    inv: &Inventory,
    root: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    if !list_has_focus(ui, st) {
        return;
    }
    if !actions
        .iter()
        .any(|action| matches!(action, InvAction::Edit(_) | InvAction::Delete(_)))
    {
        super::delete::shortcut(ui, inv, st, prefs, facts, actions);
    }
    if st.dialog.is_some() {
        return;
    }
    super::keyboard::open_shortcut(ui, inv, st, prefs, facts, actions);
    // egui-winit consumes the physical C/X/V presses and emits these native
    // clipboard events instead. Testing key_pressed(C) would miss real input.
    let mut events = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if matches!(event, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)) {
                events.push(event.clone());
                false
            } else {
                true
            }
        });
    });
    for event in events {
        if st.pending.is_some()
            || actions
                .iter()
                .any(|action| matches!(action, InvAction::Edit(_) | InvAction::Delete(_)))
        {
            continue;
        }
        match event {
            egui::Event::Copy | egui::Event::Cut => {
                let mut ids: Vec<_> = st.selection.iter().copied().collect();
                ids.sort();
                offer(ui.ctx(), inv, st, ids, matches!(event, egui::Event::Cut), &prefs.protected);
            }
            egui::Event::Paste(text) => {
                if !holds_marker(st, &text) {
                    continue;
                }
                let to = st.selected.filter(|id| st.selection.contains(id)).map_or(root, |id| {
                    if inv.folders.contains_key(&id) {
                        id
                    } else {
                        rules::parent(inv, id).unwrap_or(root)
                    }
                });
                if paste_allowed(inv, st, to, prefs, facts) {
                    paste(inv, to, false, st, prefs, facts, actions);
                }
            }
            _ => {}
        }
    }
}

fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY").ok().filter(|mode| {
            mode.starts_with("clipboard")
                || mode.starts_with("aurora-clipboard")
                || mode.starts_with("search-selection")
                || mode.starts_with("keyboard")
                || mode.starts_with("aurora-keyboard")
                || mode.starts_with("enter")
        })
    })
    .as_deref()
    .map(|mode| mode.strip_prefix("aurora-").unwrap_or(mode))
}

pub(super) fn demo_row(response: &egui::Response, st: &InventoryUi, item: Uuid) {
    if demo_mode().is_some() || super::delete::demo_enabled() || crate::demo::texture_library::inventory_drag_enabled() {
        // The row's center can hit an inline action in the explorer. Delete
        // fixtures select on the label side, like the native-input tests.
        let point = if super::delete::demo_enabled() {
            response.rect.left_center() + egui::vec2(response.rect.width().min(60.0) * 0.5, 0.0)
        } else {
            response.rect.center()
        };
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::unique(("clipboard_demo_row", st.ui_id, item)), point));
    }
}

pub(super) fn demo_caret(response: &egui::Response, st: &InventoryUi, folder: Uuid) {
    if demo_mode().is_some() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::unique(("clipboard_demo_caret", st.ui_id, folder)), response.rect.center()));
    }
}

pub(super) fn demo_search(response: &egui::Response, st: &InventoryUi) {
    if demo_mode().is_some_and(|mode| mode.starts_with("search-selection")) {
        response.ctx.data_mut(|data| {
            data.insert_temp(egui::Id::unique(("inventory_demo_search", st.ui_id)), response.rect);
        });
    }
}

/// Exercise actual rows with the same native events egui-winit emits for
/// Ctrl+C/X/V, without activating the OS window or reading OS clipboard text.
pub fn demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode().filter(|mode| *mode != "clipboard") else {
        return Vec::new();
    };
    if mode.starts_with("enter") {
        // Select and reveal the demo object, keep the list's keyboard, then
        // Enter puts it on and a second Enter takes it off once it is worn
        // (LLObjectBridge::openItem, as a double-click does).
        let item = Uuid::from_u128(8100);
        if (640..680).contains(&frame) {
            st.selected = Some(item);
            st.selection = HashSet::from([item]);
            st.reveal = Some(item);
            ctx.memory_mut(|memory| memory.request_focus(focus_id(st)));
        }
        if matches!(frame, 680 | 720) {
            return vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }];
        }
        return Vec::new();
    }
    if mode.starts_with("keyboard") || mode.starts_with("aurora-keyboard") {
        if matches!(frame, 650 | 651)
            && let Some(pos) =
                ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(8100)))))
        {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
        }
        // Left from the object selects its folder, Left again closes it,
        // Right opens it again.
        if mode.ends_with("keyboard-fold") {
            let folder = Uuid::from_u128(8000);
            if matches!(frame, 690 | 720) {
                log::info!(
                    "demo inventory keyboard fold: frame={frame}, selected={:?}, folder_selected={}, folder_open={}",
                    st.selected,
                    st.selected == Some(folder),
                    st.plain.is_open(folder)
                );
            }
            let key = match frame {
                665 | 680 => egui::Key::ArrowLeft,
                700 => egui::Key::ArrowRight,
                _ => return Vec::new(),
            };
            return vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }];
        }
        if matches!(frame, 665 | 680 | 710) {
            log::info!(
                "demo inventory keyboard input: focused={:?}, list_focus={}, text={}, popup={}, window_focus={}",
                ctx.memory(|m| m.focused()),
                ctx.memory(|m| m.has_focus(focus_id(st))),
                ctx.text_edit_focused(),
                egui::Popup::is_any_open(ctx),
                ctx.input(|i| i.focused)
            );
            return vec![
                egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
                egui::Event::Key {
                    key: if frame == 710 { egui::Key::ArrowUp } else { egui::Key::ArrowDown },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
            ];
        }
        if matches!(frame, 690 | 720) {
            log::info!(
                "demo inventory keyboard: frame={frame}, selected_count={}, active={:?}, visible={}",
                st.selection.len(),
                st.selected,
                st.selected.is_some_and(|id| st.visible.contains(&id))
            );
        }
        return Vec::new();
    }
    if mode.starts_with("search-selection") {
        let keyboard = mode == "search-selection-keyboard";
        let pos = match frame {
            650 | 651 => ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(11490))))),
            720 | 721 => ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_demo_search", st.ui_id))))
                .map(|rect| {
                    if keyboard {
                        rect.left_center() + egui::vec2(15.0, 0.0)
                    } else {
                        rect.right_center() - egui::vec2(14.0, 0.0)
                    }
                }),
            _ => None,
        };
        if let Some(pos) = pos {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650 || frame == 720,
                    modifiers: Default::default(),
                },
            ];
        }
        if keyboard && matches!(frame, 730 | 731) {
            return vec![egui::Event::Key {
                key: if frame == 730 { egui::Key::A } else { egui::Key::Backspace },
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: if frame == 730 {
                    egui::Modifiers::COMMAND
                } else {
                    egui::Modifiers::NONE
                },
            }];
        }
        if frame == 800 {
            log::info!(
                "demo inventory search selection: query_empty={}, selected={:?}, visible={}",
                st.search.is_empty(),
                st.selected,
                st.selected.is_some_and(|id| st.visible.contains(&id))
            );
        }
        return Vec::new();
    }
    let window = mode == "clipboard-window";
    if mode == "clipboard-progress" {
        super::progress::demo(frame, st);
        return Vec::new();
    }
    if frame == 640 {
        super::super::widgets::place_floater(ctx, "inventory", egui::pos2(20.0, 56.0), egui::vec2(680.0, 850.0));
        for (index, w) in st.windows.iter().enumerate() {
            super::super::widgets::place_floater(
                ctx,
                &format!("inventory_{}", w.id),
                egui::pos2(730.0, 56.0 + index as f32 * 460.0),
                egui::vec2(420.0, 440.0),
            );
        }
    }
    let source_ui = if window {
        st.windows.first().map(|w| w.id).unwrap_or(st.ui_id)
    } else {
        st.ui_id
    };
    let cut = mode.starts_with("clipboard-cut");
    let folder = mode == "clipboard-folder" || mode.starts_with("clipboard-cut-folder");
    if folder && frame == 785 {
        st.expand_all = Some(true);
        st.reveal = st.selected;
    }
    if !window && matches!(frame, 780 | 781) {
        let to = if folder { st.selected } else { Some(Uuid::from_u128(8001)) };
        if let Some(pos) =
            to.and_then(|to| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_caret", st.ui_id, to)))))
        {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 780,
                    modifiers: Default::default(),
                },
            ];
        }
    }
    let row =
        |ui_id, item| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", ui_id, Uuid::from_u128(item)))));
    let (pos, ctrl) = match frame {
        650 | 651 => (
            row(
                source_ui,
                if folder {
                    8001
                } else if cut {
                    8109
                } else {
                    8100
                },
            ),
            false,
        ),
        665 | 666 if !window && !folder => (row(source_ui, if cut { 8110 } else { 8104 }), true),
        700 | 701 if window => (
            st.windows
                .get(1)
                .and_then(|w| ctx.read_response(focus_id(&w.state)))
                .map(|r| r.rect.center()),
            false,
        ),
        700 | 701 => (row(st.ui_id, if folder { 8300 } else { 8001 }), false),
        _ => (None, false),
    };
    if let Some(pos) = pos {
        let modifiers = egui::Modifiers {
            ctrl,
            command: ctrl,
            ..Default::default()
        };
        return vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(frame, 650 | 665 | 700),
                modifiers,
            },
        ];
    }
    if frame == 680 {
        return vec![if cut { egui::Event::Cut } else { egui::Event::Copy }];
    }
    if frame == 730 && mode.ends_with("-cancel") {
        return vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
    }
    if !mode.ends_with("-hold")
        && (frame == 730 || frame == 760 && mode == "clipboard-copy")
        && let Some(id) = st.clipboard.first()
    {
        return vec![egui::Event::Paste(id.to_string())];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, epaint::Shape};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    struct Harness {
        ctx: egui::Context,
        world: World,
        st: InventoryUi,
        prefs: InventoryPreferences,
        output: egui::FullOutput,
        time: f64,
        classic: bool,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            world.agent_id = id(2);
            world.inventory.root = id(1);
            world.inventory.folders.insert(
                id(1),
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: id(1),
                        type_default: 8,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
            crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
            // Keep the clipboard fixture compact; unrelated outfit links would
            // otherwise fill the viewport before the rows under test.
            let folders = [id(1), id(8000), id(8001), id(8002), id(8003)];
            world.inventory.folders.retain(|id, _| folders.contains(id));
            world.inventory.items.retain(|id, _| (8100..=8113).contains(&id.as_u128()));
            for folder in world.inventory.folders.values_mut() {
                folder.children.retain(|id| folders.contains(id));
                folder.items.retain(|id| (8100..=8113).contains(&id.as_u128()));
            }
            super::super::super::widgets::place_floater(&ctx, "inventory", egui::pos2(900.0, 40.0), egui::vec2(680.0, 950.0));
            let mut harness = Self {
                ctx,
                world,
                st: InventoryUi {
                    expand_all: Some(true),
                    ..Default::default()
                },
                prefs: InventoryPreferences::default(),
                output: Default::default(),
                time: 0.0,
                classic: true,
            };
            harness.st.show_original(&harness.world.inventory, id(8100));
            harness.st.expand_all = Some(true);
            for _ in 0..4 {
                harness.frame(Vec::new());
            }
            harness
        }

        fn frame(&mut self, mut events: Vec<Event>) -> Vec<InvAction> {
            if let Some(modifiers) = events.iter().find_map(|event| match event {
                Event::Key {
                    key: egui::Key::ArrowDown | egui::Key::ArrowUp,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            }) {
                events.insert(0, Event::ModifiersChanged(modifiers));
            }
            self.st.clipboard_input(&events);
            self.time += 0.3;
            let mut actions = Vec::new();
            self.output = headless_output(self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                    events,
                    time: Some(self.time),
                    ..Default::default()
                },
                |ui| {
                    actions = show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &mut self.world,
                        &mut self.st,
                        &mut self.prefs,
                        &mut true,
                        false,
                        &Default::default(),
                        &Default::default(),
                        &Default::default(),
                        &Default::default(),
                        self.classic,
                    );
                },
            ));
            self.st.clipboard_output(&self.output.platform_output.commands);
            actions
        }

        fn row(&self, item: Uuid) -> Pos2 {
            self.row_in(item, "inventory")
        }

        fn row_in(&self, item: Uuid, window: &str) -> Pos2 {
            fn position(shape: &Shape, label: &str) -> Option<Pos2> {
                match shape {
                    Shape::Text(t) if t.galley.text().starts_with(label) => Some(t.pos + egui::vec2(4.0, 4.0)),
                    Shape::Vec(shapes) => shapes.iter().find_map(|shape| position(shape, label)),
                    _ => None,
                }
            }
            let label = if self.world.inventory.folders.contains_key(&item) {
                folder_name(&self.world.inventory, item)
            } else {
                rules::name(&self.world.inventory, item)
            };
            let rect = self
                .ctx
                .memory(|m| m.area_rect(egui::Id::unique(window)))
                .expect("inventory window");
            let state = self
                .st
                .windows
                .iter()
                .find(|w| window == format!("inventory_{}", w.id))
                .map_or(&self.st, |w| &*w.state);
            let list = self.ctx.read_response(focus_id(state)).expect("inventory list").rect;
            let row = self
                .ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", state.ui_id, item))))
                .expect("drawn row");
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| {
                    position(&shape.shape, &label)
                        .filter(|pos| rect.contains(*pos) && list.contains(*pos) && row.contains(*pos) && shape.clip_rect.contains(*pos))
                })
                .unwrap_or_else(|| panic!("missing visible row: {label}"))
        }

        fn click(&mut self, pos: Pos2, modifiers: egui::Modifiers) -> Vec<InvAction> {
            let mut actions = Vec::new();
            for pressed in [true, false] {
                actions.extend(self.frame(vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ]));
            }
            actions
        }

        fn select(&mut self, item: Uuid, ctrl: bool) {
            self.click(
                self.row(item),
                egui::Modifiers {
                    ctrl,
                    command: ctrl,
                    ..Default::default()
                },
            );
        }

        fn paste(&mut self) -> Vec<InvAction> {
            self.frame(vec![Event::Paste(self.st.clipboard[0].to_string())])
        }

        fn text(&self, label: &str) -> egui::Rect {
            fn rect(shape: &Shape, label: &str) -> Option<egui::Rect> {
                match shape {
                    Shape::Text(t) if t.galley.text() == label => Some(t.galley.rect.translate(t.pos.to_vec2())),
                    Shape::Vec(shapes) => shapes.iter().find_map(|shape| rect(shape, label)),
                    _ => None,
                }
            }
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| rect(&shape.shape, label))
                .expect("visible label")
        }

        fn trash_button(&self, ui_id: Uuid) -> egui::Rect {
            self.ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_delete_button", ui_id))))
                .expect("trash button")
        }

        fn row_face(&self, item: Uuid) -> Option<egui::Color32> {
            let row = self
                .ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", self.st.ui_id, item))))
                .expect("drawn row");
            fn face(shape: &Shape, row: egui::Rect) -> Option<egui::Color32> {
                match shape {
                    Shape::Rect(rect)
                        if rect.rect.contains(row.center())
                            && rect.rect.height() <= row.height() + 4.0
                            && rect.rect.width() >= row.width() * 0.8
                            && rect.fill.a() > 0 =>
                    {
                        Some(rect.fill)
                    }
                    Shape::Vec(shapes) => shapes.iter().rev().find_map(|shape| face(shape, row)),
                    _ => None,
                }
            }
            self.output
                .shapes
                .iter()
                .rev()
                .filter(|shape| shape.clip_rect.contains(row.center()))
                .find_map(|shape| face(&shape.shape, row))
        }

        fn add_trash(&mut self) -> Uuid {
            let trash = id(8020);
            let mut folder = self.world.inventory.folders[&id(8001)].clone();
            folder.info.id = trash;
            folder.info.parent = id(1);
            folder.info.name = "Trash".into();
            folder.info.type_default = 14;
            folder.children.clear();
            folder.items.clear();
            self.world.inventory.folders.insert(trash, folder);
            self.world.inventory.folders.get_mut(&id(1)).expect("root").children.push(trash);
            self.world.inventory.generation += 1;
            trash
        }
    }

    fn delete_key(modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn arrow(down: bool, shift: bool) -> Event {
        Event::Key {
            key: if down { egui::Key::ArrowDown } else { egui::Key::ArrowUp },
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE },
        }
    }

    fn enter_key() -> Event {
        Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn enter_on_the_selected_inventory_object_opens_it_like_a_double_click() {
        use crate::world::appearance::Action as Wear;
        let mut h = Harness::new();
        h.select(id(8100), false);
        let actions = h.frame(vec![enter_key()]);
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, replace: true, point: 0 })] if *item == id(8100)),
            "Enter puts the object on, got {} actions",
            actions.len()
        );
    }

    #[test]
    fn inventory_keyboard_extends_shrinks_and_copies_in_every_layout() {
        for (classic, width) in [(true, 680.0), (false, 440.0), (false, 800.0), (false, 1180.0)] {
            let mut h = Harness::new();
            h.classic = classic;
            super::super::super::widgets::place_floater(&h.ctx, "inventory", egui::pos2(10.0, 40.0), egui::vec2(width, 950.0));
            h.st.show_original(&h.world.inventory, id(8100));
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            let order = if !classic && width >= 800.0 {
                &h.st.flat.order
            } else {
                &h.st.plain.order
            };
            let siblings: Vec<_> = order
                .iter()
                .copied()
                .filter(|row| h.world.inventory.items.contains_key(row))
                .collect();
            let [first, second, third, ..] = siblings[..] else {
                panic!("three item rows")
            };
            h.select(first, false);
            assert!(
                h.ctx.memory(|m| m.has_focus(focus_id(&h.st))),
                "the clicked row gives the list focus"
            );
            assert!(!h.ctx.text_edit_focused(), "the list is not a text field");
            assert!(!egui::Popup::is_any_open(&h.ctx), "no popup takes the arrows");
            let before = h.row(first);
            h.frame(vec![arrow(true, true), arrow(true, true)]);
            assert_eq!(h.st.selected, Some(third), "{classic}, {width}");
            assert_eq!(h.st.selection, HashSet::from([first, second, third]));
            assert_eq!(
                h.row(first),
                before,
                "visible rows stay in place when using the arrows ({classic}, {width})"
            );
            h.frame(vec![arrow(false, true)]);
            assert_eq!(h.st.selected, Some(second));
            assert_eq!(h.st.selection, HashSet::from([first, second]));
            h.frame(vec![Event::Copy]);
            assert_eq!(h.st.clipboard.iter().copied().collect::<HashSet<_>>(), h.st.selection);
            h.frame(vec![arrow(false, false)]);
            assert_eq!(h.st.selected, Some(first));
            assert_eq!(h.st.selection, HashSet::from([first]));
            h.select(third, true);
            h.frame(vec![arrow(false, true)]);
            assert_eq!(
                h.st.selection,
                HashSet::from([second, third]),
                "keyboard restarts at the last Ctrl-click"
            );
        }
    }

    #[test]
    fn inventory_keyboard_scrolls_filtered_virtual_rows_and_keeps_text_focus() {
        let mut h = Harness::new();
        let original = h.world.inventory.items[&id(8100)].clone();
        h.world.inventory.add_items(
            (0..90)
                .map(|n| aurora_net::inventory::InvItem {
                    id: id(20_000 + n),
                    name: format!("Keyboard {n:03}"),
                    ..original.clone()
                })
                .collect(),
        );
        h.st.search = "Keyboard".into();
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.select(id(20_000), false);
        h.frame((0..60).map(|_| arrow(true, true)).collect());
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        assert_eq!(h.st.selected, Some(id(20_060)));
        assert_eq!(h.st.selection.len(), 61);
        h.row(id(20_060)); // The active row is actually drawn inside the viewport.
        h.st.begin_rename(&h.world.inventory, id(20_060));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![arrow(false, true)]);
        assert_eq!(h.st.selection, HashSet::from([id(20_060)]));
        h.st.rename = None;
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::unique("other_field")));
        h.frame(vec![arrow(false, true)]);
        assert_eq!(h.st.selected, Some(id(20_060)));
        h.st.show_original(&h.world.inventory, id(20_000));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.select(id(20_000), false);
        h.frame((0..60).map(|_| arrow(true, true)).collect());
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        assert_eq!(h.st.selected, Some(id(20_060)));
        assert_eq!(h.st.selection.len(), 61);
        h.row(id(20_060)); // The unfiltered tree shares the same minimal scroll.
    }

    #[test]
    fn inventory_world_drop_releases_keyboard_without_closing_or_restoring_list_focus() {
        for classic in [false, true] {
            let mut h = Harness::new();
            h.classic = classic;
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(id(8100), false);
            assert!(h.ctx.egui_wants_keyboard_input());
            let selection = h.st.selection.clone();
            h.st.release_world_drag_focus(&h.ctx);
            for _ in 0..4 {
                h.frame(Vec::new());
                assert!(!h.ctx.egui_wants_keyboard_input(), "movement keys belong to the scene");
                assert_eq!(h.st.selection, selection);
            }
            assert!(h.ctx.memory(|m| m.area_rect(egui::Id::unique("inventory"))).is_some());
        }
    }

    #[test]
    fn delete_after_inventory_rez_targets_the_world_until_the_inventory_is_clicked_again() {
        use crate::build::{BuildSettings, BuildTool, FLAGS_CREATE_SELECTED, Mods, Tool};
        use aurora_net::{NetCommand, NetEvent, build::BuildCmd};

        for classic in [false, true] {
            let mut h = Harness::new();
            h.classic = classic;
            h.add_trash();
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(id(8100), false);
            let folder = h.world.inventory.items[&id(8100)].parent;
            assert!(h.ctx.egui_wants_keyboard_input());

            let mut build = BuildTool::default();
            let mut settings = BuildSettings::default();
            build.open_build(Tool::Create);
            h.st.release_world_drag_focus(&h.ctx);
            build.edit_inventory_rez(&h.world);
            let (handle, mut object) = crate::demo::events()
                .into_iter()
                .find_map(|event| match event {
                    NetEvent::ObjectUpdates { handle, objects } => objects
                        .into_iter()
                        .find(|o| o.pcode == aurora_prim::params::LL_PCODE_VOLUME && o.parent_id == 0)
                        .map(|o| (handle, o)),
                    _ => None,
                })
                .expect("demo world object");
            object.owner_id = h.world.agent_id;
            object.update_flags |= FLAGS_CREATE_SELECTED | crate::ui::context::flags::OBJECT_YOU_OWNER;
            let local_id = object.local_id;
            h.world.objects.upsert(handle, object);
            build.update(&mut h.world, &settings, None, (0.0, 0.0), true, Mods::default());

            assert!(h.frame(vec![delete_key(Default::default())]).is_empty());
            assert!(h.st.dialog.is_none());
            crate::build::ui::shortcuts(&h.ctx, &mut build, &mut h.world, &mut settings);
            assert!(build.out.iter().any(|command| matches!(
                command,
                NetCommand::Build(BuildCmd::DeRez { local_ids, destination, .. })
                    if *local_ids == vec![local_id] && *destination == aurora_net::build::derez::TRASH
            )));
            assert_eq!(h.world.inventory.items[&id(8100)].parent, folder);

            let mut release = delete_key(Default::default());
            if let Event::Key { pressed, .. } = &mut release {
                *pressed = false;
            }
            h.frame(vec![release]);
            h.select(id(8100), false);
            assert!(h.frame(vec![delete_key(Default::default())]).is_empty());
            assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if *ids == vec![id(8100)]));
        }
    }

    #[test]
    fn inventory_keyboard_uses_only_the_focused_folder_window() {
        let mut h = Harness::new();
        h.st.open_folder_window(id(8000));
        let window = h.st.windows[0].id;
        super::super::super::widgets::place_floater(
            &h.ctx,
            &format!("inventory_{window}"),
            egui::pos2(20.0, 50.0),
            egui::vec2(600.0, 800.0),
        );
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let order = &h.st.windows[0].state.plain.order;
        let siblings: Vec<_> = order
            .iter()
            .copied()
            .filter(|row| h.world.inventory.items.contains_key(row))
            .collect();
        let first = siblings[0];
        let second = siblings[1];
        let main = h.st.selection.clone();
        h.click(h.row_in(first, &format!("inventory_{window}")), Default::default());
        h.frame(vec![arrow(true, true)]);
        assert_eq!(h.st.windows[0].state.selection, HashSet::from([first, second]));
        assert_eq!(h.st.selection, main);
        h.st.windows[0].state.dialog = Some(EditDialog::Delete(vec![first], false));
        h.frame(vec![arrow(true, true)]);
        assert_eq!(h.st.windows[0].state.selection, HashSet::from([first, second]));
    }

    #[test]
    fn received_delete_key_in_both_layouts_and_classic_trash_button_submit_confirmed_moves() {
        for (classic, button) in [(false, false), (true, false), (true, true)] {
            let mut h = Harness::new();
            h.classic = classic;
            let trash = h.add_trash();
            crate::world::inventory::demo::seed_received(&mut h.world.inventory, h.world.agent_id);
            let received_item = id(8_300_002);
            h.st.show_notified(&h.world.inventory, received_item);
            h.st.expand_all = Some(true);
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(received_item, false);
            let actions = if button {
                h.click(h.trash_button(h.st.ui_id).center(), Default::default())
            } else {
                h.frame(vec![delete_key(Default::default())])
            };
            assert!(actions.is_empty(), "confirmation must precede the move");
            assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if *ids == vec![received_item]));
            h.frame(Vec::new());
            let actions = h.click(h.text(aurora_i18n::tr!("common-delete")).center(), Default::default());
            let [InvAction::Delete(deletion)] = &actions[..] else {
                panic!("confirmed Received deletion must submit one mutation (classic={classic}, button={button})")
            };
            let change = &deletion.change;
            assert!(change.move_only());
            assert_eq!(change.operations.len(), 1);
            rules::demo_mutate(&mut h.world.inventory, h.world.agent_id, change.clone()).expect("delete received item");
            assert_eq!(h.world.inventory.items[&received_item].parent, trash);
            assert_eq!(h.world.inventory.folders[&id(8_300_001)].info.parent, id(8_300_000));
        }
    }

    #[test]
    fn delete_key_confirms_and_moves_selected_folder_with_its_contents_and_no_copy_item() {
        let mut h = Harness::new();
        let trash = h.add_trash();
        let mut child = h.world.inventory.items[&id(8104)].clone();
        child.parent = id(8001);
        h.world.inventory.server_updated(Vec::new(), vec![child]);
        h.st.expand_all = Some(true);
        h.frame(Vec::new());
        h.select(id(8001), false);
        h.select(id(8104), true);
        h.select(id(8109), true);
        assert!(h.frame(vec![delete_key(Default::default())]).is_empty());
        assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if ids.len() == 3));
        h.frame(Vec::new());
        let actions = h.click(h.text(aurora_i18n::tr!("common-delete")).center(), Default::default());
        let [InvAction::Delete(deletion)] = &actions[..] else {
            panic!("confirmed deletion must submit one mutation")
        };
        let change = &deletion.change;
        assert!(change.move_only());
        assert_eq!(change.operations.len(), 2, "a selected descendant moves with its folder once");
        rules::demo_mutate(&mut h.world.inventory, h.world.agent_id, change.clone()).expect("delete");
        assert_eq!(h.world.inventory.folders[&id(8001)].info.parent, trash);
        assert_eq!(h.world.inventory.items[&id(8104)].parent, id(8001));
        assert_eq!(h.world.inventory.items[&id(8109)].parent, trash);
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    fn key(key: egui::Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn closing_the_delete_confirmation_gives_the_list_its_keyboard_back() {
        // Enter, Escape and Cancel all hand the keyboard back to the list, as
        // the Legacy alert restores the focus it saved; a click on the dimmed
        // world behind the modal answers nothing.
        for close in ["enter", "escape", "cancel"] {
            let mut h = Harness::new();
            h.add_trash();
            h.frame(Vec::new());
            h.select(id(8100), false);
            assert!(h.frame(vec![delete_key(Default::default())]).is_empty());
            assert!(h.st.dialog.is_some(), "{close}: Delete opens the confirmation");
            h.frame(Vec::new());
            assert!(h.click(Pos2::new(4.0, 4.0), Default::default()).is_empty());
            assert!(h.st.dialog.is_some(), "{close}: a click outside the modal keeps it open");
            h.frame(Vec::new());
            let mut actions = match close {
                "enter" => h.frame(vec![key(egui::Key::Enter)]),
                "escape" => h.frame(vec![key(egui::Key::Escape)]),
                _ => h.click(h.text(aurora_i18n::tr!("common-cancel")).center(), Default::default()),
            };
            // modal_keys clicks the default button on the pass after Enter
            actions.extend(h.frame(Vec::new()));
            assert_eq!(actions.len(), usize::from(close == "enter"), "{close}");
            assert!(h.st.dialog.is_none(), "{close}: the confirmation closed");
            h.frame(Vec::new());
            h.frame(Vec::new());
            assert!(
                h.ctx.memory(|m| m.has_focus(focus_id(&h.st))),
                "{close}: the list has the keyboard again"
            );
            if close != "enter" {
                // released first, or egui reports the second press as a repeat
                let release = Event::Key {
                    key: egui::Key::Delete,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                };
                h.frame(vec![release, delete_key(Default::default())]);
                assert!(h.st.dialog.is_some(), "{close}: Delete reaches the list without a click");
            }
        }
    }

    #[test]
    fn closing_a_notecard_gives_the_list_its_keyboard_back() {
        let mut h = Harness::new();
        h.st.preview = Some(id(8103));
        for _ in 0..3 {
            h.frame(Vec::new());
        }
        assert!(h.st.preview.is_some(), "the notecard opened");
        let window = super::super::document::preview_window_id(h.st.ui_id, id(8103), 7);
        crate::ui::widgets::close_floater(&h.ctx, &window);
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        assert!(h.st.preview.is_none(), "the notecard closed");
        assert!(
            h.ctx.memory(|m| m.has_focus(focus_id(&h.st))),
            "the list has the keyboard again after the document closes"
        );
    }

    #[test]
    fn a_document_window_returns_the_keyboard_to_the_list_that_opened_it() {
        let mut h = Harness::new();
        let item = id(8103);
        let owner = focus_id(&h.st);
        h.st.open_document(&h.ctx, item, 7);
        for _ in 0..3 {
            h.frame(Vec::new());
        }
        let at =
            h.st.windows
                .iter()
                .position(|w| w.state.preview == Some(item))
                .expect("the document window");
        assert_eq!(h.st.windows[at].state.document_owner, Some(owner));
        let window = super::super::document::preview_window_id(h.st.windows[at].state.ui_id, item, 7);
        crate::ui::widgets::close_floater(&h.ctx, &window);
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        assert!(
            h.st.windows.iter().all(|w| w.state.preview != Some(item)),
            "the document window closed"
        );
        assert!(
            h.ctx.memory(|m| m.has_focus(owner)),
            "the list that opened the document has the keyboard"
        );
    }

    #[test]
    fn trash_button_is_right_of_loaded_count_and_opens_same_confirmation() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        let button = h.trash_button(h.st.ui_id);
        let count = h.text(&aurora_i18n::tr!(
            "ui-inventory-objects-loaded",
            value0 = aurora_i18n::integer(h.world.inventory.total_count() as i64)
        ));
        assert!(button.left() > count.right());
        assert!(button.left() - count.right() < 18.0, "the count sits beside the trash button");
        assert!((button.center().y - count.center().y).abs() < 3.0);
        assert!(h.click(button.center(), Default::default()).is_empty());
        assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if *ids == vec![id(8100)]));
        h.frame(Vec::new());
        assert!(
            h.click(h.text(aurora_i18n::tr!("common-cancel")).center(), Default::default())
                .is_empty()
        );
        assert!(h.st.dialog.is_none());
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    #[test]
    fn inventory_count_hover_shows_live_items_and_folders_in_both_layouts() {
        fn collect(shape: &Shape, texts: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
                _ => {}
            }
        }
        for (classic, width) in [(true, 680.0), (false, 680.0), (false, 1180.0)] {
            let mut h = Harness::new();
            h.classic = classic;
            super::super::super::widgets::size_floater(&h.ctx, "inventory", egui::vec2(width, 800.0));
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            assert_eq!(h.world.inventory.item_count(), 14);
            assert_eq!(h.world.inventory.folders.len(), 5);
            for folders in [5, 4] {
                if folders == 4 {
                    h.world.inventory.remove(&[id(8001)]);
                }
                h.frame(Vec::new());
                let total = aurora_i18n::integer(14 + folders);
                let label = if classic {
                    aurora_i18n::tr!("ui-inventory-objects-loaded", value0 = total)
                } else {
                    aurora::item_count(&h.world.inventory, aurora::Layout::of_window(&h.ctx, "inventory"))
                };
                let position = h.text(&label).center();
                h.frame(vec![Event::PointerMoved(position)]);
                for _ in 0..8 {
                    h.frame(Vec::new());
                }
                let details = aurora_i18n::tr!(
                    "inventory-count-tooltip",
                    items = aurora_i18n::integer(14),
                    folders = aurora_i18n::integer(folders)
                );
                let tooltip = if classic { format!("{label}\n{details}") } else { details };
                let mut texts = Vec::new();
                for shape in &h.output.shapes {
                    collect(&shape.shape, &mut texts);
                }
                assert!(
                    texts.contains(&tooltip),
                    "classic={classic}, width={width}, folders={folders}, expected={tooltip:?}, painted={texts:?}"
                );
            }
        }
    }

    #[test]
    fn delete_keeps_text_input_priority_and_blocks_modified_keys_pending_and_protected_selection() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![delete_key(egui::Modifiers::CTRL)]);
        assert!(h.st.dialog.is_none());
        for item in [id(1), id(8003)] {
            h.select(item, false);
            h.frame(vec![delete_key(Default::default())]);
            h.click(h.trash_button(h.st.ui_id).center(), Default::default());
            assert!(h.st.dialog.is_none(), "system and library folders are protected");
        }
        h.st.selection.clear();
        h.frame(Vec::new());
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.select(id(8100), false);
        h.st.pending = Some(Uuid::new_v4());
        h.frame(vec![delete_key(Default::default())]);
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.st.pending = None;
        h.prefs.protected.insert(id(8000));
        h.select(id(8000), false); // the locked folder itself; its contents stay ordinary
        h.frame(vec![delete_key(Default::default())]);
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.prefs.protected.clear();
        let mut worn = Facts::from_world(&h.world, false);
        worn.worn.insert(id(8100));
        assert!(!super::super::delete::allowed(&h.world.inventory, &[id(8100)], &h.prefs, &worn));
        assert!(!super::super::delete::allowed(&h.world.inventory, &[id(8000)], &h.prefs, &worn));
        h.select(id(8100), false);
        h.st.begin_rename(&h.world.inventory, id(8100));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![delete_key(Default::default())]);
        assert!(h.st.dialog.is_none());
        assert_eq!(
            h.st.rename.as_ref().expect("rename").name,
            "",
            "Delete edits the selected name text"
        );
    }

    #[test]
    fn folder_window_delete_uses_its_own_selection_and_trash_items_request_purge() {
        let mut h = Harness::new();
        let trash = h.add_trash();
        let mut item = h.world.inventory.items[&id(8100)].clone();
        item.parent = trash;
        h.world.inventory.server_updated(Vec::new(), vec![item]);
        h.st.open_folder_window(trash);
        let window = h.st.windows[0].id;
        super::super::super::widgets::place_floater(
            &h.ctx,
            &format!("inventory_{window}"),
            egui::pos2(30.0, 50.0),
            egui::vec2(420.0, 440.0),
        );
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.click(h.row_in(id(8100), &format!("inventory_{window}")), Default::default());
        h.frame(vec![delete_key(Default::default())]);
        assert!(h.st.dialog.is_none());
        assert!(matches!(&h.st.windows[0].state.dialog, Some(EditDialog::Delete(ids, true)) if *ids == vec![id(8100)]));
        h.frame(Vec::new());
        let actions = h.click(h.text(aurora_i18n::tr!("ui-inventory-dialogs-purge")).center(), Default::default());
        assert!(
            matches!(&actions[..], [InvAction::Delete(deletion)] if deletion.change.removed == vec![id(8100)] && matches!(&deletion.change.operations[..], [aurora_net::inventory::operations::Operation::Delete { id: item, folder: false }] if *item == id(8100)))
        );
    }

    fn escape() -> Event {
        Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    impl Harness {
        fn row_alpha(&self, item: Uuid) -> u8 {
            let label = if self.world.inventory.folders.contains_key(&item) {
                folder_name(&self.world.inventory, item)
            } else {
                rules::name(&self.world.inventory, item)
            };
            fn find(shape: &Shape, label: &str) -> Option<u8> {
                match shape {
                    Shape::Text(text) if text.galley.text().starts_with(label) => {
                        let alpha = text.override_text_color.map(|color| color.a()).unwrap_or_else(|| {
                            text.galley
                                .rows
                                .iter()
                                .flat_map(|row| &row.visuals.mesh.vertices)
                                .map(|vertex| {
                                    if vertex.color == egui::Color32::PLACEHOLDER {
                                        text.fallback_color.a()
                                    } else {
                                        vertex.color.a()
                                    }
                                })
                                .max()
                                .unwrap_or_default()
                        });
                        Some((f32::from(alpha) * text.opacity_factor).round() as u8)
                    }
                    Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                    _ => None,
                }
            }
            self.output
                .shapes
                .iter()
                .find_map(|shape| find(&shape.shape, &label))
                .expect("painted row")
        }
    }

    #[test]
    fn cut_feedback_fades_rows_until_cancelled_in_every_inventory_layout() {
        for (classic, width) in [(true, 680.0), (false, 460.0), (false, 740.0), (false, 1100.0)] {
            let mut h = Harness::new();
            h.classic = classic;
            super::super::super::widgets::place_floater(&h.ctx, "inventory", egui::pos2(400.0, 40.0), egui::vec2(width, 950.0));
            // Switching from the tree to the explorer must reveal the source
            // folder before sending its row a native shortcut.
            h.st.show_original(&h.world.inventory, id(8109));
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(id(8109), false);
            h.frame(vec![Event::Cut]);
            h.frame(Vec::new());
            assert!((127..=128).contains(&h.row_alpha(id(8109))), "{classic}, {width}");
            assert_eq!(h.row_alpha(id(8100)), 255);
            // Changing the destination selection must not cancel the source.
            h.select(id(8001), false);
            assert!((127..=128).contains(&h.row_alpha(id(8109))));
            h.frame(vec![Event::Copy]);
            h.frame(Vec::new());
            assert_eq!(h.row_alpha(id(8109)), 255, "a new inventory copy replaces the cut");
            h.select(id(8109), false);
            h.frame(vec![Event::Cut]);
            h.frame(vec![escape()]);
            assert!(!h.st.cut);
            assert!(h.st.clipboard.is_empty());
            assert_eq!(h.row_alpha(id(8109)), 255);
            h.select(id(8109), false);
            h.frame(vec![Event::Copy]);
            h.frame(Vec::new());
            assert_eq!(h.row_alpha(id(8109)), 255, "copy never fades rows");
            let copied = h.st.clipboard.clone();
            h.frame(vec![escape()]);
            assert_eq!(h.st.clipboard, copied, "Escape preserves an ordinary copy");
        }
    }

    #[test]
    fn cut_folder_feedback_reaches_descendants_and_survives_refused_paste() {
        for classic in [true, false] {
            let mut h = Harness::new();
            h.classic = classic;
            super::super::super::widgets::place_floater(&h.ctx, "inventory", egui::pos2(400.0, 40.0), egui::vec2(460.0, 950.0));
            let mut child = h.world.inventory.items[&id(8104)].clone();
            child.id = id(8200);
            child.parent = id(8001);
            child.name = "Cut folder child".into();
            h.world.inventory.add_items(vec![child]);
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(id(8001), false);
            h.frame(vec![Event::Cut]);
            h.frame(Vec::new());
            assert!((127..=128).contains(&h.row_alpha(id(8001))));
            assert!((127..=128).contains(&h.row_alpha(id(8200))));
            assert_eq!(h.row_alpha(id(8104)), 255);
            // A folder cannot be pasted into itself: keep the pending cut.
            assert!(h.paste().is_empty());
            h.frame(Vec::new());
            assert!(h.st.cut);
            assert!((127..=128).contains(&h.row_alpha(id(8200))));
            h.st.clipboard_output(&[egui::OutputCommand::CopyText("a different field".into())]);
            h.frame(Vec::new());
            assert_eq!(h.row_alpha(id(8200)), 255);
        }
    }

    #[test]
    fn cut_feedback_tracks_native_clipboard_replacement_and_shared_windows() {
        let mut h = Harness::new();
        h.st.open_folder_window(id(8000));
        let window = h.st.windows[0].id;
        super::super::super::widgets::place_floater(
            &h.ctx,
            &format!("inventory_{window}"),
            egui::pos2(20.0, 50.0),
            egui::vec2(600.0, 800.0),
        );
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.select(id(8109), false);
        h.frame(vec![Event::Cut]);
        h.frame(Vec::new());
        assert!(h.st.windows[0].state.cut);
        let marker = h.st.clipboard[0].to_string();
        h.st.clipboard_changed(Some(&marker));
        assert!(h.st.cut, "returning to the viewer keeps its own marker");
        h.st.clipboard_changed(None);
        h.frame(Vec::new());
        assert!(!h.st.cut);
        assert!(!h.st.windows[0].state.cut);
        assert_eq!(h.row_alpha(id(8109)), 255);
        h.st.windows.clear();
        h.select(id(8109), false);
        h.frame(vec![Event::Cut]);
        assert!(h.frame(vec![Event::Paste("outside text".into())]).is_empty());
        assert!(!h.st.cut);
        h.select(id(8109), false);
        h.frame(vec![Event::Cut]);
        h.select(id(8001), false);
        assert!(matches!(&h.paste()[..], [InvAction::Edit(change)] if change.move_only()));
        assert!(h.st.consume_clipboard);
        // The successful mutation path consumes the cut in all windows.
        h.st.cancel_cut();
        h.frame(Vec::new());
        assert_eq!(h.row_alpha(id(8109)), 255);
    }

    #[test]
    fn native_copy_events_copy_every_selected_item_and_allow_repeated_paste() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.select(id(8104), true);
        assert_eq!(h.st.selection.len(), 2);
        assert!(h.frame(vec![Event::Copy]).is_empty());
        assert_eq!(h.st.clipboard, vec![id(8100), id(8104)]);
        assert!(!h.st.cut);
        assert!(
            h.output
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if *text == id(8100).to_string()))
        );
        h.select(id(8001), false);
        for _ in 0..2 {
            let actions = h.paste();
            assert!(
                matches!(&actions[..], [InvAction::Edit(change)] if change.copies.len() == 2 && change.copies.iter().all(|copy| copy.parent == id(8001)))
            );
            assert!(!h.st.consume_clipboard);
        }
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    /// The inventory refused every paste under My Outfits. It now follows
    /// the outfit rules: an outfit links what is pasted, the Current Outfit
    /// wears it and leaves the clipboard as it was.
    #[test]
    fn paste_into_an_outfit_links_and_into_the_current_outfit_wears() {
        use aurora_net::inventory::operations::Operation;
        let mut inv = Inventory::default();
        crate::world::appearance::seed_demo(&mut inv, id(1));
        let (outfit, cof, spare_shirt) = (id(703), id(701), id(722));
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let mut st = InventoryUi {
            clipboard: vec![spare_shirt],
            ..Default::default()
        };
        assert!(paste_allowed(&inv, &st, outfit, &prefs, &facts));
        let mut actions = Vec::new();
        paste(&inv, outfit, false, &mut st, &prefs, &facts, &mut actions);
        assert!(
            matches!(&actions[..], [InvAction::Edit(change)]
                if matches!(&change.operations[..], [Operation::Links { parent, links }] if *parent == outfit && links[0].target == spare_shirt)),
            "an outfit links what is pasted"
        );
        st.cut = true;
        actions.clear();
        assert!(paste_allowed(&inv, &st, cof, &prefs, &facts));
        paste(&inv, cof, false, &mut st, &prefs, &facts, &mut actions);
        assert!(
            matches!(&actions[..], [InvAction::Appearance(crate::world::appearance::Action::WearItems { items, replace: true, point: 0 })]
                if *items == vec![spare_shirt]),
            "the Current Outfit wears what is pasted"
        );
        assert!(!st.consume_clipboard, "wearing keeps what was cut");
        // Worn already: nothing to paste, the menu entry stays off.
        st.clipboard = vec![id(710)];
        assert!(!paste_allowed(&inv, &st, cof, &prefs, &facts));
    }

    #[test]
    fn native_cut_moves_no_copy_and_no_transfer_items_without_removing_them_early() {
        let mut h = Harness::new();
        h.select(id(8109), false);
        h.select(id(8110), true);
        h.frame(vec![Event::Cut]);
        assert!(h.st.cut);
        assert_eq!(h.world.inventory.items[&id(8109)].parent, id(8000));
        h.select(id(8001), false);
        let actions = h.paste();
        assert!(h.st.consume_clipboard);
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("move mutation")
        };
        assert!(change.copies.is_empty());
        assert_eq!(change.operations.len(), 2);
        for op in &change.operations {
            assert!(
                matches!(op, aurora_net::inventory::operations::Operation::Patch { id: item, folder: false, body } if [id(8109), id(8110)].contains(item) && body["parent_id"].as_uuid() == id(8001))
            );
        }
    }

    #[test]
    fn folder_copy_includes_descendants_once_and_refuses_cycles() {
        let mut h = Harness::new();
        let mut item = h.world.inventory.items[&id(8104)].clone();
        item.id = id(8200);
        item.parent = id(8001);
        item.name = "Nested texture".into();
        h.world.inventory.add_items(vec![item]);
        h.frame(Vec::new());
        h.select(id(8001), false);
        h.select(id(8200), true);
        h.frame(vec![Event::Copy]);
        let actions = h.paste();
        assert!(actions.is_empty(), "cannot copy a folder into itself");
        h.select(id(8100), false); // Paste into this item's parent.
        let actions = h.paste();
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("folder copy")
        };
        assert_eq!(change.operations.len(), 1);
        assert_eq!(change.copies.len(), 1);
        assert_eq!(change.copies[0].item, id(8200));
        assert_eq!(h.world.inventory.items[&id(8200)].parent, id(8001));
    }

    #[test]
    fn keyboard_obeys_permissions_pending_mutations_and_external_clipboard_changes() {
        let mut h = Harness::new();
        h.select(id(8109), false);
        h.frame(vec![Event::Copy]); // Still available for Paste as link.
        h.select(id(8001), false);
        assert!(h.paste().is_empty(), "no-copy item cannot be duplicated");
        h.select(id(8000), false); // the locked folder itself
        h.prefs.protected.insert(id(8000));
        h.frame(vec![Event::Cut]);
        assert!(!h.st.cut);
        assert_eq!(h.st.clipboard, vec![id(8109)]);
        h.prefs.protected.clear();
        h.select(id(8100), false);
        h.st.pending = Some(id(99));
        h.frame(vec![Event::Cut]);
        assert_eq!(h.st.clipboard, vec![id(8109)]);
        assert!(!h.st.cut);
        h.st.pending = None;
        h.frame(vec![Event::Copy]);
        h.select(id(8001), false);
        assert!(h.frame(vec![Event::Paste("external text".into())]).is_empty());
        assert!(h.frame(vec![Event::Paste(id(8104).to_string())]).is_empty());
        assert_eq!(h.st.clipboard, vec![id(8100)]);
    }

    #[test]
    fn real_shortcuts_can_copy_again_and_cut_a_no_copy_item_while_a_copy_is_arriving() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![Event::Copy]);
        h.select(id(8001), false);
        let actions = h.paste();
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("first copy")
        };
        h.st.begin_mutation(id(99), change);
        h.st.watch_copy(
            id(99),
            &aurora_net::inventory::operations::Reply {
                contents: Vec::new(),
                folders: Vec::new(),
                moves_queued: false,
                refresh: change.refresh.clone(),
                copies: change.copies.clone(),
                removed: Vec::new(),
                created: Default::default(),
            },
        );
        assert!(h.st.busy(), "copy remains visibly in progress");
        assert!(
            matches!(&h.paste()[..], [InvAction::Edit(change)] if change.copies.len() == 1),
            "Ctrl+V remains available"
        );
        h.select(id(8109), false);
        h.frame(vec![Event::Cut]);
        h.select(id(8001), false);
        assert!(
            matches!(&h.paste()[..], [InvAction::Edit(change)] if change.move_only()),
            "Ctrl+X/V can move a no-copy item during another copy"
        );
        assert_eq!(
            h.world.inventory.items[&id(8100)].parent,
            id(8000),
            "copy never consumes the original"
        );
    }

    #[test]
    fn text_fields_keep_copy_cut_and_paste_and_return_control_to_the_list_on_click() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![Event::Copy]);
        h.st.begin_rename(&h.world.inventory, id(8100));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![Event::Copy]);
        assert_eq!(h.st.clipboard, vec![id(8100)]);
        assert!(
            h.output
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if *text == rules::name(&h.world.inventory, id(8100))))
        );
        h.frame(vec![Event::Cut]);
        assert!(!h.st.cut);
        h.frame(vec![Event::Paste("New name".into())]);
        assert_eq!(h.st.rename.as_ref().expect("rename").name, "New name");
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        h.frame(Vec::new());
        h.select(id(8100), false);
        h.frame(vec![Event::Cut]);
        assert!(h.st.cut);
    }

    #[test]
    fn folder_windows_share_clipboard_and_only_the_focused_list_handles_paste() {
        let mut h = Harness::new();
        h.st.open_folder_window(id(8000));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let window = h.st.windows[0].id;
        h.ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::unique(format!("inventory_{window}")).with("place_pos"),
                egui::pos2(30.0, 50.0),
            )
        });
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.click(h.row_in(id(8100), &format!("inventory_{window}")), Default::default());
        assert_eq!(h.st.windows[0].state.selected, Some(id(8100)));
        h.frame(vec![Event::Copy]);
        assert_eq!(h.st.clipboard, vec![id(8100)]);
        h.st.open_folder_window(id(8001));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let dest = &h.st.windows[1].state;
        let root = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(format!("inventory_{}", dest.ui_id))))
            .expect("destination window");
        h.click(root.center(), Default::default());
        assert!(h.ctx.memory(|m| m.has_focus(focus_id(&h.st.windows[1].state))));
        let actions = h.paste();
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if change.copies.len() == 1 && change.copies[0].parent == id(8001)));
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::unique("unrelated_widget")));
        assert!(h.paste().is_empty());
    }

    /// Real Delete / Enter input, selection paint, scrolling and another Delete
    /// in all four layouts, including the explorer's temporarily swapped tree.
    #[test]
    fn deletion_result_preserves_or_advances_the_visible_selection_in_every_layout() {
        let mut failures = Vec::new();
        for (classic, width) in [(true, 680.0), (false, 440.0), (false, 800.0), (false, 1180.0)] {
            for confirm in [true, false] {
                for filtered in [false, true] {
                    for outcome in ["success", "failure", "queued", "changed selection"] {
                        let case = format!("classic={classic}, width={width}, confirm={confirm}, filtered={filtered}, {outcome}");
                        // Report every layout in a single regression run.
                        let result = std::panic::catch_unwind(|| {
                            let mut h = Harness::new();
                            h.classic = classic;
                            h.ctx.global_style_mut(|style| style.animation_time = 0.0);
                            h.add_trash();
                            h.prefs.confirm_delete = confirm;
                            super::super::super::widgets::place_floater(
                                &h.ctx,
                                "inventory",
                                egui::pos2(10.0, 40.0),
                                egui::vec2(width, 950.0),
                            );
                            h.st.show_original(&h.world.inventory, id(8100));
                            if filtered {
                                // Content names keep this search independent of the UI language.
                                let source = h.world.inventory.items[&id(8100)].clone();
                                h.world.inventory.add_items(
                                    (0..3)
                                        .map(|n| aurora_net::inventory::InvItem {
                                            id: id(21_000 + n),
                                            name: format!("Delete regression {n}"),
                                            ..source.clone()
                                        })
                                        .collect(),
                                );
                                h.st.search = "Delete regression".into();
                                h.st.expand_all = Some(true);
                                h.st.reveal = None;
                                h.st.reveal_path.clear();
                            }
                            for _ in 0..4 {
                                h.frame(Vec::new());
                            }
                            let order = keyboard::shown_order(&h.st);
                            assert_eq!(
                                h.st.shown_order,
                                if filtered {
                                    keyboard::ShownOrder::Results
                                } else if h.st.aurora.is_some_and(|frame| frame.layout != aurora::Layout::Compact) {
                                    keyboard::ShownOrder::Flat
                                } else {
                                    keyboard::ShownOrder::Plain
                                }
                            );
                            let index = order
                                .iter()
                                .position(|row| h.world.inventory.items.contains_key(row))
                                .unwrap_or_else(|| panic!("no item row: {case}; order={order:?}"));
                            let (target, expected, other) = (order[index], order[index + 1], order[index + 2]);
                            h.select(target, false);
                            h.frame(Vec::new());
                            let selected_face = h.row_face(target).expect("selected row has a painted highlight");
                            assert_ne!(h.row_face(expected), Some(selected_face), "the successor starts unselected");
                            let mut actions = h.frame(vec![delete_key(Default::default())]);
                            if confirm {
                                assert!(actions.is_empty() && h.st.dialog.is_some());
                                h.frame(Vec::new());
                                actions = h.frame(vec![key(egui::Key::Enter)]);
                                actions.extend(h.frame(Vec::new()));
                            } else {
                                assert!(h.st.dialog.is_none(), "disabled confirmation submits directly");
                            }
                            let [InvAction::Delete(deletion)] = &actions[..] else {
                                panic!("confirmed deletion must submit one mutation ({classic}, {width})")
                            };
                            assert_eq!(h.st.selection, HashSet::from([target]), "submission must not speculate");
                            let request = id(90_000);
                            h.st.begin_mutation(request, &deletion.change);
                            h.st.begin_delete(request, deletion.selection.clone());
                            if outcome == "changed selection" {
                                // Let the progress footer settle before locating
                                // the row the human can now click.
                                h.frame(Vec::new());
                                h.frame(Vec::new());
                                h.select(other, false);
                                assert_eq!(h.st.selected, Some(other), "the new click must reach its intended row");
                            }
                            let success = outcome != "failure";
                            h.st.finish_delete(request, success, outcome == "queued");
                            h.st.finish_mutation(success);
                            if outcome == "queued" {
                                h.frame(Vec::new());
                                assert_eq!(h.st.selected, Some(target), "queued is not an observed move");
                            }
                            if success {
                                rules::demo_mutate(&mut h.world.inventory, h.world.agent_id, deletion.change.clone()).expect("delete");
                            }
                            for _ in 0..3 {
                                h.frame(Vec::new());
                            }
                            let expected = match outcome {
                                "failure" => target,
                                "changed selection" => other,
                                _ => expected,
                            };
                            assert_eq!(h.st.selection, HashSet::from([expected]), "{case}");
                            assert_eq!((h.st.selected, h.st.anchor), (Some(expected), Some(expected)), "{case}");
                            h.row(expected); // text, row and clipping must all be inside this list's viewport
                            assert_eq!(
                                h.row_face(expected),
                                Some(selected_face),
                                "the surviving row is visibly highlighted: {case}"
                            );
                            assert!(h.ctx.memory(|m| m.has_focus(focus_id(&h.st))), "list focus: {case}");
                            let mut release = delete_key(Default::default());
                            if let Event::Key { pressed, .. } = &mut release {
                                *pressed = false;
                            }
                            h.frame(vec![release]);
                            let actions = h.frame(vec![delete_key(Default::default())]);
                            if confirm {
                                assert!(
                                    actions.is_empty()
                                        && matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if *ids == vec![expected]),
                                    "second Delete: {case}"
                                );
                            } else {
                                assert!(matches!(&actions[..], [InvAction::Delete(_)]), "second Delete: {case}");
                            }
                        });
                        if result.is_err() {
                            failures.push(case);
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "failed inventory layouts: {failures:#?}");
    }
}
