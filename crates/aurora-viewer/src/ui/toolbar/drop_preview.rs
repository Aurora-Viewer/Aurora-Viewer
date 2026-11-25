//! Proximity-aware extension to LLToolBarView::draw / handleDragTool
//! (indra/newview/lltoolbarview.cpp): one shared preview and atomic drop decision
//! for all four edges. Contact joins a group; a nearby detached drop creates one.

use super::{Bar, Drag, Edit, ToolbarSettings, ToolbarUi, structure};
use crate::theme::Palette;
use crate::ui::{
    bars,
    skin::{Layout, ToolButton},
};
use egui::{Id, LayerId, Pos2, Rect, Vec2};

const NEAR: f32 = 48.0;
const CONTACT: f32 = 4.0;

#[derive(Clone)]
pub(super) struct Target {
    pub bar: Bar,
    pub area: Rect,
    pub layer: LayerId,
    pub buttons: Vec<ToolButton>,
    pub rects: Vec<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Join,
    Group,
    Empty,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Preview {
    pub bar: Bar,
    pub rank: usize,
    pub kind: Kind,
    pub rect: Rect,
}

struct Group {
    start: usize,
    end: usize,
    rect: Rect,
}

#[derive(Clone)]
struct Shadow {
    base: std::sync::Arc<ToolbarSettings>,
    settings: std::sync::Arc<ToolbarSettings>,
    bars: [Vec<ToolButton>; 4],
    command: String,
    bar: Bar,
    region: Rect,
    layer: LayerId,
}

fn shadow_id() -> Id {
    Id::unique("toolbar_drop_layout")
}
fn shadow(ctx: &egui::Context) -> Option<std::sync::Arc<Shadow>> {
    ctx.data(|data| data.get_temp::<Option<std::sync::Arc<Shadow>>>(shadow_id()))
        .flatten()
}

pub(super) fn clear(ctx: &egui::Context) {
    if shadow(ctx).is_some() {
        ctx.request_repaint();
    }
    ctx.data_mut(|data| data.insert_temp(shadow_id(), None::<std::sync::Arc<Shadow>>));
}

pub(super) fn configuration(ctx: &egui::Context) -> Option<std::sync::Arc<ToolbarSettings>> {
    shadow(ctx).map(|state| state.settings.clone())
}

pub(super) fn shown_buttons(ctx: &egui::Context, bar: Bar) -> Option<Vec<ToolButton>> {
    shadow(ctx).map(|state| state.bars[bar.index()].clone())
}

fn display_command(command: &str) -> &str {
    if command == structure::SEPARATOR {
        "separator:drop-preview-entry"
    } else if command == structure::DIVIDER {
        "divider:drop-preview-entry"
    } else if command == super::drawers::FLOATING {
        super::drawers::FLOATING_PREVIEW
    } else {
        command
    }
}

pub(super) fn snapped(ctx: &egui::Context, command: &str) -> bool {
    shadow(ctx).is_some_and(|state| state.command == command)
        && ctx
            .read_response(bars::tool_button_id(display_command(command)))
            .is_some_and(|response| response.rect.intersects(ctx.content_rect()) && response.interact_rect.is_positive())
}

pub(super) fn is_item(ctx: &egui::Context, command: &str) -> bool {
    shadow(ctx).is_some_and(|state| display_command(&state.command) == command)
}

fn targets_id() -> Id {
    Id::unique("toolbar_drop_targets")
}
fn preview_id() -> Id {
    Id::unique("toolbar_drop_preview")
}

pub(super) fn prepare(ctx: &egui::Context, settings: &ToolbarSettings) {
    let valid = shadow(ctx).is_some_and(|state| {
        state.base.as_ref() == settings
            && egui::DragAndDrop::payload::<Drag>(ctx).is_some_and(|drag| drag.button.command == state.command)
            && ctx.pointer_interact_pos().is_some_and(|pos| {
                state.region.expand(NEAR).contains(pos)
                    && ctx.content_rect().contains(pos)
                    && ctx
                        .layer_id_at(pos)
                        .is_none_or(|layer| layer == state.layer || layer.order == egui::Order::Background)
            })
    });
    if !valid {
        clear(ctx);
    }
    ctx.data_mut(|data| {
        data.insert_temp(targets_id(), Vec::<Target>::new());
        data.insert_temp(preview_id(), None::<Preview>);
        data.insert_temp(Id::unique("toolbar_protected_drops"), Vec::<(Rect, LayerId)>::new());
    });
}

pub(super) fn protect(ctx: &egui::Context, rect: Rect, layer: LayerId) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<(Rect, LayerId)>>(Id::unique("toolbar_protected_drops"))
            .push((rect, layer))
    });
}

pub(super) fn removal(ctx: &egui::Context, drag: &Drag, settings: &ToolbarSettings) -> bool {
    drag.from.is_some() && free_spot(ctx, settings).is_some()
}

/// The catalogue's "Floating block" let go over the world, out of every bar,
/// window and block: where the new block goes.
pub(super) fn floating_spot(ctx: &egui::Context, drag: &Drag, settings: &ToolbarSettings) -> Option<egui::Pos2> {
    if drag.from.is_some() || drag.button.command != super::drawers::FLOATING {
        return None;
    }
    free_spot(ctx, settings).filter(|pos| ctx.layer_id_at(*pos).is_none_or(|layer| layer.order == egui::Order::Background))
}

/// The pointer, when a drag let go now would land on no bar, drawer or
/// floating block.
fn free_spot(ctx: &egui::Context, settings: &ToolbarSettings) -> Option<egui::Pos2> {
    if settings.locked {
        return None;
    }
    let pos = ctx.pointer_interact_pos().filter(|pos| ctx.content_rect().contains(*pos))?;
    free_at(ctx, pos).then_some(pos)
}

fn free_at(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    if ctx
        .data(|data| data.get_temp::<bool>(Id::unique("toolbar_drop_handled")))
        .unwrap_or(false)
        || ctx.data(|data| data.get_temp::<Option<Preview>>(preview_id())).flatten().is_some()
    {
        return false;
    }
    if ctx
        .data(|data| data.get_temp::<Vec<Target>>(targets_id()))
        .unwrap_or_default()
        .iter()
        .any(|target| target.area.expand(NEAR).contains(pos))
    {
        return false;
    }
    if [crate::ui::chat::bar_input_id(), Id::unique("chat_island_width")]
        .iter()
        .any(|id| ctx.read_response(*id).is_some_and(|response| response.contains_pointer()))
    {
        return false;
    }
    let layer = ctx.layer_id_at(pos);
    !ctx.data(|data| data.get_temp::<Vec<(Rect, LayerId)>>(Id::unique("toolbar_protected_drops")))
        .unwrap_or_default()
        .iter()
        .any(|(rect, target)| rect.contains(pos) && layer.is_none_or(|layer| layer == *target || layer.order == egui::Order::Background))
}

pub(super) fn collect(response: &egui::Response, bar: Bar, buttons: &[ToolButton], rects: &[Rect], locked: bool) {
    if locked || !ToolbarUi::dragging(&response.ctx) {
        return;
    }
    let mut target = Target {
        bar,
        area: response.rect,
        layer: response.layer_id,
        buttons: buttons.to_vec(),
        rects: rects.to_vec(),
    };
    if let Some(state) = shadow(&response.ctx)
        && state.bar == bar
    {
        target.area = target.area.union(state.region);
    }
    response
        .ctx
        .data_mut(|data| data.get_temp_mut_or_default::<Vec<Target>>(targets_id()).push(target));
}

#[cfg(test)]
pub(super) fn current(ctx: &egui::Context) -> Option<Preview> {
    ctx.data(|data| data.get_temp::<Option<Preview>>(preview_id())).flatten()
}

fn groups(target: &Target, commands: &[&str]) -> Vec<Group> {
    let mut result = Vec::new();
    let mut current: Option<Group> = None;
    for (index, (button, rect)) in target.buttons.iter().zip(&target.rects).enumerate() {
        if structure::is_separator(&button.command) {
            if let Some(group) = current.take() {
                result.push(group);
            }
        } else if !commands.contains(&button.command.as_str()) && rect.is_positive() {
            if let Some(group) = &mut current {
                group.end = index + 1;
                group.rect = group.rect.union(*rect);
            } else {
                current = Some(Group {
                    start: index,
                    end: index + 1,
                    rect: *rect,
                });
            }
        }
    }
    if let Some(group) = current {
        result.push(group);
    }
    result
}

/// Score actual controls rather than a full-height/width editing destination.
/// A projected button touching a group still joins it, even outside its padding.
#[cfg(test)]
pub(super) fn plan(target: &Target, command: &str, pos: Pos2, size: Vec2, screen: Rect) -> Option<(f32, Preview)> {
    plan_items(target, &[command], pos, size, screen, false)
}

fn plan_items(target: &Target, commands: &[&str], pos: Pos2, size: Vec2, screen: Rect, whole: bool) -> Option<(f32, Preview)> {
    if !screen.contains(pos) || !target.area.is_positive() || !target.area.expand(NEAR).contains(pos) {
        return None;
    }
    // The dock's existing lower bound excludes chat and conversation thumbnails.
    if target.bar == Bar::Bottom && pos.x < target.area.left() {
        return None;
    }
    let footprint = Rect::from_center_size(pos, size);
    let groups = groups(target, commands);
    let nearest = groups
        .iter()
        .min_by(|a, b| a.rect.distance_to_pos(pos).total_cmp(&b.rect.distance_to_pos(pos)));
    let touching = groups
        .iter()
        .filter(|group| group.rect.expand(CONTACT).intersects(footprint))
        .min_by(|a, b| a.rect.distance_to_pos(pos).total_cmp(&b.rect.distance_to_pos(pos)));
    let vertical = target.bar.vertical();
    let axis = |point: Pos2| if vertical { point.y } else { point.x };
    let kind = if groups.is_empty() {
        Kind::Empty
    } else if !whole
        && (touching.is_some()
            || commands
                .iter()
                .any(|command| structure::is_separator(command) || structure::is_divider(command)))
    {
        Kind::Join
    } else {
        Kind::Group
    };
    let rank = if kind == Kind::Group {
        nearest.map_or(0, |group| {
            if axis(pos) < axis(group.rect.center()) {
                group.start
            } else {
                group.end
            }
        })
    } else {
        let rank = super::insertion(&target.rects, pos, vertical);
        touching.map_or(rank, |group| rank.clamp(group.start, group.end))
    };
    let rect = if kind == Kind::Join {
        let r = target.area;
        if vertical {
            let y = target
                .rects
                .get(rank)
                .filter(|r| r.is_positive())
                .map_or_else(
                    || {
                        target
                            .rects
                            .iter()
                            .rev()
                            .find(|r| r.is_positive())
                            .map_or(r.top() + 3.0, |b| b.bottom() + 1.0)
                    },
                    |b| b.top() - 1.0,
                )
                .clamp(r.top() + 1.0, r.bottom() - 1.0);
            Rect::from_min_max(egui::pos2(r.left() + 2.0, y), egui::pos2(r.right() - 2.0, y + 2.0))
        } else {
            let x = target
                .rects
                .get(rank)
                .filter(|r| r.is_positive())
                .map_or_else(
                    || {
                        target
                            .rects
                            .iter()
                            .rev()
                            .find(|r| r.is_positive())
                            .map_or(r.left() + 3.0, |b| b.right() + 2.0)
                    },
                    |b| b.left() - 2.0,
                )
                .clamp(r.left() + 1.0, r.right() - 1.0);
            Rect::from_min_max(egui::pos2(x, r.top() + 2.0), egui::pos2(x + 2.0, r.bottom() - 2.0))
        }
    } else {
        footprint.intersect(screen.shrink(2.0))
    };
    let score = nearest.map_or(target.area.distance_to_pos(pos) + 20.0, |group| group.rect.distance_to_pos(pos));
    Some((
        score,
        Preview {
            bar: target.bar,
            rank,
            kind,
            rect,
        },
    ))
}

fn projected_size(ctx: &egui::Context, target: &Target, button: &ToolButton, settings: &ToolbarSettings, islands: bool) -> Vec2 {
    let square = if target.bar.vertical() { 34.0 } else { 28.0 };
    let style = settings.style(target.bar).resolve(target.bar, islands);
    let width = bars::natural_width(ctx, button, style.mode, square, 16.0, 38.0);
    let height = if structure::is_block(&button.command) {
        bars::chrome::height(ctx, &button.command)
    } else {
        square
    };
    Vec2::new(width.max(16.0), height.max(16.0))
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, layout: &Layout) {
    let Some(drag) = egui::DragAndDrop::payload::<Drag>(ctx) else {
        return;
    };
    if settings.locked
        || toolbar.pending.is_some()
        || ctx
            .data(|data| data.get_temp::<bool>(Id::unique("toolbar_drop_handled")))
            .unwrap_or(false)
    {
        clear(ctx);
        return;
    }
    // The field and resize handle may share the Legacy bar's background layer.
    if [crate::ui::chat::bar_input_id(), Id::unique("chat_island_width")]
        .iter()
        .any(|id| ctx.read_response(*id).is_some_and(|r| r.contains_pointer()))
    {
        clear(ctx);
        return;
    }
    let Some(pos) = ctx.pointer_interact_pos() else { return };
    let layer = ctx.layer_id_at(pos);
    let insertion = super::surface_drop::insertion(ctx);
    let targets = ctx.data(|data| data.get_temp::<Vec<Target>>(targets_id())).unwrap_or_default();
    let preview = targets
        .iter()
        .filter(|target| layer.is_none_or(|layer| layer == target.layer || layer.order == egui::Order::Background))
        .filter_map(|target| {
            let size = projected_size(ctx, target, &drag.button, settings, toolbar.islands);
            let planned = plan_items(
                target,
                &if drag.members.is_empty() {
                    vec![drag.button.command.as_str()]
                } else {
                    drag.members.iter().map(|button| button.command.as_str()).collect()
                },
                pos,
                size,
                ctx.content_rect(),
                !drag.members.is_empty(),
            );
            // The canonical face still means insertion while grouping waits.
            // A shrinking/recentred provisional block cannot turn that gesture
            // into a detached block or change the advertised insertion rank.
            let planned = if let Some(insertion) = insertion
                .filter(|insertion| insertion.bar == target.bar && target.rects.get(insertion.index).is_some_and(|rect| rect.is_positive()))
            {
                Some((
                    -1.0,
                    Preview {
                        bar: target.bar,
                        rank: insertion.index + usize::from(insertion.after),
                        kind: Kind::Join,
                        rect: Rect::from_center_size(pos, size).intersect(ctx.content_rect()),
                    },
                ))
            } else {
                planned
            };
            planned.map(|(distance, preview)| (distance, target, preview))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, target, preview)| (target, preview));
    let Some((target, mut preview)) = preview else {
        clear(ctx);
        return;
    };
    let proposed = ToolbarUi::proposed_drop(settings, layout, preview.bar, &drag, preview.rank, preview.kind == Kind::Group);
    let bars = Bar::ALL.map(|bar| proposed.bar(bar, layout).to_vec());
    // The default bottom chat and dock share one canonical list but use
    // separate glass layers. Retain the destination actually chosen above.
    let region = target.area;
    let layer = target.layer;
    let rendered = shadow(ctx);
    if rendered.as_ref().is_none_or(|state| state.bars != bars) {
        ctx.request_repaint();
    }
    let ready = rendered.as_ref().is_some_and(|state| state.bars == bars);
    ctx.data_mut(|data| {
        data.insert_temp(
            shadow_id(),
            Some(std::sync::Arc::new(Shadow {
                base: std::sync::Arc::new(settings.clone()),
                settings: std::sync::Arc::new(proposed),
                bars,
                command: drag.button.command.clone(),
                bar: preview.bar,
                region,
                layer,
            })),
        )
    });
    if ready && let Some(response) = ctx.read_response(bars::tool_button_id(display_command(&drag.button.command))) {
        preview.rect = drag
            .members
            .iter()
            .filter_map(|button| ctx.read_response(bars::tool_button_id(&button.command)))
            .fold(response.rect, |rect, item| rect.union(item.rect))
            // Block-local widths can outgrow the previous destination bounds.
            // Match the actual widget, clipping only at the viewer viewport.
            .intersect(ctx.content_rect());
        let painter = ctx.layer_painter(response.layer_id);
        let radius = ctx.global_style().visuals.widgets.inactive.corner_radius;
        painter.rect_stroke(
            preview.rect.expand(3.0),
            radius,
            egui::Stroke::new(2.0, p.violet_light),
            egui::StrokeKind::Inside,
        );
        for id in [
            "separator:drop-preview-before",
            "separator:drop-preview-after",
            "separator:drop-preview-entry",
            "divider:drop-preview-entry",
        ] {
            if let Some(gap) = ctx.read_response(bars::tool_button_id(id)) {
                let painter = ctx.layer_painter(gap.layer_id);
                painter.rect_filled(gap.rect, radius, p.violet_light.gamma_multiply(0.14));
                painter.rect_stroke(gap.rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
            }
        }
    }
    ctx.data_mut(|data| data.insert_temp(preview_id(), Some(preview)));
    if ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
        toolbar.pending = Some(if !drag.members.is_empty() {
            Edit::MoveBlock(preview.bar, drag.members.clone(), preview.rank, preview.kind == Kind::Group)
        } else if preview.kind == Kind::Group {
            Edit::InsertGroup(preview.bar, drag.button.clone(), preview.rank)
        } else {
            Edit::Insert(preview.bar, drag.button.clone(), preview.rank)
        });
        ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), true));
        egui::DragAndDrop::clear_payload(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(bar: Bar) -> Target {
        Target {
            bar,
            area: if bar.vertical() {
                Rect::from_min_max(egui::pos2(200.0, 360.0), egui::pos2(238.0, 530.0))
            } else {
                Rect::from_min_max(egui::pos2(160.0, 400.0), egui::pos2(310.0, 436.0))
            },
            layer: LayerId::background(),
            buttons: vec![super::super::tests::button("inventory")],
            rects: vec![Rect::from_min_size(egui::pos2(204.0, 404.0), Vec2::splat(28.0))],
        }
    }

    #[test]
    fn contact_and_straddling_join_while_detached_drops_bracket_the_nearest_group_on_every_edge() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0));
        for bar in Bar::ALL {
            let target = target(bar);
            let axis = if bar.vertical() { Vec2::Y } else { Vec2::X };
            let center = target.rects[0].center();
            for offset in [0.0, 20.0, 31.0] {
                let (_, preview) = plan(&target, "radar", center + axis * offset, Vec2::splat(28.0), screen).expect("contact");
                assert_eq!(preview.kind, Kind::Join, "contact on {bar:?} at {offset}");
            }
            for (offset, rank) in [(-48.0, 0), (48.0, 1)] {
                let (_, preview) = plan(&target, "radar", center + axis * offset, Vec2::splat(28.0), screen).expect("nearby group");
                assert_eq!(preview.kind, Kind::Group, "separate on {bar:?}");
                assert_eq!(preview.rank, rank);
                assert!(!preview.rect.intersects(target.rects[0]));
                assert!(screen.contains_rect(preview.rect));
            }
            let cross = if bar.vertical() { Vec2::X } else { Vec2::Y };
            let (_, preview) = plan(&target, "radar", center + cross * 48.0, Vec2::splat(28.0), screen).expect("nearby parallel drop");
            assert_eq!(preview.kind, Kind::Group);
            assert!(plan(&target, "radar", center + cross * 180.0, Vec2::splat(28.0), screen).is_none());
        }
    }

    #[test]
    fn hidden_controls_and_existing_gaps_keep_their_saved_indices_and_separators_do_not_auto_split() {
        let mut target = target(Bar::Top);
        target.buttons.extend([
            structure::separator("kept"),
            super::super::tests::button("bar_media"),
            super::super::tests::button("people"),
        ]);
        target.rects.extend([
            Rect::from_min_size(egui::pos2(234.0, 404.0), Vec2::new(80.0, 28.0)),
            Rect::NOTHING,
            Rect::from_min_size(egui::pos2(316.0, 404.0), Vec2::splat(28.0)),
        ]);
        target.area.max.x = 350.0;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0));
        let point = egui::pos2(268.0, 418.0);
        let (_, preview) = plan(&target, "radar", point, Vec2::splat(28.0), screen).expect("gap");
        assert_eq!(preview.kind, Kind::Group);
        assert_eq!(preview.rank, 1);
        assert_eq!(
            plan(&target, structure::SEPARATOR, point, Vec2::splat(28.0), screen)
                .expect("separator")
                .1
                .kind,
            Kind::Join
        );
    }
}
