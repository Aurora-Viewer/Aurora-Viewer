//! The open slot's editor, in place on its group's page: its main list and
//! its layers as tabs, how their entries take turns, the entries themselves,
//! and the box that takes new ones.
//!
//! The main list is the state's animations (FloaterAO's animation list); a
//! group plays its members together, a layer (track) plays on top with its
//! own turns. Taking turns is AOEngine's cycle (`:CY`), random order
//! (`:RN`) and cycle time (`:CT`), shown as one order menu (In order,
//! Shuffle, Manual) and a time slider.
use super::*;
use crate::ao::{StateConfig, Track, inventory::editor::List};

/// What a row shows.
struct RowSpec<'a> {
    id: Uuid,
    names: Vec<&'a str>,
    together: bool,
    playing: bool,
    previewed: bool,
}

/// The slider's stops: "Each time" (0, AOEngine's cycle without a timer: the
/// next one at each entry into the state), then seconds.
pub(super) const STOPS: [f32; 6] = [0.0, 10.0, 20.0, 30.0, 60.0, 120.0];
/// Height of an entry's row, and the room between two.
const ROW: f32 = 34.0;
const ROW_GAP: f32 = 5.0;
/// The turns card: the height of each of its lines and of its order button.
const TURNS_LINE: f32 = 22.0;
/// The track of stops under it, with their names.
const TRACK_H: f32 = 30.0;
/// How long a dropped order is shown after its save ends, waiting for the
/// inventory to bring it back (a refetch of a stale read-back included); a
/// failed save then shows the saved order.
const HELD_FOR: std::time::Duration = std::time::Duration::from_secs(8);
const ORDER_H: f32 = 26.0;
/// The tabs over a slot's lists: their height, inner padding and gap.
const TAB_H: f32 = 30.0;
const TAB_PAD: f32 = 10.0;
const TAB_GAP: f32 = 2.0;

/// How the entries of a list take turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Order {
    Sequence,
    Shuffle,
    Manual,
}

impl Order {
    /// Random order only acts while the cycle is on (AOEngine::cycle), so
    /// the three cover every saved combination.
    pub(super) fn of(cycle: &Cycle) -> Self {
        match (cycle.cycle, cycle.random) {
            (false, _) => Self::Manual,
            (true, true) => Self::Shuffle,
            (true, false) => Self::Sequence,
        }
    }

    /// The cycle with this order; Manual keeps the random flag for later.
    pub(super) fn apply(self, cycle: &Cycle) -> Cycle {
        let mut cycle = cycle.clone();
        match self {
            Self::Manual => cycle.cycle = false,
            Self::Sequence => (cycle.cycle, cycle.random) = (true, false),
            Self::Shuffle => (cycle.cycle, cycle.random) = (true, true),
        }
        cycle
    }

    fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Sequence => "ao-order-in-order",
            Self::Shuffle => "ao-order-shuffle",
            Self::Manual => "ao-order-manual",
        })
    }

    fn line(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Sequence => "ao-order-in-order-line",
            Self::Shuffle => "ao-order-shuffle-line",
            Self::Manual => "ao-order-manual-line",
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Sequence => "repeat",
            Self::Shuffle => "arrows-shuffle",
            Self::Manual => "hand-stop",
        }
    }
}

/// The slider position of a cycle time: the nearest stop.
#[cfg(test)]
pub(super) fn stop_of(seconds: f32) -> usize {
    if seconds <= 0.0 {
        return 0;
    }
    STOPS
        .iter()
        .enumerate()
        .skip(1)
        .min_by(|a, b| (a.1 - seconds).abs().total_cmp(&(b.1 - seconds).abs()))
        .map_or(0, |(i, _)| i)
}

/// A name in a small rounded chip.
pub(super) fn chip(ui: &mut egui::Ui, p: &Palette, name: &str) -> egui::Response {
    egui::Frame::new()
        .fill(p.violet.gamma_multiply(0.28))
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| ui.label(RichText::new(name).small().color(p.ink)))
        .inner
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    set: &Set,
    runtime: &mut Runtime,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    if state.state.swimming() {
        swimming_status(ui, p, set, state.state);
    }
    let empty = StateConfig::default();
    let config = set.states.get(&state.state).unwrap_or(&empty);
    let mut editor = Editor {
        p,
        inv,
        set,
        runtime,
        state,
        out,
    };
    editor.body(ui, config);
}

/// Animations dragged from the inventory onto a slot: one item or a
/// selection (FloaterAO::handleDragAndDrop, ACCEPT_YES_MULTI). The cursor and
/// the outline say before the release whether the whole batch is taken: a
/// folder or any entry that is not an animation refuses it.
#[allow(clippy::too_many_arguments)]
pub(super) fn inventory_drop(
    ui: &egui::Ui,
    p: &Palette,
    inv: &Inventory,
    set: &Set,
    target: State,
    container: Option<Uuid>,
    rect: egui::Rect,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    use super::super::inventory::{drag, tree};
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let Some(ids) = drag::carried_ids(ui.ctx()) else { return };
    let allowed = !state.busy() && !ids.is_empty() && ids.iter().all(|&id| inventory::animation(inv, id).is_some());
    tree::drop_outline(ui, p, rect, allowed);
    drag::feedback(
        ui.ctx(),
        if allowed {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        },
    );
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        if !state.busy() {
            // A refused batch still says why, as a refused single item did.
            push(out, state, inventory::add_many(inv, set, target, &ids, container));
        }
    }
}

/// The order menu's button: the order's icon in the accent, its name, and a
/// chevron, on the entries' shade.
/// Where `seconds` sits on the track of stops, 0 to 1: at its stop, or
/// between the two around it for a typed time.
fn stop_place(seconds: f32) -> f32 {
    let last = (STOPS.len() - 1) as f32;
    if seconds <= 0.0 {
        return 0.0;
    }
    for i in 1..STOPS.len() {
        if seconds <= STOPS[i] {
            let t = (seconds - STOPS[i - 1]) / (STOPS[i] - STOPS[i - 1]);
            return ((i - 1) as f32 + t) / last;
        }
    }
    1.0
}

#[cfg(test)]
pub(super) fn stop_place_for_tests(seconds: f32) -> f32 {
    stop_place(seconds)
}

/// A stop's name under the track.
fn stop_label(i: usize) -> String {
    match STOPS[i] {
        s if s <= 0.0 => aurora_i18n::tr!("ao-stop-auto").to_owned(),
        s if s >= 60.0 => aurora_i18n::tr!("ao-stop-minutes", count = (s / 60.0) as i64),
        s => aurora_i18n::tr!("ao-stop-seconds", count = s as i64),
    }
}

/// How often entries take turns, as a track of stops: each a notch with its
/// name, the chosen time a knob lit in the accent (between two notches for a
/// typed time), the way to it filled. A click on a notch, or a drag along
/// the track, picks its stop. `fill` (0 to 1): how far the time to the next
/// one has run, drawn round the knob. Returns the time picked.
fn stops_track(ui: &mut egui::Ui, p: &Palette, seconds: f32, enabled: bool, fill: Option<f32>) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TRACK_H), egui::Sense::click_and_drag());
    let enabled = enabled && ui.is_enabled();
    let inset = 8.0;
    let (x0, x1) = (rect.left() + inset, rect.right() - inset);
    let y = rect.top() + 9.0;
    let x_of = |t: f32| x0 + (x1 - x0) * t;
    let last = STOPS.len() - 1;
    let nearest = |x: f32| (((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * last as f32).round() as usize;
    // while held, the knob follows the stop under the pointer
    let pointer = response
        .interact_pointer_pos()
        .filter(|_| enabled && response.is_pointer_button_down_on());
    let shown = pointer.map_or(stop_place(seconds), |at| nearest(at.x) as f32 / last as f32);
    let hover = response.hover_pos().filter(|_| enabled).map(|at| nearest(at.x));
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let track = egui::Rect::from_min_max(egui::pos2(x0, y - 1.5), egui::pos2(x1, y + 1.5));
    ui.painter().rect_filled(track, radius, p.raised);
    let accent = if enabled { p.violet_light } else { p.muted_dim };
    let done = egui::Rect::from_min_max(track.min, egui::pos2(x_of(shown), track.max.y));
    ui.painter().rect_filled(done, radius, accent.gamma_multiply(0.85));
    let small = egui::FontId::proportional(10.5);
    let chosen = stop_place(seconds);
    for i in 0..=last {
        let at = i as f32 / last as f32;
        let x = x_of(at);
        let notch = if at <= shown + 1e-3 { accent } else { p.muted.gamma_multiply(0.6) };
        ui.painter().circle_filled(egui::pos2(x, y), 2.5, notch);
        let lit = hover == Some(i) || (chosen - at).abs() < 1e-3;
        let ink = if !enabled {
            p.muted_dim
        } else if lit {
            p.ink
        } else {
            p.muted
        };
        let galley = ui.painter().layout_no_wrap(stop_label(i), small.clone(), ink);
        let w = galley.size().x;
        // the ends' names keep inside the track
        let left = if i == 0 {
            x - 4.0
        } else if i == last {
            x - w + 4.0
        } else {
            x - w / 2.0
        };
        ui.painter().galley(egui::pos2(left, y + 7.0), galley, ink);
    }
    let knob = egui::pos2(x_of(shown), y);
    if enabled {
        ui.painter().circle_filled(knob, 9.0, p.violet.gamma_multiply(0.22));
        // the time to the next one runs round the knob
        if let Some(fill) = fill.filter(|f| *f > 0.0) {
            let steps = 32;
            let points: Vec<egui::Pos2> = (0..=((steps as f32 * fill) as usize).max(1))
                .map(|k| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (k as f32 / steps as f32).min(fill);
                    knob + 9.0 * Vec2::angled(a)
                })
                .collect();
            ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.5, p.violet_light)));
        }
    }
    ui.painter()
        .circle_filled(knob, 5.5, if enabled { p.violet_light } else { p.muted_dim });
    ui.painter().circle_filled(knob, 2.5, if enabled { p.on_accent } else { p.raised });
    response.widget_info(|| egui::WidgetInfo::slider(enabled, f64::from(seconds), aurora_i18n::tr!("ao-turns-every")));
    if !enabled {
        return None;
    }
    let released = response.drag_stopped() || response.clicked();
    released
        .then(|| response.interact_pointer_pos().or(response.hover_pos()))
        .flatten()
        .map(|at| STOPS[nearest(at.x)])
        .filter(|s| (*s - seconds).abs() > f32::EPSILON)
}

fn order_button(ui: &mut egui::Ui, p: &Palette, order: Order, most: f32) -> egui::Response {
    // as wide as what it says, at the column's right
    let font = egui::FontId::new(12.5, super::super::fonts::semibold(ui.ctx()));
    let label = widgets::elided_galley(ui, RichText::new(order.label()).font(font), most - 52.0);
    let width = 10.0 + 14.0 + 7.0 + label.size().x + 8.0 + 12.0 + 9.0;
    let (row, _) = ui.allocate_exact_size(Vec2::new(most, ORDER_H), egui::Sense::hover());
    let rect = egui::Rect::from_min_size(egui::pos2(row.right() - width, row.top()), Vec2::new(width, ORDER_H));
    let response = ui.interact(rect, ui.scope_id().with("ao_order"), egui::Sense::click());
    let shade = super::shades(p);
    let fill = if response.hovered() { shade.item_hot } else { shade.item };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let y = rect.center().y;
    widgets::paint_glyph(ui.painter(), order.icon(), egui::pos2(rect.left() + 17.0, y), 14.0, p.violet_pale);
    let label_w = label.size().x;
    widgets::paint_row_text(ui.painter(), label, rect.left() + 31.0, y, p.ink);
    let chevron = if response.hovered() { p.ink } else { p.muted };
    widgets::paint_glyph(
        ui.painter(),
        "chevron-down",
        egui::pos2(rect.left() + 31.0 + label_w + 14.0, y),
        13.0,
        chevron,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, order.label()));
    response
}

/// A double click, or a click with Shift, plays an entry in the world; a
/// click alone previews it.
fn in_world(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.double_clicked() || (response.clicked() && ui.input(|i| i.modifiers.shift))
}

/// A list's order as dropped, until its save comes back.
pub(super) struct HeldOrder {
    set: Uuid,
    state: State,
    list: List,
    keys: Vec<Uuid>,
    at: std::time::Instant,
}

/// A move of an entry waiting for the edit before it to be saved.
pub(super) struct QueuedMove {
    pub(super) set: Uuid,
    pub(super) state: State,
    pub(super) list: List,
    pub(super) source: Uuid,
    pub(super) target: Uuid,
    pub(super) after: bool,
}

struct Editor<'a> {
    p: &'a Palette,
    inv: &'a Inventory,
    set: &'a Set,
    runtime: &'a mut Runtime,
    state: &'a mut AoUi,
    out: &'a mut Vec<Action>,
}

impl Editor<'_> {
    fn busy(&self) -> bool {
        self.state.busy()
    }

    /// The order a list is shown in, as indexes into `ids` (its saved order):
    /// the order just dropped while its save is under way, then the saved
    /// one once it matches (or the save failed).
    fn shown_order(&mut self, list: List, ids: &[Uuid]) -> Vec<usize> {
        let saved: Vec<usize> = (0..ids.len()).collect();
        let Some(held) = self
            .state
            .held_order
            .as_ref()
            .filter(|h| h.set == self.set.id && h.state == self.state.state && h.list == list)
        else {
            return saved;
        };
        let waiting = self.busy()
            || self
                .state
                .queued_moves
                .iter()
                .any(|m| m.set == held.set && m.state == held.state && m.list == list);
        let same_entries = held.keys.len() == ids.len() && held.keys.iter().all(|k| ids.contains(k));
        let matched = held.keys.as_slice() == ids;
        // (counted from the save's end: a stale read-back is fetched again,
        // the new order coming a moment after "Set saved")
        let expired = !waiting && held.at.elapsed() > HELD_FOR;
        if matched || !same_entries || expired {
            if expired && !matched && same_entries {
                log::info!("AO: a moved entry's order did not come back from the inventory; showing the saved order");
            }
            self.state.held_order = None;
            return saved;
        }
        let keys = held.keys.clone();
        if waiting && let Some(held) = self.state.held_order.as_mut() {
            held.at = std::time::Instant::now();
        }
        keys.iter().filter_map(|k| ids.iter().position(|id| id == k)).collect()
    }

    /// Plays that entry of a list (`owner`: its folder) in the world. The
    /// open set is put in use first when another one is, only while the
    /// entry's state is the one playing, as FloaterAO's double-click
    /// (indra/newview/ao.cpp) leaves the set alone for another state; the
    /// entry is still chosen for when its state comes, as before.
    fn play(&mut self, owner: Uuid, index: usize) {
        if self.state.active_set != self.set.id && self.runtime.current == Some(self.state.state) {
            self.state.use_set = Some(self.set.id);
        }
        self.runtime.choose(self.state.state, owner, index);
    }

    /// The picker of animations, into `container` (a group or a layer) or
    /// the main list.
    fn picker(&mut self, container: Option<Uuid>) {
        self.state.pick_into = container;
        self.state.picker = Some(false);
        self.state.folder = crate::world::inventory::actions::system(self.inv, 20).unwrap_or(self.inv.root);
    }

    fn body(&mut self, ui: &mut egui::Ui, c: &StateConfig) {
        // A layer shows its tab; a stack just made (`AoUi::finish_editor`)
        // is previewed in the main list; anything gone (dissolved, or another
        // slot's) falls back to the main list.
        let track = self.state.container.and_then(|id| c.tracks.iter().find(|t| t.id == id));
        if track.is_none() {
            if let Some(index) = self
                .state
                .container
                .and_then(|id| c.steps.iter().position(|s| s.group && s.id == id))
            {
                self.state.preview.show_entry(index);
            }
            self.state.container = None;
        }
        self.tabs(ui, c, track.map(|t| t.id));
        let track = self.state.container.and_then(|id| c.tracks.iter().find(|t| t.id == id));
        if let Some(track) = track {
            self.layer_bar(ui, c, track);
            self.turns(ui, c, Some(track));
            self.members(ui, track);
        } else {
            self.turns(ui, c, None);
            self.main_list(ui, c);
        }
        self.drop_box(ui, track.map(|t| t.id));
    }

    /// Main and each layer as tabs sized to their names, the open one
    /// underlined in the accent, then "+ Layer" as one more tab.
    fn tabs(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: Option<Uuid>) {
        let p = self.p;
        let mut tabs: Vec<(&str, usize)> = vec![(aurora_i18n::tr!("ao-main"), c.steps.len())];
        tabs.extend(c.tracks.iter().map(|t| (t.name.as_str(), t.members.len())));
        let selected = track.and_then(|id| c.tracks.iter().position(|t| t.id == id)).map_or(0, |i| i + 1);
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TAB_H), egui::Sense::hover());
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let salt = ui.scope_id().with(("ao_slot_tab", self.state.state.name()));
        let semibold = super::super::fonts::semibold(ui.ctx());
        let add = aurora_i18n::tr!("ao-layer-tab");
        let add_w = 16.0
            + ui.painter()
                .layout_no_wrap(add.to_owned(), egui::FontId::proportional(12.5), p.muted)
                .size()
                .x
            + 2.0 * TAB_PAD;
        // each tab as wide as its name, all narrowed alike when they overflow
        let widest = ((row.width() - add_w) / tabs.len() as f32 - TAB_GAP).max(48.0);
        let mut x = row.left();
        for (i, (label, count)) in tabs.iter().enumerate() {
            let on = i == selected;
            let font = egui::FontId::new(12.5, if on { semibold.clone() } else { egui::FontFamily::Proportional });
            let num = ui
                .painter()
                .layout_no_wrap(count.to_string(), egui::FontId::proportional(11.0), p.muted);
            let name_room = widest - 2.0 * TAB_PAD - num.size().x - 5.0;
            let name = widgets::elided_galley(ui, RichText::new(*label).font(font), name_room);
            let width = 2.0 * TAB_PAD + name.size().x + 5.0 + num.size().x;
            let r = egui::Rect::from_min_size(egui::pos2(x, row.top()), Vec2::new(width, TAB_H));
            let response = ui.interact(r, salt.with(i), egui::Sense::click());
            if response.hovered() && !on {
                ui.painter()
                    .rect_filled(r.shrink2(Vec2::new(0.0, 3.0)), radius, p.raised.gamma_multiply(0.5));
            }
            let ink = if on || response.hovered() { p.ink } else { p.muted };
            let y = r.center().y - 1.0;
            let name_w = name.size().x;
            widgets::paint_row_text(ui.painter(), name, r.left() + TAB_PAD, y, ink);
            let count_ink = if on { p.violet_pale } else { p.muted };
            widgets::paint_row_text(ui.painter(), num, r.left() + TAB_PAD + name_w + 5.0, y, count_ink);
            if on {
                let bar = egui::Rect::from_min_max(
                    egui::pos2(r.left() + TAB_PAD - 2.0, r.bottom() - 2.0),
                    egui::pos2(r.right() - TAB_PAD + 2.0, r.bottom()),
                );
                ui.painter().rect_filled(bar, radius, p.violet_light);
            }
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *label));
            if response.clicked() && !on {
                self.state.container = i.checked_sub(1).and_then(|i| c.tracks.get(i)).map(|t| t.id);
                self.state.cycle_edit = None;
            }
            x += width + TAB_GAP;
        }
        // "+ Layer", the last tab
        let enabled = !self.busy();
        let r = egui::Rect::from_min_size(egui::pos2(x, row.top()), Vec2::new(add_w, TAB_H));
        let response = ui.interact(r, salt.with("add"), egui::Sense::click());
        let hot = response.hovered() && enabled;
        if hot {
            ui.painter()
                .rect_filled(r.shrink2(Vec2::new(0.0, 3.0)), radius, p.raised.gamma_multiply(0.5));
        }
        let ink = if hot {
            p.violet_light
        } else if enabled {
            p.violet_pale
        } else {
            p.muted_dim
        };
        let y = r.center().y - 1.0;
        widgets::paint_glyph(ui.painter(), "plus", egui::pos2(r.left() + TAB_PAD + 6.0, y), 13.0, ink);
        let galley = ui.painter().layout_no_wrap(add.to_owned(), egui::FontId::proportional(12.5), ink);
        widgets::paint_row_text(ui.painter(), galley, r.left() + TAB_PAD + 16.0, y, ink);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, add));
        if response.on_hover_text(aurora_i18n::tr!("ao-new-layer")).clicked() && enabled {
            let mut dialog = super::combinations::Dialog::new(self.set.id, self.state.state);
            dialog.track = true;
            dialog.name = aurora_i18n::tr!("ao-new-layer-name").into();
            self.state.picker = None;
            self.state.combination = Some(dialog);
        }
    }

    /// A layer's name and what can be done with it.
    fn layer_bar(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: &Track) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ao-layer-line", layer = track.name.as_str()))
                    .small()
                    .color(self.p.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    // Layers keep the order they are saved in (`:OR`), as
                    // the old track rows could be dragged.
                    let at = c.tracks.iter().position(|t| t.id == track.id).unwrap_or(0);
                    let later = c.tracks.get(at + 1).map(|t| t.id);
                    let earlier = at.checked_sub(1).and_then(|i| c.tracks.get(i)).map(|t| t.id);
                    if widgets::icon_button(ui, self.p, "trash-filled", aurora_i18n::tr!("ao-remove-layer"), true) {
                        self.state.remove = Some(track.id);
                        widgets::bring_floater_front(ui.ctx(), "ao_remove");
                    }
                    if widgets::flat_button(ui, self.p, aurora_i18n::tr!("ao-dissolve-layer")).clicked() {
                        self.dissolve(track.id);
                    }
                    if widgets::flat_button(ui, self.p, aurora_i18n::tr!("common-rename")).clicked() {
                        self.state.combination = Some(super::combinations::Dialog::rename(
                            self.set.id,
                            self.state.state,
                            track.id,
                            &track.name,
                            true,
                        ));
                    }
                    let moved = if widgets::icon_button(ui, self.p, "arrow-right", aurora_i18n::tr!("ao-layer-later"), later.is_some()) {
                        later.map(|target| (target, true))
                    } else if widgets::icon_button(ui, self.p, "arrow-left", aurora_i18n::tr!("ao-layer-earlier"), earlier.is_some()) {
                        earlier.map(|target| (target, false))
                    } else {
                        None
                    };
                    if let Some((target, after)) = moved {
                        match inventory::editor::reorder(self.inv, self.set, self.state.state, List::Tracks, track.id, target, after) {
                            Ok(change) if !change.operations.is_empty() => self.out.push(Action::Edit(change, None)),
                            Ok(_) => {}
                            Err(reason) => self.state.message = reason,
                        }
                    }
                });
            });
        });
    }

    fn dissolve(&mut self, id: Uuid) {
        match inventory::editor::dissolve(self.inv, self.set, self.state.state, id) {
            Ok(change) => self.out.push(Action::EditContainer(
                change,
                EditorTarget {
                    set: self.set.id,
                    state: self.state.state,
                    container: None,
                },
            )),
            Err(reason) => self.state.message = reason,
        }
    }

    /// How the list's entries take turns: how often, on a track of stops
    /// from "Each time" to two minutes (a typed time sits between them), the
    /// time left to the next one while the slot plays, and the order on the
    /// right. A list of one says that a second one would take turns.
    fn turns(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: Option<&Track>) {
        let (id, saved, len) = track.map_or((c.folder, &c.cycle, c.steps.len()), |t| (t.id, &t.cycle, t.members.len()));
        if len == 1 {
            ui.label(RichText::new(aurora_i18n::tr!("ao-turns-solo")).small().color(self.p.muted));
        }
        if id.is_nil() || len < 2 {
            return;
        }
        if self.state.cycle_edit.as_ref().is_none_or(|(old, _)| *old != id) {
            self.state.cycle_edit = Some((id, saved.clone()));
        }
        let Some((_, mut edit)) = self.state.cycle_edit.clone() else {
            return;
        };
        let order = Order::of(&edit);
        let mut commit = false;
        let busy = self.busy();
        let p = self.p;
        // the time left before the next one, while this slot plays on a timer
        let left = (order != Order::Manual && edit.seconds > 0.0 && self.runtime.current == Some(self.state.state))
            .then(|| self.runtime.next_switch(self.state.state, id))
            .flatten()
            .map(|at| at.saturating_duration_since(std::time::Instant::now()).as_secs_f32());
        egui::Frame::new()
            .fill(super::shades(p).card)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin {
                left: 12,
                right: 10,
                top: 8,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        // the order's button keeps its width at the right,
                        // centred on the card's height
                        let right = 130.0_f32.min(ui.available_width() * 0.4);
                        let middle = (ui.available_width() - right - ui.spacing().item_spacing.x).max(80.0);
                        let height = TURNS_LINE + 2.0 + TRACK_H;
                        ui.vertical(|ui| {
                            ui.set_width(middle);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.set_height(TURNS_LINE);
                                if order == Order::Manual {
                                    ui.add(egui::Label::new(aurora_i18n::tr!("ao-turns-manual")).truncate());
                                } else if edit.seconds > 0.0 {
                                    let field = 60.0;
                                    let countdown = if left.is_some() { 70.0 } else { 0.0 };
                                    let room = (ui.available_width() - field - countdown).max(20.0);
                                    ui.add(widgets::elided_label(ui, aurora_i18n::tr!("ao-turns-every"), room));
                                    let response = ui.add(
                                        egui::DragValue::new(&mut edit.seconds)
                                            // (a saved 0.5 s stays as it is:
                                            // the Legacy Viewer writes `%.2f`)
                                            .range(0.01..=86400.0)
                                            .clamp_existing_to_range(false)
                                            .speed(1.0)
                                            .suffix(aurora_i18n::tr!("ao-seconds-suffix")),
                                    );
                                    commit |= response.drag_stopped() || response.lost_focus();
                                    if let Some(left) = left {
                                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                                        let line = aurora_i18n::tr!("ao-next-in", seconds = left.ceil() as i64);
                                        ui.label(RichText::new(line).size(11.0).color(p.violet_pale));
                                    }
                                } else {
                                    ui.add(egui::Label::new(aurora_i18n::tr!("ao-turns-each-time")).truncate());
                                }
                            });
                            let fill = left.map(|left| 1.0 - (left / edit.seconds.max(0.01)).clamp(0.0, 1.0));
                            if let Some(seconds) = stops_track(ui, p, edit.seconds, order != Order::Manual, fill) {
                                edit.seconds = seconds;
                                commit = true;
                            }
                        });
                        ui.vertical(|ui| {
                            ui.set_width(right);
                            ui.add_space(((height - ORDER_H) / 2.0).max(0.0));
                            let face = order_button(ui, p, order, right).on_hover_text(aurora_i18n::tr!("ao-order-tip"));
                            super::super::menu::dropdown(&face, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                                for choice in [Order::Sequence, Order::Shuffle, Order::Manual] {
                                    if super::super::menu::item_detail(ui, p, choice.icon(), choice.label(), choice.line())
                                        && choice != order
                                    {
                                        edit = choice.apply(&edit);
                                        commit = true;
                                    }
                                }
                            });
                        });
                    });
                });
            });
        if let Some((_, buffered)) = self.state.cycle_edit.as_mut() {
            *buffered = edit.clone();
        }
        if commit && edit != *saved {
            let change = if let Some(track) = track {
                let mut track = track.clone();
                track.cycle = edit;
                inventory::editor::save_track(self.inv, &track)
            } else {
                let mut c = c.clone();
                c.cycle = edit;
                inventory::save_cycle(self.inv, self.state.state, &c)
            };
            push(self.out, self.state, change);
        }
    }

    /// The main list: single animations and stacks that play together. A
    /// click previews an entry; a double click or Shift+click plays it in
    /// the world (FloaterAO's list click, `AOEngine::selectAnimation`).
    fn main_list(&mut self, ui: &mut egui::Ui, c: &StateConfig) {
        // (only the set in use plays: another set's rows say nothing)
        let in_use = self.state.active_set == self.set.id;
        let playing = in_use.then(|| self.runtime.index(self.state.state, c.folder));
        // (in the order just dropped while it is being saved)
        let order = self.shown_order(List::Main, &c.steps.iter().map(|s| s.id).collect::<Vec<_>>());
        let specs: Vec<RowSpec<'_>> = order
            .iter()
            .map(|&index| (index, &c.steps[index]))
            .map(|(index, step)| RowSpec {
                id: step.id,
                names: if step.group {
                    step.members.iter().map(|m| m.name.as_str()).collect()
                } else {
                    vec![step.name.as_str()]
                },
                together: step.group,
                playing: playing == Some(index),
                previewed: self.state.preview.previews(None, index),
            })
            .collect();
        let rows = self.rows(ui, List::Main, &specs);
        for (&index, (response, remove)) in order.iter().zip(rows) {
            let step = &c.steps[index];
            if remove {
                self.remove(ui, step.id);
            }
            self.entry_menu(&response, c, step, index);
            if in_world(ui, &response) {
                self.play(c.folder, index);
                self.state.preview.show_entry(index);
            } else if response.clicked() {
                self.state.preview.show_entry(index);
            }
        }
    }

    /// A main entry's right-click menu: preview, play in the world, a
    /// stack's own actions and its members' removal, and removal.
    fn entry_menu(&mut self, response: &egui::Response, c: &StateConfig, step: &crate::ao::Step, index: usize) {
        let p = self.p;
        let busy = self.busy();
        super::super::menu::context_menu(response, p, |ui| {
            if super::super::menu::item(ui, p, "eye-filled", aurora_i18n::tr!("ao-preview")) {
                self.state.preview.show_entry(index);
            }
            if super::super::menu::item(ui, p, "player-play-filled", aurora_i18n::tr!("ao-play-in-world")) {
                self.play(c.folder, index);
                self.state.preview.show_entry(index);
            }
            // a stack made from this entry and others of the slot (the guided
            // dialog, this entry already chosen)
            if !step.group && step.members.len() == 1 {
                super::super::menu::separator(ui, p);
                if super::super::menu::item_if(ui, p, "stack-2-filled", aurora_i18n::tr!("ao-make-stack"), !busy) {
                    let mut dialog = super::combinations::Dialog::new(self.set.id, self.state.state);
                    dialog.selected.insert(step.id);
                    self.state.picker = None;
                    self.state.combination = Some(dialog);
                }
            }
            if step.group {
                super::super::menu::separator(ui, p);
                if super::super::menu::item_if(ui, p, "plus", aurora_i18n::tr!("ao-add-to-stack"), !busy) {
                    self.picker(Some(step.id));
                }
                if super::super::menu::item_if(ui, p, "edit-filled", aurora_i18n::tr!("common-rename"), !busy) {
                    self.state.combination = Some(super::combinations::Dialog::rename(
                        self.set.id,
                        self.state.state,
                        step.id,
                        &step.name,
                        false,
                    ));
                }
                if super::super::menu::item_if(ui, p, "arrows-split", aurora_i18n::tr!("ao-split-stack"), !busy) {
                    self.dissolve(step.id);
                }
                // (the members play together: their order does not matter)
                for member in &step.members {
                    let label = aurora_i18n::tr!("ao-remove-member", name = member.name.as_str());
                    if super::super::menu::item_if(ui, p, "trash-filled", &label, !busy) {
                        push(self.out, self.state, inventory::remove(self.inv, member.link));
                    }
                }
            }
            super::super::menu::separator(ui, p);
            if super::super::menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !busy) {
                self.remove(ui, step.id);
            }
        });
    }

    /// FloaterAO::onClickTrash (indra/newview/ao.cpp) removes an animation at
    /// once: it is only a link. A stack or a layer is a folder that goes to
    /// the Trash with what it holds, so it asks first (#3675).
    fn remove(&mut self, ui: &egui::Ui, id: Uuid) {
        if self.inv.folders.contains_key(&id) {
            self.state.remove = Some(id);
            widgets::bring_floater_front(ui.ctx(), "ao_remove");
        } else {
            push(self.out, self.state, inventory::remove(self.inv, id));
        }
    }

    /// A layer's members, taking turns: a click previews one over the main
    /// entry, a double click or Shift+click plays it in the world.
    fn members(&mut self, ui: &mut egui::Ui, track: &Track) {
        let in_use = self.state.active_set == self.set.id;
        let playing = in_use.then(|| self.runtime.index(self.state.state, track.id));
        let list = List::Members(track.id);
        let order = self.shown_order(list, &track.members.iter().map(|m| m.link).collect::<Vec<_>>());
        let specs: Vec<RowSpec<'_>> = order
            .iter()
            .map(|&index| (index, &track.members[index]))
            .map(|(index, member)| RowSpec {
                id: member.link,
                names: vec![member.name.as_str()],
                together: false,
                playing: playing == Some(index),
                previewed: self.state.preview.previews(Some(track.id), index),
            })
            .collect();
        let rows = self.rows(ui, list, &specs);
        for (&index, (response, remove)) in order.iter().zip(rows) {
            let member = &track.members[index];
            if remove {
                self.remove(ui, member.link);
            }
            let p = self.p;
            let busy = self.busy();
            super::super::menu::context_menu(&response, p, |ui| {
                if super::super::menu::item(ui, p, "eye-filled", aurora_i18n::tr!("ao-preview")) {
                    self.state.preview.show_layer(track.id, index);
                }
                if super::super::menu::item(ui, p, "player-play-filled", aurora_i18n::tr!("ao-play-in-world")) {
                    self.play(track.id, index);
                }
                super::super::menu::separator(ui, p);
                if super::super::menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !busy) {
                    self.remove(ui, member.link);
                }
            });
            if in_world(ui, &response) {
                self.play(track.id, index);
                self.state.preview.show_layer(track.id, index);
            } else if response.clicked() {
                self.state.preview.show_layer(track.id, index);
            }
        }
        if track.members.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("ao-combine-empty")).small().color(self.p.muted));
        }
    }

    /// The rows of a list, dragged to reorder them as the photo filters'
    /// cards are (`widgets::reorder`): the row held lifts as a card as wide
    /// as its name, under the pointer, the others glide out of its way and
    /// show the places they take; let go, the inventory takes the new order.
    /// Returns each row's response and whether its remove button was clicked.
    fn rows(&mut self, ui: &mut egui::Ui, list: List, specs: &[RowSpec<'_>]) -> Vec<(egui::Response, bool)> {
        let width = ui.available_width();
        let responses: Vec<egui::Response> = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                specs
                    .iter()
                    .map(|spec| {
                        let (_, rect) = ui.allocate_space(Vec2::new(width, ROW));
                        ui.interact(rect, ui.scope_id().with(("ao_row", spec.id)), egui::Sense::click_and_drag())
                    })
                    .collect()
            })
            .inner;
        let keys: Vec<Uuid> = specs.iter().map(|s| s.id).collect();
        let id = ui.scope_id().with(("ao_rows", format!("{list:?}")));
        // (a drag while a save is under way waits its turn: `queue_move`)
        let layout = widgets::reorder::update(ui, id, &keys, &responses, true, None);
        let mut out = Vec::with_capacity(specs.len());
        for (i, spec) in specs.iter().enumerate() {
            let rect = layout.rect(i, responses[i].rect);
            let place = layout.held.map_or(i, |(from, to)| widgets::reorder::place_in(i, from, to));
            let remove = if layout.lifted == Some(i) {
                // the card in the hand: as wide as what it says
                let font = egui::FontId::proportional(13.0);
                let text = spec.names.join(" + ");
                let text_w = ui.painter().layout_no_wrap(text, font, self.p.ink).size().x;
                let card = egui::Rect::from_min_size(rect.min, Vec2::new((text_w + 64.0).min(rect.width()), ROW));
                let p = self.p;
                widgets::reorder::lifted_card(ui, p, id.with(i), card, |ui| {
                    self.paint_row(ui, card, &responses[i], place, spec, true);
                });
                false
            } else {
                self.paint_row(ui, rect, &responses[i], place, spec, layout.busy())
            };
            out.push((responses[i].clone(), remove));
        }
        if let Some((from, to)) = layout.moved
            && let (Some(&source), Some(&target)) = (keys.get(from), keys.get(to))
        {
            // shown where it was dropped at once, saved behind
            let mut order = keys.clone();
            let moved = order.remove(from);
            order.insert(to, moved);
            self.state.held_order = Some(HeldOrder {
                set: self.set.id,
                state: self.state.state,
                list,
                keys: order,
                at: std::time::Instant::now(),
            });
            self.state.queued_moves.push(QueuedMove {
                set: self.set.id,
                state: self.state.state,
                list,
                source,
                target,
                after: to > from,
            });
        }
        // (saved one at a time by the window, `send_queued_move`)
        out
    }

    /// One entry at `rect`: its place, its name (a stack's members joined
    /// by "+"), "Preview" and "Playing", and its remove button at the end.
    /// The previewed one stands out in the accent. `quiet`: carried, or
    /// another is: no hover and no remove button. Returns whether its
    /// remove button was clicked.
    fn paint_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
        place: usize,
        spec: &RowSpec<'_>,
        quiet: bool,
    ) -> bool {
        let p = self.p;
        let shade = super::shades(p);
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let hovered = response.hovered() && !quiet;
        let fill = if spec.previewed {
            p.bar.lerp_to_gamma(p.violet, 0.32)
        } else if spec.together {
            shade.stack
        } else if hovered {
            shade.item_hot
        } else {
            shade.item
        };
        let edge = if spec.previewed {
            egui::Stroke::new(1.0, p.violet_light)
        } else {
            egui::Stroke::NONE
        };
        ui.painter()
            .rect(rect, ui.visuals().window_corner_radius, fill, edge, egui::StrokeKind::Inside);
        let y = rect.center().y;
        let mut x = rect.left() + 8.0;
        let number = ui
            .painter()
            .layout_no_wrap((place + 1).to_string(), egui::FontId::proportional(11.5), p.muted);
        widgets::paint_row_text(ui.painter(), number, x, y, p.muted);
        x += 18.0;
        // the remove button's room at the end
        let mut right = rect.right() - if quiet { 8.0 } else { 32.0 };
        if spec.playing {
            let galley = ui
                .painter()
                .layout_no_wrap(aurora_i18n::tr!("ao-playing").to_owned(), egui::FontId::proportional(11.0), p.teal);
            right -= galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, right, y, p.teal);
            right -= 10.0;
        }
        if spec.previewed {
            let galley = ui.painter().layout_no_wrap(
                aurora_i18n::tr!("ao-preview").to_owned(),
                egui::FontId::proportional(11.0),
                p.ink.gamma_multiply(0.7),
            );
            right -= galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, right, y, p.ink.gamma_multiply(0.7));
            right -= 10.0;
        }
        if spec.together {
            let tag = aurora_i18n::tr!("ao-together").to_uppercase();
            let galley = ui
                .painter()
                .layout_no_wrap(tag, egui::FontId::new(10.5, super::super::fonts::semibold(ui.ctx())), p.violet_pale);
            right -= galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, right, y, p.violet_pale);
            right -= 10.0;
        }
        let font = egui::FontId::proportional(13.0);
        for (i, name) in spec.names.iter().enumerate() {
            if x >= right - 20.0 {
                break;
            }
            if i > 0 {
                let plus = ui.painter().layout_no_wrap("+".to_owned(), font.clone(), p.violet_light);
                let w = plus.size().x;
                widgets::paint_row_text(ui.painter(), plus, x, y, p.violet_light);
                x += w + 6.0;
            }
            let pad = if spec.together { 6.0 } else { 0.0 };
            let galley = widgets::elided_galley(ui, RichText::new(*name).font(font.clone()).color(p.ink), right - x - 2.0 * pad);
            let w = galley.size().x;
            if spec.together {
                let face = egui::Rect::from_min_size(egui::pos2(x, y - 11.0), Vec2::new(w + 2.0 * pad, 22.0));
                ui.painter().rect_filled(face, radius, p.violet.gamma_multiply(0.28));
            }
            widgets::paint_row_text(ui.painter(), galley, x + pad, y, p.ink);
            x += w + 2.0 * pad + 6.0;
        }
        if quiet {
            return false;
        }
        let full = spec.names.join(" + ");
        response
            .clone()
            .on_hover_text(format!("{full}\n\n{}", aurora_i18n::tr!("ao-row-help")));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, spec.previewed, &full));
        // its remove button, where the row ends
        let bin = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, y), Vec2::splat(24.0));
        let hit = ui
            .interact(bin, ui.scope_id().with(("ao_row_remove", spec.id)), egui::Sense::click())
            .on_hover_text(aurora_i18n::tr!("common-remove"));
        if hit.hovered() {
            ui.painter().rect_filled(bin, radius, p.raised);
        }
        let ink = if hit.hovered() { p.danger } else { p.muted };
        widgets::paint_glyph(ui.painter(), "trash-filled", bin.center(), 13.0, ink);
        hit.clicked() && !self.busy()
    }

    /// The box that takes new animations: a drop from the inventory (the
    /// whole slot takes it too), or a click to choose one.
    fn drop_box(&mut self, ui: &mut egui::Ui, track: Option<Uuid>) {
        let p = self.p;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
        let hot = response.hovered() && !self.busy();
        widgets::dashed_outline(ui.painter(), rect, if hot { p.violet_light } else { p.muted.gamma_multiply(0.5) });
        let label = aurora_i18n::tr!("ao-drop-box");
        let ink = if hot { p.ink } else { p.muted };
        let galley = widgets::elided_galley(ui, RichText::new(label).size(12.5).color(ink), rect.width() - 40.0);
        let x = rect.center().x - (galley.size().x + 20.0) / 2.0;
        widgets::paint_glyph(ui.painter(), "plus", egui::pos2(x + 7.0, rect.center().y), 14.0, ink);
        widgets::paint_row_text(ui.painter(), galley, x + 20.0, rect.center().y, ink);
        let enabled = !self.busy();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        if response.clicked() && enabled {
            self.picker(track);
        }
    }
}
