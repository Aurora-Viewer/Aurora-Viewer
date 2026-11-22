//! Blocks taken out of the Aurora build window: a section (General, Object,
//! Features, Materials, Contents) or the Position / Size / Rotation block,
//! each in a window of its own. Alt + drag a section's rail button, or the
//! block itself, outside the build window to take it out; the rail button
//! then shows or hides that window. Drag the window by its title (the
//! Position / Size / Rotation window, which has none, by its background)
//! back onto the build window to put the block back.
//!
//! The detached windows close with the build window and come back with it
//! when editing starts again, each where the user left it: which ones are
//! shown is a build setting, their rects live in the window layout
//! (`ui_layout.ron`) like every floater's. The Legacy window style keeps
//! its own panels and has none of this.

use super::{Env, INSPECTOR_ID, common, compact, layout};
use crate::build::layout::Block;
use crate::build::{BuildSettings, BuildTool, Mods, Tool};
use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use egui::Vec2;

/// The width a block opens at when it is taken out; its height then fits
/// what it shows (`FIT_FRAMES`), except the contents list's, which is fixed
/// so that a long list scrolls instead of making the window very tall.
fn opening_width(block: Block) -> f32 {
    match block {
        Block::General => 386.0,
        Block::Shape => 310.0,
        Block::Materials => 325.0,
        Block::Features | Block::Contents | Block::Transform => 350.0,
    }
}

const CONTENTS_HEIGHT: f32 = 616.0;

/// Frames over which a window just taken out fits its height to its
/// content: one to lay it out at its opening size, the next ones once the
/// new size is in place (a wrapped line may change the height again).
const FIT_FRAMES: u8 = 4;

impl BuildTool {
    /// Whether the section of `tab` is on screen this frame: the build
    /// window's own section, or a detached window that is shown.
    pub fn section_visible(&self, tab: usize) -> bool {
        (self.tab == tab && !self.ui.detached_tabs.contains(&tab)) || self.ui.detached_shown.contains(&tab)
    }
}

pub(super) fn floater_id(block: Block) -> &'static str {
    match block {
        Block::General => "build_detached_general",
        Block::Shape => "build_detached_shape",
        Block::Features => "build_detached_features",
        Block::Materials => "build_detached_materials",
        Block::Contents => "build_detached_contents",
        Block::Transform => "build_detached_transform",
    }
}

fn title(block: Block) -> &'static str {
    block
        .item()
        .map_or_else(|| aurora_i18n::tr!("build-layout-transform"), layout::title)
}

/// Whether detaching is offered at all: the Aurora style, editing.
pub(super) fn available(s: &BuildSettings) -> bool {
    !s.legacy_window
}

pub(super) fn is_detached(s: &BuildSettings, block: Block) -> bool {
    available(s) && s.detached.contains_key(&block)
}

/// Whether the block of a section tab is detached (shown or not).
pub(super) fn tab_detached(s: &BuildSettings, tab: usize) -> bool {
    Block::of_tab(tab).is_some_and(|block| is_detached(s, block))
}

/// The sections this frame takes out of the build window, before it is
/// drawn: the build window then skips them and the material preview,
/// contents and mesh panels follow the detached windows that show them.
pub(super) fn prepare(tool: &mut BuildTool, s: &BuildSettings) {
    tool.ui.detached_tabs.clear();
    tool.ui.detached_shown.clear();
    if !available(s) {
        return;
    }
    for (block, shown) in &s.detached {
        if let Some(tab) = block.tab() {
            tool.ui.detached_tabs.push(tab);
            if *shown && tool.tool == Tool::Edit {
                tool.ui.detached_shown.push(tab);
            }
        }
    }
}

/// Every section of the build window is in a window of its own: the
/// build window keeps its shared controls only.
pub(super) fn sectionless(tool: &BuildTool, s: &BuildSettings) -> bool {
    available(s) && tool.tool == Tool::Edit && tab_detached(s, tool.tab)
}

/// Fit the build window to its shared controls while it has no section,
/// and give it back the height it had once a section comes back.
pub(super) fn fit_window(ctx: &egui::Context, tool: &BuildTool, s: &BuildSettings) {
    let id = egui::Id::unique(INSPECTOR_ID);
    let flag = id.with("build-sectionless");
    let saved = id.with("build-sections-height");
    // (its size where it is drawn: in a window of its own outside the
    // viewer's, not the size of the place it left inside)
    let Some(size) = widgets::floater_size(ctx, INSPECTOR_ID) else {
        return;
    };
    let now = sectionless(tool, s);
    let was = ctx.data_mut(|d| d.get_persisted::<bool>(flag)).unwrap_or(false);
    if now != was {
        ctx.data_mut(|d| d.insert_persisted(flag, now));
        if now {
            ctx.data_mut(|d| d.insert_persisted(saved, size.y));
        } else if let Some(height) = ctx.data_mut(|d| d.get_persisted::<f32>(saved)) {
            widgets::size_floater(ctx, INSPECTOR_ID, egui::vec2(size.x, height));
            return;
        }
    }
    if now && let (Some(body), Some(need)) = (tool.ui.body_height, tool.ui.sectionless_body) {
        let target = (size.y - body + need).round();
        if (size.y - target).abs() > 0.5 {
            widgets::size_floater(ctx, INSPECTOR_ID, egui::vec2(size.x, target));
        }
    }
}

/// The detached windows, hidden or shown, keep the windows they are glued
/// to (the build window's included) while they are away: shown again, they
/// come back glued (`layout::park`). A block put back is a window like any
/// other again.
pub(super) fn park(ctx: &egui::Context, s: &BuildSettings) {
    for block in Block::ALL {
        crate::ui::layout::park(ctx, egui::Id::unique(floater_id(block)), is_detached(s, block));
    }
    crate::ui::layout::park(ctx, egui::Id::unique(INSPECTOR_ID), available(s) && !s.detached.is_empty());
}

/// No detached window while the build window is closed.
pub(super) fn hide(tool: &mut BuildTool) {
    tool.ui.detached_tabs.clear();
    tool.ui.detached_shown.clear();
    tool.ui.detach_drag = None;
    tool.ui.detach_press.clear();
    tool.ui.detach_fit.clear();
}

/// Show or hide a detached block's window (its rail button).
pub(super) fn toggle(tool: &mut BuildTool, s: &mut BuildSettings, block: Block) {
    if block.tab().is_some() && tool.tool != Tool::Edit {
        // Back to editing, as the button of a section does: its window shows.
        tool.set_tool(Tool::Edit);
        s.detached.insert(block, true);
        return;
    }
    if let Some(shown) = s.detached.get_mut(&block) {
        *shown = !*shown;
    }
}

/// The tooltip of a detached block's button.
pub(super) fn toggle_hint(block: Block, shown: bool) -> String {
    format!(
        "{}\n{}",
        title(block),
        if shown {
            aurora_i18n::tr!("build-detach-shown")
        } else {
            aurora_i18n::tr!("build-detach-hidden")
        }
    )
}

/// The small window mark in the corner of a detached block's button.
pub(super) fn mark(ui: &egui::Ui, p: &Palette, rect: egui::Rect) {
    widgets::paint_glyph(
        ui.painter(),
        "app-window-filled",
        egui::pos2(rect.right() - 8.0, rect.top() + 8.0),
        10.0,
        p.muted,
    );
}

/// Alt + drag over `rect` starts taking `block` out of the build window.
/// Called after the controls of `rect`, so it is on top of them while Alt
/// is held (and for the whole drag, Alt released or not).
pub(super) fn source(ui: &egui::Ui, p: &Palette, tool: &mut BuildTool, s: &BuildSettings, block: Block, place: &str, rect: egui::Rect) {
    if !available(s) || tool.ui.layout.organizing {
        return;
    }
    let dragging = tool.ui.detach_drag == Some(block);
    if !dragging && (!ui.input(|i| i.modifiers.alt) || tool.ui.detach_drag.is_some()) {
        return;
    }
    let response = ui
        .interact(rect, egui::Id::unique(("build-detach", block, place)), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab);
    if response.drag_started() {
        tool.ui.detach_drag = Some(block);
    }
    if !dragging && response.hovered() && tool.ui.detach_drag.is_none() {
        widgets::organization_source(ui, p, rect, true, true);
    }
}

/// Where the build window is in the viewer's window: what a block is
/// dropped on to go back in. None while it is in a window of its own
/// outside: the place it left inside is no longer its own.
fn main_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = egui::Id::unique(INSPECTOR_ID);
    if widgets::floater_outside(ctx, id) {
        return None;
    }
    ctx.memory(|m| m.area_rect(id))
}

fn outline_main(ctx: &egui::Context, p: &Palette, rect: egui::Rect) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("build-detach-target")));
    painter.rect_stroke(
        rect,
        ctx.global_style().visuals.window_corner_radius,
        egui::Stroke::new(2.0, p.violet_light),
        egui::StrokeKind::Inside,
    );
}

/// After the build window: the block carried with Alt, dropped outside the
/// build window, opens in its own window there.
pub(super) fn finish(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, s: &mut BuildSettings) {
    let Some(block) = tool.ui.detach_drag else {
        return;
    };
    let pointer = ctx.pointer_interact_pos();
    let inside = |pos: egui::Pos2| main_rect(ctx).is_some_and(|rect| rect.contains(pos));
    if ctx.input(|i| i.pointer.primary_down()) {
        widgets::organization_drag_preview(ctx, p, egui::Id::unique("build-detach-drag"), title(block));
        if pointer.is_some_and(inside) {
            ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
        return;
    }
    tool.ui.detach_drag = None;
    let Some(pos) = pointer.filter(|pos| !inside(*pos)) else {
        return;
    };
    let id = floater_id(block);
    // Each time it is taken out it opens at a size made for its content;
    // hidden and shown again, or with the build window, it keeps its own.
    let size = default_size(tool, block);
    if block != Block::Contents {
        tool.ui.detach_fit.insert(block, FIT_FRAMES);
    }
    // The title bar under the pointer, as if the block were carried by it.
    let corner = pos - egui::vec2(40.0, 14.0);
    let screen = ctx.content_rect();
    let corner = egui::pos2(
        corner.x.clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
        corner.y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
    );
    widgets::place_floater(ctx, id, corner, size);
    s.detached.insert(block, true);
    if block.tab() == Some(tool.tab) {
        // The build window shows the next section left in it.
        tool.ui.detached_tabs.push(tool.tab);
    }
}

fn default_size(tool: &BuildTool, block: Block) -> Vec2 {
    let height = match block {
        Block::Transform => tool.ui.transform_height.unwrap_or(190.0) + 18.0,
        Block::Contents => CONTENTS_HEIGHT,
        _ => 520.0,
    };
    Vec2::new(opening_width(block), height)
}

/// The detached windows that are shown, and the drop of one of them back
/// onto the build window.
#[allow(clippy::too_many_arguments)]
pub(super) fn windows(
    ctx: &egui::Context,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
) {
    if !available(s) {
        return;
    }
    let main = main_rect(ctx);
    let pressed = ctx.input(|i| i.pointer.primary_pressed());
    let released = ctx.input(|i| i.pointer.primary_released());
    let pointer = ctx.pointer_interact_pos();
    let blocks: Vec<Block> = s.detached.iter().filter(|(_, shown)| **shown).map(|(block, _)| *block).collect();
    let mut reattach = None;
    for block in blocks {
        let shown = match block {
            Block::Transform => matches!(tool.tool, Tool::Edit | Tool::Create),
            _ => tool.tool == Tool::Edit,
        };
        if !shown {
            continue;
        }
        let id = floater_id(block);
        let area = egui::Id::unique(id);
        let mut open = true;
        let mut floater = Floater::new(
            id,
            title(block),
            egui::pos2(ctx.content_rect().center().x, 80.0),
            default_size(tool, block),
        )
        .silent()
        .steady_size();
        if block == Block::Transform {
            // its titles say what it is: the cross alone closes it
            floater = floater.headless();
        }
        floater.min_size = if block == Block::Transform {
            Vec2::new(150.0, 40.0)
        } else {
            Vec2::new(300.0, 220.0)
        };
        let fit = floater.show(ctx, p, &mut open, |ui| {
            common::typography(ui);
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            let (room, top) = (ui.available_height(), ui.cursor().min.y);
            let content = body(ui, p, tool, world, s, mods, env, block);
            (room, content.unwrap_or(ui.cursor().min.y - top))
        });
        let fitting = tool.ui.detach_fit.get(&block).copied();
        if (block == Block::Transform || fitting.is_some())
            && let (Some((room, used)), Some(rect)) = (fit, ctx.memory(|m| m.area_rect(area)))
        {
            // The transform block's height always follows its layout (fewer
            // lines, wider); a section's, while it opens.
            let most = (ctx.content_rect().height() - 80.0).max(200.0);
            let target = (rect.height() - room + used + if block == Block::Transform { 0.0 } else { 8.0 })
                .clamp(if block == Block::Transform { 40.0 } else { 160.0 }, most)
                .round();
            if (rect.height() - target).abs() > 0.5 && !ctx.input(|i| i.pointer.primary_down()) {
                widgets::size_floater(ctx, id, egui::vec2(rect.width(), target));
            }
        }
        if let Some(frames) = fitting {
            if frames <= 1 {
                tool.ui.detach_fit.remove(&block);
            } else {
                tool.ui.detach_fit.insert(block, frames - 1);
            }
        }
        if !open {
            s.detached.insert(block, false);
            tool.ui.detach_press.remove(&block);
            continue;
        }
        // Moved (its rect moves, its size stays) and let go
        // over the build window: the block goes back in.
        let rect = ctx.memory(|m| m.area_rect(area));
        if pressed
            && let (Some(rect), Some(pos)) = (rect, pointer)
            && rect.contains(pos)
        {
            tool.ui.detach_press.insert(block, rect);
        }
        let Some(start) = tool.ui.detach_press.get(&block).copied() else {
            continue;
        };
        let moved = rect.is_some_and(|rect| (rect.min - start.min).length() > 2.0 && (rect.size() - start.size()).length() < 1.0);
        let over = pointer.zip(main).is_some_and(|(pos, main)| main.contains(pos));
        if moved && over {
            if released {
                reattach = Some(block);
            } else if let Some(main) = main {
                outline_main(ctx, p, main);
            }
        }
        if released || !ctx.input(|i| i.pointer.primary_down()) {
            tool.ui.detach_press.remove(&block);
        }
    }
    if let Some(block) = reattach {
        s.detached.remove(&block);
        tool.ui.detach_press.remove(&block);
        if let Some(tab) = block.tab() {
            tool.tab = tab;
        } else {
            s.transform_expanded = true;
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// The block's content; for a section, the height of what it shows in its
/// scrolled area.
fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
    block: Block,
) -> Option<f32> {
    let Some(tab) = block.tab() else {
        if tool.selection.is_empty() {
            common::small(ui, p, aurora_i18n::tr!("build-ui-tools-nothing-selected"));
        } else {
            let arrange = super::object::TransformArrange::for_width(ui.available_width());
            super::object::show_transform_arranged(ui, p, tool, world, s, mods, arrange, false);
        }
        return None;
    };
    let width = widgets::scrolled_width(ui).max(1.0);
    let shown = egui::ScrollArea::vertical()
        .id_salt(("build_detached", block))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(width);
            let top = ui.cursor().min.y;
            if tool.selection.is_empty() && !matches!(tab, 3 | 5) {
                common::small(ui, p, aurora_i18n::tr!("build-ui-tools-nothing-selected"));
            } else {
                compact::section_tab(ui, p, tool, world, s, mods, env, tab);
            }
            ui.cursor().min.y - top
        });
    Some(shown.inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(detached: &[(Block, bool)]) -> BuildSettings {
        BuildSettings {
            legacy_window: false,
            detached: detached.iter().copied().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn detached_sections_leave_the_build_window_and_show_only_while_editing() {
        let s = settings(&[(Block::Materials, true), (Block::Contents, false), (Block::Transform, true)]);
        let mut tool = BuildTool::default();
        tool.set_tool(Tool::Edit);
        prepare(&mut tool, &s);
        let mut tabs = tool.ui.detached_tabs.clone();
        tabs.sort_unstable();
        // the transform block is no section: it has no tab
        assert_eq!(tabs, vec![3, 4]);
        assert_eq!(tool.ui.detached_shown, vec![3]);
        tool.set_tool(Tool::Grab);
        prepare(&mut tool, &s);
        assert!(tool.ui.detached_shown.is_empty(), "a section shows while editing only");
        hide(&mut tool);
        assert!(tool.ui.detached_tabs.is_empty() && tool.ui.detached_shown.is_empty());
    }

    #[test]
    fn the_legacy_window_takes_nothing_out() {
        let mut s = settings(&[(Block::Materials, true)]);
        s.legacy_window = true;
        let mut tool = BuildTool::default();
        prepare(&mut tool, &s);
        assert!(tool.ui.detached_tabs.is_empty());
        assert!(!is_detached(&s, Block::Materials));
        assert!(!sectionless(&tool, &s));
    }

    #[test]
    fn a_detached_button_shows_its_window_and_brings_editing_back() {
        let mut s = settings(&[(Block::General, true), (Block::Transform, true)]);
        let mut tool = BuildTool::default();
        tool.set_tool(Tool::Edit);
        toggle(&mut tool, &mut s, Block::General);
        assert!(!s.detached[&Block::General]);
        toggle(&mut tool, &mut s, Block::General);
        assert!(s.detached[&Block::General]);
        // from another tool, a section's button goes back to editing and shows it
        s.detached.insert(Block::General, false);
        tool.set_tool(Tool::Focus);
        toggle(&mut tool, &mut s, Block::General);
        assert_eq!(tool.tool, Tool::Edit);
        assert!(s.detached[&Block::General]);
        // the transform block only flips
        tool.set_tool(Tool::Grab);
        toggle(&mut tool, &mut s, Block::Transform);
        assert_eq!(tool.tool, Tool::Grab);
        assert!(!s.detached[&Block::Transform]);
        // a block put back is no longer toggled
        tool.set_tool(Tool::Edit);
        let mut s = settings(&[]);
        toggle(&mut tool, &mut s, Block::Shape);
        assert!(s.detached.is_empty());
    }

    #[test]
    fn the_build_window_is_sectionless_when_its_tab_is_out() {
        let s = settings(&[(Block::General, true)]);
        let mut tool = BuildTool::default();
        tool.set_tool(Tool::Edit);
        tool.tab = 6;
        assert!(sectionless(&tool, &s));
        tool.tab = 5;
        assert!(!sectionless(&tool, &s));
        tool.tab = 6;
        tool.set_tool(Tool::Grab);
        assert!(!sectionless(&tool, &s), "the other tools show no section anyway");
    }

    /// In a window of its own outside the viewer's, the build window is
    /// fitted from the size it has there, and the place it left inside is
    /// no longer its own: nothing is dropped on it nor outlined there.
    #[test]
    fn the_build_window_outside_is_fitted_from_its_size_there() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique(INSPECTOR_ID);
        // inside the viewer's window: 420 x 700
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Area::new(id).fixed_pos(egui::pos2(50.0, 60.0)).show(ui.ctx(), |ui| {
                    ui.allocate_space(egui::vec2(420.0, 700.0));
                });
            });
        }
        assert_eq!(main_rect(&ctx).map(|rect| rect.size()), Some(egui::vec2(420.0, 700.0)));
        // sent outside, where it is 500 x 900
        crate::outside::want(&ctx, id, true);
        crate::outside::set_native(&ctx, id, true);
        ctx.data_mut(|d| d.insert_persisted(id.with(widgets::OUTSIDE_SIZE), egui::vec2(500.0, 900.0)));
        assert_eq!(main_rect(&ctx), None);
        // its section is taken out: 600 of its body for what needs 100
        let mut s = settings(&[(Block::General, true)]);
        let mut tool = BuildTool::default();
        tool.set_tool(Tool::Edit);
        tool.tab = 6;
        tool.ui.body_height = Some(600.0);
        tool.ui.sectionless_body = Some(100.0);
        fit_window(&ctx, &tool, &s);
        assert_eq!(crate::outside::take_resizes(&ctx), [(id, egui::vec2(500.0, 400.0), false)]);
        // the section comes back: the height it had outside
        ctx.data_mut(|d| d.insert_persisted(id.with(widgets::OUTSIDE_SIZE), egui::vec2(500.0, 400.0)));
        s.detached.clear();
        fit_window(&ctx, &tool, &s);
        assert_eq!(crate::outside::take_resizes(&ctx), [(id, egui::vec2(500.0, 900.0), false)]);
    }

    #[test]
    fn a_block_opens_at_a_size_made_for_it() {
        let tool = BuildTool::default();
        assert_eq!(default_size(&tool, Block::General).x, 386.0);
        assert_eq!(default_size(&tool, Block::Shape).x, 310.0);
        assert_eq!(default_size(&tool, Block::Materials).x, 325.0);
        // the contents list scrolls in a fixed height instead of growing
        assert_eq!(default_size(&tool, Block::Contents), Vec2::new(350.0, CONTENTS_HEIGHT));
        for block in Block::ALL {
            let size = default_size(&tool, block);
            assert!(size.x >= 300.0 && size.y > 0.0, "{block:?}: {size:?}");
            assert!(floater_id(block).starts_with("build_detached_"));
        }
    }
}
