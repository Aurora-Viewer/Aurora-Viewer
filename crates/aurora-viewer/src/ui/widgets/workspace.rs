//! Shared hierarchy for floaters: navigation, inset surfaces and summaries.

use super::*;
use egui::AtomExt;

/// The user tree and object contents share row density and interaction styling.
pub(crate) fn inventory_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.spacing_mut().button_padding = egui::vec2(2.0, 1.0);
    ui.spacing_mut().interact_size.y = 20.0;
}

pub(crate) fn inventory_item_label(
    ui: &mut egui::Ui,
    label: egui::text::LayoutJob,
    selected: bool,
    sense: Sense,
    bold_name: Option<std::sync::Arc<egui::Galley>>,
) -> egui::Response {
    inventory_item_label_tagged(ui, label, selected, sense, bold_name, None)
}

/// `inventory_item_label` with a row tag (`paint_row_tag`), such as "Worn",
/// in the room `reserve_row_tag` left in `label` before the character `at`.
pub(crate) fn inventory_item_label_tagged(
    ui: &mut egui::Ui,
    label: egui::text::LayoutJob,
    selected: bool,
    sense: Sense,
    bold_name: Option<std::sync::Arc<egui::Galley>>,
    tag: Option<(&Palette, &str, usize)>,
) -> egui::Response {
    inventory_spacing(ui);
    let text_id = egui::IdSalt::new("label");
    let tag = tag.map(|(p, text, at)| {
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(label.clone()));
        let x = galley.pos_from_cursor(egui::text::CCursor::new(at)).min.x;
        (p, text, x - row_tag_width(ui, text))
    });
    ui.add_sized([ui.available_width(), 20.0], |ui: &mut egui::Ui| {
        let response = egui::Button::selectable(selected, "")
            .left_text(egui::Atom::from(label).atom_id(text_id))
            .wrap_mode(egui::TextWrapMode::Extend)
            .sense(sense)
            .atom_ui(ui);
        // LLItemBridge::getLabelStyle: only the worn name is bold; `label`
        // has it in semi-bold, and `bold_name` is its faux bold second pass
        // when the font has no semi-bold face
        if let Some((name, rect)) = bold_name.zip(response.rect(text_id))
            && let Some(offset) = faux_bold_offset(ui)
        {
            let color = ui.visuals().text_color();
            ui.painter().galley(rect.min + offset, name, color);
        }
        if let Some(((p, text, x), rect)) = tag.zip(response.rect(text_id)) {
            paint_row_tag(ui, p, text, true, rect.left() + x, rect.center().y);
        }
        response.response
    })
}

/// The Tabler caret of something that unfolds: down when open.
pub fn disclosure_caret(open: bool) -> &'static str {
    if open { "caret-down-filled" } else { "caret-right-filled" }
}

/// The Tabler caret of a sorted column: up or down, as the list's arrow
/// pointed.
pub fn sort_caret(up: bool) -> &'static str {
    if up { "caret-up-filled" } else { "caret-down-filled" }
}

/// The width a caret of `paint_glyph` takes after a text of `size`.
pub fn caret_room(size: f32) -> f32 {
    3.0 + size
}

/// A Tabler icon painted `size` wide, centered on `center`.
pub fn paint_glyph(painter: &egui::Painter, name: &str, center: egui::Pos2, size: f32, color: Color32) {
    if let Some(icon) = crate::ui::icons::global(name) {
        let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        painter.image(icon.id(), egui::Rect::from_center_size(center, Vec2::splat(size)), uv, color);
    }
}

/// The mark of an experimental option: a small amber warning triangle in
/// `rect`, which explains itself on hover ("work in progress"). Used beside
/// the label of a setting that may still change or cost more than expected.
pub fn experimental_mark(ui: &egui::Ui, p: &Palette, rect: egui::Rect, id: egui::Id) -> egui::Response {
    paint_glyph(
        ui.painter(),
        "alert-triangle-filled",
        rect.center(),
        rect.width().min(rect.height()),
        p.amber,
    );
    ui.interact(rect, id, Sense::hover())
        .on_hover_text(aurora_i18n::tr!("ui-experimental-tip"))
}

pub fn glyph(ui: &mut egui::Ui, name: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if let Some(icon) = crate::ui::icons::global(name) {
        ui.painter().image(
            icon.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

pub fn navigation(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool) -> egui::Response {
    navigation_beside(ui, p, icon, label, selected, 0.0)
}

/// A `navigation` entry whose label is cut `room` before its right end,
/// where the caller paints a count: at a large text size the label would
/// run under it.
pub fn navigation_beside(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool, room: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            if selected { p.violet.gamma_multiply(0.24) } else { p.raised },
        );
    }
    if selected {
        ui.painter().vline(
            rect.left() + 1.0,
            rect.top() + 6.0..=rect.bottom() - 6.0,
            Stroke::new(2.0, p.violet),
        );
    }
    if let Some(icon) = crate::ui::icons::global(icon) {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_center_size(rect.left_center() + egui::vec2(17.0, 0.0), Vec2::splat(17.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if selected { p.violet_pale } else { p.muted },
        );
    }
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - 42.0 - room).max(1.0),
        egui::FontId::proportional(12.0),
    );
    super::paint_row_text(
        ui.painter(),
        galley,
        rect.left() + 34.0,
        rect.center().y,
        if selected { p.ink } else { p.muted },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response.on_hover_text(label)
}

/// A `navigation` entry whose label wraps on two lines instead of being
/// cut, with a small `count` at its right end (a share, a number of items).
pub fn navigation_wrapped(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, count: Option<&str>, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let count_g = count.map(|c| {
        ui.painter()
            .layout_no_wrap(c.to_owned(), egui::FontId::proportional(11.0), p.muted_dim)
    });
    let count_w = count_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let ink = if selected { p.ink } else { p.muted };
    let mut job = egui::text::LayoutJob::simple(
        label.to_owned(),
        egui::FontId::proportional(12.0),
        ink,
        (width - 42.0 - count_w).max(1.0),
    );
    job.wrap.max_rows = 2;
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, (galley.size().y + 16.0).max(34.0)), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            if selected { p.violet.gamma_multiply(0.24) } else { p.raised },
        );
    }
    if selected {
        ui.painter().vline(
            rect.left() + 1.0,
            rect.top() + 6.0..=rect.bottom() - 6.0,
            Stroke::new(2.0, p.violet),
        );
    }
    if let Some(icon) = crate::ui::icons::global(icon) {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_center_size(rect.left_center() + egui::vec2(17.0, 0.0), Vec2::splat(17.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if selected { p.violet_pale } else { p.muted },
        );
    }
    super::paint_row_text(ui.painter(), galley, rect.left() + 34.0, rect.center().y, ink);
    if let Some(g) = count_g {
        let pos = super::row_text_origin(ui.painter(), &g, rect.right() - 9.0 - g.size().x, rect.center().y);
        ui.painter().galley(pos, g, p.muted_dim);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response
}

pub fn surface<R>(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.8))
        .stroke(Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(10)
        .show(ui, body)
        .inner
}

/// Shared presentation of the construction and inventory layout editors.
pub(crate) fn organization_typography(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    for text in [egui::TextStyle::Body, egui::TextStyle::Button, egui::TextStyle::Monospace] {
        if let Some(font) = style.text_styles.get_mut(&text) {
            font.size = 12.0;
        }
    }
    if let Some(font) = style.text_styles.get_mut(&egui::TextStyle::Small) {
        font.size = 11.0;
    }
}

pub(crate) fn organization_section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(13.0).strong().color(p.violet_light));
    ui.add_space(1.0);
}

/// Violet section heading with the unobtrusive close action at its right.
pub(crate) fn organization_header(ui: &mut egui::Ui, p: &Palette, title: &str) -> egui::Response {
    ui.horizontal(|ui| {
        organization_section(ui, p, title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 24.0), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
            }
            paint_glyph(
                ui.painter(),
                "x",
                rect.center(),
                14.0,
                if response.hovered() { p.ink } else { p.muted },
            );
            let title = aurora_i18n::tr!("common-close");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
            response.on_hover_text(title)
        })
        .inner
    })
    .inner
}

pub(crate) fn organization_drag_preview(ctx: &egui::Context, p: &Palette, id: egui::Id, title: &str) {
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    if let Some(pos) = ctx.pointer_interact_pos() {
        egui::Area::new(id)
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + egui::vec2(12.0, 12.0))
            .interactable(false)
            .show(ctx, |ui| {
                // This Area is reused for different commands. Its previous
                // short label must not constrain the next label's wrapping.
                ui.set_max_width(ui.spacing().tooltip_width.min(ctx.content_rect().width()));
                overlay(ui, p, egui::Margin::same(6), |ui| {
                    ui.add(egui::Label::new(RichText::new(title).size(12.0).color(p.muted)).wrap());
                });
            });
    }
}

pub fn section_header(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        glyph(ui, icon, p.violet_light, 17.0);
        ui.label(RichText::new(title).size(13.0).strong().color(p.ink));
    });
    ui.add_space(4.0);
}

/// What [`collapsing`] returns: its header row, and the value of the body
/// while the section shows it.
pub struct Collapsing<R> {
    pub header: egui::Response,
    pub body: Option<R>,
}

/// Collapsible section with the header of the interface: a full-width row
/// with a Tabler caret and the section's title, lit like a toolbar pill under
/// the pointer, without a line. The open state and the fold animation are
/// egui's (`CollapsingState`), under the id a stock `CollapsingHeader` gave
/// to the same `id_salt`: what the user left open stays open.
pub fn collapsing<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    id_salt: impl egui::AsIdSalt,
    title: impl Into<egui::WidgetText>,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> Collapsing<R> {
    collapsing_forced(ui, p, id_salt, title, default_open, None, body)
}

/// [`collapsing`] whose state can be imposed for this frame: `Some(true)`
/// opens the section (to reveal something in it), `None` leaves it to the
/// user.
pub fn collapsing_forced<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    id_salt: impl egui::AsIdSalt,
    title: impl Into<egui::WidgetText>,
    default_open: bool,
    open: Option<bool>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> Collapsing<R> {
    // One unit with the body under the header, like the stock header: a
    // section can sit in a grid, and its id is the one egui gave it there.
    ui.vertical(|ui| section(ui, p, egui::IdSalt::new(id_salt), title.into(), default_open, open, body))
        .inner
}

fn section<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    id_salt: egui::IdSalt,
    title: egui::WidgetText,
    default_open: bool,
    open: Option<bool>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> Collapsing<R> {
    const CARET: f32 = 12.0;
    let id = ui.make_persistent_id(id_salt);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    // a plain title takes the style of the section titles; a caller's own
    // rich text or galley (an outfit's name with its suffix) is kept
    let title = match title {
        egui::WidgetText::Text(text) => RichText::new(text).size(12.5).strong().color(p.ink).into(),
        styled => styled,
    };
    let indent = ui.spacing().indent;
    let width = ui.available_width();
    let galley = title.into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (width - indent - 4.0).max(0.0),
        egui::TextStyle::Button,
    );
    let height = ui.spacing().interact_size.y.max(galley.size().y + 4.0);
    let (_, rect) = ui.allocate_space(Vec2::new(width, height));
    let mut header = ui.interact(rect, id, Sense::click());
    if let Some(open) = open {
        if open != state.is_open() {
            state.toggle(ui);
            header.mark_changed();
        }
    } else if header.clicked() {
        state.toggle(ui);
        header.mark_changed();
    }
    header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, ui.is_enabled(), galley.text()));
    if ui.is_rect_visible(rect) {
        let hot = header.hovered() || header.has_focus();
        if hot {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        }
        let caret = if state.is_open() {
            "caret-down-filled"
        } else {
            "caret-right-filled"
        };
        if let Some(icon) = crate::ui::icons::global(caret) {
            ui.painter().image(
                icon.id(),
                egui::Rect::from_center_size(egui::pos2(rect.left() + indent / 2.0, rect.center().y), Vec2::splat(CARET)),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                if hot { p.ink } else { p.muted },
            );
        }
        let text = super::row_text_origin(ui.painter(), &galley, rect.left() + indent, rect.center().y);
        ui.painter().galley(text, galley, p.ink);
    }
    let body = state.show_body_indented(&header, ui, body).map(|r| r.inner);
    Collapsing { header, body }
}

pub fn stat(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.0).color(p.muted));
        ui.add(egui::Label::new(RichText::new(value).size(15.0).strong().color(p.ink)).truncate())
            .on_hover_text(value);
    });
}

/// Title icon of a window, from its id: the icon its function has in the
/// toolbar and the menus. A window opened by another one (a picker, a tool of
/// the script editor) is named after its owner: its own function comes first.
pub(super) fn window_icon(id: &str) -> &'static str {
    if id == "poser" {
        return "wand";
    }
    if id.starts_with("poser-") {
        return "adjustments-filled";
    }
    if id.contains("avatar_picker") || id.starts_with("inventory_share") {
        return "users-group";
    }
    if id.contains("color_picker") {
        return "palette-filled";
    }
    if id.starts_with("notecard") {
        return "file-text-filled";
    }
    if id.starts_with("inventory") {
        return "folder-filled";
    }
    if id.starts_with("profile") {
        return "user-filled";
    }
    if id.starts_with("bug_report") {
        return "bug-filled";
    }
    if id.starts_with("texture_library") || id.starts_with("resource_preview_") {
        return "stack-2-filled";
    }
    if id.contains("texture") || id.contains("thumbnail") || id == "link_preview_image" {
        return "photo-filled";
    }
    // the tools of a script editor carry its egui id and their own suffix
    if id.ends_with("_search") {
        return "search-filled";
    }
    if id.ends_with("_help") {
        return "help-circle-filled";
    }
    if id.ends_with("_library") {
        return "library-filled";
    }
    if id.contains("script") || id.ends_with("_goto") || id.ends_with("_revert") || id.ends_with("_file_conflict") {
        return "code";
    }
    // before the places: "marketplace" holds "place"
    if id == super::super::marketplace::WINDOW {
        return "building-store";
    }
    if id == super::super::marketplace_listings::WINDOW || id == "marketplace_validation" {
        return "tag-filled";
    }
    if id == super::super::region::WINDOW {
        return "building-castle";
    }
    if id == super::super::owned_land::WINDOW {
        return "map";
    }
    if id.contains("place") || id.contains("land") {
        return "map-pin-filled";
    }
    if id.starts_with("ao") {
        return "man-filled";
    }
    if id.starts_with("build_tools") {
        return "box";
    }
    if id == super::super::animation_explorer::WINDOW {
        return "run";
    }
    match id {
        "appearance" | "appearance_item_profile" => "shirt-filled",
        "conversations" | "contacts" => "messages-filled",
        "chat_history" => "history",
        "people" | "group_picker" | "group_profile" | "contact_set_config" => "users-group",
        "group_invite" | "group_invite_choice" => "user-plus",
        "search" => "search-filled",
        "options" => "settings-filled",
        "worldmap" | "minimap" => "map",
        "beacons" => "radar-filled",
        "environment_selector" | "personal_lighting" => "sun-filled",
        "perf" | "performance" => "gauge-filled",
        "photo" | "photo_classic" => "camera-filled",
        "controls" | "camera_controls" => "video-filled",
        "movement_controls" => "walk",
        "display_name" => "id-filled",
        "object_purchase" => "shopping-cart-filled",
        "object_payment" | "buy_currency" | "pay" | "pay_classic" => "coin-filled",
        "money_tracker" | "money_tracker_legacy" => "receipt-filled",
        "object_contents" => "package",
        "object_weights" => "box",
        "build_options" => "adjustments-filled",
        "media_settings" => "device-tv-filled",
        "bulk_perms" => "lock-filled",
        "region_restarting" => "alert-triangle-filled",
        "task_item_props" => "file-info-filled",
        "asset_imports_v3" | "asset_imports_v4" | "asset_imports_v5" | "asset_imports_v6" | "import_launcher" => "file-upload-filled",
        "import_textures" => "photo-filled",
        "import_sounds" => "file-music-filled",
        "import_animations" => "run",
        "import_materials" => "droplet-filled",
        "import_meshes" => "cube-3d-sphere",
        "import_local" => "eye-filled",
        "import_settings" => "settings-filled",
        "toolbar_editor" => "layout-bottombar-filled",
        "particle_editor" => "sparkles-filled",
        _ => "layout-grid-filled",
    }
}

/// Reserve a drawing slot before the content, then resolve its exact bounds
/// after layout. The opaque skin remains available with panel_alpha = 255.
pub(crate) fn finish_glass(painter: &egui::Painter, slot: egui::layers::ShapeIdx, rect: egui::Rect, p: &Palette, radius: f32) {
    if p.panel_alpha < 255 {
        painter.set(
            slot,
            aurora_render::glass_panel(
                rect.shrink(1.0),
                radius,
                Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha),
            ),
        );
    }
}

/// Glass of the top and bottom bars: the windows' blur under the `bar` color.
pub(crate) fn finish_bar_glass(painter: &egui::Painter, slot: egui::layers::ShapeIdx, rect: egui::Rect, p: &Palette) {
    if p.panel_alpha < 255 {
        let tint = Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), p.panel_alpha);
        painter.with_clip_rect(rect).set(slot, aurora_render::glass_panel(rect, 0.0, tint));
    }
}

/// Surface of a glass island of the Aurora bars (`ui/bars/islands.rs`):
/// the windows' shadow, the blur they share under the `bar` colour, a
/// hairline border, the windows' corners. The islands are a shade lighter
/// than the full-width bars (80 % of the colour where the bars and windows
/// have 90 %), as in the mockup the human validated. An opaque skin
/// (panel_alpha = 255) gets a plain face.
pub(crate) fn island_shape(ctx: &egui::Context, p: &Palette, rect: egui::Rect) -> egui::Shape {
    let visuals = &ctx.global_style().visuals;
    let radius = visuals.window_corner_radius;
    let shadow = visuals.window_shadow.as_shape(rect, radius);
    let border = super::border_stroke(p, Stroke::new(1.0, p.ink.gamma_multiply(0.07)));
    let face = if p.panel_alpha < 255 {
        let alpha = (f32::from(p.panel_alpha) * ISLAND_TINT).round() as u8;
        let tint = Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), alpha);
        aurora_render::glass_panel(rect.shrink(1.0), f32::from(radius.nw), tint)
    } else {
        egui::Shape::rect_filled(rect, radius, p.bar)
    };
    egui::Shape::Vec(vec![
        shadow.into(),
        face,
        egui::Shape::rect_stroke(rect, radius, border, egui::StrokeKind::Inside),
    ])
}

/// The islands' tint against the bars' (0.80 / 0.90).
const ISLAND_TINT: f32 = 0.8 / 0.9;

/// Frame of a panel drawn outside a window (toasts, drop-down panels, the
/// login cards): the windows' tint, border, corners and shadow.
pub(crate) fn overlay_frame(ctx: &egui::Context, p: &Palette) -> egui::Frame {
    let visuals = &ctx.global_style().visuals;
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            p.panel.r(),
            p.panel.g(),
            p.panel.b(),
            p.panel_alpha,
        ))
        .stroke(super::border_stroke(p, Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6))))
        .corner_radius(visuals.window_corner_radius)
        .shadow(visuals.window_shadow)
}

/// A laid-out world label, using the overlay's border, shadow and corners.
/// A lighter tint exposes the scene blur; shading keeps text legible against
/// bright skies, as on avatar name plates. Opaque skins keep their surface.
pub(crate) fn paint_world_label(painter: &egui::Painter, p: &Palette, rect: egui::Rect) {
    let mut frame = overlay_frame(painter.ctx(), p);
    let glass = p.panel_alpha < 255;
    if glass {
        frame.fill = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), (f32::from(p.panel_alpha) * 0.65) as u8);
    }
    painter.add(frame.paint(rect));
    if glass {
        let radius = f32::from(frame.corner_radius.nw);
        let inner = rect.shrink(1.0);
        painter.add(aurora_render::glass_region(inner, radius, 1.0, 0.65));
        painter.rect_filled(inner, frame.corner_radius, frame.fill);
    }
}

/// `overlay_frame` over the windows' blur of the world behind it.
pub(crate) fn overlay<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    margin: egui::Margin,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut glass_slot = None;
    let response = overlay_frame(ui.ctx(), p).inner_margin(margin).show(ui, |ui| {
        glass_slot = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
        body(ui)
    });
    if let Some((painter, slot)) = glass_slot {
        let radius = f32::from(ui.ctx().global_style().visuals.window_corner_radius.nw);
        finish_glass(&painter, slot, response.response.rect, p, radius);
    }
    response
}

/// Modal dialogs use the same surface and spacing as regular windows.
pub struct Modal(egui::Modal, Option<(Palette, f32)>);

impl Modal {
    pub fn new(id: egui::Id) -> Self {
        Self(egui::Modal::new(id), None)
    }
    /// For a modal that blurs what is behind it (`aurora_render::blurred_backdrop`)
    /// instead of dimming it.
    pub fn transparent_backdrop(self) -> Self {
        Self(self.0.backdrop_color(Color32::TRANSPARENT), self.1)
    }

    /// Login glass includes the sky and account cards, drawn below the modal.
    pub fn login_surface(mut self, p: &Palette) -> Self {
        self.1 = Some((*p, super::super::aurora_bg::LOGIN_TINT));
        if p.panel_alpha < 255 {
            self.0 = self.0.backdrop_color(Color32::TRANSPARENT);
        }
        self
    }

    /// A denser tint for editors with many fields, keeping the skin's glass.
    pub fn stronger_tint(mut self) -> Self {
        if let Some((_, tint)) = &mut self.1 {
            *tint = 0.78;
        }
        self
    }

    pub fn show<T>(self, ctx: &egui::Context, body: impl FnOnce(&mut egui::Ui) -> T) -> egui::ModalResponse<T> {
        // Theme supplies skin colors and radii; per-modal frames remain local.
        let mut frame = egui::Frame::popup(&ctx.global_style())
            .inner_margin(18)
            .corner_radius(ctx.global_style().visuals.window_corner_radius);
        if let Some((p, tint)) = self.1.filter(|(p, _)| p.panel_alpha < 255) {
            // The world sampler cannot include egui cards. The existing modal
            // backdrop blurs lower UI layers on the GPU before drawing this frame.
            aurora_render::blurred_backdrop(ctx, self.0.area.layer());
            frame.fill = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), (f32::from(p.panel_alpha) * tint) as u8);
        }
        let mut glass_slot = None;
        let response = self.0.frame(frame).show(ctx, |ui| {
            glass_slot = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            body(ui)
        });
        if let Some((painter, slot)) = glass_slot {
            let style = ctx.global_style();
            if self.1.is_none() && style.visuals.window_fill.a() < 255 {
                painter.set(
                    slot,
                    aurora_render::glass_panel(
                        response.response.rect.shrink(1.0),
                        f32::from(style.visuals.window_corner_radius.nw),
                        style.visuals.window_fill,
                    ),
                );
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_drag_preview_grows_after_short_labels_and_stays_on_screen() {
        fn text<'a>(shape: &'a egui::Shape, title: &str) -> Option<&'a egui::epaint::TextShape> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == title => Some(text),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text(shape, title)),
                _ => None,
            }
        }
        for x in [40.0, 390.0] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 300.0));
            for title in [
                "x",
                "Attacher au HUD",
                "Copier l’UUID de l’objet",
                "x",
                "Créer un dossier avec la sélection",
            ] {
                let mut output = None;
                for _ in 0..6 {
                    output = Some(crate::ui::login::test_output(ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            events: vec![egui::Event::PointerMoved(egui::pos2(x, 100.0))],
                            ..Default::default()
                        },
                        |_ui| organization_drag_preview(&ctx, &p, egui::Id::unique("changing_drag_title"), title),
                    )));
                }
                let output = output.unwrap();
                let drawn = output.shapes.iter().find_map(|shape| text(&shape.shape, title)).unwrap();
                assert_eq!(drawn.galley.rows.len(), 1, "short previous labels must not squeeze {title:?}");
                let natural = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(title.into(), egui::FontId::proportional(12.0), p.muted));
                assert!((drawn.galley.size().x - natural.size().x).abs() < 1.0);
                let rect = egui::Rect::from_min_size(drawn.pos, drawn.galley.size());
                assert!(screen.expand(1.0).contains_rect(rect), "preview text left the viewport: {rect:?}");
            }
        }
    }

    #[test]
    fn login_glass_blurs_lower_layers_before_the_translucent_frame_and_skips_opaque_skins() {
        fn has_fill(shape: &egui::Shape, color: Color32) -> bool {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_fill(shape, color)),
                egui::Shape::Rect(rect) => rect.fill == color && rect.rect.width() > 300.0 && rect.stroke.width > 0.0,
                _ => false,
            }
        }
        for alpha in [230, 255] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme {
                panel_alpha: alpha,
                ..Default::default()
            };
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let mut output = None;
            for frame in 0..8 {
                output = Some(crate::ui::login::test_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                        time: Some(frame as f64 / 30.0),
                        ..Default::default()
                    },
                    |_ui| {
                        Modal::new(egui::Id::unique("synthetic_login_glass"))
                            .login_surface(&p)
                            .show(&ctx, |ui| {
                                ui.set_width(340.0);
                                ui.label("Synthetic dialog");
                            });
                    },
                )));
            }
            let output = output.unwrap();
            let markers = output
                .shapes
                .iter()
                .enumerate()
                .filter_map(|(index, source)| {
                    matches!(&source.shape, egui::Shape::Callback(callback) if callback.rect == ctx.content_rect()).then_some(index)
                })
                .collect::<Vec<_>>();
            assert_eq!(markers.len(), usize::from(alpha < 255));
            let tint_alpha = if alpha < 255 {
                (f32::from(alpha) * super::super::super::aurora_bg::LOGIN_TINT) as u8
            } else {
                255
            };
            let tint = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), tint_alpha);
            let frame = output
                .shapes
                .iter()
                .position(|source| has_fill(&source.shape, tint))
                .expect("themed modal frame");
            if let Some(marker) = markers.first() {
                assert!(*marker < frame, "the modal frame must stay above the blur boundary");
            }
        }
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut body: impl FnMut(&mut egui::Ui)) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 300.0))),
                events,
                ..Default::default()
            },
            |ui| body(ui),
        );
        output.textures_delta.clear();
    }

    #[test]
    fn collapsing_keeps_the_id_and_the_state_of_the_stock_header() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        // a stock header opened by the user under a salt, and one under its title
        let (mut salted, mut titled) = (None, None);
        frame(&ctx, vec![], |ui| {
            let stock = egui::CollapsingHeader::new("Title")
                .id_salt(("section", 7))
                .open(Some(true))
                .show(ui, |_| {});
            salted = Some(stock.header_response.id);
            let stock = egui::CollapsingHeader::new("Other title").show(ui, |_| {});
            titled = Some(stock.header_response.id);
        });
        // the shared widget finds both: same ids, and the open state left there
        let mut bodies = 0;
        frame(&ctx, vec![], |ui| {
            let section = collapsing(ui, &p, ("section", 7), "Title", false, |_| bodies += 1);
            assert_eq!(Some(section.header.id), salted);
            let section = collapsing(ui, &p, "Other title", "Other title", false, |_| bodies += 10);
            assert_eq!(Some(section.header.id), titled);
        });
        assert_eq!(bodies, 1, "open as the stock header left it; the other one closed by default");
    }

    #[test]
    fn collapsing_folds_on_a_click_of_its_header_and_can_be_forced_open() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut header = egui::Rect::NOTHING;
        let mut shown = false;
        let mut forced = None;
        let draw = |events: Vec<egui::Event>, forced: Option<bool>, header: &mut egui::Rect, shown: &mut bool| {
            frame(&ctx, events, |ui| {
                let section = collapsing_forced(ui, &p, "fold", "Section", true, forced, |ui| {
                    ui.label("body");
                });
                *header = section.header.rect;
                *shown = section.body.is_some();
            });
        };
        draw(vec![], forced, &mut header, &mut shown);
        assert!(shown, "open by default");
        assert!(header.width() > 300.0, "the header is a full-width row");
        let pos = header.center();
        for pressed in [true, false] {
            let click = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            draw(vec![egui::Event::PointerMoved(pos), click], forced, &mut header, &mut shown);
        }
        // the fold animation ends within a few frames of test time
        for _ in 0..40 {
            draw(vec![], forced, &mut header, &mut shown);
        }
        assert!(!shown, "a click on the header folds the section");
        forced = Some(true);
        draw(vec![], forced, &mut header, &mut shown);
        assert!(shown, "forcing it open reveals the body");
    }
}
