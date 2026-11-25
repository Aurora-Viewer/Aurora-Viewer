//! The inventory's right-click menu in the texture and material picker.
//! LLFloaterTexturePicker lists the inventory in an ordinary inventory panel
//! (the `asset_filtered_inv_panel` of floater_texture_ctrl.xml, an
//! LLInventoryPanel), so its rows have the bridges' usual menus
//! (LLInvFVBridge::buildContextMenu, indra/newview/llinventorybridge.cpp),
//! Library rows the Library's. One picker shows at a time, as the Legacy
//! Viewer has one floater: every caller's picker shares this one menu.
//!
//! Hosted like the Marketplace listings panel (`listings`): its own
//! selection and rename, the inventory windows draw its dialogs (Properties,
//! Delete, Share, Move to, Image…), and the inventory clipboard is shared.
//! A picker is drawn by its caller, before or after the inventory's pass,
//! inside its own window (outside the viewer's included), so the menu is
//! drawn there, on the row. `show` takes the host back into its windows at
//! the start of its pass ([`receive`]) and lends it at the end ([`lend`]):
//! what the picker's menu did meanwhile reaches the inventory then.

use super::*;

/// The hosted panel's id, its `ui_id` and window id.
pub(crate) const PICKER: Uuid = Uuid::from_u128(0x7e3c_51a0_4d92_4b6e_9f08_3a1d_c0de_4896);

/// The preferences a picker's menu may change, written back when it did
/// (the protected folders, the upload folders, the menus' layout, the
/// explorer's pins).
#[derive(Default)]
struct Changed {
    protected: bool,
    upload_folders: bool,
    context_menus: bool,
    pinned: bool,
}

struct Host {
    window: InventoryWindow,
    /// The inventory's preferences when it lent the host; the defaults
    /// (and nothing written back) before it ever did.
    prefs: InventoryPreferences,
    lent: bool,
    changed: Changed,
    appearance_busy: bool,
    /// What the menu asked for since the inventory last took the host.
    actions: Vec<InvAction>,
    /// The clipboard when lent: a picker's Copy or Cut changes it.
    clipboard: (Vec<Uuid>, bool),
    /// A picker drew its list since the host was lent.
    drawn: bool,
    /// The picker opening (its session id) last drawn: a new
    /// one starts without the last one's message or rename.
    session: Option<u64>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            window: InventoryWindow {
                id: PICKER,
                root: Uuid::nil(),
                additional: false,
                state: Box::new(InventoryUi {
                    ui_id: PICKER,
                    ..Default::default()
                }),
                open: false,
            },
            prefs: InventoryPreferences::default(),
            lent: false,
            changed: Changed::default(),
            appearance_busy: false,
            actions: Vec::new(),
            clipboard: (Vec::new(), false),
            drawn: false,
            session: None,
        }
    }
}

/// Where the host waits between the inventory's passes: the egui context's
/// data, which every viewport's pass reads (a picker outside the viewer's
/// window included).
#[derive(Clone, Default)]
struct Slot(std::sync::Arc<std::sync::Mutex<Option<Box<Host>>>>);

fn slot(ctx: &egui::Context) -> Slot {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Slot>(egui::Id::unique("inventory_picker_menu")).clone())
}

fn take(ctx: &egui::Context) -> Option<Box<Host>> {
    slot(ctx).0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

fn put(ctx: &egui::Context, host: Box<Host>) {
    *slot(ctx).0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(host);
}

/// The start of the inventory's pass: what the picker's menu did since the
/// host was lent goes to the main window (the clipboard, the preferences,
/// the actions), and the host joins the windows, whose dialogs it uses.
pub(super) fn receive(
    ctx: &egui::Context,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    let Some(mut host) = take(ctx) else {
        return;
    };
    if host.window.state.ui_id != PICKER {
        // already among the windows (a pass that did not reach `lend`)
        put(ctx, host);
        return;
    }
    let state = &mut host.window.state;
    if (&state.clipboard, state.cut) != (&host.clipboard.0, host.clipboard.1) {
        st.clipboard.clone_from(&state.clipboard);
        st.cut = state.cut;
    }
    st.consume_clipboard |= std::mem::take(&mut state.consume_clipboard);
    if host.lent {
        let changed = std::mem::take(&mut host.changed);
        if changed.protected {
            prefs.protected.clone_from(&host.prefs.protected);
        }
        if changed.upload_folders {
            prefs.upload_folders = host.prefs.upload_folders;
        }
        if changed.context_menus {
            prefs.context_menus.clone_from(&host.prefs.context_menus);
        }
        if changed.pinned {
            prefs.pinned.clone_from(&host.prefs.pinned);
        }
    }
    // The picker closed during a rename: it ends as when its row is no
    // longer shown (`finish_hidden_rename`).
    if !host.drawn && state.rename.is_some() {
        state.visible.clear();
        finish_hidden_rename(inv, state, &mut host.actions);
    }
    actions.append(&mut host.actions);
    let window = std::mem::replace(&mut host.window, placeholder());
    st.windows.retain(|w| w.id != PICKER);
    st.windows.push(window);
    put(ctx, host);
}

/// A stand-in while the real window is among the inventory's windows (its
/// state's `ui_id` is not `PICKER`).
fn placeholder() -> InventoryWindow {
    InventoryWindow {
        id: PICKER,
        root: Uuid::nil(),
        additional: false,
        state: Box::default(),
        open: false,
    }
}

/// The end of the inventory's pass: the host leaves the windows with the
/// shared clipboard, and with the preferences while a picker shows it.
pub(super) fn lend(ctx: &egui::Context, st: &mut InventoryUi, prefs: &InventoryPreferences, appearance_busy: bool) {
    let window = st.windows.iter().position(|w| w.id == PICKER).map(|index| st.windows.remove(index));
    let host = take(ctx);
    if window.is_none() && host.is_none() {
        // no picker has shown its list yet: nothing to lend
        return;
    }
    let mut host = host.unwrap_or_default();
    if let Some(window) = window {
        host.window = window;
    } else if host.window.state.ui_id != PICKER {
        host.window = Host::default().window;
    }
    let state = &mut host.window.state;
    state.clipboard.clone_from(&st.clipboard);
    state.cut = st.cut;
    state.pending = st.pending;
    state.merchant = st.merchant;
    host.clipboard = (st.clipboard.clone(), st.cut);
    host.appearance_busy = appearance_busy;
    // Copied only while a picker shows the list: its menu reads them.
    if host.drawn {
        host.prefs.clone_from(prefs);
        host.lent = true;
    } else if host.lent {
        host.prefs = InventoryPreferences::default();
        host.lent = false;
    }
    host.changed = Changed::default();
    host.drawn = false;
    put(ctx, host);
}

/// A picker's list for one pass: [`Rows::begin`], each row's
/// [`Rows::row`] and [`Rows::menu`], then [`Rows::finish`].
pub(crate) struct Rows {
    host: Box<Host>,
    facts: Option<Facts>,
}

impl Rows {
    /// `session`: the picker's opening, unique to it whichever caller opened it.
    pub(crate) fn begin(ctx: &egui::Context, session: u64) -> Self {
        let mut host = take(ctx).unwrap_or_default();
        if host.window.state.ui_id != PICKER {
            // left among the inventory's windows by a pass that did not
            // reach `lend`: a fresh list, the next pass keeps the old one
            host.window = Host::default().window;
        }
        if host.session != Some(session) {
            host.session = Some(session);
            let state = &mut host.window.state;
            state.message.clear();
            state.rename = None;
            state.selection.clear();
            state.selected = None;
        }
        host.drawn = true;
        host.window.state.visible.clear();
        Self { host, facts: None }
    }

    /// A row of the inventory is shown: its inline rename, when the menu's
    /// Rename is on it, is drawn in `label` (true then).
    pub(crate) fn row(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, id: Uuid, label: egui::Rect) -> bool {
        let state = &mut self.host.window.state;
        state.visible.push(id);
        if state.rename.as_ref().is_none_or(|edit| edit.id != id) {
            return false;
        }
        // over the row's name, allocating nothing in the list's layout
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(label));
        rename_row(&mut field, p, inv, id, state, &mut self.host.actions)
    }

    /// The row's right-click menu: the inventory's own for `id`, on the
    /// picker's single selection (allow_multi_select="false").
    pub(crate) fn menu(&mut self, response: &egui::Response, p: &Palette, world: &World, id: Uuid) {
        let Self { host, facts } = self;
        if response.secondary_clicked() {
            host.window.state.message.clear();
        }
        crate::ui::menu::context_menu(response, p, |ui| {
            let facts = facts.get_or_insert_with(|| Facts::from_world(world, host.appearance_busy));
            let state = &mut host.window.state;
            if state.selected != Some(id) || state.selection.len() != 1 || !state.selection.contains(&id) {
                state.selection.clear();
                state.selection.insert(id);
                state.selected = Some(id);
                state.anchor = Some(id);
            }
            let before = Before::of(&host.prefs);
            context::show(ui, p, &world.inventory, id, state, &mut host.prefs, facts, &mut host.actions);
            before.compare(&host.prefs, &mut host.changed);
        });
    }

    /// What the last command of the menu answered, for the picker's status
    /// line.
    pub(crate) fn message(&self) -> &str {
        &self.host.window.state.message
    }

    /// The end of the list: a rename whose row is no longer shown ends, and
    /// a menu no longer drawn (or whose window lost the focus) lets go of
    /// its layout editing, as the inventory's windows do each pass.
    pub(crate) fn finish(mut self, ctx: &egui::Context, inv: &Inventory) {
        let host = &mut self.host;
        finish_hidden_rename(inv, &mut host.window.state, &mut host.actions);
        if host.window.state.menu_editor.shown() {
            let before = Before::of(&host.prefs);
            host.window
                .state
                .menu_editor
                .finish_inventory_frame(ctx, &mut host.prefs.context_menus);
            before.compare(&host.prefs, &mut host.changed);
        }
        put(ctx, self.host);
    }
}

/// The preferences a command may change, before it runs.
struct Before {
    protected: HashSet<Uuid>,
    upload_folders: [Uuid; 5],
    context_menus: super::menu_layout::Preferences,
    pinned: Vec<Uuid>,
}

impl Before {
    fn of(prefs: &InventoryPreferences) -> Self {
        Self {
            protected: prefs.protected.clone(),
            upload_folders: prefs.upload_folders,
            context_menus: prefs.context_menus.clone(),
            pinned: prefs.pinned.clone(),
        }
    }

    fn compare(self, prefs: &InventoryPreferences, changed: &mut Changed) {
        changed.protected |= self.protected != prefs.protected;
        changed.upload_folders |= self.upload_folders != prefs.upload_folders;
        changed.context_menus |= self.context_menus != prefs.context_menus;
        changed.pinned |= self.pinned != prefs.pinned;
    }
}

/// The actions the picker's menu asked for and not yet taken by the
/// inventory's pass.
#[cfg(test)]
pub(crate) fn pending_actions(ctx: &egui::Context) -> Vec<InvAction> {
    let Some(mut host) = take(ctx) else {
        return Vec::new();
    };
    let actions = std::mem::take(&mut host.actions);
    put(ctx, host);
    actions
}

/// Where the picker's open menu drew `key` (`context_customize::State::rect`).
#[cfg(test)]
pub(crate) fn menu_rect(ctx: &egui::Context, key: &str) -> Option<egui::Rect> {
    let host = take(ctx)?;
    let rect = host.window.state.menu_editor.rect(key);
    put(ctx, host);
    rect
}

/// The item whose Properties the hosted panel shows, wherever the host is.
#[cfg(test)]
pub(crate) fn properties(ctx: &egui::Context, st: &InventoryUi) -> Option<Uuid> {
    let of = |state: &InventoryUi| match state.dialog {
        Some(EditDialog::Properties(id)) => Some(id),
        _ => None,
    };
    let lent = take(ctx);
    let shown = lent.as_ref().and_then(|host| of(&host.window.state));
    if let Some(host) = lent {
        put(ctx, host);
    }
    shown.or_else(|| st.windows.iter().filter(|w| w.id == PICKER).find_map(|w| of(&w.state)))
}
