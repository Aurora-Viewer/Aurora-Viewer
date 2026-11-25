//! Selection and local moves share the same hit-testing in every folder view.
use super::super::{InventoryPreferences, drag, folder_draggable};
use super::{ExplorerState, Inventory, Location, Palette, Preferences, Row, Uuid, model, place_name, selection_edge};
use egui::{Context, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Response, Sense, Ui, pos2, vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    pub(super) caret: Option<Row>,
    press: Option<(Row, Modifiers, bool)>,
    focus: Option<egui::Id>,
    visible: Vec<(Row, Rect, Option<Rect>)>,
    headers: Vec<(Rect, egui::Id)>,
    pub(super) viewport: Option<Rect>,
    offset: egui::Vec2,
    pub(super) columns: usize,
    pub(super) column_height: usize,
    marquee: Option<Marquee>,
}

impl State {
    pub(super) fn cancel_pointer(&mut self) {
        self.press = None;
        self.marquee = None;
    }
}

struct Marquee {
    origin: Pos2,
    start_screen: Pos2,
    initial_row: bool,
    moved: bool,
    modifiers: Modifiers,
    initial: HashSet<Row>,
    // Virtualized rows keep content-space geometry while scrolling during this gesture.
    seen: HashMap<Row, Rect>,
}

// Custom folders from older layouts have no SL inventory ID. A selection
// containing one remains local in its entirety, including its real entries.
pub(super) struct LocalDrag {
    root: Uuid,
    ids: Vec<Uuid>,
    custom: Vec<Uuid>,
}

#[derive(Clone)]
pub(super) enum Moving {
    Inventory(drag::Cargo),
    Local(Arc<LocalDrag>),
}

impl Moving {
    pub(super) fn carried(ctx: &Context) -> Option<Self> {
        drag::Cargo::carried(ctx)
            .map(Self::Inventory)
            .or_else(|| egui::DragAndDrop::payload::<LocalDrag>(ctx).map(Self::Local))
    }

    pub(super) fn entries(&self) -> (&[Uuid], &[Uuid]) {
        match self {
            Self::Inventory(cargo) => (cargo.ids(), &[]),
            Self::Local(cargo) => (&cargo.ids, &cargo.custom),
        }
    }

    pub(super) fn same_drag(&self, other: &Self) -> bool {
        match (self, other) {
            // The cached Arc keeps this payload alive until the next check.
            (Self::Inventory(a), Self::Inventory(b)) => std::ptr::eq(a.ids(), b.ids()),
            (Self::Local(a), Self::Local(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[derive(Clone)]
struct CheckedMove {
    drag: Moving,
    key: (Uuid, u64, u64, Location),
    allowed: bool,
}

pub(super) fn dragging(ctx: &Context) -> bool {
    Moving::carried(ctx).is_some()
}

pub(super) fn drag_count(ctx: &Context) -> Option<usize> {
    egui::DragAndDrop::payload::<LocalDrag>(ctx).map(|m| m.ids.len() + m.custom.len())
}

fn press_modifiers(ui: &Ui) -> Modifiers {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(i.modifiers)
    })
}

fn focus(ui: &Ui, state: &mut ExplorerState) {
    if let Some(id) = state.input.focus {
        ui.memory_mut(|m| m.request_focus(id));
        state.list_focus = Some(id);
    }
}

pub(super) fn begin(state: &mut ExplorerState) {
    state.input.visible.clear();
    state.input.headers.clear();
    state.input.viewport = None;
}

pub(super) fn section_header(state: &mut ExplorerState, rect: Rect, id: egui::Id) {
    if rect.is_positive() {
        state.input.headers.push((rect, id));
    }
}

pub(super) fn over_header(state: &ExplorerState, pos: Pos2) -> bool {
    state.input.headers.iter().any(|(r, _)| r.contains(pos))
}

pub(super) fn header_focused(ui: &Ui, state: &ExplorerState) -> bool {
    ui.memory(|m| m.focused())
        .is_some_and(|focus| state.input.headers.iter().any(|(_, id)| *id == focus))
}

pub(super) fn retain_rows(input: &mut State, rows: &[Row]) {
    if input.press.is_some_and(|(row, _, _)| !rows.contains(&row)) {
        input.press = None;
    }
    if let Some(marquee) = &mut input.marquee {
        let visible: HashSet<_> = rows.iter().copied().collect();
        marquee.initial.retain(|r| visible.contains(r));
        // Reordering can move every stored rectangle even when all IDs survive.
        marquee.seen.clear();
    }
}

pub(super) fn surface(ui: &mut Ui, viewport: Rect, offset: egui::Vec2, state: &mut ExplorerState) -> Response {
    let viewport = viewport.intersect(ui.clip_rect());
    state.input.viewport = Some(viewport);
    state.input.offset = offset;
    let id = ui.make_persistent_id("explorer-selection-surface");
    state.input.focus = Some(id);
    ui.ctx()
        .data_mut(|d| d.insert_temp(ui.layer_id().id.with("explorer-active-list-focus"), id));
    // Register first so actual rows, scroll bars and menus receive their own hits.
    let response = ui.interact(viewport, id, Sense::click_and_drag());
    // A double-click, sidebar visit or tab switch makes the destination usable for Ctrl+V immediately.
    // Leave active editors, modal dialogs and another inventory window's keyboard alone.
    if std::mem::take(&mut state.focus_contents)
        && crate::ui::close_key::frontmost_window(ui.ctx()) == Some(ui.layer_id())
        && !ui.ctx().text_edit_focused()
        && !egui::Popup::is_any_open(ui.ctx())
        && !ui.memory(|m| m.top_modal_layer().is_some())
    {
        focus(ui, state);
    }
    response
}

pub(super) fn selecting(state: &ExplorerState) -> bool {
    state.input.marquee.as_ref().is_some_and(|m| m.moved)
}

pub(super) fn row(
    ui: &mut Ui,
    response: &Response,
    row: Row,
    prefs: &InventoryPreferences,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    open: &mut Option<Row>,
    gutter: Option<Rect>,
) -> bool {
    let mut started = false;
    state.input.visible.push((row, response.rect, gutter));
    let origin = ui.input(|i| i.pointer.press_origin());
    let gutter_press = origin.is_some_and(|pos| gutter.is_some_and(|rect| rect.contains(pos)));
    if response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
        let modifiers = press_modifiers(ui);
        let defer = gutter_press || (state.here.selected.contains(&row) && !modifiers.shift);
        state.input.press = Some((row, modifiers, defer));
        if !defer {
            state.select(row, modifiers);
        }
        focus(ui, state);
    }
    if response.clicked_by(PointerButton::Primary) && !drag::dragged_this_press(ui.ctx()) {
        match state.input.press {
            Some((pressed, modifiers, deferred)) if pressed == row => {
                if deferred {
                    state.select(row, modifiers);
                }
            }
            _ => state.select(row, ui.input(|i| i.modifiers)),
        }
        focus(ui, state);
    }
    if response.secondary_clicked() && !state.here.selected.contains(&row) {
        state.select(row, Modifiers::NONE);
        focus(ui, state);
    }
    if response.double_clicked_by(PointerButton::Primary) && !drag::dragged_this_press(ui.ctx()) {
        *open = Some(row);
    }
    if !gutter_press && drag::begins(ui, response) {
        if state.on_dashboard() && !matches!(row, Row::Place(Location::Folder(id)) if layout.is_home_child(inv, id)) {
            return false;
        }
        if !state.here.selected.contains(&row) {
            state.select(row, Modifiers::NONE);
        }
        if state.dashboard_shortcuts(inv, layout) {
            return false;
        }
        focus(ui, state);
        let mut ids = Vec::new();
        let mut custom = Vec::new();
        let mut local_only = false;
        for selected in &state.here.selected {
            match *selected {
                Row::Place(Location::Custom(id)) if !state.here.raw => custom.push(id),
                Row::Item(id) if inv.items.contains_key(&id) => ids.push(id),
                Row::Place(Location::Folder(id)) if folder_draggable(inv, id, prefs) => ids.push(id),
                Row::Place(Location::Folder(id)) if !state.here.raw && model::can_organize(inv, id) => {
                    ids.push(id);
                    local_only = true;
                }
                _ => {
                    state.status = aurora_i18n::tr!("explorer-input-protected").into();
                    return false;
                }
            }
        }
        started = true;
        state.input.marquee = None;
        ids.sort_unstable();
        if custom.is_empty() && !local_only {
            drag::carry(ui.ctx(), ids, matches!(row, Row::Place(Location::Folder(_))));
        } else {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                LocalDrag {
                    root: inv.root,
                    ids,
                    custom,
                },
            );
        }
    }
    match row {
        Row::Place(place) if !state.here.raw => {
            if receive(response, inv, layout, place) {
                state.status = aurora_i18n::tr!("explorer-panel-updated").into();
            }
        }
        _ if dragging(ui.ctx()) && drag::pointer::over(ui.ctx(), response.layer_id, response.interact_rect) => {
            if ui.input(|i| i.pointer.primary_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
            } else if let Some(pos) = ui.input(|i| i.pointer.latest_pos().filter(|_| i.pointer.primary_down())) {
                drop_feedback(ui.ctx(), pos, super::row_name(inv, layout, row, state.here.raw, ""), false);
            }
        }
        _ => {}
    }
    started
}

fn drop_target(ctx: &Context, layer: egui::LayerId, rect: Rect, inv: &Inventory, layout: &mut Preferences, place: Location) -> bool {
    if !drag::pointer::over(ctx, layer, rect) {
        return false;
    }
    let Some(moving) = Moving::carried(ctx) else { return false };
    let (ids, custom) = moving.entries();
    // A release from a secondary mouse button never completes a primary drag.
    let release = ctx.input(|i| i.pointer.primary_released());
    // A large multi-selection can be held over one destination for many frames.
    // Actual release always revalidates the complete batch in move_entries.
    let key = (inv.root, inv.generation, layout.revision(), place);
    let cache_id = egui::Id::unique("explorer-local-drag-check");
    let allowed = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Option<CheckedMove>>(cache_id);
        if let Some(checked) = cache
            && checked.key == key
            && checked.drag.same_drag(&moving)
        {
            return checked.allowed;
        }
        let same_owner = !matches!(&moving, Moving::Local(c) if c.root != inv.root);
        let allowed = same_owner && layout.can_move_entries(inv, ids, custom, place);
        *cache = Some(CheckedMove {
            drag: moving.clone(),
            key,
            allowed,
        });
        allowed
    });
    if release {
        egui::DragAndDrop::clear_payload(ctx);
        ctx.data_mut(|d| d.remove::<Option<CheckedMove>>(cache_id));
    }
    if !release && let Some(pos) = ctx.pointer_latest_pos() {
        let path = super::path(inv, layout, place, false)
            .into_iter()
            .map(|location| place_name(inv, layout, location, false, ""))
            .collect::<Vec<_>>()
            .join(" / ");
        drop_feedback(ctx, pos, path, allowed);
    }
    if !allowed {
        return false;
    }
    let stroke = ctx.global_style().visuals.selection.stroke;
    ctx.layer_painter(layer).with_clip_rect(rect).rect_stroke(
        rect.shrink(1.0),
        ctx.global_style().visuals.widgets.inactive.corner_radius,
        stroke,
        egui::StrokeKind::Inside,
    );
    release && layout.move_entries(inv, ids, custom, place)
}

fn drop_feedback(ctx: &Context, pos: Pos2, destination: String, allowed: bool) {
    let (cursor, text) = if allowed {
        (
            egui::CursorIcon::Grabbing,
            aurora_i18n::tr!("explorer-local-drop", destination = destination),
        )
    } else {
        (
            egui::CursorIcon::NotAllowed,
            aurora_i18n::tr!("explorer-denied-drop", destination = destination),
        )
    };
    ctx.set_cursor_icon(cursor);
    drag::set_feedback(ctx, cursor, true);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("explorer-drop-label")));
    let visuals = &ctx.global_style().visuals;
    let screen = ctx.content_rect();
    let galley = painter.layout(
        text,
        FontId::proportional(13.0),
        visuals.text_color(),
        (screen.width() - 16.0).max(1.0),
    );
    let size = galley.size() + vec2(12.0, 8.0);
    let origin = pos2(
        (pos.x + 15.0).min(screen.right() - size.x).max(screen.left()),
        (pos.y + 19.0).min(screen.bottom() - size.y).max(screen.top()),
    );
    let rect = Rect::from_min_size(origin, size);
    drag::note_hint_rect(ctx, rect);
    painter.rect_filled(rect, visuals.window_corner_radius, visuals.panel_fill);
    painter.galley(origin + vec2(6.0, 4.0), galley, visuals.text_color());
}

pub(super) fn receive(response: &Response, inv: &Inventory, layout: &mut Preferences, place: Location) -> bool {
    if !response.contains_pointer() {
        return false;
    }
    drop_target(&response.ctx, response.layer_id, response.interact_rect, inv, layout, place)
}

pub(super) fn background(ui: &mut Ui, rect: Rect, palette: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    let viewport = state.input.viewport.map_or(rect, |v| v.intersect(rect)).intersect(ui.clip_rect());
    let (origin, pointer, pressed, down, released, escape) = ui.input(|i| {
        (
            i.pointer.press_origin(),
            i.pointer.latest_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.key_pressed(Key::Escape),
        )
    });
    let to_content = state.input.offset - viewport.min.to_vec2();
    let on_surface = |p: Pos2| viewport.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id());
    if pressed && let Some(start) = origin.filter(|p| on_surface(*p) && !over_header(state, *p)) {
        let hit = state.input.visible.iter().find(|(_, r, _)| r.contains(start)).copied();
        if hit.is_none_or(|(_, _, g)| g.is_some_and(|r| r.contains(start))) {
            focus(ui, state);
            state.here.waiting_selection.clear();
            state.input.marquee = Some(Marquee {
                origin: start + to_content,
                start_screen: start,
                initial_row: hit.is_some(),
                moved: false,
                modifiers: press_modifiers(ui),
                initial: state.here.selected.clone(),
                seen: HashMap::new(),
            });
        }
    }
    if let Some(mut marquee) = state.input.marquee.take() {
        if escape || (!down && !released) {
            state.here.replace_selection(marquee.initial);
        } else if let Some(pos) = pointer {
            if pos.distance(marquee.start_screen) > ui.ctx().options(|o| o.input_options.max_click_dist) {
                marquee.moved = true;
            }
            if marquee.moved {
                let area = Rect::from_two_pos(marquee.origin, viewport.clamp(pos) + to_content);
                for &(row, r, _) in &state.input.visible {
                    marquee.seen.insert(row, r.translate(to_content));
                }
                let toggle = marquee.modifiers.ctrl || marquee.modifiers.command;
                let mut selected = if toggle || marquee.modifiers.shift {
                    marquee.initial.clone()
                } else {
                    HashSet::new()
                };
                for (&row, bounds) in &marquee.seen {
                    if bounds.intersects(area) && !(toggle && selected.remove(&row)) {
                        selected.insert(row);
                    }
                }
                state.here.replace_selection(selected);
                if !released {
                    let paint = ui.painter().with_clip_rect(viewport);
                    let screen = area.translate(-to_content);
                    let stroke = selection_edge(palette);
                    paint.rect_filled(screen, egui::CornerRadius::ZERO, stroke.color.gamma_multiply(0.14));
                    paint.rect_stroke(screen, egui::CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
                }
                if released {
                    let first = state.rows.iter().find(|r| state.here.selected.contains(r)).copied();
                    state.anchor = first;
                    state.input.caret = first;
                }
            } else if released && !marquee.initial_row && !marquee.modifiers.any() {
                state.here.clear_selection();
                state.anchor = None;
                state.input.caret = None;
            }
            if down {
                state.input.marquee = Some(marquee);
            }
        }
    }
    if released || !down {
        state.input.press = None;
    }
    if !state.here.raw
        && pointer.is_some_and(|p| on_surface(p) && !over_header(state, p) && !state.input.visible.iter().any(|(_, r, _)| r.contains(p)))
        && drop_target(ui.ctx(), ui.layer_id(), viewport, inv, layout, state.here.location)
    {
        state.status = aurora_i18n::tr!("explorer-panel-updated").into();
    }
}

pub(super) fn keyboard(ui: &Ui, state: &mut ExplorerState) {
    let modifiers = ui.input(|i| i.modifiers);
    let ctrl = modifiers.ctrl || modifiers.command;
    let take = |key| ui.input_mut(|i| i.consume_key(modifiers, key));
    if modifiers.alt {
        return;
    }
    if ctrl && take(Key::A) {
        state.here.waiting_selection.clear();
        state.here.replace_selection(state.rows.iter().copied().collect());
        if state.anchor.is_none() {
            state.anchor = state.rows.first().copied();
        }
        return;
    }
    if state.rows.is_empty() {
        return;
    }
    let index = state.input.caret.and_then(|r| state.rows.iter().position(|&x| x == r));
    let next = if take(Key::Home) {
        Some(0)
    } else if take(Key::End) {
        Some(state.rows.len() - 1)
    } else if take(Key::ArrowDown) {
        Some(index.map_or(0, |n| (n + state.input.columns.max(1)).min(state.rows.len() - 1)))
    } else if take(Key::ArrowUp) {
        Some(index.unwrap_or(0).saturating_sub(state.input.columns.max(1)))
    } else if (state.input.columns > 1 || state.input.column_height > 0) && take(Key::ArrowRight) {
        Some(index.map_or(0, |n| (n + state.input.column_height.max(1)).min(state.rows.len() - 1)))
    } else if (state.input.columns > 1 || state.input.column_height > 0) && take(Key::ArrowLeft) {
        Some(index.unwrap_or(0).saturating_sub(state.input.column_height.max(1)))
    } else {
        None
    };
    if let Some(next) = next {
        let row = state.rows[next];
        if ctrl && !modifiers.shift {
            state.input.caret = Some(row);
        } else {
            state.select(row, modifiers);
        }
        state.reveal_row = Some(row);
    } else if ctrl
        && take(Key::Space)
        && let Some(row) = state.input.caret
    {
        state.select(row, modifiers);
    }
}
