//! Quick sort: the friends in no contact set, one at a time, to put in
//! sets (Aurora's own; the Legacy Viewer adds residents to a set from its
//! panel or their menus only). The queue is taken when the window opens
//! (generateAvatarList's NO_SETS, online first); a click or the keys 1 to 9
//! choose the sets of the friend shown, Enter saves them and goes to the
//! next, S skips. Back returns to an earlier friend, their sets as they now
//! are. The upcoming friends are listed on the left.

use super::*;
use crate::keybinds::{Binding, Input};
use egui::{Pos2, Rect};
use std::collections::BTreeSet;
use winit::keyboard::KeyCode;

const WINDOW: &str = "contact_set_quick_sort";
const SIZE: Vec2 = Vec2::new(780.0, 560.0);
const QUEUE_W: f32 = 210.0;
/// The tiles' grid and the number keys' reach.
const COLUMNS: usize = 3;
const KEYED: usize = 9;

pub(super) struct QuickSort {
    /// The friends to sort, in their order.
    queue: Vec<Uuid>,
    /// The one shown (`queue.len()` once all are seen).
    index: usize,
    /// The sets chosen for the one shown, not saved yet.
    picks: BTreeSet<String>,
    /// For whom picks were loaded.
    loaded: Option<usize>,
    /// How many friends were put in a set.
    placed: BTreeSet<Uuid>,
    /// The keyboard to take on the next frame (the window just opened).
    grab: bool,
}

impl QuickSort {
    pub(super) fn new(world: &World) -> QuickSort {
        QuickSort {
            queue: queue(world),
            index: 0,
            picks: BTreeSet::new(),
            loaded: None,
            placed: BTreeSet::new(),
            grab: true,
        }
    }

    fn current(&self) -> Option<Uuid> {
        self.queue.get(self.index).copied()
    }

    /// The sets of the friend shown, as they are now, when they come.
    fn load(&mut self, world: &World) {
        // opened before the friend list came (a login, a demo): taken again
        if self.queue.is_empty() && self.index == 0 {
            self.queue = queue(world);
            self.loaded = None;
        }
        if self.loaded == Some(self.index) {
            return;
        }
        self.loaded = Some(self.index);
        self.picks = self
            .current()
            .map(|id| world.contact_sets.friend_sets(&id).into_iter().collect())
            .unwrap_or_default();
    }

    fn toggle(&mut self, set: &str) {
        if !self.picks.remove(set) {
            self.picks.insert(set.to_owned());
        }
    }

    /// The friend shown put in the sets chosen and out of the others, then
    /// the next one.
    fn save(&mut self, world: &mut World) {
        if let Some(id) = self.current() {
            apply_picks(world, id, &self.picks);
            if self.picks.is_empty() {
                self.placed.remove(&id);
            } else {
                self.placed.insert(id);
            }
        }
        self.step(1);
    }

    fn step(&mut self, by: isize) {
        self.index = self.index.saturating_add_signed(by).min(self.queue.len());
    }
}

/// The friends in no set, online first, then by name.
fn queue(world: &World) -> Vec<Uuid> {
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let mut ids = world
        .contact_sets
        .members(contact_sets::NO_SETS, &friends, |id| world.social.is_friend(id));
    let online = |id: &Uuid| world.social.friends.iter().any(|f| f.id == *id && f.online);
    ids.sort_by_cached_key(|id| (!online(id), crate::natural_order::key(&world.social.name_of(id))));
    ids
}

/// `id` in the sets of `picks` and out of the others.
fn apply_picks(world: &mut World, id: Uuid, picks: &BTreeSet<String>) {
    let social = &world.social;
    let is_friend = |id: &Uuid| social.is_friend(id);
    let sets = &mut world.contact_sets;
    for name in sets.set_names() {
        let member = sets.set(&name).is_some_and(|s| s.friends.contains(&id));
        if picks.contains(&name) && !member {
            sets.add_to_set(&[id], &name, is_friend);
        } else if !picks.contains(&name) && member {
            sets.remove_from_set(&[id], &name, is_friend);
        }
    }
}

/// What holds the keyboard for Quick sort's keys.
fn focus_id() -> egui::Id {
    egui::Id::unique("contact-set-quick-sort-keys")
}

fn plain(code: KeyCode) -> Binding {
    Binding {
        input: Input::key(code),
        ctrl: false,
        shift: false,
        alt: false,
    }
}

const NUMBER_KEYS: [(egui::Key, KeyCode); KEYED] = [
    (egui::Key::Num1, KeyCode::Digit1),
    (egui::Key::Num2, KeyCode::Digit2),
    (egui::Key::Num3, KeyCode::Digit3),
    (egui::Key::Num4, KeyCode::Digit4),
    (egui::Key::Num5, KeyCode::Digit5),
    (egui::Key::Num6, KeyCode::Digit6),
    (egui::Key::Num7, KeyCode::Digit7),
    (egui::Key::Num8, KeyCode::Digit8),
    (egui::Key::Num9, KeyCode::Digit9),
];

pub(super) fn window(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi, pics: &HashMap<Uuid, egui::TextureHandle>) {
    let Some(qs) = st.quick_sort.as_mut() else {
        return;
    };
    qs.load(world);
    // the keys, while the window is in front and no field takes them
    // The keys while the window holds the keyboard: its focus keeps them
    // from the viewer's own shortcuts (S walks back, Enter opens the chat).
    let keys_ours = widgets::floater_holds_keyboard(ctx, WINDOW) && ctx.memory(|m| m.focused()) == Some(focus_id());
    let names = world.contact_sets.ordered_set_names();
    if keys_ours && qs.current().is_some() {
        ctx.input_mut(|i| {
            for (n, (key, _)) in NUMBER_KEYS.iter().enumerate() {
                if let Some(name) = names.get(n)
                    && i.consume_key(egui::Modifiers::NONE, *key)
                {
                    qs.toggle(name);
                }
            }
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            qs.save(world);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::S)) {
            qs.step(1);
        }
    }
    let mut open = true;
    let mut new_set = None;
    let mut restart = false;
    widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("people-sets-quick-sort"),
        ctx.content_rect().center() - SIZE / 2.0,
        SIZE,
    )
    .flush_body()
    .show(ctx, p, &mut open, |ui| {
        let full = ui.available_rect_before_wrap();
        // the window takes the keyboard when it opens and on each click in it
        // (a click on a tile would otherwise leave the keys to the world)
        let holder = ui.interact(full, focus_id(), Sense::focusable_noninteractive());
        let clicked_in = ui.input(|i| {
            (i.pointer.any_pressed() || i.pointer.any_released()) && i.pointer.interact_pos().is_some_and(|at| full.contains(at))
        });
        if std::mem::take(&mut qs.grab) || clicked_in {
            holder.request_focus();
        }
        // progress, along the top
        let done = qs.index.min(qs.queue.len()) as f32 / qs.queue.len().max(1) as f32;
        let track = Rect::from_min_size(full.min, Vec2::new(full.width(), 3.0));
        ui.painter().rect_filled(track, 0.0, p.field);
        ui.painter()
            .rect_filled(Rect::from_min_size(track.min, Vec2::new(track.width() * done, 3.0)), 0.0, p.violet);
        let body = Rect::from_min_max(Pos2::new(full.left(), track.bottom()), full.max);
        if qs.current().is_none() {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body.shrink(24.0))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            restart = finished(&mut child, p, qs);
        } else {
            let side = Rect::from_min_max(body.min, Pos2::new(body.left() + QUEUE_W, body.bottom()));
            ui.painter().rect_filled(side, 0.0, p.bar);
            let mut side_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(side.shrink2(Vec2::new(12.0, 18.0)))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            up_next(&mut side_ui, p, world, qs, pics);
            let main = Rect::from_min_max(Pos2::new(side.right(), body.top()), body.max).shrink2(Vec2::new(28.0, 22.0));
            let mut main_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(main)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            new_set = current(&mut main_ui, p, world, qs, &names, pics);
        }
        ui.allocate_rect(full, Sense::hover());
    });
    if restart {
        st.quick_sort = Some(QuickSort::new(world));
    }
    if let Some(id) = new_set {
        st.ask_new_set_for(vec![id]);
    }
    if !open {
        st.quick_sort = None;
    }
}

/// The friends of the queue: the one shown lit, the ones seen dimmed with
/// how many sets they got.
fn up_next(ui: &mut egui::Ui, p: &Palette, world: &World, qs: &QuickSort, pics: &HashMap<Uuid, egui::TextureHandle>) {
    ui.label(
        RichText::new(aurora_i18n::tr!("quick-sort-up-next").to_uppercase())
            .size(10.5)
            .color(p.muted)
            .family(crate::ui::fonts::semibold(ui.ctx())),
    );
    ui.add_space(8.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, id) in qs.queue.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
            if i == qs.index {
                ui.painter()
                    .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
                resp.scroll_to_me(None);
            }
            let name = world.social.name_of(id);
            let disc = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + 13.0, rect.center().y), Vec2::splat(26.0));
            widgets::avatar_disc(ui, p, disc, pics.get(id), &name, i < qs.index);
            let sets = world.contact_sets.friend_sets(id).len();
            let tag = (i < qs.index && sets > 0).then(|| aurora_i18n::tr!("quick-sort-sets", count = sets));
            let tag_g = tag.map(|t| ui.painter().layout_no_wrap(t, egui::FontId::proportional(11.0), p.muted));
            let tag_w = tag_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
            let ink = if i < qs.index { p.muted } else { p.ink };
            let mut job = egui::text::LayoutJob::single_section(name, egui::TextFormat::simple(egui::FontId::proportional(12.5), ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 6.0 - tag_w - disc.right() - 8.0).max(0.0));
            widgets::paint_row_text(ui.painter(), ui.painter().layout_job(job), disc.right() + 8.0, rect.center().y, ink);
            if let Some(g) = tag_g {
                let w = g.size().x;
                widgets::paint_row_text(ui.painter(), g, rect.right() - 6.0 - w, rect.center().y, p.muted);
            }
        }
    });
}

/// The friend shown, the tiles of the sets, and Back / Skip / Save and
/// next. Returns the friend a new set was asked for.
fn current(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    qs: &mut QuickSort,
    names: &[String],
    pics: &HashMap<Uuid, egui::TextureHandle>,
) -> Option<Uuid> {
    let id = qs.current()?;
    let mut new_set = None;
    let name = world.social.name_of(&id);
    let online = world.social.friends.iter().any(|f| f.id == id && f.online);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let (disc, _) = ui.allocate_exact_size(Vec2::splat(88.0), Sense::hover());
        widgets::avatar_disc(ui, p, disc, pics.get(&id), &name, false);
        crate::ui::presence::paint_dot(ui.painter(), p, disc, crate::ui::presence::of(world, &id), p.panel);
        ui.vertical(|ui| {
            ui.add_space(10.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("quick-sort-counter", index = qs.index + 1, total = qs.queue.len()))
                    .size(12.0)
                    .color(p.muted),
            );
            ui.label(
                RichText::new(&name)
                    .size(22.0)
                    .color(p.ink)
                    .family(crate::ui::fonts::semibold(ui.ctx())),
            );
            let status = if online {
                aurora_i18n::tr!("quick-sort-online")
            } else {
                aurora_i18n::tr!("quick-sort-offline")
            };
            ui.label(RichText::new(status).size(12.5).color(p.muted));
        });
    });
    ui.add_space(20.0);
    ui.label(RichText::new(aurora_i18n::tr!("quick-sort-put-in")).size(12.5).color(p.muted));
    ui.add_space(10.0);
    let gap = 8.0;
    let w = ((ui.available_width() - gap * (COLUMNS - 1) as f32) / COLUMNS as f32).floor();
    let mut tiles: Vec<Option<&String>> = names.iter().map(Some).collect();
    tiles.push(None);
    for (r, row) in tiles.chunks(COLUMNS).enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (c, tile) in row.iter().enumerate() {
                let n = r * COLUMNS + c;
                match tile {
                    Some(set) => {
                        if set_tile_button(ui, p, world, set, qs.picks.contains(*set), (n < KEYED).then_some(n), w) {
                            qs.toggle(set);
                        }
                    }
                    None => {
                        let resp = widgets::action_button(
                            ui,
                            p,
                            "plus",
                            aurora_i18n::tr!("quick-sort-new-set"),
                            widgets::ActionKind::Flat,
                            Vec2::new(w, 52.0),
                        );
                        if resp.clicked() {
                            new_set = Some(id);
                        }
                    }
                }
            }
        });
        ui.add_space(gap);
    }
    // the buttons, at the bottom
    let bottom = ui.max_rect().bottom();
    let rest = ui.available_rect_before_wrap();
    let row = Rect::from_min_max(
        Pos2::new(rest.left(), (bottom - 32.0).max(rest.top())),
        Pos2::new(rest.right(), bottom),
    );
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = 8.0;
    if widgets::action_button_if(
        &mut buttons,
        p,
        "arrow-left",
        aurora_i18n::tr!("quick-sort-back"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 32.0),
        qs.index > 0,
    )
    .clicked()
    {
        qs.step(-1);
    }
    buttons.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = if qs.picks.is_empty() {
            aurora_i18n::tr!("quick-sort-next")
        } else {
            aurora_i18n::tr!("quick-sort-save-next")
        };
        if widgets::action_button_keyed(
            ui,
            p,
            "",
            label,
            widgets::ActionKind::Primary,
            Vec2::new(0.0, 32.0),
            true,
            Some(&plain(KeyCode::Enter)),
        )
        .clicked()
        {
            qs.save(world);
        }
        if widgets::action_button_keyed(
            ui,
            p,
            "",
            aurora_i18n::tr!("quick-sort-skip"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, 32.0),
            true,
            Some(&plain(KeyCode::KeyS)),
        )
        .clicked()
        {
            qs.step(1);
        }
    });
    new_set
}

/// A set's tile of the grid: its tile, its name and count, its number key
/// or the tick of a set chosen. Returns true when clicked.
fn set_tile_button(ui: &mut egui::Ui, p: &Palette, world: &World, set: &str, on: bool, key: Option<usize>, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.18)
    } else if resp.hovered() {
        p.field.lerp_to_gamma(p.raised, 0.6)
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
    }
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 10.0 + 14.0, rect.center().y), Vec2::splat(28.0));
    if let Some(cfg) = world.contact_sets.set(set) {
        set_tile(ui, p, tile, cfg);
    }
    let end = rect.right() - 10.0;
    let mut right = end;
    if on {
        let check = Rect::from_center_size(Pos2::new(end - 9.0, rect.center().y), Vec2::splat(18.0));
        ui.painter().rect_filled(check, radius, p.violet);
        if let Some(t) = crate::ui::icons::global("check") {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter()
                .image(t.id(), Rect::from_center_size(check.center(), Vec2::splat(12.0)), uv, p.on_accent);
        }
        right = check.left() - 6.0;
    } else if let Some(n) = key {
        let caps = widgets::keycap::caps(ui, &plain(NUMBER_KEYS[n].1), p.field);
        let cw = widgets::keycap::row_width(&caps);
        widgets::keycap::paint_row(ui, p, &caps, Pos2::new(end - cw, rect.center().y), p.field);
        right = end - cw - 6.0;
    }
    let count = world.contact_sets.set(set).map_or(0, |s| s.friends.len());
    let width = (right - tile.right() - 10.0).max(0.0);
    let line = |text: String, format: egui::TextFormat| {
        let mut job = egui::text::LayoutJob::single_section(text, format);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        ui.painter().layout_job(job)
    };
    let name = line(
        set.to_owned(),
        crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 13.0, p.ink),
    );
    let sub = line(
        aurora_i18n::tr!("people-sets-people", count = count),
        egui::TextFormat::simple(egui::FontId::proportional(11.0), p.muted),
    );
    widgets::paint_row_text(ui.painter(), name, tile.right() + 10.0, rect.center().y - 7.0, p.ink);
    widgets::paint_row_text(ui.painter(), sub, tile.right() + 10.0, rect.center().y + 8.0, p.muted);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, set));
    resp.clicked()
}

/// Everyone seen: how many were put in a set; start again or close.
fn finished(ui: &mut egui::Ui, p: &Palette, qs: &QuickSort) -> bool {
    ui.add_space((ui.available_height() / 2.0 - 90.0).max(0.0));
    if let Some(t) = crate::ui::icons::global("circle-check-filled") {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, p.success);
    }
    ui.add_space(12.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("quick-sort-done-title"))
            .size(19.0)
            .color(p.ink)
            .family(crate::ui::fonts::semibold(ui.ctx())),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new(aurora_i18n::tr!(
            "quick-sort-done",
            placed = qs.placed.len(),
            total = qs.queue.len()
        ))
        .size(13.0)
        .color(p.muted),
    );
    ui.add_space(16.0);
    widgets::action_button(
        ui,
        p,
        "refresh",
        aurora_i18n::tr!("quick-sort-again"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 32.0),
    )
    .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with(friends: &[(u128, bool)]) -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for (n, online) in friends {
            world.social.friends.push(crate::world::social::Friend {
                id: Uuid::from_u128(*n),
                online: *online,
                rights_given: 1,
                rights_has: 1,
            });
            world.social.names.insert(Uuid::from_u128(*n), format!("Friend {n}"));
        }
        world
    }

    #[test]
    fn the_queue_is_the_friends_in_no_set_online_first() {
        let mut world = world_with(&[(1, false), (2, true), (3, false), (4, true)]);
        world.contact_sets.add_set("S");
        world.contact_sets.add_to_set(&[Uuid::from_u128(3)], "S", |_| true);
        let q = queue(&world);
        assert_eq!(q, [2, 4, 1].map(Uuid::from_u128));
    }

    #[test]
    fn saving_puts_the_friend_in_the_sets_chosen_and_out_of_the_others() {
        let mut world = world_with(&[(1, true), (2, true)]);
        for s in ["A", "B"] {
            world.contact_sets.add_set(s);
        }
        let mut qs = QuickSort::new(&world);
        qs.load(&world);
        assert!(qs.picks.is_empty());
        qs.toggle("A");
        qs.toggle("B");
        qs.toggle("B");
        qs.save(&mut world);
        let first = Uuid::from_u128(1);
        assert_eq!(world.contact_sets.friend_sets(&first), ["A"]);
        assert_eq!(qs.index, 1);
        // skipped: nothing changes
        qs.load(&world);
        qs.toggle("B");
        qs.step(1);
        assert!(world.contact_sets.friend_sets(&Uuid::from_u128(2)).is_empty());
        assert_eq!(qs.current(), None);
        // back to the first: their sets as they are, changed again
        qs.step(-2);
        qs.load(&world);
        assert_eq!(qs.picks.iter().cloned().collect::<Vec<_>>(), ["A"]);
        qs.toggle("A");
        qs.toggle("B");
        qs.save(&mut world);
        assert_eq!(world.contact_sets.friend_sets(&first), ["B"]);
        assert_eq!(qs.placed.len(), 1);
        // taken out of every set: no longer counted as placed
        qs.step(-1);
        qs.load(&world);
        qs.toggle("B");
        qs.save(&mut world);
        assert!(world.contact_sets.friend_sets(&first).is_empty());
        assert!(qs.placed.is_empty());
    }

    #[test]
    fn a_queue_opened_before_the_friend_list_fills_once_it_comes() {
        let mut world = world_with(&[]);
        let mut qs = QuickSort::new(&world);
        assert!(qs.queue.is_empty());
        world.social.friends.push(crate::world::social::Friend {
            id: Uuid::from_u128(7),
            online: true,
            rights_given: 1,
            rights_has: 1,
        });
        qs.load(&world);
        assert_eq!(qs.current(), Some(Uuid::from_u128(7)));
    }

    /// The real window over several frames: it takes the keyboard (so the
    /// viewer's shortcuts, S walking back, Enter opening the chat, do not
    /// get the keys), a number picks a set, Enter saves, S skips, and a
    /// click on a tile keeps the keyboard.
    #[test]
    fn the_keys_go_to_quick_sort_and_not_to_the_world() {
        let mut world = world_with(&[(1, true), (2, true), (3, true)]);
        for s in ["A", "B"] {
            world.contact_sets.add_set(s);
        }
        let p = crate::theme::Theme::default().palette();
        let mut st = ContactsUi::default();
        st.open_quick_sort(&world);
        let pics = HashMap::new();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        type F = crate::ui::widgets::elide_tests::Frames;
        let run = |frames: &mut F, world: &mut World, st: &mut ContactsUi, events: Vec<egui::Event>| {
            frames.run(events, |ui| window(ui.ctx(), &p, world, st, &pics));
        };
        for _ in 0..3 {
            run(&mut frames, &mut world, &mut st, Vec::new());
        }
        assert!(frames.ctx.egui_wants_keyboard_input(), "the world's shortcuts would get the keys");
        run(&mut frames, &mut world, &mut st, vec![key(egui::Key::Num1)]);
        run(&mut frames, &mut world, &mut st, vec![key(egui::Key::Enter)]);
        run(&mut frames, &mut world, &mut st, Vec::new());
        // the first friend (all online, by name) is in A, the second shown
        assert_eq!(world.contact_sets.friend_sets(&Uuid::from_u128(1)), ["A"]);
        run(&mut frames, &mut world, &mut st, vec![key(egui::Key::S)]);
        run(&mut frames, &mut world, &mut st, Vec::new());
        assert!(world.contact_sets.friend_sets(&Uuid::from_u128(2)).is_empty(), "skipped");
        // a click inside the window keeps the keyboard for the next key
        let inside = frames.ctx.content_rect().center();
        run(
            &mut frames,
            &mut world,
            &mut st,
            vec![
                egui::Event::PointerMoved(inside),
                egui::Event::PointerButton {
                    pos: inside,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        run(
            &mut frames,
            &mut world,
            &mut st,
            vec![egui::Event::PointerButton {
                pos: inside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(frames.ctx.egui_wants_keyboard_input(), "a click let the keyboard go");
        run(&mut frames, &mut world, &mut st, vec![key(egui::Key::Num2)]);
        run(&mut frames, &mut world, &mut st, vec![key(egui::Key::Enter)]);
        run(&mut frames, &mut world, &mut st, Vec::new());
        assert_eq!(world.contact_sets.friend_sets(&Uuid::from_u128(3)), ["B"]);
    }

    #[test]
    fn stepping_stays_within_the_queue() {
        let world = world_with(&[(1, true)]);
        let mut qs = QuickSort::new(&world);
        qs.step(-1);
        assert_eq!(qs.index, 0);
        qs.step(5);
        assert_eq!(qs.index, 1, "one past the end: finished");
    }
}
