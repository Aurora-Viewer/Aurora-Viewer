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
use crate::ao::{Animation, StateConfig, Track, inventory::editor::List};

#[derive(Clone, Copy, Debug)]
struct EntryDrag {
    set: Uuid,
    state: State,
    list: List,
    entry: Uuid,
}

/// The slider's stops: "Each time" (0, AOEngine's cycle without a timer: the
/// next one at each entry into the state), then seconds.
pub(super) const STOPS: [f32; 6] = [0.0, 10.0, 20.0, 30.0, 60.0, 120.0];
/// Height of an entry's row.
const ROW: f32 = 34.0;

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

    /// The picker of animations, into `container` (a group or a layer) or
    /// the main list.
    fn picker(&mut self, container: Option<Uuid>) {
        self.state.pick_into = container;
        self.state.picker = Some(false);
        self.state.folder = crate::world::inventory::actions::system(self.inv, 20).unwrap_or(self.inv.root);
    }

    fn body(&mut self, ui: &mut egui::Ui, c: &StateConfig) {
        // A layer shows its tab; a stack just made (`AoUi::finish_editor`)
        // unfolds in the main list; anything gone (dissolved, or another
        // slot's) falls back to the main list.
        let track = self.state.container.and_then(|id| c.tracks.iter().find(|t| t.id == id));
        if track.is_none() {
            if let Some(stack) = self.state.container.filter(|id| c.steps.iter().any(|s| s.group && s.id == *id)) {
                self.state.open_entry = Some(stack);
            }
            self.state.container = None;
        }
        self.tabs(ui, c, track.map(|t| t.id));
        let track = self.state.container.and_then(|id| c.tracks.iter().find(|t| t.id == id));
        if let Some(track) = track {
            self.layer_bar(ui, c, track);
            self.turns(ui, c, Some(track));
            self.members(ui, c, track.id, &track.members, true);
        } else {
            self.turns(ui, c, None);
            self.main_list(ui, c);
        }
        self.drop_box(ui, c, track.map(|t| t.id));
    }

    /// Main and each layer, with their counts, then "+" for a new layer.
    fn tabs(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: Option<Uuid>) {
        let mut tabs: Vec<(&str, usize)> = vec![(aurora_i18n::tr!("ao-main"), c.steps.len())];
        tabs.extend(c.tracks.iter().map(|t| (t.name.as_str(), t.members.len())));
        let mut selected = track.and_then(|id| c.tracks.iter().position(|t| t.id == id)).map_or(0, |i| i + 1);
        ui.horizontal(|ui| {
            let room = ui.available_width() - 30.0 - ui.spacing().item_spacing.x;
            let salt = format!("ao_slot_tabs_{}", self.state.state.name());
            let changed = ui
                .allocate_ui(Vec2::new(room, 30.0), |ui| {
                    widgets::count_tabs(ui, self.p, &salt, &mut selected, &tabs)
                })
                .inner;
            if changed {
                self.state.container = selected.checked_sub(1).and_then(|i| c.tracks.get(i)).map(|t| t.id);
                self.state.cycle_edit = None;
                self.state.open_entry = None;
            }
            if widgets::icon_button(ui, self.p, "plus", aurora_i18n::tr!("ao-new-layer"), !self.busy()) {
                let mut dialog = super::combinations::Dialog::new(self.set.id, self.state.state);
                dialog.track = true;
                dialog.name = aurora_i18n::tr!("ao-new-layer-name").into();
                self.state.picker = None;
                self.state.combination = Some(dialog);
            }
        });
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

    /// How the list's entries take turns: its order, how often, and which
    /// one comes next. A list of one says that a second one would take turns.
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
        let mut step = false;
        let busy = self.busy();
        let p = self.p;
        let (next, now) = self.next_name(c, track);
        egui::Frame::new()
            .fill(p.raised.gamma_multiply(0.45))
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        let (ring, _) = ui.allocate_exact_size(Vec2::splat(38.0), egui::Sense::hover());
                        let tint = if order == Order::Manual { p.muted } else { p.violet_light };
                        ui.painter()
                            .circle_stroke(ring.center(), 15.0, egui::Stroke::new(2.5, tint.gamma_multiply(0.85)));
                        widgets::paint_glyph(ui.painter(), order.icon(), ring.center(), 15.0, tint);
                        let right = 150.0;
                        let middle = (ui.available_width() - right - ui.spacing().item_spacing.x).max(120.0);
                        ui.vertical(|ui| {
                            ui.set_width(middle);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.set_height(28.0);
                                if order == Order::Manual {
                                    ui.label(aurora_i18n::tr!("ao-turns-manual"));
                                } else if edit.seconds > 0.0 {
                                    ui.label(aurora_i18n::tr!("ao-turns-every"));
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
                                } else {
                                    ui.label(aurora_i18n::tr!("ao-turns-each-time"));
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.set_height(28.0);
                                ui.label(RichText::new(aurora_i18n::tr!("ao-each-time")).small().color(p.muted));
                                let mut stop = stop_of(edit.seconds);
                                let before = stop;
                                ui.spacing_mut().slider_width = (ui.available_width() - 44.0).max(60.0);
                                let response = ui.add_enabled(
                                    order != Order::Manual,
                                    egui::Slider::new(&mut stop, 0..=STOPS.len() - 1).show_value(false),
                                );
                                if stop != before {
                                    edit.seconds = STOPS[stop];
                                }
                                commit |= (response.changed() && !response.dragged()) || response.drag_stopped();
                                ui.label(RichText::new(aurora_i18n::tr!("ao-two-minutes")).small().color(p.muted));
                            });
                        });
                        ui.vertical(|ui| {
                            ui.set_width(right);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let face = egui::Button::new(format!("{}  ▾", order.label())).min_size(Vec2::new(right, 28.0));
                            super::super::menu::button(ui, p, face, |ui| {
                                for choice in [Order::Sequence, Order::Shuffle, Order::Manual] {
                                    if super::super::menu::item_detail(ui, p, choice.icon(), choice.label(), choice.line())
                                        && choice != order
                                    {
                                        edit = choice.apply(&edit);
                                        commit = true;
                                    }
                                }
                            });
                            let label = if order == Order::Manual {
                                aurora_i18n::tr!("ao-stays-on", name = now.as_str())
                            } else if order == Order::Shuffle {
                                // (the next one is drawn at random)
                                aurora_i18n::tr!("ao-up-next-random").to_owned()
                            } else {
                                aurora_i18n::tr!("ao-up-next", name = next.as_str())
                            };
                            step = ui
                                .add_sized(
                                    [right, 28.0],
                                    egui::Button::new(RichText::new(&label).small().color(p.muted))
                                        .frame(false)
                                        .truncate(),
                                )
                                .on_hover_text(aurora_i18n::tr!("ao-next-tip"))
                                .clicked();
                        });
                    });
                });
            });
        if step {
            self.step(c, track);
        }
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

    /// The names of the entry after the one playing, and of the one playing.
    fn next_name(&self, c: &StateConfig, track: Option<&Track>) -> (String, String) {
        let names: Vec<&str> = track.map_or_else(
            || c.steps.iter().map(|s| s.name.as_str()).collect(),
            |t| t.members.iter().map(|m| m.name.as_str()).collect(),
        );
        let owner = track.map_or(c.folder, |t| t.id);
        let index = self.runtime.index(self.state.state, owner).min(names.len().saturating_sub(1));
        let now = names.get(index).copied().unwrap_or_default();
        let next = names.get((index + 1) % names.len().max(1)).copied().unwrap_or_default();
        (next.to_owned(), now.to_owned())
    }

    /// Plays the next entry now (AOEngine::cycle(CycleNext)).
    fn step(&mut self, c: &StateConfig, track: Option<&Track>) {
        let (owner, len) = track.map_or((c.folder, c.steps.len()), |t| (t.id, t.members.len()));
        if len > 0 {
            let index = (self.runtime.index(self.state.state, owner) + 1) % len;
            self.runtime.choose(self.state.state, owner, index);
        }
    }

    /// The main list: single animations and groups that play together.
    fn main_list(&mut self, ui: &mut egui::Ui, c: &StateConfig) {
        let playing = self.runtime.index(self.state.state, c.folder);
        for (index, step) in c.steps.iter().enumerate() {
            ui.push_id(step.id, |ui| {
                let names: Vec<&str> = if step.group {
                    step.members.iter().map(|m| m.name.as_str()).collect()
                } else {
                    vec![step.name.as_str()]
                };
                let open = self.state.open_entry == Some(step.id);
                if self.row(ui, step.id, index, &names, step.group, List::Main, playing == index) {
                    self.runtime.choose(self.state.state, c.folder, index);
                    self.state.preview.show_entry(index);
                    self.state.open_entry = if open { None } else { Some(step.id) };
                }
                if self.state.open_entry == Some(step.id) {
                    self.entry_card(ui, c, step, index);
                }
            });
        }
    }

    /// The unfolded details of a main entry: a group's members, and the
    /// entry's actions.
    fn entry_card(&mut self, ui: &mut egui::Ui, c: &StateConfig, step: &crate::ao::Step, index: usize) {
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 24,
                right: 4,
                top: 2,
                bottom: 8,
            })
            .show(ui, |ui| {
                if step.group {
                    ui.label(RichText::new(aurora_i18n::tr!("ao-stack-explanation")).small().color(self.p.muted));
                    self.members(ui, c, step.id, &step.members, false);
                }
                ui.add_enabled_ui(!self.busy(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let preview = aurora_i18n::tr!("ao-preview");
                        let size = Vec2::new(widgets::action_button_fit(ui, "eye-filled", preview), 26.0);
                        if widgets::action_button(ui, self.p, "eye-filled", preview, widgets::ActionKind::Primary, size).clicked() {
                            self.state.preview.show_entry(index);
                        }
                        if step.group {
                            if widgets::flat_button(ui, self.p, aurora_i18n::tr!("ao-add-to-stack")).clicked() {
                                self.picker(Some(step.id));
                            }
                            if widgets::flat_button(ui, self.p, aurora_i18n::tr!("common-rename")).clicked() {
                                self.state.combination = Some(super::combinations::Dialog::rename(
                                    self.set.id,
                                    self.state.state,
                                    step.id,
                                    &step.name,
                                    false,
                                ));
                            }
                            if widgets::flat_button(ui, self.p, aurora_i18n::tr!("ao-split-stack")).clicked() {
                                self.dissolve(step.id);
                            }
                        }
                        let remove = aurora_i18n::tr!("common-remove");
                        let size = Vec2::new(widgets::action_button_fit(ui, "trash-filled", remove), 26.0);
                        if widgets::action_button(ui, self.p, "trash-filled", remove, widgets::ActionKind::FlatDanger, size).clicked() {
                            self.remove(ui, step.id);
                        }
                    });
                });
            });
    }

    /// FloaterAO::onClickTrash (indra/newview/ao.cpp) removes an animation at
    /// once: it is only a link. A group or a layer is a folder that goes to
    /// the Trash with what it holds, so it asks first (#3675).
    fn remove(&mut self, ui: &egui::Ui, id: Uuid) {
        if self.inv.folders.contains_key(&id) {
            self.state.remove = Some(id);
            widgets::bring_floater_front(ui.ctx(), "ao_remove");
        } else {
            push(self.out, self.state, inventory::remove(self.inv, id));
            if self.state.open_entry == Some(id) {
                self.state.open_entry = None;
            }
        }
    }

    /// The members of a group (playing together) or of a layer (taking
    /// turns), each removable and in its order.
    fn members(&mut self, ui: &mut egui::Ui, c: &StateConfig, owner: Uuid, members: &[Animation], track: bool) {
        let playing = self.runtime.index(self.state.state, owner);
        for (index, member) in members.iter().enumerate() {
            ui.push_id(member.link, |ui| {
                let selected = track && playing == index;
                if self.row(
                    ui,
                    member.link,
                    index,
                    &[member.name.as_str()],
                    false,
                    List::Members(owner),
                    selected,
                ) {
                    if track {
                        self.runtime.choose(self.state.state, owner, index);
                    } else if let Some(index) = c.steps.iter().position(|s| s.id == owner) {
                        self.runtime.choose(self.state.state, c.folder, index);
                    }
                }
            });
        }
        if members.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("ao-combine-empty")).small().color(self.p.muted));
        }
    }

    /// One entry: its number, its name (a group's members joined by "+"),
    /// whether it plays now, and a chevron (main list) or a remove button
    /// (members). Drag it to change its place. Returns whether it was
    /// clicked.
    #[allow(clippy::too_many_arguments)]
    fn row(&mut self, ui: &mut egui::Ui, id: Uuid, index: usize, names: &[&str], together: bool, list: List, playing: bool) -> bool {
        let p = self.p;
        let open = self.state.open_entry == Some(id);
        let sense = if self.busy() {
            egui::Sense::click()
        } else {
            egui::Sense::click_and_drag()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), sense);
        let radius = ui.visuals().widgets.inactive.corner_radius;
        if open {
            ui.painter().rect_filled(rect, radius, p.raised);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
        }
        let y = rect.center().y;
        let mut x = rect.left() + 8.0;
        let number = ui
            .painter()
            .layout_no_wrap((index + 1).to_string(), egui::FontId::proportional(11.5), p.muted);
        widgets::paint_row_text(ui.painter(), number, x, y, p.muted);
        x += 18.0;
        let mut right = rect.right() - 8.0;
        let member = matches!(list, List::Members(_));
        if list == List::Main {
            let chevron = if open { "chevron-down" } else { "chevron-right" };
            widgets::paint_glyph(ui.painter(), chevron, egui::pos2(right - 7.0, y), 14.0, p.muted);
            right -= 22.0;
        } else if member {
            right -= 24.0;
        }
        if playing {
            let galley = ui
                .painter()
                .layout_no_wrap(aurora_i18n::tr!("ao-playing").to_owned(), egui::FontId::proportional(11.0), p.teal);
            right -= galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, right, y, p.teal);
            right -= 10.0;
        }
        if together {
            let tag = aurora_i18n::tr!("ao-together").to_uppercase();
            let galley = ui
                .painter()
                .layout_no_wrap(tag, egui::FontId::new(10.5, super::super::fonts::semibold(ui.ctx())), p.violet_pale);
            right -= galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, right, y, p.violet_pale);
            right -= 10.0;
        }
        let font = egui::FontId::proportional(13.0);
        for (i, name) in names.iter().enumerate() {
            if x >= right - 20.0 {
                break;
            }
            if i > 0 {
                let plus = ui.painter().layout_no_wrap("+".to_owned(), font.clone(), p.violet_light);
                let w = plus.size().x;
                widgets::paint_row_text(ui.painter(), plus, x, y, p.violet_light);
                x += w + 6.0;
            }
            let pad = if together { 6.0 } else { 0.0 };
            let galley = widgets::elided_galley(ui, RichText::new(*name).font(font.clone()).color(p.ink), right - x - 2.0 * pad);
            let w = galley.size().x;
            if together {
                let face = egui::Rect::from_min_size(egui::pos2(x, y - 11.0), Vec2::new(w + 2.0 * pad, 22.0));
                ui.painter().rect_filled(face, radius, p.violet.gamma_multiply(0.28));
            }
            widgets::paint_row_text(ui.painter(), galley, x + pad, y, p.ink);
            x += w + 2.0 * pad + 6.0;
        }
        let full = names.join(" + ");
        let response = response.on_hover_text(format!("{full}\n\n{}", aurora_i18n::tr!("ao-entry-help")));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, playing, &full));
        if !self.busy() {
            response.dnd_set_drag_payload(EntryDrag {
                set: self.set.id,
                state: self.state.state,
                list,
                entry: id,
            });
            if let Some(drag) = egui::DragAndDrop::payload::<EntryDrag>(ui.ctx())
                && drag.set == self.set.id
                && drag.state == self.state.state
                && drag.list == list
                && drag.entry != id
                && response.contains_pointer()
                && let Some(pointer) = ui.ctx().pointer_hover_pos()
            {
                let after = pointer.y >= rect.center().y;
                let line = if after { rect.bottom() } else { rect.top() };
                ui.painter().hline(rect.x_range(), line, egui::Stroke::new(2.0, p.violet_light));
                if response.dnd_release_payload::<EntryDrag>().is_some() {
                    match inventory::editor::reorder(self.inv, self.set, self.state.state, list, drag.entry, id, after) {
                        Ok(change) if !change.operations.is_empty() => self.out.push(Action::Edit(change, None)),
                        Ok(_) => {}
                        Err(reason) => self.state.message = reason,
                    }
                }
            }
        }
        if member {
            // A member's own remove button, at its row's end.
            let bin = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, y), Vec2::splat(24.0));
            let hit = ui
                .interact(bin, ui.scope_id().with(("ao_member_remove", id)), egui::Sense::click())
                .on_hover_text(aurora_i18n::tr!("common-remove"));
            let ink = if hit.hovered() { p.danger } else { p.muted };
            widgets::paint_glyph(ui.painter(), "trash-filled", bin.center(), 13.0, ink);
            if hit.clicked() && !self.busy() {
                self.remove(ui, id);
            }
        }
        response.clicked()
    }

    /// The box that takes new animations: a drop from the inventory (the
    /// whole slot takes it too), or a click to choose one; and the way to
    /// make entries play together.
    fn drop_box(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: Option<Uuid>) {
        let p = self.p;
        ui.horizontal(|ui| {
            let singles = c.steps.iter().filter(|s| !s.group && s.members.len() == 1).count();
            let together = track.is_none() && singles >= 2;
            let room = ui.available_width() - if together { 150.0 + ui.spacing().item_spacing.x } else { 0.0 };
            let (rect, response) = ui.allocate_exact_size(Vec2::new(room, 40.0), egui::Sense::click());
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
            if together
                && ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(aurora_i18n::tr!("ao-combine")).min_size(Vec2::new(150.0, 40.0)),
                    )
                    .on_hover_text(aurora_i18n::tr!("ao-combine-tip"))
                    .clicked()
            {
                self.state.picker = None;
                self.state.combination = Some(super::combinations::Dialog::new(self.set.id, self.state.state));
            }
        });
    }
}
