//! A group's page in the AO window: its summary, then every slot of the group,
//! each showing what it holds. The open slot unfolds its editor in place
//! (`slot`); an animation dropped from the inventory onto any slot goes into
//! it (FloaterAO::handleDragAndDrop adds to the state shown; here each slot
//! is its own target).
use super::category::Category;
use super::*;

/// The six slots everyone around notices, shown alone by "Essentials".
pub(super) const ESSENTIALS: [State; 6] = [
    State::Standing,
    State::Sitting,
    State::Walking,
    State::Running,
    State::Jumping,
    State::Flying,
];

/// Height of a slot's title line.
const HEADER: f32 = 40.0;
/// How long a slot takes to unfold or fold (seconds).
const REVEAL: f32 = 0.2;
/// Chips shown in a closed slot before "+N more".
const PEEK: usize = 4;

/// The slots of a group the page shows.
pub(super) fn shown(category: Category, essentials: bool) -> impl Iterator<Item = State> {
    category.states().filter(move |s| !essentials || ESSENTIALS.contains(s))
}

/// The animations of a slot: its main list (a group counts each member) and
/// its layers.
pub(super) fn slot_animations(set: &Set, state: State) -> usize {
    set.states.get(&state).map_or(0, |c| {
        c.steps.iter().map(|s| s.members.len()).sum::<usize>() + c.tracks.iter().map(|t| t.members.len()).sum::<usize>()
    })
}

pub(super) fn filled(set: &Set, state: State) -> bool {
    slot_animations(set, state) > 0
}

/// Every animation of a set.
pub(super) fn animations(set: &Set) -> usize {
    set.states.keys().map(|s| slot_animations(set, *s)).sum()
}

/// The creators of a group's animations, as their originals name them.
fn creators(inv: &Inventory, set: &Set, states: &[State]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for config in states.iter().filter_map(|s| set.states.get(s)) {
        let members = config
            .steps
            .iter()
            .flat_map(|s| &s.members)
            .chain(config.tracks.iter().flat_map(|t| &t.members));
        for member in members {
            if let Some(item) = inv.items.get(&member.original).filter(|it| !it.creator.is_nil()) {
                seen.insert(item.creator);
            }
        }
    }
    seen.len()
}

/// What a slot is for, under its name.
pub(super) fn description(state: State) -> &'static str {
    aurora_i18n::text(match state {
        State::Standing => "ao-line-standing",
        State::StandingUp => "ao-line-standing-up",
        State::Sitting => "ao-line-sitting",
        State::GroundSit => "ao-line-ground-sit",
        State::Typing => "ao-line-typing",
        State::Always => "ao-line-always",
        State::Walking => "ao-line-walking",
        State::Running => "ao-line-running",
        State::Crouching => "ao-line-crouching",
        State::CrouchWalk => "ao-line-crouch-walk",
        State::TurningLeft | State::TurningRight => "ao-line-turning",
        State::PreJumping => "ao-line-pre-jumping",
        State::Jumping => "ao-line-jumping",
        State::Falling => "ao-line-falling",
        State::SoftLanding => "ao-line-soft-landing",
        State::Landing => "ao-line-landing",
        State::Flying => "ao-line-flying",
        State::FlyingSlow => "ao-line-flying-slow",
        State::Hovering => "ao-line-hovering",
        State::FlyingUp => "ao-line-flying-up",
        State::FlyingDown => "ao-line-flying-down",
        State::Floating => "ao-line-floating",
        State::SwimmingForward => "ao-line-swimming-forward",
        State::UnderwaterSwimming => "ao-line-underwater-swimming",
        State::SwimmingUp => "ao-line-swimming-up",
        State::SwimmingDown => "ao-line-swimming-down",
        State::UnderwaterFloating => "ao-line-underwater-floating",
        State::WaterEntry => "ao-line-water-entry",
        State::WaterFlightExit => "ao-line-water-flight-exit",
        State::WaterShoreExit => "ao-line-water-shore-exit",
        State::WaterDiveJump => "ao-line-water-dive-jump",
        State::WaterDiveEntry => "ao-line-water-dive-entry",
        State::WaterCannonball => "ao-line-water-cannonball",
        State::LegacySwimmingCrawl | State::LegacySwimmingBackward => "ao-line-swimming-style",
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    set: &Set,
    runtime: &mut Runtime,
    state: &mut AoUi,
    prefs: &mut crate::movement::Preferences,
    swim_ready: bool,
    out: &mut Vec<Action>,
) {
    let category = Category::of(state.state);
    let states: Vec<State> = shown(category, state.essentials).collect();
    if !states.contains(&state.state)
        && let Some(first) = states.first()
    {
        state.point_at(*first);
    }
    state.category_states[category.index()] = Some(state.state);
    ui.spacing_mut().item_spacing.y = 8.0;
    summary(ui, p, inv, set, category, &states, state);
    movement_card(ui, p, category, prefs, swim_ready, state, out);
    if category == Category::Water && !set.swimming_ready() {
        swimming_warning(ui, p);
    }
    ui.horizontal(|ui| {
        widgets::glyph(ui, "hand-finger", p.muted, 14.0);
        ui.label(RichText::new(aurora_i18n::tr!("ao-page-hint")).small().color(p.muted));
    });
    egui::ScrollArea::vertical()
        .id_salt(("ao_slots", set.id, category.index(), state.essentials))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (index, item) in states.iter().copied().enumerate() {
                let header = ui
                    .push_id(item.name(), |ui| section(ui, p, inv, set, runtime, item, state, out))
                    .inner;
                keyboard(ui, &header, &states, index, state);
            }
        });
}

/// The group's name and three tiles: how many slots are filled, how many
/// animations and creators, and what is still empty (a click opens it).
fn summary(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, set: &Set, category: Category, states: &[State], state: &mut AoUi) {
    let height = 64.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    let shade = super::shades(p);
    ui.painter().rect_filled(rect, radius, shade.card);
    let inner = rect.shrink2(Vec2::new(12.0, 8.0));
    let tiles_w = (inner.width() * 0.6).min(420.0);
    let text_room = inner.width() - tiles_w - 14.0;
    let title = egui::FontId::new(22.0, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(
        ui,
        category.label(),
        egui::pos2(inner.left(), inner.center().y - 10.0),
        text_room,
        title,
        p.ink,
    );
    widgets::paint_elided(
        ui,
        category.description(),
        egui::pos2(inner.left(), inner.center().y + 12.0),
        text_room,
        egui::FontId::proportional(12.0),
        p.muted,
    );
    let filled_states: Vec<bool> = states.iter().map(|s| filled(set, *s)).collect();
    let filled_n = filled_states.iter().filter(|f| **f).count();
    let empty: Vec<State> = states.iter().copied().filter(|s| !filled(set, *s)).collect();
    let anims: usize = states.iter().map(|s| slot_animations(set, *s)).sum();
    let gap = 6.0;
    let tile_w = (tiles_w - 2.0 * gap) / 3.0;
    let tile = |i: usize| {
        egui::Rect::from_min_size(
            egui::pos2(inner.right() - tiles_w + i as f32 * (tile_w + gap), inner.top()),
            Vec2::new(tile_w, inner.height()),
        )
    };
    let tile_radius = ui.visuals().widgets.inactive.corner_radius;
    // Filled, with one mark per slot.
    let r = tile(0);
    ui.painter().rect_filled(r, tile_radius, shade.open);
    figure(
        ui,
        p,
        r,
        &filled_n.to_string(),
        &aurora_i18n::tr!("ao-of-filled", total = states.len()),
    );
    let marks = filled_states.len().max(1) as f32;
    let mark_w = ((r.width() - 20.0 - 2.0 * (marks - 1.0)) / marks).max(2.0);
    for (i, on) in filled_states.iter().enumerate() {
        let x = r.left() + 10.0 + i as f32 * (mark_w + 2.0);
        let mark = egui::Rect::from_min_size(egui::pos2(x, r.bottom() - 13.0), Vec2::new(mark_w, 4.0));
        ui.painter()
            .rect_filled(mark, tile_radius, if *on { p.violet_light } else { p.field });
    }
    // Animations and their creators.
    let r = tile(1);
    ui.painter().rect_filled(r, tile_radius, shade.open);
    figure(ui, p, r, &anims.to_string(), &aurora_i18n::tr!("ao-animations-word", count = anims));
    let makers = creators(inv, set, states);
    let line = if makers > 0 {
        aurora_i18n::tr!("ao-from-creators", count = makers)
    } else {
        aurora_i18n::tr!("ao-none-yet").to_owned()
    };
    widgets::paint_elided(
        ui,
        &line,
        egui::pos2(r.left() + 10.0, r.bottom() - 12.0),
        r.width() - 20.0,
        egui::FontId::proportional(11.0),
        p.muted,
    );
    // Still empty, or all filled.
    let r = tile(2);
    if let Some(first) = empty.first().copied() {
        let response = ui
            .interact(r, ui.scope_id().with("ao_first_empty"), egui::Sense::click())
            .on_hover_text(aurora_i18n::tr!("ao-go-to", slot = first.label()));
        let fill = if response.hovered() { shade.open_hot } else { shade.open };
        ui.painter().rect_filled(r, tile_radius, fill);
        figure(ui, p, r, &empty.len().to_string(), aurora_i18n::tr!("ao-still-empty"));
        let next = format!("{} →", first.label());
        widgets::paint_elided(
            ui,
            &next,
            egui::pos2(r.left() + 10.0, r.bottom() - 12.0),
            r.width() - 20.0,
            egui::FontId::proportional(11.0),
            p.violet_pale,
        );
        if response.clicked() {
            state.select_state(first);
        }
    } else {
        ui.painter().rect_filled(r, tile_radius, shade.open);
        widgets::paint_glyph(
            ui.painter(),
            "circle-check-filled",
            egui::pos2(r.left() + 18.0, r.top() + 16.0),
            16.0,
            p.success,
        );
        widgets::paint_elided(
            ui,
            aurora_i18n::tr!("ao-all-filled"),
            egui::pos2(r.left() + 10.0, r.bottom() - 12.0),
            r.width() - 20.0,
            egui::FontId::proportional(11.5),
            p.muted,
        );
    }
}

/// A tile's figure in bold and its word after it.
fn figure(ui: &egui::Ui, p: &Palette, r: egui::Rect, number: &str, word: &str) {
    let bold = egui::FontId::new(16.0, super::super::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(number.to_owned(), bold, p.ink);
    let y = r.top() + 16.0;
    let width = galley.size().x;
    widgets::paint_row_text(ui.painter(), galley, r.left() + 10.0, y, p.ink);
    widgets::paint_elided(
        ui,
        word,
        egui::pos2(r.left() + 15.0 + width, y),
        r.width() - 25.0 - width,
        egui::FontId::proportional(11.5),
        p.muted,
    );
}

/// The movement enhancement that goes with Ground, Flying or Swimming, in
/// one line, with the way to its full settings.
fn movement_card(
    ui: &mut egui::Ui,
    p: &Palette,
    category: Category,
    prefs: &mut crate::movement::Preferences,
    swim_ready: bool,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    let (icon, value, label, ready) = match category {
        Category::Ground => ("run", &mut prefs.running, aurora_i18n::tr!("movement-running"), true),
        Category::Flight => (
            "plane-tilt-filled",
            &mut prefs.fly_assist,
            aurora_i18n::tr!("movement-fly-assist"),
            true,
        ),
        Category::Water => ("swimming", &mut prefs.swimming, aurora_i18n::tr!("ao-swimming-switch"), swim_ready),
        _ => return,
    };
    widgets::surface(ui, p, |ui| {
        ui.horizontal(|ui| {
            widgets::glyph(ui, icon, p.violet_pale, 15.0);
            let before = *value;
            ui.add_enabled_ui(ready || *value, |ui| widgets::switch_labeled(ui, p, value, label));
            if *value != before {
                out.push(Action::MovementPreferences);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new(aurora_i18n::tr!("ao-open-movement")).color(p.violet_pale)).frame(false))
                    .clicked()
                {
                    state.tab = 1;
                }
                ui.label(RichText::new(aurora_i18n::tr!("ao-experimental")).small().color(p.muted));
            });
        });
    });
}

/// One slot: its title line, then what it holds (closed) or its editor (open).
#[allow(clippy::too_many_arguments)]
fn section(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    set: &Set,
    runtime: &mut Runtime,
    item: State,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) -> egui::Response {
    let open = state.state == item && state.slot_open && state.tab == 0;
    // unfolding and folding glide (`reveal`)
    let time = if ui.style().animation_time > 0.0 { REVEAL } else { 0.0 };
    let t = ui
        .ctx()
        .animate_bool_with_time(ui.scope_id().with(("ao_slot_open", item.name())), open, time);
    let count = slot_animations(set, item);
    let background = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().min;
    let width = ui.available_width();
    let inner = ui
        .scope(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            let header = header(ui, p, runtime, item, count, open);
            if t > 0.0 {
                reveal(ui, ui.scope_id().with(("ao_slot_body", item.name())), t, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 10,
                            right: 10,
                            top: 2,
                            bottom: 10,
                        })
                        .show(ui, |ui| slot::show(ui, p, inv, set, runtime, state, out));
                });
            } else {
                peek(ui, p, set, item);
            }
            header
        })
        .inner;
    let rect = egui::Rect::from_min_size(top, Vec2::new(width, ui.min_rect().bottom() - top.y));
    let radius = ui.visuals().window_corner_radius;
    let shape = if open || t > 0.0 {
        egui::Shape::Rect(egui::epaint::RectShape::new(
            rect,
            radius,
            super::shades(p).open,
            widgets::border_stroke(p, egui::Stroke::new(1.0, super::shades(p).edge)),
            egui::StrokeKind::Inside,
        ))
    } else if count == 0 {
        egui::Shape::Noop
    } else {
        egui::Shape::Rect(egui::epaint::RectShape::filled(rect, radius, super::shades(p).slot))
    };
    ui.painter().set(background, shape);
    if count == 0 && !open && t == 0.0 {
        widgets::dashed_outline(ui.painter(), rect, p.muted.gamma_multiply(0.45));
    }
    // a click on the title unfolds the slot, and folds it again
    if inner.clicked() {
        if open {
            state.slot_open = false;
        } else {
            state.select_state(item);
        }
    }
    // The whole slot takes animations from the inventory.
    let container = if open { state.container } else { None };
    slot::inventory_drop(ui, p, inv, set, item, container, rect, state, out);
    inner
}

/// The body of a slot unfolding (`t` from 0 folded to 1 open): drawn whole,
/// shown down to that share of its height, the rest of the page following.
fn reveal(ui: &mut egui::Ui, id: egui::Id, t: f32, body: impl FnOnce(&mut egui::Ui)) {
    let height_id = id.with("height");
    let full: f32 = ui.data(|d| d.get_temp(height_id)).unwrap_or(0.0);
    let start = ui.cursor().min;
    let width = ui.available_width();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(start, Vec2::new(width, f32::INFINITY)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    if t < 1.0 {
        child.set_clip_rect(egui::Rect::from_min_size(start, Vec2::new(width, full * t)).intersect(ui.clip_rect()));
    }
    body(&mut child);
    let measured = child.min_rect().height();
    ui.data_mut(|d| d.insert_temp(height_id, measured));
    let shown = if t < 1.0 { (full * t).min(measured) } else { measured };
    ui.allocate_rect(egui::Rect::from_min_size(start, Vec2::new(width, shown)), egui::Sense::hover());
}

/// Up and Down on a slot's title move to the slot before or after it, inside
/// the group, wrapping at its ends, as the old state list did.
fn keyboard(ui: &egui::Ui, header: &egui::Response, states: &[State], index: usize, state: &mut AoUi) {
    let item = states[index];
    let pending = state.state_focus == Some(item);
    if pending {
        header.request_focus();
        header.scroll_to_me(Some(egui::Align::Center));
        if header.has_focus() && ui.is_rect_visible(header.rect) {
            state.state_focus = None;
        }
    }
    if !header.has_focus() && !pending {
        return;
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            header.id,
            egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
    let delta = ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            Some(1)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            Some(-1)
        } else {
            None
        }
    });
    if let Some(delta) = delta {
        let next = states[(index as isize + delta).rem_euclid(states.len() as isize) as usize];
        state.select_state(next);
        state.state_focus = Some(next);
    }
}

/// The slot's mark, name, what it is for, whether it plays now, and its
/// count.
fn header(ui: &mut egui::Ui, p: &Palette, runtime: &Runtime, item: State, count: usize, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER), egui::Sense::click());
    let inner = rect.shrink2(Vec2::new(10.0, 0.0));
    let y = inner.center().y;
    let core_swim = State::CORE_SWIMMING.contains(&item);
    let (icon, tint, tip) = if count > 0 {
        ("circle-check-filled", p.success, None)
    } else if core_swim {
        ("alert-triangle-filled", p.warn, Some(aurora_i18n::tr!("ao-swim-required-missing")))
    } else {
        ("circle-dashed", p.muted, None)
    };
    widgets::paint_glyph(ui.painter(), icon, egui::pos2(inner.left() + 8.0, y), 16.0, tint);
    let name_font = egui::FontId::new(13.5, super::super::fonts::semibold(ui.ctx()));
    let name = ui.painter().layout_no_wrap(item.label().to_owned(), name_font, p.ink);
    let name_w = name.size().x;
    let x = inner.left() + 24.0;
    widgets::paint_row_text(ui.painter(), name, x, y, p.ink);
    let count_text = if count == 0 {
        aurora_i18n::tr!("ao-slot-default").to_owned()
    } else {
        aurora_i18n::tr!("ao-animation-count", count = count)
    };
    let count_galley = ui.painter().layout_no_wrap(count_text, egui::FontId::proportional(11.5), p.muted);
    let mut right = inner.right() - count_galley.size().x;
    widgets::paint_row_text(ui.painter(), count_galley, right, y, p.muted);
    if runtime.current == Some(item) && count > 0 {
        let label = aurora_i18n::tr!("ao-playing-now");
        let galley = ui
            .painter()
            .layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.0), p.teal);
        right -= galley.size().x + 14.0;
        ui.painter().circle_filled(egui::pos2(right - 6.0, y), 3.0, p.teal);
        widgets::paint_row_text(ui.painter(), galley, right, y, p.teal);
        right -= 10.0;
    }
    let desc_x = x + name_w + 8.0;
    let room = right - 10.0 - desc_x;
    if room > 24.0 {
        widgets::paint_elided(
            ui,
            description(item),
            egui::pos2(desc_x, y),
            room,
            egui::FontId::proportional(12.0),
            p.muted,
        );
    }
    let response = match tip {
        Some(tip) => response.on_hover_text(tip),
        None => response,
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, item.label()));
    response
}

/// A closed slot's entries as chips, or what plays while it is empty.
fn peek(ui: &mut egui::Ui, p: &Palette, set: &Set, item: State) {
    let Some(config) = set.states.get(&item).filter(|_| filled(set, item)) else {
        ui.horizontal(|ui| {
            ui.add_space(34.0);
            ui.label(RichText::new(aurora_i18n::tr!("ao-slot-empty-hint")).small().color(p.muted));
        });
        ui.add_space(4.0);
        return;
    };
    let names: Vec<&str> = config
        .steps
        .iter()
        .map(|s| s.name.as_str())
        .chain(config.tracks.iter().map(|t| t.name.as_str()))
        .collect();
    ui.horizontal_wrapped(|ui| {
        ui.add_space(34.0);
        ui.spacing_mut().item_spacing.x = 5.0;
        for name in names.iter().take(PEEK) {
            slot::chip(ui, p, name);
        }
        if names.len() > PEEK {
            ui.label(
                RichText::new(aurora_i18n::tr!("ao-more-count", count = names.len() - PEEK))
                    .small()
                    .color(p.muted),
            );
        }
    });
    ui.add_space(4.0);
}

/// The EXPERIMENTAL tag, its right edge at `right` (vertically centred on
/// it): small capitals in a thin border. `on`: on the accent. Returns its
/// left edge.
pub(super) fn experimental_tag(ui: &egui::Ui, p: &Palette, right: egui::Pos2, on: bool) -> f32 {
    let ink = if on { p.on_accent } else { p.violet_pale };
    let font = egui::FontId::new(10.0, super::super::fonts::semibold(ui.ctx()));
    let galley = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("ao-experimental").to_uppercase(), font, ink);
    let size = galley.size() + Vec2::new(12.0, 6.0);
    let rect = egui::Rect::from_min_size(egui::pos2(right.x - size.x, right.y - size.y / 2.0), size);
    ui.painter().rect_stroke(
        rect,
        ui.visuals().widgets.inactive.corner_radius,
        egui::Stroke::new(1.0, ink.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    widgets::paint_ink_centered(ui.painter(), galley, rect.center(), ink);
    rect.left()
}

/// The Movement page: the experimental enhancements, apart from the AO.
pub(super) fn movement(
    ui: &mut egui::Ui,
    p: &Palette,
    movement: &mut super::super::movement::MovementUi,
    prefs: &mut crate::movement::Preferences,
    keys: &crate::keybinds::KeyBindings,
    status: &str,
    out: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("ao-tab-movement-short")).size(22.0).strong());
        let (tag, _) = ui.allocate_exact_size(Vec2::new(96.0, 22.0), egui::Sense::hover());
        experimental_tag(ui, p, egui::pos2(tag.right() - 4.0, tag.center().y), false);
    });
    ui.label(RichText::new(aurora_i18n::tr!("ao-movement-intro")).color(p.muted));
    let actions = egui::ScrollArea::vertical()
        .id_salt("ao_movement")
        .auto_shrink([false, false])
        .show(ui, |ui| movement.show(ui, p, prefs, keys, status))
        .inner;
    if actions.changed {
        out.push(Action::MovementPreferences);
    }
}
