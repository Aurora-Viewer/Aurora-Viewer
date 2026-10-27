//! Interactive presentation of native commands, shared by inventory and scene menus.
//! Editing never executes a command.
use super::menu_layout::{
    Direction, Entry, HIDDEN, ICON_SLOTS, Kind, LOCK, Layout, Profile, Resolved, TEXT, bar, can_hide, children, hidden_zone, icon_zone,
};
use crate::{
    theme::Palette,
    ui::{menu, widgets},
};
use egui::{Id, Pos2, Response, Sense, Stroke, Ui, Vec2};
use uuid::Uuid;

const WIDTH: f32 = 272.0;
const CONTROL: f32 = 28.0;

fn lost_window_focus(ctx: &egui::Context) -> bool {
    // A minimized window can lose and regain focus between rendered frames.
    ctx.input(|i| !i.focused || i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))))
}

impl super::InventoryUi {
    /// Native offline fixtures use the same saved presentation as a real menu.
    pub(crate) fn configure_menu_demo(&mut self, inv: &super::Inventory, prefs: &mut super::InventoryPreferences, id: Uuid, mode: &str) {
        let profile = prefs
            .context_menus
            .profiles
            .entry(super::context_catalog::scope(inv, id))
            .or_default();
        *profile = Profile::default();
        profile.locked = !matches!(
            mode,
            "aurora-menu-custom"
                | "aurora-menu-hidden"
                | "aurora-menu-bars"
                | "aurora-menu-drop"
                | "aurora-menu-all-texture"
                | "aurora-menu-all-folder"
                | "aurora-menu-folder-uuid"
                | "aurora-menu-all-trash"
                | "aurora-menu-empty-custom"
        );
        self.menu_editor.demo_drag = (mode == "aurora-menu-drop").then_some(0);
        if matches!(mode, "aurora-menu-empty" | "aurora-menu-empty-custom") {
            // Reproduce a saved attachment group whose options were moved out.
            for id in [
                "wear",
                "add",
                "attach-body",
                "attach-hud",
                "detach",
                "replace-outfit",
                "append-outfit",
                "wear-contents",
                "remove-outfit",
            ] {
                profile.layout.placements.insert(id.into(), TEXT.into());
            }
            profile.layout.directions.insert("attachments".into(), Direction::Horizontal);
            profile
                .layout
                .create_group(&[], aurora_i18n::tr!("inventory-menu-demo-group"), Direction::Vertical, None);
        }
        if mode == "aurora-menu-filter" {
            profile.layout.filters.insert("attach-body".into());
            profile.layout.directions.insert("attach-body".into(), Direction::Vertical);
            profile.open.insert("attach-body".into(), true);
            // Synthetic user input for the French offline capture.
            self.menu_editor.queries.insert("attach-body".into(), "main".into());
        }
        if let Some(mode) = match mode {
            "aurora-menu-export" => Some(super::context_notecards::Mode::Export),
            "aurora-menu-load" => Some(super::context_notecards::Mode::Load),
            _ => None,
        } {
            self.menu_editor.notecard = Some(mode);
        }
        if mode == "aurora-menu-organize" {
            profile.open.insert("organize".into(), true);
        }
        if mode == "aurora-menu-textures-export" {
            profile.open.insert("save-textures".into(), true);
        }
        if matches!(mode, "aurora-menu-custom" | "aurora-menu-locked" | "aurora-menu-hidden") {
            let group = "custom:demo";
            profile.layout.groups.insert(
                group.into(),
                super::menu_layout::Group {
                    name: aurora_i18n::tr!("inventory-menu-demo-group").into(),
                    direction: Direction::Vertical,
                    keep_empty: false,
                },
            );
            profile.layout.placements.insert("rename".into(), children(group));
            profile.layout.placements.insert("copy".into(), TEXT.into());
            profile.layout.placements.insert(LOCK.into(), TEXT.into());
            profile.layout.placements.insert("image".into(), "top".into());
            profile.layout.slots.entry("top".into()).or_default().insert("image".into(), 2);
            profile.layout.directions.insert("organize".into(), Direction::Horizontal);
            profile.locked = mode.ends_with("locked");
        }
        if mode == "aurora-menu-hidden" {
            profile.layout.placements.insert("image".into(), HIDDEN.into());
            profile.layout.placements.insert("organize".into(), HIDDEN.into());
        }
        if mode == "aurora-menu-folder-uuid-restored" {
            profile.layout.placements.insert("folder-uuid".into(), TEXT.into());
        }
        if mode == "aurora-menu-bars" {
            let catalog = super::context_catalog::build(inv, id, self, prefs, &super::Facts::default());
            let profile = prefs
                .context_menus
                .profiles
                .entry(super::context_catalog::scope(inv, id))
                .or_default();
            profile.layout.add_icon_bar(&catalog.entries);
            let bar = profile.layout.icon_bars.iter().next().cloned();
            if let Some(bar) = bar {
                let zone = icon_zone(&bar);
                profile.layout.move_entry(&catalog.entries, "copy", &zone, None, Some(2));
                profile.layout.move_entry(&catalog.entries, "image", &zone, None, Some(7));
            }
            profile.layout.remove_container(&catalog.entries, "bar:bottom");
            profile.layout.add_icon_bar(&catalog.entries);
        }
        if mode == "aurora-menu-restored" {
            super::context_notecards::demo_restore(self, prefs);
        }
        if matches!(mode, "aurora-menu-load" | "aurora-menu-load-search") {
            super::context_notecards::demo_picker(self, mode.ends_with("search"));
        }
    }
}

#[derive(Clone)]
struct Dragged {
    scope: String,
    root: Id,
    entry: String,
}
#[derive(Clone)]
enum Stage {
    Choices,
    Name(Direction),
}
#[derive(Clone)]
struct Editor {
    target: Option<String>,
    pos: Pos2,
    stage: Stage,
    name: String,
    focus: bool,
}
#[derive(Default)]
pub(crate) struct State {
    pub(crate) help: std::collections::BTreeMap<String, String>,
    root: Option<(Uuid, Id, String)>,
    frame: u64,
    right_frame: Option<u64>,
    keep_open_frame: Option<u64>,
    last_right: Option<Pos2>,
    editor: Option<Editor>,
    selected: Option<String>,
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    regions: Vec<egui::Rect>,
    submenus: Vec<egui::LayerId>,
    /// Offline capture fixture: the first pointer frame of a held drag.
    demo_drag: Option<u64>,
    pub(crate) notecard: Option<super::context_notecards::Mode>,
    queries: std::collections::BTreeMap<String, String>,
    #[cfg(test)]
    rects: std::collections::BTreeMap<String, egui::Rect>,
}
impl State {
    pub(crate) fn relock(&mut self, ctx: &egui::Context, profile: &mut Profile) {
        profile.locked = true;
        if let Some((_, root, scope)) = self.root.take() {
            if egui::DragAndDrop::payload::<Dragged>(ctx).is_some_and(|drag| drag.root == root && drag.scope == scope) {
                egui::DragAndDrop::clear_payload(ctx);
            }
            egui::containers::menu::MenuState::from_id(ctx, root, |state| state.open_item = None);
        }
        self.editor = None;
        self.selected = None;
        self.undo.clear();
        self.redo.clear();
        self.last_right = None;
        self.keep_open_frame = None;
        self.regions.clear();
        self.submenus.clear();
        self.demo_drag = None;
        self.queries.clear();
    }

    fn dismiss(&mut self, ctx: &egui::Context, profile: &mut Profile) {
        let root = self.root.as_ref().map(|(_, root, _)| *root);
        self.relock(ctx, profile);
        if let Some(root) = root {
            egui::Popup::close_id(ctx, root);
        }
    }

    /// Dismissal can bypass the popup body (closed inventory, replaced popup,
    /// or a native window move). Editing must end even on those frames.
    pub(crate) fn finish_frame(&mut self, ctx: &egui::Context, profile: &mut Profile) {
        if self.root.is_some() && (lost_window_focus(ctx) || self.frame != ctx.cumulative_frame_nr()) {
            self.dismiss(ctx, profile);
        }
    }

    pub(super) fn finish_inventory_frame(&mut self, ctx: &egui::Context, prefs: &mut super::menu_layout::Preferences) {
        if let Some((_, _, scope)) = &self.root
            && let Some(profile) = prefs.profiles.get_mut(scope)
        {
            self.finish_frame(ctx, profile);
        }
    }

    pub(crate) fn prepare(&mut self, ctx: &egui::Context, item: Uuid, root: Id, scope: &str, prefs: &mut super::menu_layout::Preferences) {
        if let Some((previous_item, previous_root, previous_scope)) = &self.root
            && (*previous_item != item || *previous_root != root || previous_scope != scope || ctx.cumulative_frame_nr() > self.frame + 1)
            && let Some(profile) = prefs.profiles.get_mut(previous_scope)
        {
            // A newly opened menu can reuse the previous popup's ID.
            self.relock(ctx, profile);
        }
    }

    fn remember(&mut self, before: Layout, after: &Layout) {
        if before != *after {
            self.undo.push(before);
            if self.undo.len() > 40 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    fn edit(&mut self, target: Option<String>, pos: Pos2) {
        self.editor = Some(Editor {
            target,
            pos,
            stage: Stage::Choices,
            name: String::new(),
            focus: true,
        });
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    p: &Palette,
    item: Uuid,
    scope: &str,
    available: &[Entry],
    profile: &mut Profile,
    state: &mut State,
) -> Option<usize> {
    if lost_window_focus(ui.ctx()) {
        state.dismiss(ui.ctx(), profile);
        ui.close();
        return None;
    }
    menu::distinct_surface(ui);
    let frame = ui.ctx().cumulative_frame_nr();
    let root = ui.layer_id().id;
    if state.keep_open_frame == Some(frame) {
        menu::keep_parent_open_this_pass(ui);
    }
    let fresh = state
        .root
        .as_ref()
        .is_none_or(|(id, layer, kind)| *id != item || *layer != root || kind != scope)
        || frame > state.frame + 1;
    let secondary = ui.input(|i| i.pointer.secondary_clicked());
    let handle_right = secondary && state.right_frame != Some(frame);
    let mut right_dismissed = false;
    if fresh {
        if state.root.as_ref().is_some_and(|(_, _, kind)| kind != scope) {
            state.undo.clear();
            state.redo.clear();
        }
        state.root = Some((item, root, scope.into()));
        state.editor = None;
        state.selected = None;
        state.last_right = ui.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten());
    }
    state.frame = frame;
    let pos = ui.input(|i| i.pointer.interact_pos());
    if state.last_right.zip(pos).is_some_and(|(a, b)| a.distance(b) > 2.0) {
        state.last_right = None;
    }
    if handle_right {
        state.right_frame = Some(frame);
    }
    if !fresh && handle_right {
        if state.last_right.zip(pos).is_some_and(|(a, b)| a.distance(b) <= 2.0) {
            state.last_right = None;
            if state.editor.take().is_none() {
                state.dismiss(ui.ctx(), profile);
                ui.close();
                return None;
            } else {
                state.keep_open_frame = Some(frame);
                menu::keep_parent_open_this_pass(ui);
                right_dismissed = true;
            }
        } else {
            state.last_right = pos;
        }
    }
    if profile.locked {
        state.editor = None;
        if egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_some() {
            egui::DragAndDrop::clear_payload(ui.ctx());
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_some() {
            ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
            egui::DragAndDrop::clear_payload(ui.ctx());
        } else if state.editor.take().is_some() {
            ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        } else {
            state.dismiss(ui.ctx(), profile);
            ui.close();
            return None;
        }
    }
    // The offline fixture reuses the real drop renderer and preview, warming
    // the latter with a short title before switching to a longer command.
    if let Some(start) = &mut state.demo_drag
        && !profile.locked
        && ui.ctx().pointer_interact_pos().is_some()
    {
        if *start == 0 {
            *start = frame;
        }
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            Dragged {
                root,
                scope: scope.into(),
                entry: if frame.saturating_sub(*start) < 60 { "image" } else { "attach-hud" }.into(),
            },
        );
    }
    keyboard(ui, available, profile, state);
    let mut resolved = profile.layout.resolve(available);
    // Scene menus start with text actions. Keep a saved top bar only when
    // it contains commands; extra bars can still be added while organizing.
    if scope.starts_with("scene:") && resolved.get("top").is_none_or(Vec::is_empty) {
        for rows in resolved.values_mut() {
            rows.retain(|entry| entry.id != "bar:top");
        }
    }
    if profile.locked {
        super::menu_layout::hide_empty_groups(&mut resolved);
    }
    state.regions.clear();
    state.submenus.clear();
    let mut view = View {
        p,
        scope,
        root,
        available,
        resolved: &resolved,
        profile,
        state,
        chosen: None,
        context_handled: fresh || !handle_right || right_dismissed,
        query: String::new(),
    };
    ui.set_min_width(WIDTH);
    let height = (ui.ctx().content_rect().height() - 32.0).max(100.0);
    ui.set_max_height(height);
    let body = egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| view.zone(ui, TEXT, 0));
    view.state.regions.push(ui.min_rect().expand(4.0));
    // A right click on the menu's unoccupied area opens its own editor.
    if !view.context_handled && secondary && body.inner_rect.contains(pos.unwrap_or(Pos2::ZERO)) && !view.profile.locked {
        view.state.edit(None, pos.unwrap());
    }
    view.settings(ui);
    for layer in &view.state.submenus {
        ui.ctx().move_to_top(*layer);
    }
    view.editor(ui);
    if view.state.notecard.is_some() {
        view.state.dismiss(ui.ctx(), view.profile);
        ui.close();
    }
    if !view.profile.locked
        && let Some(drag) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
        && drag.scope == scope
        && drag.root == root
        && let Some(entry) = resolved.values().flatten().find(|entry| entry.id == drag.entry)
    {
        widgets::organization_drag_preview(
            ui.ctx(),
            p,
            root.with("inventory_layout_drag"),
            if entry.is_icon_bar() {
                aurora_i18n::tr!("inventory-menu-icon-bar")
            } else if entry.is_separator() {
                aurora_i18n::tr!("inventory-menu-separator-name")
            } else {
                &entry.label
            },
        );
    }
    if !view.profile.locked
        && !right_dismissed
        && ui.input(|i| i.pointer.any_click())
        // Fixed controls can precede this editor in the same popup.
        && !menu::clicks_kept_row(ui.ctx(), root)
        && pos.is_some_and(|pos| !view.state.regions.iter().any(|rect| rect.contains(pos)))
    {
        view.state.dismiss(ui.ctx(), view.profile);
        ui.close();
    }
    view.chosen
}

#[derive(Clone, Copy)]
enum DropFeedback {
    Before,
    After(f32),
    Inside,
}

struct View<'a> {
    p: &'a Palette,
    scope: &'a str,
    root: Id,
    available: &'a [Entry],
    resolved: &'a Resolved,
    profile: &'a mut Profile,
    state: &'a mut State,
    chosen: Option<usize>,
    context_handled: bool,
    query: String,
}

fn label_matches(label: &str, query: &str) -> bool {
    let label = label.to_lowercase();
    query.split_whitespace().all(|word| label.contains(word))
}

fn matches_filter(entry: &Entry, resolved: &Resolved, query: &str, depth: usize) -> bool {
    if query.is_empty() || entry.kind == Kind::Lock || label_matches(&entry.label, query) {
        return true;
    }
    if depth > 32 {
        return false;
    }
    let zone = if entry.group() {
        children(&entry.id)
    } else if entry.is_icon_bar() {
        icon_zone(&entry.id)
    } else {
        return false;
    };
    resolved.get(&zone).is_some_and(|entries| {
        entries
            .iter()
            .any(|entry| !entry.is_separator() && matches_filter(entry, resolved, query, depth + 1))
    })
}
impl View<'_> {
    fn entry_icon<'e>(&self, entry: &'e Entry) -> &'e str {
        if entry.kind == Kind::Lock {
            if self.profile.locked { "lock-filled" } else { "lock-open" }
        } else {
            &entry.icon
        }
    }
    fn bar(&mut self, ui: &mut Ui, entry: &Entry, parent: &str, depth: usize) {
        // Every bar reserves eight absolute cells, including unused ones.
        let zone = icon_zone(&entry.id);
        let cells = self.profile.layout.icon_slots(&zone, &self.resolved[&zone]);
        let width = if ui.is_sizing_pass() {
            WIDTH
        } else {
            ui.available_width().max(WIDTH)
        };
        // The lock can toggle while rendering a cell. Keep this row's geometry
        // consistent for the whole pass so its trailing control cannot widen it.
        let editing = !self.profile.locked;
        let controls = if editing { 2.0 * CONTROL } else { 0.0 };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if editing {
                let grip = menu::icon_cell(
                    ui,
                    self.p,
                    "grip-vertical",
                    aurora_i18n::tr!("inventory-menu-icon-bar"),
                    true,
                    true,
                    Vec2::new(CONTROL, 27.0),
                );
                let grip = self.drag_cursor(grip);
                self.source(&grip, entry);
                self.context(&grip, entry);
                self.row_drop(ui, &grip, entry, parent, None);
            }
            for (slot, id) in cells.into_iter().enumerate() {
                ui.push_id((&zone, slot), |ui| {
                    let entry = id
                        .and_then(|id| self.resolved[&zone].iter().find(|e| e.id == id))
                        .filter(|entry| matches_filter(entry, self.resolved, &self.query, 0));
                    let response = menu::icon_cell(
                        ui,
                        self.p,
                        entry.map_or("", |e| self.entry_icon(e)),
                        entry.map_or("", |e| if e.kind == Kind::Lock { "" } else { e.label.as_str() }),
                        entry.is_some_and(|e| e.enabled),
                        editing,
                        Vec2::new((width - controls) / ICON_SLOTS as f32, 27.0),
                    );
                    #[cfg(test)]
                    self.state.rects.insert(format!("slot:{zone}:{slot}"), response.rect);
                    if let Some(entry) = entry {
                        self.interact(ui, &response, entry, &zone, depth + 1, true);
                    }
                    self.drop(ui, &response, &zone, None, Some(slot), DropFeedback::Inside);
                });
            }
            if editing {
                let remove = widgets::action_button(
                    ui,
                    self.p,
                    "trash-filled",
                    "",
                    widgets::ActionKind::FlatDanger,
                    Vec2::new(CONTROL, 27.0),
                )
                .on_hover_text(aurora_i18n::tr!("inventory-menu-remove-icon-bar"));
                #[cfg(test)]
                self.state.rects.insert(format!("remove:{}", entry.id), remove.rect);
                if remove.clicked() && !self.profile.locked {
                    let before = self.profile.layout.clone();
                    self.profile.layout.remove_container(self.available, &entry.id);
                    self.state.remember(before, &self.profile.layout);
                }
            }
        });
    }

    fn zone(&mut self, ui: &mut Ui, zone: &str, depth: usize) {
        if depth > 32 {
            return;
        }
        let previous_query = self.query.clone();
        if let Some(owner) = zone
            .strip_prefix("children:")
            .filter(|owner| self.profile.layout.filters.contains(*owner))
        {
            let text = self.state.queries.entry(owner.into()).or_default();
            let width = if ui.is_sizing_pass() {
                WIDTH
            } else {
                ui.available_width().max(WIDTH)
            };
            let clear_shown = !text.is_empty();
            let (_field, clear) = super::tree::search_field(
                ui,
                self.p,
                text,
                aurora_i18n::tr!("inventory-menu-filter"),
                width,
                clear_shown,
                aurora_i18n::tr!("inventory-menu-clear-filter"),
            );
            #[cfg(test)]
            self.state.rects.insert(format!("filter:{owner}"), _field.rect);
            if clear {
                text.clear();
            }
            self.query = format!("{} {}", previous_query, text.to_lowercase()).trim().into();
        }
        if let Some(entries) = self.resolved.get(zone) {
            let rows = super::menu_layout::visible_rows(
                entries
                    .iter()
                    .filter(|entry| entry.is_separator() || matches_filter(entry, self.resolved, &self.query, 0)),
            );
            if rows.is_empty() && !self.query.is_empty() {
                ui.label(aurora_i18n::tr!("common-no-results"));
            }
            for entry in rows {
                ui.push_id(&entry.id, |ui| {
                    if entry.is_icon_bar() {
                        self.bar(ui, entry, zone, depth);
                    } else if entry.is_separator() {
                        let width = if ui.is_sizing_pass() {
                            WIDTH
                        } else {
                            ui.available_width().max(WIDTH)
                        };
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(width, 8.0),
                            if self.profile.locked {
                                Sense::hover()
                            } else {
                                Sense::click_and_drag()
                            },
                        );
                        ui.painter()
                            .hline(rect.x_range().shrink(4.0), rect.center().y, widgets::divider_stroke(self.p));
                        let response = self.drag_cursor(response);
                        self.source(&response, entry);
                        self.context(&response, entry);
                        self.drop(ui, &response, zone, Some(&entry.id), None, DropFeedback::Before);
                    } else {
                        let down = match entry.kind {
                            Kind::Group { direction, .. } => Some(direction == Direction::Vertical),
                            _ => None,
                        };
                        let response = menu::editable_row(
                            ui,
                            self.p,
                            self.entry_icon(entry),
                            &entry.label,
                            entry.enabled,
                            if entry.kind == Kind::Lock {
                                Some(self.profile.locked)
                            } else {
                                entry.checked
                            },
                            down,
                            !self.profile.locked,
                        );
                        self.interact(ui, &response, entry, zone, depth, false);
                    }
                });
            }
        }
        let empty = self.resolved.get(zone).is_none_or(Vec::is_empty);
        let height = if empty && zone == HIDDEN {
            30.0
        } else if empty && zone != TEXT {
            22.0
        } else {
            7.0
        };
        // Like menu rows, drop targets must measure their natural width;
        // filling the initial popup estimate would permanently widen it.
        let width = if ui.is_sizing_pass() {
            WIDTH
        } else {
            ui.available_width().max(WIDTH)
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        if empty && zone != TEXT {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if zone == HIDDEN {
                    aurora_i18n::tr!("inventory-menu-drop-hidden")
                } else {
                    aurora_i18n::tr!("inventory-menu-drop-here")
                },
                egui::FontId::proportional(11.0),
                if zone == HIDDEN { self.p.muted } else { self.p.muted_dim },
            );
        }
        self.drop(
            ui,
            &response,
            zone,
            None,
            None,
            if zone == TEXT { DropFeedback::Before } else { DropFeedback::Inside },
        );
        self.query = previous_query;
    }

    fn source(&mut self, response: &Response, entry: &Entry) {
        #[cfg(test)]
        self.state.rects.insert(entry.id.clone(), response.rect);
        if response.hovered() {
            self.state.selected = Some(entry.id.clone());
        }
        if !self.profile.locked {
            response.dnd_set_drag_payload(Dragged {
                scope: self.scope.into(),
                root: self.root,
                entry: entry.id.clone(),
            });
        }
    }
    fn context(&mut self, response: &Response, entry: &Entry) {
        if !self.context_handled && !self.profile.locked && response.secondary_clicked() {
            self.context_handled = true;
            self.state.edit(
                Some(entry.id.clone()),
                response.interact_pointer_pos().unwrap_or(response.rect.center()),
            );
        }
    }
    fn drag_cursor(&self, response: Response) -> Response {
        if !self.profile.locked {
            let cursor = if response.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            };
            response.on_hover_and_drag_cursor(cursor)
        } else {
            response
        }
    }
    fn interact(&mut self, ui: &mut Ui, response: &Response, entry: &Entry, zone: &str, depth: usize, icon: bool) {
        if let Some(help) = self.state.help.get(&entry.id) {
            let _ = response.clone().on_hover_text(help);
        }
        if entry.id == "asset-uuid" {
            let _ = response.clone().on_hover_text(aurora_i18n::tr!("inventory-copy-asset-uuid-help"));
        }
        let _ = self.drag_cursor(response.clone());
        self.source(response, entry);
        self.context(response, entry);
        match entry.kind {
            Kind::Lock => {
                let _ = response.clone().on_hover_text(if self.profile.locked {
                    aurora_i18n::tr!("inventory-menu-unlock")
                } else {
                    aurora_i18n::tr!("inventory-menu-lock")
                });
                if response.clicked() && egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_none() {
                    self.profile.locked = !self.profile.locked;
                    self.state.editor = None;
                }
                if !icon {
                    self.row_drop(ui, response, entry, zone, None);
                }
            }
            Kind::Action(index) => {
                if self.profile.locked
                    && response.clicked()
                    && entry.enabled
                    && !hidden_zone(self.resolved, zone)
                    && egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_none()
                {
                    self.chosen = Some(index);
                }
                if !icon {
                    self.row_drop(ui, response, entry, zone, None);
                }
            }
            Kind::Group { direction, open } => {
                let query = self.query.clone();
                if !query.is_empty() && label_matches(&entry.label, &query) {
                    self.query.clear();
                }
                let child = children(&entry.id);
                let mut bottom = response.rect.bottom();
                let inside = !icon
                    && ui
                        .input(|i| i.pointer.interact_pos())
                        .is_some_and(|p| (p.y - response.rect.center().y).abs() < response.rect.height() * 0.25);
                if direction == Direction::Horizontal || icon {
                    if entry.enabled || !self.profile.locked {
                        menu::editable_submenu(
                            ui,
                            self.p,
                            response,
                            !self.profile.locked || self.profile.layout.filters.contains(&entry.id),
                            icon,
                            self.scope.starts_with("scene:"),
                            |ui| {
                                self.state.submenus.push(ui.layer_id());
                                menu::distinct_surface(ui);
                                ui.set_min_width(WIDTH);
                                egui::ScrollArea::vertical()
                                    .max_height((ui.ctx().content_rect().height() - 40.0).min(550.0))
                                    .show(ui, |ui| self.zone(ui, &child, depth + 1));
                                self.state.regions.push(ui.min_rect().expand(4.0));
                            },
                        );
                    }
                } else {
                    let mut expanded = self.profile.open.get(&entry.id).copied().unwrap_or(open);
                    if response.clicked() {
                        expanded = !expanded;
                        self.profile.open.insert(entry.id.clone(), expanded);
                    }
                    if !self.profile.locked && inside && response.dnd_hover_payload::<Dragged>().is_some_and(|d| d.entry != entry.id) {
                        expanded = true;
                        self.profile.open.insert(entry.id.clone(), true);
                    }
                    if expanded {
                        let body = ui.indent(&child, |ui| self.zone(ui, &child, depth + 1));
                        bottom = bottom.max(body.response.rect.bottom());
                        #[cfg(test)]
                        self.state.rects.insert(format!("group-body:{}", entry.id), body.response.rect);
                    }
                }
                if !icon {
                    if inside {
                        self.drop(ui, response, &child, None, None, DropFeedback::Inside);
                    } else {
                        self.row_drop(ui, response, entry, zone, Some(bottom));
                    }
                }
                self.query = query;
            }
            Kind::Separator | Kind::IconBar => {}
        }
    }
    fn row_drop(&mut self, ui: &Ui, response: &Response, entry: &Entry, zone: &str, bottom: Option<f32>) {
        let after = ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| p.y > response.rect.center().y);
        let before = if after {
            self.resolved[zone]
                .iter()
                .position(|e| e.id == entry.id)
                .and_then(|i| self.resolved[zone].get(i + 1))
                .map(|e| e.id.as_str())
        } else {
            Some(entry.id.as_str())
        };
        self.drop(
            ui,
            response,
            zone,
            before,
            None,
            if after {
                DropFeedback::After(bottom.unwrap_or(response.rect.bottom()))
            } else {
                DropFeedback::Before
            },
        );
    }
    fn drop(&mut self, ui: &Ui, response: &Response, zone: &str, before: Option<&str>, slot: Option<usize>, feedback: DropFeedback) {
        if self.profile.locked {
            return;
        }
        if let Some(drag) = response.dnd_hover_payload::<Dragged>() {
            if drag.scope != self.scope || drag.root != self.root {
                return;
            }
            let mut next = self.profile.layout.clone();
            if !next.move_entry(self.available, &drag.entry, zone, before, slot) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                return;
            }
            match feedback {
                DropFeedback::Inside => ui.painter().rect_stroke(
                    response.rect.shrink(1.0),
                    ui.visuals().window_corner_radius,
                    Stroke::new(1.5, self.p.violet_light),
                    egui::StrokeKind::Inside,
                ),
                DropFeedback::Before => {
                    ui.painter()
                        .hline(response.rect.x_range(), response.rect.top(), Stroke::new(2.0, self.p.violet_light))
                }
                DropFeedback::After(y) => ui
                    .painter()
                    .hline(response.rect.x_range(), y, Stroke::new(2.0, self.p.violet_light)),
            };
            if response.dnd_release_payload::<Dragged>().is_some() {
                let previous = std::mem::replace(&mut self.profile.layout, next);
                self.state.remember(previous, &self.profile.layout);
            }
        }
    }

    fn settings(&mut self, ui: &Ui) {
        if self.profile.locked {
            return;
        }
        menu::settings_panel(ui, self.p, self.root.with("inventory_layout_settings"), |ui| {
            let close = widgets::organization_header(ui, self.p, aurora_i18n::tr!("inventory-menu-hidden"));
            #[cfg(test)]
            self.state.rects.insert("settings:close".into(), close.rect);
            if close.clicked() {
                self.profile.locked = true;
                self.state.editor = None;
                return;
            }
            let height = (ui.ctx().content_rect().height() - 220.0).max(80.0);
            let hidden = widgets::surface(ui, self.p, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(HIDDEN)
                    .max_height(height)
                    .show(ui, |ui| self.zone(ui, HIDDEN, 0))
            });
            let response = ui.interact(hidden.inner_rect, ui.scope_id().with("hidden_drop"), Sense::hover());
            #[cfg(test)]
            self.state.rects.insert("hidden:drop".into(), hidden.inner_rect);
            // Rows handle precise insertion; the remaining shelf accepts an append.
            self.drop(ui, &response, HIDDEN, None, None, DropFeedback::Inside);
            let create = widgets::action_button(
                ui,
                self.p,
                "plus",
                aurora_i18n::tr!("inventory-menu-create-group"),
                widgets::ActionKind::Flat,
                Vec2::new(ui.available_width(), 28.0),
            );
            #[cfg(test)]
            self.state.rects.insert("footer:create".into(), create.rect);
            if create.clicked() {
                self.state.edit(None, create.rect.left_bottom());
            }
            let add_bar = widgets::action_button_if(
                ui,
                self.p,
                "plus",
                aurora_i18n::tr!("inventory-menu-add-icon-bar"),
                widgets::ActionKind::Flat,
                Vec2::new(ui.available_width(), 28.0),
                self.profile.layout.icon_bars.len() < 16,
            );
            #[cfg(test)]
            self.state.rects.insert("settings:add-bar".into(), add_bar.rect);
            if add_bar.clicked() {
                let before = self.profile.layout.clone();
                self.profile.layout.add_icon_bar(self.available);
                self.state.remember(before, &self.profile.layout);
            }
            let reset = widgets::action_button(
                ui,
                self.p,
                "restore",
                aurora_i18n::tr!("inventory-menu-reset"),
                widgets::ActionKind::Flat,
                Vec2::new(ui.available_width(), 28.0),
            );
            #[cfg(test)]
            self.state.rects.insert("footer:reset".into(), reset.rect);
            if reset.clicked() {
                let old = if self.scope.starts_with("scene:") {
                    self.profile.open.clear();
                    std::mem::take(&mut self.profile.layout)
                } else {
                    self.profile.reset()
                };
                self.state.queries.clear();
                self.state.remember(old, &self.profile.layout);
                self.state.editor = None;
            }
            widgets::divider(ui, self.p);
            for (mode, icon, title) in [
                (
                    super::context_notecards::Mode::Export,
                    "clipboard-text-filled",
                    aurora_i18n::tr!("inventory-menu-export-notecard"),
                ),
                (
                    super::context_notecards::Mode::Load,
                    "folder-filled",
                    aurora_i18n::tr!("inventory-menu-load-notecard"),
                ),
            ] {
                let button = widgets::action_button(
                    ui,
                    self.p,
                    icon,
                    title,
                    widgets::ActionKind::Flat,
                    Vec2::new(ui.available_width(), 28.0),
                );
                #[cfg(test)]
                self.state.rects.insert(format!("settings:notecard:{mode:?}"), button.rect);
                if button.clicked() {
                    self.state.notecard = Some(mode);
                }
            }
            let finish = widgets::action_button(
                ui,
                self.p,
                "adjustments-filled",
                aurora_i18n::tr!("inventory-menu-finish"),
                widgets::ActionKind::Flat,
                Vec2::new(ui.available_width(), 28.0),
            );
            #[cfg(test)]
            self.state.rects.insert("settings:finish".into(), finish.rect);
            if finish.clicked() {
                self.profile.locked = true;
                self.state.editor = None;
            }
            self.state.regions.push(ui.min_rect().expand(12.0));
        });
    }

    fn editor(&mut self, ui: &Ui) {
        let Some(mut editor) = self.state.editor.take() else {
            return;
        };
        if self.profile.locked {
            return;
        }
        let entry = editor
            .target
            .as_ref()
            .and_then(|id| self.resolved.values().flatten().find(|e| &e.id == id));
        if editor.target.is_some() && entry.is_none() {
            return;
        }
        let mut open = true;
        let before = self.profile.layout.clone();
        let mut keep = true;
        menu::choice_panel(
            ui,
            self.p,
            self.root.with("inventory_layout_choices"),
            editor.pos,
            &mut open,
            |ui| {
                menu::distinct_surface(ui);
                match editor.stage {
                    Stage::Name(direction) => {
                        menu::section(ui, self.p, aurora_i18n::tr!("inventory-menu-create-group"));
                        ui.label(aurora_i18n::tr!("inventory-menu-group-name"));
                        let field = ui.add(egui::TextEdit::singleline(&mut editor.name).char_limit(80).desired_width(220.0));
                        if editor.focus {
                            field.request_focus();
                            editor.focus = false;
                        }
                        ui.horizontal(|ui| {
                            let create = widgets::action_button_if(
                                ui,
                                self.p,
                                "plus",
                                aurora_i18n::tr!("inventory-menu-create"),
                                widgets::ActionKind::Primary,
                                Vec2::new(106.0, 25.0),
                                !editor.name.trim().is_empty(),
                            );
                            #[cfg(test)]
                            self.state.rects.insert("editor:create".into(), create.rect);
                            if create.clicked()
                                || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !editor.name.trim().is_empty())
                            {
                                self.profile
                                    .layout
                                    .create_group(self.available, &editor.name, direction, editor.target.as_deref());
                                keep = false;
                            }
                            if widgets::action_button(
                                ui,
                                self.p,
                                "x",
                                aurora_i18n::tr!("common-cancel"),
                                widgets::ActionKind::Flat,
                                Vec2::new(106.0, 25.0),
                            )
                            .clicked()
                            {
                                keep = false;
                            }
                        });
                    }
                    Stage::Choices => {
                        if let Some(entry) = entry.filter(|e| e.is_icon_bar()) {
                            let remove = menu::editable_row(
                                ui,
                                self.p,
                                "trash-filled",
                                aurora_i18n::tr!("inventory-menu-remove-icon-bar"),
                                true,
                                None,
                                None,
                                false,
                            );
                            #[cfg(test)]
                            self.state.rects.insert("editor:remove-bar".into(), remove.rect);
                            if remove.clicked() {
                                self.profile.layout.remove_container(self.available, &entry.id);
                                keep = false;
                            }
                        } else if let Some(entry) = entry.filter(|e| e.is_separator()) {
                            if menu::item(ui, self.p, "trash-filled", aurora_i18n::tr!("inventory-menu-remove-separator")) {
                                self.profile.layout.remove_separator(self.available, &entry.id);
                                keep = false;
                            }
                        } else if let Some(entry) = entry.filter(|e| e.group()) {
                            let filtered = self.profile.layout.filters.contains(&entry.id);
                            let filter = menu::editable_row(
                                ui,
                                self.p,
                                "filter",
                                if filtered {
                                    aurora_i18n::tr!("inventory-menu-remove-filter")
                                } else {
                                    aurora_i18n::tr!("inventory-menu-add-filter")
                                },
                                true,
                                None,
                                None,
                                false,
                            );
                            #[cfg(test)]
                            self.state.rects.insert("editor:filter".into(), filter.rect);
                            if filter.clicked() {
                                if filtered {
                                    self.profile.layout.filters.remove(&entry.id);
                                    self.state.queries.remove(&entry.id);
                                } else {
                                    self.profile.layout.filters.insert(entry.id.clone());
                                }
                                keep = false;
                            }
                            let direction = match entry.kind {
                                Kind::Group {
                                    direction: Direction::Vertical,
                                    ..
                                } => Direction::Horizontal,
                                _ => Direction::Vertical,
                            };
                            let label = if direction == Direction::Vertical {
                                aurora_i18n::tr!("inventory-menu-open-down")
                            } else {
                                aurora_i18n::tr!("inventory-menu-open-side")
                            };
                            if menu::item(
                                ui,
                                self.p,
                                if direction == Direction::Vertical {
                                    "arrow-down"
                                } else {
                                    "arrow-right"
                                },
                                label,
                            ) {
                                self.profile.layout.directions.insert(entry.id.clone(), direction);
                                keep = false;
                            }
                            if menu::item(ui, self.p, "trash-filled", aurora_i18n::tr!("inventory-menu-delete-group")) {
                                self.profile.layout.remove_container(self.available, &entry.id);
                                keep = false;
                            }
                        } else {
                            menu::section(ui, self.p, aurora_i18n::tr!("inventory-menu-create-group"));
                            for (direction, icon, label) in [
                                (Direction::Vertical, "arrow-down", aurora_i18n::tr!("inventory-menu-vertical")),
                                (Direction::Horizontal, "arrow-right", aurora_i18n::tr!("inventory-menu-horizontal")),
                            ] {
                                let row = menu::editable_row(ui, self.p, icon, label, true, None, None, false);
                                #[cfg(test)]
                                self.state.rects.insert(format!("editor:{direction:?}"), row.rect);
                                if row.clicked() {
                                    editor.stage = Stage::Name(direction);
                                }
                            }
                        }
                        if let Some(entry) = entry.filter(|entry| can_hide(self.resolved, &entry.id)) {
                            let zone = self
                                .resolved
                                .iter()
                                .find_map(|(zone, entries)| entries.iter().any(|e| e.id == entry.id).then_some(zone))
                                .unwrap();
                            let hidden = hidden_zone(self.resolved, zone);
                            let response = menu::editable_row(
                                ui,
                                self.p,
                                if hidden { "eye" } else { "eye-off" },
                                if hidden {
                                    aurora_i18n::tr!("inventory-menu-show-option")
                                } else {
                                    aurora_i18n::tr!("inventory-menu-hide-option")
                                },
                                true,
                                None,
                                None,
                                false,
                            );
                            #[cfg(test)]
                            self.state.rects.insert("editor:visibility".into(), response.rect);
                            if response.clicked() {
                                self.profile
                                    .layout
                                    .move_entry(self.available, &entry.id, if hidden { TEXT } else { HIDDEN }, None, None);
                                keep = false;
                            }
                        }
                        if let Some(entry) = entry.filter(|e| !e.is_separator()) {
                            let zone = self
                                .resolved
                                .iter()
                                .find_map(|(zone, entries)| entries.iter().any(|e| e.id == entry.id).then_some(zone));
                            if let Some(zone) = zone.filter(|z| !bar(z)) {
                                menu::separator(ui, self.p);
                                for (above, label) in [
                                    (true, aurora_i18n::tr!("inventory-menu-separator-above")),
                                    (false, aurora_i18n::tr!("inventory-menu-separator-below")),
                                ] {
                                    if menu::item(ui, self.p, "minus", label) {
                                        let at = self.resolved[zone].iter().position(|e| e.id == entry.id).unwrap_or(0);
                                        let next = if above {
                                            Some(entry.id.as_str())
                                        } else {
                                            self.resolved[zone].get(at + 1).map(|e| e.id.as_str())
                                        };
                                        self.profile.layout.add_separator(self.available, zone, next);
                                        keep = false;
                                    }
                                }
                            }
                        } else if entry.is_none() {
                            menu::separator(ui, self.p);
                            if menu::item(ui, self.p, "minus", aurora_i18n::tr!("inventory-menu-add-separator")) {
                                self.profile.layout.add_separator(self.available, TEXT, None);
                                keep = false;
                            }
                        }
                    }
                }
                self.state.regions.push(ui.min_rect().expand(4.0));
            },
        );
        self.state.remember(before, &self.profile.layout);
        if !keep {
            self.state.keep_open_frame = Some(ui.ctx().cumulative_frame_nr());
            menu::keep_parent_open_this_pass(ui);
        }
        if open && keep {
            self.state.editor = Some(editor);
        }
    }
}

fn keyboard(ui: &Ui, available: &[Entry], profile: &mut Profile, state: &mut State) {
    if profile.locked || state.editor.is_some() || ui.ctx().text_edit_focused() {
        return;
    }
    let modifiers = ui.input(|i| i.modifiers);
    if (modifiers.command || modifiers.ctrl) && ui.input(|i| i.key_pressed(egui::Key::Z)) {
        let (from, to) = if modifiers.shift {
            (&mut state.redo, &mut state.undo)
        } else {
            (&mut state.undo, &mut state.redo)
        };
        if let Some(layout) = from.pop() {
            to.push(std::mem::replace(&mut profile.layout, layout));
        }
        return;
    }
    if modifiers.ctrl && modifiers.alt && ui.input(|i| i.key_pressed(egui::Key::R)) {
        let previous = profile.reset();
        state.queries.clear();
        state.remember(previous, &profile.layout);
        return;
    }
    let Some(selected) = state.selected.as_ref() else {
        return;
    };
    let resolved = profile.layout.resolve(available);
    let Some((zone, entries)) = resolved.iter().find(|(_, entries)| entries.iter().any(|e| &e.id == selected)) else {
        return;
    };
    let at = entries.iter().position(|e| &e.id == selected).unwrap_or(0);
    let previous = profile.layout.clone();
    if ui.input(|i| i.key_pressed(egui::Key::Delete)) && entries[at].is_separator() {
        profile.layout.remove_separator(available, selected);
    }
    if modifiers.alt {
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) && at > 0 {
            profile
                .layout
                .move_entry(available, selected, zone, Some(&entries[at - 1].id), None);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            profile
                .layout
                .move_entry(available, selected, zone, entries.get(at + 2).map(|e| e.id.as_str()), None);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            profile.layout.move_entry(available, selected, TEXT, None, None);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight))
            && let Some(target) = resolved.keys().filter(|zone| bar(zone)).min_by_key(|zone| zone.as_str() != "top")
        {
            profile.layout.move_entry(available, selected, target, None, None);
        }
    }
    state.remember(previous, &profile.layout);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton};

    #[test]
    fn empty_vertical_horizontal_and_icon_groups_only_appear_while_customizing_and_return_when_populated() {
        for direction in [Direction::Vertical, Direction::Horizontal] {
            for icon in [false, true] {
                let mut h = Harness::new();
                h.entries.push(Entry {
                    id: "empty".into(),
                    label: "Empty tools".into(),
                    icon: "folder-filled".into(),
                    enabled: true,
                    checked: None,
                    kind: Kind::Group { direction, open: true },
                    zone: TEXT.into(),
                    slot: None,
                });
                // A stored filter field is not an option inside an empty group.
                h.profile.layout.filters.insert("empty".into());
                if icon {
                    h.profile.layout.move_entry(&h.entries, "empty", "top", None, Some(2));
                }
                let saved = h.profile.layout.clone();
                h.profile.locked = true;
                for _ in 0..4 {
                    h.frame(Vec::new(), Modifiers::NONE);
                }
                assert!(!h.state.rects.contains_key("empty"));
                h.click(LOCK);
                for _ in 0..4 {
                    h.frame(Vec::new(), Modifiers::NONE);
                }
                assert!(!h.profile.locked && h.state.rects.contains_key("empty"));
                assert_eq!(h.profile.layout, saved);
                h.click(LOCK);
                for _ in 0..4 {
                    h.frame(Vec::new(), Modifiers::NONE);
                }
                assert!(h.profile.locked && !h.state.rects.contains_key("empty"));
                // Disabled native commands remain visible; only absence is empty.
                h.profile.layout.placements.insert("disabled".into(), children("empty"));
                for _ in 0..4 {
                    h.frame(Vec::new(), Modifiers::NONE);
                }
                assert!(h.state.rects.contains_key("empty"));
                h.profile.layout.placements.insert("disabled".into(), HIDDEN.into());
                for _ in 0..4 {
                    h.frame(Vec::new(), Modifiers::NONE);
                }
                assert!(!h.state.rects.contains_key("empty"));
                assert!(h.commands.is_empty());
            }
        }
    }

    #[test]
    fn unavailable_native_options_can_be_dragged_from_the_complete_editor_and_reappear_for_another_type() {
        let mut inv = super::super::Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let st = super::super::InventoryUi::default();
        let prefs = super::super::InventoryPreferences::default();
        let facts = super::super::Facts::default();
        let texture = super::super::context_catalog::build(&inv, Uuid::from_u128(8104), &st, &prefs, &facts);
        let animation = super::super::context_catalog::build(&inv, Uuid::from_u128(8101), &st, &prefs, &facts);
        let mut h = Harness::new();
        h.entries = super::super::context_catalog::editing_entries(&texture.entries, &facts);
        for _ in 0..4 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        assert!(h.state.rects.contains_key("play-local"));
        h.click("play-local");
        assert!(h.commands.is_empty() && h.open && !h.profile.locked);
        h.drag("play-local", "slot:top:2");
        let resolved = h.profile.layout.resolve(&h.entries);
        assert_eq!(
            h.profile.layout.icon_slots("top", &resolved["top"])[2].as_deref(),
            Some("play-local")
        );
        h.profile.locked = true;
        h.entries = texture.entries;
        for _ in 0..4 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        assert!(!h.state.rects.contains_key("play-local"));
        h.entries = animation.entries;
        for _ in 0..4 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        assert_eq!(h.state.rects["play-local"], h.state.rects["slot:top:2"]);
        let Kind::Action(index) = h.entries.iter().find(|e| e.id == "play-local").unwrap().kind else {
            panic!()
        };
        h.click("play-local");
        assert_eq!(h.commands, [index]);
    }

    #[test]
    fn filters_can_be_added_typed_cleared_and_removed_in_locked_vertical_and_horizontal_groups() {
        for direction in [Direction::Vertical, Direction::Horizontal] {
            let mut h = Harness::new();
            h.ctx.global_style_mut(|style| style.animation_time = 0.0);
            h.entries.push(Entry {
                id: "tools".into(),
                label: "Tools".into(),
                icon: "folder-filled".into(),
                enabled: true,
                checked: None,
                kind: Kind::Group { direction, open: true },
                zone: TEXT.into(),
                slot: None,
            });
            let zone = children("tools");
            for id in ["open", "rename"] {
                h.profile.layout.placements.insert(id.into(), zone.clone());
            }
            for id in ["leading", "between", "duplicate", "trailing"] {
                h.entries.push(Entry {
                    id: id.into(),
                    label: String::new(),
                    icon: String::new(),
                    enabled: true,
                    checked: None,
                    kind: Kind::Separator,
                    zone: zone.clone(),
                    slot: None,
                });
            }
            h.profile.layout.orders.insert(
                zone,
                ["leading", "open", "between", "duplicate", "rename", "trailing"]
                    .map(str::to_owned)
                    .into(),
            );
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            h.click_at(h.state.rects["tools"].center(), PointerButton::Secondary);
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            h.click("editor:filter");
            assert!(
                h.profile.layout.filters.contains("tools"),
                "filter choice did not apply for {direction:?}"
            );
            h.profile.locked = true;
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            if direction == Direction::Horizontal && !h.state.rects.contains_key("filter:tools") {
                h.click("tools");
            }
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            h.click("filter:tools");
            h.frame(vec![Event::Text("rEnAmE".into())], Modifiers::NONE);
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            assert!(h.open && h.profile.locked);
            assert!(h.state.rects.contains_key("rename"));
            for id in ["open", "leading", "between", "duplicate", "trailing"] {
                assert!(!h.state.rects.contains_key(id), "unexpected filtered row {id}");
            }
            let clear = h.state.rects["filter:tools"].right_center() - Vec2::new(14.0, 0.0);
            h.click_at(clear, PointerButton::Primary);
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            assert!(h.state.rects.contains_key("open") && h.state.rects.contains_key("rename"));
            assert!(h.state.rects.contains_key("duplicate"));
            assert!(
                !h.state.rects.contains_key("between") && !h.state.rects.contains_key("leading") && !h.state.rects.contains_key("trailing")
            );
            h.profile.locked = false;
            h.click_at(h.state.rects["tools"].center(), PointerButton::Secondary);
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            h.click("editor:filter");
            assert!(!h.profile.layout.filters.contains("tools"));
            assert!(!h.state.queries.contains_key("tools"));
            assert!(h.commands.is_empty());
        }
    }

    struct Harness {
        ctx: egui::Context,
        state: State,
        profile: Profile,
        entries: Vec<Entry>,
        open: bool,
        commands: Vec<usize>,
        shapes: Vec<egui::epaint::ClippedShape>,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut entries: Vec<_> = [
                ("open", "Open", true),
                ("rename", "Rename", true),
                ("disabled", "Unavailable", false),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (id, label, enabled))| Entry {
                id: id.into(),
                label: label.into(),
                icon: "copy-filled".into(),
                enabled,
                checked: None,
                kind: Kind::Action(i),
                zone: TEXT.into(),
                slot: None,
            })
            .collect();
            entries.push(super::super::context_catalog::lock_entry());
            let mut h = Self {
                ctx,
                state: State::default(),
                profile: Profile {
                    layout: Layout::default(),
                    open: Default::default(),
                    locked: false,
                },
                entries,
                open: true,
                commands: Vec::new(),
                shapes: Vec::new(),
            };
            for _ in 0..4 {
                h.frame(Vec::new(), Modifiers::NONE);
            }
            h
        }
        fn frame(&mut self, mut events: Vec<Event>, modifiers: Modifiers) {
            self.state.rects.clear();
            events.insert(0, Event::ModifiersChanged(modifiers));
            let p = crate::theme::Theme::default().palette();
            let mut prefs = super::super::menu_layout::Preferences::default();
            prefs.profiles.insert("shared".into(), std::mem::take(&mut self.profile));
            let output = crate::ui::login::test_output(
                self.ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                        focused: events
                            .iter()
                            .rev()
                            .find_map(|event| {
                                if let Event::WindowFocused(focused) = event {
                                    Some(*focused)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(true),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let target = ui.interact(
                            egui::Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(50.0, 24.0)),
                            Id::unique("custom_menu_target"),
                            Sense::click(),
                        );
                        if target.secondary_clicked() {
                            self.open = true;
                        }
                        menu::popup_at(
                            ui.ctx(),
                            Id::unique("custom_menu_test"),
                            Pos2::new(100.0, 70.0),
                            &mut self.open,
                            &p,
                            |ui| {
                                self.state
                                    .prepare(ui.ctx(), Uuid::from_u128(1), ui.layer_id().id, "shared", &mut prefs);
                                if let Some(choice) = show(
                                    ui,
                                    &p,
                                    Uuid::from_u128(1),
                                    "shared",
                                    &self.entries,
                                    prefs.profiles.get_mut("shared").unwrap(),
                                    &mut self.state,
                                ) {
                                    self.commands.push(choice);
                                }
                            },
                        );
                        self.state.finish_inventory_frame(ui.ctx(), &mut prefs);
                    },
                ),
            );
            self.shapes = output.shapes;
            self.profile = prefs.profiles.remove("shared").unwrap();
        }
        fn click_at(&mut self, pos: Pos2, button: PointerButton) {
            for pressed in [true, false] {
                self.frame(
                    vec![
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    Modifiers::NONE,
                );
            }
        }
        fn click(&mut self, id: &str) {
            let pos = self.state.rects[id].center();
            self.click_at(pos, PointerButton::Primary);
        }
        fn drag(&mut self, id: &str, to: &str) {
            let pos = self.state.rects[to].center();
            self.start_drag(id, pos);
            self.release_drag(pos);
        }
        fn start_drag(&mut self, id: &str, pos: Pos2) {
            let from = self.state.rects[id].center();
            self.frame(
                vec![
                    Event::PointerMoved(from),
                    Event::PointerButton {
                        pos: from,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                Modifiers::NONE,
            );
            self.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
            self.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
        }
        fn release_drag(&mut self, pos: Pos2) {
            self.frame(
                vec![Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
        }
        fn key(&mut self, key: egui::Key, modifiers: Modifiers) {
            self.frame(
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                modifiers,
            );
            self.frame(
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                }],
                modifiers,
            );
        }
    }
    #[test]
    fn group_drop_markers_match_before_inside_and_after_for_open_closed_and_nested_groups() {
        fn has_marker(shape: &egui::Shape, rect: egui::Rect, band: i32, color: egui::Color32) -> bool {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_marker(shape, rect, band, color)),
                egui::Shape::LineSegment { points, stroke } if band != 0 && stroke.color == color && stroke.width == 2.0 => {
                    let y = if band < 0 { rect.top() } else { rect.bottom() };
                    points[0].distance(egui::pos2(rect.left(), y)) < 1.0 && points[1].distance(egui::pos2(rect.right(), y)) < 1.0
                }
                egui::Shape::Rect(outline) if band == 0 && outline.stroke.color == color && outline.stroke.width == 1.5 => {
                    outline.rect.min.distance(rect.shrink(1.0).min) < 1.0 && outline.rect.max.distance(rect.shrink(1.0).max) < 1.0
                }
                _ => false,
            }
        }
        for direction in [Direction::Vertical, Direction::Horizontal] {
            for open in [false, true] {
                for nested in [false, true] {
                    for band in [-1, 0, 1] {
                        let mut h = Harness::new();
                        // Compare final painted colors, without the popup's fade-in.
                        h.ctx.global_style_mut(|style| style.animation_time = 0.0);
                        let parent = if nested { children("outer") } else { TEXT.into() };
                        for (id, zone, direction, open) in [
                            ("outer", TEXT, Direction::Vertical, true),
                            ("tools", parent.as_str(), direction, open),
                        ] {
                            if id == "outer" && !nested {
                                continue;
                            }
                            h.entries.push(Entry {
                                id: id.into(),
                                zone: zone.into(),
                                label: id.into(),
                                icon: "folder-filled".into(),
                                enabled: true,
                                checked: None,
                                kind: Kind::Group { direction, open },
                                slot: None,
                            });
                        }
                        for _ in 0..4 {
                            h.frame(Vec::new(), Modifiers::NONE);
                        }
                        let rect = h.state.rects["tools"];
                        let y = match band {
                            -1 => rect.top() + 2.0,
                            1 => rect.bottom() - 2.0,
                            _ => rect.center().y,
                        };
                        let pos = egui::pos2(rect.center().x, y);
                        h.start_drag("rename", pos);
                        let color = crate::theme::Theme::default().palette().violet_light;
                        // Hover-opening the body can widen its parent menu.
                        let mut marker = h.state.rects["tools"];
                        if band > 0
                            && let Some(body) = h.state.rects.get("group-body:tools")
                        {
                            marker.max.y = marker.max.y.max(body.bottom());
                        }
                        assert!(
                            h.shapes.iter().any(|shape| has_marker(&shape.shape, marker, band, color)),
                            "wrong drop indicator: direction={direction:?}, open={open}, nested={nested}, band={band}, expected={marker:?}, current={:?}, payload={:?}, shapes={:?}",
                            h.state.rects.get("tools"),
                            egui::DragAndDrop::payload::<Dragged>(&h.ctx).map(|drag| drag.entry.clone()),
                            h.shapes
                                .iter()
                                .filter(|shape| match &shape.shape {
                                    egui::Shape::LineSegment { stroke, .. } => stroke.color == color && stroke.width == 2.0,
                                    egui::Shape::Rect(rect) => rect.stroke.color == color && rect.stroke.width == 1.5,
                                    _ => false,
                                })
                                .collect::<Vec<_>>()
                        );
                        if direction == Direction::Vertical && band != 0 {
                            assert_eq!(
                                h.profile.open.get("tools").copied().unwrap_or(open),
                                open,
                                "edge insertion must not expand the group"
                            );
                        }
                        h.release_drag(pos);
                        h.frame(Vec::new(), Modifiers::NONE);
                        let destination = if band == 0 { children("tools") } else { parent };
                        assert_eq!(h.profile.layout.placements["rename"], destination);
                        if band != 0 {
                            let resolved = h.profile.layout.resolve(&h.entries);
                            let rows = &resolved[&destination];
                            let at = rows.iter().position(|entry| entry.id == "rename").unwrap();
                            let group = rows.iter().position(|entry| entry.id == "tools").unwrap();
                            assert_eq!(at < group, band < 0);
                        }
                        assert!(h.commands.is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn native_drag_converts_text_to_an_absolute_icon_cell_without_running_the_command() {
        let mut h = Harness::new();
        h.drag("rename", "slot:top:2");
        assert!(h.commands.is_empty());
        assert!(h.open);
        assert_eq!(h.profile.layout.placements.get("rename").map(String::as_str), Some("top"));
        assert_eq!(
            h.profile.layout.icon_slots("top", &h.profile.layout.resolve(&h.entries)["top"])[2].as_deref(),
            Some("rename")
        );
        h.frame(Vec::new(), Modifiers::NONE);
        h.drag("rename", "open");
        assert_eq!(h.profile.layout.placements.get("rename").map(String::as_str), Some(TEXT));
        assert!(h.commands.is_empty());
    }
    #[test]
    fn unavailable_commands_are_draggable_but_never_executed() {
        let mut h = Harness::new();
        h.click("disabled");
        assert!(h.commands.is_empty());
        h.drag("disabled", "slot:bottom:6");
        assert_eq!(h.profile.layout.placements.get("disabled").map(String::as_str), Some("bottom"));
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("disabled");
        assert!(h.commands.is_empty());
    }
    #[test]
    fn the_padlock_blocks_drag_edit_reset_and_shortcuts_but_preserves_normal_actions() {
        let mut h = Harness::new();
        h.drag("rename", "slot:top:3");
        h.frame(Vec::new(), Modifiers::NONE);
        let saved = h.profile.layout.clone();
        h.click(LOCK);
        assert!(h.profile.locked);
        h.drag("rename", "slot:top:6");
        assert!(!h.state.rects.contains_key("footer:reset"));
        assert!(!h.state.rects.contains_key("footer:create"));
        let pos = h.state.rects["open"].center();
        h.click_at(pos, PointerButton::Secondary);
        assert!(h.state.editor.is_none());
        h.key(egui::Key::Z, Modifiers::CTRL);
        h.key(egui::Key::ArrowRight, Modifiers::ALT);
        assert_eq!(h.profile.layout, saved);
        h.click("open");
        assert_eq!(h.commands, [0]);
        h.click(LOCK);
        assert!(!h.profile.locked);
        assert_eq!(h.profile.layout, saved);
    }
    #[test]
    fn right_click_twice_without_moving_closes_only_the_latest_panel() {
        let mut h = Harness::new();
        let pos = h.state.rects["open"].center();
        h.click_at(pos, PointerButton::Secondary);
        assert!(h.state.editor.is_some());
        assert!(h.open);
        h.click_at(pos, PointerButton::Secondary);
        assert!(h.state.editor.is_none());
        assert!(h.open);
        h.frame(vec![Event::PointerMoved(pos + Vec2::new(8.0, 0.0))], Modifiers::NONE);
        h.click_at(pos, PointerButton::Secondary);
        assert!(h.state.editor.is_some());
        assert!(h.open);
    }
    #[test]
    fn group_open_state_is_saved_even_while_the_layout_is_locked() {
        let mut h = Harness::new();
        h.entries.push(Entry {
            id: "organize".into(),
            label: "Organize".into(),
            icon: "folder-filled".into(),
            enabled: true,
            checked: None,
            kind: Kind::Group {
                direction: Direction::Vertical,
                open: false,
            },
            zone: TEXT.into(),
            slot: None,
        });
        h.profile.layout.placements.insert("rename".into(), children("organize"));
        h.profile.locked = true;
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("organize");
        assert_eq!(h.profile.open.get("organize"), Some(&true));
        assert!(h.profile.locked);
        h.click("organize");
        assert_eq!(h.profile.open.get("organize"), Some(&false));
    }

    #[test]
    fn native_menu_reset_restores_requested_groups_order_and_expansion_without_executing_actions() {
        let mut inv = super::super::Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let catalog = super::super::context_catalog::build(
            &inv,
            Uuid::from_u128(8104),
            &super::super::InventoryUi::default(),
            &super::super::InventoryPreferences::default(),
            &super::super::Facts::default(),
        );
        let mut h = Harness::new();
        h.entries = catalog
            .entries
            .into_iter()
            .filter(|e| ["image", "move", "asset-uuid", "attachments", "organize", "rename", LOCK].contains(&e.id.as_str()))
            .collect();
        h.profile.layout.placements.insert("move".into(), HIDDEN.into());
        h.profile.open.insert("organize".into(), false);
        for _ in 0..4 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        h.click("footer:reset");
        for _ in 0..4 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        let resolved = h.profile.layout.resolve(&h.entries);
        assert_eq!(
            resolved[&children("organize")].iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["image", "move"]
        );
        let root: Vec<_> = resolved[TEXT].iter().map(|e| e.id.as_str()).collect();
        assert!(root.iter().position(|id| *id == "asset-uuid").unwrap() < root.iter().position(|id| *id == "organize").unwrap());
        assert_eq!(h.profile.open.get("organize"), Some(&true));
        assert_eq!(h.profile.open.get("attachments"), Some(&true));
        assert!(h.state.rects["image"].top() > h.state.rects["organize"].top());
        assert!(h.state.rects["move"].top() > h.state.rects["image"].top());
        assert!(!h.profile.locked && h.commands.is_empty());
    }

    #[test]
    fn layout_undo_redo_and_reset_leave_commands_and_lock_state_alone() {
        let mut h = Harness::new();
        h.drag("rename", "slot:top:2");
        let changed = h.profile.layout.clone();
        assert_ne!(changed, Layout::default());
        h.key(egui::Key::Z, Modifiers::CTRL);
        assert_eq!(h.profile.layout, Layout::default());
        h.key(
            egui::Key::Z,
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::NONE
            },
        );
        assert_eq!(h.profile.layout, changed);
        h.click("footer:reset");
        assert_eq!(h.profile.layout, Profile::default().layout);
        assert!(!h.profile.locked);
        assert!(h.commands.is_empty());
    }

    #[test]
    fn escape_dismisses_the_edit_panel_without_closing_the_parent_menu() {
        let mut h = Harness::new();
        let pos = h.state.rects["open"].center();
        h.click_at(pos, PointerButton::Secondary);
        assert!(h.state.editor.is_some());
        h.key(egui::Key::Escape, Modifiers::NONE);
        assert!(h.state.editor.is_none());
        assert!(h.open);
    }

    #[test]
    fn opening_with_a_right_click_then_repeating_it_at_the_same_position_closes_the_root() {
        let mut h = Harness::new();
        let pos = h.state.rects["open"].center();
        // As when a row opens a new popup: the editor first sees its release.
        h.frame(
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Secondary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            Modifiers::NONE,
        );
        h.state.root = None;
        h.frame(
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            Modifiers::NONE,
        );
        assert!(h.state.editor.is_none());
        assert!(h.open);
        h.click_at(pos, PointerButton::Secondary);
        assert!(!h.open);
    }

    #[test]
    fn adding_moving_and_removing_an_icon_bar_keeps_its_commands_and_supports_undo() {
        let mut h = Harness::new();
        h.click("settings:add-bar");
        let id = h.profile.layout.icon_bars.iter().next().unwrap().clone();
        let zone = icon_zone(&id);
        h.frame(Vec::new(), Modifiers::NONE);
        for slot in 0..ICON_SLOTS {
            assert!(h.state.rects.contains_key(&format!("slot:{zone}:{slot}")));
        }
        h.drag("rename", &format!("slot:{zone}:7"));
        h.frame(Vec::new(), Modifiers::NONE);
        assert_eq!(
            h.profile.layout.icon_slots(&zone, &h.profile.layout.resolve(&h.entries)[&zone])[7].as_deref(),
            Some("rename")
        );
        h.drag(&id, "open");
        h.frame(Vec::new(), Modifiers::NONE);
        let root = &h.profile.layout.resolve(&h.entries)[TEXT];
        assert!(root.iter().position(|e| e.id == id).unwrap() < root.iter().position(|e| e.id == "open").unwrap());
        let saved = h.profile.layout.clone();
        h.click(&format!("remove:{id}"));
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.profile.layout.icon_bars.contains(&id));
        assert_eq!(h.profile.layout.placements["rename"], TEXT);
        assert!(h.commands.is_empty());
        h.key(egui::Key::Z, Modifiers::CTRL);
        assert_eq!(h.profile.layout, saved);
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("footer:reset");
        h.frame(Vec::new(), Modifiers::NONE);
        assert_eq!(h.profile.layout, Profile::default().layout);
        assert!(h.state.rects.contains_key("slot:top:7"));
        assert!(h.state.rects.contains_key("slot:bottom:7"));
    }

    #[test]
    fn dragging_the_last_icon_out_removes_its_bar_and_undo_restores_the_bar_and_cell() {
        let mut h = Harness::new();
        h.click("settings:add-bar");
        let id = h.profile.layout.icon_bars.iter().next().unwrap().clone();
        let zone = icon_zone(&id);
        h.frame(Vec::new(), Modifiers::NONE);
        h.drag("rename", &format!("slot:{zone}:7"));
        h.frame(Vec::new(), Modifiers::NONE);
        h.drag("rename", &format!("slot:{zone}:3"));
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(h.profile.layout.icon_bars.contains(&id));
        let saved = h.profile.layout.clone();
        h.drag("rename", "hidden:drop");
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.profile.layout.icon_bars.contains(&id));
        assert!(!h.state.rects.contains_key(&format!("slot:{zone}:3")));
        assert_eq!(h.profile.layout.placements["rename"], HIDDEN);
        h.key(egui::Key::Z, Modifiers::CTRL);
        h.frame(Vec::new(), Modifiers::NONE);
        assert_eq!(h.profile.layout, saved);
        assert_eq!(
            h.profile.layout.icon_slots(&zone, &h.profile.layout.resolve(&h.entries)[&zone])[3].as_deref(),
            Some("rename")
        );
        assert!(h.state.rects.contains_key(&format!("slot:{zone}:3")));
        h.key(
            egui::Key::Z,
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::NONE
            },
        );
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.profile.layout.icon_bars.contains(&id));
        assert!(h.open);
        assert!(!h.profile.locked);
        assert!(h.commands.is_empty());
    }

    #[test]
    fn the_lock_can_be_dragged_between_icons_and_text_and_still_toggles_without_running_commands() {
        let mut h = Harness::new();
        assert_eq!(h.state.rects[LOCK], h.state.rects["slot:bottom:0"]);
        h.drag(LOCK, "open");
        h.frame(Vec::new(), Modifiers::NONE);
        assert_eq!(h.profile.layout.placements[LOCK], TEXT);
        assert!(!h.state.rects.contains_key("slot:bottom:0"), "emptying the old bar must remove it");
        let saved = h.profile.layout.clone();
        h.click(LOCK);
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(h.profile.locked);
        assert!(!h.state.rects.contains_key("hidden:drop"));
        assert_eq!(h.profile.layout, saved);
        h.click(LOCK);
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.profile.locked);
        assert!(h.state.rects.contains_key("hidden:drop"));
        h.drag(LOCK, "slot:top:3");
        h.frame(Vec::new(), Modifiers::NONE);
        assert_eq!(h.state.rects[LOCK], h.state.rects["slot:top:3"]);
        h.profile = serde_json::from_str(&serde_json::to_string(&h.profile).unwrap()).unwrap();
        let saved = h.profile.layout.clone();
        h.click(LOCK);
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(h.profile.locked);
        h.click(LOCK);
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.profile.locked);
        assert_eq!(h.profile.layout, saved);
        assert!(h.commands.is_empty());
        h.click("settings:finish");
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("open");
        assert_eq!(h.commands, [0]);
    }

    #[test]
    fn dragging_or_right_clicking_cannot_hide_the_lock_or_its_bar() {
        let mut h = Harness::new();
        let saved = h.profile.layout.clone();
        for id in [LOCK, "bar:bottom"] {
            h.drag(id, "hidden:drop");
            h.frame(Vec::new(), Modifiers::NONE);
            assert_eq!(h.profile.layout, saved);
            let pos = h.state.rects[id].center();
            h.click_at(pos, PointerButton::Secondary);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.state.editor.is_some());
            assert!(!h.state.rects.contains_key("editor:visibility"));
            h.key(egui::Key::Escape, Modifiers::NONE);
            h.frame(Vec::new(), Modifiers::NONE);
        }
        assert!(h.state.rects.contains_key(LOCK));
        assert!(h.commands.is_empty());
    }

    #[test]
    fn default_icon_bars_can_be_removed_without_losing_commands_or_leaving_controls_while_locked() {
        let mut h = Harness::new();
        h.drag("open", "slot:top:1");
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("remove:bar:top");
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.state.rects.contains_key("slot:top:0"));
        assert_eq!(h.profile.layout.placements["open"], TEXT);
        let pos = h.state.rects["bar:bottom"].center();
        h.click_at(pos, PointerButton::Secondary);
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("editor:remove-bar");
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.state.rects.contains_key("slot:bottom:0"));
        assert!(h.open);
        assert!(h.commands.is_empty());
        h.click(LOCK);
        h.frame(Vec::new(), Modifiers::NONE);
        assert!(!h.state.rects.contains_key("settings:add-bar"));
        assert!(!h.state.rects.keys().any(|id| id.starts_with("remove:bar:")));
        h.click("open");
        assert_eq!(h.commands, [0]);
    }

    #[test]
    fn menu_width_stabilizes_and_both_toolbars_keep_eight_cells() {
        let mut h = Harness::new();
        let width = h.state.rects["slot:bottom:7"].right() - h.state.rects[LOCK].left();
        assert!(width <= WIDTH + 1.0, "unused popup room inflated the menu to {width}");
        for _ in 0..30 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        assert!((h.state.rects["slot:bottom:7"].right() - h.state.rects[LOCK].left() - width).abs() < 1.0);
        for bar in ["top", "bottom"] {
            let cells: Vec<_> = (0..8).map(|i| h.state.rects[&format!("slot:{bar}:{i}")]).collect();
            assert!((cells[7].right() - cells[0].left() - width).abs() < 1.0);
            assert!(cells.windows(2).all(|w| (w[0].right() - w[1].left()).abs() < 0.1));
        }
        let bar_width = h.state.rects["remove:bar:top"].right() - h.state.rects["bar:top"].left();
        for _ in 0..6 {
            h.click(LOCK);
            h.frame(Vec::new(), Modifiers::NONE);
            let locked_width = h.state.rects["slot:top:7"].right() - h.state.rects["slot:top:0"].left();
            assert!((locked_width - bar_width).abs() < 1.0);
            h.click(LOCK);
            h.frame(Vec::new(), Modifiers::NONE);
            let editing_width = h.state.rects["remove:bar:top"].right() - h.state.rects["bar:top"].left();
            assert!(
                (editing_width - bar_width).abs() < 1.0,
                "toggling editing inflated the bar to {editing_width}"
            );
        }
    }

    #[test]
    fn unlocking_opens_settings_and_blocks_commands_until_relocked() {
        let mut h = Harness::new();
        assert!(h.state.rects.contains_key("footer:create"));
        assert!(h.state.rects.contains_key("footer:reset"));
        h.click("open");
        assert!(h.commands.is_empty(), "editing must not execute inventory commands");
        let lock = h.state.rects[LOCK];
        let trash = h.state.rects["slot:bottom:7"];
        assert!(lock.right() < trash.left());
        h.click(LOCK);
        assert!(h.profile.locked);
        assert!(!h.state.rects.contains_key("hidden:drop"));
        assert!(!h.state.rects.contains_key("footer:create"));
        h.click("open");
        assert_eq!(h.commands, [0]);
        h.click(LOCK);
        assert!(h.state.rects.contains_key("hidden:drop"));
        assert!(!h.profile.locked);
    }

    #[test]
    fn hidden_options_can_be_dragged_back_and_cannot_execute_in_settings() {
        let mut h = Harness::new();
        h.drag("rename", "hidden:drop");
        assert_eq!(h.profile.layout.placements["rename"], HIDDEN);
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("rename");
        assert!(h.commands.is_empty());
        let saved = h.profile.layout.clone();
        h.click(LOCK);
        assert!(!h.state.rects.contains_key("rename"));
        // The organizing controls disappear; allow the cells to settle.
        h.frame(Vec::new(), Modifiers::NONE);
        h.click(LOCK);
        assert_eq!(h.profile.layout, saved);
        h.frame(Vec::new(), Modifiers::NONE);
        h.drag("rename", "open");
        assert_eq!(h.profile.layout.placements["rename"], TEXT);
        assert!(h.commands.is_empty());
    }

    #[test]
    fn clicking_outside_both_editing_surfaces_relocks_and_allows_another_context_menu() {
        for button in [PointerButton::Primary, PointerButton::Secondary] {
            let mut h = Harness::new();
            h.drag("rename", "hidden:drop");
            let saved = h.profile.layout.clone();
            h.click_at(Pos2::new(900.0, 700.0), button);
            assert!(!h.open);
            assert!(h.profile.locked);
            assert_eq!(h.profile.layout, saved);
            h.click_at(Pos2::new(40.0, 30.0), PointerButton::Secondary);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.open);
            assert!(!h.state.rects.contains_key("hidden:drop"));
            h.click("open");
            assert_eq!(h.commands, [0]);
        }
    }

    #[test]
    fn window_focus_loss_closes_the_name_editor_relocks_and_restores_right_clicks() {
        for regained_before_redraw in [false, true] {
            let mut h = Harness::new();
            h.drag("rename", "hidden:drop");
            let saved = h.profile.layout.clone();
            h.click("footer:create");
            h.frame(Vec::new(), Modifiers::NONE);
            h.click("editor:Vertical");
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.ctx.text_edit_focused());
            let mut events = vec![Event::WindowFocused(false)];
            if regained_before_redraw {
                events.push(Event::WindowFocused(true));
            }
            h.frame(events, Modifiers::NONE);
            assert!(!h.open);
            assert!(h.profile.locked);
            assert!(h.state.editor.is_none());
            assert_eq!(h.profile.layout, saved);
            assert!(h.commands.is_empty());
            h.frame(vec![Event::WindowFocused(true)], Modifiers::NONE);
            h.click_at(Pos2::new(40.0, 30.0), PointerButton::Secondary);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.open);
            assert!(!h.state.rects.contains_key("hidden:drop"));
            h.click("open");
            assert_eq!(h.commands, [0]);
        }
    }

    #[test]
    fn a_disappearing_popup_relocks_and_cancels_an_unfinished_drag() {
        for focused in [true, false] {
            let mut h = Harness::new();
            h.drag("rename", "hidden:drop");
            h.frame(Vec::new(), Modifiers::NONE);
            let saved = h.profile.layout.clone();
            let from = h.state.rects["open"].center();
            h.frame(
                vec![
                    Event::PointerMoved(from),
                    Event::PointerButton {
                        pos: from,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                Modifiers::NONE,
            );
            h.frame(vec![Event::PointerMoved(from + Vec2::new(15.0, 5.0))], Modifiers::NONE);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(egui::DragAndDrop::payload::<Dragged>(&h.ctx).is_some());
            h.open = false;
            h.frame(vec![Event::WindowFocused(focused)], Modifiers::NONE);
            assert!(h.profile.locked);
            assert!(egui::DragAndDrop::payload::<Dragged>(&h.ctx).is_none());
            assert_eq!(h.profile.layout, saved);
            assert!(h.commands.is_empty());
            h.frame(
                vec![
                    Event::WindowFocused(true),
                    Event::PointerButton {
                        pos: from,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
                Modifiers::NONE,
            );
            h.click_at(Pos2::new(40.0, 30.0), PointerButton::Secondary);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.open);
            h.click("open");
            assert_eq!(h.commands, [0]);
        }
    }

    #[test]
    fn closing_or_finishing_the_organization_panel_relocks_without_losing_the_layout() {
        for button in ["settings:close", "settings:finish"] {
            let mut h = Harness::new();
            h.drag("rename", "hidden:drop");
            h.frame(Vec::new(), Modifiers::NONE);
            let saved = h.profile.layout.clone();
            h.click(button);
            h.frame(Vec::new(), Modifiers::NONE);
            assert!(h.profile.locked);
            assert!(h.open);
            assert_eq!(h.profile.layout, saved);
            assert!(!h.state.rects.contains_key("hidden:drop"));
            h.click("open");
            assert_eq!(h.commands, [0]);
        }
    }

    #[test]
    fn icon_submenus_require_a_click_in_both_bars_and_in_both_layout_modes() {
        for (zone, id) in [("top", "new"), ("bottom", "new-clothing")] {
            for locked in [true, false] {
                let mut h = Harness::new();
                h.profile.locked = locked;
                h.entries.push(Entry {
                    id: id.into(),
                    label: "New...".into(),
                    icon: "plus".into(),
                    enabled: true,
                    checked: None,
                    kind: Kind::Group {
                        direction: Direction::Horizontal,
                        open: false,
                    },
                    zone: zone.into(),
                    slot: Some(0),
                });
                h.profile.layout.placements.insert("rename".into(), children(id));
                h.frame(Vec::new(), Modifiers::NONE);
                let pos = h.state.rects[id].center();
                for _ in 0..30 {
                    h.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
                }
                assert!(!h.state.rects.contains_key("rename"), "hover opened {zone}, locked={locked}");
                h.click(id);
                h.frame(Vec::new(), Modifiers::NONE);
                assert!(h.state.rects.contains_key("rename"), "click did not open {zone}, locked={locked}");
                assert!(h.commands.is_empty());
                h.click(id);
                assert!(
                    !h.state.rects.contains_key("rename"),
                    "second click did not close {zone}, locked={locked}"
                );
                h.key(egui::Key::ArrowRight, Modifiers::NONE);
                h.frame(Vec::new(), Modifiers::NONE);
                assert!(
                    h.state.rects.contains_key("rename"),
                    "keyboard did not open {zone}, locked={locked}"
                );
                h.click(id);
                h.click(id);
                h.frame(Vec::new(), Modifiers::NONE);
                h.click("rename");
                assert_eq!(h.commands, if locked { vec![1] } else { vec![] });
            }
        }
    }

    #[test]
    fn horizontal_groups_stay_open_while_editing_with_the_companion_visible() {
        let mut h = Harness::new();
        h.entries.push(Entry {
            id: "actions".into(),
            label: "Actions".into(),
            icon: "folder-filled".into(),
            enabled: true,
            checked: None,
            kind: Kind::Group {
                direction: Direction::Horizontal,
                open: false,
            },
            zone: TEXT.into(),
            slot: None,
        });
        h.profile.layout.placements.insert("rename".into(), children("actions"));
        h.frame(Vec::new(), Modifiers::NONE);
        let pos = h.state.rects["actions"].center();
        for _ in 0..30 {
            h.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
        }
        let child = h.state.rects["rename"].center();
        for _ in 0..4 {
            h.frame(vec![Event::PointerMoved(child)], Modifiers::NONE);
        }
        assert!(h.state.rects.contains_key("rename"));
        assert!(h.state.rects.contains_key("hidden:drop"));
        h.click("rename");
        assert!(h.commands.is_empty());
        assert!(h.open);
    }

    #[test]
    fn the_bottom_bar_trash_is_editable_and_only_executes_when_locked() {
        let mut h = Harness::new();
        h.entries.push(Entry {
            id: "delete".into(),
            label: "Delete".into(),
            icon: "trash-filled".into(),
            enabled: true,
            checked: None,
            kind: Kind::Action(3),
            zone: "bottom".into(),
            slot: Some(7),
        });
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("slot:bottom:7");
        assert!(h.commands.is_empty());
        h.drag("delete", "hidden:drop");
        assert_eq!(h.profile.layout.placements["delete"], HIDDEN);
        h.frame(Vec::new(), Modifiers::NONE);
        h.drag("delete", "slot:bottom:7");
        assert_eq!(h.profile.layout.placements["delete"], "bottom");
        h.click(LOCK);
        // Relocking removes the editing controls and moves the bottom cells.
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("slot:bottom:7");
        assert_eq!(h.commands, [3]);
    }

    #[test]
    fn right_click_visibility_action_restores_a_hidden_command() {
        let mut h = Harness::new();
        let pos = h.state.rects["rename"].center();
        h.click_at(pos, PointerButton::Secondary);
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("editor:visibility");
        assert_eq!(h.profile.layout.placements["rename"], HIDDEN);
        h.frame(Vec::new(), Modifiers::NONE);
        let pos = h.state.rects["rename"].center();
        h.click_at(pos, PointerButton::Secondary);
        h.frame(Vec::new(), Modifiers::NONE);
        h.click("editor:visibility");
        assert_eq!(h.profile.layout.placements["rename"], TEXT);
        assert!(h.commands.is_empty());
    }

    #[test]
    fn settings_create_a_named_empty_group_without_dismissing_the_menu() {
        let mut h = Harness::new();
        h.click("footer:create");
        for _ in 0..3 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        h.click("editor:Vertical");
        assert!(matches!(
            h.state.editor.as_ref().map(|e| &e.stage),
            Some(Stage::Name(Direction::Vertical))
        ));
        h.frame(Vec::new(), Modifiers::NONE);
        h.frame(vec![Event::Text("Personal tools".into())], Modifiers::NONE);
        h.click("editor:create");
        assert!(h.open);
        assert!(h.state.editor.is_none());
        assert!(h.commands.is_empty());
        let group = h.profile.layout.groups.values().next().unwrap();
        assert_eq!(group.name, "Personal tools");
        assert!(group.keep_empty);
    }

    #[test]
    fn repeating_a_right_click_in_a_child_panel_outside_the_root_keeps_the_root_open() {
        let mut h = Harness::new();
        let pos = Pos2::new(600.0, 120.0);
        h.state.edit(Some("open".into()), pos);
        for _ in 0..3 {
            h.frame(Vec::new(), Modifiers::NONE);
        }
        h.click_at(pos + Vec2::new(8.0, 8.0), PointerButton::Secondary);
        assert!(h.open);
        assert!(h.state.editor.is_some());
        h.click_at(pos + Vec2::new(8.0, 8.0), PointerButton::Secondary);
        assert!(h.open);
        assert!(h.state.editor.is_none());
    }
}
