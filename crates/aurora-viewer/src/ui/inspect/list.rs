//! The Outfit (Objects) and Creators lists of Inspect: one card per object,
//! one line each (name, complexity badge, creator's picture), opened in place
//! on its creator, where it is worn, its figures and its prims.

use super::model::{self, Area, Item, Tier, Whole};
use super::{Action, CreatorShown, Frame, InspectUi, Row, Subject, megabytes};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Height of a closed card with one line, and with the creator's name under it.
const ROW: f32 = 34.0;
const ROW_TWO_LINES: f32 = 44.0;
const PICTURE: f32 = 18.0;

pub(super) fn show(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, items: &[Item], actions: &mut Vec<Action>) {
    let rows = frame.rows;
    let view = *frame.view;
    let tiers: HashMap<Uuid, Tier> = items.iter().map(|i| i.root).zip(model::tiers(items)).collect();
    let list = model::shown(rows, items, &state.query, view.sort);
    let whole = Whole::of(items);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if list.is_empty() {
            let text = if items.is_empty() || state.query.trim().is_empty() {
                aurora_i18n::tr!("inspect-empty").to_owned()
            } else {
                aurora_i18n::tr!("inspect-no-match", query = state.query.trim())
            };
            ui.add_space(12.0);
            ui.label(RichText::new(text).size(12.0).color(p.muted));
            return;
        }
        if state.tab == 1 {
            creators(state, ui, p, frame, &list, &tiers, &whole, actions);
            return;
        }
        let card_opts = CardOpts {
            creator: view.creator,
            worn_on: view.worn_on && state.avatar(),
            complexity: view.complexity,
        };
        if view.sort == model::Sort::Area && state.avatar() {
            for area in Area::ORDER {
                let section: Vec<&Item> = list.iter().copied().filter(|i| i.area == area).collect();
                if section.is_empty() {
                    continue;
                }
                let complexity: u32 = section.iter().map(|i| i.complexity).sum();
                heading(ui, p, area.title(), section.len(), view.complexity.then_some(complexity));
                for item in section {
                    card(state, ui, p, frame, item, tiers[&item.root], card_opts, &whole, actions);
                }
            }
        } else {
            ui.add_space(2.0);
            for item in &list {
                card(state, ui, p, frame, item, tiers[&item.root], card_opts, &whole, actions);
            }
        }
    });
}

/// A section's heading: its name, how many items, their complexity.
fn heading(ui: &mut egui::Ui, p: &Palette, title: &str, count: usize, complexity: Option<u32>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.set_height(22.0);
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(12.5).strong().color(p.ink));
        ui.label(
            RichText::new(aurora_i18n::tr!("inspect-summary-items", count = count))
                .size(11.5)
                .color(p.muted),
        );
        if let Some(value) = complexity {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let text = aurora_i18n::tr!("inspect-section-complexity", value = model::short(value));
                ui.label(RichText::new(text).size(11.5).color(p.muted));
            });
        }
    })
    .response
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, title));
    ui.add_space(4.0);
}

/// The Creators tab: who made what, most items first.
fn creators(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    list: &[&Item],
    tiers: &HashMap<Uuid, Tier>,
    whole: &Whole,
    actions: &mut Vec<Action>,
) {
    let rows = frame.rows;
    // by creator id, or by the shown name while the id is not known
    let mut groups: Vec<(Option<Uuid>, String, Vec<&Item>)> = Vec::new();
    for item in list {
        let row = &rows[item.first];
        match groups.iter_mut().find(|g| g.0 == row.creator && g.1 == row.creator_name) {
            Some(g) => g.2.push(item),
            None => groups.push((row.creator, row.creator_name.clone(), vec![item])),
        }
    }
    groups.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then_with(|| crate::natural_order::cmp(&a.1, &b.1)));
    let radius = ui.visuals().window_corner_radius;
    let opts = CardOpts {
        creator: CreatorShown::Hidden,
        worn_on: state.avatar(),
        complexity: frame.view.complexity,
    };
    for (creator, name, items) in groups {
        ui.add_space(6.0);
        let named_here_alone = rows[items[0].first].creator_named_here_alone;
        egui::Frame::new().fill(p.bar).corner_radius(radius).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (pic, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                if let Some(id) = creator {
                    state.wanted_pics.insert(id);
                }
                widgets::avatar_picture(ui, p, pic, creator.and_then(|id| frame.pics.get(&id)), &name, false);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(&name).size(13.0).strong().color(p.ink));
                    let complexity: u32 = items.iter().map(|i| i.complexity).sum();
                    let line = [
                        aurora_i18n::tr!("inspect-summary-items", count = items.len()),
                        aurora_i18n::tr!("inspect-section-complexity", value = model::short(complexity)),
                    ]
                    .join("  ·  ");
                    ui.label(RichText::new(line).size(11.5).color(p.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = creator.is_some();
                    if widgets::action_button_if(
                        ui,
                        p,
                        "user-filled",
                        aurora_i18n::tr!("common-profile"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                        enabled,
                    )
                    .clicked()
                        && let Some(who) = creator
                    {
                        actions.push(Action::Profile { who, named_here_alone });
                    }
                });
            });
        });
        ui.add_space(5.0);
        for item in items {
            card(state, ui, p, frame, item, tiers[&item.root], opts, whole, actions);
        }
    }
}

#[derive(Clone, Copy)]
struct CardOpts {
    creator: CreatorShown,
    worn_on: bool,
    complexity: bool,
}

/// What a screen reader says of a card: one sentence.
fn spoken(row: &Row, item: &Item, point: Option<&str>) -> String {
    let complexity = aurora_i18n::integer(i64::from(item.complexity));
    match point {
        Some(point) => aurora_i18n::tr!(
            "inspect-spoken-worn",
            name = row.name.trim(),
            point = point,
            complexity = complexity,
            creator = row.creator_name.as_str()
        ),
        None => aurora_i18n::tr!(
            "inspect-spoken-object",
            name = row.name.trim(),
            owner = row.owner_name.as_str(),
            complexity = complexity,
            creator = row.creator_name.as_str()
        ),
    }
}

/// The attachment point's name (avatar_lad.xml), as the Appearance window
/// names it.
fn point_label(frame: &Frame<'_>, point: u8) -> String {
    frame
        .point_names
        .get(&point)
        .map(|name| crate::ui::appearance::items::point_name(name).to_owned())
        .unwrap_or_else(|| aurora_i18n::tr!("inspect-area-other").to_owned())
}

/// One object: its line, and once opened, its details under it on the same
/// outlined card.
#[allow(clippy::too_many_arguments)]
fn card(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    item: &Item,
    tier: Tier,
    opts: CardOpts,
    whole: &Whole,
    actions: &mut Vec<Action>,
) {
    let row = &frame.rows[item.first];
    let open = state.opened == Some(item.root);
    let radius = ui.visuals().window_corner_radius;
    let (fill, stroke) = if open {
        (p.violet.gamma_multiply(0.13), Stroke::new(1.0, p.violet))
    } else {
        (p.raised.gamma_multiply(0.45), Stroke::new(1.0, egui::Color32::TRANSPARENT))
    };
    let point = (item.point != 0).then(|| point_label(frame, item.point));
    egui::Frame::new().fill(fill).stroke(stroke).corner_radius(radius).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let two_lines = opts.creator == CreatorShown::Name;
        let height = if two_lines { ROW_TWO_LINES } else { ROW };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
        let pointed = ui.rect_contains_pointer(rect);
        if pointed && !open {
            ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
        }
        // from the right: the chevron, the picture or the hover actions, the
        // badge, where it is worn; the name takes what is left
        let mut right = rect.right() - 10.0;
        let chevron = if open { "chevron-down" } else { "chevron-right" };
        let tint = if open { p.violet_light } else { p.muted };
        crate::ui::sheet::icon_at(ui, chevron, egui::pos2(right - 7.0, rect.center().y), 14.0, tint);
        right -= 22.0;
        let mut on_action = false;
        let quick = (pointed || resp.has_focus()) && !open;
        if quick {
            let buttons = [
                (
                    "eye-filled",
                    aurora_i18n::tr!("inspect-look-at"),
                    aurora_i18n::tr!("inspect-look-at-tip"),
                    true,
                ),
                (
                    "user-filled",
                    aurora_i18n::tr!("common-profile"),
                    aurora_i18n::tr!("inspect-profile-tip"),
                    row.creator.is_some(),
                ),
            ];
            for (i, (icon, label, tip, enabled)) in buttons.iter().enumerate().rev() {
                let width = ghost_width(ui, label);
                let r = Rect::from_min_max(
                    egui::pos2(right - width, rect.center().y - 13.0),
                    egui::pos2(right, rect.center().y + 13.0),
                );
                right -= width + 4.0;
                let b = ui.interact(
                    r,
                    ui.scope_id().with(("inspect_quick", item.root, i)),
                    if *enabled { Sense::click() } else { Sense::hover() },
                );
                on_action |= b.hovered();
                ghost(ui, p, r, icon, label, b.hovered(), *enabled);
                let b = b.on_hover_text(*tip);
                if b.clicked() {
                    if i == 0 {
                        actions.push(Action::LookAt(item.root));
                    } else if let Some(who) = row.creator {
                        actions.push(Action::Profile {
                            who,
                            named_here_alone: row.creator_named_here_alone,
                        });
                    }
                }
            }
        } else if opts.creator == CreatorShown::Picture {
            let pic = Rect::from_center_size(egui::pos2(right - PICTURE / 2.0, rect.center().y), Vec2::splat(PICTURE));
            if let Some(id) = row.creator {
                state.wanted_pics.insert(id);
            }
            widgets::avatar_picture(ui, p, pic, row.creator.and_then(|id| frame.pics.get(&id)), &row.creator_name, false);
            if ui.rect_contains_pointer(pic) {
                resp.clone()
                    .on_hover_text(aurora_i18n::tr!("inspect-made-by-tip", name = row.creator_name.as_str()));
            }
            right -= PICTURE + 8.0;
        }
        if opts.complexity {
            right = badge(ui, p, right, rect.center().y, item.complexity, tier) - 8.0;
        }
        if opts.worn_on
            && let Some(point) = &point
        {
            let font = egui::FontId::proportional(11.5);
            let galley = ui.painter().layout_no_wrap(point.clone(), font.clone(), p.muted);
            let width = galley.size().x.min(96.0);
            widgets::paint_elided(ui, point, egui::pos2(right - width, rect.center().y), width, font, p.muted);
            right -= width + 8.0;
        }
        let left = rect.left() + 10.0;
        let name_width = (right - left).max(20.0);
        if two_lines {
            let y1 = rect.top() + 14.0;
            widgets::paint_elided(
                ui,
                row.name.trim(),
                egui::pos2(left, y1),
                name_width,
                egui::FontId::proportional(13.0),
                p.ink,
            );
            let creator_pic = Rect::from_center_size(egui::pos2(left + 7.0, rect.bottom() - 13.0), Vec2::splat(14.0));
            if let Some(id) = row.creator {
                state.wanted_pics.insert(id);
            }
            widgets::avatar_picture(
                ui,
                p,
                creator_pic,
                row.creator.and_then(|id| frame.pics.get(&id)),
                &row.creator_name,
                false,
            );
            let cl = egui::pos2(creator_pic.right() + 5.0, creator_pic.center().y);
            widgets::paint_elided(
                ui,
                &row.creator_name,
                cl,
                name_width - 19.0,
                egui::FontId::proportional(11.5),
                p.muted,
            );
        } else {
            widgets::paint_elided(
                ui,
                row.name.trim(),
                egui::pos2(left, rect.center().y),
                name_width,
                egui::FontId::proportional(13.0),
                p.ink,
            );
        }
        let label = spoken(row, item, point.as_deref().filter(|_| state.avatar()));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if resp.clicked() && !on_action {
            state.opened = if open { None } else { Some(item.root) };
            state.selected = Some(item.root);
        }
        if open {
            body(state, ui, p, frame, item, point.as_deref(), whole, actions);
        }
    });
    ui.add_space(5.0);
}

/// The complexity badge, its right edge at `right`: grey, white from
/// "medium", orange for the heaviest (the land impact badge of Area Search).
/// Returns its left edge.
fn badge(ui: &egui::Ui, p: &Palette, right: f32, y: f32, value: u32, tier: Tier) -> f32 {
    let (face, ink) = match tier {
        Tier::Light => (p.raised.gamma_multiply(0.7), p.muted),
        Tier::Medium => (p.raised, p.ink),
        Tier::Heavy => (p.warn.gamma_multiply(0.16), p.warn),
    };
    let text = model::short(value);
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(11.0), ink);
    let width = (galley.size().x + 12.0).max(30.0);
    let r = Rect::from_min_max(egui::pos2(right - width, y - 9.0), egui::pos2(right, y + 9.0));
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, face);
    widgets::paint_ink_left(
        ui.painter(),
        galley.clone(),
        egui::pos2(r.center().x - galley.size().x / 2.0, y),
        ink,
    );
    r.left()
}

fn ghost_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), egui::Color32::PLACEHOLDER);
    galley.size().x + 16.0 + 6.0 + 20.0
}

/// A quiet action on a card's line: icon and label, a face only when pointed at.
fn ghost(ui: &egui::Ui, p: &Palette, r: Rect, icon: &str, label: &str, hovered: bool, enabled: bool) {
    if hovered && enabled {
        ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = match (enabled, hovered) {
        (false, _) => p.muted_dim,
        (true, true) => p.ink,
        (true, false) => p.muted,
    };
    crate::ui::sheet::icon_at(ui, icon, egui::pos2(r.left() + 18.0, r.center().y), 16.0, ink);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), ink);
    widgets::paint_ink_left(ui.painter(), galley, egui::pos2(r.left() + 32.0, r.center().y), ink);
}

/// An opened card: description, Made by and Worn on (Owned by for objects
/// of the world), the figures with their share, and the prims to outline.
#[allow(clippy::too_many_arguments)]
fn body(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    item: &Item,
    point: Option<&str>,
    whole: &Whole,
    actions: &mut Vec<Action>,
) {
    let row = &frame.rows[item.first];
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if !row.description.trim().is_empty() {
                ui.label(RichText::new(row.description.trim()).size(12.0).color(p.muted));
            }
            ui.columns(2, |cols| {
                let made_by = aurora_i18n::tr!("inspect-made-by");
                if let Some(who) = person_card(&mut cols[0], p, frame, state, made_by, row.creator, &row.creator_name, true) {
                    actions.push(Action::Profile {
                        who,
                        named_here_alone: row.creator_named_here_alone,
                    });
                }
                match (state.subject, point) {
                    (Subject::Avatar(_), Some(point)) => {
                        if place_card(&mut cols[1], p, point) {
                            actions.push(Action::LookAt(item.root));
                        }
                    }
                    _ => {
                        let owned_by = aurora_i18n::tr!("inspect-owned-by");
                        if let Some(who) = person_card(
                            &mut cols[1],
                            p,
                            frame,
                            state,
                            owned_by,
                            row.owner,
                            &row.owner_name,
                            row.owner.is_some(),
                        ) {
                            actions.push(Action::Profile {
                                who,
                                named_here_alone: false,
                            });
                        }
                    }
                }
            });
            let outfit = matches!(state.subject, Subject::Avatar(_));
            let share = |part: u64, whole: u64| {
                let percent = model::share(part, whole);
                let sentence = if outfit {
                    aurora_i18n::tr!("inspect-share-outfit", percent = percent)
                } else {
                    aurora_i18n::tr!("inspect-share-selection", percent = percent)
                };
                (percent, sentence)
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inspect-technical"))
                        .size(11.5)
                        .strong()
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let prims = aurora_i18n::tr!("inspect-prims-count", count = item.prims.len());
                    ui.label(RichText::new(prims).size(11.5).color(p.muted));
                });
            });
            let n = |v: u32| aurora_i18n::integer(i64::from(v));
            let tiles_list = [
                (
                    n(item.complexity),
                    aurora_i18n::tr!("inspect-totals-complexity"),
                    Some(share(u64::from(item.complexity), whole.complexity)),
                ),
                (
                    n(item.triangles),
                    aurora_i18n::tr!("inspect-col-triangles"),
                    Some(share(u64::from(item.triangles), whole.triangles)),
                ),
                (
                    n(item.vertices),
                    aurora_i18n::tr!("inspect-col-vertices"),
                    Some(share(u64::from(item.vertices), whole.vertices)),
                ),
                (
                    n(item.faces),
                    aurora_i18n::tr!("inspect-col-faces"),
                    Some(share(u64::from(item.faces), whole.faces)),
                ),
                (
                    megabytes(item.texture_memory),
                    aurora_i18n::tr!("inspect-tile-tmem"),
                    Some(share(item.texture_memory, whole.texture_memory)),
                ),
                (
                    megabytes(item.vram),
                    aurora_i18n::tr!("inspect-tile-vram"),
                    Some(share(item.vram, whole.vram)),
                ),
            ];
            tiles(ui, p, &tiles_list, 3);
            ui.label(RichText::new(aurora_i18n::tr!("inspect-prims-hint")).size(11.5).color(p.muted));
            ui.spacing_mut().item_spacing.y = 0.0;
            for &i in &item.prims {
                let prim = &frame.rows[i];
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                let selected = state.selected == Some(prim.id);
                let radius = ui.visuals().widgets.inactive.corner_radius;
                if selected {
                    ui.painter().rect_filled(r, radius, p.violet.gamma_multiply(0.18));
                    ui.painter()
                        .rect_stroke(r, radius, Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, radius, p.raised.gamma_multiply(0.6));
                }
                let indent = if prim.root { 6.0 } else { 20.0 };
                let figure = n(prim.triangles);
                let galley = ui
                    .painter()
                    .layout_no_wrap(figure.clone(), egui::FontId::proportional(12.0), p.muted);
                let fw = galley.size().x;
                widgets::paint_ink_left(ui.painter(), galley, egui::pos2(r.right() - 6.0 - fw, r.center().y), p.muted);
                let font = egui::FontId::proportional(12.0);
                let ink = if prim.root { p.ink } else { p.muted };
                widgets::paint_elided(
                    ui,
                    prim.name.trim(),
                    egui::pos2(r.left() + indent, r.center().y),
                    r.width() - indent - fw - 18.0,
                    font,
                    ink,
                );
                let label = format!("{}, {} {}", prim.name.trim(), figure, aurora_i18n::tr!("inspect-col-triangles"));
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label));
                if resp.clicked() {
                    state.selected = Some(prim.id);
                }
            }
        });
}

/// A small card naming a person, with the button that opens their profile.
/// Returns the person whose profile was asked for.
#[allow(clippy::too_many_arguments)]
fn person_card(
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    state: &mut InspectUi,
    label: &str,
    who: Option<Uuid>,
    name: &str,
    can_open: bool,
) -> Option<Uuid> {
    let mut asked = None;
    small_card(ui, p, |ui| {
        let (pic, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
        if let Some(id) = who {
            state.wanted_pics.insert(id);
        }
        widgets::avatar_picture(ui, p, pic, who.and_then(|id| frame.pics.get(&id)), name, false);
        let text_w = (ui.available_width() - 34.0).max(20.0);
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(label).size(11.0).color(p.muted));
            ui.add(egui::Label::new(RichText::new(name).size(12.5).strong().color(p.ink)).truncate());
        });
        if can_open
            && let Some(id) = who
            && widgets::icon_toggle_sized(ui, p, "external-link-filled", aurora_i18n::tr!("inspect-open-profile"), false, 28.0)
        {
            asked = Some(id);
        }
    });
    asked
}

/// The "Worn on" card, with Look at. Returns true when Look at was clicked.
fn place_card(ui: &mut egui::Ui, p: &Palette, point: &str) -> bool {
    let mut look = false;
    small_card(ui, p, |ui| {
        let (pin, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
        ui.painter().circle_filled(pin.center(), 13.0, p.violet.gamma_multiply(0.18));
        crate::ui::sheet::icon_at(ui, "map-pin-filled", pin.center(), 14.0, p.violet_light);
        let text_w = (ui.available_width() - 34.0).max(20.0);
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(aurora_i18n::tr!("inspect-worn-on")).size(11.0).color(p.muted));
            ui.add(egui::Label::new(RichText::new(point).size(12.5).strong().color(p.ink)).truncate());
        });
        look = widgets::icon_toggle_sized(ui, p, "eye-filled", aurora_i18n::tr!("inspect-look-at-tip"), false, 28.0);
    });
    look
}

fn small_card(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui)) {
    let radius = ui.visuals().window_corner_radius;
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(radius)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                body(ui);
            });
        });
}

/// Figure tiles in `columns` columns: the value with its share on the
/// right, then its label. The share's sentence ("14% of the outfit") is the
/// tile's tooltip and what a screen reader says.
pub(super) fn tiles(ui: &mut egui::Ui, p: &Palette, list: &[(String, &str, Option<(u32, String)>)], columns: usize) {
    let radius = ui.visuals().window_corner_radius;
    let gap = 6.0;
    let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
    for chunk in list.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (value, label, share) in chunk {
                let tile = egui::Frame::new()
                    .fill(p.bar)
                    .corner_radius(radius)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(width - 16.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(value).size(13.5).strong().color(p.ink));
                                if let Some((percent, _)) = share {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let percent = aurora_i18n::tr!("inspect-percent", percent = *percent);
                                        ui.label(RichText::new(percent).size(11.5).color(p.muted));
                                    });
                                }
                            });
                            ui.add(egui::Label::new(RichText::new(*label).size(11.0).color(p.muted)).truncate());
                        });
                    });
                if let Some((_, sentence)) = share {
                    let spoken = format!("{value} {label}, {sentence}");
                    tile.response
                        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &spoken));
                    tile.response.on_hover_text(sentence);
                }
            }
        });
        ui.add_space(gap);
    }
}
