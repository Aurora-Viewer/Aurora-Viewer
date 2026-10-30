//! Radial world menus after PieMenu::{show,handleHover,handleMouseButtonUp}
//! (Firestorm indra/newview/piemenu.cpp, originally LGPL 2.1) and menu_pie_*.xml.
//! Eight fixed directions, clockwise screen coordinates reversed to match SL,
//! disabled slices, click-through submenus. Aurora keeps its extra actions in
//! "More" and adds a center back button; it does not warp the system pointer.
//! At the user's request, right-click goes back instead of selecting a slice.

use super::entries::{Entry, Kind};
use super::{ContextMenu, CtxAction, Target};
use crate::theme::Palette;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::f32::consts::{FRAC_PI_4, PI, TAU};

const OUTER: f32 = 180.0;
const INNER: f32 = 36.0;
const SLICES: usize = 8;

fn blank() -> Entry {
    Entry {
        icon: "",
        label: String::new(),
        enabled: false,
        checked: None,
        kind: Kind::Unavailable,
    }
}

fn submenu(icon: &'static str, label: &str, entries: Vec<Entry>) -> Entry {
    Entry {
        icon,
        label: label.into(),
        enabled: true,
        checked: None,
        kind: Kind::Submenu(paginate(entries)),
    }
}

/// Every existing action stays reachable, even when a submenu exceeds eight.
fn paginate(entries: Vec<Entry>) -> Vec<Entry> {
    let mut entries: Vec<_> = entries.into_iter().filter(|e| !matches!(e.kind, Kind::Separator)).collect();
    for entry in &mut entries {
        if let Kind::Submenu(children) = &mut entry.kind {
            *children = paginate(std::mem::take(children));
        }
    }
    if entries.len() > SLICES {
        let rest = entries.split_off(SLICES - 1);
        entries.push(submenu("dots-filled", aurora_i18n::tr!("ui-pie-more"), rest));
    }
    entries
}

fn take_action(entries: &mut Vec<Entry>, predicate: impl Fn(&CtxAction) -> bool) -> Entry {
    let index = entries.iter().position(|e| matches!(&e.kind, Kind::Action(a) if predicate(a)));
    index.map_or_else(blank, |i| entries.remove(i))
}

fn take_icon(entries: &mut Vec<Entry>, icon: &str) -> Entry {
    entries
        .iter()
        .position(|e| e.icon == icon)
        .map_or_else(blank, |i| entries.remove(i))
}

fn seated(entries: &mut Vec<Entry>) -> Entry {
    let standing = take_action(entries, |a| matches!(a, CtxAction::StandUp));
    let sitting = take_action(entries, |a| matches!(a, CtxAction::Sit { .. } | CtxAction::SitGround));
    if standing.enabled {
        standing
    } else {
        sitting
    }
}

/// Main directions follow Firestorm. Only Aurora's additional/unfinished
/// entries move into "More"; list-menu order remains unchanged.
fn layout(target: &Target, entries: &[Entry]) -> Vec<Entry> {
    let mut rest = entries.to_vec();
    let mut root = match target {
        Target::Object { .. } => {
            let open = take_action(&mut rest, |a| matches!(a, CtxAction::Open(_)));
            let build = take_action(&mut rest, |a| matches!(a, CtxAction::Build));
            let touch = take_action(&mut rest, |a| matches!(a, CtxAction::Touch(_)));
            let sit = seated(&mut rest);
            let buy = take_action(&mut rest, |a| matches!(a, CtxAction::Buy(_)));
            let take = take_action(&mut rest, |a| matches!(a, CtxAction::Take(_)));
            let copy = take_action(&mut rest, |a| matches!(a, CtxAction::TakeCopy(_)));
            // several objects: "Take multiple" in place of Take / Take copy
            // (menu_pie_object.xml Take ▸, VisibleTakeMultiple)
            let takes = match take.kind {
                Kind::Action(_) => vec![take, copy],
                _ => match take_icon(&mut rest, "hand-grab").kind {
                    Kind::Submenu(entries) => entries,
                    _ => Vec::new(),
                },
            };
            let buy_or_take = if buy.enabled {
                rest.extend(takes);
                buy
            } else {
                rest.push(buy);
                submenu("hand-grab", aurora_i18n::tr!("ui-context-take"), takes)
            };
            let pay = take_action(&mut rest, |a| matches!(a, CtxAction::Pay(_)));
            let edit = take_action(&mut rest, |a| matches!(a, CtxAction::Edit(_)));
            vec![open, build, touch, sit, buy_or_take, pay, blank(), edit]
        }
        Target::Ground => {
            let about = take_action(&mut rest, |a| matches!(a, CtxAction::AboutLand(_)));
            let build = take_action(&mut rest, |a| matches!(a, CtxAction::Build));
            // Aurora has no autopilot yet: its existing Zoom action occupies
            // Go Here's direction rather than introducing an unavailable walk.
            let zoom = take_action(&mut rest, |a| matches!(a, CtxAction::Zoom(..)));
            let stand = take_action(&mut rest, |a| matches!(a, CtxAction::StandUp));
            let sit = take_icon(&mut rest, "armchair");
            let particles = take_icon(&mut rest, "sparkles-filled");
            let pass = take_icon(&mut rest, "ticket-filled");
            let edit = take_action(&mut rest, |a| matches!(a, CtxAction::EditTerrain));
            let buy = take_icon(&mut rest, "shopping-cart-filled");
            // Keep the extra terrain toggle and particle action reachable
            // without moving the other directions of menu_pie_land.xml.
            rest.push(particles);
            let more = submenu("dots-filled", aurora_i18n::tr!("ui-pie-more"), rest);
            return vec![about, build, zoom, if stand.enabled { stand } else { sit }, more, pass, edit, buy];
        }
        Target::Avatar { own: true, .. } => {
            let community = take_icon(&mut rest, "users-group");
            let mut community = match community.kind {
                Kind::Submenu(e) => e,
                _ => Vec::new(),
            };
            let profile = take_action(&mut community, |a| matches!(a, CtxAction::Profile(_)));
            let groups = take_action(&mut community, |a| matches!(a, CtxAction::OpenPeople(2)));
            rest.extend(community);
            let remove = take_icon(&mut rest, "shirt-filled");
            let sit = seated(&mut rest);
            let scripts = take_icon(&mut rest, "code");
            let appearance = take_icon(&mut rest, "man-filled");
            let textures = take_icon(&mut rest, "photo-filled");
            vec![profile, groups, remove, sit, scripts, blank(), appearance, textures]
        }
        Target::Attachment { .. } => {
            let profile = take_icon(&mut rest, "info-circle-filled");
            let drop = take_action(&mut rest, |a| matches!(a, CtxAction::Drop(_)));
            let sit = seated(&mut rest);
            let detach = take_action(&mut rest, |a| matches!(a, CtxAction::Detach(_)));
            let inventory = take_action(&mut rest, |a| matches!(a, CtxAction::AttachmentInventory(_)));
            let appearance = take_icon(&mut rest, "man-filled");
            let edit = take_action(&mut rest, |a| matches!(a, CtxAction::EditAttachment(_)));
            vec![profile, drop, blank(), sit, detach, inventory, appearance, edit]
        }
        Target::Avatar { .. } => {
            let profile = take_action(&mut rest, |a| matches!(a, CtxAction::Profile(_)));
            let mut mute = take_icon(&mut rest, "alert-triangle-filled");
            let report = match &mut mute.kind {
                Kind::Submenu(e) => take_icon(e, "flag-filled"),
                _ => blank(),
            };
            let zoom = take_action(&mut rest, |a| matches!(a, CtxAction::Zoom(..)));
            let friend = take_action(&mut rest, |a| matches!(a, CtxAction::AddFriend(_)));
            let invite = take_icon(&mut rest, "users-plus");
            let contacts = take_icon(&mut rest, "users-group");
            let add = submenu("user-plus", aurora_i18n::tr!("common-add"), vec![friend, contacts, invite]);
            let pay = take_icon(&mut rest, "coin-filled");
            let im = take_action(&mut rest, |a| matches!(a, CtxAction::Im(_)));
            vec![profile, mute, zoom, report, add, pay, blank(), im]
        }
    };
    let more_index = match target {
        Target::Attachment { .. } => 2,
        Target::Avatar { own: true, .. } => 5,
        _ => 6,
    };
    root[more_index] = submenu("dots-filled", aurora_i18n::tr!("ui-pie-more"), rest);
    paginate(root)
}

fn geometry(screen: Rect, pos: Pos2) -> (Pos2, f32) {
    let radius = OUTER.min((screen.width().min(screen.height()) * 0.5 - 8.0).max(0.0));
    let center = egui::pos2(
        pos.x.clamp(screen.left() + radius, screen.right() - radius),
        pos.y.clamp(screen.top() + radius, screen.bottom() - radius),
    );
    (center, radius)
}

fn sector(offset: Vec2, inner: f32, outer: f32) -> Option<usize> {
    let distance = offset.length();
    if distance <= inner || distance > outer {
        return None;
    }
    let angle = (-offset.y).atan2(offset.x);
    Some(((angle + PI / 8.0).rem_euclid(TAU) / FRAC_PI_4) as usize % SLICES)
}

fn ring_segment(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, index: usize, fill: Color32) {
    let mut mesh = egui::Mesh::default();
    let start = index as f32 * FRAC_PI_4 - PI / 8.0;
    for step in 0..=12 {
        let angle = start + FRAC_PI_4 * step as f32 / 12.0;
        let direction = egui::vec2(angle.cos(), -angle.sin());
        mesh.colored_vertex(center + direction * inner, fill);
        mesh.colored_vertex(center + direction * outer, fill);
        if step < 12 {
            let i = step * 2;
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i + 1, i + 3, i + 2);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_icon(painter: &egui::Painter, icon: &str, center: Pos2, size: f32, color: Color32) {
    if let Some(icon) = crate::ui::icons::global(icon) {
        painter.image(
            icon.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

/// The voice row of an avatar in our voice channel, in a panel under the
/// disc (above it when the screen ends there), the pie having no rows;
/// returns the panel's rectangle, where clicks leave the pie open.
fn voice_panel(ctx: &egui::Context, p: &Palette, center: Pos2, radius: f32, id: uuid::Uuid, voice: crate::ui::menu::VoiceVolume) -> Rect {
    const WIDTH: f32 = 250.0;
    const HEIGHT: f32 = 38.0;
    let screen = ctx.content_rect();
    let below = center.y + radius + 8.0;
    let y = if below + HEIGHT <= screen.bottom() {
        below
    } else {
        center.y - radius - 8.0 - HEIGHT
    };
    let x = (center.x - WIDTH * 0.5).clamp(screen.left(), (screen.right() - WIDTH).max(screen.left()));
    // Tooltip sits above the pie's Foreground modal surface so the slider
    // and mute stay clickable (a same-order Area lost the hit test to the
    // full-screen Sense::click_and_drag under it).
    egui::Area::new(egui::Id::unique("world_pie_voice"))
        .order(egui::Order::Tooltip)
        .fixed_pos(egui::pos2(x, y))
        .movable(false)
        .fade_in(false)
        .show(ctx, |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_width(WIDTH - 10.0);
                super::voice_control(ui, p, id, voice);
            });
        })
        .response
        .rect
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    menu: &mut ContextMenu,
    entries: &[Entry],
    voice: Option<(uuid::Uuid, crate::ui::menu::VoiceVolume)>,
    open: &mut bool,
    customized: bool,
) -> Option<usize> {
    let root = if customized {
        paginate(entries.to_vec())
    } else {
        layout(&menu.target, entries)
    };
    let mut current = root.as_slice();
    let mut valid_depth = 0;
    for &index in &menu.pie_path {
        let Some(Entry {
            kind: Kind::Submenu(children),
            ..
        }) = current.get(index)
        else {
            break;
        };
        current = children;
        valid_depth += 1;
    }
    menu.pie_path.truncate(valid_depth);
    let screen = ctx.content_rect();
    let (center, radius) = geometry(screen, menu.pos);
    let scale = radius / OUTER;
    let inner = INNER * scale;
    let offset = ctx.input(|i| i.pointer.interact_pos()).map(|pos| pos - center);
    let hovered = offset.and_then(|offset| sector(offset, inner, radius));
    let hot = hovered.filter(|&index| current.get(index).is_some_and(|e| e.enabled));
    let mut primary_released = false;
    let mut secondary_released = false;
    let panel = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha);
    egui::Area::new(egui::Id::unique("world_pie_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .movable(false)
        .show(ctx, |ui| {
            // A modal input surface prevents clicks or drags reaching the world
            // or another floater, including the menu's transparent corners.
            let response = ui.allocate_rect(screen, Sense::click_and_drag());
            ui.ctx().move_to_top(ui.layer_id());
            let painter = ui.painter();
            if p.panel_alpha < 255 {
                // a square with fully rounded corners is the disc
                let disc = egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
                painter.add(aurora_render::glass_panel(disc, radius, Color32::TRANSPARENT));
            }
            for index in 0..SLICES {
                ring_segment(
                    painter,
                    center,
                    inner,
                    radius,
                    index,
                    if hot == Some(index) { p.violet.gamma_multiply(0.7) } else { panel },
                );
                let angle = index as f32 * FRAC_PI_4 - PI / 8.0;
                let direction = egui::vec2(angle.cos(), -angle.sin());
                painter.line_segment(
                    [center + direction * inner, center + direction * radius],
                    Stroke::new(1.0, p.raised),
                );
                let Some(entry) = current.get(index) else {
                    continue;
                };
                let direction = egui::vec2((index as f32 * FRAC_PI_4).cos(), -(index as f32 * FRAC_PI_4).sin());
                let anchor = center + direction * (radius * 0.65);
                let ink = if entry.enabled { p.ink } else { p.muted_dim };
                paint_icon(
                    painter,
                    entry.icon,
                    anchor - egui::vec2(0.0, 18.0 * scale),
                    18.0 * scale,
                    if entry.enabled { p.violet_light } else { p.muted_dim },
                );
                if entry.checked == Some(true) {
                    paint_icon(
                        painter,
                        "check",
                        anchor + egui::vec2(15.0 * scale, -18.0 * scale),
                        11.0 * scale,
                        p.violet_light,
                    );
                }
                let label = match &entry.kind {
                    Kind::Submenu(_) => format!("{} ›", entry.label),
                    _ => entry.label.clone(),
                };
                let mut job = egui::text::LayoutJob::simple(label, FontId::proportional(12.0 * scale), ink, 104.0 * scale);
                job.halign = egui::Align::Center;
                job.wrap.max_rows = 3;
                let galley = painter.layout_job(job);
                // Center-aligned galleys already place glyphs around x=0.
                painter.galley(anchor - egui::vec2(0.0, 2.0 * scale), galley, ink);
            }
            painter.circle_stroke(center, radius, Stroke::new(1.0, p.raised));
            let center_hot = offset.is_some_and(|o| o.length() <= inner);
            painter.circle_filled(center, inner, if center_hot { p.raised } else { p.field });
            painter.circle_stroke(center, inner, Stroke::new(1.0, p.raised));
            paint_icon(
                painter,
                if menu.pie_path.is_empty() { "x" } else { "arrow-left" },
                center - egui::vec2(0.0, 7.0 * scale),
                16.0 * scale,
                p.muted,
            );
            painter.text(
                center + egui::vec2(0.0, 12.0 * scale),
                Align2::CENTER_CENTER,
                if menu.pie_path.is_empty() {
                    aurora_i18n::tr!("common-close")
                } else {
                    aurora_i18n::tr!("ui-pie-back")
                },
                FontId::proportional(10.0 * scale),
                p.muted,
            );
            // Only releases that landed on this modal surface. A global
            // button_released also fires when the voice / volume panels
            // (Tooltip order above us) handle the click, and would close
            // the pie or steal the drag. Read before on_hover_text, which
            // consumes the Response.
            let on_surface = response.contains_pointer();
            if let Some(entry) = hovered.and_then(|i| current.get(i)) {
                let tooltip = if matches!(entry.kind, Kind::Unavailable) && !entry.label.is_empty() {
                    format!("{}\n{}", entry.label, super::menu::not_yet())
                } else {
                    entry.label.clone()
                };
                response.on_hover_text(tooltip);
            }
            primary_released = on_surface && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            secondary_released = on_surface && ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary));
        });
    // at the first level only, with the avatar's own entries
    if let Some((id, voice)) = voice.filter(|_| menu.pie_path.is_empty()) {
        let panel = voice_panel(ctx, p, center, radius, id, voice);
        if ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| panel.contains(pos)) {
            primary_released = false;
            secondary_released = false;
        }
    }
    let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let back = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace));
    if escape {
        *open = false;
    } else if back {
        menu.pie_path.pop();
    } else if (primary_released || secondary_released) && menu.pie_input_frame.is_some_and(|frame| frame != ctx.cumulative_frame_nr()) {
        if secondary_released {
            if menu.pie_path.pop().is_none() {
                *open = false;
            }
        } else if offset.is_none_or(|o| o.length() > radius) {
            *open = false;
        } else if offset.is_some_and(|o| o.length() <= inner) {
            if menu.pie_path.pop().is_none() {
                *open = false;
            }
        } else if let Some(index) = hot {
            crate::ui::sound_cues::request(ctx, crate::ui_sound::UiSound::ClickRelease);
            match &current[index].kind {
                Kind::Action(action) => {
                    super::request(ctx, action.clone());
                    // Entering arrangement editing keeps the temporary build
                    // selection and its live permissions until editing ends.
                    if !current[index].keeps_open() {
                        *open = false;
                    }
                }
                Kind::Submenu(_) => {
                    menu.pie_path.push(index);
                    crate::ui::sound_cues::request(ctx, crate::ui_sound::UiSound::PieMenuAppear);
                }
                _ => {}
            }
        }
    }
    menu.pie_input_frame = Some(ctx.cumulative_frame_nr());
    hot
}

#[cfg(test)]
mod tests {
    use super::super::Facts;
    use super::*;
    use glam::Vec3;

    fn frame(ctx: &egui::Context, menu: &mut Option<ContextMenu>, facts: &Facts, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    menu,
                    facts,
                    true,
                    &mut Default::default(),
                    &mut Default::default(),
                );
                if ui.ctx().current_pass_index() == 0 {
                    ui.ctx().request_discard("exercise menu input across repeated egui passes");
                }
            },
        );
        // These input tests intentionally do not upload textures to a renderer.
        output.textures_delta.clear();
    }

    fn click(ctx: &egui::Context, menu: &mut Option<ContextMenu>, facts: &Facts, pos: Pos2, button: egui::PointerButton) {
        for pressed in [true, false] {
            frame(
                ctx,
                menu,
                facts,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    fn open(target: Target) -> Option<ContextMenu> {
        Some(ContextMenu {
            pos: egui::pos2(400.0, 400.0),
            point: Vec3::ZERO,
            target,
            pie_path: Vec::new(),
            pie_input_frame: None,
        })
    }

    #[test]
    fn radial_checkable_choices_keep_the_current_submenu_open() {
        fn locate(rows: &[Entry], predicate: &dyn Fn(&CtxAction) -> bool) -> Option<Vec<usize>> {
            for (index, row) in rows.iter().enumerate() {
                match &row.kind {
                    Kind::Action(action) if predicate(action) => return Some(vec![index]),
                    Kind::Submenu(children) => {
                        if let Some(mut path) = locate(children, predicate) {
                            path.insert(0, index);
                            return Some(path);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let ctx = egui::Context::default();
        let mut facts = Facts::default();
        let target = Target::Avatar {
            id: uuid::Uuid::from_u128(7),
            own: false,
            attachment: None,
        };
        let mut menu = open(target.clone());
        for mode in [2, 1, 0, 3, 3] {
            let entries = super::super::entries::world_menu(&target, &facts, Vec3::ZERO);
            let root = layout(&target, &entries);
            let path = locate(&root, &|action| {
                if mode == 3 {
                    matches!(action, CtxAction::ToggleOwnerParticleFilter(_))
                } else {
                    matches!(action, CtxAction::SetRender(_, value) if *value == mode)
                }
            })
            .expect("checkable slice");
            let (index, parent) = path.split_last().expect("slice path");
            menu.as_mut().expect("open").pie_path = parent.to_vec();
            for _ in 0..2 {
                frame(&ctx, &mut menu, &facts, Vec::new());
            }
            let angle = *index as f32 * FRAC_PI_4;
            click(
                &ctx,
                &mut menu,
                &facts,
                egui::pos2(400.0 + angle.cos() * 100.0, 400.0 - angle.sin() * 100.0),
                egui::PointerButton::Primary,
            );
            assert_eq!(menu.as_ref().expect("checkable keeps menu open").pie_path, parent);
            let actions = super::super::take_requests(&ctx);
            if mode == 3 {
                assert!(matches!(&actions[..], [CtxAction::ToggleOwnerParticleFilter(_)]), "{actions:?}");
                facts.particles_blocked = !facts.particles_blocked;
            } else {
                assert!(
                    matches!(&actions[..], [CtxAction::SetRender(_, value)] if *value == mode),
                    "{actions:?}"
                );
                facts.render = mode;
            }
        }
        click(&ctx, &mut menu, &facts, egui::pos2(900.0, 800.0), egui::PointerButton::Primary);
        assert!(menu.is_none());
    }

    #[test]
    fn disabled_slice_stays_open_and_left_click_commits_once() {
        let ctx = egui::Context::default();
        let facts = Facts {
            flying: true,
            ..Default::default()
        };
        let mut menu = open(Target::Ground);
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(330.0, 330.0), egui::PointerButton::Primary);
        assert!(menu.is_some());
        assert!(super::super::take_requests(&ctx).is_empty());
        click(&ctx, &mut menu, &facts, egui::pos2(500.0, 400.0), egui::PointerButton::Primary);
        assert!(menu.is_none());
        assert!(matches!(super::super::take_requests(&ctx).as_slice(), [CtxAction::AboutLand(_)]));
    }

    #[test]
    fn land_sit_slice_dispatches_the_clicked_ground_point() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let mut menu = open(Target::Ground);
        let point = Vec3::new(20.0, 30.0, 4.0);
        menu.as_mut().expect("menu").point = point;
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(330.0, 330.0), egui::PointerButton::Primary);
        assert!(matches!(super::super::take_requests(&ctx).as_slice(), [CtxAction::SitAt(p)] if *p == point));
        assert!(menu.is_none());
    }

    #[test]
    fn opening_the_radial_editor_keeps_the_menu_selection_alive() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let mut menu = open(Target::Ground);
        menu.as_mut().expect("menu").pie_path = vec![4];
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(470.0, 330.0), egui::PointerButton::Primary);
        assert!(matches!(
            super::super::take_requests(&ctx).as_slice(),
            [CtxAction::CustomizeMenu(_)]
        ));
        assert!(menu.is_some(), "the app must retain its temporary object selection");
    }

    #[test]
    fn right_click_returns_one_level_without_dispatching_the_hovered_action() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let mut menu = open(Target::Object {
            key: crate::world::objects::ObjKey { region: 1, local_id: 2 },
            full_id: uuid::Uuid::nil(),
            offset: Vec3::ZERO,
        });
        // More contains the Object submenu at direction 1.
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(400.0, 500.0), egui::PointerButton::Primary);
        assert_eq!(menu.as_ref().expect("open More").pie_path, vec![6]);
        click(&ctx, &mut menu, &facts, egui::pos2(470.0, 330.0), egui::PointerButton::Primary);
        assert_eq!(menu.as_ref().expect("open Object").pie_path, vec![6, 1]);
        // Back works over a sector and outside the circle, once per click.
        click(&ctx, &mut menu, &facts, egui::pos2(330.0, 470.0), egui::PointerButton::Secondary);
        assert_eq!(menu.as_ref().expect("back to More").pie_path, vec![6]);
        click(&ctx, &mut menu, &facts, egui::pos2(900.0, 400.0), egui::PointerButton::Secondary);
        assert!(menu.as_ref().expect("back to root").pie_path.is_empty());
        click(&ctx, &mut menu, &facts, egui::pos2(500.0, 400.0), egui::PointerButton::Secondary);
        assert!(menu.is_none());
        assert!(super::super::take_requests(&ctx).is_empty());
    }

    #[test]
    fn submenu_center_back_and_outside_cancel() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let key = crate::world::objects::ObjKey { region: 1, local_id: 2 };
        let mut menu = open(Target::Object {
            key,
            full_id: uuid::Uuid::nil(),
            offset: Vec3::ZERO,
        });
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(400.0, 500.0), egui::PointerButton::Primary);
        assert_eq!(menu.as_ref().expect("open More submenu").pie_path, vec![6]);
        click(&ctx, &mut menu, &facts, egui::pos2(400.0, 400.0), egui::PointerButton::Primary);
        assert!(menu.as_ref().expect("open root").pie_path.is_empty());
        click(&ctx, &mut menu, &facts, egui::pos2(900.0, 400.0), egui::PointerButton::Primary);
        assert!(menu.is_none());
        assert!(super::super::take_requests(&ctx).is_empty());
    }

    #[test]
    fn the_voice_panel_under_the_pie_changes_the_volume_without_closing_it() {
        let ctx = egui::Context::default();
        let tess = uuid::Uuid::from_u128(2);
        let facts = Facts {
            voice_volume: Some(crate::ui::menu::VoiceVolume {
                volume: 0.5,
                muted: false,
                can_mute: true,
                moderated: None,
            }),
            ..Facts::default()
        };
        let mut menu = open(Target::Avatar {
            id: tess,
            own: false,
            attachment: None,
        });
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        // the disc ends at y = 580; the slider starts 36 points into the panel
        click(&ctx, &mut menu, &facts, egui::pos2(320.0, 607.0), egui::PointerButton::Primary);
        assert!(menu.is_some(), "a click on the voice panel leaves the pie open");
        let requests = super::super::take_requests(&ctx);
        assert!(
            !requests.is_empty()
                && requests
                    .iter()
                    .all(|r| matches!(r, CtxAction::SetVoiceVolume(id, v) if *id == tess && *v < 0.5)),
            "{requests:?}"
        );
        // out of voice: no panel, a click there closes the pie
        let mut menu = open(Target::Avatar {
            id: tess,
            own: false,
            attachment: None,
        });
        let facts = Facts::default();
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(320.0, 607.0), egui::PointerButton::Primary);
        assert!(menu.is_none());
    }

    #[test]
    fn submenu_action_is_dispatched_once_across_repeated_passes() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let mut menu = open(Target::Avatar {
            id: uuid::Uuid::nil(),
            own: true,
            attachment: None,
        });
        frame(&ctx, &mut menu, &facts, Vec::new());
        frame(&ctx, &mut menu, &facts, Vec::new());
        click(&ctx, &mut menu, &facts, egui::pos2(400.0, 500.0), egui::PointerButton::Primary);
        assert_eq!(menu.as_ref().expect("open Appearance submenu").pie_path, vec![6]);
        click(&ctx, &mut menu, &facts, egui::pos2(500.0, 400.0), egui::PointerButton::Primary);
        assert!(menu.is_none());
        assert!(matches!(
            super::super::take_requests(&ctx).as_slice(),
            [CtxAction::OpenAppearance { tab: 2, editing: false }]
        ));
    }

    #[test]
    fn initial_release_near_an_edge_does_not_commit_and_escape_closes() {
        let ctx = egui::Context::default();
        let facts = Facts::default();
        let mut menu = open(Target::Ground);
        menu.as_mut().expect("new menu").pos = egui::pos2(10.0, 10.0);
        frame(
            &ctx,
            &mut menu,
            &facts,
            vec![
                egui::Event::PointerMoved(egui::pos2(10.0, 10.0)),
                egui::Event::PointerButton {
                    pos: egui::pos2(10.0, 10.0),
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(menu.is_some());
        assert!(super::super::take_requests(&ctx).is_empty());
        frame(
            &ctx,
            &mut menu,
            &facts,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(menu.is_none());
    }

    #[test]
    fn directions_dead_zone_and_outer_edge() {
        for index in 0..SLICES {
            let angle = index as f32 * FRAC_PI_4;
            assert_eq!(sector(egui::vec2(angle.cos(), -angle.sin()) * 100.0, INNER, OUTER), Some(index));
        }
        assert_eq!(sector(Vec2::ZERO, INNER, OUTER), None);
        assert_eq!(sector(egui::vec2(INNER, 0.0), INNER, OUTER), None);
        assert_eq!(sector(egui::vec2(OUTER, 0.0), INNER, OUTER), Some(0));
        assert_eq!(sector(egui::vec2(OUTER + 1.0, 0.0), INNER, OUTER), None);
    }

    #[test]
    fn pie_fits_screen_edges_and_small_windows() {
        for size in [egui::vec2(1600.0, 900.0), egui::vec2(320.0, 240.0)] {
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            for position in [screen.min, screen.max, screen.center()] {
                let (center, radius) = geometry(screen, position);
                assert!(screen.contains_rect(Rect::from_center_size(center, Vec2::splat(radius * 2.0))));
            }
        }
    }

    fn actions(entries: &[Entry], result: &mut Vec<String>) {
        for entry in entries {
            match &entry.kind {
                Kind::Action(a) => result.push(format!("{a:?}:{}:{:?}", entry.enabled, entry.checked)),
                Kind::Submenu(children) => {
                    assert!(children.len() <= SLICES);
                    actions(children, result);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn both_styles_keep_actions_permissions_and_checked_states() {
        let key = crate::world::objects::ObjKey { region: 1, local_id: 2 };
        let id = uuid::Uuid::nil();
        for facts in [
            Facts::default(),
            Facts {
                property_lines: true,
                seated: true,
                object: super::super::ObjectFacts {
                    local_hud_scale: Some(1.6),
                    saved_hud_scale: true,
                    buy: true,
                    stand: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Facts {
                object: super::super::ObjectFacts {
                    local_hud_scale: Some(1.0),
                    saved_hud_scale: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        ] {
            for target in [
                Target::Ground,
                Target::Object {
                    key,
                    full_id: id,
                    offset: Vec3::ZERO,
                },
                Target::Attachment { key },
                Target::Avatar {
                    id,
                    own: true,
                    attachment: None,
                },
                Target::Avatar {
                    id,
                    own: false,
                    attachment: Some(id),
                },
            ] {
                let entries = super::super::entries::world_menu(&target, &facts, Vec3::ZERO);
                let radial = layout(&target, &entries);
                assert_eq!(radial.len(), SLICES);
                // Sit/stand share a slice; the unavailable alternative is intentionally hidden.
                let mut classic = Vec::new();
                fn list_actions(entries: &[Entry], result: &mut Vec<String>) {
                    for e in entries {
                        match &e.kind {
                            Kind::Action(a)
                                if !matches!(
                                    a,
                                    CtxAction::Sit { .. } | CtxAction::SitAt(_) | CtxAction::SitGround | CtxAction::StandUp
                                ) =>
                            {
                                result.push(format!("{a:?}:{}:{:?}", e.enabled, e.checked))
                            }
                            Kind::Submenu(c) => list_actions(c, result),
                            _ => {}
                        }
                    }
                }
                list_actions(&entries, &mut classic);
                let mut pie = Vec::new();
                actions(&radial, &mut pie);
                pie.retain(|a| {
                    !a.starts_with("Sit ") && !a.starts_with("SitAt(") && !a.starts_with("SitGround:") && !a.starts_with("StandUp:")
                });
                classic.sort();
                pie.sort();
                assert_eq!(pie, classic, "{target:?}");
            }
        }
    }
}
